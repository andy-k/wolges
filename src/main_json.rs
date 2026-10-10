// Copyright (C) 2020-2026 Andy Kurnia.

use rand::prelude::*;
use std::io::Write;
use wolges::{
    display, equity, error, game_config, game_state, kibitzer, klv, kwg, move_filter, move_picker,
    movegen, play_scorer,
};

mod game_args;

// tile numbering follows alphabet order (not necessarily unicode order).
// rack: array of numbers. 0 for blank, 1 for A.
// board: 2D array of numbers. 0 for empty, 1 for A, -1 for blank-as-A.
// count: maximum number of moves returned.
// (note: equal moves are not stably sorted;
//  different counts may tie-break the last move differently.)
#[derive(serde::Deserialize)]
struct Question {
    rack: Vec<u8>,
    #[serde(rename = "board")]
    board_tiles: Vec<Vec<i8>>,
    #[serde(rename = "count")]
    max_gen: usize,
}

// the built-in question is on a super board.
#[derive(clap::Parser)]
#[command(
    about = "generate the built-in question's moves and print them, then as json",
    mut_arg("preset", |preset| preset.default_value("super-english"))
)]
struct Cli {
    #[arg(long, help = "the word graph is a kbwg")]
    kbwg: bool,
    #[arg(long, help = "generate with the lean arm")]
    lean: bool,
    #[arg(default_value = "lexbin/CSW24.kwg", help = "the word graph")]
    kwg: String,
    #[arg(
        default_value = "lexbin/super-CSW24.klv2",
        help = "the leaves (- for none)"
    )]
    klv: String,
    #[command(flatten)]
    game: game_args::GameArgs,
}

fn main() -> error::Returns<()> {
    let cli: Cli = clap::Parser::parse();
    let game_config = cli.game.make_game_config()?;
    // of course this should be cached
    let klv = if cli.klv == "-" {
        klv::Klv::<kwg::Node22>::from_bytes_alloc(klv::EMPTY_KLV_BYTES)?
    } else {
        klv::Klv::<kwg::Node22>::from_bytes_alloc(&std::fs::read(&cli.klv)?)?
    };
    game_config.check_leaves(klv.leave_range())?;
    let kwg_bytes = std::fs::read(&cli.kwg)?;
    if cli.kbwg {
        let kwg = kwg::Kwg::<kwg::Node24>::from_bytes_alloc(&kwg_bytes)?;
        refuse_a_wider_graph(&kwg, &game_config, &cli.kwg)?;
        answer(&kwg, &klv, &game_config, cli.lean)
    } else {
        let kwg = kwg::Kwg::<kwg::Node22>::from_bytes_alloc(&kwg_bytes)?;
        refuse_a_wider_graph(&kwg, &game_config, &cli.kwg)?;
        answer(&kwg, &klv, &game_config, cli.lean)
    }
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
fn answer<N: kwg::Node + Sync>(
    kwg: &kwg::Kwg<N>,
    klv: &klv::Klv<kwg::Node22>,
    game_config: &game_config::GameConfig,
    lean: bool,
) -> error::Returns<()> {
    let data = r#"
      {
        "actual_lexicon": "CSW19",
        "lexicon": "CSW24",
        "xrack": [ 1, 3, 10, 16, 17, 18, 19 ],
        "xboard": [
          [  0,  0,  0,  0,  0,  0,  0, 18,  0,  0,  0,  8, 15, 12,  4 ],
          [  0,  0,  0,  0,  9,  4,  5,  1, 20,  9, 22, -5,  0,  0,  0 ],
          [  0,  0,  0,  0,  0,  0,  0,  9,  0,  0,  0,  0,  0,  0,  0 ],
          [  0,  0,  0,  0,  0, 23,  0, 14,  0,  0,  0,  0,  0,  0,  0 ],
          [  0,  0,  0,  0,  0,  9,  0,  4,  0,  0,  0,  0,  0,  0,  0 ],
          [  0,  0,  0,  0,  0, 26,  0,  1,  0,  0,  0,  0,  0,  0,  0 ],
          [  0,  0,  0,  0,  0,  5,  0, 20,  0,  0,  0,  0,  0,  0,  0 ],
          [  0,  4, 15, 13,  9, 14,  5,  5,  0,  0,  0,  0,  0,  0,  0 ],
          [  7,  1,  2, 25,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0 ],
          [  0, 23, 15,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0 ],
          [  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0 ],
          [  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0 ],
          [  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0 ],
          [  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0 ],
          [  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0 ]
        ],
        "rack": [ 3, 4, 5, 12, 13, 15, 15 ],
        "board": [
          [  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0 ],
          [  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0 ],
          [  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0 ],
          [  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0 ],
          [  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0 ],
          [  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0 ],
          [  0,  0,  0,  0,  0,  0,  0,  0, 26,  1,  7,  0,  0,  0,  0 ],
          [  0,  0,  0,  0,  0,  1, 11,  5,  5,  0,  0,  0,  0,  0,  0 ],
          [  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0 ],
          [  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0 ],
          [  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0 ],
          [  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0 ],
          [  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0 ],
          [  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0 ],
          [  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0 ]
        ],
        "count": 15
      }
    "#;
    let _ = data;
    let data = r#"
      {
          "source": "https://woogles.io/game/CHWqqBC7?turn=11",
          "rack": [1, 2, 3, 7, 7, 17, 22],
          "board": [
            [0, 0, 0, 0, 0, 0, 0, 8,15,22, 5, 0, 0, 0, 0],
            [0, 0, 0, 0, 0, 0, 0, 0, 0, 0,18, 0, 0, 0, 0],
            [0, 0, 0, 0, 0, 0, 0, 0,26, 5, 1,12, 0, 0, 0],
            [0, 0, 0, 3, 1,12, 9,16, 5,18,19, 0, 0, 0, 0],
            [0, 0, 0, 0,20, 9,14, 0, 0, 0, 9, 0, 0, 0, 0],
            [0, 0, 0, 0, 0, 0, 0, 0, 0,16,15, 0, 0, 0, 0],
            [0, 0, 0, 0, 0, 0,15, 6, 0,21,14, 0, 0, 0, 0],
            [0, 0, 0, 0, 0, 0,23, 5,12,20, 0, 0, 0, 0, 0],
            [0, 0, 0, 0, 0, 0, 0, 0, 9, 0, 0, 0, 0, 0, 0],
            [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
            [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
            [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
            [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
            [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
            [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]

          ],
          "actual_lexicon": "CSW21",
          "lexicon": "CSW24",
          "actual_leave": "CSW21",
          "leave": "CSW24",
          "rules": "CrosswordGame",
          "count": 150000
      }
    "#;
    let _ = data;
    let data = r#"
      {
          "source": "https://woogles.io/game/4jUemdhr?turn=106",
          "rack": [2, 3, 5, 12, 14, 16, 19],
          "board": [
 [0, 1, 18, 0, 15, 14, 25, 0, 4, 9, 0, 8, 15, 0, 16, 8, 15, 0, 1, 20, 0],
 [0, 0, 5, 0, 0, 1, 0, 0, 0, 20, 0, 15, 0, 0, 0, 0, 22, 0, 12, 0, 0],
 [0, 0, 9, 0, 0, 15, 21, 20, 4, 1, 20, 5, 4, 0, 0, 7, 5, 14, 20, 0, 0],
 [0, 0, 14, 1, 26, 9, 0, 0, 0, 0, 21, 0, 0, 14, 0, 0, 18, 0, 0, 0, 0],
 [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 9, 14, 6, 9, 18, 13, 19, 0, 0, 0, 0],
 [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 14, 0, 0, 24, 9, 0, 0, 0, 0, 0, 0],
 [0, 0, 0, 0, 0, 0, 0, 0, 0, 10, 1, 23, 0, 0, -26, 0, 0, 0, 0, 0, 0],
 [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 5, 24, 0, 0, 0, 0, 0, 0, 0, 0],
 [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 4, 9, 0, 0, 0, 0, 0, 0, 0, 0],
 [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 7, 0, 0, 0, 0, 0, 0, 0, 0, 0],
 [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 23, 5, 0, 0, 0, 0, 0, 0, 0, 0, 0],
 [0, 0, 0, 0, 0, 0, 0, 0, 0, 11, 5, 19, 8, 0, 0, 0, 0, 0, 0, 0, 0],
 [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 14, 0, 9, 0, 0, 0, 0, 0, 0, 0, 0],
 [0, 0, 0, 0, 0, 0, 0, 0, 0, 19, 20, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
 [0, 0, 0, 0, 0, 0, 0, 0, 0, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
 [0, 0, 0, 0, 0, 0, 0, 0, 0, 22, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
 [0, 0, 0, 0, 15, 2, 15, 5, 0, 5, 0, 0, 0, 0, 0, 0, 25, 0, 0, 0, 0],
 [0, 0, 1, 13, 9, 0, 0, 13, 5, 18, 12, 19, 0, 0, 7, 0, 5, 18, 18, 0, 0],
 [0, 0, 7, 0, 12, 15, 0, 0, 18, 0, 0, 15, 18, 7, 5, 1, 20, 0, 1, 0, 0],
 [0, 0, 1, 0, 0, 15, 0, 0, 5, 0, 0, -16, 0, 0, -5, 0, 0, 0, 9, 0, 0],
 [0, 1, 18, 0, 15, 14, 25, 0, 4, 9, 0, 8, 15, 0, 16, 8, 15, 0, 1, 20, 0]

          ],
          "actual_lexicon": "super-CSW21",
          "lexicon": "super-CSW24",
          "count": 15
      }
    "#;
    /*
      .AR.ONY.DI.HO.PHO.AT.
      ..E..A...T.O....V.L..
      ..I..OUTDATED..GENT..
      ..NAZI....U..N..R....
      ..........INFIRMS....
      ..........N..XI......
      .........JAW..z......
      ...........EX........
      ...........DI........
      ...........G.........
      ..........WE.........
      .........KESH........
      ..........N.I........
      .........ST..........
      .........A...........
      .........V...........
      ....OBOE.E......Y....
      ..AMI..MERLS..G.ERR..
      ..G.LO..R..ORGEAT.A..
      ..A..O..E..p..e...I..
      .AR.ONY.DI.HO.PHO.AT.
    */
    let question = serde_json::from_str::<Question>(data)?;

    let mut rng = rand_xoshiro::Xoshiro256PlusPlus::try_from_rng(&mut rand::rngs::SysRng)?;
    let mut game_state = game_state::GameState::new(game_config);
    if game_state.players.len() < 2 {
        wolges::return_error!("the question scores two players".to_string());
    }
    // temp hardcode
    game_state.players[0].score = equity::scale_score(16);
    game_state.players[1].score = equity::scale_score(44);

    let mut kibitzer = kibitzer::Kibitzer::new();
    kibitzer.prepare(game_config, &question.rack, &question.board_tiles)?;

    display::print_board(
        game_config.alphabet(),
        game_config.board_layout(),
        &kibitzer.board_tiles,
    )?;

    let arm_tables = wolges::anagrams::ArmTables::build(kwg, game_config, lean)?;
    let mut move_filter = move_filter::GenMoves::Unfiltered;
    let mut move_picker = move_picker::MovePicker::Simmer(Box::new(move_picker::Simmer::new(
        game_config,
        kwg,
        &arm_tables,
        klv,
        move_picker::SimmerParams {
            num_sim_iters: move_picker::DEFAULT_NUM_SIM_ITERS,
            num_sim_plies: move_picker::DEFAULT_NUM_SIM_PLIES,
            sim_threads: 1,
            win_pct_table: None,
        },
    )?));
    game_state
        .board_tiles
        .copy_from_slice(&kibitzer.board_tiles);

    // put the bag and shuffle it
    game_state.bag.set_from_iter(
        (0u8..)
            .zip(kibitzer.available_tally.iter())
            .flat_map(|(tile, &count)| std::iter::repeat_n(tile, count as usize)),
    );
    game_state.bag.shuffle(&mut rng);

    let mut move_generator = movegen::KurniaMoveGenerator::new(game_config);
    let board_snapshot = &movegen::BoardSnapshot {
        board_tiles: &kibitzer.board_tiles,
        game_config,
        kwg,
        anagrams: arm_tables.anagrams(),
        rack_lengths: arm_tables.rack_lengths(),
        klv,
    };

    if false {
        for &tile in &question.rack {
            game_state.players[0].rack.push(tile);
        }
        move_picker.pick_a_move(
            &mut move_filter,
            &mut move_generator,
            board_snapshot,
            &game_state,
            &game_state.current_player().rack,
        )?;
        let plays = &move_generator.plays;
        writeln!(std::io::stdout(), "found {} moves", plays.len())?;
        for play in plays.iter() {
            writeln!(
                std::io::stdout(),
                "{} {}",
                play.equity,
                play.play.fmt(board_snapshot)
            )?;
        }
    }

    move_generator.gen_moves_unfiltered(&movegen::GenMovesParams {
        board_snapshot,
        rack: &question.rack,
        max_gen: question.max_gen,
        num_exchanges_by_this_player: game_state.current_player().num_exchanges,
        pass_policy: movegen::PassPolicy::OnlyWhenForced,
        dynamic_leaves: None,
    });
    let plays = &move_generator.plays;

    writeln!(std::io::stdout(), "found {} moves", plays.len())?;
    let mut ps = play_scorer::PlayScorer::new();
    for play in plays.iter() {
        writeln!(
            std::io::stdout(),
            "{} {}",
            play.equity,
            play.play.fmt(board_snapshot)
        )?;

        let movegen_score = match &play.play {
            movegen::Play::Exchange { .. } => 0,
            movegen::Play::Place { score, .. } => *score,
        };
        let recounted_score = ps.compute_score(board_snapshot, &play.play);
        if movegen_score != recounted_score {
            writeln!(
                std::io::stdout(),
                "{} should score {} instead of {}!",
                play.play.fmt(board_snapshot),
                recounted_score,
                movegen_score
            )?;
        }
    }

    let result = plays
        .iter()
        .map(|x| x.into())
        .collect::<Vec<kibitzer::JsonPlayWithEquity>>();
    let ret = serde_json::to_value(result)?;
    writeln!(std::io::stdout(), "{ret}")?;
    writeln!(std::io::stdout(), "{}", serde_json::to_string_pretty(&ret)?)?;

    Ok(())
}
