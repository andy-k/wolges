// Copyright (C) 2020-2026 Andy Kurnia.

use super::error;

pub trait Node: Copy {
    fn new(v: u32) -> Self;
    fn tile(&self) -> u8;
    fn accepts(&self) -> bool;
    fn is_end(&self) -> bool;
    fn arc_index(&self) -> i32;
    // the word a node with these fields is in this layout, or None where one of
    // them does not fit it
    fn pack(tile: u8, accepts: bool, is_end: bool, arc_index: i32) -> Option<u32>;
}

#[derive(Clone, Copy)]
pub struct Node22(u32);

impl Node for Node22 {
    #[inline(always)]
    fn new(v: u32) -> Self {
        Self(v)
    }

    #[inline(always)]
    fn tile(&self) -> u8 {
        (self.0 >> 24) as u8
    }

    #[inline(always)]
    fn accepts(&self) -> bool {
        self.0 & 0x800000 != 0
    }

    #[inline(always)]
    fn is_end(&self) -> bool {
        self.0 & 0x400000 != 0
    }

    #[inline(always)]
    fn arc_index(&self) -> i32 {
        (self.0 & 0x3fffff) as i32
    }

    #[inline(always)]
    fn pack(tile: u8, accepts: bool, is_end: bool, arc_index: i32) -> Option<u32> {
        (0..=0x3fffff).contains(&arc_index).then_some(
            (tile as u32) << 24 | (accepts as u32) << 23 | (is_end as u32) << 22 | arc_index as u32,
        )
    }
}

#[derive(Clone, Copy)]
pub struct Node24(u32);

impl Node for Node24 {
    #[inline(always)]
    fn new(v: u32) -> Self {
        Self(v)
    }

    #[inline(always)]
    fn tile(&self) -> u8 {
        (self.0 & 0x3f) as u8
    }

    #[inline(always)]
    fn accepts(&self) -> bool {
        self.0 & 0x80 != 0
    }

    #[inline(always)]
    fn is_end(&self) -> bool {
        self.0 & 0x40 != 0
    }

    #[inline(always)]
    fn arc_index(&self) -> i32 {
        (self.0 >> 8) as i32
    }

    #[inline(always)]
    fn pack(tile: u8, accepts: bool, is_end: bool, arc_index: i32) -> Option<u32> {
        (tile < 0x40 && (0..=0xffffff).contains(&arc_index)).then_some(
            (arc_index as u32) << 8 | (accepts as u32) << 7 | (is_end as u32) << 6 | tile as u32,
        )
    }
}

// a node of one layout in the other, the same tile, flags, and arc; refused
// where the new layout cannot hold it
#[inline(always)]
fn repack_node<Old: Node, New: Node>(node: u32) -> error::Returns<u32> {
    let old = Old::new(node);
    Ok(
        New::pack(old.tile(), old.accepts(), old.is_end(), old.arc_index()).ok_or_else(|| {
            format!(
                "a node of tile {} and arc {} does not fit the new layout",
                old.tile(),
                old.arc_index()
            )
        })?,
    )
}

// a graph's nodes rewritten in place from one layout to the other, the same
// tile, flags, and arc in each; a node the new layout cannot hold stops it with
// the nodes before it rewritten. A part node at the end is left as it is, for
// from_bytes_alloc to refuse.
#[inline(always)]
pub fn repack_in_place<Old: Node, New: Node>(buf: &mut [u8]) -> error::Returns<()> {
    for node in buf.as_chunks_mut::<4>().0 {
        *node = repack_node::<Old, New>(u32::from_le_bytes(*node))?.to_le_bytes();
    }
    Ok(())
}

// a word graph's file read into kbwg's layout, the one layout the engine works
// in: a kwg's nodes are repacked as it loads, a kbwg's are taken as they are
#[inline]
pub fn kbwg_from_bytes_alloc(buf: Vec<u8>, is_kbwg: bool) -> error::Returns<Kwg<Node24>> {
    let read = |at: usize, piece: &mut [u8]| piece.copy_from_slice(&buf[at..at + piece.len()]);
    if is_kbwg {
        Kwg::from_reader::<Node24>(buf.len(), read)
    } else {
        Kwg::from_reader::<Node22>(buf.len(), read)
    }
}

// the bytes of a file a graph or leave table is read from at a time, a whole
// number of nodes, so that the file is never held whole beside what it makes
pub(crate) const READ_PIECE: usize = 1 << 16;

#[inline(always)]
pub fn read_le_u16(bytes: &[u8], p: usize) -> u16 {
    bytes[p] as u16 | ((bytes[p + 1] as u16) << 8)
}

#[inline(always)]
pub fn read_le_u32(bytes: &[u8], p: usize) -> u32 {
    bytes[p] as u32
        | ((bytes[p + 1] as u32) << 8)
        | ((bytes[p + 2] as u32) << 16)
        | ((bytes[p + 3] as u32) << 24)
}

pub struct Kwg<N: Node>(pub Box<[N]>);

pub struct LetterBits {
    pub letter_bits: Box<[u64]>,
    pub accepting_bits: Box<[u64]>,
}

// kwg::Node22
pub static EMPTY_KWG_BYTES: &[u8] = b"\x00\x00\x40\x00\x00\x00\x40\x00";

pub static EMPTY_KBWG_BYTES: &[u8] = b"\x40\x00\x00\x00\x40\x00\x00\x00";

impl<N: Node> std::ops::Index<i32> for Kwg<N> {
    type Output = N;

    #[inline(always)]
    fn index(&self, i: i32) -> &N {
        &self.0[i as usize]
    }
}

impl<N: Node> Kwg<N> {
    #[inline(always)]
    pub fn from_bytes_alloc(buf: &[u8]) -> error::Returns<Self> {
        Self::from_reader::<N>(buf.len(), |at, piece| {
            piece.copy_from_slice(&buf[at..at + piece.len()])
        })
    }

    // a graph read from a file of len bytes whose nodes are in Old's layout, a
    // piece at a time, each node put in this layout as it comes, so that the
    // file's bytes are never held whole beside the nodes: read(at, piece) fills
    // piece with the file's bytes from at on
    #[inline]
    pub fn from_reader<Old: Node>(
        len: usize,
        mut read: impl FnMut(usize, &mut [u8]),
    ) -> error::Returns<Self> {
        Self::from_pieces::<Old>(0, len, READ_PIECE, &mut read)
    }

    // the same from the len bytes at base on, in pieces of piece_len, a whole
    // number of nodes. every arc must land inside the graph and the last node
    // must end its run, so no walk leaves it. an empty lexicon's dawg is one
    // node; its copy stands in for the empty gaddag a classic game enters at
    // node 1.
    #[inline]
    pub(crate) fn from_pieces<Old: Node>(
        base: usize,
        len: usize,
        piece_len: usize,
        read: &mut impl FnMut(usize, &mut [u8]),
    ) -> error::Returns<Self> {
        let kwg_len = len / 4;
        if kwg_len == 0 || !len.is_multiple_of(4) {
            return Err(format!("a word graph of {len} bytes is not whole nodes").into());
        }
        let mut elts = Vec::with_capacity(kwg_len.max(2));
        let mut buf = vec![0u8; piece_len.min(len)];
        let mut at = 0;
        while at < len {
            let piece = &mut buf[..piece_len.min(len - at)];
            read(base + at, piece);
            for node in piece.as_chunks::<4>().0 {
                elts.push(N::new(repack_node::<Old, N>(u32::from_le_bytes(*node))?));
            }
            at += piece.len();
        }
        // the largest arc, found in one pass with no early exit
        let max_arc_index = elts.iter().fold(0, |m, node| m.max(node.arc_index()));
        if max_arc_index as usize >= kwg_len {
            return Err(format!(
                "the word graph points to node {max_arc_index}, past its {kwg_len} nodes"
            )
            .into());
        }
        if !elts[kwg_len - 1].is_end() {
            return Err("the word graph's last node does not end its run".into());
        }
        if kwg_len == 1 {
            elts.push(elts[0]);
        }
        Ok(Kwg(elts.into_boxed_slice()))
    }

    #[inline(always)]
    pub fn seek(&self, mut p: i32, tile: u8) -> i32 {
        if p >= 0 {
            p = self[p].arc_index();
            if p > 0 {
                loop {
                    let node = self[p];
                    if node.tile() == tile {
                        return p;
                    }
                    if node.is_end() {
                        return -1;
                    }
                    p += 1;
                }
            }
        }
        -1 // intentionally return 0 as -1
    }

    #[inline(always)]
    pub fn prefetch(&self, p: i32) {
        if p > 0 {
            let ptr = self.0.as_ptr().wrapping_add(p as usize);
            #[cfg(target_arch = "x86_64")]
            // Safety: a prefetch only hints; it reads nothing the program sees
            // and never faults, whatever the address (wrapping_add makes no
            // reference).
            unsafe {
                std::arch::x86_64::_mm_prefetch::<{ std::arch::x86_64::_MM_HINT_T0 }>(
                    ptr as *const i8,
                );
            }
            #[cfg(target_arch = "aarch64")]
            // Safety: prfm only hints and never faults, whatever the address;
            // the asm writes no register or memory and keeps the flags, as its
            // options say.
            unsafe {
                std::arch::asm!(
                    "prfm pldl1keep, [{ptr}]",
                    ptr = in(reg) ptr,
                    options(nostack, preserves_flags, readonly),
                );
            }
            #[cfg(not(any(target_arch = "x86_64", target_arch = "aarch64")))]
            let _ = ptr;
        }
    }

    fn count_words_at(&self, word_counts: &mut [u32], p: i32) -> error::Returns<u32> {
        if p as usize >= word_counts.len() {
            return Ok(0);
        }
        if word_counts[p as usize] == !0 {
            return Err(format!("the word graph loops back to node {p}").into());
        };
        if word_counts[p as usize] == 0 {
            word_counts[p as usize] = !0; // marker
            let node = self[p];
            let count = node.accepts() as u64
                + if node.arc_index() != 0 {
                    self.count_words_at(word_counts, node.arc_index())? as u64
                } else {
                    0
                }
                + if node.is_end() {
                    0
                } else {
                    self.count_words_at(word_counts, p + 1)? as u64
                };
            if count >= !0u32 as u64 {
                return Err("the word graph holds too many words to count".into());
            }
            word_counts[p as usize] = count as u32;
        }
        Ok(word_counts[p as usize])
    }

    #[inline(always)]
    pub fn count_words_alloc(&self) -> error::Returns<Box<[u32]>> {
        let mut word_counts = vec![0u32; self.0.len()];
        for p in (0..word_counts.len()).rev() {
            self.count_words_at(&mut word_counts, p as i32)?;
        }
        Ok(word_counts.into_boxed_slice())
    }

    #[inline(always)]
    pub fn fits_alphabet(&self, alphabet_len: u8) -> bool {
        self.0.iter().all(|node| node.tile() < alphabet_len)
    }

    // whether every run's tiles ascend, each above the one before, as the
    // graphs built here do: then a tile's node in a run is found by counting
    // the run's tiles below it. A node of no tile, flag, or arc after a run's
    // end is a gap the layout left between runs, part of none
    #[inline(always)]
    pub fn runs_ascend(&self) -> bool {
        self.0.windows(2).all(|pair| {
            let gap = pair[0].tile() == 0
                && !pair[0].accepts()
                && !pair[0].is_end()
                && pair[0].arc_index() == 0;
            pair[0].is_end() || gap || pair[0].tile() < pair[1].tile()
        })
    }

    // by node, the tiles from it to the end of its run: compute_letter_bits's
    // letter_bits, without the accepting bits
    #[inline(always)]
    pub fn run_letter_bits(&self) -> Box<[u64]> {
        let mut letter_bits = vec![0u64; self.0.len()].into_boxed_slice();
        let mut all_acc = 0u64;
        for i in (0..self.0.len()).rev() {
            let node = self.0[i];
            if node.is_end() {
                all_acc = 0;
            }
            all_acc |= 1u64 << node.tile();
            letter_bits[i] = all_acc;
        }
        letter_bits
    }

    #[inline(always)]
    pub fn compute_letter_bits(&self) -> LetterBits {
        let len = self.0.len();
        let mut letter_bits = vec![0u64; len];
        let mut accepting_bits = vec![0u64; len];
        let mut all_acc = 0u64;
        let mut accept_acc = 0u64;
        for i in (0..len).rev() {
            let node = self.0[i];

            if node.is_end() {
                all_acc = 0;
                accept_acc = 0;
            }
            let tile_bit = 1u64 << node.tile();
            all_acc |= tile_bit;
            accept_acc |= tile_bit & (-i64::from(node.accepts())) as u64;
            letter_bits[i] = all_acc;
            accepting_bits[i] = accept_acc;
        }
        LetterBits {
            letter_bits: letter_bits.into_boxed_slice(),
            accepting_bits: accepting_bits.into_boxed_slice(),
        }
    }

    #[inline(always)]
    pub fn count_dawg_words_alloc(&self) -> error::Returns<Box<[u32]>> {
        fn max_from<N: Node>(nodes: &Kwg<N>, vis: &mut [u8], mut p: i32) -> i32 {
            let mut ret = 0;
            loop {
                let p_byte_index = (p as usize) / 8;
                let p_bit = 1 << (p as usize % 8);
                if vis[p_byte_index] & p_bit != 0 {
                    break;
                }
                vis[p_byte_index] |= p_bit;
                if nodes[p].arc_index() != 0 {
                    ret = ret.max(max_from(nodes, vis, nodes[p].arc_index()));
                }
                if nodes[p].is_end() {
                    break;
                }
                p += 1;
            }
            ret.max(p)
        }
        let required_size =
            max_from(self, &mut vec![0u8; self.0.len().div_ceil(8)], 0) as usize + 1;
        let mut word_counts = vec![0u32; required_size];
        for p in (0..word_counts.len()).rev() {
            self.count_words_at(&mut word_counts, p as i32)?;
        }
        Ok(word_counts.into_boxed_slice())
    }

    #[inline(always)]
    pub fn get_word_by_index<F: FnMut(u8)>(
        &self,
        word_counts: &[u32],
        mut p: i32,
        mut idx: u32,
        mut out: F,
    ) -> error::Returns<()> {
        let mut node = self[p];
        while !(idx == 0 && node.accepts()) {
            let words_here = if node.is_end() {
                word_counts[p as usize]
            } else {
                word_counts[p as usize] - word_counts[p as usize + 1]
            };
            if idx < words_here {
                idx -= node.accepts() as u32;
                out(node.tile());
                p = node.arc_index();
            } else {
                idx -= words_here;
                if node.is_end() {
                    return Err("the word graph has fewer words than the index".into());
                }
                p += 1;
            }
            node = self[p];
        }
        out(node.tile());
        Ok(())
    }

    #[inline(always)]
    pub fn get_word_index(&self, word_counts: &[u32], mut p: i32, word: &[u8]) -> u32 {
        let mut idx = 0;
        for (remaining, &tile) in (0..word.len()).rev().zip(word) {
            if p == 0 {
                return !0;
            }
            let mut node = self[p];
            idx += word_counts[p as usize];
            while node.tile() != tile {
                if node.is_end() {
                    return !0;
                }
                p += 1;
                node = self[p];
            }
            idx -= word_counts[p as usize];
            if remaining == 0 {
                return idx | ((node.accepts() as i32 - 1) as u32);
            }
            idx += node.accepts() as u32;
            p = node.arc_index();
        }
        !0
    }

    // slower than just using the index
    #[inline(always)]
    pub fn get_word_index_of<I: Iterator<Item = u8>>(
        &self,
        word_counts: &[u32],
        mut p: i32,
        iter: &mut I,
    ) -> u32 {
        let mut idx = 0;
        if let Some(mut tile) = iter.next() {
            while p != 0 {
                let mut node = self[p];
                idx += word_counts[p as usize];
                while node.tile() != tile {
                    if node.is_end() {
                        return !0;
                    }
                    p += 1;
                    node = self[p];
                }
                idx -= word_counts[p as usize];
                match iter.next() {
                    Some(t) => {
                        tile = t;
                    }
                    None => {
                        return idx | ((node.accepts() as i32 - 1) as u32);
                    }
                }
                idx += node.accepts() as u32;
                p = node.arc_index();
            }
        }
        !0
    }

    #[inline(always)]
    fn completes_alpha_cross_set(&self, mut p: i32, letters_tally: &[u8], next_letter: u8) -> bool {
        for letter in next_letter..letters_tally.len() as u8 {
            for _ in 0..letters_tally[letter as usize] {
                p = self.seek(p, letter);
                if p <= 0 {
                    return false;
                }
            }
        }
        self[p].accepts()
    }

    #[inline(always)]
    pub fn accepts_alpha(&self, letters_tally: &[u8]) -> bool {
        self.completes_alpha_cross_set(0, letters_tally, 1)
    }

    #[inline(always)]
    pub fn compute_alpha_cross_set(&self, letters_tally: &[u8]) -> u64 {
        let mut answer = 1; // always set bit 0 here
        let mut p = self[0].arc_index();
        if p <= 0 {
            return answer;
        }
        let letters_tally_len = letters_tally.len() as u8;
        for letter in 1..letters_tally_len {
            // 0 should be unused
            for _ in 0..letters_tally[letter as usize] {
                loop {
                    let node = self[p];
                    let tile = node.tile();
                    match tile.cmp(&letter) {
                        std::cmp::Ordering::Greater => {
                            return answer;
                        }
                        std::cmp::Ordering::Less => {
                            if self.completes_alpha_cross_set(p, letters_tally, letter) {
                                answer |= 1 << tile;
                            }
                            if node.is_end() {
                                return answer;
                            }
                            p += 1;
                        }
                        std::cmp::Ordering::Equal => {
                            let next_p = node.arc_index();
                            if next_p <= 0 {
                                return answer;
                            }
                            p = next_p;
                            break;
                        }
                    }
                }
            }
        }
        loop {
            let node = self[p];
            if self.completes_alpha_cross_set(p, letters_tally, letters_tally_len) {
                answer |= 1 << node.tile();
            }
            if node.is_end() {
                break;
            }
            p += 1;
        }
        answer
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{alphabet, bites, build, game_config};

    #[inline]
    fn bytes_of(gc: &game_config::GameConfig, words: &[&str]) -> Vec<u8> {
        let reader = alphabet::AlphabetReader::new_for_words(gc.alphabet());
        let mut buf = Vec::new();
        let mut out = Vec::<bites::Bites>::with_capacity(words.len());
        for w in words {
            reader.set_word(w, &mut buf).unwrap();
            out.push(buf[..].into());
        }
        out.sort_unstable();
        build::build(
            build::BuildContent::Gaddawg,
            build::BuildLayout::Wolges,
            build::BuildOrder::Sorted,
            &out,
        )
        .unwrap()
        .to_vec()
    }

    #[inline]
    fn graph_of(gc: &game_config::GameConfig, words: &[&str]) -> Kwg<Node22> {
        Kwg::<Node22>::from_bytes_alloc(&bytes_of(gc, words)).unwrap()
    }

    #[test]
    #[inline]
    fn a_graph_loads_only_whole_nodes_that_stay_inside_it() {
        let bytes = bytes_of(&game_config::make_english_game_config(), &["SEAT", "TEA"]);
        assert!(Kwg::<Node22>::from_bytes_alloc(&bytes).is_ok());
        assert!(Kwg::<Node22>::from_bytes_alloc(&[]).is_err());
        assert!(Kwg::<Node22>::from_bytes_alloc(&bytes[..3]).is_err());
        assert!(Kwg::<Node22>::from_bytes_alloc(&bytes[..bytes.len() - 1]).is_err());

        let num_nodes = (bytes.len() / 4) as u32;
        let with_root_arc = |arc: u32| {
            let mut v = bytes.clone();
            v[..4].copy_from_slice(&((read_le_u32(&bytes, 0) & !0x3fffff) | arc).to_le_bytes());
            v
        };
        assert!(Kwg::<Node22>::from_bytes_alloc(&with_root_arc(num_nodes - 1)).is_ok());
        assert!(Kwg::<Node22>::from_bytes_alloc(&with_root_arc(num_nodes)).is_err());
        assert!(Kwg::<Node22>::from_bytes_alloc(&with_root_arc(0x3fffff)).is_err());

        let mut open = bytes.clone();
        let last = bytes.len() - 4;
        open[last..].copy_from_slice(&(read_le_u32(&bytes, last) & !0x400000).to_le_bytes());
        assert!(Kwg::<Node22>::from_bytes_alloc(&open).is_err());
    }

    #[test]
    #[inline]
    fn a_walk_off_the_words_or_round_a_loop_is_an_error() {
        let bytes = bytes_of(&game_config::make_english_game_config(), &["SEAT", "TEA"]);
        let kwg = Kwg::<Node22>::from_bytes_alloc(&bytes).unwrap();
        let counts = kwg.count_words_alloc().unwrap();
        let dawg_root = kwg[0].arc_index();
        let mut word = Vec::new();
        kwg.get_word_by_index(&counts, dawg_root, 1, |t| word.push(t))
            .unwrap();
        assert_eq!(word, [20, 5, 1]); // TEA
        assert!(
            kwg.get_word_by_index(&counts, dawg_root, 2, |_| {})
                .is_err()
        );

        // the S that starts the dawg's first run now leads back to that run
        let mut looping = bytes.clone();
        let at = dawg_root as usize * 4;
        looping[at..at + 4].copy_from_slice(
            &((read_le_u32(&bytes, at) & !0x3fffff) | dawg_root as u32).to_le_bytes(),
        );
        let kwg = Kwg::<Node22>::from_bytes_alloc(&looping).unwrap();
        assert!(kwg.count_words_alloc().is_err());
        assert!(kwg.count_dawg_words_alloc().is_err());
    }

    #[test]
    #[inline]
    fn a_graph_repacks_into_either_layout_and_back() {
        let bytes = bytes_of(&game_config::make_polish_game_config(), &["ZOLZA", "ZABA"]);
        let mut repacked = bytes.clone();
        repack_in_place::<Node22, Node24>(&mut repacked).unwrap();
        let wide = Kwg::<Node24>::from_bytes_alloc(&repacked).unwrap();
        let narrow = Kwg::<Node22>::from_bytes_alloc(&bytes).unwrap();
        assert_eq!(wide.0.len(), narrow.0.len());
        for (w, n) in wide.0.iter().zip(&narrow.0) {
            assert_eq!(
                (w.tile(), w.accepts(), w.is_end(), w.arc_index()),
                (n.tile(), n.accepts(), n.is_end(), n.arc_index())
            );
        }
        repack_in_place::<Node24, Node22>(&mut repacked).unwrap();
        assert_eq!(repacked, bytes);

        // a tile past Node24's six bits, an arc past Node22's 22
        assert!(repack_in_place::<Node22, Node24>(&mut (0x40u32 << 24).to_le_bytes()).is_err());
        assert!(repack_in_place::<Node24, Node22>(&mut (0x400000u32 << 8).to_le_bytes()).is_err());
    }

    #[test]
    #[inline]
    fn a_graph_read_a_piece_at_a_time_is_the_graph_read_whole() {
        let bytes = bytes_of(&game_config::make_polish_game_config(), &["ZOLZA", "ZABA"]);
        let mut repacked = bytes.clone();
        repack_in_place::<Node22, Node24>(&mut repacked).unwrap();
        let whole = Kwg::<Node24>::from_bytes_alloc(&repacked).unwrap();
        for piece_len in [4, 8, 12, bytes.len(), READ_PIECE] {
            let mut reads = Vec::new();
            let pieces = Kwg::<Node24>::from_pieces::<Node22>(
                0,
                bytes.len(),
                piece_len,
                &mut |at, piece: &mut [u8]| {
                    reads.push((at, piece.len()));
                    piece.copy_from_slice(&bytes[at..at + piece.len()]);
                },
            )
            .unwrap();
            assert_eq!(pieces.0.len(), whole.0.len());
            for (p, w) in pieces.0.iter().zip(&whole.0) {
                assert_eq!(
                    (p.tile(), p.accepts(), p.is_end(), p.arc_index()),
                    (w.tile(), w.accepts(), w.is_end(), w.arc_index())
                );
            }
            // each byte read once, in order, in pieces of at most piece_len
            let mut next = 0;
            for &(at, len) in &reads {
                assert_eq!(at, next);
                assert!(len <= piece_len);
                next += len;
            }
            assert_eq!(next, bytes.len());
        }

        // a whole read's checks: part of a node, or an arc out of the graph
        let mut read = |at: usize, piece: &mut [u8]| {
            piece.copy_from_slice(&bytes[at..at + piece.len()]);
        };
        assert!(Kwg::<Node24>::from_pieces::<Node22>(0, bytes.len() - 2, 4, &mut read).is_err());
        let mut out = bytes.clone();
        out[..4].copy_from_slice(
            &((read_le_u32(&bytes, 0) & !0x3fffff) | (bytes.len() / 4) as u32).to_le_bytes(),
        );
        let mut read = |at: usize, piece: &mut [u8]| {
            piece.copy_from_slice(&out[at..at + piece.len()]);
        };
        assert!(Kwg::<Node24>::from_pieces::<Node22>(0, out.len(), 8, &mut read).is_err());
    }

    #[test]
    #[inline]
    fn a_one_node_graph_is_an_empty_dawg_and_an_empty_gaddag() {
        let kwg = Kwg::<Node22>::from_bytes_alloc(b"\x00\x00\x40\x00").unwrap();
        assert_eq!(kwg.0.len(), 2);
        assert_eq!(kwg.seek(0, 1), -1);
        assert_eq!(kwg.seek(1, 1), -1);
    }

    // a built graph's runs ascend, and so does the one-node empty graph; with
    // the first two nodes of one run swapped (each keeping its arc), they do
    // not
    #[test]
    #[inline]
    fn a_built_graph_runs_ascend_and_a_swapped_run_does_not() {
        let gc = game_config::make_english_game_config();
        let kwg = graph_of(&gc, &["SEAT", "TEA", "EATS", "ETAS"]);
        assert!(kwg.runs_ascend());
        let empty = Kwg::<Node22>::from_bytes_alloc(b"\x00\x00\x40\x00").unwrap();
        assert!(empty.runs_ascend());
        let at = (0..kwg.0.len() - 1).find(|&i| !kwg.0[i].is_end()).unwrap();
        let (a, b) = (kwg.0[at], kwg.0[at + 1]);
        let mut nodes = kwg.0.to_vec();
        nodes[at] =
            Node22::new(Node22::pack(b.tile(), b.accepts(), a.is_end(), b.arc_index()).unwrap());
        nodes[at + 1] =
            Node22::new(Node22::pack(a.tile(), a.accepts(), b.is_end(), a.arc_index()).unwrap());
        assert!(!Kwg(nodes.into_boxed_slice()).runs_ascend());
    }

    // gaps between runs, nodes of no tile, flag, or arc that no arc enters, as
    // the layout leaves where a run does not fit, leave a graph's runs
    // ascending and its words the same; past them a swapped run still does not
    // ascend
    #[test]
    #[inline]
    fn gaps_between_runs_leave_them_ascending() {
        let gc = game_config::make_english_game_config();
        let words = ["SEAT", "TEA", "EATS", "ETAS"];
        let kwg = graph_of(&gc, &words);
        let end = (2..kwg.0.len()).find(|&i| kwg.0[i].is_end()).unwrap();
        // two gaps after that run's end, every arc past them moved on
        let mut nodes = (kwg.0.iter())
            .map(|node| {
                let arc = node.arc_index();
                let arc = if arc as usize > end { arc + 2 } else { arc };
                Node22::new(Node22::pack(node.tile(), node.accepts(), node.is_end(), arc).unwrap())
            })
            .collect::<Vec<_>>();
        nodes.splice(end + 1..end + 1, [Node22::new(0), Node22::new(0)]);
        let gapped = Kwg(nodes.clone().into_boxed_slice());
        assert!(gapped.runs_ascend());
        let reader = alphabet::AlphabetReader::new_for_words(gc.alphabet());
        let mut buf = Vec::new();
        let accepts = |graph: &Kwg<Node22>, word: &[u8]| {
            let p =
                (word.iter()).try_fold(0, |p, &tile| Some(graph.seek(p, tile)).filter(|&p| p > 0));
            p.is_some_and(|p| graph[p].accepts())
        };
        for word in words.iter().chain(&["SEA", "EAST", "TEAS"]) {
            reader.set_word(word, &mut buf).unwrap();
            assert_eq!(accepts(&gapped, &buf), accepts(&kwg, &buf), "{word}");
        }
        let at = (end + 3..nodes.len() - 1)
            .find(|&i| !nodes[i].is_end())
            .unwrap();
        let (a, b) = (nodes[at], nodes[at + 1]);
        nodes[at] =
            Node22::new(Node22::pack(b.tile(), b.accepts(), a.is_end(), b.arc_index()).unwrap());
        nodes[at + 1] =
            Node22::new(Node22::pack(a.tile(), a.accepts(), b.is_end(), a.arc_index()).unwrap());
        assert!(!Kwg(nodes.into_boxed_slice()).runs_ascend());
    }

    #[test]
    #[inline]
    fn a_graph_fits_only_an_alphabet_that_has_all_its_tiles() {
        let english = game_config::make_english_game_config();
        let polish = game_config::make_polish_game_config();
        assert!(polish.alphabet().len() > english.alphabet().len());

        let theirs = graph_of(&polish, &["ZOLZA", "ZABA"]);
        assert!(theirs.fits_alphabet(polish.alphabet().len()));
        assert!(!theirs.fits_alphabet(english.alphabet().len()));

        let ours = graph_of(&english, &["SEAT", "TEA"]);
        assert!(ours.fits_alphabet(english.alphabet().len()));
        assert!(ours.fits_alphabet(polish.alphabet().len()));
        assert!(!ours.fits_alphabet(1));
    }
}
