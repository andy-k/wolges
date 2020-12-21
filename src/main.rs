#[macro_use]
mod error;

mod alphabet;
mod bites;
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

pub fn read_english_machine_words(giant_string: &str) -> error::Returns<Box<[bites::Bites]>> {
    // Memory wastage notes:
    // - Vec of 270k words have size 512k because vec grows by doubling.
    // - Size of vec is 24 bytes. Size of slice would have been 16 bytes.
    // - Each vec is individually allocated. We could instead join them all.
    // - We do not do this, because that O(n) already gives build().

    let mut machine_words = Vec::<bites::Bites>::new();
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
        machine_words.push(v[..].into());
    }
    Ok(machine_words.into_boxed_slice())
}

pub struct Bag(pub Vec<u8>);

impl Bag {
    fn new(alphabet: &alphabet::Alphabet) -> Bag {
        let mut bag = Vec::with_capacity(
            (0..alphabet.len())
                .map(|tile| alphabet.freq(tile) as usize)
                .sum(),
        );
        for tile in 0..alphabet.len() {
            for _ in 0..alphabet.freq(tile) {
                bag.push(tile as u8);
            }
        }
        Bag(bag)
    }

    fn shuffle(&mut self, mut rng: &mut dyn RngCore) {
        self.0.shuffle(&mut rng);
    }

    fn pop(&mut self) -> Option<u8> {
        self.0.pop()
    }

    // put back the tiles in random order. keep the rest of the bag in the same order.
    fn put_back(&mut self, mut rng: &mut dyn RngCore, tiles: &[u8]) {
        let mut num_new_tiles = tiles.len();
        match num_new_tiles {
            0 => {
                return;
            }
            1 => {
                self.0.insert(rng.gen_range(0, self.0.len()), tiles[0]);
                return;
            }
            _ => {}
        }
        let mut num_old_tiles = self.0.len();
        let new_len = num_new_tiles + num_old_tiles;
        self.0.reserve(new_len);
        let mut p_old_tiles = self.0.len();
        self.0.resize(2 * self.0.len(), 0);
        self.0.copy_within(0..num_old_tiles, num_old_tiles);
        let mut p_new_tiles = self.0.len();
        self.0.extend_from_slice(tiles);
        self.0[p_new_tiles..].shuffle(&mut rng);
        for wp in 0..new_len {
            if if num_new_tiles == 0 {
                true
            } else if num_old_tiles == 0 {
                false
            } else {
                rng.gen_range(0, num_old_tiles + num_new_tiles) < num_old_tiles
            } {
                self.0[wp] = self.0[p_old_tiles];
                p_old_tiles += 1;
                num_old_tiles -= 1;
            } else {
                self.0[wp] = self.0[p_new_tiles];
                p_new_tiles += 1;
                num_new_tiles -= 1;
            }
        }
        self.0.truncate(new_len);
    }
}

fn use_tiles<II: IntoIterator<Item = u8>>(
    rack: &mut Vec<u8>,
    tiles_iter: II,
) -> error::Returns<()> {
    for tile in tiles_iter {
        let pos = rack.iter().rposition(|&t| t == tile).ok_or("bad tile")?;
        rack.swap_remove(pos);
    }
    Ok(())
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
            let rounded_leave = (f32::from_str(&record[1])? * 256.0).round();
            let int_leave = rounded_leave as i16;
            assert!((int_leave as f32 - rounded_leave).abs() == 0.0);
            leave_values.push((String::from(&record[0]), int_leave));
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
        let mut bin = vec![0; 2 * 4 + leaves_kwg.len() + leave_values.len() * 2];
        let mut w = 0;
        bin[w..w + 4].copy_from_slice(&((leaves_kwg.len() / 4) as u32).to_le_bytes());
        w += 4;
        bin[w..w + leaves_kwg.len()].copy_from_slice(&leaves_kwg);
        w += leaves_kwg.len();
        bin[w..w + 4].copy_from_slice(&(leave_values.len() as u32).to_le_bytes());
        w += 4;
        for (_, v) in leave_values {
            bin[w..w + 2].copy_from_slice(&v.to_le_bytes());
            w += 2;
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
            "ecwl.kwg",
            build::build(
                build::BuildFormat::Gaddawg,
                &read_english_machine_words(&std::fs::read_to_string("ecwl.txt")?)?,
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
            "twl14.kwg",
            build::build(
                build::BuildFormat::Gaddawg,
                &read_english_machine_words(&std::fs::read_to_string("twl14.txt")?)?,
            )?,
        )?;

        if true {
            // this reads the files again, but this code is temporary
            let v_csw19 = read_english_machine_words(&std::fs::read_to_string("csw19.txt")?)?;
            let v_ecwl = read_english_machine_words(&std::fs::read_to_string("ecwl.txt")?)?;
            let v_nwl18 = read_english_machine_words(&std::fs::read_to_string("nwl18.txt")?)?;
            let v_nwl20 = read_english_machine_words(&std::fs::read_to_string("nwl20.txt")?)?;
            let v_twl14 = read_english_machine_words(&std::fs::read_to_string("twl14.txt")?)?;
            let mut v = Vec::<bites::Bites>::new();
            v.extend_from_slice(&v_csw19);
            v.extend_from_slice(&v_ecwl);
            v.extend_from_slice(&v_nwl18);
            v.extend_from_slice(&v_nwl20);
            v.extend_from_slice(&v_twl14);
            v.sort();
            v.dedup();
            let v = v.into_boxed_slice();
            println!("num dedup: {}", v.len());
            let v_bits_bytes = (v.len() + 7) / 8;
            let mut v_csw19_bits = vec![0u8; v_bits_bytes];
            let mut v_ecwl_bits = vec![0u8; v_bits_bytes];
            let mut v_nwl18_bits = vec![0u8; v_bits_bytes];
            let mut v_nwl20_bits = vec![0u8; v_bits_bytes];
            let mut v_twl14_bits = vec![0u8; v_bits_bytes];
            let mut p_csw19 = v_csw19.len();
            let mut p_ecwl = v_ecwl.len();
            let mut p_nwl18 = v_nwl18.len();
            let mut p_nwl20 = v_nwl20.len();
            let mut p_twl14 = v_twl14.len();
            for i in (0..v.len()).rev() {
                if p_csw19 > 0 && v[i] == v_csw19[p_csw19 - 1] {
                    v_csw19_bits[i / 8] |= 1 << (i % 8);
                    p_csw19 -= 1;
                }
                if p_ecwl > 0 && v[i] == v_ecwl[p_ecwl - 1] {
                    v_ecwl_bits[i / 8] |= 1 << (i % 8);
                    p_ecwl -= 1;
                }
                if p_nwl18 > 0 && v[i] == v_nwl18[p_nwl18 - 1] {
                    v_nwl18_bits[i / 8] |= 1 << (i % 8);
                    p_nwl18 -= 1;
                }
                if p_nwl20 > 0 && v[i] == v_nwl20[p_nwl20 - 1] {
                    v_nwl20_bits[i / 8] |= 1 << (i % 8);
                    p_nwl20 -= 1;
                }
                if p_twl14 > 0 && v[i] == v_twl14[p_twl14 - 1] {
                    v_twl14_bits[i / 8] |= 1 << (i % 8);
                    p_twl14 -= 1;
                }
            }
            std::fs::write("allgdw.kwg", build::build(build::BuildFormat::Gaddawg, &v)?)?;
            std::fs::write("all-csw19.kwi", v_csw19_bits)?;
            std::fs::write("all-ecwl.kwi", v_ecwl_bits)?;
            std::fs::write("all-nwl18.kwi", v_nwl18_bits)?;
            std::fs::write("all-nwl20.kwi", v_nwl20_bits)?;
            std::fs::write("all-twl14.kwi", v_twl14_bits)?;
        }

        if false {
            // proof-of-concept
            let kwg = kwg::Kwg::from_bytes_alloc(&std::fs::read("allgdw.kwg")?);
            let word_counts = kwg.count_dawg_words_alloc();
            // because dawg do not need gaddag nodes
            println!("only counting {} nodes", word_counts.len());
            let v_csw19_bits = std::fs::read("all-csw19.kwi")?;
            let v_ecwl_bits = std::fs::read("all-ecwl.kwi")?;
            let v_nwl18_bits = std::fs::read("all-nwl18.kwi")?;
            let v_nwl20_bits = std::fs::read("all-nwl20.kwi")?;
            let v_twl14_bits = std::fs::read("all-twl14.kwi")?;
            let mut out_vec = Vec::new();
            let dawg_root = kwg[0].arc_index();
            for i in 0..word_counts[dawg_root as usize] {
                out_vec.clear();
                kwg.get_word_by_index(&word_counts, dawg_root, i, |v| {
                    out_vec.push(v);
                });
                let j = kwg.get_word_index(&word_counts, dawg_root, &out_vec);
                print!("{} {} {:?}", i, j, out_vec);
                let byte_index = j as usize / 8;
                let bit = 1 << (j as usize % 8);
                if v_csw19_bits[byte_index] & bit != 0 {
                    print!(" csw19");
                }
                if v_ecwl_bits[byte_index] & bit != 0 {
                    print!(" ecwl");
                }
                if v_nwl18_bits[byte_index] & bit != 0 {
                    print!(" nwl18");
                }
                if v_nwl20_bits[byte_index] & bit != 0 {
                    print!(" nwl20");
                }
                if v_twl14_bits[byte_index] & bit != 0 {
                    print!(" twl14");
                }
                println!();
                assert_eq!(i, j);
            }
        }

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

    {
        let mut zero_turns = 0;
        let mut scores = [0, 0];
        let mut turn = 0;
        println!("\nplaying self");
        let board_layout = game_config.board_layout();
        let dim = board_layout.dim();
        let mut board_tiles = vec![0u8; (dim.rows as usize) * (dim.cols as usize)];
        let alphabet = game_config.alphabet();
        let mut rng = rand_chacha::ChaCha20Rng::from_entropy();

        let mut bag = Bag::new(&alphabet);
        bag.shuffle(&mut rng);

        print!("bag: ");
        for &tile in &bag.0 {
            print!("{}", alphabet.from_rack(tile).unwrap());
        }
        println!();

        let mut racks = [Vec::with_capacity(7), Vec::with_capacity(7)];
        for _ in 0..7 {
            racks[0].push(bag.pop().unwrap());
        }
        for _ in 0..7 {
            racks[1].push(bag.pop().unwrap());
        }

        loop {
            print_board(game_config, &board_tiles);
            println!(
                "player 1: {}, player 2: {}, turn: player {}",
                scores[0],
                scores[1],
                turn + 1
            );

            print!("pool {:2}: ", bag.0.len());
            for tile in &bag.0 {
                print!("{}", alphabet.from_rack(*tile).unwrap());
            }
            println!();
            print!("p1 rack: ");
            for tile in &*racks[0] {
                print!("{}", alphabet.from_rack(*tile).unwrap());
            }
            println!();
            print!("p2 rack: ");
            for tile in &*racks[1] {
                print!("{}", alphabet.from_rack(*tile).unwrap());
            }
            println!();

            let mut rack = &mut racks[turn];

            let plays = movegen::kurnia_gen_moves_alloc(
                &movegen::BoardSnapshot {
                    board_tiles: &board_tiles,
                    game_config,
                    kwg: &kwg,
                    klv: &klv,
                },
                &rack,
                15,
            );

            zero_turns += 1;
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
                    use_tiles(&mut rack, tiles.iter().copied())?;
                    for _ in 0..std::cmp::min(7 - rack.len(), bag.0.len()) {
                        rack.push(bag.pop().unwrap());
                    }
                    bag.put_back(&mut rng, &tiles);
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
                    for (i, &tile) in (*idx..).zip(word.iter()) {
                        if tile == 0 {
                            if !inside {
                                print!("(");
                                inside = true;
                            }
                            print!(
                                "{}",
                                alphabet.from_board(board_tiles[strider.at(i)]).unwrap()
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
                    for (i, &tile) in (*idx..).zip(word.iter()) {
                        if tile != 0 {
                            board_tiles[strider.at(i)] = tile;
                        }
                    }

                    scores[turn] += score;
                    if *score != 0 {
                        zero_turns = 0;
                    }
                    use_tiles(
                        &mut rack,
                        word.iter().filter_map(|&tile| {
                            if tile != 0 {
                                Some(if tile & 0x80 == 0 { tile } else { 0 })
                            } else {
                                None
                            }
                        }),
                    )?;
                    for _ in 0..std::cmp::min(7 - rack.len(), bag.0.len()) {
                        rack.push(bag.pop().unwrap());
                    }
                }
            }
            println!();
            println!();

            if rack.is_empty() {
                print_board(game_config, &board_tiles);
                println!(
                    "player 1: {}, player 2: {}, player {} went out (scores are before leftovers)",
                    scores[0],
                    scores[1],
                    turn + 1
                );
                scores[0] += 2 * racks[1]
                    .iter()
                    .map(|&t| alphabet.score(t) as i16)
                    .sum::<i16>();
                scores[1] += 2 * racks[0]
                    .iter()
                    .map(|&t| alphabet.score(t) as i16)
                    .sum::<i16>();
                break;
            }

            if zero_turns >= 6 {
                print_board(game_config, &board_tiles);
                println!(
                    "player 1: {}, player 2: {}, player {} ended game by making sixth zero score",
                    scores[0],
                    scores[1],
                    turn + 1
                );
                scores[0] -= racks[0]
                    .iter()
                    .map(|&t| alphabet.score(t) as i16)
                    .sum::<i16>();
                scores[1] -= racks[1]
                    .iter()
                    .map(|&t| alphabet.score(t) as i16)
                    .sum::<i16>();
                break;
            }

            turn = 1 - turn;
        }

        match scores[0].cmp(&scores[1]) {
            std::cmp::Ordering::Greater => {
                println!(
                    "final score: player 1: {}, player 2: {} (player 1 wins by {})",
                    scores[0],
                    scores[1],
                    scores[0] - scores[1],
                );
            }
            std::cmp::Ordering::Less => {
                println!(
                    "final score: player 1: {}, player 2: {} (player 2 wins by {})",
                    scores[0],
                    scores[1],
                    scores[1] - scores[0],
                );
            }
            std::cmp::Ordering::Equal => {
                println!(
                    "final score: player 1: {}, player 2: {} (it's a draw)",
                    scores[0], scores[1],
                );
            }
        };
    }

    println!("Hello, world!");
    Ok(())
}
