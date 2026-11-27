// Copyright (C) 2020-2026 Andy Kurnia.

#![cfg(test)]

use rand::SeedableRng;
use std::{collections::BTreeMap, path::PathBuf};
use wolges::{
    anagrams, build, endgame, equity, game_config, game_state, kibitzer, klv, kwg, movegen,
};

fn field<T: serde::de::DeserializeOwned>(value: &serde_json::Value, key: &str) -> T {
    serde_json::from_value(value[key].clone()).unwrap_or_else(|e| panic!("{key}: {e}"))
}

// each physical tile or pair is one deal, even if the letters match another
// deal. The two unseen Ts in the first position give a bag of T twice.
fn draw_counts(unseen: &[u8], bag: usize) -> BTreeMap<Vec<u8>, u32> {
    let mut draws = BTreeMap::new();
    match bag {
        1 => {
            for &tile in unseen {
                *draws.entry(vec![tile]).or_default() += 1;
            }
        }
        2 => {
            for (i, &first) in unseen.iter().enumerate() {
                for &second in &unseen[i + 1..] {
                    *draws.entry(vec![first, second]).or_default() += 1;
                }
            }
        }
        _ => panic!("expected a one- or two-tile bag, found {bag}"),
    }
    draws
}

#[test]
#[ignore = "requires an external CSW24.kwg"]
fn peg_draw_outcomes() {
    let cases: Vec<serde_json::Value> =
        serde_json::from_str(include_str!("fixtures/peg_draws.json")).unwrap();
    let graph = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src/lexbin/CSW24.kwg");
    let bytes = std::fs::read(&graph).unwrap_or_else(|e| panic!("{}: {e}", graph.display()));
    let kwg = kwg::kbwg_from_bytes_alloc(bytes, false).unwrap();
    let gc = game_config::make_english_game_config();
    let klv = klv::Klv::<kwg::Node24>::empty().unwrap();
    let arm_tables = anagrams::ArmTables::None;
    let mut rng = rand_xoshiro::Xoshiro256PlusPlus::seed_from_u64(0);

    for case in cases {
        let source = field::<String>(&case, "source");
        let deals = field::<u32>(&case, "deals");
        let req = &case["request"];
        assert_eq!(field::<String>(req, "lexicon"), "CSW24");
        assert_eq!(field::<String>(req, "rules"), "CrosswordGame");
        let rack = field::<Vec<u8>>(req, "rack");
        let turn = field::<u8>(req, "turn");
        let mut position = kibitzer::Kibitzer::new();
        position
            .prepare(&gc, &rack, &field::<Vec<Vec<i8>>>(req, "board"))
            .unwrap();
        let unseen = (0u8..)
            .zip(&position.available_tally)
            .flat_map(|(tile, &count)| std::iter::repeat_n(tile, count as usize))
            .collect::<Vec<_>>();
        let bag = unseen.len() - gc.rack_size() as usize;
        assert_eq!(bag, field::<usize>(&case, "bag"), "{source}");
        let draws = draw_counts(&unseen, bag);
        assert_eq!(draws.values().sum::<u32>(), deals, "{source}");

        let mover = turn as usize;
        let reply = mover ^ 1;
        let mut before = game_state::GameState::new(&gc);
        before.board_tiles.copy_from_slice(&position.board_tiles);
        before.turn = turn;
        before.pass_turns = field(req, "pass_turns");
        before.zero_turns = field(req, "zero_turns");
        before.players[mover].rack = rack;
        for (player, score) in before
            .players
            .iter_mut()
            .zip(field::<[i32; 2]>(req, "scores"))
        {
            player.score = equity::Points::new(score).scaled();
        }
        let snapshot = movegen::BoardSnapshot {
            board_tiles: &before.board_tiles,
            game_config: &gc,
            graphs: wolges::movegen::KwgForMovegen::for_game(&gc, &kwg),
            anagrams: None,
            rack_lengths: None,
            klv: &klv,
        };
        let mut generator = movegen::KurniaMoveGenerator::new(&gc);
        generator
            .gen_moves_raw_all_unsorted(
                &snapshot,
                &before.players[mover].rack,
                0,
                movegen::PassPolicy::AsACandidate,
            )
            .unwrap();

        for candidate in field::<Vec<serde_json::Value>>(&case, "candidates") {
            let play = movegen::Play::from(&field::<kibitzer::JsonPlay>(&candidate, "play"));
            let name = format!("{source} {}", field::<String>(&candidate, "name"));
            let margins = field::<BTreeMap<String, i32>>(&candidate, "margins");
            assert!(generator.plays.iter().any(|mv| mv.play == play), "{name}");
            let mut after_play = |draw: &[u8]| {
                let mut state = before.clone();
                let opp_rack = &mut state.players[reply].rack;
                opp_rack.clone_from(&unseen);
                game_state::use_tiles(opp_rack, draw.iter().copied()).unwrap();
                state.bag.set_from_iter(draw.iter().copied());
                state.play(&gc, &mut rng, &play).unwrap();
                assert!(state.bag.is_empty(), "{name}");
                state.next_turn().unwrap();
                state
            };
            let after = after_play(draws.first_key_value().unwrap().0);
            let mut words = Vec::new();
            movegen::KurniaMoveGenerator::new(&gc)
                .gen_remaining_words(
                    &movegen::BoardSnapshot {
                        board_tiles: &after.board_tiles,
                        ..snapshot
                    },
                    |word| words.push(word.into()),
                )
                .unwrap();
            let pruned = build::graph_for_game(&gc, &words).unwrap();
            let mut solver = endgame::EndgameSolver::<_, kwg::Node24>::new(
                &gc,
                wolges::movegen::KwgForMovegen::for_game(&gc, pruned.kwg()),
                &arm_tables,
            )
            .unwrap();

            assert_eq!(margins.len(), draws.len(), "{name}");
            let mut hypotheses = Vec::new();
            for (draw, &count) in &draws {
                let after = after_play(draw);
                solver.init_after_scoreless_turns(
                    &after.board_tiles,
                    [&after.players[0].rack, &after.players[1].rack],
                    after.pass_turns,
                    after.zero_turns,
                );
                let solved = solver
                    .solve_within(after.turn, &endgame::Limits::NONE)
                    .unwrap();
                let draw = gc.alphabet().fmt_rack(draw).to_string();
                assert!(solved.exact, "{name}, draw {draw}");
                let lead = after.players[mover].score - after.players[reply].score;
                let margin = lead - solved.value.unwrap();
                assert_eq!(
                    margin,
                    equity::Points::new(margins[&draw]).scaled(),
                    "{name}, draw {draw}",
                );
                hypotheses.push((count, margin));
            }
            let (win, _) = endgame::peg_aggregate(&hypotheses).unwrap();
            assert_eq!(
                win,
                f64::from(field::<u32>(&candidate, "win_deals")) / f64::from(deals),
                "{name}"
            );
        }
    }
}
