// Copyright (C) 2020-2021 Andy Kurnia.

use super::{error, game_config, game_state, move_filter, movegen};

pub struct PlayScorer {
    rack_tally: Vec<u8>,
    word_iter: move_filter::LimitedVocabChecker,
}

impl PlayScorer {
    pub fn new() -> Self {
        Self {
            rack_tally: Vec::new(),
            word_iter: move_filter::LimitedVocabChecker::new(),
        }
    }

    // Does not validate rack, may crash if invalid tile.
    fn set_rack_tally(&mut self, game_config: &game_config::GameConfig, rack: &[u8]) {
        self.rack_tally.clear();
        self.rack_tally
            .resize(game_config.alphabet().len() as usize, 0);
        rack.iter()
            .for_each(|&tile| self.rack_tally[tile as usize] += 1);
    }

    // Ok(None) if valid and canonical.
    // Ok(Some(canonical_play)) if valid but not canonical.
    // Err(reason) if invalid.
    pub fn validate_play(
        &mut self,
        board_snapshot: &movegen::BoardSnapshot,
        game_state: &game_state::GameState,
        play: &movegen::Play,
    ) -> error::Returns<Option<movegen::Play>> {
        let game_config = board_snapshot.game_config;

        let ret = match play {
            movegen::Play::Exchange { tiles } => {
                if tiles.is_empty() {
                    return Ok(None);
                } else if game_state.bag.0.len() < game_config.rack_size() as usize {
                    return_error!("not enough tiles to allow exchanges".into());
                }

                let alphabet = game_config.alphabet();
                let alphabet_len_without_blank = alphabet.len() - 1;
                if !tiles.iter().all(|&tile| tile <= alphabet_len_without_blank) {
                    return_error!("exchanged tile not in alphabet".into());
                }

                self.set_rack_tally(game_config, &game_state.current_player().rack);
                let can_consummate_tiles = tiles.iter().all(|&tile| {
                    if self.rack_tally[tile as usize] > 0 {
                        self.rack_tally[tile as usize] -= 1;
                        true
                    } else {
                        false
                    }
                });
                if !can_consummate_tiles {
                    return_error!("cannot exchange a tile not on rack".into());
                }

                if tiles
                    .iter()
                    .zip(&tiles[1..])
                    .all(|(&tile0, &tile1)| tile0 <= tile1)
                {
                    Ok(None)
                } else {
                    let mut sorted_tiles = tiles.to_vec();
                    sorted_tiles.sort_unstable();
                    Ok(Some(movegen::Play::Exchange {
                        tiles: sorted_tiles[..].into(),
                    }))
                }
            }
            movegen::Play::Place {
                down,
                lane,
                idx,
                word,
                score,
            } => {
                let word_len = word.len();
                if word_len < 2 {
                    return_error!("word is too short".into());
                }

                let (row, col) = if *down { (*lane, *idx) } else { (*idx, *lane) };
                let board_layout = game_config.board_layout();
                let dim = board_layout.dim();
                if row < 0 || col < 0 || row >= dim.rows || col >= dim.cols {
                    return_error!("invalid coordinates".into());
                }

                let strider = dim.lane(*down, *lane);
                let end_idx_exclusive = *idx as usize + word_len;
                if end_idx_exclusive > strider.len() as usize {
                    return_error!("word extends out of board".into());
                }

                if (*idx > 0 && board_snapshot.board_tiles[strider.at(idx - 1)] != 0)
                    || (end_idx_exclusive < strider.len() as usize
                        && board_snapshot.board_tiles[strider.at(end_idx_exclusive as i8)] != 0)
                {
                    return_error!("word is not whole word".into());
                }

                let mut num_played = 0;
                let mut attaches_to_existing_tile = false;
                let mut transpose_idx = None;
                let alphabet = game_config.alphabet();
                let alphabet_len_without_blank = alphabet.len() - 1;
                self.set_rack_tally(game_config, &game_state.current_player().rack);
                for (i, &tile) in (*idx..).zip(word.iter()) {
                    let board_tile = board_snapshot.board_tiles[strider.at(i)];
                    if tile != 0 {
                        if board_tile != 0 {
                            return_error!("cannot place a tile onto an occupied square".into());
                        }
                        if tile & 0x7f > alphabet_len_without_blank || tile == 0x80 {
                            return_error!("placed tile not in alphabet".into());
                        }
                        let placed_tile = tile & !((tile as i8) >> 7) as u8;
                        if self.rack_tally[placed_tile as usize] > 0 {
                            self.rack_tally[placed_tile as usize] -= 1;
                        } else {
                            return_error!("cannot place a tile not on rack".into());
                        }
                        let may_transpose = *down && num_played == 0;
                        num_played += 1;
                        if !attaches_to_existing_tile || may_transpose {
                            let perpendicular_strider = dim.lane(!*down, i);
                            if (*lane > 0
                                && board_snapshot.board_tiles[perpendicular_strider.at(*lane - 1)]
                                    != 0)
                                || (*lane + 1 < perpendicular_strider.len()
                                    && board_snapshot.board_tiles
                                        [perpendicular_strider.at(*lane + 1)]
                                        != 0)
                            {
                                if may_transpose {
                                    transpose_idx = Some(i);
                                }
                                attaches_to_existing_tile = true;
                            }
                        }
                    } else {
                        if board_tile == 0 {
                            return_error!("cannot place nothing onto an empty square".into());
                        }
                        attaches_to_existing_tile = true;
                    }
                }
                if num_played == 0 {
                    return_error!("word does not place a new tile".into());
                }

                if !game_state.board_tiles.iter().any(|&tile| tile != 0) {
                    if !if *down {
                        *lane == board_layout.star_col() && {
                            let star_idx = board_layout.star_row();
                            *idx <= star_idx && star_idx < end_idx_exclusive as i8
                        }
                    } else {
                        *lane == board_layout.star_row() && {
                            let star_idx = board_layout.star_col();
                            *idx <= star_idx && star_idx < end_idx_exclusive as i8
                        }
                    } {
                        return_error!("word does not cover starting square".into());
                    }
                } else if !attaches_to_existing_tile {
                    return_error!("word is detached from existing tiles".into());
                }

                if num_played == 1 {
                    if let Some(i) = transpose_idx {
                        // force single-tile plays to be in preferred direction
                        let perpendicular_strider = dim.lane(!*down, i);
                        let mut j = *lane;
                        while j > 0
                            && board_snapshot.board_tiles[perpendicular_strider.at(j - 1)] != 0
                        {
                            j -= 1;
                        }
                        let perpendicular_strider_len = perpendicular_strider.len();
                        let mut k = *lane + 1;
                        while k < perpendicular_strider_len
                            && board_snapshot.board_tiles[perpendicular_strider.at(k)] != 0
                        {
                            k += 1;
                        }
                        let mut transposed_word = vec![0u8; (k - j) as usize];
                        transposed_word[(*lane - j) as usize] = word[(i - *idx) as usize];
                        return Ok(Some(movegen::Play::Place {
                            down: !*down,
                            lane: i,
                            idx: j,
                            word: transposed_word[..].into(),
                            score: *score,
                        }));
                    }
                }

                Ok(None)
            }
        };
        // no-op, just to silence unused warning
        if ret.is_ok() {}
        ret
    }

    pub fn words_all<Callback: FnMut(&[u8]) -> bool>(
        &mut self,
        board_snapshot: &movegen::BoardSnapshot,
        play: &movegen::Play,
        cb: Callback,
    ) -> bool {
        match &play {
            movegen::Play::Exchange { .. } => true,
            movegen::Play::Place {
                down,
                lane,
                idx,
                word,
                ..
            } => self.word_iter.words_placed_are_ok(
                board_snapshot,
                *down,
                *lane,
                *idx,
                &word[..],
                cb,
            ),
        }
    }

    #[inline(always)]
    pub fn words_are_valid(
        &mut self,
        board_snapshot: &movegen::BoardSnapshot,
        play: &movegen::Play,
    ) -> bool {
        self.words_all(board_snapshot, play, |word: &[u8]| {
            let mut p = 0;
            for &tile in word {
                p = board_snapshot.kwg.seek(p, tile);
                if p <= 0 {
                    return false;
                }
            }
            true
        })
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

        self.set_rack_tally(game_config, &game_state.current_player().rack);
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
        let leave_value = board_snapshot.klv.leave_value_from_tally(&self.rack_tally);

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
