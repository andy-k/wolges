// Copyright (C) 2020-2026 Andy Kurnia.

// a frozen corpus of generations on the tests' own word list: each pick, one
// case's moves (on a board given or reached by a game played out) asked of an
// arm one way, generates them as a new generator of the first arm does, and
// that list is what it was frozen as

use super::tests::{
    SWEEP_RACKS, SWEEP_WORDS, parse_test_rack, sweep_kwg_covering, test_kad, uneven_game_config,
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
#[derive(PartialEq, Debug)]
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
            let play = format!("{}", p.play.fmt(board_snapshot).unwrap());
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
fn set_of<'a>(list: impl Iterator<Item = &'a ValuedMove>, places_only: bool) -> Vec<&'a Play> {
    let mut set = list
        .map(|m| &m.play)
        .filter(|p| !places_only || matches!(p, Play::Place { .. }))
        .collect::<Vec<_>>();
    set.sort_unstable_by(|a, b| cmp_play(a, b));
    set
}

#[inline]
fn lane_of(play: &Play) -> Option<(bool, i8)> {
    match play {
        Play::Place { down, lane, .. } => Some((*down, *lane)),
        Play::Exchange { .. } => None,
    }
}

// the lists alike, each move with its equity, in order; listed when not
#[inline]
fn assert_same<N: kwg::Node, L: kwg::Node>(
    got: &[ValuedMove],
    want: &[ValuedMove],
    board_snapshot: &BoardSnapshot<'_, N, L>,
    at: &dyn Fn() -> String,
) {
    if got.len() != want.len()
        || got
            .iter()
            .zip(want)
            .any(|(g, w)| g.equity != w.equity || g.play != w.play)
    {
        let at = at();
        assert_eq!(
            listed(got, board_snapshot),
            listed(want, board_snapshot),
            "{at}"
        );
        panic!("{at}: alike as listed, not as moves");
    }
}

// the sets alike; listed when not
#[inline]
fn assert_same_set<N: kwg::Node, L: kwg::Node>(
    got: &[&Play],
    want: &[&Play],
    board_snapshot: &BoardSnapshot<'_, N, L>,
    at: &dyn Fn() -> String,
) {
    if got != want {
        let at = at();
        let text = |set: &[&Play]| {
            set.iter()
                .map(|p| format!("{}", p.fmt(board_snapshot).unwrap()))
                .collect::<Vec<_>>()
        };
        assert_eq!(text(got), text(want), "{at}");
        panic!("{at}: alike as listed, not as moves");
    }
}

struct Ask<'a> {
    rack: &'a [u8],
    pass_policy: PassPolicy,
    num_exchanges_by_this_player: i16,
    masks: &'a [(u128, u128)],
    run_letter_bits: &'a [u64],
    at: &'a str,
    way: &'a str,
}

// one arm's generator asked every way for a case's moves, each set against
// expected (the whole list, best first)
#[inline]
fn check_arm<G: LaneBits, const JUMPS: bool, const ANY_SIDE: bool, N: kwg::Node, L: kwg::Node>(
    generator: &mut KurniaMoveGenerator<G, JUMPS, ANY_SIDE>,
    call: Call,
    board_snapshot: &BoardSnapshot<'_, N, L>,
    ask: &Ask<'_>,
    expected: &[ValuedMove],
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
    let generate = |g: &mut KurniaMoveGenerator<G, JUMPS, ANY_SIDE>, max_gen: usize| match call {
        Call::Tables => g.gen_moves_unfiltered(&params(max_gen)).unwrap(),
        Call::Lean => g.gen_moves_unfiltered_lean(&params(max_gen)).unwrap(),
    };
    let asked = |way: &str| ask.way == way || ask.way == "every";
    // all, then the best few, each the head of all
    if asked("all") {
        generate(generator, usize::MAX);
        assert_same(&generator.plays, expected, board_snapshot, &|| {
            format!("{at}, all")
        });
    }
    for top in TOPS
        .into_iter()
        .filter(|top| asked(&format!("the best {top}")))
    {
        generate(generator, top);
        assert_same(
            &generator.plays,
            &expected[..top.min(expected.len())],
            board_snapshot,
            &|| format!("{at}, the best {top}"),
        );
    }
    // raw: every move, unsorted and with no equity
    if asked("raw") {
        match call {
            Call::Tables => generator
                .gen_moves_raw_all_unsorted(
                    board_snapshot,
                    ask.rack,
                    ask.num_exchanges_by_this_player,
                    ask.pass_policy,
                )
                .unwrap(),
            Call::Lean => generator
                .gen_moves_raw_all_unsorted_lean(
                    board_snapshot,
                    ask.rack,
                    ask.num_exchanges_by_this_player,
                    ask.pass_policy,
                )
                .unwrap(),
        }
        assert_same_set(
            &set_of(generator.plays.iter(), false),
            &set_of(expected.iter(), false),
            board_snapshot,
            &|| format!("{at}, raw"),
        );
    }
    // some lanes alone, sorted and raw, and the endgame's walk by runs there
    for &(rows, cols) in ask.masks {
        let lanes = format!("{rows:#x} {cols:#x}");
        let on = expected
            .iter()
            .filter(|m| on_lanes(lane_of(&m.play), rows, cols))
            .cloned()
            .collect::<Vec<_>>();
        // where the lanes hold no move, a pass only when forced is forced
        let forced = on.is_empty() && matches!(ask.pass_policy, PassPolicy::OnlyWhenForced);
        let passed = |g: &KurniaMoveGenerator<G, JUMPS, ANY_SIDE>| {
            listed(&g.plays, board_snapshot)
                .into_iter()
                .map(|l| l.play)
                .collect::<Vec<_>>()
        };
        if asked(&format!("lanes {lanes}")) {
            match call {
                Call::Tables => {
                    generator
                        .gen_moves_unfiltered_lanes(&params(usize::MAX), rows, cols)
                        .unwrap();
                }
                Call::Lean => {
                    generator
                        .gen_moves_unfiltered_lanes_lean(&params(usize::MAX), rows, cols)
                        .unwrap();
                }
            }
            if forced {
                assert_eq!(
                    passed(generator),
                    ["Pass"],
                    "{at}, lanes {rows:#x} {cols:#x}, forced"
                );
            } else {
                assert_same(&generator.plays, &on, board_snapshot, &|| {
                    format!("{at}, lanes {rows:#x} {cols:#x}")
                });
            }
        }
        if asked(&format!("raw lanes {lanes}")) {
            match call {
                Call::Tables => generator
                    .gen_moves_raw_lanes_unsorted(
                        board_snapshot,
                        ask.rack,
                        ask.num_exchanges_by_this_player,
                        ask.pass_policy,
                        rows,
                        cols,
                    )
                    .unwrap(),
                Call::Lean => generator
                    .gen_moves_raw_lanes_unsorted_lean(
                        board_snapshot,
                        ask.rack,
                        ask.num_exchanges_by_this_player,
                        ask.pass_policy,
                        rows,
                        cols,
                    )
                    .unwrap(),
            }
            if forced {
                assert_eq!(
                    passed(generator),
                    ["Pass"],
                    "{at}, raw lanes {rows:#x} {cols:#x}, forced"
                );
            } else {
                assert_same_set(
                    &set_of(generator.plays.iter(), false),
                    &set_of(on.iter(), false),
                    board_snapshot,
                    &|| format!("{at}, raw lanes {rows:#x} {cols:#x}"),
                );
            }
        }
        if asked(&format!("by runs {lanes}")) {
            generator
                .gen_moves_raw_lanes_unsorted_by_runs(
                    ask.run_letter_bits,
                    board_snapshot,
                    ask.rack,
                    rows,
                    cols,
                )
                .unwrap();
            assert_same_set(
                &set_of(generator.plays.iter(), true),
                &set_of(on.iter(), true),
                board_snapshot,
                &|| format!("{at}, by runs {rows:#x} {cols:#x}"),
            );
        }
    }
    // the async lean arm's
    if call == Call::Lean && asked("async") {
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
            loop {
                if let std::task::Poll::Ready(result) = fut.as_mut().poll(&mut cx) {
                    result.unwrap();
                    break;
                }
            }
        }
        assert_same(&generator.plays, expected, board_snapshot, &|| {
            format!("{at}, async")
        });
    }
}

// a 64-bit FNV-1a of a list's lines, each ended by a newline, as given in
// https://datatracker.ietf.org/doc/html/draft-eastlake-fnv
#[inline]
fn hash_of(list: &[Listed]) -> u64 {
    let mut hash = 0xcbf2_9ce4_8422_2325u64;
    for l in list {
        for &b in l.line.as_bytes().iter().chain(b"\n") {
            hash = (hash ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3);
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
        leaves.push((vec![a], ((i32::from(a) * 37) % 17) as f32 * 2.5 - 20.0));
        if a < paired {
            for b in a..paired {
                leaves.push((
                    vec![a, b],
                    ((i32::from(a) * 5 + i32::from(b) * 3) % 13) as f32 * 3.0 - 18.0,
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

// a case's moves, by its name, asked of an arm (by its name, ending " on the
// wasm's" for that generator) one way, every way, or none (the frozen list
// alone)
struct Pick {
    case: &'static str,
    arm: &'static str,
    way: &'static str,
}

// the generators kept across a config's whole corpus, one an arm, and the
// wasm's for the lean arms where its board fits it
struct Kept {
    arms: Vec<KurniaMoveGenerator>,
    wasm: Vec<Option<KurniaMoveGenerator<u32, false, false>>>,
}

#[inline]
fn run_config(config: &Config<'_>, picks: &[Pick], out: &mut String) {
    let gc = &config.gc;
    let kwg = match gc.game_rules() {
        game_config::GameRules::Classic | game_config::GameRules::Woadwage => {
            sweep_kwg_covering(gc)
        }
        game_config::GameRules::Jumbled | game_config::GameRules::JumbledWoadwage => {
            test_kad(gc, SWEEP_WORDS)
        }
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
                let case = format!("{} {klv_name} {fen} {rack}", config.name);
                if !picks.iter().any(|p| p.case.starts_with(&case)) {
                    continue;
                }
                let rack_tiles = parse_test_rack(gc.alphabet(), rack);
                run_case(
                    config,
                    picks,
                    CaseArms {
                        kwg: &kwg,
                        klv,
                        arms: &arms,
                    },
                    &mut kept,
                    CasePosition {
                        board_tiles: &board_tiles,
                        rack: &rack_tiles,
                        masks: &masks,
                        run_letter_bits: &run_letter_bits,
                        name: &case,
                        game: false,
                    },
                    out,
                );
            }
        }
        // a game played out from the bag, the board changing each turn
        let game = format!("{} {klv_name} game ", config.name);
        if !picks.iter().any(|p| p.case.starts_with(&game)) {
            continue;
        }
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
                picks,
                CaseArms {
                    kwg: &kwg,
                    klv,
                    arms: &arms,
                },
                &mut kept,
                CasePosition {
                    board_tiles: &board_tiles,
                    rack: &rack,
                    masks: &masks,
                    run_letter_bits: &run_letter_bits,
                    name: &format!("{} {klv_name} game turn {turn} {rack_str}", config.name),
                    game: true,
                },
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
                    board_tiles[strider.at_in_lane(i)] = tile;
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

struct CasePosition<'a> {
    board_tiles: &'a [u8],
    rack: &'a [u8],
    masks: &'a [(u128, u128)],
    run_letter_bits: &'a [u64],
    name: &'a str,
    game: bool,
}

struct CaseArms<'a, 'b> {
    kwg: &'a kwg::Kwg<kwg::Node24>,
    klv: &'a klv::Klv<kwg::Node24>,
    arms: &'a [Arm<'b>],
}

// a case on every arm, every pass policy, exchanges allowed or used up; the
// best place move of the first arm, a pass only when forced, exchanges allowed
#[inline]
fn run_case<'a>(
    config: &Config<'_>,
    picks: &[Pick],
    CaseArms { kwg, klv, arms }: CaseArms<'_, 'a>,
    kept: &mut Kept,
    CasePosition {
        board_tiles,
        rack,
        masks,
        run_letter_bits,
        name: case,
        game,
    }: CasePosition<'_>,
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
            let picked = picks.iter().filter(|p| p.case == at).collect::<Vec<_>>();
            let leads = game && best.is_none();
            if picked.is_empty() && !leads {
                continue;
            }
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
                Call::Tables => fresh.gen_moves_unfiltered(&params).unwrap(),
                Call::Lean => fresh.gen_moves_unfiltered_lean(&params).unwrap(),
            }
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
            if picked.is_empty() {
                continue;
            }
            let expected = listed(&fresh.plays, &first);
            for pick in picked.iter().filter(|p| p.way != "none") {
                let wasm_arm = pick.arm.strip_suffix(" on the wasm's");
                let Some(i) = arms
                    .iter()
                    .position(|arm| arm.name == wasm_arm.unwrap_or(pick.arm))
                else {
                    panic!("{at}: no arm {}", pick.arm);
                };
                let arm = &arms[i];
                let board_snapshot = snapshot(arm);
                let ask = Ask {
                    rack,
                    pass_policy,
                    num_exchanges_by_this_player,
                    masks,
                    run_letter_bits,
                    at: &format!("{at}, {}", pick.arm),
                    way: pick.way,
                };
                if wasm_arm.is_none() {
                    check_arm(
                        &mut kept.arms[i],
                        arm.call,
                        &board_snapshot,
                        &ask,
                        &fresh.plays,
                    );
                } else {
                    let wasm = kept.wasm[i].as_mut().expect("the wasm's generator fits");
                    check_arm(wasm, arm.call, &board_snapshot, &ask, &fresh.plays);
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
fn corpus_output(picks: &[Pick]) -> String {
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
            name: "punctured",
            gc: game_config::make_punctured_english_game_config(),
            boards: &["15/15/15/15/15/15/15/6CAT6/15/15/15/15/15/15/15"],
            racks: &["AEINRST", "?EINRST"],
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
            boards: &[
                fifteen[0],
                "15/15/15/15/15/15/15/12A1A/15/15/15/15/15/15/15",
            ],
            racks: &jumbled_racks,
            bag: jumbled_bag,
        },
    ];
    let mut out = String::new();
    for config in &configs {
        run_config(config, picks, &mut out);
    }
    for pick in picks {
        assert!(
            out.contains(&format!("== {}: ", pick.case)),
            "no case {}",
            pick.case
        );
    }
    out
}

#[inline]
fn picks() -> [Pick; 12] {
    [
        // the first list: spans, bounds, and subracks
        Pick {
            case: "english no leaves 15/15/15/15/15/15/15/15/15/15/15/15/15/15/15 AEINRST forced 0",
            arm: "",
            way: "none",
        },
        // a blank in the subracks
        Pick {
            case: "english no leaves 15/15/15/15/15/15/15/15/15/15/15/15/15/15/15 ?EINRST forced 0",
            arm: "",
            way: "none",
        },
        // blanks on blanks
        Pick {
            case: "english no leaves 15/15/15/15/15/15/15/15/15/15/15/15/15/15/15 ?ANTS?? forced 0",
            arm: "",
            way: "none",
        },
        // the last lanes
        Pick {
            case: "english no leaves CARE11/15/15/15/15/15/15/15/15/15/15/15/15/15/11CARE AEINRST forced 0",
            arm: "",
            way: "none",
        },
        // the wasm's lean walk on two columns
        Pick {
            case: "english no leaves 15/15/15/15/15/15/15/6CARE5/15/15/15/15/15/15/15 ?ANTS?? forced 0",
            arm: "lean bare on the wasm's",
            way: "lanes 0x0 0x240",
        },
        // jumbled arrangements
        Pick {
            case: "jumbled no leaves 15/15/15/15/15/15/15/15/15/15/15/15/15/15/15 AEINRST forced 0",
            arm: "",
            way: "none",
        },
        // a jumbled opening
        Pick {
            case: "jumbled no leaves game turn 0 ACENRST forced 0",
            arm: "",
            way: "none",
        },
        // a word between holes, and the squares beside them
        Pick {
            case: "punctured no leaves 15/15/15/15/15/15/15/6CAT6/15/15/15/15/15/15/15 AEINRST forced 0",
            arm: "",
            way: "none",
        },
        Pick {
            case: "punctured no leaves 15/15/15/15/15/15/15/6CAT6/15/15/15/15/15/15/15 ?EINRST forced 0",
            arm: "lean",
            way: "every",
        },
        // a square between two lone tiles at a lane's end
        Pick {
            case: "jumbled no leaves 15/15/15/15/15/15/15/12A1A/15/15/15/15/15/15/15 ?AT forced 0",
            arm: "",
            way: "none",
        },
        // the lean arm in a wide game
        Pick {
            case: "super no leaves game turn 3 AEIOSTT forced 0",
            arm: "lean",
            way: "every",
        },
        // the lean arm handed the turn, capped at one
        Pick {
            case: "uneven no leaves game turn 5 EINORST forced 0",
            arm: "handed back",
            way: "the best 1",
        },
    ]
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
    let full = corpus_output(&picks());
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
        counts_and_hashes(&corpus_output(&picks())),
    )
    .unwrap();
}
