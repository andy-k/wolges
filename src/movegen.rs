use super::{board_layout, game_config, klv, kwg, matrix};

#[derive(Clone)]
struct CrossSet {
    bits: u64,
    score: i16,
}

struct WorkingBuffer {
    rack_tally: Box<[u8]>,                       // 27 for ?A-Z
    word_buffer: Box<[u8]>,                      // max(r, c)
    cross_set_for_across_plays: Box<[CrossSet]>, // r*c
    cross_set_for_down_plays: Box<[CrossSet]>,   // c*r
    num_tiles_on_board: u16,
}

impl WorkingBuffer {
    fn new(game_config: &game_config::GameConfig) -> Box<Self> {
        let dim = game_config.board_layout().dim();
        let rows_times_cols = ((dim.rows as isize) * (dim.cols as isize)) as usize;
        Box::new(Self {
            rack_tally: vec![0u8; game_config.alphabet().len() as usize].into_boxed_slice(),
            word_buffer: vec![0u8; std::cmp::max(dim.rows, dim.cols) as usize].into_boxed_slice(),
            cross_set_for_across_plays: vec![CrossSet { bits: 0, score: 0 }; rows_times_cols]
                .into_boxed_slice(),
            cross_set_for_down_plays: vec![CrossSet { bits: 0, score: 0 }; rows_times_cols]
                .into_boxed_slice(),
            num_tiles_on_board: 0,
        })
    }
}

pub struct BoardSnapshot<'a> {
    pub board_tiles: &'a [u8],
    pub game_config: &'a game_config::GameConfig<'a>,
    pub kwg: &'a kwg::Kwg,
    pub klv: &'a klv::Klv,
}

fn gen_cross_set<'a>(
    board_snapshot: &'a BoardSnapshot<'a>,
    strider: matrix::Strider,
    cross_sets: &'a mut [CrossSet],
    output_strider: matrix::Strider,
) {
    let len = strider.len();
    for i in 0..output_strider.len() {
        cross_sets[output_strider.at(i)] = CrossSet { bits: 0, score: 0 };
    }

    let alphabet = board_snapshot.game_config.alphabet();
    let mut p = 1;
    let mut score = 0i16;
    let mut k = len;
    for j in (0..len).rev() {
        let b = board_snapshot.board_tiles[strider.at(j)];
        if b != 0 {
            // board has tile
            if p >= 0 {
                // include current tile
                p = board_snapshot.kwg.seek(p, b & 0x7f);
            }
            score += alphabet.score(b) as i16;
            if j == 0 || board_snapshot.board_tiles[strider.at(j - 1)] == 0 {
                // there is a sequence of tiles from j inclusive to k exclusive
                if k < len && !(k + 1 < len && board_snapshot.board_tiles[strider.at(k + 1)] != 0) {
                    // board[k + 1] is empty, compute cross_set[k].
                    let mut bits = 1u64;
                    if p > 0 {
                        // p = DCBA
                        let q = board_snapshot.kwg.seek(p, 0);
                        if q > 0 {
                            // q = DCBA@
                            let mut q = board_snapshot.kwg[q].arc_index();
                            if q > 0 {
                                loop {
                                    if board_snapshot.kwg[q].accepts() {
                                        bits |= 1 << board_snapshot.kwg[q].tile();
                                    }
                                    if board_snapshot.kwg[q].is_end() {
                                        break;
                                    }
                                    q += 1;
                                }
                            }
                        }
                    }
                    cross_sets[output_strider.at(k)] = CrossSet { bits, score };
                }
                if j > 0 {
                    // board[j - 1] is known to be empty
                    let mut bits = 1u64;
                    if p > 0 {
                        // p = DCBA
                        p = board_snapshot.kwg[p].arc_index(); // p = after DCBA
                        if p > 0 {
                            loop {
                                let tile = board_snapshot.kwg[p].tile();
                                if tile != 0 {
                                    // not the gaddag marker
                                    let mut q = p;
                                    // board[j - 2] may or may not be empty.
                                    for k in (0..j - 1).rev() {
                                        let b = board_snapshot.board_tiles[strider.at(k)];
                                        if b == 0 {
                                            break;
                                        }
                                        q = board_snapshot.kwg.seek(q, b & 0x7f);
                                        if q <= 0 {
                                            break;
                                        }
                                    }
                                    if q > 0 && board_snapshot.kwg[q].accepts() {
                                        bits |= 1 << board_snapshot.kwg[q].tile();
                                    }
                                }
                                if board_snapshot.kwg[p].is_end() {
                                    break;
                                }
                                p += 1;
                            }
                        }
                    }
                    // score hasn't included the next batch.
                    for k in (0i8..j - 1).rev() {
                        let b = board_snapshot.board_tiles[strider.at(k)];
                        if b == 0 {
                            break;
                        }
                        score += alphabet.score(b) as i16;
                    }
                    cross_sets[output_strider.at(j - 1)] = CrossSet { bits, score };
                }
            }
        } else {
            // empty square, reset
            p = 1; // cumulative gaddag traversal results
            score = 0; // cumulative face-value score
            k = j; // last seen empty square
        }
    }
}

// word_buffer must have at least strider.len() length.
fn gen_place_moves<'a, CallbackType: FnMut(i8, &[u8], i16, &[u8])>(
    board_snapshot: &'a BoardSnapshot<'a>,
    cross_set_slice: &'a [CrossSet],
    rack_tally: &'a mut [u8],
    strider: matrix::Strider,
    word_buffer: &'a mut [u8],
    single_tile_plays: bool,
    callback: CallbackType,
) {
    let len = strider.len();
    word_buffer
        .iter_mut()
        .take(len as usize)
        .for_each(|m| *m = 0);

    struct Env<'a, CallbackType: FnMut(i8, &[u8], i16, &[u8])> {
        board_snapshot: &'a BoardSnapshot<'a>,
        cross_set_slice: &'a [CrossSet],
        rack_tally: &'a mut [u8],
        strider: matrix::Strider,
        callback: CallbackType,
        word_buffer: &'a mut [u8],
        anchor: i8,
        leftmost: i8,
        rightmost: i8,
        num_played: i8,
        idx_left: i8,
    }

    let mut env = Env {
        board_snapshot,
        cross_set_slice,
        rack_tally,
        strider,
        callback,
        word_buffer,
        anchor: 0,
        leftmost: 0,
        rightmost: 0,
        num_played: 0,
        idx_left: 0,
    };

    fn record<CallbackType: FnMut(i8, &[u8], i16, &[u8])>(
        env: &mut Env<CallbackType>,
        idx_left: i8,
        idx_right: i8,
        main_score: i16,
        perpendicular_score: i16,
        word_multiplier: i8,
    ) {
        let score = main_score * (word_multiplier as i16)
            + perpendicular_score
            + if env.num_played >= 7 { 50 } else { 0 };
        (env.callback)(
            idx_left,
            &env.word_buffer[(idx_left as usize)..(idx_right as usize)],
            score,
            env.rack_tally,
        );
    }

    fn play_right<CallbackType: FnMut(i8, &[u8], i16, &[u8])>(
        env: &mut Env<CallbackType>,
        mut idx: i8,
        mut p: i32,
        mut main_score: i16,
        perpendicular_score: i16,
        word_multiplier: i8,
        mut is_unique: bool,
    ) {
        // tail-recurse placing current sequence of tiles
        while idx < env.rightmost {
            let b = env.board_snapshot.board_tiles[env.strider.at(idx)];
            if b == 0 {
                break;
            }
            p = env.board_snapshot.kwg.seek(p, b & 0x7f);
            if p <= 0 {
                return;
            }
            main_score += env.board_snapshot.game_config.alphabet().score(b) as i16;
            idx += 1;
        }
        if idx > env.anchor + 1
            && (env.num_played + is_unique as i8) >= 2
            && idx - env.idx_left >= 2
            && env.board_snapshot.kwg[p].accepts()
        {
            record(
                env,
                env.idx_left,
                idx,
                main_score,
                perpendicular_score,
                word_multiplier,
            );
        }
        if idx >= env.rightmost {
            return;
        }

        p = env.board_snapshot.kwg[p].arc_index();
        if p <= 0 {
            return;
        }
        let mut this_premium = board_layout::Premium {
            word_multiplier: 0,
            tile_multiplier: 0,
        };
        let mut this_cross_set = CrossSet { bits: 0, score: 0 };
        if idx < env.rightmost {
            this_premium =
                env.board_snapshot.game_config.board_layout().premiums()[env.strider.at(idx)];
            this_cross_set = env.cross_set_slice[idx as usize].clone();
        }
        if this_cross_set.bits == 1 {
            // already handled '@'
            return;
        }
        let new_word_multiplier = word_multiplier * this_premium.word_multiplier;
        let this_cross_bits = if this_cross_set.bits != 0 {
            this_cross_set.bits
        } else {
            is_unique = true;
            !1
        };
        let has_perpendicular = this_cross_set.bits & 1 != 0;
        loop {
            let tile = env.board_snapshot.kwg[p].tile();
            if tile != 0 && this_cross_bits & (1 << tile) != 0 {
                if env.rack_tally[tile as usize] > 0 {
                    env.rack_tally[tile as usize] -= 1;
                    env.num_played += 1;
                    let tile_value = (env.board_snapshot.game_config.alphabet().score(tile) as i16)
                        * (this_premium.tile_multiplier as i16);
                    env.word_buffer[idx as usize] = tile;
                    play_right(
                        env,
                        idx + 1,
                        p,
                        main_score + tile_value,
                        if has_perpendicular {
                            perpendicular_score
                                + (this_cross_set.score + tile_value)
                                    * (this_premium.word_multiplier as i16)
                        } else {
                            perpendicular_score
                        },
                        new_word_multiplier,
                        is_unique,
                    );
                    env.num_played -= 1;
                    env.rack_tally[tile as usize] += 1;
                }
                if env.rack_tally[0] > 0 {
                    env.rack_tally[0] -= 1;
                    env.num_played += 1;
                    // intentional to not hardcode blank tile value as zero
                    let tile_value = (env.board_snapshot.game_config.alphabet().score(0) as i16)
                        * (this_premium.tile_multiplier as i16);
                    env.word_buffer[idx as usize] = tile | 0x80;
                    play_right(
                        env,
                        idx + 1,
                        p,
                        main_score + tile_value,
                        if has_perpendicular {
                            perpendicular_score
                                + (this_cross_set.score + tile_value)
                                    * (this_premium.word_multiplier as i16)
                        } else {
                            perpendicular_score
                        },
                        new_word_multiplier,
                        is_unique,
                    );
                    env.num_played -= 1;
                    env.rack_tally[0] += 1;
                }
            }
            if env.board_snapshot.kwg[p].is_end() {
                break;
            }
            p += 1;
        }
    }

    fn play_left<CallbackType: FnMut(i8, &[u8], i16, &[u8])>(
        env: &mut Env<CallbackType>,
        mut idx: i8,
        mut p: i32,
        mut main_score: i16,
        perpendicular_score: i16,
        word_multiplier: i8,
        mut is_unique: bool,
    ) {
        // tail-recurse placing current sequence of tiles
        while idx >= env.leftmost {
            let b = env.board_snapshot.board_tiles[env.strider.at(idx)];
            if b == 0 {
                break;
            }
            p = env.board_snapshot.kwg.seek(p, b & 0x7f);
            if p <= 0 {
                return;
            }
            main_score += env.board_snapshot.game_config.alphabet().score(b) as i16;
            idx -= 1;
        }
        if (env.num_played + is_unique as i8) >= 2
            && env.anchor - idx >= 2
            && env.board_snapshot.kwg[p].accepts()
        {
            record(
                env,
                idx + 1,
                env.anchor + 1,
                main_score,
                perpendicular_score,
                word_multiplier,
            );
        }

        p = env.board_snapshot.kwg[p].arc_index();
        if p <= 0 {
            return;
        }
        let mut this_premium = board_layout::Premium {
            word_multiplier: 0,
            tile_multiplier: 0,
        };
        let mut this_cross_set = CrossSet { bits: 0, score: 0 };
        if idx >= env.leftmost {
            this_premium =
                env.board_snapshot.game_config.board_layout().premiums()[env.strider.at(idx)];
            this_cross_set = env.cross_set_slice[idx as usize].clone();
        }
        let new_word_multiplier = word_multiplier * this_premium.word_multiplier;
        let this_cross_bits = if this_cross_set.bits != 0 {
            this_cross_set.bits
        } else {
            is_unique = true;
            !1
        };
        let has_perpendicular = this_cross_set.bits & 1 != 0;
        loop {
            let tile = env.board_snapshot.kwg[p].tile();
            if tile == 0 {
                env.idx_left = idx + 1;
                play_right(
                    env,
                    env.anchor + 1,
                    p,
                    main_score,
                    perpendicular_score,
                    word_multiplier,
                    is_unique,
                );
            } else if idx >= env.leftmost && this_cross_bits & (1 << tile) != 0 {
                if env.rack_tally[tile as usize] > 0 {
                    env.rack_tally[tile as usize] -= 1;
                    env.num_played += 1;
                    let tile_value = (env.board_snapshot.game_config.alphabet().score(tile) as i16)
                        * (this_premium.tile_multiplier as i16);
                    env.word_buffer[idx as usize] = tile;
                    play_left(
                        env,
                        idx - 1,
                        p,
                        main_score + tile_value,
                        if has_perpendicular {
                            perpendicular_score
                                + (this_cross_set.score + tile_value)
                                    * (this_premium.word_multiplier as i16)
                        } else {
                            perpendicular_score
                        },
                        new_word_multiplier,
                        is_unique,
                    );
                    env.num_played -= 1;
                    env.rack_tally[tile as usize] += 1;
                }
                if env.rack_tally[0] > 0 {
                    env.rack_tally[0] -= 1;
                    env.num_played += 1;
                    // intentional to not hardcode blank tile value as zero
                    let tile_value = (env.board_snapshot.game_config.alphabet().score(0) as i16)
                        * (this_premium.tile_multiplier as i16);
                    env.word_buffer[idx as usize] = tile | 0x80;
                    play_left(
                        env,
                        idx - 1,
                        p,
                        main_score + tile_value,
                        if has_perpendicular {
                            perpendicular_score
                                + (this_cross_set.score + tile_value)
                                    * (this_premium.word_multiplier as i16)
                        } else {
                            perpendicular_score
                        },
                        new_word_multiplier,
                        is_unique,
                    );
                    env.num_played -= 1;
                    env.rack_tally[0] += 1;
                }
            }
            if env.board_snapshot.kwg[p].is_end() {
                break;
            }
            p += 1;
        }
    }

    fn gen_moves_from<CallbackType: FnMut(i8, &[u8], i16, &[u8])>(
        env: &mut Env<CallbackType>,
        single_tile_plays: bool,
    ) {
        play_left(env, env.anchor, 1, 0, 0, 1, single_tile_plays);
    }

    let mut rightmost = len; // processed up to here
    let mut leftmost = len;
    loop {
        while leftmost > 0 && board_snapshot.board_tiles[strider.at(leftmost - 1)] == 0 {
            leftmost -= 1;
        }
        if leftmost > 0 {
            // board[leftmost - 1] is a tile.
            env.anchor = leftmost - 1;
            env.leftmost = 0;
            env.rightmost = rightmost;
            gen_moves_from(&mut env, single_tile_plays);
        }
        {
            // this part is only relevant if rack has at least two tiles, but passing that is too expensive.
            let mut leftmost = leftmost; // shadowing
            if leftmost > 0 {
                leftmost += 1;
            }
            for anchor in (leftmost..rightmost).rev() {
                let cross_set_bits = cross_set_slice[anchor as usize].bits;
                if cross_set_bits != 0 {
                    if rightmost - leftmost < 2 {
                        // not enough room for 2-tile words
                        break;
                    }
                    if cross_set_bits != 1 {
                        env.anchor = anchor;
                        env.leftmost = leftmost;
                        env.rightmost = rightmost;
                        gen_moves_from(&mut env, single_tile_plays);
                    }
                    rightmost = anchor; // prevent duplicates
                }
            }
        }
        while leftmost > 0 && board_snapshot.board_tiles[strider.at(leftmost - 1)] != 0 {
            leftmost -= 1;
        }
        if leftmost <= 1 {
            break;
        }
        rightmost = leftmost - 1; // prevent touching leftmost tile
    }
}

pub enum Play {
    Pass,
    Exchange {
        tiles: Box<[u8]>,
    },
    Place {
        down: bool,
        lane: i8,
        idx: i8,
        word: Box<[u8]>,
        score: i16,
    },
}

pub struct ValuedMove {
    pub equity: f32,
    pub play: Play,
}

impl PartialEq for ValuedMove {
    fn eq(&self, other: &Self) -> bool {
        other.equity == self.equity
    }
}

impl Eq for ValuedMove {}

impl PartialOrd for ValuedMove {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        other.equity.partial_cmp(&self.equity)
    }
}

impl Ord for ValuedMove {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        match self.partial_cmp(other) {
            Some(x) => x,
            None => std::cmp::Ordering::Equal,
        }
    }
}

use std::fmt::Write;

pub fn write_play(board_snapshot: &BoardSnapshot, play: &Play, s: &mut String) {
    match &play {
        Play::Pass => {
            s.push_str("Pass");
        }
        Play::Exchange { tiles } => {
            let alphabet = board_snapshot.game_config.alphabet();
            s.push_str("Exch. ");
            for &tile in tiles.iter() {
                s.push_str(alphabet.from_rack(tile).unwrap());
            }
        }
        Play::Place {
            down,
            lane,
            idx,
            word,
            score,
        } => {
            let dim = board_snapshot.game_config.board_layout().dim();
            let alphabet = board_snapshot.game_config.alphabet();
            if *down {
                write!(s, "{}{}", (*lane as u8 + 0x41) as char, idx + 1).unwrap();
            } else {
                write!(s, "{}{}", lane + 1, (*idx as u8 + 0x41) as char).unwrap();
            }
            s.push(' ');
            let strider = if *down {
                dim.down(*lane)
            } else {
                dim.across(*lane)
            };
            let mut inside = false;
            for (i, &tile) in word.iter().enumerate() {
                if tile == 0 {
                    if !inside {
                        s.push('(');
                        inside = true;
                    }
                    s.push_str(
                        alphabet
                            .from_board(board_snapshot.board_tiles[strider.at(idx + i as i8)])
                            .unwrap(),
                    );
                } else {
                    if inside {
                        s.push(')');
                        inside = false;
                    }
                    s.push_str(alphabet.from_board(tile).unwrap());
                }
            }
            if inside {
                s.push(')');
            }
            write!(s, " {}", score).unwrap();
        }
    }
}

pub fn kurnia_gen_moves_alloc<'a>(
    board_snapshot: &'a BoardSnapshot<'a>,
    rack: &'a mut [u8],
) -> Vec<ValuedMove> {
    rack.sort_unstable();
    let alphabet = board_snapshot.game_config.alphabet();

    let board_layout = board_snapshot.game_config.board_layout();
    let dim = board_layout.dim();

    let found_moves =
        std::rc::Rc::new(std::cell::RefCell::new(std::collections::BinaryHeap::new()));

    let max_gen = 15;

    fn push_move<F: FnMut() -> Play>(
        found_moves: &std::rc::Rc<std::cell::RefCell<std::collections::BinaryHeap<ValuedMove>>>,
        max_gen: usize,
        equity: f32,
        mut construct_play: F,
    ) {
        if max_gen == 0 {
            return;
        }
        let mut borrowed = found_moves.borrow_mut();
        if borrowed.len() >= max_gen {
            if borrowed.peek().unwrap().equity >= equity {
                return;
            }
            borrowed.pop();
        }
        borrowed.push(ValuedMove {
            equity,
            play: construct_play(),
        });
    };

    let mut working_buffer = WorkingBuffer::new(board_snapshot.game_config);
    kurnia_init_working_buffer(board_snapshot, &mut working_buffer, rack);
    let num_tiles_on_board = working_buffer.num_tiles_on_board;

    let play_out_bonus = if num_tiles_on_board >= 86 {
        let mut unseen_tiles = vec![0u8; alphabet.len() as usize];
        for i in 0..alphabet.len() {
            let af = alphabet.freq(i);
            let rf = working_buffer.rack_tally[i as usize];
            if af >= rf {
                unseen_tiles[i as usize] = af - rf;
            } else {
                panic!("bad pool/rack");
            }
        }
        board_snapshot.board_tiles.iter().for_each(|&t| {
            if t != 0 {
                let ti = if t & 0x80 == 0 { t as usize } else { 0 };
                if unseen_tiles[ti] > 0 {
                    unseen_tiles[ti] -= 1;
                } else {
                    panic!("bad pool/board");
                }
            }
        });
        2 * unseen_tiles
            .iter()
            .enumerate()
            .map(|(tile, num)| *num as i16 * alphabet.score(tile as u8) as i16)
            .sum::<i16>()
    } else {
        0
    };

    let found_place_move =
        |down: bool, lane: i8, idx: i8, word: &[u8], score: i16, rack_tally: &[u8]| {
            let leave_value = if num_tiles_on_board >= 86 {
                0.0
            } else {
                board_snapshot.klv.leave_value_from_tally(rack_tally)
            };
            let other_adjustments = if num_tiles_on_board == 0 {
                let num_lanes = if down { dim.cols } else { dim.rows };
                let strider1 = if lane > 0 {
                    Some(if down {
                        dim.down(lane - 1)
                    } else {
                        dim.across(lane - 1)
                    })
                } else {
                    None
                };
                let strider2 = if lane < num_lanes - 1 {
                    Some(if down {
                        dim.down(lane + 1)
                    } else {
                        dim.across(lane + 1)
                    })
                } else {
                    None
                };
                word.iter()
                    .enumerate()
                    .filter(|(i, &tile)| {
                        tile != 0 && alphabet.is_vowel(tile) && {
                            let ii = idx + *i as i8;
                            (match strider1 {
                                Some(strider) => {
                                    let premium = board_layout.premiums()[strider.at(ii)];
                                    premium.tile_multiplier != 1 || premium.word_multiplier != 1
                                }
                                None => false,
                            }) || (match strider2 {
                                Some(strider) => {
                                    let premium = board_layout.premiums()[strider.at(ii)];
                                    premium.tile_multiplier != 1 || premium.word_multiplier != 1
                                }
                                None => false,
                            })
                        }
                    })
                    .count() as f32
                    * -0.7
            } else if num_tiles_on_board >= 86 {
                let played_out = rack_tally.iter().all(|&num| num == 0);
                (if played_out {
                    play_out_bonus
                } else {
                    -10 - 2 * rack_tally
                        .iter()
                        .enumerate()
                        .map(|(tile, num)| *num as i16 * alphabet.score(tile as u8) as i16)
                        .sum::<i16>()
                }) as f32
            } else {
                0.0
            };
            push_move(
                &found_moves,
                max_gen,
                score as f32 + leave_value + other_adjustments,
                || Play::Place {
                    down,
                    lane,
                    idx,
                    word: word.into(),
                    score,
                },
            );
        };

    let found_exchange_move = |rack_tally: &[u8]| {
        let leave_value = if num_tiles_on_board >= 86 {
            0.0
        } else {
            board_snapshot.klv.leave_value_from_tally(rack_tally)
        };
        push_move(&found_moves, max_gen, leave_value, || {
            let mut leave_vec = Vec::new();
            let rack_len = rack.len();
            let mut i = 0;
            while i < rack_len {
                let tile = rack[i];
                i += rack_tally[tile as usize] as usize;
                while i < rack_len && rack[i] == tile {
                    leave_vec.push(tile);
                    i += 1;
                }
            }
            if leave_vec.is_empty() {
                Play::Pass
            } else {
                Play::Exchange {
                    tiles: leave_vec.into(),
                }
            }
        });
    };

    kurnia_gen_nonplace_moves(&mut working_buffer, found_exchange_move);
    kurnia_gen_place_moves(board_snapshot, &mut working_buffer, found_place_move);

    let mut borrowed = found_moves.borrow_mut();
    println!("found {} moves", borrowed.len());
    let mut result_vec = Vec::with_capacity(borrowed.len());
    while let Some(play) = borrowed.pop() {
        result_vec.push(play);
    }
    result_vec.reverse();

    let mut s = String::new();
    for play in result_vec.iter() {
        s.clear();
        write_play(board_snapshot, &play.play, &mut s);
        println!("{} {}", play.equity, s);
    }

    result_vec
}

fn kurnia_init_working_buffer<'a>(
    board_snapshot: &'a BoardSnapshot<'a>,
    working_buffer: &mut WorkingBuffer,
    rack: &'a [u8],
) {
    working_buffer.rack_tally.iter_mut().for_each(|m| *m = 0);
    for tile in &rack[..] {
        working_buffer.rack_tally[*tile as usize] += 1;
    }

    working_buffer.num_tiles_on_board = board_snapshot
        .board_tiles
        .iter()
        .filter(|&t| *t != 0)
        .count() as u16;
}

fn kurnia_gen_nonplace_moves<FoundExchangeMove: FnMut(&[u8])>(
    working_buffer: &mut WorkingBuffer,
    mut found_exchange_move: FoundExchangeMove,
) {
    struct ExchangeEnv<'a, FoundExchangeMove: FnMut(&[u8])> {
        found_exchange_move: FoundExchangeMove,
        rack_tally: &'a mut [u8],
    }
    fn generate_exchanges<'a, FoundExchangeMove: FnMut(&[u8])>(
        env: &mut ExchangeEnv<'a, FoundExchangeMove>,
        mut idx: u8,
    ) {
        while idx > 0 && env.rack_tally[idx as usize - 1] == 0 {
            idx -= 1;
        }
        if idx == 0 {
            (env.found_exchange_move)(&env.rack_tally);
            return;
        }
        idx -= 1;
        for available in 0..env.rack_tally[idx as usize] + 1 {
            env.rack_tally[idx as usize] = available;
            generate_exchanges(env, idx);
        }
    }
    // 100 tiles, 7 goes to oppo, 7 goes to me, 7 in bag = 79.
    if working_buffer.num_tiles_on_board <= 79 {
        let initial_idx = working_buffer.rack_tally.len() as u8;
        generate_exchanges(
            &mut ExchangeEnv {
                found_exchange_move,
                rack_tally: &mut working_buffer.rack_tally,
            },
            initial_idx,
        );
    } else {
        found_exchange_move(&working_buffer.rack_tally);
    }
}

fn kurnia_gen_place_moves<'a, FoundPlaceMove: FnMut(bool, i8, i8, &[u8], i16, &[u8])>(
    board_snapshot: &'a BoardSnapshot<'a>,
    working_buffer: &mut WorkingBuffer,
    mut found_place_move: FoundPlaceMove,
) {
    let board_layout = board_snapshot.game_config.board_layout();
    let dim = board_layout.dim();

    // striped by row
    for col in 0..dim.cols {
        gen_cross_set(
            &board_snapshot,
            dim.down(col),
            &mut working_buffer.cross_set_for_across_plays,
            matrix::Strider {
                base: col as i16,
                step: dim.cols,
                len: dim.rows,
            },
        );
    }
    if working_buffer.num_tiles_on_board == 0 {
        // empty board activates star
        working_buffer.cross_set_for_across_plays
            [dim.at_row_col(board_layout.star_row(), board_layout.star_col())] =
            CrossSet { bits: !1, score: 0 };
    }
    for row in 0..dim.rows {
        let cross_set_start = ((row as isize) * (dim.cols as isize)) as usize;
        gen_place_moves(
            &board_snapshot,
            &working_buffer.cross_set_for_across_plays
                [cross_set_start..cross_set_start + (dim.cols as usize)],
            &mut working_buffer.rack_tally,
            dim.across(row),
            &mut working_buffer.word_buffer,
            true,
            |idx: i8, word: &[u8], score: i16, rack_tally: &[u8]| {
                found_place_move(false, row, idx, word, score, rack_tally)
            },
        );
    }
    // striped by columns for better cache locality
    for row in 0..dim.rows {
        gen_cross_set(
            &board_snapshot,
            dim.across(row),
            &mut working_buffer.cross_set_for_down_plays,
            matrix::Strider {
                base: row as i16,
                step: dim.rows,
                len: dim.cols,
            },
        );
    }
    for col in 0..dim.cols {
        let cross_set_start = ((col as isize) * (dim.rows as isize)) as usize;
        gen_place_moves(
            &board_snapshot,
            &working_buffer.cross_set_for_down_plays
                [cross_set_start..cross_set_start + (dim.rows as usize)],
            &mut working_buffer.rack_tally,
            dim.down(col),
            &mut working_buffer.word_buffer,
            false,
            |idx: i8, word: &[u8], score: i16, rack_tally: &[u8]| {
                found_place_move(true, col, idx, word, score, rack_tally)
            },
        );
    }
}
