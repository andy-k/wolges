// Copyright (C) 2020-2021 Andy Kurnia.

use super::{bites, error, fash};

// Unconfirmed entries.
// Memory wastage notes:
// - Arc index would be 22 bytes max.
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
    fn push(&mut self, tile: &u8) {
        self.transitions.push(Transition {
            tile: *tile,
            accepts: false,
            arc_index: 0, // Filled up later.
        });
        self.indexes.push(self.transitions.len());
    }

    #[inline(always)]
    fn pop(&mut self, state_maker: &mut StateMaker<'_>) {
        let start_of_batch = self.indexes.pop().unwrap();
        let new_arc_index = state_maker.make_state(&self.transitions[start_of_batch..]);
        self.transitions[start_of_batch - 1].arc_index = new_arc_index;
        self.transitions.truncate(start_of_batch);
    }
}

// Deduplicated entries.
// Memory wastage notes:
// - Each index would be 22 bytes max.
// - Could have used u64 or a 7-byte thing instead of this 12-byte struct.
#[derive(Clone, Eq, Hash, PartialEq)]
struct State {
    tile: u8,
    accepts: bool,
    arc_index: u32,  // Refers to states.
    next_index: u32, // Refers to states.
}

struct StateMaker<'a> {
    states: &'a mut Vec<State>,
    states_finder: &'a mut fash::MyHashMap<State, u32>,
}

impl StateMaker<'_> {
    #[inline(always)]
    fn make_state(&mut self, node_transitions: &[Transition]) -> u32 {
        let mut ret = 0;
        for node_transition in node_transitions.iter().rev() {
            let state = State {
                tile: node_transition.tile,
                accepts: node_transition.accepts,
                arc_index: node_transition.arc_index,
                next_index: ret,
            };
            let new_ret = self.states.len() as u32;
            ret = *self.states_finder.entry(state.clone()).or_insert(new_ret);
            if ret == new_ret {
                self.states.push(state);
            }
        }
        ret
    }

    #[inline(always)]
    fn make_dawg(
        &mut self,
        sorted_machine_words: &[bites::Bites],
        dawg_start_state: u32,
        is_gaddag_phase: bool,
    ) -> u32 {
        let mut transition_stack = TransitionStack {
            transitions: &mut Vec::new(),
            indexes: &mut Vec::new(),
        };
        for machine_word_index in 0..sorted_machine_words.len() {
            let this_word = &sorted_machine_words[machine_word_index];
            let this_word_len = this_word.len();
            let mut prefix_len = 0;
            if machine_word_index > 0 {
                let prev_word = &sorted_machine_words[machine_word_index - 1];
                let prev_word_len = transition_stack.indexes.len(); // this can be one less than prev_word.len() for gaddag
                let min_word_len = this_word_len.min(prev_word_len);
                while prefix_len < min_word_len && prev_word[prefix_len] == this_word[prefix_len] {
                    prefix_len += 1;
                }
                for _ in prefix_len..prev_word_len {
                    transition_stack.pop(self);
                }
            }
            for tile in &this_word[prefix_len..this_word_len] {
                transition_stack.push(tile);
            }
            let transitions_len = transition_stack.transitions.len();
            if is_gaddag_phase && this_word[this_word_len - 1] == 0 {
                transition_stack.indexes.pop().unwrap();
                // gaddag["AC@"] points to dawg["CA"]
                let mut p = dawg_start_state;
                for &sought_tile in this_word[0..this_word_len - 1].iter().rev() {
                    loop {
                        if self.states[p as usize].tile == sought_tile {
                            p = self.states[p as usize].arc_index;
                            break;
                        }
                        p = self.states[p as usize].next_index;
                    }
                }
                transition_stack.transitions[transitions_len - 1].arc_index = p;
            } else {
                transition_stack.transitions[transitions_len - 1].accepts = true;
            }
        }
        for _ in 0..transition_stack.indexes.len() {
            transition_stack.pop(self);
        }
        self.make_state(&transition_stack.transitions[..])
    }
}

fn gen_machine_drowwords(machine_words: &[bites::Bites]) -> Box<[bites::Bites]> {
    let mut machine_drowword_set = fash::MyHashSet::default();
    let mut reverse_buffer = Vec::new();
    for this_word in machine_words {
        // CARE = ERAC, RAC@, AC@, C@
        reverse_buffer.clear();
        reverse_buffer.extend_from_slice(this_word);
        reverse_buffer.reverse();
        machine_drowword_set.insert(reverse_buffer[..].into());
        reverse_buffer.push(0); // the '@'
        for drow_prefix_len in 1..this_word.len() {
            machine_drowword_set.insert(reverse_buffer[drow_prefix_len..].into());
        }
    }
    drop(reverse_buffer);
    let mut machine_drowwords = machine_drowword_set.into_iter().collect::<Box<_>>();
    machine_drowwords.sort_unstable();
    machine_drowwords
}

// AlphaDawg is DawgOnly on make_alphagrams(machine_words).
pub fn make_alphagrams(machine_words: &[bites::Bites]) -> Box<[bites::Bites]> {
    let mut machine_dorws_set = fash::MyHashSet::default();
    let mut rearrange_buffer = Vec::new();
    for this_word in machine_words {
        rearrange_buffer.clear();
        rearrange_buffer.extend_from_slice(this_word);
        rearrange_buffer.sort_unstable();
        machine_dorws_set.insert(rearrange_buffer[..].into());
    }
    drop(rearrange_buffer);
    let mut machine_dorws = machine_dorws_set.into_iter().collect::<Box<_>>();
    machine_dorws.sort_unstable();
    machine_dorws
}

// zero-cost type-safety
struct IsEnd(bool);
struct Accepts(bool);

struct StatesDefragger<'a> {
    states: &'a [State],
    prev_indexes: &'a [u32],
    destination: &'a mut Vec<u32>,
    num_written: u32,
}

impl StatesDefragger<'_> {
    fn defrag(&mut self, mut p: u32) {
        loop {
            let prev = self.prev_indexes[p as usize];
            if prev == 0 {
                break;
            }
            p = prev;
        }
        if self.destination[p as usize] != 0 {
            return;
        }
        // temp value to break self-cycles.
        self.destination[p as usize] = !0;
        let mut write_p = p;
        let mut num = 0u32;
        loop {
            num += 1;
            let a = self.states[p as usize].arc_index;
            if a != 0 {
                self.defrag(a);
            }
            p = self.states[p as usize].next_index;
            if p == 0 {
                break;
            }
        }
        self.destination[write_p as usize] = 0;
        for ofs in 0..num {
            // prefer earlier index, so dawg part does not point to gaddag part
            if self.destination[write_p as usize] != 0 {
                break;
            }
            self.destination[write_p as usize] = self.num_written + ofs;
            write_p = self.states[write_p as usize].next_index;
        }
        // Always += num even if some nodes are necessarily duplicated due to sharing by different prev_nodes.
        self.num_written += num;
    }

    // encoding: little endian of
    // bits 0-21 = pointer & 0x3fffff
    // bit 22 = end
    // bit 23 = is_terminal
    // bits 24-31 = char
    #[inline(always)]
    fn write_node(
        &self,
        out: &mut [u8],
        arc_index: u32,
        is_end: IsEnd,
        accepts: Accepts,
        tile: u8,
    ) {
        let defragged_arc_index = self.destination[arc_index as usize];
        out[0] = defragged_arc_index as u8;
        out[1] = (defragged_arc_index >> 8) as u8;
        out[2] = (((defragged_arc_index >> 16) & 0x3f) as u8)
            | (0x40 & -(is_end.0 as i8) as u8)
            | (0x80 & -(accepts.0 as i8) as u8);
        out[3] = tile;
    }

    fn to_vec(
        &self,
        build_format: BuildFormat,
        dawg_start_state: u32,
        gaddag_start_state: u32,
    ) -> Vec<u8> {
        let mut ret = vec![0; (self.num_written as usize) * 4];
        self.write_node(
            &mut ret[0..],
            dawg_start_state,
            IsEnd(true),
            Accepts(false),
            0,
        );
        match build_format {
            BuildFormat::DawgOnly => (),
            BuildFormat::Gaddawg => {
                self.write_node(
                    &mut ret[4..],
                    gaddag_start_state,
                    IsEnd(true),
                    Accepts(false),
                    0,
                );
            }
        }
        for mut p in 1..self.states.len() {
            if self.prev_indexes[p] != 0 {
                continue;
            }
            let mut dp = self.destination[p] as usize;
            if dp == 0 {
                continue;
            }
            dp *= 4;
            loop {
                let np = self.states[p].next_index;
                self.write_node(
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
        ret
    }
}

fn gen_prev_indexes(states: &[State]) -> Vec<u32> {
    let states_len = states.len();
    let mut prev_indexes = vec![0u32; states_len];
    for p in (1..states_len).rev() {
        prev_indexes[states[p].next_index as usize] = p as u32;
    }
    // prev_indexes[0] is garbage, does not matter.

    prev_indexes
}

pub enum BuildFormat {
    DawgOnly,
    Gaddawg,
}

pub fn build(
    build_format: BuildFormat,
    machine_words: &[bites::Bites],
) -> error::Returns<bites::Bites> {
    // The sink state always exists.
    let mut states = vec![State {
        tile: 0,
        accepts: false,
        arc_index: 0,
        next_index: 0,
    }];

    let mut states_finder = fash::MyHashMap::default();
    states_finder.insert(states[0].clone(), 0);

    let mut state_maker = StateMaker {
        states: &mut states,
        states_finder: &mut states_finder,
    };
    let dawg_start_state = match build_format {
        BuildFormat::DawgOnly | BuildFormat::Gaddawg => {
            state_maker.make_dawg(machine_words, 0, false)
        }
    };
    let gaddag_start_state = match build_format {
        BuildFormat::DawgOnly => 0,
        BuildFormat::Gaddawg => state_maker.make_dawg(
            &gen_machine_drowwords(machine_words),
            dawg_start_state,
            true,
        ),
    };

    let mut states_defragger = StatesDefragger {
        states: &states,
        prev_indexes: &gen_prev_indexes(&states),
        destination: &mut vec![0u32; states.len()],
        num_written: match build_format {
            BuildFormat::DawgOnly => 1,
            BuildFormat::Gaddawg => 2,
        },
    };
    states_defragger.destination[0] = !0; // useful for empty lexicon
    states_defragger.defrag(dawg_start_state);
    match build_format {
        BuildFormat::DawgOnly => (),
        BuildFormat::Gaddawg => {
            states_defragger.defrag(gaddag_start_state);
        }
    }
    states_defragger.destination[0] = 0; // useful for empty lexicon

    if states_defragger.num_written > 0x400000 {
        // the format can only have 0x400000 elements, each has 4 bytes
        return_error!(format!(
            "this format cannot have {} nodes",
            states_defragger.num_written
        ));
    }

    Ok(states_defragger.to_vec(build_format, dawg_start_state, gaddag_start_state)[..].into())
}
