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
        Self {
            t0: std::time::Instant::now(),
            tick_periods: move_picker::Periods(0),
            dur_movegen: Default::default(),
            vec_placed_tile: Vec::new(),
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

    // iterative-deepening core shared by evaluate (verbose) and solve (quiet).
    // returns the final (deepest, converged) root valuation.
    fn run_id_loop(&mut self, player_idx: u8, verbose: bool) -> f32 {
        let mut last_valuation = f32::NAN;
        for max_depth in 1.. {
            let old_num_state_eval = self.work_buffer.state_eval.len();
            // reset ONCE per depth, before any aspiration re-search below, so
            // the final accepted search's depth-limit flag is what's observed.
            self.work_buffer.depth_limited = false;
            let valuation = if max_depth == 1 {
                // no previous value to aim at; search the full window.
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
            // stop once the search has fully resolved: no new states were
            // reached AND no line was cut short by the ply limit. Requiring
            // the latter avoids stopping while deep pass-then-play-out lines
            // are still truncated (they add plies but no new states).
            if self.work_buffer.state_eval.len() == old_num_state_eval
                && !self.work_buffer.depth_limited
            {
                break;
            }
        }
        last_valuation
    }

    // Aspiration window for one iterative-deepening depth. Each depth otherwise
    // re-searches the whole tree with the full (-inf, +inf) window; instead we
    // first search a narrow band around the previous depth's value, where the
    // answer almost always lands, letting alpha-beta prune far more. A fail-soft
    // result strictly inside the band is the exact value. If it falls on or
    // outside an edge (fail-low <= lo, or fail-high >= hi), that result is only a
    // bound, so we re-search once with the full window, which is always exact.
    // Same converged value as the full-window search, fewer nodes.
    fn aspiration_search(&mut self, player_idx: u8, max_depth: i8, last_valuation: f32) -> f32 {
        // narrow band half-width, in movegen's scaled unit (equity::SCALE=1000).
        const ASPIRATION_WINDOW: f32 = (3 * super::equity::SCALE) as f32;
        let lo = last_valuation - ASPIRATION_WINDOW;
        let hi = last_valuation + ASPIRATION_WINDOW;
        let v = self.negamax_eval(0, player_idx, max_depth, lo, hi, false);
        if v > lo && v < hi {
            // strictly inside the band: exact.
            v
        } else {
            // fail-low or fail-high: the narrow result is only a bound, so
            // re-search the full window once for the exact value.
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

    pub fn evaluate(&mut self, player_idx: u8) {
        self.run_id_loop(player_idx, true);
    }

    // headless entry point: returns the root valuation as data, with no
    // per-depth prints. "quiet" means no per-depth spam; the throttled
    // in-search tick can still fire on a multi-second search.
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
            // this line still had legal continuations but ran out of plies, so
            // its value here (0) is a placeholder. Record that this pass was
            // cut short so the iterative-deepening loop keeps going deeper
            // instead of mistaking the truncated value for a solved one. This
            // matters for lines that end in a run of passes (e.g. one side is
            // stuck and can only pass while the other plays out a blank several
            // plies later): those passes add depth without adding new states,
            // so a states-only stop would freeze on the truncated value.
            self.work_buffer.depth_limited = true;

            // static leaf value: the standing point margin if the game ended here
            // (both players pass), from the side-to-move's perspective. negamax keeps
            // every value from the mover's view and the caller composes it via
            // (score - v), so no explicit negation is needed. This is a meaningful
            // bound for a depth-capped search (aspiration) instead of a placeholder 0.
            return self.both_pass_value(state_idx, player_idx);
        }

        // return and/or trim range
        let alpha_orig = alpha;
        let state_eval = if let Some(state_eval) = self.work_buffer.state_eval.get(&state_idx) {
            let state_side_eval = &state_eval.best_move[player_idx as usize];
            if state_side_eval.depth >= depth {
                // invariant: a table-served (real-depth) best_move is always a PLACE
                // move, so its value does not depend on the consecutive-pass count.
                // pass-dependent values are stored with depth i8::MIN (see the
                // pass-store sites below) and never satisfy i8::MIN >= depth here.
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
                        true,
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
                            always_include_pass: false,
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

        // sort moves by equity desc
        let low_idx = state_eval.child_play_idxs[player_idx as usize];
        let high_idx = state_eval.child_play_idxs[player_idx as usize + 1];
        self.work_buffer.child_plays[low_idx..high_idx]
            .sort_unstable_by(|a, b| b.valuation.total_cmp(&a.valuation));

        // perform actual negamax
        let mut best_idx = !0;
        let mut best_valuation = f32::NEG_INFINITY;
        for child_play_idx in low_idx..high_idx {
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
                // i8::MIN keeps the single state_idx key correct w.r.t. the pass
                // count: this pass value depends on just_passed, so marking it
                // never-reusable (no real depth satisfies i8::MIN >= depth) stops
                // the TT from serving it. Do not "optimize" this depth away.
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
                // i8::MIN keeps the single state_idx key correct w.r.t. the pass
                // count: this pass value depends on just_passed, so marking it
                // never-reusable (no real depth satisfies i8::MIN >= depth) stops
                // the TT from serving it. Do not "optimize" this depth away.
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

    // collect the principal variation as owned data (each Play cloned) so the
    // caller can hold it past the solver borrow. out is caller-owned and reused.
    pub fn collect_pv(&'a self, player_idx: u8, out: &mut Vec<(f32, movegen::Play)>) {
        out.clear();
        self.append_solution(0, player_idx, |found| {
            out.push((found.equity, found.play.clone()));
        });
    }

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
}

// Reference-check tests for the endgame solver. A small self-contained gaddawg
// and a set of hand-built and fixed-seed positions drive an obviously-correct
// plain-negamax reference; the fast solver is asserted to agree with it, to
// reproduce its own principal-variation value on replay, and to return only
// legal lines. All values are in movegen's native scaled unit (equity::SCALE
// = 1000), so the reference and the solver share one consistent scale.
#[cfg(test)]
mod tests {
    use super::EndgameSolver;
    use crate::{alphabet, bites, build, game_config, klv, kwg, movegen};

    // ---- tiny gaddawg over a short English word list --------------------------
    // Two- to four-letter words drawn from the letters A, B, T, H so the racks
    // below can hook onto them. Kept intentionally small.
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
            // words with letters not in the alphabet reader would error; all of
            // the above use A/B/T/H/S/L which the english reader knows.
            if reader.set_word(w, &mut buf).is_ok() {
                words.push(buf[..].into());
            }
        }
        words.sort_unstable();
        words.dedup();
        words
    }

    fn tiny_kwg_bytes() -> bites::Bites {
        build::build(
            build::BuildContent::Gaddawg,
            build::BuildLayout::Wolges,
            &tiny_word_list(),
        )
        .unwrap()
    }

    // Apply a Place to a cloned board + the mover's rack, matching
    // print_best_line's apply logic exactly.
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

    // Obviously-correct plain negamax. Returns the game-theoretic endgame point
    // margin from `mover`'s perspective. NOTE: movegen's Play::Place.score is already
    // premultiplied by equity::SCALE (=1000); to be internally CONSISTENT the
    // reference expresses every term in that same scaled unit -- place score
    // scaled (native), play-out bonus 2*scaled_rack_score, both-pass leftover as
    // a scaled_rack_score differential. The fast solver values every term in the
    // same scaled unit, so it agrees with this reference term for term.
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
        mg.gen_moves_raw_all_unsorted(&snapshot, &racks[mover], 0, true);
        // collect this node's place moves before recursing (recursion reuses mg).
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
                    // playing out: game ends, mover empties the rack. Bonus in
                    // the SAME scaled unit as the play score.
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
            // both sides passed: leftover differential, scaled (matches the
            // scaled play scores and the solver's both_pass_value).
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

    // ---- position construction ------------------------------------------------
    struct Position {
        board: Vec<u8>,
        racks: [Vec<u8>; 2],
    }

    fn empty_board() -> Vec<u8> {
        vec![0u8; 15 * 15]
    }

    // place a word horizontally starting at (row, col0)
    fn put_word(board: &mut [u8], row: i8, col0: i8, word: &[u8]) {
        for (k, &t) in word.iter().enumerate() {
            board[(row as usize) * 15 + col0 as usize + k] = t;
        }
    }

    // human-readable rendering of a position
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

    // embedded, hand-built positions
    fn embedded_positions() -> Vec<(String, Position)> {
        let mut out = Vec::new();

        // (1) BOTH sides can only pass. Empty board; neither rack forms a listed
        // word. B=3,B=3 -> 6; H=4,T=1 -> 5. leftover from p0 = 5-6 = -1 raw.
        out.push((
            "both-pass-only (BB vs HT)".to_string(),
            Position {
                board: empty_board(),
                racks: [vec![2, 2], vec![8, 20]],
            },
        ));

        // (1b) both-pass with a nonzero symmetric-ish leftover the other way.
        out.push((
            "both-pass-only (BB vs BH)".to_string(),
            Position {
                board: empty_board(),
                racks: [vec![2, 2], vec![2, 8]],
            },
        ));

        // (2) One side must pass, other can act. Board has AT across center row 7
        // cols 6-7. p0 has [B] -> can hook (e.g. TAB/BAT/AB) ; p1 has [B,H] but
        // BB/BH not words alone off the AT? p1 can still hook. Provide a case
        // where p1 cannot move: p1 = [Q]? Q not in tiny alphabet moves. Use a
        // letter with no hooks: give p1 a single tile that forms nothing.
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

        // (3) play-out optimal: empty board, both have 2-tile playable racks.
        out.push((
            "playout race (AB vs AT)".to_string(),
            Position {
                board: empty_board(),
                racks: [vec![1, 2], vec![1, 20]],
            },
        ));

        // (3b) play-out with board present.
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

        // (4) larger-ish: 3-tile racks over a board word.
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

    // ---- fixed-seed random positions -----------------------------------------
    fn splitmix64(s: &mut u64) -> u64 {
        *s = s.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = *s;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

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
            // place one word horizontally on a random row, random start col.
            let w = &words[(splitmix64(&mut seed) as usize) % words.len()];
            let row = (splitmix64(&mut seed) % 15) as i8;
            let col0 = (splitmix64(&mut seed) % (15 - w.len() as u64)) as i8;
            put_word(&mut b, row, col0, w);
            // build two racks, size 2 (occasionally 3), from the letter set +
            // occasional blank (0).
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

    fn all_positions() -> Vec<(String, Position)> {
        let mut v = embedded_positions();
        v.extend(random_positions(200));
        v
    }

    // ---- the differential probe ----------------------------------------------
    #[test]
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

    // ---- PV-playout invariant -------------------------------------------------
    // Replay solve()'s principal variation and check the realized final value
    // equals solve()'s returned value. This is a self-consistency check, so the
    // replay mirrors the solver's own accounting exactly (scaled play score,
    // scaled play-out bonus, scaled both-pass leftover); it must reproduce the
    // returned value the solver computed.
    #[test]
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

            // replay
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

    // ---- properties -----------------------------------------------------------
    #[test]
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

            // value bound: |v| <= a generous scaled cap. Every solve value is in
            // the scaled unit (place score, doubled leftover bonus, and both-pass
            // leftover all scaled), so a scaled cap bounds it.
            let a = gc.alphabet();
            let board_pts: i32 = pos.board.iter().map(|&t| a.scaled_score(t)).sum();
            let rack_pts = a.scaled_rack_score(&pos.racks[0]) + a.scaled_rack_score(&pos.racks[1]);
            let cap = 4.0 * (board_pts + rack_pts) as f32 + 10000.0;
            if v.abs() > cap {
                oob += 1;
                println!("VALUE OOB [{name}]: v={v} cap={cap}");
            }

            // legality: replay, and at each place step re-generate and confirm
            // the played word/coords appear among generated place moves.
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
                        mg.gen_moves_raw_all_unsorted(&snapshot, &racks[mover], 0, true);
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
}
