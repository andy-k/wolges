// Copyright (C) 2020-2021 Andy Kurnia. All rights reserved.

use super::{game_config, kwg, movegen, prob};
use rand::prelude::*;

struct LimitedVocabChecker {
    word_check_buf: Vec<u8>,
}

impl LimitedVocabChecker {
    fn new() -> Self {
        Self {
            word_check_buf: Vec::new(),
        }
    }

    #[inline(always)]
    fn words_placed_are_ok<WordIsOk: FnMut(&[u8]) -> bool>(
        &mut self,
        board_snapshot: &movegen::BoardSnapshot,
        down: bool,
        lane: i8,
        idx: i8,
        word: &[u8],
        mut word_is_ok: WordIsOk,
    ) -> bool {
        let board_layout = board_snapshot.game_config.board_layout();
        let dim = board_layout.dim();
        let strider = if down {
            dim.down(lane)
        } else {
            dim.across(lane)
        };
        self.word_check_buf.clear();
        for (i, &tile) in (idx..).zip(word.iter()) {
            let placed_tile = if tile != 0 {
                tile
            } else {
                board_snapshot.board_tiles[strider.at(i)]
            };
            self.word_check_buf.push(placed_tile & 0x7f);
        }
        if !word_is_ok(&self.word_check_buf) {
            return false;
        }
        for (i, &tile) in (idx..).zip(word.iter()) {
            if tile != 0 {
                let perpendicular_strider = if down { dim.across(i) } else { dim.down(i) };
                let mut j = lane;
                while j > 0 && board_snapshot.board_tiles[perpendicular_strider.at(j - 1)] != 0 {
                    j -= 1;
                }
                let perpendicular_strider_len = perpendicular_strider.len();
                if j == lane
                    && if j + 1 < perpendicular_strider_len {
                        board_snapshot.board_tiles[perpendicular_strider.at(j + 1)] == 0
                    } else {
                        true
                    }
                {
                    // no perpendicular tile
                    continue;
                }
                self.word_check_buf.clear();
                for j in j..perpendicular_strider_len {
                    let placed_tile = if j == lane {
                        tile
                    } else {
                        board_snapshot.board_tiles[perpendicular_strider.at(j)]
                    };
                    if placed_tile == 0 {
                        break;
                    }
                    self.word_check_buf.push(placed_tile & 0x7f);
                }
                if !word_is_ok(&self.word_check_buf) {
                    return false;
                }
            }
        }
        true
    }
}

// need more realistic numbers, and should differ by bot level
static LENGTH_IMPORTANCES: &[f32] = &[
    0.0, 0.0, 2.0, 1.5, 1.0, 0.75, 0.5, 1.0, 1.0, 0.5, 0.4, 0.3, 0.2, 0.1, 0.1, 0.1,
];

pub struct Tilt<'a> {
    word_prob: Box<prob::WordProbability<'a>>,
    max_prob_by_len: Box<[u64]>,
    length_importances: &'a [f32],
    pub tilt_factor: f32,
    pub leave_scale: f32,
    bot_level: i8,
    limited_vocab_checker: LimitedVocabChecker,
}

impl<'a> Tilt<'a> {
    pub fn length_importances() -> &'a [f32] {
        LENGTH_IMPORTANCES
    }

    pub fn new(
        game_config: &'a game_config::GameConfig<'a>,
        kwg: &'a kwg::Kwg,
        length_importances: &'a [f32],
        bot_level: i8,
    ) -> Self {
        let mut word_prob = prob::WordProbability::new(&game_config.alphabet());
        let max_prob_by_len = word_prob.get_max_probs_by_len(&kwg);
        Self {
            word_prob: Box::new(word_prob),
            max_prob_by_len,
            length_importances,
            tilt_factor: 0.0,
            leave_scale: 1.0,
            bot_level,
            limited_vocab_checker: LimitedVocabChecker::new(),
        }
    }

    // to remove when rust 1.50 stabilizes x.clamp(lo, hi)
    #[inline(always)]
    fn clamp(x: f32, lo: f32, hi: f32) -> f32 {
        if x < lo {
            lo
        } else if x > hi {
            hi
        } else {
            x
        }
    }

    #[inline(always)]
    pub fn tilt_to(&mut self, new_tilt_factor: f32) {
        // 0.0 = untilted (can see all valid moves)
        // 1.0 = tilted (can see no valid moves)
        self.tilt_factor = Self::clamp(new_tilt_factor, 0.0, 1.0);
        self.leave_scale = Self::clamp(
            self.bot_level as f32 * 0.1 + (1.0 - self.tilt_factor),
            0.0,
            1.0,
        );
    }

    #[inline(always)]
    pub fn tilt_by_rng(&mut self, rng: &mut dyn RngCore) {
        self.tilt_to(rng.gen_range(0.5 - self.bot_level as f32 * 0.1..1.0));
    }

    #[inline(always)]
    fn word_is_ok(&mut self, word: &[u8]) -> bool {
        if self.tilt_factor <= 0.0 {
            true
        } else if self.tilt_factor >= 1.0 {
            false
        } else {
            let word_len = word.len();
            let this_wp = self.word_prob.count_ways(word);
            let max_wp = self.max_prob_by_len[word_len];
            let handwavy = self.length_importances[word_len]
                * (1.0 - (1.0 - (this_wp as f64 / max_wp as f64)).powi(2)) as f32;
            if handwavy >= self.tilt_factor {
                true
            } else {
                if false {
                    println!(
                        "Rejecting word {:?}, handwavy={} (this={} over max={}), tilt={}",
                        word, handwavy, this_wp, max_wp, self.tilt_factor
                    );
                }
                false
            }
        }
    }
}

pub enum GenMoves<'a> {
    Unfiltered,
    Tilt(Tilt<'a>),
}

impl GenMoves<'_> {
    #[inline(always)]
    pub fn gen_moves(
        &mut self,
        move_generator: &mut movegen::KurniaMoveGenerator,
        board_snapshot: &movegen::BoardSnapshot<'_>,
        rack: &[u8],
        max_gen: usize,
    ) {
        match self {
            Self::Unfiltered => {
                move_generator.gen_moves_unfiltered(board_snapshot, rack, max_gen);
            }
            Self::Tilt(tilt) => {
                let leave_scale = tilt.leave_scale;
                let mut limited_vocab_checker =
                    std::mem::replace(&mut tilt.limited_vocab_checker, LimitedVocabChecker::new());
                move_generator.gen_moves_filtered(
                    board_snapshot,
                    rack,
                    max_gen,
                    |down: bool,
                     lane: i8,
                     idx: i8,
                     word: &[u8],
                     _score: i16,
                     _rack_tally: &[u8]| {
                        limited_vocab_checker.words_placed_are_ok(
                            board_snapshot,
                            down,
                            lane,
                            idx,
                            word,
                            |word: &[u8]| tilt.word_is_ok(word),
                        )
                    },
                    |leave_value: f32| leave_scale * leave_value,
                );
                tilt.limited_vocab_checker = limited_vocab_checker;
            }
        }
    }
}