// Copyright (C) 2020-2022 Andy Kurnia.

pub struct Tile<'a> {
    label: &'a str,
    blank_label: &'a str,
    freq: u8,
    score: i8,
    is_vowel: bool,
}

#[derive(Default)]
pub struct StaticAlphabet<'a> {
    tiles: &'a [Tile<'a>],
    num_tiles: u16,
}

pub enum Alphabet<'a> {
    Static(StaticAlphabet<'a>),
}

impl<'a> Alphabet<'a> {
    pub fn new_static(x: StaticAlphabet<'a>) -> Self {
        Self::Static(StaticAlphabet {
            num_tiles: x.tiles.iter().map(|tile| tile.freq as u16).sum(),
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
    pub fn get(&self, idx: u8) -> &'a Tile<'a> {
        match self {
            Alphabet::Static(x) => &x.tiles[idx as usize],
        }
    }

    #[inline(always)]
    pub fn num_tiles(&self) -> u16 {
        match self {
            Alphabet::Static(x) => x.num_tiles,
        }
    }

    #[inline(always)]
    pub fn of_board(&self, idx: u8) -> Option<&'a str> {
        let c = idx & 0x7f;
        if c == 0 || c >= self.len() {
            None
        } else if idx & 0x80 == 0 {
            Some(self.get(c).label)
        } else {
            Some(self.get(c).blank_label)
        }
    }

    #[inline(always)]
    pub fn of_rack(&self, idx: u8) -> Option<&'a str> {
        if idx >= self.len() {
            None
        } else {
            Some(self.get(idx).label)
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

    pub fn fmt_rack(&'a self, rack: &'a [u8]) -> WriteableRack<'a> {
        WriteableRack {
            alphabet: self,
            rack,
        }
    }

    pub fn rack_score(&self, rack: &[u8]) -> i16 {
        rack.iter().map(|&t| self.score(t) as i16).sum::<i16>()
    }
}

pub struct WriteableRack<'a> {
    alphabet: &'a Alphabet<'a>,
    rack: &'a [u8],
}

impl std::fmt::Display for WriteableRack<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        for &tile in self.rack {
            write!(f, "{}", self.alphabet.of_rack(tile).unwrap())?;
        }
        Ok(())
    }
}

macro_rules! tile {
    ($label: expr, $blank_label: expr, $freq: expr, $score: expr, $vowel_int: expr) => {
        Tile {
            label: $label,
            blank_label: $blank_label,
            freq: $freq,
            score: $score,
            is_vowel: ($vowel_int) != 0,
        }
    };
}

// https://en.wikipedia.org/wiki/Scrabble_letter_distributions#English
pub fn make_english_alphabet<'a>() -> Alphabet<'a> {
    Alphabet::new_static(StaticAlphabet {
        tiles: &[
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
pub fn make_french_alphabet<'a>() -> Alphabet<'a> {
    Alphabet::new_static(StaticAlphabet {
        tiles: &[
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
pub fn make_hong_kong_english_alphabet<'a>() -> Alphabet<'a> {
    Alphabet::new_static(StaticAlphabet {
        tiles: &[
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
pub fn make_super_english_alphabet<'a>() -> Alphabet<'a> {
    Alphabet::new_static(StaticAlphabet {
        tiles: &[
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
pub fn make_german_alphabet<'a>() -> Alphabet<'a> {
    Alphabet::new_static(StaticAlphabet {
        tiles: &[
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
// https://webcache.googleusercontent.com/search?q=cache:z-CdwfSoN-IJ:unicode.org/mail-arch/unicode-ml/y2002-m01/0297.html
// also this ordering matches system locale files
pub fn make_norwegian_alphabet<'a>() -> Alphabet<'a> {
    Alphabet::new_static(StaticAlphabet {
        tiles: &[
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
pub fn make_polish_alphabet<'a>() -> Alphabet<'a> {
    Alphabet::new_static(StaticAlphabet {
        tiles: &[
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

// https://en.wikipedia.org/wiki/Scrabble_letter_distributions#Spanish
// based on Spanish-language sets sold outside North America
pub fn make_spanish_alphabet<'a>() -> Alphabet<'a> {
    Alphabet::new_static(StaticAlphabet {
        tiles: &[
            tile!("?", "?", 2, 0, 0),
            tile!("A", "a", 12, 1, 1),
            tile!("B", "b", 2, 3, 0),
            tile!("C", "c", 4, 3, 0),
            tile!("CH", "ch", 1, 5, 0),
            tile!("D", "d", 5, 2, 0),
            tile!("E", "e", 12, 1, 1),
            tile!("F", "f", 1, 4, 0),
            tile!("G", "g", 2, 2, 0),
            tile!("H", "h", 2, 4, 0),
            tile!("I", "i", 6, 1, 1),
            tile!("J", "j", 1, 8, 0),
            tile!("L", "l", 4, 1, 0),
            tile!("LL", "ll", 1, 8, 0),
            tile!("M", "m", 2, 3, 0),
            tile!("N", "n", 5, 1, 0),
            tile!("Ñ", "ñ", 1, 8, 0),
            tile!("O", "o", 9, 1, 1),
            tile!("P", "p", 2, 3, 0),
            tile!("Q", "q", 1, 5, 0),
            tile!("R", "r", 5, 1, 0),
            tile!("RR", "rr", 1, 8, 0),
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

pub struct AlphabetReader<'a> {
    supported_tiles: Box<[(u8, &'a [u8])]>,
    by_first_byte: [Option<(Option<u8>, usize, usize)>; 256],
}

// This is slow, but supports multi-codepoint tiles with greedy matching.
// For example, a CH/LL/RR tile will parse correctly.
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
            let (tile, label) = supported_tiles[i];
            let label0 = label[0];
            let mut j = i;
            while j > 0 && supported_tiles[j - 1].1[0] == label0 {
                j -= 1;
            }
            h[label0 as usize] = Some(if label.len() > 1 {
                (None, j, i + 1)
            } else {
                (Some(tile), j, i)
            });
            i = j;
        }
        Self {
            supported_tiles,
            by_first_byte: h,
        }
    }

    // Recognizes [A-Z].
    pub fn new_for_words(alphabet: &Alphabet<'a>) -> Self {
        let supported_tiles = (1..alphabet.len())
            .map(|tile| (tile, alphabet.of_rack(tile).unwrap().as_bytes()))
            .collect::<Box<_>>();
        Self::new_for_tiles(supported_tiles)
    }

    // Recognizes [?A-Z].
    pub fn new_for_racks(alphabet: &Alphabet<'a>) -> Self {
        let supported_tiles = (0..alphabet.len())
            .map(|tile| (tile, alphabet.of_rack(tile).unwrap().as_bytes()))
            .collect::<Box<_>>();
        Self::new_for_tiles(supported_tiles)
    }

    // Recognizes [A-Za-z]. Play-through needs to be dealt with separately.
    pub fn new_for_plays(alphabet: &Alphabet<'a>) -> Self {
        let mut supported_tiles = Vec::with_capacity((alphabet.len() - 1) as usize * 2);
        for base_tile in 1..alphabet.len() {
            for &tile in &[base_tile, base_tile | 0x80] {
                supported_tiles.push((tile, alphabet.of_board(tile).unwrap().as_bytes()));
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
        if let Some((if_single, range_lo, range_hi)) = unsafe {
            self.by_first_byte
                .get_unchecked(*sb.get_unchecked(ix) as usize)
        } {
            if range_hi > range_lo {
                // Safe after accessing sb[ix].
                let sb_remainder = unsafe { sb.get_unchecked(ix + 1..) };
                // Safe because of how by_first_byte was constructed.
                for (tile, label) in
                    unsafe { self.supported_tiles.get_unchecked(*range_lo..*range_hi) }
                {
                    if sb_remainder.starts_with(unsafe { label.get_unchecked(1..) }) {
                        return Some((*tile, ix + label.len()));
                    }
                }
            }
            return if_single.map(|tile| (tile, ix + 1));
        }
        None
    }
}
