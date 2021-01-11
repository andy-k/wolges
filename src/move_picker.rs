// Copyright (C) 2020-2021 Andy Kurnia. All rights reserved.

use super::{game_config, game_state, klv, kwg, move_filter, movegen, stats};
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

struct Candidate {
    play_index: usize,
    stats: stats::Stats,
}

pub struct Simmer<'a> {
    candidates: Vec<Candidate>,
    rng: Box<dyn RngCore>,
    move_generator: movegen::KurniaMoveGenerator,
    initial_game_state: game_state::GameState<'a>,
    game_state: game_state::GameState<'a>,
    kwg: &'a kwg::Kwg,
    klv: &'a klv::Klv,
    last_seen_leave_values: Box<[f32]>,
    final_scores: Box<[i16]>,
    rack_tally: Box<[u8]>,
    initial_score_spread: i16,
    possible_to_play_out: bool,
    num_sim_plies: usize,
    num_tiles_that_matter: usize,
}

impl<'a> Simmer<'a> {
    pub fn new(
        game_config: &'a game_config::GameConfig,
        kwg: &'a kwg::Kwg,
        klv: &'a klv::Klv,
    ) -> Self {
        Self {
            candidates: Vec::new(),
            rng: Box::new(rand_chacha::ChaCha20Rng::from_entropy()),
            move_generator: movegen::KurniaMoveGenerator::new(game_config),
            initial_game_state: game_state::GameState::new(game_config),
            game_state: game_state::GameState::new(game_config),
            kwg,
            klv,
            last_seen_leave_values: vec![0.0f32; game_config.num_players() as usize]
                .into_boxed_slice(),
            final_scores: vec![0; game_config.num_players() as usize].into_boxed_slice(),
            rack_tally: vec![0u8; game_config.alphabet().len() as usize].into_boxed_slice(),
            initial_score_spread: 0,
            possible_to_play_out: false,
            num_sim_plies: 0,
            num_tiles_that_matter: 0,
        }
    }

    #[inline(always)]
    fn prepare(
        &mut self,
        game_state: &game_state::GameState,
        num_plays: usize,
        num_sim_plies: usize,
    ) {
        self.candidates.clear();
        self.candidates.reserve(num_plays);
        for idx in 0..num_plays {
            self.candidates.push(Candidate {
                play_index: idx,
                stats: stats::Stats::new(),
            });
        }
        self.initial_game_state
            .clone_transient_stuffs_from(&game_state);
        self.game_state.clone_transient_stuffs_from(&game_state);
        self.initial_score_spread = game_state.current_player().score
            - (0..)
                .zip(game_state.players.iter())
                .filter(|&(i, _)| i != game_state.turn)
                .map(|(_, player)| player.score)
                .max()
                .unwrap_or(0);
        self.num_sim_plies = num_sim_plies;
        self.num_tiles_that_matter = num_sim_plies * game_state.game_config.rack_size() as usize;
    }

    #[inline(always)]
    fn prepare_iteration(&mut self) {
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
        self.initial_game_state
            .bag
            .shuffle_n(&mut self.rng, self.num_tiles_that_matter);
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
    fn simulate(&mut self, candidate_play: &movegen::Play) -> bool {
        self.game_state.clone_from(&self.initial_game_state);
        // reset leave values from previous iteration
        self.last_seen_leave_values
            .iter_mut()
            .for_each(|m| *m = 0.0);
        for ply in 0..=self.num_sim_plies {
            let next_play = if ply == 0 {
                &candidate_play
            } else {
                self.move_generator.gen_moves_unfiltered(
                    &movegen::BoardSnapshot {
                        board_tiles: &self.game_state.board_tiles,
                        game_config: &self.game_state.game_config,
                        kwg: &self.kwg,
                        klv: &self.klv,
                    },
                    &self.game_state.current_player().rack,
                    1,
                );
                &self.move_generator.plays[0].play
            };
            set_rack_tally_from_leave(
                &mut self.rack_tally,
                &self.game_state.current_player().rack,
                &next_play,
            );
            self.last_seen_leave_values[self.game_state.turn as usize] =
                self.klv.leave_value_from_tally(&self.rack_tally);
            self.game_state.play(&mut self.rng, &next_play).unwrap();
            match self.game_state.check_game_ended(&mut self.final_scores) {
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
    fn final_equity_spread(&self) -> f32 {
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
    fn compute_win_prob(&self, game_ended: bool, final_spread: f32) -> f64 {
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
    fn win_prob_weightage(&self) -> f64 {
        if self.possible_to_play_out {
            1000.0
        } else {
            10.0
        }
    }

    #[inline(always)]
    fn top_candidate_play_index_by_mean(&self) -> usize {
        self.candidates
            .iter()
            .max_by(|a, b| a.stats.mean().partial_cmp(&b.stats.mean()).unwrap())
            .unwrap()
            .play_index
    }
}

#[allow(clippy::large_enum_variant)]
pub enum MovePicker<'a> {
    Hasty,
    Simmer(Simmer<'a>),
}

impl MovePicker<'_> {
    #[inline(always)]
    pub fn pick_a_move(
        &mut self,
        filtered_movegen: &mut move_filter::GenMoves,
        mut move_generator: &mut movegen::KurniaMoveGenerator,
        board_snapshot: &movegen::BoardSnapshot,
        game_state: &game_state::GameState,
        rack: &[u8],
    ) {
        match self {
            MovePicker::Hasty => {
                filtered_movegen.gen_moves(&mut move_generator, board_snapshot, &rack, 1);
            }
            MovePicker::Simmer(simmer) => {
                let t0 = std::time::Instant::now();
                filtered_movegen.gen_moves(&mut move_generator, board_snapshot, &rack, 100);
                simmer.prepare(&game_state, move_generator.plays.len(), 2);
                let mut candidates = std::mem::take(&mut simmer.candidates);
                let num_sim_iters = 1000;
                let mut last_reported_elapsed_time_secs = 0;
                let mut last_prune_time_prune_intervals = 0;
                let max_time_for_move_ms = 15000u64;
                let orig_candidates_len = candidates.len();
                let prune_interval_ms =
                    std::cmp::max(1, max_time_for_move_ms / orig_candidates_len as u64);
                for sim_iter in 1..=num_sim_iters {
                    let elapsed_time_ms = t0.elapsed().as_millis() as u64;
                    let elapsed_time_secs = elapsed_time_ms / 1000;
                    if elapsed_time_secs != last_reported_elapsed_time_secs {
                        last_reported_elapsed_time_secs = elapsed_time_secs;
                        println!(
                            "After {} seconds, doing iteration {} with {} candidates",
                            last_reported_elapsed_time_secs,
                            sim_iter,
                            candidates.len()
                        );
                    }
                    simmer.prepare_iteration();
                    for candidate in candidates.iter_mut() {
                        let game_ended =
                            simmer.simulate(&move_generator.plays[candidate.play_index].play);
                        let final_spread = simmer.final_equity_spread();
                        let win_prob = simmer.compute_win_prob(game_ended, final_spread);
                        let sim_spread = final_spread - simmer.initial_score_spread as f32;
                        candidate
                            .stats
                            .update(sim_spread as f64 + win_prob * simmer.win_prob_weightage());
                    }
                    let elapsed_time_prune_intervals = elapsed_time_ms / prune_interval_ms;
                    if sim_iter >= 20
                        && (sim_iter % 32 == 0
                            || elapsed_time_prune_intervals != last_prune_time_prune_intervals)
                    {
                        last_prune_time_prune_intervals = elapsed_time_prune_intervals;
                        let z = 1.96; // 95% confidence interval
                        let low_bar = candidates
                            .iter()
                            .map(|candidate| candidate.stats.ci_max(-z))
                            .max_by(|a, b| a.partial_cmp(&b).unwrap())
                            .unwrap();
                        candidates.retain(|candidate| candidate.stats.ci_max(z) >= low_bar);
                        let max_candidates_allowed = 1
                            + (2 * max_time_for_move_ms.saturating_sub(elapsed_time_ms)
                                / prune_interval_ms) as usize;
                        while candidates.len() > max_candidates_allowed {
                            // they're all about the same anyway
                            candidates.swap_remove(
                                candidates
                                    .iter()
                                    .enumerate()
                                    .map(|(i, candidate)| (i, candidate.stats.ci_max(-z)))
                                    .min_by(|(_, a), (_, b)| a.partial_cmp(&b).unwrap())
                                    .unwrap()
                                    .0,
                            );
                        }
                        if candidates.len() < 2 {
                            break;
                        }
                    }
                }
                simmer.candidates = candidates;
                move_generator
                    .plays
                    .swap(0, simmer.top_candidate_play_index_by_mean());
                move_generator.plays.truncate(1);
            }
        }
    }
}
