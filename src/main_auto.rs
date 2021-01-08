// Copyright (C) 2020-2021 Andy Kurnia. All rights reserved.

use super::{alphabet, bag, display, error, game_config, klv, kwg, movegen, prob, stats};
use rand::prelude::*;

fn use_tiles<II: IntoIterator<Item = u8>>(
    rack: &mut Vec<u8>,
    tiles_iter: II,
) -> error::Returns<()> {
    for tile in tiles_iter {
        let pos = rack.iter().rposition(|&t| t == tile).ok_or("bad tile")?;
        rack.swap_remove(pos);
    }
    Ok(())
}

struct WriteableRack<'a> {
    alphabet: &'a alphabet::Alphabet<'a>,
    rack: &'a [u8],
}

impl std::fmt::Display for WriteableRack<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        for &tile in self.rack {
            write!(f, "{}", self.alphabet.from_rack(tile).unwrap())?;
        }
        Ok(())
    }
}

fn printable_rack<'a>(alphabet: &'a alphabet::Alphabet<'a>, rack: &'a [u8]) -> WriteableRack<'a> {
    WriteableRack {
        alphabet: &alphabet,
        rack: &rack,
    }
}

fn rack_score<'a>(alphabet: &'a alphabet::Alphabet<'a>, rack: &'a [u8]) -> i16 {
    rack.iter().map(|&t| alphabet.score(t) as i16).sum::<i16>()
}

struct GamePlayer {
    score: i16,
    rack: Vec<u8>,
}

impl<'a> Clone for GamePlayer {
    #[inline(always)]
    fn clone(&self) -> Self {
        Self {
            score: self.score,
            rack: self.rack.clone(),
        }
    }

    #[inline(always)]
    fn clone_from(&mut self, source: &Self) {
        self.score.clone_from(&source.score);
        self.rack.clone_from(&source.rack);
    }
}

struct GameState<'a> {
    game_config: &'a game_config::GameConfig<'a>,
    players: Box<[GamePlayer]>,
    board_tiles: Box<[u8]>,
    bag: bag::Bag,
    turn: u8,
}

impl<'a> Clone for GameState<'a> {
    #[inline(always)]
    fn clone(&self) -> Self {
        Self {
            game_config: self.game_config,
            players: self.players.clone(),
            board_tiles: self.board_tiles.clone(),
            bag: self.bag.clone(),
            turn: self.turn,
        }
    }

    #[inline(always)]
    fn clone_from(&mut self, source: &Self) {
        self.game_config.clone_from(&source.game_config);
        self.players.clone_from(&source.players);
        self.board_tiles.clone_from(&source.board_tiles);
        self.bag.clone_from(&source.bag);
        self.turn.clone_from(&source.turn);
    }
}

impl<'a> GameState<'a> {
    fn new(game_config: &'a game_config::GameConfig) -> Self {
        let board_layout = game_config.board_layout();
        let dim = board_layout.dim();
        let rack_size = game_config.rack_size() as usize;
        let alphabet = game_config.alphabet();
        Self {
            game_config,
            players: (0..game_config.num_players())
                .map(|_| GamePlayer {
                    score: 0,
                    rack: Vec::with_capacity(rack_size),
                })
                .collect(),
            board_tiles: vec![0u8; (dim.rows as usize) * (dim.cols as usize)].into_boxed_slice(),
            bag: bag::Bag::new(&alphabet),
            turn: 0,
        }
    }

    fn current_player(&self) -> &GamePlayer {
        &self.players[self.turn as usize]
    }

    fn play(&mut self, mut rng: &mut dyn RngCore, play: &movegen::Play) -> error::Returns<()> {
        let current_player = &mut self.players[self.turn as usize];
        match play {
            movegen::Play::Exchange { tiles } => {
                use_tiles(&mut current_player.rack, tiles.iter().copied())?;
                self.bag.replenish(
                    &mut current_player.rack,
                    self.game_config.rack_size() as usize,
                );
                self.bag.put_back(&mut rng, &tiles);
            }
            movegen::Play::Place {
                down,
                lane,
                idx,
                word,
                score,
            } => {
                let dim = self.game_config.board_layout().dim();
                let strider = if *down {
                    dim.down(*lane)
                } else {
                    dim.across(*lane)
                };

                // place the tiles
                for (i, &tile) in (*idx..).zip(word.iter()) {
                    if tile != 0 {
                        self.board_tiles[strider.at(i)] = tile;
                    }
                }

                current_player.score += score;
                use_tiles(
                    &mut current_player.rack,
                    word.iter().filter_map(|&tile| {
                        if tile != 0 {
                            Some(tile & !((tile as i8) >> 7) as u8)
                        } else {
                            None
                        }
                    }),
                )?;
                self.bag.replenish(
                    &mut current_player.rack,
                    self.game_config.rack_size() as usize,
                );
            }
        }
        Ok(())
    }

    fn next_turn(&mut self) {
        let num_players = self.players.len() as u8;
        self.turn += 1;
        self.turn -= num_players & -((self.turn >= num_players) as i8) as u8;
    }
}

fn get_max_probs_by_len<'a>(
    word_prob: &'a mut prob::WordProbability<'a>,
    g: &'a kwg::Kwg,
) -> Box<[u64]> {
    let mut v = Vec::new();
    struct Env<'a, 'b> {
        g: &'a kwg::Kwg,
        s: &'a mut Vec<u8>,
        v: &'a mut Vec<u64>,
        word_prob: &'b mut prob::WordProbability<'b>,
    }
    fn iter(env: &mut Env, mut p: i32) {
        let l = env.s.len() + 1;
        loop {
            let t = env.g[p].tile();
            env.s.push(t);
            if env.g[p].accepts() {
                while env.v.len() <= l {
                    env.v.push(0);
                }
                env.v[l] = std::cmp::max(env.v[l], env.word_prob.count_ways(env.s));
            }
            if env.g[p].arc_index() != 0 {
                iter(env, env.g[p].arc_index());
            }
            env.s.pop();
            if env.g[p].is_end() {
                break;
            }
            p += 1;
        }
    }
    iter(
        &mut Env {
            g: &g,
            s: &mut Vec::new(),
            v: &mut v,
            word_prob,
        },
        g[0].arc_index(),
    );
    v.into_boxed_slice()
}

// need more realistic numbers, and should differ by bot level
static LENGTH_IMPORTANCES: &[f32] = &[
    0.0, 0.0, 2.0, 1.5, 1.0, 0.75, 0.5, 1.0, 1.0, 0.5, 0.4, 0.3, 0.2, 0.1, 0.1, 0.1,
];

pub fn main() -> error::Returns<()> {
    let mut reusable_vec_for_candidate_moves = Vec::new();
    let kwg = kwg::Kwg::from_bytes_alloc(&std::fs::read("csw19.kwg")?);
    let klv = klv::Klv::from_bytes_alloc(&std::fs::read("leaves.klv")?);
    let game_config = &game_config::make_common_english_game_config();
    //let kwg = kwg::Kwg::from_bytes_alloc(&std::fs::read("osps42.kwg")?);
    //let game_config = &game_config::make_polish_game_config();
    let _ = &game_config::make_polish_game_config();
    let mut move_generator = movegen::KurniaMoveGenerator::new(game_config);
    let mut word_check_buf = Vec::new();

    let mut word_prob = prob::WordProbability::new(&game_config.alphabet());
    let max_prob_by_len = get_max_probs_by_len(&mut word_prob, &kwg);
    println!("max prob: {:?}", max_prob_by_len);

    // why is borrow checker disallowing reusing the earlier variable?
    let mut word_prob = prob::WordProbability::new(&game_config.alphabet());

    loop {
        let mut game_state = GameState::new(game_config);
        let mut rack_tally = vec![0u8; game_config.alphabet().len() as usize].into_boxed_slice();

        let mut zero_turns = 0;
        println!("\nplaying self");
        let mut rng = rand_chacha::ChaCha20Rng::from_entropy();

        game_state.bag.shuffle(&mut rng);

        println!(
            "bag: {}",
            printable_rack(&game_state.game_config.alphabet(), &game_state.bag.0)
        );

        for player in game_state.players.iter_mut() {
            game_state.bag.replenish(
                &mut player.rack,
                game_state.game_config.rack_size() as usize,
            );
        }

        loop {
            display::print_board(
                &game_state.game_config.alphabet(),
                &game_state.game_config.board_layout(),
                &game_state.board_tiles,
            );
            for (i, player) in (1..).zip(game_state.players.iter()) {
                print!("player {}: {}, ", i, player.score);
            }
            println!("turn: player {}", game_state.turn + 1);

            let bot_level = if game_state.turn == 0 { 6 } else { 3 };
            let length_importances = LENGTH_IMPORTANCES; // should differ by bot level
            let mut tilt_factor = rng.gen_range(0.5 - bot_level as f64 * 0.1, 1.0);
            if tilt_factor < 0.0 {
                tilt_factor = 0.0;
            }
            println!("effective tilt factor for this turn: {}", tilt_factor);
            let mut leave_scale = (bot_level as f64 * 0.1 + (1.0 - tilt_factor)) as f32;
            if leave_scale > 1.0 {
                leave_scale = 1.0;
            }
            println!("effective leave scale for this turn: {}", leave_scale);
            let mut word_is_ok = |word: &[u8]| {
                let this_wp = word_prob.count_ways(word);
                let max_wp = max_prob_by_len[word.len()];
                let some_prob = 1.0 - (1.0 - (this_wp as f64 / max_wp as f64)).powi(2);
                let mut handwavy = length_importances[word.len()] as f64 * some_prob;
                if handwavy > 1.0 {
                    handwavy = 1.0;
                }
                if handwavy >= tilt_factor {
                    return true;
                }
                println!(
                    "Rejecting word {:?}, handwavy={} (this={} over max={}), tilt={}",
                    word, handwavy, this_wp, max_wp, tilt_factor
                );
                false
            };

            println!(
                "pool {:2}: {}",
                game_state.bag.0.len(),
                printable_rack(&game_state.game_config.alphabet(), &game_state.bag.0)
            );
            for (i, player) in (1..).zip(game_state.players.iter()) {
                println!(
                    "p{} rack: {}",
                    i,
                    printable_rack(&game_state.game_config.alphabet(), &player.rack)
                );
            }

            let board_snapshot = &movegen::BoardSnapshot {
                board_tiles: &game_state.board_tiles,
                game_config,
                kwg: &kwg,
                klv: &klv,
            };

            let mut validate_word_subset =
                |board_snapshot: &movegen::BoardSnapshot,
                 down: bool,
                 lane: i8,
                 idx: i8,
                 word: &[u8],
                 _score: i16,
                 _rack_tally: &[u8]| {
                    let board_layout = board_snapshot.game_config.board_layout();
                    let dim = board_layout.dim();
                    let strider = if down {
                        dim.down(lane)
                    } else {
                        dim.across(lane)
                    };
                    word_check_buf.clear();
                    for (i, &tile) in (idx..).zip(word.iter()) {
                        let placed_tile = if tile != 0 {
                            tile
                        } else {
                            board_snapshot.board_tiles[strider.at(i)]
                        };
                        word_check_buf.push(placed_tile & 0x7f);
                    }
                    if !word_is_ok(&word_check_buf) {
                        return false;
                    }
                    for (i, &tile) in (idx..).zip(word.iter()) {
                        if tile != 0 {
                            let perpendicular_strider =
                                if down { dim.across(i) } else { dim.down(i) };
                            let mut j = lane;
                            while j > 0
                                && board_snapshot.board_tiles[perpendicular_strider.at(j - 1)] != 0
                            {
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
                            word_check_buf.clear();
                            for j in j..perpendicular_strider_len {
                                let placed_tile = if j == lane {
                                    tile
                                } else {
                                    board_snapshot.board_tiles[perpendicular_strider.at(j)]
                                };
                                if placed_tile == 0 {
                                    break;
                                }
                                word_check_buf.push(placed_tile & 0x7f);
                            }
                            if !word_is_ok(&word_check_buf) {
                                return false;
                            }
                        }
                    }
                    true
                };
            let adjust_leave_value = |leave_value: f32| leave_scale * leave_value;
            move_generator.gen_moves_alloc(
                board_snapshot,
                &game_state.current_player().rack,
                15,
                |down: bool, lane: i8, idx: i8, word: &[u8], score: i16, rack_tally: &[u8]| {
                    validate_word_subset(&board_snapshot, down, lane, idx, word, score, rack_tally)
                },
                adjust_leave_value,
            );
            let plays = &mut move_generator.plays;

            println!("found {} moves", plays.len());
            for play in plays.iter() {
                println!("{} {}", play.equity, play.play.fmt(board_snapshot));
            }

            println!("let's sim them");
            struct CandidateResult {
                equity: f32,
                stats: stats::Stats,
            }
            let mut candidates = std::collections::HashMap::new();
            for play in plays.drain(..) {
                candidates.insert(
                    play.play,
                    CandidateResult {
                        equity: play.equity,
                        stats: stats::Stats::new(),
                    },
                );
            }
            assert!(plays.is_empty());
            {
                let mut simmer_rng = rand_chacha::ChaCha20Rng::from_entropy();
                let mut simmer_move_generator = movegen::KurniaMoveGenerator::new(game_config);
                let mut simmer_initial_game_state = game_state.clone(); // will be overwritten
                let initial_spread = if simmer_initial_game_state.players.len() < 2 {
                    0
                } else {
                    simmer_initial_game_state.current_player().score
                        - (0..)
                            .zip(simmer_initial_game_state.players.iter())
                            .filter(|&(i, _)| i != simmer_initial_game_state.turn)
                            .map(|(_, player)| player.score)
                            .max()
                            .unwrap()
                };
                //println!("initial spread is {}", initial_spread);
                let mut last_seen_leave_values =
                    vec![0.0f32; simmer_initial_game_state.players.len()];
                let mut simmer_game_state = simmer_initial_game_state.clone(); // will be overwritten
                let mut simmer_rack_tally = rack_tally.clone(); // will be overwritten
                let num_sim_iters = 10;
                let num_sim_plies = 2;
                let num_tiles_that_matter = num_sim_plies * game_config.rack_size() as usize;
                let t0 = std::time::Instant::now();
                for sim_iter in 0..num_sim_iters {
                    if (sim_iter + 1) % 100 == 0 {
                        println!("{} iters in {:?}", sim_iter, t0.elapsed());
                    }
                    //println!("iter {}", sim_iter);
                    //println!(
                    //    "bag: {}",
                    //    printable_rack(&game_config.alphabet(), &simmer_initial_game_state.bag.0)
                    //);
                    loop {
                        simmer_initial_game_state.next_turn();
                        if simmer_initial_game_state.turn == game_state.turn {
                            break;
                        }
                        let player = &mut simmer_initial_game_state.players
                            [simmer_initial_game_state.turn as usize];
                        //println!(
                        //    "p{} rack was: {}",
                        //    simmer_initial_game_state.turn + 1,
                        //    printable_rack(&game_config.alphabet(), &player.rack,)
                        //);
                        simmer_initial_game_state
                            .bag
                            .put_back(&mut rng, &player.rack);
                        player.rack.clear();
                    }
                    simmer_initial_game_state
                        .bag
                        .shuffle_n(&mut simmer_rng, num_tiles_that_matter);
                    last_seen_leave_values.iter_mut().for_each(|m| *m = 0.0);
                    //println!(
                    //    "bag: {}",
                    //    printable_rack(&game_config.alphabet(), &simmer_initial_game_state.bag.0)
                    //);
                    loop {
                        simmer_initial_game_state.next_turn();
                        if simmer_initial_game_state.turn == game_state.turn {
                            break;
                        }
                        let player = &mut simmer_initial_game_state.players
                            [simmer_initial_game_state.turn as usize];
                        simmer_initial_game_state.bag.replenish(
                            &mut player.rack,
                            game_state.players[simmer_initial_game_state.turn as usize]
                                .rack
                                .len(),
                        );
                    }
                    //for (i, player) in (1..).zip(simmer_initial_game_state.players.iter()) {
                    //    println!(
                    //        "p{} rack: {}",
                    //        i,
                    //        printable_rack(&game_config.alphabet(), &player.rack)
                    //    );
                    //}
                    //println!(
                    //    "bag: {}",
                    //    printable_rack(&game_config.alphabet(), &simmer_initial_game_state.bag.0)
                    //);
                    for (play, candidate_result) in candidates.iter_mut() {
                        simmer_game_state.clone_from(&simmer_initial_game_state);
                        let mut played_out = false;
                        for ply in 0..=num_sim_plies {
                            let simmer_board_snapshot = &movegen::BoardSnapshot {
                                board_tiles: &simmer_game_state.board_tiles,
                                game_config,
                                kwg: &kwg,
                                klv: &klv,
                            };
                            let next_play = if ply == 0 {
                                &play
                            } else {
                                simmer_move_generator.gen_moves_alloc(
                                    simmer_board_snapshot,
                                    &simmer_game_state.current_player().rack,
                                    1,
                                    |down: bool,
                                     lane: i8,
                                     idx: i8,
                                     word: &[u8],
                                     score: i16,
                                     rack_tally: &[u8]| {
                                        validate_word_subset(
                                            &simmer_board_snapshot,
                                            down,
                                            lane,
                                            idx,
                                            word,
                                            score,
                                            rack_tally,
                                        )
                                    },
                                    adjust_leave_value,
                                );
                                &simmer_move_generator.plays[0].play
                            };
                            //print!(
                            //    "{} {} (leave ",
                            //    next_play.equity,
                            //    next_play.play.fmt(simmer_board_snapshot)
                            //);
                            simmer_rack_tally.iter_mut().for_each(|m| *m = 0);
                            simmer_game_state
                                .current_player()
                                .rack
                                .iter()
                                .for_each(|&tile| simmer_rack_tally[tile as usize] += 1);
                            match &next_play {
                                movegen::Play::Exchange { tiles } => {
                                    tiles
                                        .iter()
                                        .for_each(|&tile| simmer_rack_tally[tile as usize] -= 1);
                                }
                                movegen::Play::Place { word, .. } => {
                                    word.iter().for_each(|&tile| {
                                        if tile & 0x80 != 0 {
                                            simmer_rack_tally[0] -= 1;
                                        } else if tile != 0 {
                                            simmer_rack_tally[tile as usize] -= 1;
                                        }
                                    });
                                }
                            };
                            //(0u8..)
                            //    .zip(simmer_rack_tally.iter())
                            //    .for_each(|(tile, &count)| {
                            //        (0..count).for_each(|_| {
                            //            print!(
                            //                "{}",
                            //                game_config.alphabet().from_rack(tile).unwrap()
                            //            )
                            //        })
                            //    });
                            let leave_value = klv.leave_value_from_tally(&simmer_rack_tally);
                            //print!(" = {}), ", leave_value);
                            last_seen_leave_values[simmer_game_state.turn as usize] = leave_value;
                            simmer_game_state.play(&mut simmer_rng, &next_play)?;
                            if simmer_game_state.current_player().rack.is_empty() {
                                played_out = true;
                                //print!("(that played out) ");
                                break;
                            }
                            simmer_game_state.next_turn();
                        }
                        //for (i, player) in (1..).zip(simmer_game_state.players.iter()) {
                        //    print!(
                        //        "player {}: {} {}, ",
                        //        i,
                        //        player.score,
                        //        printable_rack(&game_config.alphabet(), &player.rack)
                        //    );
                        //}
                        //println!("leave is {:?}, board:", last_seen_leave_values);
                        //display::print_board(
                        //    &game_config.alphabet(),
                        //    &game_config.board_layout(),
                        //    &simmer_game_state.board_tiles,
                        //);
                        //println!(
                        //    "bag: {}",
                        //    printable_rack(&game_config.alphabet(), &simmer_game_state.bag.0)
                        //);
                        if played_out {
                            // not handling the too-many-zeros case
                            if simmer_game_state.players.len() == 2 {
                                simmer_game_state.players[simmer_game_state.turn as usize].score +=
                                    2 * rack_score(
                                        &game_config.alphabet(),
                                        &simmer_game_state.players
                                            [(1 - simmer_game_state.turn) as usize]
                                            .rack,
                                    );
                            } else {
                                let mut earned = 0;
                                for mut player in simmer_game_state.players.iter_mut() {
                                    let this_rack =
                                        rack_score(&game_config.alphabet(), &player.rack);
                                    player.score -= this_rack;
                                    earned += this_rack;
                                }
                                simmer_game_state.players[simmer_game_state.turn as usize].score +=
                                    earned;
                            }
                            last_seen_leave_values.iter_mut().for_each(|m| *m = 0.0);
                        }
                        let mut best_opponent = simmer_initial_game_state.turn;
                        let mut best_opponent_equity = f32::NEG_INFINITY;
                        for (i, player) in (0..).zip(simmer_game_state.players.iter()) {
                            //print!(
                            //    "p{} is ({}+...={})+{}={}, ",
                            //    i + 1,
                            //    simmer_initial_game_state.players[i as usize].score,
                            //    player.score,
                            //    last_seen_leave_values[i as usize],
                            //    player.score as f32 + last_seen_leave_values[i as usize]
                            //);
                            if i != simmer_initial_game_state.turn {
                                let opponent_equity =
                                    player.score as f32 + last_seen_leave_values[i as usize];
                                if opponent_equity > best_opponent_equity {
                                    best_opponent = i;
                                    best_opponent_equity = opponent_equity;
                                }
                            }
                        }
                        //print!(
                        //    "best opponent is p{} with {}, ",
                        //    best_opponent + 1,
                        //    best_opponent_equity
                        //);
                        let mut this_equity = simmer_game_state.players
                            [simmer_initial_game_state.turn as usize]
                            .score as f32
                            + last_seen_leave_values[simmer_initial_game_state.turn as usize];
                        //print!("valuation = {}", this_equity);
                        if best_opponent != simmer_initial_game_state.turn {
                            this_equity -= best_opponent_equity;
                            //print!(" - {} = {}", best_opponent_equity, this_equity);
                        }
                        //println!(
                        //    ", initial spread = {}, equity to record = {}",
                        //    initial_spread,
                        //    this_equity - initial_spread as f32
                        //);
                        candidate_result
                            .stats
                            .update((this_equity - initial_spread as f32) as f64);
                    }
                }
            }
            assert!(plays.is_empty());
            reusable_vec_for_candidate_moves.clear();
            reusable_vec_for_candidate_moves.reserve(candidates.len());
            for (play, candidate_result) in candidates.drain() {
                let mean = candidate_result.stats.mean();
                reusable_vec_for_candidate_moves.push((play, candidate_result, mean));
            }
            assert!(candidates.is_empty());
            reusable_vec_for_candidate_moves.sort_unstable_by(|a, b| {
                b.2.partial_cmp(&a.2)
                    .unwrap()
                    .then_with(|| b.1.equity.partial_cmp(&a.1.equity).unwrap())
            });
            for (play, candidate_result, _mean) in reusable_vec_for_candidate_moves.drain(..) {
                println!(
                    "{} {}: {} samples, {} mean, {} stddev",
                    candidate_result.equity,
                    play.fmt(board_snapshot),
                    candidate_result.stats.count(),
                    candidate_result.stats.mean(),
                    candidate_result.stats.standard_deviation()
                );
                plays.push(movegen::ValuedMove {
                    equity: candidate_result.equity,
                    play,
                });
            }
            assert!(reusable_vec_for_candidate_moves.is_empty());
            for play in plays.iter() {
                println!("{} {}", play.equity, play.play.fmt(board_snapshot));
            }

            // show that this is unaffected by sim
            println!(
                "bag= {}",
                printable_rack(&game_state.game_config.alphabet(), &game_state.bag.0)
            );

            let play = &plays[0]; // assume at least there's always Pass
            println!("making top move: {}", play.play.fmt(board_snapshot));

            // manually recount and double-check the score and equity given by movegen
            let mut recounted_score = 0;
            match &play.play {
                movegen::Play::Exchange { .. } => {}
                movegen::Play::Place {
                    down,
                    lane,
                    idx,
                    word,
                    ..
                } => {
                    let alphabet = game_config.alphabet();
                    let board_layout = game_config.board_layout();
                    let premiums = board_layout.premiums();
                    let dim = board_layout.dim();
                    let strider = if *down {
                        dim.down(*lane)
                    } else {
                        dim.across(*lane)
                    };
                    let mut num_played = 0;

                    print!("main word: (down={} lane={} idx={}) ", down, lane, idx);
                    {
                        let mut word_multiplier = 1;
                        let mut word_score = 0i16;
                        for (i, &tile) in (*idx..).zip(word.iter()) {
                            let strider_at_i = strider.at(i);
                            let tile_multiplier;
                            let premium = premiums[strider_at_i];
                            let placed_tile = if tile != 0 {
                                num_played += 1;
                                word_multiplier *= premium.word_multiplier;
                                tile_multiplier = premium.tile_multiplier;
                                tile
                            } else {
                                tile_multiplier = 1;
                                board_snapshot.board_tiles[strider_at_i]
                            };
                            let face_value_tile_score = alphabet.score(placed_tile);
                            let tile_score = face_value_tile_score as i16 * tile_multiplier as i16;
                            word_score += tile_score;
                            print!(
                                "{} ({} * {} = {}), ",
                                alphabet.from_board(placed_tile).unwrap(),
                                face_value_tile_score,
                                tile_multiplier,
                                tile_score
                            );
                        }
                        let multiplied_word_score = word_score * word_multiplier as i16;
                        println!(
                            "for {} * {} = {}",
                            word_score, word_multiplier, multiplied_word_score
                        );
                        recounted_score += multiplied_word_score;
                    }

                    for (i, &tile) in (*idx..).zip(word.iter()) {
                        if tile != 0 {
                            let perpendicular_strider =
                                if *down { dim.across(i) } else { dim.down(i) };
                            let mut j = *lane;
                            while j > 0
                                && board_snapshot.board_tiles[perpendicular_strider.at(j - 1)] != 0
                            {
                                j -= 1;
                            }
                            let perpendicular_strider_len = perpendicular_strider.len();
                            if j == *lane
                                && if j + 1 < perpendicular_strider_len {
                                    board_snapshot.board_tiles[perpendicular_strider.at(j + 1)] == 0
                                } else {
                                    true
                                }
                            {
                                // no perpendicular tile
                                continue;
                            }
                            print!("perpendicular word: (down={} lane={} idx={}) ", !down, i, j);
                            let mut word_multiplier = 1;
                            let mut word_score = 0i16;
                            for j in j..perpendicular_strider_len {
                                let perpendicular_strider_at_j = perpendicular_strider.at(j);
                                let tile_multiplier;
                                let premium = premiums[perpendicular_strider_at_j];
                                let placed_tile = if j == *lane {
                                    word_multiplier *= premium.word_multiplier;
                                    tile_multiplier = premium.tile_multiplier;
                                    tile
                                } else {
                                    tile_multiplier = 1;
                                    board_snapshot.board_tiles[perpendicular_strider_at_j]
                                };
                                if placed_tile == 0 {
                                    break;
                                }
                                let face_value_tile_score = alphabet.score(placed_tile);
                                let tile_score =
                                    face_value_tile_score as i16 * tile_multiplier as i16;
                                word_score += tile_score;
                                print!(
                                    "{} ({} * {} = {}), ",
                                    alphabet.from_board(placed_tile).unwrap(),
                                    face_value_tile_score,
                                    tile_multiplier,
                                    tile_score
                                );
                            }
                            let multiplied_word_score = word_score * word_multiplier as i16;
                            println!(
                                "for {} * {} = {}",
                                word_score, word_multiplier, multiplied_word_score
                            );
                            recounted_score += multiplied_word_score;
                        }
                    }
                    let num_played_bonus = game_config.num_played_bonus(num_played);
                    println!(
                        "bonus for playing {} tiles: {}",
                        num_played, num_played_bonus
                    );
                    recounted_score += num_played_bonus;
                }
            };
            let movegen_score = match play.play {
                movegen::Play::Exchange { .. } => 0,
                movegen::Play::Place { score, .. } => score,
            };
            println!(
                "recounted score = {}, difference = {}",
                recounted_score,
                movegen_score - recounted_score
            );
            assert_eq!(recounted_score, movegen_score);

            rack_tally.iter_mut().for_each(|m| *m = 0);
            game_state
                .current_player()
                .rack
                .iter()
                .for_each(|&tile| rack_tally[tile as usize] += 1);
            match &play.play {
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
            print!("leave: ");
            (0u8..).zip(rack_tally.iter()).for_each(|(tile, &count)| {
                (0..count)
                    .for_each(|_| print!("{}", game_config.alphabet().from_rack(tile).unwrap()))
            });
            print!(" = ");
            let leave_value = klv.leave_value_from_tally(&rack_tally);
            println!("{}", leave_value);

            let mut recounted_equity = recounted_score as f32;
            if game_state.bag.0.is_empty() {
                // empty bag, do not add leave.
                println!("bag is empty");
                if rack_tally.iter().any(|&count| count != 0) {
                    let kept_tiles_worth = (0u8..)
                        .zip(rack_tally.iter())
                        .map(|(tile, &count)| {
                            count as i16 * game_config.alphabet().score(tile) as i16
                        })
                        .sum::<i16>();
                    let kept_tiles_penalty = 10 + 2 * kept_tiles_worth;
                    recounted_equity -= kept_tiles_penalty as f32;
                    println!(
                        "kept tiles are worth {}, penalizing by {}: {}",
                        kept_tiles_worth, kept_tiles_penalty, recounted_equity
                    );
                } else {
                    println!("playing out");
                    let mut unplayed_tiles_worth = 0;
                    for (player_idx, player) in (0u8..).zip(game_state.players.iter()) {
                        if player_idx != game_state.turn {
                            let their_tile_worth = player
                                .rack
                                .iter()
                                .map(|&tile| game_config.alphabet().score(tile) as i16)
                                .sum::<i16>();
                            println!("p{} rack is worth {}", player_idx + 1, their_tile_worth);
                            unplayed_tiles_worth += their_tile_worth;
                        }
                    }
                    let unplayed_tiles_bonus = 2 * unplayed_tiles_worth;
                    recounted_equity += unplayed_tiles_bonus as f32;
                    println!(
                        "total worth {}, adding {}: {}",
                        unplayed_tiles_worth, unplayed_tiles_bonus, recounted_equity
                    );
                }
            } else {
                recounted_equity += leave_scale * leave_value;
                println!("after adjusting for leave: {}", recounted_equity);
                if !game_state.board_tiles.iter().any(|&tile| tile != 0) {
                    println!("nothing on board");
                    match &play.play {
                        movegen::Play::Exchange { .. } => {}
                        movegen::Play::Place {
                            down,
                            lane,
                            idx,
                            word,
                            ..
                        } => {
                            let alphabet = game_config.alphabet();
                            let board_layout = game_config.board_layout();
                            let premiums = board_layout.premiums();
                            let dim = board_layout.dim();
                            let num_lanes = if *down { dim.cols } else { dim.rows };
                            let strider1 = if *lane > 0 {
                                Some(if *down {
                                    dim.down(*lane - 1)
                                } else {
                                    dim.across(*lane - 1)
                                })
                            } else {
                                None
                            };
                            let strider2 = if *lane < num_lanes - 1 {
                                Some(if *down {
                                    dim.down(*lane + 1)
                                } else {
                                    dim.across(*lane + 1)
                                })
                            } else {
                                None
                            };
                            let dangerous_vowel_count = (*idx..)
                                .zip(word.iter())
                                .filter(|(i, &tile)| {
                                    tile != 0 && alphabet.is_vowel(tile) && {
                                        (match strider1 {
                                            Some(strider) => {
                                                let premium = premiums[strider.at(*i)];
                                                premium.tile_multiplier != 1
                                                    || premium.word_multiplier != 1
                                            }
                                            None => false,
                                        }) || (match strider2 {
                                            Some(strider) => {
                                                let premium = premiums[strider.at(*i)];
                                                premium.tile_multiplier != 1
                                                    || premium.word_multiplier != 1
                                            }
                                            None => false,
                                        })
                                    }
                                })
                                .count();
                            let dangerous_vowel_penalty = dangerous_vowel_count as f32 * 0.7;
                            recounted_equity -= dangerous_vowel_penalty as f32;
                            println!(
                                "dangerous vowel count {}, penalizing by {}: {}",
                                dangerous_vowel_count, dangerous_vowel_penalty, recounted_equity
                            );
                        }
                    }
                }
            }
            let movegen_equity = play.equity;
            println!(
                "recounted equity = {}, difference = {}",
                recounted_equity,
                movegen_equity - recounted_equity
            );
            assert_eq!(recounted_equity, movegen_equity);

            game_state.play(&mut rng, &play.play)?;

            zero_turns += 1;
            if match play.play {
                movegen::Play::Exchange { .. } => 0,
                movegen::Play::Place { score, .. } => score,
            } != 0
            {
                zero_turns = 0;
            }

            if game_state.current_player().rack.is_empty() {
                display::print_board(
                    &game_state.game_config.alphabet(),
                    &game_state.game_config.board_layout(),
                    &game_state.board_tiles,
                );
                for (i, player) in (1..).zip(game_state.players.iter()) {
                    print!("player {}: {}, ", i, player.score);
                }
                println!(
                    "player {} went out (scores are before leftovers)",
                    game_state.turn + 1
                );
                if game_state.players.len() == 2 {
                    game_state.players[game_state.turn as usize].score += 2 * rack_score(
                        &game_state.game_config.alphabet(),
                        &game_state.players[(1 - game_state.turn) as usize].rack,
                    );
                } else {
                    let mut earned = 0;
                    for mut player in game_state.players.iter_mut() {
                        let this_rack =
                            rack_score(&game_state.game_config.alphabet(), &player.rack);
                        player.score -= this_rack;
                        earned += this_rack;
                    }
                    game_state.players[game_state.turn as usize].score += earned;
                }
                break;
            }

            if zero_turns >= game_state.players.len() * 3 {
                display::print_board(
                    &game_state.game_config.alphabet(),
                    &game_state.game_config.board_layout(),
                    &game_state.board_tiles,
                );
                for (i, player) in (1..).zip(game_state.players.iter()) {
                    print!("player {}: {}, ", i, player.score);
                }
                println!(
                    "player {} ended game by making yet another zero score",
                    game_state.turn + 1
                );
                for mut player in game_state.players.iter_mut() {
                    player.score -= rack_score(&game_state.game_config.alphabet(), &player.rack);
                }
                break;
            }

            game_state.next_turn();
        }

        for (i, player) in (1..).zip(game_state.players.iter()) {
            print!("player {}: {}, ", i, player.score);
        }
        println!("final scores");
    } // temp loop

    Ok(())
}
