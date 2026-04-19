// Copyright (C) 2020-2026 Andy Kurnia.

use super::alphabet;
use rand::prelude::*;

pub struct Bag {
    tiles: Vec<u8>,
    fc: usize, // front cursor: tiles[0..fc] is dead space, tiles[fc..] is playable
    canonical: Box<[u8]>, // initial tile sequence, for zero-alloc reset
}

impl Bag {
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

    pub fn reset(&mut self) {
        self.tiles.clear();
        self.fc = 0;
        self.tiles.extend_from_slice(&self.canonical);
    }

    pub fn shuffle(&mut self, mut rng: &mut dyn Rng) {
        self.tiles[self.fc..].shuffle(&mut rng);
    }

    pub fn shuffle_n(&mut self, mut rng: &mut dyn Rng, amount: usize) {
        // this "correctly" puts the shuffled amount at the end
        self.tiles[self.fc..].partial_shuffle(&mut rng, amount);
    }

    pub fn pop(&mut self) -> Option<u8> {
        self.pop_back()
    }

    pub fn pop_back(&mut self) -> Option<u8> {
        if self.tiles.len() > self.fc {
            self.tiles.pop()
        } else {
            None
        }
    }

    pub fn pop_front(&mut self) -> Option<u8> {
        if self.fc < self.tiles.len() {
            let tile = self.tiles[self.fc];
            self.fc += 1;
            Some(tile)
        } else {
            None
        }
    }

    // Even players draw from back, odd players draw from front.
    pub fn replenish(&mut self, rack: &mut Vec<u8>, rack_size: usize, player_index: usize) {
        if player_index.is_multiple_of(2) {
            self.replenish_back(rack, rack_size);
        } else {
            self.replenish_front(rack, rack_size);
        }
    }

    pub fn replenish_back(&mut self, rack: &mut Vec<u8>, rack_size: usize) {
        let playable = self.tiles.len() - self.fc;
        for _ in 0..(rack_size - rack.len()).min(playable) {
            rack.push(self.pop_back().unwrap());
        }
    }

    pub fn replenish_front(&mut self, rack: &mut Vec<u8>, rack_size: usize) {
        let playable = self.tiles.len() - self.fc;
        for _ in 0..(rack_size - rack.len()).min(playable) {
            rack.push(self.pop_front().unwrap());
        }
    }

    pub fn return_tile(&mut self, tile: u8) {
        if self.fc > 0 {
            self.fc -= 1;
            self.tiles[self.fc] = tile;
        } else {
            self.tiles.push(tile);
        }
    }

    pub fn return_tiles(&mut self, tiles: &[u8]) {
        for &tile in tiles {
            self.return_tile(tile);
        }
    }

    pub fn set_from_iter<I: IntoIterator<Item = u8>>(&mut self, iter: I) {
        self.tiles.clear();
        self.fc = 0;
        self.tiles.extend(iter);
    }

    pub fn as_slice(&self) -> &[u8] {
        &self.tiles[self.fc..]
    }

    pub fn len(&self) -> usize {
        self.tiles.len() - self.fc
    }

    pub fn is_empty(&self) -> bool {
        self.tiles.len() <= self.fc
    }

    // Order-preserving removal: shift right portion left, pop.
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

    // Put back m tiles in random order. Keep the existing n tiles in order.
    // If fc >= m: vec.len() unchanged, fc -= m (no allocation).
    // If fc < m: vec.len() += m, fc unchanged.
    pub fn put_back(&mut self, rng: &mut dyn Rng, tiles: &[u8]) {
        let m = tiles.len();
        if m == 0 {
            return;
        }
        let n = self.len();
        if m == 1 {
            // Insert 1 tile at a uniformly random position among n+1 slots.
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
            // Insert 2 tiles at 2 uniformly random positions among n+2 slots.
            // The swap doubles as a coin flip for tile assignment order,
            // giving all (n+2)(n+1) ordered arrangements uniformly.
            let a = rng.random_range(0..n + 1);
            let b = rng.random_range(0..n + 2);
            let (a, b, first, second) = if a < b {
                (a, b, tiles[0], tiles[1])
            } else {
                (b, a + 1, tiles[1], tiles[0])
            };
            if self.fc >= 2 {
                self.fc -= 2;
                // Old at fc+2..fc+2+n. Left-to-right:
                // old[0..a] shift -2, old[a..b-1] shift -1, old[b-1..n] stays.
                self.tiles
                    .copy_within(self.fc + 2..self.fc + 2 + a, self.fc);
                self.tiles[self.fc + a] = first;
                self.tiles
                    .copy_within(self.fc + 2 + a..self.fc + 1 + b, self.fc + a + 1);
                self.tiles[self.fc + b] = second;
            } else {
                self.tiles.resize(self.fc + n + 2, 0);
                // Old at fc..fc+n. Right-to-left:
                // old[b-1..n] shift +2, old[a..b-1] shift +1, old[0..a] stays.
                self.tiles
                    .copy_within(self.fc + b - 1..self.fc + n, self.fc + b + 1);
                self.tiles[self.fc + b] = second;
                self.tiles
                    .copy_within(self.fc + a..self.fc + b - 1, self.fc + a + 1);
                self.tiles[self.fc + a] = first;
            }
            return;
        }
        // General case: m >= 3. Interleave with Fisher-Yates probability.
        // Scratch buffer for new tiles stored in self.tiles itself.
        let mut remaining_new = m;
        let mut remaining_old = n;
        if self.fc >= m {
            // Dead space: left-to-right (wp <= old_ptr since new_placed <= m).
            let new_base = if self.fc >= 2 * m {
                // Scratch in dead space [0..m), disjoint from write range [fc-m..fc+n).
                self.tiles[..m].copy_from_slice(tiles);
                0
            } else {
                // fc < 2m: dead space overlaps write range. Use vec's back.
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
            // Grow: right-to-left (wp >= old_ptr since new_placed <= m).
            // Scratch at [fc+n+m..fc+n+2m), disjoint from write range [fc..fc+n+m).
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
