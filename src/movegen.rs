use super::{board_layout, game_config, gdw, matrix};

#[derive(Clone)]
struct CrossSet {
    bits: u64,
    score: i16,
}

struct WorkingBuffer {
    rack_tally: Box<[u8]>,                       // 27 for ?A-Z
    word_vec: Box<[u8]>,                         // max(r, c)
    cross_set_for_across_plays: Box<[CrossSet]>, // r*c
    cross_set_for_down_plays: Box<[CrossSet]>,   // c*r
}

impl WorkingBuffer {
    fn new(game_config: &game_config::GameConfig) -> Box<Self> {
        let dim = game_config.board_layout().dim();
        let rows_times_cols = ((dim.rows as isize) * (dim.cols as isize)) as usize;
        Box::new(Self {
            rack_tally: vec![0u8; game_config.alphabet().len() as usize].into_boxed_slice(),
            word_vec: vec![0u8; std::cmp::max(dim.rows, dim.cols) as usize].into_boxed_slice(),
            cross_set_for_across_plays: vec![CrossSet { bits: 0, score: 0 }; rows_times_cols]
                .into_boxed_slice(),
            cross_set_for_down_plays: vec![CrossSet { bits: 0, score: 0 }; rows_times_cols]
                .into_boxed_slice(),
        })
    }
}

fn gen_cross_set<'a>(
    board_tiles: &'a [u8],
    game_config: &'a game_config::GameConfig<'a>,
    gdw: &'a gdw::Gdw,
    strider: matrix::Strider,
    cross_sets: &'a mut [CrossSet],
    output_strider: matrix::Strider,
) {
    let len = strider.len();
    for i in 0..output_strider.len() {
        cross_sets[output_strider.at(i)] = CrossSet { bits: 0, score: 0 };
    }

    let alphabet = game_config.alphabet();
    let mut p = 1;
    let mut score = 0i16;
    let mut k = len;
    for j in (0..len).rev() {
        let b = board_tiles[strider.at(j)];
        if b != 0 {
            // board has tile
            if p >= 0 {
                // include current tile
                p = gdw.in_gdw(p, b & 0x7f);
            }
            score += alphabet.get(if b & 0x80 == 0 { b } else { 0 }).score as i16;
            if j == 0 || board_tiles[strider.at(j - 1)] == 0 {
                // there is a sequence of tiles from j inclusive to k exclusive
                if k < len && !(k + 1 < len && board_tiles[strider.at(k + 1)] != 0) {
                    // board[k + 1] is empty, compute cross_set[k].
                    let mut bits = 1u64;
                    if p > 0 {
                        // p = DCBA
                        let q = gdw.in_gdw(p, 0);
                        if q > 0 {
                            // q = DCBA@
                            let mut q = gdw[q].arc_index();
                            if q > 0 {
                                loop {
                                    if gdw[q].accepts() {
                                        bits |= 1 << gdw[q].tile();
                                    }
                                    if gdw[q].is_end() {
                                        break;
                                    }
                                    q += 1;
                                }
                            }
                        }
                    }
                    cross_sets[output_strider.at(k)] = CrossSet { bits, score };
                }
                if j > 0 {
                    // board[j - 1] is known to be empty
                    let mut bits = 1u64;
                    if p > 0 {
                        // p = DCBA
                        p = gdw[p].arc_index(); // p = after DCBA
                        if p > 0 {
                            loop {
                                let tile = gdw[p].tile();
                                if tile != 0 {
                                    // not the gaddag marker
                                    let mut q = p;
                                    // board[j - 2] may or may not be empty.
                                    for k in (0..j - 1).rev() {
                                        let b = board_tiles[strider.at(k)];
                                        if b == 0 {
                                            break;
                                        }
                                        q = gdw.in_gdw(q, b & 0x7f);
                                        if q <= 0 {
                                            break;
                                        }
                                    }
                                    if q > 0 && gdw[q].accepts() {
                                        bits |= 1 << gdw[q].tile();
                                    }
                                }
                                if gdw[p].is_end() {
                                    break;
                                }
                                p += 1;
                            }
                        }
                    }
                    // score hasn't included the next batch.
                    for k in (0i8..j - 1).rev() {
                        let b = board_tiles[strider.at(k)];
                        if b == 0 {
                            break;
                        }
                        score += alphabet.get(if b & 0x80 == 0 { b } else { 0 }).score as i16;
                    }
                    cross_sets[output_strider.at(j - 1)] = CrossSet { bits, score };
                }
            }
        } else {
            // empty square, reset
            p = 1; // cumulative gaddag traversal results
            score = 0; // cumulative face-value score
            k = j; // last seen empty square
        }
    }
}

// word_buffer must have at least strider.len() length.
#[allow(clippy::too_many_arguments)]
fn gen_place_moves<'a, CallbackType: FnMut(i8, &[u8], i16, &[u8])>(
    board_tiles: &'a [u8],
    game_config: &'a game_config::GameConfig<'a>,
    gdw: &'a gdw::Gdw,
    cross_set_slice: &'a [CrossSet],
    rack_tally: &'a mut [u8],
    strider: matrix::Strider,
    word_buffer: &'a mut [u8],
    single_tile_plays: bool,
    callback: CallbackType,
) {
    let len = strider.len();

    struct Env<'a, CallbackType: FnMut(i8, &[u8], i16, &[u8])> {
        board_tiles: &'a [u8],
        game_config: &'a game_config::GameConfig<'a>,
        gdw: &'a gdw::Gdw,
        cross_set_slice: &'a [CrossSet],
        rack_tally: &'a mut [u8],
        strider: matrix::Strider,
        callback: CallbackType,
        word_buffer: &'a mut [u8],
        anchor: i8,
        leftmost: i8,
        rightmost: i8,
        num_played: i8,
        idx_left: i8,
    }

    let mut env = Env {
        board_tiles,
        game_config,
        gdw,
        cross_set_slice,
        rack_tally,
        strider,
        callback,
        word_buffer,
        anchor: 0,
        leftmost: 0,
        rightmost: 0,
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
            + if env.num_played >= 7 { 50 } else { 0 };
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
            let b = env.board_tiles[env.strider.at(idx)];
            if b == 0 {
                break;
            }
            p = env.gdw.in_gdw(p, b & 0x7f);
            if p <= 0 {
                return;
            }
            main_score += env
                .game_config
                .alphabet()
                .get(if b & 0x80 == 0 { b } else { 0 })
                .score as i16;
            env.word_buffer[idx as usize] = 0;
            idx += 1;
        }
        if idx > env.anchor + 1
            && (env.num_played + is_unique as i8) >= 2
            && idx - env.idx_left >= 2
            && env.gdw[p].accepts()
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

        p = env.gdw[p].arc_index();
        if p <= 0 {
            return;
        }
        let mut this_premium = board_layout::Premium {
            word_multiplier: 0,
            tile_multiplier: 0,
        };
        let mut this_cross_set = CrossSet { bits: 0, score: 0 };
        if idx < env.rightmost {
            this_premium = env.game_config.board_layout().premiums()[env.strider.at(idx)];
            this_cross_set = env.cross_set_slice[idx as usize].clone();
        }
        if this_cross_set.bits == 1 {
            // already handled '@'
            return;
        }
        let new_word_multiplier = word_multiplier * this_premium.word_multiplier;
        let this_cross_bits = if this_cross_set.bits != 0 {
            this_cross_set.bits
        } else {
            is_unique = true;
            !1
        };
        loop {
            let tile = env.gdw[p].tile();
            if tile != 0 && this_cross_bits & (1 << tile) != 0 {
                if env.rack_tally[tile as usize] > 0 {
                    env.rack_tally[tile as usize] -= 1;
                    env.num_played += 1;
                    let tile_value = (env.game_config.alphabet().get(tile).score as i16)
                        * (this_premium.tile_multiplier as i16);
                    env.word_buffer[idx as usize] = tile;
                    play_right(
                        env,
                        idx + 1,
                        p,
                        main_score + tile_value,
                        if this_cross_set.bits != 0 {
                            perpendicular_score
                                + (this_cross_set.score + tile_value)
                                    * (this_premium.word_multiplier as i16)
                        } else {
                            perpendicular_score
                        },
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
                    let tile_value = (env.game_config.alphabet().get(0).score as i16)
                        * (this_premium.tile_multiplier as i16);
                    env.word_buffer[idx as usize] = tile | 0x80;
                    play_right(
                        env,
                        idx + 1,
                        p,
                        main_score + tile_value,
                        if this_cross_set.bits != 0 {
                            perpendicular_score
                                + (this_cross_set.score + tile_value)
                                    * (this_premium.word_multiplier as i16)
                        } else {
                            perpendicular_score
                        },
                        new_word_multiplier,
                        is_unique,
                    );
                    env.num_played -= 1;
                    env.rack_tally[0] += 1;
                }
            }
            if env.gdw[p].is_end() {
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
            let b = env.board_tiles[env.strider.at(idx)];
            if b == 0 {
                break;
            }
            p = env.gdw.in_gdw(p, b & 0x7f);
            if p <= 0 {
                return;
            }
            main_score += env
                .game_config
                .alphabet()
                .get(if b & 0x80 == 0 { b } else { 0 })
                .score as i16;
            env.word_buffer[idx as usize] = 0;
            idx -= 1;
        }
        if (env.num_played + is_unique as i8) >= 2 && env.anchor - idx >= 2 && env.gdw[p].accepts()
        {
            record(
                env,
                idx + 1,
                env.anchor + 1,
                main_score,
                perpendicular_score,
                word_multiplier,
            );
        }

        p = env.gdw[p].arc_index();
        if p <= 0 {
            return;
        }
        let mut this_premium = board_layout::Premium {
            word_multiplier: 0,
            tile_multiplier: 0,
        };
        let mut this_cross_set = CrossSet { bits: 0, score: 0 };
        if idx >= env.leftmost {
            this_premium = env.game_config.board_layout().premiums()[env.strider.at(idx)];
            this_cross_set = env.cross_set_slice[idx as usize].clone();
        }
        let new_word_multiplier = word_multiplier * this_premium.word_multiplier;
        let this_cross_bits = if this_cross_set.bits != 0 {
            this_cross_set.bits
        } else {
            is_unique = true;
            !1
        };
        loop {
            let tile = env.gdw[p].tile();
            if tile == 0 {
                env.idx_left = idx + 1;
                play_right(
                    env,
                    env.anchor + 1,
                    p,
                    main_score,
                    perpendicular_score,
                    word_multiplier,
                    is_unique,
                );
            } else if idx >= env.leftmost && this_cross_bits & (1 << tile) != 0 {
                if env.rack_tally[tile as usize] > 0 {
                    env.rack_tally[tile as usize] -= 1;
                    env.num_played += 1;
                    let tile_value = (env.game_config.alphabet().get(tile).score as i16)
                        * (this_premium.tile_multiplier as i16);
                    env.word_buffer[idx as usize] = tile;
                    play_left(
                        env,
                        idx - 1,
                        p,
                        main_score + tile_value,
                        if this_cross_set.bits != 0 {
                            perpendicular_score
                                + (this_cross_set.score + tile_value)
                                    * (this_premium.word_multiplier as i16)
                        } else {
                            perpendicular_score
                        },
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
                    let tile_value = (env.game_config.alphabet().get(0).score as i16)
                        * (this_premium.tile_multiplier as i16);
                    env.word_buffer[idx as usize] = tile | 0x80;
                    play_left(
                        env,
                        idx - 1,
                        p,
                        main_score + tile_value,
                        if this_cross_set.bits != 0 {
                            perpendicular_score
                                + (this_cross_set.score + tile_value)
                                    * (this_premium.word_multiplier as i16)
                        } else {
                            perpendicular_score
                        },
                        new_word_multiplier,
                        is_unique,
                    );
                    env.num_played -= 1;
                    env.rack_tally[0] += 1;
                }
            }
            if env.gdw[p].is_end() {
                break;
            }
            p += 1;
        }
    }

    fn gen_moves_from<CallbackType: FnMut(i8, &[u8], i16, &[u8])>(
        env: &mut Env<CallbackType>,
        single_tile_plays: bool,
    ) {
        play_left(env, env.anchor, 1, 0, 0, 1, single_tile_plays);
    }

    let mut rightmost = len; // processed up to here
    let mut leftmost = len;
    loop {
        while leftmost > 0 && board_tiles[strider.at(leftmost - 1)] == 0 {
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
            let mut leftmost = leftmost; // shadowing
            if leftmost > 0 {
                leftmost += 1;
            }
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
        while leftmost > 0 && board_tiles[strider.at(leftmost - 1)] != 0 {
            leftmost -= 1;
        }
        if leftmost <= 1 {
            break;
        }
        rightmost = leftmost - 1; // prevent touching leftmost tile
    }
}

pub fn gen_moves<'a>(
    board_tiles: &'a [u8],
    game_config: &'a game_config::GameConfig<'a>,
    gdw: &'a gdw::Gdw,
    rack: &'a mut [u8],
) {
    let mut num_moves = 0; // TODO

    rack.sort_unstable();
    let alphabet = game_config.alphabet();

    let board_layout = game_config.board_layout();
    let dim = board_layout.dim();

    let print_leave = |rack_tally: &[u8]| {
        // rack should be pre-sorted, eg ??EGSUU.
        // rack_tally excludes played tiles.
        print!(" / played: ");
        let mut i = 0;
        while i < rack.len() {
            let tile = rack[i];
            i += rack_tally[tile as usize] as usize;
            while i < rack.len() && rack[i] == tile {
                print!("{}", alphabet.from_board(tile).unwrap_or("?"));
                i += 1;
            }
        }
        print!(" / kept: ");
        let mut i = 0;
        while i < rack.len() {
            let tile = rack[i];
            for _ in 0..rack_tally[tile as usize] {
                print!("{}", alphabet.from_board(tile).unwrap_or("?"));
            }
            i += rack_tally[tile as usize] as usize;
            while i < rack.len() && rack[i] == tile {
                i += 1;
            }
        }
    };

    {
        let mut working_buffer = WorkingBuffer::new(game_config);
        for tile in &rack[..] {
            working_buffer.rack_tally[*tile as usize] += 1;
        }

        let num_tiles_on_board = board_tiles.iter().filter(|&t| *t != 0).count() as usize;

        struct ExchangeEnv<'a> {
            print_leave: &'a dyn Fn(&[u8]),
            rack: &'a [u8],
            rack_tally: &'a mut [u8],
        }
        fn generate_exchanges<'a>(env: &mut ExchangeEnv<'a>, mut idx: u8) {
            // TODO: suboptimal when using leave values
            if (idx as usize) < env.rack.len() {
                let tile = env.rack[idx as usize];
                let available = env.rack_tally[tile as usize];
                idx += available;
                for exchanged in (0..available + 1).rev() {
                    env.rack_tally[tile as usize] = available - exchanged;
                    generate_exchanges(env, idx);
                }
                env.rack_tally[tile as usize] = available;
            } else {
                // found it. (note: pass = xchg nothing)
                print!("xchg:");
                (env.print_leave)(&env.rack_tally);
                println!();
            }
        }
        // 100 tiles, 7 goes to oppo, 7 goes to me, 7 in bag = 79.
        if num_tiles_on_board <= 79 {
            generate_exchanges(
                &mut ExchangeEnv {
                    print_leave: &print_leave,
                    rack: &rack,
                    rack_tally: &mut working_buffer.rack_tally,
                },
                0,
            );
        } else {
            println!("pass");
        }

        // striped by row
        for col in 0..dim.cols {
            gen_cross_set(
                board_tiles,
                game_config,
                &gdw,
                dim.down(col),
                &mut working_buffer.cross_set_for_across_plays,
                matrix::Strider {
                    base: col as i16,
                    step: dim.cols,
                    len: dim.rows,
                },
            );
        }
        if num_tiles_on_board == 0 {
            // empty board activates star
            working_buffer.cross_set_for_across_plays[board_layout
                .dim()
                .at_row_col(board_layout.star_row(), board_layout.star_col())] =
                CrossSet { bits: !1, score: 0 };
        }
        for row in 0..dim.rows {
            let cross_set_start = ((row as isize) * (dim.cols as isize)) as usize;
            gen_place_moves(
                board_tiles,
                game_config,
                &gdw,
                &working_buffer.cross_set_for_across_plays
                    [cross_set_start..cross_set_start + (dim.cols as usize)],
                &mut working_buffer.rack_tally,
                dim.across(row),
                &mut working_buffer.word_vec,
                true,
                |idx: i8, word: &[u8], score: i16, rack_tally: &[u8]| {
                    num_moves += 1;
                    let strider = dim.across(row);
                    print!("{}{} ", row + 1, (idx as u8 + 0x61) as char);
                    let mut inside = false;
                    for i in 0..word.len() {
                        let w = word[i];
                        if w == 0 {
                            if !inside {
                                print!("(");
                                inside = true;
                            }
                            print!(
                                "{}",
                                alphabet
                                    .from_board(board_tiles[strider.at(idx + (i as i8))])
                                    .unwrap()
                            );
                        } else {
                            if inside {
                                print!(")");
                                inside = false;
                            }
                            print!("{}", alphabet.from_board(w).unwrap());
                        }
                    }
                    if inside {
                        print!(")");
                    }
                    print!(" {}", score);
                    print_leave(rack_tally);
                    println!(" ({})", num_moves);
                },
            );
        }
        // striped by columns for better cache locality
        for row in 0..dim.rows {
            gen_cross_set(
                board_tiles,
                game_config,
                &gdw,
                dim.across(row),
                &mut working_buffer.cross_set_for_down_plays,
                matrix::Strider {
                    base: row as i16,
                    step: dim.rows,
                    len: dim.cols,
                },
            );
        }
        for col in 0..dim.cols {
            let cross_set_start = ((col as isize) * (dim.rows as isize)) as usize;
            gen_place_moves(
                board_tiles,
                game_config,
                &gdw,
                &working_buffer.cross_set_for_down_plays
                    [cross_set_start..cross_set_start + (dim.rows as usize)],
                &mut working_buffer.rack_tally,
                dim.down(col),
                &mut working_buffer.word_vec,
                false,
                |idx: i8, word: &[u8], score: i16, rack_tally: &[u8]| {
                    num_moves += 1;
                    let strider = dim.down(col);
                    print!("{}{} ", (col as u8 + 0x61) as char, idx + 1);
                    let mut inside = false;
                    for i in 0..word.len() {
                        let w = word[i];
                        if w == 0 {
                            if !inside {
                                print!("(");
                                inside = true;
                            }
                            print!(
                                "{}",
                                alphabet
                                    .from_board(board_tiles[strider.at(idx + (i as i8))])
                                    .unwrap()
                            );
                        } else {
                            if inside {
                                print!(")");
                                inside = false;
                            }
                            print!("{}", alphabet.from_board(w).unwrap());
                        }
                    }
                    if inside {
                        print!(")");
                    }
                    print!(" {}", score);
                    print_leave(rack_tally);
                    println!(" ({})", num_moves);
                },
            );
        }
    }

    // todo: leaves.
}