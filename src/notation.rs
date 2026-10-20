// Copyright (C) 2020-2026 Andy Kurnia.

// reading a place move as it is written: a coordinate (row then column for an
// across play, 8D; column then row for a down play, D8) and its word

use super::{alphabet, display, error, matrix};
use std::str::FromStr;

pub struct Coord {
    pub down: bool,
    pub lane: i8,
    pub idx: i8,
}

// /^(?:\d+[A-Z]+|[A-Z]+\d+)$/i
#[inline(always)]
pub fn parse_coord(coord: &str, dim: &matrix::Dim) -> Option<Coord> {
    let b1 = coord.as_bytes();
    let l1 = b1.iter().take_while(|c| c.is_ascii_digit()).count();
    let b2 = &b1[l1..];
    let l2 = b2.iter().take_while(|c| c.is_ascii_alphabetic()).count();
    if l2 == 0 {
        return None;
    }
    let alp2 = i8::try_from(display::str_to_column_usize_ignore_case(&b2[..l2])?).ok()?;
    if alp2 >= dim.cols {
        return None;
    }
    let b3 = &b2[l2..];
    let l3 = b3.iter().take_while(|c| c.is_ascii_digit()).count();
    if l3 != b3.len() {
        return None;
    }
    if l1 != 0 && l3 == 0 {
        let dig1 = i8::from_str(&coord[..l1]).ok()?.wrapping_sub(1);
        if (0..dim.rows).contains(&dig1) {
            return Some(Coord {
                down: false,
                lane: dig1,
                idx: alp2,
            });
        }
    } else if l1 == 0 && l3 != 0 {
        let dig3 = i8::from_str(&coord[l1 + l2..]).ok()?.wrapping_sub(1);
        if (0..dim.rows).contains(&dig3) {
            return Some(Coord {
                down: true,
                lane: alp2,
                idx: dig3,
            });
        }
    }
    None
}

// read the tiles of a place move's word at coord into v: 0 for a tile played
// through, the tile itself for one placed. A tile already on the board is
// written as '.' (preferred) or as itself, which must match it exactly, and a
// run of them may be enclosed in parentheses; a placed tile must go on an empty
// square.
#[inline]
pub fn read_place_word(
    plays_alphabet_reader: &alphabet::AlphabetReader,
    board_tiles: &[u8],
    dim: &matrix::Dim,
    coord: &Coord,
    s: &str,
    v: &mut Vec<u8>,
) -> Result<(), error::MyError> {
    let (row, col) = if coord.down {
        (coord.idx, coord.lane)
    } else {
        (coord.lane, coord.idx)
    };
    let mut num_in_paren = -1isize;
    v.clear();
    if !s.is_empty() {
        v.reserve(s.len());
        let sb = s.as_bytes();
        let mut ix = 0;
        while ix < sb.len() {
            if let Some((tile, end_ix)) = plays_alphabet_reader.next_tile(sb, ix) {
                let row = row + ((v.len() as i8) & -(coord.down as i8));
                let col = col + ((v.len() as i8) & -(!coord.down as i8));
                if row < 0 || col < 0 || row >= dim.rows || col >= dim.cols {
                    return Err(error::new(format!(
                        "invalid coord (row {row} col {col}) after {v:?} in {s:?}"
                    )));
                }
                let tile_on_board = board_tiles[dim.at_row_col(row, col)];
                if tile_on_board == 0 {
                    // empty square, place this tile. must not be in paren.
                    if num_in_paren >= 0 {
                        return Err(error::new(format!(
                            "invalid tile {tile} after {v:?} in {s:?} (no tile found on board at row {row} col {col})"
                        )));
                    }
                    v.push(tile);
                } else if tile_on_board != tile {
                    return Err(error::new(format!(
                        "invalid tile {tile} after {v:?} in {s:?} (tile {tile_on_board} found on board at row {row} col {col})"
                    )));
                } else {
                    // tile matches
                    v.push(0);
                    num_in_paren += (num_in_paren >= 0) as isize;
                }
                ix = end_ix;
            } else if sb[ix] == b'.' {
                let row = row + ((v.len() as i8) & -(coord.down as i8));
                let col = col + ((v.len() as i8) & -(!coord.down as i8));
                if row < 0 || col < 0 || row >= dim.rows || col >= dim.cols {
                    return Err(error::new(format!(
                        "invalid coord (row {row} col {col}) after {v:?} in {s:?}"
                    )));
                }
                let tile_on_board = board_tiles[dim.at_row_col(row, col)];
                if tile_on_board == 0 {
                    return Err(error::new(format!(
                        "invalid tile 0 after {v:?} in {s:?} (no tile found on board at row {row} col {col})"
                    )));
                }
                v.push(0);
                ix += 1;
                num_in_paren += (num_in_paren >= 0) as isize;
            } else if num_in_paren < 0 && sb[ix] == b'(' {
                ix += 1;
                num_in_paren = 0;
            } else if num_in_paren > 0 && sb[ix] == b')' {
                ix += 1;
                num_in_paren = -1;
            } else {
                return Err(error::new(format!("invalid tile after {v:?} in {s:?}")));
            }
        }
        if num_in_paren >= 0 {
            return Err(error::new("unclosed parenthesis in tiles".into()));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game_config;

    #[test]
    #[inline]
    fn a_coordinate_reads_across_row_first_and_down_column_first() {
        let game_config = game_config::make_english_game_config();
        let dim = game_config.board_layout().dim();
        let across = parse_coord("8D", dim).unwrap();
        assert!(!across.down && across.lane == 7 && across.idx == 3);
        let down = parse_coord("d8", dim).unwrap();
        assert!(down.down && down.lane == 3 && down.idx == 7);
        for bad in ["", "8", "D", "16A", "A16", "8P", "D8D", "8D8", "-8D"] {
            assert!(parse_coord(bad, dim).is_none(), "{bad:?}");
        }
    }

    #[test]
    #[inline]
    fn a_word_reads_its_played_through_tiles_however_written() {
        let game_config = game_config::make_english_game_config();
        let dim = game_config.board_layout().dim();
        let reader = alphabet::AlphabetReader::new_for_plays(game_config.alphabet());
        // H on 8H, so 8G reads A, then the H already there, then T
        let mut board_tiles = vec![0u8; (dim.rows as usize) * (dim.cols as usize)];
        board_tiles[dim.at_row_col(7, 7)] = 8;
        let coord = parse_coord("8G", dim).unwrap();
        let mut v = Vec::new();
        for word in ["A.T", "AHT", "A(H)T", "A(.)T"] {
            read_place_word(&reader, &board_tiles, dim, &coord, word, &mut v).unwrap();
            assert_eq!(v, [1, 0, 20], "{word}");
        }
        // a blank is lowercase
        read_place_word(&reader, &board_tiles, dim, &coord, "a.t", &mut v).unwrap();
        assert_eq!(v, [0x81, 0, 0x80 | 20]);
        for bad in ["AXT", "(A)HT", "A(H", "A.T.", "A..T", "A?T"] {
            assert!(
                read_place_word(&reader, &board_tiles, dim, &coord, bad, &mut v).is_err(),
                "{bad}"
            );
        }
    }
}
