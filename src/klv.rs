// Copyright (C) 2020-2021 Andy Kurnia. All rights reserved.

use super::kwg;

pub struct Klv {
    pub kwg: kwg::Kwg,
    pub counts: Box<[u32]>,
    pub scaled_leaves: Box<[i16]>,
}

pub static EMPTY_KLV_BYTES: &[u8] = b"\x01\x00\x00\x00\x00\x00\x40\x00\x00\x00\x00\x00";

impl Klv {
    pub fn from_bytes_alloc(buf: &[u8]) -> Klv {
        let mut r = 0;
        let kwg_bytes_len = (u32::from_le(
            buf[r] as u32
                | (buf[r + 1] as u32) << 8
                | (buf[r + 2] as u32) << 16
                | (buf[r + 3] as u32) << 24,
        ) as usize)
            * 4;
        r += 4;
        let kwg = kwg::Kwg::from_bytes_alloc(&buf[r..r + kwg_bytes_len]);
        r += kwg_bytes_len;
        let lv_len = u32::from_le(
            buf[r] as u32
                | (buf[r + 1] as u32) << 8
                | (buf[r + 2] as u32) << 16
                | (buf[r + 3] as u32) << 24,
        );
        r += 4;
        let mut elts = Vec::with_capacity(lv_len as usize);
        for _ in 0..lv_len {
            elts.push(i16::from_le(
                (buf[r] as u16 | (buf[r + 1] as u16) << 8) as i16,
            ));
            r += 2;
        }
        let counts = kwg.count_words_alloc();
        Klv {
            kwg,
            counts,
            scaled_leaves: elts.into_boxed_slice(),
        }
    }

    #[inline(always)]
    pub fn leave(&self, leave_idx: u32) -> f32 {
        self.scaled_leaves[leave_idx as usize] as f32 * (1.0 / 256.0)
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
