pub struct Tile<'a> {
    label: &'a str,
    blank_label: &'a str,
    freq: i16,
    score: i8,
    is_vowel: bool,
}

pub trait Alphabet<'a> {
    fn len(&self) -> u8;
    fn get(&self, idx: u8) -> &'a Tile<'a>;

    #[inline(always)]
    fn from_board(&self, idx: u8) -> Option<&'a str> {
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

pub struct GenericAlphabet<'a>(&'a [Tile<'a>]);

impl<'a> Alphabet<'a> for GenericAlphabet<'a> {

    #[inline(always)]
    fn len(&self) -> u8 {
        self.0.len() as u8
    }

    #[inline(always)]
    fn get(&self, idx: u8) -> &'a Tile<'a> {
        &self.0[idx as usize]
    }
}

pub static ENGLISH_ALPHABET: GenericAlphabet = GenericAlphabet(&[
    Tile {
        label: "?",
        blank_label: "?",
        freq: 0,
        score: 2,
        is_vowel: false,
    },
    Tile {
        label: "A",
        blank_label: "a",
        freq: 1,
        score: 9,
        is_vowel: true,
    },
    Tile {
        label: "B",
        blank_label: "b",
        freq: 3,
        score: 2,
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
        label: "D",
        blank_label: "d",
        freq: 2,
        score: 4,
        is_vowel: false,
    },
    Tile {
        label: "E",
        blank_label: "e",
        freq: 1,
        score: 12,
        is_vowel: true,
    },
    Tile {
        label: "F",
        blank_label: "f",
        freq: 4,
        score: 2,
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
        freq: 4,
        score: 2,
        is_vowel: false,
    },
    Tile {
        label: "I",
        blank_label: "i",
        freq: 1,
        score: 9,
        is_vowel: true,
    },
    Tile {
        label: "J",
        blank_label: "j",
        freq: 8,
        score: 1,
        is_vowel: false,
    },
    Tile {
        label: "K",
        blank_label: "k",
        freq: 5,
        score: 1,
        is_vowel: false,
    },
    Tile {
        label: "L",
        blank_label: "l",
        freq: 1,
        score: 4,
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
        freq: 1,
        score: 6,
        is_vowel: false,
    },
    Tile {
        label: "O",
        blank_label: "o",
        freq: 1,
        score: 8,
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
        label: "Q",
        blank_label: "q",
        freq: 10,
        score: 1,
        is_vowel: false,
    },
    Tile {
        label: "R",
        blank_label: "r",
        freq: 1,
        score: 6,
        is_vowel: false,
    },
    Tile {
        label: "S",
        blank_label: "s",
        freq: 1,
        score: 4,
        is_vowel: false,
    },
    Tile {
        label: "T",
        blank_label: "t",
        freq: 1,
        score: 6,
        is_vowel: false,
    },
    Tile {
        label: "U",
        blank_label: "u",
        freq: 1,
        score: 4,
        is_vowel: true,
    },
    Tile {
        label: "V",
        blank_label: "v",
        freq: 4,
        score: 2,
        is_vowel: false,
    },
    Tile {
        label: "W",
        blank_label: "w",
        freq: 4,
        score: 2,
        is_vowel: false,
    },
    Tile {
        label: "X",
        blank_label: "x",
        freq: 6,
        score: 1,
        is_vowel: false,
    },
    Tile {
        label: "Y",
        blank_label: "y",
        freq: 4,
        score: 2,
        is_vowel: false,
    },
    Tile {
        label: "Z",
        blank_label: "z",
        freq: 10,
        score: 1,
        is_vowel: false,
    },
]);
