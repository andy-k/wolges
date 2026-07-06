// Copyright (C) 2020-2026 Andy Kurnia.

use super::{census, equity, kwg};

// Stack scratch width for a decoded subrack tally. MultisetLattice caps
// num_letters at this same bound, so every alphabet fits.
const MAX_LETTERS: usize = 64;

/// The board-independent inputs the dynamic-leave pull needs, bundled so a move
/// generator can thread one optional handle through instead of four arguments.
/// `lat`/`add` are the multiset lattice and its add-table, `full_v` is the static
/// v-table (one leave value per lattice index, from a full-length klv), and
/// `min_keep` is the smallest kept-subrack size that gets reweighted.
#[derive(Clone, Copy)]
pub struct DynamicLeavesRef<'a> {
    pub lat: &'a census::MultisetLattice,
    pub add: &'a census::AddTable,
    pub full_v: &'a [i32],
    pub min_keep: usize,
}

pub struct Klv<L: kwg::Node> {
    kwg: kwg::Kwg<L>,
    counts: Box<[u32]>,
    leaves: Box<[i32]>, // millipoints, converted from f32 at load time
}

// kwg::Node22
pub static EMPTY_KLV_BYTES: &[u8] = b"\x01\x00\x00\x00\x00\x00\x40\x00\x00\x00\x00\x00";

impl<L: kwg::Node> Klv<L> {
    pub fn from_bytes_alloc(buf: &[u8]) -> Self {
        let mut r = 0;
        let kwg_bytes_len = (kwg::read_le_u32(buf, r) as usize) * 4;
        r += 4;
        let kwg = kwg::Kwg::from_bytes_alloc(&buf[r..r + kwg_bytes_len]);
        r += kwg_bytes_len;
        let lv_len = kwg::read_le_u32(buf, r);
        r += 4;
        let mut elts = Vec::with_capacity(lv_len as usize);
        if buf.len() < r + 4 * lv_len as usize {
            // klv uses i16 (fixed-point, 1/256 scale)
            for _ in 0..lv_len {
                elts.push((kwg::read_le_u16(buf, r) as i16 as i32 * equity::SCALE + 128) / 256);
                r += 2;
            }
        } else {
            // klv2 uses f32
            for _ in 0..lv_len {
                let raw = f32::from_bits(kwg::read_le_u32(buf, r));
                elts.push((raw * equity::SCALE as f32).round() as i32);
                r += 4;
            }
        }
        let counts = kwg.count_words_alloc();
        Klv {
            kwg,
            counts,
            leaves: elts.into_boxed_slice(),
        }
    }

    #[inline(always)]
    pub fn leave(&self, leave_idx: u32) -> i32 {
        self.leaves[leave_idx as usize]
    }

    #[inline(always)]
    pub fn count(&self, i: i32) -> u32 {
        self.counts[i as usize]
    }

    #[inline(always)]
    pub fn kwg(&self, i: i32) -> L {
        self.kwg[i]
    }

    #[inline(always)]
    pub fn leave_value_from_tally(&self, rack_tally: &[u8]) -> i32 {
        let leave_idx = self.kwg.get_word_index_of(
            &self.counts,
            self.kwg[0].arc_index(),
            &mut (0u8..)
                .zip(rack_tally)
                .flat_map(|(tile, &count)| std::iter::repeat_n(tile, count as usize)),
        );
        if leave_idx == !0 {
            0
        } else {
            self.leave(leave_idx)
        }
    }
}

#[derive(Clone)]
struct MultiLeavesDigit {
    count: u8,
    place_value: u32,
}

// example: EEHJTTU should make digits.len() == 27,
// [5]=(2,1), [8]=(1,3), [10]=(1,6), [20]=(2,12), [21]=(1,36),
// and the other elements are (0,0).
// unique_tiles == [5, 8, 10, 20, 21].
// leave_values.len() == 72.
// leave_values[num(E) * 1 + num(H) * 3 + num(J) * 6 + ...].
// num_playeds = [7, ..., 0] (number of tiles played, not kept)
// leave_values = [[], [E], [EE], [H], [EH], [EEH], ...]
// = [leave for exch 7, leave for exch 6 keep E, ... leave for pass].
#[derive(Default)]
pub struct MultiLeaves {
    unique_tiles: Vec<u8>, // sorted, unique, len() == rack_bits.count_ones()
    digits: Vec<MultiLeavesDigit>, // len() == rack_tally.len() == alphabet.len()
    leave_values: Vec<i32>, // typically 2**7, can be shorter when duplicates
    num_playeds: Vec<u8>,  // same length as leave_values
}

impl Clone for MultiLeaves {
    #[inline(always)]
    fn clone(&self) -> Self {
        Self {
            unique_tiles: self.unique_tiles.clone(),
            digits: self.digits.clone(),
            leave_values: self.leave_values.clone(),
            num_playeds: self.num_playeds.clone(),
        }
    }

    #[inline(always)]
    fn clone_from(&mut self, source: &Self) {
        self.unique_tiles.clone_from(&source.unique_tiles);
        self.digits.clone_from(&source.digits);
        self.leave_values.clone_from(&source.leave_values);
        self.num_playeds.clone_from(&source.num_playeds);
    }
}

impl MultiLeaves {
    pub fn new() -> Self {
        Self {
            unique_tiles: Vec::new(),
            digits: Vec::new(),
            leave_values: Vec::new(),
            num_playeds: Vec::new(),
        }
    }

    // use_klv=false means to just use 0 for all leaves, this may be slightly faster.
    pub fn init<AdjustLeaveValue: Fn(i32) -> i32, L: kwg::Node>(
        &mut self,
        rack_tally: &[u8],
        klv: &Klv<L>,
        use_klv: bool,
        adjust_leave_value: &AdjustLeaveValue,
    ) {
        self.unique_tiles.clear();
        self.digits.clear();
        self.digits.resize(
            rack_tally.len(),
            MultiLeavesDigit {
                count: 0,
                place_value: 0,
            },
        );
        let mut place_value = 1u32;
        let mut num_tiles_on_rack = 0u8;
        let mut dense = true;
        for (tile, &count) in (0u8..).zip(rack_tally.iter()) {
            if count != 0 {
                self.unique_tiles.push(tile);
                if dense {
                    self.digits[tile as usize] = MultiLeavesDigit { count, place_value };
                    match place_value.checked_mul(count as u32 + 1) {
                        Some(v) if v <= (1 << 20) => {
                            place_value = v;
                        }
                        _ => {
                            dense = false;
                        }
                    }
                }
                num_tiles_on_rack += count;
            }
        }
        self.leave_values.clear();
        self.num_playeds.clear();
        if dense {
            self.leave_values.resize(place_value as usize, 0);
            self.num_playeds.resize(place_value as usize, 0xff);
            self.num_playeds[0] = num_tiles_on_rack;
            struct Env<'a, L: kwg::Node> {
                klv: &'a Klv<L>,
                unique_tiles: &'a [u8],
                digits: &'a mut [MultiLeavesDigit],
                leave_values: &'a mut [i32],
                num_playeds: &'a mut [u8],
            }
            fn precompute_leaves<L: kwg::Node>(
                env: &mut Env<'_, L>,
                mut p: i32,
                mut idx: u32,
                leave_idx_offset: u32,
                unique_tile_idx_offset: u8,
                mut num_played: u8,
            ) {
                num_played = num_played.wrapping_sub(1);
                for unique_tile_idx in unique_tile_idx_offset as usize..env.unique_tiles.len() {
                    let tile = env.unique_tiles[unique_tile_idx];
                    let tile_usize = tile as usize;
                    if env.digits[tile_usize].count > 0 {
                        env.digits[tile_usize].count -= 1;
                        if p != 0 {
                            idx += env.klv.count(p);
                            loop {
                                let node = env.klv.kwg(p);
                                if node.tile() >= tile {
                                    break;
                                }
                                if node.is_end() {
                                    p = 0;
                                    break;
                                }
                                p += 1;
                            }
                        }
                        let leave_idx = leave_idx_offset + env.digits[tile_usize].place_value;
                        env.num_playeds[leave_idx as usize] = num_played;
                        if p != 0 {
                            idx -= env.klv.count(p);
                            let node = env.klv.kwg(p);
                            let leave_val = if node.tile() == tile && node.accepts() {
                                env.klv.leave(idx)
                            } else {
                                0
                            };
                            env.leave_values[leave_idx as usize] = leave_val;
                            precompute_leaves(
                                env,
                                node.arc_index(),
                                idx + node.accepts() as u32,
                                leave_idx,
                                unique_tile_idx as u8,
                                num_played,
                            );
                        } else {
                            env.leave_values[leave_idx as usize] = 0;
                            precompute_leaves(
                                env,
                                p,
                                idx,
                                leave_idx,
                                unique_tile_idx as u8,
                                num_played,
                            );
                        }
                        env.digits[tile_usize].count += 1;
                    }
                }
            }
            precompute_leaves(
                &mut Env {
                    klv,
                    unique_tiles: &self.unique_tiles,
                    digits: &mut self.digits,
                    leave_values: &mut self.leave_values,
                    num_playeds: &mut self.num_playeds,
                },
                if use_klv { klv.kwg[0].arc_index() } else { 0 },
                0,
                0,
                0,
                num_tiles_on_rack,
            );

            if use_klv {
                // note: adjust_leave_value(v) must return between 0 and v
                self.leave_values
                    .iter_mut()
                    .for_each(|m| *m = adjust_leave_value(*m));
            }
        }
    }

    pub fn init_endgame_leaves<AlphabetScore: Fn(u8) -> i8>(
        &mut self,
        alphabet_score: AlphabetScore,
        play_out_bonus: i32,
    ) {
        // leave value for not going out is -10 - 2 * (total score of
        // residual tiles) (in millipoints).
        self.leave_values[0] = -equity::ENDGAME_PENALTY_BASE;
        for &tile in &self.unique_tiles {
            let penalty = -2 * alphabet_score(tile) as i32 * equity::SCALE;
            let &MultiLeavesDigit { count, place_value } = &self.digits[tile as usize];
            for i in 0..place_value * count as u32 {
                self.leave_values[(i + place_value) as usize] =
                    self.leave_values[i as usize] + penalty;
            }
        }
        // leave value for keeping 0 tiles is play_out_bonus (already in millipoints).
        self.leave_values[0] = play_out_bonus;
    }

    // Compute best_leave_values by traversing the KLV's KWG, constrained by
    // available tiles. Used when the dense array is too large to build.
    // Traverses KLV entries (bounded by KLV size) rather than rack subsets.
    pub fn extract_best_leave_values_from_klv<AdjustLeaveValue: Fn(i32) -> i32, L: kwg::Node>(
        rack_tally: &mut [u8],
        klv: &Klv<L>,
        num_tiles_on_rack: u8,
        adjust_leave_value: &AdjustLeaveValue,
        best_leave_values: &mut Vec<i32>,
    ) {
        best_leave_values.clear();
        best_leave_values.resize(num_tiles_on_rack as usize + 1, i32::MIN);
        struct Env<'a, AdjustLeaveValue, L: kwg::Node> {
            klv: &'a Klv<L>,
            rack_tally: &'a mut [u8],
            kept_tally: &'a mut [u8],
            best_leave_values: &'a mut [i32],
            num_tiles_on_rack: u8,
            num_kept: u8,
            adjust_leave_value: &'a AdjustLeaveValue,
        }
        // Traverse the KLV's KWG children. At each node, if the tile is
        // available in rack_tally, consume it and recurse. At accepting nodes,
        // look up the leave value for the kept tiles.
        fn traverse<AdjustLeaveValue: Fn(i32) -> i32, L: kwg::Node>(
            env: &mut Env<'_, AdjustLeaveValue, L>,
            mut p: i32,
        ) {
            if p <= 0 {
                return;
            }
            loop {
                let node = env.klv.kwg(p);
                let tile = node.tile();
                if (tile as usize) < env.rack_tally.len() && env.rack_tally[tile as usize] > 0 {
                    env.rack_tally[tile as usize] -= 1;
                    env.kept_tally[tile as usize] += 1;
                    env.num_kept += 1;
                    if node.accepts() {
                        let leave_val = (env.adjust_leave_value)(
                            env.klv.leave_value_from_tally(env.kept_tally),
                        );
                        let num_played = (env.num_tiles_on_rack - env.num_kept) as usize;
                        if num_played < env.best_leave_values.len()
                            && leave_val > env.best_leave_values[num_played]
                        {
                            env.best_leave_values[num_played] = leave_val;
                        }
                    }
                    traverse(env, node.arc_index());
                    env.num_kept -= 1;
                    env.kept_tally[tile as usize] -= 1;
                    env.rack_tally[tile as usize] += 1;
                }
                if node.is_end() {
                    break;
                }
                p += 1;
            }
        }
        let tally_len = rack_tally.len();
        let mut kept_tally = vec![0u8; tally_len];
        traverse(
            &mut Env {
                klv,
                rack_tally,
                kept_tally: &mut kept_tally,
                best_leave_values,
                num_tiles_on_rack,
                num_kept: 0,
                adjust_leave_value,
            },
            klv.kwg[0].arc_index(),
        );
        // Leaves not found in KLV have value 0.
        for v in best_leave_values.iter_mut() {
            if *v == i32::MIN {
                *v = 0;
            }
        }
    }

    #[inline(always)]
    pub fn extract_raw_best_leave_values(&self, best_leave_values: &mut Vec<i32>) {
        best_leave_values.clear();
        best_leave_values.resize(self.num_playeds[0] as usize + 1, i32::MIN);
        for i in 0..self.leave_values.len() {
            let this_leave_value = self.leave_values[i];
            let num_tiles_exchanged = self.num_playeds[i] as usize;
            if this_leave_value > best_leave_values[num_tiles_exchanged] {
                best_leave_values[num_tiles_exchanged] = this_leave_value;
            }
        }
    }

    // Reweight the dense leave table in place by the tiles still live this move.
    // Each dense slot holds the static value of keeping some subrack S of the rack;
    // this replaces it with the dynamic value = the expected static full-rack value
    // once S is refilled by drawing rack_size - |S| tiles from `live_pool` (the
    // pool the mover can still draw: bag + opponent, already excluding this rack).
    // So the same kept tiles are valued against the actual remaining pool rather
    // than an average bag. Every downstream read (place, exchange, pass, and the
    // shadow-play bound rebuilt right after) then sees dynamic values, since they
    // all index this same table.
    //
    // Subracks smaller than `min_keep` keep their static value: they are the
    // least leave-sensitive keeps (playing 5-7 tiles) and skipping them cuts the
    // dominant draw cost of the tiny keeps. A subrack whose completion is
    // undrawable (dynamic_leave_value returns UNPLAYABLE, den == 0) also keeps its
    // static value. No-op when the dense table was not built.
    pub fn apply_dynamic_leaves(
        &mut self,
        lat: &census::MultisetLattice,
        add: &census::AddTable,
        full_v: &[i32],
        live_pool: &[u8],
        min_keep: usize,
    ) {
        if self.leave_values.is_empty() {
            return;
        }
        let num_letters = lat.num_letters();
        let rack_size = lat.rack_size();
        let pool_size: usize = live_pool[..num_letters].iter().map(|&c| c as usize).sum();
        // Non-rack positions stay zero for the whole scan; each rack tile's slot is
        // rewritten every iteration (possibly to zero), so no per-index reset.
        let mut s_tally = [0u8; MAX_LETTERS];
        for idx in 0..self.leave_values.len() {
            // Decode idx (mixed radix over the rack's distinct tiles) into the kept
            // subrack S and its size.
            let mut s_size = 0usize;
            for &tile in &self.unique_tiles {
                let digit = &self.digits[tile as usize];
                let kept = (idx as u32 / digit.place_value) % (digit.count as u32 + 1);
                s_tally[tile as usize] = kept as u8;
                s_size += kept as usize;
            }
            if s_size < min_keep {
                continue;
            }
            let s_ridx = lat.rank(&s_tally[..num_letters]);
            if s_ridx == !0 {
                continue;
            }
            let draw = (rack_size - s_size).min(pool_size);
            let dynamic =
                census::dynamic_leave_value(lat, add, full_v, live_pool, s_ridx as usize, draw);
            if dynamic != census::UNPLAYABLE {
                self.leave_values[idx] = dynamic;
            }
        }
    }

    #[inline(always)]
    pub fn kurnia_gen_exchange_moves_unconditionally<'a, FoundExchangeMove: FnMut(&[u8], i32)>(
        &self,
        found_exchange_move: FoundExchangeMove,
        rack_tally: &'a mut [u8],
        exchange_buffer: &'a mut Vec<u8>,
        max_vec_len: usize,
    ) {
        exchange_buffer.clear(); // should be no-op
        struct ExchangeEnv<'a, FoundExchangeMove: FnMut(&[u8], i32)> {
            multi_leaves: &'a MultiLeaves,
            found_exchange_move: FoundExchangeMove,
            rack_tally: &'a mut [u8],
            exchange_buffer: &'a mut Vec<u8>,
            max_vec_len: usize,
        }
        fn generate_exchanges<FoundExchangeMove: FnMut(&[u8], i32)>(
            env: &mut ExchangeEnv<'_, FoundExchangeMove>,
            unique_tile_idx_offset: u8,
            leave_idx: u32,
        ) {
            if !env.exchange_buffer.is_empty() {
                (env.found_exchange_move)(
                    env.exchange_buffer,
                    env.multi_leaves.leave_values[leave_idx as usize],
                );
            }
            if env.exchange_buffer.len() < env.max_vec_len {
                for unique_tile_idx in
                    unique_tile_idx_offset as usize..env.multi_leaves.unique_tiles.len()
                {
                    let tile = env.multi_leaves.unique_tiles[unique_tile_idx];
                    let tile_usize = tile as usize;
                    if env.rack_tally[tile_usize] > 0 {
                        env.rack_tally[tile_usize] -= 1;
                        env.exchange_buffer.push(tile);
                        generate_exchanges(
                            env,
                            unique_tile_idx as u8,
                            leave_idx - env.multi_leaves.digits[tile_usize].place_value,
                        );
                        env.exchange_buffer.pop();
                        env.rack_tally[tile_usize] += 1;
                    }
                }
            }
        }
        generate_exchanges(
            &mut ExchangeEnv {
                multi_leaves: self,
                found_exchange_move,
                rack_tally,
                exchange_buffer,
                max_vec_len,
            },
            0,
            self.pass_leave_idx(),
        );
    }

    #[inline(always)]
    pub fn is_dense(&self) -> bool {
        !self.leave_values.is_empty()
    }

    // Exchange generator that computes leave values on-the-fly via KLV.
    // Used when the dense leave table is not available.
    pub fn gen_exchange_moves_via_klv<'a, FoundExchangeMove: FnMut(&[u8], i32), L: kwg::Node>(
        klv: &Klv<L>,
        found_exchange_move: FoundExchangeMove,
        rack_tally: &'a mut [u8],
        exchange_buffer: &'a mut Vec<u8>,
        max_vec_len: usize,
    ) {
        exchange_buffer.clear();
        struct ExchangeEnv<'a, FoundExchangeMove: FnMut(&[u8], i32), L: kwg::Node> {
            rack_tally_len: u8,
            klv: &'a Klv<L>,
            found_exchange_move: FoundExchangeMove,
            rack_tally: &'a mut [u8],
            exchange_buffer: &'a mut Vec<u8>,
            max_vec_len: usize,
        }
        fn generate_exchanges<FoundExchangeMove: FnMut(&[u8], i32), L: kwg::Node>(
            env: &mut ExchangeEnv<'_, FoundExchangeMove, L>,
            tile_offset: u8,
        ) {
            if !env.exchange_buffer.is_empty() {
                let leave_value = env.klv.leave_value_from_tally(env.rack_tally);
                (env.found_exchange_move)(env.exchange_buffer, leave_value);
            }
            if env.exchange_buffer.len() < env.max_vec_len {
                for tile in tile_offset..env.rack_tally_len {
                    let tile_usize = tile as usize;
                    if env.rack_tally[tile_usize] > 0 {
                        env.rack_tally[tile_usize] -= 1;
                        env.exchange_buffer.push(tile);
                        generate_exchanges(env, tile);
                        env.exchange_buffer.pop();
                        env.rack_tally[tile_usize] += 1;
                    }
                }
            }
        }
        generate_exchanges(
            &mut ExchangeEnv {
                rack_tally_len: rack_tally.len() as u8,
                klv,
                found_exchange_move,
                rack_tally,
                exchange_buffer,
                max_vec_len,
            },
            0,
        );
    }

    #[inline(always)]
    pub fn pass_leave_idx(&self) -> u32 {
        if self.leave_values.is_empty() {
            0
        } else {
            self.leave_values.len() as u32 - 1
        }
    }

    // undefined behavior unless tile was init'ed.
    #[inline(always)]
    pub fn place_value(&self, tile: u8) -> u32 {
        self.digits[tile as usize].place_value
    }

    // undefined behavior unless idx is valid. Returns 0 when not dense.
    #[inline(always)]
    pub fn leave_value(&self, idx: u32) -> i32 {
        if self.leave_values.is_empty() {
            0
        } else {
            self.leave_values[idx as usize]
        }
    }

    #[inline(always)]
    pub fn pass_leave_value(&self) -> i32 {
        if self.leave_values.is_empty() {
            0 // fallback; caller should use klv.leave_value_from_tally
        } else {
            *self.leave_values.last().unwrap()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn binom(n: u64, k: u64) -> u64 {
        if k > n {
            return 0;
        }
        let k = k.min(n - k);
        let mut num = 1u128;
        let mut den = 1u128;
        for i in 0..k {
            num *= (n - i) as u128;
            den *= (i + 1) as u128;
        }
        (num / den) as u64
    }

    #[test]
    fn apply_dynamic_leaves_matches_brute() {
        // Hand-build the dense table for rack AAB over a 3-letter alphabet with
        // rack_size 3, so the mixed-radix decode and the pull can both be checked
        // against a from-scratch draw average. digits: A(count 2, place 1),
        // B(count 1, place 3); the 6 dense slots enumerate keep {}, A, AA, B, AB,
        // AAB via idx = keptA + 3*keptB.
        let num_letters = 3usize;
        let rack_size = 3usize;
        let lat = census::MultisetLattice::new(num_letters, rack_size);
        let add = census::AddTable::new(&lat);
        // Static v-table: value every lattice index (the pull ranks full racks S+d).
        let mut full_v = vec![0i32; lat.len()];
        for (idx, slot) in full_v.iter_mut().enumerate() {
            let h = (idx as i32).wrapping_mul(2654435761u32 as i32);
            *slot = h.rem_euclid(20_000) - 5_000;
        }
        // Recognizable static leaves so kept-static slots are detectable.
        let statics: Vec<i32> = (0..6i32).map(|i| -100 - i).collect();
        let mut ml = MultiLeaves {
            unique_tiles: vec![0u8, 1u8],
            digits: vec![
                MultiLeavesDigit {
                    count: 2,
                    place_value: 1,
                },
                MultiLeavesDigit {
                    count: 1,
                    place_value: 3,
                },
                MultiLeavesDigit {
                    count: 0,
                    place_value: 0,
                },
            ],
            leave_values: statics.clone(),
            num_playeds: vec![3, 2, 1, 2, 1, 0],
        };

        let live_pool = [2u8, 2u8, 1u8]; // A, B, C still drawable
        let min_keep = 1usize;
        ml.apply_dynamic_leaves(&lat, &add, &full_v, &live_pool, min_keep);

        // Brute force each idx's expected value independently.
        for (idx, &static_v) in statics.iter().enumerate() {
            let kept_a = (idx as u32 % 3) as u8;
            let kept_b = (idx as u32 / 3) as u8;
            let s_size = (kept_a + kept_b) as usize;
            if s_size < min_keep {
                // keep {} stays static under min_keep 1.
                assert_eq!(
                    ml.leave_values[idx], static_v,
                    "idx {idx} should stay static"
                );
                continue;
            }
            let draw = rack_size - s_size;
            // Average full_v[rank(S + d)] over completions d of size `draw` drawn
            // from live_pool, weighted by the exact draw ways prod C(pool[t], d[t]).
            let mut num = 0f64;
            let mut den = 0f64;
            for da in 0..=live_pool[0] {
                for db in 0..=live_pool[1] {
                    for dc in 0..=live_pool[2] {
                        if (da + db + dc) as usize != draw {
                            continue;
                        }
                        let ways = binom(live_pool[0] as u64, da as u64)
                            * binom(live_pool[1] as u64, db as u64)
                            * binom(live_pool[2] as u64, dc as u64);
                        if ways == 0 {
                            continue;
                        }
                        let r = [kept_a + da, kept_b + db, dc];
                        let ri = lat.rank(&r) as usize;
                        num += ways as f64 * full_v[ri] as f64;
                        den += ways as f64;
                    }
                }
            }
            let want = (num / den) as i32;
            assert_eq!(
                ml.leave_values[idx], want,
                "idx {idx} (keep {kept_a}A {kept_b}B)"
            );
        }

        // den == 0 contract that apply relies on for its keep-static fallback: an
        // infeasible draw (more completion tiles than the pool holds) is UNPLAYABLE.
        let empty_ridx = lat.rank(&[0u8, 0, 0]) as usize;
        assert_eq!(
            census::dynamic_leave_value(&lat, &add, &full_v, &[0u8, 0, 0], empty_ridx, 3),
            census::UNPLAYABLE,
        );
    }
}
