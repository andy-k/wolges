use super::{alphabet, bag, display, error, game_config, klv, kwg, movegen};
use rand::prelude::*;

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

fn print_rack<'a>(alphabet: &'a alphabet::Alphabet<'a>, rack: &'a [u8]) {
    for &tile in rack {
        print!("{}", alphabet.from_rack(tile).unwrap());
    }
}

fn rack_score<'a>(alphabet: &'a alphabet::Alphabet<'a>, rack: &'a [u8]) -> i16 {
    rack.iter().map(|&t| alphabet.score(t) as i16).sum::<i16>()
}

pub fn main() -> error::Returns<()> {
    let kwg = kwg::Kwg::from_bytes_alloc(&std::fs::read("csw19.kwg")?);
    let klv = klv::Klv::from_bytes_alloc(&std::fs::read("leaves.klv")?);
    let game_config = &game_config::COMMON_ENGLISH_GAME_CONFIG;
    let mut reusable_working_buffer = movegen::ReusableWorkingBuffer::new(game_config);

    let mut zero_turns = 0;
    let mut scores = [0, 0];
    let mut turn = 0;
    println!("\nplaying self");
    let board_layout = game_config.board_layout();
    let dim = board_layout.dim();
    let mut board_tiles = vec![0u8; (dim.rows as usize) * (dim.cols as usize)];
    let alphabet = game_config.alphabet();
    let rack_size = game_config.rack_size() as usize;
    let mut formatted_play_str = String::new();
    let mut rng = rand_chacha::ChaCha20Rng::from_entropy();

    let mut bag = bag::Bag::new(&alphabet);
    bag.shuffle(&mut rng);

    print!("bag: ");
    print_rack(&alphabet, &bag.0);
    println!();

    let mut racks = [Vec::with_capacity(rack_size), Vec::with_capacity(rack_size)];
    bag.replenish(&mut racks[0], rack_size);
    bag.replenish(&mut racks[1], rack_size);

    loop {
        display::print_board(&alphabet, &board_layout, &board_tiles);
        println!(
            "player 1: {}, player 2: {}, turn: player {}",
            scores[0],
            scores[1],
            turn + 1
        );

        print!("pool {:2}: ", bag.0.len());
        print_rack(&alphabet, &bag.0);
        println!();
        print!("p1 rack: ");
        print_rack(&alphabet, &racks[0]);
        println!();
        print!("p2 rack: ");
        print_rack(&alphabet, &racks[1]);
        println!();

        let mut rack = &mut racks[turn];

        let board_snapshot = &movegen::BoardSnapshot {
            board_tiles: &board_tiles,
            game_config,
            kwg: &kwg,
            klv: &klv,
        };

        movegen::kurnia_gen_moves_alloc(&mut reusable_working_buffer, board_snapshot, &rack, 15);
        let plays = &reusable_working_buffer.plays;

        println!("found {} moves", plays.len());
        for play in plays.iter() {
            formatted_play_str.clear();
            movegen::write_play(board_snapshot, &play.play, &mut formatted_play_str);
            println!("{} {}", play.equity, formatted_play_str);
        }
        formatted_play_str.clear();
        movegen::write_play(board_snapshot, &plays[0].play, &mut formatted_play_str);
        println!("making top move: {}", formatted_play_str);

        zero_turns += 1;
        let play = &plays[0]; // assume at least there's always Pass
        match &play.play {
            movegen::Play::Pass => {}
            movegen::Play::Exchange { tiles } => {
                use_tiles(&mut rack, tiles.iter().copied())?;
                bag.replenish(&mut rack, rack_size);
                bag.put_back(&mut rng, &tiles);
            }
            movegen::Play::Place {
                down,
                lane,
                idx,
                word,
                score,
            } => {
                let strider = if *down {
                    dim.down(*lane)
                } else {
                    dim.across(*lane)
                };

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
                bag.replenish(&mut rack, rack_size);
            }
        }
        println!();

        if rack.is_empty() {
            display::print_board(&alphabet, &board_layout, &board_tiles);
            println!(
                "player 1: {}, player 2: {}, player {} went out (scores are before leftovers)",
                scores[0],
                scores[1],
                turn + 1
            );
            scores[0] += 2 * rack_score(&alphabet, &racks[0]);
            scores[1] += 2 * rack_score(&alphabet, &racks[1]);
            break;
        }

        if zero_turns >= 6 {
            display::print_board(&alphabet, &board_layout, &board_tiles);
            println!(
                "player 1: {}, player 2: {}, player {} ended game by making sixth zero score",
                scores[0],
                scores[1],
                turn + 1
            );
            scores[0] -= rack_score(&alphabet, &racks[0]);
            scores[1] -= rack_score(&alphabet, &racks[1]);
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

    Ok(())
}
