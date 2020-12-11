#[macro_use]
mod error;

mod alphabet;
mod board_layout;
mod build;
mod display;
mod game_config;
mod klv;
mod kwg;
mod matrix;
mod movegen;

use rand::prelude::*;

fn print_dawg<'a>(a: &alphabet::Alphabet<'a>, g: &kwg::Kwg) {
    struct Env<'a> {
        a: &'a alphabet::Alphabet<'a>,
        g: &'a kwg::Kwg,
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
        g[0].arc_index(),
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

pub fn read_english_machine_words(giant_string: &str) -> error::Returns<Box<[Box<[u8]>]>> {
    // Memory wastage notes:
    // - Vec of 270k words have size 512k because vec grows by doubling.
    // - Size of vec is 24 bytes. Size of slice would have been 16 bytes.
    // - Each vec is individually allocated. We could instead join them all.
    // - We do not do this, because that O(n) already gives build().

    let mut machine_words = Vec::<Box<[u8]>>::new();
    for s in giant_string.lines() {
        let mut v = Vec::with_capacity(s.len());
        // This is English-only, and will need adjustment for multibyte.
        // The output must be 1-based because 0 has special meaning.
        // It should also not be too high to fit in a u64 cross-set.
        for c in s.chars() {
            if c >= 'A' && c <= 'Z' {
                v.push((c as u8) & 0x3f);
            } else if c == '?' {
                v.push(0); // temp hack
            } else {
                return_error!(format!("invalid tile after {:?} in {:?}", v, s));
            }
        }
        // Performance notes:
        // - .last() is slow.
        // - But the borrow checker does not like raw pointer.
        match machine_words.last() {
            Some(previous_v) => {
                if v[..] <= previous_v[..] {
                    return_error!(format!(
                        "input is not sorted, {:?} cannot come after {:?}",
                        v, previous_v
                    ));
                }
            }
            None => {
                if v.is_empty() {
                    return_error!("first line is blank".into());
                }
            }
        };
        machine_words.push(v.into_boxed_slice());
    }
    Ok(machine_words.into_boxed_slice())
}

use std::str::FromStr;

fn main() -> error::Returns<()> {
    if false {
        let f = std::fs::File::open("leaves.csv")?;
        let mut leave_values = Vec::new();
        // extern crate csv;
        let mut csv_reader = csv::ReaderBuilder::new().has_headers(false).from_reader(f);
        for result in csv_reader.records() {
            let record = result?;
            leave_values.push((String::from(&record[0]), f32::from_str(&record[1])?));
        }
        leave_values.sort_by(|(s1, _), (s2, _)| s1.cmp(s2));
        let leaves_kwg = build::build(
            build::BuildFormat::DawgOnly,
            &read_english_machine_words(&leave_values.iter().fold(
                String::new(),
                |mut acc, (s, _)| {
                    acc.push_str(s);
                    acc.push('\n');
                    acc
                },
            ))?,
        )?;
        let mut bin = vec![0; 2 * 4 + leaves_kwg.len() + leave_values.len() * 4];
        let mut w = 0;
        bin[w..w + 4].copy_from_slice(&((leaves_kwg.len() / 4) as u32).to_le_bytes());
        w += 4;
        bin[w..w + leaves_kwg.len()].copy_from_slice(&leaves_kwg);
        w += leaves_kwg.len();
        bin[w..w + 4].copy_from_slice(&(leave_values.len() as u32).to_le_bytes());
        w += 4;
        for (_, v) in leave_values {
            bin[w..w + 4].copy_from_slice(&v.to_le_bytes());
            w += 4;
        }
        assert_eq!(w, bin.len());
        std::fs::write("leaves.klv", bin)?;
        std::fs::write(
            "csw19.kwg",
            build::build(
                build::BuildFormat::Gaddawg,
                &read_english_machine_words(&std::fs::read_to_string("csw19.txt")?)?,
            )?,
        )?;
        std::fs::write(
            "nwl18.kwg",
            build::build(
                build::BuildFormat::Gaddawg,
                &read_english_machine_words(&std::fs::read_to_string("nwl18.txt")?)?,
            )?,
        )?;
        std::fs::write(
            "nwl20.kwg",
            build::build(
                build::BuildFormat::Gaddawg,
                &read_english_machine_words(&std::fs::read_to_string("nwl20.txt")?)?,
            )?,
        )?;
        std::fs::write(
            "volost.kwg",
            build::build(
                build::BuildFormat::Gaddawg,
                &read_english_machine_words("VOLOST\nVOLOSTS")?,
            )?,
        )?;
        std::fs::write(
            "empty.kwg",
            build::build(
                build::BuildFormat::Gaddawg,
                &read_english_machine_words("")?,
            )?,
        )?;
    }

    let kwg = kwg::Kwg::from_bytes_alloc(&std::fs::read("csw19.kwg")?);
    let klv = klv::Klv::from_bytes_alloc(&std::fs::read("leaves.klv")?);
    let game_config = &game_config::COMMON_ENGLISH_GAME_CONFIG;

    if false {
        print_dawg(game_config.alphabet(), &kwg);
        let t0 = std::time::Instant::now();
        let word_counts = kwg.count_words_alloc();
        println!("took {} ms", t0.elapsed().as_millis());
        println!("{:?}", &word_counts[0..100]);
        let mut out_vec = Vec::new();
        let dawg_root = kwg[0].arc_index();
        for i in 0..word_counts[dawg_root as usize] {
            out_vec.clear();
            kwg.get_word_by_index(&word_counts, dawg_root, i, |v| {
                out_vec.push(v);
            });
            let j = kwg.get_word_index(&word_counts, dawg_root, &out_vec);
            println!("{} {} {:?}", i, j, out_vec);
            assert_eq!(i, j);
        }
        assert_eq!(kwg.get_word_index(&word_counts, dawg_root, &[5, 3, 1]), !0);
        assert_eq!(kwg.get_word_index(&word_counts, dawg_root, &[]), !0);
        assert_eq!(kwg.get_word_index(&word_counts, dawg_root, &[1, 3]), !0);
        if true {
            return Ok(());
        }
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
        // ?LNS - http://liwords.localhost/game/BSCEW2NK

        board_tiles = b"\
\x00\x00\x00\x00\x00\x05\x00\x00\x00\x00\x00\x04\x05\x16\x13\
\x00\x00\x00\x00\x05\x18\x00\x00\x00\x00\x00\x15\x00\x00\x00\
\x00\x00\x00\x00\x15\x00\x00\x00\x00\x00\x02\x01\x12\x00\x00\
\x00\x00\x00\x07\x0f\x00\x00\x00\x00\x00\x09\x04\x05\x00\x00\
\x00\x00\x00\x08\x09\x00\x00\x00\x00\x00\x16\x00\x0f\x00\x00\
\x00\x00\x07\x09\x00\x00\x00\x11\x00\x07\x09\x00\x00\x00\x00\
\x00\x00\x05\x00\x00\x00\x00\x15\x00\x01\x01\x00\x10\x00\x00\
\x13\x00\x12\x00\x00\x00\x02\x0f\x14\x14\x00\x06\x12\x01\x15\
\x14\x00\x0f\x00\x00\x00\x00\x0c\x00\x05\x00\x00\x05\x00\x00\
\x05\x00\x0e\x00\x00\x0a\x09\x0c\x0c\x00\x00\x00\x14\x00\x00\
\x01\x0e\x14\x09\x0d\x01\x0e\x00\x01\x03\x05\x00\x19\x00\x00\
\x0d\x00\x09\x00\x00\x00\x00\x00\x00\x00\x08\x19\x10\x0f\x13\
\x09\x00\x03\x00\x00\x00\x06\x05\x0e\x14\x00\x00\x85\x00\x01\
\x05\x00\x00\x00\x00\x00\x00\x00\x00\x17\x0f\x0f\x04\x05\x0e\
\x12\x00\x00\x00\x00\x00\x00\x04\x1a\x0f\x00\x00\x00\x00\x0b\
";

        print_board(game_config, board_tiles);
    }

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

    let t0 = std::time::Instant::now();

    // ?UGE?US - http://liwords.localhost/game/oyMkFGLA
    movegen::kurnia_gen_moves_alloc(
        &movegen::BoardSnapshot {
            board_tiles,
            game_config,
            kwg: &kwg,
            klv: &klv,
        },
        &mut b"\x00\x15\x07\x05\x00\x15\x13".clone(),
        //&mut b"\x15\x15\x15\x15\x16\x16\x17".clone(),
        //&mut b"\x00\x04\x05\x0e\x0f\x13\x15".clone(),
        //&mut b"\x00\x0c\x0e\x13".clone(),
    );

    println!("took {} ms", t0.elapsed().as_millis());

    let mut testcases = vec![
        (vec![0], 25.19870376586914),
        (vec![17], -7.26110315322876),
        (vec![0, 9], 26.448156356811523),
        (vec![9, 0], 26.448156356811523),
        (vec![0, 4, 12, 17, 19, 22], -1.2257566452026367),
        (vec![8, 13, 18, 18, 19, 19], -7.6917290687561035),
        (vec![1, 5, 9, 14, 19, 20], 30.734148025512695),
        (vec![19, 1, 20, 9, 14, 5], 30.734148025512695),
    ];

    for (tc, _exp) in &mut testcases {
        if !tc.windows(2).all(|w| w[0] <= w[1]) {
            tc.sort_unstable();
        }
    }

    for &bn in &[1, 100, 10000, 745845, 1000000, 1322193, 1409782] {
        let t0 = std::time::Instant::now();
        let mut v = 0.0;
        for _ in 0..bn {
            for (tc, _exp) in &mut testcases {
                /*
                if !tc.windows(2).all(|w| w[0] <= w[1]) {
                    tc.sort_unstable();
                }
                */
                let leave_idx = klv
                    .kwg
                    .get_word_index(&klv.counts, klv.kwg[0].arc_index(), &tc);
                let leave_val = if leave_idx == !0 {
                    0.0
                } else {
                    klv.leaves[leave_idx as usize]
                };
                v += leave_val as f64;
            }
        }
        let dur = t0.elapsed();
        println!("{} {:#} {:?}", bn, v, dur);
        println!("{}ns/op", dur.as_nanos() / bn);
    }

    {
        let mut scores = [0, 0];
        let mut turn = 0;
        println!("\nplaying self");
        let board_layout = game_config.board_layout();
        let dim = board_layout.dim();
        let mut board_tiles = vec![0u8; (dim.rows as usize) * (dim.cols as usize)];
        let alphabet = game_config.alphabet();
        let mut rng = rand_chacha::ChaCha20Rng::from_entropy();

        loop {
            print_board(game_config, &board_tiles);
            println!(
                "player 1: {}, player 2: {}, turn: player {}",
                scores[0],
                scores[1],
                turn + 1
            );

            // this is recomputed inside, but it's cleaner this way.
            let num_tiles_on_board = board_tiles.iter().filter(|&t| *t != 0).count() as usize;

            // unseen tiles = pool minus tiles on board
            let mut unseen_tiles = vec![0u8; alphabet.len() as usize];
            for i in 0..alphabet.len() {
                unseen_tiles[i as usize] = alphabet.freq(i);
            }
            board_tiles.iter().for_each(|&t| {
                if t != 0 {
                    let ti = if t & 0x80 == 0 { t as usize } else { 0 };
                    if unseen_tiles[ti] > 0 {
                        unseen_tiles[ti] -= 1;
                    } else {
                        panic!("bad pool/board");
                    }
                }
            });

            let mut unseen_vec =
                Vec::with_capacity(unseen_tiles.iter().map(|count| *count as usize).sum());
            for (tile, &num) in unseen_tiles.iter().enumerate() {
                for _ in 0..num {
                    print!("{}", alphabet.from_rack(tile as u8).unwrap());
                    unseen_vec.push(tile as u8);
                }
            }
            println!();

            // for now, draw just before move, instead of properly
            let (rack, _leftover) = unseen_vec.partial_shuffle(&mut rng, 7);
            print!("drawn:  ");
            for tile in &*rack {
                print!("{}", alphabet.from_rack(*tile).unwrap());
            }
            println!();
            rack.sort_unstable();
            print!("sorted: ");
            for tile in &*rack {
                print!("{}", alphabet.from_rack(*tile).unwrap());
            }
            println!();

            let plays = movegen::kurnia_gen_moves_alloc(
                &movegen::BoardSnapshot {
                    board_tiles: &board_tiles,
                    game_config,
                    kwg: &kwg,
                    klv: &klv,
                },
                rack,
            );

            let mut played_out = false;
            print!("making top move: ");
            let play = &plays[0]; // assume at least there's always Pass
            match &play.play {
                movegen::Play::Pass => {
                    print!("Pass");
                }
                movegen::Play::Exchange { tiles } => {
                    print!("Exch. ");
                    for &tile in tiles.iter() {
                        print!("{}", alphabet.from_board(tile).unwrap());
                    }
                    print!(" (is a no-op because we always redraw)");
                }
                movegen::Play::Place {
                    down,
                    lane,
                    idx,
                    word,
                    score,
                } => {
                    if *down {
                        print!("{}{}", (*lane as u8 + 0x41) as char, idx + 1);
                    } else {
                        print!("{}{}", lane + 1, (*idx as u8 + 0x41) as char);
                    }
                    print!(" ");
                    let strider = if *down {
                        dim.down(*lane)
                    } else {
                        dim.across(*lane)
                    };
                    let mut inside = false;
                    for (i, &tile) in word.iter().enumerate() {
                        if tile == 0 {
                            if !inside {
                                print!("(");
                                inside = true;
                            }
                            print!(
                                "{}",
                                alphabet
                                    .from_board(board_tiles[strider.at(idx + i as i8)])
                                    .unwrap()
                            );
                        } else {
                            if inside {
                                print!(")");
                                inside = false;
                            }
                            print!("{}", alphabet.from_board(tile).unwrap());
                        }
                    }
                    if inside {
                        print!(")");
                    }
                    print!(" {}", score);

                    // place the tiles
                    let mut played_tiles = 0;
                    for (i, &tile) in word.iter().enumerate() {
                        if tile != 0 {
                            board_tiles[strider.at(idx + i as i8)] = tile;
                            played_tiles += 1;
                        }
                    }
                    if num_tiles_on_board >= 86 && played_tiles == rack.len() {
                        played_out = true;
                    }

                    scores[turn] += score;
                }
            }
            println!();
            println!();

            if played_out {
                println!("played out!");
                print_board(game_config, &board_tiles);
                println!(
                    "player 1: {}, player 2: {}, player {} went out (scores are before leftovers)",
                    scores[0],
                    scores[1],
                    turn + 1
                );
                break;
            }

            turn = 1 - turn;
        }
    }

    println!("Hello, world!");
    Ok(())
}
