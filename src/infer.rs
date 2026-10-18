// Copyright (C) 2020-2026 Andy Kurnia.

// inferring the rack behind a move: every rack the unseen tiles could have
// dealt the mover, weighted by its chance of being dealt and by how far the
// move falls short of that rack's best play, valued the way the census sheet
// values a board

use super::{census, equity, error, game_config, klv, kwg, movegen, play_scorer};

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
pub fn build_sheet_spell_once<N: kwg::Node, L: kwg::Node>(
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

// every full rack the pool can deal that could have made the move, to f(rank of
// the rack, ln of its chance to be dealt, the equity it gives up by making the
// move). The gap is the rack's best equity on the board minus the move's: the
// move's value plus what it keeps, for a placed move; the best that exchanging
// so many tiles keeps, for an exchange. The best is over the moves that are not
// a pass, and the move itself: a hasty mover never passes while it can move, a
// simmer may, so a pass gives up the rack's best move over keeping it all, and
// nothing when the rack had no move. xsheet is scratch the size of the lattice.
#[inline]
pub fn weigh_racks(
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
pub fn infer_sheet_for<N: kwg::Node, L: kwg::Node>(
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
