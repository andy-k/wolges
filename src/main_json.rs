// Copyright (C) 2020-2021 Andy Kurnia. All rights reserved.

use super::{display, error, game_config, klv, kwg, movegen};

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
        "lexicon": "CSW19",
        "rack": [ 1, 3, 10, 16, 17, 18, 19 ],
        "board": [
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
