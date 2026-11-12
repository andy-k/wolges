// Copyright (C) 2020-2026 Andy Kurnia.

use super::{
    anagrams, equity, error, game_config, game_state, infer, klv, kwg, movegen, win_pct, zone,
};
use rand::prelude::*;

// scores and leave values are millipoints; the sigmoid and its weightage are in
// whole points, so a spread passes through here before either reads it.
#[inline(always)]
pub fn spread_points(millipoints: i32) -> f64 {
    millipoints as f64 / equity::SCALE as f64
}

#[inline(always)]
pub fn win_prob_unfinished(final_spread: i32, num_unseen_tiles: usize) -> f64 {
    // handwavily: assume spread of +/- (30 + num_unseen_tiles) should be 90%/10% (-Andy Kurnia)
    // (to adjust these, adjust the 30.0 and 0.9 consts below)
    let exp_width = -(30.0 + num_unseen_tiles as f64) / (1.0 / 0.9 - 1.0f64).ln();
    1.0 / (1.0 + (-spread_points(final_spread) / exp_width).exp())
}

// the per-candidate objective: the sim spread in points, plus the win probability
// times its weightage.
#[inline(always)]
pub fn sim_objective(sim_spread: i32, win_prob: f64, weightage: f64) -> f64 {
    spread_points(sim_spread) + win_prob * weightage
}

#[inline(always)]
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

// a generator whose state can be copied, so each candidate's rollout draws what
// the shared generator would have drawn. chacha's generator is not Clone.
pub trait CopyState {
    fn copy_state(&self) -> Self;
}

impl CopyState for rand::rngs::ChaCha20Rng {
    #[inline(always)]
    fn copy_state(&self) -> Self {
        Self::deserialize_state(&self.serialize_state())
    }
}

impl CopyState for rand_xoshiro::Xoshiro256PlusPlus {
    #[inline(always)]
    fn copy_state(&self) -> Self {
        self.clone()
    }
}

// the generator a simmer draws with unless it is given another.
pub type DefaultRng = rand_xoshiro::Xoshiro256PlusPlus;

// a ply a rollout played after its candidate: the play's score, in millipoints,
// and how many tiles it placed (none for an exchange or a pass)
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PlyPlayed {
    pub score: i32,
    pub tiles: u8,
}

// the leaves a seat may have kept, packed with their weights and shared (as an
// inference's are by every sim given them), so that a weighted draw is a binary
// search
#[derive(Clone)]
struct KeptLeaves {
    seat: u8,
    leaves: std::sync::Arc<infer::PackedLeaves>,
}

// Simmer can only be reused for the same game_config and kwg.
// (Refer to note at KurniaMoveGenerator.)
// This is not enforced.
pub struct Simmer<
    R = DefaultRng,
    G: movegen::LaneBits = u128,
    const JUMPS: bool = true,
    const ANY_SIDE: bool = true,
> {
    initial_game_state: game_state::GameState,
    pub initial_score_spread: i32,
    num_sim_plies: usize,
    num_tiles_that_matter: usize,
    win_prob_weightage: f64,
    wants_win_prob: bool,
    opponents_pass: bool, // every seat but the mover's passes in a rollout

    game_state: game_state::GameState,
    last_seen_leave_values: Box<[i32]>,
    final_scores: Box<[i32]>,
    plies_played: Vec<PlyPlayed>, // the last rollout's, after its candidate
    reply_equity: Option<equity::Equity>, // the last rollout's ply-1 reply, as reply_to values it

    move_generator: movegen::KurniaMoveGenerator<G, JUMPS, ANY_SIDE>,
    start_board: movegen::BoardState, // the generator's there, once it has been
    rack_tally: Box<[u8]>,

    kept: Vec<KeptLeaves>, // a seat's each, the mover's never

    lane_bests: Vec<(movegen::ValuedMove, bool, i8)>, // the opponent's replies on the start board, best first, each with its lane
    lane_rows_taken: u128,                            // the lanes those replies lie on
    lane_cols_taken: u128,
    lanes_exhausted: bool, // whether the lanes left hold no play

    census: std::sync::Arc<SharedCensus>, // the preparation's, its clones' too
    census_taken: Option<TakenCensus>,    // this simmer's, once taken

    rng: R,
}

// the most tiles off the start board, blanks aside, a census is taken over
const CENSUS_MAX_POOL: usize = 32;

// the rollouts of a preparation that run before its census is taken: the census
// pays for itself only over more than about as many
const CENSUS_AFTER: u64 = 1024;

// a preparation's census (take_census), shared by the simmer and its clones,
// and the rollouts they have run of it while it is not taken; each keeps its
// own hold of it once it has it, so the lock is taken only to find it
#[derive(Default)]
struct SharedCensus {
    rollouts: std::sync::atomic::AtomicU64,
    taken: std::sync::Mutex<Option<TakenCensus>>,
}

// a taken census, or none where the start board has none
type TakenCensus = std::sync::Arc<Option<movegen::LaneCensus>>;

impl Simmer {
    // The other methods must be called with the same game_config.
    pub fn new(game_config: &game_config::GameConfig) -> error::Returns<Self> {
        Ok(Self::with_rng(
            game_config,
            DefaultRng::try_from_rng(&mut rand::rngs::SysRng)?,
        ))
    }
}

impl<R: rand::Rng + SeedableRng + CopyState> Simmer<R> {
    pub fn with_rng(game_config: &game_config::GameConfig, rng: R) -> Self {
        Self::with_rng_and_generator(
            game_config,
            rng,
            movegen::KurniaMoveGenerator::new(game_config),
        )
    }
}

impl<G: movegen::LaneBits, const JUMPS: bool, const ANY_SIDE: bool>
    Simmer<DefaultRng, G, JUMPS, ANY_SIDE>
{
    // as new, generating by the caller's move generator (the wasm's, in u32)
    pub fn with_generator(
        game_config: &game_config::GameConfig,
        move_generator: movegen::KurniaMoveGenerator<G, JUMPS, ANY_SIDE>,
    ) -> error::Returns<Self> {
        Ok(Self::with_rng_and_generator(
            game_config,
            DefaultRng::try_from_rng(&mut rand::rngs::SysRng)?,
            move_generator,
        ))
    }
}

impl<
    R: rand::Rng + SeedableRng + CopyState,
    G: movegen::LaneBits,
    const JUMPS: bool,
    const ANY_SIDE: bool,
> Simmer<R, G, JUMPS, ANY_SIDE>
{
    fn with_rng_and_generator(
        game_config: &game_config::GameConfig,
        rng: R,
        move_generator: movegen::KurniaMoveGenerator<G, JUMPS, ANY_SIDE>,
    ) -> Self {
        Self {
            initial_game_state: game_state::GameState::new(game_config),
            initial_score_spread: 0,
            num_sim_plies: 0,
            num_tiles_that_matter: 0,
            win_prob_weightage: 0.0,
            wants_win_prob: false,
            opponents_pass: false,

            game_state: game_state::GameState::new(game_config),
            last_seen_leave_values: vec![0i32; game_config.num_players() as usize]
                .into_boxed_slice(),
            final_scores: vec![0; game_config.num_players() as usize].into_boxed_slice(),
            plies_played: Vec::new(),
            reply_equity: None,

            move_generator,
            start_board: movegen::BoardState::default(),
            rack_tally: vec![0u8; game_config.alphabet().len() as usize].into_boxed_slice(),

            kept: Vec::new(),

            lane_bests: Vec::new(),
            lane_rows_taken: 0,
            lane_cols_taken: 0,
            lanes_exhausted: false,

            census: Default::default(),
            census_taken: None,

            rng,
        }
    }

    #[inline(always)]
    pub fn reseed(&mut self, seed: u64) {
        self.rng = R::seed_from_u64(seed);
    }

    // before the rollouts walk a graph other than the one they last walked
    #[inline(always)]
    pub fn reset_for_another_kwg(&mut self) {
        self.move_generator.reset_for_another_kwg();
    }

    // the clone's generator starts fixed: reseed it before it draws.
    #[inline(always)]
    pub fn prepared_clone(&self, game_config: &game_config::GameConfig) -> Self {
        let mut clone = Self::with_rng_and_generator(
            game_config,
            R::seed_from_u64(0),
            movegen::KurniaMoveGenerator::new_in(game_config),
        );
        clone.prepare(
            game_config,
            &self.initial_game_state,
            self.num_sim_plies,
            self.wants_win_prob,
            &[],
        );
        clone.kept.clone_from(&self.kept);
        clone.opponents_pass = self.opponents_pass;
        // one census a preparation, whichever of its clones takes it
        clone.census = self.census.clone();
        clone.census_taken.clone_from(&self.census_taken);
        clone
    }

    // the leaves a seat other than the mover's may have kept, with weights, one
    // of them dealt to it first each iteration, the rest of its rack from the
    // bag; in place of any set for that seat before. prepare sets the next
    // seat's (the opponent's in a game of two), clearing the others'; so set
    // any other seat's after it
    #[inline]
    pub fn set_kept_leaves(&mut self, seat: u8, kept_leaves: &[(Vec<u8>, f64)]) {
        let mut leaves = infer::PackedLeaves::new();
        for (leave, weight) in kept_leaves {
            if *weight > 0.0 {
                // an infinite weight would leave no total to draw against
                (leaves.push(leave, *weight)).expect("a kept leave's weight is finite");
            }
        }
        self.set_packed_kept_leaves(seat, std::sync::Arc::new(leaves));
    }

    // as set_kept_leaves, the leaves packed and shared with whatever else holds
    // them (a list of millions, as an inference's may be, is not copied); a
    // leave of weight 0 is never dealt
    #[inline]
    pub fn set_packed_kept_leaves(
        &mut self,
        seat: u8,
        leaves: std::sync::Arc<infer::PackedLeaves>,
    ) {
        self.kept.retain(|kept| kept.seat != seat);
        if leaves.total() > 0.0 {
            self.kept.push(KeptLeaves { seat, leaves });
            self.kept.sort_by_key(|kept| kept.seat);
        }
    }

    // whether every seat but the mover's passes on each of its turns in a
    // rollout: a play valued as if the opponent could make nothing of what it
    // leaves. Off unless set; prepare leaves it as it is.
    #[inline(always)]
    pub fn set_opponents_pass(&mut self, opponents_pass: bool) {
        self.opponents_pass = opponents_pass;
    }

    #[inline(always)]
    pub fn prepare(
        &mut self,
        game_config: &game_config::GameConfig,
        game_state: &game_state::GameState,
        num_sim_plies: usize,
        report_win_prob: bool,
        kept_leaves: &[(Vec<u8>, f64)], // the leaves the next seat may have kept, with weights
    ) {
        self.kept.clear();
        let next_seat = (game_state.turn + 1) % game_state.players.len() as u8;
        self.set_kept_leaves(next_seat, kept_leaves);
        self.initial_game_state.clone_from(game_state);
        self.game_state.clone_from(game_state);
        self.initial_score_spread = game_state.current_player().score
            - (0..)
                .zip(game_state.players.iter())
                .filter(|&(i, _)| i != game_state.turn)
                .map(|(_, player)| player.score)
                .max()
                .unwrap_or(0);
        self.wants_win_prob = report_win_prob;
        self.set_num_sim_plies(game_config, num_sim_plies);
        self.census = Default::default();
        self.census_taken = None;
    }

    // the start board's census over its tiles off it but the blanks, taken once
    // a preparation by the default arm's rollout that comes after CENSUS_AFTER
    // of them (of this simmer or of any clone prepared_clone made from it,
    // which share it): in a classic game where they number at most
    // CENSUS_MAX_POOL, when the arm has its tables. The later rollouts' plies,
    // and their replies at ply 1 on the start board, read from it each lane's
    // best equity for the rack, and need not generate those that cannot win.
    fn take_census<N: kwg::Node, L: kwg::Node>(
        &mut self,
        game_config: &game_config::GameConfig,
        kwg: &kwg::Kwg<N>,
        arm_tables: &anagrams::ArmTables,
        klv: &klv::Klv<L>,
    ) -> Option<movegen::LaneCensus> {
        let anagrams = arm_tables.anagrams()?;
        if !matches!(game_config.game_rules(), game_config::GameRules::Classic) {
            return None;
        }
        let alphabet = game_config.alphabet();
        let mut off_board = (0..alphabet.len())
            .map(|tile| alphabet.freq(tile))
            .collect::<Vec<_>>();
        for &tile in self.initial_game_state.board_tiles.iter() {
            if tile != 0 {
                let tile = if tile & 0x80 != 0 { 0 } else { tile };
                off_board[tile as usize] = off_board[tile as usize].saturating_sub(1);
            }
        }
        let num_off_board = off_board[1..].iter().map(|&n| n as usize).sum::<usize>();
        if num_off_board == 0 || num_off_board > CENSUS_MAX_POOL {
            return None;
        }
        // no play places more of a tile than a rack holds
        let rack_size = game_config.rack_size();
        let pool = (1..alphabet.len())
            .flat_map(|tile| {
                std::iter::repeat_n(tile, off_board[tile as usize].min(rack_size) as usize)
            })
            .collect::<Vec<_>>();
        let board_snapshot = movegen::BoardSnapshot {
            board_tiles: &self.initial_game_state.board_tiles,
            game_config,
            kwg,
            anagrams: arm_tables.anagrams(),
            rack_lengths: arm_tables.rack_lengths(),
            klv,
        };
        Some(
            self.move_generator
                .lane_census(&board_snapshot, anagrams, &pool),
        )
    }

    // the plies each rollout plays past its candidate from the next one on
    // (past them it plays on while the bag is empty), with the weight of the
    // chance to win, which reads them; a prepared sim's own otherwise
    #[inline(always)]
    pub fn set_num_sim_plies(
        &mut self,
        game_config: &game_config::GameConfig,
        num_sim_plies: usize,
    ) {
        self.num_sim_plies = num_sim_plies;
        self.num_tiles_that_matter = num_sim_plies.saturating_mul(game_config.rack_size() as usize);
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
        self.wants_win_prob |= self.win_prob_weightage != 0.0;
    }

    #[inline(always)]
    pub fn prepared_state(&self) -> &game_state::GameState {
        &self.initial_game_state
    }

    #[inline(always)]
    pub fn restore_prepared(&mut self, pristine: &game_state::GameState) {
        self.initial_game_state.clone_from(pristine);
    }

    #[inline(always)]
    pub fn prepare_iteration(&mut self) {
        // the opponent's rack is drawn anew, so its replies are found anew
        self.lane_bests.clear();
        self.lane_rows_taken = 0;
        self.lane_cols_taken = 0;
        self.lanes_exhausted = false;
        let initial_turn = self.initial_game_state.turn as usize;
        for (i, player) in self.initial_game_state.players.iter_mut().enumerate() {
            if i != initial_turn {
                self.final_scores[i] = player.rack.len() as i32;
                self.initial_game_state.bag.return_tiles(&player.rack);
                player.rack.clear();
            }
        }
        // put each seat's kept leave back on its rack first, seat by seat,
        // taking it out of the bag before the shuffle: taking particular copies
        // out of a shuffled bag would change the odds of what the rest deals
        for kept in &self.kept {
            let seat = kept.seat as usize;
            if seat == initial_turn || seat >= self.final_scores.len() {
                continue;
            }
            // no leave to put back from a list of none, or of weight 0 in all
            let total = kept.leaves.total();
            if total <= 0.0 {
                continue;
            }
            let at = self.rng.random_range(0.0..total);
            let rack_len = self.final_scores[seat] as usize;
            let game_state = &mut self.initial_game_state;
            for &tile in kept.leaves.pick(at).iter().take(rack_len) {
                if game_state.bag.remove_tile(tile).is_some() {
                    game_state.players[seat].rack.push(tile);
                }
            }
        }
        // a player at an odd seat draws from the bag's front, at an even seat
        // from its back, so the whole bag is shuffled, not only one end
        self.initial_game_state.bag.shuffle(&mut self.rng);
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
        arm_tables: &anagrams::ArmTables,
        klv: &klv::Klv<L>,
        candidate_play: &movegen::Play,
    ) -> error::Returns<bool> {
        self.simulate_impl::<false, _, _>(game_config, kwg, arm_tables, klv, candidate_play, false)
    }

    // as simulate, its moves found by the move generator's lean arm, so that a
    // build that runs only that arm (the wasm) need not carry the other
    #[inline(always)]
    pub fn simulate_lean<N: kwg::Node, L: kwg::Node>(
        &mut self,
        game_config: &game_config::GameConfig,
        kwg: &kwg::Kwg<N>,
        arm_tables: &anagrams::ArmTables,
        klv: &klv::Klv<L>,
        candidate_play: &movegen::Play,
    ) -> error::Returns<bool> {
        self.simulate_impl::<true, _, _>(game_config, kwg, arm_tables, klv, candidate_play, false)
    }

    // as simulate, its candidate a play challenged off the board: the tiles go
    // back on the rack; it scores nothing; its turn counts as a scoreless one,
    // a pass as well where the config says (GameState::challenged_off). The
    // board stays as it was, so the rest of the rollout goes as a pass's does.
    #[inline(always)]
    pub fn simulate_challenged_off<N: kwg::Node, L: kwg::Node>(
        &mut self,
        game_config: &game_config::GameConfig,
        kwg: &kwg::Kwg<N>,
        arm_tables: &anagrams::ArmTables,
        klv: &klv::Klv<L>,
    ) -> error::Returns<bool> {
        let pass = movegen::Play::Exchange {
            tiles: [][..].into(),
        };
        self.simulate_impl::<false, _, _>(game_config, kwg, arm_tables, klv, &pass, true)
    }

    // as simulate_challenged_off, by the move generator's lean arm
    #[inline(always)]
    pub fn simulate_lean_challenged_off<N: kwg::Node, L: kwg::Node>(
        &mut self,
        game_config: &game_config::GameConfig,
        kwg: &kwg::Kwg<N>,
        arm_tables: &anagrams::ArmTables,
        klv: &klv::Klv<L>,
    ) -> error::Returns<bool> {
        let pass = movegen::Play::Exchange {
            tiles: [][..].into(),
        };
        self.simulate_impl::<true, _, _>(game_config, kwg, arm_tables, klv, &pass, true)
    }

    #[inline(always)]
    fn simulate_impl<const LEAN: bool, N: kwg::Node, L: kwg::Node>(
        &mut self,
        game_config: &game_config::GameConfig,
        kwg: &kwg::Kwg<N>,
        arm_tables: &anagrams::ArmTables,
        klv: &klv::Klv<L>,
        candidate_play: &movegen::Play,
        challenged_off: bool,
    ) -> error::Returns<bool> {
        self.game_state.clone_from(&self.initial_game_state);
        // the generator starts from its state on the start board, kept the
        // first time it generated there, so it brings up only the lanes this
        // rollout changes rather than also undoing the last rollout's
        if self.start_board.is_of(&self.initial_game_state.board_tiles) {
            self.move_generator.restore_board_state(&self.start_board);
        }
        // reset leave values from previous iteration
        self.last_seen_leave_values.iter_mut().for_each(|m| *m = 0);
        self.plies_played.clear();
        self.reply_equity = None;
        // a rollout that needs randomness snapshots the rng into a local copy, so the
        // shared state stays put for the next candidate.
        let mut rollout_rng: Option<R> = None;
        let mut next_play = movegen::Play::Exchange {
            tiles: [][..].into(),
        };
        // once the bag is empty, keep playing past the horizon: with nothing
        // left to draw, the rollout plays out the endgame with the generator's
        // endgame moves until the game ends. Once each seat the generator moves
        // has had to pass in a row since tiles last moved, nothing can change;
        // where passes can then never end the game under its rules, the rollout
        // stops there whatever its horizon.
        let seats_moving = if self.opponents_pass {
            1
        } else {
            game_config.num_players() as usize
        };
        let mut forced_passes = 0;
        let mut ply = 0;
        if !LEAN
            && self.census_taken.is_none()
            && self
                .census
                .rollouts
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed)
                >= CENSUS_AFTER
        {
            let shared = self.census.clone();
            let mut taken = shared
                .taken
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if taken.is_none() {
                *taken = Some(std::sync::Arc::new(self.take_census(
                    game_config,
                    kwg,
                    arm_tables,
                    klv,
                )));
            }
            self.census_taken.clone_from(&taken);
        }
        let held = self.census_taken.clone();
        let census = held.as_deref().and_then(Option::as_ref);
        // the lanes this rollout's plays have touched since the start board
        let mut touched = (0u128, 0u128);
        while (ply <= self.num_sim_plies || self.game_state.bag.is_empty())
            && (forced_passes < seats_moving
                || game_config.num_passes_to_end() != 0
                || (game_config.num_zeros_to_end() != 0
                    && (game_config.zeros_can_end_empty_board()
                        || self.game_state.board_tiles.iter().any(|&tile| tile != 0))))
        {
            // whether the generator picks this ply's move: not the candidate's,
            // nor a pass the opponents make by choice
            let generated = ply > 0
                && !(self.opponents_pass && self.game_state.turn != self.initial_game_state.turn);
            if ply == 0 {
                next_play.clone_from(candidate_play);
            } else if self.opponents_pass && self.game_state.turn != self.initial_game_state.turn {
                next_play = movegen::Play::Exchange {
                    tiles: [][..].into(),
                };
            } else if ply == 1 && infer::bag_size(&self.game_state.board_tiles, game_config) > 0 {
                let reply = self.reply_at_ply_one::<LEAN, _, _>(
                    game_config,
                    kwg,
                    arm_tables,
                    klv,
                    candidate_play,
                    census,
                );
                self.reply_equity = Some(reply.equity);
                next_play = reply.play;
            } else {
                let params = &movegen::GenMovesParams {
                    board_snapshot: &movegen::BoardSnapshot {
                        board_tiles: &self.game_state.board_tiles,
                        game_config,
                        kwg,
                        anagrams: arm_tables.anagrams(),
                        rack_lengths: arm_tables.rack_lengths(),
                        klv,
                    },
                    rack: &self.game_state.current_player().rack,
                    max_gen: 1,
                    num_exchanges_by_this_player: self.game_state.current_player().num_exchanges,
                    pass_policy: movegen::PassPolicy::OnlyWhenForced,
                    dynamic_leaves: None,
                };
                match census {
                    Some(census) => self.move_generator.gen_moves_unfiltered_lanes_steered(
                        params,
                        !0,
                        !0,
                        &movegen::Steer {
                            census,
                            touched_rows: touched.0,
                            touched_cols: touched.1,
                        },
                    ),
                    None => {
                        if LEAN {
                            self.move_generator.gen_moves_unfiltered_lean(params);
                        } else {
                            self.move_generator.gen_moves_unfiltered(params);
                        }
                    }
                }
                next_play.clone_from(&self.move_generator.plays[0].play);
            }
            if census.is_some() {
                let dim = game_config.board_layout().dim();
                let footprint = zone::footprint(dim, &self.game_state.board_tiles, &next_play);
                let (rows, cols) = zone::zone_lanes(dim, &footprint.zone);
                touched.0 |= rows;
                touched.1 |= cols;
            }
            if ply > 0 {
                self.plies_played.push(match &next_play {
                    movegen::Play::Exchange { .. } => PlyPlayed { score: 0, tiles: 0 },
                    movegen::Play::Place { word, score, .. } => PlyPlayed {
                        score: *score,
                        tiles: word.iter().filter(|&&tile| tile != 0).count() as u8,
                    },
                });
            }
            set_rack_tally_from_leave(
                &mut self.rack_tally,
                &self.game_state.current_player().rack,
                &next_play,
            );
            self.last_seen_leave_values[self.game_state.turn as usize] =
                klv.leave_value_from_tally(&self.rack_tally);
            let rng = rollout_rng.get_or_insert_with(|| self.rng.copy_state());
            if ply == 0 && challenged_off {
                self.game_state.challenged_off(game_config);
            } else {
                self.game_state.play(game_config, rng, &next_play)?;
            }
            match &next_play {
                movegen::Play::Exchange { tiles } if tiles.is_empty() => {
                    if generated {
                        forced_passes += 1;
                    }
                }
                _ => forced_passes = 0,
            }
            match self
                .game_state
                .check_game_ended(game_config, &mut self.final_scores)?
            {
                game_state::CheckGameEnded::NotEnded => {}
                _ => {
                    // game has ended, move leave values to actual score
                    for (i, player) in self.game_state.players.iter_mut().enumerate() {
                        player.score = self.final_scores[i];
                    }
                    self.last_seen_leave_values.iter_mut().for_each(|m| *m = 0);
                    return Ok(true);
                }
            }
            self.game_state.next_turn();
            ply += 1;
        }
        // if the rollout stops before the game ends while the bag is empty,
        // each player keeps their rack to the end with nothing to draw: value
        // it as the generator values a rack it keeps, not by the leave table
        if self.game_state.bag.is_empty() {
            let alphabet = game_config.alphabet();
            for (value, player) in self
                .last_seen_leave_values
                .iter_mut()
                .zip(self.game_state.players.iter())
            {
                *value =
                    -equity::ENDGAME_PENALTY_BASE - 2 * alphabet.scaled_rack_score(&player.rack);
            }
        }
        Ok(false)
    }

    // the reply at ply 1, the opponent's move on the board after the candidate,
    // while the bag holds tiles. The candidate changes only the lanes its zone
    // touches: on every other lane the opponent's plays are those on the start
    // board, found once an iteration lane by lane, best first, as far as a
    // candidate needs them (exchanges apart, a forced pass ending them); the
    // lanes the zone touches are generated on the board as it is, with the
    // exchanges, and a forced pass when nothing else. The better of the two by
    // equity, then by the play's own order, is what a whole generation keeps,
    // since of equal equities it keeps the smaller play whichever it finds
    // first; a forced pass yields to any play.
    #[inline]
    fn reply_at_ply_one<const LEAN: bool, N: kwg::Node, L: kwg::Node>(
        &mut self,
        game_config: &game_config::GameConfig,
        kwg: &kwg::Kwg<N>,
        arm_tables: &anagrams::ArmTables,
        klv: &klv::Klv<L>,
        candidate_play: &movegen::Play,
        census: Option<&movegen::LaneCensus>,
    ) -> movegen::ValuedMove {
        let dim = game_config.board_layout().dim();
        let (rows, cols) = zone::zone_lanes(
            dim,
            &zone::footprint(dim, &self.initial_game_state.board_tiles, candidate_play).zone,
        );
        let on = |down: bool, lane: i8| ((if down { cols } else { rows }) >> lane) & 1 != 0;
        let mut off = self
            .lane_bests
            .iter()
            .position(|&(_, down, lane)| !on(down, lane));
        while off.is_none() && !self.lanes_exhausted {
            let params = &movegen::GenMovesParams {
                board_snapshot: &movegen::BoardSnapshot {
                    board_tiles: &self.initial_game_state.board_tiles,
                    game_config,
                    kwg,
                    anagrams: arm_tables.anagrams(),
                    rack_lengths: arm_tables.rack_lengths(),
                    klv,
                },
                rack: &self.game_state.players[self.game_state.turn as usize].rack,
                max_gen: 1,
                num_exchanges_by_this_player: i16::MAX,
                pass_policy: movegen::PassPolicy::OnlyWhenForced,
                dynamic_leaves: None,
            };
            let (rows, cols) = (!self.lane_rows_taken, !self.lane_cols_taken);
            match census {
                Some(census) => self.move_generator.gen_moves_unfiltered_lanes_steered(
                    params,
                    rows,
                    cols,
                    &movegen::Steer {
                        census,
                        touched_rows: 0,
                        touched_cols: 0,
                    },
                ),
                None => {
                    if LEAN {
                        (self.move_generator).gen_moves_unfiltered_lanes_lean(params, rows, cols);
                    } else {
                        (self.move_generator).gen_moves_unfiltered_lanes(params, rows, cols);
                    }
                }
            }
            if !self.start_board.is_of(&self.initial_game_state.board_tiles) {
                self.move_generator.save_board_state(&mut self.start_board);
            }
            match self.move_generator.plays[0].play {
                movegen::Play::Place { down, lane, .. } => {
                    if down {
                        self.lane_cols_taken |= 1 << lane;
                    } else {
                        self.lane_rows_taken |= 1 << lane;
                    }
                    self.lane_bests
                        .push((self.move_generator.plays[0].clone(), down, lane));
                    if !on(down, lane) {
                        off = Some(self.lane_bests.len() - 1);
                    }
                }
                movegen::Play::Exchange { .. } => self.lanes_exhausted = true,
            }
        }
        let params = &movegen::GenMovesParams {
            board_snapshot: &movegen::BoardSnapshot {
                board_tiles: &self.game_state.board_tiles,
                game_config,
                kwg,
                anagrams: arm_tables.anagrams(),
                rack_lengths: arm_tables.rack_lengths(),
                klv,
            },
            rack: &self.game_state.players[self.game_state.turn as usize].rack,
            max_gen: 1,
            num_exchanges_by_this_player: self.game_state.players[self.game_state.turn as usize]
                .num_exchanges,
            pass_policy: movegen::PassPolicy::OnlyWhenForced,
            dynamic_leaves: None,
        };
        if LEAN {
            (self.move_generator).gen_moves_unfiltered_lanes_lean(params, rows, cols);
        } else {
            (self.move_generator).gen_moves_unfiltered_lanes(params, rows, cols);
        }
        let masked = &self.move_generator.plays[0];
        let forced_pass =
            matches!(masked.play, movegen::Play::Exchange { ref tiles } if tiles.is_empty());
        match off {
            Some(i) if forced_pass || self.lane_bests[i].0 < *masked => {
                self.lane_bests[i].0.clone()
            }
            _ => masked.clone(),
        }
    }

    // the plies the last rollout played after its candidate, in order: as many
    // as it was prepared to play, fewer if the game ended, and more if the bag
    // emptied and it played the endgame out
    #[inline(always)]
    pub fn plies_played(&self) -> &[PlyPlayed] {
        &self.plies_played
    }

    // the equity of the opponent's reply at ply 1 in the last rollout, as the
    // generator valued it: the best move of the rack it was dealt, which
    // infer::reply_to weighs over every rack. None when the rollout had no such
    // reply: the opponents passing, or the bag empty after the candidate.
    #[inline(always)]
    pub fn reply_equity(&self) -> Option<equity::Equity> {
        self.reply_equity
    }

    #[inline(always)]
    pub fn final_equity_spread(&self) -> error::Returns<i32> {
        let mut best_opponent_equity = i32::MIN;
        for (i, player) in (0..).zip(self.game_state.players.iter()) {
            if i != self.initial_game_state.turn {
                let opponent_equity = player
                    .score
                    .checked_add(self.last_seen_leave_values[i as usize])
                    .ok_or("score overflow")?;
                if opponent_equity > best_opponent_equity {
                    best_opponent_equity = opponent_equity;
                }
            }
        }
        let mut this_equity = self.game_state.players[self.initial_game_state.turn as usize]
            .score
            .checked_add(self.last_seen_leave_values[self.initial_game_state.turn as usize])
            .ok_or("score overflow")?;
        if best_opponent_equity != i32::MIN {
            this_equity = this_equity
                .checked_sub(best_opponent_equity)
                .ok_or("score overflow")?;
        }
        Ok(this_equity)
    }

    // final_equity_spread in its two parts, against the same opponent: the
    // mover's score less that opponent's, and the value of the leave the mover
    // last kept less theirs; the two sum to final_equity_spread
    #[inline(always)]
    pub fn final_spread_and_leaves(&self) -> error::Returns<(i32, i32)> {
        let mover = self.initial_game_state.turn as usize;
        let mut best_opponent: Option<(i32, usize)> = None;
        for (i, player) in self.game_state.players.iter().enumerate() {
            if i != mover {
                let opponent_equity = player
                    .score
                    .checked_add(self.last_seen_leave_values[i])
                    .ok_or("score overflow")?;
                if best_opponent.is_none_or(|(best, _)| opponent_equity > best) {
                    best_opponent = Some((opponent_equity, i));
                }
            }
        }
        let score = self.game_state.players[mover].score;
        let leave = self.last_seen_leave_values[mover];
        Ok(match best_opponent {
            Some((_, opponent)) => (
                score
                    .checked_sub(self.game_state.players[opponent].score)
                    .ok_or("score overflow")?,
                leave
                    .checked_sub(self.last_seen_leave_values[opponent])
                    .ok_or("score overflow")?,
            ),
            None => (score, leave),
        })
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

            if let Some(table) = table {
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
            win_prob_unfinished(final_spread, bag + racks_total)
        }
    }

    #[inline(always)]
    pub fn wants_win_prob(&self) -> bool {
        self.wants_win_prob
    }

    #[inline(always)]
    pub fn win_prob_weightage(&self) -> f64 {
        self.win_prob_weightage
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[inline]
    fn exchanging_rollout_leaves_shared_rng_untouched() {
        let game_config = game_config::make_english_game_config();
        let mut game_state = game_state::GameState::new(&game_config);
        let mut deal_rng = DefaultRng::seed_from_u64(3);
        game_state.reset_and_draw_tiles(&game_config, &mut deal_rng);

        let kwg = kwg::kbwg_from_bytes_alloc(b"\x00\x00\x40\x00".to_vec(), false).unwrap();
        let arm_tables = anagrams::ArmTables::build(&kwg, &game_config, false).unwrap();
        let klv = klv::kbwg_from_bytes_alloc(klv::EMPTY_KLV_BYTES.to_vec()).unwrap();
        let mut simmer = Simmer::new(&game_config).unwrap();
        simmer.reseed(99);
        simmer.prepare(&game_config, &game_state, 0, false, &[]);
        simmer.prepare_iteration();

        let exchanged =
            simmer.initial_game_state.players[simmer.initial_game_state.turn as usize].rack[0];
        let candidate = movegen::Play::Exchange {
            tiles: [exchanged][..].into(),
        };

        let shared_state = simmer.rng.clone();

        simmer
            .simulate(&game_config, &kwg, &arm_tables, &klv, &candidate)
            .unwrap();
        let rack_after_first = simmer.game_state.players[simmer.initial_game_state.turn as usize]
            .rack
            .clone();

        assert_eq!(simmer.rng, shared_state);

        simmer
            .simulate(&game_config, &kwg, &arm_tables, &klv, &candidate)
            .unwrap();
        let rack_after_second = simmer.game_state.players[simmer.initial_game_state.turn as usize]
            .rack
            .clone();
        assert_eq!(simmer.rng, shared_state);
        assert_eq!(rack_after_first, rack_after_second);
    }

    #[test]
    #[inline]
    fn spread_points_descales_millipoints_to_points() {
        assert_eq!(spread_points(0), 0.0);
        assert_eq!(spread_points(equity::SCALE), 1.0);
        assert_eq!(spread_points(30 * equity::SCALE), 30.0);
        assert_eq!(spread_points(-500), -0.5);
    }

    #[test]
    #[inline]
    fn win_prob_unfinished_hits_sigmoid_prob_at_the_crafted_lead() {
        let lead_points = 30.0 + 10.0;
        let lead_millipoints = (lead_points * equity::SCALE as f64) as i32;
        assert!((win_prob_unfinished(lead_millipoints, 10) - 0.9).abs() < 1e-9);
        assert!((win_prob_unfinished(-lead_millipoints, 10) - (1.0 - 0.9)).abs() < 1e-9);
        assert_eq!(win_prob_unfinished(0, 10), 0.5);
    }

    #[test]
    #[inline]
    fn sim_objective_descales_spread_and_scales_win_prob() {
        assert_eq!(sim_objective(30 * equity::SCALE, 0.0, 10.0), 30.0);
        assert_eq!(sim_objective(0, 1.0, 10.0), 10.0);
        assert_eq!(sim_objective(0, 1.0, 10000.0), 10000.0);
        assert_eq!(sim_objective(0, 0.5, 10.0), 5.0);
    }

    #[test]
    #[inline]
    fn per_instance_rng_reseed_is_deterministic() {
        let game_config = game_config::make_english_game_config();
        let mut game_state = game_state::GameState::new(&game_config);

        let mut deal_rng = DefaultRng::seed_from_u64(1);
        game_state.reset_and_draw_tiles(&game_config, &mut deal_rng);

        let opponent_draw = |seed: u64| -> Vec<u8> {
            let mut simmer = Simmer::new(&game_config).unwrap();
            simmer.prepare(&game_config, &game_state, 2, false, &[]);
            simmer.reseed(seed);
            simmer.prepare_iteration();

            simmer.initial_game_state.players[1].rack.clone()
        };

        assert_eq!(opponent_draw(777), opponent_draw(777));

        assert_ne!(opponent_draw(777), opponent_draw(778));
    }

    #[inline]
    fn prepared_simmer() -> (Simmer, usize, usize, usize) {
        let game_config = game_config::make_english_game_config();
        let mut game_state = game_state::GameState::new(&game_config);
        let mut deal_rng = DefaultRng::seed_from_u64(1);
        game_state.reset_and_draw_tiles(&game_config, &mut deal_rng);
        let mut simmer = Simmer::new(&game_config).unwrap();
        simmer.prepare(&game_config, &game_state, 2, false, &[]);
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

    #[test]
    #[inline]
    fn table_source_uses_table_where_sampled_else_sigmoid() {
        let (simmer, bag, my, opp) = prepared_simmer();

        let final_spread = 40 * equity::SCALE;
        let sigmoid = win_prob_unfinished(final_spread, bag + my + opp);
        assert!(sigmoid < 0.99, "sigmoid {sigmoid} unexpectedly saturated");

        let mut acc = win_pct::WinPctAccumulator::new();
        for &v in &[-5, 5] {
            acc.record(bag, my, opp, 0, v);
        }
        let table = acc.finalize();
        let got = simmer.compute_win_prob(false, final_spread, Some(&table));
        assert_eq!(got, 1.0, "table value should be used at the sampled key");

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

    #[test]
    #[inline]
    fn final_equity_spread_takes_the_best_opponent_from_the_mover() {
        let (mut simmer, _, _, _) = prepared_simmer();
        let turn = simmer.initial_game_state.turn as usize;
        simmer.game_state.players[turn].score = 300 * equity::SCALE;
        simmer.game_state.players[1 - turn].score = 250 * equity::SCALE;
        simmer.last_seen_leave_values[turn] = 5 * equity::SCALE;
        simmer.last_seen_leave_values[1 - turn] = 0;
        assert_eq!(simmer.final_equity_spread().unwrap(), 55 * equity::SCALE);
    }

    #[test]
    #[inline]
    fn final_spread_and_leaves_sum_to_final_equity_spread() {
        let (mut simmer, _, _, _) = prepared_simmer();
        let turn = simmer.initial_game_state.turn as usize;
        simmer.game_state.players[turn].score = 300 * equity::SCALE;
        simmer.game_state.players[1 - turn].score = 250 * equity::SCALE;
        simmer.last_seen_leave_values[turn] = 5 * equity::SCALE;
        simmer.last_seen_leave_values[1 - turn] = 12 * equity::SCALE;
        assert_eq!(
            simmer.final_spread_and_leaves().unwrap(),
            (50 * equity::SCALE, -7 * equity::SCALE)
        );
        assert_eq!(simmer.final_equity_spread().unwrap(), 43 * equity::SCALE);
    }

    // the bag emptied onto the board: the mover holds A and B (4 points) and
    // the opponent Q (10 points); every other tile, a blank as A among them, is
    // on the board, so the generator, which counts the bag from the board,
    // knows the bag is empty
    fn a_bag_emptied_onto_the_board(
        game_config: &game_config::GameConfig,
    ) -> game_state::GameState {
        let alphabet = game_config.alphabet();
        let mut game_state = game_state::GameState::new(game_config);
        game_state.players[0].rack.extend_from_slice(&[1, 2]);
        game_state.players[1].rack.push(17);
        let mut tally = (0..alphabet.len())
            .map(|tile| alphabet.freq(tile))
            .collect::<Vec<_>>();
        for player in game_state.players.iter() {
            for &tile in player.rack.iter() {
                tally[tile as usize] -= 1;
            }
        }
        let mut square = 0;
        for (tile, &count) in (0u8..).zip(tally.iter()) {
            for _ in 0..count {
                game_state.board_tiles[square] = if tile == 0 { 0x81 } else { tile };
                square += 1;
            }
        }
        game_state.bag.set_from_iter(std::iter::empty());
        game_state
    }

    #[test]
    #[inline]
    fn a_rollout_plays_the_endgame_out_once_the_bag_is_empty() {
        let game_config = game_config::make_english_game_config();
        let alphabet = game_config.alphabet();
        let game_state = a_bag_emptied_onto_the_board(&game_config);
        // the mover holds A and B (4 points), the opponent Q (10 points)
        assert_eq!(alphabet.rack_score(&game_state.players[0].rack), 4);
        assert_eq!(alphabet.rack_score(&game_state.players[1].rack), 10);

        // with no words, every ply passes, past the two-ply horizon, until the
        // sixth scoreless turn in a row ends the game
        let kwg = kwg::kbwg_from_bytes_alloc(b"\x00\x00\x40\x00".to_vec(), false).unwrap();
        let arm_tables = anagrams::ArmTables::build(&kwg, &game_config, false).unwrap();
        let klv = klv::kbwg_from_bytes_alloc(klv::EMPTY_KLV_BYTES.to_vec()).unwrap();
        let mut simmer = Simmer::new(&game_config).unwrap();
        simmer.reseed(5);
        simmer.prepare(&game_config, &game_state, 2, false, &[]);
        simmer.prepare_iteration();
        let pass = movegen::Play::Exchange {
            tiles: [][..].into(),
        };
        let game_ended = simmer
            .simulate(&game_config, &kwg, &arm_tables, &klv, &pass)
            .unwrap();
        assert!(game_ended);
        assert_eq!(simmer.game_state.zero_turns, 6);

        // a game that ends on scoreless turns takes each player's own tiles off
        // their score, so the mover is ahead by 6 points
        assert_eq!(simmer.final_equity_spread().unwrap(), 6 * equity::SCALE);
    }

    #[test]
    #[inline]
    fn a_rollout_with_no_horizon_ends_where_the_game_does() {
        // plies past any horizon, as many as there can be: with no words every
        // ply passes. Under the english rules the sixth scoreless turn in a row
        // ends the game. Under rules where passes never end a game, the rollout
        // stops once each seat the generator moves has had to pass, whatever
        // its horizon: two plies after the candidate whichever way the opponent
        // passes. Each rack is valued as kept to the end with the game not
        // over: our A and B against the opponent's Q.
        let english = game_config::make_english_game_config();
        let mut endless = english.clone();
        let game_config::GameConfig::Static(rules) = &mut endless;
        rules.num_passes_to_end = 0;
        rules.num_zeros_to_end = 0;
        let kwg = kwg::kbwg_from_bytes_alloc(b"\x00\x00\x40\x00".to_vec(), false).unwrap();
        let klv = klv::kbwg_from_bytes_alloc(klv::EMPTY_KLV_BYTES.to_vec()).unwrap();
        let pass = movegen::Play::Exchange {
            tiles: [][..].into(),
        };
        for (game_config, plies, opponents_pass, ended, played, zero_turns, spread) in [
            (&english, usize::MAX, false, true, 5, 6, 6),
            (&english, usize::MAX, true, true, 5, 6, 6),
            (&endless, usize::MAX, false, false, 2, 3, 12),
            (&endless, usize::MAX, true, false, 2, 3, 12),
            (&endless, 5, false, false, 2, 3, 12),
            (&endless, 0, true, false, 2, 3, 12),
        ] {
            let case = (plies, opponents_pass, ended);
            let game_state = a_bag_emptied_onto_the_board(game_config);
            let arm_tables = anagrams::ArmTables::build(&kwg, game_config, false).unwrap();
            let mut simmer = Simmer::new(game_config).unwrap();
            simmer.reseed(5);
            simmer.prepare(game_config, &game_state, plies, false, &[]);
            simmer.set_opponents_pass(opponents_pass);
            simmer.prepare_iteration();
            let game_ended = simmer
                .simulate(game_config, &kwg, &arm_tables, &klv, &pass)
                .unwrap();
            assert_eq!(game_ended, ended, "{case:?}");
            assert_eq!(simmer.plies_played().len(), played, "{case:?}");
            assert_eq!(simmer.game_state.zero_turns, zero_turns, "{case:?}");
            assert_eq!(
                simmer.final_equity_spread().unwrap(),
                spread * equity::SCALE,
                "{case:?}"
            );
        }
    }

    #[test]
    #[inline]
    fn the_opponent_holds_every_unseen_tile_whatever_they_kept_once_the_bag_is_empty() {
        // the opponent's move drew the bag empty, so every tile the mover cannot
        // see is on the opponent's rack: a kept leave put back first, and the
        // refill after it, give them those tiles again, whichever leave is drawn
        let game_config = game_config::make_english_game_config();
        let mut game_state = game_state::GameState::new(&game_config);
        let mut deal_rng = DefaultRng::seed_from_u64(2);
        game_state.reset_and_draw_tiles(&game_config, &mut deal_rng);
        game_state.bag.set_from_iter(std::iter::empty());
        let opponent = 1 - game_state.turn as usize;
        let mut held = game_state.players[opponent].rack.clone();
        held.sort_unstable();
        let others = (0..game_config.alphabet().len())
            .filter(|tile| !held.contains(tile))
            .collect::<Vec<_>>();
        let kept_leaves = [
            (held[..2].to_vec(), 1.0),
            (vec![others[0], others[1]], 1.0),
            (Vec::new(), 1.0),
        ];
        let mut simmer = Simmer::new(&game_config).unwrap();
        simmer.prepare(&game_config, &game_state, 2, false, &kept_leaves);
        for seed in 0..20 {
            simmer.reseed(seed);
            simmer.prepare_iteration();
            let mut rack = simmer.initial_game_state.players[opponent].rack.clone();
            rack.sort_unstable();
            assert_eq!(rack, held, "seed {seed}");
            assert!(simmer.initial_game_state.bag.is_empty(), "seed {seed}");
        }
    }

    // C(n, k) as a float, 0 when k > n
    #[inline]
    fn choose(n: usize, k: usize) -> f64 {
        if k > n {
            return 0.0;
        }
        (0..k).fold(1.0, |acc, i| acc * (n - i) as f64 / (i + 1) as f64)
    }

    #[test]
    #[inline]
    fn the_opponent_redraws_from_the_whole_bag_whichever_seat_decides() {
        // a player at an odd seat draws from the bag's front, at an even seat
        // from its back, so shuffling only one end would give one seat's
        // opponent back much of the rack it held
        let game_config = game_config::make_english_game_config();
        for decider in 0..2u8 {
            let mut game_state = game_state::GameState::new(&game_config);
            let mut deal_rng = DefaultRng::seed_from_u64(11);
            game_state.reset_and_draw_tiles_double_ended(&game_config, &mut deal_rng);
            game_state.turn = decider;
            let opponent = 1 - decider as usize;
            let held = game_state.players[opponent].rack.clone();
            let rack_size = held.len();

            // the expected overlap of a random rack from the unseen tiles with
            // the held rack: for each letter it holds k of, the mean of
            // min(drawn, k) over the chances of drawing each count of that
            // letter
            let mut unseen = vec![0usize; game_config.alphabet().len() as usize];
            for &tile in game_state.bag.as_slice().iter().chain(held.iter()) {
                unseen[tile as usize] += 1;
            }
            let total = unseen.iter().sum::<usize>();
            let mut expected = 0.0;
            for (tile, &copies) in unseen.iter().enumerate() {
                let k = held.iter().filter(|&&t| t as usize == tile).count();
                for x in 1..=rack_size {
                    expected +=
                        x.min(k) as f64 * choose(copies, x) * choose(total - copies, rack_size - x)
                            / choose(total, rack_size);
                }
            }

            let mut simmer = Simmer::new(&game_config).unwrap();
            simmer.prepare(&game_config, &game_state, 4, false, &[]);
            let pristine = simmer.prepared_state().clone();
            let draws = 1000;
            let mut shared = 0;
            for seed in 0..draws {
                simmer.restore_prepared(&pristine);
                simmer.reseed(seed);
                simmer.prepare_iteration();
                let mut left = held.clone();
                for &tile in &simmer.initial_game_state.players[opponent].rack {
                    if let Some(pos) = left.iter().position(|&t| t == tile) {
                        left.swap_remove(pos);
                        shared += 1;
                    }
                }
            }
            let mean = shared as f64 / draws as f64;
            assert!(
                (mean - expected).abs() < 0.2,
                "seat {decider} decides: the opponent's draws share {mean} of the rack it held, {expected} expected"
            );
        }
    }

    #[test]
    #[inline]
    fn the_opponent_keeps_a_leave_drawn_by_weight() {
        let game_config = game_config::make_english_game_config();
        let alphabet = game_config.alphabet();
        let mut game_state = game_state::GameState::new(&game_config);
        let mut deal_rng = DefaultRng::seed_from_u64(11);
        game_state.reset_and_draw_tiles_double_ended(&game_config, &mut deal_rng);
        let opponent = 1 - game_state.turn as usize;

        // two leaves of six unseen tiles each, one of vowels and one of
        // consonants, so one tile more cannot make either look like the other
        let mut unseen = vec![0u8; alphabet.len() as usize];
        for &tile in game_state
            .bag
            .as_slice()
            .iter()
            .chain(game_state.players[opponent].rack.iter())
        {
            unseen[tile as usize] += 1;
        }
        let six_of = |vowels: bool| {
            let mut leave = Vec::new();
            for tile in 1..alphabet.len() {
                if alphabet.is_vowel(tile) == vowels {
                    for _ in 0..unseen[tile as usize] {
                        if leave.len() < 6 {
                            leave.push(tile);
                        }
                    }
                }
            }
            assert_eq!(leave.len(), 6);
            leave
        };
        let (vowels, consonants) = (six_of(true), six_of(false));
        let holds = |rack: &[u8], leave: &[u8]| {
            let mut left = rack.to_vec();
            leave.iter().all(|tile| {
                left.iter()
                    .position(|t| t == tile)
                    .map(|pos| left.swap_remove(pos))
                    .is_some()
            })
        };

        let mut simmer = Simmer::new(&game_config).unwrap();
        simmer.prepare(
            &game_config,
            &game_state,
            4,
            false,
            &[(vowels.clone(), 3.0), (consonants.clone(), 1.0)],
        );
        let pristine = simmer.prepared_state().clone();
        let (mut drew_vowels, mut drew_consonants) = (0, 0);
        for seed in 0..1000 {
            simmer.restore_prepared(&pristine);
            simmer.reseed(seed);
            simmer.prepare_iteration();
            let rack = &simmer.initial_game_state.players[opponent].rack;
            assert_eq!(rack.len(), 7);
            if holds(rack, &vowels) {
                drew_vowels += 1;
            } else if holds(rack, &consonants) {
                drew_consonants += 1;
            }
        }
        assert_eq!(drew_vowels + drew_consonants, 1000);
        // weighted three to one: 750 expected, with a standard deviation of
        // about 14
        assert!(
            (700..=800).contains(&drew_vowels),
            "the leave weighing 3 of 4 was drawn {drew_vowels} times in 1000"
        );

        // a new prepare forgets them
        simmer.prepare(&game_config, &game_state, 4, false, &[]);
        assert!(simmer.kept.is_empty());
    }

    // leaves set packed and shared deal as the same leaves set as a list do,
    // the simmer holding the packed ones without a copy; one of weight 0 among
    // them changes no draw
    #[test]
    #[inline]
    fn packed_kept_leaves_deal_as_a_list_of_them_does() {
        let game_config = game_config::make_english_game_config();
        let mut game_state = game_state::GameState::new(&game_config);
        let mut deal_rng = DefaultRng::seed_from_u64(11);
        game_state.reset_and_draw_tiles_double_ended(&game_config, &mut deal_rng);
        let opponent = 1 - game_state.turn;
        let rack = game_state.players[opponent as usize].rack.clone();
        let listed = [
            (rack[..3].to_vec(), 2.0),
            (rack[3..5].to_vec(), 0.0),
            (rack[..1].to_vec(), 1.0),
            (rack[1..6].to_vec(), 0.5),
        ];
        let mut packed = infer::PackedLeaves::new();
        for (leave, weight) in &listed {
            packed.push(leave, *weight).unwrap();
        }
        let packed = std::sync::Arc::new(packed);
        let mut from_list = Simmer::new(&game_config).unwrap();
        from_list.prepare(&game_config, &game_state, 4, false, &listed);
        let mut from_packed = Simmer::new(&game_config).unwrap();
        from_packed.prepare(&game_config, &game_state, 4, false, &[]);
        from_packed.set_packed_kept_leaves(opponent, packed.clone());
        assert_eq!(std::sync::Arc::strong_count(&packed), 2);
        let pristine = from_list.prepared_state().clone();
        for seed in 0..200 {
            for simmer in [&mut from_list, &mut from_packed] {
                simmer.restore_prepared(&pristine);
                simmer.reseed(seed);
                simmer.prepare_iteration();
            }
            assert_eq!(
                from_packed.initial_game_state.players[opponent as usize].rack,
                from_list.initial_game_state.players[opponent as usize].rack,
                "{seed}"
            );
        }
        // leaves of no weight at all leave the seat dealt from the bag alone
        let mut weightless = infer::PackedLeaves::new();
        weightless.push(&rack[..2], 0.0).unwrap();
        from_packed.set_packed_kept_leaves(opponent, std::sync::Arc::new(weightless));
        assert!(from_packed.kept.is_empty());
    }

    #[test]
    #[inline]
    fn each_seat_keeps_the_leave_set_for_it_in_a_game_of_three() {
        // in a game of three, prepare's leaves are the next seat's;
        // set_kept_leaves gives the other one its own. Each is dealt its known
        // tiles first, every iteration, the mover's rack untouched
        let mut game_config = game_config::make_english_game_config();
        let game_config::GameConfig::Static(rules) = &mut game_config;
        rules.num_players = 3;
        let alphabet = game_config.alphabet();
        // the mover holds A to G; every other tile is unseen, the other two
        // seats dealt seven of them each
        let mut game_state = game_state::GameState::new(&game_config);
        game_state.players[0].rack.extend(1..=7);
        let mut tally = (0..alphabet.len())
            .map(|tile| alphabet.freq(tile))
            .collect::<Vec<_>>();
        for &tile in &game_state.players[0].rack {
            tally[tile as usize] -= 1;
        }
        game_state.bag.set_from_iter(
            (0u8..)
                .zip(tally.iter())
                .flat_map(|(tile, &count)| std::iter::repeat_n(tile, count as usize)),
        );
        for seat in 1..3 {
            game_state
                .bag
                .replenish(&mut game_state.players[seat].rack, 7, seat);
        }
        let holds = |rack: &[u8], tiles: &[u8]| {
            let mut left = rack.to_vec();
            tiles.iter().all(|tile| {
                left.iter()
                    .position(|t| t == tile)
                    .map(|pos| left.swap_remove(pos))
                    .is_some()
            })
        };
        // the next seat may have kept three Zs, of which the game has one; the
        // seat after it a Q and a J, one each
        let (next, after) = (vec![26u8, 26, 26], vec![17u8, 10]);
        let mut simmer = Simmer::new(&game_config).unwrap();
        simmer.prepare(&game_config, &game_state, 2, false, &[(next, 1.0)]);
        simmer.set_kept_leaves(2, &[(after.clone(), 1.0)]);
        for seed in 0..50 {
            simmer.reseed(seed);
            simmer.prepare_iteration();
            let players = &simmer.initial_game_state.players;
            assert_eq!(players[0].rack, [1, 2, 3, 4, 5, 6, 7], "seed {seed}");
            assert_eq!(players[1].rack.len(), 7, "seed {seed}");
            assert_eq!(players[2].rack.len(), 7, "seed {seed}");
            assert!(holds(&players[1].rack, &[26]), "seed {seed}");
            assert!(holds(&players[2].rack, &after), "seed {seed}");
        }
    }

    // every word of two or three tiles over the letters given
    #[inline]
    fn kwg_of(letters: &[u8]) -> kwg::Kwg<kwg::Node24> {
        let mut words = Vec::new();
        for &a in letters {
            for &b in letters {
                words.push([a, b][..].into());
                for &c in letters {
                    words.push([a, b, c][..].into());
                }
            }
        }
        words.sort_unstable();
        let bytes = crate::build::build(
            crate::build::BuildContent::Gaddawg,
            crate::build::BuildLayout::Wolges,
            crate::build::BuildOrder::Sorted,
            &words,
        )
        .unwrap();
        kwg::kbwg_from_bytes_alloc(bytes.to_vec(), false).unwrap()
    }

    // the move a whole generation keeps for the side to move
    #[inline]
    fn whole_reply<G: movegen::LaneBits, const JUMPS: bool, const ANY_SIDE: bool>(
        generator: &mut movegen::KurniaMoveGenerator<G, JUMPS, ANY_SIDE>,
        game_config: &game_config::GameConfig,
        kwg: &kwg::Kwg<kwg::Node24>,
        arm_tables: &anagrams::ArmTables,
        klv: &klv::Klv<kwg::Node24>,
        game_state: &game_state::GameState,
    ) -> movegen::ValuedMove {
        generator.gen_moves_unfiltered(&movegen::GenMovesParams {
            board_snapshot: &movegen::BoardSnapshot {
                board_tiles: &game_state.board_tiles,
                game_config,
                kwg,
                anagrams: arm_tables.anagrams(),
                rack_lengths: arm_tables.rack_lengths(),
                klv,
            },
            rack: &game_state.current_player().rack,
            max_gen: 1,
            num_exchanges_by_this_player: game_state.current_player().num_exchanges,
            pass_policy: movegen::PassPolicy::OnlyWhenForced,
            dynamic_leaves: None,
        });
        generator.plays[0].clone()
    }

    #[test]
    #[inline]
    fn the_reply_at_ply_one_is_the_whole_generations() {
        // on the board after each candidate of a wide field, the opponent's
        // reply found lane by lane is the one a whole generation keeps: from
        // the start of a game and from boards a few plies in, under racks with
        // blanks, exchanges and a pass among the candidates
        let game_config = game_config::make_english_game_config();
        let kwg = kwg_of(&[1, 5, 9, 14, 18, 19, 20]);
        let arm_tables = anagrams::ArmTables::build(&kwg, &game_config, false).unwrap();
        let klv = klv::kbwg_from_bytes_alloc(klv::EMPTY_KLV_BYTES.to_vec()).unwrap();
        let mut whole = movegen::KurniaMoveGenerator::new(&game_config);
        let mut field = movegen::KurniaMoveGenerator::new(&game_config);
        let (mut checked, mut reused, mut from_lanes) = (0usize, 0usize, 0usize);
        for seed in 0..16u64 {
            let mut game_state = game_state::GameState::new(&game_config);
            let mut deal_rng = DefaultRng::seed_from_u64(seed);
            game_state.reset_and_draw_tiles(&game_config, &mut deal_rng);
            for _ in 0..(seed % 6) {
                let play = whole_reply(
                    &mut whole,
                    &game_config,
                    &kwg,
                    &arm_tables,
                    &klv,
                    &game_state,
                );
                game_state
                    .play(&game_config, &mut deal_rng, &play.play)
                    .unwrap();
                game_state.next_turn();
            }
            let mut simmer = Simmer::new(&game_config).unwrap();
            simmer.reseed(seed);
            simmer.prepare(&game_config, &game_state, 2, false, &[]);
            simmer.prepare_iteration();
            let initial = simmer.initial_game_state.clone();
            field.gen_moves_unfiltered(&movegen::GenMovesParams {
                board_snapshot: &movegen::BoardSnapshot {
                    board_tiles: &initial.board_tiles,
                    game_config: &game_config,
                    kwg: &kwg,
                    anagrams: arm_tables.anagrams(),
                    rack_lengths: arm_tables.rack_lengths(),
                    klv: &klv,
                },
                rack: &initial.current_player().rack,
                max_gen: 100,
                num_exchanges_by_this_player: initial.current_player().num_exchanges,
                pass_policy: movegen::PassPolicy::AsACandidate,
                dynamic_leaves: None,
            });
            let candidates = field
                .plays
                .iter()
                .map(|m| m.play.clone())
                .collect::<Vec<_>>();
            for candidate in &candidates {
                simmer.game_state.clone_from(&initial);
                let mut rng = simmer.rng.copy_state();
                simmer
                    .game_state
                    .play(&game_config, &mut rng, candidate)
                    .unwrap();
                if !matches!(
                    simmer
                        .game_state
                        .check_game_ended(&game_config, &mut simmer.final_scores)
                        .unwrap(),
                    game_state::CheckGameEnded::NotEnded
                ) {
                    continue;
                }
                simmer.game_state.next_turn();
                if infer::bag_size(&simmer.game_state.board_tiles, &game_config) <= 0 {
                    continue;
                }
                let reply = simmer.reply_at_ply_one::<false, _, _>(
                    &game_config,
                    &kwg,
                    &arm_tables,
                    &klv,
                    candidate,
                    None,
                );
                let want = whole_reply(
                    &mut whole,
                    &game_config,
                    &kwg,
                    &arm_tables,
                    &klv,
                    &simmer.game_state,
                );
                let snapshot = movegen::BoardSnapshot {
                    board_tiles: &simmer.game_state.board_tiles,
                    game_config: &game_config,
                    kwg: &kwg,
                    anagrams: None,
                    rack_lengths: None,
                    klv: &klv,
                };
                assert!(
                    reply == want,
                    "seed {seed}: {} against {}",
                    reply.play.fmt(&snapshot),
                    want.play.fmt(&snapshot)
                );
                checked += 1;
                if simmer.lane_bests.iter().any(|(m, _, _)| *m == reply) {
                    reused += 1;
                }
            }
            from_lanes += simmer.lane_bests.len();
        }
        assert!(checked > 500, "{checked} checked");
        assert!(reused > 50, "{reused} replies came from the lanes");
        assert!(from_lanes > 16, "{from_lanes} lane-bests found");
    }

    #[test]
    #[inline]
    fn a_lean_rollout_moves_as_the_full_one() {
        // the same rollouts, move for move, from the start of a game and from
        // boards a few plies in, over a wide field and several draws: the full
        // arm with anagram tables, as the bins run it, and the lean arm with
        // none, as the wasm does
        let game_config = game_config::make_english_game_config();
        let kwg = kwg_of(&[1, 5, 9, 14, 18, 19, 20]);
        let arm_tables = anagrams::ArmTables::build(&kwg, &game_config, false).unwrap();
        let no_tables = anagrams::ArmTables::None;
        let klv = klv::kbwg_from_bytes_alloc(klv::EMPTY_KLV_BYTES.to_vec()).unwrap();
        let mut whole = movegen::KurniaMoveGenerator::new(&game_config);
        let mut field = movegen::KurniaMoveGenerator::new(&game_config);
        let mut checked = 0usize;
        for seed in 0..12u64 {
            let mut game_state = game_state::GameState::new(&game_config);
            let mut deal_rng = DefaultRng::seed_from_u64(seed);
            game_state.reset_and_draw_tiles(&game_config, &mut deal_rng);
            for _ in 0..(seed % 6) {
                let play = whole_reply(
                    &mut whole,
                    &game_config,
                    &kwg,
                    &arm_tables,
                    &klv,
                    &game_state,
                );
                game_state
                    .play(&game_config, &mut deal_rng, &play.play)
                    .unwrap();
                game_state.next_turn();
            }
            field.gen_moves_unfiltered(&movegen::GenMovesParams {
                board_snapshot: &movegen::BoardSnapshot {
                    board_tiles: &game_state.board_tiles,
                    game_config: &game_config,
                    kwg: &kwg,
                    anagrams: arm_tables.anagrams(),
                    rack_lengths: arm_tables.rack_lengths(),
                    klv: &klv,
                },
                rack: &game_state.current_player().rack,
                max_gen: 30,
                num_exchanges_by_this_player: game_state.current_player().num_exchanges,
                pass_policy: movegen::PassPolicy::AsACandidate,
                dynamic_leaves: None,
            });
            let mut full = Simmer::new(&game_config).unwrap();
            let mut lean = Simmer::new(&game_config).unwrap();
            full.prepare(&game_config, &game_state, 2, true, &[]);
            lean.prepare(&game_config, &game_state, 2, true, &[]);
            let pristine = full.prepared_state().clone();
            for iteration in 0..4 {
                for simmer in [&mut full, &mut lean] {
                    simmer.restore_prepared(&pristine);
                    simmer.reseed(seed * 100 + iteration);
                    simmer.prepare_iteration();
                }
                for candidate in field.plays.iter().map(|m| &m.play) {
                    let ended = full
                        .simulate(&game_config, &kwg, &arm_tables, &klv, candidate)
                        .unwrap();
                    let lean_ended = lean
                        .simulate_lean(&game_config, &kwg, &no_tables, &klv, candidate)
                        .unwrap();
                    assert_eq!(ended, lean_ended, "seed {seed}");
                    assert_eq!(
                        full.final_equity_spread().unwrap(),
                        lean.final_equity_spread().unwrap(),
                        "seed {seed}"
                    );
                    assert_eq!(
                        full.game_state.board_tiles, lean.game_state.board_tiles,
                        "seed {seed}"
                    );
                    checked += 1;
                }
            }
        }
        assert!(checked > 1000, "{checked} checked");
    }

    #[test]
    #[inline]
    fn a_rollout_keeps_each_ply_it_played() {
        // after a rollout that did not end the game, each seat's score is its
        // score before, plus the candidate's for the mover, plus the plies kept
        // for that seat, which take turns from the seat after the mover; with
        // tiles left in the bag it played the plies it was prepared to play
        let game_config = game_config::make_english_game_config();
        let kwg = kwg_of(&[1, 5, 9, 14, 18, 19, 20]);
        let no_tables = anagrams::ArmTables::None;
        let klv = klv::kbwg_from_bytes_alloc(klv::EMPTY_KLV_BYTES.to_vec()).unwrap();
        let mut field = movegen::KurniaMoveGenerator::new(&game_config);
        let (mut checked, mut placed) = (0usize, 0usize);
        for seed in 0..8u64 {
            let mut game_state = game_state::GameState::new(&game_config);
            let mut deal_rng = DefaultRng::seed_from_u64(seed);
            game_state.reset_and_draw_tiles(&game_config, &mut deal_rng);
            field.gen_moves_unfiltered_lean(&movegen::GenMovesParams {
                board_snapshot: &movegen::BoardSnapshot {
                    board_tiles: &game_state.board_tiles,
                    game_config: &game_config,
                    kwg: &kwg,
                    anagrams: None,
                    rack_lengths: None,
                    klv: &klv,
                },
                rack: &game_state.current_player().rack,
                max_gen: 20,
                num_exchanges_by_this_player: game_state.current_player().num_exchanges,
                pass_policy: movegen::PassPolicy::AsACandidate,
                dynamic_leaves: None,
            });
            let mut simmer = Simmer::new(&game_config).unwrap();
            simmer.prepare(&game_config, &game_state, 3, true, &[]);
            simmer.reseed(seed);
            simmer.prepare_iteration();
            let turn = game_state.turn as usize;
            for candidate in field.plays.iter().map(|m| &m.play) {
                let ended = simmer
                    .simulate_lean(&game_config, &kwg, &no_tables, &klv, candidate)
                    .unwrap();
                if ended {
                    continue;
                }
                let plies = simmer.plies_played();
                if !simmer.game_state.bag.is_empty() {
                    assert_eq!(plies.len(), 3, "seed {seed}");
                }
                let mut gained = [0, 0];
                if let movegen::Play::Place { score, .. } = candidate {
                    gained[turn] = *score;
                }
                for (i, ply) in plies.iter().enumerate() {
                    gained[(turn + 1 + i) % 2] += ply.score;
                    assert!(ply.tiles <= game_config.rack_size(), "seed {seed}");
                    if ply.tiles > 0 {
                        placed += 1;
                    }
                }
                for (seat, gained) in gained.iter().enumerate() {
                    assert_eq!(
                        simmer.game_state.players[seat].score - game_state.players[seat].score,
                        *gained,
                        "seed {seed}, seat {seat}"
                    );
                }
                checked += 1;
            }
        }
        assert!(checked > 50, "{checked} checked");
        assert!(placed > 50, "{placed} plies placed tiles");
    }

    #[test]
    #[inline]
    fn a_sim_given_its_plies_after_it_was_prepared_rolls_out_as_one_prepared_so() {
        // prepared for two plies and then set to four, a sim rolls out each
        // candidate on each seed as one prepared for four does, and weighs the
        // chance to win alike
        let game_config = game_config::make_english_game_config();
        let kwg = kwg_of(&[1, 5, 9, 14, 18, 19, 20]);
        let no_tables = anagrams::ArmTables::None;
        let klv = klv::kbwg_from_bytes_alloc(klv::EMPTY_KLV_BYTES.to_vec()).unwrap();
        let mut field = movegen::KurniaMoveGenerator::new(&game_config);
        let mut checked = 0;
        for seed in 0..4u64 {
            let mut game_state = game_state::GameState::new(&game_config);
            let mut deal_rng = DefaultRng::seed_from_u64(seed);
            game_state.reset_and_draw_tiles(&game_config, &mut deal_rng);
            field.gen_moves_unfiltered_lean(&movegen::GenMovesParams {
                board_snapshot: &movegen::BoardSnapshot {
                    board_tiles: &game_state.board_tiles,
                    game_config: &game_config,
                    kwg: &kwg,
                    anagrams: None,
                    rack_lengths: None,
                    klv: &klv,
                },
                rack: &game_state.current_player().rack,
                max_gen: 5,
                num_exchanges_by_this_player: game_state.current_player().num_exchanges,
                pass_policy: movegen::PassPolicy::AsACandidate,
                dynamic_leaves: None,
            });
            let mut set = Simmer::new(&game_config).unwrap();
            set.prepare(&game_config, &game_state, 2, false, &[]);
            set.set_num_sim_plies(&game_config, 4);
            let mut made = Simmer::new(&game_config).unwrap();
            made.prepare(&game_config, &game_state, 4, false, &[]);
            assert_eq!(set.win_prob_weightage, made.win_prob_weightage);
            for candidate in field.plays.iter().map(|m| &m.play) {
                let mut rolled = Vec::new();
                for simmer in [&mut set, &mut made] {
                    simmer.reseed(seed);
                    simmer.prepare_iteration();
                    simmer
                        .simulate_lean(&game_config, &kwg, &no_tables, &klv, candidate)
                        .unwrap();
                    let plies = (simmer.plies_played().iter())
                        .map(|ply| (ply.score, ply.tiles))
                        .collect::<Vec<_>>();
                    rolled.push((plies, simmer.final_equity_spread().unwrap()));
                }
                assert_eq!(rolled[0], rolled[1], "seed {seed}");
                // a rollout of four plies, two of which a sim of two never plays
                if rolled[0].0.len() == 4 {
                    checked += 1;
                }
            }
        }
        assert!(checked >= 4, "{checked} rollouts of four plies");
    }

    #[test]
    #[inline]
    fn a_rollout_whose_opponents_pass_plays_only_the_movers_plies() {
        // with the opponents passing, every ply of theirs is a pass; the
        // mover's are found as before, and some of them place tiles
        let game_config = game_config::make_english_game_config();
        let kwg = kwg_of(&[1, 5, 9, 14, 18, 19, 20]);
        let no_tables = anagrams::ArmTables::None;
        let klv = klv::kbwg_from_bytes_alloc(klv::EMPTY_KLV_BYTES.to_vec()).unwrap();
        let mut field = movegen::KurniaMoveGenerator::new(&game_config);
        let (mut theirs, mut ours_placed) = (0usize, 0usize);
        for seed in 0..8u64 {
            let mut game_state = game_state::GameState::new(&game_config);
            let mut deal_rng = DefaultRng::seed_from_u64(seed);
            game_state.reset_and_draw_tiles(&game_config, &mut deal_rng);
            field.gen_moves_unfiltered_lean(&movegen::GenMovesParams {
                board_snapshot: &movegen::BoardSnapshot {
                    board_tiles: &game_state.board_tiles,
                    game_config: &game_config,
                    kwg: &kwg,
                    anagrams: None,
                    rack_lengths: None,
                    klv: &klv,
                },
                rack: &game_state.current_player().rack,
                max_gen: 10,
                num_exchanges_by_this_player: game_state.current_player().num_exchanges,
                pass_policy: movegen::PassPolicy::AsACandidate,
                dynamic_leaves: None,
            });
            let mut simmer = Simmer::new(&game_config).unwrap();
            simmer.prepare(&game_config, &game_state, 4, true, &[]);
            simmer.set_opponents_pass(true);
            simmer.reseed(seed);
            simmer.prepare_iteration();
            for candidate in field.plays.iter().map(|m| &m.play) {
                simmer
                    .simulate_lean(&game_config, &kwg, &no_tables, &klv, candidate)
                    .unwrap();
                for (i, ply) in simmer.plies_played().iter().enumerate() {
                    if i % 2 == 0 {
                        assert_eq!((ply.score, ply.tiles), (0, 0), "seed {seed}");
                        theirs += 1;
                    } else if ply.tiles > 0 {
                        ours_placed += 1;
                    }
                }
            }
        }
        assert!(theirs > 50, "{theirs} of their plies");
        assert!(ours_placed > 20, "{ours_placed} of ours placed tiles");
    }

    #[test]
    #[inline]
    fn a_rollout_keeps_the_equity_of_its_reply_at_ply_one() {
        // with no leave values, the reply a rollout played at ply 1, on a board
        // its candidate placed tiles on, is worth its score; a rollout whose
        // opponents pass has no such reply, and keeps none from the one before
        let game_config = game_config::make_english_game_config();
        let kwg = kwg_of(&[1, 5, 9, 14, 18, 19, 20]);
        let no_tables = anagrams::ArmTables::None;
        let klv = klv::kbwg_from_bytes_alloc(klv::EMPTY_KLV_BYTES.to_vec()).unwrap();
        let mut field = movegen::KurniaMoveGenerator::new(&game_config);
        let (mut checked, mut scored) = (0usize, 0usize);
        for seed in 0..8u64 {
            let mut game_state = game_state::GameState::new(&game_config);
            let mut deal_rng = DefaultRng::seed_from_u64(seed);
            game_state.reset_and_draw_tiles(&game_config, &mut deal_rng);
            field.gen_moves_unfiltered_lean(&movegen::GenMovesParams {
                board_snapshot: &movegen::BoardSnapshot {
                    board_tiles: &game_state.board_tiles,
                    game_config: &game_config,
                    kwg: &kwg,
                    anagrams: None,
                    rack_lengths: None,
                    klv: &klv,
                },
                rack: &game_state.current_player().rack,
                max_gen: 20,
                num_exchanges_by_this_player: game_state.current_player().num_exchanges,
                pass_policy: movegen::PassPolicy::AsACandidate,
                dynamic_leaves: None,
            });
            let mut simmer = Simmer::new(&game_config).unwrap();
            simmer.prepare(&game_config, &game_state, 3, true, &[]);
            simmer.reseed(seed);
            simmer.prepare_iteration();
            for candidate in field.plays.iter().map(|m| &m.play) {
                if !matches!(candidate, movegen::Play::Place { .. }) {
                    continue;
                }
                simmer.set_opponents_pass(false);
                simmer
                    .simulate_lean(&game_config, &kwg, &no_tables, &klv, candidate)
                    .unwrap();
                let reply = simmer.plies_played()[0];
                assert_eq!(
                    simmer.reply_equity().map(|equity| equity.raw()),
                    Some(reply.score),
                    "seed {seed}"
                );
                if reply.score > 0 {
                    scored += 1;
                }
                simmer.set_opponents_pass(true);
                simmer
                    .simulate_lean(&game_config, &kwg, &no_tables, &klv, candidate)
                    .unwrap();
                assert!(simmer.reply_equity().is_none(), "seed {seed}");
                checked += 1;
            }
        }
        assert!(checked > 50, "{checked} checked");
        assert!(scored > 50, "{scored} replies scored");
    }

    #[test]
    #[inline]
    fn a_play_challenged_off_rolls_out_as_a_pass_but_for_its_turns() {
        // on the same draws a play challenged off the board leaves the board
        // and the scores a pass leaves; its turn counts as scoreless, and as a
        // pass only where the config says challenges are passes
        let kwg = kwg_of(&[1, 5, 9, 14, 18, 19, 20]);
        let no_tables = anagrams::ArmTables::None;
        let klv = klv::kbwg_from_bytes_alloc(klv::EMPTY_KLV_BYTES.to_vec()).unwrap();
        let pass = movegen::Play::Exchange {
            tiles: [][..].into(),
        };
        let mut checked = 0;
        for (game_config, passes) in [
            (game_config::make_english_game_config(), 0),
            (game_config::make_spanish_game_config(), 1),
        ] {
            assert_eq!(game_config.challenges_are_passes(), passes == 1);
            for seed in 0..6u64 {
                let mut game_state = game_state::GameState::new(&game_config);
                let mut deal_rng = DefaultRng::seed_from_u64(seed);
                game_state.reset_and_draw_tiles(&game_config, &mut deal_rng);
                for plies in [0, 2] {
                    let mut simmer = Simmer::new(&game_config).unwrap();
                    simmer.prepare(&game_config, &game_state, plies, true, &[]);
                    let pristine = simmer.prepared_state().clone();
                    let mut after = Vec::new();
                    for challenged_off in [false, true] {
                        simmer.restore_prepared(&pristine);
                        simmer.reseed(seed);
                        simmer.prepare_iteration();
                        if challenged_off {
                            simmer
                                .simulate_lean_challenged_off(&game_config, &kwg, &no_tables, &klv)
                                .unwrap();
                        } else {
                            simmer
                                .simulate_lean(&game_config, &kwg, &no_tables, &klv, &pass)
                                .unwrap();
                        }
                        let state = &simmer.game_state;
                        after.push((
                            state.board_tiles.clone(),
                            state.players.iter().map(|p| p.score).collect::<Vec<_>>(),
                            state.zero_turns,
                            state.pass_turns,
                        ));
                    }
                    assert_eq!(after[1].0, after[0].0, "seed {seed}");
                    assert_eq!(after[1].1, after[0].1, "seed {seed}");
                    if plies == 0 {
                        assert_eq!((after[0].2, after[0].3), (1, 1), "seed {seed}");
                        assert_eq!((after[1].2, after[1].3), (1, passes), "seed {seed}");
                    }
                    checked += 1;
                }
            }
        }
        assert_eq!(checked, 24);
    }

    #[test]
    #[inline]
    fn final_equity_spread_errs_when_a_sum_would_overflow() {
        let (mut simmer, _, _, _) = prepared_simmer();
        let turn = simmer.initial_game_state.turn as usize;
        simmer.game_state.players[turn].score = i32::MAX - 5;
        simmer.last_seen_leave_values[turn] = 10;
        assert!(simmer.final_equity_spread().is_err());
    }
}
