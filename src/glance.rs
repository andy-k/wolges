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
