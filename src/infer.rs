// Copyright (C) 2020-2026 Andy Kurnia.

// inferring the rack behind a move: every rack the unseen tiles could have
// dealt the mover, weighted by its chance of being dealt and by how far the
// move falls short of that rack's best play, valued the way the census sheet
// values a board

use super::{census, equity, error, game_config, klv, kwg, movegen, play_scorer};

// how far, in points of equity, a sim takes the opponent's last move to fall
// short of their best when it weighs the racks that could have made it
pub const FUZZ: f64 = 3.105;

// the fuzz fitted for a move made from a bag of 1, 2, and so on tiles, as many
// as a game's bag starts with: a bag of another game is read at the same share
// of its own starting bag, log-linearly between and held at the ends
pub const FUZZ_BY_BAG: [f64; 86] = [
    69.796, 40.518, 24.709, 16.851, 12.431, 9.759, 8.251, 7.282, 6.737, 6.317, 6.271, 6.161, 5.770,
    5.425, 5.255, 5.083, 4.780, 4.417, 4.235, 4.104, 4.065, 3.986, 3.875, 3.772, 3.749, 3.717,
    3.796, 4.023, 3.911, 3.678, 3.310, 3.108, 2.871, 2.600, 2.375, 2.429, 2.539, 2.468, 2.283,
    2.098, 1.909, 1.909, 1.949, 2.047, 1.944, 1.798, 1.736, 1.727, 1.730, 1.783, 1.814, 1.772,
    1.720, 1.763, 1.800, 1.843, 1.779, 1.654, 1.589, 1.603, 1.580, 1.591, 1.587, 1.608, 1.657,
    1.639, 1.605, 1.610, 1.614, 1.596, 1.578, 1.524, 1.475, 1.444, 1.474, 1.524, 1.527, 1.492,
    1.484, 1.469, 1.481, 1.508, 1.516, 1.516, 1.512, 1.507,
];

// the fuzz fitted for an exchange, from any bag
pub const FUZZ_EXCHANGE: f64 = 3.334;

// the share of an inference after a move but an exchange that weighs each rack
// by its chance to be dealt alone
pub const FUZZ_PRIOR_MIX: f64 = 0.021;

// the fuzz and the prior mix fitted for a move made from a bag of bag_before
// tiles, of start_bag at the game's start
#[inline]
pub fn fitted_fuzz(bag_before: usize, start_bag: usize, exchanged: bool) -> (f64, f64) {
    if exchanged {
        return (FUZZ_EXCHANGE, 0.0);
    }
    let n = FUZZ_BY_BAG.len();
    let x = (bag_before as f64 * n as f64 / start_bag.max(1) as f64).clamp(1.0, n as f64);
    let i = (x.floor() as usize).clamp(1, n - 1);
    let t = x - i as f64;
    (
        FUZZ_BY_BAG[i - 1].powf(1.0 - t) * FUZZ_BY_BAG[i].powf(t),
        FUZZ_PRIOR_MIX,
    )
}

#[inline]
pub fn bag_size(board_tiles: &[u8], game_config: &game_config::GameConfig) -> i64 {
    let on_board = board_tiles.iter().filter(|&&t| t != 0).count() as i64;
    i64::from(game_config.alphabet().num_tiles())
        - (on_board + i64::from(game_config.num_players()) * i64::from(game_config.rack_size()))
}

pub struct SpellTables<'a, N: kwg::Node, L: kwg::Node> {
    pub game_config: &'a game_config::GameConfig,
    pub kwg: &'a kwg::Kwg<N>,
    pub klv: &'a klv::Klv<L>,
    pub lat: &'a census::MultisetLattice,
}

pub struct SpellPool<'a> {
    pub unseen_tally: &'a [u8],
    pub num_blanks_eff: usize,
    pub rack_size: usize,
    pub phase: BoardPhase,
}

#[derive(Clone, Copy, PartialEq)]
pub enum BoardPhase {
    Opening,
    Midgame,
    Endgame,
}

impl BoardPhase {
    #[inline(always)]
    pub fn of_board(board_tiles: &[u8], game_config: &game_config::GameConfig) -> Self {
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
    pub fn name(self) -> &'static str {
        match self {
            BoardPhase::Opening => "opening",
            BoardPhase::Midgame => "midgame",
            BoardPhase::Endgame => "endgame",
        }
    }
}

#[inline]
pub fn opening_adjustment(
    phase: BoardPhase,
    game_config: &game_config::GameConfig,
    down: bool,
    idx: i8,
    word: &[u8],
) -> error::Returns<i32> {
    if phase != BoardPhase::Opening {
        return Ok(0);
    }
    let alphabet = game_config.alphabet();
    let hotspots = game_config.board_layout().danger_stars(down);
    let from = usize::try_from(idx)?;
    let danger = hotspots
        .get(from..)
        .and_then(|tail| tail.get(..word.len()))
        .ok_or("opening play extends outside the board")?;
    Ok(-(equity::OPENING_HOTSPOT_PENALTY
        * word
            .iter()
            .zip(danger)
            .filter(|&(&tile, &danger)| danger && tile != 0 && alphabet.is_vowel(tile))
            .count() as i32))
}

#[inline]
pub fn build_sheet_spell_once<
    N: kwg::Node,
    L: kwg::Node,
    G: movegen::LaneBits,
    const JUMPS: bool,
    const ANY_SIDE: bool,
>(
    move_generator: &mut movegen::KurniaMoveGenerator<G, JUMPS, ANY_SIDE>,
    board_tiles: &[u8],
    tables: SpellTables<'_, N, L>,
    pool: SpellPool<'_>,
    movegen_rack: &mut Vec<u8>,
    blank_deltas: &mut Vec<(u8, i32)>,
    sheet: &mut [i32],
) -> error::Returns<u64> {
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
    census::check_letters(unseen_tally.len())?;
    let num_letters = u8::try_from(unseen_tally.len())?;
    if usize::from(num_letters) != lat.num_letters() || num_letters != game_config.alphabet().len()
    {
        crate::return_error!("inference pool does not match the alphabet and lattice".to_string());
    }
    let mut n_cand = 0u64;
    let mut failure = None;
    let board_snapshot = &movegen::BoardSnapshot {
        board_tiles,
        game_config,
        kwg,
        anagrams: None,
        rack_lengths: None,
        klv,
    };
    // jumbled, a multiset's arrangements on a span differ only in score and in
    // the opening's adjustment, which the walk weighs too, so each span offers
    // each multiset (and blank designation) once, at its best
    let jumbled = match game_config.game_rules() {
        game_config::GameRules::Classic | game_config::GameRules::Woadwage => false,
        game_config::GameRules::Jumbled | game_config::GameRules::JumbledWoadwage => true,
    };
    if jumbled {
        movegen_rack.clear();
        for (t, &c) in (0..num_letters).zip(unseen_tally) {
            for _ in 0..c {
                movegen_rack.push(t);
            }
        }
        let mut tally = [0u8; census::MAX_LETTERS];
        let tally = &mut tally[..unseen_tally.len()];
        move_generator.pool_plays_best_window(
            board_snapshot,
            movegen_rack,
            (u128::MAX, u128::MAX),
            0..u64::MAX,
            &mut movegen::PoolLists::default(),
            |down, _, idx, word: &[u8], score: i32| {
                if failure.is_some() {
                    return;
                }
                let adjustment = match opening_adjustment(phase, game_config, down, idx, word) {
                    Ok(value) => value,
                    Err(error) => {
                        failure = Some(error);
                        return;
                    }
                };
                n_cand += 1;
                let score = score + adjustment;
                tally.fill(0);
                for &tile in word {
                    if tile != 0 {
                        tally[if tile & 0x80 != 0 { 0 } else { tile as usize }] += 1;
                    }
                }
                let slot = &mut sheet[lat.rank_in_lattice(tally) as usize];
                *slot = (*slot).max(score);
            },
        );
        return failure.map_or(Ok(n_cand), Err);
    }
    movegen_rack.clear();
    for (t, &c) in (0..num_letters).zip(unseen_tally) {
        for _ in 0..(c as usize).min(rack_size) {
            movegen_rack.push(t);
        }
    }
    let params = movegen::GenMovesParams {
        board_snapshot,
        rack: &movegen_rack[..],
        max_gen: 1,
        num_exchanges_by_this_player: i16::MAX,
        pass_policy: movegen::PassPolicy::OnlyWhenForced,
        dynamic_leaves: None,
    };
    // the snapshot holds no tables, so the lean arm alone runs it
    move_generator.gen_census_sheet_lean(
        &params,
        movegen::PlacePredicate::Dyn(
            &mut |down, lane, idx, word: &[u8], _score: equity::Equity| {
                if failure.is_some() {
                    return false;
                }
                n_cand += 1;
                let adjustment = match opening_adjustment(phase, game_config, down, idx, word) {
                    Ok(value) => value,
                    Err(error) => {
                        failure = Some(error);
                        return false;
                    }
                };
                let real_score = play_scorer::score_and_blank_deltas(
                    board_snapshot,
                    down,
                    lane,
                    idx,
                    word,
                    blank_deltas,
                ) + adjustment;
                if let Err(error) = census::record_blank_variants(
                    lat,
                    sheet,
                    equity::Equity::new(real_score),
                    blank_deltas,
                    unseen_tally,
                    num_blanks_eff,
                ) {
                    failure = Some(error);
                }
                false // never keep the move
            },
        ),
        klv::AdjustLeave::Identity,
        movegen::EquityPredicate::RejectAll,
    )?;
    match failure {
        Some(error) => Err(error),
        None => Ok(n_cand),
    }
}

// what a move shows of the tiles it came from: the tiles it placed (none for a
// pass), or only how many it exchanged
pub enum MoveTiles<'a> {
    Placed(&'a [u8]),
    Exchanged(usize),
}

// ln C(n, k) for every n up to a pool's size
pub struct LnBinom {
    ln_fact: Vec<f64>,
}

impl LnBinom {
    #[inline]
    pub fn new(max_n: usize) -> Self {
        let mut ln_fact = Vec::with_capacity(max_n + 1);
        ln_fact.push(0.0);
        for i in 1..=max_n {
            ln_fact.push(ln_fact[i - 1] + (i as f64).ln());
        }
        Self { ln_fact }
    }

    #[inline(always)]
    pub fn ln_binom(&self, n: usize, k: usize) -> f64 {
        self.ln_fact[n] - self.ln_fact[k] - self.ln_fact[n - k]
    }
}

// every tally of m tiles that avail holds, to f; cap[t] is how many tiles avail
// holds from letter t on
#[inline]
pub fn each_tally_of_size(
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

pub struct ObservedMove<'a> {
    pub tiles: MoveTiles<'a>,
    pub value: i32,
}

// the board's half of an inference: what each multiset of the pool, played, can
// score at best (the census sheet), and what each multiset is worth kept
pub struct InferSheet<'a> {
    pub lat: &'a census::MultisetLattice,
    pub sheet: &'a [i32],
    pub leave: &'a [i32],
    pub ln_binom: &'a LnBinom,
}

// how many tallies of m tiles avail holds
#[inline]
pub fn count_tallies_of_size(avail: &[u8], m: usize) -> usize {
    count_tallies_of_size_in(avail, m, &mut Vec::new())
}

// the same, counted in ways, whose memory is kept
#[inline]
pub fn count_tallies_of_size_in(avail: &[u8], m: usize, ways: &mut Vec<usize>) -> usize {
    // ways[k] is the tallies of k tiles from the letters read so far
    ways.clear();
    ways.resize(m + 1, 0);
    ways[0] = 1;
    for &a in avail {
        for k in (1..=m).rev() {
            ways[k] += (1..=(a as usize).min(k))
                .map(|c| ways[k - c])
                .sum::<usize>();
        }
    }
    ways[m]
}

// every tally of m tiles that avail holds, by the counts of its first k letters:
// f(those counts, the tiles the letters after draw), each prefix once, in the
// order each_tally_of_size reaches them; the tallies of a prefix are the ones
// each_tally_of_size walks from letter k with cur holding the prefix
#[inline]
pub fn each_prefix_of_size(
    avail: &[u8],
    cap: &[usize],
    t: usize,
    k: usize,
    left: usize,
    cur: &mut [u8],
    f: &mut impl FnMut(&[u8], usize),
) {
    if cap[t] < left {
        return;
    }
    if t == k {
        f(&cur[..k], left);
        return;
    }
    for c in 0..=(avail[t] as usize).min(left) {
        cur[t] = c as u8;
        each_prefix_of_size(avail, cap, t + 1, k, left - c, cur, f);
    }
    cur[t] = 0;
}

// weigh_racks weighs the racks on the threads it is given once they number this
// many; fewer are weighed before as many threads could start
pub const WEIGH_ACROSS_THREADS_FROM: usize = 1024;

// the buffers weigh_racks and weigh_racks_by_generation work in, kept from one
// call to the next so that a call on one thread allocates nothing once they
// have grown: the tiles left to draw, none placed, the tiles from each letter
// on, the tally drawn, the rack, its tiles, and the counts of tallies
#[derive(Default)]
pub struct WeighScratch {
    avail: Vec<u8>,
    none_placed: Vec<u8>,
    cap: Vec<usize>,
    cur: Vec<u8>,
    rack: Vec<u8>,
    rack_tiles: Vec<u8>,
    ways: Vec<usize>,
}

impl WeighScratch {
    #[inline(always)]
    pub fn new() -> Self {
        Self::default()
    }

    #[inline]
    fn begin(&mut self, pool: &[u8], tiles: &MoveTiles<'_>, rack_size: usize) -> usize {
        let num_letters = pool.len();
        self.avail.clear();
        self.avail.extend_from_slice(pool);
        self.none_placed.clear();
        self.none_placed.resize(num_letters, 0);
        let left = match *tiles {
            MoveTiles::Placed(placed) => {
                for (a, &p) in self.avail.iter_mut().zip(placed) {
                    *a -= p;
                }
                rack_size - placed.iter().map(|&c| c as usize).sum::<usize>()
            }
            MoveTiles::Exchanged(_) => rack_size,
        };
        self.cap.clear();
        self.cap.resize(num_letters + 1, 0);
        for t in (0..num_letters).rev() {
            self.cap[t] = self.cap[t + 1] + self.avail[t] as usize;
        }
        self.cur.clear();
        self.cur.resize(num_letters, 0);
        self.rack.clear();
        self.rack.resize(num_letters, 0);
        left
    }
}

// weigh_racks cuts the racks into at least this many chunks a thread, so that
// the threads end close together though the chunks are uneven
const CHUNKS_A_THREAD: usize = 8;

// every full rack the pool can deal that could have made the move, to f(rank of
// the rack, ln of its chance to be dealt, the equity it gives up by making the
// move, rank of the leave it keeps). The gap is the rack's best equity on the
// board minus the move's: the move's value plus what it keeps, for a placed
// move; the best that exchanging so many tiles keeps, for an exchange (the
// first such leave in the lattice's order on a tie; nothing, for an exchange of
// more tiles than a rack holds). The best is over the moves that are not a
// pass, and the move itself: a hasty mover never passes while it can move, a
// simmer may, so a pass gives up the rack's best move over keeping it all, and
// nothing when the rack had no move. Weighed on this many threads when they are
// many, in chunks, and handed to f in the one thread's order either way.
#[inline]
pub fn weigh_racks(
    is: &InferSheet<'_>,
    pool: &[u8],
    tiles: MoveTiles<'_>,
    move_value: i32,
    threads: usize,
    scratch: &mut WeighScratch,
    f: &mut impl FnMut(u32, f64, i32, u32),
) {
    let threads = if cfg!(target_family = "wasm") {
        1
    } else {
        threads
    };
    let lat = is.lat;
    let num_letters = lat.num_letters();
    let rack_size = lat.rack_size();
    let pool_size: usize = pool.iter().map(|&c| c as usize).sum();
    let ln_deals = is.ln_binom.ln_binom(pool_size, rack_size);
    let left = scratch.begin(pool, &tiles, rack_size);
    let WeighScratch {
        avail,
        none_placed,
        cap,
        cur,
        rack,
        ways,
        ..
    } = scratch;
    let (avail, cap) = (&avail[..], &cap[..]);
    let placed = match tiles {
        MoveTiles::Placed(placed) => placed,
        MoveTiles::Exchanged(_) => &none_placed[..],
    };
    let passed = matches!(tiles, MoveTiles::Placed(placed) if placed.iter().all(|&c| c == 0));
    // one rack, from the tiles it drew (rack is its scratch), to what f takes
    let weigh = |drawn: &[u8], rack: &mut [u8]| -> (u32, f64, i32, u32) {
        for (r, (&d, &p)) in rack.iter_mut().zip(drawn.iter().zip(placed)) {
            *r = d + p;
        }
        let rank = lat.rank_in_lattice(rack);
        let mut best = census::best_equity_of(lat, is.sheet, is.leave, rack).raw();
        if passed {
            best = best.max(is.leave[rank as usize]);
        }
        let (value, kept) = match tiles {
            MoveTiles::Placed(_) => {
                let kept = lat.rank_in_lattice(drawn);
                (move_value + is.leave[kept as usize], kept)
            }
            MoveTiles::Exchanged(n) => {
                let (value, kept) =
                    census::best_keep_in_lattice(lat, is.leave, rack, rack_size.saturating_sub(n));
                (value.raw(), kept)
            }
        };
        let ln_prior = rack
            .iter()
            .zip(pool)
            .map(|(&r, &u)| is.ln_binom.ln_binom(u as usize, r as usize))
            .sum::<f64>()
            - ln_deals;
        (rank, ln_prior, best - value, kept)
    };
    if threads <= 1 || count_tallies_of_size_in(avail, left, ways) < WEIGH_ACROSS_THREADS_FROM {
        each_tally_of_size(avail, cap, 0, left, cur, &mut |drawn| {
            let (rank, ln_prior, gap, kept) = weigh(drawn, rack);
            f(rank, ln_prior, gap, kept);
        });
        return;
    }
    let mut k = 1;
    let mut prefixes: Vec<(Vec<u8>, usize)> = Vec::new();
    loop {
        prefixes.clear();
        let mut cur = vec![0u8; num_letters];
        each_prefix_of_size(avail, cap, 0, k, left, &mut cur, &mut |prefix, left| {
            prefixes.push((prefix.to_vec(), left));
        });
        if prefixes.len() >= threads * CHUNKS_A_THREAD || k >= num_letters {
            break;
        }
        k += 1;
    }
    let next = std::sync::atomic::AtomicUsize::new(0);
    let mut chunks = std::thread::scope(|s| {
        let handles = (0..threads)
            .map(|_| {
                s.spawn(|| {
                    let mut cur = vec![0u8; num_letters];
                    let mut rack = vec![0u8; num_letters];
                    let mut mine = Vec::new();
                    loop {
                        let i = next.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                        let Some((prefix, left)) = prefixes.get(i) else {
                            break;
                        };
                        cur[..k].copy_from_slice(prefix);
                        let mut chunk = Vec::new();
                        each_tally_of_size(avail, cap, k, *left, &mut cur, &mut |drawn| {
                            chunk.push(weigh(drawn, &mut rack));
                        });
                        mine.push((i, chunk));
                    }
                    mine
                })
            })
            .collect::<Vec<_>>();
        handles
            .into_iter()
            .flat_map(|handle| handle.join().unwrap())
            .collect::<Vec<_>>()
    });
    chunks.sort_unstable_by_key(|&(i, _)| i);
    for (rank, ln_prior, gap, kept) in chunks.into_iter().flat_map(|(_, chunk)| chunk) {
        f(rank, ln_prior, gap, kept);
    }
}

// the most racks an inference generates one by one instead of building the
// census sheet: one for when the anagram tables are loaded, one for the lean
// arm, whose racks each take more than twice as long
const GENERATE_EACH_RACK_UPTO: usize = 2000;
const GENERATE_EACH_RACK_UPTO_LEAN: usize = 800;

// weigh_racks, single-threaded, each rack's best equity found by generating the
// rack's own moves on the board instead of reading the census sheet (is's sheet
// is not read): when the move leaves few tiles unseen there are few racks, and
// generating each costs less than building the sheet. As there, the best is
// over the moves that are not a pass; a rack with none (it can neither play nor
// exchange) is valued as the sheet values it, the least it reads: the best
// leave it could keep, from an entry no play reached. Made lean, by the lean
// arm alone, so that a build that runs only that arm (the wasm) need not carry
// the other.
#[inline]
pub fn weigh_racks_by_generation<
    const LEAN: bool,
    N: kwg::Node,
    L: kwg::Node,
    G: movegen::LaneBits,
    const JUMPS: bool,
    const ANY_SIDE: bool,
>(
    move_generator: &mut movegen::KurniaMoveGenerator<G, JUMPS, ANY_SIDE>,
    board_snapshot: &movegen::BoardSnapshot<'_, N, L>,
    is: &InferSheet<'_>,
    pool: &[u8],
    ObservedMove {
        tiles,
        value: move_value,
    }: ObservedMove<'_>,
    scratch: &mut WeighScratch,
    f: &mut impl FnMut(u32, f64, i32, u32),
) -> error::Returns<()> {
    let (lat, leave, ln_binom) = (is.lat, is.leave, is.ln_binom);
    let rack_size = lat.rack_size();
    let pool_size: usize = pool.iter().map(|&c| c as usize).sum();
    let ln_deals = ln_binom.ln_binom(pool_size, rack_size);
    let left = scratch.begin(pool, &tiles, rack_size);
    let WeighScratch {
        avail,
        none_placed,
        cap,
        cur,
        rack,
        rack_tiles,
        ..
    } = scratch;
    let placed = match tiles {
        MoveTiles::Placed(placed) => placed,
        MoveTiles::Exchanged(_) => &none_placed[..],
    };
    let passed = matches!(tiles, MoveTiles::Placed(placed) if placed.iter().all(|&c| c == 0));
    let mut weigh = |drawn: &[u8]| -> error::Returns<()> {
        for (r, (&d, &p)) in rack.iter_mut().zip(drawn.iter().zip(placed)) {
            *r = d + p;
        }
        let rank = lat.rank_in_lattice(rack);
        rack_tiles.clear();
        rack_tiles.extend(
            (0u8..)
                .zip(rack.iter())
                .flat_map(|(tile, &count)| std::iter::repeat_n(tile, count as usize)),
        );
        let params = movegen::GenMovesParams {
            board_snapshot,
            rack: rack_tiles,
            max_gen: 1,
            num_exchanges_by_this_player: 0,
            pass_policy: movegen::PassPolicy::OnlyWhenForced,
            dynamic_leaves: None,
        };
        if LEAN {
            move_generator.gen_moves_unfiltered_lean(&params)?;
        } else {
            move_generator.gen_moves_unfiltered(&params)?;
        }
        let top = &move_generator.plays[0];
        let mut best = match &top.play {
            movegen::Play::Exchange { tiles } if tiles.is_empty() => {
                i32::MIN / 4
                    + (0..=rack_tiles.len())
                        .map(|keep| census::best_keep_in_lattice(lat, leave, rack, keep).0)
                        .max()
                        .unwrap_or(census::UNPLAYABLE)
                        .raw()
            }
            _ => top.equity.raw(),
        };
        if passed {
            best = best.max(leave[rank as usize]);
        }
        let (value, kept) = match tiles {
            MoveTiles::Placed(_) => {
                let kept = lat.rank_in_lattice(drawn);
                (move_value + leave[kept as usize], kept)
            }
            MoveTiles::Exchanged(n) => {
                let (value, kept) =
                    census::best_keep_in_lattice(lat, leave, rack, rack_size.saturating_sub(n));
                (value.raw(), kept)
            }
        };
        let ln_prior = rack
            .iter()
            .zip(pool)
            .map(|(&r, &u)| ln_binom.ln_binom(u as usize, r as usize))
            .sum::<f64>()
            - ln_deals;
        f(rank, ln_prior, best - value, kept);
        Ok(())
    };
    let mut found = Ok(());
    each_tally_of_size(avail, cap, 0, left, cur, &mut |drawn| {
        if found.is_ok() {
            found = weigh(drawn);
        }
    });
    found
}

// the census sheet for the pool on this board: every multiset of the pool, the
// best it can score played, floored at 0 when it could be exchanged instead.
// Playing nothing is no entry at all: weigh_racks weighs a pass itself.
#[inline]
pub fn infer_sheet_for<
    N: kwg::Node,
    L: kwg::Node,
    G: movegen::LaneBits,
    const JUMPS: bool,
    const ANY_SIDE: bool,
>(
    move_generator: &mut movegen::KurniaMoveGenerator<G, JUMPS, ANY_SIDE>,
    board_tiles: &[u8],
    tables: SpellTables<'_, N, L>,
    pool: &[u8],
    can_exchange: bool,
    scratch: (&mut Vec<u8>, &mut Vec<(u8, i32)>),
    sheet: &mut Vec<i32>,
) -> error::Returns<()> {
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
            num_blanks_eff: usize::from(pool.first().copied().unwrap_or(0)).min(rack_size),
            rack_size,
            phase: BoardPhase::of_board(board_tiles, game_config),
        },
        scratch.0,
        scratch.1,
        sheet,
    )?;
    sheet[lat.size_start(0)] = i32::MIN / 4;
    Ok(())
}

// the tiles a move places, as a tally (a blank as tile 0), or how many it
// exchanges; and its value as the census sheet values it: its score, with the
// opening's adjustment
#[inline]
pub fn move_tiles_and_value<N: kwg::Node, L: kwg::Node>(
    board_snapshot: &movegen::BoardSnapshot<'_, N, L>,
    play: &movegen::Play,
    placed: &mut [u8],
) -> error::Returns<(Option<usize>, i32)> {
    placed.iter_mut().for_each(|c| *c = 0);
    Ok(match play {
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
                    + opening_adjustment(phase, game_config, *down, *idx, word)?,
            )
        }
    })
}

// fuzz, given in points, in the equity's own units. 0 is the exact limit, so
// only NaN and a fuzz below 0 are refused.
#[inline]
pub fn equity_fuzz(fuzz: f64) -> error::Returns<f64> {
    if fuzz.is_nan() || fuzz < 0.0 {
        return_error!(format!("fuzz {fuzz} is not 0 or more"));
    }
    Ok(fuzz * f64::from(equity::SCALE))
}

// for a rack that gives up gap, where g0 is the smallest gap of the racks that
// make the move: the log of the factor on its chance to be dealt, which is
// -(gap - g0) / fuzz, a gap below 0 counting as 0. At fuzz 0, its limit: 0 at
// the smallest gap, and minus infinity above it. The move may be no rack's
// best, so g0 may be above 0: fuzz 0 then keeps the racks closest to their
// best, not none; above fuzz 0, g0 cancels when the weights are normalized
#[inline(always)]
pub fn fuzz_ln_weight(gap: i32, g0: i32, fuzz: f64) -> f64 {
    let d = gap.max(0) - g0;
    if fuzz == 0.0 {
        if d == 0 { 0.0 } else { f64::NEG_INFINITY }
    } else {
        -f64::from(d) / fuzz
    }
}

// each rack (rank, ln of its chance to be dealt, gap) as (weight, chance to be
// dealt, rank, gap); each weight is taken against the smallest gap, so none
// underflows to 0
#[inline]
pub fn weigh_by_fuzz(racks: &[(u32, f64, i32)], fuzz: f64) -> Vec<(f64, f64, u32, i32)> {
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

// each leave the racks (the leave each kept, its gap, and the log of its chance
// to be dealt) kept, in the lattice's order, with the sum of their weights as
// weigh_by_fuzz gives them, added in the order the racks come; a rack of weight
// 0 (fuzz 0 keeps only the racks closest to their best) adds nothing
#[inline]
fn weights_by_leave(racks: &[(u32, i32, f64)], fuzz: f64, weighed: &mut Vec<(u32, usize, f64)>) {
    let g0 = racks
        .iter()
        .map(|&(_, gap, _)| gap.max(0))
        .min()
        .unwrap_or(0);
    weighed.clear();
    weighed.extend(
        (racks.iter().enumerate()).filter_map(|(at, &(kept, gap, ln_prior))| {
            let weight = ln_prior.exp() * fuzz_ln_weight(gap, g0, fuzz).exp();
            (weight > 0.0).then_some((kept, at, weight))
        }),
    );
    weighed.sort_unstable_by_key(|&(kept, at, _)| (kept, at));
    weighed.dedup_by(|later, first| {
        let same = later.0 == first.0;
        if same {
            first.2 += later.2;
        }
        same
    });
}

// a leave's rank, where its racks begin, and its weight
type LeaveWeight = (u32, usize, f64);

// two lists of leaves by rank (weights_by_leave's or weights_by_keep's), each
// taken as a share of its own total, the second's mix of the whole
#[inline]
fn mix_weights((a, b): (&[LeaveWeight], &[LeaveWeight]), mix: f64, out: &mut Vec<LeaveWeight>) {
    let total = |list: &[LeaveWeight]| list.iter().map(|&(_, _, w)| w).sum::<f64>();
    let (ta, tb) = (total(a), total(b));
    out.clear();
    let (mut i, mut j) = (0, 0);
    while i < a.len() || j < b.len() {
        let (rank, at, w) = match (a.get(i), b.get(j)) {
            (Some(&(ra, at, wa)), Some(&(rb, _, wb))) if ra == rb => {
                i += 1;
                j += 1;
                (ra, at, (1.0 - mix) * wa / ta + mix * wb / tb)
            }
            (Some(&(ra, at, wa)), Some(&(rb, _, _))) if ra < rb => {
                i += 1;
                (ra, at, (1.0 - mix) * wa / ta)
            }
            (Some(&(ra, at, wa)), None) => {
                i += 1;
                (ra, at, (1.0 - mix) * wa / ta)
            }
            (_, Some(&(rb, at, wb))) => {
                j += 1;
                (rb, at, mix * wb / tb)
            }
            (None, None) => unreachable!(),
        };
        out.push((rank, at, w));
    }
}

// every keep of this many tiles the rack (a tally) holds, to f(rank of the
// keep, its gap): the rack's best equity less what the keep is worth
#[inline]
fn each_keep(
    lat: &census::MultisetLattice,
    leave: &[i32],
    (rack, cur, cap): (&[u8], &mut [u8], &mut [usize]),
    best: i32,
    keep: usize,
    f: &mut impl FnMut(u32, i32),
) {
    for t in (0..rack.len()).rev() {
        cap[t] = cap[t + 1] + rack[t] as usize;
    }
    each_tally_of_size(rack, cap, 0, keep, cur, &mut |kept| {
        let kept = lat.rank_in_lattice(kept);
        f(kept, best - leave[kept as usize]);
    });
}

// as weights_by_leave, for an exchange keeping keep tiles: each rack (its
// rank, its best equity, and the log of its chance before the move) weighed at
// every keep it holds, each by its own gap, summed by keep in the order the
// racks come in a table of the leaves of that size, so that no list holds a
// rack's every keep: (leave, 0, weight)
#[inline]
fn weights_by_keep(
    lat: &census::MultisetLattice,
    leave: &[i32],
    racks: &[(u32, i32, f64)],
    (keep, g0, fuzz): (usize, i32, f64),
    scratch: &mut KeepScratch,
    weighed: &mut Vec<(u32, usize, f64)>,
) -> error::Returns<()> {
    let KeepScratch {
        rack,
        keep: cur,
        cap,
        by_keep,
    } = scratch;
    let num_letters = lat.num_letters();
    rack.clear();
    rack.resize(num_letters, 0);
    cur.clear();
    cur.resize(num_letters, 0);
    cap.clear();
    cap.resize(num_letters + 1, 0);
    let start = lat.size_start(keep);
    let end = lat.size_start(keep + 1);
    let ranks = u32::try_from(start)?..u32::try_from(end)?;
    by_keep.clear();
    by_keep.resize(end - start, 0.0);
    for &(rank, best, ln_prior) in racks {
        lat.unrank_in_lattice(rank as usize, rack);
        let prior = ln_prior.exp();
        each_keep(
            lat,
            leave,
            (rack, cur, cap),
            best,
            keep,
            &mut |kept, gap| {
                by_keep[kept as usize - start] += prior * fuzz_ln_weight(gap, g0, fuzz).exp();
            },
        );
    }
    weighed.clear();
    weighed.extend(
        ranks
            .zip(by_keep.iter())
            .filter(|&(_, &weight)| weight > 0.0)
            .map(|(kept, &weight)| (kept, 0, weight)),
    );
    Ok(())
}

// the multiset lattice and its leave table, built by one inference and reused
// by the next, the logs of the binomials up to the game's tiles, and the
// scratch space an inference works in: the pool, the tiles the move placed,
// the tiles left after them, the racks weighed, the leaves they kept with
// their weights, a leave as a tally and as tiles, weigh_racks's own, and the
// ways to draw each letter (a reply's, reusing the others for its tallies,
// what the mover holds, and the board after the move)
pub struct InferTables {
    lat: census::MultisetLattice,
    leave: Vec<i32>,
    ln_binom: LnBinom,
    sheet: Vec<i32>,
    movegen_rack: Vec<u8>,
    blank_deltas: Vec<(u8, i32)>,
    pool: Vec<u8>,
    placed: Vec<u8>,
    avail: Vec<u8>,
    racks: Vec<(u32, i32, f64)>,
    weighed: Vec<(u32, usize, f64)>,
    mix: f64, // the share of an inference weighed by the chance to be dealt alone
    mixed: (Vec<LeaveWeight>, Vec<LeaveWeight>, Carried), // that weighing, both mixed, its carry
    kept: Vec<u8>,
    tiles: Vec<u8>,
    weigh: WeighScratch,
    ways: Vec<u64>,
    keeps: KeepScratch,
    held: HeldScratch,
    carry: CarryScratch,
}

// the scratch an exchange's keeps are summed in: a rack, a keep, the tiles
// from each letter on (as each_keep walks them), and each keep's weight by its
// place among the leaves of its size
#[derive(Default)]
struct KeepScratch {
    rack: Vec<u8>,
    keep: Vec<u8>,
    cap: Vec<usize>,
    by_keep: Vec<f64>,
}

impl InferTables {
    // the game config must have passed supports(Op::Inference) at init
    pub fn new<L: kwg::Node>(
        game_config: &game_config::GameConfig,
        klv: &klv::Klv<L>,
    ) -> error::Returns<Self> {
        let lat = census::MultisetLattice::new(
            game_config.alphabet().len() as usize,
            game_config.rack_size() as usize,
        )?;
        let mut leave = lat.dense(0i32)?;
        census::fill_lattice_klv_leaves(&lat, &mut leave, klv);
        Ok(Self {
            lat,
            leave,
            ln_binom: LnBinom::new(game_config.alphabet().num_tiles() as usize),
            sheet: Vec::new(),
            movegen_rack: Vec::new(),
            blank_deltas: Vec::new(),
            pool: Vec::new(),
            placed: Vec::new(),
            avail: Vec::new(),
            racks: Vec::new(),
            weighed: Vec::new(),
            mix: 0.0,
            mixed: Default::default(),
            kept: Vec::new(),
            tiles: Vec::new(),
            weigh: WeighScratch::new(),
            ways: Vec::new(),
            keeps: KeepScratch::default(),
            held: HeldScratch::default(),
            carry: CarryScratch::default(),
        })
    }

    // 0 until set
    #[inline]
    pub fn set_prior_mix(&mut self, mix: f64) {
        self.mix = mix;
    }
}

// leaves with their weights, packed for lists of millions, with no allocation
// for each leave: every leave's tiles one after another, where each one ends,
// the weights in the same order, and their running sums, which a weighted draw
// searches. No weight is below 0, past f64's finite range, or not a number
#[derive(Clone, Default)]
pub struct PackedLeaves {
    tiles: Vec<u8>,
    ends: Vec<u32>,
    weights: Vec<f64>,
    sums: Vec<f64>,
}

impl PackedLeaves {
    #[inline]
    pub fn new() -> Self {
        Self::default()
    }

    // room for this many leaves of these many tiles in all
    #[inline]
    pub fn with_capacity(leaves: usize, tiles: usize) -> Self {
        Self {
            tiles: Vec::with_capacity(tiles),
            ends: Vec::with_capacity(leaves),
            weights: Vec::with_capacity(leaves),
            sums: Vec::with_capacity(leaves),
        }
    }

    // room for this many more leaves of these many more tiles in all, or the
    // allocator's refusal
    #[inline]
    pub fn try_reserve(
        &mut self,
        leaves: usize,
        tiles: usize,
    ) -> Result<(), std::collections::TryReserveError> {
        self.tiles.try_reserve_exact(tiles)?;
        self.ends.try_reserve_exact(leaves)?;
        self.weights.try_reserve_exact(leaves)?;
        self.sums.try_reserve_exact(leaves)
    }

    // no leaves, the memory kept
    #[inline]
    pub fn clear(&mut self) {
        self.tiles.clear();
        self.ends.clear();
        self.weights.clear();
        self.sums.clear();
    }

    // a leave and its weight after the others; refused for a weight below 0,
    // not finite, or not a number, or past u32's count of tiles in all
    #[inline]
    pub fn push(&mut self, tiles: &[u8], weight: f64) -> error::Returns<()> {
        if !(weight >= 0.0 && weight.is_finite()) {
            return_error!("a leave's weight must be finite and 0 or more".into());
        }
        let end = u32::try_from(self.tiles.len() + tiles.len())
            .map_err(|_| "more tiles than a packed list of leaves can count")?;
        let sum = self.total() + weight;
        if !sum.is_finite() {
            return_error!("the leaves' weights must sum to a finite total".into());
        }
        self.tiles.extend_from_slice(tiles);
        self.ends.push(end);
        self.weights.push(weight);
        self.sums.push(sum);
        Ok(())
    }

    #[inline(always)]
    pub fn len(&self) -> usize {
        self.ends.len()
    }

    #[inline(always)]
    pub fn is_empty(&self) -> bool {
        self.ends.is_empty()
    }

    // the leave at i, in the order pushed
    #[inline(always)]
    pub fn leave(&self, i: usize) -> &[u8] {
        let start = if i == 0 { 0 } else { self.ends[i - 1] as usize };
        &self.tiles[start..self.ends[i] as usize]
    }

    #[inline(always)]
    pub fn weight(&self, i: usize) -> f64 {
        self.weights[i]
    }

    // the weights made to sum to 1, each divided by their sum taken in order,
    // and their running sums with them; none changed while they sum to 0
    #[inline]
    pub fn normalize(&mut self) {
        let total = self.weights.iter().sum::<f64>();
        if total > 0.0 {
            let mut sum = 0.0;
            for (weight, running) in self.weights.iter_mut().zip(&mut self.sums) {
                *weight /= total;
                sum += *weight;
                *running = sum;
            }
        }
    }

    // the weights' sum, as the running sums end
    #[inline(always)]
    pub fn total(&self) -> f64 {
        self.sums.last().copied().unwrap_or(0.0)
    }

    // the leave a weighted draw at this point (from 0 up to the total) picks:
    // the first whose running sum passes it, so never a leave of weight 0; no
    // tiles from a list of none
    #[inline]
    pub fn pick(&self, at: f64) -> &[u8] {
        let sums = &self.sums;
        if sums.is_empty() {
            return &[];
        }
        self.leave(sums.partition_point(|&sum| sum <= at).min(sums.len() - 1))
    }
}

// what the watcher knew of the mover's rack before its move: nothing, so that
// it is any rack the unseen tiles could make, or what the inference of the
// mover's last move carried, with the tiles the watcher has put back in the
// bag since (those it exchanged)
#[derive(Clone, Copy)]
pub enum Held<'a> {
    Any,
    Kept(&'a Carried, &'a [u8]),
}

// what the inference of a move carries to the mover's next: the tiles unseen
// just after the move, as a tally, and a weight for each set of size tiles, by
// its place among those sets, the weights summing to 1, a rack's chance then
// being, over each set it holds, its weight times the chance the rest is drawn
// from the pool less it. A placement or a pass carries the leaves the mover
// may have kept; an exchange, each rack the mover may hold after its draw,
// which came from a bag without the tiles thrown back. Nothing is carried
// where nothing was weighed
#[derive(Clone, Default)]
pub struct Carried {
    pool: Vec<u8>,
    size: usize,
    weights: Vec<f64>,
}

impl Carried {
    #[inline(always)]
    pub fn is_empty(&self) -> bool {
        self.weights.is_empty()
    }

    #[inline(always)]
    pub fn clear(&mut self) {
        self.pool.clear();
        self.size = 0;
        self.weights.clear();
    }
}

// the scratch a carried prior is taken in: what the watcher has drawn since
// the mover's last move, a rack, its letters with their counts, a set's count
// of each, and the terms of the sum
#[derive(Default)]
struct HeldScratch {
    drawn: Vec<u8>,
    rack: Vec<u8>,
    items: Vec<(u8, u8)>,
    kept: Vec<u8>,
    terms: Vec<f64>,
}

// set the scratch up for the racks to be weighed from what was carried: false,
// leaving it unread, where nothing was carried or what is unseen now could not
// have come from the carried pool (the watcher's draws since are that pool
// with the tiles put back, less what is unseen now), so that each rack is
// weighed as any the unseen tiles could make
#[inline]
fn hold((carried, put_back): (&Carried, &[u8]), unseen: &[u8], held: &mut HeldScratch) -> bool {
    if carried.weights.is_empty() || carried.pool.len() != unseen.len() {
        return false;
    }
    held.drawn.clear();
    held.drawn.extend_from_slice(&carried.pool);
    for &tile in put_back {
        match held.drawn.get_mut(tile as usize) {
            Some(count) => *count += 1,
            None => return false,
        }
    }
    for (drawn, &now) in held.drawn.iter_mut().zip(unseen) {
        match drawn.checked_sub(now) {
            Some(left) => *drawn = left,
            None => return false,
        }
    }
    held.rack.clear();
    held.rack.resize(unseen.len(), 0);
    true
}

// the log of the chance the mover holds this rack (a tally, the lattice's rank
// of it given): over each carried set of size it holds, its weight times the
// chance the rest was drawn from the carried pool less that set, all times the
// chance the watcher then drew what it drew from what the rack and those draws
// left; minus infinity where none could be
#[inline]
fn held_ln_prior(
    lat: &census::MultisetLattice,
    ln_binom: &LnBinom,
    carried: &Carried,
    held: &mut HeldScratch,
    rank: u32,
) -> f64 {
    let HeldScratch {
        drawn,
        rack,
        items,
        kept,
        terms,
    } = held;
    let (pool, size) = (&carried.pool, carried.size);
    lat.unrank_in_lattice(rank as usize, rack);
    let pool_size: usize = pool.iter().map(|&c| c as usize).sum();
    let rack_size: usize = rack.iter().map(|&c| c as usize).sum();
    let drawn_size: usize = drawn.iter().map(|&c| c as usize).sum();
    if rack_size < size || rack_size + drawn_size > pool_size {
        return f64::NEG_INFINITY;
    }
    let mut ln_drawn = -ln_binom.ln_binom(pool_size - rack_size, drawn_size);
    for ((&r, &d), &u) in rack.iter().zip(drawn.iter()).zip(pool.iter()) {
        if r + d > u {
            return f64::NEG_INFINITY;
        }
        if d > 0 {
            ln_drawn += ln_binom.ln_binom((u - r) as usize, d as usize);
        }
    }
    items.clear();
    items.extend(
        (0u8..)
            .zip(rack.iter())
            .filter(|&(_, &count)| count > 0)
            .map(|(letter, &count)| (letter, count)),
    );
    let ln_draws = ln_binom.ln_binom(pool_size - size, rack_size - size);
    let start = lat.size_start(size);
    kept.clear();
    kept.resize(items.len(), 0);
    terms.clear();
    each_sub_tally(items, 0, size, kept, &mut |kept| {
        let weight = carried.weights[lat.rank_sparse_in_lattice(
            size,
            (items.iter().zip(kept))
                .filter(|&(_, &k)| k > 0)
                .map(|(&(letter, _), &k)| (letter, k)),
        ) as usize
            - start];
        if weight <= 0.0 {
            return;
        }
        let ln_drew = (items.iter().zip(kept))
            .map(|(&(letter, r), &k)| {
                ln_binom.ln_binom((pool[letter as usize] - k) as usize, (r - k) as usize)
            })
            .sum::<f64>();
        terms.push(weight.ln() + ln_drew - ln_draws);
    });
    let top = terms.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    if top == f64::NEG_INFINITY {
        return top;
    }
    top + terms
        .iter()
        .map(|&term| (term - top).exp())
        .sum::<f64>()
        .ln()
        + ln_drawn
}

// every sub-tally of left tiles of the items (letter, count) from the one at i
// on, to f as the count kept of each item, in the items' order
#[inline]
fn each_sub_tally(
    items: &[(u8, u8)],
    i: usize,
    left: usize,
    kept: &mut [u8],
    f: &mut impl FnMut(&[u8]),
) {
    if i == items.len() {
        if left == 0 {
            f(kept);
        }
        return;
    }
    let rest = items[i + 1..]
        .iter()
        .map(|&(_, c)| c as usize)
        .sum::<usize>();
    for c in left.saturating_sub(rest)..=(items[i].1 as usize).min(left) {
        kept[i] = c as u8;
        each_sub_tally(items, i + 1, left - c, kept, f);
    }
    kept[i] = 0;
}

// every keep of left tiles inside a set of a rack's tiles, from item i of the
// rack's items (letter, count) on, the set given as its count of each item
// with what the counts after each come to, in each_sub_tally's order: f(the
// keep's place by the strides, the ways to pick the rest of the set out of the
// rack less the keep). Made an item at a time, the ways multiplied in the
// items' order, as a product of them all would be
#[inline]
fn each_keep_in(
    (items, set, after): (&[(u8, u8)], &[u8], &[usize]),
    (strides, binoms, ways_of): (&[usize], &[f64], usize),
    i: usize,
    left: usize,
    (place, ways): (usize, f64),
    f: &mut impl FnMut(usize, f64),
) {
    if i == items.len() {
        if left == 0 {
            f(place, ways);
        }
        return;
    }
    let (r, w) = (items[i].1 as usize, set[i] as usize);
    for k in left.saturating_sub(after[i + 1])..=w.min(left) {
        each_keep_in(
            (items, set, after),
            (strides, binoms, ways_of),
            i + 1,
            left - k,
            (
                place + k * strides[i],
                ways * binoms[(r - k) * ways_of + (w - k)],
            ),
            f,
        );
    }
}

// the scratch an exchange's carry is made in: the ways to pick a few of a few,
// how many tiles the pool holds from each letter on, and a tally
#[derive(Default)]
struct CarryScratch {
    binoms: Vec<f64>,
    cap: Vec<usize>,
    cur: Vec<u8>,
}

// what one chunk of an exchange's racks adds to its carry (carry_exchange says
// what): each set's signed sum by its rank, in rank order, each sum's terms
// added in the order of the racks and of each rack's sets; and the sum of the
// chunk's keeps' weights
#[inline]
fn carry_chunk(
    lat: &census::MultisetLattice,
    (leave, binoms): (&[i32], &[f64]),
    racks: &[(u32, i32, f64)],
    (keep, g0, fuzz): (usize, i32, f64),
    parts: &mut Vec<(u32, f64)>,
    spare: &mut Vec<(u32, f64)>,
) -> f64 {
    let rack_size = lat.rack_size();
    let ways_of = rack_size + 1;
    let mut rack = vec![0u8; lat.num_letters()];
    let (mut items, mut strides, mut by_place) = (Vec::new(), Vec::new(), Vec::new());
    let (mut set, mut after, mut kept) = (Vec::new(), Vec::new(), Vec::new());
    let mut total = 0.0;
    parts.clear();
    for &(rank, best, ln_prior) in racks {
        lat.unrank_in_lattice(rank as usize, &mut rack);
        items.clear();
        items.extend(
            (0u8..)
                .zip(rack.iter())
                .filter(|&(_, &count)| count > 0)
                .map(|(letter, &count)| (letter, count)),
        );
        strides.clear();
        let mut places = 1;
        for &(_, count) in items.iter() {
            strides.push(places);
            places *= count as usize + 1;
        }
        by_place.clear();
        by_place.resize(places, 0.0);
        let place = |counts: &[u8]| -> usize {
            (counts.iter().zip(strides.iter()))
                .map(|(&count, &stride)| count as usize * stride)
                .sum()
        };
        let prior = ln_prior.exp();
        kept.clear();
        kept.resize(items.len(), 0);
        each_sub_tally(&items, 0, keep, &mut kept, &mut |kept| {
            let rank = lat.rank_sparse_in_lattice(
                keep,
                (items.iter().zip(kept))
                    .filter(|&(_, &k)| k > 0)
                    .map(|(&(letter, _), &k)| (letter, k)),
            );
            let weight = prior * fuzz_ln_weight(best - leave[rank as usize], g0, fuzz).exp();
            by_place[place(kept)] = weight;
            total += weight;
        });
        set.clear();
        set.resize(items.len(), 0);
        for size in keep..=rack_size {
            let away = (size - keep) % 2 == 1;
            each_sub_tally(&items, 0, size, &mut set, &mut |set| {
                after.clear();
                after.resize(set.len() + 1, 0);
                for i in (0..set.len()).rev() {
                    after[i] = after[i + 1] + set[i] as usize;
                }
                let mut sum = 0.0;
                each_keep_in(
                    (&items, set, &after),
                    (&strides, binoms, ways_of),
                    0,
                    keep,
                    (0, 1.0),
                    &mut |place, ways| sum += by_place[place] * ways,
                );
                if sum != 0.0 {
                    let rank = lat.rank_sparse_in_lattice(
                        size,
                        (items.iter().zip(set))
                            .filter(|&(_, &w)| w > 0)
                            .map(|(&(letter, _), &w)| (letter, w)),
                    );
                    parts.push((rank, if away { -sum } else { sum }));
                }
            });
        }
    }
    sort_by_rank(lat, keep, parts, spare);
    parts.dedup_by(|later, earlier| {
        if later.0 == earlier.0 {
            earlier.1 += later.1;
            true
        } else {
            false
        }
    });
    total
}

// the parts in rank order, those of one rank in the order they came
#[inline]
fn sort_by_rank(
    lat: &census::MultisetLattice,
    keep: usize,
    parts: &mut Vec<(u32, f64)>,
    spare: &mut Vec<(u32, f64)>,
) {
    let start = lat.size_start(keep);
    let span = lat.size_start(lat.rack_size() + 1) - start;
    let bits = usize::BITS - span.saturating_sub(1).leading_zeros();
    let low = bits / 2;
    let mut counts = Vec::new();
    for (shift, width) in [(0, low), (low, bits - low)] {
        let mask = (1usize << width) - 1;
        counts.clear();
        counts.resize(1 << width, 0usize);
        for &(rank, _) in parts.iter() {
            counts[((rank as usize - start) >> shift) & mask] += 1;
        }
        let mut at = 0;
        for count in &mut counts {
            let n = *count;
            *count = at;
            at += n;
        }
        spare.resize(parts.len(), (0, 0.0));
        for &part in parts.iter() {
            let next = &mut counts[((part.0 as usize - start) >> shift) & mask];
            spare[*next] = part;
            *next += 1;
        }
        std::mem::swap(parts, spare);
    }
}

// a round's parts (carry_chunk's, in the chunks' order) added into weights,
// whose first is the rank start: on the threads, each a span of the ranks of
// its own, so that every weight takes its values in the parts' order on any
// number of threads
#[inline]
fn add_parts(weights: &mut [f64], start: usize, parts: &[Vec<(u32, f64)>], threads: usize) {
    if threads <= 1 || parts.len() <= 1 {
        for part in parts {
            for &(rank, value) in part {
                weights[rank as usize - start] += value;
            }
        }
        return;
    }
    let span = weights.len().div_ceil(threads);
    std::thread::scope(|s| {
        for (lo, slice) in (start..).step_by(span).zip(weights.chunks_mut(span)) {
            s.spawn(move || {
                let hi = lo + slice.len();
                for part in parts {
                    let from = part.partition_point(|&(rank, _)| (rank as usize) < lo);
                    let to = part.partition_point(|&(rank, _)| (rank as usize) < hi);
                    for &(rank, value) in &part[from..to] {
                        slice[rank as usize - lo] += value;
                    }
                }
            });
        }
    });
}

// what an exchange keeping keep tiles carries, from the racks it weighed (as
// weights_by_keep takes them) and the tiles unseen before it: the chance of
// each rack after the draw, which came before the tiles thrown back went in.
// Each set from the keep's size to the rack's takes, over every rack holding
// it and every keep of that rack inside it, the keep's weight times the ways
// to pick the rest of the set out of the tiles thrown back, taken away where
// that rest is of an odd count, all times the ways to draw up a rack from the
// pool less the set over those to draw the thrown tiles' count from the bag;
// then each set is drawn up to a rack a tile at a time. Made on this many
// threads
#[inline]
fn carry_exchange(
    lat: &census::MultisetLattice,
    (leave, ln_binom): (&[i32], &LnBinom),
    racks: &[(u32, i32, f64)],
    (keep, g0, fuzz): (usize, i32, f64),
    (pool, threads): (&[u8], usize),
    scratch: &mut CarryScratch,
    carried: &mut Carried,
) {
    let threads = if cfg!(target_family = "wasm") {
        1
    } else {
        threads.max(1)
    };
    let CarryScratch { binoms, cap, cur } = scratch;
    let rack_size = lat.rack_size();
    let num_letters = lat.num_letters();
    let start = lat.size_start(keep);
    let weights = &mut carried.weights;
    weights.clear();
    weights.resize(lat.size_start(rack_size + 1) - start, 0.0);
    let ways_of = rack_size + 1;
    binoms.clear();
    binoms.extend((0..ways_of * ways_of).map(|i| binom(i / ways_of, i % ways_of) as f64));
    let binoms = &binoms[..];
    // the racks in chunks of as many as weigh_racks first spreads over
    // threads, each made into its parts on whichever thread, and the parts
    // added in the chunks' order, so that the carry is the same on any number
    // of threads
    let mut total = 0.0;
    let (mut lists, mut spares, mut sums) = (
        vec![Vec::new(); threads],
        vec![Vec::new(); threads],
        vec![0.0; threads],
    );
    let chunks = racks.chunks(WEIGH_ACROSS_THREADS_FROM).collect::<Vec<_>>();
    for round in chunks.chunks(threads) {
        if let [chunk] = round {
            sums[0] = carry_chunk(
                lat,
                (leave, binoms),
                chunk,
                (keep, g0, fuzz),
                &mut lists[0],
                &mut spares[0],
            );
        } else {
            std::thread::scope(|s| {
                let each = round.iter().zip(lists.iter_mut().zip(spares.iter_mut()));
                for ((chunk, (list, spare)), sum) in each.zip(sums.iter_mut()) {
                    s.spawn(move || {
                        // the lists grow on this thread's own stack: they lie
                        // side by side, and a push to one would take the cache
                        // line its neighbors' threads push through
                        let (mut taken, mut taken_spare) =
                            (std::mem::take(list), std::mem::take(spare));
                        *sum = carry_chunk(
                            lat,
                            (leave, binoms),
                            chunk,
                            (keep, g0, fuzz),
                            &mut taken,
                            &mut taken_spare,
                        );
                        (*list, *spare) = (taken, taken_spare);
                    });
                }
            });
        }
        add_parts(weights, start, &lists[..round.len()], threads);
        for sum in &sums[..round.len()] {
            total += sum;
        }
    }
    let pool_size: usize = pool.iter().map(|&c| c as usize).sum();
    let ln_bag = ln_binom.ln_binom(pool_size - rack_size, rack_size - keep);
    for size in keep..=rack_size {
        let scale = (ln_binom.ln_binom(pool_size - size, rack_size - size) - ln_bag).exp() / total;
        let (from, to) = (
            lat.size_start(size) - start,
            lat.size_start(size + 1) - start,
        );
        for weight in &mut weights[from..to] {
            *weight *= scale;
        }
    }
    // what reaches a set of one more tile is its own weight and, over each
    // tile it holds, what reached the set without it times the chance that
    // tile was drawn next; a size's sets on the threads, cut by the counts of
    // their first letters as weigh_racks cuts its racks, each set's sum its
    // own whichever thread makes it. The lattice orders a size's sets by the
    // count of each letter in turn, so the sets of a prefix are a span of
    // ranks, from the one with the rest of the tiles in the last letter to the
    // one with them in the letter after the prefix: each prefix writes its own
    // span in place, from the sizes below it, which are made
    cap.clear();
    cap.resize(num_letters + 1, 0);
    for t in (0..num_letters).rev() {
        cap[t] = cap[t + 1] + pool[t] as usize;
    }
    let cap = &cap[..];
    cur.clear();
    cur.resize(num_letters, 0);
    let mut prefixes: Vec<(Vec<u8>, usize)> = Vec::new();
    for size in keep + 1..=rack_size {
        let left = (pool_size - (size - 1)) as f64;
        let mut k = 1;
        loop {
            prefixes.clear();
            each_prefix_of_size(pool, cap, 0, k, size, cur, &mut |prefix, rest| {
                prefixes.push((prefix.to_vec(), rest));
            });
            if prefixes.len() >= threads * CHUNKS_A_THREAD || k >= num_letters {
                break;
            }
            k += 1;
        }
        let from = lat.size_start(size);
        let (below, above) = weights.split_at_mut(from - start);
        let below = &*below;
        let mut rest_of = &mut above[..lat.size_start(size + 1) - from];
        let mut at = from;
        let mut jobs = Vec::with_capacity(prefixes.len());
        for one in &prefixes {
            let (prefix, rest) = one;
            let held = (0u8..)
                .zip(prefix.iter().copied())
                .filter(|&(_, count)| count > 0);
            let first = lat.rank_sparse_in_lattice(size, held.clone()) as usize;
            let last = if *rest > 0 && k < num_letters {
                lat.rank_sparse_in_lattice(size, held.chain([(k as u8, *rest as u8)])) as usize
            } else {
                first
            };
            let (_, tail) = std::mem::take(&mut rest_of).split_at_mut(first - at);
            let (span, tail) = tail.split_at_mut(last + 1 - first);
            rest_of = tail;
            at = last + 1;
            jobs.push((one, first, span));
        }
        let walk = |job: &mut (&(Vec<u8>, usize), usize, &mut [f64])| {
            let ((prefix, rest), first, span) = job;
            let mut tally = vec![0u8; num_letters];
            tally[..k].copy_from_slice(prefix);
            let mut items = Vec::new();
            each_tally_of_size(pool, cap, k, *rest, &mut tally, &mut |set| {
                items.clear();
                items.extend(
                    (0u8..)
                        .zip(set)
                        .filter(|&(_, &count)| count > 0)
                        .map(|(letter, &count)| (letter, count)),
                );
                let mut sum = 0.0;
                for (i, &(letter, count)) in items.iter().enumerate() {
                    let without = (items.iter().enumerate())
                        .map(|(j, &(l, c))| (l, if j == i { c - 1 } else { c }))
                        .filter(|&(_, c)| c > 0);
                    sum += below[lat.rank_sparse_in_lattice(size - 1, without) as usize - start]
                        * f64::from(pool[letter as usize] - count + 1);
                }
                span[lat.rank_sparse_in_lattice(size, items.iter().copied()) as usize - *first] +=
                    sum / left;
            });
        };
        for round in jobs.chunks_mut(threads) {
            if let [one] = round {
                walk(one);
            } else {
                let walk = &walk;
                std::thread::scope(|s| {
                    for job in round.iter_mut() {
                        s.spawn(move || walk(job));
                    }
                });
            }
        }
    }
    let full = lat.size_start(rack_size) - start;
    weights.copy_within(full.., 0);
    weights.truncate(weights.len() - full);
    carried.pool.clear();
    carried.pool.extend_from_slice(pool);
    carried.size = rack_size;
}

// the leaves the mover could have kept, inferred from their move, each with its
// weight (not normalized). The board is the one before the move, and the pool
// is what the watcher could not see then: the tiles not on that board, less the
// rack the watcher still holds. A rack keeps the tiles it did not place: all of
// them for a pass, and for an exchange (whose size alone is seen) every leave
// of that size it holds, each weighed by what it gives up against the rack's
// best (by the lean arm, the one the table values most). Empty when there is
// nothing to weigh: the bag is empty before the move, or its own draw empties
// it (the mover then holds every unseen tile, whatever they kept), a pass
// (unless InferOptions' weigh_passes), or no rack from the pool could have made
// the move. The racks are weighed on this many threads.
#[inline]
pub fn kept_leaves<
    N: kwg::Node,
    L: kwg::Node,
    G: movegen::LaneBits,
    const JUMPS: bool,
    const ANY_SIDE: bool,
>(
    tables: &mut InferTables,
    move_generator: &mut movegen::KurniaMoveGenerator<G, JUMPS, ANY_SIDE>,
    board_snapshot: &movegen::BoardSnapshot<'_, N, L>,
    watcher_rack: &[u8],
    play: &movegen::Play,
    fuzz: f64,
    threads: usize,
) -> error::Returns<Vec<(Vec<u8>, f64)>> {
    kept_leaves_impl::<false, _, _, _, JUMPS, ANY_SIDE>(
        tables,
        move_generator,
        board_snapshot,
        watcher_rack,
        play,
        fuzz,
        threads,
    )
}

// as kept_leaves, by the lean arm alone, so that a build that runs only that
// arm (the wasm) need not carry the other
#[inline]
pub fn kept_leaves_lean<
    N: kwg::Node,
    L: kwg::Node,
    G: movegen::LaneBits,
    const JUMPS: bool,
    const ANY_SIDE: bool,
>(
    tables: &mut InferTables,
    move_generator: &mut movegen::KurniaMoveGenerator<G, JUMPS, ANY_SIDE>,
    board_snapshot: &movegen::BoardSnapshot<'_, N, L>,
    watcher_rack: &[u8],
    play: &movegen::Play,
    fuzz: f64,
    threads: usize,
) -> error::Returns<Vec<(Vec<u8>, f64)>> {
    kept_leaves_impl::<true, _, _, _, JUMPS, ANY_SIDE>(
        tables,
        move_generator,
        board_snapshot,
        watcher_rack,
        play,
        fuzz,
        threads,
    )
}

#[inline(always)]
fn kept_leaves_impl<
    const LEAN: bool,
    N: kwg::Node,
    L: kwg::Node,
    G: movegen::LaneBits,
    const JUMPS: bool,
    const ANY_SIDE: bool,
>(
    tables: &mut InferTables,
    move_generator: &mut movegen::KurniaMoveGenerator<G, JUMPS, ANY_SIDE>,
    board_snapshot: &movegen::BoardSnapshot<'_, N, L>,
    watcher_rack: &[u8],
    play: &movegen::Play,
    fuzz: f64,
    threads: usize,
) -> error::Returns<Vec<(Vec<u8>, f64)>> {
    let mut leaves = PackedLeaves::new();
    kept_leaves_packed_impl::<LEAN, _, _, _, JUMPS, ANY_SIDE>(
        tables,
        move_generator,
        board_snapshot,
        watcher_rack,
        (play, Held::Any),
        InferOptions {
            fuzz,
            threads,
            weigh_passes: false,
        },
        (&mut leaves, None),
    )?;
    Ok((0..leaves.len())
        .map(|i| (leaves.leave(i).to_vec(), leaves.weight(i)))
        .collect())
}

// kept_leaves's leaves, packed, in the same order
#[inline]
pub fn kept_leaves_packed<
    N: kwg::Node,
    L: kwg::Node,
    G: movegen::LaneBits,
    const JUMPS: bool,
    const ANY_SIDE: bool,
>(
    tables: &mut InferTables,
    move_generator: &mut movegen::KurniaMoveGenerator<G, JUMPS, ANY_SIDE>,
    board_snapshot: &movegen::BoardSnapshot<'_, N, L>,
    watcher_rack: &[u8],
    play: &movegen::Play,
    fuzz: f64,
    threads: usize,
) -> error::Returns<PackedLeaves> {
    let mut leaves = PackedLeaves::new();
    kept_leaves_packed_impl::<false, _, _, _, JUMPS, ANY_SIDE>(
        tables,
        move_generator,
        board_snapshot,
        watcher_rack,
        (play, Held::Any),
        InferOptions {
            fuzz,
            threads,
            weigh_passes: false,
        },
        (&mut leaves, None),
    )?;
    Ok(leaves)
}

pub struct InferOptions {
    pub fuzz: f64,
    pub threads: usize,
    pub weigh_passes: bool, // a pass weighs racks too, not only plain deals
}

// the same, into leaves, whose memory is kept: an inference allocates nothing
// once the tables' scratch and the list have grown (on one thread)
#[inline]
pub fn kept_leaves_packed_into<
    N: kwg::Node,
    L: kwg::Node,
    G: movegen::LaneBits,
    const JUMPS: bool,
    const ANY_SIDE: bool,
>(
    tables: &mut InferTables,
    move_generator: &mut movegen::KurniaMoveGenerator<G, JUMPS, ANY_SIDE>,
    board_snapshot: &movegen::BoardSnapshot<'_, N, L>,
    watcher_rack: &[u8],
    play: &movegen::Play,
    options: InferOptions,
    leaves: &mut PackedLeaves,
) -> error::Returns<()> {
    kept_leaves_packed_impl::<false, _, _, _, JUMPS, ANY_SIDE>(
        tables,
        move_generator,
        board_snapshot,
        watcher_rack,
        (play, Held::Any),
        options,
        (leaves, None),
    )
}

// as kept_leaves_packed_into, each rack weighed from what the watcher knew
// before the move, and where carried is given, what the mover's next inference
// is to start from left in it
#[inline]
pub fn kept_leaves_carried_into<
    N: kwg::Node,
    L: kwg::Node,
    G: movegen::LaneBits,
    const JUMPS: bool,
    const ANY_SIDE: bool,
>(
    tables: &mut InferTables,
    move_generator: &mut movegen::KurniaMoveGenerator<G, JUMPS, ANY_SIDE>,
    board_snapshot: &movegen::BoardSnapshot<'_, N, L>,
    watcher_rack: &[u8],
    (play, held): (&movegen::Play, Held<'_>),
    options: InferOptions,
    (leaves, carried): (&mut PackedLeaves, Option<&mut Carried>),
) -> error::Returns<()> {
    kept_leaves_packed_impl::<false, _, _, _, JUMPS, ANY_SIDE>(
        tables,
        move_generator,
        board_snapshot,
        watcher_rack,
        (play, held),
        options,
        (leaves, carried),
    )
}

// as kept_leaves_packed, by the lean arm alone, as kept_leaves_lean
#[inline]
pub fn kept_leaves_packed_lean<
    N: kwg::Node,
    L: kwg::Node,
    G: movegen::LaneBits,
    const JUMPS: bool,
    const ANY_SIDE: bool,
>(
    tables: &mut InferTables,
    move_generator: &mut movegen::KurniaMoveGenerator<G, JUMPS, ANY_SIDE>,
    board_snapshot: &movegen::BoardSnapshot<'_, N, L>,
    watcher_rack: &[u8],
    play: &movegen::Play,
    options: InferOptions,
) -> error::Returns<PackedLeaves> {
    let mut leaves = PackedLeaves::new();
    kept_leaves_packed_impl::<true, _, _, _, JUMPS, ANY_SIDE>(
        tables,
        move_generator,
        board_snapshot,
        watcher_rack,
        (play, Held::Any),
        options,
        (&mut leaves, None),
    )?;
    Ok(leaves)
}

#[inline]
fn kept_leaves_packed_impl<
    const LEAN: bool,
    N: kwg::Node,
    L: kwg::Node,
    G: movegen::LaneBits,
    const JUMPS: bool,
    const ANY_SIDE: bool,
>(
    tables: &mut InferTables,
    move_generator: &mut movegen::KurniaMoveGenerator<G, JUMPS, ANY_SIDE>,
    board_snapshot: &movegen::BoardSnapshot<'_, N, L>,
    watcher_rack: &[u8],
    (play, held): (&movegen::Play, Held<'_>),
    InferOptions {
        fuzz,
        threads,
        weigh_passes,
    }: InferOptions,
    (leaves, mut carried): (&mut PackedLeaves, Option<&mut Carried>),
) -> error::Returns<()> {
    leaves.clear();
    if let Some(carried) = carried.as_deref_mut() {
        carried.clear();
    }
    let game_config = board_snapshot.game_config;
    let board_tiles = board_snapshot.board_tiles;
    let num_letters = game_config.alphabet().len() as usize;
    let rack_size = game_config.rack_size() as usize;
    let fuzz = equity_fuzz(fuzz)?;
    let InferTables {
        lat,
        leave,
        ln_binom,
        sheet,
        movegen_rack,
        blank_deltas,
        pool,
        placed,
        avail,
        racks,
        weighed,
        mix,
        mixed,
        kept,
        tiles: leave_tiles,
        weigh,
        keeps,
        held: held_scratch,
        carry: carry_scratch,
        ..
    } = tables;
    game_config.tiles_off_board_into(board_tiles, pool)?;
    game_config.take_rack(pool, watcher_rack)?;
    let pool_size: usize = pool.iter().map(|&c| c as usize).sum();
    if pool_size <= rack_size {
        return Ok(());
    }
    let bag = pool_size - rack_size;
    placed.clear();
    placed.resize(num_letters, 0);
    let (exchanged, move_value) = move_tiles_and_value(board_snapshot, play, placed)?;
    if placed.iter().zip(pool.iter()).any(|(&p, &u)| p > u) {
        return_error!("the move places tiles the watcher holds or the board has".into());
    }
    // a move placing as many tiles as the bag holds draws it empty (an exchange
    // leaves the bag as it was)
    if exchanged.is_none() && placed.iter().map(|&c| c as usize).sum::<usize>() >= bag {
        return Ok(());
    }
    let can_exchange = bag >= usize::try_from(game_config.exchange_tile_limit())?;
    if exchanged.is_some() && !can_exchange {
        return Ok(());
    }
    if exchanged.is_none() && !weigh_passes && placed.iter().all(|&c| c == 0) {
        return Ok(());
    }
    let tiles = match exchanged {
        Some(n) => MoveTiles::Exchanged(n),
        None => MoveTiles::Placed(placed),
    };
    // each rack as the walk finds it, with the log of its chance before the
    // move: the leave it kept and its gap, or where every keep of an exchange
    // is weighed (by all but the lean arm), the rack and its best equity, the
    // smallest gap kept apart
    let keep = match exchanged {
        Some(n) if !LEAN => Some(rack_size.saturating_sub(n)),
        _ => None,
    };
    let mut g0 = i32::MAX;
    racks.clear();
    let mut add = |rank: u32, ln_prior: f64, gap: i32, kept: u32| match keep {
        Some(_) => {
            g0 = g0.min(gap.max(0));
            racks.push((rank, gap + leave[kept as usize], ln_prior));
        }
        None => racks.push((kept, gap, ln_prior)),
    };
    let carried_from = match held {
        Held::Kept(from, put_back) if hold((from, put_back), pool, held_scratch) => Some(from),
        _ => None,
    };
    let mut add = |rank: u32, ln_prior: f64, gap: i32, kept: u32| {
        let ln_prior = match carried_from {
            Some(from) => held_ln_prior(lat, ln_binom, from, held_scratch, rank),
            None => ln_prior,
        };
        if ln_prior != f64::NEG_INFINITY {
            add(rank, ln_prior, gap, kept);
        }
    };
    let num_racks = match exchanged {
        Some(_) => count_tallies_of_size_in(pool, rack_size, &mut weigh.ways),
        None => {
            avail.clear();
            avail.extend(pool.iter().zip(placed.iter()).map(|(&u, &p)| u - p));
            count_tallies_of_size_in(
                avail,
                rack_size - placed.iter().map(|&c| c as usize).sum::<usize>(),
                &mut weigh.ways,
            )
        }
    };
    let generate_upto = if board_snapshot.anagrams.is_some() {
        GENERATE_EACH_RACK_UPTO
    } else {
        GENERATE_EACH_RACK_UPTO_LEAN
    };
    if num_racks <= generate_upto && board_tiles.iter().any(|&tile| tile != 0) {
        weigh_racks_by_generation::<LEAN, _, _, _, JUMPS, ANY_SIDE>(
            move_generator,
            board_snapshot,
            &InferSheet {
                lat,
                sheet: &[],
                leave,
                ln_binom,
            },
            pool,
            ObservedMove {
                tiles,
                value: move_value,
            },
            weigh,
            &mut add,
        )?;
    } else {
        infer_sheet_for(
            move_generator,
            board_tiles,
            SpellTables {
                game_config,
                kwg: board_snapshot.kwg,
                klv: board_snapshot.klv,
                lat,
            },
            pool,
            can_exchange,
            (movegen_rack, blank_deltas),
            sheet,
        )?;
        weigh_racks(
            &InferSheet {
                lat,
                sheet,
                leave,
                ln_binom,
            },
            pool,
            tiles,
            move_value,
            threads,
            weigh,
            &mut add,
        );
    }
    match keep {
        Some(keep) => weights_by_keep(lat, leave, racks, (keep, g0, fuzz), keeps, weighed)?,
        None => weights_by_leave(racks, fuzz, weighed),
    }
    let mix = *mix;
    let (by_chance, both, carried_by_chance) = mixed;
    if mix > 0.0 {
        match keep {
            Some(keep) => weights_by_keep(
                lat,
                leave,
                racks,
                (keep, g0, f64::INFINITY),
                keeps,
                by_chance,
            )?,
            None => weights_by_leave(racks, f64::INFINITY, by_chance),
        }
        mix_weights((weighed, by_chance), mix, both);
        std::mem::swap(weighed, both);
    }
    kept.clear();
    kept.resize(num_letters, 0);
    for &(rank, _, weight) in weighed.iter() {
        lat.unrank_in_lattice(rank as usize, kept);
        leave_tiles.clear();
        leave_tiles.extend(
            (0u8..)
                .zip(kept.iter())
                .flat_map(|(tile, &count)| std::iter::repeat_n(tile, count as usize)),
        );
        leaves.push(leave_tiles, weight)?;
    }
    // a placement or a pass carries its leaves on, with the tiles unseen after
    // it, which the mover's draw came from; an exchange, whose draw came
    // before the tiles thrown back went in, carries what carry_exchange makes
    // of its racks (but the lean arm's, which weighs one keep a rack)
    if let Some(carried) = carried
        && !weighed.is_empty()
    {
        match (exchanged, keep) {
            (None, _) => {
                let total = weighed.iter().map(|&(_, _, weight)| weight).sum::<f64>();
                carried
                    .pool
                    .extend(pool.iter().zip(placed.iter()).map(|(&u, &p)| u - p));
                let size = rack_size - placed.iter().map(|&c| c as usize).sum::<usize>();
                let start = lat.size_start(size);
                carried.size = size;
                carried
                    .weights
                    .resize(lat.size_start(size + 1) - start, 0.0);
                for &(rank, _, weight) in weighed.iter() {
                    carried.weights[rank as usize - start] += weight / total;
                }
            }
            (Some(n), Some(keep)) if bag >= n => {
                carry_exchange(
                    lat,
                    (leave, ln_binom),
                    racks,
                    (keep, g0, fuzz),
                    (pool, threads),
                    carry_scratch,
                    carried,
                );
                if mix > 0.0 {
                    carry_exchange(
                        lat,
                        (leave, ln_binom),
                        racks,
                        (keep, g0, f64::INFINITY),
                        (pool, threads),
                        carry_scratch,
                        carried_by_chance,
                    );
                    for (w, c) in carried.weights.iter_mut().zip(&carried_by_chance.weights) {
                        *w = (1.0 - mix) * *w + mix * c;
                    }
                }
            }
            _ => {}
        }
    }
    Ok(())
}

// the opponent's best reply to a move, over every rack they could hold: the
// mean and the variance of its equity, in points, as the census sheet values a
// reply (its score, with the opening's adjustment, and the leave it keeps; an
// exchange only when the bag allows one, a pass only when nothing else is
// legal), and the chance that their rack can play all its tiles
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Reply {
    pub mean: f64,
    pub variance: f64,
    pub bingo: f64,
}

// a reply's sums over the racks, each rack counted as many times as the pool
// can deal it, in millipoints
#[derive(Default)]
struct ReplySums {
    deals: u64,
    equity: i128,
    squares: i128,
    bingos: u64,
}

impl ReplySums {
    #[inline(always)]
    fn add(&mut self, other: &Self) {
        self.deals += other.deals;
        self.equity += other.equity;
        self.squares += other.squares;
        self.bingos += other.bingos;
    }

    // the reply they sum to, over all deals of the pool
    #[inline(always)]
    fn reply(&self, deals: u64) -> Reply {
        let scale = f64::from(equity::SCALE);
        let n = i128::from(deals);
        Reply {
            mean: self.equity as f64 / deals as f64 / scale,
            variance: (n * self.squares - self.equity * self.equity) as f64
                / (deals as f64 * deals as f64)
                / (scale * scale),
            bingo: self.bingos as f64 / deals as f64,
        }
    }
}

// C(n, k), exactly
#[inline(always)]
fn binom(n: usize, k: usize) -> u64 {
    if k > n {
        return 0;
    }
    (0..k).fold(1u64, |c, i| c * (n - i) as u64 / (i + 1) as u64)
}

struct ReplyTallies<'a> {
    pool: &'a [u8],
    kept: &'a [u8],
}

// the reply's sums over every full rack that holds the kept tiles (a tally) and
// draws the rest from the pool less them, each rack's best reply as
// best_equity_of gives it (its own leave, a pass, when it has no other); the
// draws walked as weigh_racks walks them, on this many threads when they are
// many
#[inline]
fn reply_sums(
    lat: &census::MultisetLattice,
    sheet: &[i32],
    leave: &[i32],
    ReplyTallies { pool, kept }: ReplyTallies<'_>,
    threads: usize,
    scratch: &mut WeighScratch,
    ways: &mut Vec<u64>,
) -> error::Returns<ReplySums> {
    let threads = if cfg!(target_family = "wasm") {
        1
    } else {
        threads
    };
    let num_letters = lat.num_letters();
    // the tiles left to draw once the kept are held, as for a move that placed
    // them, and how many to draw
    let left = scratch.begin(pool, &MoveTiles::Placed(kept), lat.rack_size());
    let WeighScratch {
        avail,
        cap,
        cur,
        rack,
        ways: counts,
        ..
    } = scratch;
    let (avail, cap) = (&avail[..], &cap[..]);
    // the ways to draw so many of each letter from what is left of it: letter
    // t's for r drawn at t * (left + 1) + r
    let stride = left + 1;
    ways.clear();
    ways.extend((avail.iter()).flat_map(|&u| (0..=left).map(move |r| binom(u as usize, r))));
    let ways = &ways[..];
    let add_rack = |drawn: &[u8], rack: &mut [u8], sums: &mut ReplySums| {
        for (r, (&d, &k)) in rack.iter_mut().zip(drawn.iter().zip(kept)) {
            *r = d + k;
        }
        let deals = (drawn.iter().zip(ways.chunks_exact(stride)))
            .map(|(&d, ways)| ways[d as usize])
            .product::<u64>();
        let rank = lat.rank_in_lattice(rack) as usize;
        let mut best = census::best_equity_of(lat, sheet, leave, rack).raw();
        if best < i32::MIN / 8 {
            best = leave[rank];
        }
        sums.deals += deals;
        sums.equity += i128::from(deals) * i128::from(best);
        sums.squares += i128::from(deals) * (i128::from(best) * i128::from(best));
        // only a play of every tile is entered for the whole rack, and it
        // scores at least its bonus
        if sheet[rank] > 0 {
            sums.bingos += deals;
        }
    };
    let mut sums = ReplySums::default();
    if threads <= 1 || count_tallies_of_size_in(avail, left, counts) < WEIGH_ACROSS_THREADS_FROM {
        each_tally_of_size(avail, cap, 0, left, cur, &mut |drawn| {
            add_rack(drawn, rack, &mut sums)
        });
        return Ok(sums);
    }
    // the draws in chunks by the counts of their first k letters, as
    // weigh_racks cuts them; the sums are exact, so the threads add theirs in
    // any order
    let mut k = 1;
    let mut prefixes: Vec<(Vec<u8>, usize)> = Vec::new();
    loop {
        prefixes.clear();
        let mut cur = vec![0u8; num_letters];
        each_prefix_of_size(avail, cap, 0, k, left, &mut cur, &mut |prefix, left| {
            prefixes.push((prefix.to_vec(), left));
        });
        if prefixes.len() >= threads * CHUNKS_A_THREAD || k >= num_letters {
            break;
        }
        k += 1;
    }
    let next = std::sync::atomic::AtomicUsize::new(0);
    std::thread::scope(|s| {
        let handles = (0..threads)
            .map(|_| {
                s.spawn(|| {
                    let mut cur = vec![0u8; num_letters];
                    let mut rack = vec![0u8; num_letters];
                    let mut mine = ReplySums::default();
                    loop {
                        let i = next.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                        let Some((prefix, left)) = prefixes.get(i) else {
                            break;
                        };
                        cur[..k].copy_from_slice(prefix);
                        each_tally_of_size(avail, cap, k, *left, &mut cur, &mut |drawn| {
                            add_rack(drawn, &mut rack, &mut mine)
                        });
                    }
                    mine
                })
            })
            .collect::<Vec<_>>();
        for handle in handles {
            match handle.join() {
                Ok(theirs) => sums.add(&theirs),
                Err(_) => return_error!("a thread weighing replies panicked".into()),
            }
        }
        Ok(sums)
    })
}

// the opponent's reply to a move, the board the one before it and the rack the
// mover's then: the opponent holds a full rack of what the mover could not see,
// and may exchange when the bag the mover draws from still holds enough after.
// Their rack is any full rack of that pool, by its chance to be dealt; or,
// given the leaves their last move kept as kept_leaves weighs them, one of
// those, by its weight, drawn to a full rack from the rest of the pool by the
// draw's chance, as a sim draws it. None when there is no bag to reply from:
// the pool holds no more than a rack, or the mover's draw empties the bag (an
// endgame, the solver's). The racks are weighed on this many threads.
#[inline]
pub fn reply_to<
    N: kwg::Node,
    L: kwg::Node,
    G: movegen::LaneBits,
    const JUMPS: bool,
    const ANY_SIDE: bool,
>(
    tables: &mut InferTables,
    move_generator: &mut movegen::KurniaMoveGenerator<G, JUMPS, ANY_SIDE>,
    board_snapshot: &movegen::BoardSnapshot<'_, N, L>,
    mover_rack: &[u8],
    kept: &[(Vec<u8>, f64)],
    play: &movegen::Play,
    threads: usize,
) -> error::Returns<Option<Reply>> {
    let game_config = board_snapshot.game_config;
    let num_letters = game_config.alphabet().len() as usize;
    let rack_size = game_config.rack_size() as usize;
    let InferTables {
        lat,
        leave,
        sheet,
        movegen_rack,
        blank_deltas,
        pool,
        placed: used,
        avail: held,
        kept: tally,
        tiles: board_tiles,
        weigh,
        ways,
        ..
    } = tables;
    game_config.tiles_off_board_into(board_snapshot.board_tiles, pool)?;
    game_config.take_rack(pool, mover_rack)?;
    let pool_size: usize = pool.iter().map(|&c| c as usize).sum();
    if pool_size <= rack_size {
        return Ok(None);
    }
    let bag = pool_size - rack_size;
    held.clear();
    held.resize(num_letters, 0);
    for &tile in mover_rack {
        held[tile as usize] += 1;
    }
    used.clear();
    used.resize(num_letters, 0);
    board_tiles.clear();
    board_tiles.extend_from_slice(board_snapshot.board_tiles);
    // the tiles the mover draws after it: as many as it places (an exchange
    // leaves the bag as it was)
    let drawn = match play {
        movegen::Play::Exchange { tiles } => {
            if !tiles.is_empty() && bag < usize::try_from(game_config.exchange_tile_limit())? {
                return_error!("the bag is too small to exchange".into());
            }
            for &tile in tiles.iter() {
                *used.get_mut(tile as usize).ok_or("no such tile")? += 1;
            }
            0
        }
        movegen::Play::Place {
            down,
            lane,
            idx,
            word,
            ..
        } => {
            let strider = game_config.board_layout().dim().lane(*down, *lane);
            for (i, &tile) in (*idx..).zip(word.iter()) {
                if tile != 0 {
                    board_tiles[strider.at_in_lane(i)] = tile;
                    *used
                        .get_mut(if tile & 0x80 != 0 { 0 } else { tile as usize })
                        .ok_or("no such tile")? += 1;
                }
            }
            used.iter().map(|&c| c as usize).sum()
        }
    };
    if used.iter().zip(held.iter()).any(|(&u, &h)| u > h) {
        return_error!("the move uses tiles the mover does not hold".into());
    }
    if drawn >= bag {
        return Ok(None);
    }
    let can_exchange = bag - drawn >= usize::try_from(game_config.exchange_tile_limit())?;
    infer_sheet_for(
        move_generator,
        board_tiles,
        SpellTables {
            game_config,
            kwg: board_snapshot.kwg,
            klv: board_snapshot.klv,
            lat,
        },
        pool,
        can_exchange,
        (movegen_rack, blank_deltas),
        sheet,
    )?;
    let (lat, sheet, leave, pool) = (&*lat, &sheet[..], &leave[..], &pool[..]);
    let mut reply_holding = |tally: &[u8]| -> error::Returns<Reply> {
        let n = tally.iter().map(|&c| c as usize).sum::<usize>();
        let sums = reply_sums(
            lat,
            sheet,
            leave,
            ReplyTallies { pool, kept: tally },
            threads,
            weigh,
            ways,
        )?;
        let deals = binom(pool_size - n, rack_size - n);
        if sums.deals != deals {
            return_error!(format!(
                "the racks were dealt {} ways, not the pool's {deals}",
                sums.deals
            ));
        }
        Ok(sums.reply(deals))
    };
    tally.clear();
    tally.resize(num_letters, 0);
    if kept.is_empty() {
        return Ok(Some(reply_holding(tally)?));
    }
    let (mut weight, mut mean, mut square, mut bingo) = (0.0, 0.0, 0.0, 0.0);
    for (tiles, w) in kept {
        if !w.is_finite() || *w < 0.0 {
            return_error!(format!("a kept leave weighs {w}"));
        }
        tally.iter_mut().for_each(|c| *c = 0);
        for &tile in tiles {
            *tally.get_mut(tile as usize).ok_or("no such tile")? += 1;
        }
        if tiles.len() > rack_size || tally.iter().zip(pool).any(|(&k, &u)| k > u) {
            return_error!("a kept leave is not in the pool".into());
        }
        let reply = reply_holding(tally)?;
        weight += w;
        mean += w * reply.mean;
        square += w * (reply.variance + reply.mean * reply.mean);
        bingo += w * reply.bingo;
    }
    if weight <= 0.0 {
        return_error!("the kept leaves weigh nothing".into());
    }
    let mean = mean / weight;
    Ok(Some(Reply {
        mean,
        variance: square / weight - mean * mean,
        bingo: bingo / weight,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{bites, build};

    // every word of two or three tiles from A, E, and T
    #[inline]
    fn tiny_words() -> Vec<bites::Bites> {
        const LETTERS: [u8; 3] = [1, 5, 20];
        let mut words = Vec::new();
        for &a in &LETTERS {
            for &b in &LETTERS {
                words.push([a, b][..].into());
                for &c in &LETTERS {
                    words.push([a, b, c][..].into());
                }
            }
        }
        words.sort_unstable();
        words
    }

    #[test]
    fn fitted_fuzz_reads_the_share_of_the_starting_bag() {
        let mix = FUZZ_PRIOR_MIX;
        for (bag, start, fuzz) in [
            (86, 86, 1.507),
            (200, 86, 1.507),
            (43, 86, 1.949),
            (1, 86, 69.796),
            (0, 86, 69.796),
            (186, 186, 1.507),
            (93, 186, 1.949),
        ] {
            assert_eq!(
                fitted_fuzz(bag, start, false),
                (fuzz, mix),
                "bag {bag} of {start}"
            );
        }
        let between = fitted_fuzz(85, 172, false).0;
        assert!(between > 1.909 && between < 1.949);
        assert_eq!(fitted_fuzz(50, 86, true), (FUZZ_EXCHANGE, 0.0));
    }

    // tiny_words as a graph
    #[inline]
    fn tiny_kwg() -> kwg::Kwg<kwg::Node24> {
        let bytes = build::build(
            build::BuildContent::Gaddawg,
            build::BuildLayout::Wolges,
            build::BuildOrder::Sorted,
            &tiny_words(),
        )
        .unwrap();
        kwg::kbwg_from_bytes_alloc(bytes.to_vec(), false).unwrap()
    }

    // the leaves a move keeps, seen either on an empty board by a player
    // holding an E, or on a board holding the first tiles in alphabet order,
    // this many, by the player holding the Z: with 79 on it, every tile but the
    // last 21 (3 S, then T to Z), so that only 20 are unseen and the inference
    // is quick
    #[inline]
    fn leaves_after(on_board: usize, play: &movegen::Play, fuzz: f64) -> Vec<(Vec<u8>, f64)> {
        leaves_under(
            &game_config::make_english_game_config(),
            &tiny_kwg(),
            on_board,
            play,
            fuzz,
        )
    }

    // the same, under these rules and words
    #[inline]
    fn leaves_under(
        game_config: &game_config::GameConfig,
        kwg: &kwg::Kwg<kwg::Node24>,
        on_board: usize,
        play: &movegen::Play,
        fuzz: f64,
    ) -> Vec<(Vec<u8>, f64)> {
        leaves_mixed(game_config, kwg, on_board, play, (fuzz, 0.0))
    }

    // the same, this share weighed by the chance to be dealt alone
    #[inline]
    fn leaves_mixed(
        game_config: &game_config::GameConfig,
        kwg: &kwg::Kwg<kwg::Node24>,
        on_board: usize,
        play: &movegen::Play,
        (fuzz, mix): (f64, f64),
    ) -> Vec<(Vec<u8>, f64)> {
        let alphabet = game_config.alphabet();
        let klv = klv::kbwg_from_bytes_alloc(klv::EMPTY_KLV_BYTES.to_vec()).unwrap();
        let mut board_tiles = vec![0u8; 15 * 15];
        let mut watcher_rack = vec![5];
        if on_board > 0 {
            let mut square = 0;
            for tile in 0..alphabet.len() {
                for _ in 0..alphabet.freq(tile) {
                    if square < on_board {
                        board_tiles[square] = if tile == 0 { 0x81 } else { tile };
                        square += 1;
                    }
                }
            }
            watcher_rack[0] = 26;
        }
        let board_snapshot = movegen::BoardSnapshot {
            board_tiles: &board_tiles,
            game_config,
            kwg,
            anagrams: None,
            rack_lengths: None,
            klv: &klv,
        };
        let mut tables = InferTables::new(game_config, &klv).unwrap();
        tables.set_prior_mix(mix);
        let mut move_generator = movegen::KurniaMoveGenerator::new(game_config);
        let mut leaves = PackedLeaves::new();
        kept_leaves_packed_into(
            &mut tables,
            &mut move_generator,
            &board_snapshot,
            &watcher_rack,
            play,
            InferOptions {
                fuzz,
                threads: 2,
                weigh_passes: true,
            },
            &mut leaves,
        )
        .unwrap();
        (0..leaves.len())
            .map(|i| (leaves.leave(i).to_vec(), leaves.weight(i)))
            .collect()
    }

    // a prior mix gives each leave that share of its weight by the chance to be
    // dealt alone and the rest of its weight at the fuzz, each a share of its
    // own total
    #[test]
    fn a_prior_mix_blends_the_fuzz_with_the_chance_to_be_dealt() {
        let english = game_config::make_english_game_config();
        let share = |leaves: &[(Vec<u8>, f64)]| {
            let total = leaves.iter().map(|(_, w)| w).sum::<f64>();
            leaves
                .iter()
                .map(|(tiles, w)| (tiles.clone(), w / total))
                .collect::<Vec<_>>()
        };
        for play in [
            movegen::Play::Exchange {
                tiles: [0u8; 3][..].into(),
            },
            movegen::Play::Exchange {
                tiles: [][..].into(),
            },
        ] {
            for fuzz in [0.0, 3.0] {
                let leaves =
                    |weights| share(&leaves_mixed(&english, &tiny_kwg(), 79, &play, weights));
                let by_fuzz = leaves((fuzz, 0.0));
                let by_chance = leaves((f64::INFINITY, 0.0));
                let mixed = leaves((fuzz, 0.25));
                assert!(!by_fuzz.is_empty());
                assert_eq!(mixed.len(), by_chance.len());
                for (tiles, w) in &mixed {
                    let of = |list: &[(Vec<u8>, f64)]| {
                        list.iter()
                            .find(|(t, _)| t == tiles)
                            .map_or(0.0, |&(_, w)| w)
                    };
                    let want = 0.75 * of(&by_fuzz) + 0.25 * of(&by_chance);
                    assert!(
                        (w - want).abs() <= f64::EPSILON * 4.0,
                        "{tiles:?}: {w} {want}"
                    );
                }
            }
        }
    }

    #[test]
    #[inline]
    fn jumbled_rules_infer_as_classic_ones_on_words_whose_anagrams_are_words() {
        let classic = game_config::make_english_game_config();
        let jumbled = game_config::make_jumbled_english_game_config();
        let bytes = build::build(
            build::BuildContent::DawgOnly,
            build::BuildLayout::Wolges,
            build::BuildOrder::Sorted,
            &build::make_alphagrams(&tiny_words()).unwrap(),
        )
        .unwrap();
        let alphagrams = kwg::kbwg_from_bytes_alloc(bytes.to_vec(), false).unwrap();
        let at = movegen::Play::Place {
            down: false,
            lane: 7,
            idx: 7,
            word: [1, 20][..].into(),
            score: equity::Equity::new(0),
        };
        for fuzz in [0.0, 3.0] {
            let want = leaves_under(&classic, &tiny_kwg(), 0, &at, fuzz);
            assert!(!want.is_empty());
            assert_eq!(leaves_under(&jumbled, &alphagrams, 0, &at, fuzz), want);
        }
    }

    // so too the sheet, which jumbled rules build each span's multisets once
    #[test]
    #[inline]
    fn jumbled_rules_build_the_classic_sheet_on_words_whose_anagrams_are_words() {
        let klv = klv::kbwg_from_bytes_alloc(klv::EMPTY_KLV_BYTES.to_vec()).unwrap();
        let bytes = build::build(
            build::BuildContent::DawgOnly,
            build::BuildLayout::Wolges,
            build::BuildOrder::Sorted,
            &build::make_alphagrams(&tiny_words()).unwrap(),
        )
        .unwrap();
        let alphagrams = kwg::kbwg_from_bytes_alloc(bytes.to_vec(), false).unwrap();
        let classic = tiny_kwg();
        let mut board_tiles = vec![0u8; 15 * 15];
        // the empty board, then a midgame one
        for step in 0..2 {
            if step > 0 {
                board_tiles[7 * 15 + 7..7 * 15 + 10].copy_from_slice(&[1, 20, 5]);
                board_tiles[8 * 15 + 9] = 0x81;
            }
            let sheet_under = |game_config: &game_config::GameConfig, kwg| {
                let lat = census::MultisetLattice::new(27, 7).unwrap();
                let pool = game_config.tiles_off_board(&board_tiles).unwrap();
                let mut sheet = Vec::new();
                infer_sheet_for(
                    &mut movegen::KurniaMoveGenerator::new(game_config),
                    &board_tiles,
                    SpellTables {
                        game_config,
                        kwg,
                        klv: &klv,
                        lat: &lat,
                    },
                    &pool,
                    true,
                    (&mut Vec::new(), &mut Vec::new()),
                    &mut sheet,
                )
                .unwrap();
                sheet
            };
            let want = sheet_under(&game_config::make_english_game_config(), &classic);
            assert!(want.iter().any(|&v| v > 0));
            assert!(
                sheet_under(
                    &game_config::make_jumbled_english_game_config(),
                    &alphagrams
                ) == want
            );
        }
    }

    #[test]
    #[inline]
    fn an_opening_two_tile_play_keeps_five_and_at_fuzz_0_nothing_that_plays_more() {
        // AT across the center: a rack holding another A, E, or T had a
        // three-tile word worth more (a blank adds nothing to a score), so at
        // fuzz 0 none of those racks weighs anything
        let play = movegen::Play::Place {
            down: false,
            lane: 7,
            idx: 7,
            word: [1, 20][..].into(),
            score: equity::Equity::new(0),
        };
        let leaves = leaves_after(0, &play, 0.0);
        assert!(!leaves.is_empty());
        for (leave, weight) in &leaves {
            assert_eq!(leave.len(), 5);
            assert!(*weight > 0.0);
            assert!(
                leave.iter().all(|&t| ![1, 5, 20].contains(&t)),
                "{leave:?} could have played more"
            );
        }
        // above fuzz 0 those weigh something too
        let leaves = leaves_after(0, &play, 3.0);
        assert!(leaves.iter().any(|(leave, _)| leave.contains(&5)));
    }

    #[test]
    #[inline]
    fn an_exchange_keeps_the_rest_and_a_pass_keeps_all() {
        let exchange = movegen::Play::Exchange {
            tiles: [0u8; 3][..].into(),
        };
        let leaves = leaves_after(79, &exchange, 3.0);
        assert!(!leaves.is_empty());
        for (leave, _) in leaves {
            assert_eq!(leave.len(), 4);
        }
        let pass = movegen::Play::Exchange {
            tiles: [][..].into(),
        };
        let leaves = leaves_after(79, &pass, 3.0);
        assert!(!leaves.is_empty());
        for (leave, _) in leaves {
            assert_eq!(leave.len(), 7);
        }
    }

    // by default a pass infers nothing at any bag size, so its rack is dealt by
    // plain draw chances; weigh_passes weighs each rack it could have kept
    #[test]
    #[inline]
    fn a_pass_infers_nothing_unless_passes_are_weighed() {
        let game_config = game_config::make_english_game_config();
        let kwg = tiny_kwg();
        let klv = klv::kbwg_from_bytes_alloc(klv::EMPTY_KLV_BYTES.to_vec()).unwrap();
        let pass = movegen::Play::Exchange {
            tiles: [][..].into(),
        };
        let mut tables = InferTables::new(&game_config, &klv).unwrap();
        let mut move_generator = movegen::KurniaMoveGenerator::new(&game_config);
        for on_board in [0, 79, 91] {
            let alphabet = game_config.alphabet();
            let mut board_tiles = vec![0u8; 15 * 15];
            let mut square = 0;
            for tile in 0..alphabet.len() {
                for _ in 0..alphabet.freq(tile) {
                    if square < on_board {
                        board_tiles[square] = if tile == 0 { 0x81 } else { tile };
                        square += 1;
                    }
                }
            }
            let watcher_rack = [if on_board > 0 { 26 } else { 5 }];
            let board_snapshot = movegen::BoardSnapshot {
                board_tiles: &board_tiles,
                game_config: &game_config,
                kwg: &kwg,
                anagrams: None,
                rack_lengths: None,
                klv: &klv,
            };
            let mut leaves = PackedLeaves::new();
            for weigh_passes in [false, true] {
                kept_leaves_packed_into(
                    &mut tables,
                    &mut move_generator,
                    &board_snapshot,
                    &watcher_rack,
                    &pass,
                    InferOptions {
                        fuzz: FUZZ,
                        threads: 2,
                        weigh_passes,
                    },
                    &mut leaves,
                )
                .unwrap();
                assert_eq!(
                    leaves.is_empty(),
                    !weigh_passes,
                    "{on_board} {weigh_passes}"
                );
            }
            assert!((0..leaves.len()).all(|i| leaves.leave(i).len() == 7));
        }
        let weighed = leaves_after(79, &pass, FUZZ);
        let plain = kept_leaves(
            &mut tables,
            &mut move_generator,
            &movegen::BoardSnapshot {
                board_tiles: &[0u8; 15 * 15],
                game_config: &game_config,
                kwg: &kwg,
                anagrams: None,
                rack_lengths: None,
                klv: &klv,
            },
            &[5],
            &pass,
            FUZZ,
            2,
        )
        .unwrap();
        assert!(!weighed.is_empty() && plain.is_empty());
    }

    #[test]
    #[inline]
    fn an_exchange_weighs_each_keep_by_what_it_gives_up() {
        // a rack of AAB keeping one: B is worth 3.105 points more than A
        let lat = census::MultisetLattice::new(3, 3).unwrap();
        let mut leave = vec![0i32; lat.len()];
        leave[lat.rank(&[0, 1, 0]).unwrap() as usize] = 1_000;
        leave[lat.rank(&[0, 0, 1]).unwrap() as usize] = 4_105;
        let mut keeps = Vec::new();
        each_keep(
            &lat,
            &leave,
            (&[0, 2, 1], &mut [0u8; 3], &mut [0usize; 4]),
            4_112,
            1,
            &mut |kept, gap| keeps.push((lat.tally(kept as usize).unwrap(), gap)),
        );
        keeps.sort_unstable();
        assert_eq!(keeps, [(vec![0, 0, 1], 7), (vec![0, 1, 0], 3_112)]);
        let mut weighed = Vec::new();
        weights_by_keep(
            &lat,
            &leave,
            &[(lat.rank(&[0, 2, 1]).unwrap(), 4_112, 0.0)],
            (1, 7, equity_fuzz(3.105).unwrap()),
            &mut KeepScratch::default(),
            &mut weighed,
        )
        .unwrap();
        assert_eq!(weighed.len(), 2);
        let (hi, lo) = (
            weighed[0].2.max(weighed[1].2),
            weighed[0].2.min(weighed[1].2),
        );
        assert!((hi / lo / std::f64::consts::E - 1.0).abs() < 1e-12);
    }

    #[test]
    #[inline]
    fn an_exchange_at_fuzz_0_keeps_every_keep_that_ties() {
        // the empty klv values every leave alike, so every 4 of the pool ties
        let game_config = game_config::make_english_game_config();
        let alphabet = game_config.alphabet();
        let mut pool = (0..alphabet.len())
            .map(|tile| alphabet.freq(tile))
            .collect::<Vec<_>>();
        let mut on_board = 79u8;
        for count in pool.iter_mut() {
            let taken = (*count).min(on_board);
            *count -= taken;
            on_board -= taken;
        }
        pool[26] -= 1;
        let exchange = movegen::Play::Exchange {
            tiles: [0u8; 3][..].into(),
        };
        let leaves = leaves_after(79, &exchange, 0.0);
        assert_eq!(leaves.len(), count_tallies_of_size(&pool, 4));
    }

    // summed by keep, an exchange's leaves weigh what a list of every keep of
    // every rack weighs, bit for bit, on threads or one
    #[test]
    #[inline]
    fn an_exchanges_keeps_summed_by_keep_are_its_keeps_listed() {
        let game_config = game_config::make_english_game_config();
        let kwg = tiny_kwg();
        let klv = klv::kbwg_from_bytes_alloc(klv::EMPTY_KLV_BYTES.to_vec()).unwrap();
        let tables = InferTables::new(&game_config, &klv).unwrap();
        let (lat, leave) = (&tables.lat, &tables.leave);
        let mut board_tiles = vec![0u8; 15 * 15];
        let mut square = 0;
        for tile in 0..game_config.alphabet().len() {
            for _ in 0..game_config.alphabet().freq(tile) {
                if square < 70 {
                    board_tiles[square] = if tile == 0 { 0x81 } else { tile };
                    square += 1;
                }
            }
        }
        let mut pool = game_config.tiles_off_board(&board_tiles).unwrap();
        game_config.take_rack(&mut pool, &[26]).unwrap();
        let ln_binom = LnBinom::new(pool.iter().map(|&c| c as usize).sum());
        let mut sheet = Vec::new();
        infer_sheet_for(
            &mut movegen::KurniaMoveGenerator::new(&game_config),
            &board_tiles,
            SpellTables {
                game_config: &game_config,
                kwg: &kwg,
                klv: &klv,
                lat,
            },
            &pool,
            true,
            (&mut Vec::new(), &mut Vec::new()),
            &mut sheet,
        )
        .unwrap();
        let is = InferSheet {
            lat,
            sheet: &sheet,
            leave,
            ln_binom: &ln_binom,
        };
        let fuzz = equity_fuzz(FUZZ).unwrap();
        let (mut racks, mut listed) = (Vec::new(), Vec::new());
        weigh_racks(
            &is,
            &pool,
            MoveTiles::Exchanged(3),
            0,
            1,
            &mut WeighScratch::new(),
            &mut |rank, ln_prior, gap, kept| {
                let best = gap + leave[kept as usize];
                racks.push((rank, best, ln_prior));
                each_keep(
                    lat,
                    leave,
                    (
                        &lat.tally(rank as usize).unwrap(),
                        &mut vec![0u8; lat.num_letters()],
                        &mut vec![0usize; lat.num_letters() + 1],
                    ),
                    best,
                    4,
                    &mut |kept, gap| listed.push((kept, gap, ln_prior)),
                );
            },
        );
        let mut by_list = Vec::new();
        weights_by_leave(&listed, fuzz, &mut by_list);
        let g0 = listed.iter().map(|&(_, gap, _)| gap.max(0)).min().unwrap();
        let mut by_keep = Vec::new();
        weights_by_keep(
            lat,
            leave,
            &racks,
            (4, g0, fuzz),
            &mut KeepScratch::default(),
            &mut by_keep,
        )
        .unwrap();
        assert!(by_keep.len() > 1);
        assert_eq!(by_keep.len(), by_list.len());
        for (a, b) in by_keep.iter().zip(&by_list) {
            assert_eq!((a.0, a.2.to_bits()), (b.0, b.2.to_bits()));
        }
        let exchange = movegen::Play::Exchange {
            tiles: [0u8; 3][..].into(),
        };
        let infer_on = |threads| {
            kept_leaves(
                &mut InferTables::new(&game_config, &klv).unwrap(),
                &mut movegen::KurniaMoveGenerator::new(&game_config),
                &movegen::BoardSnapshot {
                    board_tiles: &board_tiles,
                    game_config: &game_config,
                    kwg: &kwg,
                    anagrams: None,
                    rack_lengths: None,
                    klv: &klv,
                },
                &[26],
                &exchange,
                FUZZ,
                threads,
            )
            .unwrap()
        };
        let one = infer_on(1);
        assert_eq!(one.len(), by_list.len());
        for ((tiles, weight), b) in one.iter().zip(&by_list) {
            assert_eq!(lat.rank_bytes(tiles).unwrap(), b.0);
            assert_eq!(weight.to_bits(), b.2.to_bits());
        }
        assert_eq!(infer_on(4), one);
    }

    #[test]
    #[inline]
    fn a_move_that_draws_the_bag_empty_leaves_nothing_to_weigh() {
        // with 91 on the board and the Z watching, 8 are unseen (U, two V, two
        // W, X, two Y), so the bag holds one: a move placing a Y draws it, and
        // the mover then holds every unseen tile, whatever they kept
        let y = movegen::Play::Place {
            down: false,
            lane: 7,
            idx: 7,
            word: [25][..].into(),
            score: equity::Equity::new(0),
        };
        assert!(leaves_after(91, &y, 3.0).is_empty());
        // a pass leaves the bag as it was, so its leaves are weighed
        let pass = movegen::Play::Exchange {
            tiles: [][..].into(),
        };
        assert!(!leaves_after(91, &pass, 3.0).is_empty());
    }

    // a board holding the first tiles in alphabet order, this many (a blank as
    // an A), and that board with tiles placed
    #[inline]
    fn board_of(game_config: &game_config::GameConfig, on_board: usize) -> Vec<u8> {
        let alphabet = game_config.alphabet();
        let mut board_tiles = vec![0u8; 15 * 15];
        let mut square = 0;
        for tile in 0..alphabet.len() {
            for _ in 0..alphabet.freq(tile) {
                if square < on_board {
                    board_tiles[square] = if tile == 0 { 0x81 } else { tile };
                    square += 1;
                }
            }
        }
        board_tiles
    }

    #[inline]
    fn board_with(board_tiles: &[u8], placed: &[(usize, u8)]) -> Vec<u8> {
        let mut board_tiles = board_tiles.to_vec();
        for &(square, tile) in placed {
            board_tiles[square] = tile;
        }
        board_tiles
    }

    // one tile across the square at (lane, lane)
    #[inline]
    fn one_tile(lane: i8, tile: u8) -> movegen::Play {
        movegen::Play::Place {
            down: false,
            lane,
            idx: lane,
            word: [tile][..].into(),
            score: equity::Equity::ZERO,
        }
    }

    #[inline]
    fn exchange_of(n: usize) -> movegen::Play {
        movegen::Play::Exchange {
            tiles: vec![0u8; n][..].into(),
        }
    }

    // one inference of a move on a board by the watcher holding a rack, from
    // what it held before, on the tiny words with the empty klv: the leaves,
    // and what the inference carries on
    #[inline]
    fn infer_held(
        game_config: &game_config::GameConfig,
        (board_tiles, watcher_rack): (&[u8], &[u8]),
        play: &movegen::Play,
        held: Held<'_>,
    ) -> (Vec<(Vec<u8>, f64)>, Carried) {
        let kwg = tiny_kwg();
        let klv = klv::kbwg_from_bytes_alloc(klv::EMPTY_KLV_BYTES.to_vec()).unwrap();
        let mut leaves = PackedLeaves::new();
        let mut carried = Carried::default();
        kept_leaves_carried_into(
            &mut InferTables::new(game_config, &klv).unwrap(),
            &mut movegen::KurniaMoveGenerator::new(game_config),
            &movegen::BoardSnapshot {
                board_tiles,
                game_config,
                kwg: &kwg,
                anagrams: None,
                rack_lengths: None,
                klv: &klv,
            },
            watcher_rack,
            (play, held),
            InferOptions {
                fuzz: FUZZ,
                threads: 1,
                weigh_passes: true,
            },
            (&mut leaves, Some(&mut carried)),
        )
        .unwrap();
        let leaves = (0..leaves.len())
            .map(|i| (leaves.leave(i).to_vec(), leaves.weight(i)))
            .collect();
        (leaves, carried)
    }

    // the weights made to sum to 1, by leave
    #[inline]
    fn normalized(
        leaves: impl IntoIterator<Item = (Vec<u8>, f64)>,
    ) -> std::collections::BTreeMap<Vec<u8>, f64> {
        let mut by_leave = std::collections::BTreeMap::<Vec<u8>, f64>::new();
        for (leave, weight) in leaves {
            *by_leave.entry(leave).or_default() += weight;
        }
        by_leave.retain(|_, weight| *weight > 0.0);
        let total = by_leave.values().sum::<f64>();
        by_leave.values_mut().for_each(|weight| *weight /= total);
        by_leave
    }

    #[inline]
    fn assert_same_leaves(
        chained: &[(Vec<u8>, f64)],
        reference: &std::collections::BTreeMap<Vec<u8>, f64>,
    ) {
        let chained = normalized(chained.iter().cloned());
        assert!(!chained.is_empty());
        assert_eq!(
            chained.keys().collect::<Vec<_>>(),
            reference.keys().collect::<Vec<_>>()
        );
        for (leave, weight) in &chained {
            let want = reference[leave];
            assert!(
                (weight / want - 1.0).abs() < 1e-12,
                "{leave:?} {weight} {want}"
            );
        }
    }

    // the tiny words, the empty klv, their lattice and leave table, and a
    // generator, for the references below
    struct Tiny {
        kwg: kwg::Kwg<kwg::Node24>,
        klv: klv::Klv<kwg::Node24>,
        tables: InferTables,
        move_generator: movegen::KurniaMoveGenerator,
    }

    #[inline]
    fn tiny(game_config: &game_config::GameConfig) -> Tiny {
        let klv = klv::kbwg_from_bytes_alloc(klv::EMPTY_KLV_BYTES.to_vec()).unwrap();
        Tiny {
            kwg: tiny_kwg(),
            tables: InferTables::new(game_config, &klv).unwrap(),
            klv,
            move_generator: movegen::KurniaMoveGenerator::new(game_config),
        }
    }

    // what a rack (a tally) gives up making a move on a board (the kernel
    // every inference weighs a rack by, for the references below to hold
    // fixed): its best on the board, by its own generation (the best play or
    // exchange, a pass only when forced, and for a pass the rack kept whole
    // too), less the move's value and what it keeps; each keep of an exchange
    // with its own
    #[inline]
    fn gaps_of(
        tiny: &mut Tiny,
        game_config: &game_config::GameConfig,
        (board_tiles, rack): (&[u8], &[u8]),
        play: &movegen::Play,
    ) -> Vec<(Vec<u8>, i32)> {
        let (lat, leave) = (&tiny.tables.lat, &tiny.tables.leave);
        let board_snapshot = movegen::BoardSnapshot {
            board_tiles,
            game_config,
            kwg: &tiny.kwg,
            anagrams: None,
            rack_lengths: None,
            klv: &tiny.klv,
        };
        let rack_tiles = tiles_of(rack);
        tiny.move_generator
            .gen_moves_unfiltered(&movegen::GenMovesParams {
                board_snapshot: &board_snapshot,
                rack: &rack_tiles,
                max_gen: 1,
                num_exchanges_by_this_player: 0,
                pass_policy: movegen::PassPolicy::OnlyWhenForced,
                dynamic_leaves: None,
            })
            .unwrap();
        let top = &tiny.move_generator.plays[0];
        let mut best = match &top.play {
            movegen::Play::Exchange { tiles } if tiles.is_empty() => {
                i32::MIN / 4
                    + (0..=rack_tiles.len())
                        .map(|keep| census::best_keep_of(lat, leave, rack, keep).unwrap().0)
                        .max()
                        .unwrap()
                        .raw()
            }
            _ => top.equity.raw(),
        };
        let mut placed = vec![0u8; rack.len()];
        let (exchanged, value) = move_tiles_and_value(&board_snapshot, play, &mut placed).unwrap();
        let worth = |kept: &[u8]| leave[lat.rank(kept).unwrap() as usize];
        match exchanged {
            Some(n) => tallies_of(rack, rack_tiles.len() - n)
                .into_iter()
                .map(|kept| {
                    let gap = best - worth(&kept);
                    (kept, gap)
                })
                .collect(),
            None => {
                if placed.iter().all(|&c| c == 0) {
                    best = best.max(worth(rack));
                }
                let kept = less(rack, &placed);
                let gap = best - (value + worth(&kept));
                vec![(kept, gap)]
            }
        }
    }

    // the ways to draw these of each letter out of these
    #[inline]
    fn ways(from: &[u8], drawn: &[u8]) -> f64 {
        (from.iter().zip(drawn))
            .map(|(&f, &d)| {
                if d > f {
                    0.0
                } else {
                    binom(f as usize, d as usize) as f64
                }
            })
            .product()
    }

    #[inline]
    fn less(a: &[u8], b: &[u8]) -> Vec<u8> {
        a.iter().zip(b).map(|(&x, &y)| x - y).collect()
    }

    #[inline]
    fn plus(a: &[u8], b: &[u8]) -> Vec<u8> {
        a.iter().zip(b).map(|(&x, &y)| x + y).collect()
    }

    #[inline]
    fn tally_of(num_letters: usize, tiles: &[u8]) -> Vec<u8> {
        let mut tally = vec![0u8; num_letters];
        for &tile in tiles {
            tally[tile as usize] += 1;
        }
        tally
    }

    #[inline]
    fn tiles_of(tally: &[u8]) -> Vec<u8> {
        (0u8..)
            .zip(tally)
            .flat_map(|(tile, &count)| std::iter::repeat_n(tile, count as usize))
            .collect()
    }

    // every tally of this many tiles out of these
    #[inline]
    fn tallies_of(from: &[u8], size: usize) -> Vec<Vec<u8>> {
        let mut cap = vec![0usize; from.len() + 1];
        for t in (0..from.len()).rev() {
            cap[t] = cap[t + 1] + from[t] as usize;
        }
        let mut all = Vec::new();
        each_tally_of_size(from, &cap, 0, size, &mut vec![0u8; from.len()], &mut |t| {
            all.push(t.to_vec())
        });
        all
    }

    // the posterior of each leave the mover kept, by every hidden history
    // through its moves (a placement, a pass, an exchange, a placement), its
    // draws after the first placement and the exchange (drawn before the tiles
    // thrown back went in), and the watcher's own placements and draws
    // between, the watcher's draws read as evidence; given the watcher's view
    // alone, each inference carried from the last weighs every leave as it
    // does
    #[test]
    #[inline]
    fn a_chain_of_inferences_weighs_each_leave_as_its_whole_history_does() {
        let game_config = game_config::make_exchange_test_game_config();
        let num_letters = game_config.alphabet().len() as usize;
        let fuzz = equity_fuzz(FUZZ).unwrap();
        let kernel = |gap: i32| (-f64::from(gap.max(0)) / fuzz).exp();
        // 83 on the board: unseen are five T, four U, two V, two W, two Y, an
        // X, and the Z the watcher holds
        let board1 = board_of(&game_config, 83);
        let (t, u, v, y, z) = (20u8, 21u8, 22u8, 25u8, 26u8);
        let first = one_tile(7, y);
        let board2 = board_with(&board1, &[(7 * 15 + 7, y)]);
        // the watcher plays its Z and draws a U, then plays that and draws a
        // V, then plays that and draws a T
        let board3 = board_with(&board2, &[(9 * 15 + 9, z)]);
        let board4 = board_with(&board3, &[(11 * 15 + 11, u)]);
        let board5 = board_with(&board4, &[(13 * 15 + 13, v)]);
        let pass = exchange_of(0);
        let exchange = exchange_of(2);
        let fourth = one_tile(12, t);
        let (leaves1, carried1) = infer_held(&game_config, (&board1, &[z]), &first, Held::Any);
        let (leaves2, carried2) = infer_held(
            &game_config,
            (&board3, &[u]),
            &pass,
            Held::Kept(&carried1, &[]),
        );
        let (leaves3, carried3) = infer_held(
            &game_config,
            (&board4, &[v]),
            &exchange,
            Held::Kept(&carried2, &[]),
        );
        let (leaves4, _) = infer_held(
            &game_config,
            (&board5, &[t]),
            &fourth,
            Held::Kept(&carried3, &[]),
        );
        let mut pool1 = game_config.tiles_off_board(&board1).unwrap();
        game_config.take_rack(&mut pool1, &[z]).unwrap();
        let size1 = pool1.iter().map(|&c| c as usize).sum::<usize>();
        let one = |tile| tally_of(num_letters, &[tile]);
        let mut tiny = tiny(&game_config);
        let mut fourth_gaps = crate::fash::MyHashMap::<Vec<u8>, (Vec<u8>, i32)>::default();
        let mut posts = [(); 4].map(|_| std::collections::BTreeMap::<Vec<u8>, f64>::new());
        for rack1 in tallies_of(&pool1, 7) {
            if rack1[y as usize] == 0 {
                continue;
            }
            let prior = ways(&pool1, &rack1) / binom(size1, 7) as f64;
            let (kept1, gap1) =
                gaps_of(&mut tiny, &game_config, (&board1, &rack1), &first).remove(0);
            let w1 = prior * kernel(gap1);
            *posts[0].entry(tiles_of(&kept1)).or_default() += w1;
            let bag1 = less(&pool1, &rack1);
            for drew in tallies_of(&bag1, 1) {
                let w = w1 * ways(&bag1, &drew) / binom(size1 - 7, 1) as f64;
                // the watcher's U, drawn from what was left
                let left = less(&bag1, &drew);
                let w = w * ways(&left, &one(u)) / binom(size1 - 8, 1) as f64;
                if w == 0.0 {
                    continue;
                }
                let rack2 = plus(&kept1, &drew);
                let (_, gap2) =
                    gaps_of(&mut tiny, &game_config, (&board3, &rack2), &pass).remove(0);
                let w2 = w * kernel(gap2);
                *posts[1].entry(tiles_of(&rack2)).or_default() += w2;
                // the watcher's V, drawn from the rest
                let rest = less(&left, &one(u));
                let w = w2 * ways(&rest, &one(v)) / binom(size1 - 9, 1) as f64;
                if w == 0.0 {
                    continue;
                }
                let bag3 = less(&rest, &one(v));
                for (kept3, gap3) in gaps_of(&mut tiny, &game_config, (&board4, &rack2), &exchange)
                {
                    let w3 = w * kernel(gap3);
                    *posts[2].entry(tiles_of(&kept3)).or_default() += w3;
                    // the mover draws two, then throws back what it did not
                    // keep; the watcher plays its V and draws a T, and the
                    // mover places a T
                    let thrown = less(&rack2, &kept3);
                    for drew3 in tallies_of(&bag3, 2) {
                        let w = w3 * ways(&bag3, &drew3) / binom(size1 - 10, 2) as f64;
                        let rack3 = plus(&kept3, &drew3);
                        let bag4 = plus(&less(&bag3, &drew3), &thrown);
                        let w = w * ways(&bag4, &one(t)) / binom(size1 - 10, 1) as f64;
                        if w == 0.0 || rack3[t as usize] == 0 {
                            continue;
                        }
                        let (kept4, gap4) = fourth_gaps
                            .entry(rack3.clone())
                            .or_insert_with(|| {
                                gaps_of(&mut tiny, &game_config, (&board5, &rack3), &fourth)
                                    .remove(0)
                            })
                            .clone();
                        *posts[3].entry(tiles_of(&kept4)).or_default() += w * kernel(gap4);
                    }
                }
            }
        }
        let reference = posts.map(normalized);
        assert_same_leaves(&leaves1, &reference[0]);
        assert_same_leaves(&leaves2, &reference[1]);
        assert_same_leaves(&leaves3, &reference[2]);
        assert_same_leaves(&leaves4, &reference[3]);
        // past the exchange, the move weighed from any rack the unseen tiles
        // could make, or from the keeps alone as if their draw had come from
        // the bag with the tiles thrown back, is off by more than a hundredth
        // somewhere
        let off = |leaves: Vec<(Vec<u8>, f64)>| {
            normalized(leaves).iter().any(|(leave, weight)| {
                reference[3]
                    .get(leave)
                    .is_none_or(|want| (weight / want - 1.0).abs() > 0.01)
            })
        };
        let any = infer_held(&game_config, (&board5, &[t]), &fourth, Held::Any).0;
        assert!(off(any));
        let lat = census::MultisetLattice::new(num_letters, 7).unwrap();
        let start = lat.size_start(5);
        let mut weights = vec![0.0; lat.size_start(6) - start];
        for (leave, weight) in normalized(leaves3.iter().cloned()) {
            weights[lat.rank_bytes(&leave).unwrap() as usize - start] += weight;
        }
        let keeps = Carried {
            pool: carried3.pool.clone(),
            size: 5,
            weights,
        };
        let alone = infer_held(
            &game_config,
            (&board5, &[t]),
            &fourth,
            Held::Kept(&keeps, &[]),
        )
        .0;
        assert!(off(alone));
        // the watcher's draw is evidence: the pass weighed with the mover's
        // draw out of what is unseen now and none of the watcher's, as if it
        // had drawn nothing, is off by more than a hundredth somewhere
        let mut blind = carried1.clone();
        blind.pool = less(&carried1.pool, &one(u));
        let (blinded, _) = infer_held(
            &game_config,
            (&board3, &[u]),
            &pass,
            Held::Kept(&blind, &[]),
        );
        assert!(normalized(blinded).iter().any(|(leave, weight)| {
            reference[1]
                .get(leave)
                .is_none_or(|want| (weight / want - 1.0).abs() > 0.01)
        }));
    }

    // the watcher's exchange puts its tiles back after it draws, so the
    // mover's draw before could not hold them, and the watcher's draw is
    // evidence
    #[test]
    #[inline]
    fn a_chain_reads_the_watchers_exchange_and_draw_as_its_history_does() {
        let game_config = game_config::make_exchange_test_game_config();
        let num_letters = game_config.alphabet().len() as usize;
        let fuzz = equity_fuzz(FUZZ).unwrap();
        let kernel = |gap: i32| (-f64::from(gap.max(0)) / fuzz).exp();
        let board1 = board_of(&game_config, 83);
        let (t, u, y, z) = (20u8, 21u8, 25u8, 26u8);
        let first = one_tile(7, y);
        let board2 = board_with(&board1, &[(7 * 15 + 7, y)]);
        let second = one_tile(8, t);
        let (leaves1, carried1) = infer_held(&game_config, (&board1, &[z]), &first, Held::Any);
        // the watcher throws its Z back for a U
        let (leaves2, carried2) = infer_held(
            &game_config,
            (&board2, &[u]),
            &second,
            Held::Kept(&carried1, &[z]),
        );
        assert!(!carried2.is_empty());
        let mut pool1 = game_config.tiles_off_board(&board1).unwrap();
        game_config.take_rack(&mut pool1, &[z]).unwrap();
        let size1 = pool1.iter().map(|&c| c as usize).sum::<usize>();
        let one = |tile| tally_of(num_letters, &[tile]);
        let mut tiny = tiny(&game_config);
        let mut posts = [(); 2].map(|_| std::collections::BTreeMap::<Vec<u8>, f64>::new());
        for rack1 in tallies_of(&pool1, 7) {
            if rack1[y as usize] == 0 {
                continue;
            }
            let prior = ways(&pool1, &rack1) / binom(size1, 7) as f64;
            let (kept1, gap1) =
                gaps_of(&mut tiny, &game_config, (&board1, &rack1), &first).remove(0);
            let w1 = prior * kernel(gap1);
            *posts[0].entry(tiles_of(&kept1)).or_default() += w1;
            let bag1 = less(&pool1, &rack1);
            for drew in tallies_of(&bag1, 1) {
                let w = w1 * ways(&bag1, &drew) / binom(size1 - 7, 1) as f64;
                let left = less(&bag1, &drew);
                let w = w * ways(&left, &one(u)) / binom(size1 - 8, 1) as f64;
                let rack2 = plus(&kept1, &drew);
                if w == 0.0 || rack2[t as usize] == 0 {
                    continue;
                }
                let (kept2, gap2) =
                    gaps_of(&mut tiny, &game_config, (&board2, &rack2), &second).remove(0);
                *posts[1].entry(tiles_of(&kept2)).or_default() += w * kernel(gap2);
            }
        }
        let reference = posts.map(normalized);
        assert_same_leaves(&leaves1, &reference[0]);
        assert_same_leaves(&leaves2, &reference[1]);
        // no leave holds the Z, which was in the watcher's hand at the draw
        assert!(leaves2.iter().all(|(leave, _)| !leave.contains(&z)));
    }

    // a carried leave of nothing, weighing 1, out of what is unseen now,
    // weighs each rack as any rack the unseen tiles could make does, bit for
    // bit, on a board whose racks the sheet values (70 on it) and on one where
    // each is generated (85)
    #[test]
    #[inline]
    fn a_carried_leave_of_nothing_weighs_each_rack_as_any_rack_does() {
        let english = game_config::make_english_game_config();
        let kwg = tiny_kwg();
        let klv = klv::kbwg_from_bytes_alloc(klv::EMPTY_KLV_BYTES.to_vec()).unwrap();
        let racks_on = |on_board| {
            let mut pool = english
                .tiles_off_board(&board_of(&english, on_board))
                .unwrap();
            english.take_rack(&mut pool, &[26]).unwrap();
            count_tallies_of_size(&pool, 7)
        };
        assert!(racks_on(70) > GENERATE_EACH_RACK_UPTO);
        assert!(racks_on(85) <= GENERATE_EACH_RACK_UPTO_LEAN);
        for (on_board, play) in [
            (70, exchange_of(0)),
            (70, exchange_of(3)),
            (70, one_tile(7, 25)),
            (85, exchange_of(0)),
            (85, exchange_of(3)),
            (85, one_tile(7, 25)),
        ] {
            let board_tiles = board_of(&english, on_board);
            let mut unseen = english.tiles_off_board(&board_tiles).unwrap();
            english.take_rack(&mut unseen, &[26]).unwrap();
            let nothing = Carried {
                pool: unseen,
                size: 0,
                weights: vec![1.0],
            };
            let infer = |held| {
                let mut leaves = PackedLeaves::new();
                kept_leaves_carried_into(
                    &mut InferTables::new(&english, &klv).unwrap(),
                    &mut movegen::KurniaMoveGenerator::new(&english),
                    &movegen::BoardSnapshot {
                        board_tiles: &board_tiles,
                        game_config: &english,
                        kwg: &kwg,
                        anagrams: None,
                        rack_lengths: None,
                        klv: &klv,
                    },
                    &[26],
                    (&play, held),
                    InferOptions {
                        fuzz: FUZZ,
                        threads: 2,
                        weigh_passes: true,
                    },
                    (&mut leaves, None),
                )
                .unwrap();
                (0..leaves.len())
                    .map(|i| (leaves.leave(i).to_vec(), leaves.weight(i).to_bits()))
                    .collect::<Vec<_>>()
            };
            let any = infer(Held::Any);
            assert!(!any.is_empty());
            assert_eq!(infer(Held::Kept(&nothing, &[])), any);
        }
    }

    // an exchange's carry is the same, to the bit, on one thread, on two, and
    // on eight: its racks are cut into chunks of one size whatever the
    // threads, and the chunks' sums are added in their order (the racks here
    // fill more than two chunks)
    #[test]
    #[inline]
    fn an_exchanges_carry_is_the_same_on_any_number_of_threads() {
        let english = game_config::make_english_game_config();
        let kwg = tiny_kwg();
        let klv = klv::kbwg_from_bytes_alloc(klv::EMPTY_KLV_BYTES.to_vec()).unwrap();
        let board_tiles = board_of(&english, 70);
        let mut pool = english.tiles_off_board(&board_tiles).unwrap();
        english.take_rack(&mut pool, &[26]).unwrap();
        assert!(count_tallies_of_size(&pool, 7) > 2 * WEIGH_ACROSS_THREADS_FROM);
        let carry_on = |threads| {
            let mut leaves = PackedLeaves::new();
            let mut carried = Carried::default();
            kept_leaves_carried_into(
                &mut InferTables::new(&english, &klv).unwrap(),
                &mut movegen::KurniaMoveGenerator::new(&english),
                &movegen::BoardSnapshot {
                    board_tiles: &board_tiles,
                    game_config: &english,
                    kwg: &kwg,
                    anagrams: None,
                    rack_lengths: None,
                    klv: &klv,
                },
                &[26],
                (&exchange_of(3), Held::Any),
                InferOptions {
                    fuzz: FUZZ,
                    threads,
                    weigh_passes: true,
                },
                (&mut leaves, Some(&mut carried)),
            )
            .unwrap();
            assert!(!carried.is_empty());
            (
                (0..leaves.len())
                    .map(|i| (leaves.leave(i).to_vec(), leaves.weight(i).to_bits()))
                    .collect::<Vec<_>>(),
                carried.pool,
                carried.size,
                (carried.weights.iter())
                    .map(|weight| weight.to_bits())
                    .collect::<Vec<_>>(),
            )
        };
        let one = carry_on(1);
        assert_eq!(carry_on(2), one);
        assert_eq!(carry_on(8), one);
    }

    #[test]
    #[inline]
    fn the_sheet_and_generating_each_rack_weigh_racks_alike() {
        let game_config = game_config::make_english_game_config();
        let kwg = tiny_kwg();
        let alphabet = game_config.alphabet();
        let klv = klv::kbwg_from_bytes_alloc(klv::EMPTY_KLV_BYTES.to_vec()).unwrap();
        let mut board_tiles = vec![0u8; 15 * 15];
        let mut square = 0;
        for tile in 0..alphabet.len() {
            for _ in 0..alphabet.freq(tile) {
                if square < 79 {
                    board_tiles[square] = if tile == 0 { 0x81 } else { tile };
                    square += 1;
                }
            }
        }
        // one A moved to the middle, so that the unseen Ts have somewhere to
        // go
        board_tiles[7 * 15 + 7] = board_tiles[2];
        board_tiles.copy_within(3..79, 2);
        board_tiles[78] = 0;
        let board_snapshot = movegen::BoardSnapshot {
            board_tiles: &board_tiles,
            game_config: &game_config,
            kwg: &kwg,
            anagrams: None,
            rack_lengths: None,
            klv: &klv,
        };
        let mut pool = game_config.tiles_off_board(&board_tiles).unwrap();
        game_config.take_rack(&mut pool, &[26]).unwrap();
        let pool_size: usize = pool.iter().map(|&c| c as usize).sum();
        let mut tables = InferTables::new(&game_config, &klv).unwrap();
        let mut move_generator = movegen::KurniaMoveGenerator::new(&game_config);
        infer_sheet_for(
            &mut move_generator,
            &board_tiles,
            SpellTables {
                game_config: &game_config,
                kwg: &kwg,
                klv: &klv,
                lat: &tables.lat,
            },
            &pool,
            true,
            (&mut tables.movegen_rack, &mut tables.blank_deltas),
            &mut tables.sheet,
        )
        .unwrap();
        let ln_binom = LnBinom::new(pool_size);
        let is = InferSheet {
            lat: &tables.lat,
            sheet: &tables.sheet,
            leave: &tables.leave,
            ln_binom: &ln_binom,
        };
        let mut one_t = vec![0u8; pool.len()];
        one_t[20] = 1;
        let none = vec![0u8; pool.len()];
        let tiles = |which: usize| match which {
            0 => MoveTiles::Placed(&one_t),
            1 => MoveTiles::Placed(&none),
            _ => MoveTiles::Exchanged(which * 3 - 5),
        };
        for (which, value) in [(0, 2_000), (1, 0), (2, 0), (4, 0)] {
            let mut by_sheet = Vec::new();
            weigh_racks(
                &is,
                &pool,
                tiles(which),
                value,
                1,
                &mut WeighScratch::new(),
                &mut |rank, ln_prior, gap, kept| {
                    by_sheet.push((rank, ln_prior.to_bits(), gap, kept));
                },
            );
            assert!(by_sheet.iter().any(|&(_, _, gap, _)| gap != by_sheet[0].2));
            let mut by_generation = Vec::new();
            weigh_racks_by_generation::<true, _, _, _, true, true>(
                &mut move_generator,
                &board_snapshot,
                &is,
                &pool,
                ObservedMove {
                    tiles: tiles(which),
                    value,
                },
                &mut WeighScratch::new(),
                &mut |rank, ln_prior, gap, kept| {
                    by_generation.push((rank, ln_prior.to_bits(), gap, kept));
                },
            )
            .unwrap();
            assert_eq!(by_generation, by_sheet, "{which}");
        }
    }

    #[test]
    #[inline]
    fn each_prefix_of_size_cuts_the_tallies_where_it_is_told() {
        // the tallies of m tiles from a pool, walked prefix by prefix from any
        // letter k, are each_tally_of_size's in its order; k 0 is one prefix
        // (nothing yet, m to draw), k past the last letter is every tally with
        // nothing left to draw; and count_tallies_of_size counts them
        let avail = [3u8, 2, 2, 1];
        let mut cap = vec![0usize; avail.len() + 1];
        for t in (0..avail.len()).rev() {
            cap[t] = cap[t + 1] + avail[t] as usize;
        }
        for m in 0..=8 {
            let mut whole = Vec::new();
            let mut cur = vec![0u8; avail.len()];
            each_tally_of_size(&avail, &cap, 0, m, &mut cur, &mut |tally| {
                whole.push(tally.to_vec());
            });
            assert_eq!(whole.len(), count_tallies_of_size(&avail, m), "{m}");
            for k in 0..=avail.len() {
                cur.iter_mut().for_each(|c| *c = 0);
                let mut prefixes = Vec::new();
                each_prefix_of_size(&avail, &cap, 0, k, m, &mut cur, &mut |prefix, left| {
                    prefixes.push((prefix.to_vec(), left));
                });
                let mut by_prefix = Vec::new();
                for (prefix, left) in &prefixes {
                    cur[..k].copy_from_slice(prefix);
                    each_tally_of_size(&avail, &cap, k, *left, &mut cur, &mut |tally| {
                        by_prefix.push(tally.to_vec());
                    });
                }
                assert_eq!(by_prefix, whole, "{m} tiles from letter {k}");
                if k == 0 {
                    assert_eq!(prefixes, vec![(Vec::new(), m)]);
                } else if k == avail.len() {
                    assert_eq!(prefixes.len(), whole.len());
                    assert!(prefixes.iter().all(|(_, left)| *left == 0));
                }
            }
        }
    }

    #[test]
    #[inline]
    fn the_racks_weighed_on_threads_are_the_one_threads_in_its_order() {
        // a pool dealing more racks than WEIGH_ACROSS_THREADS_FROM: on 1, 3,
        // and 7 threads the same racks in the same order, bit for bit, for a
        // placed move, a pass, and an exchange
        let lat = census::MultisetLattice::new(9, 7).unwrap();
        let mut sheet = vec![0i32; lat.len()];
        let mut leave = vec![0i32; lat.len()];
        for (idx, (s, l)) in sheet.iter_mut().zip(leave.iter_mut()).enumerate() {
            let h = (idx as i32).wrapping_mul(2_654_435_761u32 as i32);
            *s = h.rem_euclid(30_000) - 10_000;
            *l = h.wrapping_mul(2_246_822_519u32 as i32).rem_euclid(8_000) - 4_000;
        }
        let pool = [9u8, 8, 8, 7, 7, 7, 6, 6, 5];
        let ln_binom = LnBinom::new(pool.iter().map(|&c| c as usize).sum());
        let is = InferSheet {
            lat: &lat,
            sheet: &sheet,
            leave: &leave,
            ln_binom: &ln_binom,
        };
        let placed = [0u8, 1, 0, 0, 0, 0, 1, 0, 0];
        let none = [0u8; 9];
        let tiles = |which: usize| match which {
            0 => MoveTiles::Placed(&placed[..]),
            1 => MoveTiles::Placed(&none[..]),
            _ => MoveTiles::Exchanged(3),
        };
        for (which, value, left) in [(0, 1_234, 5), (1, 0, 7), (2, 0, 7)] {
            let mut avail = pool;
            if which == 0 {
                for (a, &p) in avail.iter_mut().zip(&placed) {
                    *a -= p;
                }
            }
            let count = count_tallies_of_size(&avail, left);
            assert!(count >= WEIGH_ACROSS_THREADS_FROM, "{which}: {count}");
            let weighed_on = |threads: usize| {
                let mut got = Vec::new();
                weigh_racks(
                    &is,
                    &pool,
                    tiles(which),
                    value,
                    threads,
                    &mut WeighScratch::new(),
                    &mut |rank, ln_prior, gap, kept| {
                        got.push((rank, ln_prior.to_bits(), gap, kept));
                    },
                );
                got
            };
            let one = weighed_on(1);
            assert_eq!(one.len(), count, "{which}");
            assert_eq!(weighed_on(3), one, "{which} on 3 threads");
            assert_eq!(weighed_on(7), one, "{which} on 7 threads");
        }
    }

    // racks dealt 1 way worth 10 points with a bingo, and 3 ways worth 30
    // without: a mean of 25, a variance of 75, a bingo a quarter of the time
    #[test]
    #[inline]
    fn the_sums_give_a_replys_mean_variance_and_bingo_share() {
        let mut sums = ReplySums::default();
        for (deals, value, bingo) in [(1u64, 10_000i128, true), (3, 30_000, false)] {
            sums.add(&ReplySums {
                deals,
                equity: i128::from(deals) * value,
                squares: i128::from(deals) * value * value,
                bingos: if bingo { deals } else { 0 },
            });
        }
        assert_eq!(
            sums.reply(4),
            Reply {
                mean: 25.0,
                variance: 75.0,
                bingo: 0.25,
            }
        );
    }

    // a leave's weight is its racks' weights added in the order the racks come:
    // here each leave's first rack weighs 1 and its next three a little over
    // half of 1's last place each, so that in that order each lifts the sum by a
    // place, where the small ones first would lift it by two in all; a rack of
    // weight 0 (at fuzz 0, past the smallest gap) adds no leave, a gap below 0
    // counts as 0, and the leaves come in the lattice's order
    #[test]
    #[inline]
    fn a_leaves_weight_adds_its_racks_weights_in_the_order_they_come() {
        let small = (0.75 * f64::EPSILON).ln();
        let mut racks = Vec::new();
        for round in 0..4 {
            for leave in (0..32u32).rev() {
                racks.push((leave, 0, if round == 0 { 0.0 } else { small }));
            }
        }
        racks.push((40, 5, 0.0));
        racks.push((50, -3, 0.0));
        let s = small.exp();
        let want = ((1.0 + s) + s) + s;
        assert_ne!(want, ((s + s) + s) + 1.0);
        let mut got = Vec::new();
        weights_by_leave(&racks, 0.0, &mut got);
        let got = (got.into_iter())
            .map(|(leave, _, weight)| (leave, weight))
            .collect::<Vec<_>>();
        let mut expected = (0..32u32).map(|leave| (leave, want)).collect::<Vec<_>>();
        expected.push((50, 1.0));
        assert_eq!(got, expected);
    }

    // a pass keeps the whole rack, and at a fuzz too large for any gap to count
    // each rack weighs its chance to be dealt: every rack of the 20 unseen
    // tiles, each weighing the ways it is dealt over all the deals
    #[test]
    #[inline]
    fn a_pass_at_a_vast_fuzz_weighs_each_rack_by_its_deals() {
        let pass = movegen::Play::Exchange {
            tiles: [][..].into(),
        };
        let leaves = leaves_after(79, &pass, 1e300);
        // the pool: every tile but the board's first 79 and the watcher's Z
        let game_config = game_config::make_english_game_config();
        let alphabet = game_config.alphabet();
        let mut pool = (0..alphabet.len())
            .map(|tile| alphabet.freq(tile) as usize)
            .collect::<Vec<_>>();
        let mut on_board = 79;
        for count in pool.iter_mut() {
            let taken = (*count).min(on_board);
            *count -= taken;
            on_board -= taken;
        }
        pool[26] -= 1;
        assert_eq!(pool.iter().sum::<usize>(), 20);
        let deals = binom(20, 7) as f64;
        let mut ways_in_all = 0;
        for (leave, weight) in &leaves {
            assert_eq!(leave.len(), 7);
            let mut tally = vec![0usize; pool.len()];
            for &tile in leave {
                tally[tile as usize] += 1;
            }
            let ways = (pool.iter().zip(&tally))
                .map(|(&u, &r)| binom(u, r))
                .product::<u64>();
            ways_in_all += ways;
            let want = ways as f64 / deals;
            assert!(
                (weight - want).abs() <= want * 1e-12,
                "{leave:?}: {weight} against {want}"
            );
        }
        assert_eq!(ways_in_all, binom(20, 7));
    }

    // packed leaves keep each leave and weight in the order pushed (the empty
    // leave, the blank, more tiles than a rack, any tile byte at all), refuse a
    // weight below 0, infinite, or not a number, and draw the first leave whose
    // running sum passes the draw, never one of weight 0
    #[test]
    #[inline]
    fn packed_leaves_keep_their_order_and_draw_by_weight() {
        let pushed: [(&[u8], f64); 4] = [
            (&[], 1.5),
            (&[0, 0, 5], 0.0),
            (&[63, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10], 2.0),
            (&[u8::MAX], 0.5),
        ];
        let mut packed = PackedLeaves::new();
        assert!(packed.is_empty());
        for (tiles, weight) in pushed {
            packed.push(tiles, weight).unwrap();
        }
        assert_eq!(packed.len(), 4);
        for (i, (tiles, weight)) in pushed.iter().enumerate() {
            assert_eq!(packed.leave(i), *tiles);
            assert_eq!(packed.weight(i), *weight);
        }
        // running sums 1.5, 1.5, 3.5, 4
        assert_eq!(packed.total(), 4.0);
        for (at, i) in [
            (0.0, 0),
            (1.499, 0),
            (1.5, 2),
            (3.499, 2),
            (3.5, 3),
            (3.999, 3),
        ] {
            assert_eq!(packed.pick(at), pushed[i].0, "{at}");
        }
        for wrong in [-1.0, f64::INFINITY, f64::NAN] {
            assert!(packed.push(&[1], wrong).is_err(), "{wrong}");
        }
        assert_eq!(packed.len(), 4);
        // made to sum to 1: running sums 0.375, 0.375, 0.875, 1
        packed.normalize();
        assert_eq!(packed.weight(2), 0.5);
        assert_eq!(packed.total(), 1.0);
        assert_eq!(packed.pick(0.375), pushed[2].0);
        // a push after normalizing runs on from the normalized sum
        packed.push(&[2], 1.0).unwrap();
        assert_eq!(packed.total(), 2.0);
        assert_eq!(packed.pick(1.5), [2]);
        // a weight that would take the sum past finite is refused, the list as
        // it was
        assert!(packed.push(&[3], f64::MAX).is_ok());
        assert!(packed.push(&[3], f64::MAX).is_err());
        assert_eq!(packed.len(), 6);
        assert!(packed.total().is_finite());
        // a list of none picks no tiles
        assert!(PackedLeaves::default().pick(0.0).is_empty());
    }

    // packed leaves reserve room the allocator can give, and are refused room
    // past what any can, the leaves as they were
    #[test]
    #[inline]
    fn packed_leaves_reserve_room_or_are_refused() {
        let mut packed = PackedLeaves::new();
        packed.push(&[1, 2], 1.0).unwrap();
        packed.try_reserve(3, 12).unwrap();
        assert!(packed.try_reserve(usize::MAX, 0).is_err());
        assert!(packed.try_reserve(0, usize::MAX).is_err());
        assert_eq!(packed.len(), 1);
        assert_eq!(packed.leave(0), [1, 2]);
        assert_eq!(packed.total(), 1.0);
    }

    #[test]
    #[inline]
    fn binom_counts_exactly() {
        assert_eq!(binom(5, 2), 10);
        assert_eq!(binom(0, 0), 1);
        assert_eq!(binom(3, 5), 0);
        assert_eq!(binom(100, 7), 16_007_560_800);
    }

    // every word of two or three tiles from A, E, and T, and one of ten that
    // starts TEA, the one word that plays a whole rack
    #[inline]
    fn reply_kwg() -> kwg::Kwg<kwg::Node24> {
        const LETTERS: [u8; 3] = [1, 5, 20];
        let mut words = vec![[20u8, 5, 1, 1, 5, 20, 1, 5, 20, 1][..].into()];
        for &a in &LETTERS {
            for &b in &LETTERS {
                words.push([a, b][..].into());
                for &c in &LETTERS {
                    words.push([a, b, c][..].into());
                }
            }
        }
        words.sort_unstable();
        let bytes = build::build(
            build::BuildContent::Gaddawg,
            build::BuildLayout::Wolges,
            build::BuildOrder::Sorted,
            &words,
        )
        .unwrap();
        kwg::kbwg_from_bytes_alloc(bytes.to_vec(), false).unwrap()
    }

    // a board holding every tile but the pool's and the mover's: TEA across row
    // 11 from its first column, and the rest filling the rows from the top (a
    // blank as A), too far from TEA for a move near it to touch
    #[inline]
    fn reply_board(game_config: &game_config::GameConfig, pool: &[u8], mover: &[u8]) -> Vec<u8> {
        let alphabet = game_config.alphabet();
        let mut rest = (0..alphabet.len())
            .map(|tile| alphabet.freq(tile))
            .collect::<Vec<_>>();
        for (r, &p) in rest.iter_mut().zip(pool) {
            *r -= p;
        }
        for &tile in mover {
            rest[tile as usize] -= 1;
        }
        let mut board_tiles = vec![0u8; 15 * 15];
        for (col, tile) in [20u8, 5, 1].into_iter().enumerate() {
            board_tiles[11 * 15 + col] = tile;
            rest[tile as usize] -= 1;
        }
        let mut square = 0;
        for (tile, &count) in (0u8..).zip(&rest) {
            for _ in 0..count {
                board_tiles[square] = if tile == 0 { 0x81 } else { tile };
                square += 1;
            }
        }
        assert!(square <= 6 * 15, "the rest reach row 6");
        board_tiles
    }

    // every full rack the pool deals, as a tally, with what generating its
    // moves on the board after the move finds: its best move's equity (a pass
    // only when forced), and whether a move plays it whole
    #[inline]
    fn movegen_replies<N: kwg::Node, L: kwg::Node>(
        board_snapshot: &movegen::BoardSnapshot<'_, N, L>,
        pool: &[u8],
    ) -> Vec<(Vec<u8>, i32, bool)> {
        let rack_size = board_snapshot.game_config.rack_size() as usize;
        let mut cap = vec![0usize; pool.len() + 1];
        for t in (0..pool.len()).rev() {
            cap[t] = cap[t + 1] + pool[t] as usize;
        }
        let mut move_generator = movegen::KurniaMoveGenerator::new(board_snapshot.game_config);
        let mut replies = Vec::new();
        let mut cur = vec![0u8; pool.len()];
        each_tally_of_size(pool, &cap, 0, rack_size, &mut cur, &mut |tally| {
            let rack = (0u8..)
                .zip(tally)
                .flat_map(|(tile, &count)| std::iter::repeat_n(tile, count as usize))
                .collect::<Vec<_>>();
            move_generator
                .gen_moves_unfiltered(&movegen::GenMovesParams {
                    board_snapshot,
                    rack: &rack,
                    max_gen: 1 << 20,
                    num_exchanges_by_this_player: 0,
                    pass_policy: movegen::PassPolicy::OnlyWhenForced,
                    dynamic_leaves: None,
                })
                .unwrap();
            let plays = &move_generator.plays;
            let best = plays
                .iter()
                .map(|valued| valued.equity.raw())
                .max()
                .unwrap();
            let bingo = plays.iter().any(|valued| {
                matches!(&valued.play, movegen::Play::Place { word, .. }
                    if word.iter().filter(|&&tile| tile != 0).count() == rack_size)
            });
            replies.push((tally.to_vec(), best, bingo));
        });
        replies
    }

    // those racks' reply, each counted as often as the pool deals it
    #[inline]
    fn reply_by_deals(pool: &[u8], replies: &[(Vec<u8>, i32, bool)]) -> Reply {
        let mut sums = ReplySums::default();
        for (tally, best, bingo) in replies {
            let deals = (tally.iter().zip(pool))
                .map(|(&r, &u)| binom(u as usize, r as usize))
                .product::<u64>();
            sums.deals += deals;
            sums.equity += i128::from(deals) * i128::from(*best);
            sums.squares += i128::from(deals) * (i128::from(*best) * i128::from(*best));
            if *bingo {
                sums.bingos += deals;
            }
        }
        sums.reply(sums.deals)
    }

    // those racks' reply when the rack is a kept leave, by its weight, drawn to
    // full from the rest of the pool: each rack weighed by every leave it
    // holds, times the chance of drawing the rest
    #[inline]
    fn reply_by_kept(
        pool: &[u8],
        replies: &[(Vec<u8>, i32, bool)],
        kept: &[(Vec<u8>, f64)],
    ) -> Reply {
        let rack_size = replies[0].0.iter().map(|&c| c as usize).sum::<usize>();
        let pool_size = pool.iter().map(|&c| c as usize).sum::<usize>();
        let (mut weight, mut equity, mut squares, mut bingos) = (0.0, 0.0, 0.0, 0.0);
        for (tally, best, bingo) in replies {
            let mut chance = 0.0;
            for (tiles, w) in kept {
                let mut leave = vec![0u8; pool.len()];
                for &tile in tiles {
                    leave[tile as usize] += 1;
                }
                if leave.iter().zip(tally).any(|(&l, &r)| l > r) {
                    continue;
                }
                let draws = (0..pool.len())
                    .map(|t| {
                        binom(
                            (pool[t] - leave[t]) as usize,
                            (tally[t] - leave[t]) as usize,
                        )
                    })
                    .product::<u64>();
                chance += w * draws as f64
                    / binom(pool_size - tiles.len(), rack_size - tiles.len()) as f64;
            }
            let value = f64::from(*best);
            weight += chance;
            equity += chance * value;
            squares += chance * value * value;
            if *bingo {
                bingos += chance;
            }
        }
        let scale = f64::from(equity::SCALE);
        let mean = equity / weight;
        Reply {
            mean: mean / scale,
            variance: (squares / weight - mean * mean) / (scale * scale),
            bingo: bingos / weight,
        }
    }

    // a reply within a rounding of another
    #[inline]
    fn about(a: &Reply, b: &Reply) -> bool {
        let near = |x: f64, y: f64| (x - y).abs() <= 1e-9 * x.abs().max(y.abs()).max(1.0);
        near(a.mean, b.mean) && near(a.variance, b.variance) && near(a.bingo, b.bingo)
    }

    // the reply to a move is what generating every rack's moves finds, the
    // racks weighed by the ways the pool deals them, on threads: with
    // exchanges and a bingo in reach, after a placement; with an exchange left
    // only before the mover draws, after a pass or an exchange; and with racks
    // that can only pass, one of them a leave the table values. None when the
    // bag is empty after the mover draws, or the pool holds less than a rack
    #[test]
    #[inline]
    fn a_reply_is_every_racks_best_move_weighed_by_its_deals() {
        let game_config = game_config::make_english_game_config();
        let kwg = reply_kwg();
        let klv = klv::kbwg_from_bytes_alloc(klv::make_klv2(&[
            (&[0], 25.0),
            (&[1], 1.5),
            (&[5], 2.0),
            (&[10, 11, 17, 22, 23, 24, 25], -12.0),
            (&[17], -7.0),
            (&[19], 8.0),
            (&[19, 19], 10.0),
            (&[20], 0.5),
            (&[26], 3.0),
        ]))
        .unwrap();
        let mut tables = InferTables::new(&game_config, &klv).unwrap();
        let mut move_generator = movegen::KurniaMoveGenerator::new(&game_config);
        let mover = [1u8, 5, 20, 18, 9, 14, 15];
        let tally = |counts: &[(u8, u8)]| {
            let mut tally = vec![0u8; game_config.alphabet().len() as usize];
            for &(tile, count) in counts {
                tally[tile as usize] = count;
            }
            tally
        };
        // 23 tiles: a bag of 16, exchanges left after any move
        let big = tally(&[
            (0, 1),
            (1, 4),
            (2, 2),
            (3, 2),
            (4, 2),
            (5, 3),
            (6, 1),
            (17, 1),
            (19, 2),
            (20, 3),
            (24, 1),
            (26, 1),
        ]);
        // 14 tiles: a bag of 7, so an exchange only before the mover draws
        let mid = tally(&[
            (0, 1),
            (1, 3),
            (2, 1),
            (5, 2),
            (17, 1),
            (19, 2),
            (20, 2),
            (24, 1),
            (26, 1),
        ]);
        // 10 tiles: a bag of 3, no exchange
        let small = tally(&[(0, 1), (1, 3), (5, 2), (19, 1), (20, 2), (26, 1)]);
        // 9 tiles, one an A: a rack without it can only pass, and the leave of
        // the seven that are neither the A nor the Z is valued
        let dead = tally(&[
            (1, 1),
            (10, 1),
            (11, 1),
            (17, 1),
            (22, 1),
            (23, 1),
            (24, 1),
            (25, 1),
            (26, 1),
        ]);
        // an A under TEA's T, making TA
        let ta = movegen::Play::Place {
            down: true,
            lane: 0,
            idx: 11,
            word: [0, 1][..].into(),
            score: equity::Equity::new(2),
        };
        let pass = movegen::Play::Exchange {
            tiles: [][..].into(),
        };
        let exchange = movegen::Play::Exchange {
            tiles: [18, 9][..].into(),
        };
        let reply_on = |tables: &mut InferTables,
                        move_generator: &mut movegen::KurniaMoveGenerator,
                        board_tiles: &[u8],
                        kept: &[(Vec<u8>, f64)],
                        play: &movegen::Play,
                        threads: usize| {
            reply_to(
                tables,
                move_generator,
                &movegen::BoardSnapshot {
                    board_tiles,
                    game_config: &game_config,
                    kwg: &kwg,
                    anagrams: None,
                    rack_lengths: None,
                    klv: &klv,
                },
                &mover,
                kept,
                play,
                threads,
            )
        };
        // what the opponent's last move kept: two tiles, one, none, or a whole
        // rack, each with its weight; and for the dead pool, its A or JK
        let kept_big = [
            (vec![1, 5], 2.0),
            (vec![20], 1.0),
            (vec![], 0.5),
            (vec![0, 1, 1, 5, 5, 20, 20], 0.25),
        ];
        let kept_dead = [(vec![1], 3.0), (vec![10, 11], 1.0)];
        for (name, pool, play, bingo, kept) in [
            ("big, TA", &big, &ta, true, &kept_big[..]),
            ("mid, a pass", &mid, &pass, true, &kept_big[1..3]),
            ("mid, an exchange", &mid, &exchange, true, &kept_big[1..3]),
            ("dead, TA", &dead, &ta, false, &kept_dead),
        ] {
            let board_tiles = reply_board(&game_config, pool, &mover);
            let mut after = board_tiles.clone();
            if let movegen::Play::Place { .. } = play {
                after[12 * 15] = 1;
            }
            let replies = movegen_replies(
                &movegen::BoardSnapshot {
                    board_tiles: &after,
                    game_config: &game_config,
                    kwg: &kwg,
                    anagrams: None,
                    rack_lengths: None,
                    klv: &klv,
                },
                pool,
            );
            let want = reply_by_deals(pool, &replies);
            assert_eq!(want.bingo > 0.0, bingo, "[{name}] a rack plays whole");
            let want_kept = reply_by_kept(pool, &replies, kept);
            let mut on = |kept| {
                reply_on(
                    &mut tables,
                    &mut move_generator,
                    &board_tiles,
                    kept,
                    play,
                    4,
                )
                .unwrap()
                .unwrap()
            };
            assert_eq!(on(&[]), want, "[{name}]");
            let got = on(kept);
            assert!(
                about(&got, &want_kept),
                "[{name}] kept: {got:?}, not {want_kept:?}"
            );
        }
        // a kept leave the pool does not hold, or one of no weight, is
        // refused; so are leaves weighing nothing at all
        let board_tiles = reply_board(&game_config, &big, &mover);
        for kept in [
            &[(vec![18], 1.0)][..],
            &[(vec![1], f64::NAN)],
            &[(vec![1], 0.0)],
        ] {
            assert!(
                reply_on(&mut tables, &mut move_generator, &board_tiles, kept, &ta, 1).is_err(),
                "{kept:?}"
            );
        }
        // a blank the mover does not hold
        let blank_ta = movegen::Play::Place {
            down: true,
            lane: 0,
            idx: 11,
            word: [0, 0x81][..].into(),
            score: equity::Equity::ZERO,
        };
        assert!(
            reply_on(
                &mut tables,
                &mut move_generator,
                &board_tiles,
                &[],
                &blank_ta,
                1
            )
            .is_err()
        );
        // an exchange from a bag too small for one
        let board_tiles = reply_board(&game_config, &small, &mover);
        assert!(
            reply_on(
                &mut tables,
                &mut move_generator,
                &board_tiles,
                &[],
                &exchange,
                1
            )
            .is_err()
        );
        // the mover's draw empties the bag: three placed, three in it
        let tae = movegen::Play::Place {
            down: true,
            lane: 0,
            idx: 11,
            word: [0, 1, 5, 20][..].into(),
            score: equity::Equity::new(0),
        };
        let board_tiles = reply_board(&game_config, &small, &mover);
        assert_eq!(
            reply_on(&mut tables, &mut move_generator, &board_tiles, &[], &tae, 1).unwrap(),
            None
        );
        // the pool holds less than a rack
        let six = tally(&[(1, 3), (5, 2), (20, 1)]);
        let board_tiles = reply_board(&game_config, &six, &mover);
        assert_eq!(
            reply_on(
                &mut tables,
                &mut move_generator,
                &board_tiles,
                &[],
                &pass,
                1
            )
            .unwrap(),
            None
        );
    }

    #[test]
    #[inline]
    fn a_woadwage_sheet_holds_each_racks_brute_force_best() {
        use crate::movegen::tests::{
            WoadwageToyRules, woadwage_brute_keys, woadwage_key_play, woadwage_rng,
            woadwage_toy_boards, woadwage_toy_rack, woadwage_toy_rules,
        };
        let klv = klv::kbwg_from_bytes_alloc(klv::EMPTY_KLV_BYTES.to_vec()).unwrap();
        let mut seed = 407u64;
        let (mut racks_checked, mut played, mut inferred) = (0usize, 0usize, 0usize);
        for jumbled in [false, true] {
            for side in [5i8, 7] {
                let WoadwageToyRules(mut gc, kwg, explicit) = woadwage_toy_rules(side, jumbled);
                let game_config::GameConfig::Static(rules) = &mut gc;
                rules.rack_size = 3;
                let gc = gc;
                let lat = census::MultisetLattice::new(27, 3).unwrap();
                let mut scorer = play_scorer::PlayScorer::new();
                let mut move_generator = movegen::KurniaMoveGenerator::new(&gc);
                for board_tiles in woadwage_toy_boards(side, &explicit, &mut seed, 2, 4) {
                    let mut pool = vec![0u8; 27];
                    pool[0] = (woadwage_rng(&mut seed) % 2) as u8;
                    for count in &mut pool[1..5] {
                        *count = (woadwage_rng(&mut seed) % 3) as u8;
                    }
                    let mut sheet = Vec::new();
                    infer_sheet_for(
                        &mut move_generator,
                        &board_tiles,
                        SpellTables {
                            game_config: &gc,
                            kwg: &kwg,
                            klv: &klv,
                            lat: &lat,
                        },
                        &pool,
                        true,
                        (&mut Vec::new(), &mut Vec::new()),
                        &mut sheet,
                    )
                    .unwrap();
                    let board_snapshot = movegen::BoardSnapshot {
                        board_tiles: &board_tiles,
                        game_config: &gc,
                        kwg: &kwg,
                        anagrams: None,
                        rack_lengths: None,
                        klv: &klv,
                    };
                    let phase = BoardPhase::of_board(&board_tiles, &gc);
                    let mut tally = vec![0u8; 27];
                    let mut each = |tally: &[u8]| {
                        let rack = (0u8..)
                            .zip(tally.iter())
                            .flat_map(|(tile, &count)| std::iter::repeat_n(tile, count as usize))
                            .collect::<Vec<_>>();
                        let mut want = 0;
                        for key in woadwage_brute_keys(side, &board_tiles, &rack, &explicit) {
                            if key.3.iter().filter(|&&tile| tile != 0).count() != rack.len() {
                                continue;
                            }
                            let play = woadwage_key_play(side, &board_tiles, &key);
                            let movegen::Play::Place {
                                down, idx, word, ..
                            } = &play
                            else {
                                unreachable!();
                            };
                            let score = scorer.compute_score(&board_snapshot, &play)
                                + opening_adjustment(phase, &gc, *down, *idx, word).unwrap();
                            want = want.max(score);
                            played += 1;
                        }
                        let got = sheet[lat.rank(tally).unwrap() as usize];
                        assert_eq!(got, want, "{jumbled} {side} {rack:?} {board_tiles:?}");
                        racks_checked += 1;
                    };
                    fn walk(
                        tile: usize,
                        left: usize,
                        pool: &[u8],
                        tally: &mut [u8],
                        each: &mut impl FnMut(&[u8]),
                    ) {
                        if tile == 5 {
                            if left < 3 {
                                each(tally);
                            }
                            return;
                        }
                        for count in 0..=(pool[tile] as usize).min(left) {
                            tally[tile] = count as u8;
                            walk(tile + 1, left - count, pool, tally, each);
                        }
                        tally[tile] = 0;
                    }
                    walk(0, 3, &pool, &mut tally, &mut each);
                    let best_of = |rack: &[u8], scorer: &mut play_scorer::PlayScorer| {
                        woadwage_brute_keys(side, &board_tiles, rack, &explicit)
                            .iter()
                            .map(|key| {
                                let play = woadwage_key_play(side, &board_tiles, key);
                                let movegen::Play::Place {
                                    down, idx, word, ..
                                } = &play
                                else {
                                    unreachable!();
                                };
                                let value = scorer.compute_score(&board_snapshot, &play)
                                    + opening_adjustment(phase, &gc, *down, *idx, word).unwrap();
                                (value, play)
                            })
                            .max_by_key(|(value, _)| *value)
                    };
                    let rack =
                        woadwage_toy_rack(&mut seed, crate::movegen::tests::woadwage_toy_longest());
                    let Some((value, mut play)) = best_of(&rack, &mut scorer) else {
                        continue;
                    };
                    let scored = scorer.compute_score(&board_snapshot, &play);
                    let movegen::Play::Place { word, score, .. } = &mut play else {
                        unreachable!();
                    };
                    *score = equity::Equity::new(scored);
                    let placed = word
                        .iter()
                        .filter(|&&tile| tile != 0)
                        .map(|&tile| if tile & 0x80 != 0 { 0 } else { tile })
                        .collect::<Vec<_>>();
                    let Ok(mut unseen) = gc.tiles_off_board(&board_tiles) else {
                        continue;
                    };
                    if gc.take_rack(&mut unseen, &placed).is_err() {
                        continue;
                    }
                    let mut tables = InferTables::new(&gc, &klv).unwrap();
                    let leaves = kept_leaves(
                        &mut tables,
                        &mut move_generator,
                        &board_snapshot,
                        &[],
                        &play,
                        0.0,
                        1,
                    )
                    .unwrap();
                    for (leave, _) in &leaves {
                        let mut held = leave.clone();
                        held.extend_from_slice(&placed);
                        assert!(held.len() <= 3, "{leave:?} {placed:?}");
                        let best = best_of(&held, &mut scorer);
                        assert_eq!(best.map(|(best, _)| best), Some(value), "{held:?}");
                        inferred += 1;
                    }
                }
            }
        }
        assert!(inferred > 0, "{inferred}");
        assert!(racks_checked > 0, "{racks_checked}");
        assert!(played > 0, "{played}");
    }
}
