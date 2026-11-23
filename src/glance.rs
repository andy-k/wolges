// Copyright (C) 2020-2026 Andy Kurnia.

use super::{game_config, movegen};

// what a board shows at a glance. An anchor is an empty square a tile touches
// (the star, on an empty board). A stretch is a run of a lane's squares a word
// could fill exactly: nothing just past either end, at least one empty square
// and at most rack_size, two squares or more, and an anchor or a tile inside --
// every (dir, idx, left, right) a placement sees, each counted.
#[derive(Clone, Copy)]
pub struct Glance {
    pub anchors: u32,
    pub premium_anchors: u32,
    pub word_premium_anchors: u32,
    pub bingo_lanes: u32,
    pub stretches: u32,
    pub bingo_stretches: u32,
    pub word_premium_stretches: u32,
}

#[inline]
pub fn glance_at(game_config: &game_config::GameConfig, board_tiles: &[u8]) -> Glance {
    let board_layout = game_config.board_layout();
    let dim = board_layout.dim();
    let premiums = board_layout.premiums();
    let rack_size = game_config.rack_size() as u32;
    let empty_board = board_tiles.iter().all(|&t| t == 0);
    let star = dim.at_row_col(board_layout.star_row(), board_layout.star_col());
    let is_anchor = |row: i8, col: i8| {
        let at = dim.at_row_col(row, col);
        if board_tiles[at] != 0 {
            return false;
        }
        if empty_board {
            return at == star;
        }
        [(-1, 0), (1, 0), (0, -1), (0, 1)].iter().any(|&(dr, dc)| {
            let (r, c) = (row + dr, col + dc);
            (0..dim.rows).contains(&r)
                && (0..dim.cols).contains(&c)
                && board_tiles[dim.at_row_col(r, c)] != 0
        })
    };
    let mut glance = Glance {
        anchors: 0,
        premium_anchors: 0,
        word_premium_anchors: 0,
        bingo_lanes: 0,
        stretches: 0,
        bingo_stretches: 0,
        word_premium_stretches: 0,
    };
    for row in 0..dim.rows {
        for col in 0..dim.cols {
            if is_anchor(row, col) {
                let premium = &premiums[dim.at_row_col(row, col)];
                glance.anchors += 1;
                glance.premium_anchors +=
                    (premium.word_multiplier != 1 || premium.tile_multiplier != 1) as u32;
                glance.word_premium_anchors += (premium.word_multiplier != 1) as u32;
            }
        }
    }
    for down in [false, true] {
        for lane in 0..(if down { dim.cols } else { dim.rows }) {
            let strider = dim.lane(down, lane);
            let len = strider.len();
            let square = |i: i8| strider.at(i);
            let anchor_at = |i: i8| {
                if down {
                    is_anchor(i, lane)
                } else {
                    is_anchor(lane, i)
                }
            };
            let mut lane_has_bingo = false;
            for start in 0..len {
                if start > 0 && board_tiles[square(start - 1)] != 0 {
                    continue;
                }
                let (mut empty, mut touching, mut word_premium) = (0u32, false, false);
                for end in start..len {
                    let at = square(end);
                    if board_tiles[at] == 0 {
                        empty += 1;
                        touching |= anchor_at(end);
                        word_premium |= premiums[at].word_multiplier != 1;
                    } else {
                        touching = true;
                    }
                    if empty > rack_size {
                        break;
                    }
                    if end == start
                        || empty == 0
                        || !touching
                        || (end + 1 < len && board_tiles[square(end + 1)] != 0)
                    {
                        continue;
                    }
                    glance.stretches += 1;
                    glance.bingo_stretches += (empty == rack_size) as u32;
                    glance.word_premium_stretches += word_premium as u32;
                    lane_has_bingo |= empty == rack_size;
                }
            }
            glance.bingo_lanes += lane_has_bingo as u32;
        }
    }
    glance
}

// the board a move leaves: the tiles it places put down
#[inline]
pub fn board_after(
    game_config: &game_config::GameConfig,
    board_tiles: &[u8],
    play: &movegen::Play,
    out: &mut Vec<u8>,
) {
    out.clear();
    out.extend_from_slice(board_tiles);
    if let movegen::Play::Place {
        down,
        lane,
        idx,
        word,
        ..
    } = play
    {
        let strider = game_config.board_layout().dim().lane(*down, *lane);
        for (i, &tile) in (*idx..).zip(word.iter()) {
            if tile != 0 {
                out[strider.at(i)] = tile;
            }
        }
    }
}

#[inline]
pub fn reach_into(game_config: &game_config::GameConfig, board_tiles: &[u8], out: &mut Vec<u8>) {
    let board_layout = game_config.board_layout();
    let dim = board_layout.dim();
    let premiums = board_layout.premiums();
    let rack_size = game_config.rack_size();
    let empty_board = board_tiles.iter().all(|&t| t == 0);
    let star = dim.at_row_col(board_layout.star_row(), board_layout.star_col());
    let takes_a_tile = |at: usize| {
        board_tiles[at] == 0
            && (premiums[at].word_multiplier != 0 || premiums[at].tile_multiplier != 0)
    };
    out.clear();
    out.resize(board_tiles.len(), 0);
    for row in 0..dim.rows {
        for col in 0..dim.cols {
            let at = dim.at_row_col(row, col);
            let anchor = takes_a_tile(at)
                && if empty_board {
                    at == star
                } else {
                    [(-1, 0), (1, 0), (0, -1), (0, 1)].iter().any(|&(dr, dc)| {
                        let (r, c) = (row + dr, col + dc);
                        (0..dim.rows).contains(&r)
                            && (0..dim.cols).contains(&c)
                            && board_tiles[dim.at_row_col(r, c)] != 0
                    })
                };
            out[at] = anchor as u8;
        }
    }
    for down in [false, true] {
        for lane in 0..(if down { dim.cols } else { dim.rows }) {
            let strider = dim.lane(down, lane);
            let len = strider.len();
            for backward in [false, true] {
                let mut behind = 0u8;
                for k in 0..len {
                    let at = strider.at(if backward { len - 1 - k } else { k });
                    if !takes_a_tile(at) {
                        behind = 0;
                    } else if out[at] == 1 {
                        behind = 1;
                    } else if behind != 0 && behind < rack_size {
                        behind += 1;
                        if out[at] == 0 || behind < out[at] {
                            out[at] = behind;
                        }
                    } else {
                        behind = 0;
                    }
                }
            }
        }
    }
}

#[inline]
pub fn premium_reaches<'a>(
    game_config: &'a game_config::GameConfig,
    reach: &'a [u8],
) -> impl Iterator<Item = (i8, i8, u8)> + 'a {
    let premiums = game_config.board_layout().premiums();
    premiums.iter().zip(reach).filter_map(|(premium, &r)| {
        let kind = (premium.word_multiplier, premium.tile_multiplier);
        (r != 0 && kind != (1, 1)).then_some((kind.0, kind.1, r))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[inline]
    fn an_empty_board_is_reached_from_the_star_alone() {
        let game_config = game_config::make_english_game_config();
        let board_layout = game_config.board_layout();
        let dim = board_layout.dim();
        let rack_size = game_config.rack_size() as i8;
        let (star_row, star_col) = (board_layout.star_row(), board_layout.star_col());
        let board_tiles = vec![0u8; dim.rows as usize * dim.cols as usize];
        let mut reach = Vec::new();
        reach_into(&game_config, &board_tiles, &mut reach);
        for row in 0..dim.rows {
            for col in 0..dim.cols {
                let away = if row == star_row {
                    (col - star_col).abs()
                } else if col == star_col {
                    (row - star_row).abs()
                } else {
                    rack_size
                };
                let want = if away < rack_size { away as u8 + 1 } else { 0 };
                assert_eq!(reach[dim.at_row_col(row, col)], want, "row {row} col {col}");
            }
        }
    }

    #[test]
    #[inline]
    fn a_square_is_reached_from_the_nearest_anchor_on_its_row_or_column() {
        let game_config = game_config::make_english_game_config();
        let dim = game_config.board_layout().dim();
        let mut board_tiles = vec![0u8; dim.rows as usize * dim.cols as usize];
        board_tiles[dim.at_row_col(7, 7)] = 1;
        let mut reach = Vec::new();
        reach_into(&game_config, &board_tiles, &mut reach);
        let at = |row: i8, col: i8| reach[dim.at_row_col(row, col)];
        assert_eq!(at(7, 7), 0, "the tile's own square");
        for (row, col) in [(6, 7), (8, 7), (7, 6), (7, 8)] {
            assert_eq!(at(row, col), 1, "the anchor at row {row} col {col}");
        }
        assert_eq!(at(6, 6), 2);
        assert_eq!(at(5, 7), 2);
        assert_eq!(at(0, 7), 7);
        assert_eq!(at(7, 0), 7);
        assert_eq!(at(6, 1), 7);
        assert_eq!(at(6, 0), 0);
        assert_eq!(at(5, 5), 0);
        assert_eq!(at(0, 0), 0);
        board_tiles[dim.at_row_col(7, 7)] = 0;
        board_tiles[dim.at_row_col(0, 0)] = 1;
        reach_into(&game_config, &board_tiles, &mut reach);
        let at = |row: i8, col: i8| reach[dim.at_row_col(row, col)];
        assert_eq!((at(0, 1), at(1, 0), at(1, 1), at(0, 7)), (1, 1, 2, 7));
        assert_eq!((at(7, 7), at(0, 8), at(14, 14)), (0, 0, 0));
    }

    #[test]
    #[inline]
    fn an_empty_board_reaches_the_premium_squares_of_the_stars_lanes() {
        let game_config = game_config::make_english_game_config();
        let dim = game_config.board_layout().dim();
        let board_tiles = vec![0u8; dim.rows as usize * dim.cols as usize];
        let mut reach = Vec::new();
        reach_into(&game_config, &board_tiles, &mut reach);
        let mut found = premium_reaches(&game_config, &reach).collect::<Vec<_>>();
        found.sort_unstable();
        assert_eq!(
            found,
            [(1, 2, 5), (1, 2, 5), (1, 2, 5), (1, 2, 5), (2, 1, 1)]
        );
    }

    #[test]
    #[inline]
    fn a_square_no_tile_may_take_stops_a_walk() {
        let game_config = game_config::make_punctured_english_game_config();
        let dim = game_config.board_layout().dim();
        let board_tiles = vec![0u8; dim.rows as usize * dim.cols as usize];
        let mut reach = Vec::new();
        reach_into(&game_config, &board_tiles, &mut reach);
        let at = |(row, col): (i8, i8)| reach[dim.at_row_col(row, col)];
        assert_eq!(at((7, 7)), 1);
        for (near, punctured, past) in [
            ((7, 6), (7, 5), (7, 4)),
            ((7, 8), (7, 9), (7, 10)),
            ((6, 7), (5, 7), (4, 7)),
            ((8, 7), (9, 7), (10, 7)),
        ] {
            assert_eq!(
                (at(near), at(punctured), at(past)),
                (2, 0, 0),
                "from {near:?} on"
            );
        }
    }
}
