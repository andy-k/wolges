// Copyright (C) 2020-2026 Andy Kurnia.

use super::{bites, endgame, error, game_config, kwg};

// Unconfirmed entries.
// Memory wastage notes:
// - Arc index would be 22 bits max.
// - Could have used u32 instead of this 8-byte struct.
struct Transition {
    tile: u8,
    accepts: bool,
    arc_index: u32, // Refers to states.
}

struct TransitionStack<'a> {
    transitions: &'a mut Vec<Transition>,
    indexes: &'a mut Vec<usize>,
}

impl TransitionStack<'_> {
    #[inline(always)]
    fn push(
        &mut self,
        tile: &u8,
        memory: Option<&endgame::MemoryLimit>,
    ) -> Result<(), endgame::Refused> {
        endgame::room_in(memory, self.transitions, 1)?;
        endgame::room_in(memory, self.indexes, 1)?;
        self.transitions.push(Transition {
            tile: *tile,
            accepts: false,
            arc_index: 0, // Filled up later.
        });
        self.indexes.push(self.transitions.len());
        Ok(())
    }

    #[inline(always)]
    fn pop(&mut self, state_maker: &mut StateMaker<'_>) -> Result<(), endgame::Refused> {
        let start_of_batch = self.indexes.pop().unwrap();
        let new_arc_index = state_maker.make_state(&self.transitions[start_of_batch..])?;
        self.transitions[start_of_batch - 1].arc_index = new_arc_index;
        self.transitions.truncate(start_of_batch);
        Ok(())
    }
}

// Deduplicated entries.
// Memory wastage notes:
// - Each index would be 22 bits max.
// - Could have used u64 or a 7-byte thing instead of this 12-byte struct.
#[derive(Clone, Default, Eq, PartialEq)]
struct State {
    tile: u8,
    accepts: bool,
    arc_index: u32,  // Refers to states.
    next_index: u32, // Refers to states.
}

// what a Finder finds things by: a value and its hash
trait FinderKey: Clone + Default + Eq {
    fn hash_key(&self) -> u64;
}

impl FinderKey for State {
    #[inline(always)]
    fn hash_key(&self) -> u64 {
        ((self.arc_index as u64) | ((self.next_index as u64) << 32))
            ^ (((self.tile as u64) << 1) | self.accepts as u64).rotate_right(8)
    }
}

impl FinderKey for u64 {
    #[inline(always)]
    fn hash_key(&self) -> u64 {
        *self
    }
}

// a key and its index, or an empty slot (index !0)
#[derive(Clone)]
struct FinderSlot<K> {
    key: K,
    index: u32,
}

// the keys found so far, each with its index: an open-addressing table, probed
// linearly, of a power of two slots kept at most three quarters full, each
// slot holding its key so that a probe reads no other memory
#[derive(Default)]
struct Finder<K> {
    slots: Vec<FinderSlot<K>>,
    spare: Vec<FinderSlot<K>>, // what a growth rehashes into
    len: usize,
    shift: u32, // a hash's top bits pick its slot: 64 - log2(slots.len())
}

// the states made so far, by their content
type StatesFinder = Finder<State>;

impl<K: FinderKey> Finder<K> {
    // Fibonacci hashing's multiplier, 2^64 divided by the golden ratio
    // (https://en.wikipedia.org/wiki/Hash_function#Fibonacci_hashing)
    const MULTIPLIER: u64 = 0x9e37_79b9_7f4a_7c15;
    // the most of its slots it fills: a probe reads about 1 / (1 - load)
    // slots, held in a cache line or two
    const LOAD: (usize, usize) = (3, 4);

    // empty, with room for this many keys before it grows (len, until then,
    // is the last fill's), its slots taken within memory, if given
    #[inline(always)]
    fn clear(
        &mut self,
        expected_len: usize,
        memory: Option<&endgame::MemoryLimit>,
    ) -> Result<(), endgame::Refused> {
        let num_slots = (expected_len.max(1) * Self::LOAD.1)
            .div_ceil(Self::LOAD.0)
            .next_power_of_two();
        self.slots.clear();
        endgame::room_in(memory, &mut self.slots, num_slots)?;
        self.slots.resize(
            num_slots,
            FinderSlot {
                key: K::default(),
                index: !0,
            },
        );
        self.len = 0;
        self.shift = u64::BITS - num_slots.trailing_zeros();
        Ok(())
    }

    #[inline(always)]
    fn slot_of(shift: u32, key: &K) -> usize {
        (key.hash_key().wrapping_mul(Self::MULTIPLIER) >> shift) as usize
    }

    // the index of the key like this one, else new_index, now its index; the
    // growth after it is taken within memory, if given (refused, the key is
    // in, and the table is fuller than it keeps itself)
    #[inline(always)]
    fn find_or_insert(
        &mut self,
        key: &K,
        new_index: u32,
        memory: Option<&endgame::MemoryLimit>,
    ) -> Result<u32, endgame::Refused> {
        let mask = self.slots.len() - 1;
        let mut i = Self::slot_of(self.shift, key);
        loop {
            let slot = &self.slots[i];
            if slot.index == !0 {
                break;
            }
            if slot.key == *key {
                return Ok(slot.index);
            }
            i = (i + 1) & mask;
        }
        self.slots[i] = FinderSlot {
            key: key.clone(),
            index: new_index,
        };
        self.len += 1;
        if self.len * Self::LOAD.1 > self.slots.len() * Self::LOAD.0 {
            self.grow(memory)?;
        }
        Ok(new_index)
    }

    // twice the slots, every key rehashed into them, the growth taken within
    // memory, if given (refused, the table is as it was)
    #[cold]
    fn grow(&mut self, memory: Option<&endgame::MemoryLimit>) -> Result<(), endgame::Refused> {
        let num_slots = self.slots.len() * 2;
        self.spare.clear();
        endgame::room_in(memory, &mut self.spare, num_slots)?;
        std::mem::swap(&mut self.slots, &mut self.spare);
        self.slots.resize(
            num_slots,
            FinderSlot {
                key: K::default(),
                index: !0,
            },
        );
        self.shift -= 1;
        let mask = num_slots - 1;
        for slot in self.spare.drain(..) {
            if slot.index == !0 {
                continue;
            }
            let mut i = Self::slot_of(self.shift, &slot.key);
            while self.slots[i].index != !0 {
                i = (i + 1) & mask;
            }
            self.slots[i] = slot;
        }
        Ok(())
    }
}

// for each i > 0, states[i].arc_index < i and states[i].next_index < i.
// this ensures states is already a topologically sorted DAG.
struct StateMaker<'a> {
    states: &'a mut Vec<State>,
    states_finder: &'a mut StatesFinder,
    memory: Option<&'a endgame::MemoryLimit>, // every growth's limit
}

impl StateMaker<'_> {
    #[inline(always)]
    fn make_state(&mut self, node_transitions: &[Transition]) -> Result<u32, endgame::Refused> {
        let mut ret = 0;
        for node_transition in node_transitions.iter().rev() {
            let state = State {
                tile: node_transition.tile,
                accepts: node_transition.accepts,
                arc_index: node_transition.arc_index,
                next_index: ret,
            };
            let new_ret = self.states.len() as u32;
            endgame::room_in(self.memory, self.states, 1)?;
            ret = self
                .states_finder
                .find_or_insert(&state, new_ret, self.memory)?;
            if ret == new_ret {
                self.states.push(state);
            }
        }
        Ok(ret)
    }

    // transition_stack is the builder's, emptied here
    #[inline(always)]
    fn make_dawg<W: MachineWords + ?Sized>(
        &mut self,
        transition_stack: &mut TransitionStack<'_>,
        sorted_machine_words: &W,
        dawg_start_state: u32,
        is_gaddag_phase: bool,
    ) -> Result<u32, endgame::Refused> {
        transition_stack.transitions.clear();
        transition_stack.indexes.clear();
        for machine_word_index in 0..sorted_machine_words.num_words() {
            let this_word = sorted_machine_words.word(machine_word_index);
            let this_word_len = this_word.len();
            let mut prefix_len = 0;
            if machine_word_index > 0 {
                let prev_word = sorted_machine_words.word(machine_word_index - 1);
                let prev_word_len = transition_stack.indexes.len(); // this can be one less than prev_word.len() for gaddag
                let min_word_len = this_word_len.min(prev_word_len);
                while prefix_len < min_word_len && prev_word[prefix_len] == this_word[prefix_len] {
                    prefix_len += 1;
                }
                for _ in prefix_len..prev_word_len {
                    transition_stack.pop(self)?;
                }
            }
            for tile in &this_word[prefix_len..this_word_len] {
                transition_stack.push(tile, self.memory)?;
            }
            let transitions_len = transition_stack.transitions.len();
            if is_gaddag_phase && this_word[this_word_len - 1] == 0 {
                transition_stack.indexes.pop().unwrap();
                transition_stack.transitions[transitions_len - 1].arc_index = sorted_machine_words
                    .turnaround_target(machine_word_index, self.states, dawg_start_state);
            } else {
                transition_stack.transitions[transitions_len - 1].accepts = true;
            }
        }
        for _ in 0..transition_stack.indexes.len() {
            transition_stack.pop(self)?;
        }
        self.make_state(&transition_stack.transitions[..])
    }
}

// the dawg's node a word leads to, its tiles followed from dawg_start_state
#[inline(always)]
fn dawg_target(states: &[State], dawg_start_state: u32, word: impl Iterator<Item = u8>) -> u32 {
    let mut p = dawg_start_state;
    for sought_tile in word {
        loop {
            if states[p as usize].tile == sought_tile {
                p = states[p as usize].arc_index;
                break;
            }
            p = states[p as usize].next_index;
        }
    }
    p
}

// words in ascending order, as make_dawg reads them
trait MachineWords {
    fn num_words(&self) -> usize;
    fn word(&self, index: usize) -> &[u8];
    // for a word that ends at a turnaround (tile 0), the dawg's node of the
    // tiles before it, reversed: gaddag["AC@"] points to dawg["CA"]
    fn turnaround_target(&self, index: usize, states: &[State], dawg_start_state: u32) -> u32;
}

impl MachineWords for [bites::Bites] {
    #[inline(always)]
    fn num_words(&self) -> usize {
        self.len()
    }

    #[inline(always)]
    fn word(&self, index: usize) -> &[u8] {
        &self[index]
    }

    #[inline(always)]
    fn turnaround_target(&self, index: usize, states: &[State], dawg_start_state: u32) -> u32 {
        let word = &self[index];
        dawg_target(
            states,
            dawg_start_state,
            word[..word.len() - 1].iter().rev().copied(),
        )
    }
}

// a packed word: where its tiles start in the words' tiles, how many there
// are, and its first tiles as a number that orders as they do
#[derive(Clone, Copy)]
struct PackedWord {
    key: u64, // the first 8 tiles, big-endian, 0 past the last
    start: u32,
    len: u32,
    turnaround_target: u32, // for one that ends at a turnaround
}

// words, as the drowwords or the alphagrams of a word list, their tiles one
// after another, so that none is a value of its own
#[derive(Default)]
struct PackedWords {
    tiles: Vec<u8>,
    words: Vec<PackedWord>,
    path: Vec<u32>, // the dawg's nodes along path_tiles: the start, then each one's
    path_tiles: Vec<u8>,
}

impl MachineWords for PackedWords {
    #[inline(always)]
    fn num_words(&self) -> usize {
        self.words.len()
    }

    #[inline(always)]
    fn word(&self, index: usize) -> &[u8] {
        let word = self.words[index];
        &self.tiles[word.start as usize..(word.start + word.len) as usize]
    }

    #[inline(always)]
    fn turnaround_target(&self, index: usize, _: &[State], _: u32) -> u32 {
        self.words[index].turnaround_target
    }
}

impl PackedWords {
    // the word of the tiles just pushed, from start on, its room taken within
    // memory, if given
    #[inline(always)]
    fn end_word(
        &mut self,
        start: usize,
        turnaround_target: u32,
        memory: Option<&endgame::MemoryLimit>,
    ) -> Result<(), endgame::Refused> {
        endgame::room_in(memory, &mut self.words, 1)?;
        let tiles = &self.tiles[start..];
        let mut key = [0u8; 8];
        let num_key_tiles = tiles.len().min(key.len());
        key[..num_key_tiles].copy_from_slice(&tiles[..num_key_tiles]);
        self.words.push(PackedWord {
            key: u64::from_be_bytes(key),
            start: start as u32,
            len: tiles.len() as u32,
            turnaround_target,
        });
        Ok(())
    }

    // in ascending order of their tiles: by key, and by the tiles past it when
    // two keys tie (as when one word is the other and a 0 past it)
    #[inline(always)]
    fn sort(&mut self) {
        let tiles = &self.tiles[..];
        self.words.sort_unstable_by(|a, b| {
            a.key.cmp(&b.key).then_with(|| {
                tiles[a.start as usize..(a.start + a.len) as usize]
                    .cmp(&tiles[b.start as usize..(b.start + b.len) as usize])
            })
        });
    }

    // each word once, once they are sorted
    #[inline(always)]
    fn dedup(&mut self) {
        let tiles = &self.tiles[..];
        self.words.dedup_by(|a, b| {
            a.key == b.key
                && tiles[a.start as usize..(a.start + a.len) as usize]
                    == tiles[b.start as usize..(b.start + b.len) as usize]
        });
    }
}

// machine_drowwords, the builder's, emptied here; each turnaround's target in
// the dawg made of machine_words (states, from dawg_start_state) is found from
// the nodes of the prefix this word shares with the last one walked. Each
// growth is taken within memory, if given
#[inline]
fn gen_machine_drowwords<W: MachineWords + ?Sized>(
    machine_words: &W,
    states: &[State],
    dawg_start_state: u32,
    machine_drowwords: &mut PackedWords,
    memory: Option<&endgame::MemoryLimit>,
) -> Result<(), endgame::Refused> {
    machine_drowwords.tiles.clear();
    machine_drowwords.words.clear();
    machine_drowwords.path.clear();
    endgame::room_in(memory, &mut machine_drowwords.path, 1)?;
    machine_drowwords.path.push(dawg_start_state);
    machine_drowwords.path_tiles.clear();
    for machine_word_index in 0..machine_words.num_words() {
        let this_word = machine_words.word(machine_word_index);
        let this_word_len = this_word.len();
        let mut prefix_len = 0;
        if machine_word_index > 0 {
            let prev_word = machine_words.word(machine_word_index - 1);
            let prev_word_len = prev_word.len();
            // - 1 because CAR -> CARE means we still need to emit RAC@.
            let max_prefix_len = this_word_len.min(prev_word_len - 1);
            while prefix_len < max_prefix_len && prev_word[prefix_len] == this_word[prefix_len] {
                prefix_len += 1;
            }
        }
        // CARE = ERAC, RAC@, AC@, C@
        let start = machine_drowwords.tiles.len();
        endgame::room_in(memory, &mut machine_drowwords.tiles, this_word_len)?;
        machine_drowwords
            .tiles
            .extend(this_word.iter().rev().copied());
        machine_drowwords.end_word(start, 0, memory)?;
        let num_prefixes = this_word_len - prefix_len;
        if num_prefixes >= 2 {
            // the walk keeps the prefix it shares with this word, and goes on
            // to the prefix one tile short of it
            let PackedWords {
                path, path_tiles, ..
            } = machine_drowwords;
            let mut num_kept = 0;
            let max_kept = path_tiles.len().min(this_word_len - 1);
            while num_kept < max_kept && path_tiles[num_kept] == this_word[num_kept] {
                num_kept += 1;
            }
            path.truncate(num_kept + 1);
            path_tiles.truncate(num_kept);
            endgame::room_in(memory, path, this_word_len - 1 - num_kept)?;
            endgame::room_in(memory, path_tiles, this_word_len - 1 - num_kept)?;
            for &tile in &this_word[num_kept..this_word_len - 1] {
                let p = *path.last().unwrap();
                path.push(dawg_target(states, p, std::iter::once(tile)));
                path_tiles.push(tile);
            }
        }
        for drow_prefix_len in 1..num_prefixes {
            let start = machine_drowwords.tiles.len();
            let target_len = this_word_len - drow_prefix_len;
            endgame::room_in(memory, &mut machine_drowwords.tiles, target_len + 1)?;
            machine_drowwords
                .tiles
                .extend(this_word[..target_len].iter().rev().copied());
            machine_drowwords.tiles.push(0); // the '@'
            let turnaround_target = machine_drowwords.path[target_len];
            machine_drowwords.end_word(start, turnaround_target, memory)?;
        }
    }
    machine_drowwords.sort();
    Ok(())
}

// AlphaDawg is DawgOnly on make_alphagrams(machine_words).
#[inline]
pub fn make_alphagrams(machine_words: &[bites::Bites]) -> Box<[bites::Bites]> {
    let mut machine_dorws = PackedWords::default();
    // with no limit, nothing is refused
    let _ = fill_packed_alphagrams(machine_words, &mut machine_dorws, None);
    (0..machine_dorws.num_words())
        .map(|index| machine_dorws.word(index).into())
        .collect()
}

// make_alphagrams packed into machine_dorws, the builder's, emptied here, each
// growth taken within memory, if given
#[inline]
fn fill_packed_alphagrams(
    machine_words: &[bites::Bites],
    machine_dorws: &mut PackedWords,
    memory: Option<&endgame::MemoryLimit>,
) -> Result<(), endgame::Refused> {
    machine_dorws.tiles.clear();
    machine_dorws.words.clear();
    for this_word in machine_words {
        let start = machine_dorws.tiles.len();
        endgame::room_in(memory, &mut machine_dorws.tiles, this_word.len())?;
        machine_dorws.tiles.extend_from_slice(this_word);
        machine_dorws.tiles[start..].sort_unstable();
        machine_dorws.end_word(start, 0, memory)?;
    }
    machine_dorws.sort();
    machine_dorws.dedup();
    Ok(())
}

// build formats

pub enum BuildContent {
    DawgOnly,
    Gaddawg,
}

pub enum BuildLayout {
    Legacy,       // tiny, slow to movegen, leaf-first order. used to be the default.
    Magpie, // big, fast to movegen, BFS order, no tail dedup, easy to read. https://github.com/jvc56/MAGPIE/
    MagpieMerged, // tiny, slow to movegen, BFS order.
    Experimental, // small, fast to movegen, frequent-first order, may put dawg behind gaddag.
    Wolges, // small, faster to movegen, dawg-first then frequent-first. recommended default.
}

pub enum BuildOrder {
    Sorted,
    Reordered,
}

// zero-cost type-safety
struct IsEnd(bool);
struct Accepts(bool);

// Each block has 16 entries (hardcoded).
// 16 entries of u32 make 64 bytes, which is a common cache line size.
// 0 <= block_len[i] <= 16, from (i << 4) the first block_len[i] are occupied.
// If block_len[i] < 16, blocks_with_len[block_len[i]] stack includes i.
struct StateDefraggerExperimentalParams<'a> {
    block_len: &'a mut Vec<u8>,
    blocks_with_len: &'a mut [Vec<u32>; 16],
}

impl StateDefraggerExperimentalParams<'_> {
    // every block gone, each stack kept for the next build
    #[inline(always)]
    fn clear(&mut self) {
        self.block_len.clear();
        self.blocks_with_len.iter_mut().for_each(Vec::clear);
    }
}

struct StatesDefragger<'a> {
    states: &'a [State],
    head_indexes: &'a [u32],
    to_end_lens: &'a [u32], // using u8 costs runtime.
    destination: &'a mut Vec<u32>,
    num_written: u32,
    stack: &'a mut Vec<(u32, u32, u32)>, // walk_lists': head, next arc, entered
    memory: Option<&'a endgame::MemoryLimit>, // the walk's growths' limit
    refused: bool,                       // one refused: it places no more
}

impl StatesDefragger<'_> {
    // whether list has room for more, its growth taken within memory, if
    // given; once one is refused, none is
    #[inline(always)]
    fn room<T>(&mut self, list: &mut Vec<T>, more: usize) -> bool {
        self.refused = self.refused || endgame::room_in(self.memory, list, more).is_err();
        !self.refused
    }
}

impl StatesDefragger<'_> {
    // walks the lists reached from p's list, each from its head: enter on first
    // reaching a list, leave once every list its arcs reach is left (leave gets
    // what enter returned), the order of a recursion into each arc. A list that
    // is another's tail is reached at that one's head, whose other arcs lead
    // anywhere, so that recursion's depth grows with the word list, past a
    // Windows main thread's 1 MB: the walk keeps its own stack.
    #[inline(always)]
    fn walk_lists(
        &mut self,
        mut p: u32,
        mut enter: impl FnMut(&mut Self, u32) -> u32,
        mut leave: impl FnMut(&mut Self, u32, u32),
    ) {
        // a list's head, its next arc to follow, what enter returned
        self.stack.clear();
        loop {
            p = self.head_indexes[p as usize];
            if self.destination[p as usize] == 0 {
                // temp value to break self-cycles.
                self.destination[p as usize] = !0;
                let entered = enter(self, p);
                if self.refused || endgame::room_in(self.memory, self.stack, 1).is_err() {
                    self.refused = true;
                    return;
                }
                self.stack.push((p, p, entered));
            }
            loop {
                let Some(top) = self.stack.last_mut() else {
                    return;
                };
                let q = top.1;
                if q == 0 {
                    let (head, _, entered) = *top;
                    self.stack.pop();
                    leave(self, head, entered);
                    continue;
                }
                top.1 = self.states[q as usize].next_index;
                p = self.states[q as usize].arc_index;
                if p != 0 {
                    break;
                }
            }
        }
    }

    // the list at write_p goes from initial_num_written on, up to its first
    // node already placed
    #[inline(always)]
    fn place(&mut self, mut write_p: u32, initial_num_written: u32) {
        let num = self.to_end_lens[write_p as usize];
        self.destination[write_p as usize] = 0;
        for ofs in 0..num {
            // prefer earlier index, so dawg part does not point to gaddag part
            if self.destination[write_p as usize] != 0 {
                break;
            }
            self.destination[write_p as usize] = initial_num_written + ofs;
            write_p = self.states[write_p as usize].next_index;
        }
    }

    #[inline]
    fn defrag_legacy(&mut self, p: u32) {
        self.walk_lists(
            p,
            |_, _| 0,
            |this, p, _| {
                let num = this.to_end_lens[p as usize];
                let initial_num_written = this.num_written;
                this.place(p, initial_num_written);
                // Always += num even if some nodes are necessarily duplicated due to sharing by different prev_nodes.
                this.num_written += num;
            },
        );
    }

    fn defrag_magpie(&mut self, mut p: u32) {
        if self.destination[p as usize] != 0 {
            return;
        }
        self.destination[p as usize] = self.num_written;
        // non-legacy mode reserves the space first.
        let num = self.to_end_lens[p as usize];
        self.num_written += num;
        loop {
            let a = self.states[p as usize].arc_index;
            if a != 0 {
                self.defrag_magpie(a);
            }
            p = self.states[p as usize].next_index;
            if p == 0 {
                break;
            }
        }
    }

    #[inline]
    fn defrag_magpie_merged(&mut self, p: u32) {
        self.walk_lists(
            p,
            |this, p| {
                let initial_num_written = this.num_written;
                // non-legacy mode reserves the space first.
                this.num_written += this.to_end_lens[p as usize];
                initial_num_written
            },
            // non-legacy mode already reserves the space.
            Self::place,
        );
    }

    #[inline]
    fn defrag_cache_friendly(&mut self, params: &mut StateDefraggerExperimentalParams<'_>, p: u32) {
        self.walk_lists(
            p,
            |this, p| {
                // non-legacy mode reserves the space first.
                let num = this.to_end_lens[p as usize];
                // room for every push below: a block a 16 of num, one more to
                // align, and one for the rest, and a stack entry each
                let most = num as usize / 16 + 2;
                if !this.room(params.block_len, most)
                    || !(params.blocks_with_len.iter_mut()).all(|stack| this.room(stack, 2))
                {
                    return 0;
                }
                // choose a cache-friendly page to place these.
                let mut num_blocks = params.block_len.len() as u32;
                let initial_num_written;
                if num > 16 {
                    // always even-align for 128 byte cache line machines.
                    if num_blocks & 1 == 1 {
                        params.blocks_with_len[0].push(num_blocks);
                        params.block_len.push(0);
                        num_blocks += 1;
                    }
                    initial_num_written = num_blocks << 4;
                    let mut num = num; // shadow the variable
                    while num > 16 {
                        params.block_len.push(16);
                        num -= 16;
                    }
                    // this can be between 1 to 16.
                    if num < 16 {
                        params.blocks_with_len[num as usize].push(params.block_len.len() as u32);
                    }
                    params.block_len.push(num as u8);
                } else {
                    // 1 <= num <= 16
                    let mut required_gap = 16 - num; // 0 <= required_gap <= 15
                    loop {
                        // if found, use it
                        if let Some(place) = params.blocks_with_len[required_gap as usize].pop() {
                            // use | instead of + because it cannot overflow
                            initial_num_written = (place << 4) | required_gap;
                            // repurpose this variable.
                            required_gap += num; // 1 <= required_gap <= 16
                            if required_gap < 16 {
                                params.blocks_with_len[required_gap as usize].push(place);
                            }
                            params.block_len[place as usize] = required_gap as u8;
                            break;
                        }
                        // if 0, add new row.
                        if required_gap == 0 {
                            initial_num_written = num_blocks << 4;
                            if num < 16 {
                                params.blocks_with_len[num as usize].push(num_blocks);
                            }
                            params.block_len.push(num as u8);
                            break;
                        }
                        // if not, -1 then try again
                        required_gap -= 1;
                    }
                }
                initial_num_written
            },
            // non-legacy mode already reserves the space.
            Self::place,
        );
    }

    // into order, the builder's, emptied here: every state but the sink, those
    // in_front says first, then those with more ways, then longer to the end
    // of their list, then in index order. Each is a key of these as numbers
    // that ascend in that order, the index its low bits, so that the sort
    // compares keys and reads nothing else
    #[inline(always)]
    fn order_by_ways(
        &mut self,
        num_ways: &[u32],
        in_front: impl Fn(usize) -> bool,
        order: &mut Vec<u128>,
    ) {
        order.clear();
        if !self.room(order, self.states.len()) {
            return;
        }
        order.extend((1..self.states.len()).map(|p| {
            let field = u32::BITS;
            ((!in_front(p) as u128) << (field * 3))
                | ((!num_ways[p] as u128) << (field * 2))
                | ((!self.to_end_lens[p] as u128) << field)
                | p as u128
        }));
        order.sort_unstable();
    }

    // order and params are the builder's, emptied here
    #[inline]
    fn build_experimental(
        &mut self,
        num_ways: &[u32],
        top_indexes: &[u32],
        order: &mut Vec<u128>,
        params: &mut StateDefraggerExperimentalParams<'_>,
    ) {
        self.order_by_ways(num_ways, |_| false, order);

        params.clear();
        // num_written is either 1 or 2, both are < 16.
        params.block_len.push(self.num_written as u8);
        params.blocks_with_len[self.num_written as usize].push(0u32);
        for &key in order.iter() {
            self.defrag_cache_friendly(params, top_indexes[key as u32 as usize]);
        }
        self.num_written =
            ((params.block_len.len() as u32 - 1) << 4) + *params.block_len.last().unwrap() as u32;
    }

    // order, used_in_dawg, and params are the builder's, emptied here
    #[inline]
    fn build_wolges(
        &mut self,
        num_ways: &[u32],
        build_content: &BuildContent,
        dawg_start_state: u32,
        order: &mut Vec<u128>,
        used_in_dawg: &mut Vec<bool>,
        params: &mut StateDefraggerExperimentalParams<'_>,
    ) {
        match build_content {
            BuildContent::DawgOnly => {
                // All nodes are dawg nodes.
                self.order_by_ways(num_ways, |_| false, order);
            }
            BuildContent::Gaddawg => {
                // Check which nodes are used in dawg.
                used_in_dawg.clear();
                if !self.room(used_in_dawg, self.states.len()) {
                    return;
                }
                used_in_dawg.resize(self.states.len(), false);
                used_in_dawg
                    .iter_mut()
                    .take(dawg_start_state as usize + 1)
                    .skip(1)
                    .for_each(|m| *m = true);
                for p in dawg_start_state as usize + 1..self.states.len() {
                    if used_in_dawg[self.states[p].next_index as usize] {
                        used_in_dawg[p] = true
                    }
                }
                self.order_by_ways(num_ways, |p| used_in_dawg[p], order);
            }
        }

        params.clear();
        if !self.room(params.block_len, 1)
            || !self.room(&mut params.blocks_with_len[self.num_written as usize], 1)
        {
            return;
        }
        // num_written is either 1 or 2, both are < 16.
        params.block_len.push(self.num_written as u8);
        params.blocks_with_len[self.num_written as usize].push(0u32);
        for &key in order.iter() {
            self.defrag_cache_friendly(params, key as u32);
            if self.refused {
                return;
            }
        }
        self.num_written =
            ((params.block_len.len() as u32 - 1) << 4) + *params.block_len.last().unwrap() as u32;
    }

    // encoding: little endian of
    // (Node22)
    // bits 0-21 = pointer & 0x3fffff
    // bit 22 = end
    // bit 23 = is_terminal
    // bits 24-31 = char
    // (Node24)
    // bits 0-5 = char & 0x3f
    // bit 6 = end
    // bit 7 = is_terminal
    // bits 8-31 = pointer & 0xffffff
    #[inline(always)]
    fn write_node<const VARIANT: u8>(
        &self,
        out: &mut [u8],
        arc_index: u32,
        is_end: IsEnd,
        accepts: Accepts,
        tile: u8,
    ) {
        let defragged_arc_index = self.destination[arc_index as usize];
        match VARIANT {
            1 => {
                out[0] = defragged_arc_index as u8;
                out[1] = (defragged_arc_index >> 8) as u8;
                out[2] = (((defragged_arc_index >> 16) & 0x3f) as u8)
                    | ((is_end.0 as u8) << 6)
                    | ((accepts.0 as u8) << 7);
                out[3] = tile;
            }
            2 => {
                out[0] = (tile & 0x3f) | ((is_end.0 as u8) << 6) | ((accepts.0 as u8) << 7);
                out[1] = defragged_arc_index as u8;
                out[2] = (defragged_arc_index >> 8) as u8;
                out[3] = (defragged_arc_index >> 16) as u8;
            }
            _ => unimplemented!(),
        }
    }

    // into ret, the builder's, emptied here
    #[inline]
    fn to_vec<const VARIANT: u8>(
        &self,
        build_content: BuildContent,
        dawg_start_state: u32,
        gaddag_start_state: u32,
        ret: &mut Vec<u8>,
    ) {
        ret.clear();
        ret.resize((self.num_written as usize) * 4, 0);
        self.write_node::<VARIANT>(
            &mut ret[0..],
            dawg_start_state,
            IsEnd(true),
            Accepts(false),
            0,
        );
        match build_content {
            BuildContent::DawgOnly => {}
            BuildContent::Gaddawg => {
                self.write_node::<VARIANT>(
                    &mut ret[4..],
                    gaddag_start_state,
                    IsEnd(true),
                    Accepts(false),
                    0,
                );
            }
        }
        for mut p in 1..self.states.len() {
            let mut dp = self.destination[p] as usize;
            if dp == 0 {
                continue;
            }
            dp *= 4;
            loop {
                let np = self.states[p].next_index;
                self.write_node::<VARIANT>(
                    &mut ret[dp..],
                    self.states[p].arc_index,
                    IsEnd(np == 0),
                    Accepts(self.states[p].accepts),
                    self.states[p].tile,
                );
                if np == 0 {
                    break;
                }
                p = np as usize;
                dp += 4;
            }
        }
    }
}

// each gen_* below fills its output, the builder's, emptied first
#[inline]
fn gen_head_indexes(
    states: &[State],
    head_indexes: &mut Vec<u32>,
    memory: Option<&endgame::MemoryLimit>,
) -> Result<(), endgame::Refused> {
    let states_len = states.len();
    head_indexes.clear();
    endgame::room_in(memory, head_indexes, states_len)?;
    head_indexes.extend(0..states_len as u32);

    // point to immediate prev.
    for p in (1..states_len).rev() {
        head_indexes[states[p].next_index as usize] = p as u32;
    }
    // head_indexes[0] is garbage, does not matter.

    // adjust to point to prev heads instead.
    for p in (1..states_len).rev() {
        head_indexes[p] = head_indexes[head_indexes[p] as usize];
    }
    Ok(())
}

#[inline]
fn gen_to_end_lens(
    states: &[State],
    to_end_lens: &mut Vec<u32>,
    memory: Option<&endgame::MemoryLimit>,
) -> Result<(), endgame::Refused> {
    let states_len = states.len();
    to_end_lens.clear();
    endgame::room_in(memory, to_end_lens, states_len)?;
    to_end_lens.resize(states_len, 1);

    for p in 1..states_len {
        let next = states[p].next_index;
        if next != 0 {
            to_end_lens[p] += to_end_lens[next as usize];
        }
    }
    Ok(())
}

#[inline]
fn gen_num_ways(
    states: &[State],
    build_content: &BuildContent,
    dawg_start_state: u32,
    gaddag_start_state: u32,
    num_ways: &mut Vec<u32>,
    memory: Option<&endgame::MemoryLimit>,
) -> Result<(), endgame::Refused> {
    let states_len = states.len();
    num_ways.clear();
    endgame::room_in(memory, num_ways, states_len)?;
    num_ways.resize(states_len, 0);

    num_ways[dawg_start_state as usize] = 1;
    match build_content {
        BuildContent::DawgOnly => {}
        BuildContent::Gaddawg => {
            num_ways[gaddag_start_state as usize] = 1;
        }
    }
    for p in (1..states_len).rev() {
        let this_num_ways = num_ways[p];
        let state = &states[p];
        for p_dest in [state.next_index, state.arc_index] {
            let v = &mut num_ways[p_dest as usize];
            *v = v.saturating_add(this_num_ways);
        }
    }
    Ok(())
}

#[inline]
fn gen_top_indexes(states: &[State], head_indexes: &[u32], top_indexes: &mut Vec<u32>) {
    let states_len = states.len();
    top_indexes.clear();
    top_indexes.resize(states_len, 0);

    for (p, p_dest) in states
        .iter()
        .map(|x| x.arc_index as usize)
        .enumerate()
        .take(states_len)
        .skip(1)
    {
        top_indexes[p_dest] = p as u32 | -((top_indexes[p_dest] != 0) as i32) as u32;
    }
    // [p] = 0 (no parent), parent_index, or !0 if > 1 parents.
    // if not unique, set [p] = p.
    for (p, top_index) in top_indexes.iter_mut().enumerate().take(states_len) {
        if *top_index == 0 || *top_index == !0 {
            *top_index = p as u32;
        }
    }
    // adjust to point to prev tops's heads instead.
    for p in (1..states_len).rev() {
        top_indexes[p] = head_indexes[top_indexes[top_indexes[p] as usize] as usize];
    }
}

#[inline]
fn is_sublist(states: &[State], mut a: u32, mut b: u32) -> bool {
    while a != 0 {
        let a_state = &states[a as usize];
        loop {
            if b == 0 {
                return false;
            }
            let b_state = &states[b as usize];
            if b_state.tile > a_state.tile {
                return false;
            }
            b = b_state.next_index;
            if b_state.tile == a_state.tile {
                if b_state.accepts != a_state.accepts || b_state.arc_index != a_state.arc_index {
                    return false;
                }
                break;
            }
        }
        a = a_state.next_index;
    }
    true
}

// what a growth under no limit gives: never refused
#[inline(always)]
fn unlimited<T>(grown: Result<T, endgame::Refused>) -> T {
    match grown {
        Ok(value) => value,
        Err(endgame::Refused) => unreachable!("no limit refuses a growth"),
    }
}

// what reorder_states fills, kept by the builder from one build to the next
#[derive(Default)]
struct ReorderBuffers {
    new_states: Vec<State>,
    to_end_lens: Vec<u32>,
    is_head: Vec<bool>,
    has_dawg: Vec<bool>,
    in_dawg: Vec<bool>,
    movable: Vec<u32>,
    entry_indexes: Finder<u64>,
    entry_counts: Vec<u32>,
    element_entries: Vec<u32>, // each movable list's entries, in list order
    postings_starts: Vec<u32>,
    postings: Vec<u32>,
    postings_fill: Vec<u32>,
    rarest_entries: Vec<u32>,
    by_len: Vec<u64>, // each a movable list's key: see reorder_states
    tails: Vec<u32>,
    tried: Vec<u32>,
    path: Vec<(u32, u32, u32)>,
    order_starts: Vec<u32>,
    order_cells: Vec<u32>,
    new_heads: Vec<u32>,
}

// the states reordered into buffers.new_states, with states_finder and
// transitions, the builder's, emptied here; gives the new start states
#[inline]
fn reorder_states(
    states: &[State],
    build_content: &BuildContent,
    dawg_start_state: u32,
    gaddag_start_state: u32,
    buffers: &mut ReorderBuffers,
    states_finder: &mut StatesFinder,
    transitions: &mut Vec<Transition>,
) -> (u32, u32) {
    #[inline(always)]
    fn entry_key(state: &State) -> u64 {
        ((state.arc_index as u64) << 9) | ((state.accepts as u64) << 8) | state.tile as u64
    }

    let ReorderBuffers {
        new_states,
        to_end_lens,
        is_head,
        has_dawg,
        in_dawg,
        movable,
        entry_indexes,
        entry_counts,
        element_entries,
        postings_starts,
        postings,
        postings_fill,
        rarest_entries,
        by_len,
        tails,
        tried,
        path,
        order_starts,
        order_cells,
        new_heads,
    } = buffers;

    let states_len = states.len();
    unlimited(gen_to_end_lens(states, to_end_lens, None));

    is_head.clear();
    is_head.resize(states_len, false);
    for state in states.iter().skip(1) {
        is_head[state.arc_index as usize] = true;
    }
    for root in [dawg_start_state, gaddag_start_state] {
        is_head[root as usize] = true;
    }
    is_head[0] = false;

    has_dawg.clear();
    has_dawg.resize(states_len, false);
    if let BuildContent::Gaddawg = build_content {
        in_dawg.clear();
        in_dawg.resize(states_len, false);
        in_dawg[dawg_start_state as usize] = true;
        for p in (1..states_len).rev() {
            if in_dawg[p] {
                in_dawg[states[p].arc_index as usize] = true;
                in_dawg[states[p].next_index as usize] = true;
            }
        }
        for p in 1..states_len {
            has_dawg[p] = in_dawg[p] || has_dawg[states[p].next_index as usize];
        }
    }

    movable.clear();
    for p in 1..states_len {
        if is_head[p] && !has_dawg[p] {
            movable.push(p as u32);
        }
    }

    // room for an entry a list, or for as many as the last reorder found
    unlimited(entry_indexes.clear(movable.len().max(entry_indexes.len), None));
    entry_counts.clear();
    element_entries.clear();
    for &head in movable.iter() {
        let mut p = head;
        while p != 0 {
            let num_entries = entry_counts.len() as u32;
            let entry_index = unlimited(entry_indexes.find_or_insert(
                &entry_key(&states[p as usize]),
                num_entries,
                None,
            ));
            if entry_index == num_entries {
                entry_counts.push(0);
            }
            entry_counts[entry_index as usize] += 1;
            element_entries.push(entry_index);
            p = states[p as usize].next_index;
        }
    }
    postings_starts.clear();
    let mut num_postings = 0u32;
    for &entry_count in entry_counts.iter() {
        postings_starts.push(num_postings);
        num_postings += entry_count;
    }
    postings_starts.push(num_postings);
    postings.clear();
    postings.resize(num_postings as usize, 0);
    postings_fill.clone_from(postings_starts);
    rarest_entries.clear();
    let mut element_entries = element_entries.iter();
    for (i, &head) in movable.iter().enumerate() {
        let mut rarest_entry = 0;
        let mut rarest_count = !0;
        let mut p = head;
        while p != 0 {
            let entry_index = *element_entries.next().unwrap();
            postings[postings_fill[entry_index as usize] as usize] = i as u32;
            postings_fill[entry_index as usize] += 1;
            let entry_count = entry_counts[entry_index as usize];
            if entry_count < rarest_count {
                rarest_count = entry_count;
                rarest_entry = entry_index;
            }
            p = states[p as usize].next_index;
        }
        rarest_entries.push(rarest_entry);
    }

    // the movable lists, longest first, then by head: as movable ascends by
    // head, each is a key of its length's complement and its index, which the
    // sort compares alone
    by_len.clear();
    by_len.extend(
        movable
            .iter()
            .enumerate()
            .map(|(i, &head)| ((!to_end_lens[head as usize] as u64) << u32::BITS) | i as u64),
    );
    by_len.sort_unstable();

    struct Absorber<'a> {
        states: &'a [State],
        movable: &'a [u32],
        to_end_lens: &'a [u32],
        postings: &'a [u32],
        postings_starts: &'a [u32],
        rarest_entries: &'a [u32],
        tried: &'a mut [u32], // stamped with the list being placed
        stamp: u32,
        path: &'a mut Vec<(u32, u32, u32)>, // list, next posting, host waited on
    }
    impl Absorber<'_> {
        // finds list i a host whose tail is free, or one whose tail can move on
        // to a host of its own the same way, and so on. That path is kept on a
        // stack here, not the call stack, since it can be as long as there are
        // lists; it is tried in the order a recursion would try it.
        fn absorb(&mut self, tails: &mut [u32], i: u32) {
            let (states, movable, to_end_lens) = (self.states, self.movable, self.to_end_lens);
            let (postings, postings_starts) = (self.postings, self.postings_starts);
            let rarest_entries = self.rarest_entries;
            self.path.clear();
            self.path
                .push((i, postings_starts[rarest_entries[i as usize] as usize], !0));
            while let Some((i, k, host_waited_on)) = self.path.last_mut() {
                let head = movable[*i as usize];
                let len = to_end_lens[head as usize];
                let postings_end = postings_starts[rarest_entries[*i as usize] as usize + 1];
                *host_waited_on = !0;
                while *k < postings_end {
                    let j = postings[*k as usize];
                    *k += 1;
                    if j == *i || self.tried[j as usize] == self.stamp {
                        continue;
                    }
                    let host = movable[j as usize];
                    if to_end_lens[host as usize] <= len || !is_sublist(states, head, host) {
                        continue;
                    }
                    self.tried[j as usize] = self.stamp;
                    *host_waited_on = j;
                    break;
                }
                let j = *host_waited_on;
                if j == !0 {
                    self.path.pop();
                    continue;
                }
                let tail = tails[j as usize];
                if tail == !0 {
                    for &(i, _, j) in self.path.iter() {
                        tails[j as usize] = i;
                    }
                    return;
                }
                self.path.push((
                    tail,
                    postings_starts[rarest_entries[tail as usize] as usize],
                    !0,
                ));
            }
        }
    }
    tails.clear();
    tails.resize(movable.len(), !0);
    tried.clear();
    tried.resize(movable.len(), !0);
    let mut absorber = Absorber {
        states,
        movable,
        to_end_lens,
        postings,
        postings_starts,
        rarest_entries,
        tried,
        stamp: 0,
        path,
    };
    for &key in by_len.iter() {
        let i = key as u32;
        absorber.stamp = i;
        absorber.absorb(tails, i);
    }

    order_starts.clear();
    order_starts.resize(states_len, 0); // 0 = keep the order it has.
    order_cells.clear();
    order_cells.push(0); // index 0 stands for "no order of its own".
    for &key in by_len.iter().rev() {
        let i = key as u32;
        let tail = tails[i as usize];
        if tail == !0 {
            continue;
        }
        let head = movable[i as usize];
        let tail_head = movable[tail as usize];
        order_starts[head as usize] = order_cells.len() as u32;
        let mut p = head;
        let mut q = tail_head;
        while p != 0 {
            let p_state = &states[p as usize];
            while q != 0 && states[q as usize].tile < p_state.tile {
                q = states[q as usize].next_index;
            }
            if q != 0 && states[q as usize].tile == p_state.tile {
                q = states[q as usize].next_index;
            } else {
                order_cells.push(p);
            }
            p = p_state.next_index;
        }
        let tail_order_start = order_starts[tail_head as usize] as usize;
        if tail_order_start != 0 {
            order_cells.extend_from_within(
                tail_order_start..tail_order_start + to_end_lens[tail_head as usize] as usize,
            );
        } else {
            let mut p = tail_head;
            while p != 0 {
                order_cells.push(p);
                p = states[p as usize].next_index;
            }
        }
    }

    new_states.clear();
    new_states.reserve(states_len);
    new_states.push(states[0].clone());
    unlimited(states_finder.clear(states_len, None));
    unlimited(states_finder.find_or_insert(&states[0], 0, None));
    let mut state_maker = StateMaker {
        states: new_states,
        states_finder,
        memory: None,
    };
    new_heads.clear();
    new_heads.resize(states_len, 0);
    for p in 1..states_len {
        if !is_head[p] {
            continue;
        }
        transitions.clear();
        let order_start = order_starts[p] as usize;
        if order_start != 0 {
            for &cell in order_cells[order_start..order_start + to_end_lens[p] as usize].iter() {
                let state = &states[cell as usize];
                transitions.push(Transition {
                    tile: state.tile,
                    accepts: state.accepts,
                    arc_index: new_heads[state.arc_index as usize],
                });
            }
        } else {
            let mut q = p as u32;
            while q != 0 {
                let state = &states[q as usize];
                transitions.push(Transition {
                    tile: state.tile,
                    accepts: state.accepts,
                    arc_index: new_heads[state.arc_index as usize],
                });
                q = state.next_index;
            }
        }
        new_heads[p] = unlimited(state_maker.make_state(transitions));
    }

    (
        new_heads[dawg_start_state as usize],
        new_heads[gaddag_start_state as usize],
    )
}

// a word graph builder: every buffer a build fills, kept from one build to the
// next, so that a builder used again, as for a graph pruned for each board,
// allocates only what a bigger graph needs. Each build gives the graph's
// bytes, held until the next
#[derive(Default)]
pub struct Builder {
    states: Vec<State>,
    states_finder: StatesFinder,
    transitions: Vec<Transition>,
    indexes: Vec<usize>,
    machine_drowwords: PackedWords,
    alphagrams: PackedWords,
    reorder: ReorderBuffers,
    head_indexes: Vec<u32>,
    to_end_lens: Vec<u32>,
    destination: Vec<u32>,
    num_ways: Vec<u32>,
    top_indexes: Vec<u32>,
    used_in_dawg: Vec<bool>,
    order: Vec<u128>, // the defrag's, as order_by_ways gives it
    stack: Vec<(u32, u32, u32)>,
    block_len: Vec<u8>,
    blocks_with_len: [Vec<u32>; 16],
    out: Vec<u8>,
}

impl Builder {
    #[inline(always)]
    pub fn build(
        &mut self,
        build_content: BuildContent,
        build_layout: BuildLayout,
        build_order: BuildOrder,
        machine_words: &[bites::Bites],
    ) -> error::Returns<&[u8]> {
        self.do_build::<1, _>(
            build_content,
            build_layout,
            build_order,
            machine_words,
            None,
        )?;
        Ok(&self.out)
    }

    #[inline(always)]
    pub fn build_big(
        &mut self,
        build_content: BuildContent,
        build_layout: BuildLayout,
        build_order: BuildOrder,
        machine_words: &[bites::Bites],
    ) -> error::Returns<&[u8]> {
        self.do_build::<2, _>(
            build_content,
            build_layout,
            build_order,
            machine_words,
            None,
        )?;
        Ok(&self.out)
    }

    // as build_for_game
    #[inline]
    pub fn build_for_game(
        &mut self,
        game_config: &game_config::GameConfig,
        machine_words: &[bites::Bites],
    ) -> error::Returns<&[u8]> {
        self.build_for_game_within(game_config, machine_words, None)
    }

    // build_for_game, every growth of the build's own taken only within
    // memory, if given: refused, the error is an endgame::Refused
    #[inline]
    pub fn build_for_game_within(
        &mut self,
        game_config: &game_config::GameConfig,
        machine_words: &[bites::Bites],
        memory: Option<&endgame::MemoryLimit>,
    ) -> error::Returns<&[u8]> {
        match game_config.game_rules() {
            game_config::GameRules::Classic | game_config::GameRules::Woadwage => self
                .do_build::<2, _>(
                    BuildContent::Gaddawg,
                    BuildLayout::Wolges,
                    BuildOrder::Sorted,
                    machine_words,
                    memory,
                )?,
            game_config::GameRules::Jumbled | game_config::GameRules::JumbledWoadwage => {
                // lent to the build, then put back for the next
                let mut alphagrams = std::mem::take(&mut self.alphagrams);
                let built = fill_packed_alphagrams(machine_words, &mut alphagrams, memory)
                    .map_err(error::BoxAnyError::from)
                    .and_then(|()| {
                        self.do_build::<2, _>(
                            BuildContent::DawgOnly,
                            BuildLayout::Wolges,
                            BuildOrder::Sorted,
                            &alphagrams,
                            memory,
                        )
                    });
                self.alphagrams = alphagrams;
                built?
            }
        }
        Ok(&self.out)
    }

    // machine_words must be sorted and unique. Every growth is taken within
    // memory, if given (a reordered build's, never asked of a limited one,
    // aside)
    #[inline]
    fn do_build<const VARIANT: u8, W: MachineWords + ?Sized>(
        &mut self,
        build_content: BuildContent,
        build_layout: BuildLayout,
        build_order: BuildOrder,
        machine_words: &W,
        memory: Option<&endgame::MemoryLimit>,
    ) -> error::Returns<()> {
        // a word ends on an accepting arc, so an empty word has nowhere to
        // end; sorted, it can only come first
        if machine_words.num_words() > 0 && machine_words.word(0).is_empty() {
            return_error!("a word graph cannot hold an empty word".into());
        }
        let Self {
            states,
            states_finder,
            transitions,
            indexes,
            machine_drowwords,
            alphagrams: _,
            reorder,
            head_indexes,
            to_end_lens,
            destination,
            num_ways,
            top_indexes,
            used_in_dawg,
            order,
            stack,
            block_len,
            blocks_with_len,
            out,
        } = self;

        // The sink state always exists.
        states.clear();
        endgame::room_in(memory, states, 1)?;
        states.push(State {
            tile: 0,
            accepts: false,
            arc_index: 0,
            next_index: 0,
        });

        // room for a state a word, or for as many as the last build made
        states_finder.clear(machine_words.num_words().max(states_finder.len), memory)?;
        states_finder.find_or_insert(&states[0], 0, memory)?;

        let mut state_maker = StateMaker {
            states,
            states_finder,
            memory,
        };
        let mut transition_stack = TransitionStack {
            transitions,
            indexes,
        };
        let dawg_start_state =
            state_maker.make_dawg(&mut transition_stack, machine_words, 0, false)?;
        let gaddag_start_state = match build_content {
            BuildContent::DawgOnly => 0,
            BuildContent::Gaddawg => {
                gen_machine_drowwords(
                    machine_words,
                    state_maker.states,
                    dawg_start_state,
                    machine_drowwords,
                    memory,
                )?;
                state_maker.make_dawg(
                    &mut transition_stack,
                    machine_drowwords,
                    dawg_start_state,
                    true,
                )?
            }
        };

        let (states, dawg_start_state, gaddag_start_state) = match build_order {
            BuildOrder::Sorted => (&states[..], dawg_start_state, gaddag_start_state),
            BuildOrder::Reordered => {
                let (dawg_start_state, gaddag_start_state) = reorder_states(
                    states,
                    &build_content,
                    dawg_start_state,
                    gaddag_start_state,
                    reorder,
                    states_finder,
                    transitions,
                );
                (
                    &reorder.new_states[..],
                    dawg_start_state,
                    gaddag_start_state,
                )
            }
        };

        match build_layout {
            BuildLayout::Legacy
            | BuildLayout::MagpieMerged
            | BuildLayout::Experimental
            | BuildLayout::Wolges => gen_head_indexes(states, head_indexes, memory)?,
            BuildLayout::Magpie => head_indexes.clear(),
        }
        gen_to_end_lens(states, to_end_lens, memory)?;
        destination.clear();
        endgame::room_in(memory, destination, states.len())?;
        destination.resize(states.len(), 0);
        let mut states_defragger = StatesDefragger {
            states,
            head_indexes,
            to_end_lens,
            destination,
            num_written: match build_content {
                BuildContent::DawgOnly => 1,
                BuildContent::Gaddawg => 2,
            },
            stack,
            memory,
            refused: false,
        };
        let mut params = StateDefraggerExperimentalParams {
            block_len,
            blocks_with_len,
        };
        states_defragger.destination[0] = !0; // useful for empty lexicon
        match build_layout {
            BuildLayout::Legacy => states_defragger.defrag_legacy(dawg_start_state),
            BuildLayout::Magpie => states_defragger.defrag_magpie(dawg_start_state),
            BuildLayout::MagpieMerged => states_defragger.defrag_magpie_merged(dawg_start_state),
            BuildLayout::Experimental => {
                gen_num_ways(
                    states,
                    &build_content,
                    dawg_start_state,
                    gaddag_start_state,
                    num_ways,
                    None,
                )?;
                gen_top_indexes(states, states_defragger.head_indexes, top_indexes);
                states_defragger.build_experimental(num_ways, top_indexes, order, &mut params)
            }
            BuildLayout::Wolges => {
                gen_num_ways(
                    states,
                    &build_content,
                    dawg_start_state,
                    gaddag_start_state,
                    num_ways,
                    memory,
                )?;
                states_defragger.build_wolges(
                    num_ways,
                    &build_content,
                    dawg_start_state,
                    order,
                    used_in_dawg,
                    &mut params,
                )
            }
        }
        match build_content {
            BuildContent::DawgOnly => {}
            BuildContent::Gaddawg => match build_layout {
                BuildLayout::Legacy => states_defragger.defrag_legacy(gaddag_start_state),
                BuildLayout::Magpie => states_defragger.defrag_magpie(gaddag_start_state),
                BuildLayout::MagpieMerged => {
                    states_defragger.defrag_magpie_merged(gaddag_start_state)
                }
                BuildLayout::Experimental | BuildLayout::Wolges => {}
            },
        }
        states_defragger.destination[0] = 0; // useful for empty lexicon
        if states_defragger.refused {
            return Err(endgame::Refused.into());
        }

        if states_defragger.num_written
            > match VARIANT {
                1 => 0x400000,
                2 => 0x1000000,
                _ => 0,
            }
        {
            // the format can only have 0x400000 elements, each has 4 bytes
            return_error!(format!(
                "this format cannot have {} nodes",
                states_defragger.num_written
            ));
        }

        out.clear();
        endgame::room_in(memory, out, (states_defragger.num_written as usize) * 4)?;
        states_defragger.to_vec::<VARIANT>(
            build_content,
            dawg_start_state,
            gaddag_start_state,
            out,
        );
        Ok(())
    }
}

#[inline(always)]
pub fn build(
    build_content: BuildContent,
    build_layout: BuildLayout,
    build_order: BuildOrder,
    machine_words: &[bites::Bites],
) -> error::Returns<bites::Bites> {
    Ok(Builder::default()
        .build(build_content, build_layout, build_order, machine_words)?
        .into())
}

#[inline(always)]
pub fn build_big(
    build_content: BuildContent,
    build_layout: BuildLayout,
    build_order: BuildOrder,
    machine_words: &[bites::Bites],
) -> error::Returns<bites::Bites> {
    Ok(Builder::default()
        .build_big(build_content, build_layout, build_order, machine_words)?
        .into())
}

// the graph a game of this config generates on, of these words in ascending
// order: a gaddawg under classic rules, under jumbled ones a dawg of their
// alphagrams
#[inline]
pub fn build_for_game(
    game_config: &game_config::GameConfig,
    machine_words: &[bites::Bites],
) -> error::Returns<bites::Bites> {
    Ok(Builder::default()
        .build_for_game(game_config, machine_words)?
        .into())
}

// a graph build_for_game made, loaded: its runs ascend (kwg::Kwg::runs_ascend)
// by how it is built, as make_dawg lays each state's arcs out in the order of
// the sorted words, and the gaddag's are of words sorted too, so its run
// letters are found with no scan to check that they do
pub struct GameGraph {
    graph: kwg::RunGraph<kwg::Node24>,
}

impl GameGraph {
    #[inline(always)]
    pub fn kwg(&self) -> &kwg::Kwg<kwg::Node24> {
        self.graph.kwg()
    }

    // the graph's run letters (kwg::Kwg::run_letters), found as it was made
    // with no scan, its runs ascending
    #[inline(always)]
    pub fn run_letters(&self) -> kwg::RunLetters<'_> {
        self.graph.run_letters()
    }
}

impl Default for GameGraph {
    // no nodes, to be read only once refill_graph_for_game has filled it
    #[inline(always)]
    fn default() -> Self {
        Self {
            graph: kwg::RunGraph::empty(),
        }
    }
}

impl Builder {
    // graph made again in place as build_for_game's graph, its nodes where
    // they were (kwg::Kwg::refill_from_bytes), and its run letters found again
    #[inline]
    pub fn refill_graph_for_game(
        &mut self,
        graph: &mut GameGraph,
        game_config: &game_config::GameConfig,
        machine_words: &[bites::Bites],
    ) -> error::Returns<()> {
        (graph.graph).refill_ascending(self.build_for_game(game_config, machine_words)?)
    }
}

// build_for_game's graph, loaded
#[inline]
pub fn graph_for_game(
    game_config: &game_config::GameConfig,
    machine_words: &[bites::Bites],
) -> error::Returns<GameGraph> {
    let kwg = kwg::Kwg::from_bytes_alloc(&build_for_game(game_config, machine_words)?)?;
    Ok(GameGraph {
        graph: kwg::RunGraph::ascending(kwg),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::alphabet;
    use crate::kwg::{self, Node};

    #[test]
    #[inline]
    fn a_game_graph_matches_the_game_rules() {
        let classic = game_config::make_english_game_config();
        let jumbled = game_config::make_jumbled_english_game_config();
        let reader = alphabet::AlphabetReader::new_for_words(classic.alphabet());
        let mut buf = Vec::new();
        reader.set_word("DOG", &mut buf).unwrap();
        let word: bites::Bites = buf[..].into();
        let mut sorted_buf = buf.clone();
        sorted_buf.sort_unstable();
        let sorted: bites::Bites = sorted_buf[..].into();

        assert_ne!(word, sorted);
        let words = [word.clone()];

        let present = |kwg: &kwg::Kwg<kwg::Node24>, w: &[u8]| {
            let counts = kwg.count_dawg_words_alloc().unwrap();
            kwg.get_word_index(&counts, kwg[0].arc_index(), w) != !0
        };

        let classic_kwg =
            kwg::Kwg::<kwg::Node24>::from_bytes_alloc(&build_for_game(&classic, &words).unwrap())
                .unwrap();

        assert!(present(&classic_kwg, &word));
        assert!(!present(&classic_kwg, &sorted));

        let jumbled_kwg =
            kwg::Kwg::<kwg::Node24>::from_bytes_alloc(&build_for_game(&jumbled, &words).unwrap())
                .unwrap();

        assert!(present(&jumbled_kwg, &sorted));
        assert!(!present(&jumbled_kwg, &word));
    }

    fn collect_dawg_words<N: kwg::Node>(
        kwg: &kwg::Kwg<N>,
        p: i32,
        word: &mut Vec<u8>,
        out: &mut Vec<bites::Bites>,
    ) {
        if p <= 0 {
            return;
        }
        let mut i = p;
        loop {
            let node = kwg[i];
            word.push(node.tile());
            if node.accepts() {
                out.push(word[..].into());
            }
            if node.arc_index() != 0 {
                collect_dawg_words(kwg, node.arc_index(), word, out);
            }
            word.pop();
            if node.is_end() {
                break;
            }
            i += 1;
        }
    }

    fn collect_gaddag_words<N: kwg::Node>(
        kwg: &kwg::Kwg<N>,
        p: i32,
        rev: &mut Vec<u8>,
        out: &mut Vec<bites::Bites>,
    ) {
        let mut i = p;
        loop {
            let node = kwg[i];
            if node.tile() == 0 {
                let mut suffixes = Vec::new();
                collect_dawg_words(kwg, node.arc_index(), &mut Vec::new(), &mut suffixes);
                for suffix in suffixes.iter() {
                    let mut word = rev.clone();
                    word.reverse();
                    word.extend_from_slice(suffix);
                    out.push(word[..].into());
                }
            } else {
                rev.push(node.tile());
                if node.accepts() {
                    let mut word = rev.clone();
                    word.reverse();
                    out.push(word[..].into());
                }
                if node.arc_index() != 0 {
                    collect_gaddag_words(kwg, node.arc_index(), rev, out);
                }
                rev.pop();
            }
            if node.is_end() {
                break;
            }
            i += 1;
        }
    }

    fn collect_dawg_tiles<N: kwg::Node>(
        kwg: &kwg::Kwg<N>,
        p: i32,
        seen: &mut [bool],
        out: &mut Vec<u8>,
    ) {
        if p <= 0 || seen[p as usize] {
            return;
        }
        let mut i = p;
        loop {
            seen[i as usize] = true;
            let node = kwg[i];
            out.push(node.tile());
            if node.arc_index() != 0 {
                collect_dawg_tiles(kwg, node.arc_index(), seen, out);
            }
            if node.is_end() {
                break;
            }
            i += 1;
        }
    }

    #[inline]
    fn build_kwg(
        layout: BuildLayout,
        order: BuildOrder,
        words: &[&str],
    ) -> (Vec<bites::Bites>, kwg::Kwg<kwg::Node22>) {
        let machine_words: Vec<bites::Bites> = words
            .iter()
            .map(|w| w.bytes().collect::<Vec<u8>>()[..].into())
            .collect();
        let bytes = build(BuildContent::Gaddawg, layout, order, &machine_words).unwrap();
        (
            machine_words,
            kwg::Kwg::<kwg::Node22>::from_bytes_alloc(&bytes).unwrap(),
        )
    }

    #[inline(always)]
    fn round_trip_dawg(layout: BuildLayout, words: &[&str]) {
        round_trip_ordered(layout, BuildOrder::Sorted, words);
    }

    #[inline]
    fn round_trip_ordered(layout: BuildLayout, order: BuildOrder, words: &[&str]) {
        let (machine_words, kwg) = build_kwg(layout, order, words);
        let mut expected = machine_words.clone();
        expected.sort_unstable();
        expected.dedup();

        let mut got = Vec::new();
        collect_dawg_words(&kwg, kwg[0].arc_index(), &mut Vec::new(), &mut got);
        got.sort_unstable();
        assert_eq!(got, expected);

        let mut got_gaddag = Vec::new();
        if kwg[1].arc_index() != 0 {
            collect_gaddag_words(&kwg, kwg[1].arc_index(), &mut Vec::new(), &mut got_gaddag);
        }
        got_gaddag.sort_unstable();
        got_gaddag.dedup();
        assert_eq!(got_gaddag, expected);
    }

    static WORD_LIST: &[&str] = &[
        "AA", "AAH", "AAHED", "AAL", "AALS", "AAS", "AB", "ABA", "ABAC", "ABS", "ABY", "CAB",
        "CAD", "CAT", "CATS", "ZAP", "ZAPS", "ZED", "ZOO",
    ];

    static LONGER_WORD_LIST: &[&str] = &[
        "AE", "AH", "AI", "AL", "AN", "AR", "AS", "AT", "EAR", "EAT", "ERA", "ETA", "HAE", "HAT",
        "HEAR", "HEART", "HEAT", "HEATER", "HER", "HERS", "LEA", "LEAN", "LEARN", "LEARNS",
        "LEAST", "NEAR", "NEAT", "RAT", "RATE", "REAL", "SEAT", "SHEAR", "STEAL", "TEA", "TEAL",
        "TEAR", "TEARS", "THE", "THEN", "THERE", "TREAT",
    ];

    #[test]
    #[inline(always)]
    fn round_trip_wolges() {
        round_trip_dawg(BuildLayout::Wolges, WORD_LIST);
    }

    #[test]
    #[inline(always)]
    fn round_trip_legacy() {
        round_trip_dawg(BuildLayout::Legacy, WORD_LIST);
    }

    #[test]
    #[inline(always)]
    fn round_trip_empty() {
        round_trip_dawg(BuildLayout::Wolges, &[]);
    }

    #[test]
    #[inline(always)]
    fn round_trip_single() {
        round_trip_dawg(BuildLayout::Wolges, &["HELLO"]);
    }

    #[test]
    #[inline]
    fn empty_graph_constants_are_what_the_builder_writes() {
        assert_eq!(
            &build(
                BuildContent::Gaddawg,
                BuildLayout::Wolges,
                BuildOrder::Sorted,
                &[]
            )
            .unwrap()[..],
            kwg::EMPTY_KWG_BYTES,
        );
        assert_eq!(
            &build_big(
                BuildContent::Gaddawg,
                BuildLayout::Wolges,
                BuildOrder::Sorted,
                &[]
            )
            .unwrap()[..],
            kwg::EMPTY_KBWG_BYTES,
        );
    }

    #[test]
    #[inline]
    fn empty_leaves_constant_is_what_the_format_says() {
        let dawg = build(
            BuildContent::DawgOnly,
            BuildLayout::Wolges,
            BuildOrder::Sorted,
            &[],
        )
        .unwrap();
        let mut expected = Vec::new();
        expected.extend_from_slice(&((dawg.len() / 4) as u32).to_le_bytes());
        expected.extend_from_slice(&dawg);
        expected.extend_from_slice(&0u32.to_le_bytes());
        assert_eq!(expected, crate::klv::EMPTY_KLV_BYTES);

        let klv =
            crate::klv::Klv::<kwg::Node22>::from_bytes_alloc(crate::klv::EMPTY_KLV_BYTES).unwrap();
        assert_eq!(klv.leave_value_from_tally(&[0, 1]).raw(), 0);
    }

    #[inline]
    fn shipped_alphabets() -> Vec<(&'static str, alphabet::Alphabet)> {
        vec![
            ("catalan", alphabet::make_catalan_alphabet()),
            ("super_catalan", alphabet::make_super_catalan_alphabet()),
            ("decimal", alphabet::make_decimal_alphabet()),
            ("dutch", alphabet::make_dutch_alphabet()),
            ("english", alphabet::make_english_alphabet()),
            ("french", alphabet::make_french_alphabet()),
            ("german", alphabet::make_german_alphabet()),
            ("super_german", alphabet::make_super_german_alphabet()),
            ("hex", alphabet::make_hex_alphabet()),
            (
                "hong_kong_english",
                alphabet::make_hong_kong_english_alphabet(),
            ),
            ("italian", alphabet::make_italian_alphabet()),
            ("norwegian", alphabet::make_norwegian_alphabet()),
            ("polish", alphabet::make_polish_alphabet()),
            ("slovene", alphabet::make_slovene_alphabet()),
            ("spanish", alphabet::make_spanish_alphabet()),
            ("super_english", alphabet::make_super_english_alphabet()),
            ("swedish", alphabet::make_swedish_alphabet()),
        ]
    }

    #[inline]
    fn words_over_tiles(len: u8) -> Vec<bites::Bites> {
        let last = len - 1;
        let mut words = Vec::new();
        for tile in 1..len {
            words.push(vec![tile, tile][..].into());
            words.push(vec![tile, last][..].into());
            words.push(vec![last, tile][..].into());
            words.push(vec![1, tile, last][..].into());
            words.push(vec![tile, tile, tile, last, 1][..].into());
        }
        words.sort_unstable();
        words.dedup();
        words
    }

    #[inline]
    fn words_in_graph<N: kwg::Node>(bytes: &[u8]) -> Vec<bites::Bites> {
        let kwg = kwg::Kwg::<N>::from_bytes_alloc(bytes).unwrap();
        let mut got = Vec::new();
        collect_dawg_words(&kwg, kwg[0].arc_index(), &mut Vec::new(), &mut got);
        got.sort_unstable();
        got
    }

    #[test]
    #[inline]
    fn every_graph_holds_every_shipped_alphabet() {
        for (name, alphabet) in shipped_alphabets() {
            let len = alphabet.len();
            assert!(len >= 2, "{name}: an alphabet is a blank and some tiles");
            let words = words_over_tiles(len);

            let bytes = build(
                BuildContent::Gaddawg,
                BuildLayout::Wolges,
                BuildOrder::Sorted,
                &words,
            )
            .unwrap_or_else(|e| panic!("{name}: kwg refused {len} tiles: {e}"));
            assert_eq!(words_in_graph::<kwg::Node22>(&bytes), words, "{name}: kwg");

            let bytes = build(
                BuildContent::DawgOnly,
                BuildLayout::Wolges,
                BuildOrder::Sorted,
                &words,
            )
            .unwrap_or_else(|e| panic!("{name}: dawg refused {len} tiles: {e}"));
            assert_eq!(words_in_graph::<kwg::Node22>(&bytes), words, "{name}: dawg");

            let bytes = build_big(
                BuildContent::Gaddawg,
                BuildLayout::Wolges,
                BuildOrder::Sorted,
                &words,
            )
            .unwrap_or_else(|e| panic!("{name}: kbwg refused {len} tiles: {e}"));
            assert_eq!(words_in_graph::<kwg::Node24>(&bytes), words, "{name}: kbwg");

            let alphagrams = make_alphagrams(&words);
            let bytes = build(
                BuildContent::DawgOnly,
                BuildLayout::Wolges,
                BuildOrder::Sorted,
                &alphagrams,
            )
            .unwrap_or_else(|e| panic!("{name}: kad refused {len} tiles: {e}"));
            assert_eq!(
                words_in_graph::<kwg::Node22>(&bytes),
                alphagrams.to_vec(),
                "{name}: kad",
            );
        }
    }

    #[test]
    #[inline]
    fn the_matrix_holds_a_full_sixty_four_tile_alphabet() {
        let widest = shipped_alphabets()
            .iter()
            .map(|(_, alphabet)| alphabet.len())
            .max();
        assert_eq!(
            widest,
            Some(64),
            "no shipped alphabet fills the tile field, so nothing here tests it",
        );
    }

    #[test]
    #[inline]
    fn the_matrix_holds_every_alphabet_the_crate_ships() {
        // cargo runs each test from its package's root
        let dir = std::path::Path::new("src").join("alphabets");
        let mut on_disk = std::fs::read_dir(&dir)
            .unwrap_or_else(|e| panic!("cannot read {}: {e}", dir.display()))
            .map(|entry| {
                entry
                    .unwrap()
                    .file_name()
                    .to_string_lossy()
                    .trim_end_matches(".txt")
                    .to_string()
            })
            .collect::<Vec<_>>();
        on_disk.sort_unstable();
        let mut in_matrix = shipped_alphabets()
            .iter()
            .map(|(name, _)| name.to_string())
            .collect::<Vec<_>>();
        in_matrix.sort_unstable();
        assert_eq!(in_matrix, on_disk, "the matrix and the crate disagree");
    }

    #[test]
    #[inline]
    fn round_trip_reordered() {
        round_trip_ordered(BuildLayout::Wolges, BuildOrder::Reordered, WORD_LIST);
        round_trip_ordered(BuildLayout::Legacy, BuildOrder::Reordered, WORD_LIST);
        round_trip_ordered(BuildLayout::MagpieMerged, BuildOrder::Reordered, WORD_LIST);
        round_trip_ordered(BuildLayout::Wolges, BuildOrder::Reordered, &[]);
        round_trip_ordered(BuildLayout::Wolges, BuildOrder::Reordered, &["HELLO"]);
    }

    #[test]
    #[inline]
    fn a_game_graph_runs_ascend_and_has_its_run_letters() {
        for game_config in [
            game_config::make_english_game_config(),
            game_config::make_jumbled_english_game_config(),
        ] {
            let reader = alphabet::AlphabetReader::new_for_words(game_config.alphabet());
            let mut buf = Vec::new();
            let mut words = (LONGER_WORD_LIST.iter())
                .map(|word| {
                    reader.set_word(word, &mut buf).unwrap();
                    bites::Bites::from(&buf[..])
                })
                .collect::<Vec<_>>();
            words.sort_unstable();
            words.dedup();
            let graph = graph_for_game(&game_config, &words).unwrap();
            assert!(graph.kwg().runs_ascend());
            let found = graph.kwg().run_letters();
            assert_eq!(
                graph.run_letters().of(graph.kwg()).unwrap(),
                found.of(graph.kwg()).unwrap()
            );
        }
    }

    #[test]
    #[inline]
    fn reordered_gaddawg_keeps_its_dawg() {
        let (_, sorted) = build_kwg(BuildLayout::Wolges, BuildOrder::Sorted, LONGER_WORD_LIST);
        let (_, reordered) =
            build_kwg(BuildLayout::Wolges, BuildOrder::Reordered, LONGER_WORD_LIST);

        let mut sorted_tiles = Vec::new();
        collect_dawg_tiles(
            &sorted,
            sorted[0].arc_index(),
            &mut vec![false; sorted.0.len()],
            &mut sorted_tiles,
        );
        let mut reordered_tiles = Vec::new();
        collect_dawg_tiles(
            &reordered,
            reordered[0].arc_index(),
            &mut vec![false; reordered.0.len()],
            &mut reordered_tiles,
        );
        assert_eq!(sorted_tiles, reordered_tiles);

        let mut num_descents = 0;
        for i in 1..reordered.0.len() - 1 {
            if !reordered[i as i32].is_end()
                && reordered[i as i32].tile() > reordered[i as i32 + 1].tile()
            {
                num_descents += 1;
            }
        }
        assert!(num_descents > 0, "nothing was reordered");

        let mut num_moved_turnarounds = 0;
        for i in 2..reordered.0.len() {
            num_moved_turnarounds +=
                (reordered[i as i32].tile() == 0 && !reordered[i as i32 - 1].is_end()) as usize;
        }
        assert!(
            num_moved_turnarounds > 0,
            "no turnaround tile moved off the front of its list"
        );
    }

    #[test]
    #[inline]
    fn reordered_dawg_only_round_trips() {
        let machine_words: Vec<bites::Bites> = WORD_LIST
            .iter()
            .map(|w| w.bytes().collect::<Vec<u8>>()[..].into())
            .collect();
        let bytes = build(
            BuildContent::DawgOnly,
            BuildLayout::Wolges,
            BuildOrder::Reordered,
            &machine_words,
        )
        .unwrap();
        let kwg = kwg::Kwg::<kwg::Node22>::from_bytes_alloc(&bytes).unwrap();
        let mut got = Vec::new();
        collect_dawg_words(&kwg, kwg[0].arc_index(), &mut Vec::new(), &mut got);
        got.sort_unstable();
        let mut expected = machine_words.clone();
        expected.sort_unstable();
        expected.dedup();
        assert_eq!(got, expected);
    }

    #[test]
    #[inline]
    fn a_reordered_graph_builds_on_a_small_stack() {
        use rand::prelude::*;
        // random words: reordered, their lists share tails in chains that grow
        // with the word list, which a recursion would follow a frame a list
        let mut rng = rand_xoshiro::Xoshiro256PlusPlus::seed_from_u64(0);
        let mut words = Vec::<bites::Bites>::new();
        for _ in 0..50_000 {
            let len = rng.random_range(2..=12);
            words.push(
                (0..len)
                    .map(|_| rng.random_range(1..=26))
                    .collect::<Vec<u8>>()[..]
                    .into(),
            );
        }
        words.sort_unstable();
        words.dedup();
        let bytes = std::thread::scope(|s| {
            std::thread::Builder::new()
                .stack_size(64 << 10)
                .spawn_scoped(s, || {
                    build(
                        BuildContent::Gaddawg,
                        BuildLayout::Wolges,
                        BuildOrder::Reordered,
                        &words,
                    )
                })
                .unwrap()
                .join()
                .unwrap()
        })
        .unwrap();
        assert_eq!(words_in_graph::<kwg::Node22>(&bytes), words);
    }

    #[test]
    #[inline]
    fn an_empty_word_is_refused_not_a_panic() {
        // a leave file's ",1.5" reaches here as an empty leave
        let machine_words: Vec<bites::Bites> = vec![[0u8; 0][..].into(), [1u8, 2][..].into()];
        for content in [BuildContent::DawgOnly, BuildContent::Gaddawg] {
            assert!(
                build(
                    content,
                    BuildLayout::Wolges,
                    BuildOrder::Sorted,
                    &machine_words,
                )
                .is_err()
            );
        }
    }

    // a game graph built under a memory limit, classic and jumbled, is the one
    // built with none, or refused; a limit of a byte refuses it, one of every
    // byte never does, and one between refuses some growth past the first
    #[test]
    #[inline]
    fn a_game_graph_under_a_limit_is_the_graph_or_refused() {
        let machine_words: Vec<bites::Bites> = WORD_LIST
            .iter()
            .map(|w| w.bytes().collect::<Vec<u8>>()[..].into())
            .collect();
        let held = || 0;
        for game_config in [
            game_config::make_english_game_config(),
            game_config::make_jumbled_english_game_config(),
        ] {
            let mut builder = Builder::default();
            let every = builder
                .build_for_game(&game_config, &machine_words)
                .unwrap()
                .to_vec();
            let mut refused = Vec::new();
            for bytes in [1, 256, 4096, 65536, usize::MAX] {
                let memory = endgame::MemoryLimit {
                    bytes,
                    held,
                    reserved: None,
                };
                match Builder::default().build_for_game_within(
                    &game_config,
                    &machine_words,
                    Some(&memory),
                ) {
                    Ok(built) => assert_eq!(built, &every[..], "{bytes}"),
                    Err(e) => assert!(e.is::<endgame::Refused>(), "{bytes}: {e}"),
                }
                refused.push(
                    Builder::default()
                        .build_for_game_within(&game_config, &machine_words, Some(&memory))
                        .is_err(),
                );
            }
            assert!(
                refused[0] && refused[1] && !refused[refused.len() - 1],
                "{refused:?}"
            );
        }
    }
}
