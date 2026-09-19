// Copyright (C) 2020-2026 Andy Kurnia.

use rand::prelude::*;
use std::fmt::Write;
use std::io::Write as _;
use std::str::FromStr;
use wolges::{
    alphabet, bites, build, census, display, equity, error, fash, game_config, game_state, klv,
    kwg, move_filter, move_picker, movegen, play_scorer, prob, simmer, stats, win_pct,
};

static BASE62: &[u8; 62] = b"\
0123456789\
ABCDEFGHIJKLMNOPQRSTUVWXYZ\
abcdefghijklmnopqrstuvwxyz\
";

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

fn run_stamp() -> String {
    let d = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    let ticks = (d.as_secs() << 16) | (d.subsec_nanos() as u64 * 65536 / 1_000_000_000);
    format!("{ticks:012x}")
}

fn claim_output_path(desired: &str) -> std::io::Result<String> {
    use std::fmt::Write as _;
    match std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(desired)
    {
        Ok(_) => return Ok(desired.to_owned()),
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
        Err(e) => return Err(e),
    }

    let dot = desired.rfind('.').unwrap_or(desired.len());
    let (stem, ext) = (&desired[..dot], &desired[dot..]);
    let mut buf = String::with_capacity(desired.len() + 4);
    for n in 1u32.. {
        buf.clear();
        let _ = write!(buf, "{stem}_{n}{ext}");
        match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&buf)
        {
            Ok(_) => {
                eprintln!("warning: {desired} already exists; writing {buf} instead");
                return Ok(buf);
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e),
        }
    }
    unreachable!()
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
    // dutch-big-autoplay
    if args[1]
        .strip_prefix(language_name)
        .is_some_and(|x| x.starts_with("-big"))
        && do_lang_kwg::<_, kwg::Node24>(args, &format!("{language_name}-big"), &make_game_config)?
    {
        return Ok(true);
    }
    do_lang_kwg::<_, kwg::Node22>(args, language_name, &make_game_config)
}

fn do_lang_kwg<GameConfigMaker: Fn() -> game_config::GameConfig, N: kwg::Node + Sync + Send>(
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
                let min_samples_per_rack = if args.len() > 6 {
                    u64::from_str(&args[6])?
                } else {
                    0
                };
                let seed = if args.len() > 7 {
                    Some(u64::from_str(&args[7])?)
                } else {
                    None
                };
                let kwg =
                    kwg::Kwg::<N>::from_bytes_alloc(&read_to_end(&mut make_reader(&args[2])?)?);
                let arc_klv0 = if args3 == "-" {
                    std::sync::Arc::new(klv::Klv::<kwg::Node22>::from_bytes_alloc(
                        klv::EMPTY_KLV_BYTES,
                    ))
                } else {
                    std::sync::Arc::new(klv::Klv::<kwg::Node22>::from_bytes_alloc(&std::fs::read(
                        args3,
                    )?))
                };
                let arc_klv1 = if args3 == args4 {
                    std::sync::Arc::clone(&arc_klv0)
                } else if args4 == "-" {
                    std::sync::Arc::new(klv::Klv::<kwg::Node22>::from_bytes_alloc(
                        klv::EMPTY_KLV_BYTES,
                    ))
                } else {
                    std::sync::Arc::new(klv::Klv::<kwg::Node22>::from_bytes_alloc(&std::fs::read(
                        args4,
                    )?))
                };
                generate_autoplay_logs::<true, false, _, _>(
                    make_game_config(),
                    kwg,
                    arc_klv0,
                    arc_klv1,
                    num_games,
                    min_samples_per_rack,
                    seed,
                )?;
                Ok(true)
            }
            "-autoplay-summarize" => {
                let args3 = if args.len() > 3 { &args[3] } else { "-" };
                let args4 = if args.len() > 4 { &args[4] } else { "-" };
                let num_games = if args.len() > 5 {
                    u64::from_str(&args[5])?
                } else {
                    1_000_000
                };
                let min_samples_per_rack = if args.len() > 6 {
                    u64::from_str(&args[6])?
                } else {
                    0
                };
                let seed = if args.len() > 7 {
                    Some(u64::from_str(&args[7])?)
                } else {
                    None
                };
                let kwg =
                    kwg::Kwg::<N>::from_bytes_alloc(&read_to_end(&mut make_reader(&args[2])?)?);
                let arc_klv0 = if args3 == "-" {
                    std::sync::Arc::new(klv::Klv::<kwg::Node22>::from_bytes_alloc(
                        klv::EMPTY_KLV_BYTES,
                    ))
                } else {
                    std::sync::Arc::new(klv::Klv::<kwg::Node22>::from_bytes_alloc(&std::fs::read(
                        args3,
                    )?))
                };
                let arc_klv1 = if args3 == args4 {
                    std::sync::Arc::clone(&arc_klv0)
                } else if args4 == "-" {
                    std::sync::Arc::new(klv::Klv::<kwg::Node22>::from_bytes_alloc(
                        klv::EMPTY_KLV_BYTES,
                    ))
                } else {
                    std::sync::Arc::new(klv::Klv::<kwg::Node22>::from_bytes_alloc(&std::fs::read(
                        args4,
                    )?))
                };
                generate_autoplay_logs::<true, true, _, _>(
                    make_game_config(),
                    kwg,
                    arc_klv0,
                    arc_klv1,
                    num_games,
                    min_samples_per_rack,
                    seed,
                )?;
                Ok(true)
            }
            "-autoplay-summarize-only" => {
                let args3 = if args.len() > 3 { &args[3] } else { "-" };
                let args4 = if args.len() > 4 { &args[4] } else { "-" };
                let num_games = if args.len() > 5 {
                    u64::from_str(&args[5])?
                } else {
                    1_000_000
                };
                let min_samples_per_rack = if args.len() > 6 {
                    u64::from_str(&args[6])?
                } else {
                    0
                };
                let seed = if args.len() > 7 {
                    Some(u64::from_str(&args[7])?)
                } else {
                    None
                };
                let kwg =
                    kwg::Kwg::<N>::from_bytes_alloc(&read_to_end(&mut make_reader(&args[2])?)?);
                let arc_klv0 = if args3 == "-" {
                    std::sync::Arc::new(klv::Klv::<kwg::Node22>::from_bytes_alloc(
                        klv::EMPTY_KLV_BYTES,
                    ))
                } else {
                    std::sync::Arc::new(klv::Klv::<kwg::Node22>::from_bytes_alloc(&std::fs::read(
                        args3,
                    )?))
                };
                let arc_klv1 = if args3 == args4 {
                    std::sync::Arc::clone(&arc_klv0)
                } else if args4 == "-" {
                    std::sync::Arc::new(klv::Klv::<kwg::Node22>::from_bytes_alloc(
                        klv::EMPTY_KLV_BYTES,
                    ))
                } else {
                    std::sync::Arc::new(klv::Klv::<kwg::Node22>::from_bytes_alloc(&std::fs::read(
                        args4,
                    )?))
                };
                generate_autoplay_logs::<false, true, _, _>(
                    make_game_config(),
                    kwg,
                    arc_klv0,
                    arc_klv1,
                    num_games,
                    min_samples_per_rack,
                    seed,
                )?;
                Ok(true)
            }
            "-gilles" => {
                let args3 = if args.len() > 3 { &args[3] } else { "-" };
                let args4 = if args.len() > 4 { &args[4] } else { "-" };
                let num_games = if args.len() > 5 {
                    u64::from_str(&args[5])?
                } else {
                    1_000_000
                };
                let min_samples = if args.len() > 6 {
                    u64::from_str(&args[6])?
                } else {
                    0
                };
                let seed = if args.len() > 7 {
                    Some(u64::from_str(&args[7])?)
                } else {
                    None
                };
                let kwg =
                    kwg::Kwg::<N>::from_bytes_alloc(&read_to_end(&mut make_reader(&args[2])?)?);
                let arc_klv0 = if args3 == "-" {
                    std::sync::Arc::new(klv::Klv::<kwg::Node22>::from_bytes_alloc(
                        klv::EMPTY_KLV_BYTES,
                    ))
                } else {
                    std::sync::Arc::new(klv::Klv::<kwg::Node22>::from_bytes_alloc(&std::fs::read(
                        args3,
                    )?))
                };
                let arc_klv1 = if args3 == args4 {
                    std::sync::Arc::clone(&arc_klv0)
                } else if args4 == "-" {
                    std::sync::Arc::new(klv::Klv::<kwg::Node22>::from_bytes_alloc(
                        klv::EMPTY_KLV_BYTES,
                    ))
                } else {
                    std::sync::Arc::new(klv::Klv::<kwg::Node22>::from_bytes_alloc(&std::fs::read(
                        args4,
                    )?))
                };
                generate_gilles_summary(
                    make_game_config(),
                    kwg,
                    arc_klv0,
                    arc_klv1,
                    num_games,
                    min_samples,
                    seed,
                )?;
                Ok(true)
            }
            "-census" => {
                let args3 = if args.len() > 3 { &args[3] } else { "-" };
                let args4 = if args.len() > 4 { &args[4] } else { "-" };
                let board_counts = if args.len() > 5 {
                    parse_board_counts(&args[5])?
                } else {
                    vec![500]
                };
                let seed = if args.len() > 6 {
                    Some(u64::from_str(&args[6])?)
                } else {
                    None
                };
                let kwg =
                    kwg::Kwg::<N>::from_bytes_alloc(&read_to_end(&mut make_reader(&args[2])?)?);
                let arc_klv0 = if args3 == "-" {
                    std::sync::Arc::new(klv::Klv::<kwg::Node22>::from_bytes_alloc(
                        klv::EMPTY_KLV_BYTES,
                    ))
                } else {
                    std::sync::Arc::new(klv::Klv::<kwg::Node22>::from_bytes_alloc(&std::fs::read(
                        args3,
                    )?))
                };
                let arc_klv1 = if args3 == args4 {
                    std::sync::Arc::clone(&arc_klv0)
                } else if args4 == "-" {
                    std::sync::Arc::new(klv::Klv::<kwg::Node22>::from_bytes_alloc(
                        klv::EMPTY_KLV_BYTES,
                    ))
                } else {
                    std::sync::Arc::new(klv::Klv::<kwg::Node22>::from_bytes_alloc(&std::fs::read(
                        args4,
                    )?))
                };
                generate_census_leaves(
                    make_game_config(),
                    kwg,
                    arc_klv0,
                    arc_klv1,
                    board_counts,
                    seed,
                )?;
                Ok(true)
            }
            "-compare" => {
                let args3 = if args.len() > 3 { &args[3] } else { "-" };
                let args4 = if args.len() > 4 { &args[4] } else { "-" };
                let num_game_pairs = if args.len() > 5 {
                    u64::from_str(&args[5])?
                } else {
                    10_000
                };
                let seed = if args.len() > 6 {
                    Some(u64::from_str(&args[6])?)
                } else {
                    None
                };
                let kwg =
                    kwg::Kwg::<N>::from_bytes_alloc(&read_to_end(&mut make_reader(&args[2])?)?);
                let arc_klv0 = if args3 == "-" {
                    std::sync::Arc::new(klv::Klv::<kwg::Node22>::from_bytes_alloc(
                        klv::EMPTY_KLV_BYTES,
                    ))
                } else {
                    std::sync::Arc::new(klv::Klv::<kwg::Node22>::from_bytes_alloc(&std::fs::read(
                        args3,
                    )?))
                };
                let arc_klv1 = if args3 == args4 {
                    std::sync::Arc::clone(&arc_klv0)
                } else if args4 == "-" {
                    std::sync::Arc::new(klv::Klv::<kwg::Node22>::from_bytes_alloc(
                        klv::EMPTY_KLV_BYTES,
                    ))
                } else {
                    std::sync::Arc::new(klv::Klv::<kwg::Node22>::from_bytes_alloc(&std::fs::read(
                        args4,
                    )?))
                };
                compare_leaves(
                    make_game_config(),
                    kwg,
                    arc_klv0,
                    arc_klv1,
                    num_game_pairs,
                    seed,
                )?;
                Ok(true)
            }
            "-sim-compare" => {
                let arc_klv = if args.len() > 3 && args[3] != "-" {
                    std::sync::Arc::new(klv::Klv::<kwg::Node22>::from_bytes_alloc(&std::fs::read(
                        &args[3],
                    )?))
                } else {
                    std::sync::Arc::new(klv::Klv::<kwg::Node22>::from_bytes_alloc(
                        klv::EMPTY_KLV_BYTES,
                    ))
                };
                let num_game_pairs = if args.len() > 4 {
                    u64::from_str(&args[4])?
                } else {
                    1_000
                };
                let seed = if args.len() > 5 {
                    Some(u64::from_str(&args[5])?)
                } else {
                    None
                };
                let kwg =
                    kwg::Kwg::<N>::from_bytes_alloc(&read_to_end(&mut make_reader(&args[2])?)?);
                sim_compare(make_game_config(), kwg, arc_klv, num_game_pairs, seed)?;
                Ok(true)
            }
            "-sim-study-check" => {
                let klv = if args.len() > 3 && args[3] != "-" {
                    klv::Klv::<kwg::Node22>::from_bytes_alloc(&std::fs::read(&args[3])?)
                } else {
                    klv::Klv::<kwg::Node22>::from_bytes_alloc(klv::EMPTY_KLV_BYTES)
                };
                let iters = if args.len() > 4 {
                    u64::from_str(&args[4])?
                } else {
                    64
                };
                let seed = if args.len() > 5 {
                    u64::from_str(&args[5])?
                } else {
                    1
                };
                let kwg =
                    kwg::Kwg::<N>::from_bytes_alloc(&read_to_end(&mut make_reader(&args[2])?)?);
                let game_config = make_game_config();
                let mut rng = rand::rngs::ChaCha20Rng::seed_from_u64(seed);
                let mut game_state = game_state::GameState::new(&game_config);
                game_state.reset_and_draw_tiles(&game_config, &mut rng);
                let mut move_generator = movegen::KurniaMoveGenerator::new(&game_config);
                move_generator.gen_moves_unfiltered(&movegen::GenMovesParams {
                    board_snapshot: &movegen::BoardSnapshot {
                        board_tiles: &game_state.board_tiles,
                        game_config: &game_config,
                        kwg: &kwg,
                        klv: &klv,
                    },
                    rack: &game_state.current_player().rack,
                    max_gen: 100,
                    num_exchanges_by_this_player: game_state.current_player().num_exchanges,
                    pass_policy: movegen::PassPolicy::OnlyWhenForced,
                    dynamic_leaves: None,
                });
                let mut driver = move_picker::Simmer::new(&game_config, &kwg, &klv);
                driver.set_num_sim_iters(iters);
                driver.reseed(seed);
                driver.begin_decision(&move_generator, &game_state, iters);
                let one_shot = driver.leader_summary();
                driver.reseed(seed);
                let half = iters / 2;
                driver.begin_decision(&move_generator, &game_state, half);
                driver.resume(&move_generator, iters - half);
                let split = driver.leader_summary();
                println!(
                    "one_shot leader play_index={} mean={} count={}",
                    one_shot.0, one_shot.1, one_shot.2,
                );
                println!(
                    "split    leader play_index={} mean={} count={}",
                    split.0, split.1, split.2,
                );
                if one_shot == split {
                    println!("SIM_RESUME_OK");
                    Ok(true)
                } else {
                    wolges::return_error!(
                        "resume mismatch: split decision differs from one-shot".to_string()
                    )
                }
            }
            "-sim-mutate-check" => {
                let klv = if args.len() > 3 && args[3] != "-" {
                    klv::Klv::<kwg::Node22>::from_bytes_alloc(&std::fs::read(&args[3])?)
                } else {
                    klv::Klv::<kwg::Node22>::from_bytes_alloc(klv::EMPTY_KLV_BYTES)
                };
                let iters = if args.len() > 4 {
                    u64::from_str(&args[4])?
                } else {
                    96
                };
                let seed = if args.len() > 5 {
                    u64::from_str(&args[5])?
                } else {
                    1
                };
                let kwg =
                    kwg::Kwg::<N>::from_bytes_alloc(&read_to_end(&mut make_reader(&args[2])?)?);
                let game_config = make_game_config();
                let mut rng = rand::rngs::ChaCha20Rng::seed_from_u64(seed);
                let mut game_state = game_state::GameState::new(&game_config);
                game_state.reset_and_draw_tiles(&game_config, &mut rng);
                let mut move_generator = movegen::KurniaMoveGenerator::new(&game_config);
                move_generator.gen_moves_unfiltered(&movegen::GenMovesParams {
                    board_snapshot: &movegen::BoardSnapshot {
                        board_tiles: &game_state.board_tiles,
                        game_config: &game_config,
                        kwg: &kwg,
                        klv: &klv,
                    },
                    rack: &game_state.current_player().rack,
                    max_gen: 100,
                    num_exchanges_by_this_player: game_state.current_player().num_exchanges,
                    pass_policy: movegen::PassPolicy::OnlyWhenForced,
                    dynamic_leaves: None,
                });
                let mut driver = move_picker::Simmer::new(&game_config, &kwg, &klv);
                driver.set_num_sim_iters(iters);
                driver.reseed(seed);
                driver.begin_decision(&move_generator, &game_state, iters);
                let retired_id = driver.retired_stream_ids().next();
                match retired_id {
                    None => wolges::return_error!(
                        "no candidates were pruned; raise the iteration budget".to_string()
                    ),
                    Some(id) => {
                        let before = driver.stream_count(id).unwrap();
                        let readmitted = driver.readmit_with_history(id);
                        driver.resume(&move_generator, iters);
                        let after = driver.stream_count(id).unwrap();
                        println!("readmit stream {id}: count before={before} after={after}");
                        if readmitted && after >= before {
                            println!("SIM_MUTATE_OK");
                            Ok(true)
                        } else {
                            wolges::return_error!(
                                "readmit dropped history: count reset".to_string()
                            )
                        }
                    }
                }
            }
            "-rollout" => {
                let args3 = if args.len() > 3 { &args[3] } else { "-" };
                let num_games = if args.len() > 4 {
                    u64::from_str(&args[4])?
                } else {
                    10_000
                };
                let seed = if args.len() > 5 {
                    Some(u64::from_str(&args[5])?)
                } else {
                    None
                };
                let kwg =
                    kwg::Kwg::<N>::from_bytes_alloc(&read_to_end(&mut make_reader(&args[2])?)?);
                let arc_klv = if args3 == "-" {
                    std::sync::Arc::new(klv::Klv::<kwg::Node22>::from_bytes_alloc(
                        klv::EMPTY_KLV_BYTES,
                    ))
                } else {
                    std::sync::Arc::new(klv::Klv::<kwg::Node22>::from_bytes_alloc(&std::fs::read(
                        args3,
                    )?))
                };
                generate_rollout_leaves(make_game_config(), kwg, arc_klv, num_games, seed)?;
                Ok(true)
            }
            "-winpct" => {
                let args3 = if args.len() > 3 { &args[3] } else { "-" };
                let num_games = if args.len() > 4 {
                    u64::from_str(&args[4])?
                } else {
                    1_000_000
                };
                let seed = if args.len() > 5 {
                    Some(u64::from_str(&args[5])?)
                } else {
                    None
                };
                let kwg =
                    kwg::Kwg::<N>::from_bytes_alloc(&read_to_end(&mut make_reader(&args[2])?)?);
                let arc_klv = if args3 == "-" {
                    std::sync::Arc::new(klv::Klv::<kwg::Node22>::from_bytes_alloc(
                        klv::EMPTY_KLV_BYTES,
                    ))
                } else {
                    std::sync::Arc::new(klv::Klv::<kwg::Node22>::from_bytes_alloc(&std::fs::read(
                        args3,
                    )?))
                };
                generate_winpct_table(make_game_config(), kwg, arc_klv, num_games, seed)?;
                Ok(true)
            }
            "-winpct-eval" => {
                let args3 = if args.len() > 3 { &args[3] } else { "-" };
                let num_games = if args.len() > 5 {
                    u64::from_str(&args[5])?
                } else {
                    1_000_000
                };
                let seed = if args.len() > 6 {
                    Some(u64::from_str(&args[6])?)
                } else {
                    None
                };
                let kwg =
                    kwg::Kwg::<N>::from_bytes_alloc(&read_to_end(&mut make_reader(&args[2])?)?);
                let arc_klv = if args3 == "-" {
                    std::sync::Arc::new(klv::Klv::<kwg::Node22>::from_bytes_alloc(
                        klv::EMPTY_KLV_BYTES,
                    ))
                } else {
                    std::sync::Arc::new(klv::Klv::<kwg::Node22>::from_bytes_alloc(&std::fs::read(
                        args3,
                    )?))
                };
                let table = win_pct::WinPctTable::from_csv(&std::fs::read_to_string(&args[4])?)?;
                generate_winpct_eval(make_game_config(), kwg, arc_klv, table, num_games, seed)?;
                Ok(true)
            }
            "-winpct-combine" => {
                if args.len() < 4 {
                    return Err(
                        "english-winpct-combine needs an output and at least one input".into(),
                    );
                }
                let mut acc = win_pct::WinPctAccumulator::new();
                for path in &args[3..] {
                    acc.merge(&win_pct::WinPctAccumulator::from_csv(
                        &std::fs::read_to_string(path)?,
                    )?);
                }
                make_writer(&args[2])?.write_all(acc.to_csv().as_bytes())?;
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
            "-resummarize" => {
                resummarize_summaries::<'a', _, _>(
                    make_game_config(),
                    csv::ReaderBuilder::new()
                        .has_headers(false)
                        .from_reader(make_reader(&args[2])?),
                    csv::Writer::from_writer(make_writer(&args[3])?),
                )?;
                Ok(true)
            }
            "-resummarize-playability" => {
                resummarize_summaries::<'p', _, _>(
                    make_game_config(),
                    csv::ReaderBuilder::new()
                        .has_headers(false)
                        .from_reader(make_reader(&args[2])?),
                    csv::Writer::from_writer(make_writer(&args[3])?),
                )?;
                Ok(true)
            }
            "-resummarize-playability-all" => {
                resummarize_summaries::<'P', _, _>(
                    make_game_config(),
                    csv::ReaderBuilder::new()
                        .has_headers(false)
                        .from_reader(make_reader(&args[2])?),
                    csv::Writer::from_writer(make_writer(&args[3])?),
                )?;
                Ok(true)
            }
            "-generate" => {
                generate_leaves::<_, _, false>(
                    make_game_config(),
                    csv::ReaderBuilder::new()
                        .has_headers(false)
                        .from_reader(make_reader(&args[2])?),
                    csv::Writer::from_writer(make_writer(&args[3])?),
                    args.get(4).map(|x| x.as_str()),
                )?;
                Ok(true)
            }
            "-generate-full" => {
                generate_leaves::<_, _, true>(
                    make_game_config(),
                    csv::ReaderBuilder::new()
                        .has_headers(false)
                        .from_reader(make_reader(&args[2])?),
                    csv::Writer::from_writer(make_writer(&args[3])?),
                    args.get(4).map(|x| x.as_str()),
                )?;
                Ok(true)
            }
            "-playability" => {
                let args3 = if args.len() > 3 { &args[3] } else { "-" };
                let num_games = if args.len() > 4 {
                    u64::from_str(&args[4])?
                } else {
                    1_000_000
                };
                let seed = if args.len() > 5 {
                    Some(u64::from_str(&args[5])?)
                } else {
                    None
                };
                let kwg =
                    kwg::Kwg::<N>::from_bytes_alloc(&read_to_end(&mut make_reader(&args[2])?)?);
                let klv = if args3 == "-" {
                    klv::Klv::<kwg::Node22>::from_bytes_alloc(klv::EMPTY_KLV_BYTES)
                } else {
                    klv::Klv::<kwg::Node22>::from_bytes_alloc(&std::fs::read(args3)?)
                };
                discover_playability(make_game_config(), kwg, klv, num_games, seed)?;
                Ok(true)
            }
            _ => Ok(false),
        },
        None => Ok(false),
    }
}

// leave = listing extrapolated accumulated values empirically

fn main() -> error::Returns<()> {
    let args = std::env::args().collect::<Vec<_>>();
    if args.len() <= 1 {
        println!(
            "args:
  english-autoplay CSW24.kwg leave0.klv leave1.klv 1000000 0 [seed]
    autoplay 1000000 games, logs to a pair of csv.
    (changing output filenames needs recompile.)
    if leave is \"-\" or omitted, uses no leave.
    number of games is optional.
    min samples per rack is optional, but must be 0 for non-summarize.
    seed is optional; prints auto-generated seed to stderr if not provided.
  english-autoplay-summarize CSW24.kwg leave0.klv leave1.klv 1000000 0 [seed]
    same as english-autoplay and also save summary file.
  english-autoplay-summarize-only CSW24.kwg leave0.klv leave1.klv 1000000 0 [seed]
    same as english-autoplay-summarize but do not save the log files.
  english-gilles CSW24.kwg leave0.klv leave1.klv 1000000 [min_samples] [seed]
    GillesB board-sampling leave generation. plays greedy (leave-modified)
    games, snapshots boards, samples worst racks, records best-play equity.
    writes a gilles-summary-* csv in the same format as autoplay-summarize,
    so it merges via -resummarize and decomposes via -generate.
    parameters scale with the game config (works on any variant).
    min_samples is optional (default 0 = pure board sampling); when nonzero,
    remediation games keep playing after the first 1000000 and direct their
    samples at racks still seen fewer than min_samples times, growing the worst
    group as needed, until every rack reaches min_samples or no further progress
    is possible. tune via WOLGES_GILLES_* env vars.
    seed is optional; prints auto-generated seed to stderr if not provided.
  english-summarize logfile summary.csv
    summarize logfile into summary.csv
  english-resummarize concatenated_summaries.csv summary.csv
    combine multiple summaries into one summary.csv and recompute totals
  english-generate summary.csv leaves.csv [rare.csv]
    generate leaves up to rack_size - 1
  english-generate-full summary.csv leaves.csv [rare.csv]
    generate leaves up to rack_size
    a leave too thinly sampled to trust borrows its value from its
    one-tile-swap neighbors; tune which those are via
    WOLGES_GENERATE_SMOOTH_MIN / _CI
    [rare.csv] on any -generate adds direct coverage for undersampled subracks
  english-playability CSW24.kwg leave.klv 1000000 [seed]
    autoplay (not saved) and record prorated found best words (at the end)
    (run fewer number of games and use resummarize to merge to mitigate risks)
    seed is optional; prints auto-generated seed to stderr if not provided.
  english-winpct CSW24.kwg leave.klv 1000000 [seed]
    Hasty self-play, recording an empirical win% table (P(mover wins) by
    lead and count-state (bag, my, opp)) as raw sparse csv to stdout.
    if leave is \"-\" or omitted, uses no leave.
    number of games is optional (default 1000000).
    seed is optional; prints auto-generated seed to stderr if not provided.
  english-winpct-eval CSW24.kwg leave.klv table.csv 1000000 [seed]
    score a win% table and the simmer win_prob sigmoid by Brier (lower is
    better) against Hasty self-play outcomes; use a held-out seed.
    number of games is optional (default 1000000).
    seed is optional; prints auto-generated seed to stderr if not provided.
  english-winpct-combine win_pct.csv win_pct1.csv win_pct2.csv [...]
    merge several english-winpct raw tables into one by summing their
    per-count-state histograms (counts add exactly, no rounding).
    the first argument is the output (\"-\" = stdout); the rest are inputs.
    run english-winpct on separate seeds/processes, then combine here.
  english-resummarize-playability concatenated_playabilities.csv playability.csv
    same as english-resummarize but sorts differently (by length first)
  english-resummarize-playability-all concat_playabilities.csv playability.csv
    same as english-resummarize but sorts differently (by playability first)
  english-compare CSW24.kwg klv0.klv2 klv1.klv2 10000 [seed]
    play game pairs to compare two sets of leaves.
    p0 uses klv0, p1 uses klv1 for move selection (static play, max=1).
    reports wins/losses/draws, score stats, divergent games, and
    confidence that one set of leaves is better.
    if klv is \"-\" or omitted, uses no leave.
    number of game pairs is optional (default 10000).
    seed is optional; prints auto-generated seed to stderr if not provided.
  english-sim-compare CSW24.kwg leaves.klv2 1000 [seed]
    play game pairs where both seats choose moves by the 2-ply simmer,
    each seat configured by WOLGES_SIM_P0_* / WOLGES_SIM_P1_* (and a shared
    WOLGES_SIM_ITERS budget), to A/B simmer configurations.
    each pair: same tile draw, alternating starting player.
  english-sim-study-check CSW24.kwg leaves.klv2 64 [seed]
    self-check that a resumed decision (begin_decision then resume) matches
    the same decision run in one call; prints SIM_RESUME_OK on success.
  english-sim-mutate-check CSW24.kwg leaves.klv2 96 [seed]
    self-check that readmitting a retired candidate keeps its statistics;
    prints SIM_MUTATE_OK on success.
  (english can also be catalan, dutch, french, german, norwegian, polish,
    slovene, spanish, swedish, super-english, super-catalan)
  (add -big after language, such as dutch-big-autoplay, to use kbwg)
  jumbled-english-autoplay CSW24.kad leave0.klv leave1.klv 1000
    (all also take jumbled- prefix, including jumbled-super-;
    note that jumbled autoplay requires .kad instead of .kwg)
input/output files can be \"-\" (not advisable for binary files).
for english-autoplay only the kwg can come from \"-\".
when low disk space, note that in bash:
  english-autoplay ... 1000
  english-summarize log1 summary1.csv
  english-autoplay ... 1000
  english-summarize log2 summary2.csv
  english-resummarize <( cat summary1.csv summary2.csv ) summary.csv
  english-generate summary.csv leaves.csv
    is the same as
  english-autoplay ... 1000
  english-summarize log1 summary1.csv
  english-autoplay ... 1000
  english-summarize log2 summary2.csv
  english-generate <( cat summary1.csv summary2.csv ) leaves.csv
    which is the same as
  english-autoplay ... 1000
  english-autoplay ... 1000
  english-summarize <( cat log1 log2 ) summary.csv
  english-generate summary.csv leaves.csv
    but it becomes possible to remove log1 to free up disk space for log2.
    using resummarize also allows removing summary1.csv earlier."
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
            || do_lang(&args, "dutch", game_config::make_dutch_game_config)?
            || do_lang(
                &args,
                "jumbled-dutch",
                game_config::make_jumbled_dutch_game_config,
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
            || do_lang(&args, "swedish", game_config::make_swedish_game_config)?
            || do_lang(
                &args,
                "jumbled-swedish",
                game_config::make_jumbled_swedish_game_config,
            )?
        {
        } else {
            return Err("invalid argument".into());
        }
        writeln!(boxed_stdout_or_stderr(), "time taken: {:?}", t0.elapsed())?;
        Ok(())
    }
}

fn env_parse<T: std::str::FromStr>(name: &str, default: T) -> T {
    std::env::var(name)
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(default)
}

fn env_flag(name: &str, default: bool) -> bool {
    env_parse::<u64>(name, default as u64) != 0
}

fn wolges_threads() -> usize {
    std::env::var("WOLGES_THREADS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or_else(num_cpus::get)
}

#[derive(Clone, Copy)]
enum Apportion {
    FullRack,
    Entering,
}

fn wolges_apportion() -> error::Returns<Apportion> {
    match std::env::var("WOLGES_APPORTION").ok().as_deref() {
        None | Some("full-rack") => Ok(Apportion::FullRack),
        Some("entering") => Ok(Apportion::Entering),
        Some(other) => {
            Err(format!("WOLGES_APPORTION must be full-rack or entering, got {other:?}").into())
        }
    }
}

#[derive(Clone, Copy)]
enum CiReport {
    Off,
    Rack,
    Leave,
}

fn wolges_census_ci_report() -> error::Returns<CiReport> {
    match std::env::var("WOLGES_CENSUS_CI_REPORT").ok().as_deref() {
        None | Some("off") => Ok(CiReport::Off),
        Some("rack") => Ok(CiReport::Rack),
        Some("leave") => Ok(CiReport::Leave),
        Some(other) => Err(format!(
            "WOLGES_CENSUS_CI_REPORT must be off, rack, or leave, got {other:?}"
        )
        .into()),
    }
}

fn generate_autoplay_logs<
    const WRITE_LOGS: bool,
    const SUMMARIZE: bool,
    N: kwg::Node + Sync + Send,
    L: kwg::Node + Sync + Send,
>(
    game_config: game_config::GameConfig,
    kwg: kwg::Kwg<N>,
    arc_klv0: std::sync::Arc<klv::Klv<L>>,
    arc_klv1: std::sync::Arc<klv::Klv<L>>,
    num_games: u64,
    min_samples_per_rack: u64,
    seed: Option<u64>,
) -> error::Returns<()> {
    if !SUMMARIZE && min_samples_per_rack != 0 {
        return Err("min_samples_per_rack requires summarize".into());
    }

    let impossible_ok = env_flag("WOLGES_IMPOSSIBLE_OK", true);

    let full_rack_forcing = env_flag("WOLGES_AUTOPLAY_FULL_RACK_FORCING", false);

    let entering = match wolges_apportion()? {
        Apportion::Entering => true,
        Apportion::FullRack => false,
    };

    let oppdenial_leave = env_parse::<f64>("WOLGES_OPPDENIAL_LEAVE", 0.0);

    let oppdenial_rack = env_parse::<f64>("WOLGES_OPPDENIAL_RACK", 0.0);

    let oppdenial_exact = env_parse::<f64>("WOLGES_OPPDENIAL_EXACT", 0.0);
    let oppdenial_exact_pool_max = env_usize("WOLGES_OPPDENIAL_EXACT_POOL_MAX", 32);

    let oppdenial_exact_me2 = env_parse::<f64>("WOLGES_OPPDENIAL_EXACT_ME2", 1.0);

    let winpct_table: Option<win_pct::WinPctTable> = if env_flag("WOLGES_WINPCT", false) {
        let path =
            std::env::var("WOLGES_WINPCT_TABLE").unwrap_or_else(|_| "win_pct.csv".to_string());
        let t = win_pct::WinPctTable::from_csv(&std::fs::read_to_string(&path)?)?;
        eprintln!("autoplay: win%-objective from {path}");
        Some(t)
    } else {
        None
    };

    let winpct_blend = env_parse::<f64>("WOLGES_WINPCT_BLEND", 1.0);
    let opp_on = (oppdenial_leave != 0.0 || oppdenial_rack != 0.0 || oppdenial_exact != 0.0)
        && winpct_table.is_none();

    let opp_ctx: Option<(census::MultisetLattice, census::AddTable, Vec<i32>)> = if opp_on {
        let num_letters = game_config.alphabet().len() as usize;
        let rack_size = game_config.rack_size() as usize;
        let lat = census::MultisetLattice::new(num_letters, rack_size);
        let add_table = census::AddTable::new(&lat);
        let mut leave = vec![0i32; lat.len()];
        census::fill_lattice_leaves(&lat, &mut leave, |tally| {
            arc_klv0.leave_value_from_tally(tally)
        });
        eprintln!(
            "autoplay: WOLGES_OPPDENIAL_LEAVE={oppdenial_leave} WOLGES_OPPDENIAL_RACK={oppdenial_rack} WOLGES_OPPDENIAL_EXACT={oppdenial_exact} \
             oppdenial_exact_pool_max={oppdenial_exact_pool_max} opponent-denial machinery on ({} lattice leaves)",
            lat.len(),
        );
        Some((lat, add_table, leave))
    } else {
        None
    };

    let game_config = std::sync::Arc::new(game_config);
    let kwg = std::sync::Arc::new(kwg);
    let player_aliases = std::sync::Arc::new(
        (1..=game_config.num_players())
            .map(|x| format!("p{x}"))
            .collect::<Box<[String]>>(),
    );
    let seed = seed.unwrap_or_else(rand::random);
    eprintln!("seed: {seed}");
    let num_threads = wolges_threads();

    let dynamic_leaves_on = std::env::var("WOLGES_DYNAMIC_LEAVES")
        .ok()
        .and_then(|s| s.parse::<u64>().ok())
        .unwrap_or(0)
        != 0;
    let dynamic_min_keep = std::env::var("WOLGES_DYNAMIC_LEAVES_MIN_KEEP")
        .ok()
        .and_then(|s| s.parse::<usize>().ok())
        .unwrap_or(2);
    let dyn_ctx: Option<(census::MultisetLattice, census::AddTable, Vec<i32>)> =
        if dynamic_leaves_on {
            let num_letters = game_config.alphabet().len() as usize;
            let rack_size = game_config.rack_size() as usize;
            let lat = census::MultisetLattice::new(num_letters, rack_size);
            let add = census::AddTable::new_with_threads(&lat, num_threads);
            let mut full_v = vec![0i32; lat.len()];
            census::fill_lattice_leaves(&lat, &mut full_v, |tally| {
                arc_klv0.leave_value_from_tally(tally)
            });
            Some((lat, add, full_v))
        } else {
            None
        };
    let dyn_ref = dyn_ctx
        .as_ref()
        .map(|(lat, add, full_v)| klv::DynamicLeavesRef {
            lat,
            add,
            full_v: full_v.as_slice(),
            min_keep: dynamic_min_keep,
        });
    eprintln!(
        "WOLGES_DYNAMIC_LEAVES={} WOLGES_DYNAMIC_LEAVES_MIN_KEEP={dynamic_min_keep} ({})",
        dynamic_leaves_on as u8,
        if dynamic_leaves_on {
            "dynamic leaves on for the klv0 side; needs a --full (len 1-7) klv0"
        } else {
            "off, static leaves"
        },
    );

    let num_processed_games = std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0));

    let run_identifier = std::sync::Arc::new(format!("log-{}", run_stamp()));
    eprintln!("logging to {run_identifier}");
    let mut csv_log = if WRITE_LOGS {
        Some(csv::Writer::from_path(claim_output_path(&run_identifier)?)?)
    } else {
        None
    };
    if let Some(ref mut c) = csv_log {
        c.serialize((
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
    }
    let csv_log_writer = if let Some(c) = csv_log {
        Some(c.into_inner()?)
    } else {
        None
    };
    let mut csv_game =
        csv::Writer::from_path(claim_output_path(&format!("games-{run_identifier}"))?)?;
    csv_game.serialize((
        "gameID",
        player_aliases
            .iter()
            .map(|x| format!("{x}_score"))
            .collect::<Box<[String]>>(),
        player_aliases
            .iter()
            .map(|x| format!("{x}_bingos"))
            .collect::<Box<[String]>>(),
        player_aliases
            .iter()
            .map(|x| format!("{x}_turns"))
            .collect::<Box<[String]>>(),
        "first",
    ))?;
    let csv_game_writer = csv_game.into_inner()?;
    let completed_games = std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0));
    let logged_games = std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0));
    let completed_moves = std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0));
    let full_rack_map = fash::MyHashMap::<bites::Bites, Cumulate>::default();

    let rare_subrack_map = fash::MyHashMap::<bites::Bites, Cumulate>::default();

    // 0 = threads are collaboratively accumulating first num_games games.
    // 1 = one thread is determining which racks are undersampled after the
    //     first num_games games.
    // 2 = threads are playing more games to accumulate at least
    //     min_samples_per_rack samples per rack.
    // u64 is overkill. noted. so be it.
    let undersampling_remediation_state = std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0));
    // number of threads that have submitted their samples.
    let undersampling_remediation_submission =
        std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0));
    // unique and in any order.
    let undersampled_racks = Vec::<bites::Bites>::new();
    // countdown that may reset itself. needs to be signed.
    let undersampling_remediation_countdown =
        std::sync::Arc::new(std::sync::atomic::AtomicI64::new(0));
    // generation id.
    let undersampling_remediation_generation_id =
        std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0));
    let undersampling_comment = String::new();

    let t0 = std::time::Instant::now();
    let tick_periods = move_picker::Periods(0);
    struct MutexedStuffs {
        csv_game_writer: std::fs::File,
        csv_log_writer: Option<std::fs::File>,
        full_rack_map: fash::MyHashMap<bites::Bites, Cumulate>,
        rare_subrack_map: fash::MyHashMap<bites::Bites, Cumulate>,
        undersampled_racks: Vec<bites::Bites>,
        undersampled_generation: u64,
        undersampling_comment: String,
        tick_periods: move_picker::Periods,
        oppdenial_leave_sum_marg: Vec<f64>,
        oppdenial_leave_boards: u64,
    }
    let mutexed_stuffs = std::sync::Arc::new(std::sync::Mutex::new(MutexedStuffs {
        csv_game_writer,
        csv_log_writer,
        full_rack_map,
        rare_subrack_map,
        undersampled_racks,
        undersampled_generation: u64::MAX,
        undersampling_comment,
        tick_periods,
        oppdenial_leave_sum_marg: if oppdenial_leave != 0.0 {
            vec![0f64; game_config.alphabet().len() as usize]
        } else {
            Vec::new()
        },
        oppdenial_leave_boards: 0,
    }));
    let batch_size = 100;

    std::thread::scope(|s| {
        let mut threads = vec![];

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
            let undersampling_remediation_state =
                std::sync::Arc::clone(&undersampling_remediation_state);
            let undersampling_remediation_submission =
                std::sync::Arc::clone(&undersampling_remediation_submission);
            let undersampling_remediation_countdown =
                std::sync::Arc::clone(&undersampling_remediation_countdown);
            let undersampling_remediation_generation_id =
                std::sync::Arc::clone(&undersampling_remediation_generation_id);
            let mutexed_stuffs = std::sync::Arc::clone(&mutexed_stuffs);
            let opp_ctx = opp_ctx.as_ref();
            let winpct_table = winpct_table.as_ref();
            threads.push(s.spawn(move || {
                let mut rng = rand::rngs::ChaCha20Rng::seed_from_u64(seed);
                let mut game_id = String::with_capacity(8);
                let mut move_generator = movegen::KurniaMoveGenerator::new(&game_config);
                let mut game_state = game_state::GameState::new(&game_config);
                let mut cur_rack_as_vec = if SUMMARIZE {
                    Vec::with_capacity(game_config.rack_size() as usize)
                } else {
                    Vec::new()
                };
                let mut cur_rack_ser = String::new();
                let mut aft_rack = Vec::with_capacity(game_config.rack_size() as usize);
                let mut aft_rack_ser = String::new();
                let mut play_fmt = String::new();
                let mut equity_fmt = String::new();
                let mut final_scores = vec![0; game_config.num_players() as usize];

                let mut final_scores_pts = vec![0; game_config.num_players() as usize];
                let mut num_bingos = vec![0; game_config.num_players() as usize];
                let mut num_turns = vec![0; game_config.num_players() as usize];
                let mut num_moves;
                let mut num_batched_games_here = 0;
                let mut batched_csv_log = csv::Writer::from_writer(Vec::new());
                let mut batched_csv_game = csv::Writer::from_writer(Vec::new());
                let mut thread_full_rack_map = fash::MyHashMap::<bites::Bites, Cumulate>::default();

                let mut last_kept: Vec<Option<Vec<u8>>> = if SUMMARIZE && entering {
                    vec![None; game_config.num_players() as usize]
                } else {
                    Vec::new()
                };
                let mut aft_rack_entering = if SUMMARIZE && entering {
                    Vec::with_capacity(game_config.rack_size() as usize)
                } else {
                    Vec::new()
                };

                let mut thread_rare_subrack_map =
                    fash::MyHashMap::<bites::Bites, Cumulate>::default();
                let mut exchange_buffer = if SUMMARIZE && min_samples_per_rack != 0 {
                    Vec::with_capacity(game_config.rack_size() as usize)
                } else {
                    Vec::new()
                };
                let mut alphabet_freqs = if SUMMARIZE && min_samples_per_rack != 0 {
                    (0..game_config.alphabet().len())
                        .map(|tile| game_config.alphabet().freq(tile))
                        .collect::<Vec<_>>()
                } else {
                    Vec::new()
                };
                let mut unseen_tally = if SUMMARIZE && min_samples_per_rack != 0 {
                    vec![0u8; game_config.alphabet().len() as usize]
                } else {
                    Vec::new()
                };

                let mut unseen_pool = Vec::<u8>::new();
                let mut sample_rack_buf = if SUMMARIZE && min_samples_per_rack != 0 {
                    Vec::with_capacity(game_config.rack_size() as usize)
                } else {
                    Vec::new()
                };
                let mut undersampled_thread_racks = Vec::<bites::Bites>::new();

                let opp_num_letters = game_config.alphabet().len() as usize;
                let mut opp_sheet: Vec<i32> = Vec::new();
                let mut opp_best: Vec<i32> = Vec::new();
                let mut opp_marginal: Vec<f64> = Vec::new();
                let mut opp_base_freqs: Vec<u8> = Vec::new();
                let mut opp_unseen: Vec<u8> = Vec::new();
                let mut opp_movegen_rack: Vec<u8> = Vec::new();
                let mut opp_blank_deltas: Vec<(u8, i32)> = Vec::new();

                let mut oppdenial_exact_kept_idx: Vec<u32> = Vec::new();
                let mut oppdenial_exact_kept_size: Vec<u8> = Vec::new();
                let mut oppdenial_exact_term: Vec<f64> = Vec::new();
                if let Some((lat, _, _)) = opp_ctx {
                    opp_sheet = vec![0i32; lat.len()];
                    opp_best = vec![census::UNPLAYABLE; lat.len()];
                    opp_marginal = vec![0f64; opp_num_letters];
                    opp_base_freqs = (0..game_config.alphabet().len())
                        .map(|tile| game_config.alphabet().freq(tile))
                        .collect();
                    opp_unseen = vec![0u8; opp_num_letters];
                    if oppdenial_exact != 0.0 {
                        oppdenial_exact_kept_idx = vec![0u32; lat.len()];
                        oppdenial_exact_kept_size = vec![0u8; lat.len()];
                        oppdenial_exact_term = vec![0f64; lat.len()];
                    }
                }

                let mut oppdenial_leave_sum_marg: Vec<f64> = if oppdenial_leave != 0.0 {
                    vec![0f64; opp_num_letters]
                } else {
                    Vec::new()
                };
                let mut oppdenial_leave_boards = 0u64;

                let leave_size = if SUMMARIZE && min_samples_per_rack != 0 {
                    game_config.rack_size() - 1
                } else {
                    0
                };

                let mut word_prob = if SUMMARIZE && min_samples_per_rack != 0 {
                    Some(prob::WordProbability::new(game_config.alphabet()))
                } else {
                    None
                };
                let mut subrack_count_map = fash::MyHashMap::<bites::Bites, u64>::default();
                let mut recompute_rack_tally = if SUMMARIZE && min_samples_per_rack != 0 {
                    vec![0u8; game_config.alphabet().len() as usize]
                } else {
                    Vec::new()
                };
                let mut full_rack_tally = if SUMMARIZE && min_samples_per_rack != 0 {
                    vec![0u8; game_config.alphabet().len() as usize]
                } else {
                    Vec::new()
                };
                let mut subrack_tally = if SUMMARIZE && min_samples_per_rack != 0 {
                    vec![0u8; game_config.alphabet().len() as usize]
                } else {
                    Vec::new()
                };
                let mut undersampling_remediation_thread_generation_id = 0;
                let mut undersampling_remediation_thread_begun = false;
                loop {
                    let mut num_prior_games =
                        num_processed_games.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    rng.set_stream(num_prior_games);
                    if num_prior_games >= num_games {
                        if !undersampling_remediation_thread_begun {

                            {
                                let mut mutex_guard = mutexed_stuffs.lock().unwrap();
                                merge_rack_map(
                                    &mut mutex_guard.full_rack_map,
                                    &mut thread_full_rack_map,
                                );

                                merge_rack_map(
                                    &mut mutex_guard.rare_subrack_map,
                                    &mut thread_rare_subrack_map,
                                );
                            }
                            undersampling_remediation_submission
                                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);

                            while undersampling_remediation_submission
                                .load(std::sync::atomic::Ordering::Relaxed)
                                != num_threads as u64
                            {}
                            match undersampling_remediation_state.compare_exchange(
                                0,
                                1,
                                std::sync::atomic::Ordering::Relaxed,
                                std::sync::atomic::Ordering::Relaxed,
                            ) {
                                Ok(_) => {

                                    {
                                        let mut mutex_guard = mutexed_stuffs.lock().unwrap();

                                        std::mem::swap(
                                            &mut thread_full_rack_map,
                                            &mut mutex_guard.full_rack_map,
                                        );
                                        std::mem::swap(
                                            &mut thread_rare_subrack_map,
                                            &mut mutex_guard.rare_subrack_map,
                                        );
                                        let deficit = recompute_undersampled_subracks(
                                            &thread_full_rack_map,
                                            &thread_rare_subrack_map,
                                            &mut mutex_guard.undersampled_racks,
                                            &mut subrack_count_map,
                                            word_prob.as_mut(),
                                            RecomputeScratch {
                                                rack_tally: &mut recompute_rack_tally,
                                                full_rack_tally: &mut full_rack_tally,
                                                subrack_tally: &mut subrack_tally,
                                                alphabet_freqs: &mut alphabet_freqs,
                                                exchange_buffer: &mut exchange_buffer,
                                            },
                                            RecomputeParams {
                                                leave_size,
                                                full_rack_forcing,
                                                min_samples: min_samples_per_rack,
                                            },
                                        );
                                        std::mem::swap(
                                            &mut thread_full_rack_map,
                                            &mut mutex_guard.full_rack_map,
                                        );
                                        std::mem::swap(
                                            &mut thread_rare_subrack_map,
                                            &mut mutex_guard.rare_subrack_map,
                                        );
                                        mutex_guard.undersampled_generation = 0;
                                        mutex_guard.undersampling_comment.clear();
                                        if deficit != 0 {
                                            let num_undersampled =
                                                mutex_guard.undersampled_racks.len();
                                            write!(
                                                mutex_guard.undersampling_comment,
                                                " (need to force {num_undersampled} targets over {deficit} moves)"
                                            )
                                            .unwrap();
                                        }
                                        undersampling_remediation_countdown.store(
                                            deficit as i64,
                                            std::sync::atomic::Ordering::Relaxed,
                                        );
                                    }
                                    undersampling_remediation_state
                                        .compare_exchange(
                                            1,
                                            2,
                                            std::sync::atomic::Ordering::Relaxed,
                                            std::sync::atomic::Ordering::Relaxed,
                                        )
                                        .unwrap();
                                }
                                Err(_) => {

                                    while undersampling_remediation_state
                                        .load(std::sync::atomic::Ordering::Relaxed)
                                        <= 1
                                    {}
                                }
                            }
                            undersampling_remediation_thread_begun = true;
                        }
                        if undersampled_thread_racks.is_empty() {
                            let mut mutex_guard = mutexed_stuffs.lock().unwrap();

                            merge_rack_map(
                                &mut mutex_guard.full_rack_map,
                                &mut thread_full_rack_map,
                            );
                            merge_rack_map(
                                &mut mutex_guard.rare_subrack_map,
                                &mut thread_rare_subrack_map,
                            );

                            let current_generation = undersampling_remediation_generation_id
                                .load(std::sync::atomic::Ordering::Relaxed);
                            if mutex_guard.undersampled_generation != current_generation {
                                std::mem::swap(
                                    &mut thread_full_rack_map,
                                    &mut mutex_guard.full_rack_map,
                                );
                                std::mem::swap(
                                    &mut thread_rare_subrack_map,
                                    &mut mutex_guard.rare_subrack_map,
                                );
                                let deficit = recompute_undersampled_subracks(
                                    &thread_full_rack_map,
                                    &thread_rare_subrack_map,
                                    &mut mutex_guard.undersampled_racks,
                                    &mut subrack_count_map,
                                    word_prob.as_mut(),
                                    RecomputeScratch {
                                        rack_tally: &mut recompute_rack_tally,
                                        full_rack_tally: &mut full_rack_tally,
                                        subrack_tally: &mut subrack_tally,
                                        alphabet_freqs: &mut alphabet_freqs,
                                        exchange_buffer: &mut exchange_buffer,
                                    },
                                    RecomputeParams {
                                        leave_size,
                                        full_rack_forcing,
                                        min_samples: min_samples_per_rack,
                                    },
                                );
                                std::mem::swap(
                                    &mut thread_full_rack_map,
                                    &mut mutex_guard.full_rack_map,
                                );
                                std::mem::swap(
                                    &mut thread_rare_subrack_map,
                                    &mut mutex_guard.rare_subrack_map,
                                );
                                mutex_guard.undersampled_generation = current_generation;
                                mutex_guard.undersampling_comment.clear();
                                if deficit != 0 {
                                    let num_undersampled = mutex_guard.undersampled_racks.len();
                                    write!(
                                        mutex_guard.undersampling_comment,
                                        " (need to force {num_undersampled} targets over {deficit} moves)"
                                    )
                                    .unwrap();
                                }
                                undersampling_remediation_countdown.store(
                                    deficit as i64,
                                    std::sync::atomic::Ordering::Relaxed,
                                );
                            }
                            undersampled_thread_racks.clone_from(&mutex_guard.undersampled_racks);

                            if undersampled_thread_racks.is_empty() {

                                num_processed_games
                                    .fetch_sub(1, std::sync::atomic::Ordering::Relaxed);
                                break;
                            }


                            let ideal_number_of_undersampled_thread_racks = (num_threads * 32)
                                / undersampled_thread_racks.len()
                                * undersampled_thread_racks.len();


                            while undersampled_thread_racks.len()
                                < ideal_number_of_undersampled_thread_racks
                            {
                                undersampled_thread_racks.extend_from_within(
                                    ..undersampled_thread_racks.len().min(
                                        ideal_number_of_undersampled_thread_racks
                                            - undersampled_thread_racks.len(),
                                    ),
                                );
                            }
                        }
                    }

                    num_moves = 0;
                    num_bingos.iter_mut().for_each(|m| *m = 0);
                    num_turns.iter_mut().for_each(|m| *m = 0);
                    game_id.clear();

                    for _ in 0..4 {
                        game_id.push(*BASE62.choose(&mut rng).unwrap() as char);
                    }

                    num_prior_games = num_prior_games.wrapping_add(1);
                    game_id.push(BASE62[(num_prior_games / (62 * 62 * 62) % 62) as usize] as char);
                    game_id.push(BASE62[(num_prior_games / (62 * 62) % 62) as usize] as char);
                    game_id.push(BASE62[(num_prior_games / 62 % 62) as usize] as char);
                    game_id.push(BASE62[(num_prior_games % 62) as usize] as char);
                    game_state.reset_and_draw_tiles_double_ended(&game_config, &mut rng);
                    if SUMMARIZE && entering {
                        last_kept.iter_mut().for_each(|slot| *slot = None);
                    }
                    loop {
                        num_moves += 1;

                        game_state.players[game_state.turn as usize]
                            .rack
                            .sort_unstable();
                        let cur_rack = &game_state.current_player().rack;

                        let old_bag_len = game_state.bag.len();
                        if SUMMARIZE && old_bag_len > 0 {
                            cur_rack_as_vec.clone_from(cur_rack);
                        }

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


                        let mut oppdenial_exact_active = false;

                        if SUMMARIZE
                            && old_bag_len > 0
                            && let Some((lat, add, leave)) = opp_ctx
                        {
                            opp_unseen.clone_from_slice(&opp_base_freqs);
                            for &tile in game_state.board_tiles.iter() {
                                if tile != 0 {
                                    let base = tile & !((tile as i8) >> 7) as u8;
                                    opp_unseen[base as usize] =
                                        opp_unseen[base as usize].saturating_sub(1);
                                }
                            }
                            opp_sheet.iter_mut().for_each(|v| *v = 0);
                            let num_blanks_eff =
                                (opp_unseen[0] as usize).min(game_config.rack_size() as usize);
                            build_sheet_spell_once(
                                &mut move_generator,
                                &game_state.board_tiles,
                                SpellTables {
                                    game_config: &game_config,
                                    kwg: &kwg,
                                    klv: &arc_klv0,
                                    lat,
                                },
                                SpellPool {
                                    unseen_tally: &opp_unseen,
                                    num_blanks_eff,
                                    rack_size: game_config.rack_size() as usize,
                                    blank_cap: game_config.rack_size() as usize,
                                },
                                &mut opp_movegen_rack,
                                &mut opp_blank_deltas,
                                &mut opp_sheet,
                            );

                            let pool: usize = opp_unseen.iter().map(|&c| c as usize).sum();
                            let oppdenial_exact_board = oppdenial_exact != 0.0 && pool <= oppdenial_exact_pool_max;
                            if oppdenial_exact_board {
                                census::best_equity_argmax_table(
                                    lat,
                                    &opp_sheet,
                                    leave,
                                    &mut opp_best,
                                    &mut oppdenial_exact_kept_idx,
                                    &mut oppdenial_exact_kept_size,
                                );
                            } else {
                                census::best_equity_table(lat, &opp_sheet, leave, &mut opp_best);
                            }
                            if oppdenial_leave != 0.0 || oppdenial_rack != 0.0 {
                                census::opp_denial_marginals(
                                    lat,
                                    add,
                                    &opp_best,
                                    &opp_unseen,
                                    &mut opp_marginal,
                                );
                                if oppdenial_leave != 0.0 {
                                    for (a, m) in
                                        oppdenial_leave_sum_marg.iter_mut().zip(opp_marginal.iter())
                                    {
                                        *a += *m;
                                    }
                                    oppdenial_leave_boards += 1;
                                }
                            }
                            if oppdenial_exact_board {
                                oppdenial_exact_term.iter_mut().for_each(|x| *x = 0.0);
                                census::opp_me2_per_rack(
                                    lat,
                                    add,
                                    &opp_best,
                                    &census::KeptArgmax {
                                        idx: &oppdenial_exact_kept_idx,
                                        size: &oppdenial_exact_kept_size,
                                    },
                                    &opp_unseen,
                                    oppdenial_exact_me2,
                                    &mut oppdenial_exact_term,
                                );
                            }
                            oppdenial_exact_active = oppdenial_exact_board;
                        }


                        let winpct_board = WinpctBoard::from_bag(
                            winpct_table,
                            old_bag_len,
                            game_config.rack_size() as usize,
                            winpct_blend,
                        );

                        let knob = KnobFold {
                            winpct_board: &winpct_board,
                            oppdenial_rack,
                            opp_marginal: &opp_marginal,
                            oppdenial_exact,
                            oppdenial_exact_term: &oppdenial_exact_term,
                            oppdenial_exact_lat: if oppdenial_exact_active {
                                opp_ctx.map(|(lat, _, _)| lat)
                            } else {
                                None
                            },
                        };


                        if SUMMARIZE && old_bag_len > 0 && !undersampled_thread_racks.is_empty() {
                            let chosen_undersampled_thread_rack_index =
                                rng.random_range(0..undersampled_thread_racks.len());


                            unseen_tally.clone_from_slice(&alphabet_freqs);
                            for &tile in game_state.board_tiles.iter() {
                                if tile != 0 {
                                    let base = tile & !((tile as i8) >> 7) as u8;
                                    unseen_tally[base as usize] =
                                        unseen_tally[base as usize].saturating_sub(1);
                                }
                            }

                            let mut s_possible = true;
                            for &tile in undersampled_thread_racks
                                [chosen_undersampled_thread_rack_index]
                                .iter()
                            {
                                if unseen_tally[tile as usize] > 0 {
                                    unseen_tally[tile as usize] -= 1;
                                } else {
                                    s_possible = false;
                                }
                            }


                            if s_possible || impossible_ok {
                                let s_subrack = &undersampled_thread_racks
                                    [chosen_undersampled_thread_rack_index];

                                let num_filler =
                                    (game_config.rack_size() as usize).saturating_sub(s_subrack.len());
                                sample_rack_buf.clear();
                                sample_rack_buf.extend_from_slice(s_subrack);
                                unseen_pool.clear();
                                for (tile, &c) in unseen_tally.iter().enumerate() {
                                    for _ in 0..c {
                                        unseen_pool.push(tile as u8);
                                    }
                                }
                                let take = num_filler.min(unseen_pool.len());
                                for i in 0..take {
                                    let j = rng.random_range(i..unseen_pool.len());
                                    unseen_pool.swap(i, j);
                                }
                                sample_rack_buf.extend_from_slice(&unseen_pool[..take]);
                                sample_rack_buf.sort_unstable();

                                move_generator.gen_moves_unfiltered(&movegen::GenMovesParams {
                                    board_snapshot,
                                    rack: &sample_rack_buf,
                                    max_gen: 1,
                                    num_exchanges_by_this_player: game_state
                                        .current_player()
                                        .num_exchanges,
                                    pass_policy: movegen::PassPolicy::OnlyWhenForced,
                                    dynamic_leaves: None,
                                });
                                let play = &move_generator.plays[0];

                                let rounded_equity = knob.apply(play.equity, &sample_rack_buf);
                                if full_rack_forcing {

                                    pool_one(&mut thread_full_rack_map, &sample_rack_buf[..], rounded_equity);
                                } else {

                                    pool_one(&mut thread_rare_subrack_map, &s_subrack[..], rounded_equity);
                                }
                                undersampled_thread_racks
                                    .swap_remove(chosen_undersampled_thread_rack_index);
                                if undersampling_remediation_countdown
                                    .fetch_sub(1, std::sync::atomic::Ordering::Relaxed)
                                    <= 0
                                {

                                    undersampling_remediation_countdown
                                        .fetch_add(1, std::sync::atomic::Ordering::Relaxed);

                                    undersampling_remediation_generation_id
                                        .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                                }
                            }

                            let current_undersampling_remediation_generation_id =
                                undersampling_remediation_generation_id
                                    .load(std::sync::atomic::Ordering::Relaxed);
                            if undersampling_remediation_thread_generation_id
                                != current_undersampling_remediation_generation_id
                            {
                                undersampling_remediation_thread_generation_id =
                                    current_undersampling_remediation_generation_id;

                                undersampled_thread_racks.clear();
                            }
                        }

                        move_generator.gen_moves_unfiltered(&movegen::GenMovesParams {
                            board_snapshot,
                            rack: cur_rack,
                            max_gen: 1,
                            num_exchanges_by_this_player: game_state.current_player().num_exchanges,
                            pass_policy: movegen::PassPolicy::OnlyWhenForced,
                            dynamic_leaves: if game_state.turn == 0 { dyn_ref } else { None },
                        });

                        let plays = &move_generator.plays;
                        let play = &plays[0];
                        if WRITE_LOGS {
                            cur_rack_ser.clear();
                            for &tile in cur_rack.iter() {
                                cur_rack_ser
                                    .push_str(game_config.alphabet().of_rack(tile).unwrap());
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
                                aft_rack_ser
                                    .push_str(game_config.alphabet().of_rack(tile).unwrap());
                            }

                            play_fmt.clear();
                            match &play.play {
                                movegen::Play::Exchange { tiles } => {
                                    if tiles.is_empty() {
                                        play_fmt.push_str("(Pass)");
                                    } else {
                                        let alphabet = game_config.alphabet();
                                        play_fmt.push_str("(exch ");
                                        for &tile in tiles.iter() {
                                            play_fmt.push_str(alphabet.of_rack(tile).unwrap());
                                        }
                                        play_fmt.push(')');
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
                                            play_fmt.push('.');
                                        } else {
                                            play_fmt.push_str(alphabet.of_board(tile).unwrap());
                                        }
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

                        game_state.play(&game_config, &mut rng, &play.play).unwrap();

                        let old_turn = game_state.turn;
                        num_turns[old_turn as usize] += 1;
                        game_state.next_turn();
                        let new_turn = game_state.turn;
                        game_state.turn = old_turn;

                        if SUMMARIZE && old_bag_len > 0 {

                            let rounded_equity = knob.apply(play.equity, &cur_rack_as_vec);
                            if entering {

                                if let Some(l) = &last_kept[old_turn as usize]
                                    && !l.is_empty()
                                {
                                    pool_one(&mut thread_full_rack_map, &l[..], rounded_equity);
                                }

                                aft_rack_entering.clone_from(&cur_rack_as_vec);
                                match &play.play {
                                    movegen::Play::Exchange { tiles } => {
                                        game_state::use_tiles(
                                            &mut aft_rack_entering,
                                            tiles.iter().copied(),
                                        )
                                        .unwrap();
                                    }
                                    movegen::Play::Place { word, .. } => {
                                        game_state::use_tiles(
                                            &mut aft_rack_entering,
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
                                aft_rack_entering.sort_unstable();
                                match &mut last_kept[old_turn as usize] {
                                    Some(v) => v.clone_from(&aft_rack_entering),
                                    slot => *slot = Some(aft_rack_entering.clone()),
                                }
                            } else {
                                pool_one(&mut thread_full_rack_map, &cur_rack_as_vec[..], rounded_equity);
                            }
                        }

                        if WRITE_LOGS {
                            equity_fmt.clear();

                            write!(equity_fmt, "{}", play.equity).unwrap();
                        }

                        let res = {
                            let game_ended =
                                game_state.check_game_ended(&game_config, &mut final_scores);

                            match game_ended {
                                game_state::CheckGameEnded::NotEnded
                                    if !WRITE_LOGS && old_bag_len == 0 =>
                                {

                                    for (i, p) in game_state.players.iter().enumerate() {
                                        final_scores[i] = p.score;
                                    }
                                    game_state::CheckGameEnded::PlayedOut
                                }
                                _ => game_ended,
                            }
                        };
                        match res {
                            game_state::CheckGameEnded::PlayedOut
                            | game_state::CheckGameEnded::ZeroScores => {
                                let completed_moves = completed_moves
                                    .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                                completed_games.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                                if WRITE_LOGS {
                                    batched_csv_log
                                        .serialize((
                                            &player_aliases[old_turn as usize],
                                            &game_id,
                                            num_moves,
                                            &cur_rack_ser,
                                            &play_fmt,
                                            equity::descale_score(play_score),
                                            equity::descale_score(
                                                final_scores[old_turn as usize],
                                            ),
                                            tiles_played,
                                            &aft_rack_ser,
                                            &equity_fmt,
                                            old_bag_len,
                                            equity::descale_score(
                                                final_scores[new_turn as usize],
                                            ),
                                        ))
                                        .unwrap();
                                }
                                for (pts, &mp) in
                                    final_scores_pts.iter_mut().zip(final_scores.iter())
                                {
                                    *pts = equity::descale_score(mp);
                                }
                                batched_csv_game
                                    .serialize((
                                        &game_id,
                                        &final_scores_pts,
                                        &num_bingos,
                                        &num_turns,
                                        &player_aliases[0],
                                    ))
                                    .unwrap();
                                num_batched_games_here += 1;
                                if num_batched_games_here >= batch_size {
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
                                    {
                                        let mut mutex_guard = mutexed_stuffs.lock().unwrap();
                                        if WRITE_LOGS
                                            && let Some(c) = &mut mutex_guard.csv_log_writer
                                        {
                                            c.write_all(&batched_csv_log_buf).unwrap()
                                        }
                                        mutex_guard
                                            .csv_game_writer
                                            .write_all(&batched_csv_game_buf)
                                            .unwrap();
                                        if mutex_guard.tick_periods.update(elapsed_time_secs) {
                                            eprint!(
                                                "After {elapsed_time_secs} seconds, have logged {logged_games} games ({completed_moves} moves)"
                                            );
                                            if !mutex_guard.undersampling_comment.is_empty() {
                                                eprint!("{}", mutex_guard.undersampling_comment);
                                                let num_todo = undersampling_remediation_countdown
                                                    .load(std::sync::atomic::Ordering::Relaxed);
                                                if num_todo > 0 {
                                                    eprint!(" (to do: {num_todo})");
                                                }
                                            }
                                            eprintln!(" into {run_identifier}");
                                        }
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

                        if WRITE_LOGS {
                            batched_csv_log
                                .serialize((
                                    &player_aliases[old_turn as usize],
                                    &game_id,
                                    num_moves,
                                    &cur_rack_ser,
                                    &play_fmt,
                                    equity::descale_score(play_score),
                                    equity::descale_score(
                                        game_state.players[old_turn as usize].score,
                                    ),
                                    tiles_played,
                                    &aft_rack_ser,
                                    &equity_fmt,
                                    old_bag_len,
                                    equity::descale_score(
                                        game_state.players[new_turn as usize].score,
                                    ),
                                ))
                                .unwrap();
                        }
                        completed_moves.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                        game_state.turn = new_turn;
                    }
                }

                let batched_csv_log_buf = batched_csv_log.into_inner().unwrap();
                let batched_csv_game_buf = batched_csv_game.into_inner().unwrap();
                let mut mutex_guard = mutexed_stuffs.lock().unwrap();
                if WRITE_LOGS && let Some(c) = &mut mutex_guard.csv_log_writer {
                    c.write_all(&batched_csv_log_buf).unwrap();
                }
                mutex_guard
                    .csv_game_writer
                    .write_all(&batched_csv_game_buf)
                    .unwrap();

                if SUMMARIZE {
                    merge_rack_map(&mut mutex_guard.full_rack_map, &mut thread_full_rack_map);
                    merge_rack_map(
                        &mut mutex_guard.rare_subrack_map,
                        &mut thread_rare_subrack_map,
                    );

                    if oppdenial_leave != 0.0 {
                        for (a, b) in mutex_guard
                            .oppdenial_leave_sum_marg
                            .iter_mut()
                            .zip(oppdenial_leave_sum_marg.iter())
                        {
                            *a += *b;
                        }
                        mutex_guard.oppdenial_leave_boards += oppdenial_leave_boards;
                    }
                }
            }));
        }

        for thread in threads {
            if let Err(e) = thread.join() {
                eprintln!("{e:?}");
            }
        }
    });

    if SUMMARIZE {
        let mutex_guard = mutexed_stuffs.lock().unwrap();
        let full_rack_map = &mutex_guard.full_rack_map;

        let mut total_equity = 0.0;
        let mut row_count = 0;

        let mut total_sumsq = 0.0;
        for x in full_rack_map.values() {
            total_equity += x.equity;
            row_count += x.count;
            total_sumsq += x.sumsq;
        }

        eprintln!(
            "{} records, {} unique racks",
            row_count,
            full_rack_map.len()
        );

        let mut kv = full_rack_map.iter().collect::<Vec<_>>();
        kv.sort_unstable_by(|a, b| a.0.len().cmp(&b.0.len()).then_with(|| a.0.cmp(b.0)));

        let mut csv_out =
            csv::Writer::from_path(claim_output_path(&format!("summary-{run_identifier}"))?)?;
        let mut cur_rack_ser = String::new();
        csv_out.serialize(("", total_equity, row_count))?;
        for (k, fv) in kv.iter() {
            cur_rack_ser.clear();
            for &tile in k.iter() {
                cur_rack_ser.push_str(game_config.alphabet().of_rack(tile).unwrap());
            }
            csv_out.serialize((&cur_rack_ser, fv.equity, fv.count))?;
        }

        {
            let mut sq_out = csv::Writer::from_path(claim_output_path(&format!(
                "summary-sq-{run_identifier}"
            ))?)?;
            sq_out.serialize(("", total_sumsq, row_count))?;
            for (k, fv) in kv.iter() {
                cur_rack_ser.clear();
                for &tile in k.iter() {
                    cur_rack_ser.push_str(game_config.alphabet().of_rack(tile).unwrap());
                }
                sq_out.serialize((&cur_rack_ser, fv.sumsq, fv.count))?;
            }
        }

        let rare_subrack_map = &mutex_guard.rare_subrack_map;
        if !rare_subrack_map.is_empty() {
            let mut rare_kv = rare_subrack_map.iter().collect::<Vec<_>>();
            rare_kv.sort_unstable_by(|a, b| a.0.len().cmp(&b.0.len()).then_with(|| a.0.cmp(b.0)));
            let mut rare_out = csv::Writer::from_path(claim_output_path(&format!(
                "summary-rare-{run_identifier}"
            ))?)?;
            for (k, fv) in rare_kv.iter() {
                cur_rack_ser.clear();
                for &tile in k.iter() {
                    cur_rack_ser.push_str(game_config.alphabet().of_rack(tile).unwrap());
                }
                rare_out.serialize((&cur_rack_ser, fv.equity, fv.count))?;
            }
            eprintln!(
                "{} rare samples over {} unique subracks into summary-rare-{run_identifier}",
                rare_subrack_map.values().fold(0u64, |a, x| a + x.count),
                rare_subrack_map.len(),
            );
        }

        if oppdenial_leave != 0.0 && mutex_guard.oppdenial_leave_boards > 0 {
            write_oppdenial_leave_marginal_sidecar(
                &mutex_guard.oppdenial_leave_sum_marg,
                mutex_guard.oppdenial_leave_boards,
            )?;
        }
    }

    eprintln!(
        "After {} seconds, have logged {} games ({} moves) into {}",
        t0.elapsed().as_secs(),
        completed_games.load(std::sync::atomic::Ordering::Relaxed),
        completed_moves.load(std::sync::atomic::Ordering::Relaxed),
        run_identifier
    );

    Ok(())
}

fn env_usize(name: &str, default: usize) -> usize {
    env_parse(name, default)
}

fn env_path(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|x| !x.is_empty())
}

#[derive(Clone, Copy)]
enum GillesRealRack {
    Off,
    AllTurns,
    InWindow,
}

fn wolges_gilles_real_rack() -> error::Returns<GillesRealRack> {
    match std::env::var("WOLGES_GILLES_REAL_RACK").ok().as_deref() {
        None | Some("off") => Ok(GillesRealRack::Off),
        Some("all-turns") => Ok(GillesRealRack::AllTurns),
        Some("in-window") => Ok(GillesRealRack::InWindow),
        Some(other) => Err(format!(
            "WOLGES_GILLES_REAL_RACK must be off, all-turns, or in-window, got {other:?}"
        )
        .into()),
    }
}

fn parse_board_counts(spec: &str) -> error::Returns<Vec<u64>> {
    let mut out = Vec::new();
    for part in spec.split(',') {
        let part = part.trim();
        if part.is_empty() {
            return Err("census board-count spec has an empty element".into());
        }
        if let Some((k, n)) = part.split_once('x') {
            let k: u64 = k.trim().parse()?;
            let n: u64 = n.trim().parse()?;
            for _ in 0..k {
                out.push(n);
            }
        } else {
            out.push(part.parse()?);
        }
    }
    if out.is_empty() {
        return Err("census board-count spec is empty".into());
    }
    Ok(out)
}

fn generate_gilles_summary<N: kwg::Node + Sync + Send, L: kwg::Node + Sync + Send>(
    game_config: game_config::GameConfig,
    kwg: kwg::Kwg<N>,
    arc_klv0: std::sync::Arc<klv::Klv<L>>,
    arc_klv1: std::sync::Arc<klv::Klv<L>>,
    num_games: u64,
    min_samples: u64,
    seed: Option<u64>,
) -> error::Returns<()> {
    let game_config = std::sync::Arc::new(game_config);
    let kwg = std::sync::Arc::new(kwg);
    let seed = seed.unwrap_or_else(rand::random);
    eprintln!("seed: {seed}");
    let num_threads = wolges_threads();

    let run_identifier = format!("gilles-summary-{}", run_stamp());

    let rack_size = game_config.rack_size();
    let num_tiles: u32 = {
        let alphabet = game_config.alphabet();
        (0..alphabet.len()).map(|t| alphabet.freq(t) as u32).sum()
    };

    let pool_min = env_usize("WOLGES_POOL_MIN", (num_tiles / 4) as usize);
    let pool_max = env_usize(
        "WOLGES_POOL_MAX",
        (num_tiles as usize).saturating_sub(pool_min),
    );
    let group_size = env_usize(
        "WOLGES_GILLES_GROUP",
        (2 * rack_size as usize).saturating_sub(1),
    )
    .max(rack_size as usize);
    let num_draws = env_usize("WOLGES_GILLES_DRAWS", 10);
    let turn_stride = env_usize("WOLGES_GILLES_STRIDE", 3) as u32;

    let samples_per_snapshot = env_usize(
        "WOLGES_GILLES_SAMPLES_PER_SNAPSHOT",
        n_choose_k(group_size, rack_size as usize),
    ) as u32;
    let min_undersampled = env_usize(
        "WOLGES_GILLES_MIN_UNDERSAMPLED",
        samples_per_snapshot as usize,
    )
    .min(samples_per_snapshot as usize) as u32;
    let growth_cap = env_usize("WOLGES_GILLES_GROWTH", rack_size as usize);
    let max_no_progress = env_usize("WOLGES_GILLES_MAX_NO_PROGRESS", 2) as u32;
    let force_recompute_games = env_usize("WOLGES_GILLES_FORCE_RECOMPUTE_GAMES", 2000) as u64;

    let (real_rack_enabled, real_rack_in_window_only, real_rack_mode) =
        match wolges_gilles_real_rack()? {
            GillesRealRack::Off => (false, false, "off"),
            GillesRealRack::AllTurns => (true, false, "all-turns"),
            GillesRealRack::InWindow => (true, true, "in-window"),
        };

    let real_rack_weight = env_usize("WOLGES_GILLES_REAL_RACK_WEIGHT", 1) as u64;

    let reserve_enabled = env_flag("WOLGES_GILLES_RESERVE", false);
    let reserve_budget = env_usize(
        "WOLGES_GILLES_RESERVE_BUDGET",
        (num_tiles as usize).saturating_sub(
            pool_min + rack_size as usize * game_config.num_players() as usize + rack_size as usize,
        ),
    );

    let oppdenial_leave = env_parse::<f64>("WOLGES_OPPDENIAL_LEAVE", 0.0);

    let oppdenial_rack = env_parse::<f64>("WOLGES_OPPDENIAL_RACK", 0.0);

    let oppdenial_exact = env_parse::<f64>("WOLGES_OPPDENIAL_EXACT", 0.0);
    let oppdenial_exact_pool_max = env_usize("WOLGES_OPPDENIAL_EXACT_POOL_MAX", 32);

    let oppdenial_exact_me2 = env_parse::<f64>("WOLGES_OPPDENIAL_EXACT_ME2", 1.0);

    let winpct_table: Option<win_pct::WinPctTable> = if env_flag("WOLGES_WINPCT", false) {
        let path =
            std::env::var("WOLGES_WINPCT_TABLE").unwrap_or_else(|_| "win_pct.csv".to_string());
        let t = win_pct::WinPctTable::from_csv(&std::fs::read_to_string(&path)?)?;
        eprintln!("gilles: win%-objective from {path}");
        Some(t)
    } else {
        None
    };

    let winpct_blend = env_parse::<f64>("WOLGES_WINPCT_BLEND", 1.0);
    let opp_on = (oppdenial_leave != 0.0 || oppdenial_rack != 0.0 || oppdenial_exact != 0.0)
        && winpct_table.is_none();

    let opp_ctx: Option<(census::MultisetLattice, census::AddTable, Vec<i32>)> = if opp_on {
        let num_letters = game_config.alphabet().len() as usize;
        let lat = census::MultisetLattice::new(num_letters, rack_size as usize);
        let add_table = census::AddTable::new(&lat);
        let mut leave = vec![0i32; lat.len()];
        census::fill_lattice_leaves(&lat, &mut leave, |tally| {
            arc_klv0.leave_value_from_tally(tally)
        });
        eprintln!(
            "gilles: WOLGES_OPPDENIAL_LEAVE={oppdenial_leave} WOLGES_OPPDENIAL_RACK={oppdenial_rack} WOLGES_OPPDENIAL_EXACT={oppdenial_exact} \
             oppdenial_exact_pool_max={oppdenial_exact_pool_max} opponent-denial machinery on ({} lattice leaves)",
            lat.len(),
        );
        Some((lat, add_table, leave))
    } else {
        None
    };

    let dynamic_leaves_on = std::env::var("WOLGES_DYNAMIC_LEAVES")
        .ok()
        .and_then(|s| s.parse::<u64>().ok())
        .unwrap_or(0)
        != 0;
    let dynamic_min_keep = std::env::var("WOLGES_DYNAMIC_LEAVES_MIN_KEEP")
        .ok()
        .and_then(|s| s.parse::<usize>().ok())
        .unwrap_or(2);
    let dyn_ctx: Option<(census::MultisetLattice, census::AddTable, Vec<i32>)> =
        if dynamic_leaves_on {
            let num_letters = game_config.alphabet().len() as usize;
            let lat = census::MultisetLattice::new(num_letters, rack_size as usize);
            let add = census::AddTable::new_with_threads(&lat, num_threads);
            let mut full_v = vec![0i32; lat.len()];
            census::fill_lattice_leaves(&lat, &mut full_v, |tally| {
                arc_klv0.leave_value_from_tally(tally)
            });
            Some((lat, add, full_v))
        } else {
            None
        };
    let dyn_ref = dyn_ctx
        .as_ref()
        .map(|(lat, add, full_v)| klv::DynamicLeavesRef {
            lat,
            add,
            full_v: full_v.as_slice(),
            min_keep: dynamic_min_keep,
        });
    eprintln!(
        "WOLGES_DYNAMIC_LEAVES={} WOLGES_DYNAMIC_LEAVES_MIN_KEEP={dynamic_min_keep} ({})",
        dynamic_leaves_on as u8,
        if dynamic_leaves_on {
            "dynamic leaves on for the klv0 side; needs a --full (len 1-7) klv0"
        } else {
            "off, static leaves"
        },
    );
    eprintln!(
        "gilles: rack_size={rack_size} num_tiles={num_tiles} snapshot_pool={pool_min}..={pool_max} group_size={group_size} draws={num_draws} stride={turn_stride} min_samples={min_samples} samples_per_snapshot={samples_per_snapshot} min_undersampled={min_undersampled} growth_cap={growth_cap} reserve={reserve_enabled} reserve_budget={reserve_budget} real_rack={real_rack_mode}"
    );

    let num_processed_games = std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0));
    let completed_games = std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0));
    let completed_samples = std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0));

    let remediation_state =
        std::sync::Arc::new(std::sync::atomic::AtomicU64::new(if min_samples == 0 {
            3
        } else {
            0
        }));
    let remediation_submission = std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0));
    let remediation_countdown = std::sync::Arc::new(std::sync::atomic::AtomicI64::new(0));
    let remediation_generation_id = std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0));
    let mutexed = std::sync::Arc::new(std::sync::Mutex::new(GillesMutexed {
        full_rack_map: fash::MyHashMap::<bites::Bites, Cumulate>::default(),
        undersampled_racks: Vec::new(),
        best_remaining: u64::MAX,
        no_progress: 0,
        oppdenial_leave_sum_marg: if oppdenial_leave != 0.0 {
            vec![0f64; game_config.alphabet().len() as usize]
        } else {
            Vec::new()
        },
        oppdenial_leave_boards: 0,
    }));
    let mutexed_tick = std::sync::Arc::new(std::sync::Mutex::new(move_picker::Periods(0)));
    let t0 = std::time::Instant::now();

    std::thread::scope(|s| {
        let mut threads = vec![];
        for _ in 0..num_threads {
            let game_config = std::sync::Arc::clone(&game_config);
            let kwg = std::sync::Arc::clone(&kwg);
            let arc_klv0 = std::sync::Arc::clone(&arc_klv0);
            let arc_klv1 = std::sync::Arc::clone(&arc_klv1);
            let num_processed_games = std::sync::Arc::clone(&num_processed_games);
            let completed_games = std::sync::Arc::clone(&completed_games);
            let completed_samples = std::sync::Arc::clone(&completed_samples);
            let remediation_state = std::sync::Arc::clone(&remediation_state);
            let remediation_submission = std::sync::Arc::clone(&remediation_submission);
            let remediation_countdown = std::sync::Arc::clone(&remediation_countdown);
            let remediation_generation_id = std::sync::Arc::clone(&remediation_generation_id);
            let mutexed = std::sync::Arc::clone(&mutexed);
            let mutexed_tick = std::sync::Arc::clone(&mutexed_tick);
            let run_identifier = run_identifier.clone();
            let opp_ctx = opp_ctx.as_ref();
            let winpct_table = winpct_table.as_ref();
            threads.push(s.spawn(move || {
                let mut rng = rand::rngs::ChaCha20Rng::seed_from_u64(seed);
                let mut move_generator = movegen::KurniaMoveGenerator::new(&game_config);
                let mut game_state = game_state::GameState::new(&game_config);
                let alphabet = game_config.alphabet();
                let num_letters = alphabet.len() as usize;
                let base_freqs = (0..alphabet.len())
                    .map(|t| alphabet.freq(t))
                    .collect::<Vec<u8>>();

                let impossible = env_flag("WOLGES_IMPOSSIBLE_OK", true);
                let mut unseen_tally = vec![0u8; num_letters];
                let mut cand_tally = vec![0u8; num_letters];
                let mut best_group_tally = vec![0u8; num_letters];
                let mut grown_tally = vec![0u8; num_letters];
                let mut rack_tally = vec![0u8; num_letters];
                let mut unseen_pool = Vec::<u8>::new();
                let mut group_pool = Vec::<u8>::new();
                let mut exchange_buffer = Vec::with_capacity(rack_size as usize);
                let mut thread_map = fash::MyHashMap::<bites::Bites, Cumulate>::default();
                let mut final_scores = vec![0; game_config.num_players() as usize];

                let mut local_undersampled = fash::MyHashSet::<bites::Bites>::default();
                let mut reserved_tally = vec![0u8; num_letters];
                let mut real_rack_buf = Vec::<u8>::with_capacity(rack_size as usize);

                let mut opp_sheet: Vec<i32> = Vec::new();
                let mut opp_best: Vec<i32> = Vec::new();
                let mut opp_marginal: Vec<f64> = Vec::new();
                let mut opp_movegen_rack: Vec<u8> = Vec::new();
                let mut opp_blank_deltas: Vec<(u8, i32)> = Vec::new();

                let mut oppdenial_exact_kept_idx: Vec<u32> = Vec::new();
                let mut oppdenial_exact_kept_size: Vec<u8> = Vec::new();
                let mut oppdenial_exact_term: Vec<f64> = Vec::new();
                if let Some((lat, _, _)) = opp_ctx {
                    opp_sheet = vec![0i32; lat.len()];
                    opp_best = vec![census::UNPLAYABLE; lat.len()];
                    opp_marginal = vec![0f64; num_letters];
                    if oppdenial_exact != 0.0 {
                        oppdenial_exact_kept_idx = vec![0u32; lat.len()];
                        oppdenial_exact_kept_size = vec![0u8; lat.len()];
                        oppdenial_exact_term = vec![0f64; lat.len()];
                    }
                }

                let mut oppdenial_leave_sum_marg: Vec<f64> = if oppdenial_leave != 0.0 {
                    vec![0f64; num_letters]
                } else {
                    Vec::new()
                };
                let mut oppdenial_leave_boards = 0u64;
                let mut remediation_begun = false;
                let mut thread_generation_id = 0u64;
                let mut games_this_gen = 0u64;

                let ln_fact = {
                    let mut v = vec![0.0f64; num_tiles as usize + 1];
                    for i in 2..v.len() {
                        v[i] = v[i - 1] + (i as f64).ln();
                    }
                    v
                };
                let ln_choose = |n: usize, k: usize| -> f64 {
                    if k > n {
                        f64::NEG_INFINITY
                    } else {
                        ln_fact[n] - ln_fact[k] - ln_fact[n - k]
                    }
                };

                loop {
                    let num_prior_games =
                        num_processed_games.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    let remediating = num_prior_games >= num_games;
                    if remediating {
                        if min_samples == 0 {
                            num_processed_games.fetch_sub(1, std::sync::atomic::Ordering::Relaxed);
                            break;
                        }

                        if !remediation_begun {
                            {
                                let mut g = mutexed.lock().unwrap();
                                merge_rack_map(&mut g.full_rack_map, &mut thread_map);
                            }
                            remediation_submission
                                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                            while remediation_submission.load(std::sync::atomic::Ordering::Relaxed)
                                != num_threads as u64
                            {}
                            if remediation_state
                                .compare_exchange(
                                    0,
                                    1,
                                    std::sync::atomic::Ordering::Relaxed,
                                    std::sync::atomic::Ordering::Relaxed,
                                )
                                .is_ok()
                            {
                                let mut g = mutexed.lock().unwrap();
                                let remaining = recompute_undersampled(
                                    &mut g,
                                    &mut thread_map,
                                    &base_freqs,
                                    &mut rack_tally,
                                    &mut exchange_buffer,
                                    rack_size,
                                    min_samples,
                                );
                                g.best_remaining = remaining;
                                remediation_countdown
                                    .store(remaining as i64, std::sync::atomic::Ordering::Relaxed);
                                eprintln!(
                                    "After {} seconds, remediation begins: {} racks below min_samples, {remaining} total deficit, into {run_identifier}",
                                    t0.elapsed().as_secs(),
                                    g.undersampled_racks.len(),
                                );
                                remediation_state.store(2, std::sync::atomic::Ordering::Relaxed);
                            } else {
                                while remediation_state.load(std::sync::atomic::Ordering::Relaxed)
                                    < 2
                                {}
                            }
                            remediation_begun = true;
                        }

                        let cur_gen =
                            remediation_generation_id.load(std::sync::atomic::Ordering::Relaxed);
                        if thread_generation_id != cur_gen
                            || local_undersampled.is_empty()
                            || remediation_countdown.load(std::sync::atomic::Ordering::Relaxed) <= 0
                            || games_this_gen >= force_recompute_games
                        {
                            let mut g = mutexed.lock().unwrap();
                            merge_rack_map(&mut g.full_rack_map, &mut thread_map);
                            let want_recompute = (remediation_countdown
                                .load(std::sync::atomic::Ordering::Relaxed)
                                <= 0
                                || games_this_gen >= force_recompute_games)
                                && remediation_generation_id
                                    .load(std::sync::atomic::Ordering::Relaxed)
                                    == cur_gen;
                            if want_recompute {
                                let remaining = recompute_undersampled(
                                    &mut g,
                                    &mut thread_map,
                                    &base_freqs,
                                    &mut rack_tally,
                                    &mut exchange_buffer,
                                    rack_size,
                                    min_samples,
                                );
                                if remaining == 0 || remaining >= g.best_remaining {
                                    g.no_progress += 1;
                                } else {
                                    g.no_progress = 0;
                                    g.best_remaining = remaining;
                                }
                                if remaining == 0 || g.no_progress >= max_no_progress {
                                    remediation_state
                                        .store(3, std::sync::atomic::Ordering::Relaxed);
                                }
                                remediation_countdown
                                    .store(remaining as i64, std::sync::atomic::Ordering::Relaxed);
                                remediation_generation_id
                                    .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                                eprintln!(
                                    "After {} seconds, remediation recompute: {} racks below min_samples, {remaining} deficit, {} samples, into {run_identifier}",
                                    t0.elapsed().as_secs(),
                                    g.undersampled_racks.len(),
                                    completed_samples.load(std::sync::atomic::Ordering::Relaxed),
                                );
                            }
                            games_this_gen = 0;
                            thread_generation_id = remediation_generation_id
                                .load(std::sync::atomic::Ordering::Relaxed);
                            local_undersampled.clear();
                            for r in g.undersampled_racks.iter() {
                                local_undersampled.insert(r.clone());
                            }
                        }
                        if remediation_state.load(std::sync::atomic::Ordering::Relaxed) >= 3 {
                            num_processed_games.fetch_sub(1, std::sync::atomic::Ordering::Relaxed);
                            break;
                        }
                        games_this_gen += 1;
                    }

                    rng.set_stream(num_prior_games);
                    if remediating && reserve_enabled && !local_undersampled.is_empty() {

                        reserved_tally.iter_mut().for_each(|m| *m = 0);
                        let mut reserved_total = 0usize;
                        for rack in local_undersampled.iter().take(1024) {
                            if reserved_total + rack_size as usize > reserve_budget {
                                break;
                            }

                            let mut delta = 0usize;
                            let mut i = 0;
                            while i < rack.len() {
                                let t = rack[i] as usize;
                                let mut c = 0u8;
                                while i < rack.len() && rack[i] as usize == t {
                                    c += 1;
                                    i += 1;
                                }
                                if c > reserved_tally[t] {
                                    delta += (c - reserved_tally[t]) as usize;
                                }
                            }
                            if reserved_total + delta > reserve_budget {
                                continue;
                            }
                            let mut i = 0;
                            while i < rack.len() {
                                let t = rack[i] as usize;
                                let mut c = 0u8;
                                while i < rack.len() && rack[i] as usize == t {
                                    c += 1;
                                    i += 1;
                                }
                                if c > reserved_tally[t] {
                                    reserved_tally[t] = c;
                                }
                            }
                            reserved_total += delta;
                        }
                        game_state.reset();
                        game_state.bag.shuffle(&mut rng);
                        for (t, &c) in reserved_tally.iter().enumerate() {
                            for _ in 0..c {
                                game_state.bag.remove_tile(t as u8);
                            }
                        }
                        let rsz = game_config.rack_size() as usize;
                        let bag = &mut game_state.bag;
                        let players = &mut game_state.players;
                        for (i, player) in players.iter_mut().enumerate() {
                            bag.replenish(&mut player.rack, rsz, i);
                        }
                    } else {
                        game_state.reset_and_draw_tiles_double_ended(&game_config, &mut rng);
                    }

                    let mut turn_idx = 0u32;
                    let mut base_turn: Option<u32> = None;
                    loop {
                        let board_tiles_count =
                            game_state.board_tiles.iter().filter(|&&t| t != 0).count();

                        let pool_count = (num_tiles as usize).saturating_sub(board_tiles_count);

                        let winpct_board =
                            WinpctBoard::new(winpct_table, pool_count, rack_size as usize, winpct_blend);

                        if pool_count >= pool_min
                            && pool_count <= pool_max
                            && board_tiles_count > 0
                            && !game_state.bag.is_empty()
                            && (turn_idx - *base_turn.get_or_insert(turn_idx))
                                .is_multiple_of(turn_stride)
                        {

                            unseen_tally.clone_from_slice(&base_freqs);
                            for &t in game_state.board_tiles.iter() {
                                if t != 0 {
                                    let base = t & !((t as i8) >> 7) as u8;
                                    unseen_tally[base as usize] =
                                        unseen_tally[base as usize].saturating_sub(1);
                                }
                            }

                            let group_src: &[u8] =
                                if impossible { &base_freqs } else { &unseen_tally };
                            let num_unseen =
                                group_src.iter().map(|&c| c as usize).sum::<usize>();
                            if num_unseen >= group_size {
                                unseen_pool.clear();
                                for (tile, &c) in group_src.iter().enumerate() {
                                    for _ in 0..c {
                                        unseen_pool.push(tile as u8);
                                    }
                                }

                                let mut best_lnp = f64::INFINITY;
                                for _ in 0..num_draws {
                                    for i in 0..group_size {
                                        let j = rng.random_range(i..unseen_pool.len());
                                        unseen_pool.swap(i, j);
                                    }
                                    cand_tally.iter_mut().for_each(|m| *m = 0);
                                    for &t in &unseen_pool[..group_size] {
                                        cand_tally[t as usize] += 1;
                                    }
                                    let mut lnp = 0.0f64;
                                    for (tile, &k) in cand_tally.iter().enumerate() {
                                        if k > 0 {
                                            lnp +=
                                                ln_choose(group_src[tile] as usize, k as usize);
                                        }
                                    }
                                    if lnp < best_lnp {
                                        best_lnp = lnp;
                                        best_group_tally.clone_from(&cand_tally);
                                    }
                                }

                                let board_snapshot = movegen::BoardSnapshot {
                                    board_tiles: &game_state.board_tiles,
                                    game_config: &game_config,
                                    kwg: &kwg,
                                    klv: if game_state.turn == 0 {
                                        &arc_klv0
                                    } else {
                                        &arc_klv1
                                    },
                                };


                                let mut oppdenial_exact_active = false;

                                if let Some((lat, add, leave)) = opp_ctx {
                                    opp_sheet.iter_mut().for_each(|v| *v = 0);
                                    let num_blanks_eff =
                                        (unseen_tally[0] as usize).min(rack_size as usize);
                                    build_sheet_spell_once(
                                        &mut move_generator,
                                        &game_state.board_tiles,
                                        SpellTables {
                                            game_config: &game_config,
                                            kwg: &kwg,
                                            klv: &arc_klv0,
                                            lat,
                                        },
                                        SpellPool {
                                            unseen_tally: &unseen_tally,
                                            num_blanks_eff,
                                            rack_size: rack_size as usize,
                                            blank_cap: rack_size as usize,
                                        },
                                        &mut opp_movegen_rack,
                                        &mut opp_blank_deltas,
                                        &mut opp_sheet,
                                    );

                                    let pool: usize =
                                        unseen_tally.iter().map(|&c| c as usize).sum();
                                    let oppdenial_exact_board = oppdenial_exact != 0.0 && pool <= oppdenial_exact_pool_max;
                                    if oppdenial_exact_board {
                                        census::best_equity_argmax_table(
                                            lat,
                                            &opp_sheet,
                                            leave,
                                            &mut opp_best,
                                            &mut oppdenial_exact_kept_idx,
                                            &mut oppdenial_exact_kept_size,
                                        );
                                    } else {
                                        census::best_equity_table(
                                            lat,
                                            &opp_sheet,
                                            leave,
                                            &mut opp_best,
                                        );
                                    }
                                    if oppdenial_leave != 0.0 || oppdenial_rack != 0.0 {
                                        census::opp_denial_marginals(
                                            lat,
                                            add,
                                            &opp_best,
                                            &unseen_tally,
                                            &mut opp_marginal,
                                        );
                                        if oppdenial_leave != 0.0 {
                                            for (a, m) in
                                                oppdenial_leave_sum_marg.iter_mut().zip(opp_marginal.iter())
                                            {
                                                *a += *m;
                                            }
                                            oppdenial_leave_boards += 1;
                                        }
                                    }
                                    if oppdenial_exact_board {
                                        oppdenial_exact_term.iter_mut().for_each(|x| *x = 0.0);
                                        census::opp_me2_per_rack(
                                            lat,
                                            add,
                                            &opp_best,
                                            &census::KeptArgmax {
                                                idx: &oppdenial_exact_kept_idx,
                                                size: &oppdenial_exact_kept_size,
                                            },
                                            &unseen_tally,
                                            oppdenial_exact_me2,
                                            &mut oppdenial_exact_term,
                                        );
                                    }
                                    oppdenial_exact_active = oppdenial_exact_board;
                                }


                                let knob = KnobFold {
                                    winpct_board: &winpct_board,
                                    oppdenial_rack,
                                    opp_marginal: &opp_marginal,
                                    oppdenial_exact,
                                    oppdenial_exact_term: &oppdenial_exact_term,
                                    oppdenial_exact_lat: if oppdenial_exact_active {
                                        opp_ctx.map(|(lat, _, _)| lat)
                                    } else {
                                        None
                                    },
                                };

                                if !remediating {

                                    rack_tally.clone_from(&best_group_tally);
                                    let move_generator = &mut move_generator;
                                    let thread_map = &mut thread_map;
                                    let completed_samples = &completed_samples;
                                    generate_exchanges(&mut ExchangeEnv {
                                        found_exchange_move: |rack_bytes: &[u8]| {
                                            move_generator.gen_moves_unfiltered(
                                                &movegen::GenMovesParams {
                                                    board_snapshot: &board_snapshot,
                                                    rack: rack_bytes,
                                                    max_gen: 1,
                                                    num_exchanges_by_this_player: 0,
                                                    pass_policy: movegen::PassPolicy::OnlyWhenForced,
                                                    dynamic_leaves: None,
                                                },
                                            );
                                            let equity =
                                                knob.apply(move_generator.plays[0].equity, rack_bytes);
                                            pool_one(thread_map, rack_bytes, equity);
                                            completed_samples
                                                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                                        },
                                        rack_tally: &mut rack_tally,
                                        min_len: rack_size,
                                        max_len: rack_size,
                                        exchange_buffer: &mut exchange_buffer,
                                    });
                                } else {

                                    let mut movegens_done = 0u32;
                                    let mut undersampled_done = 0u32;

                                    rack_tally.clone_from(&best_group_tally);
                                    sample_undersampled(
                                        rack_size,
                                        &mut move_generator,
                                        &board_snapshot,
                                        &mut thread_map,
                                        &mut local_undersampled,
                                        SampleScratch {
                                            rack_tally: &mut rack_tally,
                                            exchange_buffer: &mut exchange_buffer,
                                        },
                                        SampleBudget {
                                            countdown: &remediation_countdown,
                                            completed_samples: &completed_samples,
                                            movegens_done: &mut movegens_done,
                                            undersampled_done: &mut undersampled_done,
                                            samples_per_snapshot,
                                            target: samples_per_snapshot,
                                            knob,
                                        },
                                    );

                                    let mut grew = false;
                                    if undersampled_done < min_undersampled {
                                        grown_tally.clone_from(&best_group_tally);
                                        let mut cur_group = group_size;
                                        while cur_group < num_unseen
                                            && cur_group - group_size < growth_cap
                                        {
                                            let mut best_i = usize::MAX;
                                            let mut best_ratio = f64::INFINITY;
                                            for i in 0..num_letters {
                                                if unseen_tally[i] as usize
                                                    > grown_tally[i] as usize
                                                {
                                                    let ratio = (unseen_tally[i] as f64
                                                        - grown_tally[i] as f64)
                                                        / (grown_tally[i] as f64 + 1.0);
                                                    if ratio < best_ratio {
                                                        best_ratio = ratio;
                                                        best_i = i;
                                                    }
                                                }
                                            }
                                            if best_i == usize::MAX {
                                                break;
                                            }
                                            grown_tally[best_i] += 1;
                                            cur_group += 1;
                                            grew = true;
                                        }
                                        if grew {
                                            rack_tally.clone_from(&grown_tally);
                                            sample_undersampled(
                                                rack_size,
                                                &mut move_generator,
                                                &board_snapshot,
                                                &mut thread_map,
                                                &mut local_undersampled,
                                                SampleScratch {
                                                    rack_tally: &mut rack_tally,
                                                    exchange_buffer: &mut exchange_buffer,
                                                },
                                                SampleBudget {
                                                    countdown: &remediation_countdown,
                                                    completed_samples: &completed_samples,
                                                    movegens_done: &mut movegens_done,
                                                    undersampled_done: &mut undersampled_done,
                                                    samples_per_snapshot,
                                                    target: min_undersampled,
                                                    knob,
                                                },
                                            );
                                        }
                                    }

                                    if movegens_done < samples_per_snapshot {
                                        group_pool.clear();
                                        let cur_tally = if grew {
                                            &grown_tally
                                        } else {
                                            &best_group_tally
                                        };
                                        for (tile, &c) in cur_tally.iter().enumerate() {
                                            for _ in 0..c {
                                                group_pool.push(tile as u8);
                                            }
                                        }
                                        while movegens_done < samples_per_snapshot
                                            && group_pool.len() >= rack_size as usize
                                        {
                                            for i in 0..rack_size as usize {
                                                let j = rng.random_range(i..group_pool.len());
                                                group_pool.swap(i, j);
                                            }
                                            exchange_buffer.clear();
                                            exchange_buffer.extend_from_slice(
                                                &group_pool[..rack_size as usize],
                                            );
                                            exchange_buffer.sort_unstable();
                                            move_generator.gen_moves_unfiltered(
                                                &movegen::GenMovesParams {
                                                    board_snapshot: &board_snapshot,
                                                    rack: &exchange_buffer,
                                                    max_gen: 1,
                                                    num_exchanges_by_this_player: 0,
                                                    pass_policy: movegen::PassPolicy::OnlyWhenForced,
                                                    dynamic_leaves: None,
                                                },
                                            );
                                            let equity =
                                                knob.apply(move_generator.plays[0].equity, &exchange_buffer);
                                            pool_one(&mut thread_map, &exchange_buffer[..], equity);
                                            completed_samples
                                                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                                            movegens_done += 1;
                                            if local_undersampled.remove(&exchange_buffer[..]) {
                                                remediation_countdown.fetch_sub(
                                                    1,
                                                    std::sync::atomic::Ordering::Relaxed,
                                                );
                                            }
                                        }
                                    }
                                }
                            }
                        }


                        let real_rack_here = real_rack_enabled
                            && !game_state.bag.is_empty()
                            && (!real_rack_in_window_only
                                || (pool_count >= pool_min && pool_count <= pool_max));
                        let mut rr_oppdenial_exact_active = false;
                        if real_rack_here
                            && let Some((lat, add, leave)) = opp_ctx
                        {

                            unseen_tally.clone_from_slice(&base_freqs);
                            for &t in game_state.board_tiles.iter() {
                                if t != 0 {
                                    let base = t & !((t as i8) >> 7) as u8;
                                    unseen_tally[base as usize] =
                                        unseen_tally[base as usize].saturating_sub(1);
                                }
                            }
                            opp_sheet.iter_mut().for_each(|v| *v = 0);
                            let num_blanks_eff =
                                (unseen_tally[0] as usize).min(rack_size as usize);
                            build_sheet_spell_once(
                                &mut move_generator,
                                &game_state.board_tiles,
                                SpellTables {
                                    game_config: &game_config,
                                    kwg: &kwg,
                                    klv: &arc_klv0,
                                    lat,
                                },
                                SpellPool {
                                    unseen_tally: &unseen_tally,
                                    num_blanks_eff,
                                    rack_size: rack_size as usize,
                                    blank_cap: rack_size as usize,
                                },
                                &mut opp_movegen_rack,
                                &mut opp_blank_deltas,
                                &mut opp_sheet,
                            );
                            let pool: usize = unseen_tally.iter().map(|&c| c as usize).sum();
                            let oppdenial_exact_board = oppdenial_exact != 0.0 && pool <= oppdenial_exact_pool_max;
                            if oppdenial_exact_board {
                                census::best_equity_argmax_table(
                                    lat,
                                    &opp_sheet,
                                    leave,
                                    &mut opp_best,
                                    &mut oppdenial_exact_kept_idx,
                                    &mut oppdenial_exact_kept_size,
                                );
                            } else {
                                census::best_equity_table(lat, &opp_sheet, leave, &mut opp_best);
                            }

                            if oppdenial_leave != 0.0 || oppdenial_rack != 0.0 {
                                census::opp_denial_marginals(
                                    lat,
                                    add,
                                    &opp_best,
                                    &unseen_tally,
                                    &mut opp_marginal,
                                );
                            }
                            if oppdenial_exact_board {
                                oppdenial_exact_term.iter_mut().for_each(|x| *x = 0.0);
                                census::opp_me2_per_rack(
                                    lat,
                                    add,
                                    &opp_best,
                                    &census::KeptArgmax {
                                        idx: &oppdenial_exact_kept_idx,
                                        size: &oppdenial_exact_kept_size,
                                    },
                                    &unseen_tally,
                                    oppdenial_exact_me2,
                                    &mut oppdenial_exact_term,
                                );
                            }
                            rr_oppdenial_exact_active = oppdenial_exact_board;
                        }


                        let board_snapshot = movegen::BoardSnapshot {
                            board_tiles: &game_state.board_tiles,
                            game_config: &game_config,
                            kwg: &kwg,
                            klv: if game_state.turn == 0 {
                                &arc_klv0
                            } else {
                                &arc_klv1
                            },
                        };
                        move_generator.gen_moves_unfiltered(&movegen::GenMovesParams {
                            board_snapshot: &board_snapshot,
                            rack: &game_state.current_player().rack,
                            max_gen: 1,
                            num_exchanges_by_this_player: game_state.current_player().num_exchanges,
                            pass_policy: movegen::PassPolicy::OnlyWhenForced,
                            dynamic_leaves: if game_state.turn == 0 { dyn_ref } else { None },
                        });

                        if real_rack_here {
                            let w = real_rack_weight;
                            real_rack_buf.clone_from(&game_state.current_player().rack);
                            real_rack_buf.sort_unstable();

                            let rr_knob = KnobFold {
                                winpct_board: &winpct_board,
                                oppdenial_rack,
                                opp_marginal: &opp_marginal,
                                oppdenial_exact,
                                oppdenial_exact_term: &oppdenial_exact_term,
                                oppdenial_exact_lat: if rr_oppdenial_exact_active {
                                    opp_ctx.map(|(lat, _, _)| lat)
                                } else {
                                    None
                                },
                            };
                            let eq = rr_knob.apply(move_generator.plays[0].equity, &real_rack_buf)
                                * w as f64;

                            let sumsq_w = eq.powi(2) / w as f64;
                            thread_map
                                .entry(real_rack_buf[..].into())
                                .and_modify(|e| {
                                    e.equity += eq;
                                    e.count += w;
                                    e.sumsq += sumsq_w;
                                })
                                .or_insert(Cumulate {
                                    equity: eq,
                                    count: w,
                                    sumsq: sumsq_w,
                                });
                            completed_samples.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                        }
                        let play = &move_generator.plays[0];
                        game_state.play(&game_config, &mut rng, &play.play).unwrap();
                        let game_ended =
                            game_state.check_game_ended(&game_config, &mut final_scores);
                        game_state.next_turn();
                        turn_idx += 1;
                        if !matches!(game_ended, game_state::CheckGameEnded::NotEnded) {
                            break;
                        }
                    }
                    completed_games.fetch_add(1, std::sync::atomic::Ordering::Relaxed);

                    let elapsed = t0.elapsed().as_secs();
                    let mut tick = mutexed_tick.lock().unwrap();
                    if tick.update(elapsed) {
                        eprintln!(
                            "After {elapsed} seconds, {} games, {} samples into {run_identifier}",
                            completed_games.load(std::sync::atomic::Ordering::Relaxed),
                            completed_samples.load(std::sync::atomic::Ordering::Relaxed),
                        );
                    }
                }

                let mut g = mutexed.lock().unwrap();
                merge_rack_map(&mut g.full_rack_map, &mut thread_map);

                if oppdenial_leave != 0.0 {
                    for (a, b) in g.oppdenial_leave_sum_marg.iter_mut().zip(oppdenial_leave_sum_marg.iter()) {
                        *a += *b;
                    }
                    g.oppdenial_leave_boards += oppdenial_leave_boards;
                }
            }));
        }
        for thread in threads {
            if let Err(e) = thread.join() {
                eprintln!("{e:?}");
            }
        }
    });

    let g = mutexed.lock().unwrap();
    let map = &g.full_rack_map;
    if min_samples != 0 && !g.undersampled_racks.is_empty() {
        eprintln!(
            "gilles: {} racks still below min_samples after remediation (blocked tail)",
            g.undersampled_racks.len(),
        );
    }
    let mut total_equity = 0.0;
    let mut row_count = 0u64;
    for v in map.values() {
        total_equity += v.equity;
        row_count += v.count;
    }
    eprintln!("{} records, {} unique racks", row_count, map.len());
    let mut kv = map.iter().collect::<Vec<_>>();
    kv.sort_unstable_by(|a, b| a.0.len().cmp(&b.0.len()).then_with(|| a.0.cmp(b.0)));
    let mut csv_out = csv::Writer::from_path(claim_output_path(&run_identifier)?)?;
    let mut cur_rack_ser = String::new();
    csv_out.serialize(("", total_equity, row_count))?;
    for (k, fv) in kv.iter() {
        cur_rack_ser.clear();
        for &tile in k.iter() {
            cur_rack_ser.push_str(game_config.alphabet().of_rack(tile).unwrap());
        }
        csv_out.serialize((&cur_rack_ser, fv.equity, fv.count))?;
    }
    eprintln!(
        "After {} seconds, {} games, {} samples into {run_identifier}",
        t0.elapsed().as_secs(),
        completed_games.load(std::sync::atomic::Ordering::Relaxed),
        completed_samples.load(std::sync::atomic::Ordering::Relaxed),
    );

    if oppdenial_leave != 0.0 && g.oppdenial_leave_boards > 0 {
        write_oppdenial_leave_marginal_sidecar(
            &g.oppdenial_leave_sum_marg,
            g.oppdenial_leave_boards,
        )?;
    }

    Ok(())
}

// handles the equivalent of '?', A-Z
#[inline(always)]
fn parse_rack(
    alphabet_reader: &alphabet::AlphabetReader,
    s: &str,
    v: &mut Vec<u8>,
) -> Result<(), Box<dyn std::error::Error>> {
    alphabet_reader.set_word(s, v)
}

#[derive(Clone)]
struct Cumulate {
    equity: f64,
    sumsq: f64,
    count: u64,
}

#[inline]
fn pool_one(map: &mut fash::MyHashMap<bites::Bites, Cumulate>, key: &[u8], equity: f64) {
    let sumsq = equity.powi(2);
    map.entry(key.into())
        .and_modify(|v| {
            v.equity += equity;
            v.sumsq += sumsq;
            v.count += 1;
        })
        .or_insert_with(|| Cumulate {
            equity,
            sumsq,
            count: 1,
        });
}

#[inline]
fn pool_rare_one(
    subrack_map: &mut fash::MyHashMap<bites::Bites, Cumulate>,
    key: &[u8],
    equity: f64,
    count: u64,
    sumsq: f64,
) {
    subrack_map
        .entry(key.into())
        .and_modify(|v| {
            v.equity += equity;
            v.sumsq += sumsq;
            v.count += count;
        })
        .or_insert(Cumulate {
            equity,
            sumsq,
            count,
        });
}

struct GillesMutexed {
    full_rack_map: fash::MyHashMap<bites::Bites, Cumulate>,
    undersampled_racks: Vec<bites::Bites>,
    best_remaining: u64,
    no_progress: u32,
    oppdenial_leave_sum_marg: Vec<f64>,
    oppdenial_leave_boards: u64,
}

fn merge_rack_map(
    dst: &mut fash::MyHashMap<bites::Bites, Cumulate>,
    src: &mut fash::MyHashMap<bites::Bites, Cumulate>,
) {
    for (k, v) in src.drain() {
        if v.count > 0 {
            dst.entry(k)
                .and_modify(|e| {
                    e.equity += v.equity;
                    e.sumsq += v.sumsq;
                    e.count += v.count;
                })
                .or_insert(v);
        }
    }
}

fn recompute_undersampled(
    g: &mut GillesMutexed,
    scratch_map: &mut fash::MyHashMap<bites::Bites, Cumulate>,
    base_freqs: &[u8],
    rack_tally: &mut Vec<u8>,
    exchange_buffer: &mut Vec<u8>,
    rack_size: u8,
    min_samples: u64,
) -> u64 {
    std::mem::swap(&mut g.full_rack_map, scratch_map);
    g.undersampled_racks.clear();
    rack_tally.clear();
    rack_tally.extend_from_slice(base_freqs);
    let mut remaining = 0u64;
    {
        let map = &*scratch_map;
        let undersampled = &mut g.undersampled_racks;
        generate_exchanges(&mut ExchangeEnv {
            found_exchange_move: |rack_bytes: &[u8]| {
                let count = map.get(rack_bytes).map_or(0, |v| v.count);
                if count < min_samples {
                    undersampled.push(rack_bytes.into());
                    remaining += min_samples - count;
                }
            },
            rack_tally: &mut rack_tally[..],
            min_len: rack_size,
            max_len: rack_size,
            exchange_buffer,
        });
    }
    std::mem::swap(&mut g.full_rack_map, scratch_map);
    remaining
}

struct RecomputeScratch<'a> {
    rack_tally: &'a mut [u8],
    full_rack_tally: &'a mut [u8],
    subrack_tally: &'a mut [u8],
    alphabet_freqs: &'a mut [u8],
    exchange_buffer: &'a mut Vec<u8>,
}

struct RecomputeParams {
    leave_size: u8,
    full_rack_forcing: bool,
    min_samples: u64,
}

fn recompute_undersampled_subracks(
    full_rack_map: &fash::MyHashMap<bites::Bites, Cumulate>,
    rare_subrack_map: &fash::MyHashMap<bites::Bites, Cumulate>,
    undersampled: &mut Vec<bites::Bites>,
    subrack_count: &mut fash::MyHashMap<bites::Bites, u64>,
    word_prob: Option<&mut prob::WordProbability>,
    scratch: RecomputeScratch<'_>,
    params: RecomputeParams,
) -> u64 {
    let RecomputeScratch {
        rack_tally,
        full_rack_tally,
        subrack_tally,
        alphabet_freqs,
        exchange_buffer,
    } = scratch;
    let RecomputeParams {
        leave_size,
        full_rack_forcing,
        min_samples,
    } = params;

    let Some(word_prob) = word_prob else {
        undersampled.clear();
        return 0;
    };

    if min_samples == 0 {
        undersampled.clear();
        return 0;
    }
    if full_rack_forcing {
        let rack_size = leave_size + 1;
        full_rack_tally.copy_from_slice(alphabet_freqs);
        let frozen_freq: &[u8] = full_rack_tally;
        undersampled.clear();
        let mut remaining = 0u64;
        generate_exchanges(&mut ExchangeEnv {
            found_exchange_move: |rack_bytes: &[u8]| {
                let mut i = 0;
                while i < rack_bytes.len() {
                    let t = rack_bytes[i] as usize;
                    let mut run = 1u8;
                    while i + (run as usize) < rack_bytes.len()
                        && rack_bytes[i + (run as usize)] as usize == t
                    {
                        run += 1;
                    }
                    if run > frozen_freq[t] {
                        return;
                    }
                    i += run as usize;
                }
                let count = full_rack_map.get(rack_bytes).map_or(0, |c| c.count);
                if count < min_samples {
                    undersampled.push(rack_bytes.into());
                    remaining += min_samples - count;
                }
            },
            rack_tally: alphabet_freqs,
            min_len: rack_size,
            max_len: rack_size,
            exchange_buffer,
        });
        return remaining;
    }

    subrack_count.clear();
    for (k, fv) in full_rack_map.iter() {
        if fv.count == 0 {
            continue;
        }
        rack_tally.iter_mut().for_each(|m| *m = 0);
        k.iter().for_each(|&tile| rack_tally[tile as usize] += 1);
        full_rack_tally.copy_from_slice(rack_tally);
        let count = fv.count;
        let frozen_full = &*full_rack_tally;
        generate_exchanges(&mut ExchangeEnv {
            found_exchange_move: |subrack_bytes: &[u8]| {
                subrack_tally.iter_mut().for_each(|m| *m = 0);
                subrack_bytes
                    .iter()
                    .for_each(|&tile| subrack_tally[tile as usize] += 1);
                let w = word_prob.completion_draw_ways(frozen_full, subrack_tally, word_prob.bag());
                *subrack_count.entry(subrack_bytes.into()).or_insert(0) += count * w;
            },
            rack_tally,
            min_len: 1,
            max_len: leave_size,
            exchange_buffer,
        });
    }

    for (k, fv) in rare_subrack_map.iter() {
        if fv.count > 0 {
            *subrack_count.entry(k[..].into()).or_insert(0) += fv.count;
        }
    }

    undersampled.clear();
    let mut remaining = 0u64;
    {
        let subrack_count = &*subrack_count;
        let undersampled = &mut *undersampled;
        generate_exchanges(&mut ExchangeEnv {
            found_exchange_move: |subrack_bytes: &[u8]| {
                let count = subrack_count.get(subrack_bytes).copied().unwrap_or(0);
                if count < min_samples {
                    undersampled.push(subrack_bytes.into());
                    remaining += min_samples - count;
                }
            },
            rack_tally: alphabet_freqs,
            min_len: 1,
            max_len: leave_size,
            exchange_buffer,
        });
    }
    remaining
}

struct SampleScratch<'a> {
    rack_tally: &'a mut [u8],
    exchange_buffer: &'a mut Vec<u8>,
}

struct SampleBudget<'a> {
    countdown: &'a std::sync::atomic::AtomicI64,
    completed_samples: &'a std::sync::atomic::AtomicU64,
    movegens_done: &'a mut u32,
    undersampled_done: &'a mut u32,
    samples_per_snapshot: u32,
    target: u32,
    knob: KnobFold<'a>,
}

fn sample_undersampled<N: kwg::Node, L: kwg::Node>(
    rack_size: u8,
    move_generator: &mut movegen::KurniaMoveGenerator,
    board_snapshot: &movegen::BoardSnapshot<'_, N, L>,
    thread_map: &mut fash::MyHashMap<bites::Bites, Cumulate>,
    local_undersampled: &mut fash::MyHashSet<bites::Bites>,
    scratch: SampleScratch<'_>,
    budget: SampleBudget<'_>,
) {
    let SampleScratch {
        rack_tally,
        exchange_buffer,
    } = scratch;
    let SampleBudget {
        countdown,
        completed_samples,
        movegens_done,
        undersampled_done,
        samples_per_snapshot,
        target,
        knob,
    } = budget;
    generate_exchanges(&mut ExchangeEnv {
        found_exchange_move: |rack_bytes: &[u8]| {
            if *movegens_done >= samples_per_snapshot || *undersampled_done >= target {
                return;
            }
            if !local_undersampled.contains(rack_bytes) {
                return;
            }
            move_generator.gen_moves_unfiltered(&movegen::GenMovesParams {
                board_snapshot,
                rack: rack_bytes,
                max_gen: 1,
                num_exchanges_by_this_player: 0,
                pass_policy: movegen::PassPolicy::OnlyWhenForced,
                dynamic_leaves: None,
            });
            let equity = knob.apply(move_generator.plays[0].equity, rack_bytes);
            pool_one(thread_map, rack_bytes, equity);
            completed_samples.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            *movegens_done += 1;
            local_undersampled.remove(rack_bytes);
            *undersampled_done += 1;
            countdown.fetch_sub(1, std::sync::atomic::Ordering::Relaxed);
        },
        rack_tally,
        min_len: rack_size,
        max_len: rack_size,
        exchange_buffer,
    });
}

fn n_choose_k(n: usize, k: usize) -> usize {
    if k > n {
        return 0;
    }
    let k = k.min(n - k);
    let mut result = 1usize;
    for i in 0..k {
        result = result * (n - i) / (i + 1);
    }
    result
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
            if i16::from_str(&record[10])? > 0 {
                let equity = f32::from_str(&record[9])? as f64;
                //let score = i16::from_str(&record[5])? as i64;
                parse_rack(&rack_reader, &record[3], &mut rack_bytes)?;
                rack_bytes.sort_unstable();
                row_count += 1;
                pool_one(&mut full_rack_map, &rack_bytes[..], equity);
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

#[inline(always)]
fn generate_exchanges<FoundExchangeMove: FnMut(&[u8])>(
    env: &mut ExchangeEnv<'_, FoundExchangeMove>,
) {
    fn generate_exchanges_inner<FoundExchangeMove: FnMut(&[u8])>(
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
                    generate_exchanges_inner(env, i as u8);
                    env.exchange_buffer.pop();
                    env.rack_tally[i] += 1;
                }
            }
        }
    }

    env.exchange_buffer.clear();
    generate_exchanges_inner(env, 0);
}

// generates neighbors of same length, same number of blanks,
// and max of one insertion and one deletion.
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

fn resummarize_summaries<const SORT_MODE: char, Readable: std::io::Read, W: std::io::Write>(
    game_config: game_config::GameConfig,
    mut csv_in: csv::Reader<Readable>,
    mut csv_out: csv::Writer<W>,
) -> error::Returns<()> {
    let mut stdout_or_stderr = boxed_stdout_or_stderr();
    let mut rack_bytes = Vec::new();
    let rack_reader = alphabet::AlphabetReader::new_for_racks(game_config.alphabet());
    let mut full_rack_map = fash::MyHashMap::<bites::Bites, Cumulate>::default();
    for result in csv_in.records() {
        let record = result?;
        parse_rack(&rack_reader, &record[0], &mut rack_bytes)?;
        let thing = Cumulate {
            equity: f64::from_str(&record[1])?,
            count: u64::from_str(&record[2])?,
            sumsq: f64::NAN,
        };
        full_rack_map
            .entry(rack_bytes[..].into())
            .and_modify(|e| {
                e.equity += thing.equity;
                e.count += thing.count;
                e.sumsq += thing.sumsq;
            })
            .or_insert(thing);
    }
    drop(csv_in);

    full_rack_map.remove([][..].into());

    let mut total_equity = 0.0;
    let mut row_count = 0;
    for x in full_rack_map.values() {
        total_equity += x.equity;
        row_count += x.count;
    }

    writeln!(
        stdout_or_stderr,
        "{} records, {} unique racks",
        row_count,
        full_rack_map.len()
    )?;

    let mut kv = full_rack_map.into_iter().collect::<Vec<_>>();
    match SORT_MODE {
        'a' => kv.sort_unstable_by(|a, b| a.0.len().cmp(&b.0.len()).then_with(|| a.0.cmp(&b.0))),
        'p' => kv.sort_unstable_by(|a, b| {
            a.0.len().cmp(&b.0.len()).then_with(|| {
                b.1.equity
                    .total_cmp(&a.1.equity)
                    .then_with(|| a.0.cmp(&b.0))
            })
        }),
        'P' => kv.sort_unstable_by(|a, b| {
            b.1.equity
                .total_cmp(&a.1.equity)
                .then_with(|| a.0.len().cmp(&b.0.len()).then_with(|| a.0.cmp(&b.0)))
        }),
        _ => unimplemented!(),
    }

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

const fn census_mix64(mut z: u64) -> u64 {
    z = (z ^ (z >> 30)).wrapping_mul(0xbf58476d1ce4e5b9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94d049bb133111eb);
    z ^ (z >> 31)
}

struct SpellTables<'a, N: kwg::Node, L: kwg::Node> {
    game_config: &'a game_config::GameConfig,
    kwg: &'a kwg::Kwg<N>,
    klv: &'a klv::Klv<L>,
    lat: &'a census::MultisetLattice,
}

struct SpellPool<'a> {
    unseen_tally: &'a [u8],
    num_blanks_eff: usize,
    rack_size: usize,
    blank_cap: usize,
}

fn oppdenial_leave_marginal_path() -> String {
    std::env::var("WOLGES_OPPDENIAL_LEAVE_MARGINAL")
        .unwrap_or_else(|_| "oppdenial-leave-marginal.csv".to_string())
}

fn write_oppdenial_leave_marginal_sidecar(sum_marg: &[f64], boards: u64) -> error::Returns<()> {
    let path = oppdenial_leave_marginal_path();
    let mut w = csv::Writer::from_path(&path)?;
    w.serialize(("tile_index", "avg_marginal"))?;
    let boards = boards as f64;
    for (t, &s) in sum_marg.iter().enumerate() {
        w.serialize((t, s / boards))?;
    }
    w.flush()?;
    eprintln!(
        "wrote {} board-averaged oppdenial_leave marginals to {path}",
        sum_marg.len()
    );
    Ok(())
}

fn load_oppdenial_leave_marginal_sidecar(
    path: &str,
    num_letters: usize,
) -> error::Returns<Vec<f64>> {
    let mut avg = vec![0f64; num_letters];
    let mut rd = csv::ReaderBuilder::new()
        .has_headers(true)
        .from_path(path)?;
    for result in rd.records() {
        let record = result?;
        let t = usize::from_str(&record[0])?;
        if t < num_letters {
            avg[t] = f64::from_str(&record[1])?;
        }
    }
    Ok(avg)
}

fn oppdenial_rack_fold(oppdenial_rack: f64, marginal: &[f64], rack_bytes: &[u8]) -> f64 {
    if marginal.is_empty() {
        return 0.0;
    }
    let mut d = 0.0f64;
    for &t in rack_bytes {
        d += marginal[t as usize];
    }
    oppdenial_rack * d / equity::SCALE as f64
}

fn oppdenial_exact_fold(oppdenial_exact: f64, oppdenial_exact_term: &[f64], rank: usize) -> f64 {
    if rank >= oppdenial_exact_term.len() {
        return 0.0;
    }
    -oppdenial_exact * oppdenial_exact_term[rank] / equity::SCALE as f64
}

#[derive(Clone, Copy)]
struct KnobFold<'a> {
    winpct_board: &'a Option<WinpctBoard<'a>>,
    oppdenial_rack: f64,
    opp_marginal: &'a [f64],
    oppdenial_exact: f64,
    oppdenial_exact_term: &'a [f64],
    oppdenial_exact_lat: Option<&'a census::MultisetLattice>,
}

impl KnobFold<'_> {
    fn apply(&self, base: equity::Equity, rack: &[u8]) -> f64 {
        let rank = match self.oppdenial_exact_lat {
            Some(lat) => lat.rank_bytes(rack) as usize,
            None => usize::MAX,
        };
        winpct_apply(self.winpct_board, base)
            + oppdenial_rack_fold(self.oppdenial_rack, self.opp_marginal, rack)
            + oppdenial_exact_fold(self.oppdenial_exact, self.oppdenial_exact_term, rank)
    }
}

fn winpct_remap(
    table: &win_pct::WinPctTable,
    best: &mut [i32],
    full_rack_start: usize,
    bag: usize,
    my: usize,
    opp: usize,
    blend: f64,
) {
    let Some(inv_slope_mp) = winpct_inv_slope(table, bag, my, opp) else {
        return;
    };
    for e in best[full_rack_start..].iter_mut() {
        *e = winpct_g(table, *e, bag, my, opp, inv_slope_mp, blend);
    }
}

fn winpct_inv_slope(
    table: &win_pct::WinPctTable,
    bag: usize,
    my: usize,
    opp: usize,
) -> Option<f64> {
    let dd = 25i32; // slope measurement half-width (points)
    let slope =
        (table.get(dd, bag, my, opp) - table.get(-dd, bag, my, opp)) as f64 / (2.0 * dd as f64);
    if slope <= 1e-6 {
        None
    } else {
        Some(equity::SCALE as f64 / slope)
    }
}

fn winpct_g(
    table: &win_pct::WinPctTable,
    e_mp: i32,
    bag: usize,
    my: usize,
    opp: usize,
    inv_slope_mp: f64,
    blend: f64,
) -> i32 {
    let e_pts = equity::descale_score(e_mp);
    let wprob = table.get(e_pts, bag, my, opp) as f64;
    let g_mp = (wprob - 0.5) * inv_slope_mp;
    ((1.0 - blend) * e_mp as f64 + blend * g_mp).round() as i32
}

#[derive(Clone, Copy)]
struct WinpctBoard<'a> {
    table: &'a win_pct::WinPctTable,
    bag: usize,
    my: usize,
    opp: usize,
    inv_slope_mp: f64,
    blend: f64,
}

impl WinpctBoard<'_> {
    fn new(
        table: Option<&win_pct::WinPctTable>,
        unseen: usize,
        rack_size: usize,
        blend: f64,
    ) -> Option<WinpctBoard<'_>> {
        WinpctBoard::from_bag(
            table,
            unseen.saturating_sub(2 * rack_size),
            rack_size,
            blend,
        )
    }

    fn from_bag(
        table: Option<&win_pct::WinPctTable>,
        bag: usize,
        rack_size: usize,
        blend: f64,
    ) -> Option<WinpctBoard<'_>> {
        let table = table?;
        let inv_slope_mp = winpct_inv_slope(table, bag, rack_size, rack_size)?;
        Some(WinpctBoard {
            table,
            bag,
            my: rack_size,
            opp: rack_size,
            inv_slope_mp,
            blend,
        })
    }
}

fn winpct_apply(wpb: &Option<WinpctBoard>, e: equity::Equity) -> f64 {
    match wpb {
        Some(w) => {
            winpct_g(
                w.table,
                e.raw(),
                w.bag,
                w.my,
                w.opp,
                w.inv_slope_mp,
                w.blend,
            ) as f64
                / equity::SCALE as f64
        }
        None => e.as_f64(),
    }
}

fn build_sheet_spell_once<N: kwg::Node, L: kwg::Node>(
    move_generator: &mut movegen::KurniaMoveGenerator,
    board_tiles: &[u8],
    tables: SpellTables<'_, N, L>,
    pool: SpellPool<'_>,
    movegen_rack: &mut Vec<u8>,
    blank_deltas: &mut Vec<(u8, i32)>,
    sheet: &mut [i32],
) -> u64 {
    let SpellTables {
        game_config,
        kwg,
        klv,
        lat,
    } = tables;
    let SpellPool {
        unseen_tally,
        num_blanks_eff,
        rack_size,
        blank_cap,
    } = pool;
    movegen_rack.clear();
    for (t, &c) in unseen_tally.iter().enumerate() {
        let cap = if t == 0 { blank_cap } else { rack_size };
        for _ in 0..(c as usize).min(cap) {
            movegen_rack.push(t as u8);
        }
    }
    let mut n_cand = 0u64;
    let board_snapshot = &movegen::BoardSnapshot {
        board_tiles,
        game_config,
        kwg,
        klv,
    };
    let params = movegen::GenMovesParams {
        board_snapshot,
        rack: &movegen_rack[..],
        max_gen: 1,
        num_exchanges_by_this_player: i16::MAX,
        pass_policy: movegen::PassPolicy::OnlyWhenForced,
        dynamic_leaves: None,
    };
    move_generator.set_spell_once(true);
    move_generator.gen_moves_filtered(
        &params,
        |down, lane, idx, word: &[u8], _score: i32| {
            n_cand += 1;
            let real_score = play_scorer::score_and_blank_deltas(
                board_snapshot,
                down,
                lane,
                idx,
                word,
                blank_deltas,
            );
            census::record_blank_variants(
                lat,
                sheet,
                real_score,
                blank_deltas,
                unseen_tally,
                num_blanks_eff,
            );
            false // never keep the move
        },
        |leave_value| leave_value,
        |_equity, _play| false,
    );
    move_generator.set_spell_once(false);
    n_cand
}

#[derive(Clone, Copy)]
enum Scatter {
    Off,
    On,
    Auto,
}

fn wolges_census_scatter() -> error::Returns<Scatter> {
    match std::env::var("WOLGES_CENSUS_SCATTER").ok().as_deref() {
        None | Some("auto") => Ok(Scatter::Auto),
        Some("off") => Ok(Scatter::Off),
        Some("on") => Ok(Scatter::On),
        Some(other) => {
            Err(format!("WOLGES_CENSUS_SCATTER must be off, on, or auto, got {other:?}").into())
        }
    }
}

type SheetCacheSlot = std::sync::Mutex<Option<(Vec<i32>, Vec<u8>)>>;

fn census_sheet_reuse_plan(board_counts: &[u64]) -> (Vec<usize>, usize) {
    let gens = board_counts.len();
    let mut live_after = vec![0usize; gens];
    for g in (0..gens.saturating_sub(1)).rev() {
        live_after[g] = (board_counts[g + 1] as usize).max(live_after[g + 1]);
    }
    let cache_len = (0..gens)
        .map(|g| (board_counts[g] as usize).min(live_after[g]))
        .max()
        .unwrap_or(0);
    (live_after, cache_len)
}

fn write_census_klv2(
    lat: &census::MultisetLattice,
    value_mp: &dyn Fn(usize) -> f64,
    baseline_mp: f64,
    is_valued: &dyn Fn(usize) -> bool,
    full: bool,
    path: &str,
) -> error::Returns<usize> {
    let n = lat.num_letters();
    let rack_size = lat.rack_size();

    let max_keep = if full {
        rack_size
    } else {
        rack_size.saturating_sub(1)
    };
    let mut leaves_map = fash::MyHashMap::<bites::Bites, f32>::default();
    let mut tally = vec![0u8; n];
    let mut word_buf = Vec::<u8>::new();
    for idx in 0..lat.len() {
        if !is_valued(idx) {
            continue;
        }
        lat.unrank_into(idx, &mut tally);
        let size: usize = tally.iter().map(|&c| c as usize).sum();
        if size == 0 || size > max_keep {
            continue; // skip empty (baseline), over-size, and (non-full) full racks.
        }
        word_buf.clear();
        for (t, &c) in tally.iter().enumerate() {
            for _ in 0..c {
                word_buf.push(t as u8);
            }
        }

        let pts = (value_mp(idx) - baseline_mp) / equity::SCALE as f64;
        leaves_map.insert(word_buf[..].into(), pts as f32);
    }
    let mut sorted_words = leaves_map.keys().cloned().collect::<Box<_>>();
    sorted_words.sort_unstable();
    let leaves_kwg = build::build(
        build::BuildContent::DawgOnly,
        build::BuildLayout::Wolges,
        &sorted_words,
    )?;
    let leave_values = sorted_words
        .iter()
        .map(|s| leaves_map[s])
        .collect::<Box<_>>();
    let mut bin = vec![0u8; 2 * 4 + leaves_kwg.len() + leave_values.len() * 4];
    let mut w = 0;
    bin[w..w + 4].copy_from_slice(&((leaves_kwg.len() / 4) as u32).to_le_bytes());
    w += 4;
    bin[w..w + leaves_kwg.len()].copy_from_slice(&leaves_kwg);
    w += leaves_kwg.len();
    bin[w..w + 4].copy_from_slice(&(leave_values.len() as u32).to_le_bytes());
    w += 4;
    for v in &leave_values[..] {
        bin[w..w + 4].copy_from_slice(&v.to_le_bytes());
        w += 4;
    }
    assert_eq!(w, bin.len());
    std::fs::write(path, &bin)?;
    Ok(leave_values.len())
}

fn generate_census_leaves<N: kwg::Node + Sync + Send, L: kwg::Node + Sync + Send>(
    game_config: game_config::GameConfig,
    kwg: kwg::Kwg<N>,
    arc_klv0: std::sync::Arc<klv::Klv<L>>,
    arc_klv1: std::sync::Arc<klv::Klv<L>>,
    board_counts: Vec<u64>,
    seed: Option<u64>,
) -> error::Returns<()> {
    let t0 = std::time::Instant::now();
    let alphabet = game_config.alphabet();
    let num_letters = alphabet.len() as usize;
    let rack_size = game_config.rack_size() as usize;
    let num_tiles: usize = (0..alphabet.len()).map(|t| alphabet.freq(t) as usize).sum();
    let racks_tiles = game_config.num_players() as usize * rack_size;

    let pool_max = env_usize("WOLGES_POOL_MAX", num_tiles.saturating_sub(racks_tiles));

    let min_pool = racks_tiles + 1;
    let pool_min = {
        let req = env_usize("WOLGES_POOL_MIN", min_pool);
        if req < min_pool {
            eprintln!(
                "census: raising pool_min {req} -> {min_pool} (a smaller unseen pool \
                 implies an empty bag = endgame, where the klv leave is unused)"
            );
            min_pool
        } else {
            req
        }
    };
    let blank_cap = env_usize("WOLGES_CENSUS_BLANK_CAP", rack_size);
    let low_tiles = num_tiles.saturating_sub(pool_max);
    let high_tiles = num_tiles.saturating_sub(pool_min);
    let verify = env_flag("WOLGES_CENSUS_VERIFY", false);

    let full_rack = match wolges_apportion()? {
        Apportion::FullRack => true,
        Apportion::Entering => false,
    };

    let winpct_table: Option<win_pct::WinPctTable> = if env_flag("WOLGES_WINPCT", false) {
        let path =
            std::env::var("WOLGES_WINPCT_TABLE").unwrap_or_else(|_| "win_pct.csv".to_string());
        let t = win_pct::WinPctTable::from_csv(&std::fs::read_to_string(&path)?)?;
        eprintln!("census: win%-objective from {path}");
        Some(t)
    } else {
        None
    };

    let winpct_blend = env_parse::<f64>("WOLGES_WINPCT_BLEND", 1.0);

    let entering_push = env_flag("WOLGES_CENSUS_ENTERING_PUSH", false);

    let per_game = env_flag("WOLGES_CENSUS_PER_GAME", false);

    let gens = board_counts.len();

    let max_boards = board_counts.iter().copied().max().unwrap_or(1).max(1);
    let multigen = gens > 1;

    let batch_size = (env_usize("WOLGES_CENSUS_BATCH", board_counts[0] as usize) as u64).max(1);
    let alpha = env_parse::<f64>("WOLGES_CENSUS_ALPHA", 0.5);
    let sgd = !multigen && batch_size < board_counts[0];

    let rack_summary = full_rack && !sgd && env_flag("WOLGES_CENSUS_RACK_SUMMARY", false);
    let impossible_ok = env_flag("WOLGES_IMPOSSIBLE_OK", true);

    let global_apportion = rack_summary
        || (full_rack && !sgd && !multigen && env_flag("WOLGES_CENSUS_GLOBAL_APPORTION", false));

    let ga_drawable = (rack_summary && !impossible_ok)
        || (global_apportion && env_flag("WOLGES_CENSUS_GLOBAL_APPORTION_DRAWABLE", false));

    let global_weights =
        full_rack && !global_apportion && env_flag("WOLGES_CENSUS_GLOBAL_WEIGHTS", false);

    let opening_samples = rack_summary && env_flag("WOLGES_OPENING_SAMPLES", false);

    let opening_weight = env_usize("WOLGES_OPENING_WEIGHT", 1).max(1) as u64;

    let ci_report_level = match wolges_census_ci_report()? {
        CiReport::Off => 0usize,
        CiReport::Rack => 1,
        CiReport::Leave => 2,
    };
    let ci_conf = env_parse::<f64>("WOLGES_CENSUS_CI_CONF", 0.999);
    let ci_conf = if ci_conf > 0.0 && ci_conf < 1.0 {
        ci_conf
    } else {
        0.999
    };

    let ci_target_mp = env_usize("WOLGES_CENSUS_CI_TARGET", 500) as f64;

    let ci_stop_frac = env_parse::<f64>("WOLGES_CENSUS_CI_STOP_FRAC", 0.0);
    let ci_stop_frac = if ci_stop_frac > 0.0 && ci_stop_frac <= 1.0 {
        ci_stop_frac
    } else {
        0.0
    };
    let ci_stop_every = env_usize("WOLGES_CENSUS_CI_STOP_EVERY", 64).max(1) as u64;

    let ci_stop = ci_stop_frac > 0.0 && full_rack && rack_summary && !sgd && !multigen;

    let ci_report = full_rack && (ci_report_level != 0 || ci_stop);

    let sheet_reuse = multigen && !per_game && env_flag("WOLGES_CENSUS_SHEET_REUSE", true);

    let (live_after, sheet_cache_len) = census_sheet_reuse_plan(&board_counts);

    let sheet_cache_len = if sheet_reuse { sheet_cache_len } else { 0 };

    let persist_gens = multigen && env_flag("WOLGES_CENSUS_PERSIST_GENS", true);
    let resume = multigen && env_flag("WOLGES_CENSUS_RESUME", false);

    let num_buckets = env_usize("WOLGES_CENSUS_BUCKETS", 0);

    let lat = census::MultisetLattice::new(num_letters, rack_size);
    let empty_rank = lat.rank(&vec![0u8; num_letters]) as usize;
    let full_rack_start = lat.full_rack_start();
    eprintln!(
        "census: lattice {} leaves (letters {num_letters}, rack_size {rack_size}), \
         window [{low_tiles},{high_tiles}] of {num_tiles} tiles",
        lat.len(),
    );

    let add_table = if full_rack {
        let t = std::time::Instant::now();
        let at = census::AddTable::new_with_threads(&lat, wolges_threads());
        eprintln!(
            "census: add-table {} rows x {num_letters} letters built in {:?}",
            lat.full_rack_start(),
            t.elapsed(),
        );
        Some(at)
    } else {
        None
    };

    let zeta_pool_min = env_usize("WOLGES_CENSUS_ZETA_POOL", 36);

    let scatter = match wolges_census_scatter()? {
        Scatter::Off => false,
        Scatter::On => true,
        Scatter::Auto => lat.len() <= 12_000_000,
    };

    let oppdenial_leave = env_parse::<f64>("WOLGES_OPPDENIAL_LEAVE", 0.0);

    let oppdenial_rack = env_parse::<f64>("WOLGES_OPPDENIAL_RACK", 0.0);

    let oppdenial_exact = env_parse::<f64>("WOLGES_OPPDENIAL_EXACT", 0.0);
    let oppdenial_exact_pool_max = env_usize("WOLGES_OPPDENIAL_EXACT_POOL_MAX", 32);

    let oppdenial_exact_me2 = env_parse::<f64>("WOLGES_OPPDENIAL_EXACT_ME2", 1.0);

    let base_freqs: Vec<u8> = (0..alphabet.len()).map(|t| alphabet.freq(t)).collect();

    let withhold_budget = env_usize("WOLGES_CENSUS_WITHHOLD", 0);
    let withhold_tally: Vec<u8> = if withhold_budget > 0 && !per_game {
        let mut tiles: Vec<usize> = (0..num_letters).filter(|&t| base_freqs[t] > 0).collect();
        tiles.sort_by_key(|&t| base_freqs[t]);
        let mut wt = vec![0u8; num_letters];
        for &t in tiles.iter().take(withhold_budget) {
            wt[t] = 1;
        }
        eprintln!(
            "census: withholding {} rarest tiles from the bag for rare-rack coverage",
            wt.iter().filter(|&&c| c > 0).count(),
        );
        wt
    } else {
        Vec::new()
    };

    let withhold_frac = env_parse::<f64>("WOLGES_CENSUS_WITHHOLD_FRAC", 1.0);
    let withhold_frac = if withhold_frac > 0.0 {
        withhold_frac
    } else {
        1.0
    };
    let withhold_period = if withhold_frac >= 1.0 {
        1
    } else {
        (1.0 / withhold_frac).round().max(1.0) as usize
    };
    if !withhold_tally.is_empty() && withhold_period > 1 {
        eprintln!(
            "census: withhold fraction {:.3} -> 1 in {} boards (phase-balanced) is a withhold board",
            withhold_frac, withhold_period,
        );
    }
    let seed = seed.unwrap_or_else(rand::random);

    let mut leave_cur = vec![0i32; lat.len()];
    let mut tally_buf = vec![0u8; num_letters];
    for (idx, slot) in leave_cur.iter_mut().enumerate() {
        lat.unrank_into(idx, &mut tally_buf);
        *slot = arc_klv0.leave_value_from_tally(&tally_buf);
    }

    let mut start_gen = 0usize;
    let census_run_epoch;
    let mut resumed: Option<(String, usize, std::path::PathBuf)> = None;
    if resume && let Ok(rd) = std::fs::read_dir(".") {
        for e in rd.flatten() {
            let name = e.file_name();
            let name = name.to_string_lossy();
            if let Some((rid, gg)) = name
                .strip_prefix("census-gen-")
                .and_then(|r| r.strip_suffix(".klv2"))
                .and_then(|r| r.split_once('-'))
                && u64::from_str_radix(rid, 16).is_ok()
                && let Ok(gg) = gg.parse::<usize>()
                && resumed
                    .as_ref()
                    .is_none_or(|(br, bg, _)| (gg, rid) > (*bg, br.as_str()))
            {
                resumed = Some((rid.to_owned(), gg, e.path()));
            }
        }
    }
    if let Some((rid, num, path)) = resumed {
        let bytes = std::fs::read(&path)?;
        let resume_klv = klv::Klv::<L>::from_bytes_alloc(&bytes);
        for (idx, slot) in leave_cur.iter_mut().enumerate() {
            lat.unrank_into(idx, &mut tally_buf);
            *slot = resume_klv.leave_value_from_tally(&tally_buf);
        }
        start_gen = num;
        census_run_epoch = rid;
        eprintln!(
            "census: resuming from {} (gen {num} done) -> starting gen {}",
            path.display(),
            num + 1
        );
    } else {
        if resume {
            eprintln!("census: resume requested but no census-gen-*.klv2 found; fresh start");
        }
        census_run_epoch = run_stamp();
    }

    if start_gen >= gens {
        return Err(format!(
            "census resume: {start_gen} generation(s) already completed but the \
             spec has only {gens}; extend the board-count spec or remove \
             census-gen-*.klv2"
        )
        .into());
    }

    let leave_lock = std::sync::RwLock::new(leave_cur);

    let num_threads = wolges_threads().max(1).min(max_boards as usize);

    let dynamic_leaves_on = std::env::var("WOLGES_DYNAMIC_LEAVES")
        .ok()
        .and_then(|s| s.parse::<u64>().ok())
        .unwrap_or(0)
        != 0;
    let dynamic_min_keep = std::env::var("WOLGES_DYNAMIC_LEAVES_MIN_KEEP")
        .ok()
        .and_then(|s| s.parse::<usize>().ok())
        .unwrap_or(2);
    let dyn_ctx: Option<(census::AddTable, Vec<i32>)> = if dynamic_leaves_on {
        let add = census::AddTable::new_with_threads(&lat, num_threads);
        let mut full_v = vec![0i32; lat.len()];
        census::fill_lattice_leaves(&lat, &mut full_v, |tally| {
            arc_klv0.leave_value_from_tally(tally)
        });
        Some((add, full_v))
    } else {
        None
    };
    let dyn_ref = dyn_ctx.as_ref().map(|(add, full_v)| klv::DynamicLeavesRef {
        lat: &lat,
        add,
        full_v: full_v.as_slice(),
        min_keep: dynamic_min_keep,
    });
    eprintln!(
        "WOLGES_DYNAMIC_LEAVES={} WOLGES_DYNAMIC_LEAVES_MIN_KEEP={dynamic_min_keep} ({})",
        dynamic_leaves_on as u8,
        if dynamic_leaves_on {
            "dynamic leaves on for the klv0 side; needs a --full (len 1-7) klv0"
        } else {
            "off, static leaves"
        },
    );

    let lat_len = lat.len();

    let globally_possible_count = {
        let mut tally = vec![0u8; num_letters];
        (0..lat_len)
            .filter(|&idx| {
                lat.unrank_into(idx, &mut tally);
                (0..num_letters).all(|t| tally[t] <= base_freqs[t])
            })
            .count()
    };
    let next_board = std::sync::atomic::AtomicU64::new(0);

    let stop_now = std::sync::atomic::AtomicBool::new(false);

    let ci_check_at = std::sync::atomic::AtomicU64::new(ci_stop_every);

    let shared = std::sync::Mutex::new((
        vec![0f64; lat_len],
        vec![0u64; lat_len],
        0u64,
        0u64,
        if sgd || multigen {
            vec![false; lat_len]
        } else {
            Vec::new()
        },
    ));

    let ci_sumsq = std::sync::Mutex::new(if ci_report {
        vec![0f64; lat_len]
    } else {
        Vec::new()
    });

    let ci_scratch: std::sync::Mutex<(Vec<f64>, Vec<f64>, Vec<f64>)> =
        std::sync::Mutex::new((Vec::new(), Vec::new(), Vec::new()));

    let barrier = std::sync::Barrier::new(num_threads);

    let pool_hist: Vec<std::sync::atomic::AtomicU64> = if per_game {
        (0..=num_tiles)
            .map(|_| std::sync::atomic::AtomicU64::new(0))
            .collect()
    } else {
        Vec::new()
    };

    let sheet_cache: Vec<SheetCacheSlot> = (0..sheet_cache_len)
        .map(|_| std::sync::Mutex::new(None))
        .collect();
    eprintln!("census: {num_threads} threads over {board_counts:?} boards/gen");

    std::thread::scope(|s| {
        for _ in 0..num_threads {
            s.spawn(|| {

                let mut game_state = game_state::GameState::new(&game_config);
                let mut move_generator = movegen::KurniaMoveGenerator::new(&game_config);
                let mut sheet = vec![census::UNPLAYABLE; lat_len];

                let mut blank_deltas = Vec::<(u8, i32)>::new();

                let mut best = if full_rack {
                    Vec::new()
                } else {
                    vec![census::UNPLAYABLE; lat_len]
                };
                let mut contrib = vec![census::UNPLAYABLE; lat_len];

                let mut num_board = if full_rack { vec![0f64; lat_len] } else { Vec::new() };
                let mut den_board = if full_rack { vec![0f64; lat_len] } else { Vec::new() };

                let mut maxsheet = if full_rack { vec![0i32; lat_len] } else { Vec::new() };

                let opp_term = oppdenial_leave != 0.0 || oppdenial_rack != 0.0;

                let mut oppdenial_leave_best = if full_rack
                    && (opp_term || oppdenial_exact != 0.0 || global_apportion || winpct_table.is_some())
                {
                    vec![census::UNPLAYABLE; lat_len]
                } else {
                    Vec::new()
                };
                let mut oppdenial_leave_marginal = if full_rack && opp_term {
                    vec![0f64; num_letters]
                } else {
                    Vec::new()
                };

                let mut oppdenial_exact_kept_idx = if full_rack && oppdenial_exact != 0.0 {
                    vec![0u32; lat_len]
                } else {
                    Vec::new()
                };
                let mut oppdenial_exact_kept_size = if full_rack && oppdenial_exact != 0.0 {
                    vec![0u8; lat_len]
                } else {
                    Vec::new()
                };
                let mut oppdenial_exact_term = if full_rack && oppdenial_exact != 0.0 {
                    vec![0f64; lat_len]
                } else {
                    Vec::new()
                };

                let mut num_e = if !full_rack && entering_push {
                    vec![0i128; lat_len]
                } else {
                    Vec::new()
                };
                let mut den_e = if !full_rack && entering_push {
                    vec![0i128; lat_len]
                } else {
                    Vec::new()
                };
                let mut tally_buf = vec![0u8; num_letters];
                let mut unseen_tally = vec![0u8; num_letters];
                let mut unseen_pool = Vec::<u8>::new();
                let mut movegen_rack = Vec::<u8>::new();
                let mut verify_rack = Vec::<u8>::new();
                let mut final_scores = vec![0; game_config.num_players() as usize];

                let mut open_buf = Vec::<(u32, i32)>::new();


                let mut value_board = |move_generator: &mut movegen::KurniaMoveGenerator,
                                       game_state: &game_state::GameState,
                                       rng: &mut rand::rngs::ChaCha20Rng,
                                       leave: &[i32],
                                       null_leave: bool,
                                       log_first: bool,
                                       do_verify: bool,
                                       cache_slot: Option<&SheetCacheSlot>,
                                       reuse: bool,
                                       cur_boards: u64| {

                    if reuse {
                        let g = cache_slot.unwrap().lock().unwrap();
                        let (cs, cu) = g.as_ref().expect("sheet-reuse: gen 0 must cache");
                        sheet.copy_from_slice(cs);
                        unseen_tally.copy_from_slice(cu);
                    } else {

                    unseen_tally.clone_from_slice(&base_freqs);
                    for &t in game_state.board_tiles.iter() {
                        if t != 0 {
                            let base = t & !((t as i8) >> 7) as u8;
                            unseen_tally[base as usize] =
                                unseen_tally[base as usize].saturating_sub(1);
                        }
                    }


                    sheet.iter_mut().for_each(|v| *v = 0);

                    let sheet_pool: &[u8] = if global_weights || (rack_summary && impossible_ok) {
                        &base_freqs
                    } else {
                        &unseen_tally
                    };
                    let num_blanks_eff = (sheet_pool[0] as usize).min(blank_cap);
                    let ts = std::time::Instant::now();

                    let n_cand = build_sheet_spell_once(
                        move_generator,
                        &game_state.board_tiles,
                        SpellTables {
                            game_config: &game_config,
                            kwg: &kwg,
                            klv: &arc_klv0,
                            lat: &lat,
                        },
                        SpellPool {
                            unseen_tally: sheet_pool,
                            num_blanks_eff,
                            rack_size,
                            blank_cap,
                        },
                        &mut movegen_rack,
                        &mut blank_deltas,
                        &mut sheet,
                    );
                    if log_first {
                        eprintln!(
                            "  step1 sheet: {} tiles in pool -> {} candidate plays (unstored) in {:?}",
                            movegen_rack.len(),
                            n_cand,
                            ts.elapsed(),
                        );
                    }

                    if let Some(slot) = cache_slot {
                        *slot.lock().unwrap() = Some((sheet.clone(), unseen_tally.clone()));
                    }
                    } // end of the !reuse step-1 build branch


                    let ts = std::time::Instant::now();
                    if !full_rack {
                        census::best_equity_table(&lat, &sheet, leave, &mut best);
                        if log_first {
                            eprintln!("  step2 best_equity_table: {:?}", ts.elapsed());
                        }
                    }


                    if do_verify {
                        unseen_pool.clear();
                        for (t, &c) in unseen_tally.iter().enumerate() {
                            for _ in 0..c {
                                unseen_pool.push(t as u8);
                            }
                        }
                        let mut ok = 0u32;
                        let mut bad = 0u32;
                        if unseen_pool.len() >= rack_size {
                            for _ in 0..32 {

                                for i in 0..rack_size {
                                    let j = rng.random_range(i..unseen_pool.len());
                                    unseen_pool.swap(i, j);
                                }
                                verify_rack.clear();
                                verify_rack.extend_from_slice(&unseen_pool[..rack_size]);
                                verify_rack.sort_unstable();
                                let rr = lat.rank_bytes(&verify_rack);
                                if rr == !0 {
                                    continue;
                                }
                                let board_snapshot = &movegen::BoardSnapshot {
                                    board_tiles: &game_state.board_tiles,
                                    game_config: &game_config,
                                    kwg: &kwg,
                                    klv: &arc_klv0,
                                };
                                move_generator.gen_moves_unfiltered(
                                    &movegen::GenMovesParams {
                                        board_snapshot,
                                        rack: &verify_rack,
                                        max_gen: 1,
                                        num_exchanges_by_this_player: 0,
                                        pass_policy: movegen::PassPolicy::OnlyWhenForced,
                                        dynamic_leaves: None,
                                    },
                                );
                                let engine_mp = (move_generator.plays[0].equity.as_f64()
                                    * equity::SCALE as f64)
                                    .round()
                                    as i32;
                                let census_mp = if full_rack {

                                    tally_buf.iter_mut().for_each(|x| *x = 0);
                                    for &t in &verify_rack {
                                        tally_buf[t as usize] += 1;
                                    }
                                    census::naive_best_equity(
                                        &lat, &sheet, leave, &tally_buf,
                                    )
                                    .0
                                } else {
                                    best[rr as usize]
                                };
                                if engine_mp == census_mp {
                                    ok += 1;
                                } else {
                                    bad += 1;
                                    if bad <= 5 {
                                        eprintln!(
                                            "  census VERIFY mismatch rack {:?}: engine {} census {}",
                                            verify_rack, engine_mp, census_mp,
                                        );
                                    }
                                }
                            }
                        }
                        eprintln!(
                            "census VERIFY: {ok} ok, {bad} mismatch (null-klv/engine invariant)"
                        );
                    }


                    let ts = std::time::Instant::now();
                    if full_rack && !global_apportion {
                        num_board.iter_mut().for_each(|x| *x = 0.0);
                        den_board.iter_mut().for_each(|x| *x = 0.0);

                        let weight_pool: &[u8] = if global_weights {
                            &base_freqs
                        } else {
                            &unseen_tally
                        };
                        let pool: usize = weight_pool.iter().map(|&c| c as usize).sum();
                        if let Some(wp_table) = winpct_table.as_ref() {

                            census::best_equity_table(&lat, &sheet, leave, &mut oppdenial_leave_best);
                            let u: usize = unseen_tally.iter().map(|&c| c as usize).sum();
                            let bag = u.saturating_sub(2 * rack_size);
                            winpct_remap(
                                wp_table,
                                &mut oppdenial_leave_best,
                                full_rack_start,
                                bag,
                                rack_size,
                                rack_size,
                                winpct_blend,
                            );
                            census::apportion_table(
                                &lat,
                                &oppdenial_leave_best,
                                weight_pool,
                                &mut num_board,
                                &mut den_board,
                            );
                        } else {

                        let oppdenial_exact_board = oppdenial_exact != 0.0 && pool <= oppdenial_exact_pool_max;
                        if opp_term || oppdenial_exact_board {
                            if oppdenial_exact_board {
                                census::best_equity_argmax_table(
                                    &lat,
                                    &sheet,
                                    leave,
                                    &mut oppdenial_leave_best,
                                    &mut oppdenial_exact_kept_idx,
                                    &mut oppdenial_exact_kept_size,
                                );
                            } else {
                                census::best_equity_table(&lat, &sheet, leave, &mut oppdenial_leave_best);
                            }
                        }
                        if opp_term {
                            census::opp_denial_marginals(
                                &lat,
                                add_table.as_ref().unwrap(),
                                &oppdenial_leave_best,
                                &unseen_tally,
                                &mut oppdenial_leave_marginal,
                            );
                        }
                        if oppdenial_exact_board {

                            oppdenial_exact_term.iter_mut().for_each(|x| *x = 0.0);
                            census::opp_me2_per_rack(
                                &lat,
                                add_table.as_ref().unwrap(),
                                &oppdenial_leave_best,
                                &census::KeptArgmax {
                                    idx: &oppdenial_exact_kept_idx,
                                    size: &oppdenial_exact_kept_size,
                                },
                                &unseen_tally,
                                oppdenial_exact_me2,
                                &mut oppdenial_exact_term,
                            );
                        } else if oppdenial_exact != 0.0 && log_first {
                            eprintln!(
                                "  oppdenial_exact: pool {pool} > {oppdenial_exact_pool_max}, skipping the term this board"
                            );
                        }
                        census::apportion_fused(
                            &lat,
                            add_table.as_ref().unwrap(),
                            &census::ApportionBoard {
                                sheet: &sheet,
                                leave,
                                unseen: weight_pool,
                            },
                            census::ApportionOut {
                                num: &mut num_board,
                                den: &mut den_board,
                            },
                            &mut maxsheet,
                            census::ApportionMode {
                                zeta: pool >= zeta_pool_min,
                                null_leave,
                                scatter,
                            },
                            &census::OppDenialParams {
                                oppdenial_rack,
                                marginal: if oppdenial_rack != 0.0 {
                                    &oppdenial_leave_marginal
                                } else {
                                    &[]
                                },
                                oppdenial_exact: if oppdenial_exact_board { oppdenial_exact } else { 0.0 },
                                oppdenial_exact_term: if oppdenial_exact_board {
                                    &oppdenial_exact_term
                                } else {
                                    &[]
                                },
                            },
                        );
                        }
                        for (idx, slot) in contrib.iter_mut().enumerate() {
                            *slot = if den_board[idx] > 0.0 {
                                let mut v = (num_board[idx] / den_board[idx]).round() as i32;
                                if oppdenial_leave != 0.0 {

                                    lat.unrank_into(idx, &mut tally_buf);
                                    let mut d = 0.0f64;
                                    for (t, &c) in tally_buf.iter().enumerate() {
                                        d += c as f64 * oppdenial_leave_marginal[t];
                                    }
                                    v += (oppdenial_leave * d).round() as i32;
                                }
                                v
                            } else {
                                census::UNPLAYABLE
                            };
                        }
                    } else if full_rack && global_apportion {

                        census::best_equity_table(&lat, &sheet, leave, &mut oppdenial_leave_best);
                        if let Some(wp_table) = winpct_table.as_ref() {

                            let u: usize = unseen_tally.iter().map(|&c| c as usize).sum();
                            let bag = u.saturating_sub(2 * rack_size);
                            winpct_remap(
                                wp_table,
                                &mut oppdenial_leave_best,
                                full_rack_start,
                                bag,
                                rack_size,
                                rack_size,
                                winpct_blend,
                            );
                        }
                        contrib.iter_mut().for_each(|x| *x = census::UNPLAYABLE);
                        if ga_drawable {

                            census::mark_drawable_best(
                                &lat,
                                add_table.as_ref().unwrap(),
                                &oppdenial_leave_best,
                                &unseen_tally,
                                &mut contrib,
                            );
                        } else {

                            contrib[full_rack_start..]
                                .iter_mut()
                                .zip(oppdenial_leave_best[full_rack_start..].iter())
                                .for_each(|(slot, &b)| *slot = b);
                        }
                    } else if entering_push {

                        num_e.iter_mut().for_each(|x| *x = 0);
                        den_e.iter_mut().for_each(|x| *x = 0);
                        census::entering_fused(&lat, &best, &unseen_tally, &mut num_e, &mut den_e);
                        for (idx, slot) in contrib.iter_mut().enumerate() {
                            *slot = if den_e[idx] != 0 {
                                (num_e[idx] / den_e[idx]) as i32
                            } else {
                                census::UNPLAYABLE
                            };
                        }
                    } else {
                        for (idx, slot) in contrib.iter_mut().enumerate() {
                            lat.unrank_into(idx, &mut tally_buf);
                            *slot = census::leave_value_by_draw(
                                &lat,
                                &best,
                                &unseen_tally,
                                &tally_buf,
                            );
                        }
                    }
                    if log_first {
                        eprintln!(
                            "  step3 {}: {:?}",
                            if full_rack { "full-rack" } else { "draw-average" },
                            ts.elapsed(),
                        );
                    }


                    let mut g = shared.lock().unwrap();
                    let (sum, cnt, completed, valued, _ever) = &mut *g;
                    let mut sq = if ci_report {
                        Some(ci_sumsq.lock().unwrap())
                    } else {
                        None
                    };
                    for idx in 0..lat_len {
                        let v = contrib[idx];
                        if v != census::UNPLAYABLE {
                            if cnt[idx] == 0 {
                                *valued += 1;
                            }
                            sum[idx] += v as f64;
                            cnt[idx] += 1;
                            if let Some(sq) = sq.as_mut() {
                                sq[idx] += (v as f64) * (v as f64);
                            }
                        }
                    }
                    *completed += 1;
                    eprintln!(
                        "census: board {}/{} done ({}s), {} of {} leaves valued so far",
                        *completed,
                        cur_boards,
                        t0.elapsed().as_secs(),
                        *valued,
                        globally_possible_count,
                    );
                };


                let mut batch_start = 0u64;
                let mut gen_idx = start_gen;

                let mut num_boards = board_counts[gen_idx];

                let mut prior_max_boards = 0usize;
                loop {

                    let batch_end = if sgd {
                        (batch_start + batch_size).min(num_boards)
                    } else {
                        num_boards
                    };

                    {
                        let leave = leave_lock.read().unwrap();

                        let null_leave = leave.iter().all(|&x| x == 0);
                        loop {
                            if ci_stop && stop_now.load(std::sync::atomic::Ordering::Relaxed) {
                                break;
                            }
                            let b = next_board.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                            if b >= batch_end {
                                break;
                            }
                    let mut rng = rand::rngs::ChaCha20Rng::seed_from_u64(census_mix64(
                        seed.wrapping_add(census_mix64(b)),
                    ));

                    if per_game {

                        use std::sync::atomic::Ordering::Relaxed;
                        let goal = 1 + (pool_min..=pool_max)
                            .map(|p| pool_hist[p].load(Relaxed))
                            .min()
                            .unwrap_or(0);
                        let deepest = (pool_min..=pool_max)
                            .find(|&p| pool_hist[p].load(Relaxed) < goal)
                            .unwrap_or(pool_min);
                        game_state.reset_and_draw_tiles_double_ended(&game_config, &mut rng);
                        let mut logged = false;
                        loop {
                            let fill =
                                game_state.board_tiles.iter().filter(|&&t| t != 0).count();
                            let pool = num_tiles - fill;
                            if pool < deepest {
                                break; // no under-goal bucket remains below (and pool

                            }
                            if pool <= pool_max && pool_hist[pool].load(Relaxed) < goal {
                                pool_hist[pool].fetch_add(1, Relaxed);

                                let lf = b == 0 && !logged;
                                logged |= lf;
                                value_board(
                                    &mut move_generator,
                                    &game_state,
                                    &mut rng,
                                    &leave,
                                    null_leave,
                                    lf,
                                    verify && lf,
                                    None, // per-game path never reuses (sheet_reuse gates on !per_game)
                                    false,
                                    num_boards,
                                );
                            }

                            game_state.players[game_state.turn as usize]
                                .rack
                                .sort_unstable();
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
                            move_generator.gen_moves_unfiltered(&movegen::GenMovesParams {
                                board_snapshot,
                                rack: &game_state.current_player().rack,
                                max_gen: 1,
                                num_exchanges_by_this_player: game_state
                                    .current_player()
                                    .num_exchanges,
                                pass_policy: movegen::PassPolicy::OnlyWhenForced,
                                dynamic_leaves: if game_state.turn == 0 { dyn_ref } else { None },
                            });
                            game_state
                                .play(&game_config, &mut rng, &move_generator.plays[0].play)
                                .unwrap();
                            let ended =
                                game_state.check_game_ended(&game_config, &mut final_scores);
                            game_state.next_turn();
                            if !matches!(ended, game_state::CheckGameEnded::NotEnded) {
                                break; // game ended; this game is done.
                            }
                        }
                    } else {

                        let reuse_board = sheet_reuse && (b as usize) < prior_max_boards;
                        if !reuse_board {

                        let target = if high_tiles <= low_tiles {
                            low_tiles
                        } else if num_buckets >= 2 {

                            let span = high_tiles - low_tiles;
                            let j = b as usize % num_buckets;
                            low_tiles + (j * span + (num_buckets - 1) / 2) / (num_buckets - 1)
                        } else {

                            low_tiles + (b as usize % (high_tiles - low_tiles + 1))
                        };

                        let phase_buckets = if high_tiles <= low_tiles {
                            1
                        } else if num_buckets >= 2 {
                            num_buckets
                        } else {
                            high_tiles - low_tiles + 1
                        };
                        let do_withhold = !withhold_tally.is_empty()
                            && (b as usize / phase_buckets).is_multiple_of(withhold_period);
                        let mut tries = 0u32;
                        let reached = loop {
                            if !do_withhold {
                                game_state
                                    .reset_and_draw_tiles_double_ended(&game_config, &mut rng);
                            } else {

                                game_state.reset();
                                game_state.bag.shuffle(&mut rng);
                                for (t, &c) in withhold_tally.iter().enumerate() {
                                    for _ in 0..c {
                                        game_state.bag.remove_tile(t as u8);
                                    }
                                }
                                let rsz = game_config.rack_size() as usize;
                                let bag = &mut game_state.bag;
                                let players = &mut game_state.players;
                                for (i, player) in players.iter_mut().enumerate() {
                                    bag.replenish(&mut player.rack, rsz, i);
                                }
                            }
                            let mut got = false;
                            if opening_samples {

                                open_buf.clear();
                            }
                            loop {
                                let fill =
                                    game_state.board_tiles.iter().filter(|&&t| t != 0).count();
                                if fill >= target {

                                    got = fill <= high_tiles;
                                    break;
                                }
                                game_state.players[game_state.turn as usize]
                                    .rack
                                    .sort_unstable();
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
                                move_generator.gen_moves_unfiltered(&movegen::GenMovesParams {
                                    board_snapshot,
                                    rack: &game_state.current_player().rack,
                                    max_gen: 1,
                                    num_exchanges_by_this_player: game_state
                                        .current_player()
                                        .num_exchanges,
                                    pass_policy: movegen::PassPolicy::OnlyWhenForced,
                                    dynamic_leaves: if game_state.turn == 0 { dyn_ref } else { None },
                                });
                                if opening_samples
                                    && game_state.current_player().rack.len() == rack_size
                                {

                                    let rank = lat.rank_bytes(&game_state.current_player().rack);
                                    if rank != !0 {
                                        open_buf.push((rank, move_generator.plays[0].equity.raw()));
                                    }
                                }
                                game_state
                                    .play(&game_config, &mut rng, &move_generator.plays[0].play)
                                    .unwrap();
                                let ended =
                                    game_state.check_game_ended(&game_config, &mut final_scores);
                                game_state.next_turn();
                                if !matches!(ended, game_state::CheckGameEnded::NotEnded) {
                                    break; // game ended before the window; try a fresh game.
                                }
                            }
                            if got {
                                break true;
                            }
                            tries += 1;
                            if tries >= 1_000_000 {
                                break false;
                            }
                        };
                        if !reached {
                            eprintln!(
                                "census: board slot {b} never reached window [{low_tiles},{high_tiles}]; skipping"
                            );
                            continue;
                        }
                        } // end of the !reuse_board game replay
                        value_board(
                            &mut move_generator,
                            &game_state,
                            &mut rng,
                            &leave,
                            null_leave,
                            b == 0,
                            verify && b == 0 && !reuse_board,

                            if sheet_reuse
                                && (reuse_board || (b as usize) < live_after[gen_idx])
                            {
                                Some(&sheet_cache[b as usize])
                            } else {
                                None
                            },
                            reuse_board,
                            num_boards,
                        );
                        if opening_samples && !open_buf.is_empty() {

                            let mut g = shared.lock().unwrap();
                            let (sum, cnt, _completed, valued, _ever) = &mut *g;
                            for &(rank, milli) in &open_buf {
                                let idx = rank as usize;
                                if cnt[idx] == 0 {
                                    *valued += 1;
                                }
                                sum[idx] += milli as f64 * opening_weight as f64;
                                cnt[idx] += opening_weight;
                            }
                        }
                    }

                    if ci_stop {
                        let completed = {
                            let g = shared.lock().unwrap();
                            g.2
                        };

                        let due = ci_check_at.load(std::sync::atomic::Ordering::Relaxed);
                        if completed >= due
                            && ci_check_at
                                .compare_exchange(
                                    due,
                                    due + ci_stop_every,
                                    std::sync::atomic::Ordering::Relaxed,
                                    std::sync::atomic::Ordering::Relaxed,
                                )
                                .is_ok()
                        {

                            let (frac, n_boards) = {
                                let g = shared.lock().unwrap();
                                let sq = ci_sumsq.lock().unwrap();
                                let mut scratch = ci_scratch.lock().unwrap();
                                let (sum, cnt, comp, _, _) = &*g;
                                let z = stats::NormalDistribution::reverse_ci(ci_conf);
                                let (varr, den, w2v) = &mut *scratch;
                                if varr.len() != lat_len {
                                    *varr = vec![0.0f64; lat_len];
                                    *den = vec![0.0f64; lat_len];
                                    *w2v = vec![0.0f64; lat_len];
                                }
                                for v in varr[..full_rack_start].iter_mut() {
                                    *v = -1.0;
                                }
                                for idx in full_rack_start..lat_len {
                                    let n = cnt[idx];
                                    varr[idx] = if n >= 2 {
                                        let var = ((sq[idx] - sum[idx] * sum[idx] / n as f64)
                                            / (n as f64 - 1.0))
                                            .max(0.0);
                                        var / n as f64
                                    } else if n == 1 {
                                        0.0
                                    } else {
                                        -1.0
                                    };
                                }
                                for idx in 0..lat_len {
                                    den[idx] = 0.0;
                                    w2v[idx] = 0.0;
                                }
                                census::entering_leave_ci_fused(
                                    &lat,
                                    varr,
                                    &base_freqs,
                                    den,
                                    w2v,
                                );
                                let mut total = 0usize;
                                let mut under = 0usize;
                                for idx in 0..full_rack_start {
                                    if den[idx] > 0.0 {
                                        total += 1;
                                        let ci_half =
                                            z * (w2v[idx] / (den[idx] * den[idx])).sqrt();
                                        if ci_half <= ci_target_mp {
                                            under += 1;
                                        }
                                    }
                                }
                                let frac = if total > 0 {
                                    under as f64 / total as f64
                                } else {
                                    0.0
                                };
                                (frac, *comp)
                            };
                            eprintln!(
                                "census CI-stop check: {n_boards} boards, {:.1}% of leaves \
                                 within target {:.0} mp (need {:.1}%)",
                                100.0 * frac,
                                ci_target_mp,
                                100.0 * ci_stop_frac,
                            );
                            if frac >= ci_stop_frac {
                                stop_now.store(true, std::sync::atomic::Ordering::Relaxed);
                                eprintln!(
                                    "census CI-stop: target met at {n_boards} boards; stopping."
                                );
                            }
                        }
                    }
                    }
                    }

                    if sgd {
                        if barrier.wait().is_leader() {
                            let mut g = shared.lock().unwrap();
                            let (sum, cnt, _completed, _valued, ever) = &mut *g;
                            let mut lv = leave_lock.write().unwrap();
                            let base = if cnt[empty_rank] > 0 {
                                sum[empty_rank] / cnt[empty_rank] as f64
                            } else {
                                0.0
                            };
                            for idx in 0..lat_len {
                                if cnt[idx] > 0 {
                                    ever[idx] = true;
                                    let centered = sum[idx] / cnt[idx] as f64 - base;
                                    lv[idx] = ((1.0 - alpha) * lv[idx] as f64 + alpha * centered)
                                        .round() as i32;
                                }
                                sum[idx] = 0.0;
                                cnt[idx] = 0;
                            }
                            next_board.store(batch_end, std::sync::atomic::Ordering::Relaxed);
                        }
                        barrier.wait();
                    }
                    batch_start = batch_end;
                    if batch_start >= num_boards {

                        if multigen {
                            if barrier.wait().is_leader() {

                                {
                                    let mut g = shared.lock().unwrap();
                                    let (sum, cnt, completed, valued, ever) = &mut *g;
                                    let mut lv = leave_lock.write().unwrap();
                                    if rack_summary {

                                        let mut rmean = vec![census::UNPLAYABLE; lat_len];
                                        for idx in full_rack_start..lat_len {
                                            if cnt[idx] > 0 {
                                                rmean[idx] =
                                                    (sum[idx] / cnt[idx] as f64).round() as i32;
                                            }
                                        }
                                        let mut gnum = vec![0f64; lat_len];
                                        let mut gden = vec![0f64; lat_len];
                                        census::generate_fused(
                                            &lat, &rmean, &base_freqs, &mut gnum, &mut gden,
                                        );
                                        let gbase = if gden[empty_rank] != 0.0 {
                                            gnum[empty_rank] / gden[empty_rank]
                                        } else {
                                            0.0
                                        };
                                        for idx in 0..lat_len {
                                            if gden[idx] != 0.0 {
                                                ever[idx] = true;
                                                lv[idx] = (gnum[idx] / gden[idx] - gbase).round()
                                                    as i32;
                                            }
                                        }
                                    } else {
                                        let base = if cnt[empty_rank] > 0 {
                                            sum[empty_rank] / cnt[empty_rank] as f64
                                        } else {
                                            0.0
                                        };
                                        for idx in 0..lat_len {
                                            if cnt[idx] > 0 {
                                                ever[idx] = true;
                                                lv[idx] = (sum[idx] / cnt[idx] as f64 - base)
                                                    .round()
                                                    as i32;
                                            }
                                        }
                                    }
                                    eprintln!(
                                        "census: gen {}/{} done ({} of {} leaves valued)",
                                        gen_idx + 1,
                                        gens,
                                        *valued,
                                        lat_len,
                                    );
                                    if gen_idx + 1 < gens {

                                        for idx in 0..lat_len {
                                            sum[idx] = 0.0;
                                            cnt[idx] = 0;
                                        }
                                        *completed = 0;
                                        *valued = 0;
                                        next_board
                                            .store(0, std::sync::atomic::Ordering::Relaxed);
                                        for h in &pool_hist {
                                            h.store(0, std::sync::atomic::Ordering::Relaxed);
                                        }
                                    }
                                }

                                if persist_gens {
                                    let g = shared.lock().unwrap();
                                    let lv = leave_lock.read().unwrap();
                                    let desired =
                                        format!("census-gen-{census_run_epoch}-{:02}.klv2", gen_idx + 1);
                                    let p = claim_output_path(&desired).unwrap_or(desired);
                                    match write_census_klv2(
                                        &lat,
                                        &|i| lv[i] as f64,
                                        0.0,
                                        &|i| g.4[i],
                                        true, // resume snapshots stay full
                                        &p,
                                    ) {
                                        Ok(nk) => eprintln!(
                                            "census: persisted gen {} -> {p} ({nk} leaves)",
                                            gen_idx + 1
                                        ),
                                        Err(e) => eprintln!(
                                            "census: gen {} klv2 persist failed: {e}",
                                            gen_idx + 1
                                        ),
                                    }
                                }

                                if sheet_reuse {
                                    for slot in sheet_cache.iter().skip(live_after[gen_idx]) {
                                        *slot.lock().unwrap() = None;
                                    }
                                }
                            }
                            barrier.wait();
                            if gen_idx + 1 < gens {

                                prior_max_boards = prior_max_boards.max(num_boards as usize);
                                gen_idx += 1;
                                num_boards = board_counts[gen_idx];
                                batch_start = 0;
                                continue;
                            }
                        }
                        break;
                    }
                }
            });
        }
    });

    let (accum_sum, accum_cnt, _, _, ever) = shared.into_inner().unwrap();
    let leave_final = leave_lock.into_inner().unwrap();

    if ci_report {
        let sumsq = ci_sumsq.into_inner().unwrap();
        let z = stats::NormalDistribution::reverse_ci(ci_conf);
        let mut ci_halves = Vec::new();
        let mut boards_needed = Vec::new();
        let mut n_under = 0usize;
        let mut sum_n = 0u64;

        let report_lo = if rack_summary { full_rack_start } else { 0 };
        for idx in report_lo..lat_len {
            let n = accum_cnt[idx];
            if n >= 2 {
                let var = ((sumsq[idx] - accum_sum[idx] * accum_sum[idx] / n as f64)
                    / (n as f64 - 1.0))
                    .max(0.0);
                let ci_half = z * (var / n as f64).sqrt();
                if ci_half <= ci_target_mp {
                    n_under += 1;
                }

                boards_needed.push(n as f64 * (ci_half / ci_target_mp.max(1.0)).powi(2));
                ci_halves.push(ci_half);
                sum_n += n;
            }
        }
        ci_halves.sort_unstable_by(|a, b| a.partial_cmp(b).unwrap());
        boards_needed.sort_unstable_by(|a, b| a.partial_cmp(b).unwrap());
        let pctl = |v: &[f64], p: f64| -> f64 {
            if v.is_empty() {
                0.0
            } else {
                v[(((v.len() - 1) as f64) * p) as usize]
            }
        };
        let m = ci_halves.len();
        eprintln!(
            "census CI report (conf {:.3}, z {:.3}, {m} entries with n>=2, avg n {:.1}):",
            ci_conf,
            z,
            if m > 0 { sum_n as f64 / m as f64 } else { 0.0 },
        );
        eprintln!(
            "  per-entry CI half-width (mp): p50 {:.1}  p90 {:.1}  p99 {:.1}  max {:.1}",
            pctl(&ci_halves, 0.5),
            pctl(&ci_halves, 0.9),
            pctl(&ci_halves, 0.99),
            ci_halves.last().copied().unwrap_or(0.0),
        );
        eprintln!(
            "  {:.1}% of entries within target {:.0} mp at the current count; \
             boards to pin a fraction: p50 {:.0}  p90 {:.0}  p99 {:.0}",
            if m > 0 {
                100.0 * n_under as f64 / m as f64
            } else {
                0.0
            },
            ci_target_mp,
            pctl(&boards_needed, 0.5),
            pctl(&boards_needed, 0.9),
            pctl(&boards_needed, 0.99),
        );

        if ci_report_level >= 2 && rack_summary {
            let mut varr = vec![-1.0f64; lat_len]; // -1 = never valued -> excluded
            for idx in full_rack_start..lat_len {
                let n = accum_cnt[idx];
                varr[idx] = if n >= 2 {
                    let var = ((sumsq[idx] - accum_sum[idx] * accum_sum[idx] / n as f64)
                        / (n as f64 - 1.0))
                        .max(0.0);
                    var / n as f64
                } else if n == 1 {
                    0.0 // single sample: across-board variance unknown, treated as 0
                } else {
                    -1.0 // never valued
                };
            }
            let mut den = vec![0.0f64; lat_len];
            let mut w2v = vec![0.0f64; lat_len];
            census::entering_leave_ci_fused(&lat, &varr, &base_freqs, &mut den, &mut w2v);
            let mut leave_ci = Vec::new();
            let mut leave_scale = Vec::new();
            let mut leave_under = 0usize;
            for idx in 0..full_rack_start {
                if den[idx] > 0.0 {
                    let ci_half = z * (w2v[idx] / (den[idx] * den[idx])).sqrt();
                    if ci_half <= ci_target_mp {
                        leave_under += 1;
                    }
                    leave_ci.push(ci_half);

                    leave_scale.push((ci_half / ci_target_mp.max(1.0)).powi(2));
                }
            }
            leave_ci.sort_unstable_by(|a, b| a.partial_cmp(b).unwrap());
            leave_scale.sort_unstable_by(|a, b| a.partial_cmp(b).unwrap());
            let lm = leave_ci.len();
            eprintln!("  leave-level CI ({lm} leaves, draw-ways-propagated):");
            eprintln!(
                "    half-width (mp): p50 {:.2}  p90 {:.2}  p99 {:.2}  max {:.2}",
                pctl(&leave_ci, 0.5),
                pctl(&leave_ci, 0.9),
                pctl(&leave_ci, 0.99),
                leave_ci.last().copied().unwrap_or(0.0),
            );
            eprintln!(
                "    {:.1}% of leaves within target {:.0} mp; board-scale x_current to pin a \
                 fraction: p50 {:.3}  p90 {:.3}  p99 {:.3}",
                if lm > 0 {
                    100.0 * leave_under as f64 / lm as f64
                } else {
                    0.0
                },
                ci_target_mp,
                pctl(&leave_scale, 0.5),
                pctl(&leave_scale, 0.9),
                pctl(&leave_scale, 0.99),
            );
        }
    }

    let (ga_num, ga_den) = if global_apportion && !rack_summary {
        let mut vr = vec![census::UNPLAYABLE; lat_len];
        for idx in full_rack_start..lat_len {
            if accum_cnt[idx] > 0 {
                vr[idx] = (accum_sum[idx] / accum_cnt[idx] as f64).round() as i32;
            }
        }
        let mut gn = vec![0i128; lat_len];
        let mut gd = vec![0i128; lat_len];
        census::entering_fused(&lat, &vr, &base_freqs, &mut gn, &mut gd);
        (gn, gd)
    } else {
        (Vec::new(), Vec::new())
    };

    let value_mp = |idx: usize| -> f64 {
        if sgd || multigen {
            leave_final[idx] as f64
        } else if global_apportion {
            if ga_den[idx] != 0 {
                (ga_num[idx] / ga_den[idx]) as f64
            } else {
                0.0
            }
        } else if accum_cnt[idx] > 0 {
            accum_sum[idx] / accum_cnt[idx] as f64
        } else {
            0.0
        }
    };
    if rack_summary && !multigen {
        let summary_name = claim_output_path(&format!("census-summary-{census_run_epoch}.csv"))?;
        let mut sw = csv::Writer::from_path(&summary_name)?;
        let mut tally_buf = vec![0u8; num_letters];
        let mut leave_ser = String::new();

        let globally_possible = |idx: usize, tally: &mut [u8]| -> bool {
            lat.unrank_into(idx, tally);
            (0..num_letters).all(|t| tally[t] <= base_freqs[t])
        };
        let mut tot_e = 0f64;
        let mut tot_c = 0u64;
        for idx in full_rack_start..lat.len() {
            if accum_cnt[idx] > 0 && globally_possible(idx, &mut tally_buf) {
                tot_e += accum_sum[idx] / equity::SCALE as f64;
                tot_c += accum_cnt[idx];
            }
        }
        sw.serialize(("", tot_e, tot_c))?;
        let mut nrows = 0usize;
        for idx in full_rack_start..lat.len() {
            if accum_cnt[idx] == 0 || !globally_possible(idx, &mut tally_buf) {
                continue;
            }
            leave_ser.clear();
            for (t, &c) in tally_buf.iter().enumerate() {
                for _ in 0..c {
                    leave_ser.push_str(alphabet.of_rack(t as u8).unwrap());
                }
            }
            sw.serialize((
                &leave_ser,
                accum_sum[idx] / equity::SCALE as f64,
                accum_cnt[idx],
            ))?;
            nrows += 1;
        }
        sw.flush()?;
        eprintln!(
            "census: wrote autoplay-faithful summary ({nrows} full racks) to {summary_name} in {}s",
            t0.elapsed().as_secs(),
        );
        return Ok(());
    }
    let baseline = value_mp(empty_rank);
    let out_name = claim_output_path(&format!("census-leaves-{census_run_epoch}.csv"))?;

    let emit_full = env_flag("WOLGES_FULL", false);
    let max_keep = if emit_full {
        rack_size
    } else {
        rack_size.saturating_sub(1)
    };
    let mut rows: Vec<(usize, String, f64)> = Vec::new();
    let mut leave_ser = String::new();
    for idx in 0..lat.len() {
        let valued = if sgd || multigen {
            ever[idx]
        } else if global_apportion {
            ga_den[idx] != 0
        } else {
            accum_cnt[idx] > 0
        };
        if !valued {
            continue;
        }
        lat.unrank_into(idx, &mut tally_buf);
        let size: usize = tally_buf.iter().map(|&c| c as usize).sum();
        if size == 0 || size > max_keep {
            continue; // skip empty (baseline), over-size, and (non-full) full racks.
        }
        let centered_points = (value_mp(idx) - baseline) / equity::SCALE as f64;
        leave_ser.clear();
        for (t, &c) in tally_buf.iter().enumerate() {
            for _ in 0..c {
                leave_ser.push_str(alphabet.of_rack(t as u8).unwrap());
            }
        }
        rows.push((size, leave_ser.clone(), centered_points));
    }
    rows.sort_unstable_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.cmp(&b.1)));
    let mut csv_out = csv::Writer::from_path(&out_name)?;
    for (_, leave, value) in &rows {
        csv_out.serialize((leave, value))?;
    }
    csv_out.flush()?;
    eprintln!(
        "census: wrote {} leaves to {} in {}s (baseline {:.3} pts)",
        rows.len(),
        out_name,
        t0.elapsed().as_secs(),
        baseline / equity::SCALE as f64,
    );

    let klv_name = claim_output_path(&format!("census-leaves-{census_run_epoch}.klv2"))?;
    let is_valued = |idx: usize| {
        if sgd || multigen {
            ever[idx]
        } else {
            accum_cnt[idx] > 0
        }
    };
    let n_klv = write_census_klv2(&lat, &value_mp, baseline, &is_valued, emit_full, &klv_name)?;
    eprintln!("census: wrote klv2 to {klv_name} ({n_klv} leaves)");
    Ok(())
}

fn decompose_contribution(fv: &Cumulate, w: u64, per_rack: bool) -> (f64, u64) {
    if per_rack {
        (fv.equity / fv.count as f64 * w as f64, w)
    } else {
        (fv.equity * w as f64, fv.count * w)
    }
}

fn generate_leaves<Readable: std::io::Read, W: std::io::Write, const IS_FULL_RACK: bool>(
    game_config: game_config::GameConfig,
    mut csv_in: csv::Reader<Readable>,
    mut csv_out: csv::Writer<W>,
    rare_path: Option<&str>,
) -> error::Returns<()> {
    let mut stdout_or_stderr = boxed_stdout_or_stderr();

    let per_rack = env_flag("WOLGES_GENERATE_PER_RACK", true);
    let mut rack_tally = vec![0u8; game_config.alphabet().len() as usize];
    let mut exchange_buffer = Vec::with_capacity(game_config.rack_size() as usize);
    let mut rack_bytes = Vec::new();
    let rack_reader = alphabet::AlphabetReader::new_for_racks(game_config.alphabet());
    let mut full_rack_map = fash::MyHashMap::<bites::Bites, Cumulate>::default();
    let t0 = std::time::Instant::now();
    let mut tick_periods = move_picker::Periods(0);
    for result in csv_in.records() {
        let record = result?;
        parse_rack(&rack_reader, &record[0], &mut rack_bytes)?;
        let thing = Cumulate {
            equity: f64::from_str(&record[1])?,
            count: u64::from_str(&record[2])?,
            sumsq: f64::NAN,
        };
        full_rack_map
            .entry(rack_bytes[..].into())
            .and_modify(|e| {
                e.equity += thing.equity;
                e.count += thing.count;
                e.sumsq += thing.sumsq;
            })
            .or_insert(thing);
    }
    drop(csv_in);
    // ("", total_equity, row_count) must exist.
    full_rack_map
        .remove([][..].into())
        .ok_or("input file does not include totals line")?;

    if let Some(fp) = env_path("WOLGES_GENERATE_SMOOTH_SQ") {
        let mut sq_reader = csv::ReaderBuilder::new()
            .has_headers(false)
            .from_path(&fp)?;
        let mut n_sq = 0u64;
        let mut n_stale = 0u64;
        for result in sq_reader.records() {
            let record = result?;

            if record[0].is_empty() {
                continue;
            }
            parse_rack(&rack_reader, &record[0], &mut rack_bytes)?;
            if let Some(e) = full_rack_map.get_mut(&rack_bytes[..]) {
                if u64::from_str(&record[2])? == e.count {
                    let v = f64::from_str(&record[1])?;

                    e.sumsq = if e.sumsq.is_nan() { v } else { e.sumsq + v };
                    n_sq += 1;
                } else {
                    n_stale += 1;
                }
            }
        }
        writeln!(
            stdout_or_stderr,
            "read {n_sq} sum-of-squares rows from {fp}{}",
            if n_stale == 0 {
                String::new()
            } else {
                format!(" ({n_stale} racks skipped: count disagrees with the summary)")
            }
        )?;
    }

    let leave_size = game_config.rack_size() - 1 + IS_FULL_RACK as u8;

    // subrack_map[subrack] = sum(full_rack_map[subrack + completion]).
    let mut subrack_map = fash::MyHashMap::<bites::Bites, Cumulate>::default();

    let mut subrack_support = fash::MyHashMap::<bites::Bites, u64>::default();

    let mut subrack_raw = fash::MyHashMap::<bites::Bites, (f64, f64)>::default();
    {
        let word_prob = prob::WordProbability::new(game_config.alphabet());
        let mut full_rack_tally = vec![0u8; rack_tally.len()];
        let mut subrack_tally = vec![0u8; rack_tally.len()];
        for (idx, (k, fv)) in full_rack_map.iter().enumerate() {
            rack_tally.iter_mut().for_each(|m| *m = 0);
            k.iter().for_each(|&tile| rack_tally[tile as usize] += 1);
            full_rack_tally.clone_from(&rack_tally);
            generate_exchanges(&mut ExchangeEnv {
                found_exchange_move: |subrack_bytes: &[u8]| {
                    subrack_tally.iter_mut().for_each(|m| *m = 0);
                    subrack_bytes
                        .iter()
                        .for_each(|&tile| subrack_tally[tile as usize] += 1);
                    let w = word_prob.completion_draw_ways(
                        &full_rack_tally,
                        &subrack_tally,
                        word_prob.bag(),
                    );
                    let (add_equity, add_count) = decompose_contribution(fv, w, per_rack);
                    subrack_map
                        .entry(subrack_bytes.into())
                        .and_modify(|v| {
                            v.equity += add_equity;
                            v.count += add_count;
                        })
                        .or_insert_with(|| Cumulate {
                            equity: add_equity,
                            count: add_count,
                            sumsq: 0.0,
                        });

                    *subrack_support.entry(subrack_bytes.into()).or_insert(0u64) += fv.count;

                    let e = subrack_raw
                        .entry(subrack_bytes.into())
                        .or_insert((0.0f64, 0.0f64));
                    e.0 += fv.equity;
                    e.1 += fv.sumsq;
                },
                rack_tally: &mut rack_tally,
                min_len: 0,
                max_len: leave_size,
                exchange_buffer: &mut exchange_buffer,
            });
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
    }
    writeln!(stdout_or_stderr, "{} unique subracks", subrack_map.len())?;
    // take out subrack_map[""] now before it gets smoothed.
    let Cumulate {
        equity: total_equity,
        count: row_count,
        sumsq: _,
    } = subrack_map
        .remove([][..].into())
        .ok_or("empty-rack entry should not be missing")?;

    if let Some(fp) = rare_path {
        let mut rare_reader = csv::ReaderBuilder::new().has_headers(false).from_path(fp)?;
        for result in rare_reader.records() {
            let record = result?;
            if record[0].is_empty() {
                continue;
            }
            let equity = f64::from_str(&record[1])?;
            let count = u64::from_str(&record[2])?;

            let sumsq = f64::NAN;
            parse_rack(&rack_reader, &record[0], &mut rack_bytes)?;
            pool_rare_one(&mut subrack_map, &rack_bytes, equity, count, sumsq);
            *subrack_support.entry(rack_bytes[..].into()).or_insert(0u64) += count;
            let e = subrack_raw
                .entry(rack_bytes[..].into())
                .or_insert((0.0f64, 0.0f64));
            e.0 += equity;
            e.1 += sumsq;
        }
    }

    let smooth_min = env_usize("WOLGES_GENERATE_SMOOTH_MIN", 50) as u64;

    let smooth_ci = env_parse::<f64>("WOLGES_GENERATE_SMOOTH_CI", 0.0);
    let smooth_ci_conf = env_parse::<f64>("WOLGES_GENERATE_SMOOTH_CONF", 0.99);
    let smooth_ci_conf = if smooth_ci_conf > 0.0 && smooth_ci_conf < 1.0 {
        smooth_ci_conf
    } else {
        0.99
    };
    let smooth_ci_z = if smooth_ci > 0.0 {
        stats::NormalDistribution::reverse_ci(smooth_ci_conf)
    } else {
        0.0
    };

    let well_sampled = |rack: &[u8], support: u64| -> bool {
        if support < smooth_min {
            return false;
        }
        if smooth_ci <= 0.0 {
            return true;
        }
        match subrack_raw.get(rack) {
            Some(&(sum, sumsq)) if support > 1 && sumsq.is_finite() => {
                let n = support as f64;
                let mean = sum / n;

                let var = ((sumsq - n * mean.powi(2)) / (n - 1.0)).max(0.0);
                smooth_ci_z * (var / n).sqrt() <= smooth_ci
            }
            _ => true,
        }
    };
    let mut ev_map = fash::MyHashMap::<bites::Bites, _>::default();
    let mut alphabet_freqs = (0..game_config.alphabet().len())
        .map(|tile| game_config.alphabet().freq(tile))
        .collect::<Box<_>>();
    let mut neighbor_buffer = Vec::with_capacity(game_config.rack_size() as usize);
    let mut num_smoothed = 0u64;
    generate_exchanges(&mut ExchangeEnv {
        found_exchange_move: |rack_bytes: &[u8]| {
            let mut new_v = if let Some(v) = subrack_map.get(rack_bytes) {
                if well_sampled(
                    rack_bytes,
                    subrack_support.get(rack_bytes).copied().unwrap_or(0),
                ) {
                    v.equity / v.count as f64
                } else {
                    // perform smoothing if there are too few samples.
                    f64::NAN
                }
            } else {
                f64::NAN
            };
            if new_v.is_nan() {
                rack_tally.iter_mut().for_each(|m| *m = 0);
                rack_bytes
                    .iter()
                    .for_each(|&tile| rack_tally[tile as usize] += 1);
                let mut equity = 0.0f64;
                let mut count = 0u64;
                // combine distinct neighbors with the few samples of self.
                // each rack is weighted only by sample count, not probability.
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
                    new_v = equity / count as f64;
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
        max_len: leave_size,
        exchange_buffer: &mut exchange_buffer,
    });
    drop(neighbor_buffer);

    let smooth_rule = if smooth_ci > 0.0 {
        format!("support floor {smooth_min} or interval wider than {smooth_ci}")
    } else {
        format!("support floor {smooth_min}")
    };
    writeln!(
        stdout_or_stderr,
        "After {} seconds, have processed {} subracks and smoothed {} ({:.1}%, rule: {})",
        t0.elapsed().as_secs(),
        ev_map.len(),
        num_smoothed,
        if ev_map.is_empty() {
            0.0
        } else {
            100.0 * num_smoothed as f64 / ev_map.len() as f64
        },
        smooth_rule,
    )?;
    {
        // make expected values relative to value of empty rack.
        // however, that is before smoothing.
        // no after-smoothing value, because of chicken-and-egg issue.
        // therefore value of empty rack might not be zero after all.
        let mean_equity = total_equity / row_count as f64;
        for v in ev_map.values_mut() {
            *v -= mean_equity;
        }
    }
    let mut num_filled_in = 0u64;

    let mut subrack_bytes = Vec::with_capacity(leave_size as usize);
    for len_to_complete in 2..=leave_size {
        let len_minus_one = len_to_complete as usize - 1;
        // ensure every subrack of each length has samples.
        // if not, fill it in based on subracks one tile fewer.
        generate_exchanges(&mut ExchangeEnv {
            found_exchange_move: |rack_bytes: &[u8]| {
                if ev_map.get(rack_bytes).unwrap_or(&f64::NAN).is_nan() {
                    let mut vn = 0.0f64;
                    let mut vd = 0i64;
                    let mut process_subrack = |v: f64| {
                        if !v.is_nan() {
                            vn += v;
                            vd += 1;
                        }
                    };
                    // process each subrack one tile fewer.
                    // on duplicate tiles, count it that many times.
                    subrack_bytes.clear();
                    subrack_bytes.extend_from_slice(rack_bytes);
                    let mut v = *ev_map
                        .get(&subrack_bytes[..len_minus_one])
                        .unwrap_or(&f64::NAN);
                    process_subrack(v);
                    for which_tile in (0..len_minus_one).rev() {
                        let c1 = subrack_bytes[which_tile];
                        let c2 = subrack_bytes[len_minus_one];
                        if c1 != c2 {
                            subrack_bytes[which_tile] = c2;
                            subrack_bytes[len_minus_one] = c1;
                            v = *ev_map
                                .get(&subrack_bytes[..len_minus_one])
                                .unwrap_or(&f64::NAN);
                        }
                        process_subrack(v);
                    }
                    if vd > 0 {
                        // just use straight average.
                        ev_map.insert(rack_bytes.into(), vn / vd as f64);
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
        });
    }
    writeln!(
        stdout_or_stderr,
        "After {} seconds, have processed {} subracks, smoothed {}, filled in {}",
        t0.elapsed().as_secs(),
        ev_map.len(),
        num_smoothed,
        num_filled_in,
    )?;

    let oppdenial_leave = env_parse::<f64>("WOLGES_OPPDENIAL_LEAVE", 0.0);
    if oppdenial_leave != 0.0 {
        let path = oppdenial_leave_marginal_path();
        if std::path::Path::new(&path).exists() {
            let num_letters = game_config.alphabet().len() as usize;
            let avg_marginal = load_oppdenial_leave_marginal_sidecar(&path, num_letters)?;
            for (k, v) in ev_map.iter_mut() {
                let mut d = 0.0f64;
                for &tile in k.iter() {
                    d += avg_marginal[tile as usize];
                }
                *v += oppdenial_leave * d / equity::SCALE as f64;
            }
            writeln!(
                stdout_or_stderr,
                "generate: folded WOLGES_OPPDENIAL_LEAVE={oppdenial_leave} from {path}"
            )?;
        } else {
            writeln!(
                stdout_or_stderr,
                "generate: WOLGES_OPPDENIAL_LEAVE={oppdenial_leave} set but sidecar {path} not found; leaves unchanged"
            )?;
        }
    }

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

fn discover_playability<N: kwg::Node + Sync + Send, L: kwg::Node + Sync + Send>(
    game_config: game_config::GameConfig,
    kwg: kwg::Kwg<N>,
    klv: klv::Klv<L>,
    num_games: u64,
    seed: Option<u64>,
) -> error::Returns<()> {
    let game_config = std::sync::Arc::new(game_config);
    let kwg = std::sync::Arc::new(kwg);
    let klv = std::sync::Arc::new(klv);
    let seed = seed.unwrap_or_else(rand::random);
    eprintln!("seed: {seed}");
    let num_threads = wolges_threads();
    let num_processed_games = std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0));

    let run_identifier = std::sync::Arc::new(run_stamp());
    eprintln!("run identifier is {run_identifier}");
    let completed_games = std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0));
    let logged_games = std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0));
    let completed_moves = std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0));
    let full_word_map = fash::MyHashMap::<bites::Bites, Cumulate>::default();
    let t0 = std::time::Instant::now();
    let tick_periods = move_picker::Periods(0);
    struct MutexedStuffs {
        full_word_map: fash::MyHashMap<bites::Bites, Cumulate>,
        tick_periods: move_picker::Periods,
    }
    let mutexed_stuffs = std::sync::Arc::new(std::sync::Mutex::new(MutexedStuffs {
        full_word_map,
        tick_periods,
    }));
    let batch_size = match game_config.game_rules() {
        game_config::GameRules::Classic => 100,
        game_config::GameRules::Jumbled => 1,
    };

    std::thread::scope(|s| {
        let mut threads = vec![];

        for _thread_id in 0..num_threads {
            let game_config = std::sync::Arc::clone(&game_config);
            let kwg = std::sync::Arc::clone(&kwg);
            let klv = std::sync::Arc::clone(&klv);
            let num_processed_games = std::sync::Arc::clone(&num_processed_games);
            let run_identifier = std::sync::Arc::clone(&run_identifier);
            let completed_games = std::sync::Arc::clone(&completed_games);
            let logged_games = std::sync::Arc::clone(&logged_games);
            let completed_moves = std::sync::Arc::clone(&completed_moves);
            let mutexed_stuffs = std::sync::Arc::clone(&mutexed_stuffs);
            threads.push(s.spawn(move || {
                let mut rng = rand::rngs::ChaCha20Rng::seed_from_u64(seed);
                let mut move_generator = movegen::KurniaMoveGenerator::new(&game_config);
                let mut game_state = game_state::GameState::new(&game_config);
                let mut final_scores = vec![0; game_config.num_players() as usize];
                let mut num_batched_games_here = 0;
                let mut thread_full_word_map = fash::MyHashMap::<bites::Bites, Cumulate>::default();
                let mut word_iter = move_filter::LimitedVocabChecker::new();
                let mut unjumble_buf = match game_config.game_rules() {
                    game_config::GameRules::Classic => Vec::new(),
                    game_config::GameRules::Jumbled => Vec::with_capacity(
                        game_config
                            .board_layout()
                            .dim()
                            .rows
                            .max(game_config.board_layout().dim().cols)
                            as usize,
                    ),
                };
                let mut tally_word =
                    |v: &mut Vec<(bites::Bites, usize)>, num_plays: usize, w: &[u8]| {
                        match game_config.game_rules() {
                            game_config::GameRules::Classic => {
                                v.push((w.into(), num_plays));
                            }
                            game_config::GameRules::Jumbled => {
                                if w.windows(2).all(|x| x[0] <= x[1]) {
                                    v.push((w.into(), num_plays));
                                } else {

                                    let w_len = w.len();
                                    unjumble_buf.resize(w_len.max(unjumble_buf.len()), 0);
                                    unjumble_buf[..w_len].clone_from_slice(w);
                                    unjumble_buf[..w_len].sort_unstable();
                                    v.push((unjumble_buf[..w_len].into(), num_plays));
                                }
                            }
                        }
                    };

                let mut vec_played = Vec::<(bites::Bites, usize)>::new();
                loop {
                    let num_prior_games =
                        num_processed_games.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    if num_prior_games >= num_games {
                        num_processed_games.fetch_sub(1, std::sync::atomic::Ordering::Relaxed);
                        break;
                    }
                    rng.set_stream(num_prior_games);

                    game_state.reset_and_draw_tiles_double_ended(&game_config, &mut rng);
                    loop {
                        game_state.players[game_state.turn as usize]
                            .rack
                            .sort_unstable();
                        let cur_rack = &game_state.current_player().rack;

                        let old_bag_len = game_state.bag.len();

                        let board_snapshot = &movegen::BoardSnapshot {
                            board_tiles: &game_state.board_tiles,
                            game_config: &game_config,
                            kwg: &kwg,
                            klv: &klv,
                        };

                        let moves_made_before_ending: u64 = if old_bag_len > 0 {
                            let mut best_equity_so_far = equity::Equity::NEG_INFINITY;
                            let mut num_plays = 0usize;
                            vec_played.clear();
                            move_generator.gen_moves_filtered(
                                &movegen::GenMovesParams {
                                    board_snapshot,
                                    rack: cur_rack,
                                    max_gen: 2, // to allow finding equal-equity plays.
                                    num_exchanges_by_this_player: game_state
                                        .current_player()
                                        .num_exchanges,
                                    pass_policy: movegen::PassPolicy::OnlyWhenForced,
                                    dynamic_leaves: None,
                                },
                                |_down: bool, _lane: i8, _idx: i8, _word: &[u8], _score: i32| true,
                                |leave_value: i32| leave_value,
                                |equity: equity::Equity, play: &movegen::Play| {
                                    match equity.cmp(&best_equity_so_far) {
                                        std::cmp::Ordering::Greater => {
                                            best_equity_so_far = equity;
                                            vec_played.clear();
                                            num_plays = 0;
                                            match play {
                                                movegen::Play::Exchange { .. } => {}
                                                movegen::Play::Place {
                                                    down,
                                                    lane,
                                                    idx,
                                                    word,
                                                    ..
                                                } => {
                                                    word_iter.words_placed_are_ok(
                                                        board_snapshot,
                                                        *down,
                                                        *lane,
                                                        *idx,
                                                        &word[..],
                                                        |w: &[u8]| {
                                                            tally_word(
                                                                &mut vec_played,
                                                                num_plays,
                                                                w,
                                                            );
                                                            true
                                                        },
                                                    );
                                                }
                                            }
                                            num_plays += 1;
                                            true
                                        }
                                        std::cmp::Ordering::Equal => {
                                            match play {
                                                movegen::Play::Exchange { .. } => {}
                                                movegen::Play::Place {
                                                    down,
                                                    lane,
                                                    idx,
                                                    word,
                                                    ..
                                                } => {
                                                    word_iter.words_placed_are_ok(
                                                        board_snapshot,
                                                        *down,
                                                        *lane,
                                                        *idx,
                                                        &word[..],
                                                        |w: &[u8]| {
                                                            tally_word(
                                                                &mut vec_played,
                                                                num_plays,
                                                                w,
                                                            );
                                                            true
                                                        },
                                                    );
                                                }
                                            }
                                            num_plays += 1;
                                            false // ensure top two have different equities.
                                        }
                                        std::cmp::Ordering::Less => false,
                                    }
                                },
                            );

                            if num_plays > 0 {
                                vec_played.sort_unstable();
                                vec_played.dedup(); // playing the same word as main+hook or hook+hook counts once.

                                let multiplier = (num_plays as f64).recip();
                                for same_words in vec_played.chunk_by(|a, b| a.0 == b.0) {
                                    let occurrence = same_words.len() as f64 * multiplier;

                                    thread_full_word_map
                                        .entry(same_words[0].0[..].into())
                                        .and_modify(|e| {
                                            e.equity += occurrence;
                                            e.count += 1;
                                        })
                                        .or_insert(Cumulate {
                                            equity: occurrence,
                                            sumsq: f64::NAN,
                                            count: 1,
                                        });
                                }
                            }

                            let plays = &move_generator.plays;
                            let play = &plays[0];

                            game_state.play(&game_config, &mut rng, &play.play).unwrap();

                            match game_state.check_game_ended(&game_config, &mut final_scores) {
                                game_state::CheckGameEnded::PlayedOut
                                | game_state::CheckGameEnded::ZeroScores => 1,
                                game_state::CheckGameEnded::NotEnded => !0,
                            }
                        } else {

                            0
                        };

                        if moves_made_before_ending != !0 {
                            let completed_moves = completed_moves.fetch_add(
                                moves_made_before_ending,
                                std::sync::atomic::Ordering::Relaxed,
                            );
                            completed_games.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                            num_batched_games_here += 1;
                            if num_batched_games_here >= batch_size {

                                let logged_games = logged_games.fetch_add(
                                    num_batched_games_here,
                                    std::sync::atomic::Ordering::Relaxed,
                                ) + num_batched_games_here;
                                num_batched_games_here = 0;
                                let elapsed_time_secs = t0.elapsed().as_secs();
                                let tick_changed = {
                                    let mut mutex_guard = mutexed_stuffs.lock().unwrap();
                                    mutex_guard.tick_periods.update(elapsed_time_secs)
                                };
                                if tick_changed {
                                    eprintln!(
                                        "After {elapsed_time_secs} seconds, have played {logged_games} games ({completed_moves} moves) for {run_identifier}"
                                    );
                                }
                            }
                            break;
                        }

                        completed_moves.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                        game_state.next_turn();
                    }
                }

                let mut mutex_guard = mutexed_stuffs.lock().unwrap();

                for (k, thread_v) in thread_full_word_map.into_iter() {
                    if thread_v.count > 0 {
                        mutex_guard
                            .full_word_map
                            .entry(k)
                            .and_modify(|v| {
                                v.equity += thread_v.equity;
                                v.count += thread_v.count;
                            })
                            .or_insert(thread_v);
                    }
                }
            }));
        }

        for thread in threads {
            if let Err(e) = thread.join() {
                eprintln!("{e:?}");
            }
        }
    });

    {
        let mutex_guard = mutexed_stuffs.lock().unwrap();
        let full_word_map = &mutex_guard.full_word_map;

        let mut total_equity = 0.0;
        let mut row_count = 0;
        for x in full_word_map.values() {
            total_equity += x.equity;
            row_count += x.count;
        }

        eprintln!(
            "{} records, {} unique words",
            row_count,
            full_word_map.len()
        );

        let mut kv = full_word_map.iter().collect::<Vec<_>>();
        kv.sort_unstable_by(|a, b| {
            a.0.len()
                .cmp(&b.0.len())
                .then_with(|| b.1.equity.total_cmp(&a.1.equity).then_with(|| a.0.cmp(b.0)))
        });

        let mut csv_out =
            csv::Writer::from_path(claim_output_path(&format!("playability-{run_identifier}"))?)?;
        let mut cur_word_ser = String::new();
        csv_out.serialize(("", total_equity, row_count))?;
        for (k, fv) in kv.iter() {
            cur_word_ser.clear();
            for &tile in k.iter() {
                // using of_board because blanks should not be possible.
                cur_word_ser.push_str(game_config.alphabet().of_board(tile).unwrap());
            }
            csv_out.serialize((&cur_word_ser, fv.equity, fv.count))?;
        }
    }

    eprintln!(
        "After {} seconds, have played {} games ({} moves) for {}",
        t0.elapsed().as_secs(),
        completed_games.load(std::sync::atomic::Ordering::Relaxed),
        completed_moves.load(std::sync::atomic::Ordering::Relaxed),
        run_identifier
    );

    Ok(())
}

fn plural<'a>(n: u64, singular: &'a str, plural: &'a str) -> &'a str {
    if n == 1 { singular } else { plural }
}

struct GameStats {
    p0_wins: u64,
    p0_losses: u64,
    p0_draws: u64,
    p0_score: stats::Stats,
    p1_score: stats::Stats,
    turns: stats::Stats,
    played_out: u64,
    zero_scores: u64,
}

impl GameStats {
    fn new() -> Self {
        Self {
            p0_wins: 0,
            p0_losses: 0,
            p0_draws: 0,
            p0_score: stats::Stats::new(),
            p1_score: stats::Stats::new(),
            turns: stats::Stats::new(),
            played_out: 0,
            zero_scores: 0,
        }
    }

    fn add_game(
        &mut self,
        p0_final: i32,
        p1_final: i32,
        turns: u32,
        end_reason: game_state::CheckGameEnded,
    ) {
        self.p0_score.update(equity::descale_score(p0_final) as f64);
        self.p1_score.update(equity::descale_score(p1_final) as f64);
        self.turns.update(turns as f64);
        match end_reason {
            game_state::CheckGameEnded::PlayedOut => self.played_out += 1,
            game_state::CheckGameEnded::ZeroScores => self.zero_scores += 1,
            game_state::CheckGameEnded::NotEnded => {}
        }
        match p0_final.cmp(&p1_final) {
            std::cmp::Ordering::Greater => self.p0_wins += 1,
            std::cmp::Ordering::Less => self.p0_losses += 1,
            std::cmp::Ordering::Equal => self.p0_draws += 1,
        }
    }

    fn merge(&mut self, other: &GameStats) {
        self.p0_wins += other.p0_wins;
        self.p0_losses += other.p0_losses;
        self.p0_draws += other.p0_draws;
        self.p0_score.update_bulk(&other.p0_score);
        self.p1_score.update_bulk(&other.p1_score);
        self.turns.update_bulk(&other.turns);
        self.played_out += other.played_out;
        self.zero_scores += other.zero_scores;
    }

    fn total_games(&self) -> u64 {
        self.p0_wins + self.p0_losses + self.p0_draws
    }

    fn print(&self, label: &str) {
        let total = self.total_games();
        if total == 0 {
            return;
        }
        let p0_total = self.p0_wins as f64 + self.p0_draws as f64 / 2.0;
        let p1_total = total as f64 - p0_total;
        println!("{label}");
        println!(
            "  turns per game: {:.2} (sd={:.2})",
            self.turns.mean(),
            self.turns.standard_deviation(),
        );
        println!(
            "  played out: {} ({:.2}%)  zero scores: {} ({:.2}%)",
            self.played_out,
            self.played_out as f64 / total as f64 * 100.0,
            self.zero_scores,
            self.zero_scores as f64 / total as f64 * 100.0,
        );
        println!(
            "  p0 (klv0): {:.1} ({:.2}%)  p1 (klv1): {:.1} ({:.2}%)",
            p0_total,
            p0_total / total as f64 * 100.0,
            p1_total,
            p1_total / total as f64 * 100.0,
        );
        println!(
            "  wins: {} ({:.2}%)  losses: {} ({:.2}%)  draws: {} ({:.2}%)",
            self.p0_wins,
            self.p0_wins as f64 / total as f64 * 100.0,
            self.p0_losses,
            self.p0_losses as f64 / total as f64 * 100.0,
            self.p0_draws,
            self.p0_draws as f64 / total as f64 * 100.0,
        );
        println!(
            "  score: p0={:.2} (sd={:.2})  p1={:.2} (sd={:.2})",
            self.p0_score.mean(),
            self.p0_score.standard_deviation(),
            self.p1_score.mean(),
            self.p1_score.standard_deviation(),
        );
        let corrected_pct = (p0_total.max(p1_total) - 0.5) / total as f64;
        if corrected_pct > 0.5 {
            let z = (corrected_pct - 0.5) * 2.0 * (total as f64).sqrt();
            let confidence = stats::NormalDistribution::cumulative_normal_density(z) * 100.0;
            let leading = if p0_total > p1_total {
                "p0 (klv0)"
            } else {
                "p1 (klv1)"
            };
            println!("  {leading} leads, confidence: {confidence:.2}%");
        } else {
            println!("  no significant difference");
        }
    }

    fn print_porcelain(&self) {
        let total = self.total_games();
        if total == 0 {
            return;
        }
        let p0_total = self.p0_wins as f64 + self.p0_draws as f64 / 2.0;
        let p1_total = total as f64 - p0_total;
        let corrected_pct = (p0_total.max(p1_total) - 0.5) / total as f64;
        let confidence = if corrected_pct > 0.5 {
            let z = (corrected_pct - 0.5) * 2.0 * (total as f64).sqrt();
            stats::NormalDistribution::cumulative_normal_density(z) * 100.0
        } else {
            0.0
        };
        println!("WCMP_P0_PCT {:.4}", p0_total / total as f64 * 100.0);
        println!("WCMP_P1_PCT {:.4}", p1_total / total as f64 * 100.0);
        println!(
            "WCMP_P0_WINS_PCT {:.4}",
            self.p0_wins as f64 / total as f64 * 100.0
        );
        println!(
            "WCMP_DRAWS_PCT {:.4}",
            self.p0_draws as f64 / total as f64 * 100.0
        );
        println!("WCMP_CONF_PCT {confidence:.4}");
        println!("WCMP_LEADER {}", if p0_total >= p1_total { 0 } else { 1 });
        println!("WCMP_GAMES {total}");
        println!("WCMP_PAIRS {}", total / 2);
    }
}

struct GamePairStats {
    all: GameStats,
    divergent: GameStats,
}

impl GamePairStats {
    fn new() -> Self {
        Self {
            all: GameStats::new(),
            divergent: GameStats::new(),
        }
    }

    fn add_game(
        &mut self,
        p0_final: i32,
        p1_final: i32,
        turns: u32,
        end_reason: game_state::CheckGameEnded,
        divergent: bool,
    ) {
        self.all.add_game(p0_final, p1_final, turns, end_reason);
        if divergent {
            self.divergent
                .add_game(p0_final, p1_final, turns, end_reason);
        }
    }

    fn merge(&mut self, other: &GamePairStats) {
        self.all.merge(&other.all);
        self.divergent.merge(&other.divergent);
    }

    fn print(&self) {
        let all_total = self.all.total_games();
        let all_pairs = all_total / 2;
        self.all.print(&format!(
            "{all_total} {} ({all_pairs} {}):",
            plural(all_total, "game", "games"),
            plural(all_pairs, "pair", "pairs"),
        ));
        let div_total = self.divergent.total_games();
        if div_total > 0 && div_total < all_total {
            let div_pairs = div_total / 2;
            self.divergent.print(&format!(
                "\n{div_total} divergent {} ({div_pairs} {} = {:.2}%):",
                plural(div_total, "game", "games"),
                plural(div_pairs, "pair", "pairs"),
                div_pairs as f64 / all_pairs as f64 * 100.0,
            ));
        }

        let porcelain = std::env::var("WOLGES_COMPARE_PORCELAIN")
            .ok()
            .and_then(|s| s.parse::<u64>().ok())
            .unwrap_or(0)
            != 0;
        if porcelain {
            self.all.print_porcelain();
        }
    }
}

fn apportion_subracks(
    lat: &census::MultisetLattice,
    r_tally: &[u8],
    w: f64,
    wo: f64,
    num: &mut [f64],
    den: &mut [f64],
    scnt: &mut [f64],
) {
    const M: usize = 64; // >= any alphabet (MultisetLattice caps num_letters at 64).
    let num_letters = lat.num_letters();
    let mut nz = [(0usize, 0u8); M];
    let mut ndistinct = 0;
    for (t, &c) in r_tally.iter().enumerate() {
        if c > 0 {
            nz[ndistinct] = (t, c);
            ndistinct += 1;
        }
    }
    let mut s_tally = [0u8; M];

    struct Ctx<'a> {
        nz: &'a [(usize, u8)],
        ndistinct: usize,
        num_letters: usize,
        s_tally: &'a mut [u8],
        lat: &'a census::MultisetLattice,
        w: f64,
        wo: f64,
        num: &'a mut [f64],
        den: &'a mut [f64],
        scnt: &'a mut [f64],
    }
    impl Ctx<'_> {
        fn rec(&mut self, i: usize) {
            if i == self.ndistinct {
                let sr = self.lat.rank(&self.s_tally[..self.num_letters]) as usize;
                self.num[sr] += self.wo;
                self.den[sr] += self.w;
                self.scnt[sr] += 1.0;
                return;
            }
            let (t, c) = self.nz[i];
            for k in 0..=c {
                self.s_tally[t] = k;
                self.rec(i + 1);
            }
            self.s_tally[t] = 0;
        }
    }
    Ctx {
        nz: &nz,
        ndistinct,
        num_letters,
        s_tally: &mut s_tally,
        lat,
        w,
        wo,
        num,
        den,
        scnt,
    }
    .rec(0);
}

fn generate_rollout_leaves<N: kwg::Node + Sync + Send, L: kwg::Node + Sync + Send>(
    game_config: game_config::GameConfig,
    kwg: kwg::Kwg<N>,
    arc_klv: std::sync::Arc<klv::Klv<L>>,
    num_games: u64,
    seed: Option<u64>,
) -> error::Returns<()> {
    let t0 = std::time::Instant::now();
    let game_config = std::sync::Arc::new(game_config);
    let alphabet = game_config.alphabet();
    let rack_size = game_config.rack_size() as usize;
    let num_letters = alphabet.len() as usize;
    let lat = census::MultisetLattice::new(num_letters, rack_size);
    let lat_len = lat.len();
    let empty_rank = lat.rank(&vec![0u8; num_letters]) as usize;
    let base_freqs: Vec<u8> = (0..alphabet.len()).map(|t| alphabet.freq(t)).collect();
    let seed = seed.unwrap_or_else(rand::random);
    let num_threads = wolges_threads().max(1).min(num_games.max(1) as usize);
    eprintln!(
        "rollout: seed {seed}, {num_games} games, {num_threads} threads, lattice {lat_len} leaves"
    );

    let cv = env_flag("WOLGES_ROLLOUT_CV", false);
    if cv {
        eprintln!(
            "rollout: baseline-subtraction mode (credit margin - play equity, add prior back)"
        );
    }

    let td_lambda = std::env::var("WOLGES_ROLLOUT_LAMBDA")
        .ok()
        .and_then(|s| s.parse::<f64>().ok())
        .filter(|l| (0.0..=1.0).contains(l));
    if let Some(l) = td_lambda {
        eprintln!("rollout: next-turn-blend mode, strength={l} (forward return, census value)");
    }
    let kwg = std::sync::Arc::new(kwg);
    let next_game = std::sync::atomic::AtomicU64::new(0);

    let shared = std::sync::Mutex::new((
        vec![0f64; lat_len],
        vec![0f64; lat_len],
        vec![0f64; lat_len],
    ));

    std::thread::scope(|s| {
        for _ in 0..num_threads {
            s.spawn(|| {
                let mut rng = rand::rngs::ChaCha20Rng::seed_from_u64(seed);
                let mut move_generator = movegen::KurniaMoveGenerator::new(&game_config);
                let mut game_state = game_state::GameState::new(&game_config);
                let mut final_scores = vec![0i32; game_config.num_players() as usize];
                let mut num_local = vec![0f64; lat_len];
                let mut den_local = vec![0f64; lat_len];
                let mut cnt_local = vec![0f64; lat_len];
                let mut unseen_tally = vec![0u8; num_letters];
                let mut rack_scratch = vec![0u8; num_letters];

                let mut plies: Vec<(u8, f64, f64, i32)> = Vec::new();
                let mut ply_tallies: Vec<Vec<u8>> = Vec::new();
                loop {
                    let g = next_game.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    if g >= num_games {
                        break;
                    }
                    rng.set_stream(g);
                    game_state.reset_and_draw_tiles_double_ended(&game_config, &mut rng);
                    plies.clear();
                    let final_margin = loop {
                        let mover = game_state.turn;

                        rack_scratch.iter_mut().for_each(|x| *x = 0);
                        for &t in game_state.current_player().rack.iter() {
                            rack_scratch[t as usize] += 1;
                        }

                        unseen_tally.clone_from_slice(&base_freqs);
                        for &t in game_state.board_tiles.iter() {
                            if t != 0 {
                                let base = t & !((t as i8) >> 7) as u8;
                                unseen_tally[base as usize] =
                                    unseen_tally[base as usize].saturating_sub(1);
                            }
                        }
                        let mut w = 1.0f64;
                        for (t, &c) in rack_scratch.iter().enumerate() {
                            if c > 0 {
                                w *= n_choose_k(unseen_tally[t] as usize, c as usize) as f64;
                            }
                        }
                        let board_snapshot = movegen::BoardSnapshot {
                            board_tiles: &game_state.board_tiles,
                            game_config: &game_config,
                            kwg: &kwg,
                            klv: &arc_klv,
                        };
                        move_generator.gen_moves_unfiltered(&movegen::GenMovesParams {
                            board_snapshot: &board_snapshot,
                            rack: &game_state.current_player().rack,
                            max_gen: 1,
                            num_exchanges_by_this_player: game_state.current_player().num_exchanges,
                            pass_policy: movegen::PassPolicy::OnlyWhenForced,
                            dynamic_leaves: None,
                        });

                        let e_t = move_generator.plays[0].equity.as_f64();
                        let score_before = game_state.players[mover as usize].score;
                        let play = &move_generator.plays[0].play;
                        game_state.play(&game_config, &mut rng, play).unwrap();

                        let score_t = game_state.players[mover as usize].score - score_before;

                        let ply = plies.len();
                        if ply < ply_tallies.len() {
                            ply_tallies[ply].clone_from(&rack_scratch);
                        } else {
                            ply_tallies.push(rack_scratch.clone());
                        }
                        plies.push((mover, w, e_t, score_t));
                        let end = game_state.check_game_ended(&game_config, &mut final_scores);
                        match end {
                            game_state::CheckGameEnded::PlayedOut
                            | game_state::CheckGameEnded::ZeroScores => {
                                break (final_scores[0] - final_scores[1]) as f64;
                            }
                            game_state::CheckGameEnded::NotEnded => {}
                        }
                        game_state.next_turn();
                    };

                    if let Some(strength) = td_lambda {
                        let scored: f64 = plies
                            .iter()
                            .map(|(m, _, _, s)| if *m == 0 { *s as f64 } else { -(*s as f64) })
                            .sum();
                        let endgame_adj = final_margin - scored;
                        let mut g_next = endgame_adj; // terminal reward after the last play
                        let mut v_next = 0.0f64; // terminal state value
                        for t in (0..plies.len()).rev() {
                            let (mover, w, e_t, score_t) = &plies[t];
                            let r_tally = &ply_tallies[t];
                            let sgn = if *mover == 0 { 1.0 } else { -1.0 };
                            let r = sgn * (*score_t as f64);
                            let g_t = r + (1.0 - strength) * v_next + strength * g_next;
                            apportion_subracks(
                                &lat,
                                r_tally,
                                *w,
                                *w * sgn * g_t, // mover-perspective return
                                &mut num_local,
                                &mut den_local,
                                &mut cnt_local,
                            );
                            g_next = g_t;
                            v_next = sgn * *e_t;
                        }
                    } else {
                        for (r_tally, (mover, w, e_t, _score_t)) in
                            ply_tallies.iter().zip(plies.iter())
                        {
                            let g = if *mover == 0 {
                                final_margin
                            } else {
                                -final_margin
                            };
                            let v = if cv { g - *e_t } else { g };
                            apportion_subracks(
                                &lat,
                                r_tally,
                                *w,
                                *w * v,
                                &mut num_local,
                                &mut den_local,
                                &mut cnt_local,
                            );
                        }
                    }
                }
                let mut guard = shared.lock().unwrap();
                let (gnum, gden, gcnt) = &mut *guard;
                for i in 0..lat_len {
                    gnum[i] += num_local[i];
                    gden[i] += den_local[i];
                    gcnt[i] += cnt_local[i];
                }
            });
        }
    });

    let (num, den, scnt) = shared.into_inner().unwrap();

    let value_mp = |idx: usize| -> f64 {
        if den[idx] > 0.0 {
            num[idx] / den[idx]
        } else {
            0.0
        }
    };
    let baseline = value_mp(empty_rank);

    let shrink_k = std::env::var("WOLGES_ROLLOUT_SHRINK")
        .ok()
        .and_then(|s| s.parse::<f64>().ok())
        .unwrap_or(0.0);
    if shrink_k > 0.0 {
        eprintln!("rollout: shrinking toward prior klv with K={shrink_k}");
    }
    let out_name = claim_output_path(&format!("rollout-leaves-{}.csv", run_stamp()))?;
    let mut tally_buf = vec![0u8; num_letters];
    let mut rows: Vec<(usize, String, f64)> = Vec::new();
    let mut leave_ser = String::new();
    for (idx, &den_val) in den.iter().enumerate() {
        if den_val <= 0.0 {
            continue;
        }
        lat.unrank_into(idx, &mut tally_buf);
        let size: usize = tally_buf.iter().map(|&c| c as usize).sum();
        if size == 0 || size > rack_size {
            continue; // skip the empty (baseline) and over-size leaves.
        }

        let centered_mp = value_mp(idx) - baseline;
        let centered = if cv {
            let prior_mp = arc_klv.leave_value_from_tally(&tally_buf) as f64;
            let trust = if shrink_k > 0.0 {
                scnt[idx] / (scnt[idx] + shrink_k)
            } else {
                1.0
            };
            (prior_mp + trust * centered_mp) / equity::SCALE as f64
        } else if shrink_k > 0.0 {
            let prior_mp = arc_klv.leave_value_from_tally(&tally_buf) as f64;
            let trust = scnt[idx] / (scnt[idx] + shrink_k);
            (prior_mp + trust * (centered_mp - prior_mp)) / equity::SCALE as f64
        } else {
            centered_mp / equity::SCALE as f64
        };
        leave_ser.clear();
        for (t, &c) in tally_buf.iter().enumerate() {
            for _ in 0..c {
                leave_ser.push_str(alphabet.of_rack(t as u8).unwrap());
            }
        }
        rows.push((size, leave_ser.clone(), centered));
    }
    rows.sort_unstable_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.cmp(&b.1)));
    let mut csv_out = csv::Writer::from_path(&out_name)?;
    for (_, leave, value) in &rows {
        csv_out.serialize((leave, value))?;
    }
    csv_out.flush()?;
    eprintln!(
        "rollout: wrote {} leaves to {} in {}s (baseline {:.3} pts)",
        rows.len(),
        out_name,
        t0.elapsed().as_secs(),
        baseline / equity::SCALE as f64,
    );
    Ok(())
}

struct WinpctTables<'a, N: kwg::Node, L: kwg::Node> {
    game_config: &'a game_config::GameConfig,
    kwg: &'a kwg::Kwg<N>,
    arc_klv: &'a klv::Klv<L>,
}

fn winpct_play_game<N: kwg::Node, L: kwg::Node>(
    tables: WinpctTables<'_, N, L>,
    move_generator: &mut movegen::KurniaMoveGenerator,
    game_state: &mut game_state::GameState,
    rng: &mut rand::rngs::ChaCha20Rng,
    snapshots: &mut Vec<(usize, usize, usize, usize, i32)>,
    final_scores: &mut [i32],
) {
    let WinpctTables {
        game_config,
        kwg,
        arc_klv,
    } = tables;
    snapshots.clear();
    loop {
        let mover = game_state.turn as usize;
        let other = 1 - mover;
        let lead = equity::descale_score(game_state.players[mover].score)
            - equity::descale_score(game_state.players[other].score);
        snapshots.push((
            game_state.bag.len(),
            game_state.players[mover].rack.len(),
            game_state.players[other].rack.len(),
            mover,
            lead,
        ));
        let board_snapshot = movegen::BoardSnapshot {
            board_tiles: &game_state.board_tiles,
            game_config,
            kwg,
            klv: arc_klv,
        };
        move_generator.gen_moves_unfiltered(&movegen::GenMovesParams {
            board_snapshot: &board_snapshot,
            rack: &game_state.current_player().rack,
            max_gen: 1,
            num_exchanges_by_this_player: game_state.current_player().num_exchanges,
            pass_policy: movegen::PassPolicy::OnlyWhenForced,
            dynamic_leaves: None,
        });
        let play = &move_generator.plays[0].play;
        game_state.play(game_config, rng, play).unwrap();
        match game_state.check_game_ended(game_config, final_scores) {
            game_state::CheckGameEnded::NotEnded => {}
            _ => break,
        }
        game_state.next_turn();
    }
}

fn generate_winpct_table<N: kwg::Node + Sync + Send, L: kwg::Node + Sync + Send>(
    game_config: game_config::GameConfig,
    kwg: kwg::Kwg<N>,
    arc_klv: std::sync::Arc<klv::Klv<L>>,
    num_games: u64,
    seed: Option<u64>,
) -> error::Returns<()> {
    let t0 = std::time::Instant::now();
    let game_config = std::sync::Arc::new(game_config);
    let seed = seed.unwrap_or_else(rand::random);
    let num_threads = wolges_threads().max(1).min(num_games.max(1) as usize);
    eprintln!("winpct: seed {seed}, {num_games} games, {num_threads} threads");
    let kwg = std::sync::Arc::new(kwg);
    let next_game = std::sync::atomic::AtomicU64::new(0);
    let report_every = 10_000u64;
    let shared = std::sync::Mutex::new(win_pct::WinPctAccumulator::new());

    std::thread::scope(|s| {
        for _ in 0..num_threads {
            s.spawn(|| {
                let mut rng = rand::rngs::ChaCha20Rng::seed_from_u64(seed);
                let mut move_generator = movegen::KurniaMoveGenerator::new(&game_config);
                let mut game_state = game_state::GameState::new(&game_config);
                let mut final_scores = vec![0i32; game_config.num_players() as usize];
                let mut acc = win_pct::WinPctAccumulator::new();

                let mut snapshots = Vec::<(usize, usize, usize, usize, i32)>::new();
                loop {
                    let g = next_game.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    if g >= num_games {
                        break;
                    }
                    rng.set_stream(g);
                    game_state.reset_and_draw_tiles_double_ended(&game_config, &mut rng);
                    winpct_play_game(
                        WinpctTables {
                            game_config: &game_config,
                            kwg: &kwg,
                            arc_klv: &arc_klv,
                        },
                        &mut move_generator,
                        &mut game_state,
                        &mut rng,
                        &mut snapshots,
                        &mut final_scores,
                    );

                    for &(bag, my, opp, mover, lead) in &snapshots {
                        let mover_final = equity::descale_score(final_scores[mover])
                            - equity::descale_score(final_scores[1 - mover]);
                        acc.record(bag, my, opp, lead, mover_final);
                    }
                    if (g + 1).is_multiple_of(report_every) {
                        eprintln!("winpct: {} games", g + 1);
                    }
                }
                shared.lock().unwrap().merge(&acc);
            });
        }
    });

    let acc = shared.into_inner().unwrap();

    let mut out = std::io::BufWriter::new(make_writer("-")?);
    out.write_all(acc.to_csv().as_bytes())?;
    out.flush()?;
    eprintln!("winpct: {num_games} games in {}s", t0.elapsed().as_secs());
    Ok(())
}

fn generate_winpct_eval<N: kwg::Node + Sync + Send, L: kwg::Node + Sync + Send>(
    game_config: game_config::GameConfig,
    kwg: kwg::Kwg<N>,
    arc_klv: std::sync::Arc<klv::Klv<L>>,
    table: win_pct::WinPctTable,
    num_games: u64,
    seed: Option<u64>,
) -> error::Returns<()> {
    let t0 = std::time::Instant::now();
    let game_config = std::sync::Arc::new(game_config);
    let table = std::sync::Arc::new(table);
    let seed = seed.unwrap_or_else(rand::random);
    let num_threads = wolges_threads().max(1).min(num_games.max(1) as usize);
    eprintln!("winpct-eval: seed {seed}, {num_games} games, {num_threads} threads");
    let kwg = std::sync::Arc::new(kwg);
    let next_game = std::sync::atomic::AtomicU64::new(0);

    let ln_ratio = (1.0f64 / 0.9 - 1.0).ln();

    let shared = std::sync::Mutex::new((0.0f64, 0.0f64, 0u64));

    std::thread::scope(|s| {
        for _ in 0..num_threads {
            s.spawn(|| {
                let mut rng = rand::rngs::ChaCha20Rng::seed_from_u64(seed);
                let mut move_generator = movegen::KurniaMoveGenerator::new(&game_config);
                let mut game_state = game_state::GameState::new(&game_config);
                let mut final_scores = vec![0i32; game_config.num_players() as usize];
                let mut snapshots = Vec::<(usize, usize, usize, usize, i32)>::new();
                let (mut bt, mut bs, mut n) = (0.0f64, 0.0f64, 0u64);
                loop {
                    let g = next_game.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    if g >= num_games {
                        break;
                    }
                    rng.set_stream(g);
                    game_state.reset_and_draw_tiles_double_ended(&game_config, &mut rng);
                    winpct_play_game(
                        WinpctTables {
                            game_config: &game_config,
                            kwg: &kwg,
                            arc_klv: &arc_klv,
                        },
                        &mut move_generator,
                        &mut game_state,
                        &mut rng,
                        &mut snapshots,
                        &mut final_scores,
                    );
                    for &(bag, my, opp, mover, lead) in &snapshots {
                        let mover_final = equity::descale_score(final_scores[mover])
                            - equity::descale_score(final_scores[1 - mover]);
                        let result = match mover_final.signum() {
                            1 => 1.0f64,
                            -1 => 0.0,
                            _ => 0.5,
                        };
                        let tab = table.get(lead, bag, my, opp) as f64;

                        let exp_width = -(30.0 + (bag + my + opp) as f64) / ln_ratio;
                        let sig = 1.0 / (1.0 + (-(lead as f64) / exp_width).exp());
                        bt += (tab - result) * (tab - result);
                        bs += (sig - result) * (sig - result);
                        n += 1;
                    }
                }
                let mut acc = shared.lock().unwrap();
                acc.0 += bt;
                acc.1 += bs;
                acc.2 += n;
            });
        }
    });

    let (bt, bs, n) = shared.into_inner().unwrap();
    let d = n.max(1) as f64;
    eprintln!(
        "winpct-eval: {n} samples, brier table={:.5} sigmoid={:.5} (lower better)",
        bt / d,
        bs / d
    );
    eprintln!(
        "winpct-eval: {num_games} games in {}s",
        t0.elapsed().as_secs()
    );
    Ok(())
}

fn compare_leaves<N: kwg::Node + Sync + Send, L: kwg::Node + Sync + Send>(
    game_config: game_config::GameConfig,
    kwg: kwg::Kwg<N>,
    arc_klv0: std::sync::Arc<klv::Klv<L>>,
    arc_klv1: std::sync::Arc<klv::Klv<L>>,
    num_game_pairs: u64,
    seed: Option<u64>,
) -> error::Returns<()> {
    let game_config = std::sync::Arc::new(game_config);
    let kwg = std::sync::Arc::new(kwg);
    let seed = seed.unwrap_or_else(rand::random);
    eprintln!("seed: {seed}");
    let num_threads = wolges_threads();
    let completed_pairs = std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0));
    let reported_secs = std::sync::atomic::AtomicU64::new(0);
    let t0 = std::time::Instant::now();

    let dynamic_leaves_on = std::env::var("WOLGES_DYNAMIC_LEAVES")
        .ok()
        .and_then(|s| s.parse::<u64>().ok())
        .unwrap_or(0)
        != 0;
    let dynamic_min_keep = std::env::var("WOLGES_DYNAMIC_LEAVES_MIN_KEEP")
        .ok()
        .and_then(|s| s.parse::<usize>().ok())
        .unwrap_or(2);

    let dyn_ctx: Option<(census::MultisetLattice, census::AddTable, Vec<i32>)> =
        if dynamic_leaves_on {
            let num_letters = game_config.alphabet().len() as usize;
            let rack_size = game_config.rack_size() as usize;
            let lat = census::MultisetLattice::new(num_letters, rack_size);
            let add = census::AddTable::new_with_threads(&lat, num_threads);
            let mut full_v = vec![0i32; lat.len()];
            census::fill_lattice_leaves(&lat, &mut full_v, |tally| {
                arc_klv0.leave_value_from_tally(tally)
            });
            Some((lat, add, full_v))
        } else {
            None
        };
    let dyn_ref = dyn_ctx
        .as_ref()
        .map(|(lat, add, full_v)| klv::DynamicLeavesRef {
            lat,
            add,
            full_v: full_v.as_slice(),
            min_keep: dynamic_min_keep,
        });
    eprintln!(
        "WOLGES_DYNAMIC_LEAVES={} WOLGES_DYNAMIC_LEAVES_MIN_KEEP={dynamic_min_keep} ({})",
        dynamic_leaves_on as u8,
        if dynamic_leaves_on {
            "dynamic leaves on for the klv0 (player 0) side"
        } else {
            "off, static leaves both sides"
        },
    );

    std::thread::scope(|s| -> error::Returns<()> {
        let mut thread_handles = Vec::new();
        for _ in 0..num_threads {
            let game_config = std::sync::Arc::clone(&game_config);
            let kwg = std::sync::Arc::clone(&kwg);
            let arc_klv0 = std::sync::Arc::clone(&arc_klv0);
            let arc_klv1 = std::sync::Arc::clone(&arc_klv1);
            let completed_pairs = std::sync::Arc::clone(&completed_pairs);
            let reported_secs = &reported_secs;
            thread_handles.push(s.spawn(move || {
                let mut rng = rand::rngs::ChaCha20Rng::seed_from_u64(seed);
                let mut move_generator = movegen::KurniaMoveGenerator::new(&game_config);
                let mut game_state = game_state::GameState::new(&game_config);
                let mut saved_game_state = game_state.clone();
                let mut final_scores = vec![0i32; game_config.num_players() as usize];
                let mut stats = GamePairStats::new();
                let mut first_game_moves: Vec<movegen::Play> = Vec::new();

                loop {
                    let pair_idx =
                        completed_pairs.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    if pair_idx >= num_game_pairs {
                        break;
                    }

                    rng.set_stream(pair_idx);
                    game_state.reset_and_draw_tiles_double_ended(&game_config, &mut rng);
                    saved_game_state.clone_from(&game_state);
                    let saved_rng_state = rng.serialize_state();

                    let mut pair_diverged = false;
                    let mut pair_results =
                        [(0i32, 0i32, 0u32, game_state::CheckGameEnded::NotEnded); 2];

                    for game_in_pair in 0..2u8 {
                        if game_in_pair > 0 {
                            game_state.clone_from(&saved_game_state);
                            rng = rand::rngs::ChaCha20Rng::deserialize_state(&saved_rng_state);
                        }
                        let klv_swapped = game_in_pair != 0;
                        let mut num_turns = 0u32;
                        if !klv_swapped {
                            first_game_moves.clear();
                        }

                        let end_reason = loop {
                            let is_klv0_side = (game_state.turn == 0) != klv_swapped;
                            let board_snapshot = movegen::BoardSnapshot {
                                board_tiles: &game_state.board_tiles,
                                game_config: &game_config,
                                kwg: &kwg,
                                klv: if is_klv0_side { &arc_klv0 } else { &arc_klv1 },
                            };
                            move_generator.gen_moves_unfiltered(&movegen::GenMovesParams {
                                board_snapshot: &board_snapshot,
                                rack: &game_state.current_player().rack,
                                max_gen: 1,
                                num_exchanges_by_this_player: game_state
                                    .current_player()
                                    .num_exchanges,
                                pass_policy: movegen::PassPolicy::OnlyWhenForced,
                                dynamic_leaves: if is_klv0_side { dyn_ref } else { None },
                            });
                            let play = &move_generator.plays[0].play;
                            if klv_swapped {
                                if !pair_diverged
                                    && (num_turns as usize >= first_game_moves.len()
                                        || first_game_moves[num_turns as usize] != *play)
                                {
                                    pair_diverged = true;
                                }
                            } else {
                                first_game_moves.push(play.clone());
                            }
                            game_state.play(&game_config, &mut rng, play).unwrap();
                            num_turns += 1;
                            let end = game_state.check_game_ended(&game_config, &mut final_scores);
                            match end {
                                game_state::CheckGameEnded::PlayedOut
                                | game_state::CheckGameEnded::ZeroScores => break end,
                                game_state::CheckGameEnded::NotEnded => {}
                            }
                            game_state.next_turn();
                        };

                        let (klv0_score, klv1_score) = if klv_swapped {
                            (final_scores[1], final_scores[0])
                        } else {
                            (final_scores[0], final_scores[1])
                        };
                        pair_results[game_in_pair as usize] =
                            (klv0_score, klv1_score, num_turns, end_reason);
                    }

                    if !pair_diverged && pair_results[0].2 != pair_results[1].2 {
                        pair_diverged = true;
                    }
                    for &(klv0_score, klv1_score, num_turns, end_reason) in &pair_results {
                        stats.add_game(
                            klv0_score,
                            klv1_score,
                            num_turns,
                            end_reason,
                            pair_diverged,
                        );
                    }

                    let secs = t0.elapsed().as_secs();
                    let prev = reported_secs.fetch_max(secs, std::sync::atomic::Ordering::Relaxed);
                    if secs > prev {
                        eprintln!("After {}s: {} pairs", secs, pair_idx + 1);
                    }
                }

                stats
            }));
        }

        let mut combined = GamePairStats::new();
        for handle in thread_handles {
            combined.merge(&handle.join().unwrap());
        }

        println!();
        combined.print();

        Ok(())
    })
}

fn sim_compare_seat_config(prefix: &str) -> simmer::SimmerConfig {
    let mut config = simmer::SimmerConfig::default();
    if let Some(descale) = std::env::var(format!("{prefix}DESCALE"))
        .ok()
        .and_then(|s| s.parse::<u64>().ok())
    {
        config.descale = descale != 0;
    }

    if let Some("table") = std::env::var(format!("{prefix}WINPROB")).ok().as_deref() {
        config.win_prob_source = simmer::WinProbSource::Table;
    }
    config
}

fn win_prob_source_name(source: simmer::WinProbSource) -> &'static str {
    match source {
        simmer::WinProbSource::Sigmoid => "sigmoid",
        simmer::WinProbSource::Table => "table",
    }
}

fn sim_compare_allocator(prefix: &str) -> move_picker::Allocator {
    match std::env::var(format!("{prefix}ALLOCATOR")).ok().as_deref() {
        Some("adaptive") => move_picker::Allocator::Adaptive,
        _ => move_picker::Allocator::RoundRobin,
    }
}

fn allocator_name(allocator: move_picker::Allocator) -> &'static str {
    match allocator {
        move_picker::Allocator::RoundRobin => "round-robin",
        move_picker::Allocator::Adaptive => "adaptive",
    }
}

fn sim_compare_stop_rule(prefix: &str) -> move_picker::StopRule {
    match std::env::var(format!("{prefix}STOP")).ok().as_deref() {
        Some("confidence") => move_picker::StopRule::Confidence,
        _ => move_picker::StopRule::FixedCap,
    }
}

fn sim_compare_stop_delta(prefix: &str) -> Option<f64> {
    std::env::var(format!("{prefix}STOP_DELTA"))
        .ok()
        .and_then(|s| s.parse::<f64>().ok())
}

fn stop_rule_name(stop_rule: move_picker::StopRule) -> &'static str {
    match stop_rule {
        move_picker::StopRule::FixedCap => "fixed-cap",
        move_picker::StopRule::Confidence => "confidence",
    }
}

fn sim_compare<N: kwg::Node + Sync + Send, L: kwg::Node + Sync + Send>(
    game_config: game_config::GameConfig,
    kwg: kwg::Kwg<N>,
    arc_klv: std::sync::Arc<klv::Klv<L>>,
    num_game_pairs: u64,
    seed: Option<u64>,
) -> error::Returns<()> {
    let game_config = std::sync::Arc::new(game_config);
    let kwg = std::sync::Arc::new(kwg);
    let seed = seed.unwrap_or_else(rand::random);
    eprintln!("seed: {seed}");
    let num_threads = wolges_threads();
    let completed_pairs = std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0));
    let reported_secs = std::sync::atomic::AtomicU64::new(0);
    let t0 = std::time::Instant::now();

    let num_sim_iters = std::env::var("WOLGES_SIM_ITERS")
        .ok()
        .and_then(|s| s.parse::<u64>().ok())
        .unwrap_or(1_000);

    let sim_driver_threads = std::env::var("WOLGES_SIM_DRIVER_THREADS")
        .ok()
        .and_then(|s| s.parse::<usize>().ok())
        .unwrap_or(1);
    let config_p0 = sim_compare_seat_config("WOLGES_SIM_P0_");
    let config_p1 = sim_compare_seat_config("WOLGES_SIM_P1_");
    let allocator_p0 = sim_compare_allocator("WOLGES_SIM_P0_");
    let allocator_p1 = sim_compare_allocator("WOLGES_SIM_P1_");
    let stop_p0 = sim_compare_stop_rule("WOLGES_SIM_P0_");
    let stop_p1 = sim_compare_stop_rule("WOLGES_SIM_P1_");
    let stop_delta_p0 = sim_compare_stop_delta("WOLGES_SIM_P0_");
    let stop_delta_p1 = sim_compare_stop_delta("WOLGES_SIM_P1_");

    let winpct_table: Option<win_pct::WinPctTable> = match std::env::var("WOLGES_SIM_WINPCT_TABLE")
    {
        Ok(path) => Some(win_pct::WinPctTable::from_csv(&std::fs::read_to_string(
            &path,
        )?)?),
        Err(_) => None,
    };
    let winpct_table_ref = winpct_table.as_ref();
    eprintln!(
        "WOLGES_SIM_ITERS={num_sim_iters} winpct_table={} P0.descale={} P0.alloc={} P0.stop={} P0.winprob={} P1.descale={} P1.alloc={} P1.stop={} P1.winprob={}",
        winpct_table_ref.is_some() as u8,
        config_p0.descale as u8,
        allocator_name(allocator_p0),
        stop_rule_name(stop_p0),
        win_prob_source_name(config_p0.win_prob_source),
        config_p1.descale as u8,
        allocator_name(allocator_p1),
        stop_rule_name(stop_p1),
        win_prob_source_name(config_p1.win_prob_source),
    );

    std::thread::scope(|s| -> error::Returns<()> {
        let mut thread_handles = Vec::new();
        for _ in 0..num_threads {
            let game_config = std::sync::Arc::clone(&game_config);
            let kwg = std::sync::Arc::clone(&kwg);
            let arc_klv = std::sync::Arc::clone(&arc_klv);
            let completed_pairs = std::sync::Arc::clone(&completed_pairs);
            let reported_secs = &reported_secs;
            thread_handles.push(s.spawn(move || {
                let mut rng = rand::rngs::ChaCha20Rng::seed_from_u64(seed);
                let mut move_generator = movegen::KurniaMoveGenerator::new(&game_config);
                let mut filtered_movegen = move_filter::GenMoves::Unfiltered;

                let mut driver_p0 = move_picker::MovePicker::Simmer(move_picker::Simmer::new(
                    &game_config,
                    &kwg,
                    &arc_klv,
                ));
                if let move_picker::MovePicker::Simmer(driver) = &mut driver_p0 {
                    driver.set_config(config_p0);
                    driver.set_win_pct_table(winpct_table_ref);
                    driver.set_num_sim_iters(num_sim_iters);
                    driver.set_allocator(allocator_p0);
                    driver.set_stop_rule(stop_p0);
                    driver.set_sim_threads(sim_driver_threads);
                    if let Some(delta) = stop_delta_p0 {
                        driver.set_stop_delta(delta);
                    }
                }
                let mut driver_p1 = move_picker::MovePicker::Simmer(move_picker::Simmer::new(
                    &game_config,
                    &kwg,
                    &arc_klv,
                ));
                if let move_picker::MovePicker::Simmer(driver) = &mut driver_p1 {
                    driver.set_config(config_p1);
                    driver.set_win_pct_table(winpct_table_ref);
                    driver.set_num_sim_iters(num_sim_iters);
                    driver.set_allocator(allocator_p1);
                    driver.set_stop_rule(stop_p1);
                    driver.set_sim_threads(sim_driver_threads);
                    if let Some(delta) = stop_delta_p1 {
                        driver.set_stop_delta(delta);
                    }
                }
                let mut game_state = game_state::GameState::new(&game_config);
                let mut saved_game_state = game_state.clone();
                let mut final_scores = vec![0i32; game_config.num_players() as usize];
                let mut stats = GamePairStats::new();
                let mut first_game_moves: Vec<movegen::Play> = Vec::new();

                loop {
                    let pair_idx =
                        completed_pairs.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    if pair_idx >= num_game_pairs {
                        break;
                    }

                    rng.set_stream(pair_idx);
                    game_state.reset_and_draw_tiles_double_ended(&game_config, &mut rng);
                    saved_game_state.clone_from(&game_state);
                    let saved_rng_state = rng.serialize_state();

                    let mut pair_diverged = false;
                    let mut pair_results =
                        [(0i32, 0i32, 0u32, game_state::CheckGameEnded::NotEnded); 2];

                    for game_in_pair in 0..2u8 {
                        if game_in_pair > 0 {
                            game_state.clone_from(&saved_game_state);
                            rng = rand::rngs::ChaCha20Rng::deserialize_state(&saved_rng_state);
                        }
                        let seat_swapped = game_in_pair != 0;
                        let mut num_turns = 0u32;
                        if !seat_swapped {
                            first_game_moves.clear();
                        }

                        let end_reason = loop {
                            let is_p0_seat = (game_state.turn == 0) != seat_swapped;
                            let board_snapshot = movegen::BoardSnapshot {
                                board_tiles: &game_state.board_tiles,
                                game_config: &game_config,
                                kwg: &kwg,
                                klv: &arc_klv,
                            };
                            let driver = if is_p0_seat {
                                &mut driver_p0
                            } else {
                                &mut driver_p1
                            };

                            if let move_picker::MovePicker::Simmer(simmer_driver) = &mut *driver {
                                simmer_driver.reseed(census_mix64(
                                    seed.wrapping_add(census_mix64(
                                        pair_idx
                                            .wrapping_mul(0x9E37_79B9_7F4A_7C15)
                                            .wrapping_add((game_in_pair as u64).wrapping_shl(40))
                                            .wrapping_add(num_turns as u64),
                                    )),
                                ));
                            }
                            driver.pick_a_move(
                                &mut filtered_movegen,
                                &mut move_generator,
                                &board_snapshot,
                                &game_state,
                                &game_state.current_player().rack,
                            );
                            let play = &move_generator.plays[0].play;
                            if seat_swapped {
                                if !pair_diverged
                                    && (num_turns as usize >= first_game_moves.len()
                                        || first_game_moves[num_turns as usize] != *play)
                                {
                                    pair_diverged = true;
                                }
                            } else {
                                first_game_moves.push(play.clone());
                            }
                            game_state.play(&game_config, &mut rng, play).unwrap();
                            num_turns += 1;
                            let end = game_state.check_game_ended(&game_config, &mut final_scores);
                            match end {
                                game_state::CheckGameEnded::PlayedOut
                                | game_state::CheckGameEnded::ZeroScores => break end,
                                game_state::CheckGameEnded::NotEnded => {}
                            }
                            game_state.next_turn();
                        };

                        let (p0_score, p1_score) = if seat_swapped {
                            (final_scores[1], final_scores[0])
                        } else {
                            (final_scores[0], final_scores[1])
                        };
                        pair_results[game_in_pair as usize] =
                            (p0_score, p1_score, num_turns, end_reason);
                    }
                    if !pair_diverged && pair_results[0].2 != pair_results[1].2 {
                        pair_diverged = true;
                    }
                    for &(p0_score, p1_score, num_turns, end_reason) in &pair_results {
                        stats.add_game(p0_score, p1_score, num_turns, end_reason, pair_diverged);
                    }

                    let secs = t0.elapsed().as_secs();
                    let prev = reported_secs.fetch_max(secs, std::sync::atomic::Ordering::Relaxed);
                    if secs > prev {
                        eprintln!("After {}s: {} pairs", secs, pair_idx + 1);
                    }
                }

                stats
            }));
        }

        let mut combined = GamePairStats::new();
        for handle in thread_handles {
            combined.merge(&handle.join().unwrap());
        }

        println!();
        combined.print();

        Ok(())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const SHEET_PLANS: &[(&[u64], &[usize], usize)] = &[
        (&[400, 100, 300], &[300, 300, 0], 300),
        (&[400, 300, 100], &[300, 100, 0], 300),
        (&[256, 256, 256, 256], &[256, 256, 256, 0], 256),
        (&[256, 512, 1024], &[1024, 1024, 0], 512),
        (&[256, 256, 256, 2048], &[2048, 2048, 2048, 0], 256),
        (&[200, 1000, 400, 300], &[1000, 400, 300, 0], 400),
        (&[200, 1000, 400, 700, 350], &[1000, 700, 700, 350, 0], 700),
        (&[256], &[0], 0),
    ];

    #[test]
    fn census_sheet_reuse_plan_looks_past_the_next_generation() {
        for &(counts, want_live, want_len) in SHEET_PLANS {
            let (live_after, cache_len) = census_sheet_reuse_plan(counts);
            assert_eq!(live_after, want_live, "live_after for {counts:?}");
            assert_eq!(cache_len, want_len, "cache_len for {counts:?}");
        }
    }

    #[test]
    fn census_sheet_reuse_plan_never_reads_an_uncached_slot() {
        for &(counts, _, want_len) in SHEET_PLANS {
            let (live_after, cache_len) = census_sheet_reuse_plan(counts);
            let mut cached = std::collections::HashSet::<usize>::new();
            let mut prior_max = 0usize;
            let mut high_water = 0usize;
            for (g, &n) in counts.iter().enumerate() {
                for b in 0..n as usize {
                    if b < prior_max {
                        assert!(
                            cached.contains(&b),
                            "{counts:?} gen {g} reuses slot {b} but it was never cached"
                        );
                    } else if b < live_after[g] {
                        cached.insert(b);
                        high_water = high_water.max(b + 1);
                    }
                }
                cached.retain(|&b| b < live_after[g]);
                prior_max = prior_max.max(n as usize);
            }
            assert!(
                high_water <= cache_len,
                "{counts:?} cached slot {high_water} past cache_len {cache_len}"
            );

            assert_eq!(high_water, want_len, "cache_len not tight for {counts:?}");
            assert!(
                cached.is_empty(),
                "{counts:?} kept sheets after the last gen"
            );
        }
    }

    #[test]
    fn pooling_keeps_value_square_and_count_together() {
        let mut m = fash::MyHashMap::<bites::Bites, Cumulate>::default();
        pool_one(&mut m, &b"\x01"[..], 3.0);
        pool_one(&mut m, &b"\x01"[..], 4.0);
        let a = m.get(&b"\x01"[..]).unwrap();
        assert_eq!(a.count, 2);
        assert!((a.equity - 7.0).abs() < 1e-9);

        assert!((a.sumsq - 25.0).abs() < 1e-9);
    }

    #[test]
    fn merging_thread_maps_keeps_every_square() {
        let mut dst = fash::MyHashMap::<bites::Bites, Cumulate>::default();
        pool_one(&mut dst, &b"\x01"[..], 3.0);
        let mut src = fash::MyHashMap::<bites::Bites, Cumulate>::default();
        pool_one(&mut src, &b"\x01"[..], 4.0);
        pool_one(&mut src, &b"\x02"[..], 5.0);
        merge_rack_map(&mut dst, &mut src);
        let a = dst.get(&b"\x01"[..]).unwrap();
        assert_eq!(a.count, 2);
        assert!((a.equity - 7.0).abs() < 1e-9);
        assert!((a.sumsq - 25.0).abs() < 1e-9, "the merge dropped a square");

        let b = dst.get(&b"\x02"[..]).unwrap();
        assert_eq!(b.count, 1);
        assert!((b.sumsq - 25.0).abs() < 1e-9);
        assert!(src.is_empty(), "merge_rack_map must drain the source");
    }

    #[test]
    fn pooled_spread_cannot_undercut_its_mean() {
        let mut m = fash::MyHashMap::<bites::Bites, Cumulate>::default();
        for v in [12.5f64, -3.0, 40.0, 0.0, 7.25] {
            pool_one(&mut m, &b"\x01"[..], v);
        }
        let a = m.get(&b"\x01"[..]).unwrap();
        assert!(a.sumsq >= a.equity.powi(2) / a.count as f64 - 1e-9);
    }

    #[test]
    fn parse_board_counts_expands_repeats() {
        assert_eq!(parse_board_counts("256").unwrap(), vec![256]);
        assert_eq!(
            parse_board_counts("100,200,200,300,500,500,500").unwrap(),
            vec![100, 200, 200, 300, 500, 500, 500]
        );

        assert_eq!(
            parse_board_counts("100,2x200,300,3x500").unwrap(),
            vec![100, 200, 200, 300, 500, 500, 500]
        );
        assert_eq!(
            parse_board_counts("4x256").unwrap(),
            vec![256, 256, 256, 256]
        );

        assert_eq!(
            parse_board_counts("100, 2x200").unwrap(),
            vec![100, 200, 200]
        );
        assert!(parse_board_counts("").is_err());
        assert!(parse_board_counts("abc").is_err());
        assert!(parse_board_counts("1,,2").is_err());
    }

    #[test]
    fn per_rack_decompose_weights_by_mean_not_count() {
        let fv = Cumulate {
            equity: 10.0,
            count: 2,
            sumsq: 0.0,
        };

        let (eq, cnt) = decompose_contribution(&fv, 3, false);
        assert!((eq - 30.0).abs() < 1e-9); // 10 * 3
        assert_eq!(cnt, 6); // 2 * 3

        let (eq, cnt) = decompose_contribution(&fv, 3, true);
        assert!((eq - 15.0).abs() < 1e-9); // (10/2) * 3
        assert_eq!(cnt, 3); // w only
    }

    #[test]
    fn rare_pools_by_count_into_subrack_map() {
        let mut m = fash::MyHashMap::<bites::Bites, Cumulate>::default();
        m.insert(
            b"\x01"[..].into(),
            Cumulate {
                equity: 10.0,
                count: 2,
                sumsq: 50.0,
            },
        ); // full-rack A, sum10 n2
        pool_rare_one(&mut m, &b"\x01"[..], 5.0, 3, 9.0); // rare A, sum5 n3
        let a = m.get(&b"\x01"[..]).unwrap();
        assert_eq!(a.count, 5);
        assert!((a.equity - 15.0).abs() < 1e-9); // mean 15/5 = 3.0

        assert!((a.sumsq - 59.0).abs() < 1e-9);
    }
}
