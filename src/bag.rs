// Copyright (C) 2020-2026 Andy Kurnia.

use super::alphabet;
use rand::prelude::*;

pub struct Bag {
    tiles: Vec<u8>,
    fc: usize, // front cursor: tiles[0..fc] is dead space, tiles[fc..] is playable
    canonical: Box<[u8]>, // initial tile sequence, for zero-alloc reset
}

impl Bag {
    #[inline(always)]
    pub fn new(alphabet: &alphabet::Alphabet) -> Bag {
        let total_tiles: usize = (0..alphabet.len())
            .map(|tile| alphabet.freq(tile) as usize)
            .sum();
        let mut tiles = Vec::with_capacity(total_tiles + 16);
        for tile in 0..alphabet.len() {
            for _ in 0..alphabet.freq(tile) {
                tiles.push(tile);
            }
        }
        let canonical = tiles.clone().into_boxed_slice();
        Bag {
            tiles,
            fc: 0,
            canonical,
        }
    }

    #[inline(always)]
    pub fn reset(&mut self) {
        self.tiles.clear();
        self.fc = 0;
        self.tiles.extend_from_slice(&self.canonical);
    }

    pub fn shuffle<R: Rng + ?Sized>(&mut self, rng: &mut R) {
        self.tiles[self.fc..].shuffle(rng);
    }

    #[inline(always)]
    pub fn shuffle_n<R: Rng + ?Sized>(&mut self, rng: &mut R, amount: usize) {
        // this "correctly" puts the shuffled amount at the end
        let _ = self.tiles[self.fc..].partial_shuffle(rng, amount);
    }

    #[inline(always)]
    pub fn pop(&mut self) -> Option<u8> {
        self.pop_back()
    }

    #[inline(always)]
    pub fn pop_back(&mut self) -> Option<u8> {
        if self.tiles.len() > self.fc {
            self.tiles.pop()
        } else {
            None
        }
    }

    #[inline(always)]
    pub fn pop_front(&mut self) -> Option<u8> {
        if self.fc < self.tiles.len() {
            let tile = self.tiles[self.fc];
            self.fc += 1;
            Some(tile)
        } else {
            None
        }
    }

    #[inline(always)]
    pub fn replenish(&mut self, rack: &mut Vec<u8>, rack_size: usize, player_index: usize) {
        if player_index.is_multiple_of(2) {
            self.replenish_back(rack, rack_size);
        } else {
            self.replenish_front(rack, rack_size);
        }
    }

    #[inline(always)]
    pub fn replenish_back(&mut self, rack: &mut Vec<u8>, rack_size: usize) {
        let playable = self.tiles.len() - self.fc;
        for _ in 0..(rack_size - rack.len()).min(playable) {
            rack.push(self.pop_back().unwrap());
        }
    }

    #[inline(always)]
    pub fn replenish_front(&mut self, rack: &mut Vec<u8>, rack_size: usize) {
        let playable = self.tiles.len() - self.fc;
        for _ in 0..(rack_size - rack.len()).min(playable) {
            rack.push(self.pop_front().unwrap());
        }
    }

    #[inline(always)]
    pub fn return_tile(&mut self, tile: u8) {
        if self.fc > 0 {
            self.fc -= 1;
            self.tiles[self.fc] = tile;
        } else {
            self.tiles.push(tile);
        }
    }

    #[inline(always)]
    pub fn return_tiles(&mut self, tiles: &[u8]) {
        for &tile in tiles {
            self.return_tile(tile);
        }
    }

    #[inline(always)]
    pub fn set_from_iter<I: IntoIterator<Item = u8>>(&mut self, iter: I) {
        self.tiles.clear();
        self.fc = 0;
        self.tiles.extend(iter);
    }

    #[inline(always)]
    pub fn as_slice(&self) -> &[u8] {
        &self.tiles[self.fc..]
    }

    pub fn len(&self) -> usize {
        self.tiles.len() - self.fc
    }

    #[inline(always)]
    pub fn is_empty(&self) -> bool {
        self.tiles.len() <= self.fc
    }

    #[inline(always)]
    pub fn remove_tile(&mut self, tile: u8) -> Option<()> {
        self.tiles[self.fc..]
            .iter()
            .rposition(|&t| t == tile)
            .map(|pos| {
                let abs_pos = self.fc + pos;
                let len = self.tiles.len();
                self.tiles.copy_within(abs_pos + 1..len, abs_pos);
                self.tiles.pop();
            })
    }

    #[inline(always)]
    pub fn put_back<R: Rng + ?Sized>(&mut self, rng: &mut R, tiles: &[u8]) {
        let m = tiles.len();
        if m == 0 {
            return;
        }
        let n = self.len();
        if m == 1 {
            let pos = rng.random_range(0..n + 1);
            if self.fc >= 1 {
                self.fc -= 1;
                self.tiles
                    .copy_within(self.fc + 1..self.fc + 1 + pos, self.fc);
            } else {
                self.tiles.push(0);
                self.tiles
                    .copy_within(self.fc + pos..self.fc + n, self.fc + pos + 1);
            }
            self.tiles[self.fc + pos] = tiles[0];
            return;
        }
        if m == 2 {
            let a = rng.random_range(0..n + 1);
            let b = rng.random_range(0..n + 2);
            let (a, b, first, second) = if a < b {
                (a, b, tiles[0], tiles[1])
            } else {
                (b, a + 1, tiles[1], tiles[0])
            };
            if self.fc >= 2 {
                self.fc -= 2;

                self.tiles
                    .copy_within(self.fc + 2..self.fc + 2 + a, self.fc);
                self.tiles[self.fc + a] = first;
                self.tiles
                    .copy_within(self.fc + 2 + a..self.fc + 1 + b, self.fc + a + 1);
                self.tiles[self.fc + b] = second;
            } else {
                self.tiles.resize(self.fc + n + 2, 0);

                self.tiles
                    .copy_within(self.fc + b - 1..self.fc + n, self.fc + b + 1);
                self.tiles[self.fc + b] = second;
                self.tiles
                    .copy_within(self.fc + a..self.fc + b - 1, self.fc + a + 1);
                self.tiles[self.fc + a] = first;
            }
            return;
        }

        let mut remaining_new = m;
        let mut remaining_old = n;
        if self.fc >= m {
            let new_base = if self.fc >= 2 * m {
                self.tiles[..m].copy_from_slice(tiles);
                0
            } else {
                self.tiles.extend_from_slice(tiles);
                self.fc + n
            };
            self.fc -= m;
            let mut old_ptr = self.fc + m;
            for wp in self.fc..self.fc + m + n {
                if remaining_new == 0 {
                    break; // old_ptr == wp; remaining old tiles are already in place.
                }
                if remaining_old > 0
                    && rng.random_range(0..remaining_new + remaining_old) >= remaining_new
                {
                    unsafe {
                        *self.tiles.get_unchecked_mut(wp) = *self.tiles.get_unchecked(old_ptr);
                    }
                    old_ptr += 1;
                    remaining_old -= 1;
                } else {
                    let pick = rng.random_range(0..remaining_new);
                    unsafe {
                        *self.tiles.get_unchecked_mut(wp) =
                            *self.tiles.get_unchecked(new_base + pick);
                    }
                    remaining_new -= 1;
                    self.tiles.swap(new_base + pick, new_base + remaining_new);
                }
            }
            if new_base > 0 {
                self.tiles.truncate(self.fc + m + n);
            }
        } else {
            let final_len = self.fc + n + m;
            self.tiles.resize(final_len + m, 0);
            self.tiles[final_len..].copy_from_slice(tiles);
            let new_base = final_len;
            let mut old_ptr = self.fc + n;
            for wp in (self.fc..final_len).rev() {
                if remaining_new == 0 {
                    break; // remaining old tiles at fc..old_ptr are already in place.
                }
                if remaining_old > 0
                    && rng.random_range(0..remaining_new + remaining_old) >= remaining_new
                {
                    old_ptr -= 1;
                    unsafe {
                        *self.tiles.get_unchecked_mut(wp) = *self.tiles.get_unchecked(old_ptr);
                    }
                    remaining_old -= 1;
                } else {
                    let pick = rng.random_range(0..remaining_new);
                    unsafe {
                        *self.tiles.get_unchecked_mut(wp) =
                            *self.tiles.get_unchecked(new_base + pick);
                    }
                    remaining_new -= 1;
                    self.tiles.swap(new_base + pick, new_base + remaining_new);
                }
            }
            self.tiles.truncate(final_len);
        }
    }
}

impl Clone for Bag {
    #[inline(always)]
    fn clone(&self) -> Self {
        Self {
            tiles: self.tiles.clone(),
            fc: self.fc,
            canonical: self.canonical.clone(),
        }
    }

    #[inline(always)]
    fn clone_from(&mut self, source: &Self) {
        self.tiles.clone_from(&source.tiles);
        self.fc = source.fc;
        self.canonical.clone_from(&source.canonical);
    }
}
