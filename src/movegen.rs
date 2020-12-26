use super::{bites, board_layout, game_config, klv, kwg, matrix};

#[derive(Clone)]
struct CrossSet {
    bits: u64,
    score: i16,
}

#[derive(Clone, Copy)]
struct CachedCrossSet {
    p_left: i32,
    p_right: i32,
    bits: u64,
}

#[derive(Clone)]
struct CrossSetComputation {
    score: i16,
    b_letter: u8,
    end_range: i8,
    p: i32,
}

struct WorkingBuffer {
    rack_tally: Box<[u8]>,                                    // 27 for ?A-Z
    word_buffer: Box<[u8]>,                                   // max(r, c)
    cross_set_for_across_plays: Box<[CrossSet]>,              // r*c
    cross_set_for_down_plays: Box<[CrossSet]>,                // c*r
    cached_cross_set_for_across_plays: Box<[CachedCrossSet]>, // c*r
    cached_cross_set_for_down_plays: Box<[CachedCrossSet]>,   // r*c
    cross_set_buffer: Box<[CrossSetComputation]>,             // max(r,c)
    num_tiles_on_board: u16,
    exchange_buffer: Vec<u8>, // rack.len()
}

impl Clone for WorkingBuffer {
    #[inline(always)]
    fn clone(&self) -> Self {
        Self {
            rack_tally: self.rack_tally.clone(),
            word_buffer: self.word_buffer.clone(),
            cross_set_for_across_plays: self.cross_set_for_across_plays.clone(),
            cross_set_for_down_plays: self.cross_set_for_down_plays.clone(),
            cached_cross_set_for_across_plays: self.cached_cross_set_for_across_plays.clone(),
            cached_cross_set_for_down_plays: self.cached_cross_set_for_down_plays.clone(),
            cross_set_buffer: self.cross_set_buffer.clone(),
            num_tiles_on_board: self.num_tiles_on_board,
            exchange_buffer: self.exchange_buffer.clone(),
        }
    }

    #[inline(always)]
    fn clone_from(&mut self, source: &Self) {
        self.rack_tally.clone_from(&source.rack_tally);
        self.word_buffer.clone_from(&source.word_buffer);
        self.cross_set_for_across_plays
            .clone_from(&source.cross_set_for_across_plays);
        self.cross_set_for_down_plays
            .clone_from(&source.cross_set_for_down_plays);
        self.cached_cross_set_for_across_plays
            .clone_from(&source.cached_cross_set_for_across_plays);
        self.cached_cross_set_for_down_plays
            .clone_from(&source.cached_cross_set_for_down_plays);
        self.cross_set_buffer.clone_from(&source.cross_set_buffer);
        self.num_tiles_on_board
            .clone_from(&source.num_tiles_on_board);
        self.exchange_buffer.clone_from(&source.exchange_buffer);
    }
}

impl WorkingBuffer {
    fn new(game_config: &game_config::GameConfig) -> Self {
        let dim = game_config.board_layout().dim();
        let rows_times_cols = ((dim.rows as isize) * (dim.cols as isize)) as usize;
        Self {
            rack_tally: vec![0u8; game_config.alphabet().len() as usize].into_boxed_slice(),
            word_buffer: vec![0u8; std::cmp::max(dim.rows, dim.cols) as usize].into_boxed_slice(),
            cross_set_for_across_plays: vec![CrossSet { bits: 0, score: 0 }; rows_times_cols]
                .into_boxed_slice(),
            cross_set_for_down_plays: vec![CrossSet { bits: 0, score: 0 }; rows_times_cols]
                .into_boxed_slice(),
            cached_cross_set_for_across_plays: vec![
                CachedCrossSet {
                    p_left: 0,
                    p_right: 0,
                    bits: 0
                };
                rows_times_cols
            ]
            .into_boxed_slice(),
            cached_cross_set_for_down_plays: vec![
                CachedCrossSet {
                    p_left: 0,
                    p_right: 0,
                    bits: 0
                };
                rows_times_cols
            ]
            .into_boxed_slice(),
            cross_set_buffer: vec![
                CrossSetComputation {
                    score: 0,
                    b_letter: 0,
                    end_range: 0,
                    p: 0,
                };
                std::cmp::max(dim.rows, dim.cols) as usize
            ]
            .into_boxed_slice(),
            num_tiles_on_board: 0,
            exchange_buffer: Vec::new(),
        }
    }

    fn init(&mut self, board_snapshot: &BoardSnapshot<'_>, rack: &[u8]) {
        self.exchange_buffer.clear();
        self.exchange_buffer.reserve(rack.len());
        self.rack_tally.iter_mut().for_each(|m| *m = 0);
        for tile in &rack[..] {
            self.rack_tally[*tile as usize] += 1;
        }
        self.num_tiles_on_board = board_snapshot
            .board_tiles
            .iter()
            .filter(|&t| *t != 0)
            .count() as u16;
    }
}

pub struct BoardSnapshot<'a> {
    pub board_tiles: &'a [u8],
    pub game_config: &'a game_config::GameConfig<'a>,
    pub kwg: &'a kwg::Kwg,
    pub klv: &'a klv::Klv,
}

// cached_cross_sets is just one slice, so it is transposed from cross_sets
fn gen_cross_set<'a>(
    board_snapshot: &'a BoardSnapshot<'a>,
    strider: matrix::Strider,
    cross_sets: &'a mut [CrossSet],
    output_strider: matrix::Strider,
    cross_set_buffer: &'a mut [CrossSetComputation],
    mut cached_cross_sets: &'a mut [CachedCrossSet],
) {
    let len = strider.len();
    for i in 0..output_strider.len() {
        cross_sets[output_strider.at(i)] = CrossSet { bits: 0, score: 0 };
    }

    let alphabet = board_snapshot.game_config.alphabet();

    let reuse_cross_set =
        |cached_cross_sets: &mut [CachedCrossSet], out_idx: i8, p_left, p_right| -> u64 {
            if cached_cross_sets[out_idx as usize].p_left == p_left
                && cached_cross_sets[out_idx as usize].p_right == p_right
            {
                cached_cross_sets[out_idx as usize].bits
            } else {
                cached_cross_sets[out_idx as usize].p_left = p_left;
                cached_cross_sets[out_idx as usize].p_right = p_right;
                0 // means unset, because bit 0 should always be set
            }
        };

    let mut last_nonempty = len;
    {
        let mut p = 1;
        let mut score = 0i16;
        let mut last_empty = len;
        for j in (0..len).rev() {
            let b = board_snapshot.board_tiles[strider.at(j)];
            if b != 0 {
                let b_letter = b & 0x7f;
                p = board_snapshot.kwg.seek(p, b_letter);
                score += alphabet.score(b) as i16;
                cross_set_buffer[j as usize] = CrossSetComputation {
                    score,
                    b_letter,
                    end_range: last_empty,
                    p,
                };
                last_nonempty = j;
            } else {
                // empty square, reset
                p = 1; // cumulative gaddag traversal results
                score = 0; // cumulative face-value score
                last_empty = j; // last seen empty square
                cross_set_buffer[j as usize].b_letter = 0;
                cross_set_buffer[j as usize].end_range = last_nonempty;
            }
        }
    }
    let mut j = last_nonempty;
    while j < len {
        if j > 0 {
            // [j-1] has right, no left.
            let mut p = cross_set_buffer[j as usize].p;
            let mut bits = reuse_cross_set(cached_cross_sets, j - 1, -2, p);
            if bits == 0 {
                bits = 1u64;
                if p > 0 {
                    p = board_snapshot.kwg[p].arc_index();
                    if p > 0 {
                        loop {
                            let node = board_snapshot.kwg[p];
                            bits |= (node.accepts() as u64) << node.tile();
                            if node.is_end() {
                                break;
                            }
                            p += 1;
                        }
                    }
                }
                cached_cross_sets[j as usize - 1].bits = bits;
            }
            cross_sets[output_strider.at(j - 1)] = CrossSet {
                bits,
                score: cross_set_buffer[j as usize].score,
            };
        }
        let mut prev_j = j;
        j = cross_set_buffer[j as usize].end_range;
        if j >= len {
            break;
        }
        while j + 1 < len && cross_set_buffer[j as usize + 1].b_letter != 0 {
            j += 1;
            // [j-1] has left and right.
            let j_end = cross_set_buffer[j as usize].end_range;
            let mut p_right = cross_set_buffer[j as usize].p;
            let mut p_left = board_snapshot
                .kwg
                .seek(cross_set_buffer[prev_j as usize].p, 0);
            let mut bits = reuse_cross_set(&mut cached_cross_sets, j - 1, p_left, p_right);
            if bits == 0 {
                bits = 1u64;
                if p_right > 0 && p_left > 0 {
                    p_right = board_snapshot.kwg[p_right].arc_index();
                    if p_right > 0 {
                        p_left = board_snapshot.kwg[p_left].arc_index();
                        if p_left > 0 {
                            let mut node_left = board_snapshot.kwg[p_left];
                            let mut node_right = board_snapshot.kwg[p_right];
                            let mut node_left_tile = node_left.tile();
                            if j_end - j > j - 1 - prev_j {
                                // Right is longer than left.
                                loop {
                                    match node_left_tile.cmp(&node_right.tile()) {
                                        std::cmp::Ordering::Less => {
                                            // left < right: advance left
                                            if node_left.is_end() {
                                                break;
                                            }
                                            p_left += 1;
                                            node_left = board_snapshot.kwg[p_left];
                                            node_left_tile = node_left.tile();
                                        }
                                        std::cmp::Ordering::Greater => {
                                            // left > right: advance right
                                            if node_right.is_end() {
                                                break;
                                            }
                                            p_right += 1;
                                            node_right = board_snapshot.kwg[p_right];
                                        }
                                        std::cmp::Ordering::Equal => {
                                            // left == right (right is longer than left):
                                            // complete right half with the shorter left half
                                            let mut q = p_right;
                                            for qi in (prev_j..j - 1).rev() {
                                                q = board_snapshot.kwg.seek(
                                                    q,
                                                    cross_set_buffer[qi as usize].b_letter,
                                                );
                                                if q <= 0 {
                                                    break;
                                                }
                                            }
                                            if q > 0 {
                                                bits |= (board_snapshot.kwg[q].accepts() as u64)
                                                    << node_left_tile;
                                            }
                                            if node_left.is_end() {
                                                break;
                                            }
                                            p_left += 1;
                                            node_left = board_snapshot.kwg[p_left];
                                            node_left_tile = node_left.tile();
                                            if node_right.is_end() {
                                                break;
                                            }
                                            p_right += 1;
                                            node_right = board_snapshot.kwg[p_right];
                                        }
                                    }
                                }
                            } else {
                                loop {
                                    match node_left_tile.cmp(&node_right.tile()) {
                                        std::cmp::Ordering::Less => {
                                            // left < right: advance left
                                            if node_left.is_end() {
                                                break;
                                            }
                                            p_left += 1;
                                            node_left = board_snapshot.kwg[p_left];
                                            node_left_tile = node_left.tile();
                                        }
                                        std::cmp::Ordering::Greater => {
                                            // left > right: advance right
                                            if node_right.is_end() {
                                                break;
                                            }
                                            p_right += 1;
                                            node_right = board_snapshot.kwg[p_right];
                                        }
                                        std::cmp::Ordering::Equal => {
                                            // left == right (right is not longer than left):
                                            // complete left half with right half
                                            let mut q = p_left;
                                            for qi in j..j_end {
                                                q = board_snapshot.kwg.seek(
                                                    q,
                                                    cross_set_buffer[qi as usize].b_letter,
                                                );
                                                if q <= 0 {
                                                    break;
                                                }
                                            }
                                            if q > 0 {
                                                bits |= (board_snapshot.kwg[q].accepts() as u64)
                                                    << node_left_tile;
                                            }
                                            if node_right.is_end() {
                                                break;
                                            }
                                            p_right += 1;
                                            node_right = board_snapshot.kwg[p_right];
                                            if node_left.is_end() {
                                                break;
                                            }
                                            p_left += 1;
                                            node_left = board_snapshot.kwg[p_left];
                                            node_left_tile = node_left.tile();
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
                cached_cross_sets[j as usize - 1].bits = bits;
            }
            cross_sets[output_strider.at(j - 1)] = CrossSet {
                bits,
                score: cross_set_buffer[prev_j as usize].score + cross_set_buffer[j as usize].score,
            };
            prev_j = j;
            j = j_end;
        }
        if j >= len {
            break;
        }
        // [j] has left, no right.
        let mut p = board_snapshot
            .kwg
            .seek(cross_set_buffer[prev_j as usize].p, 0);
        let mut bits = reuse_cross_set(&mut cached_cross_sets, j, p, -2);
        if bits == 0 {
            bits = 1u64;
            if p > 0 {
                p = board_snapshot.kwg[p].arc_index();
                if p > 0 {
                    loop {
                        let node = board_snapshot.kwg[p];
                        bits |= (node.accepts() as u64) << node.tile();
                        if node.is_end() {
                            break;
                        }
                        p += 1;
                    }
                }
            }
            cached_cross_sets[j as usize].bits = bits;
        }
        cross_sets[output_strider.at(j)] = CrossSet {
            bits,
            score: cross_set_buffer[prev_j as usize].score,
        };
        j = cross_set_buffer[j as usize].end_range;
    }
}

// word_buffer must have at least strider.len() length.
#[allow(clippy::too_many_arguments)]
fn gen_place_moves<'a, CallbackType: FnMut(i8, &[u8], i16, &[u8])>(
    board_snapshot: &'a BoardSnapshot<'a>,
    cross_set_slice: &'a [CrossSet],
    rack_tally: &'a mut [u8],
    strider: matrix::Strider,
    word_buffer: &'a mut [u8],
    num_max_played: u8,
    single_tile_plays: bool,
    callback: CallbackType,
) {
    let len = strider.len();
    word_buffer
        .iter_mut()
        .take(len as usize)
        .for_each(|m| *m = 0);

    struct Env<'a, CallbackType: FnMut(i8, &[u8], i16, &[u8])> {
        board_snapshot: &'a BoardSnapshot<'a>,
        cross_set_slice: &'a [CrossSet],
        rack_tally: &'a mut [u8],
        strider: matrix::Strider,
        callback: CallbackType,
        word_buffer: &'a mut [u8],
        anchor: i8,
        leftmost: i8,
        rightmost: i8,
        num_max_played: u8,
        num_played: i8,
        idx_left: i8,
    }

    let mut env = Env {
        board_snapshot,
        cross_set_slice,
        rack_tally,
        strider,
        callback,
        word_buffer,
        anchor: 0,
        leftmost: 0,
        rightmost: 0,
        num_max_played,
        num_played: 0,
        idx_left: 0,
    };

    fn record<CallbackType: FnMut(i8, &[u8], i16, &[u8])>(
        env: &mut Env<CallbackType>,
        idx_left: i8,
        idx_right: i8,
        main_score: i16,
        perpendicular_score: i16,
        word_multiplier: i8,
    ) {
        let score = main_score * (word_multiplier as i16)
            + perpendicular_score
            + env
                .board_snapshot
                .game_config
                .num_played_bonus(env.num_played);
        (env.callback)(
            idx_left,
            &env.word_buffer[(idx_left as usize)..(idx_right as usize)],
            score,
            env.rack_tally,
        );
    }

    fn play_right<CallbackType: FnMut(i8, &[u8], i16, &[u8])>(
        env: &mut Env<CallbackType>,
        mut idx: i8,
        mut p: i32,
        mut main_score: i16,
        perpendicular_score: i16,
        word_multiplier: i8,
        mut is_unique: bool,
    ) {
        // tail-recurse placing current sequence of tiles
        while idx < env.rightmost {
            let b = env.board_snapshot.board_tiles[env.strider.at(idx)];
            if b == 0 {
                break;
            }
            p = env.board_snapshot.kwg.seek(p, b & 0x7f);
            if p <= 0 {
                return;
            }
            main_score += env.board_snapshot.game_config.alphabet().score(b) as i16;
            idx += 1;
        }
        let node = env.board_snapshot.kwg[p];
        if idx > env.anchor + 1
            && (env.num_played + is_unique as i8) >= 2
            && idx - env.idx_left >= 2
            && node.accepts()
        {
            record(
                env,
                env.idx_left,
                idx,
                main_score,
                perpendicular_score,
                word_multiplier,
            );
        }
        if idx >= env.rightmost {
            return;
        }
        if env.num_played as u8 >= env.num_max_played {
            return;
        }

        p = node.arc_index();
        if p <= 0 {
            return;
        }
        let this_cross_set = env.cross_set_slice[idx as usize].clone();
        if this_cross_set.bits == 1 {
            // already handled '@'
            return;
        }
        let this_premium =
            env.board_snapshot.game_config.board_layout().premiums()[env.strider.at(idx)];
        let new_word_multiplier = word_multiplier * this_premium.word_multiplier;
        let this_cross_bits;
        if this_cross_set.bits != 0 {
            // turn off bit 0 so it cannot match later
            this_cross_bits = this_cross_set.bits & !1;
        } else {
            this_cross_bits = !1;
            is_unique = true;
        };
        let perpendicular_word_multiplier =
            this_premium.word_multiplier as i16 & -(this_cross_set.bits as i16 & 1);
        loop {
            let node = env.board_snapshot.kwg[p];
            let tile = node.tile();
            if this_cross_bits & (1 << tile) != 0 {
                if env.rack_tally[tile as usize] > 0 {
                    env.rack_tally[tile as usize] -= 1;
                    env.num_played += 1;
                    let tile_value = (env.board_snapshot.game_config.alphabet().score(tile) as i16)
                        * (this_premium.tile_multiplier as i16);
                    env.word_buffer[idx as usize] = tile;
                    play_right(
                        env,
                        idx + 1,
                        p,
                        main_score + tile_value,
                        perpendicular_score
                            + (this_cross_set.score + tile_value) * perpendicular_word_multiplier,
                        new_word_multiplier,
                        is_unique,
                    );
                    env.num_played -= 1;
                    env.rack_tally[tile as usize] += 1;
                }
                if env.rack_tally[0] > 0 {
                    env.rack_tally[0] -= 1;
                    env.num_played += 1;
                    // intentional to not hardcode blank tile value as zero
                    let tile_value = (env.board_snapshot.game_config.alphabet().score(0) as i16)
                        * (this_premium.tile_multiplier as i16);
                    env.word_buffer[idx as usize] = tile | 0x80;
                    play_right(
                        env,
                        idx + 1,
                        p,
                        main_score + tile_value,
                        perpendicular_score
                            + (this_cross_set.score + tile_value) * perpendicular_word_multiplier,
                        new_word_multiplier,
                        is_unique,
                    );
                    env.num_played -= 1;
                    env.rack_tally[0] += 1;
                }
            }
            if node.is_end() {
                break;
            }
            p += 1;
        }
    }

    fn play_left<CallbackType: FnMut(i8, &[u8], i16, &[u8])>(
        env: &mut Env<CallbackType>,
        mut idx: i8,
        mut p: i32,
        mut main_score: i16,
        perpendicular_score: i16,
        word_multiplier: i8,
        mut is_unique: bool,
    ) {
        // tail-recurse placing current sequence of tiles
        while idx >= env.leftmost {
            let b = env.board_snapshot.board_tiles[env.strider.at(idx)];
            if b == 0 {
                break;
            }
            p = env.board_snapshot.kwg.seek(p, b & 0x7f);
            if p <= 0 {
                return;
            }
            main_score += env.board_snapshot.game_config.alphabet().score(b) as i16;
            idx -= 1;
        }
        let node = env.board_snapshot.kwg[p];
        if (env.num_played + is_unique as i8) >= 2 && env.anchor - idx >= 2 && node.accepts() {
            record(
                env,
                idx + 1,
                env.anchor + 1,
                main_score,
                perpendicular_score,
                word_multiplier,
            );
        }
        if env.num_played as u8 >= env.num_max_played {
            return;
        }

        p = node.arc_index();
        if p <= 0 {
            return;
        }
        let this_premium;
        let this_cross_set;
        if idx >= env.leftmost {
            this_premium =
                env.board_snapshot.game_config.board_layout().premiums()[env.strider.at(idx)];
            this_cross_set = env.cross_set_slice[idx as usize].clone();
        } else {
            this_premium = board_layout::Premium {
                word_multiplier: 0,
                tile_multiplier: 0,
            };
            this_cross_set = CrossSet { bits: 0, score: 0 };
        }
        let new_word_multiplier = word_multiplier * this_premium.word_multiplier;
        let prev_is_unique = is_unique;
        let this_cross_bits;
        if idx < env.leftmost {
            // check this case once instead of at every iteration
            this_cross_bits = 0;
        } else if this_cross_set.bits != 0 {
            this_cross_bits = this_cross_set.bits;
        } else {
            this_cross_bits = !1;
            is_unique = true;
        }
        let perpendicular_word_multiplier =
            this_premium.word_multiplier as i16 & -(1 & this_cross_set.bits as i16);
        loop {
            let node = env.board_snapshot.kwg[p];
            let tile = node.tile();
            if tile == 0 {
                env.idx_left = idx + 1;
                play_right(
                    env,
                    env.anchor + 1,
                    p,
                    main_score,
                    perpendicular_score,
                    word_multiplier,
                    prev_is_unique,
                );
            } else if this_cross_bits & (1 << tile) != 0 {
                if env.rack_tally[tile as usize] > 0 {
                    env.rack_tally[tile as usize] -= 1;
                    env.num_played += 1;
                    let tile_value = (env.board_snapshot.game_config.alphabet().score(tile) as i16)
                        * (this_premium.tile_multiplier as i16);
                    env.word_buffer[idx as usize] = tile;
                    play_left(
                        env,
                        idx - 1,
                        p,
                        main_score + tile_value,
                        perpendicular_score
                            + (this_cross_set.score + tile_value) * perpendicular_word_multiplier,
                        new_word_multiplier,
                        is_unique,
                    );
                    env.num_played -= 1;
                    env.rack_tally[tile as usize] += 1;
                }
                if env.rack_tally[0] > 0 {
                    env.rack_tally[0] -= 1;
                    env.num_played += 1;
                    // intentional to not hardcode blank tile value as zero
                    let tile_value = (env.board_snapshot.game_config.alphabet().score(0) as i16)
                        * (this_premium.tile_multiplier as i16);
                    env.word_buffer[idx as usize] = tile | 0x80;
                    play_left(
                        env,
                        idx - 1,
                        p,
                        main_score + tile_value,
                        perpendicular_score
                            + (this_cross_set.score + tile_value) * perpendicular_word_multiplier,
                        new_word_multiplier,
                        is_unique,
                    );
                    env.num_played -= 1;
                    env.rack_tally[0] += 1;
                }
            }
            if node.is_end() {
                break;
            }
            p += 1;
        }
    }

    #[inline(always)]
    fn gen_moves_from<CallbackType: FnMut(i8, &[u8], i16, &[u8])>(
        env: &mut Env<CallbackType>,
        single_tile_plays: bool,
    ) {
        play_left(env, env.anchor, 1, 0, 0, 1, single_tile_plays);
    }

    let mut rightmost = len; // processed up to here
    let mut leftmost = len;
    loop {
        while leftmost > 0 && board_snapshot.board_tiles[strider.at(leftmost - 1)] == 0 {
            leftmost -= 1;
        }
        if leftmost > 0 {
            // board[leftmost - 1] is a tile.
            env.anchor = leftmost - 1;
            env.leftmost = 0;
            env.rightmost = rightmost;
            gen_moves_from(&mut env, single_tile_plays);
        }
        {
            // this part is only relevant if rack has at least two tiles, but passing that is too expensive.
            let leftmost = leftmost + (leftmost > 0) as i8; // shadowing
            for anchor in (leftmost..rightmost).rev() {
                let cross_set_bits = cross_set_slice[anchor as usize].bits;
                if cross_set_bits != 0 {
                    if rightmost - leftmost < 2 {
                        // not enough room for 2-tile words
                        break;
                    }
                    if cross_set_bits != 1 {
                        env.anchor = anchor;
                        env.leftmost = leftmost;
                        env.rightmost = rightmost;
                        gen_moves_from(&mut env, single_tile_plays);
                    }
                    rightmost = anchor; // prevent duplicates
                }
            }
        }
        while leftmost > 0 && board_snapshot.board_tiles[strider.at(leftmost - 1)] != 0 {
            leftmost -= 1;
        }
        if leftmost <= 1 {
            break;
        }
        rightmost = leftmost - 1; // prevent touching leftmost tile
    }
}

pub enum Play {
    Exchange {
        tiles: bites::Bites,
    },
    Place {
        down: bool,
        lane: i8,
        idx: i8,
        word: bites::Bites,
        score: i16,
    },
}

impl Clone for Play {
    #[inline(always)]
    fn clone(&self) -> Self {
        match self {
            Self::Exchange { tiles } => Self::Exchange {
                tiles: tiles.clone(),
            },
            Self::Place {
                down,
                lane,
                idx,
                word,
                score,
            } => Self::Place {
                down: *down,
                lane: *lane,
                idx: *idx,
                word: word.clone(),
                score: *score,
            },
        }
    }

    #[inline(always)]
    fn clone_from(&mut self, source: &Self) {
        match self {
            Self::Exchange { tiles: self_tiles } => {
                if let Self::Exchange {
                    tiles: source_tiles,
                } = source
                {
                    self_tiles.clone_from(&source_tiles);
                } else {
                    *self = source.clone();
                }
            }
            Self::Place {
                down: self_down,
                lane: self_lane,
                idx: self_idx,
                word: self_word,
                score: self_score,
            } => {
                if let Self::Place {
                    down: source_down,
                    lane: source_lane,
                    idx: source_idx,
                    word: source_word,
                    score: source_score,
                } = source
                {
                    self_down.clone_from(&source_down);
                    self_lane.clone_from(&source_lane);
                    self_idx.clone_from(&source_idx);
                    self_word.clone_from(&source_word);
                    self_score.clone_from(&source_score);
                } else {
                    *self = source.clone();
                }
            }
        }
    }
}

pub struct ValuedMove {
    pub equity: f32,
    pub play: Play,
}

impl Clone for ValuedMove {
    #[inline(always)]
    fn clone(&self) -> Self {
        Self {
            equity: self.equity,
            play: self.play.clone(),
        }
    }

    #[inline(always)]
    fn clone_from(&mut self, source: &Self) {
        self.equity.clone_from(&source.equity);
        self.play.clone_from(&source.play);
    }
}

impl PartialEq for ValuedMove {
    #[inline(always)]
    fn eq(&self, other: &Self) -> bool {
        other.equity == self.equity
    }
}

impl Eq for ValuedMove {}

impl PartialOrd for ValuedMove {
    #[inline(always)]
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        other.equity.partial_cmp(&self.equity)
    }
}

impl Ord for ValuedMove {
    #[inline(always)]
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        match self.partial_cmp(other) {
            Some(x) => x,
            None => std::cmp::Ordering::Equal,
        }
    }
}

pub struct WriteablePlay<'a> {
    board_snapshot: &'a BoardSnapshot<'a>,
    play: &'a Play,
}

impl std::fmt::Display for WriteablePlay<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match &self.play {
            Play::Exchange { tiles } => {
                if tiles.is_empty() {
                    write!(f, "Pass")?;
                } else {
                    let alphabet = self.board_snapshot.game_config.alphabet();
                    write!(f, "Exch. ")?;
                    for &tile in tiles.iter() {
                        write!(f, "{}", alphabet.from_rack(tile).unwrap())?;
                    }
                }
            }
            Play::Place {
                down,
                lane,
                idx,
                word,
                score,
            } => {
                let dim = self.board_snapshot.game_config.board_layout().dim();
                let alphabet = self.board_snapshot.game_config.alphabet();
                if *down {
                    write!(f, "{}{} ", (*lane as u8 + 0x41) as char, idx + 1)?;
                } else {
                    write!(f, "{}{} ", lane + 1, (*idx as u8 + 0x41) as char)?;
                }
                let strider = if *down {
                    dim.down(*lane)
                } else {
                    dim.across(*lane)
                };
                let mut inside = false;
                for (i, &tile) in (*idx..).zip(word.iter()) {
                    if tile == 0 {
                        if !inside {
                            write!(f, "(")?;
                            inside = true;
                        }
                        write!(
                            f,
                            "{}",
                            alphabet
                                .from_board(self.board_snapshot.board_tiles[strider.at(i)])
                                .unwrap(),
                        )?;
                    } else {
                        if inside {
                            write!(f, ")")?;
                            inside = false;
                        }
                        write!(f, "{}", alphabet.from_board(tile).unwrap())?;
                    }
                }
                if inside {
                    write!(f, ")")?;
                }
                write!(f, " {}", score)?;
            }
        }
        Ok(())
    }
}

impl Play {
    pub fn fmt<'a>(&'a self, board_snapshot: &'a BoardSnapshot) -> WriteablePlay<'a> {
        WriteablePlay {
            board_snapshot: &board_snapshot,
            play: self,
        }
    }
}

pub struct KurniaMoveGenerator {
    working_buffer: WorkingBuffer,
    pub plays: Vec<ValuedMove>,
}

impl Clone for KurniaMoveGenerator {
    #[inline(always)]
    fn clone(&self) -> Self {
        Self {
            working_buffer: self.working_buffer.clone(),
            plays: self.plays.clone(),
        }
    }

    #[inline(always)]
    fn clone_from(&mut self, source: &Self) {
        self.working_buffer.clone_from(&source.working_buffer);
        self.plays.clone_from(&source.plays);
    }
}

impl KurniaMoveGenerator {
    pub fn new(game_config: &game_config::GameConfig) -> Self {
        Self {
            working_buffer: WorkingBuffer::new(game_config),
            plays: Vec::new(),
        }
    }

    // this does not alloc except for growing the results and exchange_buffer
    pub fn gen_moves_alloc<'a>(
        &mut self,
        board_snapshot: &'a BoardSnapshot<'a>,
        rack: &'a [u8],
        max_gen: usize,
    ) {
        let alphabet = board_snapshot.game_config.alphabet();

        let board_layout = board_snapshot.game_config.board_layout();
        let dim = board_layout.dim();

        self.plays.clear();
        let found_moves = std::cell::RefCell::new(std::collections::BinaryHeap::from(
            std::mem::take(&mut self.plays),
        ));

        fn push_move<F: FnMut() -> Play>(
            found_moves: &std::cell::RefCell<std::collections::BinaryHeap<ValuedMove>>,
            max_gen: usize,
            equity: f32,
            mut construct_play: F,
        ) {
            if max_gen == 0 {
                return;
            }
            let mut borrowed = found_moves.borrow_mut();
            if borrowed.len() >= max_gen {
                if borrowed.peek().unwrap().equity >= equity {
                    return;
                }
                borrowed.pop();
            }
            borrowed.push(ValuedMove {
                equity,
                play: construct_play(),
            });
        };

        let mut working_buffer = &mut self.working_buffer;
        working_buffer.init(board_snapshot, rack);
        let num_tiles_on_board = working_buffer.num_tiles_on_board;
        let bag_is_empty = num_tiles_on_board
            + board_snapshot.game_config.num_players() as u16
                * (board_snapshot.game_config.rack_size() as u16)
            >= alphabet.num_tiles();

        let play_out_bonus = if bag_is_empty {
            2 * ((0u8..)
                .zip(working_buffer.rack_tally.iter())
                .map(|(tile, &num)| {
                    (alphabet.freq(tile) as i16 - num as i16) * alphabet.score(tile) as i16
                })
                .sum::<i16>()
                - board_snapshot
                    .board_tiles
                    .iter()
                    .map(|&t| if t != 0 { alphabet.score(t) as i16 } else { 0 })
                    .sum::<i16>())
        } else {
            0
        };

        // temp for debugging
        if bag_is_empty {
            println!("play out bonus: {}", play_out_bonus);
            println!("rack_tally: {:?}", working_buffer.rack_tally);
            println!(
                "freq score: {}",
                (0u8..)
                    .zip(working_buffer.rack_tally.iter())
                    .map(|(tile, &num)| {
                        (alphabet.freq(tile) as i16) * alphabet.score(tile) as i16
                    })
                    .sum::<i16>()
            );
            {
                let mut z = 0;
                for (tile, &num) in (0u8..).zip(working_buffer.rack_tally.iter()) {
                    let tf = alphabet.freq(tile) as i16;
                    let ts = alphabet.score(tile) as i16;
                    let tm = tf * ts;
                    z += tm;
                    //println!("{} * {} = {} total {}", tf, ts, tm,z);
                }
            }
            println!(
                "rack score: {}",
                (0u8..)
                    .zip(working_buffer.rack_tally.iter())
                    .map(|(tile, &num)| { (num as i16) * alphabet.score(tile) as i16 })
                    .sum::<i16>()
            );
            println!(
                "freq score - rack score: {}",
                (0u8..)
                    .zip(working_buffer.rack_tally.iter())
                    .map(|(tile, &num)| {
                        print!(
                            "[{:?} freq={} num={} score={}] ",
                            alphabet.from_rack(tile).unwrap(),
                            alphabet.freq(tile),
                            num,
                            alphabet.score(tile)
                        );
                        (alphabet.freq(tile) as i16 - num as i16) * alphabet.score(tile) as i16
                    })
                    .sum::<i16>()
            );
            println!(
                "board score: {}",
                board_snapshot
                    .board_tiles
                    .iter()
                    .map(|&t| if t != 0 { alphabet.score(t) as i16 } else { 0 })
                    .sum::<i16>()
            );
            println!(
                "board tiles: {:?}",
                board_snapshot
                    .board_tiles
                    .iter()
                    .filter(|&&t| t != 0)
                    .collect::<Vec<_>>()
            );
            println!(
                "freq score - rack score - board score: {}",
                ((0u8..)
                    .zip(working_buffer.rack_tally.iter())
                    .map(|(tile, &num)| {
                        (alphabet.freq(tile) as i16 - num as i16) * alphabet.score(tile) as i16
                    })
                    .sum::<i16>()
                    - board_snapshot
                        .board_tiles
                        .iter()
                        .map(|&t| if t != 0 { alphabet.score(t) as i16 } else { 0 })
                        .sum::<i16>())
            );
        }

        let leave_value_from_tally = |rack_tally: &[u8]| {
            if bag_is_empty {
                0.0
            } else {
                board_snapshot.klv.leave_value_from_tally(rack_tally)
            }
        };

        let found_place_move =
            |down: bool, lane: i8, idx: i8, word: &[u8], score: i16, rack_tally: &[u8]| {
                let leave_value = leave_value_from_tally(rack_tally);
                let other_adjustments = if num_tiles_on_board == 0 {
                    let num_lanes = if down { dim.cols } else { dim.rows };
                    let strider1 = if lane > 0 {
                        Some(if down {
                            dim.down(lane - 1)
                        } else {
                            dim.across(lane - 1)
                        })
                    } else {
                        None
                    };
                    let strider2 = if lane < num_lanes - 1 {
                        Some(if down {
                            dim.down(lane + 1)
                        } else {
                            dim.across(lane + 1)
                        })
                    } else {
                        None
                    };
                    (idx..)
                        .zip(word)
                        .filter(|(i, &tile)| {
                            tile != 0 && alphabet.is_vowel(tile) && {
                                (match strider1 {
                                    Some(strider) => {
                                        let premium = board_layout.premiums()[strider.at(*i)];
                                        premium.tile_multiplier != 1 || premium.word_multiplier != 1
                                    }
                                    None => false,
                                }) || (match strider2 {
                                    Some(strider) => {
                                        let premium = board_layout.premiums()[strider.at(*i)];
                                        premium.tile_multiplier != 1 || premium.word_multiplier != 1
                                    }
                                    None => false,
                                })
                            }
                        })
                        .count() as f32
                        * -0.7
                } else if bag_is_empty {
                    let played_out = rack_tally.iter().all(|&num| num == 0);
                    (if played_out {
                        play_out_bonus
                    } else {
                        -10 - 2
                            * (0u8..)
                                .zip(rack_tally)
                                .map(|(tile, num)| *num as i16 * alphabet.score(tile) as i16)
                                .sum::<i16>()
                    }) as f32
                } else {
                    0.0
                };
                push_move(
                    &found_moves,
                    max_gen,
                    score as f32 + leave_value + other_adjustments,
                    || Play::Place {
                        down,
                        lane,
                        idx,
                        word: word.into(),
                        score,
                    },
                );
            };

        let found_exchange_move = |rack_tally: &[u8], exchanged_tiles: &[u8]| {
            let leave_value = leave_value_from_tally(rack_tally);
            let other_adjustments = if num_tiles_on_board == 0 {
                0.0
            } else if bag_is_empty {
                (-10 - 2
                    * (0u8..)
                        .zip(rack_tally)
                        .map(|(tile, num)| *num as i16 * alphabet.score(tile) as i16)
                        .sum::<i16>()) as f32
            } else {
                0.0
            };
            push_move(
                &found_moves,
                max_gen,
                leave_value + other_adjustments,
                || Play::Exchange {
                    tiles: exchanged_tiles.into(),
                },
            );
        };

        kurnia_gen_nonplace_moves(board_snapshot, &mut working_buffer, found_exchange_move);
        kurnia_gen_place_moves(board_snapshot, &mut working_buffer, found_place_move);

        self.plays = found_moves.into_inner().into_vec();
        self.plays.sort_unstable();
    }
}

fn kurnia_gen_nonplace_moves<'a, FoundExchangeMove: FnMut(&[u8], &[u8])>(
    board_snapshot: &'a BoardSnapshot<'a>,
    working_buffer: &mut WorkingBuffer,
    mut found_exchange_move: FoundExchangeMove,
) {
    working_buffer.exchange_buffer.clear(); // should be no-op
    struct ExchangeEnv<'a, FoundExchangeMove: FnMut(&[u8], &[u8])> {
        found_exchange_move: FoundExchangeMove,
        rack_tally: &'a mut [u8],
        exchange_buffer: &'a mut Vec<u8>,
    }
    fn generate_exchanges<'a, FoundExchangeMove: FnMut(&[u8], &[u8])>(
        env: &mut ExchangeEnv<'a, FoundExchangeMove>,
        mut idx: u8,
    ) {
        let rack_tally_len = env.rack_tally.len();
        while (idx as usize) < rack_tally_len && env.rack_tally[idx as usize] == 0 {
            idx += 1;
        }
        if idx as usize >= rack_tally_len {
            (env.found_exchange_move)(&env.rack_tally, &env.exchange_buffer);
            return;
        }
        let original_count = env.rack_tally[idx as usize];
        let vec_len = env.exchange_buffer.len();
        loop {
            generate_exchanges(env, idx + 1);
            if env.rack_tally[idx as usize] == 0 {
                break;
            }
            env.rack_tally[idx as usize] -= 1;
            env.exchange_buffer.push(idx);
        }
        env.rack_tally[idx as usize] = original_count;
        env.exchange_buffer.truncate(vec_len);
    }
    if working_buffer.num_tiles_on_board
        + (board_snapshot.game_config.num_players() as u16 + 1)
            * (board_snapshot.game_config.rack_size() as u16)
        <= board_snapshot.game_config.alphabet().num_tiles()
    {
        generate_exchanges(
            &mut ExchangeEnv {
                found_exchange_move,
                rack_tally: &mut working_buffer.rack_tally,
                exchange_buffer: &mut working_buffer.exchange_buffer,
            },
            0,
        );
    } else {
        found_exchange_move(&working_buffer.rack_tally, &working_buffer.exchange_buffer);
    }
}

fn kurnia_gen_place_moves<'a, FoundPlaceMove: FnMut(bool, i8, i8, &[u8], i16, &[u8])>(
    board_snapshot: &'a BoardSnapshot<'a>,
    working_buffer: &mut WorkingBuffer,
    mut found_place_move: FoundPlaceMove,
) {
    let game_config = &board_snapshot.game_config;
    let board_layout = game_config.board_layout();
    let dim = board_layout.dim();
    let max_rack_size = game_config.rack_size() as u8;

    // striped by row
    for col in 0..dim.cols {
        let cached_cross_set_start = ((col as isize) * (dim.rows as isize)) as usize;
        gen_cross_set(
            &board_snapshot,
            dim.down(col),
            &mut working_buffer.cross_set_for_across_plays,
            dim.down(col),
            &mut working_buffer.cross_set_buffer,
            &mut working_buffer.cached_cross_set_for_across_plays
                [cached_cross_set_start..cached_cross_set_start + (dim.rows as usize)],
        );
    }
    if working_buffer.num_tiles_on_board == 0 {
        // empty board activates star
        working_buffer.cross_set_for_across_plays
            [dim.at_row_col(board_layout.star_row(), board_layout.star_col())] =
            CrossSet { bits: !1, score: 0 };
    }
    for row in 0..dim.rows {
        let cross_set_start = ((row as isize) * (dim.cols as isize)) as usize;
        gen_place_moves(
            &board_snapshot,
            &working_buffer.cross_set_for_across_plays
                [cross_set_start..cross_set_start + (dim.cols as usize)],
            &mut working_buffer.rack_tally,
            dim.across(row),
            &mut working_buffer.word_buffer,
            max_rack_size,
            true,
            |idx: i8, word: &[u8], score: i16, rack_tally: &[u8]| {
                found_place_move(false, row, idx, word, score, rack_tally)
            },
        );
    }
    let transposed_dim = matrix::Dim {
        rows: dim.cols,
        cols: dim.rows,
    };
    // striped by columns for better cache locality
    for row in 0..dim.rows {
        let cached_cross_set_start = ((row as isize) * (dim.cols as isize)) as usize;
        gen_cross_set(
            &board_snapshot,
            dim.across(row),
            &mut working_buffer.cross_set_for_down_plays,
            transposed_dim.down(row),
            &mut working_buffer.cross_set_buffer,
            &mut working_buffer.cached_cross_set_for_down_plays
                [cached_cross_set_start..cached_cross_set_start + (dim.cols as usize)],
        );
    }
    if !board_layout.is_symmetric() && working_buffer.num_tiles_on_board == 0 {
        // empty board activates star
        working_buffer.cross_set_for_down_plays
            [transposed_dim.at_row_col(board_layout.star_col(), board_layout.star_row())] =
            CrossSet { bits: !1, score: 0 };
    }
    for col in 0..dim.cols {
        let cross_set_start = ((col as isize) * (dim.rows as isize)) as usize;
        gen_place_moves(
            &board_snapshot,
            &working_buffer.cross_set_for_down_plays
                [cross_set_start..cross_set_start + (dim.rows as usize)],
            &mut working_buffer.rack_tally,
            dim.down(col),
            &mut working_buffer.word_buffer,
            max_rack_size,
            false,
            |idx: i8, word: &[u8], score: i16, rack_tally: &[u8]| {
                found_place_move(true, col, idx, word, score, rack_tally)
            },
        );
    }
}
