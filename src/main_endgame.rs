// Copyright (C) 2020-2022 Andy Kurnia.

use rand::prelude::*;
use wolges::{
    alphabet, display, endgame, error, game_config, game_state, klv, kwg, matrix, movegen,
    play_scorer,
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

// /^(?:\d+[A-Z]+|[A-Z]+\d+)$/i
// does not validate that the coordinate is within bounds
fn is_coord_token(coord: &str) -> bool {
    let b = coord.as_bytes();
    let l1 = b
        .iter()
        .position(|c| !c.is_ascii_digit())
        .unwrap_or(b.len());
    let b = &b[l1..];
    let l2 = b
        .iter()
        .position(|c| !c.is_ascii_alphabetic())
        .unwrap_or(b.len());
    if l2 == 0 {
        return false;
    }
    if l1 != 0 {
        return l2 == b.len();
    }
    let b = &b[l2..];
    let l3 = b
        .iter()
        .position(|c| !c.is_ascii_digit())
        .unwrap_or(b.len());
    l3 == b.len()
}

// TODO remove derive
#[derive(Debug)]
struct Coord {
    down: bool,
    lane: i8,
    idx: i8,
}

fn parse_coord_token(coord: &str, dim: matrix::Dim) -> Option<Coord> {
    let b = coord.as_bytes();
    let l1 = b
        .iter()
        .position(|c| !c.is_ascii_digit())
        .unwrap_or(b.len());
    let dig1 = if l1 != 0 {
        i8::try_from(usize::from_str(unsafe { std::str::from_utf8_unchecked(&b[..l1]) }).ok()? - 1)
            .ok()?
    } else {
        0
    };
    let b = &b[l1..];
    let l2 = b
        .iter()
        .position(|c| !c.is_ascii_alphabetic())
        .unwrap_or(b.len());
    if l2 == 0 {
        return None;
    }
    if l1 != 0 && l2 != b.len() {
        return None;
    }
    let alp2 = i8::try_from(display::str_to_column_usize_ignore_case(&b[..l2])?).ok()?;
    if alp2 >= dim.cols {
        return None;
    }
    if l1 != 0 {
        if dig1 >= dim.rows {
            return None;
        }
        return Some(Coord {
            down: false,
            lane: dig1,
            idx: alp2,
        });
    }
    let b = &b[l2..];
    let l3 = b
        .iter()
        .position(|c| !c.is_ascii_digit())
        .unwrap_or(b.len());
    if l3 != b.len() {
        return None;
    }
    let dig3 = i8::try_from(usize::from_str(unsafe { std::str::from_utf8_unchecked(b) }).ok()? - 1)
        .ok()?;
    if dig3 >= dim.rows {
        return None;
    }
    Some(Coord {
        down: true,
        lane: alp2,
        idx: dig3,
    })
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
    let b = &b[if !b.is_empty() && b[0] == b'-' { 1 } else { 0 }..];
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
        game_config: &game_config::GameConfig<'_>,
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
        let mut rng = rand_chacha::ChaCha20Rng::from_entropy();
        let mut game_state = game_state::GameState::new(game_config);
        game_state.reset_and_draw_tiles(game_config, &mut rng);
        let mut game_state_undo = game_state.clone();
        let mut can_withdraw = false;
        let mut v = Vec::new(); // temp buffer
        let parse_rack = |v: &mut Vec<_>, rack: &str| -> Result<(), String> {
            let s = rack;
            v.clear();
            if !s.is_empty() {
                v.reserve(s.len());
                let sb = s.as_bytes();
                let mut ix = 0;
                while ix < sb.len() {
                    if let Some((tile, end_ix)) = racks_alphabet_reader.next_tile(sb, ix) {
                        v.push(tile);
                        ix = end_ix;
                    } else {
                        return Err(format!("invalid tile after {:?} in {:?}", v, s));
                    }
                }
            }
            Ok(())
        };
        for (line_number, line) in (1usize..).zip(gcg.lines()) {
            if !line.starts_with('>') {
                continue;
            }
            let mut tokens = line.split_whitespace();
            macro_rules! fmt_error {
                ($msg: expr) => {
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
            let word_token = if !is_coord_token(coord_token) {
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
            let mut move_score = i16::from_str(score_token)
                .map_err(|e| fmt_error!(format!("invalid score token: {}", e)))?;
            parse_rack(&mut v, rack_token)
                .map_err(|e| fmt_error!(format!("invalid rack token: {}", e)))?;
            game_state.set_current_rack(&v);
            let mut move_to_play = None;
            if coord_token.is_empty() {
                #[allow(clippy::if_same_then_else)]
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
                                    return Err(fmt_error!(format!(
                                        "invalid exchanged tiles {:?}: {}",
                                        exchanged, e
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
                // this parser only supports '.' for skipped tiles
                let s = word_token;
                v.clear();
                if !s.is_empty() {
                    v.reserve(s.len());
                    let sb = s.as_bytes();
                    let mut ix = 0;
                    while ix < sb.len() {
                        if let Some((tile, end_ix)) = plays_alphabet_reader.next_tile(sb, ix) {
                            v.push(tile);
                            ix = end_ix;
                        } else if sb[ix] == b'.' {
                            v.push(0);
                            ix += 1;
                        } else {
                            return Err(fmt_error!(format!(
                                "invalid tile after {:?} in {:?}",
                                v, s
                            )));
                        }
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
                        return Err(fmt_error!(format!(
                            "invalid play {}: {}",
                            play.fmt(board_snapshot),
                            err
                        )));
                    }
                    Ok(_adjusted_play) => {
                        let recounted_score = ps.compute_score(board_snapshot, &play);
                        if move_score != recounted_score {
                            return Err(fmt_error!(format!(
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
                    .map_err(|e| fmt_error!(format!("invalid play: {}", e)))?;
                game_state.next_turn();
            }
        }
        let board_tiles = game_state
            .board_tiles
            .chunks_exact(dim.rows as usize)
            .map(|row| {
                row.iter()
                    .map(|&x| {
                        // turn -1i8, -2i8 into 0x81u8, 0x82u8
                        if x & 0x80 == 0 {
                            x as i8
                        } else {
                            -0x80i8 - (x as i8)
                        }
                    })
                    .collect::<Vec<_>>()
            })
            .collect::<Vec<_>>();
        parse_rack(&mut v, rack)
            .map_err(|e| error::new(format!("invalid rack {:?}: {}", rack, e)))?;
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
        "lexicon": "NWL18",
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
        "lexicon": "NWL18",
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
        "lexicon": "NWL18",
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
        "lexicon": "NWL18",
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
        "lexicon": "NWL20",
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
        "lexicon": "NWL20",
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
        "lexicon": "NWL20",
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
        "lexicon": "CSW19",
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
        "lexicon": "CSW19",
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
        "lexicon": "CSW19",
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
        "OSPS44",
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
        "OSPS44",
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
        "OSPS44",
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
    let question = Question::from_gcg(
        &game_config::make_polish_game_config(),
        "OSPS44",
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

    let kwg;
    let game_config;

    // of course this should be cached
    match question.lexicon.as_str() {
        "CSW21" => {
            kwg = kwg::Kwg::from_bytes_alloc(&std::fs::read("lexbin/CSW21.kwg")?);
            game_config = game_config::make_common_english_game_config();
        }
        "CSW19" => {
            kwg = kwg::Kwg::from_bytes_alloc(&std::fs::read("lexbin/CSW19.kwg")?);
            game_config = game_config::make_common_english_game_config();
        }
        "NWL18" => {
            kwg = kwg::Kwg::from_bytes_alloc(&std::fs::read("lexbin/NWL18.kwg")?);
            game_config = game_config::make_common_english_game_config();
        }
        "NWL20" => {
            kwg = kwg::Kwg::from_bytes_alloc(&std::fs::read("lexbin/NWL20.kwg")?);
            game_config = game_config::make_common_english_game_config();
        }
        "ECWL" => {
            kwg = kwg::Kwg::from_bytes_alloc(&std::fs::read("lexbin/ECWL.kwg")?);
            game_config = game_config::make_common_english_game_config();
        }
        "OSPS42" => {
            kwg = kwg::Kwg::from_bytes_alloc(&std::fs::read("lexbin/OSPS42.kwg")?);
            game_config = game_config::make_polish_game_config();
        }
        "OSPS44" => {
            kwg = kwg::Kwg::from_bytes_alloc(&std::fs::read("lexbin/OSPS44.kwg")?);
            game_config = game_config::make_polish_game_config();
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
                "rack has invalid tile {}, alphabet size is {}",
                tile, alphabet_len_without_blank
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
            question.board_tiles.len()
        ));
    }
    for (row_num, row) in (0..).zip(question.board_tiles.iter()) {
        if row.len() != expected_dim.cols as usize {
            wolges::return_error!(format!(
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
        wolges::return_error!(format!(
            "not endgame yet as there are {} unseen tiles",
            oppo_rack.len()
        ));
    }

    let mut egs = endgame::EndgameSolver::new(&game_config, &kwg);
    egs.init(&board_tiles, [&question.rack, &oppo_rack]);
    egs.evaluate(0);

    Ok(())
}
