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
    data: Box<[u8]>,   // the index, then each rack's leaves (i32, little-endian) and order (u8)
    data_start: usize, // past the index, by rank where each rack's setup ends past it (u64)
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
    fn num_racks(&self) -> error::Returns<usize> {
        Ok(usize::try_from(self.ways[self.rack_size as usize])?)
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

    // the racks of fewer tiles than k, before the first of k in leave_index's
    // order
    #[inline(always)]
    fn leaves_before(&self, k: usize) -> usize {
        self.ways[..k].iter().sum::<u64>() as usize
    }

    // the racks of at most rack_size tiles the bag can deal
    #[inline(always)]
    fn num_leaves(&self) -> usize {
        self.leaves_before(self.rack_size as usize + 1)
    }

    // the racks before a rack of n tiles, within those of n, from the c of tile
    // t it holds, when it holds n less c of the tiles past t
    #[inline(always)]
    fn leave_step(&self, t: usize, n: usize, c: usize) -> usize {
        let width = self.rack_size as usize + 1;
        (0..c)
            .map(|fewer| self.ways[(t + 1) * width + n - fewer])
            .sum::<u64>() as usize
    }

    // a rack's index among the racks of at most rack_size tiles, by their
    // sizes, then as rank orders racks of one size; none for one the bag
    // cannot deal
    #[inline(always)]
    fn leave_index(&self, rack_tally: &[u8]) -> Option<usize> {
        let size = rack_tally.iter().map(|&c| c as usize).sum::<usize>();
        if size > self.rack_size as usize
            || rack_tally.len() > self.counts.len()
            || rack_tally
                .iter()
                .zip(&self.counts[..])
                .any(|(&c, &most)| c > most)
        {
            return None;
        }
        let mut left = size;
        let mut index = self.leaves_before(size);
        for (t, &count) in rack_tally.iter().enumerate() {
            index += self.leave_step(t, left, count as usize);
            left -= count as usize;
        }
        Some(index)
    }

    // each subrack's value of a rack, its tiles' digits (tile, count, place
    // value) in tile order, read from the values of leave_index's racks
    pub(crate) fn subrack_values(
        &self,
        digits: &[(u8, u8, u32)],
        leaves: &[i32],
        values: &mut [i32],
    ) {
        struct Gather<'a> {
            table: &'a RackTable,
            leaves: &'a [i32],
            values: &'a mut [i32],
        }

        impl Gather<'_> {
            fn smaller(
                &mut self,
                digits: &[(u8, u8, u32)],
                kept: usize,
                within: usize,
                index: usize,
            ) {
                for (i, &(tile, count, place)) in digits.iter().enumerate().rev() {
                    for ct in 1..=count as usize {
                        let n = kept + ct;
                        let rank = within + self.table.leave_step(tile as usize, n, ct);
                        let index = index + ct * place as usize;
                        self.values[index] = self.leaves[self.table.leaves_before(n) + rank];
                        self.smaller(&digits[..i], n, rank, index);
                    }
                }
            }
        }

        values[0] = 0;
        Gather {
            table: self,
            leaves,
            values,
        }
        .smaller(digits, 0, 0, 0);
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
            data: Box::new([]),
            data_start: 0,
        };
        let num_racks = table.num_racks()?;
        let threads = threads.max(1).min(num_racks.max(1));
        let subrack_ways = Self::ways_of::<true>(&table.counts, rack_size);
        let num_bytes = usize::try_from(subrack_ways[rack_size as usize] * SUBRACK_BYTES as u64)
            .map_err(|_| "the rack table does not fit in memory")?;
        let ranked = (!klv.is_empty() && klv.has_ordered_leave_runs())
            .then(|| {
                let mut values = Vec::new();
                values.try_reserve_exact(table.num_leaves()).ok()?;
                values.resize(table.num_leaves(), 0);
                klv.for_each_tally_leave_up_to(
                    table.counts.len(),
                    rack_size as usize,
                    |tally, val| {
                        if let Some(index) = table.leave_index(tally) {
                            values[index] = val;
                        }
                    },
                );
                Some(values)
            })
            .flatten();
        let data_start = (num_racks.checked_mul(8))
            .filter(|start| start.checked_add(num_bytes).is_some())
            .ok_or("the rack table does not fit in memory")?;
        let mut data = vec![0u8; data_start + num_bytes];
        let (ends, setups) = data.split_at_mut(data_start);
        let ends = ends.as_chunks_mut::<8>().0;
        let build_rows = |from, mut at, ends: &mut [[u8; 8]], mut rows: &mut [u8]| {
            let mut rack_tally = vec![0u8; table.counts.len()];
            let mut multi_leaves = klv::MultiLeaves::new();
            let mut keys = Vec::new();
            let mut order = Vec::new();
            table.unrank(from as u64, &mut rack_tally);
            for (i, end) in ends.iter_mut().enumerate() {
                if i != 0 {
                    table.next_rack(&mut rack_tally);
                }
                if let Some(values) = &ranked {
                    multi_leaves.init_for_ranked_rack(&rack_tally, &table, values);
                } else {
                    multi_leaves.init_for_rack_table(&rack_tally, klv);
                }
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
                *end = (at as u64).to_le_bytes();
            }
            debug_assert!(rows.is_empty());
        };
        if threads == 1 {
            // the serial builder needs no thread support, including in wasm
            if !ends.is_empty() {
                build_rows(0, 0, ends, setups);
            }
        } else {
            let per_thread = num_racks.div_ceil(threads);
            let build_rows = &build_rows;
            std::thread::scope(|scope| {
                let mut handles = Vec::with_capacity(threads);
                let mut remaining = setups;
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
        table.data = data.into_boxed_slice();
        table.data_start = data_start;
        Ok(table)
    }

    // the file: by rank, where each rack's setup ends (u64, little-endian),
    // then the setups; no header, its racks those this config's bag deals
    pub fn write(&self, out: &mut impl std::io::Write) -> error::Returns<()> {
        out.write_all(&self.data)?;
        Ok(())
    }

    // a file write made, for this config's bag and rack size
    pub fn read(game_config: &game_config::GameConfig, bytes: &[u8]) -> error::Returns<Self> {
        let (mut table, start) = Self::read_index(game_config, bytes)?;
        table.data = bytes.into();
        table.data_start = start;
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
        let table = Self {
            rack_size,
            counts,
            ways,
            data: Box::new([]),
            data_start: 0,
        };
        let num_racks = table.num_racks()?;
        let ends_len = num_racks
            .checked_mul(8)
            .filter(|&len| len <= bytes.len())
            .ok_or("the rack table is too short for this config's racks")?;
        let (ends, data) = bytes.split_at(ends_len);
        let mut from = 0u64;
        for end in ends.as_chunks::<8>().0 {
            let end = u64::from_le_bytes(*end);
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
        Ok((table, ends_len))
    }

    // where the setup of the rack of this rank ends, past the index
    #[inline(always)]
    fn end(&self, rank: usize) -> usize {
        u64::from_le_bytes(self.data[..self.data_start].as_chunks::<8>().0[rank]) as usize
    }

    // a full rack's setup: its leaves (i32, little-endian) and its subracks'
    // order; none for a rack this table does not hold
    #[inline(always)]
    pub fn get(&self, rack_tally: &[u8]) -> Option<(&[u8], &[u8])> {
        let rank = self.rank(rack_tally)?;
        let from = if rank == 0 { 0 } else { self.end(rank - 1) };
        let setup = &self.data[self.data_start + from..self.data_start + self.end(rank)];
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
                        assert!(rank < table.num_racks().unwrap());
                        let mut back = [9u8; 6];
                        back[5] = 0;
                        table.unrank(rank as u64, &mut back);
                        assert_eq!(back, rack);
                        seen += 1;
                    }
                }
            }
        }
        assert_eq!(seen, table.num_racks().unwrap());
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
            leaves.push((vec![a], f32::from(a % 7) - 3.0));
            for b in (a..tiles).step_by(5) {
                leaves.push((vec![a, b], f32::from(a + 2 * b) % 9.0 - 4.5));
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
        assert!(serial.data_start == table.data_start && serial.data == table.data);
        let mut file = Vec::new();
        table.write(&mut file).unwrap();
        let read_back = RackTable::read(&gc, &file).unwrap();
        assert!(read_back.data_start == table.data_start && read_back.data == table.data);
        let read_owned = RackTable::from_bytes_alloc(&gc, file.clone()).unwrap();
        let mut rewritten = Vec::new();
        read_owned.write(&mut rewritten).unwrap();
        assert_eq!(rewritten, file);
        let mut rack = vec![0u8; tiles as usize];
        let mut walked = klv::MultiLeaves::new();
        let mut read = klv::MultiLeaves::new();
        let mut order = Vec::new();
        let mut keys = Vec::new();
        for rank in (0..table.num_racks().unwrap()).step_by(97) {
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
