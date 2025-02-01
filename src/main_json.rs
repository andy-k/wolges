// Copyright (C) 2020-2025 Andy Kurnia.

use rand::prelude::*;
use wolges::{
    display, error, game_config, game_state, kibitzer, klv, kwg, move_filter, move_picker, movegen,
    play_scorer,
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
        "actual_lexicon": "CSW19",
        "lexicon": "CSW24",
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
    let _ = data;
    let data = r#"
      {
          "source": "https://woogles.io/game/CHWqqBC7?turn=11",
          "rack": [1, 2, 3, 7, 7, 17, 22],
          "board": [
            [0, 0, 0, 0, 0, 0, 0, 8,15,22, 5, 0, 0, 0, 0],
            [0, 0, 0, 0, 0, 0, 0, 0, 0, 0,18, 0, 0, 0, 0],
            [0, 0, 0, 0, 0, 0, 0, 0,26, 5, 1,12, 0, 0, 0],
            [0, 0, 0, 3, 1,12, 9,16, 5,18,19, 0, 0, 0, 0],
            [0, 0, 0, 0,20, 9,14, 0, 0, 0, 9, 0, 0, 0, 0],
            [0, 0, 0, 0, 0, 0, 0, 0, 0,16,15, 0, 0, 0, 0],
            [0, 0, 0, 0, 0, 0,15, 6, 0,21,14, 0, 0, 0, 0],
            [0, 0, 0, 0, 0, 0,23, 5,12,20, 0, 0, 0, 0, 0],
            [0, 0, 0, 0, 0, 0, 0, 0, 9, 0, 0, 0, 0, 0, 0],
            [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
            [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
            [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
            [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
            [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
            [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]

          ],
          "actual_lexicon": "CSW21",
          "lexicon": "CSW24",
          "actual_leave": "CSW21",
          "leave": "CSW24",
          "rules": "CrosswordGame",
          "count": 150000
      }
    "#;
    let _ = data;
    let data = r#"
      {
          "source": "https://woogles.io/game/4jUemdhr?turn=106",
          "rack": [2, 3, 5, 12, 14, 16, 19],
          "board": [
 [0, 1, 18, 0, 15, 14, 25, 0, 4, 9, 0, 8, 15, 0, 16, 8, 15, 0, 1, 20, 0],
 [0, 0, 5, 0, 0, 1, 0, 0, 0, 20, 0, 15, 0, 0, 0, 0, 22, 0, 12, 0, 0],
 [0, 0, 9, 0, 0, 15, 21, 20, 4, 1, 20, 5, 4, 0, 0, 7, 5, 14, 20, 0, 0],
 [0, 0, 14, 1, 26, 9, 0, 0, 0, 0, 21, 0, 0, 14, 0, 0, 18, 0, 0, 0, 0],
 [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 9, 14, 6, 9, 18, 13, 19, 0, 0, 0, 0],
 [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 14, 0, 0, 24, 9, 0, 0, 0, 0, 0, 0],
 [0, 0, 0, 0, 0, 0, 0, 0, 0, 10, 1, 23, 0, 0, -26, 0, 0, 0, 0, 0, 0],
 [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 5, 24, 0, 0, 0, 0, 0, 0, 0, 0],
 [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 4, 9, 0, 0, 0, 0, 0, 0, 0, 0],
 [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 7, 0, 0, 0, 0, 0, 0, 0, 0, 0],
 [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 23, 5, 0, 0, 0, 0, 0, 0, 0, 0, 0],
 [0, 0, 0, 0, 0, 0, 0, 0, 0, 11, 5, 19, 8, 0, 0, 0, 0, 0, 0, 0, 0],
 [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 14, 0, 9, 0, 0, 0, 0, 0, 0, 0, 0],
 [0, 0, 0, 0, 0, 0, 0, 0, 0, 19, 20, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
 [0, 0, 0, 0, 0, 0, 0, 0, 0, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
 [0, 0, 0, 0, 0, 0, 0, 0, 0, 22, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
 [0, 0, 0, 0, 15, 2, 15, 5, 0, 5, 0, 0, 0, 0, 0, 0, 25, 0, 0, 0, 0],
 [0, 0, 1, 13, 9, 0, 0, 13, 5, 18, 12, 19, 0, 0, 7, 0, 5, 18, 18, 0, 0],
 [0, 0, 7, 0, 12, 15, 0, 0, 18, 0, 0, 15, 18, 7, 5, 1, 20, 0, 1, 0, 0],
 [0, 0, 1, 0, 0, 15, 0, 0, 5, 0, 0, -16, 0, 0, -5, 0, 0, 0, 9, 0, 0],
 [0, 1, 18, 0, 15, 14, 25, 0, 4, 9, 0, 8, 15, 0, 16, 8, 15, 0, 1, 20, 0]

          ],
          "actual_lexicon": "super-CSW21",
          "lexicon": "super-CSW24",
          "count": 15
      }
    "#;
    /*
      .AR.ONY.DI.HO.PHO.AT.
      ..E..A...T.O....V.L..
      ..I..OUTDATED..GENT..
      ..NAZI....U..N..R....
      ..........INFIRMS....
      ..........N..XI......
      .........JAW..z......
      ...........EX........
      ...........DI........
      ...........G.........
      ..........WE.........
      .........KESH........
      ..........N.I........
      .........ST..........
      .........A...........
      .........V...........
      ....OBOE.E......Y....
      ..AMI..MERLS..G.ERR..
      ..G.LO..R..ORGEAT.A..
      ..A..O..E..p..e...I..
      .AR.ONY.DI.HO.PHO.AT.
    */
    let question = serde_json::from_str::<Question>(data)?;

    let kwg;
    let klv;
    let game_config;

    // of course this should be cached
    match question.lexicon.as_str() {
        "CSW24" => {
            kwg = kwg::Kwg::from_bytes_alloc(&std::fs::read("lexbin/CSW24.kwg")?);
            klv = klv::Klv::from_bytes_alloc(&std::fs::read("lexbin/CSW24.klv2")?);
            game_config = game_config::make_english_game_config();
        }
        "super-CSW24" => {
            kwg = kwg::Kwg::from_bytes_alloc(&std::fs::read("lexbin/CSW24.kwg")?);
            klv = klv::Klv::from_bytes_alloc(&std::fs::read("lexbin/super-CSW24.klv2")?);
            game_config = game_config::make_super_english_game_config();
        }
        "NWL23" => {
            kwg = kwg::Kwg::from_bytes_alloc(&std::fs::read("lexbin/NWL23.kwg")?);
            klv = klv::Klv::from_bytes_alloc(&std::fs::read("lexbin/NWL23.klv2")?);
            game_config = game_config::make_english_game_config();
        }
        "super-NWL23" => {
            kwg = kwg::Kwg::from_bytes_alloc(&std::fs::read("lexbin/NWL23.kwg")?);
            klv = klv::Klv::from_bytes_alloc(&std::fs::read("lexbin/super-NWL23.klv2")?);
            game_config = game_config::make_english_game_config();
        }
        "ECWL" => {
            kwg = kwg::Kwg::from_bytes_alloc(&std::fs::read("lexbin/ECWL.kwg")?);
            klv = klv::Klv::from_bytes_alloc(&std::fs::read("lexbin/ECWL.klv2")?);
            game_config = game_config::make_english_game_config();
        }
        "super-ECWL" => {
            kwg = kwg::Kwg::from_bytes_alloc(&std::fs::read("lexbin/ECWL.kwg")?);
            klv = klv::Klv::from_bytes_alloc(&std::fs::read("lexbin/super-ECWL.klv2")?);
            game_config = game_config::make_english_game_config();
        }
        _ => {
            wolges::return_error!(format!("invalid lexicon {:?}", question.lexicon));
        }
    };

    let mut rng = rand_chacha::ChaCha20Rng::from_os_rng();
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
            .flat_map(|(tile, &count)| std::iter::repeat_n(tile, count as usize)),
    );
    game_state.bag.shuffle(&mut rng);

    let mut move_generator = movegen::KurniaMoveGenerator::new(&game_config);
    let board_snapshot = &movegen::BoardSnapshot {
        board_tiles: &kibitzer.board_tiles,
        game_config: &game_config,
        kwg: &kwg,
        klv: &klv,
    };

    if false {
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
        num_exchanges_by_this_player: game_state.current_player().num_exchanges,
        always_include_pass: false,
    });
    let plays = &move_generator.plays;

    println!("found {} moves", plays.len());
    let mut ps = play_scorer::PlayScorer::new();
    for play in plays.iter() {
        println!("{} {}", play.equity, play.play.fmt(board_snapshot));

        let movegen_score = match &play.play {
            movegen::Play::Exchange { .. } => 0,
            movegen::Play::Place { score, .. } => *score,
        };
        let recounted_score = ps.compute_score(board_snapshot, &play.play);
        if movegen_score != recounted_score {
            println!(
                "{} should score {} instead of {}!",
                play.play.fmt(board_snapshot),
                recounted_score,
                movegen_score
            );
        }
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
