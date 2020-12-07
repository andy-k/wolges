mod alphabet;
mod board_layout;
mod display;
mod game_config;
mod gdw;
mod matrix;

fn print_dawg<'a>(a: &alphabet::Alphabet<'a>, g: &gdw::Gdw) {
    struct Env<'a> {
        a: &'a alphabet::Alphabet<'a>,
        g: &'a gdw::Gdw,
        s: &'a mut String,
    }
    fn iter(env: &mut Env, mut p: i32) {
        let l = env.s.len();
        loop {
            let t = env.g[p].tile();
            env.s.push_str(if t == 0 {
                "@"
            } else if t & 0x80 == 0 {
                env.a.from_board(t).unwrap()
            } else {
                panic!()
            });
            if env.g[p].accepts() {
                println!("{}", env.s);
            }
            if env.g[p].arc_index() != 0 {
                iter(env, env.g[p].arc_index());
            }
            env.s.truncate(l);
            if env.g[p].is_end() {
                break;
            }
            p += 1;
        }
    }
    iter(
        &mut Env {
            a: &a,
            g: &g,
            s: &mut String::new(),
        },
        g[0i32].arc_index(),
    );
}

fn print_board<'a>(game_config: &game_config::GameConfig<'a>, board_tiles: &[u8]) {
    let alphabet = game_config.alphabet();
    let board_layout = game_config.board_layout();
    print!("  ");
    for c in 0..board_layout.dim().cols {
        print!(" {}", ((c as u8) + 0x61) as char);
    }
    println!();
    print!("  +");
    for _ in 1..board_layout.dim().cols {
        print!("--");
    }
    println!("-+");
    for r in 0..board_layout.dim().rows {
        print!("{:2}|", r + 1);
        for c in 0..board_layout.dim().cols {
            if c > 0 {
                print!(" ")
            }
            print!(
                "{}",
                display::board_label(alphabet, board_layout, board_tiles, r, c)
            );
        }
        println!("|{}", r + 1);
    }
    print!("  +");
    for _ in 1..board_layout.dim().cols {
        print!("--");
    }
    println!("-+");
    print!("  ");
    for c in 0..board_layout.dim().cols {
        print!(" {}", ((c as u8) + 0x61) as char);
    }
    println!();
}

#[derive(Clone)]
struct CrossSet {
    bits: u64,
    score: i16,
}

struct Tally(pub Box<[u8]>);

impl Tally {
    // length should include blank (so, 27 for ?A-Z).
    fn new(alphabet_len: u8) -> Tally {
        Tally(vec![0u8; alphabet_len as usize].into_boxed_slice())
    }

    fn clear(&mut self) {
        self.0.iter_mut().for_each(|m| *m = 0);
    }

    fn add_all(&mut self, tiles: &[u8]) {
        for t in tiles {
            self.0[*t as usize] += 1;
        }
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
    let len_usize = len as usize;
    for i in 0..output_strider.len() {
        cross_sets[output_strider.at(i)] = CrossSet { bits: 0, score: 0 };
    }
    if false {
        assert_eq!(strider.len(), output_strider.len());
        print!("generating cross set for [");
        for i in 0..len {
            print!(
                " {}",
                game_config
                    .alphabet()
                    .from_board(board_tiles[strider.at(i)])
                    .unwrap_or(".")
            );
        }
        println!(" ]...");
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

    if false {
        for i in 0..len_usize {
            let w = &cross_sets[output_strider.at(i as i8)];
            if w.bits != 0 || w.score != 0 {
                print!(
                    "[{:2}@{:3}] bits={:064b} score={}",
                    i,
                    output_strider.at(i as i8),
                    w.bits,
                    w.score
                );
                for t in 0..63 {
                    if w.bits & (1 << t) != 0 {
                        print!(" {}", alphabet.from_board(t).unwrap_or("."));
                    }
                }
                println!();
            }
        }
    }
}

// rack must be sorted, and rack_tally must tally.
// work_buf must have at least strider.len() length.
fn gen_moves<'a, CallbackType: FnMut(i8, &[u8], i16)>(
    board_tiles: &'a [u8],
    game_config: &'a game_config::GameConfig<'a>,
    gdw: &'a gdw::Gdw,
    cross_set_slice: &'a [CrossSet],
    rack: &'a [u8],
    rack_tally: &'a mut Tally,
    rack_bits: u64,
    strider: matrix::Strider,
    work_buf: &'a mut [u8],
    callback: CallbackType,
) {
    let len = strider.len();
    let len_usize = len as usize;
    if true {
        assert_eq!(strider.len() as usize, cross_set_slice.len());
        print!("using cross set for [");
        for i in 0..len {
            print!(
                " {}",
                game_config
                    .alphabet()
                    .from_board(board_tiles[strider.at(i)])
                    .unwrap_or(".")
            );
        }
        println!(" ]...");
    }

    let alphabet = game_config.alphabet();

    if true {
        for i in 0..len_usize {
            let w = &cross_set_slice[i];
            if w.bits != 0 || w.score != 0 {
                print!("[{:2}] bits={:064b} score={}", i, w.bits, w.score);
                for t in 0..63 {
                    if w.bits & (1 << t) != 0 {
                        print!(" {}", alphabet.from_board(t).unwrap_or("."));
                    }
                }
                println!();
            }
        }
    }

    struct Env<'a, CallbackType: FnMut(i8, &[u8], i16)> {
        board_tiles: &'a [u8],
        game_config: &'a game_config::GameConfig<'a>,
        gdw: &'a gdw::Gdw,
        cross_set_slice: &'a [CrossSet],
        rack: &'a [u8],
        rack_tally: &'a mut Tally,
        strider: matrix::Strider,
        callback: CallbackType,
        work_buf: &'a mut [u8],
        alphabet: &'a alphabet::Alphabet<'a>,
        board_layout: &'a board_layout::BoardLayout<'a>,
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
        rack,
        rack_tally,
        strider,
        callback,
        work_buf,
        alphabet: game_config.alphabet(),
        board_layout: game_config.board_layout(),
        anchor: 0,
        leftmost: 0,
        rightmost: 0,
        num_played: 0,
        idx_left: 0,
    };

    fn record<'a, CallbackType: FnMut(i8, &[u8], i16)>(
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
            &env.work_buf[(idx_left as usize)..(idx_right as usize)],
            score,
        );
    }

    fn play_right<'a, CallbackType: FnMut(i8, &[u8], i16)>(
        env: &mut Env<CallbackType>,
        mut idx: i8,
        mut p: i32,
        mut main_score: i16,
        perpendicular_score: i16,
        word_multiplier: i8,
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
            main_score += env.alphabet.get(if b & 0x80 == 0 { b } else { 0 }).score as i16;
            idx += 1;
        }
        // TODO: min_num_played dedup
        if idx > env.anchor + 1
            && env.num_played > 0
            && env.anchor - idx >= 2
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
            this_premium = env.board_layout.premiums()[env.strider.at(idx)];
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
            !1
        };
        loop {
            let tile = env.gdw[p].tile();
            if tile != 0 && this_cross_bits & (1 << tile) != 0 {
                if env.rack_tally.0[tile as usize] > 0 {
                    env.rack_tally.0[tile as usize] -= 1;
                    env.num_played += 1;
                    let tile_value = (env.alphabet.get(tile).score as i16)
                        * (this_premium.tile_multiplier as i16);
                    env.work_buf[idx as usize] = tile;
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
                    );
                    env.num_played -= 1;
                    env.rack_tally.0[tile as usize] += 1;
                }
                if env.rack_tally.0[0] > 0 {
                    env.rack_tally.0[0] -= 1;
                    env.num_played += 1;
                    // intentional to not hardcode blank tile value as zero
                    let tile_value =
                        (env.alphabet.get(0).score as i16) * (this_premium.tile_multiplier as i16);
                    env.work_buf[idx as usize] = tile | 0x80;
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
                    );
                    env.num_played -= 1;
                    env.rack_tally.0[0] += 1;
                }
            }
            if env.gdw[p].is_end() {
                break;
            }
            p += 1;
        }
    }

    fn play_left<'a, CallbackType: FnMut(i8, &[u8], i16)>(
        env: &mut Env<CallbackType>,
        mut idx: i8,
        mut p: i32,
        mut main_score: i16,
        perpendicular_score: i16,
        word_multiplier: i8,
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
            main_score += env.alphabet.get(if b & 0x80 == 0 { b } else { 0 }).score as i16;
            idx -= 1;
        }
        // TODO: min_num_played dedup
        if env.num_played > 0 && env.anchor - idx >= 2 && env.gdw[p].accepts() {
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
            this_premium = env.board_layout.premiums()[env.strider.at(idx)];
            this_cross_set = env.cross_set_slice[idx as usize].clone();
        }
        let new_word_multiplier = word_multiplier * this_premium.word_multiplier;
        let this_cross_bits = if this_cross_set.bits != 0 {
            this_cross_set.bits
        } else {
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
                );
            } else if idx >= env.leftmost && this_cross_bits & (1 << tile) != 0 {
                if env.rack_tally.0[tile as usize] > 0 {
                    env.rack_tally.0[tile as usize] -= 1;
                    env.num_played += 1;
                    let tile_value = (env.alphabet.get(tile).score as i16)
                        * (this_premium.tile_multiplier as i16);
                    env.work_buf[idx as usize] = tile;
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
                    );
                    env.num_played -= 1;
                    env.rack_tally.0[tile as usize] += 1;
                }
                if env.rack_tally.0[0] > 0 {
                    env.rack_tally.0[0] -= 1;
                    env.num_played += 1;
                    // intentional to not hardcode blank tile value as zero
                    let tile_value =
                        (env.alphabet.get(0).score as i16) * (this_premium.tile_multiplier as i16);
                    env.work_buf[idx as usize] = tile | 0x80;
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
                    );
                    env.num_played -= 1;
                    env.rack_tally.0[0] += 1;
                }
            }
            if env.gdw[p].is_end() {
                break;
            }
            p += 1;
        }
    }

    fn gen_moves_from<'a, CallbackType: FnMut(i8, &[u8], i16)>(env: &mut Env<CallbackType>) {
        println!("moves({}, {}..{})", env.anchor, env.leftmost, env.rightmost);
        play_left(env, env.anchor, 1, 0, 0, 1);
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
            gen_moves_from(&mut env);
        }
        if rack.len() >= 2 {
            let mut leftmost = leftmost; // shadowing
            if leftmost > 0 {
                leftmost += 1;
            }
            for anchor in (leftmost..rightmost).rev() {
                let cross_set_bits = cross_set_slice[anchor as usize].bits;
                if cross_set_bits != 0 {
                    if rightmost - leftmost < 2 {
                        // not enough room for 2-letter words
                        break;
                    }
                    if (cross_set_bits & rack_bits) != 1 {
                        // only if something on rack might fit
                        env.anchor = anchor;
                        env.leftmost = leftmost;
                        env.rightmost = rightmost;
                        gen_moves_from(&mut env);
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

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let gdw = gdw::from_bytes(&std::fs::read("csw19.gdw")?);
    let game_config = &game_config::COMMON_ENGLISH_GAME_CONFIG;
    let mut tally = Tally::new(game_config.alphabet().len());
    println!("{:?}", tally.0);
    tally.add_all(b"\x1a\x19\x00\x00\x19\x16\x01");
    println!("{:?}", tally.0);
    tally.add_all(b"\x1a\x19\x00\x00\x19\x16\x01");
    println!("{:?}", tally.0);
    tally.0[3] += 4;
    println!("{:?}", tally.0);
    tally.clear();
    println!("{:?}", tally.0);
    if false {
        return Ok(());
    }
    if false {
        print_dawg(game_config.alphabet(), &gdw);
        println!("{}", gdw.0.len());
    }

    // mut because of additional test case
    let mut board_tiles = b"\
\x0f\x04\x00\x00\x00\x00\x08\x01\x12\x0c\x0f\x14\x13\x00\x00\
\x06\x09\x0e\x00\x00\x00\x00\x00\x00\x00\x00\x17\x00\x00\x00\
\x00\x14\x05\x05\x00\x07\x00\x00\x00\x00\x00\x09\x00\x00\x00\
\x00\x1a\x01\x18\x00\x12\x00\x00\x00\x00\x00\x03\x00\x00\x00\
\x00\x00\x14\x0f\x12\x09\x00\x00\x03\x00\x04\x05\x00\x00\x00\
\x00\x00\x0c\x00\x01\x0d\x00\x14\x15\x0e\x01\x00\x00\x00\x00\
\x00\x00\x19\x05\x0e\x00\x11\x09\x0e\x00\x08\x00\x00\x00\x00\
\x00\x00\x00\x16\x09\x02\x09\x13\x14\x00\x0c\x00\x00\x00\x00\
\x00\x00\x00\x05\x00\x00\x00\x00\x00\x00\x09\x00\x00\x00\x00\
\x00\x00\x00\x0a\x01\x19\x00\x00\x00\x0e\x01\x00\x00\x00\x00\
\x00\x00\x00\x01\x0d\x05\x00\x00\x06\x01\x13\x00\x00\x00\x00\
\x00\x00\x00\x12\x05\x10\x00\x12\x15\x0e\x00\x00\x00\x00\x00\
\x00\x00\x0f\x00\x02\x00\x00\x00\x07\x00\x00\x00\x00\x00\x00\
\x00\x00\x17\x12\x01\x10\x00\x00\x00\x00\x00\x00\x00\x00\x00\
\x0f\x0b\x05\x00\x05\x09\x04\x05\x00\x00\x00\x00\x00\x00\x00\
";

    print_board(game_config, board_tiles);

    if false {
        board_tiles = b"\
\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\
\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\
\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\
\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\
\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\
\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\
\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\
\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\
\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\
\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\
\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\
\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\
\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\
\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\
\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\
";

        print_board(game_config, board_tiles);
    }

    // todo: check for empty board, etc.
    {
        // ?UGE?US - http://liwords.localhost/game/oyMkFGLA
        let mut rack = *b"\x00\x15\x07\x05\x00\x15\x13";
        rack.sort_unstable();
        let alphabet = game_config.alphabet();
        for &tile in rack.iter() {
            print!(
                "{}",
                if tile == 0 {
                    "?"
                } else if tile & 0x80 == 0 {
                    alphabet.from_board(tile).unwrap()
                } else {
                    panic!()
                }
            );
        }
        println!();

        let board_layout = game_config.board_layout();
        let dim = board_layout.dim();
        let mut rack_tally = Tally::new(game_config.alphabet().len());
        rack_tally.add_all(&rack);
        println!("{:?}", rack_tally.0);

        // rack_bits = 1 | whatever_else
        let mut rack_bits = 1u64;
        if rack_tally.0[0] != 0 {
            rack_bits = !0; // blank
        } else {
            rack.iter().for_each(|&t| rack_bits |= 1 << t);
        }

        {
            let mut num_moves = 0; // for testing closure
            let mut work_vec = vec![0u8; std::cmp::max(dim.rows, dim.cols) as usize];

            let rows_times_cols = ((dim.rows as isize) * (dim.cols as isize)) as usize;
            // tally the played tiles

            let mut tally = Tally::new(game_config.alphabet().len());
            //tally.clear(); // already cleared
            board_tiles.iter().for_each(|&t| {
                if t & 0x80 != 0 {
                    tally.0[0] += 1;
                } else if t != 0 {
                    tally.0[t as usize] += 1;
                }
            });
            println!("{:?}", tally.0);

            let num_tiles_on_board: i16 = tally.0.iter().map(|&x| x as i16).sum();
            println!("{:?} tiles on board", num_tiles_on_board);

            // tally is on board, print unseens (this includes on racks)
            (0..alphabet.len()).for_each(|t| {
                let ag = alphabet.get(t);
                println!(
                    "{} total: {:2}, on board: {:2}, unseen: {:2}",
                    ag.label,
                    ag.freq,
                    tally.0[t as usize],
                    ag.freq - tally.0[t as usize]
                );
            });

            // striped by row
            let mut cross_set_for_across_plays =
                vec![CrossSet { bits: 0, score: 0 }; rows_times_cols];
            for col in 0..dim.cols {
                gen_cross_set(
                    board_tiles,
                    game_config,
                    &gdw,
                    dim.down(col),
                    &mut cross_set_for_across_plays,
                    matrix::Strider {
                        base: col as i16,
                        step: dim.cols,
                        len: dim.rows,
                    },
                );
            }
            if num_tiles_on_board == 0 {
                // empty board activates star
                cross_set_for_across_plays[board_layout
                    .dim()
                    .at_row_col(board_layout.star_row(), board_layout.star_col())] =
                    CrossSet { bits: !1, score: 0 };
            }
            for row in 0..dim.rows {
                let cross_set_start = ((row as isize) * (dim.cols as isize)) as usize;
                gen_moves(
                    board_tiles,
                    game_config,
                    &gdw,
                    &cross_set_for_across_plays
                        [cross_set_start..cross_set_start + (dim.cols as usize)],
                    &rack,
                    &mut rack_tally,
                    rack_bits,
                    dim.across(row),
                    &mut work_vec,
                    |idx: i8, word: &[u8], score: i16| {
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
                        println!(
                            " -- move {}: row={}, col={}, word={:?}, score={}",
                            num_moves, row, idx, word, score
                        );
                    },
                );
            }
            // striped by columns for better cache locality
            let mut cross_set_for_down_plays =
                vec![CrossSet { bits: 0, score: 0 }; rows_times_cols];
            for row in 0..dim.rows {
                gen_cross_set(
                    board_tiles,
                    game_config,
                    &gdw,
                    dim.across(row),
                    &mut cross_set_for_down_plays,
                    matrix::Strider {
                        base: row as i16,
                        step: dim.rows,
                        len: dim.cols,
                    },
                );
            }
            for col in 0..dim.cols {
                let cross_set_start = ((col as isize) * (dim.rows as isize)) as usize;
                gen_moves(
                    board_tiles,
                    game_config,
                    &gdw,
                    &cross_set_for_down_plays
                        [cross_set_start..cross_set_start + (dim.rows as usize)],
                    &rack,
                    &mut rack_tally,
                    rack_bits,
                    dim.down(col),
                    &mut work_vec,
                    |idx: i8, word: &[u8], score: i16| {
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
                        println!(
                            " -- move {}: col={}, row={}, word={:?}, score={}",
                            num_moves, col, idx, word, score
                        );
                    },
                );
            }
        }

        // todo: actually gen moves.
        // todo: xchg.
        // todo: leaves.
    }

    println!("Hello, world!");
    Ok(())
}
