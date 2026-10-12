// Copyright (C) 2020-2026 Andy Kurnia.

use super::{equity, error, game_config, movegen};

// note: only this representation uses -1i8 for blank-as-A (in "board" input
// and "word" response for "action":"play"). everywhere else, use 0x81u8.

#[derive(serde::Serialize, serde::Deserialize, Debug)]
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
                Self::Play {
                    down: *down,
                    lane: *lane,
                    idx: *idx,
                    word: word_played.into(),
                    score: equity::descale_score(*score),
                }
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
                    score: *score * equity::SCALE,
                }
            }
        }
    }
}

#[derive(serde::Serialize, serde::Deserialize, Debug)]
pub struct JsonPlayWithEquity {
    #[serde(serialize_with = "equity_as_printed")]
    pub equity: f32,
    #[serde(flatten)]
    pub play: JsonPlay,
}

// an equity in json as the f32 prints itself with {}: "32" where serde would
// write "32.0", and "71.303" where an f64 made from the f32 would write
// "71.3030014038086"; one that is not finite stays serde's null
fn equity_as_printed<S: serde::Serializer>(equity: &f32, serializer: S) -> Result<S::Ok, S::Error> {
    if !equity.is_finite() {
        serializer.serialize_f32(*equity)
    } else if equity.fract() == 0.0 && equity.abs() < i64::MAX as f32 {
        serializer.serialize_i64(*equity as i64)
    } else {
        serializer.serialize_f64(equity.to_string().parse().unwrap_or(*equity as f64))
    }
}

impl From<&movegen::ValuedMove> for JsonPlayWithEquity {
    #[inline(always)]
    fn from(play: &movegen::ValuedMove) -> Self {
        Self {
            equity: play.equity.as_f64() as f32,
            play: (&play.play).into(),
        }
    }
}

impl From<&JsonPlayWithEquity> for movegen::ValuedMove {
    #[inline(always)]
    fn from(play: &JsonPlayWithEquity) -> Self {
        Self {
            equity: equity::Equity::new((play.equity * equity::SCALE as f32).round() as i32),
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
    use super::{JsonPlay, JsonPlayWithEquity};

    #[test]
    #[inline]
    fn an_equity_goes_into_json_as_the_f32_prints_itself() {
        for (equity, written) in [
            (32.0, "32"),
            (-87.25, "-87.25"),
            (71.303, "71.303"),
            (0.001, "0.001"),
            (0.0, "0"),
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
}
