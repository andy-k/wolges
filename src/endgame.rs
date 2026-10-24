// Copyright (C) 2020-2026 Andy Kurnia.

// note: this module is very slow and may need a lot of space
// and it still has many bugs

use super::{anagrams, display, equity, error, fash, game_config, klv, kwg, movegen};
use std::io::Write;

// how many nodes the search visits between looks at the clock and the memory,
// for their limits
const LIMIT_CADENCE: u64 = 1024;

// splitmix64's increment: https://prng.di.unimi.it/splitmix64.c
const SPLITMIX64_GAMMA: u64 = 0x9e3779b97f4a7c15;

// https://doi.org/10.1145/2714064.2660195 https://prng.di.unimi.it/splitmix64.c
#[inline(always)]
fn splitmix64(s: &mut u64) -> u64 {
    *s = s.wrapping_add(SPLITMIX64_GAMMA);
    let mut z = *s;
    z = (z ^ (z >> 30)).wrapping_mul(0xbf58476d1ce4e5b9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94d049bb133111eb);
    z ^ (z >> 31)
}

// a position's key is the XOR of the keys of its parts (zobrist hashing): each
// tile on its square, each count of each tile in each rack, the second player
// to move, and the scoreless turns before it (their counts, when any, or that a
// pass is answered). The index-th key is splitmix64's draw after index others
// from state 0, so no table needs to hold them
#[inline(always)]
fn zobrist(index: u64) -> u64 {
    let mut state = index.wrapping_mul(SPLITMIX64_GAMMA);
    splitmix64(&mut state)
}

// the kind of key, in an index's upper half
const SQUARE_KEYS: u64 = 0;
const RACK_KEYS: u64 = 1 << 32;
const SECOND_TO_MOVE: u64 = 2 << 32;
const SCORELESS_TURNS: u64 = 3 << 32;
const ANSWERING_A_PASS: u64 = 4 << 32;

// the scoreless turns before a node, as a pass there sees them: either the
// counts the config checks (the position's own until the line places tiles;
// none from then on) or that the line's move before was a pass. A pass
// answering a pass ends the line: the position is as it was two plies back, so
// a player who would rather place than let the game end places now as well as
// after any more passes, which makes the line worth the game's end here
// whatever the counts
#[derive(Clone, Copy, PartialEq)]
enum Scoreless {
    Counted { passes: u8, zeros: u8 },
    Answering,
}

impl Scoreless {
    // the key of the turns, which the position's key takes on; none for a
    // placement's state
    #[inline(always)]
    fn key(self) -> u64 {
        match self {
            Scoreless::Counted {
                passes: 0,
                zeros: 0,
            } => 0,
            Scoreless::Counted { passes, zeros } => {
                zobrist(SCORELESS_TURNS | ((passes as u64) << 8) | zeros as u64)
            }
            Scoreless::Answering => zobrist(ANSWERING_A_PASS),
        }
    }

    // whether a pass at a node with these turns before it ends the line:
    // answering a pass, or bringing the counts to the config's (the board as
    // scoreless_turns_end takes it)
    #[inline(always)]
    fn a_pass_ends(self, gc: &game_config::GameConfig, board: &[u8]) -> bool {
        match self {
            Scoreless::Answering => true,
            Scoreless::Counted { passes, zeros } => scoreless_turns_end(
                gc,
                Some(board),
                passes.saturating_add(1),
                zeros.saturating_add(1),
            ),
        }
    }
}

// the key of a tile on a square; a blank is keyed apart from the tile it stands
// for
#[inline(always)]
fn square_key(square: usize, tile: u8) -> u64 {
    zobrist(SQUARE_KEYS | ((square as u64) << 8) | tile as u64)
}

// the change to a player's rack key when their count of a tile (a blank as 0)
// goes from count to count + 1, or back; a count of zero has no key
#[inline(always)]
fn rack_step(player: u8, tile: u8, count: usize) -> u64 {
    let key = |count: usize| {
        zobrist(RACK_KEYS | ((player as u64) << 24) | ((tile as u64) << 16) | count as u64)
    };
    if count == 0 {
        key(1)
    } else {
        key(count) ^ key(count + 1)
    }
}

// the rack tile behind a tile on the board: a blank for a blank
#[inline(always)]
fn held_tile(tile: u8) -> u8 {
    tile & !((tile as i8) >> 7) as u8
}

// how a stored value bounds its position's true value
#[derive(Clone, Copy, PartialEq)]
enum Bound {
    Exact, // ==
    Lower, // >=
    Upper, // <=
}

// the depth stored with a value when no line below it was cut short by the
// depth: it is then the position's value to the end, good for a search of any
// depth
const COMPLETE: i8 = i8::MAX;

// the best move as stored: its place among the moves the generator gives for
// the position, or a pass, or none
const PASS_PLAY: u16 = u16::MAX - 1;
const NO_PLAY: u16 = u16::MAX;

// what the table knows of a position
#[derive(Clone, Copy)]
struct Entry {
    value: f32,
    play: u16,
    depth: i8,
    bound: Bound,
}

// the positions the search has valued, in buckets of four entries, 64 bytes
// each, a cache line's size (not aligned to one). An entry is two words: the
// position's key, then, from the low bits up, the value's bits, the best move,
// the depth, the bound (none for an empty entry), and the generation of the
// solve that stored it
struct Table {
    buckets: Vec<[u64; 8]>,
    held: usize,     // entries holding a position
    generation: u64, // the solve storing now
}

impl Table {
    // a bucket to start with; the table grows as the search fills it
    #[inline(always)]
    fn new() -> Self {
        Self {
            buckets: vec![[0; 8]],
            held: 0,
            generation: 0,
        }
    }

    // the positions the table has room for
    #[inline(always)]
    fn room(&self) -> usize {
        4 * self.buckets.len()
    }

    // a key's bucket, by multiply-shift, so the table can have any number of
    // buckets
    #[inline(always)]
    fn bucket(&self, key: u64) -> usize {
        ((key as u128 * self.buckets.len() as u128) >> 64) as usize
    }

    #[inline(always)]
    fn word(&self, entry: Entry) -> u64 {
        entry.value.to_bits() as u64
            | (entry.play as u64) << 32
            | (entry.depth as u8 as u64) << 48
            | (match entry.bound {
                Bound::Exact => 1,
                Bound::Lower => 2,
                Bound::Upper => 3,
            }) << 56
            | (self.generation & 0x3f) << 58
    }

    #[inline(always)]
    fn entry(word: u64) -> Option<Entry> {
        let bound = match (word >> 56) & 3 {
            1 => Bound::Exact,
            2 => Bound::Lower,
            3 => Bound::Upper,
            _ => return None,
        };
        Some(Entry {
            value: f32::from_bits(word as u32),
            play: (word >> 32) as u16,
            depth: (word >> 48) as u8 as i8,
            bound,
        })
    }

    #[inline(always)]
    fn probe(&self, key: u64) -> Option<Entry> {
        let bucket = &self.buckets[self.bucket(key)];
        (0..4)
            .find(|&slot| bucket[2 * slot] == key && (bucket[2 * slot + 1] >> 56) & 3 != 0)
            .and_then(|slot| Self::entry(bucket[2 * slot + 1]))
    }

    // store into the position's own entry, else an empty one, else over the
    // shallowest one an earlier solve stored, else over the shallowest; a
    // deeper value of the position stays unless this one is exact. Returns
    // whether it took an empty entry
    #[inline(always)]
    fn store(&mut self, key: u64, entry: Entry) -> bool {
        let word = self.word(entry);
        let generation = self.generation & 0x3f;
        let index = self.bucket(key);
        let bucket = &mut self.buckets[index];
        let mut victim = 0;
        let mut victim_rank = i32::MAX;
        for slot in 0..4 {
            let old = bucket[2 * slot + 1];
            if (old >> 56) & 3 == 0 {
                bucket[2 * slot] = key;
                bucket[2 * slot + 1] = word;
                self.held += 1;
                return true;
            }
            let depth = (old >> 48) as u8 as i8;
            if bucket[2 * slot] == key {
                if depth <= entry.depth || entry.bound == Bound::Exact {
                    bucket[2 * slot + 1] = word;
                }
                return false;
            }
            let rank = depth as i32 + if old >> 58 == generation { 256 } else { 0 };
            if rank < victim_rank {
                victim = slot;
                victim_rank = rank;
            }
        }
        bucket[2 * victim] = key;
        bucket[2 * victim + 1] = word;
        false
    }

    // move the table to this many buckets, keeping every entry that fits
    #[inline(never)]
    fn resize(&mut self, buckets: usize) {
        let old = std::mem::replace(&mut self.buckets, vec![[0; 8]; buckets]);
        self.held = 0;
        for bucket in &old {
            for slot in 0..4 {
                let word = bucket[2 * slot + 1];
                if (word >> 56) & 3 != 0 {
                    self.put(bucket[2 * slot], word);
                }
            }
        }
    }

    // place a stored word of a position not in the table, as a resize keeps it:
    // in an empty entry, else in place of the shallowest if it is deeper
    #[inline(always)]
    fn put(&mut self, key: u64, word: u64) {
        let index = self.bucket(key);
        let bucket = &mut self.buckets[index];
        let mut victim = 0;
        let mut victim_depth = i8::MAX;
        for slot in 0..4 {
            let old = bucket[2 * slot + 1];
            if (old >> 56) & 3 == 0 {
                bucket[2 * slot] = key;
                bucket[2 * slot + 1] = word;
                self.held += 1;
                return;
            }
            let depth = (old >> 48) as u8 as i8;
            if depth < victim_depth {
                victim = slot;
                victim_depth = depth;
            }
        }
        if (word >> 48) as u8 as i8 > victim_depth {
            bucket[2 * victim] = key;
            bucket[2 * victim + 1] = word;
        }
    }
}

// a position's value as a search found it, and whether no line below it was cut
// short by the depth
struct Searched {
    value: f32,
    complete: bool,
}

// a move generated for a position, as the search orders it
struct Candidate {
    play: movegen::Play,
    index: u16,     // its place in the sorted list, NO_PLAY past what fits
    valuation: f32, // a play out's whole value, else its score
    plays_out: bool,
}

// a move's sort key: valuation (best first), then direction, lane, and first
// square as movegen::cmp_play has them, then the move's place in the list
#[inline(always)]
fn order_key(valuation: i32, down: bool, lane: i8, idx: i8, found: usize) -> u128 {
    ((!(valuation as u32 ^ 1 << 31)) as u128) << 64
        | (down as u128) << 48
        | (lane as u8 as u128) << 40
        | (idx as u8 as u128) << 32
        | found as u32 as u128
}

// sort ordered (a key for each move of list, from order_key; list stays as
// found) by valuation, ties broken by the plays, so the order does not depend
// on how the moves were found. Each move's index becomes its place, counted
// from first_place (ordered may be a tail); the table's move goes first
#[inline(always)]
fn order(list: &mut [Candidate], ordered: &mut [u128], first_place: usize, table_play: u16) {
    ordered.sort_unstable();
    let mut start = 0;
    while start < ordered.len() {
        let end = start
            + ordered[start..]
                .iter()
                .take_while(|&&key| key >> 32 == ordered[start] >> 32)
                .count();
        if end - start > 1 {
            ordered[start..end].sort_unstable_by(|&a, &b| {
                movegen::cmp_play(&list[a as u32 as usize].play, &list[b as u32 as usize].play)
            });
        }
        start = end;
    }
    for (place, &key) in (first_place..).zip(ordered.iter()) {
        list[key as u32 as usize].index = if place < PASS_PLAY as usize {
            place as u16
        } else {
            NO_PLAY
        };
    }
    if table_play < PASS_PLAY && (table_play as usize) < ordered.len() {
        ordered[..=table_play as usize].rotate_right(1);
    }
}

// bring only the first move in order to the front of ordered, with index 0, the
// rest left unsorted (most nodes cut off at their first move); ties as in order
#[inline(always)]
fn order_first(list: &mut [Candidate], ordered: &mut [u128]) {
    let Some(mut first) = (0..ordered.len()).min_by_key(|&i| ordered[i]) else {
        return;
    };
    for i in 0..ordered.len() {
        if ordered[i] >> 32 == ordered[first] >> 32
            && movegen::cmp_play(
                &list[ordered[i] as u32 as usize].play,
                &list[ordered[first] as u32 as usize].play,
            )
            .is_lt()
        {
            first = i;
        }
    }
    ordered.swap(0, first);
    list[ordered[0] as u32 as usize].index = 0;
}

// WorkBuffer contains reusable allocations.
// WorkBuffer can only be reused for the same game_config and kwg.
// (Refer to note at KurniaMoveGenerator.)
// This is not enforced.
struct WorkBuffer {
    t0: std::time::Instant,           // the solve's start, for its time limit
    dur_movegen: std::time::Duration, // for progress, timed only while something listens
    movegen: movegen::KurniaMoveGenerator,
    board_tiles: Vec<u8>,  // the position searched, made and unmade in place
    racks: [Vec<u8>; 2],   // its racks, each sorted
    rack_scores: [i32; 2], // what its racks' tiles score
    key: u64,              // its board's and racks' keys
    lists: Vec<Vec<Candidate>>, // the moves generated at each ply from the root
    orders: Vec<Vec<u128>>, // each ply's sort keys (order_key)
    table: Table,
    can_grow: bool, // the memory limit has not yet refused the table more room this solve
    max_depth: i8,  // this pass's depth, for progress
    finished: i8,   // the deepest pass this solve finished, for the principal variation
    time_limit: Option<std::time::Duration>,
    state_limit: Option<usize>,
    memory_limit: Option<MemoryLimit>,
    check_limits: bool, // from the second pass on, when a limit is set
    nodes: u64,         // this pass's nodes, to space the looks at the clock
    searched: u64,      // this solve's nodes, for progress
    cut_short: bool,    // a limit stopped this pass
    progress: Option<ProgressFn>,
    in_bag: Option<u64>, // inside a search with a tile in the bag, its endgames so far
}

impl WorkBuffer {
    fn new(game_config: &game_config::GameConfig) -> Self {
        Self {
            t0: std::time::Instant::now(),
            dur_movegen: Default::default(),
            movegen: movegen::KurniaMoveGenerator::new(game_config),
            board_tiles: Vec::new(),
            racks: [Vec::new(), Vec::new()],
            rack_scores: [0, 0],
            key: 0,
            lists: Vec::new(),
            orders: Vec::new(),
            table: Table::new(),
            can_grow: true,
            max_depth: 0,
            finished: 0,
            time_limit: None,
            state_limit: None,
            memory_limit: None,
            check_limits: false,
            nodes: 0,
            searched: 0,
            cut_short: false,
            progress: None,
            in_bag: None,
        }
    }

    // store a value, growing the table once three quarters of it holds
    // positions: to twice its size, or as big as the memory limit allows while
    // the old table is still held. Once the limit refuses, the table keeps its
    // size and stores replace other positions
    #[inline(always)]
    fn store(&mut self, key: u64, entry: Entry) {
        if self.table.store(key, entry)
            && self.can_grow
            && self.table.held >= 3 * self.table.buckets.len()
        {
            let now = self.table.buckets.len();
            let mut buckets = now.saturating_mul(2);
            if let Some(limit) = self.memory_limit {
                let room = limit.bytes.saturating_sub((limit.held)()).saturating_sub(1)
                    / std::mem::size_of::<[u64; 8]>();
                buckets = buckets.min(room);
            }
            if buckets > now {
                self.table.resize(buckets);
            } else {
                self.can_grow = false;
            }
        }
    }
}

// past any limit a solve stops, and answers from the deepest search that
// finished
pub struct Limits {
    pub time: Option<std::time::Duration>,
    pub states: Option<usize>, // a solve stops once the table holds this many positions
    pub memory: Option<MemoryLimit>,
}

impl Limits {
    pub const NONE: Self = Self {
        time: None,
        states: None,
        memory: None,
    };
}

// the most bytes the program may use, and how to read how many it holds. The
// table grows only while the program stays within them, then replaces other
// positions, so a solve stops on the limit only when the rest of the program
// passes it
#[derive(Clone, Copy)] // each endgame of a search with a tile in the bag takes a copy
pub struct MemoryLimit {
    pub bytes: usize,
    pub held: fn() -> usize,
}

impl MemoryLimit {
    #[inline(always)]
    fn passed(&self) -> bool {
        (self.held)() >= self.bytes
    }
}

// a search with a tile in the bag solves many endgames as one solve: its time
// limit is one deadline for them all, on its clock, and its state limit applies
// to the table they share (the solver's own, lent with what its earlier solves
// found); the endgames take turns holding its one listener
struct BagLimits {
    time: Option<std::time::Duration>,
    states: Option<usize>,
    memory: Option<MemoryLimit>,
    t0: std::time::Instant,
    dur_movegen: std::cell::Cell<std::time::Duration>,
    endgames: std::cell::Cell<u64>,
    progress: std::cell::Cell<Option<ProgressFn>>,
}

impl BagLimits {
    // its clock starts now
    #[inline(always)]
    fn new(limits: &Limits, progress: Option<ProgressFn>) -> Self {
        Self {
            time: limits.time,
            states: limits.states,
            memory: limits.memory,
            t0: std::time::Instant::now(),
            dur_movegen: Default::default(),
            endgames: Default::default(),
            progress: std::cell::Cell::new(progress),
        }
    }
}

// the solver counts millipoints in f32, which holds whole ones exactly; an
// average, such as the peg's margin, rounds to the nearest
#[inline(always)]
fn to_equity(v: f32) -> equity::Equity {
    equity::Equity::new(v.round() as i32)
}

// a solve's answer: exact, or the value of the deepest search that finished
// before a limit stopped it
pub struct Solved {
    pub value: equity::Equity,
    pub depth: i8,
    pub exact: bool,
}

// how far a solve has got, sent to its listener at every node, and at each
// finished depth with that depth's value; with a tile in the bag, at every node
// alone. The listener keeps the time, on its own clock
pub struct Progress {
    pub endgames: Option<u64>, // with a tile in the bag, the endgames searched so far
    pub depth: i8,
    pub value: Option<equity::Equity>, // the depth's value, once it has finished
    pub movegen: std::time::Duration,
    pub nodes: u64,  // the solve's nodes so far
    pub held: usize, // the positions the table holds
    pub room: usize, // the positions it has room for
}

impl Progress {
    // its line, with the time its listener has kept since the solve began
    #[inline(always)]
    pub fn after(&self, elapsed: std::time::Duration) -> ProgressLine<'_> {
        ProgressLine {
            progress: self,
            elapsed,
        }
    }
}

pub struct ProgressLine<'a> {
    progress: &'a Progress,
    elapsed: std::time::Duration,
}

impl std::fmt::Display for ProgressLine<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let p = self.progress;
        if let Some(endgames) = p.endgames {
            write!(f, "{endgames} endgames searched, now ")?;
        }
        match p.value {
            Some(value) => write!(f, "depth {} is {}", p.depth, value)?,
            None => write!(f, "depth {}", p.depth)?,
        }
        write!(
            f,
            ": after {:?} ({:?} on movegen), {} nodes, the table holds {} positions of {}",
            self.elapsed, p.movegen, p.nodes, p.held, p.room,
        )
    }
}

type ProgressFn = Box<dyn FnMut(&Progress) -> error::Returns<()>>;

// EndgameSolver is the main two-player endgame solver.
// EndgameSolver can only be reused for the same game_config and kwg.
// (Refer to note at WorkBuffer.)
// This is not enforced.
pub struct EndgameSolver<'a, N: kwg::Node, L: kwg::Node> {
    game_config: &'a game_config::GameConfig,
    kwg: &'a kwg::Kwg<N>,
    arm_tables: &'a anagrams::ArmTables,
    klv: Box<klv::Klv<L>>,
    board_tiles: Vec<u8>,
    racks: [Vec<u8>; 2],
    scoreless: [u8; 2], // the passes and the zeros before the position to solve
    work_buffer: WorkBuffer,
}

impl<'a, N: kwg::Node, L: kwg::Node> EndgameSolver<'a, N, L> {
    pub fn new(
        game_config: &'a game_config::GameConfig,
        kwg: &'a kwg::Kwg<N>,
        arm_tables: &'a anagrams::ArmTables,
    ) -> error::Returns<Self> {
        if game_config.num_players() != 2 {
            return Err("cannot solve non-2-player endgames".into());
        }
        Ok(Self {
            game_config,
            kwg,
            arm_tables,
            klv: Box::new(klv::Klv::<L>::from_bytes_alloc(klv::EMPTY_KLV_BYTES)?),
            board_tiles: Vec::new(),
            racks: [Vec::new(), Vec::new()],
            scoreless: [0, 0],
            work_buffer: WorkBuffer::new(game_config),
        })
    }

    // set the position to solve, with no scoreless turns before it; the table
    // keeps what it knows, which holds for any position of the same game and
    // lexicon
    pub fn init(&mut self, board_tiles: &[u8], racks: [&[u8]; 2]) {
        self.init_after_scoreless_turns(board_tiles, racks, 0, 0);
    }

    // as init, after this many passes and this many scoreless turns (the zeros,
    // which count the passes too) as the game counts them: a line of the search
    // ends when they reach the config's counts
    pub fn init_after_scoreless_turns(
        &mut self,
        board_tiles: &[u8],
        racks: [&[u8]; 2],
        passes: u16,
        zeros: u16,
    ) {
        self.board_tiles.clear();
        self.board_tiles.extend_from_slice(board_tiles);
        self.racks[0].clear();
        self.racks[0].extend_from_slice(racks[0]);
        self.racks[1].clear();
        self.racks[1].extend_from_slice(racks[1]);
        self.set_scoreless_turns(passes, zeros);
    }

    // the passes and the scoreless turns before the position, as the game counts
    // them, for a search given its position another way (the peg's)
    pub fn set_scoreless_turns(&mut self, passes: u16, zeros: u16) {
        self.scoreless = [
            passes.min(u8::MAX as u16) as u8,
            zeros.min(u8::MAX as u16) as u8,
        ];
    }

    // reset the position searched to the one to solve, computing its key afresh
    #[inline]
    fn reset_position(&mut self) {
        let alphabet = self.game_config.alphabet();
        let work_buffer = &mut self.work_buffer;
        work_buffer.board_tiles.clone_from(&self.board_tiles);
        work_buffer.key = 0;
        for (square, &tile) in work_buffer.board_tiles.iter().enumerate() {
            if tile != 0 {
                work_buffer.key ^= square_key(square, tile);
            }
        }
        for player in 0..2 {
            let rack = &mut work_buffer.racks[player];
            rack.clone_from(&self.racks[player]);
            rack.sort_unstable();
            for (i, &tile) in rack.iter().enumerate() {
                let count = rack[..i].iter().rev().take_while(|&&t| t == tile).count();
                work_buffer.key ^= rack_step(player as u8, tile, count);
            }
            work_buffer.rack_scores[player] = alphabet.scaled_rack_score(rack);
        }
    }

    // the scoreless turns before the position to solve, as its search starts
    #[inline(always)]
    fn entering(&self) -> Scoreless {
        Scoreless::Counted {
            passes: self.scoreless[0],
            zeros: self.scoreless[1],
        }
    }

    // the key of the position searched with player_idx to move and these
    // scoreless turns before it
    #[inline(always)]
    fn node_key(&self, player_idx: u8, scoreless: Scoreless) -> u64 {
        let mut key = self.work_buffer.key;
        if player_idx != 0 {
            key ^= zobrist(SECOND_TO_MOVE);
        }
        key ^ scoreless.key()
    }

    // whether a pass at a node with these scoreless turns before it ends the
    // line, on the position searched
    #[inline(always)]
    fn pass_ends_the_line(&self, scoreless: Scoreless) -> bool {
        scoreless.a_pass_ends(self.game_config, &self.work_buffer.board_tiles)
    }

    // play player_idx's move on the position searched
    #[inline(always)]
    fn place(&mut self, player_idx: u8, play: &movegen::Play) {
        if let movegen::Play::Place {
            down,
            lane,
            idx,
            word,
            ..
        } = play
        {
            let strider = self.game_config.board_layout().dim().lane(*down, *lane);
            let alphabet = self.game_config.alphabet();
            let work_buffer = &mut self.work_buffer;
            let rack = &mut work_buffer.racks[player_idx as usize];
            for (i, &tile) in (*idx..).zip(word.iter()) {
                if tile != 0 {
                    let square = strider.at(i);
                    work_buffer.board_tiles[square] = tile;
                    work_buffer.key ^= square_key(square, tile);
                    let held = held_tile(tile);
                    let at = rack.partition_point(|&t| t < held);
                    let count = rack[at..].iter().take_while(|&&t| t == held).count();
                    work_buffer.key ^= rack_step(player_idx, held, count - 1);
                    rack.remove(at);
                    work_buffer.rack_scores[player_idx as usize] -= alphabet.scaled_score(held);
                }
            }
        }
    }

    // take player_idx's move back from the position searched
    #[inline(always)]
    fn unplace(&mut self, player_idx: u8, play: &movegen::Play) {
        if let movegen::Play::Place {
            down,
            lane,
            idx,
            word,
            ..
        } = play
        {
            let strider = self.game_config.board_layout().dim().lane(*down, *lane);
            let alphabet = self.game_config.alphabet();
            let work_buffer = &mut self.work_buffer;
            let rack = &mut work_buffer.racks[player_idx as usize];
            for (i, &tile) in (*idx..).zip(word.iter()) {
                if tile != 0 {
                    let square = strider.at(i);
                    work_buffer.board_tiles[square] = 0;
                    work_buffer.key ^= square_key(square, tile);
                    let held = held_tile(tile);
                    let at = rack.partition_point(|&t| t < held);
                    let count = rack[at..].iter().take_while(|&&t| t == held).count();
                    work_buffer.key ^= rack_step(player_idx, held, count);
                    rack.insert(at, held);
                    work_buffer.rack_scores[player_idx as usize] += alphabet.scaled_score(held);
                }
            }
        }
    }

    // the value for player_idx if both players pass now, each losing their rack
    #[inline(always)]
    fn both_pass_value(&self, player_idx: u8) -> f32 {
        let rack_scores = &self.work_buffer.rack_scores;
        (rack_scores[player_idx as usize ^ 1] - rack_scores[player_idx as usize]) as f32
    }

    // generate player_idx's moves that place tiles (a pass is searched on its
    // own) on the position searched, into this ply's list, and their keys,
    // unsorted, into its orders
    #[inline]
    fn generate(&mut self, ply: usize, player_idx: u8) {
        let work_buffer = &mut self.work_buffer;
        if work_buffer.lists.len() <= ply {
            work_buffer.lists.resize_with(ply + 1, Vec::new);
            work_buffer.orders.resize_with(ply + 1, Vec::new);
        }
        // timed only while something listens
        let t1 = work_buffer.progress.is_some().then(std::time::Instant::now);
        work_buffer.movegen.gen_moves_raw_all_unsorted(
            &movegen::BoardSnapshot {
                board_tiles: &work_buffer.board_tiles,
                game_config: self.game_config,
                kwg: self.kwg,
                anagrams: self.arm_tables.anagrams(),
                rack_lengths: self.arm_tables.rack_lengths(),
                klv: &self.klv,
            },
            &work_buffer.racks[player_idx as usize],
            0, // TODO: use game_state to track this
            movegen::PassPolicy::AsACandidate,
        );
        if let Some(t1) = t1 {
            work_buffer.dur_movegen += t1.elapsed();
        }
        let rack_len = work_buffer.racks[player_idx as usize].len();
        let out_bonus = 2 * work_buffer.rack_scores[player_idx as usize ^ 1];
        let list = &mut work_buffer.lists[ply];
        let ordered = &mut work_buffer.orders[ply];
        list.clear();
        ordered.clear();
        for candidate in work_buffer.movegen.plays.iter() {
            if let movegen::Play::Place {
                down,
                lane,
                idx,
                word,
                score,
            } = &candidate.play
            {
                let plays_out = word.iter().filter(|&&t| t != 0).count() == rack_len;
                let valuation = if plays_out { score + out_bonus } else { *score };
                ordered.push(order_key(valuation, *down, *lane, *idx, list.len()));
                list.push(Candidate {
                    play: candidate.play.clone(),
                    index: NO_PLAY,
                    valuation: valuation as f32,
                    plays_out,
                });
            }
        }
    }

    #[inline]
    fn run_id_loop(
        &mut self,
        player_idx: u8,
        verbose: bool,
        limits: &Limits,
    ) -> error::Returns<Solved> {
        let mut last_valuation = f32::NAN;
        // the longest line: a move that does not end it places tiles and leaves
        // the mover some, or passes; a pass answering a pass ends it, as does a
        // pass bringing the scoreless turns to the config's count, and a play
        // out. A mover holding one tile can pass before and after every tile the
        // other places, so its line can run a ply longer than any a mover holding
        // more can make; a rack with no tiles can only pass, as a rack holding
        // its last tile does short of ending the game, so it counts as one tile
        let mine = self.racks[player_idx as usize].len().max(1);
        let theirs = self.racks[player_idx as usize ^ 1].len().max(1);
        let longest = (2 * (mine + theirs) + (mine == 1) as usize - 3).min(COMPLETE as usize - 1);
        let work_buffer = &mut self.work_buffer;
        work_buffer.time_limit = limits.time;
        work_buffer.state_limit = limits.states;
        work_buffer.memory_limit = limits.memory;
        work_buffer.can_grow = true;
        work_buffer.searched = 0;
        work_buffer.finished = 0;
        work_buffer.table.generation += 1;
        for max_depth in 1..=longest as i8 {
            self.work_buffer.max_depth = max_depth;
            // the first pass always finishes, so a solve cut short still has a value
            self.work_buffer.check_limits = max_depth > 1
                && (limits.time.is_some() || limits.states.is_some() || limits.memory.is_some());
            self.work_buffer.nodes = 0;
            self.work_buffer.cut_short = false;
            let searched = if max_depth == 1 {
                self.root_search(player_idx, max_depth, f32::NEG_INFINITY, f32::INFINITY)
            } else {
                self.aspiration_search(player_idx, max_depth, last_valuation)
            };
            let searched = match searched {
                Ok(searched) => searched,
                Err(_) if self.work_buffer.cut_short => {
                    return Ok(Solved {
                        value: to_equity(last_valuation),
                        depth: max_depth - 1,
                        exact: false,
                    });
                }
                Err(e) => return Err(e),
            };
            last_valuation = searched.value;
            self.work_buffer.finished = max_depth;
            if self.work_buffer.in_bag.is_none() {
                self.report(Some(to_equity(searched.value)))?;
            }
            if verbose {
                self.print_best_line(player_idx)?;
            }

            // no line was cut short by the depth, so the value is the
            // position's
            if searched.complete {
                break;
            }
            // a pass that ended past a limit is the last
            if (max_depth as usize) < longest && self.past_a_limit() {
                return Ok(Solved {
                    value: to_equity(searched.value),
                    depth: max_depth,
                    exact: false,
                });
            }
        }
        Ok(Solved {
            value: to_equity(last_valuation),
            depth: self.work_buffer.max_depth,
            exact: true,
        })
    }

    #[inline(always)]
    fn past_a_limit(&self) -> bool {
        let work_buffer = &self.work_buffer;
        work_buffer
            .time_limit
            .is_some_and(|limit| work_buffer.t0.elapsed() >= limit)
            || work_buffer
                .state_limit
                .is_some_and(|limit| work_buffer.table.held >= limit)
            || work_buffer.memory_limit.is_some_and(|limit| limit.passed())
    }

    // search the position to solve to depth within alpha and beta
    #[inline]
    fn root_search(
        &mut self,
        player_idx: u8,
        depth: i8,
        alpha: f32,
        beta: f32,
    ) -> error::Returns<Searched> {
        self.reset_position();
        self.negamax_eval(0, player_idx, depth, alpha, beta, self.entering())
    }

    #[inline]
    fn aspiration_search(
        &mut self,
        player_idx: u8,
        max_depth: i8,
        last_valuation: f32,
    ) -> error::Returns<Searched> {
        const ASPIRATION_WINDOW: f32 = (3 * super::equity::SCALE) as f32;
        let lo = last_valuation - ASPIRATION_WINDOW;
        let hi = last_valuation + ASPIRATION_WINDOW;
        let searched = self.root_search(player_idx, max_depth, lo, hi)?;
        if searched.value > lo && searched.value < hi {
            Ok(searched)
        } else {
            self.root_search(player_idx, max_depth, f32::NEG_INFINITY, f32::INFINITY)
        }
    }

    // send every later solve's progress to the given listener, at every node
    // and at each finished depth; until then it goes nowhere
    #[inline(always)]
    pub fn report_progress(
        &mut self,
        progress: impl FnMut(&Progress) -> error::Returns<()> + 'static,
    ) {
        self.work_buffer.progress = Some(Box::new(progress));
    }

    // a solve of its own starts its clock
    #[inline(always)]
    fn start_clock(&mut self) {
        self.work_buffer.t0 = std::time::Instant::now();
        self.work_buffer.dur_movegen = Default::default();
    }

    // solve within the limits, also printing each finished depth's best line
    #[inline(always)]
    pub fn evaluate(&mut self, player_idx: u8, limits: &Limits) -> error::Returns<Solved> {
        self.start_clock();
        self.run_id_loop(player_idx, true, limits)
    }

    #[inline(always)]
    pub fn solve(&mut self, player_idx: u8) -> error::Returns<equity::Equity> {
        Ok(self.solve_within(player_idx, &Limits::NONE)?.value)
    }

    #[inline(always)]
    pub fn solve_within(&mut self, player_idx: u8, limits: &Limits) -> error::Returns<Solved> {
        self.start_clock();
        self.run_id_loop(player_idx, false, limits)
    }

    // based on https://en.wikipedia.org/wiki/Negamax
    // search the position for player_idx to move, depth plies on, within alpha
    // and beta, its moves at this ply's list; scoreless is the scoreless turns
    // before it, which say whether a pass now ends the line
    fn negamax_eval(
        &mut self,
        ply: usize,
        player_idx: u8,
        depth: i8,
        mut alpha: f32,
        beta: f32,
        scoreless: Scoreless,
    ) -> error::Returns<Searched> {
        // a line cut short by the depth is valued as if both passed
        if depth == 0 {
            return Ok(Searched {
                value: self.both_pass_value(player_idx),
                complete: false,
            });
        }

        // the listener, if any, hears every node
        if self.work_buffer.progress.is_some() {
            self.report(None)?;
        }
        self.work_buffer.searched += 1;

        // the table is looked at every node, the clock and the memory every
        // LIMIT_CADENCE nodes
        if self.work_buffer.check_limits {
            let work_buffer = &mut self.work_buffer;
            work_buffer.nodes += 1;
            if work_buffer
                .state_limit
                .is_some_and(|limit| work_buffer.table.held >= limit)
                || (work_buffer.nodes.is_multiple_of(LIMIT_CADENCE)
                    && (work_buffer
                        .time_limit
                        .is_some_and(|limit| work_buffer.t0.elapsed() >= limit)
                        || work_buffer.memory_limit.is_some_and(|limit| limit.passed())))
            {
                work_buffer.cut_short = true;
                return Err("a limit stopped the search".into());
            }
        }

        // a value stored deep enough answers when it is exact or a bound past
        // the window; its move goes first either way. A bound inside the window
        // does not narrow it, so what this search finds is bounded by the
        // window alone, and a full window always finds the exact value
        let key = self.node_key(player_idx, scoreless);
        let mut table_play = NO_PLAY;
        if let Some(entry) = self.work_buffer.table.probe(key) {
            table_play = entry.play;
            if entry.depth >= depth
                && match entry.bound {
                    Bound::Exact => true,
                    Bound::Lower => entry.value >= beta,
                    Bound::Upper => entry.value <= alpha,
                }
            {
                return Ok(Searched {
                    value: entry.value,
                    complete: entry.depth == COMPLETE,
                });
            }
        }
        let alpha_searched = alpha;

        // sort lazily: with a table move, all of them, that one first; else the
        // first only, and the rest once it has not cut off
        self.generate(ply, player_idx);
        let mut list = std::mem::take(&mut self.work_buffer.lists[ply]);
        let mut ordered = std::mem::take(&mut self.work_buffer.orders[ply]);
        let table_first = table_play < PASS_PLAY && (table_play as usize) < ordered.len();
        if table_first {
            order(&mut list, &mut ordered, 0, table_play);
        } else {
            order_first(&mut list, &mut ordered);
        }
        let mut best = Searched {
            value: f32::NEG_INFINITY,
            complete: true,
        };
        let mut best_play = NO_PLAY;
        let mut failed = None;
        for i in 0..ordered.len() {
            if i == 1 && !table_first {
                order(&mut list, &mut ordered[1..], 1, NO_PLAY);
            }
            let candidate = &list[ordered[i] as u32 as usize];
            let value = if candidate.plays_out {
                candidate.valuation
            } else {
                // the math goes like this:
                // child negamax returns v.
                // this parent negamax wants (score - v),
                // where alpha <= (score - v) <= beta.
                // so (score - beta) <= v <= (score - alpha).
                // since child_alpha <= v <= child_beta,
                // we set child_alpha = (score - beta)
                // and child_beta = (score - alpha).
                let score = candidate.valuation;
                self.place(player_idx, &candidate.play);
                let child = self.negamax_eval(
                    ply + 1,
                    player_idx ^ 1,
                    depth - 1,
                    score - beta,
                    score - alpha,
                    Scoreless::Counted {
                        passes: 0,
                        zeros: 0,
                    },
                );
                self.unplace(player_idx, &candidate.play);
                match child {
                    Ok(child) => {
                        best.complete &= child.complete;
                        score - child.value
                    }
                    Err(e) => {
                        failed = Some(e);
                        break;
                    }
                }
            };
            if value > best.value {
                best.value = value;
                best_play = candidate.index;
                if value > alpha {
                    alpha = value;
                }
                if value >= beta {
                    break;
                }
            }
        }
        self.work_buffer.lists[ply] = list;
        self.work_buffer.orders[ply] = ordered;
        if let Some(e) = failed {
            return Err(e);
        }

        // the pass, unless a move placing tiles has already reached beta: it ends
        // the line when it answers a pass or brings the scoreless turns to the
        // config's count, each player losing their rack
        if best.value < beta {
            let value = if self.pass_ends_the_line(scoreless) {
                self.both_pass_value(player_idx)
            } else {
                let child = self.negamax_eval(
                    ply + 1,
                    player_idx ^ 1,
                    depth - 1,
                    -beta,
                    -alpha,
                    Scoreless::Answering,
                )?;
                best.complete &= child.complete;
                -child.value
            };
            if value > best.value {
                best.value = value;
                best_play = PASS_PLAY;
            }
        }

        self.work_buffer.store(
            key,
            Entry {
                value: best.value,
                play: best_play,
                depth: if best.complete { COMPLETE } else { depth },
                bound: if best.value <= alpha_searched {
                    Bound::Upper
                } else if best.value >= beta {
                    Bound::Lower
                } else {
                    Bound::Exact
                },
            },
        );
        Ok(best)
    }

    // the principal variation of the deepest pass that finished: each move with
    // the value of the position it is played from, for its mover, read from the
    // table to the end of the line or as deep as that pass went; a position the
    // table cannot answer exactly is searched again with a full window
    #[inline]
    pub fn collect_pv(&mut self, player_idx: u8, out: &mut Vec<(equity::Equity, movegen::Play)>) {
        out.clear();
        let check_limits = std::mem::replace(&mut self.work_buffer.check_limits, false);
        self.reset_position();
        let mut player_idx = player_idx;
        let mut scoreless = self.entering();
        let mut depth = self.work_buffer.finished;
        while depth > 0 {
            let key = self.node_key(player_idx, scoreless);
            let fits = |entry: &Entry| {
                entry.bound == Bound::Exact && entry.depth >= depth && entry.play != NO_PLAY
            };
            let entry = match self.work_buffer.table.probe(key).filter(fits) {
                Some(entry) => entry,
                None => {
                    if self
                        .negamax_eval(
                            0,
                            player_idx,
                            depth,
                            f32::NEG_INFINITY,
                            f32::INFINITY,
                            scoreless,
                        )
                        .is_err()
                    {
                        break;
                    }
                    match self.work_buffer.table.probe(key).filter(fits) {
                        Some(entry) => entry,
                        None => break,
                    }
                }
            };
            let value = to_equity(entry.value);
            if entry.play == PASS_PLAY {
                out.push((
                    value,
                    movegen::Play::Exchange {
                        tiles: [][..].into(),
                    },
                ));
                // a pass answering a pass, or bringing the scoreless turns to the
                // config's count, ends the line
                if self.pass_ends_the_line(scoreless) {
                    break;
                }
                scoreless = Scoreless::Answering;
            } else {
                // the move the table keeps, generated again
                self.generate(0, player_idx);
                let work_buffer = &mut self.work_buffer;
                order(
                    &mut work_buffer.lists[0],
                    &mut work_buffer.orders[0],
                    0,
                    NO_PLAY,
                );
                let Some(candidate) = self.work_buffer.lists[0]
                    .iter()
                    .find(|candidate| candidate.index == entry.play)
                else {
                    break;
                };
                let play = candidate.play.clone();
                let plays_out = candidate.plays_out;
                self.place(player_idx, &play);
                out.push((value, play));
                if plays_out {
                    break;
                }
                scoreless = Scoreless::Counted {
                    passes: 0,
                    zeros: 0,
                };
            }
            player_idx ^= 1;
            depth -= 1;
        }
        self.work_buffer.check_limits = check_limits;
    }

    #[inline]
    pub fn print_best_line(&mut self, player_idx: u8) -> std::io::Result<()> {
        let mut line = Vec::new();
        self.collect_pv(player_idx, &mut line);
        let mut board_tiles = self.board_tiles.clone();
        let mut racks = self.racks.clone();
        let mut leftovers = [None, None];
        let mut i = 0usize;
        for (equity, play) in &line {
            let player_turn_idx = (player_idx as usize + i) & 1;
            let rack = &mut racks[player_turn_idx];
            leftovers[player_turn_idx ^ 1] = None;
            leftovers[player_turn_idx] = Some(equity::Equity::new(
                equity.raw()
                    - match play {
                        movegen::Play::Exchange { .. } => 0,
                        movegen::Play::Place { score, .. } => *score,
                    },
            ));
            writeln!(
                std::io::stdout(),
                "{}: p{}: {}{:width$} {} {}",
                i,
                player_turn_idx,
                self.game_config.alphabet().fmt_rack(rack),
                "",
                equity,
                play.fmt(&movegen::BoardSnapshot {
                    board_tiles: &board_tiles,
                    game_config: self.game_config,
                    kwg: self.kwg,
                    anagrams: self.arm_tables.anagrams(),
                    rack_lengths: self.arm_tables.rack_lengths(),
                    klv: &self.klv,
                }),
                width = self.game_config.rack_size() as usize - rack.len(),
            )?;
            if let movegen::Play::Place {
                down,
                lane,
                idx,
                word,
                score: _,
            } = play
            {
                let strider = self.game_config.board_layout().dim().lane(*down, *lane);

                // place the tiles
                for (i, &tile) in (*idx..).zip(word.iter()) {
                    if tile != 0 {
                        board_tiles[strider.at(i)] = tile;
                        let tombstone_idx =
                            rack.iter().rposition(|&t| t == held_tile(tile)).unwrap();
                        rack[tombstone_idx] = 0x80;
                    }
                }
                rack.retain(|&t| t != 0x80);
            }
            i += 1;
        }
        for _ in 0..2 {
            let player_turn_idx = (player_idx as usize + i) & 1;
            let rack = &racks[player_turn_idx];
            let leftover = leftovers[player_turn_idx];
            write!(
                std::io::stdout(),
                "e: p{}: {}",
                player_turn_idx,
                self.game_config.alphabet().fmt_rack(rack)
            )?;
            if let Some(leftover) = leftover {
                write!(
                    std::io::stdout(),
                    "{:width$} {}",
                    "",
                    leftover,
                    width = self.game_config.rack_size() as usize - rack.len(),
                )?;
            }
            writeln!(std::io::stdout())?;
            i += 1;
        }
        display::print_board(
            self.game_config.alphabet(),
            self.game_config.board_layout(),
            &board_tiles,
        )?;
        Ok(())
    }

    // send the progress, if anything is listening
    #[inline]
    fn report(&mut self, value: Option<equity::Equity>) -> error::Returns<()> {
        let work_buffer = &mut self.work_buffer;
        match work_buffer.progress.as_mut() {
            Some(progress_fn) => progress_fn(&Progress {
                endgames: work_buffer.in_bag,
                depth: work_buffer.max_depth,
                value,
                movegen: work_buffer.dur_movegen,
                nodes: work_buffer.searched,
                held: work_buffer.table.held,
                room: work_buffer.table.room(),
            }),
            None => Ok(()),
        }
    }

    #[inline]
    pub fn solve_one_in_bag(
        &mut self,
        player_idx: u8,
        bag_tile: u8,
    ) -> error::Returns<equity::Equity> {
        let limits = BagLimits::new(&Limits::NONE, self.work_buffer.progress.take());
        let mut exact = true;
        let value = self.one_in_bag(player_idx, bag_tile, &limits, &mut exact);
        self.work_buffer.progress = limits.progress.take();
        value
    }

    // an endgame inside a search with a tile in the bag, on that search's clock
    // and within its limits, telling its listener; one cut short makes the
    // search's answer inexact
    #[inline(always)]
    fn solve_in_bag(
        &mut self,
        player_idx: u8,
        limits: &BagLimits,
        exact: &mut bool,
    ) -> error::Returns<f32> {
        let work_buffer = &mut self.work_buffer;
        work_buffer.t0 = limits.t0;
        work_buffer.dur_movegen = limits.dur_movegen.get();
        work_buffer.in_bag = Some(limits.endgames.get());
        work_buffer.progress = limits.progress.take();
        let solved = self.run_id_loop(
            player_idx,
            false,
            &Limits {
                time: limits.time,
                states: limits.states,
                memory: limits.memory,
            },
        );
        let work_buffer = &mut self.work_buffer;
        limits.dur_movegen.set(work_buffer.dur_movegen);
        limits.endgames.set(limits.endgames.get() + 1);
        limits.progress.set(work_buffer.progress.take());
        work_buffer.in_bag = None;
        let solved = solved?;
        *exact &= solved.exact;
        Ok(solved.value.raw() as f32)
    }

    // the one-in-bag value within the limits; an endgame inside it that a limit
    // cut short clears exact
    fn one_in_bag(
        &mut self,
        player_idx: u8,
        bag_tile: u8,
        limits: &BagLimits,
        exact: &mut bool,
    ) -> error::Returns<equity::Equity> {
        let gc = self.game_config;
        let kwg = self.kwg;
        let arm_tables = self.arm_tables;

        let mut sub = EndgameSolver::<N, L>::new(gc, kwg, arm_tables)?;

        let mut mg = movegen::KurniaMoveGenerator::new(gc);

        let klv = klv::Klv::<L>::from_bytes_alloc(klv::EMPTY_KLV_BYTES)?;

        let board = self.board_tiles.clone();
        let racks = [self.racks[0].clone(), self.racks[1].clone()];

        // the endgames keep what they learn in this solver's table, lent to the
        // one solving them, so each knows what the others found
        std::mem::swap(&mut sub.work_buffer.table, &mut self.work_buffer.table);
        let mut search = OneInBag {
            gc,
            kwg,
            arm_tables,
            klv: &klv,
            sub: &mut sub,
            mg: &mut mg,
            memo: fash::MyHashMap::default(),
            limits,
            exact: true,
        };
        let [passes, zeros] = self.scoreless;
        let value = if one_in_bag_exchange_solvable(gc) {
            Self::one_in_bag_minimax_ex(
                &mut search,
                &board,
                &racks,
                player_idx,
                bag_tile,
                passes,
                zeros,
            )
        } else {
            Self::one_in_bag_minimax(
                &mut search,
                &board,
                &racks,
                player_idx,
                bag_tile,
                passes,
                zeros,
            )
        };
        *exact &= search.exact;
        std::mem::swap(&mut sub.work_buffer.table, &mut self.work_buffer.table);
        Ok(to_equity(value?))
    }

    // the one-in-bag value for the mover, who places and draws the tile, or
    // passes; passes and zeros are the scoreless turns before the position,
    // counted to the config's end as the game counts them, since a pass here is
    // a move the peg model values on its own, not a node's best
    fn one_in_bag_minimax(
        search: &mut OneInBag<'_, 'a, N, L>,
        board: &[u8],
        racks: &[Vec<u8>; 2],
        mover: u8,
        bag_tile: u8,
        passes: u8,
        zeros: u8,
    ) -> error::Returns<f32> {
        let alphabet = search.gc.alphabet();
        let opp = mover ^ 1;
        let snapshot = movegen::BoardSnapshot {
            board_tiles: board,
            game_config: search.gc,
            kwg: search.kwg,
            anagrams: search.arm_tables.anagrams(),
            rack_lengths: search.arm_tables.rack_lengths(),
            klv: search.klv,
        };
        search.mg.gen_moves_raw_all_unsorted(
            &snapshot,
            &racks[mover as usize],
            0,
            movegen::PassPolicy::AsACandidate,
        );

        let drawn = bag_tile & !((bag_tile as i8) >> 7) as u8;

        let mut best = f32::NEG_INFINITY;
        for vm in &search.mg.plays {
            if let movegen::Play::Place {
                down,
                lane,
                idx,
                word,
                score,
            } = &vm.play
            {
                let mut nb = board.to_vec();
                let mut nr = racks.clone();
                {
                    let rack = &mut nr[mover as usize];
                    let strider = search.gc.board_layout().dim().lane(*down, *lane);
                    for (i, &tile) in (*idx..).zip(word.iter()) {
                        if tile != 0 {
                            nb[strider.at(i)] = tile;
                            let blanked = tile & !((tile as i8) >> 7) as u8;
                            let p = rack.iter().rposition(|&t| t == blanked).unwrap();
                            rack[p] = 0x80;
                        }
                    }
                    rack.retain(|&t| t != 0x80);
                    rack.push(drawn);
                }

                search.sub.init(&nb, [&nr[0], &nr[1]]);
                let v = search
                    .sub
                    .solve_in_bag(opp, search.limits, &mut search.exact)?;

                let val = *score as f32 - v;
                if val > best {
                    best = val;
                }
            }
        }

        let pass_passes = passes.saturating_add(1);
        let pass_zeros = zeros.saturating_add(1);
        let pass_val = if scoreless_turns_end(search.gc, None, pass_passes, pass_zeros) {
            (alphabet.scaled_rack_score(&racks[opp as usize])
                - alphabet.scaled_rack_score(&racks[mover as usize])) as f32
        } else {
            -Self::one_in_bag_minimax(search, board, racks, opp, bag_tile, pass_passes, pass_zeros)?
        };
        if pass_val > best {
            best = pass_val;
        }
        Ok(best)
    }

    fn one_in_bag_minimax_ex(
        search: &mut OneInBag<'_, 'a, N, L>,
        board: &[u8],
        racks: &[Vec<u8>; 2],
        mover: u8,
        bag_tile: u8,
        passes: u8,
        zeros: u8,
    ) -> error::Returns<f32> {
        let mut rack0 = racks[0].clone();
        let mut rack1 = racks[1].clone();
        rack0.sort_unstable();
        rack1.sort_unstable();
        let key = BagExchangeKey {
            mover,
            bag_tile,
            passes,
            zeros,
            rack0,
            rack1,
        };
        if let Some(&v) = search.memo.get(&key) {
            return Ok(v);
        }

        let alphabet = search.gc.alphabet();
        let opp = mover ^ 1;
        let snapshot = movegen::BoardSnapshot {
            board_tiles: board,
            game_config: search.gc,
            kwg: search.kwg,
            anagrams: search.arm_tables.anagrams(),
            rack_lengths: search.arm_tables.rack_lengths(),
            klv: search.klv,
        };
        search.mg.gen_moves_raw_all_unsorted(
            &snapshot,
            &racks[mover as usize],
            0,
            movegen::PassPolicy::AsACandidate,
        );

        let mut places: Vec<movegen::Play> = Vec::new();
        for vm in &search.mg.plays {
            if let movegen::Play::Place { .. } = &vm.play {
                places.push(vm.play.clone());
            }
        }

        let drawn = bag_tile & !((bag_tile as i8) >> 7) as u8;

        let mut best = f32::NEG_INFINITY;

        for play in &places {
            if let movegen::Play::Place {
                down,
                lane,
                idx,
                word,
                score,
            } = play
            {
                let mut nb = board.to_vec();
                let mut nr = racks.clone();
                {
                    let rack = &mut nr[mover as usize];
                    let strider = search.gc.board_layout().dim().lane(*down, *lane);
                    for (i, &tile) in (*idx..).zip(word.iter()) {
                        if tile != 0 {
                            nb[strider.at(i)] = tile;
                            let blanked = tile & !((tile as i8) >> 7) as u8;
                            let p = rack.iter().rposition(|&t| t == blanked).unwrap();
                            rack[p] = 0x80;
                        }
                    }
                    rack.retain(|&t| t != 0x80);
                    rack.push(drawn);
                }
                search.sub.init(&nb, [&nr[0], &nr[1]]);
                let v = search
                    .sub
                    .solve_in_bag(opp, search.limits, &mut search.exact)?;
                let val = *score as f32 - v;
                if val > best {
                    best = val;
                }
            }
        }

        let pass_passes = passes.saturating_add(1);
        let pass_zeros = zeros.saturating_add(1);
        let pass_val = if scoreless_turns_end(search.gc, None, pass_passes, pass_zeros) {
            (alphabet.scaled_rack_score(&racks[opp as usize])
                - alphabet.scaled_rack_score(&racks[mover as usize])) as f32
        } else {
            -Self::one_in_bag_minimax_ex(
                search,
                board,
                racks,
                opp,
                bag_tile,
                pass_passes,
                pass_zeros,
            )?
        };
        if pass_val > best {
            best = pass_val;
        }

        let exch_zeros = zeros + 1;
        let mut seen: u64 = 0; // bitset over rack tile values 0..=63
        for i in 0..racks[mover as usize].len() {
            let r = racks[mover as usize][i];
            if r == drawn {
                continue;
            }
            let bit = 1u64 << (r & 0x3f);
            if seen & bit != 0 {
                continue;
            }
            seen |= bit;

            let mut nr = racks.clone();
            {
                let rack = &mut nr[mover as usize];
                let p = rack.iter().position(|&t| t == r).unwrap();
                rack[p] = drawn;
            }
            let exch_val = if scoreless_turns_end(search.gc, None, 0, exch_zeros) {
                (alphabet.scaled_rack_score(&nr[opp as usize])
                    - alphabet.scaled_rack_score(&nr[mover as usize])) as f32
            } else {
                -Self::one_in_bag_minimax_ex(search, board, &nr, opp, r, 0, exch_zeros)?
            };
            if exch_val > best {
                best = exch_val;
            }
        }

        search.memo.insert(key, best);
        Ok(best)
    }

    #[inline]
    pub fn solve_peg_one_in_bag(
        &mut self,
        mover: u8,
        board: &[u8],
        mover_rack: &[u8],
        unseen_tally: &[u8],
        score_diff: equity::Equity,
        limits: &Limits,
    ) -> error::Returns<Result<PegResult, PegUnsupported>> {
        let gc = self.game_config;
        if one_in_bag_exchange_legal(gc) && !one_in_bag_exchange_solvable(gc) {
            return Ok(Err(PegUnsupported::ExchangeWithoutForcedEnd));
        }
        let score_diff = score_diff.raw() as f32;
        let limits = BagLimits::new(limits, self.work_buffer.progress.take());

        let result = if one_in_bag_exchange_legal(gc) {
            self.peg_clairvoyant_aggregate(
                mover,
                board,
                mover_rack,
                unseen_tally,
                score_diff,
                &limits,
            )
        } else {
            let unseen: Vec<(u8, u32)> = unseen_tally
                .iter()
                .enumerate()
                .filter(|&(_, &c)| c != 0)
                .map(|(t, &c)| (t as u8, c as u32))
                .collect();
            self.peg_committed_no_exchange(mover, board, mover_rack, &unseen, score_diff, &limits)
        };
        self.work_buffer.progress = limits.progress.take();
        Ok(Ok(result?))
    }

    #[inline]
    fn peg_clairvoyant_aggregate(
        &mut self,
        mover: u8,
        board: &[u8],
        mover_rack: &[u8],
        unseen_tally: &[u8],
        score_diff: f32,
        limits: &BagLimits,
    ) -> error::Returns<PegResult> {
        let mut hypotheses: Vec<(u8, u32, equity::Equity)> = Vec::new();
        let mut exact = true;

        let mut opp_rack: Vec<u8> = Vec::new();
        for (t, &count) in unseen_tally.iter().enumerate() {
            if count == 0 {
                continue;
            }
            let bag_tile = t as u8;

            opp_rack.clear();
            for (u, &c) in unseen_tally.iter().enumerate() {
                let take = if u == t { c - 1 } else { c } as usize;
                opp_rack.extend(std::iter::repeat_n(u as u8, take));
            }
            let mut racks: [&[u8]; 2] = [&[], &[]];
            racks[mover as usize] = mover_rack;
            racks[(mover ^ 1) as usize] = &opp_rack;
            self.init(board, racks);
            let v = self.one_in_bag(mover, bag_tile, limits, &mut exact)?.raw() as f32 + score_diff;
            hypotheses.push((bag_tile, count as u32, to_equity(v)));
        }
        let weighted: Vec<(u32, equity::Equity)> =
            hypotheses.iter().map(|&(_, w, v)| (w, v)).collect();
        let (win_pct, expected_margin) = peg_aggregate(&weighted);
        Ok(PegResult {
            win_pct,
            expected_margin,
            hypotheses,
            best_move: None,
            committed: false,
            exact,
        })
    }

    #[inline]
    fn peg_committed_no_exchange(
        &mut self,
        mover: u8,
        board: &[u8],
        mover_rack: &[u8],
        unseen: &[(u8, u32)],
        score_diff: f32,
        limits: &BagLimits,
    ) -> error::Returns<PegResult> {
        let mut sub = EndgameSolver::<N, L>::new(self.game_config, self.kwg, self.arm_tables)?;
        // the endgames keep what they learn in this solver's table, lent to the
        // one solving them, so each knows what the others found
        std::mem::swap(&mut sub.work_buffer.table, &mut self.work_buffer.table);
        let result = Self::peg_committed_by(
            &mut sub,
            &PegPosition {
                mover,
                board,
                mover_rack,
                unseen,
                scoreless: self.scoreless,
            },
            score_diff,
            limits,
        );
        std::mem::swap(&mut sub.work_buffer.table, &mut self.work_buffer.table);
        result
    }

    // the committed model's search, its endgames solved by sub
    #[inline]
    fn peg_committed_by(
        sub: &mut Self,
        position: &PegPosition<'_>,
        score_diff: f32,
        limits: &BagLimits,
    ) -> error::Returns<PegResult> {
        let PegPosition {
            mover,
            board,
            mover_rack,
            unseen,
            scoreless,
        } = *position;
        let gc = sub.game_config;
        let kwg = sub.kwg;
        let arm_tables = sub.arm_tables;
        let klv = klv::Klv::<L>::from_bytes_alloc(klv::EMPTY_KLV_BYTES)?;
        let alphabet = gc.alphabet();
        let opp = mover ^ 1;
        let mut exact = true;
        let total = unseen.iter().map(|&(_, w)| w).sum::<u32>() as f32;
        let pass_passes = scoreless[0].saturating_add(1);
        let pass_zeros = scoreless[1].saturating_add(1);
        let pass_ends_the_game = scoreless_turns_end(gc, None, pass_passes, pass_zeros);

        let mut mg = movegen::KurniaMoveGenerator::new(gc);
        mg.gen_moves_raw_all_unsorted(
            &movegen::BoardSnapshot {
                board_tiles: board,
                game_config: gc,
                kwg,
                anagrams: arm_tables.anagrams(),
                rack_lengths: arm_tables.rack_lengths(),
                klv: &klv,
            },
            mover_rack,
            0,
            movegen::PassPolicy::AsACandidate,
        );
        let mut candidates: Vec<PegMove> = Vec::with_capacity(mg.plays.len() + 1);
        candidates.push(PegMove::Pass);
        for vm in &mg.plays {
            if matches!(vm.play, movegen::Play::Place { .. }) {
                candidates.push(PegMove::Place(vm.play.clone()));
            }
        }

        let mut best_win = f32::NEG_INFINITY;
        let mut best_marg = f32::NEG_INFINITY;
        let mut best_move = PegMove::Pass;
        let mut best_hyps: Vec<(u8, u32, equity::Equity)> = Vec::new();

        for cand in &candidates {
            let mut win = 0.0f32;
            let mut marg = 0.0f32;
            let mut hyps: Vec<(u8, u32, equity::Equity)> = Vec::with_capacity(unseen.len());
            for &(t, w) in unseen {
                let drawn = t & !((t as i8) >> 7) as u8;

                let mut opp_rack: Vec<u8> = Vec::new();
                for &(u, c) in unseen {
                    let take = if u == t { c - 1 } else { c };
                    for _ in 0..take {
                        opp_rack.push(u);
                    }
                }
                let v = match cand {
                    PegMove::Place(movegen::Play::Place {
                        down,
                        lane,
                        idx,
                        word,
                        score,
                    }) => {
                        let mut nb = board.to_vec();
                        let mut mr = mover_rack.to_vec();
                        let strider = gc.board_layout().dim().lane(*down, *lane);
                        for (i, &tile) in (*idx..).zip(word.iter()) {
                            if tile != 0 {
                                nb[strider.at(i)] = tile;
                                let b = tile & !((tile as i8) >> 7) as u8;
                                let p = mr.iter().rposition(|&x| x == b).unwrap();
                                mr[p] = 0x80;
                            }
                        }
                        mr.retain(|&x| x != 0x80);
                        mr.push(drawn);
                        let mut r2: [Vec<u8>; 2] = [Vec::new(), Vec::new()];
                        r2[mover as usize] = mr;
                        r2[opp as usize] = opp_rack;
                        sub.init(&nb, [&r2[0], &r2[1]]);
                        *score as f32 - sub.solve_in_bag(opp, limits, &mut exact)?
                    }
                    // the pass ends the game at once when it brings the scoreless
                    // turns to the config's count, each losing their rack
                    PegMove::Pass if pass_ends_the_game => {
                        (alphabet.scaled_rack_score(&opp_rack)
                            - alphabet.scaled_rack_score(mover_rack)) as f32
                    }
                    PegMove::Pass => {
                        // PEG2-7: with two or more in the bag, draw every tile from the SAME end -- pop,
                        // the back -- and store a scenario reversed to match. Do NOT reuse Bag::replenish
                        // for the enumeration: it pops for even players and shifts for odd ones, which on
                        // a reversed bag hands p1 the leftover instead of the next popped tile.
                        let mut r2: [Vec<u8>; 2] = [Vec::new(), Vec::new()];
                        r2[mover as usize] = mover_rack.to_vec();
                        r2[opp as usize] = opp_rack;
                        let mut search = OneInBag {
                            gc,
                            kwg,
                            arm_tables,
                            klv: &klv,
                            sub: &mut *sub,
                            mg: &mut mg,
                            memo: fash::MyHashMap::default(),
                            limits,
                            exact: true,
                        };
                        let v = -Self::one_in_bag_minimax(
                            &mut search,
                            board,
                            &r2,
                            opp,
                            t,
                            pass_passes,
                            pass_zeros,
                        )?;
                        exact &= search.exact;
                        v
                    }
                    PegMove::Place(_) => {
                        unreachable!("Place candidate always holds a Play::Place")
                    }
                };
                let vv = v + score_diff;
                marg += w as f32 * vv;
                win += w as f32
                    * if vv > 0.0 {
                        1.0
                    } else if vv == 0.0 {
                        0.5
                    } else {
                        0.0
                    };
                hyps.push((t, w, to_equity(vv)));
            }
            win /= total;
            marg /= total;

            if win > best_win || (win == best_win && marg > best_marg) {
                best_win = win;
                best_marg = marg;
                best_move = cand.clone();
                best_hyps = hyps;
            }
        }

        Ok(PegResult {
            win_pct: best_win,
            expected_margin: to_equity(best_marg),
            hypotheses: best_hyps,
            best_move: Some(best_move),
            committed: true,
            exact,
        })
    }
}

#[derive(Clone)]
pub enum PegMove {
    Pass,
    Place(movegen::Play),
}

pub struct PegResult {
    pub win_pct: f32,
    pub expected_margin: equity::Equity,
    pub hypotheses: Vec<(u8, u32, equity::Equity)>,
    pub best_move: Option<PegMove>,
    pub committed: bool,
    pub exact: bool, // false when a limit cut an endgame inside it short
}

#[derive(Debug)]
pub enum PegUnsupported {
    ExchangeWithoutForcedEnd,
}

impl std::fmt::Display for PegUnsupported {
    #[inline]
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PegUnsupported::ExchangeWithoutForcedEnd => f.write_str(
                "exchange is legal with one tile in the bag but scoreless turns \
                 never force the game to end, so this position is not solvable",
            ),
        }
    }
}

// the one-in-bag search's fixed parts, carried down its recursion
struct OneInBag<'s, 'a, N: kwg::Node, L: kwg::Node> {
    gc: &'s game_config::GameConfig,
    kwg: &'s kwg::Kwg<N>,
    arm_tables: &'s anagrams::ArmTables,
    klv: &'s klv::Klv<L>,
    sub: &'s mut EndgameSolver<'a, N, L>,
    mg: &'s mut movegen::KurniaMoveGenerator,
    memo: fash::MyHashMap<BagExchangeKey, f32>,
    limits: &'s BagLimits,
    exact: bool, // cleared by an endgame a limit cut short
}

#[derive(Eq, Hash, PartialEq)]
struct BagExchangeKey {
    mover: u8,
    bag_tile: u8,
    passes: u8,
    zeros: u8,
    rack0: Vec<u8>,
    rack1: Vec<u8>,
}

// a position with a tile in the bag as the peg model searches it: the mover,
// the board, the mover's rack, the tiles they cannot see with each one's count,
// and the scoreless turns before it (the passes and the zeros)
#[derive(Clone, Copy)]
struct PegPosition<'a> {
    mover: u8,
    board: &'a [u8],
    mover_rack: &'a [u8],
    unseen: &'a [(u8, u32)],
    scoreless: [u8; 2],
}

#[inline(always)]
fn one_in_bag_exchange_legal(gc: &game_config::GameConfig) -> bool {
    gc.exchange_tile_limit() <= 1
}

#[inline(always)]
fn one_in_bag_exchange_solvable(gc: &game_config::GameConfig) -> bool {
    one_in_bag_exchange_legal(gc) && gc.exchanges_are_zeros() && gc.num_zeros_to_end() != 0
}

// whether these scoreless turns (the passes, and the zeros, which count the
// passes too) end the game by the config's counts: a count of zero never ends
// it. Given the board, the zeros' count is barred on an empty one when the
// config says so, as the game itself counts; the searches with a tile in the
// bag give none, since their boards hold tiles in any game and their lines have
// no depth to cap them
#[inline]
fn scoreless_turns_end(
    gc: &game_config::GameConfig,
    board: Option<&[u8]>,
    passes: u8,
    zeros: u8,
) -> bool {
    let npte = gc.num_passes_to_end();
    let nzte = gc.num_zeros_to_end();
    (npte != 0 && passes >= npte)
        || (nzte != 0
            && zeros >= nzte
            && board.is_none_or(|board| {
                gc.zeros_can_end_empty_board() || board.iter().any(|&tile| tile != 0)
            }))
}

#[inline]
pub fn peg_aggregate(hypotheses: &[(u32, equity::Equity)]) -> (f32, equity::Equity) {
    let mut total = 0.0f32;
    let mut win_sum = 0.0f32;
    let mut margin_sum = 0.0f32;
    for &(weight, value) in hypotheses {
        let w = weight as f32;
        let win_score = match value.cmp(&equity::Equity::ZERO) {
            std::cmp::Ordering::Greater => 1.0,
            std::cmp::Ordering::Equal => 0.5,
            std::cmp::Ordering::Less => 0.0,
        };
        total += w;
        win_sum += win_score * w;
        margin_sum += value.raw() as f32 * w;
    }
    if total == 0.0 {
        (0.0, equity::Equity::ZERO)
    } else {
        (win_sum / total, to_equity(margin_sum / total))
    }
}

#[cfg(test)]
mod tests {
    use super::{EndgameSolver, PegMove, splitmix64};
    use crate::{alphabet, anagrams, bites, build, equity, game_config, klv, kwg, movegen};

    #[inline]
    fn tiny_word_list() -> Vec<bites::Bites> {
        let gc = game_config::make_english_game_config();
        let reader = alphabet::AlphabetReader::new_for_words(gc.alphabet());
        let word_strs = [
            "AA", "AB", "AH", "AT", "BA", "HA", "TA", "AAH", "ABA", "BAA", "BAT", "TAB", "TAT",
            "HAT", "THAT", "AAHS", "BATH", "HAH", "AAL",
        ];
        let mut words = Vec::<bites::Bites>::new();
        let mut buf = Vec::new();
        for w in word_strs {
            if reader.set_word(w, &mut buf).is_ok() {
                words.push(buf[..].into());
            }
        }
        words.sort_unstable();
        words.dedup();
        words
    }

    #[inline]
    fn tiny_kwg_bytes() -> bites::Bites {
        build::build(
            build::BuildContent::Gaddawg,
            build::BuildLayout::Wolges,
            build::BuildOrder::Sorted,
            &tiny_word_list(),
        )
        .unwrap()
    }

    // each arm, flagged true for the lean one
    #[inline]
    fn both_arms(
        kwg: &kwg::Kwg<kwg::Node22>,
        gc: &game_config::GameConfig,
    ) -> [(bool, anagrams::ArmTables); 2] {
        [false, true].map(|lean| (lean, anagrams::ArmTables::build(kwg, gc, lean).unwrap()))
    }

    #[inline]
    fn apply_place(
        gc: &game_config::GameConfig,
        board: &mut [u8],
        rack: &mut Vec<u8>,
        down: bool,
        lane: i8,
        idx: i8,
        word: &[u8],
    ) {
        let strider = gc.board_layout().dim().lane(down, lane);
        for (i, &tile) in (idx..).zip(word.iter()) {
            if tile != 0 {
                board[strider.at(i)] = tile;
                let blanked = tile & !((tile as i8) >> 7) as u8;
                let p = rack.iter().rposition(|&t| t == blanked).unwrap();
                rack[p] = 0x80;
            }
        }
        rack.retain(|&t| t != 0x80);
    }

    // a state a reference solve has valued: the kind of search, the board, the
    // racks, the mover, the bag's tile, the passes, the zeros
    type ReferenceState = (u8, Vec<u8>, Vec<u8>, Vec<u8>, u8, u8, u8, u8);

    // a reference solve's fixed parts, carried down its recursion, and the
    // states it has valued, since the scoreless turns the config counts to make
    // the pass lines many
    struct Reference<'s, N: kwg::Node, L: kwg::Node> {
        gc: &'s game_config::GameConfig,
        kwg: &'s kwg::Kwg<N>,
        klv: &'s klv::Klv<L>,
        mg: &'s mut movegen::KurniaMoveGenerator,
        memo: super::fash::MyHashMap<ReferenceState, f32>,
    }

    // the value of the position for the mover, by a plain search with no table:
    // passes and zeros are the scoreless turns so far, ending the game at the
    // config's counts
    fn reference<N: kwg::Node, L: kwg::Node>(
        fixed: &mut Reference<'_, N, L>,
        board: &[u8],
        racks: &[Vec<u8>; 2],
        mover: usize,
        passes: u8,
        zeros: u8,
    ) -> f32 {
        let key = (
            0u8,
            board.to_vec(),
            racks[0].clone(),
            racks[1].clone(),
            mover as u8,
            0u8,
            passes,
            zeros,
        );
        if let Some(&value) = fixed.memo.get(&key) {
            return value;
        }
        let alphabet = fixed.gc.alphabet();
        let snapshot = movegen::BoardSnapshot {
            board_tiles: board,
            game_config: fixed.gc,
            kwg: fixed.kwg,
            anagrams: None,
            rack_lengths: None,
            klv: fixed.klv,
        };
        fixed.mg.gen_moves_raw_all_unsorted(
            &snapshot,
            &racks[mover],
            0,
            movegen::PassPolicy::AsACandidate,
        );

        let mut places: Vec<movegen::Play> = Vec::new();
        for vm in &fixed.mg.plays {
            if let movegen::Play::Place { .. } = &vm.play {
                places.push(vm.play.clone());
            }
        }

        let opp = mover ^ 1;
        let mut best = f32::NEG_INFINITY;
        for play in &places {
            if let movegen::Play::Place {
                down,
                lane,
                idx,
                word,
                score,
            } = play
            {
                let placed = word.iter().filter(|&&t| t != 0).count();
                let val = if placed == racks[mover].len() {
                    (*score + 2 * alphabet.scaled_rack_score(&racks[opp])) as f32
                } else {
                    let mut nb = board.to_vec();
                    let mut nr = racks.clone();
                    apply_place(fixed.gc, &mut nb, &mut nr[mover], *down, *lane, *idx, word);
                    *score as f32 - reference(fixed, &nb, &nr, opp, 0, 0)
                };
                if val > best {
                    best = val;
                }
            }
        }

        let pass_passes = passes.saturating_add(1);
        let pass_zeros = zeros.saturating_add(1);
        let pass_val = if super::scoreless_turns_end(fixed.gc, Some(board), pass_passes, pass_zeros)
        {
            (alphabet.scaled_rack_score(&racks[opp]) - alphabet.scaled_rack_score(&racks[mover]))
                as f32
        } else {
            -reference(fixed, board, racks, opp, pass_passes, pass_zeros)
        };
        if pass_val > best {
            best = pass_val;
        }
        fixed.memo.insert(key, best);
        best
    }

    fn reference_one_in_bag<N: kwg::Node, L: kwg::Node>(
        fixed: &mut Reference<'_, N, L>,
        board: &[u8],
        racks: &[Vec<u8>; 2],
        mover: usize,
        bag_tile: u8,
        passes: u8,
        zeros: u8,
    ) -> f32 {
        let key = (
            1u8,
            board.to_vec(),
            racks[0].clone(),
            racks[1].clone(),
            mover as u8,
            bag_tile,
            passes,
            zeros,
        );
        if let Some(&value) = fixed.memo.get(&key) {
            return value;
        }
        let alphabet = fixed.gc.alphabet();
        let snapshot = movegen::BoardSnapshot {
            board_tiles: board,
            game_config: fixed.gc,
            kwg: fixed.kwg,
            anagrams: None,
            rack_lengths: None,
            klv: fixed.klv,
        };
        fixed.mg.gen_moves_raw_all_unsorted(
            &snapshot,
            &racks[mover],
            0,
            movegen::PassPolicy::AsACandidate,
        );

        let mut places: Vec<movegen::Play> = Vec::new();
        for vm in &fixed.mg.plays {
            if let movegen::Play::Place { .. } = &vm.play {
                places.push(vm.play.clone());
            }
        }

        let drawn = bag_tile & !((bag_tile as i8) >> 7) as u8;

        let opp = mover ^ 1;
        let mut best = f32::NEG_INFINITY;
        for play in &places {
            if let movegen::Play::Place {
                down,
                lane,
                idx,
                word,
                score,
            } = play
            {
                let mut nb = board.to_vec();
                let mut nr = racks.clone();
                apply_place(fixed.gc, &mut nb, &mut nr[mover], *down, *lane, *idx, word);
                nr[mover].push(drawn);

                let val = *score as f32 - reference(fixed, &nb, &nr, opp, 0, 0);
                if val > best {
                    best = val;
                }
            }
        }

        let pass_passes = passes.saturating_add(1);
        let pass_zeros = zeros.saturating_add(1);
        let pass_val = if super::scoreless_turns_end(fixed.gc, None, pass_passes, pass_zeros) {
            (alphabet.scaled_rack_score(&racks[opp]) - alphabet.scaled_rack_score(&racks[mover]))
                as f32
        } else {
            -reference_one_in_bag(fixed, board, racks, opp, bag_tile, pass_passes, pass_zeros)
        };
        if pass_val > best {
            best = pass_val;
        }
        fixed.memo.insert(key, best);
        best
    }

    fn reference_one_in_bag_ex<N: kwg::Node, L: kwg::Node>(
        fixed: &mut Reference<'_, N, L>,
        board: &[u8],
        racks: &[Vec<u8>; 2],
        mover: usize,
        bag_tile: u8,
        passes: u8,
        zeros: u8,
    ) -> f32 {
        let key = (
            2u8,
            board.to_vec(),
            racks[0].clone(),
            racks[1].clone(),
            mover as u8,
            bag_tile,
            passes,
            zeros,
        );
        if let Some(&value) = fixed.memo.get(&key) {
            return value;
        }
        let alphabet = fixed.gc.alphabet();
        let snapshot = movegen::BoardSnapshot {
            board_tiles: board,
            game_config: fixed.gc,
            kwg: fixed.kwg,
            anagrams: None,
            rack_lengths: None,
            klv: fixed.klv,
        };
        fixed.mg.gen_moves_raw_all_unsorted(
            &snapshot,
            &racks[mover],
            0,
            movegen::PassPolicy::AsACandidate,
        );
        let mut places: Vec<movegen::Play> = Vec::new();
        for vm in &fixed.mg.plays {
            if let movegen::Play::Place { .. } = &vm.play {
                places.push(vm.play.clone());
            }
        }

        let drawn = bag_tile & !((bag_tile as i8) >> 7) as u8;
        let opp = mover ^ 1;
        let mut best = f32::NEG_INFINITY;
        for play in &places {
            if let movegen::Play::Place {
                down,
                lane,
                idx,
                word,
                score,
            } = play
            {
                let mut nb = board.to_vec();
                let mut nr = racks.clone();
                apply_place(fixed.gc, &mut nb, &mut nr[mover], *down, *lane, *idx, word);
                nr[mover].push(drawn);
                let val = *score as f32 - reference(fixed, &nb, &nr, opp, 0, 0);
                if val > best {
                    best = val;
                }
            }
        }

        let pass_passes = passes + 1;
        let pass_zeros = zeros + 1;
        let pass_val = if super::scoreless_turns_end(fixed.gc, None, pass_passes, pass_zeros) {
            (alphabet.scaled_rack_score(&racks[opp]) - alphabet.scaled_rack_score(&racks[mover]))
                as f32
        } else {
            -reference_one_in_bag_ex(fixed, board, racks, opp, bag_tile, pass_passes, pass_zeros)
        };
        if pass_val > best {
            best = pass_val;
        }

        let exch_zeros = zeros + 1;
        let mut seen: u64 = 0;
        for i in 0..racks[mover].len() {
            let r = racks[mover][i];
            if r == drawn {
                continue;
            }
            let bit = 1u64 << (r & 0x3f);
            if seen & bit != 0 {
                continue;
            }
            seen |= bit;
            let mut nr = racks.clone();
            {
                let rack = &mut nr[mover];
                let p = rack.iter().position(|&t| t == r).unwrap();
                rack[p] = drawn;
            }
            let exch_val = if super::scoreless_turns_end(fixed.gc, None, 0, exch_zeros) {
                (alphabet.scaled_rack_score(&nr[opp]) - alphabet.scaled_rack_score(&nr[mover]))
                    as f32
            } else {
                -reference_one_in_bag_ex(fixed, board, &nr, opp, r, 0, exch_zeros)
            };
            if exch_val > best {
                best = exch_val;
            }
        }
        fixed.memo.insert(key, best);
        best
    }

    struct Position {
        board: Vec<u8>,
        racks: [Vec<u8>; 2],
    }

    #[inline(always)]
    fn empty_board() -> Vec<u8> {
        vec![0u8; 15 * 15]
    }

    #[inline]
    fn put_word(board: &mut [u8], row: i8, col0: i8, word: &[u8]) {
        for (k, &t) in word.iter().enumerate() {
            board[(row as usize) * 15 + col0 as usize + k] = t;
        }
    }

    #[inline]
    fn describe(gc: &game_config::GameConfig, pos: &Position) -> String {
        let a = gc.alphabet();
        let mut s = String::new();
        s.push_str("board:");
        for (i, &t) in pos.board.iter().enumerate() {
            if t != 0 {
                let r = i / 15;
                let c = i % 15;
                s.push_str(&format!(" r{r}c{c}={}", a.of_board(t).unwrap_or("?")));
            }
        }
        s.push_str(&format!(
            " rack0=[{}] rack1=[{}]",
            a.fmt_rack(&pos.racks[0]),
            a.fmt_rack(&pos.racks[1]),
        ));
        s.push_str(&format!(
            " bytes board={:?} r0={:?} r1={:?}",
            pos.board
                .iter()
                .enumerate()
                .filter(|&(_, &t)| t != 0)
                .map(|(i, &t)| (i, t))
                .collect::<Vec<_>>(),
            pos.racks[0],
            pos.racks[1]
        ));
        s
    }

    #[inline]
    fn embedded_positions() -> Vec<(String, Position)> {
        let mut out = Vec::new();

        out.push((
            "both-pass-only (BB vs HT)".to_string(),
            Position {
                board: empty_board(),
                racks: [vec![2, 2], vec![8, 20]],
            },
        ));

        out.push((
            "both-pass-only (BB vs BH)".to_string(),
            Position {
                board: empty_board(),
                racks: [vec![2, 2], vec![2, 8]],
            },
        ));

        {
            let mut b = empty_board();
            put_word(&mut b, 7, 6, &[1, 20]); // A T at r7 c6,c7
            out.push((
                "one-side-pass (board AT; p0=[B] p1=[B])".to_string(),
                Position {
                    board: b,
                    racks: [vec![2], vec![2]],
                },
            ));
        }

        out.push((
            "playout race (AB vs AT)".to_string(),
            Position {
                board: empty_board(),
                racks: [vec![1, 2], vec![1, 20]],
            },
        ));

        {
            let mut b = empty_board();
            put_word(&mut b, 7, 6, &[1, 20]); // AT
            out.push((
                "playout with board (p0=[B,A] p1=[H,A])".to_string(),
                Position {
                    board: b,
                    racks: [vec![2, 1], vec![8, 1]],
                },
            ));
        }

        {
            let mut b = empty_board();
            put_word(&mut b, 7, 6, &[1, 1, 8]); // A A H (AAH)
            out.push((
                "3-tile (board AAH; p0=[B,A,T] p1=[H,A,T])".to_string(),
                Position {
                    board: b,
                    racks: [vec![2, 1, 20], vec![8, 1, 20]],
                },
            ));
        }

        {
            let mut b = empty_board();
            put_word(&mut b, 7, 6, &[1, 20]); // AT
            out.push((
                "finished game, the mover's rack empty (board AT; p0=[] p1=[H])".to_string(),
                Position {
                    board: b,
                    racks: [vec![], vec![8]],
                },
            ));
        }

        {
            let mut b = empty_board();
            put_word(&mut b, 7, 6, &[1, 20]); // AT
            out.push((
                "finished game, the other's rack empty (board AT; p0=[H,B] p1=[])".to_string(),
                Position {
                    board: b,
                    racks: [vec![8, 2], vec![]],
                },
            ));
        }

        out
    }

    #[inline]
    fn random_positions(n: usize) -> Vec<(String, Position)> {
        let letters = [1u8, 2, 20, 8]; // A B T H
        let words: Vec<Vec<u8>> = vec![
            vec![1, 20],     // AT
            vec![1, 1],      // AA
            vec![8, 1],      // HA
            vec![1, 1, 8],   // AAH
            vec![2, 1, 20],  // BAT
            vec![20, 1, 2],  // TAB
            vec![20, 1, 20], // TAT
        ];
        let mut seed = 0u64;
        let mut out = Vec::new();
        let mut made = 0usize;
        while made < n {
            let mut b = empty_board();

            let w = &words[(splitmix64(&mut seed) as usize) % words.len()];
            let row = (splitmix64(&mut seed) % 15) as i8;
            let col0 = (splitmix64(&mut seed) % (15 - w.len() as u64)) as i8;
            put_word(&mut b, row, col0, w);

            let mut racks: [Vec<u8>; 2] = [Vec::new(), Vec::new()];
            for rack in racks.iter_mut() {
                let size = if splitmix64(&mut seed).is_multiple_of(5) {
                    3
                } else {
                    2
                };
                for _ in 0..size {
                    let roll = splitmix64(&mut seed);
                    let t = if roll.is_multiple_of(11) {
                        0u8 // blank
                    } else {
                        letters[(roll as usize / 11) % letters.len()]
                    };
                    rack.push(t);
                }
                rack.sort_unstable();
            }
            if racks[0].is_empty() || racks[1].is_empty() {
                continue;
            }
            out.push((format!("rand#{made}"), Position { board: b, racks }));
            made += 1;
        }
        out
    }

    #[inline]
    fn all_positions() -> Vec<(String, Position)> {
        let mut v = embedded_positions();
        v.extend(random_positions(200));
        v
    }

    // sort a ply's whole list, as the search does past the first move
    #[inline]
    fn order_ply(
        egs: &mut EndgameSolver<'_, kwg::Node22, kwg::Node22>,
        ply: usize,
        table_play: u16,
    ) {
        let work_buffer = &mut egs.work_buffer;
        super::order(
            &mut work_buffer.lists[ply],
            &mut work_buffer.orders[ply],
            0,
            table_play,
        );
    }

    #[test]
    #[inline]
    fn order_first_agrees_with_order() {
        // order_first picks the move order sorts first, ties included, and
        // gives it index 0
        let gc = game_config::make_english_game_config();
        let kwg_bytes = tiny_kwg_bytes();
        let kwg = kwg::Kwg::<kwg::Node22>::from_bytes_alloc(&kwg_bytes).unwrap();
        let arm_tables = anagrams::ArmTables::build(&kwg, &gc, false).unwrap();
        let mut egs =
            EndgameSolver::<kwg::Node22, kwg::Node22>::new(&gc, &kwg, &arm_tables).unwrap();
        let mut firsts = 0usize;
        let mut tied = 0usize;
        for (name, pos) in all_positions() {
            for mover in 0..2u8 {
                egs.init(&pos.board, [&pos.racks[0][..], &pos.racks[1][..]]);
                egs.reset_position();
                egs.generate(0, mover);
                let mut keys = egs.work_buffer.orders[0].clone();
                super::order_first(&mut egs.work_buffer.lists[0], &mut keys);
                let Some(&key) = keys.first() else {
                    continue;
                };
                let first = egs.work_buffer.lists[0][key as u32 as usize].play.clone();
                assert_eq!(
                    egs.work_buffer.lists[0][key as u32 as usize].index, 0,
                    "[{name}] the first move's index"
                );
                if keys.iter().filter(|&&k| k >> 32 == key >> 32).count() > 1 {
                    tied += 1;
                }
                order_ply(&mut egs, 0, super::NO_PLAY);
                let whole = egs.work_buffer.orders[0][0];
                assert!(
                    first == egs.work_buffer.lists[0][whole as u32 as usize].play,
                    "[{name}] order_first's first is order's first"
                );
                firsts += 1;
            }
        }
        println!("order_first: {firsts} lists, {tied} tied at the top");
        assert!(firsts > 0);
    }

    #[test]
    #[inline]
    fn the_order_ignores_generation_order() {
        // the list reversed, or rotated, sorts the same: by valuation, ties by
        // play, each index its place; a table move goes first and the rest keep
        // their order
        let gc = game_config::make_english_game_config();
        let kwg_bytes = tiny_kwg_bytes();
        let kwg = kwg::Kwg::<kwg::Node22>::from_bytes_alloc(&kwg_bytes).unwrap();
        let arm_tables = anagrams::ArmTables::build(&kwg, &gc, false).unwrap();
        let mut egs =
            EndgameSolver::<kwg::Node22, kwg::Node22>::new(&gc, &kwg, &arm_tables).unwrap();
        let mut ties = 0usize;
        for (name, pos) in all_positions() {
            for mover in 0..2u8 {
                egs.init(&pos.board, [&pos.racks[0][..], &pos.racks[1][..]]);
                egs.reset_position();
                egs.generate(0, mover);
                order_ply(&mut egs, 0, super::NO_PLAY);
                let list = std::mem::take(&mut egs.work_buffer.lists[0]);
                let ordered = std::mem::take(&mut egs.work_buffer.orders[0]);
                let moves = ordered
                    .iter()
                    .map(|&key| &list[key as u32 as usize])
                    .collect::<Vec<_>>();
                for (i, candidate) in moves.iter().enumerate() {
                    assert_eq!(
                        candidate.index as usize, i,
                        "[{name}] an index is its place"
                    );
                }
                for pair in moves.windows(2) {
                    let by_valuation = pair[1].valuation.total_cmp(&pair[0].valuation);
                    if by_valuation.is_eq() {
                        ties += 1;
                        assert!(
                            movegen::cmp_play(&pair[0].play, &pair[1].play).is_lt(),
                            "[{name}] ties in play order"
                        );
                    } else {
                        assert!(by_valuation.is_lt(), "[{name}] the best valuation first");
                    }
                }
                for turn in [0, list.len() / 3] {
                    let mut again = list
                        .iter()
                        .rev()
                        .map(|candidate| super::Candidate {
                            play: candidate.play.clone(),
                            index: super::NO_PLAY,
                            valuation: candidate.valuation,
                            plays_out: candidate.plays_out,
                        })
                        .collect::<Vec<_>>();
                    again.rotate_left(turn);
                    let mut keys = again
                        .iter()
                        .enumerate()
                        .map(|(found, candidate)| {
                            let movegen::Play::Place {
                                down, lane, idx, ..
                            } = candidate.play
                            else {
                                unreachable!("a candidate places tiles");
                            };
                            super::order_key(candidate.valuation as i32, down, lane, idx, found)
                        })
                        .collect::<Vec<_>>();
                    super::order(&mut again, &mut keys, 0, super::NO_PLAY);
                    assert!(
                        keys.iter()
                            .map(|&key| &again[key as u32 as usize])
                            .zip(&moves)
                            .all(|(a, b)| a.play == b.play && a.index == b.index),
                        "[{name}] the same order however generated"
                    );
                }
                if moves.len() >= 2 {
                    let table_play = (moves.len() / 2) as u16;
                    egs.generate(0, mover);
                    order_ply(&mut egs, 0, table_play);
                    let again = &egs.work_buffer.lists[0];
                    let first = egs.work_buffer.orders[0]
                        .iter()
                        .map(|&key| &again[key as u32 as usize])
                        .collect::<Vec<_>>();
                    assert!(
                        first[0].play == moves[table_play as usize].play,
                        "[{name}] the table's move first"
                    );
                    assert!(
                        first[1..].iter().map(|c| &c.play).eq(moves
                            .iter()
                            .filter(|c| c.index != table_play)
                            .map(|c| &c.play)),
                        "[{name}] the rest in order"
                    );
                }
            }
        }
        assert!(ties > 0, "no position had a tie");
    }

    #[test]
    #[inline]
    fn differential_reference_vs_solve() {
        let gc = game_config::make_english_game_config();
        let kwg_bytes = tiny_kwg_bytes();
        let kwg = kwg::Kwg::<kwg::Node22>::from_bytes_alloc(&kwg_bytes).unwrap();
        let klv = klv::Klv::<kwg::Node22>::from_bytes_alloc(klv::EMPTY_KLV_BYTES).unwrap();
        let mut mg = movegen::KurniaMoveGenerator::new(&gc);
        let arms = both_arms(&kwg, &gc);

        let mut disagreements = 0usize;
        let mut total = 0usize;
        // from no scoreless turns, and from some, up to one short of the end
        for (name, pos) in all_positions() {
            for (passes, zeros) in [(0, 0), (1, 1), (5, 5)] {
                let refv = reference(
                    &mut Reference {
                        gc: &gc,
                        kwg: &kwg,
                        klv: &klv,
                        mg: &mut mg,
                        memo: Default::default(),
                    },
                    &pos.board,
                    &pos.racks,
                    0,
                    passes,
                    zeros,
                );
                for (lean, arm_tables) in &arms {
                    total += 1;
                    let mut egs =
                        EndgameSolver::<kwg::Node22, kwg::Node22>::new(&gc, &kwg, arm_tables)
                            .unwrap();
                    egs.init_after_scoreless_turns(
                        &pos.board,
                        [&pos.racks[0][..], &pos.racks[1][..]],
                        passes as u16,
                        zeros as u16,
                    );
                    let solvev = egs.solve(0).unwrap();
                    if solvev != super::to_equity(refv) {
                        disagreements += 1;
                        let ratio = if refv != 0.0 {
                            solvev.raw() as f32 / refv
                        } else {
                            f32::NAN
                        };
                        println!(
                            "DISAGREE [{name}] lean={lean} scoreless={passes},{zeros}: ref={refv} solve={} ratio={ratio}\n   {}",
                            solvev.raw(),
                            describe(&gc, &pos)
                        );
                    }
                }
            }
        }
        println!("differential: {disagreements} disagreements out of {total} solves");
        assert_eq!(disagreements, 0, "solve() disagreed with the reference");
    }

    #[test]
    #[inline]
    fn a_solve_a_limit_stops_answers_from_a_finished_depth() {
        let gc = game_config::make_english_game_config();
        let kwg_bytes = tiny_kwg_bytes();
        let kwg = kwg::Kwg::<kwg::Node22>::from_bytes_alloc(&kwg_bytes).unwrap();
        let arm_tables = anagrams::ArmTables::build(&kwg, &gc, false).unwrap();

        let mut cut = 0usize;
        let mut crowded = 0usize;
        for (name, pos) in all_positions() {
            // every depth an unlimited solve finishes: its value and the
            // positions its table holds
            let finished = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
            let mut egs =
                EndgameSolver::<kwg::Node22, kwg::Node22>::new(&gc, &kwg, &arm_tables).unwrap();
            egs.init(&pos.board, [&pos.racks[0][..], &pos.racks[1][..]]);
            let seen = finished.clone();
            egs.report_progress(move |p: &super::Progress| {
                if let Some(value) = p.value {
                    seen.borrow_mut().push((p.depth, value, p.held));
                }
                Ok(())
            });
            let full = egs.solve_within(0, &super::Limits::NONE).unwrap();
            let finished = finished.borrow();
            assert!(full.exact, "[{name}] an unlimited solve is exact");
            assert_eq!(
                finished.last().map(|&(d, v, _)| (d, v)),
                Some((full.depth, full.value)),
                "[{name}] the last finished depth is the answer"
            );
            if finished.len() < 2 {
                continue;
            }

            // no time at all: the first depth always finishes, and is the answer
            let mut egs =
                EndgameSolver::<kwg::Node22, kwg::Node22>::new(&gc, &kwg, &arm_tables).unwrap();
            egs.init(&pos.board, [&pos.racks[0][..], &pos.racks[1][..]]);
            let limits = super::Limits {
                time: Some(std::time::Duration::ZERO),
                states: None,
                memory: None,
            };
            let solved = egs.solve_within(0, &limits).unwrap();
            assert!(
                !solved.exact,
                "[{name}] a zero time limit cuts the solve short"
            );
            assert_eq!(
                (solved.depth, solved.value),
                (finished[0].0, finished[0].1),
                "[{name}] a zero time limit answers the first depth"
            );

            // the memory past its limit from the start: the same
            let mut egs =
                EndgameSolver::<kwg::Node22, kwg::Node22>::new(&gc, &kwg, &arm_tables).unwrap();
            egs.init(&pos.board, [&pos.racks[0][..], &pos.racks[1][..]]);
            let limits = super::Limits {
                time: None,
                states: None,
                memory: Some(super::MemoryLimit {
                    bytes: 1,
                    held: || usize::MAX,
                }),
            };
            let solved = egs.solve_within(0, &limits).unwrap();
            assert!(
                !solved.exact,
                "[{name}] a memory limit passed cuts the solve short"
            );
            assert_eq!(
                (solved.depth, solved.value),
                (finished[0].0, finished[0].1),
                "[{name}] a memory limit passed answers the first depth"
            );

            // a memory limit never reached changes nothing
            let mut egs =
                EndgameSolver::<kwg::Node22, kwg::Node22>::new(&gc, &kwg, &arm_tables).unwrap();
            egs.init(&pos.board, [&pos.racks[0][..], &pos.racks[1][..]]);
            let limits = super::Limits {
                time: None,
                states: None,
                memory: Some(super::MemoryLimit {
                    bytes: usize::MAX,
                    held: || 0,
                }),
            };
            let solved = egs.solve_within(0, &limits).unwrap();
            assert!(
                solved.exact && (solved.depth, solved.value) == (full.depth, full.value),
                "[{name}] a memory limit never reached answers as an unlimited solve"
            );

            // with nothing held and one byte allowed, the table cannot grow
            // past its first bucket, so its positions replace each other, and
            // the solve still runs to the end
            let mut egs =
                EndgameSolver::<kwg::Node22, kwg::Node22>::new(&gc, &kwg, &arm_tables).unwrap();
            egs.init(&pos.board, [&pos.racks[0][..], &pos.racks[1][..]]);
            let limits = super::Limits {
                time: None,
                states: None,
                memory: Some(super::MemoryLimit {
                    bytes: 1,
                    held: || 0,
                }),
            };
            let solved = egs.solve_within(0, &limits).unwrap();
            assert!(
                solved.exact && solved.value == full.value,
                "[{name}] a table that may not grow answers as an unlimited solve"
            );
            assert_eq!(egs.work_buffer.table.room(), 4);
            crowded += (finished.last().unwrap().2 > 4) as usize;

            // one state past the first depth's: the search stops within a later
            // pass, or after it, and answers a depth that finished
            let mut egs =
                EndgameSolver::<kwg::Node22, kwg::Node22>::new(&gc, &kwg, &arm_tables).unwrap();
            egs.init(&pos.board, [&pos.racks[0][..], &pos.racks[1][..]]);
            let limits = super::Limits {
                time: None,
                states: Some(finished[0].2 + 1),
                memory: None,
            };
            let solved = egs.solve_within(0, &limits).unwrap();
            if !solved.exact {
                cut += 1;
                assert!(
                    finished
                        .iter()
                        .any(|&(d, v, _)| (d, v) == (solved.depth, solved.value)),
                    "[{name}] a state limit answers depth {} with {}, which no pass finished",
                    solved.depth,
                    solved.value
                );
            } else {
                assert_eq!(
                    (solved.depth, solved.value),
                    (full.depth, full.value),
                    "[{name}] a solve the state limit never stopped is the unlimited one"
                );
            }
        }
        assert!(cut > 0, "no state limit cut a solve short");
        assert!(crowded > 0, "no solve needed more than a bucket");
    }

    #[test]
    #[inline]
    fn a_search_with_a_tile_in_the_bag_keeps_one_listener() {
        let gc = game_config::make_english_game_config();
        let kwg_bytes = tiny_kwg_bytes();
        let kwg = kwg::Kwg::<kwg::Node22>::from_bytes_alloc(&kwg_bytes).unwrap();
        let arm_tables = anagrams::ArmTables::build(&kwg, &gc, false).unwrap();

        // every endgame of the search tells the one listener it was given, so
        // its clock is the search's: at every node, with the endgames before
        // it, and never a finished depth of its own
        let heard = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
        let seen = heard.clone();
        let limits = super::BagLimits::new(
            &super::Limits::NONE,
            Some(Box::new(move |p: &super::Progress| {
                seen.borrow_mut().push((p.endgames, p.value));
                Ok(())
            })),
        );
        let positions = all_positions();
        let mut egs =
            EndgameSolver::<kwg::Node22, kwg::Node22>::new(&gc, &kwg, &arm_tables).unwrap();
        let mut exact = true;
        for (_, pos) in &positions {
            egs.init(&pos.board, [&pos.racks[0][..], &pos.racks[1][..]]);
            egs.solve_in_bag(0, &limits, &mut exact).unwrap();
        }
        assert!(exact, "a search with no limits is exact");
        assert_eq!(limits.endgames.get(), positions.len() as u64);
        assert!(limits.progress.take().is_some(), "the listener comes back");
        let heard = heard.borrow();
        assert!(heard.iter().all(|&(_, value)| value.is_none()));
        let endgames = heard.iter().map(|&(n, _)| n).collect::<Vec<_>>();
        assert!(endgames.windows(2).all(|w| w[0] <= w[1]));
        for n in 0..positions.len() as u64 {
            assert!(endgames.contains(&Some(n)), "endgame {n} told nothing");
        }
    }

    #[test]
    #[inline]
    fn a_solve_times_its_movegens_only_while_something_listens() {
        let gc = game_config::make_english_game_config();
        let kwg_bytes = tiny_kwg_bytes();
        let kwg = kwg::Kwg::<kwg::Node22>::from_bytes_alloc(&kwg_bytes).unwrap();
        let arm_tables = anagrams::ArmTables::build(&kwg, &gc, false).unwrap();
        let positions = all_positions();
        let mut egs =
            EndgameSolver::<kwg::Node22, kwg::Node22>::new(&gc, &kwg, &arm_tables).unwrap();
        let mut movegen = std::time::Duration::ZERO;
        for (_, pos) in &positions {
            egs.init(&pos.board, [&pos.racks[0][..], &pos.racks[1][..]]);
            egs.solve(0).unwrap();
            movegen += egs.work_buffer.dur_movegen;
        }
        assert_eq!(movegen, std::time::Duration::ZERO, "no listener, no clock");
        egs.report_progress(|_: &super::Progress| Ok(()));
        for (_, pos) in &positions {
            egs.init(&pos.board, [&pos.racks[0][..], &pos.racks[1][..]]);
            egs.solve(0).unwrap();
            movegen += egs.work_buffer.dur_movegen;
        }
        assert!(movegen > std::time::Duration::ZERO, "a listener, the time");
    }

    #[test]
    #[inline]
    fn a_solve_fits_a_small_stack() {
        // a solve recurses a frame a ply, never a frame a move, so it runs on a
        // thread with far less stack than a main thread has (a test's own
        // thread has more, so this one asks for a small one)
        let gc = game_config::make_english_game_config();
        let kwg_bytes = tiny_kwg_bytes();
        let kwg = kwg::Kwg::<kwg::Node22>::from_bytes_alloc(&kwg_bytes).unwrap();
        let arm_tables = anagrams::ArmTables::build(&kwg, &gc, false).unwrap();
        let positions = all_positions();
        let solved = std::thread::scope(|s| {
            std::thread::Builder::new()
                .stack_size(256 << 10)
                .spawn_scoped(s, || {
                    let mut egs =
                        EndgameSolver::<kwg::Node22, kwg::Node22>::new(&gc, &kwg, &arm_tables)
                            .unwrap();
                    let mut solved = 0usize;
                    for (_, pos) in &positions {
                        egs.init(&pos.board, [&pos.racks[0][..], &pos.racks[1][..]]);
                        egs.solve(0).unwrap();
                        solved += 1;
                    }
                    // and a deep one: full racks, as many plies as they hold
                    let mut board = empty_board();
                    put_word(&mut board, 7, 6, &[1, 20]);
                    egs.init(&board, [&[1, 1, 2, 8, 20, 20, 1], &[1, 2, 1, 20, 8, 1, 20]]);
                    egs.solve_within(
                        0,
                        &super::Limits {
                            time: Some(std::time::Duration::from_secs(2)),
                            states: None,
                            memory: None,
                        },
                    )
                    .unwrap();
                    solved + 1
                })
                .unwrap()
                .join()
                .unwrap()
        });
        assert_eq!(solved, positions.len() + 1);
    }

    #[test]
    #[inline]
    fn a_key_made_move_by_move_is_the_key_made_afresh() {
        let gc = game_config::make_english_game_config();
        let kwg_bytes = tiny_kwg_bytes();
        let kwg = kwg::Kwg::<kwg::Node22>::from_bytes_alloc(&kwg_bytes).unwrap();
        let arm_tables = anagrams::ArmTables::build(&kwg, &gc, false).unwrap();
        let mut egs =
            EndgameSolver::<kwg::Node22, kwg::Node22>::new(&gc, &kwg, &arm_tables).unwrap();
        let mut afresh =
            EndgameSolver::<kwg::Node22, kwg::Node22>::new(&gc, &kwg, &arm_tables).unwrap();
        let mut moves = 0usize;
        for (name, pos) in all_positions() {
            for mover in 0..2u8 {
                egs.init(&pos.board, [&pos.racks[0][..], &pos.racks[1][..]]);
                egs.reset_position();
                let before = egs.work_buffer.key;
                egs.generate(0, mover);
                let list = std::mem::take(&mut egs.work_buffer.lists[0]);
                for candidate in &list {
                    let movegen::Play::Place {
                        down,
                        lane,
                        idx,
                        word,
                        ..
                    } = &candidate.play
                    else {
                        unreachable!("a candidate places tiles");
                    };
                    egs.place(mover, &candidate.play);
                    let mut board = pos.board.clone();
                    let mut racks = pos.racks.clone();
                    apply_place(
                        &gc,
                        &mut board,
                        &mut racks[mover as usize],
                        *down,
                        *lane,
                        *idx,
                        word,
                    );
                    afresh.init(&board, [&racks[0][..], &racks[1][..]]);
                    afresh.reset_position();
                    assert_eq!(
                        egs.work_buffer.key, afresh.work_buffer.key,
                        "[{name}] the key after a move"
                    );
                    assert_eq!(egs.work_buffer.racks, afresh.work_buffer.racks);
                    assert_eq!(egs.work_buffer.rack_scores, afresh.work_buffer.rack_scores);
                    egs.unplace(mover, &candidate.play);
                    assert_eq!(
                        egs.work_buffer.key, before,
                        "[{name}] the key after the move is taken back"
                    );
                    assert_eq!(egs.work_buffer.board_tiles, pos.board);
                    moves += 1;
                }
            }
        }
        assert!(moves > 0, "no move was made");
    }

    #[test]
    #[inline]
    fn a_rack_of_seven_of_one_tile_or_six_and_a_blank_is_keyed_like_any_other() {
        // a rack's key is made of a key for each count of each tile it holds,
        // so a rack holding every copy it can of one tile is keyed as any other
        // is: made move by move it is the key made afresh, and racks that
        // differ in one tile key apart
        let gc = game_config::make_english_game_config();
        let kwg_bytes = tiny_kwg_bytes();
        let kwg = kwg::Kwg::<kwg::Node22>::from_bytes_alloc(&kwg_bytes).unwrap();
        let arm_tables = anagrams::ArmTables::build(&kwg, &gc, false).unwrap();
        let mut egs =
            EndgameSolver::<kwg::Node22, kwg::Node22>::new(&gc, &kwg, &arm_tables).unwrap();
        let mut afresh =
            EndgameSolver::<kwg::Node22, kwg::Node22>::new(&gc, &kwg, &arm_tables).unwrap();
        let mut board = empty_board();
        put_word(&mut board, 7, 6, &[1, 20]);
        let seven_e = vec![5u8; 7];
        let six_e_blank = vec![0u8, 5, 5, 5, 5, 5, 5];
        let six_e = vec![5u8; 6];
        let other = vec![1u8, 20];
        let mut keys = Vec::new();
        let mut moves = 0usize;
        for racks in [
            [seven_e.clone(), other.clone()],
            [six_e_blank.clone(), other.clone()],
            [six_e.clone(), other.clone()],
            [other.clone(), seven_e.clone()],
            [other.clone(), six_e_blank.clone()],
        ] {
            egs.init(&board, [&racks[0][..], &racks[1][..]]);
            egs.reset_position();
            keys.push(egs.work_buffer.key);
            for mover in 0..2u8 {
                egs.init(&board, [&racks[0][..], &racks[1][..]]);
                egs.reset_position();
                let before = egs.work_buffer.key;
                egs.generate(0, mover);
                let list = std::mem::take(&mut egs.work_buffer.lists[0]);
                for candidate in &list {
                    let movegen::Play::Place {
                        down,
                        lane,
                        idx,
                        word,
                        ..
                    } = &candidate.play
                    else {
                        unreachable!("a candidate places tiles");
                    };
                    egs.place(mover, &candidate.play);
                    let mut board_after = board.clone();
                    let mut racks_after = racks.clone();
                    apply_place(
                        &gc,
                        &mut board_after,
                        &mut racks_after[mover as usize],
                        *down,
                        *lane,
                        *idx,
                        word,
                    );
                    afresh.init(&board_after, [&racks_after[0][..], &racks_after[1][..]]);
                    afresh.reset_position();
                    assert_eq!(egs.work_buffer.key, afresh.work_buffer.key);
                    assert_eq!(egs.work_buffer.racks, afresh.work_buffer.racks);
                    egs.unplace(mover, &candidate.play);
                    assert_eq!(egs.work_buffer.key, before);
                    moves += 1;
                }
            }
        }
        assert!(moves > 0, "no move was made");
        for (i, a) in keys.iter().enumerate() {
            for b in &keys[i + 1..] {
                assert_ne!(a, b, "two positions share a key");
            }
        }
    }

    #[test]
    #[inline]
    fn the_table_keeps_what_it_is_given_and_the_deeper_of_two() {
        let entry = |value: f32, depth: i8, bound: super::Bound| super::Entry {
            value,
            play: 7,
            depth,
            bound,
        };
        let mut table = super::Table::new();
        table.generation = 1;
        assert!(table.probe(1).is_none(), "an empty table knows nothing");
        assert!(table.store(1, entry(-2.5, 3, super::Bound::Lower)));
        let found = table.probe(1).unwrap();
        assert_eq!(
            (found.value, found.play, found.depth),
            (-2.5, 7, 3),
            "an entry comes back as it went in"
        );
        assert!(found.bound == super::Bound::Lower);
        // a shallower bound leaves the deeper value; an exact one takes its
        // place
        assert!(!table.store(1, entry(9.0, 2, super::Bound::Upper)));
        assert_eq!(table.probe(1).unwrap().value, -2.5);
        assert!(!table.store(1, entry(4.0, 1, super::Bound::Exact)));
        assert_eq!(table.probe(1).unwrap().value, 4.0);
        assert_eq!(table.held, 1);
        // a full bucket gives up its shallowest entry
        for key in 2..=4 {
            assert!(table.store(key, entry(0.0, 5, super::Bound::Exact)));
        }
        assert!(!table.store(5, entry(0.0, 5, super::Bound::Exact)));
        assert!(table.probe(1).is_none(), "the shallowest entry made room");
        assert!((2..=5).all(|key| table.probe(key).is_some()));
        // a table twice as big keeps every entry
        table.resize(2);
        assert_eq!(table.held, 4);
        assert!((2..=5).all(|key| table.probe(key).is_some()));
    }

    #[test]
    #[inline]
    fn peg_aggregate_weights_each_outcome_by_how_many_tiles_give_it() {
        let e = equity::Equity::new;
        let (win_pct, expected) = super::peg_aggregate(&[(2, e(3000)), (1, e(0)), (1, e(-5000))]);
        assert_eq!(win_pct, 0.625);
        assert_eq!(expected, e(250));

        let (win_pct, expected) = super::peg_aggregate(&[(1, e(1)), (1, e(0)), (1, e(-1))]);
        assert_eq!(win_pct, 0.5);
        assert_eq!(expected, e(0));

        assert_eq!(super::peg_aggregate(&[]), (0.0, e(0)));
    }

    #[test]
    #[inline]
    fn a_solver_for_three_players_is_an_error() {
        let gc = game_config::Options {
            preset: "english",
            tiles: None,
            board: None,
            rack_size: None,
            jumbled: false,
            players: Some(3),
            bingo_bonus: None,
            zeros_to_end: None,
            passes_to_end: None,
            exchange_limit: None,
            exchanges: None,
        }
        .make_game_config(&|_| Err("no files here".into()))
        .unwrap();
        let kwg_bytes = tiny_kwg_bytes();
        let kwg = kwg::Kwg::<kwg::Node22>::from_bytes_alloc(&kwg_bytes).unwrap();
        let arm_tables = anagrams::ArmTables::build(&kwg, &gc, false).unwrap();
        assert!(EndgameSolver::<kwg::Node22, kwg::Node22>::new(&gc, &kwg, &arm_tables).is_err());
    }

    #[test]
    #[inline]
    fn peg_one_in_bag_hand_checkable() {
        let gc = game_config::make_english_game_config();
        let kwg_bytes = tiny_kwg_bytes();
        let kwg = kwg::Kwg::<kwg::Node22>::from_bytes_alloc(&kwg_bytes).unwrap();

        let board = empty_board();
        let mover_rack = [2u8]; // B
        let mut unseen_tally = vec![0u8; gc.alphabet().len() as usize];
        unseen_tally[8] = 1; // H
        unseen_tally[20] = 1; // T

        let arm_tables = anagrams::ArmTables::build(&kwg, &gc, false).unwrap();
        let mut egs =
            EndgameSolver::<kwg::Node22, kwg::Node22>::new(&gc, &kwg, &arm_tables).unwrap();
        let result = egs
            .solve_peg_one_in_bag(
                0,
                &board,
                &mover_rack,
                &unseen_tally,
                equity::Equity::ZERO,
                &super::Limits::NONE,
            )
            .unwrap()
            .expect("english one-in-bag is always solvable");

        let e = equity::Equity::new;
        assert_eq!(result.hypotheses, vec![(8, 1, e(-2000)), (20, 1, e(1000))]);
        assert_eq!(result.win_pct, 0.5);
        assert_eq!(result.expected_margin, e(-500));
    }

    #[test]
    #[inline]
    fn peg_one_in_bag_score_diff_shifts_the_outcome() {
        let gc = game_config::make_english_game_config();
        let kwg_bytes = tiny_kwg_bytes();
        let kwg = kwg::Kwg::<kwg::Node22>::from_bytes_alloc(&kwg_bytes).unwrap();

        let board = empty_board();
        let mover_rack = [2u8]; // B
        let mut unseen_tally = vec![0u8; gc.alphabet().len() as usize];
        unseen_tally[8] = 1; // H
        unseen_tally[20] = 1; // T

        let arm_tables = anagrams::ArmTables::build(&kwg, &gc, false).unwrap();
        let mut egs =
            EndgameSolver::<kwg::Node22, kwg::Node22>::new(&gc, &kwg, &arm_tables).unwrap();
        let e = equity::Equity::new;
        let result = egs
            .solve_peg_one_in_bag(
                0,
                &board,
                &mover_rack,
                &unseen_tally,
                e(3000),
                &super::Limits::NONE,
            )
            .unwrap()
            .expect("english one-in-bag is always solvable");

        assert_eq!(result.hypotheses, vec![(8, 1, e(1000)), (20, 1, e(4000))]);
        assert_eq!(result.win_pct, 1.0);
        assert_eq!(result.expected_margin, e(2500));
    }

    #[test]
    #[inline]
    fn a_peg_solve_says_when_a_limit_cut_an_endgame_inside_it() {
        let kwg_bytes = tiny_kwg_bytes();
        let kwg = kwg::Kwg::<kwg::Node22>::from_bytes_alloc(&kwg_bytes).unwrap();
        let no_time = super::Limits {
            time: Some(std::time::Duration::ZERO),
            states: None,
            memory: None,
        };
        let one_state = super::Limits {
            time: None,
            states: Some(1),
            memory: None,
        };
        let no_memory = super::Limits {
            time: None,
            states: None,
            memory: Some(super::MemoryLimit {
                bytes: 1,
                held: || usize::MAX,
            }),
        };
        let tiles = [1u8, 2, 8, 20]; // A B H T
        let mut board = empty_board();
        put_word(&mut board, 7, 6, &[1, 20]);
        let mut cut = [0usize; 3];
        // the committed model, and the clairvoyant one where exchanges are legal
        for gc in [
            game_config::make_english_game_config(),
            game_config::make_exchange_test_game_config(),
        ] {
            let arm_tables = anagrams::ArmTables::build(&kwg, &gc, false).unwrap();
            let mut egs =
                EndgameSolver::<kwg::Node22, kwg::Node22>::new(&gc, &kwg, &arm_tables).unwrap();
            let mut unseen_tally = vec![0u8; gc.alphabet().len() as usize];
            unseen_tally[8] = 1; // H
            unseen_tally[20] = 1; // T
            for &x in &tiles {
                for &y in &tiles {
                    let mover_rack = [x, y];
                    let mut solve = |limits: &super::Limits| {
                        egs.solve_peg_one_in_bag(
                            0,
                            &board,
                            &mover_rack,
                            &unseen_tally,
                            equity::Equity::ZERO,
                            limits,
                        )
                        .unwrap()
                        .expect("both configs solve one in the bag")
                    };
                    let full = solve(&super::Limits::NONE);
                    assert!(full.exact, "an unlimited solve is exact");
                    for (i, limits) in [&no_time, &one_state, &no_memory].into_iter().enumerate() {
                        let limited = solve(limits);
                        assert_eq!(limited.hypotheses.len(), full.hypotheses.len());
                        if limited.exact {
                            assert_eq!(limited.hypotheses, full.hypotheses);
                        } else {
                            cut[i] += 1;
                        }
                    }
                }
            }
        }
        assert!(
            cut.iter().all(|&n| n > 0),
            "the time, state, and memory limits cut {cut:?} endgames short"
        );
    }

    #[test]
    #[inline]
    fn the_endgames_of_a_search_with_a_tile_in_the_bag_share_the_solvers_table() {
        let kwg_bytes = tiny_kwg_bytes();
        let kwg = kwg::Kwg::<kwg::Node22>::from_bytes_alloc(&kwg_bytes).unwrap();
        let mut board = empty_board();
        put_word(&mut board, 7, 6, &[1, 20]);
        // the committed model, and the clairvoyant one where exchanges are
        // legal
        for gc in [
            game_config::make_english_game_config(),
            game_config::make_exchange_test_game_config(),
        ] {
            let arm_tables = anagrams::ArmTables::build(&kwg, &gc, false).unwrap();
            let mut egs =
                EndgameSolver::<kwg::Node22, kwg::Node22>::new(&gc, &kwg, &arm_tables).unwrap();
            let nodes = std::rc::Rc::new(std::cell::Cell::new(0u64));
            let heard = nodes.clone();
            egs.report_progress(move |p: &super::Progress| {
                if p.value.is_none() {
                    heard.set(heard.get() + 1);
                }
                Ok(())
            });
            let mut unseen_tally = vec![0u8; gc.alphabet().len() as usize];
            unseen_tally[8] = 1; // H
            unseen_tally[20] = 1; // T
            let solve = |egs: &mut EndgameSolver<kwg::Node22, kwg::Node22>| {
                nodes.set(0);
                let result = egs
                    .solve_peg_one_in_bag(
                        0,
                        &board,
                        &[1, 2], // A B
                        &unseen_tally,
                        equity::Equity::ZERO,
                        &super::Limits::NONE,
                    )
                    .unwrap()
                    .expect("both configs solve one in the bag");
                (result.hypotheses, nodes.get())
            };
            let (first, first_nodes) = solve(&mut egs);
            assert!(
                egs.work_buffer.table.held > 0,
                "the solver's table holds what its endgames found"
            );
            // the same search again finds its endgames in the table
            let (second, second_nodes) = solve(&mut egs);
            assert_eq!(first, second);
            assert!(
                second_nodes < first_nodes,
                "the second search took {second_nodes} nodes, the first {first_nodes}"
            );
        }
    }

    #[test]
    #[inline]
    fn a_principal_variation_plays_out_to_the_value_it_claims() {
        let gc = game_config::make_english_game_config();
        let kwg_bytes = tiny_kwg_bytes();
        let kwg = kwg::Kwg::<kwg::Node22>::from_bytes_alloc(&kwg_bytes).unwrap();

        let mut ok = 0usize;
        let mut bad = 0usize;
        let mut pv: Vec<(equity::Equity, movegen::Play)> = Vec::new();
        let positions = all_positions();
        for (lean, arm_tables) in &both_arms(&kwg, &gc) {
            for (name, pos) in &positions {
                let mut egs =
                    EndgameSolver::<kwg::Node22, kwg::Node22>::new(&gc, &kwg, arm_tables).unwrap();
                egs.init(&pos.board, [&pos.racks[0][..], &pos.racks[1][..]]);
                let v = egs.solve(0).unwrap();
                egs.collect_pv(0, &mut pv);

                let a = gc.alphabet();
                let mut board = pos.board.clone();
                let mut racks = pos.racks.clone();
                let mut mover = 0usize;
                let mut sign = 1.0f32;
                let mut realized = 0.0f32;
                let mut prev_pass = false;
                let mut ended = false;
                for (_eq, play) in &pv {
                    match play {
                        movegen::Play::Place {
                            down,
                            lane,
                            idx,
                            word,
                            score,
                        } => {
                            realized += sign * (*score as f32);
                            apply_place(
                                &gc,
                                &mut board,
                                &mut racks[mover],
                                *down,
                                *lane,
                                *idx,
                                word,
                            );
                            if racks[mover].is_empty() {
                                realized +=
                                    sign * (2 * a.scaled_rack_score(&racks[mover ^ 1])) as f32;
                                ended = true;
                            }
                            prev_pass = false;
                        }
                        movegen::Play::Exchange { .. } => {
                            if prev_pass {
                                realized += sign
                                    * (a.scaled_rack_score(&racks[mover ^ 1])
                                        - a.scaled_rack_score(&racks[mover]))
                                        as f32;
                                ended = true;
                            }
                            prev_pass = true;
                        }
                    }
                    mover ^= 1;
                    sign = -sign;
                    if ended {
                        break;
                    }
                }
                if super::to_equity(realized) == v {
                    ok += 1;
                } else {
                    bad += 1;
                    println!(
                        "PV-REPLAY MISMATCH [{name}] lean={lean}: solve={} realized={realized} pvlen={}",
                        v.raw(),
                        pv.len()
                    );
                }
            }
        }
        println!("pv-playout: {ok} ok, {bad} mismatches");
        assert_eq!(bad, 0, "PV replay did not reproduce solve()'s value");
    }

    #[test]
    #[inline]
    fn every_principal_variation_is_legal_and_its_value_bounded() {
        let gc = game_config::make_english_game_config();
        let kwg_bytes = tiny_kwg_bytes();
        let kwg = kwg::Kwg::<kwg::Node22>::from_bytes_alloc(&kwg_bytes).unwrap();
        let klv = klv::Klv::<kwg::Node22>::from_bytes_alloc(klv::EMPTY_KLV_BYTES).unwrap();
        let mut mg = movegen::KurniaMoveGenerator::new(&gc);

        let mut illegal = 0usize;
        let mut oob = 0usize;
        let mut pv: Vec<(equity::Equity, movegen::Play)> = Vec::new();
        let positions = all_positions();
        for (lean, arm_tables) in &both_arms(&kwg, &gc) {
            for (name, pos) in &positions {
                let mut egs =
                    EndgameSolver::<kwg::Node22, kwg::Node22>::new(&gc, &kwg, arm_tables).unwrap();
                egs.init(&pos.board, [&pos.racks[0][..], &pos.racks[1][..]]);
                let v = egs.solve(0).unwrap();
                egs.collect_pv(0, &mut pv);

                let a = gc.alphabet();
                let board_pts: i32 = pos.board.iter().map(|&t| a.scaled_score(t)).sum();
                let rack_pts =
                    a.scaled_rack_score(&pos.racks[0]) + a.scaled_rack_score(&pos.racks[1]);
                let cap = 4 * (board_pts + rack_pts) + 10000;
                if v.raw().abs() > cap {
                    oob += 1;
                    println!("VALUE OOB [{name}] lean={lean}: v={} cap={cap}", v.raw());
                }

                let mut board = pos.board.clone();
                let mut racks = pos.racks.clone();
                let mut mover = 0usize;
                let mut prev_pass = false;
                let mut ended = false;
                for (_eq, play) in &pv {
                    match play {
                        movegen::Play::Place {
                            down,
                            lane,
                            idx,
                            word,
                            ..
                        } => {
                            let snapshot = movegen::BoardSnapshot {
                                board_tiles: &board,
                                game_config: &gc,
                                kwg: &kwg,
                                anagrams: None,
                                rack_lengths: None,
                                klv: &klv,
                            };
                            mg.gen_moves_raw_all_unsorted(
                                &snapshot,
                                &racks[mover],
                                0,
                                movegen::PassPolicy::AsACandidate,
                            );
                            let found = mg.plays.iter().any(|vm| {
                                if let movegen::Play::Place {
                                    down: d2,
                                    lane: l2,
                                    idx: i2,
                                    word: w2,
                                    ..
                                } = &vm.play
                                {
                                    d2 == down && l2 == lane && i2 == idx && w2[..] == word[..]
                                } else {
                                    false
                                }
                            });
                            if !found {
                                illegal += 1;
                                println!("ILLEGAL PV PLAY [{name}] lean={lean} mover={mover}");
                            }
                            apply_place(
                                &gc,
                                &mut board,
                                &mut racks[mover],
                                *down,
                                *lane,
                                *idx,
                                word,
                            );
                            if racks[mover].is_empty() {
                                ended = true;
                            }
                            prev_pass = false;
                        }
                        movegen::Play::Exchange { .. } => {
                            if prev_pass {
                                ended = true;
                            }
                            prev_pass = true;
                        }
                    }
                    mover ^= 1;
                    if ended {
                        break;
                    }
                }
            }
        }
        println!("properties: {illegal} illegal PV plays, {oob} out-of-bounds values");
        assert_eq!(illegal, 0, "a PV play was not legal");
        assert_eq!(oob, 0, "a solve() value exceeded the scaled bound");
    }

    #[test]
    #[inline]
    fn differential_one_in_bag() {
        let gc = game_config::make_english_game_config();
        let kwg_bytes = tiny_kwg_bytes();
        let kwg = kwg::Kwg::<kwg::Node22>::from_bytes_alloc(&kwg_bytes).unwrap();
        let klv = klv::Klv::<kwg::Node22>::from_bytes_alloc(klv::EMPTY_KLV_BYTES).unwrap();
        let mut mg = movegen::KurniaMoveGenerator::new(&gc);

        let bag_tiles: [u8; 4] = [1, 8, 20, 0];
        let arms = both_arms(&kwg, &gc);

        let mut disagreements = 0usize;
        let mut total = 0usize;

        let mut positions = embedded_positions();
        positions.extend(random_positions(12));
        for (name, pos) in positions {
            for &t in &bag_tiles {
                for mover in 0u8..2 {
                    let refv = reference_one_in_bag(
                        &mut Reference {
                            gc: &gc,
                            kwg: &kwg,
                            klv: &klv,
                            mg: &mut mg,
                            memo: Default::default(),
                        },
                        &pos.board,
                        &pos.racks,
                        mover as usize,
                        t,
                        0,
                        0,
                    );
                    for (lean, arm_tables) in &arms {
                        total += 1;
                        let mut egs =
                            EndgameSolver::<kwg::Node22, kwg::Node22>::new(&gc, &kwg, arm_tables)
                                .unwrap();
                        egs.init(&pos.board, [&pos.racks[0][..], &pos.racks[1][..]]);
                        let solvev = egs.solve_one_in_bag(mover, t).unwrap();
                        if solvev != super::to_equity(refv) {
                            disagreements += 1;
                            println!(
                                "DISAGREE [{name}] lean={lean} mover={mover} bag={t}: ref={refv} solve={}\n   {}",
                                solvev.raw(),
                                describe(&gc, &pos)
                            );
                        }
                    }
                }
            }
        }
        println!("one-in-bag differential: {disagreements} disagreements out of {total} cases");
        assert_eq!(
            disagreements, 0,
            "solve_one_in_bag disagreed with reference_one_in_bag"
        );
    }

    #[test]
    #[inline]
    fn one_in_bag_forced_double_pass() {
        let gc = game_config::make_english_game_config();
        let kwg_bytes = tiny_kwg_bytes();
        let kwg = kwg::Kwg::<kwg::Node22>::from_bytes_alloc(&kwg_bytes).unwrap();

        let board = empty_board();
        let racks = [vec![2u8, 2], vec![8u8, 20]];
        let arm_tables = anagrams::ArmTables::build(&kwg, &gc, false).unwrap();

        let mut egs =
            EndgameSolver::<kwg::Node22, kwg::Node22>::new(&gc, &kwg, &arm_tables).unwrap();
        egs.init(&board, [&racks[0][..], &racks[1][..]]);

        let v0 = egs.solve_one_in_bag(0, 2).unwrap();
        assert_eq!(
            v0,
            equity::Equity::new(-1000),
            "p0 forced-pass leftover should be -1000"
        );

        let mut egs =
            EndgameSolver::<kwg::Node22, kwg::Node22>::new(&gc, &kwg, &arm_tables).unwrap();
        egs.init(&board, [&racks[0][..], &racks[1][..]]);
        let v1 = egs.solve_one_in_bag(1, 2).unwrap();
        assert_eq!(
            v1,
            equity::Equity::new(1000),
            "p1 forced-pass leftover should be +1000"
        );
    }

    #[test]
    #[inline]
    fn the_scoreless_turns_key_a_position_apart() {
        // none is no key, so a placement's state keys as the position alone;
        // each count of turns keys apart from the others, and so does a pass to
        // answer
        assert_eq!(
            super::Scoreless::Counted {
                passes: 0,
                zeros: 0
            }
            .key(),
            0
        );
        let mut keys = [(1, 1), (2, 2), (1, 2), (0, 1), (5, 5), (6, 6)]
            .map(|(passes, zeros)| super::Scoreless::Counted { passes, zeros }.key())
            .to_vec();
        keys.push(super::Scoreless::Answering.key());
        for (i, &key) in keys.iter().enumerate() {
            assert_ne!(key, 0);
            assert!(keys[i + 1..].iter().all(|&other| other != key), "{i}");
        }
    }

    // the solver's line from the position for the player to move, after these
    // scoreless turns: its value and its moves, each as the tiles it places (a
    // pass as none)
    #[inline]
    fn line_after_scoreless_turns(
        racks: [&[u8]; 2],
        player_idx: u8,
        passes: u16,
        zeros: u16,
    ) -> (equity::Equity, Vec<Vec<u8>>) {
        let gc = game_config::make_english_game_config();
        let kwg_bytes = tiny_kwg_bytes();
        let kwg = kwg::Kwg::<kwg::Node22>::from_bytes_alloc(&kwg_bytes).unwrap();
        let arm_tables = anagrams::ArmTables::build(&kwg, &gc, false).unwrap();
        let mut egs =
            EndgameSolver::<kwg::Node22, kwg::Node22>::new(&gc, &kwg, &arm_tables).unwrap();
        egs.init_after_scoreless_turns(&empty_board(), racks, passes, zeros);
        let value = egs.solve(player_idx).unwrap();
        let mut pv = Vec::new();
        egs.collect_pv(player_idx, &mut pv);
        let line = pv
            .into_iter()
            .map(|(_, play)| match play {
                movegen::Play::Exchange { .. } => Vec::new(),
                movegen::Play::Place { word, .. } => word.to_vec(),
            })
            .collect();
        (value, line)
    }

    #[test]
    #[inline]
    fn after_a_phony_taken_back_an_opponent_who_can_only_pass_does_not_end_the_game() {
        // the phony counted as a pass (one pass, one zero); B alone plays
        // nothing, AT plays out for 4 and twice the B: the line is the pass,
        // then AT, and the game goes on past the pass
        let (value, line) = line_after_scoreless_turns([&[1, 20], &[2]], 1, 1, 1);
        assert_eq!(value, equity::Equity::new(-10_000));
        assert_eq!(line, vec![vec![], vec![1, 20]]);
    }

    #[test]
    #[inline]
    fn a_pass_at_one_scoreless_turn_short_of_the_end_ends_the_game_at_once() {
        // five zeros before the position and the sixth ends the game (English
        // counts to six): B's pass ends it, each losing their rack (the A and
        // the T are 2, the B 3), before AT can be played; one more pass to the
        // count, and a seventh never comes
        let (value, line) = line_after_scoreless_turns([&[1, 20], &[2]], 1, 5, 5);
        assert_eq!(value, equity::Equity::new(-1_000));
        assert_eq!(line, vec![Vec::<u8>::new()]);
        // with four, B's pass leaves AT to be played out: as after the phony
        let (value, line) = line_after_scoreless_turns([&[1, 20], &[2]], 1, 4, 4);
        assert_eq!(value, equity::Equity::new(-10_000));
        assert_eq!(line, vec![vec![], vec![1, 20]]);
    }

    #[test]
    #[inline]
    fn a_one_in_bag_solve_agrees_with_the_reference_and_beats_passing() {
        let gc = game_config::make_english_game_config();
        let kwg_bytes = tiny_kwg_bytes();
        let kwg = kwg::Kwg::<kwg::Node22>::from_bytes_alloc(&kwg_bytes).unwrap();
        let klv = klv::Klv::<kwg::Node22>::from_bytes_alloc(klv::EMPTY_KLV_BYTES).unwrap();
        let mut mg = movegen::KurniaMoveGenerator::new(&gc);

        let board = empty_board();
        let racks = [vec![1u8, 2], vec![8u8, 20]]; // p0=[A,B] p1=[H,T]
        let bag = 1u8; // one A in the bag
        let arm_tables = anagrams::ArmTables::build(&kwg, &gc, false).unwrap();

        // from no scoreless turns, and from some, up to one short of the end:
        // the reference counts them to the end while the solve ends a pass
        // answering a pass
        for (passes, zeros) in [(0, 0), (1, 1), (5, 5)] {
            let refv = reference_one_in_bag(
                &mut Reference {
                    gc: &gc,
                    kwg: &kwg,
                    klv: &klv,
                    mg: &mut mg,
                    memo: Default::default(),
                },
                &board,
                &racks,
                0,
                bag,
                passes,
                zeros,
            );
            let mut egs =
                EndgameSolver::<kwg::Node22, kwg::Node22>::new(&gc, &kwg, &arm_tables).unwrap();
            egs.init_after_scoreless_turns(
                &board,
                [&racks[0][..], &racks[1][..]],
                passes as u16,
                zeros as u16,
            );
            let solvev = egs.solve_one_in_bag(0, bag).unwrap();
            assert_eq!(
                solvev,
                super::to_equity(refv),
                "bag layer vs reference from {passes},{zeros}"
            );

            let a = gc.alphabet();
            let pass_only = equity::Equity::new(
                a.scaled_rack_score(&racks[1]) - a.scaled_rack_score(&racks[0]),
            );
            assert!(
                solvev >= pass_only,
                "playing should not be worse than the pass-only leftover: solve={solvev} pass_only={pass_only}"
            );
        }
    }

    #[test]
    #[inline]
    fn a_peg_pass_at_one_scoreless_turn_short_of_the_end_ends_the_game_at_once() {
        // T alone places nothing; with five zeros before the position (English
        // counts to six) its pass ends the game before the other side can play
        // AH: each loses their rack, whichever tile the bag holds (the A and
        // the T are 1, the H 4)
        let gc = game_config::make_english_game_config();
        let kwg_bytes = tiny_kwg_bytes();
        let kwg = kwg::Kwg::<kwg::Node22>::from_bytes_alloc(&kwg_bytes).unwrap();
        let board = empty_board();
        let mover_rack = [20u8]; // T
        let mut unseen_tally = vec![0u8; gc.alphabet().len() as usize];
        unseen_tally[1] = 2; // A, A
        unseen_tally[8] = 1; // H
        let arm_tables = anagrams::ArmTables::build(&kwg, &gc, false).unwrap();
        let mut egs =
            EndgameSolver::<kwg::Node22, kwg::Node22>::new(&gc, &kwg, &arm_tables).unwrap();
        let e = equity::Equity::new;
        let mut solve = |passes, zeros| {
            egs.set_scoreless_turns(passes, zeros);
            egs.solve_peg_one_in_bag(
                0,
                &board,
                &mover_rack,
                &unseen_tally,
                equity::Equity::ZERO,
                &super::Limits::NONE,
            )
            .unwrap()
            .expect("english one-in-bag is always solvable")
        };
        let result = solve(5, 5);
        assert_eq!(result.hypotheses, vec![(1, 2, e(4000)), (8, 1, e(1000))]);
        assert_eq!(result.win_pct, 1.0);
        assert_eq!(result.expected_margin, e(3000));
        // with four, the other side answers the pass with AH when it holds it,
        // and ends the game by passing when it holds the two A
        let result = solve(4, 4);
        assert!(result.hypotheses[0].2 < e(-1000), "{:?}", result.hypotheses);
        assert_eq!(result.hypotheses[1], (8, 1, e(1000)));
    }

    #[test]
    #[inline]
    fn differential_one_in_bag_exchange() {
        let gc = game_config::make_exchange_test_game_config();
        let kwg_bytes = tiny_kwg_bytes();
        let kwg = kwg::Kwg::<kwg::Node22>::from_bytes_alloc(&kwg_bytes).unwrap();
        let klv = klv::Klv::<kwg::Node22>::from_bytes_alloc(klv::EMPTY_KLV_BYTES).unwrap();
        let mut mg = movegen::KurniaMoveGenerator::new(&gc);

        let bag_tiles: [u8; 2] = [1, 0]; // A, blank
        let arms = both_arms(&kwg, &gc);

        let mut disagreements = 0usize;
        let mut total = 0usize;

        let positions: Vec<(String, Position)> = embedded_positions()
            .into_iter()
            .filter(|(_, pos)| pos.racks[0].len() <= 2 && pos.racks[1].len() <= 2)
            .collect();
        for (name, pos) in positions {
            for &t in &bag_tiles {
                for mover in 0u8..2 {
                    let refv = reference_one_in_bag_ex(
                        &mut Reference {
                            gc: &gc,
                            kwg: &kwg,
                            klv: &klv,
                            mg: &mut mg,
                            memo: Default::default(),
                        },
                        &pos.board,
                        &pos.racks,
                        mover as usize,
                        t,
                        0,
                        0,
                    );
                    for (lean, arm_tables) in &arms {
                        total += 1;
                        let mut egs =
                            EndgameSolver::<kwg::Node22, kwg::Node22>::new(&gc, &kwg, arm_tables)
                                .unwrap();
                        egs.init(&pos.board, [&pos.racks[0][..], &pos.racks[1][..]]);
                        let solvev = egs.solve_one_in_bag(mover, t).unwrap();
                        if solvev != super::to_equity(refv) {
                            disagreements += 1;
                            println!(
                                "DISAGREE [{name}] lean={lean} mover={mover} bag={t}: ref={refv} solve={}\n   {}",
                                solvev.raw(),
                                describe(&gc, &pos)
                            );
                        }
                    }
                }
            }
        }
        println!(
            "one-in-bag exchange differential: {disagreements} disagreements out of {total} cases"
        );
        assert_eq!(
            disagreements, 0,
            "solve_one_in_bag disagreed with reference_one_in_bag_ex"
        );
    }

    #[test]
    #[inline]
    fn one_in_bag_exchange_beats_passing() {
        let gc = game_config::make_exchange_test_game_config();
        let kwg_bytes = tiny_kwg_bytes();
        let kwg = kwg::Kwg::<kwg::Node22>::from_bytes_alloc(&kwg_bytes).unwrap();

        let board = empty_board();
        let racks = [vec![17u8], vec![1u8]]; // p0 = [Q], p1 = [A]
        let bag = 1u8; // one A in the bag

        let arm_tables = anagrams::ArmTables::build(&kwg, &gc, false).unwrap();
        let mut egs =
            EndgameSolver::<kwg::Node22, kwg::Node22>::new(&gc, &kwg, &arm_tables).unwrap();
        egs.init(&board, [&racks[0][..], &racks[1][..]]);
        let solvev = egs.solve_one_in_bag(0, bag).unwrap();

        let a = gc.alphabet();
        let pass_only =
            equity::Equity::new(a.scaled_rack_score(&racks[1]) - a.scaled_rack_score(&racks[0])); // -9000
        assert_eq!(
            solvev,
            equity::Equity::ZERO,
            "exchange line value should be exactly 0"
        );
        assert!(
            solvev > pass_only,
            "exchange should beat passing: solve={solvev} pass_only={pass_only}"
        );
    }

    #[test]
    #[inline]
    fn peg_one_in_bag_exchange_solved_or_declined() {
        let kwg_bytes = tiny_kwg_bytes();
        let kwg = kwg::Kwg::<kwg::Node22>::from_bytes_alloc(&kwg_bytes).unwrap();

        let gc = game_config::make_exchange_test_game_config();
        let board = empty_board();
        let mover_rack = [2u8]; // B, unplayable alone on an empty board
        let mut unseen_tally = vec![0u8; gc.alphabet().len() as usize];
        unseen_tally[8] = 1; // H
        unseen_tally[20] = 1; // T
        let arm_tables = anagrams::ArmTables::build(&kwg, &gc, false).unwrap();
        let mut egs =
            EndgameSolver::<kwg::Node22, kwg::Node22>::new(&gc, &kwg, &arm_tables).unwrap();
        let result = egs
            .solve_peg_one_in_bag(
                0,
                &board,
                &mover_rack,
                &unseen_tally,
                equity::Equity::ZERO,
                &super::Limits::NONE,
            )
            .unwrap()
            .expect("the exchange test config forces an end and is solvable");
        assert_eq!(result.hypotheses.len(), 2);

        let gc_en = game_config::make_english_game_config();
        let arm_tables_en = anagrams::ArmTables::build(&kwg, &gc_en, false).unwrap();
        let mut egs_en =
            EndgameSolver::<kwg::Node22, kwg::Node22>::new(&gc_en, &kwg, &arm_tables_en).unwrap();
        assert!(
            egs_en
                .solve_peg_one_in_bag(
                    0,
                    &board,
                    &mover_rack,
                    &unseen_tally,
                    equity::Equity::ZERO,
                    &super::Limits::NONE
                )
                .unwrap()
                .is_ok()
        );

        let gc_bad = game_config::make_exchange_unsolvable_test_game_config();
        let arm_tables_bad = anagrams::ArmTables::build(&kwg, &gc_bad, false).unwrap();
        let mut egs_bad =
            EndgameSolver::<kwg::Node22, kwg::Node22>::new(&gc_bad, &kwg, &arm_tables_bad).unwrap();
        assert!(matches!(
            egs_bad
                .solve_peg_one_in_bag(
                    0,
                    &board,
                    &mover_rack,
                    &unseen_tally,
                    equity::Equity::ZERO,
                    &super::Limits::NONE
                )
                .unwrap(),
            Err(super::PegUnsupported::ExchangeWithoutForcedEnd)
        ));
    }

    #[test]
    #[inline]
    fn differential_peg_committed() {
        let gc = game_config::make_english_game_config();
        let kwg_bytes = tiny_kwg_bytes();
        let kwg = kwg::Kwg::<kwg::Node22>::from_bytes_alloc(&kwg_bytes).unwrap();
        let klv = klv::Klv::<kwg::Node22>::from_bytes_alloc(klv::EMPTY_KLV_BYTES).unwrap();
        let mut mg = movegen::KurniaMoveGenerator::new(&gc);
        let a = gc.alphabet();
        let alen = a.len() as usize;
        let arms = both_arms(&kwg, &gc);

        let committed = |board: &[u8],
                         mover_rack: &[u8],
                         unseen: &[(u8, u32)],
                         mg: &mut movegen::KurniaMoveGenerator|
         -> (f32, f32, bool, String) {
            let total: u32 = unseen.iter().map(|&(_, w)| w).sum();
            let mover = 0usize;
            let opp = 1usize;
            let snap = movegen::BoardSnapshot {
                board_tiles: board,
                game_config: &gc,
                kwg: &kwg,
                anagrams: None,
                rack_lengths: None,
                klv: &klv,
            };
            mg.gen_moves_raw_all_unsorted(&snap, mover_rack, 0, movegen::PassPolicy::AsACandidate);
            let mut cands: Vec<Option<movegen::Play>> = vec![None]; // None = pass
            for vm in &mg.plays {
                if let movegen::Play::Place { .. } = &vm.play {
                    cands.push(Some(vm.play.clone()));
                }
            }
            let mut best = (f32::NEG_INFINITY, f32::NEG_INFINITY, false, String::new());
            for cand in &cands {
                let mut win = 0.0f32;
                let mut marg = 0.0f32;
                for &(t, w) in unseen {
                    let mut opp_rack = Vec::new();
                    for &(u, c) in unseen {
                        let take = if u == t { c - 1 } else { c };
                        for _ in 0..take {
                            opp_rack.push(u);
                        }
                    }
                    let drawn = t & !((t as i8) >> 7) as u8;
                    let v = match cand {
                        Some(movegen::Play::Place {
                            down,
                            lane,
                            idx,
                            word,
                            score,
                        }) => {
                            let mut nb = board.to_vec();
                            let mut nr = [mover_rack.to_vec(), Vec::new()];
                            apply_place(&gc, &mut nb, &mut nr[mover], *down, *lane, *idx, word);
                            nr[mover].push(drawn);
                            nr[opp] = opp_rack;
                            *score as f32
                                - reference(
                                    &mut Reference {
                                        gc: &gc,
                                        kwg: &kwg,
                                        klv: &klv,
                                        mg,
                                        memo: Default::default(),
                                    },
                                    &nb,
                                    &nr,
                                    opp,
                                    0,
                                    0,
                                )
                        }
                        _ => {
                            let nr = [mover_rack.to_vec(), opp_rack];
                            -reference_one_in_bag(
                                &mut Reference {
                                    gc: &gc,
                                    kwg: &kwg,
                                    klv: &klv,
                                    mg,
                                    memo: Default::default(),
                                },
                                board,
                                &nr,
                                opp,
                                t,
                                1,
                                1,
                            )
                        }
                    };
                    marg += (w as f32) * v;
                    win += (w as f32)
                        * if v > 0.0 {
                            1.0
                        } else if v == 0.0 {
                            0.5
                        } else {
                            0.0
                        };
                }
                marg /= total as f32;
                win /= total as f32;
                if win > best.0 || (win == best.0 && marg > best.1) {
                    let label = if cand.is_some() { "place" } else { "PASS" };
                    best = (win, marg, cand.is_some(), label.to_string());
                }
            }
            best
        };

        let tiles = [1u8, 2, 8, 20]; // A B H T
        let mut boards: Vec<(String, Vec<u8>)> = Vec::new();
        boards.push(("empty".into(), empty_board()));
        {
            let mut b = empty_board();
            put_word(&mut b, 7, 6, &[1, 20]);
            boards.push(("AT".into(), b));
        }
        {
            let mut b = empty_board();
            put_word(&mut b, 7, 6, &[1, 1, 8]);
            boards.push(("AAH".into(), b));
        }
        {
            let mut b = empty_board();
            put_word(&mut b, 7, 6, &[1, 20]);
            put_word(&mut b, 5, 7, &[8, 1, 20]);
            boards.push(("AT+HAT".into(), b));
        }

        let mut worst_gap = 0.0f32;
        let mut n_gap = 0;
        let mut n_gap_place = 0;
        let mut report = String::new();
        for (bname, board) in &boards {
            let mut racks: Vec<Vec<u8>> = Vec::new();
            for &x in &tiles {
                racks.push(vec![x]);
                for &y in &tiles {
                    racks.push(vec![x, y]);
                }
            }
            for mrack in &racks {
                for i in 0..tiles.len() {
                    for j in i..tiles.len() {
                        let (ti, tj) = (tiles[i], tiles[j]);
                        let mut unseen_tally = vec![0u8; alen];
                        let mut unseen: Vec<(u8, u32)> = Vec::new();
                        if ti == tj {
                            unseen_tally[ti as usize] = 2;
                            unseen.push((ti, 2));
                        } else {
                            unseen_tally[ti as usize] = 1;
                            unseen_tally[tj as usize] = 1;
                            unseen.push((ti, 1));
                            unseen.push((tj, 1));
                        }

                        let (ref_win, ref_marg, _ref_place, _ref_label) =
                            committed(board, mrack, &unseen, &mut mg);

                        for (lean, arm_tables) in &arms {
                            let mut egs = EndgameSolver::<kwg::Node22, kwg::Node22>::new(
                                &gc, &kwg, arm_tables,
                            )
                            .unwrap();
                            let got = egs
                                .solve_peg_one_in_bag(
                                    0,
                                    board,
                                    mrack,
                                    &unseen_tally,
                                    equity::Equity::ZERO,
                                    &super::Limits::NONE,
                                )
                                .unwrap()
                                .expect("english one-in-bag is always solvable");

                            let mut egs2 = EndgameSolver::<kwg::Node22, kwg::Node22>::new(
                                &gc, &kwg, arm_tables,
                            )
                            .unwrap();
                            let clair = egs2
                                .peg_clairvoyant_aggregate(
                                    0,
                                    board,
                                    mrack,
                                    &unseen_tally,
                                    0.0,
                                    &super::BagLimits::new(&super::Limits::NONE, None),
                                )
                                .unwrap();

                            assert!(got.committed, "english path must be committed");
                            assert!(got.best_move.is_some(), "committed result must name a move");
                            assert!(got.exact && clair.exact, "an unlimited solve is exact");
                            assert!(
                                (got.win_pct - ref_win).abs() < 1e-6,
                                "[{bname}] lean={lean} mover=[{}] unseen={unseen:?}: committed solver win% {} != reference {}",
                                a.fmt_rack(mrack),
                                got.win_pct,
                                ref_win,
                            );
                            // the solver's margin is rounded to a whole millipoint
                            assert!(
                                (got.expected_margin.raw() as f32 - ref_marg).abs() < 0.6,
                                "[{bname}] lean={lean} mover=[{}] unseen={unseen:?}: committed solver margin {} != reference {}",
                                a.fmt_rack(mrack),
                                got.expected_margin.raw(),
                                ref_marg,
                            );
                            assert!(
                                got.win_pct <= clair.win_pct + 1e-6,
                                "[{bname}] lean={lean} mover=[{}] unseen={unseen:?}: committed win% {} exceeds clairvoyant bound {}",
                                a.fmt_rack(mrack),
                                got.win_pct,
                                clair.win_pct,
                            );
                            let gap = clair.win_pct - got.win_pct;
                            if gap > 1e-6 {
                                n_gap += 1;
                                if gap > worst_gap {
                                    worst_gap = gap;
                                }
                                n_gap_place +=
                                    matches!(got.best_move, Some(PegMove::Place(_))) as usize;
                                if report.lines().count() < 25 {
                                    report.push_str(&format!(
                                        "gap win%={gap:.3} [{bname}] lean={lean} mover=[{}] unseen={unseen:?}: clairvoyant {:.3} vs committed {:.3}\n",
                                        a.fmt_rack(mrack), clair.win_pct, got.win_pct,
                                    ));
                                }
                            }
                        }
                    }
                }
            }
        }
        println!(
            "committed differential: solver matched reference on all scanned positions; \
             {n_gap} showed strict committed < clairvoyant ({n_gap_place} where committed best is a place move), worst gap = {worst_gap:.3}"
        );
        print!("{report}");
        assert!(
            n_gap >= 1,
            "toy corpus never exercised committed < clairvoyant; the differential proves nothing"
        );
    }
}
