// Copyright (C) 2020-2026 Andy Kurnia.

use super::{
    equity, error, game_config, game_state, jumbled_woadwage, kwg, move_filter, movegen, woadwage,
};

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
    #[inline(always)]
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
    #[inline(always)]
    pub fn validate_play<N: kwg::Node, L: kwg::Node>(
        &mut self,
        board_snapshot: &movegen::BoardSnapshot<'_, N, L>,
        game_state: &game_state::GameState,
        play: &movegen::Play,
    ) -> error::Returns<Option<movegen::Play>> {
        let game_config = board_snapshot.game_config;

        match play {
            movegen::Play::Exchange { tiles } => {
                if tiles.is_empty() {
                    return Ok(None);
                } else if game_state.bag.len() < usize::try_from(game_config.exchange_tile_limit())?
                {
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

                let (row, col) = if *down { (*idx, *lane) } else { (*lane, *idx) };
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

                if (*idx > 0 && board_snapshot.board_tiles[strider.at_in_lane(idx - 1)] != 0)
                    || (end_idx_exclusive < strider.len() as usize
                        && board_snapshot.board_tiles[strider.at_in_lane(end_idx_exclusive as i8)]
                            != 0)
                {
                    return_error!("word is not whole word".into());
                }

                let mut num_played = 0;
                let mut attaches_to_existing_tile = false;
                let mut transpose_idx = None;
                let alphabet = game_config.alphabet();
                let alphabet_len_without_blank = alphabet.len() - 1;
                self.set_rack_tally(game_config, &game_state.current_player().rack);
                let premiums = board_layout.premiums();
                for (i, &tile) in (*idx..).zip(word.iter()) {
                    let strider_at_i = strider.at_in_lane(i);
                    let board_tile = board_snapshot.board_tiles[strider_at_i];
                    if tile != 0 {
                        if board_tile != 0 {
                            return_error!("cannot place a tile onto an occupied square".into());
                        }
                        let premium = &premiums[strider_at_i];
                        if premium.word_multiplier == 0 && premium.tile_multiplier == 0 {
                            return_error!("cannot place a tile onto a punctured square".into());
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
                                && board_snapshot.board_tiles
                                    [perpendicular_strider.at_in_lane(*lane - 1)]
                                    != 0)
                                || (*lane + 1 < perpendicular_strider.len()
                                    && board_snapshot.board_tiles
                                        [perpendicular_strider.at_in_lane(*lane + 1)]
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

                if num_played == 1
                    && let Some(i) = transpose_idx
                {
                    // force single-tile plays to be in preferred direction
                    let perpendicular_strider = dim.lane(!*down, i);
                    let mut j = *lane;
                    while j > 0
                        && board_snapshot.board_tiles[perpendicular_strider.at_in_lane(j - 1)] != 0
                    {
                        j -= 1;
                    }
                    let perpendicular_strider_len = perpendicular_strider.len();
                    let mut k = *lane + 1;
                    while k < perpendicular_strider_len
                        && board_snapshot.board_tiles[perpendicular_strider.at_in_lane(k)] != 0
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

                Ok(None)
            }
        }
    }

    #[inline(always)]
    pub fn words_all<Callback: FnMut(&[u8]) -> bool, N: kwg::Node, L: kwg::Node>(
        &mut self,
        board_snapshot: &movegen::BoardSnapshot<'_, N, L>,
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
    fn classic_words_checker<'a, N: kwg::Node, L: kwg::Node>(
        &mut self,
        board_snapshot: &'a movegen::BoardSnapshot<'_, N, L>,
    ) -> impl FnMut(&[u8]) -> bool + 'a + use<'a, N, L> {
        move |word: &[u8]| {
            let mut p = 0;
            for &tile in word {
                p = board_snapshot.kwg.seek(p, tile);
                if p <= 0 {
                    return false;
                }
            }
            board_snapshot.kwg.node_in_run(p).accepts()
        }
    }

    #[inline(always)]
    fn woadwage_words_checker<'a, N: kwg::Node, L: kwg::Node>(
        &mut self,
        board_snapshot: &'a movegen::BoardSnapshot<'_, N, L>,
    ) -> impl FnMut(&[u8]) -> bool + 'a + use<'a, N, L> {
        move |word: &[u8]| woadwage::accepts(board_snapshot.kwg, word)
    }

    #[inline(always)]
    fn jumbled_woadwage_words_checker<'a, N: kwg::Node, L: kwg::Node>(
        &mut self,
        board_snapshot: &'a movegen::BoardSnapshot<'_, N, L>,
    ) -> impl FnMut(&[u8]) -> bool + 'a + use<'a, N, L> {
        move |word: &[u8]| jumbled_woadwage::accepts(board_snapshot.kwg, word)
    }

    #[inline(always)]
    fn jumbled_words_checker<'a, N: kwg::Node, L: kwg::Node>(
        &mut self,
        board_snapshot: &'a movegen::BoardSnapshot<'_, N, L>,
    ) -> impl FnMut(&[u8]) -> bool + 'a + use<'a, N, L> {
        move |word: &[u8]| {
            // doing this the slow way, with no additional space
            let mut p = 0;
            let mut current_min_tile = word.iter().min();
            while let Some(&current_tile) = current_min_tile {
                for &tile in word {
                    if tile == current_tile {
                        p = board_snapshot.kwg.seek(p, tile);
                        if p <= 0 {
                            return false;
                        }
                    }
                }
                current_min_tile = word.iter().filter(|&&tile| tile > current_tile).min();
            }
            board_snapshot.kwg.node_in_run(p).accepts()
        }
    }

    #[inline(always)]
    pub fn words_are_valid<N: kwg::Node, L: kwg::Node>(
        &mut self,
        board_snapshot: &movegen::BoardSnapshot<'_, N, L>,
        play: &movegen::Play,
    ) -> bool {
        match board_snapshot.game_config.game_rules() {
            game_config::GameRules::Classic => {
                let checker = self.classic_words_checker(board_snapshot);
                self.words_all(board_snapshot, play, checker)
            }
            game_config::GameRules::Woadwage => {
                let checker = self.woadwage_words_checker(board_snapshot);
                self.words_all(board_snapshot, play, checker)
            }
            game_config::GameRules::JumbledWoadwage => {
                let checker = self.jumbled_woadwage_words_checker(board_snapshot);
                self.words_all(board_snapshot, play, checker)
            }
            game_config::GameRules::Jumbled => {
                let checker = self.jumbled_words_checker(board_snapshot);
                self.words_all(board_snapshot, play, checker)
            }
        }
    }

    #[inline(always)]
    fn collect_invalid_words<'a>(
        &mut self,
        mut checker: impl FnMut(&[u8]) -> bool + 'a,
        mut save_invalid_word: impl FnMut(&[u8]) + 'a,
    ) -> impl FnMut(&[u8]) -> bool + 'a {
        move |word: &[u8]| {
            if !checker(word) {
                save_invalid_word(word);
            }
            true // always continue iterating
        }
    }

    #[inline(always)]
    pub fn find_invalid_words<N: kwg::Node, L: kwg::Node>(
        &mut self,
        board_snapshot: &movegen::BoardSnapshot<'_, N, L>,
        play: &movegen::Play,
        save_invalid_word: impl FnMut(&[u8]),
    ) {
        match board_snapshot.game_config.game_rules() {
            game_config::GameRules::Classic => {
                let checker = self.classic_words_checker(board_snapshot);
                let checker = self.collect_invalid_words(checker, save_invalid_word);
                self.words_all(board_snapshot, play, checker)
            }
            game_config::GameRules::Woadwage => {
                let checker = self.woadwage_words_checker(board_snapshot);
                let checker = self.collect_invalid_words(checker, save_invalid_word);
                self.words_all(board_snapshot, play, checker)
            }
            game_config::GameRules::JumbledWoadwage => {
                let checker = self.jumbled_woadwage_words_checker(board_snapshot);
                let checker = self.collect_invalid_words(checker, save_invalid_word);
                self.words_all(board_snapshot, play, checker)
            }
            game_config::GameRules::Jumbled => {
                let checker = self.jumbled_words_checker(board_snapshot);
                let checker = self.collect_invalid_words(checker, save_invalid_word);
                self.words_all(board_snapshot, play, checker)
            }
        };
    }

    // Unused &mut self for future-proofing.
    // Assume play is valid.
    #[inline(always)]
    pub fn compute_score<N: kwg::Node, L: kwg::Node>(
        &mut self,
        board_snapshot: &movegen::BoardSnapshot<'_, N, L>,
        play: &movegen::Play,
    ) -> i32 {
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
                    let mut word_score = 0i32;
                    for (i, &tile) in (*idx..).zip(word.iter()) {
                        let strider_at_i = strider.at_in_lane(i);
                        let tile_multiplier;
                        let premium = &premiums[strider_at_i];
                        let placed_tile = if tile != 0 {
                            num_played += 1;
                            word_multiplier *= i32::from(premium.word_multiplier);
                            tile_multiplier = premium.tile_multiplier;
                            tile
                        } else {
                            tile_multiplier = 1;
                            board_snapshot.board_tiles[strider_at_i]
                        };
                        let face_value_tile_score = alphabet.scaled_score(placed_tile).raw();
                        let tile_score = face_value_tile_score * i32::from(tile_multiplier);
                        word_score += tile_score;
                    }
                    let multiplied_word_score = word_score * word_multiplier;
                    recounted_score += multiplied_word_score;
                }

                for (i, &tile) in (*idx..).zip(word.iter()) {
                    if tile != 0 {
                        let perpendicular_strider = dim.lane(!*down, i);
                        let mut j = *lane;
                        while j > 0
                            && board_snapshot.board_tiles[perpendicular_strider.at_in_lane(j - 1)]
                                != 0
                        {
                            j -= 1;
                        }
                        let perpendicular_strider_len = perpendicular_strider.len();
                        if j == *lane
                            && if j + 1 < perpendicular_strider_len {
                                board_snapshot.board_tiles[perpendicular_strider.at_in_lane(j + 1)]
                                    == 0
                            } else {
                                true
                            }
                        {
                            continue;
                        }
                        let mut word_multiplier = 1;
                        let mut word_score = 0i32;
                        for j in j..perpendicular_strider_len {
                            let perpendicular_strider_at_j = perpendicular_strider.at_in_lane(j);
                            let tile_multiplier;
                            let premium = &premiums[perpendicular_strider_at_j];
                            let placed_tile = if j == *lane {
                                word_multiplier *= i32::from(premium.word_multiplier);
                                tile_multiplier = premium.tile_multiplier;
                                tile
                            } else {
                                tile_multiplier = 1;
                                board_snapshot.board_tiles[perpendicular_strider_at_j]
                            };
                            if placed_tile == 0 {
                                break;
                            }
                            let face_value_tile_score = alphabet.scaled_score(placed_tile).raw();
                            let tile_score = face_value_tile_score * i32::from(tile_multiplier);
                            word_score += tile_score;
                        }
                        let multiplied_word_score = word_score * word_multiplier;
                        recounted_score += multiplied_word_score;
                    }
                }
                // playing more tiles than a rack holds (from an overdrawn rack)
                // earns a full rack's bonus
                let num_played_bonus =
                    game_config.num_played_bonus(num_played.min(game_config.rack_size()));
                recounted_score += i32::from(num_played_bonus) * equity::SCALE;
            }
        };

        recounted_score
    }

    // Assume recounted_score came from compute_score().
    #[inline(always)]
    pub fn compute_equity<N: kwg::Node, L: kwg::Node>(
        &mut self,
        board_snapshot: &movegen::BoardSnapshot<'_, N, L>,
        game_state: &game_state::GameState,
        play: &movegen::Play,
        leave_scale: i32,
        recounted_score: i32,
    ) -> error::Returns<i32> {
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

        let mut recounted_equity = recounted_score;
        if game_state.bag.is_empty() {
            // empty bag, do not add leave.
            if self.rack_tally.iter().any(|&count| count != 0) {
                let kept_tiles_worth = (0u8..)
                    .zip(self.rack_tally.iter())
                    .map(|(tile, &count)| {
                        i32::from(count) * i32::from(game_config.alphabet().score(tile))
                    })
                    .sum::<i32>();
                let kept_tiles_penalty = 10 + 2 * kept_tiles_worth;
                recounted_equity -= kept_tiles_penalty * equity::SCALE;
            } else {
                let mut unplayed_tiles_worth = 0;
                for (player_idx, player) in (0u8..).zip(game_state.players.iter()) {
                    if player_idx != game_state.turn {
                        let their_tile_worth = player
                            .rack
                            .iter()
                            .map(|&tile| i32::from(game_config.alphabet().score(tile)))
                            .sum::<i32>();
                        unplayed_tiles_worth += their_tile_worth;
                    }
                }
                let unplayed_tiles_bonus = 2 * unplayed_tiles_worth;
                recounted_equity += unplayed_tiles_bonus * equity::SCALE;
            }
        } else {
            let leave_value = board_snapshot
                .klv
                .leave_value_from_tally(&self.rack_tally)
                .raw();
            let adjusted = i32::try_from(
                i64::from(leave_value) * i64::from(leave_scale)
                    / i64::from(move_filter::LEAVE_SCALE_DENOM),
            )?;
            recounted_equity = recounted_equity
                .checked_add(adjusted)
                .ok_or("score plus adjusted leave is out of range")?;
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
                            .filter(|&(ref i, &tile)| {
                                tile != 0 && alphabet.is_vowel(tile) && {
                                    (match strider1 {
                                        Some(ref strider) => {
                                            let premium = &premiums[strider.at_in_lane(*i)];
                                            premium.tile_multiplier != 1
                                                || premium.word_multiplier != 1
                                        }
                                        None => false,
                                    }) || (match strider2 {
                                        Some(ref strider) => {
                                            let premium = &premiums[strider.at_in_lane(*i)];
                                            premium.tile_multiplier != 1
                                                || premium.word_multiplier != 1
                                        }
                                        None => false,
                                    })
                                }
                            })
                            .count();
                        let dangerous_vowel_penalty =
                            dangerous_vowel_count as i32 * equity::OPENING_HOTSPOT_PENALTY;
                        recounted_equity -= dangerous_vowel_penalty;
                    }
                }
            }
        }

        Ok(recounted_equity)
    }
}

impl Default for PlayScorer {
    #[inline(always)]
    fn default() -> Self {
        Self::new()
    }
}

#[inline(always)]
pub fn score_and_blank_deltas<N: kwg::Node, L: kwg::Node>(
    board_snapshot: &movegen::BoardSnapshot<'_, N, L>,
    down: bool,
    lane: i8,
    idx: i8,
    word: &[u8],
    out: &mut Vec<(u8, i32)>,
) -> i32 {
    let game_config = board_snapshot.game_config;
    let alphabet = game_config.alphabet();
    let board_layout = game_config.board_layout();
    let premiums = board_layout.premiums();
    let dim = board_layout.dim();
    let strider = dim.lane(down, lane);
    let blank_scaled = alphabet.scaled_score(0).raw();
    out.clear();

    let mut recounted_score = 0;
    let mut num_played = 0;

    {
        let mut word_multiplier = 1;
        let mut word_score = 0i32;
        for (i, &tile) in (idx..).zip(word.iter()) {
            let strider_at_i = strider.at_in_lane(i);
            let tile_multiplier;
            let premium = &premiums[strider_at_i];
            let placed_tile = if tile != 0 {
                num_played += 1;
                word_multiplier *= i32::from(premium.word_multiplier);
                tile_multiplier = premium.tile_multiplier;

                let letter = tile & 0x7f;
                out.push((
                    letter,
                    (alphabet.scaled_score(letter).raw() - blank_scaled)
                        * i32::from(tile_multiplier),
                ));
                letter
            } else {
                tile_multiplier = 1;
                board_snapshot.board_tiles[strider_at_i]
            };
            let face_value_tile_score = alphabet.scaled_score(placed_tile).raw();
            word_score += face_value_tile_score * i32::from(tile_multiplier);
        }
        recounted_score += word_score * word_multiplier;

        for e in out.iter_mut() {
            e.1 *= word_multiplier;
        }
    }

    let mut k = 0;
    for (i, &tile) in (idx..).zip(word.iter()) {
        if tile != 0 {
            let letter = tile & 0x7f;
            let perpendicular_strider = dim.lane(!down, i);
            let mut j = lane;
            while j > 0 && board_snapshot.board_tiles[perpendicular_strider.at_in_lane(j - 1)] != 0
            {
                j -= 1;
            }
            let perpendicular_strider_len = perpendicular_strider.len();
            if j == lane
                && if j + 1 < perpendicular_strider_len {
                    board_snapshot.board_tiles[perpendicular_strider.at_in_lane(j + 1)] == 0
                } else {
                    true
                }
            {
                k += 1;
                continue;
            }
            let mut word_multiplier = 1;
            let mut word_score = 0i32;

            let mut placed_tile_multiplier = 1i32;
            for j in j..perpendicular_strider_len {
                let perpendicular_strider_at_j = perpendicular_strider.at_in_lane(j);
                let tile_multiplier;
                let premium = &premiums[perpendicular_strider_at_j];
                let placed_tile = if j == lane {
                    word_multiplier *= i32::from(premium.word_multiplier);
                    tile_multiplier = premium.tile_multiplier;
                    placed_tile_multiplier = i32::from(premium.tile_multiplier);
                    letter
                } else {
                    tile_multiplier = 1;
                    board_snapshot.board_tiles[perpendicular_strider_at_j]
                };
                if placed_tile == 0 {
                    break;
                }
                let face_value_tile_score = alphabet.scaled_score(placed_tile).raw();
                word_score += face_value_tile_score * i32::from(tile_multiplier);
            }
            recounted_score += word_score * word_multiplier;
            out[k].1 += (alphabet.scaled_score(letter).raw() - blank_scaled)
                * placed_tile_multiplier
                * word_multiplier;
            k += 1;
        }
    }

    let num_played_bonus = game_config.num_played_bonus(num_played);
    recounted_score += i32::from(num_played_bonus) * equity::SCALE;

    recounted_score
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{bites, board_layout, build, klv, matrix};
    use std::collections::BTreeSet;

    #[test]
    #[inline]
    fn a_tile_onto_a_punctured_square_is_refused() {
        let game_config = game_config::make_punctured_english_game_config();
        let words: [bites::Bites; 1] = [[1u8, 20][..].into()];
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
        let mut game_state = game_state::GameState::new(&game_config);
        game_state.players[0].rack = vec![1, 20];
        let board_snapshot = movegen::BoardSnapshot {
            alphagrams: None,
            board_tiles: &game_state.board_tiles,
            game_config: &game_config,
            kwg: &kwg,
            anagrams: None,
            rack_lengths: None,
            klv: &klv,
        };
        let at = |idx: i8| movegen::Play::Place {
            down: false,
            lane: 7,
            idx,
            word: [1, 20][..].into(),
            score: equity::Equity::ZERO,
        };
        let mut play_scorer = PlayScorer::new();
        // F8 is a hole on this board; G8 and H8 take the star
        assert_eq!(
            play_scorer
                .validate_play(&board_snapshot, &game_state, &at(4))
                .err()
                .unwrap()
                .to_string(),
            "cannot place a tile onto a punctured square"
        );
        assert!(
            play_scorer
                .validate_play(&board_snapshot, &game_state, &at(6))
                .is_ok()
        );
    }

    #[test]
    #[inline]
    fn a_jumbled_woadwage_play_takes_a_jumble_or_two_joined_in_every_word_it_forms() {
        let words = [vec![4u8, 1], vec![2, 3], vec![4, 1, 2, 3]];
        let explicit = jumbled_woadwage::tests::explicit_variant(&words);
        let kad = jumbled_woadwage::tests::kad_for(&words);
        let klv = klv::kbwg_from_bytes_alloc(klv::EMPTY_KLV_BYTES.to_vec()).unwrap();
        let premiums = vec![
            board_layout::Premium {
                word_multiplier: 1,
                tile_multiplier: 1,
            };
            49
        ];
        let game_config =
            game_config::make_board_test_game_config(board_layout::make_test_board_layout(
                premiums.into_boxed_slice(),
                matrix::Dim { rows: 7, cols: 7 },
                3,
                3,
            ))
            .jumbled_woadwage();
        let mut board_tiles = vec![0u8; 49];
        board_tiles[2 * 7 + 3] = 4;
        board_tiles[4 * 7 + 3] = 2;
        let board_snapshot = movegen::BoardSnapshot {
            alphagrams: None,
            board_tiles: &board_tiles,
            game_config: &game_config,
            kwg: &kad,
            anagrams: None,
            rack_lengths: None,
            klv: &klv,
        };
        let mut play_scorer = PlayScorer::new();
        let mut cases = 0;
        let mut valid = 0;
        let mut strings = vec![Vec::new()];
        for _ in 0..5 {
            strings = strings
                .iter()
                .flat_map(|s: &Vec<u8>| (1..=4).map(move |t| [s.as_slice(), &[t]].concat()))
                .collect();
            for word in &strings {
                if word.len() >= 2 {
                    let play = movegen::Play::Place {
                        down: false,
                        lane: 0,
                        idx: 0,
                        word: word[..].into(),
                        score: equity::Equity::ZERO,
                    };
                    let is_valid = play_scorer.words_are_valid(&board_snapshot, &play);
                    assert_eq!(is_valid, explicit.contains(word), "{word:?}");
                    valid += is_valid as usize;
                }
                if word.len() <= 4 {
                    let play = movegen::Play::Place {
                        down: false,
                        lane: 3,
                        idx: 3,
                        word: word[..].into(),
                        score: equity::Equity::ZERO,
                    };
                    let cross = [4, word[0], 2];
                    let mut invalid = Vec::new();
                    play_scorer.find_invalid_words(&board_snapshot, &play, |w: &[u8]| {
                        invalid.push(w.to_vec())
                    });
                    let mut expected = Vec::new();
                    if !explicit.contains(word) {
                        expected.push(word.clone());
                    }
                    if !explicit.contains(&cross[..]) {
                        expected.push(cross.to_vec());
                    }
                    invalid.sort();
                    expected.sort();
                    assert_eq!(invalid, expected, "{word:?}");
                    assert_eq!(
                        play_scorer.words_are_valid(&board_snapshot, &play),
                        expected.is_empty(),
                        "{word:?}"
                    );
                }
                cases += 1;
            }
        }
        assert_eq!(cases, 4 + 16 + 64 + 256 + 1024);
        assert!(valid > 20, "{valid}");
    }

    #[test]
    #[inline]
    fn a_woadwage_play_takes_a_word_or_two_joined_words_in_every_word_it_forms() {
        let words = [vec![1u8], vec![1, 1], vec![1, 2], vec![2, 3], vec![3, 2, 1]];
        let explicit = words
            .iter()
            .cloned()
            .chain(
                words
                    .iter()
                    .flat_map(|a| words.iter().map(|b| [a.as_slice(), b.as_slice()].concat())),
            )
            .collect::<BTreeSet<_>>();
        let machine_words = words
            .iter()
            .map(|word| bites::Bites::from(word.as_slice()))
            .collect::<Vec<_>>();
        let kwg = kwg::kbwg_from_bytes_alloc(
            build::build(
                build::BuildContent::Gaddawg,
                build::BuildLayout::Wolges,
                build::BuildOrder::Sorted,
                &machine_words,
            )
            .unwrap()
            .to_vec(),
            false,
        )
        .unwrap();
        let klv = klv::kbwg_from_bytes_alloc(klv::EMPTY_KLV_BYTES.to_vec()).unwrap();
        let premiums = vec![
            board_layout::Premium {
                word_multiplier: 1,
                tile_multiplier: 1,
            };
            49
        ];
        let game_config =
            game_config::make_board_test_game_config(board_layout::make_test_board_layout(
                premiums.into_boxed_slice(),
                matrix::Dim { rows: 7, cols: 7 },
                3,
                3,
            ))
            .woadwage();
        let mut board_tiles = vec![0u8; 49];
        board_tiles[2 * 7 + 3] = 1;
        board_tiles[4 * 7 + 3] = 2;
        let board_snapshot = movegen::BoardSnapshot {
            alphagrams: None,
            board_tiles: &board_tiles,
            game_config: &game_config,
            kwg: &kwg,
            anagrams: None,
            rack_lengths: None,
            klv: &klv,
        };
        let mut play_scorer = PlayScorer::new();
        let mut cases = 0;
        let mut strings = vec![Vec::new()];
        for _ in 0..5 {
            strings = strings
                .iter()
                .flat_map(|s: &Vec<u8>| (1..=4).map(move |t| [s.as_slice(), &[t]].concat()))
                .collect();
            for word in &strings {
                if word.len() >= 2 {
                    let play = movegen::Play::Place {
                        down: false,
                        lane: 0,
                        idx: 0,
                        word: word[..].into(),
                        score: equity::Equity::ZERO,
                    };
                    assert_eq!(
                        play_scorer.words_are_valid(&board_snapshot, &play),
                        explicit.contains(word),
                        "{word:?}"
                    );
                }
                if word.len() <= 4 {
                    let play = movegen::Play::Place {
                        down: false,
                        lane: 3,
                        idx: 3,
                        word: word[..].into(),
                        score: equity::Equity::ZERO,
                    };
                    let cross = [1, word[0], 2];
                    let mut invalid = Vec::new();
                    play_scorer.find_invalid_words(&board_snapshot, &play, |w: &[u8]| {
                        invalid.push(w.to_vec())
                    });
                    let mut expected = Vec::new();
                    if !explicit.contains(word) {
                        expected.push(word.clone());
                    }
                    if !explicit.contains(&cross[..]) {
                        expected.push(cross.to_vec());
                    }
                    invalid.sort();
                    expected.sort();
                    assert_eq!(invalid, expected, "{word:?}");
                    assert_eq!(
                        play_scorer.words_are_valid(&board_snapshot, &play),
                        expected.is_empty(),
                        "{word:?}"
                    );
                }
                cases += 1;
            }
        }
        assert_eq!(cases, 4 + 16 + 64 + 256 + 1024);
    }
}
