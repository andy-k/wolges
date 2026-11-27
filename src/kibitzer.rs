// Copyright (C) 2020-2026 Andy Kurnia.

use super::{equity, error, game_config, movegen};

// note: only this representation uses -1i8 for blank-as-A (in "board" input
// and "word" response for "action":"play"). everywhere else, use 0x81u8.

#[derive(serde::Serialize, serde::Deserialize, Debug, PartialEq)]
#[serde(tag = "action")]
pub enum JsonPlay {
    #[serde(rename = "exchange")]
    Exchange { tiles: Box<[u8]> },
    #[serde(rename = "play")]
    Play {
        down: bool,
        lane: i8,
        idx: i8,
        word: Box<[i8]>,
        score: i32,
    },
}

impl From<&movegen::Play> for JsonPlay {
    #[inline(always)]
    fn from(play: &movegen::Play) -> Self {
        match &play {
            movegen::Play::Exchange { tiles } => {
                // tiles: array of numbers. 0 for blank, 1 for A.
                Self::Exchange {
                    tiles: tiles[..].into(),
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
                let word_played = word.iter().map(|&x| signed_tile(x)).collect::<Vec<i8>>();
                // across plays: down=false, lane=row, idx=col (0-based).
                // down plays: down=true, lane=col, idx=row (0-based).
                // word: 0 for play-through, 1 for A, -1 for blank-as-A.
                Self::Play {
                    down: *down,
                    lane: *lane,
                    idx: *idx,
                    word: word_played.into(),
                    score: score.descaled().raw(),
                }
            }
        }
    }
}

// a tile as JsonPlay's word writes it: 0x81u8, 0x82u8 turned into -1i8, -2i8
#[inline(always)]
fn signed_tile(x: u8) -> i8 {
    if x & 0x80 != 0 {
        -((x & !0x80) as i8)
    } else {
        x as i8
    }
}

// a play as JsonPlay writes it, read where it is held, so that writing it
// copies no tiles: its word and score as movegen::Play holds them
pub enum JsonPlayRef<'a> {
    Exchange {
        tiles: &'a [u8],
    },
    Play {
        down: bool,
        lane: i8,
        idx: i8,
        word: &'a [u8],
        score: equity::Equity,
    },
}

impl<'a> From<&'a movegen::Play> for JsonPlayRef<'a> {
    #[inline(always)]
    fn from(play: &'a movegen::Play) -> Self {
        match play {
            movegen::Play::Exchange { tiles } => Self::Exchange { tiles },
            movegen::Play::Place {
                down,
                lane,
                idx,
                word,
                score,
            } => Self::Play {
                down: *down,
                lane: *lane,
                idx: *idx,
                word,
                score: *score,
            },
        }
    }
}

// a word's tiles as JsonPlay's word writes them
struct SignedWord<'a>(&'a [u8]);

impl serde::Serialize for SignedWord<'_> {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_seq(self.0.iter().map(|&x| signed_tile(x)))
    }
}

// the fields JsonPlay's derive writes, in its order, its tag first
impl serde::Serialize for JsonPlayRef<'_> {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeStruct;
        match *self {
            Self::Exchange { tiles } => {
                let mut play = serializer.serialize_struct("JsonPlay", 2)?;
                play.serialize_field("action", "exchange")?;
                play.serialize_field("tiles", tiles)?;
                play.end()
            }
            Self::Play {
                down,
                lane,
                idx,
                word,
                score,
            } => {
                let mut play = serializer.serialize_struct("JsonPlay", 6)?;
                play.serialize_field("action", "play")?;
                play.serialize_field("down", &down)?;
                play.serialize_field("lane", &lane)?;
                play.serialize_field("idx", &idx)?;
                play.serialize_field("word", &SignedWord(word))?;
                play.serialize_field("score", &score.descaled().raw())?;
                play.end()
            }
        }
    }
}

impl From<&JsonPlay> for movegen::Play {
    #[inline(always)]
    fn from(play: &JsonPlay) -> Self {
        match &play {
            JsonPlay::Exchange { tiles } => {
                // tiles: array of numbers. 0 for blank, 1 for A.
                Self::Exchange {
                    tiles: tiles[..].into(),
                }
            }
            JsonPlay::Play {
                down,
                lane,
                idx,
                word,
                score,
            } => {
                // turn -1i8, -2i8 into 0x81u8, 0x82u8
                let word_played = word
                    .iter()
                    .map(|&x| if x < 0 { 0x81 + !x as u8 } else { x as u8 })
                    .collect::<Vec<u8>>();
                // across plays: down=false, lane=row, idx=col (0-based).
                // down plays: down=true, lane=col, idx=row (0-based).
                // word: 0 for play-through, 1 for A, -1 for blank-as-A.
                Self::Place {
                    down: *down,
                    lane: *lane,
                    idx: *idx,
                    word: word_played[..].into(),
                    score: equity::Points::new(*score).scaled(),
                }
            }
        }
    }
}

#[derive(serde::Serialize, serde::Deserialize, Debug)]
pub struct JsonPlayWithEquity {
    #[serde(serialize_with = "equity_as_printed")]
    pub equity: f64,
    #[serde(flatten)]
    pub play: JsonPlay,
}

// an equity in json as its shortest decimal, "32" for a whole one where serde
// would write "32.0"; one that is not finite stays serde's null
fn equity_as_printed<S: serde::Serializer>(equity: &f64, serializer: S) -> Result<S::Ok, S::Error> {
    if equity.is_finite() && equity.fract() == 0.0 && equity.abs() < i64::MAX as f64 {
        serializer.serialize_i64(*equity as i64)
    } else {
        serializer.serialize_f64(*equity)
    }
}

impl From<&movegen::ValuedMove> for JsonPlayWithEquity {
    #[inline(always)]
    fn from(play: &movegen::ValuedMove) -> Self {
        Self {
            equity: play.equity.as_f64(),
            play: (&play.play).into(),
        }
    }
}

// a valued play as JsonPlayWithEquity writes it, read where it is held
#[derive(serde::Serialize)]
pub struct JsonPlayRefWithEquity<'a> {
    #[serde(serialize_with = "equity_as_printed")]
    pub equity: f64,
    #[serde(flatten)]
    pub play: JsonPlayRef<'a>,
}

impl<'a> From<&'a movegen::ValuedMove> for JsonPlayRefWithEquity<'a> {
    #[inline(always)]
    fn from(play: &'a movegen::ValuedMove) -> Self {
        Self {
            equity: play.equity.as_f64(),
            play: (&play.play).into(),
        }
    }
}

impl From<&JsonPlayWithEquity> for movegen::ValuedMove {
    #[inline(always)]
    fn from(play: &JsonPlayWithEquity) -> Self {
        Self {
            equity: equity::Equity::new((play.equity * equity::SCALE as f64).round() as i32),
            play: (&play.play).into(),
        }
    }
}

pub struct Kibitzer {
    pub available_tally: Vec<u8>,
    pub board_tiles: Vec<u8>,
}

impl Kibitzer {
    pub fn new() -> Self {
        Self {
            available_tally: Vec::new(),
            board_tiles: Vec::new(),
        }
    }

    #[inline(always)]
    pub fn prepare(
        &mut self,
        game_config: &game_config::GameConfig,
        rack: &[u8],
        signed_board_tiles: &[Vec<i8>],
    ) -> error::Returns<()> {
        let alphabet = game_config.alphabet();
        let alphabet_len_without_blank = alphabet.len() - 1;

        self.available_tally.resize(alphabet.len() as usize, 0);
        for tile in 0..alphabet.len() {
            self.available_tally[tile as usize] = alphabet.freq(tile);
        }

        for &tile in rack {
            if tile > alphabet_len_without_blank {
                return_error!(format!(
                    "rack has invalid tile {tile}, alphabet size is {alphabet_len_without_blank}"
                ));
            }
            if self.available_tally[tile as usize] > 0 {
                self.available_tally[tile as usize] -= 1;
            } else {
                return_error!(format!(
                    "too many tile {} (bag contains only {})",
                    tile,
                    alphabet.freq(tile),
                ));
            }
        }

        let expected_dim = game_config.board_layout().dim();
        if signed_board_tiles.len() != expected_dim.rows as usize {
            return_error!(format!(
                "board: need {} rows, found {} rows",
                expected_dim.rows,
                signed_board_tiles.len()
            ));
        }
        for (row_num, row) in (0..).zip(signed_board_tiles.iter()) {
            if row.len() != expected_dim.cols as usize {
                return_error!(format!(
                    "board row {} (0-based): need {} cols, found {} cols",
                    row_num,
                    expected_dim.cols,
                    row.len()
                ));
            }
        }
        self.board_tiles.clear();
        self.board_tiles
            .reserve((expected_dim.rows as usize) * (expected_dim.cols as usize));
        for (row_num, row) in (0..).zip(signed_board_tiles.iter()) {
            for (col_num, &signed_tile) in (0..).zip(row) {
                if signed_tile == 0 {
                    self.board_tiles.push(0);
                } else if signed_tile as u8 <= alphabet_len_without_blank {
                    let tile = signed_tile as u8;
                    self.board_tiles.push(tile);
                    if self.available_tally[tile as usize] > 0 {
                        self.available_tally[tile as usize] -= 1;
                    } else {
                        return_error!(format!(
                            "too many tile {} (bag contains only {})",
                            tile,
                            alphabet.freq(tile),
                        ));
                    }
                } else if (!signed_tile as u8) < alphabet_len_without_blank {
                    // turn -1i8, -2i8 into 0x81u8, 0x82u8
                    self.board_tiles.push(0x81 + !signed_tile as u8);
                    // verify usage of blank tile
                    if self.available_tally[0] > 0 {
                        self.available_tally[0] -= 1;
                    } else {
                        return_error!(format!(
                            "too many tile {} (bag contains only {})",
                            0,
                            alphabet.freq(0),
                        ));
                    }
                } else {
                    return_error!(format!(
                        "board row {row_num} col {col_num} (0-based): invalid tile {signed_tile}, alphabet size is {alphabet_len_without_blank}"
                    ));
                }
            }
        }

        Ok(())
    }
}

impl Default for Kibitzer {
    #[inline(always)]
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::{
        JsonPlay, JsonPlayRef, JsonPlayRefWithEquity, JsonPlayWithEquity, equity, movegen,
    };

    #[test]
    #[inline]
    fn an_equity_goes_into_json_as_its_shortest_decimal() {
        for (equity, written) in [
            (32.0, "32"),
            (-87.25, "-87.25"),
            (71.303, "71.303"),
            (0.001, "0.001"),
            (0.0, "0"),
            (16384.001, "16384.001"),
            (-2147483.648, "-2147483.648"),
        ] {
            let play = JsonPlayWithEquity {
                equity,
                play: JsonPlay::Exchange {
                    tiles: [][..].into(),
                },
            };
            let json = serde_json::to_string(&play).unwrap();
            assert!(
                json.starts_with(&format!("{{\"equity\":{written},")),
                "{json}"
            );
            assert_eq!(
                serde_json::to_value(&play).unwrap()["equity"].to_string(),
                written
            );
            assert_eq!(
                serde_json::from_str::<JsonPlayWithEquity>(&json)
                    .unwrap()
                    .equity,
                equity
            );
        }
    }

    #[test]
    #[inline]
    fn an_equity_read_from_json_keeps_its_millipoints() {
        for (json, millipoints) in [
            (r#"{"equity":71.303,"action":"exchange","tiles":[]}"#, 71303),
            (
                r#"{"equity":16384.001,"action":"exchange","tiles":[]}"#,
                16384001,
            ),
            (
                r#"{"equity":-2147483.648,"action":"exchange","tiles":[]}"#,
                i32::MIN,
            ),
        ] {
            let play = serde_json::from_str::<JsonPlayWithEquity>(json).unwrap();
            let valued = movegen::ValuedMove::from(&play);
            assert_eq!(valued.equity, equity::Equity::new(millipoints), "{json}");
        }
    }

    #[test]
    #[inline]
    fn a_play_read_in_place_writes_the_json_its_copy_writes() {
        let plays = [
            movegen::Play::Exchange {
                tiles: [][..].into(),
            },
            movegen::Play::Exchange {
                tiles: [0, 3, 3, 26][..].into(),
            },
            movegen::Play::Place {
                down: true,
                lane: 7,
                idx: 3,
                word: [3, 0, 0x81, 20, 0x9a][..].into(),
                score: equity::Points::new(27).scaled(),
            },
            movegen::Play::Place {
                down: false,
                lane: 0,
                idx: 14,
                word: [1][..].into(),
                score: equity::Equity::ZERO,
            },
        ];
        for play in &plays {
            assert_eq!(
                serde_json::to_string(&JsonPlayRef::from(play)).unwrap(),
                serde_json::to_string(&JsonPlay::from(play)).unwrap()
            );
            for equity in [32.0, -87.25, 71.303, 0.001, 0.0] {
                let valued = movegen::ValuedMove {
                    equity: equity::Equity::new((equity * equity::SCALE as f64).round() as i32),
                    play: play.clone(),
                };
                assert_eq!(
                    serde_json::to_string(&JsonPlayRefWithEquity::from(&valued)).unwrap(),
                    serde_json::to_string(&JsonPlayWithEquity::from(&valued)).unwrap()
                );
            }
        }
    }
}
