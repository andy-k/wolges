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
mod main_build;
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

fn main() -> error::Returns<()> {
    if false {
        main_build::main()?;
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
        let rack_size = game_config.rack_size() as usize;
        let mut rng = rand_chacha::ChaCha20Rng::from_entropy();

        let mut bag = Bag::new(&alphabet);
        bag.shuffle(&mut rng);

        print!("bag: ");
        for &tile in &bag.0 {
            print!("{}", alphabet.from_rack(tile).unwrap());
        }
        println!();

        let mut racks = [Vec::with_capacity(rack_size), Vec::with_capacity(rack_size)];
        for _ in 0..rack_size {
            racks[0].push(bag.pop().unwrap());
        }
        for _ in 0..rack_size {
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

            let board_snapshot = &movegen::BoardSnapshot {
                board_tiles: &board_tiles,
                game_config,
                kwg: &kwg,
                klv: &klv,
            };

            let mut reusable_working_buffer =
                movegen::ReusableWorkingBuffer::new(board_snapshot.game_config);
            movegen::kurnia_gen_moves_alloc(
                &mut reusable_working_buffer,
                board_snapshot,
                &rack,
                15,
            );
            let plays = reusable_working_buffer.plays;

            {
                println!("found {} moves", plays.len());
                let mut s = String::new();
                for play in plays.iter() {
                    s.clear();
                    movegen::write_play(board_snapshot, &play.play, &mut s);
                    println!("{} {}", play.equity, s);
                }
            }

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
                    for _ in 0..std::cmp::min(rack_size - rack.len(), bag.0.len()) {
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
                                Some(tile & !((tile as i8) >> 7) as u8)
                            } else {
                                None
                            }
                        }),
                    )?;
                    for _ in 0..std::cmp::min(rack_size - rack.len(), bag.0.len()) {
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
