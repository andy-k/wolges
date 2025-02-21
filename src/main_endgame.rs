// Copyright (C) 2020-2025 Andy Kurnia.

use rand::prelude::*;
use wolges::{
    alphabet, bites, build, display, endgame, error, fash, game_config, game_state, klv, kwg,
    matrix, movegen, play_scorer,
};

// this is reusing most of main_json, but main_json is the most current code.

// tile numbering follows alphabet order (not necessarily unicode order).
// rack: array of numbers. 0 for blank, 1 for A.
// board: 2D array of numbers. 0 for empty, 1 for A, -1 for blank-as-A.
// lexicon: this implies board size and other rules too.
#[derive(serde::Deserialize)]
struct Question {
    lexicon: String,
    rack: Vec<u8>,
    #[serde(rename = "board")]
    board_tiles: Vec<Vec<i8>>,
}

// note: only this representation uses -1i8 for blank-as-A (in "board" input
// and "word" response for "action":"play"). everywhere else, use 0x81u8.

struct Coord {
    down: bool,
    lane: i8,
    idx: i8,
}

// /^(?:\d+[A-Z]+|[A-Z]+\d+)$/i
fn parse_coord_token(coord: &str, dim: &matrix::Dim) -> Option<Coord> {
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

// /^[+-](?:0|[1-9]\d*)$/
fn is_score_token(coord: &str) -> bool {
    let b = coord.as_bytes();
    if !(!b.is_empty() && (b[0] == b'+' || b[0] == b'-')) {
        return false;
    }
    let b = &b[1..];
    if b.is_empty() {
        return false;
    }
    if b[0] == b'0' {
        return b.len() == 1;
    }
    let l1 = b
        .iter()
        .position(|c| !c.is_ascii_digit())
        .unwrap_or(b.len());
    l1 == b.len()
}

// /^-?\d+$/
fn is_cum_token(coord: &str) -> bool {
    let b = coord.as_bytes();
    let b = &b[(!b.is_empty() && b[0] == b'-') as usize..];
    if b.is_empty() {
        return false;
    }
    let l1 = b
        .iter()
        .position(|c| !c.is_ascii_digit())
        .unwrap_or(b.len());
    l1 == b.len()
}

use std::str::FromStr;

impl Question {
    // not-very-strict gcg parser
    fn from_gcg(
        game_config: &game_config::GameConfig,
        lexicon: &str,
        gcg: &str,
        rack: &str,
    ) -> Result<Question, error::MyError> {
        let empty_kwg = kwg::Kwg::from_bytes_alloc(kwg::EMPTY_KWG_BYTES);
        let empty_klv = klv::Klv::from_bytes_alloc(klv::EMPTY_KLV_BYTES);
        let mut ps = play_scorer::PlayScorer::new();
        let alphabet = game_config.alphabet();
        let plays_alphabet_reader = alphabet::AlphabetReader::new_for_plays(alphabet);
        let racks_alphabet_reader = alphabet::AlphabetReader::new_for_racks(alphabet);
        let dim = game_config.board_layout().dim();
        let mut rng = rand_chacha::ChaCha20Rng::from_os_rng();
        let mut game_state = game_state::GameState::new(game_config);
        game_state.reset_and_draw_tiles(game_config, &mut rng);
        let mut game_state_undo = game_state.clone();
        let mut can_withdraw = false;
        let mut v = Vec::new(); // temp buffer
        let parse_rack = |v: &mut Vec<_>, rack: &str| -> Result<(), _> {
            racks_alphabet_reader.set_word(rack, v)
        };
        for (line_number, line) in (1usize..).zip(gcg.lines()) {
            if !line.starts_with('>') {
                continue;
            }
            let mut tokens = line.split_whitespace();
            macro_rules! fmt_error {
                ($msg: expr_2021) => {
                    error::new(format!("{} on line {} {:?}", $msg, line_number, line))
                };
            }
            macro_rules! next_token {
                ($what: tt) => {
                    tokens
                        .next()
                        .ok_or_else(|| fmt_error!(concat!("missing ", $what, " token")))
                };
            }
            let player_token = next_token!("player")?;
            let mut rack_token = next_token!("rack")?;
            let mut coord_token = if rack_token.starts_with('(') && rack_token.ends_with(')') {
                std::mem::take(&mut rack_token)
            } else {
                next_token!("coord")?
            };
            let word_token = if parse_coord_token(coord_token, dim).is_none() {
                std::mem::take(&mut coord_token)
            } else {
                next_token!("word")?
            };
            let score_token = next_token!("score")?;
            if !is_score_token(score_token) {
                return Err(fmt_error!("invalid score token"));
            }
            let cum_token = next_token!("cum")?;
            if !is_cum_token(cum_token) {
                return Err(fmt_error!("invalid cum token"));
            }
            if tokens.next().is_some() {
                return Err(fmt_error!("too many tokens"));
            }
            let _ = player_token;
            let _ = cum_token;
            let mut move_score = i32::from_str(score_token)
                .map_err(|e| fmt_error!(format_args!("invalid score token: {e}")))?;
            parse_rack(&mut v, rack_token)
                .map_err(|e| fmt_error!(format_args!("invalid rack token: {e}")))?;
            game_state.set_current_rack(&v);
            let mut move_to_play = None;
            if coord_token.is_empty() {
                if word_token == "-" && move_score == 0 {
                    // pass
                    move_to_play = Some(movegen::Play::Exchange {
                        tiles: [][..].into(),
                    })
                } else if word_token == "(challenge)" && move_score >= 0 {
                    // bonus, unsuccessful challenge
                    // do nothing, even if there is additional score
                } else if word_token == "--" && move_score <= 0 {
                    // withdraw, successful challenge
                    if !can_withdraw {
                        return Err(fmt_error!("cannot withdraw"));
                    }
                    game_state.clone_from(&game_state_undo);
                    // make a pass
                    move_to_play = Some(movegen::Play::Exchange {
                        tiles: [][..].into(),
                    });
                    move_score = 0;
                } else if word_token.starts_with('-') && word_token.len() >= 2 && move_score == 0 {
                    // exchange tiles
                    let exchanged = &word_token[1..];
                    move_to_play = Some(movegen::Play::Exchange {
                        tiles: match parse_rack(&mut v, exchanged) {
                            Ok(()) => v[..].into(),
                            Err(e) => {
                                // could be number of tiles
                                if !match usize::from_str(exchanged) {
                                    Ok(num) => {
                                        num >= 1 && num <= game_state.current_player().rack.len()
                                    }
                                    Err(_) => false,
                                } {
                                    return Err(fmt_error!(format_args!(
                                        "invalid exchanged tiles {exchanged:?}: {e}"
                                    )));
                                }
                                [][..].into()
                            }
                        },
                    })
                } else if word_token == "(time)" && move_score <= 0 {
                    // time penalty
                    // do nothing
                } else if word_token.starts_with('(')
                    && word_token.ends_with(')')
                    && word_token.len() >= 3
                {
                    // rack adjustments
                    // do nothing
                } else {
                    return Err(fmt_error!("invalid line"));
                }
                can_withdraw = false;
            } else {
                // this is a place move
                let coord = parse_coord_token(coord_token, dim)
                    .ok_or_else(|| fmt_error!("invalid coord token"))?;
                game_state_undo.clone_from(&game_state);
                can_withdraw = true;

                let (row, col) = if coord.down {
                    (coord.idx, coord.lane)
                } else {
                    (coord.lane, coord.idx)
                };

                // for skipped tiles, this parser supports:
                // - '.' (preferred)
                // - "X" (tile must exactly match the tile already on board)
                // - optional '('...')' denoting one or more tiles already on board, each can be '.' or "X"
                let s = word_token;
                let mut num_in_paren = -1isize;
                v.clear();
                if !s.is_empty() {
                    v.reserve(s.len());
                    let sb = s.as_bytes();
                    let mut ix = 0;
                    while ix < sb.len() {
                        match plays_alphabet_reader.next_tile(sb, ix) { Some((tile, end_ix)) => {
                            let row = row + ((v.len() as i8) & -(coord.down as i8));
                            let col = col + ((v.len() as i8) & -(!coord.down as i8));
                            if row < 0 || col < 0 || row >= dim.rows || col >= dim.cols {
                                return Err(fmt_error!(format_args!(
                                    "invalid coord (row {row} col {col}) after {v:?} in {s:?}"
                                )));
                            }
                            let tile_on_board = game_state.board_tiles[dim.at_row_col(row, col)];
                            if tile_on_board == 0 {
                                // empty square, place this tile. must not be in paren.
                                if num_in_paren >= 0 {
                                    return Err(fmt_error!(format_args!(
                                        "invalid tile {tile} after {v:?} in {s:?} (no tile found on board at row {row} col {col})"
                                    )));
                                }
                                v.push(tile);
                            } else if tile_on_board != tile {
                                return Err(fmt_error!(format_args!(
                                    "invalid tile {tile} after {v:?} in {s:?} (tile {tile_on_board} found on board at row {row} col {col})"
                                )));
                            } else {
                                // tile matches
                                v.push(0);
                                num_in_paren += (num_in_paren >= 0) as isize;
                            }
                            ix = end_ix;
                        } _ => if sb[ix] == b'.' {
                            let row = row + ((v.len() as i8) & -(coord.down as i8));
                            let col = col + ((v.len() as i8) & -(!coord.down as i8));
                            if row < 0 || col < 0 || row >= dim.rows || col >= dim.cols {
                                return Err(fmt_error!(format_args!(
                                    "invalid coord (row {row} col {col}) after {v:?} in {s:?}"
                                )));
                            }
                            let tile_on_board = game_state.board_tiles[dim.at_row_col(row, col)];
                            if tile_on_board == 0 {
                                return Err(fmt_error!(format_args!(
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
                            return Err(fmt_error!(format_args!(
                                "invalid tile after {v:?} in {s:?}"
                            )));
                        }}
                    }
                    if num_in_paren >= 0 {
                        return Err(fmt_error!("unclosed parenthesis in tiles"));
                    }
                }
                move_to_play = Some(movegen::Play::Place {
                    down: coord.down,
                    lane: coord.lane,
                    idx: coord.idx,
                    word: v[..].into(),
                    score: move_score,
                });
            }
            if let Some(play) = move_to_play {
                let board_snapshot = &movegen::BoardSnapshot {
                    board_tiles: &game_state.board_tiles,
                    game_config,
                    kwg: &empty_kwg,
                    klv: &empty_klv,
                };
                match ps.validate_play(board_snapshot, &game_state, &play) {
                    Err(err) => {
                        return Err(fmt_error!(format_args!(
                            "invalid play {}: {}",
                            play.fmt(board_snapshot),
                            err
                        )));
                    }
                    Ok(_adjusted_play) => {
                        let recounted_score = ps.compute_score(board_snapshot, &play);
                        if move_score != recounted_score {
                            return Err(fmt_error!(format_args!(
                                "wrong score for {}: should score {}",
                                play.fmt(board_snapshot),
                                recounted_score,
                            )));
                        } else {
                            // ok
                        }
                    }
                }
                game_state
                    .play(game_config, &mut rng, &play)
                    .map_err(|e| fmt_error!(format_args!("invalid play: {e}")))?;
                game_state.next_turn();
            }
        }
        let board_tiles = game_state
            .board_tiles
            .chunks_exact(dim.rows as usize)
            .map(|row| {
                row.iter()
                    .map(|&x| {
                        // turn 0x81u8, 0x82u8 into -1i8, -2i8
                        if x & 0x80 == 0 {
                            x as i8
                        } else {
                            -0x80i8 - (x as i8)
                        }
                    })
                    .collect::<Vec<_>>()
            })
            .collect::<Vec<_>>();
        parse_rack(&mut v, rack).map_err(|e| error::new(format!("invalid rack {rack:?}: {e}")))?;
        Ok(Question {
            lexicon: lexicon.to_string(),
            rack: v,
            board_tiles,
        })
    }

    fn from_fen(
        game_config: &game_config::GameConfig,
        lexicon: &str,
        fen_str: &str,
        rack: &str,
    ) -> Result<Question, error::MyError> {
        let alphabet = game_config.alphabet();
        let racks_alphabet_reader = alphabet::AlphabetReader::new_for_racks(alphabet);
        let board_layout = game_config.board_layout();
        let dim = board_layout.dim();
        let mut v = Vec::new(); // temp buffer
        let parse_rack = |v: &mut Vec<_>, rack: &str| -> Result<(), _> {
            racks_alphabet_reader.set_word(rack, v)
        };
        let mut fen_parser = display::BoardFenParser::new(alphabet, board_layout);
        let parsed_fen = fen_parser.parse(fen_str)?;
        let board_tiles = parsed_fen
            .chunks_exact(dim.rows as usize)
            .map(|row| {
                row.iter()
                    .map(|&x| {
                        // turn 0x81u8, 0x82u8 into -1i8, -2i8
                        if x & 0x80 == 0 {
                            x as i8
                        } else {
                            -0x80i8 - (x as i8)
                        }
                    })
                    .collect::<Vec<_>>()
            })
            .collect::<Vec<_>>();
        parse_rack(&mut v, rack).map_err(|e| error::new(format!("invalid rack {rack:?}: {e}")))?;
        Ok(Question {
            lexicon: lexicon.to_string(),
            rack: v,
            board_tiles,
        })
    }
}

fn main() -> error::Returns<()> {
    let data = [
        r#"
      {
        "actual_lexicon": "NWL18",
        "lexicon": "NWL23",
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
      }
    "#,
        r#"
      {
        "actual_lexicon": "NWL18",
        "lexicon": "NWL23",
        "source": "https://woogles.io/game/SBRtWRzo?turn=22",
        "rack": [ 5, 9, 10, 12, 13, 19, 20 ],
        "board": [
          [  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0, 23 ],
          [  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0, 17,  9 ],
          [  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  1,  0, 18 ],
          [  0,  0,  0,  0,  0,  0,  0,  0, 22,  9, 18,  5,-12,  1, 25 ],
          [  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0, 14,  0,  0 ],
          [  0, 11,  0,  0,  0,  0,  0,  0,  0,  6,  0, 20,  9, 26,  0 ],
          [  0,  1,  0,  0,  0,  0,  0,  0,  0, 12,  0,  0,  3,  0,  0 ],
          [  3, 18,  9,  2,  0, 19,  8,  1, 22,  5,  0,  2, 15, 14,  4 ],
          [  0, 18,  0,  0,  7, 21, 13,  0,  0,  5, 23,  5, 19,  0,  9 ],
          [ 16,  9,  0,  0,  0, 12,  0,  0,  0, 20,  1, 14,  0,  0, 19 ],
          [ 12,  0,  0,  0,  0,  6,  0,  7,  0,  0,  5, 20,  0,  0,  5 ],
          [ 21,  0, 21,  8,  0,  9,  0, 21,  0,  0,  0, 15,  0, 16,  1 ],
          [ 14,  0, 14, 15,  0, 20,  0, 25,  0,  0,  0,  0,  0, 15,-19 ],
          [  7,  1,  4,  1, 18,  5, 14,  5,  0,  0,  0,  0,  0, 15,  5 ],
          [  5,  0, 15, 24,  0,  0,  0,  4,  0,  0,  0,  0,  0,  0,  4 ]
        ]
      }
    "#,
        r#"
      {
        "actual_lexicon": "NWL18",
        "lexicon": "NWL23",
        "source": "https://woogles.io/game/SBRtWRzo?turn=22, LIM(A)S, G(AE)",
        "rack": [ 5, 10, 20 ],
        "board": [
          [  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0, 23 ],
          [  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0, 17,  9 ],
          [  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  1,  0, 18 ],
          [  0,  0,  0,  0,  0,  0,  0,  0, 22,  9, 18,  5,-12,  1, 25 ],
          [  0,  0,  0,  0,  0,  0,  0, 12,  0,  0,  0,  0, 14,  0,  0 ],
          [  0, 11,  0,  0,  0,  0,  0,  9,  0,  6,  0, 20,  9, 26,  0 ],
          [  0,  1,  0,  0,  0,  0,  0, 13,  0, 12,  0,  0,  3,  0,  0 ],
          [  3, 18,  9,  2,  0, 19,  8,  1, 22,  5,  0,  2, 15, 14,  4 ],
          [  0, 18,  0,  0,  7, 21, 13, 19,  0,  5, 23,  5, 19,  0,  9 ],
          [ 16,  9,  0,  0,  1, 12,  0,  0,  0, 20,  1, 14,  0,  0, 19 ],
          [ 12,  0,  0,  0,  5,  6,  0,  7,  0,  0,  5, 20,  0,  0,  5 ],
          [ 21,  0, 21,  8,  0,  9,  0, 21,  0,  0,  0, 15,  0, 16,  1 ],
          [ 14,  0, 14, 15,  0, 20,  0, 25,  0,  0,  0,  0,  0, 15,-19 ],
          [  7,  1,  4,  1, 18,  5, 14,  5,  0,  0,  0,  0,  0, 15,  5 ],
          [  5,  0, 15, 24,  0,  0,  0,  4,  0,  0,  0,  0,  0,  0,  4 ]
        ]
      }
    "#,
        r#"
      {
        "actual_lexicon": "NWL18",
        "lexicon": "NWL23",
        "source": "https://woogles.io/game/SBRtWRzo?turn=22, MIS",
        "rack": [ 1, 5, 9, 15, 15, 18, 20 ],
        "board": [
          [  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0, 23 ],
          [  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0, 17,  9 ],
          [  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  1,  0, 18 ],
          [  0,  0,  0,  0,  0,  0,  0,  0, 22,  9, 18,  5,-12,  1, 25 ],
          [  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0, 14,  0,  0 ],
          [  0, 11,  0,  0,  0,  0,  0,  0,  0,  6,  0, 20,  9, 26,  0 ],
          [  0,  1,  0,  0,  0,  0,  0,  0,  0, 12,  0,  0,  3,  0,  0 ],
          [  3, 18,  9,  2,  0, 19,  8,  1, 22,  5,  0,  2, 15, 14,  4 ],
          [  0, 18,  0,  0,  7, 21, 13,  0,  0,  5, 23,  5, 19,  0,  9 ],
          [ 16,  9,  0,  0,  0, 12,  0,  0,  0, 20,  1, 14,  0,  0, 19 ],
          [ 12,  0,  0,  0,  0,  6,  0,  7,  0,  0,  5, 20,  0,  0,  5 ],
          [ 21,  0, 21,  8,  0,  9,  0, 21,  0,  0,  0, 15,  0, 16,  1 ],
          [ 14,  0, 14, 15,  0, 20,  0, 25,  0, 13,  9, 19,  0, 15,-19 ],
          [  7,  1,  4,  1, 18,  5, 14,  5,  0,  0,  0,  0,  0, 15,  5 ],
          [  5,  0, 15, 24,  0,  0,  0,  4,  0,  0,  0,  0,  0,  0,  4 ]
        ]
      }
    "#,
        r#"
      {
        "actual_lexicon": "NWL20",
        "lexicon": "NWL23",
        "source": "https://woogles.io/game/iUsasmWy?turn=24",
        "rack": [ 1, 5, 5, 15, 19, 23, 0 ],
        "board": [
          [ 22,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0 ],
          [  9,  0,  0,  0,  0,  0,  0,  0,  0,  0,  7,  0,  0,  0,  0 ],
          [ 14,  0,  0, 20,  0,  0,  0,  0,  0,  0, 18,  0,  0,  0,  0 ],
          [ 25,  0,  0,  5,  0,  0,  0, 19, 17, 21,  9,  4,  0,  0,  0 ],
          [ 12, 15,  1, 20,  8,  0,  0,  0,  1,  0, 16,  9,  0,  0,  0 ],
          [  0, 23,  9, 18,  5,  0, 20,  5, 20,  0,  0,-22,  0,  0,  0 ],
          [ 15, 12, 12,  1,  0,  2,  1,  7,  0,  0,  0, 15,  0,  0,  0 ],
          [  0,  0,  0,  3, 18,  5,  4, 15,  0,  0,  2, 18,  1,  9, 14 ],
          [  0,  0, 18,  9,  5, 12, 19,  0,  0,  0,  0,  3,  0,  0,  0 ],
          [  0, 26,  5,  4,  0,  0,  0, 13,  0,  0,  0,  5,  0,  0,  0 ],
          [  0,  0,  0,  0,  0,  0,  0, 15,  0,  0, 25,  5,  0,  0,  0 ],
          [  0,  0,  0,  0,  0,  0,  0, 22,  9, 14, 15, 19,  0,  0,  0 ],
          [  0, 24, 21,  0,  0,  0,  0,  1,  0,  0,  0,  0,  0,  0,  0 ],
          [  0, 21, 14, 10,  1, 13,  0, 14,  0, 11,  5,  6,  0,  0,  0 ],
          [  0,  0,  0,  0,  0, 15, 21, 20,  6,  9, 18,  5,  4,  0,  0 ]
        ]
      }
    "#,
        r#"
      {
        "actual_lexicon": "NWL20",
        "lexicon": "NWL23",
        "source": "https://woogles.io/game/iUsasmWy?turn=25",
        "rack": [ 1, 7, 8, 9, 14, 16, 20 ],
        "board": [
          [ 22,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0 ],
          [  9,  0,  0,  0,  0,  0,  0,  0,  0,  0,  7,  0,  0,  0,  0 ],
          [ 14,  0,  0, 20,  0,  0,  0,  0,  0,  0, 18,  0,  0,  0,  0 ],
          [ 25,  0,  0,  5,  0,  0,  0, 19, 17, 21,  9,  4,  0,  0,  0 ],
          [ 12, 15,  1, 20,  8,  0,  0,  0,  1,  0, 16,  9,  0,  0,  0 ],
          [  0, 23,  9, 18,  5,  0, 20,  5, 20,  0,  0,-22,  0,  0,  0 ],
          [ 15, 12, 12,  1,  0,  2,  1,  7,  0,  0,  0, 15,  0,  0,  0 ],
          [  0,  0,  0,  3, 18,  5,  4, 15,  0,  0,  2, 18,  1,  9, 14 ],
          [  0,  0, 18,  9,  5, 12, 19,  0,  0,  0,  0,  3,  0,  0,  0 ],
          [  0, 26,  5,  4,  0,  0,  0, 13,  0,  0,  0,  5,  0,  0,  0 ],
          [ 23,  1,  5, 19,  0,  0,  0, 15,  0,  0, 25,  5,  0,  0,  0 ],
          [  0,  0,  0,  0,  0,  0,  0, 22,  9, 14, 15, 19,  0,  0,  0 ],
          [  0, 24, 21,  0,  0,  0,  0,  1,  0,  0,  0,  0,  0,  0,  0 ],
          [  0, 21, 14, 10,  1, 13,  0, 14,  0, 11,  5,  6,  0,  0,  0 ],
          [  0,  0,  0,  0,  0, 15, 21, 20,  6,  9, 18,  5,  4,  0,  0 ]
        ]
      }
    "#,
        r#"
      {
        "actual_lexicon": "NWL20",
        "lexicon": "NWL23",
        "source": "https://woogles.io/game/iUsasmWy?turn=26",
        "rack": [ 5, 15, 0 ],
        "board": [
          [ 22,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0 ],
          [  9,  0,  0,  0,  0,  0,  0,  0,  0,  0,  7,  0,  0,  0,  0 ],
          [ 14,  0,  0, 20,  0,  0,  0,  0,  0,  0, 18,  0,  0,  0,  0 ],
          [ 25,  0,  0,  5,  0,  0,  0, 19, 17, 21,  9,  4,  0,  0,  0 ],
          [ 12, 15,  1, 20,  8,  0,  0,  0,  1,  0, 16,  9,  0,  0,  0 ],
          [  0, 23,  9, 18,  5,  0, 20,  5, 20,  0,  0,-22,  0,  0,  0 ],
          [ 15, 12, 12,  1,  0,  2,  1,  7,  0,  0,  0, 15,  0, 16,  0 ],
          [  0,  0,  0,  3, 18,  5,  4, 15,  0,  0,  2, 18,  1,  9, 14 ],
          [  0,  0, 18,  9,  5, 12, 19,  0,  0,  0,  0,  3,  0, 20,  0 ],
          [  0, 26,  5,  4,  0,  0,  0, 13,  0,  0,  0,  5,  0,  8,  0 ],
          [ 23,  1,  5, 19,  0,  0,  0, 15,  0,  0, 25,  5,  0,  9,  0 ],
          [  0,  0,  0,  0,  0,  0,  0, 22,  9, 14, 15, 19,  0, 14,  0 ],
          [  0, 24, 21,  0,  0,  0,  0,  1,  0,  0,  0,  0,  0,  7,  0 ],
          [  0, 21, 14, 10,  1, 13,  0, 14,  0, 11,  5,  6,  0,  0,  0 ],
          [  0,  0,  0,  0,  0, 15, 21, 20,  6,  9, 18,  5,  4,  0,  0 ]
        ]
      }
    "#,
        r#"
      {
        "actual_lexicon": "CSW19",
        "lexicon": "CSW24",
        "source": "https://woogles.io/game/mQLyde5N?turn=29",
        "rack": [ 4, 9, 12, 12, 19, 20 ],
        "board": [
          [  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0 ],
          [  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0, 16,  0,  0 ],
          [  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0, 23,  5,  0,  0 ],
          [  0,  0,  0,  0,  0,  0,  0,  0,  0, 20, 15,  5,  4,  0,  0 ],
          [  0,  0,  0,  0,  0,  0,  0,  0,  3, 15, 19, 13,  9,  3,  0 ],
          [  0,  0,  0,  0,  0,  0,  0, 15,  8, 13,  0,  0,  0, 18,  0 ],
          [  0,  0,  0,  0,  0,  0,  0,  0,  0,  5, 18,  0, 17,  9, 14 ],
          [  0,  0,  0,  0,  0,  0, 22, 15, 24,  0, 21, 18,  9, 14,  5 ],
          [  0,  0,  0,  0,  0,  0,  0, 15, 21, 16,  1,  0,  0,  7,  0 ],
          [  0,  0,  0,  0,  0, 11,  1, 20,  0,  5, 14,  0, 26,  5,  4 ],
          [  7,  0,  0,  0,  6,  1,  5,  0,  2,  8,  1, 10,  9,  0,  1 ],
          [ 21, 18,  0,  7,  1,  5,  0,  0, 18,  0,  0,  5, 14,  0, 23 ],
          [ 22,  1, 21,  0,  0,  0,  0,  0, 18,  0,  0,  1,  5,  0, 20 ],
          [  0,  9, -7, 14, 15,  2, 12,  5,  0,  9, 15, 14,  0,  0,  0 ],
          [  0,  4,  0, 25, 15,  0,  0, 19,  1, 20,  9, 19,  6, 25,  0 ]
        ]
      }
    "#,
        r#"
      {
        "actual_lexicon": "CSW19",
        "lexicon": "CSW24",
        "source": "https://woogles.io/game/mQLyde5N?turn=30",
        "rack": [ 0, 5, 9, 12, 20 ],
        "board": [
          [  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0, 12,  0 ],
          [  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0, 16,  9,  0 ],
          [  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0, 23,  5,  4,  0 ],
          [  0,  0,  0,  0,  0,  0,  0,  0,  0, 20, 15,  5,  4,  0,  0 ],
          [  0,  0,  0,  0,  0,  0,  0,  0,  3, 15, 19, 13,  9,  3,  0 ],
          [  0,  0,  0,  0,  0,  0,  0, 15,  8, 13,  0,  0,  0, 18,  0 ],
          [  0,  0,  0,  0,  0,  0,  0,  0,  0,  5, 18,  0, 17,  9, 14 ],
          [  0,  0,  0,  0,  0,  0, 22, 15, 24,  0, 21, 18,  9, 14,  5 ],
          [  0,  0,  0,  0,  0,  0,  0, 15, 21, 16,  1,  0,  0,  7,  0 ],
          [  0,  0,  0,  0,  0, 11,  1, 20,  0,  5, 14,  0, 26,  5,  4 ],
          [  7,  0,  0,  0,  6,  1,  5,  0,  2,  8,  1, 10,  9,  0,  1 ],
          [ 21, 18,  0,  7,  1,  5,  0,  0, 18,  0,  0,  5, 14,  0, 23 ],
          [ 22,  1, 21,  0,  0,  0,  0,  0, 18,  0,  0,  1,  5,  0, 20 ],
          [  0,  9, -7, 14, 15,  2, 12,  5,  0,  9, 15, 14,  0,  0,  0 ],
          [  0,  4,  0, 25, 15,  0,  0, 19,  1, 20,  9, 19,  6, 25,  0 ]
        ]
      }
    "#,
        r#"
      {
        "actual_lexicon": "CSW19",
        "lexicon": "CSW24",
        "source": "https://woogles.io/game/mQLyde5N?turn=28",
        "rack": [ 0, 5, 5, 9, 12, 16, 20 ],
        "board": [
          [  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0 ],
          [  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0 ],
          [  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0, 23,  0,  0,  0 ],
          [  0,  0,  0,  0,  0,  0,  0,  0,  0, 20, 15,  5,  4,  0,  0 ],
          [  0,  0,  0,  0,  0,  0,  0,  0,  3, 15, 19, 13,  9,  3,  0 ],
          [  0,  0,  0,  0,  0,  0,  0, 15,  8, 13,  0,  0,  0, 18,  0 ],
          [  0,  0,  0,  0,  0,  0,  0,  0,  0,  5, 18,  0, 17,  9, 14 ],
          [  0,  0,  0,  0,  0,  0, 22, 15, 24,  0, 21, 18,  9, 14,  5 ],
          [  0,  0,  0,  0,  0,  0,  0, 15, 21, 16,  1,  0,  0,  7,  0 ],
          [  0,  0,  0,  0,  0, 11,  1, 20,  0,  5, 14,  0, 26,  5,  4 ],
          [  7,  0,  0,  0,  6,  1,  5,  0,  2,  8,  1, 10,  9,  0,  1 ],
          [ 21, 18,  0,  7,  1,  5,  0,  0, 18,  0,  0,  5, 14,  0, 23 ],
          [ 22,  1, 21,  0,  0,  0,  0,  0, 18,  0,  0,  1,  5,  0, 20 ],
          [  0,  9, -7, 14, 15,  2, 12,  5,  0,  9, 15, 14,  0,  0,  0 ],
          [  0,  4,  0, 25, 15,  0,  0, 19,  1, 20,  9, 19,  6, 25,  0 ]
        ]
      }
    "#,
    ][9];
    let question = serde_json::from_str::<Question>(data)?;
    let _ = question;
    // https://github.com/domino14/macondo/issues/154
    let _question = Question::from_gcg(
        &game_config::make_polish_game_config(),
        "OSPS49", // actually "OSPS44",
        r"#character-encoding UTF-8
#player1 1 ptf1559
#player2 2 smut3k
>1: AHIJOUY 8F HUJA +20 20
>1: AHIJOUY --  -20 0
>2: ĆĘIKPST 8G STĘPIĆ +46 46
>1: AHIJOUY 7I HOI +24 24
>2: CFKNWYZ J5 CZ..Y +10 56
>1: AJMSUWY -  +0 24
>2: FKLNŃWW -  +0 56
>1: AJMSUWY -J +0 24
>2: FKLNŃWW -KWŃW +0 56
>1: AAMSUWY 9F AU +14 38
>2: AFLLŁNO 5J .ŁA +12 68
>1: AAMSWYZ 10E SAMY +18 56
>2: CEFLLNO 11C CLE +14 82
>1: AEŁNRWZ 12D AR +13 69
>2: FLNOOOW L3 FL.N +18 100
>1: DEŁNTWZ 13E WENT +10 79
>2: EJNOOOW H13 .EJ +18 118
>1: ?ADIŁZZ 15A ZDZIAŁa. +89 168
>2: IKNOOOW I13 ON +12 130
>1: ABEIMNS 12I SAMBIE +21 189
>2: IIKOOTW M11 K.I +8 138
>1: ?EJNŚWW A14 E. +2 191
>2: AILOOTW C11 .LI +14 152
>1: ?JNNŚWW 12C (.A.)W +7 198
>2: AOOOTWŹ -Ź +0 152
>1: ?AJNNŚW 3L .iŚ +20 218
>2: AGOOOTW K10 GA.O +14 166
>1: AIJNNRW 4L .I +4 222
>2: OORSTWY N12 .WY +10 176
>1: AJNNRRW 11J J. +8 230
>2: ADOORST 12C ....O +8 184
>1: ANNPRRW 9F ..R +10 240
>2: ADORSTY J14 DY +16 200
>1: ACNNPRW 13A CN. +8 248
>2: AOOPRST 10B PO +13 213
>1: ADNPRWZ H8 ...P +7 255
>2: AEKORST 2M KET +25 238
>2: AEKORST --  -25 213
>1: AĄDNRWZ 15J .ARD +7 262
>2: AEKORST 2N ET +18 231
>1: ĄBEGNWZ O1 E. +9 271
>2: AKKORSŹ C10 ....S. +8 239
>1: ĄBGNWZZ 13M ..Ą +7 278
>2: AIKKORŹ M9 RA... +7 246
>1: BGNŃWZZ 10K .N.Ń +26 304
>2: IKKMOŹŻ 9M .OK +12 258
#>1: BGHUWZZ N7 ZG.. +12 316
#>2: IKMÓŹŻ M1 ŻM.. +13 271
#>1: BHUWZ -  +0 316
#>2: IKÓŹ B8 KÓ. +9 280
#>1: BHUWZ -  +0 316
#>2: IŹ 8B .IŹ +21 301
#>2:  (BHUWZ) +22 323
    ",
        "BGHUWZZ",
    )?;
    let _question = Question::from_gcg(
        &game_config::make_polish_game_config(),
        "OSPS49", // actually "OSPS44",
        r"#character-encoding UTF-8
#player1 1 ptf1559
#player2 2 smut3k
>1: AHIJOUY 8F HUJA +20 20
>1: AHIJOUY --  -20 0
>2: ĆĘIKPST 8G STĘPIĆ +46 46
>1: AHIJOUY 7I HOI +24 24
>2: CFKNWYZ J5 CZ..Y +10 56
>1: AJMSUWY -  +0 24
>2: FKLNŃWW -  +0 56
>1: AJMSUWY -J +0 24
>2: FKLNŃWW -KWŃW +0 56
>1: AAMSUWY 9F AU +14 38
>2: AFLLŁNO 5J .ŁA +12 68
>1: AAMSWYZ 10E SAMY +18 56
>2: CEFLLNO 11C CLE +14 82
>1: AEŁNRWZ 12D AR +13 69
>2: FLNOOOW L3 FL.N +18 100
>1: DEŁNTWZ 13E WENT +10 79
>2: EJNOOOW H13 .EJ +18 118
>1: ?ADIŁZZ 15A ZDZIAŁa. +89 168
>2: IKNOOOW I13 ON +12 130
>1: ABEIMNS 12I SAMBIE +21 189
>2: IIKOOTW M11 K.I +8 138
>1: ?EJNŚWW A14 E. +2 191
>2: AILOOTW C11 .LI +14 152
>1: ?JNNŚWW 12C ...W +7 198
>2: AOOOTWŹ -Ź +0 152
>1: ?AJNNŚW 3L .iŚ +20 218
>2: AGOOOTW K10 GA.O +14 166
>1: AIJNNRW 4L .I +4 222
>2: OORSTWY N12 .WY +10 176
>1: AJNNRRW 11J J. +8 230
>2: ADOORST 12C ....O +8 184
>1: ANNPRRW 9F ..R +10 240
>2: ADORSTY J14 DY +16 200
>1: ACNNPRW 13A CN. +8 248
>2: AOOPRST 10B PO +13 213
>1: ADNPRWZ H8 ...P +7 255
>2: AEKORST 2M KET +25 238
>2: AEKORST --  -25 213
>1: AĄDNRWZ 15J .ARD +7 262
>2: AEKORST 2N ET +18 231
>1: ĄBEGNWZ O1 E. +9 271
>2: AKKORSŹ C10 ....S. +8 239
>1: ĄBGNWZZ 13M ..Ą +7 278
>2: AIKKORŹ M9 RA... +7 246
>1: BGNŃWZZ 10K .N.Ń +26 304
>2: IKKMOŹŻ 9M .OK +12 258
>1: BGHUWZZ N7 ZG.. +12 316
#>2: IKMÓŹŻ M1 ŻM.. +13 271
#>1: BHUWZ -  +0 316
#>2: IKÓŹ B8 KÓ. +9 280
#>1: BHUWZ -  +0 316
#>2: IŹ 8B .IŹ +21 301
#>2:  (BHUWZ) +22 323
    ",
        "IKMÓŹŻ",
    )?;
    let _question = Question::from_gcg(
        &game_config::make_polish_game_config(),
        "OSPS49", // actually "OSPS44",
        r"#character-encoding UTF-8
#player1 1 ptf1559
#player2 2 smut3k
>1: AHIJOUY 8F HUJA +20 20
>1: AHIJOUY --  -20 0
>2: ĆĘIKPST 8G STĘPIĆ +46 46
>1: AHIJOUY 7I HOI +24 24
>2: CFKNWYZ J5 CZ..Y +10 56
>1: AJMSUWY -  +0 24
>2: FKLNŃWW -  +0 56
>1: AJMSUWY -J +0 24
>2: FKLNŃWW -KWŃW +0 56
>1: AAMSUWY 9F AU +14 38
>2: AFLLŁNO 5J .ŁA +12 68
>1: AAMSWYZ 10E SAMY +18 56
>2: CEFLLNO 11C CLE +14 82
>1: AEŁNRWZ 12D AR +13 69
>2: FLNOOOW L3 FL.N +18 100
>1: DEŁNTWZ 13E WENT +10 79
>2: EJNOOOW H13 .EJ +18 118
>1: ?ADIŁZZ 15A ZDZIAŁa. +89 168
>2: IKNOOOW I13 ON +12 130
>1: ABEIMNS 12I SAMBIE +21 189
>2: IIKOOTW M11 K.I +8 138
>1: ?EJNŚWW A14 E. +2 191
>2: AILOOTW C11 .LI +14 152
>1: ?JNNŚWW 12C ...W +7 198
>2: AOOOTWŹ -Ź +0 152
>1: ?AJNNŚW 3L .iŚ +20 218
>2: AGOOOTW K10 GA.O +14 166
>1: AIJNNRW 4L .I +4 222
>2: OORSTWY N12 .WY +10 176
>1: AJNNRRW 11J J. +8 230
>2: ADOORST 12C ....O +8 184
>1: ANNPRRW 9F ..R +10 240
>2: ADORSTY J14 DY +16 200
>1: ACNNPRW 13A CN. +8 248
>2: AOOPRST 10B PO +13 213
>1: ADNPRWZ H8 ...P +7 255
>2: AEKORST 2M KET +25 238
>2: AEKORST --  -25 213
>1: AĄDNRWZ 15J .ARD +7 262
>2: AEKORST 2N ET +18 231
>1: ĄBEGNWZ O1 E. +9 271
>2: AKKORSŹ C10 ....S. +8 239
>1: ĄBGNWZZ 13M ..Ą +7 278
>2: AIKKORŹ M9 RA... +7 246
>1: BGNŃWZZ 10K .N.Ń +26 304
>2: IKKMOŹŻ 9M .OK +12 258
>1: BGHUWZZ N7 ZG.. +12 316
>2: IKMÓŹŻ M1 ŻM.. +13 271
>1: BHUWZ -  +0 316
>2: IKÓŹ 1L I. +7 280
#>1: BHUWZ -  +0 316
#>2: IŹ 8B .IŹ +21 301
#>2:  (BHUWZ) +22 323
    ",
        "BHUWZ",
    )?;
    let _question = Question::from_gcg(
        &game_config::make_polish_game_config(),
        "OSPS49", // actually "OSPS44",
        r"#character-encoding UTF-8
#player1 1 ptf1559
#player2 2 smut3k
>1: AHIJOUY 8F HUJA +20 20
>1: AHIJOUY --  -20 0
>2: ĆĘIKPST 8G STĘPIĆ +46 46
>1: AHIJOUY 7I HOI +24 24
>2: CFKNWYZ J5 CZ..Y +10 56
>1: AJMSUWY -  +0 24
>2: FKLNŃWW -  +0 56
>1: AJMSUWY -J +0 24
>2: FKLNŃWW -KWŃW +0 56
>1: AAMSUWY 9F AU +14 38
>2: AFLLŁNO 5J .ŁA +12 68
>1: AAMSWYZ 10E SAMY +18 56
>2: CEFLLNO 11C CLE +14 82
>1: AEŁNRWZ 12D AR +13 69
>2: FLNOOOW L3 FL.N +18 100
>1: DEŁNTWZ 13E WENT +10 79
>2: EJNOOOW H13 .EJ +18 118
>1: ?ADIŁZZ 15A ZDZIAŁa. +89 168
>2: IKNOOOW I13 ON +12 130
>1: ABEIMNS 12I SAMBIE +21 189
>2: IIKOOTW M11 K.I +8 138
>1: ?EJNŚWW A14 E. +2 191
>2: AILOOTW C11 .LI +14 152
>1: ?JNNŚWW 12C ...W +7 198
>2: AOOOTWŹ -Ź +0 152
>1: ?AJNNŚW 3L .iŚ +20 218
>2: AGOOOTW K10 GA.O +14 166
>1: AIJNNRW 4L .I +4 222
>2: OORSTWY N12 .WY +10 176
>1: AJNNRRW 11J J. +8 230
>2: ADOORST 12C ....O +8 184
>1: ANNPRRW 9F ..R +10 240
>2: ADORSTY J14 DY +16 200
>1: ACNNPRW 13A CN. +8 248
>2: AOOPRST 10B PO +13 213
>1: ADNPRWZ H8 ...P +7 255
>2: AEKORST 2M KET +25 238
>2: AEKORST --  -25 213
>1: AĄDNRWZ 15J .ARD +7 262
>2: AEKORST 2N ET +18 231
>1: ĄBEGNWZ O1 E. +9 271
>2: AKKORSŹ C10 ....S. +8 239
>1: ĄBGNWZZ 13M ..Ą +7 278
>2: AIKKORŹ M9 RA... +7 246
>1: BGNŃWZZ 10K .N.Ń +26 304
>2: IKKMOŹŻ 9M .OK +12 258
>1: BGHUWZZ N7 ZG.. +12 316
>2: IKMÓŹŻ M1 ŻM.. +13 271
>1: BHUWZ -  +0 316
>2: IKÓŹ 1L I. +7 280
>1: BHUWZ 7M B.U +10 316
    ",
        "KÓŹ",
    )?;
    // https://woogles.io/game/8hg8XMSK?turn=28
    let _question = Question::from_gcg(
        &game_config::make_english_game_config(),
        "ECWL",
        r"#character-encoding UTF-8
#description Created with Macondo
#id io.woogles 8hg8XMSK
#lexicon ECWL
#note Variant: classic
#note BoardLayout: CrosswordGame
#note LetterDistribution: english
#player1 deldar182 deldar182
#player2 BobaFett BobaFett
>deldar182: EFHIIST H8 IF +10 10
>BobaFett: ADENRRV -RV +0 0
>deldar182: EEHIOST G8 OI +11 21
>BobaFett: ACDEFNR 9F D..F +15 15
>deldar182: EEHPSTU 7F UP +15 36
>BobaFett: ACEEINR E5 NICE +19 34
>deldar182: EEEHLST D8 HEELS +33 69
>BobaFett: ACEERST C2 CREATES +76 110
>deldar182: EETTUUV 11D .UV +12 81
>BobaFett: EIKLTWX 4A WE.K +30 140
>deldar182: EEOTTUY F9 .U.ET +11 92
>BobaFett: EEILLTX A4 .ILLET +27 167
>deldar182: EGOTTYZ -GOTYZ +0 92
>BobaFett: DEGHIOX B9 OX +27 194
>deldar182: AEEGOPT 10I AGE +13 105
>BobaFett: BDEGHIO 11I BOD +29 223
>deldar182: ELOPRTY L11 YO +20 125
>BobaFett: AAEGHIT C12 AHI +14 237
>deldar182: ELMPRTT K5 TEMPT.. +24 149
>BobaFett: AAEGRTW B12 WAG +31 268
>deldar182: ADLNRSU 13K DUAL +16 165
>BobaFett: AEOOORT 2C .OO +5 273
>deldar182: ?MNNORS 1E NORM +20 185
>BobaFett: AAEORRT 14J AORTA +30 303
>deldar182: ?EGINNS 15F ENdINGS +90 275
>BobaFett: DENRRVY 8K .ERVY +42 345
>deldar182: AABIIJS J4 JAB +30 305
    ",
        "DNNQRRZ",
    )?;
    // https://woogles.io/game/YDRLWKJj?turn=23
    let _question = Question::from_gcg(
        &game_config::make_english_game_config(),
        "CSW24", // actually "CSW21",
        r"#character-encoding UTF-8
#description Created with Macondo
#id io.woogles YDRLWKJj
#lexicon CSW21
#note BoardLayout: CrosswordGame
#note LetterDistribution: english
#player1 thams Adheesha Dissanayake
#player2 STEEBot STEEBot
>thams: CDIKNNY 8H DICKY +38 38
>STEEBot: GILOQUW K5 GOW. +24 24
>thams: BEIMNNP L4 PINE. +30 68
>STEEBot: EFGILQU I7 F.QUE +31 55
>thams: BLMNRRU 11H B.RM +16 84
>STEEBot: GGIILNR H11 .LING +27 82
>thams: AILNRRU 12K UR +8 92
>STEEBot: GIIPRST M3 GIP +22 104
>thams: AAILNRS 15D LARI.ANS +60 152
>STEEBot: EEIRSST 12G E.S +21 125
>thams: AANSTTZ 14E ZA +28 180
>STEEBot: AEIORST H1 ASTEROI. +85 210
>thams: AENSTTX 2A SEXTANT. +84 264
>STEEBot: DEEINNU 3B NIED +35 245
>thams: ADEHIOR 1A OH +27 291
>STEEBot: ?EHNTUV F10 HUT +18 263
>thams: ADEEIOR 4D DOE +20 311
>STEEBot: ??AENTV M9 VAuNTEd +77 340
>thams: AACEILR 10D AC. +8 319
>STEEBot: EEIJOVY C7 JIVE +35 375
>thams: ABEILRU B10 BURIAL +46 365
>STEEBot: AEOOOTY A13 OYE +31 406
#>thams: DEEMORW 8A WE.D +30 395
#>STEEBot: AFLOOT N1 FOOT +22 428
#>thams: EMOR O1 EM +31 426
#>STEEBot: AL D6 LA. +13 441
#>STEEBot: (OR) +4 445
    ",
        "DEEMORW",
    )?;
    let _question = Question::from_gcg(
        &game_config::make_german_game_config(),
        "RD28",
        r"#player1 Thomas Thomas
#player2 Alex Alex
#description Saved by Elise version 0.1.8
#lexicon GERMAN
>Thomas: DEEINNR H4 DIENERN +66 66
>Alex: EEHORSZ 7F ZON +8 8
>Thomas: FO 10F FON +15 81
>Alex: EEEHJRS G6 JO +15 23
>Thomas: AGLRTÖÜ -LÖÜ +0 81
>Alex: EEEHRSS I7 EH +15 38
>Thomas: U F7 ZU +4 85
>Alex: EEMMRSS 11H SEMS +20 58
>Thomas: CEEHIRS K4 SCHIERES +74 159
>Alex: EMMNNRU 4A NUMMERND +76 134
>Thomas: ADEEHRT A3 ANDREHTE +77 236
>Alex: GILOSÖ? 4J ÖSI +20 154
>Thomas: NNV E3 VENN +18 254
>Alex: EGLOST? M2 LOSGEhT +79 233
>Thomas: ADS N1 DAS +15 269
>Alex: EHKNRTY 10J HENRY +48 281
#note 4-ply winprob simulation (1150), +11.25 / 45.5% [54.18s]
>Thomas: AKM C2 KAMM +22 291
>Alex: EEKNTTÄ 8M TÄT +24 305
>Thomas: ABST O1 ABTS +37 328
>Alex: EEEGKNT F10 FEG +7 312
>Thomas: IX B9 IX +52 380
>Alex: EEKNTUÜ 11A TENÜ +29 341
>Thomas: CENT 13C CENT +22 402
>Alex: EGKRSU? D9 KRÜGE +28 369
>Thomas: FL 7A ELF +11 413
>Alex: EIILSU? 6J UH +5 374
>Thomas: UW O6 WUT +5 418
#>Alex: EIILQS? O10 SEIL +22 396
#>Alex: EIILQS? -- -22 374
#>Thomas: ABDEPUU J3 BÖ +11 429
#>Alex: EIILQS? N10 YEtI +12 386
#>Thomas: ADEPUU M13 PUD +22 451
#>Alex: ILQS 14L LUS +34 420
#>Thomas: AEU G1 AUEN +5 456
#>Thomas: (IQ) +22 478
    ",
        "EIILQS?",
    )?;
    let question = Question::from_fen(
        &game_config::make_english_game_config(),
        "NWL23", // actually "NWL20",
        "5BERGS5/4PA3U5/2QAID3R5/3BEE3F2S2/1P1ET2VIATIC2/MA1TAW3c2H2/ES3IS2E2A2/AT1FOLIA4V2/LI1L1EX1E6/1N1O1D2N2Y3/1GNU2C1JETE3/2ER2OHO2N3/2O3GOY6/1INDOW1U7/4DORR7",
        "IKLMTZ",
    )?;
    let _ = question;
    let question = Question::from_fen(
        &game_config::make_english_game_config(),
        "CSW24", // actually "CSW21",
        "3J1Q1CILIA3/1GLUEISH3LAP1/3S1N5OXO1/3T3E2ZO3/4BROMATEs3/5E1Y2N4/2DUIT1DE6/1DIG1A1EM6/5I2p6/5L2OF2P2/4WO2REV1U2/4AR1KITINGS1/4U3ATT1HEY/4R5A2WE/CONFS1ABOVE2NA",
        "DEEIN",
    )?;
    // https://discord.com/channels/741321677828522035/768655474467012608/1010179294665916476
    // 353-236: +117
    let _ = question;
    let question = Question::from_fen(
        &game_config::make_english_game_config(),
        "CSW24", // actually "CSW19",
        "14C/13QI/12FIE/10VEE1R/9KIT2G/8CIG1IDE/8UTA2AS/7ST1SYPh1/6JA5A1/5WOLD2BOBA/3PLOT1R1NU1EX/Y1VEIN1NOR1mOA1/UT1AT1N1L2FEH1/GUR2WIRER5/SNEEZED8", // 353-236: +117 "AHIILMM"
        "ADENOOO",
    )?;
    // 353-236: +117
    // Bob must helplessly pass 6 times in a row as Alfred starts with M8 PHO, then HAM/MAX, MAXI, MAXIM, MAXIMA, MAXIMAL. once Alfred is done now Bob can do AL/DAL/ODAL/NODAL/ENODAL, then LO/RORE to the E in ENODAL, then NOLO, to win by 1 point, for a total of 25 moves, 12 of which are passes
    let _ = question;
    let question = Question::from_fen(
        &game_config::make_english_game_config(),
        "CSW24", // actually "CSW19",
        // "14C/13QI/12FIE/10VEE1R/9KIT2G/8CIG1IDE/8UTA2AS/7ST1SYPh1/6JA4HA1/5WOLD2BOBA/3PLOT1R1NU1EX/Y1VEIN1NOR1mOA1/UT1AT1N1L2FEH1/GUR2WIRER5/SNEEZED8", // 9M H(A): +21 -96 "AIILMM"
        // "14C/13QI/12FIE/10VEE1R/9KIT2G/8CIG1IDE/8UTA2AS/7ST1SYPh1/6JA4HAM/5WOLD2BOBA/3PLOT1R1NU1EX/Y1VEIN1NOR1mOA1/UT1AT1N1L2FEH1/GUR2WIRER5/SNEEZED8", // 9M (HA)M: +20 -76 "AIILM"
        // "14C/13QI/12FIE/10VEE1R/9KIT2G/8CIG1IDE/8UTA2AS/7ST1SYPh1/6JA4HAM/5WOLD2BOBA/3PLOT1R1NU1EX/Y1VEIN1NOR1mOAI/UT1AT1N1L2FEH1/GUR2WIRER5/SNEEZED8", // 12I (mOA)I: +18 -58 "AILM"
        // "14C/13QI/12FIE/10VEE1R/9KIT2G/8CIG1IDE/8UTA2AS/7ST1SYPh1/6JA4HAM/5WOLD2BOBA/3PLOT1R1NU1EX/Y1VEIN1NOR1mOAI/UT1AT1N1L2FEHM/GUR2WIRER5/SNEEZED8", // 13I (FEH)M: +28 -30 "AIL"
        // "14C/13QI/12FIE/10VEE1R/9KIT2G/8CIG1IDE/8UTA2AS/7ST1SYPh1/6JA4HAM/5WOLD2BOBA/3PLOT1R1NU1EX/Y1VEIN1NOR1mOAI/UT1AT1N1L2FEHM/GUR2WIRER4A/SNEEZED8", // O9 (MAXIM)A: +17 -13 "IL"
        "14C/13QI/12FIE/10VEE1R/9KIT2G/8CIG1IDE/8UTA2AS/7ST1SYPh1/6JA4HAM/5WOLD2BOBA/3PLOT1R1NU1EX/Y1VEIN1NOR1mOAI/UT1AT1N1L2FEHM/GUR2WIRER4A/SNEEZED7L", // O9 (MAXIMA)L: +54 +41 "I"
        "ADENOOO",
    )?;
    let _ = question;
    // https://woogles.io/game/BLRma3oH?turn=20
    let _question = Question::from_gcg(
        &game_config::make_english_game_config(),
        "CSW24", // actually "CSW21",
        r"#character-encoding UTF-8
#description Created with Macondo
#id io.woogles BLRma3oH
#lexicon CSW21
#note Variant: classic
#note BoardLayout: CrosswordGame
#note LetterDistribution: english
#player1 STEEBot STEEBot
#player2 rak1507 rak1507
>STEEBot: AEHNORT 8H ANOTHER +78 78
>rak1507: AADEOOR 9J ROADEO +25 25
>STEEBot: AIIOOPU 7M POI +20 98
>rak1507: ABIMRSW 6K BRAW +39 64
>STEEBot: AINOOTU 5K OUT +24 122
>rak1507: AEGIIMS 10G IMAGE +23 87
>STEEBot: ?AINOST 4F rATIONS +75 197
>rak1507: EEFILSS 11D SELFIES +86 173
>STEEBot: EILNTXY 3G YEX +53 250
>rak1507: AIIMRUU -IIMUU +0 173
>STEEBot: ABIILNT 12A BINIT +26 276
>rak1507: AAEPRVY O9 .VERPAY +48 221
>STEEBot: AIKLNRS 14J KIRAN. +40 316
>rak1507: AAEFHMT B10 FA.TH +38 259
>STEEBot: ELLRSUW 7E WULL +10 326
>rak1507: AEEGIMO 6D MAGE +25 284
>STEEBot: DEORSVZ 15H DZO +45 371
>rak1507: DEEILNO C3 ELOINED +80 364
>STEEBot: EEJQRSV 8A RE.VES +49 420
#>rak1507: ?CGINTU 13K GUT +17 381
#>STEEBot: CDJQU E5 J...D +32 452
#>rak1507: ?CIN 13A I. +6 387
#>STEEBot: CQU H10 ..C +10 462
#>rak1507: ?CN A12 ..Ce +28 415
#>STEEBot: QU M12 U.. +3 465
#>rak1507: N D11 ..N +3 418
#>rak1507: (Q) +20 438
    ",
        "CGINTU?",
    )?;

    let kwg;
    let game_config;

    // of course this should be cached
    match question.lexicon.as_str() {
        "CSW24" => {
            kwg = kwg::Kwg::from_bytes_alloc(&std::fs::read("lexbin/CSW24.kwg")?);
            game_config = game_config::make_english_game_config();
        }
        "NWL23" => {
            kwg = kwg::Kwg::from_bytes_alloc(&std::fs::read("lexbin/NWL23.kwg")?);
            game_config = game_config::make_english_game_config();
        }
        "ECWL" => {
            kwg = kwg::Kwg::from_bytes_alloc(&std::fs::read("lexbin/ECWL.kwg")?);
            game_config = game_config::make_english_game_config();
        }
        "OSPS49" => {
            kwg = kwg::Kwg::from_bytes_alloc(&std::fs::read("lexbin/OSPS49.kwg")?);
            game_config = game_config::make_polish_game_config();
        }
        "RD28" => {
            kwg = kwg::Kwg::from_bytes_alloc(&std::fs::read("lexbin/RD28.kwg")?);
            game_config = game_config::make_german_game_config();
        }
        _ => {
            wolges::return_error!(format!("invalid lexicon {:?}", question.lexicon));
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
            wolges::return_error!(format!(
                "rack has invalid tile {tile}, alphabet size is {alphabet_len_without_blank}",
            ));
        }
        if available_tally[tile as usize] > 0 {
            available_tally[tile as usize] -= 1;
        } else {
            wolges::return_error!(format!(
                "too many tile {} (bag contains only {})",
                tile,
                alphabet.freq(tile),
            ));
        }
    }

    let expected_dim = game_config.board_layout().dim();
    if question.board_tiles.len() != expected_dim.rows as usize {
        wolges::return_error!(format!(
            "board: need {} rows, found {} rows",
            expected_dim.rows,
            question.board_tiles.len(),
        ));
    }
    for (row_num, row) in (0..).zip(question.board_tiles.iter()) {
        if row.len() != expected_dim.cols as usize {
            wolges::return_error!(format!(
                "board row {} (0-based): need {} cols, found {} cols",
                row_num,
                expected_dim.cols,
                row.len(),
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
                    wolges::return_error!(format!(
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
                    wolges::return_error!(format!(
                        "too many tile {} (bag contains only {})",
                        0,
                        alphabet.freq(0),
                    ));
                }
            } else {
                wolges::return_error!(format!(
                    "board row {row_num} col {col_num} (0-based): invalid tile {signed_tile}, alphabet size is {alphabet_len_without_blank}",
                ));
            }
        }
    }

    // this allocates
    let oppo_rack = (0u8..)
        .zip(available_tally.iter())
        .flat_map(|(tile, &count)| std::iter::repeat_n(tile, count as usize))
        .collect::<Box<_>>();
    if oppo_rack.len() > game_config.rack_size() as usize {
        wolges::return_error!(format!(
            "not endgame yet as there are {} unseen tiles",
            oppo_rack.len(),
        ));
    }

    // perform word prune.

    let mut move_generator = movegen::KurniaMoveGenerator::new(&game_config);
    // these always allocate for now.
    let mut set_of_words = fash::MyHashSet::<bites::Bites>::default();
    move_generator.gen_remaining_words(
        &movegen::BoardSnapshot {
            board_tiles: &board_tiles,
            game_config: &game_config,
            kwg: &kwg,
            klv: &klv::Klv::from_bytes_alloc(klv::EMPTY_KLV_BYTES),
        },
        |word: &[u8]| {
            set_of_words.insert(word.into());
        },
    );
    println!("word_prune: {} words", set_of_words.len());
    let mut vec_of_words = set_of_words.into_iter().collect::<Vec<_>>();
    vec_of_words.sort_unstable();
    let smaller_kwg_bytes = build::build(
        build::BuildContent::Gaddawg,
        build::BuildLayout::Wolges,
        &vec_of_words.into_boxed_slice(),
    )?;
    println!("word_prune: {} bytes kwg", smaller_kwg_bytes.len());
    let smaller_kwg = kwg::Kwg::from_bytes_alloc(&smaller_kwg_bytes);
    move_generator.reset_for_another_kwg();

    let mut egs = endgame::EndgameSolver::new(&game_config, &smaller_kwg);
    egs.init(&board_tiles, [&question.rack, &oppo_rack]);
    egs.evaluate(0);

    Ok(())
}
