// Copyright (C) 2020-2024 Andy Kurnia.

struct Tile {
    label: String,
    blank_label: String,
    freq: u8,
    score: i8,
    is_vowel: bool,
    alias_labels: Vec<String>,
    alias_blank_labels: Vec<String>,
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
    pub fn new_static(x: StaticAlphabet) -> Self {
        let num_letters = x.tiles.len() as u8;
        let mut same_score_tile = Box::from_iter(0..num_letters);
        let mut same_score_tile_bits = Vec::with_capacity(num_letters as usize);
        // sameness is defined only by same scores (is_vowel may mismatch).
        for i in 0..num_letters {
            if same_score_tile[i as usize] == i {
                let mut b = 1u64 << i;
                let v = x.tiles[i as usize].score;
                for j in i + 1..num_letters {
                    if x.tiles[j as usize].score == v {
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
            x.tiles[b as usize]
                .score
                .cmp(&x.tiles[a as usize].score)
                .then(a.cmp(&b))
        });
        Self::Static(StaticAlphabet {
            widest_label_len: x.tiles.iter().fold(0, |acc, tile| {
                acc.max(tile.label.chars().count())
                    .max(tile.blank_label.chars().count())
            }),
            num_tiles: x.tiles.iter().map(|tile| tile.freq as u16).sum(),
            same_score_tile,
            same_score_tile_bits: same_score_tile_bits.into_boxed_slice(),
            tiles_by_descending_scores,
            ..x
        })
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
            #[allow(clippy::recursive_format_impl)]
            return f.pad(&format!("{self}"));
        }
        for &tile in self.rack {
            write!(f, "{}", self.alphabet.of_rack(tile).unwrap())?;
        }
        Ok(())
    }
}

macro_rules! v {
    ($($item: expr),*) => { vec![$($item.into(), )*] };
}

macro_rules! tile {
    ($label: expr, $blank_label: expr, $freq: expr, $score: expr, $vowel_int: expr) => {
        tile!($label, $blank_label, $freq, $score, $vowel_int, v![], v![])
    };
    ($label: expr, $blank_label: expr, $freq: expr, $score: expr, $vowel_int: expr, $alias_labels: expr, $alias_blank_labels: expr) => {
        Tile {
            label: $label.into(),
            blank_label: $blank_label.into(),
            freq: $freq,
            score: $score,
            is_vowel: ($vowel_int) != 0,
            alias_labels: $alias_labels,
            alias_blank_labels: $alias_blank_labels,
        }
    };
}

// https://en.wikipedia.org/wiki/Scrabble_letter_distributions#Catalan
// with QU tile instead of Q
pub fn make_catalan_alphabet() -> Alphabet {
    Alphabet::new_static(StaticAlphabet {
        tiles: vec![
            tile!("?", "?", 2, 0, 0),
            tile!("A", "a", 12, 1, 1),
            tile!("B", "b", 2, 3, 0),
            tile!("C", "c", 3, 2, 0),
            tile!("Ç", "ç", 1, 10, 0, v!["K"], v!["k"]),
            tile!("D", "d", 3, 2, 0),
            tile!("E", "e", 13, 1, 1),
            tile!("F", "f", 1, 4, 0),
            tile!("G", "g", 2, 3, 0),
            tile!("H", "h", 1, 8, 0),
            tile!("I", "i", 8, 1, 1),
            tile!("J", "j", 1, 8, 0),
            tile!("L", "l", 4, 1, 0),
            tile!("L·L", "l·l", 1, 10, 0, v!["W"], v!["w"]),
            tile!("M", "m", 3, 2, 0),
            tile!("N", "n", 6, 1, 0),
            tile!("NY", "ny", 1, 10, 0, v!["Y"], v!["y"]),
            tile!("O", "o", 5, 1, 1),
            tile!("P", "p", 2, 3, 0),
            tile!("QU", "qu", 1, 8, 0, v!["Q"], v!["q"]),
            tile!("R", "r", 8, 1, 0),
            tile!("S", "s", 8, 1, 0),
            tile!("T", "t", 5, 1, 0),
            tile!("U", "u", 4, 1, 1),
            tile!("V", "v", 1, 4, 0),
            tile!("X", "x", 1, 10, 0),
            tile!("Z", "z", 1, 8, 0),
        ],
        ..Default::default()
    })
}

// https://en.wikipedia.org/wiki/Scrabble_letter_distributions#Catalan
pub fn make_super_catalan_alphabet() -> Alphabet {
    Alphabet::new_static(StaticAlphabet {
        tiles: vec![
            tile!("?", "?", 5, 0, 0),
            tile!("A", "a", 25, 1, 1),
            tile!("B", "b", 3, 3, 0),
            tile!("C", "c", 5, 2, 0),
            tile!("Ç", "ç", 2, 12, 0, v!["K"], v!["k"]), // note: different score from regular
            tile!("D", "d", 5, 2, 0),
            tile!("E", "e", 27, 1, 1),
            tile!("F", "f", 2, 4, 0),
            tile!("G", "g", 3, 3, 0),
            tile!("H", "h", 2, 8, 0),
            tile!("I", "i", 17, 1, 1),
            tile!("J", "j", 2, 8, 0),
            tile!("L", "l", 8, 1, 0),
            tile!("L·L", "l·l", 1, 15, 0, v!["W"], v!["w"]), // note: different score from regular
            tile!("M", "m", 7, 2, 0),
            tile!("N", "n", 12, 1, 0),
            tile!("NY", "ny", 2, 10, 0, v!["Y"], v!["y"]),
            tile!("O", "o", 10, 1, 1),
            tile!("P", "p", 3, 3, 0),
            tile!("QU", "qu", 2, 8, 0, v!["Q"], v!["q"]),
            tile!("R", "r", 16, 1, 0),
            tile!("S", "s", 19, 1, 0),
            tile!("T", "t", 10, 1, 0),
            tile!("U", "u", 6, 1, 1),
            tile!("V", "v", 2, 4, 0),
            tile!("X", "x", 2, 10, 0),
            tile!("Z", "z", 2, 8, 0),
        ],
        ..Default::default()
    })
}

// https://en.wikipedia.org/wiki/Scrabble_letter_distributions#English
pub fn make_english_alphabet() -> Alphabet {
    Alphabet::new_static(StaticAlphabet {
        tiles: vec![
            tile!("?", "?", 2, 0, 0),
            tile!("A", "a", 9, 1, 1),
            tile!("B", "b", 2, 3, 0),
            tile!("C", "c", 2, 3, 0),
            tile!("D", "d", 4, 2, 0),
            tile!("E", "e", 12, 1, 1),
            tile!("F", "f", 2, 4, 0),
            tile!("G", "g", 3, 2, 0),
            tile!("H", "h", 2, 4, 0),
            tile!("I", "i", 9, 1, 1),
            tile!("J", "j", 1, 8, 0),
            tile!("K", "k", 1, 5, 0),
            tile!("L", "l", 4, 1, 0),
            tile!("M", "m", 2, 3, 0),
            tile!("N", "n", 6, 1, 0),
            tile!("O", "o", 8, 1, 1),
            tile!("P", "p", 2, 3, 0),
            tile!("Q", "q", 1, 10, 0),
            tile!("R", "r", 6, 1, 0),
            tile!("S", "s", 4, 1, 0),
            tile!("T", "t", 6, 1, 0),
            tile!("U", "u", 4, 1, 1),
            tile!("V", "v", 2, 4, 0),
            tile!("W", "w", 2, 4, 0),
            tile!("X", "x", 1, 8, 0),
            tile!("Y", "y", 2, 4, 0),
            tile!("Z", "z", 1, 10, 0),
        ],
        ..Default::default()
    })
}

// https://en.wikipedia.org/wiki/Scrabble_letter_distributions#French
// https://en.wikipedia.org/wiki/French_orthography
pub fn make_french_alphabet() -> Alphabet {
    Alphabet::new_static(StaticAlphabet {
        tiles: vec![
            tile!("?", "?", 2, 0, 0),
            tile!("A", "a", 9, 1, 1),
            tile!("B", "b", 2, 3, 0),
            tile!("C", "c", 2, 3, 0),
            tile!("D", "d", 3, 2, 0),
            tile!("E", "e", 15, 1, 1),
            tile!("F", "f", 2, 4, 0),
            tile!("G", "g", 2, 2, 0),
            tile!("H", "h", 2, 4, 0),
            tile!("I", "i", 8, 1, 1),
            tile!("J", "j", 1, 8, 0),
            tile!("K", "k", 1, 10, 0),
            tile!("L", "l", 5, 1, 0),
            tile!("M", "m", 3, 2, 0),
            tile!("N", "n", 6, 1, 0),
            tile!("O", "o", 6, 1, 1),
            tile!("P", "p", 2, 3, 0),
            tile!("Q", "q", 1, 8, 0),
            tile!("R", "r", 6, 1, 0),
            tile!("S", "s", 6, 1, 0),
            tile!("T", "t", 6, 1, 0),
            tile!("U", "u", 6, 1, 1),
            tile!("V", "v", 2, 4, 0),
            tile!("W", "w", 1, 10, 0),
            tile!("X", "x", 1, 10, 0),
            tile!("Y", "y", 1, 10, 1),
            tile!("Z", "z", 1, 10, 0),
        ],
        ..Default::default()
    })
}

// http://hkcrosswordclub.com/?cat=14
pub fn make_hong_kong_english_alphabet() -> Alphabet {
    Alphabet::new_static(StaticAlphabet {
        tiles: vec![
            tile!("?", "?", 4, 0, 0),
            tile!("A", "a", 9, 1, 1),
            tile!("B", "b", 2, 3, 0),
            tile!("C", "c", 2, 3, 0),
            tile!("D", "d", 4, 2, 0),
            tile!("E", "e", 12, 1, 1),
            tile!("F", "f", 2, 4, 0),
            tile!("G", "g", 3, 2, 0),
            tile!("H", "h", 2, 4, 0),
            tile!("I", "i", 9, 1, 1),
            tile!("J", "j", 2, 8, 0),
            tile!("K", "k", 2, 5, 0),
            tile!("L", "l", 4, 1, 0),
            tile!("M", "m", 2, 3, 0),
            tile!("N", "n", 6, 1, 0),
            tile!("O", "o", 8, 1, 1),
            tile!("P", "p", 2, 3, 0),
            tile!("Q", "q", 2, 10, 0),
            tile!("R", "r", 6, 1, 0),
            tile!("S", "s", 4, 1, 0),
            tile!("T", "t", 6, 1, 0),
            tile!("U", "u", 4, 1, 1),
            tile!("V", "v", 2, 4, 0),
            tile!("W", "w", 2, 4, 0),
            tile!("X", "x", 2, 8, 0),
            tile!("Y", "y", 2, 4, 0),
            tile!("Z", "z", 2, 10, 0),
        ],
        ..Default::default()
    })
}

// https://en.wikipedia.org/wiki/Super_Scrabble
pub fn make_super_english_alphabet() -> Alphabet {
    Alphabet::new_static(StaticAlphabet {
        tiles: vec![
            tile!("?", "?", 4, 0, 0),
            tile!("A", "a", 16, 1, 1),
            tile!("B", "b", 4, 3, 0),
            tile!("C", "c", 6, 3, 0),
            tile!("D", "d", 8, 2, 0),
            tile!("E", "e", 24, 1, 1),
            tile!("F", "f", 4, 4, 0),
            tile!("G", "g", 5, 2, 0),
            tile!("H", "h", 5, 4, 0),
            tile!("I", "i", 13, 1, 1),
            tile!("J", "j", 2, 8, 0),
            tile!("K", "k", 2, 5, 0),
            tile!("L", "l", 7, 1, 0),
            tile!("M", "m", 6, 3, 0),
            tile!("N", "n", 13, 1, 0),
            tile!("O", "o", 15, 1, 1),
            tile!("P", "p", 4, 3, 0),
            tile!("Q", "q", 2, 10, 0),
            tile!("R", "r", 13, 1, 0),
            tile!("S", "s", 10, 1, 0),
            tile!("T", "t", 15, 1, 0),
            tile!("U", "u", 7, 1, 1),
            tile!("V", "v", 3, 4, 0),
            tile!("W", "w", 4, 4, 0),
            tile!("X", "x", 2, 8, 0),
            tile!("Y", "y", 4, 4, 0),
            tile!("Z", "z", 2, 10, 0),
        ],
        ..Default::default()
    })
}

// https://en.wikipedia.org/wiki/Scrabble_letter_distributions#German
pub fn make_german_alphabet() -> Alphabet {
    Alphabet::new_static(StaticAlphabet {
        tiles: vec![
            tile!("?", "?", 2, 0, 0),
            tile!("A", "a", 5, 1, 1),
            tile!("Ä", "ä", 1, 6, 1),
            tile!("B", "b", 2, 3, 0),
            tile!("C", "c", 2, 4, 0),
            tile!("D", "d", 4, 1, 0),
            tile!("E", "e", 15, 1, 1),
            tile!("F", "f", 2, 4, 0),
            tile!("G", "g", 3, 2, 0),
            tile!("H", "h", 4, 2, 0),
            tile!("I", "i", 6, 1, 1),
            tile!("J", "j", 1, 6, 0),
            tile!("K", "k", 2, 4, 0),
            tile!("L", "l", 3, 2, 0),
            tile!("M", "m", 4, 3, 0),
            tile!("N", "n", 9, 1, 0),
            tile!("O", "o", 3, 2, 1),
            tile!("Ö", "ö", 1, 8, 1),
            tile!("P", "p", 1, 4, 0),
            tile!("Q", "q", 1, 10, 0),
            tile!("R", "r", 6, 1, 0),
            tile!("S", "s", 7, 1, 0),
            tile!("T", "t", 6, 1, 0),
            tile!("U", "u", 6, 1, 1),
            tile!("Ü", "ü", 1, 6, 1),
            tile!("V", "v", 1, 6, 0),
            tile!("W", "w", 1, 3, 0),
            tile!("X", "x", 1, 8, 0),
            tile!("Y", "y", 1, 10, 0),
            tile!("Z", "z", 1, 3, 0),
        ],
        ..Default::default()
    })
}

// https://en.wikipedia.org/wiki/Scrabble_letter_distributions#Norwegian
// https://en.wikipedia.org/wiki/Norwegian_orthography
// https://unicode.org/mail-arch/unicode-ml/y2002-m01/0297.html
// also this ordering matches system locale files
pub fn make_norwegian_alphabet() -> Alphabet {
    Alphabet::new_static(StaticAlphabet {
        tiles: vec![
            tile!("?", "?", 2, 0, 0),
            tile!("A", "a", 7, 1, 1),
            tile!("B", "b", 3, 4, 0),
            tile!("C", "c", 1, 10, 0),
            tile!("D", "d", 5, 1, 0),
            tile!("E", "e", 9, 1, 1),
            tile!("F", "f", 4, 2, 0),
            tile!("G", "g", 4, 2, 0),
            tile!("H", "h", 3, 3, 0),
            tile!("I", "i", 5, 1, 1),
            tile!("J", "j", 2, 4, 0),
            tile!("K", "k", 4, 2, 0),
            tile!("L", "l", 5, 1, 0),
            tile!("M", "m", 3, 2, 0),
            tile!("N", "n", 6, 1, 0),
            tile!("O", "o", 4, 2, 1),
            tile!("P", "p", 2, 4, 0),
            tile!("Q", "q", 0, 0, 0),
            tile!("R", "r", 6, 1, 0),
            tile!("S", "s", 6, 1, 0),
            tile!("T", "t", 6, 1, 0),
            tile!("U", "u", 3, 4, 1),
            tile!("V", "v", 3, 4, 0),
            tile!("W", "w", 1, 8, 0),
            tile!("X", "x", 0, 0, 0),
            tile!("Y", "y", 1, 6, 1),
            tile!("Ü", "ü", 0, 0, 1),
            tile!("Z", "z", 0, 0, 0),
            tile!("Æ", "æ", 1, 6, 1),
            tile!("Ä", "ä", 0, 0, 1),
            tile!("Ø", "ø", 2, 5, 1),
            tile!("Ö", "ö", 0, 0, 1),
            tile!("Å", "å", 2, 4, 1),
        ],
        ..Default::default()
    })
}

// https://en.wikipedia.org/wiki/Scrabble_letter_distributions#Polish
// https://en.wikipedia.org/wiki/Polish_alphabet#Letters
// https://en.wikipedia.org/wiki/Polish_phonology#Vowels
pub fn make_polish_alphabet() -> Alphabet {
    Alphabet::new_static(StaticAlphabet {
        tiles: vec![
            tile!("?", "?", 2, 0, 0),
            tile!("A", "a", 9, 1, 1),
            tile!("Ą", "ą", 1, 5, 1),
            tile!("B", "b", 2, 3, 0),
            tile!("C", "c", 3, 2, 0),
            tile!("Ć", "ć", 1, 6, 0),
            tile!("D", "d", 3, 2, 0),
            tile!("E", "e", 7, 1, 1),
            tile!("Ę", "ę", 1, 5, 1),
            tile!("F", "f", 1, 5, 0),
            tile!("G", "g", 2, 3, 0),
            tile!("H", "h", 2, 3, 0),
            tile!("I", "i", 8, 1, 1),
            tile!("J", "j", 2, 3, 0),
            tile!("K", "k", 3, 2, 0),
            tile!("L", "l", 3, 2, 0),
            tile!("Ł", "ł", 2, 3, 0),
            tile!("M", "m", 3, 2, 0),
            tile!("N", "n", 5, 1, 0),
            tile!("Ń", "ń", 1, 7, 0),
            tile!("O", "o", 6, 1, 1),
            tile!("Ó", "ó", 1, 5, 1),
            tile!("P", "p", 3, 2, 0),
            tile!("R", "r", 4, 1, 0),
            tile!("S", "s", 4, 1, 0),
            tile!("Ś", "ś", 1, 5, 0),
            tile!("T", "t", 3, 2, 0),
            tile!("U", "u", 2, 3, 1),
            tile!("W", "w", 4, 1, 0),
            tile!("Y", "y", 4, 2, 1),
            tile!("Z", "z", 5, 1, 0),
            tile!("Ź", "ź", 1, 9, 0),
            tile!("Ż", "ż", 1, 5, 0),
        ],
        ..Default::default()
    })
}

// https://en.wikipedia.org/wiki/Scrabble_letter_distributions#Slovenian
// the additional letters are unofficial and experimental
// (so data files may not be stable).
pub fn make_slovene_alphabet() -> Alphabet {
    Alphabet::new_static(StaticAlphabet {
        tiles: vec![
            tile!("?", "?", 2, 0, 0),
            tile!("A", "a", 10, 1, 1),
            tile!("Å", "å", 0, 0, 1), // ?
            tile!("Ä", "ä", 0, 0, 1), // ?
            tile!("B", "b", 2, 4, 0),
            tile!("C", "c", 1, 8, 0),
            tile!("Ç", "ç", 0, 0, 0), // ?
            tile!("Č", "č", 1, 5, 0),
            tile!("D", "d", 4, 2, 0),
            tile!("E", "e", 11, 1, 1),
            tile!("F", "f", 1, 10, 0),
            tile!("G", "g", 2, 4, 0),
            tile!("H", "h", 1, 5, 0),
            tile!("I", "i", 9, 1, 1),
            tile!("J", "j", 4, 1, 0),
            tile!("K", "k", 3, 3, 0),
            tile!("L", "l", 4, 1, 0),
            tile!("M", "m", 2, 3, 0),
            tile!("N", "n", 7, 1, 0),
            tile!("Ñ", "ñ", 0, 0, 0), // ?
            tile!("O", "o", 8, 1, 1),
            tile!("Ö", "ö", 0, 0, 1), // ?
            tile!("P", "p", 2, 3, 0),
            tile!("Q", "q", 0, 0, 0), // ?
            tile!("R", "r", 6, 1, 0),
            tile!("S", "s", 6, 1, 0),
            tile!("Š", "š", 1, 6, 0),
            tile!("T", "t", 4, 1, 0),
            tile!("U", "u", 2, 3, 1),
            tile!("Ü", "ü", 0, 0, 1), // ?
            tile!("V", "v", 4, 2, 0),
            tile!("W", "w", 0, 0, 0), // ?
            tile!("X", "x", 0, 0, 0), // ?
            tile!("Y", "y", 0, 0, 0), // ?
            tile!("Z", "z", 2, 4, 0),
            tile!("Ž", "ž", 1, 10, 0),
        ],
        ..Default::default()
    })
}

// https://en.wikipedia.org/wiki/Scrabble_letter_distributions#Spanish
// based on Spanish-language sets sold outside North America
// (CH/LL/RR are ambiguous and should not be supported)
pub fn make_spanish_alphabet() -> Alphabet {
    Alphabet::new_static(StaticAlphabet {
        tiles: vec![
            tile!("?", "?", 2, 0, 0),
            tile!("A", "a", 12, 1, 1),
            tile!("B", "b", 2, 3, 0),
            tile!("C", "c", 4, 3, 0),
            tile!("[CH]", "[ch]", 1, 5, 0, v!["1"], v![]),
            tile!("D", "d", 5, 2, 0),
            tile!("E", "e", 12, 1, 1),
            tile!("F", "f", 1, 4, 0),
            tile!("G", "g", 2, 2, 0),
            tile!("H", "h", 2, 4, 0),
            tile!("I", "i", 6, 1, 1),
            tile!("J", "j", 1, 8, 0),
            tile!("L", "l", 4, 1, 0),
            tile!("[LL]", "[ll]", 1, 8, 0, v!["2"], v![]),
            tile!("M", "m", 2, 3, 0),
            tile!("N", "n", 5, 1, 0),
            tile!("Ñ", "ñ", 1, 8, 0),
            tile!("O", "o", 9, 1, 1),
            tile!("P", "p", 2, 3, 0),
            tile!("Q", "q", 1, 5, 0),
            tile!("R", "r", 5, 1, 0),
            tile!("[RR]", "[rr]", 1, 8, 0, v!["3"], v![]),
            tile!("S", "s", 6, 1, 0),
            tile!("T", "t", 4, 1, 0),
            tile!("U", "u", 5, 1, 1),
            tile!("V", "v", 1, 4, 0),
            tile!("X", "x", 1, 8, 0),
            tile!("Y", "y", 1, 4, 0),
            tile!("Z", "z", 1, 10, 0),
        ],
        ..Default::default()
    })
}

// TODO: find citeable source
// https://discord.com/channels/741321677828522035/778469677588283403/1171937313224392704
pub fn make_yupik_alphabet() -> Alphabet {
    Alphabet::new_static(StaticAlphabet {
        tiles: vec![
            tile!("?", "?", 2, 0, 0),
            tile!("A", "a", 17, 1, 1),
            tile!("C", "c", 2, 6, 0),
            tile!("E", "e", 6, 1, 1),
            tile!("G", "g", 5, 2, 0),
            tile!("I", "i", 9, 1, 1),
            tile!("K", "k", 5, 2, 0),
            tile!("L", "l", 8, 1, 0),
            tile!("M", "m", 4, 4, 0),
            tile!("N", "n", 8, 1, 0),
            tile!("P", "p", 1, 8, 0),
            tile!("Q", "q", 4, 4, 0),
            tile!("R", "r", 6, 1, 0),
            tile!("S", "s", 1, 8, 0),
            tile!("T", "t", 8, 1, 0),
            tile!("U", "u", 12, 1, 1),
            tile!("V", "v", 1, 10, 0),
            tile!("W", "w", 1, 10, 0),
            tile!("Y", "y", 2, 6, 0),
        ],
        ..Default::default()
    })
}

pub struct AlphabetReader<'a> {
    supported_tiles: Box<[(u8, &'a [u8])]>,
    by_first_byte: [Option<(usize, usize)>; 256],
}

// This is slow, but supports multi-codepoint tiles with greedy matching.
impl<'a> AlphabetReader<'a> {
    pub fn new_for_tiles(mut supported_tiles: Box<[(u8, &'a [u8])]>) -> Self {
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
    pub fn new_for_words(alphabet: &'a Alphabet) -> Self {
        let alphabet_len = alphabet.len();
        let mut cap = 0;
        for idx in 1..alphabet_len {
            let tile = alphabet.get(idx);
            cap += 2 + tile.alias_labels.len() + tile.alias_blank_labels.len();
        }
        let mut supported_tiles = Vec::with_capacity(cap);
        for idx in 1..alphabet_len {
            let tile = alphabet.get(idx);
            supported_tiles.push((idx, tile.label.as_bytes()));
            for alias in tile.alias_labels.iter() {
                supported_tiles.push((idx, alias.as_bytes()));
            }
            supported_tiles.push((idx, tile.blank_label.as_bytes()));
            for alias in tile.alias_blank_labels.iter() {
                supported_tiles.push((idx, alias.as_bytes()));
            }
        }
        let supported_tiles = supported_tiles.into_boxed_slice();
        Self::new_for_tiles(supported_tiles)
    }

    // Same as new_for_words but merge tiles with same score.
    pub fn new_for_word_scores(alphabet: &'a Alphabet) -> Self {
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
            supported_tiles.push((representative_idx, tile.label.as_bytes()));
            for alias in tile.alias_labels.iter() {
                supported_tiles.push((representative_idx, alias.as_bytes()));
            }
            supported_tiles.push((representative_idx, tile.blank_label.as_bytes()));
            for alias in tile.alias_blank_labels.iter() {
                supported_tiles.push((representative_idx, alias.as_bytes()));
            }
        }
        let supported_tiles = supported_tiles.into_boxed_slice();
        Self::new_for_tiles(supported_tiles)
    }

    // Recognizes [?A-Z] and [a-z] identically, as well as aliases.
    pub fn new_for_racks(alphabet: &'a Alphabet) -> Self {
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
            supported_tiles.push((0, tile.label.as_bytes()));
            for alias in tile.alias_labels.iter() {
                supported_tiles.push((0, alias.as_bytes()));
            }
        }
        for idx in 1..alphabet_len {
            let tile = alphabet.get(idx);
            supported_tiles.push((idx, tile.label.as_bytes()));
            for alias in tile.alias_labels.iter() {
                supported_tiles.push((idx, alias.as_bytes()));
            }
            supported_tiles.push((idx, tile.blank_label.as_bytes()));
            for alias in tile.alias_blank_labels.iter() {
                supported_tiles.push((idx, alias.as_bytes()));
            }
        }
        let supported_tiles = supported_tiles.into_boxed_slice();
        Self::new_for_tiles(supported_tiles)
    }

    // Recognizes [A-Za-z] and aliases. Play-through needs to be dealt with separately.
    pub fn new_for_plays(alphabet: &'a Alphabet) -> Self {
        let alphabet_len = alphabet.len();
        let mut cap = 0;
        for idx in 1..alphabet_len {
            let tile = alphabet.get(idx);
            cap += 2 + tile.alias_labels.len() + tile.alias_blank_labels.len();
        }
        let mut supported_tiles = Vec::with_capacity(cap);
        for idx in 1..alphabet_len {
            let tile = alphabet.get(idx);
            supported_tiles.push((idx, tile.label.as_bytes()));
            for alias in tile.alias_labels.iter() {
                supported_tiles.push((idx, alias.as_bytes()));
            }
            let blank_idx = idx | 0x80;
            supported_tiles.push((blank_idx, tile.blank_label.as_bytes()));
            for alias in tile.alias_blank_labels.iter() {
                supported_tiles.push((blank_idx, alias.as_bytes()));
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
