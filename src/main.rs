#[macro_use]
mod error;

mod alphabet;
mod board_layout;
mod build;
mod display;
mod game_config;
mod gdw;
mod matrix;
mod movegen;

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

fn save_gaddawg(
    build_format: build::BuildFormat,
    giant_string: &str,
    output_filename: &str,
) -> error::Returns<()> {
    let t0 = std::time::Instant::now();
    let machine_words = read_english_machine_words(&giant_string)?;
    drop(giant_string);
    let t1 = std::time::Instant::now();
    println!(
        "{:10}ns to construct the machine words ({} words)",
        (t1 - t0).as_nanos(),
        machine_words.len()
    );
    let bin = build::build(build_format, &machine_words)?;
    drop(machine_words);
    let t2 = std::time::Instant::now();
    println!(
        "{:10}ns to make the gaddawg ({} bytes)",
        (t2 - t1).as_nanos(),
        bin.len()
    );
    std::fs::write(output_filename, bin)?;
    let t3 = std::time::Instant::now();
    println!(
        "{:10}ns to save the gaddawg into {}",
        (t3 - t2).as_nanos(),
        output_filename
    );
    Ok(())
}

fn save_gaddawg_from_file(
    build_format: build::BuildFormat,
    input_filename: &str,
    output_filename: &str,
) -> error::Returns<()> {
    let t0 = std::time::Instant::now();
    // Memory wastage notes:
    // - We allocate and read the whole file at once.
    // - We could have streamed it, but that's noticeably slower.
    let giant_string = std::fs::read_to_string(input_filename)?;
    let t1 = std::time::Instant::now();
    println!(
        "{:10}ns to read the lexicon from {} ({} bytes)",
        (t1 - t0).as_nanos(),
        input_filename,
        giant_string.len()
    );
    save_gaddawg(build_format, &giant_string, output_filename)
}

use std::str::FromStr;

fn main() -> error::Returns<()> {
    if true {
        //save_gaddawg_from_file(build::BuildFormat::DawgOnly, "leaves.txt", "leaves.gdw")?;
        //save_gaddawg_from_file(build::BuildFormat::DawgOnly, "csw19.txt", "csw19.gdw")?;
        save_gaddawg_from_file(build::BuildFormat::Gaddawg, "csw19.txt", "csw19.gdw")?;
        save_gaddawg_from_file(build::BuildFormat::Gaddawg, "nwl18.txt", "nwl18.gdw")?;
        save_gaddawg_from_file(build::BuildFormat::Gaddawg, "nwl20.txt", "nwl20.gdw")?;
        save_gaddawg(build::BuildFormat::Gaddawg, "VOLOST\nVOLOSTS", "volost.gdw")?;
        save_gaddawg(build::BuildFormat::Gaddawg, "", "empty.gdw")?;
        //return_error!(format!("all done"));
    }

    let gdw = gdw::Gdw::from_bytes_alloc(&std::fs::read("csw19.gdw")?);
    let game_config = &game_config::COMMON_ENGLISH_GAME_CONFIG;

    if false {
        let f = std::fs::File::open("leaves.csv")?;
        let mut rdr = csv::Reader::from_reader(f);
        for result in rdr.records() {
            let record = result?;
            println!("{:?} {:?}", &record[0], f32::from_str(&record[1]));
        }
        return Ok(());
    }

    if false {
        let t0 = std::time::Instant::now();
        let word_counts = gdw.count_words_alloc();
        println!("took {} ms", t0.elapsed().as_millis());
        println!("{:?}", &word_counts[0..100]);
        let mut out_vec = Vec::new();
        let dawg_root = gdw[0i32].arc_index();
        for i in 0..word_counts[dawg_root as usize] {
            out_vec = gdw.get_word_by_index(&word_counts, dawg_root, i, out_vec);
            let j = gdw.get_word_index(&word_counts, dawg_root, &out_vec);
            println!("{} {} {:?}", i, j, out_vec);
            assert_eq!(i, j);
        }
        assert_eq!(gdw.get_word_index(&word_counts, dawg_root, &[5, 3, 1]), !0);
        assert_eq!(gdw.get_word_index(&word_counts, dawg_root, &[]), !0);
        assert_eq!(gdw.get_word_index(&word_counts, dawg_root, &[1, 3]), !0);
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
    movegen::gen_moves_alloc(
        &movegen::BoardSnapshot {
            board_tiles,
            game_config,
            gdw: &gdw,
        },
        &mut b"\x00\x15\x07\x05\x00\x15\x13".clone(),
    );

    println!("took {} ms", t0.elapsed().as_millis());
    println!("Hello, world!");
    Ok(())
}
