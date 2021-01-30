// Copyright (C) 2020-2021 Andy Kurnia.

use super::{alphabet, bites, game_config, klv, kwg, matrix};

#[derive(Clone)]
struct CrossSet {
    bits: u64,
    score: i16,
}

#[derive(Clone, Copy)]
struct CachedCrossSet {
    p_left: i32,
    p_right: i32,
    bits: u64,
}

#[derive(Clone)]
struct CrossSetComputation {
    score: i16,
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
    perpendicular_scores_for_across_plays: Box<[i16]>, // r*c (multiplied by perpendicular_word_multipliers)
    perpendicular_scores_for_down_plays: Box<[i16]>,   // c*r
    transposed_board_tiles: Box<[u8]>,                 // c*r
    num_tiles_on_board: u16,
    num_tiles_in_bag: i16, // negative when players also have less than full racks
    play_out_bonus: i16,
    num_tiles_on_rack: u8,
    rack_bits: u64, // bit 0 = blank conveniently matches bit 0 = have cross set
    descending_scores: Vec<i8>, // rack.len()
    exchange_buffer: Vec<u8>, // rack.len()
    square_multipliers_by_aggregated_word_multipliers_buffer: std::collections::HashMap<i16, usize>,
    precomputed_square_multiplier_buffer: Vec<i16>,
    indexes_to_descending_square_multiplier_buffer: Vec<i8>,
    best_leave_values: Vec<f32>, // rack.len() + 1
    found_placements: Vec<PossiblePlacement>,
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
            play_out_bonus: self.play_out_bonus,
            num_tiles_on_rack: self.num_tiles_on_rack,
            rack_bits: self.rack_bits,
            descending_scores: self.descending_scores.clone(),
            exchange_buffer: self.exchange_buffer.clone(),
            square_multipliers_by_aggregated_word_multipliers_buffer: self
                .square_multipliers_by_aggregated_word_multipliers_buffer
                .clone(),
            precomputed_square_multiplier_buffer: self.precomputed_square_multiplier_buffer.clone(),
            indexes_to_descending_square_multiplier_buffer: self
                .indexes_to_descending_square_multiplier_buffer
                .clone(),
            best_leave_values: self.best_leave_values.clone(),
            found_placements: self.found_placements.clone(),
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
        self.play_out_bonus.clone_from(&source.play_out_bonus);
        self.num_tiles_on_rack.clone_from(&source.num_tiles_on_rack);
        self.rack_bits.clone_from(&source.rack_bits);
        self.descending_scores.clone_from(&source.descending_scores);
        self.exchange_buffer.clone_from(&source.exchange_buffer);
        self.square_multipliers_by_aggregated_word_multipliers_buffer
            .clone_from(&source.square_multipliers_by_aggregated_word_multipliers_buffer);
        self.precomputed_square_multiplier_buffer
            .clone_from(&source.precomputed_square_multiplier_buffer);
        self.indexes_to_descending_square_multiplier_buffer
            .clone_from(&source.indexes_to_descending_square_multiplier_buffer);
        self.best_leave_values.clone_from(&source.best_leave_values);
        self.found_placements.clone_from(&source.found_placements);
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
                    bits: 0
                };
                rows_times_cols
            ]
            .into_boxed_slice(),
            cached_cross_set_for_down_plays: vec![
                CachedCrossSet {
                    p_left: 0,
                    p_right: 0,
                    bits: 0
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
                std::cmp::max(dim.rows, dim.cols) as usize
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
            perpendicular_scores_for_across_plays: vec![0i16; rows_times_cols].into_boxed_slice(),
            perpendicular_scores_for_down_plays: vec![0i16; rows_times_cols].into_boxed_slice(),
            transposed_board_tiles: vec![0u8; rows_times_cols].into_boxed_slice(),
            num_tiles_on_board: 0,
            num_tiles_in_bag: 0,
            play_out_bonus: 0,
            num_tiles_on_rack: 0,
            rack_bits: 0,
            descending_scores: Vec::new(),
            exchange_buffer: Vec::new(),
            square_multipliers_by_aggregated_word_multipliers_buffer:
                std::collections::HashMap::new(),
            precomputed_square_multiplier_buffer: Vec::new(),
            indexes_to_descending_square_multiplier_buffer: Vec::new(),
            best_leave_values: Vec::new(),
            found_placements: Vec::new(),
        }
    }

    fn init(&mut self, board_snapshot: &BoardSnapshot<'_>, rack: &[u8]) {
        self.exchange_buffer.clear();
        self.exchange_buffer.reserve(rack.len());
        self.rack_tally.iter_mut().for_each(|m| *m = 0);
        for tile in &rack[..] {
            self.rack_tally[*tile as usize] += 1;
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

        let alphabet = board_snapshot.game_config.alphabet();
        let board_layout = board_snapshot.game_config.board_layout();
        let dim = board_layout.dim();
        let premiums = board_layout.premiums();
        let transposed_premiums = board_layout.transposed_premiums();
        for row in 0..dim.rows {
            let strip_range_start = (row as isize * dim.cols as isize) as usize;
            for col in 0..dim.cols {
                let idx = strip_range_start + col as usize;
                let b = board_snapshot.board_tiles[idx];
                if b == 0 {
                    let premium = premiums[idx];
                    self.remaining_word_multipliers_for_across_plays[idx] = premium.word_multiplier;
                    self.remaining_tile_multipliers_for_across_plays[idx] = premium.tile_multiplier;
                    self.face_value_scores_for_across_plays[idx] = 0;
                } else {
                    self.remaining_word_multipliers_for_across_plays[idx] = 1; // needed for the HashMap
                    self.remaining_tile_multipliers_for_across_plays[idx] = 1; // not as crucial to set to 1
                    self.face_value_scores_for_across_plays[idx] = alphabet.score(b);
                }
            }
        }
        for col in 0..dim.cols {
            for row in 0..dim.rows {
                self.transposed_board_tiles
                    [(col as isize * dim.rows as isize + row as isize) as usize] = board_snapshot
                    .board_tiles[(row as isize * dim.cols as isize + col as isize) as usize];
            }
        }
        for col in 0..dim.cols {
            let strip_range_start = (col as isize * dim.rows as isize) as usize;
            for row in 0..dim.rows {
                let idx = strip_range_start + row as usize;
                let b = self.transposed_board_tiles[idx];
                if b == 0 {
                    let premium = transposed_premiums[idx];
                    self.remaining_word_multipliers_for_down_plays[idx] = premium.word_multiplier;
                    self.remaining_tile_multipliers_for_down_plays[idx] = premium.tile_multiplier;
                    self.face_value_scores_for_down_plays[idx] = 0;
                } else {
                    self.remaining_word_multipliers_for_down_plays[idx] = 1; // needed for the HashMap
                    self.remaining_tile_multipliers_for_down_plays[idx] = 1; // not as crucial to set to 1
                    self.face_value_scores_for_down_plays[idx] = alphabet.score(b);
                }
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
        self.play_out_bonus = if self.num_tiles_in_bag <= 0 {
            2 * ((0u8..)
                .zip(self.rack_tally.iter())
                .map(|(tile, &num)| {
                    (alphabet.freq(tile) as i16 - num as i16) * alphabet.score(tile) as i16
                })
                .sum::<i16>()
                - board_snapshot
                    .board_tiles
                    .iter()
                    .map(|&t| if t != 0 { alphabet.score(t) as i16 } else { 0 })
                    .sum::<i16>())
        } else {
            0
        };

        // eg if my rack is ZY??YVA it'd be [10,4,4,4,1,1,0].
        self.num_tiles_on_rack = 0;
        self.rack_bits = 0u64;
        for (tile, &count) in (0u8..).zip(self.rack_tally.iter()) {
            self.num_tiles_on_rack += count;
            self.rack_bits |= ((count != 0) as u64) << tile;
        }
        self.descending_scores.clear();
        self.descending_scores
            .reserve(self.num_tiles_on_rack as usize);
        for (tile, &count) in (0u8..).zip(self.rack_tally.iter()) {
            if count != 0 {
                let score = alphabet.score(tile);
                for _ in 0..count {
                    self.descending_scores.push(score);
                }
            }
        }
        self.descending_scores.sort_unstable();
        self.descending_scores.reverse();

        self.best_leave_values.clear();
        self.best_leave_values
            .resize(self.num_tiles_on_rack as usize + 1, f32::NEG_INFINITY);
        if self.num_tiles_in_bag <= 0 {
            let mut unpaid = 0i16;
            for i in (0..self.num_tiles_on_rack).rev() {
                unpaid += self.descending_scores[i as usize] as i16;
                self.best_leave_values[i as usize] = (-10 - 2 * unpaid) as f32;
            }
            self.best_leave_values[self.num_tiles_on_rack as usize] = self.play_out_bonus as f32;
        } else {
            struct Env<'a> {
                klv: &'a klv::Klv,
                best_leave_values: &'a mut [f32],
                rack_tally: &'a mut [u8],
            }
            #[inline(always)]
            fn pretend_to_generate_exchanges(
                mut env: &mut Env<'_>,
                mut num_tiles_exchanged: u16,
                mut idx: u8,
            ) {
                let rack_tally_len = env.rack_tally.len();
                while (idx as usize) < rack_tally_len && env.rack_tally[idx as usize] == 0 {
                    idx += 1;
                }
                if idx as usize >= rack_tally_len {
                    // note: tilt not applied, assume 0.0 <= tilt <= 1.0
                    let this_leave_value = env.klv.leave_value_from_tally(env.rack_tally);
                    if this_leave_value > env.best_leave_values[num_tiles_exchanged as usize] {
                        env.best_leave_values[num_tiles_exchanged as usize] = this_leave_value;
                    }
                    return;
                }
                let original_count = env.rack_tally[idx as usize];
                loop {
                    pretend_to_generate_exchanges(&mut env, num_tiles_exchanged, idx + 1);
                    if env.rack_tally[idx as usize] == 0 {
                        break;
                    }
                    env.rack_tally[idx as usize] -= 1;
                    num_tiles_exchanged += 1;
                }
                env.rack_tally[idx as usize] = original_count;
            }
            pretend_to_generate_exchanges(
                &mut Env {
                    klv: &board_snapshot.klv,
                    best_leave_values: &mut self.best_leave_values,
                    rack_tally: &mut self.rack_tally,
                },
                0,
                0,
            );
        }
        for i in 0..=self.num_tiles_on_rack {
            self.best_leave_values[i as usize] +=
                board_snapshot.game_config.num_played_bonus(i as i8) as f32;
        }
    }

    fn init_after_cross_sets(&mut self, board_snapshot: &BoardSnapshot<'_>) {
        let board_layout = board_snapshot.game_config.board_layout();
        let dim = board_layout.dim();
        for row in 0..dim.rows {
            let strip_range_start = (row as isize * dim.cols as isize) as usize;
            for col in 0..dim.cols {
                let idx = strip_range_start + col as usize;
                let cross_set = &self.cross_set_for_across_plays[idx];
                let effective_pwm = self.remaining_word_multipliers_for_across_plays[idx]
                    & -(cross_set.bits as i8 & 1);
                self.perpendicular_word_multipliers_for_across_plays[idx] = effective_pwm;
                self.perpendicular_scores_for_across_plays[idx] =
                    cross_set.score * effective_pwm as i16;
            }
        }
        for col in 0..dim.cols {
            let strip_range_start = (col as isize * dim.rows as isize) as usize;
            for row in 0..dim.rows {
                let idx = strip_range_start + row as usize;
                let cross_set = &self.cross_set_for_down_plays[idx];
                let effective_pwm = self.remaining_word_multipliers_for_down_plays[idx]
                    & -(cross_set.bits as i8 & 1);
                self.perpendicular_word_multipliers_for_down_plays[idx] = effective_pwm;
                self.perpendicular_scores_for_down_plays[idx] =
                    cross_set.score * effective_pwm as i16;
            }
        }
    }
}

pub struct BoardSnapshot<'a> {
    pub board_tiles: &'a [u8],
    pub game_config: &'a game_config::GameConfig<'a>,
    pub kwg: &'a kwg::Kwg,
    pub klv: &'a klv::Klv,
}

// cached_cross_sets is just one strip, so it is transposed from cross_sets
fn gen_cross_set<'a>(
    board_snapshot: &'a BoardSnapshot<'a>,
    board_strip: &'a [u8],
    cross_sets: &'a mut [CrossSet],
    output_strider: matrix::Strider,
    cross_set_buffer: &'a mut [CrossSetComputation],
    mut cached_cross_sets: &'a mut [CachedCrossSet],
) {
    let len = output_strider.len();
    let step = output_strider.step() as usize;
    let kwg = &board_snapshot.kwg;
    let mut last_nonempty = len;
    {
        let alphabet = board_snapshot.game_config.alphabet();
        let mut p = 1;
        let mut score = 0i16;
        let mut last_empty = len;
        for j in (0..len).rev() {
            let b = board_strip[j as usize];
            if b != 0 {
                let b_letter = b & 0x7f;
                p = kwg.seek(p, b_letter);
                score += alphabet.score(b) as i16;
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
            let mut bits = reuse_cross_set(&mut cached_cross_sets, j - 1, p_left, p_right);
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
        let mut bits = reuse_cross_set(&mut cached_cross_sets, j, p, -2);
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

struct GenPlacePlacementsParams<'a> {
    board_strip: &'a [u8],
    cross_set_strip: &'a [CrossSet],
    remaining_word_multipliers_strip: &'a [i8],
    face_value_scores_strip: &'a [i8],
    perpendicular_scores_strip: &'a [i16],
    rack_bits: u64,
    descending_scores: &'a [i8],
    square_multipliers_by_aggregated_word_multipliers_buffer:
        &'a mut std::collections::HashMap<i16, usize>,
    precomputed_square_multiplier_buffer: &'a mut Vec<i16>,
    indexes_to_descending_square_multiplier_buffer: &'a mut Vec<i8>,
    best_leave_values: &'a [f32],
    num_max_played: u8,
}

fn gen_place_placements<'a, PossibleStripPlacementCallbackType: FnMut(i8, i8, i8, f32)>(
    want_raw: bool,
    params: &'a mut GenPlacePlacementsParams<'a>,
    remaining_tile_multipliers_strip: &'a [i8],
    perpendicular_word_multipliers_strip: &'a [i8],
    num_tiles_on_rack: u8,
    single_tile_plays: bool,
    mut possible_strip_placement_callback: PossibleStripPlacementCallbackType,
) {
    let strider_len = params.board_strip.len();

    if num_tiles_on_rack < params.num_max_played {
        params.num_max_played = num_tiles_on_rack;
    }

    if !want_raw {
        params
            .square_multipliers_by_aggregated_word_multipliers_buffer
            .clear();
        let mut vec_size = 0usize;
        for i in 0..strider_len {
            let mut wm = 1;
            for &wm_val in &params.remaining_word_multipliers_strip[i..strider_len] {
                wm *= wm_val as i16;
                if let std::collections::hash_map::Entry::Vacant(entry) = params
                    .square_multipliers_by_aggregated_word_multipliers_buffer
                    .entry(wm)
                {
                    entry.insert(vec_size);
                    vec_size += strider_len;
                }
            }
        }
        params.precomputed_square_multiplier_buffer.clear();
        params
            .precomputed_square_multiplier_buffer
            .resize(vec_size, 0);
        params
            .indexes_to_descending_square_multiplier_buffer
            .clear();
        params
            .indexes_to_descending_square_multiplier_buffer
            .resize(vec_size, 0);
        for (k, &low_end) in params
            .square_multipliers_by_aggregated_word_multipliers_buffer
            .iter()
        {
            // k is the aggregated main word multiplier.
            // low_end is the index of the strider_len-length slice.
            let high_end = low_end + strider_len;
            let precomputed_square_multiplier_slice =
                &mut params.precomputed_square_multiplier_buffer[low_end..high_end];
            let indexes_to_descending_square_multiplier_slice =
                &mut params.indexes_to_descending_square_multiplier_buffer[low_end..high_end];
            for j in 0..strider_len {
                // perpendicular_word_multipliers_strip[j] is 0 if no perpendicular tile.
                precomputed_square_multiplier_slice[j] = remaining_tile_multipliers_strip[j] as i16
                    * (k + perpendicular_word_multipliers_strip[j] as i16);
                indexes_to_descending_square_multiplier_slice[j] = j as i8;
            }
            indexes_to_descending_square_multiplier_slice.sort_unstable_by(|&a, &b| {
                precomputed_square_multiplier_slice[b as usize]
                    .cmp(&precomputed_square_multiplier_slice[a as usize])
            });
        }
    }

    struct Env<'a> {
        params: &'a GenPlacePlacementsParams<'a>,
        strider_len: usize,
        anchor: i8,
        leftmost: i8,
        rightmost: i8,
        num_played: i8,
        idx_left: i8,
        best_possible_equity: f32,
    }

    let mut env = Env {
        params,
        strider_len,
        anchor: 0,
        leftmost: 0,
        rightmost: 0,
        num_played: 0,
        idx_left: 0,
        best_possible_equity: f32::NEG_INFINITY,
    };

    fn shadow_record(
        env: &mut Env,
        idx_left: i8,
        idx_right: i8,
        main_played_through_score: i16,
        perpendicular_additional_score: i16,
        word_multiplier: i16,
    ) {
        let low_end = env
            .params
            .square_multipliers_by_aggregated_word_multipliers_buffer[&word_multiplier];
        let high_end = low_end + env.strider_len;
        let precomputed_square_multiplier_slice =
            &env.params.precomputed_square_multiplier_buffer[low_end..high_end];
        let mut desc_scores_iter = env.params.descending_scores.iter();
        let mut best_scoring = 0;
        let mut to_assign = env.num_played;
        for &idx in &env.params.indexes_to_descending_square_multiplier_buffer[low_end..high_end] {
            if idx_left <= idx && idx < idx_right && env.params.board_strip[idx as usize] == 0 {
                best_scoring += *desc_scores_iter.next().unwrap() as i16
                    * precomputed_square_multiplier_slice[idx as usize] as i16;
                to_assign -= 1;
                if to_assign == 0 {
                    break;
                }
            }
        }
        let equity = (main_played_through_score * word_multiplier
            + perpendicular_additional_score
            + best_scoring) as f32
            + env.params.best_leave_values[env.num_played as usize];
        if equity > env.best_possible_equity {
            env.best_possible_equity = equity;
        }
    }

    fn shadow_play_right(
        env: &mut Env,
        mut idx: i8,
        mut main_played_through_score: i16,
        perpendicular_additional_score: i16,
        word_multiplier: i16,
        is_unique: bool,
    ) {
        // tail-recurse placing current sequence of tiles
        while idx < env.rightmost {
            let b = env.params.board_strip[idx as usize];
            if b == 0 {
                break;
            }
            main_played_through_score += env.params.face_value_scores_strip[idx as usize] as i16;
            idx += 1;
        }
        // tiles have been placed from env.idx_left to idx - 1.
        // here idx <= env.rightmost.
        // check if [env.idx_left, idx) is a thing
        if idx > env.anchor + 1
            && (env.num_played + is_unique as i8) >= 2
            && idx - env.idx_left >= 2
        {
            shadow_record(
                env,
                env.idx_left,
                idx,
                main_played_through_score,
                perpendicular_additional_score,
                word_multiplier,
            );
        }
        if env.num_played as u8 >= env.params.num_max_played {
            return;
        }

        // place a tile at [idx] if it is still in bounds
        if idx < env.rightmost {
            let this_cross_bits = env.params.cross_set_strip[idx as usize].bits;
            if this_cross_bits & 1 == 0 {
                // nothing hooks here
                env.num_played += 1;
                shadow_play_right(
                    env,
                    idx + 1,
                    main_played_through_score,
                    perpendicular_additional_score,
                    word_multiplier
                        * env.params.remaining_word_multipliers_strip[idx as usize] as i16,
                    true,
                );
                env.num_played -= 1;
            } else if this_cross_bits & env.params.rack_bits != 0 {
                // something hooks here
                // rack_bits remains unchanged because assignment is tentative.
                env.num_played += 1;
                shadow_play_right(
                    env,
                    idx + 1,
                    main_played_through_score,
                    perpendicular_additional_score
                        + env.params.perpendicular_scores_strip[idx as usize],
                    word_multiplier
                        * env.params.remaining_word_multipliers_strip[idx as usize] as i16,
                    is_unique,
                );
                env.num_played -= 1;
            }
        }
    }

    fn shadow_play_left(
        env: &mut Env,
        mut idx: i8,
        mut main_played_through_score: i16,
        perpendicular_additional_score: i16,
        word_multiplier: i16,
        is_unique: bool,
    ) {
        // tail-recurse placing current sequence of tiles
        while idx >= env.leftmost {
            let b = env.params.board_strip[idx as usize];
            if b == 0 {
                break;
            }
            main_played_through_score += env.params.face_value_scores_strip[idx as usize] as i16;
            idx -= 1;
        }
        // tiles have been placed from env.anchor to idx + 1.
        // here idx >= env.leftmost - 1.
        // check if [idx + 1, env.anchor + 1) is a thing
        if (env.num_played + is_unique as i8) >= 2 && env.anchor - idx >= 2 {
            shadow_record(
                env,
                idx + 1,
                env.anchor + 1,
                main_played_through_score,
                perpendicular_additional_score,
                word_multiplier,
            );
        }
        if env.num_played as u8 >= env.params.num_max_played {
            return;
        }

        // can switch direction only after using the anchor square
        if idx < env.anchor {
            env.idx_left = idx + 1;
            shadow_play_right(
                env,
                env.anchor + 1,
                main_played_through_score,
                perpendicular_additional_score,
                word_multiplier,
                is_unique,
            );
        }

        // place a tile at [idx] if it is still in bounds
        if idx >= env.leftmost {
            let this_cross_bits = env.params.cross_set_strip[idx as usize].bits;
            if this_cross_bits & 1 == 0 {
                // nothing hooks here
                env.num_played += 1;
                shadow_play_left(
                    env,
                    idx - 1,
                    main_played_through_score,
                    perpendicular_additional_score,
                    word_multiplier
                        * env.params.remaining_word_multipliers_strip[idx as usize] as i16,
                    true,
                );
                env.num_played -= 1;
            } else if this_cross_bits & env.params.rack_bits != 0 {
                // something hooks here
                // rack_bits remains unchanged because assignment is tentative.
                env.num_played += 1;
                shadow_play_left(
                    env,
                    idx - 1,
                    main_played_through_score,
                    perpendicular_additional_score
                        + env.params.perpendicular_scores_strip[idx as usize],
                    word_multiplier
                        * env.params.remaining_word_multipliers_strip[idx as usize] as i16,
                    is_unique,
                );
                env.num_played -= 1;
            }
        }
    }

    #[inline(always)]
    fn gen_moves_from<PossibleStripPlacementCallbackType: FnMut(i8, i8, i8, f32)>(
        want_raw: bool,
        env: &mut Env,
        single_tile_plays: bool,
        mut possible_strip_placement_callback: PossibleStripPlacementCallbackType,
    ) {
        if want_raw {
            possible_strip_placement_callback(env.anchor, env.leftmost, env.rightmost, 0.0);
        } else {
            env.best_possible_equity = f32::NEG_INFINITY;
            shadow_play_left(env, env.anchor, 0, 0, 1, single_tile_plays);
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

    let mut rightmost = strider_len as i8; // processed up to here
    let mut leftmost = rightmost;
    loop {
        while leftmost > 0 && params.board_strip[leftmost as usize - 1] == 0 {
            leftmost -= 1;
        }
        if leftmost > 0 {
            // board[leftmost - 1] is a tile.
            env.anchor = leftmost - 1;
            env.leftmost = 0;
            env.rightmost = rightmost;
            gen_moves_from(
                want_raw,
                &mut env,
                single_tile_plays,
                &mut possible_strip_placement_callback,
            );
        }
        {
            // this part is only relevant if rack has at least two tiles, but passing that is too expensive.
            let leftmost = leftmost + (leftmost > 0) as i8; // shadowing
            for anchor in (leftmost..rightmost).rev() {
                let cross_set_bits = params.cross_set_strip[anchor as usize].bits;
                if cross_set_bits != 0 {
                    if rightmost - leftmost < 2 {
                        // not enough room for 2-tile words
                        break;
                    }
                    if cross_set_bits != 1 {
                        env.anchor = anchor;
                        env.leftmost = leftmost;
                        env.rightmost = rightmost;
                        gen_moves_from(
                            want_raw,
                            &mut env,
                            single_tile_plays,
                            &mut possible_strip_placement_callback,
                        );
                    }
                    rightmost = anchor; // prevent duplicates
                }
            }
        }
        while leftmost > 0 && params.board_strip[leftmost as usize - 1] != 0 {
            leftmost -= 1;
        }
        if leftmost <= 1 {
            break;
        }
        rightmost = leftmost - 1; // prevent touching leftmost tile
    }
}

struct GenPlaceMovesParams<'a, CallbackType: FnMut(i8, &[u8], i16, &[u8])> {
    board_snapshot: &'a BoardSnapshot<'a>,
    board_strip: &'a [u8],
    cross_set_strip: &'a [CrossSet],
    remaining_word_multipliers_strip: &'a [i8],
    remaining_tile_multipliers_strip: &'a [i8],
    face_value_scores_strip: &'a [i8],
    perpendicular_word_multipliers_strip: &'a [i8],
    perpendicular_scores_strip: &'a [i16],
    rack_tally: &'a mut [u8],
    word_strip_buffer: &'a mut [u8],
    num_max_played: u8,
    anchor: i8,
    leftmost: i8,
    rightmost: i8,
    callback: CallbackType,
}

fn gen_place_moves<'a, CallbackType: FnMut(i8, &[u8], i16, &[u8])>(
    params: &'a mut GenPlaceMovesParams<'a, CallbackType>,
    single_tile_plays: bool,
) {
    struct Env<'a, CallbackType: FnMut(i8, &[u8], i16, &[u8])> {
        params: &'a mut GenPlaceMovesParams<'a, CallbackType>,
        alphabet: &'a alphabet::Alphabet<'a>,
        num_played: i8,
        idx_left: i8,
    }

    fn record<CallbackType: FnMut(i8, &[u8], i16, &[u8])>(
        env: &mut Env<CallbackType>,
        idx_left: i8,
        idx_right: i8,
        main_score: i16,
        perpendicular_cumulative_score: i16,
        word_multiplier: i16,
    ) {
        let score = main_score * word_multiplier
            + perpendicular_cumulative_score
            + env
                .params
                .board_snapshot
                .game_config
                .num_played_bonus(env.num_played);
        (env.params.callback)(
            idx_left,
            &env.params.word_strip_buffer[idx_left as usize..idx_right as usize],
            score,
            env.params.rack_tally,
        );
    }

    fn play_right<CallbackType: FnMut(i8, &[u8], i16, &[u8])>(
        env: &mut Env<CallbackType>,
        mut idx: i8,
        mut p: i32,
        mut main_score: i16,
        perpendicular_cumulative_score: i16,
        word_multiplier: i16,
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
            main_score += env.params.face_value_scores_strip[idx as usize] as i16;
            idx += 1;
        }
        let node = env.params.board_snapshot.kwg[p];
        if idx > env.params.anchor + 1
            && (env.num_played + is_unique as i8) >= 2
            && idx - env.idx_left >= 2
            && node.accepts()
        {
            record(
                env,
                env.idx_left,
                idx,
                main_score,
                perpendicular_cumulative_score,
                word_multiplier,
            );
        }
        if env.num_played as u8 >= env.params.num_max_played {
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
            let new_word_multiplier =
                word_multiplier * env.params.remaining_word_multipliers_strip[idx as usize] as i16;
            let tile_multiplier = env.params.remaining_tile_multipliers_strip[idx as usize];
            let perpendicular_word_multiplier =
                env.params.perpendicular_word_multipliers_strip[idx as usize];
            let perpendicular_score = env.params.perpendicular_scores_strip[idx as usize];
            loop {
                let node = env.params.board_snapshot.kwg[p];
                let tile = node.tile();
                if this_cross_bits & (1 << tile) != 0 {
                    if env.params.rack_tally[tile as usize] > 0 {
                        env.params.rack_tally[tile as usize] -= 1;
                        env.num_played += 1;
                        let tile_value = env.alphabet.score(tile) as i16 * tile_multiplier as i16;
                        env.params.word_strip_buffer[idx as usize] = tile;
                        play_right(
                            env,
                            idx + 1,
                            p,
                            main_score + tile_value,
                            perpendicular_cumulative_score
                                + perpendicular_score
                                + tile_value * perpendicular_word_multiplier as i16,
                            new_word_multiplier,
                            is_unique,
                        );
                        env.num_played -= 1;
                        env.params.rack_tally[tile as usize] += 1;
                    }
                    if env.params.rack_tally[0] > 0 {
                        env.params.rack_tally[0] -= 1;
                        env.num_played += 1;
                        // intentional to not hardcode blank tile value as zero
                        let tile_value = env.alphabet.score(0) as i16 * tile_multiplier as i16;
                        env.params.word_strip_buffer[idx as usize] = tile | 0x80;
                        play_right(
                            env,
                            idx + 1,
                            p,
                            main_score + tile_value,
                            perpendicular_cumulative_score
                                + perpendicular_score
                                + tile_value * perpendicular_word_multiplier as i16,
                            new_word_multiplier,
                            is_unique,
                        );
                        env.num_played -= 1;
                        env.params.rack_tally[0] += 1;
                    }
                }
                if node.is_end() {
                    break;
                }
                p += 1;
            }
        }
    }

    fn play_left<CallbackType: FnMut(i8, &[u8], i16, &[u8])>(
        env: &mut Env<CallbackType>,
        mut idx: i8,
        mut p: i32,
        mut main_score: i16,
        perpendicular_cumulative_score: i16,
        word_multiplier: i16,
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
            main_score += env.params.face_value_scores_strip[idx as usize] as i16;
            idx -= 1;
        }
        let mut node = env.params.board_snapshot.kwg[p];
        if (env.num_played + is_unique as i8) >= 2 && env.params.anchor - idx >= 2 && node.accepts()
        {
            record(
                env,
                idx + 1,
                env.params.anchor + 1,
                main_score,
                perpendicular_cumulative_score,
                word_multiplier,
            );
        }
        if env.num_played as u8 >= env.params.num_max_played {
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
            play_right(
                env,
                env.params.anchor + 1,
                p,
                main_score,
                perpendicular_cumulative_score,
                word_multiplier,
                is_unique,
            );
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
            let new_word_multiplier =
                word_multiplier * env.params.remaining_word_multipliers_strip[idx as usize] as i16;
            let tile_multiplier = env.params.remaining_tile_multipliers_strip[idx as usize];
            let perpendicular_word_multiplier =
                env.params.perpendicular_word_multipliers_strip[idx as usize];
            let perpendicular_score = env.params.perpendicular_scores_strip[idx as usize];
            loop {
                let node = env.params.board_snapshot.kwg[p];
                let tile = node.tile();
                if this_cross_bits & (1 << tile) != 0 {
                    if env.params.rack_tally[tile as usize] > 0 {
                        env.params.rack_tally[tile as usize] -= 1;
                        env.num_played += 1;
                        let tile_value = env.alphabet.score(tile) as i16 * tile_multiplier as i16;
                        env.params.word_strip_buffer[idx as usize] = tile;
                        play_left(
                            env,
                            idx - 1,
                            p,
                            main_score + tile_value,
                            perpendicular_cumulative_score
                                + perpendicular_score
                                + tile_value * perpendicular_word_multiplier as i16,
                            new_word_multiplier,
                            is_unique,
                        );
                        env.num_played -= 1;
                        env.params.rack_tally[tile as usize] += 1;
                    }
                    if env.params.rack_tally[0] > 0 {
                        env.params.rack_tally[0] -= 1;
                        env.num_played += 1;
                        // intentional to not hardcode blank tile value as zero
                        let tile_value = env.alphabet.score(0) as i16 * tile_multiplier as i16;
                        env.params.word_strip_buffer[idx as usize] = tile | 0x80;
                        play_left(
                            env,
                            idx - 1,
                            p,
                            main_score + tile_value,
                            perpendicular_cumulative_score
                                + perpendicular_score
                                + tile_value * perpendicular_word_multiplier as i16,
                            new_word_multiplier,
                            is_unique,
                        );
                        env.num_played -= 1;
                        env.params.rack_tally[0] += 1;
                    }
                }
                if node.is_end() {
                    break;
                }
                p += 1;
            }
        }
    }

    let alphabet = params.board_snapshot.game_config.alphabet();
    let anchor = params.anchor;
    play_left(
        &mut Env {
            params,
            alphabet,
            num_played: 0,
            idx_left: 0,
        },
        anchor,
        1,
        0,
        0,
        1,
        single_tile_plays,
    );
}

fn gen_place_moves_at<'a, FoundPlaceMove: FnMut(bool, i8, i8, &[u8], i16, &[u8])>(
    board_snapshot: &'a BoardSnapshot<'a>,
    working_buffer: &mut WorkingBuffer,
    placement: &PossiblePlacement,
    max_rack_size: u8,
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
            board_snapshot: &board_snapshot,
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
            num_max_played: max_rack_size,
            anchor: placement.anchor,
            leftmost: placement.leftmost,
            rightmost: placement.rightmost,
            callback: |idx: i8, word: &[u8], score: i16, rack_tally: &[u8]| {
                found_place_move(placement.down, placement.lane, idx, word, score, rack_tally)
            },
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
        score: i16,
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
                    self_tiles.clone_from(&source_tiles);
                } else {
                    *self = source.clone();
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
                    self_down.clone_from(&source_down);
                    self_lane.clone_from(&source_lane);
                    self_idx.clone_from(&source_idx);
                    self_word.clone_from(&source_word);
                    self_score.clone_from(&source_score);
                } else {
                    *self = source.clone();
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
        other.equity.partial_cmp(&self.equity)
    }
}

impl Ord for ValuedMove {
    #[inline(always)]
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        match self.partial_cmp(other) {
            Some(x) => x,
            None => std::cmp::Ordering::Equal,
        }
    }
}

pub struct WriteablePlay<'a> {
    board_snapshot: &'a BoardSnapshot<'a>,
    play: &'a Play,
}

impl std::fmt::Display for WriteablePlay<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match &self.play {
            Play::Exchange { tiles } => {
                if tiles.is_empty() {
                    write!(f, "Pass")?;
                } else {
                    let alphabet = self.board_snapshot.game_config.alphabet();
                    write!(f, "Exch. ")?;
                    for &tile in tiles.iter() {
                        write!(f, "{}", alphabet.from_rack(tile).unwrap())?;
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
                    write!(f, "{}{} ", (*lane as u8 + 0x41) as char, idx + 1)?;
                } else {
                    write!(f, "{}{} ", lane + 1, (*idx as u8 + 0x41) as char)?;
                }
                let strider = if *down {
                    dim.down(*lane)
                } else {
                    dim.across(*lane)
                };
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
                                .from_board(self.board_snapshot.board_tiles[strider.at(i)])
                                .unwrap(),
                        )?;
                    } else {
                        if inside {
                            write!(f, ")")?;
                            inside = false;
                        }
                        write!(f, "{}", alphabet.from_board(tile).unwrap())?;
                    }
                }
                if inside {
                    write!(f, ")")?;
                }
                write!(f, " {}", score)?;
            }
        }
        Ok(())
    }
}

impl Play {
    pub fn fmt<'a>(&'a self, board_snapshot: &'a BoardSnapshot) -> WriteablePlay<'a> {
        WriteablePlay {
            board_snapshot: &board_snapshot,
            play: self,
        }
    }
}

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

    // skip equity computation and sorting
    #[allow(dead_code)]
    pub fn gen_all_raw_moves_unsorted<'a>(
        &mut self,
        board_snapshot: &'a BoardSnapshot<'a>,
        rack: &'a [u8],
    ) {
        self.plays.clear();

        let vec_moves = std::cell::RefCell::new(std::mem::take(&mut self.plays));

        let mut working_buffer = &mut self.working_buffer;
        working_buffer.init(board_snapshot, rack);

        let found_place_move =
            |down: bool, lane: i8, idx: i8, word: &[u8], score: i16, _rack_tally: &[u8]| {
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

        let found_exchange_move = |_rack_tally: &[u8], exchanged_tiles: &[u8]| {
            vec_moves.borrow_mut().push(ValuedMove {
                equity: 0.0,
                play: Play::Exchange {
                    tiles: exchanged_tiles.into(),
                },
            });
        };

        kurnia_gen_place_moves(
            true,
            board_snapshot,
            &mut working_buffer,
            found_place_move,
            |_best_possible_equity: f32| true,
        );
        kurnia_gen_nonplace_moves(board_snapshot, &mut working_buffer, found_exchange_move);

        self.plays = vec_moves.into_inner();
    }

    pub fn gen_moves_filtered<
        'a,
        PlaceMovePredicate: FnMut(bool, i8, i8, &[u8], i16, &[u8]) -> bool,
        AdjustLeaveValue: Fn(f32) -> f32,
    >(
        &mut self,
        board_snapshot: &'a BoardSnapshot<'a>,
        rack: &'a [u8],
        max_gen: usize,
        mut place_move_predicate: PlaceMovePredicate,
        adjust_leave_value: AdjustLeaveValue,
    ) {
        self.plays.clear();
        if max_gen == 0 {
            return;
        }

        let alphabet = board_snapshot.game_config.alphabet();
        let board_layout = board_snapshot.game_config.board_layout();

        let found_moves = std::cell::RefCell::new(std::collections::BinaryHeap::from(
            std::mem::take(&mut self.plays),
        ));

        #[inline(always)]
        fn push_move<F: FnMut() -> Play>(
            found_moves: &std::cell::RefCell<std::collections::BinaryHeap<ValuedMove>>,
            max_gen: usize,
            equity: f32,
            mut construct_play: F,
        ) {
            let mut borrowed = found_moves.borrow_mut();
            if borrowed.len() >= max_gen {
                if borrowed.peek().unwrap().equity >= equity {
                    return;
                }
                borrowed.pop();
            }
            borrowed.push(ValuedMove {
                equity,
                play: construct_play(),
            });
        }

        let mut working_buffer = &mut self.working_buffer;
        working_buffer.init(board_snapshot, rack);
        let num_tiles_on_board = working_buffer.num_tiles_on_board;
        let num_tiles_in_bag = working_buffer.num_tiles_in_bag;
        let play_out_bonus = working_buffer.play_out_bonus;

        let leave_value_from_tally = |rack_tally: &[u8]| {
            if num_tiles_in_bag <= 0 {
                let played_out = rack_tally.iter().all(|&num| num == 0);
                (if played_out {
                    play_out_bonus
                } else {
                    -10 - 2
                        * (0u8..)
                            .zip(rack_tally)
                            .map(|(tile, num)| *num as i16 * alphabet.score(tile) as i16)
                            .sum::<i16>()
                }) as f32
            } else {
                // note: adjust_leave_value(f) must return between 0.0 and f
                adjust_leave_value(board_snapshot.klv.leave_value_from_tally(rack_tally))
            }
        };

        let found_place_move =
            |down: bool, lane: i8, idx: i8, word: &[u8], score: i16, rack_tally: &[u8]| {
                if place_move_predicate(down, lane, idx, word, score, rack_tally) {
                    let leave_value = leave_value_from_tally(rack_tally);
                    let other_adjustments = if num_tiles_on_board == 0 {
                        (idx..)
                            .zip(word)
                            .filter(|(i, &tile)| {
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
                    push_move(&found_moves, max_gen, equity, || Play::Place {
                        down,
                        lane,
                        idx,
                        word: word.into(),
                        score,
                    });
                }
            };

        let found_exchange_move = |rack_tally: &[u8], exchanged_tiles: &[u8]| {
            push_move(
                &found_moves,
                max_gen,
                leave_value_from_tally(rack_tally),
                || Play::Exchange {
                    tiles: exchanged_tiles.into(),
                },
            );
        };

        let can_accept = |best_possible_equity: f32| {
            let borrowed = found_moves.borrow();
            return !(borrowed.len() >= max_gen
                && borrowed.peek().unwrap().equity >= best_possible_equity);
        };

        kurnia_gen_place_moves(
            false,
            board_snapshot,
            &mut working_buffer,
            found_place_move,
            can_accept,
        );
        kurnia_gen_nonplace_moves(board_snapshot, &mut working_buffer, found_exchange_move);

        self.plays = found_moves.into_inner().into_vec();
        self.plays.sort_unstable();
    }

    #[inline(always)]
    pub fn gen_moves_unfiltered<'a>(
        &mut self,
        board_snapshot: &'a BoardSnapshot<'a>,
        rack: &'a [u8],
        max_gen: usize,
    ) {
        self.gen_moves_filtered(
            board_snapshot,
            rack,
            max_gen,
            |_down: bool, _lane: i8, _idx: i8, _word: &[u8], _score: i16, _rack_tally: &[u8]| true,
            |leave_value: f32| leave_value,
        );
    }
}

fn kurnia_gen_nonplace_moves<'a, FoundExchangeMove: FnMut(&[u8], &[u8])>(
    board_snapshot: &'a BoardSnapshot<'a>,
    working_buffer: &mut WorkingBuffer,
    mut found_exchange_move: FoundExchangeMove,
) {
    working_buffer.exchange_buffer.clear(); // should be no-op
    struct ExchangeEnv<'a, FoundExchangeMove: FnMut(&[u8], &[u8])> {
        found_exchange_move: FoundExchangeMove,
        rack_tally: &'a mut [u8],
        exchange_buffer: &'a mut Vec<u8>,
    }
    fn generate_exchanges<FoundExchangeMove: FnMut(&[u8], &[u8])>(
        env: &mut ExchangeEnv<FoundExchangeMove>,
        mut idx: u8,
    ) {
        let rack_tally_len = env.rack_tally.len();
        while (idx as usize) < rack_tally_len && env.rack_tally[idx as usize] == 0 {
            idx += 1;
        }
        if idx as usize >= rack_tally_len {
            (env.found_exchange_move)(&env.rack_tally, &env.exchange_buffer);
            return;
        }
        let original_count = env.rack_tally[idx as usize];
        let vec_len = env.exchange_buffer.len();
        loop {
            generate_exchanges(env, idx + 1);
            if env.rack_tally[idx as usize] == 0 {
                break;
            }
            env.rack_tally[idx as usize] -= 1;
            env.exchange_buffer.push(idx);
        }
        env.rack_tally[idx as usize] = original_count;
        env.exchange_buffer.truncate(vec_len);
    }
    if working_buffer.num_tiles_in_bag >= board_snapshot.game_config.rack_size() as i16 {
        generate_exchanges(
            &mut ExchangeEnv {
                found_exchange_move,
                rack_tally: &mut working_buffer.rack_tally,
                exchange_buffer: &mut working_buffer.exchange_buffer,
            },
            0,
        );
    } else {
        found_exchange_move(&working_buffer.rack_tally, &working_buffer.exchange_buffer);
    }
}

fn kurnia_gen_place_moves<
    'a,
    FoundPlaceMove: FnMut(bool, i8, i8, &[u8], i16, &[u8]),
    CanAccept: Fn(f32) -> bool,
>(
    want_raw: bool,
    board_snapshot: &'a BoardSnapshot<'a>,
    working_buffer: &mut WorkingBuffer,
    mut found_place_move: FoundPlaceMove,
    can_accept: CanAccept,
) {
    let game_config = &board_snapshot.game_config;
    let board_layout = game_config.board_layout();
    let dim = board_layout.dim();
    let max_rack_size = game_config.rack_size() as u8;

    // striped by row
    for col in 0..dim.cols {
        let strip_range_start = (col as isize * dim.rows as isize) as usize;
        let strip_range_end = strip_range_start + dim.rows as usize;
        gen_cross_set(
            &board_snapshot,
            &working_buffer.transposed_board_tiles[strip_range_start..strip_range_end],
            &mut working_buffer.cross_set_for_across_plays,
            dim.down(col),
            &mut working_buffer.cross_set_buffer,
            &mut working_buffer.cached_cross_set_for_across_plays
                [strip_range_start..strip_range_end],
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
            &board_snapshot,
            &board_snapshot.board_tiles[strip_range_start..strip_range_end],
            &mut working_buffer.cross_set_for_down_plays,
            transposed_dim.down(row),
            &mut working_buffer.cross_set_buffer,
            &mut working_buffer.cached_cross_set_for_down_plays[strip_range_start..strip_range_end],
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
            want_raw,
            &mut GenPlacePlacementsParams {
                board_strip: &board_snapshot.board_tiles[strip_range_start..strip_range_end],
                cross_set_strip: &working_buffer.cross_set_for_across_plays
                    [strip_range_start..strip_range_end],
                remaining_word_multipliers_strip: &working_buffer
                    .remaining_word_multipliers_for_across_plays
                    [strip_range_start..strip_range_end],
                face_value_scores_strip: &working_buffer.face_value_scores_for_across_plays
                    [strip_range_start..strip_range_end],
                perpendicular_scores_strip: &working_buffer.perpendicular_scores_for_across_plays
                    [strip_range_start..strip_range_end],
                rack_bits: working_buffer.rack_bits,
                descending_scores: &working_buffer.descending_scores,
                square_multipliers_by_aggregated_word_multipliers_buffer: &mut working_buffer
                    .square_multipliers_by_aggregated_word_multipliers_buffer,
                precomputed_square_multiplier_buffer: &mut working_buffer
                    .precomputed_square_multiplier_buffer,
                indexes_to_descending_square_multiplier_buffer: &mut working_buffer
                    .indexes_to_descending_square_multiplier_buffer,
                best_leave_values: &working_buffer.best_leave_values,
                num_max_played: max_rack_size,
            },
            &working_buffer.remaining_tile_multipliers_for_across_plays
                [strip_range_start..strip_range_end],
            &working_buffer.perpendicular_word_multipliers_for_across_plays
                [strip_range_start..strip_range_end],
            working_buffer.num_tiles_on_rack,
            true,
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
            want_raw,
            &mut GenPlacePlacementsParams {
                board_strip: &working_buffer.transposed_board_tiles
                    [strip_range_start..strip_range_end],
                cross_set_strip: &working_buffer.cross_set_for_down_plays
                    [strip_range_start..strip_range_end],
                remaining_word_multipliers_strip: &working_buffer
                    .remaining_word_multipliers_for_down_plays[strip_range_start..strip_range_end],
                face_value_scores_strip: &working_buffer.face_value_scores_for_down_plays
                    [strip_range_start..strip_range_end],
                perpendicular_scores_strip: &working_buffer.perpendicular_scores_for_down_plays
                    [strip_range_start..strip_range_end],
                rack_bits: working_buffer.rack_bits,
                descending_scores: &working_buffer.descending_scores,
                square_multipliers_by_aggregated_word_multipliers_buffer: &mut working_buffer
                    .square_multipliers_by_aggregated_word_multipliers_buffer,
                precomputed_square_multiplier_buffer: &mut working_buffer
                    .precomputed_square_multiplier_buffer,
                indexes_to_descending_square_multiplier_buffer: &mut working_buffer
                    .indexes_to_descending_square_multiplier_buffer,
                best_leave_values: &working_buffer.best_leave_values,
                num_max_played: max_rack_size,
            },
            &working_buffer.remaining_tile_multipliers_for_down_plays
                [strip_range_start..strip_range_end],
            &working_buffer.perpendicular_word_multipliers_for_down_plays
                [strip_range_start..strip_range_end],
            working_buffer.num_tiles_on_rack,
            false,
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
        found_placements.sort_unstable_by(|a, b| {
            b.best_possible_equity
                .partial_cmp(&a.best_possible_equity)
                .unwrap()
        });
    }
    for placement in &found_placements {
        if !can_accept(placement.best_possible_equity) {
            break;
        }
        gen_place_moves_at(
            &board_snapshot,
            working_buffer,
            &placement,
            max_rack_size,
            &mut found_place_move,
        );
    }
    working_buffer.found_placements = found_placements;
}
