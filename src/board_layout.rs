// Copyright (C) 2020-2026 Andy Kurnia.

use super::{error, matrix};
use std::str::FromStr;

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
    #[inline(always)]
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

    // a board as text: one grid line per row, starting and ending with |, each
    // square between the bars one of ~ 4W, = 3W, - 2W, ^ 4L, " 3L, ' 2L, a
    // space for a plain square and # for a punctured one; and one line "star
    // ROW COL" (counted from 0) for the starting square. A line starting with #
    // is a comment and a blank line is ignored.
    #[inline]
    pub fn new_static_from_text(s: &str) -> error::Returns<Self> {
        let mut premiums = Vec::new();
        let mut rows = 0usize;
        let mut cols = None;
        let mut star = None;
        for line in s.lines() {
            if line.trim().is_empty() || line.starts_with('#') {
                continue;
            }
            let line = line.trim_end();
            if let Some(grid) = line.strip_prefix('|') {
                let grid = grid.strip_suffix('|').ok_or("a grid line ends with |")?;
                let mut num_squares = 0usize;
                for c in grid.chars() {
                    premiums.push(match c {
                        '~' => qws(),
                        '=' => tws(),
                        '-' => dws(),
                        '^' => qls(),
                        '"' => tls(),
                        '\'' => dls(),
                        ' ' => fvs(),
                        '#' => del(),
                        _ => return Err(format!("{c:?} is not a square").into()),
                    });
                    num_squares += 1;
                }
                if num_squares == 0 || *cols.get_or_insert(num_squares) != num_squares {
                    return Err("every grid line has the first one's squares".into());
                }
                rows += 1;
            } else if let Some(at) = line.strip_prefix("star ") {
                let mut words = at.split_whitespace();
                let (Some(row), Some(col), None) = (words.next(), words.next(), words.next())
                else {
                    return Err("star takes a row and a column".into());
                };
                if star
                    .replace((usize::from_str(row)?, usize::from_str(col)?))
                    .is_some()
                {
                    return Err("a board has one star".into());
                }
            } else {
                return Err(format!("{line:?} is not a grid line or a star").into());
            }
        }
        let cols = cols.ok_or("a board has grid lines")?;
        if rows > i8::MAX as usize || cols > i8::MAX as usize {
            return Err("a board has at most 127 rows and 127 columns".into());
        }
        let (star_row, star_col) = star.ok_or("a board has a star")?;
        if star_row >= rows || star_col >= cols {
            return Err("the star is outside the grid".into());
        }
        let star_premium = &premiums[star_row * cols + star_col];
        if star_premium.word_multiplier == 0 && star_premium.tile_multiplier == 0 {
            return Err("the star is on a punctured square".into());
        }
        Ok(Self::new_static(StaticBoardLayout {
            premiums: premiums.into_boxed_slice(),
            dim: matrix::Dim {
                rows: rows as i8,
                cols: cols as i8,
            },
            star_row: star_row as i8,
            star_col: star_col as i8,
            transposed_premiums: Box::new([]),
            danger_star_across: Box::new([]),
            danger_star_down: Box::new([]),
            is_symmetric: false,
        }))
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
#[inline(always)]
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
#[inline(always)]
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
#[inline(always)]
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

pub type MakeBoardLayout = fn() -> BoardLayout;

pub const BOARD_LAYOUTS: &[(&str, MakeBoardLayout)] = &[
    ("standard", make_standard_board_layout),
    ("super", make_super_board_layout),
];

#[inline]
pub fn make_board_layout_by_name(name: &str) -> Option<BoardLayout> {
    BOARD_LAYOUTS
        .iter()
        .find(|(this_name, _)| *this_name == name)
        .map(|(_, make)| make())
}

// a bundled board layout by its name, else the text read_file returns for it.
#[inline]
pub fn make_board_layout_from(
    name_or_path: &str,
    read_file: &impl Fn(&str) -> error::Returns<String>,
) -> error::Returns<BoardLayout> {
    match make_board_layout_by_name(name_or_path) {
        Some(board_layout) => Ok(board_layout),
        None => {
            let text = read_file(name_or_path).map_err(|e| {
                format!("{name_or_path:?} is not a bundled board or a readable file: {e}")
            })?;
            Ok(BoardLayout::new_static_from_text(&text)
                .map_err(|e| format!("{name_or_path}: {e}"))?)
        }
    }
}

#[cfg(test)]
#[inline]
pub fn make_test_board_layout(
    premiums: Box<[Premium]>,
    dim: matrix::Dim,
    star_row: i8,
    star_col: i8,
) -> BoardLayout {
    BoardLayout::new_static(StaticBoardLayout {
        premiums,
        dim,
        star_row,
        star_col,
        transposed_premiums: Box::new([]),
        danger_star_across: Box::new([]),
        danger_star_down: Box::new([]),
        is_symmetric: false,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[inline]
    fn text_of(board_layout: &BoardLayout) -> String {
        let dim = board_layout.dim();
        let mut text = format!(
            "# a comment, then a blank line\n\nstar {} {}\n",
            board_layout.star_row(),
            board_layout.star_col()
        );
        for row in 0..dim.rows {
            text.push('|');
            for col in 0..dim.cols {
                let premium = &board_layout.premiums()[dim.at_row_col(row, col)];
                text.push(match (premium.word_multiplier, premium.tile_multiplier) {
                    (4, 1) => '~',
                    (3, 1) => '=',
                    (2, 1) => '-',
                    (1, 4) => '^',
                    (1, 3) => '"',
                    (1, 2) => '\'',
                    (1, 1) => ' ',
                    (0, 0) => '#',
                    _ => '?',
                });
            }
            text.push_str("|\n");
        }
        text
    }

    #[inline]
    fn same_layout(a: &BoardLayout, b: &BoardLayout) -> bool {
        a.dim().rows == b.dim().rows
            && a.dim().cols == b.dim().cols
            && a.star_row() == b.star_row()
            && a.star_col() == b.star_col()
            && a.is_symmetric() == b.is_symmetric()
            && a.premiums().len() == b.premiums().len()
            && a.premiums().iter().zip(b.premiums()).all(|(p, q)| {
                p.word_multiplier == q.word_multiplier && p.tile_multiplier == q.tile_multiplier
            })
    }

    #[test]
    #[inline]
    fn every_board_reads_back_from_its_text() {
        for board_layout in [
            make_standard_board_layout(),
            make_punctured_board_layout(),
            make_super_board_layout(),
        ] {
            let read = BoardLayout::new_static_from_text(&text_of(&board_layout)).unwrap();
            assert!(same_layout(&read, &board_layout));
        }
    }

    #[test]
    #[inline]
    fn every_bundled_board_is_found_by_its_name() {
        for (i, (name, make)) in BOARD_LAYOUTS.iter().enumerate() {
            assert!(
                i == 0 || BOARD_LAYOUTS[i - 1].0 < *name,
                "{name} is out of order"
            );
            assert!(
                same_layout(&make_board_layout_by_name(name).unwrap(), &make()),
                "{name}"
            );
        }
        assert!(make_board_layout_by_name("round").is_none());
    }

    #[test]
    #[inline]
    fn a_board_file_that_cannot_be_played_is_refused() {
        let ok = "star 0 1\n|- |\n| '|\n";
        assert!(BoardLayout::new_static_from_text(ok).is_ok());
        let refused = [
            "",
            "star 0 0\n",
            "star 0 0\n|  |\n| |\n",
            "star 0 0\n||\n",
            "star 0 0\n|x|\n",
            "star 0 0\n|  \n",
            "star 0 0\n  |  |\n",
            "|  |\n",
            "star 0 0\nstar 0 1\n|  |\n",
            "star 1\n|  |\n",
            "star 0 x\n|  |\n",
            "star 0 2\n|  |\n",
            "star 1 0\n|  |\n",
            "star 0 0\n|# |\n",
            "hello\nstar 0 0\n|  |\n",
        ];
        for text in refused {
            assert!(BoardLayout::new_static_from_text(text).is_err(), "{text:?}");
        }
        let widest = format!("star 0 0\n|{}|\n", " ".repeat(127));
        assert!(BoardLayout::new_static_from_text(&widest).is_ok());
        let too_wide = format!("star 0 0\n|{}|\n", " ".repeat(128));
        assert!(BoardLayout::new_static_from_text(&too_wide).is_err());
        let tallest = format!("star 0 0\n{}", "| |\n".repeat(127));
        assert!(BoardLayout::new_static_from_text(&tallest).is_ok());
        let too_tall = format!("star 0 0\n{}", "| |\n".repeat(128));
        assert!(BoardLayout::new_static_from_text(&too_tall).is_err());
    }

    #[test]
    #[inline]
    fn a_board_is_a_name_or_else_a_file() {
        let read_file = |path: &str| -> error::Returns<String> {
            match path {
                "tiny.txt" => Ok("star 0 1\n|- |\n| '|\n".into()),
                "starless.txt" => Ok("|- |\n| '|\n".into()),
                _ => Err(format!("no file {path}").into()),
            }
        };
        assert!(same_layout(
            &make_board_layout_from("super", &read_file).unwrap(),
            &make_super_board_layout()
        ));
        assert_eq!(
            make_board_layout_from("tiny.txt", &read_file)
                .unwrap()
                .dim()
                .cols,
            2
        );
        let unread = make_board_layout_from("round", &read_file)
            .err()
            .unwrap()
            .to_string();
        assert!(
            unread.starts_with("\"round\" is not a bundled board"),
            "{unread}"
        );
        let unparsed = make_board_layout_from("starless.txt", &read_file)
            .err()
            .unwrap()
            .to_string();
        assert!(unparsed.starts_with("starless.txt: "), "{unparsed}");
    }
}
