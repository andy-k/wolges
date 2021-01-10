// Copyright (C) 2020-2021 Andy Kurnia. All rights reserved.

use super::{
    display, error, game_config, game_state, game_timers, klv, kwg, move_filter, move_picker,
    movegen,
};
use rand::prelude::*;

pub fn main() -> error::Returns<()> {
    let kwg = kwg::Kwg::from_bytes_alloc(&std::fs::read("csw19.kwg")?);
    let klv = klv::Klv::from_bytes_alloc(&std::fs::read("leaves.klv")?);
    let game_config = &game_config::make_common_english_game_config();
    let mut move_generator = movegen::KurniaMoveGenerator::new(game_config);

    let mut filtered_movegen_0 = move_filter::GenMoves::Tilt(move_filter::Tilt::new(
        &game_config,
        &kwg,
        move_filter::Tilt::length_importances(),
        1,
    ));
    let mut filtered_movegen_1 = move_filter::GenMoves::Unfiltered;

    let mut move_picker_0 = move_picker::MovePicker::Hasty;
    let mut move_picker_1 =
        move_picker::MovePicker::Simmer(move_picker::Simmer::new(game_config, &kwg, &klv));

    let mut game_state = game_state::GameState::new(game_config);
    let mut rng = rand_chacha::ChaCha20Rng::from_entropy();
    let mut timers = game_timers::GameTimers::new(game_config.num_players());
    loop {
        game_state.reset_and_draw_tiles(&mut rng);
        let mut final_scores = vec![0; game_state.players.len()];
        timers.reset_to(25 * 60 * 1000);

        loop {
            timers.set_turn(game_state.turn as i8);
            display::print_game_state(&game_state, Some(&timers));

            let filtered_movegen = if game_state.turn == 0 {
                &mut filtered_movegen_0
            } else {
                &mut filtered_movegen_1
            };
            if let move_filter::GenMoves::Tilt(tilt) = filtered_movegen {
                tilt.tilt_by_rng(&mut rng);
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
        let mut has_time_penalty = false;
        for (i, &timer) in timers.clocks_ms.iter().enumerate() {
            if timer < 0 {
                let penalty = (((!timer / 60000) + 1) * 10) as i16;
                println!("Player {} penalty {}", i + 1, penalty);
                final_scores[i] -= penalty;
                has_time_penalty = true;
            }
        }
        if has_time_penalty {
            println!("Really final scores: {:?}", final_scores);
        }
    } // temp loop

    //Ok(())
}
