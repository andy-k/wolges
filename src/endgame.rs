// Copyright (C) 2020-2026 Andy Kurnia.

// note: this module is very slow and may need a lot of space
// and it still has many bugs

use super::{display, fash, game_config, klv, kwg, move_picker, movegen};

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
    t0: std::time::Instant, // for timing only
    tick_periods: move_picker::Periods,
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
}

impl WorkBuffer {
    fn new(game_config: &game_config::GameConfig) -> Self {
        let dim = game_config.board_layout().dim();
        let board_area = dim.rows as usize * dim.cols as usize;
        Self {
            t0: std::time::Instant::now(),
            tick_periods: move_picker::Periods(0),
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
        }
    }

    #[inline]
    fn init(&mut self) {
        self.t0 = std::time::Instant::now();
        self.tick_periods = move_picker::Periods(0);
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
    pub equity: f32,
    pub play: &'a movegen::Play,
}

// EndgameSolver is the main two-player endgame solver.
// EndgameSolver can only be reused for the same game_config and kwg.
// (Refer to note at WorkBuffer.)
// This is not enforced.
pub struct EndgameSolver<'a, N: kwg::Node, L: kwg::Node> {
    game_config: &'a game_config::GameConfig,
    kwg: &'a kwg::Kwg<N>,
    klv: Box<klv::Klv<L>>,
    board_tiles: Vec<u8>,
    racks: [Vec<u8>; 2],
    rack_scores: [i32; 2],
    work_buffer: WorkBuffer,
}

impl<'a, N: kwg::Node, L: kwg::Node> EndgameSolver<'a, N, L> {
    pub fn new(game_config: &'a game_config::GameConfig, kwg: &'a kwg::Kwg<N>) -> Self {
        if game_config.num_players() != 2 {
            panic!("cannot solve non-2-player endgames");
        }
        Self {
            game_config,
            kwg,
            klv: Box::new(klv::Klv::<L>::from_bytes_alloc(klv::EMPTY_KLV_BYTES)),
            board_tiles: Vec::new(),
            racks: [Vec::new(), Vec::new()],
            rack_scores: [0, 0],
            work_buffer: WorkBuffer::new(game_config),
        }
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
    fn get_new_state_idx(&mut self, state_idx: u32, which_player: u8, play_idx: u32) -> u32 {
        match &self.work_buffer.plays[play_idx as usize] {
            movegen::Play::Exchange { .. } => state_idx,
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
                            panic!("too many states");
                        }
                    }
                }

                new_state_idx
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
    fn run_id_loop(&mut self, player_idx: u8, verbose: bool) -> f32 {
        let mut last_valuation = f32::NAN;
        for max_depth in 1.. {
            let old_num_state_eval = self.work_buffer.state_eval.len();

            self.work_buffer.depth_limited = false;
            let valuation = if max_depth == 1 {
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
            last_valuation = valuation;
            if verbose {
                println!(
                    "valuation for depth {max_depth} is {}",
                    valuation / super::equity::SCALE as f32
                );
                self.print_progress();
                self.print_best_line(player_idx);
            }
            // check for time limit here

            if self.work_buffer.state_eval.len() == old_num_state_eval
                && !self.work_buffer.depth_limited
            {
                break;
            }
        }
        last_valuation
    }

    #[inline]
    fn aspiration_search(&mut self, player_idx: u8, max_depth: i8, last_valuation: f32) -> f32 {
        const ASPIRATION_WINDOW: f32 = (3 * super::equity::SCALE) as f32;
        let lo = last_valuation - ASPIRATION_WINDOW;
        let hi = last_valuation + ASPIRATION_WINDOW;
        let v = self.negamax_eval(0, player_idx, max_depth, lo, hi, false);
        if v > lo && v < hi {
            v
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

    #[inline(always)]
    pub fn evaluate(&mut self, player_idx: u8) -> f32 {
        self.run_id_loop(player_idx, true)
    }

    #[inline(always)]
    pub fn solve(&mut self, player_idx: u8) -> f32 {
        self.run_id_loop(player_idx, false)
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
    ) -> f32 {
        // movegen not done for depth == 0, so no state_eval.
        if depth == 0 {
            self.work_buffer.depth_limited = true;

            return self.both_pass_value(state_idx, player_idx);
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
                        return state_side_eval.equity;
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
                    return state_side_eval.equity;
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
            println!(
                "position {} has racks {:?} and board",
                state_idx, current_ply_buffer.racks
            );
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
                                    panic!("too many plays");
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
                                    );
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
                                )
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
            -self.negamax_eval(state_idx, player_idx ^ 1, depth - 1, -beta, -alpha, true)
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

        // quell impatience
        if self
            .work_buffer
            .tick_periods
            .update(self.work_buffer.t0.elapsed().as_millis() as u64 / 10000)
        {
            self.print_progress();
        }

        best_valuation
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
                equity: ans1.equity,
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
                    equity: ans1.equity,
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
    pub fn collect_pv(&'a self, player_idx: u8, out: &mut Vec<(f32, movegen::Play)>) {
        out.clear();
        self.append_solution(0, player_idx, |found| {
            out.push((found.equity, found.play.clone()));
        });
    }

    #[inline]
    pub fn print_best_line(&mut self, player_idx: u8) {
        let mut current_ply_buffer = std::mem::take(&mut self.work_buffer.current_ply_buffer);
        let board_tiles = &mut current_ply_buffer.board_tiles;
        board_tiles.clone_from(&self.board_tiles);
        let racks = &mut current_ply_buffer.racks;
        racks.clone_from(&self.racks);
        let mut leftovers = [f32::NAN, f32::NAN];
        let mut i = 0usize;
        self.append_solution(0, player_idx, |ply| {
            let player_turn_idx = (player_idx as usize + i) & 1;
            let rack = &mut racks[player_turn_idx];
            leftovers[player_turn_idx ^ 1] = f32::NAN;
            leftovers[player_turn_idx] = ply.equity
                - match ply.play {
                    movegen::Play::Exchange { .. } => 0.0,
                    movegen::Play::Place { score, .. } => *score as f32,
                };
            println!(
                "{}: p{}: {}{:width$} {} {}",
                i,
                player_turn_idx,
                self.game_config.alphabet().fmt_rack(rack),
                "",
                ply.equity / super::equity::SCALE as f32,
                ply.play.fmt(&movegen::BoardSnapshot {
                    board_tiles,
                    game_config: self.game_config,
                    kwg: self.kwg,
                    klv: &self.klv,
                }),
                width = self.game_config.rack_size() as usize - rack.len(),
            );
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
        for _ in 0..2 {
            let player_turn_idx = (player_idx as usize + i) & 1;
            let rack = &racks[player_turn_idx];
            let leftover = leftovers[player_turn_idx];
            print!(
                "e: p{}: {}",
                player_turn_idx,
                self.game_config.alphabet().fmt_rack(rack)
            );
            if !leftover.is_nan() {
                print!(
                    "{:width$} {}",
                    "",
                    leftover / super::equity::SCALE as f32,
                    width = self.game_config.rack_size() as usize - rack.len(),
                );
            }
            println!();
            i += 1;
        }
        display::print_board(
            self.game_config.alphabet(),
            self.game_config.board_layout(),
            board_tiles,
        );
        self.work_buffer.current_ply_buffer = current_ply_buffer;
    }

    #[inline]
    fn print_progress(&self) {
        let dur0 = self.work_buffer.t0.elapsed();
        let dur1 = self.work_buffer.dur_movegen;
        println!(
            "after {:?} ({:?} on movegen), there are {} states, {} evaluated, {} child_plays, {} plays",
            dur0,
            dur1,
            self.work_buffer.states.len(),
            self.work_buffer.state_eval.len(),
            self.work_buffer.child_plays.len(),
            self.work_buffer.plays.len(),
        );
    }

    #[inline]
    pub fn solve_one_in_bag(&mut self, player_idx: u8, bag_tile: u8) -> f32 {
        let gc = self.game_config;
        let kwg = self.kwg;

        let mut sub = EndgameSolver::<N, L>::new(gc, kwg);

        let mut mg = movegen::KurniaMoveGenerator::new(gc);

        let klv = klv::Klv::<L>::from_bytes_alloc(klv::EMPTY_KLV_BYTES);

        let board = self.board_tiles.clone();
        let racks = [self.racks[0].clone(), self.racks[1].clone()];

        if one_in_bag_exchange_solvable(gc) {
            let mut memo = fash::MyHashMap::<BagExchangeKey, f32>::default();
            Self::one_in_bag_minimax_ex(
                gc, kwg, &klv, &mut sub, &mut mg, &board, &racks, player_idx, bag_tile, 0, 0,
                &mut memo,
            )
        } else {
            Self::one_in_bag_minimax(
                gc, kwg, &klv, &mut sub, &mut mg, &board, &racks, player_idx, bag_tile, false,
            )
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn one_in_bag_minimax(
        gc: &game_config::GameConfig,
        kwg: &kwg::Kwg<N>,
        klv: &klv::Klv<L>,
        sub: &mut EndgameSolver<'a, N, L>,
        mg: &mut movegen::KurniaMoveGenerator,
        board: &[u8],
        racks: &[Vec<u8>; 2],
        mover: u8,
        bag_tile: u8,
        just_passed: bool,
    ) -> f32 {
        let alphabet = gc.alphabet();
        let opp = mover ^ 1;
        let snapshot = movegen::BoardSnapshot {
            board_tiles: board,
            game_config: gc,
            kwg,
            klv,
        };
        mg.gen_moves_raw_all_unsorted(
            &snapshot,
            &racks[mover as usize],
            0,
            movegen::PassPolicy::AsACandidate,
        );

        let drawn = bag_tile & !((bag_tile as i8) >> 7) as u8;

        let mut best = f32::NEG_INFINITY;
        for vm in &mg.plays {
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
                    let strider = gc.board_layout().dim().lane(*down, *lane);
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

                sub.init(&nb, [&nr[0], &nr[1]]);
                let v = sub.solve(opp);

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
            -Self::one_in_bag_minimax(gc, kwg, klv, sub, mg, board, racks, opp, bag_tile, true)
        };
        if pass_val > best {
            best = pass_val;
        }
        best
    }

    #[allow(clippy::too_many_arguments)]
    fn one_in_bag_minimax_ex(
        gc: &game_config::GameConfig,
        kwg: &kwg::Kwg<N>,
        klv: &klv::Klv<L>,
        sub: &mut EndgameSolver<'a, N, L>,
        mg: &mut movegen::KurniaMoveGenerator,
        board: &[u8],
        racks: &[Vec<u8>; 2],
        mover: u8,
        bag_tile: u8,
        passes: u8,
        zeros: u8,
        memo: &mut fash::MyHashMap<BagExchangeKey, f32>,
    ) -> f32 {
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
        if let Some(&v) = memo.get(&key) {
            return v;
        }

        let alphabet = gc.alphabet();
        let opp = mover ^ 1;
        let snapshot = movegen::BoardSnapshot {
            board_tiles: board,
            game_config: gc,
            kwg,
            klv,
        };
        mg.gen_moves_raw_all_unsorted(
            &snapshot,
            &racks[mover as usize],
            0,
            movegen::PassPolicy::AsACandidate,
        );

        let mut places: Vec<movegen::Play> = Vec::new();
        for vm in &mg.plays {
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
                    let strider = gc.board_layout().dim().lane(*down, *lane);
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
                sub.init(&nb, [&nr[0], &nr[1]]);
                let v = sub.solve(opp);
                let val = *score as f32 - v;
                if val > best {
                    best = val;
                }
            }
        }

        let pass_passes = passes + 1;
        let pass_zeros = zeros + 1;
        let pass_val = if scoreless_turns_end(gc, pass_passes, pass_zeros) {
            (alphabet.scaled_rack_score(&racks[opp as usize])
                - alphabet.scaled_rack_score(&racks[mover as usize])) as f32
        } else {
            -Self::one_in_bag_minimax_ex(
                gc,
                kwg,
                klv,
                sub,
                mg,
                board,
                racks,
                opp,
                bag_tile,
                pass_passes,
                pass_zeros,
                memo,
            )
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
            let exch_val = if scoreless_turns_end(gc, 0, exch_zeros) {
                (alphabet.scaled_rack_score(&nr[opp as usize])
                    - alphabet.scaled_rack_score(&nr[mover as usize])) as f32
            } else {
                -Self::one_in_bag_minimax_ex(
                    gc, kwg, klv, sub, mg, board, &nr, opp, r, 0, exch_zeros, memo,
                )
            };
            if exch_val > best {
                best = exch_val;
            }
        }

        memo.insert(key, best);
        best
    }

    #[inline]
    pub fn solve_peg_one_in_bag(
        &mut self,
        mover: u8,
        board: &[u8],
        mover_rack: &[u8],
        unseen_tally: &[u8],
        score_diff: f32,
    ) -> Result<PegResult, PegUnsupported> {
        let gc = self.game_config;
        if one_in_bag_exchange_legal(gc) && !one_in_bag_exchange_solvable(gc) {
            return Err(PegUnsupported::ExchangeWithoutForcedEnd);
        }

        if one_in_bag_exchange_legal(gc) {
            Ok(self.peg_clairvoyant_aggregate(mover, board, mover_rack, unseen_tally, score_diff))
        } else {
            let unseen: Vec<(u8, u32)> = unseen_tally
                .iter()
                .enumerate()
                .filter(|&(_, &c)| c != 0)
                .map(|(t, &c)| (t as u8, c as u32))
                .collect();
            Ok(self.peg_committed_no_exchange(mover, board, mover_rack, &unseen, score_diff))
        }
    }

    #[inline]
    fn peg_clairvoyant_aggregate(
        &mut self,
        mover: u8,
        board: &[u8],
        mover_rack: &[u8],
        unseen_tally: &[u8],
        score_diff: f32,
    ) -> PegResult {
        let mut hypotheses: Vec<(u8, u32, f32)> = Vec::new();

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
            let v = self.solve_one_in_bag(mover, bag_tile) + score_diff;
            hypotheses.push((bag_tile, count as u32, v));
        }
        let weighted: Vec<(u32, f32)> = hypotheses.iter().map(|&(_, w, v)| (w, v)).collect();
        let (win_pct, expected_margin) = peg_aggregate(&weighted);
        PegResult {
            win_pct,
            expected_margin,
            hypotheses,
            best_move: None,
            committed: false,
        }
    }

    #[inline]
    fn peg_committed_no_exchange(
        &self,
        mover: u8,
        board: &[u8],
        mover_rack: &[u8],
        unseen: &[(u8, u32)],
        score_diff: f32,
    ) -> PegResult {
        let gc = self.game_config;
        let kwg = self.kwg;
        let klv = klv::Klv::<L>::from_bytes_alloc(klv::EMPTY_KLV_BYTES);
        let opp = mover ^ 1;
        let total = unseen.iter().map(|&(_, w)| w).sum::<u32>() as f32;

        let mut mg = movegen::KurniaMoveGenerator::new(gc);
        mg.gen_moves_raw_all_unsorted(
            &movegen::BoardSnapshot {
                board_tiles: board,
                game_config: gc,
                kwg,
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

        let mut sub = EndgameSolver::<N, L>::new(gc, kwg);

        let mut best_win = f32::NEG_INFINITY;
        let mut best_marg = f32::NEG_INFINITY;
        let mut best_move = PegMove::Pass;
        let mut best_hyps: Vec<(u8, u32, f32)> = Vec::new();

        for cand in &candidates {
            let mut win = 0.0f32;
            let mut marg = 0.0f32;
            let mut hyps: Vec<(u8, u32, f32)> = Vec::with_capacity(unseen.len());
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
                        *score as f32 - sub.solve(opp)
                    }
                    PegMove::Pass => {
                        // PEG2-7: with two or more in the bag, draw every tile from the SAME end -- pop,
                        // the back -- and store a scenario reversed to match. Do NOT reuse Bag::replenish
                        // for the enumeration: it pops for even players and shifts for odd ones, which on
                        // a reversed bag hands p1 the leftover instead of the next popped tile.
                        let mut r2: [Vec<u8>; 2] = [Vec::new(), Vec::new()];
                        r2[mover as usize] = mover_rack.to_vec();
                        r2[opp as usize] = opp_rack;
                        -Self::one_in_bag_minimax(
                            gc, kwg, &klv, &mut sub, &mut mg, board, &r2, opp, t, true,
                        )
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
                hyps.push((t, w, vv));
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

        PegResult {
            win_pct: best_win,
            expected_margin: best_marg,
            hypotheses: best_hyps,
            best_move: Some(best_move),
            committed: true,
        }
    }
}

#[derive(Clone)]
pub enum PegMove {
    Pass,
    Place(movegen::Play),
}

pub struct PegResult {
    pub win_pct: f32,
    pub expected_margin: f32,
    pub hypotheses: Vec<(u8, u32, f32)>,
    pub best_move: Option<PegMove>,
    pub committed: bool,
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
pub fn peg_aggregate(hypotheses: &[(u32, f32)]) -> (f32, f32) {
    let mut total = 0.0f32;
    let mut win_sum = 0.0f32;
    let mut margin_sum = 0.0f32;
    for &(weight, value) in hypotheses {
        let w = weight as f32;
        let win_score = if value > 0.0 {
            1.0
        } else if value == 0.0 {
            0.5
        } else {
            0.0
        };
        total += w;
        win_sum += win_score * w;
        margin_sum += value * w;
    }
    if total == 0.0 {
        (0.0, 0.0)
    } else {
        (win_sum / total, margin_sum / total)
    }
}

#[cfg(test)]
mod tests {
    use super::{EndgameSolver, PegMove};
    use crate::{alphabet, bites, build, game_config, klv, kwg, movegen};

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
            &tiny_word_list(),
        )
        .unwrap()
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

    #[allow(clippy::too_many_arguments)]
    fn reference<N: kwg::Node, L: kwg::Node>(
        gc: &game_config::GameConfig,
        kwg: &kwg::Kwg<N>,
        klv: &klv::Klv<L>,
        mg: &mut movegen::KurniaMoveGenerator,
        board: &[u8],
        racks: &[Vec<u8>; 2],
        mover: usize,
        just_passed: bool,
    ) -> f32 {
        let alphabet = gc.alphabet();
        let snapshot = movegen::BoardSnapshot {
            board_tiles: board,
            game_config: gc,
            kwg,
            klv,
        };
        mg.gen_moves_raw_all_unsorted(
            &snapshot,
            &racks[mover],
            0,
            movegen::PassPolicy::AsACandidate,
        );

        let mut places: Vec<movegen::Play> = Vec::new();
        for vm in &mg.plays {
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
                    apply_place(gc, &mut nb, &mut nr[mover], *down, *lane, *idx, word);
                    *score as f32 - reference(gc, kwg, klv, mg, &nb, &nr, opp, false)
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
            -reference(gc, kwg, klv, mg, board, racks, opp, true)
        };
        if pass_val > best {
            best = pass_val;
        }
        best
    }

    #[allow(clippy::too_many_arguments)]
    fn reference_one_in_bag<N: kwg::Node, L: kwg::Node>(
        gc: &game_config::GameConfig,
        kwg: &kwg::Kwg<N>,
        klv: &klv::Klv<L>,
        mg: &mut movegen::KurniaMoveGenerator,
        board: &[u8],
        racks: &[Vec<u8>; 2],
        mover: usize,
        bag_tile: u8,
        just_passed: bool,
    ) -> f32 {
        let alphabet = gc.alphabet();
        let snapshot = movegen::BoardSnapshot {
            board_tiles: board,
            game_config: gc,
            kwg,
            klv,
        };
        mg.gen_moves_raw_all_unsorted(
            &snapshot,
            &racks[mover],
            0,
            movegen::PassPolicy::AsACandidate,
        );

        let mut places: Vec<movegen::Play> = Vec::new();
        for vm in &mg.plays {
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
                apply_place(gc, &mut nb, &mut nr[mover], *down, *lane, *idx, word);
                nr[mover].push(drawn);

                let val = *score as f32 - reference(gc, kwg, klv, mg, &nb, &nr, opp, false);
                if val > best {
                    best = val;
                }
            }
        }

        let pass_val = if just_passed {
            (alphabet.scaled_rack_score(&racks[opp]) - alphabet.scaled_rack_score(&racks[mover]))
                as f32
        } else {
            -reference_one_in_bag(gc, kwg, klv, mg, board, racks, opp, bag_tile, true)
        };
        if pass_val > best {
            best = pass_val;
        }
        best
    }

    #[allow(clippy::too_many_arguments)]
    fn reference_one_in_bag_ex<N: kwg::Node, L: kwg::Node>(
        gc: &game_config::GameConfig,
        kwg: &kwg::Kwg<N>,
        klv: &klv::Klv<L>,
        mg: &mut movegen::KurniaMoveGenerator,
        board: &[u8],
        racks: &[Vec<u8>; 2],
        mover: usize,
        bag_tile: u8,
        passes: u8,
        zeros: u8,
    ) -> f32 {
        let alphabet = gc.alphabet();
        let snapshot = movegen::BoardSnapshot {
            board_tiles: board,
            game_config: gc,
            kwg,
            klv,
        };
        mg.gen_moves_raw_all_unsorted(
            &snapshot,
            &racks[mover],
            0,
            movegen::PassPolicy::AsACandidate,
        );
        let mut places: Vec<movegen::Play> = Vec::new();
        for vm in &mg.plays {
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
                apply_place(gc, &mut nb, &mut nr[mover], *down, *lane, *idx, word);
                nr[mover].push(drawn);
                let val = *score as f32 - reference(gc, kwg, klv, mg, &nb, &nr, opp, false);
                if val > best {
                    best = val;
                }
            }
        }

        let pass_passes = passes + 1;
        let pass_zeros = zeros + 1;
        let pass_val = if super::scoreless_turns_end(gc, pass_passes, pass_zeros) {
            (alphabet.scaled_rack_score(&racks[opp]) - alphabet.scaled_rack_score(&racks[mover]))
                as f32
        } else {
            -reference_one_in_bag_ex(
                gc,
                kwg,
                klv,
                mg,
                board,
                racks,
                opp,
                bag_tile,
                pass_passes,
                pass_zeros,
            )
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
            let exch_val = if super::scoreless_turns_end(gc, 0, exch_zeros) {
                (alphabet.scaled_rack_score(&nr[opp]) - alphabet.scaled_rack_score(&nr[mover]))
                    as f32
            } else {
                -reference_one_in_bag_ex(gc, kwg, klv, mg, board, &nr, opp, r, 0, exch_zeros)
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

        out
    }

    #[inline]
    fn splitmix64(s: &mut u64) -> u64 {
        *s = s.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = *s;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
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
        let mut seed = 0xD2D2_0007_2026u64;
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
        let kwg = kwg::Kwg::<kwg::Node22>::from_bytes_alloc(&kwg_bytes);
        let klv = klv::Klv::<kwg::Node22>::from_bytes_alloc(klv::EMPTY_KLV_BYTES);
        let mut mg = movegen::KurniaMoveGenerator::new(&gc);

        let mut disagreements = 0usize;
        let mut total = 0usize;
        for (name, pos) in all_positions() {
            total += 1;
            let refv = reference(&gc, &kwg, &klv, &mut mg, &pos.board, &pos.racks, 0, false);
            let mut egs = EndgameSolver::<kwg::Node22, kwg::Node22>::new(&gc, &kwg);
            egs.init(&pos.board, [&pos.racks[0][..], &pos.racks[1][..]]);
            let solvev = egs.solve(0);
            if refv.to_bits() != solvev.to_bits() {
                disagreements += 1;
                let ratio = if refv != 0.0 { solvev / refv } else { f32::NAN };
                println!(
                    "DISAGREE [{name}]: ref={refv} solve={solvev} ratio={ratio}\n   {}",
                    describe(&gc, &pos)
                );
            }
        }
        println!("differential: {disagreements} disagreements out of {total} positions");
        assert_eq!(disagreements, 0, "solve() disagreed with the reference");
    }

    #[test]
    #[inline]
    fn peg_aggregate_arithmetic() {
        let (win_pct, expected) = super::peg_aggregate(&[(2, 3000.0), (1, 0.0), (1, -5000.0)]);
        assert_eq!(win_pct, 0.625);
        assert_eq!(expected, 250.0);

        let (win_pct, expected) = super::peg_aggregate(&[(1, 1.0), (1, 0.0), (1, -1.0)]);
        assert_eq!(win_pct, 0.5);
        assert_eq!(expected, 0.0);

        assert_eq!(super::peg_aggregate(&[]), (0.0, 0.0));
    }

    #[test]
    #[inline]
    fn peg_one_in_bag_hand_checkable() {
        let gc = game_config::make_english_game_config();
        let kwg_bytes = tiny_kwg_bytes();
        let kwg = kwg::Kwg::<kwg::Node22>::from_bytes_alloc(&kwg_bytes);

        let board = empty_board();
        let mover_rack = [2u8]; // B
        let mut unseen_tally = vec![0u8; gc.alphabet().len() as usize];
        unseen_tally[8] = 1; // H
        unseen_tally[20] = 1; // T

        let mut egs = EndgameSolver::<kwg::Node22, kwg::Node22>::new(&gc, &kwg);
        let result = egs
            .solve_peg_one_in_bag(0, &board, &mover_rack, &unseen_tally, 0.0)
            .expect("english one-in-bag is always solvable");

        assert_eq!(result.hypotheses, vec![(8, 1, -2000.0), (20, 1, 1000.0)]);
        assert_eq!(result.win_pct, 0.5);
        assert_eq!(result.expected_margin, -500.0);
    }

    #[test]
    #[inline]
    fn peg_one_in_bag_score_diff_shifts_the_outcome() {
        let gc = game_config::make_english_game_config();
        let kwg_bytes = tiny_kwg_bytes();
        let kwg = kwg::Kwg::<kwg::Node22>::from_bytes_alloc(&kwg_bytes);

        let board = empty_board();
        let mover_rack = [2u8]; // B
        let mut unseen_tally = vec![0u8; gc.alphabet().len() as usize];
        unseen_tally[8] = 1; // H
        unseen_tally[20] = 1; // T

        let mut egs = EndgameSolver::<kwg::Node22, kwg::Node22>::new(&gc, &kwg);
        let result = egs
            .solve_peg_one_in_bag(0, &board, &mover_rack, &unseen_tally, 3000.0)
            .expect("english one-in-bag is always solvable");

        assert_eq!(result.hypotheses, vec![(8, 1, 1000.0), (20, 1, 4000.0)]);
        assert_eq!(result.win_pct, 1.0);
        assert_eq!(result.expected_margin, 2500.0);
    }

    #[test]
    #[inline]
    fn pv_playout_invariant() {
        let gc = game_config::make_english_game_config();
        let kwg_bytes = tiny_kwg_bytes();
        let kwg = kwg::Kwg::<kwg::Node22>::from_bytes_alloc(&kwg_bytes);

        let mut ok = 0usize;
        let mut bad = 0usize;
        let mut pv: Vec<(f32, movegen::Play)> = Vec::new();
        for (name, pos) in all_positions() {
            let mut egs = EndgameSolver::<kwg::Node22, kwg::Node22>::new(&gc, &kwg);
            egs.init(&pos.board, [&pos.racks[0][..], &pos.racks[1][..]]);
            let v = egs.solve(0);
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
                        apply_place(&gc, &mut board, &mut racks[mover], *down, *lane, *idx, word);
                        if racks[mover].is_empty() {
                            realized += sign * (2 * a.scaled_rack_score(&racks[mover ^ 1])) as f32;
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
            if realized.to_bits() == v.to_bits() {
                ok += 1;
            } else {
                bad += 1;
                println!(
                    "PV-REPLAY MISMATCH [{name}]: solve={v} realized={realized} pvlen={}",
                    pv.len()
                );
            }
        }
        println!("pv-playout: {ok} ok, {bad} mismatches");
        assert_eq!(bad, 0, "PV replay did not reproduce solve()'s value");
    }

    #[test]
    #[inline]
    fn properties_pv_legal_and_value_bounds() {
        let gc = game_config::make_english_game_config();
        let kwg_bytes = tiny_kwg_bytes();
        let kwg = kwg::Kwg::<kwg::Node22>::from_bytes_alloc(&kwg_bytes);
        let klv = klv::Klv::<kwg::Node22>::from_bytes_alloc(klv::EMPTY_KLV_BYTES);
        let mut mg = movegen::KurniaMoveGenerator::new(&gc);

        let mut illegal = 0usize;
        let mut oob = 0usize;
        let mut pv: Vec<(f32, movegen::Play)> = Vec::new();
        for (name, pos) in all_positions() {
            let mut egs = EndgameSolver::<kwg::Node22, kwg::Node22>::new(&gc, &kwg);
            egs.init(&pos.board, [&pos.racks[0][..], &pos.racks[1][..]]);
            let v = egs.solve(0);
            egs.collect_pv(0, &mut pv);

            let a = gc.alphabet();
            let board_pts: i32 = pos.board.iter().map(|&t| a.scaled_score(t)).sum();
            let rack_pts = a.scaled_rack_score(&pos.racks[0]) + a.scaled_rack_score(&pos.racks[1]);
            let cap = 4.0 * (board_pts + rack_pts) as f32 + 10000.0;
            if v.abs() > cap {
                oob += 1;
                println!("VALUE OOB [{name}]: v={v} cap={cap}");
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
                            println!("ILLEGAL PV PLAY [{name}] mover={mover}");
                        }
                        apply_place(&gc, &mut board, &mut racks[mover], *down, *lane, *idx, word);
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
        println!("properties: {illegal} illegal PV plays, {oob} out-of-bounds values");
        assert_eq!(illegal, 0, "a PV play was not legal");
        assert_eq!(oob, 0, "a solve() value exceeded the scaled bound");
    }

    #[test]
    #[inline]
    fn differential_one_in_bag() {
        let gc = game_config::make_english_game_config();
        let kwg_bytes = tiny_kwg_bytes();
        let kwg = kwg::Kwg::<kwg::Node22>::from_bytes_alloc(&kwg_bytes);
        let klv = klv::Klv::<kwg::Node22>::from_bytes_alloc(klv::EMPTY_KLV_BYTES);
        let mut mg = movegen::KurniaMoveGenerator::new(&gc);

        let bag_tiles: [u8; 4] = [1, 8, 20, 0];

        let mut disagreements = 0usize;
        let mut total = 0usize;

        let mut positions = embedded_positions();
        positions.extend(random_positions(12));
        for (name, pos) in positions {
            for &t in &bag_tiles {
                for mover in 0u8..2 {
                    total += 1;
                    let refv = reference_one_in_bag(
                        &gc,
                        &kwg,
                        &klv,
                        &mut mg,
                        &pos.board,
                        &pos.racks,
                        mover as usize,
                        t,
                        false,
                    );
                    let mut egs = EndgameSolver::<kwg::Node22, kwg::Node22>::new(&gc, &kwg);
                    egs.init(&pos.board, [&pos.racks[0][..], &pos.racks[1][..]]);
                    let solvev = egs.solve_one_in_bag(mover, t);
                    if refv.to_bits() != solvev.to_bits() {
                        disagreements += 1;
                        println!(
                            "DISAGREE [{name}] mover={mover} bag={t}: ref={refv} solve={solvev}\n   {}",
                            describe(&gc, &pos)
                        );
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
        let kwg = kwg::Kwg::<kwg::Node22>::from_bytes_alloc(&kwg_bytes);

        let board = empty_board();
        let racks = [vec![2u8, 2], vec![8u8, 20]];

        let mut egs = EndgameSolver::<kwg::Node22, kwg::Node22>::new(&gc, &kwg);
        egs.init(&board, [&racks[0][..], &racks[1][..]]);

        let v0 = egs.solve_one_in_bag(0, 2);
        assert_eq!(v0, -1000.0, "p0 forced-pass leftover should be -1000");

        let mut egs = EndgameSolver::<kwg::Node22, kwg::Node22>::new(&gc, &kwg);
        egs.init(&board, [&racks[0][..], &racks[1][..]]);
        let v1 = egs.solve_one_in_bag(1, 2);
        assert_eq!(v1, 1000.0, "p1 forced-pass leftover should be +1000");
    }

    #[test]
    #[inline]
    fn one_in_bag_play_then_draw() {
        let gc = game_config::make_english_game_config();
        let kwg_bytes = tiny_kwg_bytes();
        let kwg = kwg::Kwg::<kwg::Node22>::from_bytes_alloc(&kwg_bytes);
        let klv = klv::Klv::<kwg::Node22>::from_bytes_alloc(klv::EMPTY_KLV_BYTES);
        let mut mg = movegen::KurniaMoveGenerator::new(&gc);

        let board = empty_board();
        let racks = [vec![1u8, 2], vec![8u8, 20]]; // p0=[A,B] p1=[H,T]
        let bag = 1u8; // one A in the bag

        let refv = reference_one_in_bag(&gc, &kwg, &klv, &mut mg, &board, &racks, 0, bag, false);
        let mut egs = EndgameSolver::<kwg::Node22, kwg::Node22>::new(&gc, &kwg);
        egs.init(&board, [&racks[0][..], &racks[1][..]]);
        let solvev = egs.solve_one_in_bag(0, bag);
        assert_eq!(solvev.to_bits(), refv.to_bits(), "bag layer vs reference");

        let a = gc.alphabet();
        let pass_only = (a.scaled_rack_score(&racks[1]) - a.scaled_rack_score(&racks[0])) as f32;
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
        let kwg = kwg::Kwg::<kwg::Node22>::from_bytes_alloc(&kwg_bytes);
        let klv = klv::Klv::<kwg::Node22>::from_bytes_alloc(klv::EMPTY_KLV_BYTES);
        let mut mg = movegen::KurniaMoveGenerator::new(&gc);

        let bag_tiles: [u8; 2] = [1, 0]; // A, blank

        let mut disagreements = 0usize;
        let mut total = 0usize;

        let positions: Vec<(String, Position)> = embedded_positions()
            .into_iter()
            .filter(|(_, pos)| pos.racks[0].len() <= 2 && pos.racks[1].len() <= 2)
            .collect();
        for (name, pos) in positions {
            for &t in &bag_tiles {
                for mover in 0u8..2 {
                    total += 1;
                    let refv = reference_one_in_bag_ex(
                        &gc,
                        &kwg,
                        &klv,
                        &mut mg,
                        &pos.board,
                        &pos.racks,
                        mover as usize,
                        t,
                        0,
                        0,
                    );
                    let mut egs = EndgameSolver::<kwg::Node22, kwg::Node22>::new(&gc, &kwg);
                    egs.init(&pos.board, [&pos.racks[0][..], &pos.racks[1][..]]);
                    let solvev = egs.solve_one_in_bag(mover, t);
                    if refv.to_bits() != solvev.to_bits() {
                        disagreements += 1;
                        println!(
                            "DISAGREE [{name}] mover={mover} bag={t}: ref={refv} solve={solvev}\n   {}",
                            describe(&gc, &pos)
                        );
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
        let kwg = kwg::Kwg::<kwg::Node22>::from_bytes_alloc(&kwg_bytes);

        let board = empty_board();
        let racks = [vec![17u8], vec![1u8]]; // p0 = [Q], p1 = [A]
        let bag = 1u8; // one A in the bag

        let mut egs = EndgameSolver::<kwg::Node22, kwg::Node22>::new(&gc, &kwg);
        egs.init(&board, [&racks[0][..], &racks[1][..]]);
        let solvev = egs.solve_one_in_bag(0, bag);

        let a = gc.alphabet();
        let pass_only = (a.scaled_rack_score(&racks[1]) - a.scaled_rack_score(&racks[0])) as f32; // -9000
        assert_eq!(solvev, 0.0, "exchange line value should be exactly 0");
        assert!(
            solvev > pass_only,
            "exchange should beat passing: solve={solvev} pass_only={pass_only}"
        );
    }

    #[test]
    #[inline]
    fn peg_one_in_bag_exchange_solved_or_declined() {
        let kwg_bytes = tiny_kwg_bytes();
        let kwg = kwg::Kwg::<kwg::Node22>::from_bytes_alloc(&kwg_bytes);

        let gc = game_config::make_exchange_test_game_config();
        let board = empty_board();
        let mover_rack = [2u8]; // B, unplayable alone on an empty board
        let mut unseen_tally = vec![0u8; gc.alphabet().len() as usize];
        unseen_tally[8] = 1; // H
        unseen_tally[20] = 1; // T
        let mut egs = EndgameSolver::<kwg::Node22, kwg::Node22>::new(&gc, &kwg);
        let result = egs
            .solve_peg_one_in_bag(0, &board, &mover_rack, &unseen_tally, 0.0)
            .expect("the exchange test config forces an end and is solvable");
        assert_eq!(result.hypotheses.len(), 2);

        let gc_en = game_config::make_english_game_config();
        let mut egs_en = EndgameSolver::<kwg::Node22, kwg::Node22>::new(&gc_en, &kwg);
        assert!(
            egs_en
                .solve_peg_one_in_bag(0, &board, &mover_rack, &unseen_tally, 0.0)
                .is_ok()
        );

        let gc_bad = game_config::make_exchange_unsolvable_test_game_config();
        let mut egs_bad = EndgameSolver::<kwg::Node22, kwg::Node22>::new(&gc_bad, &kwg);
        assert!(matches!(
            egs_bad.solve_peg_one_in_bag(0, &board, &mover_rack, &unseen_tally, 0.0),
            Err(super::PegUnsupported::ExchangeWithoutForcedEnd)
        ));
    }

    #[test]
    #[inline]
    fn differential_peg_committed() {
        let gc = game_config::make_english_game_config();
        let kwg_bytes = tiny_kwg_bytes();
        let kwg = kwg::Kwg::<kwg::Node22>::from_bytes_alloc(&kwg_bytes);
        let klv = klv::Klv::<kwg::Node22>::from_bytes_alloc(klv::EMPTY_KLV_BYTES);
        let mut mg = movegen::KurniaMoveGenerator::new(&gc);
        let a = gc.alphabet();
        let alen = a.len() as usize;

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
                            *score as f32 - reference(&gc, &kwg, &klv, mg, &nb, &nr, opp, false)
                        }
                        _ => {
                            let nr = [mover_rack.to_vec(), opp_rack];
                            -reference_one_in_bag(&gc, &kwg, &klv, mg, board, &nr, opp, t, true)
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

                        let mut egs = EndgameSolver::<kwg::Node22, kwg::Node22>::new(&gc, &kwg);
                        let got = egs
                            .solve_peg_one_in_bag(0, board, mrack, &unseen_tally, 0.0)
                            .expect("english one-in-bag is always solvable");

                        let (ref_win, ref_marg, _ref_place, _ref_label) =
                            committed(board, mrack, &unseen, &mut mg);

                        let mut egs2 = EndgameSolver::<kwg::Node22, kwg::Node22>::new(&gc, &kwg);
                        let clair =
                            egs2.peg_clairvoyant_aggregate(0, board, mrack, &unseen_tally, 0.0);

                        assert!(got.committed, "english path must be committed");
                        assert!(got.best_move.is_some(), "committed result must name a move");
                        assert!(
                            (got.win_pct - ref_win).abs() < 1e-6,
                            "[{bname}] mover=[{}] unseen={unseen:?}: committed solver win% {} != reference {}",
                            a.fmt_rack(mrack),
                            got.win_pct,
                            ref_win,
                        );
                        assert!(
                            (got.expected_margin - ref_marg).abs() < 1e-1,
                            "[{bname}] mover=[{}] unseen={unseen:?}: committed solver margin {} != reference {}",
                            a.fmt_rack(mrack),
                            got.expected_margin,
                            ref_marg,
                        );
                        assert!(
                            got.win_pct <= clair.win_pct + 1e-6,
                            "[{bname}] mover=[{}] unseen={unseen:?}: committed win% {} exceeds clairvoyant bound {}",
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
                            if matches!(got.best_move, Some(PegMove::Place(_))) {
                                n_gap_place += 1;
                            }
                            if report.lines().count() < 25 {
                                report.push_str(&format!(
                                    "gap win%={gap:.3} [{bname}] mover=[{}] unseen={unseen:?}: clairvoyant {:.3} vs committed {:.3}\n",
                                    a.fmt_rack(mrack), clair.win_pct, got.win_pct,
                                ));
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
