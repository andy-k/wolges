// Copyright (C) 2020-2026 Andy Kurnia.

use rand::prelude::*;
use std::fmt::Write;
use std::io::Write as _;
use std::str::FromStr;
use wolges::glance::{Glance, board_after, glance_at};
use wolges::simmer::CopyState;
use wolges::{
    alphabet, anagrams, bites, board_layout, build, census, display, equity, error, fash,
    game_config, game_state, klv, kwg, move_filter, move_picker, movegen, play_scorer, prob, stats,
    win_pct,
};

mod game_args;

static BASE62: &[u8; 62] = b"\
0123456789\
ABCDEFGHIJKLMNOPQRSTUVWXYZ\
abcdefghijklmnopqrstuvwxyz\
";

// the generator the tasks draw with. another can take its place if it
// implements Streams and CopyState.
type GameRng = rand_xoshiro::Xoshiro256PlusPlus;

// numbered streams of one seed: what a run's kth game draws depends on the seed
// and k alone, whichever thread plays it.
trait Streams: SeedableRng {
    fn from_stream(seed: u64, stream: u64) -> Self;
}

impl Streams for rand::rngs::ChaCha20Rng {
    // set_stream starts the stream at its first word.
    #[inline(always)]
    fn from_stream(seed: u64, stream: u64) -> Self {
        let mut rng = Self::seed_from_u64(seed);
        rng.set_stream(stream);
        rng
    }
}

impl Streams for rand_xoshiro::Xoshiro256PlusPlus {
    // xoshiro has no streams, so each one seeds from a mix of the seed and its
    // number.
    #[inline(always)]
    fn from_stream(seed: u64, stream: u64) -> Self {
        Self::seed_from_u64(census_mix64(seed) ^ stream)
    }
}

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

// the given seed, else one drawn from the system
#[inline]
fn seed_or_random(seed: Option<u64>) -> error::Returns<u64> {
    Ok(match seed {
        Some(seed) => seed,
        None => GameRng::try_from_rng(&mut rand::rngs::SysRng)?.next_u64(),
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
    let kwg = kwg::Kwg::<N>::from_bytes_alloc(&read_to_end(&mut make_reader(path)?)?)?;
    refuse_a_wider_graph(&kwg, game_config, path)?;
    Ok(kwg)
}

#[inline]
fn read_klv(
    game_config: &game_config::GameConfig,
    path: &str,
) -> error::Returns<klv::Klv<kwg::Node22>> {
    let klv = if path == "-" {
        klv::Klv::<kwg::Node22>::from_bytes_alloc(klv::EMPTY_KLV_BYTES)?
    } else {
        klv::Klv::<kwg::Node22>::from_bytes_alloc(&std::fs::read(path)?)?
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
    #[arg(long, help = "generate with the lean arm")]
    lean: bool,
    #[arg(long, help = "print no progress")]
    quiet: bool,
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
    #[arg(long, help = "generate with the lean arm")]
    lean: bool,
    #[arg(long, help = "print no progress")]
    quiet: bool,
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
    #[arg(long, help = "print no progress")]
    quiet: bool,
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
    #[arg(long, help = "generate with the lean arm")]
    lean: bool,
    #[arg(
        long,
        value_enum,
        default_value_t = SeatPicker::Hasty,
        help = "how seat p0 picks its moves"
    )]
    p0_picker: SeatPicker,
    #[arg(
        long,
        value_enum,
        default_value_t = SeatPicker::Hasty,
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
    #[arg(
        long,
        default_value_t = move_picker::DEFAULT_NUM_SIM_PLIES as u8,
        help = "the moves seat p0's simmer plays out after each candidate"
    )]
    p0_plies: u8,
    #[arg(
        long,
        default_value_t = move_picker::DEFAULT_NUM_SIM_PLIES as u8,
        help = "the moves seat p1's simmer plays out after each candidate"
    )]
    p1_plies: u8,
    #[arg(
        long,
        default_value_t = move_picker::DEFAULT_NUM_SIM_ITERS,
        help = "the rollouts seat p0's simmer makes for each move it picks"
    )]
    p0_iters: u64,
    #[arg(
        long,
        default_value_t = move_picker::DEFAULT_NUM_SIM_ITERS,
        help = "the rollouts seat p1's simmer makes for each move it picks"
    )]
    p1_iters: u64,
    #[arg(
        long,
        value_name = "POINTS",
        help = "seat p0, deft, takes among its moves within POINTS of its best the one leaving the fewest bingo lanes [default: 1]"
    )]
    p0_window: Option<f64>,
    #[arg(
        long,
        value_name = "POINTS",
        help = "seat p1, deft, takes among its moves within POINTS of its best the one leaving the fewest bingo lanes [default: 1]"
    )]
    p1_window: Option<f64>,
    #[arg(long, default_value = "1", help = "rollout threads a seat")]
    sim_threads: std::num::NonZeroUsize,
    #[arg(long, help = "print no progress")]
    quiet: bool,
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
    #[arg(long, help = "generate with the lean arm")]
    lean: bool,
    #[arg(long, help = "print no progress")]
    quiet: bool,
}

#[derive(clap::Args)]
struct HarvestBoards {
    #[arg(help = "the word graph (- for stdin)")]
    kwg: String,
    #[arg(help = "the leaves (- for none)")]
    leave: String,
    #[arg(help = "the fen file to write")]
    out: String,
    #[arg(default_value_t = 100)]
    games: u64,
    #[arg(default_value_t = 1)]
    seed: u64,
    #[arg(
        default_value_t = 0,
        help = "also write every kth ply's board (0: each finished board only)"
    )]
    stride: u32,
    #[arg(long, help = "generate with the lean arm")]
    lean: bool,
    #[arg(long, help = "print no progress")]
    quiet: bool,
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
struct Infer {
    #[arg(help = "the word graph (- for stdin)")]
    kwg: String,
    #[arg(help = "the leaves (- for none)")]
    klv: String,
    #[arg(help = "the board before the move, as a fen")]
    board: String,
    #[arg(
        help = "the move: \"8H QI\" (across from row 8, column H) or \"H8 Q.I\" (down), with . for a tile already on the board and lowercase for a blank; \"exch 3\"; or \"pass\""
    )]
    play: String,
    #[arg(
        long,
        help = "the rack of the player who watched the move, such as AEINRST: none of its tiles is the mover's"
    )]
    rack: Option<String>,
    #[arg(
        long,
        help = "how many points of equity below its best play make a rack e (2.718) times less likely to have made the move; 0 keeps only the racks closest to their best"
    )]
    fuzz: f64,
    #[arg(long, default_value_t = 20, help = "how many racks to list")]
    racks: usize,
}

#[derive(clap::Args)]
struct InferFit {
    #[arg(help = "the word graph (- for stdin)")]
    kwg: String,
    #[arg(help = "the leaves both players use (- for none)")]
    klv: String,
    #[arg(default_value_t = 100)]
    games: u64,
    #[arg(help = "prints the one it picks if omitted")]
    seed: Option<u64>,
    #[arg(
        long,
        value_enum,
        default_value_t = SeatPicker::Simmer,
        help = "how the mover picks its moves"
    )]
    mover: SeatPicker,
    #[arg(long, default_value_t = move_picker::DEFAULT_NUM_SIM_ITERS, help = "rollouts a move")]
    iters: u64,
    #[arg(long, help = "generate with the lean arm")]
    lean: bool,
    #[arg(long, help = "print no progress")]
    quiet: bool,
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
    #[arg(long, help = "print no progress")]
    quiet: bool,
}

#[derive(clap::Args)]
struct Openness {
    #[arg(help = "the word graph (- for stdin)")]
    kwg: String,
    #[arg(help = "the leaves (- for none)")]
    klv: String,
    #[arg(help = "the fen file, one board a line")]
    boards: String,
    #[arg(long, help = "print no progress")]
    quiet: bool,
}

#[derive(clap::Args)]
struct OpennessMoves {
    #[arg(help = "the word graph (- for stdin)")]
    kwg: String,
    #[arg(help = "the leaves both players use (- for none)")]
    klv: String,
    #[arg(default_value_t = 50)]
    games: u64,
    #[arg(help = "prints the one it picks if omitted")]
    seed: Option<u64>,
    #[arg(
        long,
        default_value_t = 1.0,
        help = "how many points of equity below the best a candidate may be"
    )]
    window: f64,
    #[arg(long, default_value_t = 5, help = "how many candidates a move at most")]
    candidates: usize,
    #[arg(long, help = "print no progress")]
    quiet: bool,
}

#[derive(clap::Args)]
struct Position {
    #[arg(help = "the word graph (- for stdin)")]
    kwg: String,
    #[arg(help = "the leaves (- for none)")]
    klv: String,
    #[arg(help = "the board, as a fen")]
    board: String,
    #[arg(help = "the rack of the player to move, such as AEINRST")]
    rack: Option<String>,
    #[arg(long, default_value_t = 10, help = "how many moves to list")]
    moves: usize,
    #[arg(
        long,
        value_name = "FILE",
        help = "write the board's dynamic leaves, from the tiles not on it, to FILE as a klv2 (needs a --full table)"
    )]
    dynamic: Option<String>,
    #[arg(long, help = "generate with the lean arm")]
    lean: bool,
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
    #[arg(long, help = "generate with the lean arm")]
    lean: bool,
    #[arg(long, help = "print no progress")]
    quiet: bool,
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
    #[arg(long, help = "generate with the lean arm")]
    lean: bool,
    #[arg(long, help = "print no progress")]
    quiet: bool,
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
    #[arg(long, help = "generate with the lean arm")]
    lean: bool,
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
    #[arg(long, help = "generate with the lean arm")]
    lean: bool,
    #[arg(long, help = "print no progress")]
    quiet: bool,
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
    #[arg(long, help = "generate with the lean arm")]
    lean: bool,
    #[arg(long, help = "print no progress")]
    quiet: bool,
}

#[derive(clap::Args)]
struct InOut {
    input: String,
    output: String,
}

#[derive(clap::Args)]
struct SummaryInOut {
    input: String,
    output: String,
    #[arg(long, help = "print no progress")]
    quiet: bool,
}

#[derive(clap::Args)]
struct Generate {
    summary: String,
    leaves: String,
    #[arg(long, help = "print no progress")]
    quiet: bool,
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
    #[arg(long, help = "generate with the lean arm")]
    lean: bool,
    #[arg(long, help = "print no progress")]
    quiet: bool,
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
    #[command(about = "play game pairs between two seats, each with its own leaves and picker")]
    Compare(Compare),
    #[command(about = "check that a resumed decision matches the same decision run in one call")]
    SimStudyCheck(SimStudyCheck),
    #[command(about = "play games and write their boards as fen, one per line")]
    HarvestBoards(HarvestBoards),
    #[command(
        about = "census one board given as fen: the best equity each listed rack could get on it; no sampling, no coverage, no mover's rack"
    )]
    CensusBoard(CensusBoard),
    #[command(
        about = "the racks the player who made one move could have held, each weighed by its chance to be dealt and by the equity the move gives up for it"
    )]
    Infer(Infer),
    #[command(
        about = "play games and find the fuzz of infer under which the movers' true racks were likeliest"
    )]
    InferFit(InferFit),
    #[command(
        about = "what the pool reweight does to one rack on one board: the static and dynamic value of every kept subrack"
    )]
    Dynaleaves(Dynaleaves),
    #[command(
        about = "shape of the boards in a fen file: tiles, words, mean word length, perimeter per tile, radius of gyration, holes"
    )]
    BoardStats(BoardStats),
    #[command(
        about = "how open each board in a fen file is: what a rack dealt from its unseen pool makes on it at best, exactly, beside what the board shows at a glance, and how well each glance predicts it"
    )]
    Openness(Openness),
    #[command(
        about = "play hasty games and weigh, at every move, the candidates near its best by how open each leaves the board for the opponent, exactly, and how well each glance at the board orders them"
    )]
    OpennessMoves(OpennessMoves),
    #[command(
        about = "one board given as fen: the board, the tiles not on it, its words and its shape; with a rack, the rack's moves and dynamic leaves"
    )]
    Position(Position),
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
    Summarize(SummaryInOut),
    #[command(about = "combine summaries into one and recompute totals")]
    Resummarize(SummaryInOut),
    #[command(about = "resummarize, sorted by length first")]
    ResummarizePlayability(SummaryInOut),
    #[command(about = "resummarize, sorted by playability first")]
    ResummarizePlayabilityAll(SummaryInOut),
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
            Task::Compare(_) | Task::Winpct(_) | Task::WinpctEval(_) | Task::InferFit(_)
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
                    lean: a.lean,
                    quiet: a.quiet,
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
                    lean: a.lean,
                    quiet: a.quiet,
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
                    lean: a.lean,
                    quiet: a.quiet,
                },
            )
        }
        Task::Gilles(a) => {
            let kwg = read_kwg::<N>(&game_config, &a.kwg)?;
            let (klv0, klv1) = read_klv_pair(&game_config, &a.leave0, &a.leave1)?;
            generate_gilles_summary(
                game_config,
                kwg,
                klv0,
                klv1,
                GamesParams {
                    num_games: a.games,
                    seed: a.seed,
                    threads,
                    lean: a.lean,
                    quiet: a.quiet,
                },
            )
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
                    quiet: a.quiet,
                },
            )
        }
        Task::Compare(a) => {
            // a deft seat's window, in points to milli-points: 1 point unless
            // given, and given for no other seat
            let window = |w: Option<f64>, picker: SeatPicker| -> error::Returns<i32> {
                if w.is_some() && !matches!(picker, SeatPicker::Deft) {
                    wolges::return_error!(
                        "--p0-window and --p1-window are for a deft seat".to_string()
                    );
                }
                match w {
                    None => Ok(move_picker::DEFAULT_DEFT_WINDOW),
                    Some(w) if w.is_finite() && w >= 0.0 => {
                        Ok((w * equity::SCALE as f64).round() as i32)
                    }
                    Some(_) => wolges::return_error!(
                        "a deft seat's window is points, 0 or more".to_string()
                    ),
                }
            };
            let p0_window = window(a.p0_window, a.p0_picker)?;
            let p1_window = window(a.p1_window, a.p1_picker)?;
            let kwg = read_kwg::<N>(&game_config, &a.kwg)?;
            let (klv0, klv1) = read_klv_pair(&game_config, &a.klv0, &a.klv1)?;
            let hasty_only = matches!(a.p0_picker, SeatPicker::Hasty)
                && matches!(a.p1_picker, SeatPicker::Hasty);
            let params = CompareParams {
                num_game_pairs: a.pairs,
                seed: a.seed,
                threads,
                sim_threads: a.sim_threads.get(),
                p0_picker: a.p0_picker,
                p1_picker: a.p1_picker,
                p0_win_pct: a.p0_win_pct,
                p1_win_pct: a.p1_win_pct,
                p0_plies: a.p0_plies.into(),
                p1_plies: a.p1_plies.into(),
                p0_iters: a.p0_iters,
                p1_iters: a.p1_iters,
                p0_window,
                p1_window,
                quiet: a.quiet,
            };
            match (a.lean, hasty_only) {
                (false, false) => {
                    compare_seats::<_, _, false, false>(game_config, kwg, klv0, klv1, params)
                }
                (false, true) => {
                    compare_seats::<_, _, false, true>(game_config, kwg, klv0, klv1, params)
                }
                (true, false) => {
                    compare_seats::<_, _, true, false>(game_config, kwg, klv0, klv1, params)
                }
                (true, true) => {
                    compare_seats::<_, _, true, true>(game_config, kwg, klv0, klv1, params)
                }
            }
        }
        Task::SimStudyCheck(a) => {
            let klv = read_klv(&game_config, &a.klv)?;
            let kwg = read_kwg::<N>(&game_config, &a.kwg)?;
            let arm_tables = anagrams::ArmTables::build(&kwg, &game_config, a.lean)?;
            let (iters, seed) = (a.iters, a.seed);
            let mut rng = GameRng::seed_from_u64(seed);
            let mut game_state = game_state::GameState::new(&game_config);
            game_state.reset_and_draw_tiles(&game_config, &mut rng);
            let mut move_generator = movegen::KurniaMoveGenerator::new(&game_config);
            move_generator.gen_moves_unfiltered(&movegen::GenMovesParams {
                board_snapshot: &movegen::BoardSnapshot {
                    board_tiles: &game_state.board_tiles,
                    game_config: &game_config,
                    kwg: &kwg,
                    anagrams: arm_tables.anagrams(),
                    rack_lengths: arm_tables.rack_lengths(),
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
                &arm_tables,
                &klv,
                move_picker::SimmerParams {
                    num_sim_iters: iters,
                    num_sim_plies: move_picker::DEFAULT_NUM_SIM_PLIES,
                    sim_threads: 1,
                    win_pct_table: None,
                    quiet: a.quiet,
                },
            )?;
            driver.reseed(seed);
            driver.begin_decision(&move_generator, &game_state, iters)?;
            let one_shot = driver.leader_summary();
            driver.reseed(seed);
            let half = iters / 2;
            driver.begin_decision(&move_generator, &game_state, half)?;
            driver.resume(&move_generator, iters - half)?;
            let split = driver.leader_summary();
            writeln!(
                std::io::stdout(),
                "one_shot leader play_index={} mean={} count={}",
                one_shot.0,
                one_shot.1,
                one_shot.2,
            )?;
            writeln!(
                std::io::stdout(),
                "split    leader play_index={} mean={} count={}",
                split.0,
                split.1,
                split.2,
            )?;
            if one_shot == split {
                writeln!(std::io::stdout(), "SIM_RESUME_OK")?;
                Ok(())
            } else {
                wolges::return_error!(
                    "resume mismatch: split decision differs from one-shot".to_string()
                )
            }
        }
        Task::HarvestBoards(a) => {
            let kwg = read_kwg::<N>(&game_config, &a.kwg)?;
            let klv = read_klv(&game_config, &a.leave)?;
            harvest_boards(
                game_config,
                kwg,
                klv,
                &a.out,
                HarvestParams {
                    num_games: a.games,
                    seed: a.seed,
                    stride: a.stride,
                    lean: a.lean,
                    quiet: a.quiet,
                },
            )
        }
        Task::CensusBoard(a) => {
            let kwg = read_kwg::<N>(&game_config, &a.kwg)?;
            let klv = read_klv(&game_config, &a.klv)?;
            census_one_board(game_config, kwg, klv, &a.board, &a.racks)
        }
        Task::Infer(a) => {
            let kwg = read_kwg::<N>(&game_config, &a.kwg)?;
            let klv = read_klv(&game_config, &a.klv)?;
            infer_one_move(game_config, kwg, klv, &a)
        }
        Task::InferFit(a) => {
            let kwg = read_kwg::<N>(&game_config, &a.kwg)?;
            let klv = read_klv(&game_config, &a.klv)?;
            infer_fit(
                game_config,
                kwg,
                klv,
                InferFitParams {
                    num_games: a.games,
                    seed: a.seed,
                    threads,
                    num_sim_iters: a.iters,
                    mover: a.mover,
                    lean: a.lean,
                    quiet: a.quiet,
                },
            )
        }
        Task::Dynaleaves(a) => {
            let klv = read_klv(&game_config, &a.klv)?;
            inspect_dynamic_leaves(game_config, klv, &a.rack, &a.board, threads)
        }
        Task::BoardStats(a) => board_stats(game_config, &mut make_reader(&a.boards)?, a.quiet),
        Task::Openness(a) => {
            let kwg = read_kwg::<N>(&game_config, &a.kwg)?;
            let klv = read_klv(&game_config, &a.klv)?;
            board_openness(game_config, kwg, klv, &a.boards, threads, a.quiet)
        }
        Task::OpennessMoves(a) => {
            if a.window.is_nan() || a.window < 0.0 || a.candidates < 2 {
                wolges::return_error!(
                    "the window is a number of points not below 0, and candidates 2 or more"
                        .to_string()
                );
            }
            let kwg = read_kwg::<N>(&game_config, &a.kwg)?;
            let klv = read_klv(&game_config, &a.klv)?;
            openness_moves(
                game_config,
                kwg,
                klv,
                OpennessMovesParams {
                    num_games: a.games,
                    seed: a.seed,
                    threads,
                    window: (a.window * equity::SCALE as f64).round() as i32,
                    num_candidates: a.candidates,
                    quiet: a.quiet,
                },
            )
        }
        Task::Position(a) => {
            let kwg = read_kwg::<N>(&game_config, &a.kwg)?;
            let klv = read_klv(&game_config, &a.klv)?;
            inspect_position(
                game_config,
                kwg,
                klv,
                PositionParams {
                    fen: &a.board,
                    rack: a.rack.as_deref(),
                    num_moves: a.moves,
                    dynamic_out: a.dynamic.as_deref(),
                    threads,
                    lean: a.lean,
                },
            )
        }
        Task::SimChunkCheck(a) => {
            let klv = read_klv(&game_config, &a.klv)?;
            let kwg = read_kwg::<N>(&game_config, &a.kwg)?;
            sim_chunk_check(game_config, kwg, klv, &a)
        }
        Task::SimMutateCheck(a) => {
            let klv = read_klv(&game_config, &a.klv)?;
            let kwg = read_kwg::<N>(&game_config, &a.kwg)?;
            let arm_tables = anagrams::ArmTables::build(&kwg, &game_config, a.lean)?;
            let (iters, seed) = (a.iters, a.seed);
            let mut rng = GameRng::seed_from_u64(seed);
            let mut game_state = game_state::GameState::new(&game_config);
            game_state.reset_and_draw_tiles(&game_config, &mut rng);
            let mut move_generator = movegen::KurniaMoveGenerator::new(&game_config);
            move_generator.gen_moves_unfiltered(&movegen::GenMovesParams {
                board_snapshot: &movegen::BoardSnapshot {
                    board_tiles: &game_state.board_tiles,
                    game_config: &game_config,
                    kwg: &kwg,
                    anagrams: arm_tables.anagrams(),
                    rack_lengths: arm_tables.rack_lengths(),
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
                &arm_tables,
                &klv,
                move_picker::SimmerParams {
                    num_sim_iters: iters,
                    num_sim_plies: move_picker::DEFAULT_NUM_SIM_PLIES,
                    sim_threads: 1,
                    win_pct_table: None,
                    quiet: a.quiet,
                },
            )?;
            driver.reseed(seed);
            let opening = (iters / 4).max(1);
            driver.begin_decision(&move_generator, &game_state, opening)?;
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
                writeln!(
                    std::io::stdout(),
                    "retire stream {by_hand}: count {at_retire} -> {kept}, retired={now_retired} active={still_active}",
                )?;
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
            driver.resume(&move_generator, iters.saturating_sub(opening))?;
            let pruned_id = driver.retired_stream_ids().next();
            match pruned_id {
                None => wolges::return_error!(
                    "no candidates were pruned; raise the iteration budget".to_string()
                ),
                Some(id) => {
                    let before = driver.stream_count(id).unwrap();
                    let readmitted = driver.readmit_with_history(id);
                    driver.resume(&move_generator, iters)?;
                    let after = driver.stream_count(id).unwrap();
                    writeln!(
                        std::io::stdout(),
                        "readmit stream {id}: count before={before} after={after}"
                    )?;
                    if readmitted && after >= before {
                        writeln!(std::io::stdout(), "SIM_MUTATE_OK")?;
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
            sim_field_check(game_config, kwg, klv, a.seed, a.turns, a.lean)
        }
        Task::Winpct(a) => {
            let kwg = read_kwg::<N>(&game_config, &a.kwg)?;
            let klv = std::sync::Arc::new(read_klv(&game_config, &a.leave)?);
            generate_winpct_table(
                game_config,
                kwg,
                klv,
                &a.out,
                GamesParams {
                    num_games: a.games,
                    seed: a.seed,
                    threads,
                    lean: a.lean,
                    quiet: a.quiet,
                },
            )
        }
        Task::WinpctEval(a) => {
            let kwg = read_kwg::<N>(&game_config, &a.kwg)?;
            let klv = std::sync::Arc::new(read_klv(&game_config, &a.leave)?);
            let table = win_pct::WinPctTable::from_csv(make_reader(&a.table)?)?;
            generate_winpct_eval(
                game_config,
                kwg,
                klv,
                table,
                GamesParams {
                    num_games: a.games,
                    seed: a.seed,
                    threads,
                    lean: a.lean,
                    quiet: a.quiet,
                },
            )
        }
        Task::WinpctCombine(a) => {
            let acc = win_pct::WinPctAccumulator::from_csv(make_reader(&a.input)?)?;
            acc.to_csv(make_writer(&a.output)?)
        }
        Task::Summarize(a) => generate_summary(
            game_config,
            make_reader(&a.input)?,
            csv::Writer::from_writer(make_writer(&a.output)?),
            a.quiet,
        ),
        Task::Resummarize(a) => resummarize_summaries::<'a', _, _>(
            game_config,
            csv::ReaderBuilder::new()
                .has_headers(false)
                .from_reader(make_reader(&a.input)?),
            csv::Writer::from_writer(make_writer(&a.output)?),
            a.quiet,
        ),
        Task::ResummarizePlayability(a) => resummarize_summaries::<'p', _, _>(
            game_config,
            csv::ReaderBuilder::new()
                .has_headers(false)
                .from_reader(make_reader(&a.input)?),
            csv::Writer::from_writer(make_writer(&a.output)?),
            a.quiet,
        ),
        Task::ResummarizePlayabilityAll(a) => resummarize_summaries::<'P', _, _>(
            game_config,
            csv::ReaderBuilder::new()
                .has_headers(false)
                .from_reader(make_reader(&a.input)?),
            csv::Writer::from_writer(make_writer(&a.output)?),
            a.quiet,
        ),
        Task::Generate(a) => generate_leaves::<_, _, false>(
            game_config,
            csv::ReaderBuilder::new()
                .has_headers(false)
                .from_reader(make_reader(&a.summary)?),
            csv::Writer::from_writer(make_writer(&a.leaves)?),
            a.quiet,
        ),
        Task::GenerateFull(a) => generate_leaves::<_, _, true>(
            game_config,
            csv::ReaderBuilder::new()
                .has_headers(false)
                .from_reader(make_reader(&a.summary)?),
            csv::Writer::from_writer(make_writer(&a.leaves)?),
            a.quiet,
        ),
        Task::Playability(a) => {
            let kwg = read_kwg::<N>(&game_config, &a.kwg)?;
            let klv = read_klv(&game_config, &a.leave)?;
            discover_playability(
                game_config,
                kwg,
                klv,
                GamesParams {
                    num_games: a.games,
                    seed: a.seed,
                    threads,
                    lean: a.lean,
                    quiet: a.quiet,
                },
            )
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
        .or_else(|| std::thread::available_parallelism().ok())
        .map_or(1, std::num::NonZeroUsize::get);
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
    lean: bool,
    quiet: bool,
}

struct GamesParams {
    num_games: u64,
    seed: Option<u64>,
    threads: usize,
    lean: bool,
    quiet: bool,
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
        lean,
        quiet,
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
    let seed = seed_or_random(seed)?;
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

    let arm_tables = anagrams::ArmTables::build(&kwg, &game_config, lean)?;
    let anagrams = arm_tables.anagrams();
    let rack_lengths = arm_tables.rack_lengths();
    // a thread that fails sets this, so the others stop rather than wait on it
    let failed = std::sync::atomic::AtomicBool::new(false);

    std::thread::scope(|s| -> error::Returns<()> {
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
            let failed = &failed;
            threads.push(s.spawn(move || -> error::Returns<()> {
                let fail = |e: error::BoxAnyError| {
                    failed.store(true, std::sync::atomic::Ordering::Relaxed);
                    e
                };
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
                    if failed.load(std::sync::atomic::Ordering::Relaxed) {
                        return Ok(());
                    }
                    let mut num_prior_games =
                        num_processed_games.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    let mut rng = GameRng::from_stream(seed, num_prior_games);
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
                                && !failed.load(std::sync::atomic::Ordering::Relaxed)
                            {}
                            if failed.load(std::sync::atomic::Ordering::Relaxed) {
                                return Ok(());
                            }
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
                                        && !failed.load(std::sync::atomic::Ordering::Relaxed)
                                    {}
                                    if failed.load(std::sync::atomic::Ordering::Relaxed) {
                                        return Ok(());
                                    }
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
                            anagrams,
                            rack_lengths,
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

                        game_state
                            .play(&game_config, &mut rng, &play.play)
                            .map_err(fail)?;

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
                            let game_ended = game_state
                                .check_game_ended(&game_config, &mut final_scores)
                                .map_err(fail)?;

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
                                            c.write_all(&batched_csv_log_buf)
                                                .map_err(|e| fail(e.into()))?;
                                        }
                                        mutex_guard
                                            .csv_game_writer
                                            .write_all(&batched_csv_game_buf)
                                            .map_err(|e| fail(e.into()))?;
                                        if !quiet && mutex_guard.tick_periods.update(elapsed_time_secs) {
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
                    c.write_all(&batched_csv_log_buf)
                        .map_err(|e| fail(e.into()))?;
                }
                mutex_guard
                    .csv_game_writer
                    .write_all(&batched_csv_game_buf)
                    .map_err(|e| fail(e.into()))?;

                if SUMMARIZE {
                    merge_rack_map(&mut mutex_guard.full_rack_map, &mut thread_full_rack_map);
                }
                Ok(())
            }));
        }

        for thread in threads {
            match thread.join() {
                Ok(done) => done?,
                Err(e) => {
                    writeln!(boxed_stdout_or_stderr(), "{e:?}").ok();
                }
            }
        }
        Ok(())
    })?;

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
    GamesParams {
        num_games,
        seed,
        threads,
        lean,
        quiet,
    }: GamesParams,
) -> error::Returns<()> {
    let game_config = std::sync::Arc::new(game_config);
    let kwg = std::sync::Arc::new(kwg);
    let seed = seed_or_random(seed)?;
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

    let arm_tables = anagrams::ArmTables::build(&kwg, &game_config, lean)?;
    let anagrams = arm_tables.anagrams();
    let rack_lengths = arm_tables.rack_lengths();

    std::thread::scope(|s| -> error::Returns<()> {
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
            threads.push(s.spawn(move || -> error::Returns<()> {
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

                    let mut rng = GameRng::from_stream(seed, num_prior_games);
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
                                    anagrams,
                                    rack_lengths,
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
                            anagrams,
                            rack_lengths,
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
                        game_state.play(&game_config, &mut rng, &play.play)?;
                        let game_ended =
                            game_state.check_game_ended(&game_config, &mut final_scores)?;
                        game_state.next_turn();
                        turn_idx += 1;
                        if !matches!(game_ended, game_state::CheckGameEnded::NotEnded) {
                            break;
                        }
                    }
                    completed_games.fetch_add(1, std::sync::atomic::Ordering::Relaxed);

                    let elapsed = t0.elapsed().as_secs();
                    let mut tick = mutexed_tick.lock().unwrap();
                    if !quiet && tick.update(elapsed) {
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
                Ok(())
            }));
        }
        for thread in threads {
            match thread.join() {
                Ok(done) => done?,
                Err(e) => {
                    writeln!(boxed_stdout_or_stderr(), "{e:?}").ok();
                }
            }
        }
        Ok(())
    })?;

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
) -> error::Returns<()> {
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
    quiet: bool,
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
            if record.len() < 11 {
                return Err("a log line has fewer than 11 fields".into());
            }
            if i16::from_str(&record[10])? > 0 {
                let equity = f32::from_str(&record[9])? as f64;
                //let score = i16::from_str(&record[5])? as i64;
                parse_rack(&rack_reader, &record[3], &mut rack_bytes)?;
                rack_bytes.sort_unstable();
                row_count += 1;
                pool_one(&mut full_rack_map, &rack_bytes[..], equity);
                let elapsed_time_secs = t0.elapsed().as_secs();
                if !quiet && tick_periods.update(elapsed_time_secs) {
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
    quiet: bool,
) -> error::Returns<()> {
    let mut stdout_or_stderr = boxed_stdout_or_stderr();
    let mut rack_bytes = Vec::new();
    let rack_reader = alphabet::AlphabetReader::new_for_racks(game_config.alphabet());
    let mut full_rack_map = fash::MyHashMap::<bites::Bites, Cumulate>::default();
    let t0 = std::time::Instant::now();
    let mut tick_periods = move_picker::Periods(0);
    for (rows_read, result) in (1u64..).zip(csv_in.records()) {
        let elapsed_time_secs = t0.elapsed().as_secs();
        if !quiet && tick_periods.update(elapsed_time_secs) {
            writeln!(
                stdout_or_stderr,
                "After {elapsed_time_secs} seconds, have read {rows_read} rows"
            )?;
        }
        let record = result?;
        if record.len() < 3 {
            return Err("a summary line has fewer than 3 fields".into());
        }
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

// https://doi.org/10.1145/2714064.2660195
// https://prng.di.unimi.it/splitmix64.c
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
    phase: BoardPhase,
}

#[derive(Clone, Copy, PartialEq)]
enum BoardPhase {
    Opening,
    Midgame,
    Endgame,
}

impl BoardPhase {
    #[inline(always)]
    fn of_board(board_tiles: &[u8], game_config: &game_config::GameConfig) -> Self {
        if board_tiles.iter().all(|&t| t == 0) {
            return BoardPhase::Opening;
        }
        if bag_size(board_tiles, game_config) <= 0 {
            BoardPhase::Endgame
        } else {
            BoardPhase::Midgame
        }
    }

    #[inline]
    fn name(self) -> &'static str {
        match self {
            BoardPhase::Opening => "opening",
            BoardPhase::Midgame => "midgame",
            BoardPhase::Endgame => "endgame",
        }
    }
}

#[inline]
fn opening_adjustment(
    phase: BoardPhase,
    game_config: &game_config::GameConfig,
    down: bool,
    idx: i8,
    word: &[u8],
) -> i32 {
    if phase != BoardPhase::Opening {
        return 0;
    }
    let alphabet = game_config.alphabet();
    let board_layout = game_config.board_layout();
    -(equity::OPENING_HOTSPOT_PENALTY
        * (idx..)
            .zip(word)
            .filter(|&(i, &tile)| {
                tile != 0
                    && alphabet.is_vowel(tile)
                    && if down {
                        board_layout.danger_star_down(i)
                    } else {
                        board_layout.danger_star_across(i)
                    }
            })
            .count() as i32)
}

#[inline]
fn endgame_leave_value(game_config: &game_config::GameConfig, tally: &[u8]) -> i32 {
    let alphabet = game_config.alphabet();
    let held: i32 = tally
        .iter()
        .enumerate()
        .map(|(t, &n)| n as i32 * alphabet.score(t as u8) as i32)
        .sum();
    -equity::ENDGAME_PENALTY_BASE - 2 * held * equity::SCALE
}

#[inline]
fn play_out_bonus(game_config: &game_config::GameConfig, unseen_tally: &[u8], rack: &[u8]) -> i32 {
    let alphabet = game_config.alphabet();
    let mut left: i32 = unseen_tally
        .iter()
        .enumerate()
        .map(|(t, &n)| n as i32 * alphabet.score(t as u8) as i32)
        .sum();
    for &t in rack {
        left -= alphabet.score(t) as i32;
    }
    2 * left * equity::SCALE
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
        phase,
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
        anagrams: None,
        rack_lengths: None,
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
        movegen::PlacePredicate::Dyn(&mut |down, lane, idx, word: &[u8], _score: i32| {
            n_cand += 1;
            let real_score = play_scorer::score_and_blank_deltas(
                board_snapshot,
                down,
                lane,
                idx,
                word,
                blank_deltas,
            ) + opening_adjustment(phase, game_config, down, idx, word);
            census::record_blank_variants(
                lat,
                sheet,
                real_score,
                blank_deltas,
                unseen_tally,
                num_blanks_eff,
            );
            false // never keep the move
        }),
        klv::AdjustLeave::Identity,
        movegen::EquityPredicate::RejectAll,
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
        build::BuildOrder::Sorted,
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
    quiet: bool,
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
        quiet,
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

    let lat = census::MultisetLattice::new(num_letters, rack_size)?;
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

    // from a pool of this many unseen tiles, the superset-sum pass (one sweep
    // of the lattice a letter, whatever the pool) apportions faster than
    // pushing each drawable rack into its subracks, which costs more the more
    // racks the pool can deal. Both give the same values; the crossover was
    // timed on the english tiles.
    let zeta_pool_min = 36;

    // up to this many multisets, filling every rack's best from the words each
    // spells beats searching each rack's splits. The cutoff sits between the
    // lattices of the 27 to 29 letter alphabets, where it was timed faster, and
    // the 33 letter ones, where it was slower. Both give the same values.
    let scatter = lat.len() <= 12_000_000;

    let base_freqs: Vec<u8> = (0..alphabet.len()).map(|t| alphabet.freq(t)).collect();

    let seed = seed_or_random(seed)?;

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
        let resume_klv = klv::Klv::<L>::from_bytes_alloc(&bytes)?;
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
    // a thread that fails sets this; every thread then stops at the next
    // barrier instead of leaving the others waiting there
    let failed = std::sync::atomic::AtomicBool::new(false);

    let sheet_cache: Vec<SheetCacheSlot> = (0..sheet_cache_len)
        .map(|_| std::sync::Mutex::new(None))
        .collect();
    writeln!(
        boxed_stdout_or_stderr(),
        "census: {num_threads} threads over {board_counts:?} boards/gen"
    )?;

    std::thread::scope(|s| -> error::Returns<()> {
        let mut handles = Vec::with_capacity(num_threads);
        for _ in 0..num_threads {
            handles.push(s.spawn(|| -> error::Returns<()> {
                let mut error = None;

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
                            phase: BoardPhase::Midgame,
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
                    if !quiet {
                        writeln!(boxed_stdout_or_stderr(),
                            "census: board {}/{} done ({}s), {} of {} leaves valued so far",
                            *completed,
                            cur_boards,
                            t0.elapsed().as_secs(),
                            *valued,
                            globally_possible_count,).ok();
                    }
                };


                let mut gen_idx = start_gen;

                let mut num_boards = board_counts[gen_idx];

                let mut prior_max_boards = 0usize;
                loop {

                    {
                        let leave = leave_lock.read().unwrap();

                        let null_leave = leave.iter().all(|&x| x == 0);
                        'boards: loop {
                            if failed.load(std::sync::atomic::Ordering::Relaxed) {
                                break;
                            }
                            let b = next_board.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                            if b >= num_boards {
                                break;
                            }
                    let mut rng = GameRng::seed_from_u64(census_mix64(
                        seed.wrapping_add(census_mix64(b)),
                    ));

                    let reuse_board = sheet_reuse && (b as usize) < prior_max_boards;
                    if !reuse_board {

                    let target = if high_tiles <= low_tiles {
                        low_tiles
                    } else {

                        low_tiles + (b as usize % (high_tiles - low_tiles + 1))
                    };

                    let mut tries = 0u64;
                    let t_slot = std::time::Instant::now();
                    let mut ticked = 0u64;
                    loop {
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
                                anagrams: None,
                                rack_lengths: None,
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
                            let ended = match game_state
                                .play(&game_config, &mut rng, &move_generator.plays[0].play)
                                .and_then(|()| {
                                    game_state.check_game_ended(&game_config, &mut final_scores)
                                }) {
                                Ok(ended) => ended,
                                Err(e) => {
                                    error = Some(e);
                                    failed.store(true, std::sync::atomic::Ordering::Relaxed);
                                    break 'boards;
                                }
                            };
                            game_state.next_turn();
                            if !matches!(ended, game_state::CheckGameEnded::NotEnded) {
                                break; // game ended before the window; try a fresh game.
                            }
                        }
                        if got {
                            break;
                        }
                        tries += 1;
                        // a slot plays fresh games until one stops in the
                        // window, and says so each second it keeps missing
                        let secs = t_slot.elapsed().as_secs();
                        if !quiet && secs > ticked {
                            ticked = secs;
                            writeln!(boxed_stdout_or_stderr(),
                                "census: board slot {b} after {secs}s, {tries} games, none stopped in window [{low_tiles},{high_tiles}]").ok();
                        }
                    }
                    } // end of the !reuse_board game replay
                    value_board(
                        &mut move_generator,
                        &game_state,
                        &leave,
                        null_leave,
                        !quiet && b == 0,

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
                        if barrier.wait().is_leader()
                            && !failed.load(std::sync::atomic::Ordering::Relaxed)
                        {

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
                                if !quiet {
                                    writeln!(boxed_stdout_or_stderr(),
                                        "census: gen {}/{} done ({}s), {} of {} leaves valued",
                                        gen_idx + 1,
                                        gens,
                                        t0.elapsed().as_secs(),
                                        *valued,
                                        lat_len,).ok();
                                }
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
                        if gen_idx + 1 < gens && !failed.load(std::sync::atomic::Ordering::Relaxed) {

                            prior_max_boards = prior_max_boards.max(num_boards as usize);
                            gen_idx += 1;
                            num_boards = board_counts[gen_idx];
                            continue;
                        }
                    }
                    break;
                }
                match error {
                    Some(e) => Err(e),
                    None => Ok(()),
                }
            }));
        }
        for handle in handles {
            handle.join().unwrap()?;
        }
        Ok(())
    })?;

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

    let summary_name = claim_output_path(&format!("census-summary-{census_run_epoch}.csv"))?;
    let mut summary_out = csv::Writer::from_path(&summary_name)?;
    let full_racks = lat.full_rack_start()..lat.len();
    let mut total_points = 0f64;
    let mut total_count = 0u64;
    for idx in full_racks.clone() {
        if accum_cnt[idx] > 0 {
            total_points += accum_sum[idx] / equity::SCALE as f64;
            total_count += accum_cnt[idx];
        }
    }
    summary_out.serialize(("", total_points, total_count))?;
    let mut num_racks = 0usize;
    for idx in full_racks {
        if accum_cnt[idx] == 0 {
            continue;
        }
        lat.unrank_into(idx, &mut tally_buf);
        leave_ser.clear();
        for (t, &c) in tally_buf.iter().enumerate() {
            for _ in 0..c {
                leave_ser.push_str(alphabet.of_rack(t as u8).unwrap());
            }
        }
        summary_out.serialize((
            &leave_ser,
            accum_sum[idx] / equity::SCALE as f64,
            accum_cnt[idx],
        ))?;
        num_racks += 1;
    }
    summary_out.flush()?;
    writeln!(
        boxed_stdout_or_stderr(),
        "census: wrote the last generation's summary to {summary_name} ({num_racks} full racks)"
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
    quiet: bool,
) -> error::Returns<()> {
    let mut stdout_or_stderr = boxed_stdout_or_stderr();

    let mut rack_tally = vec![0u8; game_config.alphabet().len() as usize];
    let mut exchange_buffer = Vec::with_capacity(game_config.rack_size() as usize);
    let mut rack_bytes = Vec::new();
    let rack_reader = alphabet::AlphabetReader::new_for_racks(game_config.alphabet());
    let mut full_rack_map = fash::MyHashMap::<bites::Bites, Cumulate>::default();
    let t0 = std::time::Instant::now();
    let mut tick_periods = move_picker::Periods(0);
    for (rows_read, result) in (1u64..).zip(csv_in.records()) {
        let elapsed_time_secs = t0.elapsed().as_secs();
        if !quiet && tick_periods.update(elapsed_time_secs) {
            writeln!(
                stdout_or_stderr,
                "After {elapsed_time_secs} seconds, have read {rows_read} rows"
            )?;
        }
        let record = result?;
        if record.len() < 3 {
            return Err("a summary line has fewer than 3 fields".into());
        }
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
            if !quiet && tick_periods.update(elapsed_time_secs) {
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
    let mut written = Ok(());
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
            if !quiet && tick_periods.update(elapsed_time_secs) && written.is_ok() {
                written = writeln!(
                    stdout_or_stderr,
                    "After {} seconds, have processed {} subracks and smoothed {}",
                    elapsed_time_secs,
                    ev_map.len(),
                    num_smoothed,
                );
            }
        },
        rack_tally: &mut alphabet_freqs,
        min_len: 1,
        max_len: leave_size,
        exchange_buffer: &mut exchange_buffer,
    });
    written?;
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
        let mut written = Ok(());
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
                    } else if written.is_ok() {
                        written = writeln!(
                            stdout_or_stderr,
                            "not enough samples to derive {rack_bytes:?}"
                        );
                    }
                }
                let elapsed_time_secs = t0.elapsed().as_secs();
                if !quiet && tick_periods.update(elapsed_time_secs) && written.is_ok() {
                    written = writeln!(
                        stdout_or_stderr,
                        "After {elapsed_time_secs} seconds, filling in the {len_to_complete}-tile subracks (up to {leave_size}), have filled in {num_filled_in}",
                    );
                }
            },
            rack_tally: &mut alphabet_freqs,
            min_len: len_to_complete,
            max_len: len_to_complete,
            exchange_buffer: &mut exchange_buffer,
        });
        written?;
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
    GamesParams {
        num_games,
        seed,
        threads,
        lean,
        quiet,
    }: GamesParams,
) -> error::Returns<()> {
    let game_config = std::sync::Arc::new(game_config);
    let kwg = std::sync::Arc::new(kwg);
    let klv = std::sync::Arc::new(klv);
    let seed = seed_or_random(seed)?;
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

    let arm_tables = anagrams::ArmTables::build(&kwg, &game_config, lean)?;
    let anagrams = arm_tables.anagrams();
    let rack_lengths = arm_tables.rack_lengths();

    std::thread::scope(|s| -> error::Returns<()> {
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
            threads.push(s.spawn(move || -> error::Returns<()> {
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
                    let mut rng = GameRng::from_stream(seed, num_prior_games);

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
                            anagrams,
                            rack_lengths,
                            klv: &klv,
                        };

                        let moves_made_before_ending: u64 = if old_bag_len > 0 {
                            let mut best_equity_so_far = equity::Equity::NEG_INFINITY;
                            let mut num_plays = 0usize;
                            vec_played.clear();
                            let mut best_play_seen = |equity: equity::Equity, play: &movegen::Play| {
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
                                };
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
                                movegen::PlacePredicate::AcceptAll,
                                klv::AdjustLeave::Identity,
                                movegen::EquityPredicate::Dyn(&mut best_play_seen),
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

                            game_state.play(&game_config, &mut rng, &play.play)?;

                            match game_state.check_game_ended(&game_config, &mut final_scores)? {
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
                                if !quiet && tick_changed {
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
                Ok(())
            }));
        }

        for thread in threads {
            match thread.join() {
                Ok(done) => done?,
                Err(e) => {
                    writeln!(boxed_stdout_or_stderr(), "{e:?}").ok();
                }
            }
        }
        Ok(())
    })?;

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
    fn print(&self, label: &str) -> std::io::Result<()> {
        let total = self.total_games();
        if total == 0 {
            return Ok(());
        }
        let p0_total = self.p0_wins as f64 + self.p0_draws as f64 / 2.0;
        let p1_total = total as f64 - p0_total;
        writeln!(std::io::stdout(), "{label}")?;
        writeln!(
            std::io::stdout(),
            "  turns per game: {:.2} (sd={:.2})",
            self.turns.mean(),
            self.turns.standard_deviation(),
        )?;
        writeln!(
            std::io::stdout(),
            "  played out: {} ({:.2}%)  zero scores: {} ({:.2}%)",
            self.played_out,
            self.played_out as f64 / total as f64 * 100.0,
            self.zero_scores,
            self.zero_scores as f64 / total as f64 * 100.0,
        )?;
        writeln!(
            std::io::stdout(),
            "  p0 (klv0): {:.1} ({:.2}%)  p1 (klv1): {:.1} ({:.2}%)",
            p0_total,
            p0_total / total as f64 * 100.0,
            p1_total,
            p1_total / total as f64 * 100.0,
        )?;
        writeln!(
            std::io::stdout(),
            "  wins: {} ({:.2}%)  losses: {} ({:.2}%)  draws: {} ({:.2}%)",
            self.p0_wins,
            self.p0_wins as f64 / total as f64 * 100.0,
            self.p0_losses,
            self.p0_losses as f64 / total as f64 * 100.0,
            self.p0_draws,
            self.p0_draws as f64 / total as f64 * 100.0,
        )?;
        writeln!(
            std::io::stdout(),
            "  score: p0={:.2} (sd={:.2})  p1={:.2} (sd={:.2})",
            self.p0_score.mean(),
            self.p0_score.standard_deviation(),
            self.p1_score.mean(),
            self.p1_score.standard_deviation(),
        )?;
        let corrected_pct = (p0_total.max(p1_total) - 0.5) / total as f64;
        if corrected_pct > 0.5 {
            let z = (corrected_pct - 0.5) * 2.0 * (total as f64).sqrt();
            let confidence = stats::NormalDistribution::cumulative_normal_density(z) * 100.0;
            let leading = if p0_total > p1_total {
                "p0 (klv0)"
            } else {
                "p1 (klv1)"
            };
            writeln!(
                std::io::stdout(),
                "  {leading} leads, confidence: {confidence:.2}%"
            )?;
        } else {
            writeln!(std::io::stdout(), "  no significant difference")?;
        }
        Ok(())
    }

    #[inline]
    fn print_porcelain(&self) -> std::io::Result<()> {
        let total = self.total_games();
        if total == 0 {
            return Ok(());
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
        writeln!(
            std::io::stdout(),
            "WCMP_P0_PCT {:.4}",
            p0_total / total as f64 * 100.0
        )?;
        writeln!(
            std::io::stdout(),
            "WCMP_P1_PCT {:.4}",
            p1_total / total as f64 * 100.0
        )?;
        writeln!(
            std::io::stdout(),
            "WCMP_P0_WINS_PCT {:.4}",
            self.p0_wins as f64 / total as f64 * 100.0
        )?;
        writeln!(
            std::io::stdout(),
            "WCMP_DRAWS_PCT {:.4}",
            self.p0_draws as f64 / total as f64 * 100.0
        )?;
        writeln!(std::io::stdout(), "WCMP_CONF_PCT {confidence:.4}")?;
        writeln!(
            std::io::stdout(),
            "WCMP_LEADER {}",
            if p0_total >= p1_total { 0 } else { 1 }
        )?;
        writeln!(std::io::stdout(), "WCMP_GAMES {total}")?;
        writeln!(std::io::stdout(), "WCMP_PAIRS {}", total / 2)?;
        Ok(())
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

    fn print(&self) -> std::io::Result<()> {
        let all_total = self.all.total_games();
        let all_pairs = all_total / 2;
        self.all.print(&format!(
            "{all_total} {} ({all_pairs} {}):",
            plural(all_total, "game", "games"),
            plural(all_pairs, "pair", "pairs"),
        ))?;
        let div_total = self.divergent.total_games();
        if div_total > 0 && div_total < all_total {
            let div_pairs = div_total / 2;
            self.divergent.print(&format!(
                "\n{div_total} divergent {} ({div_pairs} {} = {:.2}%):",
                plural(div_total, "game", "games"),
                plural(div_pairs, "pair", "pairs"),
                div_pairs as f64 / all_pairs as f64 * 100.0,
            ))?;
        }

        self.all.print_porcelain()
    }
}

struct WinpctTables<'a, N: kwg::Node, L: kwg::Node> {
    game_config: &'a game_config::GameConfig,
    kwg: &'a kwg::Kwg<N>,
    anagrams: Option<&'a wolges::anagrams::Anagrams>,
    rack_lengths: Option<&'a wolges::anagrams::RackLengths>,
    arc_klv: &'a klv::Klv<L>,
}

#[inline]
fn winpct_play_game<N: kwg::Node, L: kwg::Node>(
    tables: WinpctTables<'_, N, L>,
    move_generator: &mut movegen::KurniaMoveGenerator,
    game_state: &mut game_state::GameState,
    rng: &mut GameRng,
    snapshots: &mut Vec<(usize, usize, usize, usize, i32)>,
    final_scores: &mut [i32],
) -> error::Returns<()> {
    let WinpctTables {
        game_config,
        kwg,
        anagrams,
        rack_lengths,
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
            anagrams,
            rack_lengths,
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
        game_state.play(game_config, rng, play)?;
        match game_state.check_game_ended(game_config, final_scores)? {
            game_state::CheckGameEnded::NotEnded => {}
            _ => break,
        }
        game_state.next_turn();
    }
    Ok(())
}

#[inline]
fn generate_winpct_table<N: kwg::Node + Sync + Send, L: kwg::Node + Sync + Send>(
    game_config: game_config::GameConfig,
    kwg: kwg::Kwg<N>,
    arc_klv: std::sync::Arc<klv::Klv<L>>,
    out_path: &str,
    GamesParams {
        num_games,
        seed,
        threads,
        lean,
        quiet,
    }: GamesParams,
) -> error::Returns<()> {
    let t0 = std::time::Instant::now();
    let game_config = std::sync::Arc::new(game_config);
    let seed = seed_or_random(seed)?;
    let num_threads = threads.max(1).min(num_games.max(1) as usize);
    writeln!(
        boxed_stdout_or_stderr(),
        "winpct: seed {seed}, {num_games} games, {num_threads} threads"
    )?;
    let kwg = std::sync::Arc::new(kwg);
    let next_game = std::sync::atomic::AtomicU64::new(0);
    let finished_games = std::sync::atomic::AtomicU64::new(0);
    let tick = std::sync::atomic::AtomicU64::new(0);
    let shared = std::sync::Mutex::new(win_pct::WinPctAccumulator::new());

    let arm_tables = anagrams::ArmTables::build(&kwg, &game_config, lean)?;
    let anagrams = arm_tables.anagrams();
    let rack_lengths = arm_tables.rack_lengths();

    std::thread::scope(|s| -> error::Returns<()> {
        let mut handles = Vec::with_capacity(num_threads);
        for _ in 0..num_threads {
            handles.push(s.spawn(|| -> error::Returns<()> {
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
                    let mut rng = GameRng::from_stream(seed, g);
                    game_state.reset_and_draw_tiles_double_ended(&game_config, &mut rng);
                    winpct_play_game(
                        WinpctTables {
                            game_config: &game_config,
                            kwg: &kwg,
                            anagrams,
                            rack_lengths,
                            arc_klv: &arc_klv,
                        },
                        &mut move_generator,
                        &mut game_state,
                        &mut rng,
                        &mut snapshots,
                        &mut final_scores,
                    )?;

                    for &(bag, my, opp, mover, lead) in &snapshots {
                        let mover_final = equity::descale_score(final_scores[mover])
                            - equity::descale_score(final_scores[1 - mover]);
                        acc.record(bag, my, opp, lead, mover_final);
                    }
                    let done =
                        finished_games.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
                    let secs = t0.elapsed().as_secs();
                    if !quiet && tick.fetch_max(secs, std::sync::atomic::Ordering::Relaxed) < secs {
                        writeln!(
                            boxed_stdout_or_stderr(),
                            "After {secs} seconds, have played {done} of {num_games} games"
                        )
                        .ok();
                    }
                }
                shared.lock().unwrap().merge(&acc);
                Ok(())
            }));
        }
        for handle in handles {
            handle.join().unwrap()?;
        }
        Ok(())
    })?;

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
    GamesParams {
        num_games,
        seed,
        threads,
        lean,
        quiet,
    }: GamesParams,
) -> error::Returns<()> {
    let t0 = std::time::Instant::now();
    let game_config = std::sync::Arc::new(game_config);
    let table = std::sync::Arc::new(table);
    let seed = seed_or_random(seed)?;
    let num_threads = threads.max(1).min(num_games.max(1) as usize);
    writeln!(
        boxed_stdout_or_stderr(),
        "winpct-eval: seed {seed}, {num_games} games, {num_threads} threads"
    )?;
    let kwg = std::sync::Arc::new(kwg);
    let next_game = std::sync::atomic::AtomicU64::new(0);
    let finished_games = std::sync::atomic::AtomicU64::new(0);
    let tick = std::sync::atomic::AtomicU64::new(0);

    let ln_ratio = (1.0f64 / 0.9 - 1.0).ln();

    let shared = std::sync::Mutex::new((0.0f64, 0.0f64, 0u64));

    let arm_tables = anagrams::ArmTables::build(&kwg, &game_config, lean)?;
    let anagrams = arm_tables.anagrams();
    let rack_lengths = arm_tables.rack_lengths();

    std::thread::scope(|s| -> error::Returns<()> {
        let mut handles = Vec::with_capacity(num_threads);
        for _ in 0..num_threads {
            handles.push(s.spawn(|| -> error::Returns<()> {
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
                    let mut rng = GameRng::from_stream(seed, g);
                    game_state.reset_and_draw_tiles_double_ended(&game_config, &mut rng);
                    winpct_play_game(
                        WinpctTables {
                            game_config: &game_config,
                            kwg: &kwg,
                            anagrams,
                            rack_lengths,
                            arc_klv: &arc_klv,
                        },
                        &mut move_generator,
                        &mut game_state,
                        &mut rng,
                        &mut snapshots,
                        &mut final_scores,
                    )?;
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
                    let done =
                        finished_games.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
                    let secs = t0.elapsed().as_secs();
                    if !quiet && tick.fetch_max(secs, std::sync::atomic::Ordering::Relaxed) < secs {
                        writeln!(
                            boxed_stdout_or_stderr(),
                            "After {secs} seconds, have played {done} of {num_games} games"
                        )
                        .ok();
                    }
                }
                let mut acc = shared.lock().unwrap();
                acc.0 += bt;
                acc.1 += bs;
                acc.2 += n;
                Ok(())
            }));
        }
        for handle in handles {
            handle.join().unwrap()?;
        }
        Ok(())
    })?;

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

#[derive(Clone, Copy, clap::ValueEnum)]
enum SeatPicker {
    Hasty,
    Dynamic,
    Simmer,
    Deft,
}

#[inline]
fn picker_name(picker: SeatPicker) -> &'static str {
    match picker {
        SeatPicker::Hasty => "hasty",
        SeatPicker::Dynamic => "dynamic",
        SeatPicker::Simmer => "simmer",
        SeatPicker::Deft => "deft",
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
    lean: bool,
) -> error::Returns<()> {
    let alphabet = game_config.alphabet();
    let rack_reader = alphabet::AlphabetReader::new_for_racks(alphabet);
    let arm_tables = anagrams::ArmTables::build(&kwg, &game_config, lean)?;
    let mut rng = GameRng::seed_from_u64(seed);
    let mut game_state = game_state::GameState::new(&game_config);
    game_state.reset_and_draw_tiles(&game_config, &mut rng);
    let mut move_generator = movegen::KurniaMoveGenerator::new(&game_config);
    let mut filtered_movegen = move_filter::GenMoves::Unfiltered;
    let mut scratch = Vec::new();
    let play_one_greedy_turn = |game_state: &mut game_state::GameState,
                                rng: &mut GameRng,
                                move_generator: &mut movegen::KurniaMoveGenerator|
     -> error::Returns<bool> {
        move_generator.gen_moves_unfiltered(&movegen::GenMovesParams {
            board_snapshot: &movegen::BoardSnapshot {
                board_tiles: &game_state.board_tiles,
                game_config: &game_config,
                kwg: &kwg,
                anagrams: arm_tables.anagrams(),
                rack_lengths: arm_tables.rack_lengths(),
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
        match game_state.check_game_ended(&game_config, &mut final_scores)? {
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
     -> std::io::Result<usize> {
        let board_snapshot = movegen::BoardSnapshot {
            board_tiles: &game_state.board_tiles,
            game_config: &game_config,
            kwg: &kwg,
            anagrams: arm_tables.anagrams(),
            rack_lengths: arm_tables.rack_lengths(),
            klv: &klv,
        };
        let rack = &game_state.current_player().rack;
        let num_exchanges = game_state.current_player().num_exchanges;
        move_generator.gen_moves_unfiltered(&movegen::GenMovesParams {
            board_snapshot: &board_snapshot,
            rack,
            max_gen: usize::MAX,
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
        writeln!(
            std::io::stdout(),
            "{label}: {}; field of {} holds {without} without the quota, {with} with it",
            match rank {
                Some(rank) => format!(
                    "best places-nothing move ranks {} of {num_moves} by static equity",
                    rank + 1
                ),
                None => "only tile plays are generated".to_string(),
            },
            move_picker::SIMMER_FIELD.max_gen,
        )?;
        Ok(with)
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
        )?;
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
    writeln!(
        std::io::stdout(),
        "after {} more turns the bag holds {}, under the {exchange_tile_limit} an exchange needs",
        extra_turns,
        move_generator.num_tiles_in_bag(),
    )?;
    let with = probe(
        &game_state,
        &mut move_generator,
        &mut scratch,
        "bag too short to exchange",
    )?;
    if with == 0 {
        writeln!(
            boxed_stdout_or_stderr(),
            "FAIL: field holds no pass once exchanging is illegal"
        )?;
        failures += 1;
    }

    if failures == 0 {
        writeln!(std::io::stdout(), "SIM_FIELD_OK")?;
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
    &SimChunkCheck {
        kwg: _,
        klv: _,
        iters,
        seed,
        trials,
        lean,
        quiet,
    }: &SimChunkCheck,
) -> error::Returns<()> {
    if iters < 2 {
        return Err("sim-chunk-check needs at least two iterations".into());
    }
    let arm_tables = anagrams::ArmTables::build(&kwg, &game_config, lean)?;
    let mut rng = GameRng::seed_from_u64(seed);
    let mut game_state = game_state::GameState::new(&game_config);
    game_state.reset_and_draw_tiles(&game_config, &mut rng);
    let mut move_generator = movegen::KurniaMoveGenerator::new(&game_config);
    move_generator.gen_moves_unfiltered(&movegen::GenMovesParams {
        board_snapshot: &movegen::BoardSnapshot {
            board_tiles: &game_state.board_tiles,
            game_config: &game_config,
            kwg: &kwg,
            anagrams: arm_tables.anagrams(),
            rack_lengths: arm_tables.rack_lengths(),
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
        &arm_tables,
        &klv,
        move_picker::SimmerParams {
            num_sim_iters: iters,
            num_sim_plies: move_picker::DEFAULT_NUM_SIM_PLIES,
            sim_threads: 1,
            win_pct_table: None,
            quiet,
        },
    )?;
    driver.reseed(seed);
    driver.begin_decision(&move_generator, &game_state, iters)?;
    let whole = driver.leaderboard(usize::MAX);
    writeln!(
        std::io::stdout(),
        "one call of {iters}: {} candidates",
        whole.len()
    )?;

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
        driver.begin_decision(&move_generator, &game_state, chunks[0])?;
        for &chunk in &chunks[1..] {
            driver.resume(&move_generator, chunk)?;
        }
        let split = driver.leaderboard(usize::MAX);
        if split == whole {
            writeln!(
                std::io::stdout(),
                "trial {trial}: {} chunks {chunks:?} match",
                chunks.len()
            )?;
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
        writeln!(std::io::stdout(), "SIM_CHUNK_OK")?;
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
    let phase = BoardPhase::of_board(&board_tiles, &game_config);
    let unseen_tally = game_config.tiles_off_board(&board_tiles)?;
    let unseen: usize = unseen_tally.iter().map(|&c| c as usize).sum();
    let lat = census::MultisetLattice::new(num_letters, rack_size)?;
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
            phase,
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
    match phase {
        BoardPhase::Opening | BoardPhase::Midgame => {
            census::fill_lattice_leaves(&lat, &mut leave, |tally| {
                klv.leave_value_from_tally(tally)
            });
        }
        BoardPhase::Endgame => {
            census::fill_lattice_leaves(&lat, &mut leave, |tally| {
                endgame_leave_value(&game_config, tally)
            });
        }
    }
    writeln!(
        std::io::stdout(),
        "{tiles_on_board} tiles on the board, {unseen} unseen, {num_candidates} candidate plays \
         in {sheet_elapsed:?}",
    )?;
    writeln!(
        std::io::stdout(),
        "  valued as {} ({})",
        phase.name(),
        match phase {
            BoardPhase::Opening =>
                "a vowel on a danger star costs the opening penalty; leaves from the table",
            BoardPhase::Midgame => "score plus the leave table",
            BoardPhase::Endgame =>
                "no leave table: going out is worth twice what the opponent holds",
        },
    )?;
    let rack_reader = alphabet::AlphabetReader::new_for_racks(alphabet);
    let mut rack = Vec::new();
    let mut tally = vec![0u8; num_letters];
    for rack_str in rack_strs {
        rack_reader.set_word(rack_str, &mut rack)?;
        game_config
            .take_rack(&mut unseen_tally.clone(), &rack)
            .map_err(|e| format!("{rack_str}: {e}"))?;
        tally.iter_mut().for_each(|c| *c = 0);
        for &t in &rack {
            tally[t as usize] += 1;
        }
        let mut played = vec![0u8; num_letters];
        let mut kept = tally.clone();
        let mut best = i32::MIN;
        let empty_keep = match phase {
            BoardPhase::Endgame => play_out_bonus(&game_config, &unseen_tally, &rack),
            BoardPhase::Opening | BoardPhase::Midgame => {
                let empty = vec![0u8; num_letters];
                let idx = lat.rank(&empty);
                if idx == !0 { 0 } else { leave[idx as usize] }
            }
        };
        best_split(
            &RackSplit {
                lat: &lat,
                sheet: &sheet,
                leave: &leave,
                empty_keep,
                rack: &rack,
            },
            0,
            &mut played,
            &mut kept,
            &mut best,
        );
        move_generator.gen_moves_unfiltered(&movegen::GenMovesParams {
            board_snapshot: &movegen::BoardSnapshot {
                board_tiles: &board_tiles,
                game_config: &game_config,
                kwg: &kwg,
                anagrams: None,
                rack_lengths: None,
                klv: &klv,
            },
            rack: &rack,
            max_gen: 1,
            num_exchanges_by_this_player: 0,
            pass_policy: movegen::PassPolicy::OnlyWhenForced,
            dynamic_leaves: None,
        });
        let (movegen_best, movegen_play) = move_generator.plays.first().map_or_else(
            || (f64::NAN, "nothing".to_string()),
            |m| {
                let mut shown = String::new();
                match &m.play {
                    movegen::Play::Exchange { tiles } if tiles.is_empty() => {
                        shown.push_str("pass");
                    }
                    movegen::Play::Exchange { tiles } => {
                        shown.push_str("exch ");
                        for &tile in tiles.iter() {
                            shown.push_str(alphabet.of_rack(tile).unwrap());
                        }
                    }
                    movegen::Play::Place {
                        down,
                        lane,
                        idx,
                        word,
                        ..
                    } => {
                        if *down {
                            write!(shown, "{}{} ", display::column(*lane), idx + 1).unwrap();
                        } else {
                            write!(shown, "{}{} ", lane + 1, display::column(*idx)).unwrap();
                        }
                        for &tile in word.iter() {
                            if tile == 0 {
                                shown.push('.');
                            } else {
                                shown.push_str(alphabet.of_board(tile).unwrap());
                            }
                        }
                    }
                }
                (m.equity.as_f64(), shown)
            },
        );
        let drawable = rack
            .iter()
            .all(|&t| tally[t as usize] <= unseen_tally[t as usize]);
        let sheet_best = equity::Equity::new(best).as_f64();
        writeln!(
            std::io::stdout(),
            "  {rack_str:<10} best equity {sheet_best:>8.3}  (the move generator says \
             {movegen_best:>8.3} for {movegen_play}{}){}",
            if (sheet_best - movegen_best).abs() < 0.0005 {
                ", agreed"
            } else {
                ", disagreed"
            },
            if drawable {
                ""
            } else {
                "   (not drawable from this board's unseen pool)"
            },
        )?;
    }
    Ok(())
}

struct RackSplit<'a> {
    lat: &'a census::MultisetLattice,
    sheet: &'a [i32],
    leave: &'a [i32],
    empty_keep: i32,
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
            let keep_value = if kept.iter().all(|&n| n == 0) {
                split.empty_keep
            } else {
                split.leave[kept_idx as usize]
            };
            let value = split.sheet[played_idx as usize] + keep_value;
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

// what a move shows of the tiles it came from: the tiles it placed (none for a
// pass), or only how many it exchanged
enum MoveTiles<'a> {
    Placed(&'a [u8]),
    Exchanged(usize),
}

// ln C(n, k) for every n up to a pool's size
struct LnBinom {
    ln_fact: Vec<f64>,
}

impl LnBinom {
    #[inline]
    fn new(max_n: usize) -> Self {
        let mut ln_fact = Vec::with_capacity(max_n + 1);
        ln_fact.push(0.0);
        for i in 1..=max_n {
            ln_fact.push(ln_fact[i - 1] + (i as f64).ln());
        }
        Self { ln_fact }
    }

    #[inline(always)]
    fn ln_binom(&self, n: usize, k: usize) -> f64 {
        self.ln_fact[n] - self.ln_fact[k] - self.ln_fact[n - k]
    }
}

// every tally of m tiles that avail holds, to f; cap[t] is how many tiles avail
// holds from letter t on
#[inline]
fn each_tally_of_size(
    avail: &[u8],
    cap: &[usize],
    t: usize,
    left: usize,
    cur: &mut [u8],
    f: &mut impl FnMut(&[u8]),
) {
    if t == avail.len() {
        if left == 0 {
            f(cur);
        }
        return;
    }
    if cap[t] < left {
        return;
    }
    for c in 0..=(avail[t] as usize).min(left) {
        cur[t] = c as u8;
        each_tally_of_size(avail, cap, t + 1, left - c, cur, f);
    }
    cur[t] = 0;
}

// the board's half of an inference: what each multiset of the pool, played, can
// score at best (the census sheet), and what each multiset is worth kept
struct InferSheet<'a> {
    lat: &'a census::MultisetLattice,
    sheet: &'a [i32],
    leave: &'a [i32],
    ln_binom: &'a LnBinom,
}

// every full rack the pool can deal that could have made the move, to f(rank of
// the rack, ln of its chance to be dealt, the equity it gives up by making the
// move). The gap is the rack's best equity on the board minus the move's: the
// move's value plus what it keeps, for a placed move; the best that exchanging
// so many tiles keeps, for an exchange. The best is over the moves that are not
// a pass, and the move itself: a hasty mover never passes while it can move, a
// simmer may, so a pass gives up the rack's best move over keeping it all, and
// nothing when the rack had no move. xsheet is scratch the size of the lattice.
#[inline]
fn weigh_racks(
    is: &InferSheet<'_>,
    pool: &[u8],
    tiles: MoveTiles<'_>,
    move_value: i32,
    xsheet: &mut Vec<i32>,
    f: &mut impl FnMut(u32, f64, i32),
) {
    let lat = is.lat;
    let num_letters = lat.num_letters();
    let rack_size = lat.rack_size();
    let pool_size: usize = pool.iter().map(|&c| c as usize).sum();
    let ln_deals = is.ln_binom.ln_binom(pool_size, rack_size);
    let mut avail = pool.to_vec();
    let mut rack = vec![0u8; num_letters];
    let mut cur = vec![0u8; num_letters];
    let none_placed = vec![0u8; num_letters];
    let (placed, left) = match tiles {
        MoveTiles::Placed(placed) => {
            for (a, &p) in avail.iter_mut().zip(placed) {
                *a -= p;
            }
            (
                placed,
                rack_size - placed.iter().map(|&c| c as usize).sum::<usize>(),
            )
        }
        MoveTiles::Exchanged(n) => {
            // exchanging n tiles scores 0 and keeps the rest: a sheet that is 0
            // for every n tiles and unplayable for any other count
            xsheet.clear();
            xsheet.resize(lat.len(), census::UNPLAYABLE);
            xsheet[lat.size_start(n)..lat.size_start(n + 1)]
                .iter_mut()
                .for_each(|v| *v = 0);
            (&none_placed[..], rack_size)
        }
    };
    let passed = matches!(tiles, MoveTiles::Placed(placed) if placed.iter().all(|&c| c == 0));
    let mut cap = vec![0usize; num_letters + 1];
    for t in (0..num_letters).rev() {
        cap[t] = cap[t + 1] + avail[t] as usize;
    }
    each_tally_of_size(&avail, &cap, 0, left, &mut cur, &mut |drawn| {
        for (r, (&d, &p)) in rack.iter_mut().zip(drawn.iter().zip(placed)) {
            *r = d + p;
        }
        let rank = lat.rank(&rack);
        let mut best = census::best_equity_of(lat, is.sheet, is.leave, &rack);
        if passed {
            best = best.max(is.leave[rank as usize]);
        }
        let value = match tiles {
            MoveTiles::Placed(_) => move_value + is.leave[lat.rank(drawn) as usize],
            MoveTiles::Exchanged(_) => census::best_equity_of(lat, xsheet, is.leave, &rack),
        };
        let ln_prior = rack
            .iter()
            .zip(pool)
            .map(|(&r, &u)| is.ln_binom.ln_binom(u as usize, r as usize))
            .sum::<f64>()
            - ln_deals;
        f(rank, ln_prior, best - value);
    });
}

// the census sheet for the pool on this board: every multiset of the pool, the
// best it can score played, floored at 0 when it could be exchanged instead.
// Playing nothing is no entry at all: weigh_racks weighs a pass itself.
#[inline]
fn infer_sheet_for<N: kwg::Node, L: kwg::Node>(
    move_generator: &mut movegen::KurniaMoveGenerator,
    board_tiles: &[u8],
    tables: SpellTables<'_, N, L>,
    pool: &[u8],
    can_exchange: bool,
    scratch: (&mut Vec<u8>, &mut Vec<(u8, i32)>),
    sheet: &mut Vec<i32>,
) {
    let lat = tables.lat;
    let game_config = tables.game_config;
    let rack_size = lat.rack_size();
    sheet.clear();
    sheet.resize(lat.len(), if can_exchange { 0 } else { i32::MIN / 4 });
    build_sheet_spell_once(
        move_generator,
        board_tiles,
        tables,
        SpellPool {
            unseen_tally: pool,
            num_blanks_eff: (pool[0] as usize).min(rack_size),
            rack_size,
            phase: BoardPhase::of_board(board_tiles, game_config),
        },
        scratch.0,
        scratch.1,
        sheet,
    );
    let empty = vec![0u8; lat.num_letters()];
    sheet[lat.rank(&empty) as usize] = i32::MIN / 4;
}

// the tiles a move places, as a tally (a blank as tile 0), or how many it
// exchanges; and its value as the census sheet values it: its score, with the
// opening's adjustment
#[inline]
fn move_tiles_and_value<N: kwg::Node, L: kwg::Node>(
    board_snapshot: &movegen::BoardSnapshot<'_, N, L>,
    play: &movegen::Play,
    placed: &mut [u8],
) -> (Option<usize>, i32) {
    placed.iter_mut().for_each(|c| *c = 0);
    match play {
        movegen::Play::Exchange { tiles } => {
            if tiles.is_empty() {
                (None, 0)
            } else {
                (Some(tiles.len()), 0)
            }
        }
        movegen::Play::Place {
            down, idx, word, ..
        } => {
            for &tile in word.iter() {
                if tile != 0 {
                    placed[if tile & 0x80 != 0 { 0 } else { tile as usize }] += 1;
                }
            }
            let game_config = board_snapshot.game_config;
            let phase = BoardPhase::of_board(board_snapshot.board_tiles, game_config);
            (
                None,
                play_scorer::PlayScorer::new().compute_score(board_snapshot, play)
                    + opening_adjustment(phase, game_config, *down, *idx, word),
            )
        }
    }
}

// a move written as "8H QI" (across from row 8, column H), "H8 Q.I" (down),
// "exch 3", or "pass"; in the word, . (or the letter already there) for a tile
// on the board, lowercase for a blank
#[inline]
fn parse_infer_play<N: kwg::Node, L: kwg::Node>(
    board_snapshot: &movegen::BoardSnapshot<'_, N, L>,
    s: &str,
) -> error::Returns<movegen::Play> {
    let game_config = board_snapshot.game_config;
    let board_tiles = board_snapshot.board_tiles;
    let mut tokens = s.split_whitespace();
    let first = tokens.next().ok_or("the move is empty")?;
    let second = tokens.next();
    if tokens.next().is_some() {
        wolges::return_error!(format!("{s:?}: a move is two words at most"));
    }
    match (first, second) {
        ("pass", None) => Ok(movegen::Play::Exchange {
            tiles: [][..].into(),
        }),
        ("exch", Some(n)) => {
            let n = usize::from_str(n).map_err(|e| format!("{s:?}: {e}"))?;
            if !(1..=game_config.rack_size() as usize).contains(&n) {
                wolges::return_error!(format!("{s:?}: cannot exchange {n} tiles"));
            }
            // only the count is seen; the tiles stand in for it
            Ok(movegen::Play::Exchange {
                tiles: vec![0u8; n][..].into(),
            })
        }
        (coord, Some(word_str)) => {
            let dim = game_config.board_layout().dim();
            let b = coord.as_bytes();
            let digits = b.iter().take_while(|c| c.is_ascii_digit()).count();
            let (down, row_str, col_str) = if digits > 0 {
                (false, &coord[..digits], &coord[digits..])
            } else {
                let letters = b.iter().take_while(|c| c.is_ascii_alphabetic()).count();
                (true, &coord[letters..], &coord[..letters])
            };
            // digits only: from_str would take a sign
            let row = (!row_str.is_empty() && row_str.bytes().all(|c| c.is_ascii_digit()))
                .then(|| i8::from_str(row_str).ok())
                .flatten()
                .filter(|r| (1..=dim.rows).contains(r))
                .map(|r| r - 1)
                .ok_or_else(|| format!("{coord:?} is not a square"))?;
            let col = display::str_to_column_usize_ignore_case(col_str.as_bytes())
                .and_then(|c| i8::try_from(c).ok())
                .filter(|c| (0..dim.cols).contains(c))
                .ok_or_else(|| format!("{coord:?} is not a square"))?;
            let (lane, idx) = if down { (col, row) } else { (row, col) };
            let reader = alphabet::AlphabetReader::new_for_plays(game_config.alphabet());
            let wb = word_str.as_bytes();
            let lane_len = (if down { dim.rows } else { dim.cols }) as isize;
            let mut word = Vec::new();
            let mut ix = 0;
            while ix < wb.len() {
                let at = idx as isize + word.len() as isize;
                if at >= lane_len {
                    wolges::return_error!(format!("{s:?} runs off the board"));
                }
                let (r, c) = if down {
                    (at as i8, lane)
                } else {
                    (lane, at as i8)
                };
                let on_board = board_tiles[dim.at_row_col(r, c)];
                if wb[ix] == b'.' {
                    if on_board == 0 {
                        wolges::return_error!(format!(
                            "{s:?}: no tile under its . at {}",
                            word.len() + 1
                        ));
                    }
                    word.push(0);
                    ix += 1;
                    continue;
                }
                let (tile, end) = reader
                    .next_tile(wb, ix)
                    .ok_or_else(|| format!("{s:?}: no tile at {ix}"))?;
                if on_board == 0 {
                    word.push(tile);
                } else if on_board == tile {
                    word.push(0);
                } else {
                    wolges::return_error!(format!(
                        "{s:?} crosses another tile at {}",
                        word.len() + 1
                    ));
                }
                ix = end;
            }
            if word.iter().all(|&t| t == 0) {
                wolges::return_error!(format!("{s:?} places no tile"));
            }
            // the word as written must be the whole word: one tile alone is its
            // word only the other way, and a tile just before or after it would
            // be part of it, so either would score the move wrong
            if word.len() < 2 {
                wolges::return_error!(format!(
                    "{s:?}: write a one-tile move along the word it makes"
                ));
            }
            let square = |at: isize| {
                let at = at as i8;
                board_tiles[if down {
                    dim.at_row_col(at, lane)
                } else {
                    dim.at_row_col(lane, at)
                }]
            };
            let before = idx as isize - 1;
            let after = idx as isize + word.len() as isize;
            if (before >= 0 && square(before) != 0) || (after < lane_len && square(after) != 0) {
                wolges::return_error!(format!("{s:?} leaves out a tile its word runs into"));
            }
            let mut play = movegen::Play::Place {
                down,
                lane,
                idx,
                word: word[..].into(),
                score: 0,
            };
            let scored = play_scorer::PlayScorer::new().compute_score(board_snapshot, &play);
            if let movegen::Play::Place { score, .. } = &mut play {
                *score = scored;
            }
            Ok(play)
        }
        _ => wolges::return_error!(format!(
            "{s:?} is not a move: a coordinate and a word, exch <n>, or pass"
        )),
    }
}

// fuzz, given in points, in the equity's own units. 0 is the exact limit, so
// only NaN and a fuzz below 0 are refused.
#[inline]
fn equity_fuzz(fuzz: f64) -> error::Returns<f64> {
    if fuzz.is_nan() || fuzz < 0.0 {
        wolges::return_error!(format!("fuzz {fuzz} is not 0 or more"));
    }
    Ok(fuzz * equity::SCALE as f64)
}

// for a rack that gives up gap, where g0 is the smallest gap of the racks that
// make the move: the log of the factor on its chance to be dealt, which is
// -(gap - g0) / fuzz, a gap below 0 counting as 0. At fuzz 0, its limit: 0 at
// the smallest gap, and minus infinity above it. The move may be no rack's
// best, so g0 may be above 0: fuzz 0 then keeps the racks closest to their
// best, not none; above fuzz 0, g0 cancels when the weights are normalized
#[inline(always)]
fn fuzz_ln_weight(gap: i32, g0: i32, fuzz: f64) -> f64 {
    let d = gap.max(0) - g0;
    if fuzz == 0.0 {
        if d == 0 { 0.0 } else { f64::NEG_INFINITY }
    } else {
        -(d as f64) / fuzz
    }
}

// each rack (rank, ln of its chance to be dealt, gap) as (weight, chance to be
// dealt, rank, gap); each weight is taken against the smallest gap, so none
// underflows to 0
#[inline]
fn weigh_by_fuzz(racks: &[(u32, f64, i32)], fuzz: f64) -> Vec<(f64, f64, u32, i32)> {
    let g0 = racks
        .iter()
        .map(|&(_, _, gap)| gap.max(0))
        .min()
        .unwrap_or(0);
    racks
        .iter()
        .map(|&(rank, ln_prior, gap)| {
            let prior = ln_prior.exp();
            (
                prior * fuzz_ln_weight(gap, g0, fuzz).exp(),
                prior,
                rank,
                gap,
            )
        })
        .collect()
}

// what one move says of the rack that made it: every rack the pool could have
// dealt the mover, weighed by its chance to be dealt and by how far the move
// falls short of the rack's best (exp(-gap / fuzz))
#[inline]
fn infer_one_move<N: kwg::Node, L: kwg::Node>(
    game_config: game_config::GameConfig,
    kwg: kwg::Kwg<N>,
    klv: klv::Klv<L>,
    params: &Infer,
) -> error::Returns<()> {
    let alphabet = game_config.alphabet();
    let board_layout = game_config.board_layout();
    let num_letters = alphabet.len() as usize;
    let rack_size = game_config.rack_size() as usize;
    let mut fen_parser = display::BoardFenParser::new(alphabet, board_layout);
    let board_tiles = fen_parser.parse(&params.board)?.to_vec();
    let mut pool = game_config.tiles_off_board(&board_tiles)?;
    let bag = if let Some(rack_str) = &params.rack {
        let mut rack = Vec::new();
        alphabet::AlphabetReader::new_for_racks(alphabet).set_word(rack_str, &mut rack)?;
        game_config
            .take_rack(&mut pool, &rack)
            .map_err(|e| format!("{rack_str}: {e}"))?;
        pool.iter().map(|&c| c as i64).sum::<i64>() - rack_size as i64
    } else {
        bag_size(&board_tiles, &game_config)
    };
    let pool_size: usize = pool.iter().map(|&c| c as usize).sum();
    if bag <= 0 {
        wolges::return_error!(if params.rack.is_some() {
            "the bag is empty: the watcher knows the mover's rack, the pool itself".to_string()
        } else {
            "the bag is empty: the watcher's --rack decides the mover's".to_string()
        });
    }
    let fuzz = equity_fuzz(params.fuzz)?;
    let board_snapshot = movegen::BoardSnapshot {
        board_tiles: &board_tiles,
        game_config: &game_config,
        kwg: &kwg,
        anagrams: None,
        rack_lengths: None,
        klv: &klv,
    };
    let play = parse_infer_play(&board_snapshot, &params.play)?;
    let mut placed = vec![0u8; num_letters];
    let (exchanged, move_value) = move_tiles_and_value(&board_snapshot, &play, &mut placed);
    let num_placed: usize = placed.iter().map(|&c| c as usize).sum();
    if num_placed > rack_size {
        wolges::return_error!(format!(
            "{:?} places {num_placed} tiles, more than a rack holds",
            params.play
        ));
    }
    if placed.iter().zip(&pool).any(|(&p, &u)| p > u) {
        wolges::return_error!(format!(
            "{:?} places tiles the pool does not hold",
            params.play
        ));
    }
    let lat = census::MultisetLattice::new(num_letters, rack_size)?;
    let mut leave = vec![0i32; lat.len()];
    census::fill_lattice_leaves(&lat, &mut leave, |tally| klv.leave_value_from_tally(tally));
    let ln_binom = LnBinom::new(pool_size);
    let can_exchange = bag >= game_config.exchange_tile_limit() as i64;
    if exchanged.is_some() && !can_exchange {
        wolges::return_error!(format!("{bag} in the bag is too few to exchange"));
    }
    let mut sheet = Vec::new();
    let mut move_generator = movegen::KurniaMoveGenerator::new(&game_config);
    let (mut movegen_rack, mut blank_deltas) = (Vec::new(), Vec::new());
    let t0 = std::time::Instant::now();
    infer_sheet_for(
        &mut move_generator,
        &board_tiles,
        SpellTables {
            game_config: &game_config,
            kwg: &kwg,
            klv: &klv,
            lat: &lat,
        },
        &pool,
        can_exchange,
        (&mut movegen_rack, &mut blank_deltas),
        &mut sheet,
    );
    let sheet_elapsed = t0.elapsed();
    let tiles = match exchanged {
        Some(n) => MoveTiles::Exchanged(n),
        None => MoveTiles::Placed(&placed),
    };
    let is = InferSheet {
        lat: &lat,
        sheet: &sheet,
        leave: &leave,
        ln_binom: &ln_binom,
    };
    let mut xsheet = Vec::new();
    let mut racks = Vec::new();
    weigh_racks(
        &is,
        &pool,
        tiles,
        move_value,
        &mut xsheet,
        &mut |rank, ln_prior, gap| racks.push((rank, ln_prior, gap)),
    );
    if racks.is_empty() {
        wolges::return_error!("no rack the pool can deal makes this move".to_string());
    }
    let mut weighed = weigh_by_fuzz(&racks, fuzz);
    let num_short = racks.iter().filter(|&&(_, _, gap)| gap < 0).count();
    let mut z = 0.0f64;
    let mut z_prior = 0.0f64;
    for &(w, prior, ..) in &weighed {
        z += w;
        z_prior += prior;
    }
    weighed.sort_unstable_by(|a, b| b.0.total_cmp(&a.0).then(a.2.cmp(&b.2)));
    let effective = z * z / weighed.iter().map(|&(w, ..)| w * w).sum::<f64>();
    writeln!(
        std::io::stdout(),
        "{} tiles on the board, {pool_size} unseen by the watcher, {bag} in the bag; the sheet took {sheet_elapsed:?}",
        board_tiles.iter().filter(|&&t| t != 0).count(),
    )?;
    let shown_move = match &play {
        movegen::Play::Exchange { tiles } if tiles.is_empty() => "pass".to_string(),
        movegen::Play::Exchange { tiles } => format!("exch {}", tiles.len()),
        movegen::Play::Place { .. } => format!(
            "{}, value {:.3}",
            play.fmt(&board_snapshot),
            equity::Equity::new(move_value).as_f64()
        ),
    };
    writeln!(
        std::io::stdout(),
        "{shown_move}: {} racks could have made it, {effective:.1} of them in effect at fuzz {}",
        racks.len(),
        params.fuzz,
    )?;
    if num_short > 0 {
        writeln!(
            std::io::stdout(),
            "  {num_short} racks have a best below the move itself: the sheet misses something"
        )?;
    }
    let mut tally = vec![0u8; num_letters];
    let mut expect = vec![0.0f64; num_letters];
    let mut expect_prior = vec![0.0f64; num_letters];
    for &(w, prior, rank, _) in &weighed {
        lat.unrank_into(rank as usize, &mut tally);
        for (((e, e_prior), &r), &p) in expect
            .iter_mut()
            .zip(expect_prior.iter_mut())
            .zip(&tally)
            .zip(&placed)
        {
            let k = (r - p) as f64;
            *e += w / z * k;
            *e_prior += prior / z_prior * k;
        }
    }
    writeln!(
        std::io::stdout(),
        "  {:<10} {:>8} {:>8} {:>9}",
        if exchanged.is_some() { "rack" } else { "kept" },
        "chance",
        "dealt",
        "gap",
    )?;
    for &(w, prior, rank, gap) in weighed.iter().take(params.racks) {
        lat.unrank_into(rank as usize, &mut tally);
        let mut shown = String::new();
        for (t, (&r, &p)) in tally.iter().zip(&placed).enumerate() {
            for _ in p..r {
                shown.push_str(alphabet.of_rack(t as u8).unwrap());
            }
        }
        if shown.is_empty() {
            shown.push('-');
        }
        writeln!(
            std::io::stdout(),
            "  {shown:<10} {:>7.3}% {:>7.3}% {:>9.3}",
            100.0 * w / z,
            100.0 * prior / z_prior,
            equity::Equity::new(gap).as_f64(),
        )?;
    }
    writeln!(
        std::io::stdout(),
        "  each letter the mover {}: expected count, then as the pool alone deals",
        if exchanged.is_some() { "held" } else { "kept" },
    )?;
    let mut line = String::new();
    for (t, ((&u, e), e_prior)) in pool.iter().zip(&expect).zip(&expect_prior).enumerate() {
        if u > 0 {
            write!(
                line,
                " {}={e:.2}/{e_prior:.2}",
                alphabet.of_rack(t as u8).unwrap(),
            )?;
        }
    }
    writeln!(std::io::stdout(), " {line}")?;
    Ok(())
}

// one move the fit saw (game and turn): the racks the pool could have dealt the
// mover, summed by the equity each gives up (gap ascending, each with its
// racks' chance to be dealt), and the true rack's chance and gap
struct FitMove {
    at: (u64, u32),
    buckets: Vec<(i32, f64)>,
    true_prior: f64,
    true_gap: i32,
    num_short: usize,
    kind: MoveKind,
}

#[derive(PartialEq)]
enum MoveKind {
    Placed,
    Exchanged,
    Passed,
}

// the log of the chance that each true rack made its move, summed over the
// moves: exp(-gap / fuzz) times the chance to be dealt, over the same for every
// rack the pool could have dealt. At fuzz 0 it is minus infinity once a true
// rack gave up more than the smallest gap.
#[inline]
fn fit_log_likelihood(moves: &[FitMove], fuzz: f64) -> f64 {
    let fuzz = fuzz * equity::SCALE as f64;
    moves
        .iter()
        .map(|m| {
            let g0 = m.buckets[0].0.max(0);
            let z: f64 = m
                .buckets
                .iter()
                .map(|&(g, s)| s * fuzz_ln_weight(g, g0, fuzz).exp())
                .sum();
            m.true_prior.ln() + fuzz_ln_weight(m.true_gap, g0, fuzz) - z.ln()
        })
        .sum()
}

struct InferFitParams {
    num_games: u64,
    seed: Option<u64>,
    threads: usize,
    num_sim_iters: u64,
    mover: SeatPicker,
    lean: bool,
    quiet: bool,
}

// play games, the mover against a hasty watcher, and at every move the mover
// makes while the bag holds tiles weigh every rack the watcher cannot rule out,
// as infer does; then the fuzz under which the true racks were likeliest
#[inline]
fn infer_fit<N: kwg::Node + Sync + Send, L: kwg::Node + Sync + Send>(
    game_config: game_config::GameConfig,
    kwg: kwg::Kwg<N>,
    klv: klv::Klv<L>,
    InferFitParams {
        num_games,
        seed,
        threads,
        num_sim_iters,
        mover,
        lean,
        quiet,
    }: InferFitParams,
) -> error::Returns<()> {
    let seed = seed_or_random(seed)?;
    writeln!(std::io::stdout(), "seed: {seed}")?;
    let num_letters = game_config.alphabet().len() as usize;
    let rack_size = game_config.rack_size() as usize;
    let lat = census::MultisetLattice::new(num_letters, rack_size)?;
    let mut leave = vec![0i32; lat.len()];
    census::fill_lattice_leaves(&lat, &mut leave, |tally| klv.leave_value_from_tally(tally));
    let ln_binom = LnBinom::new(game_config.alphabet().num_tiles() as usize);
    let arm_tables = anagrams::ArmTables::build(&kwg, &game_config, lean)?;
    let dyn_ctx = matches!(mover, SeatPicker::Dynamic).then(|| {
        (
            census::AddTable::new_with_threads(&lat, threads),
            full_rack_values(&lat, &klv),
        )
    });
    let dyn_ref = dyn_ctx.as_ref().map(|(add, full_v)| klv::DynamicLeavesRef {
        lat: &lat,
        add,
        full_v,
        min_keep: 0,
    });
    writeln!(
        std::io::stdout(),
        "infer-fit: the mover {}{}, the watcher hasty; {num_games} games on the {} arm",
        picker_name(mover),
        if matches!(mover, SeatPicker::Simmer) {
            format!(" ({num_sim_iters} rollouts a move)")
        } else {
            String::new()
        },
        if lean { "lean" } else { "unadorned" },
    )?;
    let t0 = std::time::Instant::now();
    let claimed_games = std::sync::atomic::AtomicU64::new(0);
    let finished_games = std::sync::atomic::AtomicU64::new(0);
    let tick = std::sync::atomic::AtomicU64::new(0);
    let weighed = std::sync::atomic::AtomicU64::new(0);
    let mut moves = std::thread::scope(|s| -> error::Returns<Vec<FitMove>> {
        let mut thread_handles = Vec::new();
        for _ in 0..threads {
            let game_config = &game_config;
            let kwg = &kwg;
            let klv = &klv;
            let arm_tables = &arm_tables;
            let claimed_games = &claimed_games;
            let (finished_games, tick, weighed) = (&finished_games, &tick, &weighed);
            // each move's own sheet goes in at the move
            let is = InferSheet {
                lat: &lat,
                sheet: &[],
                leave: &leave,
                ln_binom: &ln_binom,
            };
            thread_handles.push(s.spawn(move || -> error::Returns<Vec<FitMove>> {
                let mut move_generator = movegen::KurniaMoveGenerator::new(game_config);
                let mut filtered_movegen = move_filter::GenMoves::Unfiltered;
                let mut driver = seat_picker(
                    mover,
                    game_config,
                    kwg,
                    arm_tables,
                    klv,
                    SeatParams {
                        dynamic_leaves: dyn_ref,
                        simmer_params: move_picker::SimmerParams {
                            num_sim_iters,
                            num_sim_plies: move_picker::DEFAULT_NUM_SIM_PLIES,
                            sim_threads: 1,
                            win_pct_table: None,
                            quiet: true,
                        },
                        deft_window: move_picker::DEFAULT_DEFT_WINDOW,
                    },
                );
                let mut game_state = game_state::GameState::new(game_config);
                let mut final_scores = vec![0i32; game_config.num_players() as usize];
                let (mut sheet, mut xsheet) = (Vec::new(), Vec::new());
                let (mut movegen_rack, mut blank_deltas) = (Vec::new(), Vec::new());
                let mut placed = vec![0u8; num_letters];
                let mut true_tally = vec![0u8; num_letters];
                let mut racks = Vec::new();
                let mut out = Vec::new();
                loop {
                    let game_idx = claimed_games.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    if game_idx >= num_games {
                        break;
                    }
                    let mut rng = GameRng::from_stream(seed, game_idx);
                    game_state.reset_and_draw_tiles(game_config, &mut rng);
                    let mut num_turns = 0u32;
                    loop {
                        let is_mover = game_state.turn == 0;
                        let board_snapshot = movegen::BoardSnapshot {
                            board_tiles: &game_state.board_tiles,
                            game_config,
                            kwg,
                            anagrams: arm_tables.anagrams(),
                            rack_lengths: arm_tables.rack_lengths(),
                            klv,
                        };
                        if !is_mover || matches!(driver, move_picker::MovePicker::Hasty) {
                            let gen_params = movegen::GenMovesParams {
                                board_snapshot: &board_snapshot,
                                rack: &game_state.current_player().rack,
                                max_gen: 1,
                                num_exchanges_by_this_player: game_state
                                    .current_player()
                                    .num_exchanges,
                                pass_policy: movegen::PassPolicy::OnlyWhenForced,
                                dynamic_leaves: None,
                            };
                            if lean {
                                move_generator.gen_moves_unfiltered_lean(&gen_params);
                            } else {
                                move_generator.gen_moves_unfiltered(&gen_params);
                            }
                        } else {
                            if let move_picker::MovePicker::Simmer(simmer_driver) = &mut driver {
                                // https://doi.org/10.1145/2714064.2660195
                                // https://prng.di.unimi.it/splitmix64.c
                                simmer_driver.reseed(census_mix64(seed.wrapping_add(
                                    census_mix64(
                                        game_idx
                                            .wrapping_mul(0x9e3779b97f4a7c15)
                                            .wrapping_add(num_turns as u64),
                                    ),
                                )));
                            }
                            driver.pick_a_move(
                                &mut filtered_movegen,
                                &mut move_generator,
                                &board_snapshot,
                                &game_state,
                                &game_state.current_player().rack,
                            )?;
                        }
                        let play = move_generator.plays[0].play.clone();
                        if is_mover && !game_state.bag.is_empty() {
                            // what the watcher cannot see: the bag and the mover's rack
                            let mut pool = game_config.tiles_off_board(&game_state.board_tiles)?;
                            game_config.take_rack(&mut pool, &game_state.players[1].rack)?;
                            let (exchanged, move_value) =
                                move_tiles_and_value(&board_snapshot, &play, &mut placed);
                            let can_exchange = game_state.bag.len()
                                >= game_config.exchange_tile_limit() as usize;
                            infer_sheet_for(
                                &mut move_generator,
                                &game_state.board_tiles,
                                SpellTables {
                                    game_config,
                                    kwg,
                                    klv,
                                    lat: is.lat,
                                },
                                &pool,
                                can_exchange,
                                (&mut movegen_rack, &mut blank_deltas),
                                &mut sheet,
                            );
                            true_tally.iter_mut().for_each(|c| *c = 0);
                            for &t in game_state.current_player().rack.iter() {
                                true_tally[t as usize] += 1;
                            }
                            let true_rank = is.lat.rank(&true_tally);
                            let kind = match exchanged {
                                Some(_) => MoveKind::Exchanged,
                                None if placed.iter().all(|&c| c == 0) => MoveKind::Passed,
                                None => MoveKind::Placed,
                            };
                            let mut seen = None;
                            let mut num_short = 0usize;
                            racks.clear();
                            weigh_racks(
                                &InferSheet {
                                    lat: is.lat,
                                    sheet: &sheet,
                                    leave: is.leave,
                                    ln_binom: is.ln_binom,
                                },
                                &pool,
                                match exchanged {
                                    Some(n) => MoveTiles::Exchanged(n),
                                    None => MoveTiles::Placed(&placed),
                                },
                                move_value,
                                &mut xsheet,
                                &mut |rank, ln_prior, gap| {
                                    let prior = ln_prior.exp();
                                    if rank == true_rank {
                                        seen = Some((prior, gap));
                                    }
                                    num_short += (gap < 0) as usize;
                                    racks.push((gap, prior));
                                },
                            );
                            let (true_prior, true_gap) = seen.ok_or_else(|| {
                                format!(
                                    "game {game_idx} turn {num_turns}: the mover's own rack is not among the racks that make its move"
                                )
                            })?;
                            racks.sort_unstable_by_key(|&(gap, _)| gap);
                            let mut buckets: Vec<(i32, f64)> = Vec::new();
                            for &(gap, prior) in &racks {
                                match buckets.last_mut() {
                                    Some(last) if last.0 == gap => last.1 += prior,
                                    _ => buckets.push((gap, prior)),
                                }
                            }
                            out.push(FitMove {
                                at: (game_idx, num_turns),
                                buckets,
                                true_prior,
                                true_gap,
                                num_short,
                                kind,
                            });
                            weighed.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                        }
                        game_state.play(game_config, &mut rng, &play)?;
                        num_turns += 1;
                        let secs = t0.elapsed().as_secs();
                        // a line a second, from the first thread to end a turn in it
                        if !quiet && tick.fetch_max(secs, std::sync::atomic::Ordering::Relaxed) < secs {
                            writeln!(
                                boxed_stdout_or_stderr(),
                                "After {secs} seconds, have played {} of {num_games} games, weighed {} moves",
                                finished_games.load(std::sync::atomic::Ordering::Relaxed),
                                weighed.load(std::sync::atomic::Ordering::Relaxed),
                            )
                            .ok();
                        }
                        match game_state.check_game_ended(game_config, &mut final_scores)? {
                            game_state::CheckGameEnded::PlayedOut
                            | game_state::CheckGameEnded::ZeroScores => break,
                            game_state::CheckGameEnded::NotEnded => {}
                        }
                        game_state.next_turn();
                    }
                    finished_games.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                }
                Ok(out)
            }));
        }
        let mut moves = Vec::new();
        for handle in thread_handles {
            moves.extend(handle.join().map_err(|_| "a fit thread panicked")??);
        }
        Ok(moves)
    })?;
    // in game and turn order, whichever thread played them, so the sums are the
    // same at any thread count
    moves.sort_unstable_by_key(|m| m.at);
    let n = moves.len();
    if n == 0 {
        wolges::return_error!("no move with tiles in the bag: play more games".to_string());
    }
    let count_of = |kind: MoveKind| moves.iter().filter(|m| m.kind == kind).count();
    writeln!(
        std::io::stdout(),
        "{n} moves with tiles in the bag ({} placing, {} exchanges, {} passes) in {:?}",
        count_of(MoveKind::Placed),
        count_of(MoveKind::Exchanged),
        count_of(MoveKind::Passed),
        t0.elapsed(),
    )?;
    let zero = moves.iter().filter(|m| m.true_gap <= 0).count();
    let mut gaps: Vec<i32> = moves.iter().map(|m| m.true_gap.max(0)).collect();
    gaps.sort_unstable();
    let points = |g: i32| equity::Equity::new(g).as_f64();
    writeln!(
        std::io::stdout(),
        "  the true rack gave up nothing in {zero} ({:.2}%); its gap: mean {:.3}, median {:.3}, 90th percentile {:.3}, largest {:.3}",
        100.0 * zero as f64 / n as f64,
        gaps.iter().map(|&g| points(g)).sum::<f64>() / n as f64,
        points(gaps[n.div_ceil(2) - 1]),
        points(gaps[(n * 9).div_ceil(10) - 1]),
        points(gaps[n - 1]),
    )?;
    let num_short: usize = moves.iter().map(|m| m.num_short).sum();
    let short_moves = moves.iter().filter(|m| m.num_short > 0).count();
    writeln!(
        std::io::stdout(),
        "  racks whose best is below the move itself: {num_short} in {short_moves} moves"
    )?;
    let prior_only: f64 = moves
        .iter()
        .map(|m| (m.true_prior / m.buckets.iter().map(|&(_, s)| s).sum::<f64>()).ln())
        .sum();
    // fuzz is in points; the likelihood is read on a grid of 0.01 to 1000, then
    // narrowed by golden sections around the grid's best
    let grid: Vec<f64> = (0..=250)
        .map(|i| 10f64.powf(-2.0 + i as f64 / 50.0))
        .collect();
    let lls: Vec<f64> = grid
        .iter()
        .map(|&fuzz| fit_log_likelihood(&moves, fuzz))
        .collect();
    let best_i = (1..grid.len()).fold(0, |best, i| if lls[i] > lls[best] { i } else { best });
    let (mut lo, mut hi) = (
        grid[best_i.saturating_sub(1)].ln(),
        grid[(best_i + 1).min(grid.len() - 1)].ln(),
    );
    let golden = (5f64.sqrt() - 1.0) / 2.0;
    for _ in 0..60 {
        let a = hi - golden * (hi - lo);
        let b = lo + golden * (hi - lo);
        if fit_log_likelihood(&moves, a.exp()) < fit_log_likelihood(&moves, b.exp()) {
            lo = a;
        } else {
            hi = b;
        }
    }
    let fuzz = ((lo + hi) / 2.0).exp();
    let ll = fit_log_likelihood(&moves, fuzz);
    // the likelihood's curvature in ln fuzz at its top gives one standard
    // error; at the grid's edge there is no top to read it from
    let h = 0.01f64;
    let curvature = (fit_log_likelihood(&moves, fuzz * h.exp()) - 2.0 * ll
        + fit_log_likelihood(&moves, fuzz / h.exp()))
        / (h * h);
    let within = if curvature < 0.0 {
        let se = (-1.0 / curvature).sqrt();
        format!(
            " ({:.3} to {:.3} within one standard error)",
            fuzz / se.exp(),
            fuzz * se.exp()
        )
    } else {
        String::new()
    };
    writeln!(
        std::io::stdout(),
        "fuzz {fuzz:.3} points is the likeliest{within}: {:.4} nats a move, against {:.4} for the pool alone; the move tells {:.3} bits of the rack",
        ll / n as f64,
        prior_only / n as f64,
        (ll - prior_only) / n as f64 / std::f64::consts::LN_2,
    )?;
    if best_i == 0 || best_i == grid.len() - 1 {
        writeln!(
            std::io::stdout(),
            "  at the grid's edge: the likeliest fuzz may lie beyond {:.2} points",
            grid[best_i]
        )?;
    }
    writeln!(std::io::stdout(), "  fuzz       nats a move")?;
    for (fuzz, ll) in grid.iter().zip(&lls).step_by(25) {
        writeln!(std::io::stdout(), "  {fuzz:<10.3} {:.4}", ll / n as f64)?;
    }
    Ok(())
}

// one board's exact openness, over every full rack its unseen pool can deal,
// each by its chance to be dealt: the best score it can make, the best equity,
// and the chance it holds a play of all its tiles
#[derive(Clone, Copy)]
struct BoardOpenness {
    pool: u32,
    best_score: f64,
    best_equity: f64,
    bingo: f64,
}

// the board's exact openness for a rack dealt from pool (more tiles than a
// rack), leaves being (the leave table, all zeros); scratch is (movegen rack,
// blank deltas, sheet)
#[inline]
fn exact_openness<N: kwg::Node, L: kwg::Node>(
    move_generator: &mut movegen::KurniaMoveGenerator,
    board_tiles: &[u8],
    tables: SpellTables<'_, N, L>,
    leaves: (&[i32], &[i32]),
    pool: &[u8],
    ln_binom: &LnBinom,
    scratch: (&mut Vec<u8>, &mut Vec<(u8, i32)>, &mut Vec<i32>),
) -> BoardOpenness {
    let game_config = tables.game_config;
    let lat = tables.lat;
    let num_letters = lat.num_letters();
    let rack_size = lat.rack_size();
    let pool_size: usize = pool.iter().map(|&c| c as usize).sum();
    let can_exchange =
        bag_size(board_tiles, game_config) >= game_config.exchange_tile_limit() as i64;
    let sheet = scratch.2;
    infer_sheet_for(
        move_generator,
        board_tiles,
        tables,
        pool,
        can_exchange,
        (scratch.0, scratch.1),
        sheet,
    );
    // a play of every tile is a full rack's own entry, above the sheet's floor
    // only if some word takes them all
    let floor = if can_exchange { 0 } else { i32::MIN / 4 };
    let ln_deals = ln_binom.ln_binom(pool_size, rack_size);
    let mut cap = vec![0usize; num_letters + 1];
    for t in (0..num_letters).rev() {
        cap[t] = cap[t + 1] + pool[t] as usize;
    }
    let mut cur = vec![0u8; num_letters];
    let mut openness = BoardOpenness {
        pool: pool_size as u32,
        best_score: 0.0,
        best_equity: 0.0,
        bingo: 0.0,
    };
    each_tally_of_size(pool, &cap, 0, rack_size, &mut cur, &mut |rack| {
        let chance = (rack
            .iter()
            .zip(pool)
            .map(|(&r, &u)| ln_binom.ln_binom(u as usize, r as usize))
            .sum::<f64>()
            - ln_deals)
            .exp();
        let rank = lat.rank(rack) as usize;
        // with no play and no exchange a rack must pass: it scores nothing and
        // keeps every tile (the sheet has no entry for playing nothing)
        let or_pass = |best: i32, pass: i32| if best < i32::MIN / 8 { pass } else { best };
        openness.best_score +=
            chance * or_pass(census::best_equity_of(lat, sheet, leaves.1, rack), 0) as f64;
        openness.best_equity += chance
            * or_pass(
                census::best_equity_of(lat, sheet, leaves.0, rack),
                leaves.0[rank],
            ) as f64;
        openness.bingo += chance * (sheet[rank] > floor) as u8 as f64;
    });
    openness
}

#[inline]
fn board_openness<N: kwg::Node + Sync + Send, L: kwg::Node + Sync + Send>(
    game_config: game_config::GameConfig,
    kwg: kwg::Kwg<N>,
    klv: klv::Klv<L>,
    boards_path: &str,
    threads: usize,
    quiet: bool,
) -> error::Returns<()> {
    let alphabet = game_config.alphabet();
    let num_letters = alphabet.len() as usize;
    let rack_size = game_config.rack_size() as usize;
    let mut fen_parser = display::BoardFenParser::new(alphabet, game_config.board_layout());
    let mut boards = Vec::new();
    for line in std::fs::read_to_string(boards_path)?.lines() {
        if !line.is_empty() {
            boards.push(fen_parser.parse(line)?.to_vec());
        }
    }
    let lat = census::MultisetLattice::new(num_letters, rack_size)?;
    let mut leave = vec![0i32; lat.len()];
    census::fill_lattice_leaves(&lat, &mut leave, |tally| klv.leave_value_from_tally(tally));
    let no_leave = vec![0i32; lat.len()];
    let ln_binom = LnBinom::new(alphabet.num_tiles() as usize);
    let t0 = std::time::Instant::now();
    let claimed = std::sync::atomic::AtomicUsize::new(0);
    let tick = std::sync::atomic::AtomicU64::new(0);
    let mut found =
        std::thread::scope(|s| -> error::Returns<Vec<(usize, BoardOpenness, Glance)>> {
            let mut thread_handles = Vec::new();
            for _ in 0..threads {
                let (game_config, kwg, klv, boards) = (&game_config, &kwg, &klv, &boards);
                let (lat, leave, no_leave, ln_binom, claimed) =
                    (&lat, &leave, &no_leave, &ln_binom, &claimed);
                let tick = &tick;
                thread_handles.push(s.spawn(
                    move || -> error::Returns<Vec<(usize, BoardOpenness, Glance)>> {
                        let mut move_generator = movegen::KurniaMoveGenerator::new(game_config);
                        let (mut movegen_rack, mut blank_deltas) = (Vec::new(), Vec::new());
                        let mut sheet = Vec::new();
                        let mut out = Vec::new();
                        loop {
                            let i = claimed.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                            if i >= boards.len() {
                                break;
                            }
                            let secs = t0.elapsed().as_secs();
                            // a line a second, from the first thread to begin a board in it
                            if !quiet
                                && tick.fetch_max(secs, std::sync::atomic::Ordering::Relaxed) < secs
                            {
                                writeln!(
                                    boxed_stdout_or_stderr(),
                                    "After {secs} seconds, have begun {i} of {} boards",
                                    boards.len()
                                )
                                .ok();
                            }
                            let board_tiles = &boards[i];
                            let pool = game_config.tiles_off_board(board_tiles)?;
                            if pool.iter().map(|&c| c as usize).sum::<usize>() <= rack_size {
                                continue;
                            }
                            let openness = exact_openness(
                                &mut move_generator,
                                board_tiles,
                                SpellTables {
                                    game_config,
                                    kwg,
                                    klv,
                                    lat,
                                },
                                (leave, no_leave),
                                &pool,
                                ln_binom,
                                (&mut movegen_rack, &mut blank_deltas, &mut sheet),
                            );
                            out.push((i, openness, glance_at(game_config, board_tiles)));
                        }
                        Ok(out)
                    },
                ));
            }
            let mut found = Vec::new();
            for handle in thread_handles {
                found.extend(handle.join().map_err(|_| "an openness thread panicked")??);
            }
            Ok(found)
        })?;
    found.sort_unstable_by_key(|&(i, ..)| i);
    let n = found.len();
    if n == 0 {
        wolges::return_error!("no board with more unseen tiles than a rack".to_string());
    }
    let points = equity::SCALE as f64;
    writeln!(
        std::io::stdout(),
        "{n} boards of {} read in {:?}; per board: line, unseen, best score, best equity, bingo%, anchors, premium anchors, word-premium anchors, bingo lanes, stretches, bingo stretches, word-premium stretches",
        boards.len(),
        t0.elapsed(),
    )?;
    for &(i, o, g) in &found {
        writeln!(
            std::io::stdout(),
            "{}\t{}\t{:.3}\t{:.3}\t{:.2}\t{}\t{}\t{}\t{}\t{}\t{}\t{}",
            i + 1,
            o.pool,
            o.best_score / points,
            o.best_equity / points,
            100.0 * o.bingo,
            g.anchors,
            g.premium_anchors,
            g.word_premium_anchors,
            g.bingo_lanes,
            g.stretches,
            g.bingo_stretches,
            g.word_premium_stretches,
        )?;
    }
    // each count's correlation with each exact label, over the boards
    let pearson = |xs: &[f64], ys: &[f64]| {
        let nf = xs.len() as f64;
        let (mx, my) = (xs.iter().sum::<f64>() / nf, ys.iter().sum::<f64>() / nf);
        let sxy: f64 = xs.iter().zip(ys).map(|(x, y)| (x - mx) * (y - my)).sum();
        let sxx: f64 = xs.iter().map(|x| (x - mx) * (x - mx)).sum();
        let syy: f64 = ys.iter().map(|y| (y - my) * (y - my)).sum();
        sxy / (sxx * syy).sqrt()
    };
    let labels: [(&str, Vec<f64>); 3] = [
        (
            "best score",
            found.iter().map(|(_, o, _)| o.best_score).collect(),
        ),
        (
            "best equity",
            found.iter().map(|(_, o, _)| o.best_equity).collect(),
        ),
        ("bingo", found.iter().map(|(_, o, _)| o.bingo).collect()),
    ];
    let counts: [(&str, Vec<f64>); 8] = [
        (
            "unseen",
            found.iter().map(|(_, o, _)| o.pool as f64).collect(),
        ),
        (
            "anchors",
            found.iter().map(|(_, _, g)| g.anchors as f64).collect(),
        ),
        (
            "premium anchors",
            found
                .iter()
                .map(|(_, _, g)| g.premium_anchors as f64)
                .collect(),
        ),
        (
            "word-premium anchors",
            found
                .iter()
                .map(|(_, _, g)| g.word_premium_anchors as f64)
                .collect(),
        ),
        (
            "bingo lanes",
            found.iter().map(|(_, _, g)| g.bingo_lanes as f64).collect(),
        ),
        (
            "stretches",
            found.iter().map(|(_, _, g)| g.stretches as f64).collect(),
        ),
        (
            "bingo stretches",
            found
                .iter()
                .map(|(_, _, g)| g.bingo_stretches as f64)
                .collect(),
        ),
        (
            "word-premium stretches",
            found
                .iter()
                .map(|(_, _, g)| g.word_premium_stretches as f64)
                .collect(),
        ),
    ];
    writeln!(
        std::io::stdout(),
        "correlation      {:>11} {:>11} {:>11}",
        labels[0].0,
        labels[1].0,
        labels[2].0,
    )?;
    for (name, xs) in &counts {
        writeln!(
            std::io::stdout(),
            "{name:<22} {:>+.3}      {:>+.3}      {:>+.3}",
            pearson(xs, &labels[0].1),
            pearson(xs, &labels[1].1),
            pearson(xs, &labels[2].1),
        )?;
    }
    Ok(())
}

// the index of the item f makes least, the first of equals
#[inline]
fn least_of<T>(items: &[T], mut f: impl FnMut(&T) -> f64) -> usize {
    let mut least = (0, f64::INFINITY);
    for (i, item) in items.iter().enumerate() {
        let key = f(item);
        if key < least.1 {
            least = (i, key);
        }
    }
    least.0
}

struct OpennessMovesParams {
    num_games: u64,
    seed: Option<u64>,
    threads: usize,
    window: i32,
    num_candidates: usize,
    quiet: bool,
}

// hasty plays both seats; at every move with tiles in the bag, its candidates
// within window of the best (at most num_candidates of them), each with the
// exact openness of the board it leaves for the opponent -- the pool the
// opponent draws from being the tiles off that board less the mover's leave --
// and the glance counts of that board. Then, over the positions, how far the
// candidates' openness spreads, what the least open one costs and saves, and
// how often each count, and the equity, order the candidates as the openness
// does.
#[inline]
fn openness_moves<N: kwg::Node + Sync + Send, L: kwg::Node + Sync + Send>(
    game_config: game_config::GameConfig,
    kwg: kwg::Kwg<N>,
    klv: klv::Klv<L>,
    OpennessMovesParams {
        num_games,
        seed,
        threads,
        window,
        num_candidates,
        quiet,
    }: OpennessMovesParams,
) -> error::Returns<()> {
    let seed = seed_or_random(seed)?;
    writeln!(std::io::stdout(), "seed: {seed}")?;
    let num_letters = game_config.alphabet().len() as usize;
    let rack_size = game_config.rack_size() as usize;
    let lat = census::MultisetLattice::new(num_letters, rack_size)?;
    let mut leave = vec![0i32; lat.len()];
    census::fill_lattice_leaves(&lat, &mut leave, |tally| klv.leave_value_from_tally(tally));
    let no_leave = vec![0i32; lat.len()];
    let ln_binom = LnBinom::new(game_config.alphabet().num_tiles() as usize);
    let t0 = std::time::Instant::now();
    let claimed_games = std::sync::atomic::AtomicU64::new(0);
    let tick = std::sync::atomic::AtomicU64::new(0);
    let weighed = std::sync::atomic::AtomicU64::new(0);
    type Candidate = (i32, BoardOpenness, Glance);
    // a position: its game and turn, and its candidates
    type Position = ((u64, u32), Vec<Candidate>);
    let mut positions = std::thread::scope(|s| -> error::Returns<Vec<Position>> {
        let mut thread_handles = Vec::new();
        for _ in 0..threads {
            let (game_config, kwg, klv, claimed_games) = (&game_config, &kwg, &klv, &claimed_games);
            let (lat, leave, no_leave, ln_binom) = (&lat, &leave, &no_leave, &ln_binom);
            let (tick, weighed) = (&tick, &weighed);
            thread_handles.push(s.spawn(move || -> error::Returns<Vec<Position>> {
                let mut move_generator = movegen::KurniaMoveGenerator::new(game_config);
                let mut sheet_generator = movegen::KurniaMoveGenerator::new(game_config);
                let (mut movegen_rack, mut blank_deltas) = (Vec::new(), Vec::new());
                let mut sheet = Vec::new();
                let mut game_state = game_state::GameState::new(game_config);
                let mut final_scores = vec![0i32; game_config.num_players() as usize];
                let mut after = Vec::new();
                let mut out = Vec::new();
                loop {
                    let game_idx = claimed_games.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    if game_idx >= num_games {
                        break;
                    }
                    let mut rng = GameRng::from_stream(seed, game_idx);
                    game_state.reset_and_draw_tiles(game_config, &mut rng);
                    let mut num_turns = 0u32;
                    loop {
                        let board_snapshot = movegen::BoardSnapshot {
                            board_tiles: &game_state.board_tiles,
                            game_config,
                            kwg,
                            anagrams: None,
                            rack_lengths: None,
                            klv,
                        };
                        move_generator.gen_moves_unfiltered(&movegen::GenMovesParams {
                            board_snapshot: &board_snapshot,
                            rack: &game_state.current_player().rack,
                            max_gen: num_candidates,
                            num_exchanges_by_this_player: game_state.current_player().num_exchanges,
                            pass_policy: movegen::PassPolicy::OnlyWhenForced,
                            dynamic_leaves: None,
                        });
                        let best = move_generator.plays[0].equity.raw();
                        let within = move_generator
                            .plays
                            .iter()
                            .take_while(|m| best - m.equity.raw() <= window)
                            .count();
                        // the opponent's rack was dealt from the tiles the mover
                        // cannot see, whichever move the mover makes: one pool
                        let mut pool = game_config.tiles_off_board(&game_state.board_tiles)?;
                        game_config.take_rack(&mut pool, &game_state.current_player().rack)?;
                        if !game_state.bag.is_empty()
                            && within >= 2
                            && pool.iter().map(|&c| c as usize).sum::<usize>() > rack_size
                        {
                            let mut cands = Vec::with_capacity(within);
                            for m in &move_generator.plays[..within] {
                                board_after(
                                    game_config,
                                    &game_state.board_tiles,
                                    &m.play,
                                    &mut after,
                                );
                                let openness = exact_openness(
                                    &mut sheet_generator,
                                    &after,
                                    SpellTables {
                                        game_config,
                                        kwg,
                                        klv,
                                        lat,
                                    },
                                    (leave, no_leave),
                                    &pool,
                                    ln_binom,
                                    (&mut movegen_rack, &mut blank_deltas, &mut sheet),
                                );
                                cands.push((
                                    m.equity.raw(),
                                    openness,
                                    glance_at(game_config, &after),
                                ));
                            }
                            if cands.len() >= 2 {
                                out.push(((game_idx, num_turns), cands));
                                weighed.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                            }
                        }
                        let play = move_generator.plays[0].play.clone();
                        game_state.play(game_config, &mut rng, &play)?;
                        num_turns += 1;
                        let secs = t0.elapsed().as_secs();
                        // a line a second, from the first thread to end a turn in it
                        if !quiet && tick.fetch_max(secs, std::sync::atomic::Ordering::Relaxed) < secs {
                            writeln!(
                                boxed_stdout_or_stderr(),
                                "After {secs} seconds, have begun {} of {num_games} games, weighed {} positions",
                                claimed_games.load(std::sync::atomic::Ordering::Relaxed).min(num_games),
                                weighed.load(std::sync::atomic::Ordering::Relaxed),
                            )
                            .ok();
                        }
                        match game_state.check_game_ended(game_config, &mut final_scores)? {
                            game_state::CheckGameEnded::PlayedOut
                            | game_state::CheckGameEnded::ZeroScores => break,
                            game_state::CheckGameEnded::NotEnded => {}
                        }
                        game_state.next_turn();
                    }
                }
                Ok(out)
            }));
        }
        let mut positions = Vec::new();
        for handle in thread_handles {
            positions.extend(handle.join().map_err(|_| "an openness thread panicked")??);
        }
        Ok(positions)
    })?;
    // in game and turn order, whichever thread played them
    positions.sort_unstable_by_key(|&(at, _)| at);
    let n = positions.len();
    if n == 0 {
        wolges::return_error!("no move with two candidates in its window".to_string());
    }
    let points = equity::SCALE as f64;
    let num_cands: usize = positions.iter().map(|(_, c)| c.len()).sum();
    writeln!(
        std::io::stdout(),
        "{n} positions with 2 to {num_candidates} candidates within {} points of the best ({:.2} a position), in {:?}",
        window as f64 / points,
        num_cands as f64 / n as f64,
        t0.elapsed(),
    )?;
    // what the candidates leave the opponent: its spread, and the least open
    // candidate's saving against the best's, and the equity it costs
    for (name, label) in [
        (
            "best score",
            (|o: &BoardOpenness| o.best_score / equity::SCALE as f64) as fn(&BoardOpenness) -> f64,
        ),
        ("best equity", |o: &BoardOpenness| {
            o.best_equity / equity::SCALE as f64
        }),
        ("bingo%", |o: &BoardOpenness| 100.0 * o.bingo),
    ] {
        let (mut spread, mut saved, mut cost) = (0.0, 0.0, 0.0);
        for (_, cands) in &positions {
            let labels: Vec<f64> = cands.iter().map(|(_, o, _)| label(o)).collect();
            let lo = labels.iter().copied().fold(f64::INFINITY, f64::min);
            let hi = labels.iter().copied().fold(f64::NEG_INFINITY, f64::max);
            let least = least_of(&labels, |&x| x);
            spread += hi - lo;
            saved += labels[0] - labels[least];
            cost += (cands[0].0 - cands[least].0) as f64 / points;
        }
        writeln!(
            std::io::stdout(),
            "  the opponent's {name}: spread {:.3} a position; the least open candidate leaves {:.3} less than the best by equity, for {:.3} points of equity",
            spread / n as f64,
            saved / n as f64,
            cost / n as f64,
        )?;
    }
    // over every pair of candidates in a position, how often a count orders
    // them as the opponent's best score does (Kendall's rank correlation over
    // the pairs they tell apart), and how often its least is the least open
    // candidate
    type Count = fn(&Candidate) -> f64;
    let counts: [(&str, Count); 8] = [
        ("equity", |c| c.0 as f64),
        ("anchors", |c| c.2.anchors as f64),
        ("premium anchors", |c| c.2.premium_anchors as f64),
        ("word-premium anchors", |c| c.2.word_premium_anchors as f64),
        ("bingo lanes", |c| c.2.bingo_lanes as f64),
        ("stretches", |c| c.2.stretches as f64),
        ("bingo stretches", |c| c.2.bingo_stretches as f64),
        ("word-premium stretches", |c| {
            c.2.word_premium_stretches as f64
        }),
    ];
    writeln!(
        std::io::stdout(),
        "  against the opponent's best score  kendall   picks the least open"
    )?;
    for (name, count) in counts {
        let (mut agree, mut disagree, mut picks) = (0u64, 0u64, 0u64);
        for (_, cands) in &positions {
            for a in 0..cands.len() {
                for b in a + 1..cands.len() {
                    let dx = count(&cands[a]) - count(&cands[b]);
                    let dy = cands[a].1.best_score - cands[b].1.best_score;
                    if dx * dy > 0.0 {
                        agree += 1;
                    } else if dx * dy < 0.0 {
                        disagree += 1;
                    }
                }
            }
            picks += (least_of(cands, count) == least_of(cands, |c| c.1.best_score)) as u64;
        }
        writeln!(
            std::io::stdout(),
            "  {name:<34} {:>+.3}    {:.2}%",
            (agree as f64 - disagree as f64) / (agree + disagree).max(1) as f64,
            100.0 * picks as f64 / n as f64,
        )?;
    }
    Ok(())
}

#[inline]
fn full_rack_values<L: kwg::Node>(lat: &census::MultisetLattice, klv: &klv::Klv<L>) -> Vec<i32> {
    let mut full_v = vec![0i32; lat.len()];
    census::fill_lattice_leaves(lat, &mut full_v, |tally| klv.leave_value_from_tally(tally));
    full_v
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
    let mut fen_parser = display::BoardFenParser::new(alphabet, board_layout);
    let mut pool = game_config.tiles_off_board(fen_parser.parse(board_fen)?)?;
    game_config.take_rack(&mut pool, &rack)?;
    let pool_size: usize = pool.iter().map(|&c| c as usize).sum();
    let (lat, add, full_v) = lattice_leaves(&klv, num_letters, rack_size, threads)?;
    writeln!(
        std::io::stdout(),
        "rack {rack_str}, {pool_size} tiles unseen"
    )?;
    print_rack_keeps(alphabet, &klv, &lat, &add, &full_v, &pool, &rack)?;
    Ok(())
}

// the lattice of every multiset up to a full rack, and each one's value in the
// leaves; the leaves must be a --full table, which values full racks too.
#[inline]
fn lattice_leaves<L: kwg::Node>(
    klv: &klv::Klv<L>,
    num_letters: usize,
    rack_size: usize,
    threads: usize,
) -> error::Returns<(census::MultisetLattice, census::AddTable, Vec<i32>)> {
    let lat = census::MultisetLattice::new(num_letters, rack_size)?;
    let full_v = full_rack_values(&lat, klv);
    let add = census::AddTable::new_with_threads(&lat, threads);
    Ok((lat, add, full_v))
}

// every subrack of the rack: its value in the leaves, and its value once the
// tiles still to draw come from the pool.
#[inline]
fn print_rack_keeps<L: kwg::Node>(
    alphabet: &alphabet::Alphabet,
    klv: &klv::Klv<L>,
    lat: &census::MultisetLattice,
    add: &census::AddTable,
    full_v: &[i32],
    pool: &[u8],
    rack: &[u8],
) -> std::io::Result<()> {
    let num_letters = alphabet.len() as usize;
    let rack_size = lat.rack_size();
    let pool_size: usize = pool.iter().map(|&c| c as usize).sum();
    writeln!(
        std::io::stdout(),
        "  keep                 static    dynamic     delta"
    )?;
    let mut distinct: Vec<(u8, u8)> = Vec::new();
    for &tile in rack {
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
                census::dynamic_leave_value(lat, add, full_v, pool, s_ridx as usize, draw);
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
            writeln!(
                std::io::stdout(),
                "  {kept:<16} {:>10.3}  undrawable",
                static_value as f64 / scale,
            )?;
        } else {
            writeln!(
                std::io::stdout(),
                "  {kept:<16} {:>10.3} {:>10.3} {:>9.3}",
                static_value as f64 / scale,
                dynamic as f64 / scale,
                (dynamic - static_value) as f64 / scale,
            )?;
        }
    }
    Ok(())
}

// each multiset's value once the tiles still to draw come from the pool less
// itself, one draw at a time from the full racks down; NaN where the pool
// cannot hold it.
#[inline]
fn dynamic_leaves_of_pool(
    lat: &census::MultisetLattice,
    add: &census::AddTable,
    full_v: &[i32],
    pool: &[u8],
) -> Vec<f64> {
    let n = lat.num_letters();
    let full_start = lat.full_rack_start();
    let pool_size: usize = pool.iter().map(|&c| c as usize).sum();
    let mut value = vec![f64::NAN; lat.len()];
    let mut tally = vec![0u8; n];
    for idx in (0..lat.len()).rev() {
        lat.unrank_into(idx, &mut tally);
        if tally.iter().zip(pool).any(|(&s, &p)| s > p) {
            continue;
        }
        let size: usize = tally.iter().map(|&c| c as usize).sum();
        let left = pool_size - size;
        if idx >= full_start || left == 0 {
            value[idx] = full_v[idx] as f64;
            continue;
        }
        let mut sum = 0.0;
        for t in 0..n {
            let r = pool[t] - tally[t];
            if r != 0 {
                sum += r as f64 * value[add.add(idx, t)];
            }
        }
        value[idx] = sum / left as f64;
    }
    value
}

// whether the lexicon holds the word; a jumbled lexicon holds alphagrams.
#[inline]
fn lexicon_holds<N: kwg::Node>(kwg: &kwg::Kwg<N>, jumbled: bool, word: &mut [u8]) -> bool {
    if jumbled {
        word.sort_unstable();
    }
    let mut p = 0;
    for &tile in word.iter() {
        p = kwg.seek(p, tile);
        if p <= 0 {
            return false;
        }
    }
    kwg[p].accepts()
}

#[inline]
fn print_moves<N: kwg::Node, L: kwg::Node>(
    move_generator: &mut movegen::KurniaMoveGenerator,
    board_snapshot: &movegen::BoardSnapshot<'_, N, L>,
    rack: &[u8],
    num_moves: usize,
    dynamic_leaves: Option<klv::DynamicLeavesRef<'_>>,
) -> std::io::Result<()> {
    move_generator.gen_moves_unfiltered(&movegen::GenMovesParams {
        board_snapshot,
        rack,
        max_gen: num_moves,
        num_exchanges_by_this_player: 0,
        pass_policy: movegen::PassPolicy::OnlyWhenForced,
        dynamic_leaves,
    });
    for play in &move_generator.plays {
        writeln!(
            std::io::stdout(),
            "  {:>9.3}  {}",
            play.equity.raw() as f64 / equity::SCALE as f64,
            play.play.fmt(board_snapshot),
        )?;
    }
    Ok(())
}

struct PositionParams<'a> {
    fen: &'a str,
    rack: Option<&'a str>,
    num_moves: usize,
    dynamic_out: Option<&'a str>,
    threads: usize,
    lean: bool,
}

#[inline]
fn inspect_position<N: kwg::Node, L: kwg::Node>(
    game_config: game_config::GameConfig,
    kwg: kwg::Kwg<N>,
    klv: klv::Klv<L>,
    PositionParams {
        fen,
        rack,
        num_moves,
        dynamic_out,
        threads,
        lean,
    }: PositionParams<'_>,
) -> error::Returns<()> {
    let arm_tables = anagrams::ArmTables::build(&kwg, &game_config, lean)?;
    let anagrams = arm_tables.anagrams();
    let rack_lengths = arm_tables.rack_lengths();
    let alphabet = game_config.alphabet();
    let board_layout = game_config.board_layout();
    let dim = board_layout.dim();
    let num_letters = alphabet.len() as usize;
    let rack_size = game_config.rack_size() as usize;
    let mut fen_parser = display::BoardFenParser::new(alphabet, board_layout);
    let board_tiles = fen_parser.parse(fen)?.to_vec();
    display::print_board(alphabet, board_layout, &board_tiles)?;

    let off_board = game_config.tiles_off_board(&board_tiles)?;
    let mut unseen = off_board.clone();
    let mut rack_tiles = Vec::new();
    if let Some(rack) = rack {
        alphabet::AlphabetReader::new_for_racks(alphabet).set_word(rack, &mut rack_tiles)?;
        rack_tiles.sort_unstable();
        game_config.take_rack(&mut unseen, &rack_tiles)?;
    }
    let unseen_size: usize = unseen.iter().map(|&c| c as usize).sum();
    if rack.is_some() {
        writeln!(
            std::io::stdout(),
            "unseen: {unseen_size} tiles, the bag and the other racks"
        )?;
    } else {
        writeln!(
            std::io::stdout(),
            "not on the board: {unseen_size} tiles, the bag and every rack"
        )?;
    }
    let (mut vowels, mut consonants) = (0usize, 0usize);
    let mut cells = Vec::with_capacity(num_letters);
    for (tile, &count) in (0u8..).zip(unseen.iter()) {
        if tile != 0 {
            if alphabet.is_vowel(tile) {
                vowels += count as usize;
            } else {
                consonants += count as usize;
            }
        }
        cells.push(format!(
            "{:>2} {:>2}/{:<2}",
            alphabet.of_rack(tile).unwrap(),
            count,
            alphabet.freq(tile)
        ));
    }
    for line in cells.chunks(8) {
        writeln!(std::io::stdout(), "  {}", line.join("  ").trim_end())?;
    }
    writeln!(
        std::io::stdout(),
        "  vowels {vowels}, consonants {consonants}, blanks {}",
        unseen[0]
    )?;
    if rack.is_some() {
        let others = (game_config.num_players() as usize).saturating_sub(1) * rack_size;
        if unseen_size > others {
            writeln!(std::io::stdout(), "  in the bag: {}", unseen_size - others)?;
        } else if game_config.num_players() == 2 {
            let mut other = Vec::new();
            for (tile, &count) in (0u8..).zip(unseen.iter()) {
                other.extend(std::iter::repeat_n(tile, count as usize));
            }
            writeln!(
                std::io::stdout(),
                "  the bag is empty: the other rack is {}",
                alphabet.fmt_rack(&other)
            )?;
        } else {
            writeln!(std::io::stdout(), "  the bag is empty")?;
        }
    }

    let jumbled = matches!(game_config.game_rules(), game_config::GameRules::Jumbled);
    let mut num_words = 0usize;
    let mut not_words = Vec::<String>::new();
    let mut word = Vec::new();
    for down in [false, true] {
        for lane in 0..if down { dim.cols } else { dim.rows } {
            let strider = dim.lane(down, lane);
            let mut i = 0;
            while i < strider.len() {
                let start = i;
                while i < strider.len() && board_tiles[strider.at(i)] != 0 {
                    i += 1;
                }
                if i - start >= 2 {
                    num_words += 1;
                    word.clear();
                    word.extend((start..i).map(|j| board_tiles[strider.at(j)] & 0x7f));
                    if !lexicon_holds(&kwg, jumbled, &mut word) {
                        not_words.push(
                            (start..i)
                                .map(|j| alphabet.of_board(board_tiles[strider.at(j)]).unwrap())
                                .collect(),
                        );
                    }
                }
                i += 1;
            }
        }
    }
    if not_words.is_empty() {
        writeln!(
            std::io::stdout(),
            "words: {num_words}, every one in the lexicon"
        )?;
    } else {
        writeln!(
            std::io::stdout(),
            "words: {num_words}, {} not in the lexicon: {}",
            not_words.len(),
            not_words.join(" ")
        )?;
    }
    let shape = board_shape(board_layout, &board_tiles);
    writeln!(
        std::io::stdout(),
        "shape: {} tiles, mean word length {:.3}, perimeter per tile {:.3}, radius of gyration {:.3}, {} holes",
        shape.area,
        if shape.num_words > 0 {
            shape.total_word_length as f64 / shape.num_words as f64
        } else {
            0.0
        },
        if shape.area > 0 {
            shape.perimeter as f64 / shape.area as f64
        } else {
            0.0
        },
        shape.radius_of_gyration,
        shape.holes,
    )?;

    let lattice = if rack.is_some() || dynamic_out.is_some() {
        match lattice_leaves(&klv, num_letters, rack_size, threads) {
            Ok(lattice) => Some(lattice),
            Err(e) if dynamic_out.is_none() => {
                writeln!(std::io::stdout(), "no dynamic leaves: {e}")?;
                None
            }
            Err(e) => return Err(e),
        }
    } else {
        None
    };
    if let Some(rack) = rack {
        let board_snapshot = movegen::BoardSnapshot {
            board_tiles: &board_tiles,
            game_config: &game_config,
            kwg: &kwg,
            anagrams,
            rack_lengths,
            klv: &klv,
        };
        let mut move_generator = movegen::KurniaMoveGenerator::new(&game_config);
        writeln!(
            std::io::stdout(),
            "rack {rack}: the best {num_moves} moves by the leaves"
        )?;
        print_moves(
            &mut move_generator,
            &board_snapshot,
            &rack_tiles,
            num_moves,
            None,
        )?;
        if let Some((lat, add, full_v)) = &lattice {
            writeln!(
                std::io::stdout(),
                "rack {rack}: the best {num_moves} moves by dynamic leaves"
            )?;
            print_moves(
                &mut move_generator,
                &board_snapshot,
                &rack_tiles,
                num_moves,
                Some(klv::DynamicLeavesRef {
                    lat,
                    add,
                    full_v,
                    min_keep: 0,
                }),
            )?;
            writeln!(
                std::io::stdout(),
                "rack {rack}: every subrack kept, {unseen_size} tiles unseen"
            )?;
            print_rack_keeps(alphabet, &klv, lat, add, full_v, &unseen, &rack_tiles)?;
        }
    }
    if let (Some(path), Some((lat, add, full_v))) = (dynamic_out, &lattice) {
        let value = dynamic_leaves_of_pool(lat, add, full_v, &off_board);
        let baseline = value[0];
        if baseline.is_nan() {
            return Err("the tiles not on the board cannot fill a rack".into());
        }
        let written = write_census_klv2(
            lat,
            &|idx| value[idx],
            baseline,
            &|idx| !value[idx].is_nan(),
            false,
            path,
        )?;
        writeln!(
            std::io::stdout(),
            "dynamic leaves: wrote {written} to {path}, keeping nothing worth {:.3}",
            baseline / equity::SCALE as f64
        )?;
    }
    Ok(())
}

struct HarvestParams {
    num_games: u64,
    seed: u64,
    stride: u32,
    lean: bool,
    quiet: bool,
}

#[inline]
fn harvest_boards<N: kwg::Node, L: kwg::Node>(
    game_config: game_config::GameConfig,
    kwg: kwg::Kwg<N>,
    klv: klv::Klv<L>,
    out_path: &str,
    HarvestParams {
        num_games,
        seed,
        stride,
        lean,
        quiet,
    }: HarvestParams,
) -> error::Returns<()> {
    let t0 = std::time::Instant::now();
    let mut ticked = 0u64;
    let arm_tables = anagrams::ArmTables::build(&kwg, &game_config, lean)?;
    let anagrams = arm_tables.anagrams();
    let rack_lengths = arm_tables.rack_lengths();
    let alphabet = game_config.alphabet();
    let board_layout = game_config.board_layout();
    let mut move_generator = movegen::KurniaMoveGenerator::new(&game_config);
    let mut game_state = game_state::GameState::new(&game_config);
    let mut final_scores = vec![0i32; game_config.num_players() as usize];
    let mut out = std::io::BufWriter::new(make_writer(out_path)?);
    for game in 0..num_games {
        let mut rng = GameRng::seed_from_u64(census_mix64(seed.wrapping_add(census_mix64(game))));
        game_state.reset_and_draw_tiles(&game_config, &mut rng);
        let mut ply = 0u32;
        loop {
            move_generator.gen_moves_unfiltered(&movegen::GenMovesParams {
                board_snapshot: &movegen::BoardSnapshot {
                    board_tiles: &game_state.board_tiles,
                    game_config: &game_config,
                    kwg: &kwg,
                    anagrams,
                    rack_lengths,
                    klv: &klv,
                },
                rack: &game_state.current_player().rack,
                max_gen: 1,
                num_exchanges_by_this_player: game_state.current_player().num_exchanges,
                pass_policy: movegen::PassPolicy::OnlyWhenForced,
                dynamic_leaves: None,
            });
            game_state.play(&game_config, &mut rng, &move_generator.plays[0].play)?;
            ply += 1;
            if stride > 0 && ply.is_multiple_of(stride) {
                writeln!(
                    out,
                    "{}",
                    display::BoardFenner::new(alphabet, board_layout, &game_state.board_tiles),
                )?;
            }
            match game_state.check_game_ended(&game_config, &mut final_scores)? {
                game_state::CheckGameEnded::NotEnded => game_state.next_turn(),
                _ => break,
            }
        }
        writeln!(
            out,
            "{}",
            display::BoardFenner::new(alphabet, board_layout, &game_state.board_tiles),
        )?;
        let secs = t0.elapsed().as_secs();
        if !quiet && secs > ticked {
            ticked = secs;
            writeln!(
                boxed_stdout_or_stderr(),
                "After {secs} seconds, have played {} of {num_games} games",
                game + 1
            )?;
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
                    perimeter += !filled(r + dr, c + dc) as usize;
                }
            } else {
                holes += [(-1, 0), (1, 0), (0, -1), (0, 1)]
                    .iter()
                    .all(|&(dr, dc)| filled_or_off(r + dr, c + dc))
                    as usize;
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
    quiet: bool,
) -> error::Returns<()> {
    let t0 = std::time::Instant::now();
    let mut ticked = 0u64;
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
    let num_boards = text.lines().filter(|line| !line.is_empty()).count();
    for line in text.lines() {
        if line.is_empty() {
            continue;
        }
        let shape = board_shape(board_layout, fen_parser.parse(line)?);
        boards += 1;
        let secs = t0.elapsed().as_secs();
        if !quiet && secs > ticked {
            ticked = secs;
            writeln!(
                boxed_stdout_or_stderr(),
                "After {secs} seconds, have measured {boards} of {num_boards} boards"
            )?;
        }
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
    writeln!(std::io::stdout(), "boards: {boards}")?;
    for (label, stat) in [
        ("tiles on board", &area),
        ("words", &words),
        ("mean word length", &word_length),
        ("perimeter per tile", &perimeter_per_area),
        ("radius of gyration", &radius),
        ("holes", &holes),
    ] {
        writeln!(
            std::io::stdout(),
            "  {label}: {:.3} (sd={:.3})",
            stat.mean(),
            stat.standard_deviation(),
        )?;
    }
    Ok(())
}

struct CompareParams {
    num_game_pairs: u64,
    seed: Option<u64>,
    threads: usize,
    sim_threads: usize,
    p0_picker: SeatPicker,
    p1_picker: SeatPicker,
    p0_win_pct: Option<String>,
    p1_win_pct: Option<String>,
    p0_plies: usize,
    p1_plies: usize,
    p0_iters: u64,
    p1_iters: u64,
    p0_window: i32,
    p1_window: i32,
    quiet: bool,
}

struct SeatParams<'a> {
    dynamic_leaves: Option<klv::DynamicLeavesRef<'a>>,
    simmer_params: move_picker::SimmerParams<'a>,
    deft_window: i32,
}

#[inline]
fn seat_picker<'a, N: kwg::Node, L: kwg::Node>(
    picker: SeatPicker,
    game_config: &'a game_config::GameConfig,
    kwg: &'a kwg::Kwg<N>,
    arm_tables: &'a anagrams::ArmTables,
    klv: &'a klv::Klv<L>,
    SeatParams {
        dynamic_leaves,
        simmer_params,
        deft_window,
    }: SeatParams<'a>,
) -> move_picker::MovePicker<'a, N, L> {
    match picker {
        SeatPicker::Hasty => move_picker::MovePicker::Hasty,
        SeatPicker::Deft => move_picker::MovePicker::Deft(deft_window),
        SeatPicker::Dynamic => move_picker::MovePicker::Dynamic(dynamic_leaves.unwrap()),
        // reseeded before each of its turns, so its first generator is fixed
        // rather than drawn from the system
        SeatPicker::Simmer => {
            move_picker::MovePicker::Simmer(Box::new(move_picker::Simmer::with_rng(
                game_config,
                kwg,
                arm_tables,
                klv,
                simmer_params,
                wolges::simmer::DefaultRng::seed_from_u64(0),
            )))
        }
    }
}

#[inline]
fn compare_seats<
    N: kwg::Node + Sync + Send,
    L: kwg::Node + Sync + Send,
    const LEAN: bool,
    const HASTY_ONLY: bool,
>(
    game_config: game_config::GameConfig,
    kwg: kwg::Kwg<N>,
    arc_klv0: std::sync::Arc<klv::Klv<L>>,
    arc_klv1: std::sync::Arc<klv::Klv<L>>,
    CompareParams {
        num_game_pairs,
        seed,
        threads,
        sim_threads,
        p0_picker,
        p1_picker,
        p0_win_pct,
        p1_win_pct,
        p0_plies,
        p1_plies,
        p0_iters,
        p1_iters,
        p0_window,
        p1_window,
        quiet,
    }: CompareParams,
) -> error::Returns<()> {
    let game_config = std::sync::Arc::new(game_config);
    let kwg = std::sync::Arc::new(kwg);
    let seed = seed_or_random(seed)?;
    writeln!(boxed_stdout_or_stderr(), "seed: {seed}")?;
    let num_threads = threads;
    let claimed_pairs = std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0));
    let finished_pairs = std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0));
    let reported_secs = std::sync::atomic::AtomicU64::new(0);
    let t0 = std::time::Instant::now();

    let arm_tables = anagrams::ArmTables::build(&kwg, &game_config, LEAN)?;

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
    let dyn_ctx: Option<(census::MultisetLattice, census::AddTable, [Vec<i32>; 2])> =
        if matches!(p0_picker, SeatPicker::Dynamic) || matches!(p1_picker, SeatPicker::Dynamic) {
            let num_letters = game_config.alphabet().len() as usize;
            let rack_size = game_config.rack_size() as usize;
            let lat = census::MultisetLattice::new(num_letters, rack_size)?;
            let full_v_of = |picker: SeatPicker, klv: &klv::Klv<L>| match picker {
                SeatPicker::Dynamic => full_rack_values(&lat, klv),
                SeatPicker::Hasty | SeatPicker::Simmer | SeatPicker::Deft => Vec::new(),
            };
            let full_v = [
                full_v_of(p0_picker, &arc_klv0),
                full_v_of(p1_picker, &arc_klv1),
            ];
            let add = census::AddTable::new_with_threads(&lat, num_threads);
            Some((lat, add, full_v))
        } else {
            None
        };
    let dyn_refs = dyn_ctx.as_ref().map(|(lat, add, full_v)| {
        [0, 1].map(|seat| klv::DynamicLeavesRef {
            lat,
            add,
            full_v: full_v[seat].as_slice(),
            min_keep: 0,
        })
    });
    let seat_line = |picker: SeatPicker,
                     plies: usize,
                     iters: u64,
                     win_pct: Option<&str>,
                     window: i32| match picker {
        SeatPicker::Simmer => format!(
            "{} ({iters} rollouts a move, {plies} plies each, win%={})",
            picker_name(picker),
            win_pct.unwrap_or("sigmoid"),
        ),
        SeatPicker::Deft => format!(
            "{} (of its moves within {} of its best equity, the one leaving the fewest bingo lanes)",
            picker_name(picker),
            equity::Equity::new(window),
        ),
        SeatPicker::Hasty | SeatPicker::Dynamic => picker_name(picker).to_string(),
    };
    writeln!(
        boxed_stdout_or_stderr(),
        "compare: the {} arm; p0 {}; p1 {}",
        if LEAN { "lean" } else { "unadorned" },
        seat_line(
            p0_picker,
            p0_plies,
            p0_iters,
            p0_win_pct.as_deref(),
            p0_window
        ),
        seat_line(
            p1_picker,
            p1_plies,
            p1_iters,
            p1_win_pct.as_deref(),
            p1_window
        ),
    )?;

    std::thread::scope(|s| -> error::Returns<()> {
        let mut thread_handles = Vec::new();
        for _ in 0..num_threads {
            let game_config = std::sync::Arc::clone(&game_config);
            let kwg = std::sync::Arc::clone(&kwg);
            let arc_klv0 = std::sync::Arc::clone(&arc_klv0);
            let arc_klv1 = std::sync::Arc::clone(&arc_klv1);
            let arm_tables = &arm_tables;
            let claimed_pairs = std::sync::Arc::clone(&claimed_pairs);
            let finished_pairs = std::sync::Arc::clone(&finished_pairs);
            let reported_secs = &reported_secs;
            thread_handles.push(s.spawn(move || -> error::Returns<GamePairStats> {
                let mut move_generator = movegen::KurniaMoveGenerator::new(&game_config);
                let mut filtered_movegen = move_filter::GenMoves::Unfiltered;
                let mut driver_p0 = seat_picker(
                    p0_picker,
                    &game_config,
                    &kwg,
                    arm_tables,
                    &arc_klv0,
                    SeatParams {
                        dynamic_leaves: dyn_refs.map(|refs| refs[0]),
                        simmer_params: move_picker::SimmerParams {
                            num_sim_iters: p0_iters,
                            num_sim_plies: p0_plies,
                            sim_threads,
                            win_pct_table: winpct_p0,
                            quiet: true,
                        },
                        deft_window: p0_window,
                    },
                );
                let mut driver_p1 = seat_picker(
                    p1_picker,
                    &game_config,
                    &kwg,
                    arm_tables,
                    &arc_klv1,
                    SeatParams {
                        dynamic_leaves: dyn_refs.map(|refs| refs[1]),
                        simmer_params: move_picker::SimmerParams {
                            num_sim_iters: p1_iters,
                            num_sim_plies: p1_plies,
                            sim_threads,
                            win_pct_table: winpct_p1,
                            quiet: true,
                        },
                        deft_window: p1_window,
                    },
                );
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

                    let mut rng = GameRng::from_stream(seed, pair_idx);
                    game_state.reset_and_draw_tiles_double_ended(&game_config, &mut rng);
                    saved_game_state.clone_from(&game_state);
                    let saved_rng = rng.copy_state();

                    let mut pair_diverged = false;
                    let mut pair_results =
                        [(0i32, 0i32, 0u32, game_state::CheckGameEnded::NotEnded); 2];

                    for game_in_pair in 0..2u8 {
                        if game_in_pair > 0 {
                            game_state.clone_from(&saved_game_state);
                            rng = saved_rng.copy_state();
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
                                anagrams: arm_tables.anagrams(),
                                rack_lengths: arm_tables.rack_lengths(),
                                klv: if is_p0_seat { &arc_klv0 } else { &arc_klv1 },
                            };
                            let driver = if is_p0_seat {
                                &mut driver_p0
                            } else {
                                &mut driver_p1
                            };

                            if let move_picker::MovePicker::Simmer(simmer_driver) = &mut *driver {
                                // https://doi.org/10.1145/2714064.2660195
                                // https://prng.di.unimi.it/splitmix64.c
                                simmer_driver.reseed(census_mix64(
                                    seed.wrapping_add(census_mix64(
                                        pair_idx
                                            .wrapping_mul(0x9e3779b97f4a7c15)
                                            .wrapping_add((game_in_pair as u64).wrapping_shl(40))
                                            .wrapping_add(num_turns as u64),
                                    )),
                                ));
                            }
                            if HASTY_ONLY || matches!(driver, move_picker::MovePicker::Hasty) {
                                let gen_params = movegen::GenMovesParams {
                                    board_snapshot: &board_snapshot,
                                    rack: &game_state.current_player().rack,
                                    max_gen: 1,
                                    num_exchanges_by_this_player: game_state
                                        .current_player()
                                        .num_exchanges,
                                    pass_policy: movegen::PassPolicy::OnlyWhenForced,
                                    dynamic_leaves: None,
                                };
                                if LEAN {
                                    move_generator.gen_moves_unfiltered_lean(&gen_params);
                                } else {
                                    move_generator.gen_moves_unfiltered(&gen_params);
                                }
                            } else {
                                driver.pick_a_move(
                                    &mut filtered_movegen,
                                    &mut move_generator,
                                    &board_snapshot,
                                    &game_state,
                                    &game_state.current_player().rack,
                                )?;
                            }
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
                            game_state.play(&game_config, &mut rng, play)?;
                            num_turns += 1;
                            let end =
                                game_state.check_game_ended(&game_config, &mut final_scores)?;
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
                    if !quiet && secs > prev {
                        writeln!(
                            boxed_stdout_or_stderr(),
                            "After {secs}s: {} pairs",
                            finished_pairs.load(std::sync::atomic::Ordering::Relaxed),
                        )
                        .ok();
                    }
                }

                Ok(stats)
            }));
        }

        let mut combined = GamePairStats::new();
        for handle in thread_handles {
            combined.merge(&handle.join().unwrap()?);
        }

        writeln!(std::io::stdout())?;
        combined.print()?;

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

    #[test]
    #[inline]
    fn dynamic_leaves_of_a_pool_match_each_leave_drawn_alone() {
        let lat = census::MultisetLattice::new(4, 3).unwrap();
        let add = census::AddTable::new_with_threads(&lat, 1);
        let full_v = (0..lat.len() as i32)
            .map(|i| i * 7919 % 1000 - 500)
            .collect::<Vec<_>>();
        let pool = [1u8, 2, 0, 3];
        let pool_size: usize = pool.iter().map(|&c| c as usize).sum();
        let value = dynamic_leaves_of_pool(&lat, &add, &full_v, &pool);
        let mut tally = vec![0u8; lat.num_letters()];
        let mut checked = 0;
        for (idx, &got) in value.iter().enumerate() {
            lat.unrank_into(idx, &mut tally);
            if tally.iter().zip(&pool).any(|(&s, &p)| s > p) {
                assert!(got.is_nan(), "{tally:?} does not fit the pool");
                continue;
            }
            let size: usize = tally.iter().map(|&c| c as usize).sum();
            let rest = pool
                .iter()
                .zip(&tally)
                .map(|(&p, &s)| p - s)
                .collect::<Vec<_>>();
            let draw = (lat.rack_size() - size).min(pool_size - size);
            let want = census::dynamic_leave_value(&lat, &add, &full_v, &rest, idx, draw);
            assert!(
                (got - want as f64).abs() <= 1.0,
                "{tally:?}: {got} against {want}"
            );
            checked += 1;
        }
        assert!(checked > 10, "only {checked} leaves fit the pool");
    }

    #[test]
    #[inline]
    fn each_tally_of_size_deals_every_multiset_once() {
        let avail = [2u8, 1, 1];
        let cap = [4usize, 2, 1, 0];
        let mut cur = [0u8; 3];
        let mut seen = Vec::new();
        each_tally_of_size(&avail, &cap, 0, 2, &mut cur, &mut |tally| {
            seen.push(tally.to_vec())
        });
        seen.sort();
        assert_eq!(
            seen,
            vec![vec![0, 1, 1], vec![1, 0, 1], vec![1, 1, 0], vec![2, 0, 0]]
        );
        assert_eq!(cur, [0, 0, 0]);
    }

    // a pool of AABC dealing racks of 2. Playing B scores 10 points, AB 12, AC
    // 7, and nothing else plays (scoring 0 when it can be exchanged instead);
    // keeping A is worth 1 point, C 3
    #[inline]
    fn weigh_racks_of_aabc(
        can_exchange: bool,
        tiles: MoveTiles<'_>,
        move_value: i32,
    ) -> Vec<(Vec<u8>, f64, i32)> {
        let lat = census::MultisetLattice::new(3, 2).unwrap();
        let ln_binom = LnBinom::new(4);
        // as infer_sheet_for leaves it: playing nothing is no entry
        let mut sheet = vec![if can_exchange { 0 } else { i32::MIN / 4 }; lat.len()];
        let mut leave = vec![0i32; lat.len()];
        sheet[lat.rank(&[0, 0, 0]) as usize] = i32::MIN / 4;
        sheet[lat.rank(&[0, 1, 0]) as usize] = 10_000;
        sheet[lat.rank(&[1, 1, 0]) as usize] = 12_000;
        sheet[lat.rank(&[1, 0, 1]) as usize] = 7_000;
        leave[lat.rank(&[1, 0, 0]) as usize] = 1_000;
        leave[lat.rank(&[0, 0, 1]) as usize] = 3_000;
        let is = InferSheet {
            lat: &lat,
            sheet: &sheet,
            leave: &leave,
            ln_binom: &ln_binom,
        };
        let mut got = Vec::new();
        weigh_racks(
            &is,
            &[2, 1, 1],
            tiles,
            move_value,
            &mut Vec::new(),
            &mut |rank, ln_prior, gap| got.push((lat.tally(rank as usize), ln_prior.exp(), gap)),
        );
        got.sort_by(|a, b| a.0.cmp(&b.0));
        got
    }

    #[test]
    #[inline]
    fn weigh_racks_finds_the_racks_a_placed_move_could_come_from() {
        // B played for 10: from BC (best: B, keeping C, 13) it gives up nothing;
        // from AB (best: AB, 12; the move keeps A, 11) it gives up 1. Of the 6
        // ways to deal 2 of AABC, 1 is BC and 2 are AB. Exchanging or not
        // changes nothing here.
        let got = weigh_racks_of_aabc(true, MoveTiles::Placed(&[0, 1, 0]), 10_000);
        assert_eq!(got.len(), 2);
        assert_eq!((&got[0].0[..], got[0].2), (&[0u8, 1, 1][..], 0));
        assert_eq!((&got[1].0[..], got[1].2), (&[1u8, 1, 0][..], 1_000));
        assert!((got[0].1 - 1.0 / 6.0).abs() < 1e-12);
        assert!((got[1].1 - 2.0 / 6.0).abs() < 1e-12);
        assert_eq!(
            got,
            weigh_racks_of_aabc(false, MoveTiles::Placed(&[0, 1, 0]), 10_000)
        );
    }

    #[test]
    #[inline]
    fn weigh_racks_weighs_a_pass_against_each_racks_best_move() {
        // no exchanges: AA has no move, so its pass gives up nothing; AB gives
        // up AB (12), AC gives up AC (7), BC gives up B keeping C (13)
        let got = weigh_racks_of_aabc(false, MoveTiles::Placed(&[0, 0, 0]), 0);
        let racks: Vec<(&[u8], i32)> = got.iter().map(|g| (&g.0[..], g.2)).collect();
        assert_eq!(
            racks,
            vec![
                (&[0u8, 1, 1][..], 13_000),
                (&[1u8, 0, 1][..], 7_000),
                (&[1u8, 1, 0][..], 12_000),
                (&[2u8, 0, 0][..], 0),
            ]
        );
        // with exchanges, AA's best is exchanging one A to keep the other (1)
        let got = weigh_racks_of_aabc(true, MoveTiles::Placed(&[0, 0, 0]), 0);
        assert_eq!((&got[3].0[..], got[3].2), (&[2u8, 0, 0][..], 1_000));
    }

    #[test]
    #[inline]
    fn weigh_racks_weighs_every_rack_behind_an_exchange() {
        // one tile exchanged, the rest kept: AA keeps A (1, its best: gap 0);
        // AB keeps A (1) for 12; AC keeps C (3) for 7; BC keeps C (3) for 13
        let got = weigh_racks_of_aabc(true, MoveTiles::Exchanged(1), 0);
        let racks: Vec<(&[u8], i32)> = got.iter().map(|g| (&g.0[..], g.2)).collect();
        assert_eq!(
            racks,
            vec![
                (&[0u8, 1, 1][..], 10_000),
                (&[1u8, 0, 1][..], 4_000),
                (&[1u8, 1, 0][..], 11_000),
                (&[2u8, 0, 0][..], 0),
            ]
        );
        assert!((got.iter().map(|g| g.1).sum::<f64>() - 1.0).abs() < 1e-12);
    }

    #[test]
    #[inline]
    fn parse_infer_play_reads_a_move_whole_or_refuses_it() {
        let game_config = game_config::make_english_game_config();
        let kwg = kwg::Kwg::<kwg::Node22>::from_bytes_alloc(kwg::EMPTY_KWG_BYTES).unwrap();
        let klv = klv::Klv::<kwg::Node22>::from_bytes_alloc(klv::EMPTY_KLV_BYTES).unwrap();
        let dim = game_config.board_layout().dim();
        let mut board_tiles = vec![0u8; dim.rows as usize * dim.cols as usize];
        // QI across row 8 from column H
        board_tiles[dim.at_row_col(7, 7)] = 17;
        board_tiles[dim.at_row_col(7, 8)] = 9;
        let board_snapshot = movegen::BoardSnapshot {
            board_tiles: &board_tiles,
            game_config: &game_config,
            kwg: &kwg,
            anagrams: None,
            rack_lengths: None,
            klv: &klv,
        };
        let read = |s: &str| parse_infer_play(&board_snapshot, s);
        let refused = |s: &str| read(s).err().map(|e| e.to_string()).unwrap_or_default();
        assert!(matches!(
            read("8H QIS"),
            Ok(movegen::Play::Place { down: false, lane: 7, idx: 7, ref word, .. })
                if word[..] == [0, 0, 19]
        ));
        assert!(read("8H ..S").is_ok());
        assert!(read("8H QIX").is_ok());
        assert!(refused("8I IS").contains("leaves out a tile"));
        assert!(refused("8J S").contains("one-tile move"));
        assert!(refused("H8 Q.").contains("no tile under"));
        assert!(refused("H-128 QI").contains("is not a square"));
        assert!(refused("8+H QI").contains("is not a square"));
        assert!(refused("16A QI").contains("is not a square"));
        assert!(refused("8H QAS").contains("crosses another tile"));
        assert!(
            matches!(read("exch 3"), Ok(movegen::Play::Exchange { tiles }) if tiles.len() == 3)
        );
        assert!(refused("exch 8").contains("cannot exchange"));
    }

    #[test]
    #[inline]
    fn fit_log_likelihood_reads_the_true_rack_against_the_rest() {
        let moves = [FitMove {
            at: (0, 0),
            buckets: vec![(0, 0.5), (1_000, 0.5)],
            true_prior: 0.5,
            true_gap: 0,
            num_short: 0,
            kind: MoveKind::Placed,
        }];
        let want = -(1.0 + (-1.0f64).exp()).ln();
        assert!((fit_log_likelihood(&moves, 1.0) - want).abs() < 1e-12);
    }

    #[test]
    #[inline]
    fn equity_fuzz_refuses_nan_and_below_0_and_nothing_else() {
        for fuzz in [f64::NAN, f64::NEG_INFINITY, -1.0, -f64::MIN_POSITIVE] {
            assert!(equity_fuzz(fuzz).is_err(), "{fuzz} is taken");
        }
        for fuzz in [
            0.0,
            -0.0,
            f64::MIN_POSITIVE,
            0.5,
            2.5,
            f64::MAX,
            f64::INFINITY,
        ] {
            assert!(equity_fuzz(fuzz).is_ok(), "{fuzz} is refused");
        }
    }

    #[test]
    #[inline]
    fn weigh_by_fuzz_at_0_keeps_the_racks_at_the_smallest_gap_alone() {
        // two racks tie at the smallest gap, half a point; one gives up a
        // thousandth of a point more
        let racks = [
            (0, 0.1f64.ln(), 500),
            (1, 0.2f64.ln(), 501),
            (2, 0.3f64.ln(), 500),
            (3, 0.4f64.ln(), 9_000),
        ];
        let weighed = weigh_by_fuzz(&racks, 0.0);
        let z: f64 = weighed.iter().map(|&(w, ..)| w).sum();
        for (&(w, prior, rank, gap), &(r, ln_prior, g)) in weighed.iter().zip(&racks) {
            assert_eq!((prior, rank, gap), (ln_prior.exp(), r, g));
            assert_eq!(w, if g == 500 { prior } else { 0.0 });
        }
        assert!((weighed[0].0 / z - 0.25).abs() < 1e-12);
        assert!((weighed[2].0 / z - 0.75).abs() < 1e-12);
        // a gap below 0 counts as 0, so it ties with 0
        let weighed = weigh_by_fuzz(
            &[
                (0, 0.1f64.ln(), -300),
                (1, 0.2f64.ln(), 1),
                (2, 0.3f64.ln(), 0),
            ],
            0.0,
        );
        let kept: Vec<f64> = weighed.iter().map(|&(w, ..)| w).collect();
        assert_eq!(kept, vec![weighed[0].1, 0.0, weighed[2].1]);
    }

    #[test]
    #[inline]
    fn weigh_by_fuzz_above_0_takes_exp_of_the_gap_over_fuzz() {
        let racks = [
            (0, 0.1f64.ln(), -300),
            (1, 0.2f64.ln(), 500),
            (2, 0.3f64.ln(), 2_345),
            (3, 0.4f64.ln(), 9_000),
        ];
        for points in [0.001, 0.5, 2.5, 1e6] {
            let weighed = weigh_by_fuzz(&racks, equity_fuzz(points).unwrap());
            for (&(w, prior, rank, gap), &(r, ln_prior, g)) in weighed.iter().zip(&racks) {
                assert_eq!((prior, rank, gap), (ln_prior.exp(), r, g));
                let fuzz = points * equity::SCALE as f64;
                assert_eq!(w, prior * (-(g.max(0) as f64) / fuzz).exp());
            }
        }
    }

    #[test]
    #[inline]
    fn fit_log_likelihood_at_0_is_minus_infinity_past_the_smallest_gap() {
        let fit_move = |true_gap| FitMove {
            at: (0, 0),
            buckets: vec![(-300, 0.25), (0, 0.25), (1_000, 0.5)],
            true_prior: 0.25,
            true_gap,
            num_short: 1,
            kind: MoveKind::Placed,
        };
        assert_eq!(
            fit_log_likelihood(&[fit_move(1_000)], 0.0),
            f64::NEG_INFINITY
        );
        assert_eq!(
            fit_log_likelihood(&[fit_move(0), fit_move(1_000)], 0.0),
            f64::NEG_INFINITY
        );
        // at the smallest gap, the true rack's share of the racks there
        assert!((fit_log_likelihood(&[fit_move(0)], 0.0) - 0.5f64.ln()).abs() < 1e-12);
    }

    #[test]
    #[inline]
    fn an_empty_board_shows_its_star_and_the_stretches_through_it() {
        let game_config = game_config::make_english_game_config();
        let dim = game_config.board_layout().dim();
        let board_tiles = vec![0u8; dim.rows as usize * dim.cols as usize];
        let g = glance_at(&game_config, &board_tiles);
        // the star alone, a double word; its row and its column each hold the
        // stretches of 2 to 7 squares through it: 2 + 3 + ... + 7 = 27, of
        // which 7 have seven squares
        assert_eq!(
            (g.anchors, g.premium_anchors, g.word_premium_anchors),
            (1, 1, 1)
        );
        assert_eq!(
            (
                g.bingo_lanes,
                g.stretches,
                g.bingo_stretches,
                g.word_premium_stretches
            ),
            (2, 54, 14, 54)
        );
    }

    #[test]
    #[inline]
    fn board_after_puts_down_what_a_move_places_and_least_of_keeps_the_first() {
        let game_config = game_config::make_english_game_config();
        let dim = game_config.board_layout().dim();
        let mut board_tiles = vec![0u8; dim.rows as usize * dim.cols as usize];
        board_tiles[dim.at_row_col(7, 8)] = 9;
        let mut after = Vec::new();
        board_after(
            &game_config,
            &board_tiles,
            &movegen::Play::Place {
                down: false,
                lane: 7,
                idx: 7,
                word: [17u8, 0, 19][..].into(),
                score: 0,
            },
            &mut after,
        );
        assert_eq!(
            (
                after[dim.at_row_col(7, 7)],
                after[dim.at_row_col(7, 8)],
                after[dim.at_row_col(7, 9)]
            ),
            (17, 9, 19)
        );
        assert_eq!(after.iter().filter(|&&t| t != 0).count(), 3);
        assert_eq!(least_of(&[3.0, 1.0, 1.0, 2.0], |&x| x), 1);
        assert_eq!(least_of(&[] as &[f64], |&x| x), 0);
    }
}
