// Copyright (C) 2020-2025 Andy Kurnia.

use super::matrix;

#[derive(Clone)]
pub struct Premium {
    pub word_multiplier: i8,
    pub tile_multiplier: i8,
}

#[inline(always)]
fn qws() -> Premium {
    Premium {
        word_multiplier: 4,
        tile_multiplier: 1,
    }
}

#[inline(always)]
fn tws() -> Premium {
    Premium {
        word_multiplier: 3,
        tile_multiplier: 1,
    }
}

#[inline(always)]
fn dws() -> Premium {
    Premium {
        word_multiplier: 2,
        tile_multiplier: 1,
    }
}

#[inline(always)]
fn qls() -> Premium {
    Premium {
        word_multiplier: 1,
        tile_multiplier: 4,
    }
}

#[inline(always)]
fn tls() -> Premium {
    Premium {
        word_multiplier: 1,
        tile_multiplier: 3,
    }
}

#[inline(always)]
fn dls() -> Premium {
    Premium {
        word_multiplier: 1,
        tile_multiplier: 2,
    }
}

#[inline(always)]
fn fvs() -> Premium {
    Premium {
        word_multiplier: 1,
        tile_multiplier: 1,
    }
}

// This is a punctured square. No tile may be played on it.
#[inline(always)]
fn del() -> Premium {
    Premium {
        word_multiplier: 0,
        tile_multiplier: 0,
    }
}

#[derive(Default)]
pub struct StaticBoardLayout {
    premiums: Box<[Premium]>,
    dim: matrix::Dim,
    star_row: i8,
    star_col: i8,
    transposed_premiums: Box<[Premium]>,
    danger_star_across: Box<[bool]>,
    danger_star_down: Box<[bool]>,
    is_symmetric: bool,
}

pub enum BoardLayout {
    Static(StaticBoardLayout),
}

impl BoardLayout {
    pub fn new_static(x: StaticBoardLayout) -> Self {
        let rows_times_cols = (x.dim.rows as isize * x.dim.cols as isize) as usize;
        let mut transposed_premiums = Vec::with_capacity(rows_times_cols);
        for col in 0..x.dim.cols {
            for row in 0..x.dim.rows {
                transposed_premiums.push(x.premiums[x.dim.at_row_col(row, col)].clone());
            }
        }
        let mut danger_star_across = vec![false; x.dim.cols as usize];
        if x.star_row > 0 {
            let range_start = ((x.star_row as isize - 1) * x.dim.cols as isize) as usize;
            (0..)
                .zip(x.premiums[range_start..range_start + x.dim.cols as usize].iter())
                .for_each(|(col, premium)| {
                    if premium.tile_multiplier > 1 || premium.word_multiplier > 1 {
                        danger_star_across[col] = true;
                    }
                });
        }
        if x.star_row < x.dim.rows - 1 {
            let range_start = ((x.star_row as isize + 1) * x.dim.cols as isize) as usize;
            (0..)
                .zip(x.premiums[range_start..range_start + x.dim.cols as usize].iter())
                .for_each(|(col, premium)| {
                    if premium.tile_multiplier > 1 || premium.word_multiplier > 1 {
                        danger_star_across[col] = true;
                    }
                });
        }
        let mut danger_star_down = vec![false; x.dim.rows as usize];
        if x.star_col > 0 {
            let range_start = ((x.star_col as isize - 1) * x.dim.rows as isize) as usize;
            (0..)
                .zip(transposed_premiums[range_start..range_start + x.dim.rows as usize].iter())
                .for_each(|(row, premium)| {
                    if premium.tile_multiplier > 1 || premium.word_multiplier > 1 {
                        danger_star_down[row] = true;
                    }
                });
        }
        if x.star_col < x.dim.cols - 1 {
            let range_start = ((x.star_col as isize + 1) * x.dim.rows as isize) as usize;
            (0..)
                .zip(transposed_premiums[range_start..range_start + x.dim.rows as usize].iter())
                .for_each(|(row, premium)| {
                    if premium.tile_multiplier > 1 || premium.word_multiplier > 1 {
                        danger_star_down[row] = true;
                    }
                });
        }
        Self::Static(StaticBoardLayout {
            transposed_premiums: transposed_premiums.into_boxed_slice(),
            danger_star_across: danger_star_across.into_boxed_slice(),
            danger_star_down: danger_star_down.into_boxed_slice(),
            is_symmetric: x.dim.rows == x.dim.cols
                && x.star_row == x.star_col
                && (0..x.dim.rows).all(|row| {
                    (0..row).all(|col| {
                        let p1 = &x.premiums[x.dim.at_row_col(row, col)];
                        let p2 = &x.premiums[x.dim.at_row_col(col, row)];
                        p1.word_multiplier == p2.word_multiplier
                            && p1.tile_multiplier == p2.tile_multiplier
                    })
                }),
            ..x
        })
    }

    #[inline(always)]
    pub fn dim(&self) -> &matrix::Dim {
        match self {
            BoardLayout::Static(x) => &x.dim,
        }
    }

    #[inline(always)]
    pub fn star_row(&self) -> i8 {
        match self {
            BoardLayout::Static(x) => x.star_row,
        }
    }

    #[inline(always)]
    pub fn star_col(&self) -> i8 {
        match self {
            BoardLayout::Static(x) => x.star_col,
        }
    }

    #[inline(always)]
    pub fn premiums(&self) -> &[Premium] {
        match self {
            BoardLayout::Static(x) => &x.premiums,
        }
    }

    #[inline(always)]
    pub fn transposed_premiums(&self) -> &[Premium] {
        match self {
            BoardLayout::Static(x) => &x.transposed_premiums,
        }
    }

    #[inline(always)]
    pub fn danger_star_across(&self, col: i8) -> bool {
        match self {
            BoardLayout::Static(x) => x.danger_star_across[col as usize],
        }
    }

    #[inline(always)]
    pub fn danger_star_down(&self, row: i8) -> bool {
        match self {
            BoardLayout::Static(x) => x.danger_star_down[row as usize],
        }
    }

    // This should return false if any of these is true:
    // - dim.rows != dim.cols
    // - exists (r,c) premium at (r,c) != premium at (c,r)
    // - star_row != star_col
    #[inline(always)]
    pub fn is_symmetric(&self) -> bool {
        match self {
            BoardLayout::Static(x) => x.is_symmetric,
        }
    }
}

// https://en.wikipedia.org/wiki/Scrabble
pub fn make_standard_board_layout() -> BoardLayout {
    BoardLayout::new_static(StaticBoardLayout {
        premiums: Box::new([
            tws(),
            fvs(),
            fvs(),
            dls(),
            fvs(),
            fvs(),
            fvs(),
            tws(),
            fvs(),
            fvs(),
            fvs(),
            dls(),
            fvs(),
            fvs(),
            tws(), //
            fvs(),
            dws(),
            fvs(),
            fvs(),
            fvs(),
            tls(),
            fvs(),
            fvs(),
            fvs(),
            tls(),
            fvs(),
            fvs(),
            fvs(),
            dws(),
            fvs(), //
            fvs(),
            fvs(),
            dws(),
            fvs(),
            fvs(),
            fvs(),
            dls(),
            fvs(),
            dls(),
            fvs(),
            fvs(),
            fvs(),
            dws(),
            fvs(),
            fvs(), //
            dls(),
            fvs(),
            fvs(),
            dws(),
            fvs(),
            fvs(),
            fvs(),
            dls(),
            fvs(),
            fvs(),
            fvs(),
            dws(),
            fvs(),
            fvs(),
            dls(), //
            fvs(),
            fvs(),
            fvs(),
            fvs(),
            dws(),
            fvs(),
            fvs(),
            fvs(),
            fvs(),
            fvs(),
            dws(),
            fvs(),
            fvs(),
            fvs(),
            fvs(), //
            fvs(),
            tls(),
            fvs(),
            fvs(),
            fvs(),
            tls(),
            fvs(),
            fvs(),
            fvs(),
            tls(),
            fvs(),
            fvs(),
            fvs(),
            tls(),
            fvs(), //
            fvs(),
            fvs(),
            dls(),
            fvs(),
            fvs(),
            fvs(),
            dls(),
            fvs(),
            dls(),
            fvs(),
            fvs(),
            fvs(),
            dls(),
            fvs(),
            fvs(), //
            tws(),
            fvs(),
            fvs(),
            dls(),
            fvs(),
            fvs(),
            fvs(),
            dws(),
            fvs(),
            fvs(),
            fvs(),
            dls(),
            fvs(),
            fvs(),
            tws(), //
            fvs(),
            fvs(),
            dls(),
            fvs(),
            fvs(),
            fvs(),
            dls(),
            fvs(),
            dls(),
            fvs(),
            fvs(),
            fvs(),
            dls(),
            fvs(),
            fvs(), //
            fvs(),
            tls(),
            fvs(),
            fvs(),
            fvs(),
            tls(),
            fvs(),
            fvs(),
            fvs(),
            tls(),
            fvs(),
            fvs(),
            fvs(),
            tls(),
            fvs(), //
            fvs(),
            fvs(),
            fvs(),
            fvs(),
            dws(),
            fvs(),
            fvs(),
            fvs(),
            fvs(),
            fvs(),
            dws(),
            fvs(),
            fvs(),
            fvs(),
            fvs(), //
            dls(),
            fvs(),
            fvs(),
            dws(),
            fvs(),
            fvs(),
            fvs(),
            dls(),
            fvs(),
            fvs(),
            fvs(),
            dws(),
            fvs(),
            fvs(),
            dls(), //
            fvs(),
            fvs(),
            dws(),
            fvs(),
            fvs(),
            fvs(),
            dls(),
            fvs(),
            dls(),
            fvs(),
            fvs(),
            fvs(),
            dws(),
            fvs(),
            fvs(), //
            fvs(),
            dws(),
            fvs(),
            fvs(),
            fvs(),
            tls(),
            fvs(),
            fvs(),
            fvs(),
            tls(),
            fvs(),
            fvs(),
            fvs(),
            dws(),
            fvs(), //
            tws(),
            fvs(),
            fvs(),
            dls(),
            fvs(),
            fvs(),
            fvs(),
            tws(),
            fvs(),
            fvs(),
            fvs(),
            dls(),
            fvs(),
            fvs(),
            tws(), //
        ]),
        dim: matrix::Dim { rows: 15, cols: 15 },
        star_row: 7,
        star_col: 7,
        ..Default::default()
    })
}

// Add some punctured squares for fun. This is not an official layout.
pub fn make_punctured_board_layout() -> BoardLayout {
    BoardLayout::new_static(StaticBoardLayout {
        premiums: Box::new([
            del(),
            fvs(),
            fvs(),
            dls(),
            fvs(),
            fvs(),
            fvs(),
            tws(),
            fvs(),
            fvs(),
            fvs(),
            dls(),
            fvs(),
            fvs(),
            del(), //
            fvs(),
            dws(),
            fvs(),
            fvs(),
            fvs(),
            tls(),
            fvs(),
            del(),
            fvs(),
            tls(),
            fvs(),
            fvs(),
            fvs(),
            dws(),
            fvs(), //
            fvs(),
            fvs(),
            del(),
            fvs(),
            fvs(),
            fvs(),
            dls(),
            fvs(),
            dls(),
            fvs(),
            fvs(),
            fvs(),
            del(),
            fvs(),
            fvs(), //
            dls(),
            fvs(),
            fvs(),
            dws(),
            fvs(),
            fvs(),
            fvs(),
            dls(),
            fvs(),
            fvs(),
            fvs(),
            dws(),
            fvs(),
            fvs(),
            dls(), //
            fvs(),
            fvs(),
            fvs(),
            fvs(),
            del(),
            fvs(),
            fvs(),
            fvs(),
            fvs(),
            fvs(),
            del(),
            fvs(),
            fvs(),
            fvs(),
            fvs(), //
            fvs(),
            tls(),
            fvs(),
            fvs(),
            fvs(),
            tls(),
            fvs(),
            del(),
            fvs(),
            tls(),
            fvs(),
            fvs(),
            fvs(),
            tls(),
            fvs(), //
            fvs(),
            fvs(),
            dls(),
            fvs(),
            fvs(),
            fvs(),
            dls(),
            fvs(),
            dls(),
            fvs(),
            fvs(),
            fvs(),
            dls(),
            fvs(),
            fvs(), //
            tws(),
            del(),
            fvs(),
            dls(),
            fvs(),
            del(),
            fvs(),
            dws(),
            fvs(),
            del(),
            fvs(),
            dls(),
            fvs(),
            del(),
            tws(), //
            fvs(),
            fvs(),
            dls(),
            fvs(),
            fvs(),
            fvs(),
            dls(),
            fvs(),
            dls(),
            fvs(),
            fvs(),
            fvs(),
            dls(),
            fvs(),
            fvs(), //
            fvs(),
            tls(),
            fvs(),
            fvs(),
            fvs(),
            tls(),
            fvs(),
            del(),
            fvs(),
            tls(),
            fvs(),
            fvs(),
            fvs(),
            tls(),
            fvs(), //
            fvs(),
            fvs(),
            fvs(),
            fvs(),
            del(),
            fvs(),
            fvs(),
            fvs(),
            fvs(),
            fvs(),
            del(),
            fvs(),
            fvs(),
            fvs(),
            fvs(), //
            dls(),
            fvs(),
            fvs(),
            dws(),
            fvs(),
            fvs(),
            fvs(),
            dls(),
            fvs(),
            fvs(),
            fvs(),
            dws(),
            fvs(),
            fvs(),
            dls(), //
            fvs(),
            fvs(),
            del(),
            fvs(),
            fvs(),
            fvs(),
            dls(),
            fvs(),
            dls(),
            fvs(),
            fvs(),
            fvs(),
            del(),
            fvs(),
            fvs(), //
            fvs(),
            dws(),
            fvs(),
            fvs(),
            fvs(),
            tls(),
            fvs(),
            del(),
            fvs(),
            tls(),
            fvs(),
            fvs(),
            fvs(),
            dws(),
            fvs(), //
            del(),
            fvs(),
            fvs(),
            dls(),
            fvs(),
            fvs(),
            fvs(),
            tws(),
            fvs(),
            fvs(),
            fvs(),
            dls(),
            fvs(),
            fvs(),
            del(), //
        ]),
        dim: matrix::Dim { rows: 15, cols: 15 },
        star_row: 7,
        star_col: 7,
        ..Default::default()
    })
}

// https://www.boardgamegeek.com/image/52794/super-scrabble
pub fn make_super_board_layout() -> BoardLayout {
    BoardLayout::new_static(StaticBoardLayout {
        premiums: Box::new([
            qws(),
            fvs(),
            fvs(),
            dls(),
            fvs(),
            fvs(),
            fvs(),
            tws(),
            fvs(),
            fvs(),
            dls(),
            fvs(),
            fvs(),
            tws(),
            fvs(),
            fvs(),
            fvs(),
            dls(),
            fvs(),
            fvs(),
            qws(), //
            fvs(),
            dws(),
            fvs(),
            fvs(),
            tls(),
            fvs(),
            fvs(),
            fvs(),
            dws(),
            fvs(),
            fvs(),
            fvs(),
            dws(),
            fvs(),
            fvs(),
            fvs(),
            tls(),
            fvs(),
            fvs(),
            dws(),
            fvs(), //
            fvs(),
            fvs(),
            dws(),
            fvs(),
            fvs(),
            qls(),
            fvs(),
            fvs(),
            fvs(),
            dws(),
            fvs(),
            dws(),
            fvs(),
            fvs(),
            fvs(),
            qls(),
            fvs(),
            fvs(),
            dws(),
            fvs(),
            fvs(), //
            dls(),
            fvs(),
            fvs(),
            tws(),
            fvs(),
            fvs(),
            dls(),
            fvs(),
            fvs(),
            fvs(),
            tws(),
            fvs(),
            fvs(),
            fvs(),
            dls(),
            fvs(),
            fvs(),
            tws(),
            fvs(),
            fvs(),
            dls(), //
            fvs(),
            tls(),
            fvs(),
            fvs(),
            dws(),
            fvs(),
            fvs(),
            fvs(),
            tls(),
            fvs(),
            fvs(),
            fvs(),
            tls(),
            fvs(),
            fvs(),
            fvs(),
            dws(),
            fvs(),
            fvs(),
            tls(),
            fvs(), //
            fvs(),
            fvs(),
            qls(),
            fvs(),
            fvs(),
            dws(),
            fvs(),
            fvs(),
            fvs(),
            dls(),
            fvs(),
            dls(),
            fvs(),
            fvs(),
            fvs(),
            dws(),
            fvs(),
            fvs(),
            qls(),
            fvs(),
            fvs(), //
            fvs(),
            fvs(),
            fvs(),
            dls(),
            fvs(),
            fvs(),
            dws(),
            fvs(),
            fvs(),
            fvs(),
            dls(),
            fvs(),
            fvs(),
            fvs(),
            dws(),
            fvs(),
            fvs(),
            dls(),
            fvs(),
            fvs(),
            fvs(), //
            tws(),
            fvs(),
            fvs(),
            fvs(),
            fvs(),
            fvs(),
            fvs(),
            dws(),
            fvs(),
            fvs(),
            fvs(),
            fvs(),
            fvs(),
            dws(),
            fvs(),
            fvs(),
            fvs(),
            fvs(),
            fvs(),
            fvs(),
            tws(), //
            fvs(),
            dws(),
            fvs(),
            fvs(),
            tls(),
            fvs(),
            fvs(),
            fvs(),
            tls(),
            fvs(),
            fvs(),
            fvs(),
            tls(),
            fvs(),
            fvs(),
            fvs(),
            tls(),
            fvs(),
            fvs(),
            dws(),
            fvs(), //
            fvs(),
            fvs(),
            dws(),
            fvs(),
            fvs(),
            dls(),
            fvs(),
            fvs(),
            fvs(),
            dls(),
            fvs(),
            dls(),
            fvs(),
            fvs(),
            fvs(),
            dls(),
            fvs(),
            fvs(),
            dws(),
            fvs(),
            fvs(), //
            dls(),
            fvs(),
            fvs(),
            tws(),
            fvs(),
            fvs(),
            dls(),
            fvs(),
            fvs(),
            fvs(),
            dws(),
            fvs(),
            fvs(),
            fvs(),
            dls(),
            fvs(),
            fvs(),
            tws(),
            fvs(),
            fvs(),
            dls(), //
            fvs(),
            fvs(),
            dws(),
            fvs(),
            fvs(),
            dls(),
            fvs(),
            fvs(),
            fvs(),
            dls(),
            fvs(),
            dls(),
            fvs(),
            fvs(),
            fvs(),
            dls(),
            fvs(),
            fvs(),
            dws(),
            fvs(),
            fvs(), //
            fvs(),
            dws(),
            fvs(),
            fvs(),
            tls(),
            fvs(),
            fvs(),
            fvs(),
            tls(),
            fvs(),
            fvs(),
            fvs(),
            tls(),
            fvs(),
            fvs(),
            fvs(),
            tls(),
            fvs(),
            fvs(),
            dws(),
            fvs(), //
            tws(),
            fvs(),
            fvs(),
            fvs(),
            fvs(),
            fvs(),
            fvs(),
            dws(),
            fvs(),
            fvs(),
            fvs(),
            fvs(),
            fvs(),
            dws(),
            fvs(),
            fvs(),
            fvs(),
            fvs(),
            fvs(),
            fvs(),
            tws(), //
            fvs(),
            fvs(),
            fvs(),
            dls(),
            fvs(),
            fvs(),
            dws(),
            fvs(),
            fvs(),
            fvs(),
            dls(),
            fvs(),
            fvs(),
            fvs(),
            dws(),
            fvs(),
            fvs(),
            dls(),
            fvs(),
            fvs(),
            fvs(), //
            fvs(),
            fvs(),
            qls(),
            fvs(),
            fvs(),
            dws(),
            fvs(),
            fvs(),
            fvs(),
            dls(),
            fvs(),
            dls(),
            fvs(),
            fvs(),
            fvs(),
            dws(),
            fvs(),
            fvs(),
            qls(),
            fvs(),
            fvs(), //
            fvs(),
            tls(),
            fvs(),
            fvs(),
            dws(),
            fvs(),
            fvs(),
            fvs(),
            tls(),
            fvs(),
            fvs(),
            fvs(),
            tls(),
            fvs(),
            fvs(),
            fvs(),
            dws(),
            fvs(),
            fvs(),
            tls(),
            fvs(), //
            dls(),
            fvs(),
            fvs(),
            tws(),
            fvs(),
            fvs(),
            dls(),
            fvs(),
            fvs(),
            fvs(),
            tws(),
            fvs(),
            fvs(),
            fvs(),
            dls(),
            fvs(),
            fvs(),
            tws(),
            fvs(),
            fvs(),
            dls(), //
            fvs(),
            fvs(),
            dws(),
            fvs(),
            fvs(),
            qls(),
            fvs(),
            fvs(),
            fvs(),
            dws(),
            fvs(),
            dws(),
            fvs(),
            fvs(),
            fvs(),
            qls(),
            fvs(),
            fvs(),
            dws(),
            fvs(),
            fvs(), //
            fvs(),
            dws(),
            fvs(),
            fvs(),
            tls(),
            fvs(),
            fvs(),
            fvs(),
            dws(),
            fvs(),
            fvs(),
            fvs(),
            dws(),
            fvs(),
            fvs(),
            fvs(),
            tls(),
            fvs(),
            fvs(),
            dws(),
            fvs(), //
            qws(),
            fvs(),
            fvs(),
            dls(),
            fvs(),
            fvs(),
            fvs(),
            tws(),
            fvs(),
            fvs(),
            dls(),
            fvs(),
            fvs(),
            tws(),
            fvs(),
            fvs(),
            fvs(),
            dls(),
            fvs(),
            fvs(),
            qws(), //
        ]),
        dim: matrix::Dim { rows: 21, cols: 21 },
        star_row: 10,
        star_col: 10,
        ..Default::default()
    })
}
