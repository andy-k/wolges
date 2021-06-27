// Copyright (C) 2020-2021 Andy Kurnia.

use rand::prelude::*;
use std::fmt::Write;
use wolges::{display, error, game_config, game_state, klv, kwg, move_picker, movegen};

thread_local! {
    static RNG: std::cell::RefCell<Box<dyn RngCore>> =
        std::cell::RefCell::new(Box::new(rand_chacha::ChaCha20Rng::from_entropy()));
}

// omits 01IOl
static BASE57: &[u8; 57] = b"\
23456789\
ABCDEFGHJKLMNPQRSTUVWXYZ\
abcdefghijkmnopqrstuvwxyz\
";

const GAME_ID_LEN: usize = 8;

enum CSVRow<T1, T2> {
    Log(T1),
    Game(T2),
}

struct SerializeArc<T>(std::sync::Arc<T>);

impl<T: serde::Serialize> serde::Serialize for SerializeArc<T> {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        (*self.0).serialize(serializer)
    }
}

pub fn main() -> error::Returns<()> {
    let args = std::env::args().collect::<Vec<_>>();
    let game_config;
    let kwg;
    if args.len() <= 1 {
        return Err("need argument".into());
    } else if args[1] == "gen-no" {
        game_config = std::sync::Arc::new(game_config::make_norwegian_game_config());
        kwg = std::sync::Arc::new(kwg::Kwg::from_bytes_alloc(&std::fs::read(
            "lexbin/NSF20.kwg",
        )?));
    } else if args[1] == "gen-de" {
        game_config = std::sync::Arc::new(game_config::make_german_game_config());
        kwg = std::sync::Arc::new(kwg::Kwg::from_bytes_alloc(&std::fs::read(
            "lexbin/RD28.kwg",
        )?));
    } else {
        return Err("invalid argument".into());
    }
    let klv = std::sync::Arc::new(klv::Klv::from_bytes_alloc(klv::EMPTY_KLV_BYTES));
    let player_aliases = std::sync::Arc::new(
        (1..=game_config.num_players())
            .map(|x| std::sync::Arc::new(format!("p{}", x)))
            .collect::<Box<_>>(),
    );
    let num_threads = num_cpus::get();
    let num_games = 10_000_000;
    let num_processed_games = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let mut threads = vec![];
    let (tx, rx) = std::sync::mpsc::channel();
    for _ in 0..num_threads {
        let tx = tx.clone();
        let game_config = std::sync::Arc::clone(&game_config);
        let kwg = std::sync::Arc::clone(&kwg);
        let klv = std::sync::Arc::clone(&klv);
        let player_aliases = std::sync::Arc::clone(&player_aliases);
        let num_processed_games = std::sync::Arc::clone(&num_processed_games);
        threads.push(std::thread::spawn(move || {
            RNG.with(|rng| {
                let mut rng = &mut *rng.borrow_mut();
                let mut game_id = String::with_capacity(GAME_ID_LEN);
                let mut move_generator = movegen::KurniaMoveGenerator::new(&game_config);
                let mut game_state = game_state::GameState::new(&game_config);
                let mut cur_rack_ser = String::new();
                let mut aft_rack = Vec::with_capacity(game_config.rack_size() as usize);
                let mut aft_rack_ser = String::new();
                let mut play_fmt = String::new();
                let mut final_scores = vec![0; game_config.num_players() as usize];
                let mut num_bingos = vec![0; game_config.num_players() as usize];
                let mut num_moves;
                loop {
                    if num_processed_games.fetch_add(1, std::sync::atomic::Ordering::SeqCst)
                        >= num_games
                    {
                        num_processed_games.fetch_sub(1, std::sync::atomic::Ordering::Relaxed);
                        break;
                    }

                    num_moves = 0;
                    num_bingos.iter_mut().for_each(|m| *m = 0);
                    game_id.clear();
                    for _ in 0..GAME_ID_LEN {
                        game_id.push(*BASE57.choose(&mut rng).unwrap() as char);
                    }
                    let game_id = std::sync::Arc::new(game_id.clone());
                    let went_first = rng.gen_range(0..game_config.num_players());
                    game_state.reset_and_draw_tiles(&game_config, &mut rng);
                    game_state.turn = went_first;
                    loop {
                        num_moves += 1;

                        let board_snapshot = &movegen::BoardSnapshot {
                            board_tiles: &game_state.board_tiles,
                            game_config: &game_config,
                            kwg: &kwg,
                            klv: &klv,
                        };
                        game_state.players[game_state.turn as usize]
                            .rack
                            .sort_unstable();
                        let cur_rack = &game_state.current_player().rack;

                        move_generator.gen_moves_unfiltered(&movegen::GenMovesParams {
                            board_snapshot,
                            rack: &cur_rack,
                            max_gen: 1,
                            always_include_pass: false,
                        });

                        let plays = &mut move_generator.plays;
                        let play = &plays[0];
                        cur_rack_ser.clear();
                        for &tile in cur_rack.iter() {
                            cur_rack_ser.push_str(game_config.alphabet().from_rack(tile).unwrap());
                        }

                        aft_rack.clone_from(cur_rack);
                        match &play.play {
                            movegen::Play::Exchange { tiles } => {
                                game_state::use_tiles(&mut aft_rack, tiles.iter().copied())
                                    .unwrap();
                            }
                            movegen::Play::Place { word, .. } => {
                                game_state::use_tiles(
                                    &mut aft_rack,
                                    word.iter().filter_map(|&tile| {
                                        if tile != 0 {
                                            Some(tile & !((tile as i8) >> 7) as u8)
                                        } else {
                                            None
                                        }
                                    }),
                                )
                                .unwrap();
                            }
                        }
                        aft_rack.sort_unstable();
                        aft_rack_ser.clear();
                        for &tile in aft_rack.iter() {
                            aft_rack_ser.push_str(game_config.alphabet().from_rack(tile).unwrap());
                        }

                        play_fmt.clear();
                        match &play.play {
                            movegen::Play::Exchange { tiles } => {
                                if tiles.is_empty() {
                                    write!(play_fmt, "(Pass)").unwrap();
                                } else {
                                    let alphabet = game_config.alphabet();
                                    write!(play_fmt, "(exch ").unwrap();
                                    for &tile in tiles.iter() {
                                        write!(play_fmt, "{}", alphabet.from_rack(tile).unwrap())
                                            .unwrap();
                                    }
                                    write!(play_fmt, ")").unwrap();
                                }
                            }
                            movegen::Play::Place {
                                down,
                                lane,
                                idx,
                                word,
                                ..
                            } => {
                                let alphabet = game_config.alphabet();
                                if *down {
                                    write!(play_fmt, "{}{} ", display::column(*lane), idx + 1)
                                        .unwrap();
                                } else {
                                    write!(play_fmt, "{}{} ", lane + 1, display::column(*idx))
                                        .unwrap();
                                }
                                for &tile in word.iter() {
                                    if tile == 0 {
                                        write!(play_fmt, ".").unwrap();
                                    } else {
                                        write!(play_fmt, "{}", alphabet.from_board(tile).unwrap())
                                            .unwrap();
                                    }
                                }
                            }
                        }

                        let play_score = match &play.play {
                            movegen::Play::Exchange { .. } => 0,
                            movegen::Play::Place { score, .. } => *score,
                        };

                        let tiles_played = match &play.play {
                            movegen::Play::Exchange { tiles } => tiles.len(),
                            movegen::Play::Place { word, .. } => {
                                word.iter().filter(|&&tile| tile != 0).count()
                            }
                        };

                        match &play.play {
                            movegen::Play::Exchange { .. } => {}
                            movegen::Play::Place { .. } => {
                                if tiles_played >= game_config.rack_size() as usize {
                                    num_bingos[game_state.turn as usize] += 1;
                                }
                            }
                        };

                        let old_bag_len = game_state.bag.0.len();
                        game_state.play(&game_config, &mut rng, &play.play).unwrap();

                        let old_turn = game_state.turn;
                        game_state.next_turn();

                        match game_state.check_game_ended(&game_config, &mut final_scores) {
                            game_state::CheckGameEnded::PlayedOut
                            | game_state::CheckGameEnded::ZeroScores => {
                                tx.send(CSVRow::Log((
                                    SerializeArc(std::sync::Arc::clone(
                                        &player_aliases[old_turn as usize],
                                    )),
                                    SerializeArc(std::sync::Arc::clone(&game_id)),
                                    num_moves,
                                    cur_rack_ser.clone(),
                                    play_fmt.clone(),
                                    play_score,
                                    final_scores[old_turn as usize],
                                    tiles_played,
                                    aft_rack_ser.clone(),
                                    format!("{:.3}", play.equity),
                                    old_bag_len,
                                    game_state.players[game_state.turn as usize].score,
                                )))
                                .unwrap();
                                tx.send(CSVRow::Game((
                                    SerializeArc(std::sync::Arc::clone(&game_id)),
                                    final_scores.clone(),
                                    num_bingos.clone(),
                                    SerializeArc(std::sync::Arc::clone(
                                        &player_aliases[went_first as usize],
                                    )),
                                )))
                                .unwrap();
                                break;
                            }
                            game_state::CheckGameEnded::NotEnded => {}
                        }

                        tx.send(CSVRow::Log((
                            SerializeArc(std::sync::Arc::clone(&player_aliases[old_turn as usize])),
                            SerializeArc(std::sync::Arc::clone(&game_id)),
                            num_moves,
                            cur_rack_ser.clone(),
                            play_fmt.clone(),
                            play_score,
                            game_state.players[old_turn as usize].score,
                            tiles_played,
                            aft_rack_ser.clone(),
                            format!("{:.3}", play.equity),
                            old_bag_len,
                            game_state.players[game_state.turn as usize].score,
                        )))
                        .unwrap();
                    }
                }
            })
        }));
    }
    drop(tx);

    let epoch_secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let run_identifier = format!("log-{:08x}", epoch_secs);
    println!("logging to {}", run_identifier);
    let mut csv_log = csv::Writer::from_path(run_identifier.to_string())?;
    let mut csv_game = csv::Writer::from_path(format!("games-{}", run_identifier))?;
    csv_log.serialize((
        "playerID",
        "gameID",
        "turn",
        "rack",
        "play",
        "score",
        "totalscore",
        "tilesplayed",
        "leave",
        "equity",
        "tilesremaining",
        "oppscore",
    ))?;
    csv_game.serialize((
        "gameID",
        player_aliases
            .iter()
            .map(|x| format!("{}_score", x))
            .collect::<Box<_>>(),
        player_aliases
            .iter()
            .map(|x| format!("{}_bingos", x))
            .collect::<Box<_>>(),
        "first",
    ))?;
    let mut completed_games = 0u64;
    let mut completed_moves = 0u64;
    let t0 = std::time::Instant::now();
    let mut tick_periods = move_picker::Periods(0);
    for row in rx.iter() {
        match row {
            CSVRow::Log(r) => {
                csv_log.serialize(r)?;
                completed_moves += 1;
            }
            CSVRow::Game(r) => {
                csv_game.serialize(r)?;
                completed_games += 1;
                let elapsed_time_ms = t0.elapsed().as_millis() as u64;
                if tick_periods.update(elapsed_time_ms / 1000) {
                    println!(
                        "After {} seconds, have logged {} games ({} moves) into {}",
                        tick_periods.0, completed_games, completed_moves, run_identifier
                    );
                }
            }
        }
    }
    let elapsed_time_ms = t0.elapsed().as_millis() as u64;
    println!(
        "After {} seconds, have logged {} games ({} moves) into {}",
        elapsed_time_ms / 1000,
        completed_games,
        completed_moves,
        run_identifier
    );

    for thread in threads {
        if let Err(e) = thread.join() {
            println!("{:?}", e);
        }
    }

    Ok(())
}
