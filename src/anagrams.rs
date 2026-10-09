// Copyright (C) 2020-2026 Andy Kurnia.

use super::{alphagram, error, game_config, kwg};

const FILTER_BLOCK_BITS: usize = 512;
const FILTER_BLOCK_WORDS: usize = FILTER_BLOCK_BITS / 64;
const FILTER_BLOCK_SHIFT: u32 = FILTER_BLOCK_BITS.trailing_zeros();
const FILTER_PROBES: usize = 3;
// a filter spends at least this many bits on each key, then rounds its blocks
// up to a power of two; the blank filter gets fewer, since only a rack with a
// blank reads it
const KEY_FILTER_BITS: usize = 32;
const BLANK_FILTER_BITS: usize = 16;

#[inline(always)]
fn filter_hash(key: u128, len: u8) -> u64 {
    alphagram::mixed(key).rotate_left(len as u32)
}

#[inline(always)]
fn filter_blocks(keys: usize, bits: usize) -> usize {
    (keys * bits)
        .div_ceil(FILTER_BLOCK_BITS)
        .next_power_of_two()
}

#[inline(always)]
fn filter_bytes(keys: usize, bits: usize) -> usize {
    if keys == 0 {
        return 0;
    }
    filter_blocks(keys, bits) * FILTER_BLOCK_WORDS * std::mem::size_of::<u64>()
}

// the most tiles a play's word holds: no window is longer than the board, and a
// rack's tiles alone spell no longer
#[inline(always)]
fn longest_play(game_config: &game_config::GameConfig) -> u8 {
    let dim = game_config.board_layout().dim();
    (dim.rows.max(dim.cols) as u8).max(game_config.rack_size())
}

// the bytes std's hash table of this capacity allocates: its slots, a power of
// two keeping an eighth free (a quarter below eight), padded to where the
// control bytes start, then a control byte a slot and a group's worth more (16,
// the widest group)
#[inline(always)]
fn table_bytes<T>(capacity: usize) -> usize {
    if capacity == 0 {
        return 0;
    }
    let slots = if capacity < 4 {
        4
    } else if capacity < 8 {
        8
    } else {
        (capacity * 8 / 7).next_power_of_two()
    };
    (slots * std::mem::size_of::<T>()).next_multiple_of(std::mem::align_of::<T>().max(16))
        + slots
        + 16
}

// what a build holds at once, refused before the allocation that would take it
// past max
struct Ledger {
    live: usize,
    max: usize,
    refused: bool,
}

impl Ledger {
    #[inline(always)]
    fn up_to(max: usize) -> Self {
        Self {
            live: 0,
            max,
            refused: false,
        }
    }

    #[inline(always)]
    fn take(&mut self, bytes: usize) -> Option<()> {
        match self.live.checked_add(bytes) {
            Some(live) if live <= self.max => {
                self.live = live;
                Some(())
            }
            _ => {
                self.refused = true;
                None
            }
        }
    }

    #[inline(always)]
    fn give(&mut self, bytes: usize) {
        self.live -= bytes;
    }
}

// a key layout holds two bytes a tile, after a third while it is made
#[inline(always)]
fn layout_for_config(
    game_config: &game_config::GameConfig,
    ledger: &mut Ledger,
) -> Option<alphagram::KeyLayout> {
    let tiles = game_config.alphabet().len() as usize;
    ledger.take(3 * tiles)?;
    let dim = game_config.board_layout().dim();
    let layout = alphagram::KeyLayout::of(game_config.alphabet(), dim.rows.max(dim.cols) as u8);
    ledger.give(tiles);
    layout
}

// one arena, sorted by length, then alphagram, then word
pub struct Anagrams {
    layout: alphagram::KeyLayout,
    max_word_len: u8,
    arena: Box<[u8]>,
    groups: Box<[Table]>,
    blanked: Box<[Table]>,
    answers: Box<[BlankAnswer]>,
    filter: Box<[u64]>,
    filter_block_mask: usize,
    blank_filter: Box<[u64]>,
    blank_filter_block_mask: usize,
}

#[derive(Clone, Copy)]
pub struct Span {
    pub at: u32,
    pub n: u32,
}

#[derive(Clone, Copy)]
struct BlankAnswer {
    span: Span,
    tile: u8,
}

#[derive(Default)]
struct KeyHasher(u64);

impl std::hash::Hasher for KeyHasher {
    #[inline(always)]
    fn finish(&self) -> u64 {
        self.0
    }

    #[inline(always)]
    fn write(&mut self, bytes: &[u8]) {
        for &b in bytes {
            self.0 = self.0.rotate_left(8) ^ b as u64;
        }
    }

    #[inline(always)]
    fn write_u128(&mut self, v: u128) {
        self.0 ^= alphagram::mixed(v);
    }
}

// a key as the tables keep it: a u128's bytes, so no record holding one is
// padded to 16-byte alignment; equal, ordered and hashed as that u128
#[derive(Clone, Copy, PartialEq, Eq)]
struct KeyBytes([u8; 16]);

impl KeyBytes {
    #[inline(always)]
    fn of(key: u128) -> Self {
        Self(key.to_ne_bytes())
    }

    #[inline(always)]
    fn key(self) -> u128 {
        u128::from_ne_bytes(self.0)
    }
}

impl Ord for KeyBytes {
    #[inline(always)]
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.key().cmp(&other.key())
    }
}

impl PartialOrd for KeyBytes {
    #[inline(always)]
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl std::hash::Hash for KeyBytes {
    #[inline(always)]
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        state.write_u128(self.key());
    }
}

type Table = std::collections::HashMap<KeyBytes, Span, std::hash::BuildHasherDefault<KeyHasher>>;

impl Anagrams {
    #[inline(always)]
    pub fn build_for_config<N: kwg::Node>(
        g: &kwg::Kwg<N>,
        game_config: &game_config::GameConfig,
    ) -> error::Returns<Option<Self>> {
        Self::build_for_config_within(g, game_config, &mut Ledger::up_to(usize::MAX))
    }

    #[inline(always)]
    fn build_for_config_within<N: kwg::Node>(
        g: &kwg::Kwg<N>,
        game_config: &game_config::GameConfig,
        ledger: &mut Ledger,
    ) -> error::Returns<Option<Self>> {
        match layout_for_config(game_config, ledger) {
            Some(layout) => Self::build_within(g, layout, longest_play(game_config), ledger),
            None => Ok(None),
        }
    }

    pub fn build<N: kwg::Node>(
        g: &kwg::Kwg<N>,
        layout: alphagram::KeyLayout,
        longest: u8,
    ) -> error::Returns<Option<Self>> {
        Self::build_within(g, layout, longest, &mut Ledger::up_to(usize::MAX))
    }

    fn build_within<N: kwg::Node>(
        g: &kwg::Kwg<N>,
        layout: alphagram::KeyLayout,
        longest: u8,
        ledger: &mut Ledger,
    ) -> error::Returns<Option<Self>> {
        Ok(match dawg_words(g, ledger)? {
            Some(words) => Self::build_counted(g, layout, longest, words, ledger),
            None => None,
        })
    }

    // g's dawg holds this many words and no loop
    fn build_counted<N: kwg::Node>(
        g: &kwg::Kwg<N>,
        layout: alphagram::KeyLayout,
        longest: u8,
        words: usize,
        ledger: &mut Ledger,
    ) -> Option<Self> {
        // each word the layout keys, by length, key, then its place among the
        // words walked, none longer than longest
        ledger.take(words * std::mem::size_of::<(u8, KeyBytes, u32)>())?;
        let mut keyed = Vec::<(u8, KeyBytes, u32)>::with_capacity(words);
        let mut index = 0u32;
        ledger.take(longest as usize)?;
        for_each_word(g, longest as usize, &mut |w| {
            if let Some(key) = layout.key_of(w) {
                keyed.push((w.len() as u8, KeyBytes::of(key), index));
            }
            index += 1;
        });
        ledger.give(longest as usize);
        keyed.sort_unstable();

        // every table and buffer below is made at its size: the keys of each
        // length, and the pairs a blank makes of them, one per tile of a key
        let lengths = keyed.last().map_or(0, |k| k.0 as usize) + 1;
        ledger.take(2 * lengths * std::mem::size_of::<usize>())?;
        let mut keys_of = vec![0usize; lengths];
        let mut pairs_of = vec![0usize; lengths];
        for group in keyed.chunk_by(|a, b| (a.0, a.1) == (b.0, b.1)) {
            keys_of[group[0].0 as usize] += 1;
            pairs_of[group[0].0 as usize] += layout.tiles_in(group[0].1.key()).count();
        }

        // where each word's letters go in the arena, by index; u32::MAX for a
        // word left out
        ledger.take(index as usize * std::mem::size_of::<u32>())?;
        let mut at = vec![u32::MAX; index as usize];
        let mut letters = 0usize;
        ledger.take(2 * lengths * std::mem::size_of::<Table>())?;
        let mut groups = Vec::<Table>::with_capacity(lengths);
        let mut blanked = Vec::<Table>::with_capacity(lengths);
        let all_pairs = pairs_of.iter().sum::<usize>();
        ledger.take(all_pairs * std::mem::size_of::<BlankAnswer>())?;
        let mut answers = Vec::<BlankAnswer>::with_capacity(all_pairs);
        let mut runs = keyed.chunk_by(|a, b| a.0 == b.0).peekable();
        for len in 0..lengths {
            ledger.take(table_bytes::<(KeyBytes, Span)>(keys_of[len]))?;
            let mut table = Table::with_capacity_and_hasher(keys_of[len], Default::default());
            let pair_bytes = pairs_of[len] * std::mem::size_of::<(KeyBytes, Span, u8)>();
            ledger.take(pair_bytes)?;
            let mut pairs = Vec::<(KeyBytes, Span, u8)>::with_capacity(pairs_of[len]);
            if let Some(run) = runs.next_if(|run| run[0].0 as usize == len) {
                for group in run.chunk_by(|a, b| a.1 == b.1) {
                    let key = group[0].1.key();
                    let span = Span {
                        at: u32::try_from(letters).ok()?,
                        n: u32::try_from(group.len()).ok()?,
                    };
                    for &(_, _, index) in group {
                        at[index as usize] = u32::try_from(letters).ok()?;
                        letters += len;
                    }
                    table.insert(group[0].1, span);
                    for tile in layout.tiles_in(key) {
                        pairs.push((KeyBytes::of(key - layout.place_value(tile)), span, tile));
                    }
                }
            }
            groups.push(table);

            // a length's pairs, gone before the next length's are made
            pairs.sort_unstable_by_key(|&(key, span, tile)| (key, span.at, tile));
            let blank_keys = pairs.chunk_by(|a, b| a.0 == b.0).count();
            ledger.take(table_bytes::<(KeyBytes, Span)>(blank_keys))?;
            let mut table = Table::with_capacity_and_hasher(blank_keys, Default::default());
            for group in pairs.chunk_by(|a, b| a.0 == b.0) {
                table.insert(
                    group[0].0,
                    Span {
                        at: u32::try_from(answers.len()).ok()?,
                        n: group.len() as u32,
                    },
                );
                answers.extend(
                    group
                        .iter()
                        .map(|&(_, span, tile)| BlankAnswer { span, tile }),
                );
            }
            blanked.push(table);
            drop(pairs);
            ledger.give(pair_bytes);
        }
        drop(keyed);
        ledger.give(words * std::mem::size_of::<(u8, KeyBytes, u32)>());

        ledger.take(letters)?;
        let mut arena = vec![0u8; letters].into_boxed_slice();
        let mut index = 0usize;
        ledger.take(longest as usize)?;
        for_each_word(g, longest as usize, &mut |w| {
            if at[index] != u32::MAX {
                arena[at[index] as usize..][..w.len()].copy_from_slice(w);
            }
            index += 1;
        });
        ledger.give(longest as usize);
        drop(at);
        ledger.give(index * std::mem::size_of::<u32>());

        let key_count = groups.iter().map(|t| t.len()).sum::<usize>();
        ledger.take(filter_bytes(key_count, KEY_FILTER_BITS))?;
        let (filter, filter_block_mask) = Self::filter_of(&groups, key_count, KEY_FILTER_BITS);
        let blank_key_count = blanked.iter().map(|t| t.len()).sum::<usize>();
        ledger.take(filter_bytes(blank_key_count, BLANK_FILTER_BITS))?;
        let (blank_filter, blank_filter_block_mask) =
            Self::filter_of(&blanked, blank_key_count, BLANK_FILTER_BITS);
        Some(Anagrams {
            layout,
            max_word_len: longest,
            filter,
            filter_block_mask,
            blank_filter,
            blank_filter_block_mask,
            arena,
            groups: groups.into_boxed_slice(),
            blanked: blanked.into_boxed_slice(),
            answers: answers.into_boxed_slice(),
        })
    }

    #[inline(always)]
    fn filter_may_hold(&self, key: u128, len: u8) -> bool {
        Self::filter_may_hold_in(&self.filter, self.filter_block_mask, key, len)
    }

    #[inline(always)]
    fn blank_filter_may_hold(&self, key: u128, len: u8) -> bool {
        Self::filter_may_hold_in(&self.blank_filter, self.blank_filter_block_mask, key, len)
    }

    #[inline(always)]
    fn filter_may_hold_in(filter: &[u64], block_mask: usize, key: u128, len: u8) -> bool {
        if filter.is_empty() {
            return true;
        }
        let h = filter_hash(key, len);
        let base = ((h >> 32) as usize & block_mask) * FILTER_BLOCK_WORDS;
        let mut probe = h;
        for _ in 0..FILTER_PROBES {
            let bit = probe as usize & (FILTER_BLOCK_BITS - 1);
            if filter[base + (bit >> 6)] >> (bit & 63) & 1 == 0 {
                return false;
            }
            probe >>= FILTER_BLOCK_SHIFT;
        }
        true
    }

    #[inline(always)]
    fn filter_of(groups: &[Table], keys: usize, bits: usize) -> (Box<[u64]>, usize) {
        if keys == 0 {
            return (Box::new([]), 0);
        }
        let blocks = filter_blocks(keys, bits);
        let mut filter = vec![0u64; blocks * FILTER_BLOCK_WORDS].into_boxed_slice();
        let mask = blocks - 1;
        for (len, table) in groups.iter().enumerate() {
            for &key in table.keys() {
                let h = filter_hash(key.key(), len as u8);
                let base = ((h >> 32) as usize & mask) * FILTER_BLOCK_WORDS;
                let mut probe = h;
                for _ in 0..FILTER_PROBES {
                    let bit = probe as usize & (FILTER_BLOCK_BITS - 1);
                    filter[base + (bit >> 6)] |= 1u64 << (bit & 63);
                    probe >>= FILTER_BLOCK_SHIFT;
                }
            }
        }
        (filter, mask)
    }

    #[inline(always)]
    pub fn words_of(&self, len: u8, span: Span) -> alphagram::Words<'_> {
        let from = span.at as usize;
        alphagram::Words {
            words: &self.arena[from..from + span.n as usize * len as usize],
            len,
        }
    }

    // the bytes it holds
    #[inline(always)]
    fn bytes(&self) -> usize {
        let tables = |tables: &[Table]| {
            std::mem::size_of_val(tables)
                + tables
                    .iter()
                    .map(|table| table_bytes::<(KeyBytes, Span)>(table.capacity()))
                    .sum::<usize>()
        };
        self.arena.len()
            + tables(&self.groups)
            + tables(&self.blanked)
            + std::mem::size_of_val(&*self.answers)
            + std::mem::size_of_val(&*self.filter)
            + std::mem::size_of_val(&*self.blank_filter)
    }
}

impl Anagrams {
    #[inline(always)]
    pub fn layout(&self) -> &alphagram::KeyLayout {
        &self.layout
    }

    // they hold every word a game of this config can play, keyed as it keys
    #[inline(always)]
    pub fn fit(&self, game_config: &game_config::GameConfig) -> bool {
        let dim = game_config.board_layout().dim();
        self.layout
            .matches(game_config.alphabet(), dim.rows.max(dim.cols) as u8)
            && self.max_word_len >= longest_play(game_config)
    }

    #[inline(always)]
    pub fn words(&self, key: alphagram::Key, len: u8) -> Option<alphagram::Words<'_>> {
        if !self.filter_may_hold(key.0, len) {
            return None;
        }
        Some(self.words_of(
            len,
            *self.groups.get(len as usize)?.get(&KeyBytes::of(key.0))?,
        ))
    }

    #[inline(always)]
    pub fn span(&self, key: alphagram::Key, len: u8) -> Option<Span> {
        if !self.filter_may_hold(key.0, len) {
            return None;
        }
        self.groups
            .get(len as usize)?
            .get(&KeyBytes::of(key.0))
            .copied()
    }

    #[inline(always)]
    pub fn words_at(&self, at: alphagram::WordsAt) -> alphagram::Words<'_> {
        self.words_of(
            at.len,
            Span {
                at: at.at,
                n: at.n as u32,
            },
        )
    }

    #[inline(always)]
    pub fn blank_groups<F: FnMut(u8, alphagram::WordsAt)>(
        &self,
        key: alphagram::Key,
        len: u8,
        ok: u64,
        mut f: F,
    ) {
        if !self.blank_filter_may_hold(key.0, len) {
            return;
        }
        let Some(table) = self.blanked.get(len as usize) else {
            return;
        };
        let Some(&run) = table.get(&KeyBytes::of(key.0)) else {
            return;
        };
        for &a in &self.answers[run.at as usize..][..run.n as usize] {
            if ok >> a.tile & 1 == 0 {
                continue;
            }
            f(
                a.tile,
                alphagram::WordsAt {
                    at: a.span.at,
                    n: a.span.n as u16,
                    len,
                },
            );
        }
    }
}

pub struct RackLengths {
    layout: alphagram::KeyLayout,
    present: std::collections::HashSet<u128, std::hash::BuildHasherDefault<KeyHasher>>,
}

impl RackLengths {
    #[inline(always)]
    pub fn build_for_config<N: kwg::Node>(
        g: &kwg::Kwg<N>,
        game_config: &game_config::GameConfig,
    ) -> error::Returns<Option<Self>> {
        Self::build_for_config_within(g, game_config, &mut Ledger::up_to(usize::MAX))
    }

    #[inline(always)]
    fn build_for_config_within<N: kwg::Node>(
        g: &kwg::Kwg<N>,
        game_config: &game_config::GameConfig,
        ledger: &mut Ledger,
    ) -> error::Returns<Option<Self>> {
        match layout_for_config(game_config, ledger) {
            Some(layout) => {
                Self::build_within(g, layout, game_config.rack_size() as usize + 1, ledger)
            }
            None => Ok(None),
        }
    }

    fn build_within<N: kwg::Node>(
        g: &kwg::Kwg<N>,
        layout: alphagram::KeyLayout,
        max_tiles: usize,
        ledger: &mut Ledger,
    ) -> error::Returns<Option<Self>> {
        if refuse_a_loop(g, ledger)?.is_none() || ledger.take(max_tiles).is_none() {
            return Ok(None);
        }
        let mut present =
            std::collections::HashSet::<u128, std::hash::BuildHasherDefault<KeyHasher>>::default();
        let mut room = true;
        for_each_word(g, max_tiles, &mut |w| {
            let Some(key) = layout.key_of(w) else {
                return;
            };
            // an insert into a full set moves it to one twice the size, both
            // held while it moves
            let full = room && present.len() == present.capacity();
            if full {
                room = ledger
                    .take(table_bytes::<u128>(present.capacity() + 1))
                    .is_some();
            }
            if room {
                let old = table_bytes::<u128>(present.capacity());
                present.insert(key);
                if full {
                    ledger.give(old);
                }
            }
        });
        ledger.give(max_tiles);
        Ok(room.then_some(Self { layout, present }))
    }

    #[inline(always)]
    pub fn layout(&self) -> &alphagram::KeyLayout {
        &self.layout
    }

    #[inline(always)]
    pub fn contains(&self, key: u128) -> bool {
        self.present.contains(&key)
    }

    // the bytes it holds
    #[inline(always)]
    fn bytes(&self) -> usize {
        table_bytes::<u128>(self.present.capacity())
    }
}

// what the generator reads beside the word graph: the anagrams, or on the lean
// arm the rack lengths alone
pub enum ArmTables {
    Anagrams(Anagrams),
    RackLengths(RackLengths),
    None,
}

impl ArmTables {
    #[inline]
    pub fn build<N: kwg::Node>(
        kwg: &kwg::Kwg<N>,
        game_config: &game_config::GameConfig,
        lean: bool,
    ) -> error::Returns<Self> {
        Self::build_in(kwg, game_config, lean, &mut Ledger::up_to(usize::MAX))
    }

    // the tables, or none if building them would hold more than max_bytes at
    // once; a build is stopped before the allocation that would pass it
    #[inline]
    pub fn build_within<N: kwg::Node>(
        kwg: &kwg::Kwg<N>,
        game_config: &game_config::GameConfig,
        lean: bool,
        max_bytes: usize,
    ) -> error::Returns<Option<Self>> {
        let mut ledger = Ledger::up_to(max_bytes);
        let tables = Self::build_in(kwg, game_config, lean, &mut ledger)?;
        Ok((!ledger.refused).then_some(tables))
    }

    #[inline]
    fn build_in<N: kwg::Node>(
        kwg: &kwg::Kwg<N>,
        game_config: &game_config::GameConfig,
        lean: bool,
        ledger: &mut Ledger,
    ) -> error::Returns<Self> {
        Ok(if lean {
            RackLengths::build_for_config_within(kwg, game_config, ledger)?
                .map_or(Self::None, Self::RackLengths)
        } else {
            Anagrams::build_for_config_within(kwg, game_config, ledger)?
                .map_or(Self::None, Self::Anagrams)
        })
    }

    #[inline(always)]
    pub fn anagrams(&self) -> Option<&Anagrams> {
        match self {
            Self::Anagrams(anagrams) => Some(anagrams),
            Self::RackLengths(_) | Self::None => None,
        }
    }

    #[inline(always)]
    pub fn rack_lengths(&self) -> Option<&RackLengths> {
        match self {
            Self::RackLengths(rack_lengths) => Some(rack_lengths),
            Self::Anagrams(_) | Self::None => None,
        }
    }

    // the bytes the tables hold
    #[inline(always)]
    pub fn bytes(&self) -> usize {
        match self {
            Self::Anagrams(anagrams) => anagrams.bytes(),
            Self::RackLengths(rack_lengths) => rack_lengths.bytes(),
            Self::None => 0,
        }
    }
}

// the words in g's dawg, by a count that refuses a graph that loops; the count
// holds at most 4 bytes a node (a bit a node, then 4 bytes a dawg node)
#[inline(always)]
fn dawg_words<N: kwg::Node>(g: &kwg::Kwg<N>, ledger: &mut Ledger) -> error::Returns<Option<usize>> {
    let held = 4 * g.0.len();
    if ledger.take(held).is_none() {
        return Ok(None);
    }
    let counts = g.count_dawg_words_alloc()?;
    let words = match g[0].arc_index() {
        root if root > 0 => counts[root as usize] as usize,
        _ => 0,
    };
    drop(counts);
    ledger.give(held);
    Ok(Some(words))
}

// g's dawg refused if it loops, by a walk that marks each node while what
// follows it is walked: a byte a node, where the lean arm needs no word count
fn refuse_a_loop<N: kwg::Node>(g: &kwg::Kwg<N>, ledger: &mut Ledger) -> error::Returns<Option<()>> {
    fn visit<N: kwg::Node>(g: &kwg::Kwg<N>, p: i32, state: &mut [u8]) -> error::Returns<()> {
        match state[p as usize] {
            1 => return Err(format!("the word graph loops back to node {p}").into()),
            2 => return Ok(()),
            _ => {}
        }
        state[p as usize] = 1;
        let node = g[p];
        if node.arc_index() > 0 {
            visit(g, node.arc_index(), state)?;
        }
        if !node.is_end() {
            visit(g, p + 1, state)?;
        }
        state[p as usize] = 2;
        Ok(())
    }
    if ledger.take(g.0.len()).is_none() {
        return Ok(None);
    }
    let mut state = vec![0u8; g.0.len()];
    let root = g[0].arc_index();
    if root > 0 {
        visit(g, root, &mut state)?;
    }
    drop(state);
    ledger.give(g.0.len());
    Ok(Some(()))
}

// each word of g's dawg of at most max_len tiles, in the graph's order; g was
// counted first, so it does not loop
fn for_each_word<N: kwg::Node, F: FnMut(&[u8])>(g: &kwg::Kwg<N>, max_len: usize, f: &mut F) {
    fn walk<N: kwg::Node, F: FnMut(&[u8])>(
        g: &kwg::Kwg<N>,
        mut p: i32,
        max_len: usize,
        w: &mut Vec<u8>,
        f: &mut F,
    ) {
        loop {
            let node = g[p];
            w.push(node.tile());
            if node.accepts() {
                f(w);
            }
            if node.arc_index() > 0 && w.len() < max_len {
                walk(g, node.arc_index(), max_len, w, f);
            }
            w.pop();
            if node.is_end() {
                break;
            }
            p += 1;
        }
    }
    let root = g[0].arc_index();
    if root > 0 && max_len > 0 {
        walk(g, root, max_len, &mut Vec::with_capacity(max_len), f);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[inline]
    fn three_letter_words() -> Vec<Vec<u8>> {
        let mut words = Vec::new();
        for a in 1u8..=5 {
            for b in 1u8..=5 {
                for c in 1u8..=5 {
                    if a + b + c != 9 {
                        words.push(vec![a, b, c]);
                    }
                }
            }
        }
        words
    }

    #[inline]
    fn test_layout() -> alphagram::KeyLayout {
        layout_for_config(
            &crate::game_config::make_english_game_config(),
            &mut Ledger::up_to(usize::MAX),
        )
        .unwrap()
    }

    #[inline]
    fn graph_of(words: &[Vec<u8>]) -> kwg::Kwg<kwg::Node22> {
        let mut words = words.to_vec();
        words.sort_unstable();
        let words = words
            .iter()
            .map(|w| w[..].into())
            .collect::<Vec<crate::bites::Bites>>();
        let bytes = crate::build::build(
            crate::build::BuildContent::Gaddawg,
            crate::build::BuildLayout::Wolges,
            crate::build::BuildOrder::Sorted,
            &words,
        )
        .unwrap();
        kwg::Kwg::<kwg::Node22>::from_bytes_alloc(&bytes).unwrap()
    }

    #[inline(always)]
    fn built(words: &[Vec<u8>]) -> Anagrams {
        let longest = longest_play(&crate::game_config::make_english_game_config());
        Anagrams::build(&graph_of(words), test_layout(), longest)
            .unwrap()
            .unwrap()
    }

    #[inline]
    fn from_the_list(words: &[Vec<u8>], layout: &alphagram::KeyLayout, key: u128) -> Vec<Vec<u8>> {
        let mut want = words
            .iter()
            .filter(|w| layout.key_of(w) == Some(key))
            .map(|w| w.to_vec())
            .collect::<Vec<_>>();
        want.sort_unstable();
        want
    }

    #[inline]
    fn sorted(found: Option<alphagram::Words<'_>>) -> Vec<Vec<u8>> {
        let mut got = found
            .map(|w| w.iter().map(|w| w.to_vec()).collect::<Vec<_>>())
            .unwrap_or_default();
        got.sort_unstable();
        got
    }

    #[test]
    #[inline]
    fn nothing_the_tables_hold_is_refused_before_they_are_read() {
        let words = three_letter_words();
        let held = built(&words);
        let layout = test_layout();
        for word in &words {
            let key = layout.key_of(word).unwrap();
            assert!(
                held.filter_may_hold(key, word.len() as u8),
                "{word:?} is in the tables and was refused",
            );
        }
        let absent = layout.key_of(&[9, 9, 9]).unwrap();
        assert!(held.words(alphagram::Fitted(absent), 3).is_none());
        assert!(held.words(alphagram::Fitted(absent), 7).is_none());
    }

    #[test]
    #[inline]
    fn a_key_answers_with_every_word_that_spells_it() {
        let words = three_letter_words();
        let held = built(&words);
        let layout = test_layout();
        let key = alphagram::Fitted(layout.key_of(&[1, 2, 3]).unwrap());
        assert_eq!(
            sorted(held.words(key, 3)),
            vec![
                vec![1u8, 2, 3],
                vec![1, 3, 2],
                vec![2, 1, 3],
                vec![2, 3, 1],
                vec![3, 1, 2],
                vec![3, 2, 1],
            ],
        );
        assert!(held.words(key, 4).is_none());
        assert!(held.words(key, 99).is_none());
    }

    #[test]
    #[inline]
    fn every_word_comes_back_for_the_key_it_spells() {
        let words = three_letter_words();
        let held = built(&words);
        let layout = test_layout();
        let mut hits = 0;
        let mut misses = 0;
        for a in 1u8..=6 {
            for b in 1u8..=6 {
                for c in 1u8..=6 {
                    let raw = layout.key_of(&[a, b, c]).unwrap();
                    let got = sorted(held.words(alphagram::Fitted(raw), 3));
                    assert_eq!(got, from_the_list(&words, &layout, raw), "{a} {b} {c}");
                    if got.is_empty() {
                        misses += 1;
                    } else {
                        hits += 1;
                    }
                }
            }
        }
        assert!(hits > 0 && misses > 0, "{hits} hit, {misses} missed");
    }

    #[test]
    #[inline]
    fn a_blank_hands_back_the_words_a_mask_would_have_found() {
        let words = three_letter_words();
        let held = built(&words);
        let layout = test_layout();
        let mut found = 0;
        for a in 1u8..=6 {
            for b in 1u8..=6 {
                let key = alphagram::Fitted(layout.key_of(&[a, b]).unwrap());
                let mut got = Vec::new();
                held.blank_groups(key, 3, u64::MAX, |_, at| {
                    got.push(sorted(Some(held.words_at(at))));
                });
                let mut want = Vec::new();
                for tile in 1u8..=6 {
                    let spelled = from_the_list(&words, &layout, key.0 + layout.place_value(tile));
                    if !spelled.is_empty() {
                        want.push(spelled);
                    }
                }
                assert_eq!(got, want, "{a} {b}");
                found += got.len();
            }
        }
        assert!(found > 0, "nothing was found");
    }

    #[test]
    #[inline]
    fn a_blank_that_doubles_a_letter_is_subtracted_back_out() {
        let words = [vec![1u8, 1, 2], vec![1, 2, 1], vec![2, 1, 1]];
        let held = built(&words);
        let layout = test_layout();
        let key = alphagram::Fitted(layout.key_of(&[1, 2]).unwrap());
        let mut got = Vec::new();
        let mut letters = Vec::new();
        held.blank_groups(key, 3, u64::MAX, |tile, at| {
            letters.push(tile);
            got.push(sorted(Some(held.words_at(at))));
        });
        assert_eq!(letters, [1]);
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].len(), 3);
    }

    #[test]
    #[inline]
    fn a_word_the_layout_cannot_key_is_left_out_rather_than_kept() {
        let layout = test_layout();
        let twelve = vec![1u8; 12];
        let words = [vec![1u8, 2], twelve.clone()];
        let held = built(&words);
        assert!(layout.key_of(&twelve).is_none());
        let key = alphagram::Fitted(layout.key_of(&[1, 2]).unwrap());
        assert_eq!(sorted(held.words(key, 2)), vec![vec![1u8, 2]]);
    }

    #[test]
    #[inline]
    fn a_word_longer_than_any_play_is_left_out() {
        let gc = crate::game_config::make_english_game_config();
        let layout = test_layout();
        let sixteen = (1u8..=16).collect::<Vec<_>>();
        let g = graph_of(&[vec![1u8, 2], sixteen.clone()]);
        let key = alphagram::Fitted(layout.key_of(&sixteen).unwrap());
        let kept = |longest| {
            let held = Anagrams::build(&g, layout.clone(), longest)
                .unwrap()
                .unwrap();
            held.words(key, 16).is_some()
        };
        assert_eq!(longest_play(&gc), 15);
        assert!(!kept(15));
        assert!(kept(16));
    }

    #[test]
    #[inline]
    fn tables_fit_only_a_game_whose_plays_they_hold() {
        let gc = crate::game_config::make_english_game_config();
        let g = graph_of(&three_letter_words());
        let longest = longest_play(&gc);
        let fit = |longest| {
            Anagrams::build(&g, test_layout(), longest)
                .unwrap()
                .unwrap()
                .fit(&gc)
        };
        assert!(fit(longest));
        assert!(!fit(longest - 1));
        let wide = crate::game_config::make_super_english_game_config();
        let held = Anagrams::build_for_config(&g, &gc).unwrap().unwrap();
        assert!(!held.fit(&wide));
    }

    #[test]
    #[inline]
    fn a_word_graph_that_loops_builds_no_tables() {
        use kwg::Node;
        let gc = crate::game_config::make_english_game_config();
        let words: Vec<crate::bites::Bites> =
            vec![[19u8, 5, 1, 20][..].into(), [20u8, 5, 1][..].into()]; // SEAT, TEA
        let bytes = crate::build::build(
            crate::build::BuildContent::Gaddawg,
            crate::build::BuildLayout::Wolges,
            crate::build::BuildOrder::Sorted,
            &words,
        )
        .unwrap();
        let kwg = kwg::Kwg::<kwg::Node22>::from_bytes_alloc(&bytes).unwrap();
        for lean in [false, true] {
            assert!(ArmTables::build(&kwg, &gc, lean).is_ok());
        }
        // the S that starts the dawg's first run now leads back to that run
        let dawg_root = kwg[0].arc_index() as usize;
        let at = dawg_root * 4;
        let mut looping = bytes.to_vec();
        let node = kwg::read_le_u32(&looping, at);
        looping[at..at + 4].copy_from_slice(&((node & !0x3fffff) | dawg_root as u32).to_le_bytes());
        let kwg = kwg::Kwg::<kwg::Node22>::from_bytes_alloc(&looping).unwrap();
        for lean in [false, true] {
            assert!(ArmTables::build(&kwg, &gc, lean).is_err());
        }
    }

    #[test]
    #[inline]
    fn the_tables_count_the_bytes_they_hold() {
        let gc = crate::game_config::make_english_game_config();
        let words = three_letter_words()
            .iter()
            .map(|w| w[..].into())
            .collect::<Vec<crate::bites::Bites>>();
        let bytes = crate::build::build(
            crate::build::BuildContent::Gaddawg,
            crate::build::BuildLayout::Wolges,
            crate::build::BuildOrder::Sorted,
            &words,
        )
        .unwrap();
        let kwg = kwg::Kwg::<kwg::Node22>::from_bytes_alloc(&bytes).unwrap();
        let anagrams = ArmTables::build(&kwg, &gc, false).unwrap().bytes();
        let lean = ArmTables::build(&kwg, &gc, true).unwrap().bytes();
        // the anagrams hold every word's letters beside its keys, the lean arm
        // the keys alone
        assert!(anagrams > words.len() * 3, "{anagrams}");
        assert!(0 < lean && lean < anagrams, "{lean} {anagrams}");
        let none = ArmTables::None;
        assert_eq!(none.bytes(), 0);
    }
}
