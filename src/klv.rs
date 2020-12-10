use super::kwg;

// kwg is double-boxed :-(
pub struct Klv {
    pub kwg: Box<kwg::Kwg>,
    pub counts: Box<[u32]>,
    pub leaves: Box<[f32]>,
}

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
            elts.push(f32::from_bits(u32::from_le(
                buf[r] as u32
                    | (buf[r + 1] as u32) << 8
                    | (buf[r + 2] as u32) << 16
                    | (buf[r + 3] as u32) << 24,
            )));
            r += 4;
        }
        let counts = kwg.count_words_alloc();
        Klv {
            kwg: Box::new(kwg),
            counts,
            leaves: elts.into_boxed_slice(),
        }
    }

    #[inline(always)]
    pub fn leave_value_from_tally(&self, rack_tally: &[u8]) -> f32 {
        let leave_idx = self.kwg.get_word_index_of(
            &self.counts,
            self.kwg[0].arc_index(),
            &mut rack_tally
                .iter()
                .enumerate()
                .flat_map(|(tile, &count)| std::iter::repeat(tile as u8).take(count as usize)),
        );
        if leave_idx == !0 {
            0.0
        } else {
            self.leaves[leave_idx as usize]
        }
    }
}
