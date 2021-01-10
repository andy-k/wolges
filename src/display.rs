// Copyright (C) 2020-2021 Andy Kurnia. All rights reserved.

use super::{alphabet, board_layout, game_state, game_timers};

#[inline(always)]
pub fn empty_label(board_layout: &board_layout::BoardLayout, row: i8, col: i8) -> &'static str {
    if row == board_layout.star_row() && col == board_layout.star_col() {
        return "*";
    }
    let premium = board_layout.premiums()[board_layout.dim().at_row_col(row, col)];
    match premium.word_multiplier {
        4 => "~",
        3 => "=",
        2 => "-",
        1 => match premium.tile_multiplier {
            4 => "^",
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

pub fn print_board(
    alphabet: &alphabet::Alphabet<'_>,
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

fn print_ms(mut ms: i64) {
    if ms < 0 {
        print!("-");
        ms = -ms;
    }
    let just_ms = ms % 1000;
    let sec = ms / 1000;
    let just_sec = sec % 60;
    let min = sec / 60;
    print!("{:02}:{:02}.{:03}", min, just_sec, just_ms);
}

pub fn print_game_state(
    game_state: &game_state::GameState,
    optional_game_timers: Option<&game_timers::GameTimers>,
) {
    print_board(
        &game_state.game_config.alphabet(),
        &game_state.game_config.board_layout(),
        &game_state.board_tiles,
    );
    println!(
        "Pool {}: {}",
        game_state.bag.0.len(),
        game_state
            .game_config
            .alphabet()
            .fmt_rack(&game_state.bag.0)
    );
    for (i, player) in game_state.players.iter().enumerate() {
        print!(
            "Player {}: {} {}",
            i + 1,
            player.score,
            game_state.game_config.alphabet().fmt_rack(&player.rack)
        );
        if let Some(game_timers) = optional_game_timers {
            let clock_ms = game_timers.clocks_ms[i];
            print!(" ");
            print_ms(clock_ms);
            let adjustment = game_state.game_config.time_adjustment(clock_ms);
            if adjustment != 0 {
                print!(" ({})", adjustment);
            }
            if game_timers.turn as usize == i {
                // may differ from game_state.turn if timer is paused
                print!(" (timer running)");
            }
        }
        if game_state.turn as usize == i {
            print!(" (turn)");
        }
        println!();
    }
}
