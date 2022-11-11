// Copyright (C) 2020-2022 Andy Kurnia.

use super::alphabet;
use rand::prelude::*;

pub struct Bag(pub Vec<u8>);

impl Bag {
    pub fn new(alphabet: &alphabet::Alphabet<'_>) -> Bag {
        let mut bag = Vec::with_capacity(
            (0..alphabet.len())
                .map(|tile| alphabet.freq(tile) as usize)
                .sum(),
        );
        for tile in 0..alphabet.len() {
            for _ in 0..alphabet.freq(tile) {
                bag.push(tile);
            }
        }
        Bag(bag)
    }

    pub fn shuffle(&mut self, mut rng: &mut dyn RngCore) {
        self.0.shuffle(&mut rng);
    }

    pub fn shuffle_n(&mut self, mut rng: &mut dyn RngCore, amount: usize) {
        // this "correctly" puts the shuffled amount at the end
        self.0.partial_shuffle(&mut rng, amount);
    }

    pub fn pop(&mut self) -> Option<u8> {
        self.0.pop()
    }

    pub fn replenish(&mut self, rack: &mut Vec<u8>, rack_size: usize) {
        for _ in 0..(rack_size - rack.len()).min(self.0.len()) {
            rack.push(self.pop().unwrap());
        }
    }

    // put back the tiles in random order. keep the rest of the bag in the same order.
    pub fn put_back(&mut self, mut rng: &mut dyn RngCore, tiles: &[u8]) {
        let mut num_new_tiles = tiles.len();
        match num_new_tiles {
            0 => {
                return;
            }
            1 => {
                self.0.insert(rng.gen_range(0..=self.0.len()), unsafe {
                    *tiles.get_unchecked(0)
                });
                return;
            }
            _ => {}
        }
        let mut num_old_tiles = self.0.len();
        let num_same_prefix = rng.gen_range(0..=num_old_tiles);
        if num_same_prefix == num_old_tiles {
            // old does not move
            self.0.extend_from_slice(tiles); // [old,new]
            unsafe { self.0.get_unchecked_mut(num_old_tiles..) }.shuffle(&mut rng);
            return;
        }
        let new_len = num_new_tiles + num_old_tiles;
        self.0.reserve(num_new_tiles + new_len); // cap = old+(new+old)+new
        #[allow(clippy::uninit_vec)]
        unsafe {
            self.0.set_len(new_len + num_old_tiles);
        } // [old,?,?]
        let mut p_old_tiles = new_len; // after old+new
        self.0
            .copy_within(num_same_prefix..num_old_tiles, p_old_tiles); // [old,?,ld?]
        num_old_tiles -= num_same_prefix;
        let mut p_new_tiles = self.0.len(); // after old+new+old
        self.0.extend_from_slice(tiles); // [old,?,ld?,new]
        unsafe { self.0.get_unchecked_mut(p_new_tiles..) }.shuffle(&mut rng);
        num_new_tiles -= 1;
        unsafe {
            *self.0.get_unchecked_mut(num_same_prefix) =
                *self.0.get_unchecked(p_new_tiles + num_new_tiles);
        }
        for wp in num_same_prefix + 1..new_len {
            if if num_new_tiles == 0 {
                true
            } else if num_old_tiles == 0 {
                false
            } else {
                rng.gen_range(0..num_old_tiles + num_new_tiles) < num_old_tiles
            } {
                unsafe {
                    *self.0.get_unchecked_mut(wp) = *self.0.get_unchecked(p_old_tiles);
                }
                p_old_tiles += 1;
                num_old_tiles -= 1;
            } else {
                unsafe {
                    *self.0.get_unchecked_mut(wp) = *self.0.get_unchecked(p_new_tiles);
                }
                p_new_tiles += 1;
                num_new_tiles -= 1;
            }
        }
        unsafe {
            self.0.set_len(new_len);
        }
    }
}

impl Clone for Bag {
    #[inline(always)]
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }

    #[inline(always)]
    fn clone_from(&mut self, source: &Self) {
        self.0.clone_from(&source.0);
    }
}
