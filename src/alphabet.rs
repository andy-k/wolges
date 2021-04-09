// Copyright (C) 2020-2021 Andy Kurnia.

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
    pub fn from_board(&self, idx: u8) -> Option<&'a str> {
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
    pub fn from_rack(&self, idx: u8) -> Option<&'a str> {
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

    pub fn fmt_rack(&'a self, rack: &'a [u8]) -> WriteableRack<'a> {
        WriteableRack {
            alphabet: &self,
            rack: &rack,
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
            write!(f, "{}", self.alphabet.from_rack(tile).unwrap())?;
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
pub fn make_norwegian_alphabet<'a>() -> Alphabet<'a> {
    Alphabet::new_static(StaticAlphabet {
        tiles: &[
            tile!("?", "?", 2, 0, 0),
            tile!("A", "a", 7, 1, 1),
            tile!("Ä", "ä", 0, 0, 0),
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
            tile!("Ö", "ö", 0, 0, 0),
            tile!("P", "p", 2, 4, 0),
            tile!("Q", "q", 0, 0, 0),
            tile!("R", "r", 6, 1, 0),
            tile!("S", "s", 6, 1, 0),
            tile!("T", "t", 6, 1, 0),
            tile!("U", "u", 3, 4, 1),
            tile!("Ü", "ü", 0, 0, 0),
            tile!("V", "v", 3, 4, 0),
            tile!("W", "w", 1, 8, 0),
            tile!("X", "x", 0, 0, 0),
            tile!("Y", "y", 1, 6, 1),
            tile!("Z", "z", 0, 0, 0),
            tile!("Æ", "æ", 1, 6, 1),
            tile!("Ø", "ø", 2, 5, 1),
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
