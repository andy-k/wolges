use super::error;

struct MyHasher(u64);

impl std::hash::Hasher for MyHasher {
    fn finish(&self) -> u64 {
        self.0
    }
    fn write(&mut self, bytes: &[u8]) {
        for &b in bytes {
            self.0 = (std::num::Wrapping(self.0) * std::num::Wrapping(3467)).0 ^ (!b as u64);
        }
    }
}

impl Default for MyHasher {
    fn default() -> MyHasher {
        MyHasher(0)
    }
}

type MyHasherDefault = std::hash::BuildHasherDefault<MyHasher>;

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
    fn push(&mut self, tile: &u8) {
        self.transitions.push(Transition {
            tile: *tile,
            accepts: false,
            arc_index: 0, // Filled up later.
        });
        self.indexes.push(self.transitions.len());
    }

    fn pop(&mut self, state_maker: &mut StateMaker) {
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
    states_finder: &'a mut std::collections::HashMap<State, u32, MyHasherDefault>,
}

impl StateMaker<'_> {
    fn make_state(&mut self, node_transitions: &[Transition]) -> u32 {
        let mut ret = 0;
        for node_transition in node_transitions.iter().rev() {
            let state = State {
                tile: node_transition.tile,
                accepts: node_transition.accepts,
                arc_index: node_transition.arc_index,
                next_index: ret,
            };
            use std::collections::hash_map::Entry::{Occupied, Vacant};
            match self.states_finder.entry(state) {
                Occupied(entry) => {
                    ret = *entry.get();
                }
                Vacant(entry) => {
                    ret = self.states.len() as u32;
                    self.states.push(entry.key().clone());
                    entry.insert(ret);
                }
            }
        }
        ret
    }

    fn make_dawg(
        &mut self,
        sorted_machine_words: &[Box<[u8]>],
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
                let min_word_len = std::cmp::min(this_word_len, prev_word_len);
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

fn gen_machine_drowwords(machine_words: &[Box<[u8]>]) -> Box<[Box<[u8]>]> {
    let mut machine_drowword_set = std::collections::HashSet::<_, MyHasherDefault>::default();
    let mut reverse_buffer = Vec::new();
    for this_word in machine_words {
        // CARE = ERAC, RAC@, AC@, C@
        reverse_buffer.clear();
        reverse_buffer.extend_from_slice(this_word);
        reverse_buffer.reverse();
        //machine_drowword_set.insert(reverse_buffer[..].to_vec());
        machine_drowword_set.insert(reverse_buffer.clone().into_boxed_slice());
        reverse_buffer.push(0); // the '@'
        for drow_prefix_len in 1..this_word.len() {
            machine_drowword_set.insert(reverse_buffer[drow_prefix_len..].into());
        }
        /*
        reverse_buffer.clear();
        reverse_buffer.extend_from_slice(this_word);
        reverse_buffer.sort();
        machine_drowword_set.insert(reverse_buffer.to_vec());
        let len_minus_one = this_word.len() - 1;
        for which_tile in (0..len_minus_one).rev() {
            let c1 = reverse_buffer[which_tile];
            let c2 = reverse_buffer[len_minus_one];
            if c1 != c2 {
                //reverse_buffer.swap(which_tile, this_word.len() - 1);
                reverse_buffer[which_tile] = c2;
                reverse_buffer[len_minus_one] = c1;
                machine_drowword_set.insert(reverse_buffer.to_vec());
            }
        }
        */
    }
    drop(reverse_buffer);
    let mut machine_drowwords = machine_drowword_set.into_iter().collect::<Box<_>>();
    machine_drowwords.sort();
    machine_drowwords
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
        for ofs in 0..num {
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
        out[2] = ((defragged_arc_index >> 16) & 0x3f
            | if is_end.0 { 0x40 } else { 0 }
            | if accepts.0 { 0x80 } else { 0 }) as u8;
        out[3] = tile;
    }

    fn to_vec(&self, dawg_start_state: u32, gaddag_start_state: u32) -> Vec<u8> {
        let mut ret = vec![0; (self.num_written as usize) << 2];
        self.write_node(
            &mut ret[0..],
            dawg_start_state,
            IsEnd(true),
            Accepts(false),
            0,
        );
        self.write_node(
            &mut ret[4..],
            gaddag_start_state,
            IsEnd(true),
            Accepts(false),
            0,
        );
        for mut p in 1..self.states.len() {
            if self.prev_indexes[p] != 0 {
                continue;
            }
            let mut dp = self.destination[p] as usize;
            if dp == 0 {
                continue;
            }
            dp <<= 2;
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

pub fn build(build_format: BuildFormat, machine_words: &[Box<[u8]>]) -> error::Returns<Vec<u8>> {
    // The sink state always exists.
    let mut states = Vec::new();
    states.push(State {
        tile: 0,
        accepts: false,
        arc_index: 0,
        next_index: 0,
    });

    let mut states_finder = std::collections::HashMap::<_, _, MyHasherDefault>::default();
    states_finder.insert(states[0].clone(), 0);

    let mut state_maker = StateMaker {
        states: &mut states,
        states_finder: &mut states_finder,
    };
    let dawg_start_state = state_maker.make_dawg(machine_words, 0, false);
    //let mut dawg_start_state = 0;
    let gaddag_start_state = match build_format {
      BuildFormat::DawgOnly =>
        0,
      BuildFormat::Gaddawg =>
        state_maker.make_dawg(
            &gen_machine_drowwords(machine_words),
            dawg_start_state,
            true,
        ),
    };
    //dawg_start_state = gaddag_start_state;

    let mut states_defragger = StatesDefragger {
        states: &states,
        prev_indexes: &gen_prev_indexes(&states),
        destination: &mut vec![0u32; states.len()],
        num_written: 2, // Convention: [0] points to dawg, [1] to gaddag.
    };
    states_defragger.destination[0] = !0; // useful for empty lexicon
    states_defragger.defrag(dawg_start_state);
    states_defragger.defrag(gaddag_start_state);
    states_defragger.destination[0] = 0; // useful for empty lexicon

    if states_defragger.num_written > 0x400000 {
        // the format can only have 0x400000 elements, each has 4 bytes
        return_error!(format!(
            "this format cannot have {} nodes",
            states_defragger.num_written
        ));
    }

    Ok(states_defragger.to_vec(dawg_start_state, gaddag_start_state))
}
