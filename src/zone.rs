// Copyright (C) 2020-2026 Andy Kurnia.

// a play's footprint on a board, and the lanes a change inside it can reach

use super::{matrix, movegen};

// a board's squares as bits, for a board of up to SQUARE_WORDS * 64 of them
pub const SQUARE_WORDS: usize = 8;

#[derive(Clone, Copy)]
pub struct Squares(pub [u64; SQUARE_WORDS]);

impl Squares {
    pub const NONE: Self = Self([0; SQUARE_WORDS]);

    #[inline(always)]
    pub fn set(&mut self, square: usize) {
        self.0[square >> 6] |= 1 << (square & 63);
    }

    #[inline(always)]
    pub fn meets(&self, other: &Self) -> bool {
        self.0.iter().zip(&other.0).any(|(a, b)| a & b != 0)
    }

    #[inline(always)]
    pub fn each(&self, mut f: impl FnMut(usize)) {
        for (w, &bits) in self.0.iter().enumerate() {
            let mut b = bits;
            while b != 0 {
                f(w * 64 + b.trailing_zeros() as usize);
                b &= b - 1;
            }
        }
    }
}

// a play's footprint on the board it is made on: the squares it fills; its
// zone, the squares whose filling would change it (those it fills, and the
// first empty square past each end of its main word and of each word it makes
// across); and the lane its main word lies on
pub struct Footprint {
    pub fills: Squares,
    pub zone: Squares,
    pub down: bool,
    pub lane: i8, // -1 when it places no tiles
}

impl Footprint {
    // whether its lane is among the rows and columns of the masks
    #[inline(always)]
    pub fn on(&self, rows: u128, cols: u128) -> bool {
        self.lane >= 0 && ((if self.down { cols } else { rows }) >> self.lane) & 1 != 0
    }
}

#[inline]
pub fn footprint(dim: &matrix::Dim, board_tiles: &[u8], play: &movegen::Play) -> Footprint {
    let mut fp = Footprint {
        fills: Squares::NONE,
        zone: Squares::NONE,
        down: false,
        lane: -1,
    };
    let movegen::Play::Place {
        down,
        lane,
        idx,
        word,
        ..
    } = play
    else {
        return fp;
    };
    fp.down = *down;
    fp.lane = *lane;
    visit_place(
        dim,
        board_tiles,
        (*down, *lane, *idx, word),
        |square| fp.fills.set(square),
        |square| fp.zone.set(square),
    );
    fp
}

#[inline]
fn visit_place(
    dim: &matrix::Dim,
    board_tiles: &[u8],
    (down, lane, idx, word): (bool, i8, i8, &[u8]),
    mut fills: impl FnMut(usize),
    mut zone: impl FnMut(usize),
) {
    let strider = dim.lane(down, lane);
    let len = word.len() as i8;
    if idx > 0 {
        zone(strider.at(idx - 1));
    }
    if idx + len < strider.len() {
        zone(strider.at(idx + len));
    }
    for (pos, &tile) in (idx..).zip(word.iter()) {
        if tile == 0 {
            continue;
        }
        let square = strider.at(pos);
        fills(square);
        zone(square);
        let across = dim.lane(!down, pos);
        let mut j = lane - 1;
        while j >= 0 && board_tiles[across.at(j)] != 0 {
            j -= 1;
        }
        if j >= 0 {
            zone(across.at(j));
        }
        let mut j = lane + 1;
        while j < across.len() && board_tiles[across.at(j)] != 0 {
            j += 1;
        }
        if j < across.len() {
            zone(across.at(j));
        }
    }
}

#[cfg(not(target_family = "wasm"))]
pub(crate) fn place_lanes(
    dim: &matrix::Dim,
    board_tiles: &[u8],
    place: (bool, i8, i8, &[u8]),
) -> (u128, u128) {
    let (mut rows, mut cols) = (0u128, 0u128);
    let width = dim.cols as usize;
    visit_place(
        dim,
        board_tiles,
        place,
        |_| {},
        |square| {
            rows |= 1 << (square / width);
            cols |= 1 << (square % width);
        },
    );
    (rows, cols)
}

// the rows and columns holding a square of a zone: the lanes whose plays a move
// with that zone can change
#[inline(always)]
pub fn zone_lanes(dim: &matrix::Dim, zone: &Squares) -> (u128, u128) {
    let (mut rows, mut cols) = (0u128, 0u128);
    let n = dim.cols as usize;
    zone.each(|square| {
        rows |= 1 << (square / n);
        cols |= 1 << (square % n);
    });
    (rows, cols)
}
