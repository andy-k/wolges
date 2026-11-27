// Copyright (C) 2020-2026 Andy Kurnia.

use super::{
    alphabet, alphagram, anagrams, bites, display, endgame, equity, error, game_config, klv, kwg,
    matrix, play_scorer, woadwage,
};
use alphagram::KeyPlaces as _;
use endgame::Grow as _;

pub const MAX_ALPHABET_LEN: usize = 64;

#[inline(always)]
pub fn bag_count_from_board(
    game_config: &game_config::GameConfig,
    num_tiles_on_board: u16,
) -> error::Returns<i16> {
    Ok(i16::try_from(bag_count_wide(
        game_config,
        num_tiles_on_board,
    ))?)
}

#[inline(always)]
fn bag_count_wide(game_config: &game_config::GameConfig, num_tiles_on_board: u16) -> i32 {
    i32::from(game_config.alphabet().num_tiles())
        - i32::from(num_tiles_on_board)
        - i32::from(game_config.num_players()) * i32::from(game_config.rack_size())
}

#[inline(always)]
pub fn live_pool_into(
    out: &mut [u8],
    alphabet: &alphabet::Alphabet,
    board_tiles: &[u8],
    rack_tally: &[u8],
) -> error::Returns<()> {
    if out.len() > usize::from(alphabet.len()) {
        crate::return_error!("live pool exceeds the alphabet".to_string());
    }
    fill_live_pool(out, alphabet, board_tiles, rack_tally);
    Ok(())
}

#[inline(always)]
fn fill_live_pool(
    out: &mut [u8],
    alphabet: &alphabet::Alphabet,
    board_tiles: &[u8],
    rack_tally: &[u8],
) {
    for (t, slot) in (0..alphabet.len()).zip(out.iter_mut()) {
        *slot = alphabet.freq(t);
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

// the letters a play may put on a square no cross word reaches, a bit each: a
// tile's own letter, else every letter. An empty square's bit 0 says a cross
// word reaches it; its cross set then has bit 0 set, so a dead square is 1 and
// a cross set allowing every letter of a 64-tile alphabet is not free.
#[inline(always)]
fn unhooked_letters(tile: u8) -> u64 {
    let on_tile = u64::from(tile != 0).wrapping_neg();
    (1u64 << (tile & 0x7f)) & on_tile | !1 & !on_tile
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

type PlaceMoveFn<'a> = &'a mut dyn FnMut(bool, i8, i8, &[u8], equity::Equity) -> bool;

type EquityFn<'a> = &'a mut dyn FnMut(equity::Equity, &Play) -> bool;

// a generation's found_place_move behind a dyn: handed a play as a walk finds
// it, it hands back the threshold
type FoundPlaceMoveFn<'a> =
    &'a mut dyn FnMut(bool, i8, i8, &[u8], equity::Equity, equity::Equity) -> equity::Equity;

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
    fn test(&mut self, down: bool, lane: i8, idx: i8, word: &[u8], score: equity::Equity) -> bool {
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
    kept_at: u32, // its kept span's multipliers' start (RcsLane), or NO_KEPT
}

// a placement no kept span made (the shadow pass's)
const NO_KEPT: u32 = u32::MAX;

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
    square_keys: Vec<u64>,
    ranked: Vec<bool>, // per aggregated word multiplier, whether its order is filled
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
            square_keys: Vec::new(),
            ranked: Vec::new(),
        }
    }

    #[inline(always)]
    fn ranking(
        &mut self,
        word_multiplier: i32,
        board_strip: &[u8],
        remaining_tile_multipliers_strip: &[i8],
        perpendicular_word_multipliers_strip: &[i8],
    ) -> usize {
        let at = self
            .aggregated_word_multipliers
            .binary_search(&word_multiplier)
            .unwrap();
        if !self.ranked[at] {
            self.rank(
                at,
                board_strip,
                remaining_tile_multipliers_strip,
                perpendicular_word_multipliers_strip,
            );
        }
        at * board_strip.len()
    }

    #[inline(always)]
    fn rank(
        &mut self,
        at: usize,
        board_strip: &[u8],
        remaining_tile_multipliers_strip: &[i8],
        perpendicular_word_multipliers_strip: &[i8],
    ) {
        let strider_len = board_strip.len();
        let k = self.aggregated_word_multipliers[at];
        let low_end = at * strider_len;
        let high_end = low_end + strider_len;
        let precomputed_square_multiplier_slice =
            &mut self.precomputed_square_multiplier[low_end..high_end];
        let indexes_to_descending_square_multiplier_slice =
            &mut self.indexes_to_descending_square_multiplier[low_end..high_end];
        if self.square_keys.len() < strider_len {
            self.square_keys.resize(strider_len, 0);
        }
        let mut n = 0;
        for j in 0..strider_len {
            let multiplier = i32::from(remaining_tile_multipliers_strip[j])
                * (k + i32::from(perpendicular_word_multipliers_strip[j]));
            precomputed_square_multiplier_slice[j] = multiplier;
            self.square_keys[n] = u64::from(!(multiplier as u32)) << u8::BITS | j as u64;
            n += usize::from(board_strip[j] == 0);
        }
        let keys = &self.square_keys[..n];
        let square_ranks_slice = &mut self.square_ranks[low_end..high_end];
        for &key in keys {
            let rank = keys.iter().filter(|&&other| other < key).count();
            let j = key as u8;
            indexes_to_descending_square_multiplier_slice[rank] = j as i8;
            square_ranks_slice[j as usize] = rank as u8;
        }
        self.ranked[at] = true;
    }

    #[inline]
    fn refresh<const D: usize>(
        &mut self,
        room: &endgame::Room,
        alphabet: &alphabet::Alphabet,
        board_strip: &[u8],
        remaining_word_multipliers_strip: &[i8],
        remaining_tile_multipliers_strip: &[i8],
        perpendicular_word_multipliers_strip: &[i8],
    ) -> bool {
        if self.valid
            && self.board_strip == board_strip
            && self.remaining_word_multipliers_strip == remaining_word_multipliers_strip
            && self.remaining_tile_multipliers_strip == remaining_tile_multipliers_strip
            && self.perpendicular_word_multipliers_strip == perpendicular_word_multipliers_strip
        {
            return true;
        }
        let strider_len = if D == 0 { board_strip.len() } else { D };
        if D != 0 {
            assert_eq!(board_strip.len(), D);
            assert_eq!(remaining_word_multipliers_strip.len(), D);
            assert_eq!(remaining_tile_multipliers_strip.len(), D);
            assert_eq!(perpendicular_word_multipliers_strip.len(), D);
        }
        self.valid = false;
        self.aggregated_word_multipliers.clear();
        let mut last_was_one = false;
        for i in 0..strider_len {
            let mut wm = i32::from(remaining_word_multipliers_strip[i]);
            if last_was_one {
                last_was_one = wm == 1;
                continue;
            }
            last_was_one = wm == 1;
            if let Err(idx) = self.aggregated_word_multipliers.binary_search(&wm) {
                if !self.aggregated_word_multipliers.room_for(room, 1) {
                    return false;
                }
                self.aggregated_word_multipliers.insert(idx, wm);
            }
            for &wm_val in &remaining_word_multipliers_strip[i + 1..strider_len] {
                if wm_val != 1 {
                    if wm > i32::MAX / equity::SCALE {
                        break;
                    }
                    wm *= i32::from(wm_val);
                    if let Err(idx) = self.aggregated_word_multipliers.binary_search(&wm) {
                        if !self.aggregated_word_multipliers.room_for(room, 1) {
                            return false;
                        }
                        self.aggregated_word_multipliers.insert(idx, wm);
                    }
                }
            }
        }
        let vec_size = strider_len * self.aggregated_word_multipliers.len();
        self.precomputed_square_multiplier
            .resize_in(room, vec_size, 0);
        self.indexes_to_descending_square_multiplier
            .resize_in(room, vec_size, 0);
        self.square_ranks.resize_in(room, vec_size, 0);
        self.ranked.clear();
        self.ranked
            .resize_in(room, self.aggregated_word_multipliers.len(), false);
        if self.square_keys.len() < strider_len {
            self.square_keys.resize_in(room, strider_len, 0);
        }
        let jump = MultiJump {
            left_score: 0,
            right_score: 0,
            left_idx: 0,
            right_idx: 0,
        };
        self.multi_jumps.resize_in(room, strider_len, jump);
        self.board_strip.clear();
        self.board_strip.extend_in(room, board_strip);
        self.remaining_word_multipliers_strip.clear();
        self.remaining_word_multipliers_strip
            .extend_in(room, remaining_word_multipliers_strip);
        self.remaining_tile_multipliers_strip.clear();
        self.remaining_tile_multipliers_strip
            .extend_in(room, remaining_tile_multipliers_strip);
        self.perpendicular_word_multipliers_strip.clear();
        self.perpendicular_word_multipliers_strip
            .extend_in(room, perpendicular_word_multipliers_strip);
        if room.refused() {
            return false;
        }

        fill_multi_jumps::<D>(alphabet, board_strip, &mut self.multi_jumps);

        self.valid = true;
        true
    }
}

// a lane's jumps over its runs of tiles: from each square, the nearest empty
// square at or to its right (the lane's end if none) and at or to its left (-1
// if none), with the face values of the tiles passed
#[inline(always)]
fn fill_multi_jumps<const D: usize>(
    alphabet: &alphabet::Alphabet,
    board_strip: &[u8],
    multi_jumps: &mut Vec<MultiJump>,
) {
    let strider_len = if D == 0 { board_strip.len() } else { D };
    if D != 0 {
        assert_eq!(board_strip.len(), D);
    }
    multi_jumps.resize(
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
            score += alphabet.scaled_score(b).raw();
        } else {
            score = 0; // cumulative face-value score (millipoints)
            last_empty = j as i8; // last seen empty square
        }
        multi_jumps[j].right_score = score;
        multi_jumps[j].right_idx = last_empty;
    }
    score = 0i32;
    last_empty = -1i8;
    for (j, &b) in board_strip.iter().enumerate() {
        if b != 0 {
            score += alphabet.scaled_score(b).raw();
        } else {
            score = 0; // cumulative face-value score (millipoints)
            last_empty = j as i8; // last seen empty square
        }
        multi_jumps[j].left_score = score;
        multi_jumps[j].left_idx = last_empty;
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

// a square a walk took: its multiplier under the walk's word multiplier in the
// high half, the square in the low byte
#[derive(Clone, Copy, Default)]
struct Taken(u64);

impl Taken {
    #[inline(always)]
    fn new(multiplier: i32, idx: u8) -> Self {
        Self(u64::from(multiplier.cast_unsigned()) << u32::BITS | u64::from(idx))
    }

    #[inline(always)]
    fn multiplier(self) -> i32 {
        ((self.0 >> u32::BITS) as u32).cast_signed()
    }

    #[inline(always)]
    fn idx(self) -> usize {
        (self.0 & u64::from(u8::MAX)) as usize
    }
}

// the table walk's spans of a lane, kept until its inputs change: found by the
// shadow pass with every letter on hand without limit, so they are every span a
// rack can have; each with what of its bound needs no rack (rcs_placements)
#[derive(Default)]
struct RcsLane {
    valid: bool,
    stamp: (u64, u64), // who found its spans and when (rcs_owner, rcs_rev)
    tiles: Vec<u8>,
    allowed: Vec<u64>,
    word_multipliers: Vec<i8>,
    tile_multipliers: Vec<i8>,
    perpendicular_word_multipliers: Vec<i8>,
    perpendicular_scores: Vec<i32>,
    jumps: Vec<MultiJump>, // over its runs of tiles (fill_multi_jumps)
    hooks: u64,            // its empty squares a cross word reaches, a bit each
    spans: Vec<RcsSpan>,
    multipliers: Vec<Taken>, // each span's empty squares' with the square, descending, from its at
}

// a copy into a lane reuses its vectors
impl Clone for RcsLane {
    #[inline(always)]
    fn clone(&self) -> Self {
        let mut lane = Self::default();
        lane.clone_from(self);
        lane
    }

    #[inline(always)]
    fn clone_from(&mut self, source: &Self) {
        self.valid = source.valid;
        self.stamp = source.stamp;
        self.tiles.clone_from(&source.tiles);
        self.allowed.clone_from(&source.allowed);
        self.word_multipliers.clone_from(&source.word_multipliers);
        self.tile_multipliers.clone_from(&source.tile_multipliers);
        self.perpendicular_word_multipliers
            .clone_from(&source.perpendicular_word_multipliers);
        self.perpendicular_scores
            .clone_from(&source.perpendicular_scores);
        self.jumps.clone_from(&source.jumps);
        self.hooks = source.hooks;
        self.spans.clone_from(&source.spans);
        self.multipliers.clone_from(&source.multipliers);
    }
}

#[derive(Clone)]
struct RcsSpan {
    anchor: i8,
    left: i8,
    right: i8,
    num_played: u8,
    crossed: bool, // it holds a board tile
    fixed: i32, // its board tiles' scores times its word multiplier, plus its cross words' board scores
    at: u32,
}

// a working buffer's own number, for the stamps of the lanes it finds: no two
// buffers of a process share one
#[inline(always)]
fn next_rcs_owner() -> u64 {
    static OWNERS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    OWNERS.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
}

// what a tile placement adds to its equity for the bag it leaves: by the bag
// size after the play (negative when it overdraws: the bag empties and the
// mover keeps a short rack), values[b - lowest] in millipoints for b from
// lowest on; a size outside the values adds none, and so does any play when the
// bag is already empty. An exchange leaves the bag as it was and gets none. The
// values are the caller's: there are none by default.
#[derive(Clone, Debug, PartialEq)]
pub struct BagTerm {
    lowest: i16,
    values: Box<[i32]>,
}

impl BagTerm {
    pub fn new(lowest: i16, values: Vec<i32>) -> Self {
        Self {
            lowest,
            values: values.into_boxed_slice(),
        }
    }

    // the term for a play leaving bag_after tiles in the bag
    #[inline(always)]
    pub fn at(&self, bag_after: i16) -> i32 {
        self.at_wide(i32::from(bag_after))
    }

    #[inline(always)]
    fn at_wide(&self, bag_after: i32) -> i32 {
        usize::try_from(bag_after - i32::from(self.lowest))
            .ok()
            .and_then(|i| self.values.get(i).copied())
            .unwrap_or(0)
    }

    // "b value" lines, value in points (blank lines and # comments skipped)
    pub fn parse(text: &str) -> error::Returns<Self> {
        let mut pairs = Vec::new();
        for line in text.lines() {
            let line = line.split('#').next().unwrap_or("").trim();
            if line.is_empty() {
                continue;
            }
            let mut it = line.split_whitespace();
            let (Some(b), Some(v), None) = (it.next(), it.next(), it.next()) else {
                return_error!(format!("a bag term line is \"b value\", not {line:?}"));
            };
            let b: i16 = b.parse()?;
            let v: f64 = v.parse()?;
            let v = (v * f64::from(equity::SCALE)).round();
            if !v.is_finite() || v.abs() > f64::from(i32::MAX / 4) {
                return_error!(format!("the bag term {v} for {b} is out of range"));
            }
            pairs.push((b, v as i32));
        }
        let Some(lowest) = pairs.iter().map(|&(b, _)| b).min() else {
            return_error!("a bag term needs at least one value".to_string());
        };
        let highest = pairs.iter().map(|&(b, _)| b).max().unwrap();
        let mut values = vec![0i32; (i32::from(highest) - i32::from(lowest) + 1) as usize];
        let mut seen = vec![false; values.len()];
        for (b, v) in pairs {
            let i = usize::try_from(i32::from(b) - i32::from(lowest))?;
            if seen[i] {
                return_error!(format!("the bag term for {b} is given twice"));
            }
            seen[i] = true;
            values[i] = v;
        }
        Ok(Self::new(lowest, values))
    }
}

pub trait BoardBuf<T: Clone>: std::ops::DerefMut<Target = [T]> + Clone {
    fn copy_in(&mut self, from: &[T]);
}

impl<T: Clone> BoardBuf<T> for Box<[T]> {
    #[inline(always)]
    fn copy_in(&mut self, from: &[T]) {
        if self.len() == from.len() {
            self.clone_from_slice(from);
        } else {
            *self = from.into();
        }
    }
}

#[derive(Clone)]
pub struct Squares<T, const N: usize>([T; N]);

impl<T, const N: usize> std::ops::Deref for Squares<T, N> {
    type Target = [T];
    #[inline(always)]
    fn deref(&self) -> &[T] {
        &self.0
    }
}

impl<T, const N: usize> std::ops::DerefMut for Squares<T, N> {
    #[inline(always)]
    fn deref_mut(&mut self) -> &mut [T] {
        &mut self.0
    }
}

impl<T: Clone, const N: usize> BoardBuf<T> for Squares<T, N> {
    #[inline(always)]
    fn copy_in(&mut self, from: &[T]) {
        self.0.clone_from_slice(from);
    }
}

// a generator's board buffers, for one board area (0 for any)
pub trait BoardStore {
    const SQUARES: usize;
    type Of<T: Clone>: BoardBuf<T>;
    fn filled<T: Clone>(len: usize, value: T) -> Self::Of<T>;

    #[inline(always)]
    fn filled_in<T: Clone>(room: &endgame::Room, len: usize, value: T) -> Self::Of<T> {
        let bytes = if Self::SQUARES == 0 {
            len * std::mem::size_of::<T>()
        } else {
            0
        };
        let taken = room.take(bytes);
        Self::filled(if taken.is_some() { len } else { 0 }, value)
    }
}

#[inline(always)]
fn boxed_in<T: Clone>(room: &endgame::Room, len: usize, value: T) -> Box<[T]> {
    let taken = room.take(len * std::mem::size_of::<T>());
    let len = if taken.is_some() { len } else { 0 };
    vec![value; len].into_boxed_slice()
}

pub struct BoxStore;

impl BoardStore for BoxStore {
    const SQUARES: usize = 0;
    type Of<T: Clone> = Box<[T]>;
    #[inline(always)]
    fn filled<T: Clone>(len: usize, value: T) -> Box<[T]> {
        vec![value; len].into_boxed_slice()
    }
}

pub struct InlineStore<const N: usize>;

impl<const N: usize> BoardStore for InlineStore<N> {
    const SQUARES: usize = N;
    type Of<T: Clone> = Squares<T, N>;
    #[inline(always)]
    fn filled<T: Clone>(_len: usize, value: T) -> Squares<T, N> {
        Squares(std::array::from_fn(|_| value.clone()))
    }
}

// WorkingBuffer can only be reused for the same game_config and kwg.
// (The kwg is partially cached in cached_cross_set.)
// WorkingBuffer can also be reset for reuse with another kwg by calling
// reset_for_another_kwg().
// This is not enforced.
struct WorkingBuffer<G: LaneBits, const BAG_TERM: bool, S: BoardStore> {
    rack_tally: [u8; MAX_ALPHABET_LEN],
    word_buffer_for_across_plays: S::Of<u8>,  // r*c
    word_buffer_for_down_plays: S::Of<u8>,    // c*r
    cross_score_for_across_plays: S::Of<i32>, // r*c (the cross word's tiles' score)
    cross_score_for_down_plays: S::Of<i32>,   // c*r
    cached_cross_set_for_across_plays: S::Of<CachedCrossSet>, // c*r
    cached_cross_set_for_down_plays: S::Of<CachedCrossSet>, // r*c
    cross_set_buffer_for_across_plays: S::Of<CrossSetComputation>, // c*r (perpendicular strips)
    cross_set_buffer_for_down_plays: S::Of<CrossSetComputation>, // r*c (perpendicular strips)
    prev_board_tiles: S::Of<u8>,              // r*c (previous board tiles for dirty tracking)
    dirty_rows: G,
    dirty_cols: G,
    lane_rows: G,
    lane_cols: G,
    remaining_word_multipliers_for_across_plays: S::Of<i8>, // r*c (1 if tile placed)
    remaining_word_multipliers_for_down_plays: S::Of<i8>,   // c*r
    remaining_tile_multipliers_for_across_plays: S::Of<i8>, // r*c (1 if tile placed)
    remaining_tile_multipliers_for_down_plays: S::Of<i8>,   // c*r
    face_value_scores_for_across_plays: S::Of<i32>,         // r*c (premultiplied by SCALE)
    face_value_scores_for_down_plays: S::Of<i32>,           // c*r (premultiplied by SCALE)
    perpendicular_word_multipliers_for_across_plays: S::Of<i8>, // r*c (0 if no perpendicularly adjacent tile)
    perpendicular_word_multipliers_for_down_plays: S::Of<i8>,   // c*r
    perpendicular_scores_for_across_plays: S::Of<i32>, // r*c (multiplied by perpendicular_word_multipliers)
    perpendicular_scores_for_down_plays: S::Of<i32>,   // c*r
    allowed_for_across_plays: S::Of<u64>,              // r*c (unhooked_letters or a cross set)
    allowed_for_down_plays: S::Of<u64>,                // c*r
    transposed_board_tiles: S::Of<u8>,                 // c*r
    num_tiles_on_board: u16,
    board_tile_score: i32, // the board's tiles' face values, as init last found them
    prev_tile_count: u16,  // num_tiles_on_board for prev_board_tiles (with no 0xff)
    prev_tile_score: i32,  // board_tile_score for prev_board_tiles (with no 0xff)
    num_tiles_in_bag: i32, // negative when players also have less than full racks
    play_out_bonus: i32,
    bag_term: Option<BagTerm>, // the caller's, if any
    num_tiles_on_rack: u8,
    rack_bits: u64, // bit 0 = blank conveniently matches bit 0 = have cross set
    multi_leaves: klv::MultiLeaves,
    dynamic_kept: klv::DynamicKept,
    descending_scores: Vec<i32>, // rack.len() (premultiplied by SCALE)
    exchange_buffer: Vec<u8>,    // rack.len(), or max(word length) with word prune
    lane_scaffold: Box<[LaneScaffold]>, // rows + cols, across lanes first
    best_leave_values: Vec<i32>, // rack.len() + 1
    span_out: Vec<(i8, i8, u8, i32)>,
    subracks: Vec<Subrack>,
    subracks_by_played: Vec<u32>,
    subrack_order: Vec<u64>,
    subracks_spare: Vec<Subrack>,
    found_placements: Vec<PossiblePlacement>,
    raw_anchors: Vec<(i8, i8, i8)>, // a lane's anchors, raw: (anchor, leftmost, rightmost)
    sure_plays: Vec<equity::Equity>, // sure_floor's, reused
    rcs_lanes: Vec<RcsLane>,        // the table walk's, rows then columns
    prune: PruneBuffers,            // gen_remaining_words's, reused
    board_cols: u8,                 // the board's columns, for restore_board_state's lanes
    longest_lane: usize,            // the board's rows or columns, whichever are more
    grown_bytes: usize,             // what a play grows a raw list by, with its word
    rcs_pending_rows: G,            // rows whose tiles changed since the table walk last looked
    rcs_pending_cols: G,            // columns whose tiles changed since then
    rcs_stale_rows: G,              // across lanes whose kept spans may be out of date
    rcs_stale_cols: G,              // down lanes whose kept spans may be out of date
    rcs_owner: u64,                 // this buffer's own, for its lanes' stamps (none other's)
    rcs_rev: u64,                   // the lanes it has found, for their stamps
    placement_order: Vec<(i32, u32)>,
    placement_scratch: Vec<(i32, u32)>,
    span_memo: SpanMemo,
    span_prefix: Box<[SpanPrefix]>, // r*(c+1), then c*(r+1): each lane's sums
    span_lanes: Box<[SpanLane]>,    // rows + cols, across lanes first
    span_turn: u32,                 // the generation's, for span_lanes
    span_look_turn: u32,            // the generation the table walk last checked its lanes in
    span_source: usize,             // the tables and keys span_lanes's kept sums are by
    exist_memo: Box<[ExistSlot]>,   // key check answers, made at first use
    exist_turn: u32,                // the generation's, for exist_memo
    anchor_left_bounds: Vec<i32>,   // per lean placement, two blocks by left edge
    bounds_at: usize,               // the popped lean placement's block, or usize::MAX
    used_letters_tally: Vec<u8>,
    woadwage: WoadwageScratch,
    jumbled: JumbledMultisets,                  // jumbled mode only
    pool: PoolScratch,                          // the pool walks'
    used_tile_scores_shadowl: ShadowScores,     // for shadow_play_left, premultiplied by SCALE
    used_tile_scores_shadowr: ShadowScores,     // for shadow_play_right, premultiplied by SCALE
    rack_tally_shadowl: [u8; MAX_ALPHABET_LEN], // for shadow_play_left
    rack_tally_shadowr: [u8; MAX_ALPHABET_LEN], // for shadow_play_right
    anagrams_fit_config: bool,
    is_census: bool,
    room: endgame::Room, // the request's limit, and any growth it refused
}

impl<G: LaneBits, const BAG_TERM: bool, S: BoardStore> Clone for WorkingBuffer<G, BAG_TERM, S> {
    #[inline(always)]
    fn clone(&self) -> Self {
        Self {
            rack_tally: self.rack_tally,
            word_buffer_for_across_plays: self.word_buffer_for_across_plays.clone(),
            word_buffer_for_down_plays: self.word_buffer_for_down_plays.clone(),
            cross_score_for_across_plays: self.cross_score_for_across_plays.clone(),
            cross_score_for_down_plays: self.cross_score_for_down_plays.clone(),
            cached_cross_set_for_across_plays: self.cached_cross_set_for_across_plays.clone(),
            cached_cross_set_for_down_plays: self.cached_cross_set_for_down_plays.clone(),
            cross_set_buffer_for_across_plays: self.cross_set_buffer_for_across_plays.clone(),
            cross_set_buffer_for_down_plays: self.cross_set_buffer_for_down_plays.clone(),
            prev_board_tiles: self.prev_board_tiles.clone(),
            dirty_rows: self.dirty_rows,
            dirty_cols: self.dirty_cols,
            lane_rows: self.lane_rows,
            lane_cols: self.lane_cols,
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
            allowed_for_across_plays: self.allowed_for_across_plays.clone(),
            allowed_for_down_plays: self.allowed_for_down_plays.clone(),
            transposed_board_tiles: self.transposed_board_tiles.clone(),
            num_tiles_on_board: self.num_tiles_on_board,
            board_tile_score: self.board_tile_score,
            prev_tile_count: self.prev_tile_count,
            prev_tile_score: self.prev_tile_score,
            num_tiles_in_bag: self.num_tiles_in_bag,
            play_out_bonus: self.play_out_bonus,
            bag_term: self.bag_term.clone(),
            num_tiles_on_rack: self.num_tiles_on_rack,
            rack_bits: self.rack_bits,
            multi_leaves: self.multi_leaves.clone(),
            dynamic_kept: self.dynamic_kept.clone(),
            descending_scores: self.descending_scores.clone(),
            exchange_buffer: self.exchange_buffer.clone(),
            lane_scaffold: self.lane_scaffold.clone(),
            best_leave_values: self.best_leave_values.clone(),
            span_out: self.span_out.clone(),
            subracks: self.subracks.clone(),
            subracks_by_played: self.subracks_by_played.clone(),
            subrack_order: self.subrack_order.clone(),
            subracks_spare: self.subracks_spare.clone(),
            found_placements: self.found_placements.clone(),
            raw_anchors: self.raw_anchors.clone(),
            sure_plays: Vec::new(),
            rcs_lanes: self.rcs_lanes.clone(),
            prune: self.prune.clone(),
            board_cols: self.board_cols,
            longest_lane: self.longest_lane,
            grown_bytes: self.grown_bytes,
            rcs_pending_rows: self.rcs_pending_rows,
            rcs_pending_cols: self.rcs_pending_cols,
            rcs_stale_rows: self.rcs_stale_rows,
            rcs_stale_cols: self.rcs_stale_cols,
            rcs_owner: next_rcs_owner(),
            rcs_rev: 0,
            placement_order: self.placement_order.clone(),
            placement_scratch: self.placement_scratch.clone(),
            span_memo: self.span_memo.clone(),
            span_prefix: self.span_prefix.clone(),
            span_lanes: self.span_lanes.clone(),
            span_turn: self.span_turn,
            span_look_turn: self.span_look_turn,
            span_source: self.span_source,
            exist_memo: self.exist_memo.clone(),
            exist_turn: self.exist_turn,
            anchor_left_bounds: self.anchor_left_bounds.clone(),
            bounds_at: self.bounds_at,
            used_letters_tally: self.used_letters_tally.clone(),
            jumbled: self.jumbled.clone(),
            woadwage: WoadwageScratch::default(),
            pool: PoolScratch::default(),
            used_tile_scores_shadowl: self.used_tile_scores_shadowl.clone(),
            used_tile_scores_shadowr: self.used_tile_scores_shadowr.clone(),
            rack_tally_shadowl: self.rack_tally_shadowl,
            rack_tally_shadowr: self.rack_tally_shadowr,
            is_census: self.is_census,
            anagrams_fit_config: self.anagrams_fit_config,
            room: self.room.clone(),
        }
    }

    #[inline(always)]
    fn clone_from(&mut self, source: &Self) {
        self.rack_tally.clone_from(&source.rack_tally);
        self.word_buffer_for_across_plays
            .clone_from(&source.word_buffer_for_across_plays);
        self.word_buffer_for_down_plays
            .clone_from(&source.word_buffer_for_down_plays);
        self.cross_score_for_across_plays
            .clone_from(&source.cross_score_for_across_plays);
        self.cross_score_for_down_plays
            .clone_from(&source.cross_score_for_down_plays);
        self.cached_cross_set_for_across_plays
            .clone_from(&source.cached_cross_set_for_across_plays);
        self.cached_cross_set_for_down_plays
            .clone_from(&source.cached_cross_set_for_down_plays);
        self.cross_set_buffer_for_across_plays
            .clone_from(&source.cross_set_buffer_for_across_plays);
        self.cross_set_buffer_for_down_plays
            .clone_from(&source.cross_set_buffer_for_down_plays);
        self.prev_board_tiles.clone_from(&source.prev_board_tiles);
        self.dirty_rows = source.dirty_rows;
        self.dirty_cols = source.dirty_cols;
        self.lane_rows = source.lane_rows;
        self.lane_cols = source.lane_cols;
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
        self.allowed_for_across_plays
            .clone_from(&source.allowed_for_across_plays);
        self.allowed_for_down_plays
            .clone_from(&source.allowed_for_down_plays);
        self.transposed_board_tiles
            .clone_from(&source.transposed_board_tiles);
        self.num_tiles_on_board
            .clone_from(&source.num_tiles_on_board);
        self.board_tile_score.clone_from(&source.board_tile_score);
        self.prev_tile_count.clone_from(&source.prev_tile_count);
        self.prev_tile_score.clone_from(&source.prev_tile_score);
        self.num_tiles_in_bag.clone_from(&source.num_tiles_in_bag);
        self.play_out_bonus.clone_from(&source.play_out_bonus);
        self.bag_term.clone_from(&source.bag_term);
        self.num_tiles_on_rack.clone_from(&source.num_tiles_on_rack);
        self.rack_bits.clone_from(&source.rack_bits);
        self.multi_leaves.clone_from(&source.multi_leaves);
        self.dynamic_kept.clone_from(&source.dynamic_kept);
        self.descending_scores.clone_from(&source.descending_scores);
        self.exchange_buffer.clone_from(&source.exchange_buffer);
        self.lane_scaffold.clone_from(&source.lane_scaffold);
        self.best_leave_values.clone_from(&source.best_leave_values);
        self.span_out.clone_from(&source.span_out);
        self.subracks.clone_from(&source.subracks);
        self.subracks_by_played
            .clone_from(&source.subracks_by_played);
        self.subrack_order.clone_from(&source.subrack_order);
        self.subracks_spare.clone_from(&source.subracks_spare);
        self.found_placements.clone_from(&source.found_placements);
        self.raw_anchors.clone_from(&source.raw_anchors);
        self.rcs_lanes.clone_from(&source.rcs_lanes);
        self.board_cols = source.board_cols;
        self.longest_lane = source.longest_lane;
        self.grown_bytes = source.grown_bytes;
        self.rcs_pending_rows = source.rcs_pending_rows;
        self.rcs_pending_cols = source.rcs_pending_cols;
        self.rcs_stale_rows = source.rcs_stale_rows;
        self.rcs_stale_cols = source.rcs_stale_cols;
        self.placement_order.clone_from(&source.placement_order);
        self.placement_scratch.clone_from(&source.placement_scratch);
        self.span_memo.clone_from(&source.span_memo);
        self.span_prefix.clone_from(&source.span_prefix);
        self.span_lanes.clone_from(&source.span_lanes);
        self.span_turn = source.span_turn;
        self.span_look_turn = source.span_look_turn;
        self.span_source = source.span_source;
        self.exist_memo.clone_from(&source.exist_memo);
        self.exist_turn = source.exist_turn;
        self.anchor_left_bounds
            .clone_from(&source.anchor_left_bounds);
        self.bounds_at = source.bounds_at;
        self.used_letters_tally
            .clone_from(&source.used_letters_tally);
        self.jumbled.clone_from(&source.jumbled);
        self.used_tile_scores_shadowl
            .clone_from(&source.used_tile_scores_shadowl);
        self.used_tile_scores_shadowr
            .clone_from(&source.used_tile_scores_shadowr);
        self.rack_tally_shadowl
            .clone_from(&source.rack_tally_shadowl);
        self.rack_tally_shadowr
            .clone_from(&source.rack_tally_shadowr);
        self.is_census = source.is_census;
        self.anagrams_fit_config = source.anagrams_fit_config;
    }
}

impl<G: LaneBits, const BAG_TERM: bool, S: BoardStore> WorkingBuffer<G, BAG_TERM, S> {
    fn new(game_config: &game_config::GameConfig, room: endgame::Room) -> Self {
        let dim = game_config.board_layout().dim();
        let rows_times_cols = (dim.rows as isize * dim.cols as isize) as usize;
        Self {
            rack_tally: [0u8; MAX_ALPHABET_LEN],
            word_buffer_for_across_plays: S::filled_in(&room, rows_times_cols, 0u8),
            word_buffer_for_down_plays: S::filled_in(&room, rows_times_cols, 0u8),
            cross_score_for_across_plays: S::filled_in(&room, rows_times_cols, 0i32),
            cross_score_for_down_plays: S::filled_in(&room, rows_times_cols, 0i32),
            cached_cross_set_for_across_plays: S::filled_in(
                &room,
                rows_times_cols,
                CachedCrossSet {
                    p_left: 0,
                    p_right: 0,
                    bits: 0,
                },
            ),
            cached_cross_set_for_down_plays: S::filled_in(
                &room,
                rows_times_cols,
                CachedCrossSet {
                    p_left: 0,
                    p_right: 0,
                    bits: 0,
                },
            ),
            cross_set_buffer_for_across_plays: S::filled_in(
                &room,
                rows_times_cols,
                CrossSetComputation {
                    score: 0,
                    b_letter: 0,
                    end_range: 0,
                    p: 0,
                },
            ),
            cross_set_buffer_for_down_plays: S::filled_in(
                &room,
                rows_times_cols,
                CrossSetComputation {
                    score: 0,
                    b_letter: 0,
                    end_range: 0,
                    p: 0,
                },
            ),
            prev_board_tiles: S::filled_in(&room, rows_times_cols, 0xffu8),
            dirty_rows: G::from(0),
            dirty_cols: G::from(0),
            lane_rows: !G::from(0),
            lane_cols: !G::from(0),
            remaining_word_multipliers_for_across_plays: S::filled_in(&room, rows_times_cols, 0i8),
            remaining_word_multipliers_for_down_plays: S::filled_in(&room, rows_times_cols, 0i8),
            remaining_tile_multipliers_for_across_plays: S::filled_in(&room, rows_times_cols, 0i8),
            remaining_tile_multipliers_for_down_plays: S::filled_in(&room, rows_times_cols, 0i8),
            face_value_scores_for_across_plays: S::filled_in(&room, rows_times_cols, 0i32),
            face_value_scores_for_down_plays: S::filled_in(&room, rows_times_cols, 0i32),
            perpendicular_word_multipliers_for_across_plays: S::filled_in(
                &room,
                rows_times_cols,
                0i8,
            ),
            perpendicular_word_multipliers_for_down_plays: S::filled_in(
                &room,
                rows_times_cols,
                0i8,
            ),
            perpendicular_scores_for_across_plays: S::filled_in(&room, rows_times_cols, 0i32),
            perpendicular_scores_for_down_plays: S::filled_in(&room, rows_times_cols, 0i32),
            allowed_for_across_plays: S::filled_in(&room, rows_times_cols, !1u64),
            allowed_for_down_plays: S::filled_in(&room, rows_times_cols, !1u64),
            transposed_board_tiles: S::filled_in(&room, rows_times_cols, 0u8),
            num_tiles_on_board: 0,
            board_tile_score: 0,
            prev_tile_count: 0,
            prev_tile_score: 0,
            num_tiles_in_bag: 0,
            play_out_bonus: 0,
            bag_term: None,
            num_tiles_on_rack: 0,
            rack_bits: 0,
            multi_leaves: klv::MultiLeaves::new(),
            dynamic_kept: klv::DynamicKept::default(),
            descending_scores: Vec::new(),
            exchange_buffer: Vec::new(),
            lane_scaffold: boxed_in(
                &room,
                dim.rows as usize + dim.cols as usize,
                LaneScaffold::new(),
            ),
            best_leave_values: Vec::new(),
            span_out: Vec::new(),
            subracks: Vec::new(),
            subracks_by_played: Vec::new(),
            subrack_order: Vec::new(),
            subracks_spare: Vec::new(),
            found_placements: Vec::new(),
            raw_anchors: Vec::new(),
            sure_plays: Vec::new(),
            rcs_lanes: Vec::new(),
            prune: PruneBuffers::default(),
            board_cols: dim.cols as u8,
            longest_lane: dim.rows.max(dim.cols) as usize,
            grown_bytes: 2 * std::mem::size_of::<ValuedMove>() + dim.rows.max(dim.cols) as usize,
            rcs_pending_rows: !G::from(0),
            rcs_pending_cols: !G::from(0),
            rcs_stale_rows: !G::from(0),
            rcs_stale_cols: !G::from(0),
            rcs_owner: next_rcs_owner(),
            rcs_rev: 0,
            placement_order: Vec::new(),
            placement_scratch: Vec::new(),
            span_memo: SpanMemo::new(),
            span_prefix: boxed_in(
                &room,
                dim.rows as usize * (dim.cols as usize + 1)
                    + dim.cols as usize * (dim.rows as usize + 1),
                SpanPrefix::new(),
            ),
            span_lanes: boxed_in(
                &room,
                dim.rows as usize + dim.cols as usize,
                SpanLane::new(),
            ),
            span_turn: 0,
            span_look_turn: 0,
            span_source: 0,
            exist_memo: Box::new([]),
            exist_turn: 0,
            anchor_left_bounds: Vec::new(),
            bounds_at: usize::MAX,
            used_letters_tally: Vec::new(),
            jumbled: JumbledMultisets::new(),
            woadwage: WoadwageScratch::default(),
            pool: PoolScratch::default(),
            used_tile_scores_shadowl: ShadowScores::new(),
            used_tile_scores_shadowr: ShadowScores::new(),
            rack_tally_shadowl: [0u8; MAX_ALPHABET_LEN],
            rack_tally_shadowr: [0u8; MAX_ALPHABET_LEN],
            is_census: false,
            anagrams_fit_config: false,
            room,
        }
    }

    #[inline]
    fn turn_is_supported<N: kwg::Node, L: kwg::Node>(
        &self,
        board_snapshot: &BoardSnapshot<'_, N, L>,
    ) -> bool {
        match board_snapshot.game_config.game_rules() {
            game_config::GameRules::Classic => {}
            game_config::GameRules::Jumbled
            | game_config::GameRules::JumbledWoadwage
            | game_config::GameRules::Woadwage => return false,
        }
        let dim = board_snapshot.game_config.board_layout().dim();
        let extent = dim.rows.max(dim.cols) as u8;
        let layout = board_snapshot.anagrams.map(anagrams::Anagrams::layout);
        self.anagrams_fit_config
            && !self.subracks.is_empty()
            && layout
                .is_some_and(|layout| layout.covers(board_snapshot.game_config.alphabet(), extent))
    }

    // T: the key places the subracks are keyed by, the anagrams' own
    fn init<const D: usize, const LEAVES: bool, T: TablePlaces, N: kwg::Node, L: kwg::Node>(
        &mut self,
        board_snapshot: &BoardSnapshot<'_, N, L>,
        rack: &[u8],
        adjust_leave_value: klv::AdjustLeave,
        dynamic_leaves: Option<klv::DynamicLeavesRef<'_>>,
        dense_most: u32,
        other_leaves: OtherLeaves<'_, L>,
    ) {
        let alphabet = board_snapshot.game_config.alphabet();
        self.anagrams_fit_config = board_snapshot
            .anagrams
            .is_some_and(|anagrams| anagrams.fit(board_snapshot.game_config));
        // a new generation: each lane's span sums are filled again when read
        self.span_turn = self.span_turn.wrapping_add(1);
        if self.span_turn == 0 {
            self.span_lanes.fill(SpanLane::new());
            self.span_turn = 1;
            self.span_look_turn = 0;
        }
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
        self.jumbled.forget();
        let board_layout = board_snapshot.game_config.board_layout();
        let fixed;
        let dim = if D != 0 {
            assert!(board_layout.dim().rows as usize == D && board_layout.dim().cols as usize == D);
            fixed = square_dim::<D>();
            &fixed
        } else {
            board_layout.dim()
        };
        let area = (dim.rows as isize * dim.cols as isize) as usize;
        let was_empty = self.num_tiles_on_board == 0;

        let premiums = board_layout.premiums();
        let transposed_premiums = board_layout.transposed_premiums();
        let mut dirty_rows = G::from(0);
        let mut dirty_cols = G::from(0);
        // the tiles and face values the squares that changed gained, against
        // prev_board_tiles's counts; a board it holds as unknown (0xff) is
        // counted afresh
        let face = |t: u8| {
            if t != 0 {
                i32::from(alphabet.score(t))
            } else {
                0
            }
        };
        let mut tiles_gained = 0i32;
        let mut score_gained = 0i32;
        let mut unknown = false;
        let board = &board_snapshot.board_tiles[..area];
        let mut base = 0;
        while base < area {
            let end = (base + 16).min(area);
            let same = if end - base == 16 {
                u128::from_ne_bytes(board[base..end].try_into().unwrap())
                    == u128::from_ne_bytes(self.prev_board_tiles[base..end].try_into().unwrap())
            } else {
                board[base..end] == self.prev_board_tiles[base..end]
            };
            if same {
                base = end;
                continue;
            }
            for idx in base..end {
                let b = board[idx];
                let prev = self.prev_board_tiles[idx];
                if b == prev {
                    continue;
                }
                if prev == 0xff {
                    unknown = true;
                } else {
                    tiles_gained += i32::from(b != 0) - i32::from(prev != 0);
                    score_gained += face(b) - face(prev);
                }
                let row = idx / dim.cols as usize;
                let col = idx % dim.cols as usize;
                dirty_rows |= G::from(1) << row as u32;
                dirty_cols |= G::from(1) << col as u32;
                let transposed_idx = col * dim.rows as usize + row;
                self.transposed_board_tiles[transposed_idx] = b;
                // a played word reads 0 on the board's tiles, so a square that
                // changed keeps no letter a walk left on it
                self.word_buffer_for_across_plays[idx] = 0;
                self.word_buffer_for_down_plays[transposed_idx] = 0;
                if b == 0 {
                    let premium = &premiums[idx];
                    self.remaining_word_multipliers_for_across_plays[idx] = premium.word_multiplier;
                    self.remaining_tile_multipliers_for_across_plays[idx] = premium.tile_multiplier;
                    self.face_value_scores_for_across_plays[idx] = 0;
                    let premium = &transposed_premiums[transposed_idx];
                    self.remaining_word_multipliers_for_down_plays[transposed_idx] =
                        premium.word_multiplier;
                    self.remaining_tile_multipliers_for_down_plays[transposed_idx] =
                        premium.tile_multiplier;
                    self.face_value_scores_for_down_plays[transposed_idx] = 0;
                } else {
                    self.remaining_word_multipliers_for_across_plays[idx] = 1; // needed for the HashMap
                    self.face_value_scores_for_across_plays[idx] = alphabet.scaled_score(b).raw();
                    self.remaining_word_multipliers_for_down_plays[transposed_idx] = 1; // needed for the HashMap
                    self.face_value_scores_for_down_plays[transposed_idx] =
                        alphabet.scaled_score(b).raw();
                }
            }
            base = end;
        }
        if unknown {
            self.num_tiles_on_board = board.iter().filter(|&&t| t != 0).count() as u16;
            self.board_tile_score = board.iter().map(|&t| face(t)).sum();
        } else {
            self.num_tiles_on_board = (i32::from(self.prev_tile_count) + tiles_gained) as u16;
            self.board_tile_score = self.prev_tile_score + score_gained;
        }
        if was_empty || self.num_tiles_on_board == 0 {
            dirty_rows |= G::from(1) << board_layout.star_row() as u32;
            dirty_cols |= G::from(1) << board_layout.star_col() as u32;
        }
        self.dirty_rows = dirty_rows;
        self.dirty_cols = dirty_cols;
        // kept for the table walk until it looks, whichever arm generates now
        self.rcs_pending_rows |= dirty_rows;
        self.rcs_pending_cols |= dirty_cols;
        self.num_tiles_in_bag = bag_count_wide(board_snapshot.game_config, self.num_tiles_on_board);
        let play_out_bonus = if self.num_tiles_in_bag <= 0 {
            2 * ((0u8..)
                .zip(self.rack_tally.iter().take(alphabet.len() as usize))
                .map(|(tile, &num)| {
                    (i32::from(alphabet.freq(tile)) - i32::from(num))
                        * i32::from(alphabet.score(tile))
                })
                .sum::<i32>()
                - self.board_tile_score)
                * equity::SCALE
        } else {
            0
        };
        self.play_out_bonus = play_out_bonus;

        self.descending_scores.clear();
        self.descending_scores
            .reserve(self.num_tiles_on_rack as usize);
        // each tile's first place in descending_scores, for the subracks'
        // places (a rack holds at most u8::MAX tiles)
        let mut first_places = [0u8; MAX_ALPHABET_LEN];
        for &tile in alphabet.tiles_by_descending_scores() {
            let count = self.rack_tally[tile as usize];
            if count != 0 {
                first_places[tile as usize] = self.descending_scores.len() as u8;
                let score = alphabet.scaled_score(tile).raw();
                for _ in 0..count {
                    self.descending_scores.push(score);
                }
            }
        }

        let setup = if self.num_tiles_in_bag > 0
            && dynamic_leaves.is_none()
            && matches!(adjust_leave_value, klv::AdjustLeave::Identity)
        {
            board_snapshot
                .klv
                .racks()
                .and_then(|racks| racks.get(&self.rack_tally))
        } else {
            None
        };
        let saved_shape = setup.and_then(|(_, order)| {
            let rack_size = self.num_tiles_on_rack as usize;
            if (1..=RACK_SHAPE_MAX).contains(&rack_size) && board_snapshot.anagrams.is_some() {
                let shape = rack_shape(&self.rack_tally, self.rack_bits, rack_size);
                (shape.leave_indices.len() == order.len()).then_some(shape)
            } else {
                None
            }
        });
        if self.num_tiles_in_bag <= 0 {
            if !LEAVES {
                self.multi_leaves
                    .init_digits(&self.rack_tally, dense_most, std::iter::empty());
            } else {
                self.multi_leaves.init_validated(
                    &self.rack_tally,
                    board_snapshot.klv,
                    false,
                    adjust_leave_value,
                    dense_most,
                    std::iter::empty(),
                );
                if self.multi_leaves.is_dense() {
                    self.multi_leaves
                        .init_endgame_leaves(|tile| alphabet.score(tile), play_out_bonus);
                }
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
            // the bag is not empty here
            let bag = self.num_tiles_in_bag;
            let on_rack = i32::from(self.num_tiles_on_rack);
            let bag_terms = (if BAG_TERM {
                self.bag_term.as_ref()
            } else {
                None
            })
            .into_iter()
            .flat_map(move |bag_term| {
                (0..=on_rack).map(move |num_played| {
                    if num_played == 0 {
                        0
                    } else {
                        bag_term.at_wide(bag - num_played)
                    }
                })
            });
            let saved_bounds = saved_shape.filter(|_| bag_terms.clone().all(|term| term == 0));
            if let (Some((leaves, order)), Some(shape)) = (setup, saved_bounds) {
                self.multi_leaves.init_from_saved_leaves::<false>(
                    &self.rack_tally,
                    leaves,
                    dense_most,
                    bag_terms,
                );
                self.best_leave_values.clear();
                let values = self.multi_leaves.leave_values();
                self.best_leave_values.extend(
                    shape.by_played[..=self.num_tiles_on_rack as usize]
                        .iter()
                        .map(|&start| {
                            values[shape.leave_indices[order[start as usize] as usize] as usize]
                        }),
                );
            } else {
                match setup {
                    Some((leaves, _)) => self.multi_leaves.init_from_leaves(
                        &self.rack_tally,
                        leaves,
                        dense_most,
                        bag_terms,
                    ),
                    None => {
                        if let Some((tables, pair)) = other_leaves {
                            if tables.same_graph() {
                                self.multi_leaves.init_shared_graph(
                                    &self.rack_tally,
                                    tables,
                                    &mut pair.values,
                                    dense_most,
                                );
                            } else {
                                self.multi_leaves.init_validated(
                                    &self.rack_tally,
                                    board_snapshot.klv,
                                    true,
                                    adjust_leave_value,
                                    dense_most,
                                    bag_terms.clone(),
                                );
                                pair.alternate.init_validated(
                                    &self.rack_tally,
                                    tables.table(1),
                                    true,
                                    adjust_leave_value,
                                    dense_most,
                                    bag_terms,
                                );
                                self.multi_leaves
                                    .bound_two_tables(&pair.alternate, &mut pair.values[0]);
                                pair.values[1].clear();
                                pair.values[1].extend_from_slice(pair.alternate.leave_values());
                            }
                        } else {
                            self.multi_leaves.init_validated(
                                &self.rack_tally,
                                board_snapshot.klv,
                                true,
                                adjust_leave_value,
                                dense_most,
                                bag_terms,
                            );
                        }
                    }
                }
                if self.multi_leaves.is_dense() {
                    if let Some(dyn_ref) = dynamic_leaves {
                        let n = dyn_ref.lat.num_letters();
                        let mut live_pool = [0u8; MAX_ALPHABET_LEN];
                        fill_live_pool(
                            &mut live_pool[..n],
                            alphabet,
                            board_snapshot.board_tiles,
                            &self.rack_tally,
                        );
                        self.multi_leaves.apply_dynamic_leaves_kept(
                            &dyn_ref,
                            &live_pool[..n],
                            self.num_tiles_in_bag.max(0) as usize,
                            &mut self.dynamic_kept,
                        );
                    }
                    // each placement's leave with its bag term, so every bound read
                    // from the leaves has it too
                    if BAG_TERM {
                        self.multi_leaves.add_bag_terms();
                    }
                    self.multi_leaves
                        .extract_raw_best_leave_values(&mut self.best_leave_values);
                } else {
                    klv::MultiLeaves::extract_best_validated(
                        &mut self.rack_tally,
                        board_snapshot.klv,
                        self.num_tiles_on_rack,
                        adjust_leave_value,
                        &mut self.best_leave_values,
                    );
                    if BAG_TERM {
                        for (num_played, v) in self.best_leave_values.iter_mut().enumerate() {
                            *v += self.multi_leaves.bag_term(num_played);
                        }
                    }
                }
            }
        }
        for i in 0..=self.num_tiles_on_rack {
            self.best_leave_values[i as usize] +=
                i32::from(board_snapshot.game_config.num_played_bonus(i)) * equity::SCALE;
        }
        if let (true, Some(places)) = (
            board_snapshot.anagrams.is_some() && self.multi_leaves.is_dense(),
            board_snapshot.anagrams.map(T::of),
        ) {
            build_subracks(
                &self.multi_leaves,
                places,
                (&self.rack_tally, &first_places, self.num_tiles_on_rack),
                &mut self.subracks,
                &mut self.subracks_by_played,
                (&mut self.subrack_order, &mut self.subracks_spare),
                (setup.map(|(_, order)| order), saved_shape),
            );
        } else {
            self.subracks.clear();
            self.subracks_by_played.clear();
        }
        self.used_letters_tally.clear();
        match board_snapshot.game_config.game_rules() {
            game_config::GameRules::Classic | game_config::GameRules::Woadwage => {}
            game_config::GameRules::Jumbled | game_config::GameRules::JumbledWoadwage => {
                self.used_letters_tally.resize(alphabet.len() as usize, 0);
            }
        }
        self.used_tile_scores_shadowl.clear();
        self.used_tile_scores_shadowr.clear();
    }

    // an empty board's star takes every letter and is an anchor, so it is
    // given as a cross set of every letter with no cross word to score
    #[inline]
    fn activate_star(&mut self, board_layout: &crate::board_layout::BoardLayout) {
        let star_row = board_layout.star_row();
        let star_col = board_layout.star_col();
        let dim = board_layout.dim();
        let transposed_dim = matrix::Dim {
            rows: dim.cols,
            cols: dim.rows,
        };
        let idx = dim.at_on_board(star_row, star_col);
        self.allowed_for_across_plays[idx] = u64::MAX;
        self.perpendicular_word_multipliers_for_across_plays[idx] = 0;
        self.perpendicular_scores_for_across_plays[idx] = 0;
        if !board_layout.is_symmetric() {
            let idx = transposed_dim.at_on_board(star_col, star_row);
            self.allowed_for_down_plays[idx] = u64::MAX;
            self.perpendicular_word_multipliers_for_down_plays[idx] = 0;
            self.perpendicular_scores_for_down_plays[idx] = 0;
        }
    }

    #[inline]
    fn init_after_cross_sets<const D: usize, N: kwg::Node, L: kwg::Node>(
        &mut self,
        board_snapshot: &BoardSnapshot<'_, N, L>,
        dirty_cols: G,
        dirty_rows: G,
    ) {
        let board_layout = board_snapshot.game_config.board_layout();
        let fixed;
        let dim = if D != 0 {
            assert!(board_layout.dim().rows as usize == D && board_layout.dim().cols as usize == D);
            fixed = square_dim::<D>();
            &fixed
        } else {
            board_layout.dim()
        };
        if dirty_cols != G::from(0) {
            let premiums = board_layout.premiums();

            for col in 0..dim.cols {
                if dirty_cols & (G::from(1) << col as u32) == G::from(0) {
                    continue;
                }
                let strider = dim.down(col);
                for i in 0..strider.len() {
                    let idx = strider.at_in_lane(i);
                    let premium = &premiums[idx];
                    let allowed = &mut self.allowed_for_across_plays[idx];
                    if premium.word_multiplier == 0 && premium.tile_multiplier == 0 {
                        *allowed = 1;
                    }
                    let effective_pwm = self.remaining_word_multipliers_for_across_plays[idx]
                        & -(*allowed as i8 & 1);
                    self.perpendicular_word_multipliers_for_across_plays[idx] = effective_pwm;
                    self.perpendicular_scores_for_across_plays[idx] =
                        self.cross_score_for_across_plays[idx] * i32::from(effective_pwm);
                }
            }
        }
        if dirty_rows != G::from(0) {
            let transposed_dim = matrix::Dim {
                rows: dim.cols,
                cols: dim.rows,
            };
            let transposed_premiums = board_layout.transposed_premiums();

            for row in 0..dim.rows {
                if dirty_rows & (G::from(1) << row as u32) == G::from(0) {
                    continue;
                }
                let strider = transposed_dim.down(row);
                for i in 0..strider.len() {
                    let idx = strider.at_in_lane(i);
                    let premium = &transposed_premiums[idx];
                    let allowed = &mut self.allowed_for_down_plays[idx];
                    if premium.word_multiplier == 0 && premium.tile_multiplier == 0 {
                        *allowed = 1;
                    }
                    let effective_pwm =
                        self.remaining_word_multipliers_for_down_plays[idx] & -(*allowed as i8 & 1);
                    self.perpendicular_word_multipliers_for_down_plays[idx] = effective_pwm;
                    self.perpendicular_scores_for_down_plays[idx] =
                        self.cross_score_for_down_plays[idx] * i32::from(effective_pwm);
                }
            }
        }
    }

    // a lane's span sums (only the anagram tables' path reads them), filled
    // once a generation, when a span of it is read: where the sums start in
    // span_prefix, and the lane's index in span_lanes. Sums kept from an
    // earlier generation are taken again when the table walk has checked its
    // lanes this generation and found this one as it kept it (rcs_placements),
    // the sums filled while it was so, by the same tables and keys (source)
    #[inline(always)]
    fn fill_span_lane<const D: usize, W: LaneBits, P: alphagram::KeyPlaces>(
        &mut self,
        board_tiles: &[u8],
        dim: &matrix::Dim,
        places: P,
        source: usize,
        down: bool,
        lane: i8,
    ) -> (usize, usize) {
        let rows = dim.rows as usize;
        let cols = dim.cols as usize;
        let lane = lane as usize;
        let (lane_at, len, start, at) = if down {
            (
                rows + lane,
                if D == 0 { rows } else { D },
                lane * rows,
                rows * (cols + 1) + lane * (rows + 1),
            )
        } else {
            (
                lane,
                if D == 0 { cols } else { D },
                lane * cols,
                lane * (cols + 1),
            )
        };
        if self.span_lanes[lane_at].turn == self.span_turn {
            return (at, lane_at);
        }
        if self.span_source != source {
            for span_lane in self.span_lanes.iter_mut() {
                span_lane.kept = false;
            }
            self.span_source = source;
        }
        // kept is set only in a generation the table walk looked in (below),
        // and cleared at its look for a lane it found changed
        if self.span_lanes[lane_at].kept {
            self.span_lanes[lane_at].turn = self.span_turn;
            return (at, lane_at);
        }
        let (board_strip, allowed, face_values, perpendicular_scores, word_multipliers) = if down {
            (
                &self.transposed_board_tiles[start..start + len],
                &self.allowed_for_down_plays[start..start + len],
                &self.face_value_scores_for_down_plays[start..start + len],
                &self.perpendicular_scores_for_down_plays[start..start + len],
                &self.remaining_word_multipliers_for_down_plays[start..start + len],
            )
        } else {
            (
                &board_tiles[start..start + len],
                &self.allowed_for_across_plays[start..start + len],
                &self.face_value_scores_for_across_plays[start..start + len],
                &self.perpendicular_scores_for_across_plays[start..start + len],
                &self.remaining_word_multipliers_for_across_plays[start..start + len],
            )
        };
        let out = &mut self.span_prefix[at..at + len + 1];
        let mut sum = SpanPrefix::new();
        let mut empty = 0u128;
        let mut multiplied = 0u128;
        let narrow = W::BITS == u16::BITS;
        let mut empty16 = 0u16;
        let mut multiplied16 = 0u16;
        let mid = W::BITS == u32::BITS;
        let mut empty32 = 0u32;
        let mut multiplied32 = 0u32;
        out[0] = sum;
        for j in 0..len {
            let b = board_strip[j];
            if b != 0 {
                sum.face += face_values[j];
                sum.key += places.place_value(b & 0x7f);
            } else {
                let bits = allowed[j];
                sum.free += u8::from(bits & 1 == 0);
                sum.dead += u8::from(bits == 1);
                sum.played += 1;
                sum.perp += perpendicular_scores[j];
                if narrow {
                    empty16 |= 1u16 << j;
                    multiplied16 |= u16::from(word_multipliers[j] != 1) << j;
                } else if mid {
                    empty32 |= 1u32 << j;
                    multiplied32 |= u32::from(word_multipliers[j] != 1) << j;
                } else {
                    empty |= 1u128 << j;
                    multiplied |= u128::from(word_multipliers[j] != 1) << j;
                }
            }
            out[j + 1] = sum;
        }
        if narrow {
            empty = u128::from(empty16);
            multiplied = u128::from(multiplied16);
        } else if mid {
            empty = u128::from(empty32);
            multiplied = u128::from(multiplied32);
        }
        self.span_lanes[lane_at] = SpanLane {
            turn: self.span_turn,
            kept: self.span_look_turn == self.span_turn,
            empty,
            multiplied,
        };
        (at, lane_at)
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
        // a strip's walk keeps, by the tile on each square, the kwg's node for
        // the tiles from there to the right; no tile matches an empty square's
        for computation in (self.cross_set_buffer_for_across_plays.iter_mut())
            .chain(self.cross_set_buffer_for_down_plays.iter_mut())
        {
            computation.b_letter = 0;
        }
        self.prev_board_tiles.fill(0xff);
        for lane in self.lane_scaffold.iter_mut() {
            lane.valid = false;
        }
    }
}

// kwg must be Gaddawg for Classic and Woadwage, AlphaDawg for Jumbled and
// JumbledWoadwage.
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
    allowed: &'a mut [u64],
    cross_scores: &'a mut [i32],
    output_strider: matrix::Strider,
    cross_set_buffer: &'a mut [CrossSetComputation],
    cached_cross_sets: &'a mut [CachedCrossSet],
) {
    let sibling_bits = |kwg: &kwg::Kwg<N>, mut p: i32, accepting_only: bool| -> u64 {
        let mut bits = 0u64;
        if p > 0 {
            loop {
                let node = kwg.node_in_run(p);
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
                    score += alphabet.scaled_score(b).raw();
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
                    let arc = kwg.node_in_run(p).arc_index();
                    bits |= sibling_bits(kwg, arc, true);
                }
                cached_cross_sets[j as usize - 1].bits = bits;
            }
            for i in wi..j - 1 {
                allowed[wp] = unhooked_letters(board_strip[i as usize]);
                cross_scores[wp] = 0;
                wp += step;
            }
            allowed[wp] = bits;
            cross_scores[wp] = cross_set_buffer[j as usize].score;
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
                    let arc_right = kwg.node_in_run(p_right).arc_index();
                    let arc_left = kwg.node_in_run(p_left).arc_index();
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
                                        q = kwg
                                            .seek(q, cross_set_buffer[qi as usize].b_letter & 0x7f);
                                        if q <= 0 {
                                            break;
                                        }
                                    }
                                    if q > 0 {
                                        bits |= u64::from(kwg.node_in_run(q).accepts()) << tile;
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
                                        q = kwg
                                            .seek(q, cross_set_buffer[qi as usize].b_letter & 0x7f);
                                        if q <= 0 {
                                            break;
                                        }
                                    }
                                    if q > 0 {
                                        bits |= u64::from(kwg.node_in_run(q).accepts()) << tile;
                                    }
                                }
                            }
                        }
                    }
                }
                cached_cross_sets[j as usize - 1].bits = bits;
            }
            for i in wi..j - 1 {
                allowed[wp] = unhooked_letters(board_strip[i as usize]);
                cross_scores[wp] = 0;
                wp += step;
            }
            allowed[wp] = bits;
            cross_scores[wp] =
                cross_set_buffer[prev_j as usize].score + cross_set_buffer[j as usize].score;
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
                let arc = kwg.node_in_run(p).arc_index();
                bits |= sibling_bits(kwg, arc, true);
            }
            cached_cross_sets[j as usize].bits = bits;
        }
        for i in wi..j {
            allowed[wp] = unhooked_letters(board_strip[i as usize]);
            cross_scores[wp] = 0;
            wp += step;
        }
        allowed[wp] = bits;
        cross_scores[wp] = cross_set_buffer[prev_j as usize].score;
        wi = j + 1;
        wp += step;
        j = cross_set_buffer[j as usize].end_range;
    }
    for i in wi..len {
        allowed[wp] = unhooked_letters(board_strip[i as usize]);
        cross_scores[wp] = 0;
        wp += step;
    }
}

// jumbled only: its lone island reuse holds for one jumbled word, so the
// caller never gives it a jumbled woadwage game
#[inline]
fn gen_jumbled_cross_set<'a, N: kwg::Node, L: kwg::Node>(
    board_snapshot: &'a BoardSnapshot<'a, N, L>,
    board_strip: &'a [u8],
    allowed: &'a mut [u64],
    cross_scores: &'a mut [i32],
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
            allowed[wp] = unhooked_letters(b);
            cross_scores[wp] = 0;
        } else if prev_wp != !0 && (i + 1 >= len || board_strip[i as usize + 1] == 0) {
            // this is the matching right side of a lone island.
            // reuse the computed left side's cross set.
            allowed[wp] = allowed[prev_wp];
            cross_scores[wp] = cross_scores[prev_wp];
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
                score += alphabet.scaled_score(b).raw();
                used_letters_tally[(b & 0x7f) as usize] += 1;
            }
            let mut k = i + 1;
            while k < len {
                let b = board_strip[k as usize];
                if b == 0 {
                    break;
                }
                k += 1;
                score += alphabet.scaled_score(b).raw();
                used_letters_tally[(b & 0x7f) as usize] += 1;
            }
            if k == j + 1 {
                allowed[wp] = !1;
                cross_scores[wp] = 0;
            } else {
                allowed[wp] =
                    kwg.compute_alpha_cross_set_of_len(used_letters_tally, alphabet.len());
                cross_scores[wp] = score;
                // if j == i, this is the left side of a possible lone island.
                // otherwise set to !0.
                prev_wp = wp | (isize::from(j == i) - 1) as usize;
                used_letters_tally.iter_mut().for_each(|m| *m = 0);
            }
        }
        wp += step;
    }
}

struct CrossSetOutput<'a> {
    allowed: &'a mut [u64],
    cross_scores: &'a mut [i32],
    output_strider: matrix::Strider,
}

// nibble keys come with classic games only
#[inline(always)]
fn gen_cross_set<'a, B: Keys, N: kwg::Node, L: kwg::Node>(
    board_snapshot: &'a BoardSnapshot<'a, N, L>,
    board_strip: &'a [u8],
    CrossSetOutput {
        allowed,
        cross_scores,
        output_strider,
    }: CrossSetOutput<'a>,
    cross_set_buffer: &'a mut [CrossSetComputation],
    cached_cross_sets: &'a mut [CachedCrossSet],
    used_letters_tally: &'a mut [u8],
    woadwage_stack: &'a mut Vec<i32>,
) {
    if B::NIBBLES {
        return gen_classic_cross_set(
            board_snapshot,
            board_strip,
            allowed,
            cross_scores,
            output_strider,
            cross_set_buffer,
            cached_cross_sets,
        );
    }
    match board_snapshot.game_config.game_rules() {
        game_config::GameRules::Classic => gen_classic_cross_set(
            board_snapshot,
            board_strip,
            allowed,
            cross_scores,
            output_strider,
            cross_set_buffer,
            cached_cross_sets,
        ),
        game_config::GameRules::Jumbled => gen_jumbled_cross_set(
            board_snapshot,
            board_strip,
            allowed,
            cross_scores,
            output_strider,
            used_letters_tally,
        ),
        game_config::GameRules::Woadwage => gen_woadwage_cross_set(
            board_snapshot,
            board_strip,
            allowed,
            cross_scores,
            output_strider,
            woadwage_stack,
        ),
        game_config::GameRules::JumbledWoadwage => gen_jumbled_woadwage_cross_set(
            board_snapshot,
            board_strip,
            allowed,
            cross_scores,
            output_strider,
        ),
    }
}

#[inline]
fn gen_jumbled_woadwage_cross_set<'a, N: kwg::Node, L: kwg::Node>(
    board_snapshot: &'a BoardSnapshot<'a, N, L>,
    board_strip: &'a [u8],
    allowed: &'a mut [u64],
    cross_scores: &'a mut [i32],
    output_strider: matrix::Strider,
) {
    let len = output_strider.len() as usize;
    let step = output_strider.step() as usize;
    let mut wp = output_strider.base() as usize;
    let kwg = board_snapshot.kwg;
    let alphabet = board_snapshot.game_config.alphabet();
    let alphabet_len = alphabet.len() as usize;
    let tally_of = |tiles: &[u8], more: &[u8]| {
        let mut tally = [0u8; MAX_ALPHABET_LEN];
        for &t in tiles.iter().chain(more) {
            tally[(t & 0x7f) as usize] += 1;
        }
        tally
    };
    for i in 0..len {
        let b = board_strip[i];
        if b != 0 {
            allowed[wp] = unhooked_letters(b);
            cross_scores[wp] = 0;
            wp += step;
            continue;
        }
        let mut j = i;
        while j > 0 && board_strip[j - 1] != 0 {
            j -= 1;
        }
        let mut k = i + 1;
        while k < len && board_strip[k] != 0 {
            k += 1;
        }
        if j == i && k == i + 1 {
            allowed[wp] = !1;
            cross_scores[wp] = 0;
            wp += step;
            continue;
        }
        let before = &board_strip[j..i];
        let after = &board_strip[i + 1..k];
        let mut score = 0i32;
        for &t in before.iter().chain(after) {
            score += alphabet.scaled_score(t).raw();
        }
        let mut bits = 1u64;
        for m in 0..=before.len() {
            if m == 0
                || kwg.accepts_alpha_of_len(
                    &tally_of(&before[..m], &[])[..alphabet_len],
                    alphabet_len as u8,
                )
            {
                bits |= kwg.compute_alpha_cross_set_of_len(
                    &tally_of(&before[m..], after)[..alphabet_len],
                    alphabet_len as u8,
                );
            }
        }
        for m in 0..after.len() {
            if kwg.accepts_alpha_of_len(
                &tally_of(&after[m..], &[])[..alphabet_len],
                alphabet_len as u8,
            ) {
                bits |= kwg.compute_alpha_cross_set_of_len(
                    &tally_of(before, &after[..m])[..alphabet_len],
                    alphabet_len as u8,
                );
            }
        }
        allowed[wp] = bits;
        cross_scores[wp] = score;
        wp += step;
    }
}

#[inline]
fn gen_woadwage_cross_set<'a, N: kwg::Node, L: kwg::Node>(
    board_snapshot: &'a BoardSnapshot<'a, N, L>,
    board_strip: &'a [u8],
    allowed: &'a mut [u64],
    cross_scores: &'a mut [i32],
    output_strider: matrix::Strider,
    stack: &'a mut Vec<i32>,
) {
    let len = output_strider.len() as usize;
    let step = output_strider.step() as usize;
    let mut wp = output_strider.base() as usize;
    let kwg = board_snapshot.kwg;
    let alphabet = board_snapshot.game_config.alphabet();
    for i in 0..len {
        let b = board_strip[i];
        if b != 0 {
            allowed[wp] = unhooked_letters(b);
            cross_scores[wp] = 0;
            wp += step;
            continue;
        }
        let mut j = i;
        while j > 0 && board_strip[j - 1] != 0 {
            j -= 1;
        }
        let mut k = i + 1;
        while k < len && board_strip[k] != 0 {
            k += 1;
        }
        if j == i && k == i + 1 {
            allowed[wp] = unhooked_letters(0);
            cross_scores[wp] = 0;
            wp += step;
            continue;
        }
        let mut score = 0i32;
        stack.clear();
        let mut prefix = Some(woadwage::StackedStep::new());
        for &t in &board_strip[j..i] {
            score += alphabet.scaled_score(t).raw();
            prefix = prefix.and_then(|state| state.step(kwg, stack, t & 0x7f));
        }
        for &t in &board_strip[i + 1..k] {
            score += alphabet.scaled_score(t).raw();
        }
        let mut bits = 1u64;
        if let Some(prefix) = prefix {
            let mark = stack.len();
            for letter in 1..alphabet.len() {
                let mut state = prefix.step(kwg, stack, letter);
                for &t in &board_strip[i + 1..k] {
                    state = state.and_then(|state| state.step(kwg, stack, t & 0x7f));
                }
                if state.is_some_and(|state| state.accepts(kwg, stack)) {
                    bits |= 1u64 << letter;
                }
                stack.truncate(mark);
            }
        }
        allowed[wp] = bits;
        cross_scores[wp] = score;
        wp += step;
    }
}

// what the shadow asks of a span through board tiles before it keeps it: under
// the tables it asks their key filter, under the lean arm nothing (its check is
// (), and compiles away)
trait SpanCheck {
    fn refuses(&mut self, idx_left: i8, idx_right: i8, num_played: u8) -> bool;
}

impl SpanCheck for () {
    #[inline(always)]
    fn refuses(&mut self, _idx_left: i8, _idx_right: i8, _num_played: u8) -> bool {
        false
    }
}

// a span through board tiles holds a word only if the key filter holds some
// subrack of its size plus the tiles it crosses; the answer kept a generation
// by (the crossed tiles' key, the tiles played). Asked only of a span that
// plays min_played tiles or more
struct KeyCheck<'a> {
    source: &'a anagrams::Anagrams,
    subracks: &'a [Subrack],
    by_played: &'a [u32],
    prefix: &'a [SpanPrefix], // the lane's
    turn: u32,
    memo: &'a mut [ExistSlot],
    min_played: u8, // EXIST_MIN_PLAYED, or the whole rack when it holds fewer
}

const EXIST_MEMO_SLOTS: usize = 4096;
const EXIST_MIN_PLAYED: u8 = 5;

#[derive(Clone)]
struct ExistSlot {
    key: u128,
    turn: u32,
    played: u8,
    held: bool,
}

impl ExistSlot {
    #[inline(always)]
    fn new() -> Self {
        Self {
            key: 0,
            turn: 0,
            played: 0,
            held: false,
        }
    }
}

impl SpanCheck for Option<KeyCheck<'_>> {
    #[inline(always)]
    fn refuses(&mut self, idx_left: i8, idx_right: i8, num_played: u8) -> bool {
        let Some(check) = self.as_mut() else {
            return false;
        };
        if num_played < check.min_played {
            return false;
        }
        let crossed = check.prefix[idx_right as usize].key - check.prefix[idx_left as usize].key;
        let crossed_part = anagrams::key_part(crossed);
        let at = (anagrams::part_hash(crossed_part) ^ u64::from(num_played)) as usize
            & (EXIST_MEMO_SLOTS - 1);
        let slot = &mut check.memo[at];
        if slot.turn != check.turn || slot.key != crossed || slot.played != num_played {
            let from = check.by_played[num_played as usize] as usize;
            let upto = check.by_played[num_played as usize + 1] as usize;
            let held = check.subracks[from..upto].iter().any(|subrack| {
                check
                    .source
                    .may_hold_part(crossed_part.wrapping_add(subrack.part))
            });
            *slot = ExistSlot {
                key: crossed,
                turn: check.turn,
                played: num_played,
                held,
            };
        }
        !slot.held
    }
}

struct GenPlacePlacementsParams<'a, C: SpanCheck = ()> {
    board_strip: &'a [u8],
    alphabet: &'a alphabet::Alphabet,
    rack_tally: &'a mut [u8],
    used_tile_scores_shadowl: &'a mut ShadowScores,
    used_tile_scores_shadowr: &'a mut ShadowScores,
    allowed_strip: &'a [u64],
    remaining_word_multipliers_strip: &'a [i8],
    remaining_tile_multipliers_strip: &'a [i8],
    perpendicular_word_multipliers_strip: &'a [i8],
    perpendicular_scores_strip: &'a [i32],
    rack_bits: u64,
    feasible_lengths: u64,
    descending_scores: &'a [i32],
    lane_scaffold: &'a mut LaneScaffold,
    room: &'a endgame::Room,
    best_leave_values: &'a [i32],
    span_out: &'a mut Vec<(i8, i8, u8, i32)>,
    per_span: bool,
    num_max_played: u8,
    rack_tally_shadowl: &'a mut [u8],
    rack_tally_shadowr: &'a mut [u8],
    left_bounds: &'a mut Vec<i32>,
    bounds_stride: usize,
    check: C,
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

// one copy for each kind of keys (T), as bucket_by_equity
#[inline]
fn feasible_lengths_of_subracks<T: TablePlaces>(
    anagrams: Option<&anagrams::Anagrams>,
    rack_lengths: Option<&anagrams::RackLengths>,
    rack_tally: &[u8],
    subracks: &[Subrack],
    by_played: &[u32],
) -> u64 {
    let Some(anagrams) = anagrams else {
        return feasible_word_lengths::<true>(anagrams, rack_lengths, rack_tally);
    };
    if rack_tally[0] > 0 {
        return !0;
    }
    let mut found = 3u64;
    for len in 2..by_played.len() - 1 {
        let from = by_played[len] as usize;
        let upto = by_played[len + 1] as usize;
        if subracks[from..upto].iter().any(|subrack| {
            anagrams
                .words(alphagram::Fitted(subrack.key), len as u8)
                .is_some()
        }) {
            found |= 1 << len;
        }
    }
    found
}

// the board of an instance compiled for a square board of side D (D is not 0)
#[inline(always)]
fn square_dim<const D: usize>() -> matrix::Dim {
    matrix::Dim {
        rows: D as i8,
        cols: D as i8,
    }
}

mod lane_bits {
    pub trait Sealed {}

    impl Sealed for u16 {}

    impl Sealed for u32 {}

    impl Sealed for u128 {}
}

// the set type an instance takes for a lane's squares, bit i for square i: u16
// for a side of at most 16, u32 for one of at most 32, u128 for any; and a
// generator's for its sets of rows and of columns (KurniaMoveGenerator's G)
pub trait LaneBits:
    lane_bits::Sealed
    + Copy
    + Send
    + Eq
    + From<u8>
    + std::ops::BitAnd<Output = Self>
    + std::ops::BitOrAssign
    + std::ops::Not<Output = Self>
    + std::ops::Shl<u32, Output = Self>
{
    const BITS: u32;

    // x's lowest bits, as many as the type holds
    fn of_u128(x: u128) -> Self;
}

impl LaneBits for u16 {
    const BITS: u32 = u16::BITS;

    #[inline(always)]
    fn of_u128(x: u128) -> Self {
        x as u16
    }
}

impl LaneBits for u32 {
    const BITS: u32 = u32::BITS;

    #[inline(always)]
    fn of_u128(x: u128) -> Self {
        x as u32
    }
}

impl LaneBits for u128 {
    const BITS: u32 = u128::BITS;

    #[inline(always)]
    fn of_u128(x: u128) -> Self {
        x
    }
}

// what an instance's keys are: nibbles (a classic game's, on a square board of
// the instance's side) or not
trait Keys {
    const NIBBLES: bool;
}

// the lean arm's, which reads no tables
struct NoTables;

impl Keys for NoTables {
    const NIBBLES: bool = false;
}

// the key places the table walk reads its anagrams' keys by: their layout or
// nibbles
trait TablePlaces: Keys {
    type Places<'x>: alphagram::KeyPlaces;
    fn of(anagrams: &anagrams::Anagrams) -> Self::Places<'_>;
}

// keys by the tables' layout
struct ByLayout;

impl Keys for ByLayout {
    const NIBBLES: bool = false;
}

impl TablePlaces for ByLayout {
    type Places<'x> = &'x alphagram::KeyLayout;

    #[inline(always)]
    fn of(anagrams: &anagrams::Anagrams) -> &alphagram::KeyLayout {
        anagrams.layout()
    }
}

// the tables and their keys' kind, as fill_span_lane tells kept sums apart: the
// tables' address, its low bit (an Anagrams is aligned past a byte) for nibble
// keys
#[inline(always)]
fn span_source<T: TablePlaces>(anagrams: &anagrams::Anagrams) -> usize {
    std::ptr::from_ref(anagrams) as usize | usize::from(T::NIBBLES)
}

// keys by nibbles
struct ByNibble;

impl Keys for ByNibble {
    const NIBBLES: bool = true;
}

impl TablePlaces for ByNibble {
    type Places<'x> = alphagram::Nibble;

    #[inline(always)]
    fn of(_anagrams: &anagrams::Anagrams) -> alphagram::Nibble {
        alphagram::Nibble
    }
}

// the lane's span sums filled, when a span through board tiles can be refused
// by the key filter at shadow time (KeyCheck): the unblanked classic game under
// the tables, whose memo is made at its first use
#[inline(always)]
fn fill_key_check<
    'a,
    const D: usize,
    W: LaneBits,
    T: TablePlaces,
    N: kwg::Node,
    L: kwg::Node,
    G: LaneBits,
    const BAG_TERM: bool,
    S: BoardStore,
>(
    working_buffer: &mut WorkingBuffer<G, BAG_TERM, S>,
    board_snapshot: &BoardSnapshot<'a, N, L>,
    dim: &matrix::Dim,
    down: bool,
    lane: i8,
) -> Option<(&'a anagrams::Anagrams, usize)> {
    let source = board_snapshot.anagrams?;
    let classic = match board_snapshot.game_config.game_rules() {
        game_config::GameRules::Classic => true,
        game_config::GameRules::Jumbled
        | game_config::GameRules::JumbledWoadwage
        | game_config::GameRules::Woadwage => false,
    };
    if !classic || working_buffer.rack_tally[0] != 0 || working_buffer.subracks.is_empty() {
        return None;
    }
    if working_buffer.exist_memo.is_empty() {
        working_buffer.exist_memo = vec![ExistSlot::new(); EXIST_MEMO_SLOTS].into_boxed_slice();
    }
    let (at, _) = working_buffer.fill_span_lane::<D, W, _>(
        board_snapshot.board_tiles,
        dim,
        T::of(source),
        span_source::<T>(source),
        down,
        lane,
    );
    Some((source, at))
}

#[inline]
fn gen_place_placements<
    'a,
    C: SpanCheck,
    PossibleStripPlacementCallbackType: FnMut(i8, i8, i8, i32, u8),
>(
    params: &'a mut GenPlacePlacementsParams<'a, C>,
    single_tile_plays: bool,
    want_raw: bool,
    possible_strip_placement_callback: PossibleStripPlacementCallbackType,
) {
    if params.per_span {
        gen_place_placements_impl::<true, C, _>(
            params,
            single_tile_plays,
            want_raw,
            possible_strip_placement_callback,
        )
    } else {
        gen_place_placements_impl::<false, C, _>(
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
    C: SpanCheck,
    PossibleStripPlacementCallbackType: FnMut(i8, i8, i8, i32, u8),
>(
    params: &'a mut GenPlacePlacementsParams<'a, C>,
    single_tile_plays: bool,
    want_raw: bool,
    mut possible_strip_placement_callback: PossibleStripPlacementCallbackType,
) {
    let strider_len = params.board_strip.len();

    if !want_raw
        && !params.lane_scaffold.refresh::<0>(
            params.room,
            params.alphabet,
            params.board_strip,
            params.remaining_word_multipliers_strip,
            params.remaining_tile_multipliers_strip,
            params.perpendicular_word_multipliers_strip,
        )
    {
        return;
    }

    struct Env<'a, C: SpanCheck> {
        params: &'a mut GenPlacePlacementsParams<'a, C>,
        strider_len: usize,
        anchor: i8,
        leftmost: i8,
        rightmost: i8,
        best_possible_equity: i32,
        bounds_at: usize,
    }

    let mut env = Env {
        params,
        strider_len,
        anchor: 0,
        leftmost: 0,
        rightmost: 0,
        best_possible_equity: i32::MIN,
        bounds_at: 0,
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
    fn ranked_from<C: SpanCheck>(env: &Env<'_, C>, deferred: u128, low_end: usize) -> u128 {
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
    fn shadow_record<const PER_SPAN: bool, C: SpanCheck>(
        env: &mut Env<'_, C>,
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
        if acc.crossed_board_tiles && env.params.check.refuses(idx_left, idx_right, num_played) {
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
        } else {
            let at = &mut env.params.left_bounds[env.bounds_at + idx_left as usize];
            *at = (*at).max(equity);
        }
        if equity > env.best_possible_equity {
            env.best_possible_equity = equity;
        }
    }

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
    fn shadow_play_right<const PER_SPAN: bool, C: SpanCheck>(
        env: &mut Env<'_, C>,
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
            if idx > env.anchor + 1 && num_played > u8::from(!is_unique) && idx - idx_left >= 2 {
                shadow_record::<PER_SPAN, C>(env, &acc, idx_left, idx, num_played, low_end, ranked);
            }
            if num_played >= env.params.num_max_played {
                break;
            }

            if idx >= env.rightmost {
                break;
            }

            // place a tile at [idx] since it is still in bounds.
            let this_cross_bits = env.params.allowed_strip[idx as usize];
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
                        & (-i64::from(env.params.rack_tally_shadowr[tile as usize] == 0)) as u64;
                    // fall-through to case 2 (assume the optimized asm does not recheck the condition).
                }
                if matching_bits.is_power_of_two()
                    || matching_bits & env.params.alphabet.same_score_tile_bits(tile)
                        == matching_bits
                {
                    // case 2: multiple tiles fit, but they all have the same score.
                    // consume the square, but not the tile.
                    // rack_bits remains unchanged because assignment is tentative.
                    let tile_score = env.params.alphabet.scaled_score(tile).raw();
                    env.params
                        .used_tile_scores_shadowr
                        .insert_sorted(tile_score);
                    let tile_value = tile_score
                        * i32::from(env.params.remaining_tile_multipliers_strip[idx as usize]);
                    acc.main_score += tile_value;
                    acc.perpendicular_cumulative_score += env.params.perpendicular_scores_strip
                        [idx as usize]
                        + tile_value
                            * i32::from(
                                env.params.perpendicular_word_multipliers_strip[idx as usize],
                            );
                } else {
                    // case 3: multiple tiles fit, and they have different scores.
                    // rack_bits remains unchanged because assignment is tentative.
                    // defer to greedy algorithm.
                    let mut remaining = matching_bits;
                    let mut admissible_max = i32::MIN;
                    while remaining != 0 {
                        let t = remaining.trailing_zeros() as u8;
                        remaining &= remaining - 1;
                        admissible_max =
                            admissible_max.max(env.params.alphabet.scaled_score(t).raw());
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
            let word_multiplier =
                i32::from(env.params.remaining_word_multipliers_strip[idx as usize]);
            if word_multiplier != 1 {
                acc.word_multiplier *= word_multiplier;
                low_end = env.params.lane_scaffold.ranking(
                    acc.word_multiplier,
                    env.params.board_strip,
                    env.params.remaining_tile_multipliers_strip,
                    env.params.perpendicular_word_multipliers_strip,
                );
                ranked = ranked_from(env, deferred, low_end);
            }
            idx += 1;
        }
        env.params.used_tile_scores_shadowr.clear(); // use shadowl in shadow_record
        took_a_tile
    }

    #[inline(always)]
    fn shadow_play_left<const PER_SPAN: bool, C: SpanCheck>(
        env: &mut Env<'_, C>,
        mut acc: Accumulator,
        mut idx: i8,
        mut is_unique: bool,
    ) {
        let mut deferred = 0u128;
        let mut ranked = 0u128;
        let mut low_end = env.params.lane_scaffold.ranking(
            acc.word_multiplier,
            env.params.board_strip,
            env.params.remaining_tile_multipliers_strip,
            env.params.perpendicular_word_multipliers_strip,
        );
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
            if num_played > u8::from(!is_unique) && env.anchor - idx >= 2 {
                shadow_record::<PER_SPAN, C>(
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
                stale_rack = shadow_play_right::<PER_SPAN, C>(
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
            let this_cross_bits = env.params.allowed_strip[idx as usize];
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
                        & (-i64::from(env.params.rack_tally_shadowl[tile as usize] == 0)) as u64;
                    // fall-through to case 2 (assume the optimized asm does not recheck the condition).
                }
                if matching_bits.is_power_of_two()
                    || matching_bits & env.params.alphabet.same_score_tile_bits(tile)
                        == matching_bits
                {
                    // case 2: multiple tiles fit, but they all have the same score.
                    // consume the square, but not the tile.
                    // rack_bits remains unchanged because assignment is tentative.
                    let tile_score = env.params.alphabet.scaled_score(tile).raw();
                    env.params
                        .used_tile_scores_shadowl
                        .insert_sorted(tile_score);
                    let tile_value = tile_score
                        * i32::from(env.params.remaining_tile_multipliers_strip[idx as usize]);
                    acc.main_score += tile_value;
                    acc.perpendicular_cumulative_score += env.params.perpendicular_scores_strip
                        [idx as usize]
                        + tile_value
                            * i32::from(
                                env.params.perpendicular_word_multipliers_strip[idx as usize],
                            );
                } else {
                    // case 3: multiple tiles fit, and they have different scores.
                    // rack_bits remains unchanged because assignment is tentative.
                    // defer to greedy algorithm.
                    let mut remaining = matching_bits;
                    let mut admissible_max = i32::MIN;
                    while remaining != 0 {
                        let t = remaining.trailing_zeros() as u8;
                        remaining &= remaining - 1;
                        admissible_max =
                            admissible_max.max(env.params.alphabet.scaled_score(t).raw());
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
            let word_multiplier =
                i32::from(env.params.remaining_word_multipliers_strip[idx as usize]);
            if word_multiplier != 1 {
                acc.word_multiplier *= word_multiplier;
                low_end = env.params.lane_scaffold.ranking(
                    acc.word_multiplier,
                    env.params.board_strip,
                    env.params.remaining_tile_multipliers_strip,
                    env.params.perpendicular_word_multipliers_strip,
                );
                ranked = ranked_from(env, deferred, low_end);
            }
            idx -= 1;
        }
    }

    #[inline(always)]
    fn gen_places_from<
        const PER_SPAN: bool,
        C: SpanCheck,
        PossibleStripPlacementCallbackType: FnMut(i8, i8, i8, i32, u8),
    >(
        env: &mut Env<'_, C>,
        single_tile_plays: bool,
        want_raw: bool,
        mut possible_strip_placement_callback: PossibleStripPlacementCallbackType,
    ) {
        if want_raw {
            possible_strip_placement_callback(env.anchor, env.leftmost, env.rightmost, i32::MAX, 0);
        } else {
            env.best_possible_equity = i32::MIN;
            env.params.span_out.clear();
            if !PER_SPAN {
                env.bounds_at = env.params.left_bounds.len();
                env.params
                    .left_bounds
                    .resize(env.bounds_at + 2 * env.params.bounds_stride, i32::MIN);
            }
            shadow_play_left::<PER_SPAN, C>(
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
                    let stride = env.params.bounds_stride;
                    let (at, upto) = env.params.left_bounds[env.bounds_at..].split_at_mut(stride);
                    let mut best = i32::MIN;
                    for (left, &bound) in at.iter().enumerate() {
                        best = best.max(bound);
                        upto[left] = best;
                    }
                    possible_strip_placement_callback(
                        env.anchor,
                        env.leftmost,
                        env.rightmost,
                        env.best_possible_equity,
                        0,
                    );
                }
            } else if !PER_SPAN {
                env.params.left_bounds.truncate(env.bounds_at);
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
            gen_places_from::<PER_SPAN, C, _>(
                &mut env,
                single_tile_plays,
                want_raw,
                &mut possible_strip_placement_callback,
            );
        }
        {
            // this part is only relevant if rack has at least two tiles, but passing that is too expensive.
            let leftmost = leftmost + i8::from(leftmost > 0 && leftmost < rightmost); // shadowing
            for anchor in (leftmost..rightmost).rev() {
                let cross_set_bits = env.params.allowed_strip[anchor as usize];
                if cross_set_bits & 1 != 0 {
                    if rightmost - leftmost < 2 {
                        // not enough room for 2-tile words
                        break;
                    }
                    if cross_set_bits != 1 {
                        env.anchor = anchor;
                        env.leftmost = leftmost;
                        env.rightmost = rightmost;
                        gen_places_from::<PER_SPAN, C, _>(
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

// the shadow pass above for an instance whose lane sets are narrower than u128,
// an instance a set type: its text gen_place_placements_impl's with the set
// type, its 0 and 1, and the lane's side D made parameters (the test
// the_u16_shadow_is_the_shadow holds it to that text). The instances in u128
// keep the plain function: a macro's panics report its call site, so its bounds
// checks would merge and its code would not be theirs.
#[rustfmt::skip]
macro_rules! shadow_pass {
    ($name:ident, $lane:ty, $zero:expr, $one:expr) => {
#[inline]
fn $name<
    'a,
    const PER_SPAN: bool,
    const D: usize,
    C: SpanCheck,
    PossibleStripPlacementCallbackType: FnMut(i8, i8, i8, i32, u8),
>(
    params: &'a mut GenPlacePlacementsParams<'a, C>,
    single_tile_plays: bool,
    want_raw: bool,
    mut possible_strip_placement_callback: PossibleStripPlacementCallbackType,
) {
    let strider_len = if D == 0 { params.board_strip.len() } else { D };
    if D != 0 {
        // the lane's length is a constant of this instance
        assert_eq!(params.board_strip.len(), D);
        assert_eq!(params.allowed_strip.len(), D);
        assert_eq!(params.remaining_word_multipliers_strip.len(), D);
        assert_eq!(params.remaining_tile_multipliers_strip.len(), D);
        assert_eq!(params.perpendicular_word_multipliers_strip.len(), D);
        assert_eq!(params.perpendicular_scores_strip.len(), D);
    }

    if !want_raw
        && !params.lane_scaffold.refresh::<D>(
            params.room,
            params.alphabet,
            params.board_strip,
            params.remaining_word_multipliers_strip,
            params.remaining_tile_multipliers_strip,
            params.perpendicular_word_multipliers_strip,
        )
    {
        return;
    }

    struct Env<'a, C: SpanCheck> {
        params: &'a mut GenPlacePlacementsParams<'a, C>,
        strider_len: usize,
        anchor: i8,
        leftmost: i8,
        rightmost: i8,
        best_possible_equity: i32,
        bounds_at: usize,
    }

    let mut env = Env {
        params,
        strider_len,
        anchor: 0,
        leftmost: 0,
        rightmost: 0,
        best_possible_equity: i32::MIN,
        bounds_at: 0,
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
    fn ranked_from<C: SpanCheck>(env: &Env<'_, C>, deferred: $lane, low_end: usize) -> $lane {
        let square_ranks_slice =
            &env.params.lane_scaffold.square_ranks[low_end..low_end + env.strider_len];
        let mut remaining = deferred;
        let mut ranked = $zero;
        while remaining != 0 {
            let square = remaining.trailing_zeros() as usize;
            remaining &= remaining - 1;
            ranked |= $one << square_ranks_slice[square];
        }
        ranked
    }

    #[inline(always)]
    fn shadow_record<const PER_SPAN: bool, C: SpanCheck>(
        env: &mut Env<'_, C>,
        acc: &Accumulator,
        idx_left: i8,
        idx_right: i8,
        num_played: u8,
        low_end: usize,
        ranked: $lane,
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
        if acc.crossed_board_tiles && env.params.check.refuses(idx_left, idx_right, num_played) {
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
        } else {
            let at = &mut env.params.left_bounds[env.bounds_at + idx_left as usize];
            *at = (*at).max(equity);
        }
        if equity > env.best_possible_equity {
            env.best_possible_equity = equity;
        }
    }

    struct ShadowRightWalk {
        idx: i8,
        is_unique: bool,
        idx_left: i8,
        num_played: u8,
        rack_bits: u64,
        deferred: $lane,
        low_end: usize,
        ranked: $lane,
        stale_rack: bool,
    }

    #[inline(always)]
    fn shadow_play_right<const PER_SPAN: bool, C: SpanCheck>(
        env: &mut Env<'_, C>,
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
            if idx > env.anchor + 1 && num_played > u8::from(!is_unique) && idx - idx_left >= 2 {
                shadow_record::<PER_SPAN, C>(env, &acc, idx_left, idx, num_played, low_end, ranked);
            }
            if num_played >= env.params.num_max_played {
                break;
            }

            if idx >= env.rightmost {
                break;
            }

            // place a tile at [idx] since it is still in bounds.
            let this_cross_bits = env.params.allowed_strip[idx as usize];
            if this_cross_bits & 1 == 0 {
                // nothing hooks here.
                is_unique = true;
                deferred |= $one << (idx as u32);
                ranked |= $one << env.params.lane_scaffold.square_ranks[low_end + idx as usize];
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
                        & (-i64::from(env.params.rack_tally_shadowr[tile as usize] == 0)) as u64;
                    // fall-through to case 2 (assume the optimized asm does not recheck the condition).
                }
                if matching_bits.is_power_of_two()
                    || matching_bits & env.params.alphabet.same_score_tile_bits(tile)
                        == matching_bits
                {
                    // case 2: multiple tiles fit, but they all have the same score.
                    // consume the square, but not the tile.
                    // rack_bits remains unchanged because assignment is tentative.
                    let tile_score = env.params.alphabet.scaled_score(tile).raw();
                    env.params
                        .used_tile_scores_shadowr
                        .insert_sorted(tile_score);
                    let tile_value = tile_score
                        * i32::from(env.params.remaining_tile_multipliers_strip[idx as usize]);
                    acc.main_score += tile_value;
                    acc.perpendicular_cumulative_score += env.params.perpendicular_scores_strip
                        [idx as usize]
                        + tile_value
                            * i32::from(
                                env.params.perpendicular_word_multipliers_strip[idx as usize],
                            );
                } else {
                    // case 3: multiple tiles fit, and they have different scores.
                    // rack_bits remains unchanged because assignment is tentative.
                    // defer to greedy algorithm.
                    let mut remaining = matching_bits;
                    let mut admissible_max = i32::MIN;
                    while remaining != 0 {
                        let t = remaining.trailing_zeros() as u8;
                        remaining &= remaining - 1;
                        admissible_max =
                            admissible_max.max(env.params.alphabet.scaled_score(t).raw());
                    }
                    acc.deferred_score_cap = acc.deferred_score_cap.max(admissible_max);
                    deferred |= $one << (idx as u32);
                    ranked |=
                        $one << env.params.lane_scaffold.square_ranks[low_end + idx as usize];
                    acc.perpendicular_cumulative_score +=
                        env.params.perpendicular_scores_strip[idx as usize];
                }
            } else {
                break;
            }
            num_played += 1;
            let word_multiplier =
                i32::from(env.params.remaining_word_multipliers_strip[idx as usize]);
            if word_multiplier != 1 {
                acc.word_multiplier *= word_multiplier;
                low_end = env.params.lane_scaffold.ranking(
                    acc.word_multiplier,
                    env.params.board_strip,
                    env.params.remaining_tile_multipliers_strip,
                    env.params.perpendicular_word_multipliers_strip,
                );
                ranked = ranked_from(env, deferred, low_end);
            }
            idx += 1;
        }
        env.params.used_tile_scores_shadowr.clear(); // use shadowl in shadow_record
        took_a_tile
    }

    #[inline(always)]
    fn shadow_play_left<const PER_SPAN: bool, C: SpanCheck>(
        env: &mut Env<'_, C>,
        mut acc: Accumulator,
        mut idx: i8,
        mut is_unique: bool,
    ) {
        let mut deferred = $zero;
        let mut ranked = $zero;
        let mut low_end = env.params.lane_scaffold.ranking(
            acc.word_multiplier,
            env.params.board_strip,
            env.params.remaining_tile_multipliers_strip,
            env.params.perpendicular_word_multipliers_strip,
        );
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
            if num_played > u8::from(!is_unique) && env.anchor - idx >= 2 {
                shadow_record::<PER_SPAN, C>(
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
                stale_rack = shadow_play_right::<PER_SPAN, C>(
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
            let this_cross_bits = env.params.allowed_strip[idx as usize];
            if this_cross_bits & 1 == 0 {
                // nothing hooks here.
                is_unique = true;
                deferred |= $one << (idx as u32);
                ranked |= $one << env.params.lane_scaffold.square_ranks[low_end + idx as usize];
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
                        & (-i64::from(env.params.rack_tally_shadowl[tile as usize] == 0)) as u64;
                    // fall-through to case 2 (assume the optimized asm does not recheck the condition).
                }
                if matching_bits.is_power_of_two()
                    || matching_bits & env.params.alphabet.same_score_tile_bits(tile)
                        == matching_bits
                {
                    // case 2: multiple tiles fit, but they all have the same score.
                    // consume the square, but not the tile.
                    // rack_bits remains unchanged because assignment is tentative.
                    let tile_score = env.params.alphabet.scaled_score(tile).raw();
                    env.params
                        .used_tile_scores_shadowl
                        .insert_sorted(tile_score);
                    let tile_value = tile_score
                        * i32::from(env.params.remaining_tile_multipliers_strip[idx as usize]);
                    acc.main_score += tile_value;
                    acc.perpendicular_cumulative_score += env.params.perpendicular_scores_strip
                        [idx as usize]
                        + tile_value
                            * i32::from(
                                env.params.perpendicular_word_multipliers_strip[idx as usize],
                            );
                } else {
                    // case 3: multiple tiles fit, and they have different scores.
                    // rack_bits remains unchanged because assignment is tentative.
                    // defer to greedy algorithm.
                    let mut remaining = matching_bits;
                    let mut admissible_max = i32::MIN;
                    while remaining != 0 {
                        let t = remaining.trailing_zeros() as u8;
                        remaining &= remaining - 1;
                        admissible_max =
                            admissible_max.max(env.params.alphabet.scaled_score(t).raw());
                    }
                    acc.deferred_score_cap = acc.deferred_score_cap.max(admissible_max);
                    deferred |= $one << (idx as u32);
                    ranked |=
                        $one << env.params.lane_scaffold.square_ranks[low_end + idx as usize];
                    acc.perpendicular_cumulative_score +=
                        env.params.perpendicular_scores_strip[idx as usize];
                }
            } else {
                break;
            }
            num_played += 1;
            let word_multiplier =
                i32::from(env.params.remaining_word_multipliers_strip[idx as usize]);
            if word_multiplier != 1 {
                acc.word_multiplier *= word_multiplier;
                low_end = env.params.lane_scaffold.ranking(
                    acc.word_multiplier,
                    env.params.board_strip,
                    env.params.remaining_tile_multipliers_strip,
                    env.params.perpendicular_word_multipliers_strip,
                );
                ranked = ranked_from(env, deferred, low_end);
            }
            idx -= 1;
        }
    }

    #[inline(always)]
    fn gen_places_from<
        const PER_SPAN: bool,
        C: SpanCheck,
        PossibleStripPlacementCallbackType: FnMut(i8, i8, i8, i32, u8),
    >(
        env: &mut Env<'_, C>,
        single_tile_plays: bool,
        want_raw: bool,
        mut possible_strip_placement_callback: PossibleStripPlacementCallbackType,
    ) {
        if want_raw {
            possible_strip_placement_callback(env.anchor, env.leftmost, env.rightmost, i32::MAX, 0);
        } else {
            env.best_possible_equity = i32::MIN;
            env.params.span_out.clear();
            if !PER_SPAN {
                env.bounds_at = env.params.left_bounds.len();
                env.params
                    .left_bounds
                    .resize(env.bounds_at + 2 * env.params.bounds_stride, i32::MIN);
            }
            shadow_play_left::<PER_SPAN, C>(
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
                    let stride = env.params.bounds_stride;
                    let (at, upto) = env.params.left_bounds[env.bounds_at..].split_at_mut(stride);
                    let mut best = i32::MIN;
                    for (left, &bound) in at.iter().enumerate() {
                        best = best.max(bound);
                        upto[left] = best;
                    }
                    possible_strip_placement_callback(
                        env.anchor,
                        env.leftmost,
                        env.rightmost,
                        env.best_possible_equity,
                        0,
                    );
                }
            } else if !PER_SPAN {
                env.params.left_bounds.truncate(env.bounds_at);
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
            gen_places_from::<PER_SPAN, C, _>(
                &mut env,
                single_tile_plays,
                want_raw,
                &mut possible_strip_placement_callback,
            );
        }
        {
            // this part is only relevant if rack has at least two tiles, but passing that is too expensive.
            let leftmost = leftmost + i8::from(leftmost > 0 && leftmost < rightmost); // shadowing
            for anchor in (leftmost..rightmost).rev() {
                let cross_set_bits = env.params.allowed_strip[anchor as usize];
                if cross_set_bits & 1 != 0 {
                    if rightmost - leftmost < 2 {
                        // not enough room for 2-tile words
                        break;
                    }
                    if cross_set_bits != 1 {
                        env.anchor = anchor;
                        env.leftmost = leftmost;
                        env.rightmost = rightmost;
                        gen_places_from::<PER_SPAN, C, _>(
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
    };
}

shadow_pass!(gen_place_placements_impl_u16, u16, 0u16, 1u16);
shadow_pass!(gen_place_placements_impl_u32, u32, 0u32, 1u32);

// a lane's shadow pass in its instance's set type (W): in u16 sets, a span at a
// time, else gen_place_placements
#[inline(always)]
fn shadow_lane<
    'a,
    const D: usize,
    W: LaneBits,
    C: SpanCheck,
    PossibleStripPlacementCallbackType: FnMut(i8, i8, i8, i32, u8),
>(
    params: &'a mut GenPlacePlacementsParams<'a, C>,
    single_tile_plays: bool,
    want_raw: bool,
    possible_strip_placement_callback: PossibleStripPlacementCallbackType,
) {
    if W::BITS == u16::BITS {
        gen_place_placements_impl_u16::<true, D, C, _>(
            params,
            single_tile_plays,
            want_raw,
            possible_strip_placement_callback,
        )
    } else {
        gen_place_placements(
            params,
            single_tile_plays,
            want_raw,
            possible_strip_placement_callback,
        )
    }
}

// the lean arm's shadow pass of a lane, an anchor at a time, in its instance's
// set type (W): u16 or u32 sets, else gen_place_placements
#[inline(always)]
fn shadow_lane_lean<
    'a,
    const D: usize,
    W: LaneBits,
    PossibleStripPlacementCallbackType: FnMut(i8, i8, i8, i32, u8),
>(
    params: &'a mut GenPlacePlacementsParams<'a, ()>,
    single_tile_plays: bool,
    want_raw: bool,
    possible_strip_placement_callback: PossibleStripPlacementCallbackType,
) {
    debug_assert!(!params.per_span);
    if W::BITS == u16::BITS {
        gen_place_placements_impl_u16::<false, D, (), _>(
            params,
            single_tile_plays,
            want_raw,
            possible_strip_placement_callback,
        )
    } else if W::BITS == u32::BITS {
        gen_place_placements_impl_u32::<false, D, (), _>(
            params,
            single_tile_plays,
            want_raw,
            possible_strip_placement_callback,
        )
    } else {
        gen_place_placements(
            params,
            single_tile_plays,
            want_raw,
            possible_strip_placement_callback,
        )
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
    allowed_strip: &'a [u64],
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
    num_min_played: u8,
    anchor: i8,
    leftmost: i8,
    rightmost: i8,
    callback: CallbackType,
    multi_leaves: &'a klv::MultiLeaves,
    num_tiles_in_bag: i32,
    play_out_bonus: i32,
    jumbled: &'a mut JumbledMultisets, // jumbled mode only
    woadwage: &'a mut WoadwageScratch,
    is_census: bool, // real-before-blank descent for the census's spell-once sheet build
    threshold: i32,
    best_leave_values: &'a [i32], // by tiles played, the bonus included
}

// what the table walk of one span reads (gen_classic_place_moves): its lane's
// strips and sums, the rack's subracks and leaves (by leave index: dense under
// the tables), and what changes span to span
struct TableWalkParams<
    'a,
    CallbackType: FnMut(i8, &[u8], i32, i32) -> i32,
    N: kwg::Node,
    L: kwg::Node,
> {
    board_snapshot: &'a BoardSnapshot<'a, N, L>,
    board_strip: &'a [u8],
    allowed_strip: &'a [u64],
    remaining_word_multipliers_strip: &'a [i8],
    remaining_tile_multipliers_strip: &'a [i8],
    perpendicular_word_multipliers_strip: &'a [i8],
    #[cfg(debug_assertions)]
    face_value_scores_strip: &'a [i32], // the debug walk's (add_square)
    #[cfg(debug_assertions)]
    perpendicular_scores_strip: &'a [i32], // the debug walk's (add_square)
    has_blank: bool, // the rack holds a blank
    word_strip_buffer: &'a mut [u8],
    num_max_played: u8,
    leftmost: i8,
    rightmost: i8,
    span_num_played: u8,
    callback: CallbackType,
    leave_values: &'a [i32], // the rack's, by leave index (klv::MultiLeaves)
    score_bound: i32,
    threshold: i32,
    subracks: &'a [Subrack],
    subracks_by_played: &'a [u32],
    descending_scores: &'a [i32],  // the rack's, as WorkingBuffer's
    span_multipliers: &'a [Taken], // its kept span's (own_bound), or none
    span_memo: &'a mut SpanMemo,
    span_prefix: &'a [SpanPrefix], // the lane's
    span_empty: u128,              // the lane's empty squares, a bit each
    span_multiplied: u128,         // of those, the ones with a word multiplier
}

// BAG_TERM: whether the generation has a bag term (MultiLeaves::bag_terms),
// which a leave read from the klv adds here; a table's leaves have it already
#[inline(always)]
fn leave_value_of<const BAG_TERM: bool, L: kwg::Node>(
    multi_leaves: &klv::MultiLeaves,
    klv: &klv::Klv<L>,
    rack_tally: &[u8],
    leave_idx: u32,
    num_tiles_in_bag: i32,
    play_out_bonus: i32,
    alphabet: &alphabet::Alphabet,
) -> i32 {
    if multi_leaves.is_dense() {
        multi_leaves.leave_value(leave_idx).raw()
    } else if num_tiles_in_bag <= 0 {
        let is_played_out = rack_tally.iter().all(|&count| count == 0);
        if is_played_out {
            play_out_bonus
        } else {
            let residual: i32 = (0u8..)
                .zip(rack_tally.iter().take(alphabet.len() as usize))
                .map(|(tile, &count)| i32::from(count) * i32::from(alphabet.score(tile)))
                .sum();
            -equity::ENDGAME_PENALTY_BASE - 2 * residual * equity::SCALE
        }
    } else {
        let leave_value = klv.leave_value_from_tally(rack_tally).raw();
        if BAG_TERM {
            leave_value + multi_leaves.bag_term_of_kept(rack_tally)
        } else {
            leave_value
        }
    }
}

// what a walk along a run of the word graph's nodes knows of the tiles ahead of
// it in the run
trait RunLetters: Copy {
    // whether the run from node p on may hold a tile in candidates
    fn may_hold(self, p: u32, candidates: u64) -> bool;

    // the run's tiles from node p on, a bit each, when known; a walk then goes
    // straight to a tile's node, the count of the run's tiles below it past p,
    // which needs each run's tiles ascending (kwg::Kwg::runs_ascend)
    fn from(self, p: u32) -> Option<u64>;

    // read one demanded tile in a run, as when passing a tile on the board.
    #[inline(always)]
    fn find<N: kwg::Node>(self, graph: &kwg::Kwg<N>, mut p: i32, tile: u8) -> i32 {
        if p <= 0 {
            return -1;
        }
        if let Some(run) = self.from(p.cast_unsigned()) {
            let bit = 1u64 << tile;
            if run & bit != 0 {
                return p + (run & (bit - 1)).count_ones() as i32;
            }
            return -1;
        }
        loop {
            let node = graph.node_in_run(p);
            if node.tile() == tile {
                return p;
            }
            if node.is_end() {
                return -1;
            }
            p += 1;
        }
    }
}

// nothing, so every run may
impl RunLetters for () {
    #[inline(always)]
    fn may_hold(self, _p: u32, _candidates: u64) -> bool {
        true
    }

    #[inline(always)]
    fn from(self, _p: u32) -> Option<u64> {
        None
    }
}

// by node, the tiles from it to the end of its run (kwg::Kwg::compute_letter_bits)
impl RunLetters for &[u64] {
    #[inline(always)]
    fn may_hold(self, p: u32, candidates: u64) -> bool {
        self[p as usize] & candidates != 0
    }

    #[inline(always)]
    fn from(self, p: u32) -> Option<u64> {
        Some(self[p as usize])
    }
}

// a lean walk's read of a strip at i (an i8), unchecked
macro_rules! strip_at {
    ($strip:expr, $i:expr) => {{
        let (strip, i) = ($strip, $i);
        debug_assert!(i >= 0 && (i as usize) < strip.len());
        // safety: play_left reads at leftmost <= i <= anchor (left of its
        // square only while i > leftmost) and play_right at anchor < i <
        // rightmost, a window gen_classic_place_moves_lean asserts lies in each
        // strip it reads
        unsafe { strip.get_unchecked(i as usize) }
    }};
}

// a walk asked for no leaves, as a raw generation asks, keeps no leave index
// and reads no leave value: the callback is given 0 for the leave
#[inline]
fn gen_classic_place_moves_lean<
    'a,
    const LEAVES: bool,
    const BAG_TERM: bool,
    R: RunLetters,
    CallbackType: FnMut(i8, &[u8], i32, i32) -> i32,
    N: kwg::Node,
    L: kwg::Node,
>(
    params: &'a mut GenPlaceMovesParams<'a, CallbackType, N, L>,
    single_tile_plays: bool,
    left_bounds: &'a [i32],
    runs: R,
) {
    struct Env<
        'a,
        R: RunLetters,
        CallbackType: FnMut(i8, &[u8], i32, i32) -> i32,
        N: kwg::Node,
        L: kwg::Node,
    > {
        params: &'a mut GenPlaceMovesParams<'a, CallbackType, N, L>,
        alphabet: &'a alphabet::Alphabet,
        num_played: u8,
        idx_left: i8,
        rack_bits: u64,
        letter_bits: u64,
        left_bounds: &'a [i32], // by left edge, the best span bound at it, then at or before it
        runs: R,                // a walk along a run stops where these leave it no candidate
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
        const LEAVES: bool,
        const BAG_TERM: bool,
        R: RunLetters,
        CallbackType: FnMut(i8, &[u8], i32, i32) -> i32,
        N: kwg::Node,
        L: kwg::Node,
    >(
        env: &mut Env<'_, R, CallbackType, N, L>,
        acc: &Accumulator,
        idx_left: i8,
        idx_right: i8,
    ) {
        if env.num_played < env.params.num_min_played {
            return;
        }
        let score = if SPELL_ONCE {
            0
        } else {
            acc.main_score * acc.word_multiplier
                + acc.perpendicular_cumulative_score
                + i32::from(
                    env.params
                        .board_snapshot
                        .game_config
                        .num_played_bonus(env.num_played),
                ) * equity::SCALE
        };
        let leave_value = if SPELL_ONCE || !LEAVES {
            0
        } else {
            leave_value_of::<BAG_TERM, _>(
                env.params.multi_leaves,
                env.params.board_snapshot.klv,
                env.params.rack_tally,
                acc.leave_idx,
                env.params.num_tiles_in_bag,
                env.params.play_out_bonus,
                env.alphabet,
            )
        };
        debug_assert!(
            SPELL_ONCE
                || !LEAVES
                || env.left_bounds.is_empty()
                || score + leave_value <= env.left_bounds[idx_left as usize]
        );
        if !SPELL_ONCE && score + leave_value < env.params.threshold {
            return;
        }
        env.params.threshold = (env.params.callback)(
            idx_left,
            &env.params.word_strip_buffer[idx_left as usize..idx_right as usize],
            score,
            leave_value,
        );
    }

    fn play_right<
        const SPELL_ONCE: bool,
        const LEAVES: bool,
        const BAG_TERM: bool,
        R: RunLetters,
        CallbackType: FnMut(i8, &[u8], i32, i32) -> i32,
        N: kwg::Node,
        L: kwg::Node,
    >(
        env: &mut Env<'_, R, CallbackType, N, L>,
        acc: &mut Accumulator,
        mut p: i32,
        mut idx: i8,
        mut is_unique: bool,
    ) {
        // tail-recurse placing current sequence of tiles
        while idx < env.params.rightmost {
            let b = *strip_at!(env.params.board_strip, idx);
            if b == 0 {
                break;
            }
            let graph = env.params.board_snapshot.kwg;
            p = env
                .runs
                .find(graph, graph.node_in_run(p).arc_index(), b & 0x7f);
            if p <= 0 {
                return;
            }
            if !SPELL_ONCE {
                acc.main_score += *strip_at!(env.params.face_value_scores_strip, idx);
            }
            idx += 1;
        }
        let node = env.params.board_snapshot.kwg.node_in_run(p);
        if idx > env.params.anchor + 1
            && env.num_played > u8::from(!is_unique)
            && idx - env.idx_left >= 2
            && node.accepts()
        {
            record::<SPELL_ONCE, LEAVES, BAG_TERM, _, _, _, _>(env, acc, env.idx_left, idx);
        }
        if env.num_played >= env.params.num_max_played {
            return;
        }

        if idx < env.params.rightmost {
            p = node.arc_index();
            if p <= 0 {
                return;
            }
            let allowed = *strip_at!(env.params.allowed_strip, idx);
            if allowed == 1 {
                // already handled '@'
                return;
            }
            is_unique |= allowed & 1 == 0;
            // turn off bit 0 so it cannot match later
            let this_cross_bits = allowed & !1;
            let mut candidates = this_cross_bits
                & if env.params.rack_tally[0] > 0 {
                    env.letter_bits
                } else {
                    env.rack_bits
                };
            let run = env.runs.from(p.cast_unsigned());
            if candidates == 0 || run.is_some_and(|run| run & candidates == 0) {
                return;
            }
            let new_word_multiplier = acc.word_multiplier
                * i32::from(*strip_at!(env.params.remaining_word_multipliers_strip, idx));
            let tile_multiplier = *strip_at!(env.params.remaining_tile_multipliers_strip, idx);
            let perpendicular_word_multiplier =
                *strip_at!(env.params.perpendicular_word_multipliers_strip, idx);
            let perpendicular_score = *strip_at!(env.params.perpendicular_scores_strip, idx);
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
                    let tile_value =
                        env.alphabet.scaled_score(0).raw() * i32::from(tile_multiplier);
                    Accumulator {
                        main_score: acc.main_score + tile_value,
                        perpendicular_cumulative_score: acc.perpendicular_cumulative_score
                            + perpendicular_score
                            + tile_value * i32::from(perpendicular_word_multiplier),
                        word_multiplier: new_word_multiplier,
                        leave_idx: if !LEAVES {
                            0
                        } else {
                            acc.leave_idx
                                .wrapping_sub(env.params.multi_leaves.place_value(0))
                        },
                    }
                }
            });
            macro_rules! visit {
                ($p:expr, $node:expr, $tile:expr) => {{
                    let (p, node, tile) = ($p, $node, $tile);
                    let bit = 1u64 << tile;
                    env.params.board_snapshot.kwg.prefetch(node.arc_index());
                    if env.params.rack_tally[tile as usize] > 0 {
                        env.params.rack_tally[tile as usize] -= 1;
                        let spent = bit * u64::from(env.params.rack_tally[tile as usize] == 0);
                        env.rack_bits ^= spent;
                        env.params.word_strip_buffer[idx as usize] = tile;
                        play_right::<SPELL_ONCE, LEAVES, BAG_TERM, _, _, _, _>(
                            env,
                            &mut if SPELL_ONCE {
                                Accumulator {
                                    main_score: 0,
                                    perpendicular_cumulative_score: 0,
                                    word_multiplier: 0,
                                    leave_idx: 0,
                                }
                            } else {
                                let tile_value = env.alphabet.scaled_score(tile).raw()
                                    * i32::from(tile_multiplier);
                                Accumulator {
                                    main_score: acc.main_score + tile_value,
                                    perpendicular_cumulative_score: acc
                                        .perpendicular_cumulative_score
                                        + perpendicular_score
                                        + tile_value * i32::from(perpendicular_word_multiplier),
                                    word_multiplier: new_word_multiplier,
                                    leave_idx: if !LEAVES {
                                        0
                                    } else {
                                        acc.leave_idx
                                            .wrapping_sub(env.params.multi_leaves.place_value(tile))
                                    },
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
                        play_right::<SPELL_ONCE, LEAVES, BAG_TERM, _, _, _, _>(
                            env,
                            &mut Accumulator { ..*blank_acc },
                            p,
                            idx + 1,
                            is_unique,
                        );
                        env.params.rack_tally[0] += 1;
                    }
                }};
            }
            match run {
                // the run's tiles known: each candidate's node is the count of
                // the run's tiles below it past p, and the others are never read
                Some(run) => {
                    let mut hits = run & candidates;
                    while hits != 0 {
                        let tile = hits.trailing_zeros() as u8;
                        hits &= hits - 1;
                        let at = p + (run & ((1u64 << tile) - 1)).count_ones() as i32;
                        let node = env.params.board_snapshot.kwg.node_in_run(at);
                        debug_assert_eq!(node.tile(), tile);
                        visit!(at, node, tile);
                    }
                }
                None => loop {
                    let node = env.params.board_snapshot.kwg.node_in_run(p);
                    let tile = node.tile();
                    let bit = 1u64 << tile;
                    if candidates & bit != 0 {
                        candidates ^= bit;
                        visit!(p, node, tile);
                        if candidates == 0
                            || (!node.is_end()
                                && !env.runs.may_hold((p + 1).cast_unsigned(), candidates))
                        {
                            break;
                        }
                    }
                    if node.is_end() {
                        break;
                    }
                    p += 1;
                },
            }
            env.num_played -= 1;
        }
    }

    fn play_left<
        const SPELL_ONCE: bool,
        const LEAVES: bool,
        const BAG_TERM: bool,
        R: RunLetters,
        CallbackType: FnMut(i8, &[u8], i32, i32) -> i32,
        N: kwg::Node,
        L: kwg::Node,
    >(
        env: &mut Env<'_, R, CallbackType, N, L>,
        acc: &mut Accumulator,
        mut p: i32,
        mut idx: i8,
        mut is_unique: bool,
    ) {
        // tail-recurse placing current sequence of tiles
        if p == 1 && idx >= env.params.leftmost && *strip_at!(env.params.board_strip, idx) != 0 {
            let mut jump_idx = idx;
            while jump_idx > env.params.leftmost
                && env.params.board_strip[jump_idx as usize - 1] != 0
            {
                jump_idx -= 1;
            }
            p = strip_at!(env.params.cross_set_buffer_strip, jump_idx).p;
            if p <= 0 {
                return;
            }
            if !SPELL_ONCE {
                acc.main_score += strip_at!(env.params.cross_set_buffer_strip, jump_idx).score;
            }
            idx = jump_idx - 1;
        } else {
            while idx >= env.params.leftmost {
                let b = *strip_at!(env.params.board_strip, idx);
                if b == 0 {
                    break;
                }
                let graph = env.params.board_snapshot.kwg;
                p = env
                    .runs
                    .find(graph, graph.node_in_run(p).arc_index(), b & 0x7f);
                if p <= 0 {
                    return;
                }
                if !SPELL_ONCE {
                    acc.main_score += *strip_at!(env.params.face_value_scores_strip, idx);
                }
                idx -= 1;
            }
        }
        let node = env.params.board_snapshot.kwg.node_in_run(p);
        if env.num_played > u8::from(!is_unique) && env.params.anchor - idx >= 2 && node.accepts() {
            record::<SPELL_ONCE, LEAVES, BAG_TERM, _, _, _, _>(
                env,
                acc,
                idx + 1,
                env.params.anchor + 1,
            );
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
            let bits = *strip_at!(env.params.allowed_strip, idx);
            if bits != 1 {
                is_unique |= bits & 1 == 0;
                // turn off bit 0 so it cannot match later
                this_cross_bits = bits & !1;
            }
        }

        let mut turn = 1u64;
        let bounds = env.left_bounds;
        if !SPELL_ONCE && !bounds.is_empty() {
            if bounds[(idx + 1) as usize] < env.params.threshold {
                turn = 0;
            }
            if idx >= env.params.leftmost
                && bounds[bounds.len() / 2 + idx as usize] < env.params.threshold
            {
                this_cross_bits = 0;
            }
        }

        if this_cross_bits == 0 {
            if turn == 0 {
                return;
            }
            let turnaround_p = env.runs.find(env.params.board_snapshot.kwg, p, 0);
            if turnaround_p > 0 {
                // assume idx < env.params.anchor, because tile 0 does not occur at start in well-formed kwg gaddawg
                env.idx_left = idx + 1;
                play_right::<SPELL_ONCE, LEAVES, BAG_TERM, _, _, _, _>(
                    env,
                    acc,
                    turnaround_p,
                    env.params.anchor + 1,
                    turnaround_is_unique,
                );
            }
            return;
        }

        let mut candidates = turn
            | (this_cross_bits
                & if env.params.rack_tally[0] > 0 {
                    env.letter_bits
                } else {
                    env.rack_bits
                });
        let run = env.runs.from(p.cast_unsigned());
        if candidates == 0 || run.is_some_and(|run| run & candidates == 0) {
            return;
        }
        let new_word_multiplier = acc.word_multiplier
            * i32::from(*strip_at!(env.params.remaining_word_multipliers_strip, idx));
        let tile_multiplier = *strip_at!(env.params.remaining_tile_multipliers_strip, idx);
        let perpendicular_word_multiplier =
            *strip_at!(env.params.perpendicular_word_multipliers_strip, idx);
        let perpendicular_score = *strip_at!(env.params.perpendicular_scores_strip, idx);
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
                let tile_value = env.alphabet.scaled_score(0).raw() * i32::from(tile_multiplier);
                Accumulator {
                    main_score: acc.main_score + tile_value,
                    perpendicular_cumulative_score: acc.perpendicular_cumulative_score
                        + perpendicular_score
                        + tile_value * i32::from(perpendicular_word_multiplier),
                    word_multiplier: new_word_multiplier,
                    leave_idx: if !LEAVES {
                        0
                    } else {
                        acc.leave_idx
                            .wrapping_sub(env.params.multi_leaves.place_value(0))
                    },
                }
            }
        });
        macro_rules! visit {
            ($p:expr, $node:expr, $tile:expr) => {{
                let (p, node, tile) = ($p, $node, $tile);
                let bit = 1u64 << tile;
                if tile == 0 {
                    env.num_played -= 1;
                    env.idx_left = idx + 1;
                    play_right::<SPELL_ONCE, LEAVES, BAG_TERM, _, _, _, _>(
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
                        let spent = bit * u64::from(env.params.rack_tally[tile as usize] == 0);
                        env.rack_bits ^= spent;
                        env.params.word_strip_buffer[idx as usize] = tile;
                        play_left::<SPELL_ONCE, LEAVES, BAG_TERM, _, _, _, _>(
                            env,
                            &mut if SPELL_ONCE {
                                Accumulator {
                                    main_score: 0,
                                    perpendicular_cumulative_score: 0,
                                    word_multiplier: 0,
                                    leave_idx: 0,
                                }
                            } else {
                                let tile_value = env.alphabet.scaled_score(tile).raw()
                                    * i32::from(tile_multiplier);
                                Accumulator {
                                    main_score: acc.main_score + tile_value,
                                    perpendicular_cumulative_score: acc
                                        .perpendicular_cumulative_score
                                        + perpendicular_score
                                        + tile_value * i32::from(perpendicular_word_multiplier),
                                    word_multiplier: new_word_multiplier,
                                    leave_idx: if !LEAVES {
                                        0
                                    } else {
                                        acc.leave_idx
                                            .wrapping_sub(env.params.multi_leaves.place_value(tile))
                                    },
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
                        play_left::<SPELL_ONCE, LEAVES, BAG_TERM, _, _, _, _>(
                            env,
                            &mut Accumulator { ..*blank_acc },
                            p,
                            idx - 1,
                            is_unique,
                        );
                        env.params.rack_tally[0] += 1;
                    }
                }
            }};
        }
        // the run's tiles known: each candidate's node is the count of the
        // run's tiles below it past p, and the others are never read
        if let Some(run) = run {
            let mut hits = run & candidates;
            while hits != 0 {
                let tile = hits.trailing_zeros() as u8;
                hits &= hits - 1;
                let at = p + (run & ((1u64 << tile) - 1)).count_ones() as i32;
                let node = env.params.board_snapshot.kwg.node_in_run(at);
                debug_assert_eq!(node.tile(), tile);
                visit!(at, node, tile);
            }
            env.num_played -= 1;
            return;
        }
        loop {
            let node = env.params.board_snapshot.kwg.node_in_run(p);
            let tile = node.tile();
            let bit = 1u64 << tile;
            if candidates & bit != 0 {
                candidates ^= bit;
                visit!(p, node, tile);
                if candidates == 0
                    || (!node.is_end() && !env.runs.may_hold((p + 1).cast_unsigned(), candidates))
                {
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

    // the window strip_at reads in, in every strip
    {
        let p = &*params;
        let end = p.rightmost as usize;
        assert!(0 <= p.leftmost && p.leftmost <= p.anchor && p.anchor < p.rightmost);
        assert!(
            end <= p.board_strip.len()
                && end <= p.allowed_strip.len()
                && end <= p.cross_set_buffer_strip.len()
                && end <= p.remaining_word_multipliers_strip.len()
                && end <= p.remaining_tile_multipliers_strip.len()
                && end <= p.face_value_scores_strip.len()
                && end <= p.perpendicular_word_multipliers_strip.len()
                && end <= p.perpendicular_scores_strip.len()
        );
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
        letter_bits: (u64::MAX >> (64 - u32::from(alphabet.len()))) & !1,
        left_bounds,
        runs,
    };
    let mut acc = Accumulator {
        main_score: 0,
        perpendicular_cumulative_score: 0,
        word_multiplier: 1,
        leave_idx: pass_leave_idx,
    };
    if is_census {
        play_left::<true, LEAVES, BAG_TERM, _, _, _, _>(
            &mut env,
            &mut acc,
            1,
            anchor,
            single_tile_plays,
        );
    } else {
        play_left::<false, LEAVES, BAG_TERM, _, _, _, _>(
            &mut env,
            &mut acc,
            1,
            anchor,
            single_tile_plays,
        );
    }
}

#[derive(Clone, Copy)]
struct Subrack {
    key: u128,
    part: u128, // its key's part (anagrams::key_part)
    leave_idx: u32,
    leave_value: i32,
    places: u32, // its tiles' places in descending_scores, a bit each
    num_played: u8,
    blanks: u8,
}

// a memo visit's group not looked up yet (no group is at u32::MAX, as the arena
// it indexes is shorter)
const UNSEEN_GROUP: u32 = u32::MAX;

// the places start..start + taken, a bit each, or none when they pass a u32's
// bits (a rack of more tiles is bounded without them)
#[inline(always)]
fn taken_places(start: u8, taken: u8) -> u32 {
    if u32::from(start) + u32::from(taken) > u32::BITS {
        return 0;
    }
    (((1u64 << taken) - 1) << start) as u32
}

#[derive(Clone)]
struct SpanMemo {
    turn: u32,
    slots: Vec<SpanMemoSlot>,
    visits: Vec<(u32, u32)>,
}

#[derive(Clone)]
struct SpanMemoSlot {
    key: u128,
    turn: u32,
    len: u8,
    from: usize,
    upto: usize,
    cut_bound: i32, // i32::MAX when the visits were not cut
}

impl SpanMemoSlot {
    #[inline(always)]
    fn new() -> Self {
        Self {
            key: 0,
            turn: 0,
            len: 0,
            from: 0,
            upto: 0,
            cut_bound: 0,
        }
    }
}

impl SpanMemo {
    #[inline(always)]
    fn new() -> Self {
        Self {
            turn: 0,
            slots: Vec::new(),
            visits: Vec::new(),
        }
    }

    #[inline(always)]
    fn start_turn(&mut self, placements: usize) {
        if self.turn == u32::MAX {
            self.slots.fill(SpanMemoSlot::new());
            self.turn = 0;
        }
        let want = (placements * 2).next_power_of_two();
        if self.slots.len() < want {
            self.slots.resize(want, SpanMemoSlot::new());
        }
        self.turn += 1;
        self.visits.clear();
    }

    // key's part is part (anagrams::key_part)
    #[inline(always)]
    fn find(&self, key: u128, part: u128, len: u8, bound: i32) -> Result<(usize, usize), usize> {
        let mask = self.slots.len() - 1;
        let mut at = (anagrams::part_hash(part) ^ u64::from(len)) as usize & mask;
        loop {
            let slot = &self.slots[at];
            if slot.turn != self.turn {
                return Err(at);
            }
            if slot.key == key && slot.len == len {
                // visits cut at a lower score bound than this span's may lack
                // subracks it needs
                if bound > slot.cut_bound {
                    return Err(at);
                }
                return Ok((slot.from, slot.upto));
            }
            at = (at + 1) & mask;
        }
    }

    #[inline(always)]
    fn keep(&mut self, at: usize, key: u128, len: u8, from: usize, cut_bound: i32) {
        self.slots[at] = SpanMemoSlot {
            key,
            turn: self.turn,
            len,
            from,
            upto: self.visits.len(),
            cut_bound,
        };
    }
}

// a lane's running sums for gen_classic_place_moves's spans, an entry before
// each square and one after the last: what add_square adds over the squares
// before it, so a span's are two entries' difference
#[derive(Clone, Copy)]
struct SpanPrefix {
    key: u128,
    face: i32,
    perp: i32,
    played: u8,
    free: u8,
    dead: u8,
}

impl SpanPrefix {
    #[inline(always)]
    fn new() -> Self {
        Self {
            key: 0,
            face: 0,
            perp: 0,
            played: 0,
            free: 0,
            dead: 0,
        }
    }
}

// a lane's sums as filled: the generation they are for, whether they are the
// lane's inputs as the table walk keeps them (fill_span_lane), its empty
// squares, and its empty squares whose word multiplier is not 1, a bit each
#[derive(Clone)]
struct SpanLane {
    turn: u32,
    kept: bool,
    empty: u128,
    multiplied: u128,
}

impl SpanLane {
    #[inline(always)]
    fn new() -> Self {
        Self {
            turn: 0,
            kept: false,
            empty: 0,
            multiplied: 0,
        }
    }
}

// the subracks' order (positions in build_subracks's walk of them): by tiles
// played, then leave descending, then position, a key each into order, sorted
#[inline]
fn sort_subracks(subracks: &[Subrack], order: &mut Vec<u64>) {
    debug_assert!(subracks.len() <= 1 << 24);
    order.clear();
    order.extend(subracks.iter().enumerate().map(|(i, s)| {
        u64::from(s.num_played) << 56
            | u64::from(!(s.leave_value as u32 ^ i32::MIN as u32)) << 24
            | i as u64
    }));
    order.sort_unstable();
}

// a rack's subracks' order as a rack table keeps it (racks::RackTable): each
// subrack's position in build_subracks's walk of them, in sort_subracks's
// order; the rack's leaves are multi_leaves's
pub(crate) fn subrack_order(
    multi_leaves: &klv::MultiLeaves,
    rack_tally: &[u8],
    keys: &mut Vec<u64>,
    order: &mut Vec<u8>,
) {
    fn walk(digits: &[(u8, u32)], leaves: &[i32], leave_idx: u32, played: u8, keys: &mut Vec<u64>) {
        if let Some((&(count, place), rest)) = digits.split_first() {
            for kept in 0..=count {
                walk(
                    rest,
                    leaves,
                    leave_idx + u32::from(kept) * place,
                    played - kept,
                    keys,
                );
            }
        } else {
            let value = leaves[leave_idx as usize];
            const SCORE_SHIFT: u32 = u64::BITS - u8::BITS - i32::BITS;
            const PLAYED_SHIFT: u32 = SCORE_SHIFT + i32::BITS;
            keys.push(
                u64::from(played) << PLAYED_SHIFT
                    | u64::from(!(value as u32 ^ i32::MIN as u32)) << SCORE_SHIFT
                    | keys.len() as u64,
            );
        }
    }

    let mut digits = [(0, 0); crate::racks::MAX_RACK_SIZE as usize];
    let mut num_digits = 0;
    let mut played = 0;
    for (tile, &count) in rack_tally.iter().enumerate() {
        if count != 0 {
            digits[num_digits] = (count, multi_leaves.place_value(tile as u8));
            num_digits += 1;
            played += count;
        }
    }
    keys.clear();
    walk(
        &digits[..num_digits],
        multi_leaves.leave_values(),
        0,
        played,
        keys,
    );
    keys.sort_unstable();
    order.clear();
    order.extend(keys.iter().map(|&key| key as u8));
}

// a rack's subracks in build_subracks's walk of them, each with its leave: the
// rack's tiles in order, each one's count kept running from none to all, the
// last tile's turning fastest, so a step keeps one more of the last tile with
// room and none of each after it, and moves its key, the key's part (key_part
// adds up), the leave's index, the tiles played, and the blanks by place values
#[inline]
fn walk_subracks<P: alphagram::KeyPlaces>(
    multi_leaves: &klv::MultiLeaves,
    places: P,
    rack_tally: &[u8],
    first_places: &[u8],
    subracks: &mut Vec<Subrack>,
) {
    // each tile on the rack: its count; its place values in the leave's index,
    // in the key (none for the blank), and in the key's part; whether it is the
    // blank; and its first place in descending_scores (first_places: its taken
    // copies hold the places from it on)
    let mut tiles = [(0u8, 0u32, 0u128, 0u128, 0u8, 0u8); MAX_ALPHABET_LEN];
    let mut kept = [0u8; MAX_ALPHABET_LEN];
    let mut n = 0;
    let (mut key, mut part, mut played, mut blanks) = (0u128, 0u128, 0u8, 0u8);
    let mut taken_at = 0u32;
    for (tile, &count) in rack_tally.iter().enumerate() {
        if count != 0 {
            let blank = u8::from(tile == 0);
            let key_place_value = if tile == 0 {
                0
            } else {
                places.place_value(tile as u8)
            };
            let part_place_value = anagrams::key_part(key_place_value);
            tiles[n] = (
                count,
                multi_leaves.place_value(tile as u8),
                key_place_value,
                part_place_value,
                blank,
                first_places[tile],
            );
            key += u128::from(count) * key_place_value;
            part = part.wrapping_add(part_place_value.wrapping_mul(u128::from(count)));
            played += count;
            blanks += blank * count;
            taken_at |= taken_places(first_places[tile], count);
            n += 1;
        }
    }
    subracks.clear();
    let mut idx = 0u32;
    loop {
        debug_assert!(places.holds(
            key,
            &[alphagram::Nibble::MAX_COUNT; alphagram::Nibble::TILES]
        ));
        debug_assert_eq!(part, anagrams::key_part(key));
        subracks.push(Subrack {
            key,
            part,
            leave_idx: idx,
            leave_value: multi_leaves.leave_value(idx).raw(),
            places: taken_at,
            num_played: played,
            blanks,
        });
        let mut i = n;
        loop {
            if i == 0 {
                return;
            }
            i -= 1;
            let (count, place_value, key_place_value, part_place_value, blank, first) = tiles[i];
            if kept[i] < count {
                kept[i] += 1;
                idx += place_value;
                key -= key_place_value;
                part = part.wrapping_sub(part_place_value);
                played -= 1;
                blanks -= blank;
                taken_at &= !taken_places(first + count - kept[i], 1);
                break;
            }
            kept[i] = 0;
            idx -= u32::from(count) * place_value;
            key += u128::from(count) * key_place_value;
            part = part.wrapping_add(part_place_value.wrapping_mul(u128::from(count)));
            played += count;
            blanks += blank * count;
            taken_at |= taken_places(first, count);
        }
    }
}

#[derive(Clone, Copy)]
struct RackShape<'a> {
    by_played: &'a [u32],
    leave_indices: &'a [u8],
}

const RACK_SHAPE_MAX: usize = crate::racks::MAX_RACK_SIZE as usize;

#[derive(Clone, Copy)]
struct RackShapeEntry {
    by_played: [u32; RACK_SHAPE_MAX + 2],
    leave_start: usize,
}

const fn rack_shape_leave_count() -> usize {
    let mut totals = [0; RACK_SHAPE_MAX + 1];
    totals[0] = 1;
    let mut count = 0;
    let mut size = 1;
    while size <= RACK_SHAPE_MAX {
        let mut first = 1;
        while first <= size {
            totals[size] += (first + 1) * totals[size - first];
            first += 1;
        }
        count += totals[size];
        size += 1;
    }
    count
}

struct RackShapes {
    entries: [RackShapeEntry; 1 << RACK_SHAPE_MAX],
    leave_indices: [u8; rack_shape_leave_count()],
}

static RACK_SHAPES: RackShapes = {
    let mut shapes = RackShapes {
        entries: [RackShapeEntry {
            by_played: [0; RACK_SHAPE_MAX + 2],
            leave_start: 0,
        }; 1 << RACK_SHAPE_MAX],
        leave_indices: [0; rack_shape_leave_count()],
    };
    let mut start = 0;
    let mut gaps = 1usize;
    while gaps < shapes.entries.len() {
        let rack_size = gaps.ilog2() as usize + 1;
        let shape = &mut shapes.entries[gaps];
        shape.leave_start = start;
        let mut counts = [0usize; RACK_SHAPE_MAX];
        let mut places = [1usize; RACK_SHAPE_MAX];
        let mut leaves = 1;
        let mut num_digits = 0;
        let mut first = 0;
        let mut end = 1;
        while end <= rack_size {
            if gaps & (1 << (end - 1)) != 0 {
                counts[num_digits] = end - first;
                places[num_digits] = leaves;
                leaves *= counts[num_digits] + 1;
                num_digits += 1;
                first = end;
            }
            end += 1;
        }

        // the walk takes the last digit fastest, and leave indices take the
        // first digit fastest. record each walk step's leave index.
        let mut walked = 0;
        while walked < leaves {
            let mut rest = walked;
            let mut leave = 0;
            let mut played = rack_size;
            let mut digit = num_digits;
            while digit != 0 {
                digit -= 1;
                let radix = counts[digit] + 1;
                let kept = rest % radix;
                rest /= radix;
                leave += kept * places[digit];
                played -= kept;
            }
            shapes.leave_indices[start + walked] = leave as u8;
            shape.by_played[played + 1] += 1;
            walked += 1;
        }
        let mut played = 1;
        while played <= rack_size + 1 {
            shape.by_played[played] += shape.by_played[played - 1];
            played += 1;
        }
        start += leaves;
        gaps += 1;
    }
    assert!(start == shapes.leave_indices.len());
    shapes
};

#[inline(always)]
fn rack_shape(rack_tally: &[u8], mut rack_bits: u64, rack_size: usize) -> RackShape<'static> {
    let mut end = 0;
    let mut gaps = 0;
    while rack_bits != 0 {
        let tile = rack_bits.trailing_zeros() as usize;
        rack_bits &= rack_bits - 1;
        end += rack_tally[tile] as usize;
        gaps |= 1 << (end - 1);
    }
    let entry = &RACK_SHAPES.entries[gaps];
    let count = entry.by_played[rack_size + 1] as usize;
    RackShape {
        by_played: &entry.by_played[..rack_size + 2],
        leave_indices: &RACK_SHAPES.leave_indices[entry.leave_start..entry.leave_start + count],
    }
}

// a rack's subracks in sort_subracks's order, by played their starts: sorted,
// or taken in the order a rack table gives (given, positions in the walk)
#[inline]
fn build_subracks<P: alphagram::KeyPlaces>(
    multi_leaves: &klv::MultiLeaves,
    places: P,
    (rack_tally, first_places, num_tiles_on_rack): (&[u8], &[u8], u8),
    subracks: &mut Vec<Subrack>,
    by_played: &mut Vec<u32>,
    (order, spare): (&mut Vec<u64>, &mut Vec<Subrack>),
    (given, saved_shape): (Option<&[u8]>, Option<RackShape<'_>>),
) {
    walk_subracks(multi_leaves, places, rack_tally, first_places, spare);
    subracks.clear();
    match given {
        Some(given) => subracks.extend(given.iter().map(|&at| spare[at as usize])),
        None => {
            sort_subracks(spare, order);
            subracks.extend(order.iter().map(|&k| spare[(k & 0xffffff) as usize]));
        }
    }
    by_played.clear();
    if let Some(shape) = saved_shape {
        by_played.extend_from_slice(shape.by_played);
        return;
    }
    by_played.resize(num_tiles_on_rack as usize + 2, subracks.len() as u32);
    for (i, s) in subracks.iter().enumerate().rev() {
        by_played[s.num_played as usize] = i as u32;
    }
    for k in (0..by_played.len() - 1).rev() {
        by_played[k] = by_played[k].min(by_played[k + 1]);
    }
}

// a span's word multiplier, the product over the squares that keep one, and
// blank_ok, the letters a blank may be on its empty squares (only for a rack
// with a blank), on an instance's lanes in u16 sets (W)
#[inline(always)]
fn span_sets_u16(
    params: &TableWalkParams<
        '_,
        impl FnMut(i8, &[u8], i32, i32) -> i32,
        impl kwg::Node,
        impl kwg::Node,
    >,
    leftmost: i8,
    rightmost: i8,
    letters: u64,
) -> (i32, u64) {
    let span_bits = (u16::MAX >> (16 - rightmost as u32)) & (u16::MAX << leftmost as u32);
    let mut word_multiplier = 1i32;
    let mut multiplied = params.span_multiplied as u16 & span_bits;
    while multiplied != 0 {
        let idx = multiplied.trailing_zeros() as usize;
        multiplied &= multiplied - 1;
        word_multiplier *= i32::from(params.remaining_word_multipliers_strip[idx]);
    }
    let mut blank_ok = 0u64;
    if params.has_blank {
        let mut empty = params.span_empty as u16 & span_bits;
        while empty != 0 {
            let idx = empty.trailing_zeros() as usize;
            empty &= empty - 1;
            let bits = params.allowed_strip[idx];
            blank_ok |= bits & letters;
        }
    }
    (word_multiplier, blank_ok)
}

// span_sets_u16's sums in u32 sets (W)
#[inline(always)]
fn span_sets_u32(
    params: &TableWalkParams<
        '_,
        impl FnMut(i8, &[u8], i32, i32) -> i32,
        impl kwg::Node,
        impl kwg::Node,
    >,
    leftmost: i8,
    rightmost: i8,
    letters: u64,
) -> (i32, u64) {
    let span_bits = (u32::MAX >> (32 - rightmost as u32)) & (u32::MAX << leftmost as u32);
    let mut word_multiplier = 1i32;
    let mut multiplied = params.span_multiplied as u32 & span_bits;
    while multiplied != 0 {
        let idx = multiplied.trailing_zeros() as usize;
        multiplied &= multiplied - 1;
        word_multiplier *= i32::from(params.remaining_word_multipliers_strip[idx]);
    }
    let mut blank_ok = 0u64;
    if params.has_blank {
        let mut empty = params.span_empty as u32 & span_bits;
        while empty != 0 {
            let idx = empty.trailing_zeros() as usize;
            empty &= empty - 1;
            let bits = params.allowed_strip[idx];
            blank_ok |= bits & letters;
        }
    }
    (word_multiplier, blank_ok)
}

#[inline(always)]
fn gen_classic_place_moves<
    'a,
    W: LaneBits,
    T: TablePlaces,
    CallbackType: FnMut(i8, &[u8], i32, i32) -> i32,
    N: kwg::Node,
    L: kwg::Node,
>(
    params: &'a mut TableWalkParams<'a, CallbackType, N, L>,
    source: &'a anagrams::Anagrams,
    single_tile_plays: bool,
) {
    struct Env<
        'a,
        T: TablePlaces,
        CallbackType: FnMut(i8, &[u8], i32, i32) -> i32,
        N: kwg::Node,
        L: kwg::Node,
    > {
        params: &'a mut TableWalkParams<'a, CallbackType, N, L>,
        source: &'a anagrams::Anagrams,
        places: T::Places<'a>,
        alphabet: &'a alphabet::Alphabet,
        left: i8,
        right: i8,
        num_played: u8,
        base_main: i32,
        base_perp: i32,
        word_multiplier: i32,
        bound: i32,
        fixed: i32, // the span's fixed part, with its tiles' bonus
    }

    // the most the subrack whose places these are scores on the span: its
    // tiles' scores, best first, on the kept span's multipliers, best first,
    // plus the fixed part, no more than the span's score bound (no play of the
    // tiles on the squares scores more), or that bound when it has none
    #[inline(always)]
    fn own_bound<
        T: TablePlaces,
        CallbackType: FnMut(i8, &[u8], i32, i32) -> i32,
        N: kwg::Node,
        L: kwg::Node,
    >(
        env: &Env<'_, T, CallbackType, N, L>,
        mut places: u32,
    ) -> i32 {
        let multipliers = env.params.span_multipliers;
        if multipliers.is_empty() {
            return env.params.score_bound;
        }
        debug_assert_eq!(places.count_ones() as usize, multipliers.len());
        let scores = env.params.descending_scores;
        let mut bound = env.fixed;
        for &taken in multipliers {
            bound += taken.multiplier() * scores[places.trailing_zeros() as usize];
            places &= places - 1;
        }
        bound.min(env.params.score_bound)
    }

    #[inline]
    fn fit_words_apart<
        T: TablePlaces,
        CallbackType: FnMut(i8, &[u8], i32, i32) -> i32,
        N: kwg::Node,
        L: kwg::Node,
    >(
        env: &mut Env<'_, T, CallbackType, N, L>,
        found: alphagram::Words<'_>,
        leave_idx: u32,
        blank_letter: u8,
        first_blank: u8,
    ) {
        fit_words::<true, _, _, _, _>(env, found, leave_idx, blank_letter, first_blank)
    }

    #[inline(always)]
    fn fit_words<
        const BLANKED: bool,
        T: TablePlaces,
        CallbackType: FnMut(i8, &[u8], i32, i32) -> i32,
        N: kwg::Node,
        L: kwg::Node,
    >(
        env: &mut Env<'_, T, CallbackType, N, L>,
        found: alphagram::Words<'_>,
        leave_idx: u32,
        blank_letter: u8,
        first_blank: u8,
    ) {
        let len = (env.right - env.left) as u8;
        let board_strip = env.params.board_strip;
        let tile_multipliers = env.params.remaining_tile_multipliers_strip;
        let perpendicular_word_multipliers = env.params.perpendicular_word_multipliers_strip;
        let left = env.left as usize;
        let allowed = &env.params.allowed_strip[left..left + len as usize];
        debug_assert_eq!(found.len, len);
        'word: for word in found.iter() {
            let mut fits = 1u64;
            for (&a, &c) in allowed.iter().zip(word) {
                fits &= a >> c;
            }
            if fits & 1 == 0 {
                continue;
            }
            let mut main_score = env.base_main;
            let mut perpendicular_cumulative_score = env.base_perp;
            for (i, &c) in word.iter().enumerate() {
                let pos = left + i;
                if board_strip[pos] != 0 {
                    continue;
                }
                let tile_value =
                    env.alphabet.scaled_score(c).raw() * i32::from(tile_multipliers[pos]);
                main_score += tile_value;
                perpendicular_cumulative_score +=
                    tile_value * i32::from(perpendicular_word_multipliers[pos]);
            }
            for (i, &c) in word.iter().enumerate() {
                let pos = left + i;
                if board_strip[pos] == 0 {
                    env.params.word_strip_buffer[pos] = c;
                }
            }
            let score = main_score * env.word_multiplier
                + perpendicular_cumulative_score
                + i32::from(
                    env.params
                        .board_snapshot
                        .game_config
                        .num_played_bonus(env.num_played),
                ) * equity::SCALE;
            let leave_value = env.params.leave_values[leave_idx as usize];
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
            let blank_value = env.alphabet.scaled_score(0).raw();
            let real_value = env.alphabet.scaled_score(blank_letter).raw();
            if first_blank == 0 {
                for (i, &c) in word.iter().enumerate() {
                    let pos = left + i;
                    if c != blank_letter || board_strip[pos] != 0 {
                        continue;
                    }
                    let delta = (blank_value - real_value) * i32::from(tile_multipliers[pos]);
                    let blanked_score = score
                        + delta * env.word_multiplier
                        + delta * i32::from(perpendicular_word_multipliers[pos]);
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
            let first_value = env.alphabet.scaled_score(first_blank).raw();
            for (i, &c) in word.iter().enumerate() {
                let pos = left + i;
                if c != first_blank || board_strip[pos] != 0 {
                    continue;
                }
                let delta1 = (blank_value - first_value) * i32::from(tile_multipliers[pos]);
                let once_score = score
                    + delta1 * env.word_multiplier
                    + delta1 * i32::from(perpendicular_word_multipliers[pos]);
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
                    let delta2 = (blank_value - real_value) * i32::from(tile_multipliers[pos2]);
                    let blanked_score = once_score
                        + delta2 * env.word_multiplier
                        + delta2 * i32::from(perpendicular_word_multipliers[pos2]);
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

    // the walk span_prefix stands in for, kept to check it in debug builds
    #[cfg(debug_assertions)]
    #[inline(always)]
    fn add_square(
        e: &mut Extent,
        idx: usize,
        letters: u64,
        places: impl alphagram::KeyPlaces,
        params: &TableWalkParams<
            '_,
            impl FnMut(i8, &[u8], i32, i32) -> i32,
            impl kwg::Node,
            impl kwg::Node,
        >,
    ) {
        let b = params.board_strip[idx];
        if b != 0 {
            e.base_main += params.face_value_scores_strip[idx];
            e.playthrough_key += places.place_value(b & 0x7f);
            debug_assert!(places.holds(
                e.playthrough_key,
                &[alphagram::Nibble::MAX_COUNT; alphagram::Nibble::TILES]
            ));
        } else {
            let bits = params.allowed_strip[idx];
            if bits & 1 == 0 {
                e.free_squares += 1;
                e.blank_ok = letters;
            } else if bits == 1 {
                e.dead_squares += 1;
            } else {
                e.blank_ok |= bits & letters;
            }
            e.num_played += 1;
            e.word_multiplier *= i32::from(params.remaining_word_multipliers_strip[idx]);
            e.base_perp += params.perpendicular_scores_strip[idx];
        }
    }

    let alphabet = params.board_snapshot.game_config.alphabet();
    let places = T::of(source);
    let letters = (u64::MAX >> (64 - u32::from(alphabet.len()))) & !1;
    let leftmost = params.leftmost;
    let rightmost = params.rightmost;
    let num_max_played = params.num_max_played;
    let mut env = Env::<T, _, _, _> {
        params,
        source,
        places,
        alphabet,
        left: leftmost,
        right: rightmost,
        num_played: 0,
        base_main: 0,
        base_perp: 0,
        word_multiplier: 1,
        bound: 0,
        fixed: 0,
    };
    if rightmost - leftmost < 2 {
        return;
    }
    // the span's sums: two entries' difference in its lane's, as fill_span_lane
    // fills them; its word multiplier, the product over the squares that keep
    // one; and blank_ok, which only a subrack with a blank reads, found only
    // for a rack with a blank
    let lo = env.params.span_prefix[leftmost as usize];
    let hi = env.params.span_prefix[rightmost as usize];
    let (word_multiplier, blank_ok) = if W::BITS == u16::BITS {
        span_sets_u16(env.params, leftmost, rightmost, letters)
    } else if W::BITS == u32::BITS {
        span_sets_u32(env.params, leftmost, rightmost, letters)
    } else {
        let span_bits = (u128::MAX >> (128 - rightmost as u32)) & (u128::MAX << leftmost as u32);
        let mut word_multiplier = 1i32;
        let mut multiplied = env.params.span_multiplied & span_bits;
        while multiplied != 0 {
            let idx = multiplied.trailing_zeros() as usize;
            multiplied &= multiplied - 1;
            word_multiplier *= i32::from(env.params.remaining_word_multipliers_strip[idx]);
        }
        let mut blank_ok = 0u64;
        if env.params.has_blank {
            let mut empty = env.params.span_empty & span_bits;
            while empty != 0 {
                let idx = empty.trailing_zeros() as usize;
                empty &= empty - 1;
                let bits = env.params.allowed_strip[idx];
                blank_ok |= bits & letters;
            }
        }
        (word_multiplier, blank_ok)
    };
    let e = Extent {
        playthrough_key: hi.key - lo.key,
        base_main: hi.face - lo.face,
        base_perp: hi.perp - lo.perp,
        word_multiplier,
        num_played: hi.played - lo.played,
        free_squares: hi.free - lo.free,
        dead_squares: hi.dead - lo.dead,
        blank_ok,
    };
    #[cfg(debug_assertions)]
    {
        let mut walked = Extent {
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
            add_square(&mut walked, idx as usize, letters, env.places, env.params);
        }
        debug_assert_eq!(walked.playthrough_key, e.playthrough_key);
        debug_assert_eq!(walked.base_main, e.base_main);
        debug_assert_eq!(walked.base_perp, e.base_perp);
        debug_assert_eq!(walked.word_multiplier, e.word_multiplier);
        debug_assert_eq!(walked.num_played, e.num_played);
        debug_assert_eq!(walked.free_squares, e.free_squares);
        debug_assert_eq!(walked.dead_squares, e.dead_squares);
        debug_assert!(!env.params.has_blank || walked.blank_ok == e.blank_ok);
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
    env.fixed = e.base_main * e.word_multiplier
        + e.base_perp
        + i32::from(
            env.params
                .board_snapshot
                .game_config
                .num_played_bonus(e.num_played),
        ) * equity::SCALE;
    let len = (rightmost - leftmost) as u8;
    // the playthrough's part, which each subrack's adds to
    let playthrough_part = anagrams::key_part(e.playthrough_key);
    match env.params.span_memo.find(
        e.playthrough_key,
        playthrough_part,
        len,
        env.params.score_bound,
    ) {
        Ok((from, upto)) => {
            for i in from..upto {
                let (si, at) = env.params.span_memo.visits[i];
                if !visit::<_, _, _, _>(&mut env, &e, len, si as usize, Some((at, i))) {
                    break;
                }
            }
        }
        Err(at) => {
            let from = env.params.subracks_by_played[e.num_played as usize] as usize;
            let upto = env.params.subracks_by_played[e.num_played as usize + 1] as usize;
            let kept_from = env.params.span_memo.visits.len();
            let width = u64::BITS as usize;
            let mut base = from;
            let mut cut_at = i32::MAX;
            'runs: while base < upto {
                let end = (base + width).min(upto);
                let mut may = 0u64;
                let mut cut = end;
                for i in base..end {
                    let subrack = &env.params.subracks[i];
                    if env.params.score_bound.saturating_add(subrack.leave_value)
                        < env.params.threshold
                    {
                        cut = i;
                        break;
                    }
                    let part = playthrough_part.wrapping_add(subrack.part);
                    let held = match subrack.blanks {
                        0 => env.source.may_hold_part(part),
                        1 => env.source.blank_may_hold_part(part),
                        _ => true,
                    };
                    may |= u64::from(held) << (i - base);
                }
                while may != 0 {
                    let si = base + may.trailing_zeros() as usize;
                    may &= may - 1;
                    if !visit::<_, _, _, _>(&mut env, &e, len, si, None) {
                        cut_at = env.params.score_bound;
                        break 'runs;
                    }
                }
                if cut < end {
                    cut_at = env.params.score_bound;
                    break;
                }
                base = end;
            }
            env.params
                .span_memo
                .keep(at, e.playthrough_key, len, kept_from, cut_at);
        }
    }

    // a subrack on the span, false once its bound is below the threshold (a
    // size's subracks fall in leave, so none after it can be); kept is the
    // memo's (the group, UNSEEN_GROUP when not looked up yet, or 0 for a
    // subrack with a blank) with its place in the memo, None on the first
    // visit, whose key the filter has passed. One whose own tiles cannot reach
    // the threshold (own_bound) is passed over, its group not looked up.
    #[inline(always)]
    fn visit<
        T: TablePlaces,
        CallbackType: FnMut(i8, &[u8], i32, i32) -> i32,
        N: kwg::Node,
        L: kwg::Node,
    >(
        env: &mut Env<'_, T, CallbackType, N, L>,
        e: &Extent,
        len: u8,
        si: usize,
        kept: Option<(u32, usize)>,
    ) -> bool {
        let subrack = env.params.subracks[si];
        env.bound = env.params.score_bound.saturating_add(subrack.leave_value);
        if env.bound < env.params.threshold {
            return false;
        }
        env.bound = own_bound(env, subrack.places).saturating_add(subrack.leave_value);
        let short = env.bound < env.params.threshold;
        let key = e.playthrough_key + subrack.key;
        let source = env.source;
        if subrack.blanks == 0 {
            let at = match kept {
                Some((at, _)) if at != UNSEEN_GROUP => at,
                // looked up once a span needs it; one the filter passed in
                // error stays UNSEEN_GROUP
                Some((_, i)) => {
                    if short {
                        return true;
                    }
                    let Some(at) = source.group_at_held(alphagram::Fitted(key), len) else {
                        return true;
                    };
                    env.params.span_memo.visits[i].1 = at;
                    at
                }
                None => {
                    if short {
                        env.params.span_memo.visits.push((si as u32, UNSEEN_GROUP));
                        return true;
                    }
                    let Some(at) = source.group_at_held(alphagram::Fitted(key), len) else {
                        return true;
                    };
                    env.params.span_memo.visits.push((si as u32, at));
                    at
                }
            };
            if short {
                return true;
            }
            debug_assert_ne!(at, UNSEEN_GROUP);
            fit_words::<false, _, _, _, _>(env, source.words_of(len, at), subrack.leave_idx, 0, 0);
            return true;
        }
        if kept.is_none() {
            env.params.span_memo.visits.push((si as u32, 0));
        }
        if short {
            return true;
        }
        if subrack.blanks >= 3 {
            // the blanks' letters, on the stack: a subrack holds at most
            // u8::MAX of them
            let mut letters = [0u8; u8::MAX as usize];
            let letters = &mut letters[..subrack.blanks as usize];
            visit_blanks(env, e, len, subrack.leave_idx, key, letters, 0);
            return true;
        }
        if subrack.blanks >= 2 {
            // the most of each tile a key counts (nibble keys read it, a layout
            // holds its own)
            let counts = env.source.layout().max_counts();
            let key_holds = env.places.holds(key, counts);
            let mut first = e.blank_ok & !1;
            while first != 0 {
                let l1 = first.trailing_zeros() as u8;
                first &= first - 1;
                let key1 = key + env.places.place_value(l1);
                let within =
                    key_holds && env.places.count_in(key, l1) < env.places.max_count(l1, counts);
                // nibble keys count each tile only as far as a word holds it,
                // so past that a key holds no word (nor stays exact)
                if !within && (T::NIBBLES || !env.places.holds(key1, counts)) {
                    continue;
                }
                source.blank_groups(
                    alphagram::Fitted(key1),
                    len,
                    e.blank_ok,
                    #[inline(never)]
                    |tile, at| {
                        if tile < l1 {
                            return;
                        }
                        let found = source.words_at(at);
                        fit_words_apart(env, found, subrack.leave_idx, tile, l1);
                    },
                );
            }
            return true;
        }
        source.blank_groups(
            alphagram::Fitted(key),
            len,
            e.blank_ok,
            #[inline(never)]
            |tile, at| {
                let found = source.words_at(at);
                fit_words_apart(env, found, subrack.leave_idx, tile, 0);
            },
        );
        true
    }

    // a subrack of three blanks or more: the letters of all but the last blank,
    // each no less than the one before and within what a word holds, then the
    // last blank's by the blanked table, no less than the one before it, so
    // each set of letters is found once
    #[cold]
    fn visit_blanks<
        T: TablePlaces,
        CallbackType: FnMut(i8, &[u8], i32, i32) -> i32,
        N: kwg::Node,
        L: kwg::Node,
    >(
        env: &mut Env<'_, T, CallbackType, N, L>,
        e: &Extent,
        len: u8,
        leave_idx: u32,
        key: u128,
        letters: &mut [u8],
        chosen: usize,
    ) {
        let source = env.source;
        let least = if chosen == 0 { 1 } else { letters[chosen - 1] };
        if chosen + 1 == letters.len() {
            source.blank_groups(alphagram::Fitted(key), len, e.blank_ok, |tile, at| {
                if tile < least {
                    return;
                }
                letters[chosen] = tile;
                fit_words_blanks(env, source.words_at(at), leave_idx, letters);
            });
            return;
        }
        let mut bits = e.blank_ok & !((1u64 << least) - 1);
        while bits != 0 {
            let letter = bits.trailing_zeros() as u8;
            bits &= bits - 1;
            let next = key + env.places.place_value(letter);
            // past what a word holds of the letter, no word (and on nibble keys
            // no exact key)
            if !env.places.holds(next, env.source.layout().max_counts()) {
                continue;
            }
            letters[chosen] = letter;
            visit_blanks(env, e, len, leave_idx, next, letters, chosen + 1);
        }
    }

    // fit_words for a subrack of three blanks or more, letters ascending: each
    // found word with a blank on a square of each letter, the squares of one
    // letter's blanks ascending, so each set of squares is played once
    #[cold]
    fn fit_words_blanks<
        T: TablePlaces,
        CallbackType: FnMut(i8, &[u8], i32, i32) -> i32,
        N: kwg::Node,
        L: kwg::Node,
    >(
        env: &mut Env<'_, T, CallbackType, N, L>,
        found: alphagram::Words<'_>,
        leave_idx: u32,
        letters: &[u8],
    ) {
        let len = (env.right - env.left) as u8;
        let left = env.left as usize;
        debug_assert_eq!(found.len, len);
        for word in found.iter() {
            let allowed = &env.params.allowed_strip[left..left + len as usize];
            let mut fits = 1u64;
            for (&a, &c) in allowed.iter().zip(word) {
                fits &= a >> c;
            }
            if fits & 1 == 0 {
                continue;
            }
            let mut main_score = env.base_main;
            let mut perpendicular_cumulative_score = env.base_perp;
            for (i, &c) in word.iter().enumerate() {
                let pos = left + i;
                if env.params.board_strip[pos] != 0 {
                    continue;
                }
                let tile_value = env.alphabet.scaled_score(c).raw()
                    * i32::from(env.params.remaining_tile_multipliers_strip[pos]);
                main_score += tile_value;
                perpendicular_cumulative_score +=
                    tile_value * i32::from(env.params.perpendicular_word_multipliers_strip[pos]);
                env.params.word_strip_buffer[pos] = c;
            }
            let score = main_score * env.word_multiplier
                + perpendicular_cumulative_score
                + i32::from(
                    env.params
                        .board_snapshot
                        .game_config
                        .num_played_bonus(env.num_played),
                ) * equity::SCALE;
            let leave_value = env.params.leave_values[leave_idx as usize];
            place_blanks(env, word, letters, 0, 0, score, leave_value);
        }
    }

    // the blanks from the i-th letter on put on the word's squares, a letter's
    // from square from on when it repeats the letter before, then the play
    #[cold]
    fn place_blanks<
        T: TablePlaces,
        CallbackType: FnMut(i8, &[u8], i32, i32) -> i32,
        N: kwg::Node,
        L: kwg::Node,
    >(
        env: &mut Env<'_, T, CallbackType, N, L>,
        word: &[u8],
        letters: &[u8],
        i: usize,
        from: usize,
        score: i32,
        leave_value: i32,
    ) {
        if i == letters.len() {
            debug_assert!(
                score + leave_value <= env.bound,
                "found {} when the bound for {} tiles was {}",
                score + leave_value,
                env.num_played,
                env.bound,
            );
            env.params.threshold = (env.params.callback)(
                env.left,
                &env.params.word_strip_buffer[env.left as usize..env.right as usize],
                score,
                leave_value,
            );
            return;
        }
        let letter = letters[i];
        let start = if i > 0 && letters[i - 1] == letter {
            from
        } else {
            0
        };
        let left = env.left as usize;
        let blank_value = env.alphabet.scaled_score(0).raw();
        let real_value = env.alphabet.scaled_score(letter).raw();
        for (j, &c) in word.iter().enumerate().skip(start) {
            let pos = left + j;
            if c != letter || env.params.board_strip[pos] != 0 {
                continue;
            }
            let delta = (blank_value - real_value)
                * i32::from(env.params.remaining_tile_multipliers_strip[pos]);
            let blanked = score
                + delta * env.word_multiplier
                + delta * i32::from(env.params.perpendicular_word_multipliers_strip[pos]);
            env.params.word_strip_buffer[pos] = c | 0x80;
            place_blanks(env, word, letters, i + 1, j + 1, blanked, leave_value);
            env.params.word_strip_buffer[pos] = c;
        }
    }
}

// rack multisets completing a word with one set of forced letters, by size:
// each set of forced letters a generation meets is walked once, its lists kept
// in by_size from the start kept beside its letters, for the generation's other
// spans with the same letters. The lists are held end to end in one buffer,
// list i ending at size_ends[i], so that the room kept from walk to walk is
// one walk's lists together, not each list's largest; a set's walk finds its
// multisets (found, each with its size) and puts them in its lists at its end.
// jumbled only: each span's multiset is one word's, so no jumbled woadwage
// game walks it
#[derive(Clone)]
struct JumbledMultisets {
    multisets: Vec<u8>,
    by_size: Vec<usize>,
    size_ends: Vec<usize>,
    found: Vec<u8>,    // the sizes of the multisets a set's walk found
    found_from: usize, // where in multisets the first of them is
    kept: Vec<([u8; MAX_ALPHABET_LEN], usize)>,
    current: Vec<u8>,
    walk: u8,
    splits: JumbledSplits,
    window: std::ops::Range<u64>,
    through: Option<i8>,
    seen: u64,
    held: bool,
    floor: i32, // a threshold the walk starts from, set only inside a call

    part: (u64, u32), // the (multiset, blank choice) a part walk starts from
    budget: u64,      // blank choices a part walk has left
    designation: u32, // the current multiset's next blank choice
    stopped: Option<(u64, u32)>, // where a part walk stopped
}

pub struct PoolLists(JumbledMultisets);

pub struct PoolWindow {
    pub lanes: (u128, u128),
    pub window: std::ops::Range<u64>,
}

type JumbledWoadwageFound = (i8, i8, bites::Bites, i32, i32, bites::Bites);

#[derive(Default)]
struct WoadwageScratch {
    stack: Vec<i32>,
    letters: Vec<u8>,    // the span's letters, board tiles' among them
    prefixes: Vec<bool>, // by length, whether idx_left's span that long is a jumble
    jumbles: crate::fash::MyHashMap<[u8; crate::census::MAX_LETTERS], bool>, // asked: counts, answer
    found: Vec<JumbledWoadwageFound>,
    placed: Vec<u8>,
}

#[derive(Default)]
struct PoolScratch {
    walked: Vec<u8>,
    letters: Vec<u8>,
    deltas: Vec<(u8, i32)>,
    word: Vec<u8>,
}

// a windowed top walk's threshold so far, carried from one window to the next
pub struct PoolFloor(i32);

// where a part walk of a window stopped: the multiset, and its blank choice
// the next part starts from
#[derive(Default)]
pub struct PoolPart(Option<(u64, u32)>);

impl PoolPart {
    // the multiset the walk stopped in, none when it walked its window
    pub fn stopped_at(&self) -> Option<u64> {
        self.0.map(|(at, _)| at)
    }
}

impl Default for PoolFloor {
    fn default() -> Self {
        Self(i32::MIN)
    }
}

impl Default for PoolLists {
    fn default() -> Self {
        Self(JumbledMultisets::new())
    }
}

impl PoolLists {
    // as default makes them, every list's room kept for the next lane
    pub fn clear(&mut self) {
        self.0.clear();
    }

    // as clear makes them, every list's room let go
    pub fn shrink(&mut self) {
        self.0.shrink();
    }
}

const JUMBLED_EVERY_ARRANGEMENT: u8 = 0;
const JUMBLED_ONCE_PER_SCORE: u8 = 1;
const JUMBLED_ONCE_PER_RACK_TILES: u8 = 2;
const JUMBLED_BEST_PER_RACK_TILES: u8 = 3;

mod jumbled_walk {
    pub trait Sealed {}
}

#[derive(Clone, Copy)]
pub struct HeldBytes {
    pub memory: endgame::MemoryLimit,
    pub kept: usize,
}

trait RawStop {
    const STOPS: bool;
    fn reserve(&self, plays: usize, grown_bytes: usize) -> Option<endgame::Reserved>;
}

impl RawStop for () {
    const STOPS: bool = false;

    #[inline(always)]
    fn reserve(&self, _plays: usize, _grown_bytes: usize) -> Option<endgame::Reserved> {
        Some(endgame::Reserved::NONE)
    }
}

impl RawStop for HeldBytes {
    const STOPS: bool = true;

    #[inline(always)]
    fn reserve(&self, plays: usize, grown_bytes: usize) -> Option<endgame::Reserved> {
        if (self.memory.held)() == 0 {
            return Some(endgame::Reserved::NONE);
        }
        (self.memory).reserve(plays.saturating_mul(grown_bytes + 2 * self.kept))
    }
}

pub trait JumbledWalk: jumbled_walk::Sealed {
    const WALK: u8;
}

pub struct EveryArrangement;

pub struct OncePerScore;

impl jumbled_walk::Sealed for EveryArrangement {}

impl jumbled_walk::Sealed for OncePerScore {}

impl JumbledWalk for EveryArrangement {
    const WALK: u8 = JUMBLED_EVERY_ARRANGEMENT;
}

impl JumbledWalk for OncePerScore {
    const WALK: u8 = JUMBLED_ONCE_PER_SCORE;
}

#[derive(Clone, Default)]
struct JumbledSplits {
    squares: Vec<i8>,
    square_classes: Vec<u8>,
    class_weights: Vec<i32>,
    class_penalties: Vec<i32>,
    class_sizes: Vec<u8>,
    vowels_matter: bool,
    letters: Vec<u8>,
    letter_counts: Vec<u8>,
    values: Vec<(i32, bool)>,
    real_class: Vec<u8>,
    blank_class: Vec<u8>,
    stride: usize,
    members: Vec<(i32, i32, i32, usize)>,
    member_data: Vec<u8>,
    order: Vec<usize>,
    group: Vec<usize>,
    blanks: Vec<u8>,
    value_left: Vec<u8>,
    class_left: Vec<u8>,
    matrix: Vec<u8>,
    placed: Vec<u8>,
    blanks_used: Vec<u8>,
    reals_used: Vec<u8>,
    sorted_weights: Vec<i32>,
    narrowed: Vec<u8>,
    letter_values: Vec<i32>, // each letter's scaled score, as letters
    by_value: Vec<u8>,       // letters' places, their scores highest first
    blank_value: i32,
    starts: Vec<usize>,
    out_of_memory: bool,
}

// the distinct orders of tiles in these counts: n! over each count's !
fn arrangements(counts: &[u8]) -> usize {
    let mut ways = 1u128;
    let mut placed = 0u128;
    for &count in counts {
        for i in 1..=u128::from(count) {
            placed += 1;
            let Some(more) = ways.checked_mul(placed) else {
                return usize::MAX;
            };
            ways = more / i;
        }
    }
    usize::try_from(ways).unwrap_or(usize::MAX)
}

impl JumbledSplits {
    // room for a start for each way of designating the multiset's blanks, and
    // one past the last; false, with out_of_memory set, where it is refused.
    // Apart from the walk, as sort_by_value
    #[inline(never)]
    fn reserve_starts(&mut self) -> bool {
        let designations = (self.letter_counts.iter()).fold(1usize, |ways, &count| {
            ways.saturating_mul(count as usize + 1)
        });
        if self
            .starts
            .try_reserve(designations.saturating_add(1))
            .is_err()
        {
            self.out_of_memory = true;
            return false;
        }
        true
    }

    // room for every way to share this designation's tiles among the classes,
    // each a member: no more than the orders of its tiles by value, nor than
    // their orders by class; false, with out_of_memory set, where it is
    // refused. Apart from the walk, as sort_by_value
    #[inline(never)]
    fn reserve_members(&mut self) -> bool {
        let ways = arrangements(&self.value_left).min(arrangements(&self.class_sizes));
        let stride = self.blanks.len() + self.matrix.len();
        if self.members.try_reserve(ways).is_err()
            || (self.member_data)
                .try_reserve(ways.saturating_mul(stride))
                .is_err()
        {
            self.out_of_memory = true;
            return false;
        }
        true
    }

    // the multiset's letters by score, sorted once for every split's bound;
    // apart from the walk, so its one copy serves every walk the generator has
    #[inline(never)]
    fn sort_by_value(&mut self, alphabet: &alphabet::Alphabet) {
        self.letter_values.clear();
        (self.letter_values).extend(
            self.letters
                .iter()
                .map(|&letter| alphabet.scaled_score(letter).raw()),
        );
        self.by_value.clear();
        self.by_value.extend(0..self.letters.len() as u8);
        let letter_values = &self.letter_values;
        (self.by_value)
            .sort_unstable_by(|&a, &b| letter_values[b as usize].cmp(&letter_values[a as usize]));
        self.blank_value = alphabet.scaled_score(0).raw();
    }

    // from plus the tiles' scores highest first, each by its weight: merged
    // from the letters sorted once and the blanks, which all score the same;
    // apart from the walk, as sort_by_value
    #[inline(never)]
    fn tiles_bound(&self, from: i32) -> i32 {
        let mut bound = from;
        let mut weights = self.sorted_weights.iter();
        let mut blanks_left = self.blanks.iter().sum::<u8>();
        'weigh: for &j in self.by_value.iter() {
            let j = j as usize;
            let value = self.letter_values[j];
            while blanks_left != 0 && self.blank_value > value {
                let Some(&weight) = weights.next() else {
                    break 'weigh;
                };
                bound += self.blank_value * weight;
                blanks_left -= 1;
            }
            for _ in 0..self.letter_counts[j] - self.blanks[j] {
                let Some(&weight) = weights.next() else {
                    break 'weigh;
                };
                bound += value * weight;
            }
        }
        for &weight in weights.take(blanks_left as usize) {
            bound += self.blank_value * weight;
        }
        bound
    }
}

impl JumbledMultisets {
    #[inline(always)]
    fn new() -> Self {
        Self {
            multisets: Vec::new(),
            by_size: Vec::new(),
            size_ends: Vec::new(),
            found: Vec::new(),
            found_from: 0,
            kept: Vec::new(),
            current: Vec::new(),
            walk: JUMBLED_EVERY_ARRANGEMENT,
            splits: JumbledSplits::default(),
            window: 0..u64::MAX,
            through: None,
            seen: 0,
            held: false,
            floor: i32::MIN,
            part: (0, 0),
            budget: u64::MAX,
            designation: 0,
            stopped: None,
        }
    }

    // as clear makes it, the room of each list that grows with the multisets
    // walked (its splits' too) let go
    fn shrink(&mut self) {
        self.clear();
        self.by_size.shrink_to_fit();
        self.size_ends.shrink_to_fit();
        self.found.shrink_to_fit();
        self.multisets.shrink_to_fit();
        self.kept.shrink_to_fit();
        let s = &mut self.splits;
        s.members.clear();
        s.members.shrink_to_fit();
        for list in [&mut s.order, &mut s.group, &mut s.starts] {
            list.clear();
            list.shrink_to_fit();
        }
        for list in [&mut s.member_data, &mut s.matrix] {
            list.clear();
            list.shrink_to_fit();
        }
    }

    // as new makes it, its lists' room kept: the splits' scratch is filled by
    // each walk that reads it, as a generator's own is from one generation to
    // the next
    #[inline(always)]
    fn clear(&mut self) {
        self.by_size.clear();
        self.size_ends.clear();
        self.found.clear();
        self.multisets.clear();
        self.kept.clear();
        self.current.clear();
        self.walk = JUMBLED_EVERY_ARRANGEMENT;
        self.splits.out_of_memory = false;
        self.window = 0..u64::MAX;
        self.through = None;
        self.seen = 0;
        self.held = false;
        self.floor = i32::MIN;
        self.part = (0, 0);
        self.budget = u64::MAX;
        self.designation = 0;
        self.stopped = None;
    }

    // a new generation's rack: no set of forced letters is kept
    #[inline(always)]
    fn forget(&mut self) {
        if self.held {
            return;
        }
        self.multisets.clear();
        self.kept.clear();
        self.splits.out_of_memory = false;
    }

    #[inline(never)]
    fn refuse(&mut self) -> bool {
        self.splits.out_of_memory = true;
        false
    }

    // the first of the lists for these forced letters, and whether they are
    // kept already; if not, the lists after the kept sets' are let go for the
    // walk, which finds its multisets (found) for put_found
    #[inline(always)]
    fn find(&mut self, forced: &[u8; MAX_ALPHABET_LEN], max_size: usize) -> (usize, bool) {
        if let Some(&(_, start)) = self.kept.iter().find(|(letters, _)| letters == forced) {
            return (start, true);
        }
        let Some(end) = self
            .kept
            .len()
            .checked_add(1)
            .and_then(|sets| sets.checked_mul(max_size + 1))
        else {
            self.splits.out_of_memory = true;
            return (0, true);
        };
        let start = end - (max_size + 1);
        if self.kept.try_reserve(1).is_err()
            || self
                .size_ends
                .try_reserve(end.saturating_sub(self.size_ends.len()))
                .is_err()
        {
            self.splits.out_of_memory = true;
            return (0, true);
        }
        self.size_ends.truncate(start);
        let end = self.size_ends.last().map_or(0, |&end| end);
        self.by_size.truncate(end);
        self.found.clear();
        self.found_from = self.multisets.len();
        self.kept.push((*forced, start));
        self.current.clear();
        (start, false)
    }

    // the multisets a set's walk found put in its max_size + 1 lists by size,
    // after the lists before them, each list in the order found
    #[inline(never)]
    fn put_found(&mut self, max_size: usize) {
        let from = self.by_size.len();
        if self.by_size.try_reserve(self.found.len()).is_err() {
            self.splits.out_of_memory = true;
            return;
        }
        let lists = self.size_ends.len();
        self.size_ends.resize(lists + max_size + 1, 0);
        let ends = &mut self.size_ends[lists..];
        for &size in &self.found {
            ends[size as usize] += 1;
        }
        let mut at = from;
        for end in ends.iter_mut() {
            let count = *end;
            *end = at;
            at += count;
        }
        self.by_size.resize(from + self.found.len(), 0);
        let mut offset = self.found_from;
        for &size in &self.found {
            let end = &mut ends[size as usize];
            self.by_size[*end] = offset;
            *end += 1;
            offset += usize::from(size);
        }
    }

    // where list i lies in by_size
    #[inline(always)]
    fn list(&self, i: usize) -> std::ops::Range<usize> {
        let start = i.checked_sub(1).map_or(0, |before| self.size_ends[before]);
        start..self.size_ends[i]
    }
}

#[inline(always)]
fn stops_walk(threshold: i32) -> bool {
    threshold == equity::Equity::INFINITY.raw()
}

// BAG_TERM: the generator's, as leave_value_of's. jumbled only: a span's
// multiset is one word's, so the caller never gives it a jumbled woadwage game
#[inline]
fn gen_jumbled_place_moves<
    'a,
    const BAG_TERM: bool,
    CallbackType: FnMut(i8, &[u8], i32, i32) -> i32,
    N: kwg::Node,
    L: kwg::Node,
>(
    params: &'a mut GenPlaceMovesParams<'a, CallbackType, N, L>,
    single_tile_plays: bool,
    opening: Option<bool>,
) {
    if params.jumbled.splits.out_of_memory || stops_walk(params.threshold) {
        return;
    }
    match params.jumbled.walk {
        JUMBLED_EVERY_ARRANGEMENT => {
            gen_jumbled_place_moves_walk::<BAG_TERM, JUMBLED_EVERY_ARRANGEMENT, _, _, _>(
                params,
                single_tile_plays,
                opening,
            )
        }
        JUMBLED_ONCE_PER_SCORE => {
            gen_jumbled_place_moves_walk::<BAG_TERM, JUMBLED_ONCE_PER_SCORE, _, _, _>(
                params,
                single_tile_plays,
                opening,
            )
        }
        JUMBLED_BEST_PER_RACK_TILES => {
            gen_jumbled_place_moves_walk::<BAG_TERM, JUMBLED_BEST_PER_RACK_TILES, _, _, _>(
                params,
                single_tile_plays,
                opening,
            )
        }
        _ => gen_jumbled_place_moves_walk::<BAG_TERM, JUMBLED_ONCE_PER_RACK_TILES, _, _, _>(
            params,
            single_tile_plays,
            opening,
        ),
    }
}

#[inline]
fn gen_jumbled_place_moves_walk<
    'a,
    const BAG_TERM: bool,
    const WALK: u8,
    CallbackType: FnMut(i8, &[u8], i32, i32) -> i32,
    N: kwg::Node,
    L: kwg::Node,
>(
    params: &'a mut GenPlaceMovesParams<'a, CallbackType, N, L>,
    single_tile_plays: bool,
    opening: Option<bool>,
) {
    struct Env<'a, CallbackType: FnMut(i8, &[u8], i32, i32) -> i32, N: kwg::Node, L: kwg::Node> {
        params: &'a mut GenPlaceMovesParams<'a, CallbackType, N, L>,
        alphabet: &'a alphabet::Alphabet,
        idx_left: i8,
        idx_right: i8, // exclusive
        num_played: u8,
        unplaced: [u8; MAX_ALPHABET_LEN], // the multiset's letters not yet placed
        unplaced_bits: u64,
        square_bounded: bool,
        span_word_multiplier: i32,
        square_weights: [i32; JUMBLED_BOUND_SQUARES], // by place in the span
        span_leave: i32,
        bound_weights: [i32; JUMBLED_BOUND_TILES], // fill_span's bound: empty squares' weights
        bound_scores: [i32; JUMBLED_BOUND_TILES],  // and letters' scores, a step's first entries
    }
    struct Accumulator {
        main_score: i32,
        perpendicular_cumulative_score: i32,
        word_multiplier: i32,
        leave_idx: u32,
    }
    struct Collect<'a, N: kwg::Node> {
        kwg: &'a kwg::Kwg<N>,
        alphabet_len: u8,
        forced: &'a [u8; MAX_ALPHABET_LEN],
        rack_tally: &'a [u8],
        max_size: u8,
        out: &'a mut JumbledMultisets,
    }

    // letters in ascending order, as the kad is indexed
    fn collect_multisets<N: kwg::Node>(
        c: &mut Collect<'_, N>,
        mut p: i32,
        letter: u8,
        blanks_left: u8,
        size: u8,
    ) {
        if letter >= c.alphabet_len {
            if size != 0 && c.kwg.node_in_run(p).accepts() {
                // the lists grow as the walk fills them, each growth reserved
                // (kept from walk to walk, so they grow once); refused, the
                // walk is out of memory
                let out = &mut *c.out;
                if (out.found.len() == out.found.capacity() && out.found.try_reserve(1).is_err())
                    || (out.multisets.capacity() - out.multisets.len() < out.current.len()
                        && out.multisets.try_reserve(out.current.len()).is_err())
                {
                    out.refuse();
                    return;
                }
                out.found.push(size);
                out.multisets.extend_from_slice(&out.current);
            }
            return;
        }
        for _ in 0..c.forced[letter as usize] {
            p = c.kwg.seek(p, letter);
            if p <= 0 {
                return;
            }
        }
        collect_multisets(c, p, letter + 1, blanks_left, size);
        // a blank may stand for a letter held or not
        let held = c.rack_tally[letter as usize];
        let most = (held + blanks_left).min(c.max_size - size);
        let mut pushed = 0;
        for taken in 1..=most {
            p = c.kwg.seek(p, letter);
            if p <= 0 {
                break;
            }
            c.out.current.push(letter);
            pushed += 1;
            collect_multisets(
                c,
                p,
                letter + 1,
                blanks_left - taken.saturating_sub(held),
                size + taken,
            );
        }
        let len = c.out.current.len();
        c.out.current.truncate(len - pushed);
    }

    #[inline(always)]
    fn record<
        const BAG_TERM: bool,
        CallbackType: FnMut(i8, &[u8], i32, i32) -> i32,
        N: kwg::Node,
        L: kwg::Node,
    >(
        env: &mut Env<'_, CallbackType, N, L>,
        acc: &Accumulator,
    ) {
        let score = acc.main_score * acc.word_multiplier
            + acc.perpendicular_cumulative_score
            + i32::from(
                env.params
                    .board_snapshot
                    .game_config
                    .num_played_bonus(env.num_played),
            ) * equity::SCALE;
        let leave_value = leave_value_of::<BAG_TERM, _>(
            env.params.multi_leaves,
            env.params.board_snapshot.klv,
            env.params.rack_tally,
            acc.leave_idx,
            env.params.num_tiles_in_bag,
            env.params.play_out_bonus,
            env.alphabet,
        );
        // a play under the threshold is not offered, as the classic walk's; the
        // census's sheet takes every play
        if !env.params.is_census && score + leave_value < env.params.threshold {
            return;
        }
        env.params.threshold = (env.params.callback)(
            env.idx_left,
            &env.params.word_strip_buffer[env.idx_left as usize..env.idx_right as usize],
            score,
            leave_value,
        );
    }

    fn walk_splits<
        const BAG_TERM: bool,
        const WALK: u8,
        CallbackType: FnMut(i8, &[u8], i32, i32) -> i32,
        N: kwg::Node,
        L: kwg::Node,
    >(
        env: &mut Env<'_, CallbackType, N, L>,
        at: usize,
        fixed: i32,
    ) {
        if env.params.jumbled.splits.out_of_memory {
            return;
        }
        let mut s = std::mem::take(&mut env.params.jumbled.splits);
        let num_played = env.num_played as usize;
        let alphabet = env.alphabet;
        s.letters.clear();
        s.letter_counts.clear();
        for offset in 0..num_played {
            let tile = env.params.jumbled.multisets[at + offset];
            if s.letters.last() == Some(&tile) {
                *s.letter_counts.last_mut().unwrap() += 1;
            } else {
                s.letters.push(tile);
                s.letter_counts.push(1);
            }
        }
        let num_letters = s.letters.len();
        let by_tiles = WALK == JUMBLED_ONCE_PER_RACK_TILES;
        s.sort_by_value(alphabet);
        s.values.clear();
        s.real_class.clear();
        s.blank_class.clear();
        for i in 0..num_letters {
            let letter = s.letters[i];
            let vowel = s.vowels_matter && alphabet.is_vowel(letter);
            for (value, class) in [
                (s.letter_values[i], &mut s.real_class),
                (s.blank_value, &mut s.blank_class),
            ] {
                let k = match s.values.iter().position(|&v| v == (value, vowel)) {
                    Some(k) => k,
                    None => {
                        s.values.push((value, vowel));
                        s.values.len() - 1
                    }
                };
                class.push(k as u8);
            }
        }
        let num_values = s.values.len();
        let num_classes = s.class_sizes.len();
        let cells = num_classes * num_values;
        s.stride = num_letters + cells;
        s.members.clear();
        s.member_data.clear();
        s.blanks.clear();
        s.blanks.resize(num_letters, 0);
        s.starts.clear();
        if !s.reserve_starts() {
            env.params.jumbled.splits = s;
            return;
        }
        designate_blanks::<BAG_TERM, _, _, _>(env, &mut s, 0, env.params.rack_tally[0], fixed);
        if WALK == JUMBLED_BEST_PER_RACK_TILES {
            walk_best_splits(env, &mut s, cells, num_letters);
            env.params.jumbled.splits = s;
            return;
        }
        s.order.clear();
        if s.out_of_memory || s.order.try_reserve(s.members.len()).is_err() {
            s.out_of_memory = true;
            env.params.jumbled.splits = s;
            return;
        }
        s.order.extend(0..s.members.len());
        let members = &s.members;
        let member_data = &s.member_data;
        let blanks_of = |m: &(i32, i32, i32, usize)| {
            if by_tiles {
                &member_data[m.3..m.3 + num_letters]
            } else {
                &[]
            }
        };
        s.order.sort_unstable_by(|&a, &b| {
            let (a, b) = (&members[a], &members[b]);
            (b.1, b.0)
                .cmp(&(a.1, a.0))
                .then_with(|| blanks_of(a).cmp(blanks_of(b)))
        });
        s.placed.clear();
        s.placed.resize(cells, 0);
        s.blanks_used.clear();
        s.blanks_used.resize(num_letters, 0);
        s.reals_used.clear();
        s.reals_used.resize(num_letters, 0);
        let mut first = 0;
        while first < s.order.len() {
            let head = s.members[s.order[first]];
            let (score, equity, leave_value, _) = head;
            let mut last = first + 1;
            while last < s.order.len() && {
                let m = &s.members[s.order[last]];
                (m.0, m.1) == (score, equity)
                    && (!by_tiles
                        || s.member_data[m.3..m.3 + num_letters]
                            == s.member_data[head.3..head.3 + num_letters])
            } {
                last += 1;
            }
            s.group.clear();
            for i in first..last {
                s.group.push(s.members[s.order[i]].3);
            }
            first = last;
            if !env.params.is_census && score + leave_value < env.params.threshold {
                continue;
            }
            if !hold_group(&mut s) {
                break;
            }
            if place_smallest(env, &mut s, 0, 0) {
                env.params.threshold = (env.params.callback)(
                    env.idx_left,
                    &env.params.word_strip_buffer[env.idx_left as usize..env.idx_right as usize],
                    score,
                    leave_value,
                );
                if stops_walk(env.params.threshold) {
                    break;
                }
            }
        }
        env.params.jumbled.splits = s;
    }

    // each designation's ways apart, best first: the first group that places
    // is its rack tiles' best play, and the rest of its ways are not looked at
    fn walk_best_splits<
        CallbackType: FnMut(i8, &[u8], i32, i32) -> i32,
        N: kwg::Node,
        L: kwg::Node,
    >(
        env: &mut Env<'_, CallbackType, N, L>,
        s: &mut JumbledSplits,
        cells: usize,
        num_letters: usize,
    ) {
        s.order.clear();
        if s.out_of_memory || s.order.try_reserve(s.members.len()).is_err() {
            s.out_of_memory = true;
            return;
        }
        s.placed.clear();
        s.placed.resize(cells, 0);
        s.blanks_used.clear();
        s.blanks_used.resize(num_letters, 0);
        s.reals_used.clear();
        s.reals_used.resize(num_letters, 0);
        s.starts.push(s.members.len());
        for d in 1..s.starts.len() {
            let (from, to) = (s.starts[d - 1], s.starts[d]);
            s.order.clear();
            s.order.extend(from..to);
            let members = &s.members;
            s.order.sort_unstable_by(|&a, &b| {
                let (a, b) = (&members[a], &members[b]);
                (b.1, b.0).cmp(&(a.1, a.0))
            });
            let mut first = 0;
            while first < s.order.len() {
                let head = s.members[s.order[first]];
                let (score, equity, leave_value, _) = head;
                let mut last = first + 1;
                while last < s.order.len() && {
                    let m = &s.members[s.order[last]];
                    (m.0, m.1) == (score, equity)
                } {
                    last += 1;
                }
                s.group.clear();
                for i in first..last {
                    s.group.push(s.members[s.order[i]].3);
                }
                first = last;
                if !env.params.is_census && score + leave_value < env.params.threshold {
                    continue;
                }
                if !hold_group(s) {
                    return;
                }
                if place_smallest(env, s, 0, 0) {
                    env.params.threshold = (env.params.callback)(
                        env.idx_left,
                        &env.params.word_strip_buffer
                            [env.idx_left as usize..env.idx_right as usize],
                        score,
                        leave_value,
                    );
                    if stops_walk(env.params.threshold) {
                        return;
                    }
                    break;
                }
            }
        }
    }

    fn designate_blanks<
        const BAG_TERM: bool,
        CallbackType: FnMut(i8, &[u8], i32, i32) -> i32,
        N: kwg::Node,
        L: kwg::Node,
    >(
        env: &mut Env<'_, CallbackType, N, L>,
        s: &mut JumbledSplits,
        i: usize,
        blanks_left: u8,
        fixed: i32,
    ) {
        if i == s.letters.len() {
            let jumbled = &mut env.params.jumbled;
            if jumbled.walk == JUMBLED_BEST_PER_RACK_TILES {
                let at = (jumbled.seen - 1, jumbled.designation);
                jumbled.designation += 1;
                if jumbled.stopped.is_some() || at < jumbled.part {
                    return;
                }
                if jumbled.budget == 0 {
                    jumbled.stopped = Some(at);
                    jumbled.window.end = at.0 + 1;
                    return;
                }
                jumbled.budget -= 1;
            }
            let mut leave_idx = env.params.multi_leaves.pass_leave_idx();
            for j in 0..s.letters.len() {
                let letter = s.letters[j] as usize;
                let real = s.letter_counts[j] - s.blanks[j];
                let blanks = s.blanks[j];
                env.params.rack_tally[letter] -= real;
                env.params.rack_tally[0] -= blanks;
                for _ in 0..real {
                    leave_idx =
                        leave_idx.wrapping_sub(env.params.multi_leaves.place_value(letter as u8));
                }
                for _ in 0..blanks {
                    leave_idx = leave_idx.wrapping_sub(env.params.multi_leaves.place_value(0));
                }
            }
            let leave_value = leave_value_of::<BAG_TERM, _>(
                env.params.multi_leaves,
                env.params.board_snapshot.klv,
                env.params.rack_tally,
                leave_idx,
                env.params.num_tiles_in_bag,
                env.params.play_out_bonus,
                env.alphabet,
            );
            for j in 0..s.letters.len() {
                let letter = s.letters[j] as usize;
                env.params.rack_tally[letter] += s.letter_counts[j] - s.blanks[j];
                env.params.rack_tally[0] += s.blanks[j];
            }
            if !env.params.is_census && env.params.threshold != i32::MIN {
                let bound = s.tiles_bound(fixed + leave_value);
                if bound < env.params.threshold {
                    return;
                }
            }
            s.value_left.clear();
            s.value_left.resize(s.values.len(), 0);
            for j in 0..s.letters.len() {
                s.value_left[s.real_class[j] as usize] += s.letter_counts[j] - s.blanks[j];
                s.value_left[s.blank_class[j] as usize] += s.blanks[j];
            }
            s.class_left.clone_from(&s.class_sizes);
            s.matrix.clear();
            s.matrix.resize(s.class_sizes.len() * s.values.len(), 0);
            if !s.reserve_members() {
                return;
            }
            s.starts.push(s.members.len());
            share_classes(s, 0, fixed, 0, leave_value);
            return;
        }
        let count = s.letter_counts[i];
        let real = env.params.rack_tally[s.letters[i] as usize].min(count);
        let least = count - real;
        if least > blanks_left {
            return;
        }
        let most = if env.params.is_census {
            least
        } else {
            count.min(blanks_left)
        };
        for blanks in least..=most {
            s.blanks[i] = blanks;
            designate_blanks::<BAG_TERM, _, _, _>(env, s, i + 1, blanks_left - blanks, fixed);
        }
        s.blanks[i] = 0;
    }

    fn share_classes(
        s: &mut JumbledSplits,
        cell: usize,
        score: i32,
        penalty: i32,
        leave_value: i32,
    ) {
        let num_values = s.values.len();
        if cell == s.matrix.len() {
            debug_assert!(s.members.len() < s.members.capacity());
            let offset = s.member_data.len();
            s.member_data.extend_from_slice(&s.blanks);
            s.member_data.extend_from_slice(&s.matrix);
            s.members
                .push((score, score + leave_value + penalty, leave_value, offset));
            return;
        }
        let (class, k) = (cell / num_values, cell % num_values);
        let most = s.class_left[class].min(s.value_left[k]);
        let least = if k + 1 == num_values {
            s.class_left[class]
        } else {
            0
        };
        if least > most {
            return;
        }
        let (value, vowel) = s.values[k];
        for count in least..=most {
            s.matrix[cell] = count;
            s.class_left[class] -= count;
            s.value_left[k] -= count;
            share_classes(
                s,
                cell + 1,
                score + i32::from(count) * value * s.class_weights[class],
                penalty
                    + if vowel {
                        i32::from(count) * s.class_penalties[class]
                    } else {
                        0
                    },
                leave_value,
            );
            s.class_left[class] += count;
            s.value_left[k] += count;
        }
        s.matrix[cell] = 0;
    }

    // room for a group's narrowing: each take pushes at most the group's
    // members, and place_smallest's squares and an assignable's narrowed
    // squares stand at most a take each
    fn hold_group(s: &mut JumbledSplits) -> bool {
        let takes = s.squares.len() + s.narrowed.len();
        let room = s.group.len().saturating_mul(takes);
        if s.group.try_reserve(room).is_err() {
            s.out_of_memory = true;
            return false;
        }
        true
    }

    #[inline(always)]
    fn take(s: &mut JumbledSplits, class: usize, j: usize, blank: bool, from: usize) -> bool {
        let num_letters = s.letters.len();
        let cell = class * s.values.len()
            + if blank {
                s.blank_class[j]
            } else {
                s.real_class[j]
            } as usize;
        let used = if blank {
            s.blanks_used[j]
        } else {
            s.reals_used[j]
        } + 1;
        let placed = s.placed[cell] + 1;
        let to = s.group.len();
        for i in from..to {
            let offset = s.group[i];
            let blanks = s.member_data[offset + j];
            let most = if blank {
                blanks
            } else {
                s.letter_counts[j] - blanks
            };
            if used <= most && placed <= s.member_data[offset + num_letters + cell] {
                debug_assert!(s.group.len() < s.group.capacity());
                s.group.push(offset);
            }
        }
        if s.group.len() == to {
            return false;
        }
        if blank {
            s.blanks_used[j] = used;
        } else {
            s.reals_used[j] = used;
        }
        s.placed[cell] = placed;
        true
    }

    #[inline(always)]
    fn untake(s: &mut JumbledSplits, class: usize, j: usize, blank: bool, to: usize) {
        let cell = class * s.values.len()
            + if blank {
                s.blank_class[j]
            } else {
                s.real_class[j]
            } as usize;
        if blank {
            s.blanks_used[j] -= 1;
        } else {
            s.reals_used[j] -= 1;
        }
        s.placed[cell] -= 1;
        s.group.truncate(to);
    }

    fn assignable<CallbackType: FnMut(i8, &[u8], i32, i32) -> i32, N: kwg::Node, L: kwg::Node>(
        env: &Env<'_, CallbackType, N, L>,
        s: &mut JumbledSplits,
        at: usize,
        from: usize,
    ) -> bool {
        if at == s.narrowed.len() {
            return true;
        }
        let pos = s.narrowed[at] as usize;
        let class = s.square_classes[pos] as usize;
        let bits = env.params.allowed_strip[s.squares[pos] as usize];
        let to = s.group.len();
        for blank in [false, true] {
            for j in 0..s.letters.len() {
                if bits & (1 << s.letters[j]) == 0 || !take(s, class, j, blank, from) {
                    continue;
                }
                let found = assignable(env, s, at + 1, to);
                untake(s, class, j, blank, to);
                if found {
                    return true;
                }
            }
        }
        false
    }

    fn place_smallest<
        CallbackType: FnMut(i8, &[u8], i32, i32) -> i32,
        N: kwg::Node,
        L: kwg::Node,
    >(
        env: &mut Env<'_, CallbackType, N, L>,
        s: &mut JumbledSplits,
        pos: usize,
        from: usize,
    ) -> bool {
        if pos == s.squares.len() {
            return true;
        }
        let idx = s.squares[pos] as usize;
        let class = s.square_classes[pos] as usize;
        let bits = env.params.allowed_strip[idx];
        let narrowed_after = s.narrowed.partition_point(|&p| p as usize <= pos);
        let to = s.group.len();
        for blank in [false, true] {
            for j in 0..s.letters.len() {
                let letter = s.letters[j];
                if bits & (1 << letter) == 0 || !take(s, class, j, blank, from) {
                    continue;
                }
                let meets = assignable(env, s, narrowed_after, to);
                if meets {
                    env.params.word_strip_buffer[idx] = if blank { letter | 0x80 } else { letter };
                }
                let found = meets && place_smallest(env, s, pos + 1, to);
                untake(s, class, j, blank, to);
                if found {
                    return true;
                }
            }
        }
        false
    }

    fn fill_span<
        const BAG_TERM: bool,
        CallbackType: FnMut(i8, &[u8], i32, i32) -> i32,
        N: kwg::Node,
        L: kwg::Node,
    >(
        env: &mut Env<'_, CallbackType, N, L>,
        acc: &Accumulator,
        mut idx: i8,
    ) {
        while idx < env.idx_right && env.params.board_strip[idx as usize] != 0 {
            idx += 1;
        }
        if idx >= env.idx_right {
            record::<BAG_TERM, _, _, _>(env, acc);
            return;
        }
        if env.square_bounded && env.params.threshold != i32::MIN {
            // the squares still empty, their weights highest first, and the
            // letters still to place, their scores highest first
            let mut n = 0;
            let mut perpendicular = 0i32;
            for i in idx..env.idx_right {
                if env.params.board_strip[i as usize] == 0 {
                    env.bound_weights[n] = env.square_weights[(i - env.idx_left) as usize];
                    perpendicular += env.params.perpendicular_scores_strip[i as usize];
                    n += 1;
                }
            }
            let mut m = 0;
            let mut bits = env.unplaced_bits & !1;
            while bits != 0 {
                let tile = bits.trailing_zeros() as u8;
                bits &= bits - 1;
                let real = env.unplaced[tile as usize].min(env.params.rack_tally[tile as usize]);
                for k in 0..env.unplaced[tile as usize] {
                    env.bound_scores[m] = if k < real {
                        env.alphabet.scaled_score(tile).raw()
                    } else {
                        env.alphabet.scaled_score(0).raw()
                    };
                    m += 1;
                }
            }
            debug_assert_eq!(m, n);
            let (weights, scores) = (&mut env.bound_weights[..n], &mut env.bound_scores[..m]);
            weights.sort_unstable_by(|a, b| b.cmp(a));
            scores.sort_unstable_by(|a, b| b.cmp(a));
            let bound = scores.iter().zip(&*weights).fold(
                acc.main_score * env.span_word_multiplier
                    + acc.perpendicular_cumulative_score
                    + perpendicular
                    + env.span_leave,
                |sum, (&score, &weight)| sum + score * weight,
            );
            if bound < env.params.threshold {
                return;
            }
        }
        let this_cross_bits = env.params.allowed_strip[idx as usize];
        // bit 0 is no letter; unplaced_bits drops it
        let mut usable = this_cross_bits & env.unplaced_bits;
        let new_word_multiplier = acc.word_multiplier
            * i32::from(env.params.remaining_word_multipliers_strip[idx as usize]);
        let tile_multiplier = env.params.remaining_tile_multipliers_strip[idx as usize];
        let perpendicular_word_multiplier =
            env.params.perpendicular_word_multipliers_strip[idx as usize];
        let perpendicular_score = env.params.perpendicular_scores_strip[idx as usize];
        let opt_blank_acc = (env.params.rack_tally[0] > 0).then(|| {
            let tile_value = env.alphabet.scaled_score(0).raw() * i32::from(tile_multiplier);
            Accumulator {
                main_score: acc.main_score + tile_value,
                perpendicular_cumulative_score: acc.perpendicular_cumulative_score
                    + perpendicular_score
                    + tile_value * i32::from(perpendicular_word_multiplier),
                word_multiplier: new_word_multiplier,
                leave_idx: acc
                    .leave_idx
                    .wrapping_sub(env.params.multi_leaves.place_value(0)),
            }
        });
        while usable != 0 && !stops_walk(env.params.threshold) {
            let tile = usable.trailing_zeros() as u8;
            usable &= usable - 1;
            env.unplaced[tile as usize] -= 1;
            if env.unplaced[tile as usize] == 0 {
                env.unplaced_bits &= !(1 << tile);
            }
            if env.params.rack_tally[tile as usize] > 0 {
                env.params.rack_tally[tile as usize] -= 1;
                let tile_value = env.alphabet.scaled_score(tile).raw() * i32::from(tile_multiplier);
                env.params.word_strip_buffer[idx as usize] = tile;
                fill_span::<BAG_TERM, _, _, _>(
                    env,
                    &Accumulator {
                        main_score: acc.main_score + tile_value,
                        perpendicular_cumulative_score: acc.perpendicular_cumulative_score
                            + perpendicular_score
                            + tile_value * i32::from(perpendicular_word_multiplier),
                        word_multiplier: new_word_multiplier,
                        leave_idx: acc
                            .leave_idx
                            .wrapping_sub(env.params.multi_leaves.place_value(tile)),
                    },
                    idx + 1,
                );
                env.params.rack_tally[tile as usize] += 1;
            }
            if let Some(blank_acc) = &opt_blank_acc
                && (!env.params.is_census || env.params.rack_tally[tile as usize] == 0)
            {
                env.params.rack_tally[0] -= 1;
                env.params.word_strip_buffer[idx as usize] = tile | 0x80;
                fill_span::<BAG_TERM, _, _, _>(env, blank_acc, idx + 1);
                env.params.rack_tally[0] += 1;
            }
            env.unplaced[tile as usize] += 1;
            env.unplaced_bits |= 1 << tile;
        }
    }

    let alphabet = params.board_snapshot.game_config.alphabet();
    let alphabet_len = alphabet.len();
    let anchor = params.anchor;
    let leftmost = params.leftmost;
    let rightmost = params.rightmost;
    let num_max_played = params.num_max_played;
    let pass_leave_idx = params.multi_leaves.pass_leave_idx();
    let mut forced = [0u8; MAX_ALPHABET_LEN];
    // the multisets are looked up again only when the forced letters change
    let mut forced_epoch = 0u32;
    let mut collected_epoch = u32::MAX;
    let mut lists_start = 0usize;
    let mut env = Env {
        params,
        alphabet,
        idx_left: 0,
        idx_right: 0,
        num_played: 0,
        unplaced: [0u8; MAX_ALPHABET_LEN],
        unplaced_bits: 0,
        square_bounded: false,
        span_word_multiplier: 1,
        square_weights: [0i32; JUMBLED_BOUND_SQUARES],
        span_leave: 0,
        bound_weights: [0i32; JUMBLED_BOUND_TILES],
        bound_scores: [0i32; JUMBLED_BOUND_TILES],
    };
    let opening_penalty = |i: i8| {
        let board_layout = env.params.board_snapshot.game_config.board_layout();
        match opening {
            Some(true) if board_layout.danger_stars(true)[usize::from(i.cast_unsigned())] => {
                -equity::OPENING_HOTSPOT_PENALTY
            }
            Some(false) if board_layout.danger_stars(false)[usize::from(i.cast_unsigned())] => {
                -equity::OPENING_HOTSPOT_PENALTY
            }
            _ => 0,
        }
    };

    // a span holds every tile it crosses, so its ends fix its forced letters
    let mut main_score_left = 0;
    let mut empty_left = 0u8;
    let mut free_left = false;
    let mut idx_l = anchor;
    while idx_l >= leftmost {
        let b = env.params.board_strip[idx_l as usize];
        if b != 0 {
            forced[(b & 0x7f) as usize] += 1;
            forced_epoch += 1;
            main_score_left += env.params.face_value_scores_strip[idx_l as usize];
        } else {
            let bits = env.params.allowed_strip[idx_l as usize];
            if bits == 1 {
                break;
            }
            if bits & 1 == 0 {
                free_left = true;
            }
            empty_left += 1;
            if empty_left > num_max_played {
                break;
            }
        }
        if idx_l == leftmost || env.params.board_strip[idx_l as usize - 1] == 0 {
            let mut main_score_right = 0;
            let mut empty_right = 0u8;
            let mut free_right = false;
            let mut idx_r = anchor;
            loop {
                let num_played = empty_left + empty_right;
                let is_unique = single_tile_plays || free_left || free_right;
                if num_played >= env.params.num_min_played
                    && num_played > u8::from(!is_unique)
                    && idx_r > idx_l
                    && (idx_r + 1 >= rightmost || env.params.board_strip[idx_r as usize + 1] == 0)
                {
                    if collected_epoch != forced_epoch {
                        let (start, kept) =
                            env.params.jumbled.find(&forced, num_max_played as usize);
                        if !kept {
                            collect_multisets(
                                &mut Collect {
                                    kwg: env.params.board_snapshot.kwg,
                                    alphabet_len,
                                    forced: &forced,
                                    rack_tally: env.params.rack_tally,
                                    max_size: num_max_played,
                                    out: env.params.jumbled,
                                },
                                0,
                                1,
                                env.params.rack_tally[0],
                                0,
                            );
                            if !env.params.jumbled.splits.out_of_memory {
                                env.params.jumbled.put_found(num_max_played as usize);
                            }
                        }
                        if env.params.jumbled.splits.out_of_memory {
                            return;
                        }
                        lists_start = start;
                        collected_epoch = forced_epoch;
                    }
                    env.idx_left = idx_l;
                    env.idx_right = idx_r + 1;
                    env.num_played = num_played;
                    let mut fixed = 0i32;
                    if WALK != JUMBLED_EVERY_ARRANGEMENT {
                        let mut word_multiplier = 1i32;
                        let mut perpendicular = 0i32;
                        for idx in idx_l..=idx_r {
                            if env.params.board_strip[idx as usize] == 0 {
                                word_multiplier *= i32::from(
                                    env.params.remaining_word_multipliers_strip[idx as usize],
                                );
                                perpendicular +=
                                    env.params.perpendicular_scores_strip[idx as usize];
                            }
                        }
                        fixed = (main_score_left + main_score_right) * word_multiplier
                            + perpendicular
                            + i32::from(
                                env.params
                                    .board_snapshot
                                    .game_config
                                    .num_played_bonus(num_played),
                            ) * equity::SCALE;
                        let s = &mut env.params.jumbled.splits;
                        s.squares.clear();
                        s.square_classes.clear();
                        s.class_weights.clear();
                        s.class_penalties.clear();
                        s.class_sizes.clear();
                        s.narrowed.clear();
                        for idx in idx_l..=idx_r {
                            if env.params.board_strip[idx as usize] != 0 {
                                continue;
                            }
                            let weight = i32::from(
                                env.params.remaining_tile_multipliers_strip[idx as usize],
                            ) * (word_multiplier
                                + i32::from(
                                    env.params.perpendicular_word_multipliers_strip[idx as usize],
                                ));
                            let penalty = opening_penalty(idx);
                            let class = match (0..s.class_sizes.len()).find(|&c| {
                                s.class_weights[c] == weight && s.class_penalties[c] == penalty
                            }) {
                                Some(c) => c,
                                None => {
                                    s.class_weights.push(weight);
                                    s.class_penalties.push(penalty);
                                    s.class_sizes.push(0);
                                    s.class_sizes.len() - 1
                                }
                            };
                            s.class_sizes[class] += 1;
                            s.squares.push(idx);
                            if env.params.allowed_strip[idx as usize] & 1 != 0 {
                                s.narrowed.push((s.squares.len() - 1) as u8);
                            }
                            s.square_classes.push(class as u8);
                        }
                        s.vowels_matter = s.class_penalties.iter().any(|&p| p != 0);
                        s.sorted_weights.clear();
                        s.sorted_weights.extend(
                            s.square_classes
                                .iter()
                                .map(|&c| s.class_weights[c as usize]),
                        );
                        s.sorted_weights.sort_unstable_by(|a, b| b.cmp(a));
                    }
                    // for a best move (a threshold, not the census), a
                    // multiset's bound: the span's fixed part with the best
                    // leave for its tiles, and its tiles' scores highest first
                    // on the span's square weights (what a tile's score is
                    // worth on each empty square) highest first; made once a
                    // span, when a threshold first stands
                    let boundable =
                        !env.params.is_census && num_played as usize <= JUMBLED_BOUND_TILES;
                    let mut weights = [0i32; JUMBLED_BOUND_TILES];
                    let mut base = 0i32;
                    let mut weighed = false;
                    env.square_bounded = false;
                    let list = lists_start + num_played as usize;
                    let elsewhere = WALK == JUMBLED_ONCE_PER_RACK_TILES
                        && env.params.jumbled.through.is_some_and(|through| {
                            through < idx_l
                                || through > idx_r
                                || env.params.board_strip[through as usize] != 0
                        });
                    for which in env.params.jumbled.list(list) {
                        let at = env.params.jumbled.by_size[which];
                        if WALK == JUMBLED_BEST_PER_RACK_TILES {
                            let seen = env.params.jumbled.seen;
                            env.params.jumbled.seen += 1;
                            if !env.params.jumbled.window.contains(&seen) {
                                continue;
                            }
                            env.params.jumbled.designation = 0;
                        }
                        if WALK == JUMBLED_ONCE_PER_RACK_TILES {
                            let seen = env.params.jumbled.seen;
                            env.params.jumbled.seen += 1;
                            if elsewhere || !env.params.jumbled.window.contains(&seen) {
                                continue;
                            }
                        }
                        if boundable && env.params.threshold != i32::MIN {
                            if !weighed {
                                let mut word_multiplier = 1i32;
                                let mut perpendicular = 0i32;
                                for idx in idx_l..=idx_r {
                                    if env.params.board_strip[idx as usize] == 0 {
                                        word_multiplier *= i32::from(
                                            env.params.remaining_word_multipliers_strip
                                                [idx as usize],
                                        );
                                        perpendicular +=
                                            env.params.perpendicular_scores_strip[idx as usize];
                                    }
                                }
                                let mut n = 0;
                                for idx in idx_l..=idx_r {
                                    if env.params.board_strip[idx as usize] == 0 {
                                        weights[n] = i32::from(
                                            env.params.remaining_tile_multipliers_strip
                                                [idx as usize],
                                        ) * (word_multiplier
                                            + i32::from(
                                                env.params.perpendicular_word_multipliers_strip
                                                    [idx as usize],
                                            ));
                                        n += 1;
                                    }
                                }
                                // each square's weight by its place, for
                                // fill_span's bound at each square
                                if ((idx_r + 1 - idx_l) as usize) <= JUMBLED_BOUND_SQUARES {
                                    for idx in idx_l..=idx_r {
                                        env.square_weights[(idx - idx_l) as usize] = i32::from(
                                            env.params.remaining_tile_multipliers_strip
                                                [idx as usize],
                                        )
                                            * (word_multiplier
                                                + i32::from(
                                                    env.params.perpendicular_word_multipliers_strip
                                                        [idx as usize],
                                                ));
                                    }
                                    env.span_word_multiplier = word_multiplier;
                                    env.span_leave =
                                        env.params.best_leave_values[num_played as usize];
                                    env.square_bounded = true;
                                }
                                weights[..n].sort_unstable_by(|a, b| b.cmp(a));
                                base = (main_score_left + main_score_right) * word_multiplier
                                    + perpendicular
                                    + env.params.best_leave_values[num_played as usize];
                                weighed = true;
                            }
                            // its letters (ascending) as the rack holds them, a
                            // blank for each past the rack's count
                            let mut scores = [0i32; JUMBLED_BOUND_TILES];
                            let mut run_tile = u8::MAX;
                            let mut run_len = 0u8;
                            for (i, &tile) in env.params.jumbled.multisets
                                [at..at + num_played as usize]
                                .iter()
                                .enumerate()
                            {
                                if tile == run_tile {
                                    run_len += 1;
                                } else {
                                    run_tile = tile;
                                    run_len = 1;
                                }
                                scores[i] = if run_len <= env.params.rack_tally[tile as usize] {
                                    alphabet.scaled_score(tile).raw()
                                } else {
                                    alphabet.scaled_score(0).raw()
                                };
                            }
                            let scores = &mut scores[..num_played as usize];
                            scores.sort_unstable_by(|a, b| b.cmp(a));
                            let bound = scores
                                .iter()
                                .zip(&weights)
                                .fold(base, |sum, (&score, &weight)| sum + score * weight);
                            if bound < env.params.threshold {
                                continue;
                            }
                        }
                        if WALK != JUMBLED_EVERY_ARRANGEMENT {
                            walk_splits::<BAG_TERM, WALK, _, _, _>(&mut env, at, fixed);
                            if stops_walk(env.params.threshold) {
                                return;
                            }
                            continue;
                        }
                        for offset in 0..num_played as usize {
                            let tile = env.params.jumbled.multisets[at + offset];
                            env.unplaced[tile as usize] += 1;
                            env.unplaced_bits |= 1 << tile;
                        }
                        fill_span::<BAG_TERM, _, _, _>(
                            &mut env,
                            &Accumulator {
                                main_score: main_score_left + main_score_right,
                                perpendicular_cumulative_score: 0,
                                word_multiplier: 1,
                                leave_idx: pass_leave_idx,
                            },
                            idx_l,
                        );
                        for offset in 0..num_played as usize {
                            let tile = env.params.jumbled.multisets[at + offset];
                            env.unplaced[tile as usize] -= 1;
                        }
                        env.unplaced_bits = 0;
                        if stops_walk(env.params.threshold) {
                            return;
                        }
                    }
                }
                let next = idx_r + 1;
                if next >= rightmost {
                    break;
                }
                let b = env.params.board_strip[next as usize];
                if b != 0 {
                    forced[(b & 0x7f) as usize] += 1;
                    forced_epoch += 1;
                    main_score_right += env.params.face_value_scores_strip[next as usize];
                } else {
                    let bits = env.params.allowed_strip[next as usize];
                    if bits == 1 {
                        break;
                    }
                    if bits & 1 == 0 {
                        free_right = true;
                    }
                    empty_right += 1;
                    if empty_left + empty_right > num_max_played {
                        break;
                    }
                }
                idx_r = next;
            }
            for undo in anchor + 1..=idx_r {
                let b = env.params.board_strip[undo as usize];
                if b != 0 {
                    forced[(b & 0x7f) as usize] -= 1;
                    forced_epoch += 1;
                }
            }
        }
        idx_l -= 1;
    }
}

#[inline(always)]
fn gen_place_moves_lean<
    'a,
    const LEAVES: bool,
    const BAG_TERM: bool,
    R: RunLetters,
    CallbackType: FnMut(i8, &[u8], i32, i32) -> i32,
    N: kwg::Node,
    L: kwg::Node,
>(
    params: &'a mut GenPlaceMovesParams<'a, CallbackType, N, L>,
    single_tile_plays: bool,
    left_bounds: &'a [i32],
    runs: R,
) {
    match params.board_snapshot.game_config.game_rules() {
        game_config::GameRules::Classic => {
            gen_classic_place_moves_lean::<LEAVES, BAG_TERM, _, _, _, _>(
                params,
                single_tile_plays,
                left_bounds,
                runs,
            )
        }
        game_config::GameRules::Jumbled => {
            gen_jumbled_place_moves::<BAG_TERM, _, _, _>(params, single_tile_plays, None)
        }
        game_config::GameRules::Woadwage => gen_woadwage_place_moves::<LEAVES, BAG_TERM, _, _, _>(
            params,
            single_tile_plays,
            left_bounds,
        ),
        game_config::GameRules::JumbledWoadwage => {
            gen_jumbled_woadwage_place_moves::<LEAVES, BAG_TERM, _, _, _>(
                params,
                single_tile_plays,
                None,
                left_bounds,
            )
        }
    }
}

#[inline]
fn gen_jumbled_woadwage_place_moves<
    'a,
    const LEAVES: bool,
    const BAG_TERM: bool,
    CallbackType: FnMut(i8, &[u8], i32, i32) -> i32,
    N: kwg::Node,
    L: kwg::Node,
>(
    params: &'a mut GenPlaceMovesParams<'a, CallbackType, N, L>,
    single_tile_plays: bool,
    opening: Option<bool>,
    left_bounds: &'a [i32],
) {
    struct Env<'a, CallbackType: FnMut(i8, &[u8], i32, i32) -> i32, N: kwg::Node, L: kwg::Node> {
        params: &'a mut GenPlaceMovesParams<'a, CallbackType, N, L>,
        alphabet: &'a alphabet::Alphabet,
        num_played: u8,
        idx_left: i8,
        opening: Option<bool>,
        tally: [u8; crate::census::MAX_LETTERS], // the letters of idx_left..idx, each one's count
        once_per_score: bool,
        left_bounds: &'a [i32], // by left edge, the best span bound at it, then at or before it
    }
    #[derive(Clone)]
    struct Accumulator {
        main_score: i32,
        perpendicular_cumulative_score: i32,
        word_multiplier: i32,
        leave_idx: u32,
    }

    #[inline(always)]
    fn record<
        const LEAVES: bool,
        const BAG_TERM: bool,
        CallbackType: FnMut(i8, &[u8], i32, i32) -> i32,
        N: kwg::Node,
        L: kwg::Node,
    >(
        env: &mut Env<'_, CallbackType, N, L>,
        acc: &Accumulator,
        idx_right: i8,
    ) {
        if env.num_played < env.params.num_min_played {
            return;
        }
        let score = acc.main_score * acc.word_multiplier
            + acc.perpendicular_cumulative_score
            + env
                .params
                .board_snapshot
                .game_config
                .num_played_bonus(env.num_played) as i32
                * equity::SCALE;
        let leave_value = if LEAVES {
            leave_value_of::<BAG_TERM, _>(
                env.params.multi_leaves,
                env.params.board_snapshot.klv,
                env.params.rack_tally,
                acc.leave_idx,
                env.params.num_tiles_in_bag,
                env.params.play_out_bonus,
                env.alphabet,
            )
        } else {
            0
        };
        debug_assert!(
            !LEAVES
                || env.left_bounds.is_empty()
                || score + leave_value <= env.left_bounds[env.idx_left as usize]
        );
        if score + leave_value < env.params.threshold {
            return;
        }
        let word = &env.params.word_strip_buffer[env.idx_left as usize..idx_right as usize];
        if env.once_per_score {
            let placed = &mut env.params.woadwage.placed;
            placed.clear();
            placed.extend(word.iter().copied().filter(|&tile| tile != 0));
            placed.sort_unstable();
            let penalty = env.opening.map_or(0, |down| {
                hotspot_penalty(
                    env.params.board_snapshot.game_config,
                    down,
                    env.idx_left,
                    word,
                )
            });
            let placed = placed[..].into();
            env.params.woadwage.found.push((
                env.idx_left,
                idx_right,
                placed,
                score,
                leave_value + penalty,
                word.into(),
            ));
            return;
        }
        env.params.threshold = (env.params.callback)(env.idx_left, word, score, leave_value);
    }

    fn extend<
        const LEAVES: bool,
        const BAG_TERM: bool,
        CallbackType: FnMut(i8, &[u8], i32, i32) -> i32,
        N: kwg::Node,
        L: kwg::Node,
    >(
        env: &mut Env<'_, CallbackType, N, L>,
        acc: &Accumulator,
        idx: i8,
        is_unique: bool,
        splits: Splits,
    ) {
        if stops_walk(env.params.threshold) {
            return;
        }
        let ends = idx >= env.params.rightmost || env.params.board_strip[idx as usize] == 0;
        if ends
            && idx > env.params.anchor
            && idx - env.idx_left >= 2
            && env.num_played > !is_unique as u8
            && splits.accepts(env, idx)
        {
            record::<LEAVES, BAG_TERM, _, _, _>(env, acc, idx);
        }
        if idx >= env.params.rightmost {
            return;
        }
        let b = env.params.board_strip[idx as usize];
        if b != 0 {
            env.params.woadwage.letters[idx as usize] = b & 0x7f;
            let acc = Accumulator {
                main_score: acc.main_score + env.params.face_value_scores_strip[idx as usize],
                ..*acc
            };
            let next = splits.grown(env, idx);
            extend::<LEAVES, BAG_TERM, _, _, _>(env, &acc, idx + 1, is_unique, next);
            Splits::shrunk(env, b & 0x7f);
            return;
        }
        if env.num_played >= env.params.num_max_played {
            return;
        }
        let allowed = env.params.allowed_strip[idx as usize];
        let is_unique = is_unique || allowed & 1 == 0;
        let mut candidates = allowed
            & !1
            & if env.params.rack_tally[0] > 0 {
                (u64::MAX >> (64 - env.alphabet.len() as u32)) & !1
            } else {
                env.params.rack_bits
            };
        let word_multiplier =
            acc.word_multiplier * env.params.remaining_word_multipliers_strip[idx as usize] as i32;
        let tile_multiplier = env.params.remaining_tile_multipliers_strip[idx as usize] as i32;
        let perpendicular_word_multiplier =
            env.params.perpendicular_word_multipliers_strip[idx as usize] as i32;
        let perpendicular_score = env.params.perpendicular_scores_strip[idx as usize];
        env.num_played += 1;
        while candidates != 0 {
            let tile = candidates.trailing_zeros() as u8;
            candidates &= candidates - 1;
            env.params.woadwage.letters[idx as usize] = tile;
            let next = splits.grown(env, idx);
            for (spent, written) in [(tile, tile), (0, tile | 0x80)] {
                if env.params.rack_tally[spent as usize] == 0 {
                    continue;
                }
                let tile_value = env.alphabet.scaled_score(spent).raw() * tile_multiplier;
                let acc = Accumulator {
                    main_score: acc.main_score + tile_value,
                    perpendicular_cumulative_score: acc.perpendicular_cumulative_score
                        + perpendicular_score
                        + tile_value * perpendicular_word_multiplier,
                    word_multiplier,
                    leave_idx: if LEAVES {
                        acc.leave_idx
                            .wrapping_sub(env.params.multi_leaves.place_value(spent))
                    } else {
                        0
                    },
                };
                env.params.rack_tally[spent as usize] -= 1;
                env.params.word_strip_buffer[idx as usize] = written;
                extend::<LEAVES, BAG_TERM, _, _, _>(env, &acc, idx + 1, is_unique, next);
                env.params.rack_tally[spent as usize] += 1;
            }
            Splits::shrunk(env, tile);
        }
        env.params.word_strip_buffer[idx as usize] = 0;
        env.num_played -= 1;
    }

    #[derive(Clone, Copy)]
    struct Splits {
        whole: bool,
    }

    #[inline(always)]
    fn is_jumble<CallbackType: FnMut(i8, &[u8], i32, i32) -> i32, N: kwg::Node, L: kwg::Node>(
        env: &mut Env<'_, CallbackType, N, L>,
        tally: &[u8; crate::census::MAX_LETTERS],
    ) -> bool {
        let alphabet_len = env.alphabet.len() as usize;
        let kwg = env.params.board_snapshot.kwg;
        *env.params
            .woadwage
            .jumbles
            .entry(*tally)
            .or_insert_with(|| kwg.accepts_alpha_of_len(&tally[..alphabet_len], alphabet_len as u8))
    }

    impl Splits {
        #[inline(always)]
        fn shrunk<CallbackType: FnMut(i8, &[u8], i32, i32) -> i32, N: kwg::Node, L: kwg::Node>(
            env: &mut Env<'_, CallbackType, N, L>,
            tile: u8,
        ) {
            env.tally[tile as usize] -= 1;
        }

        #[inline(always)]
        fn grown<CallbackType: FnMut(i8, &[u8], i32, i32) -> i32, N: kwg::Node, L: kwg::Node>(
            self,
            env: &mut Env<'_, CallbackType, N, L>,
            idx: i8,
        ) -> Self {
            env.params.woadwage.prefixes[(idx - env.idx_left) as usize] = self.whole;
            let tile = env.params.woadwage.letters[idx as usize];
            env.tally[tile as usize] += 1;
            let tally = env.tally;
            Self {
                whole: is_jumble(env, &tally),
            }
        }

        #[inline(always)]
        fn accepts<CallbackType: FnMut(i8, &[u8], i32, i32) -> i32, N: kwg::Node, L: kwg::Node>(
            self,
            env: &mut Env<'_, CallbackType, N, L>,
            idx: i8,
        ) -> bool {
            if self.whole {
                return true;
            }
            for k in 1..idx - env.idx_left {
                if !env.params.woadwage.prefixes[k as usize] {
                    continue;
                }
                let mut rest = [0u8; crate::census::MAX_LETTERS];
                for &tile in &env.params.woadwage.letters[(env.idx_left + k) as usize..idx as usize]
                {
                    rest[tile as usize] += 1;
                }
                if is_jumble(env, &rest) {
                    return true;
                }
            }
            false
        }
    }

    let alphabet = params.board_snapshot.game_config.alphabet();
    let anchor = params.anchor;
    let leftmost = params.leftmost;
    let num_max_played = params.num_max_played;
    let once_per_score = params.jumbled.walk == JUMBLED_ONCE_PER_SCORE;
    let strip_len = params.board_strip.len();
    let scratch = &mut *params.woadwage;
    scratch.letters.clear();
    scratch.letters.resize(strip_len, 0);
    scratch.prefixes.clear();
    scratch.prefixes.resize(strip_len, false);
    scratch.jumbles.clear();
    scratch.found.clear();
    let start = Accumulator {
        main_score: 0,
        perpendicular_cumulative_score: 0,
        word_multiplier: 1,
        leave_idx: params.multi_leaves.pass_leave_idx(),
    };
    let mut env = Env {
        params,
        alphabet,
        num_played: 0,
        idx_left: 0,
        opening,
        tally: [0u8; crate::census::MAX_LETTERS],
        once_per_score,
        left_bounds,
    };
    let mut empties = 0u8;
    let mut idx_l = anchor;
    while idx_l >= leftmost {
        let b = env.params.board_strip[idx_l as usize];
        if b == 0 {
            if env.params.allowed_strip[idx_l as usize] == 1 {
                break;
            }
            empties += 1;
            if empties > num_max_played {
                break;
            }
        }
        let bounds = env.left_bounds;
        if !bounds.is_empty() && bounds[bounds.len() / 2 + idx_l as usize] < env.params.threshold {
            break;
        }
        if (idx_l == leftmost || env.params.board_strip[idx_l as usize - 1] == 0)
            && (bounds.is_empty() || bounds[idx_l as usize] >= env.params.threshold)
        {
            env.idx_left = idx_l;
            extend::<LEAVES, BAG_TERM, _, _, _>(
                &mut env,
                &start,
                idx_l,
                single_tile_plays,
                Splits { whole: false },
            );
            if stops_walk(env.params.threshold) {
                return;
            }
        }
        idx_l -= 1;
    }
    if !once_per_score {
        return;
    }
    let mut found = std::mem::take(&mut env.params.woadwage.found);
    found.sort_unstable();
    found.dedup_by(|b, a| (a.0, a.1, &a.2, a.3, a.4) == (b.0, b.1, &b.2, b.3, b.4));
    for (idx_left, _, _, score, keyed_leave, word) in found.drain(..) {
        let penalty = env.opening.map_or(0, |down| {
            hotspot_penalty(env.params.board_snapshot.game_config, down, idx_left, &word)
        });
        let leave_value = keyed_leave - penalty;
        if score + leave_value < env.params.threshold {
            continue;
        }
        env.params.threshold = (env.params.callback)(idx_left, &word, score, leave_value);
        if stops_walk(env.params.threshold) {
            break;
        }
    }
    env.params.woadwage.found = found;
}

#[inline]
fn gen_woadwage_place_moves<
    'a,
    const LEAVES: bool,
    const BAG_TERM: bool,
    CallbackType: FnMut(i8, &[u8], i32, i32) -> i32,
    N: kwg::Node,
    L: kwg::Node,
>(
    params: &'a mut GenPlaceMovesParams<'a, CallbackType, N, L>,
    single_tile_plays: bool,
    left_bounds: &'a [i32],
) {
    struct Env<'a, CallbackType: FnMut(i8, &[u8], i32, i32) -> i32, N: kwg::Node, L: kwg::Node> {
        params: &'a mut GenPlaceMovesParams<'a, CallbackType, N, L>,
        alphabet: &'a alphabet::Alphabet,
        num_played: u8,
        idx_left: i8,
        left_bounds: &'a [i32], // by left edge, the best span bound at it, then at or before it
    }
    #[derive(Clone)]
    struct Accumulator {
        main_score: i32,
        perpendicular_cumulative_score: i32,
        word_multiplier: i32,
        leave_idx: u32,
    }

    #[inline(always)]
    fn record<
        const LEAVES: bool,
        const BAG_TERM: bool,
        CallbackType: FnMut(i8, &[u8], i32, i32) -> i32,
        N: kwg::Node,
        L: kwg::Node,
    >(
        env: &mut Env<'_, CallbackType, N, L>,
        acc: &Accumulator,
        idx_right: i8,
    ) {
        if env.num_played < env.params.num_min_played {
            return;
        }
        let score = acc.main_score * acc.word_multiplier
            + acc.perpendicular_cumulative_score
            + env
                .params
                .board_snapshot
                .game_config
                .num_played_bonus(env.num_played) as i32
                * equity::SCALE;
        let leave_value = if LEAVES {
            leave_value_of::<BAG_TERM, _>(
                env.params.multi_leaves,
                env.params.board_snapshot.klv,
                env.params.rack_tally,
                acc.leave_idx,
                env.params.num_tiles_in_bag,
                env.params.play_out_bonus,
                env.alphabet,
            )
        } else {
            0
        };
        debug_assert!(
            !LEAVES
                || env.left_bounds.is_empty()
                || score + leave_value <= env.left_bounds[env.idx_left as usize]
        );
        if score + leave_value < env.params.threshold {
            return;
        }
        env.params.threshold = (env.params.callback)(
            env.idx_left,
            &env.params.word_strip_buffer[env.idx_left as usize..idx_right as usize],
            score,
            leave_value,
        );
    }

    fn extend<
        const LEAVES: bool,
        const BAG_TERM: bool,
        CallbackType: FnMut(i8, &[u8], i32, i32) -> i32,
        N: kwg::Node,
        L: kwg::Node,
    >(
        env: &mut Env<'_, CallbackType, N, L>,
        acc: &Accumulator,
        state: &woadwage::StackedStep,
        idx: i8,
        is_unique: bool,
    ) {
        if stops_walk(env.params.threshold) {
            return;
        }
        let kwg = env.params.board_snapshot.kwg;
        let ends = idx >= env.params.rightmost || env.params.board_strip[idx as usize] == 0;
        if ends
            && idx > env.params.anchor
            && idx - env.idx_left >= 2
            && env.num_played > !is_unique as u8
            && state.accepts(kwg, &env.params.woadwage.stack)
        {
            record::<LEAVES, BAG_TERM, _, _, _>(env, acc, idx);
        }
        if idx >= env.params.rightmost {
            return;
        }
        let b = env.params.board_strip[idx as usize];
        if b != 0 {
            let mark = env.params.woadwage.stack.len();
            if let Some(next) = state.step(kwg, &mut env.params.woadwage.stack, b & 0x7f) {
                let acc = Accumulator {
                    main_score: acc.main_score + env.params.face_value_scores_strip[idx as usize],
                    ..*acc
                };
                extend::<LEAVES, BAG_TERM, _, _, _>(env, &acc, &next, idx + 1, is_unique);
                env.params.woadwage.stack.truncate(mark);
            }
            return;
        }
        if env.num_played >= env.params.num_max_played {
            return;
        }
        let allowed = env.params.allowed_strip[idx as usize];
        let is_unique = is_unique || allowed & 1 == 0;
        let mut candidates = allowed
            & !1
            & if env.params.rack_tally[0] > 0 {
                (u64::MAX >> (64 - env.alphabet.len() as u32)) & !1
            } else {
                env.params.rack_bits
            };
        let word_multiplier =
            acc.word_multiplier * env.params.remaining_word_multipliers_strip[idx as usize] as i32;
        let tile_multiplier = env.params.remaining_tile_multipliers_strip[idx as usize] as i32;
        let perpendicular_word_multiplier =
            env.params.perpendicular_word_multipliers_strip[idx as usize] as i32;
        let perpendicular_score = env.params.perpendicular_scores_strip[idx as usize];
        env.num_played += 1;
        while candidates != 0 {
            let tile = candidates.trailing_zeros() as u8;
            candidates &= candidates - 1;
            let mark = env.params.woadwage.stack.len();
            let Some(next) = state.step(kwg, &mut env.params.woadwage.stack, tile) else {
                continue;
            };
            for (spent, written) in [(tile, tile), (0, tile | 0x80)] {
                if env.params.rack_tally[spent as usize] == 0 {
                    continue;
                }
                let tile_value = env.alphabet.scaled_score(spent).raw() * tile_multiplier;
                let acc = Accumulator {
                    main_score: acc.main_score + tile_value,
                    perpendicular_cumulative_score: acc.perpendicular_cumulative_score
                        + perpendicular_score
                        + tile_value * perpendicular_word_multiplier,
                    word_multiplier,
                    leave_idx: if LEAVES {
                        acc.leave_idx
                            .wrapping_sub(env.params.multi_leaves.place_value(spent))
                    } else {
                        0
                    },
                };
                env.params.rack_tally[spent as usize] -= 1;
                env.params.word_strip_buffer[idx as usize] = written;
                extend::<LEAVES, BAG_TERM, _, _, _>(env, &acc, &next, idx + 1, is_unique);
                env.params.rack_tally[spent as usize] += 1;
            }
            env.params.woadwage.stack.truncate(mark);
        }
        env.num_played -= 1;
    }

    let alphabet = params.board_snapshot.game_config.alphabet();
    let anchor = params.anchor;
    let leftmost = params.leftmost;
    let num_max_played = params.num_max_played;
    let start = Accumulator {
        main_score: 0,
        perpendicular_cumulative_score: 0,
        word_multiplier: 1,
        leave_idx: params.multi_leaves.pass_leave_idx(),
    };
    let mut env = Env {
        params,
        alphabet,
        num_played: 0,
        idx_left: 0,
        left_bounds,
    };
    let mut empties = 0u8;
    let mut idx_l = anchor;
    while idx_l >= leftmost {
        let b = env.params.board_strip[idx_l as usize];
        if b == 0 {
            if env.params.allowed_strip[idx_l as usize] == 1 {
                break;
            }
            empties += 1;
            if empties > num_max_played {
                break;
            }
        }
        let bounds = env.left_bounds;
        if !bounds.is_empty() && bounds[bounds.len() / 2 + idx_l as usize] < env.params.threshold {
            break;
        }
        if (idx_l == leftmost || env.params.board_strip[idx_l as usize - 1] == 0)
            && (bounds.is_empty() || bounds[idx_l as usize] >= env.params.threshold)
        {
            env.idx_left = idx_l;
            env.params.woadwage.stack.clear();
            extend::<LEAVES, BAG_TERM, _, _, _>(
                &mut env,
                &start,
                &woadwage::StackedStep::new(),
                idx_l,
                single_tile_plays,
            );
            if stops_walk(env.params.threshold) {
                return;
            }
        }
        idx_l -= 1;
    }
}

struct GenPlaceMovesAtParams<
    'a,
    R: RunLetters,
    FoundPlaceMove: FnMut(bool, i8, i8, &[u8], equity::Equity, equity::Equity) -> equity::Equity,
    N: kwg::Node,
    L: kwg::Node,
    G: LaneBits,
    const BAG_TERM: bool,
    S: BoardStore,
> {
    runs: R,
    board_snapshot: &'a BoardSnapshot<'a, N, L>,
    working_buffer: &'a mut WorkingBuffer<G, BAG_TERM, S>,
    multi_leaves: &'a klv::MultiLeaves,
    placement: &'a PossiblePlacement,
    num_max_played: u8,
    threshold: i32,
    found_place_move: FoundPlaceMove,
}

#[inline(always)]
fn gen_place_moves_at_lean<
    'a,
    const D: usize,
    R: RunLetters,
    FoundPlaceMove: FnMut(bool, i8, i8, &[u8], equity::Equity, equity::Equity) -> equity::Equity,
    N: kwg::Node,
    L: kwg::Node,
    G: LaneBits,
    const BAG_TERM: bool,
    S: BoardStore,
>(
    p: GenPlaceMovesAtParams<'a, R, FoundPlaceMove, N, L, G, BAG_TERM, S>,
) {
    let GenPlaceMovesAtParams {
        runs,
        board_snapshot,
        working_buffer,
        multi_leaves,
        placement,
        num_max_played,
        threshold,
        mut found_place_move,
    } = p;
    let fixed;
    let dim = if D != 0 {
        // init, compiled for the same side, asserts the board's
        fixed = square_dim::<D>();
        &fixed
    } else {
        board_snapshot.game_config.board_layout().dim()
    };
    let strip_range_start;

    let strip_range_end = if placement.down {
        strip_range_start = (placement.lane as isize * dim.rows as isize) as usize;
        strip_range_start + dim.rows as usize
    } else {
        strip_range_start = (placement.lane as isize * dim.cols as isize) as usize;
        strip_range_start + dim.cols as usize
    };
    let bounds_at = working_buffer.bounds_at;
    let stride = dim.rows.max(dim.cols) as usize + 1;
    let left_bounds = &working_buffer.anchor_left_bounds[bounds_at..bounds_at + 2 * stride];
    gen_place_moves_lean::<true, BAG_TERM, _, _, _, _>(
        &mut GenPlaceMovesParams {
            board_snapshot,
            board_strip: if placement.down {
                &working_buffer.transposed_board_tiles[strip_range_start..strip_range_end]
            } else {
                &board_snapshot.board_tiles[strip_range_start..strip_range_end]
            },
            allowed_strip: if placement.down {
                &working_buffer.allowed_for_down_plays[strip_range_start..strip_range_end]
            } else {
                &working_buffer.allowed_for_across_plays[strip_range_start..strip_range_end]
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
            num_min_played: 0,
            anchor: placement.anchor,
            leftmost: placement.leftmost,
            rightmost: placement.rightmost,
            callback: |idx: i8, word: &[u8], score: i32, leave_value: i32| {
                found_place_move(
                    placement.down,
                    placement.lane,
                    idx,
                    word,
                    equity::Equity::new(score),
                    equity::Equity::new(leave_value),
                )
                .raw()
            },
            multi_leaves,
            num_tiles_in_bag: working_buffer.num_tiles_in_bag,
            play_out_bonus: working_buffer.play_out_bonus,
            jumbled: &mut working_buffer.jumbled,
            woadwage: &mut working_buffer.woadwage,
            is_census: working_buffer.is_census,
            threshold,
            best_leave_values: &working_buffer.best_leave_values,
        },
        !placement.down,
        left_bounds,
        runs,
    );
}

#[inline(always)]
fn gen_place_moves_at<
    'a,
    const D: usize,
    W: LaneBits,
    T: TablePlaces,
    FoundPlaceMove: FnMut(bool, i8, i8, &[u8], equity::Equity, equity::Equity) -> equity::Equity,
    N: kwg::Node,
    L: kwg::Node,
    G: LaneBits,
    const BAG_TERM: bool,
    S: BoardStore,
>(
    p: GenPlaceMovesAtParams<'a, (), FoundPlaceMove, N, L, G, BAG_TERM, S>,
) {
    let GenPlaceMovesAtParams {
        runs: (),
        board_snapshot,
        working_buffer,
        multi_leaves,
        placement,
        num_max_played,
        threshold,
        mut found_place_move,
    } = p;
    let fixed;
    let dim = if D != 0 {
        // its one caller, the iterator, asserts the board's side once a turn
        let board = board_snapshot.game_config.board_layout().dim();
        debug_assert!(board.rows as usize == D && board.cols as usize == D);
        fixed = square_dim::<D>();
        &fixed
    } else {
        board_snapshot.game_config.board_layout().dim()
    };
    let strip_range_start;

    let strip_range_end = if placement.down {
        strip_range_start = (placement.lane as isize * dim.rows as isize) as usize;
        strip_range_start + dim.rows as usize
    } else {
        strip_range_start = (placement.lane as isize * dim.cols as isize) as usize;
        strip_range_start + dim.cols as usize
    };
    let strip_len = strip_range_end - strip_range_start;
    // the table walk plays only classic games (gen_moves_tables hands the rest
    // back), holding its anagrams
    match board_snapshot.game_config.game_rules() {
        game_config::GameRules::Classic => {}
        game_config::GameRules::Jumbled
        | game_config::GameRules::JumbledWoadwage
        | game_config::GameRules::Woadwage => return,
    }
    let Some(held) = board_snapshot.anagrams else {
        return;
    };
    let (prefix_at, lane_at) = working_buffer.fill_span_lane::<D, W, _>(
        board_snapshot.board_tiles,
        dim,
        T::of(held),
        span_source::<T>(held),
        placement.down,
        placement.lane,
    );
    // the kept span's multipliers, best first, for own_bound: none for a
    // placement no kept span made, or a rack whose tiles' places pass a u32's
    let span_multipliers: &[Taken] = if placement.kept_at == NO_KEPT
        || working_buffer.descending_scores.len() > u32::BITS as usize
    {
        &[]
    } else {
        let lane_at = if placement.down {
            dim.rows as usize + placement.lane as usize
        } else {
            placement.lane as usize
        };
        let at = placement.kept_at as usize;
        &working_buffer.rcs_lanes[lane_at].multipliers[at..at + placement.num_played as usize]
    };
    // the lane's face values and cross scores, which only the debug walk reads
    // (add_square)
    #[cfg(debug_assertions)]
    let (face_values, perpendicular_scores): (&[i32], &[i32]) = if placement.down {
        (
            &working_buffer.face_value_scores_for_down_plays[strip_range_start..strip_range_end],
            &working_buffer.perpendicular_scores_for_down_plays[strip_range_start..strip_range_end],
        )
    } else {
        (
            &working_buffer.face_value_scores_for_across_plays[strip_range_start..strip_range_end],
            &working_buffer.perpendicular_scores_for_across_plays
                [strip_range_start..strip_range_end],
        )
    };
    gen_classic_place_moves::<W, T, _, _, _>(
        &mut TableWalkParams {
            board_snapshot,
            board_strip: if placement.down {
                &working_buffer.transposed_board_tiles[strip_range_start..strip_range_end]
            } else {
                &board_snapshot.board_tiles[strip_range_start..strip_range_end]
            },
            allowed_strip: if placement.down {
                &working_buffer.allowed_for_down_plays[strip_range_start..strip_range_end]
            } else {
                &working_buffer.allowed_for_across_plays[strip_range_start..strip_range_end]
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
            perpendicular_word_multipliers_strip: if placement.down {
                &working_buffer.perpendicular_word_multipliers_for_down_plays
                    [strip_range_start..strip_range_end]
            } else {
                &working_buffer.perpendicular_word_multipliers_for_across_plays
                    [strip_range_start..strip_range_end]
            },
            #[cfg(debug_assertions)]
            face_value_scores_strip: face_values,
            #[cfg(debug_assertions)]
            perpendicular_scores_strip: perpendicular_scores,
            has_blank: working_buffer.rack_tally[0] != 0,
            word_strip_buffer: if placement.down {
                &mut working_buffer.word_buffer_for_down_plays[strip_range_start..strip_range_end]
            } else {
                &mut working_buffer.word_buffer_for_across_plays[strip_range_start..strip_range_end]
            },
            num_max_played,
            leftmost: placement.leftmost,
            rightmost: placement.rightmost,
            span_num_played: placement.num_played,
            callback: |idx: i8, word: &[u8], score: i32, leave_value: i32| {
                found_place_move(
                    placement.down,
                    placement.lane,
                    idx,
                    word,
                    equity::Equity::new(score),
                    equity::Equity::new(leave_value),
                )
                .raw()
            },
            leave_values: multi_leaves.leave_values(),
            score_bound: placement.best_possible_equity
                - working_buffer.best_leave_values[placement.num_played as usize]
                + i32::from(
                    board_snapshot
                        .game_config
                        .num_played_bonus(placement.num_played),
                ) * equity::SCALE,
            threshold,
            subracks: &working_buffer.subracks,
            subracks_by_played: &working_buffer.subracks_by_played,
            descending_scores: &working_buffer.descending_scores,
            span_multipliers,
            span_memo: &mut working_buffer.span_memo,
            span_prefix: &working_buffer.span_prefix[prefix_at..prefix_at + strip_len + 1],
            span_empty: working_buffer.span_lanes[lane_at].empty,
            span_multiplied: working_buffer.span_lanes[lane_at].multiplied,
        },
        held,
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
        score: equity::Equity,
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
pub(crate) fn cmp_play(a: &Play, b: &Play) -> std::cmp::Ordering {
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
    column: Option<display::ColumnStr>,
}

impl<N: kwg::Node, L: kwg::Node> std::fmt::Display for WriteablePlay<'_, N, L> {
    #[inline]
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if f.width().is_some() {
            return display::pad_with(f, |s| self.write_unpadded(s));
        }
        self.write_unpadded(f)
    }
}

impl<N: kwg::Node, L: kwg::Node> WriteablePlay<'_, N, L> {
    #[inline]
    fn write_unpadded(&self, f: &mut impl std::fmt::Write) -> std::fmt::Result {
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
                let column = self.column.as_ref().ok_or(std::fmt::Error)?;
                if *down {
                    write!(f, "{column}{} ", i16::from(*idx) + 1)?;
                } else {
                    write!(f, "{}{column} ", i16::from(*lane) + 1)?;
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
                                .of_board(self.board_snapshot.board_tiles[strider.at_in_lane(i)])
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
                write!(f, " {}", score.descaled())?;
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
    ) -> error::Returns<WriteablePlay<'a, N, L>> {
        let column = match self {
            Play::Place {
                down,
                lane,
                idx,
                word,
                ..
            } => {
                board_snapshot.game_config.board_layout().dim().check_span(
                    *down,
                    *lane,
                    *idx,
                    word.len(),
                )?;
                Some(display::column(if *down { *lane } else { *idx })?)
            }
            Play::Exchange { .. } => None,
        };
        Ok(WriteablePlay {
            board_snapshot,
            play: self,
            column,
        })
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

// a census of a board: for each multiset of tiles a play places, keyed as the
// table walk keys a subrack (by nibbles or by the tables' layout), the best
// score of such a play in each lane (rows first, then columns; NO_PLAY where it
// has none). Each multiset is given a slot once, as it is first found: the
// slots are found again by their keys in an open table, each slot's best scores
// lying together, a lane each.
pub struct LaneCensus {
    nibbles: bool,
    lanes: usize,
    shift: u32,      // 64 less the bits of the slots' count
    keys: Vec<u128>, // a slot's, or 0 (a play places tiles, so no key is 0)
    best: Vec<i32>,  // a slot's lanes, lanes a slot
    used: usize,
    grown_from: (Vec<u128>, Vec<i32>), // a growth's old slots, a play's blank deltas
    deltas: Vec<(u8, i32)>,
}

// a lane's best score in a census where a multiset has no play there, low
// enough that a leave added to it stays below any play's
const NO_PLAY: i32 = i32::MIN / 2;

// the ways to take from tally the tiles placed at at or past it by word (a
// play's in its letters). Each tile is taken as itself or as a blank. Each way
// is found with score less the deltas of its blanks: play_scorer's
// score_and_blank_deltas gives one for each tile placed. k is the first such
// tile's place among them.
fn designate(
    tally: &mut [u8],
    deltas: &[(u8, i32)],
    word: &mut [u8],
    at: usize,
    k: usize,
    score: i32,
    found: &mut dyn FnMut(&[u8], i32),
) {
    let Some(skip) = word[at..].iter().position(|&tile| tile != 0) else {
        found(word, score);
        return;
    };
    let at = at + skip;
    let letter = word[at];
    if tally[letter as usize] > 0 {
        tally[letter as usize] -= 1;
        designate(tally, deltas, word, at + 1, k + 1, score, found);
        tally[letter as usize] += 1;
    }
    if tally[0] > 0 {
        tally[0] -= 1;
        word[at] = letter | 0x80;
        designate(
            tally,
            deltas,
            word,
            at + 1,
            k + 1,
            score - deltas[k].1,
            found,
        );
        word[at] = letter;
        tally[0] += 1;
    }
}

impl LaneCensus {
    #[inline(always)]
    fn new(nibbles: bool, lanes: usize) -> Self {
        const SLOTS: usize = 1 << 10;
        Self {
            nibbles,
            lanes,
            shift: 64 - SLOTS.trailing_zeros(),
            keys: vec![0; SLOTS],
            best: vec![NO_PLAY; SLOTS * lanes],
            used: 0,
            grown_from: (Vec::new(), Vec::new()),
            deltas: Vec::new(),
        }
    }

    // emptied, for a census of these keys and lanes, its slots' memory kept
    // (as many slots as it had: a slot is found by its key, wherever it lies)
    #[inline]
    fn reset(&mut self, nibbles: bool, lanes: usize) {
        let slots = self.keys.len();
        self.nibbles = nibbles;
        self.lanes = lanes;
        self.shift = 64 - slots.trailing_zeros();
        self.keys.fill(0);
        self.best.clear();
        self.best.resize(slots * lanes, NO_PLAY);
        self.used = 0;
    }

    // the slot holding key, or the empty one it would go in
    #[inline(always)]
    fn slot_of(&self, key: u128) -> usize {
        let mask = self.keys.len() - 1;
        let mut at = (alphagram::mixed(key) >> self.shift) as usize;
        while self.keys[at] != key && self.keys[at] != 0 {
            at = (at + 1) & mask;
        }
        at
    }

    // the best scores, a lane each, of the multiset of key, if it has a play
    #[inline(always)]
    fn lanes_of(&self, key: u128) -> Option<&[i32]> {
        let at = self.slot_of(key);
        (self.keys[at] == key).then(|| &self.best[at * self.lanes..(at + 1) * self.lanes])
    }

    #[inline(always)]
    fn add(&mut self, key: u128, lane_id: u8, score: i32) {
        if (self.used + 1) * 2 > self.keys.len() {
            self.grow();
        }
        let at = self.slot_of(key);
        if self.keys[at] == 0 {
            self.keys[at] = key;
            self.used += 1;
        }
        let best = &mut self.best[at * self.lanes + lane_id as usize];
        *best = (*best).max(score);
    }

    // twice the slots, each multiset moved to its slot there
    #[inline(never)]
    fn grow(&mut self) {
        let slots = self.keys.len() * 2;
        let (mut keys, mut best) = std::mem::take(&mut self.grown_from);
        std::mem::swap(&mut keys, &mut self.keys);
        std::mem::swap(&mut best, &mut self.best);
        self.keys.clear();
        self.keys.resize(slots, 0);
        self.best.clear();
        self.best.resize(slots * self.lanes, NO_PLAY);
        self.shift -= 1;
        for (from, &key) in keys.iter().enumerate() {
            if key != 0 {
                let at = self.slot_of(key);
                self.keys[at] = key;
                self.best[at * self.lanes..(at + 1) * self.lanes]
                    .copy_from_slice(&best[from * self.lanes..(from + 1) * self.lanes]);
            }
        }
        self.grown_from = (keys, best);
    }
}

// a census of the board a generation is steered by, and the lanes changed since
// it was taken: those are generated as ever; of the others, only those whose
// best equity for the rack (the census's best score of a subrack there plus its
// leave) is the best of theirs, for a rack without a blank while tiles remain
// in the bag
pub struct Steer<'a> {
    pub census: &'a LaneCensus,
    pub touched_rows: u128,
    pub touched_cols: u128,
}

// KurniaMoveGenerator can only be reused for the same game_config and kwg.
// (Refer to note at WorkingBuffer.)
// This is not enforced.
// Its parameters, new's by default: its sets of rows and of columns (u128, for
// a board of up to 128 squares a side; the wasm's u32); whether its lean walk
// may go straight to a tile's node by the run letters a lean arm's table keeps
// (true; the wasm's false, so no walk that jumps is compiled for it); and
// whether it keeps the lean arm's copy for any side beside those for 15 by 15
// and 21 by 21 (true; the wasm's false, so it takes those two boards alone,
// each by its own copy, the raw walk's too). BAG_TERM: whether it adds the bag
// term it is made with (with_bag_term); false, the default, has none of it.
pub struct KurniaMoveGenerator<
    G: LaneBits = u128,
    const JUMPS: bool = true,
    const ANY_SIDE: bool = true,
    const BAG_TERM: bool = false,
    S: BoardStore = BoxStore,
> {
    working_buffer: WorkingBuffer<G, BAG_TERM, S>,
    pub plays: Vec<ValuedMove>,
    empty_klvs: klv::EmptyKlvs, // a census's, made at its first use
}

impl<G: LaneBits, const JUMPS: bool, const ANY_SIDE: bool, const BAG_TERM: bool, S: BoardStore>
    Clone for KurniaMoveGenerator<G, JUMPS, ANY_SIDE, BAG_TERM, S>
{
    #[inline(always)]
    fn clone(&self) -> Self {
        Self {
            working_buffer: self.working_buffer.clone(),
            plays: self.plays.clone(),
            empty_klvs: klv::EmptyKlvs::default(),
        }
    }

    #[inline(always)]
    fn clone_from(&mut self, source: &Self) {
        self.working_buffer.clone_from(&source.working_buffer);
        self.plays.clone_from(&source.plays);
    }
}

impl<G: LaneBits, const JUMPS: bool, const ANY_SIDE: bool>
    KurniaMoveGenerator<G, JUMPS, ANY_SIDE, true>
{
    // each generation adds the bag term to a tile placement's equity
    #[inline(always)]
    pub(crate) fn with_bag_term_in(
        game_config: &game_config::GameConfig,
        bag_term: BagTerm,
        dense_most: u32,
    ) -> Self {
        let mut move_generator = Self::new_in_dense(game_config, dense_most);
        move_generator.working_buffer.bag_term = Some(bag_term);
        move_generator
    }
}

impl KurniaMoveGenerator {
    #[inline(always)]
    pub fn with_bag_term(
        game_config: &game_config::GameConfig,
        bag_term: BagTerm,
    ) -> KurniaMoveGenerator<u128, true, true, true> {
        KurniaMoveGenerator::<u128, true, true, true>::with_bag_term_in(
            game_config,
            bag_term,
            klv::DENSE_LEAVES_MOST,
        )
    }

    #[inline(always)]
    pub fn new(game_config: &game_config::GameConfig) -> Self {
        Self::new_in(game_config)
    }
}

pub type InlineGenerator<const N: usize> =
    KurniaMoveGenerator<u128, true, true, false, InlineStore<N>>;

// what a generator holds of the board it last generated on, each part brought
// up to date lane by lane from the board before: kept to put back when the
// generator comes back to that board (each of a sim's rollouts starts from the
// same one), so its next generation brings up only the lanes since changed
#[derive(Default)]
pub struct BoardState {
    board_tiles: Box<[u8]>,
    transposed_board_tiles: Box<[u8]>,
    counts: (u16, i32, u16, i32), // tiles and face values, now and as kept
    allowed: [Box<[u64]>; 2],     // across plays', then down plays'
    cross_scores: [Box<[i32]>; 2],
    cached_cross_sets: [Box<[CachedCrossSet]>; 2],
    cross_set_buffers: [Box<[CrossSetComputation]>; 2],
    word_multipliers: [Box<[i8]>; 2],
    tile_multipliers: [Box<[i8]>; 2],
    face_values: [Box<[i32]>; 2],
    perpendicular_word_multipliers: [Box<[i8]>; 2],
    perpendicular_scores: [Box<[i32]>; 2],
    rcs_lanes: Vec<RcsLane>, // the walk's lanes, or none if changes were unread
}

impl BoardState {
    // whether it is a generator's state on these board tiles
    #[inline(always)]
    pub fn is_of(&self, board_tiles: &[u8]) -> bool {
        *self.board_tiles == *board_tiles
    }
}

#[inline(always)]
fn copy_board_part<T: Clone, B: BoardBuf<T>>(to: &mut B, from: &[T]) {
    to.copy_in(from);
}

impl<G: LaneBits, const JUMPS: bool, const ANY_SIDE: bool, const BAG_TERM: bool, S: BoardStore>
    KurniaMoveGenerator<G, JUMPS, ANY_SIDE, BAG_TERM, S>
{
    // a generator whose sets of rows and of columns are G; a board with more
    // rows or columns than G holds is refused
    #[inline(always)]
    pub fn try_new(game_config: &game_config::GameConfig) -> error::Returns<Self> {
        Self::check_board(game_config)?;
        Ok(Self::new_in(game_config))
    }

    #[inline(always)]
    fn check_board(game_config: &game_config::GameConfig) -> error::Returns<()> {
        let dim = game_config.board_layout().dim();
        if dim.rows as u32 > G::BITS || dim.cols as u32 > G::BITS {
            return_error!(format!(
                "a board of {} by {} is past this generator's {} lanes",
                dim.rows,
                dim.cols,
                G::BITS
            ));
        }
        if S::SQUARES != 0 && dim.rows as usize * dim.cols as usize != S::SQUARES {
            return_error!(format!(
                "a board of {} by {} is not this generator's {} squares",
                dim.rows,
                dim.cols,
                S::SQUARES
            ));
        }
        if !ANY_SIDE && !(dim.rows == dim.cols && (dim.rows == 15 || dim.rows == 21)) {
            return_error!(format!(
                "a board of {} by {} has no copy of the lean arm in this generator",
                dim.rows, dim.cols
            ));
        }
        Ok(())
    }

    // try_new's generator, for a game whose board a generator in G has taken
    #[inline(always)]
    pub(crate) fn new_in(game_config: &game_config::GameConfig) -> Self {
        Self {
            working_buffer: WorkingBuffer::new(game_config, endgame::Room::default()),
            plays: Vec::new(),
            empty_klvs: klv::EmptyKlvs::default(),
        }
    }

    // the board's rows or columns, whichever are more, as made at new
    #[inline(always)]
    pub(crate) fn longest_lane(&self) -> usize {
        self.working_buffer.longest_lane
    }

    #[inline]
    pub(crate) fn new_within(
        game_config: &game_config::GameConfig,
        memory: Option<endgame::MemoryLimit>,
    ) -> Option<Self> {
        let working_buffer = WorkingBuffer::new(game_config, endgame::Room::of(memory));
        (!working_buffer.room.refused()).then(|| Self {
            working_buffer,
            plays: Vec::new(),
            empty_klvs: klv::EmptyKlvs::default(),
        })
    }

    #[inline]
    pub fn try_new_within(
        game_config: &game_config::GameConfig,
        memory: Option<endgame::MemoryLimit>,
    ) -> error::Returns<Option<Self>> {
        Self::check_board(game_config)?;
        Ok(Self::new_within(game_config, memory))
    }

    #[inline(always)]
    pub fn begin_within(&mut self, memory: Option<endgame::MemoryLimit>) {
        self.working_buffer.room = endgame::Room::of(memory);
    }

    #[inline(always)]
    pub fn out_of_memory(&self) -> bool {
        self.working_buffer.jumbled.splits.out_of_memory || self.working_buffer.room.refused()
    }

    // dense_most: the most leaves a rack may have to be kept as a table
    #[inline(always)]
    pub(crate) fn new_in_dense(game_config: &game_config::GameConfig, dense_most: u32) -> Self {
        Self::new_in(game_config).with_dense_most(dense_most)
    }

    #[inline(always)]
    pub(crate) fn with_dense_most(mut self, dense_most: u32) -> Self {
        self.working_buffer.multi_leaves = klv::MultiLeaves::with_dense_most(dense_most);
        self
    }

    #[inline(always)]
    pub fn num_tiles_in_bag(&self) -> i32 {
        self.working_buffer.num_tiles_in_bag
    }

    #[inline]
    pub fn save_board_state(&self, state: &mut BoardState) {
        let w = &self.working_buffer;
        let squares = w.prev_board_tiles.len();
        let _parts = if state.board_tiles.len() != squares {
            use std::mem::size_of;
            let per_square = 2 * size_of::<u8>()
                + 2 * (size_of::<u64>()
                    + 2 * size_of::<i32>()
                    + size_of::<CachedCrossSet>()
                    + size_of::<CrossSetComputation>()
                    + 3 * size_of::<i8>()
                    + size_of::<i32>());
            let Some(parts) = w.room.take(squares * per_square) else {
                return;
            };
            parts
        } else {
            endgame::Reserved::NONE
        };
        copy_board_part(&mut state.board_tiles, &w.prev_board_tiles);
        copy_board_part(&mut state.transposed_board_tiles, &w.transposed_board_tiles);
        state.counts = (
            w.num_tiles_on_board,
            w.board_tile_score,
            w.prev_tile_count,
            w.prev_tile_score,
        );
        copy_board_part(&mut state.allowed[0], &w.allowed_for_across_plays);
        copy_board_part(&mut state.allowed[1], &w.allowed_for_down_plays);
        copy_board_part(&mut state.cross_scores[0], &w.cross_score_for_across_plays);
        copy_board_part(&mut state.cross_scores[1], &w.cross_score_for_down_plays);
        copy_board_part(
            &mut state.cached_cross_sets[0],
            &w.cached_cross_set_for_across_plays,
        );
        copy_board_part(
            &mut state.cached_cross_sets[1],
            &w.cached_cross_set_for_down_plays,
        );
        copy_board_part(
            &mut state.cross_set_buffers[0],
            &w.cross_set_buffer_for_across_plays,
        );
        copy_board_part(
            &mut state.cross_set_buffers[1],
            &w.cross_set_buffer_for_down_plays,
        );
        copy_board_part(
            &mut state.word_multipliers[0],
            &w.remaining_word_multipliers_for_across_plays,
        );
        copy_board_part(
            &mut state.word_multipliers[1],
            &w.remaining_word_multipliers_for_down_plays,
        );
        copy_board_part(
            &mut state.tile_multipliers[0],
            &w.remaining_tile_multipliers_for_across_plays,
        );
        copy_board_part(
            &mut state.tile_multipliers[1],
            &w.remaining_tile_multipliers_for_down_plays,
        );
        copy_board_part(
            &mut state.face_values[0],
            &w.face_value_scores_for_across_plays,
        );
        copy_board_part(
            &mut state.face_values[1],
            &w.face_value_scores_for_down_plays,
        );
        copy_board_part(
            &mut state.perpendicular_word_multipliers[0],
            &w.perpendicular_word_multipliers_for_across_plays,
        );
        copy_board_part(
            &mut state.perpendicular_word_multipliers[1],
            &w.perpendicular_word_multipliers_for_down_plays,
        );
        copy_board_part(
            &mut state.perpendicular_scores[0],
            &w.perpendicular_scores_for_across_plays,
        );
        copy_board_part(
            &mut state.perpendicular_scores[1],
            &w.perpendicular_scores_for_down_plays,
        );
        if w.rcs_pending_rows != G::from(0)
            || w.rcs_pending_cols != G::from(0)
            || w.rcs_lanes.len() < w.board_cols as usize
            || w.room.memory.is_some()
        {
            state.rcs_lanes.clear();
        } else {
            let rows = w.rcs_lanes.len() - w.board_cols as usize;
            state
                .rcs_lanes
                .resize(w.rcs_lanes.len(), RcsLane::default());
            for (at, (to, from)) in state.rcs_lanes.iter_mut().zip(&w.rcs_lanes).enumerate() {
                let stale = if at >= rows {
                    w.rcs_stale_cols & (G::from(1) << (at - rows) as u32)
                } else {
                    w.rcs_stale_rows & (G::from(1) << at as u32)
                } != G::from(0);
                let valid = from.valid && !stale;
                if to.stamp != from.stamp || to.valid != valid {
                    to.clone_from(from);
                    to.valid = valid;
                }
            }
        }
    }

    // puts back what save_board_state kept, for a generator of the same game;
    // its next generation then brings up only the lanes where its board differs
    // from the kept one
    #[inline]
    pub fn restore_board_state(&mut self, state: &BoardState) {
        let w = &mut self.working_buffer;
        if state.board_tiles.len() != w.prev_board_tiles.len() {
            return;
        }
        // the squares where the board put back differs from the one the
        // generator last held. A played word reads 0 on the board's tiles, so
        // such a square keeps no letter a walk left on it (init clears only the
        // squares it sees change)
        let cols = w.board_cols as usize;
        let rows = w.prev_board_tiles.len() / cols.max(1);
        let keeps_lanes = !state.rcs_lanes.is_empty();
        for (idx, (&held, &kept)) in w
            .prev_board_tiles
            .iter()
            .zip(&state.board_tiles)
            .enumerate()
        {
            if held != kept {
                let (row, col) = (idx / cols, idx % cols);
                // with no kept lanes, the table walk looks at these rows and
                // columns again, its kept spans being of the board it last held
                if !keeps_lanes {
                    w.rcs_pending_rows |= G::from(1) << row as u32;
                    w.rcs_pending_cols |= G::from(1) << col as u32;
                }
                w.word_buffer_for_across_plays[idx] = 0;
                w.word_buffer_for_down_plays[col * rows + row] = 0;
            }
        }
        if keeps_lanes {
            // the kept board's lanes, each of the inputs put back below (or not
            // valid): a lane the generator holds a copy of stays, every other
            // is copied, and the span sums of each one copied or marked are
            // filled again
            let rows = state.rcs_lanes.len() - cols;
            if w.rcs_lanes.len() != state.rcs_lanes.len() {
                w.rcs_lanes.clone_from(&state.rcs_lanes);
                w.span_lanes.iter_mut().for_each(|lane| lane.kept = false);
            } else {
                for (at, (to, from)) in w.rcs_lanes.iter_mut().zip(&state.rcs_lanes).enumerate() {
                    let stale = if at >= rows {
                        w.rcs_stale_cols & (G::from(1) << (at - rows) as u32)
                    } else {
                        w.rcs_stale_rows & (G::from(1) << at as u32)
                    } != G::from(0);
                    if to.stamp != from.stamp || to.valid != from.valid {
                        to.clone_from(from);
                        w.span_lanes[at].kept = false;
                    } else if stale {
                        w.span_lanes[at].kept = false;
                    }
                }
            }
            w.rcs_pending_rows = G::from(0);
            w.rcs_pending_cols = G::from(0);
            w.rcs_stale_rows = G::from(0);
            w.rcs_stale_cols = G::from(0);
        }
        copy_board_part(&mut w.prev_board_tiles, &state.board_tiles);
        copy_board_part(&mut w.transposed_board_tiles, &state.transposed_board_tiles);
        (
            w.num_tiles_on_board,
            w.board_tile_score,
            w.prev_tile_count,
            w.prev_tile_score,
        ) = state.counts;
        copy_board_part(&mut w.allowed_for_across_plays, &state.allowed[0]);
        copy_board_part(&mut w.allowed_for_down_plays, &state.allowed[1]);
        copy_board_part(&mut w.cross_score_for_across_plays, &state.cross_scores[0]);
        copy_board_part(&mut w.cross_score_for_down_plays, &state.cross_scores[1]);
        copy_board_part(
            &mut w.cached_cross_set_for_across_plays,
            &state.cached_cross_sets[0],
        );
        copy_board_part(
            &mut w.cached_cross_set_for_down_plays,
            &state.cached_cross_sets[1],
        );
        copy_board_part(
            &mut w.cross_set_buffer_for_across_plays,
            &state.cross_set_buffers[0],
        );
        copy_board_part(
            &mut w.cross_set_buffer_for_down_plays,
            &state.cross_set_buffers[1],
        );
        copy_board_part(
            &mut w.remaining_word_multipliers_for_across_plays,
            &state.word_multipliers[0],
        );
        copy_board_part(
            &mut w.remaining_word_multipliers_for_down_plays,
            &state.word_multipliers[1],
        );
        copy_board_part(
            &mut w.remaining_tile_multipliers_for_across_plays,
            &state.tile_multipliers[0],
        );
        copy_board_part(
            &mut w.remaining_tile_multipliers_for_down_plays,
            &state.tile_multipliers[1],
        );
        copy_board_part(
            &mut w.face_value_scores_for_across_plays,
            &state.face_values[0],
        );
        copy_board_part(
            &mut w.face_value_scores_for_down_plays,
            &state.face_values[1],
        );
        copy_board_part(
            &mut w.perpendicular_word_multipliers_for_across_plays,
            &state.perpendicular_word_multipliers[0],
        );
        copy_board_part(
            &mut w.perpendicular_word_multipliers_for_down_plays,
            &state.perpendicular_word_multipliers[1],
        );
        copy_board_part(
            &mut w.perpendicular_scores_for_across_plays,
            &state.perpendicular_scores[0],
        );
        copy_board_part(
            &mut w.perpendicular_scores_for_down_plays,
            &state.perpendicular_scores[1],
        );
    }

    // call this before passing a different kwg.
    #[inline(always)]
    pub fn reset_for_another_kwg(&mut self) {
        self.working_buffer.reset_for_another_kwg();
    }

    // the buffers that grow with what is walked, not with the board or rack
    // (the plays, subracks, leaves, placements, spans, memos, and the jumbled
    // and pool lists), emptied and their room let go, so that a generator kept
    // idle holds what the board and rack size alone; the key check memo, made
    // at first use, is let go to be made again
    pub fn shrink_walk_buffers(&mut self) {
        let w = &mut self.working_buffer;
        self.plays.clear();
        self.plays.shrink_to_fit();
        w.subracks.clear();
        w.subracks.shrink_to_fit();
        w.subracks_spare.clear();
        w.subracks_spare.shrink_to_fit();
        w.subracks_by_played.clear();
        w.subracks_by_played.shrink_to_fit();
        w.subrack_order.clear();
        w.subrack_order.shrink_to_fit();
        w.span_out.clear();
        w.span_out.shrink_to_fit();
        w.found_placements.clear();
        w.found_placements.shrink_to_fit();
        w.raw_anchors.clear();
        w.raw_anchors.shrink_to_fit();
        w.sure_plays.clear();
        w.sure_plays.shrink_to_fit();
        w.placement_order.clear();
        w.placement_order.shrink_to_fit();
        w.placement_scratch.clear();
        w.placement_scratch.shrink_to_fit();
        w.anchor_left_bounds.clear();
        w.anchor_left_bounds.shrink_to_fit();
        // the span memo's slots stay a power of two of them, as its lookups
        // mask by their count; each turn grows them to its placements'
        w.span_memo.slots.truncate(1);
        w.span_memo.slots.shrink_to_fit();
        w.span_memo.visits.clear();
        w.span_memo.visits.shrink_to_fit();
        w.exist_memo = Box::default();
        w.multi_leaves.shrink();
        w.jumbled.shrink();
        for list in [&mut w.pool.walked, &mut w.pool.letters, &mut w.pool.word] {
            list.clear();
            list.shrink_to_fit();
        }
        w.pool.deltas.clear();
        w.pool.deltas.shrink_to_fit();
    }

    #[cfg(not(target_family = "wasm"))]
    pub(crate) fn gen_outs<N: kwg::Node, L: kwg::Node>(
        &mut self,
        snapshot: &BoardSnapshot<'_, N, L>,
        rack: &[u8],
        (rows, cols): (u128, u128),
        mut found: impl FnMut(bool, i8, equity::Equity),
    ) {
        let dim = snapshot.game_config.board_layout().dim();
        if rack.is_empty()
            || rack.len()
                > (dim.rows.max(dim.cols) as usize)
                    .min(usize::from(snapshot.game_config.rack_size()))
        {
            return;
        }
        let w = &mut self.working_buffer;
        w.init::<0, false, ByLayout, _, _>(
            snapshot,
            rack,
            klv::AdjustLeave::Identity,
            None,
            w.multi_leaves.dense_most(),
            None,
        );
        let previous_lanes = (w.lane_rows, w.lane_cols);
        (w.lane_rows, w.lane_cols) = (G::of_u128(rows), G::of_u128(cols));
        if rack.len() > 1
            && w.num_tiles_on_board != 0
            && matches!(
                snapshot.game_config.game_rules(),
                game_config::GameRules::Classic
            )
            && let Some(table) = snapshot
                .anagrams
                .filter(|table| table.fit(snapshot.game_config))
        {
            lean_cross_sets::<0, _, _, _, _, _>(snapshot, w);
            table_outs(snapshot, w, table, rack, &mut found);
            (w.lane_rows, w.lane_cols) = previous_lanes;
            return;
        }
        let multi_leaves = std::mem::take(&mut w.multi_leaves);
        let opening = w.num_tiles_on_board == 0;
        walk_raw_places::<true, ANY_SIDE, _, _, _, _, _, _, _>(
            (),
            snapshot,
            w,
            &multi_leaves,
            |down, lane, _, word, score, _| {
                if !opening || word.iter().filter(|&&tile| tile != 0).count() == rack.len() {
                    found(down, lane, score);
                }
                equity::Equity::NEG_INFINITY
            },
        );
        w.multi_leaves = multi_leaves;
        (w.lane_rows, w.lane_cols) = previous_lanes;
    }

    // as gen_moves_raw_all_unsorted, the place moves on the rows and columns in
    // the masks only (bit i, row or column i): the moves a change to the board
    // can touch, when the rest are known
    #[inline(always)]
    pub fn gen_moves_raw_lanes_unsorted<'a, N: kwg::Node, L: kwg::Node>(
        &mut self,
        board_snapshot: &'a BoardSnapshot<'a, N, L>,
        rack: &'a [u8],
        num_exchanges_by_this_player: i16,
        pass_policy: PassPolicy,
        rows: u128,
        cols: u128,
    ) -> error::Returns<()> {
        check_exchange_limit(board_snapshot.game_config)?;
        self.gen_moves_raw_lanes_unsorted_impl(
            board_snapshot,
            rack,
            num_exchanges_by_this_player,
            pass_policy,
            rows,
            cols,
        );
        Ok(())
    }

    // as gen_moves_raw_lanes_unsorted, by the lean arm alone, so that a build
    // that runs only that arm (the wasm) need not carry the other
    #[inline(always)]
    pub fn gen_moves_raw_lanes_unsorted_lean<'a, N: kwg::Node, L: kwg::Node>(
        &mut self,
        board_snapshot: &'a BoardSnapshot<'a, N, L>,
        rack: &'a [u8],
        num_exchanges_by_this_player: i16,
        pass_policy: PassPolicy,
        rows: u128,
        cols: u128,
    ) -> error::Returns<()> {
        check_exchange_limit(board_snapshot.game_config)?;
        self.gen_moves_raw_lanes_unsorted_impl(
            board_snapshot,
            rack,
            num_exchanges_by_this_player,
            pass_policy,
            rows,
            cols,
        );
        Ok(())
    }

    #[inline(always)]
    pub(crate) fn gen_raw_placements_in_lanes<'a, N: kwg::Node, L: kwg::Node>(
        &mut self,
        board_snapshot: &'a BoardSnapshot<'a, N, L>,
        rack: &'a [u8],
        rows: u128,
        cols: u128,
    ) {
        self.gen_moves_raw_lanes_unsorted_impl(
            board_snapshot,
            rack,
            i16::MAX,
            PassPolicy::OnlyWhenForced,
            rows,
            cols,
        );
    }

    #[inline(always)]
    fn gen_moves_raw_lanes_unsorted_impl<'a, N: kwg::Node, L: kwg::Node>(
        &mut self,
        board_snapshot: &'a BoardSnapshot<'a, N, L>,
        rack: &'a [u8],
        num_exchanges_by_this_player: i16,
        pass_policy: PassPolicy,
        rows: u128,
        cols: u128,
    ) {
        self.working_buffer.lane_rows = G::of_u128(rows);
        self.working_buffer.lane_cols = G::of_u128(cols);
        self.gen_moves_raw_all_unsorted_impl(
            board_snapshot,
            rack,
            num_exchanges_by_this_player,
            pass_policy,
            (),
        );
        self.working_buffer.lane_rows = !G::from(0);
        self.working_buffer.lane_cols = !G::from(0);
    }

    // as gen_moves_raw_lanes_unsorted with no exchange made and a pass a
    // candidate, as an endgame searches, where each walk along a run of the
    // word graph's nodes stops once run_letter_bits (by node, the tiles from it
    // to the end of its run: kwg::Kwg::compute_letter_bits) leaves it no
    // candidate; by the lean arm
    #[inline(always)]
    pub(crate) fn gen_moves_raw_lanes_unsorted_by_runs<'a, N: kwg::Node, L: kwg::Node>(
        &mut self,
        run_letter_bits: &'a [u64],
        board_snapshot: &'a BoardSnapshot<'a, N, L>,
        rack: &'a [u8],
        rows: u128,
        cols: u128,
    ) -> error::Returns<()> {
        check_exchange_limit(board_snapshot.game_config)?;
        debug_assert_eq!(run_letter_bits.len(), board_snapshot.kwg.0.len());
        self.working_buffer.lane_rows = G::of_u128(rows);
        self.working_buffer.lane_cols = G::of_u128(cols);
        self.gen_moves_raw_all_unsorted_impl(
            board_snapshot,
            rack,
            0,
            PassPolicy::AsACandidate,
            run_letter_bits,
        );
        self.working_buffer.lane_rows = !G::from(0);
        self.working_buffer.lane_cols = !G::from(0);
        Ok(())
    }

    // skip equity computation and sorting
    #[inline(always)]
    pub fn gen_moves_raw_all_unsorted<'a, N: kwg::Node, L: kwg::Node>(
        &mut self,
        board_snapshot: &'a BoardSnapshot<'a, N, L>,
        rack: &'a [u8],
        num_exchanges_by_this_player: i16,
        pass_policy: PassPolicy,
    ) -> error::Returns<()> {
        check_exchange_limit(board_snapshot.game_config)?;
        self.gen_moves_raw_all_unsorted_impl(
            board_snapshot,
            rack,
            num_exchanges_by_this_player,
            pass_policy,
            (),
        );
        Ok(())
    }

    #[inline(always)]
    pub fn gen_moves_raw_all_unsorted_lean<'a, N: kwg::Node, L: kwg::Node>(
        &mut self,
        board_snapshot: &'a BoardSnapshot<'a, N, L>,
        rack: &'a [u8],
        num_exchanges_by_this_player: i16,
        pass_policy: PassPolicy,
    ) -> error::Returns<()> {
        check_exchange_limit(board_snapshot.game_config)?;
        self.gen_moves_raw_all_unsorted_impl(
            board_snapshot,
            rack,
            num_exchanges_by_this_player,
            pass_policy,
            (),
        );
        Ok(())
    }

    // gen_moves_raw_all_unsorted under a limit: true, the list cut short,
    // where growing it would pass the limit
    #[inline(always)]
    pub(crate) fn gen_moves_raw_all_unsorted_held<'a, N: kwg::Node, L: kwg::Node>(
        &mut self,
        board_snapshot: &'a BoardSnapshot<'a, N, L>,
        rack: &'a [u8],
        num_exchanges_by_this_player: i16,
        pass_policy: PassPolicy,
        limit: HeldBytes,
    ) -> error::Returns<bool> {
        check_exchange_limit(board_snapshot.game_config)?;
        Ok(self.gen_moves_raw_all_unsorted_within(
            board_snapshot,
            rack,
            num_exchanges_by_this_player,
            pass_policy,
            (),
            limit,
        ))
    }

    #[inline(always)]
    pub(crate) fn gen_moves_raw_lanes_unsorted_within<'a, N: kwg::Node, L: kwg::Node>(
        &mut self,
        board_snapshot: &'a BoardSnapshot<'a, N, L>,
        rack: &'a [u8],
        rows: u128,
        cols: u128,
        limit: HeldBytes,
    ) -> error::Returns<bool> {
        check_exchange_limit(board_snapshot.game_config)?;
        self.working_buffer.lane_rows = G::of_u128(rows);
        self.working_buffer.lane_cols = G::of_u128(cols);
        let stopped = self.gen_moves_raw_all_unsorted_within(
            board_snapshot,
            rack,
            0,
            PassPolicy::AsACandidate,
            (),
            limit,
        );
        self.working_buffer.lane_rows = !G::from(0);
        self.working_buffer.lane_cols = !G::from(0);
        Ok(stopped)
    }

    #[inline(always)]
    pub(crate) fn gen_moves_raw_lanes_unsorted_by_runs_within<'a, N: kwg::Node, L: kwg::Node>(
        &mut self,
        run_letter_bits: &'a [u64],
        board_snapshot: &'a BoardSnapshot<'a, N, L>,
        rack: &'a [u8],
        rows: u128,
        cols: u128,
        limit: HeldBytes,
    ) -> error::Returns<bool> {
        check_exchange_limit(board_snapshot.game_config)?;
        debug_assert_eq!(run_letter_bits.len(), board_snapshot.kwg.0.len());
        self.working_buffer.lane_rows = G::of_u128(rows);
        self.working_buffer.lane_cols = G::of_u128(cols);
        let stopped = self.gen_moves_raw_all_unsorted_within(
            board_snapshot,
            rack,
            0,
            PassPolicy::AsACandidate,
            run_letter_bits,
            limit,
        );
        self.working_buffer.lane_rows = !G::from(0);
        self.working_buffer.lane_cols = !G::from(0);
        Ok(stopped)
    }

    #[inline]
    fn gen_moves_raw_all_unsorted_impl<'a, R: 'a + RunLetters, N: kwg::Node, L: kwg::Node>(
        &mut self,
        board_snapshot: &'a BoardSnapshot<'a, N, L>,
        rack: &'a [u8],
        num_exchanges_by_this_player: i16,
        pass_policy: PassPolicy,
        runs: R,
    ) {
        self.gen_moves_raw_all_unsorted_within(
            board_snapshot,
            rack,
            num_exchanges_by_this_player,
            pass_policy,
            runs,
            (),
        );
    }

    #[inline]
    fn gen_moves_raw_all_unsorted_within<
        'a,
        R: 'a + RunLetters,
        T: RawStop,
        N: kwg::Node,
        L: kwg::Node,
    >(
        &mut self,
        board_snapshot: &'a BoardSnapshot<'a, N, L>,
        rack: &'a [u8],
        num_exchanges_by_this_player: i16,
        pass_policy: PassPolicy,
        runs: R,
        stop: T,
    ) -> bool {
        self.plays.clear();
        let mut vec_moves = std::mem::take(&mut self.plays);

        let working_buffer = &mut self.working_buffer;
        working_buffer.init::<0, false, ByLayout, _, _>(
            board_snapshot,
            rack,
            klv::AdjustLeave::Identity,
            None,
            working_buffer.multi_leaves.dense_most(),
            None,
        );
        let multi_leaves = std::mem::take(&mut working_buffer.multi_leaves);

        let grown_bytes = working_buffer.grown_bytes;
        let mut stopped = false;
        let mut held = endgame::Reserved::NONE;
        #[inline(always)]
        fn cut<T: RawStop>(
            stop: &T,
            moves: &Vec<ValuedMove>,
            grown: usize,
            held: &mut endgame::Reserved,
        ) -> bool {
            if moves.len() != moves.capacity() {
                return false;
            }
            *held = endgame::Reserved::NONE;
            match stop.reserve(moves.capacity().max(1), grown) {
                Some(taken) => {
                    *held = taken;
                    false
                }
                None => true,
            }
        }
        let found_place_move = |down: bool,
                                lane: i8,
                                idx: i8,
                                word: &[u8],
                                score: equity::Equity,
                                _leave_value: equity::Equity| {
            if T::STOPS && (stopped || cut(&stop, &vec_moves, grown_bytes, &mut held)) {
                stopped = true;
                return equity::Equity::INFINITY;
            }
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
            equity::Equity::NEG_INFINITY
        };
        walk_raw_places::<false, ANY_SIDE, _, _, _, _, _, _, _>(
            runs,
            board_snapshot,
            working_buffer,
            &multi_leaves,
            found_place_move,
        );
        kurnia_gen_exchange_moves(
            board_snapshot,
            working_buffer,
            &multi_leaves,
            num_exchanges_by_this_player,
            |exchanged_tiles: &[u8], _leave_value: equity::Equity| {
                if T::STOPS && (stopped || cut(&stop, &vec_moves, grown_bytes, &mut held)) {
                    stopped = true;
                    return;
                }
                vec_moves.push(ValuedMove {
                    equity: equity::Equity::ZERO,
                    play: Play::Exchange {
                        tiles: exchanged_tiles.into(),
                    },
                });
            },
        );
        if T::STOPS
            && !stopped
            && (pass_policy.emits_unconditionally() || vec_moves.is_empty())
            && cut(&stop, &vec_moves, grown_bytes, &mut held)
        {
            stopped = true;
        }
        if !stopped && (pass_policy.emits_unconditionally() || vec_moves.is_empty()) {
            vec_moves.push(ValuedMove {
                equity: equity::Equity::ZERO,
                play: Play::Exchange {
                    tiles: (&working_buffer.exchange_buffer[..]).into(),
                },
            });
        }

        self.plays = vec_moves;

        working_buffer.multi_leaves = multi_leaves;
        stopped || working_buffer.room.refused()
    }

    // the best one kept in one slot, more of them in a heap
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
    ) -> error::Returns<()> {
        check_exchange_limit(params.board_snapshot.game_config)?;
        adjust_leave_value.check_for_klv(params.board_snapshot.klv)?;
        if params.max_gen == 1 {
            self.gen_moves_filtered_async_lean_kept::<KeptOne, _, _, _>(
                params,
                place_move_predicate,
                adjust_leave_value,
                equity_predicate,
                breathe,
            )
            .await
        } else {
            self.gen_moves_filtered_async_lean_kept::<KeptHeap, _, _, _>(
                params,
                place_move_predicate,
                adjust_leave_value,
                equity_predicate,
                breathe,
            )
            .await
        }
        Ok(())
    }

    #[inline(always)]
    async fn gen_moves_filtered_async_lean_kept<
        'a,
        K: Kept,
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

        let max_gen = params.max_gen;

        let mut found_moves = K::from_vec(std::mem::take(&mut self.plays));
        let mut equity_predicate = equity_predicate;
        let threshold = std::cell::Cell::new(equity::Equity::NEG_INFINITY);

        let working_buffer = &mut self.working_buffer;
        working_buffer.init::<0, true, ByLayout, _, _>(
            params.board_snapshot,
            params.rack,
            adjust_leave_value,
            params.dynamic_leaves,
            working_buffer.multi_leaves.dense_most(),
            None,
        );
        let multi_leaves = std::mem::take(&mut working_buffer.multi_leaves);
        if let Some(floor) = sure_floor(params, working_buffer, &multi_leaves, &equity_predicate) {
            threshold.set(floor);
        }

        let found_place_move = |down: bool,
                                lane: i8,
                                idx: i8,
                                word: &[u8],
                                score: equity::Equity,
                                leave_value: equity::Equity| {
            if place_move_predicate.test(down, lane, idx, word, score) {
                push_kept(
                    &mut found_moves,
                    &mut equity_predicate,
                    &threshold,
                    max_gen,
                    equity::Equity::new(score.raw() + leave_value.raw()),
                    || Play::Place {
                        down,
                        lane,
                        idx,
                        word: word.into(),
                        score,
                    },
                );
            }
            threshold.get()
        };
        if working_buffer.num_tiles_on_rack == 1 {
            lean_cross_sets::<0, _, _, _, _, _>(params.board_snapshot, working_buffer);
            kurnia_gen_one_tile_moves(
                params.board_snapshot,
                working_buffer,
                &multi_leaves,
                found_place_move,
            );
        } else if working_buffer.num_tiles_on_board == 0 {
            gen_empty_board_place_moves::<0, _, _, _, _, _, _, _>(
                (),
                params.board_snapshot,
                working_buffer,
                &multi_leaves,
                threshold.get().raw(),
                found_place_move,
            );
            breathe().await;
        } else {
            for _ in kurnia_gen_place_moves_iter_lean::<0, G, _, _, _, _, _, _, _, _, _>(
                KurniaIterParams {
                    runs: (),
                    board_snapshot: params.board_snapshot,
                    working_buffer,
                    multi_leaves: &multi_leaves,
                    found_place_move,
                    can_accept: |best_possible_equity: equity::Equity| {
                        threshold.get() <= best_possible_equity
                    },
                    current_threshold: || threshold.get(),
                },
            ) {
                breathe().await;
            }
        }
        if threshold.get()
            <= best_exchange_equity::<BAG_TERM>(
                params.board_snapshot.game_config,
                &working_buffer.best_leave_values,
                &multi_leaves,
            )
        {
            kurnia_gen_exchange_moves(
                params.board_snapshot,
                working_buffer,
                &multi_leaves,
                params.num_exchanges_by_this_player,
                |exchanged_tiles: &[u8], leave_value: equity::Equity| {
                    push_kept(
                        &mut found_moves,
                        &mut equity_predicate,
                        &threshold,
                        max_gen,
                        leave_value,
                        || Play::Exchange {
                            tiles: exchanged_tiles.into(),
                        },
                    );
                },
            );
        }
        if params.pass_policy.emits_unconditionally() || found_moves.is_empty() {
            push_kept(
                &mut found_moves,
                &mut equity_predicate,
                &threshold,
                max_gen,
                equity::Equity::new(if multi_leaves.is_dense() {
                    multi_leaves.pass_leave_value().raw()
                } else {
                    params
                        .board_snapshot
                        .klv
                        .leave_value_from_tally(&working_buffer.rack_tally)
                        .raw()
                }),
                || Play::Exchange {
                    tiles: (&working_buffer.exchange_buffer[..]).into(),
                },
            );
        }

        working_buffer.jumbled.splits.out_of_memory |= found_moves.refused();
        self.plays = found_moves.into_sorted_vec();

        working_buffer.multi_leaves = multi_leaves;
    }

    #[inline(always)]
    // the census sheet wants each word once, not each play, so its descent
    // takes a real tile before a blank. That is a different generator, not a
    // setting: the placement path that reads the anagrams does not run it, so
    // the census is the lean arm's, as gen_census_sheet_lean.
    pub fn gen_census_sheet<'a, N: kwg::Node, L: kwg::Node>(
        &mut self,
        params: &'a GenMovesParams<'a, N, L>,
        place_move_predicate: PlacePredicate<'_>,
        adjust_leave_value: klv::AdjustLeave,
        equity_predicate: EquityPredicate<'_>,
    ) -> error::Returns<()> {
        adjust_leave_value.check_for_klv(params.board_snapshot.klv)?;
        self.gen_census_sheet_impl(
            params,
            place_move_predicate,
            adjust_leave_value,
            equity_predicate,
        );
        Ok(())
    }

    // as gen_census_sheet, itself the lean arm's: kept beside the other lean
    // entries for a build that runs only that arm (the wasm)
    #[inline(always)]
    pub fn gen_census_sheet_lean<'a, N: kwg::Node, L: kwg::Node>(
        &mut self,
        params: &'a GenMovesParams<'a, N, L>,
        place_move_predicate: PlacePredicate<'_>,
        adjust_leave_value: klv::AdjustLeave,
        equity_predicate: EquityPredicate<'_>,
    ) -> error::Returns<()> {
        adjust_leave_value.check_for_klv(params.board_snapshot.klv)?;
        self.gen_census_sheet_impl(
            params,
            place_move_predicate,
            adjust_leave_value,
            equity_predicate,
        );
        Ok(())
    }

    // the census of a board over the tiles of pool (no blank among them), by
    // the lean arm's census walk: each play of them of at most a rack's tiles
    // is scored (the walk scores none), its tiles keyed as the anagram tables
    // key a subrack, and its best score kept lane by lane
    #[inline]
    pub fn lane_census<N: kwg::Node, L: kwg::Node>(
        &mut self,
        board_snapshot: &BoardSnapshot<'_, N, L>,
        anagrams: &anagrams::Anagrams,
        pool: &[u8],
    ) -> LaneCensus {
        let dim = board_snapshot.game_config.board_layout().dim();
        let mut census = LaneCensus::new(
            anagrams.nibble_side().is_some(),
            dim.rows as usize + dim.cols as usize,
        );
        self.lane_census_into(board_snapshot, anagrams, pool, &mut census);
        census
    }

    // the same, taken in census in place of the one it held, its memory kept
    #[inline]
    pub fn lane_census_into<N: kwg::Node, L: kwg::Node>(
        &mut self,
        board_snapshot: &BoardSnapshot<'_, N, L>,
        anagrams: &anagrams::Anagrams,
        pool: &[u8],
        census: &mut LaneCensus,
    ) {
        let nibbles = anagrams.nibble_side().is_some();
        let layout = anagrams.layout();
        let dim = board_snapshot.game_config.board_layout().dim();
        let rows = dim.rows as u8;
        let rack_size = board_snapshot.game_config.rack_size() as usize;
        census.reset(nibbles, dim.rows as usize + dim.cols as usize);
        let mut deltas = std::mem::take(&mut census.deltas);
        let snapshot = BoardSnapshot {
            board_tiles: board_snapshot.board_tiles,
            game_config: board_snapshot.game_config,
            kwg: board_snapshot.kwg,
            anagrams: None,
            rack_lengths: None,
            klv: board_snapshot.klv,
        };
        let params = GenMovesParams {
            board_snapshot: &snapshot,
            rack: pool,
            max_gen: 1,
            num_exchanges_by_this_player: i16::MAX,
            pass_policy: PassPolicy::OnlyWhenForced,
            dynamic_leaves: None,
        };
        self.gen_census_sheet_impl(
            &params,
            PlacePredicate::Dyn(
                &mut |down, lane, idx, word: &[u8], _score: equity::Equity| {
                    if word.iter().filter(|&&tile| tile != 0).count() <= rack_size {
                        let key = word
                            .iter()
                            .filter(|&&tile| tile != 0)
                            .map(|&tile| {
                                if nibbles {
                                    alphagram::Nibble.place_value(tile)
                                } else {
                                    layout.place_value(tile)
                                }
                            })
                            .sum::<u128>();
                        let score = play_scorer::score_and_blank_deltas(
                            &snapshot,
                            down,
                            lane,
                            idx,
                            word,
                            &mut deltas,
                        );
                        let lane_id = if down { rows + lane as u8 } else { lane as u8 };
                        census.add(key, lane_id, score);
                    }
                    false
                },
            ),
            klv::AdjustLeave::Identity,
            EquityPredicate::RejectAll,
        );
        census.deltas = deltas;
    }

    #[inline(always)]
    pub fn gen_moves_filtered_lean_walked<'a, W: JumbledWalk, N: kwg::Node, L: kwg::Node>(
        &mut self,
        params: &'a GenMovesParams<'a, N, L>,
        place_move_predicate: PlacePredicate<'_>,
        adjust_leave_value: klv::AdjustLeave,
        equity_predicate: EquityPredicate<'_>,
    ) -> error::Returns<()> {
        check_exchange_limit(params.board_snapshot.game_config)?;
        adjust_leave_value.check_for_klv(params.board_snapshot.klv)?;
        self.working_buffer.jumbled.walk = W::WALK;
        self.gen_moves_filtered_lean_by_side(
            params,
            place_move_predicate,
            adjust_leave_value,
            equity_predicate,
            None,
        );
        self.working_buffer.jumbled.walk = JUMBLED_EVERY_ARRANGEMENT;
        Ok(())
    }

    // every play of at most a rack's tiles placeable from the tiles of pool
    // (blanks among them as 0). Each way of designating blanks is its own play.
    // found(down, lane, idx, word, score) is called once for each play; word
    // and score are as a Play::Place's. It runs the census walk that reads no
    // leave. That walk spells each word once with a real tile before a blank.
    // So each word is taken in its letters to score every designation the pool
    // allows.
    // one-word rules only: it walks the graph's words, so its caller never
    // gives it a woadwage game
    pub fn pool_plays<N: kwg::Node, L: kwg::Node>(
        &mut self,
        board_snapshot: &BoardSnapshot<'_, N, L>,
        pool: &[u8],
        mut found: impl FnMut(bool, i8, i8, &[u8], i32),
    ) {
        let game_config = board_snapshot.game_config;
        let rack_size = game_config.rack_size();
        // no play places more of a tile than a rack holds
        let mut tally = [0u8; MAX_ALPHABET_LEN];
        let tally = &mut tally[..game_config.alphabet().len() as usize];
        let Some(walked) = self.pool_rack(pool, rack_size, tally) else {
            return;
        };
        let snapshot = BoardSnapshot {
            board_tiles: board_snapshot.board_tiles,
            game_config,
            kwg: board_snapshot.kwg,
            anagrams: None,
            rack_lengths: None,
            klv: board_snapshot.klv,
        };
        let params = GenMovesParams {
            board_snapshot: &snapshot,
            rack: &walked,
            max_gen: 1,
            num_exchanges_by_this_player: i16::MAX,
            pass_policy: PassPolicy::OnlyWhenForced,
            dynamic_leaves: None,
        };
        let jumbled = match game_config.game_rules() {
            game_config::GameRules::Classic | game_config::GameRules::Woadwage => false,
            game_config::GameRules::Jumbled | game_config::GameRules::JumbledWoadwage => true,
        };
        if jumbled {
            let walk = self.working_buffer.jumbled.walk;
            self.working_buffer.jumbled.walk = JUMBLED_ONCE_PER_RACK_TILES;
            self.gen_sheet_impl(
                &params,
                PlacePredicate::Dyn(&mut |down, lane, idx, word: &[u8], score: equity::Equity| {
                    if word.iter().filter(|&&tile| tile != 0).count() <= rack_size as usize {
                        found(down, lane, idx, word, score.raw());
                    }
                    false
                }),
                klv::AdjustLeave::Identity,
                EquityPredicate::RejectAll,
                false,
            );
            self.working_buffer.jumbled.walk = walk;
            self.working_buffer.pool.walked = walked;
            return;
        }
        let dim = game_config.board_layout().dim();
        let longest = dim.rows.max(dim.cols) as usize;
        let mut letters = std::mem::take(&mut self.working_buffer.pool.letters);
        let mut deltas = std::mem::take(&mut self.working_buffer.pool.deltas);
        let mut word = std::mem::take(&mut self.working_buffer.pool.word);
        letters.clear();
        deltas.clear();
        word.clear();
        if letters.try_reserve(longest).is_err()
            || deltas.try_reserve(longest).is_err()
            || word.try_reserve(longest).is_err()
        {
            self.working_buffer.pool = PoolScratch {
                walked,
                letters,
                deltas,
                word,
            };
            self.working_buffer.jumbled.splits.out_of_memory = true;
            return;
        }
        self.gen_census_sheet_impl(
            &params,
            PlacePredicate::Dyn(
                &mut |down, lane, idx, spelled: &[u8], _score: equity::Equity| {
                    if spelled.iter().filter(|&&tile| tile != 0).count() <= rack_size as usize {
                        letters.clear();
                        letters.extend(spelled.iter().map(|&tile| tile & 0x7f));
                        let score = play_scorer::score_and_blank_deltas(
                            &snapshot,
                            down,
                            lane,
                            idx,
                            &letters,
                            &mut deltas,
                        );
                        word.clear();
                        word.extend_from_slice(&letters);
                        designate(
                            tally,
                            &deltas,
                            &mut word,
                            0,
                            0,
                            score,
                            &mut |word: &[u8], score| found(down, lane, idx, word, score),
                        );
                    }
                    false
                },
            ),
            klv::AdjustLeave::Identity,
            EquityPredicate::RejectAll,
        );
        self.working_buffer.pool = PoolScratch {
            walked,
            letters,
            deltas,
            word,
        };
    }

    // the rack a pool's walk takes, at most a rack's of each tile, tallied in
    // tally, in the generator's own buffer; None where it found no room
    fn pool_rack(&mut self, pool: &[u8], rack_size: u8, tally: &mut [u8]) -> Option<Vec<u8>> {
        for &tile in pool {
            tally[tile as usize] = tally[tile as usize].saturating_add(1);
        }
        let mut walked = std::mem::take(&mut self.working_buffer.pool.walked);
        walked.clear();
        let len = tally
            .iter()
            .map(|&count| count.min(rack_size) as usize)
            .sum();
        if walked.try_reserve(len).is_err() {
            self.working_buffer.pool.walked = walked;
            self.working_buffer.jumbled.splits.out_of_memory = true;
            return None;
        }
        walked.extend(
            (0u8..).zip(tally.iter()).flat_map(|(tile, &count)| {
                std::iter::repeat_n(tile, count.min(rack_size) as usize)
            }),
        );
        Some(walked)
    }

    // one-word rules only: it walks the graph's words, so its caller never
    // gives it a woadwage game
    pub fn pool_plays_through<N: kwg::Node, L: kwg::Node>(
        &mut self,
        board_snapshot: &BoardSnapshot<'_, N, L>,
        pool: &[u8],
        (row, col): (i8, i8),
        mut found: impl FnMut(bool, i8, i8, &[u8], i32),
    ) {
        self.pool_plays_lanes(
            board_snapshot,
            pool,
            (1 << row, 1 << col),
            |down, lane, idx, word, score| {
                let (along, at) = if down { (col, row) } else { (row, col) };
                if lane == along
                    && at >= idx
                    && word.get((at - idx) as usize).is_some_and(|&tile| tile != 0)
                {
                    found(down, lane, idx, word, score);
                }
            },
        );
    }

    // one-word rules only: it walks the graph's words, so its caller never
    // gives it a woadwage game
    pub fn pool_plays_spans_through<N: kwg::Node, L: kwg::Node>(
        &mut self,
        board_snapshot: &BoardSnapshot<'_, N, L>,
        pool: &[u8],
        (row, col): (i8, i8),
        mut found: impl FnMut(bool, i8, i8, &[u8], i32),
    ) {
        let mut out_of_memory = false;
        for (lanes, through) in [((1 << row, 0), col), ((0, 1 << col), row)] {
            self.pool_plays_window(
                board_snapshot,
                pool,
                lanes,
                0..u64::MAX,
                Some(through),
                |down, lane, idx, word, score| {
                    let (along, at) = if down { (col, row) } else { (row, col) };
                    if lane == along
                        && at >= idx
                        && word.get((at - idx) as usize).is_some_and(|&tile| tile != 0)
                    {
                        found(down, lane, idx, word, score);
                    }
                },
            );
            out_of_memory |= self.out_of_memory();
        }
        self.working_buffer.jumbled.splits.out_of_memory = out_of_memory;
    }

    // one-word rules only: it walks the graph's words, so its caller never
    // gives it a woadwage game
    pub fn pool_plays_spans_through_top<N: kwg::Node, L: kwg::Node>(
        &mut self,
        board_snapshot: &BoardSnapshot<'_, N, L>,
        pool: &[u8],
        (row, col): (i8, i8),
        most: usize,
        mut found: impl FnMut(bool, i8, i8, &[u8], i32),
    ) {
        let game_config = board_snapshot.game_config;
        let jumbled = match game_config.game_rules() {
            game_config::GameRules::Classic | game_config::GameRules::Woadwage => false,
            game_config::GameRules::Jumbled | game_config::GameRules::JumbledWoadwage => true,
        };
        if !jumbled {
            self.pool_plays_spans_through(board_snapshot, pool, (row, col), found);
            return;
        }
        let rack_size = game_config.rack_size();
        let mut tally = [0u8; MAX_ALPHABET_LEN];
        let tally = &mut tally[..game_config.alphabet().len() as usize];
        let Some(walked) = self.pool_rack(pool, rack_size, tally) else {
            return;
        };
        let params = GenMovesParams {
            board_snapshot,
            rack: &walked,
            max_gen: most,
            num_exchanges_by_this_player: i16::MAX,
            pass_policy: PassPolicy::OnlyWhenForced,
            dynamic_leaves: None,
        };
        let walk = self.working_buffer.jumbled.walk;
        for (lanes, through) in [((1u128 << row, 0), col), ((0, 1u128 << col), row)] {
            self.working_buffer.lane_rows = G::of_u128(lanes.0);
            self.working_buffer.lane_cols = G::of_u128(lanes.1);
            self.working_buffer.jumbled.walk = JUMBLED_ONCE_PER_RACK_TILES;
            self.working_buffer.jumbled.through = Some(through);
            self.gen_sheet_impl(
                &params,
                PlacePredicate::Dyn(&mut |down, lane, idx, word: &[u8], score: equity::Equity| {
                    let (along, at) = if down { (col, row) } else { (row, col) };
                    let kept = word.iter().filter(|&&tile| tile != 0).count() <= rack_size as usize
                        && lane == along
                        && at >= idx
                        && word.get((at - idx) as usize).is_some_and(|&tile| tile != 0);
                    if kept {
                        found(down, lane, idx, word, score.raw());
                    }
                    kept
                }),
                klv::AdjustLeave::Identity,
                EquityPredicate::AcceptAll,
                false,
            );
            self.working_buffer.jumbled.through = None;
            self.working_buffer.jumbled.walk = walk;
            self.working_buffer.lane_rows = !G::from(0);
            self.working_buffer.lane_cols = !G::from(0);
        }
        self.working_buffer.pool.walked = walked;
        self.plays.clear();
    }

    // one-word rules only: it walks the graph's words, so its caller never
    // gives it a woadwage game; a jumbled woadwage game (as inference gives
    // it) walks the jumbled sheet
    pub fn pool_plays_best_window<N: kwg::Node, L: kwg::Node>(
        &mut self,
        board_snapshot: &BoardSnapshot<'_, N, L>,
        pool: &[u8],
        (rows, cols): (u128, u128),
        window: std::ops::Range<u64>,
        lists: &mut PoolLists,
        mut found: impl FnMut(bool, i8, i8, &[u8], i32),
    ) -> u64 {
        let game_config = board_snapshot.game_config;
        let jumbled = match game_config.game_rules() {
            game_config::GameRules::Classic | game_config::GameRules::Woadwage => false,
            game_config::GameRules::Jumbled | game_config::GameRules::JumbledWoadwage => true,
        };
        if !jumbled {
            return self.pool_plays_window(board_snapshot, pool, (rows, cols), window, None, found);
        }
        let rack_size = game_config.rack_size();
        let mut tally = [0u8; MAX_ALPHABET_LEN];
        let tally = &mut tally[..game_config.alphabet().len() as usize];
        let Some(walked) = self.pool_rack(pool, rack_size, tally) else {
            return 0;
        };
        let params = GenMovesParams {
            board_snapshot,
            rack: &walked,
            max_gen: 1,
            num_exchanges_by_this_player: i16::MAX,
            pass_policy: PassPolicy::OnlyWhenForced,
            dynamic_leaves: None,
        };
        std::mem::swap(&mut self.working_buffer.jumbled, &mut lists.0);
        self.working_buffer.jumbled.held = !self.working_buffer.jumbled.kept.is_empty();
        let walk = self.working_buffer.jumbled.walk;
        self.working_buffer.lane_rows = G::of_u128(rows);
        self.working_buffer.lane_cols = G::of_u128(cols);
        self.working_buffer.jumbled.walk = JUMBLED_BEST_PER_RACK_TILES;
        self.working_buffer.jumbled.window = window;
        self.working_buffer.jumbled.seen = 0;
        self.gen_sheet_impl(
            &params,
            PlacePredicate::Dyn(&mut |down, lane, idx, word: &[u8], score: equity::Equity| {
                if word.iter().filter(|&&tile| tile != 0).count() <= rack_size as usize {
                    found(down, lane, idx, word, score.raw());
                }
                false
            }),
            klv::AdjustLeave::Identity,
            EquityPredicate::RejectAll,
            false,
        );
        let seen = self.working_buffer.jumbled.seen;
        self.working_buffer.jumbled.window = 0..u64::MAX;
        self.working_buffer.jumbled.seen = 0;
        self.working_buffer.jumbled.walk = walk;
        self.working_buffer.jumbled.held = false;
        std::mem::swap(&mut self.working_buffer.jumbled, &mut lists.0);
        self.working_buffer.jumbled.splits.out_of_memory = lists.0.splits.out_of_memory;
        self.working_buffer.pool.walked = walked;
        self.working_buffer.lane_rows = !G::from(0);
        self.working_buffer.lane_cols = !G::from(0);
        seen
    }

    // as pool_plays_best_window, walking at most budget (1 or more) blank
    // choices, from where part says the walk before stopped if the window
    // starts there; part then says where this one stopped
    // one-word rules only: it walks the graph's words, so its caller never
    // gives it a woadwage game
    pub fn pool_plays_best_part<N: kwg::Node, L: kwg::Node>(
        &mut self,
        board_snapshot: &BoardSnapshot<'_, N, L>,
        pool: &[u8],
        PoolWindow { lanes, window }: PoolWindow,
        budget: u64,
        (lists, part): (&mut PoolLists, &mut PoolPart),
        found: impl FnMut(bool, i8, i8, &[u8], i32),
    ) -> u64 {
        let jumbled = &mut lists.0;
        jumbled.part = match part.0 {
            Some((at, from)) if at == window.start => (at, from),
            _ => (0, 0),
        };
        jumbled.budget = budget.max(1);
        jumbled.stopped = None;
        let seen = self.pool_plays_best_window(board_snapshot, pool, lanes, window, lists, found);
        let jumbled = &mut lists.0;
        part.0 = jumbled.stopped.take();
        jumbled.part = (0, 0);
        jumbled.budget = u64::MAX;
        seen
    }

    // one-word rules only: it walks the graph's words, so its caller never
    // gives it a woadwage game
    pub fn pool_plays_top<N: kwg::Node, L: kwg::Node>(
        &mut self,
        board_snapshot: &BoardSnapshot<'_, N, L>,
        pool: &[u8],
        most: usize,
        mut found: impl FnMut(bool, i8, i8, &[u8], i32),
    ) {
        let game_config = board_snapshot.game_config;
        let jumbled = match game_config.game_rules() {
            game_config::GameRules::Classic | game_config::GameRules::Woadwage => false,
            game_config::GameRules::Jumbled | game_config::GameRules::JumbledWoadwage => true,
        };
        if !jumbled {
            self.pool_plays(board_snapshot, pool, found);
            return;
        }
        let rack_size = game_config.rack_size();
        let mut tally = [0u8; MAX_ALPHABET_LEN];
        let tally = &mut tally[..game_config.alphabet().len() as usize];
        let Some(walked) = self.pool_rack(pool, rack_size, tally) else {
            return;
        };
        let params = GenMovesParams {
            board_snapshot,
            rack: &walked,
            max_gen: most,
            num_exchanges_by_this_player: i16::MAX,
            pass_policy: PassPolicy::OnlyWhenForced,
            dynamic_leaves: None,
        };
        let walk = self.working_buffer.jumbled.walk;
        self.working_buffer.jumbled.walk = JUMBLED_ONCE_PER_RACK_TILES;
        self.gen_sheet_impl(
            &params,
            PlacePredicate::Dyn(&mut |down, lane, idx, word: &[u8], score: equity::Equity| {
                let kept = word.iter().filter(|&&tile| tile != 0).count() <= rack_size as usize;
                if kept {
                    found(down, lane, idx, word, score.raw());
                }
                kept
            }),
            klv::AdjustLeave::Identity,
            EquityPredicate::AcceptAll,
            false,
        );
        self.working_buffer.jumbled.walk = walk;
        self.working_buffer.pool.walked = walked;
        self.plays.clear();
    }

    // as pool_plays_top, a window of the lanes' multisets at a time, from the
    // threshold the windows before it reached (floor, carried in and out): the
    // windows of every lane, together, offer every play of the pool's top most
    // one-word rules only: it walks the graph's words, so its caller never
    // gives it a woadwage game
    pub fn pool_plays_top_window<N: kwg::Node, L: kwg::Node>(
        &mut self,
        board_snapshot: &BoardSnapshot<'_, N, L>,
        pool: &[u8],
        PoolWindow {
            lanes: (rows, cols),
            window,
        }: PoolWindow,
        most: usize,
        (lists, floor): (&mut PoolLists, &mut PoolFloor),
        mut found: impl FnMut(bool, i8, i8, &[u8], i32),
    ) -> u64 {
        let game_config = board_snapshot.game_config;
        let jumbled = match game_config.game_rules() {
            game_config::GameRules::Classic | game_config::GameRules::Woadwage => false,
            game_config::GameRules::Jumbled | game_config::GameRules::JumbledWoadwage => true,
        };
        if !jumbled {
            return self.pool_plays_window(board_snapshot, pool, (rows, cols), window, None, found);
        }
        let rack_size = game_config.rack_size();
        let mut tally = [0u8; MAX_ALPHABET_LEN];
        let tally = &mut tally[..game_config.alphabet().len() as usize];
        let Some(walked) = self.pool_rack(pool, rack_size, tally) else {
            return 0;
        };
        let params = GenMovesParams {
            board_snapshot,
            rack: &walked,
            max_gen: most,
            num_exchanges_by_this_player: i16::MAX,
            pass_policy: PassPolicy::OnlyWhenForced,
            dynamic_leaves: None,
        };
        std::mem::swap(&mut self.working_buffer.jumbled, &mut lists.0);
        self.working_buffer.jumbled.held = !self.working_buffer.jumbled.kept.is_empty();
        let walk = self.working_buffer.jumbled.walk;
        self.working_buffer.lane_rows = G::of_u128(rows);
        self.working_buffer.lane_cols = G::of_u128(cols);
        self.working_buffer.jumbled.walk = JUMBLED_ONCE_PER_RACK_TILES;
        self.working_buffer.jumbled.window = window;
        self.working_buffer.jumbled.seen = 0;
        self.working_buffer.jumbled.floor = floor.0;
        self.gen_sheet_impl(
            &params,
            PlacePredicate::Dyn(&mut |down, lane, idx, word: &[u8], score: equity::Equity| {
                let kept = word.iter().filter(|&&tile| tile != 0).count() <= rack_size as usize;
                if kept {
                    found(down, lane, idx, word, score.raw());
                }
                kept
            }),
            klv::AdjustLeave::Identity,
            EquityPredicate::AcceptAll,
            false,
        );
        let seen = self.working_buffer.jumbled.seen;
        self.working_buffer.jumbled.floor = i32::MIN;
        self.working_buffer.jumbled.window = 0..u64::MAX;
        self.working_buffer.jumbled.seen = 0;
        self.working_buffer.jumbled.walk = walk;
        self.working_buffer.jumbled.held = false;
        std::mem::swap(&mut self.working_buffer.jumbled, &mut lists.0);
        self.working_buffer.jumbled.splits.out_of_memory = lists.0.splits.out_of_memory;
        self.working_buffer.pool.walked = walked;
        self.working_buffer.lane_rows = !G::from(0);
        self.working_buffer.lane_cols = !G::from(0);
        if let Some(worst) = (self.plays.iter())
            .filter(|play| matches!(play.play, Play::Place { .. }))
            .nth(most.saturating_sub(1))
        {
            floor.0 = floor.0.max(worst.equity.raw());
        }
        self.plays.clear();
        seen
    }

    // one-word rules only: it walks the graph's words, so its caller never
    // gives it a woadwage game
    pub fn pool_plays_window<N: kwg::Node, L: kwg::Node>(
        &mut self,
        board_snapshot: &BoardSnapshot<'_, N, L>,
        pool: &[u8],
        lanes: (u128, u128),
        window: std::ops::Range<u64>,
        through: Option<i8>,
        found: impl FnMut(bool, i8, i8, &[u8], i32),
    ) -> u64 {
        let jumbled = &mut self.working_buffer.jumbled;
        jumbled.window = window;
        jumbled.through = through;
        jumbled.seen = 0;
        self.pool_plays_lanes(board_snapshot, pool, lanes, found);
        let jumbled = &mut self.working_buffer.jumbled;
        let seen = jumbled.seen;
        jumbled.window = 0..u64::MAX;
        jumbled.through = None;
        jumbled.seen = 0;
        seen
    }

    // one-word rules only: it walks the graph's words, so its caller never
    // gives it a woadwage game
    pub fn pool_plays_lanes<N: kwg::Node, L: kwg::Node>(
        &mut self,
        board_snapshot: &BoardSnapshot<'_, N, L>,
        pool: &[u8],
        (rows, cols): (u128, u128),
        found: impl FnMut(bool, i8, i8, &[u8], i32),
    ) {
        self.working_buffer.lane_rows = G::of_u128(rows);
        self.working_buffer.lane_cols = G::of_u128(cols);
        self.pool_plays(board_snapshot, pool, found);
        self.working_buffer.lane_rows = !G::from(0);
        self.working_buffer.lane_cols = !G::from(0);
    }

    #[inline(always)]
    fn gen_census_sheet_impl<'a, N: kwg::Node, L: kwg::Node>(
        &mut self,
        params: &'a GenMovesParams<'a, N, L>,
        place_move_predicate: PlacePredicate<'_>,
        adjust_leave_value: klv::AdjustLeave,
        equity_predicate: EquityPredicate<'_>,
    ) {
        self.gen_sheet_impl(
            params,
            place_move_predicate,
            adjust_leave_value,
            equity_predicate,
            true,
        );
    }

    #[inline(always)]
    fn gen_sheet_impl<'a, N: kwg::Node, L: kwg::Node>(
        &mut self,
        params: &'a GenMovesParams<'a, N, L>,
        place_move_predicate: PlacePredicate<'_>,
        adjust_leave_value: klv::AdjustLeave,
        equity_predicate: EquityPredicate<'_>,
        is_census: bool,
    ) {
        // a census whose equity predicate rejects every play keeps none, and
        // its predicates take scores alone, so it reads no leave: its walk
        // reads the empty klv (the generator's own, taken out for the walk and
        // put back) and keeps no leaves' table, however many tiles it
        // walks
        let no_leaves = matches!(equity_predicate, EquityPredicate::RejectAll);
        let mut empty_klvs = std::mem::take(&mut self.empty_klvs);
        let empty = (no_leaves && !params.board_snapshot.klv.is_empty())
            .then(|| L::kept_empty_klv(&mut empty_klvs))
            .flatten();
        let snapshot;
        let without_leaves;
        let params = match &empty {
            Some(empty) => {
                let board_snapshot = params.board_snapshot;
                snapshot = BoardSnapshot {
                    board_tiles: board_snapshot.board_tiles,
                    game_config: board_snapshot.game_config,
                    kwg: board_snapshot.kwg,
                    anagrams: board_snapshot.anagrams,
                    rack_lengths: board_snapshot.rack_lengths,
                    klv: empty,
                };
                without_leaves = GenMovesParams {
                    board_snapshot: &snapshot,
                    rack: params.rack,
                    max_gen: params.max_gen,
                    num_exchanges_by_this_player: params.num_exchanges_by_this_player,
                    pass_policy: params.pass_policy,
                    dynamic_leaves: None,
                };
                &without_leaves
            }
            None => params,
        };
        self.working_buffer.is_census = is_census;
        self.gen_moves_filtered_lean_by_side(
            params,
            place_move_predicate,
            adjust_leave_value,
            equity_predicate,
            None,
        );
        self.working_buffer.is_census = false;
        self.empty_klvs = empty_klvs;
    }

    #[inline(always)]
    pub fn gen_moves_filtered<'a, N: kwg::Node, L: kwg::Node>(
        &mut self,
        params: &'a GenMovesParams<'a, N, L>,
        place_move_predicate: PlacePredicate<'_>,
        adjust_leave_value: klv::AdjustLeave,
        equity_predicate: EquityPredicate<'_>,
    ) -> error::Returns<()> {
        check_exchange_limit(params.board_snapshot.game_config)?;
        adjust_leave_value.check_for_klv(params.board_snapshot.klv)?;
        self.gen_moves_filtered_steered(
            params,
            place_move_predicate,
            adjust_leave_value,
            equity_predicate,
            None,
        );
        Ok(())
    }

    // as gen_moves_filtered, the tables' generation steered when steer is some
    #[inline(always)]
    fn gen_moves_filtered_steered<'a, N: kwg::Node, L: kwg::Node>(
        &mut self,
        params: &'a GenMovesParams<'a, N, L>,
        place_move_predicate: PlacePredicate<'_>,
        adjust_leave_value: klv::AdjustLeave,
        equity_predicate: EquityPredicate<'_>,
        steer: Option<&Steer<'_>>,
    ) {
        self.gen_moves_with_leaves(
            params,
            MoveFilters {
                place: place_move_predicate,
                equity: equity_predicate,
            },
            adjust_leave_value,
            steer,
            false,
            None,
        );
    }

    #[inline(always)]
    fn gen_moves_with_leaves<'a, N: kwg::Node, L: kwg::Node>(
        &mut self,
        params: &'a GenMovesParams<'a, N, L>,
        MoveFilters {
            place: place_move_predicate,
            equity: equity_predicate,
        }: MoveFilters<'_, '_>,
        adjust_leave_value: klv::AdjustLeave,
        steer: Option<&Steer<'_>>,
        lean: bool,
        mut other: OtherLeaves<'_, L>,
    ) {
        let handed_back = match params.board_snapshot.anagrams.filter(|_| !lean) {
            Some(anagrams) if anagrams.nibble_side().is_some() => self
                .gen_moves_tables::<15, u16, ByNibble, _, _>(
                    params,
                    place_move_predicate,
                    adjust_leave_value,
                    equity_predicate,
                    steer,
                    other.as_mut().map(|(tables, pair)| (*tables, &mut **pair)),
                ),
            Some(_)
                if {
                    let dim = params.board_snapshot.game_config.board_layout().dim();
                    dim.rows == 15 && dim.cols == 15
                } =>
            {
                self.gen_moves_tables::<15, u16, ByLayout, _, _>(
                    params,
                    place_move_predicate,
                    adjust_leave_value,
                    equity_predicate,
                    steer,
                    other.as_mut().map(|(tables, pair)| (*tables, &mut **pair)),
                )
            }
            Some(_)
                if {
                    let dim = params.board_snapshot.game_config.board_layout().dim();
                    dim.rows == 21 && dim.cols == 21
                } =>
            {
                self.gen_moves_tables::<21, u32, ByLayout, _, _>(
                    params,
                    place_move_predicate,
                    adjust_leave_value,
                    equity_predicate,
                    steer,
                    other.as_mut().map(|(tables, pair)| (*tables, &mut **pair)),
                )
            }
            Some(_) => self.gen_moves_tables::<0, u128, ByLayout, _, _>(
                params,
                place_move_predicate,
                adjust_leave_value,
                equity_predicate,
                steer,
                other.as_mut().map(|(tables, pair)| (*tables, &mut **pair)),
            ),
            None => Some((place_move_predicate, equity_predicate)),
        };
        if let Some((place_move_predicate, equity_predicate)) = handed_back {
            self.gen_moves_filtered_lean_by_side(
                params,
                place_move_predicate,
                adjust_leave_value,
                equity_predicate,
                other,
            );
        }
    }

    // the generation by the tables on the generator's own working state, keyed
    // by nibbles for its instance (a classic game on a square board of side D,
    // u16 lane sets), else by its tables' layout (any board). A turn the tables
    // do not play (a census sheet, a game its tables do not fit, a rack its
    // subracks do not hold, or a rack of one tile, which the lean arm plays
    // from the cross sets) is handed back before anything is generated, with
    // the predicates, for the lean arm.
    // the best one kept in one slot, more of them in a heap
    #[inline(always)]
    fn gen_moves_tables<
        'a,
        'p,
        'e,
        const D: usize,
        W: LaneBits,
        T: TablePlaces,
        N: kwg::Node,
        L: kwg::Node,
    >(
        &mut self,
        params: &'a GenMovesParams<'a, N, L>,
        place_move_predicate: PlacePredicate<'p>,
        adjust_leave_value: klv::AdjustLeave,
        equity_predicate: EquityPredicate<'e>,
        steer: Option<&Steer<'_>>,
        other: OtherLeaves<'_, L>,
    ) -> Option<(PlacePredicate<'p>, EquityPredicate<'e>)> {
        if params.max_gen == 1 {
            self.gen_moves_tables_kept::<KeptOne, D, W, T, N, L>(
                params,
                place_move_predicate,
                adjust_leave_value,
                equity_predicate,
                steer,
                other,
            )
        } else {
            self.gen_moves_tables_kept::<KeptHeap, D, W, T, N, L>(
                params,
                place_move_predicate,
                adjust_leave_value,
                equity_predicate,
                steer,
                other,
            )
        }
    }

    #[inline]
    fn gen_moves_tables_kept<
        'a,
        'p,
        'e,
        K: Kept,
        const D: usize,
        W: LaneBits,
        T: TablePlaces,
        N: kwg::Node,
        L: kwg::Node,
    >(
        &mut self,
        params: &'a GenMovesParams<'a, N, L>,
        mut place_move_predicate: PlacePredicate<'p>,
        adjust_leave_value: klv::AdjustLeave,
        equity_predicate: EquityPredicate<'e>,
        steer: Option<&Steer<'_>>,
        mut other: OtherLeaves<'_, L>,
    ) -> Option<(PlacePredicate<'p>, EquityPredicate<'e>)> {
        if self.working_buffer.is_census || params.rack.len() == 1 {
            return Some((place_move_predicate, equity_predicate));
        }
        self.plays.clear();
        if params.max_gen == 0 {
            return None;
        }

        let alphabet = params.board_snapshot.game_config.alphabet();
        let board_layout = params.board_snapshot.game_config.board_layout();
        let dim = board_layout.dim();

        // nibble keys: a classic game on a square board of the instance's side,
        // its tables built for that side, checked before init, which is
        // compiled for the side
        if T::NIBBLES
            && !(dim.rows as usize == D
                && dim.cols as usize == D
                && match params.board_snapshot.game_config.game_rules() {
                    game_config::GameRules::Classic => true,
                    game_config::GameRules::Jumbled
                    | game_config::GameRules::JumbledWoadwage
                    | game_config::GameRules::Woadwage => false,
                }
                && params
                    .board_snapshot
                    .anagrams
                    .and_then(anagrams::Anagrams::nibble_side)
                    == Some(D as u8))
        {
            return Some((place_move_predicate, equity_predicate));
        }
        let working_buffer = &mut self.working_buffer;
        working_buffer.init::<D, true, T, _, _>(
            params.board_snapshot,
            params.rack,
            adjust_leave_value,
            params.dynamic_leaves,
            working_buffer.multi_leaves.dense_most(),
            other.as_mut().map(|(tables, pair)| (*tables, &mut **pair)),
        );
        let classic = if T::NIBBLES {
            working_buffer.anagrams_fit_config && !working_buffer.subracks.is_empty()
        } else {
            working_buffer.turn_is_supported(params.board_snapshot)
        };
        if !classic {
            return Some((place_move_predicate, equity_predicate));
        }

        let threshold = std::cell::Cell::new(equity::Equity::NEG_INFINITY);

        let multi_leaves = std::mem::take(&mut working_buffer.multi_leaves);
        let num_tiles_on_board = working_buffer.num_tiles_on_board;

        if other.is_none()
            && let Some(floor) =
                sure_floor(params, working_buffer, &multi_leaves, &equity_predicate)
        {
            threshold.set(floor);
        }

        let mut selected = CappedMoves {
            kept: K::from_vec(std::mem::take(&mut self.plays)),
            predicate: equity_predicate,
            pair: other.map(|(_, pair)| pair),
            threshold: &threshold,
            max_gen: params.max_gen,
        };
        let found_place_move = |down: bool,
                                lane: i8,
                                idx: i8,
                                word: &[u8],
                                score: equity::Equity,
                                leave_value: equity::Equity| {
            if place_move_predicate.test(down, lane, idx, word, score) {
                let other_adjustments = if num_tiles_on_board == 0 {
                    (idx..)
                        .zip(word)
                        .filter(|&(ref i, &tile)| {
                            tile != 0
                                && alphabet.is_vowel(tile)
                                && if down {
                                    board_layout.danger_stars(true)
                                        [usize::from((*i).cast_unsigned())]
                                } else {
                                    board_layout.danger_stars(false)
                                        [usize::from((*i).cast_unsigned())]
                                }
                        })
                        .count() as i32
                        * -equity::OPENING_HOTSPOT_PENALTY
                } else {
                    0
                };
                selected.offer(
                    &multi_leaves,
                    equity::Equity::new(score.raw() + other_adjustments),
                    leave_value,
                    word.iter().copied().filter(|&tile| tile != 0),
                    || Play::Place {
                        down,
                        lane,
                        idx,
                        word: word.into(),
                        score,
                    },
                );
            }
            threshold.get()
        };
        let lanes_before = (working_buffer.lane_rows, working_buffer.lane_cols);
        if let Some(steer) = steer
            && steer.census.nibbles == T::NIBBLES
            && working_buffer.rack_tally[0] == 0
            && working_buffer.num_tiles_in_bag > 0
            && multi_leaves.is_dense()
        {
            let rows = dim.rows as usize;
            let num_lanes = steer.census.lanes;
            let mut bounds = [NO_PLAY; 256];
            let bound = &mut bounds[..num_lanes];
            for subrack in working_buffer.subracks.iter() {
                if subrack.num_played != 0
                    && let Some(best) = steer.census.lanes_of(subrack.key)
                {
                    let leave = subrack.leave_value;
                    for (b, &score) in bound.iter_mut().zip(best) {
                        *b = (*b).max(score + leave);
                    }
                }
            }
            let (allowed_rows, allowed_cols) = lanes_before;
            let untouched = |lane: usize| {
                if lane < rows {
                    allowed_rows & (G::from(1) << lane as u32) != G::from(0)
                        && (steer.touched_rows >> lane) & 1 == 0
                } else {
                    let col = lane - rows;
                    allowed_cols & (G::from(1) << col as u32) != G::from(0)
                        && (steer.touched_cols >> col) & 1 == 0
                }
            };
            let top = (0..num_lanes)
                .filter(|&lane| untouched(lane))
                .map(|lane| bound[lane])
                .max()
                .unwrap_or(NO_PLAY);
            let mut keep_rows = G::of_u128(steer.touched_rows);
            let mut keep_cols = G::of_u128(steer.touched_cols);
            if top > NO_PLAY / 2 {
                for lane in (0..num_lanes).filter(|&lane| bound[lane] == top && untouched(lane)) {
                    if lane < rows {
                        keep_rows |= G::from(1) << lane as u32;
                    } else {
                        keep_cols |= G::from(1) << (lane - rows) as u32;
                    }
                }
            }
            working_buffer.lane_rows = allowed_rows & keep_rows;
            working_buffer.lane_cols = allowed_cols & keep_cols;
        }
        for _ in kurnia_gen_place_moves_iter::<D, W, T, _, _, _, _, _, _, _, _>(KurniaIterParams {
            runs: (),
            board_snapshot: params.board_snapshot,
            working_buffer: &mut *working_buffer,
            multi_leaves: &multi_leaves,
            found_place_move,
            can_accept: |best_possible_equity: equity::Equity| {
                threshold.get() <= best_possible_equity
            },
            current_threshold: || threshold.get(),
        }) {}
        (working_buffer.lane_rows, working_buffer.lane_cols) = lanes_before;
        self.plays = selected.finish(params, working_buffer, &multi_leaves);
        working_buffer.multi_leaves = multi_leaves;
        None
    }

    #[inline(always)]
    pub fn gen_moves_filtered_lean<'a, N: kwg::Node, L: kwg::Node>(
        &mut self,
        params: &'a GenMovesParams<'a, N, L>,
        place_move_predicate: PlacePredicate<'_>,
        adjust_leave_value: klv::AdjustLeave,
        equity_predicate: EquityPredicate<'_>,
    ) -> error::Returns<()> {
        check_exchange_limit(params.board_snapshot.game_config)?;
        adjust_leave_value.check_for_klv(params.board_snapshot.klv)?;
        self.gen_moves_filtered_lean_by_side(
            params,
            place_move_predicate,
            adjust_leave_value,
            equity_predicate,
            None,
        );
        Ok(())
    }

    // the lean arm's generation compiled for the board's side: a 15 by 15
    // board, a 21 by 21 one, else any
    #[inline(always)]
    fn gen_moves_filtered_lean_by_side<'a, N: kwg::Node, L: kwg::Node>(
        &mut self,
        params: &'a GenMovesParams<'a, N, L>,
        place_move_predicate: PlacePredicate<'_>,
        adjust_leave_value: klv::AdjustLeave,
        equity_predicate: EquityPredicate<'_>,
        other: OtherLeaves<'_, L>,
    ) {
        let dim = params.board_snapshot.game_config.board_layout().dim();
        if dim.rows == 15 && dim.cols == 15 {
            return self.gen_moves_filtered_impl::<15, u32, _, _>(
                params,
                place_move_predicate,
                adjust_leave_value,
                equity_predicate,
                other,
            );
        }
        if dim.rows == 21 && dim.cols == 21 {
            return self.gen_moves_filtered_impl::<21, u32, _, _>(
                params,
                place_move_predicate,
                adjust_leave_value,
                equity_predicate,
                other,
            );
        }
        // any other side, by the copy for any side in G; a generator with none
        // (the wasm's) refused this board at try_new
        if !ANY_SIDE {
            unreachable!("a board with no copy of the lean arm in this generator");
        }
        self.gen_moves_filtered_impl::<0, G, _, _>(
            params,
            place_move_predicate,
            adjust_leave_value,
            equity_predicate,
            other,
        )
    }

    // the best one kept in one slot, more of them in a heap
    #[inline(always)]
    fn gen_moves_filtered_impl<'a, const D: usize, W: LaneBits, N: kwg::Node, L: kwg::Node>(
        &mut self,
        params: &'a GenMovesParams<'a, N, L>,
        place_move_predicate: PlacePredicate<'_>,
        adjust_leave_value: klv::AdjustLeave,
        equity_predicate: EquityPredicate<'_>,
        other: OtherLeaves<'_, L>,
    ) {
        if params.max_gen == 1 {
            self.gen_moves_filtered_impl_kept::<KeptOne, D, W, N, L>(
                params,
                place_move_predicate,
                adjust_leave_value,
                equity_predicate,
                other,
            )
        } else {
            self.gen_moves_filtered_impl_kept::<KeptHeap, D, W, N, L>(
                params,
                place_move_predicate,
                adjust_leave_value,
                equity_predicate,
                other,
            )
        }
    }

    #[inline]
    fn gen_moves_filtered_impl_kept<
        'a,
        K: Kept,
        const D: usize,
        W: LaneBits,
        N: kwg::Node,
        L: kwg::Node,
    >(
        &mut self,
        params: &'a GenMovesParams<'a, N, L>,
        mut place_move_predicate: PlacePredicate<'_>,
        adjust_leave_value: klv::AdjustLeave,
        equity_predicate: EquityPredicate<'_>,
        mut other: OtherLeaves<'_, L>,
    ) {
        self.plays.clear();
        if params.max_gen == 0 {
            return;
        }

        let threshold = std::cell::Cell::new(equity::Equity::NEG_INFINITY);

        let working_buffer = &mut self.working_buffer;
        let dense_most = if matches!(equity_predicate, EquityPredicate::RejectAll) {
            0
        } else {
            working_buffer.multi_leaves.dense_most()
        };
        working_buffer.init::<D, true, ByLayout, _, _>(
            params.board_snapshot,
            params.rack,
            adjust_leave_value,
            params.dynamic_leaves,
            dense_most,
            other.as_mut().map(|(tables, pair)| (*tables, &mut **pair)),
        );
        let multi_leaves = std::mem::take(&mut working_buffer.multi_leaves);
        if other.is_none()
            && let Some(floor) =
                sure_floor(params, working_buffer, &multi_leaves, &equity_predicate)
        {
            threshold.set(floor);
        }
        threshold.set(
            threshold
                .get()
                .max(equity::Equity::new(working_buffer.jumbled.floor)),
        );

        let mut selected = CappedMoves {
            kept: K::from_vec(std::mem::take(&mut self.plays)),
            predicate: equity_predicate,
            pair: other.map(|(_, pair)| pair),
            threshold: &threshold,
            max_gen: params.max_gen,
        };
        let found_place_move = |down: bool,
                                lane: i8,
                                idx: i8,
                                word: &[u8],
                                score: equity::Equity,
                                leave_value: equity::Equity| {
            if place_move_predicate.test(down, lane, idx, word, score) {
                selected.offer(
                    &multi_leaves,
                    score,
                    leave_value,
                    word.iter().copied().filter(|&tile| tile != 0),
                    || Play::Place {
                        down,
                        lane,
                        idx,
                        word: word.into(),
                        score,
                    },
                );
            }
            threshold.get()
        };
        let can_accept =
            |best_possible_equity: equity::Equity| threshold.get() <= best_possible_equity;
        let current_threshold = || threshold.get();
        // with the run letters the lean arm's table keeps, the walk goes
        // straight to a tile's node; a generator that never jumps (the wasm's,
        // whose table keeps none) has no walk that does
        let runs = if JUMPS {
            params
                .board_snapshot
                .rack_lengths
                .and_then(anagrams::RackLengths::run_letter_bits)
        } else {
            None
        };
        let empty = working_buffer.num_tiles_on_board == 0;
        match runs {
            _ if working_buffer.num_tiles_on_rack == 1 => {
                lean_cross_sets::<D, _, _, _, _, _>(params.board_snapshot, working_buffer);
                kurnia_gen_one_tile_moves(
                    params.board_snapshot,
                    working_buffer,
                    &multi_leaves,
                    found_place_move,
                );
            }
            Some(runs) if empty => gen_empty_board_place_moves::<D, _, _, _, _, _, _, _>(
                runs,
                params.board_snapshot,
                working_buffer,
                &multi_leaves,
                current_threshold().raw(),
                found_place_move,
            ),
            None if empty => gen_empty_board_place_moves::<D, _, _, _, _, _, _, _>(
                (),
                params.board_snapshot,
                working_buffer,
                &multi_leaves,
                current_threshold().raw(),
                found_place_move,
            ),
            Some(runs) => {
                for _ in kurnia_gen_place_moves_iter_lean::<D, W, _, _, _, _, _, _, _, _, _>(
                    KurniaIterParams {
                        runs,
                        board_snapshot: params.board_snapshot,
                        working_buffer,
                        multi_leaves: &multi_leaves,
                        found_place_move,
                        can_accept,
                        current_threshold,
                    },
                ) {}
            }
            None => {
                for _ in kurnia_gen_place_moves_iter_lean::<D, W, _, _, _, _, _, _, _, _, _>(
                    KurniaIterParams {
                        runs: (),
                        board_snapshot: params.board_snapshot,
                        working_buffer,
                        multi_leaves: &multi_leaves,
                        found_place_move,
                        can_accept,
                        current_threshold,
                    },
                ) {}
            }
        }
        self.plays = selected.finish(params, working_buffer, &multi_leaves);
        working_buffer.multi_leaves = multi_leaves;
    }

    // as gen_moves_unfiltered, the place moves on the rows and columns in the
    // masks only (bit i, row or column i), the exchanges and a forced pass as
    // ever: the moves a change to the board can touch, when the rest are known
    #[inline(always)]
    pub fn gen_moves_unfiltered_lanes<'a, N: kwg::Node, L: kwg::Node>(
        &mut self,
        params: &'a GenMovesParams<'a, N, L>,
        rows: u128,
        cols: u128,
    ) -> error::Returns<()> {
        self.working_buffer.lane_rows = G::of_u128(rows);
        self.working_buffer.lane_cols = G::of_u128(cols);
        let generated = self.gen_moves_unfiltered(params);
        self.working_buffer.lane_rows = !G::from(0);
        self.working_buffer.lane_cols = !G::from(0);
        generated
    }

    // as gen_moves_unfiltered_lanes, steered by a census of the board (Steer)
    #[inline(always)]
    pub fn gen_moves_unfiltered_lanes_steered<'a, N: kwg::Node, L: kwg::Node>(
        &mut self,
        params: &'a GenMovesParams<'a, N, L>,
        rows: u128,
        cols: u128,
        steer: &Steer<'_>,
    ) -> error::Returns<()> {
        check_exchange_limit(params.board_snapshot.game_config)?;
        self.working_buffer.lane_rows = G::of_u128(rows);
        self.working_buffer.lane_cols = G::of_u128(cols);
        self.gen_moves_filtered_steered(
            params,
            PlacePredicate::AcceptAll,
            klv::AdjustLeave::Identity,
            EquityPredicate::AcceptAll,
            Some(steer),
        );
        self.working_buffer.lane_rows = !G::from(0);
        self.working_buffer.lane_cols = !G::from(0);
        Ok(())
    }

    // as gen_moves_unfiltered_lanes, by the lean arm
    #[inline(always)]
    pub fn gen_moves_unfiltered_lanes_lean<'a, N: kwg::Node, L: kwg::Node>(
        &mut self,
        params: &'a GenMovesParams<'a, N, L>,
        rows: u128,
        cols: u128,
    ) -> error::Returns<()> {
        self.working_buffer.lane_rows = G::of_u128(rows);
        self.working_buffer.lane_cols = G::of_u128(cols);
        let generated = self.gen_moves_unfiltered_lean(params);
        self.working_buffer.lane_rows = !G::from(0);
        self.working_buffer.lane_cols = !G::from(0);
        generated
    }

    #[inline(always)]
    pub fn gen_moves_unfiltered<'a, N: kwg::Node, L: kwg::Node>(
        &mut self,
        params: &'a GenMovesParams<'a, N, L>,
    ) -> error::Returns<()> {
        check_exchange_limit(params.board_snapshot.game_config)?;
        self.gen_moves_filtered_steered(
            params,
            PlacePredicate::AcceptAll,
            klv::AdjustLeave::Identity,
            EquityPredicate::AcceptAll,
            None,
        );
        Ok(())
    }

    #[inline(always)]
    pub fn gen_moves_unfiltered_lean<'a, N: kwg::Node, L: kwg::Node>(
        &mut self,
        params: &'a GenMovesParams<'a, N, L>,
    ) -> error::Returns<()> {
        check_exchange_limit(params.board_snapshot.game_config)?;
        self.gen_moves_filtered_lean_by_side(
            params,
            PlacePredicate::AcceptAll,
            klv::AdjustLeave::Identity,
            EquityPredicate::AcceptAll,
            None,
        );
        Ok(())
    }

    // generate both tables' best plays on one board and rack with top-one
    // parameters. The caller can reuse the scratch between searches.
    pub fn gen_best_moves_for_two_leaves<N: kwg::Node, L: kwg::Node>(
        &mut self,
        params: &GenMovesParams<'_, N, L>,
        tables: klv::LeavePair<'_, L>,
        pair: &mut PairedHasty,
        lean: bool,
    ) -> error::Returns<[Play; 2]> {
        check_exchange_limit(params.board_snapshot.game_config)?;
        assert!(params.max_gen == 1 && params.dynamic_leaves.is_none());
        let board = params.board_snapshot;
        assert!(std::ptr::eq(board.klv, tables.table(0)));
        let other_klv = tables.table(1);
        // sharing needs a dense rack and ordinary leave lookups.
        if BAG_TERM
            || 1u32
                .checked_shl(params.rack.len() as u32)
                .is_none_or(|limit| limit > self.working_buffer.multi_leaves.dense_most())
            || board.klv.racks().is_some()
            || other_klv.racks().is_some()
        {
            let mut best = [None, None];
            for (out, klv) in best.iter_mut().zip([board.klv, other_klv]) {
                let snapshot = BoardSnapshot { klv, ..*board };
                let params = GenMovesParams {
                    board_snapshot: &snapshot,
                    ..*params
                };
                self.gen_moves_with_leaves(
                    &params,
                    MoveFilters {
                        place: PlacePredicate::AcceptAll,
                        equity: EquityPredicate::AcceptAll,
                    },
                    klv::AdjustLeave::Identity,
                    None,
                    lean,
                    None,
                );
                *out = self.plays.pop().map(|valued| valued.play);
            }
            return Ok(best.map(Option::unwrap));
        }

        pair.values[0].clear();
        pair.best = [None, None];
        self.gen_moves_with_leaves(
            params,
            MoveFilters {
                place: PlacePredicate::AcceptAll,
                equity: EquityPredicate::AcceptAll,
            },
            klv::AdjustLeave::Identity,
            None,
            lean,
            Some((tables, pair)),
        );
        Ok(std::mem::take(&mut pair.best).map(|best| best.unwrap().play))
    }

    // each word the board can still take, whatever the racks and the bag hold:
    // found_word is called once for each, in ascending order
    // it prunes to the graph's words, one word a play; a woadwage or jumbled
    // woadwage game's (a simmer's, an endgame's) it prunes to the graph's
    // words some span of some line can take, as a play's two words may split
    // anywhere
    #[inline(always)]
    pub fn gen_remaining_words<'a, FoundWord: 'a + FnMut(&[u8]), N: kwg::Node, L: kwg::Node>(
        &mut self,
        board_snapshot: &'a BoardSnapshot<'a, N, L>,
        found_word: FoundWord,
    ) -> error::Returns<()> {
        self.gen_remaining_words_within(board_snapshot, None, found_word)
    }

    // the buffers the word prune keeps from one prune to the next let go, for
    // a generator lent to others whose prunes are far apart
    #[inline]
    pub fn let_go_of_prune(&mut self) {
        self.working_buffer.prune = PruneBuffers::default();
    }

    // gen_remaining_words, every growth of the prune's own taken only within
    // memory, if given: refused, found_word is never called. Under a limit the
    // prune keeps nothing past itself, so what earlier prunes kept goes first,
    // and what this one held goes once it ends
    #[inline(always)]
    pub fn gen_remaining_words_within<
        'a,
        FoundWord: 'a + FnMut(&[u8]),
        N: kwg::Node,
        L: kwg::Node,
    >(
        &mut self,
        board_snapshot: &'a BoardSnapshot<'a, N, L>,
        memory: Option<&endgame::MemoryLimit>,
        found_word: FoundWord,
    ) -> error::Returns<()> {
        let working_buffer = &mut self.working_buffer;
        if memory.is_some() {
            working_buffer.prune = PruneBuffers::default();
        }
        working_buffer.init::<0, true, ByLayout, _, _>(
            board_snapshot,
            &[],
            klv::AdjustLeave::Identity,
            None,
            working_buffer.multi_leaves.dense_most(),
            None,
        );
        working_buffer.prev_board_tiles.fill(0xff);
        let pruned = gen_remaining_words(board_snapshot, working_buffer, memory, found_word);
        if memory.is_some() {
            working_buffer.prune = PruneBuffers::default();
        }
        pruned
    }

    // the letters each square of the board takes, the cross sets a generation
    // reads: for each square, row by row, those that make the run of tiles down
    // through it a word, then those that make the run across one, a bit (1 <<
    // tile) for each, and bit 0 when a tile is beside the square that way; 0
    // when none is. A square of the board's own tiles is given too, and means
    // nothing
    pub fn gen_hooks<N: kwg::Node, L: kwg::Node>(
        &mut self,
        board_snapshot: &BoardSnapshot<'_, N, L>,
        mut found: impl FnMut(usize, u64, u64),
    ) {
        let working_buffer = &mut self.working_buffer;
        working_buffer.init::<0, true, ByLayout, _, _>(
            board_snapshot,
            &[],
            klv::AdjustLeave::Identity,
            None,
            working_buffer.multi_leaves.dense_most(),
            None,
        );
        let dim = board_snapshot.game_config.board_layout().dim();
        let transposed_dim = matrix::Dim {
            rows: dim.cols,
            cols: dim.rows,
        };
        for col in 0..dim.cols {
            let strip_range_start = (col as isize * dim.rows as isize) as usize;
            let strip_range_end = strip_range_start + dim.rows as usize;
            gen_cross_set::<NoTables, _, _>(
                board_snapshot,
                &working_buffer.transposed_board_tiles[strip_range_start..strip_range_end],
                CrossSetOutput {
                    allowed: &mut working_buffer.allowed_for_across_plays,
                    cross_scores: &mut working_buffer.cross_score_for_across_plays,
                    output_strider: dim.down(col),
                },
                &mut working_buffer.cross_set_buffer_for_across_plays
                    [strip_range_start..strip_range_end],
                &mut working_buffer.cached_cross_set_for_across_plays
                    [strip_range_start..strip_range_end],
                &mut working_buffer.used_letters_tally,
                &mut working_buffer.woadwage.stack,
            );
        }
        for row in 0..dim.rows {
            let strip_range_start = (row as isize * dim.cols as isize) as usize;
            let strip_range_end = strip_range_start + dim.cols as usize;
            gen_cross_set::<NoTables, _, _>(
                board_snapshot,
                &board_snapshot.board_tiles[strip_range_start..strip_range_end],
                CrossSetOutput {
                    allowed: &mut working_buffer.allowed_for_down_plays,
                    cross_scores: &mut working_buffer.cross_score_for_down_plays,
                    output_strider: transposed_dim.down(row),
                },
                &mut working_buffer.cross_set_buffer_for_down_plays
                    [strip_range_start..strip_range_end],
                &mut working_buffer.cached_cross_set_for_down_plays
                    [strip_range_start..strip_range_end],
                &mut working_buffer.used_letters_tally,
                &mut working_buffer.woadwage.stack,
            );
        }
        // a square no cross word reaches has bit 0 clear, and gives 0
        let hooked = |bits: u64| bits & (bits & 1).wrapping_neg();
        for row in 0..dim.rows {
            for col in 0..dim.cols {
                found(
                    dim.at_on_board(row, col),
                    hooked(working_buffer.allowed_for_across_plays[dim.at_on_board(row, col)]),
                    hooked(
                        working_buffer.allowed_for_down_plays[transposed_dim.at_on_board(col, row)],
                    ),
                );
            }
        }
        working_buffer.prev_board_tiles.fill(0xff);
    }
}

// a shared top-one walk prunes bounds below both seats' best equities.
#[derive(Default)]
pub struct PairedHasty {
    alternate: klv::MultiLeaves,
    values: [Vec<i32>; 2],
    best: [Option<ValuedMove>; 2],
}

struct MoveFilters<'p, 'e> {
    place: PlacePredicate<'p>,
    equity: EquityPredicate<'e>,
}

type OtherLeaves<'a, L> = Option<(klv::LeavePair<'a, L>, &'a mut PairedHasty)>;

impl PairedHasty {
    #[inline]
    fn offer(
        &mut self,
        threshold: &std::cell::Cell<equity::Equity>,
        leaves: &klv::MultiLeaves,
        score: i32,
        bounded_leave: i32,
        tiles: impl Iterator<Item = u8>,
        make_play: impl FnOnce() -> Play,
    ) {
        if score + bounded_leave < threshold.get().raw() {
            return;
        }
        let values = if self.values[0].is_empty() {
            [bounded_leave; 2]
        } else {
            let index = tiles.fold(leaves.pass_leave_idx(), |index, tile| {
                index - leaves.place_value(tile & !((tile as i8 >> 7) as u8))
            });
            // the lean arm's offered leave includes both seats' penalty.
            let adjustment = bounded_leave - leaves.leave_value(index).raw();
            [
                self.values[0][index as usize] + adjustment,
                self.values[1][index as usize] + adjustment,
            ]
        };
        let equities = values.map(|value| equity::Equity::new(score + value));
        if self
            .best
            .iter()
            .zip(equities)
            .all(|(best, equity)| best.as_ref().is_some_and(|best| best.equity > equity))
        {
            return;
        }
        let play = make_play();
        for (best, equity) in self.best.iter_mut().zip(equities) {
            if best.as_ref().is_none_or(|best| {
                equity > best.equity
                    || (equity == best.equity && cmp_play(&play, &best.play).is_lt())
            }) {
                *best = Some(ValuedMove {
                    equity,
                    play: play.clone(),
                });
            }
        }
        if let [Some(first), Some(second)] = &self.best {
            threshold.set(first.equity.min(second.equity));
        }
    }
}

// the moves a capped generation keeps: in a heap, the worst on top; or, for the
// best one alone, in one slot, with no heap work. Either is made from the
// generator's list, so its room is reused, and gives the moves back sorted
trait Kept {
    fn from_vec(plays: Vec<ValuedMove>) -> Self;
    fn len(&self) -> usize;
    #[inline(always)]
    fn is_empty(&self) -> bool {
        self.len() == 0
    }
    fn push(&mut self, candidate: ValuedMove) -> bool;
    fn refused(&self) -> bool;
    // the candidate in place of the worst kept, false if it is no better
    fn replace_worst(&mut self, candidate: ValuedMove) -> bool;
    fn worst_equity(&self) -> equity::Equity;
    fn into_sorted_vec(self) -> Vec<ValuedMove>;
}

struct KeptHeap(std::collections::BinaryHeap<ValuedMove>, bool);

impl Kept for KeptHeap {
    #[inline(always)]
    fn from_vec(plays: Vec<ValuedMove>) -> Self {
        Self(std::collections::BinaryHeap::from(plays), false)
    }

    #[inline(always)]
    fn len(&self) -> usize {
        self.0.len()
    }

    #[inline(always)]
    fn push(&mut self, candidate: ValuedMove) -> bool {
        if self.0.try_reserve(1).is_err() {
            self.1 = true;
            return false;
        }
        self.0.push(candidate);
        true
    }

    #[inline(always)]
    fn refused(&self) -> bool {
        self.1
    }

    #[inline(always)]
    fn replace_worst(&mut self, candidate: ValuedMove) -> bool {
        let mut worst = self.0.peek_mut().unwrap();
        if candidate >= *worst {
            return false;
        }
        *worst = candidate;
        true
    }

    #[inline(always)]
    fn worst_equity(&self) -> equity::Equity {
        self.0.peek().unwrap().equity
    }

    #[inline(always)]
    fn into_sorted_vec(self) -> Vec<ValuedMove> {
        self.0.into_sorted_vec()
    }
}

// for a cap of one: the best so far, and the list it gives back
struct KeptOne {
    best: Option<ValuedMove>,
    plays: Vec<ValuedMove>,
}

impl Kept for KeptOne {
    #[inline(always)]
    fn from_vec(mut plays: Vec<ValuedMove>) -> Self {
        plays.clear();
        Self { best: None, plays }
    }

    #[inline(always)]
    fn len(&self) -> usize {
        usize::from(self.best.is_some())
    }

    #[inline(always)]
    fn push(&mut self, candidate: ValuedMove) -> bool {
        debug_assert!(self.best.is_none());
        self.best = Some(candidate);
        true
    }

    #[inline(always)]
    fn refused(&self) -> bool {
        false
    }

    #[inline(always)]
    fn replace_worst(&mut self, candidate: ValuedMove) -> bool {
        let best = self.best.as_mut().unwrap();
        if candidate >= *best {
            return false;
        }
        *best = candidate;
        true
    }

    #[inline(always)]
    fn worst_equity(&self) -> equity::Equity {
        self.best.as_ref().unwrap().equity
    }

    #[inline(always)]
    fn into_sorted_vec(mut self) -> Vec<ValuedMove> {
        self.plays.extend(self.best);
        self.plays
    }
}

// a move offered to what a capped generation keeps: one worse than the
// threshold, once the cap is reached, is turned away before it is made. Once
// what is kept is refused its room, the threshold is the infinite one that
// stops the walk (stops_walk), and every later move is turned away
#[inline(always)]
fn push_kept<K: Kept, F: FnOnce() -> Play>(
    found_moves: &mut K,
    equity_pred: &mut EquityPredicate<'_>,
    threshold: &std::cell::Cell<equity::Equity>,
    max_gen: usize,
    equity: equity::Equity,
    construct_play: F,
) {
    if threshold.get() > equity && (found_moves.len() >= max_gen || found_moves.refused()) {
        return;
    }
    let play = construct_play();
    if equity_pred.test(equity, &play) {
        let candidate = ValuedMove { equity, play };
        if found_moves.len() >= max_gen {
            if !found_moves.replace_worst(candidate) {
                return;
            }
        } else if !found_moves.push(candidate) {
            threshold.set(equity::Equity::INFINITY);
            return;
        }
        if found_moves.len() >= max_gen {
            threshold.set(found_moves.worst_equity());
        }
    }
}

struct CappedMoves<'a, 'p, K> {
    kept: K,
    predicate: EquityPredicate<'p>,
    pair: Option<&'a mut PairedHasty>,
    threshold: &'a std::cell::Cell<equity::Equity>,
    max_gen: usize,
}

impl<K: Kept> CappedMoves<'_, '_, K> {
    #[inline(always)]
    fn offer(
        &mut self,
        leaves: &klv::MultiLeaves,
        score: equity::Equity,
        leave: equity::Equity,
        tiles: impl Iterator<Item = u8>,
        make_play: impl FnOnce() -> Play,
    ) {
        if let Some(pair) = self.pair.as_deref_mut() {
            pair.offer(
                self.threshold,
                leaves,
                score.raw(),
                leave.raw(),
                tiles,
                make_play,
            );
        } else {
            push_kept(
                &mut self.kept,
                &mut self.predicate,
                self.threshold,
                self.max_gen,
                equity::Equity::new(score.raw() + leave.raw()),
                make_play,
            );
        }
    }

    #[inline]
    fn finish<N: kwg::Node, L: kwg::Node, G: LaneBits, const BAG_TERM: bool, S: BoardStore>(
        mut self,
        params: &GenMovesParams<'_, N, L>,
        buffer: &mut WorkingBuffer<G, BAG_TERM, S>,
        leaves: &klv::MultiLeaves,
    ) -> Vec<ValuedMove> {
        let board = params.board_snapshot;
        if self.threshold.get()
            <= best_exchange_equity::<BAG_TERM>(
                board.game_config,
                &buffer.best_leave_values,
                leaves,
            )
        {
            kurnia_gen_exchange_moves(
                board,
                buffer,
                leaves,
                params.num_exchanges_by_this_player,
                |tiles, leave| {
                    self.offer(
                        leaves,
                        equity::Equity::new(0),
                        leave,
                        tiles.iter().copied(),
                        || Play::Exchange {
                            tiles: tiles.into(),
                        },
                    );
                },
            );
        }
        let empty = self
            .pair
            .as_ref()
            .map_or_else(|| self.kept.is_empty(), |pair| pair.best[0].is_none());
        if params.pass_policy.emits_unconditionally() || empty {
            let leave = if leaves.is_dense() {
                leaves.pass_leave_value()
            } else {
                board.klv.leave_value_from_tally(&buffer.rack_tally)
            };
            self.offer(
                leaves,
                equity::Equity::new(0),
                leave,
                std::iter::empty(),
                || Play::Exchange {
                    tiles: (&buffer.exchange_buffer[..]).into(),
                },
            );
        }
        buffer.jumbled.splits.out_of_memory |= self.kept.refused();
        self.kept.into_sorted_vec()
    }
}

// the max_gen-th best of the plays sure to be pushed, when every play is taken
// and there are that many: for each count exchanged, the best exchange of that
// many, its value exact (best_leave_values[k] is a real leave of the rack's
// tiles but k, the k exchanged; the exchanges generated find it), and the pass
// when it is pushed unconditionally (else a worse play takes its place); a walk
// can start from it
#[inline(always)]
fn sure_floor<N: kwg::Node, L: kwg::Node, G: LaneBits, const BAG_TERM: bool, S: BoardStore>(
    params: &GenMovesParams<'_, N, L>,
    working_buffer: &mut WorkingBuffer<G, BAG_TERM, S>,
    multi_leaves: &klv::MultiLeaves,
    equity_predicate: &EquityPredicate<'_>,
) -> Option<equity::Equity> {
    if !matches!(equity_predicate, EquityPredicate::AcceptAll) {
        return None;
    }
    let max_gen = params.max_gen;
    let game_config = params.board_snapshot.game_config;
    let mut sure = std::mem::take(&mut working_buffer.sure_plays);
    sure.clear();
    if working_buffer.num_tiles_in_bag >= i32::from(game_config.exchange_tile_limit())
        && params.num_exchanges_by_this_player < game_config.exchanges_allowed_per_player()
    {
        let most = (working_buffer.best_leave_values.len() - 1)
            .min(working_buffer.num_tiles_in_bag.max(0) as usize);
        for (exchanged, &best_leave_value) in (working_buffer.best_leave_values.iter().enumerate())
            .take(most + 1)
            .skip(1)
        {
            // an exchange gets no bag term: take it back out, when there is one
            sure.push(equity::Equity::new(best_leave_value.saturating_sub(
                i32::from(game_config.num_played_bonus(exchanged as u8)) * equity::SCALE
                    + if BAG_TERM {
                        multi_leaves.bag_term(exchanged)
                    } else {
                        0
                    },
            )));
        }
    }
    if params.pass_policy.emits_unconditionally() {
        sure.push(equity::Equity::new(if multi_leaves.is_dense() {
            multi_leaves.pass_leave_value().raw()
        } else {
            params
                .board_snapshot
                .klv
                .leave_value_from_tally(&working_buffer.rack_tally)
                .raw()
        }));
    }
    let floor = (max_gen != 0 && max_gen <= sure.len()).then(|| {
        sure.select_nth_unstable_by(max_gen - 1, |a, b| b.cmp(a));
        sure[max_gen - 1]
    });
    working_buffer.sure_plays = sure;
    floor
}

// the best equity an exchange can have: the best leave of the tiles kept when
// one or more go, best_leave_values less the bonus for playing them (an
// exchange plays none)
#[inline(always)]
fn best_exchange_equity<const BAG_TERM: bool>(
    game_config: &game_config::GameConfig,
    best_leave_values: &[i32],
    multi_leaves: &klv::MultiLeaves,
) -> equity::Equity {
    equity::Equity::new(
        (1..best_leave_values.len())
            .map(|exchanged| {
                best_leave_values[exchanged].saturating_sub(
                    i32::from(game_config.num_played_bonus(exchanged as u8)) * equity::SCALE
                        + if BAG_TERM {
                            multi_leaves.bag_term(exchanged)
                        } else {
                            0
                        },
                )
            })
            .max()
            .unwrap_or(i32::MIN),
    )
}

#[inline(always)]
fn check_exchange_limit(game_config: &game_config::GameConfig) -> error::Returns<()> {
    usize::try_from(game_config.exchange_tile_limit())
        .map(|_| ())
        .map_err(|_| {
            format!(
                "negative exchange limit {}",
                game_config.exchange_tile_limit()
            )
            .into()
        })
}

#[inline]
fn kurnia_gen_exchange_moves<
    'a,
    FoundExchangeMove: FnMut(&[u8], equity::Equity),
    N: kwg::Node,
    L: kwg::Node,
    G: LaneBits,
    const BAG_TERM: bool,
    S: BoardStore,
>(
    board_snapshot: &'a BoardSnapshot<'a, N, L>,
    working_buffer: &mut WorkingBuffer<G, BAG_TERM, S>,
    multi_leaves: &klv::MultiLeaves,
    num_exchanges_by_this_player: i16,
    found_exchange_move: FoundExchangeMove,
) {
    if working_buffer.num_tiles_in_bag
        >= i32::from(board_snapshot.game_config.exchange_tile_limit())
        && num_exchanges_by_this_player < board_snapshot.game_config.exchanges_allowed_per_player()
    {
        let remaining = working_buffer.num_tiles_in_bag.max(0).cast_unsigned() as usize;
        if multi_leaves.is_dense() {
            // the table's leaves have the bag term, which an exchange takes back
            if BAG_TERM {
                let mut found_exchange_move = found_exchange_move;
                multi_leaves.kurnia_gen_exchange_moves_unconditionally(
                    |exchanged_tiles: &[u8], leave_value: equity::Equity| {
                        found_exchange_move(
                            exchanged_tiles,
                            equity::Equity::new(
                                leave_value.raw() - multi_leaves.bag_term(exchanged_tiles.len()),
                            ),
                        )
                    },
                    &mut working_buffer.rack_tally,
                    &mut working_buffer.exchange_buffer,
                    remaining,
                );
                return;
            }
            multi_leaves.kurnia_gen_exchange_moves_unconditionally(
                found_exchange_move,
                &mut working_buffer.rack_tally,
                &mut working_buffer.exchange_buffer,
                remaining,
            );
        } else {
            klv::MultiLeaves::gen_exchange_moves_via_klv_of_len(
                board_snapshot.klv,
                found_exchange_move,
                &mut working_buffer.rack_tally,
                board_snapshot.game_config.alphabet().len(),
                &mut working_buffer.exchange_buffer,
                remaining,
            );
        }
    }
}

// the lean arm's cross sets of the lanes the board changed in, the star of an
// empty board, the board kept as the last one seen, and what init leaves to be
// done after the cross sets
#[inline(always)]
fn lean_cross_sets<
    const D: usize,
    N: kwg::Node,
    L: kwg::Node,
    G: LaneBits,
    const BAG_TERM: bool,
    S: BoardStore,
>(
    board_snapshot: &BoardSnapshot<'_, N, L>,
    working_buffer: &mut WorkingBuffer<G, BAG_TERM, S>,
) {
    let board_layout = board_snapshot.game_config.board_layout();
    let fixed;
    let dim = if D != 0 {
        // init, compiled for the same side, asserts the board's
        fixed = square_dim::<D>();
        &fixed
    } else {
        board_layout.dim()
    };
    let area = (dim.rows as isize * dim.cols as isize) as usize;
    let dirty_rows = working_buffer.dirty_rows;
    let dirty_cols = working_buffer.dirty_cols;
    for col in 0..dim.cols {
        if dirty_cols & (G::from(1) << col as u32) != G::from(0) {
            let strip_range_start = (col as isize * dim.rows as isize) as usize;
            let strip_range_end = strip_range_start + dim.rows as usize;
            gen_cross_set::<NoTables, _, _>(
                board_snapshot,
                &working_buffer.transposed_board_tiles[strip_range_start..strip_range_end],
                CrossSetOutput {
                    allowed: &mut working_buffer.allowed_for_across_plays,
                    cross_scores: &mut working_buffer.cross_score_for_across_plays,
                    output_strider: dim.down(col),
                },
                &mut working_buffer.cross_set_buffer_for_across_plays
                    [strip_range_start..strip_range_end],
                &mut working_buffer.cached_cross_set_for_across_plays
                    [strip_range_start..strip_range_end],
                &mut working_buffer.used_letters_tally,
                &mut working_buffer.woadwage.stack,
            );
        }
    }
    let transposed_dim = matrix::Dim {
        rows: dim.cols,
        cols: dim.rows,
    };

    for row in 0..dim.rows {
        if dirty_rows & (G::from(1) << row as u32) != G::from(0) {
            let strip_range_start = (row as isize * dim.cols as isize) as usize;
            let strip_range_end = strip_range_start + dim.cols as usize;
            gen_cross_set::<NoTables, _, _>(
                board_snapshot,
                &board_snapshot.board_tiles[strip_range_start..strip_range_end],
                CrossSetOutput {
                    allowed: &mut working_buffer.allowed_for_down_plays,
                    cross_scores: &mut working_buffer.cross_score_for_down_plays,
                    output_strider: transposed_dim.down(row),
                },
                &mut working_buffer.cross_set_buffer_for_down_plays
                    [strip_range_start..strip_range_end],
                &mut working_buffer.cached_cross_set_for_down_plays
                    [strip_range_start..strip_range_end],
                &mut working_buffer.used_letters_tally,
                &mut working_buffer.woadwage.stack,
            );
        }
    }

    if dirty_rows != G::from(0) || dirty_cols != G::from(0) {
        working_buffer.prev_board_tiles[..area]
            .copy_from_slice(&board_snapshot.board_tiles[..area]);
        working_buffer.prev_tile_count = working_buffer.num_tiles_on_board;
        working_buffer.prev_tile_score = working_buffer.board_tile_score;
    }
    working_buffer.init_after_cross_sets::<D, _, _>(board_snapshot, dirty_cols, dirty_rows);
    if working_buffer.num_tiles_on_board == 0 {
        working_buffer.activate_star(board_layout);
    }
}

#[inline(always)]
fn hotspot_penalty(game_config: &game_config::GameConfig, down: bool, idx: i8, word: &[u8]) -> i32 {
    let alphabet = game_config.alphabet();
    let board_layout = game_config.board_layout();
    (idx..)
        .zip(word)
        .filter(|&(i, &tile)| {
            tile != 0
                && alphabet.is_vowel(tile)
                && if down {
                    board_layout.danger_stars(true)[usize::from(i.cast_unsigned())]
                } else {
                    board_layout.danger_stars(false)[usize::from(i.cast_unsigned())]
                }
        })
        .count() as i32
        * -equity::OPENING_HOTSPOT_PENALTY
}

#[derive(Clone, Copy)]
struct StarLane<'a> {
    down: bool,
    lane: i8,
    star: i8,
    len: i8,
    open: u128,
    word_multipliers: &'a [i8],
    tile_multipliers: &'a [i8],
}

#[inline(never)]
fn walk_jumbled_star_lanes<
    'a,
    N: kwg::Node,
    L: kwg::Node,
    G: LaneBits,
    const BAG_TERM: bool,
    S: BoardStore,
>(
    board_snapshot: &'a BoardSnapshot<'a, N, L>,
    working_buffer: &'a mut WorkingBuffer<G, BAG_TERM, S>,
    multi_leaves: &'a klv::MultiLeaves,
    lanes: [Option<(bool, i8, i8)>; 2],
    threshold: i32,
    found_place_move: FoundPlaceMoveFn<'_>,
) {
    let game_config = board_snapshot.game_config;
    let dim = game_config.board_layout().dim();
    let num_max_played = game_config
        .rack_size()
        .min(working_buffer.num_tiles_on_rack);
    let mut threshold = threshold;
    for (down, lane, anchor) in lanes.into_iter().flatten() {
        let len = if down { dim.rows } else { dim.cols } as usize;
        let start = lane as usize * len;
        let end = start + len;
        let first_threshold = threshold;
        let mut callback = |idx: i8, word: &[u8], score: i32, leave_value: i32| {
            threshold = found_place_move(
                down,
                lane,
                idx,
                word,
                equity::Equity::new(score),
                equity::Equity::new(leave_value + hotspot_penalty(game_config, down, idx, word)),
            )
            .raw();
            threshold
        };
        gen_jumbled_place_moves::<BAG_TERM, _, _, _>(
            &mut GenPlaceMovesParams {
                board_snapshot,
                board_strip: if down {
                    &working_buffer.transposed_board_tiles[start..end]
                } else {
                    &board_snapshot.board_tiles[start..end]
                },
                allowed_strip: if down {
                    &working_buffer.allowed_for_down_plays[start..end]
                } else {
                    &working_buffer.allowed_for_across_plays[start..end]
                },
                cross_set_buffer_strip: if down {
                    &working_buffer.cross_set_buffer_for_across_plays[start..end]
                } else {
                    &working_buffer.cross_set_buffer_for_down_plays[start..end]
                },
                remaining_word_multipliers_strip: if down {
                    &working_buffer.remaining_word_multipliers_for_down_plays[start..end]
                } else {
                    &working_buffer.remaining_word_multipliers_for_across_plays[start..end]
                },
                remaining_tile_multipliers_strip: if down {
                    &working_buffer.remaining_tile_multipliers_for_down_plays[start..end]
                } else {
                    &working_buffer.remaining_tile_multipliers_for_across_plays[start..end]
                },
                face_value_scores_strip: if down {
                    &working_buffer.face_value_scores_for_down_plays[start..end]
                } else {
                    &working_buffer.face_value_scores_for_across_plays[start..end]
                },
                perpendicular_word_multipliers_strip: if down {
                    &working_buffer.perpendicular_word_multipliers_for_down_plays[start..end]
                } else {
                    &working_buffer.perpendicular_word_multipliers_for_across_plays[start..end]
                },
                perpendicular_scores_strip: if down {
                    &working_buffer.perpendicular_scores_for_down_plays[start..end]
                } else {
                    &working_buffer.perpendicular_scores_for_across_plays[start..end]
                },
                rack_tally: &mut working_buffer.rack_tally,
                rack_bits: working_buffer.rack_bits,
                word_strip_buffer: if down {
                    &mut working_buffer.word_buffer_for_down_plays[start..end]
                } else {
                    &mut working_buffer.word_buffer_for_across_plays[start..end]
                },
                num_max_played,
                num_min_played: 0,
                anchor,
                leftmost: 0,
                rightmost: len as i8,
                callback: &mut callback as &mut dyn FnMut(i8, &[u8], i32, i32) -> i32,
                multi_leaves,
                num_tiles_in_bag: working_buffer.num_tiles_in_bag,
                play_out_bonus: working_buffer.play_out_bonus,
                jumbled: &mut working_buffer.jumbled,
                woadwage: &mut working_buffer.woadwage,
                is_census: working_buffer.is_census,
                threshold: first_threshold,
                best_leave_values: &working_buffer.best_leave_values,
            },
            !down,
            Some(down),
        );
    }
}

#[inline(never)]
fn walk_woadwage_star_lanes<
    'a,
    N: kwg::Node,
    L: kwg::Node,
    G: LaneBits,
    const BAG_TERM: bool,
    S: BoardStore,
>(
    board_snapshot: &'a BoardSnapshot<'a, N, L>,
    working_buffer: &'a mut WorkingBuffer<G, BAG_TERM, S>,
    multi_leaves: &'a klv::MultiLeaves,
    lanes: [Option<(bool, i8, i8)>; 2],
    threshold: i32,
    found_place_move: FoundPlaceMoveFn<'_>,
) {
    let game_config = board_snapshot.game_config;
    let dim = game_config.board_layout().dim();
    let num_max_played = game_config
        .rack_size()
        .min(working_buffer.num_tiles_on_rack);
    let mut threshold = threshold;
    for (down, lane, anchor) in lanes.into_iter().flatten() {
        let len = if down { dim.rows } else { dim.cols } as usize;
        let start = lane as usize * len;
        let end = start + len;
        let first_threshold = threshold;
        let mut callback = |idx: i8, word: &[u8], score: i32, leave_value: i32| {
            threshold = found_place_move(
                down,
                lane,
                idx,
                word,
                equity::Equity::new(score),
                equity::Equity::new(leave_value + hotspot_penalty(game_config, down, idx, word)),
            )
            .raw();
            threshold
        };
        gen_woadwage_place_moves::<true, BAG_TERM, _, _, _>(
            &mut GenPlaceMovesParams {
                board_snapshot,
                board_strip: if down {
                    &working_buffer.transposed_board_tiles[start..end]
                } else {
                    &board_snapshot.board_tiles[start..end]
                },
                allowed_strip: if down {
                    &working_buffer.allowed_for_down_plays[start..end]
                } else {
                    &working_buffer.allowed_for_across_plays[start..end]
                },
                cross_set_buffer_strip: if down {
                    &working_buffer.cross_set_buffer_for_across_plays[start..end]
                } else {
                    &working_buffer.cross_set_buffer_for_down_plays[start..end]
                },
                remaining_word_multipliers_strip: if down {
                    &working_buffer.remaining_word_multipliers_for_down_plays[start..end]
                } else {
                    &working_buffer.remaining_word_multipliers_for_across_plays[start..end]
                },
                remaining_tile_multipliers_strip: if down {
                    &working_buffer.remaining_tile_multipliers_for_down_plays[start..end]
                } else {
                    &working_buffer.remaining_tile_multipliers_for_across_plays[start..end]
                },
                face_value_scores_strip: if down {
                    &working_buffer.face_value_scores_for_down_plays[start..end]
                } else {
                    &working_buffer.face_value_scores_for_across_plays[start..end]
                },
                perpendicular_word_multipliers_strip: if down {
                    &working_buffer.perpendicular_word_multipliers_for_down_plays[start..end]
                } else {
                    &working_buffer.perpendicular_word_multipliers_for_across_plays[start..end]
                },
                perpendicular_scores_strip: if down {
                    &working_buffer.perpendicular_scores_for_down_plays[start..end]
                } else {
                    &working_buffer.perpendicular_scores_for_across_plays[start..end]
                },
                rack_tally: &mut working_buffer.rack_tally,
                rack_bits: working_buffer.rack_bits,
                word_strip_buffer: if down {
                    &mut working_buffer.word_buffer_for_down_plays[start..end]
                } else {
                    &mut working_buffer.word_buffer_for_across_plays[start..end]
                },
                num_max_played,
                num_min_played: 0,
                anchor,
                leftmost: 0,
                rightmost: len as i8,
                callback: &mut callback as &mut dyn FnMut(i8, &[u8], i32, i32) -> i32,
                multi_leaves,
                num_tiles_in_bag: working_buffer.num_tiles_in_bag,
                play_out_bonus: working_buffer.play_out_bonus,
                jumbled: &mut working_buffer.jumbled,
                woadwage: &mut working_buffer.woadwage,
                is_census: working_buffer.is_census,
                threshold: first_threshold,
                best_leave_values: &working_buffer.best_leave_values,
            },
            !down,
            &[],
        );
    }
}

#[inline(never)]
fn walk_jumbled_woadwage_star_lanes<
    'a,
    N: kwg::Node,
    L: kwg::Node,
    G: LaneBits,
    const BAG_TERM: bool,
    S: BoardStore,
>(
    board_snapshot: &'a BoardSnapshot<'a, N, L>,
    working_buffer: &'a mut WorkingBuffer<G, BAG_TERM, S>,
    multi_leaves: &'a klv::MultiLeaves,
    lanes: [Option<(bool, i8, i8)>; 2],
    threshold: i32,
    found_place_move: FoundPlaceMoveFn<'_>,
) {
    let game_config = board_snapshot.game_config;
    let dim = game_config.board_layout().dim();
    let num_max_played = game_config
        .rack_size()
        .min(working_buffer.num_tiles_on_rack);
    let mut threshold = threshold;
    for (down, lane, anchor) in lanes.into_iter().flatten() {
        let len = if down { dim.rows } else { dim.cols } as usize;
        let start = lane as usize * len;
        let end = start + len;
        let first_threshold = threshold;
        let mut callback = |idx: i8, word: &[u8], score: i32, leave_value: i32| {
            threshold = found_place_move(
                down,
                lane,
                idx,
                word,
                equity::Equity::new(score),
                equity::Equity::new(leave_value + hotspot_penalty(game_config, down, idx, word)),
            )
            .raw();
            threshold
        };
        gen_jumbled_woadwage_place_moves::<true, BAG_TERM, _, _, _>(
            &mut GenPlaceMovesParams {
                board_snapshot,
                board_strip: if down {
                    &working_buffer.transposed_board_tiles[start..end]
                } else {
                    &board_snapshot.board_tiles[start..end]
                },
                allowed_strip: if down {
                    &working_buffer.allowed_for_down_plays[start..end]
                } else {
                    &working_buffer.allowed_for_across_plays[start..end]
                },
                cross_set_buffer_strip: if down {
                    &working_buffer.cross_set_buffer_for_across_plays[start..end]
                } else {
                    &working_buffer.cross_set_buffer_for_down_plays[start..end]
                },
                remaining_word_multipliers_strip: if down {
                    &working_buffer.remaining_word_multipliers_for_down_plays[start..end]
                } else {
                    &working_buffer.remaining_word_multipliers_for_across_plays[start..end]
                },
                remaining_tile_multipliers_strip: if down {
                    &working_buffer.remaining_tile_multipliers_for_down_plays[start..end]
                } else {
                    &working_buffer.remaining_tile_multipliers_for_across_plays[start..end]
                },
                face_value_scores_strip: if down {
                    &working_buffer.face_value_scores_for_down_plays[start..end]
                } else {
                    &working_buffer.face_value_scores_for_across_plays[start..end]
                },
                perpendicular_word_multipliers_strip: if down {
                    &working_buffer.perpendicular_word_multipliers_for_down_plays[start..end]
                } else {
                    &working_buffer.perpendicular_word_multipliers_for_across_plays[start..end]
                },
                perpendicular_scores_strip: if down {
                    &working_buffer.perpendicular_scores_for_down_plays[start..end]
                } else {
                    &working_buffer.perpendicular_scores_for_across_plays[start..end]
                },
                rack_tally: &mut working_buffer.rack_tally,
                rack_bits: working_buffer.rack_bits,
                word_strip_buffer: if down {
                    &mut working_buffer.word_buffer_for_down_plays[start..end]
                } else {
                    &mut working_buffer.word_buffer_for_across_plays[start..end]
                },
                num_max_played,
                num_min_played: 0,
                anchor,
                leftmost: 0,
                rightmost: len as i8,
                callback: &mut callback as &mut dyn FnMut(i8, &[u8], i32, i32) -> i32,
                multi_leaves,
                num_tiles_in_bag: working_buffer.num_tiles_in_bag,
                play_out_bonus: working_buffer.play_out_bonus,
                jumbled: &mut working_buffer.jumbled,
                woadwage: &mut working_buffer.woadwage,
                is_census: working_buffer.is_census,
                threshold: first_threshold,
                best_leave_values: &working_buffer.best_leave_values,
            },
            !down,
            Some(down),
            &[],
        );
    }
}

// an empty board's place moves, made without the walk, which reads every lane
// to find plays on the star's alone: on the lanes through the star the masks
// hold, across its row and, where the layout is not the same both ways, down
// its column. Each word the rack spells is read from the dawg once (a real tile
// before a blank for each letter, as the walk takes them) and laid on every run
// of open squares over the star, scored from the premiums; a play under the
// threshold is not offered. A census spells each word as the walk does; a
// jumbled game's lanes are filled by the walk's span filler. The cross sets are
// made first, as the walk makes them, for the generation after; the opening's
// penalty comes with each leave value found_place_move is given.
#[inline(always)]
fn gen_empty_board_place_moves<
    'a,
    const D: usize,
    R: RunLetters,
    FoundPlaceMove: FnMut(bool, i8, i8, &[u8], equity::Equity, equity::Equity) -> equity::Equity,
    N: kwg::Node,
    L: kwg::Node,
    G: LaneBits,
    const BAG_TERM: bool,
    S: BoardStore,
>(
    runs: R,
    board_snapshot: &'a BoardSnapshot<'a, N, L>,
    working_buffer: &'a mut WorkingBuffer<G, BAG_TERM, S>,
    multi_leaves: &'a klv::MultiLeaves,
    threshold: i32,
    mut found_place_move: FoundPlaceMove,
) {
    lean_cross_sets::<D, _, _, _, _, _>(board_snapshot, working_buffer);
    gen_empty_board_place_moves_made(
        runs,
        board_snapshot,
        working_buffer,
        multi_leaves,
        threshold,
        &mut found_place_move,
    );
}

// gen_empty_board_place_moves once the cross sets are made, one copy for every
// generation, by the dyn callback
#[inline(never)]
fn gen_empty_board_place_moves_made<
    'a,
    R: RunLetters,
    N: kwg::Node,
    L: kwg::Node,
    G: LaneBits,
    const BAG_TERM: bool,
    S: BoardStore,
>(
    runs: R,
    board_snapshot: &'a BoardSnapshot<'a, N, L>,
    working_buffer: &'a mut WorkingBuffer<G, BAG_TERM, S>,
    multi_leaves: &'a klv::MultiLeaves,
    threshold: i32,
    found_place_move: FoundPlaceMoveFn<'_>,
) {
    let game_config = board_snapshot.game_config;
    let board_layout = game_config.board_layout();
    let dim = board_layout.dim();
    let star_row = board_layout.star_row();
    let star_col = board_layout.star_col();
    // each lane a play may take: its way, its number, the star's place along it
    let lanes = [
        (working_buffer.lane_rows & (G::from(1) << star_row as u32) != G::from(0))
            .then_some((false, star_row, star_col)),
        (!board_layout.is_symmetric()
            && working_buffer.lane_cols & (G::from(1) << star_col as u32) != G::from(0))
        .then_some((true, star_col, star_row)),
    ];
    match game_config.game_rules() {
        game_config::GameRules::Classic => {}
        game_config::GameRules::JumbledWoadwage => {
            return walk_jumbled_woadwage_star_lanes(
                board_snapshot,
                working_buffer,
                multi_leaves,
                lanes,
                threshold,
                found_place_move,
            );
        }
        game_config::GameRules::Jumbled => {
            return walk_jumbled_star_lanes(
                board_snapshot,
                working_buffer,
                multi_leaves,
                lanes,
                threshold,
                found_place_move,
            );
        }
        game_config::GameRules::Woadwage => {
            return walk_woadwage_star_lanes(
                board_snapshot,
                working_buffer,
                multi_leaves,
                lanes,
                threshold,
                found_place_move,
            );
        }
    }
    let num_max_played = game_config
        .rack_size()
        .min(working_buffer.num_tiles_on_rack);

    struct Env<'a, R: RunLetters, N: kwg::Node, L: kwg::Node> {
        board_snapshot: &'a BoardSnapshot<'a, N, L>,
        alphabet: &'a alphabet::Alphabet,
        multi_leaves: &'a klv::MultiLeaves,
        rack_tally: &'a mut [u8],
        rack_bits: u64,
        letter_bits: u64,
        max_len: usize,
        num_tiles_in_bag: i32,
        play_out_bonus: i32,
        lanes: [Option<StarLane<'a>>; 2],
        word: [u8; 128],         // its tiles, a blank's with 0x80
        tile_values: [i32; 128], // each tile's face value, scaled as a score
        threshold: i32,
        found_place_move: FoundPlaceMoveFn<'a>,
        runs: R, // a walk along a run stops where these leave it no candidate
    }

    // the word of len tiles at each place it may take; a census's, scored and
    // valued as nothing, spelled at each place as the walk spells it there: a
    // letter's real tiles on its squares the walk takes first (the star's,
    // those to its left from the nearest, then those to its right), its blanks
    // on the rest
    #[inline(always)]
    fn lay<
        const SPELL_ONCE: bool,
        const BAG_TERM: bool,
        R: RunLetters,
        N: kwg::Node,
        L: kwg::Node,
    >(
        env: &mut Env<'_, R, N, L>,
        len: usize,
        leave_idx: u32,
    ) {
        if SPELL_ONCE {
            let game_config = env.board_snapshot.game_config;
            let squares = (1u128 << len) - 1;
            // the letters the blanks stand for (a word holds no more of them
            // than its tiles)
            let mut blanked = [0u8; 128];
            let mut num_blanked = 0;
            for &tile in &env.word[..len] {
                blanked[num_blanked] = tile & 0x7f;
                num_blanked += usize::from(tile & 0x80 != 0);
            }
            let mut spelled = [0u8; 128];
            for (to, &tile) in spelled.iter_mut().zip(&env.word[..len]) {
                *to = tile & 0x7f;
            }
            for lane in env.lanes.into_iter().flatten() {
                let first = (lane.star as isize + 1 - len as isize).max(0);
                let last = (lane.star as isize).min(lane.len as isize - len as isize);
                for start in first..=last {
                    if (lane.open >> start) & squares != squares {
                        continue;
                    }
                    let start = start as usize;
                    let word = &mut spelled[..len];
                    if num_blanked != 0 {
                        for tile in word.iter_mut() {
                            *tile &= 0x7f;
                        }
                        // the squares the walk takes last, first
                        let star = lane.star as usize - start;
                        let mut left = blanked;
                        let mut num_left = num_blanked;
                        for i in (star + 1..len).rev().chain(0..=star) {
                            if num_left == 0 {
                                break;
                            }
                            if let Some(j) = left[..num_left].iter().position(|&b| b == word[i]) {
                                num_left -= 1;
                                left[j] = left[num_left];
                                word[i] |= 0x80;
                            }
                        }
                    }
                    let word = &spelled[..len];
                    env.threshold = (env.found_place_move)(
                        lane.down,
                        lane.lane,
                        start as i8,
                        word,
                        equity::Equity::new(0),
                        equity::Equity::new(hotspot_penalty(
                            game_config,
                            lane.down,
                            start as i8,
                            word,
                        )),
                    )
                    .raw();
                }
            }
            return;
        }
        let leave_value = leave_value_of::<BAG_TERM, _>(
            env.multi_leaves,
            env.board_snapshot.klv,
            env.rack_tally,
            leave_idx,
            env.num_tiles_in_bag,
            env.play_out_bonus,
            env.alphabet,
        );
        let game_config = env.board_snapshot.game_config;
        let bonus = i32::from(game_config.num_played_bonus(len as u8)) * equity::SCALE;
        let squares = (1u128 << len) - 1;
        for lane in env.lanes.into_iter().flatten() {
            let first = (lane.star as isize + 1 - len as isize).max(0);
            let last = (lane.star as isize).min(lane.len as isize - len as isize);
            for start in first..=last {
                if (lane.open >> start) & squares != squares {
                    continue;
                }
                let start = start as usize;
                let mut main_score = 0i32;
                let mut word_multiplier = 1i32;
                for i in 0..len {
                    main_score += env.tile_values[i] * i32::from(lane.tile_multipliers[start + i]);
                    word_multiplier *= i32::from(lane.word_multipliers[start + i]);
                }
                let score = main_score * word_multiplier + bonus;
                if score + leave_value < env.threshold {
                    continue;
                }
                let word = &env.word[..len];
                env.threshold = (env.found_place_move)(
                    lane.down,
                    lane.lane,
                    start as i8,
                    word,
                    equity::Equity::new(score),
                    equity::Equity::new(
                        leave_value + hotspot_penalty(game_config, lane.down, start as i8, word),
                    ),
                )
                .raw();
            }
        }
    }

    // the words from the run of nodes at p, with depth tiles before them; a
    // census's each once, a blank taken only for a letter the rack has run out
    // of, as the walk takes it
    fn walk<
        const SPELL_ONCE: bool,
        const BAG_TERM: bool,
        R: RunLetters,
        N: kwg::Node,
        L: kwg::Node,
    >(
        env: &mut Env<'_, R, N, L>,
        mut p: i32,
        depth: usize,
        leave_idx: u32,
    ) {
        let mut candidates = if env.rack_tally[0] > 0 {
            env.letter_bits
        } else {
            env.rack_bits & !1
        };
        if candidates == 0 || !env.runs.may_hold(p.cast_unsigned(), candidates) {
            return;
        }
        macro_rules! visit {
            ($node:expr, $tile:expr) => {{
                let (node, tile) = ($node, $tile);
                let len = depth + 1;
                let next = if len < env.max_len {
                    node.arc_index()
                } else {
                    0
                };
                env.board_snapshot.kwg.prefetch(next);
                if env.rack_tally[tile as usize] > 0 {
                    env.rack_tally[tile as usize] -= 1;
                    let spent = (1u64 << tile) * u64::from(env.rack_tally[tile as usize] == 0);
                    env.rack_bits ^= spent;
                    env.word[depth] = tile;
                    env.tile_values[depth] = env.alphabet.scaled_score(tile).raw();
                    let leave_idx = leave_idx.wrapping_sub(env.multi_leaves.place_value(tile));
                    if len >= 2 && node.accepts() {
                        lay::<SPELL_ONCE, BAG_TERM, _, _, _>(env, len, leave_idx);
                    }
                    if next > 0 {
                        walk::<SPELL_ONCE, BAG_TERM, _, _, _>(env, next, len, leave_idx);
                    }
                    env.rack_bits ^= spent;
                    env.rack_tally[tile as usize] += 1;
                }
                if env.rack_tally[0] > 0 && (!SPELL_ONCE || env.rack_tally[tile as usize] == 0) {
                    env.rack_tally[0] -= 1;
                    env.word[depth] = tile | 0x80;
                    env.tile_values[depth] = env.alphabet.scaled_score(0).raw();
                    let leave_idx = leave_idx.wrapping_sub(env.multi_leaves.place_value(0));
                    if len >= 2 && node.accepts() {
                        lay::<SPELL_ONCE, BAG_TERM, _, _, _>(env, len, leave_idx);
                    }
                    if next > 0 {
                        walk::<SPELL_ONCE, BAG_TERM, _, _, _>(env, next, len, leave_idx);
                    }
                    env.rack_tally[0] += 1;
                }
            }};
        }
        match env.runs.from(p.cast_unsigned()) {
            // the run's tiles known: each candidate's node is the count of the
            // run's tiles below it past p
            Some(run) => {
                let mut hits = run & candidates;
                while hits != 0 {
                    let tile = hits.trailing_zeros() as u8;
                    hits &= hits - 1;
                    let at = p + (run & ((1u64 << tile) - 1)).count_ones() as i32;
                    let node = env.board_snapshot.kwg.node_in_run(at);
                    debug_assert_eq!(node.tile(), tile);
                    visit!(node, tile);
                }
            }
            None => loop {
                let node = env.board_snapshot.kwg.node_in_run(p);
                let tile = node.tile();
                let bit = 1u64 << tile;
                if candidates & bit != 0 {
                    candidates ^= bit;
                    visit!(node, tile);
                    if candidates == 0
                        || (!node.is_end()
                            && !env.runs.may_hold((p + 1).cast_unsigned(), candidates))
                    {
                        break;
                    }
                }
                if node.is_end() {
                    break;
                }
                p += 1;
            },
        }
    }

    // each lane's squares, those a tile may take as the walk's cross sets mark
    // them (a punctured one takes none)
    let lanes = lanes.map(|taken| {
        taken.map(|(down, lane, star)| {
            let len = if down { dim.rows } else { dim.cols };
            let start = lane as usize * len as usize;
            let end = start + len as usize;
            let allowed = if down {
                &working_buffer.allowed_for_down_plays[start..end]
            } else {
                &working_buffer.allowed_for_across_plays[start..end]
            };
            let mut open = 0u128;
            for (i, &bits) in allowed.iter().enumerate() {
                open |= u128::from(bits != 1) << i;
            }
            StarLane {
                down,
                lane,
                star,
                len,
                open,
                word_multipliers: if down {
                    &working_buffer.remaining_word_multipliers_for_down_plays[start..end]
                } else {
                    &working_buffer.remaining_word_multipliers_for_across_plays[start..end]
                },
                tile_multipliers: if down {
                    &working_buffer.remaining_tile_multipliers_for_down_plays[start..end]
                } else {
                    &working_buffer.remaining_tile_multipliers_for_across_plays[start..end]
                },
            }
        })
    });
    let longest = lanes
        .iter()
        .flatten()
        .map(|lane| lane.len)
        .max()
        .unwrap_or(0);
    let max_len = (num_max_played as usize).min(longest as usize);
    let root = board_snapshot.kwg.node_in_run(0).arc_index();
    if max_len < 2 || root <= 0 {
        return;
    }
    let alphabet = game_config.alphabet();
    let is_census = working_buffer.is_census;
    let env = &mut Env {
        board_snapshot,
        alphabet,
        multi_leaves,
        rack_tally: &mut working_buffer.rack_tally,
        rack_bits: working_buffer.rack_bits,
        letter_bits: (u64::MAX >> (64 - u32::from(alphabet.len()))) & !1,
        max_len,
        num_tiles_in_bag: working_buffer.num_tiles_in_bag,
        play_out_bonus: working_buffer.play_out_bonus,
        lanes,
        word: [0; 128],
        tile_values: [0; 128],
        threshold,
        found_place_move,
        runs,
    };
    if is_census {
        walk::<true, false, _, _, _>(env, root, 0, multi_leaves.pass_leave_idx());
    } else if BAG_TERM {
        walk::<false, true, _, _, _>(env, root, 0, multi_leaves.pass_leave_idx());
    } else {
        walk::<false, false, _, _, _>(env, root, 0, multi_leaves.pass_leave_idx());
    }
}

struct KurniaIterParams<
    'a,
    R: RunLetters,
    FoundPlaceMove: 'a + FnMut(bool, i8, i8, &[u8], equity::Equity, equity::Equity) -> equity::Equity,
    CanAccept: 'a + Fn(equity::Equity) -> bool,
    CurrentThreshold: 'a + Fn() -> equity::Equity,
    N: kwg::Node,
    L: kwg::Node,
    G: LaneBits,
    const BAG_TERM: bool,
    S: BoardStore,
> {
    runs: R,
    board_snapshot: &'a BoardSnapshot<'a, N, L>,
    working_buffer: &'a mut WorkingBuffer<G, BAG_TERM, S>,
    multi_leaves: &'a klv::MultiLeaves,
    found_place_move: FoundPlaceMove,
    can_accept: CanAccept,
    current_threshold: CurrentThreshold,
}

#[inline]
fn kurnia_gen_place_moves_iter_lean<
    'a,
    const D: usize,
    W: LaneBits,
    R: 'a + RunLetters,
    FoundPlaceMove: 'a + FnMut(bool, i8, i8, &[u8], equity::Equity, equity::Equity) -> equity::Equity,
    CanAccept: 'a + Fn(equity::Equity) -> bool,
    CurrentThreshold: 'a + Fn() -> equity::Equity,
    N: kwg::Node,
    L: kwg::Node,
    G: LaneBits,
    const BAG_TERM: bool,
    S: BoardStore,
>(
    p: KurniaIterParams<'a, R, FoundPlaceMove, CanAccept, CurrentThreshold, N, L, G, BAG_TERM, S>,
) -> impl 'a + Iterator {
    let KurniaIterParams {
        runs,
        board_snapshot,
        working_buffer,
        multi_leaves,
        mut found_place_move,
        can_accept,
        current_threshold,
    } = p;
    let game_config = &board_snapshot.game_config;
    let board_layout = game_config.board_layout();
    let fixed;
    let dim = if D != 0 {
        // init, compiled for the same side, asserts the board's
        fixed = square_dim::<D>();
        &fixed
    } else {
        board_layout.dim()
    };
    let max_rack_size = game_config.rack_size();
    let num_max_played = max_rack_size.min(working_buffer.num_tiles_on_rack);

    lean_cross_sets::<D, _, _, _, _, _>(board_snapshot, working_buffer);

    let feasible_lengths = feasible_word_lengths::<false>(
        board_snapshot.anagrams,
        board_snapshot.rack_lengths,
        &working_buffer.rack_tally,
    );
    let mut found_placements = std::mem::take(&mut working_buffer.found_placements);
    found_placements.clear();
    let mut placement_order = std::mem::take(&mut working_buffer.placement_order);
    placement_order.clear();
    let bounds_stride = dim.rows.max(dim.cols) as usize + 1;
    working_buffer.anchor_left_bounds.clear();
    for row in 0..dim.rows {
        if working_buffer.lane_rows & (G::from(1) << row as u32) == G::from(0) {
            continue;
        }
        let lane_len = dim.cols as usize;
        if !(found_placements.room_for(&working_buffer.room, lane_len)
            && (working_buffer.anchor_left_bounds)
                .room_for(&working_buffer.room, lane_len * 2 * bounds_stride))
        {
            continue;
        }
        let strip_range_start = (row as isize * dim.cols as isize) as usize;
        let strip_range_end = strip_range_start + dim.cols as usize;
        shadow_lane_lean::<D, W, _>(
            &mut GenPlacePlacementsParams {
                board_strip: &board_snapshot.board_tiles[strip_range_start..strip_range_end],
                alphabet: board_snapshot.game_config.alphabet(),
                rack_tally: &mut working_buffer.rack_tally,
                used_tile_scores_shadowl: &mut working_buffer.used_tile_scores_shadowl,
                used_tile_scores_shadowr: &mut working_buffer.used_tile_scores_shadowr,
                allowed_strip: &working_buffer.allowed_for_across_plays
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
                room: &working_buffer.room,
                best_leave_values: &working_buffer.best_leave_values,
                num_max_played,
                rack_tally_shadowl: &mut working_buffer.rack_tally_shadowl,
                rack_tally_shadowr: &mut working_buffer.rack_tally_shadowr,
                span_out: &mut working_buffer.span_out,
                per_span: false,
                left_bounds: &mut working_buffer.anchor_left_bounds,
                bounds_stride,
                check: (),
            },
            true,
            false,
            |anchor: i8, leftmost: i8, rightmost: i8, best_possible_equity: i32, num_played: u8| {
                found_placements.push_in(
                    &working_buffer.room,
                    PossiblePlacement {
                        num_played,
                        down: false,
                        lane: row,
                        anchor,
                        leftmost,
                        rightmost,
                        best_possible_equity,
                        kept_at: NO_KEPT,
                    },
                );
            },
        );
    }
    for col in 0..dim.cols {
        if working_buffer.lane_cols & (G::from(1) << col as u32) == G::from(0) {
            continue;
        }
        let lane_len = dim.rows as usize;
        if !(found_placements.room_for(&working_buffer.room, lane_len)
            && (working_buffer.anchor_left_bounds)
                .room_for(&working_buffer.room, lane_len * 2 * bounds_stride))
        {
            continue;
        }
        let strip_range_start = (col as isize * dim.rows as isize) as usize;
        let strip_range_end = strip_range_start + dim.rows as usize;
        shadow_lane_lean::<D, W, _>(
            &mut GenPlacePlacementsParams {
                board_strip: &working_buffer.transposed_board_tiles
                    [strip_range_start..strip_range_end],
                alphabet: board_snapshot.game_config.alphabet(),
                rack_tally: &mut working_buffer.rack_tally,
                used_tile_scores_shadowl: &mut working_buffer.used_tile_scores_shadowl,
                used_tile_scores_shadowr: &mut working_buffer.used_tile_scores_shadowr,
                allowed_strip: &working_buffer.allowed_for_down_plays
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
                room: &working_buffer.room,
                best_leave_values: &working_buffer.best_leave_values,
                num_max_played,
                rack_tally_shadowl: &mut working_buffer.rack_tally_shadowl,
                rack_tally_shadowr: &mut working_buffer.rack_tally_shadowr,
                span_out: &mut working_buffer.span_out,
                per_span: false,
                left_bounds: &mut working_buffer.anchor_left_bounds,
                bounds_stride,
                check: (),
            },
            false,
            false,
            |anchor: i8, leftmost: i8, rightmost: i8, best_possible_equity: i32, num_played: u8| {
                found_placements.push_in(
                    &working_buffer.room,
                    PossiblePlacement {
                        num_played,
                        down: true,
                        lane: col,
                        anchor,
                        leftmost,
                        rightmost,
                        best_possible_equity,
                        kept_at: NO_KEPT,
                    },
                );
            },
        );
    }
    if !placement_order.room_for(&working_buffer.room, found_placements.len()) {
        found_placements.clear();
    }
    placement_order.extend(
        found_placements
            .iter()
            .enumerate()
            .map(|(i, p)| (p.best_possible_equity, i as u32)),
    );
    placement_order.sort_unstable_by_key(|&(equity, _)| equity);
    working_buffer.found_placements = found_placements;
    working_buffer.placement_order = placement_order;
    // the classic census spells placements. Its caller scores them separately.
    let spells_only = working_buffer.is_census
        && match game_config.game_rules() {
            game_config::GameRules::Classic => true,
            game_config::GameRules::Jumbled
            | game_config::GameRules::JumbledWoadwage
            | game_config::GameRules::Woadwage => false,
        };
    std::iter::from_fn(move || match working_buffer.placement_order.pop() {
        Some((equity, idx)) => {
            if can_accept(equity::Equity::new(equity)) {
                let placement = working_buffer.found_placements[idx as usize];
                working_buffer.bounds_at = idx as usize * 2 * bounds_stride;
                gen_place_moves_at_lean::<D, _, _, _, _, _, _, _>(GenPlaceMovesAtParams {
                    runs,
                    board_snapshot,
                    working_buffer,
                    multi_leaves,
                    placement: &placement,
                    num_max_played,
                    threshold: current_threshold().raw(),
                    found_place_move:
                        &mut |down: bool,
                              lane: i8,
                              idx: i8,
                              word: &[u8],
                              score: equity::Equity,
                              leave_value: equity::Equity| {
                            let this_best = score.raw() + leave_value.raw();
                            debug_assert!(
                                spells_only || this_best <= placement.best_possible_equity,
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
fn walk_raw_places<
    const OUT_ONLY: bool,
    const ANY_SIDE: bool,
    R: RunLetters,
    N: kwg::Node,
    L: kwg::Node,
    G: LaneBits,
    const BAG_TERM: bool,
    S: BoardStore,
    F: FnMut(bool, i8, i8, &[u8], equity::Equity, equity::Equity) -> equity::Equity,
>(
    runs: R,
    board_snapshot: &BoardSnapshot<'_, N, L>,
    working_buffer: &mut WorkingBuffer<G, BAG_TERM, S>,
    multi_leaves: &klv::MultiLeaves,
    found_place_move: F,
) {
    let dim = board_snapshot.game_config.board_layout().dim();
    if working_buffer.num_tiles_on_board == 0 {
        if ANY_SIDE {
            gen_empty_board_place_moves::<0, _, _, _, _, _, _, _>(
                runs,
                board_snapshot,
                working_buffer,
                multi_leaves,
                i32::MIN,
                found_place_move,
            );
        } else if dim.rows == 15 {
            gen_empty_board_place_moves::<15, _, _, _, _, _, _, _>(
                runs,
                board_snapshot,
                working_buffer,
                multi_leaves,
                i32::MIN,
                found_place_move,
            );
        } else {
            gen_empty_board_place_moves::<21, _, _, _, _, _, _, _>(
                runs,
                board_snapshot,
                working_buffer,
                multi_leaves,
                i32::MIN,
                found_place_move,
            );
        }
    } else if ANY_SIDE {
        kurnia_gen_raw_place_moves::<OUT_ONLY, 0, G, _, _, _, _, _, _, _>(
            runs,
            board_snapshot,
            working_buffer,
            multi_leaves,
            found_place_move,
        );
    } else if dim.rows == 15 {
        kurnia_gen_raw_place_moves::<OUT_ONLY, 15, u32, _, _, _, _, _, _, _>(
            runs,
            board_snapshot,
            working_buffer,
            multi_leaves,
            found_place_move,
        );
    } else {
        kurnia_gen_raw_place_moves::<OUT_ONLY, 21, u32, _, _, _, _, _, _, _>(
            runs,
            board_snapshot,
            working_buffer,
            multi_leaves,
            found_place_move,
        );
    }
}

#[cfg(not(target_family = "wasm"))]
fn table_outs<N: kwg::Node, L: kwg::Node, G: LaneBits, const BAG_TERM: bool, S: BoardStore>(
    snapshot: &BoardSnapshot<'_, N, L>,
    w: &mut WorkingBuffer<G, BAG_TERM, S>,
    table: &anagrams::Anagrams,
    rack: &[u8],
    found: &mut impl FnMut(bool, i8, equity::Equity),
) {
    let layout = snapshot.game_config.board_layout();
    let dim = layout.dim();
    let opening = w.num_tiles_on_board == 0;
    let mut tally = [0u8; MAX_ALPHABET_LEN];
    for &tile in rack {
        tally[usize::from(tile)] += 1;
    }
    let blanks = tally[0];
    let letters = (u64::MAX >> (u64::BITS - u32::from(snapshot.game_config.alphabet().len()))) & !1;
    let rack_letters = if blanks != 0 { letters } else { w.rack_bits };
    let keyer = |word: &[u8]| {
        let layout = table.layout();
        if table.nibble_side().is_some() {
            alphagram::Nibble::key_of(layout.max_counts(), word)
        } else {
            layout.key_of(word)
        }
    };
    let scratch = &mut w.pool;
    for (down, mask, board, allowed) in [
        (
            false,
            w.lane_rows,
            snapshot.board_tiles,
            &w.allowed_for_across_plays[..],
        ),
        (
            true,
            w.lane_cols,
            &w.transposed_board_tiles[..],
            &w.allowed_for_down_plays[..],
        ),
    ] {
        let side = if down { dim.rows } else { dim.cols } as usize;
        for (lane, (board, allowed)) in board
            .chunks_exact(side)
            .zip(allowed.chunks_exact(side))
            .enumerate()
        {
            if (opening && down && layout.is_symmetric())
                || mask & (G::from(1) << lane as u32) == G::from(0)
                || !room_for_out(board, allowed, rack_letters, rack.len() as u8)
            {
                continue;
            }
            let mut key_letters = [0; i8::MAX as usize];
            let real = rack.len() - usize::from(blanks);
            for (to, &tile) in key_letters
                .iter_mut()
                .zip(rack.iter().filter(|&&tile| tile != 0))
            {
                *to = tile;
            }
            for left in 0..board.len() {
                if left != 0 && board[left - 1] != 0 {
                    continue;
                }
                let mut count = 0;
                let mut connected = false;
                let mut key_len = real;
                let mut blank_bits = 0;
                for (right, (&tile, &allowed_here)) in
                    board.iter().zip(allowed).enumerate().skip(left)
                {
                    if tile == 0 {
                        if count == rack.len()
                            || allowed_here & rack_letters == 0
                            || (down && rack.len() == 1 && allowed_here & 1 != 0)
                        {
                            break;
                        }
                        connected |= allowed_here & 1 != 0;
                        blank_bits |= allowed_here & letters;
                        count += 1;
                    } else {
                        if key_len == key_letters.len() {
                            break;
                        }
                        connected = true;
                        key_letters[key_len] = tile & 0x7f;
                        key_len += 1;
                    }
                    if count != rack.len()
                        || left == right
                        || !connected
                        || (right + 1 < board.len() && board[right + 1] != 0)
                    {
                        continue;
                    }
                    let len = (right + 1 - left) as u8;
                    let Some(key) = keyer(&key_letters[..key_len]) else {
                        continue;
                    };
                    let mut fit = |words: alphagram::Words<'_>| {
                        for word in words.iter() {
                            let span = left..=right;
                            if word
                                .iter()
                                .zip(&board[span.clone()])
                                .zip(&allowed[span.clone()])
                                .any(|((&letter, &fixed), &cross)| {
                                    if fixed == 0 {
                                        cross & (1 << letter) == 0
                                    } else {
                                        fixed & 0x7f != letter
                                    }
                                })
                            {
                                continue;
                            }
                            scratch.word.clear();
                            scratch.word.extend(word.iter().zip(&board[span]).map(
                                |(&letter, &fixed)| {
                                    if fixed == 0 { letter } else { 0 }
                                },
                            ));
                            let score = play_scorer::score_and_blank_deltas(
                                snapshot,
                                down,
                                lane as i8,
                                left as i8,
                                &scratch.word,
                                &mut scratch.deltas,
                            );
                            designate(
                                &mut tally,
                                &scratch.deltas,
                                &mut scratch.word,
                                0,
                                0,
                                score,
                                &mut |_, score| {
                                    found(down, lane as i8, equity::Equity::new(score));
                                },
                            );
                        }
                    };
                    if blanks == 0 {
                        if let Some(words) = table.words(alphagram::Fitted(key), len) {
                            fit(words);
                        }
                    } else {
                        blank_out_words(table, key, len, blanks, blank_bits, &mut fit);
                    }
                }
            }
        }
    }
}

#[cfg(not(target_family = "wasm"))]
fn blank_out_words(
    table: &anagrams::Anagrams,
    key: u128,
    len: u8,
    blanks: u8,
    mut possible: u64,
    visit: &mut impl FnMut(alphagram::Words<'_>),
) {
    if blanks == 1 {
        table.blank_groups(alphagram::Fitted(key), len, possible, |_, at| {
            visit(table.words_at(at))
        });
        return;
    }
    let layout = table.layout();
    let add = |tile| {
        if table.nibble_side().is_some() {
            add_blank_letter(alphagram::Nibble, key, tile, layout.max_counts())
        } else {
            add_blank_letter(layout, key, tile, layout.max_counts())
        }
    };
    while possible != 0 {
        let tile = possible.trailing_zeros() as u8;
        if let Some(key) = add(tile) {
            blank_out_words(table, key, len, blanks - 1, possible, visit);
        }
        possible &= possible - 1;
    }
}

#[cfg(not(target_family = "wasm"))]
fn add_blank_letter(
    places: impl alphagram::KeyPlaces,
    key: u128,
    tile: u8,
    counts: &[u8],
) -> Option<u128> {
    (places.count_in(key, tile) < places.max_count(tile, counts))
        .then(|| key + places.place_value(tile))
}

fn room_for_out(board: &[u8], allowed: &[u64], rack_letters: u64, required: u8) -> bool {
    let mut fitting = 0;
    for (&tile, &letters) in board.iter().zip(allowed) {
        if tile == 0 {
            if letters & rack_letters == 0 {
                fitting = 0;
            } else {
                fitting += 1;
                if fitting == required {
                    return true;
                }
            }
        }
    }
    false
}

fn word_fits_out(
    anagrams: &anagrams::Anagrams,
    board: &[u8],
    allowed: &[u64],
    rack: &[u8],
    rack_letters: u64,
) -> bool {
    let mut word = [0; i8::MAX as usize];
    word[..rack.len()].copy_from_slice(rack);
    for left in 0..board.len() {
        if left != 0 && board[left - 1] != 0 {
            continue;
        }
        let mut blanks = 0;
        let mut len = rack.len();
        for right in left..board.len() {
            let tile = board[right];
            if tile == 0 {
                if blanks == rack.len() || allowed[right] & rack_letters == 0 {
                    break;
                }
                blanks += 1;
            } else {
                if len == word.len() {
                    break;
                }
                word[len] = tile & 0x7f;
                len += 1;
            }
            if blanks == rack.len() && (right + 1 == board.len() || board[right + 1] == 0) {
                let word = &word[..len];
                let layout = anagrams.layout();
                let key = if anagrams.nibble_side().is_some() {
                    alphagram::Nibble::key_of(layout.max_counts(), word)
                } else {
                    layout.key_of(word)
                };
                if key.is_none_or(|key| {
                    anagrams
                        .group_at(alphagram::Fitted(key), len as u8)
                        .is_some()
                }) {
                    return true;
                }
            }
        }
    }
    false
}

// the raw moves of the lanes in working_buffer's masks, no bound and no order:
// each lane's anchors found by the shadow pass's raw arm and walked at once,
// from the last found, the lanes from the last column and then from the last
// row (the order the lean iterator popped its raw placements in), with no
// placement kept and each lane's strips cut once
#[inline]
fn kurnia_gen_raw_place_moves<
    'a,
    const OUT_ONLY: bool,
    const D: usize,
    W: LaneBits,
    R: 'a + RunLetters,
    FoundPlaceMove: FnMut(bool, i8, i8, &[u8], equity::Equity, equity::Equity) -> equity::Equity,
    N: kwg::Node,
    L: kwg::Node,
    G: LaneBits,
    const BAG_TERM: bool,
    S: BoardStore,
>(
    runs: R,
    board_snapshot: &'a BoardSnapshot<'a, N, L>,
    working_buffer: &'a mut WorkingBuffer<G, BAG_TERM, S>,
    multi_leaves: &'a klv::MultiLeaves,
    mut found_place_move: FoundPlaceMove,
) {
    let game_config = &board_snapshot.game_config;
    let dim = game_config.board_layout().dim();
    let num_max_played = game_config
        .rack_size()
        .min(working_buffer.num_tiles_on_rack);
    lean_cross_sets::<D, _, _, _, _, _>(board_snapshot, working_buffer);
    // a rack of one tile, whose every play places that tile alone: the plays
    // the cross sets give, in the classic walk's order where the graph's runs
    // ascend
    if working_buffer.num_tiles_on_rack == 1 {
        kurnia_gen_one_tile_moves(
            board_snapshot,
            working_buffer,
            multi_leaves,
            found_place_move,
        );
        return;
    }
    let num_min_played = if OUT_ONLY { num_max_played } else { 0 };
    let feasible_lengths = if OUT_ONLY {
        !0
    } else {
        feasible_word_lengths::<false>(
            board_snapshot.anagrams,
            board_snapshot.rack_lengths,
            &working_buffer.rack_tally,
        )
    };
    let table = if OUT_ONLY
        && working_buffer.rack_tally[0] == 0
        && matches!(
            game_config.game_rules(),
            game_config::GameRules::Classic | game_config::GameRules::Jumbled
        ) {
        board_snapshot
            .anagrams
            .filter(|table| table.fit(game_config))
    } else {
        None
    };
    let mut rack = [0u8; i8::MAX as usize];
    let mut held = 0;
    if table.is_some() {
        for (tile, &count) in working_buffer.rack_tally.iter().enumerate() {
            let end = held + usize::from(count);
            rack[held..end].fill(tile as u8);
            held = end;
        }
    }
    let bounds_stride = dim.rows.max(dim.cols) as usize + 1;
    let mut anchors = std::mem::take(&mut working_buffer.raw_anchors);
    for at in (0..dim.rows as usize + dim.cols as usize).rev() {
        let down = at >= dim.rows as usize;
        let lane = if down { at - dim.rows as usize } else { at } as i8;
        if (if down {
            working_buffer.lane_cols
        } else {
            working_buffer.lane_rows
        }) & (G::from(1) << lane as u32)
            == G::from(0)
        {
            continue;
        }
        let len = if down { dim.rows } else { dim.cols } as usize;
        let start = lane as usize * len;
        let end = start + len;
        let board_strip = if down {
            &working_buffer.transposed_board_tiles[start..end]
        } else {
            &board_snapshot.board_tiles[start..end]
        };
        let allowed_strip = if down {
            &working_buffer.allowed_for_down_plays[start..end]
        } else {
            &working_buffer.allowed_for_across_plays[start..end]
        };
        let rack_letters = if working_buffer.rack_tally[0] == 0 {
            working_buffer.rack_bits
        } else {
            !1
        };
        if OUT_ONLY && !room_for_out(board_strip, allowed_strip, rack_letters, num_min_played) {
            continue;
        }
        if let Some(table) = table
            && !word_fits_out(
                table,
                board_strip,
                allowed_strip,
                &rack[..held],
                rack_letters,
            )
        {
            continue;
        }
        anchors.clear();
        shadow_lane_lean::<D, W, _>(
            &mut GenPlacePlacementsParams {
                board_strip: if down {
                    &working_buffer.transposed_board_tiles[start..end]
                } else {
                    &board_snapshot.board_tiles[start..end]
                },
                alphabet: board_snapshot.game_config.alphabet(),
                rack_tally: &mut working_buffer.rack_tally,
                used_tile_scores_shadowl: &mut working_buffer.used_tile_scores_shadowl,
                used_tile_scores_shadowr: &mut working_buffer.used_tile_scores_shadowr,
                allowed_strip: if down {
                    &working_buffer.allowed_for_down_plays[start..end]
                } else {
                    &working_buffer.allowed_for_across_plays[start..end]
                },
                remaining_word_multipliers_strip: if down {
                    &working_buffer.remaining_word_multipliers_for_down_plays[start..end]
                } else {
                    &working_buffer.remaining_word_multipliers_for_across_plays[start..end]
                },
                remaining_tile_multipliers_strip: if down {
                    &working_buffer.remaining_tile_multipliers_for_down_plays[start..end]
                } else {
                    &working_buffer.remaining_tile_multipliers_for_across_plays[start..end]
                },
                perpendicular_word_multipliers_strip: if down {
                    &working_buffer.perpendicular_word_multipliers_for_down_plays[start..end]
                } else {
                    &working_buffer.perpendicular_word_multipliers_for_across_plays[start..end]
                },
                perpendicular_scores_strip: if down {
                    &working_buffer.perpendicular_scores_for_down_plays[start..end]
                } else {
                    &working_buffer.perpendicular_scores_for_across_plays[start..end]
                },
                rack_bits: working_buffer.rack_bits,
                feasible_lengths,
                descending_scores: &working_buffer.descending_scores,
                lane_scaffold: &mut working_buffer.lane_scaffold[at],
                room: &working_buffer.room,
                best_leave_values: &working_buffer.best_leave_values,
                num_max_played,
                rack_tally_shadowl: &mut working_buffer.rack_tally_shadowl,
                rack_tally_shadowr: &mut working_buffer.rack_tally_shadowr,
                span_out: &mut working_buffer.span_out,
                per_span: false,
                left_bounds: &mut working_buffer.anchor_left_bounds,
                bounds_stride,
                check: (),
            },
            !down,
            true,
            |anchor: i8, leftmost: i8, rightmost: i8, _bound: i32, _num_played: u8| {
                anchors.push((anchor, leftmost, rightmost));
            },
        );
        if anchors.is_empty() {
            continue;
        }
        let board_strip = if down {
            &working_buffer.transposed_board_tiles[start..end]
        } else {
            &board_snapshot.board_tiles[start..end]
        };
        let (allowed_strip, cross_set_buffer_strip) = if down {
            (
                &working_buffer.allowed_for_down_plays[start..end],
                &working_buffer.cross_set_buffer_for_across_plays[start..end],
            )
        } else {
            (
                &working_buffer.allowed_for_across_plays[start..end],
                &working_buffer.cross_set_buffer_for_down_plays[start..end],
            )
        };
        let (word_multipliers, tile_multipliers, face_values) = if down {
            (
                &working_buffer.remaining_word_multipliers_for_down_plays[start..end],
                &working_buffer.remaining_tile_multipliers_for_down_plays[start..end],
                &working_buffer.face_value_scores_for_down_plays[start..end],
            )
        } else {
            (
                &working_buffer.remaining_word_multipliers_for_across_plays[start..end],
                &working_buffer.remaining_tile_multipliers_for_across_plays[start..end],
                &working_buffer.face_value_scores_for_across_plays[start..end],
            )
        };
        let (perpendicular_word_multipliers, perpendicular_scores) = if down {
            (
                &working_buffer.perpendicular_word_multipliers_for_down_plays[start..end],
                &working_buffer.perpendicular_scores_for_down_plays[start..end],
            )
        } else {
            (
                &working_buffer.perpendicular_word_multipliers_for_across_plays[start..end],
                &working_buffer.perpendicular_scores_for_across_plays[start..end],
            )
        };
        let word_buffer = if down {
            &mut working_buffer.word_buffer_for_down_plays[start..end]
        } else {
            &mut working_buffer.word_buffer_for_across_plays[start..end]
        };
        for &(anchor, leftmost, rightmost) in anchors.iter().rev() {
            let span = leftmost as usize..rightmost as usize;
            if OUT_ONLY
                && !room_for_out(
                    &board_strip[span.clone()],
                    &allowed_strip[span],
                    rack_letters,
                    num_min_played,
                )
            {
                continue;
            }
            gen_place_moves_lean::<false, false, _, _, _, _>(
                &mut GenPlaceMovesParams {
                    board_snapshot,
                    board_strip,
                    allowed_strip,
                    cross_set_buffer_strip,
                    remaining_word_multipliers_strip: word_multipliers,
                    remaining_tile_multipliers_strip: tile_multipliers,
                    face_value_scores_strip: face_values,
                    perpendicular_word_multipliers_strip: perpendicular_word_multipliers,
                    perpendicular_scores_strip: perpendicular_scores,
                    rack_tally: &mut working_buffer.rack_tally,
                    rack_bits: working_buffer.rack_bits,
                    word_strip_buffer: &mut *word_buffer,
                    num_max_played,
                    num_min_played,
                    anchor,
                    leftmost,
                    rightmost,
                    callback: |idx: i8, word: &[u8], score: i32, leave_value: i32| {
                        found_place_move(
                            down,
                            lane,
                            idx,
                            word,
                            equity::Equity::new(score),
                            equity::Equity::new(leave_value),
                        )
                        .raw()
                    },
                    multi_leaves,
                    num_tiles_in_bag: working_buffer.num_tiles_in_bag,
                    play_out_bonus: working_buffer.play_out_bonus,
                    jumbled: &mut working_buffer.jumbled,
                    woadwage: &mut working_buffer.woadwage,
                    is_census: working_buffer.is_census,
                    threshold: i32::MIN,
                    best_leave_values: &working_buffer.best_leave_values,
                },
                !down,
                &[],
                runs,
            );
        }
    }
    working_buffer.raw_anchors = anchors;
}

// the moves of a rack of one tile, on the lanes in working_buffer's masks, with
// no walk: a tile placed alone makes at most one word across and one down (its
// letters in any order under jumbled rules), and its square's cross set each
// way says which letters make that way's word (bit 0, whether a tile is beside
// it that way) and what that way's tiles beside it score at face value. They
// come as the classic raw walk finds them where the graph's runs ascend: lane
// by lane, each lane's runs of tiles from the left, a run's square after it
// (unless it is the one square between two runs, which the next run's walk
// takes) and then its square before it, whose word takes in a run just beyond.
// A down lane plays only a square no tile is beside across: the across lane
// plays the others
#[inline]
fn kurnia_gen_one_tile_moves<
    'a,
    FoundPlaceMove: FnMut(bool, i8, i8, &[u8], equity::Equity, equity::Equity) -> equity::Equity,
    N: kwg::Node,
    L: kwg::Node,
    G: LaneBits,
    const BAG_TERM: bool,
    S: BoardStore,
>(
    board_snapshot: &'a BoardSnapshot<'a, N, L>,
    working_buffer: &'a mut WorkingBuffer<G, BAG_TERM, S>,
    multi_leaves: &'a klv::MultiLeaves,
    mut found_place_move: FoundPlaceMove,
) {
    let game_config = board_snapshot.game_config;
    let alphabet = game_config.alphabet();
    let dim = game_config.board_layout().dim();
    let (rows, cols) = (dim.rows as usize, dim.cols as usize);
    let Some(tile) = working_buffer
        .rack_tally
        .iter()
        .position(|&count| count != 0)
    else {
        return;
    };
    let tile = tile as u8;
    // the letters the tile may be (a blank, any), its value before its square's
    // multiplier, and the leave once it is played, as the walk has them
    let (letters, value, blank_bit) = if tile == 0 {
        (
            (u64::MAX >> (64 - u32::from(alphabet.len()))) & !1,
            alphabet.scaled_score(0).raw(),
            0x80,
        )
    } else {
        (1u64 << tile, alphabet.scaled_score(tile).raw(), 0)
    };
    working_buffer.rack_tally[tile as usize] -= 1;
    let leave_value = leave_value_of::<BAG_TERM, _>(
        multi_leaves,
        board_snapshot.klv,
        &working_buffer.rack_tally,
        multi_leaves
            .pass_leave_idx()
            .wrapping_sub(multi_leaves.place_value(tile)),
        working_buffer.num_tiles_in_bag,
        working_buffer.play_out_bonus,
        alphabet,
    );
    working_buffer.rack_tally[tile as usize] += 1;
    let bonus = i32::from(game_config.num_played_bonus(1)) * equity::SCALE;
    let mut threshold = i32::MIN;
    for at in (0..rows + cols).rev() {
        let down = at >= rows;
        let lane = if down { at - rows } else { at };
        if (if down {
            working_buffer.lane_cols
        } else {
            working_buffer.lane_rows
        }) & (G::from(1) << lane as u32)
            == G::from(0)
        {
            continue;
        }
        let len = if down { rows } else { cols };
        let start = lane * len;
        let end = start + len;
        // the lane's strips, and by square the cross sets the other way, which
        // say the letters that make the lane's word there and what its tiles
        // score
        let (board_strip, allowed_strip, words_allowed, words_scores, stride) = if down {
            (
                &working_buffer.transposed_board_tiles[start..end],
                &working_buffer.allowed_for_down_plays[start..end],
                &working_buffer.allowed_for_across_plays[lane..],
                &working_buffer.cross_score_for_across_plays[lane..],
                cols,
            )
        } else {
            (
                &board_snapshot.board_tiles[start..end],
                &working_buffer.allowed_for_across_plays[start..end],
                &working_buffer.allowed_for_down_plays[lane..],
                &working_buffer.cross_score_for_down_plays[lane..],
                rows,
            )
        };
        let (word_multipliers, tile_multipliers) = if down {
            (
                &working_buffer.remaining_word_multipliers_for_down_plays[start..end],
                &working_buffer.remaining_tile_multipliers_for_down_plays[start..end],
            )
        } else {
            (
                &working_buffer.remaining_word_multipliers_for_across_plays[start..end],
                &working_buffer.remaining_tile_multipliers_for_across_plays[start..end],
            )
        };
        let (perpendicular_word_multipliers, perpendicular_scores) = if down {
            (
                &working_buffer.perpendicular_word_multipliers_for_down_plays[start..end],
                &working_buffer.perpendicular_scores_for_down_plays[start..end],
            )
        } else {
            (
                &working_buffer.perpendicular_word_multipliers_for_across_plays[start..end],
                &working_buffer.perpendicular_scores_for_across_plays[start..end],
            )
        };
        let word_buffer = if down {
            &mut working_buffer.word_buffer_for_down_plays[start..end]
        } else {
            &mut working_buffer.word_buffer_for_across_plays[start..end]
        };
        // the plays on square idx of the word from word_start to word_end
        let mut play_at = |idx: usize, word_start: usize, word_end: usize| {
            let bits = allowed_strip[idx];
            if bits == 1 || (down && bits & 1 != 0) {
                return;
            }
            let mut fits = words_allowed[idx * stride] & letters & bits;
            let tile_value = value * i32::from(tile_multipliers[idx]);
            let score = (words_scores[idx * stride] + tile_value)
                * i32::from(word_multipliers[idx])
                + perpendicular_scores[idx]
                + tile_value * i32::from(perpendicular_word_multipliers[idx])
                + bonus;
            while fits != 0 {
                let letter = fits.trailing_zeros() as u8;
                fits &= fits - 1;
                // as in the walk, a play under the threshold found_place_move
                // last returned is not offered
                if score + leave_value < threshold {
                    continue;
                }
                word_buffer[idx] = letter | blank_bit;
                threshold = found_place_move(
                    down,
                    lane as i8,
                    word_start as i8,
                    &word_buffer[word_start..word_end],
                    equity::Equity::new(score),
                    equity::Equity::new(leave_value),
                )
                .raw();
            }
        };
        let mut idx = 0;
        while idx < len {
            if board_strip[idx] == 0 {
                idx += 1;
                continue;
            }
            let run_start = idx;
            while idx < len && board_strip[idx] != 0 {
                idx += 1;
            }
            if idx < len && (idx + 1 == len || board_strip[idx + 1] == 0) {
                play_at(idx, run_start, idx + 1);
            }
            if run_start > 0 {
                let mut word_start = run_start - 1;
                while word_start > 0 && board_strip[word_start - 1] != 0 {
                    word_start -= 1;
                }
                play_at(run_start - 1, word_start, idx);
            }
        }
    }
}

const PLACEMENT_BUCKETS: usize = 64;

// the most tiles a jumbled span's bound sorts on the stack; a span of more is
// filled unbounded
const JUMBLED_BOUND_TILES: usize = 16;

// the most squares a jumbled span's weights are kept for, by their place in it
// (fill_span's bound at each square); a longer span is bounded per multiset
// only
const JUMBLED_BOUND_SQUARES: usize = 32;

// range b is pairs[starts[b]..starts[b + 1]]; tops[b] is its highest equity.
// one copy for each kind of keys (T), so neither's inlining depends on the
// other's
#[inline]
fn bucket_by_equity<T: TablePlaces>(
    pairs: &mut Vec<(i32, u32)>,
    scratch: &mut Vec<(i32, u32)>,
    starts: &mut [u32; PLACEMENT_BUCKETS + 1],
    tops: &mut [i32; PLACEMENT_BUCKETS],
) {
    let mut min = i32::MAX;
    let mut max = i32::MIN;
    for &(equity, _) in pairs.iter() {
        min = min.min(equity);
        max = max.max(equity);
    }
    let range = max.abs_diff(min);
    let shift =
        (u32::BITS - range.leading_zeros()).saturating_sub(PLACEMENT_BUCKETS.trailing_zeros());
    let bucket = |equity: i32| (equity.abs_diff(min) >> shift) as usize;
    *starts = [0; PLACEMENT_BUCKETS + 1];
    *tops = [i32::MIN; PLACEMENT_BUCKETS];
    for &(equity, _) in pairs.iter() {
        let b = bucket(equity);
        starts[b + 1] += 1;
        tops[b] = tops[b].max(equity);
    }
    for b in 0..PLACEMENT_BUCKETS {
        starts[b + 1] += starts[b];
    }
    let mut next = [0u32; PLACEMENT_BUCKETS];
    next.copy_from_slice(&starts[..PLACEMENT_BUCKETS]);
    scratch.clear();
    scratch.resize(pairs.len(), (0, 0));
    for &pair in pairs.iter() {
        let b = bucket(pair.0);
        scratch[next[b] as usize] = pair;
        next[b] += 1;
    }
    std::mem::swap(pairs, scratch);
}

// a lane's spans as the shadow pass finds them with every letter on hand
// without limit, in its order: gen_place_placements_impl_u16's anchors and
// walks, with what the rack decides taken out (no square refuses a letter but a
// dead one, no count runs out, and no bound is made), each span's fixed part
// and multipliers (rcs_placements) recorded instead
struct RcsGeometry<'a> {
    kept: &'a mut RcsLane,
    multi_jumps: &'a [MultiJump],
    tiles: &'a [u8],
    allowed: &'a [u64],
    word_multipliers: &'a [i8],
    tile_multipliers: &'a [i8],
    perpendicular_word_multipliers: &'a [i8],
    perpendicular_scores: &'a [i32],
    single_tile_plays: bool,
    num_max_played: u8,
}

// a walk's state from its anchor: the board tiles' scores, the cross words',
// the word multiplier, the empty squares taken (a bit each) and how many, and
// whether a board tile is in it
#[derive(Clone, Copy)]
struct RcsWalk {
    board_score: i32,
    cross_score: i32,
    word_multiplier: i32,
    placed: u64,
    num_played: u8,
    crossed: bool,
}

// how many entries of a walk's list RcsGeometry's record and right copy when
// the walk holds no more (the rest dropped or never read)
const RCS_COPIED: usize = 8;

// whether a kept lane's input is the lane's: as arrays of the board's side when
// it is fixed, every element compared and folded without a branch
#[inline(always)]
fn same_lane<const D: usize, T: Copy + PartialEq>(kept: &[T], lane: &[T]) -> bool {
    if D != 0 && kept.len() == D && lane.len() == D {
        kept[..D]
            .iter()
            .zip(&lane[..D])
            .fold(true, |same, (a, b)| same & (a == b))
    } else {
        kept == lane
    }
}

// a lane's walks' taken squares by their multipliers under the walk's word
// multiplier, best first (each with the square): the left walk's, and a right
// walk's (the left walk's it starts from, then its own); and the lane's empty
// squares a cross word reaches, a bit each
struct RcsLists {
    left: [Taken; u64::BITS as usize],
    right: [Taken; u64::BITS as usize],
    hooks: u64,
}

impl RcsLists {
    #[inline(always)]
    fn new() -> Self {
        Self {
            left: [Taken::default(); u64::BITS as usize],
            right: [Taken::default(); u64::BITS as usize],
            hooks: 0,
        }
    }
}

impl RcsGeometry<'_> {
    // a span of the walk, its multipliers the walk's list
    #[inline(always)]
    fn record(&mut self, walk: &RcsWalk, list: &[Taken], anchor: i8, left: i8, right: i8) {
        let at = self.kept.multipliers.len();
        let n = walk.num_played as usize;
        self.kept.multipliers.extend_from_slice(&list[..RCS_COPIED]);
        if n > RCS_COPIED {
            self.kept
                .multipliers
                .extend_from_slice(&list[RCS_COPIED..n]);
        } else {
            self.kept.multipliers.truncate(at + n);
        }
        self.kept.spans.push(RcsSpan {
            anchor,
            left,
            right,
            num_played: walk.num_played,
            crossed: walk.crossed,
            fixed: walk.board_score * walk.word_multiplier + walk.cross_score,
            at: at as u32,
        });
    }

    // the walk's list once it took idx (having had word_multiplier before): the
    // square's multiplier put in its place, or, when the square changed the
    // word multiplier, every taken square's again, sorted
    #[inline(always)]
    fn list_took(
        &self,
        list: &mut [Taken; u64::BITS as usize],
        walk: &RcsWalk,
        word_multiplier: i32,
        idx: i8,
    ) {
        let multiplier_of = |i: usize| {
            i32::from(self.tile_multipliers[i])
                * (walk.word_multiplier + i32::from(self.perpendicular_word_multipliers[i]))
        };
        let n = walk.num_played as usize;
        if walk.word_multiplier != word_multiplier {
            let mut placed = walk.placed;
            for entry in &mut list[..n] {
                let i = placed.trailing_zeros() as usize;
                placed &= placed - 1;
                *entry = Taken::new(multiplier_of(i), i as u8);
            }
            list[..n].sort_unstable_by_key(|taken| std::cmp::Reverse(taken.multiplier()));
            return;
        }
        let multiplier = multiplier_of(idx as usize);
        let mut j = n - 1;
        while j > 0 && list[j - 1].multiplier() < multiplier {
            list[j] = list[j - 1];
            j -= 1;
        }
        list[j] = Taken::new(multiplier, idx as u8);
    }

    // the square idx taken by a tile: false for a dead one (the walk ends)
    #[inline(always)]
    fn take(&self, walk: &mut RcsWalk, is_unique: &mut bool, idx: i8) -> bool {
        let bits = self.allowed[idx as usize];
        if bits & 1 == 0 {
            *is_unique = true;
        } else if bits == 1 {
            return false;
        }
        walk.cross_score += self.perpendicular_scores[idx as usize];
        walk.placed |= 1 << idx;
        walk.num_played += 1;
        walk.word_multiplier *= i32::from(self.word_multipliers[idx as usize]);
        true
    }

    #[inline(always)]
    fn right(
        &mut self,
        lists: &mut RcsLists,
        mut walk: RcsWalk,
        mut is_unique: bool,
        anchor: i8,
        idx_left: i8,
        rightmost: i8,
    ) {
        let n = walk.num_played as usize;
        lists.right[..RCS_COPIED].copy_from_slice(&lists.left[..RCS_COPIED]);
        if n > RCS_COPIED {
            lists.right[RCS_COPIED..n].copy_from_slice(&lists.left[RCS_COPIED..n]);
        }
        let mut idx = anchor + 1;
        loop {
            if idx < rightmost {
                let multi_jump = &self.multi_jumps[idx as usize];
                walk.board_score += multi_jump.right_score;
                walk.crossed |= multi_jump.right_idx != idx;
                idx = multi_jump.right_idx;
            }
            if idx > anchor + 1 && walk.num_played > u8::from(!is_unique) && idx - idx_left >= 2 {
                self.record(&walk, &lists.right, anchor, idx_left, idx);
            }
            if walk.num_played >= self.num_max_played || idx >= rightmost {
                break;
            }
            let word_multiplier = walk.word_multiplier;
            if !self.take(&mut walk, &mut is_unique, idx) {
                break;
            }
            self.list_took(&mut lists.right, &walk, word_multiplier, idx);
            idx += 1;
        }
    }

    #[inline(always)]
    fn from(&mut self, lists: &mut RcsLists, anchor: i8, leftmost: i8, rightmost: i8) {
        let mut walk = RcsWalk {
            board_score: 0,
            cross_score: 0,
            word_multiplier: 1,
            placed: 0,
            num_played: 0,
            crossed: false,
        };
        let mut is_unique = self.single_tile_plays;
        let mut idx = anchor;
        loop {
            if idx >= leftmost {
                let multi_jump = &self.multi_jumps[idx as usize];
                walk.board_score += multi_jump.left_score;
                walk.crossed |= multi_jump.left_idx != idx;
                idx = multi_jump.left_idx;
            }
            if walk.num_played > u8::from(!is_unique) && anchor - idx >= 2 {
                self.record(&walk, &lists.left, anchor, idx + 1, anchor + 1);
            }
            if walk.num_played >= self.num_max_played {
                break;
            }
            if idx < anchor {
                self.right(lists, walk, is_unique, anchor, idx + 1, rightmost);
            }
            let word_multiplier = walk.word_multiplier;
            if idx < leftmost || !self.take(&mut walk, &mut is_unique, idx) {
                break;
            }
            self.list_took(&mut lists.left, &walk, word_multiplier, idx);
            idx -= 1;
        }
    }

    // the shadow pass's anchors, in its order (a lane holds at most 64 squares,
    // so a walk at most 64 taken); the lists are the caller's, made once for
    // every lane (a walk reads only the entries it wrote), their hooks set here
    #[inline(never)]
    fn find(&mut self, lists: &mut RcsLists) {
        assert!(self.tiles.len() <= u64::BITS as usize);
        lists.hooks = self
            .allowed
            .iter()
            .enumerate()
            .fold(0, |hooks, (i, &bits)| hooks | (bits & 1) << i);
        let mut leftmost = self.tiles.len() as i8;
        loop {
            let mut rightmost = leftmost;
            while leftmost > 0 && self.tiles[leftmost as usize - 1] == 0 {
                leftmost -= 1;
            }
            if leftmost > 0 {
                self.from(lists, leftmost - 1, 0, rightmost);
            }
            {
                let leftmost = leftmost + i8::from(leftmost > 0 && leftmost < rightmost);
                for anchor in (leftmost..rightmost).rev() {
                    let bits = self.allowed[anchor as usize];
                    if bits & 1 != 0 {
                        if rightmost - leftmost < 2 {
                            break;
                        }
                        if bits != 1 {
                            self.from(lists, anchor, leftmost, rightmost);
                        }
                        rightmost = anchor;
                    }
                }
            }
            loop {
                leftmost -= 1;
                if leftmost <= 1 {
                    return;
                }
                if self.tiles[leftmost as usize] == 0 {
                    break;
                }
            }
        }
    }
}

// the table walk's placements of a turn, a span each, by span bounds that need
// no shadow pass: a lane's spans and what of each bound needs no rack are kept
// in rcs_lanes until the lane's inputs change, then found by the shadow pass
// with every letter on hand without limit; a span the rack can fill (no
// crossing square fits none of its tiles, unless it holds a blank) is bounded
// by its fixed part, the rack's best scores on its multipliers, best first, and
// the best leave of the tiles it plays. No play scores more: its tiles on any
// of the span's squares score at most the best on the best. The span lengths a
// rack's tiles cannot fill alone and the key check are asked as the shadow pass
// asks them.
#[inline(always)]
fn rcs_placements<
    const D: usize,
    W: LaneBits,
    T: TablePlaces,
    N: kwg::Node,
    L: kwg::Node,
    G: LaneBits,
    const BAG_TERM: bool,
    S: BoardStore,
>(
    board_snapshot: &BoardSnapshot<'_, N, L>,
    working_buffer: &mut WorkingBuffer<G, BAG_TERM, S>,
    dim: &matrix::Dim,
    num_max_played: u8,
    feasible_lengths: u64,
    min_played: u8,
    found_placements: &mut Vec<PossiblePlacement>,
) {
    let alphabet = board_snapshot.game_config.alphabet();
    let rack_size = board_snapshot.game_config.rack_size() as usize;
    let num_lanes = dim.rows as usize + dim.cols as usize;
    if working_buffer.rcs_lanes.len() != num_lanes {
        working_buffer.rcs_lanes.clear();
        working_buffer
            .rcs_lanes
            .resize(num_lanes, RcsLane::default());
    }
    // the kept lanes the board's changes since the last look may have moved: a
    // lane whose tiles changed (and with them its multipliers), and one that
    // crosses a changed strip where its square there no longer reads as kept
    // (that strip's cross sets, and from them its perpendicular scores and
    // multipliers, were made again). Every other kept lane keeps its spans
    // with no look at its inputs
    let mut stale_rows = working_buffer.rcs_stale_rows;
    stale_rows |= working_buffer.rcs_pending_rows;
    let mut stale_cols = working_buffer.rcs_stale_cols;
    stale_cols |= working_buffer.rcs_pending_cols;
    {
        let rows = dim.rows as usize;
        let cols = dim.cols as usize;
        let lanes = &working_buffer.rcs_lanes;
        for col in 0..cols {
            if working_buffer.rcs_pending_cols & (G::from(1) << col as u32) == G::from(0) {
                continue;
            }
            for (row, kept) in lanes[..rows].iter().enumerate() {
                let bit = G::from(1) << row as u32;
                if stale_rows & bit != G::from(0) {
                    continue;
                }
                let idx = row * cols + col;
                if !kept.valid
                    || kept.allowed[col] != working_buffer.allowed_for_across_plays[idx]
                    || kept.perpendicular_scores[col]
                        != working_buffer.perpendicular_scores_for_across_plays[idx]
                    || kept.perpendicular_word_multipliers[col]
                        != working_buffer.perpendicular_word_multipliers_for_across_plays[idx]
                {
                    stale_rows |= bit;
                }
            }
        }
        for row in 0..rows {
            if working_buffer.rcs_pending_rows & (G::from(1) << row as u32) == G::from(0) {
                continue;
            }
            for (col, kept) in lanes[rows..rows + cols].iter().enumerate() {
                let bit = G::from(1) << col as u32;
                if stale_cols & bit != G::from(0) {
                    continue;
                }
                let idx = col * rows + row;
                if !kept.valid
                    || kept.allowed[row] != working_buffer.allowed_for_down_plays[idx]
                    || kept.perpendicular_scores[row]
                        != working_buffer.perpendicular_scores_for_down_plays[idx]
                    || kept.perpendicular_word_multipliers[row]
                        != working_buffer.perpendicular_word_multipliers_for_down_plays[idx]
                {
                    stale_cols |= bit;
                }
            }
        }
    }
    working_buffer.rcs_pending_rows = G::from(0);
    working_buffer.rcs_pending_cols = G::from(0);
    // the lanes are checked: span sums kept for a lane found again, or marked
    // to be, are its old inputs', so filled again (fill_span_lane)
    working_buffer.span_look_turn = working_buffer.span_turn;
    for (at, kept) in working_buffer.rcs_lanes.iter().enumerate() {
        let stale = if at >= dim.rows as usize {
            stale_cols & (G::from(1) << (at - dim.rows as usize) as u32)
        } else {
            stale_rows & (G::from(1) << at as u32)
        } != G::from(0);
        if stale || !kept.valid {
            working_buffer.span_lanes[at].kept = false;
        }
    }
    let rack_bits = working_buffer.rack_bits;
    // the rack's tiles (a bit each) and their scores, for the caps
    let mut rack_tiles = [(0u64, 0i32); u64::BITS as usize];
    let mut num_rack_tiles = 0;
    let mut bits = rack_bits;
    while bits != 0 {
        let tile = bits.trailing_zeros() as u8;
        bits &= bits - 1;
        rack_tiles[num_rack_tiles] = (1 << tile, alphabet.scaled_score(tile).raw());
        num_rack_tiles += 1;
    }
    let rack_tiles = &rack_tiles[..num_rack_tiles];
    // an open square can take the rack's highest scoring tile.
    //
    // 64 caps fit in an i32 even at the highest face score.
    let unrestricted_cap = working_buffer
        .descending_scores
        .first()
        .copied()
        .unwrap_or(0)
        .max(0);
    let mut caps = [unrestricted_cap; u64::BITS as usize];
    // the walks' lists of a lane found again (RcsGeometry::find), made once a
    // turn
    let mut lists = RcsLists::new();
    for at in 0..num_lanes {
        let down = at >= dim.rows as usize;
        let lane = if down { at - dim.rows as usize } else { at } as i8;
        let in_lanes = if down {
            working_buffer.lane_cols
        } else {
            working_buffer.lane_rows
        } & (G::from(1) << lane as u32);
        if in_lanes == G::from(0) {
            continue;
        }
        let len = if down { dim.rows } else { dim.cols } as usize;
        let start = lane as usize * len;
        let end = start + len;
        // the key check is asked only of a span through a board tile, so a lane
        // with none needs no span sums for it (a walk fills its own)
        let holds_tiles = if down {
            &working_buffer.transposed_board_tiles[start..end]
        } else {
            &board_snapshot.board_tiles[start..end]
        }
        .iter()
        .any(|&tile| tile != 0);
        let check = if holds_tiles {
            fill_key_check::<D, W, T, _, _, _, _, _>(
                working_buffer,
                board_snapshot,
                dim,
                down,
                lane,
            )
        } else {
            None
        };
        let kept = &mut working_buffer.rcs_lanes[at];
        let tiles = if down {
            &working_buffer.transposed_board_tiles[start..end]
        } else {
            &board_snapshot.board_tiles[start..end]
        };
        let allowed = if down {
            &working_buffer.allowed_for_down_plays[start..end]
        } else {
            &working_buffer.allowed_for_across_plays[start..end]
        };
        let (word_multipliers, tile_multipliers, perpendicular_word_multipliers) = if down {
            (
                &working_buffer.remaining_word_multipliers_for_down_plays[start..end],
                &working_buffer.remaining_tile_multipliers_for_down_plays[start..end],
                &working_buffer.perpendicular_word_multipliers_for_down_plays[start..end],
            )
        } else {
            (
                &working_buffer.remaining_word_multipliers_for_across_plays[start..end],
                &working_buffer.remaining_tile_multipliers_for_across_plays[start..end],
                &working_buffer.perpendicular_word_multipliers_for_across_plays[start..end],
            )
        };
        let perpendicular_scores = if down {
            &working_buffer.perpendicular_scores_for_down_plays[start..end]
        } else {
            &working_buffer.perpendicular_scores_for_across_plays[start..end]
        };
        let lane_bit = G::from(1) << lane as u32;
        let stale = if down { stale_cols } else { stale_rows } & lane_bit != G::from(0);
        // a lane left unmarked has the inputs its spans were found from
        debug_assert!(
            stale
                || !kept.valid
                || (same_lane::<D, _>(&kept.tiles, tiles)
                    && same_lane::<D, _>(&kept.word_multipliers, word_multipliers)
                    && same_lane::<D, _>(&kept.tile_multipliers, tile_multipliers)
                    && same_lane::<D, _>(
                        &kept.perpendicular_word_multipliers,
                        perpendicular_word_multipliers,
                    )
                    && same_lane::<D, _>(&kept.perpendicular_scores, perpendicular_scores)
                    && kept
                        .allowed
                        .iter()
                        .zip(allowed)
                        .all(|(&kept_bits, &bits)| kept_bits == bits)),
            "a kept lane's inputs changed unmarked"
        );
        if stale || !kept.valid {
            if down {
                stale_cols = stale_cols & !lane_bit;
            } else {
                stale_rows = stale_rows & !lane_bit;
            }
            let mut jumps = std::mem::take(&mut kept.jumps);
            fill_multi_jumps::<D>(alphabet, tiles, &mut jumps);
            kept.spans.clear();
            kept.multipliers.clear();
            RcsGeometry {
                kept,
                multi_jumps: &jumps,
                tiles,
                allowed,
                word_multipliers,
                tile_multipliers,
                perpendicular_word_multipliers,
                perpendicular_scores,
                single_tile_plays: !down,
                num_max_played: rack_size as u8,
            }
            .find(&mut lists);
            kept.jumps = jumps;
            kept.hooks = lists.hooks;
            kept.tiles.clear();
            kept.tiles.extend_from_slice(tiles);
            kept.allowed.clear();
            kept.allowed.extend_from_slice(allowed);
            kept.word_multipliers.clear();
            kept.word_multipliers.extend_from_slice(word_multipliers);
            kept.tile_multipliers.clear();
            kept.tile_multipliers.extend_from_slice(tile_multipliers);
            kept.perpendicular_word_multipliers.clear();
            kept.perpendicular_word_multipliers
                .extend_from_slice(perpendicular_word_multipliers);
            kept.perpendicular_scores.clear();
            kept.perpendicular_scores
                .extend_from_slice(perpendicular_scores);
            kept.valid = true;
            working_buffer.rcs_rev += 1;
            kept.stamp = (working_buffer.rcs_owner, working_buffer.rcs_rev);
        }
        let check = check.map(|(source, prefix_at)| KeyCheck {
            source,
            subracks: &working_buffer.subracks,
            by_played: &working_buffer.subracks_by_played,
            prefix: &working_buffer.span_prefix[prefix_at..prefix_at + len + 1],
            turn: working_buffer.exist_turn,
            memo: &mut working_buffer.exist_memo,
            min_played,
        });
        rcs_lane_placements(
            kept,
            allowed,
            &mut caps,
            check,
            RcsRack {
                bits: rack_bits,
                tiles: rack_tiles,
                unrestricted_cap,
                descending_scores: &working_buffer.descending_scores,
                best_leave_values: &working_buffer.best_leave_values,
                num_max_played,
                feasible_lengths,
            },
            RcsPlacementOutput {
                down,
                lane,
                found_placements,
            },
        );
    }
    // a lane this turn passed by keeps its mark for the turn that walks it
    working_buffer.rcs_stale_rows = stale_rows;
    working_buffer.rcs_stale_cols = stale_cols;
}

struct RcsRack<'a> {
    bits: u64,
    tiles: &'a [(u64, i32)],
    unrestricted_cap: i32,
    descending_scores: &'a [i32],
    best_leave_values: &'a [i32],
    num_max_played: u8,
    feasible_lengths: u64,
}

struct RcsPlacementOutput<'a> {
    down: bool,
    lane: i8,
    found_placements: &'a mut Vec<PossiblePlacement>,
}

#[inline(never)]
fn rcs_lane_placements(
    kept: &RcsLane,
    allowed: &[u64],
    caps: &mut [i32; u64::BITS as usize],
    mut check: Option<KeyCheck<'_>>,
    RcsRack {
        bits: rack_bits,
        tiles: rack_tiles,
        unrestricted_cap,
        descending_scores,
        best_leave_values,
        num_max_played,
        feasible_lengths,
    }: RcsRack<'_>,
    RcsPlacementOutput {
        down,
        lane,
        found_placements,
    }: RcsPlacementOutput<'_>,
) {
    // the crossing squares none of the rack's tiles fit (with a blank on hand,
    // none: a cross set's bit 0 is the blank's), and each crossing square's
    // cap, the best score of the rack's tiles that fit it, each tile's kept or
    // dropped by a select (a square no cross word reaches takes any tile)
    let mut unfillable = 0u64;
    let mut hooks = kept.hooks;
    while hooks != 0 {
        let idx = hooks.trailing_zeros() as usize;
        hooks &= hooks - 1;
        let fits = allowed[idx] & rack_bits;
        if fits == 0 {
            unfillable |= 1 << idx;
            continue;
        }
        caps[idx] = rack_tiles
            .iter()
            .map(|&(bit, score)| if fits & bit != 0 { score } else { 0 })
            .fold(0, i32::max);
    }
    for span in &kept.spans {
        // a span cannot be filled if the first unfillable hook at or after its
        // left edge falls inside it.
        if span.num_played > num_max_played
            || (unfillable >> span.left as u32).trailing_zeros() < (span.right - span.left) as u32
            || (!span.crossed && feasible_lengths & 1 << span.num_played == 0)
            || (span.crossed && check.refuses(span.left, span.right, span.num_played))
        {
            continue;
        }
        let multipliers =
            &kept.multipliers[span.at as usize..span.at as usize + span.num_played as usize];
        let mut bound = span.fixed + best_leave_values[span.num_played as usize];
        let mut best_scores = 0i32;
        let mut capped = 0i32;
        let mut least = 0i32;
        for (&score, &taken) in descending_scores.iter().zip(multipliers) {
            best_scores += score;
            capped += caps[taken.idx() % caps.len()];
            let next_least = best_scores.min(capped);
            bound += taken.multiplier() * (next_least - least);
            least = next_least;
        }
        found_placements.push(PossiblePlacement {
            num_played: span.num_played,
            down,
            lane,
            anchor: span.anchor,
            leftmost: span.left,
            rightmost: span.right,
            best_possible_equity: bound,
            kept_at: span.at,
        });
    }
    let mut hooks = kept.hooks;
    while hooks != 0 {
        caps[hooks.trailing_zeros() as usize] = unrestricted_cap;
        hooks &= hooks - 1;
    }
}

#[inline]
fn kurnia_gen_place_moves_iter<
    'a,
    const D: usize,
    W: LaneBits,
    T: 'a + TablePlaces,
    FoundPlaceMove: 'a + FnMut(bool, i8, i8, &[u8], equity::Equity, equity::Equity) -> equity::Equity,
    CanAccept: 'a + Fn(equity::Equity) -> bool,
    CurrentThreshold: 'a + Fn() -> equity::Equity,
    N: kwg::Node,
    L: kwg::Node,
    G: LaneBits,
    const BAG_TERM: bool,
    S: BoardStore,
>(
    p: KurniaIterParams<'a, (), FoundPlaceMove, CanAccept, CurrentThreshold, N, L, G, BAG_TERM, S>,
) -> impl 'a + Iterator {
    let KurniaIterParams {
        runs: (),
        board_snapshot,
        working_buffer,
        multi_leaves,
        mut found_place_move,
        can_accept,
        current_threshold,
    } = p;
    let game_config = &board_snapshot.game_config;
    let board_layout = game_config.board_layout();
    let fixed;
    let dim = if D != 0 {
        assert!(board_layout.dim().rows as usize == D && board_layout.dim().cols as usize == D);
        fixed = square_dim::<D>();
        &fixed
    } else {
        board_layout.dim()
    };
    let max_rack_size = game_config.rack_size();
    let num_max_played = max_rack_size.min(working_buffer.num_tiles_on_rack);
    // the key check's answers are asked afresh each generation of this arm, and
    // its memo cleared when the generation number wraps
    working_buffer.exist_turn = working_buffer.exist_turn.wrapping_add(1);
    if working_buffer.exist_turn == 0 {
        working_buffer.exist_memo.fill(ExistSlot::new());
        working_buffer.exist_turn = 1;
    }
    let min_played = EXIST_MIN_PLAYED.min(num_max_played);

    let area = (dim.rows as isize * dim.cols as isize) as usize;
    let dirty_rows = working_buffer.dirty_rows;
    let dirty_cols = working_buffer.dirty_cols;
    for col in 0..dim.cols {
        if dirty_cols & (G::from(1) << col as u32) != G::from(0) {
            let strip_range_start = (col as isize * dim.rows as isize) as usize;
            let strip_range_end = strip_range_start + dim.rows as usize;
            gen_cross_set::<T, _, _>(
                board_snapshot,
                &working_buffer.transposed_board_tiles[strip_range_start..strip_range_end],
                CrossSetOutput {
                    allowed: &mut working_buffer.allowed_for_across_plays,
                    cross_scores: &mut working_buffer.cross_score_for_across_plays,
                    output_strider: dim.down(col),
                },
                &mut working_buffer.cross_set_buffer_for_across_plays
                    [strip_range_start..strip_range_end],
                &mut working_buffer.cached_cross_set_for_across_plays
                    [strip_range_start..strip_range_end],
                &mut working_buffer.used_letters_tally,
                &mut working_buffer.woadwage.stack,
            );
        }
    }
    let transposed_dim = matrix::Dim {
        rows: dim.cols,
        cols: dim.rows,
    };

    for row in 0..dim.rows {
        if dirty_rows & (G::from(1) << row as u32) != G::from(0) {
            let strip_range_start = (row as isize * dim.cols as isize) as usize;
            let strip_range_end = strip_range_start + dim.cols as usize;
            gen_cross_set::<T, _, _>(
                board_snapshot,
                &board_snapshot.board_tiles[strip_range_start..strip_range_end],
                CrossSetOutput {
                    allowed: &mut working_buffer.allowed_for_down_plays,
                    cross_scores: &mut working_buffer.cross_score_for_down_plays,
                    output_strider: transposed_dim.down(row),
                },
                &mut working_buffer.cross_set_buffer_for_down_plays
                    [strip_range_start..strip_range_end],
                &mut working_buffer.cached_cross_set_for_down_plays
                    [strip_range_start..strip_range_end],
                &mut working_buffer.used_letters_tally,
                &mut working_buffer.woadwage.stack,
            );
        }
    }

    if dirty_rows != G::from(0) || dirty_cols != G::from(0) {
        working_buffer.prev_board_tiles[..area]
            .copy_from_slice(&board_snapshot.board_tiles[..area]);
        working_buffer.prev_tile_count = working_buffer.num_tiles_on_board;
        working_buffer.prev_tile_score = working_buffer.board_tile_score;
    }
    working_buffer.init_after_cross_sets::<D, _, _>(board_snapshot, dirty_cols, dirty_rows);
    if working_buffer.num_tiles_on_board == 0 {
        working_buffer.activate_star(board_layout);
    }

    let feasible_lengths = feasible_lengths_of_subracks::<T>(
        board_snapshot.anagrams,
        board_snapshot.rack_lengths,
        &working_buffer.rack_tally,
        &working_buffer.subracks,
        &working_buffer.subracks_by_played,
    );
    let mut found_placements = std::mem::take(&mut working_buffer.found_placements);
    found_placements.clear();
    let mut placement_order = std::mem::take(&mut working_buffer.placement_order);
    placement_order.clear();
    let bounds_stride = dim.rows.max(dim.cols) as usize + 1;
    working_buffer.anchor_left_bounds.clear();
    // the kept lanes' spans (whatever the keys, on lanes of at most 64
    // squares), else the shadow pass's, lane by lane
    let kept_lanes = dim.rows.max(dim.cols) as u32 <= u64::BITS;
    if kept_lanes {
        rcs_placements::<D, W, T, _, _, _, _, _>(
            board_snapshot,
            working_buffer,
            dim,
            num_max_played,
            feasible_lengths,
            min_played,
            &mut found_placements,
        );
    }
    for row in 0..dim.rows {
        if kept_lanes || working_buffer.lane_rows & (G::from(1) << row as u32) == G::from(0) {
            continue;
        }
        let lane_len = dim.cols as usize;
        if !(found_placements.room_for(&working_buffer.room, lane_len)
            && (working_buffer.anchor_left_bounds)
                .room_for(&working_buffer.room, lane_len * 2 * bounds_stride))
        {
            continue;
        }
        let strip_range_start = (row as isize * dim.cols as isize) as usize;
        let strip_range_end = strip_range_start + dim.cols as usize;
        let check = fill_key_check::<D, W, T, _, _, _, _, _>(
            working_buffer,
            board_snapshot,
            dim,
            false,
            row,
        );
        shadow_lane::<D, W, _, _>(
            &mut GenPlacePlacementsParams {
                board_strip: &board_snapshot.board_tiles[strip_range_start..strip_range_end],
                alphabet: board_snapshot.game_config.alphabet(),
                rack_tally: &mut working_buffer.rack_tally,
                used_tile_scores_shadowl: &mut working_buffer.used_tile_scores_shadowl,
                used_tile_scores_shadowr: &mut working_buffer.used_tile_scores_shadowr,
                allowed_strip: &working_buffer.allowed_for_across_plays
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
                room: &working_buffer.room,
                best_leave_values: &working_buffer.best_leave_values,
                num_max_played,
                rack_tally_shadowl: &mut working_buffer.rack_tally_shadowl,
                rack_tally_shadowr: &mut working_buffer.rack_tally_shadowr,
                span_out: &mut working_buffer.span_out,
                per_span: true,
                left_bounds: &mut working_buffer.anchor_left_bounds,
                bounds_stride,
                check: check.map(|(source, at)| KeyCheck {
                    source,
                    subracks: &working_buffer.subracks,
                    by_played: &working_buffer.subracks_by_played,
                    prefix: &working_buffer.span_prefix[at..at + dim.cols as usize + 1],
                    turn: working_buffer.exist_turn,
                    memo: &mut working_buffer.exist_memo,
                    min_played,
                }),
            },
            true,
            false,
            |anchor: i8, leftmost: i8, rightmost: i8, best_possible_equity: i32, num_played: u8| {
                found_placements.push_in(
                    &working_buffer.room,
                    PossiblePlacement {
                        num_played,
                        down: false,
                        lane: row,
                        anchor,
                        leftmost,
                        rightmost,
                        best_possible_equity,
                        kept_at: NO_KEPT,
                    },
                );
            },
        );
    }
    for col in 0..dim.cols {
        if kept_lanes || working_buffer.lane_cols & (G::from(1) << col as u32) == G::from(0) {
            continue;
        }
        let lane_len = dim.rows as usize;
        if !(found_placements.room_for(&working_buffer.room, lane_len)
            && (working_buffer.anchor_left_bounds)
                .room_for(&working_buffer.room, lane_len * 2 * bounds_stride))
        {
            continue;
        }
        let strip_range_start = (col as isize * dim.rows as isize) as usize;
        let strip_range_end = strip_range_start + dim.rows as usize;
        let check = fill_key_check::<D, W, T, _, _, _, _, _>(
            working_buffer,
            board_snapshot,
            dim,
            true,
            col,
        );
        shadow_lane::<D, W, _, _>(
            &mut GenPlacePlacementsParams {
                board_strip: &working_buffer.transposed_board_tiles
                    [strip_range_start..strip_range_end],
                alphabet: board_snapshot.game_config.alphabet(),
                rack_tally: &mut working_buffer.rack_tally,
                used_tile_scores_shadowl: &mut working_buffer.used_tile_scores_shadowl,
                used_tile_scores_shadowr: &mut working_buffer.used_tile_scores_shadowr,
                allowed_strip: &working_buffer.allowed_for_down_plays
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
                room: &working_buffer.room,
                best_leave_values: &working_buffer.best_leave_values,
                num_max_played,
                rack_tally_shadowl: &mut working_buffer.rack_tally_shadowl,
                rack_tally_shadowr: &mut working_buffer.rack_tally_shadowr,
                span_out: &mut working_buffer.span_out,
                per_span: true,
                left_bounds: &mut working_buffer.anchor_left_bounds,
                bounds_stride,
                check: check.map(|(source, at)| KeyCheck {
                    source,
                    subracks: &working_buffer.subracks,
                    by_played: &working_buffer.subracks_by_played,
                    prefix: &working_buffer.span_prefix[at..at + dim.rows as usize + 1],
                    turn: working_buffer.exist_turn,
                    memo: &mut working_buffer.exist_memo,
                    min_played,
                }),
            },
            false,
            false,
            |anchor: i8, leftmost: i8, rightmost: i8, best_possible_equity: i32, num_played: u8| {
                found_placements.push_in(
                    &working_buffer.room,
                    PossiblePlacement {
                        num_played,
                        down: true,
                        lane: col,
                        anchor,
                        leftmost,
                        rightmost,
                        best_possible_equity,
                        kept_at: NO_KEPT,
                    },
                );
            },
        );
    }
    if !placement_order.room_for(&working_buffer.room, found_placements.len()) {
        found_placements.clear();
    }
    placement_order.extend(
        found_placements
            .iter()
            .enumerate()
            .map(|(i, p)| (p.best_possible_equity, i as u32)),
    );
    let mut starts = [0u32; PLACEMENT_BUCKETS + 1];
    let mut tops = [i32::MIN; PLACEMENT_BUCKETS];
    let mut bucket = PLACEMENT_BUCKETS;
    bucket_by_equity::<T>(
        &mut placement_order,
        &mut working_buffer.placement_scratch,
        &mut starts,
        &mut tops,
    );
    // where the bucket being popped starts; the buckets are popped highest
    // first, each in no order, since the threshold only rises: a placement it
    // refuses is passed over, and the walk ends at a bucket whose highest
    // equity it refuses. One call of the closure walks every placement (its one
    // caller takes no item)
    let mut bucket_from = placement_order.len();
    working_buffer.span_memo.start_turn(found_placements.len());
    working_buffer.found_placements = found_placements;
    working_buffer.placement_order = placement_order;
    std::iter::from_fn(
        #[inline(never)]
        move || -> Option<()> {
            loop {
                let len = working_buffer.placement_order.len();
                if len == bucket_from {
                    while bucket > 0 {
                        bucket -= 1;
                        if (starts[bucket] as usize) < len {
                            break;
                        }
                    }
                    bucket_from = starts[bucket] as usize;
                    if len == 0 || !can_accept(equity::Equity::new(tops[bucket])) {
                        working_buffer.placement_order.clear();
                        return None;
                    }
                }
                let (equity, idx) = working_buffer.placement_order.pop()?;
                if !can_accept(equity::Equity::new(equity)) {
                    continue;
                }
                {
                    let placement = working_buffer.found_placements[idx as usize];
                    gen_place_moves_at::<D, W, T, _, _, _, _, _, _>(GenPlaceMovesAtParams {
                        runs: (),
                        board_snapshot,
                        working_buffer,
                        multi_leaves,
                        placement: &placement,
                        num_max_played,
                        threshold: current_threshold().raw(),
                        found_place_move:
                            &mut |down: bool,
                                  lane: i8,
                                  idx: i8,
                                  word: &[u8],
                                  score: equity::Equity,
                                  leave_value: equity::Equity| {
                                let this_best = score.raw() + leave_value.raw();
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
                }
            }
        },
    )
}

// a line of the board as the word prune walks it: its tiles, a blank's letter
// with 0x80 set and 0 where empty, and by square the letters a word along it
// may put there, a bit (1 << letter) each
struct PruneLine<'a, N: kwg::Node> {
    kwg: &'a kwg::Kwg<N>,
    tiles: &'a [u8],
    takes: &'a [u64],
    available: &'a mut [u8], // the tiles not on the board, a blank at 0
    letters: &'a mut [u8],   // the line's letters as the walk placed them
    seen: &'a mut [u64],     // by square, the letters of the words found on it
    found: &'a mut FoundWords,
    bits: &'a mut Vec<u64>, // by word of found, whether this walk found it
    memory: Option<&'a endgame::MemoryLimit>, // found's and bits' limit
}

// every word the line can take that holds one of its tiles at least, its empty
// squares filled from the tiles not on the board, a blank as any letter, each
// with a letter the square takes: each place a word fits adds it to found
// (once for all the places it fits), marks it in bits, and adds its letters on
// the empty squares to seen
fn prune_line<N: kwg::Node>(line: &mut PruneLine<'_, N>) {
    for (letter, &tile) in line.letters.iter_mut().zip(line.tiles) {
        *letter = tile & 0x7f;
    }
    line.seen.fill(0);

    struct Env<'a, 'b, N: kwg::Node> {
        line: &'b mut PruneLine<'a, N>,
        anchor: i8,
        rightmost: i8,
        num_played: i8,
        idx_left: i8,
    }

    #[inline(always)]
    fn record<N: kwg::Node>(env: &mut Env<'_, '_, N>, idx_left: i8, idx_right: i8) {
        let line = &mut *env.line;
        let span = idx_left as usize..idx_right as usize;
        if let Some(index) = line.found.insert(&line.letters[span.clone()], line.memory) {
            line.found.mark(line.bits, index, line.memory);
        }
        for idx in span {
            if line.tiles[idx] == 0 {
                line.seen[idx] |= 1 << line.letters[idx];
            }
        }
    }

    // the tally a letter put on the empty square at idx comes from: its own
    // tile's, else a blank's; None when the square does not take it or neither
    // is left
    #[inline(always)]
    fn take_from<N: kwg::Node>(line: &PruneLine<'_, N>, idx: i8, tile: u8) -> Option<usize> {
        if tile == 0 || line.takes[idx as usize] & (1 << tile) == 0 {
            None
        } else if line.available[tile as usize] > 0 {
            Some(tile as usize)
        } else if line.available[0] > 0 {
            Some(0)
        } else {
            None
        }
    }

    fn play_right<N: kwg::Node>(env: &mut Env<'_, '_, N>, mut p: i32, mut idx: i8) {
        // tail-recurse placing current sequence of tiles
        while idx < env.rightmost {
            let b = env.line.tiles[idx as usize];
            if b == 0 {
                break;
            }
            p = env.line.kwg.seek(p, b & 0x7f);
            if p <= 0 {
                return;
            }
            idx += 1;
        }
        let node = env.line.kwg.node_in_run(p);
        if idx > env.anchor + 1 && idx - env.idx_left >= 2 && node.accepts() {
            record(env, env.idx_left, idx);
        }

        if idx < env.rightmost {
            p = node.arc_index();
            if p <= 0 {
                return;
            }
            loop {
                let node = env.line.kwg.node_in_run(p);
                let tile = node.tile();
                if let Some(from) = take_from(env.line, idx, tile) {
                    env.line.available[from] -= 1;
                    // not blanked for kwg.
                    env.line.letters[idx as usize] = tile;
                    play_right(env, p, idx + 1);
                    env.line.available[from] += 1;
                }
                if node.is_end() {
                    break;
                }
                p += 1;
            }
        }
    }

    fn play_left<N: kwg::Node>(env: &mut Env<'_, '_, N>, mut p: i32, mut idx: i8) {
        // tail-recurse placing current sequence of tiles
        while idx >= 0 {
            let b = env.line.tiles[idx as usize];
            if b == 0 {
                break;
            }
            p = env.line.kwg.seek(p, b & 0x7f);
            if p <= 0 {
                return;
            }
            idx -= 1;
        }
        let mut node = env.line.kwg.node_in_run(p);
        if env.num_played > 0 && env.anchor - idx >= 2 && node.accepts() {
            record(env, idx + 1, env.anchor + 1);
        }

        p = node.arc_index();
        if p <= 0 {
            return;
        }

        let mut turnaround_p = p;
        loop {
            node = env.line.kwg.node_in_run(turnaround_p);
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
                let node = env.line.kwg.node_in_run(p);
                let tile = node.tile();
                if let Some(from) = take_from(env.line, idx, tile) {
                    env.line.available[from] -= 1;
                    env.num_played += 1;
                    // not blanked for kwg.
                    env.line.letters[idx as usize] = tile;
                    play_left(env, p, idx - 1);
                    env.num_played -= 1;
                    env.line.available[from] += 1;
                }
                if node.is_end() {
                    break;
                }
                p += 1;
            }
        }
    }

    let strider_len = line.tiles.len();
    let mut env = Env {
        line,
        anchor: 0,
        rightmost: 0,
        num_played: 0,
        idx_left: 0,
    };
    let mut leftmost = strider_len as i8; // processed up to here
    loop {
        env.rightmost = leftmost;
        while leftmost > 0 && env.line.tiles[leftmost as usize - 1] == 0 {
            leftmost -= 1;
        }
        if leftmost > 0 {
            // board[leftmost - 1] is a tile.
            env.anchor = leftmost - 1;
            // board[anchor + 1] is empty or off-board, board[anchor] has a tile.
            let mut p = 1;
            while leftmost > 0 && env.line.tiles[leftmost as usize - 1] != 0 {
                leftmost -= 1;
                p = env
                    .line
                    .kwg
                    .seek(p, env.line.tiles[leftmost as usize] & 0x7f);
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
}

// the letters that, between the tiles before them along a line and those after,
// make a run of tiles some word holds: those the gaddag leads to from its root
// by the tiles after reversed, then the letter, then the tiles before reversed;
// a bit (1 << letter) each
fn letters_between<N: kwg::Node>(kwg: &kwg::Kwg<N>, before: &[u8], after: &[u8]) -> u64 {
    let mut p = 1;
    for &tile in after.iter().rev() {
        p = kwg.seek(p, tile & 0x7f);
        if p <= 0 {
            return 0;
        }
    }
    let mut q = kwg.node_in_run(p).arc_index();
    if q <= 0 {
        return 0;
    }
    let mut letters = 0;
    loop {
        let node = kwg.node_in_run(q);
        let tile = node.tile();
        if tile != 0
            && (before.iter().rev())
                .try_fold(q, |p, &tile| {
                    Some(kwg.seek(p, tile & 0x7f)).filter(|&p| p > 0)
                })
                .is_some()
        {
            letters |= 1 << tile;
        }
        if node.is_end() {
            break;
        }
        q += 1;
    }
    letters
}

// the walk of lone_words: by depth and letter, the shapes that take the letter
// at that depth, a bit each in chunks words
struct LoneWalk<'a, N: kwg::Node> {
    kwg: &'a kwg::Kwg<N>,
    available: &'a mut [u8], // the tiles not on the board, a blank at 0
    fits: &'a [u64],
    alphabet_len: usize,
    chunks: usize,
    longest: usize,
    active: &'a mut [u64], // by depth, the shapes the word so far fits
    word: &'a mut Vec<u8>,
    found: &'a mut FoundWords,
    bits: &'a mut Vec<u64>, // by word of found, whether it is a lone word
    memory: Option<&'a endgame::MemoryLimit>, // found's and bits' limit
}

fn walk_lone_words<N: kwg::Node>(walk: &mut LoneWalk<'_, N>, mut p: i32, depth: usize) {
    let fits = walk.fits;
    let chunks = walk.chunks;
    loop {
        let node = walk.kwg.node_in_run(p);
        let tile = node.tile() as usize;
        let from = if walk.available[tile] > 0 { tile } else { 0 };
        if walk.available[from] > 0 {
            let fit = &fits[(depth * walk.alphabet_len + tile) * chunks..][..chunks];
            let (now, next) = walk.active[depth * chunks..].split_at_mut(chunks);
            let mut any = 0;
            for ((next, &now), &fit) in next.iter_mut().zip(now.iter()).zip(fit) {
                *next = now & fit;
                any |= *next;
            }
            if any != 0 {
                walk.available[from] -= 1;
                walk.word.push(tile as u8);
                if depth > 0
                    && node.accepts()
                    && let Some(index) = walk.found.insert(walk.word, walk.memory)
                {
                    walk.found.mark(walk.bits, index, walk.memory);
                }
                let next_p = node.arc_index();
                if next_p != 0 && depth + 1 < walk.longest {
                    walk_lone_words(walk, next_p, depth + 1);
                }
                walk.word.pop();
                walk.available[from] += 1;
            }
        }
        if node.is_end() {
            break;
        }
        p += 1;
    }
}

// the words of two letters or more the tiles not on the board spell alone, a
// blank as any letter, that fit a shape from its first square on: a shape is a
// run of squares of takes, (first, len), and the longest comes first. Each word
// is added to found and marked in bits. The walk's tables are kept in buffers
// from one prune to the next; each growth is taken within memory, if given
fn lone_words<N: kwg::Node>(
    kwg: &kwg::Kwg<N>,
    available: &mut [u8],
    LoneGeometry { takes, shapes }: LoneGeometry<'_>,
    found: &mut FoundWords,
    bits: &mut Vec<u64>,
    buffers: &mut LoneBuffers,
    memory: Option<&endgame::MemoryLimit>,
) -> error::Returns<()> {
    let root = kwg.node_in_run(0).arc_index();
    let Some(&(_, longest)) = shapes.first() else {
        return Ok(());
    };
    if root == 0 {
        return Ok(());
    }
    let LoneBuffers { fits, active, word } = buffers;
    let alphabet_len = available.len();
    let chunks = shapes.len().div_ceil(64);
    fits.clear();
    endgame::room_in(memory, fits, longest * alphabet_len * chunks)?;
    fits.resize(longest * alphabet_len * chunks, 0);
    for (index, &(first, len)) in shapes.iter().enumerate() {
        for (depth, &letters) in takes[first..first + len].iter().enumerate() {
            for tile in 1..alphabet_len {
                if letters & (1 << tile) != 0 {
                    fits[(depth * alphabet_len + tile) * chunks + index / 64] |= 1 << (index % 64);
                }
            }
        }
    }
    active.clear();
    endgame::room_in(memory, active, (longest + 1) * chunks)?;
    active.resize((longest + 1) * chunks, 0);
    for index in 0..shapes.len() {
        active[index / 64] |= 1 << (index % 64);
    }
    word.clear();
    endgame::room_in(memory, word, longest)?;
    walk_lone_words(
        &mut LoneWalk {
            kwg,
            available,
            fits,
            alphabet_len,
            chunks,
            longest,
            active,
            word,
            found,
            bits,
            memory,
        },
        root,
        0,
    );
    found.refused()
}

struct LoneGeometry<'a> {
    takes: &'a [u64],
    shapes: &'a [(usize, usize)],
}

// lone_words' tables: by depth and letter the shapes that take it, by depth
// the shapes the word so far fits, and the word
#[derive(Clone, Default)]
struct LoneBuffers {
    fits: Vec<u64>,
    active: Vec<u64>,
    word: Vec<u8>,
}

// what the word prune fills, kept by a generator's working buffer from one
// prune to the next
#[derive(Clone, Default)]
struct PruneBuffers {
    found: FoundWords,
    kept: Vec<u64>,       // by word of found, whether the prune keeps it
    order: Vec<u32>,      // the words kept, by index in found, as they ascend
    required: Vec<u8>,    // a jumbled run's tiles still to place
    lone_takes: Vec<u64>, // what a jumbled lone word's squares take
    takes: Vec<u64>,
    has_tiles: Vec<bool>,
    to_walk: Vec<bool>,
    words: Vec<Vec<u64>>, // by line, the words of found its last walk found
    letters: Vec<u8>,
    seen: Vec<u64>,
    shapes: Vec<(usize, usize)>,
    kept_shapes: Vec<(usize, usize)>,
    lone: LoneBuffers,
    empty_line: Vec<u8>,
}

// the words a prune finds, each kept once however often it is found: their
// tiles one after another, each word its length then its letters; by word,
// where it starts; and an open-addressing table, probed linearly, of a power
// of two slots kept at most three quarters full, each a word's index plus one
// with its hash's top half above it (0 for an empty slot), so that a probe
// compares a word's tiles only when their halves match. Once a memory limit
// refuses it a growth it takes no more words, nor marks
#[derive(Clone, Default)]
struct FoundWords {
    tiles: Vec<u8>,
    starts: Vec<u32>,
    slots: Vec<u64>,
    shift: u32, // a hash's top bits pick its slot: 64 - log2(slots.len())
    refused: bool,
    index_overflow: bool,
}

impl FoundWords {
    // Fibonacci hashing's multiplier, 2^64 divided by the golden ratio
    // (https://en.wikipedia.org/wiki/Hash_function#Fibonacci_hashing), as the
    // graph builder's table takes it
    const MULTIPLIER: u64 = 0x9e37_79b9_7f4a_7c15;
    // the most of its slots it fills
    const LOAD: (usize, usize) = (3, 4);

    // empty, its slots kept
    #[inline(always)]
    fn clear(&mut self) {
        self.tiles.clear();
        self.starts.clear();
        self.slots.fill(0);
        self.refused = false;
        self.index_overflow = false;
    }

    // whether a memory limit has refused it a growth
    #[inline(always)]
    fn refused(&self) -> error::Returns<()> {
        if self.index_overflow {
            crate::return_error!("word prune indices are out of range".to_string());
        }
        if self.refused {
            Err(endgame::Refused.into())
        } else {
            Ok(())
        }
    }

    #[inline(always)]
    fn len(&self) -> usize {
        self.starts.len()
    }

    #[inline(always)]
    fn word(&self, index: usize) -> &[u8] {
        let start = self.starts[index] as usize;
        &self.tiles[start + 1..start + 1 + self.tiles[start] as usize]
    }

    // a word's hash: its tiles eight at a time, each folded in and mixed
    #[inline(always)]
    fn hash(word: &[u8]) -> u64 {
        let (chunks, rest) = word.as_chunks::<8>();
        let mut last = [0u8; 8];
        last[..rest.len()].copy_from_slice(rest);
        (chunks.iter().chain(std::iter::once(&last))).fold(word.len() as u64, |hash, chunk| {
            (hash ^ u64::from_le_bytes(*chunk))
                .wrapping_mul(Self::MULTIPLIER)
                .rotate_left(u64::BITS / 2)
        })
    }

    // the index of the word found, added if it is new, each growth taken
    // within memory, if given; none once refused
    #[inline(always)]
    fn insert(&mut self, word: &[u8], memory: Option<&endgame::MemoryLimit>) -> Option<usize> {
        // a table that a new word would fill past three quarters grows first,
        // so a refused growth leaves every slot as it was
        if self.refused
            || ((self.starts.len() + 1) * Self::LOAD.1 > self.slots.len() * Self::LOAD.0
                && self.grow(memory).is_err())
        {
            self.refused = true;
            return None;
        }
        let hash = Self::hash(word);
        let tag = hash >> u32::BITS << u32::BITS;
        let mask = self.slots.len() - 1;
        let mut i = (hash.wrapping_mul(Self::MULTIPLIER) >> self.shift) as usize;
        loop {
            let slot = self.slots[i];
            if slot == 0 {
                break;
            }
            if slot & !u64::from(u32::MAX) == tag {
                let index = (slot as u32 - 1) as usize;
                if self.word(index) == word {
                    return Some(index);
                }
            }
            i = (i + 1) & mask;
        }
        let index = self.starts.len();
        let (Ok(start), Ok(word_len), Ok(slot_index)) = (
            u32::try_from(self.tiles.len()),
            u8::try_from(word.len()),
            u32::try_from(index + 1),
        ) else {
            self.refused = true;
            self.index_overflow = true;
            return None;
        };
        if endgame::room_in(memory, &mut self.starts, 1).is_err()
            || endgame::room_in(memory, &mut self.tiles, 1 + word.len()).is_err()
        {
            self.refused = true;
            return None;
        }
        self.starts.push(start);
        self.tiles.push(word_len);
        self.tiles.extend_from_slice(word);
        self.slots[i] = tag | u64::from(slot_index);
        Some(index)
    }

    // bit index of bits set, bits grown to hold it within memory, if given;
    // none once refused
    #[inline(always)]
    fn mark(&mut self, bits: &mut Vec<u64>, index: usize, memory: Option<&endgame::MemoryLimit>) {
        if index / 64 >= bits.len() {
            if endgame::room_in(memory, bits, index / 64 + 1 - bits.len()).is_err() {
                self.refused = true;
                return;
            }
            bits.resize(index / 64 + 1, 0);
        }
        bits[index / 64] |= 1 << (index % 64);
    }

    // twice the slots (or four, the first time), every word's put in them
    // again, the growth taken within memory, if given
    #[cold]
    fn grow(&mut self, memory: Option<&endgame::MemoryLimit>) -> Result<(), endgame::Refused> {
        let num_slots = (self.slots.len() * 2).max(Self::LOAD.1);
        if self.slots.capacity() < num_slots {
            // the old slots are let go first, so they and the new are never
            // held at once
            self.slots = Vec::new();
            endgame::room_in(memory, &mut self.slots, num_slots)?;
        }
        self.slots.clear();
        self.slots.resize(num_slots, 0);
        self.shift = u64::BITS - num_slots.trailing_zeros();
        let mask = num_slots - 1;
        for index in 0..self.starts.len() {
            let hash = Self::hash(self.word(index));
            let mut i = (hash.wrapping_mul(Self::MULTIPLIER) >> self.shift) as usize;
            while self.slots[i] != 0 {
                i = (i + 1) & mask;
            }
            self.slots[i] = hash >> u32::BITS << u32::BITS | (index as u64 + 1);
        }
        Ok(())
    }

    // the words keep says, by index, to found_word, in ascending order; order
    // is kept from one prune to the next, grown within memory, if given
    #[inline(always)]
    fn each_kept(
        &self,
        keep: impl Fn(usize) -> bool,
        order: &mut Vec<u32>,
        found_word: impl FnMut(&[u8]),
        memory: Option<&endgame::MemoryLimit>,
    ) -> error::Returns<()> {
        order.clear();
        endgame::room_in(memory, order, self.len())?;
        for index in (0..self.len()).filter(|&index| keep(index)) {
            order.push(u32::try_from(index)?);
        }
        order.sort_unstable_by(|&a, &b| self.word(a as usize).cmp(self.word(b as usize)));
        order
            .iter()
            .map(|&index| self.word(index as usize))
            .for_each(found_word);
        Ok(())
    }
}

// under jumbled rules a word's tiles lie on the board in any order, and the
// graph holds each word's alphagram, its tiles in rising order. A word fills a
// run of a line, the squares between an empty square or the edge and the next:
// it holds every tile of the board in its run, and the rest of its tiles, at
// least one, come from those not on the board. So, for each such run, each
// alphagram as long as the run that holds its board tiles; and the words of the
// tiles not on the board alone. Each is added to found (the prune's), once
// however many runs take it, every growth taken within memory, if given
fn gen_remaining_jumbled_words<
    'a,
    N: kwg::Node,
    L: kwg::Node,
    G: LaneBits,
    const BAG_TERM: bool,
    S: BoardStore,
>(
    board_snapshot: &'a BoardSnapshot<'a, N, L>,
    working_buffer: &'a mut WorkingBuffer<G, BAG_TERM, S>,
    memory: Option<&endgame::MemoryLimit>,
) -> error::Returns<()> {
    struct Env<'a, N: kwg::Node> {
        kwg: &'a kwg::Kwg<N>,
        available: &'a mut [u8], // the tiles not on the board, a blank at 0
        required: &'a mut [u8],  // the run's tiles still to place
        word: &'a mut Vec<u8>,
        found: &'a mut FoundWords,
        memory: Option<&'a endgame::MemoryLimit>,
    }

    // the alphagrams of left more tiles from the node list at p that place
    // every required tile, owed of them still to place
    fn walk<N: kwg::Node>(env: &mut Env<'_, N>, mut p: i32, left: usize, owed: usize) {
        // a required tile is placed before any tile above it, or never
        let lowest_owed = if owed > 0 {
            env.required.iter().position(|&count| count > 0)
        } else {
            None
        };
        loop {
            let node = env.kwg.node_in_run(p);
            let tile = node.tile() as usize;
            if lowest_owed.is_some_and(|lowest| tile > lowest) {
                break;
            }
            // a board tile first, then a tile not on the board, then a blank
            let from = if env.required[tile] > 0 {
                Some((true, tile))
            } else if env.available[tile] > 0 {
                Some((false, tile))
            } else if env.available[0] > 0 {
                Some((false, 0))
            } else {
                None
            };
            if let Some((required, index)) = from {
                let tally = if required {
                    &mut *env.required
                } else {
                    &mut *env.available
                };
                tally[index] -= 1;
                env.word.push(tile as u8);
                let owed = owed - usize::from(required);
                if left == 1 {
                    if owed == 0 && node.accepts() {
                        env.found.insert(env.word, env.memory);
                    }
                } else if owed < left {
                    let next = node.arc_index();
                    if next != 0 {
                        walk(env, next, left - 1, owed);
                    }
                }
                env.word.pop();
                let tally = if required {
                    &mut *env.required
                } else {
                    &mut *env.available
                };
                tally[index] += 1;
            }
            if node.is_end() {
                break;
            }
            p += 1;
        }
    }

    let game_config = &board_snapshot.game_config;
    let dim = game_config.board_layout().dim();
    let PruneBuffers {
        found,
        kept,
        required,
        lone_takes,
        lone,
        ..
    } = &mut working_buffer.prune;
    let longest = dim.rows.max(dim.cols) as usize;
    required.clear();
    endgame::room_in(memory, required, game_config.alphabet().len() as usize)?;
    required.resize(game_config.alphabet().len() as usize, 0);
    let root = board_snapshot.kwg.node_in_run(0).arc_index();
    working_buffer.exchange_buffer.clear();
    endgame::room_in(memory, &mut working_buffer.exchange_buffer, longest)?;
    for (tiles, len) in [
        (board_snapshot.board_tiles, dim.cols as usize),
        (
            &working_buffer.transposed_board_tiles[..],
            dim.rows as usize,
        ),
    ] {
        for line in tiles.chunks_exact(len) {
            for start in 0..len {
                if start > 0 && line[start - 1] != 0 {
                    continue;
                }
                required.fill(0);
                let mut held = 0;
                for end in start..len {
                    if line[end] != 0 {
                        required[(line[end] & 0x7f) as usize] += 1;
                        held += 1;
                    }
                    if end + 1 < len && line[end + 1] != 0 {
                        continue;
                    }
                    let run = end + 1 - start;
                    if run >= 2 && held > 0 && held < run && root != 0 {
                        walk(
                            &mut Env {
                                kwg: board_snapshot.kwg,
                                available: &mut working_buffer.used_letters_tally,
                                required,
                                word: &mut working_buffer.exchange_buffer,
                                found,
                                memory,
                            },
                            root,
                            run,
                            held,
                        );
                    }
                }
            }
        }
    }
    found.refused()?;
    lone_takes.clear();
    endgame::room_in(memory, lone_takes, longest)?;
    lone_takes.resize(longest, !1);
    kept.clear();
    lone_words(
        board_snapshot.kwg,
        &mut working_buffer.used_letters_tally,
        LoneGeometry {
            takes: lone_takes,
            shapes: &[(0, longest)],
        },
        found,
        kept,
        lone,
        memory,
    )
}

fn gen_remaining_woadwage_words<
    'a,
    N: kwg::Node,
    L: kwg::Node,
    G: LaneBits,
    const BAG_TERM: bool,
    S: BoardStore,
>(
    board_snapshot: &'a BoardSnapshot<'a, N, L>,
    working_buffer: &'a mut WorkingBuffer<G, BAG_TERM, S>,
    memory: Option<&endgame::MemoryLimit>,
) -> error::Returns<()> {
    struct Env<'a, N: kwg::Node> {
        kwg: &'a kwg::Kwg<N>,
        available: &'a mut [u8], // the tiles not on the board, a blank at 0
        required: &'a mut [u8],  // a jumbled span's tiles still to place
        word: &'a mut Vec<u8>,
        found: &'a mut FoundWords,
        memory: Option<&'a endgame::MemoryLimit>,
    }

    #[inline(always)]
    fn take_from(available: &[u8], tile: usize) -> Option<usize> {
        if available[tile] > 0 {
            Some(tile)
        } else if available[0] > 0 {
            Some(0)
        } else {
            None
        }
    }

    fn walk_along<N: kwg::Node>(env: &mut Env<'_, N>, mut p: i32, squares: &[u8]) {
        let Some((&square, rest)) = squares.split_first() else {
            return;
        };
        loop {
            let node = env.kwg.node_in_run(p);
            let tile = node.tile() as usize;
            let from = if square != 0 {
                if tile == (square & 0x7f) as usize {
                    Some(None)
                } else {
                    None
                }
            } else {
                take_from(env.available, tile).map(Some)
            };
            if let Some(from) = from {
                if let Some(from) = from {
                    env.available[from] -= 1;
                }
                env.word.push(tile as u8);
                if node.accepts() {
                    env.found.insert(env.word, env.memory);
                }
                let next = node.arc_index();
                if next != 0 {
                    walk_along(env, next, rest);
                }
                env.word.pop();
                if let Some(from) = from {
                    env.available[from] += 1;
                }
            }
            if node.is_end() {
                break;
            }
            p += 1;
        }
    }

    fn walk_jumbled<N: kwg::Node>(env: &mut Env<'_, N>, mut p: i32, left: usize, owed: usize) {
        let lowest_owed = if owed > 0 {
            env.required.iter().position(|&count| count > 0)
        } else {
            None
        };
        loop {
            let node = env.kwg.node_in_run(p);
            let tile = node.tile() as usize;
            if lowest_owed.is_some_and(|lowest| tile > lowest) {
                break;
            }
            let from = if env.required[tile] > 0 {
                Some((true, tile))
            } else {
                take_from(env.available, tile).map(|index| (false, index))
            };
            if let Some((required, index)) = from {
                let tally = if required {
                    &mut *env.required
                } else {
                    &mut *env.available
                };
                tally[index] -= 1;
                env.word.push(tile as u8);
                let owed = owed - required as usize;
                if left == 1 {
                    if owed == 0 && node.accepts() {
                        env.found.insert(env.word, env.memory);
                    }
                } else if owed < left {
                    let next = node.arc_index();
                    if next != 0 {
                        walk_jumbled(env, next, left - 1, owed);
                    }
                }
                env.word.pop();
                let tally = if required {
                    &mut *env.required
                } else {
                    &mut *env.available
                };
                tally[index] += 1;
            }
            if node.is_end() {
                break;
            }
            p += 1;
        }
    }

    let game_config = &board_snapshot.game_config;
    let dim = game_config.board_layout().dim();
    let jumbled = matches!(
        game_config.game_rules(),
        game_config::GameRules::JumbledWoadwage
    );
    let PruneBuffers {
        found,
        required,
        empty_line,
        ..
    } = &mut working_buffer.prune;
    let longest = dim.rows.max(dim.cols) as usize;
    required.clear();
    endgame::room_in(memory, required, game_config.alphabet().len() as usize)?;
    required.resize(game_config.alphabet().len() as usize, 0);
    let root = board_snapshot.kwg.node_in_run(0).arc_index();
    if root == 0 {
        return Ok(());
    }
    let word = &mut working_buffer.exchange_buffer;
    word.clear();
    endgame::room_in(memory, word, longest)?;
    let mut env = Env {
        kwg: board_snapshot.kwg,
        available: &mut working_buffer.used_letters_tally,
        required,
        word,
        found,
        memory,
    };
    empty_line.clear();
    endgame::room_in(memory, empty_line, longest)?;
    empty_line.resize(longest, 0);
    let lines = std::iter::once(&empty_line[..]).chain(
        (board_snapshot.board_tiles.chunks_exact(dim.cols as usize))
            .chain(
                working_buffer
                    .transposed_board_tiles
                    .chunks_exact(dim.rows as usize),
            )
            .filter(|line| line.iter().any(|&tile| tile != 0)),
    );
    for (index, line) in lines.enumerate() {
        let len = line.len();
        for start in 0..if index == 0 { 1 } else { len } {
            if !jumbled {
                walk_along(&mut env, root, &line[start..]);
                continue;
            }
            env.required.fill(0);
            let mut held = 0;
            for end in start..len {
                if line[end] != 0 {
                    env.required[(line[end] & 0x7f) as usize] += 1;
                    held += 1;
                }
                walk_jumbled(&mut env, root, end + 1 - start, held);
            }
        }
    }
    env.found.refused()
}

// each word the board can still take: the tiles on it stay, and those not on
// it, a blank as any letter, may fill its empty squares however they come to
// the racks. found_word is called once for each, in ascending order. A word
// along a line holds one of its tiles, and that line's walk from its tiles
// finds it; else it lies in a run of empty squares beside no tile along the
// line. Whenever a letter is put beside a tile, it makes a word with that tile
// along the line they share; so a square takes, for a word the other way, only
// the letters the words along that line put on it. Each line is walked again
// until nothing it reads changes, and its words are those of its last walk.
#[inline]
fn gen_remaining_words<
    'a,
    FoundWord: 'a + FnMut(&[u8]),
    N: kwg::Node,
    L: kwg::Node,
    G: LaneBits,
    const BAG_TERM: bool,
    S: BoardStore,
>(
    board_snapshot: &'a BoardSnapshot<'a, N, L>,
    working_buffer: &'a mut WorkingBuffer<G, BAG_TERM, S>,
    memory: Option<&endgame::MemoryLimit>,
    found_word: FoundWord,
) -> error::Returns<()> {
    let game_config = &board_snapshot.game_config;
    let board_layout = game_config.board_layout();
    let dim = board_layout.dim();
    let alphabet = game_config.alphabet();

    let available_tally = &mut working_buffer.used_letters_tally;
    available_tally.clear();
    endgame::room_in(memory, available_tally, alphabet.len() as usize)?;
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
    working_buffer.prune.found.clear();
    match game_config.game_rules() {
        game_config::GameRules::Classic => {}
        game_config::GameRules::Woadwage | game_config::GameRules::JumbledWoadwage => {
            gen_remaining_woadwage_words(board_snapshot, working_buffer, memory)?;
            let PruneBuffers { found, order, .. } = &mut working_buffer.prune;
            return found.each_kept(|_| true, order, found_word, memory);
        }
        game_config::GameRules::Jumbled => {
            gen_remaining_jumbled_words(board_snapshot, working_buffer, memory)?;
            let PruneBuffers { found, order, .. } = &mut working_buffer.prune;
            return found.each_kept(|_| true, order, found_word, memory);
        }
    }
    let rows = dim.rows as usize;
    let cols = dim.cols as usize;
    let area = rows * cols;
    let board_tiles = &board_snapshot.board_tiles[..area];
    let transposed_board_tiles = &working_buffer.transposed_board_tiles[..area];
    let available = &mut working_buffer.used_letters_tally[..];
    let PruneBuffers {
        found,
        kept,
        order,
        takes,
        has_tiles,
        to_walk,
        words,
        letters,
        seen,
        shapes,
        kept_shapes,
        lone,
        ..
    } = &mut working_buffer.prune;
    // the rows, then the columns: where a line's squares start in takes (an
    // across word's squares row by row, then a down word's column by column),
    // and how many it has
    let line_at = |line: usize| {
        if line < rows {
            (line * cols, cols)
        } else {
            (area + (line - rows) * rows, rows)
        }
    };
    let tiles_at = |start: usize, len: usize| {
        if start < area {
            &board_tiles[start..start + len]
        } else {
            &transposed_board_tiles[start - area..start - area + len]
        }
    };
    let lines = rows + cols;
    // by square, the letters a word across may put there, then those of a word
    // down, a bit (1 << letter) each
    takes.clear();
    endgame::room_in(memory, takes, 2 * area)?;
    takes.resize(2 * area, !1);
    has_tiles.clear();
    endgame::room_in(memory, has_tiles, lines)?;
    has_tiles.extend((0..lines).map(|line| {
        let (start, len) = line_at(line);
        tiles_at(start, len).iter().any(|&tile| tile != 0)
    }));
    to_walk.clear();
    endgame::room_in(memory, to_walk, lines)?;
    to_walk.clone_from(has_tiles);
    // by line, the words of found its last walk found; each line's is emptied
    // as the line is walked, a line never walked has none, and lines past
    // these are never read
    if words.len() < lines {
        endgame::room_in(memory, words, lines - words.len())?;
        words.resize_with(lines, Vec::new);
    }
    words[..lines].iter_mut().for_each(Vec::clear);
    letters.clear();
    endgame::room_in(memory, letters, rows.max(cols))?;
    letters.resize(rows.max(cols), 0);
    seen.clear();
    endgame::room_in(memory, seen, rows.max(cols))?;
    seen.resize(rows.max(cols), 0);
    // whether a square of a line is empty and beside one of its tiles
    let beside = |tiles: &[u8], idx: usize| {
        tiles[idx] == 0
            && ((idx > 0 && tiles[idx - 1] != 0) || (idx + 1 < tiles.len() && tiles[idx + 1] != 0))
    };
    // where in takes a square of a line keeps the letters a word the other way
    // may put there, and that word's line
    let across = |line: usize, idx: usize| {
        if line < rows {
            (area + idx * rows + line, rows + idx)
        } else {
            (idx * cols + line - rows, idx)
        }
    };
    // to begin with, a square beside a tile takes, for a word the other way,
    // only letters that make a run some word holds with the tiles beside it
    for line in 0..lines {
        let (start, len) = line_at(line);
        let tiles = tiles_at(start, len);
        for idx in 0..len {
            if beside(tiles, idx) {
                let first = idx
                    - (tiles[..idx].iter().rev())
                        .take_while(|&&tile| tile != 0)
                        .count();
                let last = idx
                    + 1
                    + (tiles[idx + 1..].iter())
                        .take_while(|&&tile| tile != 0)
                        .count();
                takes[across(line, idx).0] &= letters_between(
                    board_snapshot.kwg,
                    &tiles[first..idx],
                    &tiles[idx + 1..last],
                );
            }
        }
    }
    loop {
        let mut walked = false;
        for line in 0..lines {
            if !to_walk[line] {
                continue;
            }
            to_walk[line] = false;
            walked = true;
            let (start, len) = line_at(line);
            let tiles = tiles_at(start, len);
            let bits = &mut words[line];
            bits.clear();
            prune_line(&mut PruneLine {
                kwg: board_snapshot.kwg,
                tiles,
                takes: &takes[start..start + len],
                available,
                letters: &mut letters[..len],
                seen: &mut seen[..len],
                found,
                bits,
                memory,
            });
            found.refused()?;
            for (idx, &letters_seen) in seen[..len].iter().enumerate() {
                if !beside(tiles, idx) {
                    continue;
                }
                let (square, other) = across(line, idx);
                let narrowed = takes[square] & letters_seen;
                if narrowed != takes[square] {
                    takes[square] = narrowed;
                    to_walk[other] |= has_tiles[other];
                }
            }
        }
        if !walked {
            break;
        }
    }
    // where a word of the tiles not on the board alone may lie: from each
    // square of a run of a line's empty squares that are beside no tile along
    // it and take some letter, to the run's end; the longest first, each kept
    // only when no shape kept before it takes every letter it takes
    shapes.clear();
    for line in 0..lines {
        let (start, len) = line_at(line);
        let tiles = tiles_at(start, len);
        let free = |idx: usize| {
            tiles[idx] == 0
                && (idx == 0 || tiles[idx - 1] == 0)
                && (idx + 1 == len || tiles[idx + 1] == 0)
                && takes[start + idx] != 0
        };
        let mut idx = 0;
        while idx < len {
            let run = (idx..len).take_while(|&at| free(at)).count();
            for first in idx..(idx + run).saturating_sub(1) {
                endgame::room_in(memory, shapes, 1)?;
                shapes.push((start + first, idx + run - first));
            }
            idx += run + 1;
        }
    }
    shapes.sort_unstable_by_key(|&(_, len)| std::cmp::Reverse(len));
    kept_shapes.clear();
    for &(first, len) in shapes.iter() {
        let shape = &takes[first..first + len];
        if !kept_shapes.iter().any(|&(other, _)| {
            (shape.iter().zip(&takes[other..])).all(|(&letters, &others)| letters & !others == 0)
        }) {
            endgame::room_in(memory, kept_shapes, 1)?;
            kept_shapes.push((first, len));
        }
    }
    kept.clear();
    lone_words(
        board_snapshot.kwg,
        available,
        LoneGeometry {
            takes,
            shapes: kept_shapes,
        },
        found,
        kept,
        lone,
        memory,
    )?;
    let num_chunks = found.len().div_ceil(64);
    endgame::room_in(memory, kept, num_chunks.saturating_sub(kept.len()))?;
    kept.resize(num_chunks, 0);
    for bits in &words[..lines] {
        kept.iter_mut()
            .zip(bits)
            .for_each(|(kept, &bits)| *kept |= bits);
    }
    found.each_kept(
        |index| kept[index / 64] & (1 << (index % 64)) != 0,
        order,
        found_word,
        memory,
    )
}

// a frozen corpus of every arm's generations, asked every way
#[cfg(test)]
mod corpus;

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::{alphabet, bites, board_layout, build, display, game_config, klv, kwg};

    #[test]
    fn every_rack_shape_agrees_with_the_subrack_walk() {
        let klv = klv::Klv::<kwg::Node24>::empty().unwrap();
        let mut leaves = klv::MultiLeaves::new();
        let mut walked = Vec::new();
        for rack_size in 1..=RACK_SHAPE_MAX {
            for gaps in 0..1 << (rack_size - 1) {
                let mut rack = [0u8; MAX_ALPHABET_LEN];
                let mut tile = 0;
                let mut bits = 0u64;
                for pos in 0..rack_size {
                    rack[tile] += 1;
                    bits |= 1 << tile;
                    tile += usize::from(gaps & (1 << pos) != 0);
                }
                leaves
                    .init(
                        &rack,
                        &klv,
                        false,
                        klv::AdjustLeave::Identity,
                        leaves.dense_most(),
                        std::iter::empty(),
                    )
                    .unwrap();
                walk_subracks(
                    &leaves,
                    alphagram::Nibble,
                    &rack,
                    &[0; MAX_ALPHABET_LEN],
                    &mut walked,
                );
                let shape = rack_shape(&rack, bits, rack_size);
                let mut starts = vec![0u32; rack_size + 2];
                for (i, subrack) in walked.iter().enumerate() {
                    assert_eq!(
                        u32::from(shape.leave_indices[i]),
                        subrack.leave_idx,
                        "{gaps}"
                    );
                    starts[subrack.num_played as usize + 1] += 1;
                }
                for i in 1..starts.len() {
                    starts[i] += starts[i - 1];
                }
                assert_eq!(shape.by_played, starts, "{gaps}");
            }
        }
    }

    static TEST_WORDS: &[&str] = &[
        "AS", "AT", "EAST", "EAT", "EATS", "ETA", "ETAS", "SAT", "SEA", "SEAT", "SEATS", "SET",
        "TA", "TAE", "TAS", "TEA", "TEAS",
    ];

    pub(super) static SWEEP_WORDS: &[&str] = &[
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
    fn sweep_kwg(gc: &game_config::GameConfig) -> kwg::Kwg<kwg::Node24> {
        let reader = alphabet::AlphabetReader::new_for_words(gc.alphabet());
        let mut word_buf = Vec::new();
        let mut words = Vec::<bites::Bites>::with_capacity(SWEEP_WORDS.len());
        for w in SWEEP_WORDS {
            reader.set_word(w, &mut word_buf).unwrap();
            words.push(word_buf[..].into());
        }
        words.sort_unstable();
        words.dedup();
        kwg::kbwg_from_bytes_alloc(
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
        .unwrap()
    }

    #[inline]
    pub(super) fn sweep_kwg_covering(gc: &game_config::GameConfig) -> kwg::Kwg<kwg::Node24> {
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
        kwg::kbwg_from_bytes_alloc(
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
        .unwrap()
    }

    #[inline]
    fn sweep_output() -> String {
        let gc = game_config::make_english_game_config();
        let kwg = sweep_kwg(&gc);
        let klv = klv::kbwg_from_bytes_alloc(klv::EMPTY_KLV_BYTES.to_vec()).unwrap();
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
            move_generator
                .gen_moves_unfiltered(&GenMovesParams {
                    board_snapshot: &board_snapshot,
                    rack: &parse_test_rack(gc.alphabet(), rack),
                    max_gen: *max_gen,
                    num_exchanges_by_this_player: 0,
                    pass_policy: PassPolicy::OnlyWhenForced,
                    dynamic_leaves: None,
                })
                .unwrap();
            out.push_str(&format!("== {fen} {rack} {max_gen}\n"));
            for p in &move_generator.plays {
                out.push_str(&format!(
                    "{} {}\n",
                    p.equity.raw(),
                    p.play.fmt(&board_snapshot).unwrap()
                ));
            }
        }
        out
    }

    #[inline]
    #[test]
    fn the_generator_still_generates_what_it_generated() {
        assert_eq!(
            sweep_output(),
            include_str!("movegen-sweep-baseline.txt"),
            "generation moved; read the diff before refreshing the baseline"
        );
    }

    #[inline]
    #[test]
    #[ignore]
    fn write_sweep_baseline() {
        std::fs::write("src/movegen-sweep-baseline.txt", sweep_output()).unwrap();
    }

    #[inline]
    fn both_arms_agree(gc: &game_config::GameConfig, boards: &[&str], racks: &[&str]) {
        let kwg = sweep_kwg_covering(gc);
        let held = anagrams::Anagrams::build_for_config(&kwg, gc)
            .unwrap()
            .unwrap();
        let rack_lengths = anagrams::RackLengths::build_for_config(&kwg, gc)
            .unwrap()
            .unwrap();
        let klv = klv::kbwg_from_bytes_alloc(klv::EMPTY_KLV_BYTES.to_vec()).unwrap();
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
                        move_generator
                            .gen_moves_unfiltered(&GenMovesParams {
                                board_snapshot: &board_snapshot,
                                rack: &rack,
                                max_gen,
                                num_exchanges_by_this_player: 0,
                                pass_policy: PassPolicy::OnlyWhenForced,
                                dynamic_leaves: None,
                            })
                            .unwrap();
                        for p in &move_generator.plays {
                            out.push_str(&format!(
                                "{} {}\n",
                                p.equity.raw(),
                                p.play.fmt(&board_snapshot).unwrap()
                            ));
                        }
                    }
                    assert_eq!(walked, fetched, "{fen} at a cap of {max_gen}");
                    assert!(!walked.is_empty(), "{fen} generated nothing at all");
                }
            }
        }
    }

    pub(super) static SWEEP_RACKS: &[&str] = &[
        "AEINRST", "CARTONS", "STARTED", "SEATERS", "RATTANS", "AAAAAAA", "AE", "ST", "CAT",
        "NNNNTTT", "ERASECS", "?EINRST", "?AT", "C?T", "?ANTES", "?", "?A", "??TANS", "?ANTS??",
    ];

    #[inline]
    #[test]
    fn the_tables_find_exactly_what_the_descent_finds() {
        let gc = game_config::make_english_game_config();
        let empty = "15/15/15/15/15/15/15/15/15/15/15/15/15/15/15";
        let one = "15/15/15/15/15/15/15/6CARE5/15/15/15/15/15/15/15";
        let two = "15/15/15/15/15/15/15/6CARE5/6A8/6N8/6E8/15/15/15/15";
        let three = "15/15/15/15/15/15/15/6CARE5/6A8/6N8/6EATS5/15/15/15/15";
        let dead = "15/15/15/15/15/15/15/6N3A4/15/6N8/15/15/15/15/15";
        both_arms_agree(&gc, &[empty, one, two, three, dead], SWEEP_RACKS);
    }

    #[inline]
    #[test]
    fn a_wide_board_is_read_the_same_on_every_extent() {
        let gc = game_config::make_super_english_game_config();
        let dim = gc.board_layout().dim();
        assert!(dim.cols > 15, "this board is not wider than the old guard");
        let empty = "21/21/21/21/21/21/21/21/21/21/21/21/21/21/21/21/21/21/21/21/21";
        let one = "21/21/21/21/21/21/21/21/21/21/8CARE9/21/21/21/21/21/21/21/21/21/21";
        let two = "21/21/21/21/21/21/21/21/21/21/8CARE9/8A12/8N12/8E12/21/21/21/21/21/21/21";
        both_arms_agree(&gc, &[empty, one, two], SWEEP_RACKS);
    }

    pub(super) static LONG_RACKS: &[&str] = &[
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

    #[inline]
    #[test]
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

    // a walk along a run that stops where the run letters leave it no candidate
    // finds the plays a walk to the run's end finds, in its order
    #[inline]
    #[test]
    fn stopping_at_the_last_candidate_finds_the_same_plays() {
        let klv = klv::kbwg_from_bytes_alloc(klv::EMPTY_KLV_BYTES.to_vec()).unwrap();
        let fifteen = [
            "15/15/15/15/15/15/15/15/15/15/15/15/15/15/15",
            "15/15/15/15/15/15/15/6CARE5/15/15/15/15/15/15/15",
            "15/15/15/15/15/15/15/6CARE5/6A8/6N8/6EATS5/15/15/15/15",
            "15/15/15/15/15/15/15/6N3A4/15/6N8/15/15/15/15/15",
        ];
        let twenty_one = [
            "21/21/21/21/21/21/21/21/21/21/21/21/21/21/21/21/21/21/21/21/21",
            "21/21/21/21/21/21/21/21/21/21/8CARE9/8A12/8N12/8E12/21/21/21/21/21/21/21",
        ];
        for (gc, boards, racks) in [
            (
                game_config::make_english_game_config(),
                &fifteen[..],
                SWEEP_RACKS,
            ),
            (
                game_config::make_super_english_game_config(),
                &twenty_one[..],
                SWEEP_RACKS,
            ),
            (
                game_config::make_hong_kong_english_game_config(),
                &fifteen[..],
                LONG_RACKS,
            ),
        ] {
            let kwg = sweep_kwg_covering(&gc);
            let run_letter_bits = kwg.compute_letter_bits().letter_bits;
            let mut fen_parser = display::BoardFenParser::new(gc.alphabet(), gc.board_layout());
            for fen in boards {
                let board_tiles = fen_parser.parse(fen).unwrap().to_vec();
                let board_snapshot = BoardSnapshot {
                    board_tiles: &board_tiles,
                    game_config: &gc,
                    kwg: &kwg,
                    anagrams: None,
                    rack_lengths: None,
                    klv: &klv,
                };
                for rack in racks {
                    let rack = parse_test_rack(gc.alphabet(), rack);
                    let mut move_generator = KurniaMoveGenerator::new(&gc);
                    let plays = |g: &KurniaMoveGenerator| {
                        g.plays
                            .iter()
                            .map(|p| format!("{}\n", p.play.fmt(&board_snapshot).unwrap()))
                            .collect::<String>()
                    };
                    move_generator
                        .gen_moves_raw_lanes_unsorted(
                            &board_snapshot,
                            &rack,
                            0,
                            PassPolicy::AsACandidate,
                            !0,
                            !0,
                        )
                        .unwrap();
                    let walked = plays(&move_generator);
                    move_generator
                        .gen_moves_raw_lanes_unsorted_by_runs(
                            &run_letter_bits,
                            &board_snapshot,
                            &rack,
                            !0,
                            !0,
                        )
                        .unwrap();
                    let stopped = plays(&move_generator);
                    assert_eq!(walked, stopped, "{fen}");
                    assert!(!walked.is_empty(), "{fen} generated nothing at all");
                }
            }
        }
    }

    // a rack of one tile, its plays from the cross sets, plays what the walk
    // finds, with its scores, on any lanes, under classic and jumbled rules, in
    // every generation: raw (with the run letters or without), by the lean arm,
    // and by the tables, on a fresh generator and on one whose cross sets a
    // walk left. The walk is of the tile beside a Z, which no word here holds,
    // so it plays the tile alone
    #[inline]
    #[test]
    fn a_lone_tile_plays_what_the_walk_finds() {
        fn params<'a, N: kwg::Node, L: kwg::Node>(
            board_snapshot: &'a BoardSnapshot<'a, N, L>,
            rack: &'a [u8],
            max_gen: usize,
        ) -> GenMovesParams<'a, N, L> {
            GenMovesParams {
                board_snapshot,
                rack,
                max_gen,
                num_exchanges_by_this_player: 0,
                pass_policy: PassPolicy::OnlyWhenForced,
                dynamic_leaves: None,
            }
        }
        let klv = klv::kbwg_from_bytes_alloc(klv::EMPTY_KLV_BYTES.to_vec()).unwrap();
        let fifteen = [
            "15/15/15/15/15/15/15/6CARE5/15/15/15/15/15/15/15",
            "15/15/15/15/15/15/15/6CARE5/6A8/6N8/6EATS5/15/15/15/15",
            "15/15/15/15/15/15/15/6N3A4/15/6N8/15/15/15/15/15",
            "15/15/15/15/15/15/15/3CAT1EAT5/15/15/15/15/15/15/15",
            "15/15/15/15/15/15/15/2CAT2EAT5/15/15/15/15/15/15/15",
            "CARE11/15/15/15/15/15/15/15/15/15/15/15/15/15/11CARE",
            "A14/R14/T14/15/15/15/15/15/15/15/15/15/14c/14a/14t",
            "15/15/15/15/15/15/4cArE7/4A10/4N10/4EAT8/15/15/15/15/15",
        ];
        let twenty_one = [
            "21/21/21/21/21/21/21/21/21/21/8CARE9/8A12/8N12/8E12/21/21/21/21/21/21/21",
            "CAT1EAT14/21/21/21/21/21/21/21/21/21/21/21/21/21/21/21/21/21/21/21/17CARE",
        ];
        let racks = ["A", "C", "E", "N", "R", "S", "T", "Z", "?"];
        let masks = [
            (!0u128, !0u128),
            (1 << 7, 0),
            (0, 1 << 6),
            ((1 << 7) | (1 << 10), (1 << 6) | (1 << 9)),
        ];
        for (gc, boards) in [
            (game_config::make_english_game_config(), &fifteen[..]),
            (
                game_config::make_super_english_game_config(),
                &twenty_one[..],
            ),
            (
                game_config::make_jumbled_english_game_config(),
                &fifteen[..],
            ),
            (
                game_config::make_jumbled_super_english_game_config(),
                &twenty_one[..],
            ),
        ] {
            let kwg = match gc.game_rules() {
                game_config::GameRules::Classic | game_config::GameRules::Woadwage => {
                    sweep_kwg_covering(&gc)
                }
                game_config::GameRules::Jumbled | game_config::GameRules::JumbledWoadwage => {
                    test_kad(&gc, SWEEP_WORDS)
                }
            };
            let run_letter_bits = kwg.compute_letter_bits().letter_bits;
            let held = anagrams::Anagrams::build_for_config(&kwg, &gc)
                .unwrap()
                .unwrap();
            let dead = parse_test_rack(gc.alphabet(), "Z");
            let mut fen_parser = display::BoardFenParser::new(gc.alphabet(), gc.board_layout());
            for fen in boards {
                let board_tiles = fen_parser.parse(fen).unwrap().to_vec();
                let mut placed = 0;
                for anagrams in [None, Some(&held)] {
                    let board_snapshot = BoardSnapshot {
                        board_tiles: &board_tiles,
                        game_config: &gc,
                        kwg: &kwg,
                        anagrams,
                        rack_lengths: None,
                        klv: &klv,
                    };
                    let places = |g: &KurniaMoveGenerator| {
                        let mut lines = g
                            .plays
                            .iter()
                            .filter_map(|p| match &p.play {
                                Play::Place { score, .. } => Some(format!(
                                    "{} {score}",
                                    p.play.fmt(&board_snapshot).unwrap()
                                )),
                                Play::Exchange { .. } => None,
                            })
                            .collect::<Vec<_>>();
                        lines.sort_unstable();
                        lines
                    };
                    for rack in racks {
                        let lone = parse_test_rack(gc.alphabet(), rack);
                        let mut beside = [&lone[..], &dead[..]].concat();
                        beside.sort_unstable();
                        for (rows, cols) in masks {
                            let at = format!(
                                "{fen}, rack {rack}, tables {}, rows {rows:#x}, columns {cols:#x}",
                                anagrams.is_some()
                            );
                            let mut walker = KurniaMoveGenerator::new(&gc);
                            walker
                                .gen_moves_raw_lanes_unsorted(
                                    &board_snapshot,
                                    &beside,
                                    0,
                                    PassPolicy::AsACandidate,
                                    rows,
                                    cols,
                                )
                                .unwrap();
                            let walked = places(&walker);
                            placed += walked.len();
                            walker
                                .gen_moves_unfiltered_lanes(
                                    &params(&board_snapshot, &beside, usize::MAX),
                                    rows,
                                    cols,
                                )
                                .unwrap();
                            assert_eq!(places(&walker), walked, "{at}");
                            walker
                                .gen_moves_unfiltered_lanes_lean(
                                    &params(&board_snapshot, &beside, usize::MAX),
                                    rows,
                                    cols,
                                )
                                .unwrap();
                            assert_eq!(places(&walker), walked, "{at}");
                            let mut fresh = KurniaMoveGenerator::new(&gc);
                            for move_generator in [&mut fresh, &mut walker] {
                                move_generator
                                    .gen_moves_raw_lanes_unsorted_by_runs(
                                        &run_letter_bits,
                                        &board_snapshot,
                                        &lone,
                                        rows,
                                        cols,
                                    )
                                    .unwrap();
                                assert_eq!(places(move_generator), walked, "{at}");
                                move_generator
                                    .gen_moves_raw_lanes_unsorted(
                                        &board_snapshot,
                                        &lone,
                                        0,
                                        PassPolicy::AsACandidate,
                                        rows,
                                        cols,
                                    )
                                    .unwrap();
                                assert_eq!(places(move_generator), walked, "{at}");
                                for lean in [false, true] {
                                    let generate = |g: &mut KurniaMoveGenerator, max_gen: usize| {
                                        let params = params(&board_snapshot, &lone, max_gen);
                                        if lean {
                                            g.gen_moves_unfiltered_lanes_lean(&params, rows, cols)
                                                .unwrap();
                                        } else {
                                            g.gen_moves_unfiltered_lanes(&params, rows, cols)
                                                .unwrap();
                                        }
                                        g.plays
                                            .iter()
                                            .map(|p| (p.equity, p.play.clone()))
                                            .collect::<Vec<_>>()
                                    };
                                    let all = generate(move_generator, usize::MAX);
                                    assert_eq!(places(move_generator), walked, "{at}");
                                    // the best few, under a threshold that
                                    // rises as they come
                                    for max_gen in [1, 3] {
                                        let best = generate(move_generator, max_gen);
                                        assert!(
                                            best.iter().eq(all.iter().take(max_gen)),
                                            "{at}, at most {max_gen}"
                                        );
                                    }
                                    // the async lean arm's, on every lane
                                    if lean && (rows, cols) == (!0, !0) {
                                        let params = params(&board_snapshot, &lone, usize::MAX);
                                        {
                                            let mut fut = std::pin::pin!(
                                                move_generator.gen_moves_filtered_async_lean(
                                                    &params,
                                                    PlacePredicate::AcceptAll,
                                                    klv::AdjustLeave::Identity,
                                                    EquityPredicate::AcceptAll,
                                                    || std::future::ready(()),
                                                )
                                            );
                                            let mut cx = std::task::Context::from_waker(
                                                std::task::Waker::noop(),
                                            );
                                            loop {
                                                if let std::task::Poll::Ready(result) =
                                                    fut.as_mut().poll(&mut cx)
                                                {
                                                    result.unwrap();
                                                    break;
                                                }
                                            }
                                        }
                                        assert!(
                                            move_generator
                                                .plays
                                                .iter()
                                                .map(|p| (p.equity, p.play.clone()))
                                                .eq(all.iter().cloned()),
                                            "{at}, async"
                                        );
                                    }
                                }
                            }
                        }
                    }
                }
                assert!(placed > 0, "{fen} placed no lone tile");
            }
        }
    }

    // a generator's state kept on a board and put back after it generated on
    // others (some of them on a few lanes only, leaving lanes unlooked at)
    // generates what a new generator does there, on that board and the next
    #[test]
    fn a_board_state_put_back_generates_as_a_new_generator() {
        let gc = game_config::make_english_game_config();
        let kwg = sweep_kwg_covering(&gc);
        let held = anagrams::Anagrams::build_for_config(&kwg, &gc)
            .unwrap()
            .unwrap();
        let rack_lengths = anagrams::RackLengths::build_for_config(&kwg, &gc)
            .unwrap()
            .unwrap();
        let klv = klv::kbwg_from_bytes_alloc(klv::EMPTY_KLV_BYTES.to_vec()).unwrap();
        let mut fen_parser = display::BoardFenParser::new(gc.alphabet(), gc.board_layout());
        let boards = [
            "15/15/15/15/15/15/15/6CARE5/15/15/15/15/15/15/15",
            "15/15/15/15/15/15/15/6CARE5/6A8/6N8/6EATS5/15/15/15/15",
            "15/15/15/15/15/15/6S8/6CARE5/6A8/6N8/6EATS5/9C5/9A5/9T5/15",
            "15/15/15/15/15/15/15/4SCARE6/6A8/6N8/6EATS5/15/15/15/15",
        ]
        .map(|fen| fen_parser.parse(fen).unwrap().to_vec());
        let rack = parse_test_rack(gc.alphabet(), "AEINRST");
        for anagrams in [None, Some(&held)] {
            let gen_on = |move_generator: &mut KurniaMoveGenerator, board_tiles: &[u8]| {
                let board_snapshot = BoardSnapshot {
                    board_tiles,
                    game_config: &gc,
                    kwg: &kwg,
                    anagrams,
                    rack_lengths: anagrams.is_none().then_some(&rack_lengths),
                    klv: &klv,
                };
                move_generator
                    .gen_moves_unfiltered(&GenMovesParams {
                        board_snapshot: &board_snapshot,
                        rack: &rack,
                        max_gen: 100_000,
                        num_exchanges_by_this_player: 0,
                        pass_policy: PassPolicy::OnlyWhenForced,
                        dynamic_leaves: None,
                    })
                    .unwrap();
                let mut lines = move_generator
                    .plays
                    .iter()
                    .map(|p| format!("{} {}", p.play.fmt(&board_snapshot).unwrap(), p.equity))
                    .collect::<Vec<_>>();
                lines.sort_unstable();
                lines
            };
            let fresh =
                |board_tiles: &[u8]| gen_on(&mut KurniaMoveGenerator::new(&gc), board_tiles);
            for kept in 0..boards.len() {
                for masked in [false, true] {
                    let mut move_generator = KurniaMoveGenerator::new(&gc);
                    // the kept board's generation itself on a few lanes, or on
                    // all
                    if masked {
                        gen_on(&mut move_generator, &boards[(kept + 1) % boards.len()]);
                        move_generator.working_buffer.lane_rows = (1 << 7) | (1 << 10);
                        move_generator.working_buffer.lane_cols = 1 << 6;
                    }
                    gen_on(&mut move_generator, &boards[kept]);
                    move_generator.working_buffer.lane_rows = !0;
                    move_generator.working_buffer.lane_cols = !0;
                    let mut state = BoardState::default();
                    move_generator.save_board_state(&mut state);
                    for round in 0..2 {
                        for (other, board_tiles) in boards.iter().enumerate() {
                            if other == kept {
                                continue;
                            }
                            if round == 1 {
                                move_generator.working_buffer.lane_rows = 1 << 7;
                                move_generator.working_buffer.lane_cols = (1 << 6) | (1 << 9);
                            }
                            gen_on(&mut move_generator, board_tiles);
                            move_generator.working_buffer.lane_rows = !0;
                            move_generator.working_buffer.lane_cols = !0;
                            move_generator.restore_board_state(&state);
                            assert_eq!(
                                gen_on(&mut move_generator, &boards[kept]),
                                fresh(&boards[kept]),
                                "kept {kept}, masked {masked}, after {other}, round {round}"
                            );
                            let next = &boards[(kept + 1 + other) % boards.len()];
                            assert_eq!(
                                gen_on(&mut move_generator, next),
                                fresh(next),
                                "the next after kept {kept}, masked {masked}, other {other}"
                            );
                            move_generator.restore_board_state(&state);
                        }
                    }
                }
            }
        }
    }

    // a generation of some lanes alone finds exactly the plays a whole one
    // finds on them, raw or not, in either arm, and the next is whole again
    #[inline]
    #[test]
    fn a_generation_of_some_lanes_finds_exactly_their_plays() {
        let gc = game_config::make_english_game_config();
        let kwg = sweep_kwg_covering(&gc);
        let held = anagrams::Anagrams::build_for_config(&kwg, &gc)
            .unwrap()
            .unwrap();
        let rack_lengths = anagrams::RackLengths::build_for_config(&kwg, &gc)
            .unwrap()
            .unwrap();
        let klv = klv::kbwg_from_bytes_alloc(klv::EMPTY_KLV_BYTES.to_vec()).unwrap();
        let mut fen_parser = display::BoardFenParser::new(gc.alphabet(), gc.board_layout());
        let one = "15/15/15/15/15/15/15/6CARE5/15/15/15/15/15/15/15";
        let three = "15/15/15/15/15/15/15/6CARE5/6A8/6N8/6EATS5/15/15/15/15";
        let masks = [
            (1u128 << 7, 0u128),
            (0, 1 << 6),
            ((1 << 7) | (1 << 10), (1 << 6) | (1 << 9)),
            (0, 0),
        ];
        for fen in [one, three] {
            let board_tiles = fen_parser.parse(fen).unwrap().to_vec();
            for rack in ["AEINRST", "CAT", "?ANTS"] {
                let rack = parse_test_rack(gc.alphabet(), rack);
                for anagrams in [None, Some(&held)] {
                    let board_snapshot = BoardSnapshot {
                        board_tiles: &board_tiles,
                        game_config: &gc,
                        kwg: &kwg,
                        anagrams,
                        rack_lengths: anagrams.is_none().then_some(&rack_lengths),
                        klv: &klv,
                    };
                    let on = |plays: &[ValuedMove], rows: u128, cols: u128| {
                        let mut lines = plays
                            .iter()
                            .filter(|p| match &p.play {
                                Play::Place { down, lane, .. } => {
                                    ((if *down { cols } else { rows }) >> lane) & 1 != 0
                                }
                                Play::Exchange { .. } => false,
                            })
                            .map(|p| p.play.fmt(&board_snapshot).unwrap().to_string())
                            .collect::<Vec<_>>();
                        lines.sort_unstable();
                        lines
                    };
                    let mut move_generator = KurniaMoveGenerator::new(&gc);
                    let gen_raw = |move_generator: &mut KurniaMoveGenerator| {
                        move_generator
                            .gen_moves_raw_all_unsorted(
                                &board_snapshot,
                                &rack,
                                i16::MAX,
                                PassPolicy::OnlyWhenForced,
                            )
                            .unwrap()
                    };
                    let gen_unfiltered = |move_generator: &mut KurniaMoveGenerator| {
                        move_generator
                            .gen_moves_unfiltered(&GenMovesParams {
                                board_snapshot: &board_snapshot,
                                rack: &rack,
                                max_gen: 100_000,
                                num_exchanges_by_this_player: 0,
                                pass_policy: PassPolicy::OnlyWhenForced,
                                dynamic_leaves: None,
                            })
                            .unwrap()
                    };
                    gen_raw(&mut move_generator);
                    let raw = move_generator.plays.clone();
                    assert!(!on(&raw, !0, !0).is_empty(), "{fen} generated nothing");
                    gen_unfiltered(&mut move_generator);
                    let unfiltered = move_generator.plays.clone();
                    for (rows, cols) in masks {
                        move_generator
                            .gen_moves_raw_lanes_unsorted(
                                &board_snapshot,
                                &rack,
                                i16::MAX,
                                PassPolicy::OnlyWhenForced,
                                rows,
                                cols,
                            )
                            .unwrap();
                        assert_eq!(
                            on(&move_generator.plays, !0, !0),
                            on(&raw, rows, cols),
                            "{fen} raw, rows {rows:#x}, columns {cols:#x}",
                        );
                        gen_raw(&mut move_generator);
                        assert_eq!(on(&move_generator.plays, !0, !0), on(&raw, !0, !0));
                        // the generation that is not raw honors the masks too
                        move_generator.working_buffer.lane_rows = rows;
                        move_generator.working_buffer.lane_cols = cols;
                        gen_unfiltered(&mut move_generator);
                        move_generator.working_buffer.lane_rows = !0;
                        move_generator.working_buffer.lane_cols = !0;
                        assert_eq!(
                            on(&move_generator.plays, !0, !0),
                            on(&unfiltered, rows, cols),
                            "{fen}, rows {rows:#x}, columns {cols:#x}",
                        );
                    }
                }
            }
        }
    }

    #[inline]
    fn test_kwg(gc: &game_config::GameConfig) -> kwg::Kwg<kwg::Node24> {
        test_kwg_of(gc, TEST_WORDS)
    }

    #[inline]
    fn test_kwg_of(gc: &game_config::GameConfig, test_words: &[&str]) -> kwg::Kwg<kwg::Node24> {
        let reader = alphabet::AlphabetReader::new_for_words(gc.alphabet());
        let mut word_buf = Vec::new();
        let mut words = Vec::<bites::Bites>::with_capacity(test_words.len());
        for w in test_words {
            reader.set_word(w, &mut word_buf).unwrap();
            words.push(word_buf[..].into());
        }
        words.sort_unstable();
        kwg::kbwg_from_bytes_alloc(
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
        .unwrap()
    }

    #[test]
    fn two_leave_tables_choose_as_their_separate_walks_do() {
        let game = game_config::make_english_game_config();
        let kwg = test_kwg_of(&game, &["AT", "CAT", "COAT", "COT", "TA", "TO"]);
        let [first, second] = [
            klv::make_klv2(&[(&[1, 3], 80.0), (&[26], 50.0)]),
            klv::make_klv2(&[(&[15], 80.0), (&[17], 50.0)]),
        ]
        .map(|bytes| klv::kbwg_from_bytes_alloc(bytes).unwrap());
        let mut board = vec![0; 225];
        let mut paired = PairedHasty::default();
        for lean in [false, true] {
            let tables = anagrams::ArmTables::build(&kwg, &game, lean).unwrap();
            let mut shared = KurniaMoveGenerator::new(&game);
            let mut separate = KurniaMoveGenerator::new(&game);
            for (i, (rack, exchanges)) in [
                (&[1, 3, 15, 20][..], 0),
                (&[17, 26][..], 0),
                (&[17, 26][..], i16::MAX),
                (&[0, 1, 3, 15, 20][..], 0),
                (&[15][..], 0),
            ]
            .into_iter()
            .enumerate()
            {
                board.fill(0);
                if i == 4 {
                    board[112..114].copy_from_slice(&[1, 20]);
                }
                let board_snapshot = BoardSnapshot {
                    board_tiles: &board,
                    game_config: &game,
                    kwg: &kwg,
                    anagrams: tables.anagrams(),
                    rack_lengths: tables.rack_lengths(),
                    klv: &first,
                };
                let params = GenMovesParams {
                    board_snapshot: &board_snapshot,
                    rack,
                    max_gen: 1,
                    num_exchanges_by_this_player: exchanges,
                    pass_policy: PassPolicy::OnlyWhenForced,
                    dynamic_leaves: None,
                };
                let actual = shared
                    .gen_best_moves_for_two_leaves(
                        &params,
                        klv::LeavePair::new([&first, &second]),
                        &mut paired,
                        lean,
                    )
                    .unwrap();
                for (actual, klv) in actual.iter().zip([&first, &second]) {
                    let snapshot = BoardSnapshot {
                        klv,
                        ..board_snapshot
                    };
                    let params = GenMovesParams {
                        board_snapshot: &snapshot,
                        ..params
                    };
                    if lean {
                        separate.gen_moves_unfiltered_lean(&params).unwrap();
                    } else {
                        separate.gen_moves_unfiltered(&params).unwrap();
                    }
                    let expected = &separate.plays[0].play;
                    assert!(
                        actual == expected,
                        "lean={lean}, rack={rack:?}: {} vs {}",
                        actual.fmt(&snapshot).unwrap(),
                        expected.fmt(&snapshot).unwrap()
                    );
                }
                if i <= 1 {
                    assert!(
                        actual[0] != actual[1],
                        "the two tables should choose different plays"
                    );
                }
                if i == 2 {
                    assert!(matches!(&actual[0], Play::Exchange { tiles } if tiles.is_empty()));
                }
            }
        }
    }

    #[inline]
    pub(super) fn test_kad(
        gc: &game_config::GameConfig,
        test_words: &[&str],
    ) -> kwg::Kwg<kwg::Node24> {
        let reader = alphabet::AlphabetReader::new_for_words(gc.alphabet());
        let mut word_buf = Vec::new();
        let mut words = Vec::<bites::Bites>::with_capacity(test_words.len());
        for w in test_words {
            reader.set_word(w, &mut word_buf).unwrap();
            word_buf.sort_unstable();
            words.push(word_buf[..].into());
        }
        words.sort_unstable();
        words.dedup();
        kwg::kbwg_from_bytes_alloc(
            build::build(
                build::BuildContent::DawgOnly,
                build::BuildLayout::Wolges,
                build::BuildOrder::Sorted,
                &words,
            )
            .unwrap()
            .to_vec(),
            false,
        )
        .unwrap()
    }

    #[inline]
    pub(super) fn parse_test_rack(alphabet: &alphabet::Alphabet, rack_str: &str) -> Vec<u8> {
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

    // every placement by brute force, with no word graph: each as its tiles by
    // square and its score, in millipoints
    #[inline]
    fn reference_placements(
        gc: &game_config::GameConfig,
        board_tiles: &[u8],
        rack: &[u8],
        words: &crate::fash::MyHashSet<Vec<u8>>,
    ) -> std::collections::BTreeSet<(Vec<(usize, u8)>, i32)> {
        let alphabet = gc.alphabet();
        let layout = gc.board_layout();
        let dim = layout.dim();
        let premiums = layout.premiums();
        let prefixes = words
            .iter()
            .flat_map(|word| (1..=word.len()).map(|n| word[..n].to_vec()))
            .collect::<crate::fash::MyHashSet<_>>();
        let empty_board = board_tiles.iter().all(|&tile| tile == 0);
        let star = dim.at_on_board(layout.star_row(), layout.star_col());
        let mut held = vec![0u8; alphabet.len() as usize];
        for &tile in rack {
            held[tile as usize] += 1;
        }
        let mut found = std::collections::BTreeSet::new();
        let word_at = |at: (i8, i8), step: (i8, i8), placed: &[(usize, u8)]| {
            let tile_at = |r: i8, c: i8| -> Option<(usize, u8)> {
                if r < 0 || c < 0 || r >= dim.rows || c >= dim.cols {
                    return None;
                }
                let sq = dim.at_on_board(r, c);
                if board_tiles[sq] != 0 {
                    Some((sq, board_tiles[sq]))
                } else {
                    placed.iter().find(|&&(p, _)| p == sq).copied()
                }
            };
            let (mut r, mut c) = at;
            while tile_at(r - step.0, c - step.1).is_some() {
                r -= step.0;
                c -= step.1;
            }
            let mut word = Vec::new();
            while let Some(found) = tile_at(r, c) {
                word.push(found);
                r += step.0;
                c += step.1;
            }
            word
        };
        let score_of = |word: &[(usize, u8)]| -> i32 {
            let mut sum = 0;
            let mut times = 1;
            for &(sq, tile) in word {
                let face = if tile & 0x80 != 0 {
                    0
                } else {
                    alphabet.scaled_score(tile).raw()
                };
                if board_tiles[sq] == 0 {
                    sum += face * i32::from(premiums[sq].tile_multiplier);
                    times *= i32::from(premiums[sq].word_multiplier);
                } else {
                    sum += face;
                }
            }
            sum * times
        };
        // on an empty symmetric board, a down placement is an across one
        // mirrored, and only the across ones are listed
        let directions: &[bool] = if empty_board && layout.is_symmetric() {
            &[false]
        } else {
            &[false, true]
        };
        for &down in directions {
            let (step, cross) = if down {
                ((1, 0), (0, 1))
            } else {
                ((0, 1), (1, 0))
            };
            for r0 in 0..dim.rows {
                for c0 in 0..dim.cols {
                    // (next square, tiles placed so far, main word so far)
                    let mut stack = vec![((r0, c0), Vec::<(usize, u8)>::new(), Vec::<u8>::new())];
                    while let Some(((r, c), placed, main)) = stack.pop() {
                        let on_board = r < dim.rows && c < dim.cols;
                        let sq = on_board.then(|| dim.at_on_board(r, c));
                        if !placed.is_empty() && sq.is_none_or(|sq| board_tiles[sq] == 0) {
                            let mut ok = true;
                            let mut formed = 0;
                            let mut touches = false;
                            let mut score = 0;
                            let main_word = word_at((r - step.0, c - step.1), step, &placed);
                            if main_word.len() >= 2 {
                                formed += 1;
                                touches |= main_word.iter().any(|&(s, _)| board_tiles[s] != 0);
                                ok &= words.contains(
                                    &main_word.iter().map(|&(_, t)| t & 0x7f).collect::<Vec<_>>(),
                                );
                                score += score_of(&main_word);
                            }
                            for &(p, _) in &placed {
                                let (pr, pc) =
                                    ((p / dim.cols as usize) as i8, (p % dim.cols as usize) as i8);
                                let cross_word = word_at((pr, pc), cross, &placed);
                                if cross_word.len() >= 2 {
                                    formed += 1;
                                    touches = true;
                                    ok &= words.contains(
                                        &cross_word
                                            .iter()
                                            .map(|&(_, t)| t & 0x7f)
                                            .collect::<Vec<_>>(),
                                    );
                                    score += score_of(&cross_word);
                                }
                            }
                            let covers = placed.iter().any(|&(s, _)| s == star);
                            if ok && formed > 0 && (touches || (empty_board && covers)) {
                                score += i32::from(gc.num_played_bonus(placed.len() as u8))
                                    * equity::SCALE;
                                let mut key = placed.clone();
                                key.sort_unstable();
                                found.insert((key, score));
                            }
                        }
                        let Some(sq) = sq else {
                            continue;
                        };
                        if placed.is_empty() && main.is_empty() {
                            let (pr, pc) = (r - step.0, c - step.1);
                            if pr >= 0 && pc >= 0 && board_tiles[dim.at_on_board(pr, pc)] != 0 {
                                continue;
                            }
                        }
                        let next = (r + step.0, c + step.1);
                        if board_tiles[sq] != 0 {
                            let mut main = main.clone();
                            main.push(board_tiles[sq] & 0x7f);
                            if prefixes.contains(&main) {
                                stack.push((next, placed, main));
                            }
                            continue;
                        }
                        for tile in 0..alphabet.len() {
                            let used = placed
                                .iter()
                                .filter(
                                    |&&(_, t)| {
                                        if tile == 0 { t & 0x80 != 0 } else { t == tile }
                                    },
                                )
                                .count();
                            if used >= held[tile as usize] as usize {
                                continue;
                            }
                            let letters = if tile == 0 {
                                1..alphabet.len()
                            } else {
                                tile..tile + 1
                            };
                            for letter in letters {
                                let mut main = main.clone();
                                main.push(letter);
                                if !prefixes.contains(&main) {
                                    continue;
                                }
                                let mut placed = placed.clone();
                                placed.push((sq, if tile == 0 { letter | 0x80 } else { letter }));
                                stack.push((next, placed, main));
                            }
                        }
                    }
                }
            }
        }
        found
    }

    #[test]
    #[inline]
    fn the_generators_find_what_brute_force_finds() {
        const LETTERS: [u8; 3] = [1, 5, 20];
        let mut word_list = Vec::<Vec<u8>>::new();
        for &a in &LETTERS {
            for &b in &LETTERS {
                if a != b {
                    word_list.push(vec![a, b]);
                }
                for &c in &LETTERS {
                    if (a, b) != (c, a) {
                        word_list.push(vec![a, b, c]);
                    }
                }
            }
        }
        word_list.push(vec![20, 1, 20, 5]);
        word_list.sort_unstable();
        let bites_list = word_list
            .iter()
            .map(|w| w[..].into())
            .collect::<Vec<bites::Bites>>();
        let words = word_list
            .iter()
            .cloned()
            .collect::<crate::fash::MyHashSet<_>>();
        let kwg_bytes = build::build(
            build::BuildContent::Gaddawg,
            build::BuildLayout::Wolges,
            build::BuildOrder::Sorted,
            &bites_list,
        )
        .unwrap();
        let kwg = kwg::kbwg_from_bytes_alloc(kwg_bytes.to_vec(), false).unwrap();
        let klv = klv::kbwg_from_bytes_alloc(klv::EMPTY_KLV_BYTES.to_vec()).unwrap();
        for (gc, fens) in [
            (
                game_config::make_english_game_config(),
                &[
                    "15/15/15/15/15/15/15/15/15/15/15/15/15/15/15",
                    "15/15/15/15/15/15/15/6TAT6/15/15/15/15/15/15/15",
                    "TE13/A14/15/15/15/15/15/7a7/7T7/15/15/15/15/15/14E",
                ][..],
            ),
            (
                uneven_game_config(5, 9, 2, 4, false),
                &["9/9/9/9/9", "9/9/3EAT3/9/9", "T8/9/9/9/8A"][..],
            ),
        ] {
            let mut fen_parser = display::BoardFenParser::new(gc.alphabet(), gc.board_layout());
            for fen in fens {
                let board_tiles = fen_parser.parse(fen).unwrap().to_vec();
                for rack_text in ["A", "AT", "ETA", "?A", "TATE", "?TE", "A??T"] {
                    let rack = parse_test_rack(gc.alphabet(), rack_text);
                    let want = reference_placements(&gc, &board_tiles, &rack, &words);
                    for lean in [false, true] {
                        let board_snapshot = BoardSnapshot {
                            board_tiles: &board_tiles,
                            game_config: &gc,
                            kwg: &kwg,
                            anagrams: None,
                            rack_lengths: None,
                            klv: &klv,
                        };
                        let mut move_generator = KurniaMoveGenerator::new(&gc);
                        let params = GenMovesParams {
                            board_snapshot: &board_snapshot,
                            rack: &rack,
                            max_gen: usize::MAX,
                            num_exchanges_by_this_player: 0,
                            pass_policy: PassPolicy::OnlyWhenForced,
                            dynamic_leaves: None,
                        };
                        if lean {
                            move_generator.gen_moves_unfiltered_lean(&params).unwrap();
                        } else {
                            move_generator.gen_moves_unfiltered(&params).unwrap();
                        }
                        let dim = gc.board_layout().dim();
                        let got = move_generator
                            .plays
                            .iter()
                            .filter_map(|p| match &p.play {
                                Play::Place {
                                    down,
                                    lane,
                                    idx,
                                    word,
                                    score,
                                } => {
                                    let mut key = word
                                        .iter()
                                        .enumerate()
                                        .filter(|&(_, &t)| t != 0)
                                        .map(|(i, &t)| {
                                            let i = *idx + i as i8;
                                            let sq = if *down {
                                                dim.at_on_board(i, *lane)
                                            } else {
                                                dim.at_on_board(*lane, i)
                                            };
                                            (sq, t)
                                        })
                                        .collect::<Vec<_>>();
                                    key.sort_unstable();
                                    Some((key, score.raw()))
                                }
                                _ => None,
                            })
                            .collect::<std::collections::BTreeSet<_>>();
                        assert_eq!(got, want, "{fen} {rack_text} lean={lean}");
                    }
                }
            }
        }
    }

    #[inline]
    fn placements(fen: &str, rack: &str) -> Vec<String> {
        let gc = game_config::make_english_game_config();
        let kwg = test_kwg(&gc);
        let klv = klv::kbwg_from_bytes_alloc(klv::EMPTY_KLV_BYTES.to_vec()).unwrap();
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
        move_generator
            .gen_moves_unfiltered(&GenMovesParams {
                board_snapshot: &board_snapshot,
                rack: &parse_test_rack(gc.alphabet(), rack),
                max_gen: usize::MAX,
                num_exchanges_by_this_player: 0,
                pass_policy: PassPolicy::OnlyWhenForced,
                dynamic_leaves: None,
            })
            .unwrap();
        let mut out = move_generator
            .plays
            .iter()
            .filter(|p| matches!(p.play, Play::Place { .. }))
            .map(|p| format!("{}", p.play.fmt(&board_snapshot).unwrap()))
            .collect::<Vec<_>>();
        out.sort_unstable();
        out
    }

    #[inline]
    fn valued_plays(fen: &str, rack: &str, max_gen: usize) -> Vec<String> {
        let gc = game_config::make_english_game_config();
        let kwg = test_kwg(&gc);
        let klv = klv::kbwg_from_bytes_alloc(klv::EMPTY_KLV_BYTES.to_vec()).unwrap();
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
        move_generator
            .gen_moves_unfiltered(&GenMovesParams {
                board_snapshot: &board_snapshot,
                rack: &parse_test_rack(gc.alphabet(), rack),
                max_gen,
                num_exchanges_by_this_player: 0,
                pass_policy: PassPolicy::OnlyWhenForced,
                dynamic_leaves: None,
            })
            .unwrap();
        move_generator
            .plays
            .iter()
            .map(|p| {
                format!(
                    "{} {}",
                    p.equity.raw(),
                    p.play.fmt(&board_snapshot).unwrap()
                )
            })
            .collect::<Vec<_>>()
    }

    // a capped generation by the tables, its walk started from the worst of the
    // best plays sure to be pushed (an exchange of each count, the pass when
    // unconditional), keeps what an uncapped one keeps first, over leaves that
    // are not all 0 and a board that fills turn by turn
    #[inline]
    #[test]
    fn a_capped_generation_from_a_floor_keeps_the_uncapped_head() {
        let gc = game_config::make_english_game_config();
        let kwg = sweep_kwg_covering(&gc);
        let held = anagrams::Anagrams::build_for_config(&kwg, &gc)
            .unwrap()
            .unwrap();
        let racks = [
            "AEINRST", "?EIORST", "AADEILN", "EEIRSTT", "ACEHORT", "?AEILNS",
        ];
        let racks = racks
            .iter()
            .map(|rack| parse_test_rack(gc.alphabet(), rack))
            .collect::<Vec<_>>();
        // every subset of each rack, valued by its tiles, up to 6 of them
        let mut leaves = Vec::new();
        for rack in &racks {
            for mask in 1u32..(1 << rack.len()) {
                let mut leave = (0..rack.len())
                    .filter(|&i| mask & (1 << i) != 0)
                    .map(|i| rack[i])
                    .collect::<Vec<_>>();
                if leave.len() <= 6 {
                    leave.sort_unstable();
                    leaves.push(leave);
                }
            }
        }
        leaves.sort_unstable();
        leaves.dedup();
        let valued = leaves
            .iter()
            .map(|leave| {
                let v = leave
                    .iter()
                    .fold(7i32, |a, &t| (a * 31 + i32::from(t)) % 97);
                (&leave[..], (v - 40) as f32 / 4.0)
            })
            .collect::<Vec<_>>();
        let klv = klv::kbwg_from_bytes_alloc(klv::make_klv2(&valued)).unwrap();
        let mut board_tiles = vec![0u8; 225];
        let mut move_generator = KurniaMoveGenerator::new(&gc);
        for rack in &racks {
            let mut best = None;
            for (anagrams, lean) in [(None, false), (Some(&held), false), (Some(&held), true)] {
                let board_snapshot = BoardSnapshot {
                    board_tiles: &board_tiles,
                    game_config: &gc,
                    kwg: &kwg,
                    anagrams,
                    rack_lengths: None,
                    klv: &klv,
                };
                for pass_policy in [PassPolicy::OnlyWhenForced, PassPolicy::AsACandidate] {
                    let mut generate = |max_gen: usize| {
                        let params = GenMovesParams {
                            board_snapshot: &board_snapshot,
                            rack,
                            max_gen,
                            num_exchanges_by_this_player: 0,
                            pass_policy,
                            dynamic_leaves: None,
                        };
                        if lean {
                            move_generator.gen_moves_unfiltered_lean(&params).unwrap();
                        } else {
                            move_generator.gen_moves_unfiltered(&params).unwrap();
                        }
                        move_generator
                            .plays
                            .iter()
                            .map(|p| {
                                format!(
                                    "{} {}",
                                    p.equity.raw(),
                                    p.play.fmt(&board_snapshot).unwrap()
                                )
                            })
                            .collect::<Vec<_>>()
                    };
                    let all = generate(usize::MAX);
                    assert!(all.iter().any(|line| line.contains("xch")), "no exchange");
                    for max_gen in [1, 2, 3, 5, 8, 15] {
                        let head = &all[..max_gen.min(all.len())];
                        assert_eq!(
                            generate(max_gen),
                            head,
                            "at most {max_gen}, tables {}, lean {lean}",
                            anagrams.is_some()
                        );
                    }
                    best = Some(move_generator.plays.first().map(|p| p.play.clone()));
                }
            }
            if let Some(Some(Play::Place {
                down,
                lane,
                idx,
                word,
                ..
            })) = best
            {
                let strider = gc.board_layout().dim().lane(down, lane);
                for (i, &tile) in (idx..).zip(word.iter()) {
                    if tile != 0 {
                        board_tiles[strider.at_in_lane(i)] = tile;
                    }
                }
            }
        }
        assert!(board_tiles.iter().filter(|&&t| t != 0).count() > 10);
    }

    #[inline]
    #[test]
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

    // the letters that, put between the tiles before a square (nearest last)
    // and those after it, make a word, each looked up from the graph's start (a
    // jumbled graph's by its alphagram): a bit each, and bit 0 when there is a
    // run of tiles to make a word with; 0 when there is none
    fn hooks_by_hand(
        kwg: &kwg::Kwg<kwg::Node24>,
        jumbled: bool,
        alphabet_len: u8,
        before: &[u8],
        after: &[u8],
    ) -> u64 {
        if before.is_empty() && after.is_empty() {
            return 0;
        }
        let mut bits = 1u64;
        for letter in 1..alphabet_len {
            let mut word = before.to_vec();
            word.push(letter);
            word.extend_from_slice(after);
            if jumbled {
                word.sort_unstable();
            }
            let mut p = 0;
            if word.iter().all(|&tile| {
                p = kwg.seek(p, tile);
                p > 0
            }) && kwg::Node::accepts(&kwg.node_in_run(p))
            {
                bits |= 1 << letter;
            }
        }
        bits
    }

    // the cross sets gen_hooks gives, for the run down through each empty
    // square and the run across, are what looking each word up finds, under
    // classic rules and under jumbled ones (a graph of alphagrams)
    #[test]
    #[inline]
    fn gen_hooks_gives_the_letters_looking_each_word_up_finds() {
        let boards = [
            "15/15/15/15/15/15/15/6CARE5/15/15/15/15/15/15/15",
            "15/15/15/15/15/15/15/4TRANCE5/15/15/15/15/15/15/15",
            "15/15/15/15/15/15/15/4TRANCE5/15/6NEST5/15/15/15/15/15",
            "15/15/15/15/15/6C8/6A8/4TRANCE5/6E8/6S8/15/15/15/15/15",
        ];
        for jumbled in [false, true] {
            let gc = if jumbled {
                game_config::make_jumbled_english_game_config()
            } else {
                game_config::make_english_game_config()
            };
            let kwg = if jumbled {
                let reader = alphabet::AlphabetReader::new_for_words(gc.alphabet());
                let mut word_buf = Vec::new();
                let words = (SWEEP_WORDS.iter())
                    .map(|w| {
                        reader.set_word(w, &mut word_buf).unwrap();
                        word_buf[..].into()
                    })
                    .collect::<Vec<bites::Bites>>();
                kwg::kbwg_from_bytes_alloc(
                    build::build(
                        build::BuildContent::DawgOnly,
                        build::BuildLayout::Wolges,
                        build::BuildOrder::Sorted,
                        &build::make_alphagrams(&words).unwrap(),
                    )
                    .unwrap()
                    .to_vec(),
                    false,
                )
                .unwrap()
            } else {
                sweep_kwg(&gc)
            };
            let klv = klv::kbwg_from_bytes_alloc(klv::EMPTY_KLV_BYTES.to_vec()).unwrap();
            let mut fen_parser = display::BoardFenParser::new(gc.alphabet(), gc.board_layout());
            let dim = gc.board_layout().dim();
            let alphabet_len = gc.alphabet().len();
            let mut hooked = 0;
            for fen in boards {
                let board_tiles = fen_parser.parse(fen).unwrap().to_vec();
                let mut got = vec![(0u64, 0u64); board_tiles.len()];
                KurniaMoveGenerator::new(&gc).gen_hooks(
                    &BoardSnapshot {
                        board_tiles: &board_tiles,
                        game_config: &gc,
                        kwg: &kwg,
                        anagrams: None,
                        rack_lengths: None,
                        klv: &klv,
                    },
                    |idx, down, across| got[idx] = (down, across),
                );
                let tile_at = |r: i8, c: i8| {
                    if (0..dim.rows).contains(&r) && (0..dim.cols).contains(&c) {
                        board_tiles[dim.at_on_board(r, c)] & 0x7f
                    } else {
                        0
                    }
                };
                let run = |mut r: i8, mut c: i8, dr: i8, dc: i8| {
                    let mut tiles = Vec::new();
                    while tile_at(r + dr, c + dc) != 0 {
                        r += dr;
                        c += dc;
                        tiles.push(tile_at(r, c));
                    }
                    tiles
                };
                for r in 0..dim.rows {
                    for c in 0..dim.cols {
                        if tile_at(r, c) != 0 {
                            continue;
                        }
                        let mut above = run(r, c, -1, 0);
                        above.reverse();
                        let mut left = run(r, c, 0, -1);
                        left.reverse();
                        let want = (
                            hooks_by_hand(&kwg, jumbled, alphabet_len, &above, &run(r, c, 1, 0)),
                            hooks_by_hand(&kwg, jumbled, alphabet_len, &left, &run(r, c, 0, 1)),
                        );
                        assert_eq!(
                            got[dim.at_on_board(r, c)],
                            want,
                            "[{fen}] row {r} col {c}, jumbled {jumbled}"
                        );
                        hooked += usize::from(want.0 | want.1 > 1);
                    }
                }
            }
            assert!(hooked > 0, "jumbled {jumbled}: no square took a letter");
        }
    }

    #[inline]
    fn jumbled_hex_placements(words: &[&[u8]], rack: &[u8]) -> Vec<String> {
        let gc = game_config::make_jumbled_hex_game_config();
        let words = words
            .iter()
            .map(|w| (*w).into())
            .collect::<Vec<bites::Bites>>();
        let kwg = kwg::kbwg_from_bytes_alloc(
            build::build(
                build::BuildContent::DawgOnly,
                build::BuildLayout::Wolges,
                build::BuildOrder::Sorted,
                &build::make_alphagrams(&words).unwrap(),
            )
            .unwrap()
            .to_vec(),
            false,
        )
        .unwrap();
        let klv = klv::kbwg_from_bytes_alloc(klv::EMPTY_KLV_BYTES.to_vec()).unwrap();
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
        move_generator
            .gen_moves_unfiltered(&GenMovesParams {
                board_snapshot: &board_snapshot,
                rack,
                max_gen: usize::MAX,
                num_exchanges_by_this_player: 0,
                pass_policy: PassPolicy::OnlyWhenForced,
                dynamic_leaves: None,
            })
            .unwrap();
        let mut out = move_generator
            .plays
            .iter()
            .filter(|p| matches!(p.play, Play::Place { .. }))
            .map(|p| format!("{}", p.play.fmt(&board_snapshot).unwrap()))
            .collect::<Vec<_>>();
        out.sort_unstable();
        out
    }

    #[inline]
    #[test]
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
            let kwg = kwg::kbwg_from_bytes_alloc(kwg_bytes.to_vec(), false).unwrap();
            let klv = klv::kbwg_from_bytes_alloc(klv::EMPTY_KLV_BYTES.to_vec()).unwrap();
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
            move_generator
                .gen_moves_unfiltered(&GenMovesParams {
                    board_snapshot: &board_snapshot,
                    rack: &rack,
                    max_gen: usize::MAX,
                    num_exchanges_by_this_player: 0,
                    pass_policy: PassPolicy::OnlyWhenForced,
                    dynamic_leaves: None,
                })
                .unwrap();
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
            } => (
                *equity,
                1 + u8::from(*down),
                *lane,
                *idx,
                word.to_vec(),
                score.raw(),
            ),
        };
        let mut sorted = plays_from(build::BuildOrder::Sorted);
        let mut reordered = plays_from(build::BuildOrder::Reordered);
        assert!(sorted.len() > 100, "the test needs plays to compare");
        assert_eq!(sorted.len(), reordered.len());
        sorted.sort_unstable_by_key(sort_key);
        reordered.sort_unstable_by_key(sort_key);
        assert!(
            sorted
                .iter()
                .zip(reordered.iter())
                .all(|(a, b)| a.0 == b.0 && a.1 == b.1),
            "a reordered kwg generated different plays"
        );
    }

    #[inline]
    #[test]
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
        let kwg = kwg::kbwg_from_bytes_alloc(kwg_bytes.to_vec(), false).unwrap();
        let klv = klv::kbwg_from_bytes_alloc(klv::EMPTY_KLV_BYTES.to_vec()).unwrap();
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
        let natural_a_score = alphabet.scaled_score(natural_a).raw();
        assert!(
            natural_a_score > 0,
            "the test needs a natural tile that actually scores points"
        );

        let len = gc.board_layout().dim().cols as usize;
        let output_strider = gc.board_layout().dim().across(0);
        let mut allowed = vec![!1u64; len];
        let mut cross_scores = vec![0i32; len];
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
            &mut allowed,
            &mut cross_scores,
            output_strider.clone(),
            &mut cross_set_buffer,
            &mut cached_cross_sets,
        );
        assert_eq!(cross_scores[1], natural_a_score);
        assert_eq!(cross_scores[3], natural_a_score);

        board_strip[2] = blank_a;
        gen_classic_cross_set(
            &board_snapshot,
            &board_strip,
            &mut allowed,
            &mut cross_scores,
            output_strider,
            &mut cross_set_buffer,
            &mut cached_cross_sets,
        );
        assert_eq!(cross_scores[1], 0);
        assert_eq!(cross_scores[3], 0);
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

    #[inline]
    #[test]
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
        assert!(plays.iter().any(|i| i == "8E SEAT 8"));
        assert_eq!(plays.len(), 50);
    }

    #[test]
    #[inline]
    fn a_generation_fits_a_small_stack() {
        // a generation recurses a frame a tile along a lane, never deeper than
        // the board is wide, so it runs on a thread with far less stack than a
        // main thread has
        let want = placements(
            "15/15/15/15/15/15/15/6SEAT5/15/15/15/15/15/15/15",
            "AEST??T",
        );
        assert!(want.len() > 2);
        let got = std::thread::scope(|s| {
            std::thread::Builder::new()
                .stack_size(256 << 10)
                .spawn_scoped(s, || {
                    placements(
                        "15/15/15/15/15/15/15/6SEAT5/15/15/15/15/15/15/15",
                        "AEST??T",
                    )
                })
                .unwrap()
                .join()
                .unwrap()
        });
        assert_eq!(got, want);
    }

    #[inline]
    #[test]
    fn movegen_hooks_onto_a_word_on_the_board() {
        assert_eq!(
            placements("15/15/15/15/15/15/15/6SEAT5/15/15/15/15/15/15/15", "S"),
            ["8G (SEAT)S 5", "I8 (A)S 3"]
        );
    }

    #[inline]
    #[test]
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
            score: equity::Equity::new(score),
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

    #[inline]
    #[test]
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

    #[inline]
    #[test]
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

    #[inline]
    #[test]
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
        live_pool_into(&mut pool[..n], alphabet, &board_tiles, &rack_tally).unwrap();

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
        live_pool_into(&mut pool[..n], alphabet, &[], &greedy_rack).unwrap();
        assert!(live_pool_into(&mut vec![0; n + 1], alphabet, &[], &[]).is_err());
        assert!(bag_count_from_board(&gc, u16::MAX).is_err());
        assert_eq!(pool[natural_a as usize], 0);
    }
    #[inline]
    fn jumbled_placements_once(fen: &str, rack: &str, is_census: bool) -> Vec<String> {
        let gc = game_config::make_jumbled_english_game_config();
        let kwg = test_kad(&gc, TEST_WORDS);
        let klv = klv::kbwg_from_bytes_alloc(klv::EMPTY_KLV_BYTES.to_vec()).unwrap();
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
        move_generator
            .gen_moves_unfiltered(&GenMovesParams {
                board_snapshot: &board_snapshot,
                rack: &parse_test_rack(gc.alphabet(), rack),
                max_gen: usize::MAX,
                num_exchanges_by_this_player: 0,
                pass_policy: PassPolicy::OnlyWhenForced,
                dynamic_leaves: None,
            })
            .unwrap();
        let mut out = move_generator
            .plays
            .iter()
            .filter(|p| matches!(p.play, Play::Place { .. }))
            .map(|p| format!("{}", p.play.fmt(&board_snapshot).unwrap()))
            .collect::<Vec<_>>();
        out.sort_unstable();
        out
    }

    #[inline(always)]
    fn jumbled_placements(fen: &str, rack: &str) -> Vec<String> {
        jumbled_placements_once(fen, rack, false)
    }

    // a jumbled game is the lean arm's: under the tables it makes the walk's
    // plays in the walk's order from every entry, sorted at every cap, async,
    // and raw
    #[inline]
    #[test]
    fn jumbled_movegen_under_the_tables_plays_as_the_walk() {
        let gc = game_config::make_jumbled_english_game_config();
        let kwg = test_kad(&gc, SWEEP_WORDS);
        let held = anagrams::Anagrams::build_for_config(&kwg, &gc)
            .unwrap()
            .unwrap();
        let klv = klv::kbwg_from_bytes_alloc(klv::EMPTY_KLV_BYTES.to_vec()).unwrap();
        let board_tiles = vec![0u8; 15 * 15];
        let rack = parse_test_rack(gc.alphabet(), "AEINRST");
        let mut made = Vec::new();
        for anagrams in [None, Some(&held)] {
            let board_snapshot = BoardSnapshot {
                board_tiles: &board_tiles,
                game_config: &gc,
                kwg: &kwg,
                anagrams,
                rack_lengths: None,
                klv: &klv,
            };
            let plays = |g: &KurniaMoveGenerator| {
                g.plays
                    .iter()
                    .map(|p| {
                        format!(
                            "{} {}\n",
                            p.equity.raw(),
                            p.play.fmt(&board_snapshot).unwrap()
                        )
                    })
                    .collect::<String>()
            };
            let mut move_generator = KurniaMoveGenerator::new(&gc);
            let mut sorted = Vec::new();
            for max_gen in [1usize, 5, usize::MAX] {
                move_generator
                    .gen_moves_unfiltered(&GenMovesParams {
                        board_snapshot: &board_snapshot,
                        rack: &rack,
                        max_gen,
                        num_exchanges_by_this_player: 0,
                        pass_policy: PassPolicy::OnlyWhenForced,
                        dynamic_leaves: None,
                    })
                    .unwrap();
                sorted.push(plays(&move_generator));
            }
            let params = GenMovesParams {
                board_snapshot: &board_snapshot,
                rack: &rack,
                max_gen: usize::MAX,
                num_exchanges_by_this_player: 0,
                pass_policy: PassPolicy::OnlyWhenForced,
                dynamic_leaves: None,
            };
            {
                let mut fut = std::pin::pin!(move_generator.gen_moves_filtered_async_lean(
                    &params,
                    PlacePredicate::AcceptAll,
                    klv::AdjustLeave::Identity,
                    EquityPredicate::AcceptAll,
                    || std::future::ready(()),
                ));
                let mut cx = std::task::Context::from_waker(std::task::Waker::noop());
                loop {
                    if let std::task::Poll::Ready(result) = fut.as_mut().poll(&mut cx) {
                        result.unwrap();
                        break;
                    }
                }
            }
            sorted.push(plays(&move_generator));
            move_generator
                .gen_moves_raw_all_unsorted(&board_snapshot, &rack, 0, PassPolicy::OnlyWhenForced)
                .unwrap();
            made.push((sorted, plays(&move_generator)));
        }
        assert_eq!(made[0], made[1]);
        assert!(!made[0].1.is_empty(), "generated nothing at all");
    }

    #[inline]
    #[test]
    fn jumbled_movegen_once_per_score_is_every_arrangement_deduplicated() {
        type Key = (bool, i8, i8, usize, Vec<u8>, i32, i32);
        let gc = game_config::make_jumbled_english_game_config();
        let kwg = test_kad(&gc, SWEEP_WORDS);
        let klv = klv::kbwg_from_bytes_alloc(klv::EMPTY_KLV_BYTES.to_vec()).unwrap();
        let board_tiles = vec![0u8; 15 * 15];
        let rack = parse_test_rack(gc.alphabet(), "?ANTS??");
        let board_snapshot = BoardSnapshot {
            board_tiles: &board_tiles,
            game_config: &gc,
            kwg: &kwg,
            anagrams: None,
            rack_lengths: None,
            klv: &klv,
        };
        let mut move_generator = KurniaMoveGenerator::new(&gc);
        let mut generate = |once: bool, max_gen: usize| {
            let params = GenMovesParams {
                board_snapshot: &board_snapshot,
                rack: &rack,
                max_gen,
                num_exchanges_by_this_player: 0,
                pass_policy: PassPolicy::OnlyWhenForced,
                dynamic_leaves: None,
            };
            if once {
                move_generator
                    .gen_moves_filtered_lean_walked::<OncePerScore, _, _>(
                        &params,
                        PlacePredicate::AcceptAll,
                        klv::AdjustLeave::Identity,
                        EquityPredicate::AcceptAll,
                    )
                    .unwrap();
            } else {
                move_generator
                    .gen_moves_filtered_lean_walked::<EveryArrangement, _, _>(
                        &params,
                        PlacePredicate::AcceptAll,
                        klv::AdjustLeave::Identity,
                        EquityPredicate::AcceptAll,
                    )
                    .unwrap();
            }
            move_generator
                .plays
                .iter()
                .filter_map(|p| match &p.play {
                    Play::Place {
                        down,
                        lane,
                        idx,
                        word,
                        score,
                    } => {
                        let mut letters = word
                            .iter()
                            .filter(|&&t| t != 0)
                            .map(|&t| t & 0x7f)
                            .collect::<Vec<_>>();
                        letters.sort_unstable();
                        let placed = word
                            .iter()
                            .filter(|&&t| t != 0)
                            .copied()
                            .collect::<Vec<_>>();
                        let key: Key = (
                            *down,
                            *lane,
                            *idx,
                            word.len(),
                            letters,
                            score.raw(),
                            p.equity.raw(),
                        );
                        Some((key, placed))
                    }
                    _ => None,
                })
                .collect::<Vec<_>>()
        };
        let mut smallest = std::collections::BTreeMap::<Key, Vec<u8>>::new();
        let every = generate(false, usize::MAX);
        assert!(!every.is_empty(), "generated nothing at all");
        for (key, placed) in every.iter().cloned() {
            let kept = smallest.entry(key).or_insert_with(|| placed.clone());
            if placed < *kept {
                *kept = placed;
            }
        }
        assert!(
            every.len() > smallest.len(),
            "no arrangement repeated a tuple, so the test proves nothing"
        );
        let once = generate(true, usize::MAX);
        let made = once
            .iter()
            .cloned()
            .collect::<std::collections::BTreeMap<_, _>>();
        assert_eq!(made.len(), once.len(), "a tuple made twice");
        assert_eq!(made, smallest);
        let best = |plays: &[(Key, Vec<u8>)]| plays.iter().map(|(k, _)| k.6).max();
        let once5 = generate(true, 5);
        let every5 = generate(false, 5);
        assert_eq!(best(&once5), best(&every5));
        let mut tops = once5.iter().map(|(k, _)| k.6).collect::<Vec<_>>();
        let mut groups = made.keys().map(|k| k.6).collect::<Vec<_>>();
        groups.sort_unstable_by(|a, b| b.cmp(a));
        groups.truncate(tops.len());
        tops.sort_unstable_by(|a, b| b.cmp(a));
        assert_eq!(tops, groups);
    }

    #[inline]
    #[test]
    fn jumbled_movegen_opens_with_either_arrangement() {
        assert_eq!(
            jumbled_placements("15/15/15/15/15/15/15/15/15/15/15/15/15/15/15", "AT"),
            ["8G AT 4", "8G TA 4", "8H AT 4", "8H TA 4"]
        );
    }

    #[inline]
    #[test]
    fn jumbled_movegen_lets_both_blanks_be_one_letter_not_held() {
        // SEATS: both blanks an S
        assert!(
            jumbled_placements("15/15/15/15/15/15/15/15/15/15/15/15/15/15/15", "??EAT")
                .iter()
                .any(|play| play.split(' ').nth(1).is_some_and(|word| word.len() == 5))
        );
    }

    #[inline]
    #[test]
    fn jumbled_movegen_opens_with_a_blank() {
        assert_eq!(
            jumbled_placements("15/15/15/15/15/15/15/15/15/15/15/15/15/15/15", "?A"),
            [
                "8G As 2", "8G At 2", "8G sA 2", "8G tA 2", "8H As 2", "8H At 2", "8H sA 2",
                "8H tA 2",
            ]
        );
    }

    #[inline]
    #[test]
    fn jumbled_movegen_takes_any_arrangement_of_a_word() {
        assert_eq!(
            jumbled_placements("15/15/15/15/15/15/15/6SEAT5/15/15/15/15/15/15/15", "S"),
            ["8F S(SEAT) 5", "8G (SEAT)S 5", "I7 S(A) 3", "I8 (A)S 3"]
        );
    }

    #[inline]
    #[test]
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

    #[inline]
    #[test]
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

    #[inline]
    #[test]
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

    #[inline]
    fn jumbled_plays(
        words: &[&str],
        tiles: &[(usize, usize, &str)],
        rack: &str,
        is_census: bool,
    ) -> Vec<String> {
        let gc = game_config::make_jumbled_english_game_config();
        let alphabet = gc.alphabet();
        let word_reader = alphabet::AlphabetReader::new_for_words(alphabet);
        let play_reader = alphabet::AlphabetReader::new_for_plays(alphabet);
        let mut buf = Vec::new();
        let mut machine_words = Vec::<bites::Bites>::new();
        for word in words {
            word_reader.set_word(word, &mut buf).unwrap();
            machine_words.push(buf[..].into());
        }
        machine_words.sort_unstable();
        machine_words.dedup();
        let kwg = kwg::kbwg_from_bytes_alloc(
            build::build(
                build::BuildContent::DawgOnly,
                build::BuildLayout::Wolges,
                build::BuildOrder::Sorted,
                &build::make_alphagrams(&machine_words).unwrap(),
            )
            .unwrap()
            .to_vec(),
            false,
        )
        .unwrap();
        let klv = klv::kbwg_from_bytes_alloc(klv::EMPTY_KLV_BYTES.to_vec()).unwrap();
        let dim = gc.board_layout().dim();
        let mut board_tiles = vec![0u8; dim.rows as usize * dim.cols as usize];
        for &(row, col, tile) in tiles {
            play_reader.set_word(tile, &mut buf).unwrap();
            assert_eq!(buf.len(), 1, "one board square takes one tile");
            board_tiles[row * dim.cols as usize + col] = buf[0];
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
        move_generator.working_buffer.is_census = is_census;
        move_generator
            .gen_moves_unfiltered(&GenMovesParams {
                board_snapshot: &board_snapshot,
                rack: &parse_test_rack(alphabet, rack),
                max_gen: usize::MAX,
                num_exchanges_by_this_player: 0,
                pass_policy: PassPolicy::OnlyWhenForced,
                dynamic_leaves: None,
            })
            .unwrap();
        move_generator
            .plays
            .iter()
            .map(|p| format!("{}", p.play.fmt(&board_snapshot).unwrap()))
            .collect()
    }

    #[inline]
    #[test]
    fn jumbled_movegen_lists_every_play_from_the_star() {
        assert_eq!(
            jumbled_plays(&["AB", "ABC", "AC"], &[], "AB", false),
            [
                "8G BA 8", "8H AB 8", "8G AB 8", "8H BA 8", "Exch. A", "Exch. AB", "Exch. B",
            ]
        );
    }

    #[inline]
    #[test]
    fn jumbled_movegen_lists_hooks_and_one_tile_plays() {
        assert_eq!(
            jumbled_plays(&["AB", "ABC", "AC", "BC"], &[(7, 7, "A")], "BC", false),
            [
                "7G BC 13",
                "7G CB 13",
                "7H BC 13",
                "7H CB 13",
                "9G BC 13",
                "9G CB 13",
                "9H BC 13",
                "9H CB 13",
                "G7 BC 13",
                "G7 CB 13",
                "G8 BC 13",
                "G8 CB 13",
                "I7 BC 13",
                "I7 CB 13",
                "I8 BC 13",
                "I8 CB 13",
                "8F BC(A) 7",
                "8F CB(A) 7",
                "8G B(A)C 7",
                "8G C(A)B 7",
                "8H (A)BC 7",
                "8H (A)CB 7",
                "H6 BC(A) 7",
                "H6 CB(A) 7",
                "H7 B(A)C 7",
                "H7 C(A)B 7",
                "H8 (A)BC 7",
                "H8 (A)CB 7",
                "8G B(A) 4",
                "8G C(A) 4",
                "8H (A)B 4",
                "8H (A)C 4",
                "H7 B(A) 4",
                "H7 C(A) 4",
                "H8 (A)B 4",
                "H8 (A)C 4",
                "Exch. B",
                "Exch. BC",
                "Exch. C",
            ]
        );
    }

    #[inline]
    #[test]
    fn jumbled_movegen_spans_two_islands() {
        assert_eq!(
            jumbled_plays(
                &["ABC", "ABCD", "ABCDE"],
                &[(7, 6, "A"), (7, 8, "C")],
                "BD",
                false
            ),
            [
                "8F B(A)D(C) 18",
                "8F D(A)B(C) 18",
                "8G (A)B(C)D 18",
                "8G (A)D(C)B 18",
                "8G (A)B(C) 14",
                "Exch. B",
                "Exch. BD",
                "Exch. D",
            ]
        );
    }

    #[inline]
    #[test]
    fn jumbled_movegen_lets_a_blank_stand_for_any_letter() {
        assert_eq!(
            jumbled_plays(&["AB", "ABC"], &[(7, 7, "A")], "B?", false),
            [
                "7G aB 7",
                "7H Ba 7",
                "9G aB 7",
                "9H Ba 7",
                "G7 aB 7",
                "G8 Ba 7",
                "I7 aB 7",
                "I8 Ba 7",
                "8F Bc(A) 4",
                "8F cB(A) 4",
                "8G B(A) 4",
                "8G B(A)c 4",
                "8G c(A)B 4",
                "8H (A)B 4",
                "8H (A)Bc 4",
                "8H (A)cB 4",
                "H6 Bc(A) 4",
                "H6 cB(A) 4",
                "H7 B(A) 4",
                "H7 B(A)c 4",
                "H7 c(A)B 4",
                "H8 (A)B 4",
                "H8 (A)Bc 4",
                "H8 (A)cB 4",
                "8G b(A) 1",
                "8H (A)b 1",
                "H7 b(A) 1",
                "H8 (A)b 1",
                "Exch. ?",
                "Exch. ?B",
                "Exch. B",
            ]
        );
    }

    #[inline]
    #[test]
    fn jumbled_movegen_census_drops_the_blank_a_real_tile_can_replace() {
        assert_eq!(
            jumbled_plays(&["AB", "ABC"], &[(7, 7, "A")], "B?", true),
            [
                "7G aB 7",
                "7H Ba 7",
                "9G aB 7",
                "9H Ba 7",
                "G7 aB 7",
                "G8 Ba 7",
                "I7 aB 7",
                "I8 Ba 7",
                "8F Bc(A) 4",
                "8F cB(A) 4",
                "8G B(A) 4",
                "8G B(A)c 4",
                "8G c(A)B 4",
                "8H (A)B 4",
                "8H (A)Bc 4",
                "8H (A)cB 4",
                "H6 Bc(A) 4",
                "H6 cB(A) 4",
                "H7 B(A) 4",
                "H7 B(A)c 4",
                "H7 c(A)B 4",
                "H8 (A)B 4",
                "H8 (A)Bc 4",
                "H8 (A)cB 4",
                "Exch. ?",
                "Exch. ?B",
                "Exch. B",
            ]
        );
    }

    #[inline]
    #[test]
    fn jumbled_movegen_stops_a_span_at_a_dead_square() {
        assert_eq!(
            jumbled_plays(
                &["AB", "ABC", "ABCD"],
                &[(7, 7, "A"), (6, 5, "X")],
                "BCD",
                false
            ),
            [
                "8G B(A)CD 9",
                "8G B(A)DC 9",
                "8G C(A)BD 9",
                "8G C(A)DB 9",
                "8G D(A)BC 9",
                "8G D(A)CB 9",
                "8H (A)BCD 9",
                "8H (A)BDC 9",
                "8H (A)CBD 9",
                "8H (A)CDB 9",
                "8H (A)DBC 9",
                "8H (A)DCB 9",
                "H5 BCD(A) 9",
                "H5 BDC(A) 9",
                "H5 CBD(A) 9",
                "H5 CDB(A) 9",
                "H5 DBC(A) 9",
                "H5 DCB(A) 9",
                "H6 BC(A)D 9",
                "H6 BD(A)C 9",
                "H6 CB(A)D 9",
                "H6 CD(A)B 9",
                "H6 DB(A)C 9",
                "H6 DC(A)B 9",
                "H7 B(A)CD 9",
                "H7 B(A)DC 9",
                "H7 C(A)BD 9",
                "H7 C(A)DB 9",
                "H7 D(A)BC 9",
                "H7 D(A)CB 9",
                "H8 (A)BCD 9",
                "H8 (A)BDC 9",
                "H8 (A)CBD 9",
                "H8 (A)CDB 9",
                "H8 (A)DBC 9",
                "H8 (A)DCB 9",
                "8G B(A)C 7",
                "8G C(A)B 7",
                "8H (A)BC 7",
                "8H (A)CB 7",
                "H6 BC(A) 7",
                "H6 CB(A) 7",
                "H7 B(A)C 7",
                "H7 C(A)B 7",
                "H8 (A)BC 7",
                "H8 (A)CB 7",
                "8G B(A) 4",
                "8H (A)B 4",
                "H7 B(A) 4",
                "H8 (A)B 4",
                "Exch. B",
                "Exch. BC",
                "Exch. BCD",
                "Exch. BD",
                "Exch. C",
                "Exch. CD",
                "Exch. D",
            ]
        );
    }

    #[inline]
    #[test]
    fn a_generation_after_remaining_words_matches_a_fresh_one() {
        let gc = game_config::make_english_game_config();
        let kwg = test_kwg(&gc);
        let klv = klv::kbwg_from_bytes_alloc(klv::EMPTY_KLV_BYTES.to_vec()).unwrap();
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
            move_generator
                .gen_moves_unfiltered(&GenMovesParams {
                    board_snapshot: &seat_snapshot,
                    rack: &rack,
                    max_gen: usize::MAX,
                    num_exchanges_by_this_player: 0,
                    pass_policy: PassPolicy::OnlyWhenForced,
                    dynamic_leaves: None,
                })
                .unwrap();
            move_generator
                .plays
                .iter()
                .map(|p| format!("{} {}", p.equity.raw(), p.play.fmt(&seat_snapshot).unwrap()))
                .collect::<Vec<_>>()
        };
        let fresh = plays(&mut KurniaMoveGenerator::new(&gc));
        let mut move_generator = KurniaMoveGenerator::new(&gc);
        assert_eq!(plays(&mut move_generator), fresh);
        move_generator
            .gen_remaining_words(&empty_snapshot, |_| {})
            .unwrap();
        assert_eq!(plays(&mut move_generator), fresh);
    }

    // a word prune under a memory limit, classic and jumbled, either gives
    // every word the prune with none gives, or is refused and gives none; a
    // limit of a byte refuses it, one of every byte never does
    #[inline]
    #[test]
    fn a_word_prune_under_a_limit_gives_every_word_or_none() {
        let held = || 0;
        let classic = game_config::make_english_game_config();
        let jumbled = game_config::make_jumbled_english_game_config();
        let alphagrams = kwg::kbwg_from_bytes_alloc(
            build::build(
                build::BuildContent::DawgOnly,
                build::BuildLayout::Wolges,
                build::BuildOrder::Sorted,
                &build::make_alphagrams(
                    &SWEEP_WORDS
                        .iter()
                        .map(|word| {
                            let mut buf = Vec::new();
                            alphabet::AlphabetReader::new_for_words(jumbled.alphabet())
                                .set_word(word, &mut buf)
                                .unwrap();
                            buf[..].into()
                        })
                        .collect::<Vec<bites::Bites>>(),
                )
                .unwrap(),
            )
            .unwrap()
            .to_vec(),
            false,
        )
        .unwrap();
        let klv = klv::kbwg_from_bytes_alloc(klv::EMPTY_KLV_BYTES.to_vec()).unwrap();
        for (gc, kwg) in [(&classic, sweep_kwg(&classic)), (&jumbled, alphagrams)] {
            let mut fen_parser = display::BoardFenParser::new(gc.alphabet(), gc.board_layout());
            let board = fen_parser
                .parse("15/15/15/15/15/15/15/6SEAT5/7A7/7T7/15/15/15/15/15")
                .unwrap()
                .to_vec();
            let snapshot = BoardSnapshot {
                board_tiles: &board,
                game_config: gc,
                kwg: &kwg,
                anagrams: None,
                rack_lengths: None,
                klv: &klv,
            };
            let mut move_generator = KurniaMoveGenerator::new(gc);
            let mut every = Vec::new();
            move_generator
                .gen_remaining_words(&snapshot, |word| every.push(word.to_vec()))
                .unwrap();
            assert!(!every.is_empty());
            let mut refusals = Vec::new();
            for bytes in [1, 64, 256, 1024, 4096, 16384, usize::MAX] {
                let memory = endgame::MemoryLimit {
                    bytes,
                    held,
                    reserved: None,
                };
                let mut words = Vec::new();
                let pruned =
                    move_generator.gen_remaining_words_within(&snapshot, Some(&memory), |word| {
                        words.push(word.to_vec())
                    });
                if pruned.is_ok() {
                    assert_eq!(words, every, "{bytes}");
                } else {
                    assert!(words.is_empty(), "{bytes}");
                }
                refusals.push(pruned.is_err());
            }
            assert!(refusals[0] && !refusals[refusals.len() - 1], "{refusals:?}");
        }
    }

    #[test]
    #[inline]
    fn a_generation_after_an_empty_board_matches_a_fresh_one() {
        let gc = game_config::make_english_game_config();
        let kwg = test_kwg(&gc);
        let klv = klv::kbwg_from_bytes_alloc(klv::EMPTY_KLV_BYTES.to_vec()).unwrap();
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
            move_generator
                .gen_moves_unfiltered(&GenMovesParams {
                    board_snapshot: &board_snapshot,
                    rack: &rack,
                    max_gen: usize::MAX,
                    num_exchanges_by_this_player: 0,
                    pass_policy: PassPolicy::OnlyWhenForced,
                    dynamic_leaves: None,
                })
                .unwrap();
            move_generator
                .plays
                .iter()
                .map(|p| {
                    format!(
                        "{} {}",
                        p.equity.raw(),
                        p.play.fmt(&board_snapshot).unwrap()
                    )
                })
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

    // a generator kept from board to board, reading the anagram tables,
    // generates what a fresh one does on each
    #[test]
    #[inline]
    fn a_kept_generator_with_the_tables_matches_a_fresh_one() {
        let gc = game_config::make_english_game_config();
        let kwg = sweep_kwg_covering(&gc);
        let held = anagrams::Anagrams::build_for_config(&kwg, &gc)
            .unwrap()
            .unwrap();
        let klv = klv::kbwg_from_bytes_alloc(klv::EMPTY_KLV_BYTES.to_vec()).unwrap();
        let mut fen_parser = display::BoardFenParser::new(gc.alphabet(), gc.board_layout());
        let boards = [
            "15/15/15/15/15/15/15/15/15/15/15/15/15/15/15",
            "15/15/15/15/15/15/15/6CARE5/15/15/15/15/15/15/15",
            "15/15/15/15/15/15/15/6CARE5/6A8/6N8/6E8/15/15/15/15",
            "15/15/15/15/15/15/15/6CARE5/6A8/6N8/6EATS5/15/15/15/15",
            "15/15/15/15/15/15/15/6N3A4/15/6N8/15/15/15/15/15",
            "15/15/15/15/15/15/15/6CARE5/6A8/6N8/6E8/15/15/15/15",
            "15/15/15/15/15/15/15/15/15/15/15/15/15/15/15",
            "15/15/15/15/15/15/15/6CARE5/6A8/6N8/6EATS5/15/15/15/15",
        ]
        .map(|fen| fen_parser.parse(fen).unwrap().to_vec());
        let racks = ["?EINRST", "AEINRST", "CARTONS", "??TANS"];
        let plays = |move_generator: &mut KurniaMoveGenerator, board_tiles: &[u8], rack: &str| {
            let board_snapshot = BoardSnapshot {
                board_tiles,
                game_config: &gc,
                kwg: &kwg,
                anagrams: Some(&held),
                rack_lengths: None,
                klv: &klv,
            };
            move_generator
                .gen_moves_unfiltered(&GenMovesParams {
                    board_snapshot: &board_snapshot,
                    rack: &parse_test_rack(gc.alphabet(), rack),
                    max_gen: usize::MAX,
                    num_exchanges_by_this_player: 0,
                    pass_policy: PassPolicy::OnlyWhenForced,
                    dynamic_leaves: None,
                })
                .unwrap();
            move_generator
                .plays
                .iter()
                .map(|p| {
                    format!(
                        "{} {}",
                        p.equity.raw(),
                        p.play.fmt(&board_snapshot).unwrap()
                    )
                })
                .collect::<Vec<_>>()
        };
        let mut kept = KurniaMoveGenerator::new(&gc);
        for (i, board_tiles) in boards.iter().enumerate() {
            for rack in [racks[i % racks.len()], racks[(i + 1) % racks.len()]] {
                assert_eq!(
                    plays(&mut kept, board_tiles, rack),
                    plays(&mut KurniaMoveGenerator::new(&gc), board_tiles, rack),
                    "board {i}, rack {rack}"
                );
            }
        }
    }

    // with a 64-tile alphabet, a square whose perpendicular word allows every
    // letter is constrained, not free, in the table walk as in its sums
    #[test]
    #[inline]
    fn a_cross_set_allowing_all_63_letters_is_not_a_free_square() {
        let mut gc = game_config::make_english_game_config();
        let tiles = std::iter::once("?\t?\t2\t0\t0\t0\t0\n".to_string())
            .chain((1..64).map(|i| format!("L{i}\tl{i}\t1\t1\t0\t0\t0\n")))
            .collect::<String>();
        let game_config::GameConfig::Static(rules) = &mut gc;
        rules.alphabet = alphabet::Alphabet::new_static_from_text(&tiles).unwrap();
        assert_eq!(gc.alphabet().len(), 64);
        let mut words = Vec::<bites::Bites>::new();
        for a in 1..64u8 {
            for b in 1..64u8 {
                words.push([a, b][..].into());
            }
        }
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
        let held = anagrams::Anagrams::build_for_config(&kwg, &gc)
            .unwrap()
            .unwrap();
        let klv = klv::kbwg_from_bytes_alloc(klv::EMPTY_KLV_BYTES.to_vec()).unwrap();
        let dim = gc.board_layout().dim();
        let mut board_tiles = vec![0u8; dim.rows as usize * dim.cols as usize];
        board_tiles[dim.at_on_board(7, 7)] = 1;
        let plays = |anagrams| {
            let board_snapshot = BoardSnapshot {
                board_tiles: &board_tiles,
                game_config: &gc,
                kwg: &kwg,
                anagrams,
                rack_lengths: None,
                klv: &klv,
            };
            let mut move_generator = KurniaMoveGenerator::new(&gc);
            move_generator
                .gen_moves_unfiltered(&GenMovesParams {
                    board_snapshot: &board_snapshot,
                    rack: &[2, 3],
                    max_gen: usize::MAX,
                    num_exchanges_by_this_player: 0,
                    pass_policy: PassPolicy::OnlyWhenForced,
                    dynamic_leaves: None,
                })
                .unwrap();
            (move_generator.plays.iter())
                .map(|p| {
                    format!(
                        "{} {}",
                        p.equity.raw(),
                        p.play.fmt(&board_snapshot).unwrap()
                    )
                })
                .collect::<Vec<_>>()
        };
        let walked = plays(Some(&held));
        assert!(!walked.is_empty());
        assert_eq!(walked, plays(None));
    }

    // a generator kept across boards, some generated with the tables and some
    // without (so with no look by the table walk at its lanes), boards coming
    // back to one seen before, generates as a fresh one each time: span sums
    // kept from an earlier generation are taken only for a lane as it was
    #[test]
    #[inline]
    fn a_kept_generator_between_arms_matches_a_fresh_one() {
        let gc = game_config::make_english_game_config();
        let kwg = sweep_kwg_covering(&gc);
        let held = anagrams::Anagrams::build_for_config(&kwg, &gc)
            .unwrap()
            .unwrap();
        let rack_lengths = anagrams::RackLengths::build_for_config(&kwg, &gc)
            .unwrap()
            .unwrap();
        let klv = klv::kbwg_from_bytes_alloc(klv::EMPTY_KLV_BYTES.to_vec()).unwrap();
        let mut fen_parser = display::BoardFenParser::new(gc.alphabet(), gc.board_layout());
        // each board, and whether the tables generate on it
        let steps = [
            ("15/15/15/15/15/15/15/15/15/15/15/15/15/15/15", true),
            ("15/15/15/15/15/15/15/6CARE5/15/15/15/15/15/15/15", true),
            ("15/15/15/15/15/15/15/6CARE5/6A8/6N8/6E8/15/15/15/15", false),
            ("15/15/15/15/15/15/15/6CARE5/15/15/15/15/15/15/15", true),
            (
                "15/15/15/15/15/15/15/6CARE5/6A8/6N8/6EATS5/15/15/15/15",
                false,
            ),
            ("15/15/15/15/15/15/15/6CARE5/6A8/6N8/6E8/15/15/15/15", true),
            (
                "15/15/15/15/15/15/15/6CARE5/6A8/6N8/6EATS5/15/15/15/15",
                true,
            ),
            ("15/15/15/15/15/15/15/6N3A4/15/6N8/15/15/15/15/15", false),
            (
                "15/15/15/15/15/15/15/6CARE5/6A8/6N8/6EATS5/15/15/15/15",
                true,
            ),
            ("15/15/15/15/15/15/15/6CARE5/6A8/6N8/6E8/15/15/15/15", true),
        ]
        .map(|(fen, tables)| (fen_parser.parse(fen).unwrap().to_vec(), tables));
        let racks = ["AEINRST", "CARTONS", "?EINRST", "DEILOPS"];
        let plays = |move_generator: &mut KurniaMoveGenerator,
                     board_tiles: &[u8],
                     tables: bool,
                     rack: &str| {
            let board_snapshot = BoardSnapshot {
                board_tiles,
                game_config: &gc,
                kwg: &kwg,
                anagrams: tables.then_some(&held),
                rack_lengths: (!tables).then_some(&rack_lengths),
                klv: &klv,
            };
            move_generator
                .gen_moves_unfiltered(&GenMovesParams {
                    board_snapshot: &board_snapshot,
                    rack: &parse_test_rack(gc.alphabet(), rack),
                    max_gen: usize::MAX,
                    num_exchanges_by_this_player: 0,
                    pass_policy: PassPolicy::OnlyWhenForced,
                    dynamic_leaves: None,
                })
                .unwrap();
            move_generator
                .plays
                .iter()
                .map(|p| {
                    format!(
                        "{} {}",
                        p.equity.raw(),
                        p.play.fmt(&board_snapshot).unwrap()
                    )
                })
                .collect::<Vec<_>>()
        };
        let mut kept = KurniaMoveGenerator::new(&gc);
        for (i, (board_tiles, tables)) in steps.iter().enumerate() {
            let rack = racks[i % racks.len()];
            assert_eq!(
                plays(&mut kept, board_tiles, *tables, rack),
                plays(
                    &mut KurniaMoveGenerator::new(&gc),
                    board_tiles,
                    *tables,
                    rack
                ),
                "step {i}, rack {rack}"
            );
        }
    }

    // a generator cloned from one that has generated, or made it by clone_from
    // after a board of its own, generates what its source does on the source's
    // board: the tables kept between generations go with it
    #[test]
    #[inline]
    fn a_cloned_generator_generates_as_its_source_does() {
        let gc = game_config::make_english_game_config();
        let kwg = sweep_kwg_covering(&gc);
        let held = anagrams::Anagrams::build_for_config(&kwg, &gc)
            .unwrap()
            .unwrap();
        let rack_lengths = anagrams::RackLengths::build_for_config(&kwg, &gc)
            .unwrap()
            .unwrap();
        let klv = klv::kbwg_from_bytes_alloc(klv::EMPTY_KLV_BYTES.to_vec()).unwrap();
        let mut fen_parser = display::BoardFenParser::new(gc.alphabet(), gc.board_layout());
        let three = fen_parser
            .parse("15/15/15/15/15/15/15/6CARE5/6A8/6N8/6EATS5/15/15/15/15")
            .unwrap()
            .to_vec();
        let dead = fen_parser
            .parse("15/15/15/15/15/15/15/6N3A4/15/6N8/15/15/15/15/15")
            .unwrap()
            .to_vec();
        let rack = parse_test_rack(gc.alphabet(), "?EINRST");
        for anagrams in [None, Some(&held)] {
            let plays = |move_generator: &mut KurniaMoveGenerator, board_tiles: &[u8]| {
                let board_snapshot = BoardSnapshot {
                    board_tiles,
                    game_config: &gc,
                    kwg: &kwg,
                    anagrams,
                    rack_lengths: anagrams.is_none().then_some(&rack_lengths),
                    klv: &klv,
                };
                move_generator
                    .gen_moves_unfiltered(&GenMovesParams {
                        board_snapshot: &board_snapshot,
                        rack: &rack,
                        max_gen: usize::MAX,
                        num_exchanges_by_this_player: 0,
                        pass_policy: PassPolicy::OnlyWhenForced,
                        dynamic_leaves: None,
                    })
                    .unwrap();
                move_generator
                    .plays
                    .iter()
                    .map(|p| {
                        format!(
                            "{} {}",
                            p.equity.raw(),
                            p.play.fmt(&board_snapshot).unwrap()
                        )
                    })
                    .collect::<Vec<_>>()
            };
            let mut source = KurniaMoveGenerator::new(&gc);
            let want = plays(&mut source, &three);
            assert!(want.len() > 100, "too few plays to tell anything apart");
            assert_eq!(plays(&mut source.clone(), &three), want);
            let mut copied = KurniaMoveGenerator::new(&gc);
            assert_ne!(plays(&mut copied, &dead), want);
            copied.clone_from(&source);
            assert_eq!(plays(&mut copied, &three), want);
        }
    }

    #[inline]
    fn uneven_premium(row: i8, col: i8) -> board_layout::Premium {
        let (row, col) = (i32::from(row), i32::from(col));
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
    pub(super) fn uneven_game_config(
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

    // the woadwage toy: tiles 1 to 4 of the english alphabet
    const WOADWAGE_TOY_WORDS: [&[u8]; 5] = [&[1], &[1, 1], &[1, 2], &[2, 3], &[3, 2, 1]];

    #[inline]
    pub(crate) fn woadwage_toy_letters() -> u8 {
        WOADWAGE_TOY_WORDS
            .iter()
            .flat_map(|word| word.iter())
            .copied()
            .max()
            .unwrap()
    }

    #[inline]
    pub(crate) fn woadwage_toy_most_tiles() -> u64 {
        woadwage_toy_letters() as u64
    }

    #[inline]
    pub(crate) fn woadwage_toy_longest() -> u64 {
        WOADWAGE_TOY_WORDS
            .iter()
            .map(|word| word.len())
            .max()
            .unwrap() as u64
    }

    #[inline]
    pub(crate) fn woadwage_toy() -> (kwg::Kwg<kwg::Node24>, std::collections::BTreeSet<Vec<u8>>) {
        let words = WOADWAGE_TOY_WORDS
            .iter()
            .map(|&word| bites::Bites::from(word))
            .collect::<Vec<_>>();
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
        let explicit = WOADWAGE_TOY_WORDS
            .iter()
            .map(|&word| word.to_vec())
            .chain(
                WOADWAGE_TOY_WORDS
                    .iter()
                    .flat_map(|&a| WOADWAGE_TOY_WORDS.iter().map(move |&b| [a, b].concat())),
            )
            .collect();
        (kwg, explicit)
    }

    #[inline]
    pub(crate) fn woadwage_toy_config(side: i8) -> game_config::GameConfig {
        let mut premiums = Vec::new();
        for row in 0..side {
            for col in 0..side {
                let (word_multiplier, tile_multiplier) = match (row * 3 + col * 5) % 7 {
                    0 => (2, 1),
                    1 => (1, 3),
                    2 => (1, 2),
                    _ => (1, 1),
                };
                premiums.push(board_layout::Premium {
                    word_multiplier,
                    tile_multiplier,
                });
            }
        }
        game_config::make_board_test_game_config(board_layout::make_test_board_layout(
            premiums.into_boxed_slice(),
            matrix::Dim {
                rows: side,
                cols: side,
            },
            side / 2,
            side / 2,
        ))
        .woadwage()
    }

    #[inline]
    pub(crate) fn woadwage_rng(seed: &mut u64) -> u64 {
        *seed = seed
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        *seed >> 33
    }

    #[test]
    #[inline]
    fn a_woadwage_cross_set_takes_each_letter_its_cross_word_accepts() {
        let (kwg, explicit) = woadwage_toy();
        let klv = klv::kbwg_from_bytes_alloc(klv::EMPTY_KLV_BYTES.to_vec()).unwrap();
        let mut seed = 175u64;
        let mut squares = 0usize;
        for side in [5i8, 7] {
            let gc = woadwage_toy_config(side);
            let dim = gc.board_layout().dim();
            let n = side as usize;
            let alphabet_len = gc.alphabet().len();
            let mut move_generator = KurniaMoveGenerator::new(&gc);
            for _ in 0..40 {
                let mut board_tiles = vec![0u8; n * n];
                for tile in board_tiles.iter_mut() {
                    if woadwage_rng(&mut seed).is_multiple_of(3) {
                        let letter =
                            1 + (woadwage_rng(&mut seed) % woadwage_toy_letters() as u64) as u8;
                        *tile = letter
                            | if woadwage_rng(&mut seed).is_multiple_of(5) {
                                0x80
                            } else {
                                0
                            };
                    }
                }
                if board_tiles.iter().all(|&tile| tile == 0) {
                    board_tiles[0] = 1;
                }
                let board_snapshot = BoardSnapshot {
                    board_tiles: &board_tiles,
                    game_config: &gc,
                    kwg: &kwg,
                    anagrams: None,
                    rack_lengths: None,
                    klv: &klv,
                };
                place_plays(&mut move_generator, &board_snapshot, &[1, 2], false);
                let wb = &move_generator.working_buffer;
                for row in 0..side {
                    for col in 0..side {
                        let at = dim.at_on_board(row, col);
                        if board_tiles[at] != 0 {
                            continue;
                        }
                        let run = |down: bool, step: i8| {
                            let mut tiles = Vec::new();
                            let (mut r, mut c) = (row, col);
                            loop {
                                if down {
                                    r += step;
                                } else {
                                    c += step;
                                }
                                if !(0..side).contains(&r) || !(0..side).contains(&c) {
                                    break;
                                }
                                let tile = board_tiles[dim.at_on_board(r, c)];
                                if tile == 0 {
                                    break;
                                }
                                tiles.push(tile & 0x7f);
                            }
                            if step < 0 {
                                tiles.reverse();
                            }
                            tiles
                        };
                        for (down_word, got) in [
                            (true, wb.allowed_for_across_plays[at]),
                            (
                                false,
                                wb.allowed_for_down_plays[(col as usize) * n + row as usize],
                            ),
                        ] {
                            let (before, after) = (run(down_word, -1), run(down_word, 1));
                            let want = if before.is_empty() && after.is_empty() {
                                !1
                            } else {
                                let mut bits = 1u64;
                                for letter in 1..alphabet_len {
                                    let word = [&before[..], &[letter], &after[..]].concat();
                                    assert_eq!(
                                        woadwage::accepts(&kwg, &word),
                                        explicit.contains(&word),
                                        "{word:?}"
                                    );
                                    if woadwage::accepts(&kwg, &word) {
                                        bits |= 1 << letter;
                                    }
                                }
                                bits
                            };
                            assert_eq!(got, want, "{side} {row} {col} {down_word}");
                            squares += 1;
                        }
                    }
                }
            }
        }
        assert!(squares > 0, "{squares}");
    }

    pub(crate) struct WoadwageBrute(
        pub(crate) Vec<(bool, i8, i8, Vec<u8>)>,
        pub(crate) std::collections::BTreeSet<(i8, i8, u8)>,
    );

    #[inline]
    pub(crate) fn woadwage_brute_force(
        side: i8,
        board_tiles: &[u8],
        rack: &[u8],
        explicit: &std::collections::BTreeSet<Vec<u8>>,
    ) -> WoadwageBrute {
        let n = side as usize;
        let at = |row: i8, col: i8| row as usize * n + col as usize;
        let empty_board = board_tiles.iter().all(|&tile| tile == 0);
        let star = side / 2;
        let mut multi = std::collections::BTreeSet::new();
        let mut single = std::collections::BTreeSet::new();
        for down in [false, true] {
            for lane in 0..side {
                let square = |idx: i8| if down { (idx, lane) } else { (lane, idx) };
                let tile_at = |idx: i8| {
                    let (row, col) = square(idx);
                    board_tiles[at(row, col)]
                };
                for start in 0..side {
                    if start > 0 && tile_at(start - 1) != 0 {
                        continue;
                    }
                    for end in start + 1..=side {
                        if end < side && tile_at(end) != 0 {
                            continue;
                        }
                        let empties = (start..end)
                            .filter(|&idx| tile_at(idx) == 0)
                            .collect::<Vec<_>>();
                        if empties.is_empty() || empties.len() > rack.len() {
                            continue;
                        }
                        let mut fillings = std::collections::BTreeSet::new();
                        let mut used = vec![false; rack.len()];
                        let mut word = vec![0u8; (end - start) as usize];
                        fn fill(
                            k: usize,
                            empties: &[i8],
                            start: i8,
                            rack: &[u8],
                            used: &mut [bool],
                            word: &mut [u8],
                            fillings: &mut std::collections::BTreeSet<Vec<u8>>,
                        ) {
                            if k == empties.len() {
                                fillings.insert(word.to_vec());
                                return;
                            }
                            for i in 0..rack.len() {
                                if used[i] {
                                    continue;
                                }
                                used[i] = true;
                                let letters = if rack[i] == 0 {
                                    (1..=woadwage_toy_letters())
                                        .map(|letter| letter | 0x80)
                                        .collect()
                                } else {
                                    vec![rack[i]]
                                };
                                for letter in letters {
                                    word[(empties[k] - start) as usize] = letter;
                                    fill(k + 1, empties, start, rack, used, word, fillings);
                                }
                                used[i] = false;
                            }
                        }
                        fill(
                            0,
                            &empties,
                            start,
                            rack,
                            &mut used,
                            &mut word,
                            &mut fillings,
                        );
                        for word in fillings {
                            let mut board = board_tiles.to_vec();
                            for &idx in &empties {
                                let (row, col) = square(idx);
                                board[at(row, col)] = word[(idx - start) as usize];
                            }
                            let tile = |row: i8, col: i8| {
                                if (0..side).contains(&row) && (0..side).contains(&col) {
                                    board[at(row, col)]
                                } else {
                                    0
                                }
                            };
                            let run = |row: i8, col: i8, down: bool| {
                                let (dr, dc) = if down { (1, 0) } else { (0, 1) };
                                let (mut r, mut c) = (row, col);
                                while tile(r - dr, c - dc) != 0 {
                                    r -= dr;
                                    c -= dc;
                                }
                                let mut letters = Vec::new();
                                while tile(r, c) != 0 {
                                    letters.push(tile(r, c) & 0x7f);
                                    r += dr;
                                    c += dc;
                                }
                                letters
                            };
                            let connected = if empty_board {
                                empties.iter().any(|&idx| square(idx) == (star, star))
                            } else {
                                empties.len() < (end - start) as usize
                                    || empties.iter().any(|&idx| {
                                        let (row, col) = square(idx);
                                        let (dr, dc) = if down { (0, 1) } else { (1, 0) };
                                        tile(row - dr, col - dc) != 0
                                            || tile(row + dr, col + dc) != 0
                                    })
                            };
                            if !connected {
                                continue;
                            }
                            let (row0, col0) = square(start);
                            let mut formed = vec![run(row0, col0, down)];
                            for &idx in &empties {
                                let (row, col) = square(idx);
                                formed.push(run(row, col, !down));
                            }
                            formed.retain(|word| word.len() >= 2);
                            if formed.is_empty()
                                || !formed.iter().all(|word| explicit.contains(word))
                            {
                                continue;
                            }
                            if empties.len() == 1 {
                                let (row, col) = square(empties[0]);
                                single.insert((row, col, word[(empties[0] - start) as usize]));
                            } else {
                                multi.insert((down, lane, start, word));
                            }
                        }
                    }
                }
            }
        }
        WoadwageBrute(multi.into_iter().collect(), single)
    }

    #[test]
    #[inline]
    fn woadwage_plays_are_the_brute_force_ones_and_its_best_are_the_best() {
        let (kwg, explicit) = woadwage_toy();
        let klv = klv::kbwg_from_bytes_alloc(klv::EMPTY_KLV_BYTES.to_vec()).unwrap();
        let mut seed = 178u64;
        let mut cases = 0usize;
        let mut plays_seen = 0usize;
        for side in [5i8, 7] {
            let gc = woadwage_toy_config(side);
            let n = side as usize;
            let mut move_generator = KurniaMoveGenerator::new(&gc);
            let mut scorer = play_scorer::PlayScorer::new();
            for _game in 0..12 {
                let mut board_tiles = vec![0u8; n * n];
                for _ply in 0..8 {
                    let mut chosen = None;
                    for _rack in 0..3 {
                        let len =
                            1 + (woadwage_rng(&mut seed) % woadwage_toy_most_tiles()) as usize;
                        let rack = {
                            let mut rack = (0..len)
                                .map(|_| {
                                    (woadwage_rng(&mut seed) % (woadwage_toy_letters() as u64 + 1))
                                        as u8
                                })
                                .collect::<Vec<_>>();
                            rack.sort_unstable();
                            rack
                        };
                        let board_snapshot = BoardSnapshot {
                            board_tiles: &board_tiles,
                            game_config: &gc,
                            kwg: &kwg,
                            anagrams: None,
                            rack_lengths: None,
                            klv: &klv,
                        };
                        let generated =
                            place_plays(&mut move_generator, &board_snapshot, &rack, false);
                        let mut multi = Vec::new();
                        let mut single = Vec::new();
                        for (down, lane, idx, word, score) in &generated {
                            let play = Play::Place {
                                down: *down,
                                lane: *lane,
                                idx: *idx,
                                word: word[..].into(),
                                score: equity::Equity::new(*score),
                            };
                            assert_eq!(
                                scorer.compute_score(&board_snapshot, &play),
                                *score,
                                "{word:?}"
                            );
                            assert!(scorer.words_are_valid(&board_snapshot, &play), "{word:?}");
                            let placed = (0i8..)
                                .zip(word.iter())
                                .filter(|&(_, &tile)| tile != 0)
                                .collect::<Vec<_>>();
                            if placed.len() == 1 {
                                let along = *idx + placed[0].0;
                                let (row, col) = if *down {
                                    (along, *lane)
                                } else {
                                    (*lane, along)
                                };
                                single.push((row, col, *placed[0].1));
                            } else {
                                multi.push((*down, *lane, *idx, word.clone()));
                            }
                        }
                        multi.sort_unstable();
                        let single_set = single
                            .iter()
                            .copied()
                            .collect::<std::collections::BTreeSet<_>>();
                        assert_eq!(single.len(), single_set.len(), "a tile played twice");
                        let WoadwageBrute(want_multi, want_single) =
                            woadwage_brute_force(side, &board_tiles, &rack, &explicit);
                        assert_eq!(multi, want_multi, "{side} {rack:?} {board_tiles:?}");
                        assert_eq!(single_set, want_single, "{side} {rack:?} {board_tiles:?}");
                        plays_seen += multi.len() + single_set.len();
                        let params = |max_gen| GenMovesParams {
                            board_snapshot: &board_snapshot,
                            rack: &rack,
                            max_gen,
                            num_exchanges_by_this_player: 0,
                            pass_policy: PassPolicy::OnlyWhenForced,
                            dynamic_leaves: None,
                        };
                        move_generator
                            .gen_moves_unfiltered(&params(usize::MAX))
                            .unwrap();
                        let mut all = move_generator
                            .plays
                            .iter()
                            .map(|valued| valued.equity)
                            .collect::<Vec<_>>();
                        all.sort_unstable_by(|a, b| b.cmp(a));
                        for k in 1..=all.len() {
                            move_generator.gen_moves_unfiltered(&params(k)).unwrap();
                            let mut best = move_generator
                                .plays
                                .iter()
                                .map(|valued| valued.equity)
                                .collect::<Vec<_>>();
                            best.sort_unstable_by(|a, b| b.cmp(a));
                            assert_eq!(best, all[..k.min(all.len())], "{side} {rack:?} {k}");
                        }
                        cases += 1;
                        let count = want_multi.len() + want_single.len();
                        if chosen.is_none() && count > 0 {
                            let pick = woadwage_rng(&mut seed) as usize % count;
                            chosen = Some(if pick < want_multi.len() {
                                let (down, lane, idx, word) = want_multi[pick].clone();
                                (0i8..)
                                    .zip(word)
                                    .filter(|&(_, tile)| tile != 0)
                                    .map(|(i, tile)| {
                                        let along = idx + i;
                                        if down {
                                            (along, lane, tile)
                                        } else {
                                            (lane, along, tile)
                                        }
                                    })
                                    .collect::<Vec<_>>()
                            } else {
                                vec![*want_single.iter().nth(pick - want_multi.len()).unwrap()]
                            });
                        }
                    }
                    let Some(placed) = chosen else {
                        break;
                    };
                    for (row, col, tile) in placed {
                        board_tiles[row as usize * n + col as usize] = tile;
                    }
                }
            }
        }
        assert!(cases > 0, "{cases}");
        assert!(plays_seen > 0, "{plays_seen}");
        eprintln!("woadwage brute force: {cases} cases, {plays_seen} plays");
    }

    #[inline]
    pub(crate) fn jumbled_woadwage_toy()
    -> (kwg::Kwg<kwg::Node24>, std::collections::BTreeSet<Vec<u8>>) {
        let words = WOADWAGE_TOY_WORDS
            .iter()
            .map(|&word| word.to_vec())
            .collect::<Vec<_>>();
        (
            crate::jumbled_woadwage::tests::kad_for(&words),
            crate::jumbled_woadwage::tests::explicit_variant(&words),
        )
    }

    #[test]
    #[inline]
    fn a_jumbled_woadwage_cross_set_takes_each_letter_its_cross_word_accepts() {
        let (kad, explicit) = jumbled_woadwage_toy();
        let klv = klv::kbwg_from_bytes_alloc(klv::EMPTY_KLV_BYTES.to_vec()).unwrap();
        let mut seed = 218u64;
        let mut squares = 0usize;
        let mut taken = 0usize;
        for side in [5i8, 7] {
            let gc = woadwage_toy_config(side).jumbled_woadwage();
            let dim = gc.board_layout().dim();
            let n = side as usize;
            let alphabet_len = gc.alphabet().len();
            let mut move_generator = KurniaMoveGenerator::new(&gc);
            for _ in 0..40 {
                let mut board_tiles = vec![0u8; n * n];
                for tile in board_tiles.iter_mut() {
                    if woadwage_rng(&mut seed).is_multiple_of(3) {
                        let letter =
                            1 + (woadwage_rng(&mut seed) % woadwage_toy_letters() as u64) as u8;
                        *tile = letter
                            | if woadwage_rng(&mut seed).is_multiple_of(5) {
                                0x80
                            } else {
                                0
                            };
                    }
                }
                if board_tiles.iter().all(|&tile| tile == 0) {
                    board_tiles[0] = 1;
                }
                let board_snapshot = BoardSnapshot {
                    board_tiles: &board_tiles,
                    game_config: &gc,
                    kwg: &kad,
                    anagrams: None,
                    rack_lengths: None,
                    klv: &klv,
                };
                place_plays(&mut move_generator, &board_snapshot, &[1, 2], false);
                let wb = &move_generator.working_buffer;
                for row in 0..side {
                    for col in 0..side {
                        let at = dim.at_on_board(row, col);
                        if board_tiles[at] != 0 {
                            continue;
                        }
                        let run = |down: bool, step: i8| {
                            let mut tiles = Vec::new();
                            let (mut r, mut c) = (row, col);
                            loop {
                                if down {
                                    r += step;
                                } else {
                                    c += step;
                                }
                                if !(0..side).contains(&r) || !(0..side).contains(&c) {
                                    break;
                                }
                                let tile = board_tiles[dim.at_on_board(r, c)];
                                if tile == 0 {
                                    break;
                                }
                                tiles.push(tile & 0x7f);
                            }
                            if step < 0 {
                                tiles.reverse();
                            }
                            tiles
                        };
                        for (down_word, got) in [
                            (true, wb.allowed_for_across_plays[at]),
                            (
                                false,
                                wb.allowed_for_down_plays[(col as usize) * n + row as usize],
                            ),
                        ] {
                            let (before, after) = (run(down_word, -1), run(down_word, 1));
                            let want = if before.is_empty() && after.is_empty() {
                                !1
                            } else {
                                let mut bits = 1u64;
                                for letter in 1..alphabet_len {
                                    let word = [&before[..], &[letter], &after[..]].concat();
                                    if explicit.contains(&word) {
                                        bits |= 1 << letter;
                                        taken += 1;
                                    }
                                }
                                bits
                            };
                            assert_eq!(got, want, "{side} {row} {col} {down_word}");
                            squares += 1;
                        }
                    }
                }
            }
        }
        assert!(squares > 0, "{squares}");
        assert!(taken > 0, "{taken}");
    }

    #[derive(Debug, PartialEq, Eq, PartialOrd, Ord)]
    struct PlayWithEquity(bool, i8, i8, Vec<u8>, i32, i32);

    #[inline]
    fn plays_with_equity<G: LaneBits, const JUMPS: bool, const ANY_SIDE: bool>(
        move_generator: &KurniaMoveGenerator<G, JUMPS, ANY_SIDE>,
    ) -> Vec<PlayWithEquity> {
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
                } => Some(PlayWithEquity(
                    *down,
                    *lane,
                    *idx,
                    word.to_vec(),
                    score.raw(),
                    p.equity.raw(),
                )),
                Play::Exchange { .. } => None,
            })
            .collect::<Vec<_>>();
        out.sort_unstable();
        out
    }

    #[test]
    #[inline]
    fn jumbled_woadwage_plays_are_the_brute_force_ones_and_its_best_are_the_best() {
        let (kad, explicit) = jumbled_woadwage_toy();
        let klv = klv::kbwg_from_bytes_alloc(klv::EMPTY_KLV_BYTES.to_vec()).unwrap();
        let mut seed = 218u64;
        let mut cases = 0usize;
        let mut plays_seen = 0usize;
        let mut collapsed = 0usize;
        for side in [5i8, 7] {
            let gc = woadwage_toy_config(side).jumbled_woadwage();
            let n = side as usize;
            let mut move_generator = KurniaMoveGenerator::new(&gc);
            let mut scorer = play_scorer::PlayScorer::new();
            for _game in 0..12 {
                let mut board_tiles = vec![0u8; n * n];
                for _ply in 0..8 {
                    let mut chosen = None;
                    for _rack in 0..3 {
                        let len =
                            1 + (woadwage_rng(&mut seed) % woadwage_toy_most_tiles()) as usize;
                        let rack = {
                            let mut rack = (0..len)
                                .map(|_| {
                                    (woadwage_rng(&mut seed) % (woadwage_toy_letters() as u64 + 1))
                                        as u8
                                })
                                .collect::<Vec<_>>();
                            rack.sort_unstable();
                            rack
                        };
                        let board_snapshot = BoardSnapshot {
                            board_tiles: &board_tiles,
                            game_config: &gc,
                            kwg: &kad,
                            anagrams: None,
                            rack_lengths: None,
                            klv: &klv,
                        };
                        let generated =
                            place_plays(&mut move_generator, &board_snapshot, &rack, false);
                        let mut multi = Vec::new();
                        let mut single = Vec::new();
                        for (down, lane, idx, word, score) in &generated {
                            let play = Play::Place {
                                down: *down,
                                lane: *lane,
                                idx: *idx,
                                word: word[..].into(),
                                score: equity::Equity::new(*score),
                            };
                            assert_eq!(
                                scorer.compute_score(&board_snapshot, &play),
                                *score,
                                "{word:?}"
                            );
                            assert!(scorer.words_are_valid(&board_snapshot, &play), "{word:?}");
                            let placed = (0i8..)
                                .zip(word.iter())
                                .filter(|&(_, &tile)| tile != 0)
                                .collect::<Vec<_>>();
                            if placed.len() == 1 {
                                let along = *idx + placed[0].0;
                                let (row, col) = if *down {
                                    (along, *lane)
                                } else {
                                    (*lane, along)
                                };
                                single.push((row, col, *placed[0].1));
                            } else {
                                multi.push((*down, *lane, *idx, word.clone()));
                            }
                        }
                        multi.sort_unstable();
                        let single_set = single
                            .iter()
                            .copied()
                            .collect::<std::collections::BTreeSet<_>>();
                        assert_eq!(single.len(), single_set.len(), "a tile played twice");
                        let WoadwageBrute(want_multi, want_single) =
                            woadwage_brute_force(side, &board_tiles, &rack, &explicit);
                        assert_eq!(multi, want_multi, "{side} {rack:?} {board_tiles:?}");
                        assert_eq!(single_set, want_single, "{side} {rack:?} {board_tiles:?}");
                        plays_seen += multi.len() + single_set.len();
                        let params = |max_gen| GenMovesParams {
                            board_snapshot: &board_snapshot,
                            rack: &rack,
                            max_gen,
                            num_exchanges_by_this_player: 0,
                            pass_policy: PassPolicy::OnlyWhenForced,
                            dynamic_leaves: None,
                        };
                        move_generator
                            .gen_moves_unfiltered(&params(usize::MAX))
                            .unwrap();
                        let every = plays_with_equity(&move_generator);
                        let mut want_once = every
                            .iter()
                            .map(|PlayWithEquity(down, lane, idx, word, score, equity)| {
                                let mut placed = word
                                    .iter()
                                    .copied()
                                    .filter(|&tile| tile != 0)
                                    .collect::<Vec<_>>();
                                placed.sort_unstable();
                                (
                                    (*down, *lane, *idx, word.len(), placed, *score, *equity),
                                    word.clone(),
                                )
                            })
                            .collect::<Vec<_>>();
                        want_once.sort_unstable();
                        want_once.dedup_by(|b, a| a.0 == b.0);
                        let mut want_once = want_once
                            .into_iter()
                            .map(|((down, lane, idx, _, _, score, equity), word)| {
                                PlayWithEquity(down, lane, idx, word, score, equity)
                            })
                            .collect::<Vec<_>>();
                        want_once.sort_unstable();
                        move_generator
                            .gen_moves_filtered_lean_walked::<OncePerScore, _, _>(
                                &params(usize::MAX),
                                PlacePredicate::AcceptAll,
                                klv::AdjustLeave::Identity,
                                EquityPredicate::AcceptAll,
                            )
                            .unwrap();
                        let once = plays_with_equity(&move_generator);
                        assert_eq!(once, want_once, "{side} {rack:?} {board_tiles:?}");
                        collapsed += every.len() - once.len();
                        let mut all = every.iter().map(|play| play.5).collect::<Vec<_>>();
                        all.sort_unstable_by(|a, b| b.cmp(a));
                        for k in 1..=all.len() {
                            move_generator.gen_moves_unfiltered(&params(k)).unwrap();
                            let mut best = move_generator
                                .plays
                                .iter()
                                .filter(|valued| matches!(valued.play, Play::Place { .. }))
                                .map(|valued| valued.equity.raw())
                                .collect::<Vec<_>>();
                            best.sort_unstable_by(|a, b| b.cmp(a));
                            assert_eq!(best[..], all[..best.len()], "{side} {k}");
                        }
                        cases += 1;
                        let count = want_multi.len() + want_single.len();
                        if chosen.is_none() && count > 0 {
                            let pick = woadwage_rng(&mut seed) as usize % count;
                            chosen = Some(if pick < want_multi.len() {
                                let (down, lane, idx, word) = want_multi[pick].clone();
                                (0i8..)
                                    .zip(word)
                                    .filter(|&(_, tile)| tile != 0)
                                    .map(|(i, tile)| {
                                        let along = idx + i;
                                        if down {
                                            (along, lane, tile)
                                        } else {
                                            (lane, along, tile)
                                        }
                                    })
                                    .collect::<Vec<_>>()
                            } else {
                                vec![*want_single.iter().nth(pick - want_multi.len()).unwrap()]
                            });
                        }
                    }
                    let Some(placed) = chosen else {
                        break;
                    };
                    for (row, col, tile) in placed {
                        board_tiles[row as usize * n + col as usize] = tile;
                    }
                }
            }
        }
        assert!(cases > 0, "{cases}");
        assert!(plays_seen > 0, "{plays_seen}");
        assert!(collapsed > 0, "{collapsed}");
        eprintln!(
            "jumbled woadwage brute force: {cases} cases, {plays_seen} plays, {collapsed} collapsed"
        );
    }

    pub(crate) struct WoadwageToyRules(
        pub(crate) game_config::GameConfig,
        pub(crate) kwg::Kwg<kwg::Node24>,
        pub(crate) std::collections::BTreeSet<Vec<u8>>,
    );

    #[inline]
    pub(crate) fn woadwage_toy_rules(side: i8, jumbled: bool) -> WoadwageToyRules {
        if jumbled {
            let (kad, explicit) = jumbled_woadwage_toy();
            WoadwageToyRules(woadwage_toy_config(side).jumbled_woadwage(), kad, explicit)
        } else {
            let (kwg, explicit) = woadwage_toy();
            WoadwageToyRules(woadwage_toy_config(side), kwg, explicit)
        }
    }

    #[inline]
    pub(crate) fn woadwage_play_key(
        down: bool,
        lane: i8,
        idx: i8,
        word: &[u8],
    ) -> (u8, i8, i8, Vec<u8>) {
        let placed = (0i8..)
            .zip(word.iter())
            .filter(|&(_, &tile)| tile != 0)
            .collect::<Vec<_>>();
        if placed.len() == 1 {
            let along = idx + placed[0].0;
            let (row, col) = if down { (along, lane) } else { (lane, along) };
            (2, row, col, vec![*placed[0].1])
        } else {
            (down as u8, lane, idx, word.to_vec())
        }
    }

    #[inline]
    pub(crate) fn woadwage_brute_keys(
        side: i8,
        board_tiles: &[u8],
        rack: &[u8],
        explicit: &std::collections::BTreeSet<Vec<u8>>,
    ) -> std::collections::BTreeSet<(u8, i8, i8, Vec<u8>)> {
        let WoadwageBrute(multi, single) = woadwage_brute_force(side, board_tiles, rack, explicit);
        multi
            .into_iter()
            .map(|(down, lane, idx, word)| (down as u8, lane, idx, word))
            .chain(
                single
                    .into_iter()
                    .map(|(row, col, tile)| (2, row, col, vec![tile])),
            )
            .collect()
    }

    #[inline]
    pub(crate) fn woadwage_key_play(
        side: i8,
        board_tiles: &[u8],
        (kind, a, b, word): &(u8, i8, i8, Vec<u8>),
    ) -> Play {
        let (down, lane, idx, word) = if *kind == 2 {
            let (row, col) = (*a, *b);
            let at = |row: i8, col: i8| {
                (0..side).contains(&row)
                    && (0..side).contains(&col)
                    && board_tiles[row as usize * side as usize + col as usize] != 0
            };
            let down = !at(row, col - 1) && !at(row, col + 1);
            let (lane, along) = if down { (col, row) } else { (row, col) };
            let tile_at = |i: i8| if down { at(i, lane) } else { at(lane, i) };
            let (mut start, mut end) = (along, along + 1);
            while tile_at(start - 1) {
                start -= 1;
            }
            while tile_at(end) {
                end += 1;
            }
            let mut spanned = vec![0u8; (end - start) as usize];
            spanned[(along - start) as usize] = word[0];
            (down, lane, start, spanned)
        } else {
            (*kind == 1, *a, *b, word.clone())
        };
        Play::Place {
            down,
            lane,
            idx,
            word: word[..].into(),
            score: equity::Equity::new(0),
        }
    }

    #[inline]
    pub(crate) fn woadwage_toy_rack(seed: &mut u64, most: u64) -> Vec<u8> {
        let len = 1 + (woadwage_rng(seed) % most) as usize;
        let mut rack = (0..len)
            .map(|_| (woadwage_rng(seed) % (woadwage_toy_letters() as u64 + 1)) as u8)
            .collect::<Vec<_>>();
        rack.sort_unstable();
        rack
    }

    #[inline]
    pub(crate) fn woadwage_toy_boards(
        side: i8,
        explicit: &std::collections::BTreeSet<Vec<u8>>,
        seed: &mut u64,
        games: usize,
        plies: usize,
    ) -> Vec<Vec<u8>> {
        let n = side as usize;
        let mut boards = Vec::new();
        for _game in 0..games {
            let mut board_tiles = vec![0u8; n * n];
            boards.push(board_tiles.clone());
            for _ply in 0..plies {
                let rack = woadwage_toy_rack(seed, woadwage_toy_most_tiles());
                let keys = woadwage_brute_keys(side, &board_tiles, &rack, explicit);
                if keys.is_empty() {
                    continue;
                }
                let pick = woadwage_rng(seed) as usize % keys.len();
                let (kind, a, b, word) = keys.into_iter().nth(pick).unwrap();
                if kind == 2 {
                    board_tiles[a as usize * n + b as usize] = word[0];
                } else {
                    for (i, &tile) in (b..).zip(word.iter()) {
                        if tile != 0 {
                            let (row, col) = if kind == 1 { (i, a) } else { (a, i) };
                            board_tiles[row as usize * n + col as usize] = tile;
                        }
                    }
                }
                boards.push(board_tiles.clone());
            }
        }
        boards
    }

    #[inline]
    pub(crate) fn woadwage_within_bag(gc: &game_config::GameConfig, board_tiles: &[u8]) -> bool {
        let alphabet = gc.alphabet();
        let mut left = (0..alphabet.len())
            .map(|tile| alphabet.freq(tile) as i32)
            .collect::<Vec<_>>();
        for &tile in board_tiles.iter().filter(|&&tile| tile != 0) {
            left[if tile & 0x80 == 0 { tile as usize } else { 0 }] -= 1;
        }
        left.iter().all(|&count| count >= 0)
    }

    #[test]
    #[inline]
    fn the_word_prune_keeps_every_woadwage_play() {
        let klv = klv::kbwg_from_bytes_alloc(klv::EMPTY_KLV_BYTES.to_vec()).unwrap();
        let mut seed = 577u64;
        let mut cases = 0usize;
        for jumbled in [false, true] {
            for side in [5i8, 7] {
                let WoadwageToyRules(gc, kwg, explicit) = woadwage_toy_rules(side, jumbled);
                for board_tiles in woadwage_toy_boards(side, &explicit, &mut seed, 6, 8)
                    .into_iter()
                    .filter(|board_tiles| woadwage_within_bag(&gc, board_tiles))
                {
                    let mut kept = Vec::<Vec<u8>>::new();
                    KurniaMoveGenerator::new(&gc)
                        .gen_remaining_words(
                            &BoardSnapshot {
                                board_tiles: &board_tiles,
                                game_config: &gc,
                                kwg: &kwg,
                                anagrams: None,
                                rack_lengths: None,
                                klv: &klv,
                            },
                            |word: &[u8]| kept.push(word.to_vec()),
                        )
                        .unwrap();
                    let pruned = if jumbled {
                        crate::jumbled_woadwage::tests::explicit_variant(&kept)
                    } else {
                        (kept.iter().cloned())
                            .chain(
                                (kept.iter())
                                    .flat_map(|a| kept.iter().map(move |b| [&a[..], b].concat())),
                            )
                            .collect()
                    };
                    for _rack in 0..5 {
                        let rack = woadwage_toy_rack(&mut seed, woadwage_toy_most_tiles());
                        let with_rack = (board_tiles.iter().copied())
                            .chain(rack.iter().map(|&tile| if tile == 0 { 0x80 } else { tile }))
                            .collect::<Vec<_>>();
                        if !woadwage_within_bag(&gc, &with_rack) {
                            continue;
                        }
                        assert_eq!(
                            woadwage_brute_keys(side, &board_tiles, &rack, &pruned),
                            woadwage_brute_keys(side, &board_tiles, &rack, &explicit),
                            "[jumbled {jumbled} side {side} {board_tiles:?} {rack:?} {kept:?}]"
                        );
                        cases += 1;
                    }
                }
            }
        }
        assert!(cases > 0, "{cases}");
    }

    #[test]
    #[inline]
    fn the_word_prune_lets_a_woadwage_split_move_go_or_come() {
        let klv = klv::kbwg_from_bytes_alloc(klv::EMPTY_KLV_BYTES.to_vec()).unwrap();
        // the letters: a is 1, b 2, c 3, e 5, l 12, o 15, s 19, t 20
        let shared: Vec<Vec<u8>> = vec![
            vec![1, 2],
            vec![1, 2, 19],
            vec![3, 1, 20],
            vec![15, 3, 1],
            vec![19, 3, 1, 20],
            vec![20, 1, 2, 19],
        ];
        let side = 9i8;
        let catab = [3u8, 1, 20, 1, 2];
        type Case<'a> = (&'a [u8], &'a [u8], usize);
        let shared_cases: [Case<'_>; 3] = [
            (&catab, &[15, 19], 7),
            (&[3, 1, 20], &[1, 2], 5),
            (&catab, &[19, 19], 7),
        ];
        for jumbled in [false, true] {
            let (more, more_case): (&[&[u8]], Case<'_>) = if jumbled {
                (&[&[1, 20], &[1, 2, 1, 20, 5]], (&[1, 20, 1, 2], &[5], 5))
            } else {
                (
                    &[&[12, 15, 3, 1, 20, 1, 2, 12, 5]],
                    (&catab, &[5, 12, 12, 15], 9),
                )
            };
            let words = (shared.iter().cloned())
                .chain(more.iter().map(|word| word.to_vec()))
                .collect::<Vec<_>>();
            let cases = shared_cases.iter().copied().chain([more_case]);
            let (gc, kwg, explicit) = if jumbled {
                (
                    woadwage_toy_config(side).jumbled_woadwage(),
                    crate::jumbled_woadwage::tests::kad_for(&words),
                    crate::jumbled_woadwage::tests::explicit_variant(&words),
                )
            } else {
                let sorted = {
                    let mut sorted = words.clone();
                    sorted.sort_unstable();
                    sorted
                        .iter()
                        .map(|word| bites::Bites::from(&word[..]))
                        .collect::<Vec<_>>()
                };
                let kwg = kwg::kbwg_from_bytes_alloc(
                    build::build(
                        build::BuildContent::Gaddawg,
                        build::BuildLayout::Wolges,
                        build::BuildOrder::Sorted,
                        &sorted,
                    )
                    .unwrap()
                    .to_vec(),
                    false,
                )
                .unwrap();
                let explicit = (words.iter().cloned())
                    .chain(
                        (words.iter())
                            .flat_map(|a| words.iter().map(move |b| [&a[..], b].concat())),
                    )
                    .collect();
                (woadwage_toy_config(side), kwg, explicit)
            };
            for (run, rack, long) in cases {
                let mut board_tiles = vec![0u8; side as usize * side as usize];
                let row = (side / 2) as usize * side as usize;
                board_tiles[row + 2..row + 2 + run.len()].copy_from_slice(run);
                let n = side as usize;
                let mut blockers = [
                    0x84u8, 0x84, 4, 4, 4, 4, 7, 7, 7, 6, 6, 8, 8, 13, 13, 16, 16,
                ]
                .into_iter();
                for col in 2..2 + run.len() {
                    board_tiles[row - n + col] = blockers.next().unwrap();
                    board_tiles[row + n + col] = blockers.next().unwrap();
                }
                for (corner, beside) in [(0, [1, n]), (n - 1, [n - 2, 2 * n - 1])] {
                    board_tiles[corner] = if corner == 0 { 3 } else { 2 };
                    for at in beside {
                        board_tiles[at] = blockers.next().unwrap();
                    }
                }
                let mut kept = Vec::<Vec<u8>>::new();
                KurniaMoveGenerator::new(&gc)
                    .gen_remaining_words(
                        &BoardSnapshot {
                            board_tiles: &board_tiles,
                            game_config: &gc,
                            kwg: &kwg,
                            anagrams: None,
                            rack_lengths: None,
                            klv: &klv,
                        },
                        |word: &[u8]| kept.push(word.to_vec()),
                    )
                    .unwrap();
                let pruned = if jumbled {
                    crate::jumbled_woadwage::tests::explicit_variant(&kept)
                } else {
                    (kept.iter().cloned())
                        .chain(
                            (kept.iter())
                                .flat_map(|a| kept.iter().map(move |b| [&a[..], b].concat())),
                        )
                        .collect()
                };
                let whole = woadwage_brute_keys(side, &board_tiles, rack, &explicit);
                assert!(
                    (whole.iter()).any(|key| matches!(woadwage_key_play(side, &board_tiles, key), Play::Place { word, .. } if word.len() == long)),
                    "[jumbled {jumbled} {run:?} {rack:?}]"
                );
                assert_eq!(
                    woadwage_brute_keys(side, &board_tiles, rack, &pruned),
                    whole,
                    "[jumbled {jumbled} {run:?} {rack:?} {kept:?}]"
                );
            }
        }
    }

    #[test]
    #[inline]
    fn the_woadwage_word_prune_drops_a_word_no_span_can_take() {
        let klv = klv::kbwg_from_bytes_alloc(klv::EMPTY_KLV_BYTES.to_vec()).unwrap();
        let words: Vec<Vec<u8>> = vec![vec![1, 2], vec![1, 4], vec![26, 26]];
        let side = 5i8;
        for jumbled in [false, true] {
            let (gc, kwg) = if jumbled {
                (
                    woadwage_toy_config(side).jumbled_woadwage(),
                    crate::jumbled_woadwage::tests::kad_for(&words),
                )
            } else {
                let kwg = kwg::kbwg_from_bytes_alloc(
                    build::build(
                        build::BuildContent::Gaddawg,
                        build::BuildLayout::Wolges,
                        build::BuildOrder::Sorted,
                        &(words.iter())
                            .map(|word| bites::Bites::from(&word[..]))
                            .collect::<Vec<_>>(),
                    )
                    .unwrap()
                    .to_vec(),
                    false,
                )
                .unwrap();
                (woadwage_toy_config(side), kwg)
            };
            let mut board_tiles = vec![0u8; side as usize * side as usize];
            board_tiles[12] = 1;
            board_tiles[13] = 0x84;
            board_tiles[0] = 0x84;
            let mut kept = Vec::<Vec<u8>>::new();
            KurniaMoveGenerator::new(&gc)
                .gen_remaining_words(
                    &BoardSnapshot {
                        board_tiles: &board_tiles,
                        game_config: &gc,
                        kwg: &kwg,
                        anagrams: None,
                        rack_lengths: None,
                        klv: &klv,
                    },
                    |word: &[u8]| kept.push(word.to_vec()),
                )
                .unwrap();
            assert_eq!(kept, vec![vec![1, 2], vec![1, 4]], "[jumbled {jumbled}]");
        }
    }

    #[test]
    #[inline]
    fn woadwage_raw_plays_are_the_brute_force_ones() {
        let klv = klv::kbwg_from_bytes_alloc(klv::EMPTY_KLV_BYTES.to_vec()).unwrap();
        let mut seed = 401u64;
        let mut cases = 0usize;
        let mut plays_seen = 0usize;
        for jumbled in [false, true] {
            for side in [5i8, 7] {
                let WoadwageToyRules(gc, kwg, explicit) = woadwage_toy_rules(side, jumbled);
                let run_letter_bits = kwg.compute_letter_bits().letter_bits;
                let mut move_generator = KurniaMoveGenerator::new(&gc);
                let mut scorer = play_scorer::PlayScorer::new();
                for board_tiles in woadwage_toy_boards(side, &explicit, &mut seed, 4, 6) {
                    let board_snapshot = BoardSnapshot {
                        board_tiles: &board_tiles,
                        game_config: &gc,
                        kwg: &kwg,
                        anagrams: None,
                        rack_lengths: None,
                        klv: &klv,
                    };
                    for _rack in 0..3 {
                        let rack = woadwage_toy_rack(&mut seed, woadwage_toy_most_tiles());
                        let want = woadwage_brute_keys(side, &board_tiles, &rack, &explicit);
                        let rows = woadwage_rng(&mut seed) as u128;
                        let cols = woadwage_rng(&mut seed) as u128;
                        for way in 0..6 {
                            match way {
                                0 => move_generator.gen_moves_raw_all_unsorted(
                                    &board_snapshot,
                                    &rack,
                                    0,
                                    PassPolicy::AsACandidate,
                                ),
                                1 => move_generator.gen_moves_raw_all_unsorted_lean(
                                    &board_snapshot,
                                    &rack,
                                    0,
                                    PassPolicy::AsACandidate,
                                ),
                                2 => move_generator.gen_moves_raw_lanes_unsorted(
                                    &board_snapshot,
                                    &rack,
                                    0,
                                    PassPolicy::AsACandidate,
                                    !0,
                                    !0,
                                ),
                                3 => move_generator.gen_moves_raw_lanes_unsorted_lean(
                                    &board_snapshot,
                                    &rack,
                                    0,
                                    PassPolicy::AsACandidate,
                                    !0,
                                    !0,
                                ),
                                4 => move_generator.gen_moves_raw_lanes_unsorted_by_runs(
                                    &run_letter_bits,
                                    &board_snapshot,
                                    &rack,
                                    !0,
                                    !0,
                                ),
                                _ => move_generator.gen_moves_raw_lanes_unsorted(
                                    &board_snapshot,
                                    &rack,
                                    0,
                                    PassPolicy::AsACandidate,
                                    rows,
                                    cols,
                                ),
                            }
                            .unwrap();
                            let mut got = Vec::new();
                            let mut passes = 0usize;
                            for valued in &move_generator.plays {
                                match &valued.play {
                                    Play::Place {
                                        down,
                                        lane,
                                        idx,
                                        word,
                                        score,
                                    } => {
                                        assert_eq!(
                                            scorer.compute_score(&board_snapshot, &valued.play),
                                            score.raw(),
                                            "{word:?}"
                                        );
                                        if way == 5 {
                                            let mask = if *down { cols } else { rows };
                                            assert!(mask >> *lane & 1 != 0, "{word:?}");
                                        }
                                        got.push(woadwage_play_key(*down, *lane, *idx, word));
                                    }
                                    Play::Exchange { tiles } => {
                                        if tiles.is_empty() {
                                            passes += 1;
                                        }
                                    }
                                }
                            }
                            assert_eq!(passes, 1, "{jumbled} {side} {way}");
                            let got_set = got
                                .iter()
                                .cloned()
                                .collect::<std::collections::BTreeSet<_>>();
                            assert_eq!(got.len(), got_set.len(), "a play found twice");
                            if way == 5 {
                                let lane_want = want
                                    .iter()
                                    .filter(|(kind, a, b, _)| match kind {
                                        0 => rows >> a & 1 != 0,
                                        1 => cols >> a & 1 != 0,
                                        _ => rows >> a & 1 != 0 || cols >> b & 1 != 0,
                                    })
                                    .cloned()
                                    .collect::<std::collections::BTreeSet<_>>();
                                assert!(got_set.is_subset(&lane_want), "{jumbled} {side}");
                                assert!(
                                    lane_want
                                        .iter()
                                        .filter(|(kind, _, _, _)| *kind < 2)
                                        .all(|key| got_set.contains(key)),
                                    "{jumbled} {side} {rack:?} {board_tiles:?}"
                                );
                            } else {
                                assert_eq!(
                                    got_set, want,
                                    "{jumbled} {side} {way} {rack:?} {board_tiles:?}"
                                );
                                plays_seen += got_set.len();
                            }
                        }
                        cases += 1;
                    }
                }
            }
        }
        assert!(cases > 0, "{cases}");
        assert!(plays_seen > 0, "{plays_seen}");
    }

    #[test]
    #[inline]
    fn a_one_letter_word_is_a_woadwage_half() {
        let klv = klv::kbwg_from_bytes_alloc(klv::EMPTY_KLV_BYTES.to_vec()).unwrap();
        let side = 5i8;
        type Lexicon<'a> = (Vec<Vec<u8>>, &'a [u8], usize);
        let lexicons: [Lexicon<'_>; 2] = [
            (vec![vec![1], vec![2]], &[1, 2], 2),
            (vec![vec![1], vec![2, 3]], &[1, 2, 3], 3),
        ];
        for jumbled in [false, true] {
            for (words, rack, joined) in &lexicons {
                let (gc, kwg, explicit) = if jumbled {
                    (
                        woadwage_toy_config(side).jumbled_woadwage(),
                        crate::jumbled_woadwage::tests::kad_for(words),
                        crate::jumbled_woadwage::tests::explicit_variant(words),
                    )
                } else {
                    let kwg = kwg::kbwg_from_bytes_alloc(
                        build::build(
                            build::BuildContent::Gaddawg,
                            build::BuildLayout::Wolges,
                            build::BuildOrder::Sorted,
                            &(words.iter())
                                .map(|word| bites::Bites::from(&word[..]))
                                .collect::<Vec<_>>(),
                        )
                        .unwrap()
                        .to_vec(),
                        false,
                    )
                    .unwrap();
                    let explicit = (words.iter().cloned())
                        .chain(
                            (words.iter())
                                .flat_map(|a| words.iter().map(move |b| [&a[..], b].concat())),
                        )
                        .collect();
                    (woadwage_toy_config(side), kwg, explicit)
                };
                let board_tiles = vec![0u8; side as usize * side as usize];
                let board_snapshot = BoardSnapshot {
                    board_tiles: &board_tiles,
                    game_config: &gc,
                    kwg: &kwg,
                    anagrams: None,
                    rack_lengths: None,
                    klv: &klv,
                };
                let mut move_generator = KurniaMoveGenerator::new(&gc);
                move_generator
                    .gen_moves_raw_all_unsorted(&board_snapshot, rack, 0, PassPolicy::AsACandidate)
                    .unwrap();
                let got = (move_generator.plays.iter())
                    .filter_map(|valued| match &valued.play {
                        Play::Place {
                            down,
                            lane,
                            idx,
                            word,
                            ..
                        } => Some(woadwage_play_key(*down, *lane, *idx, word)),
                        Play::Exchange { .. } => None,
                    })
                    .collect::<std::collections::BTreeSet<_>>();
                assert_eq!(
                    got,
                    woadwage_brute_keys(side, &board_tiles, rack, &explicit),
                    "[jumbled {jumbled} {words:?}]"
                );
                assert!(
                    got.iter().any(|(_, _, _, word)| word.len() == *joined),
                    "[jumbled {jumbled} {words:?}] {got:?}"
                );
            }
        }
    }

    #[inline]
    fn place_plays<G: LaneBits, const JUMPS: bool, const ANY_SIDE: bool>(
        move_generator: &mut KurniaMoveGenerator<G, JUMPS, ANY_SIDE>,
        board_snapshot: &BoardSnapshot<'_, kwg::Node24, kwg::Node24>,
        rack: &[u8],
        transposed: bool,
    ) -> Vec<(bool, i8, i8, Vec<u8>, i32)> {
        move_generator
            .gen_moves_unfiltered(&GenMovesParams {
                board_snapshot,
                rack,
                max_gen: usize::MAX,
                num_exchanges_by_this_player: 0,
                pass_policy: PassPolicy::OnlyWhenForced,
                dynamic_leaves: None,
            })
            .unwrap();
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
                } => Some((*down != transposed, *lane, *idx, word.to_vec(), score.raw())),
                Play::Exchange { .. } => None,
            })
            .collect::<Vec<_>>();
        out.sort_unstable();
        out
    }

    #[test]
    #[inline]
    fn a_generator_reset_for_another_kwg_plays_as_a_new_one() {
        // a strip's walk keeps the kwg's nodes for the tiles on the board; a
        // generator reset for another kwg finds on the same board what a new
        // generator finds, under anagrams keyed by their layout, anagrams keyed
        // by nibbles, and the rack lengths
        let gc = game_config::make_english_game_config();
        let kwg = test_kwg(&gc);
        // the other kwg lacks every word with an S, so its nodes lie elsewhere
        // and the same board crosses other sets
        let other_words = TEST_WORDS
            .iter()
            .copied()
            .filter(|word| !word.contains('S'))
            .collect::<Vec<_>>();
        let other_kwg = test_kwg_of(&gc, &other_words);
        let klv = klv::kbwg_from_bytes_alloc(klv::EMPTY_KLV_BYTES.to_vec()).unwrap();
        let rack = parse_test_rack(gc.alphabet(), "?AEST");
        let mut board_tiles = vec![0u8; 15 * 15];
        for i in 0..8 {
            let (row, col) = (i, ((i32::from(i) * 7 + 3) % 15) as i8);
            board_tiles[gc.board_layout().dim().at_on_board(row, col)] =
                rack[1 + i as usize % (rack.len() - 1)];
        }
        type Build = fn(&kwg::Kwg<kwg::Node24>, &game_config::GameConfig) -> anagrams::ArmTables;
        let builds: [(&str, Build); 3] = [
            ("layout", |kwg, gc| {
                anagrams::ArmTables::build_by_layout(kwg, gc).unwrap()
            }),
            ("nibbles", |kwg, gc| {
                anagrams::ArmTables::build(kwg, gc, false).unwrap()
            }),
            ("lean", |kwg, gc| {
                anagrams::ArmTables::build(kwg, gc, true).unwrap()
            }),
        ];
        for (arm, build) in builds {
            let tables = build(&kwg, &gc);
            let other_tables = build(&other_kwg, &gc);
            assert_eq!(
                tables.anagrams().and_then(anagrams::Anagrams::nibble_side),
                (arm == "nibbles").then_some(15),
            );
            let [by_first, by_other] =
                [(&kwg, &tables), (&other_kwg, &other_tables)].map(|(kwg, tables)| BoardSnapshot {
                    board_tiles: &board_tiles,
                    game_config: &gc,
                    kwg,
                    anagrams: tables.anagrams(),
                    rack_lengths: tables.rack_lengths(),
                    klv: &klv,
                });
            let want = place_plays(&mut KurniaMoveGenerator::new(&gc), &by_other, &rack, false);
            assert!(!want.is_empty());
            let mut reused = KurniaMoveGenerator::new(&gc);
            assert_ne!(place_plays(&mut reused, &by_first, &rack, false), want);
            reused.reset_for_another_kwg();
            let found = place_plays(&mut reused, &by_other, &rack, false);
            assert_eq!(found, want, "{arm}");
        }
    }

    #[test]
    #[inline]
    fn nibble_keys_play_as_layout_keys() {
        // a classic game on a square board of 15 whose tiles fit nibbles is
        // keyed by nibbles, a board of 16 by the tables' layout; on the board
        // of 15, as tiles go down, nibble keys find the moves layout keys find,
        // for racks of no blank, one, and two, each generator reused from step
        // to step
        let gc = game_config::make_english_game_config();
        let kwg = test_kwg(&gc);
        let nibbles = anagrams::ArmTables::build(&kwg, &gc, false).unwrap();
        assert_eq!(
            nibbles.anagrams().and_then(anagrams::Anagrams::nibble_side),
            Some(15)
        );
        let layout = anagrams::ArmTables::build_by_layout(&kwg, &gc).unwrap();
        assert!(layout.anagrams().is_some_and(|a| a.nibble_side().is_none()));
        let wide = uneven_game_config(16, 16, 7, 7, false);
        let wide_kwg = test_kwg(&wide);
        assert!(
            anagrams::ArmTables::build(&wide_kwg, &wide, false)
                .unwrap()
                .anagrams()
                .is_some_and(|a| a.nibble_side().is_none())
        );
        let klv = klv::kbwg_from_bytes_alloc(klv::EMPTY_KLV_BYTES.to_vec()).unwrap();
        let racks = ["AEST", "?AEST", "??AE"].map(|r| parse_test_rack(gc.alphabet(), r));
        let tiles = (0..15)
            .map(|i| (i, ((i32::from(i) * 7 + 3) % 15) as i8))
            .collect::<Vec<_>>();
        let mut board_tiles = vec![0u8; 15 * 15];
        let mut by_nibbles = KurniaMoveGenerator::new(&gc);
        let mut by_layout = KurniaMoveGenerator::new(&gc);
        let mut compared = 0;
        for step in 0..=tiles.len() {
            if step > 0 {
                let (row, col) = tiles[step - 1];
                board_tiles[gc.board_layout().dim().at_on_board(row, col)] =
                    racks[0][(step - 1) % racks[0].len()];
            }
            let [by_nibbles_tables, by_layout_tables] =
                [&nibbles, &layout].map(|tables| BoardSnapshot {
                    board_tiles: &board_tiles,
                    game_config: &gc,
                    kwg: &kwg,
                    anagrams: tables.anagrams(),
                    rack_lengths: tables.rack_lengths(),
                    klv: &klv,
                });
            for rack in &racks {
                let found = place_plays(&mut by_nibbles, &by_nibbles_tables, rack, false);
                assert!(!found.is_empty(), "nothing to play at step {step}");
                assert_eq!(
                    found,
                    place_plays(&mut by_layout, &by_layout_tables, rack, false),
                    "step {step}, rack {rack:?}"
                );
                compared += 1;
            }
        }
        assert_eq!(compared, 3 * (tiles.len() + 1));
    }

    #[test]
    #[inline]
    fn a_pools_plays_are_what_a_generation_from_it_places() {
        // on an empty board and then as tiles go down: the plays pool_plays
        // finds from a pool are the placements of at most a rack's tiles that a
        // generation with the pool as its rack finds. Each is found once. Pools
        // with no blank and with blanks are both tried.
        let gc = game_config::make_english_game_config();
        let kwg = test_kwg(&gc);
        let klv = klv::kbwg_from_bytes_alloc(klv::EMPTY_KLV_BYTES.to_vec()).unwrap();
        let rack_size = gc.rack_size() as usize;
        let placed = parse_test_rack(gc.alphabet(), "AEST");
        let pools = ["AAEEIILNORSST", "?AEEST", "??AEST", "?ACEST"]
            .map(|pool| parse_test_rack(gc.alphabet(), pool));
        let mut board_tiles = vec![0u8; 15 * 15];
        let mut generator = KurniaMoveGenerator::new(&gc);
        let mut compared = 0;
        for step in 0..=15 {
            if step > 0 {
                let at = gc
                    .board_layout()
                    .dim()
                    .at_on_board(step - 1, ((i32::from(step - 1) * 7 + 3) % 15) as i8);
                board_tiles[at] = placed[(step - 1) as usize % placed.len()];
            }
            let board_snapshot = BoardSnapshot {
                board_tiles: &board_tiles,
                game_config: &gc,
                kwg: &kwg,
                anagrams: None,
                rack_lengths: None,
                klv: &klv,
            };
            for pool in &pools {
                let mut found = Vec::new();
                generator.pool_plays(&board_snapshot, pool, |down, lane, idx, word, score| {
                    found.push((down, lane, idx, word.to_vec(), score));
                });
                found.sort_unstable();
                let mut want = place_plays(&mut generator, &board_snapshot, pool, false);
                want.retain(|(_, _, _, word, _)| {
                    word.iter().filter(|&&tile| tile != 0).count() <= rack_size
                });
                assert!(!want.is_empty(), "step {step}, pool {pool:?}");
                assert_eq!(found, want, "step {step}, pool {pool:?}");
                compared += 1;
            }
        }
        assert_eq!(compared, 16 * pools.len());
    }

    #[test]
    #[inline]
    fn a_jumbled_pools_plays_answer_as_every_arrangement() {
        type Answer = (
            std::collections::BTreeMap<Vec<u8>, i32>,
            std::collections::BTreeMap<usize, i32>,
            std::collections::BTreeMap<(Vec<u8>, usize), i32>,
        );
        type Key = (bool, i8, i8, usize, Vec<u8>, Vec<u8>, i32);
        let gc = game_config::make_jumbled_english_game_config();
        let kwg = test_kad(&gc, SWEEP_WORDS);
        let klv = klv::kbwg_from_bytes_alloc(klv::EMPTY_KLV_BYTES.to_vec()).unwrap();
        let rack_size = gc.rack_size() as usize;
        let cols = gc.board_layout().dim().cols as usize;
        let mut generator = KurniaMoveGenerator::new(&gc);
        // a pool of a whole rack plays all of it
        for (pool, on_board) in [("AERT?", 1), ("ACENRST", 0), ("??ACERT", 1)] {
            let pool = &parse_test_rack(gc.alphabet(), pool);
            let mut collapsed = 0;
            let mut board_tiles = vec![0u8; 15 * 15];
            board_tiles[gc.board_layout().dim().at_on_board(4, 3)] = on_board;
            let board_snapshot = BoardSnapshot {
                board_tiles: &board_tiles,
                game_config: &gc,
                kwg: &kwg,
                anagrams: None,
                rack_lengths: None,
                klv: &klv,
            };
            let mut answer: Answer = Default::default();
            let add = |answer: &mut Answer, down: bool, lane: i8, idx: i8, word: &[u8], score| {
                let mut key = word
                    .iter()
                    .filter(|&&t| t != 0)
                    .map(|&t| if t & 0x80 != 0 { 0 } else { t })
                    .collect::<Vec<_>>();
                key.sort_unstable();
                let best = answer.0.entry(key.clone()).or_insert(score);
                *best = (*best).max(score);
                for (i, &t) in word.iter().enumerate() {
                    if t != 0 {
                        let at = if down {
                            (idx as usize + i) * cols + lane as usize
                        } else {
                            lane as usize * cols + idx as usize + i
                        };
                        let best = answer.1.entry(at).or_insert(score);
                        *best = (*best).max(score);
                        let best = answer.2.entry((key.clone(), at)).or_insert(score);
                        *best = (*best).max(score);
                    }
                }
            };
            let mut found = Vec::<(Key, Vec<u8>)>::new();
            generator.pool_plays(&board_snapshot, pool, |down, lane, idx, word, score| {
                add(&mut answer, down, lane, idx, word, score);
                let tiles = word.iter().filter(|&&t| t != 0);
                let mut letters = tiles.clone().map(|&t| t & 0x7f).collect::<Vec<_>>();
                letters.sort_unstable();
                let mut blanked = tiles
                    .clone()
                    .filter(|&&t| t & 0x80 != 0)
                    .map(|&t| t & 0x7f)
                    .collect::<Vec<_>>();
                blanked.sort_unstable();
                let key = (down, lane, idx, word.len(), letters, blanked, score);
                found.push((key, tiles.copied().collect()));
            });
            generator
                .gen_moves_unfiltered(&GenMovesParams {
                    board_snapshot: &board_snapshot,
                    rack: pool,
                    max_gen: usize::MAX,
                    num_exchanges_by_this_player: 0,
                    pass_policy: PassPolicy::OnlyWhenForced,
                    dynamic_leaves: None,
                })
                .unwrap();
            let mut want: Answer = Default::default();
            let mut smallest = std::collections::BTreeMap::<Key, Vec<u8>>::new();
            for p in generator.plays.iter() {
                if let Play::Place {
                    down,
                    lane,
                    idx,
                    word,
                    score,
                } = &p.play
                    && word.iter().filter(|&&t| t != 0).count() <= rack_size
                {
                    add(&mut want, *down, *lane, *idx, word, score.raw());
                    let tiles = word.iter().filter(|&&t| t != 0);
                    let mut letters = tiles.clone().map(|&t| t & 0x7f).collect::<Vec<_>>();
                    letters.sort_unstable();
                    let mut blanked = tiles
                        .clone()
                        .filter(|&&t| t & 0x80 != 0)
                        .map(|&t| t & 0x7f)
                        .collect::<Vec<_>>();
                    blanked.sort_unstable();
                    let key = (
                        *down,
                        *lane,
                        *idx,
                        word.len(),
                        letters,
                        blanked,
                        score.raw(),
                    );
                    let tiles = tiles.copied().collect::<Vec<_>>();
                    let kept = smallest.entry(key).or_insert_with(|| tiles.clone());
                    if tiles < *kept {
                        *kept = tiles;
                    }
                    collapsed += 1;
                }
            }
            assert!(!want.0.is_empty());
            assert_eq!(answer, want);
            if on_board != 0 {
                let made = found
                    .iter()
                    .cloned()
                    .collect::<std::collections::BTreeMap<_, _>>();
                assert_eq!(made.len(), found.len());
                assert_eq!(made, smallest);
                collapsed -= found.len();
                assert!(
                    collapsed > 0,
                    "no arrangement was collapsed, so the test proves nothing"
                );
            }
        }
    }

    // a lane walked in parts of 1 or 2 blank choices, each part from where the
    // one before stopped, offers the plays the whole lane does, in its order
    #[test]
    fn a_pools_best_plays_walked_in_parts_are_the_whole_walks() {
        type Found = (bool, i8, i8, Vec<u8>, i32);
        let gc = game_config::make_jumbled_english_game_config();
        let kwg = test_kad(&gc, SWEEP_WORDS);
        let klv = klv::kbwg_from_bytes_alloc(klv::EMPTY_KLV_BYTES.to_vec()).unwrap();
        let dim = gc.board_layout().dim();
        let cols = dim.cols as usize;
        let placed = parse_test_rack(gc.alphabet(), "AEST");
        let pools = ["AERT?", "ACERST", "AEERT??"].map(|pool| parse_test_rack(gc.alphabet(), pool));
        let mut board_tiles = vec![0u8; dim.rows as usize * cols];
        let mut generator = KurniaMoveGenerator::new(&gc);
        let mut split = 0;
        for step in 0..=4 {
            if step > 0 {
                let at = dim.at_on_board(7, (step + 5) as i8);
                board_tiles[at] = placed[(step - 1) as usize % placed.len()];
            }
            let board_snapshot = BoardSnapshot {
                board_tiles: &board_tiles,
                game_config: &gc,
                kwg: &kwg,
                anagrams: None,
                rack_lengths: None,
                klv: &klv,
            };
            for pool in &pools {
                for lane in 0..(dim.rows + dim.cols) as u32 {
                    let masks = if lane < dim.rows as u32 {
                        (1 << lane, 0)
                    } else {
                        (0, 1 << (lane - dim.rows as u32))
                    };
                    let mut whole = Vec::<Found>::new();
                    generator.pool_plays_best_window(
                        &board_snapshot,
                        pool,
                        masks,
                        0..u64::MAX,
                        &mut PoolLists::default(),
                        |down, lane, idx, word, score| {
                            whole.push((down, lane, idx, word.to_vec(), score));
                        },
                    );
                    for budget in [1, 2] {
                        let mut parts = Vec::<Found>::new();
                        let mut lists = PoolLists::default();
                        let mut part = PoolPart::default();
                        let mut from = 0;
                        loop {
                            generator.pool_plays_best_part(
                                &board_snapshot,
                                pool,
                                PoolWindow {
                                    lanes: masks,
                                    window: from..u64::MAX,
                                },
                                budget,
                                (&mut lists, &mut part),
                                |down, lane, idx, word, score| {
                                    parts.push((down, lane, idx, word.to_vec(), score));
                                },
                            );
                            let Some(at) = part.stopped_at() else {
                                break;
                            };
                            split += usize::from(at == from);
                            from = at;
                        }
                        assert_eq!(parts, whole, "{step} {pool:?} {lane} {budget}");
                    }
                }
            }
        }
        assert!(split > 0, "no multiset was walked in parts");
    }

    #[test]
    fn a_pools_best_plays_by_window_answer_as_all_its_plays() {
        type Found = (bool, i8, i8, Vec<u8>, i32);
        type Answer = (
            std::collections::BTreeMap<Vec<u8>, i32>,
            std::collections::BTreeMap<usize, i32>,
            std::collections::BTreeMap<(Vec<u8>, usize), i32>,
        );
        let gc = game_config::make_jumbled_english_game_config();
        let kwg = test_kad(&gc, SWEEP_WORDS);
        let klv = klv::kbwg_from_bytes_alloc(klv::EMPTY_KLV_BYTES.to_vec()).unwrap();
        let dim = gc.board_layout().dim();
        let cols = dim.cols as usize;
        let placed = parse_test_rack(gc.alphabet(), "AEST");
        let pools = ["AERT?", "ACERST", "AEERT??"].map(|pool| parse_test_rack(gc.alphabet(), pool));
        let mut board_tiles = vec![0u8; dim.rows as usize * cols];
        let mut generator = KurniaMoveGenerator::new(&gc);
        let add = |answer: &mut Answer, (down, lane, idx, word, score): &Found| {
            let mut key = (word.iter().filter(|&&t| t != 0))
                .map(|&t| if t & 0x80 != 0 { 0 } else { t })
                .collect::<Vec<_>>();
            key.sort_unstable();
            let best = answer.0.entry(key.clone()).or_insert(*score);
            *best = (*best).max(*score);
            for (i, &t) in word.iter().enumerate() {
                if t != 0 {
                    let at = if *down {
                        (*idx as usize + i) * cols + *lane as usize
                    } else {
                        *lane as usize * cols + *idx as usize + i
                    };
                    let best = answer.1.entry(at).or_insert(*score);
                    *best = (*best).max(*score);
                    let best = answer.2.entry((key.clone(), at)).or_insert(*score);
                    *best = (*best).max(*score);
                }
            }
        };
        let mut fewer = 0;
        for step in 0..=4 {
            if step > 0 {
                let at = dim.at_on_board(7, (step + 5) as i8);
                board_tiles[at] = placed[(step - 1) as usize % placed.len()];
            }
            let board_snapshot = BoardSnapshot {
                board_tiles: &board_tiles,
                game_config: &gc,
                kwg: &kwg,
                anagrams: None,
                rack_lengths: None,
                klv: &klv,
            };
            for pool in &pools {
                let mut all = Vec::<Found>::new();
                generator.pool_plays(&board_snapshot, pool, |down, lane, idx, word, score| {
                    all.push((down, lane, idx, word.to_vec(), score));
                });
                let mut want: Answer = Default::default();
                for found in &all {
                    add(&mut want, found);
                }
                let mut best = Vec::<Found>::new();
                let mut lists = PoolLists::default();
                for lane in 0..(dim.rows + dim.cols) as u32 {
                    let masks = if lane < dim.rows as u32 {
                        (1 << lane, 0)
                    } else {
                        (0, 1 << (lane - dim.rows as u32))
                    };
                    let mut from = 0;
                    loop {
                        let seen = generator.pool_plays_best_window(
                            &board_snapshot,
                            pool,
                            masks,
                            from..from + 3,
                            &mut lists,
                            |down, lane, idx, word, score| {
                                best.push((down, lane, idx, word.to_vec(), score));
                            },
                        );
                        from += 3;
                        if from >= seen {
                            break;
                        }
                    }
                }
                let mut answer: Answer = Default::default();
                for found in &best {
                    add(&mut answer, found);
                }
                assert_eq!(answer, want, "{step} {pool:?}");
                fewer += all.len() - best.len();
            }
        }
        assert!(fewer > 0);
    }

    #[test]
    fn a_pools_top_plays_are_the_best_of_all_its_plays() {
        type Found = (bool, i8, i8, Vec<u8>, i32);
        let gc = game_config::make_jumbled_english_game_config();
        let kwg = test_kad(&gc, SWEEP_WORDS);
        let klv = klv::kbwg_from_bytes_alloc(klv::EMPTY_KLV_BYTES.to_vec()).unwrap();
        let dim = gc.board_layout().dim();
        let cols = dim.cols as usize;
        let placed = parse_test_rack(gc.alphabet(), "AEST");
        let pools = ["AERT?", "ACERST", "AEERT??"].map(|pool| parse_test_rack(gc.alphabet(), pool));
        let mut board_tiles = vec![0u8; dim.rows as usize * cols];
        let mut generator = KurniaMoveGenerator::new(&gc);
        let rank =
            |a: &Found, b: &Found| (b.4, a.0, a.1, a.2, &a.3).cmp(&(a.4, b.0, b.1, b.2, &b.3));
        let mut pruned = 0;
        for step in 0..=4 {
            if step > 0 {
                let at = dim.at_on_board(7, (step + 5) as i8);
                board_tiles[at] = placed[(step - 1) as usize % placed.len()];
            }
            let board_snapshot = BoardSnapshot {
                board_tiles: &board_tiles,
                game_config: &gc,
                kwg: &kwg,
                anagrams: None,
                rack_lengths: None,
                klv: &klv,
            };
            for pool in &pools {
                let mut all = Vec::<Found>::new();
                generator.pool_plays(&board_snapshot, pool, |down, lane, idx, word, score| {
                    all.push((down, lane, idx, word.to_vec(), score));
                });
                all.sort_unstable_by(rank);
                for most in [1, 3, 10] {
                    let mut top = Vec::<Found>::new();
                    generator.pool_plays_top(
                        &board_snapshot,
                        pool,
                        most,
                        |down, lane, idx, word, score| {
                            top.push((down, lane, idx, word.to_vec(), score));
                        },
                    );
                    top.sort_unstable_by(rank);
                    top.truncate(most);
                    assert_eq!(
                        top[..],
                        all[..most.min(all.len())],
                        "{step} {pool:?} {most}"
                    );
                    let mut windowed = Vec::<Found>::new();
                    let mut floor = PoolFloor::default();
                    for lane in 0..(dim.rows + dim.cols) as u32 {
                        let masks = if lane < dim.rows as u32 {
                            (1 << lane, 0)
                        } else {
                            (0, 1 << (lane - dim.rows as u32))
                        };
                        let mut lists = PoolLists::default();
                        let mut from = 0;
                        loop {
                            let seen = generator.pool_plays_top_window(
                                &board_snapshot,
                                pool,
                                PoolWindow {
                                    lanes: masks,
                                    window: from..from + 3,
                                },
                                most,
                                (&mut lists, &mut floor),
                                |down, lane, idx, word, score| {
                                    windowed.push((down, lane, idx, word.to_vec(), score));
                                },
                            );
                            from += 3;
                            if from >= seen {
                                break;
                            }
                        }
                    }
                    pruned += all.len() - windowed.len();
                    windowed.sort_unstable_by(rank);
                    windowed.truncate(most);
                    assert_eq!(windowed, top, "{step} {pool:?} {most} windowed");
                }
            }
        }
        assert!(pruned > 0);
    }

    #[test]
    fn a_pools_plays_through_a_square_are_its_plays_placing_a_tile_there() {
        type Found = (bool, i8, i8, Vec<u8>, i32);
        let configs = [
            (game_config::make_english_game_config(), false, "AERT?"),
            (
                game_config::make_jumbled_english_game_config(),
                true,
                "AEERT??",
            ),
        ];
        let klv = klv::kbwg_from_bytes_alloc(klv::EMPTY_KLV_BYTES.to_vec()).unwrap();
        let mut squares_with_plays = 0;
        let mut windowed = 0;
        for (gc, jumbled, pool) in &configs {
            let kwg = if *jumbled {
                test_kad(gc, SWEEP_WORDS)
            } else {
                test_kwg_of(gc, SWEEP_WORDS)
            };
            let dim = gc.board_layout().dim();
            let cols = dim.cols as usize;
            let pool = &parse_test_rack(gc.alphabet(), pool);
            let board_tiles = vec![0u8; dim.rows as usize * cols];
            let mut generator = KurniaMoveGenerator::new(gc);
            let board_snapshot = BoardSnapshot {
                board_tiles: &board_tiles,
                game_config: gc,
                kwg: &kwg,
                anagrams: None,
                rack_lengths: None,
                klv: &klv,
            };
            let mut all = Vec::<Found>::new();
            generator.pool_plays(&board_snapshot, pool, |down, lane, idx, word, score| {
                all.push((down, lane, idx, word.to_vec(), score));
            });
            for row in 0..dim.rows {
                for col in 0..dim.cols {
                    let want = all
                        .iter()
                        .filter(|(down, lane, idx, word, _)| {
                            let (along, at) = if *down { (col, row) } else { (row, col) };
                            *lane == along
                                && at >= *idx
                                && word.get((at - idx) as usize).is_some_and(|&t| t != 0)
                        })
                        .cloned()
                        .collect::<Vec<_>>();
                    let mut through = Vec::<Found>::new();
                    generator.pool_plays_through(
                        &board_snapshot,
                        pool,
                        (row, col),
                        |down, lane, idx, word, score| {
                            through.push((down, lane, idx, word.to_vec(), score));
                        },
                    );
                    assert_eq!(through, want, "{jumbled} {row} {col}");
                    let mut spans = Vec::<Found>::new();
                    generator.pool_plays_spans_through(
                        &board_snapshot,
                        pool,
                        (row, col),
                        |down, lane, idx, word, score| {
                            spans.push((down, lane, idx, word.to_vec(), score));
                        },
                    );
                    let mut want = want;
                    want.sort_unstable();
                    spans.sort_unstable();
                    assert_eq!(spans, want, "{jumbled} {row} {col}");
                    let rank = |a: &Found, b: &Found| {
                        (b.4, a.0, a.1, a.2, &a.3).cmp(&(a.4, b.0, b.1, b.2, &b.3))
                    };
                    want.sort_unstable_by(rank);
                    for most in [1, 3, 10] {
                        let mut top = Vec::<Found>::new();
                        generator.pool_plays_spans_through_top(
                            &board_snapshot,
                            pool,
                            (row, col),
                            most,
                            |down, lane, idx, word, score| {
                                top.push((down, lane, idx, word.to_vec(), score));
                            },
                        );
                        top.sort_unstable_by(rank);
                        top.truncate(most);
                        let mut wanted = want.clone();
                        wanted.truncate(most);
                        assert_eq!(top, wanted, "{jumbled} {row} {col} {most}");
                    }
                    squares_with_plays += usize::from(!want.is_empty());
                }
            }
            let mut lanes = Vec::<Found>::new();
            for lane in 0..dim.rows {
                generator.pool_plays_lanes(
                    &board_snapshot,
                    pool,
                    (1 << lane, 0),
                    |down, lane, idx, word, score| {
                        lanes.push((down, lane, idx, word.to_vec(), score));
                    },
                );
            }
            for lane in 0..dim.cols {
                generator.pool_plays_lanes(
                    &board_snapshot,
                    pool,
                    (0, 1 << lane),
                    |down, lane, idx, word, score| {
                        lanes.push((down, lane, idx, word.to_vec(), score));
                    },
                );
            }
            for lane in 0..(dim.rows + dim.cols) as u32 {
                let masks = if lane < dim.rows as u32 {
                    (1 << lane, 0)
                } else {
                    (0, 1 << (lane - dim.rows as u32))
                };
                let mut whole = Vec::<Found>::new();
                let total = generator.pool_plays_window(
                    &board_snapshot,
                    pool,
                    masks,
                    0..u64::MAX,
                    None,
                    |down, lane, idx, word, score| {
                        whole.push((down, lane, idx, word.to_vec(), score));
                    },
                );
                windowed += usize::from(*jumbled && total > 50 && !whole.is_empty());
                for size in [1, 2, 7, 50] {
                    let mut pieces = Vec::<Found>::new();
                    let mut from = 0;
                    loop {
                        let seen = generator.pool_plays_window(
                            &board_snapshot,
                            pool,
                            masks,
                            from..from + size,
                            None,
                            |down, lane, idx, word, score| {
                                pieces.push((down, lane, idx, word.to_vec(), score));
                            },
                        );
                        assert_eq!(seen, total);
                        from += size;
                        if from >= seen {
                            break;
                        }
                    }
                    if *jumbled {
                        assert_eq!(pieces, whole, "{lane} {size}");
                    }
                }
            }
            let mut sorted = all.clone();
            sorted.sort_unstable();
            lanes.sort_unstable();
            assert_eq!(lanes, sorted, "{jumbled}");
            let mut again = Vec::<Found>::new();
            generator.pool_plays(&board_snapshot, pool, |down, lane, idx, word, score| {
                again.push((down, lane, idx, word.to_vec(), score));
            });
            assert_eq!(again, all);
        }
        assert!(squares_with_plays > 0);
        assert!(windowed > 0);
    }

    #[test]
    fn a_late_lane_census_scores_words_it_spelled() {
        let gc = game_config::make_english_game_config();
        let alphabet = gc.alphabet();
        let kwg = test_kwg_of(&gc, &["AT", "TA"]);
        let klv = klv::Klv::<kwg::Node24>::empty().unwrap();
        let tables = anagrams::ArmTables::build(&kwg, &gc, false).unwrap();
        let rack = parse_test_rack(alphabet, "AQZ");
        let mut board = vec![0; 225];
        let tiles = (1..alphabet.len()).flat_map(|tile| {
            let kept = rack.contains(&tile) || tile == 20;
            std::iter::repeat_n(tile, (alphabet.freq(tile) - u8::from(kept)) as usize)
        });
        for (cell, tile) in board[..86].iter_mut().zip(tiles) {
            *cell = tile;
        }
        board[7 * 15 + 7] = 20;
        let snapshot = BoardSnapshot {
            board_tiles: &board,
            game_config: &gc,
            kwg: &kwg,
            anagrams: tables.anagrams(),
            rack_lengths: tables.rack_lengths(),
            klv: &klv,
        };
        let mut generator = KurniaMoveGenerator::new(&gc);
        let census = generator.lane_census(&snapshot, tables.anagrams().unwrap(), &rack);
        assert!(generator.num_tiles_in_bag() <= 0);
        let key = alphagram::Nibble.place_value(1);
        let lanes = census.lanes_of(key).unwrap();
        assert!(lanes.contains(&equity::Points::new(2).scaled().raw()));
    }

    #[test]
    #[inline]
    fn a_steered_generation_keeps_the_whole_ones_best() {
        // a board's census over a pool, then racks of that pool: steered by it,
        // with no lane touched or some, a generation keeps the move the whole
        // one keeps, by nibble keys and by layout keys alike
        let gc = game_config::make_english_game_config();
        let kwg = test_kwg(&gc);
        let klv = klv::kbwg_from_bytes_alloc(klv::EMPTY_KLV_BYTES.to_vec()).unwrap();
        let nibbles = anagrams::ArmTables::build(&kwg, &gc, false).unwrap();
        let layout = anagrams::ArmTables::build_by_layout(&kwg, &gc).unwrap();
        let mut board_tiles = vec![0u8; 15 * 15];
        let placed = parse_test_rack(gc.alphabet(), "AEST");
        for i in 0..15 {
            let at = gc
                .board_layout()
                .dim()
                .at_on_board(i, ((i32::from(i) * 7 + 3) % 15) as i8);
            board_tiles[at] = placed[i as usize % placed.len()];
        }
        let pool = parse_test_rack(gc.alphabet(), "AAEEIILNORSST");
        let racks = ["AEILNST", "AEORSST", "EIINORS", "AALNST", "EIOS"]
            .map(|r| parse_test_rack(gc.alphabet(), r));
        let touched = [(0u128, 0u128), (1 << 7, 0), (0b1010, 1 << 3)];
        let mut compared = 0;
        for tables in [&nibbles, &layout] {
            let board_snapshot = BoardSnapshot {
                board_tiles: &board_tiles,
                game_config: &gc,
                kwg: &kwg,
                anagrams: tables.anagrams(),
                rack_lengths: tables.rack_lengths(),
                klv: &klv,
            };
            let mut whole = KurniaMoveGenerator::new(&gc);
            let mut steered = KurniaMoveGenerator::new(&gc);
            let census = steered.lane_census(&board_snapshot, tables.anagrams().unwrap(), &pool);
            for rack in &racks {
                let params = GenMovesParams {
                    board_snapshot: &board_snapshot,
                    rack,
                    max_gen: 1,
                    num_exchanges_by_this_player: 0,
                    pass_policy: PassPolicy::OnlyWhenForced,
                    dynamic_leaves: None,
                };
                whole.gen_moves_unfiltered(&params).unwrap();
                for &(touched_rows, touched_cols) in &touched {
                    steered
                        .gen_moves_unfiltered_lanes_steered(
                            &params,
                            !0,
                            !0,
                            &Steer {
                                census: &census,
                                touched_rows,
                                touched_cols,
                            },
                        )
                        .unwrap();
                    assert!(
                        steered.plays[0].play == whole.plays[0].play,
                        "rack {rack:?}"
                    );
                    compared += 1;
                }
            }
        }
        assert_eq!(compared, 2 * 5 * 3);
    }

    #[test]
    #[inline]
    fn the_kept_spans_are_the_shadow_passs() {
        // the spans kept for a lane under nibble keys are those the shadow pass
        // finds with every letter on hand without limit, in its order, lane by
        // lane as tiles go down a board of 15
        let gc = game_config::make_english_game_config();
        let dim = gc.board_layout().dim();
        let kwg = test_kwg(&gc);
        let nibbles = anagrams::ArmTables::build(&kwg, &gc, false).unwrap();
        let klv = klv::kbwg_from_bytes_alloc(klv::EMPTY_KLV_BYTES.to_vec()).unwrap();
        let rack = parse_test_rack(gc.alphabet(), "AEST");
        let rack_size = gc.rack_size();
        let every_letter = u64::MAX >> (u64::BITS - u32::from(gc.alphabet().len()));
        let zeros = vec![0i32; rack_size as usize + 1];
        let tiles = (0..15)
            .map(|i| (i, ((i32::from(i) * 7 + 3) % 15) as i8))
            .collect::<Vec<_>>();
        let mut board_tiles = vec![0u8; 15 * 15];
        let mut generator = KurniaMoveGenerator::new(&gc);
        let mut spans = 0;
        for step in 0..=tiles.len() {
            if step > 0 {
                let (row, col) = tiles[step - 1];
                board_tiles[dim.at_on_board(row, col)] = rack[(step - 1) % rack.len()];
            }
            let snapshot = BoardSnapshot {
                board_tiles: &board_tiles,
                game_config: &gc,
                kwg: &kwg,
                anagrams: nibbles.anagrams(),
                rack_lengths: nibbles.rack_lengths(),
                klv: &klv,
            };
            place_plays(&mut generator, &snapshot, &rack, false);
            let wb = &mut generator.working_buffer;
            for at in 0..30 {
                let down = at >= 15;
                let start = (at % 15) * 15;
                let end = start + 15;
                let lane_tiles = if down {
                    wb.transposed_board_tiles[start..end].to_vec()
                } else {
                    board_tiles[start..end].to_vec()
                };
                let (allowed, word_multipliers, tile_multipliers) = if down {
                    (
                        &wb.allowed_for_down_plays[start..end],
                        &wb.remaining_word_multipliers_for_down_plays[start..end],
                        &wb.remaining_tile_multipliers_for_down_plays[start..end],
                    )
                } else {
                    (
                        &wb.allowed_for_across_plays[start..end],
                        &wb.remaining_word_multipliers_for_across_plays[start..end],
                        &wb.remaining_tile_multipliers_for_across_plays[start..end],
                    )
                };
                let (perpendicular_word_multipliers, perpendicular_scores) = if down {
                    (
                        &wb.perpendicular_word_multipliers_for_down_plays[start..end],
                        &wb.perpendicular_scores_for_down_plays[start..end],
                    )
                } else {
                    (
                        &wb.perpendicular_word_multipliers_for_across_plays[start..end],
                        &wb.perpendicular_scores_for_across_plays[start..end],
                    )
                };
                let mut found = Vec::new();
                let mut without_limit = [u8::MAX; MAX_ALPHABET_LEN];
                shadow_lane::<15, u16, _, _>(
                    &mut GenPlacePlacementsParams {
                        board_strip: &lane_tiles,
                        alphabet: gc.alphabet(),
                        rack_tally: &mut without_limit,
                        used_tile_scores_shadowl: &mut wb.used_tile_scores_shadowl,
                        used_tile_scores_shadowr: &mut wb.used_tile_scores_shadowr,
                        allowed_strip: allowed,
                        remaining_word_multipliers_strip: word_multipliers,
                        remaining_tile_multipliers_strip: tile_multipliers,
                        perpendicular_word_multipliers_strip: perpendicular_word_multipliers,
                        perpendicular_scores_strip: perpendicular_scores,
                        rack_bits: every_letter,
                        feasible_lengths: !0,
                        descending_scores: &zeros[..rack_size as usize],
                        lane_scaffold: &mut wb.lane_scaffold[at],
                        room: &wb.room,
                        best_leave_values: &zeros,
                        num_max_played: rack_size,
                        rack_tally_shadowl: &mut wb.rack_tally_shadowl,
                        rack_tally_shadowr: &mut wb.rack_tally_shadowr,
                        span_out: &mut wb.span_out,
                        per_span: true,
                        left_bounds: &mut wb.anchor_left_bounds,
                        bounds_stride: 16,
                        check: (),
                    },
                    !down,
                    false,
                    |anchor: i8, left: i8, right: i8, _bound: i32, num_played: u8| {
                        found.push((anchor, left, right, num_played));
                    },
                );
                let mut kept = RcsLane::default();
                RcsGeometry {
                    kept: &mut kept,
                    multi_jumps: &wb.lane_scaffold[at].multi_jumps,
                    tiles: &lane_tiles,
                    allowed,
                    word_multipliers,
                    tile_multipliers,
                    perpendicular_word_multipliers,
                    perpendicular_scores,
                    single_tile_plays: !down,
                    num_max_played: rack_size,
                }
                .find(&mut RcsLists::new());
                let kept_spans = kept
                    .spans
                    .iter()
                    .map(|span| (span.anchor, span.left, span.right, span.num_played))
                    .collect::<Vec<_>>();
                assert_eq!(kept_spans, found, "step {step}, lane {at}");
                spans += found.len();
            }
        }
        assert!(spans > 0);
    }

    #[test]
    #[inline]
    fn nibble_keys_keep_what_a_caller_sets_and_reads() {
        // what a caller sets on the generator before a generation, or reads of
        // it after one, holds under nibble keys: a census sheet (the lean
        // arm's) takes each word once, the bag's count is the board's, and a
        // reset for another kwg leaves its kept state as a new generator's
        let gc = game_config::make_english_game_config();
        let dim = gc.board_layout().dim();
        let kwg = test_kwg(&gc);
        let klv = klv::kbwg_from_bytes_alloc(klv::EMPTY_KLV_BYTES.to_vec()).unwrap();
        let nibbles = anagrams::ArmTables::build(&kwg, &gc, false).unwrap();
        assert!(
            nibbles
                .anagrams()
                .is_some_and(|a| a.nibble_side().is_some())
        );
        let layout = anagrams::ArmTables::build_by_layout(&kwg, &gc).unwrap();
        let rack = parse_test_rack(gc.alphabet(), "?AEST");
        let mut board_tiles = vec![0u8; dim.rows as usize * dim.cols as usize];
        for i in 0..8 {
            let (row, col) = (i, ((i32::from(i) * 7 + 3) % 15) as i8);
            board_tiles[dim.at_on_board(row, col)] = rack[1 + i as usize % (rack.len() - 1)];
        }
        let [by_nibbles, by_layout] = [&nibbles, &layout].map(|tables| BoardSnapshot {
            board_tiles: &board_tiles,
            game_config: &gc,
            kwg: &kwg,
            anagrams: tables.anagrams(),
            rack_lengths: tables.rack_lengths(),
            klv: &klv,
        });
        let census =
            |move_generator: &mut KurniaMoveGenerator,
             board_snapshot: &BoardSnapshot<'_, kwg::Node24, kwg::Node24>| {
                let mut words = Vec::new();
                move_generator
                    .gen_census_sheet(
                        &GenMovesParams {
                            board_snapshot,
                            rack: &rack,
                            max_gen: 1,
                            num_exchanges_by_this_player: i16::MAX,
                            pass_policy: PassPolicy::OnlyWhenForced,
                            dynamic_leaves: None,
                        },
                        PlacePredicate::Dyn(
                            &mut |down, lane, idx, word: &[u8], score: equity::Equity| {
                                words.push((down, lane, idx, word.to_vec(), score));
                                false
                            },
                        ),
                        klv::AdjustLeave::Identity,
                        EquityPredicate::RejectAll,
                    )
                    .unwrap();
                words.sort_unstable();
                words
            };
        let mut by_nibbles_generator = KurniaMoveGenerator::new(&gc);
        let mut by_layout_generator = KurniaMoveGenerator::new(&gc);
        let words = census(&mut by_nibbles_generator, &by_nibbles);
        assert!(!words.is_empty());
        assert_eq!(words, census(&mut by_layout_generator, &by_layout));
        let mut fresh = KurniaMoveGenerator::new(&gc);
        assert!(!place_plays(&mut fresh, &by_nibbles, &rack, false).is_empty());
        place_plays(&mut by_layout_generator, &by_layout, &rack, false);
        assert_eq!(
            fresh.num_tiles_in_bag(),
            by_layout_generator.num_tiles_in_bag()
        );
        // the other kwg lacks every word with an S, so its nodes lie elsewhere
        // and the same board crosses other sets
        let other_words = TEST_WORDS
            .iter()
            .copied()
            .filter(|word| !word.contains('S'))
            .collect::<Vec<_>>();
        let other_kwg = test_kwg_of(&gc, &other_words);
        let other_nibbles = anagrams::ArmTables::build(&other_kwg, &gc, false).unwrap();
        assert!(
            other_nibbles
                .anagrams()
                .is_some_and(|a| a.nibble_side().is_some())
        );
        let by_other = BoardSnapshot {
            board_tiles: &board_tiles,
            game_config: &gc,
            kwg: &other_kwg,
            anagrams: other_nibbles.anagrams(),
            rack_lengths: other_nibbles.rack_lengths(),
            klv: &klv,
        };
        let want = place_plays(&mut KurniaMoveGenerator::new(&gc), &by_other, &rack, false);
        assert_ne!(want, place_plays(&mut fresh, &by_nibbles, &rack, false));
        fresh.reset_for_another_kwg();
        assert_eq!(place_plays(&mut fresh, &by_other, &rack, false), want);
    }

    #[test]
    #[inline]
    fn the_u16_shadow_is_the_shadow() {
        // shadow_pass's body is gen_place_placements_impl's text with the set
        // type, its 0 and 1, and the lane's side as parameters: written back in
        // the u128 instance's terms, it is that function, word for word
        let source = include_str!("movegen.rs");
        let plain_at = source
            .find("#[inline]\nfn gen_place_placements_impl<")
            .unwrap();
        let plain = &source[plain_at..plain_at + source[plain_at..].find("\n}\n").unwrap() + 3];
        let header = "    ($name:ident, $lane:ty, $zero:expr, $one:expr) => {\n";
        let body_at = source.find(header).unwrap() + header.len();
        let body = &source[body_at..body_at + source[body_at..].find("\n    };\n}\n").unwrap() + 1];
        let side = "    let strider_len = if D == 0 { params.board_strip.len() } else { D };
    if D != 0 {
        // the lane's length is a constant of this instance
        assert_eq!(params.board_strip.len(), D);
        assert_eq!(params.allowed_strip.len(), D);
        assert_eq!(params.remaining_word_multipliers_strip.len(), D);
        assert_eq!(params.remaining_tile_multipliers_strip.len(), D);
        assert_eq!(params.perpendicular_word_multipliers_strip.len(), D);
        assert_eq!(params.perpendicular_scores_strip.len(), D);
    }
";
        assert_eq!(body.matches(side).count(), 1);
        let as_plain = body
            .replacen("fn $name<", "fn gen_place_placements_impl<", 1)
            .replacen("    const D: usize,\n", "", 1)
            .replacen(side, "    let strider_len = params.board_strip.len();\n", 1)
            .replacen("refresh::<D>(", "refresh::<0>(", 1)
            .replace("$zero", "0u128")
            .replace("$one", "1u128")
            .replace("$lane", "u128");
        assert_eq!(as_plain, plain);
    }

    #[test]
    #[inline]
    fn racks_of_three_blanks_or_more_play_by_the_tables_as_by_the_walk() {
        // super english deals four blanks: a rack of three or four plays by the
        // tables what the lean arm's walk finds, on an empty board and on one
        // with tiles
        let gc = game_config::make_super_english_game_config();
        let kwg = test_kwg(&gc);
        let klv = klv::kbwg_from_bytes_alloc(klv::EMPTY_KLV_BYTES.to_vec()).unwrap();
        let with_tables = anagrams::ArmTables::build(&kwg, &gc, false).unwrap();
        assert!(matches!(with_tables, anagrams::ArmTables::Anagrams(_)));
        let lean = anagrams::ArmTables::build(&kwg, &gc, true).unwrap();
        assert!(matches!(lean, anagrams::ArmTables::RackLengths(_)));
        let mut fen_parser = display::BoardFenParser::new(gc.alphabet(), gc.board_layout());
        let mut played = 0;
        for fen in [
            "21/21/21/21/21/21/21/21/21/21/21/21/21/21/21/21/21/21/21/21/21",
            "21/21/21/21/21/21/21/21/21/21/8CARE9/8A12/8N12/8E12/21/21/21/21/21/21/21",
        ] {
            let board_tiles = fen_parser.parse(fen).unwrap().to_vec();
            for rack in ["???AE", "????A", "?ANTS??", "???"] {
                let rack = parse_test_rack(gc.alphabet(), rack);
                let [by_tables, by_walk] = [&with_tables, &lean].map(|tables| BoardSnapshot {
                    board_tiles: &board_tiles,
                    game_config: &gc,
                    kwg: &kwg,
                    anagrams: tables.anagrams(),
                    rack_lengths: tables.rack_lengths(),
                    klv: &klv,
                });
                let want = place_plays(&mut KurniaMoveGenerator::new(&gc), &by_walk, &rack, false);
                let found =
                    place_plays(&mut KurniaMoveGenerator::new(&gc), &by_tables, &rack, false);
                assert_eq!(found, want, "{fen} {rack:?}");
                played += usize::from(!want.is_empty());
            }
        }
        assert!(played > 0, "nothing was played");
    }

    #[test]
    fn a_generator_in_u32_plays_as_the_default_one() {
        // the wasm's generator (its sets in u32, never jumping, with no copy of
        // the lean arm for any side) plays what the default one plays on a board
        // of 15 and on one of 21, reused turn after turn
        let klv = klv::kbwg_from_bytes_alloc(klv::EMPTY_KLV_BYTES.to_vec()).unwrap();
        let mut played = 0;
        for (gc, fens) in [
            (
                game_config::make_english_game_config(),
                [
                    "15/15/15/15/15/15/15/15/15/15/15/15/15/15/15",
                    "15/15/15/15/15/15/15/4CARE7/4A10/4N10/4E10/15/15/15/15",
                ],
            ),
            (
                game_config::make_super_english_game_config(),
                [
                    "21/21/21/21/21/21/21/21/21/21/21/21/21/21/21/21/21/21/21/21/21",
                    "21/21/21/21/21/21/21/21/21/21/8CARE9/8A12/8N12/8E12/21/21/21/21/21/21/21",
                ],
            ),
        ] {
            let kwg = test_kwg(&gc);
            let mut fen_parser = display::BoardFenParser::new(gc.alphabet(), gc.board_layout());
            let mut reused = KurniaMoveGenerator::<u32, false, false>::try_new(&gc).unwrap();
            for fen in fens {
                let board_tiles = fen_parser.parse(fen).unwrap().to_vec();
                let board_snapshot = BoardSnapshot {
                    board_tiles: &board_tiles,
                    game_config: &gc,
                    kwg: &kwg,
                    anagrams: None,
                    rack_lengths: None,
                    klv: &klv,
                };
                for rack in ["AEST", "?ANTS", "RETAINS", "Q"] {
                    let rack = parse_test_rack(gc.alphabet(), rack);
                    let want = place_plays(
                        &mut KurniaMoveGenerator::new(&gc),
                        &board_snapshot,
                        &rack,
                        false,
                    );
                    let found = place_plays(&mut reused, &board_snapshot, &rack, false);
                    assert_eq!(found, want, "{fen} {rack:?}");
                    played += usize::from(!want.is_empty());
                }
            }
        }
        assert!(played > 0, "nothing was played");
    }

    #[test]
    fn a_generator_that_never_jumps_plays_as_one_that_does() {
        // with the run letters a lean arm's table keeps, the default generator
        // jumps to a tile's node; one that never jumps walks the run and plays
        // the same
        let gc = game_config::make_english_game_config();
        let kwg = test_kwg(&gc);
        let klv = klv::kbwg_from_bytes_alloc(klv::EMPTY_KLV_BYTES.to_vec()).unwrap();
        let lean = anagrams::ArmTables::build(&kwg, &gc, true).unwrap();
        let rack_lengths = lean.rack_lengths();
        assert!(
            rack_lengths
                .and_then(anagrams::RackLengths::run_letter_bits)
                .is_some()
        );
        let mut fen_parser = display::BoardFenParser::new(gc.alphabet(), gc.board_layout());
        let mut walker = KurniaMoveGenerator::<u128, false>::try_new(&gc).unwrap();
        let mut played = 0;
        for fen in [
            "15/15/15/15/15/15/15/15/15/15/15/15/15/15/15",
            "15/15/15/15/15/15/15/4CARE7/4A10/4N10/4E10/15/15/15/15",
        ] {
            let board_tiles = fen_parser.parse(fen).unwrap().to_vec();
            let board_snapshot = BoardSnapshot {
                board_tiles: &board_tiles,
                game_config: &gc,
                kwg: &kwg,
                anagrams: None,
                rack_lengths,
                klv: &klv,
            };
            for rack in ["AEST", "?ANTS", "RETAINS", "Q"] {
                let rack = parse_test_rack(gc.alphabet(), rack);
                let want = place_plays(
                    &mut KurniaMoveGenerator::new(&gc),
                    &board_snapshot,
                    &rack,
                    false,
                );
                let found = place_plays(&mut walker, &board_snapshot, &rack, false);
                assert_eq!(found, want, "{fen} {rack:?}");
                played += usize::from(!want.is_empty());
            }
        }
        assert!(played > 0, "nothing was played");
    }

    #[test]
    fn a_generator_with_no_copy_for_any_side_refuses_other_boards() {
        // the wasm's generator takes a board of 15 by 15 or of 21 by 21 alone
        for gc in [
            uneven_game_config(9, 9, 4, 4, false),
            uneven_game_config(15, 21, 7, 10, false),
            uneven_game_config(17, 17, 8, 8, false),
        ] {
            assert!(KurniaMoveGenerator::<u32, false, false>::try_new(&gc).is_err());
            assert!(KurniaMoveGenerator::<u32, false, true>::try_new(&gc).is_ok());
        }
        for gc in [
            game_config::make_english_game_config(),
            game_config::make_super_english_game_config(),
        ] {
            assert!(KurniaMoveGenerator::<u32, false, false>::try_new(&gc).is_ok());
        }
    }

    #[test]
    fn a_generator_in_u32_refuses_a_board_past_32() {
        let fits = uneven_game_config(32, 15, 16, 7, false);
        assert!(KurniaMoveGenerator::<u32>::try_new(&fits).is_ok());
        for gc in [
            uneven_game_config(33, 15, 16, 7, false),
            uneven_game_config(15, 33, 7, 16, false),
        ] {
            assert!(KurniaMoveGenerator::<u32>::try_new(&gc).is_err());
            assert!(KurniaMoveGenerator::<u128>::try_new(&gc).is_ok());
        }
    }

    #[inline]
    fn a_reused_generator_agrees_on_an_uneven_board(
        rows: i8,
        cols: i8,
        star_row: i8,
        star_col: i8,
        tiles: &[(i8, i8)],
        per_step: usize,
    ) {
        let gc = uneven_game_config(rows, cols, star_row, star_col, false);
        let transposed_gc = uneven_game_config(cols, rows, star_col, star_row, true);
        let kwg = test_kwg(&gc);
        let layout = alphagram::KeyLayout::of(gc.alphabet(), rows.max(cols) as u8).unwrap();
        let held = anagrams::Anagrams::build(&kwg, layout, rows.max(cols) as u8)
            .unwrap()
            .unwrap();
        assert!(held.fit(&gc) && held.fit(&transposed_gc));
        let klv = klv::kbwg_from_bytes_alloc(klv::EMPTY_KLV_BYTES.to_vec()).unwrap();
        let rack = parse_test_rack(gc.alphabet(), "AEST");
        let area = (rows as isize * cols as isize) as usize;
        let mut walked = Vec::new();
        for anagrams in [None, Some(&held)] {
            let mut board_tiles = vec![0u8; area];
            let mut transposed_board_tiles = vec![0u8; area];
            let mut move_generator = KurniaMoveGenerator::new(&gc);
            let mut transposed_move_generator = KurniaMoveGenerator::new(&transposed_gc);
            let mut lanes_played = [vec![false; rows as usize], vec![false; cols as usize]];
            let steps = tiles.len().div_ceil(per_step);
            for step in 0..steps + 2 {
                if step == steps + 1 {
                    board_tiles.fill(0);
                    transposed_board_tiles.fill(0);
                } else if step > 0 {
                    let placed = tiles.iter().enumerate().skip((step - 1) * per_step);
                    for (i, &(row, col)) in placed.take(per_step) {
                        let tile = rack[i % rack.len()];
                        board_tiles[gc.board_layout().dim().at_on_board(row, col)] = tile;
                        transposed_board_tiles
                            [transposed_gc.board_layout().dim().at_on_board(col, row)] = tile;
                    }
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
                    lanes_played[usize::from(*down)][*lane as usize] = true;
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
            .map(|row| (row, ((i32::from(row) * 7 + 3) % 20) as i8))
            .collect::<Vec<_>>();
        for per_step in [5, 25] {
            a_reused_generator_agrees_on_an_uneven_board(100, 20, 66, 9, &tiles, per_step);
        }
    }

    #[test]
    #[inline]
    fn the_widest_board_agrees_with_its_transpose() {
        let tiles = (0..127)
            .map(|row| (row, ((i32::from(row) * 45 + 7) % 127) as i8))
            .collect::<Vec<_>>();
        a_reused_generator_agrees_on_an_uneven_board(127, 127, 70, 90, &tiles, 4);
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
        let klv = klv::kbwg_from_bytes_alloc(klv::EMPTY_KLV_BYTES.to_vec()).unwrap();
        let mut board_tiles = vec![0u8; (rows as isize * cols as isize) as usize];
        let rack = parse_test_rack(gc.alphabet(), "AEST");
        board_tiles[gc.board_layout().dim().at_on_board(0, 9)] = rack[0];
        board_tiles[gc.board_layout().dim().at_on_board(1, 9)] = rack[3];
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

    // each play as text: its equity, then the play
    #[inline]
    fn plays_text(
        plays: &[ValuedMove],
        board_snapshot: &BoardSnapshot<'_, kwg::Node24, kwg::Node24>,
    ) -> Vec<String> {
        plays
            .iter()
            .map(|p| format!("{} {}", p.equity.raw(), p.play.fmt(board_snapshot).unwrap()))
            .collect()
    }

    // every place move the lean arm's walk finds on a board, its threshold
    // never rising, each valued as a generation values it (on an empty board
    // the opening's penalty counted as the walk's callers counted it), sorted
    // as a generation sorts them; raw, each unvalued, the raw walk's
    #[inline]
    fn walked_plays<G: LaneBits, const JUMPS: bool, const ANY_SIDE: bool>(
        move_generator: &mut KurniaMoveGenerator<G, JUMPS, ANY_SIDE>,
        board_snapshot: &BoardSnapshot<'_, kwg::Node24, kwg::Node24>,
        rack: &[u8],
        is_census: bool,
        raw: bool,
    ) -> Vec<ValuedMove> {
        let alphabet = board_snapshot.game_config.alphabet();
        let board_layout = board_snapshot.game_config.board_layout();
        let working_buffer = &mut move_generator.working_buffer;
        working_buffer.is_census = is_census;
        working_buffer.init::<0, true, ByLayout, _, _>(
            board_snapshot,
            rack,
            klv::AdjustLeave::Identity,
            None,
            working_buffer.multi_leaves.dense_most(),
            None,
        );
        let multi_leaves = std::mem::take(&mut working_buffer.multi_leaves);
        let empty = working_buffer.num_tiles_on_board == 0;
        let mut plays = Vec::new();
        let found_place_move = |down: bool,
                                lane: i8,
                                idx: i8,
                                word: &[u8],
                                score: equity::Equity,
                                leave_value: equity::Equity| {
            let other_adjustments = if empty && !raw {
                (idx..)
                    .zip(word)
                    .filter(|&(i, &tile)| {
                        tile != 0
                            && alphabet.is_vowel(tile)
                            && if down {
                                board_layout.danger_stars(true)[usize::from(i.cast_unsigned())]
                            } else {
                                board_layout.danger_stars(false)[usize::from(i.cast_unsigned())]
                            }
                    })
                    .count() as i32
                    * -equity::OPENING_HOTSPOT_PENALTY
            } else {
                0
            };
            plays.push(ValuedMove {
                equity: equity::Equity::new(if raw {
                    0
                } else {
                    score.raw() + leave_value.raw() + other_adjustments
                }),
                play: Play::Place {
                    down,
                    lane,
                    idx,
                    word: word.into(),
                    score,
                },
            });
            equity::Equity::NEG_INFINITY
        };
        if raw {
            kurnia_gen_raw_place_moves::<false, 0, G, _, _, _, _, _, _, _>(
                (),
                board_snapshot,
                &mut *working_buffer,
                &multi_leaves,
                found_place_move,
            );
        } else {
            for _ in kurnia_gen_place_moves_iter_lean::<0, G, _, _, _, _, _, _, _, _, _>(
                KurniaIterParams {
                    runs: (),
                    board_snapshot,
                    working_buffer: &mut *working_buffer,
                    multi_leaves: &multi_leaves,
                    found_place_move,
                    can_accept: |_: equity::Equity| true,
                    current_threshold: || equity::Equity::NEG_INFINITY,
                },
            ) {}
        }
        working_buffer.multi_leaves = multi_leaves;
        working_buffer.is_census = false;
        plays.sort_unstable();
        plays
    }

    // the place moves of a generation's plays on the lanes in the masks, each
    // its way, lane, place, tiles, score, and equity, in its order or sorted
    #[inline]
    fn place_list(
        plays: &[ValuedMove],
        rows: u128,
        cols: u128,
        sorted: bool,
    ) -> Vec<(bool, i8, i8, Vec<u8>, i32, i32)> {
        let mut out = plays
            .iter()
            .filter_map(|p| match &p.play {
                Play::Place {
                    down,
                    lane,
                    idx,
                    word,
                    score,
                } if ((if *down { cols } else { rows }) >> lane) & 1 != 0 => Some((
                    *down,
                    *lane,
                    *idx,
                    word.to_vec(),
                    score.raw(),
                    p.equity.raw(),
                )),
                _ => None,
            })
            .collect::<Vec<_>>();
        if sorted {
            out.sort_unstable();
        }
        out
    }

    // an empty board's moves made without the walk are the walk's: every play,
    // with its score and its equity (the opening's penalty taken), from every
    // kind of generation, on boards of 15 and 21, on punctured and uneven ones,
    // and under jumbled rules, by fresh generators and by one kept from board
    // to board (the board after an empty one is played as a fresh one plays it)
    #[test]
    #[inline]
    fn an_empty_board_is_played_as_the_walk_plays_it() {
        let leaves: &[(&[u8], f32)] = &[
            (&[0], 25.5),
            (&[0, 5], 30.25),
            (&[1], 1.5),
            (&[1, 1], -4.0),
            (&[5], 2.0),
            (&[5, 19], 6.125),
            (&[19], 8.0),
            (&[20], 0.5),
            (&[26], -3.75),
        ];
        let klv = klv::kbwg_from_bytes_alloc(klv::make_klv2(leaves)).unwrap();
        let configs = [
            game_config::make_english_game_config(),
            game_config::make_super_english_game_config(),
            game_config::make_punctured_english_game_config(),
            game_config::make_hong_kong_english_game_config(),
            uneven_game_config(15, 15, 6, 9, false),
            uneven_game_config(15, 15, 9, 6, true),
            uneven_game_config(21, 13, 14, 4, false),
            game_config::make_jumbled_english_game_config(),
            game_config::make_jumbled_super_english_game_config(),
        ];
        let all = !0u128;
        for (ci, gc) in configs.iter().enumerate() {
            let jumbled = match gc.game_rules() {
                game_config::GameRules::Classic | game_config::GameRules::Woadwage => false,
                game_config::GameRules::Jumbled | game_config::GameRules::JumbledWoadwage => true,
            };
            let kwg = if jumbled {
                test_kad(gc, SWEEP_WORDS)
            } else {
                sweep_kwg_covering(gc)
            };
            let run_letter_bits = kwg.compute_letter_bits().letter_bits;
            let detected = anagrams::ArmTables::build(&kwg, gc, false).unwrap();
            let by_layout = anagrams::ArmTables::build_by_layout(&kwg, gc).unwrap();
            let lean = anagrams::ArmTables::build(&kwg, gc, true).unwrap();
            let none = anagrams::ArmTables::None;
            let dim = gc.board_layout().dim();
            let empty = vec![0u8; (dim.rows as isize * dim.cols as isize) as usize];
            let star_row = gc.board_layout().star_row();
            let star_col = gc.board_layout().star_col();
            // the first config takes every rack under every kind of tables;
            // another takes some racks under the tables detected or under none.
            // Under jumbled rules a long rack with a blank has too many
            // arrangements to list again and again (and the tables are the rack
            // lengths whatever is asked); a rack of 21 tiles has leaves too
            // many to be dense
            let first = ci == 0;
            let racks = if jumbled {
                vec!["AEINRST", "CARTONS", "AE", "CAT", "?AT", "C?T", "?A"]
            } else if gc.rack_size() > 7 {
                vec!["AEINRSTCS", "?EINRSTCA", "??TANS", "AE", "?A"]
            } else if first {
                let mut racks = SWEEP_RACKS.to_vec();
                racks.push("AABCDEEFGHIJKLMNORST?");
                racks
            } else {
                vec![
                    "AEINRST",
                    "CARTONS",
                    "?EINRST",
                    "?AT",
                    "??TANS",
                    "AE",
                    "AABCDEEFGHIJKLMNORST?",
                ]
            };
            let arms: &[(&str, &anagrams::ArmTables)] = if jumbled {
                &[("lean", &lean), ("none", &none)]
            } else if first {
                &[
                    ("detected", &detected),
                    ("by layout", &by_layout),
                    ("lean", &lean),
                    ("none", &none),
                ]
            } else {
                &[("detected", &detected), ("none", &none)]
            };
            // the wasm's generator, for the two boards it takes
            let square = dim.rows == dim.cols && (dim.rows == 15 || dim.rows == 21);
            let mut kept = KurniaMoveGenerator::new(gc);
            let mut kept_wasm = KurniaMoveGenerator::<u32, false, false>::try_new(gc).ok();
            let mut played_on = 0;
            for (ri, rack) in racks.iter().enumerate() {
                let rack = parse_test_rack(gc.alphabet(), rack);
                let walked_snapshot = BoardSnapshot {
                    board_tiles: &empty,
                    game_config: gc,
                    kwg: &kwg,
                    anagrams: None,
                    rack_lengths: None,
                    klv: &klv,
                };
                let walk = |is_census: bool, raw: bool| {
                    walked_plays(
                        &mut KurniaMoveGenerator::new(gc),
                        &walked_snapshot,
                        &rack,
                        is_census,
                        raw,
                    )
                };
                let walked = walk(false, false);
                let walked_list = place_list(&walked, all, all, false);
                let walked_raw = place_list(&walk(false, true), all, all, true);
                let census = place_list(&walk(true, false), all, all, false);
                let mut census_calls = census
                    .iter()
                    .map(|(down, lane, idx, word, score, _)| {
                        (*down, *lane, *idx, word.clone(), *score)
                    })
                    .collect::<Vec<_>>();
                census_calls.sort_unstable();
                if !walked.is_empty() {
                    played_on += 1;
                }
                for &(tname, tables) in arms {
                    let board_snapshot = BoardSnapshot {
                        anagrams: tables.anagrams(),
                        rack_lengths: tables.rack_lengths(),
                        ..walked_snapshot
                    };
                    let params = |max_gen: usize| GenMovesParams {
                        board_snapshot: &board_snapshot,
                        rack: &rack,
                        max_gen,
                        num_exchanges_by_this_player: i16::MAX,
                        pass_policy: PassPolicy::OnlyWhenForced,
                        dynamic_leaves: None,
                    };
                    let why = format!("{tname}, rack {ri}");
                    // a fresh generator at every cap, and raw
                    let mut fresh = KurniaMoveGenerator::new(gc);
                    for max_gen in [1, 5, usize::MAX] {
                        fresh.gen_moves_unfiltered(&params(max_gen)).unwrap();
                        assert_eq!(
                            place_list(&fresh.plays, all, all, false),
                            &walked_list[..walked_list.len().min(max_gen)],
                            "{why}, fresh, at a cap of {max_gen}"
                        );
                    }
                    assert_eq!(
                        fresh.working_buffer.multi_leaves.is_dense(),
                        rack.len() <= 9,
                        "{why}, dense or not"
                    );
                    let mut fresh = KurniaMoveGenerator::new(gc);
                    fresh
                        .gen_moves_unfiltered_lean(&params(usize::MAX))
                        .unwrap();
                    assert_eq!(
                        place_list(&fresh.plays, all, all, false),
                        walked_list,
                        "{why}, fresh, lean"
                    );
                    let mut fresh = KurniaMoveGenerator::new(gc);
                    fresh
                        .gen_moves_raw_all_unsorted(
                            &board_snapshot,
                            &rack,
                            i16::MAX,
                            PassPolicy::OnlyWhenForced,
                        )
                        .unwrap();
                    assert_eq!(
                        place_list(&fresh.plays, all, all, true),
                        walked_raw,
                        "{why}, fresh, raw"
                    );
                    // the kept generator, every kind
                    kept.gen_moves_unfiltered(&params(5)).unwrap();
                    assert_eq!(
                        place_list(&kept.plays, all, all, false),
                        &walked_list[..walked_list.len().min(5)],
                        "{why}, kept, at a cap of 5"
                    );
                    kept.gen_moves_unfiltered_lean(&params(usize::MAX)).unwrap();
                    assert_eq!(
                        place_list(&kept.plays, all, all, false),
                        walked_list,
                        "{why}, kept, lean"
                    );
                    {
                        let params = params(usize::MAX);
                        let mut fut = std::pin::pin!(kept.gen_moves_filtered_async_lean(
                            &params,
                            PlacePredicate::AcceptAll,
                            klv::AdjustLeave::Identity,
                            EquityPredicate::AcceptAll,
                            || std::future::ready(()),
                        ));
                        let mut cx = std::task::Context::from_waker(std::task::Waker::noop());
                        loop {
                            if let std::task::Poll::Ready(result) = fut.as_mut().poll(&mut cx) {
                                result.unwrap();
                                break;
                            }
                        }
                    }
                    assert_eq!(
                        place_list(&kept.plays, all, all, false),
                        walked_list,
                        "{why}, kept, async"
                    );
                    kept.gen_moves_raw_lanes_unsorted_by_runs(
                        &run_letter_bits,
                        &board_snapshot,
                        &rack,
                        all,
                        all,
                    )
                    .unwrap();
                    assert_eq!(
                        place_list(&kept.plays, all, all, true),
                        walked_raw,
                        "{why}, kept, by runs"
                    );
                    for (rows, cols) in [
                        (1u128 << star_row, 0u128),
                        (0, 1u128 << star_col),
                        (!(1u128 << star_row), !(1u128 << star_col)),
                    ] {
                        kept.gen_moves_raw_lanes_unsorted(
                            &board_snapshot,
                            &rack,
                            i16::MAX,
                            PassPolicy::OnlyWhenForced,
                            rows,
                            cols,
                        )
                        .unwrap();
                        let mut want = walked_raw.clone();
                        want.retain(|(down, lane, ..)| {
                            ((if *down { cols } else { rows }) >> lane) & 1 != 0
                        });
                        assert_eq!(
                            place_list(&kept.plays, all, all, true),
                            want,
                            "{why}, kept, raw, rows {rows:#x}, columns {cols:#x}"
                        );
                        kept.gen_moves_unfiltered_lanes(&params(usize::MAX), rows, cols)
                            .unwrap();
                        assert_eq!(
                            place_list(&kept.plays, all, all, false),
                            place_list(&walked, rows, cols, false),
                            "{why}, kept, rows {rows:#x}, columns {cols:#x}"
                        );
                    }
                    // the census's every call, and what it keeps
                    let mut calls = Vec::new();
                    kept.gen_census_sheet(
                        &params(usize::MAX),
                        PlacePredicate::Dyn(&mut |down, lane, idx, word: &[u8], score| {
                            calls.push((down, lane, idx, word.to_vec(), score.raw()));
                            true
                        }),
                        klv::AdjustLeave::Identity,
                        EquityPredicate::AcceptAll,
                    )
                    .unwrap();
                    calls.sort_unstable();
                    assert_eq!(calls, census_calls, "{why}, kept, the census's calls");
                    assert_eq!(
                        place_list(&kept.plays, all, all, false),
                        census,
                        "{why}, kept, the census"
                    );
                    if square && let Some(kept_wasm) = kept_wasm.as_mut() {
                        for max_gen in [1, usize::MAX] {
                            kept_wasm
                                .gen_moves_unfiltered_lean(&params(max_gen))
                                .unwrap();
                            assert_eq!(
                                place_list(&kept_wasm.plays, all, all, false),
                                &walked_list[..walked_list.len().min(max_gen)],
                                "{why}, the wasm's, at a cap of {max_gen}"
                            );
                        }
                        kept_wasm
                            .gen_moves_raw_all_unsorted_lean(
                                &board_snapshot,
                                &rack,
                                i16::MAX,
                                PassPolicy::OnlyWhenForced,
                            )
                            .unwrap();
                        assert_eq!(
                            place_list(&kept_wasm.plays, all, all, true),
                            walked_raw,
                            "{why}, the wasm's, raw"
                        );
                    }
                    // the board the best play makes, played by the kept
                    // generators as by fresh ones, before the next empty one
                    let Some(ValuedMove {
                        play:
                            Play::Place {
                                down,
                                lane,
                                idx,
                                word,
                                ..
                            },
                        ..
                    }) = walked.first()
                    else {
                        continue;
                    };
                    let mut board_tiles = empty.clone();
                    let strider = dim.lane(*down, *lane);
                    for (i, &tile) in (*idx..).zip(word.iter()) {
                        board_tiles[strider.at_in_lane(i)] = tile;
                    }
                    let next_snapshot = BoardSnapshot {
                        board_tiles: &board_tiles,
                        ..board_snapshot
                    };
                    let next_params = GenMovesParams {
                        board_snapshot: &next_snapshot,
                        ..params(usize::MAX)
                    };
                    let mut fresh = KurniaMoveGenerator::new(gc);
                    fresh.gen_moves_unfiltered(&next_params).unwrap();
                    kept.gen_moves_unfiltered(&next_params).unwrap();
                    assert_eq!(
                        plays_text(&kept.plays, &next_snapshot),
                        plays_text(&fresh.plays, &next_snapshot),
                        "{why}, the board after"
                    );
                    if square && let Some(kept_wasm) = kept_wasm.as_mut() {
                        fresh.gen_moves_unfiltered_lean(&next_params).unwrap();
                        kept_wasm.gen_moves_unfiltered_lean(&next_params).unwrap();
                        assert_eq!(
                            plays_text(&kept_wasm.plays, &next_snapshot),
                            plays_text(&fresh.plays, &next_snapshot),
                            "{why}, the wasm's, the board after"
                        );
                    }
                }
            }
            assert!(played_on > 3, "too few racks played anything");
        }
    }

    mod bag_term {
        use super::*;

        // the SEAT board, its snapshot's pieces, and the bag it leaves counted
        // from the tiles alone: 100 less the 4 on the board less two full racks
        const BAG_TERM_FEN: &str = "15/15/15/15/15/15/15/6SEAT5/15/15/15/15/15/15/15";
        const BAG_TERM_BAG: i16 = 100 - 4 - 2 * 7;

        // every play of the rack on the SEAT board, by its text, with its equity;
        // dense or not, and by the walk or the lean arm
        fn bag_term_plays(
            bag_term: Option<BagTerm>,
            rack: &str,
            max_gen: usize,
            dense: bool,
            lean: bool,
        ) -> Vec<(String, i32, Option<u8>)> {
            bag_term_plays_on(
                BagTermPosition {
                    jumbled: false,
                    fen: BAG_TERM_FEN,
                    bag: BAG_TERM_BAG,
                    rack,
                },
                bag_term,
                max_gen,
                dense,
                lean,
            )
        }

        struct BagTermPosition<'a> {
            jumbled: bool,
            fen: &'a str,
            bag: i16,
            rack: &'a str,
        }

        // as bag_term_plays, on the board given (its bag counted by the caller),
        // by the jumbled rules or the classic
        fn bag_term_plays_on(
            BagTermPosition {
                jumbled,
                fen,
                bag,
                rack,
            }: BagTermPosition<'_>,
            bag_term: Option<BagTerm>,
            max_gen: usize,
            dense: bool,
            lean: bool,
        ) -> Vec<(String, i32, Option<u8>)> {
            let gc = if jumbled {
                game_config::make_jumbled_english_game_config()
            } else {
                game_config::make_english_game_config()
            };
            let kwg = if jumbled {
                test_kad(&gc, TEST_WORDS)
            } else {
                test_kwg(&gc)
            };
            let klv = klv::kbwg_from_bytes_alloc(klv::EMPTY_KLV_BYTES.to_vec()).unwrap();
            let mut fen_parser = display::BoardFenParser::new(gc.alphabet(), gc.board_layout());
            let board = fen_parser.parse(fen).unwrap().to_vec();
            let rack = parse_test_rack(gc.alphabet(), rack);
            let snapshot = BoardSnapshot {
                board_tiles: &board,
                game_config: &gc,
                kwg: &kwg,
                anagrams: None,
                rack_lengths: None,
                klv: &klv,
            };
            fn generate<const BAG_TERM: bool>(
                mut move_generator: KurniaMoveGenerator<u128, true, true, BAG_TERM>,
                params: &GenMovesParams<'_, kwg::Node24, kwg::Node24>,
                lean: bool,
            ) -> (Vec<ValuedMove>, i32) {
                if lean {
                    move_generator.gen_moves_unfiltered_lean(params).unwrap();
                } else {
                    move_generator.gen_moves_unfiltered(params).unwrap();
                }
                let bag = move_generator.num_tiles_in_bag();
                (move_generator.plays, bag)
            }
            let params = GenMovesParams {
                board_snapshot: &snapshot,
                rack: &rack,
                max_gen,
                num_exchanges_by_this_player: 0,
                pass_policy: PassPolicy::OnlyWhenForced,
                dynamic_leaves: None,
            };
            let dense_most = if dense { klv::DENSE_LEAVES_MOST } else { 0 };
            let (plays, bag_seen) = match bag_term {
                None => generate(
                    KurniaMoveGenerator::<u128, true, true, false>::new_in_dense(&gc, dense_most),
                    &params,
                    lean,
                ),
                Some(bag_term) => generate(
                    KurniaMoveGenerator::<u128, true, true, true>::with_bag_term_in(
                        &gc, bag_term, dense_most,
                    ),
                    &params,
                    lean,
                ),
            };
            assert_eq!(bag_seen, i32::from(bag));
            plays
                .iter()
                .map(|p| {
                    let num_played = match &p.play {
                        Play::Exchange { .. } => None,
                        Play::Place { word, .. } => {
                            Some(word.iter().filter(|&&tile| tile != 0).count() as u8)
                        }
                    };
                    (
                        p.play.fmt(&snapshot).unwrap().to_string(),
                        p.equity.raw(),
                        num_played,
                    )
                })
                .collect()
        }

        // a term for each bag size a play of up to 7 tiles can leave, each its own
        fn every_bag_term() -> BagTerm {
            BagTerm::new(
                BAG_TERM_BAG - 7,
                vec![-7000, 11000, -5000, 3000, 2000, -13000, 17000, 0],
            )
        }

        #[test]
        #[inline]
        fn a_bag_term_adds_to_each_placement_by_the_bag_it_leaves() {
            let term = every_bag_term();
            for (dense, lean) in [(true, false), (false, false), (true, true), (false, true)] {
                let without = bag_term_plays(None, "AAEESTT", usize::MAX, dense, lean);
                let with = bag_term_plays(Some(term.clone()), "AAEESTT", usize::MAX, dense, lean);
                let mut with = with
                    .into_iter()
                    .map(|(text, equity, num_played)| (text, (equity, num_played)))
                    .collect::<crate::fash::MyHashMap<_, _>>();
                assert_eq!(without.len(), with.len());
                let mut placements = 0;
                for (text, equity, num_played) in without {
                    let (with_equity, _) = with.remove(&text).unwrap();
                    // the bag the board leaves, less the tiles the play places
                    let expected = num_played.map_or(0, |n| term.at(BAG_TERM_BAG - i16::from(n)));
                    placements += u32::from(num_played.is_some());
                    assert_eq!(with_equity - equity, expected, "{text} {dense} {lean}");
                }
                assert!(placements > 5);
            }
        }

        #[test]
        #[inline]
        fn a_bag_term_adds_on_every_walk() {
            // the empty board's walk, one tile's plays, the jumbled walk, and the
            // classic: each placement gets its term, each exchange none, and the
            // best few are the best few of every play
            const EMPTY_FEN: &str = "15/15/15/15/15/15/15/15/15/15/15/15/15/15/15";
            for (jumbled, fen, bag, rack) in [
                (false, EMPTY_FEN, 100 - 2 * 7, "AAEESTT"),
                (false, BAG_TERM_FEN, BAG_TERM_BAG, "A"),
                (true, BAG_TERM_FEN, BAG_TERM_BAG, "AEST"),
                (true, EMPTY_FEN, 100 - 2 * 7, "AEST"),
            ] {
                let term = BagTerm::new(
                    bag - 7,
                    vec![-7000, 11000, -5000, 3000, 2000, -13000, 17000, 0],
                );
                for (dense, lean) in [(true, false), (false, false), (true, true), (false, true)] {
                    let why = format!("{jumbled} {fen} {rack} {dense} {lean}");
                    let plays = |term: Option<BagTerm>, max_gen: usize| {
                        bag_term_plays_on(
                            BagTermPosition {
                                jumbled,
                                fen,
                                bag,
                                rack,
                            },
                            term,
                            max_gen,
                            dense,
                            lean,
                        )
                    };
                    let without = plays(None, usize::MAX);
                    let with = plays(Some(term.clone()), usize::MAX);
                    assert_eq!(without.len(), with.len(), "{why}");
                    let with_by_text = with
                        .iter()
                        .map(|p| (p.0.clone(), p.1))
                        .collect::<crate::fash::MyHashMap<_, _>>();
                    let mut placements = 0;
                    for (text, equity, num_played) in &without {
                        let expected = num_played.map_or(0, |n| term.at(bag - i16::from(n)));
                        placements += u32::from(num_played.is_some());
                        assert_eq!(with_by_text[text] - equity, expected, "{text} {why}");
                    }
                    assert!(placements > 0, "{why}");
                    let mut want = with.iter().map(|p| p.1).collect::<Vec<_>>();
                    want.sort_unstable_by(|a, b| b.cmp(a));
                    for max_gen in [1, 3] {
                        let top = plays(Some(term.clone()), max_gen);
                        let n = max_gen.min(want.len());
                        assert_eq!(
                            top.iter().map(|p| p.1).collect::<Vec<_>>(),
                            want[..n],
                            "{max_gen} {why}"
                        );
                    }
                }
            }
        }

        #[test]
        #[inline]
        fn a_bag_term_leaves_each_exchange_as_it_was() {
            let term = every_bag_term();
            for (dense, lean) in [(true, false), (false, false), (true, true), (false, true)] {
                let without = bag_term_plays(None, "AAEESTT", usize::MAX, dense, lean);
                let with = bag_term_plays(Some(term.clone()), "AAEESTT", usize::MAX, dense, lean);
                let exchanges = |plays: Vec<(String, i32, Option<u8>)>| {
                    let mut v = plays
                        .into_iter()
                        .filter(|p| p.2.is_none())
                        .map(|p| (p.0, p.1))
                        .collect::<Vec<_>>();
                    v.sort_unstable();
                    v
                };
                let without = exchanges(without);
                assert!(without.len() > 50);
                assert_eq!(exchanges(with), without, "{dense} {lean}");
            }
        }

        #[test]
        #[inline]
        fn a_bag_term_never_prunes_the_best_play() {
            // each count played in turn made the best by far, then the worst; a
            // bound without the term would prune the play it lifts
            for num_played in 1..=7i16 {
                for sign in [1, -1] {
                    let mut values = vec![0i32; 8];
                    values[(7 - num_played) as usize] = sign * 500_000;
                    let term = BagTerm::new(BAG_TERM_BAG - 7, values);
                    for (dense, lean) in
                        [(true, false), (false, false), (true, true), (false, true)]
                    {
                        let all =
                            bag_term_plays(Some(term.clone()), "AAEESTT", usize::MAX, dense, lean);
                        let best = all.iter().map(|p| p.1).max().unwrap();
                        for max_gen in [1, 3] {
                            let top =
                                bag_term_plays(Some(term.clone()), "AAEESTT", max_gen, dense, lean);
                            assert_eq!(top[0].1, best, "{num_played} {sign} {dense} {lean}");
                            let mut want = all.iter().map(|p| p.1).collect::<Vec<_>>();
                            want.sort_unstable_by(|a, b| b.cmp(a));
                            assert_eq!(
                                top.iter().map(|p| p.1).collect::<Vec<_>>(),
                                want[..max_gen],
                                "{num_played} {sign} {dense} {lean}"
                            );
                        }
                    }
                }
            }
        }

        #[test]
        #[inline]
        fn a_bag_term_below_every_placement_leaves_an_exchange_best() {
            // a term that sinks every placement: the bound on exchanges must not
            // carry it, or the exchanges are cut off by the placements' threshold
            let term = BagTerm::new(BAG_TERM_BAG - 7, vec![-500_000; 8]);
            for (dense, lean) in [(true, false), (false, false), (true, true), (false, true)] {
                for max_gen in [1, 2, 5] {
                    let top = bag_term_plays(Some(term.clone()), "AAEESTT", max_gen, dense, lean);
                    assert_eq!(top.len(), max_gen);
                    assert!(
                        top.iter().all(|p| p.2.is_none() && p.1 == 0),
                        "{max_gen} {dense} {lean} {top:?}"
                    );
                }
            }
        }

        #[test]
        #[inline]
        fn a_bag_term_reads_the_board_not_a_rebuilt_bag() {
            // the generator counts the bag from the board, as it is in the game
            // (num_tiles_in_bag is asserted in bag_term_plays): a term keyed to
            // one bag size lifts only the placements leaving it, here of 2 tiles
            let mut values = vec![0i32; 8];
            values[5] = 9000;
            let term = BagTerm::new(BAG_TERM_BAG - 7, values);
            let without = bag_term_plays(None, "AAEESTT", usize::MAX, true, false);
            let with = bag_term_plays(Some(term), "AAEESTT", usize::MAX, true, false);
            let without = without
                .into_iter()
                .map(|p| (p.0, p.1))
                .collect::<crate::fash::MyHashMap<_, _>>();
            let lifted = with
                .iter()
                .filter(|p| without[&p.0] != p.1)
                .map(|p| p.2)
                .collect::<Vec<_>>();
            assert!(!lifted.is_empty());
            assert!(lifted.iter().all(|&n| n == Some(2)));
        }

        #[test]
        #[inline]
        fn a_bag_term_adds_nothing_once_the_bag_is_empty() {
            let gc = game_config::make_english_game_config();
            let kwg = test_kwg(&gc);
            let klv = klv::kbwg_from_bytes_alloc(klv::EMPTY_KLV_BYTES.to_vec()).unwrap();
            let mut fen_parser = display::BoardFenParser::new(gc.alphabet(), gc.board_layout());
            // 90 tiles on the board: the bag is empty
            let board = fen_parser
                .parse("15/15/15/15/15/15/15/6SEAT5/15/AAAAAAAAAAAAAAA/AAAAAAAAAAAAAAA/AAAAAAAAAAAAAAA/AAAAAAAAAAAAAAA/AAAAAAAAAAAAAAA/AAAAAAAAAA5")
                .unwrap()
                .to_vec();
            let rack = parse_test_rack(gc.alphabet(), "AEST");
            let snapshot = BoardSnapshot {
                board_tiles: &board,
                game_config: &gc,
                kwg: &kwg,
                anagrams: None,
                rack_lengths: None,
                klv: &klv,
            };
            let params = GenMovesParams {
                board_snapshot: &snapshot,
                rack: &rack,
                max_gen: usize::MAX,
                num_exchanges_by_this_player: 0,
                pass_policy: PassPolicy::OnlyWhenForced,
                dynamic_leaves: None,
            };
            let mut plain = KurniaMoveGenerator::new(&gc);
            plain.gen_moves_unfiltered(&params).unwrap();
            assert!(plain.num_tiles_in_bag() <= 0);
            let mut with =
                KurniaMoveGenerator::with_bag_term(&gc, BagTerm::new(-7, vec![1_000_000; 15]));
            with.gen_moves_unfiltered(&params).unwrap();
            assert!(plain.plays.len() > 3);
            assert!(with.plays == plain.plays);
        }

        #[test]
        #[inline]
        fn a_bag_term_parses_its_values_file() {
            let term =
                BagTerm::parse("# by the bag left\n-2 -1.5\n\n1 6.04 # one left\n0 0\n").unwrap();
            assert_eq!(term, BagTerm::new(-2, vec![-1500, 0, 0, 6040]));
            assert_eq!(term.at(-3), 0);
            assert_eq!(term.at(-2), -1500);
            assert_eq!(term.at(1), 6040);
            assert_eq!(term.at(2), 0);
            let full_range = BagTerm::parse(&format!("{} 1\n{} -1\n", i16::MIN, i16::MAX)).unwrap();
            assert_eq!(full_range.at(i16::MIN), equity::SCALE);
            assert_eq!(full_range.at(i16::MAX), -equity::SCALE);
            assert!(BagTerm::parse("").is_err());
            assert!(BagTerm::parse("1 2\n1 3\n").is_err());
            assert!(BagTerm::parse("1\n").is_err());
            assert!(BagTerm::parse("1 2 3\n").is_err());
            assert!(BagTerm::parse("x 2\n").is_err());
            assert!(BagTerm::parse("1 1e12\n").is_err());
        }
    }
}
