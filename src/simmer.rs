// Copyright (C) 2020-2026 Andy Kurnia.

use super::{equity, game_config, game_state, klv, kwg, movegen, win_pct};
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

/// How the simmer estimates the win probability at an unfinished terminal
/// position. Sigmoid is the shipped default: a hand-tuned sigmoid of the score
/// margin. Table looks the position up in an empirical WinPctTable and falls
/// back to the sigmoid for any count-and-margin the table never sampled.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum WinProbSource {
    Sigmoid,
    Table,
}

/// Tunable weights for the simmer's win-probability model and objective. Default
/// is the shipped configuration. `descale: false` reproduces the pre-descale
/// units mismatch on purpose; it exists only as an A/B baseline and is never
/// shipped.
#[derive(Clone, Copy)]
pub struct SimmerConfig {
    /// Feed the spread to the sigmoid and objective in points (true) instead of
    /// raw millipoints (false, the pre-descale baseline).
    pub descale: bool,
    /// Win-probability weightage when the game cannot be played out this sim.
    pub w_no_out: f64,
    /// Win-probability weightage when the game can be played out this sim.
    pub w_out: f64,
    /// Lead (in points) at which the unfinished-game sigmoid reads sigmoid_prob.
    pub sigmoid_spread: f64,
    /// Win probability the sigmoid assigns at +sigmoid_spread points of lead.
    pub sigmoid_prob: f64,
    /// Where the unfinished-game win probability comes from. Default Sigmoid
    /// keeps the shipped config unchanged; Table opts into the empirical table.
    pub win_prob_source: WinProbSource,
}

impl Default for SimmerConfig {
    fn default() -> Self {
        Self {
            descale: true,
            w_no_out: 10.0,
            w_out: 10000.0,
            sigmoid_spread: 30.0,
            sigmoid_prob: 0.9,
            win_prob_source: WinProbSource::Sigmoid,
        }
    }
}

/// The unfinished-game win-probability sigmoid: handwavily, a lead of
/// +/- (sigmoid_spread + num_unseen_tiles) points reads as sigmoid_prob /
/// (1 - sigmoid_prob) (default 90%/10%). final_spread is in millipoints and is
/// descaled to points first, unless cfg.descale is false (the A/B baseline).
#[inline(always)]
pub fn win_prob_unfinished(final_spread: i32, num_unseen_tiles: usize, cfg: &SimmerConfig) -> f64 {
    // this could be precomputed for every possible num_unseen_tiles (1 to 93)
    let exp_width =
        -(cfg.sigmoid_spread + num_unseen_tiles as f64) / (1.0 / cfg.sigmoid_prob - 1.0).ln();
    let spread = if cfg.descale {
        spread_points(final_spread)
    } else {
        final_spread as f64
    };
    1.0 / (1.0 + (-spread / exp_width).exp())
}

/// The per-candidate objective: the sim spread (descaled to points unless the
/// A/B baseline disables descaling) plus the win probability scaled by its
/// weightage. Both callers route their per-candidate stat through this so the
/// objective is defined in exactly one place.
#[inline(always)]
pub fn sim_objective(sim_spread: i32, win_prob: f64, weightage: f64, descale: bool) -> f64 {
    let spread = if descale {
        spread_points(sim_spread)
    } else {
        sim_spread as f64
    };
    spread + win_prob * weightage
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

    // per-instance Monte-Carlo RNG (was a thread_local) + tunable config
    rng: rand::rngs::ChaCha20Rng,
    config: SimmerConfig,
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

            // system entropy by default, so existing callers keep their original
            // nondeterministic behavior until they opt into reseed().
            rng: rand::rngs::ChaCha20Rng::try_from_rng(&mut rand::rngs::SysRng).unwrap(),
            config: SimmerConfig::default(),
        }
    }

    /// Reseed the per-instance RNG for a reproducible sim stream: the same seed
    /// and config replay the same draws, independent of thread count. new()
    /// seeds from system entropy instead, so nondeterministic callers are
    /// unaffected until they opt in.
    #[inline(always)]
    pub fn reseed(&mut self, seed: u64) {
        self.rng = rand::rngs::ChaCha20Rng::seed_from_u64(seed);
    }

    #[inline(always)]
    pub fn set_config(&mut self, config: SimmerConfig) {
        self.config = config;
    }

    /// Build a fresh simmer already prepared to this one's current position and
    /// config, for a worker thread that runs its own rollouts. The ChaCha20 RNG
    /// is not Clone, so this rebuilds from the retained initial game state rather
    /// than copying field by field; the caller reseeds it before every iteration,
    /// so the new RNG's starting state does not matter. prepare() recomputes the
    /// weightage and tile counts deterministically from the position and config,
    /// so the copy matches the original. Call after prepare(), before any
    /// prepare_iteration() has advanced the original's initial state.
    pub fn prepared_clone(&self, game_config: &game_config::GameConfig) -> Self {
        let mut clone = Simmer::new(game_config);
        clone.config = self.config;
        clone.prepare(game_config, &self.initial_game_state, self.num_sim_plies);
        clone
    }

    #[inline(always)]
    pub fn config(&self) -> &SimmerConfig {
        &self.config
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
        let w_no_out = self.config.w_no_out;
        let w_out = self.config.w_out;
        self.win_prob_weightage = if num_unseen_tiles <= self.num_tiles_that_matter {
            // possible to play out
            w_out
        } else if num_unseen_tiles < 2 * self.num_tiles_that_matter {
            w_out
                + ((num_unseen_tiles - self.num_tiles_that_matter) as f64
                    / self.num_tiles_that_matter as f64)
                    * (w_no_out - w_out)
        } else {
            w_no_out
        };
    }

    /// The prepared initial game state, right after prepare() and before any
    /// prepare_iteration() has drawn from it. The parallel move picker snapshots
    /// this once per worker so it can restore it before every iteration.
    #[inline(always)]
    pub fn prepared_state(&self) -> &game_state::GameState {
        &self.initial_game_state
    }

    /// Restore the initial game state to a pristine prepared position (a snapshot
    /// from prepared_state). prepare_iteration draws destructively from the bag,
    /// so it is not a pure reset: a later iteration's draw depends on how many
    /// earlier iterations ran on this simmer. Restoring the snapshot before each
    /// iteration makes every iteration's draw depend only on its own reseed, so
    /// the parallel path is independent of how the iterations split across
    /// threads.
    #[inline(always)]
    pub fn restore_prepared(&mut self, pristine: &game_state::GameState) {
        self.initial_game_state.clone_from(pristine);
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
        let num_tiles_that_matter = self.num_tiles_that_matter;
        self.initial_game_state
            .bag
            .shuffle_n(&mut self.rng, num_tiles_that_matter);
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
        // prepare_iteration() left self.rng at the state every candidate in this
        // iteration must roll out from. A Place ply draws its replenishment tiles
        // off the front of the already-shuffled bag (no rng needed) and an empty
        // exchange (a pass) draws nothing either; only a real exchange consumes
        // randomness (returning the exchanged tiles to a random bag position). So
        // this rollout never mutates self.rng directly: the first ply that needs
        // randomness snapshots it into a stack-local ChaCha20 generator
        // (serialize_state/deserialize_state, no heap allocation) and every later
        // draw in this rollout advances that local copy instead, leaving self.rng
        // untouched for the next candidate.
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
                rand::rngs::ChaCha20Rng::deserialize_state(&self.rng.serialize_state())
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
    pub fn compute_win_prob(
        &self,
        game_ended: bool,
        final_spread: i32,
        table: Option<&win_pct::WinPctTable>,
    ) -> f64 {
        if game_ended {
            match final_spread.signum() {
                1 => 1.0,
                -1 => 0.0,
                _ => 0.5,
            }
        } else {
            let bag = self.game_state.bag.len();
            let racks_total = self
                .game_state
                .players
                .iter()
                .map(|player| player.rack.len())
                .sum::<usize>();
            // Where the empirical table is selected and has data for this
            // count-state and margin, use it; otherwise fall through to the
            // sigmoid. The default (Sigmoid, or no table) runs neither branch
            // and is byte-identical to the pre-table path.
            if self.config.win_prob_source == WinProbSource::Table
                && let Some(table) = table
            {
                let my = self.game_state.players[self.initial_game_state.turn as usize]
                    .rack
                    .len();
                let opp = racks_total - my;
                if let Some(win_prob) =
                    table.get_opt(equity::descale_score(final_spread), bag, my, opp)
                {
                    return win_prob as f64;
                }
            }
            win_prob_unfinished(final_spread, bag + racks_total, &self.config)
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

    // A rollout that exchanges tiles must draw from a copy of self.rng, not
    // self.rng itself, so the shared post-prepare_iteration state stays put for
    // the next candidate (common random numbers: every candidate in the
    // iteration rolls out from the same randomness). This drives one
    // candidate's rollout through an exchange and checks self.rng is unchanged
    // afterwards, then repeats the same rollout from that unchanged state and
    // checks it reproduces exactly.
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
        simmer.reseed(99);
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
        let shared_state = simmer.rng.serialize_state();

        simmer.simulate(&game_config, &kwg, &klv, &candidate);
        let rack_after_first = simmer.game_state.players[simmer.initial_game_state.turn as usize]
            .rack
            .clone();
        // the exchange drew from a copy, so self.rng is untouched.
        assert_eq!(simmer.rng.serialize_state(), shared_state);

        // a second rollout of the same candidate, from the same untouched
        // shared state, reproduces the first exactly: identical randomness per
        // candidate.
        simmer.simulate(&game_config, &kwg, &klv, &candidate);
        let rack_after_second = simmer.game_state.players[simmer.initial_game_state.turn as usize]
            .rack
            .clone();
        assert_eq!(simmer.rng.serialize_state(), shared_state);
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

    #[test]
    fn win_prob_unfinished_hits_sigmoid_prob_at_the_crafted_lead() {
        // by construction a lead of (sigmoid_spread + num_unseen) points reads as
        // sigmoid_prob, and its negation as 1 - sigmoid_prob. num_unseen = 10 ->
        // a 40-point lead = 40000 millipoints.
        let cfg = SimmerConfig::default();
        let lead_points = cfg.sigmoid_spread + 10.0; // 40.0
        let lead_millipoints = (lead_points * equity::SCALE as f64) as i32;
        assert!((win_prob_unfinished(lead_millipoints, 10, &cfg) - cfg.sigmoid_prob).abs() < 1e-9);
        assert!(
            (win_prob_unfinished(-lead_millipoints, 10, &cfg) - (1.0 - cfg.sigmoid_prob)).abs()
                < 1e-9
        );
        assert_eq!(win_prob_unfinished(0, 10, &cfg), 0.5);

        // descale: false feeds raw millipoints, so the same numeric input reads as
        // a 1000x larger lead and saturates far past sigmoid_prob.
        let raw = SimmerConfig {
            descale: false,
            ..cfg
        };
        assert!(win_prob_unfinished(lead_millipoints, 10, &raw) > 0.999);
    }

    #[test]
    fn sim_objective_descales_spread_and_scales_win_prob() {
        // descale true: spread reads in points and the win term adds weightage.
        assert_eq!(sim_objective(30 * equity::SCALE, 0.0, 10.0, true), 30.0);
        assert_eq!(sim_objective(0, 1.0, 10.0, true), 10.0);
        assert_eq!(sim_objective(0, 1.0, 10000.0, true), 10000.0);
        assert_eq!(sim_objective(0, 0.5, 10.0, true), 5.0);
        // descale false: spread stays in raw millipoints (the A/B baseline).
        assert_eq!(sim_objective(30 * equity::SCALE, 0.0, 10.0, false), 30000.0);
    }

    #[test]
    fn per_instance_rng_reseed_is_deterministic() {
        let game_config = game_config::make_english_game_config();
        let mut game_state = game_state::GameState::new(&game_config);
        // deal a full random position so prepare_iteration has tiles to draw.
        let mut deal_rng = rand::rngs::ChaCha20Rng::seed_from_u64(1);
        game_state.reset_and_draw_tiles(&game_config, &mut deal_rng);

        // draw the opponent (player 1) rack that prepare_iteration replenishes,
        // from a fresh Simmer prepared on the same position and reseeded to `seed`.
        let opponent_draw = |seed: u64| -> Vec<u8> {
            let mut simmer = Simmer::new(&game_config);
            simmer.prepare(&game_config, &game_state, 2);
            simmer.reseed(seed);
            simmer.prepare_iteration();
            // turn is 0, so player 1 is the opponent redrawn from the shuffled bag.
            simmer.initial_game_state.players[1].rack.clone()
        };

        // two independent instances, same seed -> identical (and reproducible) draw.
        assert_eq!(opponent_draw(777), opponent_draw(777));
        // a different seed almost surely draws a different rack.
        assert_ne!(opponent_draw(777), opponent_draw(778));
    }

    // Prepare a simmer on a freshly dealt english position and report its
    // (bag, my, opp) count-state so a test table can be keyed to it exactly.
    fn prepared_simmer() -> (Simmer, usize, usize, usize) {
        let game_config = game_config::make_english_game_config();
        let mut game_state = game_state::GameState::new(&game_config);
        let mut deal_rng = rand::rngs::ChaCha20Rng::seed_from_u64(1);
        game_state.reset_and_draw_tiles(&game_config, &mut deal_rng);
        let mut simmer = Simmer::new(&game_config);
        simmer.prepare(&game_config, &game_state, 2);
        let bag = simmer.game_state.bag.len();
        let turn = simmer.initial_game_state.turn as usize;
        let my = simmer.game_state.players[turn].rack.len();
        let total: usize = simmer
            .game_state
            .players
            .iter()
            .map(|player| player.rack.len())
            .sum();
        (simmer, bag, my, total - my)
    }

    // With WinProbSource::Table, an unfinished terminal whose count-state the
    // table sampled reads the table value, and one it never sampled falls back
    // to the sigmoid.
    #[test]
    fn table_source_uses_table_where_sampled_else_sigmoid() {
        let (mut simmer, bag, my, opp) = prepared_simmer();
        let cfg = SimmerConfig {
            win_prob_source: WinProbSource::Table,
            ..SimmerConfig::default()
        };
        simmer.set_config(cfg);

        // a +40-point lead in millipoints; the sigmoid reads well under 1.0 here.
        let final_spread = 40 * equity::SCALE;
        let sigmoid = win_prob_unfinished(final_spread, bag + my + opp, &cfg);
        assert!(sigmoid < 0.99, "sigmoid {sigmoid} unexpectedly saturated");

        // a table keyed exactly to this state, with swings so tight that a
        // +40 lead is a certain win (past the observed range) -> 1.0 != sigmoid.
        let mut acc = win_pct::WinPctAccumulator::new();
        for &v in &[-5, 5] {
            acc.record(bag, my, opp, 0, v);
        }
        let table = acc.finalize();
        let got = simmer.compute_win_prob(false, final_spread, Some(&table));
        assert_eq!(got, 1.0, "table value should be used at the sampled key");

        // a table that only has a different key leaves this state to the sigmoid.
        let mut acc_other = win_pct::WinPctAccumulator::new();
        for &v in &[-5, 5] {
            acc_other.record(bag + 1, my, opp, 0, v);
        }
        let table_other = acc_other.finalize();
        let fell_back = simmer.compute_win_prob(false, final_spread, Some(&table_other));
        assert_eq!(
            fell_back, sigmoid,
            "absent key should fall back to the sigmoid"
        );
    }

    // WinProbSource::Sigmoid (the default) ignores a provided table entirely.
    #[test]
    fn sigmoid_source_ignores_table() {
        let (mut simmer, bag, my, opp) = prepared_simmer();
        let cfg = SimmerConfig::default();
        simmer.set_config(cfg);
        let final_spread = 40 * equity::SCALE;

        // a table that WOULD return 1.0 at this key if consulted.
        let mut acc = win_pct::WinPctAccumulator::new();
        for &v in &[-5, 5] {
            acc.record(bag, my, opp, 0, v);
        }
        let table = acc.finalize();
        let got = simmer.compute_win_prob(false, final_spread, Some(&table));
        assert_eq!(got, win_prob_unfinished(final_spread, bag + my + opp, &cfg));
    }
}
