// Copyright (C) 2020-2021 Andy Kurnia.

// No longer actively used/maintained.

use super::super::{game_state, movegen};

pub struct ScoringChecker {
    rack_tally: Vec<u8>,
}

#[allow(dead_code)]
impl ScoringChecker {
    pub fn new() -> Self {
        Self {
            rack_tally: Vec::new(),
        }
    }

    pub fn check_scoring(
        &mut self,
        board_snapshot: &movegen::BoardSnapshot,
        game_state: &game_state::GameState,
        play: &movegen::Play,
        leave_scale: f32,
        movegen_equity: f32,
    ) {
        let game_config = board_snapshot.game_config;
        let klv = board_snapshot.klv;

        // manually recount and double-check the score and equity given by movegen
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

                print!("main word: (down={} lane={} idx={}) ", down, lane, idx);
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
                        print!(
                            "{} ({} * {} = {}), ",
                            alphabet.from_board(placed_tile).unwrap(),
                            face_value_tile_score,
                            tile_multiplier,
                            tile_score
                        );
                    }
                    let multiplied_word_score = word_score * word_multiplier as i16;
                    println!(
                        "for {} * {} = {}",
                        word_score, word_multiplier, multiplied_word_score
                    );
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
                        print!("perpendicular word: (down={} lane={} idx={}) ", !down, i, j);
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
                            print!(
                                "{} ({} * {} = {}), ",
                                alphabet.from_board(placed_tile).unwrap(),
                                face_value_tile_score,
                                tile_multiplier,
                                tile_score
                            );
                        }
                        let multiplied_word_score = word_score * word_multiplier as i16;
                        println!(
                            "for {} * {} = {}",
                            word_score, word_multiplier, multiplied_word_score
                        );
                        recounted_score += multiplied_word_score;
                    }
                }
                let num_played_bonus = game_config.num_played_bonus(num_played);
                println!(
                    "bonus for playing {} tiles: {}",
                    num_played, num_played_bonus
                );
                recounted_score += num_played_bonus;
            }
        };
        let movegen_score = match play {
            movegen::Play::Exchange { .. } => 0,
            movegen::Play::Place { score, .. } => *score,
        };
        println!(
            "recounted score = {}, difference = {}",
            recounted_score,
            movegen_score - recounted_score
        );
        assert_eq!(recounted_score, movegen_score);

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
        print!("leave: ");
        (0u8..)
            .zip(self.rack_tally.iter())
            .for_each(|(tile, &count)| {
                (0..count)
                    .for_each(|_| print!("{}", game_config.alphabet().from_rack(tile).unwrap()))
            });
        print!(" = ");
        let leave_value = klv.leave_value_from_tally(&self.rack_tally);
        println!("{}", leave_value);

        let mut recounted_equity = recounted_score as f32;
        if game_state.bag.0.is_empty() {
            // empty bag, do not add leave.
            println!("bag is empty");
            if self.rack_tally.iter().any(|&count| count != 0) {
                let kept_tiles_worth = (0u8..)
                    .zip(self.rack_tally.iter())
                    .map(|(tile, &count)| count as i16 * game_config.alphabet().score(tile) as i16)
                    .sum::<i16>();
                let kept_tiles_penalty = 10 + 2 * kept_tiles_worth;
                recounted_equity -= kept_tiles_penalty as f32;
                println!(
                    "kept tiles are worth {}, penalizing by {}: {}",
                    kept_tiles_worth, kept_tiles_penalty, recounted_equity
                );
            } else {
                println!("playing out");
                let mut unplayed_tiles_worth = 0;
                for (player_idx, player) in (0u8..).zip(game_state.players.iter()) {
                    if player_idx != game_state.turn {
                        let their_tile_worth = player
                            .rack
                            .iter()
                            .map(|&tile| game_config.alphabet().score(tile) as i16)
                            .sum::<i16>();
                        println!("p{} rack is worth {}", player_idx + 1, their_tile_worth);
                        unplayed_tiles_worth += their_tile_worth;
                    }
                }
                let unplayed_tiles_bonus = 2 * unplayed_tiles_worth;
                recounted_equity += unplayed_tiles_bonus as f32;
                println!(
                    "total worth {}, adding {}: {}",
                    unplayed_tiles_worth, unplayed_tiles_bonus, recounted_equity
                );
            }
        } else {
            recounted_equity += leave_scale * leave_value;
            println!("after adjusting for leave: {}", recounted_equity);
            if !game_state.board_tiles.iter().any(|&tile| tile != 0) {
                println!("nothing on board");
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
                        println!(
                            "dangerous vowel count {}, penalizing by {}: {}",
                            dangerous_vowel_count, dangerous_vowel_penalty, recounted_equity
                        );
                    }
                }
            }
        }
        println!(
            "recounted equity = {}, difference = {}",
            recounted_equity,
            movegen_equity - recounted_equity
        );
        assert_eq!(recounted_equity.to_le_bytes(), movegen_equity.to_le_bytes());
    }
}
