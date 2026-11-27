// Copyright (C) 2020-2026 Andy Kurnia.

#![cfg(test)]

use wolges::{alphabet, anagrams, bites, build, endgame, equity, game_config, klv, kwg, movegen};

thread_local! {
    static COUNT: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    static REFUSE_AT: std::cell::Cell<usize> = const { std::cell::Cell::new(usize::MAX) };
    static REFUSED: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    static LARGEST: std::cell::Cell<(usize, usize)> = const { std::cell::Cell::new((0, 0)) };
}

// counts a request for size bytes, refusing it where refusing_at says
fn refused(size: usize) -> bool {
    let Ok(n) = COUNT.try_with(|count| count.replace(count.get() + 1)) else {
        return false;
    };
    let _ = LARGEST.try_with(|largest| {
        if size > largest.get().0 {
            largest.set((size, n));
        }
    });
    let refused = REFUSE_AT.try_with(|at| at.get() == n).unwrap_or(false);
    if refused {
        REFUSED.with(|refused| refused.set(refused.get() + 1));
    }
    refused
}

// f's answer, and how many requests into it its largest came
fn largest_in<T>(f: impl FnOnce() -> T) -> (T, usize) {
    let start = COUNT.with(|count| count.get());
    LARGEST.with(|largest| largest.set((0, 0)));
    let answer = f();
    (answer, LARGEST.with(|largest| largest.get().1) - start)
}

// f's answer, its request this many in refused, and how many it had refused
fn refusing_at<T>(nth: usize, f: impl FnOnce() -> T) -> (T, usize) {
    REFUSED.with(|refused| refused.set(0));
    REFUSE_AT.with(|at| at.set(COUNT.with(|count| count.get()) + nth));
    let answer = f();
    REFUSE_AT.with(|at| at.set(usize::MAX));
    (answer, REFUSED.with(|refused| refused.get()))
}

// on this thread, refuses the one request refusing_at names
struct Refusing;

// safety: all allocations use System. The refusal and accounting only track
// sizes. A refused realloc leaves the original allocation in place.
unsafe impl std::alloc::GlobalAlloc for Refusing {
    unsafe fn alloc(&self, layout: std::alloc::Layout) -> *mut u8 {
        if refused(layout.size()) {
            return std::ptr::null_mut();
        }
        // safety: the caller supplies a valid, nonzero allocation layout.
        unsafe { std::alloc::System.alloc(layout) }
    }

    unsafe fn alloc_zeroed(&self, layout: std::alloc::Layout) -> *mut u8 {
        if refused(layout.size()) {
            return std::ptr::null_mut();
        }
        // safety: the caller supplies a valid, nonzero allocation layout.
        unsafe { std::alloc::System.alloc_zeroed(layout) }
    }

    unsafe fn dealloc(&self, p: *mut u8, layout: std::alloc::Layout) {
        // safety: the caller passes a live allocation from System and its layout.
        unsafe { std::alloc::System.dealloc(p, layout) }
    }

    unsafe fn realloc(&self, p: *mut u8, layout: std::alloc::Layout, new_size: usize) -> *mut u8 {
        if refused(new_size) {
            return std::ptr::null_mut();
        }
        // safety: forward the live System allocation, layout, and valid size.
        unsafe { std::alloc::System.realloc(p, layout, new_size) }
    }
}

#[global_allocator]
static REFUSING: Refusing = Refusing;

fn test_kwg(
    gc: &game_config::GameConfig,
    content: build::BuildContent,
    test_words: &[&str],
) -> kwg::Kwg<kwg::Node24> {
    let reader = alphabet::AlphabetReader::new_for_words(gc.alphabet());
    let mut word_buf = Vec::new();
    let mut words = Vec::<bites::Bites>::with_capacity(test_words.len());
    for w in test_words {
        reader.set_word(w, &mut word_buf).unwrap();
        if matches!(content, build::BuildContent::DawgOnly) {
            word_buf.sort_unstable();
        }
        words.push(word_buf[..].into());
    }
    words.sort_unstable();
    words.dedup();
    kwg::kbwg_from_bytes_alloc(
        build::build(
            content,
            build::BuildLayout::Wolges,
            build::BuildOrder::Sorted,
            &words,
        )
        .unwrap()
        .to_vec(),
        false,
    )
    .unwrap()
}

fn parse_test_rack(alphabet: &alphabet::Alphabet, rack_str: &str) -> Vec<u8> {
    let reader = alphabet::AlphabetReader::new_for_racks(alphabet);
    let sb = rack_str.as_bytes();
    let mut rack = Vec::new();
    let mut ix = 0;
    while ix < sb.len() {
        let (tile, next_ix) = reader.next_tile(sb, ix).unwrap();
        rack.push(tile);
        ix = next_ix;
    }
    rack
}

const ENDGAME_WORDS: [&str; 19] = [
    "AA", "AB", "AH", "AT", "BA", "HA", "TA", "AAH", "ABA", "BAA", "BAT", "TAB", "TAT", "HAT",
    "THAT", "AAHS", "BATH", "HAH", "AAL",
];

// a board with BAT across its center
fn endgame_board(gc: &game_config::GameConfig) -> Vec<u8> {
    let dim = gc.board_layout().dim();
    let mut board = vec![0u8; dim.rows as usize * dim.cols as usize];
    let center = 7 * dim.cols as usize + 6;
    board[center..center + 3].copy_from_slice(&parse_test_rack(gc.alphabet(), "BAT"));
    board
}

const NO_MEMORY_LIMIT: endgame::Limits = endgame::Limits {
    memory: Some(endgame::MemoryLimit {
        bytes: usize::MAX,
        held: || 0,
        reserved: None,
    }),
    ..endgame::Limits::NONE
};

#[test]
fn an_endgame_solve_refused_its_largest_growth_answers_as_before() {
    let gc = game_config::make_english_game_config();
    let kwg = test_kwg(&gc, build::BuildContent::Gaddawg, &ENDGAME_WORDS);
    let arm_tables = anagrams::ArmTables::build(&kwg, &gc, false).unwrap();
    let board = endgame_board(&gc);
    let racks = ["AAHST", "ABHLT"].map(|rack| parse_test_rack(gc.alphabet(), rack));
    let solve = || {
        let mut egs =
            endgame::EndgameSolver::<kwg::Node24, kwg::Node24>::new(&gc, &kwg, &arm_tables)
                .unwrap();
        egs.init(&board, [&racks[0], &racks[1]]);
        let solved = egs.solve_within(0, &NO_MEMORY_LIMIT).unwrap();
        (solved.value, solved.exact, solved.out_of_memory)
    };
    let (want, largest) = largest_in(solve);
    assert!(want.1);
    let (answer, refused) = refusing_at(largest, solve);
    assert_eq!(refused, 1);
    assert_eq!(answer, want);
}

#[test]
fn a_one_in_bag_solve_refused_its_largest_growth_answers_as_before() {
    let gc = game_config::make_english_game_config();
    let kwg = test_kwg(&gc, build::BuildContent::Gaddawg, &ENDGAME_WORDS);
    let arm_tables = anagrams::ArmTables::build(&kwg, &gc, false).unwrap();
    let board = endgame_board(&gc);
    let rack = parse_test_rack(gc.alphabet(), "AAH");
    let mut unseen = vec![0u8; gc.alphabet().len() as usize];
    for tile in parse_test_rack(gc.alphabet(), "AABHLT") {
        unseen[tile as usize] += 1;
    }
    let solve = || {
        let mut egs =
            endgame::EndgameSolver::<kwg::Node24, kwg::Node24>::new(&gc, &kwg, &arm_tables)
                .unwrap();
        let score_diff = equity::Equity::new(0);
        let result = egs
            .solve_peg_one_in_bag(0, &board, &rack, &unseen, score_diff, &NO_MEMORY_LIMIT)
            .unwrap()
            .ok()
            .unwrap();
        let answer = (result.win_pct, result.expected_margin);
        (answer, result.exact, result.out_of_memory)
    };
    let (want, largest) = largest_in(solve);
    assert!(want.1);
    let (answer, refused) = refusing_at(largest, solve);
    assert_eq!(refused, 1);
    assert_eq!(answer, want);
}

#[test]
fn a_jumbled_generation_refused_its_largest_growth_says_so() {
    let gc = game_config::make_jumbled_english_game_config();
    let letters = b"ACENRST";
    let mut words = vec![String::new()];
    for _ in 0..gc.rack_size() {
        let longer = (words.iter())
            .flat_map(|word| {
                let from = (letters.iter())
                    .position(|&l| word.as_bytes().last().is_some_and(|&w| w == l))
                    .unwrap_or(0);
                letters[from..]
                    .iter()
                    .map(move |&l| format!("{word}{}", l as char))
            })
            .collect::<Vec<_>>();
        words.extend(longer);
        words.sort_unstable();
        words.dedup();
    }
    let words = (words.iter())
        .filter(|word| word.len() >= 2)
        .map(String::as_str)
        .collect::<Vec<_>>();
    let kwg = test_kwg(&gc, build::BuildContent::DawgOnly, &words);
    let klv = klv::kbwg_from_bytes_alloc(klv::EMPTY_KLV_BYTES.to_vec()).unwrap();
    let dim = gc.board_layout().dim();
    let board_tiles = vec![0u8; dim.rows as usize * dim.cols as usize];
    let board_snapshot = movegen::BoardSnapshot {
        alphagrams: None,
        board_tiles: &board_tiles,
        game_config: &gc,
        kwg: &kwg,
        anagrams: None,
        rack_lengths: None,
        klv: &klv,
    };
    let pool = parse_test_rack(gc.alphabet(), "AACCEENNRRSSTT??");
    let plays = || {
        let mut generator = movegen::KurniaMoveGenerator::new(&gc);
        let mut count = 0usize;
        generator.pool_plays(&board_snapshot, &pool, |_, _, _, _, _| count += 1);
        (count, generator.out_of_memory())
    };
    let ((all, refused), largest) = largest_in(plays);
    assert!(all > 0);
    assert!(!refused);
    let ((some, refused), refusals) = refusing_at(largest, plays);
    assert_eq!(refusals, 1);
    assert!(refused);
    assert!(some < all, "{some} {all}");
    assert_eq!(plays(), (all, false));
}
