// Copyright (C) 2020-2026 Andy Kurnia.

use rand::SeedableRng;
use std::collections::BTreeMap;
use wolges::kwg::Node;
use wolges::{
    bites, board_layout, build, equity, error, fash, game_config, game_state, klv, kwg, movegen,
};

type PlayKey = (u8, i8, i8, Vec<u8>);

fn lexicon_words(kwg: &kwg::Kwg<kwg::Node24>) -> error::Returns<Vec<Vec<u8>>> {
    fn walk(
        kwg: &kwg::Kwg<kwg::Node24>,
        mut p: i32,
        word: &mut Vec<u8>,
        words: &mut Vec<Vec<u8>>,
    ) -> error::Returns<()> {
        if p <= 0 {
            return Ok(());
        }
        loop {
            let node = *kwg.node(p)?;
            word.push(node.tile());
            if node.accepts() {
                words.push(word.clone());
            }
            walk(kwg, node.arc_index(), word, words)?;
            word.pop();
            if node.is_end() {
                return Ok(());
            }
            p += 1;
        }
    }
    let mut words = Vec::new();
    walk(kwg, kwg.node(0)?.arc_index(), &mut Vec::new(), &mut words)?;
    Ok(words)
}

fn kad_for(words: &[Vec<u8>]) -> error::Returns<kwg::Kwg<kwg::Node24>> {
    let mut alphagrams = words
        .iter()
        .map(|word| {
            let mut word = word.clone();
            word.sort_unstable();
            word
        })
        .collect::<Vec<_>>();
    alphagrams.sort_unstable();
    alphagrams.dedup();
    let alphagrams = alphagrams
        .iter()
        .map(|word| bites::Bites::from(&word[..]))
        .collect::<Vec<_>>();
    let bytes = build::build(
        build::BuildContent::DawgOnly,
        build::BuildLayout::Wolges,
        build::BuildOrder::Sorted,
        &alphagrams,
    )?;
    kwg::kbwg_from_bytes_alloc(bytes.to_vec(), false)
}

struct JoinedLexicon {
    jumbled: bool,
    joined: bool,
    words: fash::MyHashSet<Vec<u8>>,
    parts: fash::MyHashSet<u64>,
}

impl JoinedLexicon {
    fn new(words: &[Vec<u8>], jumbled: bool, joined: bool) -> Self {
        fn sub_multisets(runs: &[(u8, usize)], acc: &mut Vec<u8>, out: &mut fash::MyHashSet<u64>) {
            match runs.split_first() {
                None => {
                    out.insert(JoinedLexicon::hash(acc));
                }
                Some((&(tile, count), rest)) => {
                    let len = acc.len();
                    for k in 0..=count {
                        if k > 0 {
                            acc.push(tile);
                        }
                        sub_multisets(rest, acc, out);
                    }
                    acc.truncate(len);
                }
            }
        }
        let mut lexicon = Self {
            jumbled,
            joined,
            words: Default::default(),
            parts: Default::default(),
        };
        for word in words {
            let word = lexicon.key(word);
            if jumbled {
                let mut runs = Vec::<(u8, usize)>::new();
                for &tile in &word {
                    match runs.last_mut() {
                        Some((last, count)) if *last == tile => *count += 1,
                        _ => runs.push((tile, 1)),
                    }
                }
                sub_multisets(&runs, &mut Vec::new(), &mut lexicon.parts);
            } else {
                for len in 0..=word.len() {
                    lexicon.parts.insert(Self::hash(&word[..len]));
                }
            }
            lexicon.words.insert(word);
        }
        lexicon
    }

    fn hash(letters: &[u8]) -> u64 {
        use std::hash::BuildHasher;
        fash::MyHasherDefault::default().hash_one(letters)
    }

    fn key(&self, letters: &[u8]) -> Vec<u8> {
        let mut key = letters.to_vec();
        if self.jumbled {
            key.sort_unstable();
        }
        key
    }

    fn whole(&self, letters: &[u8]) -> bool {
        self.words.contains(&self.key(letters))
    }

    fn part(&self, letters: &[u8]) -> bool {
        self.parts.contains(&Self::hash(&self.key(letters)))
    }

    fn takes(&self, letters: &[u8]) -> bool {
        self.whole(letters)
            || self.joined
                && (1..letters.len())
                    .any(|i| self.whole(&letters[..i]) && self.whole(&letters[i..]))
    }

    fn may_start(&self, letters: &[u8]) -> bool {
        self.part(letters)
            || self.joined
                && (1..letters.len()).any(|i| self.whole(&letters[..i]) && self.part(&letters[i..]))
    }
}

struct JoinedFill<'a> {
    lexicon: &'a JoinedLexicon,
    alphabet_len: u8,
    line: Vec<u8>,
    allowed: Vec<u64>,
    crossed: bool,
    counts: Vec<u8>,
    letters: Vec<u8>,
    word: Vec<u8>,
    found: Vec<Vec<u8>>,
}

impl JoinedFill<'_> {
    fn fill(&mut self, k: usize) {
        if k == self.line.len() {
            if if self.letters.len() >= 2 {
                self.lexicon.takes(&self.letters)
            } else {
                self.crossed
            } {
                self.found.push(self.word.clone());
            }
            return;
        }
        let viable = |fill: &Self| fill.line.len() < 2 || fill.lexicon.may_start(&fill.letters);
        if self.line[k] != 0 {
            self.letters.push(self.line[k] & 0x7f);
            self.word.push(0);
            if viable(self) {
                self.fill(k + 1);
            }
            self.word.pop();
            self.letters.pop();
            return;
        }
        for tile in 0..self.alphabet_len {
            if self.counts[tile as usize] == 0 {
                continue;
            }
            let letters = if tile == 0 {
                1..self.alphabet_len
            } else {
                tile..tile + 1
            };
            for letter in letters {
                if self.allowed[k] & (1 << letter) == 0 {
                    continue;
                }
                self.counts[tile as usize] -= 1;
                self.letters.push(letter);
                self.word
                    .push(if tile == 0 { letter | 0x80 } else { letter });
                if viable(self) {
                    self.fill(k + 1);
                }
                self.word.pop();
                self.letters.pop();
                self.counts[tile as usize] += 1;
            }
        }
    }
}

fn play_key(down: bool, lane: i8, idx: i8, word: &[u8]) -> PlayKey {
    let placed = (0i8..)
        .zip(word.iter())
        .filter(|&(_, &tile)| tile != 0)
        .collect::<Vec<_>>();
    if placed.len() == 1 {
        let along = idx + placed[0].0;
        let (row, col) = if down { (along, lane) } else { (lane, along) };
        (2, row, col, vec![*placed[0].1])
    } else {
        (down as u8, lane, idx, word.to_vec())
    }
}

fn brute_force(
    gc: &game_config::GameConfig,
    board_tiles: &[u8],
    rack: &[u8],
    lexicon: &JoinedLexicon,
) -> BTreeMap<PlayKey, i32> {
    let dim = gc.board_layout().dim();
    let alphabet_len = gc.alphabet().len();
    let tile = |row: i8, col: i8| {
        if (0..dim.rows).contains(&row) && (0..dim.cols).contains(&col) {
            board_tiles[dim.at_row_col(row, col).unwrap()]
        } else {
            0
        }
    };
    let cross = |row: i8, col: i8, down: bool| {
        let (dr, dc) = if down { (1, 0) } else { (0, 1) };
        let mut before = Vec::new();
        let (mut r, mut c) = (row - dr, col - dc);
        while tile(r, c) != 0 {
            before.push(tile(r, c) & 0x7f);
            r -= dr;
            c -= dc;
        }
        before.reverse();
        let mut after = Vec::new();
        let (mut r, mut c) = (row + dr, col + dc);
        while tile(r, c) != 0 {
            after.push(tile(r, c) & 0x7f);
            r += dr;
            c += dc;
        }
        if before.is_empty() && after.is_empty() {
            (false, !0u64)
        } else {
            let mut bits = 0u64;
            for letter in 1..alphabet_len {
                if lexicon.takes(&[&before[..], &[letter], &after[..]].concat()) {
                    bits |= 1 << letter;
                }
            }
            (true, bits)
        }
    };
    let empty_board = board_tiles.iter().all(|&t| t == 0);
    let star = (gc.board_layout().star_row(), gc.board_layout().star_col());
    let mut counts = vec![0u8; alphabet_len as usize];
    for &t in rack {
        counts[t as usize] += 1;
    }
    let mut keys = BTreeMap::new();
    for down in [false, true] {
        if down && empty_board && gc.board_layout().is_symmetric() {
            continue;
        }
        let (lanes, len) = if down {
            (dim.cols, dim.rows)
        } else {
            (dim.rows, dim.cols)
        };
        for lane in 0..lanes {
            let square = |idx: i8| if down { (idx, lane) } else { (lane, idx) };
            let tile_at = |idx: i8| {
                let (row, col) = square(idx);
                tile(row, col)
            };
            let crosses = (0..len)
                .map(|idx| {
                    let (row, col) = square(idx);
                    cross(row, col, !down)
                })
                .collect::<Vec<_>>();
            for start in 0..len {
                if tile_at(start - 1) != 0 {
                    continue;
                }
                for end in start + 1..=len {
                    if tile_at(end) != 0 {
                        continue;
                    }
                    let empties = (start..end)
                        .filter(|&idx| tile_at(idx) == 0)
                        .collect::<Vec<_>>();
                    if empties.is_empty() || empties.len() > rack.len() {
                        continue;
                    }
                    let connected = if empty_board {
                        empties.iter().any(|&idx| square(idx) == star)
                    } else {
                        empties.len() < (end - start) as usize
                            || empties.iter().any(|&idx| crosses[idx as usize].0)
                    };
                    if !connected {
                        continue;
                    }
                    let mut fill = JoinedFill {
                        lexicon,
                        alphabet_len,
                        line: (start..end).map(tile_at).collect(),
                        allowed: (start..end).map(|idx| crosses[idx as usize].1).collect(),
                        crossed: empties.iter().any(|&idx| crosses[idx as usize].0),
                        counts: counts.clone(),
                        letters: Vec::new(),
                        word: Vec::new(),
                        found: Vec::new(),
                    };
                    fill.fill(0);
                    for word in fill.found {
                        let points = play_points(gc, board_tiles, down, lane, start, &word);
                        keys.insert(play_key(down, lane, start, &word), points);
                    }
                }
            }
        }
    }
    keys
}

fn play_points(
    gc: &game_config::GameConfig,
    board_tiles: &[u8],
    down: bool,
    lane: i8,
    start: i8,
    word: &[u8],
) -> i32 {
    let layout = gc.board_layout();
    let dim = layout.dim();
    let alphabet = gc.alphabet();
    let square = |idx: i8| if down { (idx, lane) } else { (lane, idx) };
    let at = |row: i8, col: i8| dim.at_row_col(row, col).unwrap();
    let on = |row: i8, col: i8| (0..dim.rows).contains(&row) && (0..dim.cols).contains(&col);
    let mut placed = 0u8;
    let (mut main, mut main_multiplier, mut crosses) = (0i32, 1i32, 0i32);
    for (idx, &tile) in (start..).zip(word.iter()) {
        let (row, col) = square(idx);
        if tile == 0 {
            main += i32::from(alphabet.score(board_tiles[at(row, col)]));
            continue;
        }
        placed += 1;
        let premium = &layout.premiums()[at(row, col)];
        let letter = i32::from(alphabet.score(tile)) * i32::from(premium.tile_multiplier);
        main += letter;
        main_multiplier *= i32::from(premium.word_multiplier);
        let (dr, dc) = if down { (0, 1) } else { (1, 0) };
        let mut cross = 0i32;
        let mut cross_len = 1usize;
        for step in [-1i8, 1] {
            let (mut r, mut c) = (row + step * dr, col + step * dc);
            while on(r, c) && board_tiles[at(r, c)] != 0 {
                cross += i32::from(alphabet.score(board_tiles[at(r, c)]));
                cross_len += 1;
                r += step * dr;
                c += step * dc;
            }
        }
        if cross_len > 1 {
            crosses += (cross + letter) * i32::from(premium.word_multiplier);
        }
    }
    let main = if word.len() > 1 {
        main * main_multiplier
    } else {
        0
    };
    main + crosses + i32::from(gc.num_played_bonus(placed))
}

struct Position<'a> {
    gc: &'a game_config::GameConfig,
    graph: &'a std::sync::Arc<kwg::Kwg<kwg::Node24>>,
    klv: &'a klv::Klv<kwg::Node24>,
    lexicon: &'a JoinedLexicon,
    board_tiles: &'a [u8],
    rack: &'a [u8],
    num_exchanges: i16,
}

fn check(
    move_generator: &mut movegen::KurniaMoveGenerator,
    position: &Position<'_>,
) -> error::Returns<(usize, usize, usize, bool)> {
    let board_snapshot = movegen::BoardSnapshot {
        board_tiles: position.board_tiles,
        game_config: position.gc,
        vocab: &wolges::movegen::Vocab::lean(position.graph),
        anagrams: None,
        rack_lengths: None,
        klv: position.klv,
    };
    move_generator.gen_moves_unfiltered(&movegen::GenMovesParams {
        board_snapshot: &board_snapshot,
        rack: position.rack,
        max_gen: usize::MAX,
        num_exchanges_by_this_player: position.num_exchanges,
        pass_policy: movegen::PassPolicy::OnlyWhenForced,
        dynamic_leaves: None,
    })?;
    let mut listed = 0usize;
    let mut got = BTreeMap::new();
    for valued in &move_generator.plays {
        if let movegen::Play::Place {
            down,
            lane,
            idx,
            word,
            score,
        } = &valued.play
        {
            got.insert(play_key(*down, *lane, *idx, word), score.raw());
            listed += 1;
        }
    }
    let want = brute_force(
        position.gc,
        position.board_tiles,
        position.rack,
        position.lexicon,
    )
    .into_iter()
    .map(|(key, points)| (key, points * equity::SCALE))
    .collect::<BTreeMap<_, _>>();
    let down = want.keys().filter(|key| key.0 == 1).count();
    Ok((want.len(), down, listed, listed != got.len() || got != want))
}

fn shifted_layout(
    layout: &board_layout::BoardLayout,
    rows: i8,
    shift: i8,
) -> error::Returns<board_layout::BoardLayout> {
    let dim = layout.dim();
    let mut text = format!("star {} {}\n", layout.star_row(), layout.star_col());
    for row in 0..rows {
        text.push('|');
        for col in 0..dim.cols {
            let premium = &layout.premiums()[dim.at_row_col(row, (col + shift) % dim.cols)?];
            text.push(match (premium.word_multiplier, premium.tile_multiplier) {
                (4, 1) => '~',
                (3, 1) => '=',
                (2, 1) => '-',
                (1, 4) => '^',
                (1, 3) => '"',
                (1, 2) => '\'',
                (1, 1) => ' ',
                (0, 0) => '#',
                _ => return Err("a premium with no square".into()),
            });
        }
        text.push_str("|\n");
    }
    board_layout::BoardLayout::new_static_from_text(&text)
}

fn main() -> error::Returns<()> {
    let args = std::env::args().collect::<Vec<_>>();
    if args.len() != 4 {
        return Err("usage: woadwage_check <kwg> <klv> <seed>".into());
    }
    let kwg = std::sync::Arc::new(kwg::kbwg_from_bytes_alloc(std::fs::read(&args[1])?, false)?);
    let klv = klv::kbwg_from_bytes_alloc(std::fs::read(&args[2])?)?;
    let seed = args[3].parse::<u64>()?;
    let words = lexicon_words(&kwg)?;
    let kad = std::sync::Arc::new(kad_for(&words)?);
    let standard = board_layout::make_standard_board_layout();
    let dim = standard.dim();
    let layouts = [
        ("shifted", shifted_layout(&standard, dim.rows, 1)?),
        ("short", shifted_layout(&standard, dim.rows - 1, 0)?),
    ];
    let mut mismatches = 0usize;
    for (name, jumbled, joined) in [
        ("classic", false, false),
        ("woadwage", false, true),
        ("jumbled", true, false),
        ("jumbled woadwage", true, true),
    ] {
        let rules = |gc: game_config::GameConfig| match (jumbled, joined) {
            (false, false) => gc,
            (false, true) => gc.woadwage(),
            (true, false) => gc.jumbled(),
            (true, true) => gc.jumbled_woadwage(),
        };
        let graph = if jumbled { &kad } else { &kwg };
        let lexicon = JoinedLexicon::new(&words, jumbled, joined);
        for (board, layout) in &layouts {
            let game_config::GameConfig::Static(mut english) =
                game_config::make_english_game_config();
            english.board_layout = layout.clone();
            let gc = rules(game_config::GameConfig::Static(english));
            let mut move_generator = movegen::KurniaMoveGenerator::new(&gc);
            let mut game_state = game_state::GameState::new(&gc);
            let mut rng = rand_xoshiro::Xoshiro256PlusPlus::seed_from_u64(seed);
            game_state.reset_and_draw_tiles(&gc, &mut rng);
            let (plays, down, listed, differ) = check(
                &mut move_generator,
                &Position {
                    gc: &gc,
                    graph,
                    klv: &klv,
                    lexicon: &lexicon,
                    board_tiles: &game_state.board_tiles,
                    rack: &game_state.current_player().rack,
                    num_exchanges: 0,
                },
            )?;
            println!(
                "{name} on the {board} board: seed {seed}, empty board, {plays} plays, {down} down, {listed} listed, {}",
                if differ { "differ" } else { "same" }
            );
            mismatches += differ as usize;
        }
        let gc = rules(game_config::make_english_game_config());
        let mut move_generator = movegen::KurniaMoveGenerator::new(&gc);
        let mut game_state = game_state::GameState::new(&gc);
        let mut rng = rand_xoshiro::Xoshiro256PlusPlus::seed_from_u64(seed);
        game_state.reset_and_draw_tiles(&gc, &mut rng);
        let mut final_scores = vec![0; gc.num_players() as usize];
        let (mut positions, mut plays_seen, mut differ) = (0usize, 0usize, 0usize);
        loop {
            let rack = game_state.current_player().rack.clone();
            let num_exchanges = game_state.current_player().num_exchanges;
            let (plays, _, listed, differs) = check(
                &mut move_generator,
                &Position {
                    gc: &gc,
                    graph,
                    klv: &klv,
                    lexicon: &lexicon,
                    board_tiles: &game_state.board_tiles,
                    rack: &rack,
                    num_exchanges,
                },
            )?;
            if differs {
                println!(
                    "{name}: position {positions} rack {rack:?}: {plays} plays, {listed} listed, differ"
                );
                differ += 1;
            }
            positions += 1;
            plays_seen += plays;
            move_generator.gen_moves_unfiltered(&movegen::GenMovesParams {
                board_snapshot: &movegen::BoardSnapshot {
                    board_tiles: &game_state.board_tiles,
                    game_config: &gc,
                    vocab: &wolges::movegen::Vocab::lean(graph),
                    anagrams: None,
                    rack_lengths: None,
                    klv: &klv,
                },
                rack: &rack,
                max_gen: 1,
                num_exchanges_by_this_player: num_exchanges,
                pass_policy: movegen::PassPolicy::OnlyWhenForced,
                dynamic_leaves: None,
            })?;
            let play = move_generator.plays[0].play.clone();
            game_state.play(&gc, &mut rng, &play)?;
            match game_state.check_game_ended(&gc, &mut final_scores)? {
                game_state::CheckGameEnded::NotEnded => game_state.next_turn()?,
                _ => break,
            }
        }
        println!("{name}: seed {seed}, {positions} positions, {plays_seen} plays, {differ} differ");
        mismatches += differ;
    }
    if mismatches > 0 {
        println!("fail");
        std::process::exit(1);
    }
    println!("ok");
    Ok(())
}
