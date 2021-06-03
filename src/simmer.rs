// Copyright (C) 2020-2021 Andy Kurnia.

use super::{game_config, game_state, klv, kwg, movegen};
use rand::prelude::*;

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
    static RNG: std::cell::RefCell<Box<dyn RngCore>> =
        std::cell::RefCell::new(Box::new(rand_chacha::ChaCha20Rng::from_entropy()));
}

pub struct Simmer {
    // prepare() sets/resets these
    initial_game_state: game_state::GameState,
    pub initial_score_spread: i16,
    num_sim_plies: usize,
    num_tiles_that_matter: usize,

    // prepare_iteration() sets these
    possible_to_play_out: bool,

    // simulate() simulates a single iteration and sets these
    game_state: game_state::GameState,
    last_seen_leave_values: Box<[f32]>,
    final_scores: Box<[i16]>,

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

            possible_to_play_out: false,

            game_state: game_state::GameState::new(game_config),
            last_seen_leave_values: vec![0.0f32; game_config.num_players() as usize]
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
        self.initial_game_state.clone_from(&game_state);
        self.game_state.clone_from(&game_state);
        self.initial_score_spread = game_state.current_player().score
            - (0..)
                .zip(game_state.players.iter())
                .filter(|&(i, _)| i != game_state.turn)
                .map(|(_, player)| player.score)
                .max()
                .unwrap_or(0);
        self.num_sim_plies = num_sim_plies;
        self.num_tiles_that_matter = num_sim_plies * game_config.rack_size() as usize;
    }

    #[inline(always)]
    pub fn prepare_iteration(&mut self) {
        let initial_turn = self.initial_game_state.turn as usize;
        for (i, player) in self.initial_game_state.players.iter_mut().enumerate() {
            if i != initial_turn {
                self.final_scores[i] = player.rack.len() as i16;
                self.initial_game_state
                    .bag
                    .0
                    .extend_from_slice(&player.rack);
                player.rack.clear();
            }
        }
        self.possible_to_play_out =
            self.initial_game_state.bag.0.len() <= self.num_tiles_that_matter;
        RNG.with(|rng| {
            self.initial_game_state
                .bag
                .shuffle_n(&mut *rng.borrow_mut(), self.num_tiles_that_matter);
        });
        for (i, player) in self.initial_game_state.players.iter_mut().enumerate() {
            if i != initial_turn {
                self.initial_game_state
                    .bag
                    .replenish(&mut player.rack, self.final_scores[i] as usize);
            }
        }
    }

    // true iff played out
    #[inline(always)]
    pub fn simulate(
        &mut self,
        game_config: &game_config::GameConfig,
        kwg: &kwg::Kwg,
        klv: &klv::Klv,
        candidate_play: &movegen::Play,
    ) -> bool {
        self.game_state.clone_from(&self.initial_game_state);
        // reset leave values from previous iteration
        self.last_seen_leave_values
            .iter_mut()
            .for_each(|m| *m = 0.0);
        let mut next_play = movegen::Play::Exchange {
            tiles: [][..].into(),
        };
        for ply in 0..=self.num_sim_plies {
            next_play.clone_from(if ply == 0 {
                &candidate_play
            } else {
                self.move_generator
                    .gen_moves_unfiltered(&movegen::GenMovesParams {
                        board_snapshot: &movegen::BoardSnapshot {
                            board_tiles: &self.game_state.board_tiles,
                            game_config: &game_config,
                            kwg: &kwg,
                            klv: &klv,
                        },
                        rack: &self.game_state.current_player().rack,
                        max_gen: 1,
                        always_include_pass: false,
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
            RNG.with(|rng| {
                self.game_state
                    .play(&game_config, &mut *rng.borrow_mut(), &next_play)
                    .unwrap();
            });
            match self
                .game_state
                .check_game_ended(&game_config, &mut self.final_scores)
            {
                game_state::CheckGameEnded::NotEnded => {}
                _ => {
                    // game has ended, move leave values to actual score
                    for (i, player) in self.game_state.players.iter_mut().enumerate() {
                        player.score = self.final_scores[i];
                    }
                    self.last_seen_leave_values
                        .iter_mut()
                        .for_each(|m| *m = 0.0);
                    return true;
                }
            }
            self.game_state.next_turn();
        }
        false
    }

    #[inline(always)]
    pub fn final_equity_spread(&self) -> f32 {
        let mut best_opponent_equity = f32::NEG_INFINITY;
        for (i, player) in (0..).zip(self.game_state.players.iter()) {
            if i != self.initial_game_state.turn {
                let opponent_equity = player.score as f32 + self.last_seen_leave_values[i as usize];
                if opponent_equity > best_opponent_equity {
                    best_opponent_equity = opponent_equity;
                }
            }
        }
        let mut this_equity = self.game_state.players[self.initial_game_state.turn as usize].score
            as f32
            + self.last_seen_leave_values[self.initial_game_state.turn as usize];
        if best_opponent_equity != f32::NEG_INFINITY {
            this_equity -= best_opponent_equity;
        }
        this_equity - self.initial_score_spread as f32
    }

    #[inline(always)]
    pub fn compute_win_prob(&self, game_ended: bool, final_spread: f32) -> f64 {
        if game_ended {
            if final_spread > 0.0 {
                1.0
            } else if final_spread < 0.0 {
                0.0
            } else {
                0.5
            }
        } else {
            // handwavily: assume spread of +/- (30 + num_unseen_tiles) should be 90%/10% (-Andy Kurnia)
            let num_unseen_tiles = self.game_state.bag.0.len()
                + self
                    .game_state
                    .players
                    .iter()
                    .map(|player| player.rack.len())
                    .sum::<usize>();
            // this could be precomputed for every possible num_unseen_tiles (1 to 93)
            let exp_width = -(30.0 + num_unseen_tiles as f64) / ((1.0 / 0.9 - 1.0) as f64).ln();
            1.0 / (1.0 + (-(final_spread as f64) / exp_width).exp())
        }
    }

    #[inline(always)]
    pub fn win_prob_weightage(&self) -> f64 {
        if self.possible_to_play_out {
            1000.0
        } else {
            10.0
        }
    }
}
