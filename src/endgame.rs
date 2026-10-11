// Copyright (C) 2020-2026 Andy Kurnia.

// note: this module is very slow and may need a lot of space
// and it still has many bugs

use super::{anagrams, display, equity, error, fash, game_config, klv, kwg, movegen};
use std::io::Write;

// how many nodes the search visits between looks at the clock for its time limit
const LIMIT_CADENCE: u64 = 1024;

// move one tile at a time from rack
#[derive(Clone, Eq, Hash, PartialEq)]
struct PlacedTile {
    tile: u8,  // 0x01-0x3f, 0x81-0xbf
    whose: u8, // 0 or 1
    idx: i16,  // 0..r * c
}

// canonical order of tile placements from start-of-endgame state (state 0)
// - tiles are placed in (tile, idx) order
// - each (blank-wiped) tile coming from both players are placed by p0 first
#[derive(Clone, Eq, Hash, PartialEq)]
struct State {
    parent: u32,
    placed_tile: PlacedTile,
}

#[derive(Clone)]
enum StateSideEvalEquityType {
    Exact,      // ==
    LowerBound, // >=
    UpperBound, // <=
}

// best move for a side
#[derive(Clone)]
struct StateSideEval {
    equity: f32,
    play_idx: u32,
    new_state_idx: u32, // not cheap to regen
    equity_type: StateSideEvalEquityType,
    depth: i8,
}

impl StateSideEval {
    #[inline(always)]
    fn new() -> Self {
        Self {
            equity: f32::NEG_INFINITY,
            play_idx: !0,
            new_state_idx: !0,
            equity_type: StateSideEvalEquityType::LowerBound,
            depth: i8::MIN,
        }
    }
}

// best move for both sides
struct StateEval {
    best_place_move: [StateSideEval; 2],
    best_move: [StateSideEval; 2], // pass allowed
    child_play_idxs: [usize; 3],   // workbuf.child_plays[a..b]=p0, [b..c]=p1
}

// misnomer now. there used to be one per ply, now there's just one.
#[derive(Default)]
struct PlyBuffer {
    board_tiles: Vec<u8>,
    racks: [Vec<u8>; 2],
}

struct ChildPlay {
    new_state_idx: u32, // workbuf.states; 0=play out, !0=missing, same idx = pass
    play_idx: u32,      // workbuf.plays
    valuation: f32,     // refined over time
}

// WorkBuffer contains reusable allocations.
// WorkBuffer can only be reused for the same game_config and kwg.
// (Refer to note at KurniaMoveGenerator.)
// This is not enforced.
struct WorkBuffer {
    t0: std::time::Instant, // the solve's start, for its time limit
    dur_movegen: std::time::Duration,
    vec_placed_tile: Vec<PlacedTile>,
    current_ply_buffer: PlyBuffer, // only using one
    movegen: movegen::KurniaMoveGenerator,
    states: Vec<State>, // [0] = dummy initial state, excludes play outs
    state_finder: fash::MyHashMap<State, u32>, // maps all states except 0
    state_eval: fash::MyHashMap<u32, StateEval>,
    plays: Vec<movegen::Play>, // global u32->Play mapping. [0] = pass, [1..] = place
    play_finder: fash::MyHashMap<movegen::Play, u32>, // maps all plays except pass
    child_plays: Vec<ChildPlay>, // subslices of StateEval, often re-sorted; excludes pass
    depth_limited: bool,       // set when a line was cut short by the ply limit this pass
    max_depth: i8,             // this pass's depth, for progress
    time_limit: Option<std::time::Duration>,
    state_limit: Option<usize>,
    check_limits: bool, // from the second pass on, when a limit is set
    nodes: u64,         // this pass's nodes, to space the looks at the clock
    cut_short: bool,    // a limit stopped this pass
    progress: Option<ProgressFn>,
    in_bag: Option<u64>, // inside a search with a tile in the bag, its endgames so far
}

impl WorkBuffer {
    fn new(game_config: &game_config::GameConfig) -> Self {
        let dim = game_config.board_layout().dim();
        let board_area = dim.rows as usize * dim.cols as usize;
        Self {
            t0: std::time::Instant::now(),
            dur_movegen: Default::default(),
            vec_placed_tile: Vec::with_capacity(board_area),
            current_ply_buffer: Default::default(),
            movegen: movegen::KurniaMoveGenerator::new(game_config),
            states: Vec::new(),
            state_finder: Default::default(),
            state_eval: Default::default(),
            plays: Vec::new(),
            play_finder: fash::MyHashMap::default(),
            child_plays: Vec::new(),
            depth_limited: false,
            max_depth: 0,
            time_limit: None,
            state_limit: None,
            check_limits: false,
            nodes: 0,
            cut_short: false,
            progress: None,
            in_bag: None,
        }
    }

    #[inline]
    fn init(&mut self) {
        self.t0 = std::time::Instant::now();
        self.dur_movegen = Default::default();
        // no need to clear temp spaces here
        // put an unused entry in states, because index 0 is special
        self.states.clear();
        self.states.push(State {
            parent: !0,
            placed_tile: PlacedTile {
                tile: !0,
                whose: !0,
                idx: !0,
            },
        });
        self.state_finder.clear();
        self.state_eval.clear();
        self.plays.clear();
        // plays[0] is always Pass
        self.plays.push(movegen::Play::Exchange {
            tiles: [][..].into(),
        });
        self.play_finder.clear();
        self.child_plays.clear();
    }
}

// only for reporting
pub struct FoundPlay<'a> {
    pub equity: equity::Equity,
    pub play: &'a movegen::Play,
}

// the solver counts millipoints in f32, which holds whole ones exactly; an
// average, such as the peg's margin, rounds to the nearest
#[inline(always)]
fn to_equity(v: f32) -> equity::Equity {
    equity::Equity::new(v.round() as i32)
}

// past either limit a solve stops, and answers from the deepest search that
// finished
pub struct Limits {
    pub time: Option<std::time::Duration>,
    pub states: Option<usize>,
}

impl Limits {
    pub const NONE: Self = Self {
        time: None,
        states: None,
    };
}

// a search with a tile in the bag solves many endgames as one solve: its time
// limit is one deadline for them all, on its clock, and its state limit is each
// one's; the endgames take turns holding its one listener
struct BagLimits {
    time: Option<std::time::Duration>,
    states: Option<usize>,
    t0: std::time::Instant,
    dur_movegen: std::cell::Cell<std::time::Duration>,
    endgames: std::cell::Cell<u64>,
    progress: std::cell::Cell<Option<ProgressFn>>,
}

impl BagLimits {
    // its clock starts now
    #[inline(always)]
    fn new(limits: &Limits, progress: Option<ProgressFn>) -> Self {
        Self {
            time: limits.time,
            states: limits.states,
            t0: std::time::Instant::now(),
            dur_movegen: Default::default(),
            endgames: Default::default(),
            progress: std::cell::Cell::new(progress),
        }
    }
}

// a solve's answer: exact, or the value of the deepest search that finished
// before a limit stopped it
pub struct Solved {
    pub value: equity::Equity,
    pub depth: i8,
    pub exact: bool,
}

// how far a solve has got, sent to its listener at every node, and at each
// finished depth with that depth's value; with a tile in the bag, at every node
// alone. The listener keeps the time, on its own clock
pub struct Progress {
    pub endgames: Option<u64>, // with a tile in the bag, the endgames searched so far
    pub depth: i8,
    pub value: Option<equity::Equity>, // the depth's value, once it has finished
    pub movegen: std::time::Duration,
    pub states: usize,
    pub evaluated: usize,
    pub child_plays: usize,
    pub plays: usize,
}

impl Progress {
    // its line, with the time its listener has kept since the solve began
    #[inline(always)]
    pub fn after(&self, elapsed: std::time::Duration) -> ProgressLine<'_> {
        ProgressLine {
            progress: self,
            elapsed,
        }
    }
}

pub struct ProgressLine<'a> {
    progress: &'a Progress,
    elapsed: std::time::Duration,
}

impl std::fmt::Display for ProgressLine<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let p = self.progress;
        if let Some(endgames) = p.endgames {
            write!(f, "{endgames} endgames searched, now ")?;
        }
        match p.value {
            Some(value) => write!(f, "depth {} is {}", p.depth, value)?,
            None => write!(f, "depth {}", p.depth)?,
        }
        write!(
            f,
            ": after {:?} ({:?} on movegen), there are {} states, {} evaluated, {} child_plays, {} plays",
            self.elapsed, p.movegen, p.states, p.evaluated, p.child_plays, p.plays,
        )
    }
}

type ProgressFn = Box<dyn FnMut(&Progress) -> error::Returns<()>>;

// EndgameSolver is the main two-player endgame solver.
// EndgameSolver can only be reused for the same game_config and kwg.
// (Refer to note at WorkBuffer.)
// This is not enforced.
pub struct EndgameSolver<'a, N: kwg::Node, L: kwg::Node> {
    game_config: &'a game_config::GameConfig,
    kwg: &'a kwg::Kwg<N>,
    arm_tables: &'a anagrams::ArmTables,
    klv: Box<klv::Klv<L>>,
    board_tiles: Vec<u8>,
    racks: [Vec<u8>; 2],
    rack_scores: [i32; 2],
    work_buffer: WorkBuffer,
}

impl<'a, N: kwg::Node, L: kwg::Node> EndgameSolver<'a, N, L> {
    pub fn new(
        game_config: &'a game_config::GameConfig,
        kwg: &'a kwg::Kwg<N>,
        arm_tables: &'a anagrams::ArmTables,
    ) -> error::Returns<Self> {
        if game_config.num_players() != 2 {
            return Err("cannot solve non-2-player endgames".into());
        }
        Ok(Self {
            game_config,
            kwg,
            arm_tables,
            klv: Box::new(klv::Klv::<L>::from_bytes_alloc(klv::EMPTY_KLV_BYTES)?),
            board_tiles: Vec::new(),
            racks: [Vec::new(), Vec::new()],
            rack_scores: [0, 0],
            work_buffer: WorkBuffer::new(game_config),
        })
    }

    pub fn init(&mut self, board_tiles: &[u8], racks: [&[u8]; 2]) {
        self.board_tiles.clear();
        self.board_tiles.extend_from_slice(board_tiles);
        self.racks[0].clear();
        self.racks[0].extend_from_slice(racks[0]);
        self.racks[1].clear();
        self.racks[1].extend_from_slice(racks[1]);
        self.rack_scores[0] = self.game_config.alphabet().scaled_rack_score(racks[0]);
        self.rack_scores[1] = self.game_config.alphabet().scaled_rack_score(racks[1]);
        self.work_buffer.init();
    }

    #[inline(always)]
    fn get_new_state_idx(
        &mut self,
        state_idx: u32,
        which_player: u8,
        play_idx: u32,
    ) -> error::Returns<u32> {
        match &self.work_buffer.plays[play_idx as usize] {
            movegen::Play::Exchange { .. } => Ok(state_idx),
            movegen::Play::Place {
                down,
                lane,
                idx,
                word,
                score: _score,
            } => {
                // rebuild the current stack
                {
                    self.work_buffer.vec_placed_tile.clear();
                    let mut state_idx = state_idx;
                    while state_idx != 0 {
                        let state = &self.work_buffer.states[state_idx as usize];
                        self.work_buffer
                            .vec_placed_tile
                            .push(state.placed_tile.clone());
                        state_idx = state.parent;
                    }
                    self.work_buffer.vec_placed_tile.reverse();
                }

                // place the tiles
                let dim = self.game_config.board_layout().dim();
                let strider = dim.lane(*down, *lane);
                for (i, &tile) in (*idx..).zip(word.iter()) {
                    if tile != 0 {
                        self.work_buffer.vec_placed_tile.push(PlacedTile {
                            tile,
                            whose: which_player,
                            idx: strider.at(i) as i16,
                        });
                    }
                }

                debug_assert!(
                    self.work_buffer.vec_placed_tile.len() <= dim.rows as usize * dim.cols as usize,
                    "placed-tile buffer exceeded the board area"
                );

                // normalize the ordering
                self.work_buffer
                    .vec_placed_tile
                    .sort_unstable_by(|a, b| a.tile.cmp(&b.tile).then_with(|| a.idx.cmp(&b.idx)));

                // normalize the tile owner
                {
                    // the blanks (0x81-0xbf) are sorted at end and treated as one group
                    let mut threshold = 0x80u8;
                    let mut freq = [0, 0];
                    for cursor in (0..self.work_buffer.vec_placed_tile.len()).rev() {
                        let new_threshold = self.work_buffer.vec_placed_tile[cursor].tile;
                        if new_threshold < threshold {
                            threshold = new_threshold;
                            let mut p = cursor + 1;
                            for _ in 0..freq[0] {
                                self.work_buffer.vec_placed_tile[p].whose = 0;
                                p += 1;
                            }
                            for _ in 0..freq[1] {
                                self.work_buffer.vec_placed_tile[p].whose = 1;
                                p += 1;
                            }
                            freq[0] = 0;
                            freq[1] = 0;
                        }
                        freq[self.work_buffer.vec_placed_tile[cursor].whose as usize] += 1;
                    }
                    // assign the "whose" of the final leftmost group
                    {
                        let mut p = 0;
                        for _ in 0..freq[0] {
                            self.work_buffer.vec_placed_tile[p].whose = 0;
                            p += 1;
                        }
                        for _ in 0..freq[1] {
                            self.work_buffer.vec_placed_tile[p].whose = 1;
                            p += 1;
                        }
                    }
                }

                // get the new state_idx
                let mut new_state_idx = 0;
                for placed_tile in self.work_buffer.vec_placed_tile.iter() {
                    let new_state = State {
                        parent: new_state_idx,
                        placed_tile: placed_tile.clone(),
                    };
                    let new_new_state_idx = self.work_buffer.states.len() as u32;
                    new_state_idx = *self
                        .work_buffer
                        .state_finder
                        .entry(new_state.clone())
                        .or_insert(new_new_state_idx);
                    if new_state_idx == new_new_state_idx {
                        self.work_buffer.states.push(new_state);
                        if new_new_state_idx == !0 {
                            // this might happen, but only after a very long time
                            return Err("too many states".into());
                        }
                    }
                }

                Ok(new_state_idx)
            }
        }
    }

    #[inline(always)]
    fn both_pass_value(&self, mut state_idx: u32, player_idx: u8) -> f32 {
        let mut rack_scores = [self.rack_scores[0], self.rack_scores[1]];
        let alphabet = self.game_config.alphabet();
        while state_idx != 0 {
            let state = &self.work_buffer.states[state_idx as usize];
            let blanked_tile =
                state.placed_tile.tile & !((state.placed_tile.tile as i8) >> 7) as u8;
            rack_scores[state.placed_tile.whose as usize] -= alphabet.scaled_score(blanked_tile);
            state_idx = state.parent;
        }
        (rack_scores[player_idx as usize ^ 1] - rack_scores[player_idx as usize]) as f32
    }

    #[inline]
    fn run_id_loop(
        &mut self,
        player_idx: u8,
        verbose: bool,
        limits: &Limits,
    ) -> error::Returns<Solved> {
        let mut last_valuation = f32::NAN;
        // a move that does not end the line places tiles and leaves the mover some, or passes;
        // a pass answering a pass repeats the position two plies back and ends it, and so does
        // a play out. a mover holding one tile can pass before and after every tile the other
        // places, so its line can run a ply longer than any a mover holding more can make. a
        // rack with no tiles can only pass, as a rack holding its last tile does short of
        // ending the game, so it counts as one tile.
        let mine = self.racks[player_idx as usize].len().max(1);
        let theirs = self.racks[player_idx as usize ^ 1].len().max(1);
        let longest = (2 * (mine + theirs) + (mine == 1) as usize - 3).min(i8::MAX as usize);
        self.work_buffer.time_limit = limits.time;
        self.work_buffer.state_limit = limits.states;
        for max_depth in 1..=longest as i8 {
            let old_num_state_eval = self.work_buffer.state_eval.len();

            self.work_buffer.depth_limited = false;
            self.work_buffer.max_depth = max_depth;
            // the first pass always finishes, so a solve cut short still has a value
            self.work_buffer.check_limits =
                max_depth > 1 && (limits.time.is_some() || limits.states.is_some());
            self.work_buffer.nodes = 0;
            self.work_buffer.cut_short = false;
            let searched = if max_depth == 1 {
                self.negamax_eval(
                    0,
                    player_idx,
                    max_depth,
                    f32::NEG_INFINITY,
                    f32::INFINITY,
                    false,
                )
            } else {
                self.aspiration_search(player_idx, max_depth, last_valuation)
            };
            let valuation = match searched {
                Ok(valuation) => valuation,
                Err(_) if self.work_buffer.cut_short => {
                    return Ok(Solved {
                        value: to_equity(last_valuation),
                        depth: max_depth - 1,
                        exact: false,
                    });
                }
                Err(e) => return Err(e),
            };
            last_valuation = valuation;
            if self.work_buffer.in_bag.is_none() {
                self.report(Some(to_equity(valuation)))?;
            }
            if verbose {
                self.print_best_line(player_idx)?;
            }

            if self.work_buffer.state_eval.len() == old_num_state_eval
                && !self.work_buffer.depth_limited
            {
                break;
            }
            // a pass that ended past a limit is the last
            if (max_depth as usize) < longest && self.past_a_limit() {
                return Ok(Solved {
                    value: to_equity(valuation),
                    depth: max_depth,
                    exact: false,
                });
            }
        }
        Ok(Solved {
            value: to_equity(last_valuation),
            depth: self.work_buffer.max_depth,
            exact: true,
        })
    }

    #[inline(always)]
    fn past_a_limit(&self) -> bool {
        self.work_buffer
            .time_limit
            .is_some_and(|limit| self.work_buffer.t0.elapsed() >= limit)
            || self
                .work_buffer
                .state_limit
                .is_some_and(|limit| self.work_buffer.states.len() >= limit)
    }

    #[inline]
    fn aspiration_search(
        &mut self,
        player_idx: u8,
        max_depth: i8,
        last_valuation: f32,
    ) -> error::Returns<f32> {
        const ASPIRATION_WINDOW: f32 = (3 * super::equity::SCALE) as f32;
        let lo = last_valuation - ASPIRATION_WINDOW;
        let hi = last_valuation + ASPIRATION_WINDOW;
        let v = self.negamax_eval(0, player_idx, max_depth, lo, hi, false)?;
        if v > lo && v < hi {
            Ok(v)
        } else {
            self.negamax_eval(
                0,
                player_idx,
                max_depth,
                f32::NEG_INFINITY,
                f32::INFINITY,
                false,
            )
        }
    }

    // send every later solve's progress to the given listener, at every node
    // and at each finished depth; until then it goes nowhere
    #[inline(always)]
    pub fn report_progress(
        &mut self,
        progress: impl FnMut(&Progress) -> error::Returns<()> + 'static,
    ) {
        self.work_buffer.progress = Some(Box::new(progress));
    }

    // a solve of its own starts its clock
    #[inline(always)]
    fn start_clock(&mut self) {
        self.work_buffer.t0 = std::time::Instant::now();
        self.work_buffer.dur_movegen = Default::default();
    }

    // solve within the limits, also printing each finished depth's best line
    #[inline(always)]
    pub fn evaluate(&mut self, player_idx: u8, limits: &Limits) -> error::Returns<Solved> {
        self.start_clock();
        self.run_id_loop(player_idx, true, limits)
    }

    #[inline(always)]
    pub fn solve(&mut self, player_idx: u8) -> error::Returns<equity::Equity> {
        Ok(self.solve_within(player_idx, &Limits::NONE)?.value)
    }

    #[inline(always)]
    pub fn solve_within(&mut self, player_idx: u8, limits: &Limits) -> error::Returns<Solved> {
        self.start_clock();
        self.run_id_loop(player_idx, false, limits)
    }

    // based on https://en.wikipedia.org/wiki/Negamax
    fn negamax_eval(
        &mut self,
        state_idx: u32,
        player_idx: u8,
        depth: i8,
        mut alpha: f32,
        mut beta: f32,
        just_passed: bool,
    ) -> error::Returns<f32> {
        // movegen not done for depth == 0, so no state_eval.
        if depth == 0 {
            self.work_buffer.depth_limited = true;

            return Ok(self.both_pass_value(state_idx, player_idx));
        }

        // the listener, if any, hears every node
        if self.work_buffer.progress.is_some() {
            self.report(None)?;
        }

        // the states are looked at every node, the clock every LIMIT_CADENCE nodes
        if self.work_buffer.check_limits {
            self.work_buffer.nodes += 1;
            if self
                .work_buffer
                .state_limit
                .is_some_and(|limit| self.work_buffer.states.len() >= limit)
                || (self.work_buffer.nodes.is_multiple_of(LIMIT_CADENCE)
                    && self
                        .work_buffer
                        .time_limit
                        .is_some_and(|limit| self.work_buffer.t0.elapsed() >= limit))
            {
                self.work_buffer.cut_short = true;
                return Err("a limit stopped the search".into());
            }
        }

        // return and/or trim range
        let alpha_orig = alpha;
        let state_eval = if let Some(state_eval) = self.work_buffer.state_eval.get(&state_idx) {
            let state_side_eval = &state_eval.best_move[player_idx as usize];
            if state_side_eval.depth >= depth {
                // a table-served best_move is always a PLACE, so its value cannot depend on the
                // consecutive-pass count.
                debug_assert!(
                    state_side_eval.play_idx != 0,
                    "endgame TT served a pass value; pass-count invariant broken",
                );
                match state_side_eval.equity_type {
                    StateSideEvalEquityType::Exact => {
                        return Ok(state_side_eval.equity);
                    }
                    StateSideEvalEquityType::LowerBound => {
                        if state_side_eval.equity > alpha {
                            alpha = state_side_eval.equity;
                        }
                    }
                    StateSideEvalEquityType::UpperBound => {
                        if state_side_eval.equity < beta {
                            beta = state_side_eval.equity;
                        }
                    }
                }
                if alpha >= beta {
                    return Ok(state_side_eval.equity);
                }
            }
            state_eval
        } else {
            let current_ply_buffer = &mut self.work_buffer.current_ply_buffer;
            current_ply_buffer.board_tiles.clear();
            current_ply_buffer
                .board_tiles
                .extend_from_slice(&self.board_tiles);
            current_ply_buffer.racks[0].clear();
            current_ply_buffer.racks[0].extend_from_slice(&self.racks[0]);
            current_ply_buffer.racks[1].clear();
            current_ply_buffer.racks[1].extend_from_slice(&self.racks[1]);

            // revivify the state
            {
                let mut state_idx = state_idx;
                while state_idx != 0 {
                    let state = &self.work_buffer.states[state_idx as usize];
                    current_ply_buffer.board_tiles[state.placed_tile.idx as usize] =
                        state.placed_tile.tile;
                    let rack = &mut current_ply_buffer.racks[state.placed_tile.whose as usize];
                    let blanked_tile =
                        state.placed_tile.tile & !((state.placed_tile.tile as i8) >> 7) as u8;
                    let tombstone_idx = rack.iter().rposition(|&t| t == blanked_tile).unwrap();
                    rack[tombstone_idx] = 0x80;
                    state_idx = state.parent;
                }
                current_ply_buffer.racks[0].retain(|&t| t != 0x80);
                current_ply_buffer.racks[1].retain(|&t| t != 0x80);
            }
            let alphabet = self.game_config.alphabet();
            let rack_scores = [
                alphabet.scaled_rack_score(&current_ply_buffer.racks[0]),
                alphabet.scaled_rack_score(&current_ply_buffer.racks[1]),
            ];

            /*
            writeln!(std::io::stdout(),
                "position {} has racks {:?} and board",
                state_idx, current_ply_buffer.racks
            )?;
            super::display::print_board(
                self.game_config.alphabet(),
                self.game_config.board_layout(),
                &current_ply_buffer.board_tiles,
            );
            */

            // generate moves
            let board_snapshot = movegen::BoardSnapshot {
                board_tiles: &current_ply_buffer.board_tiles,
                game_config: self.game_config,
                kwg: self.kwg,
                anagrams: self.arm_tables.anagrams(),
                rack_lengths: self.arm_tables.rack_lengths(),
                klv: &self.klv,
            };
            let mut state_eval = StateEval {
                best_place_move: [StateSideEval::new(), StateSideEval::new()],
                best_move: [StateSideEval::new(), StateSideEval::new()],
                child_play_idxs: [self.work_buffer.child_plays.len(), 0, 0],
            };
            for which_player in 0..2 {
                let t1 = std::time::Instant::now();
                const DO_HASTY: bool = false;
                if !DO_HASTY || which_player == 0 {
                    self.work_buffer.movegen.gen_moves_raw_all_unsorted(
                        &board_snapshot,
                        &current_ply_buffer.racks[which_player],
                        0, // TODO: use game_state to track this
                        movegen::PassPolicy::AsACandidate,
                    );
                } else {
                    // simulate hasty blunders
                    self.work_buffer
                        .movegen
                        .gen_moves_unfiltered(&movegen::GenMovesParams {
                            board_snapshot: &board_snapshot,
                            rack: &current_ply_buffer.racks[which_player],
                            max_gen: 1,
                            num_exchanges_by_this_player: 0, // TODO: use game_state to track this
                            pass_policy: movegen::PassPolicy::OnlyWhenForced,
                            dynamic_leaves: None,
                        });
                }
                self.work_buffer.dur_movegen += t1.elapsed();
                for candidate in &self.work_buffer.movegen.plays {
                    match &candidate.play {
                        movegen::Play::Exchange { .. } => {
                            // no need to store pass explicitly
                        }
                        movegen::Play::Place { word, score, .. } => {
                            let new_new_play_idx = self.work_buffer.plays.len() as u32;
                            let new_play_idx = *self
                                .work_buffer
                                .play_finder
                                .entry(candidate.play.clone())
                                .or_insert(new_new_play_idx);
                            if new_play_idx == new_new_play_idx {
                                self.work_buffer.plays.push(candidate.play.clone());
                                if new_new_play_idx == !0 {
                                    // this should not happen
                                    return Err("too many plays".into());
                                }
                            }
                            self.work_buffer.child_plays.push(
                                if word.iter().filter(|&&t| t != 0).count()
                                    == current_ply_buffer.racks[which_player].len()
                                {
                                    // playing out
                                    ChildPlay {
                                        new_state_idx: 0,
                                        play_idx: new_play_idx,
                                        valuation: (score + 2 * rack_scores[which_player ^ 1])
                                            as f32,
                                    }
                                } else {
                                    ChildPlay {
                                        new_state_idx: !0, // filled in later
                                        play_idx: new_play_idx,
                                        valuation: *score as f32,
                                    }
                                },
                            );
                        }
                    }
                }
                state_eval.child_play_idxs[which_player + 1] = self.work_buffer.child_plays.len();
            }

            self.work_buffer
                .state_eval
                .entry(state_idx)
                .or_insert(state_eval)
        };

        let low_idx = state_eval.child_play_idxs[player_idx as usize];
        let high_idx = state_eval.child_play_idxs[player_idx as usize + 1];

        const LAZY_MOVE_ORDER_PREFIX: Option<usize> = None;

        let mut sorted_end = high_idx;
        match LAZY_MOVE_ORDER_PREFIX {
            Some(k) if high_idx - low_idx > k => {
                let slice = &mut self.work_buffer.child_plays[low_idx..high_idx];

                slice.select_nth_unstable_by(k, |a, b| b.valuation.total_cmp(&a.valuation));
                slice[..k].sort_unstable_by(|a, b| b.valuation.total_cmp(&a.valuation));
                sorted_end = low_idx + k;
            }
            _ => {
                self.work_buffer.child_plays[low_idx..high_idx]
                    .sort_unstable_by(|a, b| b.valuation.total_cmp(&a.valuation));
            }
        }

        // perform actual negamax
        let mut best_idx = !0;
        let mut best_valuation = f32::NEG_INFINITY;
        for child_play_idx in low_idx..high_idx {
            if sorted_end < high_idx && child_play_idx == sorted_end {
                self.work_buffer.child_plays[sorted_end..high_idx]
                    .sort_unstable_by(|a, b| b.valuation.total_cmp(&a.valuation));
                sorted_end = high_idx;
            }
            match &self.work_buffer.plays
                [self.work_buffer.child_plays[child_play_idx].play_idx as usize]
            {
                movegen::Play::Exchange { .. } => {
                    unreachable!();
                }
                movegen::Play::Place { score, .. } => {
                    let child_valuation =
                        if self.work_buffer.child_plays[child_play_idx].new_state_idx == 0 {
                            // playing out, valuation is already correct
                            self.work_buffer.child_plays[child_play_idx].valuation
                        } else {
                            let score = *score as f32;
                            if self.work_buffer.child_plays[child_play_idx].new_state_idx == !0 {
                                // construct the new state
                                self.work_buffer.child_plays[child_play_idx].new_state_idx = self
                                    .get_new_state_idx(
                                    state_idx,
                                    player_idx,
                                    self.work_buffer.child_plays[child_play_idx].play_idx,
                                )?;
                            }
                            // the math goes like this:
                            // child negamax returns v.
                            // this parent negamax wants (score - v),
                            // where alpha <= (score - v) <= beta.
                            // so (score - beta) <= v <= (score - alpha).
                            // since child_alpha <= v <= child_beta,
                            // we set child_alpha = (score - beta)
                            // and child_beta = (score - alpha).
                            score
                                - self.negamax_eval(
                                    self.work_buffer.child_plays[child_play_idx].new_state_idx,
                                    player_idx ^ 1,
                                    depth - 1,
                                    score - beta,
                                    score - alpha,
                                    false,
                                )?
                        };
                    self.work_buffer.child_plays[child_play_idx].valuation = child_valuation;
                    // only place moves affect alpha/beta
                    if child_valuation > best_valuation {
                        best_valuation = child_valuation;
                        best_idx = child_play_idx;
                        if child_valuation > alpha {
                            alpha = child_valuation;
                        }
                        if child_valuation >= beta {
                            break;
                        }
                    }
                }
            };
        }

        // expand no-pass side first
        let pass_valuation = if just_passed {
            self.both_pass_value(state_idx, player_idx)
        } else {
            -self.negamax_eval(state_idx, player_idx ^ 1, depth - 1, -beta, -alpha, true)?
        };

        // fill in best_place_move
        let state_eval = self.work_buffer.state_eval.get_mut(&state_idx).unwrap();
        if best_idx == !0 {
            // no valid place moves exist, must pass
            state_eval.best_place_move[player_idx as usize] = StateSideEval {
                equity: pass_valuation,
                play_idx: 0,
                new_state_idx: state_idx,
                equity_type: StateSideEvalEquityType::Exact,
                depth: i8::MIN, // cannot cache pass_valuation
            };
        } else {
            let best_play = &self.work_buffer.child_plays[best_idx];
            state_eval.best_place_move[player_idx as usize] = StateSideEval {
                equity: best_valuation,
                play_idx: best_play.play_idx,
                new_state_idx: best_play.new_state_idx,
                equity_type: if best_valuation <= alpha_orig {
                    StateSideEvalEquityType::UpperBound
                } else if best_valuation >= beta {
                    StateSideEvalEquityType::LowerBound
                } else {
                    StateSideEvalEquityType::Exact
                },
                depth,
            };
        }

        // best_move is the better of best_place_move or pass_valuation.
        if pass_valuation > best_valuation {
            state_eval.best_move[player_idx as usize] = StateSideEval {
                equity: pass_valuation,
                play_idx: 0,
                new_state_idx: state_idx,
                equity_type: StateSideEvalEquityType::Exact,
                depth: i8::MIN, // cannot cache pass_valuation
            };
            best_valuation = pass_valuation;
        } else {
            state_eval.best_move[player_idx as usize] =
                state_eval.best_place_move[player_idx as usize].clone();
        }

        Ok(best_valuation)
    }

    // must have been precomputed
    #[inline(always)]
    pub fn append_solution<'b, F: FnMut(FoundPlay<'b>)>(
        &'a self,
        mut state_idx: u32,
        mut player_idx: u8,
        mut out: F,
    ) where
        'a: 'b,
    {
        while let Some(ans) = self.work_buffer.state_eval.get(&state_idx) {
            let mut ans1 = &ans.best_move[player_idx as usize];
            let play = &self.work_buffer.plays[ans1.play_idx as usize];
            out(FoundPlay {
                equity: to_equity(ans1.equity),
                play,
            });
            if let movegen::Play::Exchange { .. } = play {
                player_idx ^= 1;
                ans1 = &ans.best_move[player_idx as usize];
                if ans1.play_idx == !0 {
                    // not yet evaluated
                    break;
                }
                let play = &self.work_buffer.plays[ans1.play_idx as usize];
                out(FoundPlay {
                    equity: to_equity(ans1.equity),
                    play,
                });
                if let movegen::Play::Exchange { .. } = play {
                    // both passed, done
                    break;
                }
            }
            state_idx = ans1.new_state_idx;
            if state_idx == 0 || state_idx == !0 {
                break;
            }
            player_idx ^= 1;
        }
    }

    #[inline]
    pub fn collect_pv(&'a self, player_idx: u8, out: &mut Vec<(equity::Equity, movegen::Play)>) {
        out.clear();
        self.append_solution(0, player_idx, |found| {
            out.push((found.equity, found.play.clone()));
        });
    }

    #[inline]
    pub fn print_best_line(&mut self, player_idx: u8) -> std::io::Result<()> {
        let mut current_ply_buffer = std::mem::take(&mut self.work_buffer.current_ply_buffer);
        let board_tiles = &mut current_ply_buffer.board_tiles;
        board_tiles.clone_from(&self.board_tiles);
        let racks = &mut current_ply_buffer.racks;
        racks.clone_from(&self.racks);
        let mut leftovers = [None, None];
        let mut i = 0usize;
        let mut written = Ok(());
        self.append_solution(0, player_idx, |ply| {
            let player_turn_idx = (player_idx as usize + i) & 1;
            let rack = &mut racks[player_turn_idx];
            leftovers[player_turn_idx ^ 1] = None;
            leftovers[player_turn_idx] = Some(equity::Equity::new(
                ply.equity.raw()
                    - match ply.play {
                        movegen::Play::Exchange { .. } => 0,
                        movegen::Play::Place { score, .. } => *score,
                    },
            ));
            if written.is_ok() {
                written = writeln!(
                    std::io::stdout(),
                    "{}: p{}: {}{:width$} {} {}",
                    i,
                    player_turn_idx,
                    self.game_config.alphabet().fmt_rack(rack),
                    "",
                    ply.equity,
                    ply.play.fmt(&movegen::BoardSnapshot {
                        board_tiles,
                        game_config: self.game_config,
                        kwg: self.kwg,
                        anagrams: self.arm_tables.anagrams(),
                        rack_lengths: self.arm_tables.rack_lengths(),
                        klv: &self.klv,
                    }),
                    width = self.game_config.rack_size() as usize - rack.len(),
                );
            }
            match &ply.play {
                movegen::Play::Exchange { .. } => {}
                movegen::Play::Place {
                    down,
                    lane,
                    idx,
                    word,
                    score: _,
                } => {
                    let strider = self.game_config.board_layout().dim().lane(*down, *lane);

                    // place the tiles
                    for (i, &tile) in (*idx..).zip(word.iter()) {
                        if tile != 0 {
                            board_tiles[strider.at(i)] = tile;
                            let blanked_tile = tile & !((tile as i8) >> 7) as u8;
                            let tombstone_idx =
                                rack.iter().rposition(|&t| t == blanked_tile).unwrap();
                            rack[tombstone_idx] = 0x80;
                        }
                    }
                    rack.retain(|&t| t != 0x80);
                }
            }
            i += 1;
        });
        written?;
        for _ in 0..2 {
            let player_turn_idx = (player_idx as usize + i) & 1;
            let rack = &racks[player_turn_idx];
            let leftover = leftovers[player_turn_idx];
            write!(
                std::io::stdout(),
                "e: p{}: {}",
                player_turn_idx,
                self.game_config.alphabet().fmt_rack(rack)
            )?;
            if let Some(leftover) = leftover {
                write!(
                    std::io::stdout(),
                    "{:width$} {}",
                    "",
                    leftover,
                    width = self.game_config.rack_size() as usize - rack.len(),
                )?;
            }
            writeln!(std::io::stdout())?;
            i += 1;
        }
        display::print_board(
            self.game_config.alphabet(),
            self.game_config.board_layout(),
            board_tiles,
        )?;
        self.work_buffer.current_ply_buffer = current_ply_buffer;
        Ok(())
    }

    // send the progress, if anything is listening
    #[inline]
    fn report(&mut self, value: Option<equity::Equity>) -> error::Returns<()> {
        let work_buffer = &mut self.work_buffer;
        match work_buffer.progress.as_mut() {
            Some(progress_fn) => progress_fn(&Progress {
                endgames: work_buffer.in_bag,
                depth: work_buffer.max_depth,
                value,
                movegen: work_buffer.dur_movegen,
                states: work_buffer.states.len(),
                evaluated: work_buffer.state_eval.len(),
                child_plays: work_buffer.child_plays.len(),
                plays: work_buffer.plays.len(),
            }),
            None => Ok(()),
        }
    }

    #[inline]
    pub fn solve_one_in_bag(
        &mut self,
        player_idx: u8,
        bag_tile: u8,
    ) -> error::Returns<equity::Equity> {
        let limits = BagLimits::new(&Limits::NONE, self.work_buffer.progress.take());
        let mut exact = true;
        let value = self.one_in_bag(player_idx, bag_tile, &limits, &mut exact);
        self.work_buffer.progress = limits.progress.take();
        value
    }

    // an endgame inside a search with a tile in the bag, on that search's clock
    // and within its limits, telling its listener; one cut short makes the
    // search's answer inexact
    #[inline(always)]
    fn solve_in_bag(
        &mut self,
        player_idx: u8,
        limits: &BagLimits,
        exact: &mut bool,
    ) -> error::Returns<f32> {
        let work_buffer = &mut self.work_buffer;
        work_buffer.t0 = limits.t0;
        work_buffer.dur_movegen = limits.dur_movegen.get();
        work_buffer.in_bag = Some(limits.endgames.get());
        work_buffer.progress = limits.progress.take();
        let solved = self.run_id_loop(
            player_idx,
            false,
            &Limits {
                time: limits.time,
                states: limits.states,
            },
        );
        let work_buffer = &mut self.work_buffer;
        limits.dur_movegen.set(work_buffer.dur_movegen);
        limits.endgames.set(limits.endgames.get() + 1);
        limits.progress.set(work_buffer.progress.take());
        work_buffer.in_bag = None;
        let solved = solved?;
        *exact &= solved.exact;
        Ok(solved.value.raw() as f32)
    }

    // the one-in-bag value within the limits; an endgame inside it that a limit
    // cut short clears exact
    fn one_in_bag(
        &mut self,
        player_idx: u8,
        bag_tile: u8,
        limits: &BagLimits,
        exact: &mut bool,
    ) -> error::Returns<equity::Equity> {
        let gc = self.game_config;
        let kwg = self.kwg;
        let arm_tables = self.arm_tables;

        let mut sub = EndgameSolver::<N, L>::new(gc, kwg, arm_tables)?;

        let mut mg = movegen::KurniaMoveGenerator::new(gc);

        let klv = klv::Klv::<L>::from_bytes_alloc(klv::EMPTY_KLV_BYTES)?;

        let board = self.board_tiles.clone();
        let racks = [self.racks[0].clone(), self.racks[1].clone()];

        let mut search = OneInBag {
            gc,
            kwg,
            arm_tables,
            klv: &klv,
            sub: &mut sub,
            mg: &mut mg,
            memo: fash::MyHashMap::default(),
            limits,
            exact: true,
        };
        let value = to_equity(if one_in_bag_exchange_solvable(gc) {
            Self::one_in_bag_minimax_ex(&mut search, &board, &racks, player_idx, bag_tile, 0, 0)?
        } else {
            Self::one_in_bag_minimax(&mut search, &board, &racks, player_idx, bag_tile, false)?
        });
        *exact &= search.exact;
        Ok(value)
    }

    fn one_in_bag_minimax(
        search: &mut OneInBag<'_, 'a, N, L>,
        board: &[u8],
        racks: &[Vec<u8>; 2],
        mover: u8,
        bag_tile: u8,
        just_passed: bool,
    ) -> error::Returns<f32> {
        let alphabet = search.gc.alphabet();
        let opp = mover ^ 1;
        let snapshot = movegen::BoardSnapshot {
            board_tiles: board,
            game_config: search.gc,
            kwg: search.kwg,
            anagrams: search.arm_tables.anagrams(),
            rack_lengths: search.arm_tables.rack_lengths(),
            klv: search.klv,
        };
        search.mg.gen_moves_raw_all_unsorted(
            &snapshot,
            &racks[mover as usize],
            0,
            movegen::PassPolicy::AsACandidate,
        );

        let drawn = bag_tile & !((bag_tile as i8) >> 7) as u8;

        let mut best = f32::NEG_INFINITY;
        for vm in &search.mg.plays {
            if let movegen::Play::Place {
                down,
                lane,
                idx,
                word,
                score,
            } = &vm.play
            {
                let mut nb = board.to_vec();
                let mut nr = racks.clone();
                {
                    let rack = &mut nr[mover as usize];
                    let strider = search.gc.board_layout().dim().lane(*down, *lane);
                    for (i, &tile) in (*idx..).zip(word.iter()) {
                        if tile != 0 {
                            nb[strider.at(i)] = tile;
                            let blanked = tile & !((tile as i8) >> 7) as u8;
                            let p = rack.iter().rposition(|&t| t == blanked).unwrap();
                            rack[p] = 0x80;
                        }
                    }
                    rack.retain(|&t| t != 0x80);
                    rack.push(drawn);
                }

                search.sub.init(&nb, [&nr[0], &nr[1]]);
                let v = search
                    .sub
                    .solve_in_bag(opp, search.limits, &mut search.exact)?;

                let val = *score as f32 - v;
                if val > best {
                    best = val;
                }
            }
        }

        let pass_val = if just_passed {
            (alphabet.scaled_rack_score(&racks[opp as usize])
                - alphabet.scaled_rack_score(&racks[mover as usize])) as f32
        } else {
            -Self::one_in_bag_minimax(search, board, racks, opp, bag_tile, true)?
        };
        if pass_val > best {
            best = pass_val;
        }
        Ok(best)
    }

    fn one_in_bag_minimax_ex(
        search: &mut OneInBag<'_, 'a, N, L>,
        board: &[u8],
        racks: &[Vec<u8>; 2],
        mover: u8,
        bag_tile: u8,
        passes: u8,
        zeros: u8,
    ) -> error::Returns<f32> {
        let mut rack0 = racks[0].clone();
        let mut rack1 = racks[1].clone();
        rack0.sort_unstable();
        rack1.sort_unstable();
        let key = BagExchangeKey {
            mover,
            bag_tile,
            passes,
            zeros,
            rack0,
            rack1,
        };
        if let Some(&v) = search.memo.get(&key) {
            return Ok(v);
        }

        let alphabet = search.gc.alphabet();
        let opp = mover ^ 1;
        let snapshot = movegen::BoardSnapshot {
            board_tiles: board,
            game_config: search.gc,
            kwg: search.kwg,
            anagrams: search.arm_tables.anagrams(),
            rack_lengths: search.arm_tables.rack_lengths(),
            klv: search.klv,
        };
        search.mg.gen_moves_raw_all_unsorted(
            &snapshot,
            &racks[mover as usize],
            0,
            movegen::PassPolicy::AsACandidate,
        );

        let mut places: Vec<movegen::Play> = Vec::new();
        for vm in &search.mg.plays {
            if let movegen::Play::Place { .. } = &vm.play {
                places.push(vm.play.clone());
            }
        }

        let drawn = bag_tile & !((bag_tile as i8) >> 7) as u8;

        let mut best = f32::NEG_INFINITY;

        for play in &places {
            if let movegen::Play::Place {
                down,
                lane,
                idx,
                word,
                score,
            } = play
            {
                let mut nb = board.to_vec();
                let mut nr = racks.clone();
                {
                    let rack = &mut nr[mover as usize];
                    let strider = search.gc.board_layout().dim().lane(*down, *lane);
                    for (i, &tile) in (*idx..).zip(word.iter()) {
                        if tile != 0 {
                            nb[strider.at(i)] = tile;
                            let blanked = tile & !((tile as i8) >> 7) as u8;
                            let p = rack.iter().rposition(|&t| t == blanked).unwrap();
                            rack[p] = 0x80;
                        }
                    }
                    rack.retain(|&t| t != 0x80);
                    rack.push(drawn);
                }
                search.sub.init(&nb, [&nr[0], &nr[1]]);
                let v = search
                    .sub
                    .solve_in_bag(opp, search.limits, &mut search.exact)?;
                let val = *score as f32 - v;
                if val > best {
                    best = val;
                }
            }
        }

        let pass_passes = passes + 1;
        let pass_zeros = zeros + 1;
        let pass_val = if scoreless_turns_end(search.gc, pass_passes, pass_zeros) {
            (alphabet.scaled_rack_score(&racks[opp as usize])
                - alphabet.scaled_rack_score(&racks[mover as usize])) as f32
        } else {
            -Self::one_in_bag_minimax_ex(
                search,
                board,
                racks,
                opp,
                bag_tile,
                pass_passes,
                pass_zeros,
            )?
        };
        if pass_val > best {
            best = pass_val;
        }

        let exch_zeros = zeros + 1;
        let mut seen: u64 = 0; // bitset over rack tile values 0..=63
        for i in 0..racks[mover as usize].len() {
            let r = racks[mover as usize][i];
            if r == drawn {
                continue;
            }
            let bit = 1u64 << (r & 0x3f);
            if seen & bit != 0 {
                continue;
            }
            seen |= bit;

            let mut nr = racks.clone();
            {
                let rack = &mut nr[mover as usize];
                let p = rack.iter().position(|&t| t == r).unwrap();
                rack[p] = drawn;
            }
            let exch_val = if scoreless_turns_end(search.gc, 0, exch_zeros) {
                (alphabet.scaled_rack_score(&nr[opp as usize])
                    - alphabet.scaled_rack_score(&nr[mover as usize])) as f32
            } else {
                -Self::one_in_bag_minimax_ex(search, board, &nr, opp, r, 0, exch_zeros)?
            };
            if exch_val > best {
                best = exch_val;
            }
        }

        search.memo.insert(key, best);
        Ok(best)
    }

    #[inline]
    pub fn solve_peg_one_in_bag(
        &mut self,
        mover: u8,
        board: &[u8],
        mover_rack: &[u8],
        unseen_tally: &[u8],
        score_diff: equity::Equity,
        limits: &Limits,
    ) -> error::Returns<Result<PegResult, PegUnsupported>> {
        let gc = self.game_config;
        if one_in_bag_exchange_legal(gc) && !one_in_bag_exchange_solvable(gc) {
            return Ok(Err(PegUnsupported::ExchangeWithoutForcedEnd));
        }
        let score_diff = score_diff.raw() as f32;
        let limits = BagLimits::new(limits, self.work_buffer.progress.take());

        let result = if one_in_bag_exchange_legal(gc) {
            self.peg_clairvoyant_aggregate(
                mover,
                board,
                mover_rack,
                unseen_tally,
                score_diff,
                &limits,
            )
        } else {
            let unseen: Vec<(u8, u32)> = unseen_tally
                .iter()
                .enumerate()
                .filter(|&(_, &c)| c != 0)
                .map(|(t, &c)| (t as u8, c as u32))
                .collect();
            self.peg_committed_no_exchange(mover, board, mover_rack, &unseen, score_diff, &limits)
        };
        self.work_buffer.progress = limits.progress.take();
        Ok(Ok(result?))
    }

    #[inline]
    fn peg_clairvoyant_aggregate(
        &mut self,
        mover: u8,
        board: &[u8],
        mover_rack: &[u8],
        unseen_tally: &[u8],
        score_diff: f32,
        limits: &BagLimits,
    ) -> error::Returns<PegResult> {
        let mut hypotheses: Vec<(u8, u32, equity::Equity)> = Vec::new();
        let mut exact = true;

        let mut opp_rack: Vec<u8> = Vec::new();
        for (t, &count) in unseen_tally.iter().enumerate() {
            if count == 0 {
                continue;
            }
            let bag_tile = t as u8;

            opp_rack.clear();
            for (u, &c) in unseen_tally.iter().enumerate() {
                let take = if u == t { c - 1 } else { c } as usize;
                opp_rack.extend(std::iter::repeat_n(u as u8, take));
            }
            let mut racks: [&[u8]; 2] = [&[], &[]];
            racks[mover as usize] = mover_rack;
            racks[(mover ^ 1) as usize] = &opp_rack;
            self.init(board, racks);
            let v = self.one_in_bag(mover, bag_tile, limits, &mut exact)?.raw() as f32 + score_diff;
            hypotheses.push((bag_tile, count as u32, to_equity(v)));
        }
        let weighted: Vec<(u32, equity::Equity)> =
            hypotheses.iter().map(|&(_, w, v)| (w, v)).collect();
        let (win_pct, expected_margin) = peg_aggregate(&weighted);
        Ok(PegResult {
            win_pct,
            expected_margin,
            hypotheses,
            best_move: None,
            committed: false,
            exact,
        })
    }

    #[inline]
    fn peg_committed_no_exchange(
        &self,
        mover: u8,
        board: &[u8],
        mover_rack: &[u8],
        unseen: &[(u8, u32)],
        score_diff: f32,
        limits: &BagLimits,
    ) -> error::Returns<PegResult> {
        let gc = self.game_config;
        let kwg = self.kwg;
        let arm_tables = self.arm_tables;
        let klv = klv::Klv::<L>::from_bytes_alloc(klv::EMPTY_KLV_BYTES)?;
        let opp = mover ^ 1;
        let mut exact = true;
        let total = unseen.iter().map(|&(_, w)| w).sum::<u32>() as f32;

        let mut mg = movegen::KurniaMoveGenerator::new(gc);
        mg.gen_moves_raw_all_unsorted(
            &movegen::BoardSnapshot {
                board_tiles: board,
                game_config: gc,
                kwg,
                anagrams: arm_tables.anagrams(),
                rack_lengths: arm_tables.rack_lengths(),
                klv: &klv,
            },
            mover_rack,
            0,
            movegen::PassPolicy::AsACandidate,
        );
        let mut candidates: Vec<PegMove> = Vec::with_capacity(mg.plays.len() + 1);
        candidates.push(PegMove::Pass);
        for vm in &mg.plays {
            if matches!(vm.play, movegen::Play::Place { .. }) {
                candidates.push(PegMove::Place(vm.play.clone()));
            }
        }

        let mut sub = EndgameSolver::<N, L>::new(gc, kwg, arm_tables)?;

        let mut best_win = f32::NEG_INFINITY;
        let mut best_marg = f32::NEG_INFINITY;
        let mut best_move = PegMove::Pass;
        let mut best_hyps: Vec<(u8, u32, equity::Equity)> = Vec::new();

        for cand in &candidates {
            let mut win = 0.0f32;
            let mut marg = 0.0f32;
            let mut hyps: Vec<(u8, u32, equity::Equity)> = Vec::with_capacity(unseen.len());
            for &(t, w) in unseen {
                let drawn = t & !((t as i8) >> 7) as u8;

                let mut opp_rack: Vec<u8> = Vec::new();
                for &(u, c) in unseen {
                    let take = if u == t { c - 1 } else { c };
                    for _ in 0..take {
                        opp_rack.push(u);
                    }
                }
                let v = match cand {
                    PegMove::Place(movegen::Play::Place {
                        down,
                        lane,
                        idx,
                        word,
                        score,
                    }) => {
                        let mut nb = board.to_vec();
                        let mut mr = mover_rack.to_vec();
                        let strider = gc.board_layout().dim().lane(*down, *lane);
                        for (i, &tile) in (*idx..).zip(word.iter()) {
                            if tile != 0 {
                                nb[strider.at(i)] = tile;
                                let b = tile & !((tile as i8) >> 7) as u8;
                                let p = mr.iter().rposition(|&x| x == b).unwrap();
                                mr[p] = 0x80;
                            }
                        }
                        mr.retain(|&x| x != 0x80);
                        mr.push(drawn);
                        let mut r2: [Vec<u8>; 2] = [Vec::new(), Vec::new()];
                        r2[mover as usize] = mr;
                        r2[opp as usize] = opp_rack;
                        sub.init(&nb, [&r2[0], &r2[1]]);
                        *score as f32 - sub.solve_in_bag(opp, limits, &mut exact)?
                    }
                    PegMove::Pass => {
                        // PEG2-7: with two or more in the bag, draw every tile from the SAME end -- pop,
                        // the back -- and store a scenario reversed to match. Do NOT reuse Bag::replenish
                        // for the enumeration: it pops for even players and shifts for odd ones, which on
                        // a reversed bag hands p1 the leftover instead of the next popped tile.
                        let mut r2: [Vec<u8>; 2] = [Vec::new(), Vec::new()];
                        r2[mover as usize] = mover_rack.to_vec();
                        r2[opp as usize] = opp_rack;
                        let mut search = OneInBag {
                            gc,
                            kwg,
                            arm_tables,
                            klv: &klv,
                            sub: &mut sub,
                            mg: &mut mg,
                            memo: fash::MyHashMap::default(),
                            limits,
                            exact: true,
                        };
                        let v = -Self::one_in_bag_minimax(&mut search, board, &r2, opp, t, true)?;
                        exact &= search.exact;
                        v
                    }
                    PegMove::Place(_) => {
                        unreachable!("Place candidate always holds a Play::Place")
                    }
                };
                let vv = v + score_diff;
                marg += w as f32 * vv;
                win += w as f32
                    * if vv > 0.0 {
                        1.0
                    } else if vv == 0.0 {
                        0.5
                    } else {
                        0.0
                    };
                hyps.push((t, w, to_equity(vv)));
            }
            win /= total;
            marg /= total;

            if win > best_win || (win == best_win && marg > best_marg) {
                best_win = win;
                best_marg = marg;
                best_move = cand.clone();
                best_hyps = hyps;
            }
        }

        Ok(PegResult {
            win_pct: best_win,
            expected_margin: to_equity(best_marg),
            hypotheses: best_hyps,
            best_move: Some(best_move),
            committed: true,
            exact,
        })
    }
}

#[derive(Clone)]
pub enum PegMove {
    Pass,
    Place(movegen::Play),
}

pub struct PegResult {
    pub win_pct: f32,
    pub expected_margin: equity::Equity,
    pub hypotheses: Vec<(u8, u32, equity::Equity)>,
    pub best_move: Option<PegMove>,
    pub committed: bool,
    pub exact: bool, // false when a limit cut an endgame inside it short
}

#[derive(Clone, Debug)]
pub enum PegUnsupported {
    ExchangeWithoutForcedEnd,
}

impl std::fmt::Display for PegUnsupported {
    #[inline]
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PegUnsupported::ExchangeWithoutForcedEnd => f.write_str(
                "exchange is legal with one tile in the bag but scoreless turns \
                 never force the game to end, so this position is not solvable",
            ),
        }
    }
}

// the one-in-bag search's fixed parts, carried down its recursion
struct OneInBag<'s, 'a, N: kwg::Node, L: kwg::Node> {
    gc: &'s game_config::GameConfig,
    kwg: &'s kwg::Kwg<N>,
    arm_tables: &'s anagrams::ArmTables,
    klv: &'s klv::Klv<L>,
    sub: &'s mut EndgameSolver<'a, N, L>,
    mg: &'s mut movegen::KurniaMoveGenerator,
    memo: fash::MyHashMap<BagExchangeKey, f32>,
    limits: &'s BagLimits,
    exact: bool, // cleared by an endgame a limit cut short
}

#[derive(Eq, Hash, PartialEq)]
struct BagExchangeKey {
    mover: u8,
    bag_tile: u8,
    passes: u8,
    zeros: u8,
    rack0: Vec<u8>,
    rack1: Vec<u8>,
}

#[inline(always)]
fn one_in_bag_exchange_legal(gc: &game_config::GameConfig) -> bool {
    gc.exchange_tile_limit() <= 1
}

#[inline(always)]
fn one_in_bag_exchange_solvable(gc: &game_config::GameConfig) -> bool {
    one_in_bag_exchange_legal(gc) && gc.exchanges_are_zeros() && gc.num_zeros_to_end() != 0
}

#[inline]
fn scoreless_turns_end(gc: &game_config::GameConfig, passes: u8, zeros: u8) -> bool {
    let npte = gc.num_passes_to_end();
    let nzte = gc.num_zeros_to_end();
    (npte != 0 && passes >= npte) || (nzte != 0 && zeros >= nzte)
}

#[inline]
pub fn peg_aggregate(hypotheses: &[(u32, equity::Equity)]) -> (f32, equity::Equity) {
    let mut total = 0.0f32;
    let mut win_sum = 0.0f32;
    let mut margin_sum = 0.0f32;
    for &(weight, value) in hypotheses {
        let w = weight as f32;
        let win_score = match value.cmp(&equity::Equity::ZERO) {
            std::cmp::Ordering::Greater => 1.0,
            std::cmp::Ordering::Equal => 0.5,
            std::cmp::Ordering::Less => 0.0,
        };
        total += w;
        win_sum += win_score * w;
        margin_sum += value.raw() as f32 * w;
    }
    if total == 0.0 {
        (0.0, equity::Equity::ZERO)
    } else {
        (win_sum / total, to_equity(margin_sum / total))
    }
}

#[cfg(test)]
mod tests {
    use super::{EndgameSolver, PegMove};
    use crate::{alphabet, anagrams, bites, build, equity, game_config, klv, kwg, movegen};

    #[inline]
    fn tiny_word_list() -> Vec<bites::Bites> {
        let gc = game_config::make_english_game_config();
        let reader = alphabet::AlphabetReader::new_for_words(gc.alphabet());
        let word_strs = [
            "AA", "AB", "AH", "AT", "BA", "HA", "TA", "AAH", "ABA", "BAA", "BAT", "TAB", "TAT",
            "HAT", "THAT", "AAHS", "BATH", "HAH", "AAL",
        ];
        let mut words = Vec::<bites::Bites>::new();
        let mut buf = Vec::new();
        for w in word_strs {
            if reader.set_word(w, &mut buf).is_ok() {
                words.push(buf[..].into());
            }
        }
        words.sort_unstable();
        words.dedup();
        words
    }

    #[inline]
    fn tiny_kwg_bytes() -> bites::Bites {
        build::build(
            build::BuildContent::Gaddawg,
            build::BuildLayout::Wolges,
            build::BuildOrder::Sorted,
            &tiny_word_list(),
        )
        .unwrap()
    }

    // each arm, flagged true for the lean one
    #[inline]
    fn both_arms(
        kwg: &kwg::Kwg<kwg::Node22>,
        gc: &game_config::GameConfig,
    ) -> [(bool, anagrams::ArmTables); 2] {
        [false, true].map(|lean| (lean, anagrams::ArmTables::build(kwg, gc, lean).unwrap()))
    }

    #[inline]
    fn apply_place(
        gc: &game_config::GameConfig,
        board: &mut [u8],
        rack: &mut Vec<u8>,
        down: bool,
        lane: i8,
        idx: i8,
        word: &[u8],
    ) {
        let strider = gc.board_layout().dim().lane(down, lane);
        for (i, &tile) in (idx..).zip(word.iter()) {
            if tile != 0 {
                board[strider.at(i)] = tile;
                let blanked = tile & !((tile as i8) >> 7) as u8;
                let p = rack.iter().rposition(|&t| t == blanked).unwrap();
                rack[p] = 0x80;
            }
        }
        rack.retain(|&t| t != 0x80);
    }

    // a reference solve's fixed parts, carried down its recursion
    struct Reference<'s, N: kwg::Node, L: kwg::Node> {
        gc: &'s game_config::GameConfig,
        kwg: &'s kwg::Kwg<N>,
        klv: &'s klv::Klv<L>,
        mg: &'s mut movegen::KurniaMoveGenerator,
    }

    fn reference<N: kwg::Node, L: kwg::Node>(
        fixed: &mut Reference<'_, N, L>,
        board: &[u8],
        racks: &[Vec<u8>; 2],
        mover: usize,
        just_passed: bool,
    ) -> f32 {
        let alphabet = fixed.gc.alphabet();
        let snapshot = movegen::BoardSnapshot {
            board_tiles: board,
            game_config: fixed.gc,
            kwg: fixed.kwg,
            anagrams: None,
            rack_lengths: None,
            klv: fixed.klv,
        };
        fixed.mg.gen_moves_raw_all_unsorted(
            &snapshot,
            &racks[mover],
            0,
            movegen::PassPolicy::AsACandidate,
        );

        let mut places: Vec<movegen::Play> = Vec::new();
        for vm in &fixed.mg.plays {
            if let movegen::Play::Place { .. } = &vm.play {
                places.push(vm.play.clone());
            }
        }

        let opp = mover ^ 1;
        let mut best = f32::NEG_INFINITY;
        for play in &places {
            if let movegen::Play::Place {
                down,
                lane,
                idx,
                word,
                score,
            } = play
            {
                let placed = word.iter().filter(|&&t| t != 0).count();
                let val = if placed == racks[mover].len() {
                    (*score + 2 * alphabet.scaled_rack_score(&racks[opp])) as f32
                } else {
                    let mut nb = board.to_vec();
                    let mut nr = racks.clone();
                    apply_place(fixed.gc, &mut nb, &mut nr[mover], *down, *lane, *idx, word);
                    *score as f32 - reference(fixed, &nb, &nr, opp, false)
                };
                if val > best {
                    best = val;
                }
            }
        }

        let pass_val = if just_passed {
            (alphabet.scaled_rack_score(&racks[opp]) - alphabet.scaled_rack_score(&racks[mover]))
                as f32
        } else {
            -reference(fixed, board, racks, opp, true)
        };
        if pass_val > best {
            best = pass_val;
        }
        best
    }

    fn reference_one_in_bag<N: kwg::Node, L: kwg::Node>(
        fixed: &mut Reference<'_, N, L>,
        board: &[u8],
        racks: &[Vec<u8>; 2],
        mover: usize,
        bag_tile: u8,
        just_passed: bool,
    ) -> f32 {
        let alphabet = fixed.gc.alphabet();
        let snapshot = movegen::BoardSnapshot {
            board_tiles: board,
            game_config: fixed.gc,
            kwg: fixed.kwg,
            anagrams: None,
            rack_lengths: None,
            klv: fixed.klv,
        };
        fixed.mg.gen_moves_raw_all_unsorted(
            &snapshot,
            &racks[mover],
            0,
            movegen::PassPolicy::AsACandidate,
        );

        let mut places: Vec<movegen::Play> = Vec::new();
        for vm in &fixed.mg.plays {
            if let movegen::Play::Place { .. } = &vm.play {
                places.push(vm.play.clone());
            }
        }

        let drawn = bag_tile & !((bag_tile as i8) >> 7) as u8;

        let opp = mover ^ 1;
        let mut best = f32::NEG_INFINITY;
        for play in &places {
            if let movegen::Play::Place {
                down,
                lane,
                idx,
                word,
                score,
            } = play
            {
                let mut nb = board.to_vec();
                let mut nr = racks.clone();
                apply_place(fixed.gc, &mut nb, &mut nr[mover], *down, *lane, *idx, word);
                nr[mover].push(drawn);

                let val = *score as f32 - reference(fixed, &nb, &nr, opp, false);
                if val > best {
                    best = val;
                }
            }
        }

        let pass_val = if just_passed {
            (alphabet.scaled_rack_score(&racks[opp]) - alphabet.scaled_rack_score(&racks[mover]))
                as f32
        } else {
            -reference_one_in_bag(fixed, board, racks, opp, bag_tile, true)
        };
        if pass_val > best {
            best = pass_val;
        }
        best
    }

    fn reference_one_in_bag_ex<N: kwg::Node, L: kwg::Node>(
        fixed: &mut Reference<'_, N, L>,
        board: &[u8],
        racks: &[Vec<u8>; 2],
        mover: usize,
        bag_tile: u8,
        passes: u8,
        zeros: u8,
    ) -> f32 {
        let alphabet = fixed.gc.alphabet();
        let snapshot = movegen::BoardSnapshot {
            board_tiles: board,
            game_config: fixed.gc,
            kwg: fixed.kwg,
            anagrams: None,
            rack_lengths: None,
            klv: fixed.klv,
        };
        fixed.mg.gen_moves_raw_all_unsorted(
            &snapshot,
            &racks[mover],
            0,
            movegen::PassPolicy::AsACandidate,
        );
        let mut places: Vec<movegen::Play> = Vec::new();
        for vm in &fixed.mg.plays {
            if let movegen::Play::Place { .. } = &vm.play {
                places.push(vm.play.clone());
            }
        }

        let drawn = bag_tile & !((bag_tile as i8) >> 7) as u8;
        let opp = mover ^ 1;
        let mut best = f32::NEG_INFINITY;
        for play in &places {
            if let movegen::Play::Place {
                down,
                lane,
                idx,
                word,
                score,
            } = play
            {
                let mut nb = board.to_vec();
                let mut nr = racks.clone();
                apply_place(fixed.gc, &mut nb, &mut nr[mover], *down, *lane, *idx, word);
                nr[mover].push(drawn);
                let val = *score as f32 - reference(fixed, &nb, &nr, opp, false);
                if val > best {
                    best = val;
                }
            }
        }

        let pass_passes = passes + 1;
        let pass_zeros = zeros + 1;
        let pass_val = if super::scoreless_turns_end(fixed.gc, pass_passes, pass_zeros) {
            (alphabet.scaled_rack_score(&racks[opp]) - alphabet.scaled_rack_score(&racks[mover]))
                as f32
        } else {
            -reference_one_in_bag_ex(fixed, board, racks, opp, bag_tile, pass_passes, pass_zeros)
        };
        if pass_val > best {
            best = pass_val;
        }

        let exch_zeros = zeros + 1;
        let mut seen: u64 = 0;
        for i in 0..racks[mover].len() {
            let r = racks[mover][i];
            if r == drawn {
                continue;
            }
            let bit = 1u64 << (r & 0x3f);
            if seen & bit != 0 {
                continue;
            }
            seen |= bit;
            let mut nr = racks.clone();
            {
                let rack = &mut nr[mover];
                let p = rack.iter().position(|&t| t == r).unwrap();
                rack[p] = drawn;
            }
            let exch_val = if super::scoreless_turns_end(fixed.gc, 0, exch_zeros) {
                (alphabet.scaled_rack_score(&nr[opp]) - alphabet.scaled_rack_score(&nr[mover]))
                    as f32
            } else {
                -reference_one_in_bag_ex(fixed, board, &nr, opp, r, 0, exch_zeros)
            };
            if exch_val > best {
                best = exch_val;
            }
        }

        best
    }

    struct Position {
        board: Vec<u8>,
        racks: [Vec<u8>; 2],
    }

    #[inline(always)]
    fn empty_board() -> Vec<u8> {
        vec![0u8; 15 * 15]
    }

    #[inline]
    fn put_word(board: &mut [u8], row: i8, col0: i8, word: &[u8]) {
        for (k, &t) in word.iter().enumerate() {
            board[(row as usize) * 15 + col0 as usize + k] = t;
        }
    }

    #[inline]
    fn describe(gc: &game_config::GameConfig, pos: &Position) -> String {
        let a = gc.alphabet();
        let mut s = String::new();
        s.push_str("board:");
        for (i, &t) in pos.board.iter().enumerate() {
            if t != 0 {
                let r = i / 15;
                let c = i % 15;
                s.push_str(&format!(" r{r}c{c}={}", a.of_board(t).unwrap_or("?")));
            }
        }
        s.push_str(&format!(
            " rack0=[{}] rack1=[{}]",
            a.fmt_rack(&pos.racks[0]),
            a.fmt_rack(&pos.racks[1]),
        ));
        s.push_str(&format!(
            " bytes board={:?} r0={:?} r1={:?}",
            pos.board
                .iter()
                .enumerate()
                .filter(|&(_, &t)| t != 0)
                .map(|(i, &t)| (i, t))
                .collect::<Vec<_>>(),
            pos.racks[0],
            pos.racks[1]
        ));
        s
    }

    #[inline]
    fn embedded_positions() -> Vec<(String, Position)> {
        let mut out = Vec::new();

        out.push((
            "both-pass-only (BB vs HT)".to_string(),
            Position {
                board: empty_board(),
                racks: [vec![2, 2], vec![8, 20]],
            },
        ));

        out.push((
            "both-pass-only (BB vs BH)".to_string(),
            Position {
                board: empty_board(),
                racks: [vec![2, 2], vec![2, 8]],
            },
        ));

        {
            let mut b = empty_board();
            put_word(&mut b, 7, 6, &[1, 20]); // A T at r7 c6,c7
            out.push((
                "one-side-pass (board AT; p0=[B] p1=[B])".to_string(),
                Position {
                    board: b,
                    racks: [vec![2], vec![2]],
                },
            ));
        }

        out.push((
            "playout race (AB vs AT)".to_string(),
            Position {
                board: empty_board(),
                racks: [vec![1, 2], vec![1, 20]],
            },
        ));

        {
            let mut b = empty_board();
            put_word(&mut b, 7, 6, &[1, 20]); // AT
            out.push((
                "playout with board (p0=[B,A] p1=[H,A])".to_string(),
                Position {
                    board: b,
                    racks: [vec![2, 1], vec![8, 1]],
                },
            ));
        }

        {
            let mut b = empty_board();
            put_word(&mut b, 7, 6, &[1, 1, 8]); // A A H (AAH)
            out.push((
                "3-tile (board AAH; p0=[B,A,T] p1=[H,A,T])".to_string(),
                Position {
                    board: b,
                    racks: [vec![2, 1, 20], vec![8, 1, 20]],
                },
            ));
        }

        {
            let mut b = empty_board();
            put_word(&mut b, 7, 6, &[1, 20]); // AT
            out.push((
                "finished game, the mover's rack empty (board AT; p0=[] p1=[H])".to_string(),
                Position {
                    board: b,
                    racks: [vec![], vec![8]],
                },
            ));
        }

        {
            let mut b = empty_board();
            put_word(&mut b, 7, 6, &[1, 20]); // AT
            out.push((
                "finished game, the other's rack empty (board AT; p0=[H,B] p1=[])".to_string(),
                Position {
                    board: b,
                    racks: [vec![8, 2], vec![]],
                },
            ));
        }

        out
    }

    // https://doi.org/10.1145/2714064.2660195
    // https://prng.di.unimi.it/splitmix64.c
    #[inline]
    fn splitmix64(s: &mut u64) -> u64 {
        *s = s.wrapping_add(0x9e3779b97f4a7c15);
        let mut z = *s;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58476d1ce4e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d049bb133111eb);
        z ^ (z >> 31)
    }

    #[inline]
    fn random_positions(n: usize) -> Vec<(String, Position)> {
        let letters = [1u8, 2, 20, 8]; // A B T H
        let words: Vec<Vec<u8>> = vec![
            vec![1, 20],     // AT
            vec![1, 1],      // AA
            vec![8, 1],      // HA
            vec![1, 1, 8],   // AAH
            vec![2, 1, 20],  // BAT
            vec![20, 1, 2],  // TAB
            vec![20, 1, 20], // TAT
        ];
        let mut seed = 0u64;
        let mut out = Vec::new();
        let mut made = 0usize;
        while made < n {
            let mut b = empty_board();

            let w = &words[(splitmix64(&mut seed) as usize) % words.len()];
            let row = (splitmix64(&mut seed) % 15) as i8;
            let col0 = (splitmix64(&mut seed) % (15 - w.len() as u64)) as i8;
            put_word(&mut b, row, col0, w);

            let mut racks: [Vec<u8>; 2] = [Vec::new(), Vec::new()];
            for rack in racks.iter_mut() {
                let size = if splitmix64(&mut seed).is_multiple_of(5) {
                    3
                } else {
                    2
                };
                for _ in 0..size {
                    let roll = splitmix64(&mut seed);
                    let t = if roll.is_multiple_of(11) {
                        0u8 // blank
                    } else {
                        letters[(roll as usize / 11) % letters.len()]
                    };
                    rack.push(t);
                }
                rack.sort_unstable();
            }
            if racks[0].is_empty() || racks[1].is_empty() {
                continue;
            }
            out.push((format!("rand#{made}"), Position { board: b, racks }));
            made += 1;
        }
        out
    }

    #[inline]
    fn all_positions() -> Vec<(String, Position)> {
        let mut v = embedded_positions();
        v.extend(random_positions(200));
        v
    }

    #[test]
    #[inline]
    fn differential_reference_vs_solve() {
        let gc = game_config::make_english_game_config();
        let kwg_bytes = tiny_kwg_bytes();
        let kwg = kwg::Kwg::<kwg::Node22>::from_bytes_alloc(&kwg_bytes).unwrap();
        let klv = klv::Klv::<kwg::Node22>::from_bytes_alloc(klv::EMPTY_KLV_BYTES).unwrap();
        let mut mg = movegen::KurniaMoveGenerator::new(&gc);
        let arms = both_arms(&kwg, &gc);

        let mut disagreements = 0usize;
        let mut total = 0usize;
        for (name, pos) in all_positions() {
            let refv = reference(
                &mut Reference {
                    gc: &gc,
                    kwg: &kwg,
                    klv: &klv,
                    mg: &mut mg,
                },
                &pos.board,
                &pos.racks,
                0,
                false,
            );
            for (lean, arm_tables) in &arms {
                total += 1;
                let mut egs =
                    EndgameSolver::<kwg::Node22, kwg::Node22>::new(&gc, &kwg, arm_tables).unwrap();
                egs.init(&pos.board, [&pos.racks[0][..], &pos.racks[1][..]]);
                let solvev = egs.solve(0).unwrap();
                if solvev != super::to_equity(refv) {
                    disagreements += 1;
                    let ratio = if refv != 0.0 {
                        solvev.raw() as f32 / refv
                    } else {
                        f32::NAN
                    };
                    println!(
                        "DISAGREE [{name}] lean={lean}: ref={refv} solve={} ratio={ratio}\n   {}",
                        solvev.raw(),
                        describe(&gc, &pos)
                    );
                }
            }
        }
        println!("differential: {disagreements} disagreements out of {total} solves");
        assert_eq!(disagreements, 0, "solve() disagreed with the reference");
    }

    #[test]
    #[inline]
    fn a_solve_a_limit_stops_answers_from_a_finished_depth() {
        let gc = game_config::make_english_game_config();
        let kwg_bytes = tiny_kwg_bytes();
        let kwg = kwg::Kwg::<kwg::Node22>::from_bytes_alloc(&kwg_bytes).unwrap();
        let arm_tables = anagrams::ArmTables::build(&kwg, &gc, false).unwrap();

        let mut cut = 0usize;
        for (name, pos) in all_positions() {
            // every depth an unlimited solve finishes: its value and its states
            let finished = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
            let mut egs =
                EndgameSolver::<kwg::Node22, kwg::Node22>::new(&gc, &kwg, &arm_tables).unwrap();
            egs.init(&pos.board, [&pos.racks[0][..], &pos.racks[1][..]]);
            let seen = finished.clone();
            egs.report_progress(move |p: &super::Progress| {
                if let Some(value) = p.value {
                    seen.borrow_mut().push((p.depth, value, p.states));
                }
                Ok(())
            });
            let full = egs.solve_within(0, &super::Limits::NONE).unwrap();
            let finished = finished.borrow();
            assert!(full.exact, "[{name}] an unlimited solve is exact");
            assert_eq!(
                finished.last().map(|&(d, v, _)| (d, v)),
                Some((full.depth, full.value)),
                "[{name}] the last finished depth is the answer"
            );
            if finished.len() < 2 {
                continue;
            }

            // no time at all: the first depth always finishes, and is the answer
            let mut egs =
                EndgameSolver::<kwg::Node22, kwg::Node22>::new(&gc, &kwg, &arm_tables).unwrap();
            egs.init(&pos.board, [&pos.racks[0][..], &pos.racks[1][..]]);
            let limits = super::Limits {
                time: Some(std::time::Duration::ZERO),
                states: None,
            };
            let solved = egs.solve_within(0, &limits).unwrap();
            assert!(
                !solved.exact,
                "[{name}] a zero time limit cuts the solve short"
            );
            assert_eq!(
                (solved.depth, solved.value),
                (finished[0].0, finished[0].1),
                "[{name}] a zero time limit answers the first depth"
            );

            // one state past the first depth's: the search stops within a later
            // pass, or after it, and answers a depth that finished
            let mut egs =
                EndgameSolver::<kwg::Node22, kwg::Node22>::new(&gc, &kwg, &arm_tables).unwrap();
            egs.init(&pos.board, [&pos.racks[0][..], &pos.racks[1][..]]);
            let limits = super::Limits {
                time: None,
                states: Some(finished[0].2 + 1),
            };
            let solved = egs.solve_within(0, &limits).unwrap();
            if !solved.exact {
                cut += 1;
                assert!(
                    finished
                        .iter()
                        .any(|&(d, v, _)| (d, v) == (solved.depth, solved.value)),
                    "[{name}] a state limit answers depth {} with {}, which no pass finished",
                    solved.depth,
                    solved.value
                );
            } else {
                assert_eq!(
                    (solved.depth, solved.value),
                    (full.depth, full.value),
                    "[{name}] a solve the state limit never stopped is the unlimited one"
                );
            }
        }
        assert!(cut > 0, "no state limit cut a solve short");
    }

    #[test]
    #[inline]
    fn a_search_with_a_tile_in_the_bag_keeps_one_listener() {
        let gc = game_config::make_english_game_config();
        let kwg_bytes = tiny_kwg_bytes();
        let kwg = kwg::Kwg::<kwg::Node22>::from_bytes_alloc(&kwg_bytes).unwrap();
        let arm_tables = anagrams::ArmTables::build(&kwg, &gc, false).unwrap();

        // every endgame of the search tells the one listener it was given, so
        // its clock is the search's: at every node, with the endgames before
        // it, and never a finished depth of its own
        let heard = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
        let seen = heard.clone();
        let limits = super::BagLimits::new(
            &super::Limits::NONE,
            Some(Box::new(move |p: &super::Progress| {
                seen.borrow_mut().push((p.endgames, p.value));
                Ok(())
            })),
        );
        let positions = all_positions();
        let mut egs =
            EndgameSolver::<kwg::Node22, kwg::Node22>::new(&gc, &kwg, &arm_tables).unwrap();
        let mut exact = true;
        for (_, pos) in &positions {
            egs.init(&pos.board, [&pos.racks[0][..], &pos.racks[1][..]]);
            egs.solve_in_bag(0, &limits, &mut exact).unwrap();
        }
        assert!(exact, "a search with no limits is exact");
        assert_eq!(limits.endgames.get(), positions.len() as u64);
        assert!(limits.progress.take().is_some(), "the listener comes back");
        let heard = heard.borrow();
        assert!(heard.iter().all(|&(_, value)| value.is_none()));
        let endgames = heard.iter().map(|&(n, _)| n).collect::<Vec<_>>();
        assert!(endgames.windows(2).all(|w| w[0] <= w[1]));
        for n in 0..positions.len() as u64 {
            assert!(endgames.contains(&Some(n)), "endgame {n} told nothing");
        }
    }

    #[test]
    #[inline]
    fn peg_aggregate_weights_each_outcome_by_how_many_tiles_give_it() {
        let e = equity::Equity::new;
        let (win_pct, expected) = super::peg_aggregate(&[(2, e(3000)), (1, e(0)), (1, e(-5000))]);
        assert_eq!(win_pct, 0.625);
        assert_eq!(expected, e(250));

        let (win_pct, expected) = super::peg_aggregate(&[(1, e(1)), (1, e(0)), (1, e(-1))]);
        assert_eq!(win_pct, 0.5);
        assert_eq!(expected, e(0));

        assert_eq!(super::peg_aggregate(&[]), (0.0, e(0)));
    }

    #[test]
    #[inline]
    fn a_solver_for_three_players_is_an_error() {
        let gc = game_config::Options {
            preset: "english",
            tiles: None,
            board: None,
            rack_size: None,
            jumbled: false,
            players: Some(3),
            bingo_bonus: None,
            zeros_to_end: None,
            passes_to_end: None,
            exchange_limit: None,
            exchanges: None,
        }
        .make_game_config(&|_| Err("no files here".into()))
        .unwrap();
        let kwg_bytes = tiny_kwg_bytes();
        let kwg = kwg::Kwg::<kwg::Node22>::from_bytes_alloc(&kwg_bytes).unwrap();
        let arm_tables = anagrams::ArmTables::build(&kwg, &gc, false).unwrap();
        assert!(EndgameSolver::<kwg::Node22, kwg::Node22>::new(&gc, &kwg, &arm_tables).is_err());
    }

    #[test]
    #[inline]
    fn peg_one_in_bag_hand_checkable() {
        let gc = game_config::make_english_game_config();
        let kwg_bytes = tiny_kwg_bytes();
        let kwg = kwg::Kwg::<kwg::Node22>::from_bytes_alloc(&kwg_bytes).unwrap();

        let board = empty_board();
        let mover_rack = [2u8]; // B
        let mut unseen_tally = vec![0u8; gc.alphabet().len() as usize];
        unseen_tally[8] = 1; // H
        unseen_tally[20] = 1; // T

        let arm_tables = anagrams::ArmTables::build(&kwg, &gc, false).unwrap();
        let mut egs =
            EndgameSolver::<kwg::Node22, kwg::Node22>::new(&gc, &kwg, &arm_tables).unwrap();
        let result = egs
            .solve_peg_one_in_bag(
                0,
                &board,
                &mover_rack,
                &unseen_tally,
                equity::Equity::ZERO,
                &super::Limits::NONE,
            )
            .unwrap()
            .expect("english one-in-bag is always solvable");

        let e = equity::Equity::new;
        assert_eq!(result.hypotheses, vec![(8, 1, e(-2000)), (20, 1, e(1000))]);
        assert_eq!(result.win_pct, 0.5);
        assert_eq!(result.expected_margin, e(-500));
    }

    #[test]
    #[inline]
    fn peg_one_in_bag_score_diff_shifts_the_outcome() {
        let gc = game_config::make_english_game_config();
        let kwg_bytes = tiny_kwg_bytes();
        let kwg = kwg::Kwg::<kwg::Node22>::from_bytes_alloc(&kwg_bytes).unwrap();

        let board = empty_board();
        let mover_rack = [2u8]; // B
        let mut unseen_tally = vec![0u8; gc.alphabet().len() as usize];
        unseen_tally[8] = 1; // H
        unseen_tally[20] = 1; // T

        let arm_tables = anagrams::ArmTables::build(&kwg, &gc, false).unwrap();
        let mut egs =
            EndgameSolver::<kwg::Node22, kwg::Node22>::new(&gc, &kwg, &arm_tables).unwrap();
        let e = equity::Equity::new;
        let result = egs
            .solve_peg_one_in_bag(
                0,
                &board,
                &mover_rack,
                &unseen_tally,
                e(3000),
                &super::Limits::NONE,
            )
            .unwrap()
            .expect("english one-in-bag is always solvable");

        assert_eq!(result.hypotheses, vec![(8, 1, e(1000)), (20, 1, e(4000))]);
        assert_eq!(result.win_pct, 1.0);
        assert_eq!(result.expected_margin, e(2500));
    }

    #[test]
    #[inline]
    fn a_peg_solve_says_when_a_limit_cut_an_endgame_inside_it() {
        let kwg_bytes = tiny_kwg_bytes();
        let kwg = kwg::Kwg::<kwg::Node22>::from_bytes_alloc(&kwg_bytes).unwrap();
        let no_time = super::Limits {
            time: Some(std::time::Duration::ZERO),
            states: None,
        };
        let one_state = super::Limits {
            time: None,
            states: Some(1),
        };
        let tiles = [1u8, 2, 8, 20]; // A B H T
        let mut board = empty_board();
        put_word(&mut board, 7, 6, &[1, 20]);
        let mut cut = 0usize;
        // the committed model, and the clairvoyant one where exchanges are legal
        for gc in [
            game_config::make_english_game_config(),
            game_config::make_exchange_test_game_config(),
        ] {
            let arm_tables = anagrams::ArmTables::build(&kwg, &gc, false).unwrap();
            let mut egs =
                EndgameSolver::<kwg::Node22, kwg::Node22>::new(&gc, &kwg, &arm_tables).unwrap();
            let mut unseen_tally = vec![0u8; gc.alphabet().len() as usize];
            unseen_tally[8] = 1; // H
            unseen_tally[20] = 1; // T
            for &x in &tiles {
                for &y in &tiles {
                    let mover_rack = [x, y];
                    let mut solve = |limits: &super::Limits| {
                        egs.solve_peg_one_in_bag(
                            0,
                            &board,
                            &mover_rack,
                            &unseen_tally,
                            equity::Equity::ZERO,
                            limits,
                        )
                        .unwrap()
                        .expect("both configs solve one in the bag")
                    };
                    let full = solve(&super::Limits::NONE);
                    assert!(full.exact, "an unlimited solve is exact");
                    for limits in [&no_time, &one_state] {
                        let limited = solve(limits);
                        assert_eq!(limited.hypotheses.len(), full.hypotheses.len());
                        if limited.exact {
                            assert_eq!(limited.hypotheses, full.hypotheses);
                        } else {
                            cut += 1;
                        }
                    }
                }
            }
        }
        assert!(cut > 0, "the limits cut no endgame short");
    }

    #[test]
    #[inline]
    fn a_principal_variation_plays_out_to_the_value_it_claims() {
        let gc = game_config::make_english_game_config();
        let kwg_bytes = tiny_kwg_bytes();
        let kwg = kwg::Kwg::<kwg::Node22>::from_bytes_alloc(&kwg_bytes).unwrap();

        let mut ok = 0usize;
        let mut bad = 0usize;
        let mut pv: Vec<(equity::Equity, movegen::Play)> = Vec::new();
        let positions = all_positions();
        for (lean, arm_tables) in &both_arms(&kwg, &gc) {
            for (name, pos) in &positions {
                let mut egs =
                    EndgameSolver::<kwg::Node22, kwg::Node22>::new(&gc, &kwg, arm_tables).unwrap();
                egs.init(&pos.board, [&pos.racks[0][..], &pos.racks[1][..]]);
                let v = egs.solve(0).unwrap();
                egs.collect_pv(0, &mut pv);

                let a = gc.alphabet();
                let mut board = pos.board.clone();
                let mut racks = pos.racks.clone();
                let mut mover = 0usize;
                let mut sign = 1.0f32;
                let mut realized = 0.0f32;
                let mut prev_pass = false;
                let mut ended = false;
                for (_eq, play) in &pv {
                    match play {
                        movegen::Play::Place {
                            down,
                            lane,
                            idx,
                            word,
                            score,
                        } => {
                            realized += sign * (*score as f32);
                            apply_place(
                                &gc,
                                &mut board,
                                &mut racks[mover],
                                *down,
                                *lane,
                                *idx,
                                word,
                            );
                            if racks[mover].is_empty() {
                                realized +=
                                    sign * (2 * a.scaled_rack_score(&racks[mover ^ 1])) as f32;
                                ended = true;
                            }
                            prev_pass = false;
                        }
                        movegen::Play::Exchange { .. } => {
                            if prev_pass {
                                realized += sign
                                    * (a.scaled_rack_score(&racks[mover ^ 1])
                                        - a.scaled_rack_score(&racks[mover]))
                                        as f32;
                                ended = true;
                            }
                            prev_pass = true;
                        }
                    }
                    mover ^= 1;
                    sign = -sign;
                    if ended {
                        break;
                    }
                }
                if super::to_equity(realized) == v {
                    ok += 1;
                } else {
                    bad += 1;
                    println!(
                        "PV-REPLAY MISMATCH [{name}] lean={lean}: solve={} realized={realized} pvlen={}",
                        v.raw(),
                        pv.len()
                    );
                }
            }
        }
        println!("pv-playout: {ok} ok, {bad} mismatches");
        assert_eq!(bad, 0, "PV replay did not reproduce solve()'s value");
    }

    #[test]
    #[inline]
    fn every_principal_variation_is_legal_and_its_value_bounded() {
        let gc = game_config::make_english_game_config();
        let kwg_bytes = tiny_kwg_bytes();
        let kwg = kwg::Kwg::<kwg::Node22>::from_bytes_alloc(&kwg_bytes).unwrap();
        let klv = klv::Klv::<kwg::Node22>::from_bytes_alloc(klv::EMPTY_KLV_BYTES).unwrap();
        let mut mg = movegen::KurniaMoveGenerator::new(&gc);

        let mut illegal = 0usize;
        let mut oob = 0usize;
        let mut pv: Vec<(equity::Equity, movegen::Play)> = Vec::new();
        let positions = all_positions();
        for (lean, arm_tables) in &both_arms(&kwg, &gc) {
            for (name, pos) in &positions {
                let mut egs =
                    EndgameSolver::<kwg::Node22, kwg::Node22>::new(&gc, &kwg, arm_tables).unwrap();
                egs.init(&pos.board, [&pos.racks[0][..], &pos.racks[1][..]]);
                let v = egs.solve(0).unwrap();
                egs.collect_pv(0, &mut pv);

                let a = gc.alphabet();
                let board_pts: i32 = pos.board.iter().map(|&t| a.scaled_score(t)).sum();
                let rack_pts =
                    a.scaled_rack_score(&pos.racks[0]) + a.scaled_rack_score(&pos.racks[1]);
                let cap = 4 * (board_pts + rack_pts) + 10000;
                if v.raw().abs() > cap {
                    oob += 1;
                    println!("VALUE OOB [{name}] lean={lean}: v={} cap={cap}", v.raw());
                }

                let mut board = pos.board.clone();
                let mut racks = pos.racks.clone();
                let mut mover = 0usize;
                let mut prev_pass = false;
                let mut ended = false;
                for (_eq, play) in &pv {
                    match play {
                        movegen::Play::Place {
                            down,
                            lane,
                            idx,
                            word,
                            ..
                        } => {
                            let snapshot = movegen::BoardSnapshot {
                                board_tiles: &board,
                                game_config: &gc,
                                kwg: &kwg,
                                anagrams: None,
                                rack_lengths: None,
                                klv: &klv,
                            };
                            mg.gen_moves_raw_all_unsorted(
                                &snapshot,
                                &racks[mover],
                                0,
                                movegen::PassPolicy::AsACandidate,
                            );
                            let found = mg.plays.iter().any(|vm| {
                                if let movegen::Play::Place {
                                    down: d2,
                                    lane: l2,
                                    idx: i2,
                                    word: w2,
                                    ..
                                } = &vm.play
                                {
                                    d2 == down && l2 == lane && i2 == idx && w2[..] == word[..]
                                } else {
                                    false
                                }
                            });
                            if !found {
                                illegal += 1;
                                println!("ILLEGAL PV PLAY [{name}] lean={lean} mover={mover}");
                            }
                            apply_place(
                                &gc,
                                &mut board,
                                &mut racks[mover],
                                *down,
                                *lane,
                                *idx,
                                word,
                            );
                            if racks[mover].is_empty() {
                                ended = true;
                            }
                            prev_pass = false;
                        }
                        movegen::Play::Exchange { .. } => {
                            if prev_pass {
                                ended = true;
                            }
                            prev_pass = true;
                        }
                    }
                    mover ^= 1;
                    if ended {
                        break;
                    }
                }
            }
        }
        println!("properties: {illegal} illegal PV plays, {oob} out-of-bounds values");
        assert_eq!(illegal, 0, "a PV play was not legal");
        assert_eq!(oob, 0, "a solve() value exceeded the scaled bound");
    }

    #[test]
    #[inline]
    fn differential_one_in_bag() {
        let gc = game_config::make_english_game_config();
        let kwg_bytes = tiny_kwg_bytes();
        let kwg = kwg::Kwg::<kwg::Node22>::from_bytes_alloc(&kwg_bytes).unwrap();
        let klv = klv::Klv::<kwg::Node22>::from_bytes_alloc(klv::EMPTY_KLV_BYTES).unwrap();
        let mut mg = movegen::KurniaMoveGenerator::new(&gc);

        let bag_tiles: [u8; 4] = [1, 8, 20, 0];
        let arms = both_arms(&kwg, &gc);

        let mut disagreements = 0usize;
        let mut total = 0usize;

        let mut positions = embedded_positions();
        positions.extend(random_positions(12));
        for (name, pos) in positions {
            for &t in &bag_tiles {
                for mover in 0u8..2 {
                    let refv = reference_one_in_bag(
                        &mut Reference {
                            gc: &gc,
                            kwg: &kwg,
                            klv: &klv,
                            mg: &mut mg,
                        },
                        &pos.board,
                        &pos.racks,
                        mover as usize,
                        t,
                        false,
                    );
                    for (lean, arm_tables) in &arms {
                        total += 1;
                        let mut egs =
                            EndgameSolver::<kwg::Node22, kwg::Node22>::new(&gc, &kwg, arm_tables)
                                .unwrap();
                        egs.init(&pos.board, [&pos.racks[0][..], &pos.racks[1][..]]);
                        let solvev = egs.solve_one_in_bag(mover, t).unwrap();
                        if solvev != super::to_equity(refv) {
                            disagreements += 1;
                            println!(
                                "DISAGREE [{name}] lean={lean} mover={mover} bag={t}: ref={refv} solve={}\n   {}",
                                solvev.raw(),
                                describe(&gc, &pos)
                            );
                        }
                    }
                }
            }
        }
        println!("one-in-bag differential: {disagreements} disagreements out of {total} cases");
        assert_eq!(
            disagreements, 0,
            "solve_one_in_bag disagreed with reference_one_in_bag"
        );
    }

    #[test]
    #[inline]
    fn one_in_bag_forced_double_pass() {
        let gc = game_config::make_english_game_config();
        let kwg_bytes = tiny_kwg_bytes();
        let kwg = kwg::Kwg::<kwg::Node22>::from_bytes_alloc(&kwg_bytes).unwrap();

        let board = empty_board();
        let racks = [vec![2u8, 2], vec![8u8, 20]];
        let arm_tables = anagrams::ArmTables::build(&kwg, &gc, false).unwrap();

        let mut egs =
            EndgameSolver::<kwg::Node22, kwg::Node22>::new(&gc, &kwg, &arm_tables).unwrap();
        egs.init(&board, [&racks[0][..], &racks[1][..]]);

        let v0 = egs.solve_one_in_bag(0, 2).unwrap();
        assert_eq!(
            v0,
            equity::Equity::new(-1000),
            "p0 forced-pass leftover should be -1000"
        );

        let mut egs =
            EndgameSolver::<kwg::Node22, kwg::Node22>::new(&gc, &kwg, &arm_tables).unwrap();
        egs.init(&board, [&racks[0][..], &racks[1][..]]);
        let v1 = egs.solve_one_in_bag(1, 2).unwrap();
        assert_eq!(
            v1,
            equity::Equity::new(1000),
            "p1 forced-pass leftover should be +1000"
        );
    }

    #[test]
    #[inline]
    fn a_one_in_bag_solve_agrees_with_the_reference_and_beats_passing() {
        let gc = game_config::make_english_game_config();
        let kwg_bytes = tiny_kwg_bytes();
        let kwg = kwg::Kwg::<kwg::Node22>::from_bytes_alloc(&kwg_bytes).unwrap();
        let klv = klv::Klv::<kwg::Node22>::from_bytes_alloc(klv::EMPTY_KLV_BYTES).unwrap();
        let mut mg = movegen::KurniaMoveGenerator::new(&gc);

        let board = empty_board();
        let racks = [vec![1u8, 2], vec![8u8, 20]]; // p0=[A,B] p1=[H,T]
        let bag = 1u8; // one A in the bag

        let refv = reference_one_in_bag(
            &mut Reference {
                gc: &gc,
                kwg: &kwg,
                klv: &klv,
                mg: &mut mg,
            },
            &board,
            &racks,
            0,
            bag,
            false,
        );
        let arm_tables = anagrams::ArmTables::build(&kwg, &gc, false).unwrap();
        let mut egs =
            EndgameSolver::<kwg::Node22, kwg::Node22>::new(&gc, &kwg, &arm_tables).unwrap();
        egs.init(&board, [&racks[0][..], &racks[1][..]]);
        let solvev = egs.solve_one_in_bag(0, bag).unwrap();
        assert_eq!(solvev, super::to_equity(refv), "bag layer vs reference");

        let a = gc.alphabet();
        let pass_only =
            equity::Equity::new(a.scaled_rack_score(&racks[1]) - a.scaled_rack_score(&racks[0]));
        assert!(
            solvev >= pass_only,
            "playing should not be worse than the pass-only leftover: solve={solvev} pass_only={pass_only}"
        );
    }

    #[test]
    #[inline]
    fn differential_one_in_bag_exchange() {
        let gc = game_config::make_exchange_test_game_config();
        let kwg_bytes = tiny_kwg_bytes();
        let kwg = kwg::Kwg::<kwg::Node22>::from_bytes_alloc(&kwg_bytes).unwrap();
        let klv = klv::Klv::<kwg::Node22>::from_bytes_alloc(klv::EMPTY_KLV_BYTES).unwrap();
        let mut mg = movegen::KurniaMoveGenerator::new(&gc);

        let bag_tiles: [u8; 2] = [1, 0]; // A, blank
        let arms = both_arms(&kwg, &gc);

        let mut disagreements = 0usize;
        let mut total = 0usize;

        let positions: Vec<(String, Position)> = embedded_positions()
            .into_iter()
            .filter(|(_, pos)| pos.racks[0].len() <= 2 && pos.racks[1].len() <= 2)
            .collect();
        for (name, pos) in positions {
            for &t in &bag_tiles {
                for mover in 0u8..2 {
                    let refv = reference_one_in_bag_ex(
                        &mut Reference {
                            gc: &gc,
                            kwg: &kwg,
                            klv: &klv,
                            mg: &mut mg,
                        },
                        &pos.board,
                        &pos.racks,
                        mover as usize,
                        t,
                        0,
                        0,
                    );
                    for (lean, arm_tables) in &arms {
                        total += 1;
                        let mut egs =
                            EndgameSolver::<kwg::Node22, kwg::Node22>::new(&gc, &kwg, arm_tables)
                                .unwrap();
                        egs.init(&pos.board, [&pos.racks[0][..], &pos.racks[1][..]]);
                        let solvev = egs.solve_one_in_bag(mover, t).unwrap();
                        if solvev != super::to_equity(refv) {
                            disagreements += 1;
                            println!(
                                "DISAGREE [{name}] lean={lean} mover={mover} bag={t}: ref={refv} solve={}\n   {}",
                                solvev.raw(),
                                describe(&gc, &pos)
                            );
                        }
                    }
                }
            }
        }
        println!(
            "one-in-bag exchange differential: {disagreements} disagreements out of {total} cases"
        );
        assert_eq!(
            disagreements, 0,
            "solve_one_in_bag disagreed with reference_one_in_bag_ex"
        );
    }

    #[test]
    #[inline]
    fn one_in_bag_exchange_beats_passing() {
        let gc = game_config::make_exchange_test_game_config();
        let kwg_bytes = tiny_kwg_bytes();
        let kwg = kwg::Kwg::<kwg::Node22>::from_bytes_alloc(&kwg_bytes).unwrap();

        let board = empty_board();
        let racks = [vec![17u8], vec![1u8]]; // p0 = [Q], p1 = [A]
        let bag = 1u8; // one A in the bag

        let arm_tables = anagrams::ArmTables::build(&kwg, &gc, false).unwrap();
        let mut egs =
            EndgameSolver::<kwg::Node22, kwg::Node22>::new(&gc, &kwg, &arm_tables).unwrap();
        egs.init(&board, [&racks[0][..], &racks[1][..]]);
        let solvev = egs.solve_one_in_bag(0, bag).unwrap();

        let a = gc.alphabet();
        let pass_only =
            equity::Equity::new(a.scaled_rack_score(&racks[1]) - a.scaled_rack_score(&racks[0])); // -9000
        assert_eq!(
            solvev,
            equity::Equity::ZERO,
            "exchange line value should be exactly 0"
        );
        assert!(
            solvev > pass_only,
            "exchange should beat passing: solve={solvev} pass_only={pass_only}"
        );
    }

    #[test]
    #[inline]
    fn peg_one_in_bag_exchange_solved_or_declined() {
        let kwg_bytes = tiny_kwg_bytes();
        let kwg = kwg::Kwg::<kwg::Node22>::from_bytes_alloc(&kwg_bytes).unwrap();

        let gc = game_config::make_exchange_test_game_config();
        let board = empty_board();
        let mover_rack = [2u8]; // B, unplayable alone on an empty board
        let mut unseen_tally = vec![0u8; gc.alphabet().len() as usize];
        unseen_tally[8] = 1; // H
        unseen_tally[20] = 1; // T
        let arm_tables = anagrams::ArmTables::build(&kwg, &gc, false).unwrap();
        let mut egs =
            EndgameSolver::<kwg::Node22, kwg::Node22>::new(&gc, &kwg, &arm_tables).unwrap();
        let result = egs
            .solve_peg_one_in_bag(
                0,
                &board,
                &mover_rack,
                &unseen_tally,
                equity::Equity::ZERO,
                &super::Limits::NONE,
            )
            .unwrap()
            .expect("the exchange test config forces an end and is solvable");
        assert_eq!(result.hypotheses.len(), 2);

        let gc_en = game_config::make_english_game_config();
        let arm_tables_en = anagrams::ArmTables::build(&kwg, &gc_en, false).unwrap();
        let mut egs_en =
            EndgameSolver::<kwg::Node22, kwg::Node22>::new(&gc_en, &kwg, &arm_tables_en).unwrap();
        assert!(
            egs_en
                .solve_peg_one_in_bag(
                    0,
                    &board,
                    &mover_rack,
                    &unseen_tally,
                    equity::Equity::ZERO,
                    &super::Limits::NONE
                )
                .unwrap()
                .is_ok()
        );

        let gc_bad = game_config::make_exchange_unsolvable_test_game_config();
        let arm_tables_bad = anagrams::ArmTables::build(&kwg, &gc_bad, false).unwrap();
        let mut egs_bad =
            EndgameSolver::<kwg::Node22, kwg::Node22>::new(&gc_bad, &kwg, &arm_tables_bad).unwrap();
        assert!(matches!(
            egs_bad
                .solve_peg_one_in_bag(
                    0,
                    &board,
                    &mover_rack,
                    &unseen_tally,
                    equity::Equity::ZERO,
                    &super::Limits::NONE
                )
                .unwrap(),
            Err(super::PegUnsupported::ExchangeWithoutForcedEnd)
        ));
    }

    #[test]
    #[inline]
    fn differential_peg_committed() {
        let gc = game_config::make_english_game_config();
        let kwg_bytes = tiny_kwg_bytes();
        let kwg = kwg::Kwg::<kwg::Node22>::from_bytes_alloc(&kwg_bytes).unwrap();
        let klv = klv::Klv::<kwg::Node22>::from_bytes_alloc(klv::EMPTY_KLV_BYTES).unwrap();
        let mut mg = movegen::KurniaMoveGenerator::new(&gc);
        let a = gc.alphabet();
        let alen = a.len() as usize;
        let arms = both_arms(&kwg, &gc);

        let committed = |board: &[u8],
                         mover_rack: &[u8],
                         unseen: &[(u8, u32)],
                         mg: &mut movegen::KurniaMoveGenerator|
         -> (f32, f32, bool, String) {
            let total: u32 = unseen.iter().map(|&(_, w)| w).sum();
            let mover = 0usize;
            let opp = 1usize;
            let snap = movegen::BoardSnapshot {
                board_tiles: board,
                game_config: &gc,
                kwg: &kwg,
                anagrams: None,
                rack_lengths: None,
                klv: &klv,
            };
            mg.gen_moves_raw_all_unsorted(&snap, mover_rack, 0, movegen::PassPolicy::AsACandidate);
            let mut cands: Vec<Option<movegen::Play>> = vec![None]; // None = pass
            for vm in &mg.plays {
                if let movegen::Play::Place { .. } = &vm.play {
                    cands.push(Some(vm.play.clone()));
                }
            }
            let mut best = (f32::NEG_INFINITY, f32::NEG_INFINITY, false, String::new());
            for cand in &cands {
                let mut win = 0.0f32;
                let mut marg = 0.0f32;
                for &(t, w) in unseen {
                    let mut opp_rack = Vec::new();
                    for &(u, c) in unseen {
                        let take = if u == t { c - 1 } else { c };
                        for _ in 0..take {
                            opp_rack.push(u);
                        }
                    }
                    let drawn = t & !((t as i8) >> 7) as u8;
                    let v = match cand {
                        Some(movegen::Play::Place {
                            down,
                            lane,
                            idx,
                            word,
                            score,
                        }) => {
                            let mut nb = board.to_vec();
                            let mut nr = [mover_rack.to_vec(), Vec::new()];
                            apply_place(&gc, &mut nb, &mut nr[mover], *down, *lane, *idx, word);
                            nr[mover].push(drawn);
                            nr[opp] = opp_rack;
                            *score as f32
                                - reference(
                                    &mut Reference {
                                        gc: &gc,
                                        kwg: &kwg,
                                        klv: &klv,
                                        mg,
                                    },
                                    &nb,
                                    &nr,
                                    opp,
                                    false,
                                )
                        }
                        _ => {
                            let nr = [mover_rack.to_vec(), opp_rack];
                            -reference_one_in_bag(
                                &mut Reference {
                                    gc: &gc,
                                    kwg: &kwg,
                                    klv: &klv,
                                    mg,
                                },
                                board,
                                &nr,
                                opp,
                                t,
                                true,
                            )
                        }
                    };
                    marg += (w as f32) * v;
                    win += (w as f32)
                        * if v > 0.0 {
                            1.0
                        } else if v == 0.0 {
                            0.5
                        } else {
                            0.0
                        };
                }
                marg /= total as f32;
                win /= total as f32;
                if win > best.0 || (win == best.0 && marg > best.1) {
                    let label = if cand.is_some() { "place" } else { "PASS" };
                    best = (win, marg, cand.is_some(), label.to_string());
                }
            }
            best
        };

        let tiles = [1u8, 2, 8, 20]; // A B H T
        let mut boards: Vec<(String, Vec<u8>)> = Vec::new();
        boards.push(("empty".into(), empty_board()));
        {
            let mut b = empty_board();
            put_word(&mut b, 7, 6, &[1, 20]);
            boards.push(("AT".into(), b));
        }
        {
            let mut b = empty_board();
            put_word(&mut b, 7, 6, &[1, 1, 8]);
            boards.push(("AAH".into(), b));
        }
        {
            let mut b = empty_board();
            put_word(&mut b, 7, 6, &[1, 20]);
            put_word(&mut b, 5, 7, &[8, 1, 20]);
            boards.push(("AT+HAT".into(), b));
        }

        let mut worst_gap = 0.0f32;
        let mut n_gap = 0;
        let mut n_gap_place = 0;
        let mut report = String::new();
        for (bname, board) in &boards {
            let mut racks: Vec<Vec<u8>> = Vec::new();
            for &x in &tiles {
                racks.push(vec![x]);
                for &y in &tiles {
                    racks.push(vec![x, y]);
                }
            }
            for mrack in &racks {
                for i in 0..tiles.len() {
                    for j in i..tiles.len() {
                        let (ti, tj) = (tiles[i], tiles[j]);
                        let mut unseen_tally = vec![0u8; alen];
                        let mut unseen: Vec<(u8, u32)> = Vec::new();
                        if ti == tj {
                            unseen_tally[ti as usize] = 2;
                            unseen.push((ti, 2));
                        } else {
                            unseen_tally[ti as usize] = 1;
                            unseen_tally[tj as usize] = 1;
                            unseen.push((ti, 1));
                            unseen.push((tj, 1));
                        }

                        let (ref_win, ref_marg, _ref_place, _ref_label) =
                            committed(board, mrack, &unseen, &mut mg);

                        for (lean, arm_tables) in &arms {
                            let mut egs = EndgameSolver::<kwg::Node22, kwg::Node22>::new(
                                &gc, &kwg, arm_tables,
                            )
                            .unwrap();
                            let got = egs
                                .solve_peg_one_in_bag(
                                    0,
                                    board,
                                    mrack,
                                    &unseen_tally,
                                    equity::Equity::ZERO,
                                    &super::Limits::NONE,
                                )
                                .unwrap()
                                .expect("english one-in-bag is always solvable");

                            let mut egs2 = EndgameSolver::<kwg::Node22, kwg::Node22>::new(
                                &gc, &kwg, arm_tables,
                            )
                            .unwrap();
                            let clair = egs2
                                .peg_clairvoyant_aggregate(
                                    0,
                                    board,
                                    mrack,
                                    &unseen_tally,
                                    0.0,
                                    &super::BagLimits::new(&super::Limits::NONE, None),
                                )
                                .unwrap();

                            assert!(got.committed, "english path must be committed");
                            assert!(got.best_move.is_some(), "committed result must name a move");
                            assert!(got.exact && clair.exact, "an unlimited solve is exact");
                            assert!(
                                (got.win_pct - ref_win).abs() < 1e-6,
                                "[{bname}] lean={lean} mover=[{}] unseen={unseen:?}: committed solver win% {} != reference {}",
                                a.fmt_rack(mrack),
                                got.win_pct,
                                ref_win,
                            );
                            // the solver's margin is rounded to a whole millipoint
                            assert!(
                                (got.expected_margin.raw() as f32 - ref_marg).abs() < 0.6,
                                "[{bname}] lean={lean} mover=[{}] unseen={unseen:?}: committed solver margin {} != reference {}",
                                a.fmt_rack(mrack),
                                got.expected_margin.raw(),
                                ref_marg,
                            );
                            assert!(
                                got.win_pct <= clair.win_pct + 1e-6,
                                "[{bname}] lean={lean} mover=[{}] unseen={unseen:?}: committed win% {} exceeds clairvoyant bound {}",
                                a.fmt_rack(mrack),
                                got.win_pct,
                                clair.win_pct,
                            );
                            let gap = clair.win_pct - got.win_pct;
                            if gap > 1e-6 {
                                n_gap += 1;
                                if gap > worst_gap {
                                    worst_gap = gap;
                                }
                                n_gap_place +=
                                    matches!(got.best_move, Some(PegMove::Place(_))) as usize;
                                if report.lines().count() < 25 {
                                    report.push_str(&format!(
                                        "gap win%={gap:.3} [{bname}] lean={lean} mover=[{}] unseen={unseen:?}: clairvoyant {:.3} vs committed {:.3}\n",
                                        a.fmt_rack(mrack), clair.win_pct, got.win_pct,
                                    ));
                                }
                            }
                        }
                    }
                }
            }
        }
        println!(
            "committed differential: solver matched reference on all scanned positions; \
             {n_gap} showed strict committed < clairvoyant ({n_gap_place} where committed best is a place move), worst gap = {worst_gap:.3}"
        );
        print!("{report}");
        assert!(
            n_gap >= 1,
            "toy corpus never exercised committed < clairvoyant; the differential proves nothing"
        );
    }
}
