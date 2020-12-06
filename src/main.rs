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

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let gdw = gdw::from_bytes(&std::fs::read("csw19.gdw")?);
    let game_config = &game_config::COMMON_ENGLISH_GAME_CONFIG;
    if false {
        print_dawg(game_config.alphabet(), &gdw);
        println!("{}", gdw.0.len());
    }

    let board_tiles = b"\
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

        fn gen_cross_set<'a>(
            board_tiles: &'a [u8],
            game_config: &'a game_config::GameConfig<'a>,
            gdw: &'a gdw::Gdw,
            strider: matrix::Strider,
            output_vec: &'a mut Vec<CrossSet>,
            output_strider: matrix::Strider,
        ) {
            let len = strider.len();
            let len_usize = len as usize;
            for i in 0..output_strider.len() {
                output_vec[output_strider.at(i)] = CrossSet { bits: 0, score: 0 };
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
                            output_vec[output_strider.at(k)] = CrossSet { bits, score };
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
                                score +=
                                    alphabet.get(if b & 0x80 == 0 { b } else { 0 }).score as i16;
                            }
                            output_vec[output_strider.at(j - 1)] = CrossSet { bits, score };
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
                    let w = &output_vec[output_strider.at(i as i8)];
                    if w.bits != 0 || w.score != 0 {
                        print!(
                            "[{:2}@{:3}] bits={:064b} score={}",
                            i,
                            output_strider.at(i as i8),
                            w.bits,
                            w.score
                        );
                        for t in 0..63 {
                            if ((w.bits >> t) & 1) != 0 {
                                print!(" {}", alphabet.from_board(t).unwrap_or("."));
                            }
                        }
                        println!();
                    }
                }
            }
        }

        let board_layout = game_config.board_layout();
        let dim = board_layout.dim();
        {
            let rows_times_cols = ((dim.rows as isize) * (dim.cols as isize)) as usize;
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
        }

        // todo: actually gen moves.
        // todo: xchg.
        // todo: leaves.
    }

    println!("Hello, world!");
    Ok(())
}
