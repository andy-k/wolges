// Copyright (C) 2020-2025 Andy Kurnia.

use super::{bites, error, fash};

// Unconfirmed entries.
// Memory wastage notes:
// - Arc index would be 22 bits max.
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
// - Each index would be 22 bits max.
// - Could have used u64 or a 7-byte thing instead of this 12-byte struct.
#[derive(Clone, Eq, Hash, PartialEq)]
struct State {
    tile: u8,
    accepts: bool,
    arc_index: u32,  // Refers to states.
    next_index: u32, // Refers to states.
}

// for each i > 0, states[i].arc_index < i and states[i].next_index < i.
// this ensures states is already a topologically sorted DAG.
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
    let mut machine_drowwords = Vec::new();
    let mut reverse_buffer = Vec::new();
    for machine_word_index in 0..machine_words.len() {
        let this_word = &machine_words[machine_word_index];
        let this_word_len = this_word.len();
        let mut prefix_len = 0;
        if machine_word_index > 0 {
            let prev_word = &machine_words[machine_word_index - 1];
            let prev_word_len = prev_word.len();
            // - 1 because CAR -> CARE means we still need to emit RAC@.
            let max_prefix_len = this_word_len.min(prev_word_len - 1);
            while prefix_len < max_prefix_len && prev_word[prefix_len] == this_word[prefix_len] {
                prefix_len += 1;
            }
        }
        // CARE = ERAC, RAC@, AC@, C@
        reverse_buffer.clear();
        reverse_buffer.extend_from_slice(this_word);
        reverse_buffer.reverse();
        machine_drowwords.push(reverse_buffer[..].into());
        let num_prefixes = this_word_len - prefix_len;
        if num_prefixes >= 2 {
            reverse_buffer.push(0); // the '@'
            for drow_prefix_len in 1..num_prefixes {
                machine_drowwords.push(reverse_buffer[drow_prefix_len..].into());
            }
        }
    }
    drop(reverse_buffer);
    machine_drowwords.sort_unstable();
    machine_drowwords.into_boxed_slice()
}

// AlphaDawg is DawgOnly on make_alphagrams(machine_words).
pub fn make_alphagrams(machine_words: &[bites::Bites]) -> Box<[bites::Bites]> {
    let mut machine_dorws = Vec::with_capacity(machine_words.len());
    let mut rearrange_buffer = Vec::new();
    for this_word in machine_words {
        rearrange_buffer.clear();
        rearrange_buffer.extend_from_slice(this_word);
        rearrange_buffer.sort_unstable();
        machine_dorws.push(rearrange_buffer[..].into());
    }
    drop(rearrange_buffer);
    machine_dorws.sort_unstable();
    machine_dorws.dedup();
    machine_dorws.into_boxed_slice()
}

// build formats

pub enum BuildContent {
    DawgOnly,
    Gaddawg,
}

pub enum BuildLayout {
    Legacy,       // tiny, slow to movegen, leaf-first order. used to be the default.
    Magpie, // big, fast to movegen, BFS order, no tail dedup, easy to read. https://github.com/jvc56/MAGPIE/
    MagpieMerged, // tiny, slow to movegen, BFS order.
    Experimental, // small, fast to movegen, frequent-first order, may put dawg behind gaddag.
    Wolges, // small, faster to movegen, dawg-first then frequent-first. recommended default.
}

// zero-cost type-safety
struct IsEnd(bool);
struct Accepts(bool);

// Each block has 16 entries (hardcoded).
// 16 entries of u32 make 64 bytes, which is a common cache line size.
// 0 <= block_len[i] <= 16, from (i << 4) the first block_len[i] are occupied.
// If block_len[i] < 16, blocks_with_len[block_len[i]] stack includes i.
struct StateDefraggerExperimentalParams<'a> {
    block_len: &'a mut Vec<u8>,
    blocks_with_len: &'a mut [Vec<u32>; 16],
}

struct StatesDefragger<'a> {
    states: &'a [State],
    head_indexes: &'a [u32],
    to_end_lens: &'a [u32], // using u8 costs runtime.
    destination: &'a mut Vec<u32>,
    num_written: u32,
}

impl StatesDefragger<'_> {
    fn defrag_legacy(&mut self, mut p: u32) {
        p = self.head_indexes[p as usize];
        if self.destination[p as usize] != 0 {
            return;
        }
        let num = self.to_end_lens[p as usize];
        // temp value to break self-cycles.
        self.destination[p as usize] = !0;
        let mut write_p = p;
        loop {
            let a = self.states[p as usize].arc_index;
            if a != 0 {
                self.defrag_legacy(a);
            }
            p = self.states[p as usize].next_index;
            if p == 0 {
                break;
            }
        }
        let initial_num_written = self.num_written;
        self.destination[write_p as usize] = 0;
        for ofs in 0..num {
            // prefer earlier index, so dawg part does not point to gaddag part
            if self.destination[write_p as usize] != 0 {
                break;
            }
            self.destination[write_p as usize] = initial_num_written + ofs;
            write_p = self.states[write_p as usize].next_index;
        }
        // Always += num even if some nodes are necessarily duplicated due to sharing by different prev_nodes.
        self.num_written += num;
    }

    fn defrag_magpie(&mut self, mut p: u32) {
        if self.destination[p as usize] != 0 {
            return;
        }
        self.destination[p as usize] = self.num_written;
        // non-legacy mode reserves the space first.
        let num = self.to_end_lens[p as usize];
        self.num_written += num;
        loop {
            let a = self.states[p as usize].arc_index;
            if a != 0 {
                self.defrag_magpie(a);
            }
            p = self.states[p as usize].next_index;
            if p == 0 {
                break;
            }
        }
    }

    fn defrag_magpie_merged(&mut self, mut p: u32) {
        p = self.head_indexes[p as usize];
        if self.destination[p as usize] != 0 {
            return;
        }
        let initial_num_written = self.num_written;
        // temp value to break self-cycles.
        self.destination[p as usize] = !0;
        // non-legacy mode reserves the space first.
        let num = self.to_end_lens[p as usize];
        self.num_written += num;
        let mut write_p = p;
        loop {
            let a = self.states[p as usize].arc_index;
            if a != 0 {
                self.defrag_magpie_merged(a);
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
            self.destination[write_p as usize] = initial_num_written + ofs;
            write_p = self.states[write_p as usize].next_index;
        }
        // non-legacy mode already reserves the space.
    }

    fn defrag_cache_friendly(
        &mut self,
        params: &mut StateDefraggerExperimentalParams<'_>,
        mut p: u32,
    ) {
        p = self.head_indexes[p as usize];
        if self.destination[p as usize] != 0 {
            return;
        }
        // temp value to break self-cycles.
        self.destination[p as usize] = !0;
        // non-legacy mode reserves the space first.
        let num = self.to_end_lens[p as usize];
        // choose a cache-friendly page to place these.
        let mut num_blocks = params.block_len.len() as u32;
        let initial_num_written;
        if num > 16 {
            // always even-align for 128 byte cache line machines.
            if num_blocks & 1 == 1 {
                params.blocks_with_len[0].push(num_blocks);
                params.block_len.push(0);
                num_blocks += 1;
            }
            initial_num_written = num_blocks << 4;
            let mut num = num; // shadow the variable
            while num > 16 {
                params.block_len.push(16);
                num -= 16;
            }
            // this can be between 1 to 16.
            if num < 16 {
                params.blocks_with_len[num as usize].push(params.block_len.len() as u32);
            }
            params.block_len.push(num as u8);
        } else {
            // 1 <= num <= 16
            let mut required_gap = 16 - num; // 0 <= required_gap <= 15
            loop {
                // if found, use it
                if let Some(place) = params.blocks_with_len[required_gap as usize].pop() {
                    // use | instead of + because it cannot overflow
                    initial_num_written = (place << 4) | required_gap;
                    // repurpose this variable.
                    required_gap += num; // 1 <= required_gap <= 16
                    if required_gap < 16 {
                        params.blocks_with_len[required_gap as usize].push(place);
                    }
                    params.block_len[place as usize] = required_gap as u8;
                    break;
                }
                // if 0, add new row.
                if required_gap == 0 {
                    initial_num_written = num_blocks << 4;
                    if num < 16 {
                        params.blocks_with_len[num as usize].push(num_blocks);
                    }
                    params.block_len.push(num as u8);
                    break;
                }
                // if not, -1 then try again
                required_gap -= 1;
            }
        }
        let mut write_p = p;
        loop {
            let a = self.states[p as usize].arc_index;
            if a != 0 {
                self.defrag_cache_friendly(params, a);
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
            self.destination[write_p as usize] = initial_num_written + ofs;
            write_p = self.states[write_p as usize].next_index;
        }
        // non-legacy mode already reserves the space.
    }

    fn build_experimental(&mut self, num_ways: &[u32], top_indexes: &[u32]) {
        let mut idxs = Box::from_iter(1..self.states.len() as u32);
        idxs.sort_unstable_by(|&a, &b| {
            num_ways[b as usize]
                .cmp(&num_ways[a as usize])
                .then_with(|| {
                    self.to_end_lens[b as usize]
                        .cmp(&self.to_end_lens[a as usize])
                        .then_with(|| a.cmp(&b))
                })
        });

        let mut params = StateDefraggerExperimentalParams {
            block_len: &mut Vec::new(),
            blocks_with_len: &mut [(); 16].map(|_| Vec::new()),
        };
        // num_written is either 1 or 2, both are < 16.
        params.block_len.push(self.num_written as u8);
        params.blocks_with_len[self.num_written as usize].push(0u32);
        for &p in idxs.iter() {
            self.defrag_cache_friendly(&mut params, top_indexes[p as usize]);
        }
        self.num_written =
            ((params.block_len.len() as u32 - 1) << 4) + *params.block_len.last().unwrap() as u32;
    }

    fn build_wolges(
        &mut self,
        num_ways: &[u32],
        build_content: &BuildContent,
        dawg_start_state: u32,
    ) {
        let mut idxs = Box::from_iter(1..self.states.len() as u32);
        match build_content {
            BuildContent::DawgOnly => {
                // All nodes are dawg nodes.
                idxs.sort_unstable_by(|&a, &b| {
                    num_ways[b as usize]
                        .cmp(&num_ways[a as usize])
                        .then_with(|| {
                            self.to_end_lens[b as usize]
                                .cmp(&self.to_end_lens[a as usize])
                                .then_with(|| a.cmp(&b))
                        })
                });
            }
            BuildContent::Gaddawg => {
                // Check which nodes are used in dawg.
                let mut used_in_dawg = vec![false; self.states.len()];
                used_in_dawg
                    .iter_mut()
                    .take(dawg_start_state as usize + 1)
                    .skip(1)
                    .for_each(|m| *m = true);
                for p in dawg_start_state as usize + 1..self.states.len() {
                    if used_in_dawg[self.states[p].next_index as usize] {
                        used_in_dawg[p] = true
                    }
                }
                idxs.sort_unstable_by(|&a, &b| {
                    used_in_dawg[b as usize]
                        .cmp(&used_in_dawg[a as usize])
                        .then_with(|| {
                            num_ways[b as usize]
                                .cmp(&num_ways[a as usize])
                                .then_with(|| {
                                    self.to_end_lens[b as usize]
                                        .cmp(&self.to_end_lens[a as usize])
                                        .then_with(|| a.cmp(&b))
                                })
                        })
                });
            }
        }

        let mut params = StateDefraggerExperimentalParams {
            block_len: &mut Vec::new(),
            blocks_with_len: &mut [(); 16].map(|_| Vec::new()),
        };
        // num_written is either 1 or 2, both are < 16.
        params.block_len.push(self.num_written as u8);
        params.blocks_with_len[self.num_written as usize].push(0u32);
        for &p in idxs.iter() {
            self.defrag_cache_friendly(&mut params, p);
        }
        self.num_written =
            ((params.block_len.len() as u32 - 1) << 4) + *params.block_len.last().unwrap() as u32;
    }

    // encoding: little endian of
    // bits 0-21 = pointer & 0x3fffff
    // bit 22 = end
    // bit 23 = is_terminal
    // bits 24-31 = char
    #[inline(always)]
    fn write_node<const VARIANT: u8>(
        &self,
        out: &mut [u8],
        arc_index: u32,
        is_end: IsEnd,
        accepts: Accepts,
        tile: u8,
    ) {
        let defragged_arc_index = self.destination[arc_index as usize];
        match VARIANT {
            1 => {
                out[0] = defragged_arc_index as u8;
                out[1] = (defragged_arc_index >> 8) as u8;
                out[2] = (((defragged_arc_index >> 16) & 0x3f) as u8)
                    | ((is_end.0 as u8) << 6)
                    | ((accepts.0 as u8) << 7);
                out[3] = tile;
            }
            2 => {
                out[0] = defragged_arc_index as u8;
                out[1] = (defragged_arc_index >> 8) as u8;
                out[2] = (defragged_arc_index >> 16) as u8;
                out[3] = (tile & 0x3f) | ((is_end.0 as u8) << 6) | ((accepts.0 as u8) << 7);
            }
            _ => unimplemented!(),
        }
    }

    fn to_vec<const VARIANT: u8>(
        &self,
        build_content: BuildContent,
        dawg_start_state: u32,
        gaddag_start_state: u32,
    ) -> Vec<u8> {
        let mut ret = vec![0; (self.num_written as usize) * 4];
        self.write_node::<VARIANT>(
            &mut ret[0..],
            dawg_start_state,
            IsEnd(true),
            Accepts(false),
            0,
        );
        match build_content {
            BuildContent::DawgOnly => {}
            BuildContent::Gaddawg => {
                self.write_node::<VARIANT>(
                    &mut ret[4..],
                    gaddag_start_state,
                    IsEnd(true),
                    Accepts(false),
                    0,
                );
            }
        }
        for mut p in 1..self.states.len() {
            let mut dp = self.destination[p] as usize;
            if dp == 0 {
                continue;
            }
            dp *= 4;
            loop {
                let np = self.states[p].next_index;
                self.write_node::<VARIANT>(
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

fn gen_head_indexes(states: &[State]) -> Vec<u32> {
    let states_len = states.len();
    let mut head_indexes = Vec::from_iter(0..states_len as u32);

    // point to immediate prev.
    for p in (1..states_len).rev() {
        head_indexes[states[p].next_index as usize] = p as u32;
    }
    // head_indexes[0] is garbage, does not matter.

    // adjust to point to prev heads instead.
    for p in (1..states_len).rev() {
        head_indexes[p] = head_indexes[head_indexes[p] as usize];
    }

    head_indexes
}

fn gen_to_end_lens(states: &[State]) -> Vec<u32> {
    let states_len = states.len();
    let mut to_end_lens = vec![1u32; states_len];

    for p in 1..states_len {
        let next = states[p].next_index;
        if next != 0 {
            to_end_lens[p] += to_end_lens[next as usize];
        }
    }

    to_end_lens
}

fn gen_num_ways(
    states: &[State],
    build_content: &BuildContent,
    dawg_start_state: u32,
    gaddag_start_state: u32,
) -> Vec<u32> {
    let states_len = states.len();
    let mut num_ways = vec![0u32; states_len];

    num_ways[dawg_start_state as usize] = 1;
    match build_content {
        BuildContent::DawgOnly => {}
        BuildContent::Gaddawg => {
            num_ways[gaddag_start_state as usize] = 1;
        }
    }
    for p in (1..states_len).rev() {
        let this_num_ways = num_ways[p];
        let state = &states[p];
        for p_dest in [state.next_index, state.arc_index] {
            let v = &mut num_ways[p_dest as usize];
            *v = v.saturating_add(this_num_ways);
        }
    }

    num_ways
}

fn gen_top_indexes(states: &[State], head_indexes: &[u32]) -> Vec<u32> {
    let states_len = states.len();
    let mut top_indexes = vec![0u32; states_len];

    for (p, p_dest) in states
        .iter()
        .map(|x| x.arc_index as usize)
        .enumerate()
        .take(states_len)
        .skip(1)
    {
        top_indexes[p_dest] = p as u32 | -((top_indexes[p_dest] != 0) as i32) as u32;
    }
    // [p] = 0 (no parent), parent_index, or !0 if > 1 parents.
    // if not unique, set [p] = p.
    for (p, top_index) in top_indexes.iter_mut().enumerate().take(states_len) {
        if *top_index == 0 || *top_index == !0 {
            *top_index = p as u32;
        }
    }
    // adjust to point to prev tops's heads instead.
    for p in (1..states_len).rev() {
        top_indexes[p] = head_indexes[top_indexes[top_indexes[p] as usize] as usize];
    }

    top_indexes
}

// machine_words must be sorted and unique.
fn do_build<const VARIANT: u8>(
    build_content: BuildContent,
    build_layout: BuildLayout,
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
    let dawg_start_state = state_maker.make_dawg(machine_words, 0, false);
    let gaddag_start_state = match build_content {
        BuildContent::DawgOnly => 0,
        BuildContent::Gaddawg => state_maker.make_dawg(
            &gen_machine_drowwords(machine_words),
            dawg_start_state,
            true,
        ),
    };

    let mut states_defragger = StatesDefragger {
        states: &states,
        head_indexes: &match build_layout {
            BuildLayout::Legacy
            | BuildLayout::MagpieMerged
            | BuildLayout::Experimental
            | BuildLayout::Wolges => gen_head_indexes(&states),
            BuildLayout::Magpie => Vec::new(),
        },
        to_end_lens: &gen_to_end_lens(&states),
        destination: &mut vec![0u32; states.len()],
        num_written: match build_content {
            BuildContent::DawgOnly => 1,
            BuildContent::Gaddawg => 2,
        },
    };
    states_defragger.destination[0] = !0; // useful for empty lexicon
    match build_layout {
        BuildLayout::Legacy => states_defragger.defrag_legacy(dawg_start_state),
        BuildLayout::Magpie => states_defragger.defrag_magpie(dawg_start_state),
        BuildLayout::MagpieMerged => states_defragger.defrag_magpie_merged(dawg_start_state),
        BuildLayout::Experimental => states_defragger.build_experimental(
            &gen_num_ways(
                &states,
                &build_content,
                dawg_start_state,
                gaddag_start_state,
            ),
            &gen_top_indexes(&states, states_defragger.head_indexes),
        ),
        BuildLayout::Wolges => states_defragger.build_wolges(
            &gen_num_ways(
                &states,
                &build_content,
                dawg_start_state,
                gaddag_start_state,
            ),
            &build_content,
            dawg_start_state,
        ),
    }
    match build_content {
        BuildContent::DawgOnly => {}
        BuildContent::Gaddawg => match build_layout {
            BuildLayout::Legacy => states_defragger.defrag_legacy(gaddag_start_state),
            BuildLayout::Magpie => states_defragger.defrag_magpie(gaddag_start_state),
            BuildLayout::MagpieMerged => states_defragger.defrag_magpie_merged(gaddag_start_state),
            BuildLayout::Experimental | BuildLayout::Wolges => {}
        },
    }
    states_defragger.destination[0] = 0; // useful for empty lexicon

    if states_defragger.num_written
        > match VARIANT {
            1 => 0x400000,
            2 => 0x1000000,
            _ => 0,
        }
    {
        // the format can only have 0x400000 elements, each has 4 bytes
        return_error!(format!(
            "this format cannot have {} nodes",
            states_defragger.num_written
        ));
    }

    Ok(
        states_defragger.to_vec::<VARIANT>(build_content, dawg_start_state, gaddag_start_state)[..]
            .into(),
    )
}

#[inline(always)]
pub fn build(
    build_content: BuildContent,
    build_layout: BuildLayout,
    machine_words: &[bites::Bites],
) -> error::Returns<bites::Bites> {
    do_build::<1>(build_content, build_layout, machine_words)
}

#[inline(always)]
pub fn build_big(
    build_content: BuildContent,
    build_layout: BuildLayout,
    machine_words: &[bites::Bites],
) -> error::Returns<bites::Bites> {
    do_build::<2>(build_content, build_layout, machine_words)
}
