// Copyright (C) 2020-2026 Andy Kurnia.

pub struct WordList {
    blob: Vec<u8>,
    ends: Vec<u32>,
}

impl WordList {
    #[inline(always)]
    pub fn build<N: super::kwg::Node>(g: &super::kwg::Kwg<N>) -> Self {
        fn walk<N: super::kwg::Node>(
            g: &super::kwg::Kwg<N>,
            mut p: i32,
            w: &mut Vec<u8>,
            out: &mut WordList,
        ) {
            if p <= 0 {
                return;
            }
            loop {
                let node = g[p];
                w.push(node.tile());
                if node.accepts() {
                    out.blob.extend_from_slice(w);
                    out.ends.push(out.blob.len() as u32);
                }
                walk(g, node.arc_index(), w, out);
                w.pop();
                if node.is_end() {
                    return;
                }
                p += 1;
            }
        }
        let mut out = WordList {
            blob: Vec::new(),
            ends: Vec::new(),
        };
        walk(g, g[0].arc_index(), &mut Vec::new(), &mut out);
        out
    }

    pub fn iter(&self) -> impl Iterator<Item = &[u8]> {
        let mut at = 0usize;
        self.ends.iter().map(move |&end| {
            let w = &self.blob[at..end as usize];
            at = end as usize;
            w
        })
    }
}

// MurmurHash3's 64-bit finalizer, public domain:
// https://github.com/aappleby/smhasher/blob/master/src/MurmurHash3.cpp
#[inline(always)]
pub fn mixed(v: u128) -> u64 {
    let mut k = (v >> 64) as u64;
    k ^= k.rotate_left(17);
    k ^= v as u64;
    k ^= k >> 33;
    k = k.wrapping_mul(0xff51afd7ed558ccd);
    k ^= k >> 33;
    k = k.wrapping_mul(0xc4ceb9fe1a85ec53);
    k ^= k >> 33;
    k
}

const MAX_LAYOUT_TILES: usize = 64;

#[derive(Clone, PartialEq)]
pub struct KeyLayout {
    shift: Box<[u8]>,
    max_count: Box<[u8]>,
    bits: u32,
}

impl KeyLayout {
    pub const BITS: u32 = 128;

    #[inline(always)]
    pub fn of(alphabet: &super::alphabet::Alphabet, board_dim: u8) -> Option<Self> {
        let blanks = alphabet.freq(0);
        let mut max_count = vec![0u8; alphabet.len() as usize];
        for (tile, slot) in max_count.iter_mut().enumerate().skip(1) {
            *slot = alphabet
                .freq(tile as u8)
                .saturating_add(blanks)
                .min(board_dim);
        }
        Self::of_max_counts(&max_count)
    }

    #[inline(always)]
    pub fn of_max_counts(max_count: &[u8]) -> Option<Self> {
        let mut shift = vec![0u8; max_count.len()].into_boxed_slice();
        let mut bits = 0u32;
        for (tile, &supply) in max_count.iter().enumerate().skip(1) {
            if supply == 0 {
                continue;
            }
            shift[tile] = bits as u8;
            if tile > u8::MAX as usize {
                return None;
            }
            let width = u8::BITS - supply.leading_zeros();
            if bits + width > Self::BITS {
                return None;
            }
            bits += width;
        }
        if bits == 0 {
            return None;
        }
        Some(Self {
            shift,
            max_count: max_count.to_vec().into_boxed_slice(),
            bits,
        })
    }

    #[inline(always)]
    pub fn covers(&self, alphabet: &super::alphabet::Alphabet, board_dim: u8) -> bool {
        let blanks = alphabet.freq(0);
        (1..alphabet.len()).all(|tile| {
            let reachable = alphabet.freq(tile).saturating_add(blanks).min(board_dim);
            self.max_count
                .get(tile as usize)
                .is_some_and(|&room| room >= reachable)
        })
    }

    #[inline(always)]
    pub fn place_value(&self, tile: u8) -> u128 {
        1u128 << self.shift[tile as usize]
    }

    #[inline(always)]
    pub fn max_count(&self, tile: u8) -> u8 {
        self.max_count[tile as usize]
    }

    #[inline(always)]
    pub fn count_in(&self, key: u128, tile: u8) -> u8 {
        let width = u8::BITS - self.max_count[tile as usize].leading_zeros();
        ((key >> self.shift[tile as usize]) & ((1u128 << width) - 1)) as u8
    }

    #[inline(always)]
    pub fn bits(&self) -> u32 {
        self.bits
    }

    #[inline(always)]
    pub fn matches(&self, alphabet: &super::alphabet::Alphabet, board_dim: u8) -> bool {
        match Self::of(alphabet, board_dim) {
            Some(other) => {
                let tiles = self.max_count.len().max(other.max_count.len());
                (0..tiles).all(|tile| {
                    self.max_count.get(tile).copied().unwrap_or(0)
                        == other.max_count.get(tile).copied().unwrap_or(0)
                })
            }
            None => false,
        }
    }

    #[inline(always)]
    pub fn holds(&self, key: u128) -> bool {
        for tile in 1..self.max_count.len() {
            let max = self.max_count[tile];
            if max == 0 {
                continue;
            }
            let width = u8::BITS - max.leading_zeros();
            let field = (key >> self.shift[tile]) & ((1u128 << width) - 1);
            if field > max as u128 {
                return false;
            }
        }
        true
    }

    #[inline(always)]
    pub fn key_of(&self, word: &[u8]) -> Option<u128> {
        let mut counts = [0u8; MAX_LAYOUT_TILES];
        let mut key = 0u128;
        for &tile in word {
            let seen = counts.get_mut(tile as usize)?;
            *seen += 1;
            if tile == 0 || *seen > *self.max_count.get(tile as usize)? {
                return None;
            }
            key += 1u128 << self.shift[tile as usize];
        }
        Some(key)
    }
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd)]
pub struct Fitted(pub u128);

impl Fitted {}

#[derive(Clone, PartialEq)]
pub struct WordsAt {
    pub at: u32,
    pub n: u16,
    pub len: u8,
}

pub struct Words<'a> {
    pub words: &'a [u8],
    pub len: u8,
}

impl<'a> Words<'a> {
    #[inline(always)]
    pub fn iter(&self) -> impl Iterator<Item = &'a [u8]> {
        self.words.chunks_exact(self.len as usize)
    }
}

pub type Key = Fitted;

#[cfg(test)]
mod tests {
    use super::*;

    #[inline]
    fn shipped_alphabets() -> Vec<(&'static str, crate::alphabet::Alphabet, u8)> {
        use crate::alphabet::*;
        vec![
            ("catalan", make_catalan_alphabet(), 15),
            ("super_catalan", make_super_catalan_alphabet(), 21),
            ("decimal", make_decimal_alphabet(), 15),
            ("dutch", make_dutch_alphabet(), 15),
            ("english", make_english_alphabet(), 15),
            ("french", make_french_alphabet(), 15),
            ("german", make_german_alphabet(), 15),
            ("hex", make_hex_alphabet(), 15),
            ("hong_kong_english", make_hong_kong_english_alphabet(), 15),
            ("norwegian", make_norwegian_alphabet(), 15),
            ("polish", make_polish_alphabet(), 15),
            ("slovene", make_slovene_alphabet(), 15),
            ("spanish", make_spanish_alphabet(), 15),
            ("super_english", make_super_english_alphabet(), 21),
            ("swedish", make_swedish_alphabet(), 15),
        ]
    }

    #[test]
    #[inline]
    fn the_key_is_sized_by_the_alphabet_it_holds() {
        let mut sized = Vec::new();
        let mut refused = Vec::new();
        for (name, alphabet, dim) in shipped_alphabets() {
            match KeyLayout::of(&alphabet, dim) {
                Some(layout) => {
                    assert!(
                        layout.bits() <= KeyLayout::BITS,
                        "{name}: {} bits does not fit a key",
                        layout.bits(),
                    );
                    sized.push((name, layout.bits()));
                }
                None => refused.push(name),
            }
        }
        assert_eq!(
            refused,
            vec!["decimal", "hex"],
            "only an alphabet with nothing to deal should be refused",
        );
        assert_eq!(
            sized
                .iter()
                .find(|(name, _)| *name == "english")
                .map(|(_, bits)| *bits),
            Some(80),
        );
        for (name, bits) in &sized {
            assert!(*bits <= 128, "{name}: {bits} bits wants a second limb");
        }
    }

    #[test]
    #[inline]
    fn a_key_composes_by_adding() {
        let alphabet = crate::alphabet::make_english_alphabet();
        let layout = KeyLayout::of(&alphabet, 15).unwrap();
        let left = layout.key_of(&[1, 2, 3]).unwrap();
        let right = layout.key_of(&[1, 20]).unwrap();
        assert_eq!(layout.key_of(&[1, 2, 3, 1, 20]).unwrap(), left + right);
        let mut acc = 0u128;
        for tile in [1u8, 2, 3, 1, 20] {
            acc += layout.place_value(tile);
        }
        assert_eq!(layout.key_of(&[1, 1, 2, 3, 20]).unwrap(), acc);
        assert_eq!(layout.key_of(&[3, 1, 20]), layout.key_of(&[20, 1, 3]));
        assert_ne!(layout.key_of(&[1, 1, 20]), layout.key_of(&[1, 20]));
        assert_ne!(layout.key_of(&[1]).unwrap(), 0);
    }

    #[test]
    #[inline]
    fn a_count_the_bag_cannot_supply_is_refused() {
        let alphabet = crate::alphabet::make_english_alphabet();
        let layout = KeyLayout::of(&alphabet, 15).unwrap();
        assert_eq!(layout.max_count(26), 3);
        assert!(layout.key_of(&[26, 26, 26]).is_some());
        assert!(layout.key_of(&[26, 26, 26, 26]).is_none());
        assert_eq!(layout.max_count(5), 14);
        assert!(layout.key_of(&[5; 14]).is_some());
        assert!(layout.key_of(&[5; 15]).is_none());
        assert!(layout.key_of(&[27]).is_none());
        assert!(layout.key_of(&[0]).is_none());
    }

    #[test]
    #[inline]
    fn a_layout_knows_whose_alphabet_it_is() {
        let english = crate::alphabet::make_english_alphabet();
        let layout = KeyLayout::of(&english, 15).unwrap();
        assert!(layout.matches(&english, 15));
        assert!(!layout.matches(&crate::alphabet::make_french_alphabet(), 15));
        assert!(!layout.matches(&crate::alphabet::make_hong_kong_english_alphabet(), 15));
        assert!(layout.matches(&english, 21));
        let super_english = crate::alphabet::make_super_english_alphabet();
        let wide = KeyLayout::of(&super_english, 21).unwrap();
        assert!(!wide.matches(&super_english, 15));
        assert_eq!(wide.max_count(5), 21);
        assert_eq!(KeyLayout::of(&super_english, 15).unwrap().max_count(5), 15);
    }
}
