// Copyright (C) 2020-2023 Andy Kurnia.

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
            | (buf[r + 1] as u32) << 8
            | (buf[r + 2] as u32) << 16
            | (buf[r + 3] as u32) << 24) as usize)
            * 4;
        r += 4;
        let kwg = kwg::Kwg::from_bytes_alloc(&buf[r..r + kwg_bytes_len]);
        r += kwg_bytes_len;
        let lv_len = buf[r] as u32
            | (buf[r + 1] as u32) << 8
            | (buf[r + 2] as u32) << 16
            | (buf[r + 3] as u32) << 24;
        r += 4;
        let mut elts = Vec::with_capacity(lv_len as usize);
        if buf.len() < r + 4 * lv_len as usize {
            // klv uses i16
            for _ in 0..lv_len {
                elts.push((buf[r] as u16 | (buf[r + 1] as u16) << 8) as i16 as f32 * (1.0 / 256.0));
                r += 2;
            }
        } else {
            // klv2 uses f32
            for _ in 0..lv_len {
                elts.push(f32::from_bits(
                    buf[r] as u32
                        | (buf[r + 1] as u32) << 8
                        | (buf[r + 2] as u32) << 16
                        | (buf[r + 3] as u32) << 24,
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
                .flat_map(|(tile, &count)| std::iter::repeat(tile).take(count as usize)),
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
    tile: u8,
    place_value: u32,
}

// example: EEHJTTU should become
// [(5, 1), (8, 3), (10, 6), (20, 12), (21, 36)]
// leave_values.len() == 72
// leave_values[num(E) * 1 + num(H) * 3 + num(J) * 6 + ...]
#[derive(Default)]
pub struct MultiLeaves {
    digits: Vec<MultiLeavesDigit>, // len() == rack_bits.count_ones()
    leave_values: Vec<f32>,        // typically 2**7, can be shorter when duplicates
    num_playeds: Vec<u8>,          // same length as leave_values
}

impl Clone for MultiLeaves {
    #[inline(always)]
    fn clone(&self) -> Self {
        Self {
            digits: self.digits.clone(),
            leave_values: self.leave_values.clone(),
            num_playeds: self.num_playeds.clone(),
        }
    }

    #[inline(always)]
    fn clone_from(&mut self, source: &Self) {
        self.digits.clone_from(&source.digits);
        self.leave_values.clone_from(&source.leave_values);
        self.num_playeds.clone_from(&source.num_playeds);
    }
}

impl MultiLeaves {
    pub fn new() -> Self {
        Self {
            digits: Vec::new(),
            leave_values: Vec::new(),
            num_playeds: Vec::new(),
        }
    }

    pub fn init<AdjustLeaveValue: Fn(f32) -> f32>(
        &mut self,
        rack_tally: &mut [u8],
        klv: &Klv,
        adjust_leave_value: &AdjustLeaveValue,
    ) {
        self.digits.clear();
        let mut place_value = 1u32;
        let mut num_tiles_on_rack = 0u8;
        for (tile, &count) in (0u8..).zip(rack_tally.iter()) {
            if count != 0 {
                self.digits.push(MultiLeavesDigit { tile, place_value });
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
            rack_tally: &'a mut [u8],
            digits: &'a [MultiLeavesDigit],
            leave_values: &'a mut [f32],
            num_playeds: &'a mut [u8],
        }
        fn precompute_leaves(
            env: &mut Env<'_>,
            mut p: i32,
            mut idx: u32,
            leave_idx_offset: u32,
            digit_idx_offset: u8,
            mut num_played: u8,
        ) {
            num_played = num_played.wrapping_sub(1);
            for digit_idx in digit_idx_offset as usize..env.digits.len() {
                let &MultiLeavesDigit { tile, place_value } = &env.digits[digit_idx];
                let tile_usize = tile as usize;
                if env.rack_tally[tile_usize] > 0 {
                    env.rack_tally[tile_usize] -= 1;
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
                    let leave_idx = leave_idx_offset + place_value;
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
                            digit_idx as u8,
                            num_played,
                        );
                    } else {
                        env.leave_values[leave_idx as usize] = 0.0;
                        precompute_leaves(env, p, idx, leave_idx, digit_idx as u8, num_played);
                    }
                    env.rack_tally[tile_usize] += 1;
                }
            }
        }
        precompute_leaves(
            &mut Env {
                klv,
                rack_tally,
                digits: &self.digits,
                leave_values: &mut self.leave_values,
                num_playeds: &mut self.num_playeds,
            },
            klv.kwg[0].arc_index(),
            0,
            0,
            0,
            num_tiles_on_rack,
        );

        // note: adjust_leave_value(f) must return between 0.0 and f
        self.leave_values
            .iter_mut()
            .for_each(|m| *m = adjust_leave_value(*m));
    }

    // undefined behavior unless rack_tally is a subset of what was init'ed.
    #[inline(always)]
    pub fn leave_value_from_tally(&self, rack_tally: &[u8]) -> f32 {
        let mut leave_idx = 0u32;
        for &MultiLeavesDigit { tile, place_value } in &self.digits {
            leave_idx += rack_tally[tile as usize] as u32 * place_value;
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
}
