// Copyright (C) 2020-2021 Andy Kurnia. All rights reserved.

use board::{
    display, error, game_config, game_state, game_timers, klv, kwg, move_filter, move_picker,
    movegen,
};
use rand::prelude::*;

pub fn main() -> error::Returns<()> {
    let kwg = kwg::Kwg::from_bytes_alloc(&std::fs::read("csw19.kwg")?);
    let klv = klv::Klv::from_bytes_alloc(&std::fs::read("leaves.klv")?);
    let game_config = &game_config::make_common_english_game_config();
    let mut move_generator = movegen::KurniaMoveGenerator::new(game_config);

    let mut filtered_movegen_0 = move_filter::GenMoves::Tilt {
        tilt: move_filter::Tilt::new(&game_config, &kwg, move_filter::Tilt::length_importances()),
        bot_level: 1,
    };
    let mut filtered_movegen_1 = move_filter::GenMoves::Unfiltered;
    if true {
        filtered_movegen_0 = move_filter::GenMoves::Unfiltered;
    }

    let mut move_picker_0 = move_picker::MovePicker::Hasty;
    let mut move_picker_1 =
        move_picker::MovePicker::Simmer(move_picker::Simmer::new(game_config, &kwg, &klv));

    let mut game_state = game_state::GameState::new(game_config);
    let mut rng = rand_chacha::ChaCha20Rng::from_entropy();
    let mut timers = game_timers::GameTimers::new(game_config.num_players());
    loop {
        game_state.reset_and_draw_tiles(&mut rng);
        let mut final_scores = vec![0; game_state.players.len()];
        //timers.reset_to(25 * 60 * 1000);
        timers.reset_to(15 * 1000);

        loop {
            timers.set_turn(game_state.turn as i8);
            display::print_game_state(&game_state, Some(&timers));

            let filtered_movegen = if game_state.turn == 0 {
                &mut filtered_movegen_0
            } else {
                &mut filtered_movegen_1
            };
            if let move_filter::GenMoves::Tilt { tilt, bot_level } = filtered_movegen {
                tilt.tilt_by_rng(&mut rng, *bot_level);
                println!(
                    "Effective tilt: tilt factor = {}, leave scale = {}",
                    tilt.tilt_factor, tilt.leave_scale
                );
            }

            let move_picker = if game_state.turn == 0 {
                &mut move_picker_0
            } else {
                &mut move_picker_1
            };

            let board_snapshot = &movegen::BoardSnapshot {
                board_tiles: &game_state.board_tiles,
                game_config,
                kwg: &kwg,
                klv: &klv,
            };

            move_picker.pick_a_move(
                filtered_movegen,
                &mut move_generator,
                &board_snapshot,
                &game_state,
                &game_state.current_player().rack,
            );
            let plays = &mut move_generator.plays;
            let play = &plays[0].play; // assume at least there's always Pass
            println!("Playing: {}", play.fmt(board_snapshot));

            game_state.play(&mut rng, play)?;

            match game_state.check_game_ended(&mut final_scores) {
                game_state::CheckGameEnded::PlayedOut => {
                    println!("Player {} went out", game_state.turn + 1);
                    break;
                }
                game_state::CheckGameEnded::ZeroScores => {
                    println!(
                        "Player {} ended game by making yet another zero score",
                        game_state.turn + 1
                    );
                    break;
                }
                game_state::CheckGameEnded::NotEnded => {}
            }
            game_state.next_turn();
        }
        timers.set_turn(-1);

        display::print_game_state(&game_state, Some(&timers));
        println!("Final scores: {:?}", final_scores);
        let mut has_time_adjustment = false;
        for (i, &clock_ms) in timers.clocks_ms.iter().enumerate() {
            let adjustment = game_config.time_adjustment(clock_ms);
            if adjustment != 0 {
                println!("Player {} adjustment {}", i + 1, adjustment);
                final_scores[i] += adjustment;
                has_time_adjustment = true;
            }
        }
        if has_time_adjustment {
            println!("Really final scores: {:?}", final_scores);
        }
    } // temp loop

    //Ok(())
}
