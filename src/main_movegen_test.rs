// Copyright (C) 2020-2026 Andy Kurnia.

use wolges::{alphabet, display, error, game_config, klv, kwg, movegen};

struct TestCase {
    fen: &'static str,
    rack: &'static str,
    max_gen: usize,
    always_include_pass: bool,
}

static TEST_CASES: &[TestCase] = &[
    // Empty board, common rack
    TestCase {
        fen: "15/15/15/15/15/15/15/15/15/15/15/15/15/15/15",
        rack: "AEINRST",
        max_gen: 15,
        always_include_pass: false,
    },
    // Empty board, rack with blank
    TestCase {
        fen: "15/15/15/15/15/15/15/15/15/15/15/15/15/15/15",
        rack: "?SATIRE",
        max_gen: 15,
        always_include_pass: false,
    },
    // After one move through center
    TestCase {
        fen: "15/15/15/15/15/15/15/4QUIRKY5/15/15/15/15/15/15/15",
        rack: "AEIOULD",
        max_gen: 15,
        always_include_pass: false,
    },
    // One word through center, always_include_pass=true (compare with case 10)
    TestCase {
        fen: "15/15/15/15/15/15/15/4WORD7/15/15/15/15/15/15/15",
        rack: "MOOORRT",
        max_gen: 15,
        always_include_pass: false,
    },
    // Hooks and blanks
    TestCase {
        fen: "15/15/15/15/15/15/15/4WORD7/15/15/15/15/15/15/15",
        rack: "?STLING",
        max_gen: 15,
        always_include_pass: false,
    },
    // Cross words
    TestCase {
        fen: "15/15/15/15/15/15/15/4WORD7/4I10/4N10/4E10/15/15/15/15",
        rack: "ABCDEFG",
        max_gen: 15,
        always_include_pass: false,
    },
    // Bingo rack, few tiles on board
    TestCase {
        fen: "15/15/15/15/15/15/7F7/7ALOW4/7N7/15/15/15/15/15/15",
        rack: "RETINAS",
        max_gen: 15,
        always_include_pass: false,
    },
    // Terrible rack, exchanges should appear
    TestCase {
        fen: "15/15/15/15/15/15/15/4WORD7/15/15/15/15/15/15/15",
        rack: "UUVVIIQ",
        max_gen: 30,
        always_include_pass: false,
    },
    // Only 2 tiles, limited plays, with always_include_pass=true
    TestCase {
        fen: "15/15/15/15/15/15/15/4WORD7/4I10/4N10/4E10/15/15/15/15",
        rack: "CS",
        max_gen: 15,
        always_include_pass: true,
    },
    // Double blank
    TestCase {
        fen: "15/15/15/15/15/15/15/4WORD7/15/15/15/15/15/15/15",
        rack: "??",
        max_gen: 15,
        always_include_pass: false,
    },
    // Same as case 3 but always_include_pass=false, pass should not appear
    TestCase {
        fen: "15/15/15/15/15/15/15/4WORD7/15/15/15/15/15/15/15",
        rack: "MOOORRT",
        max_gen: 15,
        always_include_pass: false,
    },
    // Late game (ZONULE position, 73 tiles)
    TestCase {
        fen: "ZONULE1B2APAID/1KY2RHANJA4/GAM4R2HUI2/7G6D/6FECIT3O/6AE1TOWIES/6I7E/1EnGUARD6D/NAOI2W8/6AT7/5PYE7/5L1L7/2COVE1L7/5X1E7/7N7",
        rack: "ST",
        max_gen: 15,
        always_include_pass: false,
    },
    // Same position, rack with few useful plays, pass not included
    TestCase {
        fen: "ZONULE1B2APAID/1KY2RHANJA4/GAM4R2HUI2/7G6D/6FECIT3O/6AE1TOWIES/6I7E/1EnGUARD6D/NAOI2W8/6AT7/5PYE7/5L1L7/2COVE1L7/5X1E7/7N7",
        rack: "OO",
        max_gen: 15,
        always_include_pass: false,
    },
    // Same as above but always_include_pass=true, pass should appear
    TestCase {
        fen: "ZONULE1B2APAID/1KY2RHANJA4/GAM4R2HUI2/7G6D/6FECIT3O/6AE1TOWIES/6I7E/1EnGUARD6D/NAOI2W8/6AT7/5PYE7/5L1L7/2COVE1L7/5X1E7/7N7",
        rack: "OO",
        max_gen: 100,
        always_include_pass: true,
    },
    // Bag empty (80 tiles on board, bag=6 < 7), no exchanges possible
    TestCase {
        fen: "5MOZ6S/2FIREPOTS4p/2Y2UTA1HWAN1E/DAK8L2C/OWE1BIB4E2I/CADGE6U1PA/I4DOGY2R1aT/L2GLORIA1LOVIE/E1XI1TAED2N1N1/RAI8S1T1/13I1/13E1/13R1/15/15",
        rack: "VUAENRU",
        max_gen: 15,
        always_include_pass: false,
    },
    // Same position, single tile, no valid plays, pass generated as fallback
    TestCase {
        fen: "5MOZ6S/2FIREPOTS4p/2Y2UTA1HWAN1E/DAK8L2C/OWE1BIB4E2I/CADGE6U1PA/I4DOGY2R1aT/L2GLORIA1LOVIE/E1XI1TAED2N1N1/RAI8S1T1/13I1/13E1/13R1/15/15",
        rack: "V",
        max_gen: 15,
        always_include_pass: false,
    },
    // Large rack (threat analysis: all unseen tiles), MultiLeaves overflow
    TestCase {
        fen: "15/15/15/15/15/15/15/4WORD7/15/15/15/15/15/15/15",
        rack: "??AAAAAAAAABBCDDDDEEEEEEEEEEEEFFGGGHHIIIIIIIIIJKLLLLMMNNNNNNOOOOOOOOPPQRRRRRRSSSSTTTTTTUVVWWXYYZ",
        max_gen: 5,
        always_include_pass: false,
    },
];

fn parse_rack(alphabet: &alphabet::Alphabet, rack_str: &str) -> Vec<u8> {
    let reader = alphabet::AlphabetReader::new_for_racks(alphabet);
    let sb = rack_str.as_bytes();
    let mut rack = Vec::new();
    let mut ix = 0;
    while ix < sb.len() {
        if let Some((tile, next_ix)) = reader.next_tile(sb, ix) {
            rack.push(tile);
            ix = next_ix;
        } else {
            panic!("unrecognized tile at position {ix} in rack {rack_str:?}");
        }
    }
    rack
}

fn main() -> error::Returns<()> {
    let kwg = kwg::Kwg::<kwg::Node22>::from_bytes_alloc(&std::fs::read("lexbin/CSW24.kwg")?);
    let klv = klv::Klv::<kwg::Node22>::from_bytes_alloc(&std::fs::read("lexbin/CSW24.klv2")?);
    let game_config = game_config::make_english_game_config();
    let alphabet = game_config.alphabet();
    let board_layout = game_config.board_layout();

    let mut fen_parser = display::BoardFenParser::new(alphabet, board_layout);
    let mut move_generator = movegen::KurniaMoveGenerator::new(&game_config);

    let check_mode = std::env::args().nth(1).as_deref() == Some("--check");
    let baseline = if check_mode {
        Some(std::fs::read_to_string("movegen-test-baseline.txt")?)
    } else {
        None
    };

    let mut output = String::new();
    let mut total_elapsed = std::time::Duration::ZERO;
    let mut total_moves = 0usize;

    for (case_idx, case) in TEST_CASES.iter().enumerate() {
        let board_tiles = fen_parser.parse(case.fen)?;
        let rack = parse_rack(alphabet, case.rack);

        let board_snapshot = movegen::BoardSnapshot {
            board_tiles,
            game_config: &game_config,
            kwg: &kwg,
            klv: &klv,
        };

        let t0 = std::time::Instant::now();
        move_generator.gen_moves_unfiltered(&movegen::GenMovesParams {
            board_snapshot: &board_snapshot,
            rack: &rack,
            max_gen: case.max_gen,
            num_exchanges_by_this_player: 0,
            always_include_pass: case.always_include_pass,
        });
        let elapsed = t0.elapsed();
        total_elapsed += elapsed;

        use std::fmt::Write;
        writeln!(output, "=== Case {case_idx}: rack={} fen={} ===",
            case.rack, case.fen).unwrap();
        for play in move_generator.plays.iter() {
            writeln!(output, "  {:.4} {}", play.equity, play.play.fmt(&board_snapshot)).unwrap();
        }
        eprintln!("Case {case_idx}: {} moves, {elapsed:?}", move_generator.plays.len());
        total_moves += move_generator.plays.len();
    }

    eprintln!("Total: {total_moves} moves, {total_elapsed:?}");

    if let Some(baseline) = &baseline {
        if output == *baseline {
            eprintln!("PASS: output matches baseline");
        } else {
            print!("{output}");
            eprintln!("FAIL: output differs from baseline");
            std::process::exit(1);
        }
    } else {
        print!("{output}");
    }
    Ok(())
}
