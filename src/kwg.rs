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

impl std::ops::Index<usize> for Kwg {
    type Output = Node;

    #[inline(always)]
    fn index(&self, i: usize) -> &Node {
        &self.0[i]
    }
}

impl std::ops::Index<i32> for Kwg {
    type Output = Node;

    #[inline(always)]
    fn index(&self, i: i32) -> &Node {
        &self[i as usize]
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
        if word_counts[p as usize] == !0 {
            panic!()
        };
        if word_counts[p as usize] == 0 {
            word_counts[p as usize] = !0; // marker
            word_counts[p as usize] = if self[p as usize].accepts() { 1 } else { 0 }
                + if self[p as usize].arc_index() != 0 {
                    self.count_words_at(&mut word_counts, self[p as usize].arc_index())
                } else {
                    0
                }
                + if self[p as usize].is_end() {
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

    pub fn get_word_by_index(
        &self,
        word_counts: &[u32],
        mut p: i32,
        mut idx: u32,
        mut out_vec: Vec<u8>,
    ) -> Vec<u8> {
        out_vec.clear();
        loop {
            if idx == 0 && self[p].accepts() {
                out_vec.push(self[p].tile());
                return out_vec;
            }
            let words_here = if self[p].is_end() {
                word_counts[p as usize]
            } else {
                word_counts[p as usize] - word_counts[(p + 1) as usize]
            };
            if idx < words_here {
                if self[p].accepts() {
                    idx -= 1;
                }
                out_vec.push(self[p].tile());
                p = self[p].arc_index();
            } else {
                idx -= words_here;
                if self[p].is_end() {
                    panic!();
                }
                p += 1;
            }
        }
    }

    pub fn get_word_index(&self, word_counts: &[u32], mut p: i32, word: &[u8]) -> u32 {
        let mut idx = 0;
        for i in 0..word.len() {
            if p == 0 {
                return !0;
            }
            while self[p].tile() != word[i] {
                if self[p].is_end() {
                    return !0;
                }
                idx += word_counts[p as usize] - word_counts[(p + 1) as usize];
                p += 1;
            }
            if i == word.len() - 1 {
                return if self[p].accepts() { idx } else { !0 };
            }
            if self[p].accepts() {
                idx += 1;
            }
            p = self[p].arc_index();
        }
        !0
    }
}
