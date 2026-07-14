// Copyright (C) 2020-2026 Andy Kurnia.

use super::{game_config, game_state, klv, kwg, move_filter, movegen, simmer, stats};

struct Candidate {
    play_index: usize,
    stats: stats::Stats,
    // stable identity for this candidate within a decision, so a retired
    // candidate can be named for readmit even as the vectors are reordered.
    stream_id: u64,
}

// Default per-decision rollout budget (see Simmer::num_sim_iters).
const DEFAULT_NUM_SIM_ITERS: u64 = 1000;

// Prune the candidate list once every this many iterations. Between prunes
// each surviving candidate gets one more rollout, so this trades the cost of
// pruning against how quickly clearly-worse candidates are dropped.
const PRUNE_CADENCE: u64 = 16;

// Default target probability of the confidence stop ending on the wrong
// leader, used when StopRule::Confidence is selected. 5% is a conventional
// choice; a caller tunes it via set_stop_delta.
const DEFAULT_STOP_DELTA: f64 = 0.05;

// How each iteration's rollouts are allocated across the surviving candidates.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Allocator {
    // Simulate every surviving candidate on every iteration's shared draw.
    // Maximum common-random-number pairing, but spends as many rollouts on
    // hopeless candidates as on the real contenders until the prune drops them.
    RoundRobin,
    // Simulate only the current leader, the strongest challenger, and one
    // least-sampled candidate (a floor so every arm keeps a live confidence
    // interval for the prune) each iteration. Concentrates the budget on the
    // moves that might actually be best.
    Adaptive,
}

// One rollout of `play` from the prepared position, returned as the objective
// value fed to the candidate's running statistics. Shared by both allocators.
#[inline(always)]
fn rollout_objective<N: kwg::Node, L: kwg::Node>(
    simmer: &mut simmer::Simmer,
    game_config: &game_config::GameConfig,
    kwg: &kwg::Kwg<N>,
    klv: &klv::Klv<L>,
    play: &movegen::Play,
) -> f64 {
    let game_ended = simmer.simulate(game_config, kwg, klv, play);
    let final_spread = simmer.final_equity_spread();
    let win_prob = simmer.compute_win_prob(game_ended, final_spread);
    let sim_spread = final_spread - simmer.initial_score_spread;
    simmer::sim_objective(
        sim_spread,
        win_prob,
        simmer.win_prob_weightage(),
        simmer.config().descale,
    )
}

// The rule that decides when a move decision is finished.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum StopRule {
    // Always run the whole fixed iteration budget (minus pruning to one arm).
    FixedCap,
    // Stop as soon as the leader is separated from every other survivor; the
    // fixed budget becomes a safety cap. See leader_is_separated.
    Confidence,
}

// The z threshold for the confidence stop, corrected for multiple comparisons.
// The stop compares the leader against each of the num_survivors - 1 other
// survivors; a union bound over those comparisons at overall error `delta`,
// using the Gaussian tail P(Z > z) <= exp(-z^2/2), gives each comparison a
// budget of delta/(num_survivors - 1) and hence z = sqrt(2 ln((num_survivors -
// 1) / delta)). This corrects for the number of arms, not for the number of
// times the stop is checked -- the fixed iteration cap bounds that peeking.
// Reference: Gaussian tail bound / Bonferroni union bound.
#[inline(always)]
fn fwer_z(num_survivors: usize, delta: f64) -> f64 {
    (2.0 * ((num_survivors - 1) as f64 / delta).ln()).sqrt()
}

// True when the leader's lower confidence bound clears every other survivor's
// upper bound at the corrected z: the leader is separated from the whole field,
// so more rollouts are unlikely to change the winner. A single survivor (or
// none) is trivially separated.
#[inline(always)]
fn leader_is_separated(candidates: &[Candidate], delta: f64) -> bool {
    let num_survivors = candidates.len();
    if num_survivors < 2 {
        return true;
    }
    let z = fwer_z(num_survivors, delta);
    let leader_idx = candidates
        .iter()
        .enumerate()
        .max_by(|(_, a), (_, b)| a.stats.mean().total_cmp(&b.stats.mean()))
        .unwrap()
        .0;
    let leader_low = candidates[leader_idx].stats.ci_max(-z);
    candidates
        .iter()
        .enumerate()
        .all(|(i, candidate)| i == leader_idx || leader_low >= candidate.stats.ci_max(z))
}

// Move the candidates the prune drops -- those whose upper confidence bound is
// below `low_bar` -- into `retired`, preserving the order of the survivors so
// the kept set is identical to a plain retain. Their statistics are kept for a
// later resume or inspection rather than thrown away.
#[inline(always)]
fn retire_below(
    candidates: &mut Vec<Candidate>,
    retired: &mut Vec<Candidate>,
    z: f64,
    low_bar: f64,
) {
    // Compact the survivors to the front in place, preserving their order (no
    // reallocation, unlike collecting into a new vec), then drain the dropped
    // tail into retired.
    let mut write = 0;
    for read in 0..candidates.len() {
        if candidates[read].stats.ci_max(z) >= low_bar {
            candidates.swap(write, read);
            write += 1;
        }
    }
    retired.extend(candidates.drain(write..));
}

// Retire the weakest survivors (by lower confidence bound) until at most
// max_candidates_allowed remain, moving each into `retired` instead of dropping
// it. The swap_remove order matches the previous discard-only version, so the
// surviving set is unchanged.
#[inline(always)]
fn limit_surviving_candidates(
    candidates: &mut Vec<Candidate>,
    retired: &mut Vec<Candidate>,
    z: f64,
    max_candidates_allowed: usize,
) {
    while candidates.len() > max_candidates_allowed {
        // pruning regularly means binary heap is not justified here.
        // also, this means candidates are almost indistinguishable.
        let idx = candidates
            .iter()
            .enumerate()
            .map(|(i, candidate)| (i, candidate.stats.ci_max(-z)))
            .min_by(|(_, a), (_, b)| a.total_cmp(b))
            .unwrap()
            .0;
        retired.push(candidates.swap_remove(idx));
    }
}

// Simmer can only be reused for the same game_config and kwg.
// (Refer to note at simmer::Simmer.)
// This is not enforced.
pub struct Simmer<'a, N: kwg::Node, L: kwg::Node> {
    game_config: &'a game_config::GameConfig,
    kwg: &'a kwg::Kwg<N>,
    klv: &'a klv::Klv<L>,
    candidates: Vec<Candidate>,
    simmer: simmer::Simmer,
    // Number of Monte-Carlo rollout iterations spent on each move decision.
    // This is a fixed budget: every decision runs this many iterations unless
    // pruning first narrows the field to a single candidate. Sized so a 2-ply
    // simmer separates the leading candidates on a typical rack; a caller that
    // wants a more accurate (slower) or quicker decision overrides it via
    // set_num_sim_iters.
    num_sim_iters: u64,
    // When false, pick_a_move skips its per-decision debug print. The batch
    // sim-vs-sim harness turns this off; the interactive picker leaves it on.
    verbose: bool,
    // How each iteration's rollouts are allocated across the candidates.
    allocator: Allocator,
    // Whether a decision runs the whole budget or stops once the leader is
    // statistically separated, and the target error for that stop.
    stop_rule: StopRule,
    stop_delta: f64,
    // Candidates dropped by the prune, kept with their statistics instead of
    // discarded, so a study session can resume and inspect them later.
    retired: Vec<Candidate>,
    // How many rollout iterations the current decision has run, so a resumed
    // decision continues from the next iteration instead of restarting.
    iters_done: u64,
    // Next stream id to hand out, so added candidates get ids distinct from the
    // initial 0..num_plays.
    next_stream_id: u64,
}

impl<'a, N: kwg::Node, L: kwg::Node> Simmer<'a, N, L> {
    pub fn new(
        game_config: &'a game_config::GameConfig,
        kwg: &'a kwg::Kwg<N>,
        klv: &'a klv::Klv<L>,
    ) -> Self {
        Self {
            game_config,
            kwg,
            klv,
            candidates: Vec::new(),
            simmer: simmer::Simmer::new(game_config),
            num_sim_iters: DEFAULT_NUM_SIM_ITERS,
            verbose: true,
            allocator: Allocator::RoundRobin,
            stop_rule: StopRule::FixedCap,
            stop_delta: DEFAULT_STOP_DELTA,
            retired: Vec::new(),
            iters_done: 0,
            next_stream_id: 0,
        }
    }

    #[inline(always)]
    pub fn set_num_sim_iters(&mut self, num_sim_iters: u64) {
        self.num_sim_iters = num_sim_iters;
    }

    #[inline(always)]
    pub fn set_verbose(&mut self, verbose: bool) {
        self.verbose = verbose;
    }

    #[inline(always)]
    pub fn set_allocator(&mut self, allocator: Allocator) {
        self.allocator = allocator;
    }

    #[inline(always)]
    pub fn set_stop_rule(&mut self, stop_rule: StopRule) {
        self.stop_rule = stop_rule;
    }

    // delta is the target probability of stopping on the wrong leader; it is
    // clamped to (0, 1) since the corrected z is only defined there.
    #[inline(always)]
    pub fn set_stop_delta(&mut self, stop_delta: f64) {
        self.stop_delta = stop_delta.clamp(f64::MIN_POSITIVE, 1.0 - f64::EPSILON);
    }

    /// Reseed the inner rollout RNG so a decision replays identically. The
    /// sim-vs-sim harness reseeds every move from a (seed, pair, game, turn)
    /// mix, making its results independent of the thread count.
    #[inline(always)]
    pub fn reseed(&mut self, seed: u64) {
        self.simmer.reseed(seed);
    }

    /// Give this seat its own rollout objective and win-probability
    /// configuration (descale, weightages, sigmoid constants).
    #[inline(always)]
    pub fn set_config(&mut self, config: simmer::SimmerConfig) {
        self.simmer.set_config(config);
    }

    #[inline(always)]
    fn take_candidates(&mut self, num_plays: usize) -> Vec<Candidate> {
        let mut candidates = std::mem::take(&mut self.candidates);
        candidates.clear();
        candidates.reserve(num_plays);
        for idx in 0..num_plays {
            candidates.push(Candidate {
                play_index: idx,
                stats: stats::Stats::new(),
                stream_id: idx as u64,
            });
        }
        candidates
    }

    // Run `count` more rollout iterations on the retained candidate set,
    // pruning (and retiring the dropped candidates) at the cadence boundaries
    // and, in Confidence mode, stopping once the leader is separated. `budget`
    // is the whole decision's iteration budget; it only tightens the survivor cap
    // toward the end, and may be exceeded when resuming past it. Advances the
    // iteration cursor; does not reset the candidate set or commit a winner.
    fn run_iterations(
        &mut self,
        move_generator: &movegen::KurniaMoveGenerator,
        budget: u64,
        count: u64,
    ) {
        let mut candidates = std::mem::take(&mut self.candidates);
        let mut retired = std::mem::take(&mut self.retired);
        const Z: f64 = 1.96; // 95% confidence interval
        let start = self.iters_done;
        for sim_iter in (start + 1)..=(start + count) {
            self.iters_done = sim_iter;
            self.simmer.prepare_iteration();
            // Round-robin warm-up: until the first prune, every candidate must
            // gather samples, or the shared prune below sees count-zero arms
            // (whose confidence interval is NaN) and empties the field. Adaptive
            // only takes over after.
            let effective_allocator = if sim_iter <= PRUNE_CADENCE {
                Allocator::RoundRobin
            } else {
                self.allocator
            };
            match effective_allocator {
                Allocator::RoundRobin => {
                    for candidate in candidates.iter_mut() {
                        let value = rollout_objective(
                            &mut self.simmer,
                            self.game_config,
                            self.kwg,
                            self.klv,
                            &move_generator.plays[candidate.play_index].play,
                        );
                        candidate.stats.update(value);
                    }
                }
                Allocator::Adaptive => {
                    // leader = highest running mean.
                    let leader_idx = candidates
                        .iter()
                        .enumerate()
                        .max_by(|(_, a), (_, b)| a.stats.mean().total_cmp(&b.stats.mean()))
                        .unwrap()
                        .0;
                    let leader_mean = candidates[leader_idx].stats.mean();
                    let leader_var = candidates[leader_idx].stats.variance();
                    let leader_n = candidates[leader_idx].stats.count();
                    // strongest challenger = the non-leader that minimizes the
                    // standardized gap (leader_mean - mean) / sqrt(var_l/n_l +
                    // var_c/n_c): the closest rival. an arm with fewer than two
                    // samples has no variance estimate yet, so treat it as
                    // maximally uncertain (gap 0) to make sure it gets sampled.
                    // Ties break deterministically: the strict `<` below keeps
                    // the lowest-indexed arm, and max_by above keeps the last
                    // leader on a mean tie -- so a co-leader falls through here as
                    // the zero-gap challenger and is still sampled, while a
                    // tied-out challenger becomes the least-sampled arm and the
                    // floor picks it up next.
                    let mut challenger_idx = leader_idx;
                    let mut best_gap = f64::INFINITY;
                    for (i, candidate) in candidates.iter().enumerate() {
                        if i == leader_idx {
                            continue;
                        }
                        let n_c = candidate.stats.count();
                        let gap = if n_c < 2.0 || leader_n < 2.0 {
                            0.0
                        } else {
                            let denom =
                                (leader_var / leader_n + candidate.stats.variance() / n_c).sqrt();
                            if denom > 0.0 {
                                (leader_mean - candidate.stats.mean()) / denom
                            } else {
                                0.0
                            }
                        };
                        if gap < best_gap {
                            best_gap = gap;
                            challenger_idx = i;
                        }
                    }
                    // floor = least-sampled candidate, so every arm keeps a live
                    // confidence interval for the prune.
                    let floor_idx = candidates
                        .iter()
                        .enumerate()
                        .min_by(|(_, a), (_, b)| a.stats.count().total_cmp(&b.stats.count()))
                        .unwrap()
                        .0;
                    // sample the deduplicated set {leader, challenger, floor}.
                    let mut to_sample = [leader_idx, challenger_idx, floor_idx];
                    to_sample.sort_unstable();
                    let mut prev = usize::MAX;
                    for &idx in &to_sample {
                        if idx == prev {
                            continue;
                        }
                        prev = idx;
                        let play_index = candidates[idx].play_index;
                        let value = rollout_objective(
                            &mut self.simmer,
                            self.game_config,
                            self.kwg,
                            self.klv,
                            &move_generator.plays[play_index].play,
                        );
                        candidates[idx].stats.update(value);
                    }
                }
            }
            if sim_iter % PRUNE_CADENCE == 0 {
                let low_bar = candidates
                    .iter()
                    .map(|candidate| candidate.stats.ci_max(-Z))
                    .max_by(|a, b| a.total_cmp(b))
                    .unwrap();
                retire_below(&mut candidates, &mut retired, Z, low_bar);
                // Hard cap on survivors that tightens as the fixed budget is
                // spent, so the field narrows toward a single winner by the last
                // iteration even when candidates are so close that the
                // confidence-interval prune above cannot separate them.
                let prune_periods_remaining = budget.saturating_sub(sim_iter) / PRUNE_CADENCE;
                limit_surviving_candidates(
                    &mut candidates,
                    &mut retired,
                    Z,
                    1 + 2 * prune_periods_remaining as usize,
                );
                if candidates.len() < 2 {
                    break;
                }
                // Confidence stop: once the leader is separated from the whole
                // field at the corrected z, the rest of the budget is only a
                // safety cap, so stop early.
                if self.stop_rule == StopRule::Confidence
                    && leader_is_separated(&candidates, self.stop_delta)
                {
                    break;
                }
            }
        }
        self.candidates = candidates;
        self.retired = retired;
    }

    // Study entry point: prepare the position, reset the candidate set and its
    // retired side list, and run `iters` rollout iterations WITHOUT committing a
    // winner, so the caller can inspect the leader or resume with more. The
    // caller must have populated move_generator.plays (via gen_moves) first.
    pub fn begin_decision(
        &mut self,
        move_generator: &movegen::KurniaMoveGenerator,
        game_state: &game_state::GameState,
        iters: u64,
    ) {
        self.simmer.prepare(self.game_config, game_state, 2);
        self.candidates = self.take_candidates(move_generator.plays.len());
        self.next_stream_id = self.candidates.len() as u64;
        self.retired.clear();
        self.iters_done = 0;
        let budget = self.num_sim_iters;
        self.run_iterations(move_generator, budget, iters);
    }

    // Study continuation: run `extra_iters` more rollouts on the retained
    // candidate set, continuing the same rollout stream (no reseed) so the
    // result matches having run the larger budget in one begin_decision call.
    // Does not commit a winner.
    pub fn resume(&mut self, move_generator: &movegen::KurniaMoveGenerator, extra_iters: u64) {
        let budget = self.num_sim_iters;
        self.run_iterations(move_generator, budget, extra_iters);
    }

    // The current leader's play index, mean objective, and sample count, read
    // without committing or stopping -- for study sessions and self-checks.
    pub fn leader_summary(&self) -> (usize, f64, f64) {
        let leader = self
            .candidates
            .iter()
            .max_by(|a, b| a.stats.mean().total_cmp(&b.stats.mean()))
            .unwrap();
        (leader.play_index, leader.stats.mean(), leader.stats.count())
    }

    // Add an already-generated play (by its index in move_generator.plays) to
    // the working set as a fresh candidate with zero samples, so a study session
    // can bring an unselected move into contention. Returns its stream id.
    pub fn add_play(&mut self, play_index: usize) -> u64 {
        let stream_id = self.next_stream_id;
        self.next_stream_id += 1;
        self.candidates.push(Candidate {
            play_index,
            stats: stats::Stats::new(),
            stream_id,
        });
        stream_id
    }

    // Readmit a retired candidate to the working set WITH its accumulated
    // statistics, named by its stream id. Returns true if it was found.
    pub fn readmit_with_history(&mut self, stream_id: u64) -> bool {
        if let Some(pos) = self.retired.iter().position(|c| c.stream_id == stream_id) {
            let candidate = self.retired.swap_remove(pos);
            self.candidates.push(candidate);
            true
        } else {
            false
        }
    }

    // Readmit a retired candidate's play with FRESH (zeroed) statistics, named
    // by its stream id: the play returns to contention but its old rollouts are
    // dropped. Returns true if it was found.
    pub fn readmit_fresh(&mut self, stream_id: u64) -> bool {
        if let Some(pos) = self.retired.iter().position(|c| c.stream_id == stream_id) {
            let play_index = self.retired.swap_remove(pos).play_index;
            self.add_play(play_index);
            true
        } else {
            false
        }
    }

    // The stream ids currently retired, so a study session can pick one to
    // readmit.
    pub fn retired_stream_ids(&self) -> impl Iterator<Item = u64> + '_ {
        self.retired.iter().map(|c| c.stream_id)
    }

    // The sample count of the candidate with this stream id, whether it is
    // active or retired, or None if there is no such candidate.
    pub fn stream_count(&self, stream_id: u64) -> Option<f64> {
        self.candidates
            .iter()
            .chain(self.retired.iter())
            .find(|c| c.stream_id == stream_id)
            .map(|c| c.stats.count())
    }

    // Anytime query: read the run's state between resume calls without
    // disturbing it -- pause, inspect the current leader, decide whether to
    // keep going or stop, then resume or commit.

    // The play index of the current leader (highest mean), read WITHOUT stopping
    // or committing -- the anytime query-best.
    pub fn best_so_far(&self) -> usize {
        top_candidate_play_index_by_mean(&self.candidates)
    }

    // Whether the leader is already confidently separated from the field at the
    // current stop_delta, i.e. whether the confidence stop would fire now, so a
    // manager can move on without spending the rest of the budget. No side
    // effect.
    pub fn is_decided(&self) -> bool {
        leader_is_separated(&self.candidates, self.stop_delta)
    }
}

#[inline(always)]
fn top_candidate_play_index_by_mean(candidates: &[Candidate]) -> usize {
    candidates
        .iter()
        .max_by(|a, b| a.stats.mean().total_cmp(&b.stats.mean()))
        .unwrap()
        .play_index
}

pub struct Periods(pub u64);

impl Periods {
    #[inline(always)]
    pub fn update(&mut self, new_periods: u64) -> bool {
        if new_periods != self.0 {
            self.0 = new_periods;
            true
        } else {
            false
        }
    }
}

#[expect(clippy::large_enum_variant)]
pub enum MovePicker<'a, N: kwg::Node, L: kwg::Node> {
    Hasty,
    Simmer(Simmer<'a, N, L>),
}

impl<N: kwg::Node, L: kwg::Node> MovePicker<'_, N, L> {
    #[inline(always)]
    pub fn pick_a_move(
        &mut self,
        filtered_movegen: &mut move_filter::GenMoves<'_>,
        move_generator: &mut movegen::KurniaMoveGenerator,
        board_snapshot: &movegen::BoardSnapshot<'_, N, L>,
        game_state: &game_state::GameState,
        rack: &[u8],
    ) {
        match self {
            MovePicker::Hasty => {
                filtered_movegen.gen_moves(
                    move_generator,
                    board_snapshot,
                    rack,
                    game_state.current_player().num_exchanges,
                    1,
                );
            }
            MovePicker::Simmer(simmer) => {
                filtered_movegen.gen_moves(
                    move_generator,
                    board_snapshot,
                    rack,
                    game_state.current_player().num_exchanges,
                    100,
                );
                let budget = simmer.num_sim_iters;
                simmer.begin_decision(move_generator, game_state, budget);
                let winner_play_index = top_candidate_play_index_by_mean(&simmer.candidates);
                if simmer.verbose {
                    const Z: f64 = 1.96; // 95% confidence interval
                    let leader = simmer
                        .candidates
                        .iter()
                        .find(|candidate| candidate.play_index == winner_play_index)
                        .unwrap();
                    println!(
                        "top candidate mean = {} (sd={} count={} range {}..{})",
                        leader.stats.mean(),
                        leader.stats.standard_deviation(),
                        leader.stats.count(),
                        leader.stats.ci_max(-Z),
                        leader.stats.ci_max(Z),
                    );
                }
                move_generator.plays.swap(0, winner_play_index);
                move_generator.plays.truncate(1);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stats_from(values: &[f64]) -> stats::Stats {
        let mut s = stats::Stats::new();
        for &v in values {
            s.update(v);
        }
        s
    }

    fn candidate_from(play_index: usize, values: &[f64]) -> Candidate {
        Candidate {
            play_index,
            stats: stats_from(values),
            stream_id: play_index as u64,
        }
    }

    #[test]
    fn fwer_z_matches_the_gaussian_union_bound() {
        // two survivors at delta 0.05: z = sqrt(2 ln(1 / 0.05)) = sqrt(2 ln 20).
        let expected = (2.0 * (1.0f64 / 0.05).ln()).sqrt();
        assert!((fwer_z(2, 0.05) - expected).abs() < 1e-12);
        // more arms -> a larger z, correcting for more comparisons.
        assert!(fwer_z(100, 0.05) > fwer_z(2, 0.05));
    }

    #[test]
    fn leader_is_separated_only_when_the_field_is_cleared() {
        // leader mean 20 (tight) vs one rival mean 10 (tight): clearly separated.
        let separated = vec![
            candidate_from(0, &[19.0, 21.0].repeat(50)),
            candidate_from(1, &[9.0, 11.0].repeat(50)),
        ];
        assert!(leader_is_separated(&separated, 0.05));
        // leader mean 10.1 vs rival 10.0 with overlapping intervals: not separated.
        let overlapping = vec![
            candidate_from(0, &[9.1, 11.1].repeat(50)),
            candidate_from(1, &[9.0, 11.0].repeat(50)),
        ];
        assert!(!leader_is_separated(&overlapping, 0.05));
        // a single survivor is trivially separated.
        let one = vec![candidate_from(0, &[10.0, 10.0])];
        assert!(leader_is_separated(&one, 0.05));
    }

    #[test]
    fn retire_below_keeps_survivors_in_order() {
        // means 20, 1, 20: a low_bar of 10 drops the middle one and keeps the
        // outer two in their original order; the dropped one lands in retired.
        let mut candidates = vec![
            candidate_from(0, &[20.0, 20.0].repeat(50)),
            candidate_from(1, &[1.0, 1.0].repeat(50)),
            candidate_from(2, &[19.0, 21.0].repeat(50)),
        ];
        let mut retired = Vec::new();
        retire_below(&mut candidates, &mut retired, 1.96, 10.0);
        assert_eq!(
            candidates.iter().map(|c| c.play_index).collect::<Vec<_>>(),
            vec![0, 2]
        );
        assert_eq!(retired.len(), 1);
        assert_eq!(retired[0].play_index, 1);
    }
}
