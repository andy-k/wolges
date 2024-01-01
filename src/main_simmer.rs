// Copyright (C) 2020-2024 Andy Kurnia.

use wolges::{alphabet, display, error, game_config, game_state, klv, kwg, movegen, simmer, stats};

// most of this is copied from main_endgame.
// parsing board into Vec<Vec<i8>> then back into Vec<u8> does not make sense,
// but here we are.

// tile numbering follows alphabet order (not necessarily unicode order).
// rack: array of numbers. 0 for blank, 1 for A.
// board: 2D array of numbers. 0 for empty, 1 for A, -1 for blank-as-A.
// lexicon: this implies board size and other rules too.
#[derive(serde::Deserialize)]
struct Question {
    lexicon: String,
    rack: Vec<u8>,
    #[serde(rename = "board")]
    board_tiles: Vec<Vec<i8>>,
}

// note: only this representation uses -1i8 for blank-as-A (in "board" input
// and "word" response for "action":"play"). everywhere else, use 0x81u8.

impl Question {
    fn from_fen(
        game_config: &game_config::GameConfig<'_>,
        lexicon: &str,
        fen_str: &str,
        rack: &str,
    ) -> Result<Question, error::MyError> {
        let alphabet = game_config.alphabet();
        let racks_alphabet_reader = alphabet::AlphabetReader::new_for_racks(alphabet);
        let board_layout = game_config.board_layout();
        let dim = board_layout.dim();
        let mut v = Vec::new(); // temp buffer
        let parse_rack = |v: &mut Vec<_>, rack: &str| -> Result<(), _> {
            racks_alphabet_reader.set_word(rack, v)
        };
        let mut fen_parser = display::BoardFenParser::new(alphabet, board_layout);
        let parsed_fen = fen_parser.parse(fen_str)?;
        let board_tiles = parsed_fen
            .chunks_exact(dim.rows as usize)
            .map(|row| {
                row.iter()
                    .map(|&x| {
                        // turn 0x81u8, 0x82u8 into -1i8, -2i8
                        if x & 0x80 == 0 {
                            x as i8
                        } else {
                            -0x80i8 - (x as i8)
                        }
                    })
                    .collect::<Vec<_>>()
            })
            .collect::<Vec<_>>();
        parse_rack(&mut v, rack).map_err(|e| error::new(format!("invalid rack {rack:?}: {e}")))?;
        Ok(Question {
            lexicon: lexicon.to_string(),
            rack: v,
            board_tiles,
        })
    }
}

struct ObservableCandidate {
    play_index: usize,
    stats: stats::Stats,
    equity_stats: stats::Stats,
    win_rate_stats: stats::Stats,
}

// Simmer can only be reused for the same game_config and kwg.
// (Refer to note at simmer::Simmer.)
// This is not enforced.
struct ObservableSimmer<'a> {
    game_config: &'a game_config::GameConfig<'a>,
    kwg: &'a kwg::Kwg,
    klv: &'a klv::Klv,
    candidates: Vec<ObservableCandidate>,
    simmer: simmer::Simmer,
}

impl<'a> ObservableSimmer<'a> {
    pub fn new(
        game_config: &'a game_config::GameConfig<'_>,
        kwg: &'a kwg::Kwg,
        klv: &'a klv::Klv,
    ) -> Self {
        Self {
            game_config,
            kwg,
            klv,
            candidates: Vec::new(),
            simmer: simmer::Simmer::new(game_config),
        }
    }

    #[inline(always)]
    fn take_candidates(&mut self, num_plays: usize) -> Vec<ObservableCandidate> {
        let mut candidates = std::mem::take(&mut self.candidates);
        candidates.clear();
        candidates.reserve(num_plays);
        for idx in 0..num_plays {
            candidates.push(ObservableCandidate {
                play_index: idx,
                stats: stats::Stats::new(),
                equity_stats: stats::Stats::new(),
                win_rate_stats: stats::Stats::new(),
            });
        }
        candidates
    }
}

fn main() -> error::Returns<()> {
    // https://github.com/domino14/macondo/issues/43
    let scores = [336, 298];
    let question = Question::from_fen(
        &game_config::make_english_game_config(),
        "NWL20",
        "C14/O2TOY9/mIRADOR8/F4DAB2PUGH1/I5GOOEY3V/T4XI2MALTHA/14N/6GUM3OWN/7PEW2DOE/9EF1DOR/2KUNA1J1BEVELS/3TURRETs2S2/7A4T2/7N7/7S7",
        "EEEIILZ",
    )?;

    let kwg;
    let klv;
    let game_config;

    // of course this should be cached
    match question.lexicon.as_str() {
        "CSW21" => {
            kwg = kwg::Kwg::from_bytes_alloc(&std::fs::read("lexbin/CSW21.kwg")?);
            klv = klv::Klv::from_bytes_alloc(&std::fs::read("lexbin/CSW21.klv2")?);
            game_config = game_config::make_english_game_config();
        }
        "CSW19" => {
            kwg = kwg::Kwg::from_bytes_alloc(&std::fs::read("lexbin/CSW19.kwg")?);
            klv = klv::Klv::from_bytes_alloc(&std::fs::read("lexbin/english.klv2")?);
            game_config = game_config::make_english_game_config();
        }
        "NWL18" => {
            kwg = kwg::Kwg::from_bytes_alloc(&std::fs::read("lexbin/NWL18.kwg")?);
            klv = klv::Klv::from_bytes_alloc(&std::fs::read("lexbin/english.klv2")?);
            game_config = game_config::make_english_game_config();
        }
        "NWL20" => {
            kwg = kwg::Kwg::from_bytes_alloc(&std::fs::read("lexbin/NWL20.kwg")?);
            klv = klv::Klv::from_bytes_alloc(&std::fs::read("lexbin/english.klv2")?);
            game_config = game_config::make_english_game_config();
        }
        "ECWL" => {
            kwg = kwg::Kwg::from_bytes_alloc(&std::fs::read("lexbin/ECWL.kwg")?);
            klv = klv::Klv::from_bytes_alloc(&std::fs::read("lexbin/english.klv2")?);
            game_config = game_config::make_english_game_config();
        }
        _ => {
            wolges::return_error!(format!("invalid lexicon {:?}", question.lexicon));
        }
    };

    let alphabet = game_config.alphabet();
    let alphabet_len_without_blank = alphabet.len() - 1;

    // note: this allocates
    let mut available_tally = (0..alphabet.len())
        .map(|tile| alphabet.freq(tile))
        .collect::<Box<_>>();

    for &tile in &question.rack {
        if tile > alphabet_len_without_blank {
            wolges::return_error!(format!(
                "rack has invalid tile {tile}, alphabet size is {alphabet_len_without_blank}",
            ));
        }
        if available_tally[tile as usize] > 0 {
            available_tally[tile as usize] -= 1;
        } else {
            wolges::return_error!(format!(
                "too many tile {} (bag contains only {})",
                tile,
                alphabet.freq(tile),
            ));
        }
    }

    let expected_dim = game_config.board_layout().dim();
    if question.board_tiles.len() != expected_dim.rows as usize {
        wolges::return_error!(format!(
            "board: need {} rows, found {} rows",
            expected_dim.rows,
            question.board_tiles.len(),
        ));
    }
    for (row_num, row) in (0..).zip(question.board_tiles.iter()) {
        if row.len() != expected_dim.cols as usize {
            wolges::return_error!(format!(
                "board row {} (0-based): need {} cols, found {} cols",
                row_num,
                expected_dim.cols,
                row.len(),
            ));
        }
    }
    let mut board_tiles =
        Vec::with_capacity((expected_dim.rows as usize) * (expected_dim.cols as usize));
    for (row_num, row) in (0..).zip(question.board_tiles.iter()) {
        for (col_num, &signed_tile) in (0..).zip(row) {
            if signed_tile == 0 {
                board_tiles.push(0);
            } else if signed_tile as u8 <= alphabet_len_without_blank {
                let tile = signed_tile as u8;
                board_tiles.push(tile);
                if available_tally[tile as usize] > 0 {
                    available_tally[tile as usize] -= 1;
                } else {
                    wolges::return_error!(format!(
                        "too many tile {} (bag contains only {})",
                        tile,
                        alphabet.freq(tile),
                    ));
                }
            } else if (!signed_tile as u8) < alphabet_len_without_blank {
                // turn -1i8, -2i8 into 0x81u8, 0x82u8
                board_tiles.push(0x81 + !signed_tile as u8);
                // verify usage of blank tile
                if available_tally[0] > 0 {
                    available_tally[0] -= 1;
                } else {
                    wolges::return_error!(format!(
                        "too many tile {} (bag contains only {})",
                        0,
                        alphabet.freq(0),
                    ));
                }
            } else {
                wolges::return_error!(format!(
                    "board row {row_num} col {col_num} (0-based): invalid tile {signed_tile}, alphabet size is {alphabet_len_without_blank}",
                ));
            }
        }
    }

    // rebuild game state
    let mut game_state = game_state::GameState::new(&game_config);
    for (i, &score) in scores.iter().enumerate() {
        game_state.players[i].score = score;
    }
    game_state.board_tiles[..].clone_from_slice(&board_tiles);
    game_state.set_current_rack(&question.rack);
    game_state.bag.0.clear();
    game_state.bag.0.extend(
        (0u8..)
            .zip(available_tally.iter())
            .flat_map(|(tile, &count)| std::iter::repeat(tile).take(count as usize)),
    );
    for player in game_state.players.iter_mut() {
        game_state
            .bag
            .replenish(&mut player.rack, game_config.rack_size() as usize);
    }
    display::print_game_state(&game_config, &game_state, None);

    // ok, let's sim...

    let mut simmer = ObservableSimmer::new(&game_config, &kwg, &klv);
    simmer.simmer.prepare(&game_config, &game_state, 2);
    let board_snapshot = &movegen::BoardSnapshot {
        board_tiles: &game_state.board_tiles,
        game_config: simmer.game_config,
        kwg: simmer.kwg,
        klv: simmer.klv,
    };
    let mut move_generator = movegen::KurniaMoveGenerator::new(&game_config);
    move_generator.gen_moves_unfiltered(&movegen::GenMovesParams {
        board_snapshot,
        rack: &game_state.current_player().rack,
        max_gen: 5,
        always_include_pass: false,
    });
    let mut candidates = simmer.take_candidates(move_generator.plays.len());
    let num_sim_iters = 10000;
    for sim_iter in 1..=num_sim_iters {
        let should_output = sim_iter % 50 == 0;
        if should_output {
            println!("\niter {sim_iter}");
        }
        simmer.simmer.prepare_iteration();
        for candidate in candidates.iter_mut() {
            let game_ended = simmer.simmer.simulate(
                simmer.game_config,
                simmer.kwg,
                simmer.klv,
                &move_generator.plays[candidate.play_index].play,
            );
            let final_spread = simmer.simmer.final_equity_spread();
            let win_prob = simmer.simmer.compute_win_prob(game_ended, final_spread);
            let sim_spread = final_spread - simmer.simmer.initial_score_spread as f32;
            candidate
                .stats
                .update(sim_spread as f64 + win_prob * simmer.simmer.win_prob_weightage());
            candidate.equity_stats.update(sim_spread as f64);
            candidate.win_rate_stats.update(win_prob);
        }
        if should_output {
            candidates
                .sort_unstable_by(|a, b| b.stats.mean().partial_cmp(&a.stats.mean()).unwrap());
            for (i, candidate) in (1..).zip(candidates.iter().take(10)) {
                println!(
                    "{:3} {:6.2} {:6.2} {}",
                    i,
                    candidate.equity_stats.mean(),
                    100.0 * candidate.win_rate_stats.mean(),
                    move_generator.plays[candidate.play_index]
                        .play
                        .fmt(board_snapshot)
                );
            }
        }
    }

    Ok(())
}
