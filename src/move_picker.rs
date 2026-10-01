// Copyright (C) 2020-2026 Andy Kurnia.

use super::{game_config, game_state, klv, kwg, move_filter, movegen, simmer, stats, win_pct};

struct Candidate {
    play_index: usize,
    stats: stats::Stats,
    stream_id: u64,
    equity_stats: stats::Stats,
    win_rate_stats: stats::Stats,
}

pub const DEFAULT_NUM_SIM_ITERS: u64 = 1000;

// an iteration's draw depends only on (decision seed, iteration index), which is
// what makes the parallel result independent of the thread count.
#[cfg(not(target_family = "wasm"))]
#[inline(always)]
fn mix(decision_seed: u64, sim_iter: u64) -> u64 {
    let mut z = decision_seed.wrapping_add(sim_iter.wrapping_mul(0x9E37_79B9_7F4A_7C15));
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

const PRUNE_CADENCE: u64 = 16;

const DEFAULT_STOP_DELTA: f64 = 0.05;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Allocator {
    RoundRobin,

    Adaptive,
}

#[inline(always)]
fn rollout_objective<N: kwg::Node, L: kwg::Node>(
    simmer: &mut simmer::Simmer,
    game_config: &game_config::GameConfig,
    kwg: &kwg::Kwg<N>,
    klv: &klv::Klv<L>,
    play: &movegen::Play,
    table: Option<&win_pct::WinPctTable>,
) -> (f64, i32, f64) {
    let game_ended = simmer.simulate(game_config, kwg, klv, play);
    let final_spread = simmer.final_equity_spread();
    let win_prob = if simmer.wants_win_prob() {
        simmer.compute_win_prob(game_ended, final_spread, table)
    } else {
        0.0
    };
    let sim_spread = final_spread - simmer.initial_score_spread;
    let objective = simmer::sim_objective(
        sim_spread,
        win_prob,
        simmer.win_prob_weightage(),
        simmer.config().descale,
    );

    (objective, sim_spread, win_prob)
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum StopRule {
    FixedCap,

    Confidence,
}

#[inline(always)]
fn fwer_z(num_survivors: usize, delta: f64) -> f64 {
    (2.0 * ((num_survivors - 1) as f64 / delta).ln()).sqrt()
}

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

#[inline(always)]
fn retire_below(
    candidates: &mut Vec<Candidate>,
    retired: &mut Vec<Candidate>,
    z: f64,
    low_bar: f64,
) {
    let mut write = 0;
    for read in 0..candidates.len() {
        if candidates[read].stats.ci_max(z) >= low_bar {
            candidates.swap(write, read);
            write += 1;
        }
    }
    retired.extend(candidates.drain(write..));
}

#[inline(always)]
fn limit_surviving_candidates(
    candidates: &mut Vec<Candidate>,
    retired: &mut Vec<Candidate>,
    z: f64,
    max_candidates_allowed: usize,
) {
    while candidates.len() > max_candidates_allowed {
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
    num_sim_iters: u64,
    allocator: Allocator,
    stop_rule: StopRule,
    stop_delta: f64,
    retired: Vec<Candidate>,
    iters_done: u64,
    next_stream_id: u64,
    observe: bool,
    win_pct_table: Option<&'a win_pct::WinPctTable>,
    #[cfg(not(target_family = "wasm"))]
    sim_threads: usize,
    decision_seed: u64,
}

pub struct SimmerParams<'a> {
    pub num_sim_iters: u64,
    pub allocator: Allocator,
    pub stop_rule: StopRule,
    pub stop_delta: Option<f64>,
    pub observe: bool,
    pub sim_threads: usize,
    pub win_pct_table: Option<&'a win_pct::WinPctTable>,
    pub config: simmer::SimmerConfig,
}

impl<'a, N: kwg::Node, L: kwg::Node> Simmer<'a, N, L> {
    pub fn new(
        game_config: &'a game_config::GameConfig,
        kwg: &'a kwg::Kwg<N>,
        klv: &'a klv::Klv<L>,
        params: SimmerParams<'a>,
    ) -> Self {
        Self {
            game_config,
            kwg,
            klv,
            candidates: Vec::new(),
            simmer: simmer::Simmer::new(game_config, params.config),
            num_sim_iters: params.num_sim_iters,
            allocator: params.allocator,
            stop_rule: params.stop_rule,
            stop_delta: params
                .stop_delta
                .unwrap_or(DEFAULT_STOP_DELTA)
                .clamp(f64::MIN_POSITIVE, 1.0 - f64::EPSILON),
            retired: Vec::new(),
            iters_done: 0,
            next_stream_id: 0,
            observe: params.observe,
            win_pct_table: params.win_pct_table,
            #[cfg(not(target_family = "wasm"))]
            sim_threads: params.sim_threads,
            decision_seed: 0,
        }
    }

    #[inline(always)]
    pub fn reseed(&mut self, seed: u64) {
        self.simmer.reseed(seed);

        self.decision_seed = seed;
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
                equity_stats: stats::Stats::new(),
                win_rate_stats: stats::Stats::new(),
            });
        }
        candidates
    }

    #[inline]
    pub fn leader_summary(&self) -> (usize, f64, f64) {
        let leader = self
            .candidates
            .iter()
            .max_by(|a, b| a.stats.mean().total_cmp(&b.stats.mean()))
            .unwrap();
        (leader.play_index, leader.stats.mean(), leader.stats.count())
    }

    #[inline]
    pub fn leaderboard(&self, top_n: usize) -> Vec<(usize, f64, f64, f64)> {
        let mut all: Vec<&Candidate> = self.candidates.iter().chain(self.retired.iter()).collect();
        all.sort_unstable_by(|a, b| b.stats.mean().total_cmp(&a.stats.mean()));
        all.iter()
            .take(top_n)
            .map(|c| {
                (
                    c.play_index,
                    c.stats.mean(),
                    c.equity_stats.mean(),
                    c.win_rate_stats.mean(),
                )
            })
            .collect()
    }

    #[inline]
    pub fn add_play(&mut self, play_index: usize) -> u64 {
        let stream_id = self.next_stream_id;
        self.next_stream_id += 1;
        self.candidates.push(Candidate {
            play_index,
            stats: stats::Stats::new(),
            stream_id,
            equity_stats: stats::Stats::new(),
            win_rate_stats: stats::Stats::new(),
        });
        stream_id
    }

    #[inline]
    pub fn readmit_with_history(&mut self, stream_id: u64) -> bool {
        if let Some(pos) = self.retired.iter().position(|c| c.stream_id == stream_id) {
            let candidate = self.retired.swap_remove(pos);
            self.candidates.push(candidate);
            true
        } else {
            false
        }
    }

    #[inline]
    pub fn readmit_fresh(&mut self, stream_id: u64) -> bool {
        if let Some(pos) = self.retired.iter().position(|c| c.stream_id == stream_id) {
            let play_index = self.retired.swap_remove(pos).play_index;
            self.add_play(play_index);
            true
        } else {
            false
        }
    }

    #[inline(always)]
    pub fn retired_stream_ids(&self) -> impl Iterator<Item = u64> + '_ {
        self.retired.iter().map(|c| c.stream_id)
    }

    #[inline]
    pub fn stream_count(&self, stream_id: u64) -> Option<f64> {
        self.candidates
            .iter()
            .chain(self.retired.iter())
            .find(|c| c.stream_id == stream_id)
            .map(|c| c.stats.count())
    }

    #[inline(always)]
    pub fn best_so_far(&self) -> usize {
        top_candidate_play_index_by_mean(&self.candidates)
    }

    #[inline(always)]
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

impl<'a, N: kwg::Node + Sync, L: kwg::Node + Sync> Simmer<'a, N, L> {
    #[inline]
    fn run_iterations(
        &mut self,
        move_generator: &movegen::KurniaMoveGenerator,
        budget: u64,
        count: u64,
    ) {
        #[cfg(not(target_family = "wasm"))]
        if self.sim_threads > 1 && self.allocator == Allocator::RoundRobin {
            self.run_iterations_parallel(move_generator, budget, count);
            return;
        }
        let mut candidates = std::mem::take(&mut self.candidates);
        let mut retired = std::mem::take(&mut self.retired);
        const Z: f64 = 1.96; // 95% confidence interval
        let start = self.iters_done;
        for sim_iter in (start + 1)..=(start + count) {
            self.iters_done = sim_iter;
            self.simmer.prepare_iteration();
            // until the first prune every candidate must gather samples, or the prune sees
            // count-zero arms whose interval is NaN and empties the field.
            let effective_allocator = if sim_iter <= PRUNE_CADENCE {
                Allocator::RoundRobin
            } else {
                self.allocator
            };
            match effective_allocator {
                Allocator::RoundRobin => {
                    for candidate in candidates.iter_mut() {
                        let (value, sim_spread, win_prob) = rollout_objective(
                            &mut self.simmer,
                            self.game_config,
                            self.kwg,
                            self.klv,
                            &move_generator.plays[candidate.play_index].play,
                            self.win_pct_table,
                        );
                        candidate.stats.update(value);
                        if self.observe {
                            candidate
                                .equity_stats
                                .update(simmer::spread_points(sim_spread));
                            candidate.win_rate_stats.update(win_prob);
                        }
                    }
                }
                Allocator::Adaptive => {
                    let leader_idx = candidates
                        .iter()
                        .enumerate()
                        .max_by(|(_, a), (_, b)| a.stats.mean().total_cmp(&b.stats.mean()))
                        .unwrap()
                        .0;
                    let leader_mean = candidates[leader_idx].stats.mean();
                    let leader_var = candidates[leader_idx].stats.variance();
                    let leader_n = candidates[leader_idx].stats.count();

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

                    let floor_idx = candidates
                        .iter()
                        .enumerate()
                        .min_by(|(_, a), (_, b)| a.stats.count().total_cmp(&b.stats.count()))
                        .unwrap()
                        .0;

                    let mut to_sample = [leader_idx, challenger_idx, floor_idx];
                    to_sample.sort_unstable();
                    let mut prev = usize::MAX;
                    for &idx in &to_sample {
                        if idx == prev {
                            continue;
                        }
                        prev = idx;
                        let play_index = candidates[idx].play_index;
                        let (value, sim_spread, win_prob) = rollout_objective(
                            &mut self.simmer,
                            self.game_config,
                            self.kwg,
                            self.klv,
                            &move_generator.plays[play_index].play,
                            self.win_pct_table,
                        );
                        candidates[idx].stats.update(value);
                        if self.observe {
                            candidates[idx]
                                .equity_stats
                                .update(simmer::spread_points(sim_spread));
                            candidates[idx].win_rate_stats.update(win_prob);
                        }
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

    // reduce every iteration in the same order whatever the thread count: the variance
    // combine is not associative in floating point.
    #[cfg(not(target_family = "wasm"))]
    #[inline]
    fn run_iterations_parallel(
        &mut self,
        move_generator: &movegen::KurniaMoveGenerator,
        budget: u64,
        count: u64,
    ) {
        let mut candidates = std::mem::take(&mut self.candidates);
        let mut retired = std::mem::take(&mut self.retired);
        const Z: f64 = 1.96; // 95% confidence interval
        let num_threads = self.sim_threads;
        let decision_seed = self.decision_seed;
        let observe = self.observe;
        let game_config = self.game_config;
        let kwg = self.kwg;
        let klv = self.klv;
        let win_pct_table = self.win_pct_table;
        let base_simmer = &self.simmer;
        let end = self.iters_done + count;

        while self.iters_done < end {
            let block_start = self.iters_done;
            let next_boundary = (block_start / PRUNE_CADENCE + 1) * PRUNE_CADENCE;
            let block_end = end.min(next_boundary);
            let num_candidates = candidates.len();
            let block_len = (block_end - block_start) as usize;

            let play_indices: Vec<usize> = candidates
                .iter()
                .map(|candidate| candidate.play_index)
                .collect();

            let mut thread_rows: Vec<(Vec<f64>, Vec<f64>, Vec<f64>)> =
                Vec::with_capacity(num_threads);
            std::thread::scope(|scope| {
                let mut handles = Vec::with_capacity(num_threads);
                for thread_index in 0..num_threads {
                    let play_indices = &play_indices;

                    let base = block_len / num_threads;
                    let extra = block_len % num_threads;
                    let lo = thread_index * base + thread_index.min(extra);
                    let hi = lo + base + if thread_index < extra { 1 } else { 0 };
                    handles.push(scope.spawn(move || {
                        let mut simmer = base_simmer.prepared_clone(game_config);

                        let pristine = simmer.prepared_state().clone();
                        let span = hi - lo;
                        let mut objective = Vec::with_capacity(span * num_candidates);
                        let mut equity = Vec::new();
                        let mut win_rate = Vec::new();
                        if observe {
                            equity.reserve(span * num_candidates);
                            win_rate.reserve(span * num_candidates);
                        }
                        for offset in lo..hi {
                            let sim_iter = block_start + 1 + offset as u64;
                            simmer.restore_prepared(&pristine);
                            simmer.reseed(mix(decision_seed, sim_iter));
                            simmer.prepare_iteration();
                            for &play_index in play_indices.iter() {
                                let (value, sim_spread, win_prob) = rollout_objective(
                                    &mut simmer,
                                    game_config,
                                    kwg,
                                    klv,
                                    &move_generator.plays[play_index].play,
                                    win_pct_table,
                                );
                                objective.push(value);
                                if observe {
                                    equity.push(simmer::spread_points(sim_spread));
                                    win_rate.push(win_prob);
                                }
                            }
                        }
                        (objective, equity, win_rate)
                    }));
                }
                for handle in handles {
                    thread_rows.push(handle.join().unwrap());
                }
            });

            let mut block_objective: Vec<f64> = Vec::with_capacity(block_len * num_candidates);
            let mut block_equity: Vec<f64> = Vec::new();
            let mut block_win_rate: Vec<f64> = Vec::new();
            if observe {
                block_equity.reserve(block_len * num_candidates);
                block_win_rate.reserve(block_len * num_candidates);
            }
            for (objective, equity, win_rate) in thread_rows {
                block_objective.extend(objective);
                if observe {
                    block_equity.extend(equity);
                    block_win_rate.extend(win_rate);
                }
            }
            for (candidate_index, candidate) in candidates.iter_mut().enumerate() {
                for iteration in 0..block_len {
                    let k = iteration * num_candidates + candidate_index;
                    candidate.stats.update(block_objective[k]);
                    if observe {
                        candidate.equity_stats.update(block_equity[k]);
                        candidate.win_rate_stats.update(block_win_rate[k]);
                    }
                }
            }
            self.iters_done = block_end;
            if block_end.is_multiple_of(PRUNE_CADENCE) {
                let low_bar = candidates
                    .iter()
                    .map(|candidate| candidate.stats.ci_max(-Z))
                    .max_by(|a, b| a.total_cmp(b))
                    .unwrap();
                retire_below(&mut candidates, &mut retired, Z, low_bar);
                let prune_periods_remaining = budget.saturating_sub(block_end) / PRUNE_CADENCE;
                limit_surviving_candidates(
                    &mut candidates,
                    &mut retired,
                    Z,
                    1 + 2 * prune_periods_remaining as usize,
                );
                if candidates.len() < 2 {
                    break;
                }
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

    #[inline]
    pub fn begin_decision(
        &mut self,
        move_generator: &movegen::KurniaMoveGenerator,
        game_state: &game_state::GameState,
        iters: u64,
    ) {
        self.simmer
            .prepare(self.game_config, game_state, 2, self.observe);
        self.candidates = self.take_candidates(move_generator.plays.len());
        self.next_stream_id = self.candidates.len() as u64;
        self.retired.clear();
        self.iters_done = 0;
        let budget = self.num_sim_iters;
        self.run_iterations(move_generator, budget, iters);
    }

    #[inline]
    pub fn resume(&mut self, move_generator: &movegen::KurniaMoveGenerator, extra_iters: u64) {
        let budget = self.num_sim_iters;
        self.run_iterations(move_generator, budget, extra_iters);
    }
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
    ) where
        N: Sync,
        L: Sync,
    {
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
                move_generator.plays.swap(0, winner_play_index);
                move_generator.plays.truncate(1);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[inline]
    fn stats_from(values: &[f64]) -> stats::Stats {
        let mut s = stats::Stats::new();
        for &v in values {
            s.update(v);
        }
        s
    }

    #[inline]
    fn candidate_from(play_index: usize, values: &[f64]) -> Candidate {
        Candidate {
            play_index,
            stats: stats_from(values),
            stream_id: play_index as u64,
            equity_stats: stats::Stats::new(),
            win_rate_stats: stats::Stats::new(),
        }
    }

    #[test]
    #[inline]
    fn fwer_z_matches_the_gaussian_union_bound() {
        let expected = (2.0 * (1.0f64 / 0.05).ln()).sqrt();
        assert!((fwer_z(2, 0.05) - expected).abs() < 1e-12);

        assert!(fwer_z(100, 0.05) > fwer_z(2, 0.05));
    }

    #[test]
    #[inline]
    fn leader_is_separated_only_when_the_field_is_cleared() {
        let separated = vec![
            candidate_from(0, &[19.0, 21.0].repeat(50)),
            candidate_from(1, &[9.0, 11.0].repeat(50)),
        ];
        assert!(leader_is_separated(&separated, 0.05));

        let overlapping = vec![
            candidate_from(0, &[9.1, 11.1].repeat(50)),
            candidate_from(1, &[9.0, 11.0].repeat(50)),
        ];
        assert!(!leader_is_separated(&overlapping, 0.05));

        let one = vec![candidate_from(0, &[10.0, 10.0])];
        assert!(leader_is_separated(&one, 0.05));
    }

    #[test]
    #[inline]
    fn retire_below_keeps_survivors_in_order() {
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
