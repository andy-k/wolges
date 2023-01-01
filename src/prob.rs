// Copyright (C) 2020-2023 Andy Kurnia.

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
}

#[derive(Clone)]
pub struct WordProbability {
    dp: Box<[u64]>,
    pascal: Pascal,
    alphabet_freqs: Box<[u8]>,
    word_tally: Box<[u8]>,
}

impl WordProbability {
    pub fn new(alphabet: &alphabet::Alphabet<'_>) -> Self {
        let alphabet_freqs = (0..alphabet.len())
            .map(|tile| alphabet.freq(tile))
            .collect::<Box<_>>();
        let word_tally = vec![0; alphabet_freqs.len()].into_boxed_slice();
        Self {
            dp: vec![0; alphabet_freqs[0] as usize + 1].into_boxed_slice(),
            pascal: Pascal::new(),
            alphabet_freqs,
            word_tally,
        }
    }

    pub fn count_ways(&mut self, word: &[u8]) -> u64 {
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

    fn get_max_probs_by_len_iter(
        &mut self,
        kwg: &kwg::Kwg,
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
                v[l] = v[l].max(self.count_ways(word));
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
    pub fn get_max_probs_by_len(&mut self, kwg: &kwg::Kwg, v: &mut Vec<u64>) {
        v.clear();
        self.get_max_probs_by_len_iter(kwg, &mut Vec::new(), v, kwg[0].arc_index());
    }
}
