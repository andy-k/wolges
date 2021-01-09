// Copyright (C) 2020-2021 Andy Kurnia. All rights reserved.

use super::{display, error, game_config, game_state, klv, kwg, move_filter, move_picker, movegen};
use rand::prelude::*;

struct GameTimers {
    instant: std::time::Instant,
    clocks_ms: Box<[i64]>,
    turn: i8, // -1 for nobody's
}

impl GameTimers {
    fn new(num_players: u8) -> Self {
        Self {
            instant: std::time::Instant::now(),
            clocks_ms: vec![0; num_players as usize].into_boxed_slice(),
            turn: -1,
        }
    }

    fn reset_to(&mut self, initial_ms: i64) {
        self.clocks_ms.iter_mut().for_each(|m| *m = initial_ms);
        self.turn = -1;
        self.instant = std::time::Instant::now();
    }

    fn set_turn(&mut self, new_turn: i8) {
        let new_instant = std::time::Instant::now();
        if self.turn >= 0 && (self.turn as usize) < self.clocks_ms.len() {
            self.clocks_ms[self.turn as usize] -= new_instant
                .saturating_duration_since(self.instant)
                .as_millis() as i64;
        }
        self.instant = new_instant;
        self.turn = new_turn;
    }
}

fn print_ms(mut ms: i64) {
    if ms < 0 {
        print!("-");
        ms = -ms;
    }
    let just_ms = ms % 1000;
    let sec = ms / 1000;
    let just_sec = sec % 60;
    let min = sec / 60;
    print!("{:02}:{:02}.{:03}", min, just_sec, just_ms);
}

fn print_timers(timers: &GameTimers) {
    print!("Timers: ");
    for (i, &timer) in timers.clocks_ms.iter().enumerate() {
        if i != 0 {
            print!(", ")
        }
        if i as isize == timers.turn as isize {
            print!("*")
        }
        print!("Player {}: ", i + 1);
        print_ms(timer);
    }
    println!();
}

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
    let mut timers = GameTimers::new(game_config.num_players());
    loop {
        game_state.reset_and_draw_tiles(&mut rng);
        let mut final_scores = vec![0; game_state.players.len()];
        timers.reset_to(25 * 60 * 1000);

        loop {
            display::print_game_state(&game_state);
            timers.set_turn(game_state.turn as i8);
            print_timers(&timers);

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

        display::print_game_state(&game_state);
        println!("Final scores: {:?}", final_scores);
        print_timers(&timers);
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
