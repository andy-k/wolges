// Copyright (C) 2020-2026 Andy Kurnia.

use super::{game_config, game_state, klv, kwg, move_filter, movegen, simmer, stats};

struct Candidate {
    play_index: usize,
    stats: stats::Stats,
}

// Default per-decision rollout budget (see Simmer::num_sim_iters).
const DEFAULT_NUM_SIM_ITERS: u64 = 1000;

// Prune the candidate list once every this many iterations. Between prunes
// each surviving candidate gets one more rollout, so this trades the cost of
// pruning against how quickly clearly-worse candidates are dropped.
const PRUNE_CADENCE: u64 = 16;

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
            });
        }
        candidates
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
    fn limit_surviving_candidates(
        candidates: &mut Vec<Candidate>,
        z: f64,
        max_candidates_allowed: usize,
    ) {
        while candidates.len() > max_candidates_allowed {
            // pruning regularly means binary heap is not justified here.
            // also, this means candidates are almost indistinguishable.
            candidates.swap_remove(
                candidates
                    .iter()
                    .enumerate()
                    .map(|(i, candidate)| (i, candidate.stats.ci_max(-z)))
                    .min_by(|(_, a), (_, b)| a.total_cmp(b))
                    .unwrap()
                    .0,
            );
        }
    }

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
                simmer.simmer.prepare(simmer.game_config, game_state, 2);
                let mut candidates = simmer.take_candidates(move_generator.plays.len());
                let num_sim_iters = simmer.num_sim_iters;
                const Z: f64 = 1.96; // 95% confidence interval
                for sim_iter in 1..=num_sim_iters {
                    simmer.simmer.prepare_iteration();
                    for candidate in candidates.iter_mut() {
                        let game_ended = simmer.simmer.simulate(
                            simmer.game_config,
                            simmer.kwg,
                            simmer.klv,
                            &move_generator.plays[candidate.play_index].play,
                        );
                        let final_spread = simmer.simmer.final_equity_spread();
                        let win_prob = simmer.simmer.compute_win_prob(game_ended, final_spread);
                        let sim_spread = final_spread - simmer.simmer.initial_score_spread;
                        candidate.stats.update(simmer::sim_objective(
                            sim_spread,
                            win_prob,
                            simmer.simmer.win_prob_weightage(),
                            simmer.simmer.config().descale,
                        ));
                    }
                    if sim_iter % PRUNE_CADENCE == 0 {
                        let low_bar = candidates
                            .iter()
                            .map(|candidate| candidate.stats.ci_max(-Z))
                            .max_by(|a, b| a.total_cmp(b))
                            .unwrap();
                        candidates.retain(|candidate| candidate.stats.ci_max(Z) >= low_bar);
                        // Hard cap on survivors that shrinks as the fixed budget
                        // is spent, so the field narrows toward a single winner by
                        // the last iteration even when candidates are so close that
                        // the confidence-interval prune above cannot separate them.
                        let prune_periods_remaining = (num_sim_iters - sim_iter) / PRUNE_CADENCE;
                        Self::limit_surviving_candidates(
                            &mut candidates,
                            Z,
                            1 + 2 * prune_periods_remaining as usize,
                        );
                        if candidates.len() < 2 {
                            break;
                        }
                    }
                }
                let top_idx = candidates
                    .iter()
                    .enumerate()
                    .max_by(|(_, a), (_, b)| a.stats.mean().total_cmp(&b.stats.mean()))
                    .unwrap()
                    .0;
                if simmer.verbose {
                    println!(
                        "top candidate mean = {} (sd={} count={} range {}..{})",
                        candidates[top_idx].stats.mean(),
                        candidates[top_idx].stats.standard_deviation(),
                        candidates[top_idx].stats.count(),
                        candidates[top_idx].stats.ci_max(-Z),
                        candidates[top_idx].stats.ci_max(Z),
                    );
                }
                assert_eq!(
                    candidates[top_idx].play_index,
                    top_candidate_play_index_by_mean(&candidates)
                );
                move_generator
                    .plays
                    .swap(0, top_candidate_play_index_by_mean(&candidates));
                move_generator.plays.truncate(1);
                simmer.candidates = candidates;
            }
        }
    }
}
