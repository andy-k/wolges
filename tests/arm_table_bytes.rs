// Copyright (C) 2020-2026 Andy Kurnia.

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicUsize, Ordering};
use wolges::{anagrams, bites, build, game_config, kwg};

struct Counting;

static NOW: AtomicUsize = AtomicUsize::new(0);
static PEAK: AtomicUsize = AtomicUsize::new(0);

#[inline(always)]
fn add(n: usize) {
    let now = NOW.fetch_add(n, Ordering::Relaxed) + n;
    PEAK.fetch_max(now, Ordering::Relaxed);
}

unsafe impl GlobalAlloc for Counting {
    #[inline]
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let p = unsafe { System.alloc(layout) };
        if !p.is_null() {
            add(layout.size());
        }
        p
    }

    #[inline]
    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        let p = unsafe { System.alloc_zeroed(layout) };
        if !p.is_null() {
            add(layout.size());
        }
        p
    }

    #[inline]
    unsafe fn dealloc(&self, p: *mut u8, layout: Layout) {
        unsafe { System.dealloc(p, layout) };
        NOW.fetch_sub(layout.size(), Ordering::Relaxed);
    }

    // both buffers may be held while one is copied to the other
    #[inline]
    unsafe fn realloc(&self, p: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        add(new_size);
        let q = unsafe { System.realloc(p, layout, new_size) };
        NOW.fetch_sub(
            if q.is_null() { new_size } else { layout.size() },
            Ordering::Relaxed,
        );
        q
    }
}

#[global_allocator]
static ALLOCATOR: Counting = Counting;

// what f returns, and the most it held at once beyond what was held before it
#[inline]
fn peak_of<T, F: FnOnce() -> T>(f: F) -> (T, usize) {
    let base = NOW.load(Ordering::Relaxed);
    PEAK.store(base, Ordering::Relaxed);
    let got = f();
    (got, PEAK.load(Ordering::Relaxed) - base)
}

// every word of two to five tiles over the first six letters: many keys, and
// the blank pairs the most the build holds
#[inline]
fn short_words() -> Vec<Vec<u8>> {
    let mut words = Vec::new();
    for len in 2..=5u32 {
        for n in 0..6usize.pow(len) {
            words.push(
                (0..len)
                    .map(|i| (n / 6usize.pow(len - 1 - i) % 6) as u8 + 1)
                    .collect::<Vec<u8>>(),
            );
        }
    }
    words.sort_unstable();
    words
}

// the first arrangements, in order, of thirty tiles -- every letter, and four
// of them twice: one key, so the words' letters are the most the build holds
#[inline]
fn long_words(n: usize) -> Vec<Vec<u8>> {
    let mut w = (1..=26).chain([1, 5, 9, 15]).collect::<Vec<u8>>();
    w.sort_unstable();
    let mut words = vec![w.clone()];
    while words.len() < n {
        let Some(i) = w.windows(2).rposition(|p| p[0] < p[1]) else {
            break;
        };
        let j = w.iter().rposition(|&t| t > w[i]).unwrap();
        w.swap(i, j);
        w[i + 1..].reverse();
        words.push(w.clone());
    }
    words
}

#[inline]
fn graph_of(words: &[Vec<u8>]) -> kwg::Kwg<kwg::Node22> {
    let words = words
        .iter()
        .map(|w| w[..].into())
        .collect::<Vec<bites::Bites>>();
    let bytes = build::build(
        build::BuildContent::Gaddawg,
        build::BuildLayout::Wolges,
        build::BuildOrder::Sorted,
        &words,
    )
    .unwrap();
    kwg::Kwg::<kwg::Node22>::from_bytes_alloc(&bytes).unwrap()
}

#[test]
fn a_table_build_stays_within_its_byte_cap() {
    let gc = game_config::make_english_game_config();
    for (shape, words) in [("short", short_words()), ("long", long_words(2000))] {
        let g = graph_of(&words);
        for lean in [false, true] {
            let within = |cap| anagrams::ArmTables::build_within(&g, &gc, lean, cap).unwrap();
            let (_, peak) = peak_of(|| anagrams::ArmTables::build(&g, &gc, lean).unwrap());
            // the least cap the tables are built under
            let (mut refused, mut fits) = (0usize, 2 * peak);
            assert!(within(fits).is_some(), "{shape} lean {lean}");
            while fits - refused > 1 {
                let cap = refused + (fits - refused) / 2;
                if within(cap).is_some() {
                    fits = cap;
                } else {
                    refused = cap;
                }
            }
            let (tables, held) = peak_of(|| within(fits));
            assert!(tables.is_some(), "{shape} lean {lean}");
            assert!(
                held <= fits,
                "{shape} lean {lean}: {held} past the cap {fits}"
            );
            // the cap counts what the build holds, not a guess well above it
            assert!(
                fits <= peak + peak / 20,
                "{shape} lean {lean}: needs {fits}, holds {peak}"
            );
            let (tables, held) = peak_of(|| within(refused));
            assert!(tables.is_none(), "{shape} lean {lean}");
            assert!(
                held <= refused,
                "{shape} lean {lean}: {held} past the cap {refused}"
            );
        }
    }
}
