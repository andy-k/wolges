pub struct Node(u32);

impl Node {
    #[inline(always)]
    pub fn tile(&self) -> u8 {
        (self.0 >> 24) as u8
    }

    #[inline(always)]
    pub fn accepts(&self) -> bool {
        self.0 & 0x800000 != 0
    }

    #[inline(always)]
    pub fn is_end(&self) -> bool {
        self.0 & 0x400000 != 0
    }

    #[inline(always)]
    pub fn arc_index(&self) -> i32 {
        (self.0 & 0x3fffff) as i32
    }
}

pub struct Kwg(pub Box<[Node]>);

impl std::ops::Index<i32> for Kwg {
    type Output = Node;

    #[inline(always)]
    fn index(&self, i: i32) -> &Node {
        &self.0[i as usize]
    }
}

impl Kwg {
    pub fn from_bytes_alloc(buf: &[u8]) -> Kwg {
        let kwg_len = buf.len() / 4;
        let mut elts = Vec::with_capacity(kwg_len);
        let mut r = 0;
        for _ in 0..kwg_len {
            elts.push(Node(u32::from_le(
                buf[r] as u32
                    | (buf[r + 1] as u32) << 8
                    | (buf[r + 2] as u32) << 16
                    | (buf[r + 3] as u32) << 24,
            )));
            r += 4;
        }
        Kwg(elts.into_boxed_slice())
    }

    #[inline(always)]
    pub fn seek(&self, mut p: i32, tile: u8) -> i32 {
        if p >= 0 {
            p = self[p].arc_index() as i32;
            if p > 0 {
                while self[p].tile() != tile {
                    if self[p].is_end() {
                        return -1;
                    }
                    p += 1;
                }
                return p;
            }
        }
        -1 // intentionally return 0 as -1
    }

    fn count_words_at(&self, mut word_counts: &mut [u32], p: i32) -> u32 {
        if p as usize >= word_counts.len() {
            return 0;
        }
        if word_counts[p as usize] == !0 {
            panic!()
        };
        if word_counts[p as usize] == 0 {
            word_counts[p as usize] = !0; // marker
            word_counts[p as usize] = self[p].accepts() as u32
                + if self[p].arc_index() != 0 {
                    self.count_words_at(&mut word_counts, self[p].arc_index())
                } else {
                    0
                }
                + if self[p].is_end() {
                    0
                } else {
                    self.count_words_at(&mut word_counts, p + 1)
                };
        }
        word_counts[p as usize]
    }

    pub fn count_words_alloc(&self) -> Box<[u32]> {
        let mut word_counts = vec![0u32; self.0.len()];
        for p in (0..word_counts.len()).rev() {
            self.count_words_at(&mut word_counts, p as i32);
        }
        word_counts.into_boxed_slice()
    }

    pub fn count_dawg_words_alloc(&self) -> Box<[u32]> {
        fn max_from(nodes: &Kwg, vis: &mut [u8], mut p: i32) -> i32 {
            let mut ret = 0;
            loop {
                let p_byte_index = (p as usize) / 8;
                let p_bit = 1 << (p as usize % 8);
                if vis[p_byte_index] & p_bit != 0 {
                    break;
                }
                vis[p_byte_index] |= p_bit;
                if nodes[p].arc_index() != 0 {
                    ret = std::cmp::max(ret, max_from(nodes, vis, nodes[p].arc_index()));
                }
                if nodes[p].is_end() {
                    break;
                }
                p += 1;
            }
            std::cmp::max(ret, p)
        }
        let required_size = max_from(&self, &mut vec![0u8; (self.0.len() + 7) / 8], 0) as usize + 1;
        let mut word_counts = vec![0u32; required_size];
        for p in (0..word_counts.len()).rev() {
            self.count_words_at(&mut word_counts, p as i32);
        }
        word_counts.into_boxed_slice()
    }

    #[inline(always)]
    pub fn get_word_by_index<F: FnMut(u8)>(
        &self,
        word_counts: &[u32],
        mut p: i32,
        mut idx: u32,
        mut out: F,
    ) {
        while !(idx == 0 && self[p].accepts()) {
            let words_here = if self[p].is_end() {
                word_counts[p as usize]
            } else {
                word_counts[p as usize] - word_counts[p as usize + 1]
            };
            if idx < words_here {
                idx -= self[p].accepts() as u32;
                out(self[p].tile());
                p = self[p].arc_index();
            } else {
                idx -= words_here;
                if self[p].is_end() {
                    panic!();
                }
                p += 1;
            }
        }
        out(self[p].tile());
    }

    #[inline(always)]
    pub fn get_word_index(&self, word_counts: &[u32], mut p: i32, word: &[u8]) -> u32 {
        let mut idx = 0;
        for (remaining, &tile) in (0..word.len()).rev().zip(word) {
            if p == 0 {
                return !0;
            }
            while self[p].tile() != tile {
                if self[p].is_end() {
                    return !0;
                }
                idx += word_counts[p as usize] - word_counts[p as usize + 1];
                p += 1;
            }
            if remaining == 0 {
                return idx | ((self[p].accepts() as i32 - 1) as u32);
            }
            idx += self[p].accepts() as u32;
            p = self[p].arc_index();
        }
        !0
    }

    // slower than just using the index
    #[inline(always)]
    pub fn get_word_index_of<I: Iterator<Item = u8>>(
        &self,
        word_counts: &[u32],
        mut p: i32,
        iter: &mut I,
    ) -> u32 {
        let mut idx = 0;
        if let Some(mut tile) = iter.next() {
            loop {
                if p == 0 {
                    break;
                }
                while self[p].tile() != tile {
                    if self[p].is_end() {
                        return !0;
                    }
                    idx += word_counts[p as usize] - word_counts[p as usize + 1];
                    p += 1;
                }
                match iter.next() {
                    Some(t) => {
                        tile = t;
                    }
                    None => {
                        return idx | ((self[p].accepts() as i32 - 1) as u32);
                    }
                }
                idx += self[p].accepts() as u32;
                p = self[p].arc_index();
            }
        }
        !0
    }
}
