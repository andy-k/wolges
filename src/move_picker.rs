// Copyright (C) 2020-2026 Andy Kurnia.

use super::{
    anagrams, equity, error, fash, game_config, game_state, glance, klv, kwg, matrix, move_filter,
    movegen, simmer, stats, win_pct,
};
use rand::SeedableRng;
use std::io::Write;

struct Candidate {
    play_index: usize,
    stats: stats::Stats,
    stream_id: u64,
    equity_stats: stats::Stats,
    win_rate_stats: stats::Stats,
    first_iter: u64,
    objectives: Vec<f64>, // from first_iter on, NaN while retired
}

pub const DEFAULT_NUM_SIM_ITERS: u64 = 1000;
pub const DEFAULT_NUM_SIM_PLIES: usize = 4;
// one point
pub const DEFAULT_DEFT_WINDOW: i32 = equity::SCALE;
// how many moves deft generates to choose among
const DEFT_FIELD: usize = 8;
// and how many once the bag holds a rack or less
const DEFT_LATE_FIELD: usize = 64;
// three points: how far below its best deft takes a move that leaves exactly
// one tile in the bag
const DEFT_ONE_WINDOW: i32 = 3 * equity::SCALE;
// of the moves out in two hopes may open a second place, how many it values
// again with their leftover's real outs; and of its best moves, how many it
// values again on the board after each
const OUT_IN_TWO_OPENERS: usize = 16;
const OUT_IN_TWO_CHECKED: usize = 3;

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
        None,
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

// https://doi.org/10.1145/2714064.2660195
// https://prng.di.unimi.it/splitmix64.c
// an iteration's draw depends only on (decision seed, iteration index), which is
// what makes the parallel result independent of the thread count.
#[inline(always)]
fn mix(decision_seed: u64, sim_iter: u64) -> u64 {
    let mut z = decision_seed.wrapping_add(sim_iter.wrapping_mul(0x9e3779b97f4a7c15));
    z = (z ^ (z >> 30)).wrapping_mul(0xbf58476d1ce4e5b9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94d049bb133111eb);
    z ^ (z >> 31)
}

const PRUNE_CADENCE: u64 = 16;

#[inline(always)]
fn rollout_objective<N: kwg::Node, L: kwg::Node, R: rand::Rng + SeedableRng + simmer::CopyState>(
    simmer: &mut simmer::Simmer<R>,
    game_config: &game_config::GameConfig,
    kwg: &kwg::Kwg<N>,
    arm_tables: &anagrams::ArmTables,
    klv: &klv::Klv<L>,
    play: &movegen::Play,
    table: Option<&win_pct::WinPctTable>,
) -> error::Returns<(f64, i32, f64)> {
    let game_ended = simmer.simulate(game_config, kwg, arm_tables, klv, play)?;
    let final_spread = simmer.final_equity_spread()?;
    let win_prob = if simmer.wants_win_prob() {
        simmer.compute_win_prob(game_ended, final_spread, table)
    } else {
        0.0
    };
    let sim_spread = final_spread
        .checked_sub(simmer.initial_score_spread)
        .ok_or("score overflow")?;
    let objective = simmer::sim_objective(sim_spread, win_prob, simmer.win_prob_weightage());

    Ok((objective, sim_spread, win_prob))
}

// the standard normal distribution's quantile at p, for p in (0, 1), by
// Acklam's rational approximation, whose relative error is under 1.2e-9
#[inline]
fn normal_quantile(p: f64) -> f64 {
    const A: [f64; 6] = [
        -3.969_683_028_665_376e1,
        2.209_460_984_245_205e2,
        -2.759_285_104_469_687e2,
        1.383_577_518_672_69e2,
        -3.066_479_806_614_716e1,
        2.506_628_277_459_239,
    ];
    const B: [f64; 5] = [
        -5.447_609_879_822_406e1,
        1.615_858_368_580_409e2,
        -1.556_989_798_598_866e2,
        6.680_131_188_771_972e1,
        -1.328_068_155_288_572e1,
    ];
    const C: [f64; 6] = [
        -7.784_894_002_430_293e-3,
        -3.223_964_580_411_365e-1,
        -2.400_758_277_161_838,
        -2.549_732_539_343_734,
        4.374_664_141_464_968,
        2.938_163_982_698_783,
    ];
    const D: [f64; 4] = [
        7.784_695_709_041_462e-3,
        3.224_671_290_700_398e-1,
        2.445_134_137_142_996,
        3.754_408_661_907_416,
    ];
    const P_LOW: f64 = 0.02425;
    let tail = |q: f64| {
        (((((C[0] * q + C[1]) * q + C[2]) * q + C[3]) * q + C[4]) * q + C[5])
            / ((((D[0] * q + D[1]) * q + D[2]) * q + D[3]) * q + 1.0)
    };
    if p < P_LOW {
        tail((-2.0 * p.ln()).sqrt())
    } else if p <= 1.0 - P_LOW {
        let q = p - 0.5;
        let r = q * q;
        (((((A[0] * r + A[1]) * r + A[2]) * r + A[3]) * r + A[4]) * r + A[5]) * q
            / (((((B[0] * r + B[1]) * r + B[2]) * r + B[3]) * r + B[4]) * r + 1.0)
    } else {
        -tail((-2.0 * (1.0 - p).ln()).sqrt())
    }
}

// the one-sided z that keeps the chance of retiring the best candidate in a
// whole decision to one in twenty, split evenly over its looks (one every
// PRUNE_CADENCE iterations)
#[inline]
fn paired_z(budget: u64) -> f64 {
    let looks = (budget / PRUNE_CADENCE).max(1) as f64;
    -normal_quantile(0.05 / looks)
}

// compare each candidate with the leader (the highest mean) on the iterations
// both were rolled out: a candidate retires when its objective minus the
// leader's is below 0 even at the top of its interval. Every candidate shares
// each iteration's draws, so the difference varies much less than either
// objective.
#[inline(always)]
fn retire_paired(candidates: &mut Vec<Candidate>, retired: &mut Vec<Candidate>, z: f64) {
    let lead = candidates
        .iter()
        .max_by(|a, b| a.stats.mean().total_cmp(&b.stats.mean()))
        .unwrap();
    let below = candidates
        .iter()
        .map(|candidate| {
            let from = candidate.first_iter.max(lead.first_iter);
            let to = (candidate.first_iter + candidate.objectives.len() as u64)
                .min(lead.first_iter + lead.objectives.len() as u64);
            let mut diff = stats::Stats::new();
            for iter in from..to {
                let mine = candidate.objectives[(iter - candidate.first_iter) as usize];
                let leads = lead.objectives[(iter - lead.first_iter) as usize];
                if !mine.is_nan() && !leads.is_nan() {
                    diff.update(mine - leads);
                }
            }
            diff.count() >= 2.0 && diff.ci_max(z) < 0.0
        })
        .collect::<Vec<_>>();
    let mut write = 0;
    for (read, &is_below) in below.iter().enumerate() {
        if !is_below {
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
pub struct Simmer<
    'a,
    N: kwg::Node,
    L: kwg::Node,
    const OBSERVE: bool = false,
    R = simmer::DefaultRng,
> {
    game_config: &'a game_config::GameConfig,
    kwg: &'a kwg::Kwg<N>,
    arm_tables: &'a anagrams::ArmTables,
    klv: &'a klv::Klv<L>,
    candidates: Vec<Candidate>,
    simmer: simmer::Simmer<R>,
    num_sim_iters: u64,
    num_sim_plies: usize,
    retired: Vec<Candidate>,
    iters_done: u64,
    next_stream_id: u64,
    win_pct_table: Option<&'a win_pct::WinPctTable>,
    #[cfg(not(target_family = "wasm"))]
    sim_threads: usize,
    decision_seed: u64,
    candidate_scratch: Vec<movegen::ValuedMove>,
    prepared_pristine: game_state::GameState,
    quiet: bool,
    decision_t0: Option<std::time::Instant>, // set only when not quiet
    ticked: u64,
}

// a block of iterations, those after lo up to hi, rolled out for the plays
// still in (indices into the move generator's plays), in the order the rows
// give them
pub struct SimBlock {
    pub lo: u64,
    pub hi: u64,
    pub plays: Vec<usize>,
}

// a block's rows, by iteration and then by play: the objectives, and when
// observing, the equities and win rates
pub struct SimRows {
    pub objective: Vec<f64>,
    pub equity: Vec<f64>,
    pub win_rate: Vec<f64>,
}

impl SimRows {
    // append the rows of the iterations that follow these
    #[inline]
    pub fn extend(&mut self, later: SimRows) {
        self.objective.extend(later.objective);
        self.equity.extend(later.equity);
        self.win_rate.extend(later.win_rate);
    }
}

pub struct SimmerParams<'a> {
    pub num_sim_iters: u64,
    pub num_sim_plies: usize,
    pub sim_threads: usize,
    pub win_pct_table: Option<&'a win_pct::WinPctTable>,
    pub quiet: bool, // no progress lines
}

impl<'a, N: kwg::Node, L: kwg::Node, const OBSERVE: bool> Simmer<'a, N, L, OBSERVE> {
    pub fn new(
        game_config: &'a game_config::GameConfig,
        kwg: &'a kwg::Kwg<N>,
        arm_tables: &'a anagrams::ArmTables,
        klv: &'a klv::Klv<L>,
        params: SimmerParams<'a>,
    ) -> error::Returns<Self> {
        Ok(Self::with_rng(
            game_config,
            kwg,
            arm_tables,
            klv,
            params,
            simmer::DefaultRng::try_from_rng(&mut rand::rngs::SysRng)?,
        ))
    }
}

impl<
    'a,
    N: kwg::Node,
    L: kwg::Node,
    const OBSERVE: bool,
    R: rand::Rng + SeedableRng + simmer::CopyState,
> Simmer<'a, N, L, OBSERVE, R>
{
    pub fn with_rng(
        game_config: &'a game_config::GameConfig,
        kwg: &'a kwg::Kwg<N>,
        arm_tables: &'a anagrams::ArmTables,
        klv: &'a klv::Klv<L>,
        params: SimmerParams<'a>,
        mut rng: R,
    ) -> Self {
        let decision_seed = rng.next_u64();
        Self {
            game_config,
            kwg,
            arm_tables,
            klv,
            candidates: Vec::new(),
            simmer: simmer::Simmer::with_rng(game_config, rng),
            num_sim_iters: params.num_sim_iters,
            num_sim_plies: params.num_sim_plies,
            retired: Vec::new(),
            iters_done: 0,
            next_stream_id: 0,
            win_pct_table: params.win_pct_table,
            #[cfg(not(target_family = "wasm"))]
            sim_threads: params.sim_threads,
            decision_seed,
            candidate_scratch: Vec::new(),
            prepared_pristine: game_state::GameState::new(game_config),
            quiet: params.quiet,
            decision_t0: None,
            ticked: 0,
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
                first_iter: 1,
                objectives: Vec::new(),
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
            first_iter: self.iters_done + 1,
            objectives: Vec::new(),
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
            let mut candidate = self.retired.swap_remove(pos);
            // the iterations it sat out
            let next = candidate.first_iter + candidate.objectives.len() as u64;
            for _ in next..=self.iters_done {
                candidate.objectives.push(f64::NAN);
            }
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

// a line a second while a decision runs, when it began with a clock: its
// seconds, its iteration of the budget, and the candidates still in it
#[inline(always)]
fn tick(
    decision_t0: Option<std::time::Instant>,
    ticked: &mut u64,
    sim_iter: u64,
    budget: u64,
    num_candidates: usize,
) {
    if let Some(t0) = decision_t0 {
        let secs = t0.elapsed().as_secs();
        if secs > *ticked {
            *ticked = secs;
            writeln!(
                std::io::stdout(),
                "After {secs} seconds, doing iteration {sim_iter} of {budget} with {num_candidates} candidates"
            )
            .ok();
        }
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

#[inline(always)]
fn tiles_placed(play: &movegen::Play) -> usize {
    match play {
        movegen::Play::Exchange { .. } => 0,
        movegen::Play::Place { word, .. } => word.iter().filter(|&&tile| tile != 0).count(),
    }
}

#[inline(always)]
fn play_score(play: &movegen::Play) -> i64 {
    match play {
        movegen::Play::Exchange { .. } => 0,
        movegen::Play::Place { score, .. } => *score as i64,
    }
}

// twice a rack's tile scores, as equity: what the other side gains when the
// side holding the rack is gone out on
#[inline(always)]
fn twice(game_config: &game_config::GameConfig, rack: &[u8]) -> i64 {
    2 * equity::SCALE as i64 * game_config.alphabet().rack_score(rack) as i64
}

// the tiles a play takes from its rack, a blank as 0, sorted
#[inline(always)]
fn used_tiles(play: &movegen::Play, out: &mut Vec<u8>) {
    out.clear();
    match play {
        movegen::Play::Exchange { tiles } => out.extend_from_slice(tiles),
        movegen::Play::Place { word, .. } => out.extend(
            word.iter()
                .filter_map(|&tile| (tile != 0).then_some(tile & !((tile as i8) >> 7) as u8)),
        ),
    }
    out.sort_unstable();
}

// a sorted rack less the sorted tiles used from it, sorted
#[inline(always)]
fn rack_less(rack: &[u8], used: &[u8], out: &mut Vec<u8>) {
    out.clear();
    let mut j = 0;
    for &tile in rack {
        if j < used.len() && used[j] == tile {
            j += 1;
        } else {
            out.push(tile);
        }
    }
}

// of plays, best first, the first DEFT_FIELD within window of the best: the one
// leaving the board the fewest bingo lanes, the earlier on a tie
#[inline]
fn lane_pick(
    game_config: &game_config::GameConfig,
    board_tiles: &[u8],
    plays: &[movegen::ValuedMove],
    window: i32,
) -> usize {
    let best = plays[0].equity.raw();
    let mut after = Vec::new();
    let mut pick = (0, u32::MAX);
    for (i, m) in plays.iter().enumerate().take(DEFT_FIELD) {
        if best - m.equity.raw() > window {
            break;
        }
        glance::board_after(game_config, board_tiles, &m.play, &mut after);
        let lanes = glance::glance_at(game_config, &after).bingo_lanes;
        if lanes < pick.1 {
            pick = (i, lanes);
        }
    }
    pick.0
}

// with the bag empty and two players, the first of plays that goes out and wins
#[inline]
fn winning_out(
    game_config: &game_config::GameConfig,
    game_state: &game_state::GameState,
    plays: &[movegen::ValuedMove],
) -> Option<usize> {
    if !game_state.bag.is_empty() || game_state.players.len() != 2 {
        return None;
    }
    let mover = game_state.turn as usize;
    let rack_len = game_state.players[mover].rack.len();
    let other = &game_state.players[1 - mover];
    let ending = game_state.players[mover].score as i64 + twice(game_config, &other.rack);
    plays.iter().position(|m| {
        matches!(m.play, movegen::Play::Place { .. })
            && tiles_placed(&m.play) == rack_len
            && ending + play_score(&m.play) > other.score as i64
    })
}

// with two to a rack's worth of tiles in the bag, when the play at fewest does
// not leave exactly one tile in the bag, the first of plays within
// DEFT_ONE_WINDOW of the best that does
#[inline]
fn one_in_the_bag(
    game_config: &game_config::GameConfig,
    game_state: &game_state::GameState,
    plays: &[movegen::ValuedMove],
    fewest: usize,
) -> Option<usize> {
    let bag = game_state.bag.len();
    if !(2..=game_config.rack_size() as usize).contains(&bag) {
        return None;
    }
    let leaves_one = |m: &movegen::ValuedMove| tiles_placed(&m.play) + 1 == bag;
    if leaves_one(&plays[fewest]) {
        return None;
    }
    let best = plays[0].equity.raw();
    plays
        .iter()
        .take_while(|m| best - m.equity.raw() <= DEFT_ONE_WINDOW)
        .position(leaves_one)
}

// the board after a play, and the rack the play leaves; false when the rack
// cannot make the play
#[inline]
fn after_play(
    game_config: &game_config::GameConfig,
    board_tiles: &[u8],
    rack: &[u8],
    play: &movegen::Play,
    after: &mut Vec<u8>,
    kept: &mut Vec<u8>,
) -> bool {
    kept.clear();
    kept.extend_from_slice(rack);
    match play {
        movegen::Play::Exchange { tiles } => {
            after.clear();
            after.extend_from_slice(board_tiles);
            game_state::use_tiles(kept, tiles.iter().copied()).is_ok()
        }
        movegen::Play::Place { word, .. } => {
            glance::board_after(game_config, board_tiles, play, after);
            game_state::use_tiles(
                kept,
                word.iter()
                    .filter_map(|&tile| (tile != 0).then_some(tile & !((tile as i8) >> 7) as u8)),
            )
            .is_ok()
        }
    }
}

// a board's squares as bits, for a board of up to SQUARE_WORDS * 64 of them
const SQUARE_WORDS: usize = 8;

#[derive(Clone, Copy)]
struct Squares([u64; SQUARE_WORDS]);

impl Squares {
    const NONE: Self = Self([0; SQUARE_WORDS]);

    #[inline(always)]
    fn set(&mut self, square: usize) {
        self.0[square >> 6] |= 1 << (square & 63);
    }

    #[inline(always)]
    fn meets(&self, other: &Self) -> bool {
        self.0.iter().zip(&other.0).any(|(a, b)| a & b != 0)
    }

    #[inline(always)]
    fn each(&self, mut f: impl FnMut(usize)) {
        for (w, &bits) in self.0.iter().enumerate() {
            let mut b = bits;
            while b != 0 {
                f(w * 64 + b.trailing_zeros() as usize);
                b &= b - 1;
            }
        }
    }
}

// a play's footprint on the board it is made on: the squares it fills; its
// zone, the squares whose filling would change it (those it fills, and the
// first empty square past each end of its main word and of each word it makes
// across); and the lane its main word lies on
#[derive(Clone, Copy)]
struct Footprint {
    fills: Squares,
    zone: Squares,
    down: bool,
    lane: i8, // -1 when it places no tiles
}

impl Footprint {
    // whether its lane is among the rows and columns of the masks
    #[inline(always)]
    fn on(&self, rows: u128, cols: u128) -> bool {
        self.lane >= 0 && ((if self.down { cols } else { rows }) >> self.lane) & 1 != 0
    }
}

#[inline]
fn footprint(dim: &matrix::Dim, board_tiles: &[u8], play: &movegen::Play) -> Footprint {
    let mut fp = Footprint {
        fills: Squares::NONE,
        zone: Squares::NONE,
        down: false,
        lane: -1,
    };
    let movegen::Play::Place {
        down,
        lane,
        idx,
        word,
        ..
    } = play
    else {
        return fp;
    };
    fp.down = *down;
    fp.lane = *lane;
    let strider = dim.lane(*down, *lane);
    let len = word.len() as i8;
    if *idx > 0 {
        fp.zone.set(strider.at(idx - 1));
    }
    if idx + len < strider.len() {
        fp.zone.set(strider.at(idx + len));
    }
    for (pos, &tile) in (*idx..).zip(word.iter()) {
        if tile == 0 {
            continue;
        }
        let square = strider.at(pos);
        fp.fills.set(square);
        fp.zone.set(square);
        let across = dim.lane(!*down, pos);
        let mut j = lane - 1;
        while j >= 0 && board_tiles[across.at(j)] != 0 {
            j -= 1;
        }
        if j >= 0 {
            fp.zone.set(across.at(j));
        }
        let mut j = lane + 1;
        while j < across.len() && board_tiles[across.at(j)] != 0 {
            j += 1;
        }
        if j < across.len() {
            fp.zone.set(across.at(j));
        }
    }
    fp
}

// the rows and columns holding a square of a zone: the lanes whose plays a move
// with that zone can change
#[inline(always)]
fn zone_lanes(dim: &matrix::Dim, zone: &Squares) -> (u128, u128) {
    let (mut rows, mut cols) = (0u128, 0u128);
    let n = dim.cols as usize;
    zone.each(|square| {
        rows |= 1 << (square / n);
        cols |= 1 << (square % n);
    });
    (rows, cols)
}

// a set of the other side's moves by their index, as bits
#[derive(Clone)]
struct PlaySet(Vec<u64>);

impl PlaySet {
    #[inline(always)]
    fn new(n: usize) -> Self {
        Self(vec![0; n.div_ceil(64)])
    }

    #[inline(always)]
    fn set(&mut self, i: usize) {
        self.0[i >> 6] |= 1 << (i & 63);
    }

    #[inline(always)]
    fn has(&self, i: usize) -> bool {
        (self.0[i >> 6] >> (i & 63)) & 1 != 0
    }

    #[inline(always)]
    fn first(&self) -> Option<usize> {
        self.0
            .iter()
            .enumerate()
            .find(|&(_, &b)| b != 0)
            .map(|(w, &b)| w * 64 + b.trailing_zeros() as usize)
    }

    #[inline(always)]
    fn is_empty(&self) -> bool {
        self.0.iter().all(|&b| b == 0)
    }
}

// the plays of rack on board_tiles, raw and unsorted, on the rows and columns
// in the masks only
#[inline]
fn gen_plays<N: kwg::Node, L: kwg::Node>(
    generator: &mut movegen::KurniaMoveGenerator,
    board_snapshot: &movegen::BoardSnapshot<'_, N, L>,
    board_tiles: &[u8],
    rack: &[u8],
    rows: u128,
    cols: u128,
) {
    generator.gen_moves_raw_lanes_unsorted(
        &movegen::BoardSnapshot {
            board_tiles,
            game_config: board_snapshot.game_config,
            kwg: board_snapshot.kwg,
            anagrams: board_snapshot.anagrams,
            rack_lengths: board_snapshot.rack_lengths,
            klv: board_snapshot.klv,
        },
        rack,
        i16::MAX,
        movegen::PassPolicy::OnlyWhenForced,
        rows,
        cols,
    );
}

// the generator out in two runs, and the buffers it fills
struct Scratch {
    generator: movegen::KurniaMoveGenerator,
    after: Vec<u8>,
    kept: Vec<u8>,
    used: Vec<u8>,
}

impl Scratch {
    #[inline]
    fn new(game_config: &game_config::GameConfig) -> Self {
        Self {
            generator: movegen::KurniaMoveGenerator::new(game_config),
            after: Vec::new(),
            kept: Vec::new(),
            used: Vec::new(),
        }
    }
}

// one of the other side's moves on the board as it is: its weight (its score
// and twice the tiles it plays), its score, whether it goes out, and its
// footprint
struct Reply {
    weight: i64,
    score: i64,
    out: bool,
    footprint: Footprint,
}

// one of the mover's moves on the board as it is: the play, its score, the rack
// it keeps (sorted), and its footprint
struct Choice {
    play: movegen::Play,
    score: i64,
    left: Vec<u8>,
    footprint: Footprint,
}

// out in two's first pass, with the bag empty and two players: every move of
// each side on the board as it is, from one generation of each rack
struct OutInTwo<'a, N: kwg::Node, L: kwg::Node> {
    board_snapshot: &'a movegen::BoardSnapshot<'a, N, L>,
    game_state: &'a game_state::GameState,
    lead: i64,
    other_twice: i64,
    replies: Vec<Reply>,    // by weight
    by_score: Vec<usize>,   // the replies by score
    fills_at: Vec<PlaySet>, // a square's: the replies that fill it
    zone_at: Vec<PlaySet>,  // a square's: the replies whose zones hold it
    going_out: PlaySet,
    not_out: PlaySet,
    choices: Vec<Choice>,
    outs_of: fash::MyHashMap<Vec<u8>, Vec<usize>>, // by the tiles taken, best first
}

impl<'a, N: kwg::Node, L: kwg::Node> OutInTwo<'a, N, L> {
    #[inline]
    fn new(
        board_snapshot: &'a movegen::BoardSnapshot<'a, N, L>,
        game_state: &'a game_state::GameState,
        scratch: &mut Scratch,
    ) -> Option<Self> {
        if !game_state.bag.is_empty() || game_state.players.len() != 2 {
            return None;
        }
        let game_config = board_snapshot.game_config;
        let dim = game_config.board_layout().dim();
        let area = dim.rows as usize * dim.cols as usize;
        if area > SQUARE_WORDS * 64 {
            return None;
        }
        let mover = game_state.turn as usize;
        let rack = &game_state.players[mover].rack;
        let other_rack = &game_state.players[1 - mover].rack;
        let board_tiles = &game_state.board_tiles[..];

        gen_plays(
            &mut scratch.generator,
            board_snapshot,
            board_tiles,
            other_rack,
            !0,
            !0,
        );
        let mut replies = Vec::new();
        for m in scratch.generator.plays.iter() {
            if !matches!(m.play, movegen::Play::Place { .. }) {
                continue;
            }
            used_tiles(&m.play, &mut scratch.used);
            let score = play_score(&m.play);
            replies.push(Reply {
                weight: score + twice(game_config, &scratch.used),
                score,
                out: scratch.used.len() == other_rack.len(),
                footprint: footprint(dim, board_tiles, &m.play),
            });
        }
        replies.sort_by_key(|reply| std::cmp::Reverse(reply.weight));
        let n = replies.len();
        let mut by_score = (0..n).collect::<Vec<_>>();
        by_score.sort_by(|&a, &b| replies[b].score.cmp(&replies[a].score));
        let mut fills_at = vec![PlaySet::new(n); area];
        let mut zone_at = vec![PlaySet::new(n); area];
        let mut going_out = PlaySet::new(n);
        let mut not_out = PlaySet::new(n);
        for (i, reply) in replies.iter().enumerate() {
            reply.footprint.fills.each(|square| fills_at[square].set(i));
            reply.footprint.zone.each(|square| zone_at[square].set(i));
            if reply.out {
                going_out.set(i);
            } else {
                not_out.set(i);
            }
        }

        let mut sorted_rack = rack.clone();
        sorted_rack.sort_unstable();
        gen_plays(
            &mut scratch.generator,
            board_snapshot,
            board_tiles,
            rack,
            !0,
            !0,
        );
        let mut choices = Vec::new();
        let mut outs_of = fash::MyHashMap::<Vec<u8>, Vec<usize>>::default();
        for m in scratch.generator.plays.iter() {
            if !matches!(m.play, movegen::Play::Place { .. }) {
                continue;
            }
            let mut used = Vec::new();
            used_tiles(&m.play, &mut used);
            let mut left = Vec::new();
            rack_less(&sorted_rack, &used, &mut left);
            outs_of.entry(used).or_default().push(choices.len());
            choices.push(Choice {
                play: m.play.clone(),
                score: play_score(&m.play),
                left,
                footprint: footprint(dim, board_tiles, &m.play),
            });
        }
        for list in outs_of.values_mut() {
            list.sort_by(|&a, &b| choices[b].score.cmp(&choices[a].score));
        }
        Some(Self {
            board_snapshot,
            game_state,
            lead: game_state.players[mover].score as i64
                - game_state.players[1 - mover].score as i64,
            other_twice: twice(game_config, other_rack),
            replies,
            by_score,
            fills_at,
            zone_at,
            going_out,
            not_out,
            choices,
            outs_of,
        })
    }

    // the other side's moves that fill a square of zone: those that block a
    // play with that zone
    #[inline]
    fn blocking(&self, zone: &Squares) -> PlaySet {
        let mut blocks = PlaySet::new(self.replies.len());
        zone.each(|square| {
            for (x, y) in blocks.0.iter_mut().zip(&self.fills_at[square].0) {
                *x |= y;
            }
        });
        blocks
    }

    // into changed, the other side's moves whose zones a move filling fills
    // meets: those the move changes
    #[inline]
    fn changed_by(&self, fills: &Squares, changed: &mut PlaySet) {
        changed.0.fill(0);
        fills.each(|square| {
            for (x, y) in changed.0.iter_mut().zip(&self.zone_at[square].0) {
                *x |= y;
            }
        });
    }

    // of the mover's moves on the board as it is that go out from the rack
    // choice keeps, those choice leaves standing and that lie off the rows and
    // columns in the masks, by score and footprint
    #[inline]
    fn standing_outs<'s>(
        &'s self,
        choice: &'s Choice,
        rows: u128,
        cols: u128,
    ) -> impl Iterator<Item = (i64, &'s Footprint)> + 's {
        self.outs_of
            .get(&choice.left)
            .into_iter()
            .flatten()
            .map(|&u| &self.choices[u])
            .filter(move |out| {
                !choice.footprint.fills.meets(&out.footprint.zone) && !out.footprint.on(rows, cols)
            })
            .map(|out| (out.score, &out.footprint))
    }

    // a move's value against the other side's best reply on footprints alone:
    // going out; passing; or a move that blocks the mover's best outs, weighed
    // by its score and the tiles it plays. changed holds the replies the move
    // changes, and outs the mover's outs next turn, best first, each by its
    // score and the replies that block it. Also whether those outs hold the
    // value down: none stands, or one reply blocks them all
    #[inline]
    fn value_on_footprints(
        &self,
        changed: &PlaySet,
        score: i64,
        left_twice: i64,
        outs: &[(i64, &PlaySet)],
        acc: &mut PlaySet,
        t: &mut PlaySet,
    ) -> (i64, bool) {
        let c = score + self.other_twice;
        let stuck = -(equity::ENDGAME_PENALTY_BASE as i64) - left_twice;
        let mut v = i64::MAX;
        for ((x, &o), &k) in t.0.iter_mut().zip(&self.going_out.0).zip(&changed.0) {
            *x = o & !k;
        }
        if let Some(p) = t.first() {
            v = v.min(score - self.replies[p].score - left_twice);
        }
        for ((x, &o), &k) in acc.0.iter_mut().zip(&self.not_out.0).zip(&changed.0) {
            *x = o & !k;
        }
        if outs.is_empty() {
            let best = self
                .by_score
                .iter()
                .find(|&&x| acc.has(x))
                .map_or(0, |&x| self.replies[x].score.max(0));
            return (v.min(score - best + stuck), true);
        }
        v = v.min(c + outs[0].0);
        for &(s_u, blocks) in outs {
            for ((x, a), &b) in t.0.iter_mut().zip(acc.0.iter_mut()).zip(&blocks.0) {
                *x = *a & !b;
                *a &= b;
            }
            if let Some(x) = t.first() {
                v = v.min(c - self.replies[x].weight + s_u);
            }
            if acc.is_empty() {
                return (v, false);
            }
        }
        match self.by_score.iter().find(|&&x| acc.has(x)) {
            Some(&x) => (v.min(score - self.replies[x].score + stuck), true),
            None => (v, false),
        }
    }

    // what a choice is worth to the mover: its score, less the other side's
    // best reply on the board after it -- going out; passing; or a move that
    // blocks the mover's outs (filling their zones), weighed by its score and
    // the tiles it leaves the other side, against the mover's best out it
    // leaves standing, or the mover stuck if it leaves none (the movegen's
    // penalty and twice the mover's rack). The plays on the board after it are
    // those of the board as it is that it leaves standing off its zone's lanes,
    // and those a generation of those lanes alone finds
    #[inline]
    fn value_after(&self, scratch: &mut Scratch, i: usize) -> Option<i64> {
        let choice = &self.choices[i];
        let game_config = self.board_snapshot.game_config;
        let dim = game_config.board_layout().dim();
        let mover = self.game_state.turn as usize;
        let rack = &self.game_state.players[mover].rack;
        let other_rack = &self.game_state.players[1 - mover].rack;
        if !after_play(
            game_config,
            &self.game_state.board_tiles,
            rack,
            &choice.play,
            &mut scratch.after,
            &mut scratch.kept,
        ) {
            return None;
        }
        let s_m = choice.score;
        let c = s_m + self.other_twice;
        if scratch.kept.is_empty() {
            return Some(c);
        }
        let kept_twice = twice(game_config, &scratch.kept);
        let stuck = -(equity::ENDGAME_PENALTY_BASE as i64) - kept_twice;
        let (rows, cols) = zone_lanes(dim, &choice.footprint.zone);
        // the mover's outs after it, best first, by score and zone
        let mut outs = self
            .standing_outs(choice, rows, cols)
            .map(|(s, fp)| (s, fp.zone))
            .collect::<Vec<_>>();
        gen_plays(
            &mut scratch.generator,
            self.board_snapshot,
            &scratch.after,
            &scratch.kept,
            rows,
            cols,
        );
        outs.extend(
            scratch
                .generator
                .plays
                .iter()
                .filter(|m| tiles_placed(&m.play) == scratch.kept.len())
                .map(|m| {
                    (
                        play_score(&m.play),
                        footprint(dim, &scratch.after, &m.play).zone,
                    )
                }),
        );
        outs.sort_by_key(|&(s, _)| std::cmp::Reverse(s));
        let mut v = match outs.first() {
            Some(&(best, _)) => c + best,
            None => s_m + stuck,
        };
        let mut reply = |s_x: i64, w_x: i64, out: bool, fills: &Squares| {
            if out {
                v = v.min(s_m - s_x - kept_twice);
                return;
            }
            match outs.iter().find(|(_, zone)| !fills.meets(zone)) {
                Some(&(s_u, _)) => v = v.min(c - w_x + s_u),
                None => v = v.min(s_m - s_x + stuck),
            }
        };
        for r in &self.replies {
            if !choice.footprint.fills.meets(&r.footprint.zone) && !r.footprint.on(rows, cols) {
                reply(r.score, r.weight, r.out, &r.footprint.fills);
            }
        }
        gen_plays(
            &mut scratch.generator,
            self.board_snapshot,
            &scratch.after,
            other_rack,
            rows,
            cols,
        );
        for m in scratch.generator.plays.iter() {
            if !matches!(m.play, movegen::Play::Place { .. }) {
                continue;
            }
            used_tiles(&m.play, &mut scratch.used);
            let s_x = play_score(&m.play);
            reply(
                s_x,
                s_x + twice(game_config, &scratch.used),
                scratch.used.len() == other_rack.len(),
                &footprint(dim, &scratch.after, &m.play).fills,
            );
        }
        Some(v)
    }

    // the move to play instead of the play at fewest, if any
    #[inline]
    fn pick(
        &self,
        scratch: &mut Scratch,
        plays: &[movegen::ValuedMove],
        fewest: usize,
    ) -> Option<movegen::Play> {
        let fewest_play = &plays[fewest].play;
        let fewest_at = self.choices.iter().position(|c| c.play == *fewest_play)?;
        let fewest_value = self.value_after(scratch, fewest_at)?;
        if self.lead + fewest_value > 0 {
            return None;
        }
        let (mut values, hopes) = self.values_and_hopes();
        self.raise_openers(scratch, &mut values, &hopes);
        // the best few by value (a move that wins, if any does), valued again
        // on the board after each; the best of those, if it beats the play at
        // fewest
        let mut order = (0..self.choices.len()).collect::<Vec<_>>();
        order.sort_by_key(|&i| std::cmp::Reverse(values[i]));
        let mut best: Option<(i64, usize)> = None;
        for &i in order.iter().take(OUT_IN_TWO_CHECKED) {
            if self.choices[i].play == *fewest_play {
                continue;
            }
            let Some(v) = self.value_after(scratch, i) else {
                continue;
            };
            if best.is_none_or(|(b, _)| v > b) {
                best = Some((v, i));
            }
        }
        best.filter(|&(v, _)| v > fewest_value)
            .map(|(_, i)| self.choices[i].play.clone())
    }

    // every choice's value on footprints alone, with the outs standing now;
    // and, for a choice whose value the lack of a standing unblockable out
    // holds down, its hope, its value with one more such out, worth its
    // leftover's best out anywhere on the board now (the others' hope is
    // i64::MIN): the choices that may open a second place
    #[inline]
    fn values_and_hopes(&self) -> (Vec<i64>, Vec<i64>) {
        let game_config = self.board_snapshot.game_config;
        let n = self.replies.len();
        let (mut changed, mut acc, mut t) = (PlaySet::new(n), PlaySet::new(n), PlaySet::new(n));
        let unblockable = PlaySet::new(n);
        let mut blockers = fash::MyHashMap::<usize, PlaySet>::default();
        let mut values = Vec::with_capacity(self.choices.len());
        let mut hopes = Vec::with_capacity(self.choices.len());
        for choice in &self.choices {
            if choice.left.is_empty() {
                values.push(choice.score + self.other_twice);
                hopes.push(i64::MIN);
                continue;
            }
            self.changed_by(&choice.footprint.fills, &mut changed);
            let list = self.outs_of.get(&choice.left);
            let standing = list
                .map(|list| {
                    list.iter()
                        .copied()
                        .filter(|&u| {
                            !choice
                                .footprint
                                .fills
                                .meets(&self.choices[u].footprint.zone)
                        })
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            for &u in &standing {
                blockers
                    .entry(u)
                    .or_insert_with(|| self.blocking(&self.choices[u].footprint.zone));
            }
            let mut outs = standing
                .iter()
                .map(|&u| (self.choices[u].score, &blockers[&u]))
                .collect::<Vec<_>>();
            let left_twice = twice(game_config, &choice.left);
            let (value, held) = self.value_on_footprints(
                &changed,
                choice.score,
                left_twice,
                &outs,
                &mut acc,
                &mut t,
            );
            values.push(value);
            if held {
                let phantom = list.map_or(0, |list| self.choices[list[0]].score);
                let at = outs.partition_point(|&(s, _)| s >= phantom);
                outs.insert(at, (phantom, &unblockable));
                let (hope, _) = self.value_on_footprints(
                    &changed,
                    choice.score,
                    left_twice,
                    &outs,
                    &mut acc,
                    &mut t,
                );
                hopes.push(hope);
            } else {
                hopes.push(i64::MIN);
            }
        }
        (values, hopes)
    }

    // the choices with the best hopes above their values, their values raised
    // to those with their leftover's real outs on the board after each, the
    // other side's replies still those standing
    #[inline]
    fn raise_openers(&self, scratch: &mut Scratch, values: &mut [i64], hopes: &[i64]) {
        let game_config = self.board_snapshot.game_config;
        let dim = game_config.board_layout().dim();
        let n = self.replies.len();
        let (mut changed, mut acc, mut t) = (PlaySet::new(n), PlaySet::new(n), PlaySet::new(n));
        let mut hoping = (0..self.choices.len())
            .filter(|&i| hopes[i] > values[i])
            .collect::<Vec<_>>();
        hoping.sort_by(|&a, &b| hopes[b].cmp(&hopes[a]));
        for &i in hoping.iter().take(OUT_IN_TWO_OPENERS) {
            let choice = &self.choices[i];
            glance::board_after(
                game_config,
                &self.game_state.board_tiles,
                &choice.play,
                &mut scratch.after,
            );
            let (rows, cols) = zone_lanes(dim, &choice.footprint.zone);
            let mut real = self
                .standing_outs(choice, rows, cols)
                .map(|(s, fp)| (s, self.blocking(&fp.zone)))
                .collect::<Vec<_>>();
            gen_plays(
                &mut scratch.generator,
                self.board_snapshot,
                &scratch.after,
                &choice.left,
                rows,
                cols,
            );
            real.extend(
                scratch
                    .generator
                    .plays
                    .iter()
                    .filter(|p| tiles_placed(&p.play) == choice.left.len())
                    .map(|p| {
                        (
                            play_score(&p.play),
                            self.blocking(&footprint(dim, &scratch.after, &p.play).zone),
                        )
                    }),
            );
            real.sort_by_key(|&(s, _)| std::cmp::Reverse(s));
            let outs = real.iter().map(|(s, b)| (*s, b)).collect::<Vec<_>>();
            self.changed_by(&choice.footprint.fills, &mut changed);
            let (value, _) = self.value_on_footprints(
                &changed,
                choice.score,
                twice(game_config, &choice.left),
                &outs,
                &mut acc,
                &mut t,
            );
            values[i] = values[i].max(value);
        }
    }
}

// out in two, with the bag empty and two players: each of the mover's moves
// valued against the other side's best reply, on footprints from one generation
// of each rack on the board as it is; the moves that may open a second place
// valued again with their leftover's real outs; and the best few valued again
// on the board after each. Their best, when the play at fewest does not win by
// its own value and that best beats it
#[inline]
fn out_in_two<N: kwg::Node, L: kwg::Node>(
    board_snapshot: &movegen::BoardSnapshot<'_, N, L>,
    game_state: &game_state::GameState,
    plays: &[movegen::ValuedMove],
    fewest: usize,
) -> Option<movegen::Play> {
    let mut scratch = Scratch::new(board_snapshot.game_config);
    OutInTwo::new(board_snapshot, game_state, &mut scratch)?.pick(&mut scratch, plays, fewest)
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

pub enum MovePicker<'a, N: kwg::Node, L: kwg::Node> {
    Hasty,
    Dynamic(klv::DynamicLeavesRef<'a>),
    Simmer(Box<Simmer<'a, N, L>>),
    // of hasty's moves within this much equity of its best, the one leaving the
    // board the fewest bingo lanes; but with the bag empty, the first move that
    // goes out and wins, else out in two's move when it finds one; and with two
    // to a rack's worth of tiles in the bag, when that one does not leave
    // exactly one there, the first within DEFT_ONE_WINDOW of its best that does
    Deft(i32),
}

impl<
    'a,
    N: kwg::Node + Sync,
    L: kwg::Node + Sync,
    const OBSERVE: bool,
    R: rand::Rng + SeedableRng + simmer::CopyState + Sync,
> Simmer<'a, N, L, OBSERVE, R>
{
    #[inline]
    fn run_iterations(
        &mut self,
        move_generator: &movegen::KurniaMoveGenerator,
        budget: u64,
        count: u64,
    ) -> error::Returns<()> {
        if self.candidates.len() < 2 {
            return Ok(());
        }

        #[cfg(not(target_family = "wasm"))]
        if self.sim_threads > 1 {
            return self.run_iterations_parallel(move_generator, count);
        }
        let mut candidates = std::mem::take(&mut self.candidates);
        let mut retired = std::mem::take(&mut self.retired);
        const Z: f64 = 1.96; // 95% confidence interval
        let paired_z = paired_z(budget);
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
                    self.arm_tables,
                    self.klv,
                    &move_generator.plays[candidate.play_index].play,
                    self.win_pct_table,
                )?;
                candidate.stats.update(value);
                candidate.objectives.push(value);
                if OBSERVE {
                    candidate
                        .equity_stats
                        .update(simmer::spread_points(sim_spread));
                    candidate.win_rate_stats.update(win_prob);
                }
            }
            tick(
                self.decision_t0,
                &mut self.ticked,
                sim_iter,
                budget,
                candidates.len(),
            );
            if sim_iter % PRUNE_CADENCE == 0 {
                retire_paired(&mut candidates, &mut retired, paired_z);

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
        Ok(())
    }

    // the next block to roll out: from the iterations done up to the next look
    // or end, whichever is first, for the candidates still in; none once end is
    // reached or fewer than two candidates are left
    #[inline]
    pub fn next_block(&self, end: u64) -> Option<SimBlock> {
        if self.candidates.len() < 2 || self.iters_done >= end {
            return None;
        }
        let lo = self.iters_done;
        Some(SimBlock {
            lo,
            hi: end.min((lo / PRUNE_CADENCE + 1) * PRUNE_CADENCE),
            plays: self
                .candidates
                .iter()
                .map(|candidate| candidate.play_index)
                .collect(),
        })
    }

    // compute the rows of the iterations after lo up to hi for plays, on a
    // thread or a separate worker: each iteration is seeded by its index alone,
    // so the rows are the same however a block is split
    #[inline]
    pub fn block_rows(
        &self,
        move_generator: &movegen::KurniaMoveGenerator,
        lo: u64,
        hi: u64,
        plays: &[usize],
    ) -> error::Returns<SimRows> {
        let mut simmer = self.simmer.prepared_clone(self.game_config);
        let pristine = simmer.prepared_state().clone();
        let size = (hi - lo) as usize * plays.len();
        let mut rows = SimRows {
            objective: Vec::new(),
            equity: Vec::new(),
            win_rate: Vec::new(),
        };
        rows.objective.reserve(size);
        if OBSERVE {
            rows.equity.reserve(size);
            rows.win_rate.reserve(size);
        }
        for sim_iter in lo + 1..=hi {
            simmer.restore_prepared(&pristine);
            simmer.reseed(mix(self.decision_seed, sim_iter));
            simmer.prepare_iteration();
            for &play_index in plays {
                let (value, sim_spread, win_prob) = rollout_objective(
                    &mut simmer,
                    self.game_config,
                    self.kwg,
                    self.arm_tables,
                    self.klv,
                    &move_generator.plays[play_index].play,
                    self.win_pct_table,
                )?;
                rows.objective.push(value);
                if OBSERVE {
                    rows.equity.push(simmer::spread_points(sim_spread));
                    rows.win_rate.push(win_prob);
                }
            }
        }
        Ok(rows)
    }

    // take in a whole block's rows, each candidate's in iteration order
    // (combining variances is not associative in floating point), then the look
    // at the block's end if it ends on one
    #[inline]
    pub fn absorb_block(&mut self, block: &SimBlock, rows: &SimRows) -> error::Returns<()> {
        let num_candidates = block.plays.len();
        let size = (block.hi - block.lo) as usize * num_candidates;
        if block.lo != self.iters_done
            || !self
                .candidates
                .iter()
                .map(|candidate| candidate.play_index)
                .eq(block.plays.iter().copied())
            || rows.objective.len() != size
            || (OBSERVE && (rows.equity.len() != size || rows.win_rate.len() != size))
        {
            return Err("the rows are not the block's the simmer is at".into());
        }
        for (candidate_index, candidate) in self.candidates.iter_mut().enumerate() {
            for k in (candidate_index..size).step_by(num_candidates) {
                candidate.stats.update(rows.objective[k]);
                candidate.objectives.push(rows.objective[k]);
                if OBSERVE {
                    candidate.equity_stats.update(rows.equity[k]);
                    candidate.win_rate_stats.update(rows.win_rate[k]);
                }
            }
        }
        self.iters_done = block.hi;
        let budget = self.num_sim_iters;
        tick(
            self.decision_t0,
            &mut self.ticked,
            block.hi,
            budget,
            self.candidates.len(),
        );
        if block.hi.is_multiple_of(PRUNE_CADENCE) {
            const Z: f64 = 1.96; // 95% confidence interval
            retire_paired(&mut self.candidates, &mut self.retired, paired_z(budget));
            let prune_periods_remaining = budget.saturating_sub(block.hi) / PRUNE_CADENCE;
            limit_surviving_candidates(
                &mut self.candidates,
                &mut self.retired,
                Z,
                1 + 2 * prune_periods_remaining as usize,
            );
        }
        Ok(())
    }

    // split each block into contiguous ranges, one per thread, and put the rows
    // back together in iteration order
    #[cfg(not(target_family = "wasm"))]
    #[inline]
    fn run_iterations_parallel(
        &mut self,
        move_generator: &movegen::KurniaMoveGenerator,
        count: u64,
    ) -> error::Returns<()> {
        let num_threads = self.sim_threads;
        let end = self.iters_done + count;
        while let Some(block) = self.next_block(end) {
            let block_len = (block.hi - block.lo) as usize;
            let mut rows = SimRows {
                objective: Vec::new(),
                equity: Vec::new(),
                win_rate: Vec::new(),
            };
            let this = &*self;
            std::thread::scope(|scope| -> error::Returns<()> {
                let mut handles = Vec::with_capacity(num_threads);
                for thread_index in 0..num_threads {
                    let base = block_len / num_threads;
                    let extra = block_len % num_threads;
                    let lo = thread_index * base + thread_index.min(extra);
                    let hi = lo + base + if thread_index < extra { 1 } else { 0 };
                    let block = &block;
                    handles.push(scope.spawn(move || {
                        this.block_rows(
                            move_generator,
                            block.lo + lo as u64,
                            block.lo + hi as u64,
                            &block.plays,
                        )
                    }));
                }
                for handle in handles {
                    rows.extend(handle.join().unwrap()?);
                }
                Ok(())
            })?;
            self.absorb_block(&block, &rows)?;
        }
        Ok(())
    }

    #[inline]
    pub fn begin_decision(
        &mut self,
        move_generator: &movegen::KurniaMoveGenerator,
        game_state: &game_state::GameState,
        iters: u64,
        kept_leaves: &[(Vec<u8>, f64)], // the leaves the opponent may have kept, with weights
    ) -> error::Returns<()> {
        self.simmer.prepare(
            self.game_config,
            game_state,
            self.num_sim_plies,
            OBSERVE,
            kept_leaves,
        );
        self.prepared_pristine
            .clone_from(self.simmer.prepared_state());
        self.candidates = self.take_candidates(move_generator.plays.len());
        self.next_stream_id = self.candidates.len() as u64;
        self.retired.clear();
        self.iters_done = 0;
        self.decision_t0 = (!self.quiet).then(std::time::Instant::now);
        self.ticked = 0;
        let budget = self.num_sim_iters;
        self.run_iterations(move_generator, budget, iters)
    }

    #[inline]
    pub fn resume(
        &mut self,
        move_generator: &movegen::KurniaMoveGenerator,
        extra_iters: u64,
    ) -> error::Returns<()> {
        let budget = self.num_sim_iters;
        self.run_iterations(move_generator, budget, extra_iters)
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
    ) -> error::Returns<()>
    where
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
                    None,
                );
            }
            MovePicker::Dynamic(dynamic_leaves) => {
                filtered_movegen.gen_moves(
                    move_generator,
                    board_snapshot,
                    rack,
                    game_state.current_player().num_exchanges,
                    1,
                    Some(*dynamic_leaves),
                );
            }
            MovePicker::Simmer(simmer) => {
                let t0 = (!simmer.quiet).then(std::time::Instant::now);
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
                simmer.begin_decision(move_generator, game_state, budget, &[])?;
                if let Some(t0) = t0 {
                    const Z: f64 = 1.96; // 95% confidence interval
                    let top = simmer
                        .candidates
                        .iter()
                        .max_by(|a, b| a.stats.mean().total_cmp(&b.stats.mean()))
                        .unwrap();
                    writeln!(
                        std::io::stdout(),
                        "top candidate mean = {} (sd={} count={} range {}..{}) took {:?}",
                        top.stats.mean(),
                        top.stats.standard_deviation(),
                        top.stats.count(),
                        top.stats.ci_max(-Z),
                        top.stats.ci_max(Z),
                        t0.elapsed(),
                    )
                    .ok();
                }
                let winner_play_index = top_candidate_play_index_by_mean(&simmer.candidates);
                move_generator.plays.swap(0, winner_play_index);
                move_generator.plays.truncate(1);
            }
            MovePicker::Deft(window) => {
                let game_config = board_snapshot.game_config;
                let late = game_state.bag.len() <= game_config.rack_size() as usize;
                filtered_movegen.gen_moves(
                    move_generator,
                    board_snapshot,
                    rack,
                    game_state.current_player().num_exchanges,
                    if late { DEFT_LATE_FIELD } else { DEFT_FIELD },
                    None,
                );
                let plays = &move_generator.plays;
                let fewest = lane_pick(game_config, &game_state.board_tiles, plays, *window);
                let pick = winning_out(game_config, game_state, plays)
                    .or_else(|| one_in_the_bag(game_config, game_state, plays, fewest));
                let found = match pick {
                    Some(_) => None,
                    None => out_in_two(board_snapshot, game_state, plays, fewest),
                };
                match found {
                    // out in two's move need not be among those generated; its
                    // equity here is its score
                    Some(play) => {
                        let equity = equity::Equity::new(play_score(&play) as i32);
                        move_generator.plays.clear();
                        move_generator
                            .plays
                            .push(movegen::ValuedMove { equity, play });
                    }
                    None => {
                        move_generator.plays.swap(0, pick.unwrap_or(fewest));
                        move_generator.plays.truncate(1);
                    }
                }
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{alphabet, bites, build, display};

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
            build::BuildOrder::Sorted,
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
        let kwg = kwg::Kwg::<kwg::Node22>::from_bytes_alloc(kwg_bytes).unwrap();
        let klv = klv::Klv::<kwg::Node22>::from_bytes_alloc(klv_bytes).unwrap();
        let board_snapshot = movegen::BoardSnapshot {
            board_tiles,
            game_config,
            kwg: &kwg,
            anagrams: None,
            rack_lengths: None,
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
                max_gen: usize::MAX,
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
                max_gen: usize::MAX,
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
        candidate_from_iter(play_index, 1, values)
    }

    #[inline]
    fn candidate_from_iter(play_index: usize, first_iter: u64, values: &[f64]) -> Candidate {
        Candidate {
            play_index,
            stats: stats_from(values),
            stream_id: play_index as u64,
            equity_stats: stats::Stats::new(),
            win_rate_stats: stats::Stats::new(),
            first_iter,
            objectives: values.to_vec(),
        }
    }

    #[test]
    #[inline]
    fn retire_paired_keeps_survivors_in_order() {
        let mut candidates = vec![
            candidate_from(0, &[20.0, 20.0].repeat(50)),
            candidate_from(1, &[1.0, 1.0].repeat(50)),
            candidate_from(2, &[19.0, 21.0].repeat(50)),
        ];
        let mut retired = Vec::new();
        retire_paired(&mut candidates, &mut retired, 1.96);
        assert_eq!(
            candidates.iter().map(|c| c.play_index).collect::<Vec<_>>(),
            vec![0, 2]
        );
        assert_eq!(retired.len(), 1);
        assert_eq!(retired[0].play_index, 1);
    }

    #[test]
    #[inline]
    fn retire_paired_sees_a_steady_gap_that_shared_noise_hides() {
        // both swing 50 either way on the same draws, and the second is always
        // half a point behind: apart, their intervals overlap widely; paired,
        // only the gap is left
        let swings = [50.0, -50.0].repeat(50);
        let leader = swings.iter().map(|s| 100.0 + s).collect::<Vec<_>>();
        let behind = swings.iter().map(|s| 99.5 + s).collect::<Vec<_>>();
        let mut candidates = vec![candidate_from(0, &leader), candidate_from(1, &behind)];
        assert!(candidates[1].stats.ci_max(1.96) > candidates[0].stats.ci_max(-1.96));
        let mut retired = Vec::new();
        retire_paired(&mut candidates, &mut retired, 3.0);
        assert_eq!(candidates.len(), 1);
        assert_eq!(retired[0].play_index, 1);
    }

    #[test]
    #[inline]
    fn retire_paired_pairs_by_iteration() {
        // the second joined at iteration 51 and was a point behind on each
        // iteration since; paired by position in its list instead, it would be
        // compared with the first's iterations 1 to 50 and look level
        let leader = (1..=100)
            .map(|i| 100.0 + 10.0 * (i % 3) as f64)
            .collect::<Vec<_>>();
        let joined = leader[50..].iter().map(|v| v - 1.0).collect::<Vec<_>>();
        let mut candidates = vec![
            candidate_from(0, &leader),
            candidate_from_iter(1, 51, &joined),
        ];
        let mut retired = Vec::new();
        retire_paired(&mut candidates, &mut retired, 3.0);
        assert_eq!(candidates.len(), 1);
        assert_eq!(retired[0].play_index, 1);
    }

    #[test]
    #[inline]
    fn normal_quantile_matches_known_values() {
        for (p, z) in [
            (0.5, 0.0),
            (0.975, 1.959_963_984_540_054),
            (0.99, 2.326_347_874_040_841),
            (0.001, -3.090_232_306_167_813),
            (0.0001, -3.719_016_485_455_709),
        ] {
            assert!(
                (normal_quantile(p) - z).abs() < 1e-8,
                "{p}: {}",
                normal_quantile(p)
            );
        }
        // a decision of 1000 iterations looks 62 times
        let z = paired_z(1000);
        assert!((z + normal_quantile(0.05 / 62.0)).abs() < 1e-12);
        assert!(3.1 < z && z < 3.2, "{z}");
    }

    #[test]
    #[inline]
    fn blocks_however_split_give_the_sequential_numbers() {
        let game_config = game_config::make_english_game_config();
        let kwg = kwg::Kwg::<kwg::Node22>::from_bytes_alloc(&tiny_lexicon()).unwrap();
        let klv = klv::Klv::<kwg::Node22>::from_bytes_alloc(klv::EMPTY_KLV_BYTES).unwrap();
        let arm_tables = anagrams::ArmTables::build(&kwg, &game_config, false).unwrap();
        let mut game_state = game_state::GameState::new(&game_config);
        let mut deal_rng = simmer::DefaultRng::seed_from_u64(7);
        game_state.reset_and_draw_tiles(&game_config, &mut deal_rng);
        game_state.set_current_rack(&[1, 1, 5, 5, 20, 20, 0]);
        let board_snapshot = movegen::BoardSnapshot {
            board_tiles: &game_state.board_tiles,
            game_config: &game_config,
            kwg: &kwg,
            anagrams: arm_tables.anagrams(),
            rack_lengths: arm_tables.rack_lengths(),
            klv: &klv,
        };
        let mut move_generator = movegen::KurniaMoveGenerator::new(&game_config);
        let mut scratch = Vec::new();
        gen_simmer_candidates(
            &mut move_filter::GenMoves::Unfiltered,
            &mut move_generator,
            &board_snapshot,
            &game_state.current_player().rack,
            0,
            SIMMER_FIELD,
            &mut scratch,
        );
        assert!(move_generator.plays.len() > 2);
        let new_simmer = |sim_threads| {
            let mut simmer = Simmer::<_, _, true>::new(
                &game_config,
                &kwg,
                &arm_tables,
                &klv,
                SimmerParams {
                    num_sim_iters: 64,
                    num_sim_plies: 2,
                    sim_threads,
                    win_pct_table: None,
                    quiet: true,
                },
            )
            .unwrap();
            simmer.reseed(11);
            simmer
        };

        let mut sequential = new_simmer(1);
        sequential
            .begin_decision(&move_generator, &game_state, 64, &[])
            .unwrap();
        assert!(sequential.retired_stream_ids().count() > 0);

        let mut threaded = new_simmer(3);
        threaded
            .begin_decision(&move_generator, &game_state, 64, &[])
            .unwrap();
        assert_eq!(
            sequential.leaderboard(usize::MAX),
            threaded.leaderboard(usize::MAX)
        );

        // the same decision a block at a time, each split in three uneven parts
        // as three workers would roll them out
        let mut blocked = new_simmer(1);
        blocked
            .begin_decision(&move_generator, &game_state, 0, &[])
            .unwrap();
        let mut stale = None;
        while let Some(block) = blocked.next_block(64) {
            let a = block.lo + (block.hi - block.lo) / 3;
            let b = a + (block.hi - a) / 2;
            let mut rows = SimRows {
                objective: Vec::new(),
                equity: Vec::new(),
                win_rate: Vec::new(),
            };
            for (lo, hi) in [(block.lo, a), (a, b), (b, block.hi)] {
                rows.extend(
                    blocked
                        .block_rows(&move_generator, lo, hi, &block.plays)
                        .unwrap(),
                );
            }
            blocked.absorb_block(&block, &rows).unwrap();
            stale.get_or_insert((block, rows));
        }
        assert_eq!(
            sequential.leaderboard(usize::MAX),
            blocked.leaderboard(usize::MAX)
        );
        assert_eq!(sequential.best_so_far(), blocked.best_so_far());

        // a block taken in out of its turn is refused
        let (block, rows) = stale.unwrap();
        assert!(blocked.absorb_block(&block, &rows).is_err());
    }

    #[inline]
    fn place(
        equity: i32,
        down: bool,
        lane: i8,
        idx: i8,
        word: &[u8],
        score: i32,
    ) -> movegen::ValuedMove {
        movegen::ValuedMove {
            equity: equity::Equity::new(equity),
            play: movegen::Play::Place {
                down,
                lane,
                idx,
                word: word.into(),
                score: score * equity::SCALE,
            },
        }
    }

    // six moves on the empty board, best first, each scoring eighteen: within a
    // point of the best, 1 and 2 leave the fewest bingo lanes; within two,
    // still them
    #[inline]
    fn lanes_field(game_config: &game_config::GameConfig) -> (Vec<u8>, Vec<movegen::ValuedMove>) {
        let dim = game_config.board_layout().dim();
        let board_tiles = vec![0u8; dim.rows as usize * dim.cols as usize];
        let plays = [
            (20_000, false, 6, &[1, 20, 5][..]),
            (19_800, false, 7, &[1, 20][..]),
            (19_600, true, 7, &[1, 20][..]),
            (19_400, false, 3, &[1, 20, 5, 1, 20][..]),
            (18_500, false, 1, &[1, 20, 5, 1, 20, 5, 1][..]),
            (18_000, true, 1, &[1, 20, 5, 1, 20, 5, 1][..]),
        ]
        .into_iter()
        .map(|(equity, down, idx, word)| place(equity, down, 7, idx, word, 18))
        .collect::<Vec<_>>();
        let mut after = Vec::new();
        let lanes = plays
            .iter()
            .map(|valued_move| {
                glance::board_after(game_config, &board_tiles, &valued_move.play, &mut after);
                glance::glance_at(game_config, &after).bingo_lanes
            })
            .collect::<Vec<_>>();
        assert_eq!(lanes, [8, 7, 7, 10, 12, 12]);
        (board_tiles, plays)
    }

    #[test]
    #[inline]
    fn the_lane_pick_stays_in_its_window_and_field_and_keeps_the_earlier_on_a_tie() {
        let game_config = game_config::make_english_game_config();
        let (board_tiles, plays) = lanes_field(&game_config);
        for (window, want) in [(0, 0), (equity::SCALE, 1), (2 * equity::SCALE, 1)] {
            assert_eq!(
                lane_pick(&game_config, &board_tiles, &plays, window),
                want,
                "window {window}",
            );
        }
        // deft's field: a move past its first DEFT_FIELD is not weighed
        let mut field = vec![plays[0].clone(); DEFT_FIELD];
        field.push(plays[1].clone());
        assert_eq!(
            lane_pick(&game_config, &board_tiles, &field, equity::SCALE),
            0
        );
        field.swap(DEFT_FIELD - 1, DEFT_FIELD);
        assert_eq!(
            lane_pick(&game_config, &board_tiles, &field, equity::SCALE),
            DEFT_FIELD - 1
        );
    }

    #[test]
    #[inline]
    fn deft_goes_out_only_to_win() {
        let game_config = game_config::make_english_game_config();
        let mut game_state = game_state::GameState::new(&game_config);
        game_state.bag.set_from_iter(std::iter::empty());
        let mover = game_state.turn as usize;
        game_state.players[mover].rack = vec![1, 20]; // AT
        game_state.players[1 - mover].rack = vec![5]; // E
        // the best keeps the T; the next goes out for 8, and twice the E's 1
        let plays = [
            place(30_000, false, 7, 7, &[1], 20),
            place(20_000, false, 7, 7, &[1, 20], 8),
        ];
        for (behind, want) in [(9, Some(1)), (10, None), (11, None)] {
            game_state.players[mover].score = 100 * equity::SCALE;
            game_state.players[1 - mover].score = (100 + behind) * equity::SCALE;
            assert_eq!(
                winning_out(&game_config, &game_state, &plays),
                want,
                "{behind} behind",
            );
        }
        game_state.players[1 - mover].score = 0;
        game_state.bag.set_from_iter([5u8]);
        assert_eq!(
            winning_out(&game_config, &game_state, &plays),
            None,
            "not with a tile in the bag",
        );
    }

    #[test]
    #[inline]
    fn deft_leaves_one_in_the_bag_within_its_window() {
        let game_config = game_config::make_english_game_config();
        let mut game_state = game_state::GameState::new(&game_config);
        // plays[i] places 7 - i tiles and is i points below the best
        let plays = (0..7)
            .map(|i| movegen::ValuedMove {
                equity: equity::Equity::new(-i * equity::SCALE),
                play: movegen::Play::Place {
                    down: false,
                    lane: 7,
                    idx: 0,
                    word: vec![1u8; 7 - i as usize][..].into(),
                    score: 0,
                },
            })
            .collect::<Vec<_>>();
        assert_eq!(DEFT_ONE_WINDOW, 3 * equity::SCALE);
        for (bag, fewest, want) in [
            (7, 0, Some(1)),
            (7, 1, None),
            (5, 0, Some(3)),
            (4, 0, None),
            (8, 1, None),
        ] {
            game_state.bag.set_from_iter(std::iter::repeat_n(5u8, bag));
            assert_eq!(
                one_in_the_bag(&game_config, &game_state, &plays, fewest),
                want,
                "bag {bag}, the move at {fewest}",
            );
        }
        // with one tile in the bag, a pass leaves one there, but that is no
        // rule
        let pass = movegen::ValuedMove {
            equity: equity::Equity::new(-equity::SCALE),
            play: movegen::Play::Exchange {
                tiles: [0u8; 0][..].into(),
            },
        };
        game_state.bag.set_from_iter([5u8]);
        assert_eq!(
            one_in_the_bag(&game_config, &game_state, &[plays[0].clone(), pass], 0),
            None,
        );
    }

    #[test]
    #[inline]
    fn a_footprint_holds_its_fills_and_the_first_empty_square_past_each_word() {
        let game_config = game_config::make_english_game_config();
        let dim = game_config.board_layout().dim();
        let squares = |list: &[(i8, i8)]| {
            let mut s = Squares::NONE;
            for &(row, col) in list {
                s.set(dim.at_row_col(row, col));
            }
            s.0
        };
        // AT on row 7 from column 5; an E and an A down column 4 from row 6,
        // the A making AAT along row 7
        let board_tiles = board_with(&game_config, 7, 0);
        let play = place(0, true, 4, 6, &[5, 1], 0).play;
        let fp = footprint(dim, &board_tiles, &play);
        assert_eq!(fp.fills.0, squares(&[(6, 4), (7, 4)]));
        assert_eq!(
            fp.zone.0,
            squares(&[
                (6, 4),
                (7, 4),
                (5, 4),
                (8, 4),
                (6, 3),
                (6, 5),
                (7, 3),
                (7, 7)
            ])
        );
        let (rows, cols) = zone_lanes(dim, &fp.zone);
        assert_eq!((rows, cols), (0b1_1110_0000, 0b1011_1000));
        assert!(fp.on(0, 1 << 4) && !fp.on(!0, !(1 << 4)));
        // AT along row 0 from its corner, on the empty board: nothing past the
        // edges
        let empty = vec![0u8; dim.rows as usize * dim.cols as usize];
        let fp = footprint(dim, &empty, &place(0, false, 0, 0, &[1, 20], 0).play);
        assert_eq!(fp.fills.0, squares(&[(0, 0), (0, 1)]));
        assert_eq!(
            fp.zone.0,
            squares(&[(0, 0), (0, 1), (0, 2), (1, 0), (1, 1)])
        );
        assert!(fp.on(1, 0) && !fp.on(!1, !0));
        // AT along row 0 from column 1, and along row 14 to column 13: the
        // squares just inside the edges are in the zone
        let fp = footprint(dim, &empty, &place(0, false, 0, 1, &[1, 20], 0).play);
        assert_eq!(
            fp.zone.0,
            squares(&[(0, 1), (0, 2), (0, 0), (0, 3), (1, 1), (1, 2)])
        );
        let fp = footprint(dim, &empty, &place(0, false, 14, 12, &[1, 20], 0).play);
        assert_eq!(
            fp.zone.0,
            squares(&[(14, 12), (14, 13), (14, 11), (14, 14), (13, 12), (13, 13)])
        );
        // a move placing no tiles has none
        let pass = movegen::Play::Exchange {
            tiles: [0u8; 0][..].into(),
        };
        let fp = footprint(dim, &board_tiles, &pass);
        assert_eq!(fp.zone.0, Squares::NONE.0);
        assert!(!fp.on(!0, !0));
    }

    // the words AT, ATE, and ATT, with ATTT as well when wanted
    #[inline]
    fn at_lexicon(with_attt: bool) -> Vec<u8> {
        let mut words = vec![
            [1, 20][..].into(),
            [1, 20, 5][..].into(),
            [1, 20, 20][..].into(),
        ];
        if with_attt {
            words.push([1, 20, 20, 20][..].into());
        }
        words.sort_unstable();
        build::build(
            build::BuildContent::Gaddawg,
            build::BuildLayout::Wolges,
            build::BuildOrder::Sorted,
            &words,
        )
        .unwrap()
        .to_vec()
    }

    // AT on row 7 from column 5, the bag empty, TT to move, the mover lead
    // points ahead, and the other side holding an E
    #[inline]
    fn at_endgame(game_config: &game_config::GameConfig, lead: i32) -> game_state::GameState {
        at_endgame_on(game_config, 7, lead)
    }

    // the same with AT on the row given
    #[inline]
    fn at_endgame_on(
        game_config: &game_config::GameConfig,
        row: usize,
        lead: i32,
    ) -> game_state::GameState {
        let mut game_state = game_state::GameState::new(game_config);
        game_state.bag.set_from_iter(std::iter::empty());
        let mover = game_state.turn as usize;
        game_state.players[mover].rack = vec![20, 20]; // TT
        game_state.players[1 - mover].rack = vec![5]; // E
        game_state.players[mover].score = (100 + lead) * equity::SCALE;
        game_state.players[1 - mover].score = 100 * equity::SCALE;
        game_state
            .board_tiles
            .copy_from_slice(&board_with(game_config, row, 0));
        game_state
    }

    // a choice's value on footprints alone, reckoned reply by reply as
    // value_after reckons it: the other side's moves the choice leaves standing
    // against the mover's outs it leaves standing, all on the board as it is
    #[inline]
    fn value_on_footprints_by_hand<N: kwg::Node, L: kwg::Node>(
        first: &OutInTwo<'_, N, L>,
        i: usize,
    ) -> i64 {
        let choice = &first.choices[i];
        let s_m = choice.score;
        let c = s_m + first.other_twice;
        if choice.left.is_empty() {
            return c;
        }
        let left_twice = twice(first.board_snapshot.game_config, &choice.left);
        let stuck = -(equity::ENDGAME_PENALTY_BASE as i64) - left_twice;
        let outs = first
            .standing_outs(choice, 0, 0)
            .map(|(s, fp)| (s, fp.zone))
            .collect::<Vec<_>>();
        let mut v = match outs.first() {
            Some(&(best, _)) => c + best,
            None => s_m + stuck,
        };
        for r in &first.replies {
            if choice.footprint.fills.meets(&r.footprint.zone) {
                continue;
            }
            if r.out {
                v = v.min(s_m - r.score - left_twice);
                continue;
            }
            match outs.iter().find(|(_, zone)| !r.footprint.fills.meets(zone)) {
                Some(&(s_u, _)) => v = v.min(c - r.weight + s_u),
                None => v = v.min(s_m - r.score + stuck),
            }
        }
        v
    }

    #[test]
    #[inline]
    fn out_in_two_values_each_choice_on_footprints() {
        let game_config = game_config::make_english_game_config();
        let kwg = kwg::Kwg::<kwg::Node22>::from_bytes_alloc(&at_lexicon(false)).unwrap();
        let klv = klv::Klv::<kwg::Node22>::from_bytes_alloc(klv::EMPTY_KLV_BYTES).unwrap();
        let board_tiles = board_with(&game_config, 7, 0);
        let board_snapshot = movegen::BoardSnapshot {
            board_tiles: &board_tiles,
            game_config: &game_config,
            kwg: &kwg,
            anagrams: None,
            rack_lengths: None,
            klv: &klv,
        };
        let game_state = at_endgame(&game_config, 0);
        let mut scratch = Scratch::new(&game_config);
        let first = OutInTwo::new(&board_snapshot, &game_state, &mut scratch).unwrap();
        let (values, hopes) = first.values_and_hopes();
        // the AT down column 5, ATT down it, and ATT along row 7 of the test
        // above, on footprints as on the board after each: no new play of
        // either side's changes their values
        for (play, want) in [
            (place(0, true, 5, 7, &[0, 20], 2).play, -6),
            (place(0, true, 5, 7, &[0, 20, 20], 5).play, 7),
            (place(0, false, 7, 5, &[0, 0, 20], 6).play, 10),
        ] {
            let i = first.choices.iter().position(|c| c.play == play).unwrap();
            assert_eq!(values[i], want * equity::SCALE as i64);
            assert_eq!(hopes[i], i64::MIN);
        }
        assert_eq!(first.choices.len(), 3);
    }

    #[test]
    #[inline]
    fn out_in_two_raises_a_choice_whose_move_opens_its_out() {
        let game_config = game_config::make_english_game_config();
        let kwg = kwg::Kwg::<kwg::Node22>::from_bytes_alloc(&at_lexicon(true)).unwrap();
        let klv = klv::Klv::<kwg::Node22>::from_bytes_alloc(klv::EMPTY_KLV_BYTES).unwrap();
        let board_tiles = board_with(&game_config, 14, 0);
        let board_snapshot = movegen::BoardSnapshot {
            board_tiles: &board_tiles,
            game_config: &game_config,
            kwg: &kwg,
            anagrams: None,
            rack_lengths: None,
            klv: &klv,
        };
        // on the bottom row, ATT (9, its T on a triple word) leaves the E no
        // move, and the T it keeps no out on the board as it is but ATTT, which
        // ATT itself opens (4): stuck there, 9 less the penalty and twice the
        // T; hoping for an out as good as ATT, 9, twice the E, and 9 again; and
        // with ATTT, 9, twice the E, and 4. ATTT at once (12) goes out
        let att = place(0, false, 14, 5, &[0, 0, 20], 9);
        let attt = place(0, false, 14, 5, &[0, 0, 20, 20], 12);
        let game_state = at_endgame_on(&game_config, 14, -15);
        let mut scratch = Scratch::new(&game_config);
        let first = OutInTwo::new(&board_snapshot, &game_state, &mut scratch).unwrap();
        assert_eq!(first.choices.len(), 2);
        let at = |play: &movegen::Play| first.choices.iter().position(|c| c.play == *play).unwrap();
        let (i, j) = (at(&att.play), at(&attt.play));
        let (mut values, hopes) = first.values_and_hopes();
        let scale = equity::SCALE as i64;
        assert_eq!((values[i], hopes[i]), (-3 * scale, 20 * scale));
        assert_eq!((values[j], hopes[j]), (14 * scale, i64::MIN));
        first.raise_openers(&mut scratch, &mut values, &hopes);
        assert_eq!((values[i], values[j]), (15 * scale, 14 * scale));
        // ATT and ATTT next beat ATTT at once unless that already wins
        for (lead, wins) in [(-15, false), (-14, false), (-13, true)] {
            let game_state = at_endgame_on(&game_config, 14, lead);
            let found = out_in_two(&board_snapshot, &game_state, std::slice::from_ref(&attt), 0);
            assert_eq!(found.is_none(), wins, "{lead} ahead");
            assert!(found.is_none_or(|play| play == att.play), "{lead} ahead");
        }
    }

    // a choice's value as OutInTwo::value_after reckons it, but from whole
    // generations of both racks on the board after it
    #[inline]
    fn value_after_in_full<N: kwg::Node, L: kwg::Node>(
        board_snapshot: &movegen::BoardSnapshot<'_, N, L>,
        game_state: &game_state::GameState,
        play: &movegen::Play,
        scratch: &mut Scratch,
    ) -> Option<i64> {
        let game_config = board_snapshot.game_config;
        let dim = game_config.board_layout().dim();
        let mover = game_state.turn as usize;
        let rack = &game_state.players[mover].rack;
        let other_rack = &game_state.players[1 - mover].rack;
        if !after_play(
            game_config,
            &game_state.board_tiles,
            rack,
            play,
            &mut scratch.after,
            &mut scratch.kept,
        ) {
            return None;
        }
        let s_m = play_score(play);
        let c = s_m + twice(game_config, other_rack);
        if scratch.kept.is_empty() {
            return Some(c);
        }
        let kept_twice = twice(game_config, &scratch.kept);
        let stuck = -(equity::ENDGAME_PENALTY_BASE as i64) - kept_twice;
        gen_plays(
            &mut scratch.generator,
            board_snapshot,
            &scratch.after,
            &scratch.kept,
            !0,
            !0,
        );
        let mut outs = scratch
            .generator
            .plays
            .iter()
            .filter(|m| tiles_placed(&m.play) == scratch.kept.len())
            .map(|m| {
                (
                    play_score(&m.play),
                    footprint(dim, &scratch.after, &m.play).zone,
                )
            })
            .collect::<Vec<_>>();
        outs.sort_by_key(|&(s, _)| std::cmp::Reverse(s));
        let mut v = match outs.first() {
            Some(&(best, _)) => c + best,
            None => s_m + stuck,
        };
        gen_plays(
            &mut scratch.generator,
            board_snapshot,
            &scratch.after,
            other_rack,
            !0,
            !0,
        );
        for m in scratch.generator.plays.iter() {
            if !matches!(m.play, movegen::Play::Place { .. }) {
                continue;
            }
            let s_x = play_score(&m.play);
            if tiles_placed(&m.play) == other_rack.len() {
                v = v.min(s_m - s_x - kept_twice);
                continue;
            }
            used_tiles(&m.play, &mut scratch.used);
            let w_x = s_x + twice(game_config, &scratch.used);
            let fills = footprint(dim, &scratch.after, &m.play).fills;
            match outs.iter().find(|(_, zone)| !fills.meets(zone)) {
                Some(&(s_u, _)) => v = v.min(c - w_x + s_u),
                None => v = v.min(s_m - s_x + stuck),
            }
        }
        Some(v)
    }

    #[test]
    #[inline]
    fn out_in_two_blocks_an_out_and_keeps_a_move_that_wins() {
        let game_config = game_config::make_english_game_config();
        let kwg = kwg::Kwg::<kwg::Node22>::from_bytes_alloc(&at_lexicon(false)).unwrap();
        let klv = klv::Klv::<kwg::Node22>::from_bytes_alloc(klv::EMPTY_KLV_BYTES).unwrap();
        let board_tiles = board_with(&game_config, 7, 0);
        let board_snapshot = movegen::BoardSnapshot {
            board_tiles: &board_tiles,
            game_config: &game_config,
            kwg: &kwg,
            anagrams: None,
            rack_lengths: None,
            klv: &klv,
        };
        // AT down column 5 (2) leaves the E its ATE along row 7 (6, on the
        // center's double word) and down column 5: 2 less 6 less twice the T
        // kept. ATT along row 7 (6) leaves the E no move and the T its AT down:
        // 6, twice the E, and 2
        let at_down = place(0, true, 5, 7, &[0, 20], 2);
        let att_along = place(0, false, 7, 5, &[0, 0, 20], 6).play;
        let game_state = at_endgame(&game_config, 0);
        let mut scratch = Scratch::new(&game_config);
        let first = OutInTwo::new(&board_snapshot, &game_state, &mut scratch).unwrap();
        let value_of = |play: &movegen::Play, scratch: &mut Scratch| {
            let i = first.choices.iter().position(|c| c.play == *play).unwrap();
            first.value_after(scratch, i)
        };
        assert_eq!(
            value_of(&at_down.play, &mut scratch),
            Some(-6 * equity::SCALE as i64)
        );
        assert_eq!(
            value_of(&att_along, &mut scratch),
            Some(10 * equity::SCALE as i64)
        );
        for (lead, blocks) in [(-20, true), (6, true), (7, false)] {
            let game_state = at_endgame(&game_config, lead);
            let found = out_in_two(
                &board_snapshot,
                &game_state,
                std::slice::from_ref(&at_down),
                0,
            );
            assert_eq!(found.is_some(), blocks, "{lead} ahead");
            assert!(
                found.is_none_or(|play| play == att_along),
                "{lead} ahead: not ATT along row 7"
            );
        }
    }

    #[test]
    #[inline]
    fn out_in_two_keeps_the_play_at_fewest_on_a_tie() {
        let game_config = game_config::make_english_game_config();
        let dim = game_config.board_layout().dim();
        let words: Vec<bites::Bites> = vec![[1u8, 20][..].into()]; // AT alone
        let kwg = kwg::Kwg::<kwg::Node22>::from_bytes_alloc(
            &build::build(
                build::BuildContent::Gaddawg,
                build::BuildLayout::Wolges,
                build::BuildOrder::Sorted,
                &words,
            )
            .unwrap(),
        )
        .unwrap();
        let klv = klv::Klv::<kwg::Node22>::from_bytes_alloc(klv::EMPTY_KLV_BYTES).unwrap();
        let mut board_tiles = vec![0u8; dim.rows as usize * dim.cols as usize];
        board_tiles[dim.at_row_col(7, 7)] = 1; // A
        let board_snapshot = movegen::BoardSnapshot {
            board_tiles: &board_tiles,
            game_config: &game_config,
            kwg: &kwg,
            anagrams: None,
            rack_lengths: None,
            klv: &klv,
        };
        let mut game_state = at_endgame(&game_config, -10);
        game_state.board_tiles.copy_from_slice(&board_tiles);
        // AT along row 7 and AT down column 7 (2 each) mirror each other: each
        // leaves the E no move and the T kept the other, and is worth 2, twice
        // the E, and 2 again
        let along = place(0, false, 7, 7, &[0, 20], 2);
        let down = place(0, true, 7, 7, &[0, 20], 2);
        let mut scratch = Scratch::new(&game_config);
        let first = OutInTwo::new(&board_snapshot, &game_state, &mut scratch).unwrap();
        assert_eq!(first.choices.len(), 2);
        for play in [&along.play, &down.play] {
            let i = first.choices.iter().position(|c| c.play == *play).unwrap();
            assert_eq!(
                first.value_after(&mut scratch, i),
                Some(6 * equity::SCALE as i64)
            );
        }
        let found = out_in_two(
            &board_snapshot,
            &game_state,
            std::slice::from_ref(&along),
            0,
        );
        assert!(
            found.is_none(),
            "a move only as good replaced the play at fewest"
        );
    }

    #[test]
    #[inline]
    fn out_in_two_sees_an_out_its_move_opens() {
        let game_config = game_config::make_english_game_config();
        let kwg = kwg::Kwg::<kwg::Node22>::from_bytes_alloc(&at_lexicon(true)).unwrap();
        let klv = klv::Klv::<kwg::Node22>::from_bytes_alloc(klv::EMPTY_KLV_BYTES).unwrap();
        let board_tiles = board_with(&game_config, 7, 0);
        let board_snapshot = movegen::BoardSnapshot {
            board_tiles: &board_tiles,
            game_config: &game_config,
            kwg: &kwg,
            anagrams: None,
            rack_lengths: None,
            klv: &klv,
        };
        // ATT along row 7 (6) opens ATTT (4) for the T it keeps, beside its AT
        // down (2): 6, twice the E, and 4; ATTT at once goes out for 8 and
        // twice the E
        let at_down = place(0, true, 5, 7, &[0, 20], 2);
        let att_along = place(0, false, 7, 5, &[0, 0, 20], 6).play;
        let attt_along = place(0, false, 7, 5, &[0, 0, 20, 20], 8).play;
        let game_state = at_endgame(&game_config, 0);
        let mut scratch = Scratch::new(&game_config);
        let first = OutInTwo::new(&board_snapshot, &game_state, &mut scratch).unwrap();
        let value_of = |play: &movegen::Play, scratch: &mut Scratch| {
            let i = first.choices.iter().position(|c| c.play == *play).unwrap();
            first.value_after(scratch, i)
        };
        assert_eq!(
            value_of(&att_along, &mut scratch),
            Some(12 * equity::SCALE as i64)
        );
        assert_eq!(
            value_of(&attt_along, &mut scratch),
            Some(10 * equity::SCALE as i64)
        );
        let found = out_in_two(&board_snapshot, &game_state, &[at_down], 0);
        assert!(found.is_some_and(|play| play == att_along));
    }

    // deft with the bag empty: going out when that wins, else out in two's move
    #[test]
    #[inline]
    fn deft_plays_out_in_two_when_going_out_would_lose() {
        let game_config = game_config::make_english_game_config();
        let kwg = kwg::Kwg::<kwg::Node22>::from_bytes_alloc(&at_lexicon(false)).unwrap();
        let klv = klv::Klv::<kwg::Node22>::from_bytes_alloc(klv::EMPTY_KLV_BYTES).unwrap();
        let board_tiles = board_with(&game_config, 7, 0);
        let board_snapshot = movegen::BoardSnapshot {
            board_tiles: &board_tiles,
            game_config: &game_config,
            kwg: &kwg,
            anagrams: None,
            rack_lengths: None,
            klv: &klv,
        };
        // ATT down column 5 goes out for 5 (its last T on a triple letter) and
        // twice the E; ATT along row 7 is worth 10 by out in two
        let att_down = place(0, true, 5, 7, &[0, 20, 20], 5).play;
        let att_along = place(0, false, 7, 5, &[0, 0, 20], 6).play;
        for (lead, want) in [(-6, &att_down), (-7, &att_along), (-9, &att_along)] {
            let game_state = at_endgame(&game_config, lead);
            let mut move_generator = movegen::KurniaMoveGenerator::new(&game_config);
            let mut filtered_movegen = move_filter::GenMoves::Unfiltered;
            let mut deft = MovePicker::<kwg::Node22, kwg::Node22>::Deft(DEFAULT_DEFT_WINDOW);
            deft.pick_a_move(
                &mut filtered_movegen,
                &mut move_generator,
                &board_snapshot,
                &game_state,
                &game_state.players[game_state.turn as usize].rack,
            )
            .unwrap();
            assert_eq!(move_generator.plays.len(), 1);
            assert!(move_generator.plays[0].play == *want, "{lead} ahead");
        }
    }

    // deft with two to a rack's worth of tiles in the bag: ATT along row 7 (6)
    // is its best and keeps a T, ATT down column 5 (5) plays both
    #[test]
    #[inline]
    fn deft_leaves_one_in_the_bag_when_a_move_near_its_best_can() {
        let game_config = game_config::make_english_game_config();
        let kwg = kwg::Kwg::<kwg::Node22>::from_bytes_alloc(&at_lexicon(false)).unwrap();
        let klv = klv::Klv::<kwg::Node22>::from_bytes_alloc(klv::EMPTY_KLV_BYTES).unwrap();
        let board_tiles = board_with(&game_config, 7, 0);
        let board_snapshot = movegen::BoardSnapshot {
            board_tiles: &board_tiles,
            game_config: &game_config,
            kwg: &kwg,
            anagrams: None,
            rack_lengths: None,
            klv: &klv,
        };
        let att_down = place(0, true, 5, 7, &[0, 20, 20], 5).play;
        let att_along = place(0, false, 7, 5, &[0, 0, 20], 6).play;
        for (bag, want) in [(3, &att_down), (4, &att_along)] {
            let mut game_state = at_endgame(&game_config, 0);
            game_state.bag.set_from_iter(std::iter::repeat_n(5u8, bag));
            let mut move_generator = movegen::KurniaMoveGenerator::new(&game_config);
            let mut filtered_movegen = move_filter::GenMoves::Unfiltered;
            let mut deft = MovePicker::<kwg::Node22, kwg::Node22>::Deft(0);
            deft.pick_a_move(
                &mut filtered_movegen,
                &mut move_generator,
                &board_snapshot,
                &game_state,
                &game_state.players[game_state.turn as usize].rack,
            )
            .unwrap();
            assert!(move_generator.plays[0].play == *want, "bag {bag}");
        }
    }

    // words of the letters A, C, E, N, R, S, and T
    static END_WORDS: &[&str] = &[
        "AE", "AN", "AR", "AS", "AT", "EA", "EN", "ER", "ES", "ET", "NA", "NE", "RE", "ST", "TA",
        "TE", "ANT", "ANTS", "ARE", "ART", "ARTS", "ATE", "CAN", "CANE", "CANES", "CANS", "CAR",
        "CARE", "CARES", "CARS", "CART", "CARTS", "CASE", "CAST", "CASTE", "CAT", "CATS", "CENT",
        "CENTS", "CRATE", "CRATES", "EAR", "EARN", "EARNS", "EARS", "EAST", "EAT", "EATS", "ERA",
        "ERAS", "NEAR", "NEARS", "NEAT", "NEST", "NET", "NETS", "RACE", "RACES", "RAN", "RANT",
        "RANTS", "RAT", "RATE", "RATES", "RATS", "REACT", "REACTS", "SAT", "SCAN", "SCANT", "SCAR",
        "SCARE", "SEA", "SEAT", "SEATS", "SENT", "SET", "STAR", "STARE", "START", "STERN", "TAN",
        "TANS", "TAR", "TARE", "TARS", "TART", "TARTS", "TEA", "TEAR", "TEARS", "TEAS", "TEN",
        "TENS", "TENT", "TENTS", "TRACE", "TRACES", "TRANCE", "TRANCES", "TREAT", "TREATS",
    ];

    #[inline]
    fn end_words_kwg(game_config: &game_config::GameConfig) -> kwg::Kwg<kwg::Node22> {
        let reader = alphabet::AlphabetReader::new_for_words(game_config.alphabet());
        let mut word_buf = Vec::new();
        let mut words = Vec::<bites::Bites>::new();
        for w in END_WORDS {
            reader.set_word(w, &mut word_buf).unwrap();
            words.push(word_buf[..].into());
        }
        words.sort_unstable();
        kwg::Kwg::<kwg::Node22>::from_bytes_alloc(
            &build::build(
                build::BuildContent::Gaddawg,
                build::BuildLayout::Wolges,
                build::BuildOrder::Sorted,
                &words,
            )
            .unwrap(),
        )
        .unwrap()
    }

    // deft's picks on a board and racks, its move as the movegen prints it
    #[inline]
    fn deft_picks<N: kwg::Node + Sync, L: kwg::Node + Sync>(
        board_snapshot: &movegen::BoardSnapshot<'_, N, L>,
        game_state: &game_state::GameState,
    ) -> String {
        let mut move_generator = movegen::KurniaMoveGenerator::new(board_snapshot.game_config);
        let mut filtered_movegen = move_filter::GenMoves::Unfiltered;
        let mut deft = MovePicker::<N, L>::Deft(DEFAULT_DEFT_WINDOW);
        deft.pick_a_move(
            &mut filtered_movegen,
            &mut move_generator,
            board_snapshot,
            game_state,
            &game_state.players[game_state.turn as usize].rack,
        )
        .unwrap();
        assert_eq!(move_generator.plays.len(), 1);
        move_generator.plays[0].play.fmt(board_snapshot).to_string()
    }

    // with the bag empty, deft goes out when that wins even where out in two
    // would keep another move that it also values as a win: holding ACERST
    // against an A, CRATES along row 12 (23) goes out, while CASTE (28), the
    // move at fewest, keeps the R
    #[test]
    #[inline]
    fn deft_takes_a_winning_out_over_a_richer_move_that_also_wins() {
        let game_config = game_config::make_english_game_config();
        let kwg = end_words_kwg(&game_config);
        let klv = klv::Klv::<kwg::Node22>::from_bytes_alloc(klv::EMPTY_KLV_BYTES).unwrap();
        let mut fen_parser =
            display::BoardFenParser::new(game_config.alphabet(), game_config.board_layout());
        let board_tiles = fen_parser
            .parse("15/15/15/15/15/15/15/6CARE5/6A8/6N8/6EATS5/15/15/15/15")
            .unwrap()
            .to_vec();
        let board_snapshot = movegen::BoardSnapshot {
            board_tiles: &board_tiles,
            game_config: &game_config,
            kwg: &kwg,
            anagrams: None,
            rack_lengths: None,
            klv: &klv,
        };
        let rack_reader = alphabet::AlphabetReader::new_for_racks(game_config.alphabet());
        let mut game_state = game_state::GameState::new(&game_config);
        game_state.board_tiles.copy_from_slice(&board_tiles);
        game_state.bag.set_from_iter(std::iter::empty());
        let mover = game_state.turn as usize;
        rack_reader
            .set_word("ACERST", &mut game_state.players[mover].rack)
            .unwrap();
        rack_reader
            .set_word("A", &mut game_state.players[1 - mover].rack)
            .unwrap();
        assert_eq!(deft_picks(&board_snapshot, &game_state), "12B CRATES 23");
        // without its winning out, deft would keep CASTE
        let mut move_generator = movegen::KurniaMoveGenerator::new(&game_config);
        let mut filtered_movegen = move_filter::GenMoves::Unfiltered;
        filtered_movegen.gen_moves(
            &mut move_generator,
            &board_snapshot,
            &game_state.players[mover].rack,
            0,
            DEFT_LATE_FIELD,
            None,
        );
        let plays = &move_generator.plays;
        let fewest = lane_pick(&game_config, &board_tiles, plays, DEFAULT_DEFT_WINDOW);
        assert_eq!(
            plays[fewest].play.fmt(&board_snapshot).to_string(),
            "F6 CASTE 28"
        );
        assert!(out_in_two(&board_snapshot, &game_state, plays, fewest).is_none());
    }

    // with the bag empty and no out that wins, deft plays out in two's move:
    // holding ERS against AT 30 behind, E(A)RS (7) goes out, where the play at
    // fewest is ES (13), which keeps the R
    #[test]
    #[inline]
    fn deft_plays_out_in_two_over_the_play_at_fewest() {
        let game_config = game_config::make_english_game_config();
        let kwg = end_words_kwg(&game_config);
        let klv = klv::Klv::<kwg::Node22>::from_bytes_alloc(klv::EMPTY_KLV_BYTES).unwrap();
        let mut fen_parser =
            display::BoardFenParser::new(game_config.alphabet(), game_config.board_layout());
        let board_tiles = fen_parser
            .parse("15/15/15/15/15/15/15/6CARE5/6A8/6N8/6EATS5/15/15/15/15")
            .unwrap()
            .to_vec();
        let board_snapshot = movegen::BoardSnapshot {
            board_tiles: &board_tiles,
            game_config: &game_config,
            kwg: &kwg,
            anagrams: None,
            rack_lengths: None,
            klv: &klv,
        };
        let rack_reader = alphabet::AlphabetReader::new_for_racks(game_config.alphabet());
        let mut game_state = game_state::GameState::new(&game_config);
        game_state.board_tiles.copy_from_slice(&board_tiles);
        game_state.bag.set_from_iter(std::iter::empty());
        let mover = game_state.turn as usize;
        rack_reader
            .set_word("ERS", &mut game_state.players[mover].rack)
            .unwrap();
        rack_reader
            .set_word("AT", &mut game_state.players[1 - mover].rack)
            .unwrap();
        game_state.players[mover].score = 170 * equity::SCALE;
        game_state.players[1 - mover].score = 200 * equity::SCALE;
        assert_eq!(deft_picks(&board_snapshot, &game_state), "H10 E(A)RS 7");
        let mut move_generator = movegen::KurniaMoveGenerator::new(&game_config);
        let mut filtered_movegen = move_filter::GenMoves::Unfiltered;
        filtered_movegen.gen_moves(
            &mut move_generator,
            &board_snapshot,
            &game_state.players[mover].rack,
            0,
            DEFT_LATE_FIELD,
            None,
        );
        let plays = &move_generator.plays;
        let fewest = lane_pick(&game_config, &board_tiles, plays, DEFAULT_DEFT_WINDOW);
        assert_eq!(
            plays[fewest].play.fmt(&board_snapshot).to_string(),
            "F10 ES 13"
        );
        assert_eq!(winning_out(&game_config, &game_state, plays), None);
    }

    // once the bag holds a rack or less, deft weighs more moves: holding ETT
    // with three tiles in the bag, the first move near its best that leaves
    // exactly one there is not among its usual field
    #[test]
    #[inline]
    fn deft_weighs_more_moves_near_the_end_of_the_bag() {
        let game_config = game_config::make_english_game_config();
        let kwg = kwg::Kwg::<kwg::Node22>::from_bytes_alloc(&tiny_lexicon()).unwrap();
        let klv = klv::Klv::<kwg::Node22>::from_bytes_alloc(klv::EMPTY_KLV_BYTES).unwrap();
        let board_tiles = board_with(&game_config, 7, 0);
        let board_snapshot = movegen::BoardSnapshot {
            board_tiles: &board_tiles,
            game_config: &game_config,
            kwg: &kwg,
            anagrams: None,
            rack_lengths: None,
            klv: &klv,
        };
        let mut game_state = game_state::GameState::new(&game_config);
        game_state.board_tiles.copy_from_slice(&board_tiles);
        game_state.bag.set_from_iter(std::iter::repeat_n(5u8, 3));
        let mover = game_state.turn as usize;
        game_state.players[mover].rack = vec![5, 20, 20]; // ETT
        assert_eq!(deft_picks(&board_snapshot, &game_state), "H7 ET 10");
        // that move ranks past the usual field
        let mut move_generator = movegen::KurniaMoveGenerator::new(&game_config);
        let mut filtered_movegen = move_filter::GenMoves::Unfiltered;
        filtered_movegen.gen_moves(
            &mut move_generator,
            &board_snapshot,
            &game_state.players[mover].rack,
            0,
            DEFT_LATE_FIELD,
            None,
        );
        let at = move_generator
            .plays
            .iter()
            .position(|m| m.play.fmt(&board_snapshot).to_string() == "H7 ET 10")
            .unwrap();
        assert!(at >= DEFT_FIELD, "it ranks {at}");
    }

    // every choice valued on its zone's lanes alone is valued as on the whole
    // board after it, and on footprints as reckoned reply by reply
    #[test]
    #[inline]
    fn out_in_two_values_each_choice_as_the_slow_ways_do() {
        let game_config = game_config::make_english_game_config();
        let kwg = end_words_kwg(&game_config);
        let mut word_buf = Vec::new();
        let klv = klv::Klv::<kwg::Node22>::from_bytes_alloc(klv::EMPTY_KLV_BYTES).unwrap();
        let rack_reader = alphabet::AlphabetReader::new_for_racks(game_config.alphabet());
        let mut fen_parser =
            display::BoardFenParser::new(game_config.alphabet(), game_config.board_layout());
        let mut valued = 0;
        for fen in [
            "15/15/15/15/15/15/15/6CARE5/6A8/6N8/6EATS5/15/15/15/15",
            "15/15/15/15/15/15/15/4TRANCE5/15/6NEST5/15/15/15/15/15",
        ] {
            let board_tiles = fen_parser.parse(fen).unwrap().to_vec();
            let board_snapshot = movegen::BoardSnapshot {
                board_tiles: &board_tiles,
                game_config: &game_config,
                kwg: &kwg,
                anagrams: None,
                rack_lengths: None,
                klv: &klv,
            };
            // the last four give the other side more tiles than it can go out
            // with, so its blocks set many values
            for (mine, theirs) in [
                ("ERS", "AT"),
                ("ACT", "NE"),
                ("ARTS", "EN"),
                ("E", "ST"),
                ("NRT", "AES"),
                ("CENT", "RA"),
                ("ERS", "CENS"),
                ("ACT", "ARRT"),
                ("ARTS", "CCNR"),
                ("NRT", "EENRT"),
            ] {
                let mut game_state = game_state::GameState::new(&game_config);
                game_state.bag.set_from_iter(std::iter::empty());
                game_state.board_tiles.copy_from_slice(&board_tiles);
                let mover = game_state.turn as usize;
                rack_reader.set_word(mine, &mut word_buf).unwrap();
                game_state.players[mover].rack = word_buf.clone();
                rack_reader.set_word(theirs, &mut word_buf).unwrap();
                game_state.players[1 - mover].rack = word_buf.clone();
                let mut scratch = Scratch::new(&game_config);
                let mut whole = Scratch::new(&game_config);
                let first = OutInTwo::new(&board_snapshot, &game_state, &mut scratch).unwrap();
                let (values, _) = first.values_and_hopes();
                for (i, choice) in first.choices.iter().enumerate() {
                    assert_eq!(
                        first.value_after(&mut scratch, i),
                        value_after_in_full(&board_snapshot, &game_state, &choice.play, &mut whole),
                        "{fen} {mine} against {theirs}: {}",
                        choice.play.fmt(&board_snapshot),
                    );
                    assert_eq!(
                        values[i],
                        value_on_footprints_by_hand(&first, i),
                        "{fen} {mine} against {theirs}, on footprints: {}",
                        choice.play.fmt(&board_snapshot),
                    );
                    valued += 1;
                }
            }
        }
        assert!(valued > 100, "only {valued} choices valued");
    }
}
