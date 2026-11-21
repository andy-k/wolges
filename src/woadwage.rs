// Copyright (C) 2020-2026 Andy Kurnia.

use crate::kwg::{Kwg, Node};

// accept a dictionary word or two joined words from the dawg root
pub fn accepts<N: Node>(graph: &Kwg<N>, word: &[u8]) -> bool {
    if graph.0.is_empty() || word.contains(&0) {
        return false;
    }
    let mut first = 0;
    for (i, &tile) in word.iter().enumerate() {
        first = graph.seek(first, tile);
        if first <= 0 {
            return false;
        }
        if graph[first].accepts() {
            let rest = &word[i + 1..];
            if rest.is_empty() {
                return true;
            }
            let mut second = 0;
            for &tile in rest {
                second = graph.seek(second, tile);
                if second <= 0 {
                    break;
                }
            }
            if second > 0 && graph[second].accepts() {
                return true;
            }
        }
    }
    false
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
    }
}
