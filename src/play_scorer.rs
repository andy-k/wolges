// Copyright (C) 2020-2021 Andy Kurnia.

use super::{game_state, movegen};

pub struct PlayScorer {
    rack_tally: Vec<u8>,
}

impl PlayScorer {
    pub fn new() -> Self {
        Self {
            rack_tally: Vec::new(),
        }
    }

    pub fn play_is_valid(
        &mut self,
        board_snapshot: &movegen::BoardSnapshot,
        play: &movegen::Play,
    ) -> bool {
        // TODO
        // also maybe check if it forms any invalid word
        let _ = board_snapshot;
        let _ = play;
        true
    }

    // Unused &mut self for future-proofing.
    // Assume play is valid.
    pub fn compute_score(
        &mut self,
        board_snapshot: &movegen::BoardSnapshot,
        play: &movegen::Play,
    ) -> i16 {
        let game_config = board_snapshot.game_config;

        let mut recounted_score = 0;
        match &play {
            movegen::Play::Exchange { .. } => {}
            movegen::Play::Place {
                down,
                lane,
                idx,
                word,
                ..
            } => {
                let alphabet = game_config.alphabet();
                let board_layout = game_config.board_layout();
                let premiums = board_layout.premiums();
                let dim = board_layout.dim();
                let strider = dim.lane(*down, *lane);
                let mut num_played = 0;

                {
                    let mut word_multiplier = 1;
                    let mut word_score = 0i16;
                    for (i, &tile) in (*idx..).zip(word.iter()) {
                        let strider_at_i = strider.at(i);
                        let tile_multiplier;
                        let premium = premiums[strider_at_i];
                        let placed_tile = if tile != 0 {
                            num_played += 1;
                            word_multiplier *= premium.word_multiplier;
                            tile_multiplier = premium.tile_multiplier;
                            tile
                        } else {
                            tile_multiplier = 1;
                            board_snapshot.board_tiles[strider_at_i]
                        };
                        let face_value_tile_score = alphabet.score(placed_tile);
                        let tile_score = face_value_tile_score as i16 * tile_multiplier as i16;
                        word_score += tile_score;
                    }
                    let multiplied_word_score = word_score * word_multiplier as i16;
                    recounted_score += multiplied_word_score;
                }

                for (i, &tile) in (*idx..).zip(word.iter()) {
                    if tile != 0 {
                        let perpendicular_strider = dim.lane(!*down, i);
                        let mut j = *lane;
                        while j > 0
                            && board_snapshot.board_tiles[perpendicular_strider.at(j - 1)] != 0
                        {
                            j -= 1;
                        }
                        let perpendicular_strider_len = perpendicular_strider.len();
                        if j == *lane
                            && if j + 1 < perpendicular_strider_len {
                                board_snapshot.board_tiles[perpendicular_strider.at(j + 1)] == 0
                            } else {
                                true
                            }
                        {
                            // no perpendicular tile
                            continue;
                        }
                        let mut word_multiplier = 1;
                        let mut word_score = 0i16;
                        for j in j..perpendicular_strider_len {
                            let perpendicular_strider_at_j = perpendicular_strider.at(j);
                            let tile_multiplier;
                            let premium = premiums[perpendicular_strider_at_j];
                            let placed_tile = if j == *lane {
                                word_multiplier *= premium.word_multiplier;
                                tile_multiplier = premium.tile_multiplier;
                                tile
                            } else {
                                tile_multiplier = 1;
                                board_snapshot.board_tiles[perpendicular_strider_at_j]
                            };
                            if placed_tile == 0 {
                                break;
                            }
                            let face_value_tile_score = alphabet.score(placed_tile);
                            let tile_score = face_value_tile_score as i16 * tile_multiplier as i16;
                            word_score += tile_score;
                        }
                        let multiplied_word_score = word_score * word_multiplier as i16;
                        recounted_score += multiplied_word_score;
                    }
                }
                let num_played_bonus = game_config.num_played_bonus(num_played);
                recounted_score += num_played_bonus;
            }
        };

        recounted_score
    }

    // Assume recounted_score came from compute_score().
    pub fn compute_equity(
        &mut self,
        board_snapshot: &movegen::BoardSnapshot,
        game_state: &game_state::GameState,
        play: &movegen::Play,
        leave_scale: f32,
        recounted_score: i16,
    ) -> f32 {
        let game_config = board_snapshot.game_config;
        let klv = board_snapshot.klv;

        self.rack_tally.clear();
        self.rack_tally
            .resize(game_config.alphabet().len() as usize, 0);
        game_state
            .current_player()
            .rack
            .iter()
            .for_each(|&tile| self.rack_tally[tile as usize] += 1);
        match play {
            movegen::Play::Exchange { tiles } => {
                tiles
                    .iter()
                    .for_each(|&tile| self.rack_tally[tile as usize] -= 1);
            }
            movegen::Play::Place { word, .. } => {
                word.iter().for_each(|&tile| {
                    if tile & 0x80 != 0 {
                        self.rack_tally[0] -= 1;
                    } else if tile != 0 {
                        self.rack_tally[tile as usize] -= 1;
                    }
                });
            }
        };
        let leave_value = klv.leave_value_from_tally(&self.rack_tally);

        let mut recounted_equity = recounted_score as f32;
        if game_state.bag.0.is_empty() {
            // empty bag, do not add leave.
            if self.rack_tally.iter().any(|&count| count != 0) {
                let kept_tiles_worth = (0u8..)
                    .zip(self.rack_tally.iter())
                    .map(|(tile, &count)| count as i16 * game_config.alphabet().score(tile) as i16)
                    .sum::<i16>();
                let kept_tiles_penalty = 10 + 2 * kept_tiles_worth;
                recounted_equity -= kept_tiles_penalty as f32;
            } else {
                let mut unplayed_tiles_worth = 0;
                for (player_idx, player) in (0u8..).zip(game_state.players.iter()) {
                    if player_idx != game_state.turn {
                        let their_tile_worth = player
                            .rack
                            .iter()
                            .map(|&tile| game_config.alphabet().score(tile) as i16)
                            .sum::<i16>();
                        unplayed_tiles_worth += their_tile_worth;
                    }
                }
                let unplayed_tiles_bonus = 2 * unplayed_tiles_worth;
                recounted_equity += unplayed_tiles_bonus as f32;
            }
        } else {
            recounted_equity += leave_scale * leave_value;
            if !game_state.board_tiles.iter().any(|&tile| tile != 0) {
                match play {
                    movegen::Play::Exchange { .. } => {}
                    movegen::Play::Place {
                        down,
                        lane,
                        idx,
                        word,
                        ..
                    } => {
                        let alphabet = game_config.alphabet();
                        let board_layout = game_config.board_layout();
                        let premiums = board_layout.premiums();
                        let dim = board_layout.dim();
                        // check board_layout's danger_star precomputation
                        let num_lanes = if *down { dim.cols } else { dim.rows };
                        let strider1 = if *lane > 0 {
                            Some(dim.lane(*down, *lane - 1))
                        } else {
                            None
                        };
                        let strider2 = if *lane < num_lanes - 1 {
                            Some(dim.lane(*down, *lane + 1))
                        } else {
                            None
                        };
                        let dangerous_vowel_count = (*idx..)
                            .zip(word.iter())
                            .filter(|(i, &tile)| {
                                tile != 0 && alphabet.is_vowel(tile) && {
                                    (match strider1 {
                                        Some(strider) => {
                                            let premium = premiums[strider.at(*i)];
                                            premium.tile_multiplier != 1
                                                || premium.word_multiplier != 1
                                        }
                                        None => false,
                                    }) || (match strider2 {
                                        Some(strider) => {
                                            let premium = premiums[strider.at(*i)];
                                            premium.tile_multiplier != 1
                                                || premium.word_multiplier != 1
                                        }
                                        None => false,
                                    })
                                }
                            })
                            .count();
                        let dangerous_vowel_penalty = dangerous_vowel_count as f32 * 0.7;
                        recounted_equity -= dangerous_vowel_penalty as f32;
                    }
                }
            }
        }

        recounted_equity
    }
}

impl Default for PlayScorer {
    fn default() -> Self {
        Self::new()
    }
}
