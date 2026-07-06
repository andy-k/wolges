// Copyright (C) 2020-2026 Andy Kurnia.

use super::{equity, game_config, game_state, klv, kwg, movegen};
use rand::SeedableRng;

/// Whole-point spread from a millipoint spread. Player scores and klv leave
/// values are accumulated in millipoints (equity::SCALE = 1000), but the
/// win-probability sigmoid and its weightage were tuned in whole points. Feed a
/// spread through this before the sigmoid or the objective so a lead of, say, 30
/// points reads as 30.0 rather than 30000.0 (which would saturate the sigmoid to
/// a step and swamp the win-probability term in the objective).
#[inline(always)]
pub fn spread_points(millipoints: i32) -> f64 {
    millipoints as f64 / equity::SCALE as f64
}

fn set_rack_tally_from_leave(rack_tally: &mut [u8], rack: &[u8], play: &movegen::Play) {
    rack_tally.iter_mut().for_each(|m| *m = 0);
    rack.iter().for_each(|&tile| rack_tally[tile as usize] += 1);
    match &play {
        movegen::Play::Exchange { tiles } => {
            tiles
                .iter()
                .for_each(|&tile| rack_tally[tile as usize] -= 1);
        }
        movegen::Play::Place { word, .. } => {
            word.iter().for_each(|&tile| {
                if tile & 0x80 != 0 {
                    rack_tally[0] -= 1;
                } else if tile != 0 {
                    rack_tally[tile as usize] -= 1;
                }
            });
        }
    };
}

thread_local! {
    // Always concretely a ChaCha20 generator (nothing swaps in another Rng
    // impl), so store it unboxed rather than behind a trait object.
    static RNG: std::cell::RefCell<rand::rngs::ChaCha20Rng> = std::cell::RefCell::new(
        rand::rngs::ChaCha20Rng::try_from_rng(&mut rand::rngs::SysRng).unwrap(),
    );
}

// Simmer can only be reused for the same game_config and kwg.
// (Refer to note at KurniaMoveGenerator.)
// This is not enforced.
pub struct Simmer {
    // prepare() sets/resets these
    initial_game_state: game_state::GameState,
    pub initial_score_spread: i32,
    num_sim_plies: usize,
    num_tiles_that_matter: usize,
    win_prob_weightage: f64,

    // simulate() simulates a single iteration and sets these
    game_state: game_state::GameState,
    last_seen_leave_values: Box<[i32]>,
    final_scores: Box<[i32]>,

    // simulate() reuses these internally
    move_generator: movegen::KurniaMoveGenerator,
    rack_tally: Box<[u8]>,
}

impl Simmer {
    // The other methods must be called with the same game_config.
    pub fn new(game_config: &game_config::GameConfig) -> Self {
        Self {
            initial_game_state: game_state::GameState::new(game_config),
            initial_score_spread: 0,
            num_sim_plies: 0,
            num_tiles_that_matter: 0,
            win_prob_weightage: 0.0,

            game_state: game_state::GameState::new(game_config),
            last_seen_leave_values: vec![0i32; game_config.num_players() as usize]
                .into_boxed_slice(),
            final_scores: vec![0; game_config.num_players() as usize].into_boxed_slice(),

            move_generator: movegen::KurniaMoveGenerator::new(game_config),
            rack_tally: vec![0u8; game_config.alphabet().len() as usize].into_boxed_slice(),
        }
    }

    #[inline(always)]
    pub fn prepare(
        &mut self,
        game_config: &game_config::GameConfig,
        game_state: &game_state::GameState,
        num_sim_plies: usize,
    ) {
        self.initial_game_state.clone_from(game_state);
        self.game_state.clone_from(game_state);
        self.initial_score_spread = game_state.current_player().score
            - (0..)
                .zip(game_state.players.iter())
                .filter(|&(i, _)| i != game_state.turn)
                .map(|(_, player)| player.score)
                .max()
                .unwrap_or(0);
        self.num_sim_plies = num_sim_plies;
        self.num_tiles_that_matter = num_sim_plies * game_config.rack_size() as usize;
        let mut num_unseen_tiles = self.initial_game_state.bag.len();
        let initial_turn = self.initial_game_state.turn as usize;
        for (i, player) in self.initial_game_state.players.iter_mut().enumerate() {
            if i != initial_turn {
                num_unseen_tiles += player.rack.len();
            }
        }
        const W_NO_OUT: f64 = 10.0;
        const W_OUT: f64 = 10000.0;
        self.win_prob_weightage = if num_unseen_tiles <= self.num_tiles_that_matter {
            // possible to play out
            W_OUT
        } else if num_unseen_tiles < 2 * self.num_tiles_that_matter {
            W_OUT
                + ((num_unseen_tiles - self.num_tiles_that_matter) as f64
                    / self.num_tiles_that_matter as f64)
                    * (W_NO_OUT - W_OUT)
        } else {
            W_NO_OUT
        };
    }

    #[inline(always)]
    pub fn prepare_iteration(&mut self) {
        let initial_turn = self.initial_game_state.turn as usize;
        for (i, player) in self.initial_game_state.players.iter_mut().enumerate() {
            if i != initial_turn {
                self.final_scores[i] = player.rack.len() as i32;
                self.initial_game_state.bag.return_tiles(&player.rack);
                player.rack.clear();
            }
        }
        RNG.with(|rng| {
            self.initial_game_state
                .bag
                .shuffle_n(&mut *rng.borrow_mut(), self.num_tiles_that_matter);
        });
        for (i, player) in self.initial_game_state.players.iter_mut().enumerate() {
            if i != initial_turn {
                self.initial_game_state.bag.replenish(
                    &mut player.rack,
                    self.final_scores[i] as usize,
                    i,
                );
            }
        }
    }

    // true iff played out
    #[inline(always)]
    pub fn simulate<N: kwg::Node, L: kwg::Node>(
        &mut self,
        game_config: &game_config::GameConfig,
        kwg: &kwg::Kwg<N>,
        klv: &klv::Klv<L>,
        candidate_play: &movegen::Play,
    ) -> bool {
        self.game_state.clone_from(&self.initial_game_state);
        // reset leave values from previous iteration
        self.last_seen_leave_values.iter_mut().for_each(|m| *m = 0);
        // Copy-on-write rollout rng, for common random numbers across candidates.
        // prepare_iteration() left the shared thread-local rng at the state every
        // candidate in this iteration must roll out from. A Place ply draws its
        // replenishment tiles off the front of the already-shuffled bag (no rng
        // needed) and an empty exchange (a pass) draws nothing either; only a
        // real exchange consumes randomness (returning the exchanged tiles to a
        // random bag position). So this rollout never mutates the shared
        // thread-local directly: the first ply that needs randomness snapshots it
        // into a stack-local ChaCha20 generator (serialize_state/deserialize_state,
        // no heap allocation) and every later draw in this rollout advances that
        // local copy instead, leaving the shared state untouched for the next
        // candidate.
        let mut rollout_rng: Option<rand::rngs::ChaCha20Rng> = None;
        let mut next_play = movegen::Play::Exchange {
            tiles: [][..].into(),
        };
        for ply in 0..=self.num_sim_plies {
            next_play.clone_from(if ply == 0 {
                candidate_play
            } else {
                self.move_generator
                    .gen_moves_unfiltered(&movegen::GenMovesParams {
                        board_snapshot: &movegen::BoardSnapshot {
                            board_tiles: &self.game_state.board_tiles,
                            game_config,
                            kwg,
                            klv,
                        },
                        rack: &self.game_state.current_player().rack,
                        max_gen: 1,
                        num_exchanges_by_this_player: self
                            .game_state
                            .current_player()
                            .num_exchanges,
                        always_include_pass: false,
                        dynamic_leaves: None,
                    });
                &self.move_generator.plays[0].play
            });
            set_rack_tally_from_leave(
                &mut self.rack_tally,
                &self.game_state.current_player().rack,
                &next_play,
            );
            self.last_seen_leave_values[self.game_state.turn as usize] =
                klv.leave_value_from_tally(&self.rack_tally);
            let rng = rollout_rng.get_or_insert_with(|| {
                RNG.with(|rng| {
                    rand::rngs::ChaCha20Rng::deserialize_state(&rng.borrow().serialize_state())
                })
            });
            self.game_state.play(game_config, rng, &next_play).unwrap();
            match self
                .game_state
                .check_game_ended(game_config, &mut self.final_scores)
            {
                game_state::CheckGameEnded::NotEnded => {}
                _ => {
                    // game has ended, move leave values to actual score
                    for (i, player) in self.game_state.players.iter_mut().enumerate() {
                        player.score = self.final_scores[i];
                    }
                    self.last_seen_leave_values.iter_mut().for_each(|m| *m = 0);
                    return true;
                }
            }
            self.game_state.next_turn();
        }
        false
    }

    #[inline(always)]
    pub fn final_equity_spread(&self) -> i32 {
        let mut best_opponent_equity = i32::MIN;
        for (i, player) in (0..).zip(self.game_state.players.iter()) {
            if i != self.initial_game_state.turn {
                let opponent_equity = player.score + self.last_seen_leave_values[i as usize];
                if opponent_equity > best_opponent_equity {
                    best_opponent_equity = opponent_equity;
                }
            }
        }
        let mut this_equity = self.game_state.players[self.initial_game_state.turn as usize].score
            + self.last_seen_leave_values[self.initial_game_state.turn as usize];
        if best_opponent_equity != i32::MIN {
            this_equity -= best_opponent_equity;
        }
        this_equity
    }

    #[inline(always)]
    pub fn compute_win_prob(&self, game_ended: bool, final_spread: i32) -> f64 {
        if game_ended {
            match final_spread.signum() {
                1 => 1.0,
                -1 => 0.0,
                _ => 0.5,
            }
        } else {
            // handwavily: assume a spread of +/- (30 + num_unseen_tiles) points
            // is 90%/10% (-Andy Kurnia). (to adjust these, change the 30.0 and 0.9
            // consts below.)
            let num_unseen_tiles = self.game_state.bag.len()
                + self
                    .game_state
                    .players
                    .iter()
                    .map(|player| player.rack.len())
                    .sum::<usize>();
            // this could be precomputed for every possible num_unseen_tiles (1 to 93)
            let exp_width = -(30.0 + num_unseen_tiles as f64) / (1.0f64 / 0.9 - 1.0).ln();
            // final_spread is in millipoints; descale to points, the units the
            // 30.0/0.9 consts assume.
            1.0 / (1.0 + (-spread_points(final_spread) / exp_width).exp())
        }
    }

    #[inline(always)]
    pub fn win_prob_weightage(&self) -> f64 {
        self.win_prob_weightage
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // A rollout that exchanges tiles must draw from a copy of the shared
    // thread-local rng, not the thread-local itself, so the shared
    // post-prepare_iteration state stays put for the next candidate (common
    // random numbers: every candidate in the iteration rolls out from the same
    // randomness). This drives one candidate's rollout through an exchange and
    // checks the shared state is unchanged afterwards, then repeats the same
    // rollout from that unchanged state and checks it reproduces exactly.
    #[test]
    fn exchanging_rollout_leaves_shared_rng_untouched() {
        let game_config = game_config::make_english_game_config();
        let mut game_state = game_state::GameState::new(&game_config);
        let mut deal_rng = rand::rngs::ChaCha20Rng::seed_from_u64(3);
        game_state.reset_and_draw_tiles(&game_config, &mut deal_rng);

        // num_sim_plies = 0 rolls out exactly the candidate play (no move
        // generation), so an empty kwg suffices and the candidate is the only
        // ply.
        let kwg = kwg::Kwg::<kwg::Node22>::from_bytes_alloc(b"\x00\x00\x40\x00");
        let klv = klv::Klv::<kwg::Node22>::from_bytes_alloc(klv::EMPTY_KLV_BYTES);
        let mut simmer = Simmer::new(&game_config);
        RNG.with(|rng| *rng.borrow_mut() = rand::rngs::ChaCha20Rng::seed_from_u64(99));
        simmer.prepare(&game_config, &game_state, 0);
        simmer.prepare_iteration();

        // exchange the first tile of the player-to-move's rack; put_back is the
        // only rng draw in the rollout.
        let exchanged =
            simmer.initial_game_state.players[simmer.initial_game_state.turn as usize].rack[0];
        let candidate = movegen::Play::Exchange {
            tiles: [exchanged][..].into(),
        };

        // the shared state right after the shared draw.
        let shared_state = RNG.with(|rng| rng.borrow().serialize_state());

        simmer.simulate(&game_config, &kwg, &klv, &candidate);
        let rack_after_first = simmer.game_state.players[simmer.initial_game_state.turn as usize]
            .rack
            .clone();
        // the exchange drew from a copy, so the shared state is untouched.
        assert_eq!(RNG.with(|rng| rng.borrow().serialize_state()), shared_state);

        // a second rollout of the same candidate, from the same untouched
        // shared state, reproduces the first exactly: identical randomness per
        // candidate.
        simmer.simulate(&game_config, &kwg, &klv, &candidate);
        let rack_after_second = simmer.game_state.players[simmer.initial_game_state.turn as usize]
            .rack
            .clone();
        assert_eq!(RNG.with(|rng| rng.borrow().serialize_state()), shared_state);
        assert_eq!(rack_after_first, rack_after_second);
    }

    #[test]
    fn spread_points_descales_millipoints_to_points() {
        // one point is SCALE (1000) millipoints.
        assert_eq!(spread_points(0), 0.0);
        assert_eq!(spread_points(equity::SCALE), 1.0);
        assert_eq!(spread_points(30 * equity::SCALE), 30.0);
        assert_eq!(spread_points(-500), -0.5);
    }
}
