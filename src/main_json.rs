// Copyright (C) 2020-2023 Andy Kurnia.

use rand::prelude::*;
use wolges::{
    display, error, game_config, game_state, kibitzer, klv, kwg, move_filter, move_picker, movegen,
};

// tile numbering follows alphabet order (not necessarily unicode order).
// rack: array of numbers. 0 for blank, 1 for A.
// board: 2D array of numbers. 0 for empty, 1 for A, -1 for blank-as-A.
// lexicon: this implies board size and other rules too.
// count: maximum number of moves returned.
// (note: equal moves are not stably sorted;
//  different counts may tie-break the last move differently.)
#[derive(serde::Deserialize)]
struct Question {
    lexicon: String,
    rack: Vec<u8>,
    #[serde(rename = "board")]
    board_tiles: Vec<Vec<i8>>,
    #[serde(rename = "count")]
    max_gen: usize,
}

fn main() -> error::Returns<()> {
    let data = r#"
      {
        "lexicon": "CSW19",
        "xrack": [ 1, 3, 10, 16, 17, 18, 19 ],
        "xboard": [
          [  0,  0,  0,  0,  0,  0,  0, 18,  0,  0,  0,  8, 15, 12,  4 ],
          [  0,  0,  0,  0,  9,  4,  5,  1, 20,  9, 22, -5,  0,  0,  0 ],
          [  0,  0,  0,  0,  0,  0,  0,  9,  0,  0,  0,  0,  0,  0,  0 ],
          [  0,  0,  0,  0,  0, 23,  0, 14,  0,  0,  0,  0,  0,  0,  0 ],
          [  0,  0,  0,  0,  0,  9,  0,  4,  0,  0,  0,  0,  0,  0,  0 ],
          [  0,  0,  0,  0,  0, 26,  0,  1,  0,  0,  0,  0,  0,  0,  0 ],
          [  0,  0,  0,  0,  0,  5,  0, 20,  0,  0,  0,  0,  0,  0,  0 ],
          [  0,  4, 15, 13,  9, 14,  5,  5,  0,  0,  0,  0,  0,  0,  0 ],
          [  7,  1,  2, 25,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0 ],
          [  0, 23, 15,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0 ],
          [  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0 ],
          [  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0 ],
          [  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0 ],
          [  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0 ],
          [  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0 ]
        ],
        "rack": [ 3, 4, 5, 12, 13, 15, 15 ],
        "board": [
          [  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0 ],
          [  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0 ],
          [  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0 ],
          [  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0 ],
          [  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0 ],
          [  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0 ],
          [  0,  0,  0,  0,  0,  0,  0,  0, 26,  1,  7,  0,  0,  0,  0 ],
          [  0,  0,  0,  0,  0,  1, 11,  5,  5,  0,  0,  0,  0,  0,  0 ],
          [  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0 ],
          [  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0 ],
          [  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0 ],
          [  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0 ],
          [  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0 ],
          [  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0 ],
          [  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0 ]
        ],
        "count": 15
      }
    "#;
    let question = serde_json::from_str::<Question>(data)?;

    let kwg;
    let klv;
    let game_config;

    // of course this should be cached
    match question.lexicon.as_str() {
        "CSW21" => {
            kwg = kwg::Kwg::from_bytes_alloc(&std::fs::read("lexbin/CSW21.kwg")?);
            klv = klv::Klv::from_bytes_alloc(&std::fs::read("lexbin/CSW21.klv")?);
            game_config = game_config::make_common_english_game_config();
        }
        "CSW19" => {
            kwg = kwg::Kwg::from_bytes_alloc(&std::fs::read("lexbin/CSW19.kwg")?);
            klv = klv::Klv::from_bytes_alloc(&std::fs::read("lexbin/english.klv")?);
            game_config = game_config::make_common_english_game_config();
        }
        "NWL18" => {
            kwg = kwg::Kwg::from_bytes_alloc(&std::fs::read("lexbin/NWL18.kwg")?);
            klv = klv::Klv::from_bytes_alloc(&std::fs::read("lexbin/english.klv")?);
            game_config = game_config::make_common_english_game_config();
        }
        "NWL20" => {
            kwg = kwg::Kwg::from_bytes_alloc(&std::fs::read("lexbin/NWL20.kwg")?);
            klv = klv::Klv::from_bytes_alloc(&std::fs::read("lexbin/english.klv")?);
            game_config = game_config::make_common_english_game_config();
        }
        "ECWL" => {
            kwg = kwg::Kwg::from_bytes_alloc(&std::fs::read("lexbin/ECWL.kwg")?);
            klv = klv::Klv::from_bytes_alloc(&std::fs::read("lexbin/english.klv")?);
            game_config = game_config::make_common_english_game_config();
        }
        "OSPS42" => {
            kwg = kwg::Kwg::from_bytes_alloc(&std::fs::read("lexbin/OSPS42.kwg")?);
            klv = klv::Klv::from_bytes_alloc(klv::EMPTY_KLV_BYTES);
            game_config = game_config::make_polish_game_config();
        }
        "OSPS44" => {
            kwg = kwg::Kwg::from_bytes_alloc(&std::fs::read("lexbin/OSPS44.kwg")?);
            klv = klv::Klv::from_bytes_alloc(klv::EMPTY_KLV_BYTES);
            game_config = game_config::make_polish_game_config();
        }
        _ => {
            wolges::return_error!(format!("invalid lexicon {:?}", question.lexicon));
        }
    };

    let mut rng = rand_chacha::ChaCha20Rng::from_entropy();
    let mut game_state = game_state::GameState::new(&game_config);
    // temp hardcode
    game_state.players[0].score = 16;
    game_state.players[1].score = 44;

    let mut kibitzer = kibitzer::Kibitzer::new();
    kibitzer.prepare(&game_config, &question.rack, &question.board_tiles)?;

    display::print_board(
        game_config.alphabet(),
        game_config.board_layout(),
        &kibitzer.board_tiles,
    );

    let mut move_filter = move_filter::GenMoves::Unfiltered;
    let mut move_picker =
        move_picker::MovePicker::Simmer(move_picker::Simmer::new(&game_config, &kwg, &klv));
    game_state
        .board_tiles
        .copy_from_slice(&kibitzer.board_tiles);

    // put the bag and shuffle it
    game_state.bag.0.clear();
    game_state
        .bag
        .0
        .reserve(kibitzer.available_tally.iter().map(|&x| x as usize).sum());
    game_state.bag.0.extend(
        (0u8..)
            .zip(kibitzer.available_tally.iter())
            .flat_map(|(tile, &count)| std::iter::repeat(tile).take(count as usize)),
    );
    game_state.bag.shuffle(&mut rng);

    let mut move_generator = movegen::KurniaMoveGenerator::new(&game_config);
    let board_snapshot = &movegen::BoardSnapshot {
        board_tiles: &kibitzer.board_tiles,
        game_config: &game_config,
        kwg: &kwg,
        klv: &klv,
    };

    if true {
        for &tile in &question.rack {
            game_state.players[0].rack.push(tile);
        }
        move_picker.pick_a_move(
            &mut move_filter,
            &mut move_generator,
            board_snapshot,
            &game_state,
            &game_state.current_player().rack,
        );
        let plays = &move_generator.plays;
        println!("found {} moves", plays.len());
        for play in plays.iter() {
            println!("{} {}", play.equity, play.play.fmt(board_snapshot));
        }
    }

    move_generator.gen_moves_unfiltered(&movegen::GenMovesParams {
        board_snapshot,
        rack: &question.rack,
        max_gen: question.max_gen,
        always_include_pass: false,
    });
    let plays = &move_generator.plays;

    println!("found {} moves", plays.len());
    for play in plays.iter() {
        println!("{} {}", play.equity, play.play.fmt(board_snapshot));
    }

    let result = plays
        .iter()
        .map(|x| x.into())
        .collect::<Vec<kibitzer::JsonPlayWithEquity>>();
    let ret = serde_json::to_value(result)?;
    println!("{ret}");
    println!("{}", serde_json::to_string_pretty(&ret)?);

    Ok(())
}
