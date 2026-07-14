// Copyright (C) 2020-2026 Andy Kurnia.

use super::{alphabet, kwg};

#[derive(Clone)]
struct Pascal {
    raw: Vec<u64>,
    rows: usize,
}

impl Pascal {
    fn new() -> Self {
        Self {
            raw: vec![1],
            rows: 1,
        }
    }

    fn row(&mut self, row: usize) -> &[u64] {
        while self.rows <= row {
            let start = self.rows * (self.rows - 1) / 2;
            let mut v = 0;
            for i in start..(start + self.rows) {
                let x = self.raw[i];
                // here wrong answer is better than no answer
                self.raw.push(x.saturating_add(v));
                v = x;
            }
            self.raw.push(1);
            self.rows += 1;
        }
        let start = row * (row + 1) / 2;
        &self.raw[start..start + row + 1]
    }

    // Eagerly build rows 0..num_rows so binom() can read immutably (and across
    // threads) without the lazy &mut growth.
    pub fn with_rows(num_rows: usize) -> Self {
        let mut p = Self::new();
        if num_rows > 1 {
            p.row(num_rows - 1);
        }
        p
    }

    // C(n, k), 0 when k > n. SAFETY: row n must already be built (with_rows).
    #[inline(always)]
    pub fn binom(&self, n: usize, k: usize) -> u64 {
        if k > n {
            0
        } else {
            unsafe { *self.raw.get_unchecked(n * (n + 1) / 2 + k) }
        }
    }
}

#[derive(Clone)]
pub struct WordProbability {
    dp: Box<[u64]>,
    pascal: Pascal,
    alphabet_freqs: Box<[u8]>,
    word_tally: Box<[u8]>,
}

impl WordProbability {
    pub fn new(alphabet: &alphabet::Alphabet) -> Self {
        let alphabet_freqs = (0..alphabet.len())
            .map(|tile| alphabet.freq(tile))
            .collect::<Box<_>>();
        let word_tally = vec![0; alphabet_freqs.len()].into_boxed_slice();
        // build Pascal eagerly up to the largest single-letter count, so binom()
        // reads immutably -- this lets the draw-ways helpers take &self.
        let max_freq = alphabet_freqs.iter().copied().max().unwrap_or(0) as usize;
        Self {
            dp: vec![0; alphabet_freqs[0] as usize + 1].into_boxed_slice(),
            pascal: Pascal::with_rows(max_freq + 1),
            alphabet_freqs,
            word_tally,
        }
    }

    pub fn word_draw_ways(&mut self, word: &[u8]) -> u64 {
        self.dp.iter_mut().for_each(|m| *m = 0);
        self.dp[0] = 1;
        self.word_tally.iter_mut().for_each(|m| *m = 0);
        word.iter().for_each(|&c| self.word_tally[c as usize] += 1);
        let n_blanks = self.alphabet_freqs[0] as isize;
        for c in 1..self.alphabet_freqs.len() {
            let n_c_in_word = self.word_tally[c] as isize;
            if n_c_in_word != 0 {
                let n_c_in_bag = self.alphabet_freqs[c] as isize;
                let this_pas = self.pascal.row(n_c_in_bag as usize);
                for j in (0..=n_blanks).rev() {
                    let baseline = j - n_c_in_word;
                    let mut v = 0;
                    for k in 0.max(baseline)..=(baseline + n_c_in_bag).min(j) {
                        v += self.dp[k as usize] * this_pas[(k - baseline) as usize];
                    }
                    self.dp[j as usize] = v;
                }
            }
        }
        self.dp
            .iter()
            .zip(self.pascal.row(n_blanks as usize))
            .map(|(prob, pas)| prob * pas)
            .sum()
    }

    fn get_max_probs_by_len_iter<N: kwg::Node>(
        &mut self,
        kwg: &kwg::Kwg<N>,
        word: &mut Vec<u8>,
        v: &mut Vec<u64>,
        mut p: i32,
    ) {
        let l = word.len() + 1;
        loop {
            let node = kwg[p];
            let t = node.tile();
            word.push(t);
            if node.accepts() {
                while v.len() <= l {
                    v.push(0);
                }
                v[l] = v[l].max(self.word_draw_ways(word));
            }
            if node.arc_index() != 0 {
                self.get_max_probs_by_len_iter(kwg, word, v, node.arc_index());
            }
            word.pop();
            if node.is_end() {
                break;
            }
            p += 1;
        }
    }

    #[inline(always)]
    pub fn get_max_probs_by_len<N: kwg::Node>(&mut self, kwg: &kwg::Kwg<N>, v: &mut Vec<u64>) {
        v.clear();
        self.get_max_probs_by_len_iter(kwg, &mut Vec::new(), v, kwg[0].arc_index());
    }

    #[inline(always)]
    pub fn combination(&mut self, n: usize, r: usize) -> u64 {
        *self.pascal.row(n).get(r).unwrap_or(&0)
    }

    // the full bag, the draw source for the global (board-independent)
    // decompose. board-conditional apportionments pass a board's unseen pool instead.
    #[inline(always)]
    pub fn bag(&self) -> &[u8] {
        &self.alphabet_freqs
    }

    // number of ways to draw the completion R-S (the tiles added to fill a held
    // leave S up to the full rack R) from `source` with S removed: the product
    // over letters of C(source[t]-S[t], R[t]-S[t]). returns 0 (never panics) when
    // the draw is impossible -- S is not a subrack of R, or R is not drawable from
    // `source`. `source` is bag() for the global decompose, or a board's unseen
    // pool for a board-conditional apportionment.
    pub fn completion_draw_ways(
        &self,
        full_rack_tally: &[u8],
        subrack_tally: &[u8],
        source: &[u8],
    ) -> u64 {
        let mut v: u64 = 1;
        for c in 0..self.alphabet_freqs.len() {
            let target = full_rack_tally[c] as isize - subrack_tally[c] as isize;
            let avail = source[c] as isize - subrack_tally[c] as isize;
            if target < 0 || avail < 0 || target > avail {
                return 0;
            }
            v = v.saturating_mul(self.pascal.binom(avail as usize, target as usize));
        }
        v
    }

    // number of ways to draw the full rack R from `source`: the product over
    // letters of C(source[t], R[t]). returns 0 when R is not drawable from source.
    pub fn full_rack_draw_ways(&self, full_rack_tally: &[u8], source: &[u8]) -> u64 {
        let mut v: u64 = 1;
        for c in 0..self.alphabet_freqs.len() {
            if full_rack_tally[c] > source[c] {
                return 0;
            }
            v = v.saturating_mul(
                self.pascal
                    .binom(source[c] as usize, full_rack_tally[c] as usize),
            );
        }
        v
    }
}
