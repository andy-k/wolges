// Copyright (C) 2020-2026 Andy Kurnia.

// inferring the rack behind a move: every rack the unseen tiles could have
// dealt the mover, weighted by its chance of being dealt and by how far the
// move falls short of that rack's best play, valued the way the census sheet
// values a board

use super::{census, equity, error, game_config, klv, kwg, movegen, play_scorer};

// how far, in points of equity, a sim takes the opponent's last move to fall
// short of their best when it weighs the racks that could have made it
pub const FUZZ: f64 = 3.105;

#[inline]
pub fn bag_size(board_tiles: &[u8], game_config: &game_config::GameConfig) -> i64 {
    let on_board = board_tiles.iter().filter(|&&t| t != 0).count() as i64;
    game_config.alphabet().num_tiles() as i64
        - (on_board + game_config.num_players() as i64 * game_config.rack_size() as i64)
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
    // the snapshot holds no tables, so the lean arm alone runs it
    move_generator.gen_census_sheet_lean(
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
    // ways[k] is the tallies of k tiles from the letters read so far; a letter
    // adds its tallies with c of it, c from 1, to those without
    let mut ways = vec![0usize; m + 1];
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
    f: &mut impl FnMut(u32, f64, i32, u32),
) {
    // a wasm build has no threads
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
    let mut avail = pool.to_vec();
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
        MoveTiles::Exchanged(_) => (&none_placed[..], rack_size),
    };
    let passed = matches!(tiles, MoveTiles::Placed(placed) if placed.iter().all(|&c| c == 0));
    let mut cap = vec![0usize; num_letters + 1];
    for t in (0..num_letters).rev() {
        cap[t] = cap[t + 1] + avail[t] as usize;
    }
    // one rack, from the tiles it drew (rack is its scratch), to what f takes
    let weigh = |drawn: &[u8], rack: &mut [u8]| -> (u32, f64, i32, u32) {
        for (r, (&d, &p)) in rack.iter_mut().zip(drawn.iter().zip(placed)) {
            *r = d + p;
        }
        let rank = lat.rank(rack);
        let mut best = census::best_equity_of(lat, is.sheet, is.leave, rack);
        if passed {
            best = best.max(is.leave[rank as usize]);
        }
        // exchanging n tiles scores 0 and keeps the best leave of the rest
        let (value, kept) = match tiles {
            MoveTiles::Placed(_) => {
                let kept = lat.rank(drawn);
                (move_value + is.leave[kept as usize], kept)
            }
            MoveTiles::Exchanged(n) => {
                census::best_keep_of(lat, is.leave, rack, rack_size.saturating_sub(n))
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
    if threads <= 1 || count_tallies_of_size(&avail, left) < WEIGH_ACROSS_THREADS_FROM {
        let mut cur = vec![0u8; num_letters];
        let mut rack = vec![0u8; num_letters];
        each_tally_of_size(&avail, &cap, 0, left, &mut cur, &mut |drawn| {
            let (rank, ln_prior, gap, kept) = weigh(drawn, &mut rack);
            f(rank, ln_prior, gap, kept);
        });
        return;
    }
    // the racks in chunks by the counts of their first k letters, k the least
    // that gives chunks enough (or every letter); each thread takes the next
    // chunk and weighs its racks in order, and the chunks are handed on in
    // theirs, which is the one thread's
    let mut k = 1;
    let mut prefixes: Vec<(Vec<u8>, usize)> = Vec::new();
    loop {
        prefixes.clear();
        let mut cur = vec![0u8; num_letters];
        each_prefix_of_size(&avail, &cap, 0, k, left, &mut cur, &mut |prefix, left| {
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
                        each_tally_of_size(&avail, &cap, k, *left, &mut cur, &mut |drawn| {
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
    tiles: MoveTiles<'_>,
    move_value: i32,
    f: &mut impl FnMut(u32, f64, i32, u32),
) {
    let (lat, leave, ln_binom) = (is.lat, is.leave, is.ln_binom);
    let num_letters = lat.num_letters();
    let rack_size = lat.rack_size();
    let pool_size: usize = pool.iter().map(|&c| c as usize).sum();
    let ln_deals = ln_binom.ln_binom(pool_size, rack_size);
    let mut avail = pool.to_vec();
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
        MoveTiles::Exchanged(_) => (&none_placed[..], rack_size),
    };
    let passed = matches!(tiles, MoveTiles::Placed(placed) if placed.iter().all(|&c| c == 0));
    let mut cap = vec![0usize; num_letters + 1];
    for t in (0..num_letters).rev() {
        cap[t] = cap[t + 1] + avail[t] as usize;
    }
    let mut cur = vec![0u8; num_letters];
    let mut rack = vec![0u8; num_letters];
    let mut rack_tiles = Vec::with_capacity(rack_size);
    each_tally_of_size(&avail, &cap, 0, left, &mut cur, &mut |drawn| {
        for (r, (&d, &p)) in rack.iter_mut().zip(drawn.iter().zip(placed)) {
            *r = d + p;
        }
        let rank = lat.rank(&rack);
        rack_tiles.clear();
        rack_tiles.extend(
            (0u8..)
                .zip(rack.iter())
                .flat_map(|(tile, &count)| std::iter::repeat_n(tile, count as usize)),
        );
        let params = movegen::GenMovesParams {
            board_snapshot,
            rack: &rack_tiles,
            max_gen: 1,
            num_exchanges_by_this_player: 0,
            pass_policy: movegen::PassPolicy::OnlyWhenForced,
            dynamic_leaves: None,
        };
        if LEAN {
            move_generator.gen_moves_unfiltered_lean(&params);
        } else {
            move_generator.gen_moves_unfiltered(&params);
        }
        let top = &move_generator.plays[0];
        let mut best = match &top.play {
            movegen::Play::Exchange { tiles } if tiles.is_empty() => {
                i32::MIN / 4
                    + (0..=rack_tiles.len())
                        .map(|keep| census::best_keep_of(lat, leave, &rack, keep).0)
                        .max()
                        .unwrap_or(census::UNPLAYABLE)
            }
            _ => top.equity.raw(),
        };
        if passed {
            best = best.max(leave[rank as usize]);
        }
        // exchanging n tiles scores 0 and keeps the best leave of the rest
        let (value, kept) = match tiles {
            MoveTiles::Placed(_) => {
                let kept = lat.rank(drawn);
                (move_value + leave[kept as usize], kept)
            }
            MoveTiles::Exchanged(n) => {
                census::best_keep_of(lat, leave, &rack, rack_size.saturating_sub(n))
            }
        };
        let ln_prior = rack
            .iter()
            .zip(pool)
            .map(|(&r, &u)| ln_binom.ln_binom(u as usize, r as usize))
            .sum::<f64>()
            - ln_deals;
        f(rank, ln_prior, best - value, kept);
    });
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
pub fn move_tiles_and_value<N: kwg::Node, L: kwg::Node>(
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

// fuzz, given in points, in the equity's own units. 0 is the exact limit, so
// only NaN and a fuzz below 0 are refused.
#[inline]
pub fn equity_fuzz(fuzz: f64) -> error::Returns<f64> {
    if fuzz.is_nan() || fuzz < 0.0 {
        return_error!(format!("fuzz {fuzz} is not 0 or more"));
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
pub fn fuzz_ln_weight(gap: i32, g0: i32, fuzz: f64) -> f64 {
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
// 0 (fuzz 0 keeps only the racks closest to their best) adds nothing. The racks'
// own list holds the work: each weighed in place, sorted by leave (stably, so a
// leave's racks keep their order), and each leave's run summed into its first
#[inline]
fn weights_by_leave(racks: Vec<(u32, i32, f64)>, fuzz: f64) -> Vec<(u32, f64)> {
    let g0 = racks
        .iter()
        .map(|&(_, gap, _)| gap.max(0))
        .min()
        .unwrap_or(0);
    let mut weighed = (racks.into_iter())
        .filter_map(|(kept, gap, ln_prior)| {
            let weight = ln_prior.exp() * fuzz_ln_weight(gap, g0, fuzz).exp();
            (weight > 0.0).then_some((kept, weight))
        })
        .collect::<Vec<_>>();
    weighed.sort_by_key(|&(kept, _)| kept);
    weighed.dedup_by(|later, first| {
        let same = later.0 == first.0;
        if same {
            first.1 += later.1;
        }
        same
    });
    weighed
}

// the multiset lattice and its leave table, built by one inference and reused
// by the next, and the scratch space an inference works in
pub struct InferTables {
    lat: census::MultisetLattice,
    leave: Vec<i32>,
    sheet: Vec<i32>,
    movegen_rack: Vec<u8>,
    blank_deltas: Vec<(u8, i32)>,
}

impl InferTables {
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
            sheet: Vec::new(),
            movegen_rack: Vec::new(),
            blank_deltas: Vec::new(),
        })
    }
}

// leaves with their weights, packed for lists of millions, with no allocation
// for each leave: every leave's tiles one after another, where each one ends,
// the weights in the same order, and their running sums, which a weighted draw
// searches. No weight is below 0, past f64's finite range, or not a number
#[derive(Default)]
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

// the leaves the mover could have kept, inferred from their move, each with its
// weight (not normalized). The board is the one before the move, and the pool
// is what the watcher could not see then: the tiles not on that board, less the
// rack the watcher still holds. A rack keeps the tiles it did not place: all of
// them for a pass, and for an exchange (whose size alone is seen) the leave of
// that size the table values most. Empty when there is nothing to weigh: the
// bag is empty before the move, or its own draw empties it (the mover then
// holds every unseen tile, whatever they kept), or no rack from the pool could
// have made the move. The racks are weighed on this many threads.
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
    let leaves = kept_leaves_packed_impl::<LEAN, _, _, _, JUMPS, ANY_SIDE>(
        tables,
        move_generator,
        board_snapshot,
        watcher_rack,
        play,
        fuzz,
        threads,
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
    kept_leaves_packed_impl::<false, _, _, _, JUMPS, ANY_SIDE>(
        tables,
        move_generator,
        board_snapshot,
        watcher_rack,
        play,
        fuzz,
        threads,
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
    fuzz: f64,
    threads: usize,
) -> error::Returns<PackedLeaves> {
    kept_leaves_packed_impl::<true, _, _, _, JUMPS, ANY_SIDE>(
        tables,
        move_generator,
        board_snapshot,
        watcher_rack,
        play,
        fuzz,
        threads,
    )
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
    play: &movegen::Play,
    fuzz: f64,
    threads: usize,
) -> error::Returns<PackedLeaves> {
    let game_config = board_snapshot.game_config;
    let board_tiles = board_snapshot.board_tiles;
    let num_letters = game_config.alphabet().len() as usize;
    let rack_size = game_config.rack_size() as usize;
    let fuzz = equity_fuzz(fuzz)?;
    let mut pool = game_config.tiles_off_board(board_tiles)?;
    game_config.take_rack(&mut pool, watcher_rack)?;
    let pool_size: usize = pool.iter().map(|&c| c as usize).sum();
    if pool_size <= rack_size {
        return Ok(PackedLeaves::new());
    }
    let bag = pool_size - rack_size;
    let mut placed = vec![0u8; num_letters];
    let (exchanged, move_value) = move_tiles_and_value(board_snapshot, play, &mut placed);
    if placed.iter().zip(&pool).any(|(&p, &u)| p > u) {
        return_error!("the move places tiles the watcher holds or the board has".into());
    }
    // a move placing as many tiles as the bag holds draws it empty (an exchange
    // leaves the bag as it was)
    if exchanged.is_none() && placed.iter().map(|&c| c as usize).sum::<usize>() >= bag {
        return Ok(PackedLeaves::new());
    }
    let can_exchange = bag >= game_config.exchange_tile_limit() as usize;
    if exchanged.is_some() && !can_exchange {
        return Ok(PackedLeaves::new());
    }
    let ln_binom = LnBinom::new(pool_size);
    let tiles = match exchanged {
        Some(n) => MoveTiles::Exchanged(n),
        None => MoveTiles::Placed(&placed),
    };
    // each rack as the walk finds it: the leave it kept, its gap, and the log of
    // its chance to be dealt
    let mut racks = Vec::new();
    // the racks the move could have come from: few, when it leaves few tiles
    // unseen, and then each is generated on the board in place of the sheet
    let num_racks = match exchanged {
        Some(_) => count_tallies_of_size(&pool, rack_size),
        None => {
            let avail: Vec<u8> = pool.iter().zip(&placed).map(|(&u, &p)| u - p).collect();
            count_tallies_of_size(
                &avail,
                rack_size - placed.iter().map(|&c| c as usize).sum::<usize>(),
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
                lat: &tables.lat,
                sheet: &[],
                leave: &tables.leave,
                ln_binom: &ln_binom,
            },
            &pool,
            tiles,
            move_value,
            &mut |_, ln_prior, gap, kept| racks.push((kept, gap, ln_prior)),
        );
    } else {
        infer_sheet_for(
            move_generator,
            board_tiles,
            SpellTables {
                game_config,
                kwg: board_snapshot.kwg,
                klv: board_snapshot.klv,
                lat: &tables.lat,
            },
            &pool,
            can_exchange,
            (&mut tables.movegen_rack, &mut tables.blank_deltas),
            &mut tables.sheet,
        );
        weigh_racks(
            &InferSheet {
                lat: &tables.lat,
                sheet: &tables.sheet,
                leave: &tables.leave,
                ln_binom: &ln_binom,
            },
            &pool,
            tiles,
            move_value,
            threads,
            &mut |_, ln_prior, gap, kept| racks.push((kept, gap, ln_prior)),
        );
    }
    let weighed = weights_by_leave(racks, fuzz);
    let mut leaves = PackedLeaves::with_capacity(weighed.len(), weighed.len() * rack_size);
    let mut kept = vec![0u8; num_letters];
    let mut tiles = Vec::with_capacity(rack_size);
    for (rank, weight) in weighed {
        tables.lat.unrank_into(rank as usize, &mut kept);
        tiles.clear();
        tiles.extend(
            (0u8..)
                .zip(&kept)
                .flat_map(|(tile, &count)| std::iter::repeat_n(tile, count as usize)),
        );
        leaves.push(&tiles, weight)?;
    }
    Ok(leaves)
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
#[derive(Clone, Copy, Default)]
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
        let scale = equity::SCALE as f64;
        let n = deals as i128;
        Reply {
            mean: self.equity as f64 / deals as f64 / scale,
            variance: (n * self.squares - self.equity * self.equity) as f64
                / (deals as f64 * deals as f64)
                / (scale * scale),
            bingo: self.bingos as f64 / deals as f64,
        }
    }
}

// C(n, k), exactly: each step's product is the next binomial times its divisor
#[inline(always)]
fn binom(n: usize, k: usize) -> u64 {
    if k > n {
        return 0;
    }
    (0..k).fold(1u64, |c, i| c * (n - i) as u64 / (i + 1) as u64)
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
    pool: &[u8],
    kept: &[u8],
    threads: usize,
) -> error::Returns<ReplySums> {
    // a wasm build has no threads
    let threads = if cfg!(target_family = "wasm") {
        1
    } else {
        threads
    };
    let num_letters = lat.num_letters();
    let avail = pool
        .iter()
        .zip(kept)
        .map(|(&u, &k)| u - k)
        .collect::<Vec<_>>();
    let left = lat.rack_size() - kept.iter().map(|&c| c as usize).sum::<usize>();
    let mut cap = vec![0usize; num_letters + 1];
    for t in (0..num_letters).rev() {
        cap[t] = cap[t + 1] + avail[t] as usize;
    }
    // the ways to draw so many of each letter from what is left of it
    let ways = (avail.iter())
        .map(|&u| (0..=left).map(|r| binom(u as usize, r)).collect::<Vec<_>>())
        .collect::<Vec<_>>();
    let add_rack = |drawn: &[u8], rack: &mut [u8], sums: &mut ReplySums| {
        for (r, (&d, &k)) in rack.iter_mut().zip(drawn.iter().zip(kept)) {
            *r = d + k;
        }
        let deals = (drawn.iter().zip(&ways))
            .map(|(&d, ways)| ways[d as usize])
            .product::<u64>();
        let rank = lat.rank(rack) as usize;
        let mut best = census::best_equity_of(lat, sheet, leave, rack);
        if best < i32::MIN / 8 {
            best = leave[rank];
        }
        sums.deals += deals;
        sums.equity += deals as i128 * best as i128;
        sums.squares += deals as i128 * (best as i128 * best as i128);
        // only a play of every tile is entered for the whole rack, and it
        // scores at least its bonus
        if sheet[rank] > 0 {
            sums.bingos += deals;
        }
    };
    let mut sums = ReplySums::default();
    if threads <= 1 || count_tallies_of_size(&avail, left) < WEIGH_ACROSS_THREADS_FROM {
        let mut cur = vec![0u8; num_letters];
        let mut rack = vec![0u8; num_letters];
        each_tally_of_size(&avail, &cap, 0, left, &mut cur, &mut |drawn| {
            add_rack(drawn, &mut rack, &mut sums)
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
        each_prefix_of_size(&avail, &cap, 0, k, left, &mut cur, &mut |prefix, left| {
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
                        each_tally_of_size(&avail, &cap, k, *left, &mut cur, &mut |drawn| {
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
    let mut pool = game_config.tiles_off_board(board_snapshot.board_tiles)?;
    game_config.take_rack(&mut pool, mover_rack)?;
    let pool_size: usize = pool.iter().map(|&c| c as usize).sum();
    if pool_size <= rack_size {
        return Ok(None);
    }
    let bag = pool_size - rack_size;
    let mut held = vec![0u8; num_letters];
    for &tile in mover_rack {
        held[tile as usize] += 1;
    }
    let mut used = vec![0u8; num_letters];
    let mut board_tiles = board_snapshot.board_tiles.to_vec();
    // the tiles the mover draws after it: as many as it places (an exchange
    // leaves the bag as it was)
    let drawn = match play {
        movegen::Play::Exchange { tiles } => {
            if !tiles.is_empty() && bag < game_config.exchange_tile_limit() as usize {
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
                    board_tiles[strider.at(i)] = tile;
                    *used
                        .get_mut(if tile & 0x80 != 0 { 0 } else { tile as usize })
                        .ok_or("no such tile")? += 1;
                }
            }
            used.iter().map(|&c| c as usize).sum()
        }
    };
    if used.iter().zip(&held).any(|(&u, &h)| u > h) {
        return_error!("the move uses tiles the mover does not hold".into());
    }
    if drawn >= bag {
        return Ok(None);
    }
    let can_exchange = bag - drawn >= game_config.exchange_tile_limit() as usize;
    infer_sheet_for(
        move_generator,
        &board_tiles,
        SpellTables {
            game_config,
            kwg: board_snapshot.kwg,
            klv: board_snapshot.klv,
            lat: &tables.lat,
        },
        &pool,
        can_exchange,
        (&mut tables.movegen_rack, &mut tables.blank_deltas),
        &mut tables.sheet,
    );
    let (lat, sheet, leave) = (&tables.lat, &tables.sheet[..], &tables.leave[..]);
    // the reply over the full racks holding this tally, the rest drawn
    let reply_holding = |tally: &[u8]| -> error::Returns<Reply> {
        let n = tally.iter().map(|&c| c as usize).sum::<usize>();
        let sums = reply_sums(lat, sheet, leave, &pool, tally, threads)?;
        let deals = binom(pool_size - n, rack_size - n);
        if sums.deals != deals {
            return_error!(format!(
                "the racks were dealt {} ways, not the pool's {deals}",
                sums.deals
            ));
        }
        Ok(sums.reply(deals))
    };
    if kept.is_empty() {
        return Ok(Some(reply_holding(&vec![0u8; num_letters])?));
    }
    let mut tally = vec![0u8; num_letters];
    let (mut weight, mut mean, mut square, mut bingo) = (0.0, 0.0, 0.0, 0.0);
    for (tiles, w) in kept {
        if !w.is_finite() || *w < 0.0 {
            return_error!(format!("a kept leave weighs {w}"));
        }
        tally.iter_mut().for_each(|c| *c = 0);
        for &tile in tiles {
            *tally.get_mut(tile as usize).ok_or("no such tile")? += 1;
        }
        if tiles.len() > rack_size || tally.iter().zip(&pool).any(|(&k, &u)| k > u) {
            return_error!("a kept leave is not in the pool".into());
        }
        let reply = reply_holding(&tally)?;
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
        let mut move_generator = movegen::KurniaMoveGenerator::new(game_config);
        kept_leaves(
            &mut tables,
            &mut move_generator,
            &board_snapshot,
            &watcher_rack,
            play,
            fuzz,
            2,
        )
        .unwrap()
    }

    // under jumbled rules, on the alphagrams of tiny_words (every order of each
    // is a word), a move keeps what it keeps under the classic rules
    #[test]
    #[inline]
    fn jumbled_rules_infer_as_classic_ones_on_words_whose_anagrams_are_words() {
        let classic = game_config::make_english_game_config();
        let jumbled = game_config::make_jumbled_english_game_config();
        let bytes = build::build(
            build::BuildContent::DawgOnly,
            build::BuildLayout::Wolges,
            build::BuildOrder::Sorted,
            &build::make_alphagrams(&tiny_words()),
        )
        .unwrap();
        let alphagrams = kwg::kbwg_from_bytes_alloc(bytes.to_vec(), false).unwrap();
        let at = movegen::Play::Place {
            down: false,
            lane: 7,
            idx: 7,
            word: [1, 20][..].into(),
            score: 0,
        };
        let exchange = movegen::Play::Exchange {
            tiles: [0u8; 3][..].into(),
        };
        for (on_board, play, fuzz) in [(0, &at, 0.0), (0, &at, 3.0), (79, &exchange, 3.0)] {
            let want = leaves_under(&classic, &tiny_kwg(), on_board, play, fuzz);
            assert!(!want.is_empty());
            assert_eq!(
                leaves_under(&jumbled, &alphagrams, on_board, play, fuzz),
                want
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
            score: 0,
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
            score: 0,
        };
        assert!(leaves_after(91, &y, 3.0).is_empty());
        // a pass leaves the bag as it was, so its leaves are weighed
        let pass = movegen::Play::Exchange {
            tiles: [][..].into(),
        };
        assert!(!leaves_after(91, &pass, 3.0).is_empty());
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
        );
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
                tiles(which),
                value,
                &mut |rank, ln_prior, gap, kept| {
                    by_generation.push((rank, ln_prior.to_bits(), gap, kept));
                },
            );
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
            let h = (idx as i32).wrapping_mul(2654435761u32 as i32);
            *s = h.rem_euclid(30_000) - 10_000;
            *l = h.wrapping_mul(2246822519u32 as i32).rem_euclid(8_000) - 4_000;
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
                equity: deals as i128 * value,
                squares: deals as i128 * value * value,
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
        let got = weights_by_leave(racks, 0.0);
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
            move_generator.gen_moves_unfiltered(&movegen::GenMovesParams {
                board_snapshot,
                rack: &rack,
                max_gen: 1 << 20,
                num_exchanges_by_this_player: 0,
                pass_policy: movegen::PassPolicy::OnlyWhenForced,
                dynamic_leaves: None,
            });
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
            sums.equity += deals as i128 * *best as i128;
            sums.squares += deals as i128 * (*best as i128 * *best as i128);
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
            let value = *best as f64;
            weight += chance;
            equity += chance * value;
            squares += chance * value * value;
            if *bingo {
                bingos += chance;
            }
        }
        let scale = equity::SCALE as f64;
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
    // racks weighed by the ways the pool deals them, on any threads: with
    // exchanges and a bingo in reach, after a placement, a pass, or an
    // exchange; with no exchange left in the bag, before or only after the
    // mover draws; and with racks that can only pass, one of them a leave the
    // table values. None when the bag is empty after the mover draws, or the
    // pool holds no more than a rack
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
            score: 2,
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
            ("big, a pass", &big, &pass, true, &kept_big),
            ("big, an exchange", &big, &exchange, true, &kept_big),
            ("mid, TA", &mid, &ta, true, &kept_big[1..3]),
            ("mid, a pass", &mid, &pass, true, &kept_big[1..3]),
            ("small, TA", &small, &ta, true, &kept_big[..3]),
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
            for threads in [1, 4] {
                let mut on = |kept| {
                    reply_on(
                        &mut tables,
                        &mut move_generator,
                        &board_tiles,
                        kept,
                        play,
                        threads,
                    )
                    .unwrap()
                    .unwrap()
                };
                assert_eq!(on(&[]), want, "[{name}] on {threads} threads");
                let got = on(kept);
                assert!(
                    about(&got, &want_kept),
                    "[{name}] kept, on {threads} threads: {got:?}, not {want_kept:?}"
                );
            }
        }
        // a kept leave the pool does not hold, or one weighing less than
        // nothing, is refused; so are leaves weighing nothing at all
        let board_tiles = reply_board(&game_config, &big, &mover);
        for kept in [
            &[(vec![18], 1.0)][..],
            &[(vec![1], -1.0)],
            &[(vec![1], f64::NAN)],
            &[(vec![1], 0.0)],
        ] {
            assert!(
                reply_on(&mut tables, &mut move_generator, &board_tiles, kept, &ta, 1).is_err(),
                "{kept:?}"
            );
        }
        // the mover's draw empties the bag: three placed, three in it
        let tae = movegen::Play::Place {
            down: true,
            lane: 0,
            idx: 11,
            word: [0, 1, 5, 20][..].into(),
            score: 0,
        };
        let board_tiles = reply_board(&game_config, &small, &mover);
        assert_eq!(
            reply_on(&mut tables, &mut move_generator, &board_tiles, &[], &tae, 1).unwrap(),
            None
        );
        // the pool holds a rack, and no bag
        let seven = tally(&[(1, 3), (5, 2), (20, 2)]);
        let board_tiles = reply_board(&game_config, &seven, &mover);
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
}
