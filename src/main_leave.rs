// Copyright (C) 2020-2024 Andy Kurnia.

use rand::prelude::*;
use std::fmt::Write;
use std::io::Write as _;
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

static USED_STDOUT: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

// support "-" to mean stdout.
fn make_writer(filename: &str) -> Result<Box<dyn std::io::Write>, std::io::Error> {
    Ok(if filename == "-" {
        USED_STDOUT.store(true, std::sync::atomic::Ordering::Relaxed);
        Box::new(std::io::stdout())
    } else {
        Box::new(std::fs::File::create(filename)?)
    })
}

// when using "-" as output filename, print things to stderr.
fn boxed_stdout_or_stderr() -> Box<dyn std::io::Write> {
    if USED_STDOUT.load(std::sync::atomic::Ordering::Relaxed) {
        Box::new(std::io::stderr()) as Box<dyn std::io::Write>
    } else {
        Box::new(std::io::stdout())
    }
}

// support "-" to mean stdin.
fn make_reader(filename: &str) -> Result<Box<dyn std::io::Read>, std::io::Error> {
    Ok(if filename == "-" {
        Box::new(std::io::stdin())
    } else {
        Box::new(std::fs::File::open(filename)?)
    })
}

// slower than std::fs::read because it cannot preallocate the correct size.
fn read_to_end(reader: &mut Box<dyn std::io::Read>) -> Result<Vec<u8>, std::io::Error> {
    let mut v = Vec::new();
    reader.read_to_end(&mut v)?;
    Ok(v)
}

fn do_lang<GameConfigMaker: Fn() -> game_config::GameConfig>(
    args: &[String],
    language_name: &str,
    make_game_config: GameConfigMaker,
) -> error::Returns<bool> {
    match args[1].strip_prefix(language_name) {
        Some(args1_suffix) => match args1_suffix {
            "-autoplay" => {
                let args3 = if args.len() > 3 { &args[3] } else { "-" };
                let args4 = if args.len() > 4 { &args[4] } else { "-" };
                let num_games = if args.len() > 5 {
                    u64::from_str(&args[5])?
                } else {
                    1_000_000
                };
                let kwg = kwg::Kwg::from_bytes_alloc(&read_to_end(&mut make_reader(&args[2])?)?);
                let arc_klv0 = if args3 == "-" {
                    std::sync::Arc::new(klv::Klv::from_bytes_alloc(klv::EMPTY_KLV_BYTES))
                } else {
                    std::sync::Arc::new(klv::Klv::from_bytes_alloc(&std::fs::read(args3)?))
                };
                let arc_klv1 = if args3 == args4 {
                    std::sync::Arc::clone(&arc_klv0)
                } else if args4 == "-" {
                    std::sync::Arc::new(klv::Klv::from_bytes_alloc(klv::EMPTY_KLV_BYTES))
                } else {
                    std::sync::Arc::new(klv::Klv::from_bytes_alloc(&std::fs::read(args4)?))
                };
                generate_autoplay_logs(make_game_config(), kwg, arc_klv0, arc_klv1, num_games)?;
                Ok(true)
            }
            "-summarize" => {
                generate_summary(
                    make_game_config(),
                    make_reader(&args[2])?,
                    csv::Writer::from_writer(make_writer(&args[3])?),
                )?;
                Ok(true)
            }
            "-generate-no-smooth" => {
                generate_leaves::<_, _, false>(
                    make_game_config(),
                    csv::ReaderBuilder::new()
                        .has_headers(false)
                        .from_reader(make_reader(&args[2])?),
                    csv::Writer::from_writer(make_writer(&args[3])?),
                )?;
                Ok(true)
            }
            "-generate" => {
                generate_leaves::<_, _, true>(
                    make_game_config(),
                    csv::ReaderBuilder::new()
                        .has_headers(false)
                        .from_reader(make_reader(&args[2])?),
                    csv::Writer::from_writer(make_writer(&args[3])?),
                )?;
                Ok(true)
            }
            _ => Ok(false),
        },
        None => Ok(false),
    }
}

fn main() -> error::Returns<()> {
    let args = std::env::args().collect::<Vec<_>>();
    if args.len() <= 1 {
        println!(
            "args:
  english-autoplay NWL18.kwg leave0.klv leave1.klv 1000000
    autoplay 1000000 games, logs to a pair of csv.
    (changing output filenames needs recompile.)
    if leave is \"-\" or omitted, uses no leave.
    number of games is optional.
  english-summarize logfile summary.csv
    summarize logfile into summary.csv
  english-generate-no-smooth summary.csv leaves.csv
    generate leaves (no smoothing)
  english-generate summary.csv leaves.csv
    generate leaves (with smoothing)
  (english can also be catalan, french, german, norwegian, polish, slovene,
    spanish, yupik, super-english, super-catalan)
  jumbled-english-autoplay NWL18.kad leave0.klv leave1.klv 1000
    (all also take jumbled- prefix, including jumbled-super-;
    note that jumbled autoplay requires .kad instead of .kwg)
input/output files can be \"-\" (not advisable for binary files).
for english-autoplay only the kwg can come from \"-\"."
        );
        Ok(())
    } else {
        let t0 = std::time::Instant::now();
        if do_lang(&args, "english", game_config::make_english_game_config)?
            || do_lang(
                &args,
                "jumbled-english",
                game_config::make_jumbled_english_game_config,
            )?
            || do_lang(
                &args,
                "super-english",
                game_config::make_super_english_game_config,
            )?
            || do_lang(
                &args,
                "jumbled-super-english",
                game_config::make_jumbled_super_english_game_config,
            )?
            || do_lang(&args, "catalan", game_config::make_catalan_game_config)?
            || do_lang(
                &args,
                "jumbled-catalan",
                game_config::make_jumbled_catalan_game_config,
            )?
            || do_lang(
                &args,
                "super-catalan",
                game_config::make_super_catalan_game_config,
            )?
            || do_lang(
                &args,
                "jumbled-super-catalan",
                game_config::make_jumbled_super_catalan_game_config,
            )?
            || do_lang(&args, "french", game_config::make_french_game_config)?
            || do_lang(
                &args,
                "jumbled-french",
                game_config::make_jumbled_french_game_config,
            )?
            || do_lang(&args, "german", game_config::make_german_game_config)?
            || do_lang(
                &args,
                "jumbled-german",
                game_config::make_jumbled_german_game_config,
            )?
            || do_lang(&args, "norwegian", game_config::make_norwegian_game_config)?
            || do_lang(
                &args,
                "jumbled-norwegian",
                game_config::make_jumbled_norwegian_game_config,
            )?
            || do_lang(&args, "polish", game_config::make_polish_game_config)?
            || do_lang(
                &args,
                "jumbled-polish",
                game_config::make_jumbled_polish_game_config,
            )?
            || do_lang(&args, "slovene", game_config::make_slovene_game_config)?
            || do_lang(
                &args,
                "jumbled-slovene",
                game_config::make_jumbled_slovene_game_config,
            )?
            || do_lang(&args, "spanish", game_config::make_spanish_game_config)?
            || do_lang(
                &args,
                "jumbled-spanish",
                game_config::make_jumbled_spanish_game_config,
            )?
            || do_lang(&args, "yupik", game_config::make_yupik_game_config)?
            || do_lang(
                &args,
                "jumbled-yupik",
                game_config::make_jumbled_yupik_game_config,
            )?
        {
        } else {
            return Err("invalid argument".into());
        }
        writeln!(boxed_stdout_or_stderr(), "time taken: {:?}", t0.elapsed())?;
        Ok(())
    }
}

fn generate_autoplay_logs(
    game_config: game_config::GameConfig,
    kwg: kwg::Kwg,
    arc_klv0: std::sync::Arc<klv::Klv>,
    arc_klv1: std::sync::Arc<klv::Klv>,
    num_games: u64,
) -> error::Returns<()> {
    let game_config = std::sync::Arc::new(game_config);
    let kwg = std::sync::Arc::new(kwg);
    let player_aliases = std::sync::Arc::new(
        (1..=game_config.num_players())
            .map(|x| format!("p{x}"))
            .collect::<Box<_>>(),
    );
    let num_threads = num_cpus::get();
    let num_processed_games = std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0));
    let mut threads = vec![];

    let epoch_secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let run_identifier = std::sync::Arc::new(format!("log-{epoch_secs:08x}"));
    println!("logging to {run_identifier}");
    let mut csv_log = csv::Writer::from_path(run_identifier.to_string())?;
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
    let csv_log_writer = csv_log.into_inner()?;
    let mut csv_game = csv::Writer::from_path(format!("games-{run_identifier}"))?;
    csv_game.serialize((
        "gameID",
        player_aliases
            .iter()
            .map(|x| format!("{x}_score"))
            .collect::<Box<_>>(),
        player_aliases
            .iter()
            .map(|x| format!("{x}_bingos"))
            .collect::<Box<_>>(),
        "first",
    ))?;
    let csv_game_writer = csv_game.into_inner()?;
    let completed_games = std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0));
    let logged_games = std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0));
    let completed_moves = std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0));
    let t0 = std::time::Instant::now();
    let tick_periods = move_picker::Periods(0);
    struct MutexedStuffs {
        csv_game_writer: std::fs::File,
        csv_log_writer: std::fs::File,
        tick_periods: move_picker::Periods,
    }
    let mutexed_stuffs = std::sync::Arc::new(std::sync::Mutex::new(MutexedStuffs {
        csv_game_writer,
        csv_log_writer,
        tick_periods,
    }));

    for _ in 0..num_threads {
        let game_config = std::sync::Arc::clone(&game_config);
        let kwg = std::sync::Arc::clone(&kwg);
        let arc_klv0 = std::sync::Arc::clone(&arc_klv0);
        let arc_klv1 = std::sync::Arc::clone(&arc_klv1);
        let player_aliases = std::sync::Arc::clone(&player_aliases);
        let num_processed_games = std::sync::Arc::clone(&num_processed_games);
        let run_identifier = std::sync::Arc::clone(&run_identifier);
        let completed_games = std::sync::Arc::clone(&completed_games);
        let logged_games = std::sync::Arc::clone(&logged_games);
        let completed_moves = std::sync::Arc::clone(&completed_moves);
        let mutexed_stuffs = std::sync::Arc::clone(&mutexed_stuffs);
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
                let mut equity_fmt = String::new();
                let mut final_scores = vec![0; game_config.num_players() as usize];
                let mut num_bingos = vec![0; game_config.num_players() as usize];
                let mut num_moves;
                let mut num_batched_games_here = 0;
                let mut batched_csv_log = csv::Writer::from_writer(Vec::new());
                let mut batched_csv_game = csv::Writer::from_writer(Vec::new());
                loop {
                    if num_processed_games.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
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
                            rack: cur_rack,
                            max_gen: 1,
                            always_include_pass: false,
                        });

                        let plays = &mut move_generator.plays;
                        let play = &plays[0];
                        cur_rack_ser.clear();
                        for &tile in cur_rack.iter() {
                            cur_rack_ser.push_str(game_config.alphabet().of_rack(tile).unwrap());
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
                            aft_rack_ser.push_str(game_config.alphabet().of_rack(tile).unwrap());
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
                                        write!(play_fmt, "{}", alphabet.of_rack(tile).unwrap())
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
                                        write!(play_fmt, "{}", alphabet.of_board(tile).unwrap())
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
                        let new_turn = game_state.turn;
                        game_state.turn = old_turn;

                        equity_fmt.clear();
                        write!(equity_fmt, "{:.3}", play.equity).unwrap();

                        match game_state.check_game_ended(&game_config, &mut final_scores) {
                            game_state::CheckGameEnded::PlayedOut
                            | game_state::CheckGameEnded::ZeroScores => {
                                let completed_moves = completed_moves
                                    .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                                completed_games.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                                batched_csv_log
                                    .serialize((
                                        &player_aliases[old_turn as usize],
                                        &game_id,
                                        num_moves,
                                        &cur_rack_ser,
                                        &play_fmt,
                                        play_score,
                                        final_scores[old_turn as usize],
                                        tiles_played,
                                        &aft_rack_ser,
                                        &equity_fmt,
                                        old_bag_len,
                                        final_scores[new_turn as usize],
                                    ))
                                    .unwrap();
                                batched_csv_game
                                    .serialize((
                                        &game_id,
                                        &final_scores,
                                        &num_bingos,
                                        &player_aliases[went_first as usize],
                                    ))
                                    .unwrap();
                                num_batched_games_here += 1;
                                if num_batched_games_here >= 100 {
                                    let logged_games = logged_games.fetch_add(
                                        num_batched_games_here,
                                        std::sync::atomic::Ordering::Relaxed,
                                    ) + num_batched_games_here;
                                    num_batched_games_here = 0;
                                    let mut batched_csv_log_buf =
                                        batched_csv_log.into_inner().unwrap();
                                    let mut batched_csv_game_buf =
                                        batched_csv_game.into_inner().unwrap();
                                    let elapsed_time_secs = t0.elapsed().as_secs();
                                    let tick_changed = {
                                        let mut mutex_guard = mutexed_stuffs.lock().unwrap();
                                        mutex_guard
                                            .csv_log_writer
                                            .write_all(&batched_csv_log_buf)
                                            .unwrap();
                                        mutex_guard
                                            .csv_game_writer
                                            .write_all(&batched_csv_game_buf)
                                            .unwrap();
                                        mutex_guard.tick_periods.update(elapsed_time_secs)
                                    };
                                    if tick_changed {
                                        println!(
                                        "After {elapsed_time_secs} seconds, have logged {logged_games} games ({completed_moves} moves) into {run_identifier}"
                                    );
                                    }
                                    batched_csv_log_buf.clear();
                                    batched_csv_log = csv::Writer::from_writer(batched_csv_log_buf);
                                    batched_csv_game_buf.clear();
                                    batched_csv_game =
                                        csv::Writer::from_writer(batched_csv_game_buf);
                                }
                                break;
                            }
                            game_state::CheckGameEnded::NotEnded => {}
                        }

                        batched_csv_log
                            .serialize((
                                &player_aliases[old_turn as usize],
                                &game_id,
                                num_moves,
                                &cur_rack_ser,
                                &play_fmt,
                                play_score,
                                game_state.players[old_turn as usize].score,
                                tiles_played,
                                &aft_rack_ser,
                                &equity_fmt,
                                old_bag_len,
                                game_state.players[new_turn as usize].score,
                            ))
                            .unwrap();
                        completed_moves.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                        game_state.turn = new_turn;
                    }
                }

                let batched_csv_log_buf = batched_csv_log.into_inner().unwrap();
                let batched_csv_game_buf = batched_csv_game.into_inner().unwrap();
                let mut mutex_guard = mutexed_stuffs.lock().unwrap();
                mutex_guard
                    .csv_log_writer
                    .write_all(&batched_csv_log_buf)
                    .unwrap();
                mutex_guard
                    .csv_game_writer
                    .write_all(&batched_csv_game_buf)
                    .unwrap();
            })
        }));
    }

    for thread in threads {
        if let Err(e) = thread.join() {
            println!("{e:?}");
        }
    }

    println!(
        "After {} seconds, have logged {} games ({} moves) into {}",
        t0.elapsed().as_secs(),
        completed_games.load(std::sync::atomic::Ordering::Relaxed),
        completed_moves.load(std::sync::atomic::Ordering::Relaxed),
        run_identifier
    );

    Ok(())
}

// handles the equivalent of '?', A-Z
fn parse_rack(
    alphabet_reader: &alphabet::AlphabetReader,
    s: &str,
    v: &mut Vec<u8>,
) -> Result<(), Box<dyn std::error::Error>> {
    alphabet_reader.set_word(s, v)
}

struct Cumulate {
    equity: f64,
    count: u64,
}

fn generate_summary<Readable: std::io::Read, W: std::io::Write>(
    game_config: game_config::GameConfig,
    f: Readable,
    mut csv_out: csv::Writer<W>,
) -> error::Returns<()> {
    let mut stdout_or_stderr = boxed_stdout_or_stderr();
    let rack_reader = alphabet::AlphabetReader::new_for_racks(game_config.alphabet());
    let mut csv_reader = csv::ReaderBuilder::new().has_headers(false).from_reader(f);
    let mut rack_bytes = Vec::new();
    let mut full_rack_map = fash::MyHashMap::<bites::Bites, Cumulate>::default();
    // playerID,gameID,turn,rack,play,score,totalscore,tilesplayed,leave,equity,tilesremaining,oppscore
    // 0       ,1     ,2   ,3   ,4   ,5    ,6         ,7          ,8    ,9     ,10            ,11
    let t0 = std::time::Instant::now();
    let mut tick_periods = move_picker::Periods(0);
    let mut row_count = 0u64;
    for (record_num, result) in csv_reader.records().enumerate() {
        let record = result?;
        if let Err(e) = (|| -> error::Returns<()> {
            if i16::from_str(&record[10])? >= 1 {
                let equity = f32::from_str(&record[9])? as f64;
                //let score = i16::from_str(&record[5])? as i64;
                parse_rack(&rack_reader, &record[3], &mut rack_bytes)?;
                rack_bytes.sort_unstable();
                row_count += 1;
                if let Some(v) = full_rack_map.get_mut(&rack_bytes[..]) {
                    *v = Cumulate {
                        equity: v.equity + equity,
                        count: v.count + 1,
                    }
                } else {
                    full_rack_map.insert(rack_bytes[..].into(), Cumulate { equity, count: 1 });
                }
                let elapsed_time_secs = t0.elapsed().as_secs();
                if tick_periods.update(elapsed_time_secs) {
                    writeln!(
                        stdout_or_stderr,
                        "After {elapsed_time_secs} seconds, have read {row_count} rows"
                    )?;
                }
            }
            Ok(())
        })() {
            writeln!(
                stdout_or_stderr,
                "parsing {}: {:?}: {:?}",
                record_num + 1,
                record,
                e
            )?;
        }
    }
    drop(csv_reader);
    let total_equity = full_rack_map.values().fold(0.0, |a, x| a + x.equity);
    writeln!(
        stdout_or_stderr,
        "{} records, {} unique racks",
        row_count,
        full_rack_map.len()
    )?;

    let mut kv = full_rack_map.into_iter().collect::<Vec<_>>();
    kv.sort_unstable_by(|a, b| a.0.len().cmp(&b.0.len()).then_with(|| a.0.cmp(&b.0)));

    let mut cur_rack_ser = String::new();
    csv_out.serialize(("", total_equity, row_count))?;
    for (k, fv) in kv.iter() {
        cur_rack_ser.clear();
        for &tile in k.iter() {
            cur_rack_ser.push_str(game_config.alphabet().of_rack(tile).unwrap());
        }
        csv_out.serialize((&cur_rack_ser, fv.equity, fv.count))?;
    }

    Ok(())
}

struct ExchangeEnv<'a, FoundExchangeMove: FnMut(&[u8])> {
    found_exchange_move: FoundExchangeMove,
    rack_tally: &'a mut [u8],
    min_len: u8,
    max_len: u8,
    exchange_buffer: &'a mut Vec<u8>,
}

fn generate_exchanges<FoundExchangeMove: FnMut(&[u8])>(
    env: &mut ExchangeEnv<'_, FoundExchangeMove>,
    idx: u8,
) {
    if env.exchange_buffer.len() >= env.min_len as usize {
        (env.found_exchange_move)(env.exchange_buffer);
    }
    if env.exchange_buffer.len() < env.max_len as usize {
        for i in idx as usize..env.rack_tally.len() {
            if env.rack_tally[i] > 0 {
                env.rack_tally[i] -= 1;
                env.exchange_buffer.push(i as u8);
                generate_exchanges(env, i as u8);
                env.exchange_buffer.pop();
                env.rack_tally[i] += 1;
            }
        }
    }
}

fn generate_neighbors<FoundNeighbor: FnMut(&[u8])>(
    freqs: &[u8],
    idx: u8,
    insed: bool,
    deled: bool,
    v: &mut Vec<u8>,
    found_neighbor: &mut FoundNeighbor,
) {
    if idx as usize >= freqs.len() {
        if insed == deled {
            found_neighbor(v);
        }
    } else {
        let ol = v.len();
        let freq = freqs[idx as usize];
        if freq > 0 {
            for _ in 1..freq {
                v.push(idx);
            }
            if idx != 0 && !deled {
                generate_neighbors(freqs, idx + 1, insed, true, v, found_neighbor);
            }
            v.push(idx);
        }
        generate_neighbors(freqs, idx + 1, insed, deled, v, found_neighbor);
        if idx != 0 && !insed {
            v.push(idx);
            generate_neighbors(freqs, idx + 1, true, deled, v, found_neighbor);
        }
        v.truncate(ol);
    }
}

fn generate_leaves<Readable: std::io::Read, W: std::io::Write, const DO_SMOOTHING: bool>(
    game_config: game_config::GameConfig,
    mut csv_in: csv::Reader<Readable>,
    mut csv_out: csv::Writer<W>,
) -> error::Returns<()> {
    let mut stdout_or_stderr = boxed_stdout_or_stderr();
    let mut rack_tally = vec![0u8; game_config.alphabet().len() as usize];
    let mut exchange_buffer = Vec::with_capacity(game_config.rack_size() as usize);
    let mut rack_bytes = Vec::new();
    let rack_reader = alphabet::AlphabetReader::new_for_racks(game_config.alphabet());
    let mut full_rack_map = fash::MyHashMap::<bites::Bites, Cumulate>::default();
    let t0 = std::time::Instant::now();
    let mut tick_periods = move_picker::Periods(0);
    let mut results = csv_in.records();
    let record = results.next().unwrap()?;
    if !record[0].is_empty() {
        return Err("invalid input file".into());
    }
    let total_equity = f64::from_str(&record[1])?;
    let row_count = u64::from_str(&record[2])?;
    for result in results {
        let record = result?;
        parse_rack(&rack_reader, &record[0], &mut rack_bytes)?;
        full_rack_map.insert(
            rack_bytes[..].into(),
            Cumulate {
                equity: f64::from_str(&record[1])?,
                count: u64::from_str(&record[2])?,
            },
        );
    }
    drop(csv_in);

    let mut subrack_map = fash::MyHashMap::<bites::Bites, Cumulate>::default();
    for (idx, (k, fv)) in full_rack_map.iter().enumerate() {
        rack_tally.iter_mut().for_each(|m| *m = 0);
        k.iter().for_each(|&tile| rack_tally[tile as usize] += 1);
        generate_exchanges(
            &mut ExchangeEnv {
                found_exchange_move: |subrack_bytes: &[u8]| {
                    if let Some(v) = subrack_map.get_mut(subrack_bytes) {
                        *v = Cumulate {
                            equity: v.equity + fv.equity,
                            count: v.count + fv.count,
                        }
                    } else {
                        subrack_map.insert(
                            subrack_bytes.into(),
                            Cumulate {
                                equity: fv.equity,
                                count: fv.count,
                            },
                        );
                    }
                },
                rack_tally: &mut rack_tally,
                min_len: 1,
                max_len: game_config.rack_size() - 1,
                exchange_buffer: &mut exchange_buffer,
            },
            0,
        );
        let elapsed_time_secs = t0.elapsed().as_secs();
        if tick_periods.update(elapsed_time_secs) {
            writeln!(
                stdout_or_stderr,
                "After {} seconds, have processed {} racks into {} unique subracks",
                elapsed_time_secs,
                idx + 1,
                subrack_map.len(),
            )?;
        }
    }
    writeln!(stdout_or_stderr, "{} unique subracks", subrack_map.len())?;

    let threshold_count = if DO_SMOOTHING {
        let total_count = subrack_map.values().fold(0, |a, x| a + x.count);
        (total_count as f64).cbrt().ceil() as u64 // inaccurate after 2^53
    } else {
        0
    };
    let mean_equity = total_equity / row_count as f64;
    let mut ev_map = fash::MyHashMap::<bites::Bites, _>::default();
    let mut alphabet_freqs = (0..game_config.alphabet().len())
        .map(|tile| game_config.alphabet().freq(tile))
        .collect::<Box<_>>();
    let mut neighbor_buffer = if DO_SMOOTHING {
        Vec::with_capacity(game_config.rack_size() as usize)
    } else {
        Vec::new()
    };
    let mut num_smoothed = 0u64;
    generate_exchanges(
        &mut ExchangeEnv {
            found_exchange_move: |rack_bytes: &[u8]| {
                let mut new_v = if let Some(v) = subrack_map.get(rack_bytes) {
                    if !DO_SMOOTHING || v.count >= threshold_count {
                        v.equity / v.count as f64 - mean_equity
                    } else {
                        f64::NAN
                    }
                } else {
                    f64::NAN
                };
                if DO_SMOOTHING && new_v.is_nan() {
                    rack_tally.iter_mut().for_each(|m| *m = 0);
                    rack_bytes
                        .iter()
                        .for_each(|&tile| rack_tally[tile as usize] += 1);
                    let mut equity = 0.0f64;
                    let mut count = 0u64;
                    generate_neighbors(
                        &rack_tally,
                        0,
                        false,
                        false,
                        &mut neighbor_buffer,
                        &mut |neighbor_bytes: &[u8]| {
                            if let Some(v) = subrack_map.get(neighbor_bytes) {
                                equity += v.equity;
                                count += v.count;
                            }
                        },
                    );
                    if count > 0 {
                        new_v = equity / count as f64 - mean_equity;
                        num_smoothed += 1;
                    }
                }
                ev_map.insert(rack_bytes.into(), new_v);
                let elapsed_time_secs = t0.elapsed().as_secs();
                if tick_periods.update(elapsed_time_secs) {
                    writeln!(
                        stdout_or_stderr,
                        "After {} seconds, have processed {} subracks and smoothed {}",
                        elapsed_time_secs,
                        ev_map.len(),
                        num_smoothed,
                    )
                    .unwrap();
                }
            },
            rack_tally: &mut alphabet_freqs,
            min_len: 1,
            max_len: game_config.rack_size() - 1,
            exchange_buffer: &mut exchange_buffer,
        },
        0,
    );
    writeln!(
        stdout_or_stderr,
        "After {} seconds, have processed {} subracks and smoothed {}",
        t0.elapsed().as_secs(),
        ev_map.len(),
        num_smoothed,
    )?;
    let mut num_filled_in = 0u64;

    let mut subrack_bytes = Vec::with_capacity(game_config.rack_size() as usize - 1);
    for len_to_complete in 2..game_config.rack_size() {
        let len_minus_one = len_to_complete as usize - 1;
        generate_exchanges(
            &mut ExchangeEnv {
                found_exchange_move: |rack_bytes: &[u8]| {
                    if ev_map.get(rack_bytes).unwrap_or(&f64::NAN).is_nan() {
                        let mut vn = 0.0f64;
                        let mut vd = 0i64;
                        let mut vmax = 0.0f64;
                        let mut vpos = 0i64;
                        let mut vmin = 0.0f64;
                        let mut vneg = 0i64;
                        let mut process_subrack = |subrack_bytes: &[u8]| {
                            let v = *ev_map.get(subrack_bytes).unwrap_or(&f64::NAN);
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
                        };
                        subrack_bytes.clear();
                        subrack_bytes.extend_from_slice(rack_bytes);
                        process_subrack(&subrack_bytes[..len_minus_one]);
                        for which_tile in (0..len_minus_one).rev() {
                            let c1 = subrack_bytes[which_tile];
                            let c2 = subrack_bytes[len_minus_one];
                            if c1 != c2 {
                                subrack_bytes[which_tile] = c2;
                                subrack_bytes[len_minus_one] = c1;
                                process_subrack(&subrack_bytes[..len_minus_one]);
                            }
                        }
                        if vd > 0 {
                            ev_map.insert(
                                rack_bytes.into(),
                                match vpos.cmp(&vneg) {
                                    std::cmp::Ordering::Greater => vmax,
                                    std::cmp::Ordering::Equal => vn / vd as f64,
                                    std::cmp::Ordering::Less => vmin,
                                },
                            );
                            num_filled_in += 1;
                        } else {
                            writeln!(
                                stdout_or_stderr,
                                "not enough samples to derive {rack_bytes:?}"
                            )
                            .unwrap();
                        }
                    }
                },
                rack_tally: &mut alphabet_freqs,
                min_len: len_to_complete,
                max_len: len_to_complete,
                exchange_buffer: &mut exchange_buffer,
            },
            0,
        );
    }
    writeln!(
        stdout_or_stderr,
        "After {} seconds, have processed {} subracks, smoothed {}, filled in {}",
        t0.elapsed().as_secs(),
        ev_map.len(),
        num_smoothed,
        num_filled_in,
    )?;

    let mut kv = ev_map.into_iter().collect::<Vec<_>>();
    kv.sort_unstable_by(|a, b| a.0.len().cmp(&b.0.len()).then_with(|| a.0.cmp(&b.0)));

    let mut cur_rack_ser = String::new();
    for (k, v) in kv.iter() {
        cur_rack_ser.clear();
        for &tile in k.iter() {
            cur_rack_ser.push_str(game_config.alphabet().of_rack(tile).unwrap());
        }
        csv_out.serialize((&cur_rack_ser, v))?;
        /*
        if let Some(orig_v) = subrack_map.get(k) {
            csv_out.serialize((&cur_rack_ser, v, orig_v.equity, orig_v.count))?;
        } else {
            csv_out.serialize((&cur_rack_ser, v, f64::NAN, 0))?;
        };
        */
    }

    Ok(())
}
