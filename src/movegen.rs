// Copyright (C) 2020-2026 Andy Kurnia.

use super::{alphabet, alphagram, anagrams, bites, display, equity, game_config, klv, kwg, matrix};

pub const MAX_ALPHABET_LEN: usize = 64;

#[inline(always)]
pub fn bag_count_from_board(game_config: &game_config::GameConfig, num_tiles_on_board: u16) -> i16 {
    game_config.alphabet().num_tiles() as i16
        - (num_tiles_on_board as i16
            + game_config.num_players() as i16 * game_config.rack_size() as i16)
}

#[inline(always)]
pub fn live_pool_into(
    out: &mut [u8],
    alphabet: &alphabet::Alphabet,
    board_tiles: &[u8],
    rack_tally: &[u8],
) {
    for (t, slot) in out.iter_mut().enumerate() {
        *slot = alphabet.freq(t as u8);
    }
    for &tile in board_tiles.iter() {
        if tile != 0 {
            let base = (tile & !((tile as i8) >> 7) as u8) as usize;
            if base < out.len() {
                out[base] = out[base].saturating_sub(1);
            }
        }
    }
    for (slot, &cnt) in out.iter_mut().zip(rack_tally.iter()) {
        *slot = slot.saturating_sub(cnt);
    }
}

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

type PlaceMoveFn<'a> = &'a mut dyn FnMut(bool, i8, i8, &[u8], i32) -> bool;

type EquityFn<'a> = &'a mut dyn FnMut(equity::Equity, &Play) -> bool;

pub enum EquityPredicate<'a> {
    AcceptAll,
    RejectAll,
    Dyn(EquityFn<'a>),
}

impl EquityPredicate<'_> {
    #[inline(always)]
    fn test(&mut self, equity: equity::Equity, play: &Play) -> bool {
        match self {
            EquityPredicate::AcceptAll => true,
            EquityPredicate::RejectAll => false,
            EquityPredicate::Dyn(f) => f(equity, play),
        }
    }
}

pub enum PlacePredicate<'a> {
    AcceptAll,
    RejectAll,
    Dyn(PlaceMoveFn<'a>),
}

impl PlacePredicate<'_> {
    #[inline(always)]
    fn test(&mut self, down: bool, lane: i8, idx: i8, word: &[u8], score: i32) -> bool {
        match self {
            PlacePredicate::AcceptAll => true,
            PlacePredicate::RejectAll => false,
            PlacePredicate::Dyn(f) => f(down, lane, idx, word, score),
        }
    }
}

#[derive(Clone, Copy)]
struct PossiblePlacement {
    num_played: u8,
    down: bool,
    lane: i8,
    anchor: i8,
    leftmost: i8,
    rightmost: i8,
    best_possible_equity: i32,
}

#[derive(Clone)]
struct MultiJump {
    left_score: i32,
    right_score: i32,
    left_idx: i8,
    right_idx: i8,
}

#[derive(Clone)]
struct LaneScaffold {
    valid: bool,
    board_strip: Vec<u8>,
    remaining_word_multipliers_strip: Vec<i8>,
    remaining_tile_multipliers_strip: Vec<i8>,
    perpendicular_word_multipliers_strip: Vec<i8>,
    aggregated_word_multipliers: Vec<i32>,
    precomputed_square_multiplier: Vec<i32>,
    indexes_to_descending_square_multiplier: Vec<i8>,
    square_ranks: Vec<u8>,
    multi_jumps: Vec<MultiJump>,
}

impl LaneScaffold {
    #[inline(always)]
    fn new() -> Self {
        Self {
            valid: false,
            board_strip: Vec::new(),
            remaining_word_multipliers_strip: Vec::new(),
            remaining_tile_multipliers_strip: Vec::new(),
            perpendicular_word_multipliers_strip: Vec::new(),
            aggregated_word_multipliers: Vec::new(),
            precomputed_square_multiplier: Vec::new(),
            indexes_to_descending_square_multiplier: Vec::new(),
            square_ranks: Vec::new(),
            multi_jumps: Vec::new(),
        }
    }

    #[inline]
    fn refresh(
        &mut self,
        alphabet: &alphabet::Alphabet,
        board_strip: &[u8],
        remaining_word_multipliers_strip: &[i8],
        remaining_tile_multipliers_strip: &[i8],
        perpendicular_word_multipliers_strip: &[i8],
    ) {
        if self.valid
            && self.board_strip == board_strip
            && self.remaining_word_multipliers_strip == remaining_word_multipliers_strip
            && self.remaining_tile_multipliers_strip == remaining_tile_multipliers_strip
            && self.perpendicular_word_multipliers_strip == perpendicular_word_multipliers_strip
        {
            return;
        }
        let strider_len = board_strip.len();
        self.aggregated_word_multipliers.clear();
        let mut last_was_one = false;
        for i in 0..strider_len {
            let mut wm = remaining_word_multipliers_strip[i] as i32;
            if last_was_one {
                last_was_one = wm == 1;
                continue;
            }
            last_was_one = wm == 1;
            if let Err(idx) = self.aggregated_word_multipliers.binary_search(&wm) {
                self.aggregated_word_multipliers.insert(idx, wm);
            }
            for &wm_val in &remaining_word_multipliers_strip[i + 1..strider_len] {
                if wm_val != 1 {
                    if wm > i32::MAX / equity::SCALE {
                        break;
                    }
                    wm *= wm_val as i32;
                    if let Err(idx) = self.aggregated_word_multipliers.binary_search(&wm) {
                        self.aggregated_word_multipliers.insert(idx, wm);
                    }
                }
            }
        }
        let vec_size = strider_len * self.aggregated_word_multipliers.len();
        self.precomputed_square_multiplier.resize(vec_size, 0);
        self.indexes_to_descending_square_multiplier
            .resize(vec_size, 0);
        self.square_ranks.resize(vec_size, 0);
        for (k, low_end) in self
            .aggregated_word_multipliers
            .iter()
            .zip((0..).step_by(strider_len))
        {
            let high_end = low_end + strider_len;
            let precomputed_square_multiplier_slice =
                &mut self.precomputed_square_multiplier[low_end..high_end];
            let indexes_to_descending_square_multiplier_slice =
                &mut self.indexes_to_descending_square_multiplier[low_end..high_end];
            let mut left = 0;
            for j in (0..strider_len).filter(|&j| board_strip[j] == 0) {
                precomputed_square_multiplier_slice[j] = remaining_tile_multipliers_strip[j] as i32
                    * (k + perpendicular_word_multipliers_strip[j] as i32);
                indexes_to_descending_square_multiplier_slice[left] = j as i8;
                left += 1;
            }
            indexes_to_descending_square_multiplier_slice[..left].sort_unstable_by(|&a, &b| {
                precomputed_square_multiplier_slice[b as usize]
                    .cmp(&precomputed_square_multiplier_slice[a as usize])
            });
            let square_ranks_slice = &mut self.square_ranks[low_end..high_end];
            for (rank, &j) in indexes_to_descending_square_multiplier_slice[..left]
                .iter()
                .enumerate()
            {
                square_ranks_slice[j as usize] = rank as u8;
            }
        }

        self.multi_jumps.resize(
            strider_len,
            MultiJump {
                left_score: 0,
                right_score: 0,
                left_idx: 0,
                right_idx: 0,
            },
        );
        let mut score = 0i32;
        let mut last_empty = strider_len as i8;
        for j in (0..strider_len).rev() {
            let b = board_strip[j];
            if b != 0 {
                score += alphabet.scaled_score(b);
            } else {
                score = 0; // cumulative face-value score (millipoints)
                last_empty = j as i8; // last seen empty square
            }
            self.multi_jumps[j].right_score = score;
            self.multi_jumps[j].right_idx = last_empty;
        }
        score = 0i32;
        last_empty = -1i8;
        for (j, &b) in board_strip.iter().enumerate() {
            if b != 0 {
                score += alphabet.scaled_score(b);
            } else {
                score = 0; // cumulative face-value score (millipoints)
                last_empty = j as i8; // last seen empty square
            }
            self.multi_jumps[j].left_score = score;
            self.multi_jumps[j].left_idx = last_empty;
        }

        self.valid = true;
        self.board_strip.clear();
        self.board_strip.extend_from_slice(board_strip);
        self.remaining_word_multipliers_strip.clear();
        self.remaining_word_multipliers_strip
            .extend_from_slice(remaining_word_multipliers_strip);
        self.remaining_tile_multipliers_strip.clear();
        self.remaining_tile_multipliers_strip
            .extend_from_slice(remaining_tile_multipliers_strip);
        self.perpendicular_word_multipliers_strip.clear();
        self.perpendicular_word_multipliers_strip
            .extend_from_slice(perpendicular_word_multipliers_strip);
    }
}

const SHADOW_SCORES_INLINE: usize = 8;

#[derive(Clone)]
struct ShadowScores {
    inline: [i32; SHADOW_SCORES_INLINE],
    len: u8,
    spill: Vec<i32>,
}

impl ShadowScores {
    #[inline(always)]
    fn new() -> Self {
        Self {
            inline: [0i32; SHADOW_SCORES_INLINE],
            len: 0,
            spill: Vec::new(),
        }
    }

    #[inline(always)]
    fn clear(&mut self) {
        self.len = 0;
        self.spill.clear();
    }

    #[inline(always)]
    fn is_empty(&self) -> bool {
        self.len == 0 && self.spill.is_empty()
    }

    #[inline(always)]
    fn as_slice(&self) -> &[i32] {
        if self.spill.is_empty() {
            &self.inline[..self.len as usize]
        } else {
            &self.spill
        }
    }

    #[inline(always)]
    fn insert_sorted(&mut self, score: i32) {
        if !self.spill.is_empty() {
            let at = self.spill.partition_point(|&x| x <= score);
            self.spill.insert(at, score);
            return;
        }
        let len = self.len as usize;
        if len < SHADOW_SCORES_INLINE {
            let at = self.inline[..len].partition_point(|&x| x <= score);
            self.inline.copy_within(at..len, at + 1);
            self.inline[at] = score;
            self.len = self.len.wrapping_add(1);
            return;
        }
        self.spill.extend_from_slice(&self.inline[..len]);
        let at = self.spill.partition_point(|&x| x <= score);
        self.spill.insert(at, score);
        self.len = 0;
    }

    #[inline(always)]
    fn copy_from(&mut self, other: &ShadowScores) {
        if other.spill.is_empty() {
            self.inline = other.inline;
            self.len = other.len;
            self.spill.clear();
        } else {
            self.spill.clone_from(&other.spill);
            self.len = 0;
        }
    }
}

// WorkingBuffer can only be reused for the same game_config and kwg.
// (The kwg is partially cached in cached_cross_set.)
// WorkingBuffer can also be reset for reuse with another kwg by calling
// reset_for_another_kwg().
// This is not enforced.
struct WorkingBuffer {
    rack_tally: [u8; MAX_ALPHABET_LEN],
    word_buffer_for_across_plays: Box<[u8]>,     // r*c
    word_buffer_for_down_plays: Box<[u8]>,       // c*r
    cross_set_for_across_plays: Box<[CrossSet]>, // r*c
    cross_set_for_down_plays: Box<[CrossSet]>,   // c*r
    cached_cross_set_for_across_plays: Box<[CachedCrossSet]>, // c*r
    cached_cross_set_for_down_plays: Box<[CachedCrossSet]>, // r*c
    cross_set_buffer_for_across_plays: Box<[CrossSetComputation]>, // c*r (perpendicular strips)
    cross_set_buffer_for_down_plays: Box<[CrossSetComputation]>, // r*c (perpendicular strips)
    prev_board_tiles: Box<[u8]>,                 // r*c (previous board tiles for dirty tracking)
    remaining_word_multipliers_for_across_plays: Box<[i8]>, // r*c (1 if tile placed)
    remaining_word_multipliers_for_down_plays: Box<[i8]>, // c*r
    remaining_tile_multipliers_for_across_plays: Box<[i8]>, // r*c (1 if tile placed)
    remaining_tile_multipliers_for_down_plays: Box<[i8]>, // c*r
    face_value_scores_for_across_plays: Box<[i32]>, // r*c (premultiplied by SCALE)
    face_value_scores_for_down_plays: Box<[i32]>, // c*r (premultiplied by SCALE)
    perpendicular_word_multipliers_for_across_plays: Box<[i8]>, // r*c (0 if no perpendicularly adjacent tile)
    perpendicular_word_multipliers_for_down_plays: Box<[i8]>,   // c*r
    perpendicular_scores_for_across_plays: Box<[i32]>, // r*c (multiplied by perpendicular_word_multipliers)
    perpendicular_scores_for_down_plays: Box<[i32]>,   // c*r
    transposed_board_tiles: Box<[u8]>,                 // c*r
    num_tiles_on_board: u16,
    num_tiles_in_bag: i16, // negative when players also have less than full racks
    play_out_bonus: i32,
    num_tiles_on_rack: u8,
    rack_bits: u64, // bit 0 = blank conveniently matches bit 0 = have cross set
    multi_leaves: klv::MultiLeaves,
    descending_scores: Vec<i32>, // rack.len() (premultiplied by SCALE)
    exchange_buffer: Vec<u8>,    // rack.len(), or max(word length) with word prune
    lane_scaffold: Box<[LaneScaffold]>, // rows + cols, across lanes first
    best_leave_values: Vec<i32>, // rack.len() + 1
    span_out: Vec<(i8, i8, u8, i32)>,
    subracks: Vec<Subrack>,
    subracks_by_played: Vec<u32>,
    found_placements: Vec<PossiblePlacement>,
    placement_order: Vec<(i32, u32)>,
    used_letters_tally: Vec<u8>, // 27 for ?A-Z, ? is always 0, jumbled mode only
    used_tile_scores_shadowl: ShadowScores, // for shadow_play_left, premultiplied by SCALE
    used_tile_scores_shadowr: ShadowScores, // for shadow_play_right, premultiplied by SCALE
    rack_tally_shadowl: [u8; MAX_ALPHABET_LEN], // for shadow_play_left
    rack_tally_shadowr: [u8; MAX_ALPHABET_LEN], // for shadow_play_right
    word_source_fits_config: bool,
    is_census: bool,
}

impl Clone for WorkingBuffer {
    #[inline(always)]
    fn clone(&self) -> Self {
        Self {
            rack_tally: self.rack_tally,
            word_buffer_for_across_plays: self.word_buffer_for_across_plays.clone(),
            word_buffer_for_down_plays: self.word_buffer_for_down_plays.clone(),
            cross_set_for_across_plays: self.cross_set_for_across_plays.clone(),
            cross_set_for_down_plays: self.cross_set_for_down_plays.clone(),
            cached_cross_set_for_across_plays: self.cached_cross_set_for_across_plays.clone(),
            cached_cross_set_for_down_plays: self.cached_cross_set_for_down_plays.clone(),
            cross_set_buffer_for_across_plays: self.cross_set_buffer_for_across_plays.clone(),
            cross_set_buffer_for_down_plays: self.cross_set_buffer_for_down_plays.clone(),
            prev_board_tiles: self.prev_board_tiles.clone(),
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
            multi_leaves: self.multi_leaves.clone(),
            descending_scores: self.descending_scores.clone(),
            exchange_buffer: self.exchange_buffer.clone(),
            lane_scaffold: self.lane_scaffold.clone(),
            best_leave_values: self.best_leave_values.clone(),
            span_out: self.span_out.clone(),
            subracks: self.subracks.clone(),
            subracks_by_played: self.subracks_by_played.clone(),
            found_placements: self.found_placements.clone(),
            placement_order: self.placement_order.clone(),
            used_letters_tally: self.used_letters_tally.clone(),
            used_tile_scores_shadowl: self.used_tile_scores_shadowl.clone(),
            used_tile_scores_shadowr: self.used_tile_scores_shadowr.clone(),
            rack_tally_shadowl: self.rack_tally_shadowl,
            rack_tally_shadowr: self.rack_tally_shadowr,
            is_census: self.is_census,
            word_source_fits_config: self.word_source_fits_config,
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
        self.cross_set_buffer_for_across_plays
            .clone_from(&source.cross_set_buffer_for_across_plays);
        self.cross_set_buffer_for_down_plays
            .clone_from(&source.cross_set_buffer_for_down_plays);
        self.prev_board_tiles.clone_from(&source.prev_board_tiles);
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
        self.multi_leaves.clone_from(&source.multi_leaves);
        self.descending_scores.clone_from(&source.descending_scores);
        self.exchange_buffer.clone_from(&source.exchange_buffer);
        self.lane_scaffold.clone_from(&source.lane_scaffold);
        self.best_leave_values.clone_from(&source.best_leave_values);
        self.span_out.clone_from(&source.span_out);
        self.subracks.clone_from(&source.subracks);
        self.subracks_by_played
            .clone_from(&source.subracks_by_played);
        self.found_placements.clone_from(&source.found_placements);
        self.placement_order.clone_from(&source.placement_order);
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
        self.is_census = source.is_census;
        self.word_source_fits_config = source.word_source_fits_config;
    }
}

impl WorkingBuffer {
    fn new(game_config: &game_config::GameConfig) -> Self {
        let dim = game_config.board_layout().dim();
        let rows_times_cols = (dim.rows as isize * dim.cols as isize) as usize;
        Self {
            rack_tally: [0u8; MAX_ALPHABET_LEN],
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
            cross_set_buffer_for_across_plays: vec![
                CrossSetComputation {
                    score: 0,
                    b_letter: 0,
                    end_range: 0,
                    p: 0,
                };
                rows_times_cols
            ]
            .into_boxed_slice(),
            cross_set_buffer_for_down_plays: vec![
                CrossSetComputation {
                    score: 0,
                    b_letter: 0,
                    end_range: 0,
                    p: 0,
                };
                rows_times_cols
            ]
            .into_boxed_slice(),
            prev_board_tiles: vec![0xffu8; rows_times_cols].into_boxed_slice(),
            remaining_word_multipliers_for_across_plays: vec![0i8; rows_times_cols]
                .into_boxed_slice(),
            remaining_word_multipliers_for_down_plays: vec![0i8; rows_times_cols]
                .into_boxed_slice(),
            remaining_tile_multipliers_for_across_plays: vec![0i8; rows_times_cols]
                .into_boxed_slice(),
            remaining_tile_multipliers_for_down_plays: vec![0i8; rows_times_cols]
                .into_boxed_slice(),
            face_value_scores_for_across_plays: vec![0i32; rows_times_cols].into_boxed_slice(),
            face_value_scores_for_down_plays: vec![0i32; rows_times_cols].into_boxed_slice(),
            perpendicular_word_multipliers_for_across_plays: vec![0i8; rows_times_cols]
                .into_boxed_slice(),
            perpendicular_word_multipliers_for_down_plays: vec![0i8; rows_times_cols]
                .into_boxed_slice(),
            perpendicular_scores_for_across_plays: vec![0i32; rows_times_cols].into_boxed_slice(),
            perpendicular_scores_for_down_plays: vec![0i32; rows_times_cols].into_boxed_slice(),
            transposed_board_tiles: vec![0u8; rows_times_cols].into_boxed_slice(),
            num_tiles_on_board: 0,
            num_tiles_in_bag: 0,
            play_out_bonus: 0,
            num_tiles_on_rack: 0,
            rack_bits: 0,
            multi_leaves: klv::MultiLeaves::new(),
            descending_scores: Vec::new(),
            exchange_buffer: Vec::new(),
            lane_scaffold: vec![LaneScaffold::new(); dim.rows as usize + dim.cols as usize]
                .into_boxed_slice(),
            best_leave_values: Vec::new(),
            span_out: Vec::new(),
            subracks: Vec::new(),
            subracks_by_played: Vec::new(),
            found_placements: Vec::new(),
            placement_order: Vec::new(),
            used_letters_tally: Vec::new(),
            used_tile_scores_shadowl: ShadowScores::new(),
            used_tile_scores_shadowr: ShadowScores::new(),
            rack_tally_shadowl: [0u8; MAX_ALPHABET_LEN],
            rack_tally_shadowr: [0u8; MAX_ALPHABET_LEN],
            is_census: false,
            word_source_fits_config: false,
        }
    }

    #[inline]
    fn turn_is_supported<N: kwg::Node, L: kwg::Node>(
        &self,
        want_raw: bool,
        board_snapshot: &BoardSnapshot<'_, N, L>,
    ) -> bool {
        if !matches!(
            board_snapshot.game_config.game_rules(),
            game_config::GameRules::Classic
        ) {
            return false;
        }
        let dim = board_snapshot.game_config.board_layout().dim();
        let extent = dim.rows.max(dim.cols) as u8;
        let layout = board_snapshot.anagrams.map(anagrams::Anagrams::layout);
        !want_raw
            && !self.is_census
            && self.word_source_fits_config
            && self.rack_tally[0] <= 2
            && !self.subracks.is_empty()
            && layout
                .is_some_and(|layout| layout.covers(board_snapshot.game_config.alphabet(), extent))
    }

    fn init<N: kwg::Node, L: kwg::Node>(
        &mut self,
        board_snapshot: &BoardSnapshot<'_, N, L>,
        rack: &[u8],
        adjust_leave_value: klv::AdjustLeave,
        dynamic_leaves: Option<klv::DynamicLeavesRef<'_>>,
    ) {
        let alphabet = board_snapshot.game_config.alphabet();
        self.word_source_fits_config = match board_snapshot.anagrams.map(anagrams::Anagrams::layout)
        {
            Some(layout) => {
                let dim = board_snapshot.game_config.board_layout().dim();
                layout.matches(alphabet, dim.rows.max(dim.cols) as u8)
            }
            None => false,
        };
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
        let board_layout = board_snapshot.game_config.board_layout();
        let dim = board_layout.dim();
        let area = (dim.rows as isize * dim.cols as isize) as usize;
        let was_empty = self.num_tiles_on_board == 0;

        if self.prev_board_tiles[..area] != board_snapshot.board_tiles[..area] {
            let premiums = board_layout.premiums();
            let transposed_premiums = board_layout.transposed_premiums();

            for (idx, &b) in board_snapshot.board_tiles.iter().enumerate().take(area) {
                if b == 0 {
                    let premium = &premiums[idx];
                    self.remaining_word_multipliers_for_across_plays[idx] = premium.word_multiplier;
                    self.remaining_tile_multipliers_for_across_plays[idx] = premium.tile_multiplier;
                    self.face_value_scores_for_across_plays[idx] = 0;
                } else {
                    self.remaining_word_multipliers_for_across_plays[idx] = 1; // needed for the HashMap

                    self.face_value_scores_for_across_plays[idx] = alphabet.scaled_score(b);
                }
            }
            for col in 0..dim.cols {
                for row in 0..dim.rows {
                    self.transposed_board_tiles
                        [(col as isize * dim.rows as isize + row as isize) as usize] =
                        board_snapshot.board_tiles
                            [(row as isize * dim.cols as isize + col as isize) as usize];
                }
            }

            for (idx, &b) in self.transposed_board_tiles.iter().enumerate().take(area) {
                if b == 0 {
                    let premium = &transposed_premiums[idx];
                    self.remaining_word_multipliers_for_down_plays[idx] = premium.word_multiplier;
                    self.remaining_tile_multipliers_for_down_plays[idx] = premium.tile_multiplier;
                    self.face_value_scores_for_down_plays[idx] = 0;
                } else {
                    self.remaining_word_multipliers_for_down_plays[idx] = 1; // needed for the HashMap

                    self.face_value_scores_for_down_plays[idx] = alphabet.scaled_score(b);
                }
            }
            self.num_tiles_on_board = board_snapshot
                .board_tiles
                .iter()
                .filter(|&t| *t != 0)
                .count() as u16;
        }
        if was_empty || self.num_tiles_on_board == 0 {
            self.prev_board_tiles
                [dim.at_row_col(board_layout.star_row(), board_layout.star_col())] = 0xff;
        }
        self.num_tiles_in_bag =
            bag_count_from_board(board_snapshot.game_config, self.num_tiles_on_board);
        let play_out_bonus = if self.num_tiles_in_bag <= 0 {
            2 * ((0u8..)
                .zip(self.rack_tally.iter().take(alphabet.len() as usize))
                .map(|(tile, &num)| {
                    (alphabet.freq(tile) as i32 - num as i32) * alphabet.score(tile) as i32
                })
                .sum::<i32>()
                - board_snapshot
                    .board_tiles
                    .iter()
                    .map(|&t| if t != 0 { alphabet.score(t) as i32 } else { 0 })
                    .sum::<i32>())
                * equity::SCALE
        } else {
            0
        };
        self.play_out_bonus = play_out_bonus;

        self.descending_scores.clear();
        self.descending_scores
            .reserve(self.num_tiles_on_rack as usize);
        for &tile in alphabet.tiles_by_descending_scores() {
            let count = self.rack_tally[tile as usize];
            if count != 0 {
                let score = alphabet.scaled_score(tile);
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
            if self.multi_leaves.is_dense() {
                self.multi_leaves
                    .init_endgame_leaves(|tile| alphabet.score(tile), play_out_bonus);
            }
            // the multi_leaves is correct but doing this directly is faster.
            self.best_leave_values.clear();
            self.best_leave_values
                .resize(self.num_tiles_on_rack as usize + 1, i32::MIN);
            let mut unplayed = 0i32;
            for i in (0..self.num_tiles_on_rack).rev() {
                unplayed += self.descending_scores[i as usize];
                self.best_leave_values[i as usize] = -equity::ENDGAME_PENALTY_BASE - 2 * unplayed;
            }
            self.best_leave_values[self.num_tiles_on_rack as usize] = play_out_bonus;
        } else {
            self.multi_leaves.init(
                &self.rack_tally,
                board_snapshot.klv,
                true,
                adjust_leave_value,
            );
            if self.multi_leaves.is_dense() {
                if let Some(dyn_ref) = dynamic_leaves {
                    let n = dyn_ref.lat.num_letters();
                    let mut live_pool = [0u8; MAX_ALPHABET_LEN];
                    live_pool_into(
                        &mut live_pool[..n],
                        alphabet,
                        board_snapshot.board_tiles,
                        &self.rack_tally,
                    );
                    self.multi_leaves.apply_dynamic_leaves(
                        &dyn_ref,
                        &live_pool[..n],
                        self.num_tiles_in_bag.max(0) as usize,
                    );
                }
                self.multi_leaves
                    .extract_raw_best_leave_values(&mut self.best_leave_values);
            } else {
                klv::MultiLeaves::extract_best_leave_values_from_klv(
                    &mut self.rack_tally,
                    board_snapshot.klv,
                    self.num_tiles_on_rack,
                    adjust_leave_value,
                    &mut self.best_leave_values,
                );
            }
        }
        for i in 0..=self.num_tiles_on_rack {
            self.best_leave_values[i as usize] +=
                board_snapshot.game_config.num_played_bonus(i) as i32 * equity::SCALE;
        }
        if let (true, Some(layout)) = (
            board_snapshot.anagrams.is_some() && self.multi_leaves.is_dense(),
            board_snapshot.anagrams.map(anagrams::Anagrams::layout),
        ) {
            build_subracks(
                &self.multi_leaves,
                layout,
                &self.rack_tally,
                self.num_tiles_on_rack,
                &mut self.subracks,
                &mut self.subracks_by_played,
            );
        } else {
            self.subracks.clear();
            self.subracks_by_played.clear();
        }
        self.used_letters_tally.clear();
        match board_snapshot.game_config.game_rules() {
            game_config::GameRules::Classic => {}
            game_config::GameRules::Jumbled => {
                self.used_letters_tally.resize(alphabet.len() as usize, 0);
            }
        }
        self.used_tile_scores_shadowl.clear();
        self.used_tile_scores_shadowr.clear();
    }

    #[inline]
    fn init_after_cross_sets<N: kwg::Node, L: kwg::Node>(
        &mut self,
        board_snapshot: &BoardSnapshot<'_, N, L>,
        dirty_cols: u128,
        dirty_rows: u128,
    ) {
        let board_layout = board_snapshot.game_config.board_layout();
        let dim = board_layout.dim();
        if dirty_cols != 0 {
            let premiums = board_layout.premiums();

            for col in 0..dim.cols {
                if dirty_cols & (1 << col) == 0 {
                    continue;
                }
                let strider = dim.down(col);
                for i in 0..strider.len() {
                    let idx = strider.at(i);
                    let premium = &premiums[idx];
                    let cross_set = &mut self.cross_set_for_across_plays[idx];
                    if premium.word_multiplier == 0 && premium.tile_multiplier == 0 {
                        cross_set.bits = 1;
                    }
                    let effective_pwm = self.remaining_word_multipliers_for_across_plays[idx]
                        & -(cross_set.bits as i8 & 1);
                    self.perpendicular_word_multipliers_for_across_plays[idx] = effective_pwm;
                    self.perpendicular_scores_for_across_plays[idx] =
                        cross_set.score * effective_pwm as i32;
                }
            }
        }
        if dirty_rows != 0 {
            let transposed_dim = matrix::Dim {
                rows: dim.cols,
                cols: dim.rows,
            };
            let transposed_premiums = board_layout.transposed_premiums();

            for row in 0..dim.rows {
                if dirty_rows & (1 << row) == 0 {
                    continue;
                }
                let strider = transposed_dim.down(row);
                for i in 0..strider.len() {
                    let idx = strider.at(i);
                    let premium = &transposed_premiums[idx];
                    let cross_set = &mut self.cross_set_for_down_plays[idx];
                    if premium.word_multiplier == 0 && premium.tile_multiplier == 0 {
                        cross_set.bits = 1;
                    }
                    let effective_pwm = self.remaining_word_multipliers_for_down_plays[idx]
                        & -(cross_set.bits as i8 & 1);
                    self.perpendicular_word_multipliers_for_down_plays[idx] = effective_pwm;
                    self.perpendicular_scores_for_down_plays[idx] =
                        cross_set.score * effective_pwm as i32;
                }
            }
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
        self.prev_board_tiles.fill(0xff);
        for lane in self.lane_scaffold.iter_mut() {
            lane.valid = false;
        }
    }
}

// kwg must be Gaddawg for Classic, AlphaDawg for Jumbled.
pub struct BoardSnapshot<'a, N: kwg::Node, L: kwg::Node> {
    pub board_tiles: &'a [u8],
    pub game_config: &'a game_config::GameConfig,
    pub kwg: &'a kwg::Kwg<N>,
    pub anagrams: Option<&'a anagrams::Anagrams>,
    pub rack_lengths: Option<&'a anagrams::RackLengths>,
    pub klv: &'a klv::Klv<L>,
}

// cached_cross_sets is just one strip, so it is transposed from cross_sets
#[inline]
fn gen_classic_cross_set<'a, N: kwg::Node, L: kwg::Node>(
    board_snapshot: &'a BoardSnapshot<'a, N, L>,
    board_strip: &'a [u8],
    cross_sets: &'a mut [CrossSet],
    output_strider: matrix::Strider,
    cross_set_buffer: &'a mut [CrossSetComputation],
    cached_cross_sets: &'a mut [CachedCrossSet],
) {
    let sibling_bits = |kwg: &kwg::Kwg<N>, mut p: i32, accepting_only: bool| -> u64 {
        let mut bits = 0u64;
        if p > 0 {
            loop {
                let node = kwg[p];
                if !accepting_only || node.accepts() {
                    bits |= 1u64 << node.tile();
                }
                if node.is_end() {
                    break;
                }
                p += 1;
            }
        }
        bits
    };

    let len = output_strider.len();
    let step = output_strider.step() as usize;
    let kwg = board_snapshot.kwg;
    let mut last_nonempty = len;
    {
        let alphabet = board_snapshot.game_config.alphabet();
        let mut p = 1;
        let mut score = 0i32;
        let mut last_empty = len;

        let mut chain_valid = true; // right edge is always a valid group start
        for j in (0..len).rev() {
            let b = board_strip[j as usize];
            if b != 0 {
                let b_letter = b & 0x7f;
                if chain_valid && cross_set_buffer[j as usize].b_letter == b {
                    p = cross_set_buffer[j as usize].p;
                    score = cross_set_buffer[j as usize].score;
                } else {
                    chain_valid = false;
                    p = kwg.seek(p, b_letter);
                    score += alphabet.scaled_score(b);
                    cross_set_buffer[j as usize] = CrossSetComputation {
                        score,
                        b_letter: b,
                        end_range: last_empty,
                        p,
                    };
                }

                cross_set_buffer[j as usize].end_range = last_empty;
                last_nonempty = j;
            } else {
                // empty square, reset
                p = 1; // cumulative gaddag traversal results
                score = 0; // cumulative face-value score
                last_empty = j; // last seen empty square

                chain_valid = cross_set_buffer[j as usize].b_letter == 0;
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
            let p = cross_set_buffer[j as usize].p;
            let mut bits = reuse_cross_set(cached_cross_sets, j - 1, -2, p);
            if bits == 0 {
                bits = 1u64;
                if p > 0 {
                    let arc = kwg[p].arc_index();
                    bits |= sibling_bits(kwg, arc, true);
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
            let p_right = cross_set_buffer[j as usize].p;
            let p_left = kwg.seek(cross_set_buffer[prev_j as usize].p, 0);
            let mut bits = reuse_cross_set(cached_cross_sets, j - 1, p_left, p_right);
            if bits == 0 {
                bits = 1u64;
                if p_right > 0 && p_left > 0 {
                    let arc_right = kwg[p_right].arc_index();
                    let arc_left = kwg[p_left].arc_index();
                    if arc_right > 0 && arc_left > 0 {
                        let mut candidates = sibling_bits(kwg, arc_right, false)
                            & sibling_bits(kwg, arc_left, false)
                            & !1; // exclude separator
                        if j_end - j > j - 1 - prev_j {
                            while candidates != 0 {
                                let tile = candidates.trailing_zeros() as u8;
                                candidates &= candidates - 1;
                                let mut q = kwg.seek(p_right, tile);
                                if q > 0 {
                                    for qi in (prev_j..j - 1).rev() {
                                        // mask off the blank bit: the seek needs the bare letter.
                                        q = kwg
                                            .seek(q, cross_set_buffer[qi as usize].b_letter & 0x7f);
                                        if q <= 0 {
                                            break;
                                        }
                                    }
                                    if q > 0 {
                                        bits |= (kwg[q].accepts() as u64) << tile;
                                    }
                                }
                            }
                        } else {
                            while candidates != 0 {
                                let tile = candidates.trailing_zeros() as u8;
                                candidates &= candidates - 1;
                                let mut q = kwg.seek(p_left, tile);
                                if q > 0 {
                                    for qi in j..j_end {
                                        // mask off the blank bit: the seek needs the bare letter.
                                        q = kwg
                                            .seek(q, cross_set_buffer[qi as usize].b_letter & 0x7f);
                                        if q <= 0 {
                                            break;
                                        }
                                    }
                                    if q > 0 {
                                        bits |= (kwg[q].accepts() as u64) << tile;
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
        let p = kwg.seek(cross_set_buffer[prev_j as usize].p, 0);
        let mut bits = reuse_cross_set(cached_cross_sets, j, p, -2);
        if bits == 0 {
            bits = 1u64;
            if p > 0 {
                let arc = kwg[p].arc_index();
                bits |= sibling_bits(kwg, arc, true);
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

#[inline]
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
    let kwg = board_snapshot.kwg;
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
                score += alphabet.scaled_score(b);
                used_letters_tally[(b & 0x7f) as usize] += 1;
            }
            let mut k = i + 1;
            while k < len {
                let b = board_strip[k as usize];
                if b == 0 {
                    break;
                }
                k += 1;
                score += alphabet.scaled_score(b);
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
    used_tile_scores_shadowl: &'a mut ShadowScores,
    used_tile_scores_shadowr: &'a mut ShadowScores,
    cross_set_strip: &'a [CrossSet],
    remaining_word_multipliers_strip: &'a [i8],
    remaining_tile_multipliers_strip: &'a [i8],
    perpendicular_word_multipliers_strip: &'a [i8],
    perpendicular_scores_strip: &'a [i32],
    rack_bits: u64,
    feasible_lengths: u64,
    descending_scores: &'a [i32],
    lane_scaffold: &'a mut LaneScaffold,
    best_leave_values: &'a [i32],
    span_out: &'a mut Vec<(i8, i8, u8, i32)>,
    per_span: bool,
    num_max_played: u8,
    rack_tally_shadowl: &'a mut [u8],
    rack_tally_shadowr: &'a mut [u8],
}

fn feasible_word_lengths<const USE_TABLE: bool>(
    anagrams: Option<&anagrams::Anagrams>,
    rack_lengths: Option<&anagrams::RackLengths>,
    rack_tally: &[u8],
) -> u64 {
    if rack_tally[0] > 0 {
        return !0;
    }
    match (anagrams.filter(|_| USE_TABLE), rack_lengths) {
        (Some(a), _) => rack_subset_lengths(a.layout(), rack_tally, |key, len| {
            a.words(alphagram::Fitted(key), len).is_some()
        }),
        (None, Some(r)) => rack_subset_lengths(r.layout(), rack_tally, |key, _| r.contains(key)),
        (None, None) => !0,
    }
}

#[inline]
fn rack_subset_lengths(
    layout: &alphagram::KeyLayout,
    rack_tally: &[u8],
    spells: impl Fn(u128, u8) -> bool,
) -> u64 {
    let mut tiles = [(0u8, 0u128); MAX_ALPHABET_LEN];
    let mut n = 0;
    for (tile, &count) in rack_tally.iter().enumerate().skip(1) {
        if count != 0 {
            tiles[n] = (count, layout.place_value(tile as u8));
            n += 1;
        }
    }
    fn rec(
        of: &[(u8, u128)],
        len: u8,
        key: u128,
        found: &mut u64,
        spells: &impl Fn(u128, u8) -> bool,
    ) {
        let Some((&(count, place_value), rest)) = of.split_first() else {
            if len >= 2 && *found & 1 << len == 0 && spells(key, len) {
                *found |= 1 << len;
            }
            return;
        };
        let mut k_key = key;
        for k in 0..=count {
            rec(rest, len + k, k_key, found, spells);
            k_key += place_value;
        }
    }
    let mut found = 0u64;
    rec(&tiles[..n], 0, 0, &mut found, &spells);
    found | 3
}

#[inline]
fn gen_place_placements<'a, PossibleStripPlacementCallbackType: FnMut(i8, i8, i8, i32, u8)>(
    params: &'a mut GenPlacePlacementsParams<'a>,
    single_tile_plays: bool,
    want_raw: bool,
    possible_strip_placement_callback: PossibleStripPlacementCallbackType,
) {
    if params.per_span {
        gen_place_placements_impl::<true, _>(
            params,
            single_tile_plays,
            want_raw,
            possible_strip_placement_callback,
        )
    } else {
        gen_place_placements_impl::<false, _>(
            params,
            single_tile_plays,
            want_raw,
            possible_strip_placement_callback,
        )
    }
}

#[inline]
fn gen_place_placements_impl<
    'a,
    const PER_SPAN: bool,
    PossibleStripPlacementCallbackType: FnMut(i8, i8, i8, i32, u8),
>(
    params: &'a mut GenPlacePlacementsParams<'a>,
    single_tile_plays: bool,
    want_raw: bool,
    mut possible_strip_placement_callback: PossibleStripPlacementCallbackType,
) {
    let strider_len = params.board_strip.len();

    if !want_raw {
        params.lane_scaffold.refresh(
            params.alphabet,
            params.board_strip,
            params.remaining_word_multipliers_strip,
            params.remaining_tile_multipliers_strip,
            params.perpendicular_word_multipliers_strip,
        );
    }

    struct Env<'a> {
        params: &'a mut GenPlacePlacementsParams<'a>,
        strider_len: usize,
        anchor: i8,
        leftmost: i8,
        rightmost: i8,
        best_possible_equity: i32,
    }

    let mut env = Env {
        params,
        strider_len,
        anchor: 0,
        leftmost: 0,
        rightmost: 0,
        best_possible_equity: i32::MIN,
    };

    // during shadow-playing, main_score and perpendicular_cumulative_score
    // assume all tiles placed from rack this turn are worth zero,
    // except forced-placement cases.
    // their scores are added separately.
    struct Accumulator {
        main_score: i32,                     // main_played_through_score
        perpendicular_cumulative_score: i32, // perpendicular_additional_score
        word_multiplier: i32,
        crossed_board_tiles: bool,
        deferred_score_cap: i32,
    }

    #[inline(always)]
    fn ranked_from(env: &Env<'_>, deferred: u128, low_end: usize) -> u128 {
        let square_ranks_slice =
            &env.params.lane_scaffold.square_ranks[low_end..low_end + env.strider_len];
        let mut remaining = deferred;
        let mut ranked = 0u128;
        while remaining != 0 {
            let square = remaining.trailing_zeros() as usize;
            remaining &= remaining - 1;
            ranked |= 1u128 << square_ranks_slice[square];
        }
        ranked
    }

    #[inline(always)]
    fn shadow_record<const PER_SPAN: bool>(
        env: &mut Env<'_>,
        acc: &Accumulator,
        idx_left: i8,
        idx_right: i8,
        num_played: u8,
        low_end: usize,
        ranked: u128,
    ) {
        debug_assert_eq!(
            env.params.board_strip[idx_left as usize..idx_right as usize]
                .iter()
                .all(|&t| t == 0),
            !acc.crossed_board_tiles
        );
        if !acc.crossed_board_tiles && env.params.feasible_lengths & 1 << num_played == 0 {
            return;
        }
        let used_tile_scores = if env.params.used_tile_scores_shadowr.is_empty() {
            env.params.used_tile_scores_shadowl.as_slice()
        } else {
            env.params.used_tile_scores_shadowr.as_slice()
        };
        let mut best_scoring = 0;
        let mut to_assign = num_played - used_tile_scores.len() as u8;
        if to_assign != 0 {
            // if a square requiring [B] is encountered while holding a B, the B
            // must go there. if a square requiring [A,B] is encountered earlier,
            // that square must be A, but this is not currently implemented.
            let high_end = low_end + env.strider_len;
            let precomputed_square_multiplier_slice =
                &env.params.lane_scaffold.precomputed_square_multiplier[low_end..high_end];
            let indexes_to_descending_square_multiplier_slice = &env
                .params
                .lane_scaffold
                .indexes_to_descending_square_multiplier[low_end..high_end];
            debug_assert!(
                ranked == 0 || acc.deferred_score_cap != i32::MIN,
                "a deferred square with no deferred_score_cap recorded"
            );
            let cap = acc.deferred_score_cap;
            let mut remaining = ranked;
            if used_tile_scores.is_empty() {
                let mut di = 0;
                while to_assign != 0 && remaining != 0 {
                    let idx = indexes_to_descending_square_multiplier_slice
                        [remaining.trailing_zeros() as usize];
                    remaining &= remaining - 1;
                    best_scoring += env.params.descending_scores[di].min(cap)
                        * precomputed_square_multiplier_slice[idx as usize];
                    di += 1;
                    to_assign -= 1;
                }
            } else {
                let mut used_tile_scores_iter = used_tile_scores.iter().rev().peekable(); // iterate from highest score
                let mut desc_scores_iter = env
                    .params
                    .descending_scores
                    .iter()
                    .filter(|&score| used_tile_scores_iter.next_if_eq(&score).is_none());
                while to_assign != 0 && remaining != 0 {
                    let idx = indexes_to_descending_square_multiplier_slice
                        [remaining.trailing_zeros() as usize];
                    remaining &= remaining - 1;
                    best_scoring += (*desc_scores_iter.next().unwrap()).min(cap)
                        * precomputed_square_multiplier_slice[idx as usize];
                    to_assign -= 1;
                }
            }
        }
        let equity = acc.main_score * acc.word_multiplier
            + acc.perpendicular_cumulative_score
            + best_scoring
            + env.params.best_leave_values[num_played as usize];
        if PER_SPAN {
            env.params
                .span_out
                .push((idx_left, idx_right, num_played, equity));
        }
        if equity > env.best_possible_equity {
            env.best_possible_equity = equity;
        }
    }

    #[derive(Clone, Copy)]
    struct ShadowRightWalk {
        idx: i8,
        is_unique: bool,
        idx_left: i8,
        num_played: u8,
        rack_bits: u64,
        deferred: u128,
        low_end: usize,
        ranked: u128,
        stale_rack: bool,
    }

    #[inline(always)]
    fn shadow_play_right<const PER_SPAN: bool>(
        env: &mut Env<'_>,
        mut acc: Accumulator,
        walk: ShadowRightWalk,
    ) -> bool {
        let ShadowRightWalk {
            mut idx,
            mut is_unique,
            idx_left,
            mut num_played,
            mut rack_bits,
            mut deferred,
            mut low_end,
            mut ranked,
            stale_rack,
        } = walk;
        if !env.params.used_tile_scores_shadowl.is_empty() {
            env.params
                .used_tile_scores_shadowr
                .copy_from(env.params.used_tile_scores_shadowl);
        }
        if stale_rack {
            env.params
                .rack_tally_shadowr
                .clone_from_slice(env.params.rack_tally_shadowl);
        }
        let mut took_a_tile = false;
        loop {
            if idx < env.rightmost {
                // tail-recurse placing current sequence of tiles in one go
                let multi_jump = &env.params.lane_scaffold.multi_jumps[idx as usize];
                acc.main_score += multi_jump.right_score;
                acc.crossed_board_tiles |= multi_jump.right_idx != idx;
                idx = multi_jump.right_idx;
            }
            // tiles have been placed from idx_left to idx - 1.
            // here idx <= env.rightmost.
            // check if [idx_left, idx) is a thing
            if idx > env.anchor + 1 && num_played > !is_unique as u8 && idx - idx_left >= 2 {
                shadow_record::<PER_SPAN>(env, &acc, idx_left, idx, num_played, low_end, ranked);
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
                deferred |= 1u128 << (idx as u32);
                ranked |= 1u128 << env.params.lane_scaffold.square_ranks[low_end + idx as usize];
                acc.deferred_score_cap = i32::MAX;
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
                    took_a_tile = true;
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
                    let tile_score = env.params.alphabet.scaled_score(tile);
                    env.params
                        .used_tile_scores_shadowr
                        .insert_sorted(tile_score);
                    let tile_value = tile_score
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
                    let mut remaining = matching_bits;
                    let mut admissible_max = i32::MIN;
                    while remaining != 0 {
                        let t = remaining.trailing_zeros() as u8;
                        remaining &= remaining - 1;
                        admissible_max = admissible_max.max(env.params.alphabet.scaled_score(t));
                    }
                    acc.deferred_score_cap = acc.deferred_score_cap.max(admissible_max);
                    deferred |= 1u128 << (idx as u32);
                    ranked |=
                        1u128 << env.params.lane_scaffold.square_ranks[low_end + idx as usize];
                    acc.perpendicular_cumulative_score +=
                        env.params.perpendicular_scores_strip[idx as usize];
                }
            } else {
                break;
            }
            num_played += 1;
            let word_multiplier = env.params.remaining_word_multipliers_strip[idx as usize] as i32;
            if word_multiplier != 1 {
                acc.word_multiplier *= word_multiplier;
                low_end = env
                    .params
                    .lane_scaffold
                    .aggregated_word_multipliers
                    .binary_search(&acc.word_multiplier)
                    .unwrap()
                    * env.strider_len;
                ranked = ranked_from(env, deferred, low_end);
            }
            idx += 1;
        }
        env.params.used_tile_scores_shadowr.clear(); // use shadowl in shadow_record
        took_a_tile
    }

    #[inline(always)]
    fn shadow_play_left<const PER_SPAN: bool>(
        env: &mut Env<'_>,
        mut acc: Accumulator,
        mut idx: i8,
        mut is_unique: bool,
    ) {
        let mut deferred = 0u128;
        let mut ranked = 0u128;
        let mut low_end = env
            .params
            .lane_scaffold
            .aggregated_word_multipliers
            .binary_search(&acc.word_multiplier)
            .unwrap()
            * env.strider_len;
        let mut num_played = 0;
        let mut stale_rack = true;
        env.params.used_tile_scores_shadowl.clear();
        let mut rack_bits = env.params.rack_bits;
        env.params
            .rack_tally_shadowl
            .clone_from_slice(env.params.rack_tally);
        loop {
            if idx >= env.leftmost {
                // tail-recurse placing current sequence of tiles in one go
                let multi_jump = &env.params.lane_scaffold.multi_jumps[idx as usize];
                acc.main_score += multi_jump.left_score;
                acc.crossed_board_tiles |= multi_jump.left_idx != idx;
                idx = multi_jump.left_idx;
            }
            // tiles have been placed from env.anchor to idx + 1.
            // here idx >= env.leftmost - 1.
            // check if [idx + 1, env.anchor + 1) is a thing
            if num_played > !is_unique as u8 && env.anchor - idx >= 2 {
                shadow_record::<PER_SPAN>(
                    env,
                    &acc,
                    idx + 1,
                    env.anchor + 1,
                    num_played,
                    low_end,
                    ranked,
                );
            }
            if num_played >= env.params.num_max_played {
                break;
            }

            // can switch direction only after using the anchor square
            if idx < env.anchor {
                stale_rack = shadow_play_right::<PER_SPAN>(
                    env,
                    Accumulator { ..acc },
                    ShadowRightWalk {
                        idx: env.anchor + 1,
                        is_unique,
                        idx_left: idx + 1,
                        num_played,
                        rack_bits,
                        deferred,
                        low_end,
                        ranked,
                        stale_rack,
                    },
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
                deferred |= 1u128 << (idx as u32);
                ranked |= 1u128 << env.params.lane_scaffold.square_ranks[low_end + idx as usize];
                acc.deferred_score_cap = i32::MAX;
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
                    stale_rack = true;
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
                    let tile_score = env.params.alphabet.scaled_score(tile);
                    env.params
                        .used_tile_scores_shadowl
                        .insert_sorted(tile_score);
                    let tile_value = tile_score
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
                    let mut remaining = matching_bits;
                    let mut admissible_max = i32::MIN;
                    while remaining != 0 {
                        let t = remaining.trailing_zeros() as u8;
                        remaining &= remaining - 1;
                        admissible_max = admissible_max.max(env.params.alphabet.scaled_score(t));
                    }
                    acc.deferred_score_cap = acc.deferred_score_cap.max(admissible_max);
                    deferred |= 1u128 << (idx as u32);
                    ranked |=
                        1u128 << env.params.lane_scaffold.square_ranks[low_end + idx as usize];
                    acc.perpendicular_cumulative_score +=
                        env.params.perpendicular_scores_strip[idx as usize];
                }
            } else {
                break;
            }
            num_played += 1;
            let word_multiplier = env.params.remaining_word_multipliers_strip[idx as usize] as i32;
            if word_multiplier != 1 {
                acc.word_multiplier *= word_multiplier;
                low_end = env
                    .params
                    .lane_scaffold
                    .aggregated_word_multipliers
                    .binary_search(&acc.word_multiplier)
                    .unwrap()
                    * env.strider_len;
                ranked = ranked_from(env, deferred, low_end);
            }
            idx -= 1;
        }
    }

    #[inline(always)]
    fn gen_places_from<
        const PER_SPAN: bool,
        PossibleStripPlacementCallbackType: FnMut(i8, i8, i8, i32, u8),
    >(
        env: &mut Env<'_>,
        single_tile_plays: bool,
        want_raw: bool,
        mut possible_strip_placement_callback: PossibleStripPlacementCallbackType,
    ) {
        if want_raw {
            possible_strip_placement_callback(env.anchor, env.leftmost, env.rightmost, i32::MAX, 0);
        } else {
            env.best_possible_equity = i32::MIN;
            env.params.span_out.clear();
            shadow_play_left::<PER_SPAN>(
                env,
                Accumulator {
                    main_score: 0,
                    perpendicular_cumulative_score: 0,
                    word_multiplier: 1,
                    crossed_board_tiles: false,
                    deferred_score_cap: i32::MIN,
                },
                env.anchor,
                single_tile_plays,
            );
            if env.best_possible_equity != i32::MIN {
                if PER_SPAN {
                    for i in 0..env.params.span_out.len() {
                        let (left, right, num_played, equity) = env.params.span_out[i];
                        possible_strip_placement_callback(
                            env.anchor, left, right, equity, num_played,
                        );
                    }
                } else {
                    possible_strip_placement_callback(
                        env.anchor,
                        env.leftmost,
                        env.rightmost,
                        env.best_possible_equity,
                        0,
                    );
                }
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
            gen_places_from::<PER_SPAN, _>(
                &mut env,
                single_tile_plays,
                want_raw,
                &mut possible_strip_placement_callback,
            );
        }
        {
            // this part is only relevant if rack has at least two tiles, but passing that is too expensive.
            let leftmost = leftmost + (leftmost > 0 && leftmost < rightmost) as i8; // shadowing
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
                        gen_places_from::<PER_SPAN, _>(
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

struct GenPlaceMovesParams<
    'a,
    CallbackType: FnMut(i8, &[u8], i32, i32) -> i32,
    N: kwg::Node,
    L: kwg::Node,
> {
    board_snapshot: &'a BoardSnapshot<'a, N, L>,
    board_strip: &'a [u8],
    cross_set_strip: &'a [CrossSet],
    cross_set_buffer_strip: &'a [CrossSetComputation], // cached GADDAG state per position
    remaining_word_multipliers_strip: &'a [i8],
    remaining_tile_multipliers_strip: &'a [i8],
    face_value_scores_strip: &'a [i32],
    perpendicular_word_multipliers_strip: &'a [i8],
    perpendicular_scores_strip: &'a [i32],
    rack_tally: &'a mut [u8],
    rack_bits: u64,
    word_strip_buffer: &'a mut [u8],
    num_max_played: u8,
    anchor: i8,
    leftmost: i8,
    rightmost: i8,
    span_num_played: u8,
    callback: CallbackType,
    multi_leaves: &'a klv::MultiLeaves,
    num_tiles_in_bag: i16,
    play_out_bonus: i32,
    used_letters_tally: &'a mut [u8], // jumbled mode only
    is_census: bool, // real-before-blank descent for the census's spell-once sheet build
    score_bound: i32,
    threshold: i32,
    subracks: &'a [Subrack],
    subracks_by_played: &'a [u32],
}

#[inline(always)]
fn leave_value_of<L: kwg::Node>(
    multi_leaves: &klv::MultiLeaves,
    klv: &klv::Klv<L>,
    rack_tally: &[u8],
    leave_idx: u32,
    num_tiles_in_bag: i16,
    play_out_bonus: i32,
    alphabet: &alphabet::Alphabet,
) -> i32 {
    if multi_leaves.is_dense() {
        multi_leaves.leave_value(leave_idx)
    } else if num_tiles_in_bag <= 0 {
        let is_played_out = rack_tally.iter().all(|&count| count == 0);
        if is_played_out {
            play_out_bonus
        } else {
            let residual: i32 = (0u8..)
                .zip(rack_tally.iter().take(alphabet.len() as usize))
                .map(|(tile, &count)| count as i32 * alphabet.score(tile) as i32)
                .sum();
            -equity::ENDGAME_PENALTY_BASE - 2 * residual * equity::SCALE
        }
    } else {
        klv.leave_value_from_tally(rack_tally)
    }
}

#[inline]
fn gen_classic_place_moves_lean<
    'a,
    CallbackType: FnMut(i8, &[u8], i32, i32) -> i32,
    N: kwg::Node,
    L: kwg::Node,
>(
    params: &'a mut GenPlaceMovesParams<'a, CallbackType, N, L>,
    single_tile_plays: bool,
) {
    struct Env<'a, CallbackType: FnMut(i8, &[u8], i32, i32) -> i32, N: kwg::Node, L: kwg::Node> {
        params: &'a mut GenPlaceMovesParams<'a, CallbackType, N, L>,
        alphabet: &'a alphabet::Alphabet,
        num_played: u8,
        idx_left: i8,
        rack_bits: u64,
        letter_bits: u64,
    }
    struct Accumulator {
        main_score: i32,
        perpendicular_cumulative_score: i32,
        word_multiplier: i32,
        leave_idx: u32,
    }

    #[inline(always)]
    fn record<
        const SPELL_ONCE: bool,
        CallbackType: FnMut(i8, &[u8], i32, i32) -> i32,
        N: kwg::Node,
        L: kwg::Node,
    >(
        env: &mut Env<'_, CallbackType, N, L>,
        acc: &Accumulator,
        idx_left: i8,
        idx_right: i8,
    ) {
        let score = if SPELL_ONCE {
            0
        } else {
            acc.main_score * acc.word_multiplier
                + acc.perpendicular_cumulative_score
                + env
                    .params
                    .board_snapshot
                    .game_config
                    .num_played_bonus(env.num_played) as i32
                    * equity::SCALE
        };
        let leave_value = if SPELL_ONCE {
            0
        } else {
            leave_value_of(
                env.params.multi_leaves,
                env.params.board_snapshot.klv,
                env.params.rack_tally,
                acc.leave_idx,
                env.params.num_tiles_in_bag,
                env.params.play_out_bonus,
                env.alphabet,
            )
        };
        (env.params.callback)(
            idx_left,
            &env.params.word_strip_buffer[idx_left as usize..idx_right as usize],
            score,
            leave_value,
        );
    }

    fn play_right<
        const SPELL_ONCE: bool,
        CallbackType: FnMut(i8, &[u8], i32, i32) -> i32,
        N: kwg::Node,
        L: kwg::Node,
    >(
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
            if !SPELL_ONCE {
                acc.main_score += env.params.face_value_scores_strip[idx as usize];
            }
            idx += 1;
        }
        let node = env.params.board_snapshot.kwg[p];
        if idx > env.params.anchor + 1
            && env.num_played > !is_unique as u8
            && idx - env.idx_left >= 2
            && node.accepts()
        {
            record::<SPELL_ONCE, _, _, _>(env, acc, env.idx_left, idx);
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
            let mut candidates = this_cross_bits
                & if env.params.rack_tally[0] > 0 {
                    env.letter_bits
                } else {
                    env.rack_bits
                };
            if candidates == 0 {
                return;
            }
            let new_word_multiplier = acc.word_multiplier
                * env.params.remaining_word_multipliers_strip[idx as usize] as i32;
            let tile_multiplier = env.params.remaining_tile_multipliers_strip[idx as usize];
            let perpendicular_word_multiplier =
                env.params.perpendicular_word_multipliers_strip[idx as usize];
            let perpendicular_score = env.params.perpendicular_scores_strip[idx as usize];
            env.num_played += 1;
            let opt_blank_acc = (env.params.rack_tally[0] > 0).then(|| {
                if SPELL_ONCE {
                    Accumulator {
                        main_score: 0,
                        perpendicular_cumulative_score: 0,
                        word_multiplier: 0,
                        leave_idx: 0,
                    }
                } else {
                    let tile_value = env.alphabet.scaled_score(0) * tile_multiplier as i32;
                    Accumulator {
                        main_score: acc.main_score + tile_value,
                        perpendicular_cumulative_score: acc.perpendicular_cumulative_score
                            + perpendicular_score
                            + tile_value * perpendicular_word_multiplier as i32,
                        word_multiplier: new_word_multiplier,
                        leave_idx: acc
                            .leave_idx
                            .wrapping_sub(env.params.multi_leaves.place_value(0)),
                    }
                }
            });
            loop {
                let node = env.params.board_snapshot.kwg[p];
                let tile = node.tile();
                let bit = 1u64 << tile;
                if candidates & bit != 0 {
                    candidates ^= bit;
                    env.params.board_snapshot.kwg.prefetch(node.arc_index());
                    if env.params.rack_tally[tile as usize] > 0 {
                        env.params.rack_tally[tile as usize] -= 1;
                        let spent = bit * (env.params.rack_tally[tile as usize] == 0) as u64;
                        env.rack_bits ^= spent;
                        env.params.word_strip_buffer[idx as usize] = tile;
                        play_right::<SPELL_ONCE, _, _, _>(
                            env,
                            &mut if SPELL_ONCE {
                                Accumulator {
                                    main_score: 0,
                                    perpendicular_cumulative_score: 0,
                                    word_multiplier: 0,
                                    leave_idx: 0,
                                }
                            } else {
                                let tile_value = env.alphabet.score(tile) as i32
                                    * equity::SCALE
                                    * tile_multiplier as i32;
                                Accumulator {
                                    main_score: acc.main_score + tile_value,
                                    perpendicular_cumulative_score: acc
                                        .perpendicular_cumulative_score
                                        + perpendicular_score
                                        + tile_value * perpendicular_word_multiplier as i32,
                                    word_multiplier: new_word_multiplier,
                                    leave_idx: acc
                                        .leave_idx
                                        .wrapping_sub(env.params.multi_leaves.place_value(tile)),
                                }
                            },
                            p,
                            idx + 1,
                            is_unique,
                        );
                        env.rack_bits ^= spent;
                        env.params.rack_tally[tile as usize] += 1;
                    }
                    if let Some(blank_acc) = &opt_blank_acc
                        && (!SPELL_ONCE || env.params.rack_tally[tile as usize] == 0)
                    {
                        env.params.rack_tally[0] -= 1;
                        env.params.word_strip_buffer[idx as usize] = tile | 0x80;
                        play_right::<SPELL_ONCE, _, _, _>(
                            env,
                            &mut Accumulator { ..*blank_acc },
                            p,
                            idx + 1,
                            is_unique,
                        );
                        env.params.rack_tally[0] += 1;
                    }
                    if candidates == 0 {
                        break;
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

    fn play_left<
        const SPELL_ONCE: bool,
        CallbackType: FnMut(i8, &[u8], i32, i32) -> i32,
        N: kwg::Node,
        L: kwg::Node,
    >(
        env: &mut Env<'_, CallbackType, N, L>,
        acc: &mut Accumulator,
        mut p: i32,
        mut idx: i8,
        mut is_unique: bool,
    ) {
        // tail-recurse placing current sequence of tiles
        if p == 1 && idx >= env.params.leftmost && env.params.board_strip[idx as usize] != 0 {
            let mut jump_idx = idx;
            while jump_idx > env.params.leftmost
                && env.params.board_strip[jump_idx as usize - 1] != 0
            {
                jump_idx -= 1;
            }
            p = env.params.cross_set_buffer_strip[jump_idx as usize].p;
            if p <= 0 {
                return;
            }
            if !SPELL_ONCE {
                acc.main_score += env.params.cross_set_buffer_strip[jump_idx as usize].score;
            }
            idx = jump_idx - 1;
        } else {
            while idx >= env.params.leftmost {
                let b = env.params.board_strip[idx as usize];
                if b == 0 {
                    break;
                }
                p = env.params.board_snapshot.kwg.seek(p, b & 0x7f);
                if p <= 0 {
                    return;
                }
                if !SPELL_ONCE {
                    acc.main_score += env.params.face_value_scores_strip[idx as usize];
                }
                idx -= 1;
            }
        }
        let mut node = env.params.board_snapshot.kwg[p];
        if env.num_played > !is_unique as u8 && env.params.anchor - idx >= 2 && node.accepts() {
            record::<SPELL_ONCE, _, _, _>(env, acc, idx + 1, env.params.anchor + 1);
        }
        if env.num_played >= env.params.num_max_played {
            return;
        }

        p = node.arc_index();
        if p <= 0 {
            return;
        }

        let turnaround_is_unique = is_unique;

        let mut this_cross_bits = 0;
        if idx >= env.params.leftmost {
            let bits = env.params.cross_set_strip[idx as usize].bits;
            if bits == 0 {
                this_cross_bits = !1;
                is_unique = true;
            } else if bits != 1 {
                // turn off bit 0 so it cannot match later
                this_cross_bits = bits & !1;
            }
        }

        if this_cross_bits == 0 {
            let mut turnaround_p = p;
            loop {
                node = env.params.board_snapshot.kwg[turnaround_p];
                if node.tile() == 0 {
                    // assume idx < env.params.anchor, because tile 0 does not occur at start in well-formed kwg gaddawg
                    env.idx_left = idx + 1;
                    play_right::<SPELL_ONCE, _, _, _>(
                        env,
                        acc,
                        turnaround_p,
                        env.params.anchor + 1,
                        turnaround_is_unique,
                    );
                    break;
                }
                if node.is_end() {
                    break;
                }
                turnaround_p += 1;
            }
            return;
        }

        let mut candidates = 1
            | (this_cross_bits
                & if env.params.rack_tally[0] > 0 {
                    env.letter_bits
                } else {
                    env.rack_bits
                });
        let new_word_multiplier =
            acc.word_multiplier * env.params.remaining_word_multipliers_strip[idx as usize] as i32;
        let tile_multiplier = env.params.remaining_tile_multipliers_strip[idx as usize];
        let perpendicular_word_multiplier =
            env.params.perpendicular_word_multipliers_strip[idx as usize];
        let perpendicular_score = env.params.perpendicular_scores_strip[idx as usize];
        env.num_played += 1;
        let opt_blank_acc = (env.params.rack_tally[0] > 0).then(|| {
            if SPELL_ONCE {
                Accumulator {
                    main_score: 0,
                    perpendicular_cumulative_score: 0,
                    word_multiplier: 0,
                    leave_idx: 0,
                }
            } else {
                let tile_value = env.alphabet.scaled_score(0) * tile_multiplier as i32;
                Accumulator {
                    main_score: acc.main_score + tile_value,
                    perpendicular_cumulative_score: acc.perpendicular_cumulative_score
                        + perpendicular_score
                        + tile_value * perpendicular_word_multiplier as i32,
                    word_multiplier: new_word_multiplier,
                    leave_idx: acc
                        .leave_idx
                        .wrapping_sub(env.params.multi_leaves.place_value(0)),
                }
            }
        });
        loop {
            let node = env.params.board_snapshot.kwg[p];
            let tile = node.tile();
            let bit = 1u64 << tile;
            if candidates & bit != 0 {
                candidates ^= bit;
                if tile == 0 {
                    env.num_played -= 1;
                    env.idx_left = idx + 1;
                    play_right::<SPELL_ONCE, _, _, _>(
                        env,
                        acc,
                        p,
                        env.params.anchor + 1,
                        turnaround_is_unique,
                    );
                    env.num_played += 1;
                } else {
                    env.params.board_snapshot.kwg.prefetch(node.arc_index());
                    if env.params.rack_tally[tile as usize] > 0 {
                        env.params.rack_tally[tile as usize] -= 1;
                        let spent = bit * (env.params.rack_tally[tile as usize] == 0) as u64;
                        env.rack_bits ^= spent;
                        env.params.word_strip_buffer[idx as usize] = tile;
                        play_left::<SPELL_ONCE, _, _, _>(
                            env,
                            &mut if SPELL_ONCE {
                                Accumulator {
                                    main_score: 0,
                                    perpendicular_cumulative_score: 0,
                                    word_multiplier: 0,
                                    leave_idx: 0,
                                }
                            } else {
                                let tile_value = env.alphabet.score(tile) as i32
                                    * equity::SCALE
                                    * tile_multiplier as i32;
                                Accumulator {
                                    main_score: acc.main_score + tile_value,
                                    perpendicular_cumulative_score: acc
                                        .perpendicular_cumulative_score
                                        + perpendicular_score
                                        + tile_value * perpendicular_word_multiplier as i32,
                                    word_multiplier: new_word_multiplier,
                                    leave_idx: acc
                                        .leave_idx
                                        .wrapping_sub(env.params.multi_leaves.place_value(tile)),
                                }
                            },
                            p,
                            idx - 1,
                            is_unique,
                        );
                        env.rack_bits ^= spent;
                        env.params.rack_tally[tile as usize] += 1;
                    }
                    if let Some(blank_acc) = &opt_blank_acc
                        && (!SPELL_ONCE || env.params.rack_tally[tile as usize] == 0)
                    {
                        env.params.rack_tally[0] -= 1;
                        env.params.word_strip_buffer[idx as usize] = tile | 0x80;
                        play_left::<SPELL_ONCE, _, _, _>(
                            env,
                            &mut Accumulator { ..*blank_acc },
                            p,
                            idx - 1,
                            is_unique,
                        );
                        env.params.rack_tally[0] += 1;
                    }
                }
                if candidates == 0 {
                    break;
                }
            }
            if node.is_end() {
                break;
            }
            p += 1;
        }
        env.num_played -= 1;
    }

    let alphabet = params.board_snapshot.game_config.alphabet();
    let anchor = params.anchor;
    let pass_leave_idx = params.multi_leaves.pass_leave_idx();
    let is_census = params.is_census;
    let rack_bits = params.rack_bits;
    let mut env = Env {
        params,
        alphabet,
        num_played: 0,
        idx_left: 0,
        rack_bits,
        letter_bits: (u64::MAX >> (64 - alphabet.len() as u32)) & !1,
    };
    let mut acc = Accumulator {
        main_score: 0,
        perpendicular_cumulative_score: 0,
        word_multiplier: 1,
        leave_idx: pass_leave_idx,
    };
    if is_census {
        play_left::<true, _, _, _>(&mut env, &mut acc, 1, anchor, single_tile_plays);
    } else {
        play_left::<false, _, _, _>(&mut env, &mut acc, 1, anchor, single_tile_plays);
    }
}

#[derive(Clone, Copy)]
struct Subrack {
    key: u128,
    leave_idx: u32,
    leave_value: i32,
    num_played: u8,
    blanks: u8,
}

#[inline]
fn build_subracks(
    multi_leaves: &klv::MultiLeaves,
    layout: &alphagram::KeyLayout,
    rack_tally: &[u8],
    num_tiles_on_rack: u8,
    subracks: &mut Vec<Subrack>,
    by_played: &mut Vec<u32>,
) {
    struct Tiles<'a> {
        layout: &'a alphagram::KeyLayout,
        of: &'a [(u8, u8, u32, u128)],
    }
    fn rec(
        tiles: &Tiles<'_>,
        i: usize,
        idx: u32,
        key: u128,
        played: u8,
        blanks: u8,
        out: &mut Vec<Subrack>,
    ) {
        if i == tiles.of.len() {
            debug_assert!(tiles.layout.holds(key));
            out.push(Subrack {
                key,
                leave_idx: idx,
                leave_value: 0,
                num_played: played,
                blanks,
            });
            return;
        }
        let (tile, count, place_value, key_place_value) = tiles.of[i];
        for kept in 0..=count {
            let taken = count - kept;
            rec(
                tiles,
                i + 1,
                idx + kept as u32 * place_value,
                key + taken as u128 * key_place_value,
                played + taken,
                blanks + if tile == 0 { taken } else { 0 },
                out,
            );
        }
    }
    subracks.clear();
    let mut tiles = [(0u8, 0u8, 0u32, 0u128); MAX_ALPHABET_LEN];
    let mut n = 0;
    for (tile, &count) in rack_tally.iter().enumerate() {
        if count != 0 {
            tiles[n] = (
                tile as u8,
                count,
                multi_leaves.place_value(tile as u8),
                if tile == 0 {
                    0
                } else {
                    layout.place_value(tile as u8)
                },
            );
            n += 1;
        }
    }
    rec(
        &Tiles {
            layout,
            of: &tiles[..n],
        },
        0,
        0,
        0,
        0,
        0,
        subracks,
    );
    for s in subracks.iter_mut() {
        s.leave_value = multi_leaves.leave_value(s.leave_idx);
    }
    subracks.sort_unstable_by(|a, b| {
        a.num_played
            .cmp(&b.num_played)
            .then(b.leave_value.cmp(&a.leave_value))
    });
    by_played.clear();
    by_played.resize(num_tiles_on_rack as usize + 2, subracks.len() as u32);
    for (i, s) in subracks.iter().enumerate().rev() {
        by_played[s.num_played as usize] = i as u32;
    }
    for k in (0..by_played.len() - 1).rev() {
        by_played[k] = by_played[k].min(by_played[k + 1]);
    }
}

#[inline]
fn gen_classic_place_moves<
    'a,
    CallbackType: FnMut(i8, &[u8], i32, i32) -> i32,
    N: kwg::Node,
    L: kwg::Node,
>(
    params: &'a mut GenPlaceMovesParams<'a, CallbackType, N, L>,
    source: &'a anagrams::Anagrams,
    single_tile_plays: bool,
) {
    struct Env<'a, CallbackType: FnMut(i8, &[u8], i32, i32) -> i32, N: kwg::Node, L: kwg::Node> {
        params: &'a mut GenPlaceMovesParams<'a, CallbackType, N, L>,
        source: &'a anagrams::Anagrams,
        layout: &'a alphagram::KeyLayout,
        alphabet: &'a alphabet::Alphabet,
        left: i8,
        right: i8,
        num_played: u8,
        base_main: i32,
        base_perp: i32,
        word_multiplier: i32,
        bound: i32,
    }

    #[inline]
    fn check_words<CallbackType: FnMut(i8, &[u8], i32, i32) -> i32, N: kwg::Node, L: kwg::Node>(
        env: &mut Env<'_, CallbackType, N, L>,
        key: alphagram::Key,
        leave_idx: u32,
    ) {
        let source = env.source;
        let len = (env.right - env.left) as u8;
        let Some(found) = source.words(key, len) else {
            return;
        };
        fit_words::<false, _, _, _>(env, found, leave_idx, 0, 0)
    }

    #[inline]
    fn fit_words_apart<
        CallbackType: FnMut(i8, &[u8], i32, i32) -> i32,
        N: kwg::Node,
        L: kwg::Node,
    >(
        env: &mut Env<'_, CallbackType, N, L>,
        found: alphagram::Words<'_>,
        leave_idx: u32,
        blank_letter: u8,
        first_blank: u8,
    ) {
        fit_words::<true, _, _, _>(env, found, leave_idx, blank_letter, first_blank)
    }

    #[inline(always)]
    fn fit_words<
        const BLANKED: bool,
        CallbackType: FnMut(i8, &[u8], i32, i32) -> i32,
        N: kwg::Node,
        L: kwg::Node,
    >(
        env: &mut Env<'_, CallbackType, N, L>,
        found: alphagram::Words<'_>,
        leave_idx: u32,
        blank_letter: u8,
        first_blank: u8,
    ) {
        let len = (env.right - env.left) as u8;
        let board_strip = env.params.board_strip;
        let cross_set_strip = env.params.cross_set_strip;
        let tile_multipliers = env.params.remaining_tile_multipliers_strip;
        let perpendicular_word_multipliers = env.params.perpendicular_word_multipliers_strip;
        let left = env.left as usize;
        debug_assert_eq!(found.len, len);
        'word: for word in found.iter() {
            for (i, &c) in word.iter().enumerate() {
                let pos = left + i;
                let b = board_strip[pos];
                if b != 0 {
                    if c != b & 0x7f {
                        continue 'word;
                    }
                    continue;
                }
                let bits = cross_set_strip[pos].bits;
                if bits != 0 && bits & (1u64 << c) == 0 {
                    continue 'word;
                }
            }
            let mut main_score = env.base_main;
            let mut perpendicular_cumulative_score = env.base_perp;
            for (i, &c) in word.iter().enumerate() {
                let pos = left + i;
                if board_strip[pos] != 0 {
                    continue;
                }
                let tile_value =
                    env.alphabet.score(c) as i32 * equity::SCALE * tile_multipliers[pos] as i32;
                main_score += tile_value;
                perpendicular_cumulative_score +=
                    tile_value * perpendicular_word_multipliers[pos] as i32;
            }
            for (i, &c) in word.iter().enumerate() {
                let pos = left + i;
                if board_strip[pos] == 0 {
                    env.params.word_strip_buffer[pos] = c;
                }
            }
            let score = main_score * env.word_multiplier
                + perpendicular_cumulative_score
                + env
                    .params
                    .board_snapshot
                    .game_config
                    .num_played_bonus(env.num_played) as i32
                    * equity::SCALE;
            let leave_value = leave_value_of(
                env.params.multi_leaves,
                env.params.board_snapshot.klv,
                env.params.rack_tally,
                leave_idx,
                env.params.num_tiles_in_bag,
                env.params.play_out_bonus,
                env.alphabet,
            );
            #[cfg(debug_assertions)]
            macro_rules! covered {
                ($score:expr) => {
                    debug_assert!(
                        $score + leave_value <= env.bound,
                        "found {} when the bound for {} tiles was {}",
                        $score + leave_value,
                        env.num_played,
                        env.bound,
                    );
                };
            }
            #[cfg(not(debug_assertions))]
            macro_rules! covered {
                ($score:expr) => {};
            }
            if !BLANKED {
                covered!(score);
                env.params.threshold = (env.params.callback)(
                    env.left,
                    &env.params.word_strip_buffer[env.left as usize..env.right as usize],
                    score,
                    leave_value,
                );
                continue 'word;
            }
            let blank_value = env.alphabet.scaled_score(0);
            let real_value = env.alphabet.score(blank_letter) as i32 * equity::SCALE;
            if first_blank == 0 {
                for (i, &c) in word.iter().enumerate() {
                    let pos = left + i;
                    if c != blank_letter || board_strip[pos] != 0 {
                        continue;
                    }
                    let delta = (blank_value - real_value) * tile_multipliers[pos] as i32;
                    let blanked_score = score
                        + delta * env.word_multiplier
                        + delta * perpendicular_word_multipliers[pos] as i32;
                    covered!(blanked_score);
                    env.params.word_strip_buffer[pos] = c | 0x80;
                    env.params.threshold = (env.params.callback)(
                        env.left,
                        &env.params.word_strip_buffer[env.left as usize..env.right as usize],
                        blanked_score,
                        leave_value,
                    );
                    env.params.word_strip_buffer[pos] = c;
                }
                continue 'word;
            }
            let first_value = env.alphabet.score(first_blank) as i32 * equity::SCALE;
            for (i, &c) in word.iter().enumerate() {
                let pos = left + i;
                if c != first_blank || board_strip[pos] != 0 {
                    continue;
                }
                let delta1 = (blank_value - first_value) * tile_multipliers[pos] as i32;
                let once_score = score
                    + delta1 * env.word_multiplier
                    + delta1 * perpendicular_word_multipliers[pos] as i32;
                env.params.word_strip_buffer[pos] = c | 0x80;
                for (j, &d) in word.iter().enumerate() {
                    let pos2 = left + j;
                    if d != blank_letter
                        || board_strip[pos2] != 0
                        || pos2 == pos
                        || (first_blank == blank_letter && pos2 < pos)
                    {
                        continue;
                    }
                    let delta2 = (blank_value - real_value) * tile_multipliers[pos2] as i32;
                    let blanked_score = once_score
                        + delta2 * env.word_multiplier
                        + delta2 * perpendicular_word_multipliers[pos2] as i32;
                    covered!(blanked_score);
                    env.params.word_strip_buffer[pos2] = d | 0x80;
                    env.params.threshold = (env.params.callback)(
                        env.left,
                        &env.params.word_strip_buffer[env.left as usize..env.right as usize],
                        blanked_score,
                        leave_value,
                    );
                    env.params.word_strip_buffer[pos2] = d;
                }
                env.params.word_strip_buffer[pos] = c;
            }
        }
    }

    #[derive(Clone, Copy)]
    struct Extent {
        playthrough_key: u128,
        base_main: i32,
        base_perp: i32,
        word_multiplier: i32,
        num_played: u8,
        free_squares: u8,
        dead_squares: u8,
        blank_ok: u64,
    }

    #[inline(always)]
    fn add_square(
        e: &mut Extent,
        idx: usize,
        letters: u64,
        layout: &alphagram::KeyLayout,
        params: &GenPlaceMovesParams<
            '_,
            impl FnMut(i8, &[u8], i32, i32) -> i32,
            impl kwg::Node,
            impl kwg::Node,
        >,
    ) {
        let b = params.board_strip[idx];
        if b != 0 {
            e.base_main += params.face_value_scores_strip[idx];
            e.playthrough_key += layout.place_value(b & 0x7f);
            debug_assert!(layout.holds(e.playthrough_key));
        } else {
            let bits = params.cross_set_strip[idx].bits;
            if bits == 0 {
                e.free_squares += 1;
                e.blank_ok = letters;
            } else if bits == 1 {
                e.dead_squares += 1;
            } else {
                e.blank_ok |= bits & letters;
            }
            e.num_played += 1;
            e.word_multiplier *= params.remaining_word_multipliers_strip[idx] as i32;
            e.base_perp += params.perpendicular_scores_strip[idx];
        }
    }

    let alphabet = params.board_snapshot.game_config.alphabet();
    let layout = source.layout();
    let letters = (u64::MAX >> (64 - alphabet.len() as u32)) & !1;
    let leftmost = params.leftmost;
    let rightmost = params.rightmost;
    let num_max_played = params.num_max_played;
    let mut env = Env {
        params,
        source,
        layout,
        alphabet,
        left: leftmost,
        right: rightmost,
        num_played: 0,
        base_main: 0,
        base_perp: 0,
        word_multiplier: 1,
        bound: 0,
    };
    let mut e = Extent {
        playthrough_key: 0,
        base_main: 0,
        base_perp: 0,
        word_multiplier: 1,
        num_played: 0,
        free_squares: 0,
        dead_squares: 0,
        blank_ok: 0,
    };
    for idx in leftmost..rightmost {
        add_square(&mut e, idx as usize, letters, env.layout, env.params);
    }
    debug_assert!(leftmost == 0 || env.params.board_strip[leftmost as usize - 1] == 0);
    debug_assert!(
        rightmost as usize == env.params.board_strip.len()
            || env.params.board_strip[rightmost as usize] == 0
    );
    debug_assert_eq!(e.num_played, env.params.span_num_played);
    if e.dead_squares != 0
        || e.num_played > num_max_played
        || e.num_played == 0
        || rightmost - leftmost < 2
    {
        return;
    }
    if e.num_played == 1 && !single_tile_plays && e.free_squares == 0 {
        return;
    }
    env.num_played = e.num_played;
    env.base_main = e.base_main;
    env.base_perp = e.base_perp;
    env.word_multiplier = e.word_multiplier;
    let len = (rightmost - leftmost) as u8;
    let from = env.params.subracks_by_played[e.num_played as usize] as usize;
    let upto = env.params.subracks_by_played[e.num_played as usize + 1] as usize;
    for si in from..upto {
        let subrack = env.params.subracks[si];
        env.bound = env.params.score_bound.saturating_add(subrack.leave_value);
        if env.bound < env.params.threshold {
            break;
        }
        let key = e.playthrough_key + subrack.key;
        if subrack.blanks == 0 {
            check_words(&mut env, alphagram::Fitted(key), subrack.leave_idx);
            continue;
        }
        let source = env.source;
        if subrack.blanks >= 2 {
            let key_holds = env.layout.holds(key);
            let mut first = e.blank_ok & !1;
            while first != 0 {
                let l1 = first.trailing_zeros() as u8;
                first &= first - 1;
                let key1 = key + env.layout.place_value(l1);
                if !(key_holds && env.layout.count_in(key, l1) < env.layout.max_count(l1))
                    && !env.layout.holds(key1)
                {
                    continue;
                }
                source.blank_groups(alphagram::Fitted(key1), len, e.blank_ok, |tile, at| {
                    if tile < l1 {
                        return;
                    }
                    let found = source.words_at(at);
                    fit_words_apart(&mut env, found, subrack.leave_idx, tile, l1);
                });
            }
            continue;
        }
        source.blank_groups(alphagram::Fitted(key), len, e.blank_ok, |tile, at| {
            let found = source.words_at(at);
            fit_words_apart(&mut env, found, subrack.leave_idx, tile, 0);
        });
    }
}

#[inline]
fn gen_jumbled_place_moves<
    'a,
    CallbackType: FnMut(i8, &[u8], i32, i32) -> i32,
    N: kwg::Node,
    L: kwg::Node,
>(
    params: &'a mut GenPlaceMovesParams<'a, CallbackType, N, L>,
    single_tile_plays: bool,
) {
    struct Env<'a, CallbackType: FnMut(i8, &[u8], i32, i32) -> i32, N: kwg::Node, L: kwg::Node> {
        params: &'a mut GenPlaceMovesParams<'a, CallbackType, N, L>,
        alphabet: &'a alphabet::Alphabet,
        num_played: u8,
        idx_left: i8,
        alpha_path: [i32; MAX_ALPHABET_LEN + 1],
        alpha_known: u8,
        alpha_dead: bool,
        alpha_bits: u64,
        alpha_walked: u64,
        alpha_stop: u8,
        rack_bits: u64,
        letter_bits: u64,
    }
    struct Accumulator {
        main_score: i32,
        perpendicular_cumulative_score: i32,
        word_multiplier: i32,
        leave_idx: u32,
    }

    #[inline(always)]
    fn tally_moved<CallbackType: FnMut(i8, &[u8], i32, i32) -> i32, N: kwg::Node, L: kwg::Node>(
        env: &mut Env<'_, CallbackType, N, L>,
        letter: u8,
    ) {
        if letter <= env.alpha_known {
            env.alpha_known = letter;
            env.alpha_dead = false;
        }
    }

    #[inline(always)]
    fn tally_add<CallbackType: FnMut(i8, &[u8], i32, i32) -> i32, N: kwg::Node, L: kwg::Node>(
        env: &mut Env<'_, CallbackType, N, L>,
        letter: u8,
    ) {
        env.params.used_letters_tally[letter as usize] += 1;
        env.alpha_bits |= 1 << letter;
        tally_moved(env, letter);
    }

    #[inline(always)]
    fn tally_sub<CallbackType: FnMut(i8, &[u8], i32, i32) -> i32, N: kwg::Node, L: kwg::Node>(
        env: &mut Env<'_, CallbackType, N, L>,
        letter: u8,
    ) {
        env.params.used_letters_tally[letter as usize] -= 1;
        if env.params.used_letters_tally[letter as usize] == 0 {
            env.alpha_bits &= !(1 << letter);
        }
        tally_moved(env, letter);
    }

    #[inline(always)]
    fn rack_take<CallbackType: FnMut(i8, &[u8], i32, i32) -> i32, N: kwg::Node, L: kwg::Node>(
        env: &mut Env<'_, CallbackType, N, L>,
        tile: u8,
    ) {
        env.params.rack_tally[tile as usize] -= 1;
        if env.params.rack_tally[tile as usize] == 0 {
            env.rack_bits &= !(1 << tile);
        }
    }

    #[inline(always)]
    fn rack_put<CallbackType: FnMut(i8, &[u8], i32, i32) -> i32, N: kwg::Node, L: kwg::Node>(
        env: &mut Env<'_, CallbackType, N, L>,
        tile: u8,
    ) {
        env.params.rack_tally[tile as usize] += 1;
        env.rack_bits |= 1 << tile;
    }

    #[inline]
    fn alpha_accepts<
        CallbackType: FnMut(i8, &[u8], i32, i32) -> i32,
        N: kwg::Node,
        L: kwg::Node,
    >(
        env: &mut Env<'_, CallbackType, N, L>,
    ) -> bool {
        if env.alpha_dead {
            return false;
        }
        let kwg = env.params.board_snapshot.kwg;
        let num_letters = env.params.used_letters_tally.len() as u8;
        let below = !0u64 >> (64 - env.alpha_known as u32);
        let above = env.alpha_walked & !below;
        let resume = if above != 0 {
            above.trailing_zeros() as u8
        } else {
            env.alpha_stop
        };
        let mut p = env.alpha_path[resume as usize];
        let mut walked = env.alpha_walked & below;
        let mut rest = env.alpha_bits & !below;
        let mut stop = num_letters;
        let accepted = 'walk: {
            while rest != 0 {
                let letter = rest.trailing_zeros() as u8;
                rest &= rest - 1;
                env.alpha_path[letter as usize] = p;
                walked |= 1 << letter;
                for _ in 0..env.params.used_letters_tally[letter as usize] {
                    p = kwg.seek(p, letter);
                    if p <= 0 {
                        stop = letter;
                        break 'walk false;
                    }
                }
            }
            env.alpha_path[num_letters as usize] = p;
            kwg[p].accepts()
        };
        env.alpha_known = stop;
        env.alpha_stop = stop;
        env.alpha_walked = walked;
        env.alpha_dead = stop < num_letters;
        accepted
    }

    #[inline(always)]
    fn record_if_valid<
        CallbackType: FnMut(i8, &[u8], i32, i32) -> i32,
        N: kwg::Node,
        L: kwg::Node,
    >(
        env: &mut Env<'_, CallbackType, N, L>,
        acc: &Accumulator,
        idx_left: i8,
        idx_right: i8,
    ) {
        let accepted = alpha_accepts(env);
        if accepted {
            let score = acc.main_score * acc.word_multiplier
                + acc.perpendicular_cumulative_score
                + env
                    .params
                    .board_snapshot
                    .game_config
                    .num_played_bonus(env.num_played) as i32
                    * equity::SCALE;
            let leave_value = if env.params.multi_leaves.is_dense() {
                env.params.multi_leaves.leave_value(acc.leave_idx)
            } else if env.params.num_tiles_in_bag <= 0 {
                let is_played_out = env.params.rack_tally.iter().all(|&count| count == 0);
                if is_played_out {
                    env.params.play_out_bonus
                } else {
                    let residual: i32 =
                        (0u8..)
                            .zip(env.params.rack_tally.iter().take(
                                env.params.board_snapshot.game_config.alphabet().len() as usize,
                            ))
                            .map(|(tile, &count)| {
                                count as i32
                                    * env.params.board_snapshot.game_config.alphabet().score(tile)
                                        as i32
                            })
                            .sum();
                    -equity::ENDGAME_PENALTY_BASE - 2 * residual * equity::SCALE
                }
            } else {
                env.params
                    .board_snapshot
                    .klv
                    .leave_value_from_tally(env.params.rack_tally)
            };
            (env.params.callback)(
                idx_left,
                &env.params.word_strip_buffer[idx_left as usize..idx_right as usize],
                score,
                leave_value,
            );
        }
    }

    fn play_right<CallbackType: FnMut(i8, &[u8], i32, i32) -> i32, N: kwg::Node, L: kwg::Node>(
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
            tally_add(env, b & 0x7f);
            acc.main_score += env.params.face_value_scores_strip[idx as usize];
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
                    this_cross_bits &= env.letter_bits;
                } else {
                    this_cross_bits = env.letter_bits;
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
                    let tile_value = env.alphabet.scaled_score(0) * tile_multiplier as i32;
                    Accumulator {
                        main_score: acc.main_score + tile_value,
                        perpendicular_cumulative_score: acc.perpendicular_cumulative_score
                            + perpendicular_score
                            + tile_value * perpendicular_word_multiplier as i32,
                        word_multiplier: new_word_multiplier,
                        leave_idx: acc
                            .leave_idx
                            .wrapping_sub(env.params.multi_leaves.place_value(0)),
                    }
                });
                let mut candidates = if env.rack_bits & 1 != 0 {
                    this_cross_bits
                } else {
                    this_cross_bits & env.rack_bits
                };
                while candidates != 0 {
                    let tile = candidates.trailing_zeros() as u8;
                    candidates &= candidates - 1;
                    if env.params.rack_tally[tile as usize] > 0 {
                        rack_take(env, tile);
                        tally_add(env, tile);
                        let tile_value = env.alphabet.score(tile) as i32
                            * equity::SCALE
                            * tile_multiplier as i32;
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
                            idx + 1,
                            is_unique,
                        );
                        tally_sub(env, tile);
                        rack_put(env, tile);
                    }
                    if let Some(blank_acc) = &opt_blank_acc
                        && (!env.params.is_census || env.params.rack_tally[tile as usize] == 0)
                    {
                        rack_take(env, 0);
                        tally_add(env, tile);
                        env.params.word_strip_buffer[idx as usize] = tile | 0x80;
                        play_right(env, &mut Accumulator { ..*blank_acc }, idx + 1, is_unique);
                        tally_sub(env, tile);
                        rack_put(env, 0);
                    }
                }
                env.num_played -= 1;
            }
        }
        for idx in orig_idx..idx {
            let b = env.params.board_strip[idx as usize];
            tally_sub(env, b & 0x7f);
        }
    }

    fn play_left<CallbackType: FnMut(i8, &[u8], i32, i32) -> i32, N: kwg::Node, L: kwg::Node>(
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
            tally_add(env, b & 0x7f);
            acc.main_score += env.params.face_value_scores_strip[idx as usize];
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
                        this_cross_bits &= env.letter_bits;
                    } else {
                        this_cross_bits = env.letter_bits;
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
                        let tile_value = env.alphabet.scaled_score(0) * tile_multiplier as i32;
                        Accumulator {
                            main_score: acc.main_score + tile_value,
                            perpendicular_cumulative_score: acc.perpendicular_cumulative_score
                                + perpendicular_score
                                + tile_value * perpendicular_word_multiplier as i32,
                            word_multiplier: new_word_multiplier,
                            leave_idx: acc
                                .leave_idx
                                .wrapping_sub(env.params.multi_leaves.place_value(0)),
                        }
                    });
                    let mut candidates = if env.rack_bits & 1 != 0 {
                        this_cross_bits
                    } else {
                        this_cross_bits & env.rack_bits
                    };
                    while candidates != 0 {
                        let tile = candidates.trailing_zeros() as u8;
                        candidates &= candidates - 1;
                        if env.params.rack_tally[tile as usize] > 0 {
                            rack_take(env, tile);
                            tally_add(env, tile);
                            let tile_value = env.alphabet.score(tile) as i32
                                * equity::SCALE
                                * tile_multiplier as i32;
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
                            tally_sub(env, tile);
                            rack_put(env, tile);
                        }
                        if let Some(blank_acc) = &opt_blank_acc
                            && (!env.params.is_census || env.params.rack_tally[tile as usize] == 0)
                        {
                            rack_take(env, 0);
                            tally_add(env, tile);
                            env.params.word_strip_buffer[idx as usize] = tile | 0x80;
                            play_left(env, &mut Accumulator { ..*blank_acc }, idx - 1, is_unique);
                            tally_sub(env, tile);
                            rack_put(env, 0);
                        }
                    }
                    env.num_played -= 1;
                }
            }
        }

        for idx in idx + 1..orig_idx + 1 {
            let b = env.params.board_strip[idx as usize];
            tally_sub(env, b & 0x7f);
        }
    }

    let alphabet = params.board_snapshot.game_config.alphabet();
    let anchor = params.anchor;
    let pass_leave_idx = params.multi_leaves.pass_leave_idx();
    let letter_bits = (1..alphabet.len()).fold(0u64, |bits, tile| bits | 1 << tile);
    let rack_bits = (0..alphabet.len()).fold(0u64, |bits, tile| {
        bits | ((params.rack_tally[tile as usize] > 0) as u64) << tile
    });
    let alpha_bits = (1..alphabet.len()).fold(0u64, |bits, tile| {
        bits | ((params.used_letters_tally[tile as usize] > 0) as u64) << tile
    });
    let mut env = Env {
        params,
        alphabet,
        num_played: 0,
        idx_left: 0,
        alpha_path: [0i32; MAX_ALPHABET_LEN + 1],
        alpha_known: 1,
        alpha_dead: false,
        alpha_bits,
        alpha_walked: 0,
        alpha_stop: 1,
        rack_bits,
        letter_bits,
    };
    play_left(
        &mut env,
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
fn gen_place_moves_lean<
    'a,
    CallbackType: FnMut(i8, &[u8], i32, i32) -> i32,
    N: kwg::Node,
    L: kwg::Node,
>(
    params: &'a mut GenPlaceMovesParams<'a, CallbackType, N, L>,
    single_tile_plays: bool,
) {
    match params.board_snapshot.game_config.game_rules() {
        game_config::GameRules::Classic => gen_classic_place_moves_lean(params, single_tile_plays),
        game_config::GameRules::Jumbled => gen_jumbled_place_moves(params, single_tile_plays),
    }
}

#[inline(always)]
fn gen_place_moves<
    'a,
    CallbackType: FnMut(i8, &[u8], i32, i32) -> i32,
    N: kwg::Node,
    L: kwg::Node,
>(
    params: &'a mut GenPlaceMovesParams<'a, CallbackType, N, L>,
    single_tile_plays: bool,
) {
    match params.board_snapshot.game_config.game_rules() {
        game_config::GameRules::Classic => match params.board_snapshot.anagrams {
            Some(held) => gen_classic_place_moves(params, held, single_tile_plays),
            None => gen_classic_place_moves_lean(params, single_tile_plays),
        },
        game_config::GameRules::Jumbled => gen_jumbled_place_moves(params, single_tile_plays),
    }
}

struct GenPlaceMovesAtParams<
    'a,
    FoundPlaceMove: FnMut(bool, i8, i8, &[u8], i32, i32) -> i32,
    N: kwg::Node,
    L: kwg::Node,
> {
    board_snapshot: &'a BoardSnapshot<'a, N, L>,
    working_buffer: &'a mut WorkingBuffer,
    multi_leaves: &'a klv::MultiLeaves,
    placement: &'a PossiblePlacement,
    num_max_played: u8,
    threshold: i32,
    found_place_move: FoundPlaceMove,
}

#[inline(always)]
fn gen_place_moves_at_lean<
    'a,
    FoundPlaceMove: FnMut(bool, i8, i8, &[u8], i32, i32) -> i32,
    N: kwg::Node,
    L: kwg::Node,
>(
    p: GenPlaceMovesAtParams<'a, FoundPlaceMove, N, L>,
) {
    let GenPlaceMovesAtParams {
        board_snapshot,
        working_buffer,
        multi_leaves,
        placement,
        num_max_played,
        threshold,
        mut found_place_move,
    } = p;
    let dim = board_snapshot.game_config.board_layout().dim();
    let strip_range_start;

    let strip_range_end = if placement.down {
        strip_range_start = (placement.lane as isize * dim.rows as isize) as usize;
        strip_range_start + dim.rows as usize
    } else {
        strip_range_start = (placement.lane as isize * dim.cols as isize) as usize;
        strip_range_start + dim.cols as usize
    };
    gen_place_moves_lean(
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
            cross_set_buffer_strip: if placement.down {
                &working_buffer.cross_set_buffer_for_across_plays
                    [strip_range_start..strip_range_end]
            } else {
                &working_buffer.cross_set_buffer_for_down_plays[strip_range_start..strip_range_end]
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
            rack_bits: working_buffer.rack_bits,
            word_strip_buffer: if placement.down {
                &mut working_buffer.word_buffer_for_down_plays[strip_range_start..strip_range_end]
            } else {
                &mut working_buffer.word_buffer_for_across_plays[strip_range_start..strip_range_end]
            },
            num_max_played,
            anchor: placement.anchor,
            leftmost: placement.leftmost,
            rightmost: placement.rightmost,
            span_num_played: placement.num_played,
            callback: |idx: i8, word: &[u8], score: i32, leave_value: i32| {
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
            num_tiles_in_bag: working_buffer.num_tiles_in_bag,
            play_out_bonus: working_buffer.play_out_bonus,
            used_letters_tally: &mut working_buffer.used_letters_tally,
            is_census: working_buffer.is_census,
            score_bound: i32::MAX,
            threshold,
            subracks: &working_buffer.subracks,
            subracks_by_played: &working_buffer.subracks_by_played,
        },
        !placement.down,
    );
}

#[inline(always)]
fn gen_place_moves_at<
    'a,
    FoundPlaceMove: FnMut(bool, i8, i8, &[u8], i32, i32) -> i32,
    N: kwg::Node,
    L: kwg::Node,
>(
    p: GenPlaceMovesAtParams<'a, FoundPlaceMove, N, L>,
) {
    let GenPlaceMovesAtParams {
        board_snapshot,
        working_buffer,
        multi_leaves,
        placement,
        num_max_played,
        threshold,
        mut found_place_move,
    } = p;
    let dim = board_snapshot.game_config.board_layout().dim();
    let strip_range_start;

    let strip_range_end = if placement.down {
        strip_range_start = (placement.lane as isize * dim.rows as isize) as usize;
        strip_range_start + dim.rows as usize
    } else {
        strip_range_start = (placement.lane as isize * dim.cols as isize) as usize;
        strip_range_start + dim.cols as usize
    };
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
            cross_set_buffer_strip: if placement.down {
                &working_buffer.cross_set_buffer_for_across_plays
                    [strip_range_start..strip_range_end]
            } else {
                &working_buffer.cross_set_buffer_for_down_plays[strip_range_start..strip_range_end]
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
            rack_bits: working_buffer.rack_bits,
            word_strip_buffer: if placement.down {
                &mut working_buffer.word_buffer_for_down_plays[strip_range_start..strip_range_end]
            } else {
                &mut working_buffer.word_buffer_for_across_plays[strip_range_start..strip_range_end]
            },
            num_max_played,
            anchor: placement.anchor,
            leftmost: placement.leftmost,
            rightmost: placement.rightmost,
            span_num_played: placement.num_played,
            callback: |idx: i8, word: &[u8], score: i32, leave_value: i32| {
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
            num_tiles_in_bag: working_buffer.num_tiles_in_bag,
            play_out_bonus: working_buffer.play_out_bonus,
            used_letters_tally: &mut working_buffer.used_letters_tally,
            is_census: working_buffer.is_census,
            score_bound: placement.best_possible_equity
                - working_buffer.best_leave_values[placement.num_played as usize]
                + board_snapshot
                    .game_config
                    .num_played_bonus(placement.num_played) as i32
                    * equity::SCALE,
            threshold,
            subracks: &working_buffer.subracks,
            subracks_by_played: &working_buffer.subracks_by_played,
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
    pub equity: equity::Equity,
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
        self.equity == other.equity && self.play == other.play
    }
}

impl Eq for ValuedMove {}

impl PartialOrd for ValuedMove {
    #[inline(always)]
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

#[inline(always)]
fn cmp_play(a: &Play, b: &Play) -> std::cmp::Ordering {
    match (a, b) {
        (
            Play::Place {
                down: a_down,
                lane: a_lane,
                idx: a_idx,
                word: a_word,
                score: a_score,
            },
            Play::Place {
                down: b_down,
                lane: b_lane,
                idx: b_idx,
                word: b_word,
                score: b_score,
            },
        ) => a_down
            .cmp(b_down)
            .then_with(|| a_lane.cmp(b_lane))
            .then_with(|| a_idx.cmp(b_idx))
            .then_with(|| a_word.cmp(b_word))
            .then_with(|| a_score.cmp(b_score)),
        (Play::Exchange { tiles: a_tiles }, Play::Exchange { tiles: b_tiles }) => {
            a_tiles.cmp(b_tiles)
        }
        (Play::Place { .. }, Play::Exchange { .. }) => std::cmp::Ordering::Less,
        (Play::Exchange { .. }, Play::Place { .. }) => std::cmp::Ordering::Greater,
    }
}

impl Ord for ValuedMove {
    #[inline(always)]
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        other
            .equity
            .cmp(&self.equity)
            .then_with(|| cmp_play(&self.play, &other.play))
    }
}

pub struct WriteablePlay<'a, N: kwg::Node, L: kwg::Node> {
    board_snapshot: &'a BoardSnapshot<'a, N, L>,
    play: &'a Play,
}

impl<N: kwg::Node, L: kwg::Node> std::fmt::Display for WriteablePlay<'_, N, L> {
    #[inline]
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
                write!(f, " {}", equity::descale_score(*score))?;
            }
        }
        Ok(())
    }
}

impl Play {
    #[inline]
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

#[derive(Clone, Copy)]
pub enum PassPolicy {
    OnlyWhenForced,
    AsACandidate,
}

impl PassPolicy {
    #[inline(always)]
    fn emits_unconditionally(self) -> bool {
        match self {
            PassPolicy::OnlyWhenForced => false,
            PassPolicy::AsACandidate => true,
        }
    }
}

pub struct GenMovesParams<'a, N: kwg::Node, L: kwg::Node> {
    pub board_snapshot: &'a BoardSnapshot<'a, N, L>,
    pub rack: &'a [u8],
    pub max_gen: usize,
    pub num_exchanges_by_this_player: i16,
    pub pass_policy: PassPolicy,
    pub dynamic_leaves: Option<klv::DynamicLeavesRef<'a>>,
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
    #[inline(always)]
    pub fn new(game_config: &game_config::GameConfig) -> Self {
        let working_buffer = WorkingBuffer::new(game_config);
        Self {
            working_buffer,
            plays: Vec::new(),
        }
    }
}

impl KurniaMoveGenerator {
    #[inline(always)]
    pub fn num_tiles_in_bag(&self) -> i16 {
        self.working_buffer.num_tiles_in_bag
    }

    // call this before passing a different kwg.
    #[inline(always)]
    pub fn reset_for_another_kwg(&mut self) {
        self.working_buffer.reset_for_another_kwg();
    }

    // skip equity computation and sorting
    #[inline(always)]
    pub fn gen_moves_raw_all_unsorted<'a, N: kwg::Node, L: kwg::Node>(
        &mut self,
        board_snapshot: &'a BoardSnapshot<'a, N, L>,
        rack: &'a [u8],
        num_exchanges_by_this_player: i16,
        pass_policy: PassPolicy,
    ) {
        self.gen_moves_raw_all_unsorted_impl::<false, _, _>(
            board_snapshot,
            rack,
            num_exchanges_by_this_player,
            pass_policy,
        )
    }

    #[inline(always)]
    pub fn gen_moves_raw_all_unsorted_lean<'a, N: kwg::Node, L: kwg::Node>(
        &mut self,
        board_snapshot: &'a BoardSnapshot<'a, N, L>,
        rack: &'a [u8],
        num_exchanges_by_this_player: i16,
        pass_policy: PassPolicy,
    ) {
        self.gen_moves_raw_all_unsorted_impl::<true, _, _>(
            board_snapshot,
            rack,
            num_exchanges_by_this_player,
            pass_policy,
        )
    }

    #[inline]
    fn gen_moves_raw_all_unsorted_impl<'a, const LEAN: bool, N: kwg::Node, L: kwg::Node>(
        &mut self,
        board_snapshot: &'a BoardSnapshot<'a, N, L>,
        rack: &'a [u8],
        num_exchanges_by_this_player: i16,
        pass_policy: PassPolicy,
    ) {
        self.plays.clear();
        let mut vec_moves = std::mem::take(&mut self.plays);

        let working_buffer = &mut self.working_buffer;
        working_buffer.init(board_snapshot, rack, klv::AdjustLeave::Identity, None);
        let multi_leaves = std::mem::take(&mut working_buffer.multi_leaves);

        let found_place_move =
            |down: bool, lane: i8, idx: i8, word: &[u8], score: i32, _leave_value: i32| {
                vec_moves.push(ValuedMove {
                    equity: equity::Equity::ZERO,
                    play: Play::Place {
                        down,
                        lane,
                        idx,
                        word: word.into(),
                        score,
                    },
                });
                i32::MIN
            };
        if !LEAN && working_buffer.turn_is_supported(true, board_snapshot) {
            for _ in kurnia_gen_place_moves_iter(KurniaIterParams {
                want_raw: true,
                board_snapshot,
                working_buffer,
                multi_leaves: &multi_leaves,
                found_place_move,
                can_accept: |_best_possible_equity: i32| true,
                current_threshold: || i32::MIN,
            }) {}
        } else {
            for _ in kurnia_gen_place_moves_iter_lean(KurniaIterParams {
                want_raw: true,
                board_snapshot,
                working_buffer,
                multi_leaves: &multi_leaves,
                found_place_move,
                can_accept: |_best_possible_equity: i32| true,
                current_threshold: || i32::MIN,
            }) {}
        }
        kurnia_gen_exchange_moves(
            board_snapshot,
            working_buffer,
            &multi_leaves,
            num_exchanges_by_this_player,
            |exchanged_tiles: &[u8], _leave_value: i32| {
                vec_moves.push(ValuedMove {
                    equity: equity::Equity::ZERO,
                    play: Play::Exchange {
                        tiles: exchanged_tiles.into(),
                    },
                });
            },
        );
        if pass_policy.emits_unconditionally() || vec_moves.is_empty() {
            vec_moves.push(ValuedMove {
                equity: equity::Equity::ZERO,
                play: Play::Exchange {
                    tiles: (&working_buffer.exchange_buffer[..]).into(),
                },
            });
        }

        self.plays = vec_moves;

        working_buffer.multi_leaves = multi_leaves;
    }

    #[inline(always)]
    pub async fn gen_moves_filtered_async<
        'a,
        BreatheFuture: std::future::Future,
        N: kwg::Node,
        L: kwg::Node,
    >(
        &mut self,
        params: &'a GenMovesParams<'a, N, L>,
        place_move_predicate: PlacePredicate<'_>,
        adjust_leave_value: klv::AdjustLeave,
        equity_predicate: EquityPredicate<'_>,
        breathe: impl FnMut() -> BreatheFuture,
    ) {
        self.gen_moves_filtered_async_impl::<false, _, _, _>(
            params,
            place_move_predicate,
            adjust_leave_value,
            equity_predicate,
            breathe,
        )
        .await
    }

    #[inline(always)]
    pub async fn gen_moves_filtered_async_lean<
        'a,
        BreatheFuture: std::future::Future,
        N: kwg::Node,
        L: kwg::Node,
    >(
        &mut self,
        params: &'a GenMovesParams<'a, N, L>,
        place_move_predicate: PlacePredicate<'_>,
        adjust_leave_value: klv::AdjustLeave,
        equity_predicate: EquityPredicate<'_>,
        breathe: impl FnMut() -> BreatheFuture,
    ) {
        self.gen_moves_filtered_async_impl::<true, _, _, _>(
            params,
            place_move_predicate,
            adjust_leave_value,
            equity_predicate,
            breathe,
        )
        .await
    }

    #[inline]
    async fn gen_moves_filtered_async_impl<
        'a,
        const LEAN: bool,
        BreatheFuture: std::future::Future,
        N: kwg::Node,
        L: kwg::Node,
    >(
        &mut self,
        params: &'a GenMovesParams<'a, N, L>,
        mut place_move_predicate: PlacePredicate<'_>,
        adjust_leave_value: klv::AdjustLeave,
        equity_predicate: EquityPredicate<'_>,
        mut breathe: impl FnMut() -> BreatheFuture,
    ) {
        self.plays.clear();
        if params.max_gen == 0 {
            return;
        }

        let alphabet = params.board_snapshot.game_config.alphabet();
        let board_layout = params.board_snapshot.game_config.board_layout();
        let max_gen = params.max_gen;

        let mut found_moves = std::collections::BinaryHeap::from(std::mem::take(&mut self.plays));
        let mut equity_predicate = equity_predicate;
        let threshold = std::cell::Cell::new(equity::Equity::NEG_INFINITY);

        #[inline(always)]
        fn push_move<F: FnMut() -> Play>(
            found_moves: &mut std::collections::BinaryHeap<ValuedMove>,
            equity_pred: &mut EquityPredicate<'_>,
            threshold: &std::cell::Cell<equity::Equity>,
            max_gen: usize,
            equity: equity::Equity,
            mut construct_play: F,
        ) {
            if found_moves.len() >= max_gen && threshold.get() > equity {
                return;
            }
            let play = construct_play();
            if equity_pred.test(equity, &play) {
                if found_moves.len() >= max_gen {
                    let candidate = ValuedMove { equity, play };
                    let mut worst = found_moves.peek_mut().unwrap();
                    if candidate >= *worst {
                        return;
                    }
                    *worst = candidate;
                } else {
                    found_moves.push(ValuedMove { equity, play });
                }
                if found_moves.len() >= max_gen {
                    threshold.set(found_moves.peek().unwrap().equity);
                }
            }
        }

        let working_buffer = &mut self.working_buffer;
        working_buffer.init(
            params.board_snapshot,
            params.rack,
            adjust_leave_value,
            params.dynamic_leaves,
        );
        let multi_leaves = std::mem::take(&mut working_buffer.multi_leaves);
        let num_tiles_on_board = working_buffer.num_tiles_on_board;

        let found_place_move =
            |down: bool, lane: i8, idx: i8, word: &[u8], score: i32, leave_value: i32| {
                if place_move_predicate.test(down, lane, idx, word, score) {
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
                            .count() as i32
                            * -equity::OPENING_HOTSPOT_PENALTY
                    } else {
                        0
                    };
                    let equity = equity::Equity::new(score + leave_value + other_adjustments);
                    push_move(
                        &mut found_moves,
                        &mut equity_predicate,
                        &threshold,
                        max_gen,
                        equity,
                        || Play::Place {
                            down,
                            lane,
                            idx,
                            word: word.into(),
                            score,
                        },
                    );
                }
                threshold.get().raw()
            };
        if !LEAN && working_buffer.turn_is_supported(false, params.board_snapshot) {
            for _ in kurnia_gen_place_moves_iter(KurniaIterParams {
                want_raw: false,
                board_snapshot: params.board_snapshot,
                working_buffer,
                multi_leaves: &multi_leaves,
                found_place_move,
                can_accept: |best_possible_equity: i32| {
                    threshold.get() <= equity::Equity::new(best_possible_equity)
                },
                current_threshold: || threshold.get().raw(),
            }) {
                breathe().await;
            }
        } else {
            for _ in kurnia_gen_place_moves_iter_lean(KurniaIterParams {
                want_raw: false,
                board_snapshot: params.board_snapshot,
                working_buffer,
                multi_leaves: &multi_leaves,
                found_place_move,
                can_accept: |best_possible_equity: i32| {
                    threshold.get() <= equity::Equity::new(best_possible_equity)
                },
                current_threshold: || threshold.get().raw(),
            }) {
                breathe().await;
            }
        }
        kurnia_gen_exchange_moves(
            params.board_snapshot,
            working_buffer,
            &multi_leaves,
            params.num_exchanges_by_this_player,
            |exchanged_tiles: &[u8], leave_value: i32| {
                push_move(
                    &mut found_moves,
                    &mut equity_predicate,
                    &threshold,
                    max_gen,
                    equity::Equity::new(leave_value),
                    || Play::Exchange {
                        tiles: exchanged_tiles.into(),
                    },
                );
            },
        );
        if params.pass_policy.emits_unconditionally() || found_moves.is_empty() {
            push_move(
                &mut found_moves,
                &mut equity_predicate,
                &threshold,
                max_gen,
                equity::Equity::new(if multi_leaves.is_dense() {
                    multi_leaves.pass_leave_value()
                } else {
                    params
                        .board_snapshot
                        .klv
                        .leave_value_from_tally(&working_buffer.rack_tally)
                }),
                || Play::Exchange {
                    tiles: (&working_buffer.exchange_buffer[..]).into(),
                },
            );
        }

        self.plays = found_moves.into_sorted_vec();

        working_buffer.multi_leaves = multi_leaves;
    }

    #[inline(always)]
    // The census sheet wants each WORD once, not each PLAY, so its descent
    // takes a real tile before a blank. That is a different generator, not a
    // setting: the placement path that reads a word source does not run it.
    pub fn gen_census_sheet<'a, N: kwg::Node, L: kwg::Node>(
        &mut self,
        params: &'a GenMovesParams<'a, N, L>,
        place_move_predicate: PlacePredicate<'_>,
        adjust_leave_value: klv::AdjustLeave,
        equity_predicate: EquityPredicate<'_>,
    ) {
        self.working_buffer.is_census = true;
        self.gen_moves_filtered(
            params,
            place_move_predicate,
            adjust_leave_value,
            equity_predicate,
        );
        self.working_buffer.is_census = false;
    }

    #[inline(always)]
    pub fn gen_moves_filtered<'a, N: kwg::Node, L: kwg::Node>(
        &mut self,
        params: &'a GenMovesParams<'a, N, L>,
        place_move_predicate: PlacePredicate<'_>,
        adjust_leave_value: klv::AdjustLeave,
        equity_predicate: EquityPredicate<'_>,
    ) {
        self.gen_moves_filtered_impl::<false, _, _>(
            params,
            place_move_predicate,
            adjust_leave_value,
            equity_predicate,
        )
    }

    #[inline(always)]
    pub fn gen_moves_filtered_lean<'a, N: kwg::Node, L: kwg::Node>(
        &mut self,
        params: &'a GenMovesParams<'a, N, L>,
        place_move_predicate: PlacePredicate<'_>,
        adjust_leave_value: klv::AdjustLeave,
        equity_predicate: EquityPredicate<'_>,
    ) {
        self.gen_moves_filtered_impl::<true, _, _>(
            params,
            place_move_predicate,
            adjust_leave_value,
            equity_predicate,
        )
    }

    #[inline]
    fn gen_moves_filtered_impl<'a, const LEAN: bool, N: kwg::Node, L: kwg::Node>(
        &mut self,
        params: &'a GenMovesParams<'a, N, L>,
        mut place_move_predicate: PlacePredicate<'_>,
        adjust_leave_value: klv::AdjustLeave,
        equity_predicate: EquityPredicate<'_>,
    ) {
        self.plays.clear();
        if params.max_gen == 0 {
            return;
        }

        let alphabet = params.board_snapshot.game_config.alphabet();
        let board_layout = params.board_snapshot.game_config.board_layout();
        let max_gen = params.max_gen;

        let mut found_moves = std::collections::BinaryHeap::from(std::mem::take(&mut self.plays));
        let mut equity_predicate = equity_predicate;
        let threshold = std::cell::Cell::new(equity::Equity::NEG_INFINITY);

        #[inline(always)]
        fn push_move<F: FnMut() -> Play>(
            found_moves: &mut std::collections::BinaryHeap<ValuedMove>,
            equity_pred: &mut EquityPredicate<'_>,
            threshold: &std::cell::Cell<equity::Equity>,
            max_gen: usize,
            equity: equity::Equity,
            mut construct_play: F,
        ) {
            if found_moves.len() >= max_gen && threshold.get() > equity {
                return;
            }
            let play = construct_play();
            if equity_pred.test(equity, &play) {
                if found_moves.len() >= max_gen {
                    let candidate = ValuedMove { equity, play };
                    let mut worst = found_moves.peek_mut().unwrap();
                    if candidate >= *worst {
                        return;
                    }
                    *worst = candidate;
                } else {
                    found_moves.push(ValuedMove { equity, play });
                }
                if found_moves.len() >= max_gen {
                    threshold.set(found_moves.peek().unwrap().equity);
                }
            }
        }

        let working_buffer = &mut self.working_buffer;
        working_buffer.init(
            params.board_snapshot,
            params.rack,
            adjust_leave_value,
            params.dynamic_leaves,
        );
        let multi_leaves = std::mem::take(&mut working_buffer.multi_leaves);
        let num_tiles_on_board = working_buffer.num_tiles_on_board;

        let found_place_move =
            |down: bool, lane: i8, idx: i8, word: &[u8], score: i32, leave_value: i32| {
                if place_move_predicate.test(down, lane, idx, word, score) {
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
                            .count() as i32
                            * -equity::OPENING_HOTSPOT_PENALTY
                    } else {
                        0
                    };
                    let equity = equity::Equity::new(score + leave_value + other_adjustments);
                    push_move(
                        &mut found_moves,
                        &mut equity_predicate,
                        &threshold,
                        max_gen,
                        equity,
                        || Play::Place {
                            down,
                            lane,
                            idx,
                            word: word.into(),
                            score,
                        },
                    );
                }
                threshold.get().raw()
            };
        if !LEAN && working_buffer.turn_is_supported(false, params.board_snapshot) {
            for _ in kurnia_gen_place_moves_iter(KurniaIterParams {
                want_raw: false,
                board_snapshot: params.board_snapshot,
                working_buffer,
                multi_leaves: &multi_leaves,
                found_place_move,
                can_accept: |best_possible_equity: i32| {
                    threshold.get() <= equity::Equity::new(best_possible_equity)
                },
                current_threshold: || threshold.get().raw(),
            }) {}
        } else {
            for _ in kurnia_gen_place_moves_iter_lean(KurniaIterParams {
                want_raw: false,
                board_snapshot: params.board_snapshot,
                working_buffer,
                multi_leaves: &multi_leaves,
                found_place_move,
                can_accept: |best_possible_equity: i32| {
                    threshold.get() <= equity::Equity::new(best_possible_equity)
                },
                current_threshold: || threshold.get().raw(),
            }) {}
        }
        kurnia_gen_exchange_moves(
            params.board_snapshot,
            working_buffer,
            &multi_leaves,
            params.num_exchanges_by_this_player,
            |exchanged_tiles: &[u8], leave_value: i32| {
                push_move(
                    &mut found_moves,
                    &mut equity_predicate,
                    &threshold,
                    max_gen,
                    equity::Equity::new(leave_value),
                    || Play::Exchange {
                        tiles: exchanged_tiles.into(),
                    },
                );
            },
        );
        if params.pass_policy.emits_unconditionally() || found_moves.is_empty() {
            push_move(
                &mut found_moves,
                &mut equity_predicate,
                &threshold,
                max_gen,
                equity::Equity::new(if multi_leaves.is_dense() {
                    multi_leaves.pass_leave_value()
                } else {
                    params
                        .board_snapshot
                        .klv
                        .leave_value_from_tally(&working_buffer.rack_tally)
                }),
                || Play::Exchange {
                    tiles: (&working_buffer.exchange_buffer[..]).into(),
                },
            );
        }

        self.plays = found_moves.into_sorted_vec();

        working_buffer.multi_leaves = multi_leaves;
    }

    #[inline(always)]
    pub fn gen_moves_unfiltered<'a, N: kwg::Node, L: kwg::Node>(
        &mut self,
        params: &'a GenMovesParams<'a, N, L>,
    ) {
        self.gen_moves_filtered(
            params,
            PlacePredicate::AcceptAll,
            klv::AdjustLeave::Identity,
            EquityPredicate::AcceptAll,
        );
    }

    #[inline(always)]
    pub fn gen_moves_unfiltered_lean<'a, N: kwg::Node, L: kwg::Node>(
        &mut self,
        params: &'a GenMovesParams<'a, N, L>,
    ) {
        self.gen_moves_filtered_lean(
            params,
            PlacePredicate::AcceptAll,
            klv::AdjustLeave::Identity,
            EquityPredicate::AcceptAll,
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
        working_buffer.init(board_snapshot, &[], klv::AdjustLeave::Identity, None);
        working_buffer.prev_board_tiles.fill(0xff);
        gen_remaining_words(board_snapshot, working_buffer, found_word)
    }
}

#[inline]
fn kurnia_gen_exchange_moves<
    'a,
    FoundExchangeMove: FnMut(&[u8], i32),
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
        if multi_leaves.is_dense() {
            multi_leaves.kurnia_gen_exchange_moves_unconditionally(
                found_exchange_move,
                &mut working_buffer.rack_tally,
                &mut working_buffer.exchange_buffer,
                working_buffer.num_tiles_in_bag as usize,
            );
        } else {
            klv::MultiLeaves::gen_exchange_moves_via_klv(
                board_snapshot.klv,
                found_exchange_move,
                &mut working_buffer.rack_tally,
                &mut working_buffer.exchange_buffer,
                working_buffer.num_tiles_in_bag as usize,
            );
        }
    }
}

struct KurniaIterParams<
    'a,
    FoundPlaceMove: 'a + FnMut(bool, i8, i8, &[u8], i32, i32) -> i32,
    CanAccept: 'a + Fn(i32) -> bool,
    CurrentThreshold: 'a + Fn() -> i32,
    N: kwg::Node,
    L: kwg::Node,
> {
    want_raw: bool,
    board_snapshot: &'a BoardSnapshot<'a, N, L>,
    working_buffer: &'a mut WorkingBuffer,
    multi_leaves: &'a klv::MultiLeaves,
    found_place_move: FoundPlaceMove,
    can_accept: CanAccept,
    current_threshold: CurrentThreshold,
}

#[inline]
fn kurnia_gen_place_moves_iter_lean<
    'a,
    FoundPlaceMove: 'a + FnMut(bool, i8, i8, &[u8], i32, i32) -> i32,
    CanAccept: 'a + Fn(i32) -> bool,
    CurrentThreshold: 'a + Fn() -> i32,
    N: kwg::Node,
    L: kwg::Node,
>(
    p: KurniaIterParams<'a, FoundPlaceMove, CanAccept, CurrentThreshold, N, L>,
) -> impl 'a + Iterator {
    let KurniaIterParams {
        want_raw,
        board_snapshot,
        working_buffer,
        multi_leaves,
        mut found_place_move,
        can_accept,
        current_threshold,
    } = p;
    let game_config = &board_snapshot.game_config;
    let board_layout = game_config.board_layout();
    let dim = board_layout.dim();
    let max_rack_size = game_config.rack_size();
    let num_max_played = max_rack_size.min(working_buffer.num_tiles_on_rack);

    let area = (dim.rows as isize * dim.cols as isize) as usize;
    let mut dirty_rows = 0u128;
    let mut dirty_cols = 0u128;
    for (idx, (&cur, &prev)) in board_snapshot.board_tiles[..area]
        .iter()
        .zip(working_buffer.prev_board_tiles[..area].iter())
        .enumerate()
    {
        if cur != prev {
            let row = idx / dim.cols as usize;
            let col = idx % dim.cols as usize;
            dirty_rows |= 1 << row;
            dirty_cols |= 1 << col;
        }
    }
    for col in 0..dim.cols {
        if dirty_cols & (1 << col) != 0 {
            let strip_range_start = (col as isize * dim.rows as isize) as usize;
            let strip_range_end = strip_range_start + dim.rows as usize;
            gen_cross_set(
                board_snapshot,
                &working_buffer.transposed_board_tiles[strip_range_start..strip_range_end],
                &mut working_buffer.cross_set_for_across_plays,
                dim.down(col),
                &mut working_buffer.cross_set_buffer_for_across_plays
                    [strip_range_start..strip_range_end],
                &mut working_buffer.cached_cross_set_for_across_plays
                    [strip_range_start..strip_range_end],
                &mut working_buffer.used_letters_tally,
            );
        }
    }
    let transposed_dim = matrix::Dim {
        rows: dim.cols,
        cols: dim.rows,
    };

    for row in 0..dim.rows {
        if dirty_rows & (1 << row) != 0 {
            let strip_range_start = (row as isize * dim.cols as isize) as usize;
            let strip_range_end = strip_range_start + dim.cols as usize;
            gen_cross_set(
                board_snapshot,
                &board_snapshot.board_tiles[strip_range_start..strip_range_end],
                &mut working_buffer.cross_set_for_down_plays,
                transposed_dim.down(row),
                &mut working_buffer.cross_set_buffer_for_down_plays
                    [strip_range_start..strip_range_end],
                &mut working_buffer.cached_cross_set_for_down_plays
                    [strip_range_start..strip_range_end],
                &mut working_buffer.used_letters_tally,
            );
        }
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

    if dirty_rows != 0 || dirty_cols != 0 {
        working_buffer.prev_board_tiles[..area]
            .copy_from_slice(&board_snapshot.board_tiles[..area]);
    }
    working_buffer.init_after_cross_sets(board_snapshot, dirty_cols, dirty_rows);

    let feasible_lengths = feasible_word_lengths::<false>(
        board_snapshot.anagrams,
        board_snapshot.rack_lengths,
        &working_buffer.rack_tally,
    );
    let mut found_placements = std::mem::take(&mut working_buffer.found_placements);
    found_placements.clear();
    let mut placement_order = std::mem::take(&mut working_buffer.placement_order);
    placement_order.clear();
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
                feasible_lengths,
                descending_scores: &working_buffer.descending_scores,
                lane_scaffold: &mut working_buffer.lane_scaffold[row as usize],
                best_leave_values: &working_buffer.best_leave_values,
                num_max_played,
                rack_tally_shadowl: &mut working_buffer.rack_tally_shadowl,
                rack_tally_shadowr: &mut working_buffer.rack_tally_shadowr,
                span_out: &mut working_buffer.span_out,
                per_span: false,
            },
            true,
            want_raw,
            |anchor: i8, leftmost: i8, rightmost: i8, best_possible_equity: i32, num_played: u8| {
                found_placements.push(PossiblePlacement {
                    num_played,
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
                feasible_lengths,
                descending_scores: &working_buffer.descending_scores,
                lane_scaffold: &mut working_buffer.lane_scaffold[dim.rows as usize + col as usize],
                best_leave_values: &working_buffer.best_leave_values,
                num_max_played,
                rack_tally_shadowl: &mut working_buffer.rack_tally_shadowl,
                rack_tally_shadowr: &mut working_buffer.rack_tally_shadowr,
                span_out: &mut working_buffer.span_out,
                per_span: false,
            },
            false,
            want_raw,
            |anchor: i8, leftmost: i8, rightmost: i8, best_possible_equity: i32, num_played: u8| {
                found_placements.push(PossiblePlacement {
                    num_played,
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
    placement_order.extend(
        found_placements
            .iter()
            .enumerate()
            .map(|(i, p)| (p.best_possible_equity, i as u32)),
    );
    if !want_raw {
        placement_order.sort_unstable_by_key(|&(equity, _)| equity);
    }
    working_buffer.found_placements = found_placements;
    working_buffer.placement_order = placement_order;
    std::iter::from_fn(move || match working_buffer.placement_order.pop() {
        Some((equity, idx)) => {
            if can_accept(equity) {
                let placement = working_buffer.found_placements[idx as usize];
                gen_place_moves_at_lean(GenPlaceMovesAtParams {
                    board_snapshot,
                    working_buffer,
                    multi_leaves,
                    placement: &placement,
                    num_max_played,
                    threshold: current_threshold(),
                    found_place_move:
                        &mut |down: bool,
                              lane: i8,
                              idx: i8,
                              word: &[u8],
                              score: i32,
                              leave_value: i32| {
                            let this_best = score + leave_value;
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
                });
                Some(())
            } else {
                // fuse the iterator
                working_buffer.placement_order.clear();
                None
            }
        }
        None => None,
    })
}

#[inline]
fn kurnia_gen_place_moves_iter<
    'a,
    FoundPlaceMove: 'a + FnMut(bool, i8, i8, &[u8], i32, i32) -> i32,
    CanAccept: 'a + Fn(i32) -> bool,
    CurrentThreshold: 'a + Fn() -> i32,
    N: kwg::Node,
    L: kwg::Node,
>(
    p: KurniaIterParams<'a, FoundPlaceMove, CanAccept, CurrentThreshold, N, L>,
) -> impl 'a + Iterator {
    let KurniaIterParams {
        want_raw,
        board_snapshot,
        working_buffer,
        multi_leaves,
        mut found_place_move,
        can_accept,
        current_threshold,
    } = p;
    let game_config = &board_snapshot.game_config;
    let board_layout = game_config.board_layout();
    let dim = board_layout.dim();
    let max_rack_size = game_config.rack_size();
    let num_max_played = max_rack_size.min(working_buffer.num_tiles_on_rack);

    let area = (dim.rows as isize * dim.cols as isize) as usize;
    let mut dirty_rows = 0u128;
    let mut dirty_cols = 0u128;
    for (idx, (&cur, &prev)) in board_snapshot.board_tiles[..area]
        .iter()
        .zip(working_buffer.prev_board_tiles[..area].iter())
        .enumerate()
    {
        if cur != prev {
            let row = idx / dim.cols as usize;
            let col = idx % dim.cols as usize;
            dirty_rows |= 1 << row;
            dirty_cols |= 1 << col;
        }
    }
    for col in 0..dim.cols {
        if dirty_cols & (1 << col) != 0 {
            let strip_range_start = (col as isize * dim.rows as isize) as usize;
            let strip_range_end = strip_range_start + dim.rows as usize;
            gen_cross_set(
                board_snapshot,
                &working_buffer.transposed_board_tiles[strip_range_start..strip_range_end],
                &mut working_buffer.cross_set_for_across_plays,
                dim.down(col),
                &mut working_buffer.cross_set_buffer_for_across_plays
                    [strip_range_start..strip_range_end],
                &mut working_buffer.cached_cross_set_for_across_plays
                    [strip_range_start..strip_range_end],
                &mut working_buffer.used_letters_tally,
            );
        }
    }
    let transposed_dim = matrix::Dim {
        rows: dim.cols,
        cols: dim.rows,
    };

    for row in 0..dim.rows {
        if dirty_rows & (1 << row) != 0 {
            let strip_range_start = (row as isize * dim.cols as isize) as usize;
            let strip_range_end = strip_range_start + dim.cols as usize;
            gen_cross_set(
                board_snapshot,
                &board_snapshot.board_tiles[strip_range_start..strip_range_end],
                &mut working_buffer.cross_set_for_down_plays,
                transposed_dim.down(row),
                &mut working_buffer.cross_set_buffer_for_down_plays
                    [strip_range_start..strip_range_end],
                &mut working_buffer.cached_cross_set_for_down_plays
                    [strip_range_start..strip_range_end],
                &mut working_buffer.used_letters_tally,
            );
        }
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

    if dirty_rows != 0 || dirty_cols != 0 {
        working_buffer.prev_board_tiles[..area]
            .copy_from_slice(&board_snapshot.board_tiles[..area]);
    }
    working_buffer.init_after_cross_sets(board_snapshot, dirty_cols, dirty_rows);

    let feasible_lengths = feasible_word_lengths::<true>(
        board_snapshot.anagrams,
        board_snapshot.rack_lengths,
        &working_buffer.rack_tally,
    );
    let mut found_placements = std::mem::take(&mut working_buffer.found_placements);
    found_placements.clear();
    let mut placement_order = std::mem::take(&mut working_buffer.placement_order);
    placement_order.clear();
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
                feasible_lengths,
                descending_scores: &working_buffer.descending_scores,
                lane_scaffold: &mut working_buffer.lane_scaffold[row as usize],
                best_leave_values: &working_buffer.best_leave_values,
                num_max_played,
                rack_tally_shadowl: &mut working_buffer.rack_tally_shadowl,
                rack_tally_shadowr: &mut working_buffer.rack_tally_shadowr,
                span_out: &mut working_buffer.span_out,
                per_span: true,
            },
            true,
            want_raw,
            |anchor: i8, leftmost: i8, rightmost: i8, best_possible_equity: i32, num_played: u8| {
                found_placements.push(PossiblePlacement {
                    num_played,
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
                feasible_lengths,
                descending_scores: &working_buffer.descending_scores,
                lane_scaffold: &mut working_buffer.lane_scaffold[dim.rows as usize + col as usize],
                best_leave_values: &working_buffer.best_leave_values,
                num_max_played,
                rack_tally_shadowl: &mut working_buffer.rack_tally_shadowl,
                rack_tally_shadowr: &mut working_buffer.rack_tally_shadowr,
                span_out: &mut working_buffer.span_out,
                per_span: true,
            },
            false,
            want_raw,
            |anchor: i8, leftmost: i8, rightmost: i8, best_possible_equity: i32, num_played: u8| {
                found_placements.push(PossiblePlacement {
                    num_played,
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
    placement_order.extend(
        found_placements
            .iter()
            .enumerate()
            .map(|(i, p)| (p.best_possible_equity, i as u32)),
    );
    if !want_raw {
        placement_order.sort_unstable_by_key(|&(equity, _)| equity);
    }
    working_buffer.found_placements = found_placements;
    working_buffer.placement_order = placement_order;
    std::iter::from_fn(move || match working_buffer.placement_order.pop() {
        Some((equity, idx)) => {
            if can_accept(equity) {
                let placement = working_buffer.found_placements[idx as usize];
                gen_place_moves_at(GenPlaceMovesAtParams {
                    board_snapshot,
                    working_buffer,
                    multi_leaves,
                    placement: &placement,
                    num_max_played,
                    threshold: current_threshold(),
                    found_place_move:
                        &mut |down: bool,
                              lane: i8,
                              idx: i8,
                              word: &[u8],
                              score: i32,
                              leave_value: i32| {
                            let this_best = score + leave_value;
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
                });
                Some(())
            } else {
                // fuse the iterator
                working_buffer.placement_order.clear();
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

#[inline]
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

    #[inline(always)]
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

        let mut turnaround_p = p;
        loop {
            node = env.params.kwg[turnaround_p];
            if node.tile() == 0 {
                // assume idx < env.anchor, because tile 0 does not occur at start in well-formed kwg gaddawg
                env.idx_left = idx + 1;
                play_right(env, turnaround_p, env.anchor + 1);
                if node.is_end() && turnaround_p == p {
                    return;
                }
                break;
            }
            if node.is_end() {
                break;
            }
            turnaround_p += 1;
        }

        if idx >= 0 {
            loop {
                let node = env.params.kwg[p];
                let tile = node.tile();
                if tile != 0 {
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

#[inline]
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
#[inline]
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{alphabet, bites, board_layout, build, display, game_config, klv, kwg};

    static TEST_WORDS: &[&str] = &[
        "AS", "AT", "EAST", "EAT", "EATS", "ETA", "ETAS", "SAT", "SEA", "SEAT", "SEATS", "SET",
        "TA", "TAE", "TAS", "TEA", "TEAS",
    ];

    static SWEEP_WORDS: &[&str] = &[
        "AN", "AND", "ANT", "ANTS", "ARE", "ART", "ARTS", "AS", "AT", "ATE", "CAN", "CANE",
        "CANES", "CANS", "CAR", "CARE", "CARES", "CARS", "CART", "CARTS", "CASE", "CASES", "CAST",
        "CASTE", "CAT", "CATS", "CENT", "CENTS", "CRATE", "CRATES", "EAR", "EARN", "EARNS", "EARS",
        "EAST", "EAT", "EATS", "ERA", "ERAS", "NEAR", "NEARS", "NEAT", "NEST", "NET", "NETS",
        "RACE", "RACES", "RAN", "RANT", "RANTS", "RAT", "RATE", "RATES", "RATS", "REACT", "REACTS",
        "SAT", "SCAN", "SCANT", "SCAR", "SCARE", "SEA", "SEAT", "SEATS", "SENT", "SET", "STAR",
        "STARE", "START", "STERN", "TA", "TAN", "TANS", "TAR", "TARE", "TARS", "TART", "TARTS",
        "TEA", "TEAR", "TEARS", "TEAS", "TEN", "TENS", "TENT", "TENTS", "TRACE", "TRACES",
        "TRANCE", "TRANCES", "TREAT", "TREATS",
    ];

    #[inline]
    fn sweep_kwg(gc: &game_config::GameConfig) -> kwg::Kwg<kwg::Node22> {
        let reader = alphabet::AlphabetReader::new_for_words(gc.alphabet());
        let mut word_buf = Vec::new();
        let mut words = Vec::<bites::Bites>::with_capacity(SWEEP_WORDS.len());
        for w in SWEEP_WORDS {
            reader.set_word(w, &mut word_buf).unwrap();
            words.push(word_buf[..].into());
        }
        words.sort_unstable();
        words.dedup();
        kwg::Kwg::<kwg::Node22>::from_bytes_alloc(
            &build::build(
                build::BuildContent::Gaddawg,
                build::BuildLayout::Wolges,
                build::BuildOrder::Sorted,
                &words,
            )
            .unwrap(),
        )
    }

    #[inline]
    fn sweep_kwg_covering(gc: &game_config::GameConfig) -> kwg::Kwg<kwg::Node22> {
        let dim = gc.board_layout().dim();
        let longest = dim.rows.max(dim.cols) as u8;
        let reader = alphabet::AlphabetReader::new_for_words(gc.alphabet());
        let mut word_buf = Vec::new();
        let mut words = Vec::<bites::Bites>::with_capacity(SWEEP_WORDS.len() + 1);
        for w in SWEEP_WORDS {
            reader.set_word(w, &mut word_buf).unwrap();
            words.push(word_buf[..].into());
        }
        let alphabet = gc.alphabet();
        let mut blanks_left = alphabet.freq(0);
        let mut spanning = Vec::with_capacity(longest as usize);
        let mut used = vec![0u8; alphabet.len() as usize];
        while spanning.len() < longest as usize {
            let before = spanning.len();
            for tile in 1..alphabet.len() {
                if spanning.len() >= longest as usize {
                    break;
                }
                if used[tile as usize] < alphabet.freq(tile) {
                    used[tile as usize] += 1;
                    spanning.push(tile);
                } else if blanks_left > 0 {
                    blanks_left -= 1;
                    used[tile as usize] += 1;
                    spanning.push(tile);
                }
            }
            assert!(
                spanning.len() > before,
                "this alphabet cannot spell a word as long as the line",
            );
        }
        words.push(spanning[..].into());
        words.sort_unstable();
        words.dedup();
        kwg::Kwg::<kwg::Node22>::from_bytes_alloc(
            &build::build(
                build::BuildContent::Gaddawg,
                build::BuildLayout::Wolges,
                build::BuildOrder::Sorted,
                &words,
            )
            .unwrap(),
        )
    }

    #[inline]
    fn sweep_output() -> String {
        let gc = game_config::make_english_game_config();
        let kwg = sweep_kwg(&gc);
        let klv = klv::Klv::<kwg::Node22>::from_bytes_alloc(klv::EMPTY_KLV_BYTES);
        let mut fen_parser = display::BoardFenParser::new(gc.alphabet(), gc.board_layout());
        let empty = "15/15/15/15/15/15/15/15/15/15/15/15/15/15/15";
        let cases: &[(&str, &str, usize)] = &[
            (empty, "AEINRST", 15),
            (empty, "?SATIRE", 10),
            (empty, "CARTONS", 10),
            (
                "15/15/15/15/15/15/15/6CARE5/15/15/15/15/15/15/15",
                "STARTED",
                15,
            ),
            (
                "15/15/15/15/15/15/15/6CARE5/15/15/15/15/15/15/15",
                "?ANTS??",
                8,
            ),
            (
                "15/15/15/15/15/15/15/4TRANCE5/15/15/15/15/15/15/15",
                "SEATERS",
                12,
            ),
            (
                "15/15/15/15/15/15/15/4TRANCE5/15/6NEST5/15/15/15/15/15",
                "RATTANS",
                12,
            ),
            (
                "15/15/15/15/15/15/15/4TRANCE5/15/6NEST5/15/15/15/15/15",
                "AAAAAAA",
                6,
            ),
        ];
        let mut out = String::new();
        for (fen, rack, max_gen) in cases {
            let board_tiles = fen_parser.parse(fen).unwrap().to_vec();
            let board_snapshot = BoardSnapshot {
                board_tiles: &board_tiles,
                game_config: &gc,
                kwg: &kwg,
                anagrams: None,
                rack_lengths: None,
                klv: &klv,
            };
            let mut move_generator = KurniaMoveGenerator::new(&gc);
            move_generator.gen_moves_unfiltered(&GenMovesParams {
                board_snapshot: &board_snapshot,
                rack: &parse_test_rack(gc.alphabet(), rack),
                max_gen: *max_gen,
                num_exchanges_by_this_player: 0,
                pass_policy: PassPolicy::OnlyWhenForced,
                dynamic_leaves: None,
            });
            out.push_str(&format!("== {fen} {rack} {max_gen}\n"));
            for p in &move_generator.plays {
                out.push_str(&format!(
                    "{} {}\n",
                    p.equity.raw(),
                    p.play.fmt(&board_snapshot)
                ));
            }
        }
        out
    }

    #[test]
    #[inline]
    fn the_generator_still_generates_what_it_generated() {
        assert_eq!(
            sweep_output(),
            include_str!("movegen-sweep-baseline.txt"),
            "generation moved; read the diff before refreshing the baseline"
        );
    }

    #[test]
    #[ignore]
    #[inline]
    fn write_sweep_baseline() {
        std::fs::write("src/movegen-sweep-baseline.txt", sweep_output()).unwrap();
    }

    #[inline]
    fn both_arms_agree(gc: &game_config::GameConfig, boards: &[&str], racks: &[&str]) {
        let kwg = sweep_kwg_covering(gc);
        let dim = gc.board_layout().dim();
        let layout = alphagram::KeyLayout::of(gc.alphabet(), dim.rows.max(dim.cols) as u8).unwrap();
        let held = anagrams::Anagrams::build(&kwg, layout.clone()).unwrap();
        let rack_lengths = anagrams::RackLengths::build(&kwg, layout, gc.rack_size() as usize + 1);
        let klv = klv::Klv::<kwg::Node22>::from_bytes_alloc(klv::EMPTY_KLV_BYTES);
        let mut fen_parser = display::BoardFenParser::new(gc.alphabet(), gc.board_layout());
        for max_gen in [1usize, 5, 100_000] {
            for fen in boards {
                let board_tiles = fen_parser.parse(fen).unwrap().to_vec();
                for rack in racks {
                    let rack = parse_test_rack(gc.alphabet(), rack);
                    let mut walked = String::new();
                    let mut fetched = String::new();
                    for (anagrams, out) in [(None, &mut walked), (Some(&held), &mut fetched)] {
                        let board_snapshot = BoardSnapshot {
                            board_tiles: &board_tiles,
                            game_config: gc,
                            kwg: &kwg,
                            anagrams,
                            rack_lengths: anagrams.is_none().then_some(&rack_lengths),
                            klv: &klv,
                        };
                        let mut move_generator = KurniaMoveGenerator::new(gc);
                        move_generator.gen_moves_unfiltered(&GenMovesParams {
                            board_snapshot: &board_snapshot,
                            rack: &rack,
                            max_gen,
                            num_exchanges_by_this_player: 0,
                            pass_policy: PassPolicy::OnlyWhenForced,
                            dynamic_leaves: None,
                        });
                        for p in &move_generator.plays {
                            out.push_str(&format!(
                                "{} {}\n",
                                p.equity.raw(),
                                p.play.fmt(&board_snapshot)
                            ));
                        }
                    }
                    assert_eq!(walked, fetched, "{fen} at a cap of {max_gen}");
                    assert!(!walked.is_empty(), "{fen} generated nothing at all");
                }
            }
        }
    }

    static SWEEP_RACKS: &[&str] = &[
        "AEINRST", "CARTONS", "STARTED", "SEATERS", "RATTANS", "AAAAAAA", "AE", "ST", "CAT",
        "NNNNTTT", "ERASECS", "?EINRST", "?AT", "C?T", "?ANTES", "?", "?A", "??TANS", "?ANTS??",
    ];

    #[test]
    #[inline]
    fn the_tables_find_exactly_what_the_descent_finds() {
        let gc = game_config::make_english_game_config();
        let empty = "15/15/15/15/15/15/15/15/15/15/15/15/15/15/15";
        let one = "15/15/15/15/15/15/15/6CARE5/15/15/15/15/15/15/15";
        let two = "15/15/15/15/15/15/15/6CARE5/6A8/6N8/6E8/15/15/15/15";
        let three = "15/15/15/15/15/15/15/6CARE5/6A8/6N8/6EATS5/15/15/15/15";
        let dead = "15/15/15/15/15/15/15/6N3A4/15/6N8/15/15/15/15/15";
        both_arms_agree(&gc, &[empty, one, two, three, dead], SWEEP_RACKS);
    }

    #[test]
    #[inline]
    fn a_wide_board_is_read_the_same_on_every_extent() {
        let gc = game_config::make_super_english_game_config();
        let dim = gc.board_layout().dim();
        assert!(dim.cols > 15, "this board is not wider than the old guard");
        let empty = "21/21/21/21/21/21/21/21/21/21/21/21/21/21/21/21/21/21/21/21/21";
        let one = "21/21/21/21/21/21/21/21/21/21/8CARE9/21/21/21/21/21/21/21/21/21/21";
        let two = "21/21/21/21/21/21/21/21/21/21/8CARE9/8A12/8N12/8E12/21/21/21/21/21/21/21";
        both_arms_agree(&gc, &[empty, one, two], SWEEP_RACKS);
    }

    static LONG_RACKS: &[&str] = &[
        "AEINRSTCS",
        "CARTONSEA",
        "STARTEDNE",
        "RATTANSEC",
        "AAAAAAANN",
        "NNNNTTTSS",
        "?EINRSTCA",
        "?ANTESCAR",
        "??TANSCAR",
        "AE",
        "CAT",
        "?A",
    ];

    #[test]
    #[inline]
    fn a_longer_rack_is_read_the_same_way() {
        let gc = game_config::make_hong_kong_english_game_config();
        assert!(
            gc.rack_size() > 7,
            "this config does not deal a longer rack than the others",
        );
        let empty = "15/15/15/15/15/15/15/15/15/15/15/15/15/15/15";
        let one = "15/15/15/15/15/15/15/6CARE5/15/15/15/15/15/15/15";
        let two = "15/15/15/15/15/15/15/6CARE5/6A8/6N8/6E8/15/15/15/15";
        both_arms_agree(&gc, &[empty, one, two], LONG_RACKS);
    }

    #[inline]
    fn test_kwg(gc: &game_config::GameConfig) -> kwg::Kwg<kwg::Node22> {
        let reader = alphabet::AlphabetReader::new_for_words(gc.alphabet());
        let mut word_buf = Vec::new();
        let mut words = Vec::<bites::Bites>::with_capacity(TEST_WORDS.len());
        for w in TEST_WORDS {
            reader.set_word(w, &mut word_buf).unwrap();
            words.push(word_buf[..].into());
        }
        words.sort_unstable();
        kwg::Kwg::<kwg::Node22>::from_bytes_alloc(
            &build::build(
                build::BuildContent::Gaddawg,
                build::BuildLayout::Wolges,
                build::BuildOrder::Sorted,
                &words,
            )
            .unwrap(),
        )
    }

    #[inline]
    fn test_kad(gc: &game_config::GameConfig) -> kwg::Kwg<kwg::Node22> {
        let reader = alphabet::AlphabetReader::new_for_words(gc.alphabet());
        let mut word_buf = Vec::new();
        let mut words = Vec::<bites::Bites>::with_capacity(TEST_WORDS.len());
        for w in TEST_WORDS {
            reader.set_word(w, &mut word_buf).unwrap();
            word_buf.sort_unstable();
            words.push(word_buf[..].into());
        }
        words.sort_unstable();
        words.dedup();
        kwg::Kwg::<kwg::Node22>::from_bytes_alloc(
            &build::build(
                build::BuildContent::DawgOnly,
                build::BuildLayout::Wolges,
                build::BuildOrder::Sorted,
                &words,
            )
            .unwrap(),
        )
    }

    #[inline]
    fn parse_test_rack(alphabet: &alphabet::Alphabet, rack_str: &str) -> Vec<u8> {
        let reader = alphabet::AlphabetReader::new_for_racks(alphabet);
        let sb = rack_str.as_bytes();
        let mut rack = Vec::new();
        let mut ix = 0;
        while ix < sb.len() {
            let (tile, next_ix) = reader.next_tile(sb, ix).unwrap();
            rack.push(tile);
            ix = next_ix;
        }
        rack
    }

    #[inline]
    fn placements(fen: &str, rack: &str) -> Vec<String> {
        let gc = game_config::make_english_game_config();
        let kwg = test_kwg(&gc);
        let klv = klv::Klv::<kwg::Node22>::from_bytes_alloc(klv::EMPTY_KLV_BYTES);
        let mut fen_parser = display::BoardFenParser::new(gc.alphabet(), gc.board_layout());
        let board_tiles = fen_parser.parse(fen).unwrap().to_vec();
        let board_snapshot = BoardSnapshot {
            board_tiles: &board_tiles,
            game_config: &gc,
            kwg: &kwg,
            anagrams: None,
            rack_lengths: None,
            klv: &klv,
        };
        let mut move_generator = KurniaMoveGenerator::new(&gc);
        move_generator.gen_moves_unfiltered(&GenMovesParams {
            board_snapshot: &board_snapshot,
            rack: &parse_test_rack(gc.alphabet(), rack),
            max_gen: usize::MAX,
            num_exchanges_by_this_player: 0,
            pass_policy: PassPolicy::OnlyWhenForced,
            dynamic_leaves: None,
        });
        let mut out = move_generator
            .plays
            .iter()
            .filter(|p| matches!(p.play, Play::Place { .. }))
            .map(|p| format!("{}", p.play.fmt(&board_snapshot)))
            .collect::<Vec<_>>();
        out.sort_unstable();
        out
    }

    #[inline]
    fn valued_plays(fen: &str, rack: &str, max_gen: usize) -> Vec<String> {
        let gc = game_config::make_english_game_config();
        let kwg = test_kwg(&gc);
        let klv = klv::Klv::<kwg::Node22>::from_bytes_alloc(klv::EMPTY_KLV_BYTES);
        let mut fen_parser = display::BoardFenParser::new(gc.alphabet(), gc.board_layout());
        let board_tiles = fen_parser.parse(fen).unwrap().to_vec();
        let board_snapshot = BoardSnapshot {
            board_tiles: &board_tiles,
            game_config: &gc,
            kwg: &kwg,
            anagrams: None,
            rack_lengths: None,
            klv: &klv,
        };
        let mut move_generator = KurniaMoveGenerator::new(&gc);
        move_generator.gen_moves_unfiltered(&GenMovesParams {
            board_snapshot: &board_snapshot,
            rack: &parse_test_rack(gc.alphabet(), rack),
            max_gen,
            num_exchanges_by_this_player: 0,
            pass_policy: PassPolicy::OnlyWhenForced,
            dynamic_leaves: None,
        });
        move_generator
            .plays
            .iter()
            .map(|p| format!("{} {}", p.equity.raw(), p.play.fmt(&board_snapshot)))
            .collect::<Vec<_>>()
    }

    #[test]
    #[inline]
    fn a_capped_generation_keeps_the_best_of_a_tie() {
        let empty = "15/15/15/15/15/15/15/15/15/15/15/15/15/15/15";
        let all = valued_plays(empty, "AEST", usize::MAX);
        assert!(all.len() > 6);
        let top_equity = all[0].split(' ').next().unwrap();
        assert_eq!(
            all[3].split(' ').next().unwrap(),
            top_equity,
            "no tie to cut"
        );
        assert_ne!(all[4].split(' ').next().unwrap(), top_equity);
        for k in 1..=6 {
            assert_eq!(valued_plays(empty, "AEST", k), all[..k], "cap of {k}");
        }
    }

    #[inline]
    fn jumbled_hex_placements(words: &[&[u8]], rack: &[u8]) -> Vec<String> {
        let gc = game_config::make_jumbled_hex_game_config();
        let words = words
            .iter()
            .map(|w| (*w).into())
            .collect::<Vec<bites::Bites>>();
        let kwg = kwg::Kwg::<kwg::Node22>::from_bytes_alloc(
            &build::build(
                build::BuildContent::DawgOnly,
                build::BuildLayout::Wolges,
                build::BuildOrder::Sorted,
                &build::make_alphagrams(&words),
            )
            .unwrap(),
        );
        let klv = klv::Klv::<kwg::Node22>::from_bytes_alloc(klv::EMPTY_KLV_BYTES);
        let dim = gc.board_layout().dim();
        let board_tiles = vec![0u8; dim.rows as usize * dim.cols as usize];
        let board_snapshot = BoardSnapshot {
            board_tiles: &board_tiles,
            game_config: &gc,
            kwg: &kwg,
            anagrams: None,
            rack_lengths: None,
            klv: &klv,
        };
        let mut move_generator = KurniaMoveGenerator::new(&gc);
        move_generator.gen_moves_unfiltered(&GenMovesParams {
            board_snapshot: &board_snapshot,
            rack,
            max_gen: usize::MAX,
            num_exchanges_by_this_player: 0,
            pass_policy: PassPolicy::OnlyWhenForced,
            dynamic_leaves: None,
        });
        let mut out = move_generator
            .plays
            .iter()
            .filter(|p| matches!(p.play, Play::Place { .. }))
            .map(|p| format!("{}", p.play.fmt(&board_snapshot)))
            .collect::<Vec<_>>();
        out.sort_unstable();
        out
    }

    #[test]
    #[inline]
    fn reordered_kwg_generates_the_same_plays() {
        static WORDS: &[&str] = &[
            "AE", "AH", "AI", "AL", "AN", "AR", "AS", "AT", "EAR", "EAT", "ERA", "ETA", "HAE",
            "HAT", "HEAR", "HEART", "HEAT", "HEATER", "HER", "HERS", "LEA", "LEAN", "LEARN",
            "LEARNS", "LEAST", "NEAR", "NEAT", "RAT", "RATE", "REAL", "SEAT", "SHEAR", "STEAL",
            "TEA", "TEAL", "TEAR", "TEARS", "THE", "THEN", "THERE", "TREAT",
        ];
        let gc = game_config::make_english_game_config();
        let reader = alphabet::AlphabetReader::new_for_words(gc.alphabet());
        let mut word_buf = Vec::new();
        let words = WORDS
            .iter()
            .map(|w| {
                reader.set_word(w, &mut word_buf).unwrap();
                word_buf[..].into()
            })
            .collect::<Vec<bites::Bites>>();

        let mut plays_from = |build_order| {
            let kwg_bytes = build::build(
                build::BuildContent::Gaddawg,
                build::BuildLayout::Wolges,
                build_order,
                &words,
            )
            .unwrap();
            let kwg = kwg::Kwg::<kwg::Node22>::from_bytes_alloc(&kwg_bytes);
            let klv = klv::Klv::<kwg::Node22>::from_bytes_alloc(klv::EMPTY_KLV_BYTES);
            let mut board_tiles = vec![0u8; gc.board_layout().dim().rows as usize * 15];
            reader.set_word("HEAT", &mut word_buf).unwrap();
            for (i, &tile) in word_buf.iter().enumerate() {
                board_tiles[7 * 15 + 5 + i] = tile;
            }
            let board_snapshot = BoardSnapshot {
                board_tiles: &board_tiles,
                game_config: &gc,
                kwg: &kwg,
                anagrams: None,
                rack_lengths: None,
                klv: &klv,
            };
            let mut move_generator = KurniaMoveGenerator::new(&gc);
            reader.set_word("AERSTLN", &mut word_buf).unwrap();
            let mut rack = word_buf.clone();
            rack.sort_unstable();
            move_generator.gen_moves_unfiltered(&GenMovesParams {
                board_snapshot: &board_snapshot,
                rack: &rack,
                max_gen: usize::MAX,
                num_exchanges_by_this_player: 0,
                pass_policy: PassPolicy::OnlyWhenForced,
                dynamic_leaves: None,
            });
            move_generator
                .plays
                .iter()
                .map(|vm| (vm.equity, vm.play.clone()))
                .collect::<Vec<_>>()
        };

        let sort_key = |(equity, play): &(equity::Equity, Play)| match play {
            Play::Exchange { tiles } => (*equity, 0u8, 0i8, 0i8, tiles.to_vec(), 0i32),
            Play::Place {
                down,
                lane,
                idx,
                word,
                score,
            } => (*equity, 1 + *down as u8, *lane, *idx, word.to_vec(), *score),
        };
        let mut sorted = plays_from(build::BuildOrder::Sorted);
        let mut reordered = plays_from(build::BuildOrder::Reordered);
        assert!(sorted.len() > 100, "the test needs plays to compare");
        assert_eq!(sorted.len(), reordered.len());
        sorted.sort_by_key(sort_key);
        reordered.sort_by_key(sort_key);
        assert!(
            sorted
                .iter()
                .zip(reordered.iter())
                .all(|(a, b)| a.0 == b.0 && a.1 == b.1),
            "a reordered kwg generated different plays"
        );
    }

    #[test]
    #[inline]
    fn cross_set_score_cache_distinguishes_blank_from_natural_tile() {
        let gc = game_config::make_english_game_config();
        let alphabet = gc.alphabet();
        let reader = alphabet::AlphabetReader::new_for_words(alphabet);
        let mut word_buf = Vec::new();
        reader.set_word("AA", &mut word_buf).unwrap();
        let words: Vec<bites::Bites> = vec![word_buf[..].into()];
        let kwg_bytes = build::build(
            build::BuildContent::Gaddawg,
            build::BuildLayout::Wolges,
            build::BuildOrder::Sorted,
            &words,
        )
        .unwrap();
        let kwg = kwg::Kwg::<kwg::Node22>::from_bytes_alloc(&kwg_bytes);
        let klv = klv::Klv::<kwg::Node22>::from_bytes_alloc(klv::EMPTY_KLV_BYTES);
        let board_snapshot = BoardSnapshot {
            board_tiles: &[],
            game_config: &gc,
            kwg: &kwg,
            anagrams: None,
            rack_lengths: None,
            klv: &klv,
        };

        let natural_a = 1u8; // 'A' is letter index 1 in the English alphabet.
        let blank_a = natural_a | 0x80; // a blank tile played as 'A'.
        let natural_a_score = alphabet.scaled_score(natural_a);
        assert!(
            natural_a_score > 0,
            "the test needs a natural tile that actually scores points"
        );

        let len = gc.board_layout().dim().cols as usize;
        let output_strider = gc.board_layout().dim().across(0);
        let mut cross_sets = vec![CrossSet { bits: 0, score: 0 }; len];
        let mut cross_set_buffer = vec![
            CrossSetComputation {
                score: 0,
                b_letter: 0,
                end_range: 0,
                p: 0,
            };
            len
        ];
        let mut cached_cross_sets = vec![
            CachedCrossSet {
                p_left: 0,
                p_right: 0,
                bits: 0,
            };
            len
        ];

        let mut board_strip = vec![0u8; len];
        board_strip[2] = natural_a;
        gen_classic_cross_set(
            &board_snapshot,
            &board_strip,
            &mut cross_sets,
            output_strider.clone(),
            &mut cross_set_buffer,
            &mut cached_cross_sets,
        );
        assert_eq!(cross_sets[1].score, natural_a_score);
        assert_eq!(cross_sets[3].score, natural_a_score);

        board_strip[2] = blank_a;
        gen_classic_cross_set(
            &board_snapshot,
            &board_strip,
            &mut cross_sets,
            output_strider,
            &mut cross_set_buffer,
            &mut cached_cross_sets,
        );
        assert_eq!(cross_sets[1].score, 0);
        assert_eq!(cross_sets[3].score, 0);
    }
    #[inline]
    fn word_of(formatted: &str) -> String {
        formatted
            .split_whitespace()
            .nth(1)
            .unwrap()
            .replace(['(', ')'], "")
            .to_uppercase()
    }

    #[test]
    #[inline]
    fn movegen_opens_through_the_star_with_dictionary_words_only() {
        let plays = placements("15/15/15/15/15/15/15/15/15/15/15/15/15/15/15", "AEST");
        for p in &plays {
            let w = word_of(p);
            assert!(TEST_WORDS.contains(&&w[..]), "{p} spells {w}, not a word");
            let coord = p.split_whitespace().next().unwrap();
            assert!(coord.starts_with('8'), "{p} is not on the star row");
            let col = coord.as_bytes()[1];
            assert!(col <= b'H', "{p} starts past the star");
            assert!(
                col as usize + w.len() > b'H' as usize,
                "{p} stops before the star"
            );
        }
        assert!(plays.contains(&"8E SEAT 8".to_string()));
        assert_eq!(plays.len(), 50);
    }

    #[test]
    #[inline]
    fn movegen_hooks_onto_a_word_on_the_board() {
        assert_eq!(
            placements("15/15/15/15/15/15/15/6SEAT5/15/15/15/15/15/15/15", "S"),
            ["8G (SEAT)S 5", "I8 (A)S 3"]
        );
    }

    #[test]
    #[inline]
    fn movegen_scores_a_blank_as_zero() {
        let plays = placements("15/15/15/15/15/15/15/6SEAT5/15/15/15/15/15/15/15", "?");
        assert_eq!(
            plays,
            [
                "8G (SEAT)s 4",
                "G7 a(S) 1",
                "I7 t(A) 1",
                "I8 (A)s 1",
                "I8 (A)t 1",
                "J7 a(T) 1",
                "J8 (T)a 1",
            ]
        );
    }

    #[inline]
    fn sample_plays() -> Vec<Play> {
        let word: bites::Bites = [1u8, 2, 3][..].into();
        let other_word: bites::Bites = [1u8, 2, 4][..].into();
        let place = |down, lane, idx, w: &bites::Bites, score| Play::Place {
            down,
            lane,
            idx,
            word: w.clone(),
            score,
        };
        vec![
            place(false, 7, 7, &word, 24),
            place(false, 7, 7, &word, 26),
            place(false, 7, 7, &other_word, 24),
            place(false, 7, 8, &word, 24),
            place(false, 8, 7, &word, 24),
            place(true, 7, 7, &word, 24),
            Play::Exchange {
                tiles: [][..].into(),
            },
            Play::Exchange {
                tiles: [1u8, 1][..].into(),
            },
            Play::Exchange {
                tiles: [1u8, 2][..].into(),
            },
        ]
    }

    #[test]
    #[inline]
    fn valued_move_order_is_total() {
        let plays = sample_plays();
        let moves: Vec<ValuedMove> = plays
            .iter()
            .flat_map(|p| {
                [10, 20].into_iter().map(move |e| ValuedMove {
                    equity: equity::Equity::new(e),
                    play: p.clone(),
                })
            })
            .collect();

        for a in moves.iter() {
            assert_eq!(a.cmp(a), std::cmp::Ordering::Equal, "not reflexive");
            for b in moves.iter() {
                assert_eq!(
                    a.cmp(b),
                    b.cmp(a).reverse(),
                    "not antisymmetric: {:?} {:?}",
                    a.equity.raw(),
                    b.equity.raw()
                );
                assert_eq!(
                    a.cmp(b) == std::cmp::Ordering::Equal,
                    a.equity == b.equity && a.play == b.play,
                    "Equal disagrees with equality"
                );
                assert_eq!(a == b, a.cmp(b) == std::cmp::Ordering::Equal);
                for c in moves.iter() {
                    if a.cmp(b) != std::cmp::Ordering::Greater
                        && b.cmp(c) != std::cmp::Ordering::Greater
                    {
                        assert_ne!(a.cmp(c), std::cmp::Ordering::Greater, "not transitive");
                    }
                }
            }
        }
    }

    #[test]
    #[inline]
    fn heap_drains_ties_the_same_whatever_order_they_arrive() {
        let plays = sample_plays();
        let build = |order: &[usize]| -> Vec<Play> {
            let mut heap = std::collections::BinaryHeap::new();
            for &i in order {
                heap.push(ValuedMove {
                    equity: equity::Equity::new(1234),
                    play: plays[i].clone(),
                });
            }
            heap.into_sorted_vec().into_iter().map(|m| m.play).collect()
        };

        let forward: Vec<usize> = (0..plays.len()).collect();
        let backward: Vec<usize> = (0..plays.len()).rev().collect();
        let mut shuffled = forward.clone();
        shuffled.swap(0, 4);
        shuffled.swap(1, 7);
        shuffled.swap(2, 5);

        let a = build(&forward);
        let b = build(&backward);
        let c = build(&shuffled);
        assert_eq!(a.len(), plays.len());
        assert!(a == b && b == c, "arrival order changed the drained order");

        let expect = [
            plays[0].clone(),
            plays[1].clone(),
            plays[2].clone(),
            plays[3].clone(),
            plays[4].clone(),
            plays[5].clone(),
            plays[6].clone(),
            plays[7].clone(),
            plays[8].clone(),
        ];
        assert!(
            a == expect,
            "the tie-break did not order the plays as documented"
        );
    }

    #[test]
    #[inline]
    fn live_pool_subtracts_board_and_rack_and_returns_blanks() {
        let gc = game_config::make_english_game_config();
        let alphabet = gc.alphabet();
        let n = alphabet.len() as usize;
        let natural_a = 1u8;
        let blank_a = natural_a | 0x80;

        let board_tiles = [natural_a, blank_a, 0u8, 0u8];
        let mut rack_tally = vec![0u8; n];
        rack_tally[natural_a as usize] = 2;

        let mut pool = [0u8; MAX_ALPHABET_LEN];
        live_pool_into(&mut pool[..n], alphabet, &board_tiles, &rack_tally);

        assert_eq!(
            pool[natural_a as usize],
            alphabet.freq(natural_a) - 3,
            "one A on the board and two on the rack come out of the A pool"
        );
        assert_eq!(
            pool[0], // the blank's own index
            alphabet.freq(0) - 1,
            "a blank played as A comes out of the blank pool, not the A pool"
        );

        for (tile, &count) in pool.iter().enumerate().take(n).skip(2) {
            assert_eq!(count, alphabet.freq(tile as u8), "tile {tile}");
        }

        let mut greedy_rack = vec![0u8; n];
        greedy_rack[natural_a as usize] = alphabet.freq(natural_a) + 5;
        live_pool_into(&mut pool[..n], alphabet, &[], &greedy_rack);
        assert_eq!(pool[natural_a as usize], 0);
    }
    #[inline]
    fn jumbled_placements_once(fen: &str, rack: &str, is_census: bool) -> Vec<String> {
        let gc = game_config::make_jumbled_english_game_config();
        let kwg = test_kad(&gc);
        let klv = klv::Klv::<kwg::Node22>::from_bytes_alloc(klv::EMPTY_KLV_BYTES);
        let mut fen_parser = display::BoardFenParser::new(gc.alphabet(), gc.board_layout());
        let board_tiles = fen_parser.parse(fen).unwrap().to_vec();
        let board_snapshot = BoardSnapshot {
            board_tiles: &board_tiles,
            game_config: &gc,
            kwg: &kwg,
            anagrams: None,
            rack_lengths: None,
            klv: &klv,
        };
        let mut move_generator = KurniaMoveGenerator::new(&gc);
        move_generator.working_buffer.is_census = is_census;
        move_generator.gen_moves_unfiltered(&GenMovesParams {
            board_snapshot: &board_snapshot,
            rack: &parse_test_rack(gc.alphabet(), rack),
            max_gen: usize::MAX,
            num_exchanges_by_this_player: 0,
            pass_policy: PassPolicy::OnlyWhenForced,
            dynamic_leaves: None,
        });
        let mut out = move_generator
            .plays
            .iter()
            .filter(|p| matches!(p.play, Play::Place { .. }))
            .map(|p| format!("{}", p.play.fmt(&board_snapshot)))
            .collect::<Vec<_>>();
        out.sort_unstable();
        out
    }

    #[inline(always)]
    fn jumbled_placements(fen: &str, rack: &str) -> Vec<String> {
        jumbled_placements_once(fen, rack, false)
    }

    #[test]
    #[inline]
    fn jumbled_movegen_opens_with_either_arrangement() {
        assert_eq!(
            jumbled_placements("15/15/15/15/15/15/15/15/15/15/15/15/15/15/15", "AT"),
            ["8G AT 4", "8G TA 4", "8H AT 4", "8H TA 4"]
        );
    }

    #[test]
    #[inline]
    fn jumbled_movegen_opens_with_a_blank() {
        assert_eq!(
            jumbled_placements("15/15/15/15/15/15/15/15/15/15/15/15/15/15/15", "?A"),
            [
                "8G As 2", "8G At 2", "8G sA 2", "8G tA 2", "8H As 2", "8H At 2", "8H sA 2",
                "8H tA 2",
            ]
        );
    }

    #[test]
    #[inline]
    fn jumbled_movegen_takes_any_arrangement_of_a_word() {
        assert_eq!(
            jumbled_placements("15/15/15/15/15/15/15/6SEAT5/15/15/15/15/15/15/15", "S"),
            ["8F S(SEAT) 5", "8G (SEAT)S 5", "I7 S(A) 3", "I8 (A)S 3"]
        );
    }

    #[test]
    #[inline]
    fn jumbled_movegen_scores_a_blank_as_zero() {
        assert_eq!(
            jumbled_placements("15/15/15/15/15/15/15/6SEAT5/15/15/15/15/15/15/15", "?"),
            [
                "8F s(SEAT) 4",
                "8G (SEAT)s 4",
                "G7 a(S) 1",
                "G8 (S)a 1",
                "I7 s(A) 1",
                "I7 t(A) 1",
                "I8 (A)s 1",
                "I8 (A)t 1",
                "J7 a(T) 1",
                "J8 (T)a 1",
            ]
        );
    }

    #[test]
    #[inline]
    fn jumbled_movegen_handles_the_widest_alphabet() {
        let words: &[&[u8]] = &[&[1, 63], &[1, 2, 63]];
        assert_eq!(
            jumbled_hex_placements(words, &[1, 63]),
            ["8G 013f 0", "8G 3f01 0", "8H 013f 0", "8H 3f01 0"]
        );
        let plays = jumbled_hex_placements(words, &[1, 2, 63]);
        assert_eq!(plays.len(), 3 * 6 + 2 * 2);
        assert!(plays.iter().all(|p| p.contains("3f")), "{plays:?}");
    }

    #[test]
    #[inline]
    fn jumbled_movegen_spell_once_keeps_the_real_tile() {
        let fen = "15/15/15/15/15/15/15/6SEAT5/15/15/15/15/15/15/15";
        let every_way = jumbled_placements_once(fen, "?S", false);
        let once = jumbled_placements_once(fen, "?S", true);
        assert!(
            every_way.iter().any(|p| p.contains('s')),
            "nothing spelled an S with the blank, so the test proves nothing"
        );
        assert!(
            !once.iter().any(|p| p.contains('s')),
            "is_census still spent the blank on an S: {once:?}"
        );
        assert!(once.iter().all(|p| every_way.contains(p)));
        let dropped = every_way
            .iter()
            .filter(|p| !once.contains(p))
            .collect::<Vec<_>>();
        assert!(!dropped.is_empty());
        assert!(dropped.iter().all(|p| p.contains('s')), "{dropped:?}");
        assert!(once.iter().any(|p| p.contains('t')));
    }

    #[test]
    #[inline]
    fn a_generation_after_remaining_words_matches_a_fresh_one() {
        let gc = game_config::make_english_game_config();
        let kwg = test_kwg(&gc);
        let klv = klv::Klv::<kwg::Node22>::from_bytes_alloc(klv::EMPTY_KLV_BYTES);
        let mut fen_parser = display::BoardFenParser::new(gc.alphabet(), gc.board_layout());
        let seat = fen_parser
            .parse("15/15/15/15/15/15/15/6SEAT5/15/15/15/15/15/15/15")
            .unwrap()
            .to_vec();
        let empty = fen_parser
            .parse("15/15/15/15/15/15/15/15/15/15/15/15/15/15/15")
            .unwrap()
            .to_vec();
        let rack = parse_test_rack(gc.alphabet(), "AEST");
        let seat_snapshot = BoardSnapshot {
            board_tiles: &seat,
            game_config: &gc,
            kwg: &kwg,
            anagrams: None,
            rack_lengths: None,
            klv: &klv,
        };
        let empty_snapshot = BoardSnapshot {
            board_tiles: &empty,
            game_config: &gc,
            kwg: &kwg,
            anagrams: None,
            rack_lengths: None,
            klv: &klv,
        };
        let plays = |move_generator: &mut KurniaMoveGenerator| {
            move_generator.gen_moves_unfiltered(&GenMovesParams {
                board_snapshot: &seat_snapshot,
                rack: &rack,
                max_gen: usize::MAX,
                num_exchanges_by_this_player: 0,
                pass_policy: PassPolicy::OnlyWhenForced,
                dynamic_leaves: None,
            });
            move_generator
                .plays
                .iter()
                .map(|p| format!("{} {}", p.equity.raw(), p.play.fmt(&seat_snapshot)))
                .collect::<Vec<_>>()
        };
        let fresh = plays(&mut KurniaMoveGenerator::new(&gc));
        let mut move_generator = KurniaMoveGenerator::new(&gc);
        assert_eq!(plays(&mut move_generator), fresh);
        move_generator.gen_remaining_words(&empty_snapshot, |_| {});
        assert_eq!(plays(&mut move_generator), fresh);
    }

    #[test]
    #[inline]
    fn a_generation_after_an_empty_board_matches_a_fresh_one() {
        let gc = game_config::make_english_game_config();
        let kwg = test_kwg(&gc);
        let klv = klv::Klv::<kwg::Node22>::from_bytes_alloc(klv::EMPTY_KLV_BYTES);
        let mut fen_parser = display::BoardFenParser::new(gc.alphabet(), gc.board_layout());
        let empty = fen_parser
            .parse("15/15/15/15/15/15/15/15/15/15/15/15/15/15/15")
            .unwrap()
            .to_vec();
        let off_the_star = fen_parser
            .parse("15/15/15/15/15/15/15/15/15/15/15/SEAT11/15/15/15")
            .unwrap()
            .to_vec();
        let rack = parse_test_rack(gc.alphabet(), "AEST");
        let plays = |move_generator: &mut KurniaMoveGenerator, board_tiles: &[u8]| {
            let board_snapshot = BoardSnapshot {
                board_tiles,
                game_config: &gc,
                kwg: &kwg,
                anagrams: None,
                rack_lengths: None,
                klv: &klv,
            };
            move_generator.gen_moves_unfiltered(&GenMovesParams {
                board_snapshot: &board_snapshot,
                rack: &rack,
                max_gen: usize::MAX,
                num_exchanges_by_this_player: 0,
                pass_policy: PassPolicy::OnlyWhenForced,
                dynamic_leaves: None,
            });
            move_generator
                .plays
                .iter()
                .map(|p| format!("{} {}", p.equity.raw(), p.play.fmt(&board_snapshot)))
                .collect::<Vec<_>>()
        };
        let mut move_generator = KurniaMoveGenerator::new(&gc);
        for board_tiles in [&empty, &off_the_star, &empty, &empty, &off_the_star] {
            assert_eq!(
                plays(&mut move_generator, board_tiles),
                plays(&mut KurniaMoveGenerator::new(&gc), board_tiles)
            );
        }
    }

    #[inline]
    fn uneven_premium(row: i8, col: i8) -> board_layout::Premium {
        let (row, col) = (row as i32, col as i32);
        let (word_multiplier, tile_multiplier) = if (row * 7 + col * 3) % 19 == 0 {
            (3, 1)
        } else if (row * 5 + col * 11) % 13 == 0 {
            (2, 1)
        } else if (row + col * 2) % 7 == 0 {
            (1, 3)
        } else if (row * 3 + col) % 5 == 0 {
            (1, 2)
        } else {
            (1, 1)
        };
        board_layout::Premium {
            word_multiplier,
            tile_multiplier,
        }
    }

    #[inline]
    fn uneven_game_config(
        rows: i8,
        cols: i8,
        star_row: i8,
        star_col: i8,
        transposed: bool,
    ) -> game_config::GameConfig {
        let mut premiums = Vec::with_capacity((rows as isize * cols as isize) as usize);
        for row in 0..rows {
            for col in 0..cols {
                premiums.push(if transposed {
                    uneven_premium(col, row)
                } else {
                    uneven_premium(row, col)
                });
            }
        }
        game_config::make_board_test_game_config(board_layout::make_test_board_layout(
            premiums.into_boxed_slice(),
            matrix::Dim { rows, cols },
            star_row,
            star_col,
        ))
    }

    #[inline]
    fn place_plays(
        move_generator: &mut KurniaMoveGenerator,
        board_snapshot: &BoardSnapshot<'_, kwg::Node22, kwg::Node22>,
        rack: &[u8],
        transposed: bool,
    ) -> Vec<(bool, i8, i8, Vec<u8>, i32)> {
        move_generator.gen_moves_unfiltered(&GenMovesParams {
            board_snapshot,
            rack,
            max_gen: usize::MAX,
            num_exchanges_by_this_player: 0,
            pass_policy: PassPolicy::OnlyWhenForced,
            dynamic_leaves: None,
        });
        let mut out = move_generator
            .plays
            .iter()
            .filter_map(|p| match &p.play {
                Play::Place {
                    down,
                    lane,
                    idx,
                    word,
                    score,
                } => Some((*down != transposed, *lane, *idx, word.to_vec(), *score)),
                Play::Exchange { .. } => None,
            })
            .collect::<Vec<_>>();
        out.sort_unstable();
        out
    }

    #[inline]
    fn a_reused_generator_agrees_on_an_uneven_board(
        rows: i8,
        cols: i8,
        star_row: i8,
        star_col: i8,
        tiles: &[(i8, i8)],
    ) {
        let gc = uneven_game_config(rows, cols, star_row, star_col, false);
        let transposed_gc = uneven_game_config(cols, rows, star_col, star_row, true);
        let kwg = test_kwg(&gc);
        let layout = alphagram::KeyLayout::of(gc.alphabet(), rows.max(cols) as u8).unwrap();
        let held = anagrams::Anagrams::build(&kwg, layout).unwrap();
        let klv = klv::Klv::<kwg::Node22>::from_bytes_alloc(klv::EMPTY_KLV_BYTES);
        let rack = parse_test_rack(gc.alphabet(), "AEST");
        let area = (rows as isize * cols as isize) as usize;
        let mut walked = Vec::new();
        for anagrams in [None, Some(&held)] {
            let mut board_tiles = vec![0u8; area];
            let mut transposed_board_tiles = vec![0u8; area];
            let mut move_generator = KurniaMoveGenerator::new(&gc);
            let mut transposed_move_generator = KurniaMoveGenerator::new(&transposed_gc);
            let mut lanes_played = [vec![false; rows as usize], vec![false; cols as usize]];
            for step in 0..tiles.len() + 2 {
                if step == tiles.len() + 1 {
                    board_tiles.fill(0);
                    transposed_board_tiles.fill(0);
                } else if step > 0 {
                    let (row, col) = tiles[step - 1];
                    let tile = rack[(step - 1) % rack.len()];
                    board_tiles[gc.board_layout().dim().at_row_col(row, col)] = tile;
                    transposed_board_tiles
                        [transposed_gc.board_layout().dim().at_row_col(col, row)] = tile;
                }
                let board_snapshot = BoardSnapshot {
                    board_tiles: &board_tiles,
                    game_config: &gc,
                    kwg: &kwg,
                    anagrams,
                    rack_lengths: None,
                    klv: &klv,
                };
                let transposed_board_snapshot = BoardSnapshot {
                    board_tiles: &transposed_board_tiles,
                    game_config: &transposed_gc,
                    kwg: &kwg,
                    anagrams,
                    rack_lengths: None,
                    klv: &klv,
                };
                let fresh = place_plays(
                    &mut KurniaMoveGenerator::new(&gc),
                    &board_snapshot,
                    &rack,
                    false,
                );
                assert!(!fresh.is_empty(), "nothing to play at step {step}");
                if anagrams.is_none() {
                    walked.push(fresh.clone());
                } else {
                    assert_eq!(fresh, walked[step], "fetched at step {step}");
                }
                assert_eq!(
                    place_plays(&mut move_generator, &board_snapshot, &rack, false),
                    fresh,
                    "reused at step {step}"
                );
                assert_eq!(
                    place_plays(
                        &mut KurniaMoveGenerator::new(&transposed_gc),
                        &transposed_board_snapshot,
                        &rack,
                        true
                    ),
                    fresh,
                    "transposed at step {step}"
                );
                assert_eq!(
                    place_plays(
                        &mut transposed_move_generator,
                        &transposed_board_snapshot,
                        &rack,
                        true
                    ),
                    fresh,
                    "transposed and reused at step {step}"
                );
                for (down, lane, ..) in &fresh {
                    lanes_played[*down as usize][*lane as usize] = true;
                }
            }
            assert!(
                lanes_played[0].iter().all(|&x| x),
                "a row never had a play across"
            );
            assert!(
                lanes_played[1].iter().all(|&x| x),
                "a column never had a play down"
            );
        }
    }

    #[test]
    #[inline]
    fn a_long_narrow_board_agrees_with_its_transpose() {
        let tiles = (0..100)
            .map(|row| (row, ((row as i32 * 7 + 3) % 20) as i8))
            .collect::<Vec<_>>();
        a_reused_generator_agrees_on_an_uneven_board(100, 20, 66, 9, &tiles);
    }

    #[test]
    #[inline]
    fn the_widest_board_agrees_with_its_transpose() {
        let tiles = (0..127)
            .map(|row| (row, ((row as i32 * 45 + 7) % 127) as i8))
            .collect::<Vec<_>>();
        a_reused_generator_agrees_on_an_uneven_board(127, 127, 70, 90, &tiles);
    }

    #[test]
    #[inline]
    fn a_lane_of_many_word_squares_is_ranked_without_overflow() {
        let (rows, cols) = (21, 21);
        let mut premiums = Vec::new();
        for row in 0..rows {
            for _ in 0..cols {
                premiums.push(board_layout::Premium {
                    word_multiplier: if row == 0 { 3 } else { 1 },
                    tile_multiplier: 1,
                });
            }
        }
        let gc = game_config::make_board_test_game_config(board_layout::make_test_board_layout(
            premiums.into_boxed_slice(),
            matrix::Dim { rows, cols },
            10,
            10,
        ));
        let kwg = test_kwg(&gc);
        let klv = klv::Klv::<kwg::Node22>::from_bytes_alloc(klv::EMPTY_KLV_BYTES);
        let mut board_tiles = vec![0u8; (rows as isize * cols as isize) as usize];
        let rack = parse_test_rack(gc.alphabet(), "AEST");
        board_tiles[gc.board_layout().dim().at_row_col(0, 9)] = rack[0];
        board_tiles[gc.board_layout().dim().at_row_col(1, 9)] = rack[3];
        let board_snapshot = BoardSnapshot {
            board_tiles: &board_tiles,
            game_config: &gc,
            kwg: &kwg,
            anagrams: None,
            rack_lengths: None,
            klv: &klv,
        };
        let plays = place_plays(
            &mut KurniaMoveGenerator::new(&gc),
            &board_snapshot,
            &rack,
            false,
        );
        assert!(plays.contains(&(false, 0, 9, vec![0, 19], 6_000)));
        assert!(plays.contains(&(false, 0, 7, vec![19, 5, 0, 20], 108_000)));
    }
}
