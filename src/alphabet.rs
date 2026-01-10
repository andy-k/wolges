// Copyright (C) 2020-2026 Andy Kurnia.

use super::{bites, bites_str, error};

use std::str::FromStr;

struct Tile {
    label: bites_str::BitesStr,
    blank_label: bites_str::BitesStr,
    freq: u8,
    score: i8,
    is_vowel: bool,
    alias_labels: Vec<bites_str::BitesStr>,
    alias_blank_labels: Vec<bites_str::BitesStr>,
}

#[derive(Default)]
pub struct StaticAlphabet {
    tiles: Vec<Tile>,
    widest_label_len: usize, // in codepoints for now (graphemes is too complex)
    num_tiles: u16,
    same_score_tile: Box<[u8]>,
    same_score_tile_bits: Box<[u64]>,
    tiles_by_descending_scores: Box<[u8]>,
}

pub enum Alphabet {
    Static(StaticAlphabet),
}

impl Alphabet {
    fn new_static(tiles: Vec<Tile>) -> Self {
        let num_letters = tiles.len() as u8;
        let mut same_score_tile = Box::from_iter(0..num_letters);
        let mut same_score_tile_bits = Vec::with_capacity(num_letters as usize);
        // sameness is defined only by same scores (is_vowel may mismatch).
        for i in 0..num_letters {
            if same_score_tile[i as usize] == i {
                let mut b = 1u64 << i;
                let v = tiles[i as usize].score;
                for j in i + 1..num_letters {
                    if tiles[j as usize].score == v {
                        same_score_tile[j as usize] = i;
                        b |= 1u64 << j;
                    }
                }
                same_score_tile_bits.push(b);
                if i == 0 && b != 1 {
                    // blank has same score as something else, use one of those.
                    // this keeps the gaddag builder working.
                    let v = (b & !1).trailing_zeros() as u8;
                    while b != 0 {
                        same_score_tile[b.trailing_zeros() as usize] = v;
                        b &= b - 1; // turn off lowest bit
                    }
                }
            } else {
                same_score_tile_bits
                    .push(same_score_tile_bits[same_score_tile[i as usize] as usize]);
            }
        }
        let mut tiles_by_descending_scores = Box::from_iter(0..num_letters);
        tiles_by_descending_scores.sort_unstable_by(|&a, &b| {
            tiles[b as usize]
                .score
                .cmp(&tiles[a as usize].score)
                .then(a.cmp(&b))
        });
        Self::Static(StaticAlphabet {
            widest_label_len: tiles.iter().fold(0, |acc, tile| {
                acc.max(tile.label.chars().count())
                    .max(tile.blank_label.chars().count())
            }),
            num_tiles: tiles.iter().map(|tile| tile.freq as u16).sum(),
            same_score_tile,
            same_score_tile_bits: same_score_tile_bits.into_boxed_slice(),
            tiles_by_descending_scores,
            tiles,
        })
    }

    fn new_static_from_text(s: &str) -> error::Returns<Self> {
        let mut tiles = Vec::new();
        for line_str in s.lines() {
            let mut tokens = line_str.split_whitespace();
            // there is minimal error handling...
            let label = tokens.next().ok_or("not enough tokens")?.into();
            let blank_label = tokens.next().ok_or("not enough tokens")?.into();
            let freq = u8::from_str(tokens.next().ok_or("not enough tokens")?)?;
            let score = i8::from_str(tokens.next().ok_or("not enough tokens")?)?;
            let is_vowel = match u8::from_str(tokens.next().ok_or("not enough tokens")?)? {
                0 => false,
                1 => true,
                _ => Err("invalid bool")?,
            };
            let num_alias_labels = usize::from_str(tokens.next().ok_or("not enough tokens")?)?;
            let mut alias_labels = Vec::with_capacity(num_alias_labels);
            for _ in 0..num_alias_labels {
                alias_labels.push(tokens.next().ok_or("not enough tokens")?.into());
            }
            let num_alias_blank_labels =
                usize::from_str(tokens.next().ok_or("not enough tokens")?)?;
            let mut alias_blank_labels = Vec::with_capacity(num_alias_blank_labels);
            for _ in 0..num_alias_blank_labels {
                alias_blank_labels.push(tokens.next().ok_or("not enough tokens")?.into());
            }
            if tokens.next().is_some() {
                return Err("too many tokens".into());
            }
            tiles.push(Tile {
                label,
                blank_label,
                freq,
                score,
                is_vowel,
                alias_labels,
                alias_blank_labels,
            });
        }
        Ok(Self::new_static(tiles))
    }

    #[inline(always)]
    pub fn len(&self) -> u8 {
        match self {
            Alphabet::Static(x) => x.tiles.len() as u8,
        }
    }

    #[inline(always)]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    #[inline(always)]
    fn get(&self, idx: u8) -> &Tile {
        match self {
            Alphabet::Static(x) => &x.tiles[idx as usize],
        }
    }

    #[inline(always)]
    pub fn widest_label_len(&self) -> usize {
        match self {
            Alphabet::Static(x) => x.widest_label_len,
        }
    }

    #[inline(always)]
    pub fn num_tiles(&self) -> u16 {
        match self {
            Alphabet::Static(x) => x.num_tiles,
        }
    }

    #[inline(always)]
    pub fn of_board(&self, idx: u8) -> Option<&str> {
        let c = idx & 0x7f;
        if c == 0 || c >= self.len() {
            None
        } else if idx & 0x80 == 0 {
            Some(&self.get(c).label)
        } else {
            Some(&self.get(c).blank_label)
        }
    }

    #[inline(always)]
    pub fn of_rack(&self, idx: u8) -> Option<&str> {
        if idx >= self.len() {
            None
        } else {
            Some(&self.get(idx).label)
        }
    }

    #[inline(always)]
    pub fn score(&self, idx: u8) -> i8 {
        self.get(idx & !((idx as i8) >> 7) as u8).score
    }

    #[inline(always)]
    pub fn is_vowel(&self, idx: u8) -> bool {
        self.get(idx & 0x7f).is_vowel
    }

    #[inline(always)]
    pub fn freq(&self, idx: u8) -> u8 {
        self.get(idx).freq
    }

    #[inline(always)]
    pub fn representative_same_score_tile(&self, idx: u8) -> u8 {
        match self {
            Alphabet::Static(x) => {
                if idx >= self.len() {
                    idx
                } else {
                    x.same_score_tile[idx as usize]
                }
            }
        }
    }

    #[inline(always)]
    pub fn same_score_tile_bits(&self, idx: u8) -> u64 {
        match self {
            Alphabet::Static(x) => {
                if idx >= self.len() {
                    0
                } else {
                    x.same_score_tile_bits[idx as usize]
                }
            }
        }
    }

    #[inline(always)]
    pub fn tiles_by_descending_scores(&self) -> &[u8] {
        match self {
            Alphabet::Static(x) => &x.tiles_by_descending_scores[..],
        }
    }

    pub fn fmt_rack<'a>(&'a self, rack: &'a [u8]) -> WriteableRack<'a> {
        WriteableRack {
            alphabet: self,
            rack,
        }
    }

    pub fn rack_score(&self, rack: &[u8]) -> i32 {
        rack.iter().map(|&t| self.score(t) as i32).sum::<i32>()
    }
}

pub struct WriteableRack<'a> {
    alphabet: &'a Alphabet,
    rack: &'a [u8],
}

impl std::fmt::Display for WriteableRack<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if f.width().is_some() {
            // allocates, but no choice.
            #[expect(clippy::recursive_format_impl)]
            return f.pad(&format!("{self}"));
        }
        for &tile in self.rack {
            write!(f, "{}", self.alphabet.of_rack(tile).unwrap())?;
        }
        Ok(())
    }
}

macro_rules! new_static_alphabet_from_file {
    ($filename: expr) => {
        Alphabet::new_static_from_text(include_str!($filename)).unwrap()
    };
}

// https://en.wikipedia.org/wiki/Scrabble_letter_distributions#Catalan
// with QU tile instead of Q
pub fn make_catalan_alphabet() -> Alphabet {
    new_static_alphabet_from_file!("alphabets/catalan.txt")
}

// https://en.wikipedia.org/wiki/Scrabble_letter_distributions#Catalan
// note: Ç and L·L have different scores from regular.
pub fn make_super_catalan_alphabet() -> Alphabet {
    new_static_alphabet_from_file!("alphabets/super_catalan.txt")
}

// for pass-through kwg reading/building, cannot be used for games.
pub fn make_decimal_alphabet() -> Alphabet {
    new_static_alphabet_from_file!("alphabets/decimal.txt")
}

// https://en.wikipedia.org/wiki/Scrabble_letter_distributions#Dutch
pub fn make_dutch_alphabet() -> Alphabet {
    new_static_alphabet_from_file!("alphabets/dutch.txt")
}

// https://en.wikipedia.org/wiki/Scrabble_letter_distributions#English
pub fn make_english_alphabet() -> Alphabet {
    new_static_alphabet_from_file!("alphabets/english.txt")
}

// https://en.wikipedia.org/wiki/Scrabble_letter_distributions#French
// https://en.wikipedia.org/wiki/French_orthography
pub fn make_french_alphabet() -> Alphabet {
    new_static_alphabet_from_file!("alphabets/french.txt")
}

// for pass-through kwg reading/building, cannot be used for games.
pub fn make_hex_alphabet() -> Alphabet {
    new_static_alphabet_from_file!("alphabets/hex.txt")
}

// http://hkcrosswordclub.com/?cat=14
pub fn make_hong_kong_english_alphabet() -> Alphabet {
    new_static_alphabet_from_file!("alphabets/hong_kong_english.txt")
}

// https://en.wikipedia.org/wiki/Super_Scrabble
pub fn make_super_english_alphabet() -> Alphabet {
    new_static_alphabet_from_file!("alphabets/super_english.txt")
}

// https://en.wikipedia.org/wiki/Scrabble_letter_distributions#German
pub fn make_german_alphabet() -> Alphabet {
    new_static_alphabet_from_file!("alphabets/german.txt")
}

// https://en.wikipedia.org/wiki/Scrabble_letter_distributions#Norwegian
// https://en.wikipedia.org/wiki/Norwegian_orthography
// https://unicode.org/mail-arch/unicode-ml/y2002-m01/0297.html
// also this ordering matches system locale files
pub fn make_norwegian_alphabet() -> Alphabet {
    new_static_alphabet_from_file!("alphabets/norwegian.txt")
}

// https://en.wikipedia.org/wiki/Scrabble_letter_distributions#Polish
// https://en.wikipedia.org/wiki/Polish_alphabet#Letters
// https://en.wikipedia.org/wiki/Polish_phonology#Vowels
pub fn make_polish_alphabet() -> Alphabet {
    new_static_alphabet_from_file!("alphabets/polish.txt")
}

// https://en.wikipedia.org/wiki/Scrabble_letter_distributions#Slovenian
pub fn make_slovene_alphabet() -> Alphabet {
    new_static_alphabet_from_file!("alphabets/slovene.txt")
}

// https://en.wikipedia.org/wiki/Scrabble_letter_distributions#Spanish
// based on Spanish-language sets sold outside North America
// (CH/LL/RR are ambiguous and should not be supported)
pub fn make_spanish_alphabet() -> Alphabet {
    new_static_alphabet_from_file!("alphabets/spanish.txt")
}

pub struct AlphabetReader {
    supported_tiles: Box<[(u8, bites::Bites)]>,
    by_first_byte: [Option<(usize, usize)>; 256],
}

// This is slow, but supports multi-codepoint tiles with greedy matching.
impl AlphabetReader {
    pub fn new_for_tiles(mut supported_tiles: Box<[(u8, bites::Bites)]>) -> Self {
        // sort supported tiles by first byte (asc), length (desc), and tile (asc).
        supported_tiles.sort_unstable_by(|(a_tile, a_label), (b_tile, b_label)| {
            a_label[0].cmp(&b_label[0]).then_with(|| {
                b_label
                    .len()
                    .cmp(&a_label.len())
                    .then_with(|| a_tile.cmp(b_tile))
            })
        });
        let mut h = [None; 256];
        let mut i = supported_tiles.len();
        while i > 0 {
            i -= 1;
            let (_tile, label) = &supported_tiles[i];
            let label0 = label[0];
            let mut j = i;
            while j > 0 && supported_tiles[j - 1].1[0] == label0 {
                j -= 1;
            }
            h[label0 as usize] = Some((j, i + 1));
            i = j;
        }
        Self {
            supported_tiles,
            by_first_byte: h,
        }
    }

    // Recognizes [A-Z] and [a-z] identically, as well as aliases.
    pub fn new_for_words(alphabet: &Alphabet) -> Self {
        let alphabet_len = alphabet.len();
        let mut cap = 0;
        for idx in 1..alphabet_len {
            let tile = alphabet.get(idx);
            cap += 2 + tile.alias_labels.len() + tile.alias_blank_labels.len();
        }
        let mut supported_tiles = Vec::with_capacity(cap);
        for idx in 1..alphabet_len {
            let tile = alphabet.get(idx);
            supported_tiles.push((idx, tile.label.as_bytes().into()));
            for alias in tile.alias_labels.iter() {
                supported_tiles.push((idx, alias.as_bytes().into()));
            }
            supported_tiles.push((idx, tile.blank_label.as_bytes().into()));
            for alias in tile.alias_blank_labels.iter() {
                supported_tiles.push((idx, alias.as_bytes().into()));
            }
        }
        let supported_tiles = supported_tiles.into_boxed_slice();
        Self::new_for_tiles(supported_tiles)
    }

    // Same as new_for_words but merge tiles with same score.
    pub fn new_for_word_scores(alphabet: &Alphabet) -> Self {
        let alphabet_len = alphabet.len();
        let mut cap = 0;
        for idx in 1..alphabet_len {
            let tile = alphabet.get(idx);
            cap += 2 + tile.alias_labels.len() + tile.alias_blank_labels.len();
        }
        let mut supported_tiles = Vec::with_capacity(cap);
        for idx in 1..alphabet_len {
            let tile = alphabet.get(idx);
            let representative_idx = alphabet.representative_same_score_tile(idx);
            supported_tiles.push((representative_idx, tile.label.as_bytes().into()));
            for alias in tile.alias_labels.iter() {
                supported_tiles.push((representative_idx, alias.as_bytes().into()));
            }
            supported_tiles.push((representative_idx, tile.blank_label.as_bytes().into()));
            for alias in tile.alias_blank_labels.iter() {
                supported_tiles.push((representative_idx, alias.as_bytes().into()));
            }
        }
        let supported_tiles = supported_tiles.into_boxed_slice();
        Self::new_for_tiles(supported_tiles)
    }

    // Recognizes [?A-Z] and [a-z] identically, as well as aliases.
    pub fn new_for_racks(alphabet: &Alphabet) -> Self {
        let alphabet_len = alphabet.len();
        let mut cap = 0;
        if alphabet_len > 0 {
            let tile = alphabet.get(0);
            cap += 1 + tile.alias_labels.len();
        }
        for idx in 1..alphabet_len {
            let tile = alphabet.get(idx);
            cap += 2 + tile.alias_labels.len() + tile.alias_blank_labels.len();
        }
        let mut supported_tiles = Vec::with_capacity(cap);
        if alphabet_len > 0 {
            let tile = alphabet.get(0);
            supported_tiles.push((0, tile.label.as_bytes().into()));
            for alias in tile.alias_labels.iter() {
                supported_tiles.push((0, alias.as_bytes().into()));
            }
        }
        for idx in 1..alphabet_len {
            let tile = alphabet.get(idx);
            supported_tiles.push((idx, tile.label.as_bytes().into()));
            for alias in tile.alias_labels.iter() {
                supported_tiles.push((idx, alias.as_bytes().into()));
            }
            supported_tiles.push((idx, tile.blank_label.as_bytes().into()));
            for alias in tile.alias_blank_labels.iter() {
                supported_tiles.push((idx, alias.as_bytes().into()));
            }
        }
        let supported_tiles = supported_tiles.into_boxed_slice();
        Self::new_for_tiles(supported_tiles)
    }

    // Recognizes [A-Za-z] and aliases. Play-through needs to be dealt with separately.
    pub fn new_for_plays(alphabet: &Alphabet) -> Self {
        let alphabet_len = alphabet.len();
        let mut cap = 0;
        for idx in 1..alphabet_len {
            let tile = alphabet.get(idx);
            cap += 2 + tile.alias_labels.len() + tile.alias_blank_labels.len();
        }
        let mut supported_tiles = Vec::with_capacity(cap);
        for idx in 1..alphabet_len {
            let tile = alphabet.get(idx);
            supported_tiles.push((idx, tile.label.as_bytes().into()));
            for alias in tile.alias_labels.iter() {
                supported_tiles.push((idx, alias.as_bytes().into()));
            }
            let blank_idx = idx | 0x80;
            supported_tiles.push((blank_idx, tile.blank_label.as_bytes().into()));
            for alias in tile.alias_blank_labels.iter() {
                supported_tiles.push((blank_idx, alias.as_bytes().into()));
            }
        }
        let supported_tiles = supported_tiles.into_boxed_slice();
        Self::new_for_tiles(supported_tiles)
    }

    // Given sb (str.as_bytes()) and ix, decode the next tile starting at sb[ix].
    // Returns Ok((tile, updated_index)) if it is a valid tile.
    // Returns None if it is not a valid tile.
    // Undefined behavior if ix >= sb.len().
    #[inline(always)]
    pub fn next_tile(&self, sb: &[u8], ix: usize) -> Option<(u8, usize)> {
        // Safe because we have all 256.
        if let Some((range_lo, range_hi)) = unsafe {
            self.by_first_byte
                .get_unchecked(*sb.get_unchecked(ix) as usize)
        } {
            let sb_len = sb.len();
            // Safe because of how by_first_byte was constructed.
            for (tile, label) in unsafe { self.supported_tiles.get_unchecked(*range_lo..*range_hi) }
            {
                let label_len = label.len();
                let end_ix = ix + label_len;
                // Safe after accessing sb[ix].
                if label_len == 1
                    || end_ix <= sb_len
                        && unsafe { sb.get_unchecked(ix + 1..end_ix) == label.get_unchecked(1..) }
                {
                    return Some((*tile, end_ix));
                }
            }
        }
        None
    }

    #[inline(always)]
    pub fn set_word(&self, s: &str, v: &mut Vec<u8>) -> Result<(), Box<dyn std::error::Error>> {
        v.clear();
        self.append_word(s, v)
    }

    #[inline(always)]
    pub fn append_word(&self, s: &str, v: &mut Vec<u8>) -> Result<(), Box<dyn std::error::Error>> {
        if !s.is_empty() {
            v.reserve(s.len());
            let sb = s.as_bytes();
            let mut ix = 0;
            while ix < sb.len() {
                if let Some((tile, end_ix)) = self.next_tile(sb, ix) {
                    v.push(tile);
                    ix = end_ix;
                } else {
                    crate::return_error!(format!("invalid tile after {v:?} in {s:?}"));
                }
            }
        }
        Ok(())
    }
}
