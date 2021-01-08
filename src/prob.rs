// Copyright (C) 2020-2021 Andy Kurnia. All rights reserved.

use super::alphabet;

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

pub struct WordProbability<'a> {
    dp: Box<[u64]>,
    pascal: Pascal,
    alphabet: &'a alphabet::Alphabet<'a>,
    word_tally: Box<[u8]>,
}

impl<'a> WordProbability<'a> {
    pub fn new(alphabet: &'a alphabet::Alphabet<'a>) -> Self {
        Self {
            dp: vec![0; alphabet.freq(0) as usize + 1].into_boxed_slice(),
            pascal: Pascal::new(),
            alphabet,
            word_tally: vec![0; alphabet.len() as usize].into_boxed_slice(),
        }
    }

    pub fn count_ways(&mut self, word: &[u8]) -> u64 {
        self.dp.iter_mut().for_each(|m| *m = 0);
        self.dp[0] = 1;
        self.word_tally.iter_mut().for_each(|m| *m = 0);
        word.iter().for_each(|&c| self.word_tally[c as usize] += 1);
        let n_blanks = self.alphabet.freq(0) as isize;
        for c in 1..self.alphabet.len() {
            let n_c_in_bag = self.alphabet.freq(c) as isize;
            let n_c_in_word = self.word_tally[c as usize] as isize;
            let this_pas = self.pascal.row(n_c_in_bag as usize);
            for j in (0..=n_blanks).rev() {
                let baseline = j - n_c_in_word;
                let mut v = 0;
                for k in std::cmp::max(0, baseline)..=std::cmp::min(baseline + n_c_in_bag, j) {
                    v += self.dp[k as usize] * this_pas[(k - baseline) as usize];
                }
                self.dp[j as usize] = v;
            }
        }
        self.dp
            .iter()
            .zip(self.pascal.row(n_blanks as usize))
            .map(|(prob, pas)| prob * pas)
            .sum()
    }
}
