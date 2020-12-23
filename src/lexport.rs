use super::{alphabet, build, kwg};

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
    let mut letter_sets = std::collections::HashMap::<u64, u32, build::MyHasherDefault>::default();
    let mut nodes = Vec::new();
    let mut node_indexes = vec![0u32; kwg.0.len()];

    struct Env<'a> {
        kwg: &'a kwg::Kwg,
        node_indexes: &'a mut [u32],
        nodes: &'a mut Vec<u32>,
        letter_sets: &'a mut std::collections::HashMap<u64, u32, build::MyHasherDefault>,
    }
    let mut env = Env {
        kwg,
        nodes: &mut nodes,
        node_indexes: &mut node_indexes,
        letter_sets: &mut letter_sets,
    };

    fn iter(env: &mut Env, mut p: i32) -> u32 {
        let mut w = env.node_indexes[p as usize];
        // The first node is at index 0, but the structure is acyclic.
        if w != 0 {
            return w;
        }
        w = env.nodes.len() as u32;
        env.node_indexes[p as usize] = w;
        let mut letter_set_bitset = 0u64;
        let orig_p = p;
        env.nodes.push(0);
        loop {
            if env.kwg[p].accepts() {
                letter_set_bitset |= 1 << env.kwg[p].tile();
            }
            if env.kwg[p].arc_index() != 0 {
                env.nodes.push(0); // reserve the space first
            }
            if env.kwg[p].is_end() {
                break;
            }
            p += 1;
        }
        letter_set_bitset >>= 1; // hide the 00 bit
        let mut letter_set_index = env.letter_sets.len() as u32;
        use std::collections::hash_map::Entry::{Occupied, Vacant};
        match env.letter_sets.entry(letter_set_bitset) {
            Occupied(entry) => {
                letter_set_index = *entry.get();
            }
            Vacant(entry) => {
                entry.insert(letter_set_index);
            }
        };
        let orig_w = w;
        w += 1;
        env.nodes[orig_w as usize] =
            ((env.nodes.len() as u32 - w) << 24) | (letter_set_index & 0xffffff);
        let mut gaddag_arc_index = 0;
        p = orig_p;
        loop {
            let arc_index = env.kwg[p].arc_index();
            if arc_index != 0 {
                let tile = env.kwg[p].tile();
                if tile != 0 {
                    // convert 1-based to 0-based tile
                    env.nodes[w as usize] =
                        (((tile - 1) as u32) << 24) | (iter(env, arc_index) & 0xffffff);
                    w += 1;
                } else {
                    gaddag_arc_index = arc_index;
                }
            }
            if env.kwg[p].is_end() {
                break;
            }
            p += 1;
        }
        if gaddag_arc_index != 0 {
            // do the marker last because of Reasons
            env.nodes[w as usize] =
                ((0x32 as u32) << 24) | (iter(env, gaddag_arc_index) & 0xffffff);
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

    drop(env);

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
        // This format only supports one codepoint per alphabet.
        let mut char_iter = alphabet.from_rack(tile).unwrap().chars();
        bin[w..w + 4].copy_from_slice(&(char_iter.next().unwrap() as u32).to_be_bytes());
        w += 4;
        assert!(char_iter.next().is_none(), "tile has multiple codepoints");
    }

    bin[w..w + 4].copy_from_slice(&(letter_sets.len() as u32).to_be_bytes());
    w += 4;
    for (&letter_set_bitset, &letter_set_index) in &letter_sets {
        let z = w + (letter_set_index as usize) * 8;
        bin[z..z + 8].copy_from_slice(&(letter_set_bitset as u64).to_be_bytes());
    }
    w += letter_sets.len() * 8;

    bin[w..w + 4].copy_from_slice(&(nodes.len() as u32).to_be_bytes());
    w += 4;
    for node in nodes {
        bin[w..w + 4].copy_from_slice(&(node as u32).to_be_bytes());
        w += 4;
    }

    bin.into_boxed_slice()
}
