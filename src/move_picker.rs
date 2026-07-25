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

#[derive(Clone)]
pub struct FieldShape {
    pub max_gen: usize,
    pub nonplacing_quota: usize,
}

pub const SIMMER_FIELD: FieldShape = FieldShape {
    max_gen: 100,
    nonplacing_quota: 5,
};

#[inline(always)]
fn places_no_tiles(play: &movegen::Play) -> bool {
    match play {
        movegen::Play::Exchange { .. } => true,
        movegen::Play::Place { .. } => false,
    }
}

#[inline]
pub fn gen_simmer_candidates<N: kwg::Node, L: kwg::Node>(
    filtered_movegen: &mut move_filter::GenMoves<'_>,
    move_generator: &mut movegen::KurniaMoveGenerator,
    board_snapshot: &movegen::BoardSnapshot<'_, N, L>,
    rack: &[u8],
    num_exchanges_by_this_player: i16,
    field: FieldShape,
    scratch: &mut Vec<movegen::ValuedMove>,
) {
    let max_gen = field.max_gen;
    filtered_movegen.gen_moves(
        move_generator,
        board_snapshot,
        rack,
        num_exchanges_by_this_player,
        max_gen,
    );
    let quota = field.nonplacing_quota.min(max_gen);
    let mut held = move_generator
        .plays
        .iter()
        .filter(|valued_move| places_no_tiles(&valued_move.play))
        .count();
    if held >= quota {
        return;
    }
    let pass_policy =
        if move_generator.num_tiles_in_bag() < board_snapshot.game_config.exchange_tile_limit() {
            movegen::PassPolicy::AsACandidate
        } else {
            movegen::PassPolicy::OnlyWhenForced
        };
    std::mem::swap(&mut move_generator.plays, scratch);
    filtered_movegen.gen_nonplacing_moves(
        move_generator,
        board_snapshot,
        rack,
        num_exchanges_by_this_player,
        quota,
        pass_policy,
    );
    let mut held_any = false;
    for candidate in move_generator.plays.drain(..) {
        if held >= quota {
            break;
        }
        if scratch
            .iter()
            .any(|valued_move| valued_move.play == candidate.play)
        {
            continue;
        }
        if scratch.len() < max_gen {
            scratch.push(candidate);
        } else if let Some(idx) = scratch
            .iter()
            .rposition(|valued_move| !places_no_tiles(&valued_move.play))
        {
            scratch[idx] = candidate;
        } else {
            break;
        }
        held += 1;
        held_any = true;
    }
    if held_any {
        scratch.sort_unstable();
    }
    std::mem::swap(&mut move_generator.plays, scratch);
}

// an iteration's draw depends only on (decision seed, iteration index), which is
// what makes the parallel result independent of the thread count.
#[inline(always)]
fn mix(decision_seed: u64, sim_iter: u64) -> u64 {
    let mut z = decision_seed.wrapping_add(sim_iter.wrapping_mul(0x9E37_79B9_7F4A_7C15));
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

const PRUNE_CADENCE: u64 = 16;

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
    let objective = simmer::sim_objective(sim_spread, win_prob, simmer.win_prob_weightage());

    (objective, sim_spread, win_prob)
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
pub struct Simmer<'a, N: kwg::Node, L: kwg::Node, const OBSERVE: bool = false> {
    game_config: &'a game_config::GameConfig,
    kwg: &'a kwg::Kwg<N>,
    klv: &'a klv::Klv<L>,
    candidates: Vec<Candidate>,
    simmer: simmer::Simmer,
    num_sim_iters: u64,
    retired: Vec<Candidate>,
    iters_done: u64,
    next_stream_id: u64,
    win_pct_table: Option<&'a win_pct::WinPctTable>,
    #[cfg(not(target_family = "wasm"))]
    sim_threads: usize,
    decision_seed: u64,
    candidate_scratch: Vec<movegen::ValuedMove>,
    prepared_pristine: game_state::GameState,
}

pub struct SimmerParams<'a> {
    pub num_sim_iters: u64,
    pub sim_threads: usize,
    pub win_pct_table: Option<&'a win_pct::WinPctTable>,
}

impl<'a, N: kwg::Node, L: kwg::Node, const OBSERVE: bool> Simmer<'a, N, L, OBSERVE> {
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
            simmer: simmer::Simmer::new(game_config),
            num_sim_iters: params.num_sim_iters,
            retired: Vec::new(),
            iters_done: 0,
            next_stream_id: 0,
            win_pct_table: params.win_pct_table,
            #[cfg(not(target_family = "wasm"))]
            sim_threads: params.sim_threads,
            decision_seed: rand::random(),
            candidate_scratch: Vec::new(),
            prepared_pristine: game_state::GameState::new(game_config),
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
    pub fn retire_stream(&mut self, stream_id: u64) -> bool {
        if self.candidates.len() < 2 {
            return false;
        }
        if let Some(pos) = self
            .candidates
            .iter()
            .position(|candidate| candidate.stream_id == stream_id)
        {
            let candidate = self.candidates.remove(pos);
            self.retired.push(candidate);
            true
        } else {
            false
        }
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

    #[inline(always)]
    pub fn active_stream_ids(&self) -> impl Iterator<Item = u64> + '_ {
        self.candidates.iter().map(|c| c.stream_id)
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

impl<'a, N: kwg::Node + Sync, L: kwg::Node + Sync, const OBSERVE: bool> Simmer<'a, N, L, OBSERVE> {
    #[inline]
    fn run_iterations(
        &mut self,
        move_generator: &movegen::KurniaMoveGenerator,
        budget: u64,
        count: u64,
    ) {
        if self.candidates.len() < 2 {
            return;
        }

        #[cfg(not(target_family = "wasm"))]
        if self.sim_threads > 1 {
            self.run_iterations_parallel(move_generator, budget, count);
            return;
        }
        let mut candidates = std::mem::take(&mut self.candidates);
        let mut retired = std::mem::take(&mut self.retired);
        const Z: f64 = 1.96; // 95% confidence interval
        let start = self.iters_done;
        for sim_iter in (start + 1)..=(start + count) {
            self.iters_done = sim_iter;
            self.simmer.restore_prepared(&self.prepared_pristine);
            self.simmer.reseed(mix(self.decision_seed, sim_iter));
            self.simmer.prepare_iteration();
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
                if OBSERVE {
                    candidate
                        .equity_stats
                        .update(simmer::spread_points(sim_spread));
                    candidate.win_rate_stats.update(win_prob);
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
            }
        }
        self.simmer.restore_prepared(&self.prepared_pristine);
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
                        if OBSERVE {
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
                                if OBSERVE {
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
            if OBSERVE {
                block_equity.reserve(block_len * num_candidates);
                block_win_rate.reserve(block_len * num_candidates);
            }
            for (objective, equity, win_rate) in thread_rows {
                block_objective.extend(objective);
                if OBSERVE {
                    block_equity.extend(equity);
                    block_win_rate.extend(win_rate);
                }
            }
            for (candidate_index, candidate) in candidates.iter_mut().enumerate() {
                for iteration in 0..block_len {
                    let k = iteration * num_candidates + candidate_index;
                    candidate.stats.update(block_objective[k]);
                    if OBSERVE {
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
            .prepare(self.game_config, game_state, 2, OBSERVE);
        self.prepared_pristine
            .clone_from(self.simmer.prepared_state());
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
                let mut scratch = std::mem::take(&mut simmer.candidate_scratch);
                gen_simmer_candidates(
                    filtered_movegen,
                    move_generator,
                    board_snapshot,
                    rack,
                    game_state.current_player().num_exchanges,
                    SIMMER_FIELD,
                    &mut scratch,
                );
                simmer.candidate_scratch = scratch;
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
    use crate::build;

    #[inline]
    fn tiny_lexicon() -> Vec<u8> {
        const LETTERS: [u8; 3] = [1, 5, 20]; // A, E, T
        let mut words = Vec::new();
        for &a in &LETTERS {
            for &b in &LETTERS {
                words.push([a, b][..].into());
                for &c in &LETTERS {
                    words.push([a, b, c][..].into());
                }
            }
        }
        words.sort_unstable();
        build::build(
            build::BuildContent::Gaddawg,
            build::BuildLayout::Wolges,
            &words,
        )
        .unwrap()
        .to_vec()
    }

    #[inline]
    fn count_nonplacing(plays: &[movegen::ValuedMove]) -> usize {
        plays
            .iter()
            .filter(|valued_move| places_no_tiles(&valued_move.play))
            .count()
    }

    #[inline]
    fn field_for(
        game_config: &game_config::GameConfig,
        kwg_bytes: &[u8],
        klv_bytes: &[u8],
        board_tiles: &[u8],
        rack: &[u8],
        field: FieldShape,
    ) -> Vec<movegen::ValuedMove> {
        let kwg = kwg::Kwg::<kwg::Node22>::from_bytes_alloc(kwg_bytes);
        let klv = klv::Klv::<kwg::Node22>::from_bytes_alloc(klv_bytes);
        let board_snapshot = movegen::BoardSnapshot {
            board_tiles,
            game_config,
            kwg: &kwg,
            klv: &klv,
        };
        let mut move_generator = movegen::KurniaMoveGenerator::new(game_config);
        let mut filtered_movegen = move_filter::GenMoves::Unfiltered;
        let mut scratch = Vec::new();
        gen_simmer_candidates(
            &mut filtered_movegen,
            &mut move_generator,
            &board_snapshot,
            rack,
            0,
            field,
            &mut scratch,
        );
        move_generator.plays
    }

    #[inline]
    fn board_with(
        game_config: &game_config::GameConfig,
        island_row: usize,
        num_dead: usize,
    ) -> Vec<u8> {
        let dim = game_config.board_layout().dim();
        let cols = dim.cols as usize;
        let mut board_tiles = vec![0u8; dim.rows as usize * cols];
        for slot in board_tiles.iter_mut().take(num_dead) {
            *slot = 26; // Z
        }
        board_tiles[island_row * cols + 5] = 1; // A
        board_tiles[island_row * cols + 6] = 20; // T
        board_tiles
    }

    #[test]
    #[inline]
    fn the_field_keeps_room_for_moves_that_place_no_tiles() {
        let game_config = game_config::make_english_game_config();
        let kwg_bytes = tiny_lexicon();
        let klv_bytes = klv::make_klv2(&[(&[1, 5], 0.5)]);
        let board_tiles = board_with(&game_config, 7, 0);
        let rack = [1u8, 1, 5, 5, 20, 20, 20]; // AAEETTT

        let unheld = field_for(
            &game_config,
            &kwg_bytes,
            &klv_bytes,
            &board_tiles,
            &rack,
            FieldShape {
                max_gen: 8,
                nonplacing_quota: 0,
            },
        );
        assert_eq!(unheld.len(), 8);
        assert_eq!(
            count_nonplacing(&unheld),
            0,
            "the tile plays should crowd the class out; without that this test proves nothing",
        );

        let held = field_for(
            &game_config,
            &kwg_bytes,
            &klv_bytes,
            &board_tiles,
            &rack,
            FieldShape {
                max_gen: 8,
                nonplacing_quota: 3,
            },
        );
        assert_eq!(held.len(), 8, "the quota displaces, it does not extend");
        assert_eq!(count_nonplacing(&held), 3);
        let best_nonplacing = held
            .iter()
            .find(|valued_move| places_no_tiles(&valued_move.play))
            .expect("the class is represented");
        match &best_nonplacing.play {
            movegen::Play::Exchange { tiles } => {
                let mut kept = rack.to_vec();
                for &tile in &tiles[..] {
                    kept.remove(kept.iter().position(|&t| t == tile).unwrap());
                }
                assert_eq!(kept, [1, 5], "should keep AE");
            }
            movegen::Play::Place { .. } => panic!("that is a tile play"),
        }
        let worst_place_kept = held
            .iter()
            .filter(|valued_move| !places_no_tiles(&valued_move.play))
            .map(|valued_move| valued_move.equity)
            .min()
            .unwrap();
        let dropped = unheld
            .iter()
            .filter(|valued_move| !held.iter().any(|kept| kept.play == valued_move.play))
            .map(|valued_move| valued_move.equity)
            .max()
            .unwrap();
        assert!(worst_place_kept >= dropped);
    }

    #[test]
    #[inline]
    fn the_field_holds_a_pass_once_the_bag_is_too_short_to_exchange() {
        let game_config = game_config::make_english_game_config();
        let kwg_bytes = tiny_lexicon();
        let klv_bytes = klv::make_klv2(&[(&[1, 5], 0.5)]);
        let board_tiles = board_with(&game_config, 12, 80);
        let rack = [1u8, 1, 5, 5, 20, 20, 20];
        let field = field_for(
            &game_config,
            &kwg_bytes,
            &klv_bytes,
            &board_tiles,
            &rack,
            FieldShape {
                max_gen: 8,
                nonplacing_quota: 3,
            },
        );
        assert_eq!(field.len(), 8, "the board should still offer tile plays");
        let passes = field
            .iter()
            .filter(|valued_move| match &valued_move.play {
                movegen::Play::Exchange { tiles } => tiles.is_empty(),
                movegen::Play::Place { .. } => false,
            })
            .count();
        assert_eq!(passes, 1, "exactly one pass, held by the quota");
        assert_eq!(
            count_nonplacing(&field),
            1,
            "and nothing else, since no exchange is legal",
        );
    }

    #[test]
    #[inline]
    fn a_field_that_already_has_the_class_is_left_alone() {
        let game_config = game_config::make_english_game_config();
        let kwg_bytes = tiny_lexicon();
        let klv_bytes = klv::make_klv2(&[(&[1, 5], 0.5)]);
        let board_tiles = board_with(&game_config, 7, 0);
        let rack = [1u8, 1, 5, 5, 20, 20, 20];
        let wide = field_for(
            &game_config,
            &kwg_bytes,
            &klv_bytes,
            &board_tiles,
            &rack,
            FieldShape {
                max_gen: 1_000,
                nonplacing_quota: 0,
            },
        );
        assert!(count_nonplacing(&wide) >= 3);
        let with_quota = field_for(
            &game_config,
            &kwg_bytes,
            &klv_bytes,
            &board_tiles,
            &rack,
            FieldShape {
                max_gen: 1_000,
                nonplacing_quota: 3,
            },
        );
        assert_eq!(
            wide.len(),
            with_quota.len(),
            "nothing to hold, so nothing changes",
        );
        for (a, b) in wide.iter().zip(with_quota.iter()) {
            assert!(a.play == b.play);
        }
    }

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
