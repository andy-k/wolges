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
        let node = graph[frame.next];
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
