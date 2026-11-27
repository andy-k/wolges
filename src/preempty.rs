// Copyright (C) 2020-2026 Andy Kurnia.

use super::{bites, equity, error, fash, kwg, movegen, zone};

#[inline(always)]
fn placed(word: &[u8]) -> impl Iterator<Item = u8> + '_ {
    word.iter()
        .filter(|&&t| t != 0)
        .map(|&t| t & !((t as i8) >> 7) as u8)
}

fn gen_all<N: kwg::Node, L: kwg::Node>(
    mg: &mut movegen::KurniaMoveGenerator,
    snap: &movegen::BoardSnapshot<'_, N, L>,
    rack: &[u8],
    num_exchanges: i16,
) {
    mg.gen_moves_unfiltered(&movegen::GenMovesParams {
        board_snapshot: snap,
        rack,
        max_gen: usize::MAX,
        num_exchanges_by_this_player: num_exchanges,
        pass_policy: movegen::PassPolicy::OnlyWhenForced,
        dynamic_leaves: None,
    });
}

#[inline(always)]
fn choose(n: u8, k: u8) -> u128 {
    (0..u128::from(k)).fold(1, |a, i| a * (u128::from(n) - i) / (i + 1))
}

fn each_draw(
    tally: &[u8],
    k: u8,
    at: usize,
    cur: &mut Vec<u8>,
    w: u128,
    out: &mut Vec<(Vec<u8>, u128)>,
) {
    if k == 0 {
        out.push((cur.clone(), w));
        return;
    }
    if at >= tally.len() {
        return;
    }
    for j in 0..=k.min(tally[at]) {
        for _ in 0..j {
            cur.push(at as u8);
        }
        each_draw(tally, k - j, at + 1, cur, w * choose(tally[at], j), out);
        cur.truncate(cur.len() - j as usize);
    }
}

// the best score of a play placing every tile of the rack, if one exists
pub fn best_out<N: kwg::Node, L: kwg::Node>(
    mg: &mut movegen::KurniaMoveGenerator,
    snap: &movegen::BoardSnapshot<'_, N, L>,
    rack: &[u8],
) -> Option<equity::Equity> {
    let mut best = None;
    mg.gen_outs(snap, rack, (!0, !0), |_, _, score| {
        best = best.max(Some(score));
    });
    best
}

struct LaneOut {
    down: bool,
    lane: i8,
    score: equity::Equity,
}

pub struct ReplyScores {
    pub out: Option<equity::Equity>,
    pub other: Option<equity::Equity>,
}

pub struct ReplyPool {
    outs: fash::MyHashMap<bites::Bites, Vec<LaneOut>>,
    other: Vec<(bites::Bites, equity::Equity)>,
    rack_size: usize,
}

impl ReplyPool {
    pub fn new<N: kwg::Node, L: kwg::Node>(
        mg: &mut movegen::KurniaMoveGenerator,
        snap: &movegen::BoardSnapshot<'_, N, L>,
        pool: &[u8],
        rack_size: usize,
    ) -> Option<Self> {
        if rack_size > usize::from(snap.game_config.rack_size()) {
            return None;
        }
        let mut outs = fash::MyHashMap::<_, Vec<LaneOut>>::default();
        let mut other = fash::MyHashMap::<_, equity::Equity>::default();
        let mut tiles = Vec::new();
        mg.pool_plays(snap, pool, |down, lane, _, word, score| {
            tiles.clear();
            tiles.extend(placed(word));
            if tiles.len() > rack_size {
                return;
            }
            tiles.sort_unstable();
            let key = bites::Bites::from(tiles.as_slice());
            let score = equity::Equity::new(score);
            if tiles.len() == rack_size {
                let lanes = outs.entry(key).or_default();
                if let Some(best) = lanes
                    .iter_mut()
                    .find(|entry| entry.down == down && entry.lane == lane)
                {
                    best.score = best.score.max(score);
                } else {
                    lanes.push(LaneOut { down, lane, score });
                }
            } else {
                let best = other.entry(key).or_insert(score);
                *best = (*best).max(score);
            }
        });
        if mg.out_of_memory() {
            return None;
        }
        let mut other: Vec<_> = other.into_iter().collect();
        other.sort_unstable_by(|(a_tiles, a_score), (b_tiles, b_score)| {
            b_score.cmp(a_score).then(a_tiles.cmp(b_tiles))
        });
        Some(Self {
            outs,
            other,
            rack_size,
        })
    }

    pub fn scores(&self, rack: &[u8]) -> Option<ReplyScores> {
        if rack.len() != self.rack_size {
            return None;
        }
        let mut rack = rack.to_vec();
        rack.sort_unstable();
        let out = self
            .outs
            .get(rack.as_slice())
            .and_then(|lanes| lanes.iter().map(|entry| entry.score).max());
        let other = self.other.iter().find_map(|(tiles, score)| {
            let mut owned = rack.iter().copied().peekable();
            tiles
                .iter()
                .all(|&tile| {
                    while owned.peek().is_some_and(|&r| r < tile) {
                        owned.next();
                    }
                    owned.next() == Some(tile)
                })
                .then_some(*score)
        });
        Some(ReplyScores { out, other })
    }
}

struct Threats {
    scores: [Vec<Option<equity::Equity>>; 2],
}

impl Threats {
    fn new<N: kwg::Node, L: kwg::Node>(
        generator: &mut movegen::KurniaMoveGenerator,
        snap: &movegen::BoardSnapshot<'_, N, L>,
        rack: &[u8],
        pool: Option<&ReplyPool>,
    ) -> Self {
        let dim = snap.game_config.board_layout().dim();
        let mut scores = [vec![None; dim.rows as usize], vec![None; dim.cols as usize]];
        if let Some(pool) = pool {
            if let Some(lanes) = pool.outs.get(rack) {
                for entry in lanes {
                    scores[usize::from(entry.down)][entry.lane as usize] = Some(entry.score);
                }
            }
        } else {
            generator.gen_outs(snap, rack, (!0, !0), |down, lane, score| {
                let best = &mut scores[usize::from(down)][lane as usize];
                *best = (*best).max(Some(score));
            });
        }
        Self { scores }
    }

    fn outside(&self, (rows, cols): (u128, u128)) -> Option<equity::Equity> {
        self.scores
            .iter()
            .zip([rows, cols])
            .flat_map(|(scores, touched)| {
                scores.iter().enumerate().filter_map(move |(lane, &score)| {
                    if touched & (1 << lane) == 0 {
                        score
                    } else {
                        None
                    }
                })
            })
            .max()
    }
}

struct Draws {
    each: Vec<(Vec<u8>, u128)>, // a draw's tiles and its ways
    theirs: Vec<Vec<u8>>,       // by draw, the opponent's rack
    ways: u128,
    scores: Vec<i128>,
}

impl Draws {
    fn new(alphabet: &crate::alphabet::Alphabet, unseen: &[u8], bag_size: u8) -> Self {
        let mut each = Vec::new();
        each_draw(unseen, bag_size, 0, &mut Vec::new(), 1, &mut each);
        let mut tally = unseen.to_vec();
        let theirs = each
            .iter()
            .map(|(d, _)| {
                for &t in d {
                    tally[t as usize] -= 1;
                }
                let rack = (0..tally.len())
                    .flat_map(|t| std::iter::repeat_n(t as u8, tally[t] as usize))
                    .collect();
                for &t in d {
                    tally[t as usize] += 1;
                }
                rack
            })
            .collect();
        let ways = each.iter().map(|&(_, w)| w).sum();
        let scores = each
            .iter()
            .map(|(draw, _)| i128::from(alphabet.scaled_rack_score(draw).raw()))
            .collect();
        Self {
            each,
            theirs,
            ways,
            scores,
        }
    }
}

struct Candidate {
    index: usize,
    score: i128,
    leave_score: i128,
    draw_lower: i128,
    draw_upper: i128,
}

impl Candidate {
    fn out_value(&self, draw_score: i128, out_score: i128) -> i128 {
        self.score - out_score - 2 * (self.leave_score + draw_score)
    }
}

struct Worker {
    base: movegen::KurniaMoveGenerator,
    after: movegen::KurniaMoveGenerator,
    found: Vec<Option<i128>>, // its share of a batch's draws, valued
}

impl Worker {
    fn new(gc: &crate::game_config::GameConfig) -> Self {
        Self {
            base: movegen::KurniaMoveGenerator::new(gc),
            after: movegen::KurniaMoveGenerator::new(gc),
            found: Vec::new(),
        }
    }

    // the play's value summed over the draws, each times its ways: where the
    // opponent can then go out, our score less their best out play's score and
    // twice what our rack then scores; else the play's equity
    fn value<N: kwg::Node, L: kwg::Node>(
        &mut self,
        job: &Job<'_, N, L>,
        draw: usize,
        threat: &mut Option<Threats>,
        upper: i128,
    ) -> Option<i128> {
        let weight = job.draws.each[draw].1 as i128;
        let rack = &job.draws.theirs[draw];
        let score = job.draws.scores[draw];
        let mut out = if job.lanes == (!0, !0) {
            None
        } else {
            threat
                .get_or_insert_with(|| Threats::new(&mut self.base, job.before, rack, job.pool))
                .outside(job.lanes)
        };
        if let Some(known) = out
            && upper
                - (job.candidate.draw_upper
                    - job.candidate.out_value(score, i128::from(known.raw())))
                    * weight
                < job.target
        {
            return None;
        }
        self.after
            .gen_outs(&job.after, rack, job.lanes, |_, _, score| {
                out = out.max(Some(score));
            });
        Some(out.map_or(job.static_value, |out| {
            job.candidate.out_value(score, i128::from(out.raw()))
        }))
    }
}

struct Job<'a, N: kwg::Node, L: kwg::Node> {
    before: &'a movegen::BoardSnapshot<'a, N, L>,
    after: movegen::BoardSnapshot<'a, N, L>,
    candidate: &'a Candidate,
    static_value: i128,
    lanes: (u128, u128),
    draws: &'a Draws,
    pool: Option<&'a ReplyPool>,
    target: i128,
}

struct Active {
    lanes: (u128, u128),
    next_draw: usize,
    lower: i128,
    upper: i128,
}

impl Active {
    fn add(&mut self, candidate: &Candidate, weight: i128, value: i128) {
        self.lower += (value - candidate.draw_lower) * weight;
        self.upper -= (candidate.draw_upper - value) * weight;
    }
}

pub struct Best<'a> {
    pub play: &'a movegen::Play,
    pub play_lower: i128,
    pub play_upper: i128,
    pub best_upper: i128,
    pub ways: u128,
    pub exact: bool,
}

pub struct Progress<'a, N: kwg::Node, L: kwg::Node> {
    before: movegen::BoardSnapshot<'a, N, L>,
    board: Vec<u8>,
    opening: bool,
    plays: Vec<movegen::ValuedMove>,
    draws: Draws,
    threats: Vec<Option<Threats>>,
    unseen: Vec<u8>,
    pool: Option<Option<ReplyPool>>,
    order: Vec<Candidate>,
    next: usize,
    active: Option<Active>,
    best: Option<(i128, usize)>,
    searched: usize,
    workers: Vec<Worker>,
}

impl<'a, N: kwg::Node, L: kwg::Node> Progress<'a, N, L> {
    pub fn new(
        mg: &mut movegen::KurniaMoveGenerator,
        snap: &movegen::BoardSnapshot<'a, N, L>,
        rack: &[u8],
        num_exchanges: i16,
        unseen: &[u8],
        bag_size: usize,
    ) -> error::Returns<Self> {
        gen_all(mg, snap, rack, num_exchanges);
        Self::from_plays(&mut mg.plays, snap, rack, unseen, bag_size)
    }

    fn from_plays(
        plays: &mut Vec<movegen::ValuedMove>,
        snap: &movegen::BoardSnapshot<'a, N, L>,
        rack: &[u8],
        unseen: &[u8],
        bag_size: usize,
    ) -> error::Returns<Self> {
        plays.sort_by_key(|vm| std::cmp::Reverse(vm.equity));
        let first = plays.first().ok_or("no play")?;
        let gc = snap.game_config;
        let applies = (1..=usize::from(gc.rack_size())).contains(&bag_size);
        let draws = if applies {
            Draws::new(gc.alphabet(), unseen, bag_size as u8)
        } else {
            Draws {
                each: Vec::new(),
                theirs: Vec::new(),
                ways: 1,
                scores: Vec::new(),
            }
        };
        if draws.ways == 0 {
            return Err("the bag holds more tiles than are unseen".into());
        }
        let ways = draws.ways as i128;
        let min_out = min_out_score(gc, &draws);
        let min_draw = draws.scores.iter().min().copied().unwrap_or_default();
        let max_draw = draws.scores.iter().max().copied().unwrap_or_default();
        let max_out = i128::from(i32::MAX);
        let rack_score = i128::from(gc.alphabet().scaled_rack_score(rack).raw());
        let mut order = Vec::new();
        let mut best = None;
        if applies {
            for (index, vm) in plays.iter().enumerate() {
                let static_value = i128::from(vm.equity.raw());
                if let movegen::Play::Place { word, score, .. } = &vm.play
                    && placed(word).count() >= bag_size
                {
                    let leave_score = rack_score
                        - placed(word)
                            .map(|tile| i128::from(gc.alphabet().scaled_score(tile).raw()))
                            .sum::<i128>();
                    let mut candidate = Candidate {
                        index,
                        score: i128::from(score.raw()),
                        leave_score,
                        draw_lower: 0,
                        draw_upper: 0,
                    };
                    candidate.draw_lower = static_value.min(candidate.out_value(max_draw, max_out));
                    candidate.draw_upper = static_value.max(candidate.out_value(min_draw, min_out));
                    order.push(candidate);
                } else if best.is_none() {
                    best = Some((static_value * ways, index));
                }
            }
        } else {
            best = Some((i128::from(first.equity.raw()), 0));
        }
        order.sort_by_key(|candidate| (std::cmp::Reverse(candidate.draw_upper), candidate.index));
        Ok(Self {
            before: movegen::BoardSnapshot { ..*snap },
            board: snap.board_tiles.to_vec(),
            opening: snap.board_tiles.iter().all(|&tile| tile == 0),
            plays: std::mem::take(plays),
            threats: (0..draws.each.len()).map(|_| None).collect(),
            unseen: unseen.to_vec(),
            pool: None,
            draws,
            order,
            next: 0,
            active: None,
            best,
            searched: 0,
            workers: Vec::new(),
        })
    }

    fn target(&self, index: usize) -> i128 {
        self.best
            .map_or(i128::MIN, |(value, best)| value + i128::from(index > best))
    }

    pub fn best(&self) -> error::Returns<Best<'_>> {
        let ways = self.draws.ways;
        let pending = self.order.get(self.next);
        let active = self.active.as_ref();
        let mut found = self.best.map(|(value, index)| (value, index, value));
        if let Some(candidate) = pending {
            let (lower, upper) = active.map_or(
                (
                    candidate.draw_lower * ways as i128,
                    candidate.draw_upper * ways as i128,
                ),
                |active| (active.lower, active.upper),
            );
            if found.is_none_or(|(value, index, _)| {
                (lower, std::cmp::Reverse(candidate.index)) > (value, std::cmp::Reverse(index))
            }) {
                found = Some((lower, candidate.index, upper));
            }
        }
        let (lower, index, upper) = found.ok_or("no play")?;
        let play = &self.plays.get(index).ok_or("no play")?.play;
        let best_upper = self
            .order
            .get(self.next + usize::from(active.is_some()))
            .map_or(upper, |candidate| {
                upper.max(candidate.draw_upper * ways as i128)
            });
        Ok(Best {
            play,
            play_lower: lower,
            play_upper: upper,
            best_upper: active.map_or(best_upper, |active| active.upper.max(best_upper)),
            ways,
            exact: pending.is_none(),
        })
    }

    pub fn plays(&self) -> &[movegen::ValuedMove] {
        &self.plays
    }

    pub fn searched(&self) -> usize {
        self.searched
    }

    pub fn step(&mut self, budget: usize) -> error::Returns<Best<'_>> {
        self.advance(budget, 1)?;
        self.best()
    }

    #[cfg(not(target_family = "wasm"))]
    pub fn step_on_threads(
        &mut self,
        budget: usize,
        threads: std::num::NonZeroUsize,
    ) -> error::Returns<Best<'_>> {
        self.advance(budget, threads.get())?;
        self.best()
    }

    fn start_candidate(&mut self) -> error::Returns<Option<Active>> {
        let Some(candidate) = self.order.get(self.next) else {
            return Ok(None);
        };
        let ways = self.draws.ways as i128;
        let upper = candidate.draw_upper * ways;
        if upper < self.target(candidate.index) {
            self.next = self.order.len();
            return Ok(None);
        }
        let movegen::Play::Place {
            down,
            lane,
            idx,
            word,
            ..
        } = &self.plays.get(candidate.index).ok_or("no play")?.play
        else {
            return Err("an emptying candidate must be a placement".into());
        };
        self.board.copy_from_slice(self.before.board_tiles);
        let dim = self.before.game_config.board_layout().dim();
        let strider = dim.lane(*down, *lane);
        for (at, &tile) in (*idx..).zip(word.iter()) {
            if tile != 0 {
                self.board[strider.at(at)] = tile;
            }
        }
        Ok(Some(Active {
            lanes: if self.opening {
                (!0, !0)
            } else {
                zone::place_lanes(dim, self.before.board_tiles, (*down, *lane, *idx, word))
            },
            next_draw: 0,
            lower: candidate.draw_lower * ways,
            upper,
        }))
    }

    fn advance(&mut self, mut budget: usize, threads: usize) -> error::Returns<()> {
        while budget != 0 {
            if self.active.is_none() {
                self.active = self.start_candidate()?;
            }
            let Some(candidate) = self.order.get(self.next) else {
                break;
            };
            let target = self.target(candidate.index);
            let active = self
                .active
                .as_mut()
                .ok_or("no active pre-empty candidate")?;
            if self.pool.is_none() {
                let pool = if self.opening || self.draws.each.len() == 1 {
                    None
                } else {
                    let unseen = self
                        .unseen
                        .iter()
                        .enumerate()
                        .flat_map(|(tile, &count)| {
                            std::iter::repeat_n(tile as u8, usize::from(count))
                        })
                        .collect::<Vec<_>>();
                    if self.workers.is_empty() {
                        self.workers.push(Worker::new(self.before.game_config));
                    }
                    ReplyPool::new(
                        &mut self.workers[0].base,
                        &self.before,
                        &unseen,
                        self.draws.theirs[0].len(),
                    )
                };
                self.pool = Some(pool);
            }
            let count = (self.draws.each.len() - active.next_draw).min(budget);
            let job = Job {
                before: &self.before,
                after: movegen::BoardSnapshot {
                    board_tiles: &self.board,
                    ..self.before
                },
                candidate,
                static_value: i128::from(self.plays[candidate.index].equity.raw()),
                lanes: active.lanes,
                draws: &self.draws,
                pool: self.pool.as_ref().and_then(Option::as_ref),
                target,
            };
            let threads = threads.min(count);
            if self.workers.len() < threads {
                self.workers
                    .resize_with(threads, || Worker::new(self.before.game_config));
            }
            let mut ruled_out = false;
            #[cfg(not(target_family = "wasm"))]
            if threads > 1 {
                let first = active.next_draw;
                let shares = count.div_ceil(threads);
                let upper = active.upper;
                let threats = &mut self.threats[first..first + count];
                let workers = &mut self.workers;
                let batches = std::thread::scope(|scope| {
                    let job = &job;
                    let mut handles = Vec::new();
                    let mut spawn_error = None;
                    for (part, (worker, threats)) in workers
                        .iter_mut()
                        .zip(threats.chunks_mut(shares))
                        .enumerate()
                    {
                        let run = move || {
                            let start = first + part * shares;
                            let mut found = std::mem::take(&mut worker.found);
                            found.clear();
                            for (i, threat) in threats.iter_mut().enumerate() {
                                found.push(worker.value(job, start + i, threat, upper));
                            }
                            worker.found = found;
                        };
                        match std::thread::Builder::new().spawn_scoped(scope, run) {
                            Ok(handle) => handles.push(handle),
                            Err(err) => {
                                spawn_error = Some(err);
                                break;
                            }
                        }
                    }
                    let batches = join_draw_batches(handles);
                    match spawn_error {
                        Some(err) => Err(err.into()),
                        None => batches,
                    }
                });
                let batches = match batches {
                    Ok(batches) => batches,
                    Err(err) => {
                        threats.fill_with(|| None);
                        self.workers.clear();
                        return Err(err);
                    }
                };
                let found = self.workers[..batches.len()].iter();
                for &value in found.flat_map(|worker| &worker.found) {
                    if let Some(value) = value {
                        active.add(
                            candidate,
                            self.draws.each[active.next_draw].1 as i128,
                            value,
                        );
                    } else {
                        ruled_out = true;
                    }
                    active.next_draw += 1;
                }
                budget -= count;
                self.searched += count;
            }
            if threads == 1 {
                for _ in 0..count {
                    let draw = active.next_draw;
                    self.searched += 1;
                    budget -= 1;
                    let Some(value) =
                        self.workers[0].value(&job, draw, &mut self.threats[draw], active.upper)
                    else {
                        ruled_out = true;
                        break;
                    };
                    active.add(candidate, self.draws.each[draw].1 as i128, value);
                    active.next_draw += 1;
                    if active.upper < target {
                        ruled_out = true;
                        break;
                    }
                }
            }
            if !ruled_out && active.upper >= target && active.next_draw == self.draws.each.len() {
                self.best = Some((active.upper, candidate.index));
            }
            if ruled_out || active.upper < target || active.next_draw == self.draws.each.len() {
                self.active = None;
                self.next += 1;
            }
        }
        Ok(())
    }
}

#[cfg(not(target_family = "wasm"))]
fn join_draw_batches<T>(
    handles: Vec<std::thread::ScopedJoinHandle<'_, T>>,
) -> error::Returns<Vec<T>> {
    let count = handles.len();
    let batches: Vec<_> = handles
        .into_iter()
        .filter_map(|handle| handle.join().ok())
        .collect();
    if batches.len() != count {
        return Err("a pre-empty draw worker panicked".into());
    }
    Ok(batches)
}

fn min_out_score(gc: &crate::game_config::GameConfig, draws: &Draws) -> i128 {
    let alphabet = gc.alphabet();
    if (0..alphabet.len()).any(|tile| alphabet.score(tile) < 0) {
        return i128::from(i32::MIN);
    }
    draws.theirs.first().map_or(0, |rack| {
        i128::from(
            gc.num_played_bonus(rack.len().min(usize::from(gc.rack_size())) as u8)
                .min(0),
        ) * i128::from(equity::SCALE)
    })
}

pub fn applies(game_config: &crate::game_config::GameConfig, bag_size: usize) -> bool {
    game_config.num_players() == 2 && (1..=usize::from(game_config.rack_size())).contains(&bag_size)
}

pub(crate) fn pick_generated<N: kwg::Node, L: kwg::Node>(
    plays: &mut Vec<movegen::ValuedMove>,
    snap: &movegen::BoardSnapshot<'_, N, L>,
    rack: &[u8],
    bag_size: usize,
) -> error::Returns<()> {
    let gc = snap.game_config;
    let mut unseen = gc.tiles_off_board(snap.board_tiles)?;
    gc.take_rack(&mut unseen, rack)?;
    let mut progress = Progress::from_plays(plays, snap, rack, &unseen, bag_size)?;
    let index = progress.advance(usize::MAX, 1).and_then(|()| {
        progress
            .best
            .map(|(_, index)| index)
            .ok_or_else(|| "no play".into())
    });
    *plays = progress.plays;
    plays.swap(0, index?);
    plays.truncate(1);
    Ok(())
}

// the best play, each play that empties the bag valued over every draw the bag
// allows; the plays are left in mg.plays
pub fn best_play<N: kwg::Node, L: kwg::Node>(
    mg: &mut movegen::KurniaMoveGenerator,
    snap: &movegen::BoardSnapshot<'_, N, L>,
    rack: &[u8],
    num_exchanges: i16,
    unseen: &[u8],
    bag_size: usize,
) -> error::Returns<movegen::Play> {
    let mut progress = Progress::new(mg, snap, rack, num_exchanges, unseen, bag_size)?;
    let picked = progress.step(usize::MAX).map(|best| best.play.clone());
    mg.plays = progress.plays;
    picked
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{bites, build, game_config, klv};

    #[test]
    fn each_draw_counts_every_way() {
        let tally = [1u8, 3, 0, 2, 4];
        for k in 0..=6u8 {
            let mut out = Vec::new();
            each_draw(&tally, k, 0, &mut Vec::new(), 1, &mut out);
            let ways: u128 = out.iter().map(|(_, w)| w).sum();
            assert_eq!(ways, choose(10, k), "k {k}");
            assert!(out.iter().all(|(d, _)| d.len() == k as usize));
            let mut tiles = out.iter().map(|(d, _)| d.clone()).collect::<Vec<_>>();
            tiles.sort();
            tiles.dedup();
            assert_eq!(tiles.len(), out.len(), "k {k}");
        }
        assert_eq!(choose(10, 3), 120);
    }

    struct Fixture {
        gc: game_config::GameConfig,
        kwg: kwg::Kwg<kwg::Node24>,
        klv: klv::Klv<kwg::Node24>,
        board: Vec<u8>,
    }

    impl Fixture {
        fn new(gc: game_config::GameConfig, words: &[&[u8]]) -> Self {
            let mut words: Vec<bites::Bites> = words.iter().map(|&w| w.into()).collect();
            words.sort();
            let kwg = kwg::Kwg::<kwg::Node24>::from_bytes_alloc(
                &build::build_for_game(&gc, &words).unwrap(),
            )
            .unwrap();
            let klv = klv::Klv::<kwg::Node24>::empty().unwrap();
            let dim = gc.board_layout().dim();
            let board = vec![0u8; dim.rows as usize * dim.cols as usize];
            Self {
                gc,
                kwg,
                klv,
                board,
            }
        }

        fn snap(&self) -> movegen::BoardSnapshot<'_, kwg::Node24, kwg::Node24> {
            movegen::BoardSnapshot {
                board_tiles: &self.board,
                game_config: &self.gc,
                kwg: &self.kwg,
                anagrams: None,
                rack_lengths: None,
                klv: &self.klv,
            }
        }

        fn tally(&self, tiles: &[u8]) -> Vec<u8> {
            let mut tally = vec![0u8; self.gc.alphabet().len() as usize];
            for &t in tiles {
                tally[t as usize] += 1;
            }
            tally
        }
    }

    // every play valued over every draw, by the full list of the opponent's
    // plays, the best kept as best_play keeps it
    fn every_play_valued(
        f: &Fixture,
        rack: &[u8],
        unseen: &[u8],
        bag_size: usize,
    ) -> (movegen::Play, Vec<i128>) {
        let snap = f.snap();
        let mut mg = movegen::KurniaMoveGenerator::new(&f.gc);
        gen_all(&mut mg, &snap, rack, 0);
        let mut plays = std::mem::take(&mut mg.plays);
        plays.sort_by_key(|vm| std::cmp::Reverse(vm.equity));
        let mut each = Vec::new();
        each_draw(unseen, bag_size as u8, 0, &mut Vec::new(), 1, &mut each);
        let ways: u128 = each.iter().map(|&(_, w)| w).sum();
        let alphabet = f.gc.alphabet();
        let mut values = Vec::new();
        for vm in &plays {
            let movegen::Play::Place {
                down,
                lane,
                idx,
                word,
                score,
            } = &vm.play
            else {
                values.push(i128::from(vm.equity.raw()) * ways as i128);
                continue;
            };
            if placed(word).count() < bag_size {
                values.push(i128::from(vm.equity.raw()) * ways as i128);
                continue;
            }
            let mut board = f.board.clone();
            let strider = f.gc.board_layout().dim().lane(*down, *lane);
            for (at, &t) in (*idx..).zip(word.iter()) {
                if t != 0 {
                    board[strider.at(at)] = t;
                }
            }
            let after = movegen::BoardSnapshot {
                board_tiles: &board,
                ..snap
            };
            let mut sum = 0i128;
            for (d, w) in &each {
                let mut tally = unseen.to_vec();
                for &t in d {
                    tally[t as usize] -= 1;
                }
                let theirs: Vec<u8> = (0..tally.len())
                    .flat_map(|t| std::iter::repeat_n(t as u8, tally[t] as usize))
                    .collect();
                gen_all(&mut mg, &after, &theirs, 0);
                let out = mg
                    .plays
                    .iter()
                    .filter_map(|p| match &p.play {
                        movegen::Play::Place { word, score, .. }
                            if placed(word).count() == theirs.len() =>
                        {
                            Some(*score)
                        }
                        _ => None,
                    })
                    .max();
                let mut mine = rack.to_vec();
                crate::game_state::use_tiles(&mut mine, placed(word)).unwrap();
                mine.extend_from_slice(d);
                let value = match out {
                    Some(out) => {
                        i128::from(score.raw())
                            - i128::from(out.raw())
                            - 2 * i128::from(alphabet.scaled_rack_score(&mine).raw())
                    }
                    None => i128::from(vm.equity.raw()),
                };
                sum += value * *w as i128;
            }
            values.push(sum);
        }
        let mut at = 0;
        for (i, &v) in values.iter().enumerate() {
            if v > values[at] {
                at = i;
            }
        }
        (plays[at].play.clone(), values)
    }

    // A=1 B=2 E=5 T=20
    const WORDS: [&[u8]; 6] = [
        &[1, 20],
        &[20, 1],
        &[2, 1, 20],
        &[2, 5, 20],
        &[5, 1, 20],
        &[2, 1, 20, 5],
    ];

    #[test]
    fn the_cutoff_picks_as_valuing_every_play_does() {
        let f = Fixture::new(game_config::make_english_game_config(), &WORDS);
        let mut mg = movegen::KurniaMoveGenerator::new(&f.gc);
        for (rack, unseen) in [
            (
                &[1u8, 2, 5, 20, 20, 20, 20][..],
                &[1u8, 1, 2, 5, 5, 20, 20, 20][..],
            ),
            (&[1, 1, 2, 5, 20, 20, 20], &[1, 2, 5, 20, 20, 20, 20, 1]),
            (&[2, 5, 20, 1], &[1, 1, 20, 20, 5, 2, 2]),
            (&[2, 5, 20, 1], &[1, 20, 20]),
            (&[2, 5, 20, 1, 1], &[1, 20, 2, 20]),
        ] {
            let unseen = f.tally(unseen);
            for bag in 1..=rack.len().min(unseen.iter().map(|&c| c as usize).sum()) {
                let p = best_play(&mut mg, &f.snap(), rack, 0, &unseen, bag).unwrap();
                let (want, _) = every_play_valued(&f, rack, &unseen, bag);
                assert!(p == want, "rack {rack:?} bag {bag}");
            }
        }
    }

    #[test]
    fn a_play_is_charged_the_opponents_best_out_after_it() {
        let f = Fixture::new(game_config::make_english_game_config(), &WORDS);
        let snap = f.snap();
        let mut mg = movegen::KurniaMoveGenerator::new(&f.gc);
        let rack = [2u8, 5, 20, 1];
        let unseen = f.tally(&[1, 20, 20]);
        let bag = 1;
        let draws = Draws::new(f.gc.alphabet(), &unseen, bag as u8);
        assert_eq!(draws.ways, 3);
        let (_, want) = every_play_valued(&f, &rack, &unseen, bag);
        gen_all(&mut mg, &snap, &rack, 0);
        let mut plays = std::mem::take(&mut mg.plays);
        plays.sort_by_key(|vm| std::cmp::Reverse(vm.equity));
        let mut worker = Worker::new(&f.gc);
        let mut charged = 0;
        for (vm, want) in plays.iter().zip(want) {
            let movegen::Play::Place {
                down,
                lane,
                idx,
                word,
                score,
            } = &vm.play
            else {
                continue;
            };
            let mut board = f.board.clone();
            let strider = f.gc.board_layout().dim().lane(*down, *lane);
            for (at, &t) in (*idx..).zip(word.iter()) {
                if t != 0 {
                    board[strider.at(at)] = t;
                }
            }
            let after = movegen::BoardSnapshot {
                board_tiles: &board,
                ..snap
            };
            let mut leave = rack.to_vec();
            crate::game_state::use_tiles(&mut leave, placed(word)).unwrap();
            let candidate = Candidate {
                index: 0,
                score: i128::from(score.raw()),
                leave_score: i128::from(f.gc.alphabet().scaled_rack_score(&leave).raw()),
                draw_lower: i128::MIN,
                draw_upper: i128::MAX,
            };
            let job = Job {
                before: &snap,
                after,
                candidate: &candidate,
                static_value: i128::from(vm.equity.raw()),
                lanes: (!0, !0),
                draws: &draws,
                pool: None,
                target: i128::MIN,
            };
            let got = draws
                .each
                .iter()
                .enumerate()
                .map(|(i, &(_, ways))| {
                    worker.value(&job, i, &mut None, i128::MAX).unwrap() * ways as i128
                })
                .sum::<i128>();
            assert_eq!(got, want);
            if got != i128::from(vm.equity.raw()) * draws.ways as i128 {
                charged += 1;
            }
        }
        assert!(charged > 0);
    }

    #[test]
    fn an_out_is_found_with_its_best_score() {
        let f = Fixture::new(game_config::make_english_game_config(), &WORDS);
        let snap = f.snap();
        let mut mg = movegen::KurniaMoveGenerator::new(&f.gc);
        for rack in [
            &[2u8, 1, 20, 5][..],
            &[2, 1, 20],
            &[1, 20],
            &[5, 5],
            &[2, 2, 1, 20],
        ] {
            gen_all(&mut mg, &snap, rack, 0);
            let want = mg
                .plays
                .iter()
                .filter_map(|p| match &p.play {
                    movegen::Play::Place { word, score, .. }
                        if placed(word).count() == rack.len() =>
                    {
                        Some(*score)
                    }
                    _ => None,
                })
                .max();
            assert_eq!(best_out(&mut mg, &snap, rack), want, "{rack:?}");
        }
        assert!(best_out(&mut mg, &snap, &[5, 5]).is_none());
        assert!(best_out(&mut mg, &snap, &[2, 1, 20, 5]).is_some());
    }

    #[test]
    fn touched_lanes_keep_the_same_out_scores_as_a_full_walk() {
        let mut changed = (false, false);
        for gc in [
            game_config::make_english_game_config(),
            game_config::make_jumbled_english_game_config(),
            game_config::make_woadwage_english_game_config(),
            game_config::make_jumbled_woadwage_english_game_config(),
        ] {
            let mut f = Fixture::new(gc, &WORDS);
            let layout = f.gc.board_layout();
            let dim = layout.dim();
            let star = dim.across(layout.star_row());
            f.board[star.at(layout.star_col())] = 1 | 0x80;
            f.board[star.at(layout.star_col() + 1)] = 20;
            let tables = crate::anagrams::ArmTables::build(&f.kwg, &f.gc, false).unwrap();
            let before = movegen::BoardSnapshot {
                anagrams: tables.anagrams(),
                rack_lengths: tables.rack_lengths(),
                ..f.snap()
            };
            let mut mg = movegen::KurniaMoveGenerator::new(&f.gc);
            gen_all(&mut mg, &before, &[0, 1, 2, 5, 20], 0);
            let plays = std::mem::take(&mut mg.plays);
            assert!(!plays.is_empty());
            for theirs in [&[2, 5][..], &[0, 20], &[1], &[5, 5]] {
                let threats = Threats::new(&mut mg, &before, theirs, None);
                let old = threats.outside((0, 0));
                for vm in &plays {
                    let movegen::Play::Place {
                        down,
                        lane,
                        idx,
                        word,
                        ..
                    } = &vm.play
                    else {
                        continue;
                    };
                    let mut board = f.board.clone();
                    for (at, &tile) in (*idx..).zip(word.iter()) {
                        if tile != 0 {
                            board[dim.lane(*down, *lane).at(at)] = tile;
                        }
                    }
                    let after = movegen::BoardSnapshot {
                        board_tiles: &board,
                        ..before
                    };
                    let lanes = zone::place_lanes(dim, &f.board, (*down, *lane, *idx, word));
                    let mut got = threats.outside(lanes);
                    mg.gen_outs(&after, theirs, lanes, |_, _, score| {
                        got = got.max(Some(score))
                    });
                    gen_all(&mut mg, &after, theirs, 0);
                    let want = mg
                        .plays
                        .iter()
                        .filter_map(|vm| match &vm.play {
                            movegen::Play::Place { word, score, .. }
                                if placed(word).count() == theirs.len() =>
                            {
                                Some(*score)
                            }
                            _ => None,
                        })
                        .max();
                    assert_eq!(got, want);
                    changed.0 |= got < old;
                    changed.1 |= got > old;
                }
            }
        }
        assert_eq!(changed, (true, true));
    }

    #[test]
    fn the_static_best_is_exact_when_no_placement_empties_the_bag() {
        let f = Fixture::new(game_config::make_english_game_config(), &[&[1, 20]]);
        let rack = [1, 20];
        let bag = rack.len() + 1;
        let unseen = f.tally(&[1, 2, 5, 20]);
        let snap = f.snap();
        let mut mg = movegen::KurniaMoveGenerator::new(&f.gc);
        gen_all(&mut mg, &snap, &rack, 0);
        mg.plays.sort_by_key(|vm| std::cmp::Reverse(vm.equity));
        let static_best = mg.plays[0].clone();
        assert!(
            mg.plays
                .iter()
                .any(|vm| matches!(vm.play, movegen::Play::Place { .. }))
        );
        assert!(mg.plays.iter().all(|vm| match &vm.play {
            movegen::Play::Place { word, .. } => placed(word).count() < bag,
            movegen::Play::Exchange { .. } => true,
        }));

        let mut progress = Progress::from_plays(&mut mg.plays, &snap, &rack, &unseen, bag).unwrap();
        let best = progress.step(usize::MAX).unwrap();
        let value = i128::from(static_best.equity.raw()) * best.ways as i128;
        assert!(best.exact);
        assert!(best.play == &static_best.play);
        assert_eq!(
            (best.play_lower, best.play_upper, best.best_upper),
            (value, value, value)
        );
        assert_eq!(progress.searched(), 0);
    }

    #[cfg(not(target_family = "wasm"))]
    #[test]
    fn a_panicking_draw_batch_joins_the_other_workers() {
        let result = std::thread::scope(|scope| {
            join_draw_batches(vec![
                scope.spawn(|| -> usize { panic!("first draw worker") }),
                scope.spawn(|| -> usize { panic!("second draw worker") }),
            ])
        });
        assert_eq!(
            result.unwrap_err().to_string(),
            "a pre-empty draw worker panicked"
        );
    }

    #[test]
    fn stepping_keeps_bounds_and_reaches_the_exhaustive_pick() {
        let f = Fixture::new(game_config::make_english_game_config(), &WORDS);
        let rack = [2, 5, 20, 1];
        let unseen = f.tally(&[1, 20, 20, 5]);
        let bag = 1;
        let (winner, values) = every_play_valued(&f, &rack, &unseen, bag);
        let optimum = *values.iter().max().unwrap();
        let mut mg = movegen::KurniaMoveGenerator::new(&f.gc);
        for parallel in [false, cfg!(not(target_family = "wasm"))] {
            let mut progress = Progress::new(&mut mg, &f.snap(), &rack, 0, &unseen, bag).unwrap();
            let mut previous_upper = progress.best().unwrap().best_upper;
            let mut previous_lower = progress.best().unwrap().play_lower;
            assert!(!progress.step(0).unwrap().exact);
            assert_eq!(progress.searched(), 0);
            let budget = if parallel {
                f.gc.num_players() as usize
            } else {
                1
            };
            loop {
                let old = progress.searched();
                #[cfg(not(target_family = "wasm"))]
                if parallel {
                    progress
                        .step_on_threads(budget, std::num::NonZeroUsize::new(budget).unwrap())
                        .unwrap();
                } else {
                    progress.step(budget).unwrap();
                }
                #[cfg(target_family = "wasm")]
                progress.step(budget).unwrap();
                assert!(progress.searched() <= old + budget);
                let current = progress.best().unwrap();
                let i = progress
                    .plays()
                    .iter()
                    .position(|vm| &vm.play == current.play)
                    .unwrap();
                assert!(current.play_lower <= values[i] && values[i] <= current.play_upper);
                assert!(current.play_lower >= previous_lower);
                assert!(current.best_upper >= optimum);
                assert!(current.best_upper <= previous_upper);
                if current.exact {
                    assert!(current.play == &winner);
                    assert_eq!(current.play_lower, optimum);
                    assert_eq!(current.play_upper, optimum);
                    assert_eq!(current.best_upper, optimum);
                    break;
                }
                previous_upper = current.best_upper;
                previous_lower = current.play_lower;
            }
        }
    }

    fn picker_fixture(words: &[&[u8]]) -> Fixture {
        let game_config::GameConfig::Static(mut gc) = game_config::make_english_game_config();
        gc.alphabet = crate::alphabet::Alphabet::new_static_from_text(
            "? ? 0 0 0 0 0\nA a 2 1 1 0 0\nB b 2 3 0 0 0\nC c 1 3 0 0 0\n",
        )
        .unwrap();
        gc.rack_size = 2;
        Fixture::new(game_config::GameConfig::Static(gc), words)
    }

    #[test]
    fn tilt_filters_the_mover_but_the_full_graph_scores_opponent_outs() {
        let full = picker_fixture(&[&[1, 2], &[3, 1, 2]]);
        let limited = picker_fixture(&[&[1, 2]]);
        let rack = [1, 2];
        let unseen = full.tally(&[1, 2, 3]);
        let mut generator = movegen::KurniaMoveGenerator::new(&full.gc);
        let mut importance = vec![0; full.gc.board_layout().dim().cols as usize + 1];
        importance[rack.len()] = crate::move_filter::IMPORTANCE_DENOM as u8;
        let mut tilt = crate::move_filter::Tilt::new(&full.gc, &full.kwg, &importance);
        tilt.tilt_to(crate::move_filter::TILT_DENOM / 2, 0);
        let mut filter = crate::move_filter::GenMoves::Tilt { tilt, bot_level: 0 };
        let mut values = Vec::new();
        for fixture in [&full, &limited] {
            let snap = fixture.snap();
            generator.reset_for_another_kwg();
            filter.gen_moves(&mut generator, &snap, &rack, 0, usize::MAX, None);
            assert!(generator.plays.iter().all(|vm| matches!(
                &vm.play, movegen::Play::Place { word, .. } if **word == rack
            )));
            let mut progress =
                Progress::from_plays(&mut generator.plays, &snap, &rack, &unseen, 1).unwrap();
            values.push(progress.step(usize::MAX).unwrap().play_lower);
        }
        assert!(values[0] < values[1], "{values:?}");

        let snap = full.snap();
        let mut tilt = crate::move_filter::Tilt::new(&full.gc, &full.kwg, &importance);
        tilt.tilt_to(crate::move_filter::TILT_DENOM, 0);
        let mut filter = crate::move_filter::GenMoves::Tilt { tilt, bot_level: 0 };
        filter.gen_moves(&mut generator, &snap, &rack, 0, usize::MAX, None);
        pick_generated(&mut generator.plays, &snap, &rack, 1).unwrap();
        assert!(
            matches!(&generator.plays[0].play, movegen::Play::Exchange { tiles } if tiles.is_empty())
        );
    }

    #[test]
    fn a_single_pass_or_exchange_keeps_its_static_equity() {
        let f = picker_fixture(&[&[1, 2]]);
        let rack = [1, 2];
        let snap = f.snap();
        for tiles in [&rack[..0], &rack[..1]] {
            let play = movegen::Play::Exchange {
                tiles: tiles.into(),
            };
            let equity = equity::Equity::new(equity::SCALE);
            let mut plays = vec![movegen::ValuedMove {
                play: play.clone(),
                equity,
            }];
            pick_generated(&mut plays, &snap, &rack, rack.len()).unwrap();
            assert_eq!(plays.len(), 1);
            assert!(plays[0].play == play);
            assert_eq!(plays[0].equity, equity);
        }
        let mut plays = Vec::new();
        let err = pick_generated(&mut plays, &snap, &rack, rack.len()).unwrap_err();
        assert_eq!(err.to_string(), "no play");
        assert!(plays.is_empty());
    }

    #[test]
    fn every_picker_can_use_the_preempty_root_pick() {
        use crate::{anagrams, census, game_state, infer, move_filter, move_picker};
        use std::sync::Arc;

        let Fixture {
            gc,
            kwg,
            klv,
            board,
        } = picker_fixture(&[&[1, 2], &[2, 3], &[3, 1, 2]]);
        let gc = Arc::new(gc);
        let kwg = Arc::new(kwg);
        let klv = Arc::new(klv);
        let tables = Arc::new(anagrams::ArmTables::None);
        let snap = movegen::BoardSnapshot {
            board_tiles: &board,
            game_config: &gc,
            kwg: &kwg,
            anagrams: None,
            rack_lengths: None,
            klv: &klv,
        };
        let mut state = game_state::GameState::new(&gc);
        state.players[0].rack = vec![1, 2];
        state.players[1].rack = vec![2, 3];
        state.bag.set_from_iter([1]);
        let rack = &state.current_player().rack;
        let mut generator = movegen::KurniaMoveGenerator::new(&gc);
        let want = best_play(
            &mut generator,
            &snap,
            rack,
            0,
            &[0, 1, 1, 1],
            state.bag.len(),
        )
        .unwrap();
        let lattice =
            census::MultisetLattice::new(gc.alphabet().len() as usize, rack.len()).unwrap();
        let add = census::AddTable::new(&lattice);
        let values = lattice.dense(0).unwrap();
        let dynamic = klv::DynamicLeavesRef {
            lat: &lattice,
            add: &add,
            full_v: &values,
            min_keep: 0,
            take_kept: false,
        };
        let simmer = move_picker::Simmer::new(
            &gc,
            &kwg,
            &tables,
            &klv,
            move_picker::SimmerParams {
                num_sim_iters: 0,
                num_sim_plies: rack.len(),
                sim_threads: 1,
                win_pct_table: None,
                quiet: true,
                memory: None,
            },
        )
        .unwrap();
        for mut picker in [
            move_picker::MovePicker::Hasty,
            move_picker::MovePicker::Dynamic(dynamic),
            move_picker::MovePicker::Deft(move_picker::Deft::new(0, None)),
            move_picker::MovePicker::Simmer(Box::new(simmer)),
        ] {
            picker
                .pick_a_move_after_with_preempty(
                    &mut move_filter::GenMoves::Unfiltered,
                    &mut generator,
                    &snap,
                    &state,
                    rack,
                    (
                        None,
                        &mut infer::Carried::default(),
                        &move_picker::PreemptyMode::WithSimmerRoot,
                    ),
                )
                .unwrap();
            assert!(generator.plays[0].play == want);
        }
    }

    #[test]
    fn a_nonplacement_among_the_candidates_keeps_its_static_value() {
        let f = picker_fixture(&[&[1, 2], &[3, 1, 2]]);
        let rack = [1, 2];
        let snap = f.snap();
        let unseen = f.tally(&[1, 2, 3]);
        let mut mg = movegen::KurniaMoveGenerator::new(&f.gc);
        mg.gen_moves_unfiltered(&movegen::GenMovesParams {
            board_snapshot: &snap,
            rack: &rack,
            max_gen: usize::MAX,
            num_exchanges_by_this_player: 0,
            pass_policy: movegen::PassPolicy::AsACandidate,
            dynamic_leaves: None,
        });
        let placement = mg
            .plays
            .iter()
            .find(|vm| matches!(vm.play, movegen::Play::Place { .. }))
            .unwrap();
        let pass = mg
            .plays
            .iter()
            .find(|vm| matches!(&vm.play, movegen::Play::Exchange { tiles } if tiles.is_empty()))
            .unwrap();
        for (pass_equity, want) in [
            (equity::Equity::NEG_INFINITY, &placement.play),
            (placement.equity, &pass.play),
        ] {
            let mut pass = pass.clone();
            pass.equity = pass_equity;
            let mut candidates = vec![pass, placement.clone()];
            let mut progress =
                Progress::from_plays(&mut candidates, &snap, &rack, &unseen, 1).unwrap();
            let pick = progress.step(usize::MAX).unwrap();
            assert!(pick.exact);
            assert!(pick.play == want);
            if matches!(pick.play, movegen::Play::Exchange { .. }) {
                assert_eq!(
                    pick.play_lower,
                    i128::from(pass_equity.raw()) * pick.ways as i128
                );
            }
        }
    }

    #[test]
    fn a_pool_agrees_with_the_whole_move_list() {
        for gc in [
            game_config::make_english_game_config(),
            game_config::make_jumbled_english_game_config(),
            game_config::make_woadwage_english_game_config(),
            game_config::make_jumbled_woadwage_english_game_config(),
        ] {
            let mut f = Fixture::new(gc, &WORDS);
            let mut mg = movegen::KurniaMoveGenerator::new(&f.gc);
            let pool = [0, 0, 1, 2, 5, 20];
            let tables = crate::anagrams::ArmTables::build(&f.kwg, &f.gc, false).unwrap();
            for opening in [true, false] {
                if !opening {
                    let layout = f.gc.board_layout();
                    let star = layout.dim().across(layout.star_row());
                    f.board[star.at(layout.star_col())] = 1;
                    f.board[star.at(layout.star_col() + 1)] = 20;
                }
                let snap = movegen::BoardSnapshot {
                    anagrams: tables.anagrams(),
                    ..f.snap()
                };
                for rack_size in 1..=usize::from(f.gc.rack_size()).min(pool.len()) {
                    let sheet = ReplyPool::new(&mut mg, &snap, &pool, rack_size).unwrap();
                    let draws = Draws::new(
                        f.gc.alphabet(),
                        &f.tally(&pool),
                        (pool.len() - rack_size) as u8,
                    );
                    for rack in &draws.theirs {
                        let cached = sheet.scores(rack).unwrap();
                        gen_all(&mut mg, &snap, rack, 0);
                        let mut out = None;
                        let mut other = None;
                        for vm in &mg.plays {
                            let movegen::Play::Place { score, word, .. } = &vm.play else {
                                continue;
                            };
                            let best = if placed(word).count() == rack.len() {
                                &mut out
                            } else {
                                &mut other
                            };
                            *best = (*best).max(Some(*score));
                        }
                        assert_eq!(
                            (cached.out, cached.other),
                            (out, other),
                            "opening {opening}, rack {rack:?}"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn exact_windows_find_the_same_out_scores_as_the_walk() {
        let mut words = WORDS.to_vec();
        words.push(&[1]);
        let mut f = Fixture::new(game_config::make_english_game_config(), &words);
        let tables = crate::anagrams::ArmTables::build(&f.kwg, &f.gc, false).unwrap();
        assert!(tables.anagrams().is_some());
        let pool = [0, 0, 1, 2, 5, 20];
        let unseen = f.tally(&pool);
        let mut mg = movegen::KurniaMoveGenerator::new(&f.gc);
        for opening in [true, false] {
            if !opening {
                let layout = f.gc.board_layout();
                let star = layout.dim().across(layout.star_row());
                f.board[star.at(layout.star_col())] = 1;
                f.board[star.at(layout.star_col() + 1)] = 20;
            }
            let walk = f.snap();
            let window = movegen::BoardSnapshot {
                anagrams: tables.anagrams(),
                ..walk
            };
            for kept in 1..=usize::from(f.gc.rack_size()).min(pool.len()) {
                let draws = Draws::new(f.gc.alphabet(), &unseen, (pool.len() - kept) as u8);
                for rack in &draws.theirs {
                    assert_eq!(
                        best_out(&mut mg, &window, rack),
                        best_out(&mut mg, &walk, rack),
                        "opening {opening}, rack {rack:?}"
                    );
                }
            }
        }
    }

    #[test]
    fn a_simmer_copies_its_preempty_rollout_policy_to_workers() {
        use crate::{anagrams, game_state, move_filter, simmer};
        let f = picker_fixture(&[&[1, 2], &[2, 3], &[3, 1, 2]]);
        let mut state = game_state::GameState::new(&f.gc);
        state.players[0].rack = vec![1, 2];
        state.players[1].rack = vec![2, 3];
        state.bag.set_from_iter([1]);
        let mut original = simmer::Simmer::new(&f.gc).unwrap();
        original.set_preempty(Some(&move_filter::GenMoves::Unfiltered));
        original.prepare(&f.gc, &state, &simmer::SimChoices::new(1, false, &[]));
        let mut worker = original.prepared_clone(&f.gc);
        assert!(worker.preempty_enabled());
        worker.prepare_iteration(None);
        let dealt = worker.prepared_state();
        let rack = &dealt.players[1].rack;
        let snap = f.snap();
        let mut mg = movegen::KurniaMoveGenerator::new(&f.gc);
        gen_all(&mut mg, &snap, rack, 0);
        pick_generated(&mut mg.plays, &snap, rack, dealt.bag.len()).unwrap();
        let expected = mg.plays[0].equity;
        worker
            .simulate(
                &f.gc,
                &f.kwg,
                &anagrams::ArmTables::None,
                &f.klv,
                &movegen::Play::Exchange {
                    tiles: [].as_slice().into(),
                },
            )
            .unwrap();
        assert_eq!(worker.reply_equity(), Some(expected));
    }

    #[test]
    fn bags_from_one_to_the_rack_size_are_valued() {
        for rack_size in [3u8, 7, 9] {
            let game_config::GameConfig::Static(mut gc) = game_config::make_english_game_config();
            gc.rack_size = rack_size;
            let f = Fixture::new(game_config::GameConfig::Static(gc), &WORDS);
            let mut mg = movegen::KurniaMoveGenerator::new(&f.gc);
            let rack = [2u8, 1, 20];
            let unseen = f.tally(&[1, 1, 2, 5, 5, 20, 20, 20, 20, 20, 1, 1]);
            gen_all(&mut mg, &f.snap(), &rack, 0);
            let mut plays = std::mem::take(&mut mg.plays);
            plays.sort_by_key(|vm| std::cmp::Reverse(vm.equity));
            for bag in 0..=usize::from(rack_size) + 1 {
                let p = best_play(&mut mg, &f.snap(), &rack, 0, &unseen, bag).unwrap();
                if bag == 0 || bag > usize::from(rack_size) {
                    assert!(p == plays[0].play, "rack size {rack_size} bag {bag}");
                } else {
                    let (want, _) = every_play_valued(&f, &rack, &unseen, bag);
                    assert!(p == want, "rack size {rack_size} bag {bag}");
                }
            }
        }
    }

    #[test]
    fn a_draw_with_no_out_keeps_the_plays_equity() {
        let f = Fixture::new(game_config::make_english_game_config(), &WORDS);
        let mut mg = movegen::KurniaMoveGenerator::new(&f.gc);
        let rack = [2u8, 1, 20, 5];
        // the opponent then holds only E's, which form no word
        let unseen = f.tally(&[5, 5, 5, 5, 5, 5, 5, 20]);
        gen_all(&mut mg, &f.snap(), &rack, 0);
        let mut plays = std::mem::take(&mut mg.plays);
        plays.sort_by_key(|vm| std::cmp::Reverse(vm.equity));
        let (_, values) = every_play_valued(&f, &rack, &unseen, 1);
        let p = best_play(&mut mg, &f.snap(), &rack, 0, &unseen, 1).unwrap();
        assert!(p == plays[0].play);
        let ways = choose(8, 1) as i128;
        assert!(
            values
                .iter()
                .zip(&plays)
                .all(|(&v, vm)| v == i128::from(vm.equity.raw()) * ways)
        );
    }
}
