// Copyright (C) 2020-2026 Andy Kurnia.

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::Mutex;
use wolges::bites;

static SIZES: Mutex<Option<std::collections::HashMap<usize, (usize, usize)>>> = Mutex::new(None);
static MISMATCHES: Mutex<Vec<(usize, usize, usize, usize)>> = Mutex::new(Vec::new());

struct Checking;

thread_local! {
    static BUSY: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

fn with_map<R>(
    f: impl FnOnce(&mut std::collections::HashMap<usize, (usize, usize)>) -> R,
) -> Option<R> {
    if BUSY.with(|b| b.get()) {
        return None;
    }
    BUSY.with(|b| b.set(true));
    let mut guard = match SIZES.lock() {
        Ok(g) => g,
        Err(_) => {
            BUSY.with(|b| b.set(false));
            return None;
        }
    };
    let map = guard.get_or_insert_with(Default::default);
    let r = f(map);
    drop(guard);
    BUSY.with(|b| b.set(false));
    Some(r)
}

unsafe impl GlobalAlloc for Checking {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let p = unsafe { System.alloc(layout) };
        if !p.is_null() {
            with_map(|m| m.insert(p as usize, (layout.size(), layout.align())));
        }
        p
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        if let Some(Some((size, align))) = with_map(|m| m.remove(&(ptr as usize)))
            && (size != layout.size() || align != layout.align())
            && let Ok(mut v) = MISMATCHES.lock()
        {
            v.push((size, align, layout.size(), layout.align()));
        }
        unsafe { System.dealloc(ptr, layout) }
    }
}

#[global_allocator]
static ALLOC: Checking = Checking;

#[test]
fn a_heap_bites_is_freed_with_the_size_it_asked_for() {
    for len in [16usize, 17, 23, 30, 31, 64, 255, 4096] {
        let src = vec![7u8; len];
        let b: bites::Bites = src[..].into();
        assert_eq!(b.len(), len);
        assert!(b.iter().all(|&x| x == 7));
        drop(b);
    }

    let src = [3u8; 40];
    let a: bites::Bites = src[..].into();
    let b = a.clone();
    let mut c: bites::Bites = vec![9u8; 40][..].into();
    c.clone_from(&a);
    let mut d: bites::Bites = vec![9u8; 17][..].into();
    d.clone_from(&a);
    assert_eq!(&b[..], &a[..]);
    assert_eq!(&c[..], &a[..]);
    assert_eq!(&d[..], &a[..]);
    drop((a, b, c, d));

    let seen = MISMATCHES.lock().unwrap();
    assert!(
        seen.is_empty(),
        "freed with a layout it was not allocated with: {seen:?}"
    );
}
