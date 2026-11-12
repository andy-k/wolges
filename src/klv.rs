// Copyright (C) 2020-2026 Andy Kurnia.

use super::{bites, census, equity, error, kwg, racks};

const MAX_LETTERS: usize = 64;

#[derive(Clone, Copy)]
pub struct DynamicLeavesRef<'a> {
    pub lat: &'a census::MultisetLattice,
    pub add: &'a census::AddTable,
    pub full_v: &'a [i32],
    pub min_keep: usize,
}

impl DynamicLeavesRef<'_> {
    #[inline(always)]
    pub fn draw_count(&self, keep_size: usize, pool_size: usize, bag_size: usize) -> usize {
        (self.lat.rack_size() - keep_size)
            .min(pool_size)
            .min(bag_size)
    }
}

#[derive(Clone, Copy)]
pub enum AdjustLeave {
    Identity,
    Scaled { scale: i32, denom: i32 },
}

impl AdjustLeave {
    #[inline(always)]
    pub fn apply(&self, leave_value: i32) -> i32 {
        match *self {
            AdjustLeave::Identity => leave_value,
            AdjustLeave::Scaled { scale, denom } => {
                (leave_value as i64 * scale as i64 / denom as i64) as i32
            }
        }
    }
}

pub struct Klv<L: kwg::Node> {
    kwg: kwg::Kwg<L>,
    counts: Box<[u32]>,
    leaves: Box<[i32]>, // millipoints, converted from f32 at load time
    racks: Option<Box<racks::RackTable>>, // every full rack's setup from these leaves, if given
}

// kwg::Node22
pub static EMPTY_KLV_BYTES: &[u8] = b"\x01\x00\x00\x00\x00\x00\x40\x00\x00\x00\x00\x00";

// a klv is the leaves' word graph (its node count, then its nodes), then the
// leaves' values (their count, then the values: each an f32 in a klv2, or an
// i16 counting 1/256 of a point in a klv), and nothing after them.
struct KlvParts<'a> {
    kwg_bytes: &'a [u8],
    lv_len: usize,
    values: &'a [u8],
    is_klv2: bool,
}

#[inline(always)]
fn split_klv(buf: &[u8]) -> error::Returns<KlvParts<'_>> {
    if buf.len() < 4 {
        return Err("klv is too short for its header".into());
    }
    let kwg_bytes = (kwg::read_le_u32(buf, 0) as usize)
        .checked_mul(4)
        .and_then(|kwg_bytes_len| buf[4..].get(..kwg_bytes_len))
        .ok_or("klv is too short for the kwg it declares")?;
    let rest = &buf[4 + kwg_bytes.len()..];
    if rest.len() < 4 {
        return Err("klv is too short for the kwg it declares".into());
    }
    let lv_len = kwg::read_le_u32(rest, 0) as usize;
    let values = &rest[4..];
    let is_klv2 = if lv_len.checked_mul(4) == Some(values.len()) {
        true
    } else if lv_len.checked_mul(2) == Some(values.len()) {
        false
    } else {
        return Err(format!(
            "klv has {} bytes for its {lv_len} leave values",
            values.len()
        )
        .into());
    };
    Ok(KlvParts {
        kwg_bytes,
        lv_len,
        values,
        is_klv2,
    })
}

// a klv's leave graph rewritten in place from one layout to the other, as
// kwg::repack_in_place does a word graph's; its values stay as they are
#[inline(always)]
pub fn repack_in_place<Old: kwg::Node, New: kwg::Node>(buf: &mut [u8]) -> error::Returns<()> {
    let kwg_len = split_klv(buf)?.kwg_bytes.len();
    kwg::repack_in_place::<Old, New>(&mut buf[4..4 + kwg_len])
}

// a klv or klv2 file, whose leaves' graph is a kwg's, read into kbwg's layout,
// the one layout the engine works in
#[inline]
pub fn kbwg_from_bytes_alloc(buf: Vec<u8>) -> error::Returns<Klv<kwg::Node24>> {
    Klv::from_reader::<kwg::Node22>(buf.len(), |at, piece| {
        piece.copy_from_slice(&buf[at..at + piece.len()])
    })
}

// a leave's value as a leave file writes it: ryu's digits, the fewest that read
// back as this very number (a tie to the even one, as kwgc's printf takes it),
// as csv would write them, but never with an exponent: 8.34465e-7 is written
// 0.000000834465, as kwgc's read-klv writes it
#[inline]
pub fn value_text<F: ryu::Float>(value: F) -> String {
    let mut buffer = ryu::Buffer::new();
    let text = buffer.format(value);
    let Some((mantissa, exponent)) = text.split_once('e') else {
        return text.to_string();
    };
    let exponent = exponent.parse::<i32>().unwrap_or(0);
    let (sign, mantissa) = match mantissa.strip_prefix('-') {
        Some(unsigned) => ("-", unsigned),
        None => ("", mantissa),
    };
    let (whole, fraction) = mantissa.split_once('.').unwrap_or((mantissa, ""));
    let digits = format!("{whole}{fraction}");
    // where the point falls among the digits
    let point = whole.len() as i32 + exponent;
    let mut out = String::from(sign);
    if point <= 0 {
        out.push_str("0.");
        out.extend(std::iter::repeat_n('0', point.unsigned_abs() as usize));
        out.push_str(&digits);
    } else if point as usize >= digits.len() {
        out.push_str(&digits);
        out.extend(std::iter::repeat_n('0', point as usize - digits.len()));
        out.push_str(".0");
    } else {
        out.push_str(&digits[..point as usize]);
        out.push('.');
        out.push_str(&digits[point as usize..]);
    }
    out
}

// a klv2 value as the millipoints a leave is kept in; one that is not a number,
// or too big for an i32 of them, is refused (`as` would make it 0 or the
// nearest end of i32), as buildlex refuses it in a leave file. The refusal
// names the kind of value, not the value: writing an f32 would build float
// formatting into the wasm, which never prints one otherwise.
#[inline(always)]
fn millipoints(i: usize, value: f32) -> error::Returns<i32> {
    let scaled = (value * equity::SCALE as f32).round();
    if (i32::MIN as f32..-(i32::MIN as f32)).contains(&scaled) {
        Ok(scaled as i32)
    } else {
        let kind = if value.is_nan() {
            "not a number"
        } else if value.is_infinite() {
            "an infinity"
        } else {
            "past an i32 of millipoints"
        };
        Err(format!("klv's leave {i} has a value that is {kind}, which a leave cannot hold").into())
    }
}

// the leaves' graph and its word counts, which also refuse a graph that loops;
// it must hold one leave for each value.
#[inline(always)]
fn load_leaves_graph<L: kwg::Node>(
    kwg_bytes: &[u8],
    lv_len: usize,
) -> error::Returns<(kwg::Kwg<L>, Box<[u32]>)> {
    let kwg = kwg::Kwg::<L>::from_bytes_alloc(kwg_bytes)?;
    let counts = kwg.count_words_alloc()?;
    let root = kwg[0].arc_index();
    let num_leaves = if root != 0 { counts[root as usize] } else { 0 };
    if num_leaves as usize != lv_len {
        return Err(format!("klv has {lv_len} leave values for {num_leaves} leaves").into());
    }
    Ok((kwg, counts))
}

impl<L: kwg::Node> Klv<L> {
    pub fn from_bytes_alloc(buf: &[u8]) -> error::Returns<Self> {
        Self::from_reader::<L>(buf.len(), |at, piece| {
            piece.copy_from_slice(&buf[at..at + piece.len()])
        })
    }

    // a klv or klv2 of len bytes whose leaves' graph is in Old's layout, read a
    // piece at a time as kwg::Kwg::from_reader reads a graph, each value made
    // the millipoints it is kept in as it comes: read(at, piece) fills piece
    // with the file's bytes from at on
    #[inline]
    pub fn from_reader<Old: kwg::Node>(
        len: usize,
        mut read: impl FnMut(usize, &mut [u8]),
    ) -> error::Returns<Self> {
        Self::from_pieces::<Old>(len, kwg::READ_PIECE, &mut read)
    }

    // the same in pieces of piece_len, a whole number of nodes; its parts and
    // their checks are split_klv's and load_leaves_graph's
    #[inline]
    fn from_pieces<Old: kwg::Node>(
        len: usize,
        piece_len: usize,
        read: &mut impl FnMut(usize, &mut [u8]),
    ) -> error::Returns<Self> {
        let mut word = [0u8; 4];
        if len < 4 {
            return Err("klv is too short for its header".into());
        }
        read(0, &mut word);
        let kwg_bytes_len = (u32::from_le_bytes(word) as usize)
            .checked_mul(4)
            .filter(|&kwg_bytes_len| kwg_bytes_len <= len - 4)
            .ok_or("klv is too short for the kwg it declares")?;
        if len - 4 - kwg_bytes_len < 4 {
            return Err("klv is too short for the kwg it declares".into());
        }
        read(4 + kwg_bytes_len, &mut word);
        let lv_len = u32::from_le_bytes(word) as usize;
        let values_at = 8 + kwg_bytes_len;
        let values_len = len - values_at;
        let is_klv2 = if lv_len.checked_mul(4) == Some(values_len) {
            true
        } else if lv_len.checked_mul(2) == Some(values_len) {
            false
        } else {
            return Err(format!("klv has {values_len} bytes for its {lv_len} leave values").into());
        };
        let kwg = kwg::Kwg::<L>::from_pieces::<Old>(4, kwg_bytes_len, piece_len, read)?;
        let counts = kwg.count_words_alloc()?;
        let root = kwg[0].arc_index();
        let num_leaves = if root != 0 { counts[root as usize] } else { 0 };
        if num_leaves as usize != lv_len {
            return Err(format!("klv has {lv_len} leave values for {num_leaves} leaves").into());
        }
        let mut leaves = Vec::with_capacity(lv_len);
        let mut buf = vec![0u8; piece_len.min(values_len)];
        let mut at = 0;
        while at < values_len {
            let piece = &mut buf[..piece_len.min(values_len - at)];
            read(values_at + at, piece);
            if is_klv2 {
                for value in piece.as_chunks::<4>().0 {
                    leaves.push(millipoints(leaves.len(), f32::from_le_bytes(*value))?);
                }
            } else {
                // a klv's i16 counts 1/256 of a point
                for value in piece.as_chunks::<2>().0 {
                    leaves.push((i16::from_le_bytes(*value) as i32 * equity::SCALE + 128) / 256);
                }
            }
            at += piece.len();
        }
        Ok(Klv {
            kwg,
            counts,
            leaves: leaves.into_boxed_slice(),
            racks: None,
        })
    }

    // every full rack's setup from these leaves (racks::RackTable::build's, or
    // a file of one read), which a generation then reads for a full rack in
    // place of walking the leaves' graph
    #[inline]
    pub fn set_racks(&mut self, racks: racks::RackTable) {
        self.racks = Some(Box::new(racks));
    }

    #[inline(always)]
    pub fn racks(&self) -> Option<&racks::RackTable> {
        self.racks.as_deref()
    }

    // the table of no leaves, its one node in this layout
    #[inline]
    pub fn empty() -> error::Returns<Self> {
        let node = L::pack(0, false, true, 0).ok_or("a layout without the empty node")?;
        let mut buf = [0u8; 12];
        buf[..4].copy_from_slice(&1u32.to_le_bytes());
        buf[4..8].copy_from_slice(&node.to_le_bytes());
        Self::from_bytes_alloc(&buf)
    }

    #[inline(always)]
    pub fn leave(&self, leave_idx: u32) -> i32 {
        self.leaves[leave_idx as usize]
    }

    #[inline]
    pub fn leave_range(&self) -> (i32, i32) {
        self.leaves
            .iter()
            .fold((0, 0), |(lo, hi), &leave| (lo.min(leave), hi.max(leave)))
    }

    // the largest tile on any node of its leaves' graph, found in one pass; a
    // reader that takes it once can check the table against any alphabet
    #[inline]
    pub fn max_tile(&self) -> Option<u8> {
        self.kwg.0.iter().map(|node| node.tile()).max()
    }

    #[inline]
    pub fn fits_alphabet(&self, alphabet_len: u8) -> bool {
        self.kwg.fits_alphabet(alphabet_len)
    }

    #[inline(always)]
    pub fn count(&self, i: i32) -> u32 {
        self.counts[i as usize]
    }

    #[inline(always)]
    pub fn kwg(&self, i: i32) -> L {
        self.kwg[i]
    }

    #[inline(always)]
    pub fn leave_value_from_tally(&self, rack_tally: &[u8]) -> i32 {
        let leave_idx = self.kwg.get_word_index_of(
            &self.counts,
            self.kwg[0].arc_index(),
            &mut (0u8..)
                .zip(rack_tally)
                .flat_map(|(tile, &count)| std::iter::repeat_n(tile, count as usize)),
        );
        if leave_idx == !0 {
            0
        } else {
            self.leave(leave_idx)
        }
    }
}

#[inline(always)]
pub fn read_leaves_alloc<L: kwg::Node>(buf: &[u8]) -> error::Returns<Vec<(bites::Bites, f32)>> {
    let KlvParts {
        kwg_bytes,
        lv_len,
        values: value_bytes,
        is_klv2,
    } = split_klv(buf)?;
    let (kwg, _) = load_leaves_graph::<L>(kwg_bytes, lv_len)?;
    let mut values = Vec::with_capacity(lv_len);
    if is_klv2 {
        for i in 0..lv_len {
            let value = f32::from_bits(kwg::read_le_u32(value_bytes, i * 4));
            // refused as a load refuses it, so what is read builds again
            millipoints(i, value)?;
            values.push(value);
        }
    } else {
        // klv holds each value as an i16 counting 1/256 of a point.
        for i in 0..lv_len {
            values.push(kwg::read_le_u16(value_bytes, i * 2) as i16 as f32 * (1.0 / 256.0));
        }
    }
    struct Env<'a, L: kwg::Node> {
        kwg: &'a kwg::Kwg<L>,
        values: &'a [f32],
        word: Vec<u8>,
        leaves: Vec<(bites::Bites, f32)>,
    }
    // the graph was counted: it does not loop, and holds one leave a value.
    fn iter<L: kwg::Node>(env: &mut Env<'_, L>, mut p: i32) {
        loop {
            let node = env.kwg[p];
            env.word.push(node.tile());
            if node.accepts() {
                let value = env.values[env.leaves.len()];
                env.leaves.push((env.word[..].into(), value));
            }
            if node.arc_index() != 0 {
                iter(env, node.arc_index());
            }
            env.word.pop();
            if node.is_end() {
                break;
            }
            p += 1;
        }
    }
    let mut env = Env {
        kwg: &kwg,
        values: &values,
        word: Vec::new(),
        leaves: Vec::with_capacity(lv_len),
    };
    let root = kwg[0].arc_index();
    if root != 0 {
        iter(&mut env, root);
    }
    Ok(env.leaves)
}

#[derive(Clone)]
struct MultiLeavesDigit {
    count: u8,
    place_value: u32,
}

// example: EEHJTTU should make digits.len() == 27,
// [5]=(2,1), [8]=(1,3), [10]=(1,6), [20]=(2,12), [21]=(1,36),
// and the other elements are (0,0).
// unique_tiles == [5, 8, 10, 20, 21].
// leave_values.len() == 72.
// leave_values[num(E) * 1 + num(H) * 3 + num(J) * 6 + ...].
// num_playeds = [7, ..., 0] (number of tiles played, not kept)
// leave_values = [[], [E], [EE], [H], [EH], [EEH], ...]
// = [leave for exch 7, leave for exch 6 keep E, ... leave for pass].
pub struct MultiLeaves {
    unique_tiles: Vec<u8>, // sorted, unique, len() == rack_bits.count_ones()
    digits: Vec<MultiLeavesDigit>, // len() == rack_tally.len()
    leave_values: Vec<i32>, // typically 2**7, can be shorter when duplicates
    num_playeds: Vec<u8>,  // same length as leave_values
    dense_most: u32,       // the most leaves kept as a table (DENSE_LEAVES_MOST)
}

// the most leaves a rack's tiles may have for them to be kept as a table, each
// read by its index; past it, each is walked from the klv when read. A table
// costs a walk of every leave and the bytes of each (LEAVE_BYTES), so a
// generator holding few bytes takes a smaller bound (set_dense_most)
pub const DENSE_LEAVES_MOST: u32 = 1 << 20;

// the bytes a table keeps for each leave: its value and the tiles played
pub const LEAVE_BYTES: usize = std::mem::size_of::<i32>() + std::mem::size_of::<u8>();

impl Default for MultiLeaves {
    #[inline(always)]
    fn default() -> Self {
        Self::new()
    }
}

impl Clone for MultiLeaves {
    #[inline(always)]
    fn clone(&self) -> Self {
        Self {
            unique_tiles: self.unique_tiles.clone(),
            digits: self.digits.clone(),
            leave_values: self.leave_values.clone(),
            num_playeds: self.num_playeds.clone(),
            dense_most: self.dense_most,
        }
    }

    #[inline(always)]
    fn clone_from(&mut self, source: &Self) {
        self.unique_tiles.clone_from(&source.unique_tiles);
        self.digits.clone_from(&source.digits);
        self.leave_values.clone_from(&source.leave_values);
        self.num_playeds.clone_from(&source.num_playeds);
        self.dense_most = source.dense_most;
    }
}

impl MultiLeaves {
    pub fn new() -> Self {
        Self {
            unique_tiles: Vec::new(),
            digits: Vec::new(),
            leave_values: Vec::new(),
            num_playeds: Vec::new(),
            dense_most: DENSE_LEAVES_MOST,
        }
    }

    // the most leaves the next racks may have to be kept as a table
    // (DENSE_LEAVES_MOST until set); the leaves already kept stay
    #[inline(always)]
    pub fn set_dense_most(&mut self, dense_most: u32) {
        self.dense_most = dense_most;
    }

    // the most leaves a rack's tiles may have to be kept as a table
    #[inline(always)]
    pub fn dense_most(&self) -> u32 {
        self.dense_most
    }

    // the rack's unique tiles and their digits (each its count and its place
    // value in the leave index): whether the leaves are dense (no more than
    // dense_most of them), how many, and the rack's tiles
    #[inline(always)]
    fn set_digits(&mut self, rack_tally: &[u8]) -> (bool, u32, u8) {
        // every digit none: the last rack's tiles' put back (the others are
        // none already), or all of them anew for an alphabet of another length
        for &tile in &self.unique_tiles {
            self.digits[tile as usize] = MultiLeavesDigit {
                count: 0,
                place_value: 0,
            };
        }
        self.unique_tiles.clear();
        if self.digits.len() != rack_tally.len() {
            self.digits.clear();
            self.digits.resize(
                rack_tally.len(),
                MultiLeavesDigit {
                    count: 0,
                    place_value: 0,
                },
            );
        }
        let dense_most = self.dense_most;
        let mut place_value = 1u32;
        let mut num_tiles_on_rack = 0u8;
        let mut dense = true;
        for (tile, &count) in (0u8..).zip(rack_tally.iter()) {
            if count != 0 {
                self.unique_tiles.push(tile);
                if dense {
                    self.digits[tile as usize] = MultiLeavesDigit { count, place_value };
                    match place_value.checked_mul(count as u32 + 1) {
                        Some(v) if v <= dense_most => {
                            place_value = v;
                        }
                        _ => {
                            dense = false;
                        }
                    }
                }
                num_tiles_on_rack += count;
            }
        }
        (dense, place_value, num_tiles_on_rack)
    }

    // the rack's digits alone, with no leaves: for a caller that reads no leave
    // value, as a raw generation
    #[inline(always)]
    pub fn init_digits(&mut self, rack_tally: &[u8]) {
        self.set_digits(rack_tally);
        self.leave_values.clear();
        self.num_playeds.clear();
    }

    // the rack's digits and leaves: each leave walked from the klv's graph
    // (none when not use_klv), adjusted
    #[inline(always)]
    pub fn init<L: kwg::Node>(
        &mut self,
        rack_tally: &[u8],
        klv: &Klv<L>,
        use_klv: bool,
        adjust_leave_value: AdjustLeave,
    ) {
        let (dense, place_value, num_tiles_on_rack) = self.set_digits(rack_tally);
        self.leave_values.clear();
        self.num_playeds.clear();
        if dense {
            self.leave_values.resize(place_value as usize, 0);
            self.num_playeds.resize(place_value as usize, 0xff);
            self.num_playeds[0] = num_tiles_on_rack;
            struct Env<'a, L: kwg::Node> {
                klv: &'a Klv<L>,
                unique_tiles: &'a [u8],
                digits: &'a mut [MultiLeavesDigit],
                leave_values: &'a mut [i32],
                num_playeds: &'a mut [u8],
            }
            fn precompute_leaves<L: kwg::Node>(
                env: &mut Env<'_, L>,
                mut p: i32,
                mut idx: u32,
                leave_idx_offset: u32,
                unique_tile_idx_offset: u8,
                mut num_played: u8,
            ) {
                num_played = num_played.wrapping_sub(1);
                for unique_tile_idx in unique_tile_idx_offset as usize..env.unique_tiles.len() {
                    let tile = env.unique_tiles[unique_tile_idx];
                    let tile_usize = tile as usize;
                    if env.digits[tile_usize].count > 0 {
                        env.digits[tile_usize].count -= 1;
                        if p != 0 {
                            idx += env.klv.count(p);
                            loop {
                                let node = env.klv.kwg(p);
                                if node.tile() >= tile {
                                    break;
                                }
                                if node.is_end() {
                                    p = 0;
                                    break;
                                }
                                p += 1;
                            }
                        }
                        let leave_idx = leave_idx_offset + env.digits[tile_usize].place_value;
                        env.num_playeds[leave_idx as usize] = num_played;
                        if p != 0 {
                            idx -= env.klv.count(p);
                            let node = env.klv.kwg(p);
                            let leave_val = if node.tile() == tile && node.accepts() {
                                env.klv.leave(idx)
                            } else {
                                0
                            };
                            env.leave_values[leave_idx as usize] = leave_val;
                            precompute_leaves(
                                env,
                                node.arc_index(),
                                idx + node.accepts() as u32,
                                leave_idx,
                                unique_tile_idx as u8,
                                num_played,
                            );
                        } else {
                            env.leave_values[leave_idx as usize] = 0;
                            precompute_leaves(
                                env,
                                p,
                                idx,
                                leave_idx,
                                unique_tile_idx as u8,
                                num_played,
                            );
                        }
                        env.digits[tile_usize].count += 1;
                    }
                }
            }
            precompute_leaves(
                &mut Env {
                    klv,
                    unique_tiles: &self.unique_tiles,
                    digits: &mut self.digits,
                    leave_values: &mut self.leave_values,
                    num_playeds: &mut self.num_playeds,
                },
                if use_klv { klv.kwg[0].arc_index() } else { 0 },
                0,
                0,
                0,
                num_tiles_on_rack,
            );

            if use_klv {
                self.leave_values
                    .iter_mut()
                    .for_each(|m| *m = adjust_leave_value.apply(*m));
            }
        }
    }

    // init's with use_klv and no adjustment, for a rack whose leaves a rack
    // table holds (racks::RackTable::get's, little-endian i32s by leave index,
    // as init walked them): the digits as init makes them, the leaves as given,
    // and each leave index's tiles played, the rack's less those it keeps
    #[inline(always)]
    pub fn init_from_leaves(&mut self, rack_tally: &[u8], leaves: &[u8]) {
        let (dense, place_value, num_tiles_on_rack) = self.set_digits(rack_tally);
        debug_assert!(dense && place_value as usize * 4 == leaves.len());
        self.leave_values.clear();
        self.leave_values.extend(
            leaves
                .as_chunks::<4>()
                .0
                .iter()
                .map(|value| i32::from_le_bytes(*value)),
        );
        self.num_playeds.clear();
        self.num_playeds
            .resize(self.leave_values.len(), num_tiles_on_rack);
        for &tile in &self.unique_tiles {
            let MultiLeavesDigit { count, place_value } = self.digits[tile as usize];
            let place_value = place_value as usize;
            for i in place_value..place_value * (count as usize + 1) {
                self.num_playeds[i] = self.num_playeds[i - place_value] - 1;
            }
        }
    }

    // the leaves by leave index, as init made them
    #[inline(always)]
    pub fn leave_values(&self) -> &[i32] {
        &self.leave_values
    }

    #[cfg(test)]
    #[inline(always)]
    pub(crate) fn num_playeds(&self) -> &[u8] {
        &self.num_playeds
    }

    #[inline(always)]
    pub fn init_endgame_leaves<AlphabetScore: Fn(u8) -> i8>(
        &mut self,
        alphabet_score: AlphabetScore,
        play_out_bonus: i32,
    ) {
        self.leave_values[0] = -equity::ENDGAME_PENALTY_BASE;
        for &tile in &self.unique_tiles {
            let penalty = -2 * alphabet_score(tile) as i32 * equity::SCALE;
            let &MultiLeavesDigit { count, place_value } = &self.digits[tile as usize];
            for i in 0..place_value * count as u32 {
                self.leave_values[(i + place_value) as usize] =
                    self.leave_values[i as usize] + penalty;
            }
        }

        self.leave_values[0] = play_out_bonus;
    }

    #[inline(always)]
    pub fn extract_best_leave_values_from_klv<L: kwg::Node>(
        rack_tally: &mut [u8],
        klv: &Klv<L>,
        num_tiles_on_rack: u8,
        adjust_leave_value: AdjustLeave,
        best_leave_values: &mut Vec<i32>,
    ) {
        best_leave_values.clear();
        best_leave_values.resize(num_tiles_on_rack as usize + 1, i32::MIN);
        struct Env<'a, L: kwg::Node> {
            klv: &'a Klv<L>,
            rack_tally: &'a mut [u8],
            kept_tally: &'a mut [u8],
            best_leave_values: &'a mut [i32],
            num_tiles_on_rack: u8,
            num_kept: u8,
            adjust_leave_value: AdjustLeave,
        }

        fn traverse<L: kwg::Node>(env: &mut Env<'_, L>, mut p: i32) {
            if p <= 0 {
                return;
            }
            loop {
                let node = env.klv.kwg(p);
                let tile = node.tile();
                if (tile as usize) < env.rack_tally.len() && env.rack_tally[tile as usize] > 0 {
                    env.rack_tally[tile as usize] -= 1;
                    env.kept_tally[tile as usize] += 1;
                    env.num_kept += 1;
                    if node.accepts() {
                        let leave_val = env
                            .adjust_leave_value
                            .apply(env.klv.leave_value_from_tally(env.kept_tally));
                        let num_played = (env.num_tiles_on_rack - env.num_kept) as usize;
                        if num_played < env.best_leave_values.len()
                            && leave_val > env.best_leave_values[num_played]
                        {
                            env.best_leave_values[num_played] = leave_val;
                        }
                    }
                    traverse(env, node.arc_index());
                    env.num_kept -= 1;
                    env.kept_tally[tile as usize] -= 1;
                    env.rack_tally[tile as usize] += 1;
                }
                if node.is_end() {
                    break;
                }
                p += 1;
            }
        }
        let tally_len = rack_tally.len();
        let mut kept_tally = vec![0u8; tally_len];
        traverse(
            &mut Env {
                klv,
                rack_tally,
                kept_tally: &mut kept_tally,
                best_leave_values,
                num_tiles_on_rack,
                num_kept: 0,
                adjust_leave_value,
            },
            klv.kwg[0].arc_index(),
        );

        for v in best_leave_values.iter_mut() {
            if *v == i32::MIN {
                *v = 0;
            }
        }
    }

    #[inline(always)]
    pub fn extract_raw_best_leave_values(&self, best_leave_values: &mut Vec<i32>) {
        best_leave_values.clear();
        best_leave_values.resize(self.num_playeds[0] as usize + 1, i32::MIN);
        // each count's best by max, not by a branch a leave
        for (&leave_value, &num_tiles_exchanged) in self.leave_values.iter().zip(&self.num_playeds)
        {
            let best = &mut best_leave_values[num_tiles_exchanged as usize];
            *best = (*best).max(leave_value);
        }
    }

    #[inline(always)]
    pub fn apply_dynamic_leaves(
        &mut self,
        dyn_ref: &DynamicLeavesRef<'_>,
        live_pool: &[u8],
        bag_size: usize,
    ) {
        if self.leave_values.is_empty() {
            return;
        }
        let (lat, add, full_v, min_keep) =
            (dyn_ref.lat, dyn_ref.add, dyn_ref.full_v, dyn_ref.min_keep);
        let num_letters = lat.num_letters();
        let pool_size: usize = live_pool[..num_letters].iter().map(|&c| c as usize).sum();

        let mut s_tally = [0u8; MAX_LETTERS];
        for idx in 0..self.leave_values.len() {
            let mut s_size = 0usize;
            for &tile in &self.unique_tiles {
                let digit = &self.digits[tile as usize];
                let kept = (idx as u32 / digit.place_value) % (digit.count as u32 + 1);
                s_tally[tile as usize] = kept as u8;
                s_size += kept as usize;
            }
            if s_size < min_keep {
                continue;
            }
            let s_ridx = lat.rank(&s_tally[..num_letters]);
            if s_ridx == !0 {
                continue;
            }
            let draw = dyn_ref.draw_count(s_size, pool_size, bag_size);
            let dynamic =
                census::dynamic_leave_value(lat, add, full_v, live_pool, s_ridx as usize, draw);
            if dynamic != census::UNPLAYABLE {
                self.leave_values[idx] = dynamic;
            }
        }
    }

    #[inline(always)]
    pub fn kurnia_gen_exchange_moves_unconditionally<'a, FoundExchangeMove: FnMut(&[u8], i32)>(
        &self,
        found_exchange_move: FoundExchangeMove,
        rack_tally: &'a mut [u8],
        exchange_buffer: &'a mut Vec<u8>,
        max_vec_len: usize,
    ) {
        exchange_buffer.clear(); // should be no-op
        struct ExchangeEnv<'a, FoundExchangeMove: FnMut(&[u8], i32)> {
            multi_leaves: &'a MultiLeaves,
            found_exchange_move: FoundExchangeMove,
            rack_tally: &'a mut [u8],
            exchange_buffer: &'a mut Vec<u8>,
            max_vec_len: usize,
        }
        fn generate_exchanges<FoundExchangeMove: FnMut(&[u8], i32)>(
            env: &mut ExchangeEnv<'_, FoundExchangeMove>,
            unique_tile_idx_offset: u8,
            leave_idx: u32,
        ) {
            if !env.exchange_buffer.is_empty() {
                (env.found_exchange_move)(
                    env.exchange_buffer,
                    env.multi_leaves.leave_values[leave_idx as usize],
                );
            }
            if env.exchange_buffer.len() < env.max_vec_len {
                for unique_tile_idx in
                    unique_tile_idx_offset as usize..env.multi_leaves.unique_tiles.len()
                {
                    let tile = env.multi_leaves.unique_tiles[unique_tile_idx];
                    let tile_usize = tile as usize;
                    if env.rack_tally[tile_usize] > 0 {
                        env.rack_tally[tile_usize] -= 1;
                        env.exchange_buffer.push(tile);
                        generate_exchanges(
                            env,
                            unique_tile_idx as u8,
                            leave_idx - env.multi_leaves.digits[tile_usize].place_value,
                        );
                        env.exchange_buffer.pop();
                        env.rack_tally[tile_usize] += 1;
                    }
                }
            }
        }
        generate_exchanges(
            &mut ExchangeEnv {
                multi_leaves: self,
                found_exchange_move,
                rack_tally,
                exchange_buffer,
                max_vec_len,
            },
            0,
            self.pass_leave_idx(),
        );
    }

    #[inline(always)]
    pub fn is_dense(&self) -> bool {
        !self.leave_values.is_empty()
    }

    #[inline(always)]
    pub fn gen_exchange_moves_via_klv<'a, FoundExchangeMove: FnMut(&[u8], i32), L: kwg::Node>(
        klv: &Klv<L>,
        found_exchange_move: FoundExchangeMove,
        rack_tally: &'a mut [u8],
        exchange_buffer: &'a mut Vec<u8>,
        max_vec_len: usize,
    ) {
        exchange_buffer.clear();
        struct ExchangeEnv<'a, FoundExchangeMove: FnMut(&[u8], i32), L: kwg::Node> {
            rack_tally_len: u8,
            klv: &'a Klv<L>,
            found_exchange_move: FoundExchangeMove,
            rack_tally: &'a mut [u8],
            exchange_buffer: &'a mut Vec<u8>,
            max_vec_len: usize,
        }
        fn generate_exchanges<FoundExchangeMove: FnMut(&[u8], i32), L: kwg::Node>(
            env: &mut ExchangeEnv<'_, FoundExchangeMove, L>,
            tile_offset: u8,
        ) {
            if !env.exchange_buffer.is_empty() {
                let leave_value = env.klv.leave_value_from_tally(env.rack_tally);
                (env.found_exchange_move)(env.exchange_buffer, leave_value);
            }
            if env.exchange_buffer.len() < env.max_vec_len {
                for tile in tile_offset..env.rack_tally_len {
                    let tile_usize = tile as usize;
                    if env.rack_tally[tile_usize] > 0 {
                        env.rack_tally[tile_usize] -= 1;
                        env.exchange_buffer.push(tile);
                        generate_exchanges(env, tile);
                        env.exchange_buffer.pop();
                        env.rack_tally[tile_usize] += 1;
                    }
                }
            }
        }
        generate_exchanges(
            &mut ExchangeEnv {
                rack_tally_len: rack_tally.len() as u8,
                klv,
                found_exchange_move,
                rack_tally,
                exchange_buffer,
                max_vec_len,
            },
            0,
        );
    }

    #[inline(always)]
    pub fn pass_leave_idx(&self) -> u32 {
        if self.leave_values.is_empty() {
            0
        } else {
            self.leave_values.len() as u32 - 1
        }
    }

    // undefined behavior unless tile was init'ed.
    #[inline(always)]
    pub fn place_value(&self, tile: u8) -> u32 {
        self.digits[tile as usize].place_value
    }

    #[inline(always)]
    pub fn leave_value(&self, idx: u32) -> i32 {
        if self.leave_values.is_empty() {
            0
        } else {
            self.leave_values[idx as usize]
        }
    }

    #[inline(always)]
    pub fn pass_leave_value(&self) -> i32 {
        if self.leave_values.is_empty() {
            0 // fallback; caller should use klv.leave_value_from_tally
        } else {
            *self.leave_values.last().unwrap()
        }
    }
}

#[cfg(test)]
#[inline]
pub(crate) fn make_klv2(leaves: &[(&[u8], f32)]) -> Vec<u8> {
    use super::build;
    let words = leaves
        .iter()
        .map(|&(tiles, _)| tiles.into())
        .collect::<Box<[bites::Bites]>>();
    let leaves_kwg = build::build(
        build::BuildContent::DawgOnly,
        build::BuildLayout::Wolges,
        build::BuildOrder::Sorted,
        &words,
    )
    .unwrap();
    let mut bin = Vec::new();
    bin.extend_from_slice(&((leaves_kwg.len() / 4) as u32).to_le_bytes());
    bin.extend_from_slice(&leaves_kwg);
    bin.extend_from_slice(&(leaves.len() as u32).to_le_bytes());
    for &(_, value) in leaves {
        bin.extend_from_slice(&value.to_le_bytes());
    }
    bin
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[inline]
    fn read_leaves_alloc_pairs_each_value_with_its_own_leave() {
        let leaves: &[(&[u8], f32)] = &[
            (&[1], -1.25),
            (&[1, 2], 27.636),
            (&[1, 2, 2], 0.0),
            (&[2], 3.5),
        ];
        let got = read_leaves_alloc::<kwg::Node22>(&make_klv2(leaves)).unwrap();
        assert_eq!(got.len(), leaves.len());
        for (i, &(tiles, value)) in leaves.iter().enumerate() {
            assert_eq!(&got[i].0[..], tiles, "leave {i}");
            assert_eq!(got[i].1, value, "value {i}");
        }
    }

    #[test]
    #[inline]
    fn a_table_repacks_into_either_layout_with_its_leaves_and_values() {
        let leaves: &[(&[u8], f32)] = &[(&[1], -1.25), (&[1, 2], 27.636), (&[2], 3.5)];
        let bin = make_klv2(leaves);
        let mut repacked = bin.clone();
        repack_in_place::<kwg::Node22, kwg::Node24>(&mut repacked).unwrap();
        let got = read_leaves_alloc::<kwg::Node24>(&repacked).unwrap();
        for (i, &(tiles, value)) in leaves.iter().enumerate() {
            assert_eq!(&got[i].0[..], tiles, "leave {i}");
            assert_eq!(got[i].1, value, "value {i}");
        }
        repack_in_place::<kwg::Node24, kwg::Node22>(&mut repacked).unwrap();
        assert_eq!(repacked, bin);
    }

    #[test]
    #[inline]
    fn a_value_is_written_in_full_with_a_point() {
        assert_eq!(value_text(8.344_65e-7_f32), "0.000000834465");
        assert_eq!(value_text(-1.5e-5_f32), "-0.000015");
        assert_eq!(value_text(100.0_f32), "100.0");
        assert_eq!(value_text(-2.025_417_3_f32), "-2.0254173");
        assert_eq!(value_text(0.0_f32), "0.0");
        assert_eq!(value_text(2_147_483.5_f32), "2147483.5");
        // exactly halfway between two shortest forms: the even one
        assert_eq!(value_text(5517.0_f32 / 128.0), "43.101562");
        assert_eq!(value_text(1e16_f64), "10000000000000000.0");
        assert_eq!(
            value_text(-3.4e38_f32),
            "-340000000000000000000000000000000000000.0"
        );
        assert_eq!(value_text(0.1_f64), "0.1");
        assert_eq!(value_text(-3.0_f64), "-3.0");
        assert_eq!(value_text(f32::NAN), "NaN");
        assert_eq!(value_text(f32::NEG_INFINITY), "-inf");
        // each reads back as the same number
        for v in [
            8.344_65e-7_f32,
            27.360_924,
            -2.025_417_3,
            1e-30,
            3.4e38,
            5517.0 / 128.0,
        ] {
            assert_eq!(value_text(v).parse::<f32>().unwrap(), v, "{v:e}");
        }
    }

    #[test]
    #[inline]
    fn a_file_loads_into_kbwgs_layout() {
        let bin = make_klv2(&[(&[1], 1.0), (&[1, 2], 2.0), (&[2], 3.0)]);
        let klv = kbwg_from_bytes_alloc(bin.clone()).unwrap();
        assert_eq!(klv.leave_value_from_tally(&[0, 1, 1]), 2 * equity::SCALE);
        let klv22 = Klv::<kwg::Node22>::from_bytes_alloc(&bin).unwrap();
        assert_eq!(klv.leave_range(), klv22.leave_range());
        assert_eq!(klv.max_tile(), klv22.max_tile());
        let empty = kbwg_from_bytes_alloc(EMPTY_KLV_BYTES.to_vec()).unwrap();
        assert_eq!(empty.leave_range(), (0, 0));
        assert_eq!(empty.leave_value_from_tally(&[0, 1, 1]), 0);
        // the empty table, made in either layout, is the file's
        let made = Klv::<kwg::Node24>::empty().unwrap();
        assert_eq!(made.leave_range(), (0, 0));
        assert_eq!(made.leave_value_from_tally(&[0, 1, 1]), 0);
        assert_eq!(made.max_tile(), empty.max_tile());
        use kwg::Node;
        let made22 = Klv::<kwg::Node22>::empty().unwrap();
        let file22 = Klv::<kwg::Node22>::from_bytes_alloc(EMPTY_KLV_BYTES).unwrap();
        assert_eq!(made22.kwg.0.len(), file22.kwg.0.len());
        for i in 0..made22.kwg.0.len() as i32 {
            let (a, b) = (made22.kwg(i), file22.kwg(i));
            assert_eq!(
                (a.tile(), a.accepts(), a.is_end(), a.arc_index()),
                (b.tile(), b.accepts(), b.is_end(), b.arc_index()),
                "node {i}"
            );
        }
    }

    #[test]
    #[inline]
    fn a_table_read_a_piece_at_a_time_holds_each_value_at_its_leave() {
        let leaves: &[(&[u8], f32)] = &[
            (&[1], -1.25),
            (&[1, 2], 27.636),
            (&[1, 2, 2], 0.0),
            (&[2], 3.5),
            (&[2, 3], 1e-3),
            (&[3], -0.5),
        ];
        let klv2 = make_klv2(leaves);
        // the same leaves with their values as a klv's i16s, 1/256 of a point
        let values_at = 8 + 4 * kwg::read_le_u32(&klv2, 0) as usize;
        let mut klv1 = klv2[..values_at].to_vec();
        for &(_, value) in leaves {
            klv1.extend_from_slice(&((value * 256.0).round() as i16).to_le_bytes());
        }
        for (bin, as_i16) in [(&klv2, false), (&klv1, true)] {
            for piece_len in [4, 8, 12, 20, bin.len(), kwg::READ_PIECE] {
                let mut read = |at: usize, piece: &mut [u8]| {
                    piece.copy_from_slice(&bin[at..at + piece.len()]);
                };
                let klv =
                    Klv::<kwg::Node24>::from_pieces::<kwg::Node22>(bin.len(), piece_len, &mut read)
                        .unwrap();
                let mut tally = [0u8; 4];
                for &(tiles, value) in leaves {
                    tally.fill(0);
                    for &tile in tiles {
                        tally[tile as usize] += 1;
                    }
                    let want = if as_i16 {
                        ((value * 256.0).round() as i16 as i32 * equity::SCALE + 128) / 256
                    } else {
                        (value * equity::SCALE as f32).round() as i32
                    };
                    assert_eq!(
                        klv.leave_value_from_tally(&tally),
                        want,
                        "{tiles:?} in pieces of {piece_len}"
                    );
                }
            }
        }

        // a whole read's checks: cut short, or a value no leave can hold
        let mut read = |at: usize, piece: &mut [u8]| {
            piece.copy_from_slice(&klv2[at..at + piece.len()]);
        };
        let short = klv2.len() - 4;
        assert!(Klv::<kwg::Node24>::from_pieces::<kwg::Node22>(short, 8, &mut read).is_err());
        let mut nan = klv2.clone();
        let last = nan.len() - 4;
        nan[last..].copy_from_slice(&f32::NAN.to_le_bytes());
        let mut read = |at: usize, piece: &mut [u8]| {
            piece.copy_from_slice(&nan[at..at + piece.len()]);
        };
        assert!(Klv::<kwg::Node24>::from_pieces::<kwg::Node22>(nan.len(), 8, &mut read).is_err());
    }

    #[test]
    #[inline]
    fn read_leaves_alloc_reads_an_empty_table() {
        assert!(
            read_leaves_alloc::<kwg::Node22>(EMPTY_KLV_BYTES)
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    #[inline]
    fn read_leaves_alloc_rejects_a_table_that_fits_neither_width() {
        let mut bin = make_klv2(&[(&[1], 1.0), (&[2], 2.0)]);
        bin.truncate(bin.len() - 2);
        assert!(read_leaves_alloc::<kwg::Node22>(&bin).is_err());
    }

    #[test]
    #[inline]
    fn a_klv_loads_only_with_one_value_for_each_leave() {
        let bin = make_klv2(&[(&[1], 1.0), (&[1, 2], 2.0), (&[2], 3.0)]);
        let klv = Klv::<kwg::Node22>::from_bytes_alloc(&bin).unwrap();
        assert_eq!(klv.leave_value_from_tally(&[0, 1, 1]), 2 * equity::SCALE);

        assert!(Klv::<kwg::Node22>::from_bytes_alloc(&[]).is_err());
        assert!(Klv::<kwg::Node22>::from_bytes_alloc(&bin[..4]).is_err());
        assert!(Klv::<kwg::Node22>::from_bytes_alloc(&bin[..8]).is_err());
        let mut long = bin.clone();
        long.push(0);
        assert!(Klv::<kwg::Node22>::from_bytes_alloc(&long).is_err());

        // two values for three leaves
        let lv_len_at = 4 + kwg::read_le_u32(&bin, 0) as usize * 4;
        let mut short = bin[..bin.len() - 4].to_vec();
        short[lv_len_at..lv_len_at + 4].copy_from_slice(&2u32.to_le_bytes());
        assert!(Klv::<kwg::Node22>::from_bytes_alloc(&short).is_err());
        // a graph too long for the file
        let mut cut = bin.clone();
        cut[..4].copy_from_slice(&u32::MAX.to_le_bytes());
        assert!(Klv::<kwg::Node22>::from_bytes_alloc(&cut).is_err());
    }

    #[test]
    #[inline]
    fn a_klv_refuses_a_value_no_leave_can_hold() {
        // i32::MAX millipoints is 2147483.647 points
        for bad in [
            f32::NAN,
            f32::INFINITY,
            f32::NEG_INFINITY,
            2_200_000.0,
            -2_200_000.0,
        ] {
            let bin = make_klv2(&[(&[1], 1.0), (&[2], bad)]);
            assert!(Klv::<kwg::Node22>::from_bytes_alloc(&bin).is_err(), "{bad}");
        }
        let bin = make_klv2(&[(&[1], 2_100_000.0), (&[2], -2_100_000.0)]);
        let klv = Klv::<kwg::Node22>::from_bytes_alloc(&bin).unwrap();
        assert_eq!(klv.leave_range(), (-2_100_000_000, 2_100_000_000));

        // read_leaves_alloc refuses the same, and hands the rest on as they are
        for bad in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY, 2_200_000.0] {
            let bin = make_klv2(&[(&[1], 1.0), (&[2], bad)]);
            assert!(read_leaves_alloc::<kwg::Node22>(&bin).is_err(), "{bad}");
        }
        let bin = make_klv2(&[(&[1], 2_147_483.5)]);
        assert_eq!(
            read_leaves_alloc::<kwg::Node22>(&bin).unwrap()[0].1,
            2_147_483.5
        );
    }

    #[test]
    #[inline]
    fn a_klv_whose_leaves_loop_is_an_error() {
        let bin = make_klv2(&[(&[1], 1.0), (&[1, 2], 2.0), (&[2], 3.0)]);
        // the first tile of the leaves leads back to the run it starts
        let root = kwg::read_le_u32(&bin, 4) & 0x3fffff;
        let at = 4 + root as usize * 4;
        let mut looping = bin.clone();
        looping[at..at + 4]
            .copy_from_slice(&((kwg::read_le_u32(&bin, at) & !0x3fffff) | root).to_le_bytes());
        assert!(Klv::<kwg::Node22>::from_bytes_alloc(&looping).is_err());
        assert!(read_leaves_alloc::<kwg::Node22>(&looping).is_err());
    }

    #[inline]
    fn binom(n: u64, k: u64) -> u64 {
        if k > n {
            return 0;
        }
        let k = k.min(n - k);
        let mut num = 1u128;
        let mut den = 1u128;
        for i in 0..k {
            num *= (n - i) as u128;
            den *= (i + 1) as u128;
        }
        (num / den) as u64
    }

    #[test]
    #[inline]
    fn apply_dynamic_leaves_matches_brute() {
        let num_letters = 3usize;
        let rack_size = 3usize;
        let lat = census::MultisetLattice::new(num_letters, rack_size).unwrap();
        let add = census::AddTable::new(&lat);

        let mut full_v = vec![0i32; lat.len()];
        for (idx, slot) in full_v.iter_mut().enumerate() {
            let h = (idx as i32).wrapping_mul(2654435761u32 as i32);
            *slot = h.rem_euclid(20_000) - 5_000;
        }

        let statics: Vec<i32> = (0..6i32).map(|i| -100 - i).collect();
        let mut ml = MultiLeaves {
            unique_tiles: vec![0u8, 1u8],
            digits: vec![
                MultiLeavesDigit {
                    count: 2,
                    place_value: 1,
                },
                MultiLeavesDigit {
                    count: 1,
                    place_value: 3,
                },
                MultiLeavesDigit {
                    count: 0,
                    place_value: 0,
                },
            ],
            leave_values: statics.clone(),
            num_playeds: vec![3, 2, 1, 2, 1, 0],
            dense_most: DENSE_LEAVES_MOST,
        };

        let live_pool = [2u8, 2u8, 1u8]; // A, B, C still drawable
        let min_keep = 1usize;
        let replacing = DynamicLeavesRef {
            lat: &lat,
            add: &add,
            full_v: &full_v,
            min_keep,
        };
        let bag_size = 5usize;
        ml.apply_dynamic_leaves(&replacing, &live_pool, bag_size);

        for (idx, &static_v) in statics.iter().enumerate() {
            let kept_a = (idx as u32 % 3) as u8;
            let kept_b = (idx as u32 / 3) as u8;
            let s_size = (kept_a + kept_b) as usize;
            if s_size < min_keep {
                assert_eq!(
                    ml.leave_values[idx], static_v,
                    "idx {idx} should stay static"
                );
                continue;
            }
            let draw = rack_size - s_size;

            let mut num = 0f64;
            let mut den = 0f64;
            for da in 0..=live_pool[0] {
                for db in 0..=live_pool[1] {
                    for dc in 0..=live_pool[2] {
                        if (da + db + dc) as usize != draw {
                            continue;
                        }
                        let ways = binom(live_pool[0] as u64, da as u64)
                            * binom(live_pool[1] as u64, db as u64)
                            * binom(live_pool[2] as u64, dc as u64);
                        if ways == 0 {
                            continue;
                        }
                        let r = [kept_a + da, kept_b + db, dc];
                        let ri = lat.rank(&r) as usize;
                        num += ways as f64 * full_v[ri] as f64;
                        den += ways as f64;
                    }
                }
            }
            let want = (num / den) as i32;
            assert_eq!(
                ml.leave_values[idx], want,
                "idx {idx} (keep {kept_a}A {kept_b}B)"
            );
        }

        let empty_ridx = lat.rank(&[0u8, 0, 0]) as usize;
        assert_eq!(
            census::dynamic_leave_value(&lat, &add, &full_v, &[0u8, 0, 0], empty_ridx, 3),
            census::UNPLAYABLE,
        );

        let mut short_bag = MultiLeaves {
            unique_tiles: ml.unique_tiles.clone(),
            digits: ml.digits.clone(),
            leave_values: statics.clone(),
            num_playeds: ml.num_playeds.clone(),
            dense_most: DENSE_LEAVES_MOST,
        };
        short_bag.apply_dynamic_leaves(&replacing, &live_pool, 1);
        let keep_one_a = 1usize; // idx = keptA + 3 * keptB
        let (mut num, mut den) = (0f64, 0f64);
        for (t, &avail) in live_pool.iter().enumerate() {
            if avail == 0 {
                continue;
            }
            let mut r = [1u8, 0, 0];
            r[t] += 1;
            num += avail as f64 * full_v[lat.rank(&r) as usize] as f64;
            den += avail as f64;
        }
        assert_eq!(
            short_bag.leave_values[keep_one_a],
            (num / den) as i32,
            "one tile in the bag draws one tile, not two"
        );
    }

    #[test]
    #[inline]
    fn a_klv_knows_its_lowest_and_highest_leave() {
        let empty = Klv::<kwg::Node22>::from_bytes_alloc(EMPTY_KLV_BYTES).unwrap();
        assert_eq!(empty.leave_range(), (0, 0));
        let klv = Klv::<kwg::Node22> {
            kwg: empty.kwg,
            counts: empty.counts,
            leaves: Box::new([-2_500, 700, 12_345, -9]),
            racks: None,
        };
        assert_eq!(klv.leave_range(), (-2_500, 12_345));
    }
}
