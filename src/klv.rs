// Copyright (C) 2020-2025 Andy Kurnia.

use super::kwg;

pub struct Klv {
    kwg: kwg::Kwg,
    counts: Box<[u32]>,
    leaves: Box<[f32]>,
}

pub static EMPTY_KLV_BYTES: &[u8] = b"\x01\x00\x00\x00\x00\x00\x40\x00\x00\x00\x00\x00";

impl Klv {
    pub fn from_bytes_alloc(buf: &[u8]) -> Klv {
        let mut r = 0;
        let kwg_bytes_len = ((buf[r] as u32
            | ((buf[r + 1] as u32) << 8)
            | ((buf[r + 2] as u32) << 16)
            | ((buf[r + 3] as u32) << 24)) as usize)
            * 4;
        r += 4;
        let kwg = kwg::Kwg::from_bytes_alloc(&buf[r..r + kwg_bytes_len]);
        r += kwg_bytes_len;
        let lv_len = buf[r] as u32
            | ((buf[r + 1] as u32) << 8)
            | ((buf[r + 2] as u32) << 16)
            | ((buf[r + 3] as u32) << 24);
        r += 4;
        let mut elts = Vec::with_capacity(lv_len as usize);
        if buf.len() < r + 4 * lv_len as usize {
            // klv uses i16
            for _ in 0..lv_len {
                elts.push(
                    (buf[r] as u16 | ((buf[r + 1] as u16) << 8)) as i16 as f32 * (1.0 / 256.0),
                );
                r += 2;
            }
        } else {
            // klv2 uses f32
            for _ in 0..lv_len {
                elts.push(f32::from_bits(
                    buf[r] as u32
                        | ((buf[r + 1] as u32) << 8)
                        | ((buf[r + 2] as u32) << 16)
                        | ((buf[r + 3] as u32) << 24),
                ));
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
    pub fn leave(&self, leave_idx: u32) -> f32 {
        self.leaves[leave_idx as usize]
    }

    #[inline(always)]
    pub fn count(&self, i: i32) -> u32 {
        self.counts[i as usize]
    }

    #[inline(always)]
    pub fn kwg(&self, i: i32) -> kwg::Node {
        self.kwg[i]
    }

    #[inline(always)]
    pub fn leave_value_from_tally(&self, rack_tally: &[u8]) -> f32 {
        let leave_idx = self.kwg.get_word_index_of(
            &self.counts,
            self.kwg[0].arc_index(),
            &mut (0u8..)
                .zip(rack_tally)
                .flat_map(|(tile, &count)| std::iter::repeat_n(tile, count as usize)),
        );
        if leave_idx == !0 {
            0.0
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
    leave_values: Vec<f32>, // typically 2**7, can be shorter when duplicates
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

    // use_klv=false means to just use 0.0 for all leaves, this may be slightly faster.
    pub fn init<AdjustLeaveValue: Fn(f32) -> f32>(
        &mut self,
        rack_tally: &[u8],
        klv: &Klv,
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
        for (tile, &count) in (0u8..).zip(rack_tally.iter()) {
            if count != 0 {
                self.unique_tiles.push(tile);
                self.digits[tile as usize] = MultiLeavesDigit { count, place_value };
                place_value *= count as u32 + 1;
                num_tiles_on_rack += count;
            }
        }
        self.leave_values.clear();
        self.leave_values.resize(place_value as usize, 0.0);
        self.num_playeds.clear();
        self.num_playeds.resize(place_value as usize, 0xff); // all entries will be overwritten anyway
        self.num_playeds[0] = num_tiles_on_rack;

        struct Env<'a> {
            klv: &'a Klv,
            unique_tiles: &'a [u8],
            digits: &'a mut [MultiLeavesDigit],
            leave_values: &'a mut [f32],
            num_playeds: &'a mut [u8],
        }
        fn precompute_leaves(
            env: &mut Env<'_>,
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
                            0.0
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
                        env.leave_values[leave_idx as usize] = 0.0;
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
            // note: adjust_leave_value(f) must return between 0.0 and f
            self.leave_values
                .iter_mut()
                .for_each(|m| *m = adjust_leave_value(*m));
        }
    }

    pub fn init_endgame_leaves<AlphabetScore: Fn(u8) -> i8>(
        &mut self,
        alphabet_score: AlphabetScore,
        play_out_bonus: f32,
    ) {
        // leave value for not going out is -10 - 2 * residual tiles.
        self.leave_values[0] = -10.0;
        for &tile in &self.unique_tiles {
            let penalty = -2.0 * alphabet_score(tile) as f32;
            let &MultiLeavesDigit { count, place_value } = &self.digits[tile as usize];
            for i in 0..place_value * count as u32 {
                self.leave_values[(i + place_value) as usize] =
                    self.leave_values[i as usize] + penalty;
            }
        }
        // leave value for keeping 0 tiles is play_out_bonus.
        self.leave_values[0] = play_out_bonus;
    }

    // undefined behavior unless rack_tally is a subset of what was init'ed.
    #[inline(always)]
    pub fn leave_value_from_tally(&self, rack_tally: &[u8]) -> f32 {
        let mut leave_idx = 0u32;
        for &tile in &self.unique_tiles {
            leave_idx += rack_tally[tile as usize] as u32 * self.digits[tile as usize].place_value;
        }
        self.leave_values[leave_idx as usize]
    }

    #[inline(always)]
    pub fn extract_raw_best_leave_values(&self, best_leave_values: &mut Vec<f32>) {
        best_leave_values.clear();
        best_leave_values.resize(self.num_playeds[0] as usize + 1, f32::NEG_INFINITY);
        for i in 0..self.leave_values.len() {
            let this_leave_value = self.leave_values[i];
            let num_tiles_exchanged = self.num_playeds[i] as usize;
            if this_leave_value > best_leave_values[num_tiles_exchanged] {
                best_leave_values[num_tiles_exchanged] = this_leave_value;
            }
        }
    }

    #[inline(always)]
    pub fn kurnia_gen_exchange_moves_unconditionally<'a, FoundExchangeMove: FnMut(&[u8], f32)>(
        &self,
        found_exchange_move: FoundExchangeMove,
        rack_tally: &'a mut [u8],
        exchange_buffer: &'a mut Vec<u8>,
        max_vec_len: usize,
    ) {
        exchange_buffer.clear(); // should be no-op
        struct ExchangeEnv<'a, FoundExchangeMove: FnMut(&[u8], f32)> {
            multi_leaves: &'a MultiLeaves,
            found_exchange_move: FoundExchangeMove,
            rack_tally: &'a mut [u8],
            exchange_buffer: &'a mut Vec<u8>,
            max_vec_len: usize,
        }
        fn generate_exchanges<FoundExchangeMove: FnMut(&[u8], f32)>(
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
    pub fn pass_leave_idx(&self) -> u32 {
        self.leave_values.len() as u32 - 1
    }

    // undefined behavior unless tile was init'ed.
    #[inline(always)]
    pub fn place_value(&self, tile: u8) -> u32 {
        self.digits[tile as usize].place_value
    }

    // undefined behavior unless idx is valid.
    #[inline(always)]
    pub fn leave_value(&self, idx: u32) -> f32 {
        self.leave_values[idx as usize]
    }

    #[inline(always)]
    pub fn pass_leave_value(&self) -> f32 {
        *self.leave_values.last().unwrap()
    }
}
