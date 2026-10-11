// Copyright (C) 2020-2026 Andy Kurnia.

#![cfg(test)]

use rand::SeedableRng;
use wolges::{game_config, game_state, movegen};

const A: u8 = 1;
const Z: u8 = 26;

#[test]
fn a_play_errs_when_the_total_would_overflow() {
    let game_config = game_config::make_english_game_config();
    let mut game_state = game_state::GameState::new(&game_config);
    let mut rng = rand_xoshiro::Xoshiro256PlusPlus::seed_from_u64(0);
    game_state.players[0].score = i32::MAX - 5;
    game_state.players[0].rack = vec![A];
    let play = movegen::Play::Place {
        down: false,
        lane: 7,
        idx: 7,
        word: (&[A][..]).into(),
        score: 10,
    };
    assert!(game_state.play(&game_config, &mut rng, &play).is_err());
}

#[test]
fn going_out_adds_twice_the_rack_left() {
    let game_config = game_config::make_english_game_config();
    let mut game_state = game_state::GameState::new(&game_config);
    game_state.players[0].score = 100_000;
    game_state.players[1].rack = vec![Z];
    let mut final_scores = vec![0; 2];
    assert!(matches!(
        game_state.check_game_ended(&game_config, &mut final_scores),
        Ok(game_state::CheckGameEnded::PlayedOut)
    ));
    assert_eq!(final_scores[0], 100_000 + 2 * 1000 * 10);
}

#[test]
fn going_out_errs_when_the_total_would_overflow() {
    let game_config = game_config::make_english_game_config();
    let mut game_state = game_state::GameState::new(&game_config);
    game_state.players[0].score = i32::MAX - 5;
    game_state.players[1].rack = vec![Z];
    let mut final_scores = vec![0; 2];
    assert!(
        game_state
            .check_game_ended(&game_config, &mut final_scores)
            .is_err()
    );
}
