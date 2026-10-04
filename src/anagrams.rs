// Copyright (C) 2020-2026 Andy Kurnia.

use super::{alphagram, error, game_config, kwg};

const FILTER_BLOCK_BITS: usize = 512;
const FILTER_BLOCK_WORDS: usize = FILTER_BLOCK_BITS / 64;
const FILTER_BLOCK_SHIFT: u32 = FILTER_BLOCK_BITS.trailing_zeros();
const FILTER_PROBES: usize = 3;

#[inline(always)]
fn filter_hash(key: u128, len: u8) -> u64 {
    alphagram::mixed(key).rotate_left(len as u32)
}

// about the bytes a hash table of this capacity holds: std's keeps an eighth of
// its slots free, and a control byte beside each
#[inline(always)]
fn hashed_bytes<T>(capacity: usize) -> usize {
    capacity.div_ceil(7) * 8 * (std::mem::size_of::<T>() + 1)
}

#[inline(always)]
fn layout_for_config(game_config: &game_config::GameConfig) -> Option<alphagram::KeyLayout> {
    let dim = game_config.board_layout().dim();
    alphagram::KeyLayout::of(game_config.alphabet(), dim.rows.max(dim.cols) as u8)
}

// one arena, sorted by length, then alphagram, then word
pub struct Anagrams {
    layout: alphagram::KeyLayout,
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

type Table = std::collections::HashMap<u128, Span, std::hash::BuildHasherDefault<KeyHasher>>;

impl Anagrams {
    #[inline(always)]
    pub fn build_for_config<N: kwg::Node>(
        g: &kwg::Kwg<N>,
        game_config: &game_config::GameConfig,
    ) -> error::Returns<Option<Self>> {
        match layout_for_config(game_config) {
            Some(layout) => Self::build(g, layout),
            None => Ok(None),
        }
    }

    pub fn build<N: kwg::Node>(
        g: &kwg::Kwg<N>,
        layout: alphagram::KeyLayout,
    ) -> error::Returns<Option<Self>> {
        let words = dawg_words(g)?;
        Ok(Self::build_counted(g, layout, words))
    }

    // g's dawg holds this many words and no loop
    fn build_counted<N: kwg::Node>(
        g: &kwg::Kwg<N>,
        layout: alphagram::KeyLayout,
        words: usize,
    ) -> Option<Self> {
        // each word the layout keys, by length, key, then its index in the
        // graph, which is its place in the word list
        let mut keyed = Vec::<(u8, u128, u32)>::with_capacity(words);
        let mut index = 0u32;
        let mut too_long = false;
        for_each_word(g, usize::MAX, &mut |w| {
            match u8::try_from(w.len()) {
                Ok(len) => {
                    if let Some(key) = layout.key_of(w) {
                        keyed.push((len, key, index));
                    }
                }
                Err(_) => too_long = true,
            }
            index += 1;
        });
        if too_long {
            return None;
        }
        keyed.sort_unstable();

        // every table and buffer below is made at its size: the keys of each
        // length, and the pairs a blank makes of them, one per tile of a key
        let lengths = keyed.last().map_or(0, |k| k.0 as usize) + 1;
        let mut keys_of = vec![0usize; lengths];
        let mut pairs_of = vec![0usize; lengths];
        for group in keyed.chunk_by(|a, b| (a.0, a.1) == (b.0, b.1)) {
            keys_of[group[0].0 as usize] += 1;
            pairs_of[group[0].0 as usize] += layout.tiles_in(group[0].1).count();
        }

        // where each word's letters go in the arena, by index; u32::MAX for a
        // word left out
        let mut at = vec![u32::MAX; index as usize];
        let mut letters = 0usize;
        let mut groups = Vec::<Table>::with_capacity(lengths);
        let mut blanked = Vec::<Table>::with_capacity(lengths);
        let mut answers = Vec::<BlankAnswer>::with_capacity(pairs_of.iter().sum());
        let mut runs = keyed.chunk_by(|a, b| a.0 == b.0).peekable();
        for len in 0..lengths {
            let mut table = Table::with_capacity_and_hasher(keys_of[len], Default::default());
            let mut pairs = Vec::<(u128, Span, u8)>::with_capacity(pairs_of[len]);
            if let Some(run) = runs.next_if(|run| run[0].0 as usize == len) {
                for group in run.chunk_by(|a, b| a.1 == b.1) {
                    let key = group[0].1;
                    let span = Span {
                        at: u32::try_from(letters).ok()?,
                        n: u32::try_from(group.len()).ok()?,
                    };
                    for &(_, _, index) in group {
                        at[index as usize] = u32::try_from(letters).ok()?;
                        letters += len;
                    }
                    table.insert(key, span);
                    for tile in layout.tiles_in(key) {
                        pairs.push((key - layout.place_value(tile), span, tile));
                    }
                }
            }
            groups.push(table);

            // a length's pairs, gone before the next length's are made
            pairs.sort_unstable_by_key(|&(key, span, tile)| (key, span.at, tile));
            let mut table = Table::with_capacity_and_hasher(
                pairs.chunk_by(|a, b| a.0 == b.0).count(),
                Default::default(),
            );
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
        }
        drop(keyed);

        let mut arena = vec![0u8; letters].into_boxed_slice();
        let mut index = 0usize;
        for_each_word(g, usize::MAX, &mut |w| {
            if at[index] != u32::MAX {
                arena[at[index] as usize..][..w.len()].copy_from_slice(w);
            }
            index += 1;
        });
        drop(at);

        let key_count = groups.iter().map(|t| t.len()).sum::<usize>();
        let (filter, filter_block_mask) = Self::filter_of(&groups, key_count);
        let blank_key_count = blanked.iter().map(|t| t.len()).sum::<usize>();
        let (blank_filter, blank_filter_block_mask) = Self::filter_of(&blanked, blank_key_count);
        Some(Anagrams {
            layout,
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
    fn filter_of(groups: &[Table], keys: usize) -> (Box<[u64]>, usize) {
        if keys == 0 {
            return (Box::new([]), 0);
        }
        let blocks = (keys * 16).div_ceil(FILTER_BLOCK_BITS).next_power_of_two();
        let mut filter = vec![0u64; blocks * FILTER_BLOCK_WORDS].into_boxed_slice();
        let mask = blocks - 1;
        for (len, table) in groups.iter().enumerate() {
            for &key in table.keys() {
                let h = filter_hash(key, len as u8);
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

    // about the bytes it holds
    #[inline(always)]
    fn bytes(&self) -> usize {
        let tables = |tables: &[Table]| {
            std::mem::size_of_val(tables)
                + tables
                    .iter()
                    .map(|table| hashed_bytes::<(u128, Span)>(table.capacity()))
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

    #[inline(always)]
    pub fn words(&self, key: alphagram::Key, len: u8) -> Option<alphagram::Words<'_>> {
        if !self.filter_may_hold(key.0, len) {
            return None;
        }
        Some(self.words_of(len, *self.groups.get(len as usize)?.get(&key.0)?))
    }

    #[inline(always)]
    pub fn span(&self, key: alphagram::Key, len: u8) -> Option<Span> {
        if !self.filter_may_hold(key.0, len) {
            return None;
        }
        self.groups.get(len as usize)?.get(&key.0).copied()
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
        let Some(&run) = table.get(&key.0) else {
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
        match layout_for_config(game_config) {
            Some(layout) => Ok(Some(Self::build(
                g,
                layout,
                game_config.rack_size() as usize + 1,
            )?)),
            None => Ok(None),
        }
    }

    pub fn build<N: kwg::Node>(
        g: &kwg::Kwg<N>,
        layout: alphagram::KeyLayout,
        max_tiles: usize,
    ) -> error::Returns<Self> {
        dawg_words(g)?;
        let mut present =
            std::collections::HashSet::<u128, std::hash::BuildHasherDefault<KeyHasher>>::default();
        for_each_word(g, max_tiles, &mut |w| {
            if let Some(key) = layout.key_of(w) {
                present.insert(key);
            }
        });
        Ok(Self { layout, present })
    }

    #[inline(always)]
    pub fn layout(&self) -> &alphagram::KeyLayout {
        &self.layout
    }

    #[inline(always)]
    pub fn contains(&self, key: u128) -> bool {
        self.present.contains(&key)
    }

    // about the bytes it holds
    #[inline(always)]
    fn bytes(&self) -> usize {
        hashed_bytes::<u128>(self.present.capacity())
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
        Ok(if lean {
            RackLengths::build_for_config(kwg, game_config)?.map_or(Self::None, Self::RackLengths)
        } else {
            Anagrams::build_for_config(kwg, game_config)?.map_or(Self::None, Self::Anagrams)
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

    // about the bytes the tables hold
    #[inline(always)]
    pub fn bytes(&self) -> usize {
        match self {
            Self::Anagrams(anagrams) => anagrams.bytes(),
            Self::RackLengths(rack_lengths) => rack_lengths.bytes(),
            Self::None => 0,
        }
    }
}

// the words in g's dawg, by a count that refuses a graph that loops
#[inline(always)]
fn dawg_words<N: kwg::Node>(g: &kwg::Kwg<N>) -> error::Returns<usize> {
    let counts = g.count_dawg_words_alloc()?;
    Ok(match g[0].arc_index() {
        root if root > 0 => counts[root as usize] as usize,
        _ => 0,
    })
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
        if p <= 0 || w.len() >= max_len {
            return;
        }
        loop {
            let node = g[p];
            w.push(node.tile());
            if node.accepts() {
                f(w);
            }
            walk(g, node.arc_index(), max_len, w, f);
            w.pop();
            if node.is_end() {
                break;
            }
            p += 1;
        }
    }
    walk(g, g[0].arc_index(), max_len, &mut Vec::new(), f);
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
        layout_for_config(&crate::game_config::make_english_game_config()).unwrap()
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
        Anagrams::build(&graph_of(words), test_layout())
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
        let sixteen = vec![1u8; 16];
        let words = [vec![1u8, 2], sixteen.clone()];
        let held = built(&words);
        assert!(layout.key_of(&sixteen).is_none());
        let key = alphagram::Fitted(layout.key_of(&[1, 2]).unwrap());
        assert_eq!(sorted(held.words(key, 2)), vec![vec![1u8, 2]]);
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
