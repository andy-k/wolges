// Copyright (C) 2020-2023 Andy Kurnia.

use super::{alphabet, fash, kwg};

pub enum MacondoFormat {
    Dawg,
    Gaddag,
}

// Macondo project is at https://github.com/domino14/macondo/.
// This function converts a KWG into a Macondo-compatible gaddag or dawg file.
pub fn to_macondo<'a>(
    kwg: &'a kwg::Kwg,
    alphabet: &'a alphabet::Alphabet<'a>,
    lexicon_name: &'a str,
    build_format: MacondoFormat,
) -> Box<[u8]> {
    let alphabet_len_excluding_blank = alphabet.len() - 1;
    assert!(
        alphabet_len_excluding_blank < 0x32,
        "too many letters for gaddag"
    );
    let mut letter_sets = Default::default();
    let mut nodes = Vec::new();
    let mut node_indexes = vec![0u32; kwg.0.len()];

    // Macondo renumbers tiles by unicode order, and inserts the gaddag marker where '^' would be.
    let mut unicode_sorted_tiles = (0..alphabet.len())
        .map(|tile| {
            (
                tile,
                if tile == 0 {
                    "^"
                } else {
                    alphabet.of_rack(tile).unwrap()
                },
            )
        })
        .collect::<Box<_>>();
    unicode_sorted_tiles.sort_unstable_by(|(a_tile, a_label), (b_tile, b_label)| {
        a_label.cmp(b_label).then_with(|| a_tile.cmp(b_tile))
    });
    let mut tile_mapping = vec![0u8; unicode_sorted_tiles.len()];
    {
        let mut mapped_tile = 0u8;
        for &(tile, _) in unicode_sorted_tiles.iter() {
            tile_mapping[tile as usize] = mapped_tile;
            mapped_tile += (tile != 0) as u8; // branchless
        }
        tile_mapping[0] = 0x32; // Macondo represents gaddag separator as 0x32.
    }

    struct Env<'a> {
        kwg: &'a kwg::Kwg,
        unicode_sorted_tiles: &'a [(u8, &'a str)],
        tile_mapping: &'a [u8],
        node_indexes: &'a mut [u32],
        nodes: &'a mut Vec<u32>,
        letter_sets: &'a mut fash::MyHashMap<u64, u32>,
    }
    let mut env = Env {
        kwg,
        unicode_sorted_tiles: &unicode_sorted_tiles,
        tile_mapping: &tile_mapping,
        nodes: &mut nodes,
        node_indexes: &mut node_indexes,
        letter_sets: &mut letter_sets,
    };

    fn iter(env: &mut Env<'_>, mut p: i32) -> u32 {
        let mut w = env.node_indexes[p as usize];
        // The first node is at index 0, but the structure is acyclic.
        if w != 0 {
            return w;
        }
        w = env.nodes.len() as u32;
        env.node_indexes[p as usize] = w;
        let mut letter_set_bitset = 0u64;
        let mut arc_set_bitset = 0u64;
        let orig_p = p;
        env.nodes.push(0);
        loop {
            let node = env.kwg[p];
            let tile = node.tile();
            // Remap later after dedup.
            letter_set_bitset |= (node.accepts() as u64) << tile;
            if node.arc_index() != 0 {
                arc_set_bitset |= 1 << tile;
                env.nodes.push(0); // reserve the space first
            }
            if node.is_end() {
                break;
            }
            p += 1;
        }
        letter_set_bitset &= !1; // disregard the 00 bit
        let mut letter_set_index = env.letter_sets.len() as u32;
        letter_set_index = *env
            .letter_sets
            .entry(letter_set_bitset)
            .or_insert(letter_set_index);
        let orig_w = w;
        w += 1;
        env.nodes[orig_w as usize] =
            ((env.nodes.len() as u32 - w) << 24) | (letter_set_index & 0xffffff);
        // Iteration must follow remapped order.
        for &(tile, _) in env.unicode_sorted_tiles {
            if arc_set_bitset & (1 << tile) != 0 {
                // Arc exists, so do unguarded linear search for that tile.
                p = orig_p;
                while env.kwg[p].tile() != tile {
                    p += 1;
                }
                env.nodes[w as usize] = ((env.tile_mapping[tile as usize] as u32) << 24)
                    | (iter(env, env.kwg[p].arc_index()) & 0xffffff);
                w += 1;
            }
        }
        orig_w
    }

    iter(
        &mut env,
        kwg[match build_format {
            MacondoFormat::Dawg => 0,
            MacondoFormat::Gaddag => 1,
        }]
        .arc_index(),
    );

    let mut bin = vec![
        0u8;
        4 + (1 + lexicon_name.len())
            + (4 + (alphabet_len_excluding_blank as usize) * 4)
            + (4 + letter_sets.len() * 8)
            + (4 + nodes.len() * 4)
    ];
    let mut w = 0;
    bin[w..w + 4].copy_from_slice(match build_format {
        MacondoFormat::Dawg => b"cdwg",
        MacondoFormat::Gaddag => b"cgdg",
    });
    w += 4;

    bin[w] = lexicon_name.len() as u8;
    assert_eq!(bin[w] as usize, lexicon_name.len(), "lexicon name too long");
    w += 1;
    bin[w..w + lexicon_name.len()].copy_from_slice(lexicon_name.as_bytes());
    w += lexicon_name.len();

    bin[w..w + 4].copy_from_slice(&(alphabet_len_excluding_blank as u32).to_be_bytes());
    w += 4;
    for tile in 1..alphabet.len() {
        let z = w + tile_mapping[tile as usize] as usize * 4;
        // This format only supports one codepoint per alphabet.
        let mut char_iter = alphabet.of_rack(tile).unwrap().chars();
        bin[z..z + 4].copy_from_slice(&(char_iter.next().unwrap() as u32).to_be_bytes());
        assert!(char_iter.next().is_none(), "tile has multiple codepoints");
    }
    w += alphabet_len_excluding_blank as usize * 4;

    bin[w..w + 4].copy_from_slice(&(letter_sets.len() as u32).to_be_bytes());
    w += 4;
    for (&letter_set_bitset, &letter_set_index) in &letter_sets {
        let z = w + (letter_set_index as usize) * 8;
        let mut remapped_letter_set_bitset = 0u64;
        for tile in 1..alphabet.len() {
            remapped_letter_set_bitset |=
                ((letter_set_bitset & (1 << tile) != 0) as u64) << tile_mapping[tile as usize];
        }
        bin[z..z + 8].copy_from_slice(&remapped_letter_set_bitset.to_be_bytes());
    }
    w += letter_sets.len() * 8;

    bin[w..w + 4].copy_from_slice(&(nodes.len() as u32).to_be_bytes());
    w += 4;
    for node in nodes {
        bin[w..w + 4].copy_from_slice(&node.to_be_bytes());
        w += 4;
    }

    bin.into_boxed_slice()
}
