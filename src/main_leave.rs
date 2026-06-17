// Copyright (C) 2020-2026 Andy Kurnia.

use rand::prelude::*;
use std::fmt::Write;
use std::io::Write as _;
use std::str::FromStr;
use wolges::{
    alphabet, bites, build, census, display, equity, error, fash, game_config, game_state, klv,
    kwg, move_filter, move_picker, movegen, play_scorer, prob, stats,
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

// A per-run stamp shared by every file one run writes, so a run's outputs stay
// grouped under one id. It is the wall-clock time at 1/65536-second resolution
// as a fixed-width hex string: 32 bits of whole seconds (good until the year
// 2106) followed by 16 bits of sub-second fraction. The sub-second part makes
// two runs collide only if they start within about fifteen microseconds of each
// other.
fn run_stamp() -> String {
    let d = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    let ticks = (d.as_secs() << 16) | (d.subsec_nanos() as u64 * 65536 / 1_000_000_000);
    format!("{ticks:012x}")
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
                // english-gilles CSW24.kwg leave0.klv leave1.klv 1000000 [min_samples] [seed]
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
                // english-census CSW24.kwg leave0.klv leave1.klv <boards> [seed]
                // <boards> = comma-separated per-generation board counts, each
                // `N` or `KxN` (K gens of N), e.g. 100,2x200,300,3x500; the gen
                // count is the expanded length. seed is optional (arg6).
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
                // english-compare CSW24.kwg klv0.klv2 klv1.klv2 10000 [seed]
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
            "-rollout" => {
                // english-rollout CSW24.kwg policy.klv2 num_games [seed]
                // whole-game Monte-Carlo rollout leaves (a measured negative).
                // policy.klv2 = the leaves both players use to pick plays ("-" = null).
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
            "-generate-no-smooth" => {
                generate_leaves::<_, _, false, false>(
                    make_game_config(),
                    csv::ReaderBuilder::new()
                        .has_headers(false)
                        .from_reader(make_reader(&args[2])?),
                    csv::Writer::from_writer(make_writer(&args[3])?),
                    args.get(4).map(|x| x.as_str()),
                )?;
                Ok(true)
            }
            "-generate" => {
                generate_leaves::<_, _, true, false>(
                    make_game_config(),
                    csv::ReaderBuilder::new()
                        .has_headers(false)
                        .from_reader(make_reader(&args[2])?),
                    csv::Writer::from_writer(make_writer(&args[3])?),
                    args.get(4).map(|x| x.as_str()),
                )?;
                Ok(true)
            }
            "-generate-full-no-smooth" => {
                generate_leaves::<_, _, false, true>(
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
                generate_leaves::<_, _, true, true>(
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
  english-generate-no-smooth summary.csv leaves.csv [rare.csv]
    generate leaves (no smoothing) up to rack_size - 1
  english-generate summary.csv leaves.csv [rare.csv]
    generate leaves (with smoothing) up to rack_size - 1
  english-generate-full-no-smooth summary.csv leaves.csv [rare.csv]
    generate leaves (no smoothing) up to rack_size
  english-generate-full summary.csv leaves.csv [rare.csv]
    generate leaves (with smoothing) up to rack_size
    [rare.csv] on any -generate adds direct coverage for undersampled subracks
  english-playability CSW24.kwg leave.klv 1000000 [seed]
    autoplay (not saved) and record prorated found best words (at the end)
    (run fewer number of games and use resummarize to merge to mitigate risks)
    seed is optional; prints auto-generated seed to stderr if not provided.
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
    each pair: same tile draw, alternating starting player.
    p0 uses klv0, p1 uses klv1 for move selection (static play, max=1).
    reports wins/losses/draws, score stats, divergent games, and
    confidence that one set of leaves is better.
    if klv is \"-\" or omitted, uses no leave.
    number of game pairs is optional (default 10000).
    seed is optional; prints auto-generated seed to stderr if not provided.
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

// read a value from an env var, or fall back to a default. one helper so every
// algorithm reads its settings the same way -- no recompile to tune a run.
fn env_parse<T: std::str::FromStr>(name: &str, default: T) -> T {
    std::env::var(name)
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(default)
}

// a boolean env var: unset (or invalid) falls back to `default`. one bool
// convention everywhere -- any nonzero integer is true, so `NAME=1` turns it on.
fn env_flag(name: &str, default: bool) -> bool {
    env_parse::<u64>(name, default as u64) != 0
}

// worker-thread count for every parallel run. honor WOLGES_THREADS if set (and
// parsable), else default to the machine's core count. reading it through one
// helper keeps the override consistent across every threaded algorithm.
fn wolges_threads() -> usize {
    std::env::var("WOLGES_THREADS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or_else(num_cpus::get)
}

// how WOLGES_APPORTION splits a sampled board's value back onto leaves:
// full-rack (the default) or the opt-in entering path. parse it once
// into a typed value so an unknown setting fails loud instead of silently
// falling back to full-rack.
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

    // impossible_ok: coverage knob for undersampled-subrack remediation.
    //
    // To sample subrack S we must draw it, so this tests S's tiles are still
    // off the board -- "impossible" here is about the DRAW, a different test
    // from the rack knob's place-time one, run earlier (before movegen, not
    // after). The knob only matters when S is not drawable (a tile S needs,
    // say Q, is already on the board).
    //
    // When drawable (either setting): build a rack = S + filler from the
    // unseen pool, gen its best play, and record the equity keyed by S alone
    // (never apportioned to S's subracks or the filler -- that apportioning
    // is what polluted common leaves at rack level).
    //
    // Off, not drawable: skip outright -- no rack is built, no movegen
    // runs, this board yields no S sample.
    //
    // On (default), not drawable: build S with a phantom tile and record
    // anyway, so every undersampled subrack reaches min_samples_per_rack
    // (no smoothing).
    let impossible_ok = env_flag("WOLGES_IMPOSSIBLE_OK", true);

    // entering-leave attribution. Off (default): record each turn's best-play
    // equity keyed by the drawn rack R (full-rack attribution). On: record it
    // keyed by the entering leave L = the tiles the player walked into the turn
    // holding (= last turn's kept leave). -generate decomposes either key to its
    // subracks identically; only the recording key changes, play is unchanged.
    let entering = match wolges_apportion()? {
        Apportion::Entering => true,
        Apportion::FullRack => false,
    };

    // WOLGES_OPPDENIAL_LEAVE (experiment, strength): the per-kept-leave opponent-denial term. It
    // does NOT fold into the recorded equity -- the summary stays byte-identical whether
    // it is on or off. Instead the sampler averages each sampled board's per-letter
    // opponent-denial marginals (how much the opponent's expected best play drops when
    // one tile of a letter leaves the unseen pool) over the boards, writes them to the
    // oppdenial-leave-marginal.csv sidecar, and the decompose (generate_leaves) adds
    // oppdenial_leave * sum_t S[t] * avg_marginal[t] to each kept subrack S's final leave. Off
    // (oppdenial_leave == 0.0) -> no marginals, no sidecar, byte-identical.
    let oppdenial_leave = env_parse::<f64>("WOLGES_OPPDENIAL_LEAVE", 0.0);
    // WOLGES_OPPDENIAL_RACK (experiment, strength): credit for the opponent's
    // next turn, ported from the census's WOLGES_OPPDENIAL_RACK. Where
    // oppdenial_leave averages each sampled board's opponent-denial marginals
    // into the sidecar and leaves the recorded equity untouched,
    // oppdenial_rack instead folds each recorded full rack R's own
    // opponent-denial straight into R's equity: recorded_equity(R) +=
    // oppdenial_rack * sum_t R[t] * marginal[t], where marginal[t] is how much
    // the opponent's expected best play drops when one tile of letter t leaves
    // this board's unseen pool (holding those tiles keeps them from the
    // opponent's draw). It reuses the very same per-board marginals
    // oppdenial_leave builds, so enabling it only widens the gates below
    // to build the marginals and adds the fold at the record sites. Off
    // (oppdenial_rack == 0.0) -> byte-identical.
    let oppdenial_rack = env_parse::<f64>("WOLGES_OPPDENIAL_RACK", 0.0);
    // WOLGES_OPPDENIAL_EXACT (experiment, strength): the joint opponent-and-me
    // next-turn term. Where oppdenial_rack folds a per-tile opponent-denial
    // marginal into the equity, oppdenial_exact folds the full drawn rack R's
    // exact joint term: recorded_equity(R) -= oppdenial_exact *
    // oppdenial_exact_term[R], where oppdenial_exact_term[R] = opp_value(U-R)
    // - my_next_value(K*, U-R) -- the opponent's expected best play drawing a
    // fresh rack from the R-depleted unseen pool U-R, minus my own expected
    // best next play after keeping R's argmax leave K* and refilling from U-R
    // (K* from best_equity_argmax_table, the term from
    // census::opp_me2_per_rack). O(drawable^2) per board, so it is gated to
    // boards whose unseen pool has at most oppdenial_exact_pool_max tiles;
    // larger boards skip it. Off (oppdenial_exact == 0.0) -> the
    // oppdenial_exact scratch, argmax table and per-rack pass never run,
    // byte-identical.
    let oppdenial_exact = env_parse::<f64>("WOLGES_OPPDENIAL_EXACT", 0.0);
    let oppdenial_exact_pool_max = env_usize("WOLGES_OPPDENIAL_EXACT_POOL_MAX", 32);
    let opp_on = oppdenial_leave != 0.0 || oppdenial_rack != 0.0 || oppdenial_exact != 0.0;
    // Build the census lattice, its add-table, and the millipoint leave-value array ONCE
    // (only when an opponent-denial term is on, so the default path allocates nothing and stays
    // byte-identical), shared read-only across the sampling threads exactly as the census
    // driver builds and shares them. The leave array is klv0's leave value for every
    // lattice multiset -- the census uses a single leave table for the marginals, and a
    // standard autoplay run has klv0 == klv1, so this is exact.
    let opp_ctx: Option<(census::MultisetLattice, census::AddTable, Vec<i32>)> = if opp_on {
        let num_letters = game_config.alphabet().len() as usize;
        let rack_size = game_config.rack_size() as usize;
        let lat = census::MultisetLattice::new(num_letters, rack_size);
        let add_table = census::AddTable::new(&lat);
        let mut leave = vec![0i32; lat.len()];
        let mut tally = vec![0u8; num_letters];
        for (idx, slot) in leave.iter_mut().enumerate() {
            lat.unrank_into(idx, &mut tally);
            *slot = arc_klv0.leave_value_from_tally(&tally);
        }
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
    let num_processed_games = std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0));

    let epoch_secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let run_identifier = std::sync::Arc::new(format!("log-{epoch_secs:08x}"));
    eprintln!("logging to {run_identifier}");
    let mut csv_log = if WRITE_LOGS {
        Some(csv::Writer::from_path(run_identifier.to_string())?)
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
    let mut csv_game = csv::Writer::from_path(format!("games-{run_identifier}"))?;
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
    // rare (direct) samples, keyed by target subrack S.
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
        // now holds undersampled subracks S (not full racks), recomputed per
        // generation by the subrack-level decomposition of full_rack_map.
        undersampled_racks: Vec<bites::Bites>,
        // generation for which undersampled_racks was last recomputed. starts at
        // u64::MAX ("not yet computed"); the 0->1 enumeration sets it to 0 and each
        // periodic recompute advances it, so the costly subrack decomposition runs once
        // per generation rather than once per thread.
        undersampled_generation: u64,
        undersampling_comment: String,
        tick_periods: move_picker::Periods,
        // WOLGES_OPPDENIAL_LEAVE: per-letter sum of each sampled board's opponent-denial marginals
        // and the board count, merged from the threads; the sidecar writes sum / boards.
        // Empty / 0 unless oppdenial_leave is on.
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
                // final_scores in whole points for the game log: the raw scores
                // are premultiplied millipoints, so descale at this boundary.
                // reused per game like final_scores.
                let mut final_scores_pts = vec![0; game_config.num_players() as usize];
                let mut num_bingos = vec![0; game_config.num_players() as usize];
                let mut num_turns = vec![0; game_config.num_players() as usize];
                let mut num_moves;
                let mut num_batched_games_here = 0;
                let mut batched_csv_log = csv::Writer::from_writer(Vec::new());
                let mut batched_csv_game = csv::Writer::from_writer(Vec::new());
                let mut thread_full_rack_map = fash::MyHashMap::<bites::Bites, Cumulate>::default();
                // entering-leave attribution: each player's last kept leave (the
                // tiles remaining after their action, before drawing replacements),
                // which is what they walk into their next turn holding. None until
                // the player has acted this game (so their first turn is skipped).
                // Reset per game. aft_rack_entering is the reused scratch leftover.
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
                // rare (direct) samples, keyed by the target subrack S
                // and attributed to S only (never decomposed). kept separate from the
                // full-rack map so the full-rack mean and per-subrack decomposition are
                // not skewed by the forced rare samples' equity.
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
                // flat unseen pool (tile ids with multiplicity) and the full rack
                // assembled from the target subrack S plus filler, for rare
                // sampling. hoisted once; reused per rare sample.
                let mut unseen_pool = Vec::<u8>::new();
                let mut sample_rack_buf = if SUMMARIZE && min_samples_per_rack != 0 {
                    Vec::with_capacity(game_config.rack_size() as usize)
                } else {
                    Vec::new()
                };
                let mut undersampled_thread_racks = Vec::<bites::Bites>::new();
                // WOLGES_OPPDENIAL_LEAVE per-thread scratch, reused across turns (allocated only
                // when an opponent-denial term is on, else empty -> the per-board block is skipped
                // and the path is byte-identical). opp_sheet and opp_best are
                // lattice-length (the STEP-1 play-value sheet and best_equity over the
                // full-rack block); opp_marginal holds this board's per-letter
                // opponent-denial marginals; opp_base_freqs and opp_unseen build the
                // board's unseen pool (autoplay only builds the min-samples base_freqs, so
                // this keeps its own); opp_movegen_rack and opp_blank_deltas are
                // build_sheet_spell_once's own scratch.
                let opp_num_letters = game_config.alphabet().len() as usize;
                let mut opp_sheet: Vec<i32> = Vec::new();
                let mut opp_best: Vec<i32> = Vec::new();
                let mut opp_marginal: Vec<f64> = Vec::new();
                let mut opp_base_freqs: Vec<u8> = Vec::new();
                let mut opp_unseen: Vec<u8> = Vec::new();
                let mut opp_movegen_rack: Vec<u8> = Vec::new();
                let mut opp_blank_deltas: Vec<(u8, i32)> = Vec::new();
                // WOLGES_OPPDENIAL_EXACT scratch (allocated only when
                // oppdenial_exact is on): each drawable full rack's argmax
                // kept leave K* (idx + size) from best_equity_argmax_table
                // and its joint opponent term from opp_me2_per_rack, both
                // indexed by the rack's lattice rank.
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
                // WOLGES_OPPDENIAL_LEAVE per-thread accumulators: the sum of each recorded turn's
                // board opponent-denial marginals and the turn count, merged into
                // MutexedStuffs at the thread's end. Empty / 0 unless oppdenial_leave is on.
                let mut oppdenial_leave_sum_marg: Vec<f64> = if oppdenial_leave != 0.0 {
                    vec![0f64; opp_num_letters]
                } else {
                    Vec::new()
                };
                let mut oppdenial_leave_boards = 0u64;
                // subrack length covered by forcing. matches `-generate`'s default
                // (IS_FULL_RACK == false) decomposition, so every rare S key is a
                // subrack `-generate` will enumerate. rare rows therefore pool cleanly
                // into subrack_map without ever keying the empty (mean) subrack.
                // `-generate-full`'s length-`rack_size` subracks are intentionally not
                // force-covered: autoplay racks are always full racks, so those subracks
                // get direct full-rack samples and need no decomposition coverage.
                let leave_size = if SUMMARIZE && min_samples_per_rack != 0 {
                    game_config.rack_size() - 1
                } else {
                    0
                };
                // scratch for the subrack-level undersampled recompute (a decomposition
                // of the full-rack map into pooled per-subrack counts). hoisted once;
                // empty / unused when no remediation is requested.
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
                            // first time this thread transitions past the first num_games games.
                            {
                                let mut mutex_guard = mutexed_stuffs.lock().unwrap();
                                merge_rack_map(
                                    &mut mutex_guard.full_rack_map,
                                    &mut thread_full_rack_map,
                                );
                                // symmetry: rare map is still empty here (forcing has
                                // not begun), but merge it too so every merge site is
                                // uniform and robust to future reordering.
                                merge_rack_map(
                                    &mut mutex_guard.rare_subrack_map,
                                    &mut thread_rare_subrack_map,
                                );
                            }
                            undersampling_remediation_submission
                                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                            // wait until all threads rendezvous here.
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
                                    // this thread is responsible to compute generation 0
                                    // of the undersampled subrack set. decompose the
                                    // full-rack map into pooled per-subrack counts (mirroring
                                    // `-generate`) and mark subracks below min_samples.
                                    // forcing has not begun, so the rare map is empty.
                                    {
                                        let mut mutex_guard = mutexed_stuffs.lock().unwrap();
                                        // swap both maps out of the guard so the helper
                                        // can read them while it writes undersampled_racks
                                        // (disjoint, but the guard deref cannot be split).
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
                                                " (need to force {num_undersampled} subracks over {deficit} moves)"
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
                                    // another thread is contemporaneously iterating the possible racks.
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
                            // publish this thread's full-rack and rare samples so the
                            // recompute sees the latest pooled counts.
                            merge_rack_map(
                                &mut mutex_guard.full_rack_map,
                                &mut thread_full_rack_map,
                            );
                            merge_rack_map(
                                &mut mutex_guard.rare_subrack_map,
                                &mut thread_rare_subrack_map,
                            );
                            // recompute the undersampled subrack set at most once per
                            // generation: the subrack decomposition is as costly as
                            // `-generate`, so the first thread to notice a new
                            // generation rebuilds and the rest reuse the shared result.
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
                                        " (need to force {num_undersampled} subracks over {deficit} moves)"
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
                                // really done. this thread need not play more games.
                                num_processed_games
                                    .fetch_sub(1, std::sync::atomic::Ordering::Relaxed);
                                break;
                            }

                            // if there are too few unique racks, repeat them.
                            // oversampling is better than locking up mutex.
                            // use floor division to find the ideal number.
                            let ideal_number_of_undersampled_thread_racks = (num_threads * 32)
                                / undersampled_thread_racks.len()
                                * undersampled_thread_racks.len();

                            // if there are too many unique racks, this no-ops.
                            // the ideal number would be zero but this is fine.
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
                    // random prefix. 62 ** 4 == 14776336, hopefully enough entropy.
                    for _ in 0..4 {
                        game_id.push(*BASE62.choose(&mut rng).unwrap() as char);
                    }
                    // wrapping sequence number. 62 ** 4 == 14776336.
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

                        // WOLGES_OPPDENIAL_EXACT gate for this turn, set
                        // inside the block below; guards the fold at the
                        // record site so a turn where the joint term was
                        // skipped (pool over the gate) or oppdenial_exact is
                        // off never folds a leftover oppdenial_exact_term.
                        let mut oppdenial_exact_active = false;
                        // WOLGES_OPPDENIAL_LEAVE: this board's opponent-denial marginals over the
                        // unseen pool, computed on the board the player faces this turn
                        // (before the play), so best_equity matches the play's board and
                        // the pool is the full tile distribution minus this board -- the
                        // census's semantics. Built by the census's own functions (the
                        // STEP-1 play-value sheet over the unseen pool, best_equity over
                        // the full-rack block, then the per-letter marginals); oppdenial_leave
                        // averages these per-board marginals for the sidecar and does not
                        // touch the recorded equity, so the summary stays byte-identical.
                        // This must run before the play borrow below (build_sheet_spell_once
                        // needs &mut move_generator, which the play reference then holds
                        // immutably) and before game_state.play mutates the board. Off
                        // (opp_ctx None) or endgame (old_bag_len == 0, no record) =>
                        // skipped, byte-identical.
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
                            // best_equity(R) over the full-rack block. When
                            // the oppdenial_exact joint term is active this
                            // turn, the argmax variant also records each
                            // rack's kept-leave argmax K* (idx + size) for
                            // opp_me2_per_rack; best is the same either way,
                            // so the marginals below read it regardless. Gate
                            // on the unseen pool size, as the census does (the
                            // per-rack pass is O(drawable^2)).
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
                                    &oppdenial_exact_kept_idx,
                                    &oppdenial_exact_kept_size,
                                    &opp_unseen,
                                    &mut oppdenial_exact_term,
                                );
                            }
                            oppdenial_exact_active = oppdenial_exact_board;
                        }

                        let knob = KnobFold {
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

                        // supplement the undersampled subracks. pick a target subrack S,
                        // complete it to a full rack with filler drawn from this board's
                        // unseen pool (minus S), and record the best play's equity
                        // attributed to S only (never decomposed to S's own subracks
                        // and never to the filler).
                        if SUMMARIZE && old_bag_len > 0 && !undersampled_thread_racks.is_empty() {
                            let chosen_undersampled_thread_rack_index =
                                rng.random_range(0..undersampled_thread_racks.len());

                            // unseen = full distribution minus tiles on board.
                            unseen_tally.clone_from_slice(&alphabet_freqs);
                            for &tile in game_state.board_tiles.iter() {
                                if tile != 0 {
                                    let base = tile & !((tile as i8) >> 7) as u8;
                                    unseen_tally[base as usize] =
                                        unseen_tally[base as usize].saturating_sub(1);
                                }
                            }
                            // remove S's tiles from unseen. S is possible on this board
                            // only if every one of its tiles is still available; if a
                            // tile is short we leave that count clamped at 0 (so filler
                            // never reuses it) and mark S impossible.
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

                            // opponent calls director if a needed tile is gone.
                            // impossible_ok short-circuits this check for full coverage:
                            // build and record S even when it is impossible here.
                            if s_possible || impossible_ok {
                                let s_subrack = &undersampled_thread_racks
                                    [chosen_undersampled_thread_rack_index];
                                // build the full rack: S plus filler drawn from the
                                // remaining unseen pool. partial Fisher-Yates, mirroring
                                // generate_gilles_summary's unseen draw. when S overdrew
                                // (impossible_ok), the pool may be short and the rack is
                                // simply smaller -- it still contains S.
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
                                    always_include_pass: false,
                                });
                                let play = &move_generator.plays[0];
                                // fold the opponent-denial of the supplemented full rack the
                                // same way the main record below does (0 when oppdenial_rack/oppdenial_exact off), so
                                // rare forced racks are valued consistently with sampled ones.
                                let rounded_equity = knob.apply(play.equity, &sample_rack_buf);
                                // attribute to the target subrack S only, count exactly 1
                                // (no positional multiplicity): S is the subrack, not a
                                // positional subset of the built rack.
                                thread_rare_subrack_map
                                    .entry(s_subrack[..].into())
                                    .and_modify(|e| {
                                        e.equity += rounded_equity;
                                        e.count += 1;
                                    })
                                    .or_insert(Cumulate {
                                        equity: rounded_equity,
                                        count: 1,
                                    });
                                undersampled_thread_racks
                                    .swap_remove(chosen_undersampled_thread_rack_index);
                                if undersampling_remediation_countdown
                                    .fetch_sub(1, std::sync::atomic::Ordering::Relaxed)
                                    <= 0
                                {
                                    // bounce back. this is why it needs to be signed (i64 not u64).
                                    undersampling_remediation_countdown
                                        .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                                    // force a global reset.
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
                                // reassess which racks are still undersampled after multiple threads worked on them.
                                undersampled_thread_racks.clear();
                            }
                        }

                        move_generator.gen_moves_unfiltered(&movegen::GenMovesParams {
                            board_snapshot,
                            rack: cur_rack,
                            max_gen: 1,
                            num_exchanges_by_this_player: game_state.current_player().num_exchanges,
                            always_include_pass: false,
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
                            // no rounding; the WOLGES_OPPDENIAL_RACK / _EXACT folds (0.0 when off) add
                            // the opponent-denial of the full drawn rack R (the per-tile
                            // marginal for oppdenial_rack, the joint opponent term for oppdenial_exact).
                            // R = cur_rack_as_vec (sorted), ranked for the oppdenial_exact lookup.
                            let rounded_equity = knob.apply(play.equity, &cur_rack_as_vec);
                            if entering {
                                // record this turn's equity keyed by the entering
                                // leave L (what this player walked in holding =
                                // their last kept). game_state.turn is restored to
                                // old_turn here, so old_turn is the player who moved.
                                // Skip the first turn (None) and keep-nothing (empty
                                // L, which would also collide with the totals key).
                                if let Some(l) = &last_kept[old_turn as usize]
                                    && !l.is_empty()
                                {
                                    thread_full_rack_map
                                        .entry(l[..].into())
                                        .and_modify(|e| {
                                            e.equity += rounded_equity;
                                            e.count += 1;
                                        })
                                        .or_insert(Cumulate {
                                            equity: rounded_equity,
                                            count: 1,
                                        });
                                }
                                // update this player's entering leave for next turn:
                                // the drawn rack minus the tiles this play used.
                                // Keep-everything (a pass) leaves L = full rack and
                                // is recorded next turn (not skipped).
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
                                thread_full_rack_map
                                    .entry(cur_rack_as_vec[..].into())
                                    .and_modify(|e| {
                                        e.equity += rounded_equity;
                                        e.count += 1;
                                    })
                                    .or_insert(Cumulate {
                                        equity: rounded_equity,
                                        count: 1,
                                    });
                            }
                        }

                        if WRITE_LOGS {
                            equity_fmt.clear();
                            // full precision, no rounding.
                            write!(equity_fmt, "{}", play.equity).unwrap();
                        }

                        let res = {
                            let game_ended =
                                game_state.check_game_ended(&game_config, &mut final_scores);
                            // do not play out the game unnecessarily. this impacts stats.
                            match game_ended {
                                game_state::CheckGameEnded::NotEnded
                                    if !WRITE_LOGS && old_bag_len == 0 =>
                                {
                                    // aborted before a real end (summarize stops
                                    // at the empty bag), so check_game_ended left
                                    // final_scores holding the previous game's
                                    // values. report THIS game's running scores.
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
                    // WOLGES_OPPDENIAL_LEAVE: fold this thread's board-marginal
                    // sums into the shared accumulator once at thread end
                    // (no-op when oppdenial_leave is off -- both sides are
                    // empty / 0).
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
        for x in full_rack_map.values() {
            total_equity += x.equity;
            row_count += x.count;
        }

        eprintln!(
            "{} records, {} unique racks",
            row_count,
            full_rack_map.len()
        );

        let mut kv = full_rack_map.iter().collect::<Vec<_>>();
        kv.sort_unstable_by(|a, b| a.0.len().cmp(&b.0.len()).then_with(|| a.0.cmp(b.0)));

        let mut csv_out = csv::Writer::from_path(format!("summary-{run_identifier}"))?;
        let mut cur_rack_ser = String::new();
        csv_out.serialize(("", total_equity, row_count))?;
        for (k, fv) in kv.iter() {
            cur_rack_ser.clear();
            for &tile in k.iter() {
                cur_rack_ser.push_str(game_config.alphabet().of_rack(tile).unwrap());
            }
            csv_out.serialize((&cur_rack_ser, fv.equity, fv.count))?;
        }

        // rare (direct) samples, keyed by target subrack S. each row
        // is (S, equity_sum, count) with count on the plain sample scale (no
        // completion-count weight); `-generate <full-rack> <out> <rare>` pools these
        // into subrack_map for S only. no totals line: the reader skips empty keys
        // and the mean stays full-rack-only. skipped entirely when nothing was rare
        // (e.g. min_samples_per_rack == 0), so that path is byte-identical to before.
        let rare_subrack_map = &mutex_guard.rare_subrack_map;
        if !rare_subrack_map.is_empty() {
            let mut rare_kv = rare_subrack_map.iter().collect::<Vec<_>>();
            rare_kv.sort_unstable_by(|a, b| a.0.len().cmp(&b.0.len()).then_with(|| a.0.cmp(b.0)));
            let mut rare_out = csv::Writer::from_path(format!("summary-rare-{run_identifier}"))?;
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

        // WOLGES_OPPDENIAL_LEAVE: write the board-averaged marginals next to the summary for the
        // decompose step. Only when oppdenial_leave is on and at least one board contributed.
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

// how WOLGES_GILLES_REAL_RACK samples the real drawn rack alongside the
// worst-group synthetic racks: off, on every turn, or only inside the snapshot
// window. parse it once into a typed value so an unknown setting fails loud
// instead of silently reverting to off.
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

// Parse the census's per-generation board-count spec: a comma-separated list
// where each element is `N` (one generation of N boards) or `KxN` (K
// generations of N boards each), e.g. "100,2x200,300,3x500" expands to
// [100, 200, 200, 300, 500, 500, 500]. The generation count is the expanded
// length, so the census derives its gens from this and needs no env. A bare
// number (no comma) is a single generation. Whitespace around elements is
// tolerated so a quoted "100, 2x200" also works.
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

// GillesB's leave generation by board sampling. Produces the same summary CSV
// as <lang>-autoplay-summarize-only (a leading ("", total_equity, total_count)
// row then rack,equity_sum,count rows), so its output merges via -resummarize
// and decomposes into leaves via -generate, identically to autoplay summaries.
//
// All parameters derive from the game_config so this works on any variant
// (classic 100-tile/15x15/rack-7, super 200-tile/21x21, larger racks, ...).
// They reduce to GillesB's published constants for the classic English game:
// snapshot while the unplayed pool holds num_tiles/4..=3*num_tiles/4 tiles (25..=75),
// draw a worst group of 2*rack_size-1 tiles (13), enumerate all rack_size
// racks (7) from it. group_size, thresholds, and stride are heuristics that
// may want tuning per variant later.
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

    let epoch_secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let run_identifier = format!("gilles-summary-{epoch_secs:08x}");

    // config-derived parameters (classic English: 25, 75, 13, 7).
    let rack_size = game_config.rack_size();
    let num_tiles: u32 = {
        let alphabet = game_config.alphabet();
        (0..alphabet.len()).map(|t| alphabet.freq(t) as u32).sum()
    };
    // config-derived defaults, overridable via env vars for experiments.
    // pool_min defaults to num_tiles/4; pool_max defaults to its mirror
    // (num_tiles - pool_min), so the default window is symmetric and setting
    // WOLGES_POOL_MIN alone shifts both edges. setting WOLGES_POOL_MAX alone
    // does NOT move pool_min (it stays num_tiles/4) -- set both for an
    // explicit asymmetric window.
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
    // min-samples coverage knobs (used only when min_samples > 0). after the
    // mandatory num_games of pure board sampling, remediation games keep playing
    // and aim each snapshot's movegens at racks still seen fewer than min_samples
    // times, until every rack reaches min_samples or no further progress is made.
    // samples_per_snapshot = movegens per remediation snapshot (kept near the pure
    // C(group_size, rack_size) for throughput); min_undersampled = how many of
    // those must hit undersampled racks before random top-up; growth_cap = how many
    // extra worst tiles the group may grow by to find more undersampled racks; the
    // run stops after max_no_progress consecutive recomputes with no shrink in the
    // remaining deficit, and force_recompute_games bounds how long a stuck
    // (unreachable) tail is chased before such a recompute happens.
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
    // also record the real rack the player actually held, not just the
    // synthetic racks drawn from the worst group. that group is the
    // least-probable tiles, so it structurally starves common vowel-rich
    // leaves (and on the rare turns they appear they come paired with junk),
    // making pure gilles undervalue vowels and overvalue rare tiles.
    // recording the real rack adds the observed mix those leaves actually
    // occur in. off (default) = pure gilles; all-turns = every turn, like
    // autoplay; in-window = only while the board is in the snapshot window.
    // the greedy movegen already found the best play, so this is nearly free.
    let (real_rack_enabled, real_rack_in_window_only, real_rack_mode) =
        match wolges_gilles_real_rack()? {
            GillesRealRack::Off => (false, false, "off"),
            GillesRealRack::AllTurns => (true, false, "all-turns"),
            GillesRealRack::InWindow => (true, true, "in-window"),
        };
    // weight per real-rack sample: count each one this many times. the worst
    // group floods many synthetic samples per game while the real rack adds
    // about one per turn, so at weight 1 the real rack is swamped; a large
    // weight lets the observed mix dominate, to test whether real-rack
    // sampling improves the leaves at all.
    let real_rack_weight = env_usize("WOLGES_GILLES_REAL_RACK_WEIGHT", 1) as u64;
    // reserved-tile-pool remediation (off by default). single-copy rare tiles
    // (e.g. Q) are usually played before the snapshot window, so racks needing
    // them stay undersampled and are expensive to reach by normal sampling.
    // when WOLGES_GILLES_RESERVE is set, each remediation game holds a batch
    // of still undersampled racks' tiles out of the bag during the draw, so
    // they remain unseen at the snapshots and become directly drawable -- at
    // the correct midgame phase and with no impossible-rack sampling. the
    // batch is filled until reserve_budget tiles are held out, so it scales
    // with the bag: reserve_budget defaults to what the bag can spare and
    // still build a board into the snapshot window (num_tiles - pool_min -
    // opening racks - a rack of slack). left off by default because the
    // held-out boards are mildly biased; enable for the rare tail and validate
    // via compare.
    let reserve_enabled = env_flag("WOLGES_GILLES_RESERVE", false);
    let reserve_budget = env_usize(
        "WOLGES_GILLES_RESERVE_BUDGET",
        (num_tiles as usize).saturating_sub(
            pool_min + rack_size as usize * game_config.num_players() as usize + rack_size as usize,
        ),
    );
    // WOLGES_OPPDENIAL_LEAVE (experiment, strength): the per-kept-leave opponent-denial term. It
    // does NOT fold into the recorded equity -- the summary stays byte-identical whether
    // it is on or off. Instead the sampler averages each sampled board's per-letter
    // opponent-denial marginals (how much the opponent's expected best play drops when
    // one tile of a letter leaves the unseen pool) over the boards, writes them to the
    // oppdenial-leave-marginal.csv sidecar, and the decompose (generate_leaves) adds
    // oppdenial_leave * sum_t S[t] * avg_marginal[t] to each kept subrack S's final leave. Off
    // (oppdenial_leave == 0.0) -> no marginals, no sidecar, byte-identical.
    let oppdenial_leave = env_parse::<f64>("WOLGES_OPPDENIAL_LEAVE", 0.0);
    // WOLGES_OPPDENIAL_RACK (experiment, strength): credit for the opponent's
    // next turn, ported from the census's WOLGES_OPPDENIAL_RACK. Where
    // oppdenial_leave averages each sampled board's opponent-denial marginals
    // into the sidecar and leaves the recorded equity untouched,
    // oppdenial_rack instead folds each recorded full rack R's own
    // opponent-denial straight into R's equity: recorded_equity(R) +=
    // oppdenial_rack * sum_t R[t] * marginal[t], where marginal[t] is how much
    // the opponent's expected best play drops when one tile of letter t leaves
    // this board's unseen pool (holding those tiles keeps them from the
    // opponent's draw). It reuses the very same per-board marginals
    // oppdenial_leave builds, so enabling it only widens the gates below
    // to build the marginals and adds the fold at the record sites. Off
    // (oppdenial_rack == 0.0) -> byte-identical.
    let oppdenial_rack = env_parse::<f64>("WOLGES_OPPDENIAL_RACK", 0.0);
    // WOLGES_OPPDENIAL_EXACT (experiment, strength): the joint opponent-and-me
    // next-turn term. Where oppdenial_rack folds a per-tile opponent-denial
    // marginal into the equity, oppdenial_exact folds the full drawn rack R's
    // exact joint term: recorded_equity(R) -= oppdenial_exact *
    // oppdenial_exact_term[R], where oppdenial_exact_term[R] = opp_value(U-R)
    // - my_next_value(K*, U-R) -- the opponent's expected best play drawing a
    // fresh rack from the R-depleted unseen pool U-R, minus my own expected
    // best next play after keeping R's argmax leave K* and refilling from U-R
    // (K* from best_equity_argmax_table, the term from
    // census::opp_me2_per_rack). O(drawable^2) per board, so it is gated to
    // boards whose unseen pool has at most oppdenial_exact_pool_max tiles;
    // larger boards skip it. Off (oppdenial_exact == 0.0) -> the
    // oppdenial_exact scratch, argmax table and per-rack pass never run,
    // byte-identical.
    let oppdenial_exact = env_parse::<f64>("WOLGES_OPPDENIAL_EXACT", 0.0);
    let oppdenial_exact_pool_max = env_usize("WOLGES_OPPDENIAL_EXACT_POOL_MAX", 32);
    let opp_on = oppdenial_leave != 0.0 || oppdenial_rack != 0.0 || oppdenial_exact != 0.0;
    // Build the census lattice, its add-table, and the millipoint leave-value array ONCE
    // (only when an opponent-denial term is on, so the default path allocates nothing and stays
    // byte-identical), shared read-only across the sampling threads exactly as the census
    // driver builds and shares them. The leave array is klv0's leave value for every
    // lattice multiset -- the census uses a single leave table for the marginals, and a
    // standard gilles run has klv0 == klv1, so this is exact.
    let opp_ctx: Option<(census::MultisetLattice, census::AddTable, Vec<i32>)> = if opp_on {
        let num_letters = game_config.alphabet().len() as usize;
        let lat = census::MultisetLattice::new(num_letters, rack_size as usize);
        let add_table = census::AddTable::new(&lat);
        let mut leave = vec![0i32; lat.len()];
        let mut tally = vec![0u8; num_letters];
        for (idx, slot) in leave.iter_mut().enumerate() {
            lat.unrank_into(idx, &mut tally);
            *slot = arc_klv0.leave_value_from_tally(&tally);
        }
        eprintln!(
            "gilles: WOLGES_OPPDENIAL_LEAVE={oppdenial_leave} WOLGES_OPPDENIAL_RACK={oppdenial_rack} WOLGES_OPPDENIAL_EXACT={oppdenial_exact} \
             oppdenial_exact_pool_max={oppdenial_exact_pool_max} opponent-denial machinery on ({} lattice leaves)",
            lat.len(),
        );
        Some((lat, add_table, leave))
    } else {
        None
    };
    eprintln!(
        "gilles: rack_size={rack_size} num_tiles={num_tiles} snapshot_pool={pool_min}..={pool_max} group_size={group_size} draws={num_draws} stride={turn_stride} min_samples={min_samples} samples_per_snapshot={samples_per_snapshot} min_undersampled={min_undersampled} growth_cap={growth_cap} reserve={reserve_enabled} reserve_budget={reserve_budget} real_rack={real_rack_mode}"
    );

    let num_processed_games = std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0));
    let completed_games = std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0));
    let completed_samples = std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0));
    // remediation coordination (mirrors generate_autoplay_logs' undersampling
    // barrier). state: 0 = mandatory sampling, 1 = one thread computing the
    // undersampled set, 2 = remediating, 3 = done. with min_samples == 0 there is
    // no remediation: threads stop once num_games have been played.
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
            threads.push(s.spawn(move || {
                let mut rng = rand::rngs::ChaCha20Rng::seed_from_u64(seed);
                let mut move_generator = movegen::KurniaMoveGenerator::new(&game_config);
                let mut game_state = game_state::GameState::new(&game_config);
                let alphabet = game_config.alphabet();
                let num_letters = alphabet.len() as usize;
                let base_freqs = (0..alphabet.len())
                    .map(|t| alphabet.freq(t))
                    .collect::<Vec<u8>>();
                // WOLGES_IMPOSSIBLE_OK (default on, the shared knob the
                // sampling path already reads): draw gilles's worst-K
                // group from the whole starting bag instead of this
                // board's unseen pool, so the group can include
                // board-impossible rares -- valued on the board with
                // their tiles forced down (the move generator plays the
                // rack regardless of what the bag still holds). Off
                // draws from the board's unseen pool (byte-identical
                // to the pre-knob behavior).
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
                // remediation thread-local state (used only when min_samples > 0).
                let mut local_undersampled = fash::MyHashSet::<bites::Bites>::default();
                let mut reserved_tally = vec![0u8; num_letters];
                let mut real_rack_buf = Vec::<u8>::with_capacity(rack_size as usize);
                // WOLGES_OPPDENIAL_LEAVE per-thread scratch, reused across boards (allocated only
                // when an opponent-denial term is on, else empty -> the per-board block is skipped
                // and the path is byte-identical). opp_sheet and opp_best are
                // lattice-length (the STEP-1 play-value sheet and best_equity over the
                // full-rack block); opp_marginal holds this board's per-letter
                // opponent-denial marginals; opp_movegen_rack and opp_blank_deltas are
                // build_sheet_spell_once's own scratch.
                let mut opp_sheet: Vec<i32> = Vec::new();
                let mut opp_best: Vec<i32> = Vec::new();
                let mut opp_marginal: Vec<f64> = Vec::new();
                let mut opp_movegen_rack: Vec<u8> = Vec::new();
                let mut opp_blank_deltas: Vec<(u8, i32)> = Vec::new();
                // WOLGES_OPPDENIAL_EXACT scratch (allocated only when
                // oppdenial_exact is on): each drawable full rack's argmax
                // kept leave K* (idx + size) from best_equity_argmax_table
                // and its joint opponent term from opp_me2_per_rack, both
                // indexed by the rack's lattice rank.
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
                // WOLGES_OPPDENIAL_LEAVE per-thread accumulators: the sum of each sampled board's
                // opponent-denial marginals and the board count, merged into the shared
                // GillesMutexed at the thread's end. Empty / 0 unless oppdenial_leave is on.
                let mut oppdenial_leave_sum_marg: Vec<f64> = if oppdenial_leave != 0.0 {
                    vec![0f64; num_letters]
                } else {
                    Vec::new()
                };
                let mut oppdenial_leave_boards = 0u64;
                let mut remediation_begun = false;
                let mut thread_generation_id = 0u64;
                let mut games_this_gen = 0u64;
                // ln C(n, k) via precomputed ln-factorials (n <= num_tiles).
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
                        // first crossing into remediation: flush this thread's
                        // samples into the shared map, wait for every thread, then
                        // one thread computes the initial undersampled set.
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
                        // refresh the thread-local undersampled view when the
                        // generation advances or it empties; one thread recomputes
                        // the global set once the countdown drains (or a stuck,
                        // unreachable tail has been chased long enough).
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
                        // reserved-tile-pool draw: hold a batch of undersampled
                        // racks' tiles (union, max per tile) out of the bag so they
                        // stay unseen at the snapshots and the racks become drawable
                        // midgame. the batch grows until reserve_budget tiles are
                        // held out (so it scales with the bag); otherwise identical
                        // to reset_and_draw_tiles_double_ended. scan a bounded slice
                        // of the undersampled set since it can be huge.
                        reserved_tally.iter_mut().for_each(|m| *m = 0);
                        let mut reserved_total = 0usize;
                        for rack in local_undersampled.iter().take(1024) {
                            if reserved_total + rack_size as usize > reserve_budget {
                                break;
                            }
                            // max-merge this rack into reserved_tally if it fits.
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
                        // sample only mid-game positions: inside the pool window, never the
                        // opening (empty board) or the endgame (empty draw bag -> no draws are
                        // taken, so the leave table is not consulted).
                        if pool_count >= pool_min
                            && pool_count <= pool_max
                            && board_tiles_count > 0
                            && !game_state.bag.is_empty()
                            && (turn_idx - *base_turn.get_or_insert(turn_idx))
                                .is_multiple_of(turn_stride)
                        {
                            // unseen = full distribution minus tiles on board.
                            unseen_tally.clone_from_slice(&base_freqs);
                            for &t in game_state.board_tiles.iter() {
                                if t != 0 {
                                    let base = t & !((t as i8) >> 7) as u8;
                                    unseen_tally[base as usize] =
                                        unseen_tally[base as usize].saturating_sub(1);
                                }
                            }
                            // gilles draws its worst-K group from this pool:
                            // the board's unseen tiles normally, or (impossible)
                            // the whole starting bag.
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
                                // keep the least-probable of num_draws groups. the
                                // denominator C(num_unseen, group_size) is constant,
                                // so minimize numerator sum ln C(unseen[i], k[i]).
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

                                // WOLGES_OPPDENIAL_EXACT gate for this board,
                                // set inside the block below; guards the fold
                                // at the record sites so a board where the
                                // joint term was skipped (pool over the gate)
                                // or oppdenial_exact is off never folds a
                                // leftover oppdenial_exact_term.
                                let mut oppdenial_exact_active = false;
                                // WOLGES_OPPDENIAL_LEAVE: this board's
                                // opponent-denial marginals over the unseen
                                // pool (how much the opponent's expected best
                                // play drops when one tile of each letter
                                // leaves the pool). Built once per sampled
                                // board by the census's own functions -- the
                                // STEP-1 play-value sheet over the board's
                                // unseen pool, best_equity over the full-rack
                                // block, then the per-letter marginals.
                                // oppdenial_leave averages these per-board
                                // marginals for the sidecar and does not touch
                                // the recorded equity, so the summary stays
                                // byte-identical. Off (opp_ctx None) =>
                                // skipped, byte-identical.
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
                                    // best_equity(R) over the full-rack block. When the
                                    // oppdenial_exact joint term is active this board, the argmax
                                    // variant also records each rack's kept-leave argmax K*
                                    // (idx + size) for opp_me2_per_rack; best is the same
                                    // either way, so the marginals below read it regardless.
                                    // Gate on the unseen pool size, as the census does (the
                                    // per-rack pass is O(drawable^2)).
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
                                            &oppdenial_exact_kept_idx,
                                            &oppdenial_exact_kept_size,
                                            &unseen_tally,
                                            &mut oppdenial_exact_term,
                                        );
                                    }
                                    oppdenial_exact_active = oppdenial_exact_board;
                                }

                                let knob = KnobFold {
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
                                    // mandatory phase: enumerate every rack of the
                                    // worst group and record its best-play equity,
                                    // evaluated with the turn player's leave file
                                    // (klv0 for p0, klv1 for p1), just like autoplay
                                    // records the turn player's rack. with the usual
                                    // klv_n vs klv_n run (klv0 == klv1) this is moot.
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
                                                    always_include_pass: false,
                                                },
                                            );
                                            let equity = knob.apply(move_generator.plays[0].equity, rack_bytes);
                                            thread_map
                                                .entry(rack_bytes.into())
                                                .and_modify(|e| {
                                                    e.equity += equity;
                                                    e.count += 1;
                                                })
                                                .or_insert(Cumulate { equity, count: 1 });
                                            completed_samples
                                                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                                        },
                                        rack_tally: &mut rack_tally,
                                        min_len: rack_size,
                                        max_len: rack_size,
                                        exchange_buffer: &mut exchange_buffer,
                                    });
                                } else {
                                    // remediation phase: aim this snapshot's movegens
                                    // at racks still below min_samples. every drawn
                                    // rack is a subset of this board's unseen pool,
                                    // hence possible -- no is_possible test needed.
                                    let mut movegens_done = 0u32;
                                    let mut undersampled_done = 0u32;
                                    // step 1: undersampled racks of the worst group.
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
                                    // step 2: grow the worst group toward
                                    // min_undersampled by adding the tiles that most
                                    // lower the group's draw probability (rarest
                                    // first), then sample the newly reachable
                                    // undersampled racks.
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
                                    // step 3: random top-up from the (grown) group
                                    // to keep about samples_per_snapshot movegens per
                                    // snapshot.
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
                                                    always_include_pass: false,
                                                },
                                            );
                                            let equity = knob.apply(move_generator.plays[0].equity, &exchange_buffer);
                                            thread_map
                                                .entry(exchange_buffer[..].into())
                                                .and_modify(|e| {
                                                    e.equity += equity;
                                                    e.count += 1;
                                                })
                                                .or_insert(Cumulate { equity, count: 1 });
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

                        // real_rack fold context: when the real rack will be recorded this
                        // turn, build this board's opponent-denial marginals with the same
                        // census pipeline the sampled records use, BEFORE the greedy gen_moves
                        // (build_sheet_spell_once borrows move_generator as scratch and
                        // overwrites its plays, so it must precede the greedy play we read and
                        // advance the board with). Off (opp_ctx None) => skipped, byte-
                        // identical. The sampler's own marginal block above is scoped to the
                        // sampled-board branch, so this is a fresh build on the actual board;
                        // real_rack fires every in-window turn, a superset of sampled turns.
                        let real_rack_here = real_rack_enabled
                            && !game_state.bag.is_empty()
                            && (!real_rack_in_window_only
                                || (pool_count >= pool_min && pool_count <= pool_max));
                        let mut rr_oppdenial_exact_active = false;
                        if real_rack_here
                            && let Some((lat, add, leave)) = opp_ctx
                        {
                            // unseen = full distribution minus tiles on board.
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
                            // opp_marginal only (no oppdenial_leave sidecar accumulation here: the
                            // sidecar averages sampled boards, not the real trajectory).
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
                                    &oppdenial_exact_kept_idx,
                                    &oppdenial_exact_kept_size,
                                    &unseen_tally,
                                    &mut oppdenial_exact_term,
                                );
                            }
                            rr_oppdenial_exact_active = oppdenial_exact_board;
                        }

                        // greedy (leave-modified) play to advance the board.
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
                            always_include_pass: false,
                        });
                        // record the real rack's best-play equity (observed
                        // mix) before playing it. only while the bag is
                        // non-empty, so there is a real leave to value (matches
                        // autoplay).
                        if real_rack_here {
                            let w = real_rack_weight;
                            real_rack_buf.clone_from(&game_state.current_player().rack);
                            real_rack_buf.sort_unstable();
                            // fold the real rack's opponent-denial the same way the sampled
                            // records do (0 when oppdenial_rack/oppdenial_exact off). the marginals were built for
                            // this exact board just above.
                            let rr_knob = KnobFold {
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
                            thread_map
                                .entry(real_rack_buf[..].into())
                                .and_modify(|e| {
                                    e.equity += eq;
                                    e.count += w;
                                })
                                .or_insert(Cumulate {
                                    equity: eq,
                                    count: w,
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
                // WOLGES_OPPDENIAL_LEAVE: fold this thread's board-marginal sums into the shared
                // accumulator (no-op when oppdenial_leave is off -- both sides are empty / 0).
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

    // write the summary CSV (same format as autoplay-summarize).
    let g = mutexed.lock().unwrap();
    let map = &g.full_rack_map;
    if min_samples != 0 && !g.undersampled_racks.is_empty() {
        // report (do not silently drop) any racks the remediation could not lift
        // to min_samples: a blocked tail of rare tiles that get played before the
        // snapshot window, addressed separately by the reserved-pool remediation.
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
    let mut csv_out = csv::Writer::from_path(&run_identifier)?;
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

    // WOLGES_OPPDENIAL_LEAVE: write the board-averaged marginals next to the summary for the
    // decompose step. Only when oppdenial_leave is on and at least one board contributed.
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
    count: u64,
}

// pool one row that already carries its own sample count into subrack_map
// by plain add.
#[inline]
fn pool_rare_one(
    subrack_map: &mut fash::MyHashMap<bites::Bites, Cumulate>,
    key: &[u8],
    equity: f64,
    count: u64,
) {
    subrack_map
        .entry(key.into())
        .and_modify(|v| {
            v.equity += equity;
            v.count += count;
        })
        .or_insert(Cumulate { equity, count });
}

// shared state guarded by the gilles mutex during min_samples remediation.
struct GillesMutexed {
    full_rack_map: fash::MyHashMap<bites::Bites, Cumulate>,
    // racks still seen fewer than min_samples times, recomputed each generation.
    undersampled_racks: Vec<bites::Bites>,
    // smallest total remaining deficit observed, for no-progress detection.
    best_remaining: u64,
    no_progress: u32,
    // WOLGES_OPPDENIAL_LEAVE: per-letter sum of each sampled board's opponent-denial marginals and
    // the number of boards, merged from the threads; the sidecar writes sum / boards.
    // Empty / 0 unless oppdenial_leave is on.
    oppdenial_leave_sum_marg: Vec<f64>,
    oppdenial_leave_boards: u64,
}

// merge a thread-local rack map into the shared map, emptying the source.
fn merge_rack_map(
    dst: &mut fash::MyHashMap<bites::Bites, Cumulate>,
    src: &mut fash::MyHashMap<bites::Bites, Cumulate>,
) {
    for (k, v) in src.drain() {
        if v.count > 0 {
            dst.entry(k)
                .and_modify(|e| {
                    e.equity += v.equity;
                    e.count += v.count;
                })
                .or_insert(v);
        }
    }
}

// rebuild undersampled_racks from the shared map (every rack_size rack seen fewer
// than min_samples times) and return the total remaining deficit. scratch_map is
// swapped with the shared map so the map can be read while undersampled_racks is
// written, then restored (left empty) on return.
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

// rebuild the autoplay undersampled set at subrack granularity, mirroring
// `-generate`'s decomposition so the rare S keys are exactly subracks that
// decomposition enumerates. pooled count of subrack S = full-rack decomposition
// (sum over full racks R of count(R) * ways(R, S)) plus the already
// accumulated rare count of S (plain, no weight) -- the same pooling
// `-generate` performs. subracks whose pooled count is below min_samples are
// pushed into `undersampled`; returns the total remaining deficit.
//
// `full_rack_map` is the full-rack map, `rare_subrack_map` the rare map; both
// are read-only here (callers swap them out of the shared mutex first so the
// borrow checker is happy and the lock is not aliased). `subrack_count` and the
// tallies are reused scratch. `alphabet_freqs` is used transiently by the
// full-subrack enumeration and is restored (generate_exchanges is balanced).
// Reused scratch buffers plus the remediation thresholds, grouped so
// recompute_undersampled_subracks stays within clippy::too_many_arguments.
struct RecomputeScratch<'a> {
    rack_tally: &'a mut [u8],
    full_rack_tally: &'a mut [u8],
    subrack_tally: &'a mut [u8],
    alphabet_freqs: &'a mut [u8],
    exchange_buffer: &'a mut Vec<u8>,
}

struct RecomputeParams {
    leave_size: u8,
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
        min_samples,
    } = params;
    // no remediation requested: nothing is ever undersampled (min_samples
    // is 0, so count >= min_samples for every rack).
    // word_prob is None in this case; bail before touching it so the barrier
    // stays a fast no-op (its only job then is to let overshoot threads stop).
    let Some(word_prob) = word_prob else {
        undersampled.clear();
        return 0;
    };
    // belt-and-suspenders guard: unreachable given current call-site gating
    // (callers pass word_prob = Some exactly when min_samples != 0), kept in
    // case that gating ever changes.
    if min_samples == 0 {
        undersampled.clear();
        return 0;
    }
    // pooled per-subrack count from the full racks. for each rack R,
    // enumerate its subracks S (length 1..=leave_size, skipping the empty
    // subrack which is the full-rack-only mean) and add count(R) * ways(R, S).
    // mirrors generate_leaves' decomposition: rack_tally is the mutable buffer
    // generate_exchanges walks, full_rack_tally a frozen copy for ways(R, S).
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
    // pool the already-accumulated rare samples (keyed by S, plain count).
    for (k, fv) in rare_subrack_map.iter() {
        if fv.count > 0 {
            *subrack_count.entry(k[..].into()).or_insert(0) += fv.count;
        }
    }
    // enumerate the full subrack space (length 1..=leave_size) exactly as
    // `-generate`'s ev_map build does, so subracks with zero full-rack and zero
    // rare samples are still detected as undersampled and get rare.
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

// movegen and record the racks of rack_tally still below min_samples, drawn
// straight from the worst (or grown) group so each is possible on this board --
// no is_possible test. stops once samples_per_snapshot movegens or `target` undersampled
// samples are reached this snapshot.
struct SampleScratch<'a> {
    rack_tally: &'a mut [u8],
    exchange_buffer: &'a mut Vec<u8>,
}

// Cross-thread progress plus the per-snapshot stopping budget for the
// undersampled pass, grouped so sample_undersampled stays within
// clippy::too_many_arguments.
struct SampleBudget<'a> {
    countdown: &'a std::sync::atomic::AtomicI64,
    completed_samples: &'a std::sync::atomic::AtomicU64,
    movegens_done: &'a mut u32,
    undersampled_done: &'a mut u32,
    samples_per_snapshot: u32,
    target: u32,
    // this board's uniform knob-fold, applied to each undersampled
    // rack's best play so remediation samples match the main pass.
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
                always_include_pass: false,
            });
            let equity = knob.apply(move_generator.plays[0].equity, rack_bytes);
            thread_map
                .entry(rack_bytes.into())
                .and_modify(|e| {
                    e.equity += equity;
                    e.count += 1;
                })
                .or_insert(Cumulate { equity, count: 1 });
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

// exact binomial coefficient via integer partial products (n, k small).
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
                full_rack_map
                    .entry(rack_bytes[..].into())
                    .and_modify(|v| {
                        v.equity += equity;
                        v.count += 1;
                    })
                    .or_insert(Cumulate { equity, count: 1 });
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
    // the recursion is balanced (push then pop), so it both requires and leaves
    // an empty buffer; clear first to be robust against a caller that left tiles
    // in it (e.g. gilles's random top-up).
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
        };
        full_rack_map
            .entry(rack_bytes[..].into())
            .and_modify(|e| {
                e.equity += thing.equity;
                e.count += thing.count;
            })
            .or_insert(thing);
    }
    drop(csv_in);

    // ("", total_equity, row_count) is ignored, it will be recomputed.
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

// Reset-board census. Per random midgame board, value EVERY leave at once:
// build a play-value sheet (best score for each playable tile-multiset) from one
// movegen over the board's unseen pool, max-plus-convolve it with the current
// leave table to get best_equity(R) for every rack R, then draw-average
// best_equity(S + drawn) to re-value each leave S (entering semantics).
// Accumulate across hard-reset boards. Output a leaves CSV directly (the census
// values are final, not a summary to be decomposed), mean-centered on the empty
// leave, ready for buildlex. klv0 (== klv1) seeds the leave table; null klv = the
// score-only gen-1 bootstrap. Boards are independent and computed in parallel.
//
// splitmix64 finalizer: mix (seed, board slot index) into a well-separated
// per-board rng seed so the produced board set is reproducible and independent of
// how the boards happen to be scheduled across threads.
const fn census_mix64(mut z: u64) -> u64 {
    z = (z ^ (z >> 30)).wrapping_mul(0xbf58476d1ce4e5b9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94d049bb133111eb);
    z ^ (z >> 31)
}

// Param bundles for build_sheet_spell_once, grouped so it stays within
// clippy::too_many_arguments. SpellTables is the static game/lexicon/lattice
// context; SpellPool is the unseen pool and its per-letter rack caps.
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

// WOLGES_OPPDENIAL_LEAVE sidecar. The per-kept-leave opponent-denial term is LINEAR in the leave,
// so a sampler cannot fold it into a full-rack summary that the decompose later averages
// -- the marginals it needs are a board property, gone by the summary. So the sampler
// writes the board-AVERAGED marginals to this sidecar and the decompose (generate_leaves)
// adds oppdenial_leave * sum_t S[t] * avg_marginal[t] to each kept subrack S's final leave. Format:
// a header row then num_letters rows of tile_index,avg_marginal (millipoints).
// avg_marginal[t] = (sum over sampled boards of that board's opponent-denial marginal for
// letter t) / boards -- the census per-board term uniformly averaged over the boards the
// sampler valued. Written next to the summary CSV, only when WOLGES_OPPDENIAL_LEAVE is set and at
// least one board contributed.
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

// Load the board-averaged marginals from the sidecar (header row + tile_index,avg_marginal
// rows) into a num_letters-length array; entries outside the range are ignored, missing
// ones stay 0.
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

// WOLGES_OPPDENIAL_RACK fold in points: oppdenial_rack * (sum over the full
// rack's tiles of the per-letter opponent-denial marginal) / equity::SCALE.
// The marginals (from the census's opp_denial_marginals) are in millipoints
// while the recorded sampler equity is in points, so the fold divides by the
// millipoint scale. An empty marginal slice (oppdenial_rack off -- so no
// marginals were built for the board) folds nothing, letting the record sites
// call this unconditionally without disturbing the byte-identical default
// output. Summing marginal[t] over each of the rack's tiles equals sum_t R[t]
// * marginal[t] (R[t] = count of letter t in the full rack R).
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

// WOLGES_OPPDENIAL_EXACT fold in points: -oppdenial_exact *
// oppdenial_exact_term[rank(R)] / equity::SCALE, the JOINT opponent-minus-me
// term the census subtracts from R's best-equity seed (best_f -=
// oppdenial_exact * oppdenial_exact_term[rank(R)]). oppdenial_exact_term (from
// census::opp_me2_per_rack) is in millipoints; the recorded equity is in
// points. `rank` is R's full-rack lattice rank. An out-of-range rank folds
// nothing -- oppdenial_exact off (empty term), this board's term inactive
// (pool over the gate, so callers pass rank = usize::MAX), or a rack outside
// the lattice / not written by opp_me2_per_rack (drawable full racks only) --
// so callers can invoke it unconditionally without disturbing the
// byte-identical default.
fn oppdenial_exact_fold(oppdenial_exact: f64, oppdenial_exact_term: &[f64], rank: usize) -> f64 {
    if rank >= oppdenial_exact_term.len() {
        return 0.0;
    }
    -oppdenial_exact * oppdenial_exact_term[rank] / equity::SCALE as f64
}

// One board/turn's uniform knob-fold, built once per board and applied at every
// rack-record site so none can silently omit a fold.
#[derive(Clone, Copy)]
struct KnobFold<'a> {
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
        base.as_f64()
            + oppdenial_rack_fold(self.oppdenial_rack, self.opp_marginal, rack)
            + oppdenial_exact_fold(self.oppdenial_exact, self.oppdenial_exact_term, rank)
    }
}

// Spell-once STEP 1 sheet build: produce the same play-value `sheet` as the wildcard
// descent without wildcarding blanks over every letter. The generator is put in
// real-before-blank mode (set_spell_once), so it descends the ACTUAL unseen rack (real
// letters capped at rack_size, blanks at blank_cap -- identical to the wildcard rack)
// and emits each feasible word ONCE (a blank is used for a letter only when no real
// copy remains; the global blank budget is enforced by the rack's blank count). Each
// all-real traversal is then expanded into its feasible blank designations
// arithmetically (play_scorer::score_and_blank_deltas gives the per-placed-tile score
// drop; census::record_blank_variants raises the sheet for every designation, incl.
// playing a blank for a letter we hold). `sheet` must be pre-initialized to the
// exchange floor (0); only raised here. The set_spell_once flag is reset afterwards so the
// generator is safe for ordinary use. Returns the candidate count for diagnostics.
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
        always_include_pass: false,
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

// how WOLGES_CENSUS_SCATTER builds best_equity on a big multi-generation pool:
// off, on, or auto (decide from the lattice size). parse it once into a typed
// value so an unknown setting fails loud instead of silently picking a mode.
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

// Per-board reuse cache for the score sheet plus its unseen tally, shared across
// generations (Mutex so the first gen's writer publishes to later gens' readers).
type SheetCacheSlot = std::sync::Mutex<Option<(Vec<i32>, Vec<u8>)>>;

// Serialize the valued leaves to a klv2 file -- the same DawgOnly/Wolges build as
// `buildlex <lang>-klv2` runs on the csv, but in-process. The machine word is the
// leave's sorted tile bytes; the value is (value_mp(idx) - baseline_mp) points as f32.
// Used for the final output and for the per-gen resume snapshots. Layout mirrors
// build_leaves_f32: [u32 kwg_len_in_u32][kwg bytes][u32 num_values][f32 values LE].
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
    // non-full (default) drops the length-rack_size full-rack values (the "pass"
    // leaves play never consults) for a smaller, play-identical table; full keeps
    // them so the resume snapshots can carry the length-rack "pass" leaves into a
    // later gen's re-valuation.
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
        // word_buf is sorted (t ascending) -- the machine-word form klv2 indexes by.
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
    // Board-fill window. The play-value sheet is one movegen over the WHOLE unseen
    // pool, whose play count explodes super-linearly with pool size (duplicate tile
    // assignments + blank wildcards): pool 27 -> about 2.3M plays (fast), pool 44 ->
    // about 3 BILLION plays (intractable). So default to LATE-midgame boards where the
    // unseen pool (= num_tiles - board) stays small but the bag is still non-empty
    // (pre-endgame, klv-applicable). pool_max keeps it tractable; pool_min keeps it
    // pre-endgame. Tune via WOLGES_POOL_MIN/MAX (the board edges follow). (This
    // enumerative builder covers only the pool-bounded window, not the full 25-75%
    // midgame.)
    let pool_max = env_usize("WOLGES_POOL_MAX", 28);
    // The window is pool-native: a board is in-window while its unseen pool (tiles not
    // on the board) lies in [pool_min, pool_max]. movegen and play_scorer both stop
    // adding the klv leave once the bag is empty (num_tiles_in_bag <= 0 -> endgame
    // penalty leaves), so a board is klv-relevant only while the bag holds >= 1 tile.
    // The bag is the pool minus the tiles held in racks (at most num_players *
    // rack_size), so pool > num_players * rack_size guarantees a non-empty bag for any
    // rack sizes. Floor pool_min there; a smaller WOLGES_POOL_MIN is raised with a
    // warning rather than silently valuing endgame boards whose leaves are never used.
    let min_pool = game_config.num_players() as usize * rack_size + 1;
    let pool_min = {
        let req = env_usize("WOLGES_POOL_MIN", 3 * rack_size);
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
    // Apportionment of a sampled board's value to leaves. Default = full-rack
    // (apportion_table): credit best_equity(R) to every subrack of R -- the
    // complete attribution the census computes inline. On CSW24 this and the
    // entering way come out even in our runs, so full-rack is the default by
    // design, not by play strength. WOLGES_APPORTION=entering opts into the
    // entering path (leave_value_by_draw): draw-average attribution crediting
    // only the held-entering leave. One unified knob, shared with autoplay's
    // entering-leave recording.
    let full_rack = match wolges_apportion()? {
        Apportion::FullRack => true,
        Apportion::Entering => false,
    };
    // entering step 3 (only used on the opt-in WOLGES_APPORTION=entering path):
    // 0 = pull (leave_value_by_draw per leave, the default), 1 = push
    // (census::entering_fused, one lattice walk for the whole table, about 20x faster
    // and bit-identical -- both exact i128). The push trades speed for memory: two
    // i128 lat_len arrays per thread (16 bytes/leave each); cut WOLGES_THREADS
    // if that is too large. Ignored on the (default) full-rack path.
    let entering_push = env_flag("WOLGES_CENSUS_ENTERING_PUSH", false);
    // board sampling: 0 = reset-per-board (the default) -- each board slot greedy-
    // plays a fresh game to one random in-window fill, values it, and resets. 1 =
    // per-game -- each slot plays one real game through and values EVERY board whose
    // fill lands in the window (consecutive real plies), so the sampled boards
    // follow a real-game phase mix (closer to autoplay's) and plies are
    // reused instead of replayed. In per-game mode the gen's board count counts
    // GAMES, not boards (one game yields a variable number of in-window boards).
    let per_game = env_flag("WOLGES_CENSUS_PER_GAME", false);
    // gens (the number of board passes) and their board counts both come from
    // the CLI spec (parse_board_counts): one list entry per gen, so gens =
    // board_counts.len() -- no env sets it and the count can differ per gen.
    // Each gen re-runs its own board count, re-centers its full-batch mean and
    // REPLACEs leave_cur (alpha = 1, the non-EMA sibling of the SGD path below),
    // so the next gen values with the improved leaves -- the leave fixed-point
    // iteration. One process amortizes the lattice + add-table build, the spin-up
    // and the external buildlex across gens (one invocation), and keeps the
    // iterated leaves in RAM. Output is the final gen's leaves, over every leave
    // valued in any gen. The worker tracks the current gen's board count as
    // gen_idx advances.
    let gens = board_counts.len();
    let multigen = gens > 1;
    // online mini-batch SGD (single-generation only). WOLGES_CENSUS_BATCH = boards
    // per mini-batch (default = the gen's board count = one batch = the plain
    // batch-mean path, byte-identical). When BATCH is below it the leaves update
    // ONLINE: after each mini-batch the leader thread EMA-blends the batch's
    // centered mean into leave_cur at rate WOLGES_CENSUS_ALPHA, so later mini-
    // batches value with improved leaves and the run converges in fewer board-
    // evals. The output is then the EMA leave_cur itself (not the batch mean). A
    // multi-gen spec always takes the full-batch iterated path (BATCH ignored).
    let batch_size = (env_usize("WOLGES_CENSUS_BATCH", board_counts[0] as usize) as u64).max(1);
    let alpha = env_parse::<f64>("WOLGES_CENSUS_ALPHA", 0.5);
    let sgd = !multigen && batch_size < board_counts[0];
    // WOLGES_CENSUS_GLOBAL_APPORTION (default 0 = the coupled per-board apportionment,
    // byte-identical). When 1 (full-rack + single full-batch gen only): decouple the
    // board-averaging from the draw-weighting. Per board, value best_equity(R) for
    // EVERY full rack from board CONTEXT (board-independent, drawable or not) and
    // accumulate a simple board mean v(R) = mean_b best_equity(R, board); then run ONE
    // apportionment over the GLOBAL bag: leave(S) = sum_{R>=S} v(R) * G(R\S) /
    // sum_{R>=S} G(R\S), where G(R\S) = ways to draw the completion R\S from the full
    // bag minus S -- via the entering push (census::entering_fused fed v(R) and
    // the full bag). This makes the completion-weighting unconditional (not each board's
    // depleted pool) and gives rare racks full board-context coverage. Ignored under SGD
    // / multi-gen (iterate via separate invocations).
    let global_apportion =
        full_rack && !sgd && !multigen && env_flag("WOLGES_CENSUS_GLOBAL_APPORTION", false);
    // WOLGES_CENSUS_GLOBAL_APPORTION_DRAWABLE (default 0; only under GLOBAL_APPORTION):
    // restrict v(R) to racks DRAWABLE from each board's unseen pool instead of valuing
    // every rack from board context. 0 = value v(R) for EVERY rack (full coverage). 1 =
    // accumulate v(R) only from boards where R is drawable; racks never drawable on any
    // sampled board stay unvalued and are masked out of the global apportionment.
    let ga_drawable =
        global_apportion && env_flag("WOLGES_CENSUS_GLOBAL_APPORTION_DRAWABLE", false);
    // WOLGES_CENSUS_SHEET_REUSE (default on; multi-gen + reset-per-board + uniform
    // spec only): the step-1 play-value sheet depends only on the board and the unseen
    // pool, NOT on the leaves, so with the deterministic reset-per-board sampler (same
    // board per slot every gen, the play policy held fixed) gen 0's sheets are valid for
    // every later gen. Cache (sheet, unseen) per board in gen 0 and reuse them in gens
    // 1.., skipping the dominant movegen sheet build -- gens 1.. then run only
    // best-equity + apportion. Byte-identical to recomputing; set to 0 to A/B. Disabled
    // under the per-game sampler (different boards each gen would make the cache stale)
    // and under a non-uniform spec (a later gen's different board count would not line
    // up with gen 0's cached slots).
    let uniform_boards = board_counts.iter().all(|&c| c == board_counts[0]);
    let sheet_reuse =
        multigen && uniform_boards && !per_game && env_flag("WOLGES_CENSUS_SHEET_REUSE", true);
    // WOLGES_CENSUS_PERSIST_GENS (default on for multi-gen): write each completed gen's
    // leaves to census-gen-<stamp>-<NN>.klv2, so a crash loses no completed gens and
    // WOLGES_CENSUS_RESUME can continue. One in-process klv2 build per gen (cheap vs the
    // board pass). WOLGES_CENSUS_RESUME (default off): on start, load the latest
    // census-gen-<stamp>-<NN>.klv2 as leave_cur and continue from the next gen.
    let persist_gens = multigen && env_flag("WOLGES_CENSUS_PERSIST_GENS", true);
    let resume = multigen && env_flag("WOLGES_CENSUS_RESUME", false);
    // reset-per-board target granularity. 0 (default) = per-tile: round-robin over
    // every fill in [low,high]. K >= 2 = round-robin over K fill targets EVENLY SPACED
    // across [low,high] ("percentage buckets" -- K=11 is 10 intervals + fencepost,
    // 10% steps): coarser, so each bucket gets more boards (less per-bucket noise at
    // low board counts), and the spacing is relative to the window so it expresses the
    // same phases on any board size. Does not change the converged leaves, only the
    // sampling granularity. The full pre-endgame span is [num_players*rack_size,
    // num_tiles - num_players*rack_size - 1] (the two racks worth of fill up to a
    // bag>=1 board); reach it with POOL_MAX = num_tiles - num_players*rack_size and
    // POOL_MIN = num_players*rack_size + 1 (English: POOL_MAX=86 POOL_MIN=15 -> fill
    // [14,85]; super-english: POOL_MAX=186 POOL_MIN=15 -> fill [14,185]). The step-1
    // sheet stays tractable even on the bigger super 21x21 board because the sheet rack
    // is capped at rack_size per letter (so its size is bounded by the alphabet, not
    // the pool) and exchange moves are skipped; an emptiest super board takes about 3s.
    let num_buckets = env_usize("WOLGES_CENSUS_BUCKETS", 0);

    let lat = census::MultisetLattice::new(num_letters, rack_size);
    let empty_rank = lat.rank(&vec![0u8; num_letters]) as usize;
    let full_rack_start = lat.full_rack_start();
    eprintln!(
        "census: lattice {} leaves (letters {num_letters}, rack_size {rack_size}), \
         window [{low_tiles},{high_tiles}] of {num_tiles} tiles",
        lat.len(),
    );

    // Parent (add-tile) index table for apportion_fused's step-3 max/add walks --
    // built once and shared read-only across the board threads. The full-rack path is
    // the default, so this builds by default; the opt-in entering path
    // (WOLGES_APPORTION=entering) does not use it, so it is skipped there.
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
    // apportion_fused step-3 method: ZETA (superset-sum) transform when the
    // board's unseen pool has at least this many tiles, else the per-rack PUSH. The
    // zeta cost is fixed (about full_rack_start * num_letters), so it wins on big pools
    // (the push touches about 2^distinct subracks per drawable rack, of which there are
    // millions) but loses on tiny pools (few drawable racks -> the push is cheaper
    // than the fixed transform). The default crossover is tuned on English; 0 forces
    // zeta always, a value above num_tiles forces the push always. English crossover:
    // the per-rack push beats the zeta below about a 35-tile pool (where the zeta's fixed
    // full_rack_start*num_letters transform outweighs touching the few drawable
    // racks) and loses above it; 36 keeps every expensive (big-pool) board on the zeta.
    let zeta_pool_min = env_usize("WOLGES_CENSUS_ZETA_POOL", 36);
    // WOLGES_CENSUS_SCATTER (gens > 1, big pool): build best_equity by a word-keyed
    // scatter (leave subset-max seed + scatter_words) instead of the per-rack rec_max
    // descent. Exact. Net-positive on realistic mixed-pool runs for the 27-29 letter
    // lattices (English +4%, French +7%, super-English +5.3%, Spanish +5%): it wins at
    // large pools (many drawable racks, the slow boards that dominate wall time) and
    // loses only a small absolute margin at the smallest zeta-gated pools. But it is
    // net-NEGATIVE on the 33-letter lattices (Polish/Norwegian, about 19M leaves: -5%),
    // where the larger maxleave subset-max pass and the scattered best[] writes over a
    // 3x bigger array outweigh the eval savings. So the default is on only below
    // a lattice-size cutoff (between Spanish's 8.35M and Polish's 18.6M); off or on
    // force it either way (on to try a big lattice, off for rec_max).
    // Values: off | on | auto, default auto.
    let scatter = match wolges_census_scatter()? {
        Scatter::Off => false,
        Scatter::On => true,
        Scatter::Auto => lat.len() <= 12_000_000,
    };
    // WOLGES_OPPDENIAL_LEAVE (strength, full-rack only): add the opponent tile-denial term
    // to each leave -- leave(S) += strength * sum_t S[t] * marginal[t], where marginal[t]
    // is how much the opponent's expected best play drops when one tile of letter t is
    // removed from the unseen pool (holding S starves the opponent's draw). The 1-ply
    // leave already sums the holder's own future; this is the sub-1-ply opponent term
    // it omits. 0 = off (byte-identical to the current census).
    let oppdenial_leave = env_parse::<f64>("WOLGES_OPPDENIAL_LEAVE", 0.0);
    // WOLGES_OPPDENIAL_RACK (strength, full-rack only): credit for the opponent's next turn (opp1).
    // Unlike oppdenial_leave (which adds strength * sum_t S[t] * marginal[t] linearly to the final
    // leave S), WOLGES_OPPDENIAL_RACK folds the opponent-denial of the FULL drawn rack R into R's
    // best-equity SEED and apportions it to subracks, so leave(S) += strength * sum_t
    // marginal[t] * E_{R>=S}[R[t]] -- the oppdenial_leave is weighted by the EXPECTED full-rack
    // count of each letter (the opponent draws from U - R, including the tiles I played),
    // the sound joint placement of the opponent term. Shares oppdenial_leave's marginals. 0 = off
    // (byte-identical). See census::apportion_fused.
    let oppdenial_rack = env_parse::<f64>("WOLGES_OPPDENIAL_RACK", 0.0);
    // WOLGES_OPPDENIAL_EXACT (strength, full-rack only): credit for both
    // coming turns (the exact model).
    // The JOINT (full-rack) opponent-minus-me term: when I draw rack R and
    // keep the argmax leave K*, the opponent draws a fresh rack from U-R (my
    // full rack gone) and my next turn refills K* from the same depleted U-R.
    // The seed becomes best_equity(R) - strength*(opp_value(U-R) -
    // my_next_value(K*, U-R)), apportioned to subracks and iterated. Unlike
    // WOLGES_OPPDENIAL_RACK (a per-tile marginal term) this is the sound
    // non-additive model -- no sum_t marginal[t] decomposition. O(drawable^2)
    // per board, so it is gated to small pools
    // (WOLGES_OPPDENIAL_EXACT_POOL_MAX, default 32); boards with a larger
    // unseen pool skip the term. 0 = off (byte-identical).
    let oppdenial_exact = env_parse::<f64>("WOLGES_OPPDENIAL_EXACT", 0.0);
    let oppdenial_exact_pool_max = env_usize("WOLGES_OPPDENIAL_EXACT_POOL_MAX", 32);

    let base_freqs: Vec<u8> = (0..alphabet.len()).map(|t| alphabet.freq(t)).collect();
    // WOLGES_CENSUS_WITHHOLD = B (default 0 = off, byte-identical): rare-rack
    // coverage remediation. Single-copy tiles (Q, Z, J, ...) are usually
    // played before the sampling window, so racks needing them are rarely
    // drawable and their leaves are starved / phase-skewed. Hold the B rarest
    // tiles out of the bag for the whole game (reset-per-board only) so they
    // stay unseen -> in the snapshot's drawable pool -> their racks get
    // board-coverage. This pass is rarity-driven (the rarest B tiles, one
    // each) and applies the same withheld set to every board. Off by default
    // (the withheld boards shift the board mix -- a quality trade to validate,
    // not a free byte-identical win like the perf levers).
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
    // WOLGES_CENSUS_WITHHOLD_FRAC = f in (0,1] (default 1.0 = every board, the rarity-driven
    // behavior): the fraction of boards that are withhold-boards. f < 1.0 dedicates only
    // every (1/f)-th board to rare-rack coverage and leaves the rest natural, so the
    // common-rack leaves keep an unbiased board mix while the rare tail still gets
    // covered. This fractioning cuts the every-board pass's all-boards mix shift
    // (every board had the rare tiles withheld, biasing every leave). The withhold boards
    // are chosen phase-balanced (every f-th board WITHIN each fill bucket, not every f-th
    // board overall) so the withheld racks are sampled across the whole pool window rather
    // than clustering in one game phase.
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
    // current leave table (millipoints), loaded from klv0 by lattice multiset.
    // null klv -> all zero -> best_equity is pure best score (gen-1 bootstrap).
    let mut leave_cur = vec![0i32; lat.len()];
    let mut tally_buf = vec![0u8; num_letters];
    for (idx, slot) in leave_cur.iter_mut().enumerate() {
        lat.unrank_into(idx, &mut tally_buf);
        *slot = arc_klv0.leave_value_from_tally(&tally_buf);
    }
    // resume: load the latest persisted gen klv2 into leave_cur and continue from the
    // next gen (0-based gen_idx == the 1-based count of completed gens).
    // The snapshots share one per-run stamp (census-gen-<stamp>-<generation>.klv2) so a
    // run's gens stay grouped; a resume reuses the stamp of the family it continues.
    // NOTE: the stamp is the wall-clock time at 1/65536-second resolution, so two runs
    // started within about fifteen microseconds of each other still share it and would
    // overwrite -- enough for crash-resume, not yet safe for two runs sharing a
    // directory at once (there is no write-time guard here yet).
    let mut start_gen = 0usize;
    let census_run_epoch;
    let mut resumed: Option<(String, usize, std::path::PathBuf)> = None;
    if resume && let Ok(rd) = std::fs::read_dir(".") {
        for e in rd.flatten() {
            let name = e.file_name();
            let name = name.to_string_lossy();
            if let Some((stamp, gg)) = name
                .strip_prefix("census-gen-")
                .and_then(|r| r.strip_suffix(".klv2"))
                .and_then(|r| r.split_once('-'))
                && u64::from_str_radix(stamp, 16).is_ok()
                && let Ok(gg) = gg.parse::<usize>()
                && resumed
                    .as_ref()
                    .is_none_or(|(bs, bg, _)| (gg, stamp) > (*bg, bs.as_str()))
            {
                resumed = Some((stamp.to_owned(), gg, e.path()));
            }
        }
    }
    if let Some((stamp, num, path)) = resumed {
        census_run_epoch = stamp;
        let bytes = std::fs::read(&path)?;
        let resume_klv = klv::Klv::<L>::from_bytes_alloc(&bytes);
        for (idx, slot) in leave_cur.iter_mut().enumerate() {
            lat.unrank_into(idx, &mut tally_buf);
            *slot = resume_klv.leave_value_from_tally(&tally_buf);
        }
        start_gen = num;
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
    // RwLock so the online-SGD path can rewrite leave_cur between mini-batches while
    // worker threads read it. In the default (one-batch) path it is only ever read.
    let leave_lock = std::sync::RwLock::new(leave_cur);

    // Boards are independent: each is one play-value sheet + best-equity
    // convolution + draw-average pass that values every leave once. Compute them in
    // parallel across threads (the VOLUME lever -- more boards = less noise + more
    // rare-spot coverage) and merge the per-leave (sum, count) accumulators under
    // one mutex. The merge is a fast add over the lattice (milliseconds); the per-board
    // compute (tens of seconds) is the cost, so lock contention is negligible.
    // Each board slot seeds its own rng deterministically from (seed, slot index)
    // so the produced board set is reproducible and independent of scheduling.
    let num_threads = wolges_threads().max(1).min(board_counts[0].max(1) as usize);
    let lat_len = lat.len();
    let next_board = std::sync::atomic::AtomicU64::new(0);
    // (accum_sum, accum_cnt, boards_completed, distinct_leaves_valued, ever_valued).
    // In SGD, accum_sum/cnt are the CURRENT mini-batch (zeroed by the leader after
    // each batch's EMA); ever_valued is persistent (which leaves to write at the end).
    // In the default one-batch path nothing is reset, so accum_sum/cnt accumulate all
    // boards across the run and ever_valued is unused.
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
    // mini-batch barrier (SGD only): all workers finish a batch, the leader EMA-updates
    // leave_cur, all resume on the next batch with the improved leaves.
    let barrier = std::sync::Barrier::new(num_threads);
    // Per-pool sample histogram for the per-game pseudo-round-robin sampler: each game
    // steers toward the least-covered pools so coverage across [pool_min, pool_max]
    // stays uniform while reusing one game's plies. Lock-free atomic counters shared
    // across threads -- this makes the per-game sample SET schedule-dependent (NOT
    // bit-reproducible, unlike reset-per-board), but the leaves converge to the same
    // seed-independent fixed point, so it is fine for generation. Unused (empty) in
    // reset-per-board mode, which stays deterministic.
    let pool_hist: Vec<std::sync::atomic::AtomicU64> = if per_game {
        (0..=num_tiles)
            .map(|_| std::sync::atomic::AtomicU64::new(0))
            .collect()
    } else {
        Vec::new()
    };
    // per-board (sheet, unseen) cache for sheet-reuse: filled in gen 0, read in gens 1..
    // (the multi-gen barrier orders all gen-0 writes before any gen-1 read; each board
    // slot is written once, by whichever thread pulls it). Empty unless sheet_reuse.
    let sheet_cache: Vec<SheetCacheSlot> = if sheet_reuse {
        (0..board_counts[0])
            .map(|_| std::sync::Mutex::new(None))
            .collect()
    } else {
        Vec::new()
    };
    eprintln!("census: {num_threads} threads over {board_counts:?} boards/gen");

    std::thread::scope(|s| {
        for _ in 0..num_threads {
            s.spawn(|| {
                // per-thread scratch, reused across this thread's boards.
                let mut game_state = game_state::GameState::new(&game_config);
                let mut move_generator = movegen::KurniaMoveGenerator::new(&game_config);
                let mut sheet = vec![census::UNPLAYABLE; lat_len];
                // spell-once scratch: the per-traversal (letter, score-drop) list,
                // reused across words.
                let mut blank_deltas = Vec::<(u8, i32)>::new();
                // entering path materializes best[]; the full-rack path fuses step2 into step3.
                let mut best = if full_rack {
                    Vec::new()
                } else {
                    vec![census::UNPLAYABLE; lat_len]
                };
                let mut contrib = vec![census::UNPLAYABLE; lat_len];
                // full-rack apportionment scratch (num/den per leave); empty unless full_rack.
                let mut num_board = if full_rack { vec![0f64; lat_len] } else { Vec::new() };
                let mut den_board = if full_rack { vec![0f64; lat_len] } else { Vec::new() };
                // gen-1 (null-leave) subset-max scratch for apportion_fused: holds the
                // sheet's subset-max so best_equity is an array read, not a per-rack
                // descent. Only touched on the null-leave + big-pool (zeta) path.
                let mut maxsheet = if full_rack { vec![0i32; lat_len] } else { Vec::new() };
                // opponent-denial scratch (WOLGES_OPPDENIAL_LEAVE or
                // WOLGES_OPPDENIAL_RACK, which share the marginals):
                // materialized best_equity over the full-rack block + the
                // per-letter oppdenial_leave marginals.
                let opp_term = oppdenial_leave != 0.0 || oppdenial_rack != 0.0;
                // best_equity is materialized for the marginal terms
                // (oppdenial_leave/oppdenial_rack) and for the oppdenial_exact
                // term, so allocate it for either.
                let mut oppdenial_leave_best = if full_rack && (opp_term || oppdenial_exact != 0.0 || global_apportion) {
                    vec![census::UNPLAYABLE; lat_len]
                } else {
                    Vec::new()
                };
                let mut oppdenial_leave_marginal = if full_rack && opp_term {
                    vec![0f64; num_letters]
                } else {
                    Vec::new()
                };
                // WOLGES_OPPDENIAL_EXACT scratch: the argmax kept leave K* per
                // full rack (index + size) and the per-rack opp_value(U-R) -
                // my_next_value(K*, U-R) term apportioned via the seed.
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
                // entering push-apportion scratch (i128 num/den); empty unless
                // entering_push.
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

                // Value one in-window board: build the play-value sheet from one
                // movegen over the unseen pool, fold it with the current leaves into
                // best_equity, optionally cross-check the null-klv/engine invariant,
                // then attribute equity to leaves (entering draw-average or full-rack
                // apportionment) and merge into the shared accumulators. Factored into a
                // closure so it reuses this thread's scratch and can be called for
                // each sampled board; the live move_generator, game_state and rng are
                // passed in so the sampler can keep driving them. `log_first` emits the
                // per-step timing diagnostics for the very first valued board;
                // `do_verify` runs the engine cross-check.
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
                    // sheet-reuse: in gens 1.. the sheet + unseen for this board were
                    // cached in gen 0 (same board, same pool -- the leaves do not affect
                    // the sheet), so load them and skip the whole step-1 build below.
                    if reuse {
                        let g = cache_slot.unwrap().lock().unwrap();
                        let (cs, cu) = g.as_ref().expect("sheet-reuse: gen 0 must cache");
                        sheet.copy_from_slice(cs);
                        unseen_tally.copy_from_slice(cu);
                    } else {
                    // unseen pool = full distribution minus tiles on the board
                    // (blank-masked). Opponent racks count as unseen/drawable, as in
                    // gilles -- sampled racks are hypothetical draws from everything
                    // not yet on the board.
                    unseen_tally.clone_from_slice(&base_freqs);
                    for &t in game_state.board_tiles.iter() {
                        if t != 0 {
                            let base = t & !((t as i8) >> 7) as u8;
                            unseen_tally[base as usize] =
                                unseen_tally[base as usize].saturating_sub(1);
                        }
                    }

                    // STEP 1 -- play-value sheet: one movegen over the unseen
                    // pool (each tile capped at rack_size, since a real rack
                    // holds at most rack_size of any letter), recording best
                    // score per played tile-multiset. Score only; the leave
                    // term is irrelevant here so any klv works. Pass = empty
                    // play, 0.
                    // init the sheet to 0 = the exchange floor: every played
                    // multiset P is worth at least 0 (dispose it via exchange,
                    // bag non-empty). The build only RAISES an entry when a
                    // word scores higher, so an unreached or negative-scoring
                    // P keeps 0; the empty P (pass / keep-all) is 0 too.
                    sheet.iter_mut().for_each(|v| *v = 0);
                    let num_blanks_eff = (unseen_tally[0] as usize).min(blank_cap);
                    let ts = std::time::Instant::now();
                    // STEP 1 build: a real-before-blank descent emits each feasible word
                    // once (a blank stands in for a letter only when no real copy is
                    // left), then play_scorer::score_and_blank_deltas and
                    // census::record_blank_variants reconstruct every blank designation
                    // arithmetically. This avoids fanning each blank out over the whole
                    // alphabet in the GADDAG word search, and produces the same sheet a
                    // plain wildcard descent would.
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
                            unseen_tally: &unseen_tally,
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
                    // sheet-reuse: cache this board's sheet + unseen for the later gens.
                    if let Some(slot) = cache_slot {
                        *slot.lock().unwrap() = Some((sheet.clone(), unseen_tally.clone()));
                    }
                    } // end of the !reuse step-1 build branch

                    // STEP 2 -- best_equity(R) for every rack, max-plus of sheet and
                    // leave_cur. Entering path only: it materializes best[] for the
                    // draw-average's random best[S+d] reads. The full-rack path fuses
                    // step 2 into step 3 (apportion_fused) -- no best[] array.
                    let ts = std::time::Instant::now();
                    if !full_rack {
                        census::best_equity_table(&lat, &sheet, leave, &mut best);
                        if log_first {
                            eprintln!("  step2 best_equity_table: {:?}", ts.elapsed());
                        }
                    }

                    // NULL-KLV / engine invariant (first board only): census
                    // best_equity(R) must equal the engine's best-play equity for R
                    // using the SAME leaves (klv0 == leave_cur), since both maximize
                    // score(P) + leave(R-P). Sample real racks and check.
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
                                // draw a random rack_size-tile rack from the pool.
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
                                        always_include_pass: false,
                                    },
                                );
                                let engine_mp = (move_generator.plays[0].equity.as_f64()
                                    * equity::SCALE as f64)
                                    .round()
                                    as i32;
                                let census_mp = if full_rack {
                                    // best[] is not materialized in the full-rack
                                    // path; recompute best_equity for this rack.
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

                    // STEP 3 -- value each leave into this board's contribution
                    // buffer. Default (full-rack apportionment): credit
                    // best_equity(R), weighted by P(draw R), to every subrack of
                    // R; this board's leave(S) = num[S]/den[S]. WOLGES_APPORTION=entering
                    // instead does the entering draw-average: best_equity(S +
                    // drawn), weighted with no replacement over completions from the
                    // unseen pool.
                    let ts = std::time::Instant::now();
                    if full_rack && !global_apportion {
                        num_board.iter_mut().for_each(|x| *x = 0.0);
                        den_board.iter_mut().for_each(|x| *x = 0.0);
                        let pool: usize = unseen_tally.iter().map(|&c| c as usize).sum();
                        // opponent terms (off unless a strength knob is set).
                        // The marginal terms (oppdenial_leave/oppdenial_rack)
                        // and the oppdenial_exact term both need best_equity
                        // materialized over the full-rack block; the
                        // oppdenial_exact path materializes it WITH the
                        // per-rack argmax kept leave K*. Computed BEFORE the
                        // apportion because oppdenial_rack/oppdenial_exact
                        // fold into best_equity's seed (oppdenial_leave applies
                        // to the final leave below). oppdenial_exact is
                        // O(drawable^2), so it is gated to small pools --
                        // larger boards skip the term.
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
                            // per-rack opp_value(U-R) - my_next_value(K*, U-R), folded into
                            // best_equity's seed by apportion_fused (drawable racks only).
                            oppdenial_exact_term.iter_mut().for_each(|x| *x = 0.0);
                            census::opp_me2_per_rack(
                                &lat,
                                add_table.as_ref().unwrap(),
                                &oppdenial_leave_best,
                                &oppdenial_exact_kept_idx,
                                &oppdenial_exact_kept_size,
                                &unseen_tally,
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
                                unseen: &unseen_tally,
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
                        for (idx, slot) in contrib.iter_mut().enumerate() {
                            *slot = if den_board[idx] > 0.0 {
                                let mut v = (num_board[idx] / den_board[idx]).round() as i32;
                                if oppdenial_leave != 0.0 {
                                    // leave(S) += strength * sum_t S[t] * marginal[t],
                                    // S = this leave-table subrack (idx).
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
                        // board-context v(R): best_equity(R) for full racks valued
                        // from board context, accumulated below as a simple board mean
                        // (sum/cnt) and apportioned once over the global bag at the
                        // finalize. The per-board pool draw-apportion is skipped.
                        census::best_equity_table(&lat, &sheet, leave, &mut oppdenial_leave_best);
                        contrib.iter_mut().for_each(|x| *x = census::UNPLAYABLE);
                        if ga_drawable {
                            // drawable-only: only racks drawable from this board's pool
                            // contribute to v(R); the rest stay UNPLAYABLE (masked out
                            // of the global apportionment by entering_fused).
                            census::mark_drawable_best(
                                &lat,
                                add_table.as_ref().unwrap(),
                                &oppdenial_leave_best,
                                &unseen_tally,
                                &mut contrib,
                            );
                        } else {
                            // every full rack valued regardless of drawability.
                            contrib[full_rack_start..]
                                .iter_mut()
                                .zip(oppdenial_leave_best[full_rack_start..].iter())
                                .for_each(|(slot, &b)| *slot = b);
                        }
                    } else if entering_push {
                        // push form: one lattice walk apportions best[] to every leave
                        // (bit-identical to the per-leave pull below, about 20x faster).
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

                    // merge this board's contribution into the shared accumulators.
                    let mut g = shared.lock().unwrap();
                    let (sum, cnt, completed, valued, _ever) = &mut *g;
                    for idx in 0..lat_len {
                        let v = contrib[idx];
                        if v != census::UNPLAYABLE {
                            if cnt[idx] == 0 {
                                *valued += 1;
                            }
                            sum[idx] += v as f64;
                            cnt[idx] += 1;
                        }
                    }
                    *completed += 1;
                    eprintln!(
                        "census: board {}/{} done ({}s), {} of {} leaves valued so far",
                        *completed,
                        cur_boards,
                        t0.elapsed().as_secs(),
                        *valued,
                        lat_len,
                    );
                };

                // outer loop over mini-batches (one batch = the whole run unless SGD),
                // wrapped by the multi-gen pass counter (`gen`; one pass unless multigen).
                let mut batch_start = 0u64;
                let mut gen_idx = start_gen;
                // boards for the current gen; advances with gen_idx at the
                // multi-gen boundary below.
                let mut num_boards = board_counts[gen_idx];
                loop {
                    // SGD slices one gen into mini-batches; otherwise a single
                    // batch covers the whole gen (num_boards = this gen's count).
                    let batch_end = if sgd {
                        (batch_start + batch_size).min(num_boards)
                    } else {
                        num_boards
                    };
                    // read-guard the leave table for this batch; the leader rewrites it
                    // (write-guard) between batches under the barrier, when no reader
                    // holds it. In the default path this guard is held for the one batch.
                    {
                        let leave = leave_lock.read().unwrap();
                        // gen-1 bootstrap: an all-zero leave lets apportion_fused take
                        // the subset-max fast path. Checked once per batch (the leave is
                        // constant within a batch; the SGD leader only rewrites it under
                        // the barrier between batches).
                        let null_leave = leave.iter().all(|&x| x == 0);
                        loop {
                            let b = next_board.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                            if b >= batch_end {
                                break;
                            }
                    let mut rng = rand::rngs::ChaCha20Rng::seed_from_u64(census_mix64(
                        seed.wrapping_add(census_mix64(b)),
                    ));

                    if per_game {
                        // Per-game pseudo-round-robin sampling: walk one real game and
                        // sample the plies whose pool bucket is currently least covered,
                        // driving toward uniform per-pool coverage while reusing the
                        // game's plies (cheaper than reset-per-board, which replays a
                        // whole game per board). `goal` = the current minimum bucket
                        // count + 1, so only buckets sitting at that minimum (the
                        // laggards) are sampled this game. `deepest` = the lowest-pool
                        // laggard; the pool only shrinks as the board fills, so once it
                        // drops below `deepest` no fillable bucket remains -- early-return
                        // instead of playing out the rest of the game. A pool that is
                        // only rarely landed on (plays jump several tiles) converges
                        // slowly but is bounded by the game budget; every pool is
                        // reachable, so it does not truly starve. `num_boards` counts
                        // GAMES here. The shared histogram makes the sample set
                        // schedule-dependent (see pool_hist).
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
                                // only shrinks); also subsumes the pool_min floor.
                            }
                            if pool <= pool_max && pool_hist[pool].load(Relaxed) < goal {
                                pool_hist[pool].fetch_add(1, Relaxed);
                                // log step timings once, on the very first valued board.
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
                            // advance the board by one greedy (leave-modified) ply.
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
                                always_include_pass: false,
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
                        // sheet-reuse: board + sheet are cached from this process's FIRST
                        // gen (gen_idx == start_gen), so later gens skip the game replay
                        // and re-value from the cache. On resume the cache starts empty,
                        // so the first resumed gen rebuilds (hence > start_gen, not > 0).
                        let reuse_board = sheet_reuse && gen_idx > start_gen;
                        if !reuse_board {
                        // greedy-play fresh games (from this slot's own rng) until one
                        // reaches this slot's target fill. The target ROUND-ROBINS across
                        // [low_tiles, high_tiles] by slot index, so every fill bucket
                        // gets an equal number of boards (uniform phase coverage) rather
                        // than the noisy bucket counts a random target gives at low
                        // board counts. Slot b's greedy games are still seeded by b, so
                        // boards stay independent and reproducible. The retry cap guards
                        // an unreachable target. The window is pool-native
                        // (WOLGES_POOL_MAX/MIN -> [low,high]): low is bounded by
                        // step-1 sheet tractability, high by the endgame floor (bag
                        // non-empty). Since the exchange-skip, even open boards (large
                        // pool) are tractable, so the window can reach early phases.
                        let target = if high_tiles <= low_tiles {
                            low_tiles
                        } else if num_buckets >= 2 {
                            // K fill targets evenly spaced across [low,high]; slot b
                            // round-robins them. j=0 -> low, j=K-1 -> high.
                            let span = high_tiles - low_tiles;
                            let j = b as usize % num_buckets;
                            low_tiles + (j * span + (num_buckets - 1) / 2) / (num_buckets - 1)
                        } else {
                            // per-tile: every fill in [low,high] is its own bucket.
                            low_tiles + (b as usize % (high_tiles - low_tiles + 1))
                        };
                        // is this slot a withhold board? Active iff withhold is on and this
                        // slot falls in the withhold fraction. Slot b's phase bucket is
                        // b % phase_buckets (mirrors the target round-robin above), so
                        // b / phase_buckets is b's round number; withholding whole rounds
                        // (round % withhold_period == 0) makes the withhold boards
                        // phase-balanced (each withhold round spans every fill bucket) and a
                        // clean 1/withhold_period fraction. Decided once per board so it is
                        // stable across the retry loop (and across gens under sheet-reuse,
                        // since the cached sheet must match the board that built it).
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
                                // withhold draw: deal the opening racks from a bag with the
                                // rare tiles removed, so they never enter play and stay in
                                // the snapshot's unseen (drawable) pool all game.
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
                            loop {
                                let fill =
                                    game_state.board_tiles.iter().filter(|&&t| t != 0).count();
                                if fill >= target {
                                    // a single play places several tiles at once, so
                                    // fill can overshoot `target` and even the window's
                                    // high end (into endgame). Count this board only if
                                    // it is still in-window (fill <= high_tiles, i.e.
                                    // pool >= pool_min); otherwise this game missed the
                                    // window -- retry a fresh game for the same target so
                                    // the sampled fill stays within [low_tiles,
                                    // high_tiles] and never reaches an empty bag.
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
                                    always_include_pass: false,
                                });
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
                            if sheet_reuse {
                                Some(&sheet_cache[b as usize])
                            } else {
                                None
                            },
                            reuse_board,
                            num_boards,
                        );
                    }
                    }
                    }
                    // mini-batch boundary (SGD only): the read guard is dropped above,
                    // so the leader can write-guard leave_cur with no readers. It
                    // EMA-blends this batch's centered mean into leave_cur, marks the
                    // valued leaves, zeroes the accumulators, and resets next_board so
                    // the next batch re-pulls from batch_end (the inner loop overshoots
                    // next_board past batch_end by up to num_threads breaking pulls).
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
                        // multi-gen boundary (gens > 1): re-center this gen's
                        // full-batch mean and REPLACE leave_cur with it (alpha = 1,
                        // not the SGD EMA above), under the barrier so the leader writes
                        // with no readers. ever[] marks every leave valued in any gen (the
                        // output set). Then reset and run the next gen, or finish -- the
                        // last gen leaves leave_cur holding the answer.
                        if multigen {
                            if barrier.wait().is_leader() {
                                // center this gen's mean into leave_cur (own lock scope so
                                // the guards drop before the persist re-locks for reading).
                                {
                                    let mut g = shared.lock().unwrap();
                                    let (sum, cnt, completed, valued, ever) = &mut *g;
                                    let mut lv = leave_lock.write().unwrap();
                                    let base = if cnt[empty_rank] > 0 {
                                        sum[empty_rank] / cnt[empty_rank] as f64
                                    } else {
                                        0.0
                                    };
                                    for idx in 0..lat_len {
                                        if cnt[idx] > 0 {
                                            ever[idx] = true;
                                            lv[idx] = (sum[idx] / cnt[idx] as f64 - base).round()
                                                as i32;
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
                                        // reset accumulators + the board cursor + the
                                        // per-pool sampler so the next gen re-runs fresh.
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
                                // persist this gen's leaves as a klv2 snapshot (resume
                                // safety). Re-lock as shared reads -- the write guard above
                                // is dropped, and the other workers are parked on the
                                // barrier below, so there is no contention.
                                if persist_gens {
                                    let g = shared.lock().unwrap();
                                    let lv = leave_lock.read().unwrap();
                                    let p = format!("census-gen-{census_run_epoch}-{:02}.klv2", gen_idx + 1);
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
                            }
                            barrier.wait();
                            if gen_idx + 1 < gens {
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

    // global-apportion: the across-board accumulators hold a per-rack board mean
    // v(R) = accum_sum[R]/accum_cnt[R] (board-context best_equity, every full rack
    // valued every board). Apportion it ONCE over the global bag to form
    // every leave -- leave(S) = sum_{R>=S} v(R)*G(R\S) / sum_{R>=S} G(R\S) -- via the
    // entering push fed v(R) as the "best" table and the full bag as the
    // "unseen" pool. (ga_num, ga_den) are then this run's num/den per leave.
    let (ga_num, ga_den) = if global_apportion {
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

    // Write (leave, value_in_points), mean-centered on the empty leave (its
    // entering-equity = the average best full-rack equity = the baseline). The
    // default one-batch path reports each leave's accumulated best-equity mean; the SGD
    // and multi-gen paths report leave_cur itself (the EMA / final-gen leaves, in the
    // same millipoint frame), restricted to leaves valued in some mini-batch / gen; the
    // global-apportion path reports the single global-bag apportionment ga_num/ga_den.
    let value_mp = |idx: usize| -> f64 {
        if global_apportion {
            if ga_den[idx] != 0 {
                (ga_num[idx] / ga_den[idx]) as f64
            } else {
                0.0
            }
        } else if sgd || multigen {
            leave_final[idx] as f64
        } else if accum_cnt[idx] > 0 {
            accum_sum[idx] / accum_cnt[idx] as f64
        } else {
            0.0
        }
    };
    let baseline = value_mp(empty_rank);
    let epoch = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let out_name = format!("census-leaves-{epoch:08x}.csv");
    // non-full by default: a play table never keeps a full rack (a pass is
    // almost never the best move mid-game, and the empty-bag endgame scores with a
    // penalty term, not the klv), so dropping the length-rack_size values is a
    // smaller, play-identical table. WOLGES_FULL=1 keeps the full lengths.
    let emit_full = env_flag("WOLGES_FULL", false);
    let max_keep = if emit_full {
        rack_size
    } else {
        rack_size.saturating_sub(1)
    };
    let mut rows: Vec<(usize, String, f64)> = Vec::new();
    let mut leave_ser = String::new();
    for idx in 0..lat.len() {
        let valued = if global_apportion {
            ga_den[idx] != 0
        } else if sgd || multigen {
            ever[idx]
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

    // Emit the klv2 in-process (skips the external buildlex; same DawgOnly/Wolges build).
    let klv_name = format!("census-leaves-{epoch:08x}.klv2");
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

// per-rack contribution to a subrack's running (equity, count) accumulator
// during `-generate` decomposition. `fv` is the full rack's (equity_sum,
// sample_count); `w` is completion_draw_ways (global-bag combos for the drawn
// completion). Per-occurrence (per_rack false) folds the sampled count(R) into
// the weight, so a rack drawn often pulls on the common subracks it shares by
// that draw count on top of its average -- double-counting the draw frequency.
// per_rack weights the rack's MEAN by the global-bag combos alone, so a rack
// counts once at its average and its draw count never inflates the weight.
fn decompose_contribution(fv: &Cumulate, w: u64, per_rack: bool) -> (f64, u64) {
    if per_rack {
        (fv.equity / fv.count as f64 * w as f64, w)
    } else {
        (fv.equity * w as f64, fv.count * w)
    }
}

fn generate_leaves<
    Readable: std::io::Read,
    W: std::io::Write,
    const DO_SMOOTHING: bool,
    const IS_FULL_RACK: bool,
>(
    game_config: game_config::GameConfig,
    mut csv_in: csv::Reader<Readable>,
    mut csv_out: csv::Writer<W>,
    rare_path: Option<&str>,
) -> error::Returns<()> {
    let mut stdout_or_stderr = boxed_stdout_or_stderr();
    // per-rack decomposition (opt-in, off = byte-identical). Default
    // off weights each rack by its sampled count; on weights each rack's mean by
    // the global-bag combos alone so a rack's draw count does not inflate its
    // pull on shared subracks. See decompose_contribution.
    let per_rack = env_flag("WOLGES_GENERATE_PER_RACK", false);
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
        };
        full_rack_map
            .entry(rack_bytes[..].into())
            .and_modify(|e| {
                e.equity += thing.equity;
                e.count += thing.count;
            })
            .or_insert(thing);
    }
    drop(csv_in);
    // ("", total_equity, row_count) must exist.
    full_rack_map
        .remove([][..].into())
        .ok_or("input file does not include totals line")?;

    let leave_size = game_config.rack_size() - 1 + IS_FULL_RACK as u8;

    // subrack_map[subrack] = sum(full_rack_map[subrack + completion]).
    let mut subrack_map = fash::MyHashMap::<bites::Bites, Cumulate>::default();
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
                        });
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
    } = subrack_map
        .remove([][..].into())
        .ok_or("empty-rack entry should not be missing")?;

    // pool rare samples directly into subrack_map.
    // these rows are keyed by their target subrack and are already on the
    // sample-count scale, so add them plainly with no ways(R, S) weight.
    // rare rows never key the empty leave, so the mean stays full-rack-only.
    if let Some(fp) = rare_path {
        let mut rare_reader = csv::ReaderBuilder::new().has_headers(false).from_path(fp)?;
        for result in rare_reader.records() {
            let record = result?;
            if record[0].is_empty() {
                continue;
            }
            let equity = f64::from_str(&record[1])?;
            let count = u64::from_str(&record[2])?;
            parse_rack(&rack_reader, &record[0], &mut rack_bytes)?;
            pool_rare_one(&mut subrack_map, &rack_bytes, equity, count);
        }
    }

    let threshold_count = if DO_SMOOTHING {
        let total_count = subrack_map.values().fold(0, |a, x| a + x.count);
        (total_count as f64).cbrt().ceil() as u64 // inaccurate after 2^53
    } else {
        0
    };
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
    generate_exchanges(&mut ExchangeEnv {
        found_exchange_move: |rack_bytes: &[u8]| {
            let mut new_v = if let Some(v) = subrack_map.get(rack_bytes) {
                if !DO_SMOOTHING || v.count >= threshold_count {
                    v.equity / v.count as f64
                } else {
                    // perform smoothing if there are too few samples.
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
    writeln!(
        stdout_or_stderr,
        "After {} seconds, have processed {} subracks and smoothed {}",
        t0.elapsed().as_secs(),
        ev_map.len(),
        num_smoothed,
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

    // WOLGES_OPPDENIAL_LEAVE: add the linear opponent tile-denial term to each FINAL leave value,
    // using the board-averaged marginals the sampler wrote to the sidecar:
    // leave(S) += oppdenial_leave * sum_t S[t] * avg_marginal[t] / equity::SCALE (avg_marginal is
    // millipoints; ev_map values are points). Applied after ev_map is finalized (smoothed,
    // filled in, mean-centred) and before the CSV write. Off (oppdenial_leave == 0.0) or no sidecar
    // present => no change => byte-identical decompose.
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

    let epoch_secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let run_identifier = std::sync::Arc::new(format!("{epoch_secs:08x}"));
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
                                    // bites::Bites does not DerefMut.
                                    let w_len = w.len();
                                    unjumble_buf.resize(w_len.max(unjumble_buf.len()), 0);
                                    unjumble_buf[..w_len].clone_from_slice(w);
                                    unjumble_buf[..w_len].sort_unstable();
                                    v.push((unjumble_buf[..w_len].into(), num_plays));
                                }
                            }
                        }
                    };
                // words played in the same turn (hooks) get the same usize.
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
                                    always_include_pass: false,
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
                            // num_plays == 0 means all moves were exchanges/pass.
                            if num_plays > 0 {
                                vec_played.sort_unstable();
                                vec_played.dedup(); // playing the same word as main+hook or hook+hook counts once.
                                // each word gets n/d if played in n of d equally top moves.
                                let multiplier = (num_plays as f64).recip();
                                for same_words in vec_played.chunk_by(|a, b| a.0 == b.0) {
                                    let occurrence = same_words.len() as f64 * multiplier;
                                    // allocs for long words, but long words are rarely played.
                                    thread_full_word_map
                                        .entry(same_words[0].0[..].into())
                                        .and_modify(|e| {
                                            e.equity += occurrence;
                                            e.count += 1;
                                        })
                                        .or_insert(Cumulate {
                                            equity: occurrence,
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
                            // bag is empty, skip the rest of the game.
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
                                // nothing logged, just grab the mutex to report time less often.
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

        let mut csv_out = csv::Writer::from_path(format!("playability-{run_identifier}"))?;
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

    // Machine-readable summary block (opt-in, WOLGES_COMPARE_PORCELAIN). One
    // `KEY VALUE` per line, the value the only token (no parens) so a driver can
    // grep it unambiguously, e.g. `awk '/^WCMP_P0_PCT /{print $2}'`. This is the
    // root-cause fix for the greedy `.*\(...\)` that misread the human-readable
    // "p0 ... (X%)  p1 ... (Y%)" line (grabbing p1's percent instead of p0's).
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
        // Opt-in machine-readable summary of the full set (not the divergent subset); trails
        // the human output so default behavior is unchanged.
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

// Apportion (w, wo) to every subrack S <= R (R given as a per-letter tally):
// num[rank(S)] += wo, den[rank(S)] += w, scnt[rank(S)] += 1 (the unweighted
// sample count, used by the prior-blend to trust well-sampled leaves and
// fall back to the prior on sparse ones). Enumerates each present letter's kept
// count 0..=R[t] (a handful of subracks per rack). The MC rollout analog of
// apportion_fused's apportion_rec, but the apportioned value is passed in rather than a
// best_equity, and it runs over one observed rack at a time.
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
    // Constant context for the subrack recursion, so rec carries only the changing
    // position -- no clippy::too_many_arguments.
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

// Monte-Carlo rollout leave generation. The model-free counterpart to the
// 1-ply full-rack census: play full self-play games with the policy leaves
// `arc_klv`, then back-attribute each game's final MARGIN (from the holder's
// perspective) to every rack held during the game, apportioned to all
// subracks weighted by the draw-ways w(R) = ProdC(unseen, R) (the same weight
// apportion_fused uses). rollout(S) = sum w*margin / sum w over all (game,
// held-rack R >= S), centered on the empty leave. Margin-delta attribution,
// full-game depth, per the MC decisions 2026-06-17.
//
// With WOLGES_ROLLOUT_SHRINK = 0 (the default) the raw rollout value is used,
// which fails badly -- rare multi-tile leaves get few samples, hence huge
// noisy values that sabotage play. WOLGES_ROLLOUT_SHRINK = K > 0 pulls the
// value toward the prior `arc_klv` (pass the 1-ply census leaves as both the
// play policy and the prior):
//   leave(S) = prior(S) + trust(S) * (rollout(S) - prior(S)),
//   trust(S) = scnt(S) / (scnt(S) + K)
// so sparse leaves keep the census's exact (low-variance) value and only
// well-sampled leaves take the rollout's multi-ply correction.
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
    // Baseline subtraction (WOLGES_ROLLOUT_CV=1), a control variate -- subtract a
    // quantity with a known average to cut noise: credit (margin - 1-ply play
    // equity) and add the census prior back, instead of the raw margin. Removes the
    // rack-quality variance the census already explains, leaving the multi-ply
    // residual; pass the 1-ply census as both policy and prior. Overrides the prior-blend.
    let cv = env_flag("WOLGES_ROLLOUT_CV", false);
    if cv {
        eprintln!(
            "rollout: baseline-subtraction mode (credit margin - play equity, add prior back)"
        );
    }
    let kwg = std::sync::Arc::new(kwg);
    let next_game = std::sync::atomic::AtomicU64::new(0);
    // (num, den, scnt) accumulators over the lattice, merged once per thread at
    // the end; scnt is the unweighted sample count per leave (for the prior-blend).
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
                // per-game plies: (mover, rack tally, draw-ways weight, play equity).
                let mut plies: Vec<(u8, Vec<u8>, f64, f64)> = Vec::new();
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
                        // R = the mover's full rack as a tally (blank = letter 0).
                        rack_scratch.iter_mut().for_each(|x| *x = 0);
                        for &t in game_state.current_player().rack.iter() {
                            rack_scratch[t as usize] += 1;
                        }
                        // unseen = full distribution minus tiles on board, so unseen
                        // contains R -> w(R) = ProdC(unseen, R) is nonzero, matching the
                        // full-rack census's unseen pool.
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
                            always_include_pass: false,
                        });
                        // the mover's best-play equity = the census's 1-ply value of R,
                        // the baseline's prediction of this turn's worth.
                        let e_t = move_generator.plays[0].equity.as_f64();
                        plies.push((mover, rack_scratch.clone(), w, e_t));
                        let play = &move_generator.plays[0].play;
                        game_state.play(&game_config, &mut rng, play).unwrap();
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
                    // back-attribute to each held rack's subracks. the raw rollout/the prior-blend apportion the
                    // raw outcome (margin, signed per holder); CV apportions the residual
                    // outcome - 1-ply play equity (the census prior is added back at
                    // output, so this carries only the multi-ply correction).
                    for (mover, r_tally, w, e_t) in plies.iter() {
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
    // rollout(S) = sum w*margin / sum w (millipoints), centered on the empty leave
    // (its value = the average game margin = the baseline).
    let value_mp = |idx: usize| -> f64 {
        if den[idx] > 0.0 {
            num[idx] / den[idx]
        } else {
            0.0
        }
    };
    let baseline = value_mp(empty_rank);
    // prior-blend toward the prior klv: K = 0 -> raw rollout; K > 0 ->
    // trust = scnt/(scnt+K), blending sparse leaves back to the prior (pass the
    // 1-ply census leaves as the prior to floor rare leaves at their exact value).
    let shrink_k = std::env::var("WOLGES_ROLLOUT_SHRINK")
        .ok()
        .and_then(|s| s.parse::<f64>().ok())
        .unwrap_or(0.0);
    if shrink_k > 0.0 {
        eprintln!("rollout: shrinking toward prior klv with K={shrink_k}");
    }
    let epoch = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let out_name = format!("rollout-leaves-{epoch:08x}.csv");
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
        // value centered on the empty leave (millipoints), converted to points
        // as the census does (margins are millipoints). Three modes:
        // - CV: add the census prior back to the centered residual.
        // - Prior-blend: blend toward the prior by sample-count trust.
        // - Raw: the raw centered rollout.
        let centered_mp = value_mp(idx) - baseline;
        let centered = if cv {
            // CV: census prior + the centered residual, sample-count shrunk so rare
            // leaves (whose residual is still full-game-margin noise) fall back to
            // the prior. WOLGES_ROLLOUT_SHRINK=0 -> trust=1 (no shrink).
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
                            let board_snapshot = movegen::BoardSnapshot {
                                board_tiles: &game_state.board_tiles,
                                game_config: &game_config,
                                kwg: &kwg,
                                klv: if (game_state.turn == 0) != klv_swapped {
                                    &arc_klv0
                                } else {
                                    &arc_klv1
                                },
                            };
                            move_generator.gen_moves_unfiltered(&movegen::GenMovesParams {
                                board_snapshot: &board_snapshot,
                                rack: &game_state.current_player().rack,
                                max_gen: 1,
                                num_exchanges_by_this_player: game_state
                                    .current_player()
                                    .num_exchanges,
                                always_include_pass: false,
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
                    // also check if game 1 ended at a different turn
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_board_counts_expands_repeats() {
        assert_eq!(parse_board_counts("256").unwrap(), vec![256]);
        assert_eq!(
            parse_board_counts("100,200,200,300,500,500,500").unwrap(),
            vec![100, 200, 200, 300, 500, 500, 500]
        );
        // KxN repeat shorthand (count-first) expands to the same list.
        assert_eq!(
            parse_board_counts("100,2x200,300,3x500").unwrap(),
            vec![100, 200, 200, 300, 500, 500, 500]
        );
        assert_eq!(
            parse_board_counts("4x256").unwrap(),
            vec![256, 256, 256, 256]
        );
        // whitespace around elements tolerated (a quoted spec).
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
        // full rack R: equity_sum 10 over count 2 (mean 5), completion ways w = 3.
        let fv = Cumulate {
            equity: 10.0,
            count: 2,
        };
        // per-occurrence: contribute equity_sum*w and count*w, so the
        // sampled count enters both, then -generate divides them back out ->
        // per-occurrence mean. a rack's draw count inflates its pull here.
        let (eq, cnt) = decompose_contribution(&fv, 3, false);
        assert!((eq - 30.0).abs() < 1e-9); // 10 * 3
        assert_eq!(cnt, 6); // 2 * 3
        // per-rack: contribute mean*w and w, so the sampled count drops
        // out of the weight -> leave(S) = sum mean(R)*w / sum w. a rack
        // counts once at its mean, never inflated by its draw count.
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
            },
        ); // full-rack A, sum10 n2
        pool_rare_one(&mut m, &b"\x01"[..], 5.0, 3); // rare A, sum5 n3
        let a = m.get(&b"\x01"[..]).unwrap();
        assert_eq!(a.count, 5);
        assert!((a.equity - 15.0).abs() < 1e-9); // mean 15/5 = 3.0
    }
}
