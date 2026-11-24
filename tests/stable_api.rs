// Copyright (C) 2020-2026 Andy Kurnia.

#![cfg(test)]

// the stable items src/lib.rs lists, each named, so that one renamed or
// removed stops this from compiling.
use wolges::{
    alphabet, bag, board_layout, display, equity, error, game_config, game_state, klv, kwg,
    move_filter, move_picker, movegen, play_scorer, win_pct,
};

#[test]
fn the_stable_items_stay_public() {
    let _: &str = wolges::VERSION;
    let _: fn(&[u8]) -> error::Returns<kwg::Kwg<kwg::Node22>> =
        kwg::Kwg::<kwg::Node22>::from_bytes_alloc;
    let _: fn(&[u8]) -> error::Returns<kwg::Kwg<kwg::Node24>> =
        kwg::Kwg::<kwg::Node24>::from_bytes_alloc;
    let _: fn(Vec<u8>, bool) -> error::Returns<kwg::Kwg<kwg::Node24>> = kwg::kbwg_from_bytes_alloc;
    let _: fn(&[u8]) -> error::Returns<klv::Klv<kwg::Node24>> =
        klv::Klv::<kwg::Node24>::from_bytes_alloc;
    let _: fn(Vec<u8>) -> error::Returns<klv::Klv<kwg::Node24>> = klv::kbwg_from_bytes_alloc;
    let _: fn() -> alphabet::Alphabet = alphabet::make_english_alphabet;
    let _: fn() -> board_layout::BoardLayout = board_layout::make_standard_board_layout;
    let game_config: game_config::GameConfig = game_config::make_english_game_config();
    game_config.supports(game_config::Op::Generation).unwrap();
    let game_state = game_state::GameState::new(&game_config);
    let _: &bag::Bag = &game_state.bag;
    let _ = display::BoardFenner::new(
        game_config.alphabet(),
        game_config.board_layout(),
        &game_state.board_tiles,
    )
    .to_string();
    let _ = movegen::KurniaMoveGenerator::new(&game_config);
    let _: Option<movegen::GenMovesParams<'_, kwg::Node24, kwg::Node24>> = None;
    let _: Option<movegen::ValuedMove> = None;
    let _ = play_scorer::PlayScorer::new();
    let _: Option<move_filter::GenMoves<'_>> = None;
    let _: Option<move_picker::MovePicker<'_, kwg::Node24, kwg::Node24>> = None;
    let _: equity::Equity = equity::Equity::new(0);
    let _: fn(std::io::Empty) -> error::Returns<win_pct::WinPctTable> =
        win_pct::WinPctTable::from_csv::<std::io::Empty>;
}
