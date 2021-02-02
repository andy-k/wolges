// Copyright (C) 2020-2021 Andy Kurnia.

// note: this module is very slow and may need a lot of space

use super::{build, game_config, klv, kwg, movegen};

// move one tile at a time from rack
#[derive(Clone, Eq, Hash, PartialEq)]
struct PlacedTile {
    tile: u8,  // 0x01-0x3f, 0x81-0xbf
    whose: u8, // 0 or 1
    idx: i16,
}

// canonical order of tile placements from start-of-endgame state
#[derive(Clone, Eq, Hash, PartialEq)]
struct State {
    parent: usize,
    placed_tile: PlacedTile,
}

// best move for a side
#[derive(Clone)]
pub struct StateSideEval {
    pub value: i16,
    pub play: movegen::Play,
}

impl StateSideEval {
    fn new() -> Self {
        Self {
            value: i16::MIN,
            play: movegen::Play::Exchange {
                tiles: [][..].into(),
            },
        }
    }
}

// best move for both sides
#[derive(Clone)]
pub struct StateEval {
    pub best: [StateSideEval; 2],
}

// per-ply
struct PlyBuffer {
    board_tiles: Vec<u8>,
    racks: [Vec<u8>; 2],
    movegen: movegen::KurniaMoveGenerator,
}

// reusable allocations
struct WorkBuffer {
    vec_placed_tile: Vec<PlacedTile>,
    ply_buffer: Vec<PlyBuffer>,
    states: Vec<State>,
    state_finder: build::MyHashMap<State, usize>,
    state_eval: build::MyHashMap<usize, StateEval>,
}

impl WorkBuffer {
    fn new() -> Self {
        Self {
            vec_placed_tile: Vec::new(),
            ply_buffer: Vec::new(),
            states: Vec::new(),
            state_finder: Default::default(),
            state_eval: Default::default(),
        }
    }

    fn init(&mut self) {
        self.vec_placed_tile.clear();
        // keep the preallocated ply_buffer
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
    }
}

// main two-player endgame solver
pub struct EndgameSolver<'a> {
    game_config: &'a game_config::GameConfig<'a>,
    kwg: &'a kwg::Kwg,
    klv: &'a klv::Klv,
    board_tiles: Vec<u8>,
    racks: [Vec<u8>; 2],
    work_buffer: WorkBuffer,
}

impl<'a> EndgameSolver<'a> {
    pub fn new(
        game_config: &'a game_config::GameConfig<'a>,
        kwg: &'a kwg::Kwg,
        klv: &'a klv::Klv,
    ) -> Self {
        if game_config.num_players() != 2 {
            panic!("cannot solve non-2-player endgames");
        }
        Self {
            game_config,
            kwg,
            klv,
            board_tiles: Vec::new(),
            racks: [Vec::new(), Vec::new()],
            work_buffer: WorkBuffer::new(),
        }
    }

    pub fn init(&mut self, board_tiles: &[u8], racks: [&[u8]; 2]) {
        self.board_tiles.clear();
        self.board_tiles.extend_from_slice(board_tiles);
        self.racks[0].clear();
        self.racks[0].extend_from_slice(racks[0]);
        self.racks[1].clear();
        self.racks[1].extend_from_slice(racks[1]);
        self.work_buffer.init();
    }

    fn get_new_pos_idx(&mut self, pos_idx: usize, which_player: u8, play: &movegen::Play) -> usize {
        match &play {
            movegen::Play::Exchange { .. } => pos_idx,
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
                    let mut pos_idx = pos_idx;
                    while pos_idx != 0 {
                        let state = &self.work_buffer.states[pos_idx];
                        self.work_buffer
                            .vec_placed_tile
                            .push(state.placed_tile.clone());
                        pos_idx = state.parent;
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

                // get the new pos_idx
                let mut new_pos_idx = 0;
                for placed_tile in self.work_buffer.vec_placed_tile.iter() {
                    let new_state = State {
                        parent: new_pos_idx,
                        placed_tile: placed_tile.clone(),
                    };
                    let new_new_pos_idx = self.work_buffer.states.len();
                    new_pos_idx = *self
                        .work_buffer
                        .state_finder
                        .entry(new_state.clone())
                        .or_insert(new_new_pos_idx);
                    if new_pos_idx == new_new_pos_idx {
                        self.work_buffer.states.push(new_state);
                    }
                }

                new_pos_idx
            }
        }
    }

    // cannot resolve the borrow checker conflict
    #[allow(clippy::map_entry)]
    pub fn solve(&mut self, pos_idx: usize) -> &StateEval {
        // borrow checker is preventing more efficient code
        if !self.work_buffer.state_eval.contains_key(&pos_idx) {
            // clone from base
            let mut current_ply_buffer =
                self.work_buffer
                    .ply_buffer
                    .pop()
                    .unwrap_or_else(|| PlyBuffer {
                        board_tiles: Vec::new(),
                        racks: [Vec::new(), Vec::new()],
                        movegen: movegen::KurniaMoveGenerator::new(self.game_config),
                    });
            current_ply_buffer.board_tiles.clear();
            current_ply_buffer
                .board_tiles
                .extend_from_slice(&self.board_tiles);
            current_ply_buffer.racks[0].clear();
            current_ply_buffer.racks[0].extend_from_slice(&self.racks[0]);
            current_ply_buffer.racks[1].clear();
            current_ply_buffer.racks[1].extend_from_slice(&self.racks[1]);

            // rebuild the state
            {
                let mut pos_idx = pos_idx;
                while pos_idx != 0 {
                    let state = &self.work_buffer.states[pos_idx];
                    current_ply_buffer.board_tiles[state.placed_tile.idx as usize] =
                        state.placed_tile.tile;
                    let rack = &mut current_ply_buffer.racks[state.placed_tile.whose as usize];
                    let blanked_tile =
                        state.placed_tile.tile & !((state.placed_tile.tile as i8) >> 7) as u8;
                    let tombstone_idx = rack.iter().rposition(|&t| t == blanked_tile).unwrap();
                    rack[tombstone_idx] = 0x80;
                    pos_idx = state.parent;
                }
                current_ply_buffer.racks[0].retain(|&t| t != 0x80);
                current_ply_buffer.racks[1].retain(|&t| t != 0x80);
            }
            let rack_scores = [
                self.game_config
                    .alphabet()
                    .rack_score(&current_ply_buffer.racks[0]),
                self.game_config
                    .alphabet()
                    .rack_score(&current_ply_buffer.racks[1]),
            ];

            let board_snapshot = movegen::BoardSnapshot {
                board_tiles: &current_ply_buffer.board_tiles,
                game_config: self.game_config,
                kwg: self.kwg,
                klv: self.klv,
            };

            /*
            println!(
                "position {} has racks {:?} and board",
                pos_idx, current_ply_buffer.racks
            );
            super::display::print_board(
                self.game_config.alphabet(),
                self.game_config.board_layout(),
                &current_ply_buffer.board_tiles,
            );
            */

            // figure out the best place move for each player
            let mut best_place_moves = [StateSideEval::new(), StateSideEval::new()];
            for which_player in 0..2 {
                // Pass is worth opponent's score minus own score
                best_place_moves[which_player].value =
                    rack_scores[which_player ^ 1] - rack_scores[which_player];
                current_ply_buffer.movegen.gen_all_raw_moves_unsorted(
                    &board_snapshot,
                    &current_ply_buffer.racks[which_player],
                );
                for candidate in &current_ply_buffer.movegen.plays {
                    if let movegen::Play::Place {
                        down: _,
                        lane: _,
                        idx: _,
                        word,
                        score,
                    } = &candidate.play
                    {
                        let value = if word.iter().filter(|&&t| t != 0).count()
                            == current_ply_buffer.racks[which_player].len()
                        {
                            // playing out
                            *score + 2 * rack_scores[which_player ^ 1]
                        } else {
                            // recursive case

                            let new_pos_idx =
                                self.get_new_pos_idx(pos_idx, which_player as u8, &candidate.play);

                            // valuation of this move is score minus opponent's best riposte
                            *score - self.solve(new_pos_idx).best[which_player ^ 1].value
                        };
                        if value > best_place_moves[which_player].value {
                            best_place_moves[which_player] = StateSideEval {
                                value,
                                play: candidate.play.clone(),
                            };
                        }
                    }
                }
            }

            let mut ret = StateEval {
                best: [StateSideEval::new(), StateSideEval::new()],
            };
            for which_player in 0..2 {
                let if_pass = -best_place_moves[which_player ^ 1].value;
                if if_pass > best_place_moves[which_player].value {
                    ret.best[which_player].value = if_pass;
                } else {
                    ret.best[which_player] = best_place_moves[which_player].clone();
                }
            }

            // keep the buffer for later reuse
            self.work_buffer.ply_buffer.push(current_ply_buffer);

            // memoize
            self.work_buffer.state_eval.insert(pos_idx, ret);
        }

        return self.work_buffer.state_eval.get(&pos_idx).unwrap();
    }

    // must have been precomputed
    pub fn append_solution(
        &mut self,
        mut pos_idx: usize,
        mut player_idx: u8,
        out: &mut Vec<StateSideEval>,
        racks: [&[u8]; 2],
    ) {
        let mut racks_len = [racks[0].len(), racks[1].len()];
        loop {
            let ans = self.work_buffer.state_eval.get(&pos_idx).unwrap();
            let mut ans1 = &ans.best[player_idx as usize];
            out.push(ans1.clone());
            if let movegen::Play::Exchange { .. } = ans1.play {
                player_idx ^= 1;
                ans1 = &ans.best[player_idx as usize];
                out.push(ans1.clone());
                if let movegen::Play::Exchange { .. } = ans1.play {
                    // both passed, done
                    break;
                }
            }

            if let movegen::Play::Place {
                down: _,
                lane: _,
                idx: _,
                word,
                score: _,
            } = &ans1.play
            {
                racks_len[player_idx as usize] -= word.iter().filter(|&&t| t != 0).count();
                if racks_len[player_idx as usize] == 0 {
                    // play out
                    return;
                }
            }

            let next_play = ans1.play.clone();
            let new_pos_idx = self.get_new_pos_idx(pos_idx, player_idx, &next_play);
            pos_idx = new_pos_idx;
            player_idx ^= 1;
        }
    }
}
