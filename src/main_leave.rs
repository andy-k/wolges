// Copyright (C) 2020-2026 Andy Kurnia.

use rand::prelude::*;
use std::fmt::Write;
use std::io::Write as _;
use std::str::FromStr;
use wolges::{
    alphabet, bites, census, display, equity, error, fash, game_config, game_state, klv, kwg,
    move_filter, move_picker, movegen, prob, stats,
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
                // english-census CSW24.kwg leave0.klv leave1.klv 500 [seed]
                let args3 = if args.len() > 3 { &args[3] } else { "-" };
                let args4 = if args.len() > 4 { &args[4] } else { "-" };
                let num_boards = if args.len() > 5 {
                    u64::from_str(&args[5])?
                } else {
                    500
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
                    num_boards,
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
                                let rounded_equity = play.equity.as_f64(); // no rounding
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
                            let rounded_equity = play.equity.as_f64(); // no rounding
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
                                            let equity = move_generator.plays[0].equity.as_f64();
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
                                            let equity = move_generator.plays[0].equity.as_f64();
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
                        if real_rack_enabled
                            && !game_state.bag.is_empty()
                            && (!real_rack_in_window_only
                                || (pool_count >= pool_min
                                    && pool_count <= pool_max))
                        {
                            let w = real_rack_weight;
                            let eq = move_generator.plays[0].equity.as_f64() * w as f64;
                            real_rack_buf.clone_from(&game_state.current_player().rack);
                            real_rack_buf.sort_unstable();
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
            let equity = move_generator.plays[0].equity.as_f64();
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

fn generate_census_leaves<N: kwg::Node + Sync + Send, L: kwg::Node + Sync + Send>(
    game_config: game_config::GameConfig,
    kwg: kwg::Kwg<N>,
    arc_klv0: std::sync::Arc<klv::Klv<L>>,
    arc_klv1: std::sync::Arc<klv::Klv<L>>,
    num_boards: u64,
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
    // Apportionment of a sampled board's value to leaves. Default = entering:
    // draw-average attribution (leave_value_by_draw), crediting the held-entering
    // leave. WOLGES_APPORTION=full-rack opts into the full-rack apportionment
    // (apportion_table): credit best_equity(R) to every subrack of R. One unified
    // knob, shared with autoplay's entering-leave recording.
    let full_rack = match std::env::var("WOLGES_APPORTION").ok().as_deref() {
        None | Some("entering") => false,
        Some("full-rack") => true,
        Some(other) => {
            return Err(
                format!("WOLGES_APPORTION must be full-rack or entering, got {other:?}").into(),
            );
        }
    };
    // board sampling: 0 = reset-per-board (the default) -- each board slot greedy-
    // plays a fresh game to one random in-window fill, values it, and resets. 1 =
    // per-game -- each slot plays one real game through and values EVERY board whose
    // fill lands in the window (consecutive real plies), so the sampled boards
    // follow a real game's phase mix (closer to autoplay's) and plies are
    // reused instead of replayed. In per-game mode num_boards counts GAMES, not
    // boards (one game yields a variable number of in-window boards).
    let per_game = env_flag("WOLGES_CENSUS_PER_GAME", false);

    let lat = census::MultisetLattice::new(num_letters, rack_size);
    let empty_rank = lat.rank(&vec![0u8; num_letters]) as usize;
    eprintln!(
        "census: lattice {} leaves (letters {num_letters}, rack_size {rack_size}), \
         window [{low_tiles},{high_tiles}] of {num_tiles} tiles",
        lat.len(),
    );

    let base_freqs: Vec<u8> = (0..alphabet.len()).map(|t| alphabet.freq(t)).collect();
    let seed = seed.unwrap_or_else(rand::random);
    // current leave table (millipoints), loaded from klv0 by lattice multiset.
    // null klv -> all zero -> best_equity is pure best score (gen-1 bootstrap).
    let mut leave_cur = vec![0i32; lat.len()];
    let mut tally_buf = vec![0u8; num_letters];
    for (idx, slot) in leave_cur.iter_mut().enumerate() {
        lat.unrank_into(idx, &mut tally_buf);
        *slot = arc_klv0.leave_value_from_tally(&tally_buf);
    }

    // Boards are independent: each is one play-value sheet + best-equity
    // convolution + draw-average pass that values every leave once. Compute them in
    // parallel across threads (the VOLUME lever -- more boards = less noise + more
    // rare-spot coverage) and merge the per-leave (sum, count) accumulators under
    // one mutex. The merge is a fast add over the lattice (milliseconds); the per-board
    // compute (tens of seconds) is the cost, so lock contention is negligible.
    // Each board slot seeds its own rng deterministically from (seed, slot index)
    // so the produced board set is reproducible and independent of scheduling.
    let num_threads = wolges_threads().max(1).min(num_boards.max(1) as usize);
    let lat_len = lat.len();
    let next_board = std::sync::atomic::AtomicU64::new(0);
    // (accum_sum, accum_cnt, boards_completed, distinct_leaves_valued)
    let shared = std::sync::Mutex::new((vec![0f64; lat_len], vec![0u64; lat_len], 0u64, 0u64));
    eprintln!("census: {num_threads} threads over {num_boards} boards");

    std::thread::scope(|s| {
        for _ in 0..num_threads {
            s.spawn(|| {
                // per-thread scratch, reused across this thread's boards.
                let mut game_state = game_state::GameState::new(&game_config);
                let mut move_generator = movegen::KurniaMoveGenerator::new(&game_config);
                let mut sheet = vec![census::UNPLAYABLE; lat_len];
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
                let mut tally_buf = vec![0u8; num_letters];
                let mut unseen_tally = vec![0u8; num_letters];
                let mut unseen_pool = Vec::<u8>::new();
                let mut movegen_rack = Vec::<u8>::new();
                let mut played_buf = Vec::<u8>::new();
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
                                       log_first: bool,
                                       do_verify: bool| {
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

                    // STEP 1 -- play-value sheet: one movegen over the unseen pool
                    // (each tile capped at rack_size, since a real rack holds at most
                    // rack_size of any letter), recording best score per played
                    // tile-multiset. Score only; the leave term is irrelevant here so
                    // any klv works. Pass = empty play, 0.
                    movegen_rack.clear();
                    for (t, &c) in unseen_tally.iter().enumerate() {
                        // blank (tile 0) plays as any letter, so each blank in the
                        // pool sends the movegen search down every word path the
                        // board allows; cap it separately to probe whether blanks
                        // drive the blowup.
                        let cap = if t == 0 { blank_cap } else { rack_size };
                        for _ in 0..(c as usize).min(cap) {
                            movegen_rack.push(t as u8);
                        }
                    }
                    // init the sheet to 0 = the exchange floor: every played
                    // multiset P is worth at least 0 (dispose it via exchange, bag
                    // non-empty). The build only RAISES an entry when a word scores
                    // higher, so an unreached or negative-scoring P keeps 0; the
                    // empty P (pass / keep-all) is 0 too.
                    sheet.iter_mut().for_each(|v| *v = 0);
                    let ts = std::time::Instant::now();
                    let mut n_cand = 0u64;
                    {
                        let board_snapshot = &movegen::BoardSnapshot {
                            board_tiles: &game_state.board_tiles,
                            game_config: &game_config,
                            kwg: &kwg,
                            klv: &arc_klv0,
                        };
                        let params = movegen::GenMovesParams {
                            board_snapshot,
                            rack: &movegen_rack,
                            max_gen: 1,
                            // suppress exchange-move generation: over the whole unseen
                            // pool it enumerates about 2^pool subsets (the actual cost,
                            // not the gaddag word search). The sheet wants place moves
                            // only; exchange is modeled by the convolution disposal
                            // floor.
                            num_exchanges_by_this_player: i16::MAX,
                            always_include_pass: false,
                        };
                        // Record the best score per played multiset directly in the
                        // place predicate and return false: the play is never stored
                        // (the pool can yield billions of candidates) and its
                        // equity/leave is never computed. The GADDAG traversal still
                        // visits each candidate.
                        move_generator.gen_moves_filtered(
                            &params,
                            |_down, _lane, _idx, word: &[u8], score: i32| {
                                n_cand += 1;
                                played_buf.clear();
                                for &tile in word {
                                    if tile != 0 {
                                        played_buf.push(tile & !((tile as i8) >> 7) as u8);
                                    }
                                }
                                if played_buf.len() <= rack_size {
                                    played_buf.sort_unstable();
                                    let pr = lat.rank_bytes(&played_buf);
                                    if pr != !0 && score > sheet[pr as usize] {
                                        sheet[pr as usize] = score;
                                    }
                                }
                                false // never keep the move
                            },
                            |leave_value| leave_value,
                            |_equity, _play| false,
                        );
                    }
                    if log_first {
                        eprintln!(
                            "  step1 sheet: {} tiles in pool -> {} candidate plays (unstored) in {:?}",
                            movegen_rack.len(),
                            n_cand,
                            ts.elapsed(),
                        );
                    }

                    // STEP 2 -- best_equity(R) for every rack, max-plus of sheet and
                    // leave_cur. Entering path only: it materializes best[] for the
                    // draw-average's random best[S+d] reads. The full-rack path fuses
                    // step 2 into step 3 (apportion_fused) -- no best[] array.
                    let ts = std::time::Instant::now();
                    if !full_rack {
                        census::best_equity_table(&lat, &sheet, &leave_cur, &mut best);
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
                                        &lat, &sheet, &leave_cur, &tally_buf,
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
                    // buffer. Default = entering: draw-average best_equity(S +
                    // drawn), weighted by how likely each completion is to be
                    // drawn from the unseen pool (no replacement).
                    // WOLGES_APPORTION=full-rack switches to full-rack
                    // apportionment: credit best_equity(R), weighted by
                    // P(draw R), to every subrack of R; this board's
                    // leave(S) = num[S]/den[S].
                    let ts = std::time::Instant::now();
                    if full_rack {
                        num_board.iter_mut().for_each(|x| *x = 0.0);
                        den_board.iter_mut().for_each(|x| *x = 0.0);
                        census::apportion_fused(
                            &lat,
                            &sheet,
                            &leave_cur,
                            &unseen_tally,
                            &mut num_board,
                            &mut den_board,
                        );
                        for (idx, slot) in contrib.iter_mut().enumerate() {
                            *slot = if den_board[idx] > 0.0 {
                                (num_board[idx] / den_board[idx]).round() as i32
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
                    let (sum, cnt, completed, valued) = &mut *g;
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
                        num_boards,
                        t0.elapsed().as_secs(),
                        *valued,
                        lat_len,
                    );
                };

                loop {
                    let b = next_board.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    if b >= num_boards {
                        break;
                    }
                    let mut rng = rand::rngs::ChaCha20Rng::seed_from_u64(census_mix64(
                        seed.wrapping_add(census_mix64(b)),
                    ));

                    if per_game {
                        // Per-game sampling: play one real game through and value EVERY
                        // board whose fill lands in the window [low_tiles, high_tiles].
                        // These are consecutive real plies, so recorded boards follow
                        // the phase mix a real game visits (closer to autoplay's) and
                        // each ply is reused instead of replayed per board. `num_boards`
                        // counts GAMES here; a game yields a variable number of boards.
                        game_state.reset_and_draw_tiles_double_ended(&game_config, &mut rng);
                        let mut logged = false;
                        loop {
                            let fill =
                                game_state.board_tiles.iter().filter(|&&t| t != 0).count();
                            if fill > high_tiles {
                                break; // past the pre-endgame window; this game is done.
                            }
                            if fill >= low_tiles {
                                // log step timings once, on the very first valued board.
                                let lf = b == 0 && !logged;
                                logged |= lf;
                                value_board(
                                    &mut move_generator,
                                    &game_state,
                                    &mut rng,
                                    lf,
                                    verify && lf,
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
                        let target = if high_tiles > low_tiles {
                            low_tiles + (b as usize % (high_tiles - low_tiles + 1))
                        } else {
                            low_tiles
                        };
                        let mut tries = 0u32;
                        let reached = loop {
                            game_state.reset_and_draw_tiles_double_ended(&game_config, &mut rng);
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
                        value_board(
                            &mut move_generator,
                            &game_state,
                            &mut rng,
                            b == 0,
                            verify && b == 0,
                        );
                    }
                }
            });
        }
    });

    let (accum_sum, accum_cnt, _, _) = shared.into_inner().unwrap();

    // mean-center on the empty leave (its entering-equity is the average best
    // full-rack equity = the baseline), then write (leave, value_in_points).
    let baseline = if accum_cnt[empty_rank] > 0 {
        accum_sum[empty_rank] / accum_cnt[empty_rank] as f64
    } else {
        0.0
    };
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
        if accum_cnt[idx] == 0 {
            continue;
        }
        lat.unrank_into(idx, &mut tally_buf);
        let size: usize = tally_buf.iter().map(|&c| c as usize).sum();
        if size == 0 || size > max_keep {
            continue; // skip empty (baseline), over-size, and (non-full) full racks.
        }
        let centered_points =
            (accum_sum[idx] / accum_cnt[idx] as f64 - baseline) / equity::SCALE as f64;
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
