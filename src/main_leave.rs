// Copyright (C) 2020-2026 Andy Kurnia.

use rand::prelude::*;
use std::fmt::Write;
use std::io::Write as _;
use std::str::FromStr;
use wolges::{
    alphabet, bites, board_layout, build, census, display, equity, error, fash, game_config,
    game_state, klv, kwg, move_filter, move_picker, movegen, play_scorer, prob, stats, win_pct,
};

mod game_args;

static BASE62: &[u8; 62] = b"\
0123456789\
ABCDEFGHIJKLMNOPQRSTUVWXYZ\
abcdefghijklmnopqrstuvwxyz\
";

static USED_STDOUT: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

// support "-" to mean stdout.
#[inline]
fn make_writer(filename: &str) -> Result<Box<dyn std::io::Write>, std::io::Error> {
    Ok(if filename == "-" {
        USED_STDOUT.store(true, std::sync::atomic::Ordering::Relaxed);
        Box::new(std::io::stdout())
    } else {
        Box::new(std::fs::File::create(filename)?)
    })
}

#[inline]
fn run_stamp() -> String {
    let d = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    let ticks = (d.as_secs() << 16) | (d.subsec_nanos() as u64 * 65536 / 1_000_000_000);
    format!("{ticks:012x}")
}

#[inline]
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
                writeln!(
                    boxed_stdout_or_stderr(),
                    "warning: {desired} already exists; writing {buf} instead"
                )?;
                return Ok(buf);
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e),
        }
    }
    unreachable!()
}

// when using "-" as output filename, print things to stderr.
#[inline]
fn boxed_stdout_or_stderr() -> Box<dyn std::io::Write> {
    if USED_STDOUT.load(std::sync::atomic::Ordering::Relaxed) {
        Box::new(std::io::stderr()) as Box<dyn std::io::Write>
    } else {
        Box::new(std::io::stdout())
    }
}

// support "-" to mean stdin.
#[inline]
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

#[inline]
fn refuse_a_wider_graph<N: kwg::Node>(
    kwg: &kwg::Kwg<N>,
    game_config: &game_config::GameConfig,
    kwg_path: &str,
) -> error::Returns<()> {
    let alphabet_len = game_config.alphabet().len();
    if kwg.fits_alphabet(alphabet_len) {
        return Ok(());
    }
    wolges::return_error!(format!(
        "{kwg_path} has tiles past this game's {alphabet_len}",
    ));
}

#[inline]
fn read_kwg<N: kwg::Node>(
    game_config: &game_config::GameConfig,
    path: &str,
) -> error::Returns<kwg::Kwg<N>> {
    let kwg = kwg::Kwg::<N>::from_bytes_alloc(&read_to_end(&mut make_reader(path)?)?);
    refuse_a_wider_graph(&kwg, game_config, path)?;
    Ok(kwg)
}

#[inline]
fn read_klv(
    game_config: &game_config::GameConfig,
    path: &str,
) -> error::Returns<klv::Klv<kwg::Node22>> {
    let klv = if path == "-" {
        klv::Klv::<kwg::Node22>::from_bytes_alloc(klv::EMPTY_KLV_BYTES)
    } else {
        klv::Klv::<kwg::Node22>::from_bytes_alloc(&std::fs::read(path)?)
    };
    game_config.check_leaves(klv.leave_range())?;
    Ok(klv)
}

type KlvPair = (
    std::sync::Arc<klv::Klv<kwg::Node22>>,
    std::sync::Arc<klv::Klv<kwg::Node22>>,
);

#[inline]
fn read_klv_pair(
    game_config: &game_config::GameConfig,
    path0: &str,
    path1: &str,
) -> error::Returns<KlvPair> {
    let klv0 = std::sync::Arc::new(read_klv(game_config, path0)?);
    let klv1 = if path1 == path0 {
        std::sync::Arc::clone(&klv0)
    } else {
        std::sync::Arc::new(read_klv(game_config, path1)?)
    };
    Ok((klv0, klv1))
}

#[derive(clap::Args)]
struct SelfPlay {
    #[arg(help = "the word graph (- for stdin)")]
    kwg: String,
    #[arg(default_value = "-", help = "player 1's leaves (- for none)")]
    leave0: String,
    #[arg(default_value = "-", help = "player 2's leaves (- for none)")]
    leave1: String,
    #[arg(default_value_t = 1_000_000)]
    games: u64,
    #[arg(
        default_value_t = 0,
        help = "keep playing until every rack has this many samples (summarizing tasks only)"
    )]
    min_samples: u64,
    #[arg(help = "prints the one it picks if omitted")]
    seed: Option<u64>,
}

#[derive(clap::Args)]
struct Gilles {
    #[arg(help = "the word graph (- for stdin)")]
    kwg: String,
    #[arg(default_value = "-", help = "player 1's leaves (- for none)")]
    leave0: String,
    #[arg(default_value = "-", help = "player 2's leaves (- for none)")]
    leave1: String,
    #[arg(default_value_t = 1_000_000)]
    games: u64,
    #[arg(help = "prints the one it picks if omitted")]
    seed: Option<u64>,
}

#[derive(clap::Args)]
struct Census {
    #[arg(help = "the word graph (- for stdin)")]
    kwg: String,
    #[arg(default_value = "-", help = "player 1's leaves (- for none)")]
    leave0: String,
    #[arg(default_value = "-", help = "player 2's leaves (- for none)")]
    leave1: String,
    #[arg(
        default_value = "500",
        help = "boards per generation, such as 3x256,2048"
    )]
    boards: String,
    #[arg(help = "prints the one it picks if omitted")]
    seed: Option<u64>,
    #[arg(
        long,
        value_name = "SNAPSHOT",
        help = "continue from this census-gen-<stamp>-<generation>.klv2"
    )]
    resume: Option<String>,
    #[arg(
        long,
        help = "also write the full-length leaves (what dynamic leaves read)"
    )]
    full: bool,
}

#[derive(clap::Args)]
struct Compare {
    #[arg(help = "the word graph (- for stdin)")]
    kwg: String,
    #[arg(default_value = "-", help = "player 1's leaves (- for none)")]
    klv0: String,
    #[arg(default_value = "-", help = "player 2's leaves (- for none)")]
    klv1: String,
    #[arg(default_value_t = 10_000)]
    pairs: u64,
    #[arg(help = "prints the one it picks if omitted")]
    seed: Option<u64>,
}

#[derive(clap::Args)]
struct SimCompare {
    #[arg(help = "the word graph (- for stdin)")]
    kwg: String,
    #[arg(default_value = "-", help = "the leaves (- for none)")]
    klv: String,
    #[arg(default_value_t = 1_000)]
    pairs: u64,
    #[arg(help = "prints the one it picks if omitted")]
    seed: Option<u64>,
    #[arg(long, default_value_t = move_picker::DEFAULT_NUM_SIM_ITERS, help = "rollouts a move")]
    iters: u64,
    #[arg(long, default_value = "1", help = "rollout threads a seat")]
    sim_threads: std::num::NonZeroUsize,
    #[arg(
        long,
        value_enum,
        default_value_t = SeatPicker::Simmer,
        help = "how seat p0 picks its moves"
    )]
    p0_picker: SeatPicker,
    #[arg(
        long,
        value_enum,
        default_value_t = SeatPicker::Simmer,
        help = "how seat p1 picks its moves"
    )]
    p1_picker: SeatPicker,
    #[arg(
        long,
        value_name = "TABLE",
        help = "seat p0 reads unfinished games' win chances from this win% table"
    )]
    p0_win_pct: Option<String>,
    #[arg(
        long,
        value_name = "TABLE",
        help = "seat p1 reads unfinished games' win chances from this win% table"
    )]
    p1_win_pct: Option<String>,
}

#[derive(clap::Args)]
struct SimStudyCheck {
    #[arg(help = "the word graph (- for stdin)")]
    kwg: String,
    #[arg(default_value = "-", help = "the leaves (- for none)")]
    klv: String,
    #[arg(default_value_t = 64)]
    iters: u64,
    #[arg(default_value_t = 1)]
    seed: u64,
}

#[derive(clap::Args)]
struct CensusBoard {
    #[arg(help = "the word graph (- for stdin)")]
    kwg: String,
    #[arg(help = "the leaves (- for none)")]
    klv: String,
    #[arg(help = "the board, as a fen")]
    board: String,
    #[arg(required = true, help = "the racks, such as AEINRST")]
    racks: Vec<String>,
}

#[derive(clap::Args)]
struct Dynaleaves {
    #[arg(help = "a --full leave table")]
    klv: String,
    #[arg(help = "the rack, such as AEINRST")]
    rack: String,
    #[arg(help = "the board, as a fen")]
    board: String,
}

#[derive(clap::Args)]
struct BoardStats {
    #[arg(help = "the fen file (- for stdin)")]
    boards: String,
}

#[derive(clap::Args)]
struct SimChunkCheck {
    #[arg(help = "the word graph (- for stdin)")]
    kwg: String,
    #[arg(default_value = "-", help = "the leaves (- for none)")]
    klv: String,
    #[arg(default_value_t = 128)]
    iters: u64,
    #[arg(default_value_t = 1)]
    seed: u64,
    #[arg(default_value_t = 8)]
    trials: u32,
}

#[derive(clap::Args)]
struct SimMutateCheck {
    #[arg(help = "the word graph (- for stdin)")]
    kwg: String,
    #[arg(default_value = "-", help = "the leaves (- for none)")]
    klv: String,
    #[arg(default_value_t = 96)]
    iters: u64,
    #[arg(default_value_t = 1)]
    seed: u64,
}

#[derive(clap::Args)]
struct SimFieldCheck {
    #[arg(help = "the word graph (- for stdin)")]
    kwg: String,
    #[arg(default_value = "-", help = "the leaves (- for none)")]
    klv: String,
    #[arg(default_value_t = 1)]
    seed: u64,
    #[arg(default_value_t = 12)]
    turns: u32,
}

#[derive(clap::Args)]
struct Winpct {
    #[arg(help = "the word graph (- for stdin)")]
    kwg: String,
    #[arg(help = "the leaves both players use (- for none)")]
    leave: String,
    #[arg(help = "the raw sparse csv to write (- for stdout)")]
    out: String,
    #[arg(default_value_t = 1_000_000)]
    games: u64,
    #[arg(help = "prints the one it picks if omitted")]
    seed: Option<u64>,
}

#[derive(clap::Args)]
struct WinpctEval {
    #[arg(help = "the word graph (- for stdin)")]
    kwg: String,
    #[arg(help = "the leaves both players use (- for none)")]
    leave: String,
    #[arg(help = "the win% table to score")]
    table: String,
    #[arg(default_value_t = 1_000_000)]
    games: u64,
    #[arg(help = "prints the one it picks if omitted; use a held-out seed")]
    seed: Option<u64>,
}

#[derive(clap::Args)]
struct InOut {
    input: String,
    output: String,
}

#[derive(clap::Args)]
struct Generate {
    summary: String,
    leaves: String,
}

#[derive(clap::Args)]
struct Playability {
    #[arg(help = "the word graph (- for stdin)")]
    kwg: String,
    #[arg(default_value = "-", help = "the leaves (- for none)")]
    leave: String,
    #[arg(default_value_t = 1_000_000)]
    games: u64,
    #[arg(help = "prints the one it picks if omitted")]
    seed: Option<u64>,
}

#[derive(clap::Subcommand)]
enum Task {
    #[command(about = "autoplay games, logging to a pair of csv")]
    Autoplay(SelfPlay),
    #[command(about = "autoplay and also save the summary")]
    AutoplaySummarize(SelfPlay),
    #[command(about = "autoplay and save only the summary")]
    AutoplaySummarizeOnly(SelfPlay),
    #[command(about = "GillesB board sampling, summarized as autoplay-summarize does")]
    Gilles(Gilles),
    #[command(about = "census leave generation")]
    Census(Census),
    #[command(about = "play game pairs to compare two sets of leaves")]
    Compare(Compare),
    #[command(about = "play game pairs where both seats choose moves by the simmer")]
    SimCompare(SimCompare),
    #[command(about = "check that a resumed decision matches the same decision run in one call")]
    SimStudyCheck(SimStudyCheck),
    #[command(
        about = "census one board given as fen: the best equity each listed rack could get on it; no sampling, no coverage, no mover's rack"
    )]
    CensusBoard(CensusBoard),
    #[command(
        about = "what the pool reweight does to one rack on one board: the static and dynamic value of every kept subrack"
    )]
    Dynaleaves(Dynaleaves),
    #[command(
        about = "shape of the boards in a fen file: tiles, words, mean word length, perimeter per tile, radius of gyration, holes"
    )]
    BoardStats(BoardStats),
    #[command(
        about = "check that cutting one decision into randomly sized chunks of resume calls gives the same leaderboard as running it in one call"
    )]
    SimChunkCheck(SimChunkCheck),
    #[command(
        about = "check that a candidate retired by hand or by the pruner keeps its statistics through a readmit"
    )]
    SimMutateCheck(SimMutateCheck),
    #[command(
        about = "check that the simmer's field still holds a move that places no tiles once the board has filled and static equity has buried the exchanges"
    )]
    SimFieldCheck(SimFieldCheck),
    #[command(about = "record an empirical win% table from Hasty self-play")]
    Winpct(Winpct),
    #[command(about = "score a win% table and the simmer's sigmoid by Brier (lower is better)")]
    WinpctEval(WinpctEval),
    #[command(about = "combine winpct raw tables by summing their histograms")]
    WinpctCombine(InOut),
    #[command(about = "summarize a log")]
    Summarize(InOut),
    #[command(about = "combine summaries into one and recompute totals")]
    Resummarize(InOut),
    #[command(about = "resummarize, sorted by length first")]
    ResummarizePlayability(InOut),
    #[command(about = "resummarize, sorted by playability first")]
    ResummarizePlayabilityAll(InOut),
    #[command(about = "generate leaves up to rack_size - 1")]
    Generate(Generate),
    #[command(about = "generate leaves up to rack_size")]
    GenerateFull(Generate),
    #[command(about = "autoplay and record prorated found best words")]
    Playability(Playability),
}

impl Task {
    // the tasks that play two seats against each other
    #[inline]
    fn needs_two_players(&self) -> bool {
        matches!(
            self,
            Task::Compare(_) | Task::SimCompare(_) | Task::Winpct(_) | Task::WinpctEval(_)
        )
    }
}

// leave = listing extrapolated accumulated values empirically
#[derive(clap::Parser)]
#[command(
    about = "leave = listing extrapolated accumulated values empirically",
    after_help = "input/output files can be \"-\" (not advisable for binary files).
for autoplay only the kwg can come from \"-\".
when low disk space, note that in bash:
  leave autoplay ... 1000
  leave summarize log1 summary1.csv
  leave autoplay ... 1000
  leave summarize log2 summary2.csv
  leave resummarize <( cat summary1.csv summary2.csv ) summary.csv
  leave generate summary.csv leaves.csv
    is the same as
  leave autoplay ... 1000
  leave summarize log1 summary1.csv
  leave autoplay ... 1000
  leave summarize log2 summary2.csv
  leave generate <( cat summary1.csv summary2.csv ) leaves.csv
    which is the same as
  leave autoplay ... 1000
  leave autoplay ... 1000
  leave summarize <( cat log1 log2 ) summary.csv
  leave generate summary.csv leaves.csv
    but it becomes possible to remove log1 to free up disk space for log2.
    using resummarize also allows removing summary1.csv earlier."
)]
struct Cli {
    #[arg(long, help = "the word graph is a kbwg")]
    kbwg: bool,
    #[arg(long, help = "worker threads [default: every core]")]
    threads: Option<std::num::NonZeroUsize>,
    #[command(subcommand)]
    task: Task,
    #[command(flatten)]
    game: game_args::GameArgs,
}

#[inline]
fn run<N: kwg::Node + Sync + Send>(
    task: Task,
    game_config: game_config::GameConfig,
    threads: usize,
) -> error::Returns<()> {
    match task {
        Task::Autoplay(a) => {
            let kwg = read_kwg::<N>(&game_config, &a.kwg)?;
            let (klv0, klv1) = read_klv_pair(&game_config, &a.leave0, &a.leave1)?;
            generate_autoplay_logs::<true, false, _, _>(
                game_config,
                kwg,
                klv0,
                klv1,
                SelfPlayParams {
                    num_games: a.games,
                    min_samples: a.min_samples,
                    seed: a.seed,
                    threads,
                },
            )
        }
        Task::AutoplaySummarize(a) => {
            let kwg = read_kwg::<N>(&game_config, &a.kwg)?;
            let (klv0, klv1) = read_klv_pair(&game_config, &a.leave0, &a.leave1)?;
            generate_autoplay_logs::<true, true, _, _>(
                game_config,
                kwg,
                klv0,
                klv1,
                SelfPlayParams {
                    num_games: a.games,
                    min_samples: a.min_samples,
                    seed: a.seed,
                    threads,
                },
            )
        }
        Task::AutoplaySummarizeOnly(a) => {
            let kwg = read_kwg::<N>(&game_config, &a.kwg)?;
            let (klv0, klv1) = read_klv_pair(&game_config, &a.leave0, &a.leave1)?;
            generate_autoplay_logs::<false, true, _, _>(
                game_config,
                kwg,
                klv0,
                klv1,
                SelfPlayParams {
                    num_games: a.games,
                    min_samples: a.min_samples,
                    seed: a.seed,
                    threads,
                },
            )
        }
        Task::Gilles(a) => {
            let kwg = read_kwg::<N>(&game_config, &a.kwg)?;
            let (klv0, klv1) = read_klv_pair(&game_config, &a.leave0, &a.leave1)?;
            generate_gilles_summary(game_config, kwg, klv0, klv1, a.games, a.seed, threads)
        }
        Task::Census(a) => {
            let board_counts = parse_board_counts(&a.boards)?;
            let kwg = read_kwg::<N>(&game_config, &a.kwg)?;
            let (klv0, klv1) = read_klv_pair(&game_config, &a.leave0, &a.leave1)?;
            generate_census_leaves(
                game_config,
                kwg,
                klv0,
                klv1,
                CensusParams {
                    board_counts,
                    seed: a.seed,
                    threads,
                    resume: a.resume,
                    full: a.full,
                },
            )
        }
        Task::Compare(a) => {
            let kwg = read_kwg::<N>(&game_config, &a.kwg)?;
            let (klv0, klv1) = read_klv_pair(&game_config, &a.klv0, &a.klv1)?;
            compare_leaves(game_config, kwg, klv0, klv1, a.pairs, a.seed, threads)
        }
        Task::SimCompare(a) => {
            let klv = std::sync::Arc::new(read_klv(&game_config, &a.klv)?);
            let kwg = read_kwg::<N>(&game_config, &a.kwg)?;
            sim_compare(
                game_config,
                kwg,
                klv,
                SimCompareParams {
                    num_game_pairs: a.pairs,
                    seed: a.seed,
                    threads,
                    num_sim_iters: a.iters,
                    sim_threads: a.sim_threads.get(),
                    p0_picker: a.p0_picker,
                    p1_picker: a.p1_picker,
                    p0_win_pct: a.p0_win_pct,
                    p1_win_pct: a.p1_win_pct,
                },
            )
        }
        Task::SimStudyCheck(a) => {
            let klv = read_klv(&game_config, &a.klv)?;
            let kwg = read_kwg::<N>(&game_config, &a.kwg)?;
            let (iters, seed) = (a.iters, a.seed);
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
            let mut driver = move_picker::Simmer::<_, _, false>::new(
                &game_config,
                &kwg,
                &klv,
                move_picker::SimmerParams {
                    num_sim_iters: iters,
                    sim_threads: 1,
                    win_pct_table: None,
                },
            );
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
                Ok(())
            } else {
                wolges::return_error!(
                    "resume mismatch: split decision differs from one-shot".to_string()
                )
            }
        }
        Task::CensusBoard(a) => {
            let kwg = read_kwg::<N>(&game_config, &a.kwg)?;
            let klv = read_klv(&game_config, &a.klv)?;
            census_one_board(game_config, kwg, klv, &a.board, &a.racks)
        }
        Task::Dynaleaves(a) => {
            let klv = read_klv(&game_config, &a.klv)?;
            inspect_dynamic_leaves(game_config, klv, &a.rack, &a.board, threads)
        }
        Task::BoardStats(a) => board_stats(game_config, &mut make_reader(&a.boards)?),
        Task::SimChunkCheck(a) => {
            let klv = read_klv(&game_config, &a.klv)?;
            let kwg = read_kwg::<N>(&game_config, &a.kwg)?;
            sim_chunk_check(game_config, kwg, klv, a.iters, a.seed, a.trials)
        }
        Task::SimMutateCheck(a) => {
            let klv = read_klv(&game_config, &a.klv)?;
            let kwg = read_kwg::<N>(&game_config, &a.kwg)?;
            let (iters, seed) = (a.iters, a.seed);
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
            let mut driver = move_picker::Simmer::<_, _, false>::new(
                &game_config,
                &kwg,
                &klv,
                move_picker::SimmerParams {
                    num_sim_iters: iters,
                    sim_threads: 1,
                    win_pct_table: None,
                },
            );
            driver.reseed(seed);
            let opening = (iters / 4).max(1);
            driver.begin_decision(&move_generator, &game_state, opening);
            {
                let by_hand = driver
                    .active_stream_ids()
                    .next()
                    .ok_or("no active candidates to retire")?;
                let at_retire = driver.stream_count(by_hand).unwrap();
                let retired_by_hand = driver.retire_stream(by_hand);
                let now_retired = driver.retired_stream_ids().any(|id| id == by_hand);
                let still_active = driver.active_stream_ids().any(|id| id == by_hand);
                let kept = driver.stream_count(by_hand).unwrap();
                println!(
                    "retire stream {by_hand}: count {at_retire} -> {kept}, retired={now_retired} active={still_active}",
                );
                if !retired_by_hand || !now_retired || still_active || kept != at_retire {
                    wolges::return_error!(
                        "hand retire did not move the candidate intact".to_string()
                    );
                }
                if !driver.readmit_with_history(by_hand)
                    || driver.stream_count(by_hand).unwrap() != at_retire
                {
                    wolges::return_error!(
                        "readmit after a hand retire lost the statistics".to_string()
                    );
                }
            }
            driver.resume(&move_generator, iters.saturating_sub(opening));
            let pruned_id = driver.retired_stream_ids().next();
            match pruned_id {
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
                        Ok(())
                    } else {
                        wolges::return_error!("readmit dropped history: count reset".to_string())
                    }
                }
            }
        }
        Task::SimFieldCheck(a) => {
            let klv = read_klv(&game_config, &a.klv)?;
            let kwg = read_kwg::<N>(&game_config, &a.kwg)?;
            sim_field_check(game_config, kwg, klv, a.seed, a.turns)
        }
        Task::Winpct(a) => {
            let kwg = read_kwg::<N>(&game_config, &a.kwg)?;
            let klv = std::sync::Arc::new(read_klv(&game_config, &a.leave)?);
            generate_winpct_table(game_config, kwg, klv, &a.out, a.games, a.seed, threads)
        }
        Task::WinpctEval(a) => {
            let kwg = read_kwg::<N>(&game_config, &a.kwg)?;
            let klv = std::sync::Arc::new(read_klv(&game_config, &a.leave)?);
            let table = win_pct::WinPctTable::from_csv(make_reader(&a.table)?)?;
            generate_winpct_eval(game_config, kwg, klv, table, a.games, a.seed, threads)
        }
        Task::WinpctCombine(a) => {
            let acc = win_pct::WinPctAccumulator::from_csv(make_reader(&a.input)?)?;
            acc.to_csv(make_writer(&a.output)?)
        }
        Task::Summarize(a) => generate_summary(
            game_config,
            make_reader(&a.input)?,
            csv::Writer::from_writer(make_writer(&a.output)?),
        ),
        Task::Resummarize(a) => resummarize_summaries::<'a', _, _>(
            game_config,
            csv::ReaderBuilder::new()
                .has_headers(false)
                .from_reader(make_reader(&a.input)?),
            csv::Writer::from_writer(make_writer(&a.output)?),
        ),
        Task::ResummarizePlayability(a) => resummarize_summaries::<'p', _, _>(
            game_config,
            csv::ReaderBuilder::new()
                .has_headers(false)
                .from_reader(make_reader(&a.input)?),
            csv::Writer::from_writer(make_writer(&a.output)?),
        ),
        Task::ResummarizePlayabilityAll(a) => resummarize_summaries::<'P', _, _>(
            game_config,
            csv::ReaderBuilder::new()
                .has_headers(false)
                .from_reader(make_reader(&a.input)?),
            csv::Writer::from_writer(make_writer(&a.output)?),
        ),
        Task::Generate(a) => generate_leaves::<_, _, false>(
            game_config,
            csv::ReaderBuilder::new()
                .has_headers(false)
                .from_reader(make_reader(&a.summary)?),
            csv::Writer::from_writer(make_writer(&a.leaves)?),
        ),
        Task::GenerateFull(a) => generate_leaves::<_, _, true>(
            game_config,
            csv::ReaderBuilder::new()
                .has_headers(false)
                .from_reader(make_reader(&a.summary)?),
            csv::Writer::from_writer(make_writer(&a.leaves)?),
        ),
        Task::Playability(a) => {
            let kwg = read_kwg::<N>(&game_config, &a.kwg)?;
            let klv = read_klv(&game_config, &a.leave)?;
            discover_playability(game_config, kwg, klv, a.games, a.seed, threads)
        }
    }
}

fn main() -> error::Returns<()> {
    let cli: Cli = clap::Parser::parse();
    let t0 = std::time::Instant::now();
    let game_config = cli.game.make_game_config()?;
    if cli.task.needs_two_players() && game_config.num_players() != 2 {
        wolges::return_error!("this task needs exactly 2 players".to_string());
    }
    let threads = cli
        .threads
        .map_or_else(num_cpus::get, std::num::NonZeroUsize::get);
    if cli.kbwg {
        run::<kwg::Node24>(cli.task, game_config, threads)?;
    } else {
        run::<kwg::Node22>(cli.task, game_config, threads)?;
    }
    writeln!(boxed_stdout_or_stderr(), "time taken: {:?}", t0.elapsed())?;
    Ok(())
}

struct SelfPlayParams {
    num_games: u64,
    min_samples: u64,
    seed: Option<u64>,
    threads: usize,
}

#[inline]
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
    SelfPlayParams {
        num_games,
        min_samples: min_samples_per_rack,
        seed,
        threads,
    }: SelfPlayParams,
) -> error::Returns<()> {
    if !SUMMARIZE && min_samples_per_rack != 0 {
        return Err("min_samples_per_rack requires summarize".into());
    }

    let game_config = std::sync::Arc::new(game_config);
    let kwg = std::sync::Arc::new(kwg);
    let player_aliases = std::sync::Arc::new(
        (1..=game_config.num_players())
            .map(|x| format!("p{x}"))
            .collect::<Box<[String]>>(),
    );
    let seed = seed.unwrap_or_else(rand::random);
    writeln!(boxed_stdout_or_stderr(), "seed: {seed}")?;
    let num_threads = threads;

    let num_processed_games = std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0));

    let run_identifier = std::sync::Arc::new(format!("log-{}", run_stamp()));
    writeln!(boxed_stdout_or_stderr(), "logging to {run_identifier}")?;
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
        undersampled_racks: Vec<bites::Bites>,
        undersampled_generation: u64,
        undersampling_comment: String,
        tick_periods: move_picker::Periods,
    }
    let mutexed_stuffs = std::sync::Arc::new(std::sync::Mutex::new(MutexedStuffs {
        csv_game_writer,
        csv_log_writer,
        full_rack_map,
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

                let mut final_scores_pts = vec![0; game_config.num_players() as usize];
                let mut num_bingos = vec![0; game_config.num_players() as usize];
                let mut num_turns = vec![0; game_config.num_players() as usize];
                let mut num_moves;
                let mut num_batched_games_here = 0;
                let mut batched_csv_log = csv::Writer::from_writer(Vec::new());
                let mut batched_csv_game = csv::Writer::from_writer(Vec::new());
                let mut thread_full_rack_map = fash::MyHashMap::<bites::Bites, Cumulate>::default();

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

                let mut sample_rack_buf = if SUMMARIZE && min_samples_per_rack != 0 {
                    Vec::with_capacity(game_config.rack_size() as usize)
                } else {
                    Vec::new()
                };
                let mut undersampled_thread_racks = Vec::<bites::Bites>::new();

                let mut full_rack_tally = if SUMMARIZE && min_samples_per_rack != 0 {
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
                                        let deficit = recompute_undersampled_racks(
                                            &thread_full_rack_map,
                                            &mut mutex_guard.undersampled_racks,
                                            RecomputeScratch {
                                                full_rack_tally: &mut full_rack_tally,
                                                alphabet_freqs: &mut alphabet_freqs,
                                                exchange_buffer: &mut exchange_buffer,
                                            },
                                            game_config.rack_size(),
                                            min_samples_per_rack,
                                        );
                                        std::mem::swap(
                                            &mut thread_full_rack_map,
                                            &mut mutex_guard.full_rack_map,
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

                            let current_generation = undersampling_remediation_generation_id
                                .load(std::sync::atomic::Ordering::Relaxed);
                            if mutex_guard.undersampled_generation != current_generation {
                                std::mem::swap(
                                    &mut thread_full_rack_map,
                                    &mut mutex_guard.full_rack_map,
                                );
                                let deficit = recompute_undersampled_racks(
                                    &thread_full_rack_map,
                                    &mut mutex_guard.undersampled_racks,
                                    RecomputeScratch {
                                        full_rack_tally: &mut full_rack_tally,
                                        alphabet_freqs: &mut alphabet_freqs,
                                        exchange_buffer: &mut exchange_buffer,
                                    },
                                    game_config.rack_size(),
                                    min_samples_per_rack,
                                );
                                std::mem::swap(
                                    &mut thread_full_rack_map,
                                    &mut mutex_guard.full_rack_map,
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

                        if SUMMARIZE && old_bag_len > 0 && !undersampled_thread_racks.is_empty() {
                            let chosen_undersampled_thread_rack_index =
                                rng.random_range(0..undersampled_thread_racks.len());

                            sample_rack_buf.clear();
                            sample_rack_buf.extend_from_slice(
                                &undersampled_thread_racks[chosen_undersampled_thread_rack_index],
                            );

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

                            let rounded_equity = play.equity.as_f64();
                            pool_one(&mut thread_full_rack_map, &sample_rack_buf[..], rounded_equity);
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
                            dynamic_leaves: None,
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

                            let rounded_equity = play.equity.as_f64();
                            pool_one(&mut thread_full_rack_map, &cur_rack_as_vec[..], rounded_equity);
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
                                            write!(boxed_stdout_or_stderr(),
                                                "After {elapsed_time_secs} seconds, have logged {logged_games} games ({completed_moves} moves)").ok();
                                            if !mutex_guard.undersampling_comment.is_empty() {
                                                write!(boxed_stdout_or_stderr(), "{}", mutex_guard.undersampling_comment).ok();
                                                let num_todo = undersampling_remediation_countdown
                                                    .load(std::sync::atomic::Ordering::Relaxed);
                                                if num_todo > 0 {
                                                    write!(boxed_stdout_or_stderr(), " (to do: {num_todo})").ok();
                                                }
                                            }
                                            writeln!(boxed_stdout_or_stderr(), " into {run_identifier}").ok();
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
                }
            }));
        }

        for thread in threads {
            if let Err(e) = thread.join() {
                writeln!(boxed_stdout_or_stderr(), "{e:?}").ok();
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

        writeln!(
            boxed_stdout_or_stderr(),
            "{} records, {} unique racks",
            row_count,
            full_rack_map.len()
        )?;

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
    }

    writeln!(
        boxed_stdout_or_stderr(),
        "After {} seconds, have logged {} games ({} moves) into {}",
        t0.elapsed().as_secs(),
        completed_games.load(std::sync::atomic::Ordering::Relaxed),
        completed_moves.load(std::sync::atomic::Ordering::Relaxed),
        run_identifier
    )?;

    Ok(())
}

#[inline]
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

#[inline]
fn generate_gilles_summary<N: kwg::Node + Sync + Send, L: kwg::Node + Sync + Send>(
    game_config: game_config::GameConfig,
    kwg: kwg::Kwg<N>,
    arc_klv0: std::sync::Arc<klv::Klv<L>>,
    arc_klv1: std::sync::Arc<klv::Klv<L>>,
    num_games: u64,
    seed: Option<u64>,
    threads: usize,
) -> error::Returns<()> {
    let game_config = std::sync::Arc::new(game_config);
    let kwg = std::sync::Arc::new(kwg);
    let seed = seed.unwrap_or_else(rand::random);
    writeln!(boxed_stdout_or_stderr(), "seed: {seed}")?;
    let num_threads = threads;

    let run_identifier = format!("gilles-summary-{}", run_stamp());

    let rack_size = game_config.rack_size();
    let num_tiles: u32 = {
        let alphabet = game_config.alphabet();
        (0..alphabet.len()).map(|t| alphabet.freq(t) as u32).sum()
    };

    let pool_min = (num_tiles / 4) as usize;
    let pool_max = (num_tiles as usize).saturating_sub(pool_min);
    let group_size = (2 * rack_size as usize).saturating_sub(1);
    let num_draws = 10;
    let turn_stride = 3u32;

    writeln!(
        boxed_stdout_or_stderr(),
        "gilles: rack_size={rack_size} num_tiles={num_tiles} snapshot_pool={pool_min}..={pool_max} group_size={group_size} draws={num_draws} stride={turn_stride}"
    )?;

    let num_processed_games = std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0));
    let completed_games = std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0));
    let completed_samples = std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0));

    let mutexed = std::sync::Arc::new(std::sync::Mutex::new(GillesMutexed {
        full_rack_map: fash::MyHashMap::<bites::Bites, Cumulate>::default(),
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

                let mut cand_tally = vec![0u8; num_letters];
                let mut best_group_tally = vec![0u8; num_letters];
                let mut rack_tally = vec![0u8; num_letters];
                let mut unseen_pool = Vec::<u8>::new();
                let mut exchange_buffer = Vec::with_capacity(rack_size as usize);
                let mut thread_map = fash::MyHashMap::<bites::Bites, Cumulate>::default();
                let mut final_scores = vec![0; game_config.num_players() as usize];

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
                    if num_prior_games >= num_games {
                        num_processed_games.fetch_sub(1, std::sync::atomic::Ordering::Relaxed);
                        break;
                    }

                    rng.set_stream(num_prior_games);
                    game_state.reset_and_draw_tiles_double_ended(&game_config, &mut rng);

                    let mut turn_idx = 0u32;
                    let mut base_turn: Option<u32> = None;
                    loop {
                        let board_tiles_count =
                            game_state.board_tiles.iter().filter(|&&t| t != 0).count();

                        let pool_count = (num_tiles as usize).saturating_sub(board_tiles_count);

                        if pool_count >= pool_min
                            && pool_count <= pool_max
                            && board_tiles_count > 0
                            && !game_state.bag.is_empty()
                            && (turn_idx - *base_turn.get_or_insert(turn_idx))
                                .is_multiple_of(turn_stride)
                        {
                            let group_src: &[u8] = &base_freqs;
                            let num_unseen = group_src.iter().map(|&c| c as usize).sum::<usize>();
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
                                            lnp += ln_choose(group_src[tile] as usize, k as usize);
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
                                        let equity = move_generator.plays[0].equity.as_f64();
                                        pool_one(thread_map, rack_bytes, equity);
                                        completed_samples
                                            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                                    },
                                    rack_tally: &mut rack_tally,
                                    min_len: rack_size,
                                    max_len: rack_size,
                                    exchange_buffer: &mut exchange_buffer,
                                });
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
                        move_generator.gen_moves_unfiltered(&movegen::GenMovesParams {
                            board_snapshot: &board_snapshot,
                            rack: &game_state.current_player().rack,
                            max_gen: 1,
                            num_exchanges_by_this_player: game_state.current_player().num_exchanges,
                            pass_policy: movegen::PassPolicy::OnlyWhenForced,
                            dynamic_leaves: None,
                        });

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
                        writeln!(
                            boxed_stdout_or_stderr(),
                            "After {elapsed} seconds, {} games, {} samples into {run_identifier}",
                            completed_games.load(std::sync::atomic::Ordering::Relaxed),
                            completed_samples.load(std::sync::atomic::Ordering::Relaxed),
                        )
                        .ok();
                    }
                }

                let mut g = mutexed.lock().unwrap();
                merge_rack_map(&mut g.full_rack_map, &mut thread_map);
            }));
        }
        for thread in threads {
            if let Err(e) = thread.join() {
                writeln!(boxed_stdout_or_stderr(), "{e:?}").ok();
            }
        }
    });

    let g = mutexed.lock().unwrap();
    let map = &g.full_rack_map;
    let mut total_equity = 0.0;
    let mut row_count = 0u64;
    for v in map.values() {
        total_equity += v.equity;
        row_count += v.count;
    }
    writeln!(
        boxed_stdout_or_stderr(),
        "{} records, {} unique racks",
        row_count,
        map.len()
    )?;
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
    writeln!(
        boxed_stdout_or_stderr(),
        "After {} seconds, {} games, {} samples into {run_identifier}",
        t0.elapsed().as_secs(),
        completed_games.load(std::sync::atomic::Ordering::Relaxed),
        completed_samples.load(std::sync::atomic::Ordering::Relaxed),
    )?;

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

#[inline]
fn pool_one(map: &mut fash::MyHashMap<bites::Bites, Cumulate>, key: &[u8], equity: f64) {
    map.entry(key.into())
        .and_modify(|v| {
            v.equity += equity;
            v.count += 1;
        })
        .or_insert_with(|| Cumulate { equity, count: 1 });
}

struct GillesMutexed {
    full_rack_map: fash::MyHashMap<bites::Bites, Cumulate>,
}

#[inline]
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

struct RecomputeScratch<'a> {
    full_rack_tally: &'a mut [u8],
    alphabet_freqs: &'a mut [u8],
    exchange_buffer: &'a mut Vec<u8>,
}

#[inline]
fn recompute_undersampled_racks(
    full_rack_map: &fash::MyHashMap<bites::Bites, Cumulate>,
    undersampled: &mut Vec<bites::Bites>,
    scratch: RecomputeScratch<'_>,
    rack_size: u8,
    min_samples: u64,
) -> u64 {
    let RecomputeScratch {
        full_rack_tally,
        alphabet_freqs,
        exchange_buffer,
    } = scratch;

    undersampled.clear();
    if min_samples == 0 {
        return 0;
    }
    full_rack_tally.copy_from_slice(alphabet_freqs);
    let frozen_freq: &[u8] = full_rack_tally;
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
    remaining
}

#[inline]
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

#[inline]
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

#[inline]
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
}

#[inline]
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
    } = pool;
    movegen_rack.clear();
    for (t, &c) in unseen_tally.iter().enumerate() {
        for _ in 0..(c as usize).min(rack_size) {
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
    move_generator.gen_census_sheet(
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
    n_cand
}

type SheetCacheSlot = std::sync::Mutex<Option<(Vec<i32>, Vec<u8>)>>;

#[inline]
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

#[inline]
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
    make_writer(path)?.write_all(&bin)?;
    Ok(leave_values.len())
}

struct CensusParams {
    board_counts: Vec<u64>,
    seed: Option<u64>,
    threads: usize,
    resume: Option<String>,
    full: bool,
}

#[inline]
fn generate_census_leaves<N: kwg::Node + Sync + Send, L: kwg::Node + Sync + Send>(
    game_config: game_config::GameConfig,
    kwg: kwg::Kwg<N>,
    arc_klv0: std::sync::Arc<klv::Klv<L>>,
    arc_klv1: std::sync::Arc<klv::Klv<L>>,
    CensusParams {
        board_counts,
        seed,
        threads,
        resume,
        full,
    }: CensusParams,
) -> error::Returns<()> {
    let t0 = std::time::Instant::now();
    let alphabet = game_config.alphabet();
    let num_letters = alphabet.len() as usize;
    let rack_size = game_config.rack_size() as usize;
    let num_tiles: usize = (0..alphabet.len()).map(|t| alphabet.freq(t) as usize).sum();
    let racks_tiles = game_config.num_players() as usize * rack_size;

    let pool_max = num_tiles.saturating_sub(racks_tiles);
    let pool_min = racks_tiles + 1;
    let low_tiles = num_tiles.saturating_sub(pool_max);
    let high_tiles = num_tiles.saturating_sub(pool_min);

    let gens = board_counts.len();

    let max_boards = board_counts.iter().copied().max().unwrap_or(1).max(1);
    let multigen = gens > 1;

    let sheet_reuse = multigen;

    let (live_after, sheet_cache_len) = census_sheet_reuse_plan(&board_counts);

    let sheet_cache_len = if sheet_reuse { sheet_cache_len } else { 0 };

    let lat = census::MultisetLattice::new(num_letters, rack_size);
    let empty_rank = lat.rank(&vec![0u8; num_letters]) as usize;
    writeln!(
        boxed_stdout_or_stderr(),
        "census: lattice {} leaves (letters {num_letters}, rack_size {rack_size}), \
         window [{low_tiles},{high_tiles}] of {num_tiles} tiles",
        lat.len(),
    )?;

    let add_table = {
        let t = std::time::Instant::now();
        let at = census::AddTable::new_with_threads(&lat, threads);
        writeln!(
            boxed_stdout_or_stderr(),
            "census: add-table {} rows x {num_letters} letters built in {:?}",
            lat.full_rack_start(),
            t.elapsed(),
        )?;
        at
    };

    let zeta_pool_min = 36;

    let scatter = lat.len() <= 12_000_000;

    let base_freqs: Vec<u8> = (0..alphabet.len()).map(|t| alphabet.freq(t)).collect();

    let seed = seed.unwrap_or_else(rand::random);

    let mut leave_cur = vec![0i32; lat.len()];
    let mut tally_buf = vec![0u8; num_letters];
    for (idx, slot) in leave_cur.iter_mut().enumerate() {
        lat.unrank_into(idx, &mut tally_buf);
        *slot = arc_klv0.leave_value_from_tally(&tally_buf);
    }

    let (start_gen, census_run_epoch) = if let Some(path) = resume {
        let name = std::path::Path::new(&path)
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let Some((rid, num)) = name
            .strip_prefix("census-gen-")
            .and_then(|r| r.strip_suffix(".klv2"))
            .and_then(|r| r.split_once('-'))
            .filter(|(rid, _)| u64::from_str_radix(rid, 16).is_ok())
            .and_then(|(rid, gg)| Some((rid.to_owned(), gg.parse::<usize>().ok()?)))
        else {
            wolges::return_error!(format!(
                "--resume wants a census-gen-<stamp>-<generation>.klv2 snapshot, got {path}"
            ))
        };
        let bytes = std::fs::read(&path)?;
        let resume_klv = klv::Klv::<L>::from_bytes_alloc(&bytes);
        for (idx, slot) in leave_cur.iter_mut().enumerate() {
            lat.unrank_into(idx, &mut tally_buf);
            *slot = resume_klv.leave_value_from_tally(&tally_buf);
        }
        writeln!(
            boxed_stdout_or_stderr(),
            "census: resuming from {path} (gen {num} done) -> starting gen {}",
            num + 1
        )?;
        (num, rid)
    } else {
        (0, run_stamp())
    };

    if start_gen >= gens {
        return Err(format!(
            "census resume: {start_gen} generation(s) already completed but the \
             spec has only {gens}; extend the board-count spec"
        )
        .into());
    }

    let leave_lock = std::sync::RwLock::new(leave_cur);

    let num_threads = threads.max(1).min(max_boards as usize);

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

    let shared = std::sync::Mutex::new((
        vec![0f64; lat_len],
        vec![0u64; lat_len],
        0u64,
        0u64,
        if multigen {
            vec![false; lat_len]
        } else {
            Vec::new()
        },
    ));

    let barrier = std::sync::Barrier::new(num_threads);

    let sheet_cache: Vec<SheetCacheSlot> = (0..sheet_cache_len)
        .map(|_| std::sync::Mutex::new(None))
        .collect();
    writeln!(
        boxed_stdout_or_stderr(),
        "census: {num_threads} threads over {board_counts:?} boards/gen"
    )?;

    std::thread::scope(|s| {
        for _ in 0..num_threads {
            s.spawn(|| {

                let mut game_state = game_state::GameState::new(&game_config);
                let mut move_generator = movegen::KurniaMoveGenerator::new(&game_config);
                let mut sheet = vec![census::UNPLAYABLE; lat_len];

                let mut blank_deltas = Vec::<(u8, i32)>::new();

                let mut contrib = vec![census::UNPLAYABLE; lat_len];

                let mut num_board = vec![0f64; lat_len];
                let mut den_board = vec![0f64; lat_len];

                let mut maxsheet = vec![0i32; lat_len];

                let mut unseen_tally = vec![0u8; num_letters];
                let mut movegen_rack = Vec::<u8>::new();
                let mut final_scores = vec![0; game_config.num_players() as usize];


                let mut value_board = |move_generator: &mut movegen::KurniaMoveGenerator,
                                       game_state: &game_state::GameState,
                                       leave: &[i32],
                                       null_leave: bool,
                                       log_first: bool,
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

                    let num_blanks_eff = (unseen_tally[0] as usize).min(rack_size);
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
                            unseen_tally: &unseen_tally,
                            num_blanks_eff,
                            rack_size,
                        },
                        &mut movegen_rack,
                        &mut blank_deltas,
                        &mut sheet,
                    );
                    if log_first {
                        writeln!(boxed_stdout_or_stderr(),
                            "  step1 sheet: {} tiles in pool -> {} candidate plays (unstored) in {:?}",
                            movegen_rack.len(),
                            n_cand,
                            ts.elapsed(),).ok();
                    }

                    if let Some(slot) = cache_slot {
                        *slot.lock().unwrap() = Some((sheet.clone(), unseen_tally.clone()));
                    }
                    } // end of the !reuse step-1 build branch


                    let ts = std::time::Instant::now();
                    num_board.iter_mut().for_each(|x| *x = 0.0);
                    den_board.iter_mut().for_each(|x| *x = 0.0);

                    let pool: usize = unseen_tally.iter().map(|&c| c as usize).sum();
                    census::apportion_fused(
                        &lat,
                        &add_table,
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
                    );
                    for (idx, slot) in contrib.iter_mut().enumerate() {
                        *slot = if den_board[idx] > 0.0 {
                            (num_board[idx] / den_board[idx]).round() as i32
                        } else {
                            census::UNPLAYABLE
                        };
                    }
                    if log_first {
                        writeln!(boxed_stdout_or_stderr(),
                            "  step3 full-rack: {:?}",
                            ts.elapsed(),).ok();
                    }


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
                    writeln!(boxed_stdout_or_stderr(),
                        "census: board {}/{} done ({}s), {} of {} leaves valued so far",
                        *completed,
                        cur_boards,
                        t0.elapsed().as_secs(),
                        *valued,
                        globally_possible_count,).ok();
                };


                let mut gen_idx = start_gen;

                let mut num_boards = board_counts[gen_idx];

                let mut prior_max_boards = 0usize;
                loop {

                    {
                        let leave = leave_lock.read().unwrap();

                        let null_leave = leave.iter().all(|&x| x == 0);
                        loop {
                            let b = next_board.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                            if b >= num_boards {
                                break;
                            }
                    let mut rng = rand::rngs::ChaCha20Rng::seed_from_u64(census_mix64(
                        seed.wrapping_add(census_mix64(b)),
                    ));

                    let reuse_board = sheet_reuse && (b as usize) < prior_max_boards;
                    if !reuse_board {

                    let target = if high_tiles <= low_tiles {
                        low_tiles
                    } else {

                        low_tiles + (b as usize % (high_tiles - low_tiles + 1))
                    };

                    let mut tries = 0u32;
                    let reached = loop {
                        game_state.reset_and_draw_tiles_double_ended(&game_config, &mut rng);
                        let mut got = false;
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
                                dynamic_leaves: None,
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
                        writeln!(boxed_stdout_or_stderr(),
                            "census: board slot {b} never reached window [{low_tiles},{high_tiles}]; skipping").ok();
                        continue;
                    }
                    } // end of the !reuse_board game replay
                    value_board(
                        &mut move_generator,
                        &game_state,
                        &leave,
                        null_leave,
                        b == 0,

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
                    }
                    }

                    if multigen {
                        if barrier.wait().is_leader() {

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
                                        lv[idx] = (sum[idx] / cnt[idx] as f64 - base)
                                            .round()
                                            as i32;
                                    }
                                }
                                writeln!(boxed_stdout_or_stderr(),
                                    "census: gen {}/{} done ({} of {} leaves valued)",
                                    gen_idx + 1,
                                    gens,
                                    *valued,
                                    lat_len,).ok();
                                if gen_idx + 1 < gens {

                                    for idx in 0..lat_len {
                                        sum[idx] = 0.0;
                                        cnt[idx] = 0;
                                    }
                                    *completed = 0;
                                    *valued = 0;
                                    next_board
                                        .store(0, std::sync::atomic::Ordering::Relaxed);
                                }
                            }

                            {
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
                                    Ok(nk) => { writeln!(boxed_stdout_or_stderr(),
     "census: persisted gen {} -> {p} ({nk} leaves)",
                                        gen_idx + 1).ok(); },
                                    Err(e) => { writeln!(boxed_stdout_or_stderr(),
     "census: gen {} klv2 persist failed: {e}",
                                        gen_idx + 1).ok(); },
                                }
                            }

                            for slot in sheet_cache.iter().skip(live_after[gen_idx]) {
                                *slot.lock().unwrap() = None;
                            }
                        }
                        barrier.wait();
                        if gen_idx + 1 < gens {

                            prior_max_boards = prior_max_boards.max(num_boards as usize);
                            gen_idx += 1;
                            num_boards = board_counts[gen_idx];
                            continue;
                        }
                    }
                    break;
                }
            });
        }
    });

    let (accum_sum, accum_cnt, _, _, ever) = shared.into_inner().unwrap();
    let leave_final = leave_lock.into_inner().unwrap();

    let value_mp = |idx: usize| -> f64 {
        if multigen {
            leave_final[idx] as f64
        } else if accum_cnt[idx] > 0 {
            accum_sum[idx] / accum_cnt[idx] as f64
        } else {
            0.0
        }
    };
    let baseline = value_mp(empty_rank);
    let out_name = claim_output_path(&format!("census-leaves-{census_run_epoch}.csv"))?;

    let max_keep = if full {
        rack_size
    } else {
        rack_size.saturating_sub(1)
    };
    let mut rows: Vec<(usize, String, f64)> = Vec::new();
    let mut leave_ser = String::new();
    for idx in 0..lat.len() {
        let valued = if multigen {
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
    writeln!(
        boxed_stdout_or_stderr(),
        "census: wrote {} leaves to {} in {}s (baseline {:.3} pts)",
        rows.len(),
        out_name,
        t0.elapsed().as_secs(),
        baseline / equity::SCALE as f64,
    )?;

    let klv_name = claim_output_path(&format!("census-leaves-{census_run_epoch}.klv2"))?;
    let is_valued = |idx: usize| {
        if multigen {
            ever[idx]
        } else {
            accum_cnt[idx] > 0
        }
    };
    let n_klv = write_census_klv2(&lat, &value_mp, baseline, &is_valued, full, &klv_name)?;
    writeln!(
        boxed_stdout_or_stderr(),
        "census: wrote klv2 to {klv_name} ({n_klv} leaves)"
    )?;
    Ok(())
}

#[inline]
fn decompose_contribution(fv: &Cumulate, w: u64) -> (f64, u64) {
    (fv.equity / fv.count as f64 * w as f64, w)
}

#[inline]
fn generate_leaves<Readable: std::io::Read, W: std::io::Write, const IS_FULL_RACK: bool>(
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

    let mut subrack_support = fash::MyHashMap::<bites::Bites, u64>::default();
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
                    let (add_equity, add_count) = decompose_contribution(fv, w);
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

                    *subrack_support.entry(subrack_bytes.into()).or_insert(0u64) += fv.count;
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

    let smooth_min = 50;
    let mut ev_map = fash::MyHashMap::<bites::Bites, _>::default();
    let mut alphabet_freqs = (0..game_config.alphabet().len())
        .map(|tile| game_config.alphabet().freq(tile))
        .collect::<Box<_>>();
    let mut neighbor_buffer = Vec::with_capacity(game_config.rack_size() as usize);
    let mut num_smoothed = 0u64;
    generate_exchanges(&mut ExchangeEnv {
        found_exchange_move: |rack_bytes: &[u8]| {
            let mut new_v = if let Some(v) = subrack_map.get(rack_bytes) {
                if subrack_support.get(rack_bytes).copied().unwrap_or(0) >= smooth_min {
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
        "After {} seconds, have processed {} subracks and smoothed {} ({:.1}% below support floor {})",
        t0.elapsed().as_secs(),
        ev_map.len(),
        num_smoothed,
        if ev_map.is_empty() {
            0.0
        } else {
            100.0 * num_smoothed as f64 / ev_map.len() as f64
        },
        smooth_min,
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

#[inline]
fn discover_playability<N: kwg::Node + Sync + Send, L: kwg::Node + Sync + Send>(
    game_config: game_config::GameConfig,
    kwg: kwg::Kwg<N>,
    klv: klv::Klv<L>,
    num_games: u64,
    seed: Option<u64>,
    threads: usize,
) -> error::Returns<()> {
    let game_config = std::sync::Arc::new(game_config);
    let kwg = std::sync::Arc::new(kwg);
    let klv = std::sync::Arc::new(klv);
    let seed = seed.unwrap_or_else(rand::random);
    writeln!(boxed_stdout_or_stderr(), "seed: {seed}")?;
    let num_threads = threads;
    let num_processed_games = std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0));

    let run_identifier = std::sync::Arc::new(run_stamp());
    writeln!(
        boxed_stdout_or_stderr(),
        "run identifier is {run_identifier}"
    )?;
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
                                    writeln!(boxed_stdout_or_stderr(),
                                        "After {elapsed_time_secs} seconds, have played {logged_games} games ({completed_moves} moves) for {run_identifier}").ok();
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
                writeln!(boxed_stdout_or_stderr(), "{e:?}").ok();
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

        writeln!(
            boxed_stdout_or_stderr(),
            "{} records, {} unique words",
            row_count,
            full_word_map.len()
        )?;

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

    writeln!(
        boxed_stdout_or_stderr(),
        "After {} seconds, have played {} games ({} moves) for {}",
        t0.elapsed().as_secs(),
        completed_games.load(std::sync::atomic::Ordering::Relaxed),
        completed_moves.load(std::sync::atomic::Ordering::Relaxed),
        run_identifier
    )?;

    Ok(())
}

#[inline]
fn plural<'a>(n: u64, singular: &'a str, plural: &'a str) -> &'a str {
    if n == 1 { singular } else { plural }
}

#[derive(Clone, Copy)]
struct SeatLabels {
    p0: &'static str,
    p1: &'static str,
}

const KLV_SEATS: SeatLabels = SeatLabels {
    p0: "p0 (klv0)",
    p1: "p1 (klv1)",
};

const SIM_CONFIG_SEATS: SeatLabels = SeatLabels {
    p0: "p0 (--p0-*)",
    p1: "p1 (--p1-*)",
};

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

    #[inline]
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

    #[inline]
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

    #[inline(always)]
    fn total_games(&self) -> u64 {
        self.p0_wins + self.p0_losses + self.p0_draws
    }

    #[inline]
    fn print(&self, label: &str, seats: SeatLabels) {
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
            "  {}: {:.1} ({:.2}%)  {}: {:.1} ({:.2}%)",
            seats.p0,
            p0_total,
            p0_total / total as f64 * 100.0,
            seats.p1,
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
                seats.p0
            } else {
                seats.p1
            };
            println!("  {leading} leads, confidence: {confidence:.2}%");
        } else {
            println!("  no significant difference");
        }
    }

    #[inline]
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

    fn print(&self, seats: SeatLabels) {
        let all_total = self.all.total_games();
        let all_pairs = all_total / 2;
        self.all.print(
            &format!(
                "{all_total} {} ({all_pairs} {}):",
                plural(all_total, "game", "games"),
                plural(all_pairs, "pair", "pairs"),
            ),
            seats,
        );
        let div_total = self.divergent.total_games();
        if div_total > 0 && div_total < all_total {
            let div_pairs = div_total / 2;
            self.divergent.print(
                &format!(
                    "\n{div_total} divergent {} ({div_pairs} {} = {:.2}%):",
                    plural(div_total, "game", "games"),
                    plural(div_pairs, "pair", "pairs"),
                    div_pairs as f64 / all_pairs as f64 * 100.0,
                ),
                seats,
            );
        }

        self.all.print_porcelain();
    }
}

struct WinpctTables<'a, N: kwg::Node, L: kwg::Node> {
    game_config: &'a game_config::GameConfig,
    kwg: &'a kwg::Kwg<N>,
    arc_klv: &'a klv::Klv<L>,
}

#[inline]
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

#[inline]
fn generate_winpct_table<N: kwg::Node + Sync + Send, L: kwg::Node + Sync + Send>(
    game_config: game_config::GameConfig,
    kwg: kwg::Kwg<N>,
    arc_klv: std::sync::Arc<klv::Klv<L>>,
    out_path: &str,
    num_games: u64,
    seed: Option<u64>,
    threads: usize,
) -> error::Returns<()> {
    let t0 = std::time::Instant::now();
    let game_config = std::sync::Arc::new(game_config);
    let seed = seed.unwrap_or_else(rand::random);
    let num_threads = threads.max(1).min(num_games.max(1) as usize);
    writeln!(
        boxed_stdout_or_stderr(),
        "winpct: seed {seed}, {num_games} games, {num_threads} threads"
    )?;
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
                        writeln!(boxed_stdout_or_stderr(), "winpct: {} games", g + 1).ok();
                    }
                }
                shared.lock().unwrap().merge(&acc);
            });
        }
    });

    let acc = shared.into_inner().unwrap();

    let mut out = std::io::BufWriter::new(make_writer(out_path)?);
    acc.to_csv(&mut out)?;
    out.flush()?;
    writeln!(
        boxed_stdout_or_stderr(),
        "winpct: {num_games} games in {}s",
        t0.elapsed().as_secs()
    )?;
    Ok(())
}

#[inline]
fn generate_winpct_eval<N: kwg::Node + Sync + Send, L: kwg::Node + Sync + Send>(
    game_config: game_config::GameConfig,
    kwg: kwg::Kwg<N>,
    arc_klv: std::sync::Arc<klv::Klv<L>>,
    table: win_pct::WinPctTable,
    num_games: u64,
    seed: Option<u64>,
    threads: usize,
) -> error::Returns<()> {
    let t0 = std::time::Instant::now();
    let game_config = std::sync::Arc::new(game_config);
    let table = std::sync::Arc::new(table);
    let seed = seed.unwrap_or_else(rand::random);
    let num_threads = threads.max(1).min(num_games.max(1) as usize);
    writeln!(
        boxed_stdout_or_stderr(),
        "winpct-eval: seed {seed}, {num_games} games, {num_threads} threads"
    )?;
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
    writeln!(
        boxed_stdout_or_stderr(),
        "winpct-eval: {n} samples, brier table={:.5} sigmoid={:.5} (lower better)",
        bt / d,
        bs / d
    )?;
    writeln!(
        boxed_stdout_or_stderr(),
        "winpct-eval: {num_games} games in {}s",
        t0.elapsed().as_secs()
    )?;
    Ok(())
}

#[inline]
fn compare_leaves<N: kwg::Node + Sync + Send, L: kwg::Node + Sync + Send>(
    game_config: game_config::GameConfig,
    kwg: kwg::Kwg<N>,
    arc_klv0: std::sync::Arc<klv::Klv<L>>,
    arc_klv1: std::sync::Arc<klv::Klv<L>>,
    num_game_pairs: u64,
    seed: Option<u64>,
    threads: usize,
) -> error::Returns<()> {
    let game_config = std::sync::Arc::new(game_config);
    let kwg = std::sync::Arc::new(kwg);
    let seed = seed.unwrap_or_else(rand::random);
    writeln!(boxed_stdout_or_stderr(), "seed: {seed}")?;
    let num_threads = threads;
    let claimed_pairs = std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0));
    let finished_pairs = std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0));
    let reported_secs = std::sync::atomic::AtomicU64::new(0);
    let t0 = std::time::Instant::now();

    std::thread::scope(|s| -> error::Returns<()> {
        let mut thread_handles = Vec::new();
        for _ in 0..num_threads {
            let game_config = std::sync::Arc::clone(&game_config);
            let kwg = std::sync::Arc::clone(&kwg);
            let arc_klv0 = std::sync::Arc::clone(&arc_klv0);
            let arc_klv1 = std::sync::Arc::clone(&arc_klv1);
            let claimed_pairs = std::sync::Arc::clone(&claimed_pairs);
            let finished_pairs = std::sync::Arc::clone(&finished_pairs);
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
                    let pair_idx = claimed_pairs.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
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
                                dynamic_leaves: None,
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

                    finished_pairs.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    let secs = t0.elapsed().as_secs();
                    let prev = reported_secs.fetch_max(secs, std::sync::atomic::Ordering::Relaxed);
                    if secs > prev {
                        writeln!(
                            boxed_stdout_or_stderr(),
                            "After {secs}s: {} pairs",
                            finished_pairs.load(std::sync::atomic::Ordering::Relaxed),
                        )
                        .ok();
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
        combined.print(KLV_SEATS);

        Ok(())
    })
}

struct SimCompareParams {
    num_game_pairs: u64,
    seed: Option<u64>,
    threads: usize,
    num_sim_iters: u64,
    sim_threads: usize,
    p0_picker: SeatPicker,
    p1_picker: SeatPicker,
    p0_win_pct: Option<String>,
    p1_win_pct: Option<String>,
}

#[derive(Clone, Copy, clap::ValueEnum)]
enum SeatPicker {
    Hasty,
    Simmer,
}

#[inline]
fn picker_name(picker: SeatPicker) -> &'static str {
    match picker {
        SeatPicker::Hasty => "hasty",
        SeatPicker::Simmer => "simmer",
    }
}

const SIM_FIELD_PROBE_RACKS: &[&str] = &["ADEINRT", "AEGIOTU", "CDIOUVW", "AAIIOUU"];

#[inline]
fn sim_field_check<N: kwg::Node, L: kwg::Node>(
    game_config: game_config::GameConfig,
    kwg: kwg::Kwg<N>,
    klv: klv::Klv<L>,
    seed: u64,
    turns: u32,
) -> error::Returns<()> {
    let alphabet = game_config.alphabet();
    let rack_reader = alphabet::AlphabetReader::new_for_racks(alphabet);
    let mut rng = rand::rngs::ChaCha20Rng::seed_from_u64(seed);
    let mut game_state = game_state::GameState::new(&game_config);
    game_state.reset_and_draw_tiles(&game_config, &mut rng);
    let mut move_generator = movegen::KurniaMoveGenerator::new(&game_config);
    let mut filtered_movegen = move_filter::GenMoves::Unfiltered;
    let mut scratch = Vec::new();
    let play_one_greedy_turn = |game_state: &mut game_state::GameState,
                                rng: &mut rand::rngs::ChaCha20Rng,
                                move_generator: &mut movegen::KurniaMoveGenerator|
     -> error::Returns<bool> {
        move_generator.gen_moves_unfiltered(&movegen::GenMovesParams {
            board_snapshot: &movegen::BoardSnapshot {
                board_tiles: &game_state.board_tiles,
                game_config: &game_config,
                kwg: &kwg,
                klv: &klv,
            },
            rack: &game_state.current_player().rack,
            max_gen: 1,
            num_exchanges_by_this_player: game_state.current_player().num_exchanges,
            pass_policy: movegen::PassPolicy::OnlyWhenForced,
            dynamic_leaves: None,
        });
        let play = move_generator.plays[0].play.clone();
        game_state.play(&game_config, rng, &play)?;
        let mut final_scores = vec![0i32; game_config.num_players() as usize];
        match game_state.check_game_ended(&game_config, &mut final_scores) {
            game_state::CheckGameEnded::NotEnded => {
                game_state.next_turn();
                Ok(true)
            }
            _ => Ok(false),
        }
    };
    for _ in 0..turns {
        if !play_one_greedy_turn(&mut game_state, &mut rng, &mut move_generator)? {
            return Err("the game ended before the board filled; lower the turn count".into());
        }
    }

    let mut failures = 0usize;
    let mut probe = |game_state: &game_state::GameState,
                     move_generator: &mut movegen::KurniaMoveGenerator,
                     scratch: &mut Vec<movegen::ValuedMove>,
                     label: &str|
     -> usize {
        let board_snapshot = movegen::BoardSnapshot {
            board_tiles: &game_state.board_tiles,
            game_config: &game_config,
            kwg: &kwg,
            klv: &klv,
        };
        let rack = &game_state.current_player().rack;
        let num_exchanges = game_state.current_player().num_exchanges;
        move_generator.gen_moves_unfiltered(&movegen::GenMovesParams {
            board_snapshot: &board_snapshot,
            rack,
            max_gen: 10_000,
            num_exchanges_by_this_player: num_exchanges,
            pass_policy: movegen::PassPolicy::OnlyWhenForced,
            dynamic_leaves: None,
        });
        let rank = move_generator
            .plays
            .iter()
            .position(|valued_move| matches!(valued_move.play, movegen::Play::Exchange { .. }));
        let num_moves = move_generator.plays.len();
        move_picker::gen_simmer_candidates(
            &mut filtered_movegen,
            move_generator,
            &board_snapshot,
            rack,
            num_exchanges,
            move_picker::FieldShape {
                max_gen: move_picker::SIMMER_FIELD.max_gen,
                nonplacing_quota: 0,
            },
            scratch,
        );
        let without = count_nonplacing_plays(&move_generator.plays);
        move_picker::gen_simmer_candidates(
            &mut filtered_movegen,
            move_generator,
            &board_snapshot,
            rack,
            num_exchanges,
            move_picker::SIMMER_FIELD,
            scratch,
        );
        let with = count_nonplacing_plays(&move_generator.plays);
        println!(
            "{label}: {}; field of {} holds {without} without the quota, {with} with it",
            match rank {
                Some(rank) => format!(
                    "best places-nothing move ranks {} of {num_moves} by static equity",
                    rank + 1
                ),
                None => "only tile plays are generated".to_string(),
            },
            move_picker::SIMMER_FIELD.max_gen,
        );
        with
    };

    for rack_str in SIM_FIELD_PROBE_RACKS {
        let mut rack = Vec::new();
        rack_reader.set_word(rack_str, &mut rack)?;
        game_state.players[game_state.turn as usize].rack = rack;
        let with = probe(
            &game_state,
            &mut move_generator,
            &mut scratch,
            &format!("rack {rack_str}"),
        );
        if with == 0 {
            writeln!(
                boxed_stdout_or_stderr(),
                "FAIL: rack {rack_str} field holds no move that places nothing"
            )?;
            failures += 1;
        }
    }

    let exchange_tile_limit = game_config.exchange_tile_limit();
    let mut extra_turns = 0u32;
    while move_generator.num_tiles_in_bag() >= exchange_tile_limit {
        if !play_one_greedy_turn(&mut game_state, &mut rng, &mut move_generator)? {
            return Err("the game ended before the bag ran down".into());
        }
        extra_turns += 1;
    }
    println!(
        "after {} more turns the bag holds {}, under the {exchange_tile_limit} an exchange needs",
        extra_turns,
        move_generator.num_tiles_in_bag(),
    );
    let with = probe(
        &game_state,
        &mut move_generator,
        &mut scratch,
        "bag too short to exchange",
    );
    if with == 0 {
        writeln!(
            boxed_stdout_or_stderr(),
            "FAIL: field holds no pass once exchanging is illegal"
        )?;
        failures += 1;
    }

    if failures == 0 {
        println!("SIM_FIELD_OK");
        Ok(())
    } else {
        wolges::return_error!(format!("{failures} field checks failed"))
    }
}

#[inline]
fn count_nonplacing_plays(plays: &[movegen::ValuedMove]) -> usize {
    plays
        .iter()
        .filter(|valued_move| matches!(valued_move.play, movegen::Play::Exchange { .. }))
        .count()
}

#[inline]
fn sim_chunk_check<N: kwg::Node + Sync, L: kwg::Node + Sync>(
    game_config: game_config::GameConfig,
    kwg: kwg::Kwg<N>,
    klv: klv::Klv<L>,
    iters: u64,
    seed: u64,
    trials: u32,
) -> error::Returns<()> {
    if iters < 2 {
        return Err("sim-chunk-check needs at least two iterations".into());
    }
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
    let mut driver = move_picker::Simmer::<_, _, true>::new(
        &game_config,
        &kwg,
        &klv,
        move_picker::SimmerParams {
            num_sim_iters: iters,
            sim_threads: 1,
            win_pct_table: None,
        },
    );
    driver.reseed(seed);
    driver.begin_decision(&move_generator, &game_state, iters);
    let whole = driver.leaderboard(usize::MAX);
    println!("one call of {iters}: {} candidates", whole.len());

    let mut mismatches = 0u32;
    for trial in 0..trials {
        let mut chunks = Vec::new();
        let mut left = iters;
        while left > 0 {
            let take = rng.random_range(1..=left);
            chunks.push(take);
            left -= take;
        }
        driver.reseed(seed);
        driver.begin_decision(&move_generator, &game_state, chunks[0]);
        for &chunk in &chunks[1..] {
            driver.resume(&move_generator, chunk);
        }
        let split = driver.leaderboard(usize::MAX);
        if split == whole {
            println!("trial {trial}: {} chunks {chunks:?} match", chunks.len());
        } else {
            mismatches += 1;
            writeln!(
                boxed_stdout_or_stderr(),
                "FAIL trial {trial}: {} chunks {chunks:?} disagree with the one-call run",
                chunks.len(),
            )?;
            for (i, (a, b)) in whole.iter().zip(split.iter()).enumerate() {
                if a != b {
                    writeln!(
                        boxed_stdout_or_stderr(),
                        "  row {i}: one call {a:?} vs chunked {b:?}"
                    )?;
                }
            }
        }
    }
    if mismatches == 0 {
        println!("SIM_CHUNK_OK");
        Ok(())
    } else {
        wolges::return_error!(format!("{mismatches} of {trials} chunkings disagreed"))
    }
}

#[inline]
fn bag_size(board_tiles: &[u8], game_config: &game_config::GameConfig) -> i64 {
    let on_board = board_tiles.iter().filter(|&&t| t != 0).count() as i64;
    game_config.alphabet().num_tiles() as i64
        - (on_board + game_config.num_players() as i64 * game_config.rack_size() as i64)
}

#[inline]
fn census_one_board<N: kwg::Node, L: kwg::Node>(
    game_config: game_config::GameConfig,
    kwg: kwg::Kwg<N>,
    klv: klv::Klv<L>,
    board_fen: &str,
    rack_strs: &[String],
) -> error::Returns<()> {
    let alphabet = game_config.alphabet();
    let board_layout = game_config.board_layout();
    let num_letters = alphabet.len() as usize;
    let rack_size = game_config.rack_size() as usize;
    let mut fen_parser = display::BoardFenParser::new(alphabet, board_layout);
    let board_tiles = fen_parser.parse(board_fen)?.to_vec();
    let tiles_on_board = board_tiles.iter().filter(|&&t| t != 0).count();
    let mut unseen_tally = (0..alphabet.len())
        .map(|t| alphabet.freq(t))
        .collect::<Vec<u8>>();
    for &t in &board_tiles {
        if t != 0 {
            let base = t & !((t as i8) >> 7) as u8;
            unseen_tally[base as usize] = unseen_tally[base as usize].saturating_sub(1);
        }
    }
    let unseen: usize = unseen_tally.iter().map(|&c| c as usize).sum();
    let lat = census::MultisetLattice::new(num_letters, rack_size);
    let can_exchange =
        bag_size(&board_tiles, &game_config) >= game_config.exchange_tile_limit() as i64;
    let sheet_floor = if can_exchange { 0 } else { i32::MIN / 4 };
    let mut sheet = vec![sheet_floor; lat.len()];
    let mut move_generator = movegen::KurniaMoveGenerator::new(&game_config);
    let mut movegen_rack = Vec::new();
    let mut blank_deltas = Vec::new();
    let num_blanks_eff = (unseen_tally[0] as usize).min(rack_size);
    let t0 = std::time::Instant::now();
    let num_candidates = build_sheet_spell_once(
        &mut move_generator,
        &board_tiles,
        SpellTables {
            game_config: &game_config,
            kwg: &kwg,
            klv: &klv,
            lat: &lat,
        },
        SpellPool {
            unseen_tally: &unseen_tally,
            num_blanks_eff,
            rack_size,
        },
        &mut movegen_rack,
        &mut blank_deltas,
        &mut sheet,
    );
    let sheet_elapsed = t0.elapsed();
    if !can_exchange {
        let empty = vec![0u8; num_letters];
        let idx = lat.rank(&empty);
        if idx != !0 {
            sheet[idx as usize] = 0;
        }
    }
    let mut leave = vec![0i32; lat.len()];
    census::fill_lattice_leaves(&lat, &mut leave, |tally| klv.leave_value_from_tally(tally));
    println!(
        "{tiles_on_board} tiles on the board, {unseen} unseen, {num_candidates} candidate plays \
         in {sheet_elapsed:?}",
    );
    let rack_reader = alphabet::AlphabetReader::new_for_racks(alphabet);
    let mut rack = Vec::new();
    let mut tally = vec![0u8; num_letters];
    for rack_str in rack_strs {
        rack_reader.set_word(rack_str, &mut rack)?;
        if rack.len() > rack_size {
            return Err(format!(
                "{rack_str} has {} tiles; a rack holds {rack_size}",
                rack.len(),
            )
            .into());
        }
        tally.iter_mut().for_each(|c| *c = 0);
        for &t in &rack {
            tally[t as usize] += 1;
        }
        let mut played = vec![0u8; num_letters];
        let mut kept = tally.clone();
        let mut best = i32::MIN;
        best_split(
            &RackSplit {
                lat: &lat,
                sheet: &sheet,
                leave: &leave,
                rack: &rack,
            },
            0,
            &mut played,
            &mut kept,
            &mut best,
        );
        let drawable = rack
            .iter()
            .all(|&t| tally[t as usize] <= unseen_tally[t as usize]);
        println!(
            "  {rack_str:<10} best equity {:>8.3}{}",
            equity::Equity::new(best).as_f64(),
            if drawable {
                ""
            } else {
                "   (not drawable from this board's unseen pool)"
            },
        );
    }
    Ok(())
}

struct RackSplit<'a> {
    lat: &'a census::MultisetLattice,
    sheet: &'a [i32],
    leave: &'a [i32],
    rack: &'a [u8],
}

fn best_split(
    split: &RackSplit<'_>,
    at: usize,
    played: &mut [u8],
    kept: &mut [u8],
    best: &mut i32,
) {
    if at == split.rack.len() {
        let played_idx = split.lat.rank(played);
        let kept_idx = split.lat.rank(kept);
        if played_idx != !0 && kept_idx != !0 {
            let value = split.sheet[played_idx as usize] + split.leave[kept_idx as usize];
            if value > *best {
                *best = value;
            }
        }
        return;
    }
    let tile = split.rack[at] as usize;
    best_split(split, at + 1, played, kept, best);
    kept[tile] -= 1;
    played[tile] += 1;
    best_split(split, at + 1, played, kept, best);
    played[tile] -= 1;
    kept[tile] += 1;
}

#[inline]
fn inspect_dynamic_leaves<L: kwg::Node>(
    game_config: game_config::GameConfig,
    klv: klv::Klv<L>,
    rack_str: &str,
    board_fen: &str,
    threads: usize,
) -> error::Returns<()> {
    let alphabet = game_config.alphabet();
    let board_layout = game_config.board_layout();
    let num_letters = alphabet.len() as usize;
    let rack_size = game_config.rack_size() as usize;
    let mut rack = Vec::new();
    alphabet::AlphabetReader::new_for_racks(alphabet).set_word(rack_str, &mut rack)?;
    rack.sort_unstable();
    if rack.len() > rack_size {
        return Err("that is more tiles than a rack holds".into());
    }
    let mut fen_parser = display::BoardFenParser::new(alphabet, board_layout);
    let board_tiles = fen_parser.parse(board_fen)?;
    let mut pool = vec![0u8; num_letters];
    for (tile, slot) in (0u8..).zip(pool.iter_mut()) {
        *slot = alphabet.freq(tile);
    }
    for &tile in board_tiles {
        if tile != 0 {
            let tile = if tile & 0x80 != 0 { 0 } else { tile } as usize;
            if pool[tile] == 0 {
                return Err("the board holds more of a tile than the bag has".into());
            }
            pool[tile] -= 1;
        }
    }
    for &tile in &rack {
        if pool[tile as usize] == 0 {
            return Err("the rack holds a tile the board has already used up".into());
        }
        pool[tile as usize] -= 1;
    }
    let pool_size: usize = pool.iter().map(|&c| c as usize).sum();
    let lat = census::MultisetLattice::new(num_letters, rack_size);
    let add = census::AddTable::new_with_threads(&lat, threads);
    let mut full_v = vec![0i32; lat.len()];
    census::fill_lattice_leaves(&lat, &mut full_v, |tally| klv.leave_value_from_tally(tally));
    println!("rack {rack_str}, {pool_size} tiles unseen");
    println!("  keep                 static    dynamic     delta");
    let mut distinct: Vec<(u8, u8)> = Vec::new();
    for &tile in &rack {
        match distinct.last_mut() {
            Some((t, count)) if *t == tile => *count += 1,
            _ => distinct.push((tile, 1)),
        }
    }
    let mut kept_tally = vec![0u8; num_letters];
    let mut rows: Vec<(usize, String, i32, i32)> = Vec::new();
    let mut counts = vec![0u8; distinct.len()];
    loop {
        let mut kept = String::new();
        let mut size = 0usize;
        kept_tally.iter_mut().for_each(|c| *c = 0);
        for (&(tile, _), &count) in distinct.iter().zip(counts.iter()) {
            kept_tally[tile as usize] = count;
            size += count as usize;
            for _ in 0..count {
                kept.push_str(alphabet.of_rack(tile).unwrap());
            }
        }
        let s_ridx = lat.rank(&kept_tally);
        if s_ridx != !0 {
            let static_value = klv.leave_value_from_tally(&kept_tally);
            let draw = (rack_size - size).min(pool_size);
            let dynamic =
                census::dynamic_leave_value(&lat, &add, &full_v, &pool, s_ridx as usize, draw);
            rows.push((size, kept, static_value, dynamic));
        }
        let mut i = 0;
        while i < counts.len() {
            if counts[i] < distinct[i].1 {
                counts[i] += 1;
                break;
            }
            counts[i] = 0;
            i += 1;
        }
        if i == counts.len() {
            break;
        }
    }
    rows.sort_unstable_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.cmp(&b.1)));
    for (_, kept, static_value, dynamic) in rows {
        let scale = equity::SCALE as f64;
        if dynamic == census::UNPLAYABLE {
            println!(
                "  {kept:<16} {:>10.3}  undrawable",
                static_value as f64 / scale,
            );
        } else {
            println!(
                "  {kept:<16} {:>10.3} {:>10.3} {:>9.3}",
                static_value as f64 / scale,
                dynamic as f64 / scale,
                (dynamic - static_value) as f64 / scale,
            );
        }
    }
    Ok(())
}

struct BoardShape {
    area: usize,
    num_words: usize,
    total_word_length: usize,
    perimeter: usize,
    radius_of_gyration: f64,
    holes: usize,
}

#[inline]
fn board_shape(board_layout: &board_layout::BoardLayout, board_tiles: &[u8]) -> BoardShape {
    let dim = board_layout.dim();
    let rows = dim.rows as isize;
    let cols = dim.cols as isize;
    let at = |r: isize, c: isize| -> u8 {
        if r < 0 || c < 0 || r >= rows || c >= cols {
            0
        } else {
            board_tiles[(r * cols + c) as usize]
        }
    };
    let filled = |r: isize, c: isize| -> bool { at(r, c) != 0 };
    let filled_or_off =
        |r: isize, c: isize| -> bool { r < 0 || c < 0 || r >= rows || c >= cols || filled(r, c) };
    let mut area = 0usize;
    let mut perimeter = 0usize;
    let mut holes = 0usize;
    let mut sum_r = 0f64;
    let mut sum_c = 0f64;
    for r in 0..rows {
        for c in 0..cols {
            if filled(r, c) {
                area += 1;
                sum_r += r as f64;
                sum_c += c as f64;
                for (dr, dc) in [(-1, 0), (1, 0), (0, -1), (0, 1)] {
                    if !filled(r + dr, c + dc) {
                        perimeter += 1;
                    }
                }
            } else if [(-1, 0), (1, 0), (0, -1), (0, 1)]
                .iter()
                .all(|&(dr, dc)| filled_or_off(r + dr, c + dc))
            {
                holes += 1;
            }
        }
    }
    let mut num_words = 0usize;
    let mut total_word_length = 0usize;
    for down in [false, true] {
        let (outer, inner) = if down { (cols, rows) } else { (rows, cols) };
        for a in 0..outer {
            let mut run = 0usize;
            for b in 0..=inner {
                let occupied = b < inner && if down { filled(b, a) } else { filled(a, b) };
                if occupied {
                    run += 1;
                } else {
                    if run >= 2 {
                        num_words += 1;
                        total_word_length += run;
                    }
                    run = 0;
                }
            }
        }
    }
    let radius_of_gyration = if area == 0 {
        0.0
    } else {
        let mean_r = sum_r / area as f64;
        let mean_c = sum_c / area as f64;
        let mut sum_sq = 0f64;
        for r in 0..rows {
            for c in 0..cols {
                if filled(r, c) {
                    let dr = r as f64 - mean_r;
                    let dc = c as f64 - mean_c;
                    sum_sq += dr * dr + dc * dc;
                }
            }
        }
        (sum_sq / area as f64).sqrt()
    };
    BoardShape {
        area,
        num_words,
        total_word_length,
        perimeter,
        radius_of_gyration,
        holes,
    }
}

#[inline]
fn board_stats(
    game_config: game_config::GameConfig,
    reader: &mut Box<dyn std::io::Read>,
) -> error::Returns<()> {
    let alphabet = game_config.alphabet();
    let board_layout = game_config.board_layout();
    let mut fen_parser = display::BoardFenParser::new(alphabet, board_layout);
    let mut boards = 0u64;
    let mut area = stats::Stats::new();
    let mut words = stats::Stats::new();
    let mut word_length = stats::Stats::new();
    let mut perimeter_per_area = stats::Stats::new();
    let mut radius = stats::Stats::new();
    let mut holes = stats::Stats::new();
    let mut text = String::new();
    reader.read_to_string(&mut text)?;
    for line in text.lines() {
        if line.is_empty() {
            continue;
        }
        let shape = board_shape(board_layout, fen_parser.parse(line)?);
        boards += 1;
        area.update(shape.area as f64);
        words.update(shape.num_words as f64);
        if shape.num_words > 0 {
            word_length.update(shape.total_word_length as f64 / shape.num_words as f64);
        }
        if shape.area > 0 {
            perimeter_per_area.update(shape.perimeter as f64 / shape.area as f64);
        }
        radius.update(shape.radius_of_gyration);
        holes.update(shape.holes as f64);
    }
    if boards == 0 {
        return Err("no boards to measure".into());
    }
    println!("boards: {boards}");
    for (label, stat) in [
        ("tiles on board", &area),
        ("words", &words),
        ("mean word length", &word_length),
        ("perimeter per tile", &perimeter_per_area),
        ("radius of gyration", &radius),
        ("holes", &holes),
    ] {
        println!(
            "  {label}: {:.3} (sd={:.3})",
            stat.mean(),
            stat.standard_deviation(),
        );
    }
    Ok(())
}

#[inline]
fn sim_compare<N: kwg::Node + Sync + Send, L: kwg::Node + Sync + Send>(
    game_config: game_config::GameConfig,
    kwg: kwg::Kwg<N>,
    arc_klv: std::sync::Arc<klv::Klv<L>>,
    SimCompareParams {
        num_game_pairs,
        seed,
        threads,
        num_sim_iters,
        sim_threads,
        p0_picker,
        p1_picker,
        p0_win_pct,
        p1_win_pct,
    }: SimCompareParams,
) -> error::Returns<()> {
    let game_config = std::sync::Arc::new(game_config);
    let kwg = std::sync::Arc::new(kwg);
    let seed = seed.unwrap_or_else(rand::random);
    writeln!(boxed_stdout_or_stderr(), "seed: {seed}")?;
    let num_threads = threads;
    let claimed_pairs = std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0));
    let finished_pairs = std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0));
    let reported_secs = std::sync::atomic::AtomicU64::new(0);
    let t0 = std::time::Instant::now();

    let winpct_p0 = match &p0_win_pct {
        Some(path) => Some(win_pct::WinPctTable::from_csv(make_reader(path)?)?),
        None => None,
    };
    let winpct_p1 = match &p1_win_pct {
        Some(path) => Some(win_pct::WinPctTable::from_csv(make_reader(path)?)?),
        None => None,
    };
    let winpct_p0 = winpct_p0.as_ref();
    let winpct_p1 = winpct_p1.as_ref();
    writeln!(
        boxed_stdout_or_stderr(),
        "sim-compare: {num_sim_iters} rollouts a move; p0 {} win%={}; p1 {} win%={}",
        picker_name(p0_picker),
        p0_win_pct.as_deref().unwrap_or("sigmoid"),
        picker_name(p1_picker),
        p1_win_pct.as_deref().unwrap_or("sigmoid"),
    )?;

    std::thread::scope(|s| -> error::Returns<()> {
        let mut thread_handles = Vec::new();
        for _ in 0..num_threads {
            let game_config = std::sync::Arc::clone(&game_config);
            let kwg = std::sync::Arc::clone(&kwg);
            let arc_klv = std::sync::Arc::clone(&arc_klv);
            let claimed_pairs = std::sync::Arc::clone(&claimed_pairs);
            let finished_pairs = std::sync::Arc::clone(&finished_pairs);
            let reported_secs = &reported_secs;
            thread_handles.push(s.spawn(move || {
                let mut rng = rand::rngs::ChaCha20Rng::seed_from_u64(seed);
                let mut move_generator = movegen::KurniaMoveGenerator::new(&game_config);
                let mut filtered_movegen = move_filter::GenMoves::Unfiltered;

                let mut driver_p0 = match p0_picker {
                    SeatPicker::Hasty => move_picker::MovePicker::Hasty,
                    SeatPicker::Simmer => {
                        move_picker::MovePicker::Simmer(move_picker::Simmer::new(
                            &game_config,
                            &kwg,
                            &arc_klv,
                            move_picker::SimmerParams {
                                num_sim_iters,
                                sim_threads,
                                win_pct_table: winpct_p0,
                            },
                        ))
                    }
                };
                let mut driver_p1 = match p1_picker {
                    SeatPicker::Hasty => move_picker::MovePicker::Hasty,
                    SeatPicker::Simmer => {
                        move_picker::MovePicker::Simmer(move_picker::Simmer::new(
                            &game_config,
                            &kwg,
                            &arc_klv,
                            move_picker::SimmerParams {
                                num_sim_iters,
                                sim_threads,
                                win_pct_table: winpct_p1,
                            },
                        ))
                    }
                };
                let mut game_state = game_state::GameState::new(&game_config);
                let mut saved_game_state = game_state.clone();
                let mut final_scores = vec![0i32; game_config.num_players() as usize];
                let mut stats = GamePairStats::new();
                let mut first_game_moves: Vec<movegen::Play> = Vec::new();

                loop {
                    let pair_idx = claimed_pairs.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
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

                    finished_pairs.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    let secs = t0.elapsed().as_secs();
                    let prev = reported_secs.fetch_max(secs, std::sync::atomic::Ordering::Relaxed);
                    if secs > prev {
                        writeln!(
                            boxed_stdout_or_stderr(),
                            "After {secs}s: {} pairs",
                            finished_pairs.load(std::sync::atomic::Ordering::Relaxed),
                        )
                        .ok();
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
        combined.print(SIM_CONFIG_SEATS);

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
    #[inline]
    fn census_sheet_reuse_plan_looks_past_the_next_generation() {
        for &(counts, want_live, want_len) in SHEET_PLANS {
            let (live_after, cache_len) = census_sheet_reuse_plan(counts);
            assert_eq!(live_after, want_live, "live_after for {counts:?}");
            assert_eq!(cache_len, want_len, "cache_len for {counts:?}");
        }
    }

    #[test]
    #[inline]
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
    #[inline]
    fn pooling_keeps_value_and_count_together() {
        let mut m = fash::MyHashMap::<bites::Bites, Cumulate>::default();
        pool_one(&mut m, &b"\x01"[..], 3.0);
        pool_one(&mut m, &b"\x01"[..], 4.0);
        let a = m.get(&b"\x01"[..]).unwrap();
        assert_eq!(a.count, 2);
        assert!((a.equity - 7.0).abs() < 1e-9);
    }

    #[test]
    #[inline]
    fn merging_thread_maps_keeps_every_sample() {
        let mut dst = fash::MyHashMap::<bites::Bites, Cumulate>::default();
        pool_one(&mut dst, &b"\x01"[..], 3.0);
        let mut src = fash::MyHashMap::<bites::Bites, Cumulate>::default();
        pool_one(&mut src, &b"\x01"[..], 4.0);
        pool_one(&mut src, &b"\x02"[..], 5.0);
        merge_rack_map(&mut dst, &mut src);
        let a = dst.get(&b"\x01"[..]).unwrap();
        assert_eq!(a.count, 2);
        assert!((a.equity - 7.0).abs() < 1e-9);

        let b = dst.get(&b"\x02"[..]).unwrap();
        assert_eq!(b.count, 1);
        assert!(src.is_empty(), "merge_rack_map must drain the source");
    }

    #[test]
    #[inline]
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
    #[inline]
    fn per_rack_decompose_weights_by_mean_not_count() {
        let fv = Cumulate {
            equity: 10.0,
            count: 2,
        };

        let (eq, cnt) = decompose_contribution(&fv, 3);
        assert!((eq - 15.0).abs() < 1e-9); // (10/2) * 3
        assert_eq!(cnt, 3); // w only
    }
}
