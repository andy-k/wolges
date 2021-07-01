// Copyright (C) 2020-2021 Andy Kurnia.

use rand::prelude::*;
use std::fmt::Write;
use std::str::FromStr;
use wolges::{
    alphabet, bites, display, error, fash, game_config, game_state, klv, kwg, move_picker, movegen,
};

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

fn do_lang<GameConfigMaker: Fn() -> game_config::GameConfig<'static>>(
    args: &[String],
    language_name: &str,
    make_game_config: GameConfigMaker,
) -> error::Returns<bool> {
    match args[1].strip_prefix(language_name) {
        Some(args1_suffix) => match args1_suffix {
            "-autoplay" => {
                let args3 = if args.len() > 3 { &args[3] } else { "-" };
                let args4 = if args.len() > 4 { &args[4] } else { "-" };
                let kwg = kwg::Kwg::from_bytes_alloc(&std::fs::read(&args[2])?);
                let arc_klv0 = if args3 == "-" {
                    std::sync::Arc::new(klv::Klv::from_bytes_alloc(klv::EMPTY_KLV_BYTES))
                } else {
                    std::sync::Arc::new(klv::Klv::from_bytes_alloc(&std::fs::read(&args3)?))
                };
                let arc_klv1 = if args3 == args4 {
                    std::sync::Arc::clone(&arc_klv0)
                } else if args4 == "-" {
                    std::sync::Arc::new(klv::Klv::from_bytes_alloc(klv::EMPTY_KLV_BYTES))
                } else {
                    std::sync::Arc::new(klv::Klv::from_bytes_alloc(&std::fs::read(&args4)?))
                };
                generate_autoplay_logs(make_game_config(), kwg, arc_klv0, arc_klv1)?;
                Ok(true)
            }
            "-generate" => {
                generate_leaves(
                    make_game_config(),
                    std::fs::File::open(&args[2])?,
                    csv::Writer::from_path(&args[3])?,
                )?;
                Ok(true)
            }
            _ => Ok(false),
        },
        None => Ok(false),
    }
}

pub fn main() -> error::Returns<()> {
    let args = std::env::args().collect::<Vec<_>>();
    if args.len() <= 1 {
        println!(
            "args:
  english-autoplay NWL18.kwg leave0.klv leave1.klv
    autoplay many games, logs to a pair of csv.
    (changing number of games or output filenames needs recompile.)
    if leave is \"-\" or omitted, uses no leave.
  english-generate logfile leaves.csv
    summarize logfile into leaves.csv
  (english can also be french, german, norwegian, polish, spanish)"
        );
        Ok(())
    } else {
        let t0 = std::time::Instant::now();
        if do_lang(
            &args,
            "english",
            game_config::make_common_english_game_config,
        )? || do_lang(&args, "french", game_config::make_french_game_config)?
            || do_lang(&args, "german", game_config::make_german_game_config)?
            || do_lang(&args, "norwegian", game_config::make_norwegian_game_config)?
            || do_lang(&args, "polish", game_config::make_polish_game_config)?
            || do_lang(&args, "spanish", game_config::make_spanish_game_config)?
        {
        } else {
            return Err("invalid argument".into());
        }
        println!("time taken: {:?}", t0.elapsed());
        Ok(())
    }
}

fn generate_autoplay_logs(
    game_config: game_config::GameConfig<'static>,
    kwg: kwg::Kwg,
    arc_klv0: std::sync::Arc<klv::Klv>,
    arc_klv1: std::sync::Arc<klv::Klv>,
) -> error::Returns<()> {
    let game_config = std::sync::Arc::new(game_config);
    let kwg = std::sync::Arc::new(kwg);
    let player_aliases = std::sync::Arc::new(
        (1..=game_config.num_players())
            .map(|x| std::sync::Arc::new(format!("p{}", x)))
            .collect::<Box<_>>(),
    );
    let num_threads = num_cpus::get();
    let num_games = 1_000_000;
    let num_processed_games = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let mut threads = vec![];
    let (tx, rx) = std::sync::mpsc::channel();
    for _ in 0..num_threads {
        let tx = tx.clone();
        let game_config = std::sync::Arc::clone(&game_config);
        let kwg = std::sync::Arc::clone(&kwg);
        let arc_klv0 = std::sync::Arc::clone(&arc_klv0);
        let arc_klv1 = std::sync::Arc::clone(&arc_klv1);
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
                            klv: if game_state.turn == 0 {
                                &arc_klv0
                            } else {
                                &arc_klv1
                            },
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

// handles the equivalent of '?', A-Z
fn parse_rack(
    alphabet_reader: &alphabet::AlphabetReader,
    s: &str,
    v: &mut Vec<u8>,
) -> Result<(), Box<dyn std::error::Error>> {
    v.clear();
    if !s.is_empty() {
        v.reserve(s.len());
        let sb = s.as_bytes();
        let mut ix = 0;
        while ix < sb.len() {
            if let Some((tile, end_ix)) = alphabet_reader.next_tile(sb, ix) {
                v.push(tile);
                ix = end_ix;
            } else {
                wolges::return_error!(format!("invalid tile after {:?} in {:?}", v, s));
            }
        }
    }
    Ok(())
}

fn generate_leaves<Readable: std::io::Read, W: std::io::Write>(
    game_config: game_config::GameConfig,
    f: Readable,
    mut csv_out: csv::Writer<W>,
) -> error::Returns<()> {
    struct ExchangeEnv<'a, FoundExchangeMove: FnMut(&[u8])> {
        found_exchange_move: FoundExchangeMove,
        rack_tally: &'a mut [u8],
        min_len: i8,
        max_len: i8,
        exchange_buffer: &'a mut Vec<u8>,
    }
    fn generate_exchanges<FoundExchangeMove: FnMut(&[u8])>(
        env: &mut ExchangeEnv<FoundExchangeMove>,
        mut idx: u8,
    ) {
        let rack_tally_len = env.rack_tally.len();
        while (idx as usize) < rack_tally_len && env.rack_tally[idx as usize] == 0 {
            idx += 1;
        }
        if idx as usize >= rack_tally_len {
            if env.exchange_buffer.len() >= env.min_len as usize {
                (env.found_exchange_move)(&env.exchange_buffer);
            }
            return;
        }
        let original_count = env.rack_tally[idx as usize];
        let vec_len = env.exchange_buffer.len();
        loop {
            generate_exchanges(env, idx + 1);
            if env.exchange_buffer.len() >= env.max_len as usize
                || env.rack_tally[idx as usize] == 0
            {
                break;
            }
            env.rack_tally[idx as usize] -= 1;
            env.exchange_buffer.push(idx);
        }
        env.rack_tally[idx as usize] = original_count;
        env.exchange_buffer.truncate(vec_len);
    }

    let game_config = std::sync::Arc::new(game_config);
    let mut rack_tally = vec![0u8; game_config.alphabet().len() as usize];
    let mut exchange_buffer = Vec::with_capacity(game_config.rack_size() as usize);
    let rack_reader = alphabet::AlphabetReader::new_for_racks(game_config.alphabet());
    let mut csv_reader = csv::ReaderBuilder::new().has_headers(false).from_reader(f);
    let mut rack_bytes = Vec::new();
    struct Cumulate {
        equity: f64,
        count: usize,
    }
    let mut full_rack_map = fash::MyHashMap::<bites::Bites, _>::default();
    // playerID,gameID,turn,rack,play,score,totalscore,tilesplayed,leave,equity,tilesremaining,oppscore
    // 0       ,1     ,2   ,3   ,4   ,5    ,6         ,7          ,8    ,9     ,10            ,11
    let t0 = std::time::Instant::now();
    let mut tick_periods = move_picker::Periods(0);
    let mut row_count = 0usize;
    for result in csv_reader.records() {
        let record = result?;
        if let Err(e) = (|| -> error::Returns<()> {
            if i16::from_str(&record[10])? >= 1 {
                let equity = f32::from_str(&record[9])? as f64;
                //let score = i16::from_str(&record[5])? as i64;
                parse_rack(&rack_reader, &record[3], &mut rack_bytes)?;
                rack_bytes.sort_unstable();
                row_count += 1;
                full_rack_map
                    .entry(rack_bytes[..].into())
                    .and_modify(|v: &mut Cumulate| {
                        *v = Cumulate {
                            equity: v.equity + equity,
                            count: v.count + 1,
                        }
                    })
                    .or_insert(Cumulate { equity, count: 1 });
                let elapsed_time_ms = t0.elapsed().as_millis() as u64;
                if tick_periods.update(elapsed_time_ms / 1000) {
                    println!(
                        "After {} seconds, have read {} rows",
                        tick_periods.0, row_count
                    );
                }
            }
            Ok(())
        })() {
            println!("parsing {:?}: {:?}", record, e);
        }
    }
    drop(csv_reader);
    let global_cumulate = Cumulate {
        equity: full_rack_map.values().fold(0.0, |a, x| a + x.equity),
        count: row_count,
    };
    println!(
        "{} records, {} unique racks",
        global_cumulate.count,
        full_rack_map.len()
    );
    let mut subrack_map = fash::MyHashMap::<bites::Bites, _>::default();
    for (idx, (k, fv)) in full_rack_map.iter().enumerate() {
        rack_tally.iter_mut().for_each(|m| *m = 0);
        k.iter().for_each(|&tile| rack_tally[tile as usize] += 1);
        generate_exchanges(
            &mut ExchangeEnv {
                found_exchange_move: |subrack_bytes: &[u8]| {
                    subrack_map
                        .entry(subrack_bytes[..].into())
                        .and_modify(|v: &mut Cumulate| {
                            *v = Cumulate {
                                equity: v.equity + fv.equity,
                                count: v.count + fv.count,
                            }
                        })
                        .or_insert(Cumulate {
                            equity: fv.equity,
                            count: fv.count,
                        });
                },
                rack_tally: &mut rack_tally,
                min_len: 1,
                max_len: game_config.rack_size() - 1,
                exchange_buffer: &mut exchange_buffer,
            },
            0,
        );
        let elapsed_time_ms = t0.elapsed().as_millis() as u64;
        if tick_periods.update(elapsed_time_ms / 1000) {
            println!(
                "After {} seconds, have processed {} racks into {} unique subracks",
                tick_periods.0,
                idx + 1,
                subrack_map.len(),
            );
        }
    }
    println!("{} unique subracks", subrack_map.len());

    let mean_equity = global_cumulate.equity / global_cumulate.count as f64;
    let mut ev_map = fash::MyHashMap::<bites::Bites, _>::default();
    let mut alphabet_freqs = (0..game_config.alphabet().len())
        .map(|tile| game_config.alphabet().freq(tile))
        .collect::<Box<_>>();
    let mut build_buffer = Vec::with_capacity(game_config.rack_size() as usize);
    generate_exchanges(
        &mut ExchangeEnv {
            found_exchange_move: |rack_bytes: &[u8]| {
                let k = rack_bytes.into();
                let new_v = if let Some(v) = subrack_map.get(&k) {
                    v.equity / (v.count as f64) - mean_equity
                } else {
                    f64::NAN
                };
                ev_map.insert(k, new_v);
            },
            rack_tally: &mut alphabet_freqs,
            min_len: 1,
            max_len: game_config.rack_size() - 1,
            exchange_buffer: &mut build_buffer,
        },
        0,
    );

    for len_to_complete in 2..game_config.rack_size() {
        generate_exchanges(
            &mut ExchangeEnv {
                found_exchange_move: |rack_bytes: &[u8]| {
                    let k: bites::Bites = rack_bytes.into();
                    if ev_map.get(&k).unwrap_or(&f64::NAN).is_nan() {
                        rack_tally.iter_mut().for_each(|m| *m = 0);
                        rack_bytes
                            .iter()
                            .for_each(|&tile| rack_tally[tile as usize] += 1);
                        let mut vn = 0.0f64;
                        let mut vd = 0i64;
                        let mut vmax = 0.0f64;
                        let mut vpos = 0i64;
                        let mut vmin = 0.0f64;
                        let mut vneg = 0i64;
                        generate_exchanges(
                            &mut ExchangeEnv {
                                found_exchange_move: |subrack_bytes: &[u8]| {
                                    let v =
                                        *ev_map.get(&subrack_bytes[..].into()).unwrap_or(&f64::NAN);
                                    if !v.is_nan() {
                                        vn += v;
                                        vd += 1;
                                        if v > 0.0 {
                                            if v > vmax {
                                                vmax = v;
                                            }
                                            vpos += 1;
                                        } else if v < 0.0 {
                                            if v < vmin {
                                                vmin = v;
                                            }
                                            vneg += 1;
                                        }
                                    }
                                },
                                rack_tally: &mut rack_tally,
                                min_len: len_to_complete - 1,
                                max_len: len_to_complete - 1,
                                exchange_buffer: &mut exchange_buffer,
                            },
                            0,
                        );
                        if vd > 0 {
                            ev_map.insert(
                                k,
                                match vpos.cmp(&vneg) {
                                    std::cmp::Ordering::Greater => vmax,
                                    std::cmp::Ordering::Equal => vn / vd as f64,
                                    std::cmp::Ordering::Less => vmin,
                                },
                            );
                        } else {
                            println!("not enough samples to derive {:?}", k);
                        }
                    }
                },
                rack_tally: &mut alphabet_freqs,
                min_len: len_to_complete,
                max_len: len_to_complete,
                exchange_buffer: &mut build_buffer,
            },
            0,
        );
    }

    let mut kv = ev_map.into_iter().collect::<Vec<_>>();
    kv.sort_unstable_by(|a, b| a.0.len().cmp(&b.0.len()).then(a.0.cmp(&b.0)));

    let mut cur_rack_ser = String::new();
    for (k, v) in kv.iter() {
        cur_rack_ser.clear();
        for &tile in k.iter() {
            cur_rack_ser.push_str(game_config.alphabet().from_rack(tile).unwrap());
        }
        csv_out.serialize((cur_rack_ser.clone(), v))?;
    }

    Ok(())
}