// Copyright (C) 2020-2026 Andy Kurnia.

// note: this module is very slow and may need a lot of space
// and it still has many bugs

use super::{
    anagrams, bites, display, equity, error, fash, game_config, game_state, infer, klv, kwg,
    matrix, movegen, play_scorer, zone,
};
use std::io::Write;

// how many nodes the search visits between looks at the clock and the memory,
// for their limits, at most
const LIMIT_CADENCE: u64 = 64;

// the time a timed search's nodes may take between looks at the clock: past it,
// it looks twice as often, down to every node, and else half as often, up to
// LIMIT_CADENCE
const LOOK_EVERY: std::time::Duration = std::time::Duration::from_millis(1);

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
// pass is answered); with tiles in the bag, each count of each tile in it and
// each player's exchanges so far too; for what one player knows (InfoSet), a
// kind of its own with their lead. The index-th key is splitmix64's draw after
// index others from state 0, so no table needs to hold them
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
const BAG_KEYS: u64 = 5 << 32;
const EXCHANGES_MADE: u64 = 6 << 32;
const INFO_SETS: u64 = 7 << 32;
const INFO_LEAD: u64 = 8 << 32;

#[inline(always)]
fn exchanges_made_key(player: usize, made: i16) -> u64 {
    zobrist(EXCHANGES_MADE | ((player as u64) << u16::BITS) | u64::from(made.cast_unsigned()))
}

#[inline(always)]
fn info_lead_key(lead: i32) -> u64 {
    zobrist(INFO_LEAD | u64::from(lead.cast_unsigned()))
}

// the scoreless turns before a node, as a pass there sees them: either the
// counts the config checks (the position's own until the line places tiles;
// none from then on) or that the line's move before was a pass. A pass
// answering a pass ends the line, worth the game's end here whatever the counts
#[derive(Clone, Copy)]
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
                zobrist(SCORELESS_TURNS | (u64::from(passes) << 8) | u64::from(zeros))
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
    zobrist(SQUARE_KEYS | ((square as u64) << 8) | u64::from(tile))
}

// the change to a player's rack key when their count of a tile (a blank as 0)
// goes from count to count + 1, or back; a count of zero has no key
#[inline(always)]
fn rack_step(player: u8, tile: u8, count: usize) -> u64 {
    let key = |count: usize| {
        zobrist(RACK_KEYS | (u64::from(player) << 24) | (u64::from(tile) << 16) | count as u64)
    };
    if count == 0 {
        key(1)
    } else {
        key(count) ^ key(count + 1)
    }
}

// the key of a player's rack, in any order: each tile's count
#[inline(always)]
fn rack_key(player: u8, rack: &[u8]) -> u64 {
    let mut key = 0;
    for (i, &tile) in rack.iter().enumerate() {
        key ^= rack_step(
            player,
            tile,
            rack[..i].iter().filter(|&&t| t == tile).count(),
        );
    }
    key
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

// the least two values can differ by: every endgame value is a whole number of
// millipoints, so a window from a value less this to the value holds none
// strictly inside it
const LEAST_STEP: equity::Equity = equity::Equity::new(1);

// the best move as stored: its place among the moves the generator gives for
// the position, or a pass, or none
const PASS_PLAY: u16 = u16::MAX - 1;
const NO_PLAY: u16 = u16::MAX;

// what the table knows of a position
#[derive(Clone, Copy)]
struct Entry {
    value: equity::Equity,
    play: u16,
    depth: i8,
    bound: Bound,
}

// the positions the search has valued, in buckets of four entries, 64 bytes
// each, a cache line's size (not aligned to one). An entry is two words: the
// position's key, then, from the low bits up, the value's bits, the best move,
// the depth, the bound (none for an empty entry), and the generation of the
// solve that stored it or last read it. While the table grows (grow), first the
// bigger table, its room taken at once, is made empty a piece at a time; then,
// once it is the table, the old buckets are moved into it a piece at a time and
// as a key of theirs is read or stored (a moved bucket is left empty)
struct Table {
    buckets: Vec<[u64; 8]>,
    held: usize,         // entries holding a position, the old buckets' included
    generation: u64,     // the solve storing now
    next: Vec<[u64; 8]>, // the bigger table while it is made empty
    target: usize,       // its buckets then
    old: Vec<[u64; 8]>,  // the old buckets while they are moved
    moved: usize,        // the old buckets below this one are moved
}

// the error a search a limit stopped returns: a unit, so it is made with no
// allocation however often a search stops; it reads as the text it stands for
struct LimitStopped;

const LIMIT_STOPPED: &str = "a limit stopped the search";

impl std::fmt::Display for LimitStopped {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.pad(LIMIT_STOPPED)
    }
}

impl std::fmt::Debug for LimitStopped {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{LIMIT_STOPPED:?}")
    }
}

impl std::error::Error for LimitStopped {}

// the same for a search the memory limit stopped before it took what passes it
struct MemoryRefused;

const MEMORY_REFUSED: &str = "the memory limit stopped the search";

impl std::fmt::Display for MemoryRefused {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.pad(MEMORY_REFUSED)
    }
}

impl std::fmt::Debug for MemoryRefused {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{MEMORY_REFUSED:?}")
    }
}

impl std::error::Error for MemoryRefused {}

// the buckets (1 MiB) a table grows to at once; a bigger one grows a piece at
// each store, so no store takes the time of a whole move, and a solve stops
// within its time limit
const GROW_AT_ONCE: usize = 1 << 14;
// the buckets a store makes empty, or moves, while the table grows
const EMPTY_PIECE: usize = 1 << 12;
const MOVE_PIECE: usize = 1 << 10;

impl Table {
    const DEPTH_SHIFT: u32 = u32::BITS + u16::BITS;

    // a bucket to start with; the table grows as the search fills it
    #[inline(always)]
    fn new() -> Self {
        Self {
            buckets: vec![[0; 8]],
            held: 0,
            generation: 0,
            next: Vec::new(),
            target: 0,
            old: Vec::new(),
            moved: 0,
        }
    }

    // the table as new makes it, in place where it holds the one bucket new's
    // does and is not growing
    #[inline]
    fn renew(&mut self) {
        if self.buckets.len() != 1 || self.growing() {
            *self = Self::new();
            return;
        }
        self.buckets[0] = [0; 8];
        self.held = 0;
        self.generation = 0;
        self.next.clear();
        self.moved = 0;
    }

    // a table with room for this many buckets from the start (at least one),
    // growing on from there as new's does
    #[inline(always)]
    fn with_buckets(buckets: usize) -> Option<Self> {
        let buckets = buckets.max(1);
        let mut table = Vec::new();
        table.try_reserve_exact(buckets).ok()?;
        table.resize(buckets, [0; 8]);
        Some(Self {
            buckets: table,
            ..Self::new()
        })
    }

    // the table as with_buckets makes one of this many buckets (at least one),
    // emptied in place where it has that many and is not growing
    #[inline]
    fn empty_to(&mut self, buckets: usize) {
        let buckets = buckets.max(1);
        if self.buckets.len() != buckets || self.growing() {
            *self = Self::with_buckets(buckets).unwrap_or_else(Self::new);
            return;
        }
        self.buckets.fill([0; 8]);
        self.held = 0;
        self.generation = 0;
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
        ((u128::from(key) * self.buckets.len() as u128) >> 64) as usize
    }

    #[inline(always)]
    fn word(&self, entry: Entry) -> u64 {
        u64::from(entry.value.raw().cast_unsigned())
            | u64::from(entry.play) << u32::BITS
            | u64::from(entry.depth.cast_unsigned()) << Self::DEPTH_SHIFT
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
            value: equity::Equity::new((word as u32).cast_signed()),
            play: (word >> 32) as u16,
            depth: Self::depth(word),
            bound,
        })
    }

    #[inline(always)]
    fn depth(word: u64) -> i8 {
        ((word >> Self::DEPTH_SHIFT) as u8).cast_signed()
    }

    // the position's entry, which becomes this solve's: what a solve reads is
    // kept before what earlier solves stored and no solve since has read, as a
    // solve in steps or a search with a tile in the bag reads what it stored
    // before
    #[inline(always)]
    fn probe(&mut self, key: u64) -> Option<Entry> {
        self.touch(key);
        let generation = self.generation & 0x3f;
        let index = self.bucket(key);
        let bucket = &mut self.buckets[index];
        let slot = (0..4)
            .find(|&slot| bucket[2 * slot] == key && (bucket[2 * slot + 1] >> 56) & 3 != 0)?;
        let word = &mut bucket[2 * slot + 1];
        *word = *word & !(0x3f << 58) | generation << 58;
        Self::entry(*word)
    }

    // store into the position's own entry, else an empty one, else over the
    // shallowest one an earlier solve stored, else over the shallowest; a
    // deeper value of the position stays unless this one is exact. Returns
    // whether it took an empty entry
    #[inline(always)]
    fn store(&mut self, key: u64, entry: Entry) -> bool {
        self.touch(key);
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
            let depth = Self::depth(old);
            if bucket[2 * slot] == key {
                if depth <= entry.depth || entry.bound == Bound::Exact {
                    bucket[2 * slot + 1] = word;
                }
                return false;
            }
            let rank = i32::from(depth) + if old >> 58 == generation { 256 } else { 0 };
            if rank < victim_rank {
                victim = slot;
                victim_rank = rank;
            }
        }
        bucket[2 * victim] = key;
        bucket[2 * victim + 1] = word;
        false
    }

    // whether the table is growing
    #[inline(always)]
    fn growing(&self) -> bool {
        self.target != 0 || !self.old.is_empty()
    }

    // the bytes of the buckets a growing table leaves: its own while the bigger
    // table is made empty, then the old ones until they are moved; none when it
    // is not growing
    #[inline(always)]
    fn leaving(&self) -> usize {
        let buckets = if self.target != 0 {
            self.buckets.capacity()
        } else {
            self.old.capacity()
        };
        buckets * std::mem::size_of::<[u64; 8]>()
    }

    // begin moving the table to this many buckets, keeping every entry that
    // fits, a piece at each advance; false, the table as it was, when the
    // memory for them is refused (as a wasm32 page's may be before any limit
    // set here is reached)
    #[inline(never)]
    fn grow(&mut self, buckets: usize) -> bool {
        let mut next = Vec::new();
        if next.try_reserve_exact(buckets).is_err() {
            return false;
        }
        self.next = next;
        self.target = buckets;
        true
    }

    // the growing done to its end
    #[inline(never)]
    fn finish(&mut self) {
        while self.growing() {
            self.advance();
        }
    }

    // a piece of the growing: empty buckets added to the bigger table, which
    // then becomes the table; or old buckets moved into it, the old table freed
    // once all are
    #[inline(never)]
    fn advance(&mut self) {
        if self.target != 0 {
            let len = (self.next.len() + EMPTY_PIECE).min(self.target);
            self.next.resize(len, [0; 8]);
            if len == self.target {
                self.old = std::mem::replace(&mut self.buckets, std::mem::take(&mut self.next));
                self.target = 0;
                self.moved = 0;
            }
            return;
        }
        let end = (self.moved + MOVE_PIECE).min(self.old.len());
        for index in self.moved..end {
            self.move_bucket(index);
        }
        self.moved = end;
        if end == self.old.len() {
            self.old = Vec::new();
        }
    }

    // a key's old bucket moved, while the table grows, before the key is read
    // or stored, so a position is only ever in the table it moved to
    #[inline(always)]
    fn touch(&mut self, key: u64) {
        if !self.old.is_empty() {
            let index = ((u128::from(key) * self.old.len() as u128) >> 64) as usize;
            if index >= self.moved {
                self.move_bucket(index);
            }
        }
    }

    // an old bucket's entries put into the table, and the bucket left empty
    #[inline(always)]
    fn move_bucket(&mut self, index: usize) {
        let bucket = std::mem::take(&mut self.old[index]);
        for slot in 0..4 {
            let word = bucket[2 * slot + 1];
            if (word >> 56) & 3 != 0 {
                self.held -= 1;
                self.put(bucket[2 * slot], word);
            }
        }
    }

    // place a stored word of a position not in the table, as a growing keeps
    // it: in an empty entry, else in place of the shallowest if it is deeper
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
            let depth = Self::depth(old);
            if depth < victim_depth {
                victim = slot;
                victim_depth = depth;
            }
        }
        if Self::depth(word) > victim_depth {
            bucket[2 * victim] = key;
            bucket[2 * victim + 1] = word;
        }
    }
}

// a position's value as a search found it, and whether no line below it was cut
// short by the depth
struct Searched {
    value: equity::Equity,
    complete: bool,
}

// a node to search: negamax_eval's arguments
#[derive(Clone, Copy)]
struct Enter {
    ply: usize,
    player_idx: u8,
    depth: i8,
    alpha: equity::Equity,
    beta: equity::Equity,
    scoreless: Scoreless,
}

// a node on the search's stack: what negamax_eval keeps of it while its moves
// are searched, the recursion made a loop over these so that a stopped search
// can go on where it stopped
struct Frame {
    node: Enter,
    root: bool,
    key: u64,
    alpha_searched: equity::Equity,
    table_first: bool,
    best: Searched,
    best_play: u16,
    first_is_best: bool,
    next: usize, // the place in its order of the next move to search
    child: Child,
}

impl Frame {
    // a node's frame before any of its moves is searched
    #[inline(always)]
    fn new(node: Enter, root: bool, key: u64, table_first: bool) -> Self {
        // its first_is_best, at a pass's root: true when the first move searched
        // was the last finished pass's best, so that a later move with a higher
        // exact value outranks it at this depth
        Self {
            node,
            root,
            key,
            alpha_searched: node.alpha,
            table_first,
            best: Searched {
                value: equity::Equity::NEG_INFINITY,
                complete: true,
            },
            best_play: NO_PLAY,
            first_is_best: false,
            next: 0,
            child: Child::Moves,
        }
    }

    // a move's value taken (the move at place i of its order); whether it
    // reached beta
    #[inline(always)]
    fn take_value(
        &mut self,
        proven: &mut Option<(u16, equity::Equity)>,
        i: usize,
        index: u16,
        value: equity::Equity,
    ) -> bool {
        if value > self.best.value {
            if i > 0 && self.first_is_best && value > self.alpha_searched && value < self.node.beta
            {
                *proven = Some((index, value));
            }
            self.best.value = value;
            self.best_play = index;
            if value > self.node.alpha {
                self.node.alpha = value;
            }
            if value >= self.node.beta {
                return true;
            }
        }
        false
    }

    // the pass's value taken, as a move's is
    #[inline(always)]
    fn take_pass_value(
        &mut self,
        proven: &mut Option<(u16, equity::Equity)>,
        value: equity::Equity,
    ) {
        if value > self.best.value {
            if self.first_is_best && value > self.alpha_searched && value < self.node.beta {
                *proven = Some((PASS_PLAY, value));
            }
            self.best.value = value;
            self.best_play = PASS_PLAY;
        }
    }
}

// what a frame searches below it
enum Child {
    Moves, // none yet, its moves not done
    Move {
        index: u16,
        score: equity::Equity,
        play: movegen::Play,
        scout: bool, // searched with a zero-width window, to be searched again past it
    }, // a move, placed
    Pass,  // the pass
    PassDone, // its moves and its pass done
}

// a frame's next step
enum Advance {
    Enter(Enter),   // a child to search
    Done(Searched), // its value, its frame popped
}

// a pass a limit stopped in a timed solve, kept for the next step's solve to go
// on from: the frames, the node it stopped at (and whether that was the root),
// which pass (the position's key, the mover, the depth, and the window), the
// move it proved, each ply's lists (which the frames read), and how many lists
// were generated, so that a list the next step's solver generates gets a serial
// none of those has
struct Stopped {
    frames: Vec<Frame>,
    pending: (Enter, bool),
    pass: (u64, u8, i8, equity::Equity, equity::Equity),
    proven: Option<(u16, equity::Equity)>,
    lists: Vec<Vec<Candidate>>,
    orders: Vec<Vec<u128>>,
    zones: Vec<(u128, u128)>,
    spent: Vec<u32>,
    keeps: Vec<bool>,
    serials: Vec<u64>,
    siblings: Vec<Siblings>,
    generated: u64,
}

// a line collect_pv read, what it was read for (the position's key, the mover,
// and the deepest finished pass), and the move and value it starts with. While
// the three stay and a stopped solve answers with that move, if any, it is the
// line again, read without the table, whose entries along it a later pass may
// replace
struct Line {
    of: (u64, u8, i8),
    start: Option<(u16, equity::Equity)>,
    plies: Vec<(equity::Equity, movegen::Play)>,
}

// a nonempty set of legal first plays of a particular position
pub struct RootMoves {
    plays: Vec<movegen::Play>,
    pass: bool,
    key: u64,
}

impl RootMoves {
    // check the plays against the position, using the normal move scorer
    pub fn checked<N: kwg::Node, L: kwg::Node>(
        snapshot: &movegen::BoardSnapshot<'_, N, L>,
        state: &game_state::GameState,
        plays: Vec<movegen::Play>,
    ) -> error::Returns<Self> {
        if plays.is_empty() || plays.len() > PASS_PLAY as usize {
            return Err(format!("root_moves: expected 1 to {PASS_PLAY} plays").into());
        }
        use std::hash::BuildHasher;
        let mut scorer = play_scorer::PlayScorer::new();
        let mut checked = Vec::new();
        checked.try_reserve_exact(plays.len())?;
        let hasher = fash::MyHasherDefault::default();
        let mut hashes = fash::MyHashSet::default();
        hashes.try_reserve(plays.len())?;
        let mut pass = false;
        for (i, play) in plays.into_iter().enumerate() {
            if let movegen::Play::Exchange { tiles } = &play
                && !tiles.is_empty()
                && (tiles.len() > state.bag.len()
                    || !exchange_legal(
                        snapshot.game_config,
                        state.bag.len(),
                        state.current_player().num_exchanges,
                    ))
            {
                return Err(format!("root_moves[{i}]: exchange is unavailable").into());
            }
            let play = scorer
                .validate_play(snapshot, state, &play)
                .map_err(|e| format!("root_moves[{i}]: {e}"))?
                .unwrap_or(play);
            if !scorer.words_are_valid(snapshot, &play) {
                return Err(format!("root_moves[{i}]: play makes an invalid word").into());
            }
            match &play {
                movegen::Play::Place { score, .. }
                    if score.raw() != scorer.compute_score(snapshot, &play) =>
                {
                    return Err(format!("root_moves[{i}]: wrong score").into());
                }
                movegen::Play::Exchange { tiles } if tiles.is_empty() => pass = true,
                _ => {}
            }
            // only a play whose hash was seen before can equal an earlier one
            if !hashes.insert(hasher.hash_one(&play)) && checked.contains(&play) {
                return Err(format!("root_moves[{i}]: duplicate play").into());
            }
            checked.push(play);
        }
        let key = hasher.hash_one(&checked).max(1);
        Ok(Self {
            plays: checked,
            pass,
            key,
        })
    }
}

// bring named moves forward in request order, leaving the others ordered
fn prioritize<T>(
    mut pending: &mut [T],
    requested: &[movegen::Play],
    matches: impl Fn(&T, &movegen::Play) -> bool,
) {
    for play in requested {
        if let Some(at) = pending.iter().position(|item| matches(item, play)) {
            pending[..=at].rotate_right(1);
            pending = &mut pending[1..];
        }
    }
}

// a move generated for a position, as the search orders it
struct Candidate {
    play: movegen::Play,
    uses: u32,  // its rack places (first slots), set when its list keeps them
    index: u16, // its place in the sorted list, NO_PLAY past what fits
    valuation: equity::Equity, // a play out's whole value, else its score
    plays_out: bool,
    placed: u8, // the tiles it places
}

#[inline(always)]
fn add_placement(
    list: &mut Vec<Candidate>,
    ordered: &mut Vec<u128>,
    play: &movegen::Play,
    uses: u32,
    placed: usize,
    rack_len: usize,
    out_bonus: i32,
) -> error::Returns<()> {
    if let movegen::Play::Place {
        down,
        lane,
        idx,
        score,
        ..
    } = play
    {
        let plays_out = placed == rack_len;
        let valuation = if plays_out {
            score.raw() + out_bonus
        } else {
            score.raw()
        };
        let found = u32::try_from(list.len())?;
        ordered.push(order_key(valuation, *down, *lane, *idx, found));
        list.push(Candidate {
            play: play.clone(),
            uses,
            index: NO_PLAY,
            valuation: equity::Equity::new(valuation),
            plays_out,
            placed: placed as u8,
        });
    }
    Ok(())
}

// a ply's moves on the lanes only the move two plies up changed, found by the
// replies so far to one node's list (of, that list's serial): every reply that
// leaves such a lane alone finds the same moves there, the mover's rack being
// the same, so later ones copy them
#[derive(Default)]
struct Siblings {
    of: u64,
    rows: u128,
    cols: u128,
    moves: Vec<(movegen::Play, u8)>, // each with the tiles it places
}

// the plays out a ply keeps of the opponent's (OutReplies)
const OUT_SLOTS: usize = 4;

// a play out by the opponent of a ply's mover, legal on that ply's position:
// its score, its zone there (zone::footprint), and where it lies, to tell it
// from another
#[derive(Clone, Copy)]
struct Out {
    score: equity::Equity,
    zone: zone::Squares,
    down: bool,
    lane: i8,
    idx: i8,
}

// a node's plays out by the opponent, highest score first, found in its
// children's lists: of, its list's serial; floor, the lists generated when its
// last child was entered, so that the child's own list is told from an older
// one. A play out its move leaves alone (no square of its zone filled) is still
// the opponent's, at the same score, so the move is worth at most its score
// minus that play's score minus twice the rack the mover keeps
struct OutReplies {
    of: u64,
    floor: u64,
    len: usize,
    slots: [Out; OUT_SLOTS],
}

impl Default for OutReplies {
    fn default() -> Self {
        Self {
            of: 0,
            floor: u64::MAX,
            len: 0,
            slots: [Out {
                score: equity::Equity::new(0),
                zone: zone::Squares::NONE,
                down: false,
                lane: 0,
                idx: 0,
            }; OUT_SLOTS],
        }
    }
}

// 128-bit FNV-1a's offset basis and prime:
// http://www.isthe.com/chongo/tech/comp/fnv/index.html
const FNV128_BASIS: u128 = 0x6c62272e07bb014262b821756295c58d;
const FNV128_PRIME: u128 = (1 << 88) + 0x13b;

// a lane's key, its rack aside: whether the board is empty, the lane, each
// square's tile, or for an empty one the tiles beside it across the lane; so
// two lanes with one key take the same moves from the same rack. By 128-bit
// FNV-1a
#[inline(always)]
fn lane_key(
    board_tiles: &[u8],
    dim: &matrix::Dim,
    board_empty: bool,
    down: bool,
    lane: i8,
) -> u128 {
    let mut key = FNV128_BASIS;
    lane_bytes(board_tiles, dim, board_empty, down, lane, |byte| {
        key = (key ^ u128::from(byte)).wrapping_mul(FNV128_PRIME)
    });
    key
}

// the bytes a lane's key hashes (lane_key), each given to mix in turn; none of
// a lane's tiles is 0xff, so two lanes that differ in what the key holds never
// give the same bytes
#[inline(always)]
fn lane_bytes(
    board_tiles: &[u8],
    dim: &matrix::Dim,
    board_empty: bool,
    down: bool,
    lane: i8,
    mut mix: impl FnMut(u8),
) {
    mix(u8::from(board_empty));
    mix(u8::from(down));
    mix(lane as u8);
    let strider = dim.lane(down, lane);
    for i in 0..strider.len() {
        let tile = board_tiles[strider.at_in_lane(i)];
        mix(tile);
        if tile == 0 {
            let across = dim.lane(!down, i);
            for step in [-1i8, 1] {
                let mut at = lane + step;
                while at >= 0 && at < across.len() && board_tiles[across.at_in_lane(at)] != 0 {
                    mix(board_tiles[across.at_in_lane(at)]);
                    at += step;
                }
                mix(0xff);
            }
        }
    }
}

// a placement's tiles, or a rack's, in a pool's places (LanePool::places): a
// count each in four bits, NIBBLE_MOST at most
type Tiles = u64;

// the most a nibble counts below its top bit, which fits lifts
const NIBBLE_MOST: usize = 7;

// whether a rack's tiles hold need's: with each of the rack's counts lifted by
// eight, need's taken away leaves at least eight in each where the rack has
// enough, borrowing from no other count
#[inline(always)]
fn fits(held: Tiles, need: Tiles) -> bool {
    const LIFT: Tiles = Tiles::MAX / 15 * 8;
    ((held | LIFT) - need) & LIFT == LIFT
}

// the lane keys a pool's doorkeeper holds (LanePool::sighted)
const SIGHTED: usize = 1 << 16;

// a lane slot's list while its key is not known, and a key table's empty slot
const NO_LIST: u32 = u32::MAX;

// 128-bit keys, each a hash, to a u32 each other than NO_LIST: an open table
// whose slots are found by a multiplicative hash of the key's halves, then one
// after another, kept under half full by doubling
#[derive(Default)]
struct KeyTable {
    keys: Vec<u128>,
    values: Vec<u32>, // NO_LIST in an empty slot
    held: usize,
}

impl KeyTable {
    #[inline(always)]
    fn first(&self, key: u128) -> usize {
        let mixed = (key as u64 ^ (key >> 64) as u64).wrapping_mul(SPLITMIX64_GAMMA);
        (mixed >> (64 - self.keys.len().trailing_zeros())) as usize
    }

    #[inline(always)]
    fn get(&self, key: u128) -> Option<u32> {
        if self.keys.is_empty() {
            return None;
        }
        let mask = self.keys.len() - 1;
        let mut at = self.first(key);
        loop {
            let value = self.values[at];
            if value == NO_LIST {
                return None;
            }
            if self.keys[at] == key {
                return Some(value);
            }
            at = (at + 1) & mask;
        }
    }

    #[inline(always)]
    fn insert(&mut self, key: u128, value: u32) -> bool {
        if 2 * (self.held + 1) > self.keys.len() && !self.grow() {
            return false;
        }
        let mask = self.keys.len() - 1;
        let mut at = self.first(key);
        while self.values[at] != NO_LIST && self.keys[at] != key {
            at = (at + 1) & mask;
        }
        self.held += usize::from(self.values[at] == NO_LIST);
        self.keys[at] = key;
        self.values[at] = value;
        true
    }

    fn grow(&mut self) -> bool {
        let size = (2 * self.keys.len()).max(1 << 10);
        let (mut keys, mut values) = (Vec::new(), Vec::new());
        if keys.try_reserve_exact(size).is_err() || values.try_reserve_exact(size).is_err() {
            return false;
        }
        keys.resize(size, 0);
        values.resize(size, NO_LIST);
        let keys = std::mem::replace(&mut self.keys, keys);
        let values = std::mem::replace(&mut self.values, values);
        self.held = 0;
        for (key, value) in keys.into_iter().zip(values) {
            if value != NO_LIST {
                self.insert(key, value);
            }
        }
        true
    }

    // the bytes it holds
    #[inline(always)]
    fn bytes(&self) -> usize {
        self.keys.capacity() * std::mem::size_of::<u128>()
            + self.values.capacity() * std::mem::size_of::<u32>()
    }

    // every slot made empty in place (a slot reads as empty by its value)
    #[inline]
    fn clear(&mut self) {
        self.values.fill(NO_LIST);
        self.held = 0;
    }
}

// whether every tile of a sorted rack is in a sorted pool, as many times
#[inline(always)]
fn within(rack: &[u8], pool: &[u8]) -> bool {
    let mut at = 0;
    rack.iter().all(|&tile| {
        while at < pool.len() && pool[at] < tile {
            at += 1;
        }
        let found = at < pool.len() && pool[at] == tile;
        at += 1;
        found
    })
}

#[inline(always)]
fn move_key_head(score: i32, len: usize, idx: i8, lane: i8, down: bool) -> u64 {
    (u64::from(score.cast_unsigned()) << u32::BITS)
        | ((len as u64) << (3 * u8::BITS))
        | (u64::from(idx.cast_unsigned()) << (2 * u8::BITS))
        | (u64::from(lane.cast_unsigned()) << u8::BITS)
        | u64::from(down)
}

// a move hashed into a list's key, eight bytes of it at a time, by FNV-1a's
// step on 128 bits (as lane_key)
#[inline(always)]
fn mix_move(key: u128, play: &movegen::Play) -> u128 {
    let movegen::Play::Place {
        down,
        lane,
        idx,
        word,
        score,
    } = play
    else {
        return key;
    };
    let mix = |key: u128, part: u64| (key ^ u128::from(part)).wrapping_mul(FNV128_PRIME);
    let head = move_key_head(score.raw(), word.len(), *idx, *lane, *down);
    word.chunks(8).fold(mix(key, head), |key, chunk| {
        let mut bytes = [0u8; 8];
        bytes[..chunk.len()].copy_from_slice(chunk);
        mix(key, u64::from_le_bytes(bytes))
    })
}

// the tiles a search with tiles in the bag deals from (its LanePool's pool):
// every one off the board at its root, sorted, the mover's among them (also
// sorted); and the bytes each of its pools may hold
#[derive(Clone, Default, PartialEq)]
struct Deal {
    pool: Vec<u8>,
    mover: Vec<u8>,
    budget: usize,
}

// the share of the memory limit a search's lane pools may hold, one over this,
// split alike among its threads
const POOL_SHARE: usize = 8;

// why a lane pool has no room for a walk's lists (LanePool::room_for): none
// past its budget or what its ids number (full), or none the memory limit
// leaves
#[derive(PartialEq)]
enum PoolRoom {
    Full,
    NoMemory,
}

// the pool's version of a lane's moves, for a search with tiles in the bag:
// made with the pool from the lane's tiles with its cross sets alone, every
// tile off the board at the search's root, placing no more tiles than a rack
// holds; so they hold for every endgame inside it, both players sharing them. A
// lane key met twice has its moves as a list of ids into one table, each move
// kept once, grouped by the tiles they place; a list made again is kept once. A
// rack takes each group its tiles hold. The slots (the rows', then the
// columns') hold the position searched's keys with their lists; a move placed
// makes its zone's stale, taking it back puts them back. Past its budget, every
// list is let go
struct LanePool {
    deal: Deal,                      // its pool, none outside such a search
    places: [u8; 256],               // each tile's nibble; 0xff off the pool
    moves: Vec<(movegen::Play, u8)>, // each move kept, with the tiles it places
    known: Vec<KeyTable>,            // each slot's moves' ids, by their hashes
    keys: KeyTable,                  // each lane key's list
    contents: KeyTable,              // each list, by its moves' hashes
    lists: Vec<[u32; 3]>,            // each list's groups and first entry
    needs: Vec<Tiles>,               // each group's tiles
    ends: Vec<u32>,                  // the place past each group's last entry
    entries: Vec<u32>,               // the groups' moves' ids
    slot_keys: Vec<u128>,
    slot_lists: Vec<u32>,         // NO_LIST where the key is not known
    undo: Vec<(u16, u128, u32)>,  // stale slots as they were
    marks: Vec<u32>,              // each placed move's start in undo
    fresh: Vec<u16>,              // a generation's slots with no list yet
    by_slot: Vec<u32>,            // a walk's moves, slot by slot
    starts: Vec<u32>,             // where each slot's moves end in by_slot
    hashes: Vec<u128>,            // a slot's moves' hashes, as walked
    sorting: Vec<(Tiles, u32)>,   // a new list's moves by their tiles
    scans: Vec<(u32, Tiles)>,     // each slot's list and rack last fetched
    ranges: Vec<Vec<(u32, u32)>>, // and the entries that fetch took
    seen: Vec<u64>,               // lane keys walked once, a tag each
    #[cfg(debug_assertions)]
    lanes: fash::MyHashMap<u128, Vec<u8>>,
}

impl Default for LanePool {
    fn default() -> Self {
        Self::new()
    }
}

impl LanePool {
    fn new() -> Self {
        Self {
            deal: Deal::default(),
            places: [0xff; 256],
            moves: Vec::new(),
            known: Vec::new(),
            keys: KeyTable::default(),
            contents: KeyTable::default(),
            lists: Vec::new(),
            needs: Vec::new(),
            ends: Vec::new(),
            entries: Vec::new(),
            slot_keys: Vec::new(),
            slot_lists: Vec::new(),
            undo: Vec::new(),
            marks: Vec::new(),
            fresh: Vec::new(),
            by_slot: Vec::new(),
            starts: Vec::new(),
            hashes: Vec::new(),
            sorting: Vec::new(),
            scans: Vec::new(),
            ranges: Vec::new(),
            seen: Vec::new(),
            #[cfg(debug_assertions)]
            lanes: fash::MyHashMap::default(),
        }
    }

    // whether a lane key no list has was walked before: a key's first walk is
    // the rack's and its second the pool's, so a lane met once makes no list
    #[inline(always)]
    fn sighted(&mut self, key: u128) -> bool {
        debug_assert_eq!(self.seen.len(), SIGHTED);
        let mixed = (key as u64).wrapping_mul(SPLITMIX64_GAMMA);
        let at = (mixed >> (64 - SIGHTED.trailing_zeros())) as usize;
        let tag = (key >> 64) as u64 | 1;
        if self.seen[at] == tag {
            true
        } else {
            self.seen[at] = tag;
            false
        }
    }

    #[inline(always)]
    fn sees_within(&mut self, memory_limit: Option<MemoryLimit>) -> bool {
        !self.seen.is_empty() || self.see_within(memory_limit)
    }

    #[cold]
    fn see_within(&mut self, memory_limit: Option<MemoryLimit>) -> bool {
        let bytes = SIGHTED * std::mem::size_of::<u64>();
        let Some(_room) = reserve_in(memory_limit.as_ref(), bytes) else {
            return false;
        };
        let mut seen = Vec::new();
        if seen.try_reserve_exact(SIGHTED).is_err() {
            return false;
        }
        seen.resize(SIGHTED, 0);
        self.seen = seen;
        true
    }

    // the bytes its lists hold
    fn bytes(&self) -> usize {
        fn of<T>(v: &Vec<T>) -> usize {
            v.capacity() * std::mem::size_of::<T>()
        }
        of(&self.moves)
            + (self.known.iter()).map(KeyTable::bytes).sum::<usize>()
            + self.keys.bytes()
            + self.contents.bytes()
            + of(&self.lists)
            + of(&self.needs)
            + of(&self.ends)
            + of(&self.entries)
    }

    // every list and move let go, the pool kept, each slot stale; with keep,
    // the lists' buffers emptied in place for the lists made again while they
    // fit the pool's budget, else freed
    fn let_go(&mut self, keep: bool) {
        if keep && self.bytes() <= self.deal.budget {
            self.moves.clear();
            self.known.iter_mut().for_each(KeyTable::clear);
            self.keys.clear();
            self.contents.clear();
            self.lists.clear();
            self.needs.clear();
            self.ends.clear();
            self.entries.clear();
        } else {
            self.moves = Vec::new();
            self.known = Vec::new();
            self.keys = KeyTable::default();
            self.contents = KeyTable::default();
            self.lists = Vec::new();
            self.needs = Vec::new();
            self.ends = Vec::new();
            self.entries = Vec::new();
        }
        self.slot_lists.fill(NO_LIST);
        self.scans.fill((NO_LIST, 0));
        for undone in self.undo.iter_mut() {
            undone.2 = NO_LIST;
        }
    }

    #[inline(always)]
    fn room_for(
        &self,
        memory_limit: Option<MemoryLimit>,
        plays: &[movegen::ValuedMove],
        (lanes, longest): (usize, usize),
    ) -> Result<Reserved, PoolRoom> {
        let moves = plays.len();
        let below = |len: usize, more: usize| len + more < NO_LIST as usize;
        if !(below(self.moves.len(), moves)
            && below(self.ends.len(), moves)
            && below(self.entries.len(), moves)
            && below(self.lists.len(), lanes))
        {
            return Err(PoolRoom::Full);
        }
        let Some(limit) = memory_limit else {
            return Ok(Reserved::NONE);
        };
        let table = |table: &KeyTable, more: usize| {
            if 2 * (table.held + more) > table.keys.len() {
                3 * table.bytes().max(1 << 14)
            } else {
                0
            }
        };
        let known: usize = (self.fresh.iter())
            .map(|&slot| match self.known.get(slot as usize) {
                Some(known) => table(known, moves),
                None => table(&KeyTable::default(), moves),
            })
            .sum();
        let needed = growth(&self.moves, moves)
            + growth(&self.needs, moves)
            + growth(&self.ends, moves)
            + growth(&self.entries, moves)
            + growth(&self.lists, lanes)
            + table(&self.keys, lanes)
            + table(&self.contents, lanes)
            + known;
        let slots = self.slot_lists.len();
        let scratch = growth(&self.by_slot, moves)
            + growth(&self.hashes, moves)
            + growth(&self.sorting, moves)
            + growth(&self.known, slots)
            + growth(&self.starts, slots + 1);
        let tiles = if bites::Bites::heap_bytes(longest) == 0 {
            0
        } else {
            (plays.iter())
                .map(|valued| play_bytes(&valued.play))
                .sum::<usize>()
        };
        let Some(room) = limit.reserve(needed + scratch + tiles) else {
            return Err(PoolRoom::NoMemory);
        };
        if self.bytes() + needed > self.deal.budget {
            Err(PoolRoom::Full)
        } else {
            Ok(room)
        }
    }

    // the deal of a search, its racks holding at most most tiles; no pool when
    // it holds more kinds than Tiles has nibbles, or a rack more tiles than a
    // nibble counts
    fn adopt_deal(&mut self, deal: &Deal, most: usize) {
        if self.deal != *deal {
            self.deal.clone_from(deal);
            self.places = [0xff; 256];
            let mut kinds = 0u32;
            for &tile in &deal.pool {
                let place = &mut self.places[tile as usize];
                if *place == 0xff {
                    *place = kinds as u8;
                    kinds += 1;
                }
            }
            if kinds > Tiles::BITS / 4 || most > NIBBLE_MOST || deal.mover.len() > most {
                self.deal.pool.clear();
            }
            self.let_go(false);
        }
    }

    // a rack's tiles, each of them in the pool
    #[inline(always)]
    fn held_by(&self, rack: &[u8]) -> Tiles {
        (rack.iter())
            .map(|&tile| 1 << (4 * u32::from(self.places[tile as usize])))
            .sum()
    }

    // every slot stale, the position searched set afresh
    #[inline(always)]
    fn reset_slots(&mut self, slots: usize) {
        self.slot_keys.resize(slots, 0);
        self.slot_lists.clear();
        self.slot_lists.resize(slots, NO_LIST);
        self.scans.resize(slots, (NO_LIST, 0));
        self.ranges.resize_with(slots, Vec::new);
        self.undo.clear();
        self.marks.clear();
    }

    // a move about to be placed: its zone's slots kept, then made stale
    #[inline(always)]
    fn stale(&mut self, rows: u128, cols: u128, num_rows: usize) {
        self.marks.push(self.undo.len() as u32);
        for (mut lanes, base) in [(rows, 0), (cols, num_rows)] {
            while lanes != 0 {
                let slot = base + lanes.trailing_zeros() as usize;
                lanes &= lanes - 1;
                self.undo
                    .push((slot as u16, self.slot_keys[slot], self.slot_lists[slot]));
                self.slot_lists[slot] = NO_LIST;
            }
        }
    }

    // a move taken back: its zone's slots as they were
    #[inline(always)]
    fn unstale(&mut self) {
        let Some(mark) = self.marks.pop() else {
            return;
        };
        for &(slot, key, list) in &self.undo[mark as usize..] {
            self.slot_keys[slot as usize] = key;
            self.slot_lists[slot as usize] = list;
        }
        self.undo.truncate(mark as usize);
    }

    // a walk's moves on the fresh slots made their lists, then keyed: a slot's
    // moves as walked found by their hashes among the lists made before, else
    // grouped by the tiles they place, each move found in the table, else put
    // in it; false, every list let go, where the memory limit has no room for
    // them. A pool full keeps the lists' buffers for those made again; the
    // memory limit or the allocator refusing them, they are freed
    fn record(
        &mut self,
        plays: &[movegen::ValuedMove],
        memory_limit: Option<MemoryLimit>,
        num_rows: usize,
    ) -> bool {
        let longest = num_rows.max(self.slot_lists.len().saturating_sub(num_rows));
        let room = self.room_for(memory_limit, plays, (self.fresh.len(), longest));
        if room.is_ok() && self.record_lists(plays, num_rows) {
            return true;
        }
        self.let_go(room.err() == Some(PoolRoom::Full));
        false
    }

    fn record_lists(&mut self, plays: &[movegen::ValuedMove], num_rows: usize) -> bool {
        let Self {
            places,
            moves,
            known,
            keys,
            contents,
            lists,
            needs,
            ends,
            entries,
            slot_keys,
            slot_lists,
            fresh,
            by_slot,
            starts,
            hashes,
            sorting,
            ..
        } = self;
        by_slot.clear();
        hashes.clear();
        sorting.clear();
        if moves.try_reserve(plays.len()).is_err()
            || needs.try_reserve(plays.len()).is_err()
            || ends.try_reserve(plays.len()).is_err()
            || entries.try_reserve(plays.len()).is_err()
            || lists.try_reserve(fresh.len()).is_err()
            || by_slot.try_reserve(plays.len()).is_err()
            || hashes.try_reserve(plays.len()).is_err()
            || sorting.try_reserve(plays.len()).is_err()
        {
            return false;
        }
        known.resize_with(slot_lists.len(), KeyTable::default);
        let slot_of = |play: &movegen::Play| match play {
            movegen::Play::Place { down, lane, .. } => {
                Some(if *down { num_rows } else { 0 } + *lane as usize)
            }
            movegen::Play::Exchange { .. } => None,
        };
        // the moves slot by slot, each slot's as walked: counted, then each put
        // before the next one of its slot, last first
        starts.clear();
        starts.resize(slot_lists.len() + 1, 0);
        for valued in plays.iter() {
            if let Some(slot) = slot_of(&valued.play) {
                starts[slot + 1] += 1;
            }
        }
        for slot in 0..slot_lists.len() {
            starts[slot + 1] += starts[slot];
        }
        let placing = starts[slot_lists.len()] as usize;
        by_slot.clear();
        by_slot.resize(placing, 0);
        for (i, valued) in plays.iter().enumerate().rev() {
            if let Some(slot) = slot_of(&valued.play) {
                starts[slot + 1] -= 1;
                by_slot[starts[slot + 1] as usize] = i as u32;
            }
        }
        for &slot in fresh.iter() {
            let slot = slot as usize;
            let begin = starts[slot + 1] as usize;
            let end = starts.get(slot + 2).map_or(placing, |&end| end as usize);
            hashes.clear();
            let mut key = FNV128_BASIS;
            for &i in &by_slot[begin..end] {
                let hash = mix_move(FNV128_BASIS, &plays[i as usize].play);
                hashes.push(hash);
                key = (key ^ hash).wrapping_mul(FNV128_PRIME);
            }
            let list = match contents.get(key) {
                Some(list) => list,
                None => {
                    sorting.clear();
                    for (k, &i) in by_slot[begin..end].iter().enumerate() {
                        if let movegen::Play::Place { word, .. } = &plays[i as usize].play {
                            let need: Tiles = (word.iter())
                                .filter(|&&tile| tile != 0)
                                .map(|&tile| 1 << (4 * u32::from(places[held_tile(tile) as usize])))
                                .sum();
                            sorting.push((need, k as u32));
                        }
                    }
                    sorting.sort_unstable();
                    let (first_group, first_entry) = (needs.len(), entries.len());
                    for &(need, k) in sorting.iter() {
                        if needs.len() == first_group || needs[needs.len() - 1] != need {
                            if needs.len() > first_group {
                                ends.push(entries.len() as u32);
                            }
                            needs.push(need);
                        }
                        let hash = hashes[k as usize];
                        let id = match known[slot].get(hash) {
                            Some(id) => id,
                            None => {
                                let id = moves.len() as u32;
                                let play = &plays[by_slot[begin + k as usize] as usize].play;
                                let placed = match play {
                                    movegen::Play::Place { word, .. } => {
                                        word.iter().filter(|&&tile| tile != 0).count()
                                    }
                                    movegen::Play::Exchange { .. } => 0,
                                };
                                moves.push((play.clone(), placed as u8));
                                if !known[slot].insert(hash, id) {
                                    return false;
                                }
                                id
                            }
                        };
                        entries.push(id);
                    }
                    if needs.len() > first_group {
                        ends.push(entries.len() as u32);
                    }
                    let list = lists.len() as u32;
                    lists.push([first_group as u32, needs.len() as u32, first_entry as u32]);
                    if !contents.insert(key, list) {
                        return false;
                    }
                    list
                }
            };
            if !keys.insert(slot_keys[slot], list) {
                return false;
            }
            slot_lists[slot] = list;
        }
        true
    }
}

// the sorted rack's slots a placement plays, as bits: of each tile's slots, the
// first ones, or with last the last ones. Two moves, one by first slots and one
// by last, fit the rack together when their bits do not overlap
#[inline(always)]
fn rack_places(rack: &[u8], word: &[u8], last: bool) -> u32 {
    let mut places = 0u32;
    for &tile in word.iter().filter(|&&t| t != 0) {
        let held = held_tile(tile);
        let first = rack.partition_point(|&t| t < held) as u32;
        let end = rack.partition_point(|&t| t <= held) as u32;
        let taken = (u64::from(places) & ((1u64 << (end - first)) - 1) << first).count_ones();
        places |= 1 << if last { end - 1 - taken } else { first + taken };
    }
    places
}

// each tile's slots in a sorted rack, as rack_places finds them by binary
// search: its first, and the one past its last (a tile the rack lacks has the
// slot it would go in for both), found once for every move of a rack
struct RackSlots {
    first: [u8; 128],
    end: [u8; 128],
}

impl RackSlots {
    #[inline(always)]
    fn of(rack: &[u8]) -> Self {
        let (mut first, mut end) = ([0u8; 128], [0u8; 128]);
        let mut at = 0;
        for tile in 0..128u8 {
            first[tile as usize] = at as u8;
            while at < rack.len() && rack[at] <= tile {
                at += 1;
            }
            end[tile as usize] = at as u8;
        }
        Self { first, end }
    }

    // rack_places of the rack these slots are of
    #[inline(always)]
    fn places(&self, word: &[u8], last: bool) -> u32 {
        let mut places = 0u32;
        for &tile in word.iter().filter(|&&t| t != 0) {
            let held = held_tile(tile) as usize;
            let (first, end) = (u32::from(self.first[held]), u32::from(self.end[held]));
            let taken = (u64::from(places) & ((1u64 << (end - first)) - 1) << first).count_ones();
            places |= 1 << if last { end - 1 - taken } else { first + taken };
        }
        places
    }
}

// a move's sort key: valuation (best first), then direction, lane, and first
// square as movegen::cmp_play has them, then the move's place in the list
#[inline(always)]
fn order_key(valuation: i32, down: bool, lane: i8, idx: i8, found: u32) -> u128 {
    let by_valuation = !(valuation.cast_unsigned() ^ (1 << (i32::BITS - 1)));
    (u128::from(by_valuation) << u64::BITS)
        | (u128::from(down) << (u32::BITS + 2 * u8::BITS))
        | (u128::from(lane.cast_unsigned()) << (u32::BITS + u8::BITS))
        | (u128::from(idx.cast_unsigned()) << u32::BITS)
        | u128::from(found)
}

#[inline(always)]
fn ordered_play_index(key: u128) -> usize {
    (key & u128::from(u32::MAX)) as usize
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
                movegen::cmp_play(
                    &list[ordered_play_index(a)].play,
                    &list[ordered_play_index(b)].play,
                )
            });
        }
        start = end;
    }
    for (place, &key) in (first_place..).zip(ordered.iter()) {
        list[ordered_play_index(key)].index = if place < PASS_PLAY as usize {
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
                &list[ordered_play_index(ordered[i])].play,
                &list[ordered_play_index(ordered[first])].play,
            )
            .is_lt()
        {
            first = i;
        }
    }
    ordered.swap(0, first);
    list[ordered_play_index(ordered[0])].index = 0;
}

// WorkBuffer contains reusable allocations.
// WorkBuffer can only be reused for the same game_config and kwg.
// (Refer to note at KurniaMoveGenerator.)
// The EndgameSolver holding it enforces this.
struct WorkBuffer<G: movegen::LaneBits, const JUMPS: bool, const ANY_SIDE: bool> {
    clock: Option<Clock>,             // the clock solves read, std's if none
    t0: Option<Started>,              // the solve's start, for its time limit
    dur_movegen: std::time::Duration, // for progress, timed only while something listens
    movegen: movegen::KurniaMoveGenerator<G, JUMPS, ANY_SIDE>,
    run_letter_bits: Option<std::sync::Arc<[u64]>>,
    board_tiles: Vec<u8>,  // the position searched, made and unmade in place
    racks: [Vec<u8>; 2],   // its racks, each sorted
    rack_scores: [i32; 2], // what its racks' tiles score
    key: u64,              // its board's and racks' keys
    lists: Vec<Vec<Candidate>>, // the moves generated at each ply from the root
    orders: Vec<Vec<u128>>, // each ply's sort keys (order_key)
    zones: Vec<(u128, u128)>, // the rows and columns each ply's move can change
    spent: Vec<u32>,       // the rack places (last slots) of each ply's move
    keeps: Vec<bool>,      // whether each ply's list has rack places (uses)
    generated: u64,        // the lists generated, each list's serial
    serials: Vec<u64>,     // the serial of each ply's list
    siblings: Vec<Siblings>, // each ply's moves its parent's replies so far share
    singles: Vec<(u64, Vec<u32>)>, // each list's one-tile moves by place, and its serial
    outs: Vec<OutReplies>, // each ply's plays out by the opponent
    out_skips: u64,        // the moves skipped so (OutReplies), over the solver's life
    out_checks: u32,       // the skips a debug build checked by a solve of the reply
    lane_pool: LanePool,
    pool_walker: Option<(u32, movegen::KurniaMoveGenerator<G, JUMPS, ANY_SIDE>)>, // LanePool's walks, with its leaves' table bound, kept from solve to solve
    next_deal: Deal, // where pool_lanes writes a search's deal, kept for the next
    #[cfg(test)]
    copied: usize, // the lanes copied from an earlier reply's list
    table: Table,
    spare_table: Option<Box<Table>>, // the box the last table given back came in
    can_grow: bool, // the memory limit has not yet refused the table more room this solve
    max_depth: i8,  // this pass's depth, for progress
    finished: i8,   // the deepest pass this solve finished, for the principal variation
    depth_cap: i8,  // how deep it may search: to the end, or a deepening bag search's pass depth
    time_limit: Option<std::time::Duration>,
    state_limit: Option<usize>,
    memory_limit: Option<MemoryLimit>,
    held_kept: usize,   // what a generated play holds beyond its list, for HeldBytes
    check_limits: bool, // from the second pass on, when a limit is set
    nodes: u64,         // this pass's nodes, to space the looks at the clock
    searched: u64,      // this solve's nodes, for progress
    cut_short: bool,    // a limit stopped this pass
    movegen_cut: bool,
    memory_cut: bool,    // the memory limit, not time nor states, stopped this solve
    in_bag: Option<u64>, // inside a search with a tile in the bag, its endgames so far
    in_bag_moves: Option<(usize, usize)>, // and its root's moves valued, of all
    at_root: bool,       // the next node searched is the root of a pass
    pass_best: u16,      // the root's best move in the last finished root search
    root_best: u16,      // the root's best move in the deepest finished pass
    proven: Option<(u16, equity::Equity)>, // a move this pass found worth more than its first, and its value
    answer: Option<(u16, equity::Equity)>, // the move and value a solve stopped partway answers with
    frames: Vec<Frame>,                    // the search's stack, root first
    pending: Option<(Enter, bool)>, // the node a limit stopped the search at, and whether the root
    resumable: bool,                // a timed solve's pass, which a limit stops without unwinding
    stopped: Option<(u64, u8, i8, equity::Equity, equity::Equity)>, // which pass the frames are, when one stopped
    resumed: u64,            // the passes this solver went on with from a step before
    line: Option<Box<Line>>, // the line collect_pv read last
    cadence: u64,            // the nodes between looks at the clock, 1 to LIMIT_CADENCE
    next_look: u64,          // the nodes this pass searches before the next look
    last_look: std::time::Duration, // the time at the last look, from the solve's start
    threads: usize,          // the solve's threads, one lane each (1: the plain search)
    verdicts: Vec<Option<(equity::Equity, bool)>>, // by split node's order: a bound on a failing move
    helper_tables: Vec<Option<KeptTable>>,         // each lane's table, kept from pass to pass
}

impl<G: movegen::LaneBits, const JUMPS: bool, const ANY_SIDE: bool> WorkBuffer<G, JUMPS, ANY_SIDE> {
    fn new(
        movegen: movegen::KurniaMoveGenerator<G, JUMPS, ANY_SIDE>,
        run_letter_bits: Option<std::sync::Arc<[u64]>>,
    ) -> Self {
        let held_kept = 2 * (std::mem::size_of::<Candidate>() + std::mem::size_of::<u128>())
            + movegen.longest_lane();
        Self {
            clock: None,
            t0: None,
            dur_movegen: Default::default(),
            movegen,
            run_letter_bits,
            board_tiles: Vec::new(),
            racks: [Vec::new(), Vec::new()],
            rack_scores: [0, 0],
            key: 0,
            lists: Vec::new(),
            orders: Vec::new(),
            zones: Vec::new(),
            spent: Vec::new(),
            keeps: Vec::new(),
            generated: 0,
            serials: Vec::new(),
            siblings: Vec::new(),
            singles: Vec::new(),
            outs: Vec::new(),
            out_skips: 0,
            out_checks: 0,
            lane_pool: LanePool::new(),
            pool_walker: None,
            next_deal: Deal::default(),
            #[cfg(test)]
            copied: 0,
            table: Table::new(),
            spare_table: None,
            can_grow: true,
            max_depth: 0,
            finished: 0,
            depth_cap: COMPLETE,
            time_limit: None,
            state_limit: None,
            memory_limit: None,
            held_kept,
            check_limits: false,
            nodes: 0,
            searched: 0,
            cut_short: false,
            movegen_cut: false,
            memory_cut: false,
            in_bag: None,
            in_bag_moves: None,
            at_root: false,
            pass_best: NO_PLAY,
            root_best: NO_PLAY,
            proven: None,
            answer: None,
            frames: Vec::new(),
            pending: None,
            resumable: false,
            stopped: None,
            resumed: 0,
            line: None,
            cadence: 1,
            next_look: 1,
            last_look: std::time::Duration::ZERO,
            threads: 1,
            verdicts: Vec::new(),
            helper_tables: Vec::new(),
        }
    }

    // the buffer as new made it, for a solver kept from one bag search to the
    // next: what it holds over a solver's life starts again, and what is made
    // and remade in place (the generator, the board, the racks, and the lists,
    // each list's serial past every earlier one's) keeps its memory. The
    // walker of the lane pool is let go, as a solve's memory limit sizes it
    fn renew(&mut self) {
        let Self {
            clock,
            t0,
            dur_movegen,
            movegen: _,
            run_letter_bits: _,
            board_tiles: _,
            racks: _,
            rack_scores,
            key,
            lists: _,
            orders: _,
            zones: _,
            spent: _,
            keeps: _,
            generated: _,
            serials: _,
            siblings: _,
            singles: _,
            outs: _,
            out_skips,
            out_checks,
            lane_pool: _,
            pool_walker: _,
            table: _,
            spare_table: _,
            can_grow,
            max_depth,
            finished,
            depth_cap,
            time_limit,
            state_limit,
            memory_limit,
            check_limits,
            nodes,
            searched,
            cut_short,
            movegen_cut,
            memory_cut,
            in_bag,
            in_bag_moves,
            at_root,
            pass_best,
            root_best,
            proven,
            answer,
            frames,
            pending,
            resumable,
            stopped,
            resumed,
            line,
            cadence,
            next_look,
            last_look,
            threads,
            verdicts,
            helper_tables,
            ..
        } = self;
        *clock = None;
        *t0 = None;
        *dur_movegen = Default::default();
        *rack_scores = [0, 0];
        *key = 0;
        *out_skips = 0;
        *out_checks = 0;
        *can_grow = true;
        *max_depth = 0;
        *finished = 0;
        *depth_cap = COMPLETE;
        *time_limit = None;
        *state_limit = None;
        *memory_limit = None;
        *check_limits = false;
        *nodes = 0;
        *searched = 0;
        *cut_short = false;
        *movegen_cut = false;
        *memory_cut = false;
        *in_bag = None;
        *in_bag_moves = None;
        *at_root = false;
        *pass_best = NO_PLAY;
        *root_best = NO_PLAY;
        *proven = None;
        *answer = None;
        frames.clear();
        *pending = None;
        *resumable = false;
        *stopped = None;
        *resumed = 0;
        *line = None;
        *cadence = 1;
        *next_look = 1;
        *last_look = std::time::Duration::ZERO;
        *threads = 1;
        verdicts.clear();
        helper_tables.clear();
    }

    // store a value, growing the table once three quarters of it holds
    // positions: to twice its size, or as big as the memory limit allows while
    // the old table is still held, at once while it is small, else a piece at
    // each store. Once the limit or the allocator refuses, the table keeps its
    // size and stores replace other positions
    #[inline(always)]
    fn store(&mut self, key: u64, entry: Entry) {
        if self.table.growing() {
            self.table.advance();
        }
        if self.table.store(key, entry)
            && self.can_grow
            && !self.table.growing()
            && self.table.held >= 3 * self.table.buckets.len()
        {
            let now = self.table.buckets.len();
            let mut buckets = now.saturating_mul(2);
            let bucket = std::mem::size_of::<[u64; 8]>();
            if let Some(limit) = self.memory_limit {
                buckets = buckets.min(limit.room() / bucket);
            }
            let room = (buckets > now)
                .then(|| reserve_in(self.memory_limit.as_ref(), buckets.saturating_mul(bucket)))
                .flatten();
            self.can_grow = room.is_some() && self.table.grow(buckets);
            drop(room);
            if buckets <= GROW_AT_ONCE {
                self.table.finish();
            }
        }
    }

    #[inline(always)]
    fn begin_list(&mut self, ply: usize) {
        if self.lists.len() <= ply {
            self.lists.resize_with(ply + 1, Vec::new);
            self.orders.resize_with(ply + 1, Vec::new);
            self.zones.resize(ply + 1, (0, 0));
            self.spent.resize(ply + 1, 0);
            self.keeps.resize(ply + 1, false);
            self.serials.resize(ply + 1, 0);
            self.siblings.resize_with(ply + 1, Siblings::default);
        }
        self.generated += 1;
        self.serials[ply] = self.generated;
        self.movegen_cut = false;
    }

    #[inline(always)]
    fn limit_memory(&mut self, memory: Option<MemoryLimit>) {
        self.memory_limit = memory;
        self.movegen.begin_within(memory);
        if let Some((_, walker)) = &mut self.pool_walker {
            walker.begin_within(memory);
        }
    }

    // whether the memory limit has passed, the buckets a growing table leaves
    // not counted
    #[inline(always)]
    fn memory_passed(&self) -> bool {
        self.memory_limit
            .is_some_and(|limit| limit.passed(self.table.leaving()))
    }

    // whether the time or the memory limit has passed, looked at once the pass
    // has searched next_look nodes; how many nodes on the next look comes is
    // set by how long the nodes since the last took (LOOK_EVERY)
    fn time_or_memory_passed(&mut self) -> bool {
        let mut cadence = LIMIT_CADENCE;
        if let (Some(limit), Some(t0)) = (self.time_limit, self.t0) {
            let now = t0.elapsed();
            if now >= limit {
                return true;
            }
            let took = now.saturating_sub(self.last_look);
            self.last_look = now;
            if took > LOOK_EVERY {
                self.cadence = (self.cadence / 2).max(1);
            } else {
                self.cadence = (self.cadence * 2).min(LIMIT_CADENCE);
            }
            cadence = self.cadence;
        }
        self.next_look = self.nodes + cadence;
        self.memory_cut = self.memory_passed();
        self.memory_cut
    }
}

// past any limit a solve stops, and answers from the deepest search that
// finished
pub struct Limits {
    pub time: Option<std::time::Duration>,
    pub states: Option<usize>, // a solve stops once the table holds this many positions
    pub memory: Option<MemoryLimit>,
    pub clock: Option<Clock>, // std's if none; a wasm32 page must give one
}

impl Limits {
    pub const NONE: Self = Self {
        time: None,
        states: None,
        memory: None,
        clock: None,
    };
}

// n of a value for each move and split of a bag search, n none if it overflowed
// counting: an error, not a failed allocation, when the program cannot hold
// them (a bag of many tiles can have more splits than the memory)
fn cells_of<T: Clone>(n: Option<usize>, value: T) -> error::Returns<Vec<T>> {
    let refused = || -> error::BoxAnyError {
        error::new(format!(
            "a bag search needs {} cells for its moves and splits, more than it can hold",
            n.map_or("too many".to_string(), |n| n.to_string())
        ))
        .into()
    };
    let n = n.ok_or_else(refused)?;
    let mut cells = Vec::new();
    cells.try_reserve_exact(n).map_err(|_| refused())?;
    cells.resize(n, value);
    Ok(cells)
}

#[derive(Clone, Copy)] // each endgame of a search with a tile in the bag takes a copy
pub struct MemoryLimit {
    pub bytes: usize,
    pub held: fn() -> usize,
    pub reserved: Option<&'static std::sync::atomic::AtomicUsize>,
}

impl MemoryLimit {
    // whether the program holds its bytes, not counting leaving: the buckets a
    // growing table leaves, freed as the growing ends. The growing took its
    // room with them held, so a solve stops as if the table had grown at once
    #[inline(always)]
    fn passed(&self, leaving: usize) -> bool {
        (self.held)().saturating_sub(leaving) >= self.bytes
    }

    #[inline(always)]
    fn room(&self) -> usize {
        let reserved = (self.reserved).map_or(0, |reserved| {
            reserved.load(std::sync::atomic::Ordering::Acquire)
        });
        (self.bytes.saturating_sub((self.held)()))
            .saturating_sub(reserved)
            .saturating_sub(1)
    }

    // whether the program may take on this many bytes and still hold fewer
    // than its bytes
    #[inline(always)]
    pub fn has_room(&self, bytes: usize) -> bool {
        bytes <= self.room()
    }

    #[inline(always)]
    pub fn reserve(&self, bytes: usize) -> Option<Reserved> {
        if bytes == 0 {
            return Some(Reserved::NONE);
        }
        match self.reserved {
            None => self.has_room(bytes).then_some(Reserved::NONE),
            Some(reserved) => self.reserve_shared(reserved, bytes),
        }
    }

    // reserve's room taken in the shared count first and held read after, so a
    // growth that gave its room back before that already held what it took,
    // and one that gives it back later was counted in it
    #[inline(never)]
    fn reserve_shared(
        &self,
        reserved: &'static std::sync::atomic::AtomicUsize,
        bytes: usize,
    ) -> Option<Reserved> {
        use std::sync::atomic::Ordering;
        let before = reserved
            .try_update(Ordering::AcqRel, Ordering::Acquire, |taken| {
                taken.checked_add(bytes)
            })
            .ok()?;
        let taken = Reserved {
            reserved: Some(reserved),
            bytes,
        };
        ((self.held)().saturating_add(before + bytes) < self.bytes).then_some(taken)
    }
}

#[must_use]
pub struct Reserved {
    reserved: Option<&'static std::sync::atomic::AtomicUsize>,
    bytes: usize,
}

impl Reserved {
    // room taken from no shared count, which gives nothing back
    pub const NONE: Self = Self {
        reserved: None,
        bytes: 0,
    };

    #[inline(always)]
    pub fn keep(&mut self, kept: usize) {
        if let Some(reserved) = self.reserved {
            let given = self.bytes.saturating_sub(kept);
            reserved.fetch_sub(given, std::sync::atomic::Ordering::Release);
            self.bytes -= given;
        }
    }
}

impl Drop for Reserved {
    #[inline(always)]
    fn drop(&mut self) {
        if let Some(reserved) = self.reserved {
            reserved.fetch_sub(self.bytes, std::sync::atomic::Ordering::Release);
        }
    }
}

// a growth a memory limit refused: taken, it would have had the program hold
// its bytes or more
#[derive(Debug)]
pub struct Refused;

impl std::fmt::Display for Refused {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("the memory limit refused the room")
    }
}

impl std::error::Error for Refused {}

#[inline(always)]
pub fn reserve_in(memory: Option<&MemoryLimit>, bytes: usize) -> Option<Reserved> {
    match memory {
        Some(memory) => memory.reserve(bytes),
        None => Some(Reserved::NONE),
    }
}

// room in list for more items, its buffer grown when it has not: with no limit
// as std grows one, else only when the limit has room for the new buffer's
// bytes as well as the old one's, which the program holds while they move; to
// twice its capacity, or less where the limit has not room for that, never
// less than fits them. Refused, the list is as it was
#[inline(always)]
pub fn room_in<T>(
    memory: Option<&MemoryLimit>,
    list: &mut Vec<T>,
    more: usize,
) -> Result<(), Refused> {
    if list.capacity() - list.len() >= more {
        Ok(())
    } else {
        grow_in(memory, list, more)
    }
}

#[cold]
#[inline(never)]
fn grow_in<T>(memory: Option<&MemoryLimit>, list: &mut Vec<T>, more: usize) -> Result<(), Refused> {
    let Some(memory) = memory else {
        list.reserve(more);
        return Ok(());
    };
    let needed = list.len().checked_add(more).ok_or(Refused)?;
    let size = std::mem::size_of::<T>().max(1);
    let fits = memory.room() / size;
    if needed > fits {
        return Err(Refused);
    }
    let capacity = needed.max(list.capacity().saturating_mul(2).min(fits));
    let _room = memory.reserve(capacity * size).ok_or(Refused)?;
    list.try_reserve_exact(capacity - list.len())
        .map_err(|_| Refused)
}

#[derive(Clone, Default)]
pub struct Room {
    pub memory: Option<MemoryLimit>, // none: no limit
    refused: std::cell::Cell<bool>,
}

impl Room {
    #[inline(always)]
    pub fn of(memory: Option<MemoryLimit>) -> Self {
        Self {
            memory,
            refused: std::cell::Cell::new(false),
        }
    }

    #[inline(always)]
    pub fn refused(&self) -> bool {
        self.refused.get()
    }

    #[inline(always)]
    pub fn take(&self, bytes: usize) -> Option<Reserved> {
        if self.refused.get() {
            return None;
        }
        let Some(memory) = self.memory else {
            return Some(Reserved::NONE);
        };
        let taken = memory.reserve(bytes);
        self.refused.set(taken.is_none());
        taken
    }
}

pub trait Grow<T> {
    fn room_for(&mut self, room: &Room, more: usize) -> bool;
    fn push_in(&mut self, room: &Room, item: T);
    fn extend_in(&mut self, room: &Room, items: &[T])
    where
        T: Clone;
    fn resize_in(&mut self, room: &Room, len: usize, item: T)
    where
        T: Clone;
}

impl<T> Grow<T> for Vec<T> {
    #[inline(always)]
    fn room_for(&mut self, room: &Room, more: usize) -> bool {
        self.capacity() - self.len() >= more || grow_for(room, self, more)
    }

    #[inline(always)]
    fn push_in(&mut self, room: &Room, item: T) {
        if self.len() < self.capacity() || grow_for(room, self, 1) {
            self.push(item);
        }
    }

    #[inline(always)]
    fn extend_in(&mut self, room: &Room, items: &[T])
    where
        T: Clone,
    {
        if self.room_for(room, items.len()) {
            self.extend_from_slice(items);
        }
    }

    #[inline(always)]
    fn resize_in(&mut self, room: &Room, len: usize, item: T)
    where
        T: Clone,
    {
        if self.room_for(room, len.saturating_sub(self.len())) {
            self.resize(len, item);
        }
    }
}

#[cold]
fn grow_for<T>(room: &Room, list: &mut Vec<T>, more: usize) -> bool {
    if !room.refused.get() && grow_in(room.memory.as_ref(), list, more).is_err() {
        room.refused.set(true);
    }
    !room.refused.get()
}

// a clock: the time since any fixed start, never going back. A solve reads
// std's (Instant) unless its solver is given one, as a wasm32 page must give
// its own: std's has no clock there and panics when read
pub type Clock = fn() -> std::time::Duration;

// when a solve started, on std's clock or on a given one
#[derive(Clone, Copy)]
enum Started {
    Std(std::time::Instant),
    Given(Clock, std::time::Duration),
}

impl Started {
    #[inline(always)]
    fn now(clock: Option<Clock>) -> Self {
        match clock {
            None => Self::Std(std::time::Instant::now()),
            Some(clock) => Self::Given(clock, clock()),
        }
    }

    #[inline(always)]
    fn elapsed(&self) -> std::time::Duration {
        match *self {
            Self::Std(t0) => t0.elapsed(),
            Self::Given(clock, t0) => clock().saturating_sub(t0),
        }
    }
}

// a search with a tile in the bag solves many endgames as one solve: its time
// limit is one deadline for them all, on its clock, and its state limit applies
// to the table they share (the solver's own, lent with what its earlier solves
// found); the endgames take turns holding its one listener. Under a time or
// state limit the search deepens: each pass searches every endgame to the
// pass's depth, one ply deeper than the pass before, so a stopped search weighs
// moves whose endgames were searched alike
struct BagLimits {
    time: Option<std::time::Duration>,
    states: Option<usize>,
    memory: Option<MemoryLimit>,
    t0: Started,
    dur_movegen: std::cell::Cell<std::time::Duration>,
    endgames: std::cell::Cell<u64>,
    nodes: std::cell::Cell<u64>, // the nodes its endgames searched
    moves: std::cell::Cell<Option<(usize, usize)>>, // the root's moves the pass valued, of all
    progress: std::cell::Cell<Option<ProgressFn>>,
    depth: std::cell::Cell<i8>, // the pass's depth, or the end for a search of one pass
    may_stop: std::cell::Cell<bool>, // a limit may stop the search: it has an answer
    stopped: std::cell::Cell<bool>, // a limit stopped it
    refused: std::cell::Cell<bool>, // the memory limit stopped it, before it grew past it
    memory_stopped: std::cell::Cell<bool>, // the limit that stopped it was the memory's
    estimates: bool,
    exact_pass: bool,
}

impl BagLimits {
    // its clock (std's if none is given) starts now
    #[inline(always)]
    fn new(limits: &Limits, progress: Option<ProgressFn>) -> Self {
        Self {
            time: limits.time,
            states: limits.states,
            memory: limits.memory,
            t0: Started::now(limits.clock),
            dur_movegen: Default::default(),
            endgames: Default::default(),
            nodes: Default::default(),
            moves: Default::default(),
            progress: std::cell::Cell::new(progress),
            depth: std::cell::Cell::new(COMPLETE),
            may_stop: std::cell::Cell::new(false),
            stopped: std::cell::Cell::new(false),
            refused: std::cell::Cell::new(false),
            memory_stopped: std::cell::Cell::new(false),
            estimates: false,
            exact_pass: false,
        }
    }

    // whether the search deepens a pass at a time, as under a time or state
    // limit, or with its passes estimating; else it is one pass, each endgame
    // searched to its end
    #[inline(always)]
    fn deepens(&self) -> bool {
        !self.exact_pass && (self.time.is_some() || self.states.is_some() || self.estimates)
    }

    // what another thread of the same search needs to keep to its limits
    #[inline(always)]
    fn on_one_clock(&self) -> OnOneClock {
        OnOneClock {
            time: self.time,
            states: self.states,
            memory: self.memory,
            t0: self.t0,
            depth: self.depth.get(),
            estimates: self.estimates,
            exact_pass: self.exact_pass,
        }
    }

    // the limits of another thread of a search: the same limits on the same
    // clock, at the same pass, with no listener
    #[inline(always)]
    fn on_the_clock(clock: OnOneClock) -> Self {
        Self {
            time: clock.time,
            states: clock.states,
            memory: clock.memory,
            t0: clock.t0,
            dur_movegen: Default::default(),
            endgames: Default::default(),
            nodes: Default::default(),
            moves: Default::default(),
            progress: std::cell::Cell::new(None),
            depth: std::cell::Cell::new(clock.depth),
            may_stop: std::cell::Cell::new(false),
            stopped: std::cell::Cell::new(false),
            refused: std::cell::Cell::new(false),
            memory_stopped: std::cell::Cell::new(false),
            estimates: clock.estimates,
            exact_pass: clock.exact_pass,
        }
    }

    #[inline(always)]
    fn reserve(&self, bytes: usize) -> error::Returns<Reserved> {
        match self.memory {
            None => Ok(Reserved::NONE),
            Some(memory) => memory.reserve(bytes).ok_or_else(|| self.stop_refused()),
        }
    }

    #[inline(always)]
    fn room<T>(&self, list: &mut Vec<T>, more: usize) -> error::Returns<()> {
        if list.capacity() - list.len() >= more {
            Ok(())
        } else {
            self.grow(list, more)
        }
    }

    #[cold]
    fn grow<T>(&self, list: &mut Vec<T>, more: usize) -> error::Returns<()> {
        room_in(self.memory.as_ref(), list, more).map_err(|_| self.stop_refused())
    }

    // the search stopped by the memory limit before it took what passes it
    // (stopped and refused set): the error it stops with
    #[inline(never)]
    fn stop_refused(&self) -> error::BoxAnyError {
        self.stopped.set(true);
        self.refused.set(true);
        Box::new(MemoryRefused)
    }

    // whether the memory limit cut the search short
    #[inline(always)]
    fn out_of_memory(&self) -> bool {
        self.refused.get() || self.memory_stopped.get()
    }

    // whether any limit has passed, on the table the endgames share; the
    // memory is looked at last, and noted as the one past only when no other
    // is
    #[inline(always)]
    fn other_passed(&self, table: &Table) -> bool {
        self.time.is_some_and(|time| self.t0.elapsed() >= time)
            || self.states.is_some_and(|states| table.held >= states)
    }

    #[inline(always)]
    fn passed(&self, table: &Table) -> bool {
        if self.other_passed(table) {
            return true;
        }
        let memory = (self.memory).is_some_and(|memory| memory.passed(table.leaving()));
        self.memory_stopped.set(self.memory_stopped.get() | memory);
        memory
    }
}

// the limits, the clock they run on, and the pass's depth, as a search with
// tiles in the bag hands them to each thread it runs on (BagLimits holds them
// in cells, which one thread alone may use)
#[derive(Clone, Copy)]
struct OnOneClock {
    time: Option<std::time::Duration>,
    states: Option<usize>,
    memory: Option<MemoryLimit>,
    t0: Started,
    depth: i8,
    estimates: bool,
    exact_pass: bool,
}

// an average of values in millipoints, such as the peg's margin, kept in f32,
// which holds whole ones exactly, and rounded to the nearest
#[cfg(test)]
#[inline(always)]
fn to_equity(v: f32) -> equity::Equity {
    debug_assert!(v.abs() <= (1u32 << f32::MANTISSA_DIGITS) as f32, "{v}");
    equity::Equity::new(v.round() as i32)
}

// a bound in f64 as an endgame's beta: a whole value is below it when it is
// below the bound rounded up
#[inline(always)]
fn beta_equity(beta: f64) -> error::Returns<equity::Equity> {
    if beta == f64::INFINITY {
        Ok(equity::Equity::INFINITY)
    } else if beta == f64::NEG_INFINITY {
        Ok(equity::Equity::NEG_INFINITY)
    } else {
        let beta = beta.ceil();
        if !(beta > f64::from(i32::MIN) && beta < f64::from(i32::MAX)) {
            crate::return_error!(format!("endgame beta is out of range: {beta}"));
        }
        Ok(equity::Equity::new(beta as i32))
    }
}

fn checked_expected_margin(margin: f64) -> error::Returns<equity::Equity> {
    let rounded = margin.round();
    if !(f64::from(i32::MIN)..=f64::from(i32::MAX)).contains(&rounded) {
        crate::return_error!(format!("expected margin is out of range: {margin}"));
    }
    Ok(equity::Equity::new(rounded as i32))
}

// a solve's answer: exact, or the value of the deepest search that finished
// before a limit stopped it, or that of a move the stopped search found worth
// more (partway is that search's depth)
// a first move of the position to solve and its value (value_first_moves): none
// when a limit stopped its first depth; exact when no line was cut short by the
// depth
pub struct FirstMove {
    pub play: movegen::Play,
    pub value: Option<equity::Equity>,
    pub exact: bool,
}

pub struct Solved {
    pub value: Option<equity::Equity>, // none when no move was proved
    pub depth: i8,
    pub exact: bool,
    pub partway: Option<i8>, // the depth stopped in, when its proved move answers
    pub out_of_memory: bool,
    pub out_of_memory_in_moves: bool,
}

// how far a solve has got, sent to its listener at every node, and at each
// finished depth with that depth's value; with a tile in the bag, at every node
// alone. The listener keeps the time, on its own clock
pub struct Progress {
    pub endgames: Option<u64>, // with a tile in the bag, the endgames searched so far
    pub moves: Option<(usize, usize)>, // and the root's moves its pass has valued, of all
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
        if let Some((valued, of)) = p.moves {
            write!(f, ", {valued} of {of} moves valued")?;
        }
        write!(
            f,
            ": after {:?} ({:?} on movegen), {} nodes, the table holds {} positions of {}",
            self.elapsed, p.movegen, p.nodes, p.held, p.room,
        )
    }
}

type ProgressFn = Box<dyn FnMut(&Progress) -> error::Returns<()>>;

// the raw moves of rack on the rows and columns in the masks, as the search
// generates them: with no exchange made, a pass a candidate, and on the default
// arm each walk along a run stopping at its last candidate. A solver made lean
// generates by the lean arm alone, so that a build that runs only that arm (the
// wasm) need not carry the other.
#[inline(always)]
fn gen_raw_lanes<
    const LEAN: bool,
    N: kwg::Node,
    L: kwg::Node,
    G: movegen::LaneBits,
    const JUMPS: bool,
    const ANY_SIDE: bool,
>(
    generator: &mut movegen::KurniaMoveGenerator<G, JUMPS, ANY_SIDE>,
    run_letter_bits: Option<&[u64]>,
    board_snapshot: &movegen::BoardSnapshot<'_, N, L>,
    rack: &[u8],
    rows: u128,
    cols: u128,
    limit: Option<movegen::HeldBytes>,
) -> error::Returns<bool> {
    match (run_letter_bits, limit) {
        (Some(run_letter_bits), None) => {
            generator.gen_moves_raw_lanes_unsorted_by_runs(
                run_letter_bits,
                board_snapshot,
                rack,
                rows,
                cols,
            )?;
            Ok(false)
        }
        (Some(run_letter_bits), Some(limit)) => generator
            .gen_moves_raw_lanes_unsorted_by_runs_within(
                run_letter_bits,
                board_snapshot,
                rack,
                rows,
                cols,
                limit,
            ),
        (None, None) => {
            if LEAN {
                generator.gen_moves_raw_lanes_unsorted_lean(
                    board_snapshot,
                    rack,
                    0, // TODO: use game_state to track this
                    movegen::PassPolicy::AsACandidate,
                    rows,
                    cols,
                )
            } else {
                generator.gen_moves_raw_lanes_unsorted(
                    board_snapshot,
                    rack,
                    0, // TODO: use game_state to track this
                    movegen::PassPolicy::AsACandidate,
                    rows,
                    cols,
                )
            }?;
            Ok(false)
        }
        (None, Some(limit)) => {
            generator.gen_moves_raw_lanes_unsorted_within(board_snapshot, rack, rows, cols, limit)
        }
    }
}

// the raw moves of rack, every lane; by the lean arm alone when lean, as
// gen_raw_lanes
#[inline(always)]
fn gen_raw_all<
    'a,
    const LEAN: bool,
    N: kwg::Node,
    L: kwg::Node,
    G: movegen::LaneBits,
    const JUMPS: bool,
    const ANY_SIDE: bool,
>(
    generator: &mut movegen::KurniaMoveGenerator<G, JUMPS, ANY_SIDE>,
    board_snapshot: &'a movegen::BoardSnapshot<'a, N, L>,
    rack: &'a [u8],
    num_exchanges_by_this_player: i16,
    pass_policy: movegen::PassPolicy,
) -> error::Returns<()> {
    if LEAN {
        generator.gen_moves_raw_all_unsorted_lean(
            board_snapshot,
            rack,
            num_exchanges_by_this_player,
            pass_policy,
        )
    } else {
        generator.gen_moves_raw_all_unsorted(
            board_snapshot,
            rack,
            num_exchanges_by_this_player,
            pass_policy,
        )
    }
}

// a solver's table kept apart from it (take_table), to be given back only to a
// solver of the game_config, kwg, and arm_tables it was made with, by address
pub struct KeptTable {
    table: Box<Table>,
    made_with: [usize; 3],
    stopped: Option<Box<Stopped>>, // a timed solve's pass a limit stopped, for the next step
    line: Option<Box<Line>>,       // the line its solver read last
    cadence: u64,                  // its solver's nodes between looks at the clock
}

impl KeptTable {
    // the table alone, for a solve of another position: a table's entries are
    // keyed by the whole position and hold values from it on, so they serve any
    // root, but a stopped pass and the line read last are the old root's
    pub fn for_another_root(self) -> Self {
        Self {
            stopped: None,
            line: None,
            ..self
        }
    }

    // the table for a solver of another graph of the same lexicon, each graph
    // pruned to the words its own board can still take: a value kept is of a
    // position holding the old board's tiles, and one the new search finds
    // holds the new board's too, so either graph keeps every word it can play
    pub fn for_another_graph<N: kwg::Node>(self, kwg: &kwg::Kwg<N>) -> Self {
        let [game_config, _, arm_tables] = self.made_with;
        Self {
            made_with: [game_config, std::ptr::from_ref(kwg).addr(), arm_tables],
            ..self.for_another_root()
        }
    }
}

// a helper thread's solver between its searches: all of it but what it
// borrows, its listener, and its own helpers, so that a thread can be handed
// it (EndgameSolver::into_helper, EndgameSolver::of_helper)
struct Helper<L: kwg::Node, G: movegen::LaneBits, const JUMPS: bool, const ANY_SIDE: bool> {
    klv: Box<klv::Klv<L>>,
    board_tiles: Vec<u8>,
    racks: [Vec<u8>; 2],
    work_buffer: WorkBuffer<G, JUMPS, ANY_SIDE>,
    kept_mg: Option<movegen::KurniaMoveGenerator<G, JUMPS, ANY_SIDE>>,
    kept_sub: Box<Option<KeptSolver<L, G, JUMPS, ANY_SIDE>>>,
}

// a solver kept apart from any (EndgameSolver::into_kept), to be made whole
// only by one of the game_config, kwg, and arm_tables it was made with, by
// address (EndgameSolver::with_kept)
pub struct KeptSolver<L: kwg::Node, G: movegen::LaneBits, const JUMPS: bool, const ANY_SIDE: bool> {
    own: Helper<L, G, JUMPS, ANY_SIDE>, // its bag searches' solver with it
    helpers: Vec<Option<Helper<L, G, JUMPS, ANY_SIDE>>>,
    made_with: [usize; 3],
}

// EndgameSolver is the main two-player endgame solver.
// EndgameSolver can only be reused for the same game_config and kwg.
// (Refer to note at WorkBuffer.)
// Its fields are private and never replaced, and its table moves only through
// swap_tables, which refuses a solver of another game or lexicon. A lean one
// generates by the lean arm alone (gen_raw_lanes).
pub struct EndgameSolver<
    'a,
    N: kwg::Node,
    L: kwg::Node,
    const LEAN: bool = false,
    G: movegen::LaneBits = u128,
    const JUMPS: bool = true,
    const ANY_SIDE: bool = true,
> {
    game_config: &'a game_config::GameConfig,
    kwg: &'a kwg::Kwg<N>,
    arm_tables: &'a anagrams::ArmTables,
    run_letters: kwg::RunLetters<'a>, // the graph's, for its sub-solvers
    klv: Box<klv::Klv<L>>,
    board_tiles: Vec<u8>,
    racks: [Vec<u8>; 2],
    scoreless: [u8; 2], // the passes and the zeros before the position to solve
    root_moves: Option<&'a RootMoves>,
    root_priority: &'a [movegen::Play],
    work_buffer: WorkBuffer<G, JUMPS, ANY_SIDE>,
    progress: Option<ProgressFn>, // its listener, apart from the buffer
    helpers: Vec<Option<Helper<L, G, JUMPS, ANY_SIDE>>>, // each lane's, kept from pass to pass
    kept_sub: Box<Option<KeptSolver<L, G, JUMPS, ANY_SIDE>>>, // bag searches', taken apart
    kept_mg: Option<movegen::KurniaMoveGenerator<G, JUMPS, ANY_SIDE>>, // and their generator
}

impl<'a, N: kwg::Node, L: kwg::Node, const LEAN: bool> EndgameSolver<'a, N, L, LEAN> {
    pub fn new(
        game_config: &'a game_config::GameConfig,
        kwg: &'a kwg::Kwg<N>,
        arm_tables: &'a anagrams::ArmTables,
    ) -> error::Returns<Self> {
        // the lean arm's tables hold the graph's run letters; else they are
        // found here, as this is made once for its graph
        let run_letters = match arm_tables.rack_lengths() {
            Some(rack_lengths) => rack_lengths.run_letters().of(kwg)?,
            None => kwg.run_letters(),
        };
        Self::with_generator(
            game_config,
            kwg,
            arm_tables,
            run_letters,
            movegen::KurniaMoveGenerator::new(game_config),
        )
    }
}

impl<
    'a,
    N: kwg::Node,
    L: kwg::Node,
    const LEAN: bool,
    G: movegen::LaneBits,
    const JUMPS: bool,
    const ANY_SIDE: bool,
> EndgameSolver<'a, N, L, LEAN, G, JUMPS, ANY_SIDE>
{
    // as new, generating by the caller's move generator (the wasm's, in u32),
    // with the run letters found where the graph loaded
    // (kwg::Kwg::run_letters);
    // the game config must have passed supports(Op::Endgame) at init, as this
    // is made for each solve and each of a search's sub-solves
    pub fn with_generator(
        game_config: &'a game_config::GameConfig,
        kwg: &'a kwg::Kwg<N>,
        arm_tables: &'a anagrams::ArmTables,
        run_letters: kwg::RunLetters<'a>,
        movegen: movegen::KurniaMoveGenerator<G, JUMPS, ANY_SIDE>,
    ) -> error::Returns<Self> {
        // the run letters, unless the tables are the lean arm's (rack lengths),
        // which reads no more than the word graph, and only for a graph whose
        // runs ascend, as a walk with them goes straight to a tile's node
        let run_letter_bits = (run_letters.of(kwg)?)
            .filter(|_| arm_tables.rack_lengths().is_none())
            .cloned();
        Ok(Self {
            game_config,
            kwg,
            arm_tables,
            run_letters,
            klv: Box::new(klv::Klv::<L>::empty()?),
            board_tiles: Vec::new(),
            racks: [Vec::new(), Vec::new()],
            scoreless: [0, 0],
            root_moves: None,
            root_priority: &[],
            work_buffer: WorkBuffer::new(movegen, run_letter_bits),
            progress: None,
            helpers: Vec::new(),
            kept_sub: Box::new(None),
            kept_mg: None,
        })
    }

    pub fn with_generator_within(
        game_config: &'a game_config::GameConfig,
        kwg: &'a kwg::Kwg<N>,
        arm_tables: &'a anagrams::ArmTables,
        run_letters: kwg::RunLetters<'a>,
        movegen: movegen::KurniaMoveGenerator<G, JUMPS, ANY_SIDE>,
        (like, memory): (&klv::Klv<L>, Option<MemoryLimit>),
    ) -> error::Returns<Option<Self>> {
        let rest = std::mem::size_of::<Option<KeptSolver<L, G, JUMPS, ANY_SIDE>>>()
            + std::mem::size_of::<klv::Klv<L>>()
            + 2 * like.held_bytes()
            + std::mem::size_of::<[u64; 8]>();
        let Some(_rest) = reserve_in(memory.as_ref(), rest) else {
            return Ok(None);
        };
        Self::with_generator(game_config, kwg, arm_tables, run_letters, movegen).map(Some)
    }

    // the solver as with_generator made it, for one kept from search to
    // search, what it holds of its generator and its lists kept
    // (WorkBuffer::renew)
    #[inline]
    fn renew(&mut self) {
        self.board_tiles.clear();
        self.racks[0].clear();
        self.racks[1].clear();
        self.scoreless = [0, 0];
        self.progress = None;
        self.work_buffer.renew();
    }

    // the solver as with_generator made it, for a caller that keeps it from
    // one solve to the next (a shell's, a batch's), with what it holds of its
    // generators, its lists, its bag searches' solver, and its threads' kept,
    // and its table as reserve_table makes one of this many bytes in a new
    // solver, emptied in place where it has that many buckets
    #[inline]
    pub fn renew_for_another_solve(&mut self, table_bytes: usize) {
        self.renew();
        (self.work_buffer.table).empty_to(table_bytes / std::mem::size_of::<[u64; 8]>());
    }

    // the solver a helper thread keeps between its searches, taken apart
    #[inline(always)]
    fn into_helper(self) -> Helper<L, G, JUMPS, ANY_SIDE> {
        Helper {
            klv: self.klv,
            board_tiles: self.board_tiles,
            racks: self.racks,
            work_buffer: self.work_buffer,
            kept_mg: self.kept_mg,
            kept_sub: self.kept_sub,
        }
    }

    // a helper thread's kept solver, made whole again with what it borrows and
    // renewed for its next search, or one just made within memory (none: no
    // limit), refused (MemoryRefused) where it has no room: its generator, its
    // kept sub-solver's box and its leaves' box, its leaves (empty, as like's,
    // made through a buffer no larger than they are), and its table's bucket;
    // like is none only where a helper is given
    #[inline]
    fn of_helper(
        game_config: &'a game_config::GameConfig,
        kwg: &'a kwg::Kwg<N>,
        arm_tables: &'a anagrams::ArmTables,
        run_letters: &kwg::RunLetters<'a>,
        helper: Option<Helper<L, G, JUMPS, ANY_SIDE>>,
        (like, memory): (Option<&klv::Klv<L>>, Option<MemoryLimit>),
    ) -> error::Returns<Self> {
        let Some(helper) = helper else {
            let Some(movegen) = movegen::KurniaMoveGenerator::new_within(game_config, memory)
            else {
                return Err(Box::new(MemoryRefused));
            };
            let like = like.ok_or("a solver made afresh needs leaves to be like")?;
            let made = Self::with_generator_within(
                game_config,
                kwg,
                arm_tables,
                run_letters.clone(),
                movegen,
                (like, memory),
            )?;
            return made.ok_or_else(|| Box::new(MemoryRefused) as error::BoxAnyError);
        };
        let mut own = Self {
            game_config,
            kwg,
            arm_tables,
            run_letters: run_letters.clone(),
            klv: helper.klv,
            board_tiles: helper.board_tiles,
            racks: helper.racks,
            scoreless: [0, 0],
            root_moves: None,
            root_priority: &[],
            work_buffer: helper.work_buffer,
            progress: None,
            helpers: Vec::new(),
            kept_sub: helper.kept_sub,
            kept_mg: helper.kept_mg,
        };
        own.renew();
        Ok(own)
    }

    // the solver taken apart from what it borrows, for a caller that keeps no
    // solver between solves (a wasm32 page keeps it in a handle, beside the
    // table it takes first with take_table), with its bag searches' solver and
    // its threads' kept too
    #[inline]
    pub fn into_kept(self) -> KeptSolver<L, G, JUMPS, ANY_SIDE> {
        KeptSolver {
            made_with: self.made_with(),
            helpers: self.helpers,
            own: Helper {
                klv: self.klv,
                board_tiles: self.board_tiles,
                racks: self.racks,
                work_buffer: self.work_buffer,
                kept_mg: self.kept_mg,
                kept_sub: self.kept_sub,
            },
        }
    }

    // a kept solver made whole again with what it borrows, as with_generator
    // makes one (renew), with the memory it holds kept; refused for any
    // game_config, kwg, or arm_tables but those it was made with, by address
    #[inline]
    pub fn with_kept(
        game_config: &'a game_config::GameConfig,
        kwg: &'a kwg::Kwg<N>,
        arm_tables: &'a anagrams::ArmTables,
        run_letters: kwg::RunLetters<'a>,
        kept: KeptSolver<L, G, JUMPS, ANY_SIDE>,
    ) -> error::Returns<Self> {
        let made_with = [
            std::ptr::from_ref(game_config).addr(),
            std::ptr::from_ref(kwg).addr(),
            std::ptr::from_ref(arm_tables).addr(),
        ];
        if kept.made_with != made_with {
            return Err("a kept solver is given back only to its own tables".into());
        }
        run_letters.of(kwg)?;
        let mut own = Self::of_helper(
            game_config,
            kwg,
            arm_tables,
            &run_letters,
            Some(kept.own),
            (None, None),
        )?;
        own.helpers = kept.helpers;
        Ok(own)
    }

    #[inline]
    fn take_sub(&mut self, limits: &BagLimits) -> error::Returns<Self> {
        let kept = self.kept_sub.take();
        let (own, helpers) = match kept {
            Some(kept) => (Some(kept.own), kept.helpers),
            None => (None, Vec::new()),
        };
        let mut sub = Self::of_helper(
            self.game_config,
            self.kwg,
            self.arm_tables,
            &self.run_letters,
            own,
            (Some(&self.klv), limits.memory),
        )
        .map_err(|e| {
            if e.is::<MemoryRefused>() {
                limits.stop_refused()
            } else {
                e
            }
        })?;
        sub.helpers = helpers;
        Ok(sub)
    }

    // the solver take_sub gave, kept again in its box
    #[inline(always)]
    fn keep_sub(&mut self, sub: Self) {
        *self.kept_sub = Some(sub.into_kept());
    }

    #[inline(always)]
    fn take_mg(
        &mut self,
        limits: &BagLimits,
    ) -> error::Returns<movegen::KurniaMoveGenerator<G, JUMPS, ANY_SIDE>> {
        let mut mg = match self.kept_mg.take() {
            Some(mg) => mg,
            None => movegen::KurniaMoveGenerator::new_within(self.game_config, limits.memory)
                .ok_or_else(|| limits.stop_refused())?,
        };
        mg.begin_within(limits.memory);
        Ok(mg)
    }

    #[inline]
    fn take_both(
        &mut self,
        limits: &BagLimits,
    ) -> error::Returns<(Self, movegen::KurniaMoveGenerator<G, JUMPS, ANY_SIDE>)> {
        let sub = self.take_sub(limits)?;
        match self.take_mg(limits) {
            Ok(mg) => Ok((sub, mg)),
            Err(e) => {
                self.keep_sub(sub);
                Err(e)
            }
        }
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
        self.scoreless = scoreless_counts([passes, zeros]);
    }

    // select the first plays checked for this empty-bag position
    pub fn with_root_moves(mut self, root_moves: Option<&'a RootMoves>) -> Self {
        self.root_moves = root_moves;
        self
    }

    // name the candidates to value first in a timed PEG or first-move solve
    pub fn with_root_priority(mut self, first: &'a [movegen::Play]) -> Self {
        self.root_priority = first;
        self
    }

    #[inline(always)]
    fn root_key(&self, player_idx: u8, scoreless: Scoreless) -> u64 {
        self.node_key(player_idx, scoreless) ^ self.root_moves.map_or(0, |root| root.key)
    }

    #[inline(always)]
    fn allows_pass(&self, root: bool) -> bool {
        !root || self.root_moves.is_none_or(|moves| moves.pass)
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
            work_buffer.rack_scores[player] = alphabet.scaled_rack_score(rack).raw();
        }
        if !work_buffer.lane_pool.deal.pool.is_empty() {
            let dim = self.game_config.board_layout().dim();
            (work_buffer.lane_pool).reset_slots(dim.rows as usize + dim.cols as usize);
        }
    }

    // the scoreless turns before the position to solve, as its search starts:
    // where the game's last turn was a pass (its passes above none), the
    // mover's pass answers it, else the counts
    #[inline(always)]
    fn entering(&self) -> Scoreless {
        if self.scoreless[0] > 0 {
            Scoreless::Answering
        } else {
            Scoreless::Counted {
                passes: self.scoreless[0],
                zeros: self.scoreless[1],
            }
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
            if !self.work_buffer.lane_pool.deal.pool.is_empty() {
                let (rows, cols) = self.zone_lanes(play);
                let num_rows = self.game_config.board_layout().dim().rows as usize;
                self.work_buffer.lane_pool.stale(rows, cols, num_rows);
            }
            let strider = self.game_config.board_layout().dim().lane(*down, *lane);
            let alphabet = self.game_config.alphabet();
            let work_buffer = &mut self.work_buffer;
            let rack = &mut work_buffer.racks[player_idx as usize];
            for (i, &tile) in (*idx..).zip(word.iter()) {
                if tile != 0 {
                    let square = strider.at_in_lane(i);
                    work_buffer.board_tiles[square] = tile;
                    work_buffer.key ^= square_key(square, tile);
                    let held = held_tile(tile);
                    let at = rack.partition_point(|&t| t < held);
                    let count = rack[at..].iter().take_while(|&&t| t == held).count();
                    work_buffer.key ^= rack_step(player_idx, held, count - 1);
                    rack.remove(at);
                    work_buffer.rack_scores[player_idx as usize] -=
                        alphabet.scaled_score(held).raw();
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
                    let square = strider.at_in_lane(i);
                    work_buffer.board_tiles[square] = 0;
                    work_buffer.key ^= square_key(square, tile);
                    let held = held_tile(tile);
                    let at = rack.partition_point(|&t| t < held);
                    let count = rack[at..].iter().take_while(|&&t| t == held).count();
                    work_buffer.key ^= rack_step(player_idx, held, count);
                    rack.insert(at, held);
                    work_buffer.rack_scores[player_idx as usize] +=
                        alphabet.scaled_score(held).raw();
                }
            }
            if !work_buffer.lane_pool.deal.pool.is_empty() {
                work_buffer.lane_pool.unstale();
            }
        }
    }

    // the value for player_idx if both players pass now, each losing their rack
    #[inline(always)]
    fn both_pass_value(&self, player_idx: u8) -> equity::Equity {
        let rack_scores = &self.work_buffer.rack_scores;
        equity::Equity::new(rack_scores[player_idx as usize ^ 1] - rack_scores[player_idx as usize])
    }

    // both_pass_value for the other player after player_idx plays the move at
    // place found of ply's list, as placing it would leave the rack scores
    #[inline(always)]
    fn both_pass_value_after(&self, player_idx: u8, ply: usize, found: usize) -> equity::Equity {
        let alphabet = self.game_config.alphabet();
        let rack_scores = &self.work_buffer.rack_scores;
        let mut mover = rack_scores[player_idx as usize];
        if let movegen::Play::Place { word, .. } = &self.work_buffer.lists[ply][found].play {
            for &tile in word.iter().filter(|&&t| t != 0) {
                mover -= alphabet.scaled_score(held_tile(tile)).raw();
            }
        }
        equity::Equity::new(mover - rack_scores[player_idx as usize ^ 1])
    }

    // the ceiling on what player_idx's last tile adds after their move from
    // this ply (its zone set there): the larger of the both-pass value and that
    // tile's highest single placement with the opponent's rack counted double.
    // Placements on lanes the move left alone are read from this ply's list,
    // made before the move; zone lanes are generated only when the best found
    // so far, the both-pass value included, is at or under need
    #[inline(always)]
    fn one_tile_bound(
        &mut self,
        ply: usize,
        player_idx: u8,
        need: equity::Equity,
    ) -> error::Returns<equity::Equity> {
        let (rows, cols) = self.work_buffer.zones[ply];
        let work_buffer = &mut self.work_buffer;
        let left = work_buffer.racks[player_idx as usize][0];
        let out_bonus = 2 * work_buffer.rack_scores[player_idx as usize ^ 1];
        let mut best = work_buffer.rack_scores[player_idx as usize ^ 1]
            - work_buffer.rack_scores[player_idx as usize];
        // not set aside with a stopped pass's lists, so grown on its own
        if work_buffer.singles.len() <= ply {
            work_buffer.singles.resize_with(ply + 1, Default::default);
        }
        let list = &work_buffer.lists[ply];
        let (of, singles) = &mut work_buffer.singles[ply];
        if *of != work_buffer.serials[ply] {
            singles.clear();
            for (index, candidate) in list.iter().enumerate() {
                if candidate.placed == 1 {
                    singles.push(u32::try_from(index)?);
                }
            }
            *of = work_buffer.serials[ply];
            // highest score first, so the read stops at the first it takes
            singles.sort_unstable_by_key(|&at| {
                std::cmp::Reverse(match &list[at as usize].play {
                    movegen::Play::Place { score, .. } => *score,
                    movegen::Play::Exchange { .. } => equity::Equity::NEG_INFINITY,
                })
            });
        }
        for &at in singles.iter() {
            let candidate = &list[at as usize];
            // a move placing one tile, whose word alone is read for it
            if let movegen::Play::Place {
                down,
                lane,
                word,
                score,
                ..
            } = &candidate.play
                && (if *down { cols } else { rows }) >> lane & 1 == 0
                && let Some(&tile) = word.iter().find(|&&t| t != 0)
                && held_tile(tile) == left
            {
                best = best.max(score.raw() + out_bonus);
                break;
            }
        }
        if equity::Equity::new(best) > need {
            return Ok(equity::Equity::new(best));
        }
        let t1 = (self.progress.is_some()).then(|| Started::now(work_buffer.clock));
        gen_raw_lanes::<LEAN, _, _, _, JUMPS, ANY_SIDE>(
            &mut work_buffer.movegen,
            work_buffer.run_letter_bits.as_deref(),
            &movegen::BoardSnapshot {
                board_tiles: &work_buffer.board_tiles,
                game_config: self.game_config,
                kwg: self.kwg,
                anagrams: self.arm_tables.anagrams(),
                rack_lengths: self.arm_tables.rack_lengths(),
                klv: &self.klv,
            },
            &work_buffer.racks[player_idx as usize],
            rows,
            cols,
            None,
        )?;
        if let Some(t1) = t1 {
            work_buffer.dur_movegen += t1.elapsed();
        }
        for valued in work_buffer.movegen.plays.iter() {
            if let movegen::Play::Place { score, .. } = &valued.play {
                best = best.max(score.raw() + out_bonus);
            }
        }
        Ok(equity::Equity::new(best))
    }

    // the rows and columns whose moves a move can change, on the position
    // searched before it is made
    #[inline(always)]
    fn zone_lanes(&self, play: &movegen::Play) -> (u128, u128) {
        let dim = self.game_config.board_layout().dim();
        zone::zone_lanes_on_board(
            dim,
            &zone::footprint_on_board(dim, &self.work_buffer.board_tiles, play).zone,
        )
    }

    // a child of ply's node about to be entered: that node's plays out, kept
    // while its list is the one they were found for, and the lists generated so
    // far, past which the child's own list is numbered
    #[inline(always)]
    fn enter_outs(&mut self, ply: usize) {
        let work_buffer = &mut self.work_buffer;
        if work_buffer.outs.len() <= ply {
            work_buffer.outs.resize_with(ply + 1, Default::default);
        }
        let outs = &mut work_buffer.outs[ply];
        if outs.of != work_buffer.serials[ply] {
            outs.of = work_buffer.serials[ply];
            outs.len = 0;
        }
        outs.floor = work_buffer.generated;
    }

    // the move ply's node searched below it taken back: the opponent's plays
    // out in the reply's list, when the reply generated one, kept among the
    // node's highest-scoring few when each is a play out on the node's own
    // position too, at the same score: the move filled no square of its zone
    // there, nor any square it plays through
    #[inline]
    fn harvest_outs(&mut self, ply: usize, depth: i8, play: &movegen::Play) {
        let work_buffer = &mut self.work_buffer;
        if ply == 0 || depth < 2 {
            return;
        }
        let Some(outs) = work_buffer.outs.get(ply) else {
            return;
        };
        if outs.of != work_buffer.serials[ply]
            || (work_buffer.serials.get(ply + 1)).is_none_or(|&serial| serial <= outs.floor)
        {
            return;
        }
        let dim = self.game_config.board_layout().dim();
        let board = &work_buffer.board_tiles[..];
        let fills = zone::footprint_on_board(dim, board, play).fills;
        let outs = &mut work_buffer.outs[ply];
        for candidate in work_buffer.lists[ply + 1].iter() {
            if !candidate.plays_out {
                continue;
            }
            let movegen::Play::Place {
                down,
                lane,
                idx,
                word,
                score,
            } = &candidate.play
            else {
                continue;
            };
            if outs.len == OUT_SLOTS && outs.slots[OUT_SLOTS - 1].score >= *score {
                continue;
            }
            if (outs.slots[..outs.len]).iter().any(|out| {
                out.down == *down && out.lane == *lane && out.idx == *idx && out.score == *score
            }) {
                continue;
            }
            let strider = dim.lane(*down, *lane);
            if (*idx..)
                .zip(word.iter())
                .any(|(i, &tile)| tile == 0 && board[strider.at_in_lane(i)] == 0)
            {
                continue;
            }
            let zone = zone::footprint_on_board(dim, board, &candidate.play).zone;
            if zone.meets(&fills) {
                continue;
            }
            let at = (outs.slots[..outs.len]).partition_point(|out| out.score >= *score);
            let end = outs.len.min(OUT_SLOTS - 1);
            outs.slots.copy_within(at..end, at + 1);
            outs.slots[at] = Out {
                score: *score,
                zone,
                down: *down,
                lane: *lane,
                idx: *idx,
            };
            outs.len = (outs.len + 1).min(OUT_SLOTS);
        }
    }

    // the cap on player_idx's move from ply before it is placed: by the
    // highest-scoring play out the node keeps that the move filled no square of
    // the zone of (fills), its score minus that play's score minus twice the
    // rack the mover keeps; with its slot, when no higher than alpha
    #[inline(always)]
    fn out_cap(
        &self,
        ply: usize,
        player_idx: u8,
        score: equity::Equity,
        alpha: equity::Equity,
        fills: &zone::Squares,
        play: &movegen::Play,
    ) -> Option<(usize, equity::Equity)> {
        let work_buffer = &self.work_buffer;
        let outs = work_buffer.outs.get(ply)?;
        if outs.of != work_buffer.serials[ply] {
            return None;
        }
        let k = (outs.slots[..outs.len])
            .iter()
            .position(|out| !out.zone.meets(fills))?;
        let movegen::Play::Place { word, .. } = play else {
            return None;
        };
        let alphabet = self.game_config.alphabet();
        let spent: i32 = word
            .iter()
            .filter(|&&tile| tile != 0)
            .map(|&tile| alphabet.scaled_score(held_tile(tile)).raw())
            .sum();
        let kept = work_buffer.rack_scores[player_idx as usize] - spent;
        let reply = outs.slots[k]
            .score
            .finite_add(equity::Equity::new(2 * kept));
        let cap = score.finite_sub(reply);
        (cap <= alpha).then_some((k, cap))
    }

    // a debug build's check of a skip (out_cap), the move placed: the play out
    // is one of the opponent's on the position after it, at its score, and, for
    // the first few skips of a solver, a solve of the reply from there values
    // the move at no more than the cap; true, as it asserts the rest
    #[inline(never)]
    fn check_out_skip(
        &mut self,
        ply: usize,
        player_idx: u8,
        k: usize,
        score: equity::Equity,
        cap: equity::Equity,
    ) -> error::Returns<bool> {
        let out = self.work_buffer.outs[ply].slots[k];
        let other = player_idx ^ 1;
        let work_buffer = &mut self.work_buffer;
        gen_raw_lanes::<LEAN, _, _, _, JUMPS, ANY_SIDE>(
            &mut work_buffer.movegen,
            work_buffer.run_letter_bits.as_deref(),
            &movegen::BoardSnapshot {
                board_tiles: &work_buffer.board_tiles,
                game_config: self.game_config,
                kwg: self.kwg,
                anagrams: self.arm_tables.anagrams(),
                rack_lengths: self.arm_tables.rack_lengths(),
                klv: &self.klv,
            },
            &work_buffer.racks[other as usize],
            !0,
            !0,
            None,
        )?;
        let held = work_buffer.racks[other as usize].len();
        assert!(
            work_buffer.movegen.plays.iter().any(|valued| matches!(
                &valued.play,
                movegen::Play::Place { down, lane, idx, word, score }
                    if *down == out.down && *lane == out.lane && *idx == out.idx
                        && *score == out.score
                        && word.iter().filter(|&&tile| tile != 0).count() == held
            )),
            "a skip's play out is not the opponent's after the move"
        );
        if work_buffer.out_checks >= 16 {
            return Ok(true);
        }
        work_buffer.out_checks += 1;
        let mut reply = Self::with_generator(
            self.game_config,
            self.kwg,
            self.arm_tables,
            self.run_letters.clone(),
            movegen::KurniaMoveGenerator::new_in(self.game_config),
        )
        .unwrap();
        // the reply skips too, but solves no replies of its own to check its
        // skips, lest the checks nest
        reply.work_buffer.out_checks = u32::MAX;
        let work_buffer = &self.work_buffer;
        reply.init(
            &work_buffer.board_tiles,
            [&work_buffer.racks[0], &work_buffer.racks[1]],
        );
        let replied = reply.solve(other)?;
        assert!(
            score - replied <= cap,
            "a skipped move is worth {} past its cap {cap}",
            score - replied
        );
        Ok(true)
    }

    // a root's requested placements. Two plies down makes its own list since
    // the requested placements cannot supply all the unchanged lanes
    fn generate_root(&mut self, player_idx: u8, keeps: bool) -> error::Returns<()> {
        let Some(root) = self.root_moves else {
            return self.generate(0, player_idx, keeps);
        };
        let work = &mut self.work_buffer;
        work.begin_list(0);
        work.keeps[0] = false;
        let list = &mut work.lists[0];
        let order = &mut work.orders[0];
        list.clear();
        order.clear();
        let places = root.plays.len() - usize::from(root.pass);
        if list.try_reserve(places).is_err() || order.try_reserve(places).is_err() {
            work.movegen_cut = true;
            return Ok(());
        }
        let rack_len = work.racks[player_idx as usize].len();
        let bonus = 2 * work.rack_scores[player_idx as usize ^ 1];
        for play in &root.plays {
            if let movegen::Play::Place { word, .. } = play {
                let placed = word.iter().filter(|&&tile| tile != 0).count();
                add_placement(list, order, play, 0, placed, rack_len, bonus)?;
            }
        }
        work.movegen_cut = work.memory_passed();
        Ok(())
    }

    // generate player_idx's moves that place tiles (a pass is searched on its
    // own) on the position searched, into this ply's list, and their keys,
    // unsorted, into its orders. Below the first two plies, only the zones of
    // the previous two moves are generated afresh, and the mover's moves on
    // other lanes are copied from the list of their previous turn if the rack
    // still has those tiles; of the lanes only the move two plies up changed,
    // those an earlier reply to the same list found are copied from it. With
    // keeps, the moves record rack places for reuse two plies down
    #[inline]
    fn generate(&mut self, ply: usize, player_idx: u8, keeps: bool) -> error::Returns<()> {
        let work_buffer = &mut self.work_buffer;
        work_buffer.begin_list(ply);
        let leaving = work_buffer.table.leaving();
        let limit = work_buffer.memory_limit.map(|limit| movegen::HeldBytes {
            memory: MemoryLimit {
                bytes: limit.bytes.saturating_add(leaving),
                ..limit
            },
            kept: work_buffer.held_kept,
        });
        let kept_from = ply >= 2 && work_buffer.keeps[ply - 2];
        work_buffer.keeps[ply] = keeps;
        let (rows, cols) = if kept_from {
            let (rows, cols) = work_buffer.zones[ply - 2];
            let (replied_rows, replied_cols) = work_buffer.zones[ply - 1];
            (rows | replied_rows, cols | replied_cols)
        } else {
            (!0, !0)
        };
        let spent = if kept_from {
            work_buffer.spent[ply - 2]
        } else {
            0
        };
        // the lanes only the move two plies up changed, and of those, the ones
        // an earlier reply to the same list found
        let (only_rows, only_cols, copy_rows, copy_cols) = if kept_from {
            let (rows, cols) = work_buffer.zones[ply - 2];
            let (replied_rows, replied_cols) = work_buffer.zones[ply - 1];
            let (only_rows, only_cols) = (rows & !replied_rows, cols & !replied_cols);
            let parent = work_buffer.serials[ply - 1];
            let siblings = &mut work_buffer.siblings[ply];
            if siblings.of != parent {
                siblings.of = parent;
                siblings.rows = 0;
                siblings.cols = 0;
                siblings.moves.clear();
            }
            let (copy_rows, copy_cols) = (only_rows & siblings.rows, only_cols & siblings.cols);
            (only_rows, only_cols, copy_rows, copy_cols)
        } else {
            (0, 0, 0, 0)
        };
        #[cfg(test)]
        {
            work_buffer.copied += (copy_rows.count_ones() + copy_cols.count_ones()) as usize;
        }
        let (walk_rows, walk_cols) = (rows & !copy_rows, cols & !copy_cols);
        let walks = walk_rows != 0 || walk_cols != 0;
        // inside a search with tiles in the bag (LanePool), each lane to walk
        // takes its moves from its key's list, the pool's walk made into one
        // the second time the key is met (the first is the rack's walk); where
        // the memory limit or the pool's budget leaves no room for a list (or
        // the limit for the keys walked once), every lane is walked with the
        // rack, as outside one. The lean arm pools nothing, so its builds
        // carry no pool
        let mut pooled = !LEAN && walks && {
            let rack = &work_buffer.racks[player_idx as usize];
            let lane_pool = &mut work_buffer.lane_pool;
            !lane_pool.deal.pool.is_empty()
                && rack.len() <= self.game_config.rack_size() as usize
                && within(rack, &lane_pool.deal.pool)
                && lane_pool.sees_within(work_buffer.memory_limit)
        };
        let (mut rack_rows, mut rack_cols) = (0u128, 0u128);
        if pooled {
            let dim = self.game_config.board_layout().dim();
            let num_rows = dim.rows as usize;
            let lane_pool = &mut work_buffer.lane_pool;
            let board = &work_buffer.board_tiles[..];
            let board_empty = || !board.iter().any(|&tile| tile != 0);
            lane_pool.fresh.clear();
            for (mask, lanes, base, down) in [
                (walk_rows, dim.rows, 0, false),
                (walk_cols, dim.cols, num_rows, true),
            ] {
                for lane in 0..lanes {
                    if mask >> lane & 1 == 0 {
                        continue;
                    }
                    let slot = base + lane as usize;
                    if lane_pool.slot_lists[slot] != NO_LIST {
                        debug_assert!(
                            lane_pool.slot_keys[slot]
                                == lane_key(board, dim, board_empty(), down, lane)
                        );
                        continue;
                    }
                    let key = lane_key(board, dim, board_empty(), down, lane);
                    #[cfg(debug_assertions)]
                    {
                        let mut bytes = Vec::new();
                        let mut fits = true;
                        lane_bytes(board, dim, board_empty(), down, lane, |byte| {
                            fits &= bytes.try_reserve(1).is_ok();
                            if fits {
                                bytes.push(byte);
                            }
                        });
                        if let Some(first) = lane_pool.lanes.get(&key) {
                            assert!(!fits || *first == bytes, "two lanes' keys clash");
                        } else if fits && lane_pool.lanes.try_reserve(1).is_ok() {
                            lane_pool.lanes.insert(key, bytes);
                        }
                    }
                    lane_pool.slot_keys[slot] = key;
                    match lane_pool.keys.get(key) {
                        Some(list) => lane_pool.slot_lists[slot] = list,
                        None if lane_pool.sighted(key) => lane_pool.fresh.push(slot as u16),
                        None if down => rack_cols |= 1 << lane,
                        None => rack_rows |= 1 << lane,
                    }
                }
            }
            if !lane_pool.fresh.is_empty() {
                let (mut new_rows, mut new_cols) = (0u128, 0u128);
                for &slot in &lane_pool.fresh {
                    if (slot as usize) < num_rows {
                        new_rows |= 1 << slot;
                    } else {
                        new_cols |= 1 << (slot as usize - num_rows);
                    }
                }
                let t1 = (self.progress.is_some()).then(|| Started::now(work_buffer.clock));
                // its walks' rack is the whole pool, so its leaves' table is
                // held to the pool's budget, and to the bound others keep
                let dense_most = (lane_pool.deal.budget / klv::LEAVE_BYTES)
                    .min(klv::DENSE_LEAVES_MOST as usize) as u32;
                let pool_walker = &mut work_buffer.pool_walker;
                let walker = match pool_walker.take() {
                    Some((made_for, walker)) if made_for == dense_most => walker,
                    Some((_, walker)) => walker.with_dense_most(dense_most),
                    None => {
                        let memory = work_buffer.memory_limit;
                        let Some(walker) =
                            movegen::KurniaMoveGenerator::new_within(self.game_config, memory)
                        else {
                            work_buffer.movegen_cut = true;
                            return Ok(());
                        };
                        walker.with_dense_most(dense_most)
                    }
                };
                let walker = &mut pool_walker.insert((dense_most, walker)).1;
                work_buffer.movegen_cut |= gen_raw_lanes::<LEAN, _, _, _, JUMPS, ANY_SIDE>(
                    walker,
                    work_buffer.run_letter_bits.as_deref(),
                    &movegen::BoardSnapshot {
                        board_tiles: board,
                        game_config: self.game_config,
                        kwg: self.kwg,
                        anagrams: self.arm_tables.anagrams(),
                        rack_lengths: self.arm_tables.rack_lengths(),
                        klv: &self.klv,
                    },
                    &lane_pool.deal.pool,
                    new_rows,
                    new_cols,
                    limit,
                )?;
                if work_buffer.movegen_cut {
                    return Ok(());
                }
                if let Some(t1) = t1 {
                    work_buffer.dur_movegen += t1.elapsed();
                }
                pooled = lane_pool.record(&walker.plays, work_buffer.memory_limit, num_rows);
            }
        }
        // the lanes met once, walked with the rack
        let racked = pooled && (rack_rows != 0 || rack_cols != 0);
        if (!pooled && walks) || racked {
            let (rack_walk_rows, rack_walk_cols) = if pooled {
                (rack_rows, rack_cols)
            } else {
                (walk_rows, walk_cols)
            };
            let t1 = (self.progress.is_some()).then(|| Started::now(work_buffer.clock));
            work_buffer.movegen_cut |= gen_raw_lanes::<LEAN, _, _, _, JUMPS, ANY_SIDE>(
                &mut work_buffer.movegen,
                work_buffer.run_letter_bits.as_deref(),
                &movegen::BoardSnapshot {
                    board_tiles: &work_buffer.board_tiles,
                    game_config: self.game_config,
                    kwg: self.kwg,
                    anagrams: self.arm_tables.anagrams(),
                    rack_lengths: self.arm_tables.rack_lengths(),
                    klv: &self.klv,
                },
                &work_buffer.racks[player_idx as usize],
                rack_walk_rows,
                rack_walk_cols,
                limit,
            )?;
            if work_buffer.movegen_cut {
                return Ok(());
            }
            if let Some(t1) = t1 {
                work_buffer.dur_movegen += t1.elapsed();
            }
        }
        let rack = &work_buffer.racks[player_idx as usize];
        let slots = keeps.then(|| RackSlots::of(rack));
        let out_bonus = 2 * work_buffer.rack_scores[player_idx as usize ^ 1];
        let (above, here) = work_buffer.lists.split_at_mut(ply);
        let list = &mut here[0];
        let ordered = &mut work_buffer.orders[ply];
        let siblings = &mut work_buffer.siblings[ply];
        list.clear();
        ordered.clear();
        let mut push = |play: &movegen::Play, uses: u32, placed: usize| {
            add_placement(list, ordered, play, uses, placed, rack.len(), out_bonus)
        };
        // the lanes walked that only the move two plies up changed are kept for
        // the later replies to the same list
        let (keep_rows, keep_cols) = (only_rows & !copy_rows, only_cols & !copy_cols);
        #[cfg(debug_assertions)]
        let mut from_pool = Vec::new();
        #[cfg(debug_assertions)]
        let as_tuple = |play: &movegen::Play| match play {
            movegen::Play::Place {
                down,
                lane,
                idx,
                word,
                score,
            } => Some((*down, *lane, *idx, word[..].to_vec(), *score)),
            movegen::Play::Exchange { .. } => None,
        };
        if pooled {
            // each walked lane's list: the groups whose tiles the rack holds,
            // found again only when the list differs from the last fetch's, or
            // the rack's tiles do
            let held = work_buffer.lane_pool.held_by(rack);
            let LanePool {
                lists,
                needs,
                ends,
                entries,
                moves,
                slot_lists,
                scans,
                ranges,
                ..
            } = &mut work_buffer.lane_pool;
            let dim = self.game_config.board_layout().dim();
            let num_rows = dim.rows as usize;
            for (mask, lanes, base, keep_mask) in [
                (walk_rows & !rack_rows, dim.rows, 0, keep_rows),
                (walk_cols & !rack_cols, dim.cols, num_rows, keep_cols),
            ] {
                for lane in 0..lanes {
                    if mask >> lane & 1 == 0 {
                        continue;
                    }
                    let keep = keep_mask >> lane & 1 != 0;
                    let slot = base + lane as usize;
                    let list = slot_lists[slot];
                    let taken = &mut ranges[slot];
                    if scans[slot] != (list, held) {
                        scans[slot] = (list, held);
                        taken.clear();
                        let [first, last, mut from] = lists[list as usize];
                        for group in first as usize..last as usize {
                            let end = ends[group];
                            if fits(held, needs[group]) {
                                match taken.last_mut() {
                                    Some(range) if range.1 == from => range.1 = end,
                                    _ => taken.push((from, end)),
                                }
                            }
                            from = end;
                        }
                    }
                    for &(from, end) in taken.iter() {
                        for &id in &entries[from as usize..end as usize] {
                            let (play, placed) = &moves[id as usize];
                            let movegen::Play::Place { word, .. } = play else {
                                continue;
                            };
                            let uses =
                                (slots.as_ref()).map_or(0, |slots| slots.places(word, false));
                            push(play, uses, *placed as usize)?;
                            if keep {
                                siblings.moves.push((play.clone(), *placed));
                            }
                            #[cfg(debug_assertions)]
                            from_pool.extend(as_tuple(play));
                        }
                    }
                }
            }
        }
        if (!pooled && walks) || racked {
            for valued in work_buffer.movegen.plays.iter() {
                if let movegen::Play::Place {
                    down, lane, word, ..
                } = &valued.play
                {
                    // a move's rack places are a bit a tile it places, so with
                    // them its tiles are counted without a second pass
                    let placed = match &slots {
                        Some(slots) => {
                            let uses = slots.places(word, false);
                            debug_assert_eq!(
                                uses.count_ones() as usize,
                                word.iter().filter(|&&t| t != 0).count()
                            );
                            push(&valued.play, uses, uses.count_ones() as usize)?;
                            uses.count_ones() as u8
                        }
                        None => {
                            let placed = word.iter().filter(|&&t| t != 0).count();
                            push(&valued.play, 0, placed)?;
                            placed as u8
                        }
                    };
                    if (if *down { keep_cols } else { keep_rows }) >> lane & 1 != 0 {
                        siblings.moves.push((valued.play.clone(), placed));
                    }
                    #[cfg(debug_assertions)]
                    from_pool.extend(as_tuple(&valued.play));
                }
            }
        }
        // a debug build walks a pooled list's lanes with the rack again, for
        // the same moves
        #[cfg(debug_assertions)]
        if pooled {
            work_buffer.movegen_cut |= gen_raw_lanes::<LEAN, _, _, _, JUMPS, ANY_SIDE>(
                &mut work_buffer.movegen,
                work_buffer.run_letter_bits.as_deref(),
                &movegen::BoardSnapshot {
                    board_tiles: &work_buffer.board_tiles,
                    game_config: self.game_config,
                    kwg: self.kwg,
                    anagrams: self.arm_tables.anagrams(),
                    rack_lengths: self.arm_tables.rack_lengths(),
                    klv: &self.klv,
                },
                rack,
                walk_rows,
                walk_cols,
                limit,
            )?;
            if work_buffer.movegen_cut {
                return Ok(());
            }
            let mut walked = (work_buffer.movegen.plays.iter())
                .filter_map(|valued| as_tuple(&valued.play))
                .collect::<Vec<_>>();
            from_pool.sort_unstable();
            walked.sort_unstable();
            assert!(
                from_pool == walked,
                "a lane's pool moves differ from its walk: {} against {}; rack {rack:?}; pool {:?}",
                from_pool.len(),
                walked.len(),
                work_buffer.lane_pool.deal.pool
            );
        }
        siblings.rows |= keep_rows;
        siblings.cols |= keep_cols;
        if copy_rows != 0 || copy_cols != 0 {
            for (play, placed) in &siblings.moves {
                if let movegen::Play::Place {
                    down, lane, word, ..
                } = play
                    && (if *down { copy_cols } else { copy_rows }) >> lane & 1 != 0
                {
                    let uses = (slots.as_ref()).map_or(0, |slots| slots.places(word, false));
                    push(play, uses, *placed as usize)?;
                }
            }
        }
        if kept_from {
            // off the reached lanes, the moves two plies up whose tiles the
            // rack still holds
            for kept in &above[ply - 2] {
                let movegen::Play::Place {
                    down, lane, word, ..
                } = &kept.play
                else {
                    continue;
                };
                if (if *down { cols } else { rows }) >> lane & 1 == 0 && kept.uses & spent == 0 {
                    let uses = (slots.as_ref()).map_or(0, |slots| slots.places(word, false));
                    push(&kept.play, uses, kept.uses.count_ones() as usize)?;
                }
            }
        }
        Ok(())
    }

    // the longest line from the position to solve, player_idx to move: a move
    // that does not end it places tiles and leaves the mover some, or passes; a
    // pass answering a pass ends it, as does a pass bringing the scoreless turns
    // to the config's count, and a play out. A mover holding one tile can pass
    // before and after every tile the other places, so its line can run a ply
    // longer than any a mover holding more can make; a rack with no tiles can
    // only pass, as a rack holding its last tile does short of ending the game,
    // so it counts as one tile
    #[inline(always)]
    fn longest_line(&self, player_idx: u8) -> usize {
        let mine = self.racks[player_idx as usize].len().max(1);
        let theirs = self.racks[player_idx as usize ^ 1].len().max(1);
        (2 * (mine + theirs) + usize::from(mine == 1) - 3).min(COMPLETE as usize - 1)
    }

    // start a search of the position to solve within the limits, on the clock
    // as it stands
    #[inline(always)]
    fn begin_search(&mut self, limits: &Limits) {
        let work_buffer = &mut self.work_buffer;
        work_buffer.time_limit = limits.time;
        work_buffer.state_limit = limits.states;
        work_buffer.limit_memory(limits.memory);
        work_buffer.can_grow = true;
        work_buffer.searched = 0;
        work_buffer.memory_cut = false;
        work_buffer.finished = 0;
        work_buffer.root_best = NO_PLAY;
        work_buffer.answer = None;
        work_buffer.table.generation += 1;
        work_buffer.last_look = std::time::Duration::ZERO;
    }

    #[inline]
    fn run_id_loop(
        &mut self,
        player_idx: u8,
        verbose: bool,
        limits: &Limits,
        finished: Option<(i8, equity::Equity)>,
        resumable: bool,
    ) -> error::Returns<Solved> {
        let limited = limits.time.is_some() || limits.states.is_some() || limits.memory.is_some();
        self.work_buffer.resumable = resumable && limited;
        let solved = self.run_id_loop_below(
            player_idx,
            verbose,
            limits,
            finished,
            equity::Equity::INFINITY,
        );
        self.work_buffer.resumable = false;
        solved
    }

    // the search deepening depth by depth, as run_id_loop; with a finite beta,
    // each depth searched below it alone, so a value not below beta says only
    // that the position is worth that much at least
    #[inline]
    fn run_id_loop_below(
        &mut self,
        player_idx: u8,
        verbose: bool,
        limits: &Limits,
        finished: Option<(i8, equity::Equity)>,
        beta: equity::Equity,
    ) -> error::Returns<Solved> {
        let mut last_valuation = None;
        let longest = self.longest_line(player_idx);
        self.begin_search(limits);
        // under a time or state limit, every depth is searched, so a stopped
        // solve answers from the latest. A memory limit rarely stops a solve
        // (the table reuses its room), so without the other two, each pass
        // after the second goes two plies deeper, up to the longest line
        let limited = limits.time.is_some() || limits.states.is_some() || limits.memory.is_some();
        let every_depth = limits.time.is_some() || limits.states.is_some();
        let mut max_depth = 0i8;
        // deepening on from a depth a solve before it finished on this table,
        // with the value found there: short of the longest line, so that the
        // pass reaching every line's end is searched
        if let Some((depth, value)) = finished {
            max_depth = depth.clamp(0, longest as i8 - 1);
            self.work_buffer.finished = max_depth;
            last_valuation = Some(value);
        }
        // a deepening search with a tile in the bag asks no deeper than its pass
        let cap = self.work_buffer.depth_cap;
        while (max_depth as usize) < longest {
            if max_depth >= cap {
                // short of the end, the deepest pass asked for answers
                return Ok(Solved {
                    value: last_valuation,
                    depth: max_depth,
                    exact: false,
                    partway: None,
                    out_of_memory: false,
                    out_of_memory_in_moves: false,
                });
            }
            max_depth = if every_depth || max_depth < 2 {
                max_depth + 1
            } else {
                (max_depth + 2).min(longest as i8)
            }
            .min(cap);
            self.work_buffer.max_depth = max_depth;
            // the first pass always finishes, so a solve cut short still has a value
            self.work_buffer.check_limits = max_depth > 1 && limited;
            self.work_buffer.nodes = 0;
            self.work_buffer.next_look = self.work_buffer.cadence;
            self.work_buffer.cut_short = false;
            // a stopped pass resumed keeps the move it proved
            if !(self.work_buffer.resumable
                && (self.work_buffer.stopped).is_some_and(|(_, _, depth, _, _)| depth == max_depth))
            {
                self.work_buffer.proven = None;
            }
            let searched = if beta < equity::Equity::INFINITY {
                self.root_search(player_idx, max_depth, equity::Equity::NEG_INFINITY, beta)
            } else if let Some(last_valuation) = last_valuation
                && max_depth > 1
            {
                self.aspiration_search(player_idx, max_depth, last_valuation)
            } else {
                self.root_search(
                    player_idx,
                    max_depth,
                    equity::Equity::NEG_INFINITY,
                    equity::Equity::INFINITY,
                )
            };
            let searched = match searched {
                Ok(searched) => searched,
                // the answer is the deepest finished pass's best move and
                // value, named here because deepening by two can skip the pass
                // before and the root's table entry may since hold another
                // move; or, when the stopped pass recorded another move worth
                // more than that best (proven), that move and its value
                Err(_) if self.work_buffer.cut_short => {
                    let work_buffer = &mut self.work_buffer;
                    let best = (work_buffer.root_best != NO_PLAY)
                        .then_some(work_buffer.root_best)
                        .zip(last_valuation);
                    work_buffer.answer = work_buffer.proven.or(best);
                    return Ok(Solved {
                        value: work_buffer.answer.map(|(_, v)| v).or(last_valuation),
                        depth: work_buffer.finished,
                        exact: false,
                        partway: work_buffer.proven.map(|_| max_depth),
                        out_of_memory: work_buffer.movegen_cut || work_buffer.memory_cut,
                        out_of_memory_in_moves: work_buffer.movegen_cut
                            && work_buffer.finished == 0,
                    });
                }
                Err(e) => return Err(e),
            };
            last_valuation = Some(searched.value);
            self.work_buffer.finished = max_depth;
            self.work_buffer.root_best = self.work_buffer.pass_best;
            if self.work_buffer.in_bag.is_none() {
                self.report(Some(searched.value))?;
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
                    value: Some(searched.value),
                    depth: max_depth,
                    exact: false,
                    partway: None,
                    out_of_memory: self.work_buffer.memory_cut,
                    out_of_memory_in_moves: false,
                });
            }
        }
        Ok(Solved {
            value: last_valuation,
            depth: self.work_buffer.max_depth,
            exact: true,
            partway: None,
            out_of_memory: false,
            out_of_memory_in_moves: false,
        })
    }

    #[inline(always)]
    fn past_a_limit(&mut self) -> bool {
        let work_buffer = &mut self.work_buffer;
        if work_buffer
            .time_limit
            .is_some_and(|limit| work_buffer.t0.is_some_and(|t0| t0.elapsed() >= limit))
            || work_buffer
                .state_limit
                .is_some_and(|limit| work_buffer.table.held >= limit)
        {
            return true;
        }
        work_buffer.memory_cut = work_buffer.memory_passed();
        work_buffer.memory_cut
    }

    // search the position to solve to depth within alpha and beta
    #[inline]
    fn root_search(
        &mut self,
        player_idx: u8,
        depth: i8,
        alpha: equity::Equity,
        beta: equity::Equity,
    ) -> error::Returns<Searched> {
        self.reset_position();
        if !self.work_buffer.resumable {
            self.work_buffer.verdicts.clear();
            self.work_buffer.at_root = true;
            return self.negamax_eval(0, player_idx, depth, alpha, beta, self.entering());
        }
        // a timed solve's pass: a stopped one of the same position and pass goes
        // on where it stopped, its moves placed again on the board as it left
        // them; any other is dropped. A limit keeps this one's frames
        let pass = (
            self.root_key(player_idx, self.entering()),
            player_idx,
            depth,
            alpha,
            beta,
        );
        let enter = if self.work_buffer.stopped == Some(pass) && self.work_buffer.pending.is_some()
        {
            for t in 0..self.work_buffer.frames.len() {
                if let Child::Move { play, .. } = &self.work_buffer.frames[t].child {
                    let (play, mover) = (play.clone(), self.work_buffer.frames[t].node.player_idx);
                    self.place(mover, &play);
                }
            }
            self.work_buffer.resumed += 1;
            self.work_buffer
                .pending
                .take()
                .map(|(node, root)| (node, Some(root)))
        } else {
            self.work_buffer.frames.clear();
            self.work_buffer.pending = None;
            self.work_buffer.verdicts.clear();
            self.work_buffer.at_root = true;
            let scoreless = self.entering();
            Some((
                Enter {
                    ply: 0,
                    player_idx,
                    depth,
                    alpha,
                    beta,
                    scoreless,
                },
                None,
            ))
        };
        self.work_buffer.stopped = None;
        let searched = self.search_from(0, enter);
        match searched {
            Err(_) if self.work_buffer.cut_short => self.work_buffer.stopped = Some(pass),
            Err(_) => self.unwind(0),
            Ok(_) => {}
        }
        searched
    }

    #[inline]
    fn aspiration_search(
        &mut self,
        player_idx: u8,
        max_depth: i8,
        last_valuation: equity::Equity,
    ) -> error::Returns<Searched> {
        const ASPIRATION_WINDOW: equity::Equity = equity::Equity::new(32 * super::equity::SCALE);
        let lo = last_valuation - ASPIRATION_WINDOW;
        let hi = last_valuation + ASPIRATION_WINDOW;
        // a stopped search with the full window goes on as that, the narrow one
        // having been searched
        let full = (self.work_buffer.stopped).is_some_and(|(_, mover, depth, alpha, beta)| {
            mover == player_idx
                && depth == max_depth
                && alpha == equity::Equity::NEG_INFINITY
                && beta == equity::Equity::INFINITY
        });
        if !(self.work_buffer.resumable && full) {
            let searched = self.root_search(player_idx, max_depth, lo, hi)?;
            if searched.value > lo && searched.value < hi {
                return Ok(searched);
            }
        }
        self.root_search(
            player_idx,
            max_depth,
            equity::Equity::NEG_INFINITY,
            equity::Equity::INFINITY,
        )
    }

    // the nodes the last solve searched, every one its listener heard of
    #[inline(always)]
    pub fn searched(&self) -> u64 {
        self.work_buffer.searched
    }

    // every solve f makes sends its progress to the listener, at every node
    // and at each finished depth; outside f it goes nowhere
    #[inline(always)]
    pub fn listening<T>(
        &mut self,
        progress: impl FnMut(&Progress) -> error::Returns<()> + 'static,
        f: impl FnOnce(&mut Self) -> T,
    ) -> T {
        let before = self.progress.replace(Box::new(progress));
        let made = f(self);
        self.progress = before;
        made
    }

    #[inline(always)]
    fn start_clock(&mut self, limits: &Limits) {
        self.work_buffer.clock = limits.clock;
        self.work_buffer.t0 = Some(Started::now(limits.clock));
        self.work_buffer.dur_movegen = Default::default();
    }

    // solve within the limits, also printing each finished depth's best line
    #[inline(always)]
    pub fn evaluate(&mut self, player_idx: u8, limits: &Limits) -> error::Returns<Solved> {
        self.start_clock(limits);
        self.run_id_loop(player_idx, true, limits, None, false)
    }

    pub fn evaluate_on_threads(
        &mut self,
        player_idx: u8,
        limits: &Limits,
        threads: usize,
    ) -> error::Returns<Solved> {
        self.work_buffer.threads = threads;
        let solved = self.evaluate(player_idx, limits);
        self.work_buffer.threads = 1;
        solved
    }

    #[inline(always)]
    pub fn solve(&mut self, player_idx: u8) -> error::Returns<equity::Equity> {
        self.solve_within(player_idx, &Limits::NONE)?
            .value
            .ok_or_else(|| "a solve without limits finished no depth".into())
    }

    #[inline(always)]
    pub fn solve_within(&mut self, player_idx: u8, limits: &Limits) -> error::Returns<Solved> {
        self.start_clock(limits);
        self.run_id_loop(player_idx, false, limits, None, true)
    }

    pub fn solve_within_on_threads(
        &mut self,
        player_idx: u8,
        limits: &Limits,
        threads: usize,
    ) -> error::Returns<Solved> {
        self.work_buffer.threads = threads;
        let solved = self.solve_within(player_idx, limits);
        self.work_buffer.threads = 1;
        solved
    }

    // a table of as many buckets as fit in this many bytes from the start, in
    // place of a smaller one holding nothing; false, the table as it was,
    // otherwise. A table grown from one bucket replaces positions while it is
    // small that the solve then searches again
    pub fn reserve_table(&mut self, bytes: usize) -> bool {
        let buckets = bytes / std::mem::size_of::<[u64; 8]>();
        let table = &self.work_buffer.table;
        if table.held != 0 || table.growing() || table.room() >= 4 * buckets {
            return false;
        }
        let Some(table) = Table::with_buckets(buckets) else {
            return false;
        };
        self.work_buffer.table = table;
        true
    }

    // solve_within, deepening on from a depth a solve of the same position on
    // this table finished, with the value it found there, as a page solving in
    // steps does: the first pass is the next depth, which a limit may stop, and
    // a solve stopped there answers from the depth it was given. A pass a limit
    // stopped goes on where it stopped, the table (taken and given) carrying it
    #[inline(always)]
    pub fn solve_within_after(
        &mut self,
        player_idx: u8,
        limits: &Limits,
        finished: (i8, equity::Equity),
    ) -> error::Returns<Solved> {
        self.start_clock(limits);
        self.run_id_loop(player_idx, false, limits, Some(finished), true)
    }

    // the move for player_idx to play, lead being their score less the other's
    // (a game is won when lead plus the value of the rest is above zero). Under
    // a time or state limit, a quarter of it first asks whether any move wins;
    // then the solve runs on the same table and clock, a tenth kept back once a
    // move won. The solve's answer is played when it finished or no win was
    // found; else the found win is kept: the answer if it is the winning move
    // or a check in the tenth shows it wins, else the winning move
    pub fn best_move_to_win(
        &mut self,
        player_idx: u8,
        lead: equity::Equity,
        limits: &Limits,
    ) -> error::Returns<Option<movegen::Play>> {
        self.start_clock(limits);
        let margin = -lead;
        let share = |part: u32, whole: u32| Limits {
            time: limits.time.map(|time| time * part / whole),
            states: limits
                .states
                .map(|states| states / whole as usize * part as usize),
            memory: limits.memory,
            clock: limits.clock,
        };
        let mut winner = None;
        if (limits.time.is_some() || limits.states.is_some())
            && let Some(index) = self.ask_margin(player_idx, margin, &share(1, 4))?
        {
            winner = self.root_play(player_idx, index)?;
        }
        let solved = self.run_id_loop(
            player_idx,
            false,
            &share(if winner.is_some() { 9 } else { 10 }, 10),
            None,
            false,
        )?;
        let mut line = Vec::new();
        self.collect_pv(player_idx, &mut line)?;
        let answer = line.into_iter().next().map(|(_, play)| play);
        let Some(winner) = winner.filter(|_| !solved.exact) else {
            return Ok(answer);
        };
        Ok(Some(match answer {
            Some(answer)
                if answer == winner || self.move_wins(player_idx, &answer, margin, limits)? =>
            {
                answer
            }
            _ => winner,
        }))
    }

    // whether a move of player_idx is worth more than margin, asked depth by
    // depth with a zero-width window there, within the limits on the clock as
    // it stands: the winning move's index, once a pass that reaches the end of
    // every line finds one
    #[inline]
    fn ask_margin(
        &mut self,
        player_idx: u8,
        margin: equity::Equity,
        limits: &Limits,
    ) -> error::Returns<Option<u16>> {
        let longest = self.longest_line(player_idx);
        self.begin_search(limits);
        let limited = limits.time.is_some() || limits.states.is_some() || limits.memory.is_some();
        let point = equity::Equity::new(equity::SCALE);
        for max_depth in 1..=longest as i8 {
            self.work_buffer.max_depth = max_depth;
            self.work_buffer.check_limits = max_depth > 1 && limited;
            self.work_buffer.nodes = 0;
            self.work_buffer.next_look = self.work_buffer.cadence;
            self.work_buffer.cut_short = false;
            self.work_buffer.proven = None;
            match self.root_search(player_idx, max_depth, margin, margin + point) {
                Ok(searched) if searched.complete || max_depth as usize == longest => {
                    return Ok((searched.value > margin).then_some(self.work_buffer.pass_best));
                }
                Ok(_) if self.past_a_limit() => return Ok(None),
                Ok(_) => {}
                Err(_) if self.work_buffer.cut_short => return Ok(None),
                Err(e) => return Err(e),
            }
        }
        Ok(None)
    }

    // the move of player_idx at the root with this index (its place in the
    // whole order), a pass for PASS_PLAY
    #[inline]
    fn root_play(&mut self, player_idx: u8, index: u16) -> error::Returns<Option<movegen::Play>> {
        if index == PASS_PLAY {
            return Ok(Some(movegen::Play::Exchange {
                tiles: [][..].into(),
            }));
        }
        self.reset_position();
        self.generate_root(player_idx, false)?;
        let work_buffer = &mut self.work_buffer;
        order(
            &mut work_buffer.lists[0],
            &mut work_buffer.orders[0],
            0,
            NO_PLAY,
        );
        Ok(self.work_buffer.lists[0]
            .iter()
            .find(|candidate| candidate.index == index)
            .map(|candidate| candidate.play.clone()))
    }

    // whether player_idx's move play from the position to solve is worth more
    // than margin: its reply asked depth by depth with a zero-width window,
    // within the limits on the clock as it stands, the table kept; false unless
    // the reply's search reaches the end of every line
    #[inline]
    fn move_wins(
        &mut self,
        player_idx: u8,
        play: &movegen::Play,
        margin: equity::Equity,
        limits: &Limits,
    ) -> error::Returns<bool> {
        self.reset_position();
        let scoreless = self.entering();
        // this list keeps no rack places, so two plies down generates whole
        self.generate_root(player_idx, false)?;
        if self.work_buffer.movegen_cut {
            return Ok(false);
        }
        let (valuation, after) = match play {
            movegen::Play::Exchange { .. } => {
                if self.pass_ends_the_line(scoreless) {
                    return Ok(self.both_pass_value(player_idx) > margin);
                }
                (equity::Equity::ZERO, Scoreless::Answering)
            }
            movegen::Play::Place { .. } => {
                let Some(candidate) = self.work_buffer.lists[0]
                    .iter()
                    .find(|candidate| candidate.play == *play)
                else {
                    return Ok(false);
                };
                if candidate.plays_out {
                    return Ok(candidate.valuation > margin);
                }
                let counted = Scoreless::Counted {
                    passes: 0,
                    zeros: 0,
                };
                (candidate.valuation, counted)
            }
        };
        self.place(player_idx, play);
        let work_buffer = &mut self.work_buffer;
        work_buffer.time_limit = limits.time;
        work_buffer.state_limit = limits.states;
        work_buffer.limit_memory(limits.memory);
        work_buffer.check_limits =
            limits.time.is_some() || limits.states.is_some() || limits.memory.is_some();
        // the move wins when its reply is worth less than valuation - margin
        let beta = valuation - margin;
        let point = equity::Equity::new(equity::SCALE);
        let mut wins = Ok(false);
        for depth in 1..COMPLETE {
            self.work_buffer.cut_short = false;
            match self.negamax_eval(1, player_idx ^ 1, depth, beta - point, beta, after) {
                Ok(searched) if searched.complete => {
                    wins = Ok(searched.value < beta);
                    break;
                }
                Ok(_) => {}
                Err(_) if self.work_buffer.cut_short => break,
                Err(e) => {
                    wins = Err(e);
                    break;
                }
            }
        }
        self.unplace(player_idx, play);
        wins
    }

    // every move of player_idx from the position to solve (each that places
    // tiles, and the pass), each with its value: its own, less its reply's, the
    // reply searched depth by depth with a full window on the table kept (warm
    // from a solve before it), till no line is cut short by the depth (exact)
    // or a limit stops it (the value of the last depth it finished, or none).
    // Best first, by value, then as the solve's root orders them. The limits
    // are on one clock for all of them
    pub fn value_first_moves(
        &mut self,
        player_idx: u8,
        limits: &Limits,
    ) -> error::Returns<Vec<FirstMove>> {
        self.start_clock(limits);
        self.reset_position();
        self.generate_root(player_idx, false)?;
        if self.work_buffer.movegen_cut {
            return Ok(Vec::new());
        }
        let work_buffer = &mut self.work_buffer;
        order(
            &mut work_buffer.lists[0],
            &mut work_buffer.orders[0],
            0,
            NO_PLAY,
        );
        let mut plays = (work_buffer.lists[0].iter())
            .map(|candidate| {
                (
                    candidate.play.clone(),
                    candidate.valuation,
                    candidate.plays_out,
                )
            })
            .collect::<Vec<_>>();
        if self.allows_pass(true) {
            let last_pass = self.pass_ends_the_line(self.entering());
            let valuation = if last_pass {
                self.both_pass_value(player_idx)
            } else {
                equity::Equity::ZERO
            };
            plays.push((
                movegen::Play::Exchange {
                    tiles: [][..].into(),
                },
                valuation,
                last_pass,
            ));
        }
        prioritize(&mut plays, self.root_priority, |candidate, play| {
            candidate.0 == *play
        });
        let mut valued = Vec::with_capacity(plays.len());
        for (play, valuation, ends) in plays {
            let (value, exact) = if ends {
                (Some(valuation), true)
            } else {
                self.first_move_value(player_idx, &play, valuation, limits)?
            };
            valued.push(FirstMove { play, value, exact });
        }
        // a sort that keeps the root's order among equals
        valued.sort_by_key(|first| std::cmp::Reverse(first.value));
        Ok(valued)
    }

    // the moves player_idx's root searches in the position to solve (those of
    // with_root_moves, where given), the pass last where it is one of them;
    // none when the memory limit cut their generation
    pub fn root_plays(&mut self, player_idx: u8) -> error::Returns<Option<Vec<movegen::Play>>> {
        self.reset_position();
        self.generate_root(player_idx, false)?;
        if self.work_buffer.movegen_cut {
            return Ok(None);
        }
        let mut plays = (self.work_buffer.lists[0].iter())
            .map(|candidate| candidate.play.clone())
            .collect::<Vec<_>>();
        if self.allows_pass(true) {
            plays.push(movegen::Play::Exchange {
                tiles: [][..].into(),
            });
        }
        Ok(Some(plays))
    }

    // the value of player_idx's move play from the position to solve, as
    // value_first_moves takes it, and whether it is exact
    #[inline]
    fn first_move_value(
        &mut self,
        player_idx: u8,
        play: &movegen::Play,
        valuation: equity::Equity,
        limits: &Limits,
    ) -> error::Returns<(Option<equity::Equity>, bool)> {
        self.reset_position();
        let after = match play {
            movegen::Play::Exchange { .. } => Scoreless::Answering,
            movegen::Play::Place { .. } => Scoreless::Counted {
                passes: 0,
                zeros: 0,
            },
        };
        self.work_buffer.at_root = false;
        self.place(player_idx, play);
        let work_buffer = &mut self.work_buffer;
        work_buffer.time_limit = limits.time;
        work_buffer.state_limit = limits.states;
        work_buffer.limit_memory(limits.memory);
        work_buffer.check_limits =
            limits.time.is_some() || limits.states.is_some() || limits.memory.is_some();
        let mut found = Ok((None, false));
        for depth in 1..COMPLETE {
            self.work_buffer.cut_short = false;
            match self.negamax_eval(
                1,
                player_idx ^ 1,
                depth,
                equity::Equity::NEG_INFINITY,
                equity::Equity::INFINITY,
                after,
            ) {
                Ok(searched) => {
                    found = Ok((Some(valuation - searched.value), searched.complete));
                    if searched.complete {
                        break;
                    }
                }
                Err(_) if self.work_buffer.cut_short => break,
                Err(e) => {
                    found = Err(e);
                    break;
                }
            }
        }
        self.unplace(player_idx, play);
        found
    }

    // based on https://en.wikipedia.org/wiki/Negamax
    // search the position for player_idx to move, depth plies on, within alpha
    // and beta, its moves at this ply's list; scoreless is the scoreless turns
    // before it, which say whether a pass now ends the line. A limit unwinds
    // the search (a timed solve's root search keeps it instead)
    fn negamax_eval(
        &mut self,
        ply: usize,
        player_idx: u8,
        depth: i8,
        alpha: equity::Equity,
        beta: equity::Equity,
        scoreless: Scoreless,
    ) -> error::Returns<Searched> {
        let base = self.work_buffer.frames.len();
        let node = Enter {
            ply,
            player_idx,
            depth,
            alpha,
            beta,
            scoreless,
        };
        let searched = self.search_from(base, Some((node, None)));
        if searched.is_err() {
            self.unwind(base);
        }
        searched
    }

    // the recursion of negamax_eval as a loop over the frames above base, from
    // the node to enter (whether it is a pass's root given when it resumes a
    // stopped search, else taken from at_root); a limit leaves the frames as
    // they stand and the node it stopped at pending
    fn search_from(
        &mut self,
        base: usize,
        mut enter: Option<(Enter, Option<bool>)>,
    ) -> error::Returns<Searched> {
        let mut value = None;
        loop {
            if let Some((node, root)) = enter.take() {
                value = self.enter_node(node, root)?;
            }
            if let Some(searched) = value.take() {
                if self.work_buffer.frames.len() == base {
                    return Ok(searched);
                }
                if let Some(again) = self.child_returned(searched) {
                    enter = Some((again, None));
                    continue;
                }
            }
            match self.advance_top()? {
                Advance::Enter(node) => enter = Some((node, None)),
                Advance::Done(searched) => value = Some(searched),
            }
        }
    }

    // a node entered: its value at once (at depth 0, or from the table), or its
    // frame pushed, its moves generated and ordered
    fn enter_node(&mut self, node: Enter, root: Option<bool>) -> error::Returns<Option<Searched>> {
        let Enter {
            ply,
            player_idx,
            depth,
            alpha,
            beta,
            scoreless,
        } = node;
        // a line cut short by the depth is valued as if both passed
        if depth == 0 {
            return Ok(Some(Searched {
                value: self.both_pass_value(player_idx),
                complete: false,
            }));
        }
        // a pass's root, the only node that records moves for a solve stopped
        // partway (the principal variation also searches from ply 0, never as a
        // root)
        let root = root.unwrap_or_else(|| std::mem::take(&mut self.work_buffer.at_root));

        if self.progress.is_some() {
            self.report(None)?;
        }
        self.work_buffer.searched += 1;

        // the table is looked at every node, the clock and the memory every few
        // nodes, the fewer the slower they come (time_or_memory_passed)
        if self.work_buffer.check_limits {
            let work_buffer = &mut self.work_buffer;
            work_buffer.nodes += 1;
            if work_buffer
                .state_limit
                .is_some_and(|limit| work_buffer.table.held >= limit)
                || (work_buffer.nodes >= work_buffer.next_look
                    && work_buffer.time_or_memory_passed())
            {
                work_buffer.cut_short = true;
                work_buffer.pending = Some((node, root));
                return Err(Box::new(LimitStopped));
            }
        }

        // a value stored deep enough answers when it is exact or a bound past
        // the window; its move goes first either way. A bound inside the window
        // does not narrow it, so what this search finds is bounded by the
        // window alone, and a full window always finds the exact value
        let key = if root {
            self.root_key(player_idx, scoreless)
        } else {
            self.node_key(player_idx, scoreless)
        };
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
                if root {
                    self.work_buffer.pass_best = entry.play;
                }
                return Ok(Some(Searched {
                    value: entry.value,
                    complete: entry.depth == COMPLETE,
                }));
            }
        }

        // the list stays at its ply while searched, for the ply two down to
        // reuse. Sort lazily: with a table move, all of them, that one first;
        // else the first only, and the rest once it has not cut off
        if root {
            self.generate_root(player_idx, depth >= 3)?;
        } else {
            self.generate(ply, player_idx, depth >= 3)?;
        }
        if self.work_buffer.movegen_cut {
            let work_buffer = &mut self.work_buffer;
            work_buffer.cut_short = true;
            work_buffer.pending = Some((node, root));
            return Err(Box::new(LimitStopped));
        }
        let table_first =
            table_play < PASS_PLAY && (table_play as usize) < self.work_buffer.orders[ply].len();
        let work_buffer = &mut self.work_buffer;
        {
            let (list, ordered) = (&mut work_buffer.lists[ply], &mut work_buffer.orders[ply]);
            if table_first {
                order(list, ordered, 0, table_play);
            } else {
                order_first(list, ordered);
            }
        }
        if depth == 1 && !work_buffer.lists[ply].is_empty() {
            return Ok(Some(self.search_last_ply(node, root, key, table_first)));
        }
        work_buffer
            .frames
            .push(Frame::new(node, root, key, table_first));
        Ok(None)
    }

    // the top frame's next step: the next move or the pass to search below it,
    // or, its moves and its pass done, its value stored and its frame popped
    fn advance_top(&mut self) -> error::Returns<Advance> {
        let t = self.work_buffer.frames.len() - 1;
        let frame = &self.work_buffer.frames[t];
        let Enter {
            ply,
            player_idx,
            depth,
            ..
        } = frame.node;
        if matches!(frame.child, Child::Moves) {
            let (root, table_first, mut next) = (frame.root, frame.table_first, frame.next);
            // use the reply's alternatives when there is only one first play.
            let split = self.work_buffer.threads > 1
                && (root
                    || (ply == 1
                        && self.work_buffer.frames[0].root
                        && self.work_buffer.orders[0].len() + usize::from(self.allows_pass(true))
                            == 1));
            let moves = self.work_buffer.orders[ply].len();
            while next < moves {
                let i = next;
                next += 1;
                let work_buffer = &mut self.work_buffer;
                if i == 1 && !table_first {
                    order(
                        &mut work_buffer.lists[ply],
                        &mut work_buffer.orders[ply][1..],
                        1,
                        NO_PLAY,
                    );
                }
                if split
                    && i == 1
                    && work_buffer.in_bag.is_none()
                    && work_buffer.time_limit.is_none()
                    && work_buffer.state_limit.is_none()
                    && work_buffer.verdicts.is_empty()
                {
                    self.split_verdicts(t, player_idx, depth)?;
                }
                let work_buffer = &mut self.work_buffer;
                let found = ordered_play_index(work_buffer.orders[ply][i]);
                let candidate = &work_buffer.lists[ply][found];
                let (index, plays_out, score) =
                    (candidate.index, candidate.plays_out, candidate.valuation);
                if root && i == 0 {
                    work_buffer.frames[t].first_is_best = index == work_buffer.root_best;
                }
                if plays_out {
                    if self.take_value(i, index, score) {
                        break;
                    }
                    continue;
                }
                if split && let Some(&Some((value, complete))) = work_buffer.verdicts.get(i) {
                    work_buffer.frames[t].best.complete &= complete;
                    self.take_value(i, index, value);
                    continue;
                }
                // the math goes like this:
                // child negamax returns v.
                // this parent negamax wants (score - v),
                // where alpha <= (score - v) <= beta.
                // so (score - beta) <= v <= (score - alpha).
                // since child_alpha <= v <= child_beta,
                // we set child_alpha = (score - beta)
                // and child_beta = (score - alpha).
                let play = &self.work_buffer.lists[ply][found].play;
                let skip_outs = ply > 0 && depth >= 2;
                let mut fills = zone::Squares::NONE;
                // read by the next two plies when they generate
                if depth >= 2 {
                    if skip_outs {
                        let dim = self.game_config.board_layout().dim();
                        let fp = zone::footprint_on_board(dim, &self.work_buffer.board_tiles, play);
                        self.work_buffer.zones[ply] = zone::zone_lanes_on_board(dim, &fp.zone);
                        fills = fp.fills;
                    } else {
                        self.work_buffer.zones[ply] = self.zone_lanes(play);
                    }
                    if let movegen::Play::Place { word, .. } = play {
                        self.work_buffer.spent[ply] =
                            rack_places(&self.work_buffer.racks[player_idx as usize], word, true);
                    }
                }
                // for a move that brings its player to a last tile, its score
                // plus one_tile_bound caps its value; a cap no higher than
                // alpha is used as the value in place of the reply's search (an
                // upper bound, as a fail low gives). Below the root only, where
                // depth is at least 2
                let Enter { alpha, beta, .. } = self.work_buffer.frames[t].node;
                // a remembered play out the move leaves alone caps it, as the
                // reply's search would take that play at that value; a cap no
                // higher than alpha is used as one_tile_bound's is
                if skip_outs
                    && let Some((k, cap)) =
                        self.out_cap(ply, player_idx, score, alpha, &fills, play)
                {
                    debug_assert!({
                        let play = play.clone();
                        self.place(player_idx, &play);
                        let checked = self.check_out_skip(ply, player_idx, k, score, cap);
                        self.unplace(player_idx, &play);
                        checked?
                    });
                    self.work_buffer.out_skips += 1;
                    if self.take_value(i, index, cap) {
                        break;
                    }
                    continue;
                }
                let play = play.clone();
                self.place(player_idx, &play);
                let bound = if ply > 0
                    && depth >= 2
                    && self.work_buffer.racks[player_idx as usize].len() == 1
                {
                    match self.one_tile_bound(ply, player_idx, alpha - score) {
                        Ok(bound) => score + bound,
                        Err(error) => {
                            self.unplace(player_idx, &play);
                            return Err(error);
                        }
                    }
                } else {
                    equity::Equity::INFINITY
                };
                if bound <= alpha {
                    self.unplace(player_idx, &play);
                    if self.take_value(i, index, bound) {
                        break;
                    }
                    continue;
                }
                // a move after the first is searched with a zero-width window
                // at alpha, which only says whether it beats alpha; one that
                // does without reaching beta is searched again with the whole
                // window (child_returned)
                let (child_alpha, child_beta) = (score - beta, score - alpha);
                let scout = i > 0 && child_alpha < child_beta - LEAST_STEP;
                if skip_outs {
                    self.enter_outs(ply);
                }
                let frame = &mut self.work_buffer.frames[t];
                frame.next = next;
                frame.child = Child::Move {
                    index,
                    score,
                    play,
                    scout,
                };
                return Ok(Advance::Enter(Enter {
                    ply: ply + 1,
                    player_idx: player_idx ^ 1,
                    depth: depth - 1,
                    alpha: if scout {
                        child_beta - LEAST_STEP
                    } else {
                        child_alpha
                    },
                    beta: child_beta,
                    scoreless: Scoreless::Counted {
                        passes: 0,
                        zeros: 0,
                    },
                }));
            }

            // the pass, unless a move placing tiles has already reached beta: it
            // ends the line when it answers a pass or brings the scoreless turns
            // to the config's count, each player losing their rack. The depth
            // stays the same across a forced pass (no move placing tiles)
            let pass = self.allows_pass(self.work_buffer.frames[t].root);
            let frame = &mut self.work_buffer.frames[t];
            frame.next = next;
            frame.child = Child::PassDone;
            if pass && frame.best.value < frame.node.beta {
                let (scoreless, alpha, beta) =
                    (frame.node.scoreless, frame.node.alpha, frame.node.beta);
                if self.pass_ends_the_line(scoreless) {
                    let value = self.both_pass_value(player_idx);
                    self.take_pass_value(value);
                } else {
                    self.work_buffer.zones[ply] = (0, 0);
                    self.work_buffer.spent[ply] = 0;
                    let forced = self.work_buffer.lists[ply].is_empty();
                    self.work_buffer.frames[t].child = Child::Pass;
                    return Ok(Advance::Enter(Enter {
                        ply: ply + 1,
                        player_idx: player_idx ^ 1,
                        depth: if forced { depth } else { depth - 1 },
                        alpha: -beta,
                        beta: -alpha,
                        scoreless: Scoreless::Answering,
                    }));
                }
            }
        }

        let frame = self.work_buffer.frames.pop().unwrap();
        self.finish(&frame);
        Ok(Advance::Done(frame.best))
    }

    // a move failing low at the first alpha never beats a later alpha, so its
    // bound stands in for its search
    fn split_verdicts(&mut self, frame: usize, player_idx: u8, depth: i8) -> error::Returns<()> {
        let work_buffer = &self.work_buffer;
        let Enter { alpha, ply, .. } = work_buffer.frames[frame].node;
        let orders = &work_buffer.orders[ply];
        let moves = (1..orders.len())
            .filter_map(|i| {
                let candidate = &work_buffer.lists[ply][ordered_play_index(orders[i])];
                (!candidate.plays_out).then(|| (i, candidate.play.clone(), candidate.valuation))
            })
            .collect::<Vec<_>>();
        let threads = work_buffer.threads.min(moves.len());
        if threads < 2 {
            return Ok(());
        }
        let limits = Limits {
            time: None,
            states: None,
            memory: work_buffer.memory_limit,
            clock: None,
        };
        let room = work_buffer.table.buckets.len() * std::mem::size_of::<[u64; 8]>();
        let mut kept = std::mem::take(&mut self.work_buffer.helper_tables);
        kept.resize_with(threads, || None);
        let mut helpers = std::mem::take(&mut self.helpers);
        helpers.resize_with(threads, || None);
        let room = limits
            .memory
            .map_or(room, |limit| room.min(limit.room() / threads));
        let (gc, kwg, arm_tables) = (self.game_config, self.kwg, self.arm_tables);
        let (run_letters, like) = (&self.run_letters, &*self.klv);
        let (board_tiles, racks, scoreless) = (
            &self.work_buffer.board_tiles,
            &self.work_buffer.racks,
            self.scoreless,
        );
        let (moves, limits) = (&moves, &limits);
        // each lane's solver, made at its first pass and kept for the next
        let lane = move |lane: usize, table: Option<KeptTable>, helper| {
            let mut found = Vec::new();
            let fresh = (Some(like), limits.memory);
            let Ok(mut own) = Self::of_helper(gc, kwg, arm_tables, run_letters, helper, fresh)
            else {
                return Ok((lane, found, table, None));
            };
            if let Some(table) = table {
                let _ = own.give_table(table);
            } else if let Some(_table) = reserve_in(limits.memory.as_ref(), room) {
                own.reserve_table(room);
            }
            own.board_tiles.clone_from(board_tiles);
            own.racks.clone_from(racks);
            own.scoreless = scoreless;
            own.begin_search(limits);
            own.reset_position();
            own.generate(0, player_idx, false)?;
            for (i, play, score) in moves.iter().skip(lane).step_by(threads) {
                own.place(player_idx, play);
                let child_beta = *score - alpha;
                let searched = own.negamax_eval(
                    1,
                    player_idx ^ 1,
                    depth - 1,
                    child_beta - LEAST_STEP,
                    child_beta,
                    Scoreless::Counted {
                        passes: 0,
                        zeros: 0,
                    },
                );
                own.unplace(player_idx, play);
                let searched = match searched {
                    Ok(searched) => searched,
                    Err(e) if e.is::<LimitStopped>() => break,
                    Err(e) => return Err(e),
                };
                let value = *score - searched.value;
                if value <= alpha {
                    found.push((*i, value, searched.complete));
                }
            }
            let table = own.take_table();
            Ok((lane, found, Some(table), Some(own.into_helper())))
        };
        let helped = std::thread::scope(|s| {
            let handles = (kept.into_iter().zip(helpers).enumerate())
                .map(|(k, (table, helper))| s.spawn(move || lane(k, table, helper)))
                .collect::<Vec<_>>();
            (handles.into_iter())
                .map(|handle| handle.join().unwrap())
                .collect::<Vec<_>>()
        });
        let work_buffer = &mut self.work_buffer;
        work_buffer
            .verdicts
            .resize(work_buffer.orders[ply].len(), None);
        for result in helped {
            let (_, found, table, helper) = result?;
            for (i, value, complete) in found {
                work_buffer.verdicts[i] = Some((value, complete));
            }
            work_buffer.helper_tables.push(table);
            self.helpers.push(helper);
        }
        Ok(())
    }

    // a node one ply from the depth with moves to search, searched in one go,
    // no frame pushed, as a limit is looked at only as a node is entered: each
    // reply, the pass's too, is valued as if both passed (as enter_node values
    // depth 0), by the rack scores alone, no move placed; its moves taken in
    // advance_top's order, and its entry stored as advance_top stores it
    #[inline(never)]
    fn search_last_ply(
        &mut self,
        node: Enter,
        root: bool,
        key: u64,
        table_first: bool,
    ) -> Searched {
        let mut frame = Frame::new(node, root, key, table_first);
        let Enter {
            ply, player_idx, ..
        } = node;
        for i in 0..self.work_buffer.orders[ply].len() {
            if i == 1 && !frame.table_first && frame.node.beta == equity::Equity::INFINITY {
                // nothing can cut: the best of the rest found by a scan, named
                // as the sort would have named it
                self.take_best_of_rest(&mut frame, player_idx, ply);
                break;
            }
            let work_buffer = &mut self.work_buffer;
            if i == 1 && !frame.table_first {
                order(
                    &mut work_buffer.lists[ply],
                    &mut work_buffer.orders[ply][1..],
                    1,
                    NO_PLAY,
                );
            }
            let found = ordered_play_index(work_buffer.orders[ply][i]);
            let candidate = &work_buffer.lists[ply][found];
            let (index, plays_out, score) =
                (candidate.index, candidate.plays_out, candidate.valuation);
            if frame.root && i == 0 {
                frame.first_is_best = index == work_buffer.root_best;
            }
            let value = if plays_out {
                score
            } else {
                frame.best.complete = false;
                score.finite_sub(self.both_pass_value_after(player_idx, ply, found))
            };
            if frame.take_value(&mut self.work_buffer.proven, i, index, value) {
                break;
            }
        }
        if self.allows_pass(root) && frame.best.value < frame.node.beta {
            let value = if self.pass_ends_the_line(frame.node.scoreless) {
                self.both_pass_value(player_idx)
            } else {
                self.work_buffer.zones[ply] = (0, 0);
                self.work_buffer.spent[ply] = 0;
                frame.best.complete = false;
                -self.both_pass_value(player_idx ^ 1)
            };
            frame.take_pass_value(&mut self.work_buffer.proven, value);
        }
        self.finish(&frame);
        frame.best
    }

    // search_last_ply's moves after its first, beta infinite: each valued, the
    // best taken (the first in order's among equals) with the index order
    // would give it, the others left unsorted
    #[inline(always)]
    fn take_best_of_rest(&mut self, frame: &mut Frame, player_idx: u8, ply: usize) {
        let work_buffer = &self.work_buffer;
        let (list, ordered) = (&work_buffer.lists[ply], &work_buffer.orders[ply]);
        let before = |a: u128, b: u128| {
            a >> 32 < b >> 32
                || a >> 32 == b >> 32
                    && movegen::cmp_play(
                        &list[ordered_play_index(a)].play,
                        &list[ordered_play_index(b)].play,
                    )
                    .is_lt()
        };
        let mut best: Option<(equity::Equity, u128)> = None;
        for &key in &ordered[1..] {
            let candidate = &list[ordered_play_index(key)];
            let value = if candidate.plays_out {
                candidate.valuation
            } else {
                frame.best.complete = false;
                let after = self.both_pass_value_after(player_idx, ply, ordered_play_index(key));
                candidate.valuation.finite_sub(after)
            };
            if best.is_none_or(|(v, b)| value > v || value == v && before(key, b)) {
                best = Some((value, key));
            }
        }
        let Some((value, key)) = best else {
            return;
        };
        let place = 1 + ordered[1..].iter().filter(|&&k| before(k, key)).count();
        let index = if place < PASS_PLAY as usize {
            place as u16
        } else {
            NO_PLAY
        };
        self.work_buffer.lists[ply][ordered_play_index(key)].index = index;
        frame.take_value(&mut self.work_buffer.proven, place, index, value);
    }

    // a node's search done: its value stored, and at a pass's root its move
    // named the pass's best
    #[inline(always)]
    fn finish(&mut self, frame: &Frame) {
        if frame.root {
            self.work_buffer.pass_best = frame.best_play;
        }
        self.work_buffer.store(
            frame.key,
            Entry {
                value: frame.best.value,
                play: frame.best_play,
                depth: if frame.best.complete {
                    COMPLETE
                } else {
                    frame.node.depth
                },
                bound: if frame.best.value <= frame.alpha_searched {
                    Bound::Upper
                } else if frame.best.value >= frame.node.beta {
                    Bound::Lower
                } else {
                    Bound::Exact
                },
            },
        );
    }

    // the top frame takes the value its child returned: a move's, unplaced
    // first, or the pass's; or, for a move its zero-width window found between
    // alpha and beta, the move's search again with the whole window, the move
    // left placed
    fn child_returned(&mut self, child: Searched) -> Option<Enter> {
        let frame = self.work_buffer.frames.last_mut().unwrap();
        if let Child::Move {
            score,
            scout: scout @ true,
            ..
        } = &mut frame.child
        {
            let value = score.finite_sub(child.value);
            if value > frame.node.alpha && value < frame.node.beta {
                *scout = false;
                let Enter {
                    ply,
                    player_idx,
                    depth,
                    alpha,
                    beta,
                    ..
                } = frame.node;
                return Some(Enter {
                    ply: ply + 1,
                    player_idx: player_idx ^ 1,
                    depth: depth - 1,
                    alpha: *score - beta,
                    beta: *score - alpha,
                    scoreless: Scoreless::Counted {
                        passes: 0,
                        zeros: 0,
                    },
                });
            }
        }
        frame.best.complete &= child.complete;
        match std::mem::replace(&mut frame.child, Child::Moves) {
            Child::Move {
                index, score, play, ..
            } => {
                let (mover, i) = (frame.node.player_idx, frame.next - 1);
                let (ply, depth) = (frame.node.ply, frame.node.depth);
                self.unplace(mover, &play);
                self.harvest_outs(ply, depth, &play);
                if self.take_value(i, index, score.finite_sub(child.value)) {
                    self.work_buffer.frames.last_mut().unwrap().next = usize::MAX;
                }
            }
            Child::Pass => {
                frame.child = Child::PassDone;
                self.take_pass_value(-child.value);
            }
            Child::Moves | Child::PassDone => unreachable!("a child returned to no search"),
        }
        None
    }

    // the top frame takes a move's value (Frame::take_value)
    #[inline(always)]
    fn take_value(&mut self, i: usize, index: u16, value: equity::Equity) -> bool {
        let work_buffer = &mut self.work_buffer;
        let frame = work_buffer.frames.last_mut().unwrap();
        frame.take_value(&mut work_buffer.proven, i, index, value)
    }

    // the top frame takes the pass's value
    #[inline(always)]
    fn take_pass_value(&mut self, value: equity::Equity) {
        let work_buffer = &mut self.work_buffer;
        let frame = work_buffer.frames.last_mut().unwrap();
        frame.take_pass_value(&mut work_buffer.proven, value);
    }

    // the frames above base unwound, each move placed for a child unplaced, as
    // an error unwound negamax_eval's recursion
    fn unwind(&mut self, base: usize) {
        while self.work_buffer.frames.len() > base {
            let frame = self.work_buffer.frames.pop().unwrap();
            if let Child::Move { play, .. } = frame.child {
                self.unplace(frame.node.player_idx, &play);
            }
        }
        self.work_buffer.pending = None;
    }

    // the principal variation of the deepest pass that finished: each move with
    // the value of the position it is played from, for its mover, read from the
    // table to the end of the line or as deep as that pass went; a position the
    // table cannot answer exactly is searched again with a full window, but
    // while a stopped pass waits, it ends the line, as a search would replace
    // entries the pass reads. A stopped solve's line starts with the move it
    // answers with. The line read last is read again while the position, the
    // mover, the deepest finished pass, and that move stay
    #[inline]
    pub fn collect_pv(
        &mut self,
        player_idx: u8,
        out: &mut Vec<(equity::Equity, movegen::Play)>,
    ) -> error::Returns<()> {
        let stopped = self.take_paused_depth();
        let search = stopped.is_none();
        let of = self.line_of(player_idx);
        let start = self.work_buffer.answer;
        let found = if (self.work_buffer.line.as_ref()).is_some_and(|line| {
            line.of == of && start.is_none_or(|start| line.start == Some(start))
        }) {
            Ok(())
        } else {
            self.keep_line(of, player_idx, search)
        };
        self.restore_paused_depth(stopped);
        found?;
        if let Some(line) = &self.work_buffer.line {
            out.clone_from(&line.plies);
        }
        Ok(())
    }

    // what a line of player_idx to move is read for: the key of the position to
    // solve, the mover, and the deepest finished pass
    #[inline]
    fn line_of(&mut self, player_idx: u8) -> (u64, u8, i8) {
        self.reset_position();
        let key = self.root_key(player_idx, self.entering());
        (key, player_idx, self.work_buffer.finished)
    }

    // the line read for of and kept, searching where the table cannot answer
    // only if search
    #[inline]
    fn keep_line(&mut self, of: (u64, u8, i8), player_idx: u8, search: bool) -> error::Returns<()> {
        let mut plies = (self.work_buffer.line.take()).map_or_else(Vec::new, |line| line.plies);
        let check_limits = std::mem::replace(&mut self.work_buffer.check_limits, false);
        let found = self.collect_pv_of(player_idx, &mut plies, search);
        self.work_buffer.check_limits = check_limits;
        let start = found?;
        self.work_buffer.line = Some(Box::new(Line { of, start, plies }));
        Ok(())
    }

    // the line, and the move and value it starts with
    #[inline]
    fn collect_pv_of(
        &mut self,
        player_idx: u8,
        out: &mut Vec<(equity::Equity, movegen::Play)>,
        search: bool,
    ) -> error::Returns<Option<(u16, equity::Equity)>> {
        out.clear();
        let mut start = None;
        self.reset_position();
        let mut player_idx = player_idx;
        let mut scoreless = self.entering();
        let mut depth = self.work_buffer.finished;
        let mut answer = self.work_buffer.answer;
        let mut restricted = self.root_moves.is_some();
        while depth > 0 {
            let key = if restricted {
                self.root_key(player_idx, scoreless)
            } else {
                self.node_key(player_idx, scoreless)
            };
            let fits = |entry: &Entry| {
                entry.bound == Bound::Exact && entry.depth >= depth && entry.play != NO_PLAY
            };
            let entry = match answer.take() {
                Some((play, value)) => Entry {
                    value,
                    play,
                    depth,
                    bound: Bound::Exact,
                },
                None => match self.work_buffer.table.probe(key).filter(fits) {
                    Some(entry) => entry,
                    None => {
                        if !search {
                            break;
                        }
                        self.work_buffer.verdicts.clear();
                        self.work_buffer.at_root = restricted;
                        match self.negamax_eval(
                            0,
                            player_idx,
                            depth,
                            equity::Equity::NEG_INFINITY,
                            equity::Equity::INFINITY,
                            scoreless,
                        ) {
                            Ok(_) => {}
                            Err(e) if e.is::<LimitStopped>() => break,
                            Err(e) => return Err(e),
                        }
                        match self.work_buffer.table.probe(key).filter(fits) {
                            Some(entry) => entry,
                            None => break,
                        }
                    }
                },
            };
            if start.is_none() {
                start = Some((entry.play, entry.value));
            }
            let value = entry.value;
            // a ply of depth, none for a forced pass, as in the search
            let mut cost = 1;
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
                if restricted {
                    self.generate_root(player_idx, false)?;
                } else {
                    self.generate(0, player_idx, false)?;
                }
                if self.work_buffer.lists[0].is_empty() {
                    cost = 0;
                }
                scoreless = Scoreless::Answering;
            } else {
                // the move the table keeps, generated again
                if restricted {
                    self.generate_root(player_idx, false)?;
                } else {
                    self.generate(0, player_idx, false)?;
                }
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
            restricted = false;
            player_idx ^= 1;
            depth -= cost;
        }
        Ok(start)
    }

    #[inline]
    pub fn print_best_line(&mut self, player_idx: u8) -> error::Returns<()> {
        let mut line = Vec::new();
        self.collect_pv(player_idx, &mut line)?;
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
                        movegen::Play::Place { score, .. } => score.raw(),
                    },
            ));
            writeln!(
                std::io::stdout(),
                "{:>2}: p{}: {}{:width$} {:>4} {}",
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
                })?,
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
                        board_tiles[strider.at_in_lane(i)] = tile;
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
                " e: p{}: {}",
                player_turn_idx,
                self.game_config.alphabet().fmt_rack(rack)
            )?;
            if let Some(leftover) = leftover {
                write!(
                    std::io::stdout(),
                    "{:width$} {:>4}",
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

    #[inline]
    fn report(&mut self, value: Option<equity::Equity>) -> error::Returns<()> {
        let work_buffer = &mut self.work_buffer;
        match self.progress.as_mut() {
            Some(progress_fn) => progress_fn(&Progress {
                endgames: work_buffer.in_bag,
                moves: work_buffer.in_bag_moves,
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
        self.work_buffer.clock = Limits::NONE.clock;
        let limits = BagLimits::new(&Limits::NONE, self.progress.take());
        let mut exact = true;
        let value = self.one_in_bag(
            player_idx,
            bag_tile,
            &limits,
            &mut exact,
            &mut BagTable::default(),
            self.scoreless,
        );
        self.progress = limits.progress.take();
        value
    }

    // an endgame inside a search with a tile in the bag, on that search's clock
    // and within its limits, to the depth of the search's pass, telling its
    // listener; one short of its end makes the search's answer inexact. Once
    // the search may stop, a limit that has passed, or that stops the endgame
    // short of the pass's depth, stops the search: an error, with stopped set
    #[inline(always)]
    fn solve_in_bag(
        &mut self,
        player_idx: u8,
        limits: &BagLimits,
        exact: &mut bool,
    ) -> error::Returns<equity::Equity> {
        self.solve_in_bag_below(player_idx, equity::Equity::INFINITY, limits, exact)
    }

    // as solve_in_bag, searched below beta alone (run_id_loop_below): a value
    // not below beta says only that the endgame is worth that much at least
    #[inline(always)]
    fn solve_in_bag_below(
        &mut self,
        player_idx: u8,
        beta: equity::Equity,
        limits: &BagLimits,
        exact: &mut bool,
    ) -> error::Returns<equity::Equity> {
        let may_stop = limits.may_stop.get();
        if may_stop && limits.passed(&self.work_buffer.table) {
            limits.stopped.set(true);
            return Err(Box::new(LimitStopped));
        }
        let work_buffer = &mut self.work_buffer;
        work_buffer.t0 = Some(limits.t0);
        work_buffer.dur_movegen = limits.dur_movegen.get();
        work_buffer.in_bag = Some(limits.endgames.get());
        work_buffer.in_bag_moves = limits.moves.get();
        self.progress = limits.progress.take();
        work_buffer.depth_cap = limits.depth.get();
        let solved = self.run_id_loop_below(
            player_idx,
            false,
            &Limits {
                time: limits.time,
                states: limits.states,
                memory: limits.memory,
                clock: None,
            },
            None,
            beta,
        );
        let work_buffer = &mut self.work_buffer;
        limits.dur_movegen.set(work_buffer.dur_movegen);
        limits.endgames.set(limits.endgames.get() + 1);
        limits.nodes.set(limits.nodes.get() + work_buffer.searched);
        limits.progress.set(self.progress.take());
        work_buffer.in_bag = None;
        work_buffer.in_bag_moves = None;
        work_buffer.depth_cap = COMPLETE;
        let solved = solved?;
        let memory = !solved.exact && solved.out_of_memory;
        limits
            .memory_stopped
            .set(limits.memory_stopped.get() | memory);
        if may_stop && !solved.exact && solved.depth < limits.depth.get() {
            limits.stopped.set(true);
            return Err(Box::new(LimitStopped));
        }
        *exact &= solved.exact;
        match solved.value {
            Some(value) => Ok(value),
            None => {
                limits.stopped.set(true);
                Err(Box::new(LimitStopped))
            }
        }
    }

    // swap tables with sub, to lend or take back; the values are for one
    // game_config, kwg, and arm_tables, so any other solver is an error
    #[inline(always)]
    fn swap_tables(&mut self, sub: &mut Self) -> error::Returns<()> {
        if !(std::ptr::eq(self.game_config, sub.game_config)
            && std::ptr::eq(self.kwg, sub.kwg)
            && std::ptr::eq(self.arm_tables, sub.arm_tables))
        {
            return Err("tables swap only between solvers of one game and lexicon".into());
        }
        std::mem::swap(&mut sub.work_buffer.table, &mut self.work_buffer.table);
        Ok(())
    }

    // swap lane pools with sub to lend one or take it back (LanePool)
    #[inline(always)]
    fn swap_lanes(&mut self, sub: &mut Self) {
        std::mem::swap(
            &mut sub.work_buffer.lane_pool,
            &mut self.work_buffer.lane_pool,
        );
    }

    // the table taken out, an empty one left in its place, for a caller that
    // keeps it apart from any solver between solves (a wasm32 page keeps it in
    // a handle, as it can keep no solver)
    #[inline(always)]
    pub fn take_table(&mut self) -> KeptTable {
        // the box the last table given back came in, holding the table it
        // displaced, made as new makes one (in place when it can be)
        let mut table =
            (self.work_buffer.spare_table.take()).unwrap_or_else(|| Box::new(Table::new()));
        table.renew();
        std::mem::swap(&mut self.work_buffer.table, &mut *table);
        KeptTable {
            stopped: self.take_paused_depth(),
            table,
            made_with: self.made_with(),
            line: self.work_buffer.line.take(),
            cadence: self.work_buffer.cadence,
        }
    }

    // a taken table given back, to this solver or another of the same
    // game_config, kwg, and arm_tables; any other is refused the table, which
    // comes back whole
    #[inline(always)]
    pub fn give_table(&mut self, kept: KeptTable) -> Result<(), KeptTable> {
        if kept.made_with != self.made_with() {
            return Err(kept);
        }
        let mut table = kept.table;
        std::mem::swap(&mut self.work_buffer.table, &mut *table);
        self.work_buffer.spare_table = Some(table);
        self.work_buffer.line = kept.line;
        self.work_buffer.cadence = kept.cadence;
        self.restore_paused_depth(kept.stopped);
        Ok(())
    }

    // a stopped pass taken out of the work buffer with the lists its frames
    // read, for a table taken or a line collected; none without one
    fn take_paused_depth(&mut self) -> Option<Box<Stopped>> {
        let work_buffer = &mut self.work_buffer;
        let pass = work_buffer.stopped.take()?;
        let pending = work_buffer.pending.take()?;
        Some(Box::new(Stopped {
            frames: std::mem::take(&mut work_buffer.frames),
            pending,
            pass,
            proven: work_buffer.proven,
            lists: std::mem::take(&mut work_buffer.lists),
            orders: std::mem::take(&mut work_buffer.orders),
            zones: std::mem::take(&mut work_buffer.zones),
            spent: std::mem::take(&mut work_buffer.spent),
            keeps: std::mem::take(&mut work_buffer.keeps),
            serials: std::mem::take(&mut work_buffer.serials),
            siblings: std::mem::take(&mut work_buffer.siblings),
            generated: work_buffer.generated,
        }))
    }

    // a stopped pass set aside put back, to be resumed
    fn restore_paused_depth(&mut self, stopped: Option<Box<Stopped>>) {
        let Some(stopped) = stopped else {
            return;
        };
        let work_buffer = &mut self.work_buffer;
        work_buffer.frames = stopped.frames;
        work_buffer.pending = Some(stopped.pending);
        work_buffer.stopped = Some(stopped.pass);
        work_buffer.proven = stopped.proven;
        work_buffer.lists = stopped.lists;
        work_buffer.orders = stopped.orders;
        work_buffer.zones = stopped.zones;
        work_buffer.spent = stopped.spent;
        work_buffer.keeps = stopped.keeps;
        work_buffer.serials = stopped.serials;
        work_buffer.siblings = stopped.siblings;
        work_buffer.generated = work_buffer.generated.max(stopped.generated);
        // its plays out were not set aside, and those held are of other lists
        work_buffer.outs.clear();
    }

    // the game_config, kwg, and arm_tables this solver was made with, by address
    #[inline(always)]
    fn made_with(&self) -> [usize; 3] {
        [
            std::ptr::from_ref(self.game_config).addr(),
            std::ptr::from_ref(self.kwg).addr(),
            std::ptr::from_ref(self.arm_tables).addr(),
        ]
    }

    // the one-in-bag value within the limits, the positions valued kept in memo;
    // an endgame inside it that a limit cut short clears exact
    fn one_in_bag(
        &mut self,
        player_idx: u8,
        bag_tile: u8,
        limits: &BagLimits,
        exact: &mut bool,
        memo: &mut BagTable<equity::Equity>,
        scoreless: [u8; 2],
    ) -> error::Returns<equity::Equity> {
        let gc = self.game_config;
        let kwg = self.kwg;
        let arm_tables = self.arm_tables;

        let (mut sub, mut mg) = self.take_both(limits)?;

        // the endgames keep what they learn in this solver's table, lent to the
        // one solving them, so each knows what the others found
        self.swap_tables(&mut sub)?;
        if !LEAN {
            self.swap_lanes(&mut sub);
        }
        let mut search = OneInBag {
            gc,
            kwg,
            arm_tables,
            klv: &self.klv,
            sub: &mut sub,
            mg: &mut mg,
            memo,
            limits,
            exact: true,
            line: None,
            placed: &mut Default::default(),
        };
        // where the game's last turn was a pass, the mover's pass answers it
        let [passes, zeros] = scoreless;
        let turn = OneInBagTurn {
            mover: player_idx,
            bag_tile,
            passes,
            zeros,
            answering: passes > 0,
        };
        let value = if one_in_bag_exchange_solvable(gc) {
            Self::one_in_bag_minimax_ex(&mut search, &self.board_tiles, &self.racks, turn)
        } else {
            Self::one_in_bag_minimax(&mut search, &self.board_tiles, &self.racks, turn)
        };
        *exact &= search.exact;
        self.swap_tables(&mut sub)?;
        if !LEAN {
            self.swap_lanes(&mut sub);
        }
        self.keep_sub(sub);
        self.kept_mg = Some(mg);
        value
    }

    // the one-in-bag value for the turn's mover, who places and draws the tile,
    // or passes; a pass answering a pass ends the game, as one bringing the
    // scoreless turns to the config's count does. With a pass line, a position
    // valued before answers at once, and one a limit stopped goes on at the
    // move it stopped at
    fn one_in_bag_minimax(
        search: &mut OneInBag<'_, 'a, N, L, LEAN, G, JUMPS, ANY_SIDE>,
        board: &[u8],
        racks: &[Vec<u8>; 2],
        turn: OneInBagTurn,
    ) -> error::Returns<equity::Equity> {
        let OneInBagTurn {
            mover,
            bag_tile,
            passes,
            zeros,
            answering,
        } = turn;
        let key = [bag_tile, mover, passes, zeros, u8::from(answering)];
        if let Some(&(value, exact)) = search.line.as_ref().and_then(|line| line.valued.get(&key)) {
            search.exact &= exact;
            return Ok(value);
        }
        let (start, mut best, exact_so_far) = (search.line.as_mut())
            .and_then(|line| line.partway.remove(&key))
            .unwrap_or((0, equity::Equity::NEG_INFINITY, true));
        // the position's own exactness, apart from that of the line before it
        let exact_before = std::mem::replace(&mut search.exact, exact_so_far);
        let value = Self::one_in_bag_minimax_from(search, board, racks, key, start, &mut best);
        match value {
            Ok(value) => {
                if let Some(line) = search.line.as_mut() {
                    line.valued.insert(key, (value, search.exact));
                }
                search.exact &= exact_before;
                Ok(value)
            }
            Err(e) => {
                search.exact = exact_before;
                Err(e)
            }
        }
    }

    // one_in_bag_minimax's search of a position (key: the tile in the bag, the
    // mover, the passes, the zeros, and whether it answers a pass), its places
    // from the start-th on, best the best so far; a limit that stops it leaves
    // where in the pass line
    fn one_in_bag_minimax_from(
        search: &mut OneInBag<'_, 'a, N, L, LEAN, G, JUMPS, ANY_SIDE>,
        board: &[u8],
        racks: &[Vec<u8>; 2],
        key: [u8; 5],
        start: usize,
        best: &mut equity::Equity,
    ) -> error::Returns<equity::Equity> {
        let [bag_tile, mover, passes, zeros, answering] = key;
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
        gen_raw_all::<LEAN, _, _, _, JUMPS, ANY_SIDE>(
            search.mg,
            &snapshot,
            &racks[mover as usize],
            0,
            movegen::PassPolicy::AsACandidate,
        )?;

        let drawn = bag_tile & !((bag_tile as i8) >> 7) as u8;

        // the places, numbered as generated, skipping those valued before
        let mut place = 0;
        for vm in &search.mg.plays {
            if let movegen::Play::Place {
                down,
                lane,
                idx,
                word,
                score,
            } = &vm.play
            {
                place += 1;
                if place <= start {
                    continue;
                }
                let Placed {
                    board: nb,
                    racks: nr,
                } = &mut *search.placed;
                nb.clear();
                nb.extend_from_slice(board);
                for (to, from) in nr.iter_mut().zip(racks) {
                    to.clear();
                    to.extend_from_slice(from);
                }
                {
                    let rack = &mut nr[mover as usize];
                    let strider = search.gc.board_layout().dim().lane(*down, *lane);
                    for (i, &tile) in (*idx..).zip(word.iter()) {
                        if tile != 0 {
                            nb[strider.at_in_lane(i)] = tile;
                            let blanked = tile & !((tile as i8) >> 7) as u8;
                            let p = rack.iter().rposition(|&t| t == blanked).unwrap();
                            rack[p] = 0x80;
                        }
                    }
                    rack.retain(|&t| t != 0x80);
                    rack.push(drawn);
                }

                search.sub.init(nb, [&nr[0], &nr[1]]);
                let v = match search
                    .sub
                    .solve_in_bag(opp, search.limits, &mut search.exact)
                {
                    Ok(v) => v,
                    Err(e) => return Err(Self::stopped_at(search, key, place - 1, *best, e)),
                };

                let val = *score - v;
                if val > *best {
                    *best = val;
                }
            }
        }

        let pass_passes = passes.saturating_add(1);
        let pass_zeros = zeros.saturating_add(1);
        let pass_val =
            if answering != 0 || scoreless_turns_end(search.gc, None, pass_passes, pass_zeros) {
                alphabet.scaled_rack_score(&racks[opp as usize])
                    - alphabet.scaled_rack_score(&racks[mover as usize])
            } else {
                match Self::one_in_bag_minimax(
                    search,
                    board,
                    racks,
                    OneInBagTurn {
                        mover: opp,
                        bag_tile,
                        passes: pass_passes,
                        zeros: pass_zeros,
                        answering: true,
                    },
                ) {
                    Ok(v) => -v,
                    Err(e) => return Err(Self::stopped_at(search, key, place, *best, e)),
                }
            };
        if pass_val > *best {
            *best = pass_val;
        }
        Ok(*best)
    }

    // the error a stop in a position of the pass line passes on, its place in
    // the line kept when a limit stopped it: the places valued, the best of
    // them, and whether all were exact
    #[inline(always)]
    fn stopped_at(
        search: &mut OneInBag<'_, 'a, N, L, LEAN, G, JUMPS, ANY_SIDE>,
        key: [u8; 5],
        valued: usize,
        best: equity::Equity,
        e: error::BoxAnyError,
    ) -> error::BoxAnyError {
        if search.limits.stopped.get()
            && let Some(line) = search.line.as_mut()
        {
            line.partway.insert(key, (valued, best, search.exact));
        }
        e
    }

    fn one_in_bag_minimax_ex(
        search: &mut OneInBag<'_, 'a, N, L, LEAN, G, JUMPS, ANY_SIDE>,
        board: &[u8],
        racks: &[Vec<u8>; 2],
        turn: OneInBagTurn,
    ) -> error::Returns<equity::Equity> {
        // a line of exchanges starts no endgame, the one other place a limit
        // is read, so the limits are read at each position, valued or not
        if search.limits.may_stop.get() && search.limits.passed(&search.sub.work_buffer.table) {
            search.limits.stopped.set(true);
            return Err(Box::new(LimitStopped));
        }
        let OneInBagTurn {
            mover,
            bag_tile,
            passes,
            zeros,
            answering,
        } = turn;
        let mut key = zobrist(BAG_KEYS | (u64::from(bag_tile) << 8) | 1);
        for (player, rack) in (0u8..).zip(racks) {
            key ^= rack_key(player, rack);
        }
        if mover != 0 {
            key ^= zobrist(SECOND_TO_MOVE);
        }
        key ^= Scoreless::Counted { passes, zeros }.key();
        if answering {
            key ^= Scoreless::Answering.key();
        }
        if let Some(&v) = search.memo.get(key) {
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
        gen_raw_all::<LEAN, _, _, _, JUMPS, ANY_SIDE>(
            search.mg,
            &snapshot,
            &racks[mover as usize],
            0,
            movegen::PassPolicy::AsACandidate,
        )?;

        let mut places: Vec<movegen::Play> = Vec::new();
        for vm in &search.mg.plays {
            if let movegen::Play::Place { .. } = &vm.play {
                places.push(vm.play.clone());
            }
        }

        let drawn = bag_tile & !((bag_tile as i8) >> 7) as u8;

        let mut best = equity::Equity::NEG_INFINITY;

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
                            nb[strider.at_in_lane(i)] = tile;
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
                let val = *score - v;
                if val > best {
                    best = val;
                }
            }
        }

        let pass_passes = passes.saturating_add(1);
        let pass_zeros = zeros.saturating_add(1);
        let pass_val = if answering || scoreless_turns_end(search.gc, None, pass_passes, pass_zeros)
        {
            alphabet.scaled_rack_score(&racks[opp as usize])
                - alphabet.scaled_rack_score(&racks[mover as usize])
        } else {
            -Self::one_in_bag_minimax_ex(
                search,
                board,
                racks,
                OneInBagTurn {
                    mover: opp,
                    bag_tile,
                    passes: pass_passes,
                    zeros: pass_zeros,
                    answering: true,
                },
            )?
        };
        if pass_val > best {
            best = pass_val;
        }

        let exch_zeros = zeros.saturating_add(1);
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
                alphabet.scaled_rack_score(&nr[opp as usize])
                    - alphabet.scaled_rack_score(&nr[mover as usize])
            } else {
                -Self::one_in_bag_minimax_ex(
                    search,
                    board,
                    &nr,
                    OneInBagTurn {
                        mover: opp,
                        bag_tile: r,
                        passes: 0,
                        zeros: exch_zeros,
                        answering: false,
                    },
                )?
            };
            if exch_val > best {
                best = exch_val;
            }
        }

        search.memo.keep(key, best, search.limits.memory, |_| 0);
        Ok(best)
    }

    // a search with tiles in the bag pools every tile off the board at its root
    // (the mover's rack and the unseen ones): each endgame inside it, whatever
    // the deal, takes its lanes' moves from the same lists (LanePool), each of
    // its threads' pools held to its share of the memory limit (POOL_SHARE)
    fn pool_lanes(
        &mut self,
        mover_rack: &[u8],
        unseen_tally: &[u8],
        memory_limit: Option<MemoryLimit>,
        threads: usize,
    ) {
        // the lean arm, whose builds hold little memory (the wasm's), pools
        // nothing: each lane is walked with the rack
        if LEAN {
            return;
        }
        // written in the buffer's own deal, as each solve of a search pools
        let mut deal = std::mem::take(&mut self.work_buffer.next_deal);
        deal.mover.clear();
        deal.mover.extend_from_slice(mover_rack);
        deal.mover.sort_unstable();
        deal.pool.clone_from(&deal.mover);
        for (tile, &count) in unseen_tally.iter().enumerate() {
            deal.pool
                .extend(std::iter::repeat_n(tile as u8, count as usize));
        }
        deal.pool.sort_unstable();
        deal.budget = memory_limit.map_or(usize::MAX, |limit| {
            limit.bytes / (POOL_SHARE * threads.max(1))
        });
        self.pool_lanes_as(&deal);
        self.work_buffer.next_deal = deal;
    }

    // the same pool as another solver's (its deal)
    fn pool_lanes_as(&mut self, deal: &Deal) {
        let most = self.game_config.rack_size() as usize;
        self.work_buffer.lane_pool.adopt_deal(deal, most);
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
        self.resume_peg_one_in_bag(
            &mut PegProgress::new(mover, board, mover_rack, unseen_tally, score_diff, [0, 0]),
            limits,
        )
    }

    // the search as resume_peg_one_in_bag, its places valued on this many
    // threads where it values them in one pass
    pub fn resume_peg_one_in_bag_on_threads(
        &mut self,
        progress: &mut PegProgress,
        limits: &Limits,
        threads: usize,
    ) -> error::Returns<Result<PegResult, PegUnsupported>> {
        let mut result = self.resume_peg_one_in_bag_best(progress, limits, threads)?;
        if let Ok(result) = &mut result {
            result.moves = progress.answer_moves(0..usize::MAX);
        }
        Ok(result)
    }

    // the search with a tile in the bag that progress holds, going on from
    // where the last solve given it stopped
    #[inline]
    pub fn resume_peg_one_in_bag(
        &mut self,
        progress: &mut PegProgress,
        limits: &Limits,
    ) -> error::Returns<Result<PegResult, PegUnsupported>> {
        self.resume_peg_one_in_bag_on_threads(progress, limits, 1)
    }

    // the search as resume_peg_one_in_bag_on_threads, its answer without the
    // moves (moves empty), which progress.answer_moves then lists as far as
    // asked
    pub fn resume_peg_one_in_bag_best(
        &mut self,
        progress: &mut PegProgress,
        limits: &Limits,
        threads: usize,
    ) -> error::Returns<Result<PegResult, PegUnsupported>> {
        self.work_buffer.threads = threads;
        let result = self.peg_one_in_bag_by(progress, limits);
        self.work_buffer.threads = 1;
        result
    }

    #[inline]
    fn peg_one_in_bag_by(
        &mut self,
        progress: &mut PegProgress,
        limits: &Limits,
    ) -> error::Returns<Result<PegResult, PegUnsupported>> {
        let gc = self.game_config;
        if progress.root_moves.is_some() && one_in_bag_exchange_legal(gc) {
            return Ok(Err(PegUnsupported::RootMovesNeedCommitted));
        }
        if one_in_bag_exchange_legal(gc) && !one_in_bag_exchange_solvable(gc) {
            return Ok(Err(PegUnsupported::ExchangeWithoutForcedEnd));
        }
        self.work_buffer.clock = limits.clock;
        let limits = BagLimits::new(limits, self.progress.take());

        // the search's lane pool, kept with its progress between solves
        std::mem::swap(&mut self.work_buffer.lane_pool, &mut progress.lanes);
        let memory = limits.memory;
        self.pool_lanes(&progress.mover_rack, &progress.unseen_tally, memory, 1);
        let result = if one_in_bag_exchange_legal(gc) {
            self.peg_clairvoyant_aggregate(progress, &limits)
        } else {
            self.peg_committed_no_exchange(progress, &limits)
        };
        std::mem::swap(&mut self.work_buffer.lane_pool, &mut progress.lanes);
        self.progress = limits.progress.take();
        progress.endgames += limits.endgames.get();
        progress.nodes += limits.nodes.get();
        let mut result = result?;
        result.out_of_memory = limits.out_of_memory();
        Ok(Ok(result))
    }

    // the search with any number of tiles in the bag: the mover commits to a
    // move not knowing which of the tiles they cannot see are in it, and each
    // move is valued over every split of those tiles into the opponent's rack
    // and the bag, weighed by the deals that give it, what each player knows
    // after it as the position's model says (PegModel, BagSearch). The answer
    // is the move with the most wins (a tie counted half), then the best mean
    // margin, ties going to the move found first (in_bag_moves's order)
    #[inline]
    pub fn solve_peg(
        &mut self,
        position: PegBagPosition,
        limits: &Limits,
        threads: usize,
    ) -> error::Returns<Result<PegBagResult, PegUnsupported>>
    where
        N: Sync,
    {
        self.resume_peg(
            &mut PegBagProgress::new(position, PegBagChoices::WHOLE)?,
            limits,
            threads,
        )
    }

    // the search with tiles in the bag that progress holds, going on from where
    // the last solve given it stopped, its moves valued on this many threads
    // (a wasm build runs one whatever is given)
    #[inline]
    pub fn resume_peg(
        &mut self,
        progress: &mut PegBagProgress,
        limits: &Limits,
        threads: usize,
    ) -> error::Returns<Result<PegBagResult, PegUnsupported>>
    where
        N: Sync,
    {
        let mut result = self.resume_peg_best(progress, limits, threads)?;
        if let Ok(result) = &mut result {
            result.moves = progress.answer_moves(0..usize::MAX);
        }
        Ok(result)
    }

    // resume and report the leader of each pass before the next one starts.
    pub fn resume_peg_reporting(
        &mut self,
        progress: &mut PegBagProgress,
        limits: &Limits,
        threads: usize,
        mut report: impl FnMut(PegBagPass) -> error::Returns<()>,
    ) -> error::Returns<Result<PegBagResult, PegUnsupported>>
    where
        N: Sync,
    {
        let mut result = self.resume_peg_best_with(progress, limits, threads, Some(&mut report))?;
        if let Ok(result) = &mut result {
            result.moves = progress.answer_moves(0..usize::MAX);
        }
        Ok(result)
    }

    // the search as resume_peg, its answer without the moves (moves empty),
    // which progress.answer_moves then lists as far as asked
    #[inline]
    pub fn resume_peg_best(
        &mut self,
        progress: &mut PegBagProgress,
        limits: &Limits,
        threads: usize,
    ) -> error::Returns<Result<PegBagResult, PegUnsupported>>
    where
        N: Sync,
    {
        self.resume_peg_best_with(progress, limits, threads, None)
    }

    fn resume_peg_best_with(
        &mut self,
        progress: &mut PegBagProgress,
        limits: &Limits,
        threads: usize,
        report: Option<&mut dyn FnMut(PegBagPass) -> error::Returns<()>>,
    ) -> error::Returns<Result<PegBagResult, PegUnsupported>>
    where
        N: Sync,
    {
        let gc = self.game_config;
        if exchanges_without_forced_end(gc, progress.position.bag_size) {
            return Ok(Err(PegUnsupported::ExchangeWithoutForcedEnd));
        }
        if gc.num_passes_to_end() == 0 && gc.num_zeros_to_end() == 0 {
            return Ok(Err(PegUnsupported::PassWithoutForcedEnd));
        }
        self.work_buffer.clock = limits.clock;
        let mut limits = BagLimits::new(limits, self.progress.take());
        limits.estimates = progress.estimates;
        limits.exact_pass = progress.exact_pass;
        let (mut sub, mut mg) = match self.take_both(&limits) {
            Ok(both) => both,
            Err(_) if limits.stopped.get() => {
                self.progress = limits.progress.take();
                let memory = !limits.other_passed(&self.work_buffer.table);
                return Ok(Ok(peg_bag_answer(&progress.state, memory)));
            }
            Err(e) => {
                self.progress = limits.progress.take();
                return Err(e);
            }
        };
        // the endgames keep what they learn in this solver's table, lent to the
        // one solving them, so each knows what the others found
        self.swap_tables(&mut sub)?;
        // the search's lane pool, kept with its progress between solves
        std::mem::swap(&mut sub.work_buffer.lane_pool, &mut progress.lanes);
        let position = &progress.position;
        // a wasm build has no threads (peg_bag_by)
        let threads = if cfg!(target_family = "wasm") {
            1
        } else {
            threads
        };
        let (rack, unseen) = (&position.mover_rack, &position.unseen_tally);
        sub.pool_lanes(rack, unseen, limits.memory, threads);
        let result = Self::peg_bag_by(
            &mut sub,
            &self.klv,
            &mut mg,
            progress,
            (self.root_priority, report),
            &limits,
            threads,
        );
        if progress.state.answer.is_none() {
            // the pass under way is the answer, and it may have valued more
            progress.state.rank_answer();
        }
        std::mem::swap(&mut sub.work_buffer.lane_pool, &mut progress.lanes);
        self.swap_tables(&mut sub)?;
        self.keep_sub(sub);
        self.kept_mg = Some(mg);
        self.progress = limits.progress.take();
        progress.endgames += limits.endgames.get();
        progress.nodes += limits.nodes.get();
        Ok(Ok(result?))
    }

    // a move's rows: each split valued again, at the depth of the pass the
    // answer is of, over what that pass valued below the moves (read through,
    // never written). Under a time or state limit, or none, on one thread,
    // they are the rows the search found
    #[inline]
    pub fn peg_bag_rows(
        &mut self,
        progress: &mut PegBagProgress,
        play: &movegen::Play,
        rows: std::ops::Range<usize>,
        limits: &Limits,
    ) -> error::Returns<Result<PegBagRows, PegBagRowsMissing>> {
        let state = &progress.state;
        let (values, depth) = match state.answer {
            Some((_, depth)) => (&state.answered, depth),
            None => (&state.valued, state.depth),
        };
        let valued = (state.moves.iter().zip(values.iter()))
            .any(|(other, value)| other == play && value.is_some());
        let Some(splits) = progress.splits.get(rows).filter(|_| valued) else {
            return Ok(Err(PegBagRowsMissing::Unvalued));
        };
        let mut limits = BagLimits::new(limits, None);
        limits.estimates = progress.estimates;
        limits.depth.set(depth);
        limits.may_stop.set(true);
        let gc = self.game_config;
        let (mut sub, mut mg) = match self.take_both(&limits) {
            Ok(both) => both,
            Err(_) if limits.stopped.get() => {
                return Ok(Err(PegBagRowsMissing::Stopped {
                    out_of_memory: true,
                }));
            }
            Err(e) => return Err(e),
        };
        self.swap_tables(&mut sub)?;
        std::mem::swap(&mut sub.work_buffer.lane_pool, &mut progress.lanes);
        let position = &progress.position;
        let arenas = &mut progress.arenas;
        let (rack, unseen) = (&position.mover_rack, &position.unseen_tally);
        sub.pool_lanes(rack, unseen, limits.memory, 1);
        let scoreless = scoreless_counts(position.scoreless);
        let at = |limits: &BagLimits, opp_rack: &[u8], bag: &[u8], pos: &mut InBag| {
            position.after_split(limits, scoreless, opp_rack, bag, pos)
        };
        let score_diff = f64::from(position.score_diff.raw());
        let state = &progress.state;
        let searched = if state.answer.is_none() || state.done {
            Some(&state.searched)
        } else {
            state.answered_searched.as_ref()
        };
        let Some(valued) = searched.map(BagValued::over) else {
            std::mem::swap(&mut sub.work_buffer.lane_pool, &mut progress.lanes);
            self.swap_tables(&mut sub)?;
            self.keep_sub(sub);
            self.kept_mg = Some(mg);
            return Ok(Err(PegBagRowsMissing::Stopped {
                out_of_memory: true,
            }));
        };
        let found = (|| {
            let tiles = gc.alphabet().len() as usize;
            BagArena::at_least(arenas, 1, tiles, position.bag_size, &limits)?;
            let mut search = BagSearch {
                gc,
                kwg: self.kwg,
                arm_tables: self.arm_tables,
                klv: &self.klv,
                sub: &mut sub,
                mg: &mut mg,
                valued: &valued,
                limits: &limits,
                model: position.model,
                exact: true,
                hand_overs: &[],
                hand_over: None,
                ply: 0,
                arena: &mut arenas[0],
            };
            let mut found = PegBagRows::default();
            for split in splits {
                let mut draws = Vec::new();
                let (value, win) =
                    value_root_split(&mut search, split, &at, play, score_diff, Some(&mut draws))?;
                let (_, bag, ways) = split;
                limits.room(&mut found.hypotheses, 1)?;
                limits.room(&mut found.wins, 1)?;
                limits.room(&mut found.draws, 1)?;
                let _tiles = limits.reserve(bag.iter().map(|&count| count as usize).sum())?;
                (found.hypotheses).push((tiles_of(bag), *ways, checked_expected_margin(value)?));
                found.wins.push(win);
                found.draws.push(draws);
            }
            Ok(found)
        })();
        std::mem::swap(&mut sub.work_buffer.lane_pool, &mut progress.lanes);
        self.swap_tables(&mut sub)?;
        self.keep_sub(sub);
        self.kept_mg = Some(mg);
        match found {
            Ok(found) => Ok(Ok(found)),
            Err(_) if limits.stopped.get() => Ok(Err(PegBagRowsMissing::Stopped {
                out_of_memory: limits.out_of_memory(),
            })),
            Err(e) => Err(e),
        }
    }

    // the search with tiles in the bag, its endgames solved by sub. Deepening,
    // the first pass values the placements by score, highest first, then the
    // other moves, and each later pass the moves best first by the pass before;
    // a limit may stop it anywhere, and the answer is the best move of the
    // deepest pass that valued every move, or, the first stopped, the best of
    // those it valued, none before it valued one. A pass keeps what it has
    // valued, so a solve resuming it goes on at the move it stopped at, and
    // the positions it valued below that move stay valued (state.searched)
    fn peg_bag_by(
        sub: &mut Self,
        klv: &klv::Klv<L>,
        mg: &mut movegen::KurniaMoveGenerator<G, JUMPS, ANY_SIDE>,
        progress: &mut PegBagProgress,
        (priority, mut report): PegBagWatch<'_, '_>,
        limits: &BagLimits,
        threads: usize,
    ) -> error::Returns<PegBagResult>
    where
        N: Sync,
    {
        let gc = sub.game_config;
        let kwg = sub.kwg;
        let arm_tables = sub.arm_tables;
        let deepens = limits.deepens();
        let PegBagProgress {
            position,
            splits,
            moves_at,
            part: _,
            estimates: _,
            root_moves: selected,
            exact_pass: _,
            steered,
            cuts,
            state,
            endgames: _,
            nodes: _,
            lanes: _,
            arenas,
        } = progress;
        let cuts = &cuts.unwrap_or(true);
        let tiles = gc.alphabet().len() as usize;
        match BagArena::at_least(arenas, 1, tiles, position.bag_size, limits) {
            Ok(()) => {}
            Err(_) if limits.stopped.get() => {
                return Ok(peg_bag_answer(state, limits.out_of_memory()));
            }
            Err(e) => return Err(e),
        }
        let scoreless = scoreless_counts(position.scoreless);
        let at = |limits: &BagLimits, opp_rack: &[u8], bag: &[u8], pos: &mut InBag| {
            position.after_split(limits, scoreless, opp_rack, bag, pos)
        };
        // a wasm build has no threads
        let threads = if cfg!(target_family = "wasm") {
            1
        } else {
            threads.max(1)
        };
        if state.moves.is_empty() {
            // the moves are the same whatever the split: they read only the
            // board, the mover's rack, and how many tiles the bag holds (so a
            // part's are made at the whole's first split)
            let (opp_rack, bag) = &*moves_at;
            // the root's lists, made once a search at their full lengths, so
            // no pass grows them. Where the memory limit has no room for them
            // the search stops before it starts, nothing kept, and the next
            // step tries again
            let lists = (|| -> error::Returns<_> {
                let mut pos = InBag::default();
                at(limits, opp_rack, bag, &mut pos)?;
                let moves = if let Some(selected) = selected {
                    let mut moves = list_in_room(limits, selected.plays.len())?;
                    for play in &selected.plays {
                        let _word = limits.reserve(play_word_len(play))?;
                        moves.push(play.clone());
                    }
                    moves
                } else {
                    root_moves::<LEAN, _, _, _, JUMPS, ANY_SIDE>(
                        gc, kwg, arm_tables, klv, &mut *mg, &pos, limits,
                    )?
                };
                let n = moves.len();
                Ok((
                    moves,
                    list_in_room(limits, n)?,
                    list_in_room(limits, n)?,
                    list_in_room(limits, n)?,
                    list_in_room(limits, n)?,
                    (list_in_room(limits, n)?, list_in_room(limits, n)?),
                ))
            })();
            let (moves, mut order, mut valued, answered, reached, ranks) = match lists {
                Ok(lists) => lists,
                // out of memory only when no other limit has passed, as passed
                // notes it
                Err(_) if limits.stopped.get() => {
                    return Ok(peg_bag_answer(
                        state,
                        !limits.other_passed(&sub.work_buffer.table),
                    ));
                }
                Err(e) => return Err(e),
            };
            order.extend(0..moves.len());
            if deepens || limits.exact_pass {
                order.sort_unstable_by_key(|&i| {
                    let score = match &moves[i] {
                        movegen::Play::Place { score, .. } => *score,
                        _ => equity::Equity::NEG_INFINITY,
                    };
                    (std::cmp::Reverse(score), i)
                });
            }
            valued.resize_with(moves.len(), || None);
            (state.moves, state.order, state.valued) = (moves, order, valued);
            (state.answered, state.reached) = (answered, reached);
            (state.ranked, state.places) = ranks;
            state.pass_exact = true;
            state.depth = if deepens { 1 } else { COMPLETE };
        }
        // what each move took in each split, which orders the threads' tasks:
        // kept only while the search runs on threads (a step may be given more
        // threads than the one before, set_threads)
        let cells = state.moves.len().checked_mul(splits.len());
        if threads > 1 && cells != Some(state.took.len()) {
            // none made, the next step tries again; none either where the
            // memory limit has no room for them (refused, the search stopped),
            // and then the threads' pass values none (value_on_threads)
            state.took = Vec::new();
            let room = match cells {
                None => Some(Reserved::NONE),
                Some(n) => (limits)
                    .reserve(n.saturating_mul(std::mem::size_of::<std::time::Duration>()))
                    .ok(),
            };
            if let Some(_cells) = room {
                state.took = cells_of(cells, std::time::Duration::ZERO)?;
            }
        }
        let score_diff = f64::from(position.score_diff.raw());
        if state.searched.shards.is_empty() {
            state.searched = BagValued::new(threads);
        }
        if !steered.is_empty() && state.standing.len() != state.moves.len() {
            state.stand(steered);
        }

        while !state.done {
            if !state.standing.is_empty() && !state.steer_order() {
                return Ok(peg_bag_answer(state, limits.out_of_memory()));
            }
            prioritize(&mut state.order[state.next..], priority, |&i, play| {
                state.moves[i] == *play
            });
            limits.depth.set(state.depth);
            let stopped = if threads > 1 {
                Self::value_on_threads(
                    sub,
                    klv,
                    &mut *mg,
                    state,
                    PegBagRoot {
                        splits,
                        at: &at,
                        score_diff,
                        arenas: &mut *arenas,
                        model: position.model,
                        cuts: *cuts,
                    },
                    limits,
                    threads,
                )?
            } else {
                let mut stopped = false;
                let arena = &mut arenas[0];
                let mut beat = (*cuts && !state.reached.is_empty()).then(|| {
                    let best = peg_bag_best(&state.reached, &state.valued);
                    halves_down(state.valued[best].as_ref().unwrap().win, splits)
                });
                while let Some(&i) = state.order.get(state.next) {
                    limits.moves.set(Some((state.next, state.order.len())));
                    limits.may_stop.set(deepens || limits.exact_pass);
                    let mut search = BagSearch {
                        gc,
                        kwg,
                        arm_tables,
                        klv,
                        sub: &mut *sub,
                        mg: &mut *mg,
                        valued: &state.searched,
                        limits,
                        model: position.model,
                        exact: true,
                        hand_overs: &[],
                        hand_over: None,
                        ply: 0,
                        arena: &mut *arena,
                    };
                    let play = &state.moves[i];
                    let order = if beat.is_some() {
                        &state.split_order[..]
                    } else {
                        &[]
                    };
                    let found = &mut state.split_found;
                    match value_root_move(
                        &mut search,
                        splits,
                        &at,
                        play,
                        (score_diff, beat),
                        order,
                        found,
                    ) {
                        Ok(value) => {
                            if *cuts {
                                let halves = halves_down(value.win, splits);
                                beat = Some(beat.map_or(halves, |beat| beat.max(halves)));
                                if state.split_order.is_empty() && !value.cut {
                                    let found = &state.split_found;
                                    if let Ok(mut order) = list_in_room(limits, splits.len()) {
                                        order.extend(0..splits.len());
                                        order.sort_by(|&a, &b| {
                                            found[a].1.total_cmp(&found[b].1).then(a.cmp(&b))
                                        });
                                        state.split_order = order;
                                    }
                                }
                            }
                            state.pass_exact &= value.exact;
                            state.valued[i] = Some(value);
                            state.reached.push(i);
                            state.next += 1;
                        }
                        Err(_) if limits.stopped.get() => {
                            stopped = true;
                            break;
                        }
                        Err(e) => return Err(e),
                    }
                }
                stopped
            };
            if stopped {
                if state.answer.is_none() {
                    // no pass valued every move: the best of those valued, if
                    // any
                    return Ok(peg_bag_answer(state, limits.out_of_memory()));
                }
                break;
            }
            let best = peg_bag_best(&state.reached, &state.valued);
            state.answer = Some((best, state.depth));
            let done = state.pass_exact || state.depth == COMPLETE;
            if !done {
                // the next pass values the moves best first by this one,
                // afresh; each list keeps its room
                std::mem::swap(&mut state.order, &mut state.reached);
                state.reached.clear();
                let valued = &state.valued;
                (state.order).sort_unstable_by(|&a, &b| peg_bag_ranks(valued, a, b));
            }
            if !state.standing.is_empty() {
                state.carry_paused();
            }
            std::mem::swap(&mut state.answered, &mut state.valued);
            state.valued.clear();
            state.valued.resize_with(state.answered.len(), || None);
            state.rank_answer();
            if done {
                state.done = true;
                break;
            }
            state.next = 0;
            state.split_order.clear();
            state.pass_exact = true;
            let (answered, copies) = state.searched.shared(limits.memory).unzip();
            state.answered_searched = answered;
            state.searched.deepen();
            drop(copies);
            state.depth += 1;
            if let Some(report) = report.as_mut() {
                let value = state.answered[best].as_ref().unwrap();
                report(PegBagPass {
                    depth: state.depth - 1,
                    moves: state.moves.len(),
                    leader: PegBagMove {
                        play: state.moves[best].clone(),
                        win_pct: value.win,
                        expected_margin: Some(value.expected_margin),
                        exact: value.exact,
                        cut: false,
                    },
                })?;
            }
        }
        if state.answer.is_none() {
            unreachable!("the first move of the first pass is always valued");
        }
        Ok(peg_bag_answer(state, limits.out_of_memory()))
    }

    // a pass's moves from state.next on, valued on threads. Each thread takes
    // the next move and split not yet taken, the costliest by the pass before
    // first: a costly move's splits spread over the threads, and none is begun
    // as the others end. This thread works with sub (its listener and the table
    // lent to it); each other one works with a solver of its own, given the
    // table it left the pass before (state.tables). They run on one clock,
    // sharing what they value below the moves (state.searched). Whether a limit
    // stopped the pass; the moves it valued in every split then go first in the
    // order, so a solve resuming the pass goes on after them
    fn value_on_threads<At: Fn(&BagLimits, &[u8], &[u8], &mut InBag) -> error::Returns<()> + Sync>(
        sub: &mut Self,
        klv: &klv::Klv<L>,
        mg: &mut movegen::KurniaMoveGenerator<G, JUMPS, ANY_SIDE>,
        state: &mut PegBagState,
        root: PegBagRoot<'_, At>,
        limits: &BagLimits,
        threads: usize,
    ) -> error::Returns<bool>
    where
        N: Sync,
    {
        use std::sync::atomic::{
            AtomicBool, AtomicU64, AtomicUsize,
            Ordering::{AcqRel, Relaxed},
        };
        let PegBagRoot {
            splits,
            at,
            score_diff,
            arenas,
            model,
            cuts,
        } = root;
        let gc = sub.game_config;
        let kwg = sub.kwg;
        let arm_tables = sub.arm_tables;
        let run_letters = &sub.run_letters.clone();
        let may_stop = limits.deepens() || limits.exact_pass;
        let base = state.next;
        let rest = state.order.len() - base;
        let num_splits = splits.len();
        // each thread's room to work in (BagArena), this one's first, sized
        // for the tiles of the game and of the bag
        let tiles = gc.alphabet().len() as usize;
        let bag_len = (splits.first()).map_or(0, |(_, bag, _)| {
            bag.iter().map(|&count| count as usize).sum()
        });
        match BagArena::at_least(arenas, threads, tiles, bag_len, limits) {
            Ok(()) => {}
            Err(_) if limits.stopped.get() => {
                return Ok(true); // stopped, nothing valued
            }
            Err(e) => return Err(e),
        }
        let (mine, theirs) = arenas.split_at_mut(1);
        let tasks = rest * num_splits;
        // what each move took, none where the memory limit had no room for it
        if state.took.len() != state.moves.len() * num_splits {
            return Ok(true); // stopped, nothing valued
        }
        // the tasks' order, what each found, and their hand-overs; the moves'
        // splits not yet valued: what each found is kept reserved through the
        // pass, as it is made after it
        type Found = Option<(f64, f64, bool)>;
        let Ok(mut lists) = limits.reserve(
            tasks
                * (std::mem::size_of::<usize>()
                    + std::mem::size_of::<Found>()
                    + std::mem::size_of::<HandOver>())
                + rest
                    * (std::mem::size_of::<AtomicUsize>()
                        + 2 * std::mem::size_of::<AtomicU64>()
                        + std::mem::size_of::<AtomicBool>()),
        ) else {
            return Ok(true); // stopped, nothing valued
        };
        let next = AtomicUsize::new(0);
        // the tasks (each a move's place in the rest of the order and a split),
        // the costliest by the pass before first, ties in the order
        let mut taking = cells_of(Some(tasks), 0usize)?;
        (taking.iter_mut().enumerate()).for_each(|(task, at)| *at = task);
        taking.sort_by_key(|&task| {
            let i = state.order[base + task / num_splits];
            std::cmp::Reverse(state.took[i * num_splits + task % num_splits])
        });
        // each move's splits not yet valued, and the moves valued in every one
        let unvalued = (0..rest)
            .map(|_| AtomicUsize::new(num_splits))
            .collect::<Vec<_>>();
        let moves_valued = AtomicUsize::new(state.reached.len());
        let all = splits.iter().map(|split| split.2).sum::<u64>().max(1);
        state.lost.clear();
        state.lost.resize_with(rest, || AtomicU64::new(0));
        state.won.clear();
        state.won.resize_with(rest, || AtomicU64::new(0));
        state.cut.clear();
        state.cut.resize_with(rest, || AtomicBool::new(false));
        let (lost, won, cut) = (&state.lost[..], &state.won[..], &state.cut[..]);
        let beat = AtomicU64::new(if cuts && !state.reached.is_empty() {
            let best = peg_bag_best(&state.reached, &state.valued);
            halves_down(state.valued[best].as_ref().unwrap().win, splits)
        } else {
            0
        });
        let stop = AtomicBool::new(false);
        let refused = AtomicBool::new(false); // any thread's memory limit stopped it
        let memory_stopped = AtomicBool::new(false);
        let on_one_clock = limits.on_one_clock();
        let (order, moves) = (&state.order, &state.moves);
        // each task's hand-over (HandOver), and how many tasks that hand the
        // turn over are still being valued
        let hand_overs = (0..tasks).map(|_| HandOver::new()).collect::<Vec<_>>();
        lists.keep(tasks * std::mem::size_of::<Found>());
        let handing = AtomicUsize::new(0);
        // a thread's share: each move and split it valued (by its task, the
        // move's place in the rest of the order times the splits, plus the
        // split's), with the value, the chance to win, whether it was exact, and
        // the time it took; and whether a limit stopped it
        type Share = (Vec<(usize, f64, f64, bool, std::time::Duration)>, bool);
        let stopped_by = |limits: &BagLimits| {
            stop.store(true, Relaxed);
            refused.fetch_or(limits.refused.get(), Relaxed);
            memory_stopped.fetch_or(limits.memory_stopped.get(), Relaxed);
        };
        let work = |search: &mut BagSearch<'_, '_, N, L, LEAN, G, JUMPS, ANY_SIDE>,
                    limits: &BagLimits|
         -> error::Returns<Share> {
            let mut mine = Vec::new();
            // the margins of a move helped with, kept from one to the next
            let mut margins = Vec::new();
            while !stop.load(Relaxed) {
                let Some(&task) = taking.get(next.fetch_add(1, Relaxed)) else {
                    break;
                };
                let (m, s) = (task / num_splits, task % num_splits);
                if cuts {
                    let most = 2 * all - lost[m].load(Relaxed);
                    if cut[m].load(Relaxed) || most < beat.load(Relaxed) {
                        cut[m].store(true, Relaxed);
                        continue;
                    }
                }
                limits
                    .moves
                    .set(Some((moves_valued.load(Relaxed), order.len())));
                limits.may_stop.set(may_stop);
                search.exact = true;
                let play = &moves[order[base + m]];
                let began = limits.t0.elapsed();
                let hands_over = matches!(play, movegen::Play::Exchange { .. });
                if hands_over {
                    handing.fetch_add(1, Relaxed);
                    search.hand_over = Some(task);
                }
                let valued = value_root_split(search, &splits[s], at, play, score_diff, None);
                if hands_over {
                    search.hand_over = None;
                    handing.fetch_sub(1, Relaxed);
                }
                match valued.and_then(|found| limits.room(&mut mine, 1).map(|()| found)) {
                    Ok((value, win)) => {
                        let took = limits.t0.elapsed().saturating_sub(began);
                        mine.push((task, value, win, search.exact, took));
                        let ways = splits[s].2;
                        lost[m].fetch_add(2 * ways - halves_up(win, ways), Relaxed);
                        let down = (win * (2 * ways) as f64).floor() as u64;
                        won[m].fetch_add(down, Relaxed);
                        if unvalued[m].fetch_sub(1, AcqRel) == 1 {
                            moves_valued.fetch_add(1, Relaxed);
                            if cuts && !cut[m].load(Relaxed) {
                                beat.fetch_max(won[m].load(Relaxed), Relaxed);
                            }
                        }
                    }
                    Err(_) if limits.stopped.get() => {
                        stopped_by(limits);
                        return Ok((mine, true));
                    }
                    Err(e) => {
                        stop.store(true, Relaxed);
                        return Err(e);
                    }
                }
            }
            // no task left: while a task that hands the turn over is valued,
            // help the one with the most moves left, from the end
            while !stop.load(Relaxed) {
                let open = (hand_overs.iter())
                    .filter_map(|help| {
                        let work = help.opened()?;
                        let back = help.back.load(Relaxed);
                        let left = back.saturating_sub(help.forward.load(Relaxed) + 1);
                        (left > 0).then_some((help, work, left))
                    })
                    .max_by_key(|(_, _, left)| *left);
                let Some((help, work, _)) = open else {
                    if handing.load(Relaxed) == 0 {
                        break;
                    }
                    std::thread::sleep(std::time::Duration::from_millis(1));
                    continue;
                };
                let Some(j) = help.take_back() else {
                    continue;
                };
                let floor = help.floor.load(Relaxed);
                let floor = (floor != NO_FLOOR).then(|| f64::from_bits(floor));
                let (info, work, splits) = &*work;
                // the info set a root move handed over is a move below the root
                search.ply = 1;
                margins.clear();
                let helped = search.info_move(info, &work[j], splits, floor, &mut margins);
                search.ply = 0;
                match helped {
                    Ok(_) => {}
                    Err(_) if limits.stopped.get() => {
                        stopped_by(limits);
                        return Ok((mine, true));
                    }
                    Err(e) => {
                        stop.store(true, Relaxed);
                        return Err(e);
                    }
                }
            }
            memory_stopped.fetch_or(limits.memory_stopped.get(), Relaxed);
            Ok((mine, false))
        };
        let valued = &state.searched;
        let hand_overs = &hand_overs[..];
        let memory = limits.memory;
        let others = threads - 1;
        let pool = &sub.work_buffer.lane_pool.deal;
        let Ok(_others) = limits.reserve(
            pool.pool.len()
                + pool.mover.len()
                + growth(&state.tables, others.saturating_sub(state.tables.len()))
                + growth(&state.lanes, others.saturating_sub(state.lanes.len()))
                + growth(&sub.helpers, others.saturating_sub(sub.helpers.len())),
        ) else {
            return Ok(true); // stopped, nothing valued
        };
        let deal = &sub.work_buffer.lane_pool.deal.clone();
        let (other_endgames, other_nodes) = (&AtomicU64::new(0), &AtomicU64::new(0));
        // each other thread's table and lane pool as the pass before left them
        let mut kept = std::mem::take(&mut state.tables);
        kept.resize_with(threads - 1, || None);
        let mut pools = std::mem::take(&mut state.lanes);
        pools.resize_with(threads - 1, LanePool::new);
        // and its solver and generator, made at its first pass and kept
        let mut helpers = std::mem::take(&mut sub.helpers);
        helpers.resize_with(threads - 1, || None);
        let (shares, kept, pools, helpers) = std::thread::scope(|s| {
            let handles = (kept
                .into_iter()
                .zip(pools)
                .zip(helpers)
                .zip(theirs.iter_mut()))
            .map(|(((table, lanes), helper), arena)| {
                let work = &work;
                s.spawn(move || {
                    let fresh = (Some(klv), memory);
                    let own = Self::of_helper(gc, kwg, arm_tables, run_letters, helper, fresh);
                    let mut own = match own {
                        Ok(own) => own,
                        Err(e) => return (Err(e), table, lanes, None),
                    };
                    if let Some(table) = table {
                        // made by a solver of this game and lexicon, so taken
                        let _ = own.give_table(table);
                    }
                    own.work_buffer.lane_pool = lanes;
                    if !LEAN {
                        own.pool_lanes_as(deal);
                    }
                    let limits = BagLimits::on_the_clock(on_one_clock);
                    let mut mg = match own.take_mg(&limits) {
                        Ok(mg) => mg,
                        Err(e) => {
                            let lanes = std::mem::take(&mut own.work_buffer.lane_pool);
                            let table = own.take_table();
                            return (Err(e), Some(table), lanes, Some(own.into_helper()));
                        }
                    };
                    let mut search = BagSearch {
                        gc,
                        kwg,
                        arm_tables,
                        klv,
                        sub: &mut own,
                        mg: &mut mg,
                        valued,
                        limits: &limits,
                        model,
                        exact: true,
                        hand_overs,
                        hand_over: None,
                        ply: 0,
                        arena,
                    };
                    let share = work(&mut search, &limits);
                    other_endgames.fetch_add(limits.endgames.get(), Relaxed);
                    other_nodes.fetch_add(limits.nodes.get(), Relaxed);
                    own.kept_mg = Some(mg);
                    let lanes = std::mem::take(&mut own.work_buffer.lane_pool);
                    let table = own.take_table();
                    (share, Some(table), lanes, Some(own.into_helper()))
                })
            })
            .collect::<Vec<_>>();
            let mut search = BagSearch {
                gc,
                kwg,
                arm_tables,
                klv,
                sub: &mut *sub,
                mg,
                valued,
                limits,
                model,
                exact: true,
                hand_overs,
                hand_over: None,
                ply: 0,
                arena: &mut mine[0],
            };
            let mine = work(&mut search, limits);
            let (mut shares, mut kept, mut pools, mut helpers) =
                (vec![mine], Vec::new(), Vec::new(), Vec::new());
            for handle in handles {
                let (share, table, lanes, helper) = handle.join().unwrap();
                shares.push(share);
                kept.push(table);
                pools.push(lanes);
                helpers.push(helper);
            }
            (shares, kept, pools, helpers)
        });
        state.tables = kept;
        state.lanes = pools;
        sub.helpers = helpers;
        state.others.0 += other_endgames.load(Relaxed);
        state.others.1 += other_nodes.load(Relaxed);
        if refused.load(Relaxed) {
            limits.refused.set(true);
        }
        if memory_stopped.load(Relaxed) {
            limits.memory_stopped.set(true);
        }
        let mut stopped = false;
        let mut found = cells_of(Some(tasks), None)?;
        drop(lists);
        for share in shares {
            let (mine, stopped_here) = share?;
            stopped |= stopped_here;
            for (task, value, win, exact, took) in mine {
                found[task] = Some((value, win, exact));
                let i = state.order[base + task / num_splits];
                state.took[i * num_splits + task % num_splits] = took;
            }
        }
        let (mut taken, mut left) = (Vec::new(), Vec::new());
        for (m, per_split) in found.chunks_mut(num_splits.max(1)).enumerate() {
            let i = state.order[base + m];
            if cut[m].load(Relaxed) && !per_split.iter().all(Option::is_some) {
                let exact =
                    (per_split.iter()).all(|found| found.as_ref().is_none_or(|found| found.2));
                let (mut win, mut marg, mut total) = (0.0, 0.0, 0u64);
                for ((_, _, ways), found) in splits.iter().zip(per_split.iter()) {
                    if let Some((value, chance, _)) = found {
                        win += *ways as f64 * chance;
                        marg += *ways as f64 * value;
                        total += ways;
                    }
                }
                let most = (win + (all - total) as f64) / all as f64;
                let mean = peg_bag_value(win, marg, total, exact)?;
                state.valued[i] = Some(PegBagValue {
                    win: most,
                    cut: true,
                    ..mean
                });
                state.pass_exact &= exact;
                state.reached.push(i);
                taken.push(i);
            } else if per_split.iter().all(Option::is_some) {
                let exact = (per_split.iter()).all(|found| found.as_ref().unwrap().2);
                let (mut win, mut marg, mut total) = (0.0, 0.0, 0u64);
                for ((_, _, ways), found) in splits.iter().zip(per_split.iter()) {
                    let (value, chance, _) = found.unwrap();
                    win += *ways as f64 * chance;
                    marg += *ways as f64 * value;
                    total += ways;
                }
                let value = peg_bag_value(win, marg, total, exact)?;
                state.pass_exact &= value.exact;
                state.valued[i] = Some(value);
                state.reached.push(i);
                taken.push(i);
            } else {
                left.push(i);
            }
        }
        state.order.truncate(base);
        state.next += taken.len();
        state.order.extend(taken);
        state.order.extend(left);
        Ok(stopped)
    }

    // the clairvoyant model's search: each tile's value, the mover knowing it.
    // Deepening, it answers from the deepest pass that valued every tile, none
    // before the first did; a limit may stop it anywhere, and a solve resuming
    // it goes on with the pass under way, the positions it valued kept
    // (exchange_memo)
    #[inline]
    fn peg_clairvoyant_aggregate(
        &mut self,
        progress: &mut PegProgress,
        limits: &BagLimits,
    ) -> error::Returns<PegResult> {
        let mover = progress.mover;
        let board = &progress.board[..];
        let mover_rack = &progress.mover_rack[..];
        let unseen_tally = &progress.unseen_tally[..];
        let score_diff = i64::from(progress.score_diff.raw());
        let exchange_memo = &mut progress.exchange_memo;
        // each tile's value to the pass's depth, from the positions the pass
        // has valued already
        let scoreless = progress.scoreless;
        let mut pass = |this: &mut Self| -> error::Returns<(PegResult, Vec<equity::Equity>)> {
            if exchange_memo.0 != limits.depth.get() {
                *exchange_memo = (limits.depth.get(), BagTable::default());
            }
            let memo = &mut exchange_memo.1;
            let mut outcomes = PegOutcomes::default();
            let mut run = vec![equity::Equity::ZERO; unseen_tally.len()];
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
                this.init(board, racks);
                let value =
                    this.one_in_bag(mover, bag_tile, limits, &mut exact, memo, scoreless)?;
                let margin = i64::from(value.raw()) + score_diff;
                outcomes.record(u32::from(count), margin)?;
                run[t] = equity::Equity::new(
                    margin.clamp(i64::from(i32::MIN), i64::from(i32::MAX)) as i32,
                );
            }
            let (win_pct, margin) = outcomes.averages();
            let expected_margin = checked_expected_margin(margin)?;
            let result = PegResult {
                win_pct,
                expected_margin,
                row: None,
                best_move: None,
                moves: Vec::new(),
                committed: false,
                exact,
                depth: limits.depth.get(),
                out_of_memory: false,
            };
            Ok((result, run))
        };
        let deepens = limits.deepens();
        let found = &mut progress.clairvoyant;
        let found_margins = &mut progress.clairvoyant_margins;
        limits.may_stop.set(deepens);
        if found.is_none() {
            limits.depth.set(if deepens { 1 } else { COMPLETE });
            match pass(self) {
                Ok((first, run)) => (*found, *found_margins) = (Some(first), run),
                Err(_) if limits.stopped.get() => {
                    // stopped before the first pass valued every tile: none yet
                    return Ok(PegResult {
                        win_pct: 0.0,
                        expected_margin: equity::Equity::ZERO,
                        row: None,
                        best_move: None,
                        moves: Vec::new(),
                        committed: false,
                        exact: false,
                        depth: 0,
                        out_of_memory: false,
                    });
                }
                Err(e) => return Err(e),
            }
        }
        limits.may_stop.set(true);
        while let Some(result) = found.as_ref().filter(|r| !r.exact && r.depth < COMPLETE) {
            limits.depth.set(result.depth + 1);
            match pass(self) {
                Ok((deeper, run)) => (*found, *found_margins) = (Some(deeper), run),
                Err(_) if limits.stopped.get() => break,
                Err(e) => return Err(e),
            }
        }
        let Some(result) = found else {
            unreachable!("the first pass always finishes");
        };
        Ok(result.clone())
    }

    #[inline]
    fn peg_committed_no_exchange(
        &mut self,
        progress: &mut PegProgress,
        limits: &BagLimits,
    ) -> error::Returns<PegResult> {
        let (mut sub, mut mg) = match self.take_both(limits) {
            Ok(both) => both,
            Err(_) if limits.stopped.get() => {
                return Ok(peg_committed_answer(&mut progress.committed));
            }
            Err(e) => return Err(e),
        };
        // the endgames keep what they learn in this solver's table, lent to the
        // one solving them, so each knows what the others found
        self.swap_tables(&mut sub)?;
        if !LEAN {
            self.swap_lanes(&mut sub);
        }
        sub.work_buffer.threads = self.work_buffer.threads;
        let result = Self::peg_committed_by(
            &mut sub,
            &self.klv,
            &mut mg,
            &PegPosition {
                mover: progress.mover,
                board: &progress.board,
                mover_rack: &progress.mover_rack,
                unseen: &progress.unseen,
                tiles: progress.unseen_tally.len(),
                scoreless: progress.scoreless,
                root_moves: progress.root_moves.as_ref(),
                priority: self.root_priority,
            },
            progress.score_diff,
            limits,
            &mut progress.committed,
        );
        self.swap_tables(&mut sub)?;
        if !LEAN {
            self.swap_lanes(&mut sub);
        }
        self.keep_sub(sub);
        self.kept_mg = Some(mg);
        result
    }

    fn peg_place_endgame(
        sub: &mut Self,
        position: &PegPosition<'_>,
        play: &movegen::Play,
        drawn: u8,
        opp_rack: &[u8],
        limits: &BagLimits,
        exact: &mut bool,
    ) -> error::Returns<equity::Equity> {
        let movegen::Play::Place {
            down,
            lane,
            idx,
            word,
            score,
        } = play
        else {
            unreachable!("Place candidate always holds a Play::Place")
        };
        let PegPosition {
            mover,
            board,
            mover_rack,
            ..
        } = *position;
        // the position after the place, made in the solver's own (as init
        // makes it, with no scoreless turns before it)
        let nb = &mut sub.board_tiles;
        nb.clear();
        nb.extend_from_slice(board);
        let mr = &mut sub.racks[mover as usize];
        mr.clear();
        mr.extend_from_slice(mover_rack);
        let strider = sub.game_config.board_layout().dim().lane(*down, *lane);
        for (i, &tile) in (*idx..).zip(word.iter()) {
            if tile != 0 {
                nb[strider.at_in_lane(i)] = tile;
                let b = tile & !((tile as i8) >> 7) as u8;
                let p = mr.iter().rposition(|&x| x == b).unwrap();
                mr[p] = 0x80;
            }
        }
        mr.retain(|&x| x != 0x80);
        mr.push(drawn);
        let or = &mut sub.racks[(mover ^ 1) as usize];
        or.clear();
        or.extend_from_slice(opp_rack);
        sub.scoreless = [0, 0];
        Ok(*score - sub.solve_in_bag(mover ^ 1, limits, exact)?)
    }

    // a place a limit stopped is left none
    fn peg_places_on_threads(
        sub: &mut Self,
        position: &PegPosition<'_>,
        score_diff: equity::Equity,
        limits: &BagLimits,
        moves: &[PegMove],
    ) -> (Vec<Option<PegValue>>, Vec<equity::Equity>) {
        use std::sync::atomic::{AtomicUsize, Ordering::Relaxed};
        // each thread's solver, made at its first solve and kept for the next
        let mut helpers = std::mem::take(&mut sub.helpers);
        helpers.resize_with(sub.work_buffer.threads, || None);
        let (gc, kwg, arm_tables) = (sub.game_config, sub.kwg, sub.arm_tables);
        let (run_letters, like) = (&sub.run_letters, &*sub.klv);
        let clock = limits.on_one_clock();
        let deal = &sub.work_buffer.lane_pool.deal;
        let buckets = sub.work_buffer.table.buckets.len();
        let next = &AtomicUsize::new(0);
        let helped = std::thread::scope(|s| {
            let handles = (helpers.into_iter())
                .map(|helper| {
                    s.spawn(move || {
                        let mut found = Vec::new();
                        // each move found's margins, a run each, in turn
                        let mut runs = Vec::new();
                        let limits = BagLimits::on_the_clock(clock);
                        let fresh = (Some(like), limits.memory);
                        let Ok(mut own) =
                            Self::of_helper(gc, kwg, arm_tables, run_letters, helper, fresh)
                        else {
                            return (found, runs, 0, 0, None);
                        };
                        // empty, with this solver's room, as each solve's
                        own.work_buffer.table.empty_to(buckets);
                        if !LEAN {
                            own.pool_lanes_as(deal);
                        }
                        let mut opp_rack = Vec::new();
                        let mut run = vec![equity::Equity::ZERO; position.tiles];
                        loop {
                            let i = next.fetch_add(1, Relaxed);
                            let Some(cand) = moves.get(i) else {
                                break;
                            };
                            let PegMove::Place(play) = cand else {
                                continue;
                            };
                            let value = peg_value(
                                position.unseen,
                                score_diff,
                                &mut opp_rack,
                                &mut run,
                                |_, drawn, opp_rack, exact| {
                                    Self::peg_place_endgame(
                                        &mut own, position, play, drawn, opp_rack, &limits, exact,
                                    )
                                },
                            );
                            let Ok(value) = value else {
                                break;
                            };
                            found.push((i, value));
                            runs.extend_from_slice(&run);
                        }
                        let (endgames, nodes) = (limits.endgames.get(), limits.nodes.get());
                        (found, runs, endgames, nodes, Some(own.into_helper()))
                    })
                })
                .collect::<Vec<_>>();
            (handles.into_iter())
                .map(|handle| handle.join().unwrap())
                .collect::<Vec<_>>()
        });
        let k = position.tiles;
        let mut ready = vec![None; moves.len()];
        let mut ready_margins = vec![equity::Equity::ZERO; moves.len() * k];
        for (found, runs, endgames, nodes, helper) in helped {
            for ((i, value), run) in found.into_iter().zip(runs.chunks_exact(k)) {
                ready[i] = Some(value);
                ready_margins[i * k..][..k].copy_from_slice(run);
            }
            limits.endgames.set(limits.endgames.get() + endgames);
            limits.nodes.set(limits.nodes.get() + nodes);
            sub.helpers.push(helper);
        }
        (ready, ready_margins)
    }

    // the committed model's search, its endgames solved by sub: the move with
    // the most wins, then the best mean margin, ties going to the move
    // generated first (the pass, then the places). Deepening, the first pass
    // values the places by score, highest first, then the pass, and each later
    // pass the moves best first by the pass before; a limit stops it once the
    // first move is valued, and the answer is the best move of the deepest pass
    // that valued every move, or, the first stopped, the best of those it valued.
    // A solve resuming it goes on at the move it stopped before
    #[inline]
    fn peg_committed_by(
        sub: &mut Self,
        klv: &klv::Klv<L>,
        mg: &mut movegen::KurniaMoveGenerator<G, JUMPS, ANY_SIDE>,
        position: &PegPosition<'_>,
        score_diff: equity::Equity,
        limits: &BagLimits,
        state: &mut PegCommitted,
    ) -> error::Returns<PegResult> {
        let PegPosition {
            mover,
            board,
            mover_rack,
            unseen,
            tiles,
            scoreless,
            root_moves,
            priority,
        } = *position;
        let gc = sub.game_config;
        let kwg = sub.kwg;
        let arm_tables = sub.arm_tables;
        let alphabet = gc.alphabet();
        let opp = mover ^ 1;
        // a pass answering the game's last turn, if that was a pass, ends it
        let pass_passes = scoreless[0].saturating_add(1);
        let pass_zeros = scoreless[1].saturating_add(1);
        let pass_ends_the_game =
            scoreless[0] > 0 || scoreless_turns_end(gc, None, pass_passes, pass_zeros);

        let deepens = limits.deepens();
        if state.moves.is_empty() {
            // the root's lists, made once a search at their full lengths, so
            // no pass grows them, as peg_bag_by's: the moves generated under
            // the memory limit, if any, and each list taken only where it has
            // room. Where it has none the search stops before it starts,
            // nothing kept, and the next step tries again
            let lists = (|| -> error::Returns<_> {
                let moves = if let Some(root_moves) = root_moves {
                    let mut moves = list_in_room(limits, root_moves.plays.len())?;
                    for play in &root_moves.plays {
                        let chosen = match play {
                            movegen::Play::Exchange { tiles } if tiles.is_empty() => PegMove::Pass,
                            movegen::Play::Place { word, .. } => {
                                let _word = limits.reserve(word.len())?;
                                PegMove::Place(play.clone())
                            }
                            _ => return Err("root_moves: exchange is unavailable".into()),
                        };
                        moves.push(chosen);
                    }
                    moves
                } else {
                    let snapshot = movegen::BoardSnapshot {
                        board_tiles: board,
                        game_config: gc,
                        kwg,
                        anagrams: arm_tables.anagrams(),
                        rack_lengths: arm_tables.rack_lengths(),
                        klv,
                    };
                    let pass = movegen::PassPolicy::AsACandidate;
                    let cut = match limits.memory {
                        None => {
                            gen_raw_all::<LEAN, _, _, _, JUMPS, ANY_SIDE>(
                                &mut *mg, &snapshot, mover_rack, 0, pass,
                            )?;
                            false
                        }
                        Some(memory) => mg.gen_moves_raw_all_unsorted_held(
                            &snapshot,
                            mover_rack,
                            0,
                            pass,
                            movegen::HeldBytes { memory, kept: 0 },
                        )?,
                    };
                    if cut {
                        return Err(limits.stop_refused());
                    }
                    let places = (mg.plays.iter())
                        .filter(|vm| matches!(vm.play, movegen::Play::Place { .. }))
                        .count();
                    let n = 1 + places;
                    let mut moves = list_in_room::<PegMove>(limits, n)?;
                    moves.push(PegMove::Pass);
                    for vm in &mg.plays {
                        if matches!(vm.play, movegen::Play::Place { .. }) {
                            let _copy = limits.reserve(play_bytes(&vm.play))?;
                            moves.push(PegMove::Place(vm.play.clone()));
                        }
                    }
                    moves
                };
                let n = moves.len();
                // a run of margins a move, one a tile of the alphabet, for the
                // pass under way and for the answer
                let runs = n.saturating_mul(tiles);
                Ok((
                    moves,
                    list_in_room(limits, n)?,
                    list_in_room(limits, n)?,
                    list_in_room(limits, n)?,
                    list_in_room(limits, n)?,
                    (list_in_room(limits, runs)?, list_in_room(limits, runs)?),
                    (list_in_room(limits, n)?, list_in_room(limits, n)?),
                ))
            })();
            let (moves, order, valued, reached, answered, margins, ranks) = match lists {
                Ok(lists) => lists,
                Err(_) if limits.stopped.get() => return Ok(peg_committed_answer(state)),
                Err(e) => return Err(e),
            };
            (state.moves, state.order, state.valued) = (moves, order, valued);
            (state.reached, state.answered) = (reached, answered);
            (state.margins, state.answered_margins) = margins;
            (state.ranked, state.places) = ranks;
            let runs = state.moves.len() * tiles;
            state.margins.resize(runs, equity::Equity::ZERO);
            state.answered_margins.resize(runs, equity::Equity::ZERO);
            // the order the first pass values the moves in
            state.order.extend(0..state.moves.len());
            if deepens {
                state.order.sort_unstable_by_key(|&i| {
                    let score = match &state.moves[i] {
                        PegMove::Place(movegen::Play::Place { score, .. }) => *score,
                        _ => equity::Equity::NEG_INFINITY,
                    };
                    (std::cmp::Reverse(score), i)
                });
            }
            state.valued.extend(state.moves.iter().map(|_| None));
            state.pass_exact = true;
            state.depth = if deepens { 1 } else { COMPLETE };
        }
        if !state.steered.is_empty() && state.standing.len() != state.moves.len() {
            state.stand();
        }

        // one pass: each move's value is exact, so the same on any thread
        let k = tiles;
        let (mut ready, mut ready_margins) = (Vec::new(), Vec::new());
        if !deepens && sub.work_buffer.threads > 1 && state.next == 0 {
            (ready, ready_margins) =
                Self::peg_places_on_threads(sub, position, score_diff, limits, &state.moves);
        }
        // the pass's memo, made once and renewed for each tile
        let mut pass_memo = BagTable::default();
        let mut value_move = |cand: &PegMove,
                              line: &mut PassLine,
                              kept: &mut PassKept,
                              margins: &mut [equity::Equity]|
         -> error::Returns<PegValue> {
            let PassKept {
                opp_rack,
                r2,
                placed,
            } = kept;
            peg_value(
                unseen,
                score_diff,
                opp_rack,
                margins,
                |t, drawn, opp_rack, exact| {
                    Ok(match cand {
                        PegMove::Place(play) => Self::peg_place_endgame(
                            sub, position, play, drawn, opp_rack, limits, exact,
                        )?,
                        // the pass ends the game at once when it brings the
                        // scoreless turns to the config's count, each losing
                        // their rack
                        PegMove::Pass if pass_ends_the_game => {
                            alphabet.scaled_rack_score(opp_rack)
                                - alphabet.scaled_rack_score(mover_rack)
                        }
                        PegMove::Pass => {
                            // PEG2-7: with two or more in the bag, draw every
                            // tile from the same end -- pop, the back -- and
                            // store a scenario reversed to match. Do NOT reuse
                            // Bag::replenish for the enumeration: it pops for
                            // even players and shifts for odd ones, which on a
                            // reversed bag hands p1 the leftover instead of
                            // the next popped tile.
                            r2[mover as usize].clear();
                            r2[mover as usize].extend_from_slice(mover_rack);
                            r2[opp as usize].clear();
                            r2[opp as usize].extend_from_slice(opp_rack);
                            pass_memo.renew();
                            let mut search = OneInBag {
                                gc,
                                kwg,
                                arm_tables,
                                klv,
                                sub: &mut *sub,
                                mg: &mut *mg,
                                memo: &mut pass_memo,
                                limits,
                                exact: true,
                                line: Some(&mut *line),
                                placed: &mut *placed,
                            };
                            let v = -Self::one_in_bag_minimax(
                                &mut search,
                                board,
                                r2,
                                OneInBagTurn {
                                    mover: opp,
                                    bag_tile: t,
                                    passes: pass_passes,
                                    zeros: pass_zeros,
                                    answering: true,
                                },
                            )?;
                            *exact &= search.exact;
                            v
                        }
                    })
                },
            )
        };

        while !state.done {
            if !state.standing.is_empty() && !state.steer_order() {
                return Ok(peg_committed_answer(state));
            }
            prioritize(
                &mut state.order[state.next..],
                priority,
                |&i, play| match &state.moves[i] {
                    PegMove::Place(candidate) => candidate == play,
                    PegMove::Pass => {
                        matches!(play, movegen::Play::Exchange { tiles } if tiles.is_empty())
                    }
                },
            );
            limits.depth.set(state.depth);
            let mut stopped = false;
            while let Some(&i) = state.order.get(state.next) {
                limits.moves.set(Some((state.next, state.order.len())));
                limits
                    .may_stop
                    .set(deepens && (state.answer.is_some() || !state.reached.is_empty()));
                let run = &mut state.margins[i * k..][..k];
                let valued = match ready.get_mut(i).and_then(Option::take) {
                    Some(value) => {
                        run.copy_from_slice(&ready_margins[i * k..][..k]);
                        Ok(value)
                    }
                    None => value_move(&state.moves[i], &mut state.passing, &mut state.kept, run),
                };
                match valued {
                    Ok(value) => {
                        state.pass_exact &= value.exact;
                        state.valued[i] = Some(value);
                        state.reached.push(i);
                        state.next += 1;
                    }
                    Err(_) if limits.stopped.get() => {
                        stopped = true;
                        break;
                    }
                    Err(e) => return Err(e),
                }
            }
            if stopped {
                if state.answer.is_none() {
                    // no pass valued every move: the best of those valued
                    return Ok(peg_committed_answer(state));
                }
                break;
            }
            let best = peg_best(&mut state.reached, &state.valued);
            state.answer = Some((best, state.depth));
            let done = state.pass_exact || state.depth == COMPLETE;
            if !done {
                // the next pass values the moves best first by this one,
                // afresh; each list keeps its room
                std::mem::swap(&mut state.order, &mut state.reached);
                state.reached.clear();
                let valued = &state.valued;
                (state.order).sort_unstable_by(|&a, &b| peg_ranks(valued, a, b));
            }
            if !state.standing.is_empty() {
                state.carry_paused();
            }
            std::mem::swap(&mut state.answered, &mut state.valued);
            std::mem::swap(&mut state.answered_margins, &mut state.margins);
            state.valued.clear();
            state.valued.resize_with(state.answered.len(), || None);
            state.rank_answer();
            if done {
                state.done = true;
                break;
            }
            state.next = 0;
            state.pass_exact = true;
            state.passing = Default::default();
            state.depth += 1;
        }
        if state.answer.is_none() {
            unreachable!("the first move of the first pass is always valued");
        }
        Ok(peg_committed_answer(state))
    }
}

fn peg_committed_answer(state: &mut PegCommitted) -> PegResult {
    match state.answer {
        Some((best, depth)) => {
            let exact = state.done && state.pass_exact;
            let value = state.answered[best].as_ref().unwrap();
            peg_result(&state.moves, best, value, depth, exact)
        }
        None if state.reached.is_empty() => PegResult {
            win_pct: 0.0,
            expected_margin: equity::Equity::ZERO,
            row: None,
            best_move: None,
            moves: Vec::new(),
            committed: true,
            exact: false,
            depth: 0,
            out_of_memory: false,
        },
        None => {
            let best = peg_best(&mut state.reached, &state.valued);
            state.rank_answer();
            let value = state.valued[best].as_ref().unwrap();
            peg_result(&state.moves, best, value, 0, false)
        }
    }
}

fn peg_value(
    unseen: &[(u8, u32)],
    score_diff: equity::Equity,
    opp_rack: &mut Vec<u8>,         // the caller's, refilled for each tile
    margins: &mut [equity::Equity], // the move's run, a margin a tile, by tile
    mut endgame: impl FnMut(u8, u8, &[u8], &mut bool) -> error::Returns<equity::Equity>,
) -> error::Returns<PegValue> {
    let mut outcomes = PegOutcomes::default();
    let mut exact = true;
    let lead = i64::from(score_diff.raw());
    // the opponent's rack for each tile in the bag, in the caller's buffer:
    // every tile unseen but the one drawn
    for &(t, w) in unseen {
        let drawn = t & !((t as i8) >> 7) as u8;

        opp_rack.clear();
        for &(u, c) in unseen {
            let take = if u == t { c - 1 } else { c };
            for _ in 0..take {
                opp_rack.push(u);
            }
        }
        let v = endgame(t, drawn, opp_rack, &mut exact)?;
        let margin = i64::from(v.raw()) + lead;
        outcomes.record(w, margin)?;
        margins[t as usize] =
            equity::Equity::new(margin.clamp(i64::from(i32::MIN), i64::from(i32::MAX)) as i32);
    }
    let (win, marg) = outcomes.averages();
    Ok(PegValue {
        win,
        marg,
        expected_margin: checked_expected_margin(marg)?,
        exact,
    })
}

// the best of these moves: the most wins, then the best mean margin, ties going
// to the move generated first
fn peg_best(reached: &mut [usize], valued: &[Option<PegValue>]) -> usize {
    reached.sort_unstable();
    let mut best = reached[0];
    for &i in &reached[1..] {
        let (value, other) = (valued[i].as_ref().unwrap(), valued[best].as_ref().unwrap());
        if value.win > other.win || (value.win == other.win && value.marg > other.marg) {
            best = i;
        }
    }
    best
}

// the first ranks before the second, best first as peg_best ranks them, then
// as generated
fn peg_ranks(valued: &[Option<PegValue>], a: usize, b: usize) -> std::cmp::Ordering {
    let (x, y) = (valued[a].as_ref().unwrap(), valued[b].as_ref().unwrap());
    (y.win.total_cmp(&x.win))
        .then(y.marg.total_cmp(&x.marg))
        .then(a.cmp(&b))
}

// the committed model's answer: a move, its value, and the depth it is of,
// without the moves (PegProgress::answer_moves lists them)
fn peg_result(
    moves: &[PegMove],
    best: usize,
    value: &PegValue,
    depth: i8,
    exact: bool,
) -> PegResult {
    PegResult {
        win_pct: value.win,
        expected_margin: value.expected_margin,
        row: Some(best),
        best_move: Some(moves[best].clone()),
        moves: Vec::new(),
        committed: true,
        exact,
        depth,
        out_of_memory: false,
    }
}

// a search with a tile in the bag that goes on across solves, its position in
// it: a caller keeps it (as a wasm32 page solving in steps keeps it with the
// solver's table), and a solve given it starts where the last one stopped,
// keeping what the passes before found. The position holds its scoreless turns
pub struct PegProgress {
    mover: u8,
    board: Vec<u8>,
    mover_rack: Vec<u8>,
    unseen_tally: Vec<u8>,
    unseen: Vec<(u8, u32)>, // each tile unseen with its count, for every solve
    score_diff: equity::Equity,
    scoreless: [u8; 2],
    root_moves: Option<RootMoves>,
    committed: PegCommitted,
    clairvoyant: Option<PegResult>, // the deepest pass that valued every tile
    clairvoyant_margins: Vec<equity::Equity>, // its margin a tile, by tile
    // with exchanges legal, the positions the pass under way has valued (and
    // its depth), kept so a solve a limit stopped goes on from them
    exchange_memo: (i8, BagTable<equity::Equity>),
    endgames: u64,   // the endgames its solves searched
    nodes: u64,      // their nodes
    lanes: LanePool, // its endgames' lane pool
}

impl PegProgress {
    // a search of the position before play resumes, with score_diff the
    // mover's lead and scoreless the passes and scoreless turns so far
    #[inline]
    pub fn new(
        mover: u8,
        board: &[u8],
        mover_rack: &[u8],
        unseen_tally: &[u8],
        score_diff: equity::Equity,
        scoreless: [u16; 2],
    ) -> Self {
        Self {
            mover,
            board: board.to_vec(),
            mover_rack: mover_rack.to_vec(),
            unseen_tally: unseen_tally.to_vec(),
            unseen: (unseen_tally.iter().enumerate())
                .filter(|&(_, &c)| c != 0)
                .map(|(t, &c)| (t as u8, u32::from(c)))
                .collect(),
            score_diff,
            scoreless: scoreless_counts(scoreless),
            root_moves: None,
            committed: Default::default(),
            clairvoyant: None,
            clairvoyant_margins: Vec::new(),
            exchange_memo: (0, BagTable::default()),
            endgames: 0,
            nodes: 0,
            lanes: LanePool::new(),
        }
    }

    // select the first plays checked for this committed position
    pub fn with_root_moves(mut self, root_moves: Option<RootMoves>) -> Self {
        self.root_moves = root_moves;
        self
    }

    // the endgames every solve of the search has searched, and their nodes
    #[inline(always)]
    pub fn searched(&self) -> (u64, u64) {
        (self.endgames, self.nodes)
    }

    // the committed model's pass under way: the moves it has valued, of all it
    // values; none before the search began, nor in the clairvoyant model
    #[inline(always)]
    pub fn moves(&self) -> Option<(usize, usize)> {
        let committed = &self.committed;
        (!committed.order.is_empty()).then_some((committed.next, committed.order.len()))
    }

    // the moves the last solve's answer valued (PegResult::moves); none in the
    // clairvoyant model
    #[inline(always)]
    pub fn answer_total(&self) -> usize {
        self.committed.ranked.len()
    }

    // those of the last solve's answer's moves, best first, at the places in
    // range (clamped to the moves valued): the same as that slice of
    // PegResult::moves, a slice of the ranking kept with the answer
    pub fn answer_moves(&self, range: std::ops::Range<usize>) -> Vec<PegMoveValue> {
        let committed = &self.committed;
        let valued = committed.answer_values();
        let to = range.end.min(committed.ranked.len());
        let from = range.start.min(to);
        (committed.ranked[from..to].iter())
            .map(|&i| {
                let value = valued[i].as_ref().unwrap();
                PegMoveValue {
                    play: committed.moves[i].clone(),
                    win_pct: value.win,
                    expected_margin: value.expected_margin,
                    row: i,
                    exact: value.exact,
                    cut: false,
                }
            })
            .collect()
    }

    // the hypotheses of a row of the last solve's answer (PegResult::row,
    // PegMoveValue::row): each tile the mover cannot see, its count, and the
    // row's margin were it in the bag, read from the answer's margins; with no
    // row, the clairvoyant model's (none before it valued every tile)
    pub fn hypotheses(&self, row: Option<usize>) -> PegHypotheses<'_> {
        let k = self.unseen_tally.len();
        let margins = match row {
            Some(i) => &self.committed.answer_margins()[i * k..][..k],
            None => &self.clairvoyant_margins[..],
        };
        PegHypotheses {
            tally: &self.unseen_tally,
            margins,
            at: 0,
        }
    }

    // where the last solve's answer lists this move, best first (the place
    // answer_moves gives it); none if the answer did not value it
    pub fn answer_place(&self, play: &PegMove) -> Option<usize> {
        let committed = &self.committed;
        let valued = committed.answer_values();
        let at = (0..committed.moves.len())
            .find(|&i| valued[i].is_some() && committed.moves[i] == *play)?;
        Some(committed.places[at])
    }

    // as PegBagProgress::change, a root move of the committed model's (the
    // pass an exchange of no tiles; any other exchange none of them)
    pub fn change(&mut self, play: &movegen::Play, change: PegBagChange) -> PegBagStanding {
        let chosen = match play {
            movegen::Play::Exchange { tiles } if tiles.is_empty() => PegMove::Pass,
            movegen::Play::Place { .. } => PegMove::Place(play.clone()),
            movegen::Play::Exchange { .. } => return PegBagStanding::Absent,
        };
        self.committed.change(chosen, change)
    }

    #[inline(always)]
    pub fn is_done(&self) -> bool {
        self.committed.done
    }
}

impl PegCommitted {
    fn change(&mut self, play: PegMove, change: PegBagChange) -> PegBagStanding {
        if !self.moves.is_empty() && !self.moves.contains(&play) {
            return PegBagStanding::Absent;
        }
        let listed = self.steered.iter().position(|(other, _)| *other == play);
        let was = listed.map_or(PegBagStanding::Running, |k| self.steered[k].1);
        let now = match (change, was) {
            (PegBagChange::Add, PegBagStanding::Absent)
            | (PegBagChange::Resume, PegBagStanding::Paused) => PegBagStanding::Running,
            (PegBagChange::Remove, _) => PegBagStanding::Absent,
            (PegBagChange::Pause, PegBagStanding::Running) => PegBagStanding::Paused,
            _ => was,
        };
        match (listed, now) {
            (_, _) if now == was => {}
            (Some(k), PegBagStanding::Running) => {
                self.steered.remove(k);
            }
            (Some(k), _) => self.steered[k].1 = now,
            (None, _) => self.steered.push((play.clone(), now)),
        }
        if now != was && !self.moves.is_empty() {
            self.stand();
            self.steered_to(&play, now);
        }
        was
    }

    fn stand(&mut self) {
        if self.steered.is_empty() && self.standing.is_empty() {
            return;
        }
        let steered = &self.steered;
        self.standing.clear();
        self.standing.extend(self.moves.iter().map(|play| {
            match steered.iter().find(|(other, _)| other == play) {
                Some((_, PegBagStanding::Paused)) => STAND_PAUSED,
                Some((_, PegBagStanding::Absent)) => STAND_ABSENT,
                _ => 0,
            }
        }));
        self.stale.resize(self.moves.len(), false);
    }

    #[inline(always)]
    fn live(&self, i: usize) -> bool {
        self.standing.get(i).is_none_or(|&s| s & 3 == 0)
    }

    fn steered_to(&mut self, play: &PegMove, now: PegBagStanding) {
        let Some(i) = self.moves.iter().position(|other| other == play) else {
            return;
        };
        if now == PegBagStanding::Absent {
            self.valued[i] = None;
            if let Some(value) = self.answered.get_mut(i) {
                *value = None;
            }
        }
        if now != PegBagStanding::Running {
            self.reached.retain(|&other| other != i);
        }
        if let Some((_, depth)) = self.answer {
            let mut live = (0..self.moves.len())
                .filter(|&k| self.live(k) && self.answered[k].is_some() && !self.stale[k])
                .collect::<Vec<_>>();
            if !live.is_empty() {
                self.answer = Some((peg_best(&mut live, &self.answered), depth));
            }
        }
        let values = self.answer_values();
        let lacking = (0..self.moves.len())
            .any(|k| self.live(k) && (values[k].is_none() || self.stale.get(k) == Some(&true)));
        if self.done && lacking {
            self.reopen();
        }
        self.rank_answer();
    }

    fn reopen(&mut self) {
        self.done = false;
        if self.answered.len() == self.moves.len() {
            self.valued.clone_from(&self.answered);
            self.margins.clone_from(&self.answered_margins);
        }
        self.reached.clear();
        for i in 0..self.moves.len() {
            if self.live(i) && self.valued[i].is_some() && !self.stale[i] {
                self.reached.push(i);
            }
        }
        self.order.clear();
        self.order.extend_from_slice(&self.reached);
        self.next = self.reached.len();
        self.pass_exact = (self.reached.iter()).all(|&i| self.valued[i].as_ref().unwrap().exact);
    }

    fn steer_order(&mut self) -> bool {
        for &i in &self.reached {
            self.standing[i] |= MARK_REACHED;
        }
        let mut keep = 0;
        for k in 0..self.order.len() {
            let i = self.order[k];
            let s = self.standing[i];
            if (k < self.next && s & MARK_REACHED != 0) || (k >= self.next && s & 3 == 0) {
                self.order[keep] = i;
                keep += 1;
            }
        }
        self.order.truncate(keep);
        self.next = self.reached.len();
        for &i in &self.order {
            self.standing[i] |= MARK_TAKEN;
        }
        for i in 0..self.moves.len() {
            if self.standing[i] == 0 {
                self.order.push(i);
            }
            self.standing[i] &= 3;
        }
        (0..self.moves.len()).any(|i| self.live(i))
    }

    // the paused moves' values, and each one's margins, carried from the
    // answer into the pass that did not value them
    fn carry_paused(&mut self) {
        let k = self.margins.len() / self.moves.len().max(1);
        for i in 0..self.moves.len() {
            self.stale[i] = false;
            if self.standing[i] & STAND_PAUSED != 0 && self.valued[i].is_none() {
                self.valued[i] = self.answered.get(i).cloned().flatten();
                self.stale[i] = self.valued[i].is_some();
                if self.stale[i] {
                    let run = i * k..(i + 1) * k;
                    self.margins[run.clone()].copy_from_slice(&self.answered_margins[run]);
                }
            }
        }
    }
}

// the committed model's search so far: the moves; the order its pass values
// them in, the next to value, each one's value in the pass, and those the pass
// has valued; whether all the pass's endgames were exact, and its depth; the
// pass move's own search in it, so far; the best move of the deepest pass that
// valued every move, with that depth and that pass's values; whether that
// answer is final; and the answer's valued moves best first, with each move's
// place among them
#[derive(Default)]
struct PegCommitted {
    moves: Vec<PegMove>,
    order: Vec<usize>,
    next: usize,
    valued: Vec<Option<PegValue>>,
    reached: Vec<usize>,
    pass_exact: bool,
    depth: i8,
    passing: PassLine,
    answer: Option<(usize, i8)>,
    answered: Vec<Option<PegValue>>,
    done: bool,
    ranked: Vec<usize>,
    places: Vec<usize>,
    kept: PassKept,
    margins: Vec<equity::Equity>,
    answered_margins: Vec<equity::Equity>,
    steered: Vec<(PegMove, PegBagStanding)>, // those not running
    standing: Vec<u8>,                       // each move's (STAND_*), empty until one is steered
    stale: Vec<bool>,                        // each answer value kept from an older pass
}

// what valuing a move of the committed model refills for each tile, kept from
// solve to solve: the tile's opponent rack, the pass's racks, and the board
// and racks after each of its places (not its memo, which a solve lets go of
// as it ends, so it holds no room while later solves grow the table)
#[derive(Default)]
struct PassKept {
    opp_rack: Vec<u8>,
    r2: [Vec<u8>; 2],
    placed: Placed,
}

impl PegCommitted {
    // the values the answer is of: the deepest pass's that valued every move,
    // or, before one did, the pass's under way
    #[inline(always)]
    fn answer_values(&self) -> &[Option<PegValue>] {
        if self.answer.is_some() {
            &self.answered
        } else {
            &self.valued
        }
    }

    // each tile's margin of those values, a run of the unseen tiles' count a
    // move, at the move's index times that count
    #[inline(always)]
    fn answer_margins(&self) -> &[equity::Equity] {
        if self.answer.is_some() {
            &self.answered_margins
        } else {
            &self.margins
        }
    }

    // rank the answer's valued moves, best first, and note each one's place:
    // made when an answer is swapped in, and when a limit stops the first pass
    #[inline]
    fn rank_answer(&mut self) {
        let valued = if self.answer.is_some() {
            &self.answered
        } else {
            &self.valued
        };
        self.ranked.clear();
        (self.ranked).extend((0..valued.len()).filter(|&i| valued[i].is_some()));
        (self.ranked).sort_unstable_by(|&a, &b| peg_ranks(valued, a, b));
        if !self.standing.is_empty() {
            let standing = &self.standing;
            (self.ranked).sort_by_key(|&i| standing[i] & 3 != 0);
        }
        self.places.clear();
        self.places.resize(valued.len(), 0);
        for (place, &i) in self.ranked.iter().enumerate() {
            self.places[i] = place;
        }
    }
}

#[derive(Clone, PartialEq)]
pub enum PegMove {
    Pass,
    Place(movegen::Play),
}

// a move of a search with a tile in the bag and its value: its share of the
// bag's tiles won (a tie counted half), its mean margin, its row of the answer
// (PegProgress::hypotheses reads its margin for each tile the bag may hold),
// and whether every endgame valuing it was searched to its end
#[derive(Clone)]
pub struct PegMoveValue {
    pub play: PegMove,
    pub win_pct: f64,
    pub expected_margin: equity::Equity,
    pub row: usize,
    pub exact: bool,
    pub cut: bool,
}

// row: the best move's row of the answer, as PegMoveValue::row; none in the
// clairvoyant model, or before a move was valued
#[derive(Clone)]
pub struct PegResult {
    pub win_pct: f64,
    pub expected_margin: equity::Equity,
    pub row: Option<usize>,
    pub best_move: Option<PegMove>,
    pub moves: Vec<PegMoveValue>, // the committed model's alone: every move valued, best first
    pub committed: bool,
    pub exact: bool, // false when a limit stopped it short of solving every endgame inside it
    pub depth: i8,   // the pass answered from: 0 none finished, i8::MAX one to the end
    pub out_of_memory: bool, // the memory limit stopped it, or refused it room
}

// a move's value in the committed model: the share of the bag's tiles it wins
// (a tie counted half), its mean margin, and whether every endgame valuing it
// was searched to its end; each tile's margin is in the search's margins
#[derive(Clone)]
struct PegValue {
    win: f64,
    marg: f64,
    expected_margin: equity::Equity,
    exact: bool,
}

// a row's hypotheses as PegProgress::hypotheses reads them: each tile the
// mover cannot see (in tile order), its count, and the row's margin, joined as
// read from the tally and the row's run of margins, a margin a tile
pub struct PegHypotheses<'a> {
    tally: &'a [u8],
    margins: &'a [equity::Equity],
    at: usize,
}

impl PegHypotheses<'_> {
    // whether the row has none: no row valued yet
    #[inline(always)]
    pub fn is_empty(&self) -> bool {
        self.margins.is_empty()
    }
}

impl Iterator for PegHypotheses<'_> {
    type Item = (u8, u32, equity::Equity);

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        while let Some(&margin) = self.margins.get(self.at) {
            let tile = self.at;
            self.at += 1;
            let count = self.tally[tile];
            if count != 0 {
                return Some((tile as u8, u32::from(count), margin));
            }
        }
        None
    }
}

#[derive(Debug)]
pub enum PegUnsupported {
    ExchangeWithoutForcedEnd,
    PassWithoutForcedEnd,
    RootMovesNeedCommitted,
}

impl std::fmt::Display for PegUnsupported {
    #[inline]
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PegUnsupported::ExchangeWithoutForcedEnd => f.write_str(
                "exchange is legal with these tiles in the bag but scoreless turns \
                 never force the game to end, so this position is not solvable",
            ),
            PegUnsupported::PassWithoutForcedEnd => f.write_str(
                "no count of scoreless turns ends the game, so passes could go on \
                 for good and this position is not solvable",
            ),
            PegUnsupported::RootMovesNeedCommitted => {
                f.write_str("root_moves: choose a committed bag search when exchanges are legal")
            }
        }
    }
}

// a position with tiles in the bag to search: the mover, the board, the mover's
// rack, the tiles they cannot see (a count per tile), how many of those are in
// the bag (the rest on the opponent's rack), the mover's lead, the passes and
// scoreless turns so far, each player's exchanges, and what the search takes
// each player to know
#[derive(Clone)]
pub struct PegBagPosition {
    pub mover: u8,
    pub board: Vec<u8>,
    pub mover_rack: Vec<u8>,
    pub unseen_tally: Vec<u8>,
    pub bag_size: usize,
    pub score_diff: equity::Equity,
    pub scoreless: [u16; 2],
    pub exchanges: [i16; 2],
    pub model: PegModel,
}

// what a search with tiles in the bag takes each player to know once the first
// move is made. InfoSets, the target: each player to move chooses blind, by the
// mean over every split of what they cannot see (weighed by its deals) of the
// other's value as the other then sees it, until the bag empties; the chance to
// win counts first, then the mean margin. Not the whole game either: a player
// infers nothing from the other's moves, and two blind players may want mixed
// strategies. RandomDraws, a fast approximation: only the first move is blind,
// both racks then known, each draw averaged over every set of tiles it may
// bring
#[derive(Clone, Copy, PartialEq)]
pub enum PegModel {
    InfoSets,
    RandomDraws,
}

impl PegBagPosition {
    // the position after a split (the other's rack and the bag), after these
    // scoreless turns, its mover to move, made in pos
    fn after_split(
        &self,
        limits: &BagLimits,
        scoreless: [u8; 2],
        opp_rack: &[u8],
        bag: &[u8],
        pos: &mut InBag,
    ) -> error::Returns<()> {
        let mover = self.mover;
        let mut racks: [&[u8]; 2] = [&[], &[]];
        racks[mover as usize] = &self.mover_rack;
        racks[(mover ^ 1) as usize] = opp_rack;
        pos.fill(limits, &self.board, racks, bag)?;
        pos.mover = mover;
        pos.passes = scoreless[0];
        pos.zeros = scoreless[1];
        pos.answering = scoreless[0] > 0; // the game's last turn a pass
        pos.exchanges = self.exchanges;
        Ok(())
    }
}

// part: (k, n) values only the splits whose index is k mod n; the n parts'
// rows, merged by their deals, are the whole search's. estimates: passes
// follow plies only to their depth, valuing a cut-short line by its score
#[derive(Clone, Copy)]
pub struct PegBagChoices {
    pub part: (usize, usize),
    pub estimates: bool,
}

impl PegBagChoices {
    pub const WHOLE: Self = Self {
        part: (0, 1),
        estimates: false,
    };
}

// a search with tiles in the bag that goes on across solves (as one with a tile
// in the bag does, PegProgress): its position, each split of the tiles the
// mover cannot see (the opponent's rack, the bag's counts, the deals giving
// it), what it is, how far it has got, and the endgames and nodes its solves
// searched on the thread solving it
pub struct PegBagProgress {
    position: PegBagPosition,
    splits: Vec<(Vec<u8>, Vec<u8>, u64)>,
    moves_at: (Vec<u8>, Vec<u8>), // the whole's first split, the moves made at it
    part: (usize, usize),
    estimates: bool,
    root_moves: Option<RootMoves>,
    exact_pass: bool,
    steered: Vec<(movegen::Play, PegBagStanding)>, // those not running
    cuts: Option<bool>,
    state: PegBagState,
    endgames: u64,
    nodes: u64,
    lanes: LanePool,       // the lane pool of its endgames on the thread solving it
    arenas: Vec<BagArena>, // each thread's search's room, this one's first, kept
}

impl PegBagProgress {
    // search root moves in one exact pass, highest-scoring placements first.
    pub fn with_exact_pass(mut self) -> error::Returns<Self> {
        if !self.state.order.is_empty() {
            return Err("choose an exact pass before starting the pre-endgame search".into());
        }
        self.exact_pass = true;
        self.estimates = false;
        Ok(self)
    }

    pub fn with_cuts(mut self) -> error::Returns<Self> {
        if !self.state.order.is_empty() {
            return Err("choose cuts before starting the pre-endgame search".into());
        }
        self.cuts = Some(true);
        Ok(self)
    }

    pub fn without_cuts(mut self) -> error::Returns<Self> {
        if !self.state.order.is_empty() {
            return Err("choose cuts before starting the pre-endgame search".into());
        }
        self.cuts = Some(false);
        Ok(self)
    }

    // a search of the position, not yet begun; an error for a bag that the
    // tiles the mover cannot see do not fill, or that holds none
    #[inline]
    pub fn new(position: PegBagPosition, choices: PegBagChoices) -> error::Returns<Self> {
        match Self::new_within(position, choices, None)? {
            Some(progress) => Ok(progress),
            None => unreachable!("no limit refuses the splits"),
        }
    }

    // new, its splits taken only where memory, if given, has room for them
    // all: else none
    #[inline]
    pub fn new_within(
        mut position: PegBagPosition,
        choices: PegBagChoices,
        memory: Option<&MemoryLimit>,
    ) -> error::Returns<Option<Self>> {
        let unseen = (position.unseen_tally.iter())
            .map(|&count| count as usize)
            .sum::<usize>();
        if position.bag_size == 0 || position.bag_size > unseen {
            return Err(format!(
                "bag_size: 1 to the {unseen} tiles the mover cannot see, found {}",
                position.bag_size
            )
            .into());
        }
        let (k, n) = choices.part;
        if k >= n {
            return Err(format!("part: k below n, found {k} of {n}").into());
        }
        position.mover_rack.sort_unstable();
        // the part's splits (every n-th from the k-th), counted first, so that
        // their list, each rack, each bag's counts, and the first split's copy
        // are taken at their full lengths, once the limit has room for them
        let rack_len = unseen - position.bag_size;
        let counts = position.unseen_tally.len();
        let walk = (counts + 1) * std::mem::size_of::<usize>() + 2 * counts;
        let Some(walking) = reserve_in(memory, walk) else {
            return Ok(None);
        };
        let mut num_splits = 0;
        each_split(&position.unseen_tally, rack_len, &mut |_, _, _| {
            num_splits += 1;
        });
        drop(walking);
        let kept = num_splits / n + usize::from(k < num_splits % n);
        let bytes = (kept.saturating_add(1))
            .saturating_mul(std::mem::size_of::<(Vec<u8>, Vec<u8>, u64)>() + rack_len + counts)
            .saturating_add(walk);
        let Some(_splits) = reserve_in(memory, bytes) else {
            return Ok(None);
        };
        let mut splits: Vec<(Vec<u8>, Vec<u8>, u64)> = Vec::new();
        splits.try_reserve_exact(kept)?;
        let mut moves_at = (Vec::new(), Vec::new());
        let mut i = 0;
        each_split(&position.unseen_tally, rack_len, &mut |rack, bag, ways| {
            let tiles = |rack: &[u8]| {
                let mut tiles = Vec::with_capacity(rack_len);
                tiles.extend(
                    (0u8..)
                        .zip(rack)
                        .flat_map(|(tile, &count)| std::iter::repeat_n(tile, count as usize)),
                );
                tiles
            };
            if i == 0 {
                moves_at = (tiles(rack), bag.to_vec());
            }
            if i % n == k {
                splits.push((tiles(rack), bag.to_vec(), ways));
            }
            i += 1;
        });
        Ok(Some(Self {
            position,
            splits,
            moves_at,
            part: choices.part,
            estimates: choices.estimates,
            root_moves: None,
            exact_pass: false,
            steered: Vec::new(),
            cuts: None,
            state: Default::default(),
            endgames: 0,
            nodes: 0,
            lanes: LanePool::new(),
            arenas: Vec::new(),
        }))
    }

    // the part it is, k of n (0 of 1 when whole)
    #[inline(always)]
    pub fn part_of(&self) -> (usize, usize) {
        self.part
    }

    // select the first plays checked for this position
    pub fn with_root_moves(mut self, root_moves: Option<RootMoves>) -> Self {
        self.root_moves = root_moves;
        self
    }

    // the endgames its solves searched on the thread solving it, and their
    // nodes (the other threads' not counted)
    #[inline(always)]
    pub fn searched(&self) -> (u64, u64) {
        (self.endgames, self.nodes)
    }

    pub fn searched_on_every_thread(&self) -> (u64, u64) {
        let (endgames, nodes) = self.state.others;
        (self.endgames + endgames, self.nodes + nodes)
    }

    // the splits each move is valued over, and the deals giving them
    #[inline(always)]
    pub fn splits(&self) -> (usize, u64) {
        (
            self.splits.len(),
            self.splits.iter().map(|split| split.2).sum(),
        )
    }

    // the pass under way: the moves it has valued, of all it values; none
    // before the search began
    #[inline(always)]
    pub fn moves(&self) -> Option<(usize, usize)> {
        let state = &self.state;
        (!state.order.is_empty()).then_some((state.next, state.order.len()))
    }

    // the moves the last solve's answer valued (PegBagResult::moves)
    #[inline(always)]
    pub fn answer_total(&self) -> usize {
        self.state.ranked.len()
    }

    // those of the last solve's answer's moves, best first, at the places in
    // range (clamped to the moves valued): the same as that slice of
    // PegBagResult::moves, a slice of the ranking kept with the answer
    pub fn answer_moves(&self, range: std::ops::Range<usize>) -> Vec<PegBagMove> {
        let state = &self.state;
        let valued = state.answer_values();
        let to = range.end.min(state.ranked.len());
        let from = range.start.min(to);
        (state.ranked[from..to].iter())
            .map(|&i| {
                let value = valued[i].as_ref().unwrap();
                PegBagMove {
                    play: state.moves[i].clone(),
                    win_pct: value.win,
                    expected_margin: (!value.cut).then_some(value.expected_margin),
                    exact: value.exact && !value.cut,
                    cut: value.cut,
                }
            })
            .collect()
    }

    // where the last solve's answer lists this move, best first (the place
    // answer_moves gives it); none if the answer did not value it
    pub fn answer_place(&self, play: &movegen::Play) -> Option<usize> {
        let state = &self.state;
        let valued = state.answer_values();
        let at =
            (0..state.moves.len()).find(|&i| valued[i].is_some() && state.moves[i] == *play)?;
        Some(state.places[at])
    }

    pub fn change(&mut self, play: &movegen::Play, change: PegBagChange) -> PegBagStanding {
        let moves = &self.state.moves;
        if !moves.is_empty() && !moves.contains(play) {
            return PegBagStanding::Absent;
        }
        let listed = self.steered.iter().position(|(other, _)| other == play);
        let was = listed.map_or(PegBagStanding::Running, |k| self.steered[k].1);
        let now = match (change, was) {
            (PegBagChange::Add, PegBagStanding::Absent)
            | (PegBagChange::Resume, PegBagStanding::Paused) => PegBagStanding::Running,
            (PegBagChange::Remove, _) => PegBagStanding::Absent,
            (PegBagChange::Pause, PegBagStanding::Running) => PegBagStanding::Paused,
            _ => was,
        };
        match (listed, now) {
            (_, _) if now == was => {}
            (Some(k), PegBagStanding::Running) => {
                self.steered.remove(k);
            }
            (Some(k), _) => self.steered[k].1 = now,
            (None, _) => self.steered.push((play.clone(), now)),
        }
        if now != was && !self.state.moves.is_empty() {
            self.state.stand(&self.steered);
            self.state.steered(play, now);
        }
        was
    }

    #[inline(always)]
    pub fn is_done(&self) -> bool {
        self.state.done
    }
}

#[derive(Clone, Copy)]
pub enum PegBagChange {
    Add,
    Remove,
    Pause,
    Resume,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum PegBagStanding {
    Absent,
    Running,
    Paused,
}

// a move's value with tiles in the bag over the splits, and its chance to win,
// each from the mover's lead before it
fn value_root_move<
    const LEAN: bool,
    N: kwg::Node,
    L: kwg::Node,
    G: movegen::LaneBits,
    const JUMPS: bool,
    const ANY_SIDE: bool,
>(
    search: &mut BagSearch<'_, '_, N, L, LEAN, G, JUMPS, ANY_SIDE>,
    splits: &[(Vec<u8>, Vec<u8>, u64)],
    at: &impl Fn(&BagLimits, &[u8], &[u8], &mut InBag) -> error::Returns<()>,
    play: &movegen::Play,
    (score_diff, beat): (f64, Option<u64>),
    order: &[usize],
    found: &mut Vec<(f64, f64)>,
) -> error::Returns<PegBagValue> {
    let (mut win, mut marg, mut total, mut won) = (0.0, 0.0, 0u64, 0u64);
    let all = splits.iter().map(|split| split.2).sum::<u64>().max(1);
    found.clear();
    search.limits.room(found, splits.len())?;
    found.resize(splits.len(), (0.0, 0.0));
    for k in 0..splits.len() {
        let s = order.get(k).copied().unwrap_or(k);
        let split = &splits[s];
        // in halves of a deal: a won split rounds up and the best's share
        // down, so a move whose share could tie the best is kept
        let most = (win + (all - total) as f64) / all as f64;
        if beat.is_some_and(|beat| won + 2 * (all - total) < beat) {
            let mean = peg_bag_value(win, marg, total, search.exact)?;
            return Ok(PegBagValue {
                win: most,
                cut: true,
                ..mean
            });
        }
        let (value, chance) = value_root_split(search, split, at, play, score_diff, None)?;
        found[s] = (value, chance);
        let ways = split.2;
        won += halves_up(chance, ways);
        win += ways as f64 * chance;
        marg += ways as f64 * value;
        total += ways;
    }
    let (mut win, mut marg) = (0.0, 0.0);
    for (split, &(value, chance)) in splits.iter().zip(found.iter()) {
        win += split.2 as f64 * chance;
        marg += split.2 as f64 * value;
    }
    peg_bag_value(win, marg, total, search.exact)
}

// a move's value with tiles in the bag in one split, and its chance to win
// there, from the mover's lead before it, under the search's model
#[inline(always)]
fn value_root_split<
    const LEAN: bool,
    N: kwg::Node,
    L: kwg::Node,
    G: movegen::LaneBits,
    const JUMPS: bool,
    const ANY_SIDE: bool,
>(
    search: &mut BagSearch<'_, '_, N, L, LEAN, G, JUMPS, ANY_SIDE>,
    (opp_rack, bag, _): &(Vec<u8>, Vec<u8>, u64),
    at: &impl Fn(&BagLimits, &[u8], &[u8], &mut InBag) -> error::Returns<()>,
    play: &movegen::Play,
    score_diff: f64,
    draws: Option<&mut Vec<PegBagDraw>>,
) -> error::Returns<(f64, f64)> {
    // the split's position, and what the mover knows there, in the arena's
    // room for the root
    let mut pos = std::mem::take(&mut search.arena.root);
    at(search.limits, opp_rack, bag, &mut pos)?;
    if search.model == PegModel::InfoSets {
        let mut info = std::mem::take(&mut search.arena.root_info);
        info.of(search.limits, &pos, score_diff as i32)?;
        search.arena.root = pos;
        let found = search.info_split(&info, play, opp_rack, bag);
        search.arena.root_info = info;
        let (win, margin) = found?;
        return Ok((margin + score_diff, win));
    }
    let found = value_root_draws(search, &mut pos, play, score_diff, draws);
    search.arena.root = pos;
    found
}

// value_root_split's value under RandomDraws at pos, with its chance to win,
// each draw the move makes listed on draws where it is given
#[inline(always)]
fn value_root_draws<
    const LEAN: bool,
    N: kwg::Node,
    L: kwg::Node,
    G: movegen::LaneBits,
    const JUMPS: bool,
    const ANY_SIDE: bool,
>(
    search: &mut BagSearch<'_, '_, N, L, LEAN, G, JUMPS, ANY_SIDE>,
    pos: &mut InBag,
    play: &movegen::Play,
    score_diff: f64,
    mut draws: Option<&mut Vec<PegBagDraw>>,
) -> error::Returns<(f64, f64)> {
    let limits = search.limits;
    let mut refused = false;
    let mut conversion = Ok(());
    let value = search.move_value_each(pos, play, |drawn, ways, value| {
        let Some(draws) = draws.as_deref_mut() else {
            return;
        };
        if refused || conversion.is_err() {
            return;
        }
        let tiles = drawn.iter().map(|&count| count as usize).sum::<usize>();
        let room = (limits.room(draws, 1)).and_then(|()| limits.reserve(tiles));
        let Ok(_tiles) = room else {
            refused = true;
            return;
        };
        match checked_expected_margin(value + score_diff) {
            Ok(margin) => draws.push(PegBagDraw {
                tiles: tiles_of(drawn),
                ways,
                margin,
                win: 0.0,
            }),
            Err(error) => conversion = Err(error),
        }
    })? + score_diff;
    if refused {
        return Err(limits.stop_refused());
    }
    conversion?;
    let mut at_draw = 0;
    let win = search.move_win_each(pos, play, -score_diff, |_, _, win| {
        if let Some(draws) = draws.as_deref_mut() {
            draws[at_draw].win = win;
        }
        at_draw += 1;
    })?;
    debug_assert!(draws.is_none_or(|draws| at_draw == draws.len()));
    Ok((value, win))
}

fn tiles_of(tally: &[u8]) -> Vec<u8> {
    let mut tiles = Vec::with_capacity(tally.iter().map(|&count| count as usize).sum());
    tiles.extend(
        (0u8..)
            .zip(tally.iter())
            .flat_map(|(tile, &count)| std::iter::repeat_n(tile, count as usize)),
    );
    tiles
}

fn halves_up(chance: f64, ways: u64) -> u64 {
    (chance * (2 * ways) as f64).ceil() as u64
}

fn halves_down(share: f64, splits: &[(Vec<u8>, Vec<u8>, u64)]) -> u64 {
    let all = splits.iter().map(|split| split.2).sum::<u64>().max(1);
    (share * (2 * all) as f64).floor() as u64
}

// a move's value, summed over its splits weighed by the deals giving them, and
// whether every endgame valuing it was exact
fn peg_bag_value(win: f64, marg: f64, total: u64, exact: bool) -> error::Returns<PegBagValue> {
    // a part given no split (PegBagProgress::part) has no deals to weigh
    let total = total.max(1);
    let marg = marg / total as f64;
    Ok(PegBagValue {
        win: win / total as f64,
        marg,
        expected_margin: checked_expected_margin(marg)?,
        exact,
        cut: false,
    })
}

// what a pass values each move against: each split of the unseen tiles (the
// other's rack, the bag, and the deals that give it), the position a split
// makes (at), the mover's lead, and the threads' arenas
struct PegBagRoot<'r, At: Fn(&BagLimits, &[u8], &[u8], &mut InBag) -> error::Returns<()> + Sync> {
    splits: &'r [(Vec<u8>, Vec<u8>, u64)],
    at: &'r At,
    score_diff: f64,
    arenas: &'r mut Vec<BagArena>, // each thread's, as PegBagProgress keeps them
    model: PegModel,
    cuts: bool,
}

// the search so far: the moves; the order its pass values them in, the next to
// value, each one's latest value, the time each of its splits took on threads
// in the latest pass to value it, and those the pass has valued; whether all
// the pass's endgames were exact, and its depth; what the search has valued
// below the moves; the table and lane pool each thread but the calling one
// left; the best move of the deepest pass that valued every move, with that
// depth, that pass's values, and what it had valued below the moves as it
// ended (none where the memory limit refused the room); whether that answer is
// final; and the answer's valued moves best first, with each move's place
// among them
#[derive(Default)]
struct PegBagState {
    moves: Vec<movegen::Play>,
    order: Vec<usize>,
    next: usize,
    valued: Vec<Option<PegBagValue>>,
    took: Vec<std::time::Duration>,
    reached: Vec<usize>,
    pass_exact: bool,
    depth: i8,
    searched: BagValued,
    tables: Vec<Option<KeptTable>>,
    lanes: Vec<LanePool>,
    answer: Option<(usize, i8)>,
    answered: Vec<Option<PegBagValue>>,
    answered_searched: Option<BagValued>,
    done: bool,
    ranked: Vec<usize>,
    places: Vec<usize>,
    standing: Vec<u8>, // each move's (STAND_*), empty until one is steered
    stale: Vec<bool>,  // each answer value kept from an older pass
    split_order: Vec<usize>,
    split_found: Vec<(f64, f64)>,
    others: (u64, u64),
    lost: Vec<std::sync::atomic::AtomicU64>,
    won: Vec<std::sync::atomic::AtomicU64>,
    cut: Vec<std::sync::atomic::AtomicBool>,
}

// a move's standing in PegBagState::standing, and the marks steer_order sets
const STAND_PAUSED: u8 = 1;
const STAND_ABSENT: u8 = 2;
const MARK_REACHED: u8 = 4;
const MARK_TAKEN: u8 = 8;

impl PegBagState {
    fn stand(&mut self, steered: &[(movegen::Play, PegBagStanding)]) {
        if steered.is_empty() && self.standing.is_empty() {
            return;
        }
        self.standing.clear();
        self.standing.extend(self.moves.iter().map(|play| {
            match steered.iter().find(|(other, _)| other == play) {
                Some((_, PegBagStanding::Paused)) => STAND_PAUSED,
                Some((_, PegBagStanding::Absent)) => STAND_ABSENT,
                _ => 0,
            }
        }));
        self.stale.resize(self.moves.len(), false);
    }

    #[inline(always)]
    fn live(&self, i: usize) -> bool {
        self.standing.get(i).is_none_or(|&s| s & 3 == 0)
    }

    fn steered(&mut self, play: &movegen::Play, now: PegBagStanding) {
        let Some(i) = self.moves.iter().position(|other| other == play) else {
            return;
        };
        if now == PegBagStanding::Absent {
            self.valued[i] = None;
            if let Some(value) = self.answered.get_mut(i) {
                *value = None;
            }
        }
        if now != PegBagStanding::Running {
            self.reached.retain(|&other| other != i);
            for values in [&mut self.valued, &mut self.answered] {
                for value in values.iter_mut() {
                    if value.as_ref().is_some_and(|value| value.cut) {
                        *value = None;
                    }
                }
            }
            let valued = &self.valued;
            (self.reached).retain(|&other| valued[other].is_some());
        }
        if let Some((_, depth)) = self.answer {
            let live = (0..self.moves.len())
                .filter(|&k| self.live(k) && self.answered[k].is_some() && !self.stale[k])
                .collect::<Vec<_>>();
            if !live.is_empty() {
                self.answer = Some((peg_bag_best(&live, &self.answered), depth));
            }
        }
        let values = self.answer_values();
        let lacking = (0..self.moves.len())
            .any(|k| self.live(k) && (values[k].is_none() || self.stale.get(k) == Some(&true)));
        if self.done && lacking {
            self.reopen();
        }
        self.rank_answer();
    }

    fn reopen(&mut self) {
        self.done = false;
        if self.answered.len() == self.moves.len() {
            self.valued.clone_from(&self.answered);
        }
        self.reached.clear();
        for i in 0..self.moves.len() {
            if self.live(i) && self.valued[i].is_some() && !self.stale[i] {
                self.reached.push(i);
            }
        }
        self.order.clear();
        self.order.extend_from_slice(&self.reached);
        self.next = self.reached.len();
        self.pass_exact = (self.reached.iter()).all(|&i| self.valued[i].as_ref().unwrap().exact);
    }

    fn steer_order(&mut self) -> bool {
        for &i in &self.reached {
            self.standing[i] |= MARK_REACHED;
        }
        let mut keep = 0;
        for k in 0..self.order.len() {
            let i = self.order[k];
            let s = self.standing[i];
            if (k < self.next && s & MARK_REACHED != 0) || (k >= self.next && s & 3 == 0) {
                self.order[keep] = i;
                keep += 1;
            }
        }
        self.order.truncate(keep);
        self.next = self.reached.len();
        for &i in &self.order {
            self.standing[i] |= MARK_TAKEN;
        }
        for i in 0..self.moves.len() {
            if self.standing[i] == 0 {
                self.order.push(i);
            }
            self.standing[i] &= 3;
        }
        (0..self.moves.len()).any(|i| self.live(i))
    }

    fn carry_paused(&mut self) {
        for i in 0..self.moves.len() {
            self.stale[i] = false;
            if self.standing[i] & STAND_PAUSED != 0 && self.valued[i].is_none() {
                self.valued[i] = self.answered.get(i).cloned().flatten();
                self.stale[i] = self.valued[i].is_some();
            }
        }
    }

    // the values the answer is of: the deepest pass's that valued every move,
    // or, before one did, the pass's under way
    #[inline(always)]
    fn answer_values(&self) -> &[Option<PegBagValue>] {
        if self.answer.is_some() {
            &self.answered
        } else {
            &self.valued
        }
    }

    // rank the answer's valued moves, best first, and note each one's place:
    // made when an answer is swapped in, and after a solve while the pass
    // under way is the answer
    #[inline]
    fn rank_answer(&mut self) {
        let valued = if self.answer.is_some() {
            &self.answered
        } else {
            &self.valued
        };
        self.ranked.clear();
        (self.ranked).extend((0..valued.len()).filter(|&i| valued[i].is_some()));
        (self.ranked).sort_unstable_by(|&a, &b| peg_bag_ranks(valued, a, b));
        if !self.standing.is_empty() {
            let standing = &self.standing;
            (self.ranked).sort_by_key(|&i| standing[i] & 3 != 0);
        }
        self.places.clear();
        self.places.resize(valued.len(), 0);
        for (place, &i) in self.ranked.iter().enumerate() {
            self.places[i] = place;
        }
    }
}

// a move's value with tiles in the bag: the share of deals it wins (a tie
// counted half), its mean margin, and whether every endgame valuing it was
// searched to its end
#[derive(Clone)]
struct PegBagValue {
    win: f64,
    marg: f64,
    expected_margin: equity::Equity,
    exact: bool,
    cut: bool,
}

// a move's rows with tiles in the bag, for some of its splits: each split's
// (the bag's tiles, the deals giving it, the margin there), its chance to win
// there (wins) and, where the model draws at the root (random draws), each
// draw the move makes there (draws), all in the splits' order
#[derive(Default, PartialEq)]
pub struct PegBagRows {
    pub hypotheses: Vec<(Vec<u8>, u64, equity::Equity)>,
    pub wins: Vec<f64>,
    pub draws: Vec<Vec<PegBagDraw>>,
}

// why a move has no rows: the answer has no value of it, or a limit stopped
// their search (out of memory when the memory limit did)
#[derive(Debug)]
pub enum PegBagRowsMissing {
    Unvalued,
    Stopped { out_of_memory: bool },
}

// a draw a move makes at the root with tiles left in the bag: the tiles it
// draws, the deals that give them, the margin then, and the chance to win
#[derive(PartialEq)]
pub struct PegBagDraw {
    pub tiles: Vec<u8>,
    pub ways: u64,
    pub margin: equity::Equity,
    pub win: f64,
}

// a move of a search with tiles in the bag and its value: its share of the
// deals won (a tie counted half), its mean margin, and whether every endgame
// valuing it was searched to its end; its rows are
// EndgameSolver::peg_bag_rows's
pub struct PegBagMove {
    pub play: movegen::Play,
    pub win_pct: f64,
    pub expected_margin: Option<equity::Equity>,
    pub exact: bool,
    pub cut: bool,
}

// the leader and coverage of a completed pass before the next pass starts.
pub struct PegBagPass {
    pub depth: i8,
    pub moves: usize,
    pub leader: PegBagMove,
}

type PegBagWatch<'priority, 'report> = (
    &'priority [movegen::Play],
    Option<&'report mut dyn FnMut(PegBagPass) -> error::Returns<()>>,
);

// a search's answer with tiles in the bag: the best move's share of wins and
// mean margin; the move, none if a limit stopped the search before it valued
// one (the share and margin then 0); every move the answering pass valued,
// best first, each with its own; whether it is exact; and the pass it is of (0
// none finished, i8::MAX one to the end)
pub struct PegBagResult {
    pub win_pct: f64,
    pub expected_margin: equity::Equity,
    pub best_move: Option<movegen::Play>,
    pub moves: Vec<PegBagMove>,
    pub exact: bool,
    pub depth: i8,
    pub out_of_memory: bool, // the memory limit stopped the search, before it grew past it
}

fn peg_bag_answer(state: &PegBagState, out_of_memory: bool) -> PegBagResult {
    match state.answer {
        Some((best, depth)) => peg_bag_result(
            &state.moves,
            &state.answered,
            Some(best),
            depth,
            state.done && state.pass_exact,
            out_of_memory,
        ),
        None => {
            let best =
                (!state.reached.is_empty()).then(|| peg_bag_best(&state.reached, &state.valued));
            peg_bag_result(&state.moves, &state.valued, best, 0, false, out_of_memory)
        }
    }
}

// the best of these moves: the most wins, then the best mean margin, ties going
// to the move found first
fn peg_bag_best(reached: &[usize], valued: &[Option<PegBagValue>]) -> usize {
    let mut best = reached[0];
    for &i in &reached[1..] {
        let (value, other) = (valued[i].as_ref().unwrap(), valued[best].as_ref().unwrap());
        if value.win > other.win
            || (value.win == other.win
                && (value.marg > other.marg || (value.marg == other.marg && i < best)))
        {
            best = i;
        }
    }
    best
}

// the answer from these values: the best move's, if any, without the moves
// (PegBagProgress::answer_moves lists them)
fn peg_bag_result(
    moves: &[movegen::Play],
    valued: &[Option<PegBagValue>],
    best: Option<usize>,
    depth: i8,
    exact: bool,
    out_of_memory: bool,
) -> PegBagResult {
    let value = best.map(|best| valued[best].as_ref().unwrap());
    PegBagResult {
        win_pct: value.map_or(0.0, |value| value.win),
        expected_margin: value.map_or(equity::Equity::ZERO, |value| value.expected_margin),
        best_move: best.map(|best| moves[best].clone()),
        moves: Vec::new(),
        exact,
        depth,
        out_of_memory,
    }
}

// the first ranks before the second, best first as peg_bag_best ranks them
fn peg_bag_ranks(valued: &[Option<PegBagValue>], a: usize, b: usize) -> std::cmp::Ordering {
    let (x, y) = (valued[a].as_ref().unwrap(), valued[b].as_ref().unwrap());
    (y.win.total_cmp(&x.win))
        .then(y.marg.total_cmp(&x.marg))
        .then(a.cmp(&b))
}

// a one-in-bag position's turn: the mover, the tile in the bag, the scoreless
// turns before it (the passes and the zeros), and whether the move before it was
// a pass
struct OneInBagTurn {
    mover: u8,
    bag_tile: u8,
    passes: u8,
    zeros: u8,
    answering: bool,
}

// the one-in-bag search's fixed parts, carried down its recursion
struct OneInBag<
    's,
    'a,
    N: kwg::Node,
    L: kwg::Node,
    const LEAN: bool,
    G: movegen::LaneBits,
    const JUMPS: bool,
    const ANY_SIDE: bool,
> {
    gc: &'s game_config::GameConfig,
    kwg: &'s kwg::Kwg<N>,
    arm_tables: &'s anagrams::ArmTables,
    klv: &'s klv::Klv<L>,
    sub: &'s mut EndgameSolver<'a, N, L, LEAN, G, JUMPS, ANY_SIDE>,
    mg: &'s mut movegen::KurniaMoveGenerator<G, JUMPS, ANY_SIDE>,
    memo: &'s mut BagTable<equity::Equity>, // the caller's, kept from call to call
    limits: &'s BagLimits,
    exact: bool, // cleared by an endgame a limit cut short
    line: Option<&'s mut PassLine>,
    placed: &'s mut Placed, // the caller's, refilled for each place
}

// the board and racks after a place of the one-in-bag search, refilled for
// each place it values (none is valued while another is, as the search goes
// down only after its places), kept by the caller from call to call
#[derive(Default)]
struct Placed {
    board: Vec<u8>,
    racks: [Vec<u8>; 2],
}

// a deepening pass's search after the committed model's pass, kept across
// solves: each position of its line (by the tile in the bag, the mover, the
// passes, and the zeros) valued, with whether every endgame inside was exact,
// or stopped by a limit, with the places it had valued, the best of them, and
// whether those were exact
#[derive(Default)]
struct PassLine {
    valued: fash::MyHashMap<[u8; 5], (equity::Equity, bool)>,
    partway: fash::MyHashMap<[u8; 5], (usize, equity::Equity, bool)>,
}

// a position with a tile in the bag as the peg model searches it: the mover,
// the board, the mover's rack, the tiles they cannot see with each one's count,
// the alphabet's tile count (blank included: a move's run of margins, one a
// tile), and the scoreless turns before it (the passes and the zeros)
struct PegPosition<'a> {
    mover: u8,
    board: &'a [u8],
    mover_rack: &'a [u8],
    unseen: &'a [(u8, u32)],
    tiles: usize,
    scoreless: [u8; 2],
    root_moves: Option<&'a RootMoves>,
    priority: &'a [movegen::Play],
}

#[inline(always)]
fn scoreless_counts(counts: [u16; 2]) -> [u8; 2] {
    counts.map(|count| count.min(u16::from(u8::MAX)) as u8)
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

// count the one-in-bag win and mean before clamping the reported margins to i32
#[derive(Default)]
struct PegOutcomes {
    deals: u64,
    wins: f64,
    margin: f64,
}

impl PegOutcomes {
    #[inline]
    fn record(&mut self, weight: u32, margin: i64) -> error::Returns<()> {
        self.deals = self
            .deals
            .checked_add(u64::from(weight))
            .ok_or("deal count is out of range")?;
        let weight = f64::from(weight);
        self.wins += weight
            * match margin.cmp(&0) {
                std::cmp::Ordering::Greater => 1.0,
                std::cmp::Ordering::Equal => 0.5,
                std::cmp::Ordering::Less => 0.0,
            };
        self.margin += weight * margin as f64;
        Ok(())
    }

    #[inline]
    fn averages(&self) -> (f64, f64) {
        if self.deals == 0 {
            (0.0, 0.0)
        } else {
            let deals = self.deals as f64;
            (self.wins / deals, self.margin / deals)
        }
    }
}

#[inline]
pub fn peg_aggregate(
    hypotheses: &[(u32, equity::Equity)],
) -> error::Returns<(f64, equity::Equity)> {
    let mut outcomes = PegOutcomes::default();
    for &(weight, value) in hypotheses {
        outcomes.record(weight, i64::from(value.raw()))?;
    }
    let (win, margin) = outcomes.averages();
    Ok((win, checked_expected_margin(margin)?))
}

// how many ways there are to choose k of n alike tiles, a deal telling them
// apart (each step's product divides exactly: it is C(n, i + 1) times i + 1)
#[inline(always)]
fn binom(n: usize, k: usize) -> u64 {
    (0..k).fold(1u64, |ways, i| ways * (n - i) as u64 / (i + 1) as u64)
}

// every way a pool of tiles (a count per tile) splits into n tiles and the
// rest, to f: the n's counts, the rest's counts, and how many deals give that
// split, the product, tile by tile, of the ways to choose the n's copies of it.
// The splits of the tiles one player cannot see into the other's rack and the
// bag, and those of the bag into a draw and what stays, are each this
fn each_split(pool: &[u8], n: usize, f: &mut impl FnMut(&[u8], &[u8], u64)) {
    let mut cap = vec![0usize; pool.len() + 1];
    for t in (0..pool.len()).rev() {
        cap[t] = cap[t + 1] + pool[t] as usize;
    }
    let mut part = vec![0u8; pool.len()];
    let mut rest = vec![0u8; pool.len()];
    infer::each_tally_of_size(pool, &cap, 0, n, &mut part, &mut |part| {
        let mut ways = 1u64;
        for (t, (&p, &taken)) in pool.iter().zip(part).enumerate() {
            rest[t] = p - taken;
            ways *= binom(p as usize, taken as usize);
        }
        f(part, &rest, ways);
    });
}

// a position with tiles in the bag as a search with the racks known walks it:
// the board, each rack (sorted), the bag's counts, the player to move, the
// scoreless turns before it (the passes, the zeros, and whether the line's move
// before was a pass), and each player's exchanges so far
#[derive(Clone, Default)]
struct InBag {
    board: Vec<u8>,
    racks: [Vec<u8>; 2],
    bag: Vec<u8>,
    mover: u8,
    passes: u8,
    zeros: u8,
    answering: bool, // the line's move before was a pass
    exchanges: [i16; 2],
}

impl InBag {
    #[inline(always)]
    fn fill(
        &mut self,
        limits: &BagLimits,
        board: &[u8],
        racks: [&[u8]; 2],
        bag: &[u8],
    ) -> error::Returns<()> {
        let bag_len = bag.iter().map(|&count| count as usize).sum::<usize>();
        self.board.clear();
        limits.room(&mut self.board, board.len())?;
        self.board.extend_from_slice(board);
        for (rack, from) in self.racks.iter_mut().zip(racks) {
            rack.clear();
            limits.room(rack, from.len() + bag_len)?;
            rack.extend_from_slice(from);
        }
        self.bag.clear();
        limits.room(&mut self.bag, bag.len())?;
        self.bag.extend_from_slice(bag);
        Ok(())
    }

    // the position's key: its board's tiles, each rack's counts, the bag's
    // counts, the mover, the scoreless turns, and the exchanges made
    fn key(&self) -> u64 {
        let mut key = 0;
        for (square, &tile) in self.board.iter().enumerate() {
            if tile != 0 {
                key ^= square_key(square, tile);
            }
        }
        for (player, rack) in (0u8..).zip(self.racks.iter()) {
            key ^= rack_key(player, rack);
        }
        for (tile, &count) in self.bag.iter().enumerate() {
            if count != 0 {
                key ^= zobrist(BAG_KEYS | ((tile as u64) << 8) | u64::from(count));
            }
        }
        if self.mover != 0 {
            key ^= zobrist(SECOND_TO_MOVE);
        }
        key ^= Scoreless::Counted {
            passes: self.passes,
            zeros: self.zeros,
        }
        .key();
        if self.answering {
            key ^= Scoreless::Answering.key();
        }
        for (player, &made) in self.exchanges.iter().enumerate() {
            if made != 0 {
                key ^= exchanges_made_key(player, made);
            }
        }
        key
    }

    // whether a pass here ends the line: answering a pass the line made, or
    // bringing the scoreless turns to the config's count
    #[inline(always)]
    fn a_pass_ends(&self, gc: &game_config::GameConfig) -> bool {
        self.answering
            || scoreless_turns_end(
                gc,
                Some(&self.board),
                self.passes.saturating_add(1),
                self.zeros.saturating_add(1),
            )
    }

    // the scoreless turns before it, to set back once a move is unmade
    #[inline(always)]
    fn turn(&self) -> InBagTurn {
        InBagTurn {
            passes: self.passes,
            zeros: self.zeros,
            answering: self.answering,
        }
    }

    #[inline(always)]
    fn set_turn(&mut self, turn: InBagTurn) {
        self.passes = turn.passes;
        self.zeros = turn.zeros;
        self.answering = turn.answering;
    }

    // the mover passes, the turn not yet passed on; a pass there answers this
    // one. What set_turn unmakes it with
    #[inline(always)]
    fn pass(&mut self) -> InBagTurn {
        let turn = self.turn();
        self.passes = self.passes.saturating_add(1);
        self.zeros = self.zeros.saturating_add(1);
        self.answering = true;
        turn
    }

    // how many tiles the bag holds
    #[inline(always)]
    fn bag_len(&self) -> usize {
        self.bag.iter().map(|&count| count as usize).sum()
    }

    // the mover places play, before they draw: the tiles on the board, off the
    // rack, and both scoreless counts back to none. What unplace unmakes it
    // with
    fn place(&mut self, gc: &game_config::GameConfig, play: &movegen::Play) -> InBagTurn {
        let turn = self.turn();
        if let movegen::Play::Place {
            down,
            lane,
            idx,
            word,
            ..
        } = play
        {
            let strider = gc.board_layout().dim().lane(*down, *lane);
            let rack = &mut self.racks[self.mover as usize];
            for (i, &tile) in (*idx..).zip(word.iter()) {
                if tile != 0 {
                    self.board[strider.at_in_lane(i)] = tile;
                    let held = held_tile(tile);
                    let p = rack.iter().rposition(|&t| t == held).unwrap();
                    rack.remove(p);
                }
            }
        }
        self.passes = 0;
        self.zeros = 0;
        self.answering = false;
        turn
    }

    // place unmade: the tiles off the board, back on the rack (sorted), and
    // the scoreless turns as they were
    fn unplace(&mut self, gc: &game_config::GameConfig, play: &movegen::Play, turn: InBagTurn) {
        if let movegen::Play::Place {
            down,
            lane,
            idx,
            word,
            ..
        } = play
        {
            let strider = gc.board_layout().dim().lane(*down, *lane);
            let rack = &mut self.racks[self.mover as usize];
            for (i, &tile) in (*idx..).zip(word.iter()) {
                if tile != 0 {
                    self.board[strider.at_in_lane(i)] = 0;
                    let held = held_tile(tile);
                    rack.insert(rack.partition_point(|&t| t <= held), held);
                }
            }
        }
        self.set_turn(turn);
    }

    // the mover draws these tiles (a count per tile) from the bag
    fn draw(&mut self, drawn: &[u8]) {
        let rack = &mut self.racks[self.mover as usize];
        for (tile, (&d, count)) in (0u8..).zip(drawn.iter().zip(self.bag.iter_mut())) {
            *count -= d;
            rack.extend(std::iter::repeat_n(tile, d as usize));
        }
        rack.sort_unstable();
    }

    // draw unmade: the tiles (a count per tile) off the mover's rack (sorted),
    // back in the bag
    fn undraw(&mut self, drawn: &[u8]) {
        let rack = &mut self.racks[self.mover as usize];
        for (tile, (&d, count)) in (0u8..).zip(drawn.iter().zip(self.bag.iter_mut())) {
            *count += d;
            if d != 0 {
                let p = rack.partition_point(|&t| t < tile);
                rack.drain(p..p + d as usize);
            }
        }
    }

    // the mover puts these tiles (a count per tile) back in the bag
    fn put_back(&mut self, tiles: &[u8]) {
        let rack = &mut self.racks[self.mover as usize];
        for (tile, (&n, count)) in (0u8..).zip(tiles.iter().zip(self.bag.iter_mut())) {
            *count += n;
            for _ in 0..n {
                let p = rack.iter().position(|&t| t == tile).unwrap();
                rack.remove(p);
            }
        }
    }

    // put_back unmade: the tiles (a count per tile) out of the bag, back on
    // the mover's rack (sorted)
    fn take_back(&mut self, tiles: &[u8]) {
        let rack = &mut self.racks[self.mover as usize];
        for (tile, (&n, count)) in (0u8..).zip(tiles.iter().zip(self.bag.iter_mut())) {
            *count -= n;
            if n != 0 {
                let p = rack.partition_point(|&t| t <= tile);
                rack.splice(p..p, std::iter::repeat_n(tile, n as usize));
            }
        }
    }

    // the mover exchanges: draws these tiles, then puts those back (a count
    // per tile each), as GameState::play makes them, the scoreless turns then
    // none but these zeros, and one more exchange made. What unexchange
    // unmakes it with
    fn exchange(&mut self, drawn: &[u8], thrown: &[u8], zeros: u8) -> InBagTurn {
        let turn = self.turn();
        self.draw(drawn);
        self.put_back(thrown);
        self.passes = 0;
        self.zeros = zeros;
        self.answering = false;
        self.exchanges[self.mover as usize] += 1;
        turn
    }

    fn unexchange(&mut self, drawn: &[u8], thrown: &[u8], turn: InBagTurn) {
        self.exchanges[self.mover as usize] -= 1;
        self.set_turn(turn);
        self.take_back(thrown);
        self.undraw(drawn);
    }

    // the turn passes to the other player
    #[inline(always)]
    fn next_turn(&mut self) {
        self.mover ^= 1;
    }

    // what the game's end is worth to the mover when scoreless turns end it:
    // each player loses their own rack
    #[inline(always)]
    fn racks_lost(&self, gc: &game_config::GameConfig) -> f64 {
        let alphabet = gc.alphabet();
        f64::from(
            (alphabet.scaled_rack_score(&self.racks[(self.mover ^ 1) as usize])
                - alphabet.scaled_rack_score(&self.racks[self.mover as usize]))
            .raw(),
        )
    }
}

// a position's scoreless turns before it (InBag), kept to unmake a move
struct InBagTurn {
    passes: u8,
    zeros: u8,
    answering: bool,
}

// what the player to move knows with tiles in the bag, as the search over each
// player's own information walks it: the board, their rack (sorted), what they
// cannot see (a count per tile: the other's rack with the bag), how many of
// those are in the bag, which player they are, the scoreless turns before it,
// each player's exchanges so far, and their lead
#[derive(Clone, Default)]
struct InfoSet {
    board: Vec<u8>,
    rack: Vec<u8>,
    unseen: Vec<u8>,
    bag_len: usize,
    mover: u8,
    passes: u8,
    zeros: u8,
    answering: bool, // the line's move before was a pass
    exchanges: [i16; 2],
    lead: i32,
}

impl InfoSet {
    // made over as what the mover of pos knows, with this lead, keeping its
    // buffers
    fn of(&mut self, limits: &BagLimits, pos: &InBag, lead: i32) -> error::Returns<()> {
        self.fill(
            limits,
            &pos.board,
            &pos.racks[pos.mover as usize],
            &pos.bag,
            &pos.racks[(pos.mover ^ 1) as usize],
        )?;
        self.bag_len = pos.bag_len();
        self.mover = pos.mover;
        self.passes = pos.passes;
        self.zeros = pos.zeros;
        self.answering = pos.answering;
        self.exchanges = pos.exchanges;
        self.lead = lead;
        Ok(())
    }

    // made over with this board and rack, what it cannot see the bag's counts
    // with the tiles of unseen, keeping its buffers, within the limits
    #[inline(always)]
    fn fill(
        &mut self,
        limits: &BagLimits,
        board: &[u8],
        rack: &[u8],
        bag: &[u8],
        unseen: &[u8],
    ) -> error::Returns<()> {
        self.board.clear();
        limits.room(&mut self.board, board.len())?;
        self.board.extend_from_slice(board);
        self.rack.clear();
        limits.room(&mut self.rack, rack.len())?;
        self.rack.extend_from_slice(rack);
        self.unseen.clear();
        limits.room(&mut self.unseen, bag.len())?;
        self.unseen.extend_from_slice(bag);
        for &tile in unseen {
            self.unseen[tile as usize] += 1;
        }
        Ok(())
    }

    // its key: the board's tiles, the mover's rack's counts, the mover, the
    // scoreless turns (and whether the move before was a pass), the exchanges
    // made, and the lead (what the mover cannot see follows from the board with
    // the rack), kept apart from a position's
    fn key(&self) -> u64 {
        let mut key = zobrist(INFO_SETS);
        for (square, &tile) in self.board.iter().enumerate() {
            if tile != 0 {
                key ^= square_key(square, tile);
            }
        }
        key ^= rack_key(self.mover, &self.rack);
        if self.mover != 0 {
            key ^= zobrist(SECOND_TO_MOVE);
        }
        key ^= Scoreless::Counted {
            passes: self.passes,
            zeros: self.zeros,
        }
        .key();
        if self.answering {
            key ^= Scoreless::Answering.key();
        }
        for (player, &made) in self.exchanges.iter().enumerate() {
            if made != 0 {
                key ^= exchanges_made_key(player, made);
            }
        }
        key ^ info_lead_key(self.lead)
    }

    // each split of what the mover cannot see into the other's rack (sorted)
    // and the bag (a count per tile), with the deals that give it, in
    // each_split's order, written over the first entries of splits (its
    // entries' buffers kept, one added where it has too few), drawn the room
    // to walk them in; how many
    fn splits(
        &self,
        limits: &BagLimits,
        splits: &mut Vec<(Vec<u8>, Vec<u8>, u64)>,
        drawn: &mut Vec<u8>,
    ) -> error::Returns<usize> {
        let pool = &self.unseen;
        let unseen = pool.iter().map(|&count| count as usize).sum::<usize>();
        let rack_len = unseen - self.bag_len;
        drawn.clear();
        limits.room(drawn, pool.len())?;
        drawn.resize(pool.len(), 0);
        let mut num = 0;
        let mut more = first_draw(pool, rack_len, drawn);
        while more {
            if num == splits.len() {
                limits.room(splits, 1)?;
                splits.push(Default::default());
            }
            let (rack, bag, ways) = &mut splits[num];
            rack.clear();
            bag.clear();
            limits.room(rack, rack_len)?;
            limits.room(bag, pool.len())?;
            for (tile, (&count, &d)) in (0u8..).zip(pool.iter().zip(drawn.iter())) {
                rack.extend(std::iter::repeat_n(tile, d as usize));
                bag.push(count - d);
            }
            *ways = draw_ways(pool, drawn);
            num += 1;
            more = next_draw(pool, drawn);
        }
        Ok(num)
    }

    // the position of one split, everything known, made in pos
    fn at(
        &self,
        limits: &BagLimits,
        other: &[u8],
        bag: &[u8],
        pos: &mut InBag,
    ) -> error::Returns<()> {
        let mut racks: [&[u8]; 2] = [&[], &[]];
        racks[self.mover as usize] = &self.rack;
        racks[(self.mover ^ 1) as usize] = other;
        pos.fill(limits, &self.board, racks, bag)?;
        pos.mover = self.mover;
        pos.passes = self.passes;
        pos.zeros = self.zeros;
        pos.answering = self.answering;
        pos.exchanges = self.exchanges;
        Ok(())
    }

    // what the other knows at one split (their rack other, the bag's counts)
    // after the mover's turn: this board, the mover's tiles sees seen with the
    // bag's, bag_len in the bag, these scoreless turns and exchanges, and this
    // lead; made in next
    #[inline(always)]
    fn theirs(
        &self,
        limits: &BagLimits,
        next: &mut InfoSet,
        (board, other, bag, sees): Tiles4,
        turn: TheirTurn,
    ) -> error::Returns<()> {
        next.fill(limits, board, other, bag, sees)?;
        next.bag_len = turn.bag_len;
        next.mover = self.mover ^ 1;
        next.passes = turn.passes;
        next.zeros = turn.zeros;
        next.answering = turn.answering;
        next.exchanges = turn.exchanges;
        next.lead = turn.lead;
        Ok(())
    }
}

// a split of what a player cannot see: the other's rack (sorted), the bag's
// counts, and the deals that give it
type Split = (Vec<u8>, Vec<u8>, u64);

// InfoSet::theirs's board, the other's rack, the bag's counts, and the tiles
// the other sees
type Tiles4<'t> = (&'t [u8], &'t [u8], &'t [u8], &'t [u8]);

// InfoSet::theirs's bag length, scoreless turns, exchanges, and lead
struct TheirTurn {
    bag_len: usize,
    passes: u8,
    zeros: u8,
    answering: bool,
    exchanges: [i16; 2],
    lead: i32,
}

// the moves at a position with tiles in the bag, in the order a tie between
// them goes to the first: the pass, the placements as the generator finds them,
// then the exchanges the bag allows, by how many tiles, then by tile
fn in_bag_moves<
    const LEAN: bool,
    N: kwg::Node,
    L: kwg::Node,
    G: movegen::LaneBits,
    const JUMPS: bool,
    const ANY_SIDE: bool,
>(
    gc: &game_config::GameConfig,
    kwg: &kwg::Kwg<N>,
    arm_tables: &anagrams::ArmTables,
    klv: &klv::Klv<L>,
    mg: &mut movegen::KurniaMoveGenerator<G, JUMPS, ANY_SIDE>,
    pos: &InBag,
    (lists, limits): (&mut PlyLists, &BagLimits),
) -> error::Returns<()> {
    let mover = pos.mover as usize;
    let (rack, made) = (&pos.racks[mover], pos.exchanges[mover]);
    moves_of::<LEAN, _, _, _, JUMPS, ANY_SIDE>(
        mg,
        &movegen::BoardSnapshot {
            board_tiles: &pos.board,
            game_config: gc,
            kwg,
            anagrams: arm_tables.anagrams(),
            rack_lengths: arm_tables.rack_lengths(),
            klv,
        },
        rack,
        made,
        pos.bag_len(),
        (&mut lists.moves, &mut lists.scratch),
        limits,
    )
}

// in_bag_moves's moves on the snapshot's board for this rack, the mover having
// exchanged made times, with bag_len tiles in the bag, into moves (cleared);
// under a memory limit, each list grown and each play's tiles taken within it,
// the placements moved out of the generator's list (root_moves's way)
fn moves_of<
    const LEAN: bool,
    N: kwg::Node,
    L: kwg::Node,
    G: movegen::LaneBits,
    const JUMPS: bool,
    const ANY_SIDE: bool,
>(
    mg: &mut movegen::KurniaMoveGenerator<G, JUMPS, ANY_SIDE>,
    snapshot: &movegen::BoardSnapshot<'_, N, L>,
    rack: &[u8],
    made: i16,
    bag_len: usize,
    (moves, scratch): (&mut Vec<movegen::Play>, &mut Vec<u8>),
    limits: &BagLimits,
) -> error::Returns<()> {
    let gc = snapshot.game_config;
    moves.clear();
    if let Some(memory) = limits.memory {
        let cut = mg.gen_moves_raw_all_unsorted_held(
            snapshot,
            rack,
            made,
            movegen::PassPolicy::OnlyWhenForced,
            movegen::HeldBytes { memory, kept: 0 },
        )?;
        if cut {
            return Err(limits.stop_refused());
        }
        let places = (mg.plays.iter())
            .filter(|vm| matches!(vm.play, movegen::Play::Place { .. }))
            .count();
        scratch.clear();
        limits.room(scratch, 2 * gc.alphabet().len() as usize + rack.len())?;
        let (mut exchanges, mut tiles) = (0, 0);
        each_exchange(gc, rack, made, bag_len, scratch, |thrown| {
            exchanges += 1;
            tiles += bites::Bites::heap_bytes(thrown.len());
        });
        limits.room(moves, 1 + places + exchanges)?;
        let _tiles = limits.reserve(tiles)?;
        moves.push(movegen::Play::Exchange {
            tiles: [][..].into(),
        });
        moves.extend(
            (mg.plays.drain(..))
                .filter(|vm| matches!(vm.play, movegen::Play::Place { .. }))
                .map(|vm| vm.play),
        );
        each_exchange(gc, rack, made, bag_len, scratch, |tiles| {
            moves.push(movegen::Play::Exchange {
                tiles: tiles.into(),
            })
        });
        return Ok(());
    }
    moves.push(movegen::Play::Exchange {
        tiles: [][..].into(),
    });
    gen_raw_all::<LEAN, _, _, _, JUMPS, ANY_SIDE>(
        mg,
        snapshot,
        rack,
        made,
        movegen::PassPolicy::OnlyWhenForced,
    )?;
    moves.extend(
        (mg.plays.iter())
            .filter(|vm| matches!(vm.play, movegen::Play::Place { .. }))
            .map(|vm| vm.play.clone()),
    );
    each_exchange(gc, rack, made, bag_len, scratch, |tiles| {
        moves.push(movegen::Play::Exchange {
            tiles: tiles.into(),
        })
    });
    Ok(())
}

// the exchanges moves_of lists after the placements, each as its tiles: by how
// many tiles, then by tile; worked in scratch, the rack's tally, an exchange's
// tally, then room for its tiles, which each is written into
fn each_exchange(
    gc: &game_config::GameConfig,
    rack: &[u8],
    made: i16,
    bag_len: usize,
    scratch: &mut Vec<u8>,
    mut found: impl FnMut(&[u8]),
) {
    if !exchange_legal(gc, bag_len, made) {
        return;
    }
    let tiles_at = gc.alphabet().len() as usize;
    scratch.clear();
    scratch.resize(2 * tiles_at + rack.len(), 0);
    let (tally, rest) = scratch.split_at_mut(tiles_at);
    let (drawn, thrown) = rest.split_at_mut(tiles_at);
    for &tile in rack {
        tally[tile as usize] += 1;
    }
    for n in 1..=rack.len().min(bag_len) {
        let mut more = first_draw(tally, n, drawn);
        while more {
            let mut len = 0;
            for (tile, &count) in (0u8..).zip(drawn.iter()) {
                thrown[len..len + count as usize].fill(tile);
                len += count as usize;
            }
            found(&thrown[..len]);
            more = next_draw(tally, drawn);
        }
    }
}

fn play_word_len(play: &movegen::Play) -> usize {
    match play {
        movegen::Play::Exchange { tiles } => tiles.len(),
        movegen::Play::Place { word, .. } => word.len(),
    }
}

// in_bag_moves's moves at a search's root, each growth of them checked against
// the memory limit, where there is one, before it is taken: the placements
// found, every one moved (not copied) into a list made at its full length, and
// the generator's list let go. Refused (the search stopped) where one would
// pass the limit, nothing kept
fn root_moves<
    const LEAN: bool,
    N: kwg::Node,
    L: kwg::Node,
    G: movegen::LaneBits,
    const JUMPS: bool,
    const ANY_SIDE: bool,
>(
    gc: &game_config::GameConfig,
    kwg: &kwg::Kwg<N>,
    arm_tables: &anagrams::ArmTables,
    klv: &klv::Klv<L>,
    mg: &mut movegen::KurniaMoveGenerator<G, JUMPS, ANY_SIDE>,
    pos: &InBag,
    limits: &BagLimits,
) -> error::Returns<Vec<movegen::Play>> {
    let Some(memory) = limits.memory else {
        let mut lists = PlyLists::default();
        in_bag_moves::<LEAN, _, _, _, JUMPS, ANY_SIDE>(
            gc,
            kwg,
            arm_tables,
            klv,
            mg,
            pos,
            (&mut lists, limits),
        )?;
        return Ok(lists.moves);
    };
    let mover = pos.mover as usize;
    let (rack, made) = (&pos.racks[mover], pos.exchanges[mover]);
    let cut = mg.gen_moves_raw_all_unsorted_held(
        &movegen::BoardSnapshot {
            board_tiles: &pos.board,
            game_config: gc,
            kwg,
            anagrams: arm_tables.anagrams(),
            rack_lengths: arm_tables.rack_lengths(),
            klv,
        },
        rack,
        made,
        movegen::PassPolicy::OnlyWhenForced,
        movegen::HeldBytes { memory, kept: 0 },
    )?;
    let mut plays = std::mem::take(&mut mg.plays);
    if cut {
        return Err(limits.stop_refused());
    }
    let places = (plays.iter())
        .filter(|vm| matches!(vm.play, movegen::Play::Place { .. }))
        .count();
    // the scratch each_exchange works in, then the pass, the placements, and
    // the exchanges, then each exchange's tiles
    let mut scratch = list_in_room::<u8>(limits, 2 * gc.alphabet().len() as usize + rack.len())?;
    let mut exchanges = 0;
    each_exchange(gc, rack, made, pos.bag_len(), &mut scratch, |_| {
        exchanges += 1
    });
    let mut moves = list_in_room::<movegen::Play>(limits, 1 + places + exchanges)?;
    let _tiles = limits.reserve((exchanges + 1) * rack.len())?;
    moves.push(movegen::Play::Exchange {
        tiles: [][..].into(),
    });
    moves.extend(
        (plays.drain(..))
            .filter(|vm| matches!(vm.play, movegen::Play::Place { .. }))
            .map(|vm| vm.play),
    );
    drop(plays);
    each_exchange(gc, rack, made, pos.bag_len(), &mut scratch, |tiles| {
        moves.push(movegen::Play::Exchange {
            tiles: tiles.into(),
        })
    });
    Ok(moves)
}

#[inline(always)]
pub(crate) fn play_bytes(play: &movegen::Play) -> usize {
    bites::Bites::heap_bytes(match play {
        movegen::Play::Place { word, .. } => word.len(),
        movegen::Play::Exchange { tiles } => tiles.len(),
    })
}

fn growth<T>(v: &Vec<T>, more: usize) -> usize {
    if v.len() + more <= v.capacity() {
        0
    } else {
        ((2 * v.capacity()).max(v.len() + more) + v.capacity()) * std::mem::size_of::<T>()
    }
}

// an empty list with room for n, under the memory limit: refused (the search
// stopped) where it would take more than the room left
fn list_in_room<T>(limits: &BagLimits, n: usize) -> error::Returns<Vec<T>> {
    let _list = limits.reserve(n.saturating_mul(std::mem::size_of::<T>()))?;
    let mut list = Vec::new();
    list.try_reserve_exact(n)
        .map_err(|_| limits.stop_refused())?;
    Ok(list)
}

// whether a player who has exchanged this many times may exchange with this
// many tiles in the bag
#[inline(always)]
fn exchange_legal(gc: &game_config::GameConfig, bag_len: usize, made: i16) -> bool {
    bag_len as i64 >= i64::from(gc.exchange_tile_limit())
        && made < gc.exchanges_allowed_per_player()
}

// whether one player's exchanges can go on with no count of scoreless turns
// ending the game, so that a search could never end: where the bag lets them
// exchange and exchanges are not zeros the config ends on
#[inline(always)]
fn exchanges_without_forced_end(gc: &game_config::GameConfig, bag_len: usize) -> bool {
    exchange_legal(gc, bag_len, 0) && !(gc.exchanges_are_zeros() && gc.num_zeros_to_end() != 0)
}

// the chance a final spread of v beats t, a tie counted half
#[inline(always)]
fn beats(v: f64, t: f64) -> f64 {
    if v > t {
        1.0
    } else if v == t {
        0.5
    } else {
        0.0
    }
}

// the first draw of n tiles a pool (a count per tile) allows, in the order
// each_split gives them (each tile's count rising, the first tile's slowest),
// written into drawn; false if the pool holds fewer than n
fn first_draw(pool: &[u8], n: usize, drawn: &mut [u8]) -> bool {
    let mut left = n;
    for (d, &count) in drawn.iter_mut().zip(pool).rev() {
        let take = left.min(count as usize);
        *d = take as u8;
        left -= take;
    }
    left == 0
}

// the draw after drawn in first_draw's order, written over it; false after the
// last
fn next_draw(pool: &[u8], drawn: &mut [u8]) -> bool {
    let mut after = 0usize; // the tiles drawn past t
    for t in (0..drawn.len()).rev() {
        if after > 0 && drawn[t] < pool[t] {
            drawn[t] += 1;
            first_draw(&pool[t + 1..], after - 1, &mut drawn[t + 1..]);
            return true;
        }
        after += drawn[t] as usize;
    }
    false
}

// how many deals give a draw from a pool, as each_split counts them
#[inline(always)]
fn draw_ways(pool: &[u8], drawn: &[u8]) -> u64 {
    (pool.iter().zip(drawn)).fold(1u64, |ways, (&p, &d)| ways * binom(p as usize, d as usize))
}

// the room the search with tiles in the bag works in, made once for a search
// and kept across its root moves, a ply's room for each ply (PlyRoom). Sized
// at setup for as many plies as the bag has tiles and one more, each placement
// drawing at least one; a line of scoreless turns past that adds a ply's room
// once, kept from then on
struct BagArena {
    tiles: usize,
    plies: Vec<PlyRoom>,
    root: InBag,        // a root split's position
    root_info: InfoSet, // what the mover knows there
}

// a ply's room: a count per tile for each of the bag a draw is made from, the
// draw, and one more tally (the tiles an exchange throws back, or the whole
// bag a placement draws), each lent out on its own; the moves of the position
// valued there with the order they are asked in, lent out together; and a
// split's position (InfoSet::at) and what the other then knows
// (InfoSet::theirs), each lent out on its own
#[derive(Default)]
struct PlyRoom {
    tallies: [Vec<u8>; 3],
    lists: PlyLists,
    pos: InBag,
    next: InfoSet,
}

// the lists of a position valued at a ply, lent out together: its moves, the
// order they are asked in, moves_of's scratch, an info set's splits (the first
// num_splits of them), and the moves info_best finds at the best chance with
// each one's chance, their margins a split each in turn
#[derive(Default)]
struct PlyLists {
    moves: Vec<movegen::Play>,
    asked: Vec<usize>,
    scratch: Vec<u8>,
    splits: Vec<(Vec<u8>, Vec<u8>, u64)>,
    num_splits: usize,
    valued: Vec<(usize, f64)>,
    margins: Vec<Option<f64>>,
}

// which of a ply's tallies (BagArena)
const DRAW_POOL: usize = 0;
const DRAW: usize = 1;
const TALLY: usize = 2;

impl BagArena {
    fn at_least(
        arenas: &mut Vec<Self>,
        n: usize,
        tiles: usize,
        bag_len: usize,
        limits: &BagLimits,
    ) -> error::Returns<()> {
        if arenas.len() < n {
            let made = n - arenas.len();
            limits.room(arenas, made)?;
            let room = std::mem::size_of::<PlyRoom>() + 3 * tiles;
            let _plies = limits.reserve(made.saturating_mul(bag_len + 1).saturating_mul(room))?;
            arenas.resize_with(n, || Self::new(tiles, bag_len));
        }
        Ok(())
    }

    fn new(tiles: usize, bag_len: usize) -> Self {
        Self {
            tiles,
            plies: (0..=bag_len).map(|_| Self::room(tiles)).collect(),
            root: Default::default(),
            root_info: Default::default(),
        }
    }

    #[inline(always)]
    fn room(tiles: usize) -> PlyRoom {
        PlyRoom {
            tallies: [vec![0u8; tiles], vec![0u8; tiles], vec![0u8; tiles]],
            ..Default::default()
        }
    }

    // a ply's room, made the first time the ply is reached
    #[inline(always)]
    fn ply(&mut self, ply: usize) -> &mut PlyRoom {
        if ply >= self.plies.len() {
            let tiles = self.tiles;
            self.plies.resize_with(ply + 1, || Self::room(tiles));
        }
        &mut self.plies[ply]
    }

    // one of a ply's tallies, lent out until given back (one an error kept is
    // made again)
    #[inline(always)]
    fn take(&mut self, ply: usize, which: usize) -> Vec<u8> {
        let tiles = self.tiles;
        let mut tally = std::mem::take(&mut self.ply(ply).tallies[which]);
        tally.resize(tiles, 0);
        tally
    }

    #[inline(always)]
    fn give(&mut self, ply: usize, which: usize, tally: Vec<u8>) {
        self.plies[ply].tallies[which] = tally;
    }

    // a ply's lists, lent out until given back
    #[inline(always)]
    fn take_lists(&mut self, ply: usize) -> PlyLists {
        std::mem::take(&mut self.ply(ply).lists)
    }

    // a ply's split position, lent out until given back
    #[inline(always)]
    fn take_pos(&mut self, ply: usize) -> InBag {
        std::mem::take(&mut self.ply(ply).pos)
    }

    #[inline(always)]
    fn give_pos(&mut self, ply: usize, pos: InBag) {
        self.plies[ply].pos = pos;
    }

    // a ply's info set for the other, lent out until given back
    #[inline(always)]
    fn take_next(&mut self, ply: usize) -> InfoSet {
        std::mem::take(&mut self.ply(ply).next)
    }

    #[inline(always)]
    fn give_next(&mut self, ply: usize, next: InfoSet) {
        self.plies[ply].next = next;
    }

    #[inline(always)]
    fn give_lists(&mut self, ply: usize, lists: PlyLists) {
        self.plies[ply].lists = lists;
    }
}

// what the search with tiles in the bag has valued, one for all its threads:
// shards by key, each behind a lock of its own (BagShard), each shared with a
// copy until written (copy on write); and, read through where its own shard
// has no entry for a key, the shards of another it was laid over (under, none
// unless over made it)
#[derive(Default)]
struct BagValued {
    shards: Box<[std::sync::Mutex<std::sync::Arc<BagShard>>]>,
    under: Box<[std::sync::Arc<BagShard>]>,
}

// one shard of what the search has valued, by key, each kind in a table of its
// own: each position's value to its mover (BagEntry); each endgame's value to
// its mover, and whether exact, or none where a shallower pass found it not
// exact; and as much as a position's for what a player knows (InfoSet), whose
// value is their chance to win with their mean margin
#[derive(Clone, Default)]
struct BagShard {
    nodes: BagTable<BagEntry<f64>>,
    endgames: BagTable<Option<(f64, bool)>>,
    infos: BagTable<BagEntry<(f64, f64)>>,
}

// what the search with tiles in the bag keeps for a position, or for what a
// player knows: its value, the move that gets it, and whether no endgame a
// depth cut short went into it; or, from a shallower pass, the move found best
// there alone, to be tried first
#[derive(Clone)]
enum BagEntry<V> {
    Valued(V, movegen::Play, bool),
    First(movegen::Play),
}

impl<V> BagEntry<V> {
    // what a full table gives up first: a move to try first, then a value not
    // exact, then an exact one
    #[inline(always)]
    fn worth(&self) -> u8 {
        match self {
            BagEntry::First(_) => 0,
            BagEntry::Valued(_, _, exact) => 1 + u8::from(*exact),
        }
    }

    // for a deeper pass: an exact value as it is, any other's move to be tried
    // first
    #[inline(always)]
    fn deepened(self) -> Self {
        match self {
            BagEntry::Valued(_, play, false) => BagEntry::First(play),
            entry => entry,
        }
    }
}

impl<V: Clone> BagEntry<V> {
    // as deepened, from an entry another table still shares
    #[inline(always)]
    fn deepened_from(&self) -> Self {
        match self {
            BagEntry::Valued(_, play, false) => BagEntry::First(play.clone()),
            entry => entry.clone(),
        }
    }
}

// what a search with tiles in the bag keeps of one kind, by key: in one row of
// entries, a key's first place by multiply-shift, its later ones each the next
// in turn, the first empty entry ending them (linear probing). Before three
// quarters of the entries would hold, it grows at once to twice as many, while
// the program stays within its memory limit with the old ones still held, so
// nothing is lost while it can grow. Refused, it keeps its size: a key not in
// it takes the place of the first entry worth least on its way, and is not kept
// when its first place is empty
#[derive(Clone)]
struct BagTable<T> {
    entries: Vec<Option<(u64, T)>>,
    held: usize, // the entries holding a key
}

impl<T> Default for BagTable<T> {
    // four entries to start with
    fn default() -> Self {
        Self {
            entries: (0..4).map(|_| None).collect(),
            held: 0,
        }
    }
}

impl<T> BagTable<T> {
    // the table as default makes it, four entries empty, in place
    #[inline(always)]
    fn renew(&mut self) {
        self.entries.truncate(4);
        self.entries.iter_mut().for_each(|entry| *entry = None);
        self.held = 0;
    }

    #[inline(always)]
    fn bytes(&self) -> usize {
        self.entries.len() * std::mem::size_of::<Option<(u64, T)>>()
    }

    // a key's first place
    #[inline(always)]
    fn first(&self, key: u64) -> usize {
        ((u128::from(key) * self.entries.len() as u128) >> 64) as usize
    }

    // the place after at, the first after the last
    #[inline(always)]
    fn after(&self, at: usize) -> usize {
        if at + 1 == self.entries.len() {
            0
        } else {
            at + 1
        }
    }

    // what the table keeps for a key
    #[inline(always)]
    fn get(&self, key: u64) -> Option<&T> {
        let mut at = self.first(key);
        loop {
            match &self.entries[at] {
                Some((kept, value)) if *kept == key => return Some(value),
                Some(_) => at = self.after(at),
                None => return None,
            }
        }
    }

    // keep a value for a key, the table grown first, as the memory limit
    // allows, once three quarters of it would hold; worth ranks what a full
    // table gives up
    #[inline(always)]
    fn keep(&mut self, key: u64, value: T, memory: Option<MemoryLimit>, worth: impl Fn(&T) -> u8) {
        if 4 * (self.held + 1) > 3 * self.entries.len() {
            self.grow(memory);
        }
        let full = 4 * (self.held + 1) > 3 * self.entries.len();
        let mut at = self.first(key);
        let mut least = None;
        loop {
            match &self.entries[at] {
                Some((kept, _)) if *kept == key => break,
                Some((_, kept)) => {
                    let kept = worth(kept);
                    if full && least.is_none_or(|(so_far, _)| kept < so_far) {
                        least = Some((kept, at));
                    }
                    at = self.after(at);
                }
                None if !full => {
                    self.held += 1;
                    break;
                }
                None => match least {
                    Some((_, place)) => {
                        at = place;
                        break;
                    }
                    None => return,
                },
            }
        }
        self.entries[at] = Some((key, value));
    }

    // the table grown to twice its entries, each kept put in the first empty
    // one of its places; as it was when the memory limit or the allocator
    // refuses the room
    #[inline(never)]
    fn grow(&mut self, memory: Option<MemoryLimit>) {
        let len = 2 * self.entries.len();
        let bytes = len * std::mem::size_of::<Option<(u64, T)>>();
        let mut entries = Vec::new();
        let room = reserve_in(memory.as_ref(), bytes);
        if room.is_none() || entries.try_reserve_exact(len).is_err() {
            return;
        }
        drop(room);
        entries.resize_with(len, || None);
        for (key, value) in std::mem::replace(&mut self.entries, entries)
            .into_iter()
            .flatten()
        {
            let mut at = self.first(key);
            while self.entries[at].is_some() {
                at = self.after(at);
            }
            self.entries[at] = Some((key, value));
        }
    }

    // each value kept passed through f
    fn each_kept(&mut self, mut f: impl FnMut(T) -> T) {
        for entry in self.entries.iter_mut() {
            if let Some((key, value)) = entry.take() {
                *entry = Some((key, f(value)));
            }
        }
    }

    // a table each value of which is one kept here passed through f, each in
    // the place it has here
    fn kept_through(&self, f: impl Fn(&T) -> T) -> Self {
        Self {
            entries: (self.entries.iter())
                .map(|entry| entry.as_ref().map(|(key, value)| (*key, f(value))))
                .collect(),
            held: self.held,
        }
    }
}

impl BagValued {
    fn shared(&self, memory: Option<MemoryLimit>) -> Option<(Self, Reserved)> {
        let shards = (self.shards.iter())
            .map(|shard| shard.lock().unwrap().clone())
            .collect::<Box<[_]>>();
        let bytes = (shards.iter())
            .map(|shard| shard.nodes.bytes() + shard.endgames.bytes() + shard.infos.bytes())
            .sum::<usize>();
        let copies = reserve_in(memory.as_ref(), bytes)?;
        let shared = Self {
            shards: shards.into_iter().map(std::sync::Mutex::new).collect(),
            under: Default::default(),
        };
        Some((shared, copies))
    }

    // nothing valued yet, in as many shards as threads search
    fn new(threads: usize) -> Self {
        Self {
            shards: (0..threads.max(1)).map(|_| Default::default()).collect(),
            under: Default::default(),
        }
    }

    // nothing valued yet, laid over what under has valued: read through to it,
    // written apart from it, which it shares but never sees
    fn over(under: &Self) -> Self {
        Self {
            shards: (0..under.shards.len())
                .map(|_| Default::default())
                .collect(),
            under: (under.shards.iter())
                .map(|shard| shard.lock().unwrap().clone())
                .collect(),
        }
    }

    // what read finds for key in the shard that holds it, else in the one
    // under it
    #[inline(always)]
    fn read<R>(&self, key: u64, read: impl Fn(&BagShard) -> Option<R>) -> Option<R> {
        let at = key as usize % self.shards.len();
        let found = read(&self.shards[at].lock().unwrap());
        found.or_else(|| self.under.get(at).and_then(|shard| read(shard)))
    }

    // write to the shard that holds key, locked, copied first where a copy
    // still shares it
    #[inline(always)]
    fn write<R>(&self, key: u64, write: impl FnOnce(&mut BagShard) -> R) -> R {
        let mut shard = self.shards[key as usize % self.shards.len()]
            .lock()
            .unwrap();
        write(std::sync::Arc::make_mut(&mut shard))
    }

    // for a deeper pass: the exact values kept, and the move found best at each
    // other position, to be tried first there; a shard a copy shares made
    // afresh in one walk, the copy keeping its own
    fn deepen(&mut self) {
        for shard in self.shards.iter_mut() {
            let shard = shard.get_mut().unwrap();
            let endgame = |endgame: &Option<(f64, bool)>| endgame.filter(|&(_, exact)| exact);
            match std::sync::Arc::get_mut(shard) {
                Some(shard) => {
                    shard.nodes.each_kept(BagEntry::deepened);
                    shard.infos.each_kept(BagEntry::deepened);
                    (shard.endgames).each_kept(|kept| endgame(&kept));
                }
                None => {
                    *shard = std::sync::Arc::new(BagShard {
                        nodes: shard.nodes.kept_through(BagEntry::deepened_from),
                        endgames: shard.endgames.kept_through(endgame),
                        infos: shard.infos.kept_through(BagEntry::deepened_from),
                    });
                }
            }
        }
    }
}

// what a root task that hands the turn over (a pass or an exchange) opens to
// the threads with no task left: the info set it hands the other, its moves in
// the order it values them, and its splits (work, put once, then open); the
// place in that order it has reached (forward), one past the next a helper
// takes from the end (back), and its floor as f64 bits (NO_FLOOR for none). A
// helper values moves from the end into the shared table, so the task finds
// them there; the task still values every move itself, so what it finds is what
// it would find alone
struct HandOver {
    work: std::sync::Mutex<Option<std::sync::Arc<HandOverWork>>>,
    open: std::sync::atomic::AtomicBool,
    forward: std::sync::atomic::AtomicUsize,
    back: std::sync::atomic::AtomicUsize,
    floor: std::sync::atomic::AtomicU64,
}

type HandOverWork = (InfoSet, Vec<movegen::Play>, Vec<(Vec<u8>, Vec<u8>, u64)>);

const NO_FLOOR: u64 = u64::MAX;

impl HandOver {
    fn new() -> Self {
        Self {
            work: std::sync::Mutex::new(None),
            open: Default::default(),
            forward: Default::default(),
            back: Default::default(),
            floor: std::sync::atomic::AtomicU64::new(NO_FLOOR),
        }
    }

    // work opened to helpers, the first time only
    fn put(&self, work: HandOverWork) {
        use std::sync::atomic::Ordering::{Relaxed, Release};
        let mut kept = self.work.lock().unwrap();
        if kept.is_none() {
            self.back.store(work.1.len(), Relaxed);
            *kept = Some(std::sync::Arc::new(work));
            self.open.store(true, Release);
        }
    }

    // the work, once opened
    fn opened(&self) -> Option<std::sync::Arc<HandOverWork>> {
        if !self.open.load(std::sync::atomic::Ordering::Acquire) {
            return None;
        }
        self.work.lock().unwrap().clone()
    }

    // the next move from the end for a helper, while it is past the task's
    // place
    fn take_back(&self) -> Option<usize> {
        use std::sync::atomic::Ordering::Relaxed;
        let forward = self.forward.load(Relaxed);
        (self.back)
            .try_update(Relaxed, Relaxed, |back| {
                (back > forward + 1).then(|| back - 1)
            })
            .ok()
            .map(|back| back - 1)
    }
}

// the search with tiles in the bag, under either model (PegModel): under
// InfoSets each player to move chooses blind (info_value); under RandomDraws,
// the fast approximation, both players know both racks; a draw is at random, so
// a move's value is what it scores less the next mover's value over what the
// draw may bring, each draw weighed by the deals that give it. Once a placement
// empties the bag, sub solves the endgame. What it carries down, with the root
// tasks' hand-overs on threads (by task, none off threads) and the one the
// first info set it values opens
struct BagSearch<
    's,
    'a,
    N: kwg::Node,
    L: kwg::Node,
    const LEAN: bool,
    G: movegen::LaneBits,
    const JUMPS: bool,
    const ANY_SIDE: bool,
> {
    gc: &'s game_config::GameConfig,
    kwg: &'s kwg::Kwg<N>,
    arm_tables: &'s anagrams::ArmTables,
    klv: &'s klv::Klv<L>,
    sub: &'s mut EndgameSolver<'a, N, L, LEAN, G, JUMPS, ANY_SIDE>,
    mg: &'s mut movegen::KurniaMoveGenerator<G, JUMPS, ANY_SIDE>,
    valued: &'s BagValued,
    limits: &'s BagLimits,
    model: PegModel,
    exact: bool, // cleared by an endgame a limit cut short
    hand_overs: &'s [HandOver],
    hand_over: Option<usize>,
    ply: usize, // the moves made from the root's position to the one valued
    arena: &'s mut BagArena,
}

impl<
    N: kwg::Node,
    L: kwg::Node,
    const LEAN: bool,
    G: movegen::LaneBits,
    const JUMPS: bool,
    const ANY_SIDE: bool,
> BagSearch<'_, '_, N, L, LEAN, G, JUMPS, ANY_SIDE>
{
    // whether a limit has passed, read at each position the search values once
    // it may stop: a line of scoreless turns, as exchanges make with seven or
    // more in the bag, starts no endgame, the one other place a limit is read
    // (solve_in_bag_below), so the search would run on past its time there.
    // An error, with stopped set
    #[inline(always)]
    fn stop_at_a_limit(&self) -> error::Returns<()> {
        if self.limits.may_stop.get() && self.limits.passed(&self.sub.work_buffer.table) {
            self.limits.stopped.set(true);
            return Err(Box::new(LimitStopped));
        }
        Ok(())
    }

    // whether a move made at the position valued, its line leaving tiles in the
    // bag, is valued by its equity (estimate): the passes estimating, one at
    // its depth in the plies before the bag empties, the move the last of them
    // it follows
    #[inline(always)]
    fn at_horizon(&self) -> bool {
        let depth = self.limits.depth.get();
        self.limits.estimates && depth != COMPLETE && self.ply + 1 >= depth as usize
    }

    // a move's equity to its mover, the line cut short there: what it scores
    // and the value of the tiles it keeps (the rack less those thrown back) by
    // the search's leaves; not exact
    fn estimate(&mut self, score: f64, rack: &[u8], thrown: &[u8]) -> f64 {
        self.exact = false;
        // the kept tiles' tally in this ply's room (BagArena)
        let mut kept = self.arena.take(self.ply, TALLY);
        kept.fill(0);
        for &tile in rack {
            kept[tile as usize] += 1;
        }
        for &tile in thrown {
            kept[tile as usize] -= 1;
        }
        let value = score + f64::from(self.klv.leave_value_from_tally(&kept).raw());
        self.arena.give(self.ply, TALLY, kept);
        value
    }

    // f's value at a position a move after the one valued
    #[inline(always)]
    fn deeper<T>(&mut self, f: impl FnOnce(&mut Self) -> error::Returns<T>) -> error::Returns<T> {
        self.ply += 1;
        let value = f(self);
        self.ply -= 1;
        value
    }

    // the tiles an exchange throws back, a count per tile, in this ply's room
    // (BagArena), to be given back
    #[inline(always)]
    fn thrown(&mut self, tiles: &[u8]) -> Vec<u8> {
        let mut put = self.arena.take(self.ply, TALLY);
        put.fill(0);
        for &tile in tiles {
            put[tile as usize] += 1;
        }
        put
    }

    // each draw of n tiles from pos's bag with its deals, to f with pos, until
    // f says to stop (false), walked in this ply's room (BagArena); f leaves
    // pos as it found it
    fn each_draw(
        &mut self,
        pos: &mut InBag,
        n: usize,
        mut f: impl FnMut(&mut Self, &mut InBag, &[u8], u64) -> error::Returns<bool>,
    ) -> error::Returns<()> {
        let ply = self.ply;
        let mut pool = self.arena.take(ply, DRAW_POOL);
        pool.copy_from_slice(&pos.bag);
        let mut drawn = self.arena.take(ply, DRAW);
        let mut walked = Ok(());
        let mut more = first_draw(&pool, n, &mut drawn);
        while more {
            more = match f(self, pos, &drawn, draw_ways(&pool, &drawn)) {
                Ok(true) => next_draw(&pool, &mut drawn),
                Ok(false) => false,
                Err(e) => {
                    walked = Err(e);
                    false
                }
            };
        }
        self.arena.give(ply, DRAW, drawn);
        self.arena.give(ply, DRAW_POOL, pool);
        walked
    }

    // f's value at pos after its mover places play, draws the whole bag, and
    // passes the turn on; pos then as it was
    fn after_emptying<T>(
        &mut self,
        pos: &mut InBag,
        play: &movegen::Play,
        f: impl FnOnce(&mut Self, &InBag) -> error::Returns<T>,
    ) -> error::Returns<T> {
        let turn = pos.place(self.gc, play);
        let mut all = self.arena.take(self.ply, TALLY);
        all.copy_from_slice(&pos.bag);
        pos.draw(&all);
        pos.next_turn();
        let value = f(self, pos);
        pos.next_turn();
        pos.undraw(&all);
        self.arena.give(self.ply, TALLY, all);
        pos.unplace(self.gc, play, turn);
        value
    }

    // a position's value to its mover: the most any move gets them, a tie going
    // to the move first in in_bag_moves's order. The move a shallower pass
    // found best goes first, then the placements, highest score first; once one
    // has a value, a placement that empties the bag is only asked whether its
    // endgame is worth little enough for it to be the best, and is valued only
    // if so (endgame_below); a pass or another placement is valued only if a
    // bound leaves it room to be the best (cannot_beat)
    fn node(&mut self, pos: &mut InBag) -> error::Returns<f64> {
        // read even where the position has its value: a node may visit a
        // great many valued ones, each exchange times each draw, between two
        // it searches
        self.stop_at_a_limit()?;
        let key = pos.key();
        let found = self.valued.read(key, |shard| {
            shard.nodes.get(key).map(|entry| match entry {
                &BagEntry::Valued(value, _, exact) => Ok((value, exact)),
                BagEntry::First(play) => Err(play.clone()),
            })
        });
        let first = match found {
            Some(Ok((value, exact))) => {
                self.exact &= exact;
                return Ok(value);
            }
            Some(Err(play)) => Some(play),
            None => None,
        };
        Ok(self.node_searched(pos, key, first)?.0)
    }

    // the move node finds best at a position: the one kept with its value, or,
    // when another entry has taken its place, searched for again
    fn node_play(&mut self, pos: &mut InBag) -> error::Returns<movegen::Play> {
        let key = pos.key();
        let found = self.valued.read(key, |shard| {
            shard.nodes.get(key).map(|entry| match entry {
                BagEntry::Valued(_, play, exact) => (play.clone(), Some(*exact)),
                BagEntry::First(play) => (play.clone(), None),
            })
        });
        let first = match found {
            Some((play, Some(exact))) => {
                self.exact &= exact;
                return Ok(play);
            }
            Some((play, None)) => Some(play),
            None => None,
        };
        Ok(self.node_searched(pos, key, first)?.1)
    }

    // a position's value to its mover and the move that gets it, searched as
    // node describes, first the move a shallower pass found best, and kept
    fn node_searched(
        &mut self,
        pos: &mut InBag,
        key: u64,
        first: Option<movegen::Play>,
    ) -> error::Returns<(f64, movegen::Play)> {
        self.stop_at_a_limit()?;
        let was_exact = std::mem::replace(&mut self.exact, true);
        // the moves, and the order they are asked in, in the ply's room
        let mut lists = self.arena.take_lists(self.ply);
        in_bag_moves::<LEAN, _, _, _, JUMPS, ANY_SIDE>(
            self.gc,
            self.kwg,
            self.arm_tables,
            self.klv,
            self.mg,
            pos,
            (&mut lists, self.limits),
        )?;
        let PlyLists { moves, asked, .. } = &mut lists;
        asked.clear();
        self.limits.room(asked, moves.len())?;
        asked.extend(0..moves.len());
        asked.sort_unstable_by_key(|&i| match &moves[i] {
            play if first.as_ref() == Some(play) => {
                (false, std::cmp::Reverse(equity::Equity::INFINITY), i)
            }
            movegen::Play::Place { score, .. } => (true, std::cmp::Reverse(*score), i),
            _ => (true, std::cmp::Reverse(equity::Equity::NEG_INFINITY), i),
        });
        let limits = self.limits;
        let found = (self.node_best(pos, moves, asked)).and_then(|(value, i)| {
            let _copies = limits.reserve(2 * play_bytes(&moves[i]))?;
            Ok((value, moves[i].clone(), moves[i].clone()))
        });
        self.arena.give_lists(self.ply, lists);
        let (value, play, kept) = found?;
        let exact = self.exact;
        self.exact = was_exact && exact;
        let entry = BagEntry::Valued(value, kept, exact);
        let memory = self.limits.memory;
        (self.valued).write(key, |shard| {
            shard.nodes.keep(key, entry, memory, BagEntry::worth)
        });
        Ok((value, play))
    }

    // the best of the moves at pos, asked in this order, as node_searched
    // finds it: its value and its place in moves
    fn node_best(
        &mut self,
        pos: &mut InBag,
        moves: &[movegen::Play],
        asked: &[usize],
    ) -> error::Returns<(f64, usize)> {
        let bag_len = pos.bag_len();
        let mut best = (f64::NEG_INFINITY, usize::MAX);
        for &i in asked {
            let value = match &moves[i] {
                movegen::Play::Place { word, score, .. }
                    if best.1 != usize::MAX
                        && word.iter().filter(|&&tile| tile != 0).count() >= bag_len =>
                {
                    // the best so far, or a tie with it coming first, needs the
                    // next mover's endgame worth score - best at most
                    let room = f64::from(score.raw()) - best.0;
                    let most = if i < best.1 {
                        room.floor()
                    } else {
                        room.ceil() - 1.0
                    };
                    (self.after_emptying(pos, &moves[i], |this, next| {
                        this.endgame_below(next, most + 1.0)
                    })?)
                    .map(|theirs| f64::from(score.raw()) - theirs)
                }
                play if best.1 != usize::MAX && self.cannot_beat(pos, play, best, i)? => None,
                play => Some(self.move_value(pos, play)?),
            };
            if let Some(value) = value
                && (value > best.0 || (value == best.0 && i < best.1))
            {
                best = (value, i);
            }
        }
        Ok(best)
    }

    // whether a pass or a placement that leaves tiles in the bag cannot be the
    // best at pos, without valuing it: it gets its mover at most what it scores
    // less the least the next mover gets after each draw (least), worth less
    // than the best so far, or no more with the best coming first. An exchange,
    // or a pass that ends the game, is never ruled out this way
    fn cannot_beat(
        &mut self,
        pos: &mut InBag,
        play: &movegen::Play,
        best: (f64, usize),
        i: usize,
    ) -> error::Returns<bool> {
        let most = match play {
            // a move valued by its equity is not ruled out by a bound below it
            _ if self.at_horizon() => return Ok(false),
            movegen::Play::Place { word, score, .. } => {
                let turn = pos.place(self.gc, play);
                let n = word.iter().filter(|&&tile| tile != 0).count();
                let (mut sum, mut total, mut bounded) = (0.0, 0u64, true);
                let walked = self.each_draw(pos, n.min(pos.bag_len()), |this, pos, drawn, ways| {
                    pos.draw(drawn);
                    pos.next_turn();
                    let least = this.least(pos);
                    pos.next_turn();
                    pos.undraw(drawn);
                    let Some(least) = least? else {
                        bounded = false;
                        return Ok(false);
                    };
                    sum += ways as f64 * least;
                    total += ways;
                    Ok(true)
                });
                pos.unplace(self.gc, play, turn);
                walked?;
                if !bounded {
                    return Ok(false);
                }
                f64::from(score.raw()) - sum / total as f64
            }
            movegen::Play::Exchange { tiles } if tiles.is_empty() => {
                if pos.a_pass_ends(self.gc) {
                    return Ok(false);
                }
                let turn = pos.pass();
                pos.next_turn();
                let least = self.least(pos);
                pos.next_turn();
                pos.set_turn(turn);
                let Some(least) = least? else {
                    return Ok(false);
                };
                -least
            }
            movegen::Play::Exchange { .. } => return Ok(false),
        };
        Ok(most < best.0 || (most == best.0 && i > best.1))
    }

    // at least what the mover at pos, with tiles in the bag, gets: its value
    // once known, else what one placement that empties the bag gets them (the
    // move a shallower pass found best there, if it is one, else the
    // highest-scoring); none without such a placement
    fn least(&mut self, pos: &mut InBag) -> error::Returns<Option<f64>> {
        let key = pos.key();
        let found = self.valued.read(key, |shard| {
            shard.nodes.get(key).map(|entry| match entry {
                &BagEntry::Valued(value, _, exact) => Ok((value, exact)),
                BagEntry::First(play) => Err(play.clone()),
            })
        });
        let first = match found {
            Some(Ok((value, exact))) => {
                self.exact &= exact;
                return Ok(Some(value));
            }
            Some(Err(play)) => Some(play),
            None => None,
        };
        let bag_len = pos.bag_len();
        let empties = |play: &movegen::Play| {
            matches!(play, movegen::Play::Place { word, .. }
                if word.iter().filter(|&&tile| tile != 0).count() >= bag_len)
        };
        let play = match first {
            Some(play) if empties(&play) => play,
            _ => {
                // in_bag_moves's placements, read from the generator
                let mover = pos.mover as usize;
                gen_raw_all::<LEAN, _, _, _, JUMPS, ANY_SIDE>(
                    self.mg,
                    &movegen::BoardSnapshot {
                        board_tiles: &pos.board,
                        game_config: self.gc,
                        kwg: self.kwg,
                        anagrams: self.arm_tables.anagrams(),
                        rack_lengths: self.arm_tables.rack_lengths(),
                        klv: self.klv,
                    },
                    &pos.racks[mover],
                    pos.exchanges[mover],
                    movegen::PassPolicy::OnlyWhenForced,
                )?;
                let plays = self.mg.plays.iter().map(|vm| &vm.play);
                let Some(play) =
                    (plays.filter(|play| empties(play))).max_by_key(|play| match play {
                        movegen::Play::Place { score, .. } => *score,
                        movegen::Play::Exchange { .. } => equity::Equity::NEG_INFINITY,
                    })
                else {
                    return Ok(None);
                };
                play.clone()
            }
        };
        let movegen::Play::Place { score, .. } = play else {
            unreachable!("a placement empties the bag");
        };
        let theirs = self.after_emptying(pos, &play, |this, next| this.endgame(next))?;
        Ok(Some(f64::from(score.raw()) - theirs))
    }

    // the value to its mover of a position whose bag is empty: its endgame
    fn endgame(&mut self, pos: &InBag) -> error::Returns<f64> {
        let key = pos.key();
        let found = (self.valued).read(key, |shard| shard.endgames.get(key).copied().flatten());
        if let Some((value, exact)) = found {
            self.exact &= exact;
            return Ok(value);
        }
        self.sub.init_after_scoreless_turns(
            &pos.board,
            [&pos.racks[0], &pos.racks[1]],
            u16::from(pos.passes),
            u16::from(pos.zeros),
        );
        let mut exact = true;
        let value = f64::from(
            self.sub
                .solve_in_bag(pos.mover, self.limits, &mut exact)?
                .raw(),
        );
        self.exact &= exact;
        self.keep_endgame(key, value, exact);
        Ok(value)
    }

    // an endgame's value kept, and whether exact: what a full table gives up
    // first is one a shallower pass found not exact, then one not exact
    #[inline(always)]
    fn keep_endgame(&self, key: u64, value: f64, exact: bool) {
        let memory = self.limits.memory;
        (self.valued).write(key, |shard| {
            (shard.endgames).keep(key, Some((value, exact)), memory, |endgame| {
                endgame.map_or(0, |(_, exact)| 1 + u8::from(exact))
            })
        });
    }

    // the endgame's value to its mover when it is below beta (a whole number of
    // millipoints, as every endgame value is), else none: searched below beta
    // alone, which stops sooner than a search for the value
    fn endgame_below(&mut self, pos: &InBag, beta: f64) -> error::Returns<Option<f64>> {
        let key = pos.key();
        let found = (self.valued).read(key, |shard| shard.endgames.get(key).copied().flatten());
        if let Some((value, exact)) = found {
            self.exact &= exact;
            return Ok((value < beta).then_some(value));
        }
        self.sub.init_after_scoreless_turns(
            &pos.board,
            [&pos.racks[0], &pos.racks[1]],
            u16::from(pos.passes),
            u16::from(pos.zeros),
        );
        let mut exact = true;
        let value = f64::from(
            (self.sub)
                .solve_in_bag_below(pos.mover, beta_equity(beta)?, self.limits, &mut exact)?
                .raw(),
        );
        self.exact &= exact;
        if value < beta {
            self.keep_endgame(key, value, exact);
            Ok(Some(value))
        } else {
            Ok(None)
        }
    }

    // the value to the next mover of the position after this one's turn, the
    // bag empty or not
    #[inline(always)]
    fn next_value(&mut self, next: &mut InBag) -> error::Returns<f64> {
        if next.bag_len() == 0 {
            self.endgame(next)
        } else {
            self.node(next)
        }
    }

    // each draw of n tiles from pos's bag with its deals, to f, at the root
    // alone: the root's draws are told (move_value_each), and a move valued by
    // its equity below it makes none
    fn each_draw_at_root(&mut self, pos: &mut InBag, n: usize, mut f: impl FnMut(&[u8], u64)) {
        if self.ply == 0 {
            // f makes no error
            let _ = self.each_draw(pos, n, |_, _, drawn, ways| {
                f(drawn, ways);
                Ok(true)
            });
        }
    }

    // a move's value to its mover
    #[inline(always)]
    fn move_value(&mut self, pos: &mut InBag, play: &movegen::Play) -> error::Returns<f64> {
        self.move_value_each(pos, play, |_, _, _| {})
    }

    // the same, each draw the move makes told with its deals and its value
    // (drawn as a tally)
    fn move_value_each(
        &mut self,
        pos: &mut InBag,
        play: &movegen::Play,
        mut each: impl FnMut(&[u8], u64, f64),
    ) -> error::Returns<f64> {
        match play {
            movegen::Play::Place { word, score, .. } => {
                // what it scores, less the next mover's value over the draws
                // that refill the rack
                let n = word.iter().filter(|&&tile| tile != 0).count();
                if n < pos.bag_len() && self.at_horizon() {
                    let turn = pos.place(self.gc, play);
                    let value =
                        self.estimate(f64::from(score.raw()), &pos.racks[pos.mover as usize], &[]);
                    pos.unplace(self.gc, play, turn);
                    self.each_draw_at_root(pos, n, |drawn, ways| each(drawn, ways, value));
                    return Ok(value);
                }
                let turn = pos.place(self.gc, play);
                let (mut sum, mut total) = (0.0, 0u64);
                let walked = self.each_draw(pos, n.min(pos.bag_len()), |this, pos, drawn, ways| {
                    pos.draw(drawn);
                    pos.next_turn();
                    let theirs = this.deeper(|this| this.next_value(pos));
                    pos.next_turn();
                    pos.undraw(drawn);
                    let theirs = theirs?;
                    each(drawn, ways, f64::from(score.raw()) - theirs);
                    sum += ways as f64 * theirs;
                    total += ways;
                    Ok(true)
                });
                pos.unplace(self.gc, play, turn);
                walked?;
                Ok(f64::from(score.raw()) - sum / total as f64)
            }
            movegen::Play::Exchange { tiles } if tiles.is_empty() => {
                // the game's end where the pass answers a pass the line made or
                // brings the scoreless turns to the config's count, else the
                // next mover's value. A pass answering a pass ends the line as
                // an empty bag's endgame ends it: the position is as it was two
                // plies back
                if pos.a_pass_ends(self.gc) {
                    return Ok(pos.racks_lost(self.gc));
                }
                if self.at_horizon() {
                    return Ok(self.estimate(0.0, &pos.racks[pos.mover as usize], &[]));
                }
                let turn = pos.pass();
                pos.next_turn();
                let theirs = self.deeper(|this| this.node(pos));
                pos.next_turn();
                pos.set_turn(turn);
                Ok(-theirs?)
            }
            movegen::Play::Exchange { tiles } => {
                // over the draws of as many tiles from the bag, made before the
                // tiles go back as GameState::play makes them: the game's end
                // where the turn ends it, else the next mover's value
                let zeros = if self.gc.exchanges_are_zeros() {
                    pos.zeros.saturating_add(1)
                } else {
                    0
                };
                let ends = scoreless_turns_end(self.gc, Some(&pos.board), 0, zeros);
                if !ends && self.at_horizon() {
                    let value = self.estimate(0.0, &pos.racks[pos.mover as usize], tiles);
                    self.each_draw_at_root(pos, tiles.len(), |drawn, ways| {
                        each(drawn, ways, value)
                    });
                    return Ok(value);
                }
                let put = self.thrown(tiles);
                let (mut sum, mut total) = (0.0, 0u64);
                let walked = self.each_draw(pos, tiles.len(), |this, pos, drawn, ways| {
                    let turn = pos.exchange(drawn, &put, zeros);
                    let value = if ends {
                        Ok(pos.racks_lost(this.gc))
                    } else {
                        pos.next_turn();
                        let theirs = this.deeper(|this| this.node(pos));
                        pos.next_turn();
                        theirs.map(|theirs| -theirs)
                    };
                    pos.unexchange(drawn, &put, turn);
                    let value = value?;
                    each(drawn, ways, value);
                    sum += ways as f64 * value;
                    total += ways;
                    Ok(true)
                });
                self.arena.give(self.ply, TALLY, put);
                walked?;
                Ok(sum / total as f64)
            }
        }
    }

    // the chance a move leaves its mover's final spread above t (a tie counted
    // half), each player after it making the move node found best
    #[inline(always)]
    fn move_win(&mut self, pos: &mut InBag, play: &movegen::Play, t: f64) -> error::Returns<f64> {
        self.move_win_each(pos, play, t, |_, _, _| {})
    }

    // the same, each draw the move makes told with its deals and its chance
    // (drawn as a tally)
    fn move_win_each(
        &mut self,
        pos: &mut InBag,
        play: &movegen::Play,
        t: f64,
        mut each: impl FnMut(&[u8], u64, f64),
    ) -> error::Returns<f64> {
        match play {
            movegen::Play::Place { word, score, .. } => {
                let score = f64::from(score.raw());
                let n = word.iter().filter(|&&tile| tile != 0).count();
                if n < pos.bag_len() && self.at_horizon() {
                    let turn = pos.place(self.gc, play);
                    let value = self.estimate(score, &pos.racks[pos.mover as usize], &[]);
                    pos.unplace(self.gc, play, turn);
                    let win = beats(value, t);
                    self.each_draw_at_root(pos, n, |drawn, ways| each(drawn, ways, win));
                    return Ok(win);
                }
                let turn = pos.place(self.gc, play);
                let (mut sum, mut total) = (0.0, 0u64);
                let walked = self.each_draw(pos, n.min(pos.bag_len()), |this, pos, drawn, ways| {
                    pos.draw(drawn);
                    pos.next_turn();
                    // the mover's spread is the score less the next mover's
                    let win = if pos.bag_len() == 0 {
                        this.endgame(pos).map(|theirs| beats(score - theirs, t))
                    } else {
                        (this.deeper(|this| this.node_win(pos, score - t))).map(|w| 1.0 - w)
                    };
                    pos.next_turn();
                    pos.undraw(drawn);
                    let win = win?;
                    each(drawn, ways, win);
                    sum += ways as f64 * win;
                    total += ways;
                    Ok(true)
                });
                pos.unplace(self.gc, play, turn);
                walked?;
                Ok(sum / total as f64)
            }
            movegen::Play::Exchange { tiles } if tiles.is_empty() => {
                if pos.a_pass_ends(self.gc) {
                    return Ok(beats(pos.racks_lost(self.gc), t));
                }
                if self.at_horizon() {
                    let value = self.estimate(0.0, &pos.racks[pos.mover as usize], &[]);
                    return Ok(beats(value, t));
                }
                let turn = pos.pass();
                pos.next_turn();
                let theirs = self.deeper(|this| this.node_win(pos, -t));
                pos.next_turn();
                pos.set_turn(turn);
                Ok(1.0 - theirs?)
            }
            movegen::Play::Exchange { tiles } => {
                let zeros = if self.gc.exchanges_are_zeros() {
                    pos.zeros.saturating_add(1)
                } else {
                    0
                };
                let ends = scoreless_turns_end(self.gc, Some(&pos.board), 0, zeros);
                if !ends && self.at_horizon() {
                    let value = self.estimate(0.0, &pos.racks[pos.mover as usize], tiles);
                    let win = beats(value, t);
                    self.each_draw_at_root(pos, tiles.len(), |drawn, ways| each(drawn, ways, win));
                    return Ok(win);
                }
                let put = self.thrown(tiles);
                let (mut sum, mut total) = (0.0, 0u64);
                let walked = self.each_draw(pos, tiles.len(), |this, pos, drawn, ways| {
                    let turn = pos.exchange(drawn, &put, zeros);
                    let win = if ends {
                        Ok(beats(pos.racks_lost(this.gc), t))
                    } else {
                        pos.next_turn();
                        let theirs = this.deeper(|this| this.node_win(pos, -t));
                        pos.next_turn();
                        theirs.map(|theirs| 1.0 - theirs)
                    };
                    pos.unexchange(drawn, &put, turn);
                    let win = win?;
                    each(drawn, ways, win);
                    sum += ways as f64 * win;
                    total += ways;
                    Ok(true)
                });
                self.arena.give(self.ply, TALLY, put);
                walked?;
                Ok(sum / total as f64)
            }
        }
    }

    // the chance a position leaves its mover's final spread above t, each
    // player making the move node found best
    fn node_win(&mut self, pos: &mut InBag, t: f64) -> error::Returns<f64> {
        let play = self.node_play(pos)?;
        self.move_win(pos, &play, t)
    }

    // what the player to move at info gets under InfoSets (their chance to win
    // with their mean margin), choosing blind: the move whose mean over every
    // split of what they cannot see, weighed by its deals, is best, the chance
    // first, then the margin, a tie going to the first in in_bag_moves's order.
    // The move a shallower pass found best goes first, then the placements,
    // highest score first; a move stops once the splits left cannot lift its
    // chance to the best's (info_move). The chances come first, then the
    // margins of the moves at the best chance alone, a split whose margin the
    // chance's search left unfound solved whole
    fn info_value(&mut self, info: &InfoSet) -> error::Returns<(f64, f64)> {
        // read even where the info set has its value, as node does
        self.stop_at_a_limit()?;
        let key = info.key();
        let found = self.valued.read(key, |shard| {
            shard.infos.get(key).map(|entry| match entry {
                &BagEntry::Valued(value, _, exact) => Ok((value, exact)),
                BagEntry::First(play) => Err(play.clone()),
            })
        });
        let first = match found {
            Some(Ok((value, exact))) => {
                self.exact &= exact;
                return Ok(value);
            }
            Some(Err(play)) => Some(play),
            None => None,
        };
        let was_exact = std::mem::replace(&mut self.exact, true);
        let made = info.exchanges[info.mover as usize];
        let (gc, kwg, arm_tables, klv) = (self.gc, self.kwg, self.arm_tables, self.klv);
        // the moves, and the order they are asked in, in the ply's room
        let mut lists = self.arena.take_lists(self.ply);
        let PlyLists {
            moves,
            asked,
            scratch,
            splits,
            num_splits,
            valued,
            margins,
        } = &mut lists;
        moves_of::<LEAN, _, _, _, JUMPS, ANY_SIDE>(
            self.mg,
            &movegen::BoardSnapshot {
                board_tiles: &info.board,
                game_config: gc,
                kwg,
                anagrams: arm_tables.anagrams(),
                rack_lengths: arm_tables.rack_lengths(),
                klv,
            },
            &info.rack,
            made,
            info.bag_len,
            (moves, scratch),
            self.limits,
        )?;
        *num_splits = info.splits(self.limits, splits, scratch)?;
        asked.clear();
        self.limits.room(asked, moves.len())?;
        asked.extend(0..moves.len());
        asked.sort_unstable_by_key(|&i| match &moves[i] {
            play if first.as_ref() == Some(play) => {
                (false, std::cmp::Reverse(equity::Equity::INFINITY), i)
            }
            movegen::Play::Place { score, .. } => (true, std::cmp::Reverse(*score), i),
            _ => (true, std::cmp::Reverse(equity::Equity::NEG_INFINITY), i),
        });
        let splits = &splits[..*num_splits];
        let limits = self.limits;
        let found = (self.info_best(info, (moves, asked, splits), valued, margins)).and_then(
            |(win, margin, i)| {
                let _copy = limits.reserve(play_bytes(&moves[i]))?;
                Ok((win, margin, moves[i].clone()))
            },
        );
        self.arena.give_lists(self.ply, lists);
        let (win, margin, play) = found?;
        let exact = self.exact;
        self.exact = was_exact && exact;
        let entry = BagEntry::Valued((win, margin), play, exact);
        let memory = self.limits.memory;
        (self.valued).write(key, |shard| {
            shard.infos.keep(key, entry, memory, BagEntry::worth)
        });
        Ok((win, margin))
    }

    // the best of the moves at info, asked in this order, over these splits,
    // as info_value finds it: its chance, its margin, and its place in moves;
    // valued and margins the room it keeps the moves at the best chance in
    fn info_best(
        &mut self,
        info: &InfoSet,
        (moves, asked, splits): (&[movegen::Play], &[usize], &[Split]),
        valued: &mut Vec<(usize, f64)>,
        margins: &mut Vec<Option<f64>>,
    ) -> error::Returns<(f64, f64, usize)> {
        // a root task's hand-over opens these moves to the threads with none
        // left, who value them from the end into the shared table
        use std::sync::atomic::Ordering::Relaxed;
        let hand_overs = self.hand_overs;
        let help = self.hand_over.take().map(|task| &hand_overs[task]);
        let num_asked = asked.len();
        if let Some(help) = help {
            let bytes = (info.board.len() + info.rack.len() + info.unseen.len())
                + (asked.iter())
                    .map(|&i| std::mem::size_of::<movegen::Play>() + play_bytes(&moves[i]))
                    .sum::<usize>()
                + (splits.iter())
                    .map(|(rack, bag, _)| std::mem::size_of::<Split>() + rack.len() + bag.len())
                    .sum::<usize>()
                + 2 * std::mem::size_of::<usize>() // the Arc's two counts
                + std::mem::size_of::<HandOverWork>();
            let room = self.limits.reserve(bytes)?;
            let work = asked.iter().map(|&i| moves[i].clone()).collect();
            help.put((info.clone(), work, splits.to_vec()));
            drop(room);
        }
        valued.clear();
        margins.clear();
        let mut floor = None;
        for (k, &i) in asked.iter().enumerate() {
            if let Some(help) = help {
                help.forward.store(k, Relaxed);
            }
            // this move's margins follow those kept
            let at = margins.len();
            let Some(win) = self.info_move(info, &moves[i], splits, floor, margins)? else {
                continue;
            };
            // only the moves at the best chance are read below: those the floor
            // passes are let go as it rises, and none is kept below it
            if floor.is_none_or(|floor| win > floor) {
                floor = Some(win);
                valued.clear();
                margins.drain(..at);
                if let Some(help) = help {
                    help.floor.store(win.to_bits(), Relaxed);
                }
            } else if floor.is_some_and(|floor| win < floor) {
                margins.truncate(at);
                continue;
            }
            self.limits.room(valued, 1)?;
            valued.push((i, win));
        }
        if let Some(help) = help {
            help.forward.store(num_asked, Relaxed);
        }
        let total = splits.iter().map(|&(_, _, ways)| ways).sum::<u64>() as f64;
        let mut best = (f64::NEG_INFINITY, f64::NEG_INFINITY, usize::MAX);
        for (&(i, win), margins) in valued.iter().zip(margins.chunks(splits.len().max(1))) {
            if Some(win) != floor {
                continue;
            }
            let mut margin = 0.0;
            for ((other, bag, ways), &found) in splits.iter().zip(margins) {
                let m = match found {
                    Some(m) => m,
                    None => self.info_split(info, &moves[i], other, bag)?.1,
                };
                margin += *ways as f64 * m;
            }
            let margin = margin / total;
            if margin > best.1 || (margin == best.1 && i < best.2) {
                best = (win, margin, i);
            }
        }
        Ok(best)
    }

    // a move's chance to win at info over the splits weighed by their deals,
    // with each split's margin where its search found it pushed on margins;
    // none, margins as they were, once even a win in every split left would
    // leave its chance below floor. While the floor is above 0 a split is
    // asked its chance alone (info_split_chance); below that every move may
    // tie at the best chance, its margins wanted
    fn info_move(
        &mut self,
        info: &InfoSet,
        play: &movegen::Play,
        splits: &[(Vec<u8>, Vec<u8>, u64)],
        floor: Option<f64>,
        margins: &mut Vec<Option<f64>>,
    ) -> error::Returns<Option<f64>> {
        let total = splits.iter().map(|&(_, _, ways)| ways).sum::<u64>() as f64;
        let (mut win, mut left) = (0.0, total);
        // the margins' room for every split, its growth (the old list held
        // beside the new) taken only within the memory limit
        room_in(self.limits.memory.as_ref(), margins, splits.len())
            .map_err(|_| self.limits.stop_refused())?;
        let at = margins.len();
        for (other, bag, ways) in splits {
            // a share a hair under the floor is not ruled out, lest a sum's
            // rounding rule out a tie
            if floor.is_some_and(|floor| (win + left) / total < floor - 1e-9) {
                margins.truncate(at);
                return Ok(None);
            }
            let (w, m) = if floor.is_some_and(|floor| floor > 0.0) {
                self.info_split_chance(info, play, other, bag)?
            } else {
                let (w, m) = self.info_split(info, play, other, bag)?;
                (w, Some(m))
            };
            win += *ways as f64 * w;
            left -= *ways as f64;
            margins.push(m);
        }
        Ok(Some(win / total))
    }

    // a move's chance to win at info in one split, as info_split finds it, with
    // its margin where that came too: a placement that empties the bag asks its
    // endgame only whether the next mover's value leaves the mover a win or a
    // tie (endgame_below), which finds the margin of either but not of a loss
    fn info_split_chance(
        &mut self,
        info: &InfoSet,
        play: &movegen::Play,
        other: &[u8],
        bag: &[u8],
    ) -> error::Returns<(f64, Option<f64>)> {
        // read at each split, as info_split does
        self.stop_at_a_limit()?;
        if let movegen::Play::Place { word, score, .. } = play
            && word.iter().filter(|&&tile| tile != 0).count() >= info.bag_len
        {
            let mut next = self.arena.take_pos(self.ply);
            info.at(self.limits, other, bag, &mut next)?;
            next.place(self.gc, play);
            next.draw(bag);
            next.next_turn();
            // a win or a tie leaves the next mover the lead with the score at
            // most, every endgame value a whole number of millipoints
            let most = f64::from(info.lead) + f64::from(score.raw());
            let below = self.endgame_below(&next, most + 1.0);
            self.arena.give_pos(self.ply, next);
            return Ok(match below? {
                Some(theirs) => {
                    let margin = f64::from(score.raw()) - theirs;
                    (beats(f64::from(info.lead) + margin, 0.0), Some(margin))
                }
                None => (0.0, None),
            });
        }
        let (win, margin) = self.info_split(info, play, other, bag)?;
        Ok((win, Some(margin)))
    }

    // a move's chance to win and margin at info in one split of what the mover
    // cannot see (the other's rack, the bag's counts): once the bag empties,
    // the endgame, everything then known; else what the other then knows
    // (info_value), their chance with their margin turned to the mover's. The
    // other sees the mover's tiles with the bag's, whatever the mover draws
    fn info_split(
        &mut self,
        info: &InfoSet,
        play: &movegen::Play,
        other: &[u8],
        bag: &[u8],
    ) -> error::Returns<(f64, f64)> {
        // read at each split: a blind choice values every move over every
        // split, and one that ends the game (an exchange with the scoreless
        // turns at the count, its draws averaged) visits no position at all
        self.stop_at_a_limit()?;
        let lead = f64::from(info.lead);
        match play {
            movegen::Play::Place { word, score, .. } => {
                let n = word.iter().filter(|&&tile| tile != 0).count();
                // the split's position after the move, in the ply's room
                let mut pos = self.arena.take_pos(self.ply);
                info.at(self.limits, other, bag, &mut pos)?;
                pos.place(self.gc, play);
                if n >= info.bag_len {
                    pos.draw(bag);
                    pos.next_turn();
                    let ev = self.endgame(&pos);
                    self.arena.give_pos(self.ply, pos);
                    let margin = f64::from(score.raw()) - ev?;
                    return Ok((beats(lead + margin, 0.0), margin));
                }
                if self.at_horizon() {
                    let margin =
                        self.estimate(f64::from(score.raw()), &pos.racks[info.mover as usize], &[]);
                    self.arena.give_pos(self.ply, pos);
                    return Ok((beats(lead + margin, 0.0), margin));
                }
                let mut next = self.arena.take_next(self.ply);
                let leave = &pos.racks[info.mover as usize];
                let turn = TheirTurn {
                    bag_len: info.bag_len - n,
                    passes: 0,
                    zeros: 0,
                    answering: false,
                    exchanges: info.exchanges,
                    lead: -(info.lead + score.raw()),
                };
                info.theirs(
                    self.limits,
                    &mut next,
                    (&pos.board, other, bag, leave),
                    turn,
                )?;
                self.arena.give_pos(self.ply, pos);
                let found = self.deeper(|this| this.info_value(&next));
                self.arena.give_next(self.ply, next);
                let (w, m) = found?;
                Ok((1.0 - w, f64::from(score.raw()) - m))
            }
            movegen::Play::Exchange { tiles } if tiles.is_empty() => {
                // the game's end where the pass answers a pass the line made or
                // brings the scoreless turns to the config's count
                let (passes, zeros) = (info.passes.saturating_add(1), info.zeros.saturating_add(1));
                if info.answering || scoreless_turns_end(self.gc, Some(&info.board), passes, zeros)
                {
                    // each player loses their own rack (InBag::racks_lost)
                    let alphabet = self.gc.alphabet();
                    let lost = f64::from(
                        (alphabet.scaled_rack_score(other)
                            - alphabet.scaled_rack_score(&info.rack))
                        .raw(),
                    );
                    return Ok((beats(lead + lost, 0.0), lost));
                }
                if self.at_horizon() {
                    let margin = self.estimate(0.0, &info.rack, &[]);
                    return Ok((beats(lead + margin, 0.0), margin));
                }
                let mut next = self.arena.take_next(self.ply);
                let turn = TheirTurn {
                    bag_len: info.bag_len,
                    passes,
                    zeros,
                    answering: true,
                    exchanges: info.exchanges,
                    lead: -info.lead,
                };
                info.theirs(
                    self.limits,
                    &mut next,
                    (&info.board, other, bag, &info.rack),
                    turn,
                )?;
                let found = self.deeper(|this| this.info_value(&next));
                self.arena.give_next(self.ply, next);
                let (w, m) = found?;
                Ok((1.0 - w, -m))
            }
            movegen::Play::Exchange { tiles } => {
                let zeros = if self.gc.exchanges_are_zeros() {
                    info.zeros.saturating_add(1)
                } else {
                    0
                };
                let mut exchanges = info.exchanges;
                exchanges[info.mover as usize] += 1;
                if scoreless_turns_end(self.gc, Some(&info.board), 0, zeros) {
                    // the game ends with the mover holding what they drew in
                    // place of the tiles: the mean over the draws
                    let put = self.thrown(tiles);
                    let (mut win, mut margin, mut total) = (0.0, 0.0, 0u64);
                    let mut pos = self.arena.take_pos(self.ply);
                    info.at(self.limits, other, bag, &mut pos)?;
                    let walked = self.each_draw(&mut pos, tiles.len(), |this, pos, drawn, ways| {
                        pos.draw(drawn);
                        pos.put_back(&put);
                        let lost = pos.racks_lost(this.gc);
                        pos.take_back(&put);
                        pos.undraw(drawn);
                        win += ways as f64 * beats(lead + lost, 0.0);
                        margin += ways as f64 * lost;
                        total += ways;
                        Ok(true)
                    });
                    self.arena.give(self.ply, TALLY, put);
                    self.arena.give_pos(self.ply, pos);
                    walked?;
                    return Ok((win / total as f64, margin / total as f64));
                }
                if self.at_horizon() {
                    let margin = self.estimate(0.0, &info.rack, tiles);
                    return Ok((beats(lead + margin, 0.0), margin));
                }
                let mut next = self.arena.take_next(self.ply);
                let turn = TheirTurn {
                    bag_len: info.bag_len,
                    passes: 0,
                    zeros,
                    answering: false,
                    exchanges,
                    lead: -info.lead,
                };
                info.theirs(
                    self.limits,
                    &mut next,
                    (&info.board, other, bag, &info.rack),
                    turn,
                )?;
                let found = self.deeper(|this| this.info_value(&next));
                self.arena.give_next(self.ply, next);
                let (w, m) = found?;
                Ok((1.0 - w, -m))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{EndgameSolver, PegMove, ordered_play_index, splitmix64};
    use crate::{alphabet, anagrams, bites, build, equity, fash, game_config, klv, kwg, movegen};

    // a one-in-bag answer with every row's hypotheses read out of the search's
    // margins, as the tests compare them
    struct Solved {
        win_pct: f64,
        expected_margin: equity::Equity,
        hypotheses: Vec<(u8, u32, equity::Equity)>,
        best_move: Option<PegMove>,
        moves: Vec<SolvedMove>,
        committed: bool,
        exact: bool,
        depth: i8,
    }

    struct SolvedMove {
        play: PegMove,
        win_pct: f64,
        expected_margin: equity::Equity,
        hypotheses: Vec<(u8, u32, equity::Equity)>,
        exact: bool,
    }

    // the answer with its hypotheses, and its moves' (all of them, as a resume
    // filling moves lists them)
    fn solved(progress: &super::PegProgress, result: super::PegResult) -> Solved {
        Solved {
            win_pct: result.win_pct,
            expected_margin: result.expected_margin,
            hypotheses: progress.hypotheses(result.row).collect(),
            best_move: result.best_move,
            moves: (result.moves.into_iter())
                .map(|m| SolvedMove {
                    hypotheses: progress.hypotheses(Some(m.row)).collect(),
                    play: m.play,
                    win_pct: m.win_pct,
                    expected_margin: m.expected_margin,
                    exact: m.exact,
                })
                .collect(),
            committed: result.committed,
            exact: result.exact,
            depth: result.depth,
        }
    }

    // solve_peg_one_in_bag, its answer read out with its progress (solved)
    trait SolveFull {
        fn solve_peg_one_in_bag_full(
            &mut self,
            mover: u8,
            board: &[u8],
            mover_rack: &[u8],
            unseen_tally: &[u8],
            score_diff: equity::Equity,
            limits: &super::Limits,
        ) -> crate::error::Returns<Result<Solved, super::PegUnsupported>>;
    }

    impl<
        N: kwg::Node,
        L: kwg::Node,
        const LEAN: bool,
        G: movegen::LaneBits,
        const JUMPS: bool,
        const ANY_SIDE: bool,
    > SolveFull for EndgameSolver<'_, N, L, LEAN, G, JUMPS, ANY_SIDE>
    {
        fn solve_peg_one_in_bag_full(
            &mut self,
            mover: u8,
            board: &[u8],
            mover_rack: &[u8],
            unseen_tally: &[u8],
            score_diff: equity::Equity,
            limits: &super::Limits,
        ) -> crate::error::Returns<Result<Solved, super::PegUnsupported>> {
            let mut progress =
                super::PegProgress::new(mover, board, mover_rack, unseen_tally, score_diff, [0, 0]);
            let result = self.resume_peg_one_in_bag(&mut progress, limits)?;
            Ok(result.map(|result| solved(&progress, result)))
        }
    }

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
        kwg: &kwg::Kwg<kwg::Node24>,
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
                board[strider.at_in_lane(i)] = tile;
                let blanked = tile & !((tile as i8) >> 7) as u8;
                let p = rack.iter().rposition(|&t| t == blanked).unwrap();
                rack[p] = 0x80;
            }
        }
        rack.retain(|&t| t != 0x80);
    }

    // a state a reference solve has valued: the kind of search, the board, the
    // racks, the mover, the bag's tile, the passes, the zeros, and whether the
    // move before was a pass
    type ReferenceState = (u8, Vec<u8>, Vec<u8>, Vec<u8>, u8, u8, u8, u8, bool);

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
    // config's counts, and answering whether the game's last turn before the
    // search was a pass, which a pass here answers and so ends the game. The
    // search counts its own passes to the end, which values every position as
    // ending a pass answering a pass does: the position is as it was two plies
    // back
    fn reference<N: kwg::Node, L: kwg::Node>(
        fixed: &mut Reference<'_, N, L>,
        board: &[u8],
        racks: &[Vec<u8>; 2],
        mover: usize,
        passes: u8,
        zeros: u8,
        answering: bool,
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
            answering,
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
        fixed
            .mg
            .gen_moves_raw_all_unsorted(
                &snapshot,
                &racks[mover],
                0,
                movegen::PassPolicy::AsACandidate,
            )
            .unwrap();

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
                    (score.raw() + 2 * alphabet.scaled_rack_score(&racks[opp]).raw()) as f32
                } else {
                    let mut nb = board.to_vec();
                    let mut nr = racks.clone();
                    apply_place(fixed.gc, &mut nb, &mut nr[mover], *down, *lane, *idx, word);
                    score.raw() as f32 - reference(fixed, &nb, &nr, opp, 0, 0, false)
                };
                if val > best {
                    best = val;
                }
            }
        }

        let pass_passes = passes.saturating_add(1);
        let pass_zeros = zeros.saturating_add(1);
        let pass_val = if answering
            || super::scoreless_turns_end(fixed.gc, Some(board), pass_passes, pass_zeros)
        {
            (alphabet.scaled_rack_score(&racks[opp]).raw()
                - alphabet.scaled_rack_score(&racks[mover]).raw()) as f32
        } else {
            -reference(fixed, board, racks, opp, pass_passes, pass_zeros, false)
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
        turn: super::OneInBagTurn,
    ) -> f32 {
        let super::OneInBagTurn {
            mover,
            bag_tile,
            passes,
            zeros,
            answering,
        } = turn;
        let mover = mover as usize;
        let key = (
            1u8,
            board.to_vec(),
            racks[0].clone(),
            racks[1].clone(),
            mover as u8,
            bag_tile,
            passes,
            zeros,
            answering,
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
        fixed
            .mg
            .gen_moves_raw_all_unsorted(
                &snapshot,
                &racks[mover],
                0,
                movegen::PassPolicy::AsACandidate,
            )
            .unwrap();

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

                let val = score.raw() as f32 - reference(fixed, &nb, &nr, opp, 0, 0, false);
                if val > best {
                    best = val;
                }
            }
        }

        // a pass answering a pass ends the game, as one bringing the scoreless
        // turns to the config's count does
        let pass_passes = passes.saturating_add(1);
        let pass_zeros = zeros.saturating_add(1);
        let pass_val =
            if answering || super::scoreless_turns_end(fixed.gc, None, pass_passes, pass_zeros) {
                (alphabet.scaled_rack_score(&racks[opp]).raw()
                    - alphabet.scaled_rack_score(&racks[mover]).raw()) as f32
            } else {
                -reference_one_in_bag(
                    fixed,
                    board,
                    racks,
                    super::OneInBagTurn {
                        mover: opp as u8,
                        bag_tile,
                        passes: pass_passes,
                        zeros: pass_zeros,
                        answering: true,
                    },
                )
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
        turn: super::OneInBagTurn,
    ) -> f32 {
        let super::OneInBagTurn {
            mover,
            bag_tile,
            passes,
            zeros,
            answering,
        } = turn;
        let mover = mover as usize;
        let key = (
            2u8,
            board.to_vec(),
            racks[0].clone(),
            racks[1].clone(),
            mover as u8,
            bag_tile,
            passes,
            zeros,
            answering,
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
        fixed
            .mg
            .gen_moves_raw_all_unsorted(
                &snapshot,
                &racks[mover],
                0,
                movegen::PassPolicy::AsACandidate,
            )
            .unwrap();
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
                let val = score.raw() as f32 - reference(fixed, &nb, &nr, opp, 0, 0, false);
                if val > best {
                    best = val;
                }
            }
        }

        // a pass answering a pass ends the game, as one bringing the scoreless
        // turns to the config's count does
        let pass_passes = passes + 1;
        let pass_zeros = zeros + 1;
        let pass_val =
            if answering || super::scoreless_turns_end(fixed.gc, None, pass_passes, pass_zeros) {
                (alphabet.scaled_rack_score(&racks[opp]).raw()
                    - alphabet.scaled_rack_score(&racks[mover]).raw()) as f32
            } else {
                -reference_one_in_bag_ex(
                    fixed,
                    board,
                    racks,
                    super::OneInBagTurn {
                        mover: opp as u8,
                        bag_tile,
                        passes: pass_passes,
                        zeros: pass_zeros,
                        answering: true,
                    },
                )
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
                (alphabet.scaled_rack_score(&nr[opp]).raw()
                    - alphabet.scaled_rack_score(&nr[mover]).raw()) as f32
            } else {
                -reference_one_in_bag_ex(
                    fixed,
                    board,
                    &nr,
                    super::OneInBagTurn {
                        mover: opp as u8,
                        bag_tile: r,
                        passes: 0,
                        zeros: exch_zeros,
                        answering: false,
                    },
                )
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

    // the positions of all_positions with these names, in its order
    fn named_positions(names: &[&str]) -> Vec<(String, Position)> {
        (all_positions().into_iter())
            .filter(|(name, _)| names.contains(&&name[..]))
            .collect()
    }

    // sort a ply's whole list, as the search does past the first move
    #[inline]
    fn order_ply(
        egs: &mut EndgameSolver<'_, kwg::Node24, kwg::Node24>,
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
    fn a_solve_on_threads_finds_one_threads_value() {
        let gc = game_config::make_english_game_config();
        let kwg_bytes = tiny_kwg_bytes();
        let kwg = kwg::kbwg_from_bytes_alloc(kwg_bytes.to_vec(), false).unwrap();
        let arm_tables = anagrams::ArmTables::build(&kwg, &gc, false).unwrap();
        let solved = |pos: &Position, mover: u8, threads: usize| {
            let mut egs =
                EndgameSolver::<kwg::Node24, kwg::Node24>::new(&gc, &kwg, &arm_tables).unwrap();
            egs.init(&pos.board, [&pos.racks[0][..], &pos.racks[1][..]]);
            let solved = egs
                .solve_within_on_threads(mover, &super::Limits::NONE, threads)
                .unwrap();
            let orders = &egs.work_buffer.orders;
            let split_ply = usize::from(orders[0].is_empty());
            assert!(
                egs.work_buffer.helper_tables.len() <= orders[split_ply].len().saturating_sub(1)
            );
            solved.value
        };
        // and one where a root move beats the first tried by under a point
        let close = random_positions(1100).pop();
        for (name, pos) in all_positions().into_iter().chain(close) {
            for mover in 0..2u8 {
                let want = solved(&pos, mover, 1);
                for threads in 2..4 {
                    assert_eq!(
                        solved(&pos, mover, threads),
                        want,
                        "[{name}] mover {mover} threads {threads}"
                    );
                }
            }
        }
    }

    #[test]
    fn helper_tables_share_the_memory_left_to_reserve() {
        let gc = game_config::make_english_game_config();
        let kwg = kwg::kbwg_from_bytes_alloc(tiny_kwg_bytes().to_vec(), false).unwrap();
        let tables = anagrams::ArmTables::build(&kwg, &gc, false).unwrap();
        let mut solver =
            EndgameSolver::<kwg::Node24, kwg::Node24>::new(&gc, &kwg, &tables).unwrap();
        solver.init(&[0; 225], [&[1, 1, 2, 8, 12, 20, 20], &[5, 6]]);
        assert!(solver.reserve_table(32 << 10));
        solver.reset_position();
        solver.generate_root(0, false).unwrap();
        let node = super::Enter {
            ply: 0,
            player_idx: 0,
            depth: 1,
            alpha: equity::Equity::ZERO,
            beta: equity::Equity::INFINITY,
            scoreless: super::Scoreless::Counted {
                passes: 0,
                zeros: 0,
            },
        };
        let work = &mut solver.work_buffer;
        work.frames.push(super::Frame::new(node, true, 0, false));
        work.threads = 4;
        let limit = super::MemoryLimit {
            bytes: 64 << 10,
            held: || 32 << 10,
            reserved: None,
        };
        work.memory_limit = Some(limit);
        // depth zero below each helper move leaves its initial table size.
        solver.split_verdicts(0, 0, node.depth).unwrap();
        assert_eq!(solver.work_buffer.helper_tables.len(), 4);
        let bytes = solver
            .work_buffer
            .helper_tables
            .iter()
            .flatten()
            .map(|kept| kept.table.buckets.len() * std::mem::size_of::<[u64; 8]>())
            .sum::<usize>();
        assert!(bytes <= limit.room(), "{bytes} helper table bytes");
    }

    #[test]
    fn the_only_first_play_can_split_its_replies() {
        let gc = game_config::make_english_game_config();
        let kwg = kwg::kbwg_from_bytes_alloc(tiny_kwg_bytes().to_vec(), false).unwrap();
        let klv = klv::kbwg_from_bytes_alloc(klv::EMPTY_KLV_BYTES.to_vec()).unwrap();
        let tables = anagrams::ArmTables::build(&kwg, &gc, false).unwrap();
        let mut split = [false; 2];
        for (name, pos) in all_positions().into_iter().take(40) {
            let mut state = crate::game_state::GameState::new(&gc);
            state.bag.set_from_iter([]);
            state.board_tiles.copy_from_slice(&pos.board);
            state.players[0].rack.extend_from_slice(&pos.racks[0]);
            state.players[1].rack.extend_from_slice(&pos.racks[1]);
            let snapshot = movegen::BoardSnapshot {
                board_tiles: &state.board_tiles,
                game_config: &gc,
                kwg: &kwg,
                anagrams: None,
                rack_lengths: None,
                klv: &klv,
            };
            let mut choices =
                EndgameSolver::<kwg::Node24, kwg::Node24>::new(&gc, &kwg, &tables).unwrap();
            choices.init(&pos.board, [&pos.racks[0], &pos.racks[1]]);
            choices.reset_position();
            choices.generate_root(0, false).unwrap();
            let place = choices.work_buffer.lists[0]
                .iter()
                .find(|candidate| !candidate.plays_out)
                .map(|candidate| candidate.play.clone());
            let pass = movegen::Play::Exchange {
                tiles: [][..].into(),
            };
            for (kind, play) in [Some(pass), place].into_iter().enumerate() {
                let Some(play) = play else { continue };
                let root = super::RootMoves::checked(&snapshot, &state, vec![play]).unwrap();
                let solve = |threads| {
                    let mut solver =
                        EndgameSolver::<kwg::Node24, kwg::Node24>::new(&gc, &kwg, &tables)
                            .unwrap()
                            .with_root_moves(Some(&root));
                    solver.init(&pos.board, [&pos.racks[0], &pos.racks[1]]);
                    let result = solver
                        .solve_within_on_threads(0, &super::Limits::NONE, threads)
                        .unwrap();
                    assert!(result.exact);
                    (result.value, solver.work_buffer.helper_tables.len())
                };
                let (serial, _) = solve(1);
                let (parallel, helpers) = solve(4);
                assert_eq!(serial, parallel, "{name}, root kind {kind}");
                split[kind] |= helpers != 0;
            }
        }
        assert_eq!(split, [true; 2]);
    }

    #[test]
    #[inline]
    fn a_one_in_bag_solve_on_threads_finds_one_threads_answer() {
        let gc = game_config::make_english_game_config();
        let kwg_bytes = tiny_kwg_bytes();
        let kwg = kwg::kbwg_from_bytes_alloc(kwg_bytes.to_vec(), false).unwrap();
        let arm_tables = anagrams::ArmTables::build(&kwg, &gc, false).unwrap();
        let named = |play: &PegMove| match play {
            PegMove::Pass => None,
            PegMove::Place(movegen::Play::Place {
                down,
                lane,
                idx,
                word,
                score,
            }) => Some((*down, *lane, *idx, word[..].to_vec(), *score)),
            PegMove::Place(movegen::Play::Exchange { .. }) => unreachable!("not a place"),
        };
        let solved = |board: &[u8], rack: &[u8], unseen_tally: &[u8], threads: usize| {
            let mut egs =
                EndgameSolver::<kwg::Node24, kwg::Node24>::new(&gc, &kwg, &arm_tables).unwrap();
            let mut progress =
                super::PegProgress::new(0, board, rack, unseen_tally, equity::Equity::ZERO, [0, 0]);
            let result = egs
                .resume_peg_one_in_bag_on_threads(&mut progress, &super::Limits::NONE, threads)
                .unwrap()
                .expect("english one-in-bag is always solvable");
            let result = solved(&progress, result);
            let moves = (result.moves.iter())
                .map(|m| {
                    (
                        named(&m.play),
                        m.win_pct,
                        m.expected_margin,
                        m.hypotheses.clone(),
                        m.exact,
                    )
                })
                .collect::<Vec<_>>();
            (
                result.win_pct,
                result.expected_margin,
                result.hypotheses,
                result.best_move.as_ref().map(named),
                moves,
                result.exact,
            )
        };
        for (name, pos) in
            named_positions(&["3-tile (board AAH; p0=[B,A,T] p1=[H,A,T])", "rand#36"])
        {
            let Some((&kept, rack)) = pos.racks[0].split_last() else {
                continue;
            };
            if rack.is_empty() {
                continue;
            }
            let mut unseen_tally = vec![0u8; gc.alphabet().len() as usize];
            for &tile in pos.racks[1].iter().chain([&kept]) {
                unseen_tally[tile as usize] += 1;
            }
            let want = solved(&pos.board, rack, &unseen_tally, 1);
            for threads in 2..4 {
                assert!(
                    solved(&pos.board, rack, &unseen_tally, threads) == want,
                    "[{name}] threads {threads}"
                );
            }
        }
    }

    #[test]
    #[inline]
    fn every_first_move_is_valued_exactly_warm_or_cold() {
        // each first move's value, with no limit, is exact; the best is the
        // solve's, none is more, and a solver warm from that solve values every
        // move as a cold one does
        let gc = game_config::make_english_game_config();
        let kwg_bytes = tiny_kwg_bytes();
        let kwg = kwg::kbwg_from_bytes_alloc(kwg_bytes.to_vec(), false).unwrap();
        let arm_tables = anagrams::ArmTables::build(&kwg, &gc, false).unwrap();
        let mut moves = 0usize;
        let mut checked = 0usize;
        for (name, pos) in all_positions()
            .into_iter()
            .enumerate()
            .filter(|(i, _)| [15, 95, 195].contains(i))
            .map(|(_, named)| named)
        {
            let racks = [&pos.racks[0][..], &pos.racks[1][..]];
            for mover in 0..2u8 {
                let mut cold =
                    EndgameSolver::<kwg::Node24, kwg::Node24>::new(&gc, &kwg, &arm_tables).unwrap();
                cold.init(&pos.board, racks);
                let cold_moves = cold.value_first_moves(mover, &super::Limits::NONE).unwrap();
                let mut warm =
                    EndgameSolver::<kwg::Node24, kwg::Node24>::new(&gc, &kwg, &arm_tables).unwrap();
                warm.init(&pos.board, racks);
                let want = warm.solve(mover).unwrap();
                let warm_moves = warm.value_first_moves(mover, &super::Limits::NONE).unwrap();
                assert!(!cold_moves.is_empty(), "[{name}] mover {mover}");
                assert!(
                    cold_moves
                        .iter()
                        .all(|first| first.exact && first.value.is_some()),
                    "[{name}] mover {mover}"
                );
                assert_eq!(cold_moves[0].value, Some(want), "[{name}] mover {mover}");
                assert!(
                    cold_moves.iter().all(|first| first.value <= Some(want)),
                    "[{name}] mover {mover}"
                );
                let values = |firsts: &[super::FirstMove]| {
                    let mut values = (firsts.iter())
                        .map(|first| {
                            let key = match &first.play {
                                movegen::Play::Place {
                                    down,
                                    lane,
                                    idx,
                                    word,
                                    score,
                                } => Some((*down, *lane, *idx, word[..].to_vec(), *score)),
                                movegen::Play::Exchange { .. } => None,
                            };
                            (key, first.value)
                        })
                        .collect::<Vec<_>>();
                    values.sort_unstable();
                    values
                };
                assert_eq!(
                    values(&cold_moves),
                    values(&warm_moves),
                    "[{name}] mover {mover}"
                );
                // a placing move that leaves the mover tiles is its score less
                // the reply's own solve on the board after it
                let mut after = Vec::new();
                for first in &cold_moves {
                    let movegen::Play::Place { word, score, .. } = &first.play else {
                        continue;
                    };
                    let mut left = racks[mover as usize].to_vec();
                    crate::game_state::use_tiles(
                        &mut left,
                        word.iter()
                            .filter(|&&tile| tile != 0)
                            .map(|&tile| if tile & 0x80 != 0 { 0 } else { tile }),
                    )
                    .unwrap();
                    if left.is_empty() {
                        continue;
                    }
                    crate::glance::board_after(&gc, &pos.board, &first.play, &mut after);
                    let mut next = [racks[0].to_vec(), racks[1].to_vec()];
                    next[mover as usize] = left;
                    let mut reply =
                        EndgameSolver::<kwg::Node24, kwg::Node24>::new(&gc, &kwg, &arm_tables)
                            .unwrap();
                    reply.init(&after, [&next[0][..], &next[1][..]]);
                    let replied = reply.solve(mover ^ 1).unwrap();
                    assert_eq!(
                        first.value,
                        Some(*score - replied),
                        "[{name}] mover {mover}"
                    );
                    checked += 1;
                }
                // the pass is less the reply's solve, the reply's own pass
                // ending the game
                let passed = (cold_moves.iter())
                    .find(|first| matches!(first.play, movegen::Play::Exchange { .. }))
                    .unwrap();
                let mut reply =
                    EndgameSolver::<kwg::Node24, kwg::Node24>::new(&gc, &kwg, &arm_tables).unwrap();
                reply.init_after_scoreless_turns(&pos.board, racks, 1, 1);
                let replied = reply.solve(mover ^ 1).unwrap();
                assert_eq!(
                    passed.value,
                    Some(equity::Equity::new(-replied.raw())),
                    "[{name}] mover {mover}: the pass"
                );
                moves += cold_moves.len();
            }
        }
        println!("{moves} first moves valued, {checked} checked by the reply's solve");
        assert!(checked > 0);
    }

    #[test]
    #[inline]
    fn a_solve_in_steps_goes_on_where_each_step_stopped() {
        // a page's steps: each a fresh solver given the table the step before
        // took (a pass a limit stopped with it), deepening on from the depth
        // the steps before finished, its line read between steps; each step
        // stops a few stored positions on, and the last answers as one solve
        let gc = game_config::make_english_game_config();
        let kwg_bytes = tiny_kwg_bytes();
        let kwg = kwg::kbwg_from_bytes_alloc(kwg_bytes.to_vec(), false).unwrap();
        let arm_tables = anagrams::ArmTables::build(&kwg, &gc, false).unwrap();
        let mut resumed = 0u64;
        let mut stepped = 0usize;
        for (name, pos) in all_positions() {
            let racks = [&pos.racks[0][..], &pos.racks[1][..]];
            for mover in 0..2u8 {
                let mut whole =
                    EndgameSolver::<kwg::Node24, kwg::Node24>::new(&gc, &kwg, &arm_tables).unwrap();
                whole.init(&pos.board, racks);
                let want = whole.solve(mover).unwrap();
                let mut table = None;
                let mut finished = None;
                for step in 0.. {
                    assert!(step < 100_000, "[{name}] mover {mover}: no end");
                    let mut solver =
                        EndgameSolver::<kwg::Node24, kwg::Node24>::new(&gc, &kwg, &arm_tables)
                            .unwrap();
                    if let Some(table) = table.take() {
                        assert!(solver.give_table(table).is_ok());
                    }
                    // a list it generates is numbered past every list kept
                    let generated = solver.work_buffer.generated;
                    let kept = &solver.work_buffer.serials;
                    assert!(kept.iter().all(|&serial| serial <= generated), "[{name}]");
                    solver.init(&pos.board, racks);
                    let limits = super::Limits {
                        states: Some(solver.work_buffer.table.held + 7),
                        ..super::Limits::NONE
                    };
                    let solved = match finished {
                        Some(finished) => solver.solve_within_after(mover, &limits, finished),
                        None => solver.solve_within(mover, &limits),
                    }
                    .unwrap();
                    resumed += solver.work_buffer.resumed;
                    let mut line = Vec::new();
                    solver.collect_pv(mover, &mut line).unwrap();
                    let mut again = Vec::new();
                    solver.collect_pv(mover, &mut again).unwrap();
                    assert!(again == line, "[{name}] mover {mover}");
                    table = Some(solver.take_table());
                    finished = Some((solved.depth, solved.value.unwrap()));
                    stepped += 1;
                    if solved.exact {
                        assert_eq!(solved.value.unwrap(), want, "[{name}] mover {mover}");
                        assert_eq!(
                            line.first().map(|ply| ply.0),
                            Some(want),
                            "[{name}] mover {mover}"
                        );
                        break;
                    }
                }
            }
        }
        println!("{stepped} steps, {resumed} passes resumed");
        assert!(resumed > 0);
    }

    #[test]
    #[inline]
    fn a_value_cut_short_is_no_bound_either_way() {
        let gc = game_config::make_english_game_config();
        let kwg_bytes = tiny_kwg_bytes();
        let kwg = kwg::kbwg_from_bytes_alloc(kwg_bytes.to_vec(), false).unwrap();
        let arm_tables = anagrams::ArmTables::build(&kwg, &gc, false).unwrap();
        let (mut below, mut above, mut equal) = ([0u64; 3], [0u64; 3], [0u64; 3]);
        for (_name, pos) in all_positions()
            .into_iter()
            .enumerate()
            .filter(|(i, _)| [15, 64].contains(i))
            .map(|(_, named)| named)
        {
            let racks = [&pos.racks[0][..], &pos.racks[1][..]];
            for mover in 0..2u8 {
                let new = || {
                    let mut s =
                        EndgameSolver::<kwg::Node24, kwg::Node24>::new(&gc, &kwg, &arm_tables)
                            .unwrap();
                    s.init(&pos.board, racks);
                    s
                };
                let want = new().solve(mover).unwrap();
                let exact_firsts = new()
                    .value_first_moves(mover, &super::Limits::NONE)
                    .unwrap();
                let mut tally = |k: usize, got: equity::Equity, want: equity::Equity| {
                    if got < want {
                        below[k] += 1
                    } else if got > want {
                        above[k] += 1
                    } else {
                        equal[k] += 1
                    }
                };
                let longest = new().longest_line(mover) as i8;
                for cap in 1..longest {
                    let mut s = new();
                    s.work_buffer.depth_cap = cap;
                    let solved = s.solve_within(mover, &super::Limits::NONE).unwrap();
                    if !solved.exact {
                        tally(0, solved.value.unwrap(), want);
                    }
                }
                for states in [1usize, 2, 3, 5, 8, 13, 21, 34, 55, 89, 144, 233] {
                    let limits = super::Limits {
                        states: Some(states),
                        ..super::Limits::NONE
                    };
                    let solved = new().solve_within(mover, &limits).unwrap();
                    if !solved.exact {
                        tally(1, solved.value.unwrap(), want);
                    }
                    for first in new().value_first_moves(mover, &limits).unwrap() {
                        if let (false, Some(v)) = (first.exact, first.value) {
                            let w = exact_firsts.iter().find(|f| f.play == first.play).unwrap();
                            tally(2, v, w.value.unwrap());
                        }
                    }
                }
            }
        }
        println!("below {below:?}, above {above:?}, equal {equal:?}");
        assert!(below.iter().chain(&above).all(|&n| n > 0));
    }

    #[test]
    #[inline]
    fn a_peg_value_cut_short_is_no_bound_either_way() {
        let kwg = kwg::kbwg_from_bytes_alloc(tiny_kwg_bytes().to_vec(), false).unwrap();
        let mut board = empty_board();
        put_word(&mut board, 7, 6, &[1, 20]);
        let (mut below, mut above, mut equal) = ([0u64; 4], [0u64; 4], [0u64; 4]);
        let mut tally = |k: usize, got: f64, want: f64| {
            if got < want {
                below[k] += 1
            } else if got > want {
                above[k] += 1
            } else {
                equal[k] += 1
            }
        };
        for gc in [
            game_config::make_english_game_config(),
            game_config::make_exchange_test_game_config(),
        ] {
            let arm_tables = anagrams::ArmTables::build(&kwg, &gc, false).unwrap();
            let mut unseen_tally = vec![0u8; gc.alphabet().len() as usize];
            for t in [1, 8] {
                unseen_tally[t] += 1;
            }
            for rack in [[2u8, 8], [8, 20]] {
                let new = || {
                    EndgameSolver::<kwg::Node24, kwg::Node24>::new(&gc, &kwg, &arm_tables).unwrap()
                };
                let solved = |limits| {
                    new()
                        .solve_peg_one_in_bag_full(
                            0,
                            &board,
                            &rack,
                            &unseen_tally,
                            equity::Equity::ZERO,
                            limits,
                        )
                        .unwrap()
                };
                let Ok(full) = solved(&super::Limits::NONE) else {
                    continue;
                };
                let got = solved(&super::Limits {
                    states: Some(118),
                    ..super::Limits::NONE
                })
                .unwrap();
                if !got.exact && got.depth > 0 {
                    tally(
                        0,
                        f64::from(got.expected_margin.raw()),
                        f64::from(full.expected_margin.raw()),
                    );
                    tally(1, got.win_pct, full.win_pct);
                }
            }
        }
        let gc = game_config::make_english_game_config();
        let arm_tables = anagrams::ArmTables::build(&kwg, &gc, false).unwrap();
        let mut egs =
            EndgameSolver::<kwg::Node24, kwg::Node24>::new(&gc, &kwg, &arm_tables).unwrap();
        let mut unseen_tally = vec![0u8; gc.alphabet().len() as usize];
        for tile in [1, 2, 8] {
            unseen_tally[tile] += 1;
        }
        for rack in [[2u8, 2], [1, 1]] {
            let position = bag_position(&board, &rack, &unseen_tally, 2, equity::Equity::ZERO);
            let full = egs
                .solve_peg(position.clone(), &super::Limits::NONE, 1)
                .unwrap()
                .unwrap();
            let limits = super::Limits {
                states: Some(62),
                ..super::Limits::NONE
            };
            let mut solver =
                EndgameSolver::<kwg::Node24, kwg::Node24>::new(&gc, &kwg, &arm_tables).unwrap();
            let got = solver.solve_peg(position, &limits, 1).unwrap().unwrap();
            if !got.exact && got.best_move.is_some() {
                tally(
                    2,
                    f64::from(got.expected_margin.raw()),
                    f64::from(full.expected_margin.raw()),
                );
                tally(3, got.win_pct, full.win_pct);
            }
        }
        println!("below {below:?}, above {above:?}, equal {equal:?}");
        assert!(below.iter().chain(&above).all(|&n| n > 0));
    }

    // line, solved for mover from the position with no scoreless turns, is a
    // best line by the reference: each ply's value is the reference's for its
    // mover there, and each ply's move gets that value; its plies counted
    fn assert_a_best_line<N: kwg::Node, L: kwg::Node>(
        fixed: &mut Reference<'_, N, L>,
        board: &[u8],
        racks: &[Vec<u8>; 2],
        mover: u8,
        line: &[(equity::Equity, movegen::Play)],
        what: &str,
    ) -> usize {
        let alphabet = fixed.gc.alphabet();
        let (mut board, mut racks) = (board.to_vec(), racks.clone());
        let (mut mover, mut passes, mut zeros) = (mover as usize, 0u8, 0u8);
        assert!(!line.is_empty(), "{what}: no line");
        for (ply, (value, play)) in line.iter().enumerate() {
            let want = reference(fixed, &board, &racks, mover, passes, zeros, false);
            assert_eq!(*value, super::to_equity(want), "{what}: ply {ply}'s value");
            let opp = mover ^ 1;
            let got = match play {
                movegen::Play::Place {
                    down,
                    lane,
                    idx,
                    word,
                    score,
                } => {
                    let placed = word.iter().filter(|&&tile| tile != 0).count();
                    let out = placed == racks[mover].len();
                    let got = if out {
                        (score.raw() + 2 * alphabet.scaled_rack_score(&racks[opp]).raw()) as f32
                    } else {
                        let mut next_board = board.clone();
                        let mut next_racks = racks.clone();
                        apply_place(
                            fixed.gc,
                            &mut next_board,
                            &mut next_racks[mover],
                            *down,
                            *lane,
                            *idx,
                            word,
                        );
                        let got = score.raw() as f32
                            - reference(fixed, &next_board, &next_racks, opp, 0, 0, false);
                        (board, racks) = (next_board, next_racks);
                        got
                    };
                    (passes, zeros) = (0, 0);
                    got
                }
                movegen::Play::Exchange { .. } => {
                    let (pass_passes, pass_zeros) = (passes + 1, zeros + 1);
                    let got = if super::scoreless_turns_end(
                        fixed.gc,
                        Some(&board),
                        pass_passes,
                        pass_zeros,
                    ) {
                        (alphabet.scaled_rack_score(&racks[opp]).raw()
                            - alphabet.scaled_rack_score(&racks[mover]).raw())
                            as f32
                    } else {
                        -reference(fixed, &board, &racks, opp, pass_passes, pass_zeros, false)
                    };
                    (passes, zeros) = (pass_passes, pass_zeros);
                    got
                }
            };
            assert_eq!(
                super::to_equity(got),
                super::to_equity(want),
                "{what}: ply {ply}'s move"
            );
            mover = opp;
        }
        line.len()
    }

    #[test]
    #[inline]
    fn a_solve_skipping_by_plays_out_answers_as_the_reference_does() {
        let kwg = kwg::kbwg_from_bytes_alloc(tiny_kwg_bytes().to_vec(), false).unwrap();
        let klv = klv::kbwg_from_bytes_alloc(klv::EMPTY_KLV_BYTES.to_vec()).unwrap();
        let mut positions = all_positions();
        // AA across row 6 from column 9, AAT against BH
        let mut aa = empty_board();
        put_word(&mut aa, 6, 9, &[1, 1]);
        positions.push((
            "AA; AAT vs BH".to_string(),
            Position {
                board: aa,
                racks: [vec![1, 1, 20], vec![2, 8]],
            },
        ));
        let mut checks = 0u64;
        // positions in which a solve skips moves
        for (rules, gc, name, movers) in [
            (
                "classic",
                game_config::make_english_game_config(),
                "rand#0",
                &[0u8, 1][..],
            ),
            (
                "classic",
                game_config::make_english_game_config(),
                "AA; AAT vs BH",
                &[1][..],
            ),
            (
                "super",
                game_config::make_super_english_game_config(),
                "rand#56",
                &[0][..],
            ),
            (
                "super",
                game_config::make_super_english_game_config(),
                "rand#87",
                &[0][..],
            ),
        ] {
            let cols = gc.board_layout().dim().cols as usize;
            let rows = gc.board_layout().dim().rows as usize;
            let mut mg = movegen::KurniaMoveGenerator::new(&gc);
            let mut fixed = Reference {
                gc: &gc,
                kwg: &kwg,
                klv: &klv,
                mg: &mut mg,
                memo: Default::default(),
            };
            let arm_tables = anagrams::ArmTables::build(&kwg, &gc, false).unwrap();
            let (_, pos) = positions.iter().find(|(n, _)| n == name).unwrap();
            let mut board = vec![0u8; rows * cols];
            for r in 0..15 {
                board[r * cols..r * cols + 15].copy_from_slice(&pos.board[r * 15..r * 15 + 15]);
            }
            let racks = [&pos.racks[0][..], &pos.racks[1][..]];
            let mut skipped = 0u64;
            for &mover in movers {
                let what = format!("[{rules} {name}] mover {mover}");
                let mut egs =
                    EndgameSolver::<kwg::Node24, kwg::Node24>::new(&gc, &kwg, &arm_tables).unwrap();
                egs.init(&board, racks);
                let value = egs.solve(mover).unwrap();
                let mut line = Vec::new();
                egs.collect_pv(mover, &mut line).unwrap();
                skipped += egs.work_buffer.out_skips;
                checks += u64::from(egs.work_buffer.out_checks);
                let want = reference(&mut fixed, &board, &pos.racks, mover as usize, 0, 0, false);
                assert_eq!(value, super::to_equity(want), "{what}: value");
                assert_a_best_line(&mut fixed, &board, &pos.racks, mover, &line, &what);
            }
            assert!(skipped > 0, "[{rules} {name}]: no move skipped");
        }
        debug_assert!(checks > 0);
    }

    #[test]
    #[inline]
    fn a_woadwage_solve_answers_as_the_reference_does() {
        use crate::movegen::tests::{
            WoadwageToyRules, woadwage_rng, woadwage_toy_boards, woadwage_toy_rack,
            woadwage_toy_rules,
        };
        let klv = klv::kbwg_from_bytes_alloc(klv::EMPTY_KLV_BYTES.to_vec()).unwrap();
        let mut seed = 403u64;
        let mut solved = 0usize;
        let mut lines = 0usize;
        for jumbled in [false, true] {
            for side in [5i8, 7] {
                let WoadwageToyRules(gc, kwg, explicit) = woadwage_toy_rules(side, jumbled);
                let arm_tables = anagrams::ArmTables::build(&kwg, &gc, false).unwrap();
                let mut mg = movegen::KurniaMoveGenerator::new(&gc);
                let mut fixed = Reference {
                    gc: &gc,
                    kwg: &kwg,
                    klv: &klv,
                    mg: &mut mg,
                    memo: Default::default(),
                };
                let boards = woadwage_toy_boards(side, &explicit, &mut seed, 3, 5);
                for board in boards.iter().filter(|board| board.iter().any(|&t| t != 0)) {
                    let racks = [
                        woadwage_toy_rack(&mut seed, crate::movegen::tests::woadwage_toy_longest()),
                        woadwage_toy_rack(&mut seed, crate::movegen::tests::woadwage_toy_longest()),
                    ];
                    let mover = (woadwage_rng(&mut seed) % gc.num_players() as u64) as u8;
                    let what = format!("[{jumbled} {side}] {board:?} {racks:?} mover {mover}");
                    let want = reference(&mut fixed, board, &racks, mover as usize, 0, 0, false);
                    let mut egs =
                        EndgameSolver::<kwg::Node24, kwg::Node24>::new(&gc, &kwg, &arm_tables)
                            .unwrap();
                    egs.init(board, [&racks[0][..], &racks[1][..]]);
                    assert_eq!(egs.solve(mover).unwrap(), super::to_equity(want), "{what}");
                    let mut line = Vec::new();
                    egs.collect_pv(mover, &mut line).unwrap();
                    lines += line.len();
                    assert_a_best_line(&mut fixed, board, &racks, mover, &line, &what);
                    let mut lean = EndgameSolver::<kwg::Node24, kwg::Node24, true>::new(
                        &gc,
                        &kwg,
                        &arm_tables,
                    )
                    .unwrap();
                    lean.init(board, [&racks[0][..], &racks[1][..]]);
                    assert_eq!(lean.solve(mover).unwrap(), super::to_equity(want), "{what}");
                    solved += 1;
                }
            }
        }
        assert!(solved > 0, "{solved}");
        assert!(lines > 0, "{lines}");
    }

    #[test]
    #[inline]
    fn a_woadwage_graph_pruned_to_the_words_the_board_can_take_solves_as_the_whole() {
        use crate::movegen::tests::{
            WoadwageToyRules, woadwage_rng, woadwage_toy_boards, woadwage_toy_rack,
            woadwage_toy_rules, woadwage_within_bag,
        };
        let klv = klv::kbwg_from_bytes_alloc(klv::EMPTY_KLV_BYTES.to_vec()).unwrap();
        let mut seed = 409u64;
        let mut solved = 0usize;
        for jumbled in [false, true] {
            for side in [5i8, 7] {
                let WoadwageToyRules(gc, whole, explicit) = woadwage_toy_rules(side, jumbled);
                let boards = woadwage_toy_boards(side, &explicit, &mut seed, 10, 5);
                for board in (boards.iter()).filter(|board| {
                    board.iter().any(|&t| t != 0) && woadwage_within_bag(&gc, board)
                }) {
                    let mut kept = Vec::<Vec<u8>>::new();
                    movegen::KurniaMoveGenerator::new(&gc)
                        .gen_remaining_words(
                            &movegen::BoardSnapshot {
                                board_tiles: board,
                                game_config: &gc,
                                kwg: &whole,
                                anagrams: None,
                                rack_lengths: None,
                                klv: &klv,
                            },
                            |word: &[u8]| kept.push(word.to_vec()),
                        )
                        .unwrap();
                    let pruned = if jumbled {
                        crate::jumbled_woadwage::tests::kad_for(&kept)
                    } else {
                        kwg::kbwg_from_bytes_alloc(
                            build::build(
                                build::BuildContent::Gaddawg,
                                build::BuildLayout::Wolges,
                                build::BuildOrder::Sorted,
                                &(kept.iter())
                                    .map(|word| bites::Bites::from(&word[..]))
                                    .collect::<Vec<_>>(),
                            )
                            .unwrap()
                            .to_vec(),
                            false,
                        )
                        .unwrap()
                    };
                    let racks = [
                        woadwage_toy_rack(&mut seed, crate::movegen::tests::woadwage_toy_longest()),
                        woadwage_toy_rack(&mut seed, crate::movegen::tests::woadwage_toy_longest()),
                    ];
                    let with_racks = (board.iter().copied())
                        .chain(
                            (racks.iter().flatten())
                                .map(|&tile| if tile == 0 { 0x80 } else { tile }),
                        )
                        .collect::<Vec<_>>();
                    if !woadwage_within_bag(&gc, &with_racks) {
                        continue;
                    }
                    let mover = (woadwage_rng(&mut seed) % gc.num_players() as u64) as u8;
                    let solve = |kwg: &kwg::Kwg<kwg::Node24>| {
                        let arm_tables = anagrams::ArmTables::build(kwg, &gc, false).unwrap();
                        let mut egs =
                            EndgameSolver::<kwg::Node24, kwg::Node24>::new(&gc, kwg, &arm_tables)
                                .unwrap();
                        egs.init(board, [&racks[0][..], &racks[1][..]]);
                        egs.solve(mover).unwrap()
                    };
                    assert_eq!(
                        solve(&pruned),
                        solve(&whole),
                        "[{jumbled} {side}] {board:?} {racks:?} mover {mover}"
                    );
                    solved += 1;
                }
            }
        }
        assert!(solved > 0, "{solved}");
    }

    #[test]
    #[inline]
    fn a_solve_in_steps_skipping_by_plays_out_ends_as_one_unbroken_solve() {
        let gc = game_config::make_english_game_config();
        let kwg_bytes = tiny_kwg_bytes();
        let kwg = kwg::kbwg_from_bytes_alloc(kwg_bytes.to_vec(), false).unwrap();
        let klv = klv::kbwg_from_bytes_alloc(klv::EMPTY_KLV_BYTES.to_vec()).unwrap();
        let arm_tables = anagrams::ArmTables::build(&kwg, &gc, false).unwrap();
        let mut mg = movegen::KurniaMoveGenerator::new(&gc);
        let mut fixed = Reference {
            gc: &gc,
            kwg: &kwg,
            klv: &klv,
            mg: &mut mg,
            memo: Default::default(),
        };
        let (mut resumed, mut skipped, mut stepped, mut same_line) = (0u64, 0u64, 0usize, 0usize);
        for (name, pos) in named_positions(&["rand#0", "rand#7"]) {
            let racks = [&pos.racks[0][..], &pos.racks[1][..]];
            for mover in 0..2u8 {
                let what = format!("[{name}] mover {mover}");
                let mut whole =
                    EndgameSolver::<kwg::Node24, kwg::Node24>::new(&gc, &kwg, &arm_tables).unwrap();
                whole.init(&pos.board, racks);
                let want = whole.solve(mover).unwrap();
                let mut want_line = Vec::new();
                whole.collect_pv(mover, &mut want_line).unwrap();
                let mut table = None;
                let mut finished = None;
                for step in 0.. {
                    assert!(step < 100_000, "{what}: no end");
                    let mut solver =
                        EndgameSolver::<kwg::Node24, kwg::Node24>::new(&gc, &kwg, &arm_tables)
                            .unwrap();
                    if let Some(table) = table.take() {
                        assert!(solver.give_table(table).is_ok());
                    }
                    solver.init(&pos.board, racks);
                    let limits = super::Limits {
                        states: Some(solver.work_buffer.table.held + 7),
                        ..super::Limits::NONE
                    };
                    let solved = match finished {
                        Some(finished) => solver.solve_within_after(mover, &limits, finished),
                        None => solver.solve_within(mover, &limits),
                    }
                    .unwrap();
                    resumed += solver.work_buffer.resumed;
                    skipped += solver.work_buffer.out_skips;
                    let mut line = Vec::new();
                    solver.collect_pv(mover, &mut line).unwrap();
                    table = Some(solver.take_table());
                    finished = Some((solved.depth, solved.value.unwrap()));
                    stepped += 1;
                    if solved.exact {
                        assert_eq!(solved.value.unwrap(), want, "{what}");
                        assert_eq!(line.first().map(|ply| ply.0), Some(want), "{what}");
                        assert_a_best_line(&mut fixed, &pos.board, &pos.racks, mover, &line, &what);
                        same_line += usize::from(line == want_line);
                        break;
                    }
                }
            }
        }
        println!(
            "{stepped} steps, {resumed} passes resumed, {skipped} moves skipped, {same_line} lines as the whole solve's"
        );
        assert!(resumed > 0);
        assert!(skipped > 0);
    }

    #[test]
    #[inline]
    fn a_timed_solve_on_a_slow_clock_looks_at_every_node() {
        // a clock moving twice LOOK_EVERY at each read: each look finds more
        // than LOOK_EVERY gone, so the looks come twice as often each time, and
        // the next step's solver, given the table, looks from its first node on
        // and stops a few nodes in, past its limit of sixteen LOOK_EVERY (a
        // look every LIMIT_CADENCE nodes would search 64 first)
        static READS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        fn slow() -> std::time::Duration {
            let reads = READS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            super::LOOK_EVERY * 2 * reads as u32
        }
        let gc = game_config::make_english_game_config();
        let kwg_bytes = tiny_kwg_bytes();
        let kwg = kwg::kbwg_from_bytes_alloc(kwg_bytes.to_vec(), false).unwrap();
        let arm_tables = anagrams::ArmTables::build(&kwg, &gc, false).unwrap();
        let limits = super::Limits {
            time: Some(super::LOOK_EVERY * 16),
            clock: Some(slow),
            ..super::Limits::NONE
        };
        let mut stopped = 0usize;
        for (name, pos) in all_positions() {
            for mover in 0..2u8 {
                let mut table = None;
                for step in 0..2 {
                    let mut solver =
                        EndgameSolver::<kwg::Node24, kwg::Node24>::new(&gc, &kwg, &arm_tables)
                            .unwrap();
                    if let Some(table) = table.take() {
                        assert!(solver.give_table(table).is_ok());
                    }
                    solver.init(&pos.board, [&pos.racks[0][..], &pos.racks[1][..]]);
                    let solved = solver.solve_within(mover, &limits).unwrap();
                    if step == 1 && !solved.exact {
                        let nodes = solver.work_buffer.nodes;
                        assert!(nodes <= 16, "[{name}] mover {mover}: {nodes} nodes");
                        stopped += 1;
                    }
                    table = Some(solver.take_table());
                }
            }
        }
        assert!(stopped > 0);
    }

    #[test]
    #[inline]
    fn a_lean_solver_in_u32_solves_as_the_default_one() {
        // the wasm's solver, generating by its move generator (in u32, never
        // jumping, each side its own copy), values every position as the
        // default one does
        let gc = game_config::make_english_game_config();
        let kwg_bytes = tiny_kwg_bytes();
        let kwg = kwg::kbwg_from_bytes_alloc(kwg_bytes.to_vec(), false).unwrap();
        let arm_tables = anagrams::ArmTables::None;
        let mut wide =
            EndgameSolver::<kwg::Node24, kwg::Node24, true>::new(&gc, &kwg, &arm_tables).unwrap();
        let mut narrow =
            EndgameSolver::<kwg::Node24, kwg::Node24, true, u32, false, false>::with_generator(
                &gc,
                &kwg,
                &arm_tables,
                kwg.run_letters(),
                movegen::KurniaMoveGenerator::try_new(&gc).unwrap(),
            )
            .unwrap();
        let mut solved = 0usize;
        for (name, pos) in all_positions() {
            for mover in 0..2u8 {
                wide.init(&pos.board, [&pos.racks[0][..], &pos.racks[1][..]]);
                narrow.init(&pos.board, [&pos.racks[0][..], &pos.racks[1][..]]);
                assert_eq!(
                    narrow.solve(mover).unwrap(),
                    wide.solve(mover).unwrap(),
                    "[{name}] mover {mover}"
                );
                solved += 1;
            }
        }
        assert!(solved > 0);
    }

    #[test]
    fn selected_first_moves_keep_their_own_table_answer() {
        let gc = game_config::make_english_game_config();
        let kwg = kwg::kbwg_from_bytes_alloc(tiny_kwg_bytes().to_vec(), false).unwrap();
        let klv = klv::kbwg_from_bytes_alloc(klv::EMPTY_KLV_BYTES.to_vec()).unwrap();
        let tables = anagrams::ArmTables::build(&kwg, &gc, false).unwrap();
        let mut state = crate::game_state::GameState::new(&gc);
        state.bag.set_from_iter([]);
        state.players[0].rack.extend([1, 2]); // AB
        state.players[1].rack.push(8); // H
        let make = || {
            let mut solver =
                EndgameSolver::<kwg::Node24, kwg::Node24>::new(&gc, &kwg, &tables).unwrap();
            solver.init(
                &state.board_tiles,
                [&state.players[0].rack, &state.players[1].rack],
            );
            solver
        };
        let mut full = make();
        let best = full.solve(0).unwrap();
        let first = full.value_first_moves(0, &super::Limits::NONE).unwrap();
        let pass = first
            .iter()
            .find(|p| matches!(&p.play, movegen::Play::Exchange { tiles } if tiles.is_empty()))
            .unwrap();
        assert!(pass.value.unwrap() < best);
        let root = super::RootMoves::checked(
            &movegen::BoardSnapshot {
                board_tiles: &state.board_tiles,
                game_config: &gc,
                kwg: &kwg,
                anagrams: None,
                rack_lengths: None,
                klv: &klv,
            },
            &state,
            vec![pass.play.clone()],
        )
        .unwrap();
        let mut restricted = make().with_root_moves(Some(&root));
        assert!(restricted.give_table(full.take_table()).is_ok());
        assert_eq!(restricted.solve(0).unwrap(), pass.value.unwrap());
        let mut line = Vec::new();
        restricted.collect_pv(0, &mut line).unwrap();
        assert!(line.first().is_some_and(|(_, play)| play == &pass.play));
        let selected = restricted
            .value_first_moves(0, &super::Limits::NONE)
            .unwrap();
        assert_eq!(selected.len(), 1);
        assert_eq!(selected[0].value, pass.value);
        assert!(full.give_table(restricted.take_table()).is_ok());
        assert_eq!(full.solve(0).unwrap(), best);
    }

    #[test]
    fn root_plays_are_the_first_moves() {
        let gc = game_config::make_english_game_config();
        let kwg = kwg::kbwg_from_bytes_alloc(tiny_kwg_bytes().to_vec(), false).unwrap();
        let klv = klv::kbwg_from_bytes_alloc(klv::EMPTY_KLV_BYTES.to_vec()).unwrap();
        let tables = anagrams::ArmTables::build(&kwg, &gc, false).unwrap();
        let mut state = crate::game_state::GameState::new(&gc);
        state.bag.set_from_iter([]);
        state.players[0].rack.extend([1, 2]); // AB
        state.players[1].rack.push(8); // H
        let mut solver =
            EndgameSolver::<kwg::Node24, kwg::Node24>::new(&gc, &kwg, &tables).unwrap();
        solver.init(
            &state.board_tiles,
            [&state.players[0].rack, &state.players[1].rack],
        );
        let first = (solver.value_first_moves(0, &super::Limits::NONE).unwrap())
            .into_iter()
            .map(|first| first.play)
            .collect::<Vec<_>>();
        let plays = solver.root_plays(0).unwrap().unwrap();
        assert!(
            matches!(plays.last(), Some(movegen::Play::Exchange { tiles }) if tiles.is_empty())
        );
        assert!(plays.len() >= 2 && plays.len() == first.len());
        assert!(plays.iter().all(|play| first.contains(play)));
        let root = super::RootMoves::checked(
            &movegen::BoardSnapshot {
                board_tiles: &state.board_tiles,
                game_config: &gc,
                kwg: &kwg,
                anagrams: None,
                rack_lengths: None,
                klv: &klv,
            },
            &state,
            vec![plays[0].clone()],
        )
        .unwrap();
        let mut restricted =
            EndgameSolver::<kwg::Node24, kwg::Node24>::new(&gc, &kwg, &tables).unwrap();
        restricted.init(
            &state.board_tiles,
            [&state.players[0].rack, &state.players[1].rack],
        );
        let mut restricted = restricted.with_root_moves(Some(&root));
        assert!(restricted.root_plays(0).unwrap().unwrap() == plays[..1]);
    }

    #[test]
    fn selected_first_moves_refuse_a_play_named_twice() {
        let gc = game_config::make_english_game_config();
        let kwg = kwg::kbwg_from_bytes_alloc(tiny_kwg_bytes().to_vec(), false).unwrap();
        let klv = klv::kbwg_from_bytes_alloc(klv::EMPTY_KLV_BYTES.to_vec()).unwrap();
        let tables = anagrams::ArmTables::build(&kwg, &gc, false).unwrap();
        let mut state = crate::game_state::GameState::new(&gc);
        state.bag.set_from_iter([]);
        state.players[0].rack.extend([1, 2]); // AB
        state.players[1].rack.push(8); // H
        let mut solver =
            EndgameSolver::<kwg::Node24, kwg::Node24>::new(&gc, &kwg, &tables).unwrap();
        solver.init(
            &state.board_tiles,
            [&state.players[0].rack, &state.players[1].rack],
        );
        let plays = (solver.value_first_moves(0, &super::Limits::NONE).unwrap())
            .into_iter()
            .map(|first| first.play)
            .collect::<Vec<_>>();
        assert!(plays.len() >= 2);
        let snapshot = movegen::BoardSnapshot {
            board_tiles: &state.board_tiles,
            game_config: &gc,
            kwg: &kwg,
            anagrams: None,
            rack_lengths: None,
            klv: &klv,
        };
        assert!(super::RootMoves::checked(&snapshot, &state, plays.clone()).is_ok());
        for play in &plays {
            let mut named = plays.clone();
            named.push(play.clone());
            let error = super::RootMoves::checked(&snapshot, &state, named)
                .err()
                .unwrap();
            assert_eq!(
                error.to_string(),
                format!("root_moves[{}]: duplicate play", plays.len())
            );
        }
    }

    #[test]
    #[inline]
    fn order_first_agrees_with_order() {
        // order_first picks the move order sorts first, ties included, and
        // gives it index 0
        let gc = game_config::make_english_game_config();
        let kwg_bytes = tiny_kwg_bytes();
        let kwg = kwg::kbwg_from_bytes_alloc(kwg_bytes.to_vec(), false).unwrap();
        let arm_tables = anagrams::ArmTables::build(&kwg, &gc, false).unwrap();
        let mut egs =
            EndgameSolver::<kwg::Node24, kwg::Node24>::new(&gc, &kwg, &arm_tables).unwrap();
        let mut firsts = 0usize;
        let mut tied = 0usize;
        for (name, pos) in all_positions() {
            for mover in 0..2u8 {
                egs.init(&pos.board, [&pos.racks[0][..], &pos.racks[1][..]]);
                egs.reset_position();
                egs.generate(0, mover, false).unwrap();
                let mut keys = egs.work_buffer.orders[0].clone();
                super::order_first(&mut egs.work_buffer.lists[0], &mut keys);
                let Some(&key) = keys.first() else {
                    continue;
                };
                let first = egs.work_buffer.lists[0][ordered_play_index(key)]
                    .play
                    .clone();
                assert_eq!(
                    egs.work_buffer.lists[0][ordered_play_index(key)].index,
                    0,
                    "[{name}] the first move's index"
                );
                if keys.iter().filter(|&&k| k >> 32 == key >> 32).count() > 1 {
                    tied += 1;
                }
                order_ply(&mut egs, 0, super::NO_PLAY);
                let whole = egs.work_buffer.orders[0][0];
                assert!(
                    first == egs.work_buffer.lists[0][ordered_play_index(whole)].play,
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
        let kwg = kwg::kbwg_from_bytes_alloc(kwg_bytes.to_vec(), false).unwrap();
        let arm_tables = anagrams::ArmTables::build(&kwg, &gc, false).unwrap();
        let mut egs =
            EndgameSolver::<kwg::Node24, kwg::Node24>::new(&gc, &kwg, &arm_tables).unwrap();
        let mut ties = 0usize;
        for (name, pos) in all_positions() {
            for mover in 0..2u8 {
                egs.init(&pos.board, [&pos.racks[0][..], &pos.racks[1][..]]);
                egs.reset_position();
                egs.generate(0, mover, false).unwrap();
                order_ply(&mut egs, 0, super::NO_PLAY);
                let list = std::mem::take(&mut egs.work_buffer.lists[0]);
                let ordered = std::mem::take(&mut egs.work_buffer.orders[0]);
                let moves = ordered
                    .iter()
                    .map(|&key| &list[ordered_play_index(key)])
                    .collect::<Vec<_>>();
                for (i, candidate) in moves.iter().enumerate() {
                    assert_eq!(
                        candidate.index as usize, i,
                        "[{name}] an index is its place"
                    );
                }
                for pair in moves.windows(2) {
                    let by_valuation = pair[1].valuation.cmp(&pair[0].valuation);
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
                            uses: candidate.uses,
                            index: super::NO_PLAY,
                            valuation: candidate.valuation,
                            plays_out: candidate.plays_out,
                            placed: candidate.placed,
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
                            super::order_key(
                                candidate.valuation.raw(),
                                down,
                                lane,
                                idx,
                                u32::try_from(found).unwrap(),
                            )
                        })
                        .collect::<Vec<_>>();
                    super::order(&mut again, &mut keys, 0, super::NO_PLAY);
                    assert!(
                        keys.iter()
                            .map(|&key| &again[ordered_play_index(key)])
                            .zip(&moves)
                            .all(|(a, b)| a.play == b.play && a.index == b.index),
                        "[{name}] the same order however generated"
                    );
                }
                if moves.len() >= 2 {
                    let table_play = (moves.len() / 2) as u16;
                    egs.generate(0, mover, false).unwrap();
                    order_ply(&mut egs, 0, table_play);
                    let again = &egs.work_buffer.lists[0];
                    let first = egs.work_buffer.orders[0]
                        .iter()
                        .map(|&key| &again[ordered_play_index(key)])
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
    fn reused_lists_equal_a_full_generation_along_random_lines() {
        // over random play from each position, every reused list (a reuse of a
        // reuse too) matches generating from scratch, order included, on both
        // arms
        let gc = game_config::make_english_game_config();
        let kwg_bytes = tiny_kwg_bytes();
        let kwg = kwg::kbwg_from_bytes_alloc(kwg_bytes.to_vec(), false).unwrap();
        // deep enough for most lines to keep rack places to their end
        const DEPTH: usize = 12;
        let mut seed = 0u64;
        let mut checked = 0usize;
        let mut chained = 0usize;
        let in_order = |solver: &EndgameSolver<kwg::Node24, kwg::Node24>, ply: usize| {
            solver.work_buffer.orders[ply]
                .iter()
                .map(|&key| &solver.work_buffer.lists[ply][ordered_play_index(key)])
                .map(|c| (c.play.clone(), c.index, c.valuation, c.plays_out))
                .collect::<Vec<_>>()
        };
        for (lean, arm_tables) in &both_arms(&kwg, &gc) {
            let mut egs =
                EndgameSolver::<kwg::Node24, kwg::Node24>::new(&gc, &kwg, arm_tables).unwrap();
            let mut afresh =
                EndgameSolver::<kwg::Node24, kwg::Node24>::new(&gc, &kwg, arm_tables).unwrap();
            for (name, pos) in all_positions() {
                for mover in 0..2u8 {
                    for _ in 0..16 {
                        egs.init(&pos.board, [&pos.racks[0][..], &pos.racks[1][..]]);
                        egs.reset_position();
                        let mut line = Vec::new();
                        let mut made = [false; DEPTH];
                        let mut player = mover;
                        let mut passes = 0;
                        for ply in 0..DEPTH {
                            let depth = DEPTH - ply;
                            egs.generate(ply, player, depth >= 3).unwrap();
                            order_ply(&mut egs, ply, super::NO_PLAY);
                            if ply >= 2 && egs.work_buffer.keeps[ply - 2] {
                                afresh.init(
                                    &egs.work_buffer.board_tiles,
                                    [&egs.work_buffer.racks[0][..], &egs.work_buffer.racks[1][..]],
                                );
                                afresh.reset_position();
                                afresh.generate(0, player, false).unwrap();
                                order_ply(&mut afresh, 0, super::NO_PLAY);
                                assert!(
                                    in_order(&egs, ply) == in_order(&afresh, 0),
                                    "[{name}] lean={lean} the moves made at ply {ply}"
                                );
                                made[ply] = true;
                                checked += 1;
                                chained += usize::from(made[ply - 2]);
                            }
                            // a random move or pass, lanes and rack places set
                            // as the search sets them
                            let moves = egs.work_buffer.lists[ply].len();
                            let pick = (splitmix64(&mut seed) % (moves as u64 + 1)) as usize;
                            let play = (pick < moves)
                                .then(|| egs.work_buffer.lists[ply][pick].play.clone());
                            if depth >= 2 {
                                egs.work_buffer.zones[ply] =
                                    play.as_ref().map_or((0, 0), |play| egs.zone_lanes(play));
                                egs.work_buffer.spent[ply] = match &play {
                                    Some(movegen::Play::Place { word, .. }) => super::rack_places(
                                        &egs.work_buffer.racks[player as usize],
                                        word,
                                        true,
                                    ),
                                    _ => 0,
                                };
                            }
                            if let Some(play) = &play {
                                egs.place(player, play);
                                passes = 0;
                            } else {
                                passes += 1;
                            }
                            line.push((player, play));
                            // a play out or a second pass ends the line
                            if egs.work_buffer.racks[player as usize].is_empty() || passes == 2 {
                                break;
                            }
                            player ^= 1;
                        }
                        for (player, play) in line.iter().rev() {
                            if let Some(play) = play {
                                egs.unplace(*player, play);
                            }
                        }
                    }
                }
            }
        }
        println!("along random lines: {checked} reused lists, {chained} reused from a reused one");
        assert!(checked > 0 && chained > 0);
    }

    #[test]
    #[inline]
    fn every_table_copies_the_same_lanes() {
        // no table read beside the word graph turns the lane copy off: with the
        // anagrams, the rack lengths, or neither, each with and without the run
        // letters, after each move and reply (a pass among them) a lane the
        // reply leaves alone is copied when an earlier reply to the same list
        // walked it, as many lanes as the moves' zones alone say, the same
        // count under every table, each list equal to a full generation; and
        // full solves copy lanes under every table, valuing every position the
        // same
        let gc = game_config::make_english_game_config();
        let kwg_bytes = tiny_kwg_bytes();
        let kwg = kwg::kbwg_from_bytes_alloc(kwg_bytes.to_vec(), false).unwrap();
        let [(_, with_tables), (_, lean)] = both_arms(&kwg, &gc);
        let neither = anagrams::ArmTables::None;
        assert!(with_tables.anagrams().is_some() && lean.rack_lengths().is_some());
        let spent_by = |solver: &EndgameSolver<kwg::Node24, kwg::Node24>,
                        mover: u8,
                        play: &Option<movegen::Play>| match play {
            Some(movegen::Play::Place { word, .. }) => {
                super::rack_places(&solver.work_buffer.racks[mover as usize], word, true)
            }
            _ => 0,
        };
        let in_order = |solver: &EndgameSolver<kwg::Node24, kwg::Node24>, ply: usize| {
            solver.work_buffer.orders[ply]
                .iter()
                .map(|&key| &solver.work_buffer.lists[ply][ordered_play_index(key)])
                .map(|c| (c.play.clone(), c.index, c.valuation, c.plays_out))
                .collect::<Vec<_>>()
        };
        // each table and letters setting: the lanes copied along the lines
        // below, those copied by full solves, and the values solved
        let mut runs = Vec::new();
        for (tables, arm_tables) in [
            ("anagrams", &with_tables),
            ("rack lengths", &lean),
            ("neither", &neither),
        ] {
            for letters in [true, false] {
                let new_solver = || {
                    let mut solver =
                        EndgameSolver::<kwg::Node24, kwg::Node24>::new(&gc, &kwg, arm_tables)
                            .unwrap();
                    if !letters {
                        solver.work_buffer.run_letter_bits = None;
                    }
                    solver
                };
                let mut egs = new_solver();
                if letters && egs.work_buffer.run_letter_bits.is_none() {
                    // these tables read no run letters
                    continue;
                }
                let mut afresh = new_solver();
                let mut expected = 0usize;
                let positions = named_positions(&["rand#0"]);
                for (name, pos) in &positions {
                    for mover in 0..2u8 {
                        egs.init(&pos.board, [&pos.racks[0][..], &pos.racks[1][..]]);
                        egs.reset_position();
                        egs.generate(0, mover, true).unwrap();
                        let moves = egs.work_buffer.lists[0].len();
                        for i in 0..=moves {
                            let first =
                                (i < moves).then(|| egs.work_buffer.lists[0][i].play.clone());
                            egs.work_buffer.zones[0] =
                                first.as_ref().map_or((0, 0), |play| egs.zone_lanes(play));
                            egs.work_buffer.spent[0] = spent_by(&egs, mover, &first);
                            if let Some(play) = &first {
                                egs.place(mover, play);
                            }
                            egs.generate(1, mover ^ 1, true).unwrap();
                            let replies = egs.work_buffer.lists[1].len();
                            // a lane only the move changed is walked by the
                            // first reply that leaves it alone, then copied by
                            // each later one that does
                            let (rows, cols) = egs.work_buffer.zones[0];
                            let (mut walked_rows, mut walked_cols) = (0u128, 0u128);
                            for j in 0..=replies {
                                let reply =
                                    (j < replies).then(|| egs.work_buffer.lists[1][j].play.clone());
                                egs.work_buffer.zones[1] =
                                    reply.as_ref().map_or((0, 0), |play| egs.zone_lanes(play));
                                egs.work_buffer.spent[1] = spent_by(&egs, mover ^ 1, &reply);
                                if let Some(play) = &reply {
                                    egs.place(mover ^ 1, play);
                                }
                                let (replied_rows, replied_cols) = egs.work_buffer.zones[1];
                                let (only_rows, only_cols) =
                                    (rows & !replied_rows, cols & !replied_cols);
                                expected += ((only_rows & walked_rows).count_ones()
                                    + (only_cols & walked_cols).count_ones())
                                    as usize;
                                walked_rows |= only_rows;
                                walked_cols |= only_cols;
                                egs.generate(2, mover, false).unwrap();
                                order_ply(&mut egs, 2, super::NO_PLAY);
                                afresh.init(
                                    &egs.work_buffer.board_tiles,
                                    [&egs.work_buffer.racks[0][..], &egs.work_buffer.racks[1][..]],
                                );
                                afresh.reset_position();
                                afresh.generate(0, mover, false).unwrap();
                                order_ply(&mut afresh, 0, super::NO_PLAY);
                                assert!(
                                    in_order(&egs, 2) == in_order(&afresh, 0),
                                    "[{name}] {tables} letters={letters} the moves made from two \
                                     plies up"
                                );
                                if let Some(play) = &reply {
                                    egs.unplace(mover ^ 1, play);
                                }
                            }
                            if let Some(play) = &first {
                                egs.unplace(mover, play);
                            }
                        }
                    }
                }
                assert_eq!(
                    egs.work_buffer.copied, expected,
                    "{tables} letters={letters}: the lanes copied along the lines"
                );
                // full solves, the search choosing which lists keep rack places
                let mut solver = new_solver();
                let values = positions
                    .iter()
                    .map(|(_, pos)| {
                        solver.init(&pos.board, [&pos.racks[0][..], &pos.racks[1][..]]);
                        solver.solve(0).unwrap()
                    })
                    .collect::<Vec<_>>();
                runs.push((tables, letters, expected, solver.work_buffer.copied, values));
            }
        }
        // both letters settings were tried
        assert!(runs.iter().any(|run| run.1) && runs.iter().any(|run| !run.1));
        for (tables, letters, along_lines, in_solves, values) in &runs {
            println!(
                "{tables} letters={letters}: {along_lines} lanes copied along the lines, \
                 {in_solves} in full solves"
            );
            assert!(
                *along_lines > 0,
                "{tables} letters={letters}: no lane copied"
            );
            assert_eq!(
                *along_lines, runs[0].2,
                "{tables} letters={letters}: the lanes copied"
            );
            assert!(
                *in_solves > 0,
                "{tables} letters={letters}: no lane copied in full solves"
            );
            assert!(
                *values == runs[0].4,
                "{tables} letters={letters}: the values solved"
            );
        }
    }

    #[test]
    #[inline]
    fn one_tile_bound_equals_a_full_generation() {
        // after each move that brings its player to a last tile, one_tile_bound
        // is the better of both passing and a full generation's best play out,
        // on both arms; for some the best is on the zone's lanes
        let gc = game_config::make_english_game_config();
        let kwg_bytes = tiny_kwg_bytes();
        let kwg = kwg::kbwg_from_bytes_alloc(kwg_bytes.to_vec(), false).unwrap();
        let mut checked = 0usize;
        let mut zoned = 0usize;
        for (lean, arm_tables) in &both_arms(&kwg, &gc) {
            let mut egs =
                EndgameSolver::<kwg::Node24, kwg::Node24>::new(&gc, &kwg, arm_tables).unwrap();
            let mut afresh =
                EndgameSolver::<kwg::Node24, kwg::Node24>::new(&gc, &kwg, arm_tables).unwrap();
            for (name, pos) in all_positions() {
                for mover in 0..2u8 {
                    egs.init(&pos.board, [&pos.racks[0][..], &pos.racks[1][..]]);
                    egs.reset_position();
                    egs.generate(0, mover, false).unwrap();
                    let rack_len = egs.work_buffer.racks[mover as usize].len();
                    for i in 0..egs.work_buffer.lists[0].len() {
                        let play = egs.work_buffer.lists[0][i].play.clone();
                        let movegen::Play::Place { word, .. } = &play else {
                            continue;
                        };
                        if word.iter().filter(|&&t| t != 0).count() + 1 != rack_len {
                            continue;
                        }
                        egs.work_buffer.zones[0] = egs.zone_lanes(&play);
                        egs.place(mover, &play);
                        let bound = egs
                            .one_tile_bound(0, mover, equity::Equity::INFINITY)
                            .unwrap();
                        afresh.init(
                            &egs.work_buffer.board_tiles,
                            [&egs.work_buffer.racks[0][..], &egs.work_buffer.racks[1][..]],
                        );
                        afresh.reset_position();
                        afresh.generate(0, mover, false).unwrap();
                        let mut whole = afresh.both_pass_value(mover);
                        for candidate in &afresh.work_buffer.lists[0] {
                            assert!(candidate.plays_out, "[{name}] a single plays out");
                            whole = whole.max(candidate.valuation);
                        }
                        assert_eq!(
                            bound, whole,
                            "[{name}] lean={lean} the bound after move {i}"
                        );
                        let unreached = egs
                            .one_tile_bound(0, mover, equity::Equity::NEG_INFINITY)
                            .unwrap();
                        zoned += usize::from(unreached < whole);
                        checked += 1;
                        egs.unplace(mover, &play);
                    }
                }
            }
        }
        println!("one tile left: {checked} bounds, {zoned} with the best on the zone's lanes");
        assert!(checked > 0 && zoned > 0);
    }

    #[test]
    #[inline]
    fn differential_reference_vs_solve() {
        let gc = game_config::make_english_game_config();
        let kwg_bytes = tiny_kwg_bytes();
        let kwg = kwg::kbwg_from_bytes_alloc(kwg_bytes.to_vec(), false).unwrap();
        let klv = klv::kbwg_from_bytes_alloc(klv::EMPTY_KLV_BYTES.to_vec()).unwrap();
        let mut mg = movegen::KurniaMoveGenerator::new(&gc);
        let arms = both_arms(&kwg, &gc);

        let mut disagreements = 0usize;
        let mut total = 0usize;
        let mut fixed = Reference {
            gc: &gc,
            kwg: &kwg,
            klv: &klv,
            mg: &mut mg,
            memo: Default::default(),
        };
        // from no scoreless turns, and after a pass
        for (name, pos) in named_positions(&["rand#0", "rand#7"]) {
            for (passes, zeros) in [(0, 0), (1, 1)] {
                let refv = reference(
                    &mut fixed,
                    &pos.board,
                    &pos.racks,
                    0,
                    passes,
                    zeros,
                    passes > 0, // the game's last turn a pass
                );
                for (lean, arm_tables) in &arms {
                    total += 1;
                    let mut egs =
                        EndgameSolver::<kwg::Node24, kwg::Node24>::new(&gc, &kwg, arm_tables)
                            .unwrap();
                    egs.init_after_scoreless_turns(
                        &pos.board,
                        [&pos.racks[0][..], &pos.racks[1][..]],
                        u16::from(passes),
                        u16::from(zeros),
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

    // a late board: TA across row 3 from column 6, and, from row 8 on, every
    // other tile but the racks' (a blank as an A), which no play can touch,
    // their runs too long to be words; so the tiles unseen are the racks'
    fn late_board(gc: &game_config::GameConfig, racks: [&[u8]; 2]) -> Vec<u8> {
        let alphabet = gc.alphabet();
        let cols = gc.board_layout().dim().cols as usize;
        let rows = gc.board_layout().dim().rows as usize;
        let mut left = (0..alphabet.len())
            .map(|tile| alphabet.freq(tile))
            .collect::<Vec<_>>();
        let mut board = vec![0u8; rows * cols];
        for (col, tile) in [(6, 20u8), (7, 1)] {
            board[3 * cols + col] = tile;
            left[tile as usize] -= 1;
        }
        for &tile in racks.iter().copied().flatten() {
            left[tile as usize] -= 1;
        }
        let mut squares = board[8 * cols..].iter_mut();
        for (tile, &count) in left.iter().enumerate() {
            for _ in 0..count {
                *squares.next().unwrap() = if tile == 0 { 0x81 } else { tile as u8 };
            }
        }
        board
    }

    // the endgame command solves on a graph of the words the board can still
    // take, whatever the racks: on it the solver answers as on the whole graph,
    // under classic rules and under jumbled ones, whose words lie on the board
    // with their tiles in any order (so BATH through TA, from B and H)
    #[test]
    #[inline]
    fn a_graph_pruned_to_the_words_the_board_can_take_solves_as_the_whole() {
        let klv = klv::kbwg_from_bytes_alloc(klv::EMPTY_KLV_BYTES.to_vec()).unwrap();
        // a graph of these words, of alphagrams under jumbled rules
        let graph = |jumbled: bool, words: &[bites::Bites]| {
            kwg::kbwg_from_bytes_alloc(
                build::build(
                    if jumbled {
                        build::BuildContent::DawgOnly
                    } else {
                        build::BuildContent::Gaddawg
                    },
                    build::BuildLayout::Wolges,
                    build::BuildOrder::Sorted,
                    words,
                )
                .unwrap()
                .to_vec(),
                false,
            )
            .unwrap()
        };
        for (rules, gc, jumbled) in [
            ("classic", game_config::make_english_game_config(), false),
            (
                "jumbled",
                game_config::make_jumbled_english_game_config(),
                true,
            ),
        ] {
            let words = tiny_word_list();
            let words = if jumbled {
                build::make_alphagrams(&words).unwrap().into_vec()
            } else {
                words
            };
            let whole = graph(jumbled, &words);
            let mut pruned_away = 0;
            // a blank, A B H S T, each rack sorted
            for racks in [
                [&[2u8, 8][..], &[1, 1, 19]],
                [&[1, 2], &[8, 19]],
                [&[8, 19], &[1, 2]],
                [&[2, 8, 19], &[1, 1]],
                [&[0, 8], &[1, 2]],
                [&[8, 20], &[1, 2, 19]],
            ] {
                let board = late_board(&gc, racks);
                let mut found = fash::MyHashSet::<bites::Bites>::default();
                movegen::KurniaMoveGenerator::new(&gc)
                    .gen_remaining_words(
                        &movegen::BoardSnapshot {
                            board_tiles: &board,
                            game_config: &gc,
                            kwg: &whole,
                            anagrams: None,
                            rack_lengths: None,
                            klv: &klv,
                        },
                        |word: &[u8]| {
                            found.insert(word.into());
                        },
                    )
                    .unwrap();
                let mut found = found.into_iter().collect::<Vec<_>>();
                found.sort_unstable();
                let found = if jumbled {
                    build::make_alphagrams(&found).unwrap().into_vec()
                } else {
                    found
                };
                if found.len() < words.len() {
                    pruned_away += 1;
                }
                let pruned = graph(jumbled, &found);
                let solve = |kwg: &kwg::Kwg<kwg::Node24>| {
                    let arm_tables = anagrams::ArmTables::build(kwg, &gc, false).unwrap();
                    let mut egs =
                        EndgameSolver::<kwg::Node24, kwg::Node24>::new(&gc, kwg, &arm_tables)
                            .unwrap();
                    egs.init(&board, racks);
                    egs.solve(0).unwrap()
                };
                assert_eq!(solve(&pruned), solve(&whole), "[{rules} {racks:?}]");
            }
            assert!(pruned_away > 0, "{rules}: no graph lost a word");
        }
    }

    // the commonest letters, A, E, I, N, O, R, and T
    const CROSSING_LETTERS: [u8; 7] = [1, 5, 9, 14, 15, 18, 20];

    // a word list of strings of the commonest letters, two to four long, about
    // one in four of them chosen at random, so a board of random moves crosses
    // itself often and its lines take few of the letters
    #[inline]
    fn crossing_word_list() -> Vec<bites::Bites> {
        let letters = CROSSING_LETTERS;
        let mut seed = 0u64;
        let mut words = Vec::<bites::Bites>::new();
        for len in 2..=4 {
            for index in 0..letters.len().pow(len) {
                if splitmix64(&mut seed).is_multiple_of(4) {
                    let word = (0..len)
                        .map(|place| letters[index / letters.len().pow(place) % letters.len()])
                        .collect::<Vec<_>>();
                    words.push(word[..].into());
                }
            }
        }
        words.sort_unstable();
        words
    }

    // a move placing tiles: down or across, its lane, its first square, and its
    // tiles, 0 where it plays through one on the board
    type PlayAt<'w> = (bool, i8, i8, &'w [u8]);

    // the words a move made, read on the board after it: its own along its line
    // and each across it through a tile it placed, of two letters or more
    fn words_made(
        gc: &game_config::GameConfig,
        board: &[u8],
        (down, lane, idx, word): PlayAt<'_>,
    ) -> Vec<Vec<u8>> {
        let dim = gc.board_layout().dim();
        let run = |down: bool, lane: i8, at: i8| {
            let strider = dim.lane(down, lane);
            let mut first = at;
            while first > 0 && board[strider.at_in_lane(first - 1)] != 0 {
                first -= 1;
            }
            let mut last = at;
            while last + 1 < strider.len() && board[strider.at_in_lane(last + 1)] != 0 {
                last += 1;
            }
            (first..=last)
                .map(|i| board[strider.at_in_lane(i)] & 0x7f)
                .collect::<Vec<_>>()
        };
        let mut made = vec![run(down, lane, idx)];
        for (i, &tile) in (idx..).zip(word) {
            if tile != 0 {
                made.push(run(!down, i, lane));
            }
        }
        made.retain(|word| word.len() >= 2);
        made
    }

    // the word prune keeps every word a later move can make, however the tiles
    // not on the board come to the racks: on boards of random moves, crowded
    // now and then by tiles of letters no word holds, and along lines of random
    // moves from each, every word that every move of each rack drawn from the
    // tiles left makes (its own and each across it) is one the prune of the
    // line's first board keeps; and the prune gives each word once, in
    // ascending order
    #[test]
    #[inline]
    fn the_word_prune_keeps_every_word_a_later_move_makes() {
        let gc = game_config::make_english_game_config();
        let alphabet = gc.alphabet();
        let dim = gc.board_layout().dim();
        let rack_size = gc.rack_size() as usize;
        let kwg = kwg::kbwg_from_bytes_alloc(
            build::build(
                build::BuildContent::Gaddawg,
                build::BuildLayout::Wolges,
                build::BuildOrder::Sorted,
                &crossing_word_list(),
            )
            .unwrap()
            .to_vec(),
            false,
        )
        .unwrap();
        let klv = klv::kbwg_from_bytes_alloc(klv::EMPTY_KLV_BYTES.to_vec()).unwrap();
        let mut mg = movegen::KurniaMoveGenerator::new(&gc);
        let mut seed = 1u64;
        let mut random = move |n: usize| (splitmix64(&mut seed) % n as u64) as usize;
        // a rack drawn from the tiles left, its every move, and the board after
        // one of them chosen at random with the rack's other tiles put back;
        // None when the rack has no move
        let mut step = |board: &mut Vec<u8>,
                        left: &mut Vec<u8>,
                        each_move: &mut dyn FnMut(&[u8], PlayAt<'_>)|
         -> Option<()> {
            let mut rack = Vec::new();
            while rack.len() < rack_size && !left.is_empty() {
                rack.push(left.swap_remove(random(left.len())));
            }
            mg.gen_moves_unfiltered(&movegen::GenMovesParams {
                board_snapshot: &movegen::BoardSnapshot {
                    board_tiles: board,
                    game_config: &gc,
                    kwg: &kwg,
                    anagrams: None,
                    rack_lengths: None,
                    klv: &klv,
                },
                rack: &rack,
                max_gen: usize::MAX,
                num_exchanges_by_this_player: 0,
                pass_policy: movegen::PassPolicy::OnlyWhenForced,
                dynamic_leaves: None,
            })
            .unwrap();
            let places = (mg.plays.iter())
                .filter_map(|valued| match &valued.play {
                    movegen::Play::Place {
                        down,
                        lane,
                        idx,
                        word,
                        ..
                    } => Some((*down, *lane, *idx, word.clone())),
                    movegen::Play::Exchange { .. } => None,
                })
                .collect::<Vec<_>>();
            for (down, lane, idx, word) in &places {
                let mut after = board.clone();
                apply_place(&gc, &mut after, &mut rack.clone(), *down, *lane, *idx, word);
                each_move(&after, (*down, *lane, *idx, &word[..]));
            }
            let chosen = (!places.is_empty()).then(|| {
                let (down, lane, idx, word) = &places[random(places.len())];
                apply_place(&gc, board, &mut rack, *down, *lane, *idx, word);
            });
            left.extend(rack);
            chosen
        };
        let mut scatter = 2u64;
        let (mut words_checked, mut words_kept) = (0, 0);
        for _ in 0..10 {
            let mut board = vec![0u8; dim.rows as usize * dim.cols as usize];
            let mut left = (0..alphabet.len())
                .flat_map(|tile| std::iter::repeat_n(tile, alphabet.freq(tile) as usize))
                .collect::<Vec<_>>();
            for turn in 0..30 {
                // after a few moves, tiles of letters no word holds put on
                // random empty squares: a crowded board, the squares beside
                // them taking nothing
                for _ in 0..if turn % 4 == 0 && turn > 0 {
                    splitmix64(&mut scatter) % 31
                } else {
                    0
                } {
                    let blockers = (0..left.len())
                        .filter(|&at| left[at] != 0 && !CROSSING_LETTERS.contains(&left[at]))
                        .collect::<Vec<_>>();
                    let empty = (0..board.len())
                        .filter(|&at| board[at] == 0)
                        .collect::<Vec<_>>();
                    if blockers.is_empty() || empty.is_empty() {
                        break;
                    }
                    let pick =
                        |n: usize, scatter: &mut u64| (splitmix64(scatter) % n as u64) as usize;
                    let tile = left.swap_remove(blockers[pick(blockers.len(), &mut scatter)]);
                    board[empty[pick(empty.len(), &mut scatter)]] = tile;
                }
                let mut kept = Vec::<bites::Bites>::new();
                movegen::KurniaMoveGenerator::new(&gc)
                    .gen_remaining_words(
                        &movegen::BoardSnapshot {
                            board_tiles: &board,
                            game_config: &gc,
                            kwg: &kwg,
                            anagrams: None,
                            rack_lengths: None,
                            klv: &klv,
                        },
                        |word: &[u8]| kept.push(word.into()),
                    )
                    .unwrap();
                assert!(kept.windows(2).all(|pair| pair[0] < pair[1]));
                words_kept += kept.len();
                for _ in 0..3 {
                    let (mut line_board, mut line_left) = (board.clone(), left.clone());
                    for _ in 0..6 {
                        let moved = step(&mut line_board, &mut line_left, &mut |after, play| {
                            for word in words_made(&gc, after, play) {
                                assert!(
                                    kept.binary_search_by(|k| k[..].cmp(&word)).is_ok(),
                                    "{word:?} made on {after:?}"
                                );
                                words_checked += 1;
                            }
                        });
                        if moved.is_none() {
                            break;
                        }
                    }
                }
                if step(&mut board, &mut left, &mut |_, _| {}).is_none() {
                    break;
                }
            }
        }
        assert!(words_checked > 0);
        // as many as the prune keeps
        assert_eq!(words_kept, 91021);
    }

    // on boards of random moves, each rack drawn from the tiles not on the
    // board makes the same moves, scored and in the same order, on the graph of
    // the words the board can still take with its own tables of either arm as
    // on the whole graph with the whole graph's tables
    #[test]
    #[inline]
    fn the_graph_a_board_can_still_take_generates_as_the_whole() {
        let gc = game_config::make_english_game_config();
        let alphabet = gc.alphabet();
        let dim = gc.board_layout().dim();
        let rack_size = gc.rack_size() as usize;
        let kwg = kwg::kbwg_from_bytes_alloc(
            build::build(
                build::BuildContent::Gaddawg,
                build::BuildLayout::Wolges,
                build::BuildOrder::Sorted,
                &crossing_word_list(),
            )
            .unwrap()
            .to_vec(),
            false,
        )
        .unwrap();
        let klv = klv::kbwg_from_bytes_alloc(klv::EMPTY_KLV_BYTES.to_vec()).unwrap();
        let mut whole_generator = movegen::KurniaMoveGenerator::new(&gc);
        let mut board_generator = movegen::KurniaMoveGenerator::new(&gc);
        let mut seed = 3u64;
        let mut compared = 0;
        for lean in [false, true] {
            let whole_tables = anagrams::ArmTables::build(&kwg, &gc, lean).unwrap();
            for _ in 0..6 {
                let mut board = vec![0u8; dim.rows as usize * dim.cols as usize];
                let mut left = (0..alphabet.len())
                    .flat_map(|tile| std::iter::repeat_n(tile, alphabet.freq(tile) as usize))
                    .collect::<Vec<_>>();
                for _ in 0..12 {
                    let mut words = Vec::<bites::Bites>::new();
                    movegen::KurniaMoveGenerator::new(&gc)
                        .gen_remaining_words(
                            &movegen::BoardSnapshot {
                                board_tiles: &board,
                                game_config: &gc,
                                kwg: &kwg,
                                anagrams: None,
                                rack_lengths: None,
                                klv: &klv,
                            },
                            |word: &[u8]| words.push(word.into()),
                        )
                        .unwrap();
                    let board_kwg = kwg::Kwg::<kwg::Node24>::from_bytes_alloc(
                        &build::build_for_game(&gc, &words).unwrap(),
                    )
                    .unwrap();
                    let board_tables = anagrams::ArmTables::build(&board_kwg, &gc, lean).unwrap();
                    board_generator.reset_for_another_kwg();
                    let mut moved = false;
                    for _ in 0..3 {
                        let mut rack = Vec::new();
                        while rack.len() < rack_size && !left.is_empty() {
                            let at = (splitmix64(&mut seed) % left.len() as u64) as usize;
                            rack.push(left.swap_remove(at));
                        }
                        for (generator, graph, tables) in [
                            (&mut whole_generator, &kwg, &whole_tables),
                            (&mut board_generator, &board_kwg, &board_tables),
                        ] {
                            generator
                                .gen_moves_unfiltered(&movegen::GenMovesParams {
                                    board_snapshot: &movegen::BoardSnapshot {
                                        board_tiles: &board,
                                        game_config: &gc,
                                        kwg: graph,
                                        anagrams: tables.anagrams(),
                                        rack_lengths: tables.rack_lengths(),
                                        klv: &klv,
                                    },
                                    rack: &rack,
                                    max_gen: usize::MAX,
                                    num_exchanges_by_this_player: 0,
                                    pass_policy: movegen::PassPolicy::OnlyWhenForced,
                                    dynamic_leaves: None,
                                })
                                .unwrap();
                        }
                        assert!(
                            whole_generator.plays == board_generator.plays,
                            "lean={lean}"
                        );
                        compared += 1;
                        // the game goes on by the rack's first move that places
                        // tiles, if it has one
                        let first =
                            whole_generator
                                .plays
                                .iter()
                                .find_map(|valued| match &valued.play {
                                    movegen::Play::Place {
                                        down,
                                        lane,
                                        idx,
                                        word,
                                        ..
                                    } => Some((*down, *lane, *idx, word.clone())),
                                    movegen::Play::Exchange { .. } => None,
                                });
                        if let (false, Some((down, lane, idx, word))) = (moved, first) {
                            apply_place(&gc, &mut board, &mut rack, down, lane, idx, &word);
                            moved = true;
                        }
                        left.extend(rack);
                    }
                    if !moved {
                        break;
                    }
                }
            }
        }
        assert!(compared > 0);
    }

    // a graph with runs turned round (each node keeping its arc; only runs no
    // arc enters past their first node, as the builder shares a run's tail, so
    // the same words) has runs that do not ascend; the solver walks it without
    // the run letters' jumps and answers as on the graph as built
    #[test]
    #[inline]
    fn a_graph_whose_runs_descend_solves_as_the_one_built() {
        use kwg::Node;
        let gc = game_config::make_english_game_config();
        let built = kwg::kbwg_from_bytes_alloc(
            build::build(
                build::BuildContent::Gaddawg,
                build::BuildLayout::Wolges,
                build::BuildOrder::Sorted,
                &tiny_word_list(),
            )
            .unwrap()
            .to_vec(),
            false,
        )
        .unwrap();
        assert!(built.runs_ascend());
        let mut entered = vec![false; built.0.len()];
        for node in built.0.iter() {
            entered[node.arc_index() as usize] = true;
        }
        let mut nodes = built.0.to_vec();
        let mut start = 0;
        let mut turned = 0;
        for end in 0..nodes.len() {
            if built.0[end].is_end() {
                if end > start && !entered[start + 1..=end].contains(&true) {
                    for (k, node) in built.0[start..=end].iter().rev().enumerate() {
                        let at = start + k;
                        nodes[at] = kwg::Node24::new(
                            kwg::Node24::pack(
                                node.tile(),
                                node.accepts(),
                                at == end,
                                node.arc_index(),
                            )
                            .unwrap(),
                        );
                    }
                    turned += 1;
                }
                start = end + 1;
            }
        }
        assert!(turned > 0, "no run could be turned round");
        let descending = kwg::Kwg(nodes);
        assert!(!descending.runs_ascend());
        for racks in [
            [&[2u8, 8][..], &[1, 1, 19]],
            [&[1, 2], &[8, 19]],
            [&[0, 8], &[1, 2]],
        ] {
            let board = late_board(&gc, racks);
            let solve = |kwg: &kwg::Kwg<kwg::Node24>| {
                let arm_tables = anagrams::ArmTables::build(kwg, &gc, false).unwrap();
                let mut egs =
                    EndgameSolver::<kwg::Node24, kwg::Node24>::new(&gc, kwg, &arm_tables).unwrap();
                egs.init(&board, racks);
                egs.solve(0).unwrap()
            };
            assert_eq!(solve(&descending), solve(&built), "{racks:?}");
        }
    }

    #[test]
    #[inline]
    fn a_solve_a_limit_stops_answers_from_a_finished_depth() {
        let gc = game_config::make_english_game_config();
        let kwg_bytes = tiny_kwg_bytes();
        let kwg = kwg::kbwg_from_bytes_alloc(kwg_bytes.to_vec(), false).unwrap();
        let arm_tables = anagrams::ArmTables::build(&kwg, &gc, false).unwrap();

        let mut cut = 0usize;
        let mut crowded = 0usize;
        for (name, pos) in all_positions() {
            let mut egs =
                EndgameSolver::<kwg::Node24, kwg::Node24>::new(&gc, &kwg, &arm_tables).unwrap();
            egs.init(&pos.board, [&pos.racks[0][..], &pos.racks[1][..]]);
            let full = egs.solve_within(0, &super::Limits::NONE).unwrap();
            assert!(full.exact, "[{name}] an unlimited solve is exact");

            // each depth a solve finishes under a limit never reached (an
            // unlimited one skips some), with its value and table size; its
            // answer is the unlimited one's, at most a depth sooner
            let finished = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
            let mut egs =
                EndgameSolver::<kwg::Node24, kwg::Node24>::new(&gc, &kwg, &arm_tables).unwrap();
            egs.init(&pos.board, [&pos.racks[0][..], &pos.racks[1][..]]);
            let seen = finished.clone();
            let heard = move |p: &super::Progress| {
                if let Some(value) = p.value {
                    seen.borrow_mut().push((p.depth, value, p.held));
                }
                Ok(())
            };
            let never = super::Limits {
                time: None,
                states: Some(usize::MAX),
                memory: None,
                clock: None,
            };
            let every = egs
                .listening(heard, |egs| egs.solve_within(0, &never))
                .unwrap();
            let finished = finished.borrow();
            assert!(
                every.exact
                    && every.value == full.value
                    && (full.depth - 1..=full.depth).contains(&every.depth),
                "[{name}] a limit never reached answers the unlimited value, a depth sooner at most"
            );
            assert_eq!(
                finished.last().map(|&(d, v, _)| (d, v)),
                Some((every.depth, every.value.unwrap())),
                "[{name}] the last finished depth is the answer"
            );
            if finished.len() < 2 {
                continue;
            }

            // no time at all: the first depth always finishes, and is the answer
            let mut egs =
                EndgameSolver::<kwg::Node24, kwg::Node24>::new(&gc, &kwg, &arm_tables).unwrap();
            egs.init(&pos.board, [&pos.racks[0][..], &pos.racks[1][..]]);
            let limits = super::Limits {
                time: Some(std::time::Duration::ZERO),
                states: None,
                memory: None,
                clock: None,
            };
            let solved = egs.solve_within(0, &limits).unwrap();
            assert!(
                !solved.exact,
                "[{name}] a zero time limit cuts the solve short"
            );
            assert_eq!(
                (solved.depth, solved.value),
                (finished[0].0, Some(finished[0].1)),
                "[{name}] a zero time limit answers the first depth"
            );

            // the time read on a given clock, never std's: on one that never
            // moves, however little time is given is never used up
            let mut egs =
                EndgameSolver::<kwg::Node24, kwg::Node24>::new(&gc, &kwg, &arm_tables).unwrap();
            egs.init(&pos.board, [&pos.racks[0][..], &pos.racks[1][..]]);
            let limits = super::Limits {
                time: Some(std::time::Duration::from_nanos(1)),
                states: None,
                memory: None,
                clock: Some(|| std::time::Duration::ZERO),
            };
            let solved = egs.solve_within(0, &limits).unwrap();
            assert!(
                solved.exact
                    && solved.value == full.value
                    && (solved.depth, solved.value) == (every.depth, every.value),
                "[{name}] on a clock that never moves, a time limit is never reached: \
                 depth {} value {:?} exact {}",
                solved.depth,
                solved.value,
                solved.exact
            );

            // the memory past its limit from the start: the same
            let mut egs =
                EndgameSolver::<kwg::Node24, kwg::Node24>::new(&gc, &kwg, &arm_tables).unwrap();
            egs.init(&pos.board, [&pos.racks[0][..], &pos.racks[1][..]]);
            let limits = super::Limits {
                time: None,
                states: None,
                memory: Some(super::MemoryLimit {
                    bytes: 1,
                    held: || usize::MAX,
                    reserved: None,
                }),
                clock: None,
            };
            let solved = egs.solve_within(0, &limits).unwrap();
            assert!(
                !solved.exact,
                "[{name}] a memory limit passed cuts the solve short"
            );
            assert!(
                solved.out_of_memory
                    && solved.out_of_memory_in_moves
                    && solved.depth == 0
                    && solved.value.is_none(),
                "[{name}] a memory limit passed stops the first generation, no depth finished"
            );

            // a memory limit never reached changes nothing
            let mut egs =
                EndgameSolver::<kwg::Node24, kwg::Node24>::new(&gc, &kwg, &arm_tables).unwrap();
            egs.init(&pos.board, [&pos.racks[0][..], &pos.racks[1][..]]);
            let limits = super::Limits {
                time: None,
                states: None,
                memory: Some(super::MemoryLimit {
                    bytes: usize::MAX,
                    held: || 0,
                    reserved: None,
                }),
                clock: None,
            };
            let solved = egs.solve_within(0, &limits).unwrap();
            assert!(
                solved.exact && (solved.depth, solved.value) == (full.depth, full.value),
                "[{name}] a memory limit never reached answers as no limit"
            );

            // with nothing held and one byte allowed, the table cannot grow
            // past its first bucket, so its positions replace each other, and
            // the solve still runs to the end
            let mut egs =
                EndgameSolver::<kwg::Node24, kwg::Node24>::new(&gc, &kwg, &arm_tables).unwrap();
            egs.init(&pos.board, [&pos.racks[0][..], &pos.racks[1][..]]);
            let limits = super::Limits {
                time: None,
                states: None,
                memory: Some(super::MemoryLimit {
                    bytes: 1,
                    held: || 0,
                    reserved: None,
                }),
                clock: None,
            };
            let solved = egs.solve_within(0, &limits).unwrap();
            assert!(
                solved.exact && solved.value == full.value,
                "[{name}] a table that may not grow answers as an unlimited solve"
            );
            assert_eq!(egs.work_buffer.table.room(), 4);
            crowded += usize::from(finished.last().unwrap().2 > 4);

            // one state past the first depth's: the search stops within a later
            // pass, or after it, and answers a depth that finished
            let mut egs =
                EndgameSolver::<kwg::Node24, kwg::Node24>::new(&gc, &kwg, &arm_tables).unwrap();
            egs.init(&pos.board, [&pos.racks[0][..], &pos.racks[1][..]]);
            let limits = super::Limits {
                time: None,
                states: Some(finished[0].2 + 1),
                memory: None,
                clock: None,
            };
            let solved = egs.solve_within(0, &limits).unwrap();
            if !solved.exact {
                cut += 1;
                // the finished pass's value, or that of a move the stopped pass
                // found worth more; either way the depth is the finished pass's
                assert!(
                    finished.iter().any(|&(d, v, _)| d == solved.depth
                        && (Some(v) == solved.value
                            || solved.partway.is_some_and(|stopped| stopped > d))),
                    "[{name}] a state limit answers depth {} with {:?}, which no pass finished",
                    solved.depth,
                    solved.value
                );
            } else {
                assert_eq!(
                    (solved.depth, solved.value),
                    (every.depth, every.value),
                    "[{name}] a state limit never reached answers as no limit"
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
        let kwg = kwg::kbwg_from_bytes_alloc(kwg_bytes.to_vec(), false).unwrap();
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
            EndgameSolver::<kwg::Node24, kwg::Node24>::new(&gc, &kwg, &arm_tables).unwrap();
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
        let kwg = kwg::kbwg_from_bytes_alloc(kwg_bytes.to_vec(), false).unwrap();
        let arm_tables = anagrams::ArmTables::build(&kwg, &gc, false).unwrap();
        let positions = all_positions();
        let mut egs =
            EndgameSolver::<kwg::Node24, kwg::Node24>::new(&gc, &kwg, &arm_tables).unwrap();
        let mut movegen = std::time::Duration::ZERO;
        for (_, pos) in &positions {
            egs.init(&pos.board, [&pos.racks[0][..], &pos.racks[1][..]]);
            egs.solve(0).unwrap();
            movegen += egs.work_buffer.dur_movegen;
        }
        assert_eq!(movegen, std::time::Duration::ZERO, "no listener, no clock");
        egs.listening(
            |_: &super::Progress| Ok(()),
            |egs| {
                for (_, pos) in &positions {
                    egs.init(&pos.board, [&pos.racks[0][..], &pos.racks[1][..]]);
                    egs.solve(0).unwrap();
                    movegen += egs.work_buffer.dur_movegen;
                }
            },
        );
        assert!(movegen > std::time::Duration::ZERO, "a listener, the time");
    }

    #[test]
    #[inline]
    fn a_solve_fits_a_small_stack() {
        // a solve recurses a frame a ply, never a frame a move, so it runs on a
        // thread with far less stack than a main thread has (a test's own
        // thread has more, so this one asks for a small one): full racks, as
        // many plies as they hold
        let gc = game_config::make_english_game_config();
        let kwg_bytes = tiny_kwg_bytes();
        let kwg = kwg::kbwg_from_bytes_alloc(kwg_bytes.to_vec(), false).unwrap();
        let arm_tables = anagrams::ArmTables::build(&kwg, &gc, false).unwrap();
        std::thread::scope(|s| {
            std::thread::Builder::new()
                .stack_size(256 << 10)
                .spawn_scoped(s, || {
                    let mut egs =
                        EndgameSolver::<kwg::Node24, kwg::Node24>::new(&gc, &kwg, &arm_tables)
                            .unwrap();
                    let mut board = empty_board();
                    put_word(&mut board, 7, 6, &[1, 20]);
                    egs.init(&board, [&[1, 1, 2, 8, 20, 20, 1], &[1, 2, 1, 20, 8, 1, 20]]);
                    egs.solve_within(
                        0,
                        &super::Limits {
                            time: Some(std::time::Duration::from_secs(2)),
                            states: None,
                            memory: None,
                            clock: None,
                        },
                    )
                    .unwrap();
                })
                .unwrap()
                .join()
                .unwrap()
        });
    }

    #[test]
    #[inline]
    fn a_key_made_move_by_move_is_the_key_made_afresh() {
        let gc = game_config::make_english_game_config();
        let kwg_bytes = tiny_kwg_bytes();
        let kwg = kwg::kbwg_from_bytes_alloc(kwg_bytes.to_vec(), false).unwrap();
        let arm_tables = anagrams::ArmTables::build(&kwg, &gc, false).unwrap();
        let mut egs =
            EndgameSolver::<kwg::Node24, kwg::Node24>::new(&gc, &kwg, &arm_tables).unwrap();
        let mut afresh =
            EndgameSolver::<kwg::Node24, kwg::Node24>::new(&gc, &kwg, &arm_tables).unwrap();
        let mut moves = 0usize;
        for (name, pos) in all_positions() {
            for mover in 0..2u8 {
                egs.init(&pos.board, [&pos.racks[0][..], &pos.racks[1][..]]);
                egs.reset_position();
                let before = egs.work_buffer.key;
                egs.generate(0, mover, false).unwrap();
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
        let kwg = kwg::kbwg_from_bytes_alloc(kwg_bytes.to_vec(), false).unwrap();
        let arm_tables = anagrams::ArmTables::build(&kwg, &gc, false).unwrap();
        let mut egs =
            EndgameSolver::<kwg::Node24, kwg::Node24>::new(&gc, &kwg, &arm_tables).unwrap();
        let mut afresh =
            EndgameSolver::<kwg::Node24, kwg::Node24>::new(&gc, &kwg, &arm_tables).unwrap();
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
                egs.generate(0, mover, false).unwrap();
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
    fn a_reserved_table_answers_as_a_grown_one() {
        let gc = game_config::make_english_game_config();
        let kwg = kwg::kbwg_from_bytes_alloc(tiny_kwg_bytes().to_vec(), false).unwrap();
        let arm_tables = anagrams::ArmTables::build(&kwg, &gc, false).unwrap();
        let mut grown =
            EndgameSolver::<kwg::Node24, kwg::Node24>::new(&gc, &kwg, &arm_tables).unwrap();
        let mut reserved =
            EndgameSolver::<kwg::Node24, kwg::Node24>::new(&gc, &kwg, &arm_tables).unwrap();
        let bucket = std::mem::size_of::<[u64; 8]>();
        assert!(reserved.reserve_table(bucket << 10));
        assert_eq!(reserved.work_buffer.table.room(), 4 << 10);
        assert!(
            !reserved.reserve_table(bucket << 9),
            "a smaller table is not taken"
        );
        for (_, pos) in &all_positions() {
            for egs in [&mut grown, &mut reserved] {
                egs.init(&pos.board, [&pos.racks[0][..], &pos.racks[1][..]]);
            }
            assert_eq!(reserved.solve(0).unwrap(), grown.solve(0).unwrap());
        }
        assert!(
            !reserved.reserve_table(bucket << 12),
            "a table holding positions stays"
        );
    }

    #[test]
    fn a_table_too_big_to_hold_is_not_reserved() {
        let gc = game_config::make_english_game_config();
        let kwg = kwg::kbwg_from_bytes_alloc(tiny_kwg_bytes().to_vec(), false).unwrap();
        let arm_tables = anagrams::ArmTables::build(&kwg, &gc, false).unwrap();
        let mut egs =
            EndgameSolver::<kwg::Node24, kwg::Node24>::new(&gc, &kwg, &arm_tables).unwrap();
        for bytes in [usize::MAX, isize::MAX as usize] {
            assert!(!egs.reserve_table(bytes), "{bytes}");
            assert_eq!(egs.work_buffer.table.room(), 4);
        }
        let bucket = std::mem::size_of::<[u64; 8]>();
        assert!(egs.reserve_table(bucket << 10));
        assert_eq!(egs.work_buffer.table.room(), 4 << 10);
    }

    #[test]
    #[inline]
    fn the_table_keeps_what_it_is_given_and_the_deeper_of_two() {
        let entry = |value: i32, depth: i8, bound: super::Bound| super::Entry {
            value: equity::Equity::new(value),
            play: 7,
            depth,
            bound,
        };
        let mut table = super::Table::new();
        table.generation = 1;
        assert!(table.probe(1).is_none(), "an empty table knows nothing");
        assert!(table.store(1, entry(-2500, 3, super::Bound::Lower)));
        let found = table.probe(1).unwrap();
        assert_eq!(
            (found.value, found.play, found.depth),
            (equity::Equity::new(-2500), 7, 3),
            "an entry comes back as it went in"
        );
        assert!(found.bound == super::Bound::Lower);
        // a shallower bound leaves the deeper value; an exact one takes its
        // place
        assert!(!table.store(1, entry(9000, 2, super::Bound::Upper)));
        assert_eq!(table.probe(1).unwrap().value, equity::Equity::new(-2500));
        assert!(!table.store(1, entry(4000, 1, super::Bound::Exact)));
        assert_eq!(table.probe(1).unwrap().value, equity::Equity::new(4000));
        assert_eq!(table.held, 1);
        // a full bucket gives up its shallowest entry
        for key in 2..=4 {
            assert!(table.store(key, entry(0, 5, super::Bound::Exact)));
        }
        assert!(!table.store(5, entry(0, 5, super::Bound::Exact)));
        assert!(table.probe(1).is_none(), "the shallowest entry made room");
        assert!((2..=5).all(|key| table.probe(key).is_some()));
        // a table twice as big keeps every entry
        assert!(table.grow(2));
        table.finish();
        assert_eq!(table.held, 4);
        assert!((2..=5).all(|key| table.probe(key).is_some()));
        // more buckets than memory can hold are refused, and the table is as it
        // was, not the program stopped
        assert!(!table.grow(usize::MAX));
        assert!(!table.growing());
        assert_eq!((table.buckets.len(), table.held), (2, 4));
        assert!((2..=5).all(|key| table.probe(key).is_some()));
    }

    // a table grown a piece at a time, read and stored into as it grows, as a
    // search does: each position is in it once, with what was last stored of
    // it, and the old buckets are freed at the end
    #[test]
    #[inline]
    fn a_table_growing_a_piece_at_a_time_keeps_each_position_once() {
        let entry = |value: i32, depth: i8, bound: super::Bound| super::Entry {
            value: equity::Equity::new(value),
            play: 7,
            depth,
            bound,
        };
        let key = |i: u64| {
            let mut state = i;
            splitmix64(&mut state)
        };
        let mut table = super::Table::new();
        table.generation = 1;
        assert!(table.grow(4 * super::MOVE_PIECE));
        table.finish();
        let positions = 8 * super::MOVE_PIECE as u64;
        for i in 0..positions {
            table.store(key(i), entry(i as i32, 5, super::Bound::Exact));
        }
        let held = table.held;
        assert!(held > 7 * super::MOVE_PIECE, "few positions were dropped");
        let kept = (0..positions)
            .filter(|&i| table.probe(key(i)).is_some())
            .collect::<Vec<_>>();
        assert_eq!(kept.len(), held);
        // the bigger table is made empty over more than one store, then the old
        // buckets move over more than one
        assert!(table.grow(4 * super::EMPTY_PIECE));
        table.advance();
        assert!(table.growing() && table.old.is_empty());
        while table.old.is_empty() {
            table.advance();
        }
        // while they move: a stored position not yet moved keeps its deeper
        // value, another is read where it is, and a new one is stored
        let last = *kept.last().unwrap();
        assert!(!table.store(key(last), entry(-1000, 3, super::Bound::Lower)));
        assert_eq!(
            table.probe(key(kept[0])).unwrap().value,
            equity::Equity::new(kept[0] as i32)
        );
        assert!(table.store(key(positions), entry(-2000, 1, super::Bound::Upper)));
        table.advance();
        assert!(table.growing(), "the old buckets moved in one store");
        table.finish();
        assert!(table.old.is_empty() && table.old.capacity() == 0);
        assert_eq!(table.buckets.len(), 4 * super::EMPTY_PIECE);
        assert_eq!(table.held, held + 1);
        for &i in &kept {
            assert_eq!(
                table.probe(key(i)).unwrap().value,
                equity::Equity::new(i as i32)
            );
        }
        assert_eq!(
            table.probe(key(positions)).unwrap().value,
            equity::Equity::new(-2000)
        );
        let mut keys = table
            .buckets
            .iter()
            .flat_map(|bucket| {
                (0..4)
                    .filter(|&slot| (bucket[2 * slot + 1] >> 56) & 3 != 0)
                    .map(|slot| bucket[2 * slot])
            })
            .collect::<Vec<_>>();
        assert_eq!(keys.len(), held + 1);
        keys.sort_unstable();
        keys.dedup();
        assert_eq!(keys.len(), held + 1, "a position is in the table twice");
    }

    // a search's stores grow its table at once while it is small, then a piece
    // at each store, each growing done before the next begins, and every
    // position the table holds can be read
    #[test]
    #[inline]
    fn stores_grow_a_big_table_a_piece_at_a_time() {
        let gc = game_config::make_english_game_config();
        let mut work_buffer = super::WorkBuffer::new(movegen::KurniaMoveGenerator::new(&gc), None);
        work_buffer.table.generation = 1;
        let entry = super::Entry {
            value: equity::Equity::ZERO,
            play: 7,
            depth: 5,
            bound: super::Bound::Exact,
        };
        let key = |i: u64| {
            let mut state = i;
            splitmix64(&mut state)
        };
        let (mut stores, mut growing, mut small) = (0, 0, 0);
        while work_buffer.table.buckets.len() <= 2 * super::GROW_AT_ONCE && stores < 1 << 20 {
            work_buffer.store(key(stores), entry);
            stores += 1;
            if work_buffer.table.growing() {
                growing += 1;
                small += usize::from(work_buffer.table.buckets.len() < super::GROW_AT_ONCE);
            }
        }
        assert_eq!(work_buffer.table.buckets.len(), 4 * super::GROW_AT_ONCE);
        assert_eq!(small, 0, "a small table grew a piece at a time");
        // growing to twice GROW_AT_ONCE took the store that began it and one a
        // piece after, and growing to four times it has just made its table
        // empty, so no growing began again while one ran
        let (empty, move_) = (super::EMPTY_PIECE, super::MOVE_PIECE);
        let pieces = 2 * super::GROW_AT_ONCE / empty + super::GROW_AT_ONCE / move_;
        assert_eq!(growing, pieces + 4 * super::GROW_AT_ONCE / empty + 1);
        while work_buffer.table.growing() {
            work_buffer.store(key(stores), entry);
            stores += 1;
        }
        let held = work_buffer.table.held;
        assert!(held >= 3 * 2 * super::GROW_AT_ONCE);
        assert_eq!(
            (0..stores)
                .filter(|&i| work_buffer.table.probe(key(i)).is_some())
                .count(),
            held
        );
    }

    // the memory held as a counting allocator counts it: every bucket the table
    // holds, with the rest. The limit lets the table double to 4 GROW_AT_ONCE
    // buckets, then grow to 6 of them, a growing that leaves no room. An
    // allocation then stops no solve while the old buckets wait to be moved: a
    // stopped solve stores nothing more, so the growing would never end. One
    // passing what they free stops a solve both while the table grows and after
    #[test]
    #[inline]
    fn the_buckets_a_growing_table_leaves_stop_no_solve() {
        // a limit reads one count here, its bytes moved by what is held, as the
        // count would move under a fixed limit
        const READ: usize = usize::MAX / 2;
        let size = std::mem::size_of::<[u64; 8]>();
        let at_once = super::GROW_AT_ONCE;
        let rest = at_once * size;
        let bytes = rest + (4 + 6) * at_once * size + 1;
        let limit = |held: usize| super::MemoryLimit {
            bytes: bytes + READ - held,
            held: || READ,
            reserved: None,
        };
        let counted = |table: &super::Table| {
            (table.buckets.capacity() + table.next.capacity() + table.old.capacity()) * size
        };
        let entry = super::Entry {
            value: equity::Equity::ZERO,
            play: 7,
            depth: 5,
            bound: super::Bound::Exact,
        };
        let key = |i: u64| {
            let mut state = i;
            splitmix64(&mut state)
        };
        // a store, its growing sized by what is held before it
        let store = |work_buffer: &mut super::WorkBuffer<u128, true, true>,
                     stores: &mut u64,
                     rest: usize| {
            work_buffer.memory_limit = Some(limit(counted(&work_buffer.table) + rest));
            work_buffer.store(key(*stores), entry);
            *stores += 1;
        };
        // whether a solve stops with this much held, as a bag search does
        let stops = |work_buffer: &mut super::WorkBuffer<u128, true, true>, held: usize| {
            let limit = limit(held);
            work_buffer.memory_limit = Some(limit);
            let bag_limits = super::BagLimits::new(
                &super::Limits {
                    time: None,
                    states: None,
                    memory: Some(limit),
                    clock: None,
                },
                None,
            );
            let one = work_buffer.memory_passed();
            assert_eq!(
                one,
                bag_limits.passed(&work_buffer.table),
                "a solve and a bag search disagree"
            );
            one
        };
        let gc = game_config::make_english_game_config();
        let mut work_buffer = super::WorkBuffer::new(movegen::KurniaMoveGenerator::new(&gc), None);
        work_buffer.table.generation = 1;
        let mut stores = 0;
        while work_buffer.table.target != 6 * at_once {
            assert!(stores < 1 << 22, "the table never grew to 6 GROW_AT_ONCE");
            store(&mut work_buffer, &mut stores, rest);
        }
        assert_eq!(work_buffer.table.buckets.len(), 4 * at_once);
        let held = counted(&work_buffer.table) + rest;
        assert!(!limit(held).passed(0), "the growing passed the limit");
        // a bucket's bytes more held: past the limit, counting all
        let rest = rest + size;
        assert!(limit(held + size).passed(0));
        let mut growing = 0;
        while work_buffer.table.growing() {
            let held = counted(&work_buffer.table) + rest;
            assert!(
                !stops(&mut work_buffer, held),
                "the old buckets stopped a solve"
            );
            let leaving = work_buffer.table.leaving();
            assert!(
                stops(&mut work_buffer, held + leaving),
                "passing what leaves stopped no solve"
            );
            store(&mut work_buffer, &mut stores, rest);
            growing += 1;
        }
        assert!(growing > 1, "the growing took one store");
        assert_eq!(work_buffer.table.buckets.len(), 6 * at_once);
        assert_eq!(work_buffer.table.leaving(), 0);
        let held = counted(&work_buffer.table) + rest;
        assert!(!stops(&mut work_buffer, held));
        assert!(
            stops(&mut work_buffer, held + 4 * at_once * size),
            "the rest passing the limit stopped no solve"
        );
    }

    #[test]
    #[inline]
    fn an_entry_a_later_solve_reads_is_kept_before_one_it_does_not() {
        let entry = |depth: i8| super::Entry {
            value: equity::Equity::ZERO,
            play: 7,
            depth,
            bound: super::Bound::Exact,
        };
        // one bucket, filled by a solve: the shallowest entry is 1's
        let mut table = super::Table::new();
        table.generation = 1;
        for key in 1..=4 {
            assert!(table.store(key, entry(key as i8)));
        }
        // a later solve reads 1, then stores 5: 2's, the shallowest it did not
        // read, makes room
        table.generation = 2;
        assert!(table.probe(1).is_some());
        assert!(!table.store(5, entry(9)));
        assert!(table.probe(2).is_none(), "the entry not read made room");
        assert!(
            (1..=5)
                .filter(|&key| key != 2)
                .all(|key| table.probe(key).is_some())
        );
    }

    // a solve in steps, as a wasm32 page runs one: each step a new solver,
    // given the table the steps before kept, stopped once it holds 30 positions
    // more. A step never answers from a shallower depth than one before it did,
    // and the steps end with the position's value; also with the table kept to
    // 256 buckets by a memory limit no solve passes, where once it is full each
    // step's entries replace earlier steps'
    #[test]
    #[inline]
    fn a_solve_in_steps_on_a_kept_table_never_answers_shallower() {
        let gc = game_config::make_english_game_config();
        let kwg = kwg::kbwg_from_bytes_alloc(tiny_kwg_bytes().to_vec(), false).unwrap();
        let arm_tables = anagrams::ArmTables::build(&kwg, &gc, false).unwrap();
        for memory in [
            None,
            Some(super::MemoryLimit {
                bytes: 256 * std::mem::size_of::<[u64; 8]>() + 1,
                held: || 0,
                reserved: None,
            }),
        ] {
            let mut stepped = 0;
            for (name, pos) in all_positions() {
                let mut egs =
                    EndgameSolver::<kwg::Node24, kwg::Node24>::new(&gc, &kwg, &arm_tables).unwrap();
                egs.init(&pos.board, [&pos.racks[0][..], &pos.racks[1][..]]);
                let full = egs.solve(0).unwrap();
                let mut kept = None;
                let mut deepest = 0;
                for step in 1.. {
                    assert!(step < 10000, "[{name}] the steps never ended");
                    let mut egs =
                        EndgameSolver::<kwg::Node24, kwg::Node24>::new(&gc, &kwg, &arm_tables)
                            .unwrap();
                    if let Some(table) = kept.take() {
                        assert!(egs.give_table(table).is_ok());
                    }
                    egs.init(&pos.board, [&pos.racks[0][..], &pos.racks[1][..]]);
                    let limits = super::Limits {
                        time: None,
                        states: Some(egs.work_buffer.table.held + 30),
                        memory,
                        clock: None,
                    };
                    let solved = egs.solve_within(0, &limits).unwrap();
                    kept = Some(egs.take_table());
                    assert!(
                        solved.depth >= deepest,
                        "[{name}] step {step} answered from depth {} after depth {deepest}",
                        solved.depth
                    );
                    deepest = solved.depth;
                    if solved.exact {
                        assert_eq!(solved.value.unwrap(), full, "[{name}]");
                        stepped += usize::from(step > 1);
                        break;
                    }
                }
            }
            assert!(stepped > 0, "no position took more than one step");
        }
    }

    // a solve in steps where each step deepens on from the depth the step
    // before finished, with its value, as a wasm32 page's steps do: no step
    // searches a depth it was given as finished, none answers from a shallower
    // one, and the steps end with the position's value
    #[test]
    #[inline]
    fn a_solve_resumed_past_its_finished_depth_ends_as_one_solve() {
        let gc = game_config::make_english_game_config();
        let kwg = kwg::kbwg_from_bytes_alloc(tiny_kwg_bytes().to_vec(), false).unwrap();
        let arm_tables = anagrams::ArmTables::build(&kwg, &gc, false).unwrap();
        let mut resumed = 0;
        for (name, pos) in all_positions() {
            let mut egs =
                EndgameSolver::<kwg::Node24, kwg::Node24>::new(&gc, &kwg, &arm_tables).unwrap();
            egs.init(&pos.board, [&pos.racks[0][..], &pos.racks[1][..]]);
            let full = egs.solve(0).unwrap();
            let mut kept = None;
            let mut finished = None;
            for step in 1.. {
                assert!(step < 10000, "[{name}] the steps never ended");
                let mut egs =
                    EndgameSolver::<kwg::Node24, kwg::Node24>::new(&gc, &kwg, &arm_tables).unwrap();
                if let Some(table) = kept.take() {
                    assert!(egs.give_table(table).is_ok());
                }
                egs.init(&pos.board, [&pos.racks[0][..], &pos.racks[1][..]]);
                // the shallowest depth the step searched at any node
                let shallowest = std::rc::Rc::new(std::cell::Cell::new(i8::MAX));
                let seen = shallowest.clone();
                let heard = move |progress: &super::Progress| {
                    seen.set(seen.get().min(progress.depth));
                    Ok(())
                };
                let limits = super::Limits {
                    time: None,
                    states: Some(egs.work_buffer.table.held + 30),
                    memory: None,
                    clock: None,
                };
                let solved = egs
                    .listening(heard, |egs| match finished {
                        Some(finished) => egs.solve_within_after(0, &limits, finished),
                        None => egs.solve_within(0, &limits),
                    })
                    .unwrap();
                kept = Some(egs.take_table());
                if let Some((depth, _)) = finished {
                    assert!(
                        shallowest.get() > depth,
                        "[{name}] step {step} searched depth {} after depth {depth}",
                        shallowest.get()
                    );
                    assert!(solved.depth >= depth, "[{name}] step {step}");
                    resumed += 1;
                }
                if solved.exact {
                    assert_eq!(solved.value.unwrap(), full, "[{name}]");
                    break;
                }
                finished = Some((solved.depth, solved.value.unwrap()));
            }
        }
        assert!(resumed > 0, "no position took more than one step");
    }

    #[test]
    #[inline]
    fn peg_aggregate_weights_each_outcome_by_how_many_tiles_give_it() {
        let e = equity::Equity::new;
        let (win_pct, expected) =
            super::peg_aggregate(&[(2, e(3000)), (1, e(0)), (1, e(-5000))]).unwrap();
        assert_eq!(win_pct, 0.625);
        assert_eq!(expected, e(250));

        let (win_pct, expected) =
            super::peg_aggregate(&[(1, e(1)), (1, e(0)), (1, e(-1))]).unwrap();
        assert_eq!(win_pct, 0.5);
        assert_eq!(expected, e(0));

        assert_eq!(super::peg_aggregate(&[]).unwrap(), (0.0, e(0)));
    }

    #[test]
    #[inline]
    fn an_endgame_for_three_players_is_refused_at_setup() {
        let gc = game_config::Options {
            preset: "english",
            tiles: None,
            board: None,
            rack_size: None,
            jumbled: false,
            woadwage: false,
            players: Some(3),
            bingo_bonus: None,
            zeros_to_end: None,
            passes_to_end: None,
            exchange_limit: None,
            exchanges: None,
        }
        .make_game_config(&|_| Err("no files here".into()))
        .unwrap();
        assert!(gc.supports(game_config::Op::Endgame).is_err());
    }

    #[test]
    #[inline]
    fn peg_one_in_bag_hand_checkable() {
        let gc = game_config::make_english_game_config();
        let kwg_bytes = tiny_kwg_bytes();
        let kwg = kwg::kbwg_from_bytes_alloc(kwg_bytes.to_vec(), false).unwrap();

        let board = empty_board();
        let mover_rack = [2u8]; // B
        let mut unseen_tally = vec![0u8; gc.alphabet().len() as usize];
        unseen_tally[8] = 1; // H
        unseen_tally[20] = 1; // T

        let arm_tables = anagrams::ArmTables::build(&kwg, &gc, false).unwrap();
        let mut egs =
            EndgameSolver::<kwg::Node24, kwg::Node24>::new(&gc, &kwg, &arm_tables).unwrap();
        let result = egs
            .solve_peg_one_in_bag_full(
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
        let kwg = kwg::kbwg_from_bytes_alloc(kwg_bytes.to_vec(), false).unwrap();

        let board = empty_board();
        let mover_rack = [2u8]; // B
        let mut unseen_tally = vec![0u8; gc.alphabet().len() as usize];
        unseen_tally[8] = 1; // H
        unseen_tally[20] = 1; // T

        let arm_tables = anagrams::ArmTables::build(&kwg, &gc, false).unwrap();
        let mut egs =
            EndgameSolver::<kwg::Node24, kwg::Node24>::new(&gc, &kwg, &arm_tables).unwrap();
        let e = equity::Equity::new;
        let result = egs
            .solve_peg_one_in_bag_full(
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

        let lead = 2_147_483_000;
        for (lead, outcomes, win) in [
            (lead, [lead - 2000, i32::MAX], 1.0),
            (-lead, [i32::MIN, -lead + 1000], 0.0),
        ] {
            let result = egs
                .solve_peg_one_in_bag_full(
                    0,
                    &board,
                    &mover_rack,
                    &unseen_tally,
                    e(lead),
                    &super::Limits::NONE,
                )
                .unwrap()
                .unwrap();
            assert_eq!(
                result.hypotheses,
                vec![(8, 1, e(outcomes[0])), (20, 1, e(outcomes[1]))]
            );
            assert_eq!(result.win_pct, win);
            assert_eq!(result.expected_margin, e(lead - 500));
        }
    }

    #[test]
    #[inline]
    fn a_peg_solve_says_when_a_limit_cut_an_endgame_inside_it() {
        let kwg_bytes = tiny_kwg_bytes();
        let kwg = kwg::kbwg_from_bytes_alloc(kwg_bytes.to_vec(), false).unwrap();
        let no_time = super::Limits {
            time: Some(std::time::Duration::ZERO),
            states: None,
            memory: None,
            clock: None,
        };
        let one_state = super::Limits {
            time: None,
            states: Some(1),
            memory: None,
            clock: None,
        };
        let no_memory = super::Limits {
            time: None,
            states: None,
            memory: Some(super::MemoryLimit {
                bytes: 1,
                held: || usize::MAX,
                reserved: None,
            }),
            clock: None,
        };
        let tiles = [1u8, 2, 8, 20]; // A B H T
        let mut board = empty_board();
        put_word(&mut board, 7, 6, &[1, 20]);
        let mut cut = [0usize; 3];
        let mut listed = 0usize;
        // the committed model, and the clairvoyant one where exchanges are legal
        for gc in [
            game_config::make_english_game_config(),
            game_config::make_exchange_test_game_config(),
        ] {
            let arm_tables = anagrams::ArmTables::build(&kwg, &gc, false).unwrap();
            let mut egs =
                EndgameSolver::<kwg::Node24, kwg::Node24>::new(&gc, &kwg, &arm_tables).unwrap();
            let mut unseen_tally = vec![0u8; gc.alphabet().len() as usize];
            unseen_tally[8] = 1; // H
            unseen_tally[20] = 1; // T
            for &x in &tiles {
                for &y in &tiles {
                    let mover_rack = [x, y];
                    let mut solve = |limits: &super::Limits| {
                        egs.solve_peg_one_in_bag_full(
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
                    // every move the committed model valued, best first, the
                    // first the answer, each one's tiles making its value
                    if full.committed {
                        let moves = &full.moves;
                        assert!(!moves.is_empty());
                        assert!(same_peg_move(&Some(moves[0].play.clone()), &full.best_move));
                        assert_eq!(moves[0].win_pct, full.win_pct);
                        assert_eq!(moves[0].expected_margin, full.expected_margin);
                        assert_eq!(moves[0].hypotheses, full.hypotheses);
                        for pair in moves.windows(2) {
                            assert!(
                                (pair[0].win_pct, pair[0].expected_margin)
                                    >= (pair[1].win_pct, pair[1].expected_margin)
                            );
                        }
                        for valued in moves {
                            assert!(valued.exact);
                            let weighted = (valued.hypotheses.iter())
                                .map(|&(_, count, value)| (count, value))
                                .collect::<Vec<_>>();
                            assert_eq!(
                                super::peg_aggregate(&weighted).unwrap(),
                                (valued.win_pct, valued.expected_margin)
                            );
                        }
                        listed += moves.len();
                    } else {
                        assert!(full.moves.is_empty());
                    }
                    for (i, limits) in [&no_time, &one_state, &no_memory].into_iter().enumerate() {
                        let limited = solve(limits);
                        if limited.hypotheses.is_empty() {
                            // stopped before its first pass valued every tile
                            assert!(!limited.exact && limited.depth == 0);
                            assert!(limited.best_move.is_none());
                            cut[i] += 1;
                            continue;
                        }
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
        assert!(listed > 0, "the committed model listed no move");
    }

    // the same move, or both none
    fn same_peg_move(a: &Option<super::PegMove>, b: &Option<super::PegMove>) -> bool {
        match (a, b) {
            (None, None) | (Some(super::PegMove::Pass), Some(super::PegMove::Pass)) => true,
            (Some(super::PegMove::Place(a)), Some(super::PegMove::Place(b))) => a == b,
            _ => false,
        }
    }

    // a solve with a tile in the bag on a fresh solver, and the endgames it
    // searched
    fn peg_solve_counted(
        gc: &game_config::GameConfig,
        kwg: &kwg::Kwg<kwg::Node24>,
        arm_tables: &anagrams::ArmTables,
        board: &[u8],
        mover_rack: &[u8],
        unseen_tally: &[u8],
        limits: &super::Limits,
    ) -> (Solved, u64) {
        let mut egs = EndgameSolver::<kwg::Node24, kwg::Node24>::new(gc, kwg, arm_tables).unwrap();
        let endgames = std::rc::Rc::new(std::cell::Cell::new(0u64));
        let heard = endgames.clone();
        let listener = move |p: &super::Progress| {
            if let Some(n) = p.endgames {
                heard.set(heard.get().max(n + 1));
            }
            Ok(())
        };
        let result = egs
            .listening(listener, |egs| {
                egs.solve_peg_one_in_bag_full(
                    0,
                    board,
                    mover_rack,
                    unseen_tally,
                    equity::Equity::ZERO,
                    limits,
                )
            })
            .unwrap()
            .expect("both configs solve one in the bag");
        (result, endgames.get())
    }

    #[test]
    #[inline]
    fn a_deepening_peg_search_answers_as_an_unlimited_one() {
        let kwg_bytes = tiny_kwg_bytes();
        let kwg = kwg::kbwg_from_bytes_alloc(kwg_bytes.to_vec(), false).unwrap();
        // limits that deepen the search but never stop it
        let deepening = [
            super::Limits {
                time: Some(std::time::Duration::from_secs(3600)),
                states: None,
                memory: None,
                clock: None,
            },
            super::Limits {
                time: None,
                states: Some(usize::MAX),
                memory: None,
                clock: None,
            },
        ];
        let tiles = [1u8, 2, 8, 20]; // A B H T
        let mut board = empty_board();
        put_word(&mut board, 7, 6, &[1, 20]);
        let mut more_endgames = 0;
        // the committed model, and the clairvoyant one where exchanges are legal
        for gc in [
            game_config::make_english_game_config(),
            game_config::make_exchange_test_game_config(),
        ] {
            let arm_tables = anagrams::ArmTables::build(&kwg, &gc, false).unwrap();
            let mut unseen_tally = vec![0u8; gc.alphabet().len() as usize];
            unseen_tally[8] = 1; // H
            unseen_tally[20] = 1; // T
            for &x in &tiles {
                for &y in &tiles {
                    let mover_rack = [x, y];
                    let solve = |limits: &super::Limits| {
                        peg_solve_counted(
                            &gc,
                            &kwg,
                            &arm_tables,
                            &board,
                            &mover_rack,
                            &unseen_tally,
                            limits,
                        )
                    };
                    let (full, full_endgames) = solve(&super::Limits::NONE);
                    assert_eq!(full.depth, i8::MAX, "one pass, each endgame to its end");
                    for limits in &deepening {
                        let (got, endgames) = solve(limits);
                        assert!(got.exact, "a search no limit stopped is exact");
                        assert!((1..i8::MAX).contains(&got.depth), "{}", got.depth);
                        assert_eq!(got.hypotheses, full.hypotheses);
                        assert_eq!(got.win_pct, full.win_pct);
                        assert_eq!(got.expected_margin, full.expected_margin);
                        assert!(
                            same_peg_move(&got.best_move, &full.best_move),
                            "a deepening search breaks ties as an unlimited one"
                        );
                        // and values every move as an unlimited one, in order
                        assert_eq!(got.moves.len(), full.moves.len());
                        for (a, b) in got.moves.iter().zip(&full.moves) {
                            assert!(same_peg_move(&Some(a.play.clone()), &Some(b.play.clone())));
                            assert_eq!(a.win_pct, b.win_pct);
                            assert_eq!(a.expected_margin, b.expected_margin);
                            assert_eq!(a.hypotheses, b.hypotheses);
                            assert_eq!(a.exact, b.exact);
                        }
                        if endgames > full_endgames {
                            more_endgames += 1;
                        }
                    }
                }
            }
        }
        assert!(
            more_endgames > 0,
            "some deepening search searched its endgames again a pass deeper"
        );
    }

    #[test]
    #[inline]
    fn a_peg_search_whose_lane_pool_has_no_room_answers_as_one_with_room() {
        let kwg_bytes = tiny_kwg_bytes();
        let kwg = kwg::kbwg_from_bytes_alloc(kwg_bytes.to_vec(), false).unwrap();
        let gc = game_config::make_english_game_config();
        let arm_tables = anagrams::ArmTables::build(&kwg, &gc, false).unwrap();
        // a limit no solve reaches, with room for the root's lists (a pass and
        // some places, each list a word a move) but its pool's share (one over
        // POOL_SHARE) a sliver of it
        let no_room = super::Limits {
            time: None,
            states: None,
            memory: Some(super::MemoryLimit {
                bytes: super::POOL_SHARE << 13,
                held: || 0,
                reserved: None,
            }),
            clock: None,
        };
        let tiles = [1u8, 2, 8, 20]; // A B H T
        let mut board = empty_board();
        put_word(&mut board, 7, 6, &[1, 20]);
        let mut unseen_tally = vec![0u8; gc.alphabet().len() as usize];
        unseen_tally[8] = 1; // H
        unseen_tally[20] = 1; // T
        for &x in &tiles {
            for &y in &tiles {
                let mover_rack = [x, y];
                let solve = |limits: &super::Limits| {
                    peg_solve_counted(
                        &gc,
                        &kwg,
                        &arm_tables,
                        &board,
                        &mover_rack,
                        &unseen_tally,
                        limits,
                    )
                };
                let (full, _) = solve(&super::Limits::NONE);
                let (tight, _) = solve(&no_room);
                assert_eq!(tight.hypotheses, full.hypotheses);
                assert_eq!(tight.win_pct, full.win_pct);
                assert_eq!(tight.expected_margin, full.expected_margin);
                assert!(same_peg_move(&tight.best_move, &full.best_move));
            }
        }
    }

    #[test]
    #[inline]
    fn a_full_lane_pool_lets_go_of_its_lists_and_keeps_their_buffers() {
        let mut pool = super::LanePool::new();
        pool.deal = super::Deal {
            pool: vec![1, 2, 8],
            mover: vec![1],
            budget: usize::MAX,
        };
        pool.moves.reserve(64);
        pool.lists.push([0, 1, 0]);
        pool.needs.push(1);
        pool.ends.push(1);
        pool.entries.push(0);
        pool.known.push(super::KeyTable::default());
        assert!(pool.keys.insert(7, 0));
        assert!(pool.contents.insert(8, 0));
        assert!(pool.known[0].insert(9, 0));
        let held = pool.bytes();
        pool.let_go(true);
        // every list let go, each buffer kept for the lists made again
        assert_eq!(pool.bytes(), held);
        assert!(pool.moves.is_empty() && pool.lists.is_empty());
        assert!(pool.needs.is_empty() && pool.ends.is_empty() && pool.entries.is_empty());
        assert_eq!(
            (pool.keys.get(7), pool.contents.get(8), pool.known[0].get(9)),
            (None, None, None)
        );
        assert!(pool.keys.insert(7, 3));
        assert_eq!(pool.keys.get(7), Some(3));
        // past the budget they are freed, as they are when not kept
        pool.deal.budget = held - 1;
        pool.let_go(true);
        assert_eq!(pool.bytes(), 0);
        assert!(pool.keys.insert(7, 3));
        pool.deal.budget = usize::MAX;
        pool.let_go(false);
        assert_eq!(pool.bytes(), 0);
        // a walk the budget refuses finds the pool full, and one the memory
        // limit refuses finds no memory, whatever the budget
        let limit = |bytes| {
            Some(super::MemoryLimit {
                bytes,
                held: || 0,
                reserved: None,
            })
        };
        let plays = [movegen::ValuedMove {
            equity: equity::Equity::ZERO,
            play: movegen::Play::Exchange {
                tiles: [][..].into(),
            },
        }];
        let room = |pool: &super::LanePool, bytes| pool.room_for(limit(bytes), &plays, (1, 1));
        assert!(room(&pool, 1 << 40).is_ok());
        pool.deal.budget = 0;
        assert!(room(&pool, 1 << 40).err() == Some(super::PoolRoom::Full));
        assert!(room(&pool, 1).err() == Some(super::PoolRoom::NoMemory));
        pool.deal.budget = usize::MAX;
        assert!(room(&pool, 1).err() == Some(super::PoolRoom::NoMemory));
    }

    #[test]
    #[inline]
    fn a_peg_search_out_of_time_values_one_move_and_answers_with_it() {
        let kwg_bytes = tiny_kwg_bytes();
        let kwg = kwg::kbwg_from_bytes_alloc(kwg_bytes.to_vec(), false).unwrap();
        let gc = game_config::make_english_game_config();
        let arm_tables = anagrams::ArmTables::build(&kwg, &gc, false).unwrap();
        let no_time = super::Limits {
            time: Some(std::time::Duration::ZERO),
            states: None,
            memory: None,
            clock: None,
        };
        let tiles = [1u8, 2, 8, 20]; // A B H T
        let mut board = empty_board();
        put_word(&mut board, 7, 6, &[1, 20]);
        let mut unseen_tally = vec![0u8; gc.alphabet().len() as usize];
        unseen_tally[8] = 1; // H
        unseen_tally[20] = 1; // T
        for &x in &tiles {
            for &y in &tiles {
                // every such rack can play onto AT (BAT, HAT, TAT, TA)
                let mover_rack = [x, y];
                let (got, endgames) = peg_solve_counted(
                    &gc,
                    &kwg,
                    &arm_tables,
                    &board,
                    &mover_rack,
                    &unseen_tally,
                    &no_time,
                );
                assert_eq!(
                    endgames, 2,
                    "out of time, the search values its first move alone, an endgame a tile"
                );
                assert!(!got.exact, "the time stopped it short of every move");
                assert_eq!(got.depth, 0, "no pass valued every move");
                assert!(matches!(got.best_move, Some(super::PegMove::Place(_))));
                let weighted: Vec<(u32, equity::Equity)> =
                    got.hypotheses.iter().map(|&(_, w, v)| (w, v)).collect();
                assert_eq!(
                    super::peg_aggregate(&weighted).unwrap(),
                    (got.win_pct, got.expected_margin)
                );
            }
        }
    }

    #[test]
    #[inline]
    fn a_peg_search_a_state_limit_stops_answers_with_one_moves_values() {
        let kwg_bytes = tiny_kwg_bytes();
        let kwg = kwg::kbwg_from_bytes_alloc(kwg_bytes.to_vec(), false).unwrap();
        let mut board = empty_board();
        put_word(&mut board, 7, 6, &[1, 20]);
        let mut stopped = 0;
        let english = game_config::make_english_game_config();
        let exchange_test = game_config::make_exchange_test_game_config();
        for (gc, mover_rack) in [
            (&english, [1u8, 1]),     // A A
            (&exchange_test, [1, 1]), // A A
            (&exchange_test, [2, 8]), // B H
        ] {
            let arm_tables = anagrams::ArmTables::build(&kwg, gc, false).unwrap();
            let mut unseen_tally = vec![0u8; gc.alphabet().len() as usize];
            unseen_tally[8] = 1; // H
            unseen_tally[20] = 1; // T
            let solve = |limits: &super::Limits| {
                peg_solve_counted(
                    gc,
                    &kwg,
                    &arm_tables,
                    &board,
                    &mover_rack,
                    &unseen_tally,
                    limits,
                )
                .0
            };
            let full = solve(&super::Limits::NONE);
            // a memory limit stops it before the first move is valued: none
            // yet
            let none = solve(&super::Limits {
                time: None,
                states: None,
                memory: Some(super::MemoryLimit {
                    bytes: usize::MAX / 2,
                    held: || usize::MAX,
                    reserved: None,
                }),
                clock: None,
            });
            assert!(none.hypotheses.is_empty() && !none.exact && none.depth == 0);
            assert!(none.best_move.is_none());
            // stopped at every point of its passes, the first too
            let mut states = 1;
            while states < 4096 {
                let got = solve(&super::Limits {
                    time: None,
                    states: Some(states),
                    memory: None,
                    clock: None,
                });
                if got.hypotheses.is_empty() {
                    // stopped before its first pass valued every tile
                    assert!(!got.exact && got.depth == 0 && got.best_move.is_none());
                    stopped += 1;
                    states = states * 3 / 2 + 1;
                    continue;
                }
                let weighted: Vec<(u32, equity::Equity)> =
                    got.hypotheses.iter().map(|&(_, w, v)| (w, v)).collect();
                assert_eq!(got.hypotheses.len(), full.hypotheses.len());
                assert!(got.moves.len() <= full.moves.len(), "each move once");
                assert_eq!(
                    super::peg_aggregate(&weighted).unwrap(),
                    (got.win_pct, got.expected_margin),
                    "the answer's figures are those of its tiles' values"
                );
                assert_eq!(got.best_move.is_some(), full.best_move.is_some());
                if got.exact {
                    assert_eq!(got.hypotheses, full.hypotheses);
                    assert!(same_peg_move(&got.best_move, &full.best_move));
                } else {
                    stopped += 1;
                }
                states = states * 3 / 2 + 1;
            }
        }
        assert!(stopped > 0, "some state limit stopped a search");
    }

    // a position after the committed model's pass, valued in solve after solve
    // that each go on where a limit stopped the last (each a new solver, given
    // the table and the pass line kept, stopped once the table holds 5
    // positions more), is valued as one unlimited solve values it
    #[test]
    #[inline]
    fn a_pass_line_resumed_values_as_one_search() {
        let gc = game_config::make_english_game_config();
        let kwg = kwg::kbwg_from_bytes_alloc(tiny_kwg_bytes().to_vec(), false).unwrap();
        let arm_tables = anagrams::ArmTables::build(&kwg, &gc, false).unwrap();
        let klv = klv::kbwg_from_bytes_alloc(klv::EMPTY_KLV_BYTES.to_vec()).unwrap();
        let tiles = [1u8, 2, 8, 20]; // A B H T
        let mut board = empty_board();
        put_word(&mut board, 7, 6, &[1, 20]);
        let mut resumed = 0;
        for &x in &tiles {
            for &y in &tiles {
                for (bag_tile, other) in [(8u8, 20u8), (20, 8)] {
                    // seat 1 to move after seat 0 passed, the bag holding bag_tile
                    let racks = [vec![x, y], vec![other]];
                    let value = |line: Option<&mut super::PassLine>,
                                 kept: Option<super::KeptTable>,
                                 states: Option<usize>| {
                        let mut egs =
                            EndgameSolver::<kwg::Node24, kwg::Node24>::new(&gc, &kwg, &arm_tables)
                                .unwrap();
                        if let Some(table) = kept {
                            assert!(egs.give_table(table).is_ok());
                        }
                        let limits = super::Limits {
                            time: None,
                            states: states.map(|more| egs.work_buffer.table.held + more),
                            memory: None,
                            clock: None,
                        };
                        let limits = super::BagLimits::new(&limits, None);
                        limits.may_stop.set(true);
                        let mut mg = movegen::KurniaMoveGenerator::new(&gc);
                        let mut search = super::OneInBag {
                            gc: &gc,
                            kwg: &kwg,
                            arm_tables: &arm_tables,
                            klv: &klv,
                            sub: &mut egs,
                            mg: &mut mg,
                            memo: &mut Default::default(),
                            limits: &limits,
                            exact: true,
                            line,
                            placed: &mut Default::default(),
                        };
                        let got = EndgameSolver::one_in_bag_minimax(
                            &mut search,
                            &board,
                            &racks,
                            super::OneInBagTurn {
                                mover: 1,
                                bag_tile,
                                passes: 1,
                                zeros: 1,
                                answering: true,
                            },
                        );
                        let got = got
                            .map(|v| (v, search.exact))
                            .map_err(|_| limits.stopped.get());
                        (got, egs.take_table())
                    };
                    let (full, _) = value(None, None, None);
                    let full = full.unwrap();
                    let mut line = super::PassLine::default();
                    let mut kept = None;
                    for step in 1.. {
                        assert!(step < 10000, "the steps never ended");
                        let (got, table) = value(Some(&mut line), kept.take(), Some(5));
                        kept = Some(table);
                        match got {
                            Ok(got) => {
                                assert_eq!(got, full, "{racks:?} with {bag_tile} in the bag");
                                resumed += usize::from(step > 1);
                                break;
                            }
                            Err(stopped) => assert!(stopped, "only a limit stops it"),
                        }
                    }
                }
            }
        }
        assert!(resumed > 0, "no search was resumed");
    }

    // a search with a tile in the bag resumed solve after solve, as a wasm32
    // page steps one: each step a new solver, given the table and the progress
    // kept, stopped once the table holds 30 positions more. Its depth never
    // falls, from depth 1 on its answer changes only as the depth rises, and it
    // ends with the answer of one unlimited solve
    #[test]
    #[inline]
    fn a_peg_search_resumed_in_steps_ends_as_one_solve() {
        let kwg = kwg::kbwg_from_bytes_alloc(tiny_kwg_bytes().to_vec(), false).unwrap();
        let tiles = [1u8, 2, 8, 20]; // A B H T
        let mut board = empty_board();
        put_word(&mut board, 7, 6, &[1, 20]);
        let mut stepped = 0;
        // the committed model, and the clairvoyant one where exchanges are legal
        for gc in [
            game_config::make_english_game_config(),
            game_config::make_exchange_test_game_config(),
        ] {
            let arm_tables = anagrams::ArmTables::build(&kwg, &gc, false).unwrap();
            let mut unseen_tally = vec![0u8; gc.alphabet().len() as usize];
            unseen_tally[8] = 1; // H
            unseen_tally[20] = 1; // T
            for &x in &tiles {
                for &y in &tiles {
                    let mover_rack = [x, y];
                    let full =
                        EndgameSolver::<kwg::Node24, kwg::Node24>::new(&gc, &kwg, &arm_tables)
                            .unwrap()
                            .solve_peg_one_in_bag_full(
                                0,
                                &board,
                                &mover_rack,
                                &unseen_tally,
                                equity::Equity::ZERO,
                                &super::Limits::NONE,
                            )
                            .unwrap()
                            .expect("both configs solve one in the bag");
                    let mut progress = super::PegProgress::new(
                        0,
                        &board,
                        &mover_rack,
                        &unseen_tally,
                        equity::Equity::ZERO,
                        [0, 0],
                    );
                    let mut kept = None;
                    let mut last: Option<Solved> = None;
                    for step in 1.. {
                        assert!(step < 10000, "the steps never ended");
                        let mut egs =
                            EndgameSolver::<kwg::Node24, kwg::Node24>::new(&gc, &kwg, &arm_tables)
                                .unwrap();
                        if let Some(table) = kept.take() {
                            assert!(egs.give_table(table).is_ok());
                        }
                        let limits = super::Limits {
                            time: None,
                            states: Some(egs.work_buffer.table.held + 30),
                            memory: None,
                            clock: None,
                        };
                        let got = egs
                            .resume_peg_one_in_bag(&mut progress, &limits)
                            .unwrap()
                            .expect("both configs solve one in the bag");
                        let got = solved(&progress, got);
                        // resumed with no room for a position more, it answers
                        // as it did: a search begun again would answer from
                        // depth 0
                        if got.depth > 0 {
                            let again = egs
                                .resume_peg_one_in_bag(
                                    &mut progress,
                                    &super::Limits {
                                        time: None,
                                        states: Some(egs.work_buffer.table.held),
                                        memory: None,
                                        clock: None,
                                    },
                                )
                                .unwrap()
                                .expect("both configs solve one in the bag");
                            let again = solved(&progress, again);
                            assert_eq!(
                                (again.depth, again.exact, &again.hypotheses),
                                (got.depth, got.exact, &got.hypotheses),
                                "step {step}"
                            );
                            assert!(same_peg_move(&again.best_move, &got.best_move));
                        }
                        kept = Some(egs.take_table());
                        if let Some(last) = &last {
                            assert!(
                                got.depth >= last.depth,
                                "step {step}: depth {} after {}",
                                got.depth,
                                last.depth
                            );
                            if got.depth == last.depth && got.depth > 0 {
                                assert_eq!(got.hypotheses, last.hypotheses, "step {step}");
                                assert!(same_peg_move(&got.best_move, &last.best_move));
                            }
                        }
                        if got.exact {
                            assert_eq!(got.hypotheses, full.hypotheses);
                            assert_eq!(
                                (got.win_pct, got.expected_margin),
                                (full.win_pct, full.expected_margin)
                            );
                            assert!(same_peg_move(&got.best_move, &full.best_move));
                            stepped += usize::from(step > 1);
                            break;
                        }
                        last = Some(got);
                    }
                }
            }
        }
        assert!(stepped > 0, "no search took more than one step");
    }

    // a rack's slots, found once, give each word the places rack_places gives
    // it, by first slots and by last: racks of one to seven tiles (many alike),
    // each word some of their tiles in any order, a blank as any letter, with
    // squares played through between them
    #[test]
    #[inline]
    fn a_racks_slots_place_as_rack_places_does() {
        let mut state = 1u64;
        let mut checked = 0;
        for _ in 0..2000 {
            let len = 1 + (splitmix64(&mut state) % 7) as usize;
            let mut rack = (0..len)
                .map(|_| (splitmix64(&mut state) % 6) as u8)
                .collect::<Vec<_>>();
            rack.sort_unstable();
            let slots = super::RackSlots::of(&rack);
            let mut tiles = rack.clone();
            for i in (1..tiles.len()).rev() {
                let j = (splitmix64(&mut state) % (i as u64 + 1)) as usize;
                tiles.swap(i, j);
            }
            let take = 1 + (splitmix64(&mut state) % tiles.len() as u64) as usize;
            let mut word = Vec::new();
            for &tile in &tiles[..take] {
                if splitmix64(&mut state).is_multiple_of(3) {
                    word.push(0);
                }
                word.push(if tile == 0 {
                    0x80 | (1 + (splitmix64(&mut state) % 26) as u8)
                } else {
                    tile
                });
            }
            for last in [false, true] {
                assert_eq!(
                    slots.places(&word, last),
                    super::rack_places(&rack, &word, last),
                    "{rack:?} {word:?} {last}"
                );
                checked += 1;
            }
        }
        assert_eq!(checked, 4000);
    }

    // a solve counts its nodes, every one its listener heard of
    #[test]
    #[inline]
    fn a_solve_counts_its_nodes() {
        let kwg = kwg::kbwg_from_bytes_alloc(tiny_kwg_bytes().to_vec(), false).unwrap();
        let gc = game_config::make_english_game_config();
        let arm_tables = anagrams::ArmTables::build(&kwg, &gc, false).unwrap();
        let mut board = empty_board();
        put_word(&mut board, 7, 6, &[1, 20]);
        let mut egs =
            EndgameSolver::<kwg::Node24, kwg::Node24>::new(&gc, &kwg, &arm_tables).unwrap();
        let heard = std::rc::Rc::new(std::cell::Cell::new(0u64));
        let counter = heard.clone();
        let listener = move |p: &super::Progress| {
            if p.value.is_none() {
                counter.set(counter.get() + 1);
            }
            Ok(())
        };
        egs.listening(listener, |egs| {
            egs.init(&board, [&[1, 2, 8], &[1, 20]]); // A B H, A T
            egs.solve(0).unwrap();
            assert!(egs.searched() > 1, "{} nodes", egs.searched());
            assert_eq!(egs.searched(), heard.get());
            heard.set(0);
            egs.init(&board, [&[1, 2, 8], &[1, 20]]);
            egs.solve(0).unwrap();
            assert_eq!(egs.searched(), heard.get());
        });
    }

    // a search with a tile in the bag counts, over every step, the endgames it
    // searched and their nodes, each node one its listener heard of; the
    // committed model also says how far its pass under way has got, and that it
    // valued every move once its answer is exact
    #[test]
    #[inline]
    fn a_peg_search_counts_its_endgames_and_nodes_over_its_steps() {
        let kwg = kwg::kbwg_from_bytes_alloc(tiny_kwg_bytes().to_vec(), false).unwrap();
        let mut board = empty_board();
        put_word(&mut board, 7, 6, &[1, 20]);
        for (gc, committed) in [
            (game_config::make_english_game_config(), true),
            (game_config::make_exchange_test_game_config(), false),
        ] {
            let arm_tables = anagrams::ArmTables::build(&kwg, &gc, false).unwrap();
            let mut unseen_tally = vec![0u8; gc.alphabet().len() as usize];
            unseen_tally[8] = 1; // H
            unseen_tally[20] = 1; // T
            let mut progress = super::PegProgress::new(
                0,
                &board,
                &[1, 2], // A B
                &unseen_tally,
                equity::Equity::ZERO,
                [0, 0],
            );
            assert_eq!(progress.searched(), (0, 0));
            assert!(progress.moves().is_none());
            let heard = std::rc::Rc::new(std::cell::Cell::new(0u64));
            let mut kept = None;
            let mut partway = false;
            for step in 1.. {
                assert!(step < 10000, "the steps never ended");
                let mut egs =
                    EndgameSolver::<kwg::Node24, kwg::Node24>::new(&gc, &kwg, &arm_tables).unwrap();
                if let Some(table) = kept.take() {
                    assert!(egs.give_table(table).is_ok());
                }
                let counter = heard.clone();
                let listener = move |p: &super::Progress| {
                    if p.value.is_none() {
                        counter.set(counter.get() + 1);
                    }
                    Ok(())
                };
                let (endgames_before, _) = progress.searched();
                let limits = super::Limits {
                    time: None,
                    states: Some(egs.work_buffer.table.held + 30),
                    memory: None,
                    clock: None,
                };
                let got = egs
                    .listening(listener, |egs| {
                        egs.resume_peg_one_in_bag(&mut progress, &limits)
                    })
                    .unwrap()
                    .expect("both configs solve one in the bag");
                kept = Some(egs.take_table());
                let (endgames, nodes) = progress.searched();
                assert!(
                    endgames > endgames_before,
                    "step {step}: no endgame searched"
                );
                assert_eq!(nodes, heard.get(), "step {step}");
                match progress.moves() {
                    Some((valued, of)) => {
                        assert!(committed, "step {step}: the clairvoyant model has moves");
                        assert!(valued <= of && of > 0, "step {step}: {valued} of {of}");
                        if got.exact {
                            assert_eq!(valued, of, "step {step}");
                        }
                        partway |= valued < of;
                    }
                    None => assert!(!committed, "step {step}: the committed model has none"),
                }
                if got.exact {
                    assert_eq!(partway, committed, "a step stopped partway through a pass");
                    break;
                }
            }
        }
    }

    #[test]
    #[inline]
    fn the_endgames_of_a_search_with_a_tile_in_the_bag_share_the_solvers_table() {
        let kwg_bytes = tiny_kwg_bytes();
        let kwg = kwg::kbwg_from_bytes_alloc(kwg_bytes.to_vec(), false).unwrap();
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
                EndgameSolver::<kwg::Node24, kwg::Node24>::new(&gc, &kwg, &arm_tables).unwrap();
            let nodes = std::rc::Rc::new(std::cell::Cell::new(0u64));
            let heard = || {
                let heard = nodes.clone();
                move |p: &super::Progress| {
                    if p.value.is_none() {
                        heard.set(heard.get() + 1);
                    }
                    Ok(())
                }
            };
            let mut unseen_tally = vec![0u8; gc.alphabet().len() as usize];
            unseen_tally[8] = 1; // H
            unseen_tally[20] = 1; // T
            let solve = |egs: &mut EndgameSolver<kwg::Node24, kwg::Node24>| {
                nodes.set(0);
                let result = egs
                    .listening(heard(), |egs| {
                        egs.solve_peg_one_in_bag_full(
                            0,
                            &board,
                            &[1, 2], // A B
                            &unseen_tally,
                            equity::Equity::ZERO,
                            &super::Limits::NONE,
                        )
                    })
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
    fn swap_tables_refuses_a_solver_of_another_game() {
        // swap_tables lends the table to a solver of the same game_config, kwg,
        // and arm_tables and takes it back; any other is refused, and nothing
        // moves
        let gc = game_config::make_english_game_config();
        let other_gc = game_config::make_english_game_config();
        let kwg_bytes = tiny_kwg_bytes();
        let kwg = kwg::kbwg_from_bytes_alloc(kwg_bytes.to_vec(), false).unwrap();
        let other_kwg = kwg::kbwg_from_bytes_alloc(kwg_bytes.to_vec(), false).unwrap();
        let arm_tables = anagrams::ArmTables::build(&kwg, &gc, false).unwrap();
        let other_arm_tables = anagrams::ArmTables::build(&kwg, &gc, false).unwrap();
        let mut egs =
            EndgameSolver::<kwg::Node24, kwg::Node24>::new(&gc, &kwg, &arm_tables).unwrap();
        let mut board = empty_board();
        put_word(&mut board, 7, 6, &[1, 20]); // AT
        egs.init(&board, [&[1, 2], &[8, 20]]); // A B, H T
        egs.solve(0).unwrap();
        let held = egs.work_buffer.table.held;
        assert!(held > 0, "the solve left nothing in the table");
        for (name, mut other) in [
            (
                "another game_config",
                EndgameSolver::<kwg::Node24, kwg::Node24>::new(&other_gc, &kwg, &arm_tables)
                    .unwrap(),
            ),
            (
                "another kwg",
                EndgameSolver::<kwg::Node24, kwg::Node24>::new(&gc, &other_kwg, &arm_tables)
                    .unwrap(),
            ),
            (
                "other arm_tables",
                EndgameSolver::<kwg::Node24, kwg::Node24>::new(&gc, &kwg, &other_arm_tables)
                    .unwrap(),
            ),
        ] {
            assert!(
                egs.swap_tables(&mut other).is_err(),
                "{name} was lent the table"
            );
            assert_eq!(egs.work_buffer.table.held, held, "{name}: the table moved");
            assert_eq!(other.work_buffer.table.held, 0, "{name}: the table moved");
        }
        let mut same =
            EndgameSolver::<kwg::Node24, kwg::Node24>::new(&gc, &kwg, &arm_tables).unwrap();
        egs.swap_tables(&mut same).unwrap();
        assert_eq!(same.work_buffer.table.held, held);
        assert_eq!(egs.work_buffer.table.held, 0);
        egs.swap_tables(&mut same).unwrap();
        assert_eq!(egs.work_buffer.table.held, held);
    }

    #[test]
    #[inline]
    fn a_taken_table_goes_back_only_to_a_solver_of_the_same_game() {
        // take_table leaves an empty table and keeps the full one apart; a
        // solver of another game_config, kwg, or arm_tables is refused it,
        // whole, and one of the same takes it and answers on it as before
        let gc = game_config::make_english_game_config();
        let other_gc = game_config::make_english_game_config();
        let kwg_bytes = tiny_kwg_bytes();
        let kwg = kwg::kbwg_from_bytes_alloc(kwg_bytes.to_vec(), false).unwrap();
        let other_kwg = kwg::kbwg_from_bytes_alloc(kwg_bytes.to_vec(), false).unwrap();
        let arm_tables = anagrams::ArmTables::build(&kwg, &gc, false).unwrap();
        let other_arm_tables = anagrams::ArmTables::build(&kwg, &gc, false).unwrap();
        let mut egs =
            EndgameSolver::<kwg::Node24, kwg::Node24>::new(&gc, &kwg, &arm_tables).unwrap();
        let mut board = empty_board();
        put_word(&mut board, 7, 6, &[1, 20]); // AT
        egs.init(&board, [&[1, 2], &[8, 20]]); // A B, H T
        let value = egs.solve(0).unwrap();
        let held = egs.work_buffer.table.held;
        assert!(held > 0, "the solve left nothing in the table");
        let mut kept = egs.take_table();
        assert_eq!(egs.work_buffer.table.held, 0, "the table was not taken");
        for (name, mut other) in [
            (
                "another game_config",
                EndgameSolver::<kwg::Node24, kwg::Node24>::new(&other_gc, &kwg, &arm_tables)
                    .unwrap(),
            ),
            (
                "another kwg",
                EndgameSolver::<kwg::Node24, kwg::Node24>::new(&gc, &other_kwg, &arm_tables)
                    .unwrap(),
            ),
            (
                "other arm_tables",
                EndgameSolver::<kwg::Node24, kwg::Node24>::new(&gc, &kwg, &other_arm_tables)
                    .unwrap(),
            ),
        ] {
            kept = other.give_table(kept).expect_err(name);
            assert_eq!(
                other.work_buffer.table.held, 0,
                "{name} was given the table"
            );
            assert_eq!(kept.table.held, held, "{name}: the table came back short");
        }
        let mut same =
            EndgameSolver::<kwg::Node24, kwg::Node24>::new(&gc, &kwg, &arm_tables).unwrap();
        assert!(
            same.give_table(kept).is_ok(),
            "a solver of the same game was refused"
        );
        assert_eq!(same.work_buffer.table.held, held);
        same.init(&board, [&[1, 2], &[8, 20]]);
        assert_eq!(same.solve(0).unwrap(), value);
    }

    #[test]
    #[inline]
    fn a_principal_variation_plays_out_to_the_value_it_claims() {
        let gc = game_config::make_english_game_config();
        let kwg_bytes = tiny_kwg_bytes();
        let kwg = kwg::kbwg_from_bytes_alloc(kwg_bytes.to_vec(), false).unwrap();

        let mut ok = 0usize;
        let mut bad = 0usize;
        let mut pv: Vec<(equity::Equity, movegen::Play)> = Vec::new();
        let positions = all_positions();
        for (lean, arm_tables) in &both_arms(&kwg, &gc) {
            for (name, pos) in &positions {
                let mut egs =
                    EndgameSolver::<kwg::Node24, kwg::Node24>::new(&gc, &kwg, arm_tables).unwrap();
                egs.init(&pos.board, [&pos.racks[0][..], &pos.racks[1][..]]);
                let v = egs.solve(0).unwrap();
                egs.collect_pv(0, &mut pv).unwrap();

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
                            realized += sign * (score.raw() as f32);
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
                                realized += sign
                                    * (2 * a.scaled_rack_score(&racks[mover ^ 1]).raw()) as f32;
                                ended = true;
                            }
                            prev_pass = false;
                        }
                        movegen::Play::Exchange { .. } => {
                            if prev_pass {
                                realized += sign
                                    * (a.scaled_rack_score(&racks[mover ^ 1]).raw()
                                        - a.scaled_rack_score(&racks[mover]).raw())
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
        let kwg = kwg::kbwg_from_bytes_alloc(kwg_bytes.to_vec(), false).unwrap();
        let klv = klv::kbwg_from_bytes_alloc(klv::EMPTY_KLV_BYTES.to_vec()).unwrap();
        let mut mg = movegen::KurniaMoveGenerator::new(&gc);

        let mut illegal = 0usize;
        let mut oob = 0usize;
        let mut pv: Vec<(equity::Equity, movegen::Play)> = Vec::new();
        let positions = all_positions();
        for (lean, arm_tables) in &both_arms(&kwg, &gc) {
            for (name, pos) in &positions {
                let mut egs =
                    EndgameSolver::<kwg::Node24, kwg::Node24>::new(&gc, &kwg, arm_tables).unwrap();
                egs.init(&pos.board, [&pos.racks[0][..], &pos.racks[1][..]]);
                let v = egs.solve(0).unwrap();
                egs.collect_pv(0, &mut pv).unwrap();

                let a = gc.alphabet();
                let board_pts: i32 = pos.board.iter().map(|&t| a.scaled_score(t).raw()).sum();
                let rack_pts = a.scaled_rack_score(&pos.racks[0]).raw()
                    + a.scaled_rack_score(&pos.racks[1]).raw();
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
                            )
                            .unwrap();
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
    fn one_in_bag_forced_double_pass() {
        let gc = game_config::make_english_game_config();
        let kwg_bytes = tiny_kwg_bytes();
        let kwg = kwg::kbwg_from_bytes_alloc(kwg_bytes.to_vec(), false).unwrap();

        let board = empty_board();
        let racks = [vec![2u8, 2], vec![8u8, 20]];
        let arm_tables = anagrams::ArmTables::build(&kwg, &gc, false).unwrap();

        let mut egs =
            EndgameSolver::<kwg::Node24, kwg::Node24>::new(&gc, &kwg, &arm_tables).unwrap();
        egs.init(&board, [&racks[0][..], &racks[1][..]]);

        let v0 = egs.solve_one_in_bag(0, 2).unwrap();
        assert_eq!(
            v0,
            equity::Equity::new(-1000),
            "p0 forced-pass leftover should be -1000"
        );

        let mut egs =
            EndgameSolver::<kwg::Node24, kwg::Node24>::new(&gc, &kwg, &arm_tables).unwrap();
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
        let kwg = kwg::kbwg_from_bytes_alloc(kwg_bytes.to_vec(), false).unwrap();
        let arm_tables = anagrams::ArmTables::build(&kwg, &gc, false).unwrap();
        let mut egs =
            EndgameSolver::<kwg::Node24, kwg::Node24>::new(&gc, &kwg, &arm_tables).unwrap();
        egs.init_after_scoreless_turns(&empty_board(), racks, passes, zeros);
        let value = egs.solve(player_idx).unwrap();
        let mut pv = Vec::new();
        egs.collect_pv(player_idx, &mut pv).unwrap();
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
        // the phony counted a zero alone (no pass, one zero); B alone plays
        // nothing, AT plays out for 4 and twice the B: the line is the pass,
        // then AT, and the game goes on past the pass
        let (value, line) = line_after_scoreless_turns([&[1, 20], &[2]], 1, 0, 1);
        assert_eq!(value, equity::Equity::new(-10_000));
        assert_eq!(line, vec![vec![], vec![1, 20]]);
    }

    #[test]
    #[inline]
    fn after_a_pass_an_opponent_who_can_only_pass_ends_the_game() {
        // the game's last turn a pass (one pass, one zero), whether a pass or a
        // phony taken back where the config counts it as one: B, playing
        // nothing, passes, which answers it and ends the game, each losing
        // their rack (the A and the T are 2, the B 3)
        let (value, line) = line_after_scoreless_turns([&[1, 20], &[2]], 1, 1, 1);
        assert_eq!(value, equity::Equity::new(-1_000));
        assert_eq!(line, vec![Vec::<u8>::new()]);
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
        // with four, the last of them no pass, B's pass leaves AT to be played
        // out: as after the phony
        let (value, line) = line_after_scoreless_turns([&[1, 20], &[2]], 1, 0, 4);
        assert_eq!(value, equity::Equity::new(-10_000));
        assert_eq!(line, vec![vec![], vec![1, 20]]);
    }

    #[test]
    #[inline]
    fn a_forced_pass_uses_no_depth() {
        // the other side's Z fits no word, so its every turn is a forced pass:
        // a two-ply search sees the mover's two B plays (no word takes both) to
        // the end, the full value; a pass costing a ply would stop after the
        // first
        let gc = game_config::make_english_game_config();
        let kwg_bytes = tiny_kwg_bytes();
        let kwg = kwg::kbwg_from_bytes_alloc(kwg_bytes.to_vec(), false).unwrap();
        let arm_tables = anagrams::ArmTables::build(&kwg, &gc, false).unwrap();
        let mut board = empty_board();
        put_word(&mut board, 7, 6, &[1, 20]); // AT
        let racks: [&[u8]; 2] = [&[2, 2], &[26]]; // B B, Z
        let mut egs =
            EndgameSolver::<kwg::Node24, kwg::Node24>::new(&gc, &kwg, &arm_tables).unwrap();
        egs.init(&board, racks);
        let whole = egs.solve(0).unwrap();
        let mut fresh =
            EndgameSolver::<kwg::Node24, kwg::Node24>::new(&gc, &kwg, &arm_tables).unwrap();
        fresh.init(&board, racks);
        let two = fresh
            .root_search(0, 2, equity::Equity::NEG_INFINITY, equity::Equity::INFINITY)
            .unwrap();
        assert!(two.complete, "a line was cut short at two plies");
        assert_eq!(two.value, whole);
        // the principal variation goes past the pass to the play out
        let mut line = Vec::new();
        fresh.work_buffer.finished = 2;
        fresh.collect_pv(0, &mut line).unwrap();
        assert_eq!(line.len(), 3, "the line: a B, the pass, the other B");
    }

    #[test]
    #[inline]
    fn an_unlimited_solve_deepens_by_two() {
        // a solve with no limit, or a memory limit alone, finishes depth 1,
        // depth 2, and then every second depth (perhaps an odd one last); a
        // time or state limit makes it finish each depth; the two agree on the
        // value
        let gc = game_config::make_english_game_config();
        let kwg_bytes = tiny_kwg_bytes();
        let kwg = kwg::kbwg_from_bytes_alloc(kwg_bytes.to_vec(), false).unwrap();
        let arm_tables = anagrams::ArmTables::build(&kwg, &gc, false).unwrap();
        // limits never reached: memory alone, states, and time
        let memory = super::Limits {
            time: None,
            states: None,
            memory: Some(super::MemoryLimit {
                bytes: usize::MAX,
                held: || 0,
                reserved: None,
            }),
            clock: None,
        };
        let states = super::Limits {
            time: None,
            states: Some(usize::MAX),
            memory: None,
            clock: None,
        };
        let time = super::Limits {
            time: Some(std::time::Duration::from_secs(3600)),
            states: None,
            memory: None,
            clock: None,
        };
        let mut skipped = 0usize;
        for (name, pos) in all_positions() {
            for mover in 0..2u8 {
                let mut depths = Vec::new();
                let mut values = Vec::new();
                for limits in [&super::Limits::NONE, &memory, &states, &time] {
                    let mut egs =
                        EndgameSolver::<kwg::Node24, kwg::Node24>::new(&gc, &kwg, &arm_tables)
                            .unwrap();
                    let heard = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
                    let finished = heard.clone();
                    let listener = move |p: &super::Progress| {
                        if p.value.is_some() {
                            finished.borrow_mut().push(p.depth);
                        }
                        Ok(())
                    };
                    egs.init(&pos.board, [&pos.racks[0][..], &pos.racks[1][..]]);
                    let solved = egs.listening(listener, |egs| egs.solve_within(mover, limits));
                    values.push(solved.unwrap().value);
                    depths.push(heard.borrow().clone());
                }
                assert!(
                    values.iter().all(|&v| v == values[0]),
                    "[{name}] the values differ: {values:?}"
                );
                let free = &depths[0];
                assert_eq!(&depths[1], free, "[{name}] a memory limit alone as none");
                for every in &depths[2..] {
                    assert!(
                        every.iter().enumerate().all(|(i, &d)| d as usize == i + 1),
                        "[{name}] with a time or state limit, every depth: {every:?}"
                    );
                }
                for (i, pair) in free.windows(2).enumerate() {
                    let step = pair[1] - pair[0];
                    assert!(
                        if i == 0 {
                            pair == [1, 2]
                        } else {
                            step == 2 || (step == 1 && i + 2 == free.len())
                        },
                        "[{name}] with no limit: {free:?}"
                    );
                }
                skipped += usize::from(free.len() < depths[2].len());
            }
        }
        assert!(skipped > 0, "no solve went deep enough to skip a depth");
    }

    #[test]
    #[inline]
    fn a_memory_stop_answers_from_a_finished_depth() {
        // a memory limit alone skips the odd depths past the second, so a solve
        // it stops must answer from the deepest finished depth, not the one
        // before the stopped one. Here the memory reads past the limit from a
        // chosen look on: at a table growth, within a pass, or after one
        static LOOKS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        static FROM: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        use std::sync::atomic::Ordering::Relaxed;
        let gc = game_config::make_english_game_config();
        let kwg_bytes = tiny_kwg_bytes();
        let kwg = kwg::kbwg_from_bytes_alloc(kwg_bytes.to_vec(), false).unwrap();
        let arm_tables = anagrams::ArmTables::build(&kwg, &gc, false).unwrap();
        let limits = super::Limits {
            time: None,
            states: None,
            memory: Some(super::MemoryLimit {
                bytes: usize::MAX / 2,
                held: || {
                    if LOOKS.fetch_add(1, Relaxed) >= FROM.load(Relaxed) {
                        usize::MAX
                    } else {
                        0
                    }
                },
                reserved: None,
            }),
            clock: None,
        };
        // five-tile racks of a few letters (A B H S T), so passes last long
        // enough to be stopped within: from the first look, in a generation;
        // from the 64th, past the second depth
        let mut board = empty_board();
        put_word(&mut board, 7, 6, &[1, 20]);
        let racks = [vec![1u8, 1, 2, 8, 20], vec![1u8, 2, 8, 19, 20]];
        let mut stopped_late = 0usize;
        for from in [1usize, 64] {
            let mut egs =
                EndgameSolver::<kwg::Node24, kwg::Node24>::new(&gc, &kwg, &arm_tables).unwrap();
            let heard = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
            let finished = heard.clone();
            let listener = move |p: &super::Progress| {
                if let Some(value) = p.value {
                    finished.borrow_mut().push((p.depth, value));
                }
                Ok(())
            };
            egs.init(&board, [&racks[0][..], &racks[1][..]]);
            LOOKS.store(0, Relaxed);
            FROM.store(from, Relaxed);
            let solved = egs
                .listening(listener, |egs| egs.solve_within(0, &limits))
                .unwrap();
            if solved.exact {
                continue;
            }
            // the depth is the deepest that finished, and the value is that
            // depth's unless the stopped pass found a move worth more
            let heard = heard.borrow();
            let Some(&(depth, value)) = heard.last() else {
                assert!(
                    solved.out_of_memory_in_moves && solved.depth == 0 && solved.value.is_none(),
                    "from look {from}: stopped before a depth finished, in a generation"
                );
                continue;
            };
            assert!(
                solved.depth == depth
                    && (solved.value == Some(value)
                        || solved.partway.is_some_and(|stopped| stopped > depth)),
                "from look {from}: the answer is the deepest depth that finished: {heard:?}, \
                 answered {:?} at {} (partway {:?})",
                solved.value,
                solved.depth,
                solved.partway
            );
            stopped_late += usize::from(solved.depth >= 2);
        }
        assert!(
            stopped_late > 0,
            "no solve was stopped after its second depth"
        );
    }

    #[test]
    #[inline]
    fn stopping_in_the_second_pass_can_change_the_answer() {
        // a state limit that stops the second pass gives the first pass's best
        // and value, or another move at its two-ply value, which must beat the
        // best's; the line starts with the answer. Fresh solvers check the
        // two-ply values
        let gc = game_config::make_english_game_config();
        let kwg_bytes = tiny_kwg_bytes();
        let kwg = kwg::kbwg_from_bytes_alloc(kwg_bytes.to_vec(), false).unwrap();
        let arm_tables = anagrams::ArmTables::build(&kwg, &gc, false).unwrap();
        let fresh = |pos: &Position| {
            let mut egs =
                EndgameSolver::<kwg::Node24, kwg::Node24>::new(&gc, &kwg, &arm_tables).unwrap();
            egs.init(&pos.board, [&pos.racks[0][..], &pos.racks[1][..]]);
            egs
        };
        let states = |cap: usize| super::Limits {
            time: None,
            states: Some(cap),
            memory: None,
            clock: None,
        };
        // a placing move's two-ply value, on a fresh solver
        let at_two = |pos: &Position, mover: u8, play: &movegen::Play| {
            let mut egs = fresh(pos);
            egs.reset_position();
            egs.generate(0, mover, false).unwrap();
            let candidate = egs.work_buffer.lists[0]
                .iter()
                .find(|candidate| candidate.play == *play)
                .map(|candidate| (candidate.valuation, candidate.plays_out))
                .unwrap();
            if candidate.1 {
                return candidate.0;
            }
            egs.place(mover, play);
            let reply = egs
                .negamax_eval(
                    1,
                    mover ^ 1,
                    1,
                    equity::Equity::NEG_INFINITY,
                    equity::Equity::INFINITY,
                    super::Scoreless::Counted {
                        passes: 0,
                        zeros: 0,
                    },
                )
                .unwrap();
            candidate.0 - reply.value
        };
        let mut proved = 0usize;
        let mut kept = 0usize;
        let picked = [
            ("one-side-pass (board AT; p0=[B] p1=[B])", 0u8),
            ("rand#0", 1),
            ("rand#7", 1),
            ("rand#117", 1),
        ];
        for (name, pos) in all_positions() {
            for mover in 0..2u8 {
                if !picked.contains(&(&name[..], mover)) {
                    continue;
                }
                // the depth, the table's size, and the nodes after each pass
                // of a solve nothing stops (a state limit never reached, so
                // every depth)
                let mut egs = fresh(&pos);
                let heard = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
                let seen = heard.clone();
                let listener = move |p: &super::Progress| {
                    if p.value.is_some() {
                        seen.borrow_mut().push((p.depth, p.held, p.nodes));
                    }
                    Ok(())
                };
                let full = egs
                    .listening(listener, |egs| egs.solve_within(mover, &states(usize::MAX)))
                    .unwrap();
                let passes = heard.borrow().clone();
                assert!(
                    (passes.iter().enumerate()).all(|(i, &(depth, _, _))| depth as usize == i + 1),
                    "[{name}] every depth: {passes:?}"
                );
                let ((h1, nodes), h2) = match passes[..] {
                    [(_, h1, nodes), (_, h2, _), ..] => ((h1, nodes), h2),
                    _ => continue,
                };
                // the first pass finishes whatever the limit, even with the
                // table past it from the solve before
                let again = egs.solve_within(mover, &states(1)).unwrap();
                assert_eq!(again.value, full.value, "[{name}] the first pass finished");
                // stopped at the second pass's first node: the first pass's best
                let mut egs = fresh(&pos);
                let first = egs.solve_within(mover, &states(h1)).unwrap();
                assert_eq!(egs.searched(), nodes, "[{name}] no node past the pass");
                let mut line = Vec::new();
                egs.collect_pv(mover, &mut line).unwrap();
                let Some((_, best)) = line.first().cloned() else {
                    continue;
                };
                assert!(
                    !first.exact && first.depth == 1,
                    "[{name}] stopped after one ply"
                );
                for cap in h1 + 1..=h2 {
                    let mut egs = fresh(&pos);
                    let held = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
                    let saw = held.clone();
                    let listener = move |p: &super::Progress| {
                        if p.value.is_none() {
                            saw.borrow_mut().push(p.held);
                        }
                        Ok(())
                    };
                    let solved = egs
                        .listening(listener, |egs| egs.solve_within(mover, &states(cap)))
                        .unwrap();
                    // it stops at the first node to see the table at the limit
                    let held = held.borrow();
                    assert!(
                        held[..held.len() - 1].iter().all(|&h| h < cap),
                        "[{name}] cap {cap}: past the limit"
                    );
                    if solved.exact || solved.depth != 1 {
                        continue;
                    }
                    egs.collect_pv(mover, &mut line).unwrap();
                    let (value, play) = line.first().cloned().unwrap();
                    assert_eq!(
                        Some(value),
                        solved.value,
                        "[{name}] the line's value is the answer's"
                    );
                    if play == best {
                        assert_eq!(
                            (solved.value, solved.partway),
                            (first.value, None),
                            "[{name}] the first pass's answer"
                        );
                        kept += 1;
                        continue;
                    }
                    assert_eq!(
                        solved.partway,
                        Some(2),
                        "[{name}] cap {cap}: the pass stopped"
                    );
                    if let (movegen::Play::Place { .. }, movegen::Play::Place { .. }) =
                        (&play, &best)
                    {
                        let proven = at_two(&pos, mover, &play);
                        assert_eq!(
                            Some(proven),
                            solved.value,
                            "[{name}] cap {cap}: the move's value at two plies"
                        );
                        assert!(
                            at_two(&pos, mover, &best) < proven,
                            "[{name}] cap {cap}: above the first pass's best at two plies"
                        );
                        // with the window's floor at that value, the move is
                        // only bounded there, so not proved
                        let mut egs = fresh(&pos);
                        egs.root_search(
                            mover,
                            1,
                            equity::Equity::NEG_INFINITY,
                            equity::Equity::INFINITY,
                        )
                        .unwrap();
                        egs.work_buffer.root_best = egs.work_buffer.pass_best;
                        egs.work_buffer.proven = None;
                        egs.root_search(mover, 2, proven, equity::Equity::INFINITY)
                            .unwrap();
                        assert!(
                            (egs.work_buffer.proven).is_none_or(|(_, value)| value > proven),
                            "[{name}] cap {cap}: a bound proved"
                        );
                    }
                    proved += 1;
                }
            }
        }
        println!("stopped in the second pass: {proved} proved another move, {kept} kept the first");
        assert!(proved > 0 && kept > 0);
    }

    #[test]
    #[inline]
    fn a_proof_needs_the_best_first_and_an_open_window() {
        // after the first pass (its best, the table's move, searched first),
        // the second records a later move that beats it at two plies when the
        // window is open above; not when the first move searched is another
        // one, nor when the window closes just above the first move's value
        // (the later move then fails high: a bound, not a value)
        let gc = game_config::make_english_game_config();
        let kwg_bytes = tiny_kwg_bytes();
        let kwg = kwg::kbwg_from_bytes_alloc(kwg_bytes.to_vec(), false).unwrap();
        let arm_tables = anagrams::ArmTables::build(&kwg, &gc, false).unwrap();
        // the second pass, at two plies after a first, with the first pass's
        // best as given (or as found), in a window; what the pass recorded
        let second_pass = |pos: &Position,
                           mover: u8,
                           best: Option<u16>,
                           alpha: equity::Equity,
                           beta: equity::Equity| {
            let mut egs =
                EndgameSolver::<kwg::Node24, kwg::Node24>::new(&gc, &kwg, &arm_tables).unwrap();
            egs.init(&pos.board, [&pos.racks[0][..], &pos.racks[1][..]]);
            egs.root_search(
                mover,
                1,
                equity::Equity::NEG_INFINITY,
                equity::Equity::INFINITY,
            )
            .unwrap();
            let found = egs.work_buffer.pass_best;
            egs.work_buffer.root_best = best.unwrap_or(found);
            egs.work_buffer.proven = None;
            egs.root_search(mover, 2, alpha, beta).unwrap();
            (found, egs.work_buffer.proven)
        };
        let mut tried = 0usize;
        for (name, pos) in all_positions() {
            for mover in 0..2u8 {
                let (found, open) = second_pass(
                    &pos,
                    mover,
                    None,
                    equity::Equity::NEG_INFINITY,
                    equity::Equity::INFINITY,
                );
                let Some((later, value)) = open else {
                    continue;
                };
                assert_ne!(later, found, "[{name}] a later move, not the first");
                // the first move's two-ply value: a window around it keeps it
                // exact and the later move above it failing high
                let mut egs =
                    EndgameSolver::<kwg::Node24, kwg::Node24>::new(&gc, &kwg, &arm_tables).unwrap();
                egs.init(&pos.board, [&pos.racks[0][..], &pos.racks[1][..]]);
                egs.root_search(
                    mover,
                    1,
                    equity::Equity::NEG_INFINITY,
                    equity::Equity::INFINITY,
                )
                .unwrap();
                egs.work_buffer.root_best = super::NO_PLAY;
                let first = {
                    egs.reset_position();
                    egs.generate(0, mover, false).unwrap();
                    order_ply(&mut egs, 0, super::NO_PLAY);
                    let candidate = egs.work_buffer.lists[0]
                        .iter()
                        .find(|candidate| candidate.index == found)
                        .unwrap();
                    let (play, valuation, plays_out) = (
                        candidate.play.clone(),
                        candidate.valuation,
                        candidate.plays_out,
                    );
                    if plays_out {
                        valuation
                    } else {
                        egs.place(mover, &play);
                        let reply = egs
                            .negamax_eval(
                                1,
                                mover ^ 1,
                                1,
                                equity::Equity::NEG_INFINITY,
                                equity::Equity::INFINITY,
                                super::Scoreless::Counted {
                                    passes: 0,
                                    zeros: 0,
                                },
                            )
                            .unwrap();
                        valuation - reply.value
                    }
                };
                assert!(
                    value > first,
                    "[{name}] recorded above the first pass's best"
                );
                assert_eq!(
                    second_pass(
                        &pos,
                        mover,
                        Some(super::NO_PLAY),
                        equity::Equity::NEG_INFINITY,
                        equity::Equity::INFINITY
                    )
                    .1,
                    None,
                    "[{name}] the first move searched is not the first pass's best"
                );
                assert_eq!(
                    second_pass(
                        &pos,
                        mover,
                        None,
                        first - super::LEAST_STEP,
                        first + super::LEAST_STEP
                    )
                    .1,
                    None,
                    "[{name}] the later move fails high in a window closing above the first"
                );
                assert!(
                    second_pass(
                        &pos,
                        mover,
                        None,
                        first - super::LEAST_STEP,
                        equity::Equity::INFINITY
                    )
                    .1
                    .is_some_and(|(_, v)| v > first),
                    "[{name}] the window open above records a later move"
                );
                tried += 1;
            }
        }
        println!("a second pass proving a later move: {tried} positions tried");
        assert!(tried > 0);
    }

    // the exact value of a move from the position to solve, its reply searched
    // deeper than any line of these racks, on the solver given (a fresh one)
    #[inline]
    fn move_value(
        egs: &mut EndgameSolver<'_, kwg::Node24, kwg::Node24>,
        mover: u8,
        play: &movegen::Play,
    ) -> equity::Equity {
        egs.reset_position();
        egs.generate(0, mover, false).unwrap();
        let (valuation, after) = match play {
            movegen::Play::Exchange { .. } => (equity::Equity::ZERO, super::Scoreless::Answering),
            movegen::Play::Place { .. } => {
                let candidate = egs.work_buffer.lists[0]
                    .iter()
                    .find(|candidate| candidate.play == *play)
                    .unwrap();
                if candidate.plays_out {
                    return candidate.valuation;
                }
                let counted = super::Scoreless::Counted {
                    passes: 0,
                    zeros: 0,
                };
                (candidate.valuation, counted)
            }
        };
        egs.place(mover, play);
        let reply = egs
            .negamax_eval(
                1,
                mover ^ 1,
                20,
                equity::Equity::NEG_INFINITY,
                equity::Equity::INFINITY,
                after,
            )
            .unwrap();
        assert!(reply.complete, "a line was cut short");
        egs.unplace(mover, play);
        valuation - reply.value
    }

    #[test]
    #[inline]
    fn a_move_wins_when_its_value_beats_the_margin() {
        // with no limit, move_wins says of each first move, the pass among
        // them, whether it is worth more than a margin a point under or at its
        // value
        let gc = game_config::make_english_game_config();
        let kwg_bytes = tiny_kwg_bytes();
        let kwg = kwg::kbwg_from_bytes_alloc(kwg_bytes.to_vec(), false).unwrap();
        let arm_tables = anagrams::ArmTables::build(&kwg, &gc, false).unwrap();
        let point = equity::Equity::new(equity::SCALE);
        let mut asked = 0usize;
        for (name, pos) in all_positions() {
            for mover in 0..2u8 {
                let mut egs =
                    EndgameSolver::<kwg::Node24, kwg::Node24>::new(&gc, &kwg, &arm_tables).unwrap();
                egs.init(&pos.board, [&pos.racks[0][..], &pos.racks[1][..]]);
                egs.reset_position();
                egs.generate(0, mover, false).unwrap();
                let mut plays = egs.work_buffer.lists[0]
                    .iter()
                    .map(|candidate| candidate.play.clone())
                    .collect::<Vec<_>>();
                plays.push(movegen::Play::Exchange {
                    tiles: [][..].into(),
                });
                for play in &plays {
                    let value = move_value(&mut egs, mover, play);
                    for (below, wins) in [(point, true), (equity::Equity::ZERO, false)] {
                        assert_eq!(
                            egs.move_wins(mover, play, value - below, &super::Limits::NONE)
                                .unwrap(),
                            wins,
                            "[{name}] a move of value {value} against {}",
                            value - below
                        );
                        asked += 1;
                    }
                }
            }
        }
        println!("moves asked: {asked}");
        assert!(asked > 0);
    }

    #[test]
    #[inline]
    fn playing_to_win_keeps_a_win_it_found() {
        // at margins about the exact value, under state limits that stop the
        // solve: wherever the margin's question, within the quarter of the
        // limit best_move_to_win gives it, finds a win, the move played wins by
        // exact value; no win is found where none exists. With no limit, a win
        // is found exactly where one exists, its move wins, and the move played
        // is a best one
        let gc = game_config::make_english_game_config();
        let kwg_bytes = tiny_kwg_bytes();
        let kwg = kwg::kbwg_from_bytes_alloc(kwg_bytes.to_vec(), false).unwrap();
        let arm_tables = anagrams::ArmTables::build(&kwg, &gc, false).unwrap();
        let point = equity::Equity::new(equity::SCALE);
        let fresh = |pos: &Position| {
            let mut egs =
                EndgameSolver::<kwg::Node24, kwg::Node24>::new(&gc, &kwg, &arm_tables).unwrap();
            egs.init(&pos.board, [&pos.racks[0][..], &pos.racks[1][..]]);
            egs
        };
        let states = |cap: usize| super::Limits {
            time: None,
            states: Some(cap),
            memory: None,
            clock: None,
        };
        let mut found = 0usize;
        let mut kept = 0usize;
        for (name, pos) in all_positions()
            .into_iter()
            .enumerate()
            .filter(|(i, _)| [95, 135, 197, 205].contains(i))
            .map(|(_, named)| named)
        {
            for mover in 0..2u8 {
                let mut egs = fresh(&pos);
                let exact = egs.solve(mover).unwrap();
                let held = egs.work_buffer.table.held;
                let mut egs = fresh(&pos);
                let play = egs
                    .best_move_to_win(mover, equity::Equity::new(0), &super::Limits::NONE)
                    .unwrap()
                    .unwrap();
                assert_eq!(
                    move_value(&mut fresh(&pos), mover, &play),
                    exact,
                    "[{name}] with no limit, a best move"
                );
                for k in [-3, -1, 0] {
                    let margin = exact + equity::Equity::new(k * point.raw());
                    let lead = -margin;
                    let mut asking = fresh(&pos);
                    asking.start_clock(&super::Limits::NONE);
                    let won = asking
                        .ask_margin(mover, margin, &super::Limits::NONE)
                        .unwrap();
                    assert_eq!(
                        won.is_some(),
                        k < 0,
                        "[{name}] with no limit, a win found exactly where one exists"
                    );
                    if let Some(index) = won {
                        let play = asking.root_play(mover, index).unwrap().unwrap();
                        assert!(
                            move_value(&mut fresh(&pos), mover, &play) > margin,
                            "[{name}] the question's move wins"
                        );
                    }
                    for cap in (1..=6).map(|j| (held >> j).max(8)) {
                        let mut asking = fresh(&pos);
                        asking.start_clock(&super::Limits::NONE);
                        let won = asking.ask_margin(mover, margin, &states(cap / 4)).unwrap();
                        assert!(
                            won.is_none() || k < 0,
                            "[{name}] a win found where none exists"
                        );
                        let mut egs = fresh(&pos);
                        let play = egs
                            .best_move_to_win(mover, lead, &states(cap))
                            .unwrap()
                            .unwrap();
                        if won.is_none() {
                            continue;
                        }
                        assert!(
                            move_value(&mut fresh(&pos), mover, &play) > margin,
                            "[{name}] mover {mover}, margin {k} points, cap {cap}: a found win given up"
                        );
                        found += 1;
                        let mut alone = fresh(&pos);
                        alone.solve_within(mover, &states(cap)).unwrap();
                        let mut line = Vec::new();
                        alone.collect_pv(mover, &mut line).unwrap();
                        kept +=
                            usize::from(move_value(&mut fresh(&pos), mover, &line[0].1) <= margin);
                    }
                }
            }
        }
        println!("played to win: {found} found wins kept, {kept} the solve alone gives up");
        assert!(found > 0);
    }

    #[test]
    #[inline]
    fn a_one_in_bag_solve_agrees_with_the_reference_and_beats_passing() {
        let gc = game_config::make_english_game_config();
        let kwg_bytes = tiny_kwg_bytes();
        let kwg = kwg::kbwg_from_bytes_alloc(kwg_bytes.to_vec(), false).unwrap();
        let klv = klv::kbwg_from_bytes_alloc(klv::EMPTY_KLV_BYTES.to_vec()).unwrap();
        let mut mg = movegen::KurniaMoveGenerator::new(&gc);

        let board = empty_board();
        let racks = [vec![1u8, 2], vec![8u8, 20]]; // p0=[A,B] p1=[H,T]
        let bag = 1u8; // one A in the bag
        let arm_tables = anagrams::ArmTables::build(&kwg, &gc, false).unwrap();

        // from no scoreless turns, and from some, up to one short of the end
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
                super::OneInBagTurn {
                    mover: 0,
                    bag_tile: bag,
                    passes,
                    zeros,
                    answering: passes > 0, // the game's last turn a pass
                },
            );
            let mut egs =
                EndgameSolver::<kwg::Node24, kwg::Node24>::new(&gc, &kwg, &arm_tables).unwrap();
            egs.init_after_scoreless_turns(
                &board,
                [&racks[0][..], &racks[1][..]],
                u16::from(passes),
                u16::from(zeros),
            );
            let solvev = egs.solve_one_in_bag(0, bag).unwrap();
            assert_eq!(
                solvev,
                super::to_equity(refv),
                "bag layer vs reference from {passes},{zeros}"
            );

            let a = gc.alphabet();
            let pass_only = equity::Equity::new(
                a.scaled_rack_score(&racks[1]).raw() - a.scaled_rack_score(&racks[0]).raw(),
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
        let kwg = kwg::kbwg_from_bytes_alloc(kwg_bytes.to_vec(), false).unwrap();
        let board = empty_board();
        let mover_rack = [20u8]; // T
        let mut unseen_tally = vec![0u8; gc.alphabet().len() as usize];
        unseen_tally[1] = 2; // A, A
        unseen_tally[8] = 1; // H
        let arm_tables = anagrams::ArmTables::build(&kwg, &gc, false).unwrap();
        let mut egs =
            EndgameSolver::<kwg::Node24, kwg::Node24>::new(&gc, &kwg, &arm_tables).unwrap();
        let e = equity::Equity::new;
        let mut solve = |passes, zeros| {
            let mut progress = super::PegProgress::new(
                0,
                &board,
                &mover_rack,
                &unseen_tally,
                equity::Equity::ZERO,
                [passes, zeros],
            );
            let result = egs
                .resume_peg_one_in_bag(&mut progress, &super::Limits::NONE)
                .unwrap()
                .expect("english one-in-bag is always solvable");
            solved(&progress, result)
        };
        let result = solve(5, 5);
        assert_eq!(result.hypotheses, vec![(1, 2, e(4000)), (8, 1, e(1000))]);
        assert_eq!(result.win_pct, 1.0);
        assert_eq!(result.expected_margin, e(3000));
        // with four, the last of them no pass, the other side answers the pass
        // with AH when it holds it, and ends the game by passing when it holds
        // the two A
        let result = solve(0, 4);
        assert!(result.hypotheses[0].2 < e(-1000), "{:?}", result.hypotheses);
        assert_eq!(result.hypotheses[1], (8, 1, e(1000)));
        // with four, the last of them a pass, the mover's pass answers it and
        // ends the game at once, as with five
        let result = solve(1, 4);
        assert_eq!(result.hypotheses, vec![(1, 2, e(4000)), (8, 1, e(1000))]);
        assert_eq!(result.expected_margin, e(3000));
    }

    #[test]
    #[inline]
    fn differential_one_in_bag_exchange() {
        // under the exchange test config, whose second pass ends the game,
        // from no scoreless turns; and under it with passes ending the game
        // only as scoreless turns, where a pass answering a pass ends it
        // before the config's counts do, from a pass as the game's last turn
        let exchange_test = game_config::make_exchange_test_game_config();
        let mut zeros_only = exchange_test.clone();
        let game_config::GameConfig::Static(rules) = &mut zeros_only;
        rules.num_passes_to_end = 0;
        let kwg_bytes = tiny_kwg_bytes();
        let kwg = kwg::kbwg_from_bytes_alloc(kwg_bytes.to_vec(), false).unwrap();
        let klv = klv::kbwg_from_bytes_alloc(klv::EMPTY_KLV_BYTES.to_vec()).unwrap();
        let positions = all_positions();
        for (gc, name, mover, scoreless) in [
            (&exchange_test, "both-pass-only (BB vs HT)", 0u8, 0u8),
            (&exchange_test, "rand#9", 0, 0), // an A held, as the one in the bag
            (&zeros_only, "playout race (AB vs AT)", 1, 1),
        ] {
            let mut mg = movegen::KurniaMoveGenerator::new(gc);
            let arm_tables = anagrams::ArmTables::build(&kwg, gc, false).unwrap();
            let (_, pos) = positions.iter().find(|(n, _)| n == name).unwrap();
            let bag_tile = 1; // A
            let refv = reference_one_in_bag_ex(
                &mut Reference {
                    gc,
                    kwg: &kwg,
                    klv: &klv,
                    mg: &mut mg,
                    memo: Default::default(),
                },
                &pos.board,
                &pos.racks,
                super::OneInBagTurn {
                    mover,
                    bag_tile,
                    passes: scoreless,
                    zeros: scoreless,
                    answering: scoreless > 0, // the game's last turn a pass
                },
            );
            let mut egs =
                EndgameSolver::<kwg::Node24, kwg::Node24>::new(gc, &kwg, &arm_tables).unwrap();
            egs.init_after_scoreless_turns(
                &pos.board,
                [&pos.racks[0][..], &pos.racks[1][..]],
                u16::from(scoreless),
                u16::from(scoreless),
            );
            assert_eq!(
                egs.solve_one_in_bag(mover, bag_tile).unwrap(),
                super::to_equity(refv),
                "[{name}] passes_to_end={} mover={mover} scoreless={scoreless}",
                gc.num_passes_to_end()
            );
        }
    }

    #[test]
    #[inline]
    fn one_in_bag_exchange_beats_passing() {
        let gc = game_config::make_exchange_test_game_config();
        let kwg_bytes = tiny_kwg_bytes();
        let kwg = kwg::kbwg_from_bytes_alloc(kwg_bytes.to_vec(), false).unwrap();

        let board = empty_board();
        let racks = [vec![17u8], vec![1u8]]; // p0 = [Q], p1 = [A]
        let bag = 1u8; // one A in the bag

        let arm_tables = anagrams::ArmTables::build(&kwg, &gc, false).unwrap();
        let mut egs =
            EndgameSolver::<kwg::Node24, kwg::Node24>::new(&gc, &kwg, &arm_tables).unwrap();
        egs.init(&board, [&racks[0][..], &racks[1][..]]);
        let solvev = egs.solve_one_in_bag(0, bag).unwrap();

        let a = gc.alphabet();
        let pass_only = equity::Equity::new(
            a.scaled_rack_score(&racks[1]).raw() - a.scaled_rack_score(&racks[0]).raw(),
        ); // -9000
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
    fn a_clairvoyant_peg_keeps_the_positions_scoreless_turns() {
        let mut gc = game_config::make_exchange_test_game_config();
        let game_config::GameConfig::Static(rules) = &mut gc;
        rules.zeros_can_end_empty_board = true;
        let kwg = kwg::kbwg_from_bytes_alloc(tiny_kwg_bytes().to_vec(), false).unwrap();
        let arm_tables = anagrams::ArmTables::build(&kwg, &gc, false).unwrap();
        let mut solver =
            EndgameSolver::<kwg::Node24, kwg::Node24>::new(&gc, &kwg, &arm_tables).unwrap();
        let mut unseen = vec![0; gc.alphabet().len() as usize];
        for tile in [0, 8, 20] {
            // blank, H, and T
            unseen[tile] = 1;
        }
        // the next pass or exchange ends the game. With a blank in the bag we
        // exchange B for it and win by two against HT.
        for zeros in [2, u16::MAX] {
            let mut progress = super::PegProgress::new(
                0,
                &empty_board(),
                &[2, 2], // BB
                &unseen,
                equity::Equity::ZERO,
                [0, zeros],
            );
            let result = solver
                .resume_peg_one_in_bag(&mut progress, &super::Limits::NONE)
                .unwrap()
                .unwrap();
            let result = solved(&progress, result);
            let e = equity::Equity::new;
            assert_eq!(
                result.hypotheses,
                [(0, 1, e(2000)), (8, 1, e(-5000)), (20, 1, e(0))]
            );
            assert_eq!(result.expected_margin, e(-1000));
            assert_eq!(result.win_pct, 0.5);
        }
    }

    #[test]
    #[inline]
    fn peg_one_in_bag_exchange_solved_or_declined() {
        let kwg_bytes = tiny_kwg_bytes();
        let kwg = kwg::kbwg_from_bytes_alloc(kwg_bytes.to_vec(), false).unwrap();

        let gc = game_config::make_exchange_test_game_config();
        let board = empty_board();
        let mover_rack = [2u8]; // B, unplayable alone on an empty board
        let mut unseen_tally = vec![0u8; gc.alphabet().len() as usize];
        unseen_tally[8] = 1; // H
        unseen_tally[20] = 1; // T
        let arm_tables = anagrams::ArmTables::build(&kwg, &gc, false).unwrap();
        let mut egs =
            EndgameSolver::<kwg::Node24, kwg::Node24>::new(&gc, &kwg, &arm_tables).unwrap();
        let result = egs
            .solve_peg_one_in_bag_full(
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
            EndgameSolver::<kwg::Node24, kwg::Node24>::new(&gc_en, &kwg, &arm_tables_en).unwrap();
        assert!(
            egs_en
                .solve_peg_one_in_bag_full(
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
            EndgameSolver::<kwg::Node24, kwg::Node24>::new(&gc_bad, &kwg, &arm_tables_bad).unwrap();
        assert!(matches!(
            egs_bad
                .solve_peg_one_in_bag_full(
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
        let kwg = kwg::kbwg_from_bytes_alloc(kwg_bytes.to_vec(), false).unwrap();
        let klv = klv::kbwg_from_bytes_alloc(klv::EMPTY_KLV_BYTES.to_vec()).unwrap();
        let mut mg = movegen::KurniaMoveGenerator::new(&gc);
        let a = gc.alphabet();
        let alen = a.len() as usize;
        let arms = both_arms(&kwg, &gc);

        let committed = |board: &[u8],
                         mover_rack: &[u8],
                         unseen: &[(u8, u32)],
                         fixed: &mut Reference<'_, kwg::Node24, kwg::Node24>|
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
            (fixed.mg)
                .gen_moves_raw_all_unsorted(&snap, mover_rack, 0, movegen::PassPolicy::AsACandidate)
                .unwrap();
            let mut cands: Vec<Option<movegen::Play>> = vec![None]; // None = pass
            for vm in &fixed.mg.plays {
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
                            score.raw() as f32 - reference(fixed, &nb, &nr, opp, 0, 0, false)
                        }
                        _ => {
                            let nr = [mover_rack.to_vec(), opp_rack];
                            -reference_one_in_bag(
                                fixed,
                                board,
                                &nr,
                                super::OneInBagTurn {
                                    mover: opp as u8,
                                    bag_tile: t,
                                    passes: 1,
                                    zeros: 1,
                                    answering: true,
                                },
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

        // a B, and an A with a T, on AT with A and B unseen
        let (bname, mut board) = ("AT", empty_board());
        put_word(&mut board, 7, 6, &[1, 20]);
        let board = &board;
        let unseen = [(1u8, 1u32), (2, 1)];
        let mut unseen_tally = vec![0u8; alen];
        for &(tile, n) in &unseen {
            unseen_tally[tile as usize] = n as u8;
        }

        let mut fixed = Reference {
            gc: &gc,
            kwg: &kwg,
            klv: &klv,
            mg: &mut mg,
            memo: Default::default(),
        };
        let mut worst_gap = 0.0f64;
        let mut n_gap = 0;
        let mut n_gap_place = 0;
        let mut report = String::new();
        for mrack in [&[2u8][..], &[1, 20]] {
            let (ref_win, ref_marg, _ref_place, _ref_label) =
                committed(board, mrack, &unseen, &mut fixed);

            for (lean, arm_tables) in &arms {
                let mut egs =
                    EndgameSolver::<kwg::Node24, kwg::Node24>::new(&gc, &kwg, arm_tables).unwrap();
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

                let mut egs2 =
                    EndgameSolver::<kwg::Node24, kwg::Node24>::new(&gc, &kwg, arm_tables).unwrap();
                let clair = egs2
                    .peg_clairvoyant_aggregate(
                        &mut super::PegProgress::new(
                            0,
                            board,
                            mrack,
                            &unseen_tally,
                            equity::Equity::ZERO,
                            [0, 0],
                        ),
                        &super::BagLimits::new(&super::Limits::NONE, None),
                    )
                    .unwrap();

                assert!(got.committed, "english path must be committed");
                assert!(got.best_move.is_some(), "committed result must name a move");
                assert!(got.exact && clair.exact, "an unlimited solve is exact");
                assert!(
                    (got.win_pct - f64::from(ref_win)).abs() < 1e-6,
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
                    n_gap_place += usize::from(matches!(got.best_move, Some(PegMove::Place(_))));
                    if report.lines().count() < 25 {
                        report.push_str(&format!(
                            "gap win%={gap:.3} [{bname}] lean={lean} mover=[{}] unseen={unseen:?}: clairvoyant {:.3} vs committed {:.3}\n",
                            a.fmt_rack(mrack), clair.win_pct, got.win_pct,
                        ));
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

    // a position with tiles in the bag for solve_peg, the mover first to move,
    // under RandomDraws
    fn bag_position(
        board: &[u8],
        mover_rack: &[u8],
        unseen_tally: &[u8],
        bag_size: usize,
        score_diff: equity::Equity,
    ) -> super::PegBagPosition {
        super::PegBagPosition {
            mover: 0,
            board: board.to_vec(),
            mover_rack: mover_rack.to_vec(),
            unseen_tally: unseen_tally.to_vec(),
            bag_size,
            score_diff,
            scoreless: [0, 0],
            exchanges: [0, 0],
            model: super::PegModel::RandomDraws,
        }
    }

    fn moves_rows(
        egs: &mut EndgameSolver<'_, kwg::Node24, kwg::Node24>,
        progress: &mut super::PegBagProgress,
        result: &super::PegBagResult,
        limits: &super::Limits,
    ) -> Vec<super::PegBagRows> {
        let (splits, _) = progress.splits();
        (result.moves.iter())
            .map(|valued| {
                (egs.peg_bag_rows(progress, &valued.play, 0..splits, limits))
                    .unwrap()
                    .unwrap()
            })
            .collect()
    }

    fn uncut_peg(
        egs: &mut EndgameSolver<'_, kwg::Node24, kwg::Node24>,
        position: super::PegBagPosition,
    ) -> crate::error::Returns<Result<super::PegBagResult, super::PegUnsupported>> {
        let progress = super::PegBagProgress::new(position, super::PegBagChoices::WHOLE)?;
        egs.resume_peg(&mut progress.without_cuts()?, &super::Limits::NONE, 1)
    }

    fn solve_peg_rows(
        egs: &mut EndgameSolver<'_, kwg::Node24, kwg::Node24>,
        position: super::PegBagPosition,
    ) -> (super::PegBagResult, Vec<super::PegBagRows>) {
        let mut progress = super::PegBagProgress::new(position, super::PegBagChoices::WHOLE)
            .and_then(super::PegBagProgress::without_cuts)
            .unwrap();
        let result = (egs.resume_peg(&mut progress, &super::Limits::NONE, 1))
            .unwrap()
            .unwrap();
        let rows = moves_rows(egs, &mut progress, &result, &super::Limits::NONE);
        (result, rows)
    }

    #[test]
    fn a_bag_search_with_cuts_answers_the_same() {
        let gc = game_config::make_english_game_config();
        let kwg = kwg::kbwg_from_bytes_alloc(tiny_kwg_bytes().to_vec(), false).unwrap();
        let arm_tables = anagrams::ArmTables::build(&kwg, &gc, false).unwrap();
        let mut egs =
            EndgameSolver::<kwg::Node24, kwg::Node24>::new(&gc, &kwg, &arm_tables).unwrap();
        let mut board = empty_board();
        put_word(&mut board, 7, 6, &[1, 20]); // AT
        let mut unseen_tally = vec![0; gc.alphabet().len() as usize];
        for tile in [5, 8, 19, 20, 1] {
            unseen_tally[tile] += 1;
        }
        for (rack, bag) in [(&[1, 20][..], 2), (&[5, 1, 20][..], 3)] {
            for estimates in [false, true] {
                let position = bag_position(&board, rack, &unseen_tally, bag, equity::Equity::ZERO);
                let choices = super::PegBagChoices {
                    estimates,
                    ..super::PegBagChoices::WHOLE
                };
                let mut answers = Vec::new();
                for (cuts, threads) in [(false, 1), (true, 1), (true, 3)] {
                    let mut progress = super::PegBagProgress::new(position.clone(), choices);
                    if cuts {
                        progress = progress.unwrap().with_cuts();
                    }
                    let mut progress = progress.unwrap();
                    let result = (egs.resume_peg(&mut progress, &super::Limits::NONE, threads))
                        .unwrap()
                        .unwrap();
                    answers.push((result.best_move, result.win_pct, result.expected_margin));
                }
                assert!(
                    answers[0] == answers[1] && answers[0] == answers[2],
                    "rack {rack:?} estimates {estimates}"
                );
            }
        }
    }

    #[test]
    fn a_bag_search_marks_its_cut_rows_and_cuts_unless_told_not_to() {
        let gc = game_config::make_english_game_config();
        let kwg = kwg::kbwg_from_bytes_alloc(tiny_kwg_bytes().to_vec(), false).unwrap();
        let arm_tables = anagrams::ArmTables::build(&kwg, &gc, false).unwrap();
        let mut egs =
            EndgameSolver::<kwg::Node24, kwg::Node24>::new(&gc, &kwg, &arm_tables).unwrap();
        let mut board = empty_board();
        put_word(&mut board, 7, 6, &[1, 20]); // AT
        let mut unseen_tally = vec![0; gc.alphabet().len() as usize];
        for tile in [5, 8, 19, 20, 1] {
            unseen_tally[tile] += 1;
        }
        let position = bag_position(&board, &[5, 1, 20], &unseen_tally, 3, equity::Equity::ZERO);
        let mut rows = |estimates: bool, cuts: Option<bool>, limits: &super::Limits| {
            let choices = super::PegBagChoices {
                estimates,
                ..super::PegBagChoices::WHOLE
            };
            let mut progress = super::PegBagProgress::new(position.clone(), choices).unwrap();
            progress = match cuts {
                Some(true) => progress.with_cuts().unwrap(),
                Some(false) => progress.without_cuts().unwrap(),
                None => progress,
            };
            (egs.resume_peg(&mut progress, limits, 1)).unwrap().unwrap();
            (progress.answer_moves(0..usize::MAX).into_iter())
                .map(|row| {
                    (
                        row.play,
                        row.win_pct,
                        row.expected_margin,
                        row.exact,
                        row.cut,
                    )
                })
                .collect::<Vec<_>>()
        };
        let timed = super::Limits {
            time: Some(std::time::Duration::MAX),
            ..super::Limits::NONE
        };
        for estimates in [false, true] {
            for limits in [&super::Limits::NONE, &timed] {
                let cut = rows(estimates, Some(true), limits);
                for (_, _, margin, exact, is_cut) in &cut {
                    assert_eq!(margin.is_none(), *is_cut);
                    assert!(!(*is_cut && *exact));
                }
                let whole = rows(estimates, Some(false), limits);
                assert!(whole.iter().all(|row| !row.4 && row.2.is_some()));
                assert!(rows(estimates, None, limits) == cut);
            }
        }
    }

    #[test]
    fn a_bag_search_can_restrict_only_its_first_play() {
        let gc = game_config::make_english_game_config();
        let kwg = kwg::kbwg_from_bytes_alloc(tiny_kwg_bytes().to_vec(), false).unwrap();
        let arm_tables = anagrams::ArmTables::build(&kwg, &gc, false).unwrap();
        let mut egs =
            EndgameSolver::<kwg::Node24, kwg::Node24>::new(&gc, &kwg, &arm_tables).unwrap();
        let mut board = empty_board();
        put_word(&mut board, 7, 6, &[1, 20]); // AT
        let mover_rack = [1, 20]; // AT
        let mut unseen_tally = vec![0; gc.alphabet().len() as usize];
        for tile in [5, 8, 19, 20] {
            unseen_tally[tile] += 1;
        }
        let position = bag_position(&board, &mover_rack, &unseen_tally, 2, equity::Equity::ZERO);
        let (whole, _) = solve_peg_rows(&mut egs, position.clone());
        let pass = movegen::Play::Exchange {
            tiles: [][..].into(),
        };
        assert!(whole.exact);
        assert!(whole.best_move.as_ref().is_some_and(|play| play != &pass));
        let expected = whole.moves.iter().find(|move_| move_.play == pass).unwrap();

        let mut state = crate::game_state::GameState::new(&gc);
        state.board_tiles.copy_from_slice(&board);
        state.players[0].rack.extend_from_slice(&mover_rack);
        let klv = crate::klv::Klv::<kwg::Node24>::empty().unwrap();
        let roots = super::RootMoves::checked(
            &movegen::BoardSnapshot {
                board_tiles: &board,
                game_config: &gc,
                kwg: &kwg,
                klv: &klv,
                anagrams: None,
                rack_lengths: None,
            },
            &state,
            vec![pass.clone()],
        )
        .unwrap();
        let mut progress = super::PegBagProgress::new(position, super::PegBagChoices::WHOLE)
            .unwrap()
            .with_root_moves(Some(roots));
        let selected = egs
            .resume_peg(&mut progress, &super::Limits::NONE, 1)
            .unwrap()
            .unwrap();
        assert!(selected.exact);
        assert!(selected.best_move == Some(pass.clone()));
        assert_eq!(selected.moves.len(), 1);
        assert!(selected.moves[0].play == pass);
        assert_eq!(Some(selected.expected_margin), expected.expected_margin);
        assert_eq!(selected.win_pct, expected.win_pct);
    }

    // cells for a bag search's moves and splits that cannot be held (a count
    // past counting, or past what can be allocated) are an error, not a failed
    // allocation; those that can, made
    #[test]
    #[inline]
    fn cells_a_bag_search_cannot_hold_are_refused() {
        for n in [None, Some(usize::MAX), Some(isize::MAX as usize / 2)] {
            let message = super::cells_of(n, 0u64).unwrap_err().to_string();
            assert!(message.contains("more than it can hold"), "{message}");
        }
        assert_eq!(super::cells_of(Some(3), 7u8).unwrap(), [7, 7, 7]);
    }

    // a bag search split into n parts, each solved on its own: each move's rows
    // of all the parts together are the whole search's rows, and weighed by
    // their deals give its chance to win and mean margin; so for a part count
    // past the splits too, some parts then given none
    #[test]
    #[inline]
    fn a_bag_search_in_parts_merges_into_the_whole() {
        let gc = game_config::make_english_game_config();
        let kwg = kwg::kbwg_from_bytes_alloc(tiny_kwg_bytes().to_vec(), false).unwrap();
        let arm_tables = anagrams::ArmTables::build(&kwg, &gc, false).unwrap();
        let mut board = empty_board();
        put_word(&mut board, 7, 6, &[1, 20]); // AT
        let mut unseen_tally = vec![0u8; gc.alphabet().len() as usize];
        unseen_tally[1] = 2; // A
        unseen_tally[8] = 1; // H
        unseen_tally[19] = 1; // S
        unseen_tally[20] = 2; // T
        let solve = |part: Option<(usize, usize)>| {
            let mut egs =
                EndgameSolver::<kwg::Node24, kwg::Node24>::new(&gc, &kwg, &arm_tables).unwrap();
            let position = bag_position(&board, &[5, 20], &unseen_tally, 2, equity::Equity::ZERO);
            let choices = super::PegBagChoices {
                part: part.unwrap_or((0, 1)),
                estimates: false,
            };
            let mut progress = super::PegBagProgress::new(position, choices).unwrap();
            if let Some((k, n)) = part {
                assert_eq!(progress.part_of(), (k, n));
            }
            let result = (egs.resume_peg(&mut progress, &super::Limits::NONE, 1))
                .unwrap()
                .unwrap();
            let rows = moves_rows(&mut egs, &mut progress, &result, &super::Limits::NONE);
            (result, rows)
        };
        let (whole, whole_rows) = solve(None);
        assert!(whole.exact);
        let key = |play: &movegen::Play| match play {
            movegen::Play::Place {
                down,
                lane,
                idx,
                word,
                score,
            } => Some((*down, *lane, *idx, word[..].to_vec(), *score)),
            movegen::Play::Exchange { tiles } => (!tiles.is_empty())
                .then(|| (false, -1, -1, tiles[..].to_vec(), equity::Equity::ZERO)),
        };
        let splits = whole_rows[0].hypotheses.len();
        assert!(splits > 1, "{splits} splits");
        for n in [2, splits + 1] {
            let mut rows = fash::MyHashMap::default();
            for k in 0..n {
                let (part, part_rows) = solve(Some((k, n)));
                assert!(part.exact, "part {k} of {n}");
                for (valued, own) in part.moves.iter().zip(&part_rows) {
                    let at = rows.entry(key(&valued.play)).or_insert_with(Vec::new);
                    for (row, &win) in own.hypotheses.iter().zip(own.wins.iter()) {
                        at.push((row.clone(), win));
                    }
                }
            }
            assert_eq!(rows.len(), whole.moves.len(), "{n} parts");
            for (valued, own) in whole.moves.iter().zip(&whole_rows) {
                let mut merged = rows.remove(&key(&valued.play)).unwrap();
                let mut want = (own.hypotheses.iter().cloned())
                    .zip(own.wins.iter().copied())
                    .collect::<Vec<_>>();
                merged.sort_by(|a, b| a.0.cmp(&b.0));
                want.sort_by(|a, b| a.0.cmp(&b.0));
                assert_eq!(merged.len(), want.len(), "{n} parts");
                let (mut deals, mut won, mut margin) = (0u64, 0.0f64, 0.0f64);
                for ((row, win), (want_row, want_win)) in merged.iter().zip(&want) {
                    assert_eq!(row, want_row, "{n} parts");
                    assert_eq!(win, want_win, "{n} parts");
                    deals += row.1;
                    won += row.1 as f64 * win;
                    margin += row.1 as f64 * f64::from(row.2.raw());
                }
                assert!(
                    (won / deals as f64 - valued.win_pct).abs() < 1e-9,
                    "{n} parts"
                );
                assert!(
                    (margin / deals as f64 - f64::from(valued.expected_margin.unwrap().raw()))
                        .abs()
                        <= 1.0,
                    "{n} parts"
                );
            }
        }
    }

    #[test]
    #[inline]
    fn a_pool_gives_a_rack_no_deal_holds_every_move_the_walk_does() {
        // both B are ours, but the opponent, blind to our tiles, may picture
        // one in the bag and draw it: a rack of B with three unseen tiles,
        // which can play BATH below AT making AH; the pool must give such a
        // rack every move the walk does, which a debug build checks lane by
        // lane, and a few thousand positions reach such racks
        let gc = game_config::make_english_game_config();
        let kwg = kwg::kbwg_from_bytes_alloc(tiny_kwg_bytes().to_vec(), false).unwrap();
        let arm_tables = anagrams::ArmTables::build(&kwg, &gc, false).unwrap();
        let mut board = empty_board();
        put_word(&mut board, 7, 6, &[1, 20]); // AT
        let mut unseen_tally = vec![0u8; gc.alphabet().len() as usize];
        unseen_tally[1] = 2; // A
        unseen_tally[8] = 1; // H
        unseen_tally[19] = 1; // S
        unseen_tally[20] = 2; // T
        let mut egs =
            EndgameSolver::<kwg::Node24, kwg::Node24>::new(&gc, &kwg, &arm_tables).unwrap();
        let mut position = bag_position(&board, &[2, 2], &unseen_tally, 2, equity::Equity::ZERO);
        position.model = super::PegModel::InfoSets;
        let few = super::Limits {
            time: None,
            states: Some(20_000),
            memory: None,
            clock: None,
        };
        let result = egs.solve_peg(position, &few, 1).unwrap().unwrap();
        assert!(!result.moves.is_empty());
    }

    #[test]
    #[inline]
    fn a_search_with_seven_in_the_bag_keeps_its_time_through_exchanges() {
        // seven in the bag makes exchanges legal, and a line of exchanges ends
        // on scoreless turns without an endgame, where the search read its
        // clock: once the first move had a value, such a search ran on past
        // its time. Our B has three placements and every other tile, ours and
        // unseen, none, so the first move is valued at once (the opponent
        // holds one tile, the bag the other seven) and the exchanges after it
        // must stop at the time
        let gc = game_config::make_english_game_config();
        let kwg = kwg::kbwg_from_bytes_alloc(tiny_kwg_bytes().to_vec(), false).unwrap();
        let arm_tables = anagrams::ArmTables::build(&kwg, &gc, false).unwrap();
        let mut board = empty_board();
        put_word(&mut board, 7, 6, &[1, 20]); // AT
        let mut unseen_tally = vec![0u8; gc.alphabet().len() as usize];
        for tile in [3, 4, 5, 6, 7, 9, 11, 13] {
            unseen_tally[tile] = 1; // C D E F G I K M
        }
        let rack = [2, 10, 17, 22, 23, 24, 26]; // B J Q V W X Z
        let limits = super::Limits {
            time: Some(std::time::Duration::from_millis(16)),
            states: None,
            memory: None,
            clock: None,
        };
        for model in [super::PegModel::RandomDraws, super::PegModel::InfoSets] {
            let mut egs =
                EndgameSolver::<kwg::Node24, kwg::Node24>::new(&gc, &kwg, &arm_tables).unwrap();
            let mut position = bag_position(&board, &rack, &unseen_tally, 7, equity::Equity::ZERO);
            position.model = model;
            let t0 = std::time::Instant::now();
            let result = egs.solve_peg(position, &limits, 1).unwrap().unwrap();
            let took = t0.elapsed();
            assert!(
                took < std::time::Duration::from_secs(5),
                "a search of 16 ms took {took:?}"
            );
            assert!(!result.exact);
        }
    }

    #[test]
    #[inline]
    fn a_search_stops_at_its_time_before_its_first_move_has_a_value() {
        // nine in the bag: our B plays one tile and draws one, which leaves
        // eight, so exchanges lie below every move, the first included; a
        // search that waited for its first move's value before any limit could
        // stop it ran on past its time. It stops at the time, answering no move
        let gc = game_config::make_english_game_config();
        let kwg = kwg::kbwg_from_bytes_alloc(tiny_kwg_bytes().to_vec(), false).unwrap();
        let arm_tables = anagrams::ArmTables::build(&kwg, &gc, false).unwrap();
        let mut board = empty_board();
        put_word(&mut board, 7, 6, &[1, 20]); // AT
        let mut unseen_tally = vec![0u8; gc.alphabet().len() as usize];
        for tile in [3, 4, 5, 6, 7, 9, 11, 13, 14, 15] {
            unseen_tally[tile] = 1; // C D E F G I K M N O
        }
        let rack = [2, 10, 17, 22, 23, 24, 26]; // B J Q V W X Z
        let limits = super::Limits {
            time: Some(std::time::Duration::from_millis(16)),
            states: None,
            memory: None,
            clock: None,
        };
        for model in [super::PegModel::RandomDraws, super::PegModel::InfoSets] {
            let mut egs =
                EndgameSolver::<kwg::Node24, kwg::Node24>::new(&gc, &kwg, &arm_tables).unwrap();
            let mut position = bag_position(&board, &rack, &unseen_tally, 9, equity::Equity::ZERO);
            position.model = model;
            let t0 = std::time::Instant::now();
            let result = egs.solve_peg(position, &limits, 1).unwrap().unwrap();
            let took = t0.elapsed();
            assert!(
                took < std::time::Duration::from_secs(5),
                "a search of 16 ms took {took:?}"
            );
            assert!(!result.exact);
            assert!(result.best_move.is_none());
            assert!(result.moves.is_empty());
        }
    }

    #[test]
    #[inline]
    fn a_search_with_a_tile_in_the_bag_keeps_its_time_through_exchanges() {
        // Spanish's rules: an exchange is legal with one tile in the bag, and
        // twelve scoreless turns end the game, so lines of exchanges, which
        // start no endgame, run long. Our B has three placements and every
        // other tile, ours and unseen, none; the search must stop at its time
        let gc = game_config::Options {
            preset: "english",
            tiles: None,
            board: None,
            rack_size: None,
            jumbled: false,
            woadwage: false,
            players: None,
            bingo_bonus: None,
            zeros_to_end: Some(12),
            passes_to_end: None,
            exchange_limit: Some(1),
            exchanges: None,
        }
        .make_game_config(&|_: &str| Err("no files".into()))
        .unwrap();
        let kwg = kwg::kbwg_from_bytes_alloc(tiny_kwg_bytes().to_vec(), false).unwrap();
        let arm_tables = anagrams::ArmTables::build(&kwg, &gc, false).unwrap();
        let mut board = empty_board();
        put_word(&mut board, 7, 6, &[1, 20]); // AT
        let mut unseen_tally = vec![0u8; gc.alphabet().len() as usize];
        for tile in [3, 4, 5, 6, 7, 9, 11, 13] {
            unseen_tally[tile] = 1; // C D E F G I K M: the opponent's seven, the bag's one
        }
        let rack = [2, 10, 17, 22, 23, 24, 26]; // B J Q V W X Z
        let limits = super::Limits {
            time: Some(std::time::Duration::from_millis(16)),
            states: None,
            memory: None,
            clock: None,
        };
        let mut egs =
            EndgameSolver::<kwg::Node24, kwg::Node24>::new(&gc, &kwg, &arm_tables).unwrap();
        let t0 = std::time::Instant::now();
        let result = egs
            .solve_peg_one_in_bag_full(
                0,
                &board,
                &rack,
                &unseen_tally,
                equity::Equity::ZERO,
                &limits,
            )
            .unwrap()
            .unwrap();
        let took = t0.elapsed();
        assert!(
            took < std::time::Duration::from_secs(5),
            "a search of 16 ms took {took:?}"
        );
        assert!(!result.exact);
    }

    #[test]
    #[inline]
    fn a_bag_search_of_one_tile_answers_as_the_committed_model() {
        // with one tile in the bag a draw can only be that tile, so the search
        // with tiles in the bag answers as the committed model does: the same
        // margin in each split, share of wins, mean margin, and move
        let gc = game_config::make_english_game_config();
        let kwg = kwg::kbwg_from_bytes_alloc(tiny_kwg_bytes().to_vec(), false).unwrap();
        let arm_tables = anagrams::ArmTables::build(&kwg, &gc, false).unwrap();
        let mut egs =
            EndgameSolver::<kwg::Node24, kwg::Node24>::new(&gc, &kwg, &arm_tables).unwrap();
        let e = equity::Equity::new;
        let mut at = empty_board();
        put_word(&mut at, 7, 6, &[1, 20]); // AT
        let tiles = [1u8, 2, 8, 20]; // A B H T
        let mut checked = 0;
        for board in [empty_board(), at] {
            for unseen in [&[8u8, 20][..], &[1, 2], &[1, 8, 20], &[2, 2, 8]] {
                let mut unseen_tally = vec![0u8; gc.alphabet().len() as usize];
                for &tile in unseen {
                    unseen_tally[tile as usize] += 1;
                }
                for &x in &tiles {
                    for &y in &tiles {
                        let mover_rack = [x, y];
                        for score_diff in [e(0), e(3000), e(-1000)] {
                            let case = (&board[..], mover_rack, unseen, score_diff);
                            let one = egs
                                .solve_peg_one_in_bag_full(
                                    0,
                                    &board,
                                    &mover_rack,
                                    &unseen_tally,
                                    score_diff,
                                    &super::Limits::NONE,
                                )
                                .unwrap()
                                .unwrap();
                            let (any, any_rows) = solve_peg_rows(
                                &mut egs,
                                bag_position(&board, &mover_rack, &unseen_tally, 1, score_diff),
                            );
                            assert!(one.exact && any.exact, "{case:?}");
                            let mut hyps = (one.hypotheses.iter())
                                .map(|&(tile, ways, value)| (vec![tile], u64::from(ways), value))
                                .collect::<Vec<_>>();
                            hyps.sort_unstable();
                            let mut any_hyps = any_rows[0].hypotheses.clone();
                            any_hyps.sort_unstable();
                            // the margins in each split are those of the move each
                            // picked; the moves agree, so the splits do
                            assert!((any.win_pct - one.win_pct).abs() < 1e-6, "{case:?}");
                            assert_eq!(any.expected_margin, one.expected_margin, "{case:?}");
                            match one.best_move {
                                Some(PegMove::Pass) => assert!(
                                    matches!(&any.best_move,
                                        Some(movegen::Play::Exchange { tiles }) if tiles.is_empty()),
                                    "{case:?}"
                                ),
                                Some(PegMove::Place(play)) => {
                                    assert!(any.best_move == Some(play), "{case:?}")
                                }
                                None => unreachable!("the committed model answers a move"),
                            }
                            assert_eq!(any_hyps, hyps, "{case:?}");
                            checked += 1;
                        }
                    }
                }
            }
        }
        assert_eq!(checked, 2 * 4 * 16 * 3);
    }

    #[test]
    fn an_exact_bag_pass_can_stop_and_resume() {
        let gc = game_config::make_english_game_config();
        let kwg = kwg::kbwg_from_bytes_alloc(tiny_kwg_bytes().to_vec(), false).unwrap();
        let arm_tables = anagrams::ArmTables::build(&kwg, &gc, false).unwrap();
        let mut board = empty_board();
        put_word(&mut board, 7, 6, &[1, 20]); // AT
        let mut unseen = vec![0u8; gc.alphabet().len() as usize];
        for tile in [1, 2, 8] {
            unseen[tile] += 1;
        }
        let position = bag_position(&board, &[1, 2], &unseen, 2, equity::Equity::ZERO);
        let mut solver =
            EndgameSolver::<kwg::Node24, kwg::Node24>::new(&gc, &kwg, &arm_tables).unwrap();
        let whole = uncut_peg(&mut solver, position.clone()).unwrap().unwrap();
        assert!(whole.exact);
        for threads in [1, 2] {
            let mut progress =
                super::PegBagProgress::new(position.clone(), super::PegBagChoices::WHOLE)
                    .and_then(super::PegBagProgress::without_cuts)
                    .unwrap()
                    .with_exact_pass()
                    .unwrap();
            let stopped = solver
                .resume_peg(
                    &mut progress,
                    &super::Limits {
                        time: Some(std::time::Duration::ZERO),
                        ..super::Limits::NONE
                    },
                    threads,
                )
                .unwrap()
                .unwrap();
            assert!(!stopped.exact && stopped.moves.is_empty());
            let resumed = solver
                .resume_peg(&mut progress, &super::Limits::NONE, threads)
                .unwrap()
                .unwrap();
            assert!(resumed.exact);
            assert_eq!(resumed.moves.len(), whole.moves.len());
            for (a, b) in resumed.moves.iter().zip(&whole.moves) {
                assert!(a.play == b.play && a.exact && b.exact);
                assert_eq!(a.win_pct, b.win_pct);
                assert_eq!(a.expected_margin, b.expected_margin);
            }
        }
    }

    // a search with tiles in the bag tells its listener, at every node inside
    // it, how many of its root's moves the pass under way has valued, of all,
    // and its line says so: never fewer than it told before, under either
    // model, on one thread or on two; and with one tile in the bag, as the
    // committed model searches it
    #[test]
    #[inline]
    fn a_bag_search_tells_its_listener_how_many_moves_it_has_valued() {
        let gc = game_config::make_english_game_config();
        let kwg = kwg::kbwg_from_bytes_alloc(tiny_kwg_bytes().to_vec(), false).unwrap();
        let arm_tables = anagrams::ArmTables::build(&kwg, &gc, false).unwrap();
        let mut board = empty_board();
        put_word(&mut board, 7, 6, &[1, 20]); // AT
        let mut unseen_tally = vec![0u8; gc.alphabet().len() as usize];
        for &tile in &[1u8, 2, 8] {
            unseen_tally[tile as usize] += 1; // A B H
        }
        let listened = || {
            let egs =
                EndgameSolver::<kwg::Node24, kwg::Node24>::new(&gc, &kwg, &arm_tables).unwrap();
            let heard = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
            let seen = heard.clone();
            let listener = move |p: &super::Progress| {
                let line = p.after(std::time::Duration::ZERO).to_string();
                seen.borrow_mut().push((p.moves, line));
                Ok(())
            };
            (egs, heard, listener)
        };
        // every report's moves, each never fewer than the one before, and the
        // last report's count of moves valued
        let checked = |heard: &[(Option<(usize, usize)>, String)], all: usize| {
            let mut last = 0;
            for (moves, line) in heard {
                let (valued, of) = moves.expect("a report inside the search says its moves");
                assert_eq!(of, all);
                assert!(
                    valued >= last && valued < of,
                    "{valued} of {of} after {last}"
                );
                assert!(
                    line.contains(&format!(", {valued} of {of} moves valued")),
                    "{line}"
                );
                last = valued;
            }
            last
        };
        for (name, model, threads) in [
            ("random draws", super::PegModel::RandomDraws, 1),
            ("info sets", super::PegModel::InfoSets, 1),
            ("random draws", super::PegModel::RandomDraws, 2),
        ] {
            let (mut egs, heard, listener) = listened();
            let mut position =
                bag_position(&board, &[1, 2], &unseen_tally, 2, equity::Equity::ZERO);
            position.model = model;
            let mut progress =
                super::PegBagProgress::new(position, super::PegBagChoices::WHOLE).unwrap();
            let result = egs
                .listening(listener, |egs| {
                    egs.resume_peg(&mut progress, &super::Limits::NONE, threads)
                })
                .unwrap()
                .unwrap();
            let last = checked(&heard.borrow(), result.moves.len());
            // on two threads the other may value every move left before the
            // listening one reports again, so one thread's count alone must
            // have risen past none
            assert!(
                threads > 1 || last > 0,
                "{name}, {threads} threads: the last at {last}"
            );
            // on one thread its progress counts what it searched, and its pass
            // ends with every move valued
            if threads == 1 {
                let (endgames, nodes) = progress.searched();
                assert!(endgames > 0 && nodes > 0, "{name}: {endgames}, {nodes}");
                let all = result.moves.len();
                assert_eq!(progress.moves(), Some((all, all)), "{name}");
            }
        }
        let position = bag_position(&board, &[1, 2], &unseen_tally, 2, equity::Equity::ZERO);
        let (mut egs, _, listener) = listened();
        let mut whole =
            super::PegBagProgress::new(position.clone(), super::PegBagChoices::WHOLE).unwrap();
        let mut progress =
            super::PegBagProgress::new(position, super::PegBagChoices::WHOLE).unwrap();
        let one_state = super::Limits {
            time: None,
            states: Some(1),
            memory: None,
            clock: None,
        };
        let (whole, resumed) = egs.listening(listener, |egs| {
            let whole = egs
                .resume_peg(&mut whole, &super::Limits::NONE, 1)
                .unwrap()
                .unwrap();
            let stopped = egs
                .resume_peg(&mut progress, &one_state, 1)
                .unwrap()
                .unwrap();
            assert!(!stopped.exact);
            let resumed = egs
                .resume_peg(&mut progress, &super::Limits::NONE, 2)
                .unwrap()
                .unwrap();
            (whole, resumed)
        });
        assert!(resumed.exact);
        assert!(resumed.best_move == whole.best_move);
        assert_eq!(resumed.win_pct, whole.win_pct);
        assert_eq!(resumed.expected_margin, whole.expected_margin);
        // one tile in the bag: the committed model's moves are the pass and the
        // places
        let (mut egs, heard, listener) = listened();
        unseen_tally[2] = 0; // A H, one of them in the bag
        let result = egs
            .listening(listener, |egs| {
                egs.solve_peg_one_in_bag_full(
                    0,
                    &board,
                    &[1, 2],
                    &unseen_tally,
                    equity::Equity::ZERO,
                    &super::Limits::NONE,
                )
            })
            .unwrap()
            .unwrap();
        assert!(result.exact);
        let all = heard.borrow()[0].0.unwrap().1;
        assert!(checked(&heard.borrow(), all) > 0);
    }

    // inside the random-draws search a pass answering a pass the line made ends
    // the line, as an empty bag's endgame ends it, whatever the scoreless
    // turns' counts: with no words, a pass inside it values one position (the
    // other's, whose pass ends the game) where counting the six scoreless turns
    // would value five, and the value is the game's end either way
    #[test]
    #[inline]
    fn a_pass_answering_a_pass_inside_a_bag_search_ends_the_line() {
        let gc = game_config::make_english_game_config();
        let kwg = kwg::kbwg_from_bytes_alloc(b"\x00\x00\x40\x00".to_vec(), false).unwrap();
        let arm_tables = anagrams::ArmTables::build(&kwg, &gc, false).unwrap();
        let klv = klv::Klv::<kwg::Node24>::empty().unwrap();
        let mut sub =
            EndgameSolver::<kwg::Node24, kwg::Node24>::new(&gc, &kwg, &arm_tables).unwrap();
        let mut mg = movegen::KurniaMoveGenerator::new(&gc);
        let valued = super::BagValued::new(1);
        let limits = super::BagLimits::new(&super::Limits::NONE, None);
        let mut search = super::BagSearch {
            gc: &gc,
            kwg: &kwg,
            arm_tables: &arm_tables,
            klv: &klv,
            sub: &mut sub,
            mg: &mut mg,
            valued: &valued,
            limits: &limits,
            model: super::PegModel::RandomDraws,
            exact: true,
            hand_overs: &[],
            hand_over: None,
            ply: 0,
            arena: &mut super::BagArena::new(gc.alphabet().len() as usize, 2),
        };
        let mut board = empty_board();
        put_word(&mut board, 7, 6, &[1, 20]); // AT
        let mut bag = vec![0u8; gc.alphabet().len() as usize];
        bag[1] = 1; // A
        bag[8] = 1; // H
        let mut pos = super::InBag {
            board,
            racks: [vec![2], vec![20]], // B, T
            bag,
            mover: 0,
            passes: 0,
            zeros: 0,
            answering: false,
            exchanges: [0, 0],
        };
        let pass = movegen::Play::Exchange {
            tiles: [][..].into(),
        };
        // the game's end: the mover's B (3 points) against the other's T (1)
        assert_eq!(search.move_value(&mut pos, &pass).unwrap(), -2000.0);
        let positions = (valued.shards.iter())
            .map(|shard| shard.lock().unwrap().nodes.held)
            .sum::<usize>();
        assert_eq!(positions, 1);
    }

    // a bag table keeps all it is given while it can grow, growing before three
    // quarters of it would hold; refused room by the memory limit, a key not in
    // it takes the place of the first entry worth least on its way there, a
    // key's own entry takes its new value, and a key whose first place is empty
    // is not kept
    #[test]
    #[inline]
    fn a_bag_table_keeps_all_while_it_grows_then_gives_up_the_least_worth() {
        fn none_held() -> usize {
            0
        }
        let worth = |&(_, worth): &(u64, u8)| worth;
        let mut table = super::BagTable::default();
        for i in 0..1000 {
            table.keep(super::zobrist(i), (i, (i % 3) as u8), None, worth);
        }
        assert!(table.held == 1000 && 4 * table.held <= 3 * table.entries.len());
        for i in 0..1000 {
            assert_eq!(table.get(super::zobrist(i)), Some(&(i, (i % 3) as u8)));
        }
        assert_eq!(table.get(super::zobrist(1000)), None);
        // eight entries' bytes and one more: room to grow to eight
        let eight = 8 * std::mem::size_of::<Option<(u64, (u64, u8))>>();
        let memory = Some(super::MemoryLimit {
            bytes: eight + 1,
            held: none_held,
            reserved: None,
        });
        let mut table = super::BagTable::default();
        for key in 1..=6 {
            table.keep(key, (key, 0), memory, worth);
        }
        assert!(table.entries.len() == 8 && table.held == 6);
        // eight entries' bytes alone: the program would hold all of them, so no
        // room to grow, and three hold at most. A key's first place is its top
        // two bits
        let memory = Some(super::MemoryLimit {
            bytes: eight,
            held: none_held,
            reserved: None,
        });
        let mut table = super::BagTable::default();
        for (key, its_worth) in [(1, 2), (2, 0), (3, 1), (4, 2)] {
            table.keep(key, (key, its_worth), memory, worth);
        }
        assert!(table.entries.len() == 4 && table.held == 3);
        assert_eq!(table.get(2), None);
        for key in [1, 3, 4] {
            assert!(table.get(key).is_some_and(|&(kept, _)| kept == key));
        }
        table.keep(3, (5, 0), memory, worth);
        assert!(table.held == 3 && table.get(3) == Some(&(5, 0)));
        let last = 3 << 62 | 1;
        table.keep(last, (last, 2), memory, worth);
        assert!(table.held == 3 && table.get(last).is_none());
    }

    #[test]
    #[inline]
    fn a_bag_search_stopped_by_time_holding_its_memory_is_not_out_of_memory() {
        fn all_held() -> usize {
            usize::MAX
        }
        let memory = Some(super::MemoryLimit {
            bytes: 1,
            held: all_held,
            reserved: None,
        });
        let gc = game_config::make_english_game_config();
        let kwg = kwg::kbwg_from_bytes_alloc(tiny_kwg_bytes().to_vec(), false).unwrap();
        let arm_tables = anagrams::ArmTables::build(&kwg, &gc, false).unwrap();
        let mut egs =
            EndgameSolver::<kwg::Node24, kwg::Node24>::new(&gc, &kwg, &arm_tables).unwrap();
        let mut board = empty_board();
        put_word(&mut board, 7, 6, &[1, 20]); // AT
        let mut unseen_tally = vec![0u8; gc.alphabet().len() as usize];
        for tile in [1u8, 8, 20, 20] {
            unseen_tally[tile as usize] += 1;
        }
        let (mut by_time, mut by_memory) = (0, 0);
        for model in [super::PegModel::RandomDraws, super::PegModel::InfoSets] {
            let mut position =
                bag_position(&board, &[1, 2], &unseen_tally, 3, equity::Equity::ZERO);
            position.model = model;
            // on one thread: more take room for their tasks, which the memory
            // limit refuses before the time is looked at
            for time in [None, Some(std::time::Duration::ZERO)] {
                let limits = super::Limits {
                    time,
                    states: None,
                    memory,
                    clock: None,
                };
                let mut progress =
                    super::PegBagProgress::new(position.clone(), super::PegBagChoices::WHOLE)
                        .unwrap();
                let found = egs.resume_peg(&mut progress, &limits, 1).unwrap().unwrap();
                if time.is_some() {
                    assert!(!found.exact && !found.out_of_memory);
                    by_time += 1;
                } else if found.out_of_memory {
                    by_memory += 1;
                }
            }
        }
        assert_eq!(by_time, 2);
        assert!(by_memory > 0, "the memory limit alone stopped no search");
    }

    // past the limit from the start, no table grows
    #[test]
    #[inline]
    fn a_bag_search_holding_a_few_entries_answers_as_one_with_no_limit() {
        fn none_held() -> usize {
            0
        }
        fn all_held() -> usize {
            usize::MAX
        }
        // the bytes each table may grow to, and the solver's too
        const BYTES: usize = 512;
        fn within<T>(table: &super::BagTable<T>) -> bool {
            table.entries.len() * std::mem::size_of::<Option<(u64, T)>>() < BYTES
        }
        let small = super::Limits {
            time: None,
            states: None,
            memory: Some(super::MemoryLimit {
                bytes: BYTES,
                held: none_held,
                reserved: None,
            }),
            clock: None,
        };
        let past = super::Limits {
            time: None,
            states: None,
            memory: Some(super::MemoryLimit {
                bytes: BYTES,
                held: all_held,
                reserved: None,
            }),
            clock: None,
        };
        let gc = game_config::make_english_game_config();
        let kwg = kwg::kbwg_from_bytes_alloc(tiny_kwg_bytes().to_vec(), false).unwrap();
        let arm_tables = anagrams::ArmTables::build(&kwg, &gc, false).unwrap();
        let mut egs =
            EndgameSolver::<kwg::Node24, kwg::Node24>::new(&gc, &kwg, &arm_tables).unwrap();
        let mut board = empty_board();
        put_word(&mut board, 7, 6, &[1, 20]); // AT
        let (mut gave_up, mut checked, mut refused) = (0, 0, 0);
        let model = super::PegModel::RandomDraws;
        let (unseen, bag_size) = (&[1u8, 2, 8][..], 2);
        let mut unseen_tally = vec![0u8; gc.alphabet().len() as usize];
        for &tile in unseen {
            unseen_tally[tile as usize] += 1;
        }
        let [x, y] = [1u8, 1]; // AA
        let mut position = bag_position(
            &board,
            &[x, y],
            &unseen_tally,
            bag_size,
            equity::Equity::ZERO,
        );
        position.model = model;
        let mut progress =
            super::PegBagProgress::new(position.clone(), super::PegBagChoices::WHOLE).unwrap();
        let full = egs.resume_peg(&mut progress, &super::Limits::NONE, 1);
        let full = full.unwrap().unwrap();
        let full_rows = moves_rows(&mut egs, &mut progress, &full, &super::Limits::NONE);
        let kept = |progress: &super::PegBagProgress| {
            (progress.state.searched.shards.iter())
                .map(|shard| {
                    let shard = shard.lock().unwrap();
                    shard.nodes.held + shard.infos.held
                })
                .sum::<usize>()
        };
        let kept_full = kept(&progress);
        let arenas = std::mem::take(&mut progress.arenas);
        {
            let threads = 1;
            let mut progress =
                super::PegBagProgress::new(position.clone(), super::PegBagChoices::WHOLE).unwrap();
            progress.arenas = arenas;
            let held = egs
                .resume_peg(&mut progress, &small, threads)
                .unwrap()
                .unwrap();
            if held.out_of_memory {
                assert!(!held.exact, "{unseen:?} {x} {y} {threads}");
                refused += 1;
            } else {
                assert!(
                    held.exact
                        && held.best_move == full.best_move
                        && held.win_pct == full.win_pct
                        && held.expected_margin == full.expected_margin
                        && moves_rows(&mut egs, &mut progress, &held, &super::Limits::NONE)[0]
                            == full_rows[0],
                    "{unseen:?} {x} {y} {threads}"
                );
            }
            for shard in progress.state.searched.shards.iter() {
                let shard = shard.lock().unwrap();
                assert!(within(&shard.nodes) && within(&shard.infos));
                assert!(within(&shard.endgames));
            }
            if kept(&progress) < kept_full {
                gave_up += 1;
            }
            let mut progress =
                super::PegBagProgress::new(position.clone(), super::PegBagChoices::WHOLE).unwrap();
            let stopped = egs
                .resume_peg(&mut progress, &past, threads)
                .unwrap()
                .unwrap();
            // with no other limit, only the memory stops it: stopped, it says
            // so and is not exact; ended exact, it was not out of memory
            if stopped.out_of_memory {
                assert!(!stopped.exact, "{unseen:?} {x} {y} {threads}");
            } else {
                assert!(stopped.exact, "{unseen:?} {x} {y} {threads}");
            }
            for shard in progress.state.searched.shards.iter() {
                let shard = shard.lock().unwrap();
                assert!(shard.nodes.entries.len() == 4);
                assert!(shard.infos.entries.len() == 4);
                assert!(shard.endgames.entries.len() == 4);
            }
        }
        checked += 1;
        assert!(gave_up > 0, "no table gave up what it kept");
        assert!(refused < checked, "every search was refused room");
        // the search with one tile in the bag and exchanges, its memo held to
        // its first four entries, against one with no limit
        let four = super::Limits {
            time: None,
            states: None,
            memory: Some(super::MemoryLimit {
                bytes: 4 * std::mem::size_of::<Option<(u64, equity::Equity)>>() + 1,
                held: none_held,
                reserved: None,
            }),
            clock: None,
        };
        let gc = game_config::make_exchange_test_game_config();
        let arm_tables = anagrams::ArmTables::build(&kwg, &gc, false).unwrap();
        let klv = klv::Klv::<kwg::Node24>::empty().unwrap();
        let mut sub =
            EndgameSolver::<kwg::Node24, kwg::Node24>::new(&gc, &kwg, &arm_tables).unwrap();
        let mut mg = movegen::KurniaMoveGenerator::new(&gc);
        let (mut gave_up, mut checked) = (0, 0);
        let (name, pos) = (embedded_positions().into_iter())
            .find(|(name, _)| name == "both-pass-only (BB vs HT)")
            .unwrap();
        let (bag_tile, mover) = (1u8, 0u8);
        let mut kept = [0; 2];
        let mut values = [equity::Equity::ZERO; 2];
        for (i, limits) in [&super::Limits::NONE, &four].into_iter().enumerate() {
            let limits = super::BagLimits::new(limits, None);
            let mut search = super::OneInBag {
                gc: &gc,
                kwg: &kwg,
                arm_tables: &arm_tables,
                klv: &klv,
                sub: &mut sub,
                mg: &mut mg,
                memo: &mut Default::default(),
                limits: &limits,
                exact: true,
                line: None,
                placed: &mut Default::default(),
            };
            values[i] = EndgameSolver::one_in_bag_minimax_ex(
                &mut search,
                &pos.board,
                &pos.racks,
                super::OneInBagTurn {
                    mover,
                    bag_tile,
                    passes: 0,
                    zeros: 0,
                    answering: false,
                },
            )
            .unwrap();
            assert!(search.exact && (i == 0 || search.memo.entries.len() == 4));
            kept[i] = search.memo.held;
        }
        assert_eq!(values[0], values[1], "{name} {bag_tile} {mover}");
        if kept[1] < kept[0] {
            gave_up += 1;
        }
        checked += 1;
        assert!(checked > 0 && gave_up > 0, "{gave_up} of {checked}");
    }

    // where the config ends the game at one pass, the first pass ends it in each
    // search, answering none: the mover's Q, which no word takes, can only pass,
    // and the game's end loses it against the other's rack. Under the english
    // rules the other moves next instead
    #[test]
    #[inline]
    fn a_pass_ends_the_game_where_the_config_ends_it_at_one_pass() {
        let english = game_config::make_english_game_config();
        let mut one_pass = english.clone();
        let game_config::GameConfig::Static(rules) = &mut one_pass;
        rules.num_passes_to_end = 1;
        let kwg = kwg::kbwg_from_bytes_alloc(tiny_kwg_bytes().to_vec(), false).unwrap();
        let e = equity::Equity::new;
        let mut board = empty_board();
        put_word(&mut board, 7, 6, &[1, 20]); // AT
        let (q, a, h, t) = (17u8, 1u8, 8u8, 20u8);
        let mut unseen_tally = vec![0u8; english.alphabet().len() as usize];
        for tile in [a, h, t] {
            unseen_tally[tile as usize] += 1;
        }
        for (gc, ends) in [(&one_pass, true), (&english, false)] {
            let arm_tables = anagrams::ArmTables::build(&kwg, gc, false).unwrap();
            let mut egs =
                EndgameSolver::<kwg::Node24, kwg::Node24>::new(gc, &kwg, &arm_tables).unwrap();
            // the bag empty, then holding the H: the Q (10 points) against the
            // other's A (1)
            egs.init(&board, [&[q][..], &[a][..]]);
            let empty = egs.solve(0).unwrap();
            egs.init(&board, [&[q][..], &[a][..]]);
            let one = egs.solve_one_in_bag(0, h).unwrap();
            assert_eq!((empty == e(-9000), one == e(-9000)), (ends, ends));
            // two in the bag, the other holding one of the A, H, and T (1, 4,
            // and 1)
            for model in [super::PegModel::RandomDraws, super::PegModel::InfoSets] {
                let mut position = bag_position(&board, &[q], &unseen_tally, 2, e(0));
                position.model = model;
                let result = egs
                    .solve_peg(position, &super::Limits::NONE, 1)
                    .unwrap()
                    .unwrap();
                assert!(result.exact);
                assert_eq!(result.expected_margin == e(-8000), ends);
            }
        }
    }

    // where the game's last turn was a pass, the mover's pass answers it and
    // ends the game in each search: the mover's Q, which no word takes, can only
    // pass, and the game's end loses it against the other's rack. With no pass
    // last, the other moves next instead
    #[test]
    #[inline]
    fn a_pass_answering_the_games_last_pass_ends_it() {
        let gc = game_config::make_english_game_config();
        let kwg = kwg::kbwg_from_bytes_alloc(tiny_kwg_bytes().to_vec(), false).unwrap();
        let arm_tables = anagrams::ArmTables::build(&kwg, &gc, false).unwrap();
        let e = equity::Equity::new;
        let mut board = empty_board();
        put_word(&mut board, 7, 6, &[1, 20]); // AT
        let (q, a, h, t) = (17u8, 1u8, 8u8, 20u8);
        let mut unseen_tally = vec![0u8; gc.alphabet().len() as usize];
        for tile in [a, h, t] {
            unseen_tally[tile as usize] += 1;
        }
        for (passes, ends) in [(1u16, true), (0, false)] {
            let mut egs =
                EndgameSolver::<kwg::Node24, kwg::Node24>::new(&gc, &kwg, &arm_tables).unwrap();
            // the bag empty, then holding the H: the Q (10 points) against the
            // other's A (1)
            egs.init_after_scoreless_turns(&board, [&[q][..], &[a][..]], passes, 1);
            let empty = egs.solve(0).unwrap();
            egs.init_after_scoreless_turns(&board, [&[q][..], &[a][..]], passes, 1);
            let one = egs.solve_one_in_bag(0, h).unwrap();
            assert_eq!((empty == e(-9000), one == e(-9000)), (ends, ends));
            // two in the bag, the other holding one of the A, H, and T (1, 4,
            // and 1)
            for model in [super::PegModel::RandomDraws, super::PegModel::InfoSets] {
                let mut position = bag_position(&board, &[q], &unseen_tally, 2, e(0));
                position.scoreless = [passes, 1];
                position.model = model;
                let result = egs
                    .solve_peg(position, &super::Limits::NONE, 1)
                    .unwrap()
                    .unwrap();
                assert!(result.exact);
                assert_eq!(result.expected_margin == e(-8000), ends);
            }
        }
        // a game said to have had more passes in a row than the config lets
        // happen, as a caller playing other rules may say, solves all the same,
        // the mover's pass ending it at once: four passes end the game here,
        // five are said
        let mut four_passes = gc.clone();
        let game_config::GameConfig::Static(rules) = &mut four_passes;
        rules.num_passes_to_end = 4;
        let arm_tables = anagrams::ArmTables::build(&kwg, &four_passes, false).unwrap();
        let mut egs =
            EndgameSolver::<kwg::Node24, kwg::Node24>::new(&four_passes, &kwg, &arm_tables)
                .unwrap();
        egs.init_after_scoreless_turns(&board, [&[q][..], &[a][..]], 5, 5);
        assert_eq!(egs.solve(0).unwrap(), e(-9000));
        egs.init_after_scoreless_turns(&board, [&[q][..], &[a][..]], 5, 5);
        assert_eq!(egs.solve_one_in_bag(0, h).unwrap(), e(-9000));
        let mut position = bag_position(&board, &[q], &unseen_tally, 2, e(0));
        position.scoreless = [5, 5];
        let result = egs
            .solve_peg(position, &super::Limits::NONE, 1)
            .unwrap()
            .unwrap();
        assert_eq!(result.expected_margin, e(-8000));
    }

    // with one tile exchanged at a time and passes ending the game only as six
    // scoreless turns, a pass answering a pass ends an exchange line before the
    // count does, which changes the value: on the empty board, the mover's B and
    // T against the other's H with an A in the bag are worth 2 to the mover,
    // where counting the turns instead would make them 1. The reference values
    // them the same
    #[test]
    #[inline]
    fn a_pass_answering_a_pass_ends_an_exchange_line_before_the_count() {
        let mut gc = game_config::make_exchange_test_game_config();
        let game_config::GameConfig::Static(rules) = &mut gc;
        rules.num_passes_to_end = 0;
        rules.num_zeros_to_end = 6;
        let kwg = kwg::kbwg_from_bytes_alloc(tiny_kwg_bytes().to_vec(), false).unwrap();
        let klv = klv::kbwg_from_bytes_alloc(klv::EMPTY_KLV_BYTES.to_vec()).unwrap();
        let arm_tables = anagrams::ArmTables::build(&kwg, &gc, false).unwrap();
        let mut mg = movegen::KurniaMoveGenerator::new(&gc);
        let board = empty_board();
        let racks = [vec![2u8, 20], vec![8u8]]; // B T, H
        let mut egs =
            EndgameSolver::<kwg::Node24, kwg::Node24>::new(&gc, &kwg, &arm_tables).unwrap();
        egs.init(&board, [&racks[0][..], &racks[1][..]]);
        assert_eq!(
            egs.solve_one_in_bag(0, 1).unwrap(),
            equity::Equity::new(2000)
        );
        let refv = reference_one_in_bag_ex(
            &mut Reference {
                gc: &gc,
                kwg: &kwg,
                klv: &klv,
                mg: &mut mg,
                memo: Default::default(),
            },
            &board,
            &racks,
            super::OneInBagTurn {
                mover: 0,
                bag_tile: 1,
                passes: 0,
                zeros: 0,
                answering: false,
            },
        );
        assert_eq!(super::to_equity(refv), equity::Equity::new(2000));
    }

    #[test]
    #[inline]
    fn a_bag_search_where_nobody_can_place_passes_to_the_end() {
        // no words: every move is a pass, and each player loses their rack when
        // the game ends (the search's line at the pass that answers one inside
        // it, the game at the sixth scoreless turn, both its end alike). The
        // mover's B (3 points) against the opponent's one tile: A or T (1) loses
        // by 2, H (4) wins by 1, each split weighed by the deals that give it
        let gc = game_config::make_english_game_config();
        let kwg = kwg::kbwg_from_bytes_alloc(b"\x00\x00\x40\x00".to_vec(), false).unwrap();
        let arm_tables = anagrams::ArmTables::build(&kwg, &gc, false).unwrap();
        let mut egs =
            EndgameSolver::<kwg::Node24, kwg::Node24>::new(&gc, &kwg, &arm_tables).unwrap();
        let e = equity::Equity::new;
        let mut board = empty_board();
        put_word(&mut board, 7, 6, &[1, 20]); // AT
        for (unseen, hyps, win_pct, margin) in [
            (
                &[1u8, 8, 20][..],
                vec![
                    (vec![8, 20], 1, e(-2000)),
                    (vec![1, 20], 1, e(1000)),
                    (vec![1, 8], 1, e(-2000)),
                ],
                1.0 / 3.0,
                e(-1000),
            ),
            (
                &[1, 1, 8],
                vec![(vec![1, 8], 2, e(-2000)), (vec![1, 1], 1, e(1000))],
                1.0 / 3.0,
                e(-1000),
            ),
        ] {
            let mut unseen_tally = vec![0u8; gc.alphabet().len() as usize];
            for &tile in unseen {
                unseen_tally[tile as usize] += 1;
            }
            let (result, rows) =
                solve_peg_rows(&mut egs, bag_position(&board, &[2], &unseen_tally, 2, e(0)));
            assert!(result.exact);
            assert!(
                matches!(&result.best_move, Some(movegen::Play::Exchange { tiles }) if tiles.is_empty())
            );
            assert_eq!(result.moves.len(), 1);
            let mut got = rows[0].hypotheses.clone();
            got.sort_unstable();
            let mut want = hyps.clone();
            want.sort_unstable();
            assert_eq!(got, want, "{unseen:?}");
            assert!((result.win_pct - win_pct).abs() < 1e-9, "{unseen:?}");
            assert_eq!(result.expected_margin, margin, "{unseen:?}");
        }
    }

    #[test]
    #[inline]
    fn a_placement_with_tiles_in_the_bag_is_valued_over_its_splits_and_draws() {
        // each placement's share of wins and mean margin, found again apart from
        // the search: over each split of the unseen tiles and each draw of the
        // bag, the endgame once the draw empties the bag, else the one-in-bag
        // value of the tile left, each solved on its own, weighed by the
        // deals; a bag holding a tile twice weighs its draws unequally
        let gc = game_config::make_english_game_config();
        let kwg = kwg::kbwg_from_bytes_alloc(tiny_kwg_bytes().to_vec(), false).unwrap();
        let arm_tables = anagrams::ArmTables::build(&kwg, &gc, false).unwrap();
        let mut egs =
            EndgameSolver::<kwg::Node24, kwg::Node24>::new(&gc, &kwg, &arm_tables).unwrap();
        let mut own =
            EndgameSolver::<kwg::Node24, kwg::Node24>::new(&gc, &kwg, &arm_tables).unwrap();
        let e = equity::Equity::new;
        let mut board = empty_board();
        put_word(&mut board, 7, 6, &[1, 20]); // AT
        let (mover_rack, score_diff, bag_size) = (&[1u8, 1][..], e(2000), 2); // AA
        let (mut placements, mut drawn_one, mut drawn_two, mut unequal) = (0, 0, 0, 0);
        for unseen in [&[1u8, 2, 8][..], &[1, 8, 20, 20]] {
            let mut unseen_tally = vec![0u8; gc.alphabet().len() as usize];
            for &tile in unseen {
                unseen_tally[tile as usize] += 1;
            }
            let opp_len = unseen.len() - bag_size;
            let (result, rows) = solve_peg_rows(
                &mut egs,
                bag_position(&board, mover_rack, &unseen_tally, bag_size, score_diff),
            );
            assert!(result.exact);
            let lead = f64::from(score_diff.raw());
            for (
                super::PegBagMove {
                    play,
                    win_pct: win,
                    expected_margin: margin,
                    ..
                },
                super::PegBagRows {
                    draws: own_draws, ..
                },
            ) in result.moves.iter().zip(&rows)
            {
                let movegen::Play::Place {
                    down,
                    lane,
                    idx,
                    word,
                    score,
                } = play
                else {
                    continue;
                };
                let n = word.iter().filter(|&&tile| tile != 0).count();
                if n + 1 < bag_size {
                    continue;
                }
                let n = n.min(bag_size);
                let mut placed = board.clone();
                let mut kept = mover_rack.to_vec();
                let strider = gc.board_layout().dim().lane(*down, *lane);
                for (i, &tile) in (*idx..).zip(word.iter()) {
                    if tile != 0 {
                        placed[strider.at_in_lane(i)] = tile;
                        let p = kept.iter().position(|&t| t == tile).unwrap();
                        kept.remove(p);
                    }
                }
                let (mut sum, mut wins, mut total) = (0.0f64, 0.0f64, 0u64);
                // each split's draws, in the order the reference walks them,
                // its own
                let mut split_at = 0usize;
                super::each_split(&unseen_tally, opp_len, &mut |opp, bag, ways| {
                    let split_draws = &own_draws[split_at];
                    split_at += 1;
                    let mut draw_at = 0usize;
                    let opp_rack = (0u8..)
                        .zip(opp)
                        .flat_map(|(t, &c)| std::iter::repeat_n(t, c as usize))
                        .collect::<Vec<_>>();
                    super::each_split(bag, n, &mut |drawn, left, draws| {
                        let mut rack = kept.clone();
                        for (t, &c) in (0u8..).zip(drawn) {
                            rack.extend(std::iter::repeat_n(t, c as usize));
                        }
                        own.init(&placed, [&rack, &opp_rack]);
                        let theirs = match left.iter().position(|&c| c != 0) {
                            None => {
                                drawn_two += 1;
                                own.solve(1).unwrap()
                            }
                            Some(t) => {
                                drawn_one += 1;
                                own.solve_one_in_bag(1, t as u8).unwrap()
                            }
                        };
                        if draws > 1 {
                            unequal += 1;
                        }
                        let value = f64::from(score.raw()) - f64::from(theirs.raw()) + lead;
                        let drawn_here = &split_draws[draw_at];
                        draw_at += 1;
                        assert_eq!(drawn_here.ways, draws);
                        assert!((f64::from(drawn_here.margin.raw()) - value).abs() <= 1.0);
                        assert_eq!(drawn_here.win, super::beats(value, 0.0));
                        let w = (ways * draws) as f64;
                        sum += w * value;
                        wins += w * super::beats(value, 0.0);
                        total += ways * draws;
                    });
                    assert_eq!(draw_at, split_draws.len());
                });
                assert_eq!(split_at, own_draws.len());
                let case = (
                    unseen, bag_size, mover_rack, score_diff, *down, *lane, *idx, *score,
                );
                assert!(
                    (f64::from(margin.unwrap().raw()) - sum / total as f64).abs() <= 1.0,
                    "{case:?}: {margin:?} against {}",
                    sum / total as f64
                );
                assert!((win - wins / total as f64).abs() < 1e-9, "{case:?}");
                placements += 1;
            }
        }
        assert!(
            placements > 0 && drawn_one > 0 && drawn_two > 0 && unequal > 0,
            "{placements} placements, {drawn_one} draws leaving one tile, {drawn_two} leaving \
             none, {unequal} draws weighed more than once"
        );
    }

    // a reference search with tiles in the bag: the game, its tables, the
    // generator, a solver for the endgames once the bag is empty, and what it
    // has valued, by key
    struct BagReference<'r, 's, N: kwg::Node, L: kwg::Node, V> {
        gc: &'s game_config::GameConfig,
        kwg: &'s kwg::Kwg<N>,
        arm_tables: &'s anagrams::ArmTables,
        klv: &'s klv::Klv<L>,
        mg: &'r mut movegen::KurniaMoveGenerator,
        own: &'r mut EndgameSolver<'s, N, L>,
        known: fash::MyHashMap<u64, V>,
    }

    // the value to its mover of a position with tiles in the bag found with
    // nothing ruled out: every move valued (bag_reference_move), the most kept;
    // a position whose bag is empty is the endgame's, each other one valued once
    fn bag_reference<N: kwg::Node, L: kwg::Node>(
        r: &mut BagReference<'_, '_, N, L, f64>,
        pos: &super::InBag,
    ) -> f64 {
        if pos.bag_len() == 0 {
            let racks = [&pos.racks[0][..], &pos.racks[1][..]];
            let (passes, zeros) = (u16::from(pos.passes), u16::from(pos.zeros));
            r.own
                .init_after_scoreless_turns(&pos.board, racks, passes, zeros);
            return f64::from(r.own.solve(pos.mover).unwrap().raw());
        }
        let key = pos.key();
        if let Some(&value) = r.known.get(&key) {
            return value;
        }
        let mut value = f64::NEG_INFINITY;
        let mut lists = super::PlyLists::default();
        super::in_bag_moves::<false, _, _, _, true, true>(
            r.gc,
            r.kwg,
            r.arm_tables,
            r.klv,
            r.mg,
            pos,
            (
                &mut lists,
                &super::BagLimits::new(&super::Limits::NONE, None),
            ),
        )
        .unwrap();
        for play in lists.moves {
            value = value.max(bag_reference_move(r, pos, &play));
        }
        r.known.insert(key, value);
        value
    }

    // first_draw and next_draw walk every draw each_split gives, in its order
    // and with its deals, for pools with gaps, repeats, and too few tiles
    #[test]
    fn draws_walk_as_each_split_splits() {
        let pools: [&[u8]; 6] = [&[], &[0, 0], &[1], &[2, 0, 3, 1], &[0, 4, 1, 0, 2], &[1, 1]];
        for pool in pools {
            for n in 0..=pool.iter().map(|&count| count as usize).sum::<usize>() + 1 {
                let mut split = Vec::new();
                super::each_split(pool, n, &mut |drawn, _, ways| {
                    split.push((drawn.to_vec(), ways))
                });
                let mut walked = Vec::new();
                let mut drawn = vec![0u8; pool.len()];
                let mut more = super::first_draw(pool, n, &mut drawn);
                while more {
                    walked.push((drawn.clone(), super::draw_ways(pool, &drawn)));
                    more = super::next_draw(pool, &mut drawn);
                }
                assert_eq!(walked, split, "pool {pool:?}, n {n}");
            }
        }
    }

    // the position after its mover places play, before they draw, a copy
    #[inline]
    fn placed(
        pos: &super::InBag,
        gc: &game_config::GameConfig,
        play: &movegen::Play,
    ) -> super::InBag {
        let mut next = pos.clone();
        next.place(gc, play);
        next
    }

    // a move's value to its mover, as bag_reference finds it: a placement's
    // score less the next mover's value over each draw, weighed by the deals; a
    // pass the game's end or less the next mover's value
    fn bag_reference_move<N: kwg::Node, L: kwg::Node>(
        r: &mut BagReference<'_, '_, N, L, f64>,
        pos: &super::InBag,
        play: &movegen::Play,
    ) -> f64 {
        let gc = r.gc;
        match play {
            movegen::Play::Place { word, score, .. } => {
                let placed = placed(pos, gc, play);
                let n = (word.iter().filter(|&&tile| tile != 0).count()).min(pos.bag_len());
                let (mut sum, mut total) = (0.0, 0u64);
                let mut draws = Vec::new();
                super::each_split(&pos.bag, n, &mut |drawn, _, ways| {
                    draws.push((drawn.to_vec(), ways))
                });
                for (drawn, ways) in draws {
                    let mut next = placed.clone();
                    next.draw(&drawn);
                    next.next_turn();
                    let theirs = bag_reference(r, &next);
                    sum += ways as f64 * theirs;
                    total += ways;
                }
                f64::from(score.raw()) - sum / total as f64
            }
            movegen::Play::Exchange { tiles } if tiles.is_empty() => {
                // a pass answering a pass ends the game, as one bringing the
                // scoreless turns to the config's count does
                let mut next = pos.clone();
                next.passes = pos.passes.saturating_add(1);
                next.zeros = pos.zeros.saturating_add(1);
                next.answering = true;
                if pos.answering
                    || super::scoreless_turns_end(gc, Some(&pos.board), next.passes, next.zeros)
                {
                    return pos.racks_lost(gc);
                }
                next.next_turn();
                -bag_reference(r, &next)
            }
            movegen::Play::Exchange { .. } => unreachable!("too few in the bag to exchange"),
        }
    }

    // a move's value to its mover as a pass of this depth whose passes
    // estimate finds it, ply moves below the root: as bag_reference_move, but
    // a line that still leaves tiles in the bag at the pass's last ply is
    // valued by what it scores
    fn bag_reference_move_to<N: kwg::Node, L: kwg::Node>(
        r: &mut BagReference<'_, '_, N, L, f64>,
        pos: &super::InBag,
        play: &movegen::Play,
        ply: usize,
        depth: usize,
    ) -> f64 {
        let gc = r.gc;
        let horizon = ply + 1 >= depth;
        match play {
            movegen::Play::Place { word, score, .. } => {
                let placed = placed(pos, gc, play);
                let n = word.iter().filter(|&&tile| tile != 0).count();
                if n < pos.bag_len() && horizon {
                    return f64::from(score.raw());
                }
                let (mut sum, mut total) = (0.0, 0u64);
                let mut draws = Vec::new();
                super::each_split(&pos.bag, n.min(pos.bag_len()), &mut |drawn, _, ways| {
                    draws.push((drawn.to_vec(), ways))
                });
                for (drawn, ways) in draws {
                    let mut next = placed.clone();
                    next.draw(&drawn);
                    next.next_turn();
                    let theirs = if next.bag_len() == 0 {
                        bag_reference(r, &next)
                    } else {
                        bag_reference_to(r, &next, ply + 1, depth)
                    };
                    sum += ways as f64 * theirs;
                    total += ways;
                }
                f64::from(score.raw()) - sum / total as f64
            }
            movegen::Play::Exchange { tiles } if tiles.is_empty() => {
                if pos.a_pass_ends(gc) {
                    return pos.racks_lost(gc);
                }
                if horizon {
                    return 0.0;
                }
                let mut next = pos.clone();
                next.pass();
                next.next_turn();
                -bag_reference_to(r, &next, ply + 1, depth)
            }
            movegen::Play::Exchange { .. } => unreachable!("too few in the bag to exchange"),
        }
    }

    // a position's value to its mover as bag_reference_move_to finds its moves
    fn bag_reference_to<N: kwg::Node, L: kwg::Node>(
        r: &mut BagReference<'_, '_, N, L, f64>,
        pos: &super::InBag,
        ply: usize,
        depth: usize,
    ) -> f64 {
        let mut lists = super::PlyLists::default();
        super::in_bag_moves::<false, _, _, _, true, true>(
            r.gc,
            r.kwg,
            r.arm_tables,
            r.klv,
            r.mg,
            pos,
            (
                &mut lists,
                &super::BagLimits::new(&super::Limits::NONE, None),
            ),
        )
        .unwrap();
        (lists.moves.iter())
            .map(|play| bag_reference_move_to(r, pos, play, ply, depth))
            .fold(f64::NEG_INFINITY, f64::max)
    }

    #[test]
    #[inline]
    fn a_bag_search_values_each_move_as_a_search_that_rules_nothing_out() {
        // every move's mean margin with two in the bag, against
        // bag_reference's over the same splits: whatever the search rules out
        // unvalued never changes a value. Passes and one-tile placements leave
        // tiles in the bag to the next mover
        let gc = game_config::make_english_game_config();
        let kwg = kwg::kbwg_from_bytes_alloc(tiny_kwg_bytes().to_vec(), false).unwrap();
        let arm_tables = anagrams::ArmTables::build(&kwg, &gc, false).unwrap();
        let mut egs =
            EndgameSolver::<kwg::Node24, kwg::Node24>::new(&gc, &kwg, &arm_tables).unwrap();
        let mut own =
            EndgameSolver::<kwg::Node24, kwg::Node24>::new(&gc, &kwg, &arm_tables).unwrap();
        let klv = klv::Klv::<kwg::Node24>::empty().unwrap();
        let mut mg = movegen::KurniaMoveGenerator::new(&gc);
        let e = equity::Equity::new;
        let mut board = empty_board();
        put_word(&mut board, 7, 6, &[1, 20]); // AT
        let (mut checked, mut leaving) = (0, 0);
        let (unseen, bag_size) = (&[1u8, 2, 8][..], 2);
        let mut unseen_tally = vec![0u8; gc.alphabet().len() as usize];
        for &tile in unseen {
            unseen_tally[tile as usize] += 1;
        }
        let [x, y] = [1u8, 2]; // AB
        let position = bag_position(&board, &[x, y], &unseen_tally, bag_size, e(1000));
        let result = uncut_peg(&mut egs, position.clone()).unwrap().unwrap();
        assert!(result.exact);
        let progress = super::PegBagProgress::new(position.clone(), super::PegBagChoices::WHOLE)
            .and_then(super::PegBagProgress::without_cuts)
            .unwrap();
        let mut reference = BagReference {
            gc: &gc,
            kwg: &kwg,
            arm_tables: &arm_tables,
            klv: &klv,
            mg: &mut mg,
            own: &mut own,
            known: fash::MyHashMap::default(),
        };
        for super::PegBagMove {
            play,
            expected_margin: margin,
            ..
        } in &result.moves
        {
            let (mut sum, mut total) = (0.0, 0u64);
            for (opp_rack, bag, ways) in &progress.splits {
                let pos = super::InBag {
                    board: board.clone(),
                    racks: [progress.position.mover_rack.clone(), opp_rack.clone()],
                    bag: bag.clone(),
                    mover: 0,
                    passes: 0,
                    zeros: 0,
                    answering: false,
                    exchanges: [0, 0],
                };
                let value = bag_reference_move(&mut reference, &pos, play);
                sum += *ways as f64 * (value + 1000.0);
                total += ways;
            }
            let want = sum / total as f64;
            let case = (unseen, bag_size, x, y);
            assert!(
                (f64::from(margin.unwrap().raw()) - want).abs() <= 1.0,
                "{case:?}: {margin:?} against {want}"
            );
            if match play {
                movegen::Play::Place { word, .. } => {
                    word.iter().filter(|&&tile| tile != 0).count() < bag_size
                }
                movegen::Play::Exchange { .. } => true,
            } {
                leaving += 1;
            }
            checked += 1;
        }
        assert!(
            checked > 0 && leaving > 0,
            "{checked} moves, {leaving} leaving the bag"
        );
    }

    // what the player to move at info gets under InfoSets, found with nothing
    // ruled out: every move's mean over every split (info_reference_split), the
    // best kept, the chance to win first; each info set valued once
    fn info_reference<N: kwg::Node, L: kwg::Node>(
        r: &mut BagReference<'_, '_, N, L, (f64, f64)>,
        info: &super::InfoSet,
    ) -> (f64, f64) {
        if let Some(&value) = r.known.get(&info.key()) {
            return value;
        }
        let made = info.exchanges[info.mover as usize];
        let mut moves = Vec::new();
        super::moves_of::<false, _, _, _, true, true>(
            r.mg,
            &movegen::BoardSnapshot {
                board_tiles: &info.board,
                game_config: r.gc,
                kwg: r.kwg,
                anagrams: r.arm_tables.anagrams(),
                rack_lengths: r.arm_tables.rack_lengths(),
                klv: r.klv,
            },
            &info.rack,
            made,
            info.bag_len,
            (&mut moves, &mut Vec::new()),
            &super::BagLimits::new(&super::Limits::NONE, None),
        )
        .unwrap();
        // the splits, as each_split gives them
        let unseen = info
            .unseen
            .iter()
            .map(|&count| count as usize)
            .sum::<usize>();
        let mut splits = Vec::new();
        super::each_split(
            &info.unseen,
            unseen - info.bag_len,
            &mut |rack, bag, ways| {
                splits.push((super::tiles_of(rack), bag.to_vec(), ways));
            },
        );
        let total = splits.iter().map(|&(_, _, ways)| ways).sum::<u64>() as f64;
        let mut best = (f64::NEG_INFINITY, f64::NEG_INFINITY);
        for play in &moves {
            let (mut win, mut margin) = (0.0, 0.0);
            for (other, bag, ways) in &splits {
                let (w, m) = info_reference_split(r, info, play, other, bag);
                win += *ways as f64 * w;
                margin += *ways as f64 * m;
            }
            let (win, margin) = (win / total, margin / total);
            if win > best.0 || (win == best.0 && margin > best.1) {
                best = (win, margin);
            }
        }
        r.known.insert(info.key(), best);
        best
    }

    // a move's chance to win and margin at info in one split, as info_reference
    // finds it: the endgame once the bag empties, else the other's own view
    // after the move, turned to the mover's
    fn info_reference_split<N: kwg::Node, L: kwg::Node>(
        r: &mut BagReference<'_, '_, N, L, (f64, f64)>,
        info: &super::InfoSet,
        play: &movegen::Play,
        other: &[u8],
        bag: &[u8],
    ) -> (f64, f64) {
        let gc = r.gc;
        let lead = f64::from(info.lead);
        let mut pos = super::InBag::default();
        let limits = super::BagLimits::new(&super::Limits::NONE, None);
        info.at(&limits, other, bag, &mut pos).unwrap();
        // what the other sees: their rack, and the mover's given tiles with the
        // bag's
        let theirs =
            |board: &[u8], mine: &[u8], bag_len: usize, scoreless: [u8; 2], answering, lead| {
                let mut unseen = bag.to_vec();
                for &tile in mine {
                    unseen[tile as usize] += 1;
                }
                super::InfoSet {
                    board: board.to_vec(),
                    rack: other.to_vec(),
                    unseen,
                    bag_len,
                    mover: info.mover ^ 1,
                    passes: scoreless[0],
                    zeros: scoreless[1],
                    answering,
                    exchanges: info.exchanges,
                    lead,
                }
            };
        match play {
            movegen::Play::Place { word, score, .. } => {
                let n = word.iter().filter(|&&tile| tile != 0).count();
                let placed = placed(&pos, gc, play);
                if n >= info.bag_len {
                    let mut racks = placed.racks.clone();
                    for (tile, &count) in (0u8..).zip(bag) {
                        racks[info.mover as usize]
                            .extend(std::iter::repeat_n(tile, count as usize));
                    }
                    r.own.init(&placed.board, [&racks[0], &racks[1]]);
                    let ev = f64::from(r.own.solve(info.mover ^ 1).unwrap().raw());
                    let margin = f64::from(score.raw()) - ev;
                    return (super::beats(lead + margin, 0.0), margin);
                }
                let next = theirs(
                    &placed.board,
                    &placed.racks[info.mover as usize],
                    info.bag_len - n,
                    [0, 0],
                    false,
                    -(info.lead + score.raw()),
                );
                let (w, m) = info_reference(r, &next);
                (1.0 - w, f64::from(score.raw()) - m)
            }
            movegen::Play::Exchange { tiles } if tiles.is_empty() => {
                let scoreless = [info.passes + 1, info.zeros + 1];
                if info.answering
                    || super::scoreless_turns_end(gc, Some(&info.board), scoreless[0], scoreless[1])
                {
                    let lost = pos.racks_lost(gc);
                    return (super::beats(lead + lost, 0.0), lost);
                }
                let next = theirs(
                    &info.board,
                    &info.rack,
                    info.bag_len,
                    scoreless,
                    true,
                    -info.lead,
                );
                let (w, m) = info_reference(r, &next);
                (1.0 - w, -m)
            }
            movegen::Play::Exchange { .. } => unreachable!("too few in the bag to exchange"),
        }
    }

    #[test]
    #[inline]
    fn a_search_over_what_each_player_knows_values_each_move_as_one_that_keeps_nothing() {
        // every move's chance to win and mean margin under InfoSets against
        // info_reference's over the same splits (the bag holding two tiles or
        // three): neither the search's table, its tried-first moves, nor the
        // moves it stops early change a value. A move that leaves tiles in the
        // bag hands the other a blind choice over splits of their own
        let gc = game_config::make_english_game_config();
        let kwg = kwg::kbwg_from_bytes_alloc(tiny_kwg_bytes().to_vec(), false).unwrap();
        let arm_tables = anagrams::ArmTables::build(&kwg, &gc, false).unwrap();
        let mut egs =
            EndgameSolver::<kwg::Node24, kwg::Node24>::new(&gc, &kwg, &arm_tables).unwrap();
        let mut own =
            EndgameSolver::<kwg::Node24, kwg::Node24>::new(&gc, &kwg, &arm_tables).unwrap();
        let klv = klv::Klv::<kwg::Node24>::empty().unwrap();
        let mut mg = movegen::KurniaMoveGenerator::new(&gc);
        let e = equity::Equity::new;
        let mut board = empty_board();
        put_word(&mut board, 7, 6, &[1, 20]); // AT
        let (mut checked, mut leaving) = (0, 0);
        // the unseen tiles, the bag's size, and the rack (A B H T as 1 2 8 20)
        for (unseen, bag_size, [x, y]) in [
            (&[1u8, 2, 8][..], 2, [1u8, 1]),
            (&[1, 8, 20, 20], 2, [2, 2]),
            (&[2, 8, 20, 20], 3, [1, 8]),
        ] {
            let mut unseen_tally = vec![0u8; gc.alphabet().len() as usize];
            for &tile in unseen {
                unseen_tally[tile as usize] += 1;
            }
            let lead = e(1000);
            let mut position = bag_position(&board, &[x, y], &unseen_tally, bag_size, lead);
            position.model = super::PegModel::InfoSets;
            let (result, rows) = solve_peg_rows(&mut egs, position.clone());
            assert!(result.exact);
            let progress =
                super::PegBagProgress::new(position.clone(), super::PegBagChoices::WHOLE)
                    .and_then(super::PegBagProgress::without_cuts)
                    .unwrap();
            let mut reference = BagReference {
                gc: &gc,
                kwg: &kwg,
                arm_tables: &arm_tables,
                klv: &klv,
                mg: &mut mg,
                own: &mut own,
                known: fash::MyHashMap::default(),
            };
            for (
                super::PegBagMove {
                    play,
                    win_pct: win,
                    expected_margin: margin,
                    ..
                },
                super::PegBagRows {
                    hypotheses,
                    wins,
                    draws,
                },
            ) in result.moves.iter().zip(&rows)
            {
                // each split's own row and chance, as the reference values
                // that split; no draws at the root in this model, the mover's
                // draw unseen by the other
                assert_eq!(draws.len(), progress.splits.len());
                assert!(draws.iter().all(Vec::is_empty));
                assert_eq!(hypotheses.len(), progress.splits.len());
                assert_eq!(wins.len(), progress.splits.len());
                let (mut want_win, mut want_margin, mut total) = (0.0, 0.0, 0u64);
                for (split_at, (opp_rack, bag, ways)) in progress.splits.iter().enumerate() {
                    let pos = super::InBag {
                        board: board.clone(),
                        racks: [progress.position.mover_rack.clone(), opp_rack.clone()],
                        bag: bag.clone(),
                        mover: 0,
                        passes: 0,
                        zeros: 0,
                        answering: false,
                        exchanges: [0, 0],
                    };
                    let mut info = super::InfoSet::default();
                    let limits = super::BagLimits::new(&super::Limits::NONE, None);
                    info.of(&limits, &pos, lead.raw()).unwrap();
                    let (w, m) = info_reference_split(&mut reference, &info, play, opp_rack, bag);
                    want_win += *ways as f64 * w;
                    want_margin += *ways as f64 * (m + f64::from(lead.raw()));
                    total += ways;
                    assert!((wins[split_at] - w).abs() < 1e-9, "split {split_at}");
                    assert_eq!(hypotheses[split_at].1, *ways);
                    assert!(
                        (f64::from(hypotheses[split_at].2.raw()) - (m + f64::from(lead.raw())))
                            .abs()
                            <= 1.0,
                        "split {split_at}"
                    );
                }
                let (want_win, want_margin) = (want_win / total as f64, want_margin / total as f64);
                let case = (unseen, bag_size, x, y);
                assert!(
                    (win - want_win).abs() < 1e-9,
                    "{case:?}: {win} against {want_win}"
                );
                assert!(
                    (f64::from(margin.unwrap().raw()) - want_margin).abs() <= 1.0,
                    "{case:?}: {margin:?} against {want_margin}"
                );
                if match play {
                    movegen::Play::Place { word, .. } => {
                        word.iter().filter(|&&tile| tile != 0).count() < bag_size
                    }
                    movegen::Play::Exchange { .. } => true,
                } {
                    leaving += 1;
                }
                checked += 1;
            }
        }
        assert!(
            checked == 36 && leaving == 30,
            "{checked} moves, {leaving} leaving the bag"
        );
    }

    #[test]
    #[inline]
    fn a_bag_search_exchanges_and_draws_at_random() {
        // no words, exchanges legal with a tile in the bag; two passes in a row
        // or three scoreless turns end the game, each player losing their rack.
        // Worked by hand: our B (3) against one of A, H, T (1, 4, 1), the other
        // two in the bag. Every turn is scoreless, so the third ends it; the
        // last mover passes unless the bag's mean is less than their tile.
        // Their A: our pass loses 2 (they pass back); our exchange of B draws H
        // or T. On H they pass and we exchange again, losing 1 on average; on T
        // all pass, for 0; so -0.5. Their H: our pass loses 1.5 (they
        // exchange); our exchange wins 1. Their T: as their A. So the exchange
        // has mean 0 and wins half the deals (ties half); the pass has mean
        // -11/6 and wins 1/12
        let gc = game_config::make_exchange_test_game_config();
        let kwg = kwg::kbwg_from_bytes_alloc(b"\x00\x00\x40\x00".to_vec(), false).unwrap();
        let arm_tables = anagrams::ArmTables::build(&kwg, &gc, false).unwrap();
        let mut egs =
            EndgameSolver::<kwg::Node24, kwg::Node24>::new(&gc, &kwg, &arm_tables).unwrap();
        let e = equity::Equity::new;
        let mut board = empty_board();
        put_word(&mut board, 7, 6, &[1, 20]); // AT
        let mut unseen_tally = vec![0u8; gc.alphabet().len() as usize];
        for tile in [1, 8, 20] {
            unseen_tally[tile] += 1;
        }
        let (result, rows) =
            solve_peg_rows(&mut egs, bag_position(&board, &[2], &unseen_tally, 2, e(0)));
        assert!(result.exact);
        assert!(
            matches!(&result.best_move, Some(movegen::Play::Exchange { tiles }) if tiles[..] == [2])
        );
        assert!((result.win_pct - 0.5).abs() < 1e-9);
        assert_eq!(result.expected_margin, e(0));
        let mut hyps = rows[0].hypotheses.clone();
        hyps.sort_unstable();
        assert_eq!(
            hyps,
            vec![
                (vec![1, 8], 1, e(-500)),
                (vec![1, 20], 1, e(1000)),
                (vec![8, 20], 1, e(-500)),
            ]
        );
        assert_eq!(result.moves.len(), 2);
        let super::PegBagMove {
            play: pass,
            win_pct: win,
            expected_margin: margin,
            ..
        } = &result.moves[1];
        assert!(matches!(pass, movegen::Play::Exchange { tiles } if tiles.is_empty()));
        assert!((win - 1.0 / 12.0).abs() < 1e-9);
        assert_eq!(*margin, Some(e(-1833)));
    }

    #[test]
    #[inline]
    fn a_bag_search_under_limits_answers_as_an_unlimited_one_or_says_it_was_cut() {
        // deepening limits that never stop it answer as no limit does; limits
        // that stop it answer inexactly or, exact, alike; and a stopped search
        // resumed with no limit answers as one never stopped
        let gc = game_config::make_english_game_config();
        let kwg = kwg::kbwg_from_bytes_alloc(tiny_kwg_bytes().to_vec(), false).unwrap();
        let arm_tables = anagrams::ArmTables::build(&kwg, &gc, false).unwrap();
        let mut egs =
            EndgameSolver::<kwg::Node24, kwg::Node24>::new(&gc, &kwg, &arm_tables).unwrap();
        let e = equity::Equity::new;
        let mut board = empty_board();
        put_word(&mut board, 7, 6, &[1, 20]); // AT
        let deepening = super::Limits {
            time: Some(std::time::Duration::from_secs(3600)),
            states: None,
            memory: None,
            clock: None,
        };
        let one_state = super::Limits {
            time: None,
            states: Some(1),
            memory: None,
            clock: None,
        };
        let no_time = super::Limits {
            time: Some(std::time::Duration::ZERO),
            states: None,
            memory: None,
            clock: None,
        };
        type Answer = (super::PegBagResult, Vec<super::PegBagRows>);
        let same = |a: &Answer, b: &Answer| {
            a.0.best_move == b.0.best_move
                && a.0.win_pct == b.0.win_pct
                && a.0.expected_margin == b.0.expected_margin
                && a.1.first() == b.1.first()
        };
        let mut cut = 0;
        let mut unseen_tally = vec![0u8; gc.alphabet().len() as usize];
        unseen_tally[1] = 1; // A
        unseen_tally[8] = 1; // H
        unseen_tally[20] = 2; // T
        let position = bag_position(&board, &[2, 2], &unseen_tally, 3, e(0));
        let full = solve_peg_rows(&mut egs, position.clone());
        assert!(full.0.exact && full.0.depth == super::COMPLETE);
        let mut progress =
            super::PegBagProgress::new(position.clone(), super::PegBagChoices::WHOLE).unwrap();
        let deep = egs.resume_peg(&mut progress, &deepening, 1);
        let deep = deep.unwrap().unwrap();
        let deep_rows = moves_rows(&mut egs, &mut progress, &deep, &deepening);
        let deep = (deep, deep_rows);
        assert!(deep.0.exact && same(&deep, &full));
        // on one thread or on three, each with its own share of the moves: the
        // same answers
        for threads in [1, 3] {
            let mut progress =
                super::PegBagProgress::new(position.clone(), super::PegBagChoices::WHOLE).unwrap();
            let on_threads = egs
                .resume_peg(&mut progress, &super::Limits::NONE, threads)
                .unwrap()
                .unwrap();
            let rows = moves_rows(&mut egs, &mut progress, &on_threads, &super::Limits::NONE);
            let on_threads = (on_threads, rows);
            assert!(on_threads.0.exact && same(&on_threads, &full));
            assert!(on_threads.0.moves.len() == full.0.moves.len());
            for limits in [&one_state, &no_time] {
                let mut progress =
                    super::PegBagProgress::new(position.clone(), super::PegBagChoices::WHOLE)
                        .unwrap();
                let stopped = egs
                    .resume_peg(&mut progress, limits, threads)
                    .unwrap()
                    .unwrap();
                if stopped.exact {
                    let rows = moves_rows(&mut egs, &mut progress, &stopped, &super::Limits::NONE);
                    let stopped = (stopped, rows);
                    assert!(same(&stopped, &full), "{threads}");
                } else {
                    cut += 1;
                }
                let resumed = egs
                    .resume_peg(&mut progress, &super::Limits::NONE, threads)
                    .unwrap()
                    .unwrap();
                let rows = moves_rows(&mut egs, &mut progress, &resumed, &super::Limits::NONE);
                let resumed = (resumed, rows);
                assert!(resumed.0.exact && same(&resumed, &full), "{threads}");
            }
        }
        assert!(cut > 0, "no limit cut a search short");
    }

    #[test]
    #[inline]
    fn a_bag_answers_moves_in_a_range_are_that_slice_of_them_all() {
        // stopped at each table size, and not stopped: each range of the
        // answer's moves is that slice of them all, best first
        let gc = game_config::make_english_game_config();
        let kwg = kwg::kbwg_from_bytes_alloc(tiny_kwg_bytes().to_vec(), false).unwrap();
        let arm_tables = anagrams::ArmTables::build(&kwg, &gc, false).unwrap();
        let mut board = empty_board();
        put_word(&mut board, 7, 6, &[1, 20]); // AT
        let mut unseen_tally = vec![0u8; gc.alphabet().len() as usize];
        for tile in [1u8, 8, 20, 20] {
            unseen_tally[tile as usize] += 1;
        }
        let position = bag_position(&board, &[1, 2], &unseen_tally, 3, equity::Equity::ZERO);
        // stopped with three moves valued or more, and past the first pass
        let (mut cut, mut later, mut ranged) = (0, 0, 0);
        for states in (1..8000).step_by(61).map(Some).chain([None]) {
            let limits = super::Limits {
                time: Some(std::time::Duration::from_secs(3600)),
                states,
                memory: None,
                clock: None,
            };
            let mut egs =
                EndgameSolver::<kwg::Node24, kwg::Node24>::new(&gc, &kwg, &arm_tables).unwrap();
            let mut progress =
                super::PegBagProgress::new(position.clone(), super::PegBagChoices::WHOLE)
                    .unwrap()
                    .without_cuts()
                    .unwrap();
            let best = egs.resume_peg_best(&mut progress, &limits, 1);
            let best = best.unwrap().unwrap();
            assert!(best.moves.is_empty());
            let all = progress.answer_moves(0..usize::MAX);
            assert_eq!(all.len(), progress.answer_total());
            assert!(best.best_move == all.first().map(|first| first.play.clone()));
            assert!(all.windows(2).all(|pair| {
                let (x, y) = (&pair[0], &pair[1]);
                x.win_pct > y.win_pct
                    || (x.win_pct == y.win_pct && x.expected_margin >= y.expected_margin)
            }));
            assert!(states.is_some() || best.exact);
            cut += usize::from(!best.exact && all.len() > 2);
            later += usize::from(!best.exact && best.depth > 0);
            for from in 0..=all.len() + 1 {
                for to in from..=all.len() + 1 {
                    let some = progress.answer_moves(from..to);
                    let (from, to) = (from.min(all.len()), to.min(all.len()));
                    assert_eq!(some.len(), to - from);
                    assert!(some.iter().zip(&all[from..to]).all(|(x, y)| {
                        x.play == y.play
                            && x.win_pct == y.win_pct
                            && x.expected_margin == y.expected_margin
                            && x.exact == y.exact
                    }));
                    ranged += usize::from(to - from > 1 && to - from < all.len());
                }
            }
            for (place, listed) in all.iter().enumerate() {
                assert_eq!(progress.answer_place(&listed.play), Some(place));
            }
        }
        assert!(cut > 0 && later > 0 && ranged > 0, "{cut} {later} {ranged}");
    }

    #[test]
    #[inline]
    fn a_one_in_bag_answers_moves_in_a_range_are_that_slice_of_them_all() {
        // stopped at each table size, and not stopped: each range of the
        // answer's moves is that slice of them all, best first
        let gc = game_config::make_english_game_config();
        let kwg = kwg::kbwg_from_bytes_alloc(tiny_kwg_bytes().to_vec(), false).unwrap();
        let arm_tables = anagrams::ArmTables::build(&kwg, &gc, false).unwrap();
        let mut board = empty_board();
        put_word(&mut board, 7, 6, &[1, 20]); // AT
        let mut unseen_tally = vec![0u8; gc.alphabet().len() as usize];
        for tile in [1u8, 8, 20] {
            unseen_tally[tile as usize] += 1;
        }
        // stopped with three moves valued or more, and past the first pass
        let (mut cut, mut later, mut ranged) = (0, 0, 0);
        for states in (1..4000).step_by(7).map(Some).chain([None]) {
            let limits = super::Limits {
                time: Some(std::time::Duration::from_secs(3600)),
                states,
                memory: None,
                clock: None,
            };
            let mut egs =
                EndgameSolver::<kwg::Node24, kwg::Node24>::new(&gc, &kwg, &arm_tables).unwrap();
            let mut progress = super::PegProgress::new(
                0,
                &board,
                &[1, 2],
                &unseen_tally,
                equity::Equity::ZERO,
                [0, 0],
            );
            let best = egs.resume_peg_one_in_bag_best(&mut progress, &limits, 1);
            let best = best.unwrap().unwrap();
            assert!(best.moves.is_empty());
            let all = progress.answer_moves(0..usize::MAX);
            assert_eq!(all.len(), progress.answer_total());
            assert!(best.best_move == all.first().map(|first| first.play.clone()));
            assert!(all.first().is_none_or(|first| Some(first.row) == best.row));
            assert!(all.windows(2).all(|pair| {
                let (x, y) = (&pair[0], &pair[1]);
                x.win_pct > y.win_pct
                    || (x.win_pct == y.win_pct && x.expected_margin >= y.expected_margin)
            }));
            assert!(states.is_some() || best.exact);
            cut += usize::from(!best.exact && all.len() > 2);
            later += usize::from(!best.exact && best.depth > 0);
            for from in 0..=all.len() + 1 {
                for to in from..=all.len() + 1 {
                    let some = progress.answer_moves(from..to);
                    let (from, to) = (from.min(all.len()), to.min(all.len()));
                    assert_eq!(some.len(), to - from);
                    assert!(some.iter().zip(&all[from..to]).all(|(x, y)| {
                        x.play == y.play
                            && x.win_pct == y.win_pct
                            && x.expected_margin == y.expected_margin
                            && x.row == y.row
                            && x.exact == y.exact
                    }));
                    ranged += usize::from(to - from > 1 && to - from < all.len());
                }
            }
            for (place, listed) in all.iter().enumerate() {
                assert_eq!(progress.answer_place(&listed.play), Some(place));
            }
        }
        assert!(cut > 0 && later > 0 && ranged > 0, "{cut} {later} {ranged}");
    }

    // whether two answers of a bag search are alike in everything but their
    // depth: the move, the values, the splits and their draws, and every move
    // listed with its own
    fn bag_answers_alike(
        (a, a_rows): (&super::PegBagResult, &[super::PegBagRows]),
        (b, b_rows): (&super::PegBagResult, &[super::PegBagRows]),
    ) -> bool {
        a.best_move == b.best_move
            && a.win_pct == b.win_pct
            && a.expected_margin == b.expected_margin
            && a.exact == b.exact
            && a.moves.len() == b.moves.len()
            && a.moves.iter().zip(&b.moves).all(|(x, y)| {
                x.play == y.play
                    && x.win_pct == y.win_pct
                    && x.expected_margin == y.expected_margin
                    && x.exact == y.exact
            })
            && a_rows == b_rows
    }

    #[test]
    #[inline]
    fn a_bag_search_whose_passes_estimate_ends_as_one_that_does_not() {
        // with no limit, a search whose passes estimate deepens until every
        // line reaches the bag's end, and then answers exactly as one that does
        // not estimate, under either model and on one thread or three
        let gc = game_config::make_english_game_config();
        let kwg = kwg::kbwg_from_bytes_alloc(tiny_kwg_bytes().to_vec(), false).unwrap();
        let arm_tables = anagrams::ArmTables::build(&kwg, &gc, false).unwrap();
        let mut egs =
            EndgameSolver::<kwg::Node24, kwg::Node24>::new(&gc, &kwg, &arm_tables).unwrap();
        let e = equity::Equity::new;
        let mut board = empty_board();
        put_word(&mut board, 7, 6, &[1, 20]); // AT
        let (random, info) = (super::PegModel::RandomDraws, super::PegModel::InfoSets);
        let mut refined = 0;
        for (unseen, bag_size, model, [x, y], threads) in [
            (&[1u8, 2, 8][..], 2, random, [1u8, 8], 1), // AH
            (&[1, 2, 8], 2, info, [1, 2], 3),           // AB
            (&[1, 8, 20, 20], 3, random, [1, 20], 1),   // AT
            (&[1, 8, 20, 20], 3, info, [2, 2], 1),      // BB
        ] {
            let mut unseen_tally = vec![0u8; gc.alphabet().len() as usize];
            for &tile in unseen {
                unseen_tally[tile as usize] += 1;
            }
            let mut position = bag_position(&board, &[x, y], &unseen_tally, bag_size, e(0));
            position.model = model;
            let (full, full_rows) = solve_peg_rows(&mut egs, position.clone());
            assert!(full.exact && full.depth == super::COMPLETE);
            {
                let mut progress = super::PegBagProgress::new(
                    position.clone(),
                    super::PegBagChoices {
                        part: (0, 1),
                        estimates: true,
                    },
                )
                .and_then(super::PegBagProgress::without_cuts)
                .unwrap();
                let estimated = egs
                    .resume_peg(&mut progress, &super::Limits::NONE, threads)
                    .unwrap()
                    .unwrap();
                let rows = moves_rows(&mut egs, &mut progress, &estimated, &super::Limits::NONE);
                assert!(
                    bag_answers_alike((&estimated, &rows), (&full, &full_rows)),
                    "{unseen:?} {x} {y} {threads}"
                );
                assert!(estimated.depth != super::COMPLETE);
                if estimated.depth > 1 {
                    refined += 1;
                }
            }
        }
        assert!(refined > 0, "no search needed a pass after its first");
    }

    #[test]
    #[inline]
    fn a_bag_search_whose_passes_estimate_values_every_move_before_its_limit() {
        // seven and nine in the bag: exchanges lie below the moves (at nine
        // below every one), so a search that does not estimate leaves moves
        // unvalued at a tight limit (at nine all of them). Estimating, its
        // first pass values every move by its equity, exchanges too, and the
        // answer says it is not exact
        let gc = game_config::make_english_game_config();
        let kwg = kwg::kbwg_from_bytes_alloc(tiny_kwg_bytes().to_vec(), false).unwrap();
        let arm_tables = anagrams::ArmTables::build(&kwg, &gc, false).unwrap();
        let mut board = empty_board();
        put_word(&mut board, 7, 6, &[1, 20]); // AT
        let rack = [2, 10, 17, 22, 23, 24, 26]; // B J Q V W X Z
        let limits = super::Limits {
            time: Some(std::time::Duration::from_millis(100)),
            states: None,
            memory: None,
            clock: None,
        };
        let mut checked = 0;
        for (unseen, bag_size, model) in [
            // C D E F G I K M
            (
                &[3usize, 4, 5, 6, 7, 9, 11, 13][..],
                7,
                super::PegModel::InfoSets,
            ),
            // and N O
            (
                &[3, 4, 5, 6, 7, 9, 11, 13, 14, 15][..],
                9,
                super::PegModel::RandomDraws,
            ),
        ] {
            let mut unseen_tally = vec![0u8; gc.alphabet().len() as usize];
            for &tile in unseen {
                unseen_tally[tile] = 1;
            }
            let threads = 1;
            let mut egs =
                EndgameSolver::<kwg::Node24, kwg::Node24>::new(&gc, &kwg, &arm_tables).unwrap();
            let mut position =
                bag_position(&board, &rack, &unseen_tally, bag_size, equity::Equity::ZERO);
            position.model = model;
            let mut progress = super::PegBagProgress::new(
                position,
                super::PegBagChoices {
                    part: (0, 1),
                    estimates: true,
                },
            )
            .unwrap();
            let t0 = std::time::Instant::now();
            let result = egs
                .resume_peg(&mut progress, &limits, threads)
                .unwrap()
                .unwrap();
            let took = t0.elapsed();
            assert!(
                took < std::time::Duration::from_secs(5),
                "a search of 100 ms took {took:?}"
            );
            let (_, moves) = progress.moves().unwrap();
            assert!(moves > 100, "{moves} moves: the exchanges missing");
            assert_eq!(result.moves.len(), moves, "{bag_size} {threads}");
            assert!(result.best_move.is_some());
            assert!(!result.exact);
            assert!(result.depth >= 1 && result.depth != super::COMPLETE);
            assert!(result.moves.iter().all(|valued| !valued.exact));
            assert!(result.moves.iter().any(|valued| matches!(
                &valued.play,
                movegen::Play::Exchange { tiles } if !tiles.is_empty()
            )));
            checked += 1;
        }
        assert_eq!(checked, 2);
        // with two in the bag, cut by a state limit after its second pass:
        // each move's value is the reference's to that depth, a placement that
        // empties the bag below the root valued by its endgame
        let mut egs =
            EndgameSolver::<kwg::Node24, kwg::Node24>::new(&gc, &kwg, &arm_tables).unwrap();
        let mut own =
            EndgameSolver::<kwg::Node24, kwg::Node24>::new(&gc, &kwg, &arm_tables).unwrap();
        let klv = klv::Klv::<kwg::Node24>::empty().unwrap();
        let mut mg = movegen::KurniaMoveGenerator::new(&gc);
        let mut unseen_tally = vec![0u8; gc.alphabet().len() as usize];
        for tile in [1, 2, 8] {
            unseen_tally[tile] = 1; // A B H
        }
        let position = bag_position(&board, &[1, 1], &unseen_tally, 2, equity::Equity::ZERO);
        // the least state limit whose answer is of the second pass
        let (result, progress) = (1..40)
            .find_map(|step| {
                let mut progress = super::PegBagProgress::new(
                    position.clone(),
                    super::PegBagChoices {
                        part: (0, 1),
                        estimates: true,
                    },
                )
                .unwrap()
                .without_cuts()
                .unwrap();
                let limits = super::Limits {
                    time: None,
                    states: Some(10 * step * step),
                    memory: None,
                    clock: None,
                };
                let result = egs.resume_peg(&mut progress, &limits, 1).unwrap().unwrap();
                (result.depth == 2).then_some((result, progress))
            })
            .unwrap();
        let mut reference = BagReference {
            gc: &gc,
            kwg: &kwg,
            arm_tables: &arm_tables,
            klv: &klv,
            mg: &mut mg,
            own: &mut own,
            known: fash::MyHashMap::default(),
        };
        for valued in &result.moves {
            let (mut sum, mut total) = (0.0, 0u64);
            for (opp_rack, bag, ways) in &progress.splits {
                let pos = super::InBag {
                    board: board.clone(),
                    racks: [progress.position.mover_rack.clone(), opp_rack.clone()],
                    bag: bag.clone(),
                    mover: 0,
                    passes: 0,
                    zeros: 0,
                    answering: false,
                    exchanges: [0, 0],
                };
                sum +=
                    *ways as f64 * bag_reference_move_to(&mut reference, &pos, &valued.play, 0, 2);
                total += ways;
            }
            let want = sum / total as f64;
            let margin = f64::from(valued.expected_margin.unwrap().raw());
            assert!((margin - want).abs() <= 1.0, "{margin} against {want}");
        }
    }

    #[test]
    fn a_bag_search_refused_memory_past_its_state_limit_is_not_out_of_memory() {
        let gc = game_config::make_english_game_config();
        let kwg = kwg::kbwg_from_bytes_alloc(tiny_kwg_bytes().to_vec(), false).unwrap();
        let arm_tables = anagrams::ArmTables::build(&kwg, &gc, false).unwrap();
        let mut board = empty_board();
        put_word(&mut board, 7, 6, &[1, 20]); // AT
        let mut unseen_tally = vec![0u8; gc.alphabet().len() as usize];
        for tile in [1u8, 8, 20, 20] {
            unseen_tally[tile as usize] += 1;
        }
        let position = bag_position(&board, &[1, 2], &unseen_tally, 3, equity::Equity::ZERO);
        let limits = super::Limits {
            states: Some(0),
            memory: Some(super::MemoryLimit {
                bytes: 1,
                held: || usize::MAX,
                reserved: None,
            }),
            ..super::Limits::NONE
        };
        let mut egs =
            EndgameSolver::<kwg::Node24, kwg::Node24>::new(&gc, &kwg, &arm_tables).unwrap();
        let result = egs.solve_peg(position, &limits, 1).unwrap().unwrap();
        assert!(!result.out_of_memory);
    }

    #[test]
    fn a_bag_search_with_cuts_values_again_past_a_pause() {
        use super::PegBagChange as C;
        let gc = game_config::make_english_game_config();
        let kwg = kwg::kbwg_from_bytes_alloc(tiny_kwg_bytes().to_vec(), false).unwrap();
        let arm_tables = anagrams::ArmTables::build(&kwg, &gc, false).unwrap();
        let mut egs =
            EndgameSolver::<kwg::Node24, kwg::Node24>::new(&gc, &kwg, &arm_tables).unwrap();
        let mut board = empty_board();
        put_word(&mut board, 7, 6, &[1, 20]); // AT
        let mut unseen_tally = vec![0u8; gc.alphabet().len() as usize];
        for tile in [1u8, 2, 8] {
            unseen_tally[tile as usize] += 1;
        }
        let position = bag_position(&board, &[1, 1], &unseen_tally, 2, equity::Equity::ZERO);
        let choices = super::PegBagChoices::WHOLE;
        let mut progress = super::PegBagProgress::new(position.clone(), choices).unwrap();
        let full = egs.resume_peg(&mut progress, &super::Limits::NONE, 1);
        let full = full.unwrap().unwrap();
        let first = full.moves[0].play.clone();
        let progress = super::PegBagProgress::new(position, choices).unwrap();
        let mut progress = progress.with_cuts().unwrap();
        let cut = egs.resume_peg(&mut progress, &super::Limits::NONE, 1);
        let cut = cut.unwrap().unwrap();
        assert!(cut.best_move == full.best_move && cut.win_pct == full.win_pct);
        progress.change(&first, C::Pause);
        let paused = egs.resume_peg(&mut progress, &super::Limits::NONE, 1);
        let paused = paused.unwrap().unwrap();
        assert!(paused.best_move.as_ref() == Some(&full.moves[1].play));
        assert_eq!(paused.win_pct, full.moves[1].win_pct);
        assert_eq!(Some(paused.expected_margin), full.moves[1].expected_margin);
        progress.change(&first, C::Resume);
        let resumed = egs.resume_peg(&mut progress, &super::Limits::NONE, 1);
        let resumed = resumed.unwrap().unwrap();
        assert!(resumed.best_move == full.best_move && resumed.win_pct == full.win_pct);
    }

    #[test]
    fn a_bag_search_takes_a_sims_changes_to_its_moves() {
        use super::{PegBagChange as C, PegBagStanding as S};
        let gc = game_config::make_english_game_config();
        let kwg = kwg::kbwg_from_bytes_alloc(tiny_kwg_bytes().to_vec(), false).unwrap();
        let arm_tables = anagrams::ArmTables::build(&kwg, &gc, false).unwrap();
        let mut egs =
            EndgameSolver::<kwg::Node24, kwg::Node24>::new(&gc, &kwg, &arm_tables).unwrap();
        let mut board = empty_board();
        put_word(&mut board, 7, 6, &[1, 20]); // AT
        let mut unseen_tally = vec![0u8; gc.alphabet().len() as usize];
        for tile in [1u8, 2, 8] {
            unseen_tally[tile as usize] += 1;
        }
        let position = bag_position(&board, &[1, 1], &unseen_tally, 2, equity::Equity::ZERO);
        let fresh = || {
            super::PegBagProgress::new(position.clone(), super::PegBagChoices::WHOLE)
                .and_then(super::PegBagProgress::without_cuts)
        };
        let rows = |moves: &[super::PegBagMove]| {
            (moves.iter())
                .map(|m| (m.play.clone(), m.win_pct, m.expected_margin, m.exact))
                .collect::<Vec<_>>()
        };
        let one_state = super::Limits {
            time: None,
            states: Some(1),
            memory: None,
            clock: None,
        };
        let mut progress = fresh().unwrap();
        let full = egs.resume_peg(&mut progress, &super::Limits::NONE, 1);
        let full = full.unwrap().unwrap();
        assert!(full.exact && full.moves.len() >= 3);
        let (first, second) = (full.moves[0].play.clone(), full.moves[1].play.clone());
        let mut progress = fresh().unwrap();
        assert_eq!(progress.change(&first, C::Pause), S::Running);
        assert_eq!(progress.change(&first, C::Resume), S::Paused);
        let same = egs.resume_peg(&mut progress, &super::Limits::NONE, 1);
        let same = same.unwrap().unwrap();
        assert!(same.best_move == full.best_move && rows(&same.moves) == rows(&full.moves));
        let mut progress = fresh().unwrap();
        let stopped = egs
            .resume_peg(&mut progress, &one_state, 1)
            .unwrap()
            .unwrap();
        assert!(!stopped.exact);
        assert_eq!(progress.change(&first, C::Pause), S::Running);
        assert_eq!(progress.change(&first, C::Pause), S::Paused);
        let paused = egs.resume_peg(&mut progress, &super::Limits::NONE, 1);
        let paused = paused.unwrap().unwrap();
        assert!(paused.best_move.as_ref() == Some(&second));
        assert_eq!(paused.win_pct, full.moves[1].win_pct);
        assert_eq!(Some(paused.expected_margin), full.moves[1].expected_margin);
        let at = paused.moves.iter().position(|m| m.play == first);
        assert!(at.is_none_or(|at| at + 1 == paused.moves.len()));
        assert_eq!(progress.change(&first, C::Resume), S::Paused);
        assert!(!progress.is_done());
        let resumed = egs.resume_peg(&mut progress, &super::Limits::NONE, 1);
        let resumed = resumed.unwrap().unwrap();
        assert!(resumed.best_move == full.best_move);
        assert!(rows(&resumed.moves) == rows(&full.moves));
        progress.change(&first, C::Pause);
        let listed = progress.answer_moves(0..usize::MAX);
        assert!(rows(&listed[..1]) == rows(&full.moves[1..2]));
        assert!(rows(&listed[listed.len() - 1..]) == rows(&full.moves[..1]));
        progress.change(&first, C::Resume);
        let resumed = egs.resume_peg(&mut progress, &super::Limits::NONE, 1);
        let resumed = resumed.unwrap().unwrap();
        assert!(resumed.best_move == full.best_move);
        assert!(rows(&resumed.moves) == rows(&full.moves));
        assert_eq!(progress.change(&second, C::Remove), S::Running);
        assert_eq!(progress.change(&second, C::Pause), S::Absent);
        let removed = progress.answer_moves(0..usize::MAX);
        assert!(removed.len() + 1 == full.moves.len());
        assert!(removed.iter().all(|m| m.play != second));
        assert_eq!(progress.change(&second, C::Add), S::Absent);
        assert!(!progress.is_done());
        let added = egs.resume_peg(&mut progress, &super::Limits::NONE, 1);
        let added = added.unwrap().unwrap();
        assert!(added.best_move == full.best_move && rows(&added.moves) == rows(&full.moves));
        let mut threads = fresh().unwrap();
        threads.change(&first, C::Pause);
        let two = egs.resume_peg(&mut threads, &super::Limits::NONE, 2);
        assert!(two.unwrap().unwrap().best_move.as_ref() == Some(&second));
        let pass = movegen::Play::Exchange {
            tiles: [1u8; 7][..].into(),
        };
        assert_eq!(progress.change(&pass, C::Add), S::Absent);
        for m in &full.moves {
            progress.change(&m.play, C::Pause);
        }
        let held = egs.resume_peg(&mut progress, &super::Limits::NONE, 1);
        assert!(held.unwrap().unwrap().best_move.is_some());
    }

    #[test]
    fn a_one_tile_search_takes_a_sims_changes_to_its_moves() {
        use super::{PegBagChange as C, PegBagStanding as S};
        let gc = game_config::make_english_game_config();
        let kwg = kwg::kbwg_from_bytes_alloc(tiny_kwg_bytes().to_vec(), false).unwrap();
        let arm_tables = anagrams::ArmTables::build(&kwg, &gc, false).unwrap();
        let mut egs =
            EndgameSolver::<kwg::Node24, kwg::Node24>::new(&gc, &kwg, &arm_tables).unwrap();
        let mut board = empty_board();
        put_word(&mut board, 7, 6, &[1, 20]); // AT
        let mut unseen_tally = vec![0u8; gc.alphabet().len() as usize];
        for tile in [1u8, 2, 8] {
            unseen_tally[tile as usize] += 1;
        }
        let fresh = || {
            super::PegProgress::new(
                0,
                &board,
                &[1, 1],
                &unseen_tally,
                equity::Equity::ZERO,
                [0, 0],
            )
        };
        let rows = |moves: &[super::PegMoveValue]| {
            (moves.iter())
                .map(|m| (m.play.clone(), m.win_pct, m.expected_margin, m.exact))
                .collect::<Vec<_>>()
        };
        let play_of = |play: &super::PegMove| match play {
            super::PegMove::Pass => movegen::Play::Exchange {
                tiles: [][..].into(),
            },
            super::PegMove::Place(play) => play.clone(),
        };
        let one_state = super::Limits {
            time: None,
            states: Some(1),
            memory: None,
            clock: None,
        };
        let mut solve = |progress: &mut super::PegProgress, limits: &super::Limits| {
            egs.resume_peg_one_in_bag(progress, limits)
                .unwrap()
                .unwrap()
        };
        let full = solve(&mut fresh(), &super::Limits::NONE);
        assert!(full.exact && full.moves.len() >= 3);
        let (first, second) = (play_of(&full.moves[0].play), play_of(&full.moves[1].play));
        let mut progress = fresh();
        assert_eq!(progress.change(&first, C::Pause), S::Running);
        assert_eq!(progress.change(&first, C::Resume), S::Paused);
        let same = solve(&mut progress, &super::Limits::NONE);
        assert!(same.best_move == full.best_move && rows(&same.moves) == rows(&full.moves));
        let mut progress = fresh();
        assert!(!solve(&mut progress, &one_state).exact);
        assert_eq!(progress.change(&first, C::Pause), S::Running);
        assert_eq!(progress.change(&first, C::Pause), S::Paused);
        let paused = solve(&mut progress, &super::Limits::NONE);
        assert!(paused.best_move.as_ref().map(play_of) == Some(second.clone()));
        assert_eq!(paused.win_pct, full.moves[1].win_pct);
        assert_eq!(paused.expected_margin, full.moves[1].expected_margin);
        let at = (paused.moves.iter()).position(|m| play_of(&m.play) == first);
        assert!(at.is_none_or(|at| at + 1 == paused.moves.len()));
        assert_eq!(progress.change(&first, C::Resume), S::Paused);
        assert!(!progress.is_done());
        let resumed = solve(&mut progress, &super::Limits::NONE);
        assert!(resumed.best_move == full.best_move);
        assert!(rows(&resumed.moves) == rows(&full.moves));
        progress.change(&first, C::Pause);
        let listed = progress.answer_moves(0..usize::MAX);
        assert!(rows(&listed[..1]) == rows(&full.moves[1..2]));
        assert!(rows(&listed[listed.len() - 1..]) == rows(&full.moves[..1]));
        let k = progress.hypotheses(Some(listed[listed.len() - 1].row));
        let kept = progress.hypotheses(Some(full.moves[0].row));
        assert!(k.collect::<Vec<_>>() == kept.collect::<Vec<_>>());
        progress.change(&first, C::Resume);
        let resumed = solve(&mut progress, &super::Limits::NONE);
        assert!(resumed.best_move == full.best_move);
        assert!(rows(&resumed.moves) == rows(&full.moves));
        assert_eq!(progress.change(&second, C::Remove), S::Running);
        assert_eq!(progress.change(&second, C::Pause), S::Absent);
        let removed = progress.answer_moves(0..usize::MAX);
        assert!(removed.len() + 1 == full.moves.len());
        assert!(removed.iter().all(|m| play_of(&m.play) != second));
        assert_eq!(progress.change(&second, C::Add), S::Absent);
        assert!(!progress.is_done());
        let added = solve(&mut progress, &super::Limits::NONE);
        assert!(added.best_move == full.best_move && rows(&added.moves) == rows(&full.moves));
        let exchange = movegen::Play::Exchange {
            tiles: [1u8][..].into(),
        };
        assert_eq!(progress.change(&exchange, C::Add), S::Absent);
        for m in &full.moves {
            progress.change(&play_of(&m.play), C::Pause);
        }
        assert!(
            solve(&mut progress, &super::Limits::NONE)
                .best_move
                .is_some()
        );
    }
}
