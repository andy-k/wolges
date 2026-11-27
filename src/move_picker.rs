// Copyright (C) 2020-2026 Andy Kurnia.

use super::zone::{Footprint, SQUARE_WORDS, Squares, footprint_on_board, zone_lanes_on_board};
use super::{
    anagrams, bites, build, endgame, equity, error, game_config, game_state, glance, infer, klv,
    kwg, move_filter, movegen, simmer, stats, win_pct,
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
pub const DEFT_FIELD: usize = 8;
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
) -> error::Returns<()> {
    gen_simmer_field(
        filtered_movegen,
        move_generator,
        &movegen::GenMovesParams {
            board_snapshot,
            rack,
            num_exchanges_by_this_player,
            max_gen: field.max_gen,
            pass_policy: movegen::PassPolicy::OnlyWhenForced,
            dynamic_leaves: None,
        },
        field.nonplacing_quota,
        scratch,
    )
}

#[inline]
fn gen_simmer_field<N: kwg::Node, L: kwg::Node>(
    filtered_movegen: &mut move_filter::GenMoves<'_>,
    move_generator: &mut movegen::KurniaMoveGenerator,
    params: &movegen::GenMovesParams<'_, N, L>,
    nonplacing_quota: usize,
    scratch: &mut Vec<movegen::ValuedMove>,
) -> error::Returns<()> {
    let max_gen = params.max_gen;
    filtered_movegen.gen_moves(
        move_generator,
        params.board_snapshot,
        params.rack,
        params.num_exchanges_by_this_player,
        max_gen,
        params.dynamic_leaves,
    )?;
    let quota = nonplacing_quota.min(max_gen);
    let mut held = move_generator
        .plays
        .iter()
        .filter(|valued_move| places_no_tiles(&valued_move.play))
        .count();
    if held >= quota {
        return Ok(());
    }
    let pass_policy = if move_generator.num_tiles_in_bag()
        < i32::from(params.board_snapshot.game_config.exchange_tile_limit())
    {
        movegen::PassPolicy::AsACandidate
    } else {
        movegen::PassPolicy::OnlyWhenForced
    };
    std::mem::swap(&mut move_generator.plays, scratch);
    let generated = filtered_movegen.gen_nonplacing_moves_with(
        move_generator,
        &movegen::GenMovesParams {
            max_gen: quota,
            pass_policy,
            dynamic_leaves: params
                .dynamic_leaves
                .map(|dynamic_leaves| klv::DynamicLeavesRef {
                    lat: dynamic_leaves.lat,
                    add: dynamic_leaves.add,
                    full_v: dynamic_leaves.full_v,
                    min_keep: dynamic_leaves.min_keep,
                    take_kept: true,
                }),
            ..*params
        },
    );
    if let Err(e) = generated {
        std::mem::swap(&mut move_generator.plays, scratch);
        return Err(e);
    }
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
    Ok(())
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

// where in [0, 1) the iteration's stratum is picked: steps of the golden ratio
// from a start the decision seed sets, so any run of iterations meets each
// stratum about as often as its chance
#[inline(always)]
pub fn stratum_at(decision_seed: u64, sim_iter: u64) -> f64 {
    let bits = f64::MANTISSA_DIGITS;
    let start = (mix(decision_seed, 0) >> (u64::BITS - bits)) as f64 / (1u64 << bits) as f64;
    let step = (5f64.sqrt() - 1.0) / 2.0;
    (start + sim_iter as f64 * step).fract()
}

// how many iterations apart the looks that retire candidates come
pub const PRUNE_CADENCE: u64 = 16;

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
        simmer.compute_win_prob(game_ended, final_spread, table)?
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
pub fn paired_z_among(budget: u64, candidates: usize) -> f64 {
    let looks = (budget / PRUNE_CADENCE).max(1) as f64;
    let others = candidates.saturating_sub(1).max(1) as f64;
    -normal_quantile(0.05 / (looks * others))
}

#[inline]
pub fn z_among(z: f64, candidates: usize) -> f64 {
    let others = candidates.saturating_sub(1).max(1) as f64;
    let (mut lo, mut hi) = (0.0f64, 0.5f64);
    loop {
        let mid = 0.5 * (lo + hi);
        if mid <= lo || mid >= hi {
            break;
        }
        if -normal_quantile(mid) > z {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    -normal_quantile(hi / others)
}

#[inline]
pub fn paired_z(budget: u64) -> f64 {
    paired_z_among(budget, 2)
}

// the one-sided z at a sim's iterations so far of the stitched boundary of
// Howard, Ramdas, McAuliffe, and Sekhon, "Time-uniform, nonparametric,
// nonasymptotic confidence sequences", Ann. Statist. 49(2) 2021, eq. (2) (its
// two-sided 10.4 / alpha made one-sided, 5.2 / alpha): it holds at every look
// at once, so however long a sim with no iteration budget runs, the chance it
// ever retires its best candidate behind a given other one is at most alpha.
// Iterations at least 1, alpha above 0 and below 1
#[inline]
pub fn stitched_z(iterations: u64, alpha: f64) -> f64 {
    1.7 * ((2.0 * iterations as f64).ln().ln() + 0.72 * (5.2 / alpha).ln()).sqrt()
}

// a candidate's objective minus the leader's on each iteration both were rolled
// out. Each list holds a candidate's objectives from its first iteration on,
// NaN where it sat one out.
#[inline]
pub fn paired_difference(
    objectives: &[f64],
    first_iter: u64,
    lead_objectives: &[f64],
    lead_first_iter: u64,
) -> stats::Stats {
    paired_difference_of(
        (|i| objectives[i], objectives.len(), first_iter),
        (
            |i| lead_objectives[i],
            lead_objectives.len(),
            lead_first_iter,
        ),
    )
}

// the same of objectives read one at a time, each list as its objective at
// each place, its length, and its first iteration, so that a caller whose
// objectives are made of other lists need not list them
#[inline]
pub fn paired_difference_of(
    (objective, len, first_iter): (impl Fn(usize) -> f64, usize, u64),
    (lead_objective, lead_len, lead_first_iter): (impl Fn(usize) -> f64, usize, u64),
) -> stats::Stats {
    let from = first_iter.max(lead_first_iter);
    let to = (first_iter + len as u64).min(lead_first_iter + lead_len as u64);
    let mut diff = stats::Stats::new();
    for iter in from..to {
        let mine = objective((iter - first_iter) as usize);
        let leads = lead_objective((iter - lead_first_iter) as usize);
        if !mine.is_nan() && !leads.is_nan() {
            diff.update(mine - leads);
        }
    }
    diff
}

// whether a candidate is behind the leader on the iterations both were rolled
// out: its objective minus the leader's is below 0 even at the top of its
// interval
#[inline]
pub fn student_t_of_normal_by_abramowitz_stegun_26_7_5(z: f64, degrees_of_freedom: f64) -> f64 {
    let x2 = z * z;
    let g1 = (x2 + 1.0) * z / 4.0;
    let g2 = ((5.0 * x2 + 16.0) * x2 + 3.0) * z / 96.0;
    let g3 = (((3.0 * x2 + 19.0) * x2 + 17.0) * x2 - 15.0) * z / 384.0;
    let g4 = ((((79.0 * x2 + 776.0) * x2 + 1482.0) * x2 - 1920.0) * x2 - 945.0) * z / 92160.0;
    let v = degrees_of_freedom;
    z + (g1 + (g2 + (g3 + g4 / v) / v) / v) / v
}

#[inline]
pub fn behind_paired(
    objectives: &[f64],
    first_iter: u64,
    lead_objectives: &[f64],
    lead_first_iter: u64,
    z: f64,
) -> bool {
    let diff = paired_difference(objectives, first_iter, lead_objectives, lead_first_iter);
    let n = diff.count();
    n >= 2.0 && diff.ci_max(student_t_of_normal_by_abramowitz_stegun_26_7_5(z, n - 1.0)) < 0.0
}

// compare each candidate with the leader (the highest mean) on the iterations
// both were rolled out: a candidate retires when its objective minus the
// leader's is below 0 even at the top of its interval
#[inline(always)]
fn retire_paired(candidates: &mut Vec<Candidate>, retired: &mut Vec<Candidate>, z: f64) {
    let mut lead = candidates
        .iter()
        .enumerate()
        .max_by(|(_, a), (_, b)| a.stats.mean().total_cmp(&b.stats.mean()))
        .unwrap()
        .0;
    let mut write = 0;
    for read in 0..candidates.len() {
        let (candidate, leader) = (&candidates[read], &candidates[lead]);
        if !behind_paired(
            &candidate.objectives,
            candidate.first_iter,
            &leader.objectives,
            leader.first_iter,
            z,
        ) {
            candidates.swap(write, read);
            if lead == write {
                lead = read;
            } else if lead == read {
                lead = write;
            }
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

// whether a decision builds its late graph, the graph of the words the board
// can still take. The build costs in proportion to the words it keeps (they
// grow fast with the bag, more so with a blank to place) while the graph saves
// in proportion to the rollouts the budget runs. On one thread the two meet
// where the bag holds 25 - 3000 / (budget + 200) tiles, the budget in
// iterations, or seven fewer while a blank is off the board. That is where they
// met over classic games in ten lexicons on both boards with racks of seven to
// nine tiles, jumbled games too. Over threads the build runs alone while the
// threads share the saving, so each thread's share of the budget counts: 28 -
// 5250 / (share + 200), six fewer with a blank off, fit by the clock on two to
// eight threads with budgets of 100 to 3000
#[inline]
pub fn late_graph_pays(bag: usize, blank_off_board: bool, budget: u64, threads: usize) -> bool {
    if threads <= 1 {
        let blank_shift = if blank_off_board { 7.0 } else { 0.0 };
        return bag as f64 <= 25.0 - 3000.0 / (budget as f64 + 200.0) - blank_shift;
    }
    let blank_shift = if blank_off_board { 6.0 } else { 0.0 };
    let share = budget as f64 / threads as f64;
    bag as f64 <= 28.0 - 5250.0 / (share + 200.0) - blank_shift
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
    game_config: &'a std::sync::Arc<game_config::GameConfig>,
    kwg: &'a std::sync::Arc<kwg::Kwg<N>>,
    arm_tables: &'a std::sync::Arc<anagrams::ArmTables>,
    klv: &'a std::sync::Arc<klv::Klv<L>>,
    candidates: Vec<Candidate>,
    simmer: simmer::Simmer<R>,
    num_sim_iters: u64,
    num_sim_plies: usize,
    retired: Vec<Candidate>,
    spare_objectives: Vec<Vec<f64>>, // emptied, for the next candidates
    iters_done: u64,
    next_stream_id: u64,
    win_pct_table: Option<&'a std::sync::Arc<win_pct::WinPctTable>>,
    #[cfg(not(target_family = "wasm"))]
    sim_threads: usize,
    decision_seed: u64,
    candidate_scratch: Vec<movegen::ValuedMove>,
    dynamic_root: Option<klv::DynamicLeavesRef<'a>>,
    turn_time: Option<std::time::Duration>,
    turn_clock: Option<(std::time::Instant, std::time::Duration)>,
    timed_decisions: u64,
    decisions_without_rollouts: u64,
    prepared_pristine: game_state::GameState,
    quiet: bool,
    decision_t0: Option<std::time::Instant>, // set only when not quiet
    ticked: u64,
    infer: Option<(infer::InferTables, movegen::KurniaMoveGenerator)>, // built at its first use
    inferred: std::sync::Arc<infer::PackedLeaves>, // the opponent's kept leaves, in place
    infer_fuzz: (f64, f64), // the fuzz and prior mix its inference weighs with, the default unless set
    rollout_klv: std::sync::Arc<klv::Klv<L>>, // the leaves its rollouts value with
    late_graph: Option<(kwg::Kwg<kwg::Node24>, anagrams::ArmTables)>,
    late_board: Box<[u8]>, // the board the late graph was pruned for
    late_prune: LatePrune, // what each late graph is pruned with
    memory: Option<endgame::MemoryLimit>, // what the program stays within
    thread_kept: Vec<SimWorker<R>>, // the sim's threads, kept for its life
    spare_plays: Vec<usize>, // the last block's plays, for the next
}

// what a sim's late graph is pruned with, kept from one decision to the next:
// the generator (made at its first use), the words, the builder, the graph of
// no late graph, whose nodes the next one is made in, and the tables of no
// late graph and the scratch, in whose memory the next one's are built
struct LatePrune {
    generator: Option<movegen::KurniaMoveGenerator>,
    words: Vec<bites::Bites>,
    builder: build::Builder,
    spare: kwg::Kwg<kwg::Node24>,
    spare_tables: anagrams::ArmTables,
    arm_scratch: anagrams::ArmScratch,
}

// the opponent's last move, the board before it, and what the watcher knew of
// their rack before it, from which a sim infers what they kept
#[derive(Clone, Copy)]
pub struct LastMove<'a> {
    pub board_before: &'a [u8],
    pub play: &'a movegen::Play,
    pub held: infer::Held<'a>,
}

// a block of iterations, those after lo up to hi, rolled out for the plays
// still in (indices into the move generator's plays), in the order the rows
// give them
pub struct SimBlock {
    pub lo: u64,
    pub hi: u64,
    pub plays: Vec<usize>,
}

// a late graph and its tables
type LateGraph = (kwg::Kwg<kwg::Node24>, anagrams::ArmTables);

// a sim thread: where its jobs go (the iterations after lo up to hi, and the
// decision's seed), where it says each is done, and what it works in: its
// simmer, its rows, and the state each iteration is restored to; and what all
// the threads share: the plays of the block they roll out, and the late graph
// they roll out on while a run lends it to them
type SimWorker<R> = (
    std::sync::mpsc::SyncSender<(u64, u64, u64)>,
    std::sync::mpsc::Receiver<error::Returns<()>>,
    std::sync::Arc<std::sync::Mutex<(simmer::Simmer<R>, SimRows, game_state::GameState)>>,
    std::sync::Arc<std::sync::RwLock<Vec<movegen::Play>>>,
    std::sync::Arc<std::sync::RwLock<Option<LateGraph>>>,
);

// a block's rows, by iteration and then by play: the objectives, and when
// observing, the equities and win rates
pub struct SimRows {
    pub objective: Vec<f64>,
    pub equity: Vec<f64>,
    pub win_rate: Vec<f64>,
}

impl SimRows {
    // the rows emptied, their memory kept
    #[inline(always)]
    fn clear(&mut self) {
        self.objective.clear();
        self.equity.clear();
        self.win_rate.clear();
    }

    #[inline(always)]
    fn room_for<const OBSERVE: bool>(
        &mut self,
        memory: Option<&endgame::MemoryLimit>,
        size: usize,
    ) -> bool {
        let room = |list: &mut Vec<f64>| {
            endgame::room_in(memory, list, size.saturating_sub(list.len())).is_ok()
        };
        room(&mut self.objective)
            && (!OBSERVE || (room(&mut self.equity) && room(&mut self.win_rate)))
    }

    // append a copy of the rows of the iterations that follow these
    #[inline(always)]
    fn extend_from(&mut self, later: &SimRows) {
        self.objective.extend_from_slice(&later.objective);
        self.equity.extend_from_slice(&later.equity);
        self.win_rate.extend_from_slice(&later.win_rate);
    }

    // append the rows of the iterations that follow these
    #[inline]
    pub fn extend(&mut self, later: SimRows) {
        self.extend_from(&later);
    }
}

// what a sim's block rows are rolled out by, apart from the simmer: the
// tables, the decision's seed, and the late graph, if any, borrowed apart from
// the sim so that its threads can read them while it takes rows in
struct RowsOf<'a, 'g, N: kwg::Node, L: kwg::Node> {
    game_config: &'a game_config::GameConfig,
    kwg: &'a kwg::Kwg<N>,
    arm_tables: &'a anagrams::ArmTables,
    klv: &'a klv::Klv<L>,
    win_pct_table: Option<&'a win_pct::WinPctTable>,
    decision_seed: u64,
    late_graph: Option<&'g (kwg::Kwg<kwg::Node24>, anagrams::ArmTables)>,
    memory: Option<endgame::MemoryLimit>, // what the program stays within
}

impl<N: kwg::Node, L: kwg::Node> RowsOf<'_, '_, N, L> {
    // the rows of the iterations after lo up to hi for plays, in place of
    // those held, on a simmer prepared as pristine is, which each iteration is
    // restored to; the rows and the rollouts within the memory limit, the
    // block refused out of memory where it has no room for them
    #[inline]
    fn push_rows<const OBSERVE: bool, R: rand::Rng + SeedableRng + simmer::CopyState>(
        &self,
        simmer: &mut simmer::Simmer<R>,
        pristine: &game_state::GameState,
        lo: u64,
        hi: u64,
        plays: &[movegen::Play],
        rows: &mut SimRows,
    ) -> error::Returns<()> {
        rows.clear();
        let size = (hi - lo) as usize * plays.len();
        if !rows.room_for::<OBSERVE>(self.memory.as_ref(), size) {
            return Err(out_of_memory());
        }
        simmer.begin_within(self.memory);
        for sim_iter in lo + 1..=hi {
            simmer.restore_prepared(pristine);
            simmer.reseed(mix(self.decision_seed, sim_iter));
            simmer.prepare_iteration(Some(stratum_at(self.decision_seed, sim_iter)))?;
            for play in plays {
                let (value, sim_spread, win_prob) = match self.late_graph {
                    Some((kwg, arm_tables)) => rollout_objective(
                        simmer,
                        self.game_config,
                        kwg,
                        arm_tables,
                        self.klv,
                        play,
                        self.win_pct_table,
                    )?,
                    None => rollout_objective(
                        simmer,
                        self.game_config,
                        self.kwg,
                        self.arm_tables,
                        self.klv,
                        play,
                        self.win_pct_table,
                    )?,
                };
                rows.objective.push(value);
                if OBSERVE {
                    rows.equity.push(simmer::spread_points(sim_spread));
                    rows.win_rate.push(win_prob);
                }
            }
            if simmer.out_of_memory() {
                return Err(out_of_memory());
            }
        }
        Ok(())
    }
}

#[inline(never)]
fn out_of_memory() -> error::BoxAnyError {
    error::new("out_of_memory".into()).into()
}

// what a sim's caller does at a look (Simmer::resume_looking): given the
// iterations done so far of those asked for and the candidates still in
pub type Look<'l> = dyn FnMut(u64, usize) -> error::Returns<()> + 'l;

pub struct SimmerParams<'a> {
    pub num_sim_iters: u64,
    pub num_sim_plies: usize,
    pub sim_threads: usize,
    pub win_pct_table: Option<&'a std::sync::Arc<win_pct::WinPctTable>>,
    pub quiet: bool,                          // no progress lines
    pub memory: Option<endgame::MemoryLimit>, // what the program stays within
}

impl<'a, N: kwg::Node, L: kwg::Node, const OBSERVE: bool> Simmer<'a, N, L, OBSERVE> {
    pub fn new(
        game_config: &'a std::sync::Arc<game_config::GameConfig>,
        kwg: &'a std::sync::Arc<kwg::Kwg<N>>,
        arm_tables: &'a std::sync::Arc<anagrams::ArmTables>,
        klv: &'a std::sync::Arc<klv::Klv<L>>,
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
        game_config: &'a std::sync::Arc<game_config::GameConfig>,
        kwg: &'a std::sync::Arc<kwg::Kwg<N>>,
        arm_tables: &'a std::sync::Arc<anagrams::ArmTables>,
        klv: &'a std::sync::Arc<klv::Klv<L>>,
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
            spare_objectives: Vec::new(),
            iters_done: 0,
            next_stream_id: 0,
            win_pct_table: params.win_pct_table,
            #[cfg(not(target_family = "wasm"))]
            sim_threads: params.sim_threads,
            decision_seed,
            candidate_scratch: Vec::new(),
            dynamic_root: None,
            turn_time: None,
            turn_clock: None,
            timed_decisions: 0,
            decisions_without_rollouts: 0,
            prepared_pristine: game_state::GameState::new(game_config),
            quiet: params.quiet,
            decision_t0: None,
            ticked: 0,
            infer: None,
            inferred: std::sync::Arc::default(),
            infer_fuzz: (infer::FUZZ, 0.0),
            rollout_klv: std::sync::Arc::clone(klv),
            late_graph: None,
            late_board: Box::default(),
            memory: params.memory,
            late_prune: LatePrune {
                generator: None,
                words: Vec::new(),
                builder: Default::default(),
                spare: kwg::Kwg(Vec::new()),
                spare_tables: anagrams::ArmTables::None,
                arm_scratch: Default::default(),
            },
            thread_kept: Vec::new(),
            spare_plays: Vec::new(),
        }
    }

    // the root's field ranked by exact dynamic leave values; the rollouts keep
    // the static leaves
    pub fn with_dynamic_root(mut self, dynamic_leaves: klv::DynamicLeavesRef<'a>) -> Self {
        self.dynamic_root = Some(dynamic_leaves);
        self
    }

    // the fuzz in points, and the prior mix
    #[inline]
    pub fn set_infer_fuzz(&mut self, fuzz: (f64, f64)) {
        self.infer_fuzz = fuzz;
    }

    #[inline]
    pub fn set_rollout_klv(&mut self, klv: std::sync::Arc<klv::Klv<L>>) {
        self.rollout_klv = klv;
        #[cfg(not(target_family = "wasm"))]
        {
            self.sim_threads = 1;
        }
    }

    // a decision's time budget, counted from before its candidates are
    // generated; a rollout iteration or parallel block under way finishes
    // first
    pub fn with_turn_time(mut self, time: std::time::Duration) -> Self {
        self.turn_time = Some(time);
        self
    }

    #[inline(always)]
    fn turn_time_up(&self) -> bool {
        self.turn_clock
            .is_some_and(|(began, time)| began.elapsed() >= time)
    }

    // the decisions it took under a time budget, and how many of them it took
    // with no rollout done: its best root move, played for want of time
    pub fn timed_decisions(&self) -> (u64, u64) {
        (self.timed_decisions, self.decisions_without_rollouts)
    }

    #[inline(always)]
    pub fn reseed(&mut self, seed: u64) {
        self.simmer.reseed(seed);

        self.decision_seed = seed;
    }

    // the last decision's candidates, those in, and those retired, gone; one a
    // play in their place, each with the memory of a gone one's objectives
    #[inline(always)]
    fn take_candidates(&mut self, num_plays: usize) -> Vec<Candidate> {
        let mut candidates = std::mem::take(&mut self.candidates);
        for candidate in candidates.drain(..).chain(self.retired.drain(..)) {
            let mut objectives = candidate.objectives;
            objectives.clear();
            self.spare_objectives.push(objectives);
        }
        candidates.reserve(num_plays);
        for idx in 0..num_plays {
            candidates.push(Candidate {
                play_index: idx,
                stats: stats::Stats::new(),
                stream_id: idx as u64,
                equity_stats: stats::Stats::new(),
                win_rate_stats: stats::Stats::new(),
                first_iter: 1,
                objectives: self.spare_objectives.pop().unwrap_or_default(),
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
            objectives: self.spare_objectives.pop().unwrap_or_default(),
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

    // how many iterations to run up to the next look, so that running in such
    // steps gives the threads whole blocks
    #[inline(always)]
    pub fn iters_to_next_look(&self) -> u64 {
        PRUNE_CADENCE - self.iters_done % PRUNE_CADENCE
    }

    // for a decision bounded by time, plan the budget again from the rollouts
    // left: the candidates still in are rolled out to the next look, and after
    // it the survivor cap (one more than twice the looks left) shrinks the
    // field, so the budget is as many looks as the rollouts left can pay for
    // with each look's field at its cap. Planned again before each step, a
    // decision with slow rollouts prunes harder at its next look, and a fast
    // one runs on for its time.
    #[inline]
    pub fn plan_rollouts(&mut self, rollouts_left: f64) -> error::Returns<()> {
        let in_play = self.candidates.len() as f64;
        if in_play < 2.0 {
            return Ok(());
        }
        let next_look = (self.iters_done / PRUNE_CADENCE + 1) * PRUNE_CADENCE;
        let mut left = rollouts_left.max(0.0) - (next_look - self.iters_done) as f64 * in_play;
        let mut looks = 0u64;
        loop {
            // the field after one more look: its cap, or all the candidates
            // still in
            let field = in_play.min((3 + 2 * looks) as f64);
            let cost = PRUNE_CADENCE as f64 * field;
            if field == in_play {
                let complete_looks = left.max(0.0) / cost;
                if !(0.0..2.0_f64.powi(u64::BITS as i32)).contains(&complete_looks) {
                    crate::return_error!(format!(
                        "rollout budget is out of range: {rollouts_left}"
                    ));
                }
                looks = looks.saturating_add(complete_looks as u64);
                break;
            }
            if cost > left {
                break;
            }
            left -= cost;
            looks += 1;
        }
        self.num_sim_iters = next_look.saturating_add(PRUNE_CADENCE.saturating_mul(looks));
        Ok(())
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
        movegen::Play::Place { score, .. } => i64::from(score.raw()),
    }
}

// twice a rack's tile scores, as equity: what the other side gains when the
// side holding the rack is gone out on
#[inline(always)]
fn twice(game_config: &game_config::GameConfig, rack: &[u8]) -> i64 {
    2 * i64::from(equity::SCALE) * i64::from(game_config.alphabet().rack_score(rack).raw())
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
// leaving the board the fewest bingo lanes, the earlier on a tie; each board
// after one made in after
#[inline]
fn lane_pick(
    game_config: &game_config::GameConfig,
    board_tiles: &[u8],
    plays: &[movegen::ValuedMove],
    window: i32,
    after: &mut Vec<u8>,
) -> usize {
    let best = plays[0].equity.raw();
    let mut pick = (0, u32::MAX);
    for (i, m) in plays.iter().enumerate().take(DEFT_FIELD) {
        if best - m.equity.raw() > window {
            break;
        }
        glance::board_after(game_config, board_tiles, &m.play, after);
        let lanes = glance::glance_at(game_config, after).bingo_lanes;
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
    let ending = i64::from(game_state.players[mover].score.raw()) + twice(game_config, &other.rack);
    plays.iter().position(|m| {
        matches!(m.play, movegen::Play::Place { .. })
            && tiles_placed(&m.play) == rack_len
            && ending + play_score(&m.play) > i64::from(other.score.raw())
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

struct FootprintScratch<'a> {
    acc: &'a mut PlaySet,
    t: &'a mut PlaySet,
}

// a set of the other side's moves by their index, as bits
#[derive(Default)]
struct PlaySet(Vec<u64>);

impl PlaySet {
    // emptied, for n moves, its memory kept
    #[inline(always)]
    fn reset(&mut self, n: usize) {
        self.0.clear();
        self.0.resize(n.div_ceil(64), 0);
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
    generator.gen_raw_placements_in_lanes(
        &movegen::BoardSnapshot {
            board_tiles,
            game_config: board_snapshot.game_config,
            kwg: board_snapshot.kwg,
            anagrams: board_snapshot.anagrams,
            rack_lengths: board_snapshot.rack_lengths,
            klv: board_snapshot.klv,
        },
        rack,
        rows,
        cols,
    );
}

// the generator out in two runs, the buffers it fills, the first pass's lists
// (lent to each OutInTwo and given back), and the sets and lists its
// valuations work in, all kept from one decision to the next
pub struct Scratch {
    generator: movegen::KurniaMoveGenerator,
    after: Vec<u8>,
    kept: Vec<u8>,
    used: Vec<u8>,
    lists: Lists,
    temps: Temps,
    reach: Vec<u8>, // the positional term's reach map
}

impl Scratch {
    #[inline]
    fn new(game_config: &game_config::GameConfig) -> Self {
        Self {
            generator: movegen::KurniaMoveGenerator::new(game_config),
            after: Vec::new(),
            kept: Vec::new(),
            used: Vec::new(),
            lists: Lists::default(),
            temps: Temps::default(),
            reach: Vec::new(),
        }
    }
}

// an OutInTwo's lists while none holds them, and the sorted rack its choices
// are taken from
#[derive(Default)]
struct Lists {
    replies: Vec<Reply>,
    by_score: Vec<usize>,
    fills_at: Vec<u64>,
    zone_at: Vec<u64>,
    going_out: PlaySet,
    not_out: PlaySet,
    choices: Vec<Choice>,
    tiles: Vec<u8>,
    outs: Vec<usize>,
    sorted_rack: Vec<u8>,
}

// the sets and lists a valuation works in: the replies a move changes, two
// sets of replies it walks, the sets of replies that block an out (num_sets of
// them, one after another as a set's words), each choice's own in sets once
// made, a choice's outs standing, the outs with their blocking sets, the
// values and hopes, the choices hoping, an order, the real outs of an opener,
// and the outs after a choice by score and zone
#[derive(Default)]
struct Temps {
    changed: PlaySet,
    acc: PlaySet,
    t: PlaySet,
    sets: Vec<u64>,
    num_sets: usize,
    blocker_at: Vec<Option<usize>>,
    standing: Vec<usize>,
    outs: Vec<(i64, usize)>,
    values: Vec<i64>,
    hopes: Vec<i64>,
    hoping: Vec<usize>,
    order: Vec<usize>,
    real: Vec<(i64, usize)>,
    after_outs: Vec<(i64, Squares)>,
}

// a new set after the others in sets, empty, of so many words, and its place
// there
#[inline(always)]
fn next_set(sets: &mut Vec<u64>, num_sets: &mut usize, words: usize) -> usize {
    sets.resize(sets.len() + words, 0);
    *num_sets += 1;
    *num_sets - 1
}

// one of the other side's moves on the board as it is: its place among them as
// generated, its weight (its score and twice the tiles it plays), its score,
// whether it goes out, and its footprint
struct Reply {
    at: usize,
    weight: i64,
    score: i64,
    out: bool,
    footprint: Footprint,
}

// one of the mover's moves on the board as it is: the play, its score, the
// tiles it takes from the rack and the rack it keeps (each sorted, where each
// begins and ends in the OutInTwo's tiles), and its footprint
struct Choice {
    play: movegen::Play,
    score: i64,
    used: (usize, usize),
    left: (usize, usize),
    footprint: Footprint,
}

// out in two's first pass, with the bag empty and two players: every move of
// each side on the board as it is, from one generation of each rack
struct OutInTwo<'a, N: kwg::Node, L: kwg::Node> {
    board_snapshot: &'a movegen::BoardSnapshot<'a, N, L>,
    game_state: &'a game_state::GameState,
    lead: i64,
    other_twice: i64,
    replies: Vec<Reply>,  // by weight
    by_score: Vec<usize>, // the replies by score
    words: usize,         // a square's replies' words, square by square
    fills_at: Vec<u64>,
    zone_at: Vec<u64>,
    going_out: PlaySet,
    not_out: PlaySet,
    choices: Vec<Choice>,
    tiles: Vec<u8>,   // the choices' tiles taken and kept
    outs: Vec<usize>, // the choices by the tiles taken, then best first
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
        let Scratch {
            generator,
            used,
            kept,
            lists,
            ..
        } = scratch;

        gen_plays(generator, board_snapshot, board_tiles, other_rack, !0, !0);
        let mut replies = std::mem::take(&mut lists.replies);
        replies.clear();
        for m in generator.plays.iter() {
            if !matches!(m.play, movegen::Play::Place { .. }) {
                continue;
            }
            used_tiles(&m.play, used);
            let score = play_score(&m.play);
            replies.push(Reply {
                at: replies.len(),
                weight: score + twice(game_config, used),
                score,
                out: used.len() == other_rack.len(),
                footprint: footprint_on_board(dim, board_tiles, &m.play),
            });
        }
        // by weight, highest first, ties as generated
        replies.sort_unstable_by_key(|reply| (std::cmp::Reverse(reply.weight), reply.at));
        let n = replies.len();
        let mut by_score = std::mem::take(&mut lists.by_score);
        by_score.clear();
        by_score.extend(0..n);
        by_score.sort_unstable_by(|&a, &b| replies[b].score.cmp(&replies[a].score).then(a.cmp(&b)));
        let words = n.div_ceil(64);
        let mut fills_at = std::mem::take(&mut lists.fills_at);
        let mut zone_at = std::mem::take(&mut lists.zone_at);
        for sets in [&mut fills_at, &mut zone_at] {
            sets.clear();
            sets.resize(area * words, 0);
        }
        let mut going_out = std::mem::take(&mut lists.going_out);
        let mut not_out = std::mem::take(&mut lists.not_out);
        going_out.reset(n);
        not_out.reset(n);
        for (i, reply) in replies.iter().enumerate() {
            let (word, bit) = (i >> 6, 1 << (i & 63));
            reply
                .footprint
                .fills
                .each(|square| fills_at[square * words + word] |= bit);
            reply
                .footprint
                .zone
                .each(|square| zone_at[square * words + word] |= bit);
            if reply.out {
                going_out.set(i);
            } else {
                not_out.set(i);
            }
        }

        let sorted_rack = &mut lists.sorted_rack;
        sorted_rack.clone_from(rack);
        sorted_rack.sort_unstable();
        gen_plays(generator, board_snapshot, board_tiles, rack, !0, !0);
        let mut choices = std::mem::take(&mut lists.choices);
        let mut tiles = std::mem::take(&mut lists.tiles);
        choices.clear();
        tiles.clear();
        for m in generator.plays.iter() {
            if !matches!(m.play, movegen::Play::Place { .. }) {
                continue;
            }
            used_tiles(&m.play, used);
            rack_less(sorted_rack, used, kept);
            let start = tiles.len();
            tiles.extend_from_slice(used);
            let mid = tiles.len();
            tiles.extend_from_slice(kept);
            choices.push(Choice {
                play: m.play.clone(),
                score: play_score(&m.play),
                used: (start, mid),
                left: (mid, tiles.len()),
                footprint: footprint_on_board(dim, board_tiles, &m.play),
            });
        }
        // each choice by the tiles it takes, and those taking the same best
        // first (on a tie, as generated): a rack's outs are a run of them
        let mut outs = std::mem::take(&mut lists.outs);
        outs.clear();
        outs.extend(0..choices.len());
        let used_of = |c: &Choice| &tiles[c.used.0..c.used.1];
        outs.sort_unstable_by(|&a, &b| {
            (used_of(&choices[a]).cmp(used_of(&choices[b])))
                .then(choices[b].score.cmp(&choices[a].score))
                .then(a.cmp(&b))
        });
        Some(Self {
            board_snapshot,
            game_state,
            lead: i64::from(game_state.players[mover].score.raw())
                - i64::from(game_state.players[1 - mover].score.raw()),
            other_twice: twice(game_config, other_rack),
            replies,
            by_score,
            words,
            fills_at,
            zone_at,
            going_out,
            not_out,
            choices,
            tiles,
            outs,
        })
    }

    // the tiles a choice takes from the rack, sorted
    #[inline(always)]
    fn used(&self, choice: &Choice) -> &[u8] {
        &self.tiles[choice.used.0..choice.used.1]
    }

    // the rack a choice keeps, sorted
    #[inline(always)]
    fn left(&self, choice: &Choice) -> &[u8] {
        &self.tiles[choice.left.0..choice.left.1]
    }

    // the lists given back to the scratch they were taken from
    #[inline]
    fn release(self, scratch: &mut Scratch) {
        let lists = &mut scratch.lists;
        lists.replies = self.replies;
        lists.by_score = self.by_score;
        lists.fills_at = self.fills_at;
        lists.zone_at = self.zone_at;
        lists.going_out = self.going_out;
        lists.not_out = self.not_out;
        lists.choices = self.choices;
        lists.tiles = self.tiles;
        lists.outs = self.outs;
    }

    // the choices that take these tiles (sorted) from the rack, best first:
    // the outs of a rack that keeps them
    #[inline]
    fn outs_of(&self, tiles: &[u8]) -> &[usize] {
        let outs = &self.outs;
        let lo = outs.partition_point(|&u| self.used(&self.choices[u]) < tiles);
        let len = outs[lo..].partition_point(|&u| self.used(&self.choices[u]) == tiles);
        &outs[lo..lo + len]
    }

    // into blocks, the other side's moves that fill a square of zone: those
    // that block a play with that zone
    #[inline]
    fn blocking(&self, zone: &Squares, blocks: &mut [u64]) {
        blocks.fill(0);
        zone.each(|square| {
            let w = self.words;
            for (x, y) in blocks
                .iter_mut()
                .zip(&self.fills_at[square * w..(square + 1) * w])
            {
                *x |= y;
            }
        });
    }

    // into changed, the other side's moves whose zones a move filling fills
    // meets: those the move changes
    #[inline]
    fn changed_by(&self, fills: &Squares, changed: &mut PlaySet) {
        changed.0.fill(0);
        fills.each(|square| {
            let w = self.words;
            for (x, y) in changed
                .0
                .iter_mut()
                .zip(&self.zone_at[square * w..(square + 1) * w])
            {
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
        (self.outs_of(self.left(choice)).iter())
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
    // score and the replies that block it (their set's place in sets). Also
    // whether those outs hold the value down: none stands, or one reply blocks
    // them all
    #[inline]
    fn value_on_footprints(
        &self,
        changed: &PlaySet,
        score: i64,
        left_twice: i64,
        outs: &[(i64, usize)],
        sets: &[u64],
        FootprintScratch { acc, t }: FootprintScratch<'_>,
    ) -> (i64, bool) {
        let c = score + self.other_twice;
        let stuck = -i64::from(equity::ENDGAME_PENALTY_BASE) - left_twice;
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
            let w = self.words;
            let blocks = &sets[blocks * w..(blocks + 1) * w];
            for ((x, a), &b) in t.0.iter_mut().zip(acc.0.iter_mut()).zip(blocks) {
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
        let Scratch {
            generator,
            after,
            kept,
            used,
            temps,
            ..
        } = scratch;
        if !after_play(
            game_config,
            &self.game_state.board_tiles,
            rack,
            &choice.play,
            after,
            kept,
        ) {
            return None;
        }
        let s_m = choice.score;
        let c = s_m + self.other_twice;
        if kept.is_empty() {
            return Some(c);
        }
        let kept_twice = twice(game_config, kept);
        let stuck = -i64::from(equity::ENDGAME_PENALTY_BASE) - kept_twice;
        let (rows, cols) = zone_lanes_on_board(dim, &choice.footprint.zone);
        // the mover's outs after it, best first, by score and zone
        let outs = &mut temps.after_outs;
        outs.clear();
        outs.extend(
            self.standing_outs(choice, rows, cols)
                .map(|(s, fp)| (s, fp.zone)),
        );
        gen_plays(generator, self.board_snapshot, after, kept, rows, cols);
        outs.extend(
            (generator.plays.iter())
                .filter(|m| tiles_placed(&m.play) == kept.len())
                .map(|m| {
                    (
                        play_score(&m.play),
                        footprint_on_board(dim, after, &m.play).zone,
                    )
                }),
        );
        // ties in any order: of an out, only the score is read
        outs.sort_unstable_by_key(|&(s, _)| std::cmp::Reverse(s));
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
            generator,
            self.board_snapshot,
            after,
            other_rack,
            rows,
            cols,
        );
        for m in generator.plays.iter() {
            if !matches!(m.play, movegen::Play::Place { .. }) {
                continue;
            }
            used_tiles(&m.play, used);
            let s_x = play_score(&m.play);
            reply(
                s_x,
                s_x + twice(game_config, used),
                used.len() == other_rack.len(),
                &footprint_on_board(dim, after, &m.play).fills,
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
        self.values_and_hopes_in(&mut scratch.temps);
        let mut values = std::mem::take(&mut scratch.temps.values);
        let hopes = std::mem::take(&mut scratch.temps.hopes);
        self.raise_openers(scratch, &mut values, &hopes);
        // the best few by value (a move that wins, if any does), valued again
        // on the board after each; the best of those, if it beats the play at
        // fewest
        let mut order = std::mem::take(&mut scratch.temps.order);
        order.clear();
        order.extend(0..self.choices.len());
        order.sort_unstable_by_key(|&i| (std::cmp::Reverse(values[i]), i));
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
        scratch.temps.values = values;
        scratch.temps.hopes = hopes;
        scratch.temps.order = order;
        best.filter(|&(v, _)| v > fewest_value)
            .map(|(_, i)| self.choices[i].play.clone())
    }

    // every choice's value on footprints alone, with the outs standing now;
    // and, for a choice whose value the lack of a standing unblockable out
    // holds down, its hope, its value with one more such out, worth its
    // leftover's best out anywhere on the board now (the others' hope is
    // i64::MIN): the choices that may open a second place. Into temps's values
    // and hopes, worked out in temps
    #[inline]
    fn values_and_hopes_in(&self, temps: &mut Temps) {
        let game_config = self.board_snapshot.game_config;
        let n = self.replies.len();
        let Temps {
            changed,
            acc,
            t,
            sets,
            num_sets,
            blocker_at,
            standing,
            outs,
            values,
            hopes,
            ..
        } = temps;
        changed.reset(n);
        acc.reset(n);
        t.reset(n);
        let w = self.words;
        sets.clear();
        *num_sets = 0;
        let unblockable = next_set(sets, num_sets, w);
        blocker_at.clear();
        blocker_at.resize(self.choices.len(), None);
        values.clear();
        hopes.clear();
        for choice in &self.choices {
            let left = self.left(choice);
            if left.is_empty() {
                values.push(choice.score + self.other_twice);
                hopes.push(i64::MIN);
                continue;
            }
            self.changed_by(&choice.footprint.fills, changed);
            let list = self.outs_of(left);
            standing.clear();
            standing.extend(list.iter().copied().filter(|&u| {
                !choice
                    .footprint
                    .fills
                    .meets(&self.choices[u].footprint.zone)
            }));
            outs.clear();
            for &u in standing.iter() {
                let blocks = match blocker_at[u] {
                    Some(at) => at,
                    None => {
                        let at = next_set(sets, num_sets, w);
                        self.blocking(
                            &self.choices[u].footprint.zone,
                            &mut sets[at * w..(at + 1) * w],
                        );
                        blocker_at[u] = Some(at);
                        at
                    }
                };
                outs.push((self.choices[u].score, blocks));
            }
            let left_twice = twice(game_config, left);
            let (value, held) = self.value_on_footprints(
                changed,
                choice.score,
                left_twice,
                outs,
                sets,
                FootprintScratch { acc, t },
            );
            values.push(value);
            if held {
                let phantom = list.first().map_or(0, |&u| self.choices[u].score);
                let at = outs.partition_point(|&(s, _)| s >= phantom);
                outs.insert(at, (phantom, unblockable));
                let (hope, _) = self.value_on_footprints(
                    changed,
                    choice.score,
                    left_twice,
                    outs,
                    sets,
                    FootprintScratch { acc, t },
                );
                hopes.push(hope);
            } else {
                hopes.push(i64::MIN);
            }
        }
    }

    // the choices with the best hopes above their values, their values raised
    // to those with their leftover's real outs on the board after each, the
    // other side's replies still those standing
    #[inline]
    fn raise_openers(&self, scratch: &mut Scratch, values: &mut [i64], hopes: &[i64]) {
        let game_config = self.board_snapshot.game_config;
        let dim = game_config.board_layout().dim();
        let n = self.replies.len();
        let Scratch {
            generator,
            after,
            temps,
            ..
        } = scratch;
        let Temps {
            changed,
            acc,
            t,
            sets,
            num_sets,
            outs,
            hoping,
            order,
            real,
            ..
        } = temps;
        changed.reset(n);
        acc.reset(n);
        t.reset(n);
        hoping.clear();
        hoping.extend((0..self.choices.len()).filter(|&i| hopes[i] > values[i]));
        hoping.sort_unstable_by(|&a, &b| hopes[b].cmp(&hopes[a]).then(a.cmp(&b)));
        for &i in hoping.iter().take(OUT_IN_TWO_OPENERS) {
            let choice = &self.choices[i];
            let left = self.left(choice);
            glance::board_after(
                game_config,
                &self.game_state.board_tiles,
                &choice.play,
                after,
            );
            let (rows, cols) = zone_lanes_on_board(dim, &choice.footprint.zone);
            // each real out's blocking set, from the first of sets on
            let w = self.words;
            sets.clear();
            *num_sets = 0;
            real.clear();
            for (s, fp) in self.standing_outs(choice, rows, cols) {
                let at = next_set(sets, num_sets, w);
                self.blocking(&fp.zone, &mut sets[at * w..(at + 1) * w]);
                real.push((s, at));
            }
            gen_plays(generator, self.board_snapshot, after, left, rows, cols);
            for p in (generator.plays.iter()).filter(|p| tiles_placed(&p.play) == left.len()) {
                let at = next_set(sets, num_sets, w);
                self.blocking(
                    &footprint_on_board(dim, after, &p.play).zone,
                    &mut sets[at * w..(at + 1) * w],
                );
                real.push((play_score(&p.play), at));
            }
            // by score, highest first, ties as found
            order.clear();
            order.extend(0..real.len());
            order.sort_unstable_by_key(|&k| (std::cmp::Reverse(real[k].0), k));
            outs.clear();
            outs.extend(order.iter().map(|&k| real[k]));
            self.changed_by(&choice.footprint.fills, changed);
            let (value, _) = self.value_on_footprints(
                changed,
                choice.score,
                twice(game_config, left),
                outs,
                sets,
                FootprintScratch { acc, t },
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
    scratch: &mut Scratch,
) -> Option<movegen::Play> {
    let first = OutInTwo::new(board_snapshot, game_state, scratch)?;
    let found = first.pick(scratch, plays, fewest);
    first.release(scratch);
    found
}

#[inline]
fn field_of<T: std::str::FromStr>(field: &str, line: &str) -> error::Returns<T>
where
    T::Err: std::fmt::Display,
{
    field
        .parse()
        .map_err(|e| error::new(format!("{field:?} in {line:?}: {e}")).into())
}

// a premium square's word and letter multipliers and its reach, with a value
// in millipoints
pub type SquareValue = ((i8, i8, u8), i32);

#[derive(Clone, Debug, PartialEq)]
pub struct PositionalTerm {
    values: Box<[SquareValue]>, // by (word, letter, reach)
}

impl PositionalTerm {
    pub fn new(mut values: Vec<SquareValue>) -> error::Returns<Self> {
        values.sort_unstable_by_key(|&(key, _)| key);
        if let Some(pair) = values.windows(2).find(|pair| pair[0].0 == pair[1].0) {
            return_error!(format!(
                "the positional value for {:?} is given twice",
                pair[0].0
            ));
        }
        Ok(Self {
            values: values.into_boxed_slice(),
        })
    }

    // "word letter reach value" lines, a premium square's, value in points
    // (blank lines and # comments skipped)
    pub fn parse(text: &str) -> error::Returns<Self> {
        let mut values = Vec::new();
        for line in text.lines() {
            let line = line.split('#').next().unwrap_or("").trim();
            if line.is_empty() {
                continue;
            }
            let mut it = line.split_whitespace();
            let (Some(word), Some(letter), Some(reach), Some(value), None) =
                (it.next(), it.next(), it.next(), it.next(), it.next())
            else {
                return_error!(format!(
                    "a positional line is \"word letter reach value\", not {line:?}"
                ));
            };
            let key: (i8, i8, u8) = (
                field_of(word, line)?,
                field_of(letter, line)?,
                field_of(reach, line)?,
            );
            let millipoints = (field_of::<f64>(value, line)? * equity::SCALE as f64).round();
            if !(i32::MIN as f64..=i32::MAX as f64).contains(&millipoints) {
                return_error!(format!("the positional value in {line:?} is out of range"));
            }
            if matches!((key.0, key.1), (1, 1) | (0, 0)) || key.2 == 0 {
                continue;
            }
            values.push((key, millipoints as i32));
        }
        Self::new(values)
    }

    #[inline]
    pub fn value_of(
        &self,
        game_config: &game_config::GameConfig,
        board_tiles: &[u8],
        reach: &mut Vec<u8>,
    ) -> i64 {
        glance::reach_into(game_config, board_tiles, reach);
        glance::premium_reaches(game_config, reach)
            .map(|key| {
                self.values
                    .binary_search_by_key(&key, |&(k, _)| k)
                    .map_or(0, |i| self.values[i].1 as i64)
            })
            .sum()
    }

    #[inline]
    fn add_to(
        &self,
        game_config: &game_config::GameConfig,
        board_tiles: &[u8],
        plays: &mut [movegen::ValuedMove],
        scratch: &mut Scratch,
    ) -> error::Returns<()> {
        let before = self.value_of(game_config, board_tiles, &mut scratch.reach);
        for m in plays.iter_mut() {
            if places_no_tiles(&m.play) {
                continue;
            }
            glance::board_after(game_config, board_tiles, &m.play, &mut scratch.after);
            let after = self.value_of(game_config, &scratch.after, &mut scratch.reach);
            let Some(sum) = i32::try_from(m.equity.raw() as i64 + after - before)
                .ok()
                .map(equity::Equity::new)
                .filter(|sum| sum.is_finite())
            else {
                return_error!("a move's equity with its positional term is out of range".into());
            };
            m.equity = sum;
        }
        plays.sort_by_key(|m| std::cmp::Reverse(m.equity));
        Ok(())
    }
}

pub struct Deft {
    window: i32,
    positional: Option<PositionalTerm>,
    scratch: Option<((i8, i8, u8), Box<Scratch>)>,
}

impl Deft {
    #[inline(always)]
    pub fn new(window: i32, positional: Option<PositionalTerm>) -> Self {
        Self {
            window,
            positional,
            scratch: None,
        }
    }

    #[inline(always)]
    pub fn window(&self) -> i32 {
        self.window
    }

    #[inline]
    fn term_and_scratch(
        &mut self,
        game_config: &game_config::GameConfig,
    ) -> (Option<&PositionalTerm>, &mut Scratch) {
        let dim = game_config.board_layout().dim();
        let made_for = (dim.rows, dim.cols, game_config.alphabet().len());
        if self
            .scratch
            .as_ref()
            .is_some_and(|(was, _)| *was != made_for)
        {
            self.scratch = None;
        }
        let (_, scratch) = self
            .scratch
            .get_or_insert_with(|| (made_for, Box::new(Scratch::new(game_config))));
        (self.positional.as_ref(), scratch)
    }
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

pub enum PreemptyMode {
    Off,
    Static,
    WithSimmerRoot,
}

pub enum MovePicker<'a, N: kwg::Node, L: kwg::Node> {
    Hasty,
    Dynamic(klv::DynamicLeavesRef<'a>),
    Simmer(Box<Simmer<'a, N, L>>),
    Deft(Deft),
}

impl<
    'a,
    N: kwg::Node + Send + Sync + 'static,
    L: kwg::Node + Send + Sync + 'static,
    const OBSERVE: bool,
    R: rand::Rng + SeedableRng + simmer::CopyState + Send + Sync + 'static,
> Simmer<'a, N, L, OBSERVE, R>
{
    #[inline]
    fn run_iterations(
        &mut self,
        move_generator: &movegen::KurniaMoveGenerator,
        budget: u64,
        count: u64,
        look: &mut Look<'_>,
    ) -> error::Returns<()> {
        if self.candidates.len() < 2 || self.turn_time_up() {
            return Ok(());
        }

        #[cfg(not(target_family = "wasm"))]
        if self.sim_threads > 1 {
            return self.run_iterations_parallel(move_generator, count, look);
        }
        let mut candidates = std::mem::take(&mut self.candidates);
        let mut retired = std::mem::take(&mut self.retired);
        const Z: f64 = 1.96; // 95% confidence interval
        let start = self.iters_done;
        let memory = self.memory;
        self.simmer.begin_within(memory);
        for sim_iter in (start + 1)..=(start + count) {
            self.iters_done = sim_iter;
            self.simmer.restore_prepared(&self.prepared_pristine);
            self.simmer.reseed(mix(self.decision_seed, sim_iter));
            self.simmer
                .prepare_iteration(Some(stratum_at(self.decision_seed, sim_iter)))?;
            for candidate in candidates.iter_mut() {
                let play = &move_generator.plays[candidate.play_index].play;
                let (value, sim_spread, win_prob) = match &self.late_graph {
                    Some((kwg, arm_tables)) => rollout_objective(
                        &mut self.simmer,
                        self.game_config,
                        kwg,
                        arm_tables,
                        &self.rollout_klv,
                        play,
                        self.win_pct_table.map(|table| &**table),
                    )?,
                    None => rollout_objective(
                        &mut self.simmer,
                        self.game_config,
                        self.kwg,
                        self.arm_tables,
                        &self.rollout_klv,
                        play,
                        self.win_pct_table.map(|table| &**table),
                    )?,
                };
                if endgame::room_in(memory.as_ref(), &mut candidate.objectives, 1).is_err() {
                    return Err(out_of_memory());
                }
                candidate.stats.update(value);
                candidate.objectives.push(value);
                if OBSERVE {
                    candidate
                        .equity_stats
                        .update(simmer::spread_points(sim_spread));
                    candidate.win_rate_stats.update(win_prob);
                }
            }
            if self.simmer.out_of_memory() {
                return Err(out_of_memory());
            }
            tick(
                self.decision_t0,
                &mut self.ticked,
                sim_iter,
                budget,
                candidates.len(),
            );
            if sim_iter % PRUNE_CADENCE == 0 {
                let z = paired_z_among(budget, candidates.len());
                retire_paired(&mut candidates, &mut retired, z);

                let prune_periods_remaining = budget.saturating_sub(sim_iter) / PRUNE_CADENCE;
                limit_surviving_candidates(
                    &mut candidates,
                    &mut retired,
                    Z,
                    1 + 2 * prune_periods_remaining as usize,
                );
            }
            let out_of_time = self.turn_time_up();
            if sim_iter % PRUNE_CADENCE == 0 || sim_iter == start + count || out_of_time {
                look(sim_iter - start, candidates.len())?;
                if candidates.len() < 2 || out_of_time {
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
    // reached or fewer than two candidates are left. Its list of plays is made
    // in the memory of the spare one, which a run puts back
    #[inline]
    pub fn next_block(&mut self, end: u64) -> Option<SimBlock> {
        if self.candidates.len() < 2 || self.iters_done >= end {
            return None;
        }
        let lo = self.iters_done;
        let mut plays = std::mem::take(&mut self.spare_plays);
        plays.clear();
        plays.extend(self.candidates.iter().map(|candidate| candidate.play_index));
        Some(SimBlock {
            lo,
            hi: end.min((lo / PRUNE_CADENCE + 1) * PRUNE_CADENCE),
            plays,
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
        let mut simmer = self.simmer.prepared_clone(self.game_config)?;
        let pristine = simmer.prepared_state().clone();
        let mut rows = SimRows {
            objective: Vec::new(),
            equity: Vec::new(),
            win_rate: Vec::new(),
        };
        let plays = (plays.iter())
            .map(|&play_index| move_generator.plays[play_index].play.clone())
            .collect::<Vec<_>>();
        (self.rows_of(self.late_graph.as_ref())).push_rows::<OBSERVE, R>(
            &mut simmer,
            &pristine,
            lo,
            hi,
            &plays,
            &mut rows,
        )?;
        Ok(rows)
    }

    // what a block's rows are rolled out by, apart from the simmer, on the
    // late graph given (the sim's own, or one taken out of it)
    #[inline(always)]
    fn rows_of<'s, 'g>(
        &'s self,
        late_graph: Option<&'g (kwg::Kwg<kwg::Node24>, anagrams::ArmTables)>,
    ) -> RowsOf<'s, 'g, N, L> {
        RowsOf {
            game_config: self.game_config,
            kwg: self.kwg,
            arm_tables: self.arm_tables,
            klv: &self.rollout_klv,
            win_pct_table: self.win_pct_table.map(|table| &**table),
            decision_seed: self.decision_seed,
            late_graph,
            memory: self.memory,
        }
    }

    // take in a whole block's rows, each candidate's in iteration order
    // (combining variances is not associative in floating point), then the look
    // at the block's end if it ends on one
    #[inline]
    pub fn absorb_block(&mut self, block: &SimBlock, rows: &SimRows) -> error::Returns<()> {
        let num_candidates = block.plays.len();
        let iterations = block.hi.checked_sub(block.lo).ok_or("reversed sim block")?;
        let size = usize::try_from(iterations)?
            .checked_mul(num_candidates)
            .ok_or("too many sim rows")?;
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
        let more = (block.hi - block.lo) as usize;
        let memory = self.memory.as_ref();
        if !(self.candidates.iter_mut())
            .all(|candidate| endgame::room_in(memory, &mut candidate.objectives, more).is_ok())
        {
            return Err(out_of_memory());
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
            let z = paired_z_among(budget, self.candidates.len());
            retire_paired(&mut self.candidates, &mut self.retired, z);
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
        look: &mut Look<'_>,
    ) -> error::Returns<()> {
        let stopped = || "a sim thread stopped";
        let num_threads = self.sim_threads;
        let mut workers = std::mem::take(&mut self.thread_kept);
        workers.truncate(num_threads);
        while workers.len() < num_threads {
            let worker = self.start_worker(workers.first())?;
            workers.push(worker);
        }
        for (_, _, kept, ..) in &workers {
            let mut kept = kept.lock().map_err(|_| stopped())?;
            let (simmer, _, pristine) = &mut *kept;
            simmer.prepare_as(self.game_config, &self.simmer)?;
            pristine.clone_from(simmer.prepared_state());
        }
        let plays = std::sync::Arc::clone(&workers[0].3);
        let late = std::sync::Arc::clone(&workers[0].4);
        std::mem::swap(
            &mut *late.write().map_err(|_| stopped())?,
            &mut self.late_graph,
        );
        let start = self.iters_done;
        let end = start + count;
        let mut broken = false;
        let result = (|| -> error::Returns<()> {
            while !self.turn_time_up() {
                let Some(block) = self.next_block(end) else {
                    break;
                };
                {
                    let mut plays = plays.write().map_err(|_| stopped())?;
                    plays.clear();
                    plays.extend(
                        (block.plays.iter())
                            .map(|&play_index| move_generator.plays[play_index].play.clone()),
                    );
                }
                let block_len = (block.hi - block.lo) as usize;
                let base = block_len / num_threads;
                let extra = block_len % num_threads;
                let mut sent = 0;
                for (thread_index, (job_tx, ..)) in workers.iter().enumerate() {
                    let lo = thread_index * base + thread_index.min(extra);
                    let hi = lo + base + if thread_index < extra { 1 } else { 0 };
                    let job = (
                        block.lo + lo as u64,
                        block.lo + hi as u64,
                        self.decision_seed,
                    );
                    if job_tx.send(job).is_err() {
                        broken = true;
                        break;
                    }
                    sent += 1;
                }
                // every job sent is waited for, so that none is left running
                let mut made = Ok(());
                for (_, done_rx, ..) in &workers[..sent] {
                    match done_rx.recv() {
                        Ok(done) => made = made.and(done),
                        Err(_) => broken = true,
                    }
                }
                if broken {
                    return Err(stopped().into());
                }
                made?;
                // the first thread's rows, the others' appended in order (the
                // first's next rows replace them all), within the memory limit
                let mut first = workers[0].2.lock().map_err(|_| stopped())?;
                let size = (block.hi - block.lo) as usize * block.plays.len();
                if !first.1.room_for::<OBSERVE>(self.memory.as_ref(), size) {
                    return Err(out_of_memory());
                }
                for (_, _, later, ..) in &workers[1..] {
                    let later = later.lock().map_err(|_| stopped())?;
                    first.1.extend_from(&later.1);
                }
                self.absorb_block(&block, &first.1)?;
                drop(first);
                self.spare_plays = block.plays;
                look(block.hi - start, self.candidates.len())?;
            }
            Ok(())
        })();
        std::mem::swap(
            &mut *late.write().map_err(|_| stopped())?,
            &mut self.late_graph,
        );
        if !broken {
            self.thread_kept = workers;
        }
        result
    }

    // start a sim thread, with its share of the tables, a simmer prepared as
    // the sim's, and its rows; it rolls out each job it is sent until the sim
    // lets go of it
    #[cfg(not(target_family = "wasm"))]
    #[inline]
    fn start_worker(&self, shared: Option<&SimWorker<R>>) -> error::Returns<SimWorker<R>> {
        let (job_tx, job_rx) = std::sync::mpsc::sync_channel::<(u64, u64, u64)>(1);
        let (done_tx, done_rx) = std::sync::mpsc::sync_channel(1);
        let kept = std::sync::Arc::new(std::sync::Mutex::new((
            self.simmer.prepared_clone(self.game_config)?,
            SimRows {
                objective: Vec::new(),
                equity: Vec::new(),
                win_rate: Vec::new(),
            },
            self.prepared_pristine.clone(),
        )));
        let theirs = std::sync::Arc::clone(&kept);
        let game_config = std::sync::Arc::clone(self.game_config);
        let kwg = std::sync::Arc::clone(self.kwg);
        let arm_tables = std::sync::Arc::clone(self.arm_tables);
        let klv = std::sync::Arc::clone(&self.rollout_klv);
        let win_pct_table = self.win_pct_table.cloned();
        let memory = self.memory;
        let (plays, late) = match shared {
            Some((.., plays, late)) => (std::sync::Arc::clone(plays), std::sync::Arc::clone(late)),
            None => Default::default(),
        };
        let (ours, our_late) = (std::sync::Arc::clone(&plays), std::sync::Arc::clone(&late));
        std::thread::Builder::new().spawn(move || {
            for (lo, hi, decision_seed) in job_rx {
                let made = (|| -> error::Returns<()> {
                    let stopped = || "a sim thread's share was dropped";
                    let plays = plays.read().map_err(|_| stopped())?;
                    let late = late.read().map_err(|_| stopped())?;
                    let mut kept = theirs.lock().map_err(|_| stopped())?;
                    let (simmer, rows, pristine) = &mut *kept;
                    let rows_of = RowsOf {
                        game_config: &game_config,
                        kwg: &kwg,
                        arm_tables: &arm_tables,
                        klv: &klv,
                        win_pct_table: win_pct_table.as_deref(),
                        decision_seed,
                        late_graph: late.as_ref(),
                        memory,
                    };
                    rows_of.push_rows::<OBSERVE, R>(simmer, pristine, lo, hi, &plays, rows)
                })();
                if done_tx.send(made).is_err() {
                    break;
                }
            }
        })?;
        Ok((job_tx, done_rx, kept, ours, our_late))
    }

    // the leaves the opponent's last move says they kept, each with its weight,
    // as the shell infers them, for a sim to draw their rack from: the watcher
    // is the side to move, holding this rack
    #[inline]
    pub fn kept_leaves(
        &mut self,
        watcher_rack: &[u8],
        last: LastMove<'_>,
    ) -> error::Returns<Vec<(Vec<u8>, f64)>> {
        self.infer_kept(watcher_rack, last, None)?;
        let leaves = &self.inferred;
        Ok((0..leaves.len())
            .map(|i| (leaves.leave(i).to_vec(), leaves.weight(i)))
            .collect())
    }

    // the same leaves, into the driver's packed list, in place: the simmer
    // lets go of the last decision's first, so that the list is the driver's
    // alone and its memory is kept; and into carried, where given, what the
    // inference carries to the watcher's next
    #[inline]
    fn infer_kept(
        &mut self,
        watcher_rack: &[u8],
        last: LastMove<'_>,
        carried: Option<&mut infer::Carried>,
    ) -> error::Returns<()> {
        #[cfg(not(target_family = "wasm"))]
        let threads = self.sim_threads;
        #[cfg(target_family = "wasm")]
        let threads = 1;
        let (tables, infer_generator) = match &mut self.infer {
            Some(infer) => infer,
            infer @ None => {
                self.game_config.supports(game_config::Op::Inference)?;
                infer.insert((
                    infer::InferTables::new(self.game_config, self.klv)?,
                    movegen::KurniaMoveGenerator::new(self.game_config),
                ))
            }
        };
        tables.set_prior_mix(self.infer_fuzz.1);
        self.simmer.release_kept();
        if std::sync::Arc::get_mut(&mut self.inferred).is_none() {
            self.inferred = std::sync::Arc::default();
        }
        infer::kept_leaves_carried_into(
            tables,
            infer_generator,
            &movegen::BoardSnapshot {
                board_tiles: last.board_before,
                game_config: self.game_config,
                kwg: self.kwg,
                anagrams: None,
                rack_lengths: None,
                klv: self.klv,
            },
            watcher_rack,
            (last.play, last.held),
            infer::InferOptions {
                fuzz: self.infer_fuzz.0,
                threads,
                weigh_passes: false,
            },
            (std::sync::Arc::make_mut(&mut self.inferred), carried),
        )
    }

    #[inline]
    pub fn begin_decision(
        &mut self,
        move_generator: &movegen::KurniaMoveGenerator,
        game_state: &game_state::GameState,
        iters: u64,
        kept_leaves: &[(Vec<u8>, f64)], // the leaves the opponent may have kept, with weights
    ) -> error::Returns<()> {
        self.begin_decision_of(move_generator, game_state, iters, kept_leaves, &[])
    }

    // as begin_decision, a seat's leaves also given packed, each in place of
    // any listed for its seat
    #[inline]
    fn begin_decision_of(
        &mut self,
        move_generator: &movegen::KurniaMoveGenerator,
        game_state: &game_state::GameState,
        iters: u64,
        kept_leaves: &[(Vec<u8>, f64)],
        packed: &[simmer::KeptLeaves],
    ) -> error::Returns<()> {
        self.simmer.prepare(
            self.game_config,
            game_state,
            &simmer::SimChoices {
                packed,
                ..simmer::SimChoices::new(self.num_sim_plies, OBSERVE, kept_leaves)
            },
        )?;
        self.prepared_pristine
            .clone_from(self.simmer.prepared_state());
        // late in the game, when it pays, the rollouts walk a graph of the
        // words the board can still take, whatever comes to the racks; the
        // generator keeps nothing of a graph it walked before
        #[cfg(not(target_family = "wasm"))]
        let threads = self.sim_threads;
        #[cfg(target_family = "wasm")]
        let threads = 1;
        let blanks_on_board = (game_state.board_tiles.iter())
            .filter(|&&tile| tile & 0x80 != 0)
            .count();
        // a decision with no room left under the memory limit is refused
        if self.memory.is_some_and(|memory| !memory.has_room(1)) {
            return Err(out_of_memory());
        }
        // under a memory limit the late graph is pruned within it, and only
        // for a sim of no arm tables (whose memory the limit cannot foresee)
        let late = late_graph_pays(
            game_state.bag.len(),
            blanks_on_board < self.game_config.alphabet().freq(0) as usize,
            self.num_sim_iters,
            threads,
        ) && (self.memory.is_none()
            || matches!(&**self.arm_tables, anagrams::ArmTables::None));
        #[cfg(not(target_family = "wasm"))]
        let late = late && !self.simmer.preempty_enabled();
        // only the very board the graph was pruned for keeps it: tiles added
        // since may be a phoney left standing, beside which a valid move makes
        // a word that prune dropped
        let keep =
            late && self.late_graph.is_some() && self.late_board[..] == game_state.board_tiles[..];
        if !keep && (late || self.late_graph.is_some()) {
            self.simmer.reset_for_another_kwg();
            for (_, _, kept, ..) in &self.thread_kept {
                if let Ok(mut kept) = kept.lock() {
                    kept.0.reset_for_another_kwg();
                }
            }
        }
        let LatePrune {
            generator,
            words,
            builder,
            spare,
            spare_tables,
            arm_scratch,
        } = &mut self.late_prune;
        self.late_graph = if keep {
            self.late_graph.take()
        } else if late {
            'late: {
                self.late_board.clone_from(&game_state.board_tiles);
                words.clear();
                let memory = self.memory.as_ref();
                // a word too long to be held in its Bites takes its tiles' bytes
                let mut refused = false;
                let pruned = (generator
                    .get_or_insert_with(|| movegen::KurniaMoveGenerator::new(self.game_config)))
                .gen_remaining_words_within(
                    &movegen::BoardSnapshot {
                        board_tiles: &game_state.board_tiles,
                        game_config: self.game_config,
                        kwg: self.kwg,
                        anagrams: None,
                        rack_lengths: None,
                        klv: self.klv,
                    },
                    memory,
                    |word: &[u8]| {
                        if refused {
                            return;
                        }
                        let tiles = if word.len() >= std::mem::size_of::<bites::Bites>() {
                            word.len()
                        } else {
                            0
                        };
                        let copy = (endgame::room_in(memory, words, 1).is_ok())
                            .then(|| endgame::reserve_in(memory, tiles))
                            .flatten();
                        match copy {
                            Some(_copy) => words.push(word.into()),
                            None => refused = true,
                        }
                    },
                );
                let refused = match pruned {
                    Ok(()) => refused,
                    Err(e) if e.is::<endgame::Refused>() => true,
                    Err(e) => return Err(e),
                };
                let (mut kwg, mut arm_tables) = match self.late_graph.take() {
                    Some(late) => late,
                    None => (
                        std::mem::replace(spare, kwg::Kwg(Vec::new())),
                        std::mem::replace(spare_tables, anagrams::ArmTables::None),
                    ),
                };
                // where the limit has no room for the prune or the graph, the sim
                // goes on with the lexicon's own graph: the late one is for speed
                // alone
                let built = if refused {
                    None
                } else {
                    match builder.build_for_game_within(self.game_config, words, memory) {
                        Ok(bytes) => Some(bytes),
                        Err(e) if e.is::<endgame::Refused>() => None,
                        Err(e) => return Err(e),
                    }
                };
                let Some(bytes) = built.filter(|bytes| {
                    endgame::room_in(memory, &mut kwg.0, (bytes.len() / 4).max(2)).is_ok()
                }) else {
                    *spare = kwg;
                    *spare_tables = arm_tables;
                    break 'late None;
                };
                kwg.refill_from_bytes(bytes)?;
                match &**self.arm_tables {
                    anagrams::ArmTables::Anagrams(_) => {
                        arm_tables.rebuild(&kwg, self.game_config, false, arm_scratch)?
                    }
                    anagrams::ArmTables::RackLengths(_) => {
                        arm_tables.rebuild(&kwg, self.game_config, true, arm_scratch)?
                    }
                    anagrams::ArmTables::None => arm_tables = anagrams::ArmTables::None,
                }
                Some((kwg, arm_tables))
            }
        } else {
            if let Some((kwg, arm_tables)) = self.late_graph.take() {
                *spare = kwg;
                *spare_tables = arm_tables;
            }
            None
        };
        self.candidates = self.take_candidates(move_generator.plays.len());
        self.next_stream_id = self.candidates.len() as u64;
        self.retired.clear();
        self.iters_done = 0;
        self.decision_t0 = (!self.quiet).then(std::time::Instant::now);
        self.ticked = 0;
        let budget = self.num_sim_iters;
        self.run_iterations(move_generator, budget, iters, &mut |_, _| Ok(()))
    }

    #[inline]
    pub fn resume(
        &mut self,
        move_generator: &movegen::KurniaMoveGenerator,
        extra_iters: u64,
    ) -> error::Returns<()> {
        self.resume_looking(move_generator, extra_iters, &mut |_, _| Ok(()))
    }

    // as resume, calling look at each look and at the end with the iterations
    // done so far of these and the candidates still in, so that a caller
    // watching the sim need not resume it a look at a time (a run on threads
    // starts them once for all its looks)
    #[inline]
    pub fn resume_looking(
        &mut self,
        move_generator: &movegen::KurniaMoveGenerator,
        extra_iters: u64,
        look: &mut Look<'_>,
    ) -> error::Returns<()> {
        let budget = self.num_sim_iters;
        self.run_iterations(move_generator, budget, extra_iters, look)
    }
}

impl<N: kwg::Node + Send + Sync + 'static, L: kwg::Node + Send + Sync + 'static>
    MovePicker<'_, N, L>
{
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
        self.pick_a_move_after(
            filtered_movegen,
            move_generator,
            board_snapshot,
            game_state,
            rack,
            (None, &mut infer::Carried::default()),
        )
    }

    #[cfg(not(target_family = "wasm"))]
    pub fn pick_a_move_after_with_preempty(
        &mut self,
        filtered_movegen: &mut move_filter::GenMoves<'_>,
        move_generator: &mut movegen::KurniaMoveGenerator,
        board_snapshot: &movegen::BoardSnapshot<'_, N, L>,
        game_state: &game_state::GameState,
        rack: &[u8],
        (last, carried, mode): (Option<LastMove<'_>>, &mut infer::Carried, &PreemptyMode),
    ) -> error::Returns<()> {
        let enabled = !matches!(mode, PreemptyMode::Off);
        let bag = game_state.bag.len();
        let game_config = board_snapshot.game_config;
        if enabled
            && crate::preempty::applies(game_config, bag)
            && (!matches!(self, MovePicker::Simmer(_))
                || matches!(mode, PreemptyMode::WithSimmerRoot))
        {
            carried.clear();
            let dynamic = match self {
                MovePicker::Dynamic(leaves) => Some(*leaves),
                MovePicker::Simmer(simmer) => simmer.dynamic_root,
                _ => None,
            };
            filtered_movegen.gen_moves(
                move_generator,
                board_snapshot,
                rack,
                game_state.current_player().num_exchanges,
                usize::MAX,
                dynamic,
            )?;
            if let MovePicker::Deft(deft) = self {
                let (term, scratch) = deft.term_and_scratch(game_config);
                if let Some(term) = term {
                    term.add_to(
                        game_config,
                        &game_state.board_tiles,
                        &mut move_generator.plays,
                        scratch,
                    )?;
                }
            }
            return crate::preempty::pick_generated(
                &mut move_generator.plays,
                board_snapshot,
                rack,
                bag,
            );
        }
        if enabled && let MovePicker::Simmer(simmer) = self {
            simmer.simmer.set_preempty(Some(filtered_movegen));
        }
        let picked = self.pick_a_move_after(
            filtered_movegen,
            move_generator,
            board_snapshot,
            game_state,
            rack,
            (last, carried),
        );
        if enabled && let MovePicker::Simmer(simmer) = self {
            simmer.simmer.set_preempty(None);
        }
        picked
    }

    // as pick_a_move, given the opponent's last move: a sim then draws their
    // rack from what that move says they kept, as the shell's sims do (with two
    // players; the other pickers do not read it); carried is left holding what
    // that inference carries to the watcher's next (nothing, if it made none)
    #[inline(always)]
    pub fn pick_a_move_after(
        &mut self,
        filtered_movegen: &mut move_filter::GenMoves<'_>,
        move_generator: &mut movegen::KurniaMoveGenerator,
        board_snapshot: &movegen::BoardSnapshot<'_, N, L>,
        game_state: &game_state::GameState,
        rack: &[u8],
        (last, carried): (Option<LastMove<'_>>, &mut infer::Carried),
    ) -> error::Returns<()>
    where
        N: Sync,
        L: Sync,
    {
        carried.clear();
        match self {
            MovePicker::Hasty => {
                filtered_movegen.gen_moves(
                    move_generator,
                    board_snapshot,
                    rack,
                    game_state.current_player().num_exchanges,
                    1,
                    None,
                )?;
            }
            MovePicker::Dynamic(dynamic_leaves) => {
                filtered_movegen.gen_moves(
                    move_generator,
                    board_snapshot,
                    rack,
                    game_state.current_player().num_exchanges,
                    1,
                    Some(*dynamic_leaves),
                )?;
            }
            MovePicker::Simmer(simmer) => {
                let t0 =
                    (!simmer.quiet || simmer.turn_time.is_some()).then(std::time::Instant::now);
                let mut scratch = std::mem::take(&mut simmer.candidate_scratch);
                gen_simmer_field(
                    filtered_movegen,
                    move_generator,
                    &movegen::GenMovesParams {
                        board_snapshot,
                        rack,
                        num_exchanges_by_this_player: game_state.current_player().num_exchanges,
                        max_gen: SIMMER_FIELD.max_gen,
                        pass_policy: movegen::PassPolicy::OnlyWhenForced,
                        dynamic_leaves: simmer.dynamic_root,
                    },
                    SIMMER_FIELD.nonplacing_quota,
                    &mut scratch,
                )?;
                simmer.candidate_scratch = scratch;
                // the leaves inferred from the last move, packed and shared
                // with the simmer, not copied
                let inferred = match last {
                    Some(last) if game_state.players.len() == 2 => {
                        simmer.infer_kept(rack, last, Some(carried))?;
                        Some(simmer::KeptLeaves {
                            seat: 1 - game_state.turn,
                            leaves: simmer.inferred.clone(),
                        })
                    }
                    _ => None,
                };
                let budget = simmer.num_sim_iters;
                simmer.turn_clock = t0.zip(simmer.turn_time);
                let result = simmer.begin_decision_of(
                    move_generator,
                    game_state,
                    budget,
                    &[],
                    inferred.as_slice(),
                );
                simmer.turn_clock = None;
                result?;
                if !simmer.quiet
                    && let Some(t0) = t0
                {
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
                if simmer.turn_time.is_some() {
                    simmer.timed_decisions += 1;
                    if simmer.iters_done == 0 && simmer.candidates.len() > 1 {
                        simmer.decisions_without_rollouts += 1;
                    }
                }
                let winner_play_index = if simmer.turn_time.is_some() && simmer.iters_done == 0 {
                    0
                } else {
                    top_candidate_play_index_by_mean(&simmer.candidates)
                };
                move_generator.plays.swap(0, winner_play_index);
                move_generator.plays.truncate(1);
            }
            MovePicker::Deft(deft) => {
                let game_config = board_snapshot.game_config;
                let window = deft.window;
                let (positional, scratch) = deft.term_and_scratch(game_config);
                let late = game_state.bag.len() <= game_config.rack_size() as usize;
                filtered_movegen.gen_moves(
                    move_generator,
                    board_snapshot,
                    rack,
                    game_state.current_player().num_exchanges,
                    if late { DEFT_LATE_FIELD } else { DEFT_FIELD },
                    None,
                )?;
                if let Some(term) = positional {
                    term.add_to(
                        game_config,
                        &game_state.board_tiles,
                        &mut move_generator.plays,
                        scratch,
                    )?;
                }
                let plays = &move_generator.plays;
                let fewest = lane_pick(
                    game_config,
                    &game_state.board_tiles,
                    plays,
                    window,
                    &mut scratch.after,
                );
                let pick = winning_out(game_config, game_state, plays)
                    .or_else(|| one_in_the_bag(game_config, game_state, plays, fewest));
                let found = match pick {
                    Some(_) => None,
                    None => out_in_two(board_snapshot, game_state, plays, fewest, scratch),
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
    use crate::{
        matrix,
        zone::{footprint, zone_lanes},
    };

    impl<N: kwg::Node, L: kwg::Node> OutInTwo<'_, N, L> {
        fn values_and_hopes(&self) -> (Vec<i64>, Vec<i64>) {
            let mut temps = Temps::default();
            self.values_and_hopes_in(&mut temps);
            (temps.values, temps.hopes)
        }
    }

    #[test]
    #[inline]
    fn stitched_z_reads_as_its_formula_and_grows_slowly() {
        // values of 1.7 sqrt(ln ln 2n + 0.72 ln(5.2 / alpha)) worked out apart
        // from this code
        for (iterations, alpha, z) in [
            (16, 0.05, 3.640_892_996_146_262_4),
            (1000, 0.05, 3.940_271_585_734_935),
            (87057, 0.05, 4.106_291_666_114_815),
            (1000, 0.01, 4.344_497_505_758_632),
        ] {
            assert!(
                (stitched_z(iterations, alpha) - z).abs() < 1e-12,
                "{iterations} {alpha}"
            );
        }
        // more looks and a smaller alpha each ask for more
        assert!(stitched_z(2000, 0.05) > stitched_z(1000, 0.05));
        assert!(stitched_z(1000, 0.01) > stitched_z(1000, 0.05));
    }

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
        let kwg = kwg::kbwg_from_bytes_alloc(kwg_bytes.to_vec(), false).unwrap();
        let klv = klv::kbwg_from_bytes_alloc(klv_bytes.to_vec()).unwrap();
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
        )
        .unwrap();
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

    #[inline]
    fn best_retired_share(k: usize, gap: f64, budget: u64, decisions: u64) -> f64 {
        let z = paired_z(budget);
        best_retired_share_among(k, gap, budget, decisions, |_| z)
    }

    #[inline]
    fn best_retired_share_among(
        k: usize,
        gap: f64,
        budget: u64,
        decisions: u64,
        z_of: impl Fn(usize) -> f64,
    ) -> f64 {
        let mut rng = simmer::DefaultRng::seed_from_u64(k as u64);
        let mut normal = || {
            let u: f64 = 1.0 - rand::RngExt::random::<f64>(&mut rng);
            let v: f64 = rand::RngExt::random::<f64>(&mut rng);
            (-2.0 * u.ln()).sqrt() * (std::f64::consts::TAU * v).cos()
        };
        let mut best_retired = 0u64;
        for _ in 0..decisions {
            let mut candidates = (0..k).map(|i| candidate_from(i, &[])).collect::<Vec<_>>();
            let mut retired = Vec::new();
            for iter in 1..=budget {
                let shared = normal();
                for candidate in candidates.iter_mut() {
                    let mean = if candidate.play_index == 0 { gap } else { 0.0 };
                    let value = mean + shared + normal();
                    candidate.stats.update(value);
                    candidate.objectives.push(value);
                }
                if iter % PRUNE_CADENCE == 0 && candidates.len() > 1 {
                    let z = z_of(candidates.len());
                    retire_paired(&mut candidates, &mut retired, z);
                }
            }
            if retired.iter().any(|candidate| candidate.play_index == 0) {
                best_retired += 1;
            }
        }
        best_retired as f64 / decisions as f64
    }

    #[test]
    #[inline]
    fn retire_paired_retires_the_first_of_two_tied_at_most_one_time_in_twenty() {
        let decisions = 400;
        let share = best_retired_share(2, 0.0, 256, decisions);
        // one in twenty, and three standard errors of so many decisions
        let bound = 0.05 + 3.0 * (0.05 * 0.95 / decisions as f64).sqrt();
        assert!(share <= bound, "{share}");
    }

    #[test]
    #[inline]
    fn retire_paired_retires_the_first_of_k_tied_at_most_one_time_in_twenty() {
        let budget = 256;
        let decisions = 400;
        let bound = 0.05 + 3.0 * (0.05 * 0.95 / decisions as f64).sqrt();
        for k in [2, 4, 8, 16] {
            let share = best_retired_share_among(k, 0.0, budget, decisions, |candidates| {
                paired_z_among(budget, candidates)
            });
            assert!(share <= bound, "{k}: {share}");
        }
    }

    #[test]
    #[inline]
    fn student_t_matches_known_values() {
        for (p, df, t) in [
            (0.975, 15.0, 2.131_449_545_559),
            (0.9995, 15.0, 4.072_765_195_915),
            (0.975, 63.0, 1.998_340_542_520),
            (0.9999, 255.0, 3.773_802_433_763),
        ] {
            let found = student_t_of_normal_by_abramowitz_stegun_26_7_5(normal_quantile(p), df);
            assert!((found - t).abs() / t < 1e-3, "{p} {df}: {found}");
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
            .map(|i| 100.0 + 10.0 * f64::from(i % 3))
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
    fn behind_paired_skips_the_iterations_either_sat_out() {
        // from iteration 5 the candidate is a point behind wherever both were
        // rolled out; where only it was, it scored 100, which compared with
        // nothing must not count
        let lead = (1..=40u64)
            .map(|i| if i % 4 == 3 { f64::NAN } else { 10.0 })
            .collect::<Vec<_>>();
        let mine = (5..=40u64)
            .map(|i| match i % 4 {
                1 => f64::NAN,
                3 => 100.0,
                _ => 9.0,
            })
            .collect::<Vec<_>>();
        assert!(behind_paired(&mine, 5, &lead, 1, 3.0));
        // both were rolled out on iterations 6, 8, 10, ... 40
        let diff = paired_difference(&mine, 5, &lead, 1);
        assert_eq!(
            (diff.count(), diff.mean(), diff.variance()),
            (18.0, -1.0, 0.0)
        );
        // ahead where both were rolled out: not behind
        let ahead = mine.iter().map(|v| v + 2.0).collect::<Vec<_>>();
        assert!(!behind_paired(&ahead, 5, &lead, 1, 3.0));
        // one shared iteration is too few to judge
        assert!(!behind_paired(&mine[..2], 5, &lead, 1, 3.0));
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
        assert!((z_among(z, 2) - z).abs() < 1e-8, "{}", z_among(z, 2));
        assert!((z_among(z, 16) - paired_z_among(1000, 16)).abs() < 1e-8);
        assert!((z_among(1.959_963_984_540_054, 1) - 1.959_963_984_540_054).abs() < 1e-8);
        let z = paired_z_among(1000, 16);
        assert!((z + normal_quantile(0.05 / (62.0 * 15.0))).abs() < 1e-12);
    }

    #[test]
    #[inline]
    fn blocks_however_split_give_the_sequential_numbers() {
        let game_config = std::sync::Arc::new(game_config::make_english_game_config());
        let kwg = std::sync::Arc::new(
            kwg::kbwg_from_bytes_alloc(tiny_lexicon().to_vec(), false).unwrap(),
        );
        let klv =
            std::sync::Arc::new(klv::kbwg_from_bytes_alloc(klv::EMPTY_KLV_BYTES.to_vec()).unwrap());
        let arm_tables =
            std::sync::Arc::new(anagrams::ArmTables::build(&kwg, &game_config, false).unwrap());
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
        )
        .unwrap();
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
                    memory: None,
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

        // the same decision run in steps up to each look, the first step short
        let mut stepped = new_simmer(3);
        stepped
            .begin_decision(&move_generator, &game_state, 5, &[])
            .unwrap();
        assert_eq!(stepped.iters_to_next_look(), 11);
        let mut done = 5;
        while done < 64 {
            let step = stepped.iters_to_next_look().min(64 - done);
            stepped.resume(&move_generator, step).unwrap();
            done += step;
        }
        assert_eq!(
            sequential.leaderboard(usize::MAX),
            stepped.leaderboard(usize::MAX)
        );

        // a block taken in out of its turn is refused
        let (block, rows) = stale.unwrap();
        assert!(blocked.absorb_block(&block, &rows).is_err());
    }

    #[test]
    #[inline]
    fn a_late_graph_pays_by_the_budget_and_the_blanks_and_over_threads_by_the_share() {
        for (budget, most) in [(16, 11), (100, 15), (1000, 22), (1_000_000, 24)] {
            assert!(late_graph_pays(most, false, budget, 1), "{budget}");
            assert!(!late_graph_pays(most + 1, false, budget, 1), "{budget}");
            assert!(late_graph_pays(most - 7, true, budget, 1), "{budget}");
            assert!(!late_graph_pays(most - 6, true, budget, 1), "{budget}");
        }
        for (budget, threads, most) in [
            (1000, 2, 20),
            (1000, 4, 16),
            (1000, 8, 11),
            (3000, 8, 18),
            (1_000_000, 8, 27),
        ] {
            let at = |bag, blank| late_graph_pays(bag, blank, budget, threads);
            assert!(at(most, false), "{budget} {threads}");
            assert!(!at(most + 1, false), "{budget} {threads}");
            assert!(at(most - 6, true), "{budget} {threads}");
            assert!(!at(most - 5, true), "{budget} {threads}");
        }
        // a share of a few iterations pays at no bag with a blank off the board
        assert!(!late_graph_pays(0, true, 100, 8));
    }

    #[test]
    #[inline]
    fn a_decision_past_a_racks_worth_rolls_out_on_the_late_graph_as_on_the_whole() {
        let game_config = std::sync::Arc::new(game_config::make_english_game_config());
        let kwg = std::sync::Arc::new(
            kwg::kbwg_from_bytes_alloc(tiny_lexicon().to_vec(), false).unwrap(),
        );
        let klv =
            std::sync::Arc::new(klv::kbwg_from_bytes_alloc(klv::EMPTY_KLV_BYTES.to_vec()).unwrap());
        let arm_tables =
            std::sync::Arc::new(anagrams::ArmTables::build(&kwg, &game_config, false).unwrap());
        let mut game_state = game_state::GameState::new(&game_config);
        let mut deal_rng = simmer::DefaultRng::seed_from_u64(7);
        game_state.reset_and_draw_tiles(&game_config, &mut deal_rng);
        game_state.set_current_rack(&[1, 1, 5, 5, 20, 20, 0]);
        // twelve tiles in the bag: past a rack's worth, at a bag where a
        // decision of 1000 iterations builds its graph on one thread, not on
        // eight (a blank is off the board)
        let bag = game_state.bag.as_slice()[..12].to_vec();
        game_state.bag.set_from_iter(bag);
        let board_snapshot = movegen::BoardSnapshot {
            board_tiles: &game_state.board_tiles,
            game_config: &game_config,
            kwg: &kwg,
            anagrams: arm_tables.anagrams(),
            rack_lengths: arm_tables.rack_lengths(),
            klv: &klv,
        };
        let mut move_generator = movegen::KurniaMoveGenerator::new(&game_config);
        gen_simmer_candidates(
            &mut move_filter::GenMoves::Unfiltered,
            &mut move_generator,
            &board_snapshot,
            &game_state.current_player().rack,
            0,
            SIMMER_FIELD,
            &mut Vec::new(),
        )
        .unwrap();
        assert!(move_generator.plays.len() > 2);
        let decide = |sim_threads| {
            let mut simmer = Simmer::<_, _, true>::new(
                &game_config,
                &kwg,
                &arm_tables,
                &klv,
                SimmerParams {
                    num_sim_iters: 1000,
                    num_sim_plies: 2,
                    sim_threads,
                    win_pct_table: None,
                    quiet: true,
                    memory: None,
                },
            )
            .unwrap();
            simmer.reseed(11);
            simmer
                .begin_decision(&move_generator, &game_state, 32, &[])
                .unwrap();
            simmer
        };
        let alone = decide(1);
        let shared = decide(8);
        assert!(alone.late_graph.is_some());
        assert!(shared.late_graph.is_none());
        // the rows do not depend on the threads, so the late graph's are the
        // whole graph's
        assert_eq!(
            alone.leaderboard(usize::MAX),
            shared.leaderboard(usize::MAX)
        );
    }

    #[test]
    #[inline]
    fn a_decision_whose_late_prune_has_no_room_rolls_out_on_the_whole_alike() {
        let game_config = std::sync::Arc::new(game_config::make_english_game_config());
        let kwg = std::sync::Arc::new(
            kwg::kbwg_from_bytes_alloc(tiny_lexicon().to_vec(), false).unwrap(),
        );
        let klv =
            std::sync::Arc::new(klv::kbwg_from_bytes_alloc(klv::EMPTY_KLV_BYTES.to_vec()).unwrap());
        let arm_tables = std::sync::Arc::new(anagrams::ArmTables::None);
        let mut game_state = game_state::GameState::new(&game_config);
        let mut deal_rng = simmer::DefaultRng::seed_from_u64(7);
        game_state.reset_and_draw_tiles(&game_config, &mut deal_rng);
        game_state.set_current_rack(&[1, 1, 5, 5, 20, 20, 0]);
        // twelve tiles in the bag: past a rack's worth, at a bag where a
        // decision of 1000 iterations builds its graph on one thread, not on
        // eight (a blank is off the board)
        let bag = game_state.bag.as_slice()[..12].to_vec();
        game_state.bag.set_from_iter(bag);
        let board_snapshot = movegen::BoardSnapshot {
            board_tiles: &game_state.board_tiles,
            game_config: &game_config,
            kwg: &kwg,
            anagrams: arm_tables.anagrams(),
            rack_lengths: arm_tables.rack_lengths(),
            klv: &klv,
        };
        let mut move_generator = movegen::KurniaMoveGenerator::new(&game_config);
        gen_simmer_candidates(
            &mut move_filter::GenMoves::Unfiltered,
            &mut move_generator,
            &board_snapshot,
            &game_state.current_player().rack,
            0,
            SIMMER_FIELD,
            &mut Vec::new(),
        )
        .unwrap();
        assert!(move_generator.plays.len() > 2);
        let made = |memory: Option<endgame::MemoryLimit>| {
            let mut simmer = Simmer::<_, _, true>::new(
                &game_config,
                &kwg,
                &arm_tables,
                &klv,
                SimmerParams {
                    num_sim_iters: 1000,
                    num_sim_plies: 2,
                    sim_threads: 1,
                    win_pct_table: None,
                    quiet: true,
                    memory,
                },
            )
            .unwrap();
            simmer.reseed(11);
            simmer
        };
        let decide = |memory: Option<endgame::MemoryLimit>| {
            let mut simmer = made(memory);
            let decided = simmer.begin_decision(&move_generator, &game_state, 32, &[]);
            (simmer, decided.is_ok())
        };
        let limit = |bytes| {
            Some(endgame::MemoryLimit {
                bytes,
                held: || 0,
                reserved: None,
            })
        };
        let (free, _) = decide(None);
        let (roomy, roomy_ok) = decide(limit(usize::MAX));
        let (small, small_ok) = decide(limit(64));
        let (_, none_ok) = decide(limit(0));
        assert!(free.late_graph.is_some() && roomy.late_graph.is_some());
        assert!(small.late_graph.is_none());
        assert!(roomy_ok && !small_ok && !none_ok);
        let reserved: &'static std::sync::atomic::AtomicUsize =
            Box::leak(Box::new(std::sync::atomic::AtomicUsize::new(0)));
        let mut tight = made(Some(endgame::MemoryLimit {
            bytes: 1 << 20,
            held: || 0,
            reserved: Some(reserved),
        }));
        reserved.store((1 << 20) - 64, std::sync::atomic::Ordering::Relaxed);
        let pruned = tight.begin_decision(&move_generator, &game_state, 0, &[]);
        reserved.store(0, std::sync::atomic::Ordering::Relaxed);
        let rolled = tight.resume(&move_generator, 32);
        assert!(pruned.is_ok() && rolled.is_ok() && tight.late_graph.is_none());
        let board = |simmer: &Simmer<'_, _, _, true>| simmer.leaderboard(usize::MAX);
        assert_eq!(board(&roomy), board(&free));
        assert_eq!(board(&tight), board(&free));
    }

    #[test]
    #[inline]
    fn a_decision_on_a_board_with_tiles_added_prunes_its_late_graph_anew() {
        let game_config = std::sync::Arc::new(game_config::make_english_game_config());
        let kwg = std::sync::Arc::new(
            kwg::kbwg_from_bytes_alloc(tiny_lexicon().to_vec(), false).unwrap(),
        );
        let klv =
            std::sync::Arc::new(klv::kbwg_from_bytes_alloc(klv::EMPTY_KLV_BYTES.to_vec()).unwrap());
        let arm_tables =
            std::sync::Arc::new(anagrams::ArmTables::build(&kwg, &game_config, false).unwrap());
        let mut game_state = game_state::GameState::new(&game_config);
        let mut deal_rng = simmer::DefaultRng::seed_from_u64(7);
        game_state.reset_and_draw_tiles(&game_config, &mut deal_rng);
        game_state.set_current_rack(&[1, 1, 5, 5, 20, 20, 0]);
        let bag = game_state.bag.as_slice()[..12].to_vec();
        game_state.bag.set_from_iter(bag);
        let mut move_generator = movegen::KurniaMoveGenerator::new(&game_config);
        let gen_candidates = |move_generator: &mut movegen::KurniaMoveGenerator,
                              game_state: &game_state::GameState| {
            gen_simmer_candidates(
                &mut move_filter::GenMoves::Unfiltered,
                move_generator,
                &movegen::BoardSnapshot {
                    board_tiles: &game_state.board_tiles,
                    game_config: &game_config,
                    kwg: &kwg,
                    anagrams: arm_tables.anagrams(),
                    rack_lengths: arm_tables.rack_lengths(),
                    klv: &klv,
                },
                &game_state.current_player().rack,
                0,
                SIMMER_FIELD,
                &mut Vec::new(),
            )
            .unwrap();
        };
        let new_simmer = || {
            let mut simmer = Simmer::<_, _, true>::new(
                &game_config,
                &kwg,
                &arm_tables,
                &klv,
                SimmerParams {
                    num_sim_iters: 1000,
                    num_sim_plies: 2,
                    sim_threads: 1,
                    win_pct_table: None,
                    quiet: true,
                    memory: None,
                },
            )
            .unwrap();
            simmer.reseed(11);
            simmer
        };
        gen_candidates(&mut move_generator, &game_state);
        let mut kept = new_simmer();
        kept.begin_decision(&move_generator, &game_state, 2, &[])
            .unwrap();
        let first_board = game_state.board_tiles.clone();
        let play = move_generator.plays[kept.best_so_far()].play.clone();
        game_state.play(&game_config, &mut deal_rng, &play).unwrap();
        game_state.next_turn().unwrap();
        assert_ne!(game_state.board_tiles, first_board);
        gen_candidates(&mut move_generator, &game_state);
        assert!(move_generator.plays.len() > 2);
        kept.reseed(13);
        kept.begin_decision(&move_generator, &game_state, 2, &[])
            .unwrap();
        let mut anew = new_simmer();
        anew.reseed(13);
        anew.begin_decision(&move_generator, &game_state, 2, &[])
            .unwrap();
        assert_ne!(kept.late_board, first_board);
        assert_eq!(kept.late_board, game_state.board_tiles);
        assert_eq!(anew.late_board, game_state.board_tiles);
        assert_eq!(kept.leaderboard(usize::MAX), anew.leaderboard(usize::MAX));
    }

    #[test]
    #[inline]
    fn a_decision_planned_for_its_rollouts_spends_no_more_and_ends_with_one() {
        let game_config = std::sync::Arc::new(game_config::make_english_game_config());
        let kwg = std::sync::Arc::new(
            kwg::kbwg_from_bytes_alloc(tiny_lexicon().to_vec(), false).unwrap(),
        );
        let klv =
            std::sync::Arc::new(klv::kbwg_from_bytes_alloc(klv::EMPTY_KLV_BYTES.to_vec()).unwrap());
        let arm_tables =
            std::sync::Arc::new(anagrams::ArmTables::build(&kwg, &game_config, false).unwrap());
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
        )
        .unwrap();
        let num_plays = move_generator.plays.len();
        assert!(num_plays > 2);
        let mut simmer = Simmer::<_, _, false>::new(
            &game_config,
            &kwg,
            &arm_tables,
            &klv,
            SimmerParams {
                num_sim_iters: 16,
                num_sim_plies: 2,
                sim_threads: 1,
                win_pct_table: None,
                quiet: true,
                memory: None,
            },
        )
        .unwrap();
        simmer.reseed(11);
        simmer
            .begin_decision(&move_generator, &game_state, 0, &[])
            .unwrap();

        // check the plan against a plain count of looks, each look's field the
        // smaller of its cap and the candidates still in
        let by_hand = |rollouts_left: f64| {
            let mut left = rollouts_left - (PRUNE_CADENCE * num_plays as u64) as f64;
            let mut looks = 0;
            loop {
                let cost = (PRUNE_CADENCE * num_plays.min(3 + 2 * looks) as u64) as f64;
                if cost > left {
                    break;
                }
                left -= cost;
                looks += 1;
            }
            PRUNE_CADENCE * (1 + looks as u64)
        };
        assert!(simmer.plan_rollouts(f64::MAX).is_err());
        for rollouts_left in (0..40 * PRUNE_CADENCE * num_plays as u64).step_by(7) {
            simmer.plan_rollouts(rollouts_left as f64).unwrap();
            assert_eq!(
                simmer.num_sim_iters,
                by_hand(rollouts_left as f64),
                "{rollouts_left}"
            );
        }

        // planned again before each iteration from what is left, the decision
        // rolls out no more than it was given, and ends when its cap reaches
        // one
        let given = (PRUNE_CADENCE * num_plays as u64 + 300) as f64;
        let mut spent = 0.0;
        while simmer.active_stream_ids().count() >= 2 {
            assert!(simmer.iters_done < 1000, "the plan never ended it");
            simmer.plan_rollouts(given - spent).unwrap();
            spent += simmer.active_stream_ids().count() as f64;
            simmer.resume(&move_generator, 1).unwrap();
        }
        assert!(spent <= given, "{spent} of {given}");
        assert!(simmer.iters_done > PRUNE_CADENCE);
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
                score: equity::Points::new(score).scaled(),
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
    fn a_sim_given_the_last_move_draws_from_what_the_shell_infers() {
        // the opponent opened AT across the center; the side to move, holding
        // what it was dealt, picks with that move given
        let game_config = std::sync::Arc::new(game_config::make_english_game_config());
        let kwg = std::sync::Arc::new(
            kwg::kbwg_from_bytes_alloc(tiny_lexicon().to_vec(), false).unwrap(),
        );
        let klv =
            std::sync::Arc::new(klv::kbwg_from_bytes_alloc(klv::EMPTY_KLV_BYTES.to_vec()).unwrap());
        let arm_tables =
            std::sync::Arc::new(anagrams::ArmTables::build(&kwg, &game_config, false).unwrap());
        let mut game_state = game_state::GameState::new(&game_config);
        let mut deal_rng = simmer::DefaultRng::seed_from_u64(7);
        game_state.reset_and_draw_tiles(&game_config, &mut deal_rng);
        game_state.set_current_rack(&[1, 20, 5, 5, 0, 1, 20]);
        let board_before = game_state.board_tiles.to_vec();
        let bag_before = game_state.bag.len();
        let opening = movegen::Play::Place {
            down: false,
            lane: 7,
            idx: 7,
            word: [1, 20][..].into(),
            score: equity::Equity::new(4),
        };
        game_state
            .play(&game_config, &mut deal_rng, &opening)
            .unwrap();
        game_state.next_turn().unwrap();
        let last = LastMove {
            board_before: &board_before,
            play: &opening,
            held: infer::Held::Any,
        };
        let rack = game_state.current_player().rack.clone();
        let params = || SimmerParams {
            num_sim_iters: 8,
            num_sim_plies: 2,
            sim_threads: 1,
            win_pct_table: None,
            quiet: true,
            memory: None,
        };

        // the leaves are the inference's own, as the shell asks for them
        let mut simmer =
            Simmer::<_, _, false>::new(&game_config, &kwg, &arm_tables, &klv, params()).unwrap();
        let leaves = simmer.kept_leaves(&rack, last).unwrap();
        assert!(!leaves.is_empty());
        let shell_leaves = infer::kept_leaves(
            &mut infer::InferTables::new(&game_config, &klv).unwrap(),
            &mut movegen::KurniaMoveGenerator::new(&game_config),
            &movegen::BoardSnapshot {
                board_tiles: &board_before,
                game_config: &game_config,
                kwg: &kwg,
                anagrams: None,
                rack_lengths: None,
                klv: &klv,
            },
            &rack,
            &opening,
            infer::FUZZ,
            1,
        )
        .unwrap();
        assert_eq!(leaves, shell_leaves);

        let fuzz = infer::fitted_fuzz(bag_before, bag_before, false);
        assert_eq!(fuzz, (1.507, infer::FUZZ_PRIOR_MIX));
        let mut fitted_simmer =
            Simmer::<_, _, false>::new(&game_config, &kwg, &arm_tables, &klv, params()).unwrap();
        fitted_simmer.set_infer_fuzz(fuzz);
        let fitted = fitted_simmer.kept_leaves(&rack, last).unwrap();
        let mut mixed_tables = infer::InferTables::new(&game_config, &klv).unwrap();
        mixed_tables.set_prior_mix(fuzz.1);
        let shell_fitted = infer::kept_leaves(
            &mut mixed_tables,
            &mut movegen::KurniaMoveGenerator::new(&game_config),
            &movegen::BoardSnapshot {
                board_tiles: &board_before,
                game_config: &game_config,
                kwg: &kwg,
                anagrams: None,
                rack_lengths: None,
                klv: &klv,
            },
            &rack,
            &opening,
            fuzz.0,
            1,
        )
        .unwrap();
        assert_eq!(fitted, shell_fitted);
        assert_ne!(fitted, leaves);

        // and the move picked with the last move given is the one a decision
        // handed those leaves picks
        let board_snapshot = movegen::BoardSnapshot {
            board_tiles: &game_state.board_tiles,
            game_config: &game_config,
            kwg: &kwg,
            anagrams: arm_tables.anagrams(),
            rack_lengths: arm_tables.rack_lengths(),
            klv: &klv,
        };
        let mut by_hand =
            Simmer::<_, _, false>::new(&game_config, &kwg, &arm_tables, &klv, params()).unwrap();
        by_hand.reseed(3);
        let mut move_generator = movegen::KurniaMoveGenerator::new(&game_config);
        gen_simmer_candidates(
            &mut move_filter::GenMoves::Unfiltered,
            &mut move_generator,
            &board_snapshot,
            &rack,
            0,
            SIMMER_FIELD,
            &mut Vec::new(),
        )
        .unwrap();
        by_hand
            .begin_decision(&move_generator, &game_state, 8, &shell_leaves)
            .unwrap();
        let want = move_generator.plays[top_candidate_play_index_by_mean(&by_hand.candidates)]
            .play
            .clone();
        simmer.reseed(3);
        let mut picker = MovePicker::Simmer(Box::new(simmer));
        let mut picked = movegen::KurniaMoveGenerator::new(&game_config);
        picker
            .pick_a_move_after(
                &mut move_filter::GenMoves::Unfiltered,
                &mut picked,
                &board_snapshot,
                &game_state,
                &rack,
                (Some(last), &mut infer::Carried::default()),
            )
            .unwrap();
        assert!(picked.plays[0].play == want);
        // its sim drew the opponent's racks as that decision did, so every
        // candidate's mean is the same to the bit
        let MovePicker::Simmer(picked_simmer) = &picker else {
            unreachable!()
        };
        let means = |candidates: &[Candidate]| {
            candidates
                .iter()
                .map(|candidate| candidate.stats.mean())
                .collect::<Vec<_>>()
        };
        assert_eq!(means(&picked_simmer.candidates), means(&by_hand.candidates));
    }

    #[test]
    #[inline]
    fn a_sim_infers_each_opponent_move_from_what_it_inferred_at_the_last() {
        // late in a game, 70 tiles down in the bag's order: two sims play it
        // out, each seat starting its inference of the opponent's move from
        // what its last one carried and the tiles it has put back since, as a
        // chain of inferences by hand does, and the carry changes what some
        // inference weighs
        let game_config = game_config::make_english_game_config();
        let kwg = kwg::kbwg_from_bytes_alloc(tiny_lexicon().to_vec(), false).unwrap();
        let klv = klv::kbwg_from_bytes_alloc(klv::EMPTY_KLV_BYTES.to_vec()).unwrap();
        let arm_tables = anagrams::ArmTables::build(&kwg, &game_config, false).unwrap();
        let alphabet = game_config.alphabet();
        let mut tiles = Vec::new();
        for tile in 0..alphabet.len() {
            for _ in 0..alphabet.freq(tile) {
                tiles.push(tile);
            }
        }
        let mut game_state = game_state::GameState::new(&game_config);
        for (square, &tile) in tiles[..70].iter().enumerate() {
            game_state.board_tiles[square] = if tile == 0 { 0x81 } else { tile };
        }
        game_state.players[0].rack = tiles[70..77].to_vec();
        game_state.players[1].rack = tiles[77..84].to_vec();
        game_state.bag.set_from_iter(tiles[84..].iter().copied());
        let (game_config, kwg, arm_tables, klv) = (
            std::sync::Arc::new(game_config),
            std::sync::Arc::new(kwg),
            std::sync::Arc::new(arm_tables),
            std::sync::Arc::new(klv),
        );
        let params = || SimmerParams {
            num_sim_iters: 2,
            num_sim_plies: 1,
            sim_threads: 1,
            win_pct_table: None,
            quiet: true,
            memory: None,
        };
        let mut pickers = [0, 1].map(|_| {
            MovePicker::Simmer(Box::new(
                Simmer::<_, _, false>::new(&game_config, &kwg, &arm_tables, &klv, params())
                    .unwrap(),
            ))
        });
        let mut rng = simmer::DefaultRng::seed_from_u64(7);
        let mut move_generator = movegen::KurniaMoveGenerator::new(&game_config);
        let mut tables = infer::InferTables::new(&game_config, &klv).unwrap();
        let mut infer_generator = movegen::KurniaMoveGenerator::new(&game_config);
        // each seat's carry (the pickers' and the hand's) and the tiles it has
        // put back since
        let mut carried = [(); 2].map(|_| infer::Carried::default());
        let mut by_hand = [(); 2].map(|_| infer::Carried::default());
        let mut put_back = [Vec::new(), Vec::new()];
        let mut next = infer::Carried::default();
        let mut last: Option<(Vec<u8>, movegen::Play)> = None;
        let (mut weighed, mut changed) = (0, 0);
        let mut final_scores = vec![0; 2];
        loop {
            let seat = game_state.turn as usize;
            let board_snapshot = movegen::BoardSnapshot {
                board_tiles: &game_state.board_tiles,
                game_config: &game_config,
                kwg: &kwg,
                anagrams: arm_tables.anagrams(),
                rack_lengths: arm_tables.rack_lengths(),
                klv: &klv,
            };
            let rack = game_state.current_player().rack.clone();
            pickers[seat]
                .pick_a_move_after(
                    &mut move_filter::GenMoves::Unfiltered,
                    &mut move_generator,
                    &board_snapshot,
                    &game_state,
                    &rack,
                    (
                        last.as_ref().map(|(board_before, play)| LastMove {
                            board_before,
                            play,
                            held: infer::Held::Kept(&carried[seat], &put_back[seat]),
                        }),
                        &mut next,
                    ),
                )
                .unwrap();
            if let Some((board_before, play)) = &last {
                let mut infer_by_hand = |held, carried: &mut infer::Carried| {
                    let mut leaves = infer::PackedLeaves::new();
                    infer::kept_leaves_carried_into(
                        &mut tables,
                        &mut infer_generator,
                        &movegen::BoardSnapshot {
                            board_tiles: board_before,
                            game_config: &game_config,
                            kwg: &kwg,
                            anagrams: None,
                            rack_lengths: None,
                            klv: &klv,
                        },
                        &rack,
                        (play, held),
                        infer::InferOptions {
                            fuzz: infer::FUZZ,
                            threads: 1,
                            weigh_passes: false,
                        },
                        (&mut leaves, Some(carried)),
                    )
                    .unwrap();
                    (0..leaves.len())
                        .map(|i| (leaves.leave(i).to_vec(), leaves.weight(i).to_bits()))
                        .collect::<Vec<_>>()
                };
                let mut hand_next = infer::Carried::default();
                let want = infer_by_hand(
                    infer::Held::Kept(&by_hand[seat], &put_back[seat]),
                    &mut hand_next,
                );
                let any = infer_by_hand(infer::Held::Any, &mut infer::Carried::default());
                let MovePicker::Simmer(simmer) = &pickers[seat] else {
                    unreachable!()
                };
                let leaves = &simmer.inferred;
                let got = (0..leaves.len())
                    .map(|i| (leaves.leave(i).to_vec(), leaves.weight(i).to_bits()))
                    .collect::<Vec<_>>();
                assert_eq!(got, want);
                weighed += usize::from(!want.is_empty());
                changed += usize::from(want != any);
                by_hand[seat] = hand_next;
            }
            std::mem::swap(&mut carried[seat], &mut next);
            let play = move_generator.plays[0].play.clone();
            put_back[seat].clear();
            if let movegen::Play::Exchange { tiles } = &play {
                put_back[seat].extend_from_slice(tiles);
            }
            let board_before = game_state.board_tiles.to_vec();
            game_state.play(&game_config, &mut rng, &play).unwrap();
            last = Some((board_before, play));
            match game_state
                .check_game_ended(&game_config, &mut final_scores)
                .unwrap()
            {
                game_state::CheckGameEnded::NotEnded => game_state.next_turn().unwrap(),
                _ => break,
            }
        }
        assert!(weighed > 1 && changed > 0);
    }

    #[test]
    #[inline]
    fn the_lane_pick_stays_in_its_window_and_field_and_keeps_the_earlier_on_a_tie() {
        let game_config = game_config::make_english_game_config();
        let (board_tiles, plays) = lanes_field(&game_config);
        for (window, want) in [(0, 0), (equity::SCALE, 1), (2 * equity::SCALE, 1)] {
            assert_eq!(
                lane_pick(&game_config, &board_tiles, &plays, window, &mut Vec::new()),
                want,
                "window {window}",
            );
        }
        // deft's field: a move past its first DEFT_FIELD is not weighed
        let mut field = vec![plays[0].clone(); DEFT_FIELD];
        field.push(plays[1].clone());
        assert_eq!(
            lane_pick(
                &game_config,
                &board_tiles,
                &field,
                equity::SCALE,
                &mut Vec::new()
            ),
            0
        );
        field.swap(DEFT_FIELD - 1, DEFT_FIELD);
        assert_eq!(
            lane_pick(
                &game_config,
                &board_tiles,
                &field,
                equity::SCALE,
                &mut Vec::new()
            ),
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
            game_state.players[mover].score = equity::Equity::new(100 * equity::SCALE);
            game_state.players[1 - mover].score =
                equity::Equity::new((100 + behind) * equity::SCALE);
            assert_eq!(
                winning_out(&game_config, &game_state, &plays),
                want,
                "{behind} behind",
            );
        }
        game_state.players[1 - mover].score = equity::Equity::ZERO;
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
                    score: equity::Equity::new(0),
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
                s.set(dim.at_on_board(row, col));
            }
            s.0
        };
        // AT on row 7 from column 5; an E and an A down column 4 from row 6,
        // the A making AAT along row 7
        let board_tiles = board_with(&game_config, 7, 0);
        let play = place(0, true, 4, 6, &[5, 1], 0).play;
        let fp = footprint(dim, &board_tiles, &play).unwrap();
        let malformed = matrix::Dim { rows: 15, cols: -1 };
        assert!(zone_lanes(&malformed, &fp.zone).is_err());
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
        let (rows, cols) = zone_lanes(dim, &fp.zone).unwrap();
        assert_eq!((rows, cols), (0b1_1110_0000, 0b1011_1000));
        assert!(fp.on(0, 1 << 4) && !fp.on(!0, !(1 << 4)));
        // AT along row 0 from its corner, on the empty board: nothing past the
        // edges
        let empty = vec![0u8; dim.rows as usize * dim.cols as usize];
        let fp = footprint(dim, &empty, &place(0, false, 0, 0, &[1, 20], 0).play).unwrap();
        assert_eq!(fp.fills.0, squares(&[(0, 0), (0, 1)]));
        assert_eq!(
            fp.zone.0,
            squares(&[(0, 0), (0, 1), (0, 2), (1, 0), (1, 1)])
        );
        assert!(fp.on(1, 0) && !fp.on(!1, !0));
        // AT along row 0 from column 1, and along row 14 to column 13: the
        // squares just inside the edges are in the zone
        let fp = footprint(dim, &empty, &place(0, false, 0, 1, &[1, 20], 0).play).unwrap();
        assert_eq!(
            fp.zone.0,
            squares(&[(0, 1), (0, 2), (0, 0), (0, 3), (1, 1), (1, 2)])
        );
        let fp = footprint(dim, &empty, &place(0, false, 14, 12, &[1, 20], 0).play).unwrap();
        assert_eq!(
            fp.zone.0,
            squares(&[(14, 12), (14, 13), (14, 11), (14, 14), (13, 12), (13, 13)])
        );
        // a move placing no tiles has none
        let pass = movegen::Play::Exchange {
            tiles: [0u8; 0][..].into(),
        };
        let fp = footprint(dim, &board_tiles, &pass).unwrap();
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
        game_state.players[mover].score = equity::Equity::new((100 + lead) * equity::SCALE);
        game_state.players[1 - mover].score = equity::Equity::new(100 * equity::SCALE);
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
        if first.left(choice).is_empty() {
            return c;
        }
        let left_twice = twice(first.board_snapshot.game_config, first.left(choice));
        let stuck = -i64::from(equity::ENDGAME_PENALTY_BASE) - left_twice;
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
        let kwg = kwg::kbwg_from_bytes_alloc(at_lexicon(false).to_vec(), false).unwrap();
        let klv = klv::kbwg_from_bytes_alloc(klv::EMPTY_KLV_BYTES.to_vec()).unwrap();
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
            assert_eq!(values[i], want * i64::from(equity::SCALE));
            assert_eq!(hopes[i], i64::MIN);
        }
        assert_eq!(first.choices.len(), 3);
    }

    #[test]
    #[inline]
    fn out_in_two_raises_a_choice_whose_move_opens_its_out() {
        let game_config = game_config::make_english_game_config();
        let kwg = kwg::kbwg_from_bytes_alloc(at_lexicon(true).to_vec(), false).unwrap();
        let klv = klv::kbwg_from_bytes_alloc(klv::EMPTY_KLV_BYTES.to_vec()).unwrap();
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
        let scale = i64::from(equity::SCALE);
        assert_eq!((values[i], hopes[i]), (-3 * scale, 20 * scale));
        assert_eq!((values[j], hopes[j]), (14 * scale, i64::MIN));
        first.raise_openers(&mut scratch, &mut values, &hopes);
        assert_eq!((values[i], values[j]), (15 * scale, 14 * scale));
        // ATT and ATTT next beat ATTT at once unless that already wins
        for (lead, wins) in [(-15, false), (-14, false), (-13, true)] {
            let game_state = at_endgame_on(&game_config, 14, lead);
            let found = out_in_two(
                &board_snapshot,
                &game_state,
                std::slice::from_ref(&attt),
                0,
                &mut Scratch::new(&game_config),
            );
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
        let stuck = -i64::from(equity::ENDGAME_PENALTY_BASE) - kept_twice;
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
                    footprint(dim, &scratch.after, &m.play).unwrap().zone,
                )
            })
            .collect::<Vec<_>>();
        outs.sort_unstable_by_key(|&(s, _)| std::cmp::Reverse(s));
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
            let fills = footprint(dim, &scratch.after, &m.play).unwrap().fills;
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
        let kwg = kwg::kbwg_from_bytes_alloc(at_lexicon(false).to_vec(), false).unwrap();
        let klv = klv::kbwg_from_bytes_alloc(klv::EMPTY_KLV_BYTES.to_vec()).unwrap();
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
            Some(-6 * i64::from(equity::SCALE))
        );
        assert_eq!(
            value_of(&att_along, &mut scratch),
            Some(10 * i64::from(equity::SCALE))
        );
        for (lead, blocks) in [(-20, true), (6, true), (7, false)] {
            let game_state = at_endgame(&game_config, lead);
            let found = out_in_two(
                &board_snapshot,
                &game_state,
                std::slice::from_ref(&at_down),
                0,
                &mut Scratch::new(&game_config),
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
        let kwg = kwg::kbwg_from_bytes_alloc(
            build::build(
                build::BuildContent::Gaddawg,
                build::BuildLayout::Wolges,
                build::BuildOrder::Sorted,
                &words,
            )
            .unwrap()
            .to_vec(),
            false,
        )
        .unwrap();
        let klv = klv::kbwg_from_bytes_alloc(klv::EMPTY_KLV_BYTES.to_vec()).unwrap();
        let mut board_tiles = vec![0u8; dim.rows as usize * dim.cols as usize];
        board_tiles[dim.at_on_board(7, 7)] = 1; // A
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
                Some(6 * i64::from(equity::SCALE))
            );
        }
        let found = out_in_two(
            &board_snapshot,
            &game_state,
            std::slice::from_ref(&along),
            0,
            &mut Scratch::new(&game_config),
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
        let kwg = kwg::kbwg_from_bytes_alloc(at_lexicon(true).to_vec(), false).unwrap();
        let klv = klv::kbwg_from_bytes_alloc(klv::EMPTY_KLV_BYTES.to_vec()).unwrap();
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
            Some(12 * i64::from(equity::SCALE))
        );
        assert_eq!(
            value_of(&attt_along, &mut scratch),
            Some(10 * i64::from(equity::SCALE))
        );
        let found = out_in_two(
            &board_snapshot,
            &game_state,
            &[at_down],
            0,
            &mut Scratch::new(&game_config),
        );
        assert!(found.is_some_and(|play| play == att_along));
    }

    // deft with the bag empty: going out when that wins, else out in two's move
    #[test]
    #[inline]
    fn deft_plays_out_in_two_when_going_out_would_lose() {
        let game_config = game_config::make_english_game_config();
        let kwg = kwg::kbwg_from_bytes_alloc(at_lexicon(false).to_vec(), false).unwrap();
        let klv = klv::kbwg_from_bytes_alloc(klv::EMPTY_KLV_BYTES.to_vec()).unwrap();
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
            let mut deft =
                MovePicker::<kwg::Node24, kwg::Node24>::Deft(Deft::new(DEFAULT_DEFT_WINDOW, None));
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
        let kwg = kwg::kbwg_from_bytes_alloc(at_lexicon(false).to_vec(), false).unwrap();
        let klv = klv::kbwg_from_bytes_alloc(klv::EMPTY_KLV_BYTES.to_vec()).unwrap();
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
            let mut deft = MovePicker::<kwg::Node24, kwg::Node24>::Deft(Deft::new(0, None));
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
    fn end_words_kwg(game_config: &game_config::GameConfig) -> kwg::Kwg<kwg::Node24> {
        let reader = alphabet::AlphabetReader::new_for_words(game_config.alphabet());
        let mut word_buf = Vec::new();
        let mut words = Vec::<bites::Bites>::new();
        for w in END_WORDS {
            reader.set_word(w, &mut word_buf).unwrap();
            words.push(word_buf[..].into());
        }
        words.sort_unstable();
        kwg::kbwg_from_bytes_alloc(
            build::build(
                build::BuildContent::Gaddawg,
                build::BuildLayout::Wolges,
                build::BuildOrder::Sorted,
                &words,
            )
            .unwrap()
            .to_vec(),
            false,
        )
        .unwrap()
    }

    // deft's picks on a board and racks, its move as the movegen prints it
    #[inline]
    fn deft_picks<N: kwg::Node + Send + Sync + 'static, L: kwg::Node + Send + Sync + 'static>(
        board_snapshot: &movegen::BoardSnapshot<'_, N, L>,
        game_state: &game_state::GameState,
    ) -> String {
        picks_of(
            Deft::new(DEFAULT_DEFT_WINDOW, None),
            board_snapshot,
            game_state,
        )
    }

    #[inline]
    fn picks_of<N: kwg::Node + Send + Sync + 'static, L: kwg::Node + Send + Sync + 'static>(
        deft: Deft,
        board_snapshot: &movegen::BoardSnapshot<'_, N, L>,
        game_state: &game_state::GameState,
    ) -> String {
        let mut move_generator = movegen::KurniaMoveGenerator::new(board_snapshot.game_config);
        let mut filtered_movegen = move_filter::GenMoves::Unfiltered;
        let mut deft = MovePicker::<N, L>::Deft(deft);
        deft.pick_a_move(
            &mut filtered_movegen,
            &mut move_generator,
            board_snapshot,
            game_state,
            &game_state.players[game_state.turn as usize].rack,
        )
        .unwrap();
        assert_eq!(move_generator.plays.len(), 1);
        move_generator.plays[0]
            .play
            .fmt(board_snapshot)
            .unwrap()
            .to_string()
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
        let klv = klv::kbwg_from_bytes_alloc(klv::EMPTY_KLV_BYTES.to_vec()).unwrap();
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
        filtered_movegen
            .gen_moves(
                &mut move_generator,
                &board_snapshot,
                &game_state.players[mover].rack,
                0,
                DEFT_LATE_FIELD,
                None,
            )
            .unwrap();
        let plays = &move_generator.plays;
        let fewest = lane_pick(
            &game_config,
            &board_tiles,
            plays,
            DEFAULT_DEFT_WINDOW,
            &mut Vec::new(),
        );
        assert_eq!(
            plays[fewest].play.fmt(&board_snapshot).unwrap().to_string(),
            "F6 CASTE 28"
        );
        assert!(
            out_in_two(
                &board_snapshot,
                &game_state,
                plays,
                fewest,
                &mut Scratch::new(&game_config),
            )
            .is_none()
        );
    }

    // with the bag empty and no out that wins, deft plays out in two's move:
    // holding ERS against AT 30 behind, E(A)RS (7) goes out, where the play at
    // fewest is ES (13), which keeps the R
    #[test]
    #[inline]
    fn deft_plays_out_in_two_over_the_play_at_fewest() {
        let game_config = game_config::make_english_game_config();
        let kwg = end_words_kwg(&game_config);
        let klv = klv::kbwg_from_bytes_alloc(klv::EMPTY_KLV_BYTES.to_vec()).unwrap();
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
        game_state.players[mover].score = equity::Equity::new(170 * equity::SCALE);
        game_state.players[1 - mover].score = equity::Equity::new(200 * equity::SCALE);
        assert_eq!(deft_picks(&board_snapshot, &game_state), "H10 E(A)RS 7");
        let mut move_generator = movegen::KurniaMoveGenerator::new(&game_config);
        let mut filtered_movegen = move_filter::GenMoves::Unfiltered;
        filtered_movegen
            .gen_moves(
                &mut move_generator,
                &board_snapshot,
                &game_state.players[mover].rack,
                0,
                DEFT_LATE_FIELD,
                None,
            )
            .unwrap();
        let plays = &move_generator.plays;
        let fewest = lane_pick(
            &game_config,
            &board_tiles,
            plays,
            DEFAULT_DEFT_WINDOW,
            &mut Vec::new(),
        );
        assert_eq!(
            plays[fewest].play.fmt(&board_snapshot).unwrap().to_string(),
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
        let kwg = kwg::kbwg_from_bytes_alloc(tiny_lexicon().to_vec(), false).unwrap();
        let klv = klv::kbwg_from_bytes_alloc(klv::EMPTY_KLV_BYTES.to_vec()).unwrap();
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
        filtered_movegen
            .gen_moves(
                &mut move_generator,
                &board_snapshot,
                &game_state.players[mover].rack,
                0,
                DEFT_LATE_FIELD,
                None,
            )
            .unwrap();
        let at = move_generator
            .plays
            .iter()
            .position(|m| m.play.fmt(&board_snapshot).unwrap().to_string() == "H7 ET 10")
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
        let klv = klv::kbwg_from_bytes_alloc(klv::EMPTY_KLV_BYTES.to_vec()).unwrap();
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
                        choice.play.fmt(&board_snapshot).unwrap(),
                    );
                    assert_eq!(
                        values[i],
                        value_on_footprints_by_hand(&first, i),
                        "{fen} {mine} against {theirs}, on footprints: {}",
                        choice.play.fmt(&board_snapshot).unwrap(),
                    );
                    valued += 1;
                }
            }
        }
        assert!(valued > 100, "only {valued} choices valued");
    }

    #[test]
    #[inline]
    fn a_positional_term_parses_its_values_file() {
        let term =
            PositionalTerm::parse("# by square\n3 1 7 -2\n\n2 1 1 -1.5 # the star\n1 2 5 0.0004\n")
                .unwrap();
        let want = vec![((1, 2, 5), 0), ((2, 1, 1), -1500), ((3, 1, 7), -2000)];
        assert_eq!(term, PositionalTerm::new(want).unwrap());
        for empty in ["", "# none\n"] {
            assert_eq!(
                PositionalTerm::parse(empty).unwrap(),
                PositionalTerm::new(Vec::new()).unwrap()
            );
        }
        assert_eq!(
            PositionalTerm::parse("1 1 1 2\n0 0 1 2\n3 1 0 2\n2 1 1 -1\n").unwrap(),
            PositionalTerm::new(vec![((2, 1, 1), -1000)]).unwrap()
        );
        assert_eq!(
            PositionalTerm::parse("1 1 1 2\n").unwrap(),
            PositionalTerm::new(Vec::new()).unwrap()
        );
        for refused in [
            "3 1 1\n",
            "3 1 1 2 5\n",
            "x 1 1 2\n",
            "3 1 256 2\n",
            "3 1 1 2\n3 1 1 4\n",
            "3 1 1 inf\n",
            "3 1 1 NaN\n",
            "3 1 1 3e9\n",
        ] {
            assert!(PositionalTerm::parse(refused).is_err(), "{refused:?}");
        }
    }

    #[test]
    #[inline]
    fn a_positional_term_values_the_premium_squares_a_board_leaves_in_reach() {
        let game_config = game_config::make_english_game_config();
        let dim = game_config.board_layout().dim();
        let term = PositionalTerm::parse("3 1 7 -2\n2 1 1 -1.5\n").unwrap();
        let mut board_tiles = vec![0u8; dim.rows as usize * dim.cols as usize];
        let mut reach = Vec::new();
        assert_eq!(term.value_of(&game_config, &board_tiles, &mut reach), -1500);
        board_tiles[dim.at_on_board(7, 7)] = 1;
        assert_eq!(term.value_of(&game_config, &board_tiles, &mut reach), -8000);
    }

    #[test]
    #[inline]
    fn a_positional_term_adds_to_each_moves_equity_and_orders_them_by_it() {
        let game_config = game_config::make_english_game_config();
        let dim = game_config.board_layout().dim();
        let board_tiles = vec![0u8; dim.rows as usize * dim.cols as usize];
        let term = PositionalTerm::parse("2 1 1 -5\n").unwrap();
        let exchange = movegen::ValuedMove {
            equity: equity::Equity::new(12_000),
            play: movegen::Play::Exchange {
                tiles: [5u8][..].into(),
            },
        };
        let mut plays = vec![
            exchange.clone(),
            place(10_000, false, 7, 7, &[1, 20], 4),
            place(10_000, false, 7, 6, &[1, 20], 4),
        ];
        term.add_to(
            &game_config,
            &board_tiles,
            &mut plays,
            &mut Scratch::new(&game_config),
        )
        .unwrap();
        assert!(plays[0] == place(15_000, false, 7, 7, &[1, 20], 4));
        assert!(plays[1] == place(15_000, false, 7, 6, &[1, 20], 4));
        assert!(plays[2] == exchange);
        let huge = PositionalTerm::new(vec![((2, 1, 1), i32::MIN)]).unwrap();
        assert!(
            huge.add_to(
                &game_config,
                &board_tiles,
                &mut plays,
                &mut Scratch::new(&game_config),
            )
            .is_err()
        );
    }

    #[inline]
    fn care_positions(
        game_config: &game_config::GameConfig,
    ) -> (Vec<u8>, Vec<game_state::GameState>) {
        let mut fen_parser =
            display::BoardFenParser::new(game_config.alphabet(), game_config.board_layout());
        let board_tiles = fen_parser
            .parse("15/15/15/15/15/15/15/6CARE5/6A8/6N8/6EATS5/15/15/15/15")
            .unwrap()
            .to_vec();
        let rack_reader = alphabet::AlphabetReader::new_for_racks(game_config.alphabet());
        let mut positions = Vec::new();
        for rack in ["ACERST", "AENRST", "CENT", "EATS", "RACE", "STAR"] {
            for bag in [0, 3, 20] {
                let mut game_state = game_state::GameState::new(game_config);
                game_state.board_tiles.copy_from_slice(&board_tiles);
                game_state.bag.set_from_iter(std::iter::repeat_n(5u8, bag));
                let mover = game_state.turn as usize;
                rack_reader
                    .set_word(rack, &mut game_state.players[mover].rack)
                    .unwrap();
                rack_reader
                    .set_word("ET", &mut game_state.players[1 - mover].rack)
                    .unwrap();
                positions.push(game_state);
            }
        }
        (board_tiles, positions)
    }

    #[test]
    #[inline]
    fn deft_with_a_zero_positional_term_plays_as_deft() {
        let game_config = game_config::make_english_game_config();
        let kwg = end_words_kwg(&game_config);
        let klv = klv::kbwg_from_bytes_alloc(klv::EMPTY_KLV_BYTES.to_vec()).unwrap();
        let (board_tiles, positions) = care_positions(&game_config);
        let board_snapshot = movegen::BoardSnapshot {
            board_tiles: &board_tiles,
            game_config: &game_config,
            kwg: &kwg,
            anagrams: None,
            rack_lengths: None,
            klv: &klv,
        };
        for text in ["3 1 1 0\n2 1 1 0\n1 3 1 0\n1 2 1 0\n", ""] {
            let zero = PositionalTerm::parse(text).unwrap();
            for game_state in &positions {
                for window in [0, DEFAULT_DEFT_WINDOW] {
                    assert_eq!(
                        picks_of(
                            Deft::new(window, Some(zero.clone())),
                            &board_snapshot,
                            game_state
                        ),
                        picks_of(Deft::new(window, None), &board_snapshot, game_state),
                    );
                }
            }
        }
    }

    #[test]
    #[inline]
    fn deft_takes_a_move_its_positional_term_values_most() {
        let game_config = game_config::make_english_game_config();
        let kwg = end_words_kwg(&game_config);
        let klv = klv::kbwg_from_bytes_alloc(klv::EMPTY_KLV_BYTES.to_vec()).unwrap();
        let (board_tiles, positions) = care_positions(&game_config);
        let board_snapshot = movegen::BoardSnapshot {
            board_tiles: &board_tiles,
            game_config: &game_config,
            kwg: &kwg,
            anagrams: None,
            rack_lengths: None,
            klv: &klv,
        };
        let term =
            PositionalTerm::parse("3 1 1 -100\n2 1 1 -100\n1 3 1 -100\n1 2 1 -100\n").unwrap();
        let (mut reach, mut after) = (Vec::new(), Vec::new());
        let before = term.value_of(&game_config, &board_tiles, &mut reach);
        let mut changed = 0;
        for game_state in positions
            .iter()
            .filter(|game_state| game_state.bag.len() > game_config.rack_size() as usize)
        {
            let mut move_generator = movegen::KurniaMoveGenerator::new(&game_config);
            move_filter::GenMoves::Unfiltered
                .gen_moves(
                    &mut move_generator,
                    &board_snapshot,
                    &game_state.players[game_state.turn as usize].rack,
                    0,
                    DEFT_FIELD,
                    None,
                )
                .unwrap();
            let sums = move_generator
                .plays
                .iter()
                .map(|m| {
                    glance::board_after(&game_config, &board_tiles, &m.play, &mut after);
                    let added = term.value_of(&game_config, &after, &mut reach) - before;
                    (
                        m.equity.raw() as i64 + added,
                        m.play.fmt(&board_snapshot).unwrap().to_string(),
                    )
                })
                .collect::<Vec<_>>();
            let most = sums.iter().map(|&(sum, _)| sum).max().unwrap();
            let picked = picks_of(
                Deft::new(0, Some(term.clone())),
                &board_snapshot,
                game_state,
            );
            assert!(
                sums.iter()
                    .any(|(sum, play)| *sum == most && *play == picked),
                "{picked} of {sums:?}"
            );
            changed += (picked != sums[0].1) as usize;
        }
        assert!(changed > 0, "the term changed no pick");
    }

    #[test]
    #[inline]
    fn a_sims_rollouts_value_leaves_with_the_klv_it_is_given() {
        let game_config = std::sync::Arc::new(game_config::make_english_game_config());
        let kwg = std::sync::Arc::new(
            kwg::kbwg_from_bytes_alloc(tiny_lexicon().to_vec(), false).unwrap(),
        );
        let klv = std::sync::Arc::new(
            klv::kbwg_from_bytes_alloc(klv::make_klv2(&[
                (&[1], 3.5),
                (&[1, 5], 4.0),
                (&[5], 1.25),
                (&[20], -2.0),
            ]))
            .unwrap(),
        );
        let arm_tables =
            std::sync::Arc::new(anagrams::ArmTables::build(&kwg, &game_config, false).unwrap());
        let mut game_state = game_state::GameState::new(&game_config);
        let mut deal_rng = simmer::DefaultRng::seed_from_u64(7);
        game_state.reset_and_draw_tiles(&game_config, &mut deal_rng);
        game_state.set_current_rack(&[1, 20, 5, 5, 0, 1, 20]);
        let rack = game_state.current_player().rack.clone();
        let params = || SimmerParams {
            num_sim_iters: 8,
            num_sim_plies: 2,
            sim_threads: 1,
            win_pct_table: None,
            quiet: true,
            memory: None,
        };
        let board_snapshot = movegen::BoardSnapshot {
            board_tiles: &game_state.board_tiles,
            game_config: &game_config,
            kwg: &kwg,
            anagrams: arm_tables.anagrams(),
            rack_lengths: arm_tables.rack_lengths(),
            klv: &klv,
        };
        let decide = |rollout_klv: Option<std::sync::Arc<klv::Klv<_>>>| {
            let mut simmer =
                Simmer::<_, _, false>::new(&game_config, &kwg, &arm_tables, &klv, params())
                    .unwrap();
            if let Some(rollout_klv) = rollout_klv {
                simmer.set_rollout_klv(rollout_klv);
            }
            simmer.reseed(3);
            let mut picker = MovePicker::Simmer(Box::new(simmer));
            let mut picked = movegen::KurniaMoveGenerator::new(&game_config);
            picker
                .pick_a_move_after(
                    &mut move_filter::GenMoves::Unfiltered,
                    &mut picked,
                    &board_snapshot,
                    &game_state,
                    &rack,
                    (None, &mut infer::Carried::default()),
                )
                .unwrap();
            let MovePicker::Simmer(simmer) = &picker else {
                unreachable!()
            };
            (simmer.candidates.iter())
                .map(|candidate| candidate.stats.mean())
                .collect::<Vec<_>>()
        };
        let untold = decide(None);
        assert!(untold.len() >= 2);
        assert_eq!(decide(Some(std::sync::Arc::clone(&klv))), untold);
        let num_letters = game_config.alphabet().len() as usize;
        let other = klv.revalued(num_letters, |tally, leave| leave + 1000 * tally[1] as i32);
        assert_ne!(decide(Some(std::sync::Arc::new(other))), untold);
    }
}
