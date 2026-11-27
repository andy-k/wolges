// Copyright (C) 2020-2026 Andy Kurnia.

use crate::census::MAX_LETTERS;
use crate::kwg::{Kwg, Node};

pub fn accepts<N: Node>(kad: &Kwg<N>, word: &[u8]) -> bool {
    if kad.0.is_empty()
        || word.is_empty()
        || word
            .iter()
            .any(|&tile| tile == 0 || tile as usize >= MAX_LETTERS)
    {
        return false;
    }
    let len = *word.iter().max().unwrap() as usize + 1;
    let mut suffix = [0u8; MAX_LETTERS];
    for &tile in word {
        suffix[tile as usize] += 1;
    }
    if kad.accepts_alpha_of_len(&suffix[..len], len as u8) {
        return true;
    }
    let mut prefix = [0u8; MAX_LETTERS];
    for &tile in &word[..word.len() - 1] {
        prefix[tile as usize] += 1;
        suffix[tile as usize] -= 1;
        if kad.accepts_alpha_of_len(&prefix[..len], len as u8)
            && kad.accepts_alpha_of_len(&suffix[..len], len as u8)
        {
            return true;
        }
    }
    false
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::{bites, build, kwg};
    use std::collections::BTreeSet;

    const A: u8 = 1;
    const I: u8 = 2;
    const N: u8 = 3;
    const T: u8 = 4;

    pub(crate) fn kad_for(words: &[Vec<u8>]) -> Kwg<kwg::Node24> {
        let mut words = words
            .iter()
            .map(|word| {
                let mut word = word.clone();
                word.sort_unstable();
                word
            })
            .collect::<Vec<_>>();
        words.sort_unstable();
        words.dedup();
        let words = words
            .iter()
            .map(|word| bites::Bites::from(word.as_slice()))
            .collect::<Vec<_>>();
        let bytes = build::build(
            build::BuildContent::DawgOnly,
            build::BuildLayout::Wolges,
            build::BuildOrder::Sorted,
            &words,
        )
        .unwrap();
        kwg::kbwg_from_bytes_alloc(bytes.to_vec(), false).unwrap()
    }

    fn permutations(word: &[u8]) -> BTreeSet<Vec<u8>> {
        if word.is_empty() {
            return [Vec::new()].into_iter().collect();
        }
        let mut all = BTreeSet::new();
        for i in 0..word.len() {
            let rest = [&word[..i], &word[i + 1..]].concat();
            for tail in permutations(&rest) {
                all.insert([&[word[i]][..], &tail[..]].concat());
            }
        }
        all
    }

    pub(crate) fn explicit_variant(words: &[Vec<u8>]) -> BTreeSet<Vec<u8>> {
        let jumbled = words
            .iter()
            .flat_map(|word| permutations(word))
            .collect::<BTreeSet<_>>();
        let joined = jumbled
            .iter()
            .flat_map(|a| jumbled.iter().map(move |b| [&a[..], &b[..]].concat()))
            .collect::<BTreeSet<_>>();
        jumbled.into_iter().chain(joined).collect()
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

    fn l1() -> Vec<Vec<u8>> {
        vec![vec![T, A], vec![I, N], vec![T, A, I, N]]
    }

    #[test]
    fn aint_is_a_jumble_of_tain() {
        let kad = kad_for(&l1());
        assert!(accepts(&kad, &[A, I, N, T]));
        for word in [[N, I, T, A], [T, A, I, N], [A, T, I, N], [A, T, N, I]] {
            assert!(accepts(&kad, &word), "{word:?}");
        }
        assert!(!accepts(&kad, &[T, A, I, N, T]));
    }

    #[test]
    fn taint_needs_a_one_letter_t() {
        let mut words = l1();
        words.push(vec![T]);
        let kad = kad_for(&words);
        for word in [[T, A, I, N, T], [N, T, A, I, T], [T, T, A, I, N]] {
            assert!(accepts(&kad, &word), "{word:?}");
        }
    }

    #[test]
    fn three_parts_are_not_two() {
        let mut words = l1();
        words.push(vec![T]);
        let kad = kad_for(&words);
        assert!(!accepts(&kad, &[T, A, I, N, T, T]));
    }

    #[test]
    fn tnia_is_not_a_jumble_of_ll() {
        let kad = kad_for(&[vec![T, A], vec![I, N]]);
        assert!(accepts(&kad, &[A, T, I, N]));
        assert!(accepts(&kad, &[N, I, T, A]));
        assert!(accepts(&kad, &[T, A, I, N]));
        assert!(!accepts(&kad, &[T, N, I, A]));
    }

    #[test]
    fn accepts_exactly_the_jumbles_and_their_joined_pairs() {
        let mut l2 = l1();
        l2.push(vec![T]);
        let lexicons = [
            l1(),
            l2,
            vec![vec![T, A], vec![I, N]],
            vec![vec![A, A], vec![A, I], vec![A, I, A]],
        ];
        let letters = [A, I, N, T, T + 1];
        let most = 6u32;
        let mut cases = 0;
        for words in &lexicons {
            let kad = kad_for(words);
            let explicit = explicit_variant(words);
            for word in tile_strings(&letters, most as usize) {
                assert_eq!(accepts(&kad, &word), explicit.contains(&word), "{word:?}");
                cases += 1;
            }
        }
        assert_eq!(
            cases,
            lexicons.len() * (0..=most).map(|len| letters.len().pow(len)).sum::<usize>()
        );
        assert!(!accepts(&kad_for(&l1()), &[T, 0, A]));
    }
}
