// Copyright (C) 2020-2023 Andy Kurnia.

use rand::prelude::*;
use wolges::{
    display, error, game_config, game_state, game_timers, klv, kwg, move_filter, move_picker,
    movegen, play_scorer,
};

fn main() -> error::Returns<()> {
    let jumbled = true;
    let kwg = if jumbled {
        kwg::Kwg::from_bytes_alloc(&std::fs::read("lexbin/CSW21.kad")?)
    } else {
        kwg::Kwg::from_bytes_alloc(&std::fs::read("lexbin/CSW21.kwg")?)
    };
    let _klv = klv::Klv::from_bytes_alloc(&std::fs::read("lexbin/english.klv")?);
    let klv = klv::Klv::from_bytes_alloc(&std::fs::read("lexbin/CSW21.klv")?);
    let game_config = &if jumbled {
        game_config::make_jumbled_english_game_config()
    } else {
        game_config::make_common_english_game_config()
    };
    let _ = game_config;
    let game_config = &if jumbled {
        game_config::make_jumbled_punctured_english_game_config()
    } else {
        game_config::make_punctured_english_game_config()
    };
    let _ = game_config;
    let game_config = &if jumbled {
        game_config::make_jumbled_super_english_game_config()
    } else {
        game_config::make_super_english_game_config()
    };
    //let _ = game_config;
    //let game_config = &game_config::make_hong_kong_english_game_config();
    let mut fen_parser =
        display::BoardFenParser::new(game_config.alphabet(), game_config.board_layout());
    let mut move_generator = movegen::KurniaMoveGenerator::new(game_config);

    let mut filtered_movegen_0 = move_filter::GenMoves::Tilt {
        tilt: move_filter::Tilt::new(game_config, &kwg, move_filter::Tilt::length_importances()),
        bot_level: 1,
    };
    let mut filtered_movegen_1 = move_filter::GenMoves::Unfiltered;
    if false {
        filtered_movegen_0 = move_filter::GenMoves::Unfiltered;
    }

    let mut move_picker_0 = move_picker::MovePicker::Hasty;
    let mut move_picker_1 =
        move_picker::MovePicker::Simmer(move_picker::Simmer::new(game_config, &kwg, &klv));
    if true {
        move_picker_1 = move_picker::MovePicker::Hasty;
    }

    let mut game_state = game_state::GameState::new(game_config);
    let mut rng = rand_chacha::ChaCha20Rng::from_entropy();
    let mut timers = game_timers::GameTimers::new(game_config.num_players());
    loop {
        game_state.reset_and_draw_tiles(game_config, &mut rng);
        let mut final_scores = vec![0; game_state.players.len()];
        //timers.reset_to(25 * 60 * 1000);
        timers.reset_to(15 * 1000);

        loop {
            timers.set_turn(game_state.turn as i8);
            display::print_game_state(game_config, &game_state, Some(&timers));

            if false {
                let fen_str = format!(
                    "{}",
                    display::BoardFenner::new(
                        game_config.alphabet(),
                        game_config.board_layout(),
                        &game_state.board_tiles,
                    )
                );
                println!("{fen_str}");
                let parsed_fen = fen_parser.parse(&fen_str)?;
                if parsed_fen != &game_state.board_tiles[..] {
                    println!(
                        "{} parses into {:?} (expecting {:?})",
                        fen_str, parsed_fen, game_state.board_tiles
                    );
                }
            }

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

            if false {
                move_generator.gen_moves_unfiltered(&movegen::GenMovesParams {
                    board_snapshot,
                    rack: &game_state.current_player().rack,
                    max_gen: usize::MAX,
                    always_include_pass: false,
                });
                let plays = &mut move_generator.plays;
                println!("{} moves found...", plays.len());
                for play in plays.iter() {
                    println!("{} {}", play.equity, play.play.fmt(board_snapshot));
                }
            }

            // stress-test scoring algorithm
            if match &board_snapshot.game_config.game_rules() {
                game_config::GameRules::Classic => true,
                game_config::GameRules::Jumbled => true,
            } {
                let leave_scale = if let move_filter::GenMoves::Tilt { tilt, .. } = filtered_movegen
                {
                    tilt.leave_scale
                } else {
                    1.0
                };
                move_generator.gen_moves_filtered(
                    &movegen::GenMovesParams {
                        board_snapshot,
                        rack: &game_state.current_player().rack,
                        max_gen: usize::MAX,
                        always_include_pass: true,
                    },
                    |_down: bool,
                     _lane: i8,
                     _idx: i8,
                     _word: &[u8],
                     _score: i16,
                     _rack_tally: &[u8]| true,
                    |leave_value: f32| leave_scale * leave_value,
                    |_equity: f32, _play: &movegen::Play| true,
                );
                let plays = &mut move_generator.plays;
                println!("{} moves found...", plays.len());
                let mut issues = 0;
                let mut ps = play_scorer::PlayScorer::new();
                for play in plays.iter() {
                    match ps.validate_play(board_snapshot, &game_state, &play.play) {
                        Err(err) => {
                            issues += 1;
                            println!("{} is not valid, {}!", play.play.fmt(board_snapshot), err);
                        }
                        Ok(adjusted_play) => {
                            if let Some(canonical_play) = adjusted_play {
                                issues += 1;
                                println!(
                                    "{} is valid, but reformats into {}!",
                                    play.play.fmt(board_snapshot),
                                    canonical_play.fmt(board_snapshot)
                                );
                            }
                            let movegen_score = match &play.play {
                                movegen::Play::Exchange { .. } => 0,
                                movegen::Play::Place { score, .. } => *score,
                            };
                            let recounted_score = ps.compute_score(board_snapshot, &play.play);
                            if movegen_score != recounted_score {
                                issues += 1;
                                println!(
                                    "{} should score {} instead of {}!",
                                    play.play.fmt(board_snapshot),
                                    recounted_score,
                                    movegen_score
                                );
                            } else {
                                let recounted_equity = ps.compute_equity(
                                    board_snapshot,
                                    &game_state,
                                    &play.play,
                                    leave_scale,
                                    recounted_score,
                                );
                                // If leave_scale is negative these may be 0.0 and -0.0.
                                if play.equity.to_le_bytes() != recounted_equity.to_le_bytes()
                                    && !(play.equity == 0.0 && recounted_equity == 0.0)
                                {
                                    issues += 1;
                                    println!(
                                        "{} should have equity {} instead of {}!",
                                        play.play.fmt(board_snapshot),
                                        recounted_equity,
                                        play.equity
                                    );
                                }
                            }
                            if !ps.words_are_valid(board_snapshot, &play.play) {
                                issues += 1;
                                println!("{} forms invalid words!", play.play.fmt(board_snapshot));
                            }
                        }
                    }
                }
                assert_eq!(issues, 0);
            }

            move_picker.pick_a_move(
                filtered_movegen,
                &mut move_generator,
                board_snapshot,
                &game_state,
                &game_state.current_player().rack,
            );
            let plays = &mut move_generator.plays;
            let play = &plays[0].play; // assume at least there's always Pass
            println!("Playing: {}", play.fmt(board_snapshot));

            game_state.play(game_config, &mut rng, play)?;

            match game_state.check_game_ended(game_config, &mut final_scores) {
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

        display::print_game_state(game_config, &game_state, Some(&timers));
        println!("Final scores: {final_scores:?}");
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
            println!("Really final scores: {final_scores:?}");
        }
    } // temp loop

    //Ok(())
}
