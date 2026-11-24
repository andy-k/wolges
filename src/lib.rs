// Copyright (C) 2020-2026 Andy Kurnia.

// the stable items, what a crate builds on to load a lexicon and generate a
// move: the version const; kwg's Kwg, its node readers, from_bytes_alloc, and
// kbwg_from_bytes_alloc; klv's Klv, from_bytes_alloc, and
// kbwg_from_bytes_alloc; and the alphabet, board_layout, game_config,
// game_state, bag, display, movegen, play_scorer, move_filter, move_picker,
// equity, win_pct, and error modules. Nothing else carries a promise. A stable
// name keeps its meaning; a change is additive (a new item, field, or json
// key); a new meaning takes a new name, and an old one goes only in a commit
// that says it removes it. The build identity is the version const, never a
// field in any file.

#[macro_use]
pub mod error;

pub mod alphabet;
pub mod alphagram;
pub mod anagrams;
pub mod bag;
pub mod bites;
pub mod bites_str;
pub mod board_layout;
pub mod build;
pub mod census;
pub mod display;
pub mod endgame;
pub mod equity;
pub mod fash;
pub mod game_config;
pub mod game_state;
pub mod game_timers;
pub mod glance;
pub mod infer;
pub mod kibitzer;
pub mod klv;
pub mod kwg;
pub mod lexport;
pub mod matrix;
pub mod move_filter;
pub mod move_picker;
pub mod movegen;
pub mod notation;
pub mod parallelism;
pub mod play_scorer;
pub mod prob;
pub mod racks;
pub mod simmer;
pub mod stats;
pub mod text;
pub mod win_pct;
pub mod woadwage;
pub mod zone;

// the commit this was built from and the target, as build.rs stamps them
pub const VERSION: &str = env!("WOLGES_VERSION");
