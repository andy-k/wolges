pub struct Tile<'a> {
    label: &'a str,
    blank_label: &'a str,
    freq: u8,
    score: i8,
    is_vowel: bool,
}

pub struct StaticAlphabet<'a> {
    tiles: &'a [Tile<'a>],
    num_tiles: u16,
}

pub enum Alphabet<'a> {
    Static(StaticAlphabet<'a>),
}

impl<'a> Alphabet<'a> {
    #[inline(always)]
    pub fn len(&self) -> u8 {
        match self {
            Alphabet::Static(x) => x.tiles.len() as u8,
        }
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
        let magic = idx & !((idx as i8) >> 7) as u8;
        let no_magic = if idx & 0x80 == 0 { idx } else { 0 };
        //println!("{} -> {} ({})", idx, magic, no_magic);
        assert_eq!(magic, no_magic);
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
}

pub static ENGLISH_ALPHABET: Alphabet = Alphabet::Static(StaticAlphabet {
    tiles: &[
        Tile {
            label: "?",
            blank_label: "?",
            freq: 2,
            score: 0,
            is_vowel: false,
        },
        Tile {
            label: "A",
            blank_label: "a",
            freq: 9,
            score: 1,
            is_vowel: true,
        },
        Tile {
            label: "B",
            blank_label: "b",
            freq: 2,
            score: 3,
            is_vowel: false,
        },
        Tile {
            label: "C",
            blank_label: "c",
            freq: 2,
            score: 3,
            is_vowel: false,
        },
        Tile {
            label: "D",
            blank_label: "d",
            freq: 4,
            score: 2,
            is_vowel: false,
        },
        Tile {
            label: "E",
            blank_label: "e",
            freq: 12,
            score: 1,
            is_vowel: true,
        },
        Tile {
            label: "F",
            blank_label: "f",
            freq: 2,
            score: 4,
            is_vowel: false,
        },
        Tile {
            label: "G",
            blank_label: "g",
            freq: 3,
            score: 2,
            is_vowel: false,
        },
        Tile {
            label: "H",
            blank_label: "h",
            freq: 2,
            score: 4,
            is_vowel: false,
        },
        Tile {
            label: "I",
            blank_label: "i",
            freq: 9,
            score: 1,
            is_vowel: true,
        },
        Tile {
            label: "J",
            blank_label: "j",
            freq: 1,
            score: 8,
            is_vowel: false,
        },
        Tile {
            label: "K",
            blank_label: "k",
            freq: 1,
            score: 5,
            is_vowel: false,
        },
        Tile {
            label: "L",
            blank_label: "l",
            freq: 4,
            score: 1,
            is_vowel: false,
        },
        Tile {
            label: "M",
            blank_label: "m",
            freq: 2,
            score: 3,
            is_vowel: false,
        },
        Tile {
            label: "N",
            blank_label: "n",
            freq: 6,
            score: 1,
            is_vowel: false,
        },
        Tile {
            label: "O",
            blank_label: "o",
            freq: 8,
            score: 1,
            is_vowel: true,
        },
        Tile {
            label: "P",
            blank_label: "p",
            freq: 2,
            score: 3,
            is_vowel: false,
        },
        Tile {
            label: "Q",
            blank_label: "q",
            freq: 1,
            score: 10,
            is_vowel: false,
        },
        Tile {
            label: "R",
            blank_label: "r",
            freq: 6,
            score: 1,
            is_vowel: false,
        },
        Tile {
            label: "S",
            blank_label: "s",
            freq: 4,
            score: 1,
            is_vowel: false,
        },
        Tile {
            label: "T",
            blank_label: "t",
            freq: 6,
            score: 1,
            is_vowel: false,
        },
        Tile {
            label: "U",
            blank_label: "u",
            freq: 4,
            score: 1,
            is_vowel: true,
        },
        Tile {
            label: "V",
            blank_label: "v",
            freq: 2,
            score: 4,
            is_vowel: false,
        },
        Tile {
            label: "W",
            blank_label: "w",
            freq: 2,
            score: 4,
            is_vowel: false,
        },
        Tile {
            label: "X",
            blank_label: "x",
            freq: 1,
            score: 8,
            is_vowel: false,
        },
        Tile {
            label: "Y",
            blank_label: "y",
            freq: 2,
            score: 4,
            is_vowel: false,
        },
        Tile {
            label: "Z",
            blank_label: "z",
            freq: 1,
            score: 10,
            is_vowel: false,
        },
    ],
    num_tiles: 100,
});

pub static POLISH_ALPHABET: Alphabet = Alphabet::Static(StaticAlphabet {
    tiles: &[
        Tile {
            label: "?",
            blank_label: "?",
            freq: 2,
            score: 0,
            is_vowel: false,
        },
        Tile {
            label: "A",
            blank_label: "a",
            freq: 9,
            score: 1,
            is_vowel: true,
        },
        Tile {
            label: "Ą",
            blank_label: "ą",
            freq: 1,
            score: 5,
            is_vowel: true,
        },
        Tile {
            label: "B",
            blank_label: "b",
            freq: 2,
            score: 3,
            is_vowel: false,
        },
        Tile {
            label: "C",
            blank_label: "c",
            freq: 3,
            score: 2,
            is_vowel: false,
        },
        Tile {
            label: "Ć",
            blank_label: "ć",
            freq: 1,
            score: 6,
            is_vowel: false,
        },
        Tile {
            label: "D",
            blank_label: "d",
            freq: 3,
            score: 2,
            is_vowel: false,
        },
        Tile {
            label: "E",
            blank_label: "e",
            freq: 7,
            score: 1,
            is_vowel: true,
        },
        Tile {
            label: "Ę",
            blank_label: "ę",
            freq: 1,
            score: 5,
            is_vowel: true,
        },
        Tile {
            label: "F",
            blank_label: "f",
            freq: 1,
            score: 5,
            is_vowel: false,
        },
        Tile {
            label: "G",
            blank_label: "g",
            freq: 2,
            score: 3,
            is_vowel: false,
        },
        Tile {
            label: "H",
            blank_label: "h",
            freq: 2,
            score: 3,
            is_vowel: false,
        },
        Tile {
            label: "I",
            blank_label: "i",
            freq: 8,
            score: 1,
            is_vowel: true,
        },
        Tile {
            label: "J",
            blank_label: "j",
            freq: 2,
            score: 3,
            is_vowel: false,
        },
        Tile {
            label: "K",
            blank_label: "k",
            freq: 3,
            score: 2,
            is_vowel: false,
        },
        Tile {
            label: "L",
            blank_label: "l",
            freq: 3,
            score: 2,
            is_vowel: false,
        },
        Tile {
            label: "Ł",
            blank_label: "ł",
            freq: 2,
            score: 3,
            is_vowel: false,
        },
        Tile {
            label: "M",
            blank_label: "m",
            freq: 3,
            score: 2,
            is_vowel: false,
        },
        Tile {
            label: "N",
            blank_label: "n",
            freq: 5,
            score: 1,
            is_vowel: false,
        },
        Tile {
            label: "Ń",
            blank_label: "ń",
            freq: 1,
            score: 7,
            is_vowel: false,
        },
        Tile {
            label: "O",
            blank_label: "o",
            freq: 6,
            score: 1,
            is_vowel: true,
        },
        Tile {
            label: "Ó",
            blank_label: "ó",
            freq: 1,
            score: 5,
            is_vowel: true,
        },
        Tile {
            label: "P",
            blank_label: "p",
            freq: 3,
            score: 2,
            is_vowel: false,
        },
        Tile {
            label: "R",
            blank_label: "r",
            freq: 4,
            score: 1,
            is_vowel: false,
        },
        Tile {
            label: "S",
            blank_label: "s",
            freq: 4,
            score: 1,
            is_vowel: false,
        },
        Tile {
            label: "Ś",
            blank_label: "ś",
            freq: 1,
            score: 5,
            is_vowel: false,
        },
        Tile {
            label: "T",
            blank_label: "t",
            freq: 3,
            score: 2,
            is_vowel: false,
        },
        Tile {
            label: "U",
            blank_label: "u",
            freq: 2,
            score: 3,
            is_vowel: true,
        },
        Tile {
            label: "W",
            blank_label: "w",
            freq: 4,
            score: 1,
            is_vowel: false,
        },
        Tile {
            label: "Y",
            blank_label: "y",
            freq: 4,
            score: 2,
            is_vowel: true,
        },
        Tile {
            label: "Z",
            blank_label: "z",
            freq: 5,
            score: 1,
            is_vowel: false,
        },
        Tile {
            label: "Ź",
            blank_label: "ź",
            freq: 1,
            score: 9,
            is_vowel: false,
        },
        Tile {
            label: "Ż",
            blank_label: "ż",
            freq: 1,
            score: 5,
            is_vowel: false,
        },
    ],
    num_tiles: 100,
});
