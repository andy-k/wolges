// Copyright (C) 2020-2021 Andy Kurnia.

use wolges::{endgame, error, game_config, kwg};

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
}

// note: only this representation uses -1i8 for blank-as-A (in "board" input
// and "word" response for "action":"play"). everywhere else, use 0x81u8.

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
    for player_idx in 0..2 {
        println!();
        println!("for player {}:", player_idx);
        egs.evaluate(player_idx);
    }

    Ok(())
}
