// Copyright (C) 2020-2026 Andy Kurnia.

use super::{error, game_config, klv, kwg, movegen};

pub const MAX_RACK_SIZE: u8 = u8::BITS as u8;

const LEAVE_BYTES: usize = std::mem::size_of::<i32>();
const SUBRACK_BYTES: usize = LEAVE_BYTES + std::mem::size_of::<u8>();

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
    // count the racks of each size, or the subracks in all their setups.
    fn ways_of<const SUBRACKS: bool>(counts: &[u8], rack_size: u8) -> Box<[u64]> {
        let width = rack_size as usize + 1;
        let tiles = counts.len();
        let mut ways = vec![0u64; (tiles + 1) * width];
        ways[tiles * width] = 1;
        for t in (0..tiles).rev() {
            for k in 0..width {
                ways[t * width + k] = (0..=(counts[t] as usize).min(k))
                    .map(|c| {
                        ways[(t + 1) * width + k - c] * if SUBRACKS { c as u64 + 1 } else { 1 }
                    })
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

    // advance to the next rack in count order.
    fn next_rack(&self, rack: &mut [u8]) {
        let mut suffix = 0;
        for i in (0..rack.len()).rev() {
            if suffix != 0 && rack[i] < self.counts[i] {
                rack[i] += 1;
                suffix -= 1;
                for (count, &most) in rack[i + 1..].iter_mut().zip(&self.counts[i + 1..]).rev() {
                    *count = suffix.min(most);
                    suffix -= *count;
                }
                return;
            }
            suffix += rack[i];
        }
        unreachable!("the last full rack has no next rack");
    }

    fn subracks_before(&self, rack_tally: &[u8], ways: &[u64]) -> u64 {
        let width = self.rack_size as usize + 1;
        let mut left = self.rack_size as usize;
        let mut before = 0;
        let mut prefix = 1;
        for (tile, &count) in rack_tally.iter().enumerate() {
            for earlier in 0..count as usize {
                before += prefix * (earlier as u64 + 1) * ways[(tile + 1) * width + left - earlier];
            }
            prefix *= u64::from(count) + 1;
            left -= count as usize;
        }
        before
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
        let ways = Self::ways_of::<false>(&counts, rack_size);
        let mut table = Self {
            rack_size,
            counts,
            ways,
            ends: Box::new([]),
            data: Box::new([]),
            data_start: 0,
        };
        let num_racks = table.num_racks();
        let threads = threads.max(1).min(num_racks.max(1));
        let subrack_ways = Self::ways_of::<true>(&table.counts, rack_size);
        let num_bytes = usize::try_from(subrack_ways[rack_size as usize] * SUBRACK_BYTES as u64)
            .map_err(|_| "the rack table does not fit in memory")?;
        let mut ends = vec![0u64; num_racks];
        let mut data = vec![0u8; num_bytes];
        let build_rows = |from, mut at, ends: &mut [u64], mut rows: &mut [u8]| {
            let mut rack_tally = vec![0u8; table.counts.len()];
            let mut multi_leaves = klv::MultiLeaves::new();
            let mut keys = Vec::new();
            let mut order = Vec::new();
            table.unrank(from as u64, &mut rack_tally);
            for (i, end) in ends.iter_mut().enumerate() {
                if i != 0 {
                    table.next_rack(&mut rack_tally);
                }
                multi_leaves.init_for_rack_table(&rack_tally, klv);
                let leaves = multi_leaves.leave_values();
                let (row, later) = rows.split_at_mut(leaves.len() * SUBRACK_BYTES);
                rows = later;
                let (values, permutation) = row.split_at_mut(leaves.len() * LEAVE_BYTES);
                for (to, value) in values
                    .as_chunks_mut::<LEAVE_BYTES>()
                    .0
                    .iter_mut()
                    .zip(leaves)
                {
                    *to = value.to_le_bytes();
                }
                movegen::subrack_order(&multi_leaves, &rack_tally, &mut keys, &mut order);
                permutation.copy_from_slice(&order);
                at += row.len();
                *end = at as u64;
            }
            debug_assert!(rows.is_empty());
        };
        if threads == 1 {
            // the serial builder needs no thread support, including in wasm
            if !ends.is_empty() {
                build_rows(0, 0, &mut ends, &mut data);
            }
        } else {
            let per_thread = num_racks.div_ceil(threads);
            let build_rows = &build_rows;
            std::thread::scope(|scope| {
                let mut handles = Vec::with_capacity(threads);
                let mut remaining = data.as_mut_slice();
                let mut offset = 0;
                let mut boundary_rack = vec![0u8; table.counts.len()];
                for (worker, ends) in ends.chunks_mut(per_thread).enumerate() {
                    let from = worker * per_thread;
                    let until = from + ends.len();
                    let end = if until < num_racks {
                        table.unrank(until as u64, &mut boundary_rack);
                        (table.subracks_before(&boundary_rack, &subrack_ways)
                            * SUBRACK_BYTES as u64) as usize
                    } else {
                        num_bytes
                    };
                    let (rows, later) = remaining.split_at_mut(end - offset);
                    remaining = later;
                    let at = offset;
                    offset = end;
                    handles.push(scope.spawn(move || build_rows(from, at, ends, rows)));
                }
                handles
                    .into_iter()
                    .map(|handle| handle.join())
                    .collect::<Result<Vec<_>, _>>()
            })
            .map_err(|_| "a thread making the rack table failed")?;
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
        let ways = Self::ways_of::<false>(&counts, rack_size);
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
            if !len.is_multiple_of(SUBRACK_BYTES as u64)
                || len / SUBRACK_BYTES as u64 > 1 << u8::BITS
            {
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
        Some(setup.split_at(setup.len() / SUBRACK_BYTES * LEAVE_BYTES))
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
            ways: RackTable::ways_of::<false>(&counts, rack_size),
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
        let serial = RackTable::build(&klv, &gc, 1).unwrap();
        assert!(serial.ends == table.ends && serial.data == table.data);
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
