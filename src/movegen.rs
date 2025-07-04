// Copyright (C) 2020-2025 Andy Kurnia.

use super::{alphabet, bites, display, game_config, klv, kwg, matrix};

#[derive(Clone)]
struct CrossSet {
    bits: u64,
    score: i32,
}

#[derive(Clone)]
struct CachedCrossSet {
    p_left: i32,
    p_right: i32,
    bits: u64,
}

#[derive(Clone)]
struct CrossSetComputation {
    score: i32,
    b_letter: u8,
    end_range: i8,
    p: i32,
}

#[derive(Clone)]
struct PossiblePlacement {
    down: bool,
    lane: i8,
    anchor: i8,
    leftmost: i8,
    rightmost: i8,
    best_possible_equity: f32,
}

#[derive(Clone)]
struct MultiJump {
    left_score: i32,
    right_score: i32,
    left_idx: i8,
    right_idx: i8,
}

// WorkingBuffer can only be reused for the same game_config and kwg.
// (The kwg is partially cached in cached_cross_set.)
// WorkingBuffer can also be reset for reuse with another kwg by calling
// reset_for_another_kwg().
// This is not enforced.
struct WorkingBuffer {
    rack_tally: Box<[u8]>,                                      // 27 for ?A-Z
    word_buffer_for_across_plays: Box<[u8]>,                    // r*c
    word_buffer_for_down_plays: Box<[u8]>,                      // c*r
    cross_set_for_across_plays: Box<[CrossSet]>,                // r*c
    cross_set_for_down_plays: Box<[CrossSet]>,                  // c*r
    cached_cross_set_for_across_plays: Box<[CachedCrossSet]>,   // c*r
    cached_cross_set_for_down_plays: Box<[CachedCrossSet]>,     // r*c
    cross_set_buffer: Box<[CrossSetComputation]>,               // max(r, c)
    remaining_word_multipliers_for_across_plays: Box<[i8]>,     // r*c (1 if tile placed)
    remaining_word_multipliers_for_down_plays: Box<[i8]>,       // c*r
    remaining_tile_multipliers_for_across_plays: Box<[i8]>,     // r*c (1 if tile placed)
    remaining_tile_multipliers_for_down_plays: Box<[i8]>,       // c*r
    face_value_scores_for_across_plays: Box<[i8]>,              // r*c
    face_value_scores_for_down_plays: Box<[i8]>,                // c*r
    perpendicular_word_multipliers_for_across_plays: Box<[i8]>, // r*c (0 if no perpendicularly adjacent tile)
    perpendicular_word_multipliers_for_down_plays: Box<[i8]>,   // c*r
    perpendicular_scores_for_across_plays: Box<[i32]>, // r*c (multiplied by perpendicular_word_multipliers)
    perpendicular_scores_for_down_plays: Box<[i32]>,   // c*r
    transposed_board_tiles: Box<[u8]>,                 // c*r
    num_tiles_on_board: u16,
    num_tiles_in_bag: i16, // negative when players also have less than full racks
    num_tiles_on_rack: u8,
    rack_bits: u64, // bit 0 = blank conveniently matches bit 0 = have cross set
    multi_leaves: klv::MultiLeaves,
    descending_scores: Vec<i8>,            // rack.len()
    exchange_buffer: Vec<u8>,              // rack.len(), or max(word length) with word prune
    aggregated_word_multipliers: Vec<i32>, // sorted, unique, O(n) insertion but n is tiny.
    precomputed_square_multiplier_buffer: Vec<i32>,
    indexes_to_descending_square_multiplier_buffer: Vec<i8>,
    multi_jumps_buffer: Box<[MultiJump]>, // max(r, c)
    best_leave_values: Vec<f32>,          // rack.len() + 1
    found_placements: Vec<PossiblePlacement>,
    used_letters_tally: Vec<u8>, // 27 for ?A-Z, ? is always 0, jumbled mode only
    used_tile_scores_shadowl: Vec<i8>, // rack.len() (for shadow_play_left)
    used_tile_scores_shadowr: Vec<i8>, // rack.len() (for shadow_play_right)
    rack_tally_shadowl: Box<[u8]>, // 27 for ?A-Z (for shadow_play_left)
    rack_tally_shadowr: Box<[u8]>, // 27 for ?A-Z (for shadow_play_right)
}

impl Clone for WorkingBuffer {
    #[inline(always)]
    fn clone(&self) -> Self {
        Self {
            rack_tally: self.rack_tally.clone(),
            word_buffer_for_across_plays: self.word_buffer_for_across_plays.clone(),
            word_buffer_for_down_plays: self.word_buffer_for_down_plays.clone(),
            cross_set_for_across_plays: self.cross_set_for_across_plays.clone(),
            cross_set_for_down_plays: self.cross_set_for_down_plays.clone(),
            cached_cross_set_for_across_plays: self.cached_cross_set_for_across_plays.clone(),
            cached_cross_set_for_down_plays: self.cached_cross_set_for_down_plays.clone(),
            cross_set_buffer: self.cross_set_buffer.clone(),
            remaining_word_multipliers_for_across_plays: self
                .remaining_word_multipliers_for_across_plays
                .clone(),
            remaining_word_multipliers_for_down_plays: self
                .remaining_word_multipliers_for_down_plays
                .clone(),
            remaining_tile_multipliers_for_across_plays: self
                .remaining_tile_multipliers_for_across_plays
                .clone(),
            remaining_tile_multipliers_for_down_plays: self
                .remaining_tile_multipliers_for_down_plays
                .clone(),
            face_value_scores_for_across_plays: self.face_value_scores_for_across_plays.clone(),
            face_value_scores_for_down_plays: self.face_value_scores_for_down_plays.clone(),
            perpendicular_word_multipliers_for_across_plays: self
                .perpendicular_word_multipliers_for_across_plays
                .clone(),
            perpendicular_word_multipliers_for_down_plays: self
                .perpendicular_word_multipliers_for_down_plays
                .clone(),
            perpendicular_scores_for_across_plays: self
                .perpendicular_scores_for_across_plays
                .clone(),
            perpendicular_scores_for_down_plays: self.perpendicular_scores_for_down_plays.clone(),
            transposed_board_tiles: self.transposed_board_tiles.clone(),
            num_tiles_on_board: self.num_tiles_on_board,
            num_tiles_in_bag: self.num_tiles_in_bag,
            num_tiles_on_rack: self.num_tiles_on_rack,
            rack_bits: self.rack_bits,
            multi_leaves: self.multi_leaves.clone(),
            descending_scores: self.descending_scores.clone(),
            exchange_buffer: self.exchange_buffer.clone(),
            aggregated_word_multipliers: self.aggregated_word_multipliers.clone(),
            precomputed_square_multiplier_buffer: self.precomputed_square_multiplier_buffer.clone(),
            indexes_to_descending_square_multiplier_buffer: self
                .indexes_to_descending_square_multiplier_buffer
                .clone(),
            multi_jumps_buffer: self.multi_jumps_buffer.clone(),
            best_leave_values: self.best_leave_values.clone(),
            found_placements: self.found_placements.clone(),
            used_letters_tally: self.used_letters_tally.clone(),
            used_tile_scores_shadowl: self.used_tile_scores_shadowl.clone(),
            used_tile_scores_shadowr: self.used_tile_scores_shadowr.clone(),
            rack_tally_shadowl: self.rack_tally_shadowl.clone(),
            rack_tally_shadowr: self.rack_tally_shadowr.clone(),
        }
    }

    #[inline(always)]
    fn clone_from(&mut self, source: &Self) {
        self.rack_tally.clone_from(&source.rack_tally);
        self.word_buffer_for_across_plays
            .clone_from(&source.word_buffer_for_across_plays);
        self.word_buffer_for_down_plays
            .clone_from(&source.word_buffer_for_down_plays);
        self.cross_set_for_across_plays
            .clone_from(&source.cross_set_for_across_plays);
        self.cross_set_for_down_plays
            .clone_from(&source.cross_set_for_down_plays);
        self.cached_cross_set_for_across_plays
            .clone_from(&source.cached_cross_set_for_across_plays);
        self.cached_cross_set_for_down_plays
            .clone_from(&source.cached_cross_set_for_down_plays);
        self.cross_set_buffer.clone_from(&source.cross_set_buffer);
        self.remaining_word_multipliers_for_across_plays
            .clone_from(&source.remaining_word_multipliers_for_across_plays);
        self.remaining_word_multipliers_for_down_plays
            .clone_from(&source.remaining_word_multipliers_for_down_plays);
        self.remaining_tile_multipliers_for_across_plays
            .clone_from(&source.remaining_tile_multipliers_for_across_plays);
        self.remaining_tile_multipliers_for_down_plays
            .clone_from(&source.remaining_tile_multipliers_for_down_plays);
        self.face_value_scores_for_across_plays
            .clone_from(&source.face_value_scores_for_across_plays);
        self.face_value_scores_for_down_plays
            .clone_from(&source.face_value_scores_for_down_plays);
        self.perpendicular_word_multipliers_for_across_plays
            .clone_from(&source.perpendicular_word_multipliers_for_across_plays);
        self.perpendicular_word_multipliers_for_down_plays
            .clone_from(&source.perpendicular_word_multipliers_for_down_plays);
        self.perpendicular_scores_for_across_plays
            .clone_from(&source.perpendicular_scores_for_across_plays);
        self.perpendicular_scores_for_down_plays
            .clone_from(&source.perpendicular_scores_for_down_plays);
        self.transposed_board_tiles
            .clone_from(&source.transposed_board_tiles);
        self.num_tiles_on_board
            .clone_from(&source.num_tiles_on_board);
        self.num_tiles_in_bag.clone_from(&source.num_tiles_in_bag);
        self.num_tiles_on_rack.clone_from(&source.num_tiles_on_rack);
        self.rack_bits.clone_from(&source.rack_bits);
        self.multi_leaves.clone_from(&source.multi_leaves);
        self.descending_scores.clone_from(&source.descending_scores);
        self.exchange_buffer.clone_from(&source.exchange_buffer);
        self.aggregated_word_multipliers
            .clone_from(&source.aggregated_word_multipliers);
        self.precomputed_square_multiplier_buffer
            .clone_from(&source.precomputed_square_multiplier_buffer);
        self.indexes_to_descending_square_multiplier_buffer
            .clone_from(&source.indexes_to_descending_square_multiplier_buffer);
        self.multi_jumps_buffer
            .clone_from(&source.multi_jumps_buffer);
        self.best_leave_values.clone_from(&source.best_leave_values);
        self.found_placements.clone_from(&source.found_placements);
        self.used_letters_tally
            .clone_from(&source.used_letters_tally);
        self.used_tile_scores_shadowl
            .clone_from(&source.used_tile_scores_shadowl);
        self.used_tile_scores_shadowr
            .clone_from(&source.used_tile_scores_shadowr);
        self.rack_tally_shadowl
            .clone_from(&source.rack_tally_shadowl);
        self.rack_tally_shadowr
            .clone_from(&source.rack_tally_shadowr);
    }
}

impl WorkingBuffer {
    fn new(game_config: &game_config::GameConfig) -> Self {
        let dim = game_config.board_layout().dim();
        let rows_times_cols = (dim.rows as isize * dim.cols as isize) as usize;
        Self {
            rack_tally: vec![0u8; game_config.alphabet().len() as usize].into_boxed_slice(),
            word_buffer_for_across_plays: vec![0u8; rows_times_cols].into_boxed_slice(),
            word_buffer_for_down_plays: vec![0u8; rows_times_cols].into_boxed_slice(),
            cross_set_for_across_plays: vec![CrossSet { bits: 0, score: 0 }; rows_times_cols]
                .into_boxed_slice(),
            cross_set_for_down_plays: vec![CrossSet { bits: 0, score: 0 }; rows_times_cols]
                .into_boxed_slice(),
            cached_cross_set_for_across_plays: vec![
                CachedCrossSet {
                    p_left: 0,
                    p_right: 0,
                    bits: 0,
                };
                rows_times_cols
            ]
            .into_boxed_slice(),
            cached_cross_set_for_down_plays: vec![
                CachedCrossSet {
                    p_left: 0,
                    p_right: 0,
                    bits: 0,
                };
                rows_times_cols
            ]
            .into_boxed_slice(),
            cross_set_buffer: vec![
                CrossSetComputation {
                    score: 0,
                    b_letter: 0,
                    end_range: 0,
                    p: 0,
                };
                dim.rows.max(dim.cols) as usize
            ]
            .into_boxed_slice(),
            remaining_word_multipliers_for_across_plays: vec![0i8; rows_times_cols]
                .into_boxed_slice(),
            remaining_word_multipliers_for_down_plays: vec![0i8; rows_times_cols]
                .into_boxed_slice(),
            remaining_tile_multipliers_for_across_plays: vec![0i8; rows_times_cols]
                .into_boxed_slice(),
            remaining_tile_multipliers_for_down_plays: vec![0i8; rows_times_cols]
                .into_boxed_slice(),
            face_value_scores_for_across_plays: vec![0i8; rows_times_cols].into_boxed_slice(),
            face_value_scores_for_down_plays: vec![0i8; rows_times_cols].into_boxed_slice(),
            perpendicular_word_multipliers_for_across_plays: vec![0i8; rows_times_cols]
                .into_boxed_slice(),
            perpendicular_word_multipliers_for_down_plays: vec![0i8; rows_times_cols]
                .into_boxed_slice(),
            perpendicular_scores_for_across_plays: vec![0i32; rows_times_cols].into_boxed_slice(),
            perpendicular_scores_for_down_plays: vec![0i32; rows_times_cols].into_boxed_slice(),
            transposed_board_tiles: vec![0u8; rows_times_cols].into_boxed_slice(),
            num_tiles_on_board: 0,
            num_tiles_in_bag: 0,
            num_tiles_on_rack: 0,
            rack_bits: 0,
            multi_leaves: klv::MultiLeaves::new(),
            descending_scores: Vec::new(),
            exchange_buffer: Vec::new(),
            aggregated_word_multipliers: Vec::new(),
            precomputed_square_multiplier_buffer: Vec::new(),
            indexes_to_descending_square_multiplier_buffer: Vec::new(),
            multi_jumps_buffer: vec![
                MultiJump {
                    left_score: 0,
                    right_score: 0,
                    left_idx: 0,
                    right_idx: 0,
                };
                dim.rows.max(dim.cols) as usize
            ]
            .into_boxed_slice(),
            best_leave_values: Vec::new(),
            found_placements: Vec::new(),
            used_letters_tally: Vec::new(),
            used_tile_scores_shadowl: Vec::new(),
            used_tile_scores_shadowr: Vec::new(),
            rack_tally_shadowl: vec![0u8; game_config.alphabet().len() as usize].into_boxed_slice(),
            rack_tally_shadowr: vec![0u8; game_config.alphabet().len() as usize].into_boxed_slice(),
        }
    }

    fn init<AdjustLeaveValue: Fn(f32) -> f32, N: kwg::Node, L: kwg::Node>(
        &mut self,
        board_snapshot: &BoardSnapshot<'_, N, L>,
        rack: &[u8],
        adjust_leave_value: &AdjustLeaveValue,
    ) {
        let alphabet = board_snapshot.game_config.alphabet();
        self.num_tiles_on_rack = rack.len().try_into().unwrap();
        self.exchange_buffer.clear();
        self.exchange_buffer
            .reserve(self.num_tiles_on_rack as usize);
        self.rack_tally.iter_mut().for_each(|m| *m = 0);
        self.rack_bits = 0u64;
        for tile in rack {
            self.rack_tally[*tile as usize] += 1;
            self.rack_bits |= 1u64 << tile;
        }
        self.word_buffer_for_across_plays
            .iter_mut()
            .for_each(|m| *m = 0);
        self.word_buffer_for_down_plays
            .iter_mut()
            .for_each(|m| *m = 0);
        self.cross_set_for_across_plays.iter_mut().for_each(|m| {
            m.bits = 0;
            m.score = 0;
        });
        self.cross_set_for_down_plays.iter_mut().for_each(|m| {
            m.bits = 0;
            m.score = 0;
        });

        let board_layout = board_snapshot.game_config.board_layout();
        let dim = board_layout.dim();
        let premiums = board_layout.premiums();
        let transposed_premiums = board_layout.transposed_premiums();
        let area = (dim.rows as isize * dim.cols as isize) as usize;
        // row * dim.cols + col
        for (idx, &b) in board_snapshot.board_tiles.iter().enumerate().take(area) {
            if b == 0 {
                let premium = &premiums[idx];
                self.remaining_word_multipliers_for_across_plays[idx] = premium.word_multiplier;
                self.remaining_tile_multipliers_for_across_plays[idx] = premium.tile_multiplier;
                self.face_value_scores_for_across_plays[idx] = 0;
            } else {
                self.remaining_word_multipliers_for_across_plays[idx] = 1; // needed for the HashMap
                //self.remaining_tile_multipliers_for_across_plays[idx] = 1; // not as crucial to set to 1
                self.face_value_scores_for_across_plays[idx] = alphabet.score(b);
            }
        }
        for col in 0..dim.cols {
            for row in 0..dim.rows {
                self.transposed_board_tiles
                    [(col as isize * dim.rows as isize + row as isize) as usize] = board_snapshot
                    .board_tiles[(row as isize * dim.cols as isize + col as isize) as usize];
            }
        }
        // col * dim.rows + row
        for (idx, &b) in self.transposed_board_tiles.iter().enumerate().take(area) {
            if b == 0 {
                let premium = &transposed_premiums[idx];
                self.remaining_word_multipliers_for_down_plays[idx] = premium.word_multiplier;
                self.remaining_tile_multipliers_for_down_plays[idx] = premium.tile_multiplier;
                self.face_value_scores_for_down_plays[idx] = 0;
            } else {
                self.remaining_word_multipliers_for_down_plays[idx] = 1; // needed for the HashMap
                //self.remaining_tile_multipliers_for_down_plays[idx] = 1; // not as crucial to set to 1
                self.face_value_scores_for_down_plays[idx] = alphabet.score(b);
            }
        }
        self.num_tiles_on_board = board_snapshot
            .board_tiles
            .iter()
            .filter(|&t| *t != 0)
            .count() as u16;
        self.num_tiles_in_bag = alphabet.num_tiles() as i16
            - (self.num_tiles_on_board as i16
                + board_snapshot.game_config.num_players() as i16
                    * board_snapshot.game_config.rack_size() as i16);
        let play_out_bonus = if self.num_tiles_in_bag <= 0 {
            (2 * ((0u8..)
                .zip(self.rack_tally.iter())
                .map(|(tile, &num)| {
                    (alphabet.freq(tile) as i32 - num as i32) * alphabet.score(tile) as i32
                })
                .sum::<i32>()
                - board_snapshot
                    .board_tiles
                    .iter()
                    .map(|&t| if t != 0 { alphabet.score(t) as i32 } else { 0 })
                    .sum::<i32>())) as f32
        } else {
            0.0
        };

        // eg if my rack is ZY??YVA it'd be [10,4,4,4,1,0,0].
        self.descending_scores.clear();
        self.descending_scores
            .reserve(self.num_tiles_on_rack as usize);
        for &tile in alphabet.tiles_by_descending_scores() {
            let count = self.rack_tally[tile as usize];
            if count != 0 {
                let score = alphabet.score(tile);
                for _ in 0..count {
                    self.descending_scores.push(score);
                }
            }
        }

        if self.num_tiles_in_bag <= 0 {
            self.multi_leaves.init(
                &self.rack_tally,
                board_snapshot.klv,
                false,
                adjust_leave_value,
            );
            self.multi_leaves
                .init_endgame_leaves(|tile| alphabet.score(tile), play_out_bonus);
            // the multi_leaves is correct but doing this directly is faster.
            self.best_leave_values.clear();
            self.best_leave_values
                .resize(self.num_tiles_on_rack as usize + 1, f32::NEG_INFINITY);
            let mut unplayed = 0i32;
            for i in (0..self.num_tiles_on_rack).rev() {
                unplayed += self.descending_scores[i as usize] as i32;
                self.best_leave_values[i as usize] = (-10 - 2 * unplayed) as f32;
            }
            self.best_leave_values[self.num_tiles_on_rack as usize] = play_out_bonus;
        } else {
            self.multi_leaves.init(
                &self.rack_tally,
                board_snapshot.klv,
                true,
                adjust_leave_value,
            );
            self.multi_leaves
                .extract_raw_best_leave_values(&mut self.best_leave_values);
        }
        for i in 0..=self.num_tiles_on_rack {
            self.best_leave_values[i as usize] +=
                board_snapshot.game_config.num_played_bonus(i) as f32;
        }
        self.used_letters_tally.clear();
        match board_snapshot.game_config.game_rules() {
            game_config::GameRules::Classic => {}
            game_config::GameRules::Jumbled => {
                self.used_letters_tally.resize(alphabet.len() as usize, 0);
            }
        }
        self.used_tile_scores_shadowl.clear();
        self.used_tile_scores_shadowl
            .reserve(self.num_tiles_on_rack as usize);
        self.used_tile_scores_shadowr.clear();
        self.used_tile_scores_shadowr
            .reserve(self.num_tiles_on_rack as usize);
    }

    fn init_after_cross_sets<N: kwg::Node, L: kwg::Node>(
        &mut self,
        board_snapshot: &BoardSnapshot<'_, N, L>,
    ) {
        let board_layout = board_snapshot.game_config.board_layout();
        let dim = board_layout.dim();
        let premiums = board_layout.premiums();
        let transposed_premiums = board_layout.transposed_premiums();
        let area = (dim.rows as isize * dim.cols as isize) as usize;
        // row * dim.cols + col
        for (idx, premium) in premiums.iter().enumerate().take(area) {
            let cross_set = &mut self.cross_set_for_across_plays[idx];
            if premium.word_multiplier == 0 && premium.tile_multiplier == 0 {
                cross_set.bits = 1;
            }
            let effective_pwm =
                self.remaining_word_multipliers_for_across_plays[idx] & -(cross_set.bits as i8 & 1);
            self.perpendicular_word_multipliers_for_across_plays[idx] = effective_pwm;
            self.perpendicular_scores_for_across_plays[idx] =
                cross_set.score * effective_pwm as i32;
        }
        // col * dim.rows + row
        for (idx, premium) in transposed_premiums.iter().enumerate().take(area) {
            let cross_set = &mut self.cross_set_for_down_plays[idx];
            if premium.word_multiplier == 0 && premium.tile_multiplier == 0 {
                cross_set.bits = 1;
            }
            let effective_pwm =
                self.remaining_word_multipliers_for_down_plays[idx] & -(cross_set.bits as i8 & 1);
            self.perpendicular_word_multipliers_for_down_plays[idx] = effective_pwm;
            self.perpendicular_scores_for_down_plays[idx] = cross_set.score * effective_pwm as i32;
        }
    }

    // call this before passing a different kwg.
    #[inline(always)]
    pub fn reset_for_another_kwg(&mut self) {
        self.cached_cross_set_for_across_plays.fill(CachedCrossSet {
            p_left: 0,
            p_right: 0,
            bits: 0,
        });
        self.cached_cross_set_for_down_plays.fill(CachedCrossSet {
            p_left: 0,
            p_right: 0,
            bits: 0,
        });
    }
}

// kwg must be Gaddawg for Classic, AlphaDawg for Jumbled.
pub struct BoardSnapshot<'a, N: kwg::Node, L: kwg::Node> {
    pub board_tiles: &'a [u8],
    pub game_config: &'a game_config::GameConfig,
    pub kwg: &'a kwg::Kwg<N>,
    pub klv: &'a klv::Klv<L>,
}

// cached_cross_sets is just one strip, so it is transposed from cross_sets
fn gen_classic_cross_set<'a, N: kwg::Node, L: kwg::Node>(
    board_snapshot: &'a BoardSnapshot<'a, N, L>,
    board_strip: &'a [u8],
    cross_sets: &'a mut [CrossSet],
    output_strider: matrix::Strider,
    cross_set_buffer: &'a mut [CrossSetComputation],
    cached_cross_sets: &'a mut [CachedCrossSet],
) {
    let len = output_strider.len();
    let step = output_strider.step() as usize;
    let kwg = &board_snapshot.kwg;
    let mut last_nonempty = len;
    {
        let alphabet = board_snapshot.game_config.alphabet();
        let mut p = 1;
        let mut score = 0i32;
        let mut last_empty = len;
        for j in (0..len).rev() {
            let b = board_strip[j as usize];
            if b != 0 {
                let b_letter = b & 0x7f;
                p = kwg.seek(p, b_letter);
                score += alphabet.score(b) as i32;
                cross_set_buffer[j as usize] = CrossSetComputation {
                    score,
                    b_letter,
                    end_range: last_empty,
                    p,
                };
                last_nonempty = j;
            } else {
                // empty square, reset
                p = 1; // cumulative gaddag traversal results
                score = 0; // cumulative face-value score
                last_empty = j; // last seen empty square
                cross_set_buffer[j as usize].b_letter = 0;
                cross_set_buffer[j as usize].end_range = last_nonempty;
            }
        }
    }

    let reuse_cross_set =
        |cached_cross_sets: &mut [CachedCrossSet], out_idx: i8, p_left, p_right| -> u64 {
            if cached_cross_sets[out_idx as usize].p_left == p_left
                && cached_cross_sets[out_idx as usize].p_right == p_right
            {
                cached_cross_sets[out_idx as usize].bits
            } else {
                cached_cross_sets[out_idx as usize].p_left = p_left;
                cached_cross_sets[out_idx as usize].p_right = p_right;
                0 // means unset, because bit 0 should always be set
            }
        };
    let mut wi = 0;
    let mut wp = output_strider.base() as usize;

    let mut j = last_nonempty;
    while j < len {
        if j > 0 {
            // [j-1] has right, no left.
            let mut p = cross_set_buffer[j as usize].p;
            let mut bits = reuse_cross_set(cached_cross_sets, j - 1, -2, p);
            if bits == 0 {
                bits = 1u64;
                if p > 0 {
                    p = kwg[p].arc_index();
                    if p > 0 {
                        loop {
                            let node = kwg[p];
                            bits |= (node.accepts() as u64) << node.tile();
                            if node.is_end() {
                                break;
                            }
                            p += 1;
                        }
                    }
                }
                cached_cross_sets[j as usize - 1].bits = bits;
            }
            for _ in wi..j - 1 {
                cross_sets[wp] = CrossSet { bits: 0, score: 0 };
                wp += step;
            }
            cross_sets[wp] = CrossSet {
                bits,
                score: cross_set_buffer[j as usize].score,
            };
            wi = j;
            wp += step;
        }
        let mut prev_j = j;
        j = cross_set_buffer[j as usize].end_range;
        if j >= len {
            break;
        }
        while j + 1 < len && cross_set_buffer[j as usize + 1].b_letter != 0 {
            j += 1;
            // [j-1] has left and right.
            let j_end = cross_set_buffer[j as usize].end_range;
            let mut p_right = cross_set_buffer[j as usize].p;
            let mut p_left = kwg.seek(cross_set_buffer[prev_j as usize].p, 0);
            let mut bits = reuse_cross_set(cached_cross_sets, j - 1, p_left, p_right);
            if bits == 0 {
                bits = 1u64;
                if p_right > 0 && p_left > 0 {
                    p_right = kwg[p_right].arc_index();
                    if p_right > 0 {
                        p_left = kwg[p_left].arc_index();
                        if p_left > 0 {
                            let mut node_left = kwg[p_left];
                            let mut node_right = kwg[p_right];
                            let mut node_left_tile = node_left.tile();
                            if j_end - j > j - 1 - prev_j {
                                // Right is longer than left.
                                loop {
                                    match node_left_tile.cmp(&node_right.tile()) {
                                        std::cmp::Ordering::Less => {
                                            // left < right: advance left
                                            if node_left.is_end() {
                                                break;
                                            }
                                            p_left += 1;
                                            node_left = kwg[p_left];
                                            node_left_tile = node_left.tile();
                                        }
                                        std::cmp::Ordering::Greater => {
                                            // left > right: advance right
                                            if node_right.is_end() {
                                                break;
                                            }
                                            p_right += 1;
                                            node_right = kwg[p_right];
                                        }
                                        std::cmp::Ordering::Equal => {
                                            // left == right (right is longer than left):
                                            // complete right half with the shorter left half
                                            let mut q = p_right;
                                            for qi in (prev_j..j - 1).rev() {
                                                q = kwg.seek(
                                                    q,
                                                    cross_set_buffer[qi as usize].b_letter,
                                                );
                                                if q <= 0 {
                                                    break;
                                                }
                                            }
                                            if q > 0 {
                                                bits |= (kwg[q].accepts() as u64) << node_left_tile;
                                            }
                                            if node_left.is_end() {
                                                break;
                                            }
                                            p_left += 1;
                                            node_left = kwg[p_left];
                                            node_left_tile = node_left.tile();
                                            if node_right.is_end() {
                                                break;
                                            }
                                            p_right += 1;
                                            node_right = kwg[p_right];
                                        }
                                    }
                                }
                            } else {
                                loop {
                                    match node_left_tile.cmp(&node_right.tile()) {
                                        std::cmp::Ordering::Less => {
                                            // left < right: advance left
                                            if node_left.is_end() {
                                                break;
                                            }
                                            p_left += 1;
                                            node_left = kwg[p_left];
                                            node_left_tile = node_left.tile();
                                        }
                                        std::cmp::Ordering::Greater => {
                                            // left > right: advance right
                                            if node_right.is_end() {
                                                break;
                                            }
                                            p_right += 1;
                                            node_right = kwg[p_right];
                                        }
                                        std::cmp::Ordering::Equal => {
                                            // left == right (right is not longer than left):
                                            // complete left half with right half
                                            let mut q = p_left;
                                            for qi in j..j_end {
                                                q = kwg.seek(
                                                    q,
                                                    cross_set_buffer[qi as usize].b_letter,
                                                );
                                                if q <= 0 {
                                                    break;
                                                }
                                            }
                                            if q > 0 {
                                                bits |= (kwg[q].accepts() as u64) << node_left_tile;
                                            }
                                            if node_right.is_end() {
                                                break;
                                            }
                                            p_right += 1;
                                            node_right = kwg[p_right];
                                            if node_left.is_end() {
                                                break;
                                            }
                                            p_left += 1;
                                            node_left = kwg[p_left];
                                            node_left_tile = node_left.tile();
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
                cached_cross_sets[j as usize - 1].bits = bits;
            }
            for _ in wi..j - 1 {
                cross_sets[wp] = CrossSet { bits: 0, score: 0 };
                wp += step;
            }
            cross_sets[wp] = CrossSet {
                bits,
                score: cross_set_buffer[prev_j as usize].score + cross_set_buffer[j as usize].score,
            };
            wi = j;
            wp += step;
            prev_j = j;
            j = j_end;
        }
        if j >= len {
            break;
        }
        // [j] has left, no right.
        let mut p = kwg.seek(cross_set_buffer[prev_j as usize].p, 0);
        let mut bits = reuse_cross_set(cached_cross_sets, j, p, -2);
        if bits == 0 {
            bits = 1u64;
            if p > 0 {
                p = kwg[p].arc_index();
                if p > 0 {
                    loop {
                        let node = kwg[p];
                        bits |= (node.accepts() as u64) << node.tile();
                        if node.is_end() {
                            break;
                        }
                        p += 1;
                    }
                }
            }
            cached_cross_sets[j as usize].bits = bits;
        }
        for _ in wi..j {
            cross_sets[wp] = CrossSet { bits: 0, score: 0 };
            wp += step;
        }
        cross_sets[wp] = CrossSet {
            bits,
            score: cross_set_buffer[prev_j as usize].score,
        };
        wi = j + 1;
        wp += step;
        j = cross_set_buffer[j as usize].end_range;
    }
    for _ in wi..len {
        cross_sets[wp] = CrossSet { bits: 0, score: 0 };
        wp += step;
    }
}

fn gen_jumbled_cross_set<'a, N: kwg::Node, L: kwg::Node>(
    board_snapshot: &'a BoardSnapshot<'a, N, L>,
    board_strip: &'a [u8],
    cross_sets: &'a mut [CrossSet],
    output_strider: matrix::Strider,
    used_letters_tally: &'a mut [u8],
) {
    let len = output_strider.len();
    let step = output_strider.step() as usize;
    let mut wp = output_strider.base() as usize;
    let kwg = &board_snapshot.kwg;
    let alphabet = board_snapshot.game_config.alphabet();
    let mut prev_wp = !0;
    for i in 0..len {
        let b = board_strip[i as usize];
        if b != 0 {
            cross_sets[wp] = CrossSet { bits: 0, score: 0 };
        } else if prev_wp != !0 && (i + 1 >= len || board_strip[i as usize + 1] == 0) {
            // this is the matching right side of a lone island.
            // reuse the computed left side's cross set.
            cross_sets[wp] = CrossSet {
                ..cross_sets[prev_wp]
            };
            prev_wp = !0;
        } else {
            let mut score = 0i32;
            let mut j = i;
            while j > 0 {
                let b = board_strip[j as usize - 1];
                if b == 0 {
                    break;
                }
                j -= 1;
                score += alphabet.score(b) as i32;
                used_letters_tally[(b & 0x7f) as usize] += 1;
            }
            let mut k = i + 1;
            while k < len {
                let b = board_strip[k as usize];
                if b == 0 {
                    break;
                }
                k += 1;
                score += alphabet.score(b) as i32;
                used_letters_tally[(b & 0x7f) as usize] += 1;
            }
            if k == j + 1 {
                cross_sets[wp] = CrossSet { bits: 0, score: 0 };
            } else {
                cross_sets[wp] = CrossSet {
                    bits: kwg.compute_alpha_cross_set(used_letters_tally),
                    score,
                };
                // if j == i, this is the left side of a possible lone island.
                // otherwise set to !0.
                prev_wp = wp | ((j == i) as isize - 1) as usize;
                used_letters_tally.iter_mut().for_each(|m| *m = 0);
            }
        }
        wp += step;
    }
}

#[inline(always)]
fn gen_cross_set<'a, N: kwg::Node, L: kwg::Node>(
    board_snapshot: &'a BoardSnapshot<'a, N, L>,
    board_strip: &'a [u8],
    cross_sets: &'a mut [CrossSet],
    output_strider: matrix::Strider,
    cross_set_buffer: &'a mut [CrossSetComputation],
    cached_cross_sets: &'a mut [CachedCrossSet],
    used_letters_tally: &'a mut [u8],
) {
    match board_snapshot.game_config.game_rules() {
        game_config::GameRules::Classic => gen_classic_cross_set(
            board_snapshot,
            board_strip,
            cross_sets,
            output_strider,
            cross_set_buffer,
            cached_cross_sets,
        ),
        game_config::GameRules::Jumbled => gen_jumbled_cross_set(
            board_snapshot,
            board_strip,
            cross_sets,
            output_strider,
            used_letters_tally,
        ),
    }
}

struct GenPlacePlacementsParams<'a> {
    board_strip: &'a [u8],
    alphabet: &'a alphabet::Alphabet,
    rack_tally: &'a mut [u8],
    used_tile_scores_shadowl: &'a mut Vec<i8>,
    used_tile_scores_shadowr: &'a mut Vec<i8>,
    shadow_strip_buffer: &'a mut [u8], // not really storing letters here
    cross_set_strip: &'a [CrossSet],
    remaining_word_multipliers_strip: &'a [i8],
    remaining_tile_multipliers_strip: &'a [i8],
    perpendicular_word_multipliers_strip: &'a [i8],
    perpendicular_scores_strip: &'a [i32],
    rack_bits: u64,
    descending_scores: &'a [i8],
    aggregated_word_multipliers: &'a mut Vec<i32>,
    precomputed_square_multiplier_buffer: &'a mut Vec<i32>,
    indexes_to_descending_square_multiplier_buffer: &'a mut Vec<i8>,
    multi_jumps_buffer: &'a mut [MultiJump],
    best_leave_values: &'a [f32],
    num_max_played: u8,
    rack_tally_shadowl: &'a mut [u8],
    rack_tally_shadowr: &'a mut [u8],
}

fn gen_place_placements<'a, PossibleStripPlacementCallbackType: FnMut(i8, i8, i8, f32)>(
    params: &'a mut GenPlacePlacementsParams<'a>,
    single_tile_plays: bool,
    want_raw: bool,
    mut possible_strip_placement_callback: PossibleStripPlacementCallbackType,
) {
    let strider_len = params.board_strip.len();

    if !want_raw {
        // process the square multipliers.
        params.aggregated_word_multipliers.clear();
        // each contiguous subsequence of multiple 1s needs to be processed just once.
        let mut last_was_one = false;
        for i in 0..strider_len {
            let mut wm = params.remaining_word_multipliers_strip[i] as i32;
            if last_was_one {
                last_was_one = wm == 1;
                continue;
            }
            last_was_one = wm == 1;
            if let Err(idx) = params.aggregated_word_multipliers.binary_search(&wm) {
                params.aggregated_word_multipliers.insert(idx, wm);
            }
            for &wm_val in &params.remaining_word_multipliers_strip[i + 1..strider_len] {
                if wm_val != 1 {
                    // wm_val == 1 is frequent.
                    wm *= wm_val as i32;
                    // monotonically increasing only if all multipliers are positive.
                    if let Err(idx) = params.aggregated_word_multipliers.binary_search(&wm) {
                        params.aggregated_word_multipliers.insert(idx, wm);
                    }
                }
            }
        }
        let vec_size = strider_len * params.aggregated_word_multipliers.len();
        params
            .precomputed_square_multiplier_buffer
            .resize(vec_size, 0);
        params
            .indexes_to_descending_square_multiplier_buffer
            .resize(vec_size, 0);
        for (k, low_end) in params
            .aggregated_word_multipliers
            .iter()
            .zip((0..).step_by(strider_len))
        {
            // k is the aggregated main word multiplier.
            // low_end is the index of the strider_len-length slice.
            let high_end = low_end + strider_len;
            let precomputed_square_multiplier_slice =
                &mut params.precomputed_square_multiplier_buffer[low_end..high_end];
            let indexes_to_descending_square_multiplier_slice =
                &mut params.indexes_to_descending_square_multiplier_buffer[low_end..high_end];
            let mut left = 0;
            for j in (0..strider_len).filter(|&j| params.board_strip[j] == 0) {
                // perpendicular_word_multipliers_strip[j] is 0 if no perpendicular tile.
                precomputed_square_multiplier_slice[j] = params.remaining_tile_multipliers_strip[j]
                    as i32
                    * (k + params.perpendicular_word_multipliers_strip[j] as i32);
                // put the indexes of empty squares first.
                // the indexes of non-empty squares should never be visited.
                indexes_to_descending_square_multiplier_slice[left] = j as i8;
                left += 1;
            }
            indexes_to_descending_square_multiplier_slice[..left].sort_unstable_by(|&a, &b| {
                precomputed_square_multiplier_slice[b as usize]
                    .cmp(&precomputed_square_multiplier_slice[a as usize])
            });
        }

        // precompute the multi jumps. (code is similar to cross set computation.)
        let mut score = 0i32;
        let mut last_empty = strider_len as i8;
        for j in (0..strider_len).rev() {
            let b = params.board_strip[j];
            if b != 0 {
                score += params.alphabet.score(b) as i32;
            } else {
                // empty square, reset
                score = 0; // cumulative face-value score
                last_empty = j as i8; // last seen empty square
            }
            params.multi_jumps_buffer[j].right_score = score;
            params.multi_jumps_buffer[j].right_idx = last_empty;
        }
        score = 0i32;
        last_empty = -1i8;
        for j in 0..strider_len {
            let b = params.board_strip[j];
            if b != 0 {
                score += params.alphabet.score(b) as i32;
            } else {
                // empty square, reset
                score = 0; // cumulative face-value score
                last_empty = j as i8; // last seen empty square
            }
            params.multi_jumps_buffer[j].left_score = score;
            params.multi_jumps_buffer[j].left_idx = last_empty;
        }
    }

    struct Env<'a> {
        params: &'a mut GenPlacePlacementsParams<'a>,
        strider_len: usize,
        anchor: i8,
        leftmost: i8,
        rightmost: i8,
        best_possible_equity: f32,
    }

    let mut env = Env {
        params,
        strider_len,
        anchor: 0,
        leftmost: 0,
        rightmost: 0,
        best_possible_equity: f32::NEG_INFINITY,
    };

    // during shadow-playing, main_score and perpendicular_cumulative_score
    // assume all tiles placed from rack this turn are worth zero,
    // except forced-placement cases.
    // their scores are added separately.
    struct Accumulator {
        main_score: i32,                     // main_played_through_score
        perpendicular_cumulative_score: i32, // perpendicular_additional_score
        word_multiplier: i32,
    }

    #[inline(always)]
    fn shadow_record(
        env: &mut Env<'_>,
        acc: &Accumulator,
        idx_left: i8,
        idx_right: i8,
        num_played: u8,
    ) {
        let used_tile_scores = if env.params.used_tile_scores_shadowr.is_empty() {
            &env.params.used_tile_scores_shadowl
        } else {
            &env.params.used_tile_scores_shadowr
        };
        let mut best_scoring = 0;
        let mut to_assign = num_played - used_tile_scores.len() as u8;
        if to_assign != 0 {
            // if a square requiring [B] is encountered while holding a B, the B
            // must go there. if a square requiring [A,B] is encountered earlier,
            // that square must be A, but this is not currently implemented.
            let low_end = env
                .params
                .aggregated_word_multipliers
                .binary_search(&acc.word_multiplier)
                .unwrap()
                * env.strider_len;
            let high_end = low_end + env.strider_len;
            let precomputed_square_multiplier_slice =
                &env.params.precomputed_square_multiplier_buffer[low_end..high_end];
            let mut used_tile_scores_iter = used_tile_scores.iter().rev().peekable(); // iterate from highest score
            let mut desc_scores_iter = env
                .params
                .descending_scores
                .iter()
                .filter(|&score| used_tile_scores_iter.next_if_eq(&score).is_none());
            for &idx in
                &env.params.indexes_to_descending_square_multiplier_buffer[low_end..high_end]
            {
                if idx_left <= idx
                    && idx < idx_right
                    && env.params.shadow_strip_buffer[idx as usize] == 0
                {
                    best_scoring += *desc_scores_iter.next().unwrap() as i32
                        * precomputed_square_multiplier_slice[idx as usize];
                    to_assign -= 1;
                    if to_assign == 0 {
                        break;
                    }
                }
            }
        }
        let equity = (acc.main_score * acc.word_multiplier
            + acc.perpendicular_cumulative_score
            + best_scoring) as f32
            + env.params.best_leave_values[num_played as usize];
        if equity > env.best_possible_equity {
            env.best_possible_equity = equity;
        }
    }

    #[inline(always)]
    fn shadow_play_right(
        env: &mut Env<'_>,
        mut acc: Accumulator,
        mut idx: i8,
        mut is_unique: bool,
        idx_left: i8,
        mut num_played: u8,
        mut rack_bits: u64,
    ) {
        env.params
            .used_tile_scores_shadowr
            .clone_from(env.params.used_tile_scores_shadowl);
        env.params
            .rack_tally_shadowr
            .clone_from_slice(env.params.rack_tally_shadowl);
        loop {
            if idx < env.rightmost {
                // tail-recurse placing current sequence of tiles in one go
                let multi_jump = &env.params.multi_jumps_buffer[idx as usize];
                acc.main_score += multi_jump.right_score;
                idx = multi_jump.right_idx;
            }
            // tiles have been placed from idx_left to idx - 1.
            // here idx <= env.rightmost.
            // check if [idx_left, idx) is a thing
            if idx > env.anchor + 1 && num_played > !is_unique as u8 && idx - idx_left >= 2 {
                shadow_record(env, &acc, idx_left, idx, num_played);
            }
            if num_played >= env.params.num_max_played {
                break;
            }

            if idx >= env.rightmost {
                break;
            }

            // place a tile at [idx] since it is still in bounds.
            let this_cross_bits = env.params.cross_set_strip[idx as usize].bits;
            if this_cross_bits & 1 == 0 {
                // nothing hooks here.
                is_unique = true;
            } else if this_cross_bits != 1 {
                // something hooks here and there is a valid letter.
                // this_cross_bits has bit 1 set, so blank is always allowed.
                let matching_bits = this_cross_bits & rack_bits;
                if matching_bits == 0 {
                    break;
                }
                let tile = matching_bits.trailing_zeros() as u8;
                if matching_bits.is_power_of_two() {
                    // case 1: only one tile fits.
                    // consume the square and the tile.
                    // rack_bits will turn off if the tile is depleted.
                    env.params.rack_tally_shadowr[tile as usize] -= 1;
                    // this is (rack_tally[tile] == 0 ? matching_bits : 0).
                    rack_bits ^= matching_bits
                        & (-((env.params.rack_tally_shadowr[tile as usize] == 0) as i64)) as u64;
                    // fall-through to case 2 (assume the optimized asm does not recheck the condition).
                }
                if matching_bits.is_power_of_two()
                    || matching_bits & env.params.alphabet.same_score_tile_bits(tile)
                        == matching_bits
                {
                    // case 2: multiple tiles fit, but they all have the same score.
                    // consume the square, but not the tile.
                    // rack_bits remains unchanged because assignment is tentative.
                    env.params.shadow_strip_buffer[idx as usize] = 1; // hide this square from greedy algorithm.
                    let tile_score = env.params.alphabet.score(tile);
                    env.params.used_tile_scores_shadowr.insert(
                        env.params
                            .used_tile_scores_shadowr
                            .partition_point(|&x| x <= tile_score),
                        tile_score,
                    );
                    let tile_value = tile_score as i32
                        * env.params.remaining_tile_multipliers_strip[idx as usize] as i32;
                    acc.main_score += tile_value;
                    acc.perpendicular_cumulative_score += env.params.perpendicular_scores_strip
                        [idx as usize]
                        + tile_value
                            * env.params.perpendicular_word_multipliers_strip[idx as usize] as i32;
                } else {
                    // case 3: multiple tiles fit, and they have different scores.
                    // rack_bits remains unchanged because assignment is tentative.
                    // defer to greedy algorithm.
                    env.params.shadow_strip_buffer[idx as usize] = 0; // let greedy algorithm fill this square.
                    acc.perpendicular_cumulative_score +=
                        env.params.perpendicular_scores_strip[idx as usize];
                }
            } else {
                break;
            }
            num_played += 1;
            acc.word_multiplier *= env.params.remaining_word_multipliers_strip[idx as usize] as i32;
            idx += 1;
        }
        env.params.used_tile_scores_shadowr.clear(); // use shadowl in shadow_record
    }

    #[inline(always)]
    fn shadow_play_left(env: &mut Env<'_>, mut acc: Accumulator, mut idx: i8, mut is_unique: bool) {
        let mut num_played = 0;
        env.params.used_tile_scores_shadowl.clear();
        let mut rack_bits = env.params.rack_bits;
        env.params
            .rack_tally_shadowl
            .clone_from_slice(env.params.rack_tally);
        loop {
            if idx >= env.leftmost {
                // tail-recurse placing current sequence of tiles in one go
                let multi_jump = &env.params.multi_jumps_buffer[idx as usize];
                acc.main_score += multi_jump.left_score;
                idx = multi_jump.left_idx;
            }
            // tiles have been placed from env.anchor to idx + 1.
            // here idx >= env.leftmost - 1.
            // check if [idx + 1, env.anchor + 1) is a thing
            if num_played > !is_unique as u8 && env.anchor - idx >= 2 {
                shadow_record(env, &acc, idx + 1, env.anchor + 1, num_played);
            }
            if num_played >= env.params.num_max_played {
                break;
            }

            // can switch direction only after using the anchor square
            if idx < env.anchor {
                shadow_play_right(
                    env,
                    Accumulator { ..acc },
                    env.anchor + 1,
                    is_unique,
                    idx + 1,
                    num_played,
                    rack_bits,
                );
            }

            if idx < env.leftmost {
                break;
            }

            // place a tile at [idx] since it is still in bounds.
            let this_cross_bits = env.params.cross_set_strip[idx as usize].bits;
            if this_cross_bits & 1 == 0 {
                // nothing hooks here.
                is_unique = true;
            } else if this_cross_bits != 1 {
                // something hooks here and there is a valid letter.
                // this_cross_bits has bit 1 set, so blank is always allowed.
                let matching_bits = this_cross_bits & rack_bits;
                if matching_bits == 0 {
                    break;
                }
                let tile = matching_bits.trailing_zeros() as u8;
                if matching_bits.is_power_of_two() {
                    // case 1: only one tile fits.
                    // consume the square and the tile.
                    // rack_bits will turn off if the tile is depleted.
                    env.params.rack_tally_shadowl[tile as usize] -= 1;
                    // this is (rack_tally[tile] == 0 ? matching_bits : 0).
                    rack_bits ^= matching_bits
                        & (-((env.params.rack_tally_shadowl[tile as usize] == 0) as i64)) as u64;
                    // fall-through to case 2 (assume the optimized asm does not recheck the condition).
                }
                if matching_bits.is_power_of_two()
                    || matching_bits & env.params.alphabet.same_score_tile_bits(tile)
                        == matching_bits
                {
                    // case 2: multiple tiles fit, but they all have the same score.
                    // consume the square, but not the tile.
                    // rack_bits remains unchanged because assignment is tentative.
                    env.params.shadow_strip_buffer[idx as usize] = 1; // hide this square from greedy algorithm.
                    let tile_score = env.params.alphabet.score(tile);
                    env.params.used_tile_scores_shadowl.insert(
                        env.params
                            .used_tile_scores_shadowl
                            .partition_point(|&x| x <= tile_score),
                        tile_score,
                    );
                    let tile_value = tile_score as i32
                        * env.params.remaining_tile_multipliers_strip[idx as usize] as i32;
                    acc.main_score += tile_value;
                    acc.perpendicular_cumulative_score += env.params.perpendicular_scores_strip
                        [idx as usize]
                        + tile_value
                            * env.params.perpendicular_word_multipliers_strip[idx as usize] as i32;
                } else {
                    // case 3: multiple tiles fit, and they have different scores.
                    // rack_bits remains unchanged because assignment is tentative.
                    // defer to greedy algorithm.
                    env.params.shadow_strip_buffer[idx as usize] = 0; // let greedy algorithm fill this square.
                    acc.perpendicular_cumulative_score +=
                        env.params.perpendicular_scores_strip[idx as usize];
                }
            } else {
                break;
            }
            num_played += 1;
            acc.word_multiplier *= env.params.remaining_word_multipliers_strip[idx as usize] as i32;
            idx -= 1;
        }
    }

    #[inline(always)]
    fn gen_places_from<PossibleStripPlacementCallbackType: FnMut(i8, i8, i8, f32)>(
        env: &mut Env<'_>,
        single_tile_plays: bool,
        want_raw: bool,
        mut possible_strip_placement_callback: PossibleStripPlacementCallbackType,
    ) {
        if want_raw {
            possible_strip_placement_callback(
                env.anchor,
                env.leftmost,
                env.rightmost,
                f32::INFINITY,
            );
        } else {
            env.best_possible_equity = f32::NEG_INFINITY;
            shadow_play_left(
                env,
                Accumulator {
                    main_score: 0,
                    perpendicular_cumulative_score: 0,
                    word_multiplier: 1,
                },
                env.anchor,
                single_tile_plays,
            );
            if env.best_possible_equity.is_finite() {
                possible_strip_placement_callback(
                    env.anchor,
                    env.leftmost,
                    env.rightmost,
                    env.best_possible_equity,
                );
            }
        }
    }

    let mut leftmost = strider_len as i8; // processed up to here
    loop {
        let mut rightmost = leftmost;
        while leftmost > 0 && env.params.board_strip[leftmost as usize - 1] == 0 {
            leftmost -= 1;
        }
        if leftmost > 0 {
            // board[leftmost - 1] is a tile.
            env.anchor = leftmost - 1;
            env.leftmost = 0;
            env.rightmost = rightmost;
            gen_places_from(
                &mut env,
                single_tile_plays,
                want_raw,
                &mut possible_strip_placement_callback,
            );
        }
        {
            // this part is only relevant if rack has at least two tiles, but passing that is too expensive.
            let leftmost = leftmost + (leftmost > 0) as i8; // shadowing
            for anchor in (leftmost..rightmost).rev() {
                let cross_set_bits = env.params.cross_set_strip[anchor as usize].bits;
                if cross_set_bits != 0 {
                    if rightmost - leftmost < 2 {
                        // not enough room for 2-tile words
                        break;
                    }
                    if cross_set_bits != 1 {
                        env.anchor = anchor;
                        env.leftmost = leftmost;
                        env.rightmost = rightmost;
                        gen_places_from(
                            &mut env,
                            single_tile_plays,
                            want_raw,
                            &mut possible_strip_placement_callback,
                        );
                    }
                    rightmost = anchor; // prevent duplicates
                }
            }
        }
        loop {
            leftmost -= 1;
            if leftmost <= 1 {
                // not enough room for 2-tile words
                return;
            }
            if env.params.board_strip[leftmost as usize] == 0 {
                break;
            }
        }
    }
}

struct GenPlaceMovesParams<'a, CallbackType: FnMut(i8, &[u8], i32, f32), N: kwg::Node, L: kwg::Node>
{
    board_snapshot: &'a BoardSnapshot<'a, N, L>,
    board_strip: &'a [u8],
    cross_set_strip: &'a [CrossSet],
    remaining_word_multipliers_strip: &'a [i8],
    remaining_tile_multipliers_strip: &'a [i8],
    face_value_scores_strip: &'a [i8],
    perpendicular_word_multipliers_strip: &'a [i8],
    perpendicular_scores_strip: &'a [i32],
    rack_tally: &'a mut [u8],
    word_strip_buffer: &'a mut [u8],
    num_max_played: u8,
    anchor: i8,
    leftmost: i8,
    rightmost: i8,
    callback: CallbackType,
    multi_leaves: &'a klv::MultiLeaves,
    used_letters_tally: &'a mut [u8], // jumbled mode only
}

fn gen_classic_place_moves<
    'a,
    CallbackType: FnMut(i8, &[u8], i32, f32),
    N: kwg::Node,
    L: kwg::Node,
>(
    params: &'a mut GenPlaceMovesParams<'a, CallbackType, N, L>,
    single_tile_plays: bool,
) {
    struct Env<'a, CallbackType: FnMut(i8, &[u8], i32, f32), N: kwg::Node, L: kwg::Node> {
        params: &'a mut GenPlaceMovesParams<'a, CallbackType, N, L>,
        alphabet: &'a alphabet::Alphabet,
        num_played: u8,
        idx_left: i8,
    }
    struct Accumulator {
        main_score: i32,
        perpendicular_cumulative_score: i32,
        word_multiplier: i32,
        leave_idx: u32,
    }

    fn record<CallbackType: FnMut(i8, &[u8], i32, f32), N: kwg::Node, L: kwg::Node>(
        env: &mut Env<'_, CallbackType, N, L>,
        acc: &Accumulator,
        idx_left: i8,
        idx_right: i8,
    ) {
        let score = acc.main_score * acc.word_multiplier
            + acc.perpendicular_cumulative_score
            + env
                .params
                .board_snapshot
                .game_config
                .num_played_bonus(env.num_played) as i32;
        (env.params.callback)(
            idx_left,
            &env.params.word_strip_buffer[idx_left as usize..idx_right as usize],
            score,
            env.params.multi_leaves.leave_value(acc.leave_idx),
        );
    }

    fn play_right<CallbackType: FnMut(i8, &[u8], i32, f32), N: kwg::Node, L: kwg::Node>(
        env: &mut Env<'_, CallbackType, N, L>,
        acc: &mut Accumulator,
        mut p: i32,
        mut idx: i8,
        mut is_unique: bool,
    ) {
        // tail-recurse placing current sequence of tiles
        while idx < env.params.rightmost {
            let b = env.params.board_strip[idx as usize];
            if b == 0 {
                break;
            }
            p = env.params.board_snapshot.kwg.seek(p, b & 0x7f);
            if p <= 0 {
                return;
            }
            acc.main_score += env.params.face_value_scores_strip[idx as usize] as i32;
            idx += 1;
        }
        let node = env.params.board_snapshot.kwg[p];
        if idx > env.params.anchor + 1
            && env.num_played > !is_unique as u8
            && idx - env.idx_left >= 2
            && node.accepts()
        {
            record(env, acc, env.idx_left, idx);
        }
        if env.num_played >= env.params.num_max_played {
            return;
        }

        if idx < env.params.rightmost {
            p = node.arc_index();
            if p <= 0 {
                return;
            }
            let mut this_cross_bits = env.params.cross_set_strip[idx as usize].bits;
            if this_cross_bits == 1 {
                // already handled '@'
                return;
            } else if this_cross_bits != 0 {
                // turn off bit 0 so it cannot match later
                this_cross_bits &= !1;
            } else {
                this_cross_bits = !1;
                is_unique = true;
            };
            let new_word_multiplier = acc.word_multiplier
                * env.params.remaining_word_multipliers_strip[idx as usize] as i32;
            let tile_multiplier = env.params.remaining_tile_multipliers_strip[idx as usize];
            let perpendicular_word_multiplier =
                env.params.perpendicular_word_multipliers_strip[idx as usize];
            let perpendicular_score = env.params.perpendicular_scores_strip[idx as usize];
            env.num_played += 1;
            let opt_blank_acc = (env.params.rack_tally[0] > 0).then(|| {
                // intentional to not hardcode blank tile value as zero
                let tile_value = env.alphabet.score(0) as i32 * tile_multiplier as i32;
                Accumulator {
                    main_score: acc.main_score + tile_value,
                    perpendicular_cumulative_score: acc.perpendicular_cumulative_score
                        + perpendicular_score
                        + tile_value * perpendicular_word_multiplier as i32,
                    word_multiplier: new_word_multiplier,
                    leave_idx: acc.leave_idx - env.params.multi_leaves.place_value(0),
                }
            });
            loop {
                let node = env.params.board_snapshot.kwg[p];
                let tile = node.tile();
                if this_cross_bits & (1 << tile) != 0 {
                    if env.params.rack_tally[tile as usize] > 0 {
                        env.params.rack_tally[tile as usize] -= 1;
                        let tile_value = env.alphabet.score(tile) as i32 * tile_multiplier as i32;
                        env.params.word_strip_buffer[idx as usize] = tile;
                        play_right(
                            env,
                            &mut Accumulator {
                                main_score: acc.main_score + tile_value,
                                perpendicular_cumulative_score: acc.perpendicular_cumulative_score
                                    + perpendicular_score
                                    + tile_value * perpendicular_word_multiplier as i32,
                                word_multiplier: new_word_multiplier,
                                leave_idx: acc.leave_idx
                                    - env.params.multi_leaves.place_value(tile),
                            },
                            p,
                            idx + 1,
                            is_unique,
                        );
                        env.params.rack_tally[tile as usize] += 1;
                    }
                    if let Some(blank_acc) = &opt_blank_acc {
                        env.params.rack_tally[0] -= 1;
                        env.params.word_strip_buffer[idx as usize] = tile | 0x80;
                        play_right(
                            env,
                            &mut Accumulator { ..*blank_acc },
                            p,
                            idx + 1,
                            is_unique,
                        );
                        env.params.rack_tally[0] += 1;
                    }
                }
                if node.is_end() {
                    break;
                }
                p += 1;
            }
            env.num_played -= 1;
        }
    }

    fn play_left<CallbackType: FnMut(i8, &[u8], i32, f32), N: kwg::Node, L: kwg::Node>(
        env: &mut Env<'_, CallbackType, N, L>,
        acc: &mut Accumulator,
        mut p: i32,
        mut idx: i8,
        mut is_unique: bool,
    ) {
        // tail-recurse placing current sequence of tiles
        while idx >= env.params.leftmost {
            let b = env.params.board_strip[idx as usize];
            if b == 0 {
                break;
            }
            p = env.params.board_snapshot.kwg.seek(p, b & 0x7f);
            if p <= 0 {
                return;
            }
            acc.main_score += env.params.face_value_scores_strip[idx as usize] as i32;
            idx -= 1;
        }
        let mut node = env.params.board_snapshot.kwg[p];
        if env.num_played > !is_unique as u8 && env.params.anchor - idx >= 2 && node.accepts() {
            record(env, acc, idx + 1, env.params.anchor + 1);
        }
        if env.num_played >= env.params.num_max_played {
            return;
        }

        p = node.arc_index();
        if p <= 0 {
            return;
        }

        node = env.params.board_snapshot.kwg[p];
        if node.tile() == 0 {
            // assume idx < env.params.anchor, because tile 0 does not occur at start in well-formed kwg gaddawg
            env.idx_left = idx + 1;
            play_right(env, acc, p, env.params.anchor + 1, is_unique);
            if node.is_end() {
                return;
            }
            p += 1;
        }

        if idx >= env.params.leftmost {
            let mut this_cross_bits = env.params.cross_set_strip[idx as usize].bits;
            if this_cross_bits == 1 {
                // already handled '@'
                return;
            } else if this_cross_bits != 0 {
                // turn off bit 0 so it cannot match later
                this_cross_bits &= !1;
            } else {
                this_cross_bits = !1;
                is_unique = true;
            }
            let new_word_multiplier = acc.word_multiplier
                * env.params.remaining_word_multipliers_strip[idx as usize] as i32;
            let tile_multiplier = env.params.remaining_tile_multipliers_strip[idx as usize];
            let perpendicular_word_multiplier =
                env.params.perpendicular_word_multipliers_strip[idx as usize];
            let perpendicular_score = env.params.perpendicular_scores_strip[idx as usize];
            env.num_played += 1;
            let opt_blank_acc = (env.params.rack_tally[0] > 0).then(|| {
                // intentional to not hardcode blank tile value as zero
                let tile_value = env.alphabet.score(0) as i32 * tile_multiplier as i32;
                Accumulator {
                    main_score: acc.main_score + tile_value,
                    perpendicular_cumulative_score: acc.perpendicular_cumulative_score
                        + perpendicular_score
                        + tile_value * perpendicular_word_multiplier as i32,
                    word_multiplier: new_word_multiplier,
                    leave_idx: acc.leave_idx - env.params.multi_leaves.place_value(0),
                }
            });
            loop {
                let node = env.params.board_snapshot.kwg[p];
                let tile = node.tile();
                if this_cross_bits & (1 << tile) != 0 {
                    if env.params.rack_tally[tile as usize] > 0 {
                        env.params.rack_tally[tile as usize] -= 1;
                        let tile_value = env.alphabet.score(tile) as i32 * tile_multiplier as i32;
                        env.params.word_strip_buffer[idx as usize] = tile;
                        play_left(
                            env,
                            &mut Accumulator {
                                main_score: acc.main_score + tile_value,
                                perpendicular_cumulative_score: acc.perpendicular_cumulative_score
                                    + perpendicular_score
                                    + tile_value * perpendicular_word_multiplier as i32,
                                word_multiplier: new_word_multiplier,
                                leave_idx: acc.leave_idx
                                    - env.params.multi_leaves.place_value(tile),
                            },
                            p,
                            idx - 1,
                            is_unique,
                        );
                        env.params.rack_tally[tile as usize] += 1;
                    }
                    if let Some(blank_acc) = &opt_blank_acc {
                        env.params.rack_tally[0] -= 1;
                        env.params.word_strip_buffer[idx as usize] = tile | 0x80;
                        play_left(
                            env,
                            &mut Accumulator { ..*blank_acc },
                            p,
                            idx - 1,
                            is_unique,
                        );
                        env.params.rack_tally[0] += 1;
                    }
                }
                if node.is_end() {
                    break;
                }
                p += 1;
            }
            env.num_played -= 1;
        }
    }

    let alphabet = params.board_snapshot.game_config.alphabet();
    let anchor = params.anchor;
    let pass_leave_idx = params.multi_leaves.pass_leave_idx();
    play_left(
        &mut Env {
            params,
            alphabet,
            num_played: 0,
            idx_left: 0,
        },
        &mut Accumulator {
            main_score: 0,
            perpendicular_cumulative_score: 0,
            word_multiplier: 1,
            leave_idx: pass_leave_idx,
        },
        1,
        anchor,
        single_tile_plays,
    );
}

fn gen_jumbled_place_moves<
    'a,
    CallbackType: FnMut(i8, &[u8], i32, f32),
    N: kwg::Node,
    L: kwg::Node,
>(
    params: &'a mut GenPlaceMovesParams<'a, CallbackType, N, L>,
    single_tile_plays: bool,
) {
    struct Env<'a, CallbackType: FnMut(i8, &[u8], i32, f32), N: kwg::Node, L: kwg::Node> {
        params: &'a mut GenPlaceMovesParams<'a, CallbackType, N, L>,
        alphabet: &'a alphabet::Alphabet,
        num_played: u8,
        idx_left: i8,
    }
    struct Accumulator {
        main_score: i32,
        perpendicular_cumulative_score: i32,
        word_multiplier: i32,
        leave_idx: u32,
    }

    fn record_if_valid<CallbackType: FnMut(i8, &[u8], i32, f32), N: kwg::Node, L: kwg::Node>(
        env: &mut Env<'_, CallbackType, N, L>,
        acc: &Accumulator,
        idx_left: i8,
        idx_right: i8,
    ) {
        if env
            .params
            .board_snapshot
            .kwg
            .accepts_alpha(env.params.used_letters_tally)
        {
            let score = acc.main_score * acc.word_multiplier
                + acc.perpendicular_cumulative_score
                + env
                    .params
                    .board_snapshot
                    .game_config
                    .num_played_bonus(env.num_played) as i32;
            (env.params.callback)(
                idx_left,
                &env.params.word_strip_buffer[idx_left as usize..idx_right as usize],
                score,
                env.params.multi_leaves.leave_value(acc.leave_idx),
            );
        }
    }

    fn play_right<CallbackType: FnMut(i8, &[u8], i32, f32), N: kwg::Node, L: kwg::Node>(
        env: &mut Env<'_, CallbackType, N, L>,
        acc: &mut Accumulator,
        mut idx: i8,
        mut is_unique: bool,
    ) {
        let orig_idx = idx;
        // tail-recurse placing current sequence of tiles
        while idx < env.params.rightmost {
            let b = env.params.board_strip[idx as usize];
            if b == 0 {
                break;
            }
            env.params.used_letters_tally[(b & 0x7f) as usize] += 1;
            acc.main_score += env.params.face_value_scores_strip[idx as usize] as i32;
            idx += 1;
        }
        if idx > env.params.anchor + 1
            && env.num_played > !is_unique as u8
            && idx - env.idx_left >= 2
        {
            record_if_valid(env, acc, env.idx_left, idx);
        }
        if env.num_played < env.params.num_max_played && idx < env.params.rightmost {
            let mut this_cross_bits = env.params.cross_set_strip[idx as usize].bits;
            if this_cross_bits == 1 {
                // already handled '@'
            } else {
                if this_cross_bits != 0 {
                    // turn off bit 0 so it cannot match later
                    this_cross_bits &= !1;
                } else {
                    this_cross_bits = !1;
                    is_unique = true;
                };
                let new_word_multiplier = acc.word_multiplier
                    * env.params.remaining_word_multipliers_strip[idx as usize] as i32;
                let tile_multiplier = env.params.remaining_tile_multipliers_strip[idx as usize];
                let perpendicular_word_multiplier =
                    env.params.perpendicular_word_multipliers_strip[idx as usize];
                let perpendicular_score = env.params.perpendicular_scores_strip[idx as usize];
                env.num_played += 1;
                let opt_blank_acc = (env.params.rack_tally[0] > 0).then(|| {
                    // intentional to not hardcode blank tile value as zero
                    let tile_value = env.alphabet.score(0) as i32 * tile_multiplier as i32;
                    Accumulator {
                        main_score: acc.main_score + tile_value,
                        perpendicular_cumulative_score: acc.perpendicular_cumulative_score
                            + perpendicular_score
                            + tile_value * perpendicular_word_multiplier as i32,
                        word_multiplier: new_word_multiplier,
                        leave_idx: acc.leave_idx - env.params.multi_leaves.place_value(0),
                    }
                });
                for tile in 1..env.alphabet.len() {
                    if this_cross_bits & (1 << tile) != 0 {
                        if env.params.rack_tally[tile as usize] > 0 {
                            env.params.rack_tally[tile as usize] -= 1;
                            env.params.used_letters_tally[tile as usize] += 1;
                            let tile_value =
                                env.alphabet.score(tile) as i32 * tile_multiplier as i32;
                            env.params.word_strip_buffer[idx as usize] = tile;
                            play_right(
                                env,
                                &mut Accumulator {
                                    main_score: acc.main_score + tile_value,
                                    perpendicular_cumulative_score: acc
                                        .perpendicular_cumulative_score
                                        + perpendicular_score
                                        + tile_value * perpendicular_word_multiplier as i32,
                                    word_multiplier: new_word_multiplier,
                                    leave_idx: acc.leave_idx
                                        - env.params.multi_leaves.place_value(tile),
                                },
                                idx + 1,
                                is_unique,
                            );
                            env.params.used_letters_tally[tile as usize] -= 1;
                            env.params.rack_tally[tile as usize] += 1;
                        }
                        if let Some(blank_acc) = &opt_blank_acc {
                            env.params.rack_tally[0] -= 1;
                            env.params.used_letters_tally[tile as usize] += 1;
                            env.params.word_strip_buffer[idx as usize] = tile | 0x80;
                            play_right(env, &mut Accumulator { ..*blank_acc }, idx + 1, is_unique);
                            env.params.used_letters_tally[tile as usize] -= 1;
                            env.params.rack_tally[0] += 1;
                        }
                    }
                }
                env.num_played -= 1;
            }
        }
        for idx in orig_idx..idx {
            let b = env.params.board_strip[idx as usize];
            env.params.used_letters_tally[(b & 0x7f) as usize] -= 1;
        }
    }

    fn play_left<CallbackType: FnMut(i8, &[u8], i32, f32), N: kwg::Node, L: kwg::Node>(
        env: &mut Env<'_, CallbackType, N, L>,
        acc: &mut Accumulator,
        mut idx: i8,
        mut is_unique: bool,
    ) {
        let orig_idx = idx;
        // tail-recurse placing current sequence of tiles
        while idx >= env.params.leftmost {
            let b = env.params.board_strip[idx as usize];
            if b == 0 {
                break;
            }
            env.params.used_letters_tally[(b & 0x7f) as usize] += 1;
            acc.main_score += env.params.face_value_scores_strip[idx as usize] as i32;
            idx -= 1;
        }
        if env.num_played > !is_unique as u8 && env.params.anchor - idx >= 2 {
            record_if_valid(env, acc, idx + 1, env.params.anchor + 1);
        }
        if env.num_played < env.params.num_max_played {
            if idx < env.params.anchor {
                env.idx_left = idx + 1;
                play_right(env, acc, env.params.anchor + 1, is_unique);
            }

            if idx >= env.params.leftmost {
                let mut this_cross_bits = env.params.cross_set_strip[idx as usize].bits;
                if this_cross_bits == 1 {
                    // already handled '@'
                } else {
                    if this_cross_bits != 0 {
                        // turn off bit 0 so it cannot match later
                        this_cross_bits &= !1;
                    } else {
                        this_cross_bits = !1;
                        is_unique = true;
                    }
                    let new_word_multiplier = acc.word_multiplier
                        * env.params.remaining_word_multipliers_strip[idx as usize] as i32;
                    let tile_multiplier = env.params.remaining_tile_multipliers_strip[idx as usize];
                    let perpendicular_word_multiplier =
                        env.params.perpendicular_word_multipliers_strip[idx as usize];
                    let perpendicular_score = env.params.perpendicular_scores_strip[idx as usize];
                    env.num_played += 1;
                    let opt_blank_acc = (env.params.rack_tally[0] > 0).then(|| {
                        // intentional to not hardcode blank tile value as zero
                        let tile_value = env.alphabet.score(0) as i32 * tile_multiplier as i32;
                        Accumulator {
                            main_score: acc.main_score + tile_value,
                            perpendicular_cumulative_score: acc.perpendicular_cumulative_score
                                + perpendicular_score
                                + tile_value * perpendicular_word_multiplier as i32,
                            word_multiplier: new_word_multiplier,
                            leave_idx: acc.leave_idx - env.params.multi_leaves.place_value(0),
                        }
                    });
                    for tile in 1..env.alphabet.len() {
                        if this_cross_bits & (1 << tile) != 0 {
                            if env.params.rack_tally[tile as usize] > 0 {
                                env.params.rack_tally[tile as usize] -= 1;
                                env.params.used_letters_tally[tile as usize] += 1;
                                let tile_value =
                                    env.alphabet.score(tile) as i32 * tile_multiplier as i32;
                                env.params.word_strip_buffer[idx as usize] = tile;
                                play_left(
                                    env,
                                    &mut Accumulator {
                                        main_score: acc.main_score + tile_value,
                                        perpendicular_cumulative_score: acc
                                            .perpendicular_cumulative_score
                                            + perpendicular_score
                                            + tile_value * perpendicular_word_multiplier as i32,
                                        word_multiplier: new_word_multiplier,
                                        leave_idx: acc.leave_idx
                                            - env.params.multi_leaves.place_value(tile),
                                    },
                                    idx - 1,
                                    is_unique,
                                );
                                env.params.used_letters_tally[tile as usize] -= 1;
                                env.params.rack_tally[tile as usize] += 1;
                            }
                            if let Some(blank_acc) = &opt_blank_acc {
                                env.params.rack_tally[0] -= 1;
                                env.params.used_letters_tally[tile as usize] += 1;
                                env.params.word_strip_buffer[idx as usize] = tile | 0x80;
                                play_left(
                                    env,
                                    &mut Accumulator { ..*blank_acc },
                                    idx - 1,
                                    is_unique,
                                );
                                env.params.used_letters_tally[tile as usize] -= 1;
                                env.params.rack_tally[0] += 1;
                            }
                        }
                    }
                    env.num_played -= 1;
                }
            }
        }

        for idx in idx + 1..orig_idx + 1 {
            let b = env.params.board_strip[idx as usize];
            env.params.used_letters_tally[(b & 0x7f) as usize] -= 1;
        }
    }

    let alphabet = params.board_snapshot.game_config.alphabet();
    let anchor = params.anchor;
    let pass_leave_idx = params.multi_leaves.pass_leave_idx();
    play_left(
        &mut Env {
            params,
            alphabet,
            num_played: 0,
            idx_left: 0,
        },
        &mut Accumulator {
            main_score: 0,
            perpendicular_cumulative_score: 0,
            word_multiplier: 1,
            leave_idx: pass_leave_idx,
        },
        anchor,
        single_tile_plays,
    );
}

#[inline(always)]
fn gen_place_moves<'a, CallbackType: FnMut(i8, &[u8], i32, f32), N: kwg::Node, L: kwg::Node>(
    params: &'a mut GenPlaceMovesParams<'a, CallbackType, N, L>,
    single_tile_plays: bool,
) {
    match params.board_snapshot.game_config.game_rules() {
        game_config::GameRules::Classic => gen_classic_place_moves(params, single_tile_plays),
        game_config::GameRules::Jumbled => gen_jumbled_place_moves(params, single_tile_plays),
    }
}

fn gen_place_moves_at<
    'a,
    FoundPlaceMove: FnMut(bool, i8, i8, &[u8], i32, f32),
    N: kwg::Node,
    L: kwg::Node,
>(
    board_snapshot: &'a BoardSnapshot<'a, N, L>,
    working_buffer: &mut WorkingBuffer,
    multi_leaves: &'a klv::MultiLeaves,
    placement: &PossiblePlacement,
    num_max_played: u8,
    mut found_place_move: FoundPlaceMove,
) {
    let dim = board_snapshot.game_config.board_layout().dim();
    let strip_range_start;
    let strip_range_end;
    if placement.down {
        strip_range_start = (placement.lane as isize * dim.rows as isize) as usize;
        strip_range_end = strip_range_start + dim.rows as usize;
    } else {
        strip_range_start = (placement.lane as isize * dim.cols as isize) as usize;
        strip_range_end = strip_range_start + dim.cols as usize;
    }
    gen_place_moves(
        &mut GenPlaceMovesParams {
            board_snapshot,
            board_strip: if placement.down {
                &working_buffer.transposed_board_tiles[strip_range_start..strip_range_end]
            } else {
                &board_snapshot.board_tiles[strip_range_start..strip_range_end]
            },
            cross_set_strip: if placement.down {
                &working_buffer.cross_set_for_down_plays[strip_range_start..strip_range_end]
            } else {
                &working_buffer.cross_set_for_across_plays[strip_range_start..strip_range_end]
            },
            remaining_word_multipliers_strip: if placement.down {
                &working_buffer.remaining_word_multipliers_for_down_plays
                    [strip_range_start..strip_range_end]
            } else {
                &working_buffer.remaining_word_multipliers_for_across_plays
                    [strip_range_start..strip_range_end]
            },
            remaining_tile_multipliers_strip: if placement.down {
                &working_buffer.remaining_tile_multipliers_for_down_plays
                    [strip_range_start..strip_range_end]
            } else {
                &working_buffer.remaining_tile_multipliers_for_across_plays
                    [strip_range_start..strip_range_end]
            },
            face_value_scores_strip: if placement.down {
                &working_buffer.face_value_scores_for_down_plays[strip_range_start..strip_range_end]
            } else {
                &working_buffer.face_value_scores_for_across_plays
                    [strip_range_start..strip_range_end]
            },
            perpendicular_word_multipliers_strip: if placement.down {
                &working_buffer.perpendicular_word_multipliers_for_down_plays
                    [strip_range_start..strip_range_end]
            } else {
                &working_buffer.perpendicular_word_multipliers_for_across_plays
                    [strip_range_start..strip_range_end]
            },
            perpendicular_scores_strip: if placement.down {
                &working_buffer.perpendicular_scores_for_down_plays
                    [strip_range_start..strip_range_end]
            } else {
                &working_buffer.perpendicular_scores_for_across_plays
                    [strip_range_start..strip_range_end]
            },
            rack_tally: &mut working_buffer.rack_tally,
            word_strip_buffer: if placement.down {
                &mut working_buffer.word_buffer_for_down_plays[strip_range_start..strip_range_end]
            } else {
                &mut working_buffer.word_buffer_for_across_plays[strip_range_start..strip_range_end]
            },
            num_max_played,
            anchor: placement.anchor,
            leftmost: placement.leftmost,
            rightmost: placement.rightmost,
            callback: |idx: i8, word: &[u8], score: i32, leave_value: f32| {
                found_place_move(
                    placement.down,
                    placement.lane,
                    idx,
                    word,
                    score,
                    leave_value,
                )
            },
            multi_leaves,
            used_letters_tally: &mut working_buffer.used_letters_tally,
        },
        !placement.down,
    );
}

#[derive(Eq, Hash, PartialEq)]
pub enum Play {
    Exchange {
        tiles: bites::Bites,
    },
    Place {
        down: bool,
        lane: i8,
        idx: i8,
        word: bites::Bites,
        score: i32,
    },
}

impl Clone for Play {
    #[inline(always)]
    fn clone(&self) -> Self {
        match self {
            Self::Exchange { tiles } => Self::Exchange {
                tiles: tiles.clone(),
            },
            Self::Place {
                down,
                lane,
                idx,
                word,
                score,
            } => Self::Place {
                down: *down,
                lane: *lane,
                idx: *idx,
                word: word.clone(),
                score: *score,
            },
        }
    }

    #[inline(always)]
    fn clone_from(&mut self, source: &Self) {
        match self {
            Self::Exchange { tiles: self_tiles } => {
                if let Self::Exchange {
                    tiles: source_tiles,
                } = source
                {
                    self_tiles.clone_from(source_tiles);
                } else {
                    *self = source.clone() as _;
                }
            }
            Self::Place {
                down: self_down,
                lane: self_lane,
                idx: self_idx,
                word: self_word,
                score: self_score,
            } => {
                if let Self::Place {
                    down: source_down,
                    lane: source_lane,
                    idx: source_idx,
                    word: source_word,
                    score: source_score,
                } = source
                {
                    self_down.clone_from(source_down);
                    self_lane.clone_from(source_lane);
                    self_idx.clone_from(source_idx);
                    self_word.clone_from(source_word);
                    self_score.clone_from(source_score);
                } else {
                    *self = source.clone() as _;
                }
            }
        }
    }
}

pub struct ValuedMove {
    pub equity: f32,
    pub play: Play,
}

impl Clone for ValuedMove {
    #[inline(always)]
    fn clone(&self) -> Self {
        Self {
            equity: self.equity,
            play: self.play.clone(),
        }
    }

    #[inline(always)]
    fn clone_from(&mut self, source: &Self) {
        self.equity.clone_from(&source.equity);
        self.play.clone_from(&source.play);
    }
}

impl PartialEq for ValuedMove {
    #[inline(always)]
    fn eq(&self, other: &Self) -> bool {
        (other.equity - self.equity) == 0.0
    }
}

impl Eq for ValuedMove {}

impl PartialOrd for ValuedMove {
    #[inline(always)]
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for ValuedMove {
    #[inline(always)]
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        other.equity.total_cmp(&self.equity)
    }
}

pub struct WriteablePlay<'a, N: kwg::Node, L: kwg::Node> {
    board_snapshot: &'a BoardSnapshot<'a, N, L>,
    play: &'a Play,
}

impl<N: kwg::Node, L: kwg::Node> std::fmt::Display for WriteablePlay<'_, N, L> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if f.width().is_some() {
            // allocates, but no choice.
            #[expect(clippy::recursive_format_impl)]
            return f.pad(&format!("{self}"));
        }
        match &self.play {
            Play::Exchange { tiles } => {
                if tiles.is_empty() {
                    write!(f, "Pass")?;
                } else {
                    let alphabet = self.board_snapshot.game_config.alphabet();
                    write!(f, "Exch. ")?;
                    for &tile in tiles.iter() {
                        write!(f, "{}", alphabet.of_rack(tile).unwrap())?;
                    }
                }
            }
            Play::Place {
                down,
                lane,
                idx,
                word,
                score,
            } => {
                let dim = self.board_snapshot.game_config.board_layout().dim();
                let alphabet = self.board_snapshot.game_config.alphabet();
                if *down {
                    write!(f, "{}{} ", display::column(*lane), idx + 1)?;
                } else {
                    write!(f, "{}{} ", lane + 1, display::column(*idx))?;
                }
                let strider = dim.lane(*down, *lane);
                let mut inside = false;
                for (i, &tile) in (*idx..).zip(word.iter()) {
                    if tile == 0 {
                        if !inside {
                            write!(f, "(")?;
                            inside = true;
                        }
                        write!(
                            f,
                            "{}",
                            alphabet
                                .of_board(self.board_snapshot.board_tiles[strider.at(i)])
                                .unwrap(),
                        )?;
                    } else {
                        if inside {
                            write!(f, ")")?;
                            inside = false;
                        }
                        write!(f, "{}", alphabet.of_board(tile).unwrap())?;
                    }
                }
                if inside {
                    write!(f, ")")?;
                }
                write!(f, " {score}")?;
            }
        }
        Ok(())
    }
}

impl Play {
    pub fn fmt<'a, N: kwg::Node, L: kwg::Node>(
        &'a self,
        board_snapshot: &'a BoardSnapshot<'_, N, L>,
    ) -> WriteablePlay<'a, N, L> {
        WriteablePlay {
            board_snapshot,
            play: self,
        }
    }
}

pub struct GenMovesParams<'a, N: kwg::Node, L: kwg::Node> {
    pub board_snapshot: &'a BoardSnapshot<'a, N, L>,
    pub rack: &'a [u8],
    pub max_gen: usize,
    pub num_exchanges_by_this_player: i16,
    pub always_include_pass: bool,
}

// KurniaMoveGenerator can only be reused for the same game_config and kwg.
// (Refer to note at WorkingBuffer.)
// This is not enforced.
pub struct KurniaMoveGenerator {
    working_buffer: WorkingBuffer,
    pub plays: Vec<ValuedMove>,
}

impl Clone for KurniaMoveGenerator {
    #[inline(always)]
    fn clone(&self) -> Self {
        Self {
            working_buffer: self.working_buffer.clone(),
            plays: self.plays.clone(),
        }
    }

    #[inline(always)]
    fn clone_from(&mut self, source: &Self) {
        self.working_buffer.clone_from(&source.working_buffer);
        self.plays.clone_from(&source.plays);
    }
}

impl KurniaMoveGenerator {
    pub fn new(game_config: &game_config::GameConfig) -> Self {
        Self {
            working_buffer: WorkingBuffer::new(game_config),
            plays: Vec::new(),
        }
    }

    // call this before passing a different kwg.
    #[inline(always)]
    pub fn reset_for_another_kwg(&mut self) {
        self.working_buffer.reset_for_another_kwg();
    }

    // skip equity computation and sorting
    pub fn gen_moves_raw_all_unsorted<'a, N: kwg::Node, L: kwg::Node>(
        &mut self,
        board_snapshot: &'a BoardSnapshot<'a, N, L>,
        rack: &'a [u8],
        num_exchanges_by_this_player: i16,
        always_include_pass: bool,
    ) {
        self.plays.clear();

        let vec_moves = std::cell::RefCell::new(std::mem::take(&mut self.plays));

        let working_buffer = &mut self.working_buffer;
        working_buffer.init(board_snapshot, rack, &|leave_value: f32| leave_value);
        let multi_leaves = std::mem::take(&mut working_buffer.multi_leaves);

        let found_place_move =
            |down: bool, lane: i8, idx: i8, word: &[u8], score: i32, _leave_value: f32| {
                vec_moves.borrow_mut().push(ValuedMove {
                    equity: 0.0,
                    play: Play::Place {
                        down,
                        lane,
                        idx,
                        word: word.into(),
                        score,
                    },
                });
            };

        let found_exchange_move = |exchanged_tiles: &[u8], _leave_value: f32| {
            vec_moves.borrow_mut().push(ValuedMove {
                equity: 0.0,
                play: Play::Exchange {
                    tiles: exchanged_tiles.into(),
                },
            });
        };

        for _ in kurnia_gen_place_moves_iter(
            true,
            board_snapshot,
            working_buffer,
            &multi_leaves,
            found_place_move,
            |_best_possible_equity: f32| true,
        ) {}
        kurnia_gen_exchange_moves(
            board_snapshot,
            working_buffer,
            &multi_leaves,
            num_exchanges_by_this_player,
            found_exchange_move,
        );
        if always_include_pass || vec_moves.borrow().is_empty() {
            found_exchange_move(
                &working_buffer.exchange_buffer,
                multi_leaves.pass_leave_value(),
            );
        }

        self.plays = vec_moves.into_inner();

        working_buffer.multi_leaves = multi_leaves;
    }

    pub async fn gen_moves_filtered_async<
        'a,
        PlaceMovePredicate: FnMut(bool, i8, i8, &[u8], i32) -> bool,
        AdjustLeaveValue: Fn(f32) -> f32,
        EquityPredicate: FnMut(f32, &Play) -> bool,
        BreatheFuture: std::future::Future,
        N: kwg::Node,
        L: kwg::Node,
    >(
        &mut self,
        params: &'a GenMovesParams<'a, N, L>,
        mut place_move_predicate: PlaceMovePredicate,
        adjust_leave_value: AdjustLeaveValue,
        equity_predicate: EquityPredicate,
        mut breathe: impl FnMut() -> BreatheFuture,
    ) {
        self.plays.clear();
        if params.max_gen == 0 {
            return;
        }

        let alphabet = params.board_snapshot.game_config.alphabet();
        let board_layout = params.board_snapshot.game_config.board_layout();

        let found_moves = std::cell::RefCell::new(std::collections::BinaryHeap::from(
            std::mem::take(&mut self.plays),
        ));
        let equity_pred = std::cell::RefCell::new(equity_predicate);

        #[inline(always)]
        fn push_move<F: FnMut() -> Play, EquityPredicate: FnMut(f32, &Play) -> bool>(
            found_moves: &std::cell::RefCell<std::collections::BinaryHeap<ValuedMove>>,
            equity_pred: &std::cell::RefCell<EquityPredicate>,
            max_gen: usize,
            equity: f32,
            mut construct_play: F,
        ) {
            let mut borrowed = found_moves.borrow_mut();
            if borrowed.len() >= max_gen && borrowed.peek().unwrap().equity >= equity {
                return;
            }
            let play = construct_play();
            if equity_pred.borrow_mut()(equity, &play) {
                if borrowed.len() >= max_gen {
                    *borrowed.peek_mut().unwrap() = ValuedMove { equity, play };
                } else {
                    borrowed.push(ValuedMove { equity, play });
                }
            }
        }

        let working_buffer = &mut self.working_buffer;
        working_buffer.init(params.board_snapshot, params.rack, &adjust_leave_value);
        let multi_leaves = std::mem::take(&mut working_buffer.multi_leaves);
        let num_tiles_on_board = working_buffer.num_tiles_on_board;

        let found_place_move =
            |down: bool, lane: i8, idx: i8, word: &[u8], score: i32, leave_value: f32| {
                if place_move_predicate(down, lane, idx, word, score) {
                    let other_adjustments = if num_tiles_on_board == 0 {
                        (idx..)
                            .zip(word)
                            .filter(|&(ref i, &tile)| {
                                tile != 0
                                    && alphabet.is_vowel(tile)
                                    && if down {
                                        board_layout.danger_star_down(*i)
                                    } else {
                                        board_layout.danger_star_across(*i)
                                    }
                            })
                            .count() as f32
                            * -0.7
                    } else {
                        0.0
                    };
                    let equity = score as f32 + leave_value + other_adjustments;
                    push_move(&found_moves, &equity_pred, params.max_gen, equity, || {
                        Play::Place {
                            down,
                            lane,
                            idx,
                            word: word.into(),
                            score,
                        }
                    });
                }
            };

        let found_exchange_move = |exchanged_tiles: &[u8], leave_value: f32| {
            push_move(
                &found_moves,
                &equity_pred,
                params.max_gen,
                leave_value,
                || Play::Exchange {
                    tiles: exchanged_tiles.into(),
                },
            );
        };

        let can_accept = |best_possible_equity: f32| {
            let borrowed = found_moves.borrow();
            !(borrowed.len() >= params.max_gen
                && borrowed.peek().unwrap().equity >= best_possible_equity)
        };

        for _ in kurnia_gen_place_moves_iter(
            false,
            params.board_snapshot,
            working_buffer,
            &multi_leaves,
            found_place_move,
            can_accept,
        ) {
            breathe().await;
        }
        kurnia_gen_exchange_moves(
            params.board_snapshot,
            working_buffer,
            &multi_leaves,
            params.num_exchanges_by_this_player,
            found_exchange_move,
        );
        if params.always_include_pass || found_moves.borrow().is_empty() {
            found_exchange_move(
                &working_buffer.exchange_buffer,
                multi_leaves.pass_leave_value(),
            );
        }

        self.plays = found_moves.into_inner().into_sorted_vec();

        working_buffer.multi_leaves = multi_leaves;
    }

    pub fn gen_moves_filtered<
        'a,
        PlaceMovePredicate: FnMut(bool, i8, i8, &[u8], i32) -> bool,
        AdjustLeaveValue: Fn(f32) -> f32,
        EquityPredicate: FnMut(f32, &Play) -> bool,
        N: kwg::Node,
        L: kwg::Node,
    >(
        &mut self,
        params: &'a GenMovesParams<'a, N, L>,
        mut place_move_predicate: PlaceMovePredicate,
        adjust_leave_value: AdjustLeaveValue,
        equity_predicate: EquityPredicate,
    ) {
        self.plays.clear();
        if params.max_gen == 0 {
            return;
        }

        let alphabet = params.board_snapshot.game_config.alphabet();
        let board_layout = params.board_snapshot.game_config.board_layout();

        let found_moves = std::cell::RefCell::new(std::collections::BinaryHeap::from(
            std::mem::take(&mut self.plays),
        ));
        let equity_pred = std::cell::RefCell::new(equity_predicate);

        #[inline(always)]
        fn push_move<F: FnMut() -> Play, EquityPredicate: FnMut(f32, &Play) -> bool>(
            found_moves: &std::cell::RefCell<std::collections::BinaryHeap<ValuedMove>>,
            equity_pred: &std::cell::RefCell<EquityPredicate>,
            max_gen: usize,
            equity: f32,
            mut construct_play: F,
        ) {
            let mut borrowed = found_moves.borrow_mut();
            if borrowed.len() >= max_gen && borrowed.peek().unwrap().equity >= equity {
                return;
            }
            let play = construct_play();
            if equity_pred.borrow_mut()(equity, &play) {
                if borrowed.len() >= max_gen {
                    *borrowed.peek_mut().unwrap() = ValuedMove { equity, play };
                } else {
                    borrowed.push(ValuedMove { equity, play });
                }
            }
        }

        let working_buffer = &mut self.working_buffer;
        working_buffer.init(params.board_snapshot, params.rack, &adjust_leave_value);
        let multi_leaves = std::mem::take(&mut working_buffer.multi_leaves);
        let num_tiles_on_board = working_buffer.num_tiles_on_board;

        let found_place_move =
            |down: bool, lane: i8, idx: i8, word: &[u8], score: i32, leave_value: f32| {
                if place_move_predicate(down, lane, idx, word, score) {
                    let other_adjustments = if num_tiles_on_board == 0 {
                        (idx..)
                            .zip(word)
                            .filter(|&(ref i, &tile)| {
                                tile != 0
                                    && alphabet.is_vowel(tile)
                                    && if down {
                                        board_layout.danger_star_down(*i)
                                    } else {
                                        board_layout.danger_star_across(*i)
                                    }
                            })
                            .count() as f32
                            * -0.7
                    } else {
                        0.0
                    };
                    let equity = score as f32 + leave_value + other_adjustments;
                    push_move(&found_moves, &equity_pred, params.max_gen, equity, || {
                        Play::Place {
                            down,
                            lane,
                            idx,
                            word: word.into(),
                            score,
                        }
                    });
                }
            };

        let found_exchange_move = |exchanged_tiles: &[u8], leave_value: f32| {
            push_move(
                &found_moves,
                &equity_pred,
                params.max_gen,
                leave_value,
                || Play::Exchange {
                    tiles: exchanged_tiles.into(),
                },
            );
        };

        let can_accept = |best_possible_equity: f32| {
            let borrowed = found_moves.borrow();
            !(borrowed.len() >= params.max_gen
                && borrowed.peek().unwrap().equity >= best_possible_equity)
        };

        for _ in kurnia_gen_place_moves_iter(
            false,
            params.board_snapshot,
            working_buffer,
            &multi_leaves,
            found_place_move,
            can_accept,
        ) {}
        kurnia_gen_exchange_moves(
            params.board_snapshot,
            working_buffer,
            &multi_leaves,
            params.num_exchanges_by_this_player,
            found_exchange_move,
        );
        if params.always_include_pass || found_moves.borrow().is_empty() {
            found_exchange_move(
                &working_buffer.exchange_buffer,
                multi_leaves.pass_leave_value(),
            );
        }

        self.plays = found_moves.into_inner().into_sorted_vec();

        working_buffer.multi_leaves = multi_leaves;
    }

    #[inline(always)]
    pub fn gen_moves_unfiltered<'a, N: kwg::Node, L: kwg::Node>(
        &mut self,
        params: &'a GenMovesParams<'a, N, L>,
    ) {
        self.gen_moves_filtered(
            params,
            |_down: bool, _lane: i8, _idx: i8, _word: &[u8], _score: i32| true,
            |leave_value: f32| leave_value,
            |_equity: f32, _play: &Play| true,
        );
    }

    // found_word may be called multiple times for the same word.
    #[inline(always)]
    pub fn gen_remaining_words<'a, FoundWord: 'a + FnMut(&[u8]), N: kwg::Node, L: kwg::Node>(
        &mut self,
        board_snapshot: &'a BoardSnapshot<'a, N, L>,
        found_word: FoundWord,
    ) {
        let working_buffer = &mut self.working_buffer;
        working_buffer.init(board_snapshot, &[], &|leave_value: f32| leave_value);
        gen_remaining_words(board_snapshot, working_buffer, found_word)
    }
}

fn kurnia_gen_exchange_moves<
    'a,
    FoundExchangeMove: FnMut(&[u8], f32),
    N: kwg::Node,
    L: kwg::Node,
>(
    board_snapshot: &'a BoardSnapshot<'a, N, L>,
    working_buffer: &mut WorkingBuffer,
    multi_leaves: &klv::MultiLeaves,
    num_exchanges_by_this_player: i16,
    found_exchange_move: FoundExchangeMove,
) {
    if working_buffer.num_tiles_in_bag >= board_snapshot.game_config.exchange_tile_limit()
        && num_exchanges_by_this_player < board_snapshot.game_config.exchanges_allowed_per_player()
    {
        multi_leaves.kurnia_gen_exchange_moves_unconditionally(
            found_exchange_move,
            &mut working_buffer.rack_tally,
            &mut working_buffer.exchange_buffer,
            working_buffer.num_tiles_in_bag as usize,
        );
    }
}

fn kurnia_gen_place_moves_iter<
    'a,
    FoundPlaceMove: 'a + FnMut(bool, i8, i8, &[u8], i32, f32),
    CanAccept: 'a + Fn(f32) -> bool,
    N: kwg::Node,
    L: kwg::Node,
>(
    want_raw: bool,
    board_snapshot: &'a BoardSnapshot<'a, N, L>,
    working_buffer: &'a mut WorkingBuffer,
    multi_leaves: &'a klv::MultiLeaves,
    mut found_place_move: FoundPlaceMove,
    can_accept: CanAccept,
) -> impl 'a + Iterator {
    let game_config = &board_snapshot.game_config;
    let board_layout = game_config.board_layout();
    let dim = board_layout.dim();
    let max_rack_size = game_config.rack_size();
    let num_max_played = max_rack_size.min(working_buffer.num_tiles_on_rack);

    // striped by row
    for col in 0..dim.cols {
        let strip_range_start = (col as isize * dim.rows as isize) as usize;
        let strip_range_end = strip_range_start + dim.rows as usize;
        gen_cross_set(
            board_snapshot,
            &working_buffer.transposed_board_tiles[strip_range_start..strip_range_end],
            &mut working_buffer.cross_set_for_across_plays,
            dim.down(col),
            &mut working_buffer.cross_set_buffer,
            &mut working_buffer.cached_cross_set_for_across_plays
                [strip_range_start..strip_range_end],
            &mut working_buffer.used_letters_tally,
        );
    }
    let transposed_dim = matrix::Dim {
        rows: dim.cols,
        cols: dim.rows,
    };
    // striped by columns for better cache locality
    for row in 0..dim.rows {
        let strip_range_start = (row as isize * dim.cols as isize) as usize;
        let strip_range_end = strip_range_start + dim.cols as usize;
        gen_cross_set(
            board_snapshot,
            &board_snapshot.board_tiles[strip_range_start..strip_range_end],
            &mut working_buffer.cross_set_for_down_plays,
            transposed_dim.down(row),
            &mut working_buffer.cross_set_buffer,
            &mut working_buffer.cached_cross_set_for_down_plays[strip_range_start..strip_range_end],
            &mut working_buffer.used_letters_tally,
        );
    }
    if working_buffer.num_tiles_on_board == 0 {
        // empty board activates star
        let star_row = board_layout.star_row();
        let star_col = board_layout.star_col();
        if !board_layout.is_symmetric() {
            working_buffer.cross_set_for_down_plays
                [transposed_dim.at_row_col(star_col, star_row)] = CrossSet { bits: !1, score: 0 };
        }
        working_buffer.cross_set_for_across_plays[dim.at_row_col(star_row, star_col)] =
            CrossSet { bits: !1, score: 0 };
    }
    working_buffer.init_after_cross_sets(board_snapshot);
    let mut found_placements = std::mem::take(&mut working_buffer.found_placements);
    found_placements.clear();
    for row in 0..dim.rows {
        let strip_range_start = (row as isize * dim.cols as isize) as usize;
        let strip_range_end = strip_range_start + dim.cols as usize;
        gen_place_placements(
            &mut GenPlacePlacementsParams {
                board_strip: &board_snapshot.board_tiles[strip_range_start..strip_range_end],
                alphabet: board_snapshot.game_config.alphabet(),
                rack_tally: &mut working_buffer.rack_tally,
                used_tile_scores_shadowl: &mut working_buffer.used_tile_scores_shadowl,
                used_tile_scores_shadowr: &mut working_buffer.used_tile_scores_shadowr,
                shadow_strip_buffer: &mut working_buffer.word_buffer_for_across_plays
                    [strip_range_start..strip_range_end], // repurpose
                cross_set_strip: &working_buffer.cross_set_for_across_plays
                    [strip_range_start..strip_range_end],
                remaining_word_multipliers_strip: &working_buffer
                    .remaining_word_multipliers_for_across_plays
                    [strip_range_start..strip_range_end],
                remaining_tile_multipliers_strip: &working_buffer
                    .remaining_tile_multipliers_for_across_plays
                    [strip_range_start..strip_range_end],
                perpendicular_word_multipliers_strip: &working_buffer
                    .perpendicular_word_multipliers_for_across_plays
                    [strip_range_start..strip_range_end],
                perpendicular_scores_strip: &working_buffer.perpendicular_scores_for_across_plays
                    [strip_range_start..strip_range_end],
                rack_bits: working_buffer.rack_bits,
                descending_scores: &working_buffer.descending_scores,
                aggregated_word_multipliers: &mut working_buffer.aggregated_word_multipliers,
                precomputed_square_multiplier_buffer: &mut working_buffer
                    .precomputed_square_multiplier_buffer,
                indexes_to_descending_square_multiplier_buffer: &mut working_buffer
                    .indexes_to_descending_square_multiplier_buffer,
                multi_jumps_buffer: &mut working_buffer.multi_jumps_buffer,
                best_leave_values: &working_buffer.best_leave_values,
                num_max_played,
                rack_tally_shadowl: &mut working_buffer.rack_tally_shadowl,
                rack_tally_shadowr: &mut working_buffer.rack_tally_shadowr,
            },
            true,
            want_raw,
            |anchor: i8, leftmost: i8, rightmost: i8, best_possible_equity: f32| {
                found_placements.push(PossiblePlacement {
                    down: false,
                    lane: row,
                    anchor,
                    leftmost,
                    rightmost,
                    best_possible_equity,
                });
            },
        );
    }
    for col in 0..dim.cols {
        let strip_range_start = (col as isize * dim.rows as isize) as usize;
        let strip_range_end = strip_range_start + dim.rows as usize;
        gen_place_placements(
            &mut GenPlacePlacementsParams {
                board_strip: &working_buffer.transposed_board_tiles
                    [strip_range_start..strip_range_end],
                alphabet: board_snapshot.game_config.alphabet(),
                rack_tally: &mut working_buffer.rack_tally,
                used_tile_scores_shadowl: &mut working_buffer.used_tile_scores_shadowl,
                used_tile_scores_shadowr: &mut working_buffer.used_tile_scores_shadowr,
                shadow_strip_buffer: &mut working_buffer.word_buffer_for_down_plays
                    [strip_range_start..strip_range_end], // repurpose
                cross_set_strip: &working_buffer.cross_set_for_down_plays
                    [strip_range_start..strip_range_end],
                remaining_word_multipliers_strip: &working_buffer
                    .remaining_word_multipliers_for_down_plays[strip_range_start..strip_range_end],
                remaining_tile_multipliers_strip: &working_buffer
                    .remaining_tile_multipliers_for_down_plays[strip_range_start..strip_range_end],
                perpendicular_word_multipliers_strip: &working_buffer
                    .perpendicular_word_multipliers_for_down_plays
                    [strip_range_start..strip_range_end],
                perpendicular_scores_strip: &working_buffer.perpendicular_scores_for_down_plays
                    [strip_range_start..strip_range_end],
                rack_bits: working_buffer.rack_bits,
                descending_scores: &working_buffer.descending_scores,
                aggregated_word_multipliers: &mut working_buffer.aggregated_word_multipliers,
                precomputed_square_multiplier_buffer: &mut working_buffer
                    .precomputed_square_multiplier_buffer,
                indexes_to_descending_square_multiplier_buffer: &mut working_buffer
                    .indexes_to_descending_square_multiplier_buffer,
                multi_jumps_buffer: &mut working_buffer.multi_jumps_buffer,
                best_leave_values: &working_buffer.best_leave_values,
                num_max_played,
                rack_tally_shadowl: &mut working_buffer.rack_tally_shadowl,
                rack_tally_shadowr: &mut working_buffer.rack_tally_shadowr,
            },
            false,
            want_raw,
            |anchor: i8, leftmost: i8, rightmost: i8, best_possible_equity: f32| {
                found_placements.push(PossiblePlacement {
                    down: true,
                    lane: col,
                    anchor,
                    leftmost,
                    rightmost,
                    best_possible_equity,
                });
            },
        );
    }
    if !want_raw {
        // this will be iterated in reverse order, so sort by best_possible_equity increasing.
        found_placements.sort_unstable_by(|a, b| {
            a.best_possible_equity
                .partial_cmp(&b.best_possible_equity)
                .unwrap()
        });
    }
    working_buffer.found_placements = found_placements;
    std::iter::from_fn(move || match working_buffer.found_placements.pop() {
        Some(placement) => {
            if can_accept(placement.best_possible_equity) {
                gen_place_moves_at(
                    board_snapshot,
                    working_buffer,
                    multi_leaves,
                    &placement,
                    num_max_played,
                    &mut |down: bool,
                          lane: i8,
                          idx: i8,
                          word: &[u8],
                          score: i32,
                          leave_value: f32| {
                        let this_best = score as f32 + leave_value;
                        debug_assert!(
                            this_best <= placement.best_possible_equity,
                            "found {} when expecting up to {} for ({}, {}, {}, {:?}, {}, {})",
                            this_best,
                            placement.best_possible_equity,
                            down,
                            lane,
                            idx,
                            word,
                            score,
                            leave_value,
                        );
                        found_place_move(down, lane, idx, word, score, leave_value)
                    },
                );
                Some(())
            } else {
                // fuse the iterator
                working_buffer.found_placements.clear();
                None
            }
        }
        None => None,
    })
}

struct GenRemainingConnectedWordsParams<'a, N: kwg::Node> {
    board_strip: &'a [u8],
    rack_tally: &'a mut [u8],
    word_strip_buffer: &'a mut [u8],
    kwg: &'a kwg::Kwg<N>,
}

// note: this basic word prune algorithm does not consider hooks yet.
fn gen_remaining_connected_words<
    'a,
    FoundWord: 'a + FnMut(&[u8]),
    FoundSpace: 'a + FnMut(u8),
    N: kwg::Node,
>(
    params: &'a mut GenRemainingConnectedWordsParams<'a, N>,
    found_word: FoundWord,
    mut found_space: FoundSpace,
) {
    params
        .word_strip_buffer
        .iter_mut()
        .zip(params.board_strip.iter().map(|x| x & 0x7f))
        .for_each(|(m, v)| *m = v);

    struct Env<'a, FoundWord: 'a + FnMut(&[u8]), N: kwg::Node> {
        params: &'a mut GenRemainingConnectedWordsParams<'a, N>,
        found_word: FoundWord,
        anchor: i8,
        rightmost: i8,
        num_played: i8,
        idx_left: i8,
    }

    fn record<FoundWord: FnMut(&[u8]), N: kwg::Node>(
        env: &mut Env<'_, FoundWord, N>,
        idx_left: i8,
        idx_right: i8,
    ) {
        (env.found_word)(&env.params.word_strip_buffer[idx_left as usize..idx_right as usize]);
    }

    fn play_right<FoundWord: FnMut(&[u8]), N: kwg::Node>(
        env: &mut Env<'_, FoundWord, N>,
        mut p: i32,
        mut idx: i8,
    ) {
        // tail-recurse placing current sequence of tiles
        while idx < env.rightmost {
            let b = env.params.board_strip[idx as usize];
            if b == 0 {
                break;
            }
            p = env.params.kwg.seek(p, b & 0x7f);
            if p <= 0 {
                return;
            }
            idx += 1;
        }
        let node = env.params.kwg[p];
        if idx > env.anchor + 1 && idx - env.idx_left >= 2 && node.accepts() {
            record(env, env.idx_left, idx);
        }

        if idx < env.rightmost {
            p = node.arc_index();
            if p <= 0 {
                return;
            }
            loop {
                let node = env.params.kwg[p];
                let tile = node.tile();
                if env.params.rack_tally[tile as usize] > 0 {
                    env.params.rack_tally[tile as usize] -= 1;
                    env.params.word_strip_buffer[idx as usize] = tile;
                    play_right(env, p, idx + 1);
                    env.params.rack_tally[tile as usize] += 1;
                } else if env.params.rack_tally[0] > 0 {
                    env.params.rack_tally[0] -= 1;
                    env.params.word_strip_buffer[idx as usize] = tile; // not blanked for kwg.
                    play_right(env, p, idx + 1);
                    env.params.rack_tally[0] += 1;
                }
                if node.is_end() {
                    break;
                }
                p += 1;
            }
        }
    }

    fn play_left<FoundWord: FnMut(&[u8]), N: kwg::Node>(
        env: &mut Env<'_, FoundWord, N>,
        mut p: i32,
        mut idx: i8,
    ) {
        // tail-recurse placing current sequence of tiles
        while idx >= 0 {
            let b = env.params.board_strip[idx as usize];
            if b == 0 {
                break;
            }
            p = env.params.kwg.seek(p, b & 0x7f);
            if p <= 0 {
                return;
            }
            idx -= 1;
        }
        let mut node = env.params.kwg[p];
        if env.num_played > 0 && env.anchor - idx >= 2 && node.accepts() {
            record(env, idx + 1, env.anchor + 1);
        }

        p = node.arc_index();
        if p <= 0 {
            return;
        }

        node = env.params.kwg[p];
        if node.tile() == 0 {
            // assume idx < env.anchor, because tile 0 does not occur at start in well-formed kwg gaddawg
            env.idx_left = idx + 1;
            play_right(env, p, env.anchor + 1);
            if node.is_end() {
                return;
            }
            p += 1;
        }

        if idx >= 0 {
            loop {
                let node = env.params.kwg[p];
                let tile = node.tile();
                if env.params.rack_tally[tile as usize] > 0 {
                    env.params.rack_tally[tile as usize] -= 1;
                    env.num_played += 1;
                    env.params.word_strip_buffer[idx as usize] = tile;
                    play_left(env, p, idx - 1);
                    env.num_played -= 1;
                    env.params.rack_tally[tile as usize] += 1;
                } else if env.params.rack_tally[0] > 0 {
                    env.params.rack_tally[0] -= 1;
                    env.num_played += 1;
                    env.params.word_strip_buffer[idx as usize] = tile; // not blanked for kwg.
                    play_left(env, p, idx - 1);
                    env.num_played -= 1;
                    env.params.rack_tally[0] += 1;
                }
                if node.is_end() {
                    break;
                }
                p += 1;
            }
        }
    }

    let strider_len = params.board_strip.len();
    let mut env = Env {
        params,
        found_word,
        anchor: 0,
        rightmost: 0,
        num_played: 0,
        idx_left: 0,
    };
    let mut leftmost = strider_len as i8; // processed up to here
    loop {
        env.rightmost = leftmost;
        while leftmost > 0 && env.params.board_strip[leftmost as usize - 1] == 0 {
            leftmost -= 1;
        }
        found_space((env.rightmost - leftmost - ((leftmost > 0) as i8)).max(0) as u8); // leftmost>0 requires gap from next word.
        if leftmost > 0 {
            // board[leftmost - 1] is a tile.
            env.anchor = leftmost - 1;
            // board[anchor + 1] is empty or off-board, board[anchor] has a tile.
            let mut p = 1;
            while leftmost > 0 && env.params.board_strip[leftmost as usize - 1] != 0 {
                leftmost -= 1;
                p = env
                    .params
                    .kwg
                    .seek(p, env.params.board_strip[leftmost as usize] & 0x7f);
            }
            // board[leftmost] has a tile, board[leftmost - 1] is empty or off-board.
            if p >= 0 {
                play_left(&mut env, p, leftmost - 1);
            }
        }
        // board[leftmost] was leftmost tile. need gap from previous word.
        leftmost -= 1;
        // now board[leftmost] is empty.
        if leftmost <= 1 {
            // assume words are >= 2.
            break;
        }
    }
    env.params.word_strip_buffer.iter_mut().for_each(|m| *m = 0);
}

struct GenRemainingUnconnectedWordsParams<'a, N: kwg::Node> {
    rack_tally: &'a mut [u8],
    word_vec: &'a mut Vec<u8>,
    kwg: &'a kwg::Kwg<N>,
    max_len: usize,
}

fn gen_remaining_unconnected_words<'a, FoundWord: 'a + FnMut(&[u8]), N: kwg::Node>(
    params: &'a mut GenRemainingUnconnectedWordsParams<'a, N>,
    found_word: FoundWord,
) {
    params.word_vec.clear();
    params.word_vec.reserve(params.max_len);
    struct Env<'a, FoundWord: 'a + FnMut(&[u8]), N: kwg::Node> {
        rack_tally: &'a mut [u8],
        word_vec: &'a mut Vec<u8>,
        kwg: &'a kwg::Kwg<N>,
        max_len: usize,
        found_word: FoundWord,
    }
    fn iter<FoundWord: FnMut(&[u8]), N: kwg::Node>(env: &mut Env<'_, FoundWord, N>, mut p: i32) {
        if env.word_vec.len() >= env.max_len {
            return;
        }
        loop {
            let node = &env.kwg[p];
            let tile = node.tile();
            if env.rack_tally[tile as usize] > 0 {
                env.rack_tally[tile as usize] -= 1;
                env.word_vec.push(tile);
                if node.accepts() {
                    (env.found_word)(env.word_vec);
                }
                let np = node.arc_index();
                if np != 0 {
                    iter(env, np);
                }
                env.word_vec.pop();
                env.rack_tally[tile as usize] += 1;
            } else if env.rack_tally[0] > 0 {
                env.rack_tally[0] -= 1;
                env.word_vec.push(tile);
                if node.accepts() {
                    (env.found_word)(env.word_vec);
                }
                let np = node.arc_index();
                if np != 0 {
                    iter(env, np);
                }
                env.word_vec.pop();
                env.rack_tally[0] += 1;
            }
            if node.is_end() {
                break;
            }
            p += 1;
        }
    }
    iter(
        &mut Env {
            rack_tally: params.rack_tally,
            word_vec: params.word_vec,
            kwg: params.kwg,
            max_len: params.max_len,
            found_word,
        },
        params.kwg[0].arc_index(),
    );
}

// found_word may be called multiple times for the same word.
fn gen_remaining_words<'a, FoundWord: 'a + FnMut(&[u8]), N: kwg::Node, L: kwg::Node>(
    board_snapshot: &'a BoardSnapshot<'a, N, L>,
    working_buffer: &'a mut WorkingBuffer,
    mut found_word: FoundWord,
) {
    let game_config = &board_snapshot.game_config;
    let board_layout = game_config.board_layout();
    let dim = board_layout.dim();
    let alphabet = game_config.alphabet();

    let available_tally = &mut working_buffer.used_letters_tally;
    available_tally.clear();
    available_tally.reserve(alphabet.len() as usize);
    for i in 0..alphabet.len() {
        available_tally.push(alphabet.freq(i));
    }
    // should check underflow.
    for i in board_snapshot.board_tiles.iter() {
        if *i != 0 {
            if i & 0x80 == 0 {
                available_tally[*i as usize] -= 1;
            } else {
                available_tally[0] -= 1;
            }
        }
    }
    let mut max_space_len = 0;
    let mut found_space = |space_len: u8| max_space_len = max_space_len.max(space_len);
    for row in 0..dim.rows {
        let strip_range_start = (row as isize * dim.cols as isize) as usize;
        let strip_range_end = strip_range_start + dim.cols as usize;
        gen_remaining_connected_words(
            &mut GenRemainingConnectedWordsParams {
                board_strip: &board_snapshot.board_tiles[strip_range_start..strip_range_end],
                rack_tally: &mut working_buffer.used_letters_tally, // intentional.
                word_strip_buffer: &mut working_buffer.word_buffer_for_across_plays
                    [strip_range_start..strip_range_end],
                kwg: board_snapshot.kwg,
            },
            &mut found_word,
            &mut found_space,
        );
    }
    for col in 0..dim.cols {
        let strip_range_start = (col as isize * dim.rows as isize) as usize;
        let strip_range_end = strip_range_start + dim.rows as usize;
        gen_remaining_connected_words(
            &mut GenRemainingConnectedWordsParams {
                board_strip: &working_buffer.transposed_board_tiles
                    [strip_range_start..strip_range_end],
                rack_tally: &mut working_buffer.used_letters_tally, // intentional.
                word_strip_buffer: &mut working_buffer.word_buffer_for_down_plays
                    [strip_range_start..strip_range_end],
                kwg: board_snapshot.kwg,
            },
            &mut found_word,
            &mut found_space,
        );
    }
    gen_remaining_unconnected_words(
        &mut GenRemainingUnconnectedWordsParams {
            kwg: board_snapshot.kwg,
            rack_tally: &mut working_buffer.used_letters_tally, // intentional.
            word_vec: &mut working_buffer.exchange_buffer,      // intentional.
            max_len: max_space_len as usize,
        },
        &mut found_word,
    );
}
