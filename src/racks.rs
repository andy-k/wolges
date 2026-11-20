// Copyright (C) 2020-2026 Andy Kurnia.

use super::{error, game_config, klv, kwg, movegen};

pub const MAX_RACK_SIZE: u8 = u8::BITS as u8;

// every full rack's setup as a generation makes it from a klv, made once: the
// leave of each part of the rack it keeps, by MultiLeaves's leave index, and
// the order the table walk takes its subracks in (movegen::subrack_order's),
// each rack found by its rank among the full racks the bag's tiles can deal. A
// generation given a full rack reads its setup here in place of walking the
// leaves' graph and sorting the subracks.
pub struct RackTable {
    rack_size: u8,
    counts: Box<[u8]>, // each tile's count in the bag, the blank's first
    ways: Box<[u64]>,  // the racks of k tiles that tiles t on can make, at t * (rack_size + 1) + k
    ends: Box<[u64]>,  // by rank, where each rack's setup ends in data
    data: Box<[u8]>,   // each rack's leaves (i32, little-endian), then its order (u8)
    data_start: usize, // past the index when the original file bytes back data
}

impl RackTable {
    // the racks of each size that each run of the tiles can make
    fn ways_of(counts: &[u8], rack_size: u8) -> Box<[u64]> {
        let width = rack_size as usize + 1;
        let tiles = counts.len();
        let mut ways = vec![0u64; (tiles + 1) * width];
        ways[tiles * width] = 1;
        for t in (0..tiles).rev() {
            for k in 0..width {
                ways[t * width + k] = (0..=(counts[t] as usize).min(k))
                    .map(|c| ways[(t + 1) * width + k - c])
                    .sum();
            }
        }
        ways.into_boxed_slice()
    }

    // the full racks the bag can deal
    #[inline(always)]
    fn num_racks(&self) -> usize {
        self.ways[self.rack_size as usize] as usize
    }

    // a rack's rank among the full racks, ordered by their counts of each tile
    // (the blank's first, fewer first); none for a rack that is not full or
    // holds more of a tile than the bag
    #[inline(always)]
    fn rank(&self, rack_tally: &[u8]) -> Option<usize> {
        let width = self.rack_size as usize + 1;
        let mut left = self.rack_size as usize;
        let mut rank = 0u64;
        for (t, (&count, &most)) in rack_tally.iter().zip(&self.counts[..]).enumerate() {
            if count > most || count as usize > left {
                return None;
            }
            for c in 0..count as usize {
                rank += self.ways[(t + 1) * width + left - c];
            }
            left -= count as usize;
        }
        (left == 0
            && rack_tally[self.counts.len()..]
                .iter()
                .all(|&count| count == 0))
        .then_some(rank as usize)
    }

    // the full rack of this rank, into rack_tally
    fn unrank(&self, mut rank: u64, rack_tally: &mut [u8]) {
        let width = self.rack_size as usize + 1;
        let mut left = self.rack_size as usize;
        for (t, count) in rack_tally.iter_mut().enumerate().take(self.counts.len()) {
            let mut c = 0;
            loop {
                let ways = self.ways[(t + 1) * width + left - c];
                if rank < ways {
                    break;
                }
                rank -= ways;
                c += 1;
            }
            *count = c as u8;
            left -= c;
        }
    }

    // the table for this config's bag and rack size from these leaves, made in
    // threads runs of ranks at once
    pub fn build<L: kwg::Node + Sync>(
        klv: &klv::Klv<L>,
        game_config: &game_config::GameConfig,
        threads: usize,
    ) -> error::Returns<Self> {
        game_config.supports(game_config::Op::Tables)?;
        let alphabet = game_config.alphabet();
        let counts = (0..alphabet.len())
            .map(|tile| alphabet.freq(tile))
            .collect::<Box<[u8]>>();
        let rack_size = game_config.rack_size();
        let ways = Self::ways_of(&counts, rack_size);
        let mut table = Self {
            rack_size,
            counts,
            ways,
            ends: Box::new([]),
            data: Box::new([]),
            data_start: 0,
        };
        let num_racks = table.num_racks();
        let threads = threads.max(1);
        let per_thread = num_racks.div_ceil(threads);
        let table_ref = &table;
        let parts = std::thread::scope(|scope| {
            let handles = (0..threads)
                .map(|i| {
                    let from = (i * per_thread).min(num_racks);
                    let to = ((i + 1) * per_thread).min(num_racks);
                    scope.spawn(move || {
                        let mut rack_tally = vec![0u8; table_ref.counts.len()];
                        let mut multi_leaves = klv::MultiLeaves::new();
                        let mut order = Vec::new();
                        let mut keys = Vec::new();
                        let mut lens = Vec::with_capacity(to - from);
                        let mut data = Vec::new();
                        for rank in from..to {
                            table_ref.unrank(rank as u64, &mut rack_tally);
                            multi_leaves.init(
                                &rack_tally,
                                klv,
                                true,
                                klv::AdjustLeave::Identity,
                                multi_leaves.dense_most(),
                                std::iter::empty(),
                            );
                            let at = data.len();
                            for &leave_value in multi_leaves.leave_values() {
                                data.extend_from_slice(&leave_value.to_le_bytes());
                            }
                            movegen::subrack_order(
                                &multi_leaves,
                                &rack_tally,
                                &mut keys,
                                &mut order,
                            );
                            data.extend_from_slice(&order);
                            lens.push((data.len() - at) as u64);
                        }
                        (lens, data)
                    })
                })
                .collect::<Vec<_>>();
            handles
                .into_iter()
                .map(|handle| handle.join())
                .collect::<Result<Vec<_>, _>>()
        })
        .map_err(|_| "a thread making the rack table failed")?;
        let mut ends = Vec::with_capacity(num_racks);
        let mut data = Vec::with_capacity(parts.iter().map(|(_, part)| part.len()).sum());
        for (lens, part) in parts {
            for len in lens {
                ends.push(ends.last().copied().unwrap_or(0) + len);
            }
            data.extend_from_slice(&part);
        }
        table.ends = ends.into_boxed_slice();
        table.data = data.into_boxed_slice();
        Ok(table)
    }

    // the file: by rank, where each rack's setup ends (u64, little-endian),
    // then the setups; no header, its racks those this config's bag deals
    pub fn write(&self, out: &mut impl std::io::Write) -> error::Returns<()> {
        for end in &self.ends {
            out.write_all(&end.to_le_bytes())?;
        }
        out.write_all(&self.data[self.data_start..])?;
        Ok(())
    }

    // a file write made, for this config's bag and rack size
    pub fn read(game_config: &game_config::GameConfig, bytes: &[u8]) -> error::Returns<Self> {
        let (mut table, start) = Self::read_index(game_config, bytes)?;
        table.data = bytes[start..].into();
        Ok(table)
    }

    pub fn from_bytes_alloc(
        game_config: &game_config::GameConfig,
        bytes: Vec<u8>,
    ) -> error::Returns<Self> {
        let (mut table, start) = Self::read_index(game_config, &bytes)?;
        table.data_start = start;
        table.data = bytes.into_boxed_slice();
        Ok(table)
    }

    fn read_index(
        game_config: &game_config::GameConfig,
        bytes: &[u8],
    ) -> error::Returns<(Self, usize)> {
        game_config.supports(game_config::Op::Tables)?;
        let alphabet = game_config.alphabet();
        let counts = (0..alphabet.len())
            .map(|tile| alphabet.freq(tile))
            .collect::<Box<[u8]>>();
        let rack_size = game_config.rack_size();
        let ways = Self::ways_of(&counts, rack_size);
        let mut table = Self {
            rack_size,
            counts,
            ways,
            ends: Box::new([]),
            data: Box::new([]),
            data_start: 0,
        };
        let num_racks = table.num_racks();
        let ends_len = num_racks
            .checked_mul(8)
            .filter(|&len| len <= bytes.len())
            .ok_or("the rack table is too short for this config's racks")?;
        let (ends, data) = bytes.split_at(ends_len);
        let ends = ends
            .as_chunks::<8>()
            .0
            .iter()
            .map(|end| u64::from_le_bytes(*end))
            .collect::<Box<[u64]>>();
        let mut from = 0u64;
        for &end in &ends {
            let len = end
                .checked_sub(from)
                .ok_or("the rack table's setups do not follow each other")?;
            if len % 5 != 0 || len / 5 > 1 << 8 {
                return Err("the rack table holds a setup of no rack".into());
            }
            from = end;
        }
        if from != data.len() as u64 {
            return Err("the rack table's setups do not end where it does".into());
        }
        table.ends = ends;
        Ok((table, ends_len))
    }

    // a full rack's setup: its leaves (i32, little-endian) and its subracks'
    // order; none for a rack this table does not hold
    #[inline(always)]
    pub fn get(&self, rack_tally: &[u8]) -> Option<(&[u8], &[u8])> {
        let rank = self.rank(rack_tally)?;
        let from = if rank == 0 { 0 } else { self.ends[rank - 1] } as usize;
        let setup = &self.data[self.data_start + from..self.data_start + self.ends[rank] as usize];
        Some(setup.split_at(setup.len() / 5 * 4))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn read_refuses_a_rack_too_large_for_a_table() {
        let gc = game_config::make_hong_kong_english_game_config();
        let error = RackTable::read(&gc, &[]).err().unwrap();
        assert_eq!(
            error.to_string(),
            gc.supports(game_config::Op::Tables)
                .unwrap_err()
                .to_string()
        );
    }

    #[test]
    fn each_full_rack_has_its_own_rank() {
        // every full rack ranks to its own place, and that place's rack is it
        let counts = [2u8, 3, 0, 1, 2];
        let rack_size = 4;
        let table = RackTable {
            rack_size,
            counts: counts.into(),
            ways: RackTable::ways_of(&counts, rack_size),
            ends: Box::new([]),
            data: Box::new([]),
            data_start: 0,
        };
        let mut seen = 0;
        for a in 0..=2u8 {
            for b in 0..=3u8 {
                for d in 0..=1u8 {
                    for e in 0..=2u8 {
                        let rack = [a, b, 0, d, e, 0];
                        if a + b + d + e != rack_size {
                            assert_eq!(table.rank(&rack), None);
                            continue;
                        }
                        let rank = table.rank(&rack).unwrap();
                        assert!(rank < table.num_racks());
                        let mut back = [9u8; 6];
                        back[5] = 0;
                        table.unrank(rank as u64, &mut back);
                        assert_eq!(back, rack);
                        seen += 1;
                    }
                }
            }
        }
        assert_eq!(seen, table.num_racks());
        assert_eq!(table.rank(&[0, 3, 1, 0, 0, 0]), None);
        assert_eq!(table.rank(&[0, 3, 0, 0, 0, 1]), None);
    }

    #[test]
    fn a_full_racks_setup_is_what_the_walk_makes() {
        // for full racks across the table, the leaves, tiles played, and
        // subracks' order read from it are those init and subrack_order make;
        // the table read back from its file is the same
        let game_config::GameConfig::Static(english) = game_config::make_english_game_config();
        let gc = game_config::GameConfig::Static(game_config::StaticGameConfig {
            rack_size: 5,
            ..english
        });
        let tiles = gc.alphabet().len();
        let mut leaves = Vec::new();
        for a in 0..tiles {
            leaves.push((vec![a], (a % 7) as f32 - 3.0));
            for b in (a..tiles).step_by(5) {
                leaves.push((vec![a, b], (a + 2 * b) as f32 % 9.0 - 4.5));
            }
        }
        leaves.sort_by(|x, y| x.0.cmp(&y.0));
        let leaves = leaves
            .iter()
            .map(|(tiles, value)| (&tiles[..], *value))
            .collect::<Vec<_>>();
        let klv = klv::kbwg_from_bytes_alloc(klv::make_klv2(&leaves)).unwrap();
        let table = RackTable::build(&klv, &gc, 4).unwrap();
        let mut file = Vec::new();
        table.write(&mut file).unwrap();
        let read_back = RackTable::read(&gc, &file).unwrap();
        assert!(read_back.ends == table.ends && read_back.data == table.data);
        let read_owned = RackTable::from_bytes_alloc(&gc, file.clone()).unwrap();
        let mut rewritten = Vec::new();
        read_owned.write(&mut rewritten).unwrap();
        assert_eq!(rewritten, file);
        let mut rack = vec![0u8; tiles as usize];
        let mut walked = klv::MultiLeaves::new();
        let mut read = klv::MultiLeaves::new();
        let mut order = Vec::new();
        let mut keys = Vec::new();
        for rank in (0..table.num_racks()).step_by(97) {
            table.unrank(rank as u64, &mut rack);
            assert_eq!(table.rank(&rack), Some(rank));
            let (setup_leaves, given) = table.get(&rack).unwrap();
            assert_eq!(read_owned.get(&rack), Some((setup_leaves, given)));
            walked.init(
                &rack,
                &klv,
                true,
                klv::AdjustLeave::Identity,
                walked.dense_most(),
                std::iter::empty(),
            );
            read.init_from_leaves(&rack, setup_leaves, read.dense_most(), std::iter::empty());
            assert_eq!(read.leave_values(), walked.leave_values());
            assert_eq!(read.num_playeds(), walked.num_playeds());
            movegen::subrack_order(&walked, &rack, &mut keys, &mut order);
            assert_eq!(given, &order[..]);
        }
    }
}
