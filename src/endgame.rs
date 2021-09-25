// Copyright (C) 2020-2021 Andy Kurnia.

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

// per-ply
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
    num_movegen: usize,
    vec_placed_tile: Vec<PlacedTile>,
    current_ply_buffer: PlyBuffer, // only using one
    movegen: movegen::KurniaMoveGenerator,
    blocked: Box<[[i16; 4]]>,                  // r*c, 4 directions
    vec_blocked: Vec<i16>,                     // up to 5*7
    states: Vec<State>,                        // [0] = dummy initial state, excludes play outs
    state_finder: fash::MyHashMap<State, u32>, // maps all states except 0
    state_eval: fash::MyHashMap<u32, StateEval>,
    plays: Vec<movegen::Play>, // global u32->Play mapping. [0] = pass, [1..] = place
    play_finder: fash::MyHashMap<movegen::Play, u32>, // maps all plays except pass
    child_plays: Vec<ChildPlay>, // subslices of StateEval, often re-sorted; excludes pass
}

impl WorkBuffer {
    fn new(game_config: &game_config::GameConfig<'_>) -> Self {
        let dim = game_config.board_layout().dim();
        let rows_times_cols = (dim.rows as isize * dim.cols as isize) as usize;
        Self {
            t0: std::time::Instant::now(),
            tick_periods: move_picker::Periods(0),
            dur_movegen: Default::default(),
            num_movegen: 0,
            vec_placed_tile: Vec::new(),
            current_ply_buffer: PlyBuffer {
                board_tiles: Vec::new(),
                racks: [Vec::new(), Vec::new()],
            },
            movegen: movegen::KurniaMoveGenerator::new(game_config),
            blocked: vec![[0; 4]; rows_times_cols].into_boxed_slice(),
            vec_blocked: Vec::new(),
            states: Vec::new(),
            state_finder: Default::default(),
            state_eval: Default::default(),
            plays: Vec::new(),
            play_finder: fash::MyHashMap::default(),
            child_plays: Vec::new(),
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
pub struct EndgameSolver<'a> {
    game_config: &'a game_config::GameConfig<'a>,
    kwg: &'a kwg::Kwg,
    klv: Box<klv::Klv>,
    board_tiles: Vec<u8>,
    racks: [Vec<u8>; 2],
    rack_scores: [i16; 2],
    work_buffer: WorkBuffer,
}

fn move_score(play: &movegen::Play) -> i16 {
    match play {
        movegen::Play::Exchange { .. } => 0,
        movegen::Play::Place { score, .. } => *score,
    }
}

impl<'a> EndgameSolver<'a> {
    pub fn new(game_config: &'a game_config::GameConfig<'a>, kwg: &'a kwg::Kwg) -> Self {
        if game_config.num_players() != 2 {
            panic!("cannot solve non-2-player endgames");
        }
        Self {
            game_config,
            kwg,
            klv: Box::new(klv::Klv::from_bytes_alloc(klv::EMPTY_KLV_BYTES)),
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
        self.rack_scores[0] = self.game_config.alphabet().rack_score(racks[0]);
        self.rack_scores[1] = self.game_config.alphabet().rack_score(racks[1]);
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
            rack_scores[state.placed_tile.whose as usize] -= alphabet.score(blanked_tile) as i16;
            state_idx = state.parent;
        }
        (rack_scores[player_idx as usize ^ 1] - rack_scores[player_idx as usize]) as f32
    }

    pub fn evaluate(&mut self, player_idx: u8) {
        for max_depth in 1.. {
            let old_num_states = self.work_buffer.states.len();
            let valuation = self.negamax_eval(
                0,
                player_idx,
                max_depth,
                f32::NEG_INFINITY,
                f32::INFINITY,
                false,
            );
            println!("valuation for depth {} is {}", max_depth, valuation);
            self.print_progress();
            self.print_best_line(player_idx);
            // check for time limit here
            if self.work_buffer.states.len() == old_num_states {
                break;
            }
        }
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
            return self.both_pass_value(state_idx, player_idx);
        }

        // expand no-pass side first
        let pass_valuation = if just_passed {
            self.both_pass_value(state_idx, player_idx)
        } else {
            -self.negamax_eval(state_idx, player_idx ^ 1, depth, -beta, -alpha, true)
        };

        // return and/or trim range
        let alpha_orig = alpha;
        let beta_orig = beta;
        if let Some(state_eval) = self.work_buffer.state_eval.get(&state_idx) {
            let state_side_eval = &state_eval.best_move[player_idx as usize];
            if state_side_eval.depth >= depth {
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
                alphabet.rack_score(&current_ply_buffer.racks[0]),
                alphabet.rack_score(&current_ply_buffer.racks[1]),
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

            // rework blocked array (left, right, up, down)
            let dim = self.game_config.board_layout().dim();
            for row in 0..dim.rows {
                let strider = dim.across(row);
                let strider_len = strider.len();
                let mut last_empty = strider.at(0) as i16;
                for i in 0..strider_len {
                    let here = strider.at(i);
                    self.work_buffer.blocked[here][0] = last_empty;
                    if current_ply_buffer.board_tiles[here] == 0 {
                        last_empty = here as i16;
                    }
                }
                last_empty = strider.at(strider_len - 1) as i16;
                for i in (0..strider_len).rev() {
                    let here = strider.at(i);
                    self.work_buffer.blocked[here][1] = last_empty;
                    if current_ply_buffer.board_tiles[here] == 0 {
                        last_empty = here as i16;
                    }
                }
            }
            for col in 0..dim.cols {
                let strider = dim.down(col);
                let strider_len = strider.len();
                let mut last_empty = strider.at(0) as i16;
                for i in 0..strider_len {
                    let here = strider.at(i);
                    self.work_buffer.blocked[here][2] = last_empty;
                    if current_ply_buffer.board_tiles[here] == 0 {
                        last_empty = here as i16;
                    }
                }
                last_empty = strider.at(strider_len - 1) as i16;
                for i in (0..strider_len).rev() {
                    let here = strider.at(i);
                    self.work_buffer.blocked[here][3] = last_empty;
                    if current_ply_buffer.board_tiles[here] == 0 {
                        last_empty = here as i16;
                    }
                }
            }

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
                self.work_buffer.movegen.gen_all_raw_moves_unsorted(
                    &board_snapshot,
                    &current_ply_buffer.racks[which_player],
                    true,
                );
                self.work_buffer.dur_movegen += t1.elapsed();
                self.work_buffer.num_movegen += 1;
                for candidate in &self.work_buffer.movegen.plays {
                    match &candidate.play {
                        movegen::Play::Exchange { .. } => {
                            // no need to store pass explicitly
                        }
                        movegen::Play::Place { .. } => {
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
                            self.work_buffer.child_plays.push(ChildPlay {
                                new_state_idx: !0, // filled in later
                                play_idx: new_play_idx,
                                valuation: 0.0, // filled in later
                            });
                        }
                    }
                }
                state_eval.child_play_idxs[which_player as usize + 1] =
                    self.work_buffer.child_plays.len();
            }

            // sort both sets of moves by score descending
            let both_child_play_ranges = &mut self.work_buffer.child_plays
                [state_eval.child_play_idxs[0]..state_eval.child_play_idxs[2]];
            let (p0_child_plays, p1_child_plays) = both_child_play_ranges
                .split_at_mut(state_eval.child_play_idxs[1] - state_eval.child_play_idxs[0]);
            {
                let plays = std::mem::take(&mut self.work_buffer.plays);
                p0_child_plays.sort_unstable_by(|a, b| {
                    move_score(&plays[b.play_idx as usize])
                        .cmp(&move_score(&plays[a.play_idx as usize]))
                });
                p1_child_plays.sort_unstable_by(|a, b| {
                    move_score(&plays[b.play_idx as usize])
                        .cmp(&move_score(&plays[a.play_idx as usize]))
                });
                self.work_buffer.plays = plays;
            }

            // fill in move equities
            let mut px_child_plays = [p0_child_plays, p1_child_plays];
            {
                let mut vec_blocked = std::mem::take(&mut self.work_buffer.vec_blocked);
                for which_player in 0..2 {
                    let my_child_plays = std::mem::take(&mut px_child_plays[which_player]);
                    let oppo_child_plays = std::mem::take(&mut px_child_plays[which_player ^ 1]);
                    for child_play in my_child_plays.iter_mut() {
                        match &self.work_buffer.plays[child_play.play_idx as usize] {
                            movegen::Play::Exchange { .. } => {
                                unreachable!();
                            }
                            movegen::Play::Place {
                                down,
                                lane,
                                idx,
                                word,
                                score,
                            } => {
                                if word.iter().filter(|&&t| t != 0).count()
                                    == current_ply_buffer.racks[which_player].len()
                                {
                                    // playing out
                                    child_play.new_state_idx = 0;
                                    child_play.valuation =
                                        (*score + 2 * rack_scores[which_player ^ 1]) as f32;
                                } else {
                                    // determine affected squares
                                    vec_blocked.clear();
                                    let strider = dim.lane(*down, *lane);
                                    for (i, &tile) in (*idx..).zip(word.iter()) {
                                        if tile != 0 {
                                            let there = strider.at(i);
                                            vec_blocked.push(there as i16);
                                            vec_blocked.extend_from_slice(
                                                &self.work_buffer.blocked[there],
                                            );
                                        }
                                    }

                                    // find the best unblocked oppo move's score
                                    // (slow if top moves share the same squares)
                                    let mut best_unblocked_oppo_score = 0;
                                    for oppo_child_play in oppo_child_plays.iter() {
                                        match &self.work_buffer.plays
                                            [oppo_child_play.play_idx as usize]
                                        {
                                            movegen::Play::Exchange { .. } => {
                                                break;
                                            }
                                            movegen::Play::Place {
                                                down,
                                                lane,
                                                idx,
                                                word,
                                                score,
                                            } => {
                                                let strider = dim.lane(*down, *lane);
                                                let is_blocked =
                                                    (*idx..).zip(word.iter()).any(|(i, &tile)| {
                                                        tile != 0
                                                            && vec_blocked
                                                                .contains(&(strider.at(i) as i16))
                                                    });
                                                if !is_blocked {
                                                    best_unblocked_oppo_score = *score;
                                                    break;
                                                }
                                            }
                                        }
                                    }
                                    child_play.valuation =
                                        (*score - best_unblocked_oppo_score) as f32;
                                }
                            }
                        }
                    }
                    px_child_plays[which_player] = my_child_plays;
                    px_child_plays[which_player ^ 1] = oppo_child_plays;
                }
                self.work_buffer.vec_blocked = vec_blocked;
            }

            self.work_buffer.state_eval.insert(state_idx, state_eval);
        }

        // sort moves by equity desc
        let state_eval = self.work_buffer.state_eval.get(&state_idx).unwrap();
        let low_idx = state_eval.child_play_idxs[player_idx as usize];
        let high_idx = state_eval.child_play_idxs[player_idx as usize + 1];
        self.work_buffer.child_plays[low_idx..high_idx]
            .sort_unstable_by(|a, b| b.valuation.partial_cmp(&a.valuation).unwrap());

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
                            score
                                - self.negamax_eval(
                                    self.work_buffer.child_plays[child_play_idx].new_state_idx,
                                    player_idx ^ 1,
                                    depth - 1,
                                    -beta,
                                    -alpha,
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
                            if alpha >= beta {
                                break;
                            }
                        }
                    }
                }
            };
        }

        // fill in best_place_move
        let mut state_eval = self.work_buffer.state_eval.get_mut(&state_idx).unwrap();
        if best_idx == !0 {
            // no valid place moves exist, must pass
            state_eval.best_place_move[player_idx as usize] = StateSideEval {
                equity: pass_valuation,
                play_idx: 0,
                new_state_idx: state_idx,
                equity_type: StateSideEvalEquityType::Exact,
                depth,
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
                depth,
            };
            best_valuation = pass_valuation;
        } else {
            state_eval.best_move[player_idx as usize] =
                state_eval.best_place_move[player_idx as usize].clone();
        }

        if !just_passed {
            // the initial player has just_passed=false.
            // to the initial player, the following have been evaluated:
            // - A = the opponent's best_place_move,
            // - B = the opponent's best_move,
            // - C = the player's best_place_move,
            // - D = the player's best_move.
            // let's assume A and C are correct.
            // let P = opponent's rack minus player's rack.
            // the goal is to have
            // - player's best_move = max(C, -max(A, -P)).
            // - opponent's best_move = max(A, -max(C, P)).
            // at this point, B = max(A, -P), so it's incorrect.
            // but D = max(C, -B), so it's correct.
            // so, set opponent's best_move to E.
            // let E = max(A, -D).
            // so E = max(A, -max(C, -max(A, -P))).
            // if -P >= A, this is trivially correct.
            // and if A > -P, it becomes max(A, -max(C, -A)),
            // which is max(A, min(-C, A)), which is A.

            if -best_valuation > state_eval.best_place_move[player_idx as usize ^ 1].equity {
                // -best_valuation within -beta_orig..-alpha_orig.
                state_eval.best_move[player_idx as usize ^ 1] = StateSideEval {
                    equity: -best_valuation,
                    play_idx: 0,
                    new_state_idx: state_idx,
                    equity_type: if beta_orig <= best_valuation {
                        StateSideEvalEquityType::UpperBound
                    } else if alpha_orig >= best_valuation {
                        StateSideEvalEquityType::LowerBound
                    } else {
                        StateSideEvalEquityType::Exact
                    },
                    depth,
                };
            } else {
                state_eval.best_move[player_idx as usize ^ 1] =
                    state_eval.best_place_move[player_idx as usize ^ 1].clone();
            }
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

    // allocates a lot of things that are not reused
    pub fn print_best_line(&self, player_idx: u8) {
        let mut soln = Vec::new(); // allocates
        let mut latest_board_tiles = self.board_tiles.clone(); // this allocates and is not reused
        self.append_solution(0, player_idx, |x| soln.push(x));
        let mut racks = self.racks.clone();
        for (i, ply) in soln.iter().enumerate() {
            let player_turn_idx = (player_idx as usize + i) % 2;
            let rack = &mut racks[player_turn_idx];
            println!(
                "{}: p{}: {:<width$} {} {}",
                i,
                player_turn_idx,
                format!("{}", self.game_config.alphabet().fmt_rack(rack)),
                ply.equity,
                ply.play.fmt(&movegen::BoardSnapshot {
                    board_tiles: &latest_board_tiles,
                    game_config: self.game_config,
                    kwg: self.kwg,
                    klv: &self.klv,
                }),
                width = self.game_config.rack_size() as usize,
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
                            latest_board_tiles[strider.at(i)] = tile;
                            let blanked_tile = tile & !((tile as i8) >> 7) as u8;
                            let tombstone_idx =
                                rack.iter().rposition(|&t| t == blanked_tile).unwrap();
                            rack[tombstone_idx] = 0x80;
                        }
                    }
                    rack.retain(|&t| t != 0x80);
                }
            }
        }
        for i in soln.len()..std::cmp::max(soln.len() + 1, 2) {
            let player_turn_idx = (player_idx as usize + i) % 2;
            let rack = &racks[player_turn_idx];
            println!(
                "e: p{}: {}",
                player_turn_idx,
                self.game_config.alphabet().fmt_rack(rack),
            );
        }
        display::print_board(
            self.game_config.alphabet(),
            self.game_config.board_layout(),
            &latest_board_tiles,
        );
    }

    fn print_progress(&self) {
        let dur0 = self.work_buffer.t0.elapsed();
        let dur1 = self.work_buffer.dur_movegen;
        println!(
            "after {:?} ({:?} on {} movegen), there are {} states, {} evaluated, {} child_plays, {} plays",
            dur0,
            dur1,
            self.work_buffer.num_movegen,
            self.work_buffer.states.len(),
            self.work_buffer.state_eval.len(),
            self.work_buffer.child_plays.len(),
            self.work_buffer.plays.len(),
        );
    }
}
