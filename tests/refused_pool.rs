// Copyright (C) 2020-2026 Andy Kurnia.

#![cfg(test)]

use wolges::{alphabet, bites, build, game_config, klv, kwg, movegen};

thread_local! {
    static HELD: std::cell::Cell<isize> = const { std::cell::Cell::new(0) };
    static ROOM: std::cell::Cell<Option<isize>> = const { std::cell::Cell::new(None) };
}

fn refused(size: usize) -> bool {
    let held = HELD.try_with(std::cell::Cell::get).unwrap_or(0);
    let room = ROOM.try_with(std::cell::Cell::get).ok().flatten();
    room.is_some_and(|room| held.saturating_add_unsigned(size) > room)
}

fn held(delta: isize) {
    let _ = HELD.try_with(|held| held.set(held.get().wrapping_add(delta)));
}

// with no room past what this thread holds now
fn refusing<T>(f: impl FnOnce() -> T) -> T {
    let was = ROOM.with(|room| room.replace(Some(HELD.with(std::cell::Cell::get))));
    let answer = f();
    ROOM.with(|room| room.set(was));
    answer
}

// on this thread, counts what it holds, and refuses a request past its room
struct Counting;

// safety: all allocations use System. The refusal and accounting only track
// sizes. A refused realloc leaves the original allocation in place.
unsafe impl std::alloc::GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: std::alloc::Layout) -> *mut u8 {
        if refused(layout.size()) {
            return std::ptr::null_mut();
        }
        // safety: the caller supplies a valid, nonzero allocation layout.
        let p = unsafe { std::alloc::System.alloc(layout) };
        if !p.is_null() {
            held(layout.size() as isize);
        }
        p
    }

    unsafe fn alloc_zeroed(&self, layout: std::alloc::Layout) -> *mut u8 {
        if refused(layout.size()) {
            return std::ptr::null_mut();
        }
        // safety: the caller supplies a valid, nonzero allocation layout.
        let p = unsafe { std::alloc::System.alloc_zeroed(layout) };
        if !p.is_null() {
            held(layout.size() as isize);
        }
        p
    }

    unsafe fn dealloc(&self, p: *mut u8, layout: std::alloc::Layout) {
        // safety: the caller passes a live allocation from System and its layout.
        unsafe { std::alloc::System.dealloc(p, layout) };
        held(-(layout.size() as isize));
    }

    unsafe fn realloc(&self, p: *mut u8, layout: std::alloc::Layout, new_size: usize) -> *mut u8 {
        if new_size > layout.size() && refused(new_size - layout.size()) {
            return std::ptr::null_mut();
        }
        // safety: forward the live System allocation, layout, and valid size.
        let q = unsafe { std::alloc::System.realloc(p, layout, new_size) };
        if !q.is_null() {
            held(new_size as isize - layout.size() as isize);
        }
        q
    }
}

#[global_allocator]
static COUNTING: Counting = Counting;

fn test_kad(gc: &game_config::GameConfig, test_words: &[&str]) -> kwg::Kwg<kwg::Node24> {
    let reader = alphabet::AlphabetReader::new_for_words(gc.alphabet());
    let mut word_buf = Vec::new();
    let mut words = Vec::<bites::Bites>::with_capacity(test_words.len());
    for w in test_words {
        reader.set_word(w, &mut word_buf).unwrap();
        word_buf.sort_unstable();
        words.push(word_buf[..].into());
    }
    words.sort_unstable();
    words.dedup();
    kwg::kbwg_from_bytes_alloc(
        build::build(
            build::BuildContent::DawgOnly,
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

fn test_kwg(test_words: &[&str]) -> kwg::Kwg<kwg::Node24> {
    let gc = game_config::make_english_game_config();
    let reader = alphabet::AlphabetReader::new_for_words(gc.alphabet());
    let mut word_buf = Vec::new();
    let mut words = Vec::<bites::Bites>::with_capacity(test_words.len());
    for w in test_words {
        reader.set_word(w, &mut word_buf).unwrap();
        words.push(word_buf[..].into());
    }
    words.sort_unstable();
    words.dedup();
    kwg::kbwg_from_bytes_alloc(
        build::build(
            build::BuildContent::Gaddawg,
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

fn tiles(alphabet: &alphabet::Alphabet, rack_str: &str) -> Vec<u8> {
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

const WORDS: &[&str] = &["AT", "CAT", "CATS", "SCAT", "ACT", "ACTS", "TA", "TAS"];

fn a_pool_walk_with_no_room_answers_out_of_memory(
    gc: &game_config::GameConfig,
    words: &kwg::Kwg<kwg::Node24>,
) {
    let klv = klv::kbwg_from_bytes_alloc(klv::EMPTY_KLV_BYTES.to_vec()).unwrap();
    let dim = gc.board_layout().dim();
    let board_tiles = vec![0u8; dim.rows as usize * dim.cols as usize];
    let board_snapshot = movegen::BoardSnapshot {
        board_tiles: &board_tiles,
        game_config: gc,
        kwg: words,
        anagrams: None,
        rack_lengths: None,
        klv: &klv,
    };
    let pool = tiles(gc.alphabet(), "AACSTT?");
    let mut generator = movegen::KurniaMoveGenerator::new(gc);
    let mut count = 0usize;
    let mut counted = |_, _, _, _: &[u8], _| count += 1;
    let all = !0u128;
    generator.pool_plays_window(
        &board_snapshot,
        &pool,
        (all, all),
        0..u64::MAX,
        None,
        &mut counted,
    );
    assert!(count > 0);
    assert!(!generator.out_of_memory());
    count = 0;
    let mut fresh = movegen::KurniaMoveGenerator::new(gc);
    let mut counted = |_, _, _, _: &[u8], _| count += 1;
    refusing(|| {
        fresh.pool_plays_window(
            &board_snapshot,
            &pool,
            (all, all),
            0..u64::MAX,
            None,
            &mut counted,
        )
    });
    assert!(fresh.out_of_memory());
    assert_eq!(count, 0);
}

#[test]
fn a_jumbled_pool_walk_with_no_room_answers_out_of_memory() {
    let gc = game_config::make_jumbled_english_game_config();
    a_pool_walk_with_no_room_answers_out_of_memory(&gc, &test_kad(&gc, WORDS));
}

#[test]
fn a_classic_pool_walk_with_no_room_answers_out_of_memory() {
    let gc = game_config::make_english_game_config();
    a_pool_walk_with_no_room_answers_out_of_memory(&gc, &test_kwg(WORDS));
}
