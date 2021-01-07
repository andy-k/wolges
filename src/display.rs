// Copyright (C) 2020-2021 Andy Kurnia. All rights reserved.

use super::{alphabet, board_layout};

#[inline(always)]
pub fn empty_label(board_layout: &board_layout::BoardLayout, row: i8, col: i8) -> &'static str {
    if row == board_layout.star_row() && col == board_layout.star_col() {
        return "*";
    }
    let premium = board_layout.premiums()[board_layout.dim().at_row_col(row, col)];
    match premium.word_multiplier {
        3 => "=",
        2 => "-",
        1 => match premium.tile_multiplier {
            3 => "\"",
            2 => "\'",
            1 => " ",
            _ => panic!(),
        },
        _ => panic!(),
    }
}

#[inline(always)]
pub fn board_label<'a>(
    alphabet: &'a alphabet::Alphabet<'a>,
    board_layout: &board_layout::BoardLayout,
    board_tiles: &'a [u8],
    row: i8,
    col: i8,
) -> &'a str {
    alphabet
        .from_board(board_tiles[board_layout.dim().at_row_col(row, col)])
        .unwrap_or_else(|| empty_label(board_layout, row, col))
}

pub fn print_board<'a>(
    alphabet: &'a alphabet::Alphabet<'a>,
    board_layout: &board_layout::BoardLayout,
    board_tiles: &[u8],
) {
    print!("  ");
    for c in 0..board_layout.dim().cols {
        print!(" {}", ((c as u8) + 0x61) as char);
    }
    println!();
    print!("  +");
    for _ in 1..board_layout.dim().cols {
        print!("--");
    }
    println!("-+");
    for r in 0..board_layout.dim().rows {
        print!("{:2}|", r + 1);
        for c in 0..board_layout.dim().cols {
            if c > 0 {
                print!(" ")
            }
            print!("{}", board_label(alphabet, board_layout, board_tiles, r, c));
        }
        println!("|{}", r + 1);
    }
    print!("  +");
    for _ in 1..board_layout.dim().cols {
        print!("--");
    }
    println!("-+");
    print!("  ");
    for c in 0..board_layout.dim().cols {
        print!(" {}", ((c as u8) + 0x61) as char);
    }
    println!();
}
