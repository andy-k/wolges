// Copyright (C) 2020-2026 Andy Kurnia.

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

#[inline]
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
#[inline]
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

pub enum BuildOrder {
    Sorted,
    Reordered,
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

    #[inline]
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

    #[inline]
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
    // (Node22)
    // bits 0-21 = pointer & 0x3fffff
    // bit 22 = end
    // bit 23 = is_terminal
    // bits 24-31 = char
    // (Node24)
    // bits 0-5 = char & 0x3f
    // bit 6 = end
    // bit 7 = is_terminal
    // bits 8-31 = pointer & 0xffffff
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
                out[0] = (tile & 0x3f) | ((is_end.0 as u8) << 6) | ((accepts.0 as u8) << 7);
                out[1] = defragged_arc_index as u8;
                out[2] = (defragged_arc_index >> 8) as u8;
                out[3] = (defragged_arc_index >> 16) as u8;
            }
            _ => unimplemented!(),
        }
    }

    #[inline]
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

#[inline]
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

#[inline]
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

#[inline]
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

#[inline]
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

#[inline]
fn is_sublist(states: &[State], mut a: u32, mut b: u32) -> bool {
    while a != 0 {
        let a_state = &states[a as usize];
        loop {
            if b == 0 {
                return false;
            }
            let b_state = &states[b as usize];
            if b_state.tile > a_state.tile {
                return false;
            }
            b = b_state.next_index;
            if b_state.tile == a_state.tile {
                if b_state.accepts != a_state.accepts || b_state.arc_index != a_state.arc_index {
                    return false;
                }
                break;
            }
        }
        a = a_state.next_index;
    }
    true
}

#[inline]
fn reorder_states(
    states: &[State],
    build_content: &BuildContent,
    dawg_start_state: u32,
    gaddag_start_state: u32,
) -> (Vec<State>, u32, u32) {
    #[inline(always)]
    fn entry_key(state: &State) -> u64 {
        ((state.arc_index as u64) << 9) | ((state.accepts as u64) << 8) | state.tile as u64
    }

    let states_len = states.len();
    let to_end_lens = gen_to_end_lens(states);

    let mut is_head = vec![false; states_len];
    for state in states.iter().skip(1) {
        is_head[state.arc_index as usize] = true;
    }
    for root in [dawg_start_state, gaddag_start_state] {
        is_head[root as usize] = true;
    }
    is_head[0] = false;

    let mut has_dawg = vec![false; states_len];
    if let BuildContent::Gaddawg = build_content {
        let mut in_dawg = vec![false; states_len];
        in_dawg[dawg_start_state as usize] = true;
        for p in (1..states_len).rev() {
            if in_dawg[p] {
                in_dawg[states[p].arc_index as usize] = true;
                in_dawg[states[p].next_index as usize] = true;
            }
        }
        for p in 1..states_len {
            has_dawg[p] = in_dawg[p] || has_dawg[states[p].next_index as usize];
        }
    }

    let mut movable = Vec::new();
    for p in 1..states_len {
        if is_head[p] && !has_dawg[p] {
            movable.push(p as u32);
        }
    }

    let mut entry_indexes = fash::MyHashMap::<u64, u32>::default();
    let mut entry_counts = Vec::<u32>::new();
    for &head in movable.iter() {
        let mut p = head;
        while p != 0 {
            let num_entries = entry_counts.len() as u32;
            let entry_index = *entry_indexes
                .entry(entry_key(&states[p as usize]))
                .or_insert(num_entries);
            if entry_index == num_entries {
                entry_counts.push(0);
            }
            entry_counts[entry_index as usize] += 1;
            p = states[p as usize].next_index;
        }
    }
    let mut postings_starts = Vec::with_capacity(entry_counts.len() + 1);
    let mut num_postings = 0u32;
    for &entry_count in entry_counts.iter() {
        postings_starts.push(num_postings);
        num_postings += entry_count;
    }
    postings_starts.push(num_postings);
    let mut postings = vec![0u32; num_postings as usize];
    let mut postings_fill = postings_starts.clone();
    let mut rarest_entries = Vec::with_capacity(movable.len());
    for (i, &head) in movable.iter().enumerate() {
        let mut rarest_entry = 0;
        let mut rarest_count = !0;
        let mut p = head;
        while p != 0 {
            let entry_index = entry_indexes[&entry_key(&states[p as usize])];
            postings[postings_fill[entry_index as usize] as usize] = i as u32;
            postings_fill[entry_index as usize] += 1;
            let entry_count = entry_counts[entry_index as usize];
            if entry_count < rarest_count {
                rarest_count = entry_count;
                rarest_entry = entry_index;
            }
            p = states[p as usize].next_index;
        }
        rarest_entries.push(rarest_entry);
    }

    let mut by_len = Vec::from_iter(0..movable.len() as u32);
    by_len.sort_unstable_by_key(|&i| {
        let head = movable[i as usize];
        (!to_end_lens[head as usize], head)
    });

    struct Absorber<'a> {
        states: &'a [State],
        movable: &'a [u32],
        to_end_lens: &'a [u32],
        postings: &'a [u32],
        postings_starts: &'a [u32],
        rarest_entries: &'a [u32],
        tried: Vec<u32>, // stamped with the list being placed
        stamp: u32,
    }
    impl Absorber<'_> {
        fn absorb(&mut self, tails: &mut [u32], i: u32) -> bool {
            let head = self.movable[i as usize];
            let len = self.to_end_lens[head as usize];
            let rarest_entry = self.rarest_entries[i as usize] as usize;
            for k in self.postings_starts[rarest_entry] as usize
                ..self.postings_starts[rarest_entry + 1] as usize
            {
                let j = self.postings[k];
                if j == i || self.tried[j as usize] == self.stamp {
                    continue;
                }
                let host = self.movable[j as usize];
                if self.to_end_lens[host as usize] <= len || !is_sublist(self.states, head, host) {
                    continue;
                }
                self.tried[j as usize] = self.stamp;
                if tails[j as usize] == !0 || self.absorb(tails, tails[j as usize]) {
                    tails[j as usize] = i;
                    return true;
                }
            }
            false
        }
    }
    let mut tails = vec![!0u32; movable.len()];
    let mut absorber = Absorber {
        states,
        movable: &movable,
        to_end_lens: &to_end_lens,
        postings: &postings,
        postings_starts: &postings_starts,
        rarest_entries: &rarest_entries,
        tried: vec![!0u32; movable.len()],
        stamp: 0,
    };
    for &i in by_len.iter() {
        absorber.stamp = i;
        absorber.absorb(&mut tails, i);
    }

    let mut order_starts = vec![0u32; states_len]; // 0 = keep the order it has.
    let mut order_cells = vec![0u32]; // index 0 stands for "no order of its own".
    for &i in by_len.iter().rev() {
        let tail = tails[i as usize];
        if tail == !0 {
            continue;
        }
        let head = movable[i as usize];
        let tail_head = movable[tail as usize];
        order_starts[head as usize] = order_cells.len() as u32;
        let mut p = head;
        let mut q = tail_head;
        while p != 0 {
            let p_state = &states[p as usize];
            while q != 0 && states[q as usize].tile < p_state.tile {
                q = states[q as usize].next_index;
            }
            if q != 0 && states[q as usize].tile == p_state.tile {
                q = states[q as usize].next_index;
            } else {
                order_cells.push(p);
            }
            p = p_state.next_index;
        }
        let tail_order_start = order_starts[tail_head as usize] as usize;
        if tail_order_start != 0 {
            order_cells.extend_from_within(
                tail_order_start..tail_order_start + to_end_lens[tail_head as usize] as usize,
            );
        } else {
            let mut p = tail_head;
            while p != 0 {
                order_cells.push(p);
                p = states[p as usize].next_index;
            }
        }
    }

    let mut new_states = Vec::with_capacity(states_len);
    new_states.push(states[0].clone());
    let mut states_finder = fash::MyHashMap::default();
    states_finder.insert(states[0].clone(), 0);
    let mut state_maker = StateMaker {
        states: &mut new_states,
        states_finder: &mut states_finder,
    };
    let mut new_heads = vec![0u32; states_len];
    let mut transitions = Vec::new();
    for p in 1..states_len {
        if !is_head[p] {
            continue;
        }
        transitions.clear();
        let order_start = order_starts[p] as usize;
        if order_start != 0 {
            for &cell in order_cells[order_start..order_start + to_end_lens[p] as usize].iter() {
                let state = &states[cell as usize];
                transitions.push(Transition {
                    tile: state.tile,
                    accepts: state.accepts,
                    arc_index: new_heads[state.arc_index as usize],
                });
            }
        } else {
            let mut q = p as u32;
            while q != 0 {
                let state = &states[q as usize];
                transitions.push(Transition {
                    tile: state.tile,
                    accepts: state.accepts,
                    arc_index: new_heads[state.arc_index as usize],
                });
                q = state.next_index;
            }
        }
        new_heads[p] = state_maker.make_state(&transitions);
    }

    (
        new_states,
        new_heads[dawg_start_state as usize],
        new_heads[gaddag_start_state as usize],
    )
}

// machine_words must be sorted and unique.
#[inline]
fn do_build<const VARIANT: u8>(
    build_content: BuildContent,
    build_layout: BuildLayout,
    build_order: BuildOrder,
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

    let (states, dawg_start_state, gaddag_start_state) = match build_order {
        BuildOrder::Sorted => (states, dawg_start_state, gaddag_start_state),
        BuildOrder::Reordered => reorder_states(
            &states,
            &build_content,
            dawg_start_state,
            gaddag_start_state,
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
    build_order: BuildOrder,
    machine_words: &[bites::Bites],
) -> error::Returns<bites::Bites> {
    do_build::<1>(build_content, build_layout, build_order, machine_words)
}

#[inline(always)]
pub fn build_big(
    build_content: BuildContent,
    build_layout: BuildLayout,
    build_order: BuildOrder,
    machine_words: &[bites::Bites],
) -> error::Returns<bites::Bites> {
    do_build::<2>(build_content, build_layout, build_order, machine_words)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::alphabet;
    use crate::kwg::{self, Node};

    fn collect_dawg_words<N: kwg::Node>(
        kwg: &kwg::Kwg<N>,
        p: i32,
        word: &mut Vec<u8>,
        out: &mut Vec<bites::Bites>,
    ) {
        if p <= 0 {
            return;
        }
        let mut i = p;
        loop {
            let node = kwg[i];
            word.push(node.tile());
            if node.accepts() {
                out.push(word[..].into());
            }
            if node.arc_index() != 0 {
                collect_dawg_words(kwg, node.arc_index(), word, out);
            }
            word.pop();
            if node.is_end() {
                break;
            }
            i += 1;
        }
    }

    fn collect_gaddag_words<N: kwg::Node>(
        kwg: &kwg::Kwg<N>,
        p: i32,
        rev: &mut Vec<u8>,
        out: &mut Vec<bites::Bites>,
    ) {
        let mut i = p;
        loop {
            let node = kwg[i];
            if node.tile() == 0 {
                let mut suffixes = Vec::new();
                collect_dawg_words(kwg, node.arc_index(), &mut Vec::new(), &mut suffixes);
                for suffix in suffixes.iter() {
                    let mut word = rev.clone();
                    word.reverse();
                    word.extend_from_slice(suffix);
                    out.push(word[..].into());
                }
            } else {
                rev.push(node.tile());
                if node.accepts() {
                    let mut word = rev.clone();
                    word.reverse();
                    out.push(word[..].into());
                }
                if node.arc_index() != 0 {
                    collect_gaddag_words(kwg, node.arc_index(), rev, out);
                }
                rev.pop();
            }
            if node.is_end() {
                break;
            }
            i += 1;
        }
    }

    fn collect_dawg_tiles<N: kwg::Node>(
        kwg: &kwg::Kwg<N>,
        p: i32,
        seen: &mut [bool],
        out: &mut Vec<u8>,
    ) {
        if p <= 0 || seen[p as usize] {
            return;
        }
        let mut i = p;
        loop {
            seen[i as usize] = true;
            let node = kwg[i];
            out.push(node.tile());
            if node.arc_index() != 0 {
                collect_dawg_tiles(kwg, node.arc_index(), seen, out);
            }
            if node.is_end() {
                break;
            }
            i += 1;
        }
    }

    #[inline]
    fn build_kwg(
        layout: BuildLayout,
        order: BuildOrder,
        words: &[&str],
    ) -> (Vec<bites::Bites>, kwg::Kwg<kwg::Node22>) {
        let machine_words: Vec<bites::Bites> = words
            .iter()
            .map(|w| w.bytes().collect::<Vec<u8>>()[..].into())
            .collect();
        let bytes = build(BuildContent::Gaddawg, layout, order, &machine_words).unwrap();
        (
            machine_words,
            kwg::Kwg::<kwg::Node22>::from_bytes_alloc(&bytes),
        )
    }

    #[inline(always)]
    fn round_trip_dawg(layout: BuildLayout, words: &[&str]) {
        round_trip_ordered(layout, BuildOrder::Sorted, words);
    }

    #[inline]
    fn round_trip_ordered(layout: BuildLayout, order: BuildOrder, words: &[&str]) {
        let (machine_words, kwg) = build_kwg(layout, order, words);
        let mut expected = machine_words.clone();
        expected.sort_unstable();
        expected.dedup();

        let mut got = Vec::new();
        collect_dawg_words(&kwg, kwg[0].arc_index(), &mut Vec::new(), &mut got);
        got.sort_unstable();
        assert_eq!(got, expected);

        let mut got_gaddag = Vec::new();
        if kwg[1].arc_index() != 0 {
            collect_gaddag_words(&kwg, kwg[1].arc_index(), &mut Vec::new(), &mut got_gaddag);
        }
        got_gaddag.sort_unstable();
        got_gaddag.dedup();
        assert_eq!(got_gaddag, expected);
    }

    static WORD_LIST: &[&str] = &[
        "AA", "AAH", "AAHED", "AAL", "AALS", "AAS", "AB", "ABA", "ABAC", "ABS", "ABY", "CAB",
        "CAD", "CAT", "CATS", "ZAP", "ZAPS", "ZED", "ZOO",
    ];

    static LONGER_WORD_LIST: &[&str] = &[
        "AE", "AH", "AI", "AL", "AN", "AR", "AS", "AT", "EAR", "EAT", "ERA", "ETA", "HAE", "HAT",
        "HEAR", "HEART", "HEAT", "HEATER", "HER", "HERS", "LEA", "LEAN", "LEARN", "LEARNS",
        "LEAST", "NEAR", "NEAT", "RAT", "RATE", "REAL", "SEAT", "SHEAR", "STEAL", "TEA", "TEAL",
        "TEAR", "TEARS", "THE", "THEN", "THERE", "TREAT",
    ];

    #[test]
    #[inline(always)]
    fn round_trip_wolges() {
        round_trip_dawg(BuildLayout::Wolges, WORD_LIST);
    }

    #[test]
    #[inline(always)]
    fn round_trip_legacy() {
        round_trip_dawg(BuildLayout::Legacy, WORD_LIST);
    }

    #[test]
    #[inline(always)]
    fn round_trip_empty() {
        round_trip_dawg(BuildLayout::Wolges, &[]);
    }

    #[test]
    #[inline(always)]
    fn round_trip_single() {
        round_trip_dawg(BuildLayout::Wolges, &["HELLO"]);
    }

    #[test]
    #[inline]
    fn empty_graph_constants_are_what_the_builder_writes() {
        assert_eq!(
            &build(
                BuildContent::Gaddawg,
                BuildLayout::Wolges,
                BuildOrder::Sorted,
                &[]
            )
            .unwrap()[..],
            kwg::EMPTY_KWG_BYTES,
        );
        assert_eq!(
            &build_big(
                BuildContent::Gaddawg,
                BuildLayout::Wolges,
                BuildOrder::Sorted,
                &[]
            )
            .unwrap()[..],
            kwg::EMPTY_KBWG_BYTES,
        );
    }

    #[test]
    #[inline]
    fn empty_leaves_constant_is_what_the_format_says() {
        let dawg = build(
            BuildContent::DawgOnly,
            BuildLayout::Wolges,
            BuildOrder::Sorted,
            &[],
        )
        .unwrap();
        let mut expected = Vec::new();
        expected.extend_from_slice(&((dawg.len() / 4) as u32).to_le_bytes());
        expected.extend_from_slice(&dawg);
        expected.extend_from_slice(&0u32.to_le_bytes());
        assert_eq!(expected, crate::klv::EMPTY_KLV_BYTES);

        let klv = crate::klv::Klv::<kwg::Node22>::from_bytes_alloc(crate::klv::EMPTY_KLV_BYTES);
        assert_eq!(klv.leave_value_from_tally(&[0, 1]), 0);
    }

    #[inline]
    fn shipped_alphabets() -> Vec<(&'static str, alphabet::Alphabet)> {
        vec![
            ("catalan", alphabet::make_catalan_alphabet()),
            ("super_catalan", alphabet::make_super_catalan_alphabet()),
            ("decimal", alphabet::make_decimal_alphabet()),
            ("dutch", alphabet::make_dutch_alphabet()),
            ("english", alphabet::make_english_alphabet()),
            ("french", alphabet::make_french_alphabet()),
            ("german", alphabet::make_german_alphabet()),
            ("hex", alphabet::make_hex_alphabet()),
            (
                "hong_kong_english",
                alphabet::make_hong_kong_english_alphabet(),
            ),
            ("norwegian", alphabet::make_norwegian_alphabet()),
            ("polish", alphabet::make_polish_alphabet()),
            ("slovene", alphabet::make_slovene_alphabet()),
            ("spanish", alphabet::make_spanish_alphabet()),
            ("super_english", alphabet::make_super_english_alphabet()),
            ("swedish", alphabet::make_swedish_alphabet()),
        ]
    }

    #[inline]
    fn words_over_tiles(len: u8) -> Vec<bites::Bites> {
        let last = len - 1;
        let mut words = Vec::new();
        for tile in 1..len {
            words.push(vec![tile, tile][..].into());
            words.push(vec![tile, last][..].into());
            words.push(vec![last, tile][..].into());
            words.push(vec![1, tile, last][..].into());
            words.push(vec![tile, tile, tile, last, 1][..].into());
        }
        words.sort_unstable();
        words.dedup();
        words
    }

    #[inline]
    fn words_in_graph<N: kwg::Node>(bytes: &[u8]) -> Vec<bites::Bites> {
        let kwg = kwg::Kwg::<N>::from_bytes_alloc(bytes);
        let mut got = Vec::new();
        collect_dawg_words(&kwg, kwg[0].arc_index(), &mut Vec::new(), &mut got);
        got.sort_unstable();
        got
    }

    #[test]
    #[inline]
    fn every_graph_holds_every_shipped_alphabet() {
        for (name, alphabet) in shipped_alphabets() {
            let len = alphabet.len();
            assert!(len >= 2, "{name}: an alphabet is a blank and some tiles");
            let words = words_over_tiles(len);

            let bytes = build(
                BuildContent::Gaddawg,
                BuildLayout::Wolges,
                BuildOrder::Sorted,
                &words,
            )
            .unwrap_or_else(|e| panic!("{name}: kwg refused {len} tiles: {e}"));
            assert_eq!(words_in_graph::<kwg::Node22>(&bytes), words, "{name}: kwg");

            let bytes = build(
                BuildContent::DawgOnly,
                BuildLayout::Wolges,
                BuildOrder::Sorted,
                &words,
            )
            .unwrap_or_else(|e| panic!("{name}: dawg refused {len} tiles: {e}"));
            assert_eq!(words_in_graph::<kwg::Node22>(&bytes), words, "{name}: dawg");

            let bytes = build_big(
                BuildContent::Gaddawg,
                BuildLayout::Wolges,
                BuildOrder::Sorted,
                &words,
            )
            .unwrap_or_else(|e| panic!("{name}: kbwg refused {len} tiles: {e}"));
            assert_eq!(words_in_graph::<kwg::Node24>(&bytes), words, "{name}: kbwg");

            let alphagrams = make_alphagrams(&words);
            let bytes = build(
                BuildContent::DawgOnly,
                BuildLayout::Wolges,
                BuildOrder::Sorted,
                &alphagrams,
            )
            .unwrap_or_else(|e| panic!("{name}: kad refused {len} tiles: {e}"));
            assert_eq!(
                words_in_graph::<kwg::Node22>(&bytes),
                alphagrams.to_vec(),
                "{name}: kad",
            );
        }
    }

    #[test]
    #[inline]
    fn the_matrix_holds_a_full_sixty_four_tile_alphabet() {
        let widest = shipped_alphabets()
            .iter()
            .map(|(_, alphabet)| alphabet.len())
            .max();
        assert_eq!(
            widest,
            Some(64),
            "no shipped alphabet fills the tile field, so nothing here tests it",
        );
    }

    #[test]
    #[inline]
    fn the_matrix_holds_every_alphabet_the_crate_ships() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("src")
            .join("alphabets");
        let mut on_disk = std::fs::read_dir(&dir)
            .unwrap_or_else(|e| panic!("cannot read {}: {e}", dir.display()))
            .map(|entry| {
                entry
                    .unwrap()
                    .file_name()
                    .to_string_lossy()
                    .trim_end_matches(".txt")
                    .to_string()
            })
            .collect::<Vec<_>>();
        on_disk.sort_unstable();
        let mut in_matrix = shipped_alphabets()
            .iter()
            .map(|(name, _)| name.to_string())
            .collect::<Vec<_>>();
        in_matrix.sort_unstable();
        assert_eq!(in_matrix, on_disk, "the matrix and the crate disagree");
    }

    #[test]
    #[inline]
    fn round_trip_reordered() {
        round_trip_ordered(BuildLayout::Wolges, BuildOrder::Reordered, WORD_LIST);
        round_trip_ordered(BuildLayout::Legacy, BuildOrder::Reordered, WORD_LIST);
        round_trip_ordered(BuildLayout::MagpieMerged, BuildOrder::Reordered, WORD_LIST);
        round_trip_ordered(BuildLayout::Wolges, BuildOrder::Reordered, &[]);
        round_trip_ordered(BuildLayout::Wolges, BuildOrder::Reordered, &["HELLO"]);
    }

    #[test]
    #[inline]
    fn reordered_gaddawg_keeps_its_dawg() {
        let (_, sorted) = build_kwg(BuildLayout::Wolges, BuildOrder::Sorted, LONGER_WORD_LIST);
        let (_, reordered) =
            build_kwg(BuildLayout::Wolges, BuildOrder::Reordered, LONGER_WORD_LIST);

        let mut sorted_tiles = Vec::new();
        collect_dawg_tiles(
            &sorted,
            sorted[0].arc_index(),
            &mut vec![false; sorted.0.len()],
            &mut sorted_tiles,
        );
        let mut reordered_tiles = Vec::new();
        collect_dawg_tiles(
            &reordered,
            reordered[0].arc_index(),
            &mut vec![false; reordered.0.len()],
            &mut reordered_tiles,
        );
        assert_eq!(sorted_tiles, reordered_tiles);

        let mut num_descents = 0;
        for i in 1..reordered.0.len() - 1 {
            if !reordered[i as i32].is_end()
                && reordered[i as i32].tile() > reordered[i as i32 + 1].tile()
            {
                num_descents += 1;
            }
        }
        assert!(num_descents > 0, "nothing was reordered");

        let mut num_moved_turnarounds = 0;
        for i in 2..reordered.0.len() {
            if reordered[i as i32].tile() == 0 && !reordered[i as i32 - 1].is_end() {
                num_moved_turnarounds += 1;
            }
        }
        assert!(
            num_moved_turnarounds > 0,
            "no turnaround tile moved off the front of its list"
        );
    }

    #[test]
    #[inline]
    fn reordered_dawg_only_round_trips() {
        let machine_words: Vec<bites::Bites> = WORD_LIST
            .iter()
            .map(|w| w.bytes().collect::<Vec<u8>>()[..].into())
            .collect();
        let bytes = build(
            BuildContent::DawgOnly,
            BuildLayout::Wolges,
            BuildOrder::Reordered,
            &machine_words,
        )
        .unwrap();
        let kwg = kwg::Kwg::<kwg::Node22>::from_bytes_alloc(&bytes);
        let mut got = Vec::new();
        collect_dawg_words(&kwg, kwg[0].arc_index(), &mut Vec::new(), &mut got);
        got.sort_unstable();
        let mut expected = machine_words.clone();
        expected.sort_unstable();
        expected.dedup();
        assert_eq!(got, expected);
    }
}
