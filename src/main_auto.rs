// Copyright (C) 2020-2026 Andy Kurnia.

use rand::prelude::*;
use std::fs::File;
use std::io::{BufWriter, Write};
use std::sync::{Arc, Mutex};
use wolges::{
    alphabet, anagrams, bag, bites, build, display, equity, error, fash, game_config, game_state,
    game_timers, infer, klv, kwg, move_filter, move_picker, movegen, play_scorer, stats,
};

// its counting allocator and --max-memory; its endgame watching goes unused
#[allow(dead_code)]
mod endgame_watch;
mod game_args;
mod run_line;

type HarvestWriter = Arc<Mutex<BufWriter<File>>>;
type GameRng = rand_xoshiro::Xoshiro256PlusPlus;

#[derive(clap::Parser)]
#[command(version = wolges::VERSION)]
struct Cli {
    #[arg(
        long,
        value_name = "GB",
        default_value_t = 0.0,
        help = "keep the program within GB gigabytes (of 10^9 bytes), its word graph \
                included (0: no limit): the late graph is pruned only where it fits, else \
                the sim walks the whole lexicon, and a decision is refused where no room is \
                left"
    )]
    max_memory: f64,
    #[arg(long, help = "the word graph is a kbwg")]
    kbwg: bool,
    #[arg(
        long,
        value_name = "FILE",
        help = "write each bag-empty position the games reach to FILE, as endgame --batch reads"
    )]
    harvest: Option<String>,
    #[arg(long, value_name = "N", help = "stop after N games (default: never)")]
    games: Option<u64>,
    #[arg(long, help = "generate with the lean arm")]
    lean: bool,
    #[arg(default_value = "lexbin/CSW24.kwg", help = "the word graph")]
    kwg: String,
    #[arg(default_value = "lexbin/CSW24.klv2", help = "the leaves (- for none)")]
    klv: String,
    #[command(flatten)]
    game: game_args::GameArgs,
}

fn main() -> error::Returns<()> {
    let cli: Cli = clap::Parser::parse();
    if false {
        let mut rng = rand::rngs::ChaCha20Rng::from_seed(*b"the seed is an array of 32 bytes");
        writeln!(std::io::stdout(), "{:?}", rng.get_seed())?;
        let alphabet = alphabet::make_english_alphabet();
        let mut bag;
        let mut v = Vec::new();
        for _ in 0..5 {
            v.push((rng.get_seed(), rng.get_stream(), rng.get_word_pos()));
            bag = bag::Bag::new(&alphabet);
            bag.shuffle(&mut rng);
            writeln!(
                std::io::stdout(),
                "Pool {}: {}",
                bag.len(),
                alphabet.fmt_rack(bag.as_slice())
            )?;
        }
        writeln!(std::io::stdout(), "{v:?}")?;
        for _ in 0..40 {
            write!(std::io::stdout(), " {}", rng.random_range(0..10))?;
        }
        writeln!(std::io::stdout())?;
        for _ in 0..5 {
            let (seed, stream, word_pos) = v.pop().unwrap();
            bag = bag::Bag::new(&alphabet);
            rng = rand::rngs::ChaCha20Rng::from_seed(seed);
            rng.set_stream(stream);
            rng.set_word_pos(word_pos);
            bag.shuffle(&mut rng);
            writeln!(
                std::io::stdout(),
                "Pool {}: {}",
                bag.len(),
                alphabet.fmt_rack(bag.as_slice())
            )?;
        }
        return Ok(());
    }

    let harvest: Option<HarvestWriter> = match cli.harvest {
        Some(path) => Some(Arc::new(Mutex::new(BufWriter::new(File::create(path)?)))),
        None => None,
    };

    let game_config = std::sync::Arc::new(cli.game.make_game_config()?);
    if game_config.num_players() != 2 {
        wolges::return_error!("auto plays two seats, so it needs exactly 2 players".to_string());
    }
    let klv = std::sync::Arc::new(if cli.klv == "-" {
        klv::Klv::<kwg::Node24>::empty()?
    } else {
        klv::kbwg_from_bytes_alloc(run_line::read_input(&cli.klv)?)?
    });
    game_config.check_klv(&klv)?;
    let kwg = std::sync::Arc::new(kwg::kbwg_from_bytes_alloc(
        run_line::read_input(&cli.kwg)?,
        cli.kbwg,
    )?);
    refuse_a_wider_graph(&kwg, &game_config, &cli.kwg)?;
    eprintln!(
        "{}",
        run_line::run_line(&std::env::args_os().collect::<Vec<_>>())
    );
    do_it(
        &kwg,
        &klv,
        &game_config,
        harvest,
        cli.games,
        cli.lean,
        endgame_watch::memory_limit(cli.max_memory)?,
    )
}

#[inline]
fn refuse_a_wider_graph<N: kwg::Node>(
    kwg: &kwg::Kwg<N>,
    game_config: &game_config::GameConfig,
    kwg_path: &str,
) -> error::Returns<()> {
    let alphabet_len = game_config.alphabet().len();
    if kwg.fits_alphabet(alphabet_len) {
        return Ok(());
    }
    wolges::return_error!(format!(
        "{kwg_path} has tiles past this game's {alphabet_len}",
    ));
}

#[inline(always)]
fn do_it(
    kwg: &std::sync::Arc<kwg::Kwg<kwg::Node24>>,
    klv: &std::sync::Arc<klv::Klv<kwg::Node24>>,
    game_config: &std::sync::Arc<game_config::GameConfig>,
    harvest: Option<HarvestWriter>,
    games: Option<u64>,
    lean: bool,
    memory: Option<wolges::endgame::MemoryLimit>,
) -> error::Returns<()> {
    let mut fen_parser =
        display::BoardFenParser::new(game_config.alphabet(), game_config.board_layout());
    let mut move_generator = movegen::KurniaMoveGenerator::new(game_config);
    let arm_tables = std::sync::Arc::new(anagrams::ArmTables::build(kwg, game_config, lean)?);

    let mut filtered_movegen_0 = move_filter::GenMoves::Tilt {
        tilt: move_filter::Tilt::new(game_config, kwg, move_filter::Tilt::length_importances()),
        bot_level: 1,
    };
    let mut filtered_movegen_1 = move_filter::GenMoves::Unfiltered;
    if true {
        filtered_movegen_0 = move_filter::GenMoves::Unfiltered;
    }

    let mut move_picker_0 = move_picker::MovePicker::Hasty;
    let mut move_picker_1 = move_picker::MovePicker::Simmer(Box::new(move_picker::Simmer::new(
        game_config,
        kwg,
        &arm_tables,
        klv,
        move_picker::SimmerParams {
            num_sim_iters: move_picker::DEFAULT_NUM_SIM_ITERS,
            num_sim_plies: move_picker::DEFAULT_NUM_SIM_PLIES,
            sim_threads: 1,
            win_pct_table: None,
            quiet: false,
            memory,
        },
    )?));
    if true {
        move_picker_1 = move_picker::MovePicker::Hasty;
    }

    let mut score_stats_0 = stats::Stats::new();
    let mut score_stats_1 = stats::Stats::new();
    let mut spread_stats_0 = stats::Stats::new();
    let mut win_stats_0 = stats::Stats::new();
    let mut loss_draw_win = [0u64; 3];
    let mut games_played = 0u64;

    let mut game_state = game_state::GameState::new(game_config);
    //let mut rng = GameRng::try_from_rng(&mut rand::rngs::SysRng)?;
    // "the seed is an array of 32 bytes".len() == 32.
    let mut rng = GameRng::from_seed(*b"Wolges Copyright (C) Andy Kurnia");
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
        game_state.bag.set_from_iter(
            (0u8..)
                .zip(available_tally.iter())
                .flat_map(|(tile, &count)| std::iter::repeat_n(tile, count as usize)),
        );
        //game_state.bag.shuffle(&mut rng);
        display::print_game_state(game_config, &game_state, Some(&timers))?;
        let board_snapshot = &movegen::BoardSnapshot {
            board_tiles: &game_state.board_tiles,
            game_config,
            kwg,
            anagrams: arm_tables.anagrams(),
            rack_lengths: arm_tables.rack_lengths(),
            klv,
        };
        let mut set_of_words = fash::MyHashSet::<bites::Bites>::default();
        move_generator.gen_remaining_words(board_snapshot, |word: &[u8]| {
            set_of_words.insert(word.into());
            //println!("{:?}", word);
            //println!("{}", alphabet.fmt_rack(&word))
        });
        writeln!(
            std::io::stdout(),
            "word_prune: {} words",
            set_of_words.len()
        )?;
        let mut vec_of_words = set_of_words.into_iter().collect::<Vec<_>>();
        vec_of_words.sort_unstable();
        //println!("{:?}", vec_of_words);
        let smaller_kwg_bytes = build::build_big(
            build::BuildContent::Gaddawg,
            build::BuildLayout::Wolges,
            build::BuildOrder::Sorted,
            &vec_of_words.into_boxed_slice(),
        )?;
        writeln!(
            std::io::stdout(),
            "word_prune: {} bytes kwg",
            smaller_kwg_bytes.len()
        )?;
        //std::fs::write("_word_62702.kwg", smaller_kwg_bytes)?;
        let smaller_kwg = kwg::Kwg::<kwg::Node24>::from_bytes_alloc(&smaller_kwg_bytes)?;
        let smaller_tables = anagrams::ArmTables::build(&smaller_kwg, game_config, lean)?;
        let test_rack = &[13, 15, 15, 15, 18, 18, 20]; // MOOORRT
        move_generator.gen_moves_unfiltered(&movegen::GenMovesParams {
            board_snapshot,
            rack: test_rack,
            max_gen: usize::MAX,
            num_exchanges_by_this_player: 0,
            pass_policy: movegen::PassPolicy::OnlyWhenForced,
            dynamic_leaves: None,
        });
        let plays1 = move_generator.plays.clone();
        move_generator.reset_for_another_kwg();
        move_generator.gen_moves_unfiltered(&movegen::GenMovesParams {
            board_snapshot: &movegen::BoardSnapshot {
                kwg: &smaller_kwg,
                anagrams: smaller_tables.anagrams(),
                rack_lengths: smaller_tables.rack_lengths(),
                ..*board_snapshot
            },
            rack: test_rack,
            max_gen: usize::MAX,
            num_exchanges_by_this_player: 0,
            pass_policy: movegen::PassPolicy::OnlyWhenForced,
            dynamic_leaves: None,
        });
        let plays2 = move_generator.plays.clone();
        move_generator.reset_for_another_kwg();
        move_generator.gen_moves_unfiltered(&movegen::GenMovesParams {
            board_snapshot,
            rack: test_rack,
            max_gen: usize::MAX,
            num_exchanges_by_this_player: 0,
            pass_policy: movegen::PassPolicy::OnlyWhenForced,
            dynamic_leaves: None,
        });
        let plays3 = move_generator.plays.clone();
        if plays1 != plays3 {
            return Err("movegen was confused".into());
        }
        if plays1 != plays2 {
            return Err("movegen cannot work with smaller kwg".into());
        }
        let plays = plays2;
        writeln!(std::io::stdout(), "{} moves found...", plays.len())?;
        for play in plays.iter() {
            writeln!(
                std::io::stdout(),
                "{:>8} {}",
                play.equity,
                play.play.fmt(board_snapshot)
            )?;
        }
        return Ok(());
    }
    let mut final_scores = vec![0i32; game_state.players.len()];
    let mut display_scores = vec![0i32; game_state.players.len()];
    let t0 = std::time::Instant::now();
    // the last move and the board before it, for a sim to infer from: kept
    // from game to game, the move there from the second turn of a game on
    let mut last_board = game_state.board_tiles.to_vec();
    let mut last_play: Option<movegen::Play> = None;
    // each seat's inference of its opponent, carried to its next, and the
    // tiles it has put back since, with the next carry's buffer: kept from
    // game to game, emptied at the start of each
    let mut carried = vec![infer::Carried::default(); game_state.players.len()];
    let mut put_back = vec![Vec::new(); game_state.players.len()];
    let mut next_carried = infer::Carried::default();
    loop {
        game_state.reset_and_draw_tiles_double_ended(game_config, &mut rng);
        final_scores.iter_mut().for_each(|s| *s = 0);
        carried.iter_mut().for_each(infer::Carried::clear);
        put_back.iter_mut().for_each(Vec::clear);

        timers.reset_to(15 * 1000);
        let mut has_last = false;

        loop {
            timers.set_turn(game_state.turn as i8);
            display::print_game_state(game_config, &game_state, Some(&timers))?;

            if false {
                let fen_str = format!(
                    "{}",
                    display::BoardFenner::new(
                        game_config.alphabet(),
                        game_config.board_layout(),
                        &game_state.board_tiles,
                    )
                );
                writeln!(std::io::stdout(), "{fen_str}")?;
                let parsed_fen = fen_parser.parse(&fen_str)?;
                if parsed_fen != &game_state.board_tiles[..] {
                    writeln!(
                        std::io::stdout(),
                        "{} parses into {:?} (expecting {:?})",
                        fen_str,
                        parsed_fen,
                        game_state.board_tiles
                    )?;
                }
            }

            let filtered_movegen = if game_state.turn == 0 {
                &mut filtered_movegen_0
            } else {
                &mut filtered_movegen_1
            };
            if let move_filter::GenMoves::Tilt { tilt, bot_level } = filtered_movegen {
                tilt.tilt_by_rng(&mut rng, *bot_level);
                writeln!(
                    std::io::stdout(),
                    "Effective tilt: tilt factor = {}/{}, leave scale = {}/{}",
                    tilt.tilt_factor,
                    move_filter::TILT_DENOM,
                    tilt.leave_scale,
                    move_filter::LEAVE_SCALE_DENOM
                )?;
            }

            let move_picker = if game_state.turn == 0 {
                &mut move_picker_0
            } else {
                &mut move_picker_1
            };

            let board_snapshot = &movegen::BoardSnapshot {
                board_tiles: &game_state.board_tiles,
                game_config,
                kwg,
                anagrams: arm_tables.anagrams(),
                rack_lengths: arm_tables.rack_lengths(),
                klv,
            };

            if false {
                move_generator.gen_moves_unfiltered(&movegen::GenMovesParams {
                    board_snapshot,
                    rack: &game_state.current_player().rack,
                    max_gen: usize::MAX,
                    num_exchanges_by_this_player: game_state.current_player().num_exchanges,
                    pass_policy: movegen::PassPolicy::OnlyWhenForced,
                    dynamic_leaves: None,
                });

                let plays2;
                let plays = if match &board_snapshot.game_config.game_rules() {
                    game_config::GameRules::Classic => true,
                    game_config::GameRules::Jumbled
                    | game_config::GameRules::JumbledWoadwage
                    | game_config::GameRules::Woadwage => false,
                } {
                    let plays1 = move_generator.plays.clone();

                    let mut set_of_words = fash::MyHashSet::<bites::Bites>::default();
                    move_generator.gen_remaining_words(board_snapshot, |word: &[u8]| {
                        set_of_words.insert(word.into());
                    });
                    writeln!(
                        std::io::stdout(),
                        "word_prune: {} words",
                        set_of_words.len()
                    )?;
                    let mut vec_of_words = set_of_words.into_iter().collect::<Vec<_>>();
                    vec_of_words.sort_unstable();
                    let smaller_kwg_bytes = build::build_big(
                        build::BuildContent::Gaddawg,
                        build::BuildLayout::Wolges,
                        build::BuildOrder::Sorted,
                        &vec_of_words.into_boxed_slice(),
                    )?;
                    writeln!(
                        std::io::stdout(),
                        "word_prune: {} bytes kwg",
                        smaller_kwg_bytes.len()
                    )?;
                    let smaller_kwg =
                        kwg::Kwg::<kwg::Node24>::from_bytes_alloc(&smaller_kwg_bytes)?;
                    let smaller_tables =
                        anagrams::ArmTables::build(&smaller_kwg, game_config, lean)?;
                    move_generator.reset_for_another_kwg();
                    move_generator.gen_moves_unfiltered(&movegen::GenMovesParams {
                        board_snapshot: &movegen::BoardSnapshot {
                            kwg: &smaller_kwg,
                            anagrams: smaller_tables.anagrams(),
                            rack_lengths: smaller_tables.rack_lengths(),
                            ..*board_snapshot
                        },
                        rack: &game_state.current_player().rack,
                        max_gen: usize::MAX,
                        num_exchanges_by_this_player: game_state.current_player().num_exchanges,
                        pass_policy: movegen::PassPolicy::OnlyWhenForced,
                        dynamic_leaves: None,
                    });
                    plays2 = move_generator.plays.clone();
                    move_generator.reset_for_another_kwg();
                    move_generator.gen_moves_unfiltered(&movegen::GenMovesParams {
                        board_snapshot,
                        rack: &game_state.current_player().rack,
                        max_gen: usize::MAX,
                        num_exchanges_by_this_player: game_state.current_player().num_exchanges,
                        pass_policy: movegen::PassPolicy::OnlyWhenForced,
                        dynamic_leaves: None,
                    });
                    if plays1 != move_generator.plays {
                        return Err("movegen was confused".into());
                    }
                    if plays1 != plays2 {
                        return Err("movegen cannot work with smaller kwg".into());
                    }
                    &plays2
                } else {
                    &move_generator.plays
                };
                writeln!(std::io::stdout(), "{} moves found...", plays.len())?;
                for play in plays.iter() {
                    writeln!(
                        std::io::stdout(),
                        "{:>8} {}",
                        play.equity,
                        play.play.fmt(board_snapshot)
                    )?;
                }
            }

            if match &board_snapshot.game_config.game_rules() {
                game_config::GameRules::Classic | game_config::GameRules::Woadwage => false,
                game_config::GameRules::Jumbled | game_config::GameRules::JumbledWoadwage => false,
            } {
                let leave_scale = if let move_filter::GenMoves::Tilt { tilt, .. } = filtered_movegen
                {
                    tilt.leave_scale
                } else {
                    move_filter::LEAVE_SCALE_DENOM
                };
                move_generator.gen_moves_filtered(
                    &movegen::GenMovesParams {
                        board_snapshot,
                        rack: &game_state.current_player().rack,
                        max_gen: usize::MAX,
                        num_exchanges_by_this_player: game_state.current_player().num_exchanges,
                        pass_policy: movegen::PassPolicy::AsACandidate,
                        dynamic_leaves: None,
                    },
                    movegen::PlacePredicate::AcceptAll,
                    klv::AdjustLeave::Scaled {
                        scale: leave_scale,
                        denom: move_filter::LEAVE_SCALE_DENOM,
                    },
                    movegen::EquityPredicate::AcceptAll,
                );
                let plays = &mut move_generator.plays;
                writeln!(std::io::stdout(), "{} moves found...", plays.len())?;
                let mut issues = 0;
                let mut ps = play_scorer::PlayScorer::new();
                for play in plays.iter() {
                    match ps.validate_play(board_snapshot, &game_state, &play.play) {
                        Err(err) => {
                            issues += 1;
                            writeln!(
                                std::io::stdout(),
                                "{} is not valid, {}!",
                                play.play.fmt(board_snapshot),
                                err
                            )?;
                        }
                        Ok(adjusted_play) => {
                            if let Some(canonical_play) = adjusted_play {
                                issues += 1;
                                writeln!(
                                    std::io::stdout(),
                                    "{} is valid, but reformats into {}!",
                                    play.play.fmt(board_snapshot),
                                    canonical_play.fmt(board_snapshot)
                                )?;
                            }
                            let movegen_score = match &play.play {
                                movegen::Play::Exchange { .. } => 0,
                                movegen::Play::Place { score, .. } => score.raw(),
                            };
                            let recounted_score = ps.compute_score(board_snapshot, &play.play);
                            if movegen_score != recounted_score {
                                issues += 1;
                                writeln!(
                                    std::io::stdout(),
                                    "{} should score {} instead of {}!",
                                    play.play.fmt(board_snapshot),
                                    recounted_score,
                                    movegen_score
                                )?;
                            } else {
                                let recounted_equity = ps.compute_equity(
                                    board_snapshot,
                                    &game_state,
                                    &play.play,
                                    leave_scale,
                                    recounted_score,
                                );
                                if play.equity.raw() != recounted_equity {
                                    issues += 1;
                                    writeln!(
                                        std::io::stdout(),
                                        "{} should have equity {} instead of {}!",
                                        play.play.fmt(board_snapshot),
                                        equity::Equity::new(recounted_equity),
                                        play.equity
                                    )?;
                                }
                            }
                            if !ps.words_are_valid(board_snapshot, &play.play) {
                                issues += 1;
                                writeln!(
                                    std::io::stdout(),
                                    "{} forms invalid words!",
                                    play.play.fmt(board_snapshot)
                                )?;
                            }
                        }
                    }
                }
                assert_eq!(issues, 0);
            }

            if false {
                move_generator.reset_for_another_kwg();
            }
            let seat = game_state.turn as usize;
            move_picker.pick_a_move_after(
                filtered_movegen,
                &mut move_generator,
                board_snapshot,
                &game_state,
                &game_state.current_player().rack,
                (
                    last_play
                        .as_ref()
                        .filter(|_| has_last)
                        .map(|play| move_picker::LastMove {
                            board_before: &last_board,
                            play,
                            held: infer::Held::Kept(&carried[seat], &put_back[seat]),
                        }),
                    &mut next_carried,
                ),
            )?;
            std::mem::swap(&mut carried[seat], &mut next_carried);
            let plays = &mut move_generator.plays;
            let play = &plays[0].play; // assume at least there's always Pass
            writeln!(std::io::stdout(), "Playing: {}", play.fmt(board_snapshot))?;

            put_back[seat].clear();
            if let movegen::Play::Exchange { tiles } = play {
                put_back[seat].extend_from_slice(tiles);
            }
            last_board.copy_from_slice(&game_state.board_tiles);
            match &mut last_play {
                Some(last_play) => last_play.clone_from(play),
                None => last_play = Some(play.clone()),
            }
            has_last = true;
            game_state.play(game_config, &mut rng, play)?;

            match game_state.check_game_ended(game_config, &mut final_scores)? {
                game_state::CheckGameEnded::PlayedOut => {
                    writeln!(std::io::stdout(), "Player {} went out", game_state.turn + 1)?;
                    break;
                }
                game_state::CheckGameEnded::ZeroScores => {
                    writeln!(
                        std::io::stdout(),
                        "Player {} ended game by making yet another zero score",
                        game_state.turn + 1
                    )?;
                    break;
                }
                game_state::CheckGameEnded::NotEnded => {}
            }
            game_state.next_turn();

            if let Some(harvest) = &harvest
                && game_state.bag.is_empty()
            {
                let alphabet = game_config.alphabet();
                let mut writer = harvest.lock().unwrap();
                writeln!(
                    writer,
                    "{}\t{}\t{}\t{}",
                    display::BoardFenner::new(
                        alphabet,
                        game_config.board_layout(),
                        &game_state.board_tiles,
                    ),
                    alphabet.fmt_rack(&game_state.players[0].rack),
                    alphabet.fmt_rack(&game_state.players[1].rack),
                    game_state.turn,
                )?;
            }
        }
        timers.set_turn(-1);

        display::print_game_state(game_config, &game_state, Some(&timers))?;
        for (d, &f) in display_scores.iter_mut().zip(final_scores.iter()) {
            *d = equity::Equity::new(f).descaled().raw();
        }
        writeln!(std::io::stdout(), "Final scores: {display_scores:?}")?;
        let mut has_time_adjustment = false;
        for (i, &clock_ms) in timers.clocks_ms.iter().enumerate() {
            let adjustment = game_config.time_adjustment(clock_ms);
            if adjustment != 0 {
                writeln!(
                    std::io::stdout(),
                    "Player {} adjustment {}",
                    i + 1,
                    adjustment
                )?;
                final_scores[i] = i32::try_from(adjustment * i64::from(equity::SCALE))
                    .ok()
                    .and_then(|scaled| final_scores[i].checked_add(scaled))
                    .ok_or("the time adjustment overflows the final score")?;
                has_time_adjustment = true;
            }
        }
        if has_time_adjustment {
            for (d, &f) in display_scores.iter_mut().zip(final_scores.iter()) {
                *d = equity::Equity::new(f).descaled().raw();
            }
            writeln!(std::io::stdout(), "Really final scores: {display_scores:?}")?;
        }

        let spr = display_scores[0] - display_scores[1];
        let p0dw = spr.signum() + 1; // double win
        score_stats_0.update(f64::from(display_scores[0]));
        score_stats_1.update(f64::from(display_scores[1]));
        spread_stats_0.update(f64::from(spr));
        win_stats_0.update(f64::from(p0dw) * 50.0);
        games_played = games_played.checked_add(1).ok_or("too many games played")?;
        loss_draw_win[p0dw as usize] += 1;

        writeln!(
            std::io::stdout(),
            "Stats: {display_scores:?} n={} ({}-{}-{}) p0={:.3} (sd={:.3}) p1={:.3} (sd={:.3}) p0-p1={:.3} (sd={:.3}) p0w={:.3} (sd={:.3}) after {:?}",
            games_played,
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
            t0.elapsed(),
        )?;
        if games.is_some_and(|games| games_played >= games) {
            return Ok(());
        }
    } // loop

    //Ok(())
}
