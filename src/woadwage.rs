// Copyright (C) 2020-2026 Andy Kurnia.

use crate::kwg::{Kwg, Node};

pub struct Step {
    first: i32,
    second: Vec<i32>,
}

impl Default for Step {
    #[inline(always)]
    fn default() -> Self {
        Self::new()
    }
}

impl Step {
    #[inline(always)]
    pub fn new() -> Self {
        Self {
            first: 0,
            second: Vec::new(),
        }
    }

    #[inline(always)]
    pub fn step<N: Node>(&self, graph: &Kwg<N>, tile: u8) -> Option<Self> {
        let mut next = Self {
            first: -1,
            second: Vec::new(),
        };
        let mut keep = |p: i32| {
            if p > 0 && !next.second.contains(&p) {
                next.second.push(p);
            }
        };
        for &p in &self.second {
            keep(graph.seek(p, tile));
        }
        if self.first > 0 && graph.node_in_run(self.first).accepts() {
            keep(graph.seek(0, tile));
        }
        if self.first >= 0 {
            next.first = graph.seek(self.first, tile);
        }
        (next.first > 0 || !next.second.is_empty()).then_some(next)
    }

    #[inline(always)]
    pub fn accepts<N: Node>(&self, graph: &Kwg<N>) -> bool {
        (self.first > 0 && graph.node_in_run(self.first).accepts())
            || self.second.iter().any(|&p| graph.node_in_run(p).accepts())
    }
}

#[derive(Clone)]
pub struct StackedStep {
    first: i32,
    lo: usize,
}
impl StackedStep {
    #[inline(always)]
    pub fn new() -> Self {
        Self { first: 0, lo: 0 }
    }
    #[inline(always)]
    pub fn step<N: Node>(&self, graph: &Kwg<N>, stack: &mut Vec<i32>, tile: u8) -> Option<Self> {
        let hi = stack.len();
        let lo = self.lo;
        let keep = |stack: &mut Vec<i32>, p: i32| {
            if p > 0 && !stack[hi..].contains(&p) {
                stack.push(p);
            }
        };
        for i in lo..hi {
            let p = graph.seek(stack[i], tile);
            keep(stack, p);
        }
        if self.first > 0 && graph.node_in_run(self.first).accepts() {
            keep(stack, graph.seek(0, tile));
        }
        let first = if self.first >= 0 {
            graph.seek(self.first, tile)
        } else {
            -1
        };
        if first > 0 || stack.len() > hi {
            Some(Self { first, lo: hi })
        } else {
            None
        }
    }
    #[inline(always)]
    pub fn accepts<N: Node>(&self, graph: &Kwg<N>, stack: &[i32]) -> bool {
        (self.first > 0 && graph.node_in_run(self.first).accepts())
            || stack[self.lo..]
                .iter()
                .any(|&p| graph.node_in_run(p).accepts())
    }
}
impl Default for StackedStep {
    #[inline(always)]
    fn default() -> Self {
        Self::new()
    }
}

// accept a dictionary word or two joined words from the dawg root
pub fn accepts<N: Node>(graph: &Kwg<N>, word: &[u8]) -> bool {
    if graph.0.is_empty() || word.contains(&0) {
        return false;
    }
    let from_root = |tiles: &[u8]| {
        let mut p = 0;
        for &tile in tiles {
            p = graph.seek(p, tile);
            if p <= 0 {
                return false;
            }
        }
        graph.node_in_run(p).accepts()
    };
    let mut p = 0;
    for (k, &tile) in word.iter().enumerate() {
        if k > 0 && graph.node_in_run(p).accepts() && from_root(&word[k..]) {
            return true;
        }
        p = graph.seek(p, tile);
        if p <= 0 {
            return false;
        }
    }
    graph.node_in_run(p).accepts()
}

struct AnagramFrame {
    next: i32,
    restart: i32,
    first_word: bool,
    spent: Option<u8>,
}

// find distinct full-rack spellings in tile order, with tile zero as a blank
pub fn anagrams<N: Node>(graph: &Kwg<N>, rack: &[u8]) -> Vec<Vec<u8>> {
    let Some(root) = graph.0.first() else {
        return Vec::new();
    };
    if rack.is_empty() {
        return Vec::new();
    }
    let root = root.arc_index();
    let mut counts = [0usize; u8::MAX as usize + 1];
    for &tile in rack {
        counts[tile as usize] += 1;
    }

    let mut spellings = std::collections::BTreeSet::<Vec<u8>>::new();
    let mut spelling = Vec::new();
    let mut stack = vec![AnagramFrame {
        next: root,
        restart: 0,
        first_word: true,
        spent: None,
    }];
    while let Some(frame) = stack.last_mut() {
        if frame.next <= 0 && frame.restart > 0 {
            frame.next = std::mem::take(&mut frame.restart);
            frame.first_word = false;
        }
        if frame.next <= 0 {
            if let Some(spent) = frame.spent {
                counts[spent as usize] += 1;
                spelling.pop();
            }
            stack.pop();
            continue;
        }
        let node = graph.node_in_run(frame.next);
        frame.next = if node.is_end() { 0 } else { frame.next + 1 };
        let tile = node.tile();
        let spent = if counts[tile as usize] > 0 { tile } else { 0 };
        if tile == 0 || counts[spent as usize] == 0 {
            continue;
        }

        spelling.push(tile);
        if spelling.len() == rack.len() {
            if node.accepts() && !spellings.contains(spelling.as_slice()) {
                spellings.insert(spelling.clone());
            }
            spelling.pop();
        } else {
            counts[spent as usize] -= 1;
            let first_word = frame.first_word;
            stack.push(AnagramFrame {
                next: node.arc_index(),
                restart: if first_word && node.accepts() {
                    root
                } else {
                    0
                },
                first_word,
                spent: Some(spent),
            });
        }
    }
    spellings.into_iter().collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{bites, build, kwg};
    use std::collections::BTreeSet;

    fn graph_for(words: &[Vec<u8>]) -> Kwg<kwg::Node24> {
        let mut words = words.to_vec();
        words.sort_unstable();
        let words = words
            .iter()
            .map(|word| bites::Bites::from(word.as_slice()))
            .collect::<Vec<_>>();
        let bytes = build::build(
            build::BuildContent::Gaddawg,
            build::BuildLayout::Wolges,
            build::BuildOrder::Sorted,
            &words,
        )
        .unwrap();
        kwg::kbwg_from_bytes_alloc(bytes.to_vec(), false).unwrap()
    }

    fn explicit_variant(words: &[Vec<u8>]) -> BTreeSet<Vec<u8>> {
        words
            .iter()
            .cloned()
            .chain(
                words
                    .iter()
                    .flat_map(|a| words.iter().map(|b| [a.as_slice(), b.as_slice()].concat())),
            )
            .collect()
    }

    fn tile_strings(tiles: &[u8], max_len: usize) -> Vec<Vec<u8>> {
        let mut all = vec![Vec::new()];
        let mut current = 0;
        for _ in 0..max_len {
            let end = all.len();
            for i in current..end {
                for &tile in tiles {
                    let mut next = all[i].clone();
                    next.push(tile);
                    all.push(next);
                }
            }
            current = end;
        }
        all
    }

    #[test]
    fn checks_words_and_all_possible_splits() {
        let words = vec![vec![1], vec![1, 1], vec![1, 2], vec![2, 3], vec![3, 2, 1]];
        let graph = graph_for(&words);
        let explicit = explicit_variant(&words);
        for word in tile_strings(&[1, 2, 3, 4], 6) {
            assert_eq!(accepts(&graph, &word), explicit.contains(&word), "{word:?}");
        }
        assert!(!accepts(&graph, &[1, 0, 2]));
        let long = graph_for(&[vec![1; 150], vec![2; 100]]);
        assert!(accepts(&long, &[[1; 150], [2; 150]].concat()[..250]));
        assert!(!accepts(&long, &[[1; 150], [2; 150]].concat()[..251]));
    }

    #[test]
    fn a_step_at_a_time_is_a_word_or_two_joined_words() {
        let words = vec![vec![1], vec![1, 1], vec![1, 2], vec![2, 3], vec![3, 2, 1]];
        let graph = graph_for(&words);
        let explicit = explicit_variant(&words);
        for word in tile_strings(&[1, 2, 3, 4], 6) {
            let mut state = Some(Step::new());
            for &tile in &word {
                state = state.and_then(|state| state.step(&graph, tile));
            }
            let stepped = !word.is_empty() && state.is_some_and(|state| state.accepts(&graph));
            assert_eq!(stepped, explicit.contains(&word), "{word:?}");
        }
        assert!(accepts(&graph, &[1, 2, 1, 2]));
        assert!(accepts(&graph, &[1, 1, 1]));
        assert!(!accepts(&graph, &[3, 2, 1, 1, 2, 3]));
    }

    #[test]
    fn finds_distinct_full_rack_spellings_with_or_without_blanks() {
        let words = vec![vec![1], vec![1, 1], vec![1, 2], vec![2, 3], vec![3, 2, 1]];
        let graph = graph_for(&words);
        let explicit = explicit_variant(&words);
        for mut rack in tile_strings(&[0, 1, 2, 3, 4], 4) {
            rack.sort_unstable();
            let expected = explicit
                .iter()
                .filter(|word| {
                    if word.len() != rack.len() {
                        return false;
                    }
                    let mut letters = word.to_vec();
                    for &tile in rack.iter().filter(|&&tile| tile != 0) {
                        let Some(index) = letters.iter().position(|&letter| letter == tile) else {
                            return false;
                        };
                        letters.swap_remove(index);
                    }
                    true
                })
                .cloned()
                .collect::<Vec<_>>();
            assert_eq!(anagrams(&graph, &rack), expected, "{rack:?}");
        }
    }
}
