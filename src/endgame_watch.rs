// Copyright (C) 2020-2026 Andy Kurnia.

// what a program that solves endgames uses to watch them: a count of the bytes
// it holds, for --max-memory, and a listener that prints a solve's progress

use std::alloc::GlobalAlloc;
use wolges::{endgame, error};

// count every block of at least a page that the program holds, as it is
// allocated and freed, for --max-memory. Smaller blocks come and go fastest (a
// sim's threads allocate them all the time) and add up to little beside the
// tables a limit is for, and counting each of them in one shared count would
// make every thread wait on the others
struct Counting;

static HELD: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

// the bytes of a page
const COUNTED: usize = 4096;

// add a block's bytes to the count when it is allocated, or take them off when
// it is freed, if it is big enough to count
#[inline(always)]
fn count(size: usize, taken: bool) {
    if size >= COUNTED {
        if taken {
            HELD.fetch_add(size, std::sync::atomic::Ordering::Relaxed);
        } else {
            HELD.fetch_sub(size, std::sync::atomic::Ordering::Relaxed);
        }
    }
}

unsafe impl GlobalAlloc for Counting {
    #[inline]
    unsafe fn alloc(&self, layout: std::alloc::Layout) -> *mut u8 {
        let p = unsafe { std::alloc::System.alloc(layout) };
        if !p.is_null() {
            count(layout.size(), true);
        }
        p
    }

    #[inline]
    unsafe fn alloc_zeroed(&self, layout: std::alloc::Layout) -> *mut u8 {
        let p = unsafe { std::alloc::System.alloc_zeroed(layout) };
        if !p.is_null() {
            count(layout.size(), true);
        }
        p
    }

    #[inline]
    unsafe fn dealloc(&self, p: *mut u8, layout: std::alloc::Layout) {
        unsafe { std::alloc::System.dealloc(p, layout) };
        count(layout.size(), false);
    }

    // both buffers may be held while one is copied to the other
    #[inline]
    unsafe fn realloc(&self, p: *mut u8, layout: std::alloc::Layout, new_size: usize) -> *mut u8 {
        count(new_size, true);
        let q = unsafe { std::alloc::System.realloc(p, layout, new_size) };
        if q.is_null() {
            count(new_size, false);
        } else {
            count(layout.size(), false);
        }
        q
    }
}

#[global_allocator]
static ALLOCATOR: Counting = Counting;

#[inline(always)]
fn held_bytes() -> usize {
    HELD.load(std::sync::atomic::Ordering::Relaxed)
}

// the --max-memory default, in gigabytes
pub const DEFAULT_MAX_MEMORY_GB: f64 = 60.0;

// --max-memory's gigabytes as a solve's memory limit (0 for no limit)
pub fn memory_limit(gb: f64) -> error::Returns<Option<endgame::MemoryLimit>> {
    if gb == 0.0 {
        Ok(None)
    } else if gb.is_finite() && gb > 0.0 {
        Ok(Some(endgame::MemoryLimit {
            bytes: (gb * 1e9).round() as usize,
            held: held_bytes,
        }))
    } else {
        wolges::return_error!("--max-memory is gigabytes, 0 or more".to_string());
    }
}

// how many nodes a printing listener hears between looks at its clock
const CLOCK_CADENCE: u64 = 1024;

// a listener that prints a solve's progress with print: each finished depth,
// and the counts every ten seconds, with the time since it began on the clock
// it keeps, which it reads once every CLOCK_CADENCE nodes
#[inline(always)]
pub fn ticking(
    mut print: impl FnMut(&endgame::Progress, std::time::Duration) -> error::Returns<()> + 'static,
) -> impl FnMut(&endgame::Progress) -> error::Returns<()> + 'static {
    let t0 = std::time::Instant::now();
    let mut nodes = 0u64;
    let mut tick_periods = 0u64;
    move |progress: &endgame::Progress| {
        if progress.value.is_none() {
            nodes += 1;
            if !nodes.is_multiple_of(CLOCK_CADENCE) {
                return Ok(());
            }
            let periods = t0.elapsed().as_millis() as u64 / 10000;
            if periods == tick_periods {
                return Ok(());
            }
            tick_periods = periods;
        }
        print(progress, t0.elapsed())
    }
}
