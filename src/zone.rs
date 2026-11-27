// Copyright (C) 2020-2026 Andy Kurnia.

// a play's footprint on a board, and the lanes a change inside it can reach

use super::{error, matrix, movegen};

// a board's squares as bits, for a board of up to SQUARE_WORDS * 64 of them
pub const SQUARE_WORDS: usize = 8;

#[derive(Clone)]
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
pub fn footprint(
    dim: &matrix::Dim,
    board_tiles: &[u8],
    play: &movegen::Play,
) -> error::Returns<Footprint> {
    if let movegen::Play::Place {
        down,
        lane,
        idx,
        word,
        ..
    } = play
    {
        dim.check_span(*down, *lane, *idx, word.len())?;
        let area = area(dim)?;
        if board_tiles.len() < area {
            crate::return_error!("the board is shorter than its dimensions".to_string());
        }
    }
    Ok(footprint_on_board(dim, board_tiles, play))
}

#[inline]
pub(crate) fn footprint_on_board(
    dim: &matrix::Dim,
    board_tiles: &[u8],
    play: &movegen::Play,
) -> Footprint {
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
    let end = usize::from(idx.cast_unsigned()) + word.len();
    if idx > 0 {
        zone(strider.at_in_lane(idx - 1));
    }
    if end < usize::from(strider.len().cast_unsigned()) {
        zone(strider.at_in_lane(end as i8));
    }
    for (pos, &tile) in (idx..).zip(word.iter()) {
        if tile == 0 {
            continue;
        }
        let square = strider.at_in_lane(pos);
        fills(square);
        zone(square);
        let across = dim.lane(!down, pos);
        let mut j = lane - 1;
        while j >= 0 && board_tiles[across.at_in_lane(j)] != 0 {
            j -= 1;
        }
        if j >= 0 {
            zone(across.at_in_lane(j));
        }
        let mut j = lane + 1;
        while j < across.len() && board_tiles[across.at_in_lane(j)] != 0 {
            j += 1;
        }
        if j < across.len() {
            zone(across.at_in_lane(j));
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
pub fn zone_lanes(dim: &matrix::Dim, zone: &Squares) -> error::Returns<(u128, u128)> {
    let area = area(dim)?;
    let mut outside = false;
    zone.each(|square| outside |= square >= area);
    if outside {
        crate::return_error!("zone contains a square outside the board".to_string());
    }
    Ok(zone_lanes_on_board(dim, zone))
}

fn area(dim: &matrix::Dim) -> error::Returns<usize> {
    let (rows, cols) = (usize::try_from(dim.rows)?, usize::try_from(dim.cols)?);
    let area = rows * cols;
    if cols == 0 || rows == 0 || area > SQUARE_WORDS * u64::BITS as usize {
        crate::return_error!("board dimensions do not fit the zone".to_string());
    }
    Ok(area)
}

#[inline(always)]
pub(crate) fn zone_lanes_on_board(dim: &matrix::Dim, zone: &Squares) -> (u128, u128) {
    let (mut rows, mut cols) = (0u128, 0u128);
    let n = usize::from(dim.cols.cast_unsigned());
    zone.each(|square| {
        rows |= 1 << (square / n);
        cols |= 1 << (square % n);
    });
    (rows, cols)
}
