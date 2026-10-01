// Copyright (C) 2020-2026 Andy Kurnia.

use super::{game_config, klv, kwg, movegen, prob};
use rand::prelude::*;

#[derive(Clone)]
pub struct LimitedVocabChecker {
    word_check_buf: Vec<u8>,
}

impl LimitedVocabChecker {
    pub fn new() -> Self {
        Self {
            word_check_buf: Vec::new(),
        }
    }

    #[inline(always)]
    pub fn words_placed_are_ok<WordIsOk: FnMut(&[u8]) -> bool, N: kwg::Node, L: kwg::Node>(
        &mut self,
        board_snapshot: &movegen::BoardSnapshot<'_, N, L>,
        down: bool,
        lane: i8,
        idx: i8,
        word: &[u8],
        mut word_is_ok: WordIsOk,
    ) -> bool {
        let board_layout = board_snapshot.game_config.board_layout();
        let dim = board_layout.dim();
        let strider = dim.lane(down, lane);
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
                let perpendicular_strider = dim.lane(!down, i);
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

impl Default for LimitedVocabChecker {
    #[inline(always)]
    fn default() -> Self {
        Self::new()
    }
}

// need more realistic numbers, and should differ by bot level
pub const IMPORTANCE_DENOM: u64 = 100;
static LENGTH_IMPORTANCES: &[u8] = &[
    0, 0, 200, 150, 100, 75, 50, 100, 100, 50, 40, 30, 20, 10, 10, 10,
];

#[derive(Clone)]
pub struct Tilt<'a> {
    word_prob: Box<prob::WordProbability>,
    max_prob_by_len: Box<[u64]>,
    length_importances: &'a [u8],
    pub tilt_factor: i32, // 0..=TILT_DENOM
    pub leave_scale: i32, // 0..=LEAVE_SCALE_DENOM
    limited_vocab_checker: LimitedVocabChecker,
}

pub const TILT_DENOM: i32 = 1024;
pub const LEAVE_SCALE_DENOM: i32 = 1024;

impl<'a> Tilt<'a> {
    #[inline(always)]
    pub fn length_importances() -> &'a [u8] {
        LENGTH_IMPORTANCES
    }

    pub fn new<N: kwg::Node>(
        game_config: &game_config::GameConfig,
        kwg: &kwg::Kwg<N>,
        length_importances: &'a [u8],
    ) -> Self {
        let mut word_prob = prob::WordProbability::new(game_config.alphabet());
        let mut max_prob_by_len = Vec::new();
        word_prob.get_max_probs_by_len(kwg, &mut max_prob_by_len);
        Self {
            word_prob: Box::new(word_prob),
            max_prob_by_len: max_prob_by_len.into_boxed_slice(),
            length_importances,
            tilt_factor: 0,
            leave_scale: LEAVE_SCALE_DENOM,
            limited_vocab_checker: LimitedVocabChecker::new(),
        }
    }

    #[inline(always)]
    pub fn tilt_to(&mut self, new_tilt_factor: i32, bot_level: i8) {
        self.tilt_factor = new_tilt_factor.clamp(0, TILT_DENOM);

        self.leave_scale =
            (bot_level as i32 * 102 + TILT_DENOM - self.tilt_factor).clamp(0, LEAVE_SCALE_DENOM);
    }

    #[inline(always)]
    pub fn tilt_by_rng(&mut self, rng: &mut dyn Rng, bot_level: i8) {
        let lo = TILT_DENOM / 2 - bot_level as i32 * 102;
        self.tilt_to(rng.random_range(lo..TILT_DENOM), bot_level);
    }

    #[inline(always)]
    fn word_is_ok(&mut self, word: &[u8]) -> bool {
        if self.tilt_factor <= 0 {
            true
        } else if self.tilt_factor >= TILT_DENOM {
            false
        } else {
            let word_len = word.len();
            let this_wp = self.word_prob.word_draw_ways(word);
            let max_wp = self.max_prob_by_len[word_len];
            // accept if importance[len] * p*(2-p) >= tilt_factor, p = this_wp / max_wp, cross
            // multiplied so every term stays a non-negative integer.
            let imp = self.length_importances[word_len] as u128;
            let twp = this_wp as u128;
            let mwp = max_wp as u128;
            let lhs = imp * twp * (2 * mwp - twp) * TILT_DENOM as u128;
            let rhs = self.tilt_factor as u128 * IMPORTANCE_DENOM as u128 * mwp * mwp;
            if lhs >= rhs {
                true
            } else {
                if false {
                    println!(
                        "Rejecting word {:?}, lhs={} rhs={} (this={} over max={}), tilt={}",
                        word, lhs, rhs, this_wp, max_wp, self.tilt_factor
                    );
                }
                false
            }
        }
    }
}

pub enum GenMoves<'a> {
    Unfiltered,
    Tilt { tilt: Tilt<'a>, bot_level: i8 },
}

impl GenMoves<'_> {
    #[inline(always)]
    pub fn gen_moves<N: kwg::Node, L: kwg::Node>(
        &mut self,
        move_generator: &mut movegen::KurniaMoveGenerator,
        board_snapshot: &movegen::BoardSnapshot<'_, N, L>,
        rack: &[u8],
        num_exchanges_by_this_player: i16,
        max_gen: usize,
        dynamic_leaves: Option<klv::DynamicLeavesRef<'_>>,
    ) {
        match self {
            Self::Unfiltered => {
                move_generator.gen_moves_unfiltered(&movegen::GenMovesParams {
                    board_snapshot,
                    rack,
                    max_gen,
                    num_exchanges_by_this_player,
                    pass_policy: movegen::PassPolicy::OnlyWhenForced,
                    dynamic_leaves,
                });
            }
            Self::Tilt { tilt, bot_level: _ } => {
                let leave_scale = tilt.leave_scale;
                let mut limited_vocab_checker = std::mem::take(&mut tilt.limited_vocab_checker);
                move_generator.gen_moves_filtered(
                    &movegen::GenMovesParams {
                        board_snapshot,
                        rack,
                        max_gen,
                        num_exchanges_by_this_player,
                        pass_policy: movegen::PassPolicy::OnlyWhenForced,
                        dynamic_leaves,
                    },
                    movegen::PlacePredicate::Dyn(
                        &mut |down: bool, lane: i8, idx: i8, word: &[u8], _score: i32| {
                            limited_vocab_checker.words_placed_are_ok(
                                board_snapshot,
                                down,
                                lane,
                                idx,
                                word,
                                |word: &[u8]| tilt.word_is_ok(word),
                            )
                        },
                    ),
                    klv::AdjustLeave::Scaled {
                        scale: leave_scale,
                        denom: LEAVE_SCALE_DENOM,
                    },
                    movegen::EquityPredicate::AcceptAll,
                );
                tilt.limited_vocab_checker = limited_vocab_checker;
            }
        }
    }

    #[inline(always)]
    pub fn gen_nonplacing_moves<N: kwg::Node, L: kwg::Node>(
        &mut self,
        move_generator: &mut movegen::KurniaMoveGenerator,
        board_snapshot: &movegen::BoardSnapshot<'_, N, L>,
        rack: &[u8],
        num_exchanges_by_this_player: i16,
        max_gen: usize,
        pass_policy: movegen::PassPolicy,
    ) {
        let params = movegen::GenMovesParams {
            board_snapshot,
            rack,
            max_gen,
            num_exchanges_by_this_player,
            pass_policy,
            dynamic_leaves: None,
        };
        match self {
            Self::Unfiltered => {
                move_generator.gen_moves_filtered(
                    &params,
                    movegen::PlacePredicate::RejectAll,
                    klv::AdjustLeave::Identity,
                    movegen::EquityPredicate::AcceptAll,
                );
            }
            Self::Tilt { tilt, bot_level: _ } => {
                let leave_scale = tilt.leave_scale;
                move_generator.gen_moves_filtered(
                    &params,
                    movegen::PlacePredicate::RejectAll,
                    klv::AdjustLeave::Scaled {
                        scale: leave_scale,
                        denom: LEAVE_SCALE_DENOM,
                    },
                    movegen::EquityPredicate::AcceptAll,
                );
            }
        }
    }
}
