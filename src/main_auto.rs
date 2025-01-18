// Copyright (C) 2020-2025 Andy Kurnia.

use rand::prelude::*;
use wolges::{
    alphabet, bag, bites, build, display, error, fash, game_config, game_state, game_timers, klv,
    kwg, move_filter, move_picker, movegen, play_scorer, stats,
};

fn main() -> error::Returns<()> {
    if false {
        let mut rng = rand_chacha::ChaCha20Rng::from_seed(*b"the seed is an array of 32 bytes");
        println!("{:?}", rng.get_seed());
        let alphabet = alphabet::make_english_alphabet();
        let mut bag;
        let mut v = Vec::new();
        for _ in 0..5 {
            //v.push((rng.get_seed(), rng.get_stream(), rng.get_word_pos()));
            v.push(rng.clone());
            bag = bag::Bag::new(&alphabet);
            bag.shuffle(&mut rng);
            println!("Pool {}: {}", bag.0.len(), alphabet.fmt_rack(&bag.0));
        }
        println!("{:?}", v);
        for _ in 0..40 {
            print!(" {}", rng.gen_range(0..10));
        }
        println!();
        for _ in 0..5 {
            //let (seed, stream, word_pos) = v.pop().unwrap();
            bag = bag::Bag::new(&alphabet);
            rng = v.pop().unwrap();
            //rng = rand_chacha::ChaCha20Rng::from_seed(seed);
            //rng.set_stream(stream);
            //rng.set_word_pos(word_pos);
            bag.shuffle(&mut rng);
            println!("Pool {}: {}", bag.0.len(), alphabet.fmt_rack(&bag.0));
        }
        return Ok(());
    }
    let jumbled = true;
    let _ = jumbled;
    let jumbled = false;
    let kwg = if jumbled {
        kwg::Kwg::from_bytes_alloc(&std::fs::read("lexbin/CSW24.kad")?)
    } else {
        kwg::Kwg::from_bytes_alloc(&std::fs::read("lexbin/CSW24.kwg")?)
    };
    let klv = klv::Klv::from_bytes_alloc(&std::fs::read("lexbin/CSW24.klv2")?);
    /*
    let _ = klv;
    let klv = std::sync::Arc::new(klv::Klv::from_bytes_alloc(klv::EMPTY_KLV_BYTES));
    */
    let game_config = &if jumbled {
        game_config::make_jumbled_english_game_config()
    } else {
        game_config::make_english_game_config()
    };
    let mut fen_parser =
        display::BoardFenParser::new(game_config.alphabet(), game_config.board_layout());
    let mut move_generator = movegen::KurniaMoveGenerator::new(game_config);

    let mut filtered_movegen_0 = move_filter::GenMoves::Tilt {
        tilt: move_filter::Tilt::new(game_config, &kwg, move_filter::Tilt::length_importances()),
        bot_level: 1,
    };
    let mut filtered_movegen_1 = move_filter::GenMoves::Unfiltered;
    if true {
        filtered_movegen_0 = move_filter::GenMoves::Unfiltered;
    }

    let mut move_picker_0 = move_picker::MovePicker::Hasty;
    let mut move_picker_1 =
        move_picker::MovePicker::Simmer(move_picker::Simmer::new(game_config, &kwg, &klv));
    if true {
        move_picker_1 = move_picker::MovePicker::Hasty;
    }

    let mut score_stats_0 = stats::Stats::new();
    let mut score_stats_1 = stats::Stats::new();
    let mut spread_stats_0 = stats::Stats::new();
    let mut win_stats_0 = stats::Stats::new();
    let mut loss_draw_win = [0i64; 3];

    let mut game_state = game_state::GameState::new(game_config);
    //let mut rng = rand_chacha::ChaCha20Rng::from_entropy();
    // "the seed is an array of 32 bytes".len() == 32.
    let mut rng = rand_chacha::ChaCha20Rng::from_seed(*b"Wolges Copyright (C) Andy Kurnia");
    let mut timers = game_timers::GameTimers::new(game_config.num_players());
    if false {
        // https://discord.com/channels/741321677828522035/1157118170398724176/1193946371129094154
        let fen_str = "ZONULE1B2APAID/1KY2RHANJA4/GAM4R2HUI2/7G6D/6FECIT3O/6AE1TOWIES/6I7E/1EnGUARD6D/NAOI2W8/6AT7/5PYE7/5L1L7/2COVE1L7/5X1E7/7N7";
        let parsed_fen = fen_parser.parse(fen_str)?;
        game_state.board_tiles.copy_from_slice(parsed_fen);
        let alphabet = game_config.alphabet();
        let mut available_tally = (0..alphabet.len())
            .map(|x| alphabet.freq(x))
            .collect::<Vec<u8>>();
        // should check underflow.
        for i in game_state.board_tiles.iter() {
            if *i != 0 {
                if i & 0x80 == 0 {
                    available_tally[*i as usize] -= 1;
                } else {
                    available_tally[0] -= 1;
                }
            }
        }
        // put the bag
        game_state.bag.0.clear();
        game_state
            .bag
            .0
            .reserve(available_tally.iter().map(|&x| x as usize).sum());
        game_state.bag.0.extend(
            (0u8..)
                .zip(available_tally.iter())
                .flat_map(|(tile, &count)| std::iter::repeat(tile).take(count as usize)),
        );
        //game_state.bag.shuffle(&mut rng);
        display::print_game_state(game_config, &game_state, Some(&timers));
        let board_snapshot = &movegen::BoardSnapshot {
            board_tiles: &game_state.board_tiles,
            game_config,
            kwg: &kwg,
            klv: &klv,
        };
        let mut set_of_words = fash::MyHashSet::<bites::Bites>::default();
        move_generator.gen_remaining_words(board_snapshot, |word: &[u8]| {
            set_of_words.insert(word.into());
            //println!("{:?}", word);
            //println!("{}", alphabet.fmt_rack(&word))
        });
        println!("word_prune: {} words", set_of_words.len());
        let mut vec_of_words = set_of_words.into_iter().collect::<Vec<_>>();
        vec_of_words.sort_unstable();
        //println!("{:?}", vec_of_words);
        let smaller_kwg_bytes = build::build(
            build::BuildContent::Gaddawg,
            build::BuildLayout::Wolges,
            &vec_of_words.into_boxed_slice(),
        )?;
        println!("word_prune: {} bytes kwg", smaller_kwg_bytes.len());
        //std::fs::write("_word_62702.kwg", smaller_kwg_bytes)?;
        let smaller_kwg = kwg::Kwg::from_bytes_alloc(&smaller_kwg_bytes);
        let test_rack = &[13, 15, 15, 15, 18, 18, 20]; // MOOORRT
        move_generator.gen_moves_unfiltered(&movegen::GenMovesParams {
            board_snapshot,
            rack: test_rack,
            max_gen: usize::MAX,
            num_exchanges_by_this_player: 0,
            always_include_pass: false,
        });
        let plays1 = move_generator.plays.clone();
        move_generator.reset_for_another_kwg();
        move_generator.gen_moves_unfiltered(&movegen::GenMovesParams {
            board_snapshot: &movegen::BoardSnapshot {
                kwg: &smaller_kwg,
                ..*board_snapshot
            },
            rack: test_rack,
            max_gen: usize::MAX,
            num_exchanges_by_this_player: 0,
            always_include_pass: false,
        });
        let plays2 = move_generator.plays.clone();
        move_generator.reset_for_another_kwg();
        move_generator.gen_moves_unfiltered(&movegen::GenMovesParams {
            board_snapshot,
            rack: test_rack,
            max_gen: usize::MAX,
            num_exchanges_by_this_player: 0,
            always_include_pass: false,
        });
        let plays3 = move_generator.plays.clone();
        if plays1 != plays3 {
            panic!("movegen was confused");
        }
        if plays1 != plays2 {
            panic!("movegen cannot work with smaller kwg");
        }
        let plays = plays2;
        println!("{} moves found...", plays.len());
        for play in plays.iter() {
            println!("{} {}", play.equity, play.play.fmt(board_snapshot));
        }
        return Ok(());
    }
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
                    num_exchanges_by_this_player: game_state.current_player().num_exchanges,
                    always_include_pass: false,
                });
                // test word prune, only for classic.
                let plays2;
                let plays = if match &board_snapshot.game_config.game_rules() {
                    game_config::GameRules::Classic => true,
                    game_config::GameRules::Jumbled => false,
                } {
                    let plays1 = move_generator.plays.clone();
                    // these always allocate for now.
                    let mut set_of_words = fash::MyHashSet::<bites::Bites>::default();
                    move_generator.gen_remaining_words(board_snapshot, |word: &[u8]| {
                        set_of_words.insert(word.into());
                    });
                    println!("word_prune: {} words", set_of_words.len());
                    let mut vec_of_words = set_of_words.into_iter().collect::<Vec<_>>();
                    vec_of_words.sort_unstable();
                    let smaller_kwg_bytes = build::build(
                        build::BuildContent::Gaddawg,
                        build::BuildLayout::Wolges,
                        &vec_of_words.into_boxed_slice(),
                    )?;
                    println!("word_prune: {} bytes kwg", smaller_kwg_bytes.len());
                    let smaller_kwg = kwg::Kwg::from_bytes_alloc(&smaller_kwg_bytes);
                    move_generator.reset_for_another_kwg();
                    move_generator.gen_moves_unfiltered(&movegen::GenMovesParams {
                        board_snapshot: &movegen::BoardSnapshot {
                            kwg: &smaller_kwg,
                            ..*board_snapshot
                        },
                        rack: &game_state.current_player().rack,
                        max_gen: usize::MAX,
                        num_exchanges_by_this_player: game_state.current_player().num_exchanges,
                        always_include_pass: false,
                    });
                    plays2 = move_generator.plays.clone();
                    move_generator.reset_for_another_kwg();
                    move_generator.gen_moves_unfiltered(&movegen::GenMovesParams {
                        board_snapshot,
                        rack: &game_state.current_player().rack,
                        max_gen: usize::MAX,
                        num_exchanges_by_this_player: game_state.current_player().num_exchanges,
                        always_include_pass: false,
                    });
                    if plays1 != move_generator.plays {
                        panic!("movegen was confused");
                    }
                    if plays1 != plays2 {
                        panic!("movegen cannot work with smaller kwg");
                    }
                    &plays2
                } else {
                    &move_generator.plays
                };
                println!("{} moves found...", plays.len());
                for play in plays.iter() {
                    println!("{} {}", play.equity, play.play.fmt(board_snapshot));
                }
            }

            // stress-test scoring algorithm
            if match &board_snapshot.game_config.game_rules() {
                game_config::GameRules::Classic => false,
                game_config::GameRules::Jumbled => false,
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
                        num_exchanges_by_this_player: game_state.current_player().num_exchanges,
                        always_include_pass: true,
                    },
                    |_down: bool, _lane: i8, _idx: i8, _word: &[u8], _score: i32| true,
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

            if false {
                // not required for now.
                move_generator.reset_for_another_kwg();
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
                final_scores[i] += adjustment as i32;
                has_time_adjustment = true;
            }
        }
        if has_time_adjustment {
            println!("Really final scores: {final_scores:?}");
        }

        let fs0 = final_scores[0];
        let fs1 = final_scores[1];
        let spr = fs0 - fs1;
        let p0dw = spr.signum() + 1; // double win (2 = win, 1 = draw/tie, 0 = loss)
        score_stats_0.update(fs0.into());
        score_stats_1.update(fs1.into());
        spread_stats_0.update(spr.into());
        win_stats_0.update(p0dw as f64 * 50.0);
        loss_draw_win[p0dw as usize] += 1;

        println!(
            "Stats: {final_scores:?} n={} ({}-{}-{}) p0={:.3} (sd={:.3}) p1={:.3} (sd={:.3}) p0-p1={:.3} (sd={:.3}) p0w={:.3} (sd={:.3})",
            loss_draw_win[0] + loss_draw_win[1] + loss_draw_win[2],
            loss_draw_win[2],
            loss_draw_win[0],
            loss_draw_win[1],
            score_stats_0.mean(),
            score_stats_0.standard_deviation(),
            score_stats_1.mean(),
            score_stats_1.standard_deviation(),
            spread_stats_0.mean(),
            spread_stats_0.standard_deviation(),
            win_stats_0.mean(),
            win_stats_0.standard_deviation(),
        );
    } // temp loop

    //Ok(())
}
