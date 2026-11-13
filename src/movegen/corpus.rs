// Copyright (C) 2020-2026 Andy Kurnia.

// a frozen corpus of generations on the tests' own word list: boards empty,
// sparse, dense, and closed, and the boards of games played out turn by turn on
// generators kept across all of them; racks of no blank, one, and two, of
// repeated letters, and short; with no leaves and with some; a pass only when
// forced and as a candidate; exchanges allowed and used up. Every arm (the
// tables keyed by nibbles, by their layout, the lean arm with its table and
// with none, the wasm's generator) asked every way (all, the best few, raw, on
// some lanes, the endgame's walk by runs, the async lean arm) generates each
// case's moves the same, whole and in order, and the corpus is what it was
// frozen as

use super::tests::{
    LONG_RACKS, SWEEP_RACKS, SWEEP_WORDS, parse_test_rack, sweep_kwg_covering, test_kad,
    uneven_game_config,
};
use super::*;
use crate::{anagrams, display, game_config, klv, kwg};
use std::fmt::Write;

// how many of a case's moves the full output spells out, beside the count and
// the hash of all of them, for a case that moved
const SHOWN: usize = 8;

// the best few asked for, beside all
const TOPS: [usize; 5] = [1, 2, 3, 7, 15];

#[derive(Clone, Copy, PartialEq)]
enum Call {
    Tables,
    Lean,
}

struct Arm<'a> {
    name: &'static str,
    anagrams: Option<&'a anagrams::Anagrams>,
    rack_lengths: Option<&'a anagrams::RackLengths>,
    call: Call,
}

// a move as listed: its equity and the move, and the lane of a place move
#[derive(Clone, PartialEq, Debug)]
struct Listed {
    line: String,
    play: String,
    lane: Option<(bool, i8)>,
}

#[inline]
fn listed<N: kwg::Node, L: kwg::Node>(
    plays: &[ValuedMove],
    board_snapshot: &BoardSnapshot<'_, N, L>,
) -> Vec<Listed> {
    plays
        .iter()
        .map(|p| {
            let play = format!("{}", p.play.fmt(board_snapshot));
            Listed {
                line: format!("{} {play}", p.equity.raw()),
                play,
                lane: match p.play {
                    Play::Place { down, lane, .. } => Some((down, lane)),
                    Play::Exchange { .. } => None,
                },
            }
        })
        .collect()
}

#[inline]
fn on_lanes(lane: Option<(bool, i8)>, rows: u128, cols: u128) -> bool {
    match lane {
        Some((false, lane)) => rows >> lane & 1 != 0,
        Some((true, lane)) => cols >> lane & 1 != 0,
        None => true,
    }
}

// the moves of a list, as sets: each move with no equity, sorted
#[inline]
fn set_of<'a>(list: impl Iterator<Item = &'a Listed>, places_only: bool) -> Vec<&'a str> {
    let mut set = list
        .filter(|l| !places_only || l.lane.is_some())
        .map(|l| &l.play[..])
        .collect::<Vec<_>>();
    set.sort_unstable();
    set
}

struct Ask<'a> {
    rack: &'a [u8],
    pass_policy: PassPolicy,
    num_exchanges_by_this_player: i16,
    masks: &'a [(u128, u128)],
    run_letter_bits: &'a [u64],
    at: &'a str,
}

// one arm's generator asked every way for a case's moves, each set against
// expected (the whole list, best first)
#[inline]
fn check_arm<G: LaneBits, const JUMPS: bool, const ANY_SIDE: bool, N: kwg::Node, L: kwg::Node>(
    generator: &mut KurniaMoveGenerator<G, JUMPS, ANY_SIDE>,
    call: Call,
    board_snapshot: &BoardSnapshot<'_, N, L>,
    ask: &Ask<'_>,
    expected: &[Listed],
) {
    let at = ask.at;
    let params = |max_gen: usize| GenMovesParams {
        board_snapshot,
        rack: ask.rack,
        max_gen,
        num_exchanges_by_this_player: ask.num_exchanges_by_this_player,
        pass_policy: ask.pass_policy,
        dynamic_leaves: None,
    };
    let generate = |g: &mut KurniaMoveGenerator<G, JUMPS, ANY_SIDE>, max_gen: usize| {
        match call {
            Call::Tables => g.gen_moves_unfiltered(&params(max_gen)),
            Call::Lean => g.gen_moves_unfiltered_lean(&params(max_gen)),
        }
        listed(&g.plays, board_snapshot)
    };
    // all, then the best few, each the head of all
    assert_eq!(generate(generator, usize::MAX), expected, "{at}, all");
    for top in TOPS {
        assert_eq!(
            generate(generator, top),
            expected[..top.min(expected.len())],
            "{at}, the best {top}"
        );
    }
    // raw: every move, unsorted and with no equity
    match call {
        Call::Tables => generator.gen_moves_raw_all_unsorted(
            board_snapshot,
            ask.rack,
            ask.num_exchanges_by_this_player,
            ask.pass_policy,
        ),
        Call::Lean => generator.gen_moves_raw_all_unsorted_lean(
            board_snapshot,
            ask.rack,
            ask.num_exchanges_by_this_player,
            ask.pass_policy,
        ),
    }
    assert_eq!(
        set_of(listed(&generator.plays, board_snapshot).iter(), false),
        set_of(expected.iter(), false),
        "{at}, raw"
    );
    // some lanes alone, sorted and raw, and the endgame's walk by runs there
    for &(rows, cols) in ask.masks {
        let on = expected
            .iter()
            .filter(|l| on_lanes(l.lane, rows, cols))
            .cloned()
            .collect::<Vec<_>>();
        match call {
            Call::Tables => {
                generator.gen_moves_unfiltered_lanes(&params(usize::MAX), rows, cols);
            }
            Call::Lean => {
                generator.gen_moves_unfiltered_lanes_lean(&params(usize::MAX), rows, cols);
            }
        }
        // where the lanes hold no move, a pass only when forced is forced
        let forced = on.is_empty() && matches!(ask.pass_policy, PassPolicy::OnlyWhenForced);
        let on_lanes = listed(&generator.plays, board_snapshot);
        if forced {
            assert_eq!(
                on_lanes.iter().map(|l| &l.play[..]).collect::<Vec<_>>(),
                ["Pass"],
                "{at}, lanes {rows:#x} {cols:#x}, forced"
            );
        } else {
            assert_eq!(on_lanes, on, "{at}, lanes {rows:#x} {cols:#x}");
        }
        match call {
            Call::Tables => generator.gen_moves_raw_lanes_unsorted(
                board_snapshot,
                ask.rack,
                ask.num_exchanges_by_this_player,
                ask.pass_policy,
                rows,
                cols,
            ),
            Call::Lean => generator.gen_moves_raw_lanes_unsorted_lean(
                board_snapshot,
                ask.rack,
                ask.num_exchanges_by_this_player,
                ask.pass_policy,
                rows,
                cols,
            ),
        }
        let raw = listed(&generator.plays, board_snapshot);
        if forced {
            assert_eq!(
                set_of(raw.iter(), false),
                ["Pass"],
                "{at}, raw lanes {rows:#x} {cols:#x}, forced"
            );
        } else {
            assert_eq!(
                set_of(raw.iter(), false),
                set_of(on.iter(), false),
                "{at}, raw lanes {rows:#x} {cols:#x}"
            );
        }
        generator.gen_moves_raw_lanes_unsorted_by_runs(
            ask.run_letter_bits,
            board_snapshot,
            ask.rack,
            rows,
            cols,
        );
        assert_eq!(
            set_of(listed(&generator.plays, board_snapshot).iter(), true),
            set_of(on.iter(), true),
            "{at}, by runs {rows:#x} {cols:#x}"
        );
    }
    // the async lean arm's
    if call == Call::Lean {
        let params = params(usize::MAX);
        {
            let mut fut = std::pin::pin!(generator.gen_moves_filtered_async_lean(
                &params,
                PlacePredicate::AcceptAll,
                klv::AdjustLeave::Identity,
                EquityPredicate::AcceptAll,
                || std::future::ready(()),
            ));
            let mut cx = std::task::Context::from_waker(std::task::Waker::noop());
            while fut.as_mut().poll(&mut cx).is_pending() {}
        }
        assert_eq!(
            listed(&generator.plays, board_snapshot),
            expected,
            "{at}, async"
        );
    }
}

// a 64-bit FNV-1a of a list's lines, each ended by a newline, as given in
// https://datatracker.ietf.org/doc/html/draft-eastlake-fnv
#[inline]
fn hash_of(list: &[Listed]) -> u64 {
    let mut hash = 0xcbf2_9ce4_8422_2325u64;
    for l in list {
        for &b in l.line.as_bytes().iter().chain(b"\n") {
            hash = (hash ^ b as u64).wrapping_mul(0x0100_0000_01b3);
        }
    }
    hash
}

// leaves for every tile alone and for every two of the first few, values spread
// up to twenty points either way, so exchanges compete with places
#[inline]
fn some_leaves(gc: &game_config::GameConfig) -> klv::Klv<kwg::Node24> {
    let tiles = gc.alphabet().len();
    let paired = tiles.min(9);
    let mut leaves = Vec::<(Vec<u8>, f32)>::new();
    for a in 0..tiles {
        leaves.push((vec![a], ((a as i32 * 37) % 17) as f32 * 2.5 - 20.0));
        if a < paired {
            for b in a..paired {
                leaves.push((
                    vec![a, b],
                    ((a as i32 * 5 + b as i32 * 3) % 13) as f32 * 3.0 - 18.0,
                ));
            }
        }
    }
    leaves.sort_unstable_by(|x, y| x.0.cmp(&y.0));
    let leaves = leaves
        .iter()
        .map(|(tiles, value)| (&tiles[..], *value))
        .collect::<Vec<_>>();
    klv::kbwg_from_bytes_alloc(klv::make_klv2(&leaves)).unwrap()
}

// a game config's corpus: its boards and racks, and a game played out from bag
// on the empty board, each turn the best place move of the first arm
struct Config<'a> {
    name: &'static str,
    gc: game_config::GameConfig,
    boards: &'a [&'a str],
    racks: &'a [&'a str],
    bag: &'a str,
}

// the generators kept across a config's whole corpus, one an arm, and the
// wasm's for the lean arms where its board fits it
struct Kept {
    arms: Vec<KurniaMoveGenerator>,
    wasm: Vec<Option<KurniaMoveGenerator<u32, false, false>>>,
}

#[inline]
fn run_config(config: &Config<'_>, out: &mut String) {
    let gc = &config.gc;
    let kwg = match gc.game_rules() {
        game_config::GameRules::Classic => sweep_kwg_covering(gc),
        game_config::GameRules::Jumbled => test_kad(gc, SWEEP_WORDS),
    };
    let run_letter_bits = kwg.compute_letter_bits().letter_bits;
    let detected = anagrams::ArmTables::build(&kwg, gc, false).unwrap();
    let by_layout = anagrams::ArmTables::build_by_layout(&kwg, gc).unwrap();
    let lean = anagrams::ArmTables::build(&kwg, gc, true).unwrap();
    let mut arms = Vec::new();
    if let Some(anagrams) = detected.anagrams() {
        arms.push(Arm {
            name: if anagrams.nibble_side().is_some() {
                "nibbles"
            } else {
                "layout"
            },
            anagrams: Some(anagrams),
            rack_lengths: None,
            call: Call::Tables,
        });
    }
    if let Some(anagrams) = by_layout.anagrams()
        && detected
            .anagrams()
            .is_some_and(|a| a.nibble_side().is_some())
    {
        arms.push(Arm {
            name: "layout",
            anagrams: Some(anagrams),
            rack_lengths: None,
            call: Call::Tables,
        });
    }
    arms.push(Arm {
        name: "lean",
        anagrams: None,
        rack_lengths: lean.rack_lengths(),
        call: Call::Lean,
    });
    arms.push(Arm {
        name: "lean bare",
        anagrams: None,
        rack_lengths: None,
        call: Call::Lean,
    });
    // the tables' call with no tables hands the turn to the lean arm
    arms.push(Arm {
        name: "handed back",
        anagrams: None,
        rack_lengths: lean.rack_lengths(),
        call: Call::Tables,
    });
    let dim = gc.board_layout().dim();
    let masks = [
        (1u128 << (dim.rows as u32 / 2), 0u128),
        (
            0,
            (1 << (dim.cols as u32 / 2 - 1)) | (1 << (dim.cols as u32 / 2 + 2)),
        ),
        (
            !0u128 >> (128 - dim.rows as u32) & (u128::MAX / 3),
            !0u128 >> (128 - dim.cols as u32) & (u128::MAX / 3 * 2),
        ),
    ];
    let mut kept = Kept {
        arms: arms.iter().map(|_| KurniaMoveGenerator::new(gc)).collect(),
        wasm: arms
            .iter()
            .map(|arm| {
                (arm.call == Call::Lean)
                    .then(|| KurniaMoveGenerator::<u32, false, false>::try_new(gc).ok())
                    .flatten()
            })
            .collect(),
    };
    let mut fen_parser = display::BoardFenParser::new(gc.alphabet(), gc.board_layout());
    let none = klv::kbwg_from_bytes_alloc(klv::EMPTY_KLV_BYTES.to_vec()).unwrap();
    let some = some_leaves(gc);
    for (klv_name, klv) in [("no leaves", &none), ("some leaves", &some)] {
        for fen in config.boards {
            let board_tiles = fen_parser.parse(fen).unwrap().to_vec();
            for rack in config.racks {
                let rack_tiles = parse_test_rack(gc.alphabet(), rack);
                run_case(
                    config,
                    &kwg,
                    klv,
                    &arms,
                    &mut kept,
                    &board_tiles,
                    &rack_tiles,
                    &masks,
                    &run_letter_bits,
                    &format!("{} {klv_name} {fen} {rack}", config.name),
                    out,
                );
            }
        }
        // a game played out from the bag, the board changing each turn
        let mut board_tiles = vec![0u8; dim.rows as usize * dim.cols as usize];
        let mut bag = parse_test_rack(gc.alphabet(), config.bag);
        bag.reverse();
        let mut rack = Vec::new();
        for turn in 0.. {
            while rack.len() < gc.rack_size() as usize
                && let Some(tile) = bag.pop()
            {
                rack.push(tile);
            }
            rack.sort_unstable();
            let rack_str = rack
                .iter()
                .map(|&t| gc.alphabet().of_rack(t).unwrap())
                .collect::<String>();
            let best = run_case(
                config,
                &kwg,
                klv,
                &arms,
                &mut kept,
                &board_tiles,
                &rack,
                &masks,
                &run_letter_bits,
                &format!("{} {klv_name} game turn {turn} {rack_str}", config.name),
                out,
            );
            let Some(Play::Place {
                down,
                lane,
                idx,
                word,
                ..
            }) = best
            else {
                break;
            };
            let strider = dim.lane(down, lane);
            for (i, &tile) in (idx..).zip(word.iter()) {
                if tile != 0 {
                    board_tiles[strider.at(i)] = tile;
                    let played = tile & !((tile as i8) >> 7) as u8;
                    let at = rack.iter().position(|&t| t == played).unwrap();
                    rack.remove(at);
                }
            }
            if rack.is_empty() {
                break;
            }
        }
    }
}

// a case on every arm, every pass policy, exchanges allowed or used up; the
// best place move of the first arm, a pass only when forced, exchanges allowed
#[inline]
#[allow(clippy::too_many_arguments)]
fn run_case<'a>(
    config: &Config<'_>,
    kwg: &kwg::Kwg<kwg::Node24>,
    klv: &klv::Klv<kwg::Node24>,
    arms: &[Arm<'a>],
    kept: &mut Kept,
    board_tiles: &[u8],
    rack: &[u8],
    masks: &[(u128, u128)],
    run_letter_bits: &[u64],
    case: &str,
    out: &mut String,
) -> Option<Play> {
    let gc = &config.gc;
    let mut best = None;
    for (policy_name, pass_policy) in [
        ("forced", PassPolicy::OnlyWhenForced),
        ("candidate", PassPolicy::AsACandidate),
    ] {
        for num_exchanges_by_this_player in [0, gc.exchanges_allowed_per_player()] {
            let at = format!("{case} {policy_name} {num_exchanges_by_this_player}");
            let snapshot = |arm: &Arm<'a>| BoardSnapshot {
                board_tiles,
                game_config: gc,
                kwg,
                anagrams: arm.anagrams,
                rack_lengths: arm.rack_lengths,
                klv,
            };
            // the first arm's list, by a new generator, is the expected one
            let first = snapshot(&arms[0]);
            let mut fresh = KurniaMoveGenerator::new(gc);
            let params = GenMovesParams {
                board_snapshot: &first,
                rack,
                max_gen: usize::MAX,
                num_exchanges_by_this_player,
                pass_policy,
                dynamic_leaves: None,
            };
            match arms[0].call {
                Call::Tables => fresh.gen_moves_unfiltered(&params),
                Call::Lean => fresh.gen_moves_unfiltered_lean(&params),
            }
            let expected = listed(&fresh.plays, &first);
            if best.is_none()
                && matches!(pass_policy, PassPolicy::OnlyWhenForced)
                && num_exchanges_by_this_player == 0
            {
                best = Some(
                    fresh
                        .plays
                        .iter()
                        .find(|p| matches!(p.play, Play::Place { .. }))
                        .map(|p| p.play.clone()),
                );
            }
            for (i, arm) in arms.iter().enumerate() {
                let board_snapshot = snapshot(arm);
                let ask = Ask {
                    rack,
                    pass_policy,
                    num_exchanges_by_this_player,
                    masks,
                    run_letter_bits,
                    at: &format!("{at}, {}", arm.name),
                };
                check_arm(
                    &mut kept.arms[i],
                    arm.call,
                    &board_snapshot,
                    &ask,
                    &expected,
                );
                if let Some(wasm) = &mut kept.wasm[i] {
                    let ask = Ask {
                        at: &format!("{at}, {} on the wasm's", arm.name),
                        ..ask
                    };
                    check_arm(wasm, arm.call, &board_snapshot, &ask, &expected);
                }
            }
            writeln!(
                out,
                "== {at}: {} moves, hash {:016x}",
                expected.len(),
                hash_of(&expected)
            )
            .unwrap();
            for l in expected.iter().take(SHOWN) {
                writeln!(out, "{}", l.line).unwrap();
            }
        }
    }
    best.flatten()
}

#[inline]
fn corpus_output() -> String {
    let fifteen = [
        "15/15/15/15/15/15/15/15/15/15/15/15/15/15/15",
        "15/15/15/15/15/15/15/6CARE5/15/15/15/15/15/15/15",
        "15/15/15/15/15/15/6S8/6CARE5/6A8/6N8/6EATS5/9C5/9A5/9T5/15",
        "15/15/15/4CANT7/4A10/4R10/1SCARE5T3/4S6R3/10A4/7RATES3/10T4/10S4/15/15/15",
        "15/15/15/15/15/15/15/6N3A4/15/6N8/15/15/15/15/15",
        "CARE11/15/15/15/15/15/15/15/15/15/15/15/15/15/11CARE",
    ];
    let twenty_one = [
        "21/21/21/21/21/21/21/21/21/21/21/21/21/21/21/21/21/21/21/21/21",
        "21/21/21/21/21/21/21/21/21/21/8CARE9/8A12/8N12/8E12/21/21/21/21/21/21/21",
    ];
    let seventeen = [
        "17/17/17/17/17/17/17/17/17/17/17/17/17/17/17/17/17",
        "17/17/17/17/17/17/17/17/6CARE7/6A10/6N10/6E10/17/17/17/17/17",
    ];
    let short_racks = ["AEINRST", "?SATIRE", "??TANS", "NNNNTTT", "AE", "?"];
    let bag = "AEINRSTCARTONSESTATER?CANTERSEATNRSAT?ESTRACN";
    // a jumbled game takes any arrangement, so its lists run long: fewer blanks
    // and a shorter game
    let jumbled_racks = ["AEINRST", "?AT", "NNNNTTT", "AE", "?"];
    let jumbled_bag = "CARETSNIOTAE?S";
    let configs = [
        Config {
            name: "english",
            gc: game_config::make_english_game_config(),
            boards: &fifteen,
            racks: SWEEP_RACKS,
            bag,
        },
        Config {
            name: "super",
            gc: game_config::make_super_english_game_config(),
            boards: &twenty_one,
            racks: &short_racks,
            bag,
        },
        Config {
            name: "hong kong",
            gc: game_config::make_hong_kong_english_game_config(),
            boards: &fifteen[..3],
            racks: LONG_RACKS,
            bag,
        },
        Config {
            name: "uneven",
            gc: uneven_game_config(17, 17, 8, 8, false),
            boards: &seventeen,
            racks: &short_racks,
            bag,
        },
        Config {
            name: "jumbled",
            gc: game_config::make_jumbled_english_game_config(),
            boards: &fifteen[..3],
            racks: &jumbled_racks,
            bag: jumbled_bag,
        },
    ];
    let mut out = String::new();
    for config in &configs {
        run_config(config, &mut out);
    }
    out
}

// the baseline keeps each case's count and hash alone, a line a case in the
// corpus's order; the case it names and its first moves are in the full output
#[inline]
fn counts_and_hashes(full: &str) -> String {
    let mut out = String::new();
    for line in full.lines().filter(|l| l.starts_with("== ")) {
        let tail = &line[line.rfind(": ").unwrap() + 2..];
        let (count, hash) = tail.split_once(" moves, hash ").unwrap();
        writeln!(out, "{count} {hash}").unwrap();
    }
    out
}

#[inline]
#[test]
fn every_arm_asked_every_way_generates_the_frozen_corpus() {
    let full = corpus_output();
    let got = counts_and_hashes(&full);
    let want = include_str!("../movegen-corpus-baseline.txt");
    if got == want {
        return;
    }
    // each case that moved, by the full output's block for it
    let blocks = full
        .split("\n== ")
        .map(|b| b.trim_start_matches("== "))
        .collect::<Vec<_>>();
    let mut moved = String::new();
    let (got_lines, want_lines) = (
        got.lines().collect::<Vec<_>>(),
        want.lines().collect::<Vec<_>>(),
    );
    writeln!(
        moved,
        "{} cases now, {} in the baseline",
        got_lines.len(),
        want_lines.len()
    )
    .unwrap();
    for (i, (g, w)) in got_lines.iter().zip(&want_lines).enumerate() {
        if g != w {
            writeln!(moved, "case {i}, was {w}:\n== {}", blocks[i]).unwrap();
        }
    }
    panic!("the corpus moved; read each case before refreshing the baseline\n{moved}");
}

#[inline]
#[test]
#[ignore]
fn write_corpus_baseline() {
    std::fs::write(
        "src/movegen-corpus-baseline.txt",
        counts_and_hashes(&corpus_output()),
    )
    .unwrap();
}
