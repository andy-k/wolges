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

struct WriteableRack<'a> {
    alphabet: &'a alphabet::Alphabet<'a>,
    rack: &'a [u8],
}

impl std::fmt::Display for WriteableRack<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        for &tile in self.rack {
            write!(f, "{}", self.alphabet.from_rack(tile).unwrap())?;
        }
        Ok(())
    }
}

fn printable_rack<'a>(alphabet: &'a alphabet::Alphabet<'a>, rack: &'a [u8]) -> WriteableRack<'a> {
    WriteableRack {
        alphabet: &alphabet,
        rack: &rack,
    }
}

fn rack_score<'a>(alphabet: &'a alphabet::Alphabet<'a>, rack: &'a [u8]) -> i16 {
    rack.iter().map(|&t| alphabet.score(t) as i16).sum::<i16>()
}

struct GamePlayer {
    score: i16,
    rack: Vec<u8>,
}

struct GameState<'a> {
    game_config: &'a game_config::GameConfig<'a>,
    players: Box<[GamePlayer]>,
    board_tiles: Box<[u8]>,
    bag: bag::Bag,
    turn: u8,
}

impl<'a> GameState<'a> {
    fn new(game_config: &'a game_config::GameConfig, num_players: u8) -> Self {
        let board_layout = game_config.board_layout();
        let dim = board_layout.dim();
        let rack_size = game_config.rack_size() as usize;
        let alphabet = game_config.alphabet();
        Self {
            game_config,
            players: (0..num_players)
                .map(|_| GamePlayer {
                    score: 0,
                    rack: Vec::with_capacity(rack_size),
                })
                .collect(),
            board_tiles: vec![0u8; (dim.rows as usize) * (dim.cols as usize)].into_boxed_slice(),
            bag: bag::Bag::new(&alphabet),
            turn: 0,
        }
    }
}

pub fn main() -> error::Returns<()> {
    let kwg = kwg::Kwg::from_bytes_alloc(&std::fs::read("csw19.kwg")?);
    let klv = klv::Klv::from_bytes_alloc(&std::fs::read("leaves.klv")?);
    let game_config = &game_config::COMMON_ENGLISH_GAME_CONFIG;
    let mut move_generator = movegen::KurniaMoveGenerator::new(game_config);

    let mut game_state = GameState::new(game_config, 2);

    let mut zero_turns = 0;
    println!("\nplaying self");
    let mut rng = rand_chacha::ChaCha20Rng::from_entropy();

    game_state.bag.shuffle(&mut rng);

    println!(
        "bag: {}",
        printable_rack(&game_state.game_config.alphabet(), &game_state.bag.0)
    );

    for player in game_state.players.iter_mut() {
        game_state.bag.replenish(
            &mut player.rack,
            game_state.game_config.rack_size() as usize,
        );
    }

    loop {
        display::print_board(
            &game_state.game_config.alphabet(),
            &game_state.game_config.board_layout(),
            &game_state.board_tiles,
        );
        for (i, player) in (1..).zip(game_state.players.iter()) {
            print!("player {}: {}, ", i, player.score);
        }
        println!("turn: player {}", game_state.turn + 1);

        println!(
            "pool {:2}: {}",
            game_state.bag.0.len(),
            printable_rack(&game_state.game_config.alphabet(), &game_state.bag.0)
        );
        for (i, player) in (1..).zip(game_state.players.iter()) {
            println!(
                "p{} rack: {}",
                i,
                printable_rack(&game_state.game_config.alphabet(), &player.rack)
            );
        }

        let current_player = &mut game_state.players[game_state.turn as usize];

        let board_snapshot = &movegen::BoardSnapshot {
            board_tiles: &game_state.board_tiles,
            game_config,
            kwg: &kwg,
            klv: &klv,
        };

        move_generator.gen_moves_alloc(board_snapshot, &current_player.rack, 15);
        let plays = &move_generator.plays;

        println!("found {} moves", plays.len());
        for play in plays.iter() {
            println!("{} {}", play.equity, play.play.fmt(board_snapshot));
        }
        println!("making top move: {}", plays[0].play.fmt(board_snapshot));

        zero_turns += 1;
        let play = &plays[0]; // assume at least there's always Pass
        match &play.play {
            movegen::Play::Exchange { tiles } => {
                use_tiles(&mut current_player.rack, tiles.iter().copied())?;
                game_state.bag.replenish(
                    &mut current_player.rack,
                    game_state.game_config.rack_size() as usize,
                );
                game_state.bag.put_back(&mut rng, &tiles);
            }
            movegen::Play::Place {
                down,
                lane,
                idx,
                word,
                score,
            } => {
                let dim = game_state.game_config.board_layout().dim();
                let strider = if *down {
                    dim.down(*lane)
                } else {
                    dim.across(*lane)
                };

                // place the tiles
                for (i, &tile) in (*idx..).zip(word.iter()) {
                    if tile != 0 {
                        game_state.board_tiles[strider.at(i)] = tile;
                    }
                }

                current_player.score += score;
                if *score != 0 {
                    zero_turns = 0;
                }
                use_tiles(
                    &mut current_player.rack,
                    word.iter().filter_map(|&tile| {
                        if tile != 0 {
                            Some(tile & !((tile as i8) >> 7) as u8)
                        } else {
                            None
                        }
                    }),
                )?;
                game_state.bag.replenish(
                    &mut current_player.rack,
                    game_state.game_config.rack_size() as usize,
                );
            }
        }
        println!();

        if current_player.rack.is_empty() {
            display::print_board(
                &game_state.game_config.alphabet(),
                &game_state.game_config.board_layout(),
                &game_state.board_tiles,
            );
            for (i, player) in (1..).zip(game_state.players.iter()) {
                print!("player {}: {}, ", i, player.score);
            }
            println!(
                "player {} went out (scores are before leftovers)",
                game_state.turn + 1
            );
            if game_state.players.len() == 2 {
                game_state.players[game_state.turn as usize].score += 2 * rack_score(
                    &game_state.game_config.alphabet(),
                    &game_state.players[(1 - game_state.turn) as usize].rack,
                );
            } else {
                let mut earned = 0;
                for mut player in game_state.players.iter_mut() {
                    let this_rack = rack_score(&game_state.game_config.alphabet(), &player.rack);
                    player.score -= this_rack;
                    earned += this_rack;
                }
                game_state.players[game_state.turn as usize].score += earned;
            }
            break;
        }

        if zero_turns >= 6 {
            display::print_board(
                &game_state.game_config.alphabet(),
                &game_state.game_config.board_layout(),
                &game_state.board_tiles,
            );
            for (i, player) in (1..).zip(game_state.players.iter()) {
                print!("player {}: {}, ", i, player.score);
            }
            println!(
                "player {} ended game by making sixth zero score",
                game_state.turn + 1
            );
            for mut player in game_state.players.iter_mut() {
                player.score -= rack_score(&game_state.game_config.alphabet(), &player.rack);
            }
            break;
        }

        game_state.turn += 1;
        game_state.turn -= game_state.players.len() as u8
            & -((game_state.turn >= game_state.players.len() as u8) as i8) as u8;
    }

    for (i, player) in (1..).zip(game_state.players.iter()) {
        print!("player {}: {}, ", i, player.score);
    }
    println!("final scrores");

    Ok(())
}
