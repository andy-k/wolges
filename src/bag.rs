use super::alphabet;
use rand::prelude::*;

pub struct Bag(pub Vec<u8>);

impl Bag {
    pub fn new(alphabet: &alphabet::Alphabet) -> Bag {
        let mut bag = Vec::with_capacity(
            (0..alphabet.len())
                .map(|tile| alphabet.freq(tile) as usize)
                .sum(),
        );
        for tile in 0..alphabet.len() {
            for _ in 0..alphabet.freq(tile) {
                bag.push(tile as u8);
            }
        }
        Bag(bag)
    }

    pub fn shuffle(&mut self, mut rng: &mut dyn RngCore) {
        self.0.shuffle(&mut rng);
    }

    pub fn pop(&mut self) -> Option<u8> {
        self.0.pop()
    }

    pub fn replenish(&mut self, rack: &mut Vec<u8>, rack_size: usize) {
        for _ in 0..std::cmp::min(rack_size - rack.len(), self.0.len()) {
            rack.push(self.pop().unwrap());
        }
    }

    // put back the tiles in random order. keep the rest of the bag in the same order.
    pub fn put_back(&mut self, mut rng: &mut dyn RngCore, tiles: &[u8]) {
        /*
        pool  7: ONHUAOE
        p2 rack: CB?DPQF
        making top move: Exch. BDFPQ
        pool  7: DOBQPQF
        p2 rack: C?EOAUH
        */

        // self = ON
        // tiles = BDFPQ
        let mut num_new_tiles = tiles.len();
        match num_new_tiles {
            0 => {
                return;
            }
            1 => {
                self.0.insert(rng.gen_range(0, self.0.len()), tiles[0]);
                return;
            }
            _ => {}
        }
        let mut num_old_tiles = self.0.len();
        let new_len = num_new_tiles + num_old_tiles;
        // num_new_tiles = 5, num_old_tiles = 2, new_len = 7
        self.0.reserve(new_len);
        // self.capacity >= 9
        // p_old_tiles = 2
        let mut p_old_tiles = self.0.len();
        self.0.resize(2 * self.0.len(), 0);
        self.0.copy_within(0..num_old_tiles, num_old_tiles);
        // p_new_tiles = 4
        let mut p_new_tiles = self.0.len();
        self.0.extend_from_slice(tiles);
        self.0[p_new_tiles..].shuffle(&mut rng);
        // wp = 0..7
        // pat =           nonnnon (new/old)
        // self = ONONDBQPF
        //        w o n    n
        //        Dwo  n    o
        //        DOBQPn  n  nnn
        //           o QF       on
        for wp in 0..new_len {
            if if num_new_tiles == 0 {
                true
            } else if num_old_tiles == 0 {
                false
            } else {
                rng.gen_range(0, num_old_tiles + num_new_tiles) < num_old_tiles
            } {
                self.0[wp] = self.0[p_old_tiles];
                p_old_tiles += 1;
                num_old_tiles -= 1;
            } else {
                self.0[wp] = self.0[p_new_tiles];
                p_new_tiles += 1;
                num_new_tiles -= 1;
            }
        }
        self.0.truncate(new_len);
    }
}
