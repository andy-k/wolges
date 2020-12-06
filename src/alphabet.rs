pub struct Tile<'a> {
    pub label: &'a str,
    pub blank_label: &'a str,
    pub freq: u8,
    pub score: i8,
    pub is_vowel: bool,
}

pub struct StaticAlphabet<'a>(&'a [Tile<'a>]);

pub enum Alphabet<'a> {
    Static(StaticAlphabet<'a>),
}

impl<'a> Alphabet<'a> {
    #[inline(always)]
    pub fn len(&self) -> u8 {
        match self {
            Alphabet::Static(x) => x.0.len() as u8,
        }
    }

    #[inline(always)]
    pub fn get(&self, idx: u8) -> &'a Tile<'a> {
        match self {
            Alphabet::Static(x) => &x.0[idx as usize],
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
}

pub static ENGLISH_ALPHABET: Alphabet = Alphabet::Static(StaticAlphabet(&[
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
]));
