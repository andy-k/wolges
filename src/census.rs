// Copyright (C) 2020-2026 Andy Kurnia.

use crate::bites;
use crate::fash;

pub struct MultisetLattice {
    num_letters: usize,
    rack_size: usize,
    tallies: Vec<Vec<u8>>,
    rank_of: fash::MyHashMap<bites::Bites, u32>,
}

impl MultisetLattice {
    pub fn new(num_letters: usize, rack_size: usize) -> Self {
        let mut tallies = Vec::new();
        let mut rank_of = fash::MyHashMap::default();
        let mut tally = vec![0u8; num_letters];
        fn rec(
            pos: usize,
            remaining: usize,
            num_letters: usize,
            tally: &mut Vec<u8>,
            tallies: &mut Vec<Vec<u8>>,
            rank_of: &mut fash::MyHashMap<bites::Bites, u32>,
        ) {
            if pos == num_letters {
                let idx = tallies.len() as u32;
                let mut key = Vec::new();
                for (t, &c) in tally.iter().enumerate() {
                    for _ in 0..c {
                        key.push(t as u8);
                    }
                }
                rank_of.insert(key[..].into(), idx);
                tallies.push(tally.clone());
                return;
            }
            for c in 0..=remaining {
                tally[pos] = c as u8;
                rec(pos + 1, remaining - c, num_letters, tally, tallies, rank_of);
            }
            tally[pos] = 0;
        }
        rec(
            0,
            rack_size,
            num_letters,
            &mut tally,
            &mut tallies,
            &mut rank_of,
        );
        Self {
            num_letters,
            rack_size,
            tallies,
            rank_of,
        }
    }
    #[inline]
    pub fn len(&self) -> usize {
        self.tallies.len()
    }
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.tallies.is_empty()
    }
    #[inline]
    pub fn num_letters(&self) -> usize {
        self.num_letters
    }
    #[inline]
    pub fn rack_size(&self) -> usize {
        self.rack_size
    }
    #[inline]
    pub fn tally(&self, idx: usize) -> &[u8] {
        &self.tallies[idx]
    }
    pub fn rank_bytes(&self, sorted_tiles: &[u8]) -> u32 {
        self.rank_of.get(sorted_tiles).copied().unwrap_or(!0)
    }
    pub fn rank(&self, tally: &[u8]) -> u32 {
        let mut key = Vec::new();
        for (t, &c) in tally.iter().enumerate() {
            for _ in 0..c {
                key.push(t as u8);
            }
        }
        self.rank_of.get(&key[..]).copied().unwrap_or(!0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn lattice_roundtrips_and_counts() {
        // 3 letters (incl blank=0), rack_size 2: multisets of size 0..=2.
        let lat = MultisetLattice::new(3, 2);
        // sizes: 1 (empty) + 3 (size1) + 6 (size2) = 10
        assert_eq!(lat.len(), 10);
        for idx in 0..lat.len() {
            let tally = lat.tally(idx);
            assert_eq!(lat.rank(tally), idx as u32);
            assert!(tally.iter().map(|&c| c as usize).sum::<usize>() <= 2);
        }
    }
}
