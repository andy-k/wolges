// Copyright (C) 2020-2021 Andy Kurnia. All rights reserved.

use super::{display, endgame, error, game_config, klv, kwg, movegen};

// this is reusing most of main_json, but main_json is the most current code.

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

// note: only this representation uses -1i8 for blank-as-A (in "board" input
// and "word" response for "action":"play"). everywhere else, use 0x81u8.

pub fn main() -> error::Returns<()> {
    let data = r#"
      {
        "lexicon": "NWL18",
        "ignored": {
        "rack": [ 0, 1, 5, 9, 14, 18, 21 ],
        "board": [
          [  0,  0,  0,  0,  0,  2,  5, 18,  7, 19,  0,  0,  0,  0,  0 ],
          [  0,  0,  0,  0, 16,  1,  0,  0,  0, 21,  0,  0,  0,  0,  0 ],
          [  0,  0, 17,  1,  9,  4,  0,  0,  0, 18,  0,  0,  0,  0,  0 ],
          [  0,  0,  0,  2,  5,  5,  0,  0,  0,  6,  0,  0, 19,  0,  0 ],
          [  0, 16,  0,  5, 20,  0,  0, 22,  9,  1, 20,  9,  3,  0,  0 ],
          [ 13,  1,  0, 20,  1, 23,  0,  0,  0, -3,  0,  0,  8,  0,  0 ],
          [  5, 19,  0,  0,  0,  9, 19,  0,  0,  5,  0,  0,  1,  0,  0 ],
          [  1, 20,  0,  6, 15, 12,  9,  1,  0,  0,  0,  0, 22,  0,  0 ],
          [ 12,  9,  0, 12,  0,  5, 24,  0,  5,  0,  0,  0,  0,  0,  0 ],
          [  0, 14,  0, 15,  0,  4,  0,  0, 14,  0,  0, 25,  0,  0,  0 ],
          [  0,  7, 14, 21,  0,  0,  3,  0, 10,  5, 20,  5,  0,  0,  0 ],
          [  0,  0,  5, 18,  0,  0, 15,  8, 15,  0,  0, 14,  0,  0,  0 ],
          [  0,  0, 15,  0,  0,  0,  7, 15, 25,  0,  0,  0,  0,  0,  0 ],
          [  0,  9, 14,  4, 15, 23,  0, 21,  0,  0,  0,  0,  0,  0,  0 ],
          [  0,  0,  0,  0,  4, 15, 18, 18,  0,  0,  0,  0,  0,  0,  0 ]
        ]
        },
        "rack": [ 1, 21 ],
        "board": [
          [  0,  0,  0,  0,  0,  2,  5, 18,  7, 19,  0,  0,  0,  0,  0 ],
          [  0,  0,  0,  0, 16,  1,  0,  0,  0, 21,  0,  0,  0,  0,  0 ],
          [  0,  0, 17,  1,  9,  4,  0,  0,  0, 18,  0,  0,  0,  0,  0 ],
          [  0,  0,  0,  2,  5,  5,  0,  0,  0,  6,  0, 20, 19, 11,  0 ],
          [  0, 16,  0,  5, 20,  0,  0, 22,  9,  1, 20,  9,  3,  0,  0 ],
          [ 13,  1,  0, 20,  1, 23,  0,  0,  0, -3,  0,  0,  8,  0,  0 ],
          [  5, 19,  0,  0,  0,  9, 19,  0,  0,  5,  0,  0,  1,  0,  0 ],
          [  1, 20,  0,  6, 15, 12,  9,  1,  0, -4, 18,  9, 22,  5, 14 ],
          [ 12,  9,  0, 12,  0,  5, 24,  0,  5,  0,  0,  0,  0,  0,  0 ],
          [  0, 14,  0, 15,  0,  4,  0,  0, 14,  0,  0, 25,  0,  0,  0 ],
          [  0,  7, 14, 21,  0,  0,  3,  0, 10,  5, 20,  5,  0,  0,  0 ],
          [  0,  0,  5, 18,  0,  0, 15,  8, 15,  0,  0, 14,  0,  0,  0 ],
          [  0,  0, 15,  0,  0,  0,  7, 15, 25,  0,  0,  0,  0,  0,  0 ],
          [  0,  9, 14,  4, 15, 23,  0, 21,  0,  0,  0,  0,  0,  0,  0 ],
          [  0,  0,  0,  0,  4, 15, 18, 18,  0,  0,  0,  0,  0,  0,  0 ]
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
        "CSW19" => {
            kwg = kwg::Kwg::from_bytes_alloc(&std::fs::read("csw19.kwg")?);
            klv = klv::Klv::from_bytes_alloc(&std::fs::read("leaves.klv")?);
            game_config = game_config::make_common_english_game_config();
        }
        "NWL18" => {
            kwg = kwg::Kwg::from_bytes_alloc(&std::fs::read("nwl18.kwg")?);
            klv = klv::Klv::from_bytes_alloc(&std::fs::read("leaves.klv")?);
            game_config = game_config::make_common_english_game_config();
        }
        "NWL20" => {
            kwg = kwg::Kwg::from_bytes_alloc(&std::fs::read("nwl20.kwg")?);
            klv = klv::Klv::from_bytes_alloc(&std::fs::read("leaves.klv")?);
            game_config = game_config::make_common_english_game_config();
        }
        "ECWL" => {
            kwg = kwg::Kwg::from_bytes_alloc(&std::fs::read("ecwl.kwg")?);
            klv = klv::Klv::from_bytes_alloc(&std::fs::read("leaves.klv")?);
            game_config = game_config::make_common_english_game_config();
        }
        "OSPS42" => {
            kwg = kwg::Kwg::from_bytes_alloc(&std::fs::read("osps42.kwg")?);
            klv = klv::Klv::from_bytes_alloc(klv::EMPTY_KLV_BYTES);
            game_config = game_config::make_polish_game_config();
        }
        _ => {
            return_error!(format!("invalid lexicon {:?}", question.lexicon));
        }
    };

    let alphabet = game_config.alphabet();
    let alphabet_len_without_blank = alphabet.len() - 1;

    // note: this allocates
    let mut available_tally = (0..alphabet.len())
        .map(|tile| alphabet.freq(tile))
        .collect::<Box<_>>();

    for &tile in &question.rack {
        if tile > alphabet_len_without_blank {
            return_error!(format!(
                "rack has invalid tile {}, alphabet size is {}",
                tile, alphabet_len_without_blank
            ));
        }
        if available_tally[tile as usize] > 0 {
            available_tally[tile as usize] -= 1;
        } else {
            return_error!(format!(
                "too many tile {} (bag contains only {})",
                tile,
                alphabet.freq(tile),
            ));
        }
    }

    let expected_dim = game_config.board_layout().dim();
    if question.board_tiles.len() != expected_dim.rows as usize {
        return_error!(format!(
            "board: need {} rows, found {} rows",
            expected_dim.rows,
            question.board_tiles.len()
        ));
    }
    for (row_num, row) in (0..).zip(question.board_tiles.iter()) {
        if row.len() != expected_dim.cols as usize {
            return_error!(format!(
                "board row {} (0-based): need {} cols, found {} cols",
                row_num,
                expected_dim.cols,
                row.len()
            ));
        }
    }
    let mut board_tiles =
        Vec::with_capacity((expected_dim.rows as usize) * (expected_dim.cols as usize));
    for (row_num, row) in (0..).zip(question.board_tiles.iter()) {
        for (col_num, &signed_tile) in (0..).zip(row) {
            if signed_tile == 0 {
                board_tiles.push(0);
            } else if signed_tile as u8 <= alphabet_len_without_blank {
                let tile = signed_tile as u8;
                board_tiles.push(tile);
                if available_tally[tile as usize] > 0 {
                    available_tally[tile as usize] -= 1;
                } else {
                    return_error!(format!(
                        "too many tile {} (bag contains only {})",
                        tile,
                        alphabet.freq(tile),
                    ));
                }
            } else if (!signed_tile as u8) < alphabet_len_without_blank {
                // turn -1i8, -2i8 into 0x81u8, 0x82u8
                board_tiles.push(0x81 + !signed_tile as u8);
                // verify usage of blank tile
                if available_tally[0] > 0 {
                    available_tally[0] -= 1;
                } else {
                    return_error!(format!(
                        "too many tile {} (bag contains only {})",
                        0,
                        alphabet.freq(0),
                    ));
                }
            } else {
                return_error!(format!(
                    "board row {} col {} (0-based): invalid tile {}, alphabet size is {}",
                    row_num, col_num, signed_tile, alphabet_len_without_blank
                ));
            }
        }
    }

    // this allocates
    let oppo_rack = (0u8..)
        .zip(available_tally.iter())
        .flat_map(|(tile, &count)| std::iter::repeat(tile).take(count as usize))
        .collect::<Box<_>>();
    if oppo_rack.len() > game_config.rack_size() as usize {
        return_error!(format!(
            "not endgame yet as there are {} unseen tiles",
            oppo_rack.len()
        ));
    }

    let mut egs = endgame::EndgameSolver::new(&game_config, &kwg, &klv);
    egs.init(&board_tiles, [&question.rack, &oppo_rack]);
    let out = egs.solve(0);
    let board_snapshot = &movegen::BoardSnapshot {
        board_tiles: &board_tiles,
        game_config: &game_config,
        kwg: &kwg,
        klv: &klv,
    };
    for player_idx in 0..2 {
        println!();
        println!("for player {}:", player_idx);
        println!(
            "p{}: {} {}",
            player_idx,
            out.best[player_idx].value,
            out.best[player_idx].play.fmt(board_snapshot)
        );
    }
    let mut soln = Vec::new();
    for player_idx in 0..2 {
        println!();
        println!("details for player {}:", player_idx);
        let mut latest_board_tiles = board_tiles.clone(); // this allocates and is not reused
        soln.clear();
        egs.append_solution(0, player_idx as u8, &mut soln, [&question.rack, &oppo_rack]);
        for (i, ply) in soln.iter().enumerate() {
            println!(
                "{}: p{}: {} {}",
                i,
                (player_idx + i) % 2,
                ply.value,
                ply.play.fmt(&movegen::BoardSnapshot {
                    board_tiles: &latest_board_tiles,
                    ..*board_snapshot
                })
            );
            match &ply.play {
                movegen::Play::Exchange { .. } => {}
                movegen::Play::Place {
                    down,
                    lane,
                    idx,
                    word,
                    score: _,
                } => {
                    let dim = game_config.board_layout().dim();
                    let strider = if *down {
                        dim.down(*lane)
                    } else {
                        dim.across(*lane)
                    };

                    // place the tiles
                    for (i, &tile) in (*idx..).zip(word.iter()) {
                        if tile != 0 {
                            latest_board_tiles[strider.at(i)] = tile;
                        }
                    }
                }
            }
        }
        display::print_board(&alphabet, &game_config.board_layout(), &latest_board_tiles);
    }

    if true {
        return Ok(());
    }
    // the rest of this is from move_json, but not used here

    let mut move_generator = movegen::KurniaMoveGenerator::new(&game_config);

    let board_snapshot = &movegen::BoardSnapshot {
        board_tiles: &board_tiles,
        game_config: &game_config,
        kwg: &kwg,
        klv: &klv,
    };
    display::print_board(&alphabet, &game_config.board_layout(), &board_tiles);

    move_generator.gen_moves_unfiltered(board_snapshot, &question.rack, question.max_gen);
    let plays = &move_generator.plays;
    println!("found {} moves", plays.len());
    for play in plays.iter() {
        println!("{} {}", play.equity, play.play.fmt(board_snapshot));
    }

    let mut result = Vec::<serde_json::Value>::with_capacity(plays.len());
    for play in plays.iter() {
        match &play.play {
            movegen::Play::Exchange { tiles } => {
                if tiles.is_empty() {
                    result.push(serde_json::json!({
                        "equity": play.equity,
                        "action": "pass" }));
                } else {
                    // tiles: array of numbers. 0 for blank, 1 for A.
                    result.push(serde_json::json!({
                        "equity": play.equity,
                        "action": "exchange",
                        "tiles": tiles[..] }));
                }
            }
            movegen::Play::Place {
                down,
                lane,
                idx,
                word,
                score,
            } => {
                // turn 0x81u8, 0x82u8 into -1i8, -2i8
                let word_played = word
                    .iter()
                    .map(|&x| {
                        if x & 0x80 != 0 {
                            -((x & !0x80) as i8)
                        } else {
                            x as i8
                        }
                    })
                    .collect::<Vec<i8>>();
                // across plays: down=false, lane=row, idx=col (0-based).
                // down plays: down=true, lane=col, idx=row (0-based).
                // word: 0 for play-through, 1 for A, -1 for blank-as-A.
                result.push(serde_json::json!({
                    "equity": play.equity,
                    "action": "play",
                    "down": down,
                    "lane": lane,
                    "idx": idx,
                    "word": word_played,
                    "score": score }));
            }
        }
    }
    let ret = serde_json::to_value(result)?;
    println!("{}", ret);
    println!("{}", serde_json::to_string_pretty(&ret)?);

    Ok(())
}
