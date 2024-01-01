// Copyright (C) 2020-2024 Andy Kurnia.

use super::{alphabet, board_layout, error, game_config, game_state, game_timers};
use std::str::FromStr;

#[inline(always)]
pub fn empty_label(board_layout: &board_layout::BoardLayout, row: i8, col: i8) -> &'static str {
    if row == board_layout.star_row() && col == board_layout.star_col() {
        return "*";
    }
    let premium = board_layout.premiums()[board_layout.dim().at_row_col(row, col)];
    match premium.word_multiplier {
        4 => "~",
        3 => "=",
        2 => "-",
        1 => match premium.tile_multiplier {
            4 => "^",
            3 => "\"",
            2 => "\'",
            1 => " ",
            _ => panic!(),
        },
        0 => match premium.tile_multiplier {
            0 => "#",
            _ => panic!(),
        },
        _ => panic!(),
    }
}

#[inline(always)]
pub fn board_label<'a>(
    alphabet: &'a alphabet::Alphabet<'a>,
    board_layout: &board_layout::BoardLayout,
    board_tiles: &'a [u8],
    row: i8,
    col: i8,
) -> &'a str {
    alphabet
        .of_board(board_tiles[board_layout.dim().at_row_col(row, col)])
        .unwrap_or_else(|| empty_label(board_layout, row, col))
}

pub struct ColumnStr(usize);

impl std::fmt::Display for ColumnStr {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if f.width().is_some() {
            // allocates, but no choice.
            #[allow(clippy::recursive_format_impl)]
            return f.pad(&format!("{self}"));
        }
        if self.0 >= 26 {
            // usize::MAX is about 26**14 so recursion may be ok.
            write!(f, "{}", Self(self.0 / 26 - 1))?;
        }
        write!(f, "{}", ((self.0 % 26) as u8 + 0x41) as char)?;
        Ok(())
    }
}

// Negative numbers not handled.
pub fn column(col: i8) -> ColumnStr {
    ColumnStr(col as usize)
}

// Parses ColumnStr strings (passed as str.as_bytes()).
pub fn str_to_column_usize(sb: &[u8]) -> Option<usize> {
    if sb.is_empty() {
        return None;
    }
    let c = sb[0];
    if (0x41..=0x5a).contains(&c) {
        let mut v = c as usize - 0x41;
        for &c in sb[1..].iter() {
            if (0x41..=0x5a).contains(&c) {
                v = v.checked_mul(26)?.checked_add(c as usize - (0x41 - 26))?;
            } else {
                return None;
            }
        }
        Some(v)
    } else {
        None
    }
}

// Parses ColumnStr strings (passed as str.as_bytes()).
pub fn str_to_column_usize_ignore_case(sb: &[u8]) -> Option<usize> {
    if sb.is_empty() {
        return None;
    }
    let c = sb[0] & !0x20;
    if (0x41..=0x5a).contains(&c) {
        let mut v = c as usize - 0x41;
        for &c in sb[1..].iter() {
            let c = c & !0x20;
            if (0x41..=0x5a).contains(&c) {
                v = v.checked_mul(26)?.checked_add(c as usize - (0x41 - 26))?;
            } else {
                return None;
            }
        }
        Some(v)
    } else {
        None
    }
}

struct BoardPrinter<'a> {
    alphabet: &'a alphabet::Alphabet<'a>,
    board_layout: &'a board_layout::BoardLayout,
    board_tiles: &'a [u8],
}

impl std::fmt::Display for BoardPrinter<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if f.width().is_some() {
            // allocates, but no choice.
            #[allow(clippy::recursive_format_impl)]
            return f.pad(&format!("{self}"));
        }
        let ncols: i8 = self.board_layout.dim().cols;
        let max_col_label_width = 1 + ((ncols > 26) as usize);
        let nrows: i8 = self.board_layout.dim().rows;
        let max_row_num_width = 1 + ((nrows >= 10) as usize) + ((nrows >= 100) as usize);
        let max_tile_width = self.alphabet.widest_label_len();
        let w = max_col_label_width.max(max_tile_width);
        write!(f, "{:max_row_num_width$}", "")?;
        for c in 0..ncols {
            write!(f, " {:^w$}", column(c))?;
        }
        writeln!(f)?;
        write!(f, "{:max_row_num_width$}+", "")?;
        for _ in 0..ncols {
            write!(f, "{:-<w$}", "")?;
        }
        for _ in 1..ncols {
            write!(f, "-")?;
        }
        writeln!(f, "+")?;
        for r in 0..nrows {
            write!(f, "{:max_row_num_width$}|", r + 1)?;
            for c in 0..ncols {
                if c > 0 {
                    write!(f, " ")?
                }
                write!(
                    f,
                    "{:^w$}",
                    board_label(self.alphabet, self.board_layout, self.board_tiles, r, c)
                )?;
            }
            writeln!(f, "|{}", r + 1)?;
        }
        write!(f, "{:max_row_num_width$}+", "")?;
        for _ in 0..ncols {
            write!(f, "{:-<w$}", "")?;
        }
        for _ in 1..ncols {
            write!(f, "-")?;
        }
        writeln!(f, "+")?;
        write!(f, "{:max_row_num_width$}", "")?;
        for c in 0..ncols {
            write!(f, " {:^w$}", column(c))?;
        }
        writeln!(f)?;
        Ok(())
    }
}

pub fn print_board(
    alphabet: &alphabet::Alphabet<'_>,
    board_layout: &board_layout::BoardLayout,
    board_tiles: &[u8],
) {
    print!(
        "{}",
        BoardPrinter {
            alphabet,
            board_layout,
            board_tiles
        }
    );
}

pub struct BoardFenner<'a> {
    alphabet: &'a alphabet::Alphabet<'a>,
    board_layout: &'a board_layout::BoardLayout,
    board_tiles: &'a [u8],
}

impl std::fmt::Display for BoardFenner<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if f.width().is_some() {
            // allocates, but no choice.
            #[allow(clippy::recursive_format_impl)]
            return f.pad(&format!("{self}"));
        }
        let mut p = 0usize;
        for r in 0..self.board_layout.dim().rows {
            if r > 0 {
                write!(f, "/")?;
            }
            let mut empties = 0usize;
            for _ in 0..self.board_layout.dim().cols {
                let tile = self.alphabet.of_board(self.board_tiles[p]);
                p += 1;
                match tile {
                    None => {
                        empties += 1;
                    }
                    Some(tile) => {
                        if empties > 0 {
                            write!(f, "{empties}")?;
                            empties = 0;
                        }
                        write!(f, "{tile}")?;
                    }
                }
            }
            if empties > 0 {
                write!(f, "{empties}")?;
            }
        }
        Ok(())
    }
}

impl<'a> BoardFenner<'a> {
    pub fn new(
        alphabet: &'a alphabet::Alphabet<'a>,
        board_layout: &'a board_layout::BoardLayout,
        board_tiles: &'a [u8],
    ) -> Self {
        BoardFenner {
            alphabet,
            board_layout,
            board_tiles,
        }
    }
}

pub struct BoardFenParser<'a> {
    board_layout: &'a board_layout::BoardLayout,
    buf: Box<[u8]>,
    plays_alphabet_reader: alphabet::AlphabetReader<'a>,
}

impl<'a> BoardFenParser<'a> {
    pub fn new(
        alphabet: &'a alphabet::Alphabet<'a>,
        board_layout: &'a board_layout::BoardLayout,
    ) -> Self {
        let dim = board_layout.dim();
        let expected_size = (dim.rows as isize * dim.cols as isize) as usize;
        let plays_alphabet_reader = alphabet::AlphabetReader::new_for_plays(alphabet);
        Self {
            board_layout,
            buf: vec![0; expected_size].into_boxed_slice(),
            plays_alphabet_reader,
        }
    }

    pub fn parse(&mut self, s: &str) -> Result<&[u8], error::MyError> {
        let sb = s.as_bytes();
        let mut ix = 0;
        macro_rules! fmt_error {
            ($msg: expr) => {
                error::new(format!("{} at position {}", $msg, ix))
            };
        }
        let dim = self.board_layout.dim();
        let mut p = 0usize;
        let mut r = 0i8;
        let mut c = 0i8;
        while ix < sb.len() {
            if c >= dim.cols {
                if r < dim.rows - 1 {
                    if sb[ix] == b'/' {
                        r += 1;
                        c = 0;
                        ix += 1;
                    } else {
                        return Err(fmt_error!("expecting slash"));
                    }
                } else {
                    // nothing works
                    return Err(fmt_error!("trailing chars"));
                }
            } else if sb[ix] >= b'1' && sb[ix] <= b'9' {
                // positive numbers only
                let mut jx = ix + 1;
                while jx < sb.len() && sb[jx] >= b'0' && sb[jx] <= b'9' {
                    jx += 1;
                }
                let empties = usize::from_str(&s[ix..jx]).map_err(|err| fmt_error!(err))?;
                if empties > (dim.cols - c) as usize {
                    return Err(fmt_error!("too many empty spaces"));
                }
                for _ in 0..empties {
                    self.buf[p] = 0;
                    p += 1;
                }
                c += empties as i8;
                ix = jx;
            } else if let Some((tile, end_ix)) = self.plays_alphabet_reader.next_tile(sb, ix) {
                self.buf[p] = tile;
                p += 1;
                c += 1;
                ix = end_ix;
            } else {
                return Err(fmt_error!("invalid char"));
            }
        }
        if r != dim.rows - 1 || c != dim.cols {
            return Err(fmt_error!("incomplete board"));
        }
        Ok(&self.buf)
    }
}

struct MsPrinter {
    ms: i64,
}

impl std::fmt::Display for MsPrinter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if f.width().is_some() {
            // allocates, but no choice.
            #[allow(clippy::recursive_format_impl)]
            return f.pad(&format!("{self}"));
        }
        let mut ms = self.ms;
        if ms < 0 {
            write!(f, "-")?;
            ms = -ms;
        }
        let just_ms = ms % 1000;
        let sec = ms / 1000;
        let just_sec = sec % 60;
        let min = sec / 60;
        write!(f, "{min:02}:{just_sec:02}.{just_ms:03}")?;
        Ok(())
    }
}

struct GameStatePrinter<'a> {
    game_config: &'a game_config::GameConfig<'a>,
    game_state: &'a game_state::GameState,
    optional_game_timers: Option<&'a game_timers::GameTimers>,
}

impl std::fmt::Display for GameStatePrinter<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if f.width().is_some() {
            // allocates, but no choice.
            #[allow(clippy::recursive_format_impl)]
            return f.pad(&format!("{self}"));
        }
        writeln!(
            f,
            "{}{}\nPool {}: {}",
            BoardPrinter {
                alphabet: self.game_config.alphabet(),
                board_layout: self.game_config.board_layout(),
                board_tiles: &self.game_state.board_tiles,
            },
            BoardFenner {
                alphabet: self.game_config.alphabet(),
                board_layout: self.game_config.board_layout(),
                board_tiles: &self.game_state.board_tiles,
            },
            self.game_state.bag.0.len(),
            self.game_config.alphabet().fmt_rack(&self.game_state.bag.0)
        )?;
        let now = std::time::Instant::now();
        for (i, player) in self.game_state.players.iter().enumerate() {
            write!(
                f,
                "Player {}: {} {}",
                i + 1,
                player.score,
                self.game_config.alphabet().fmt_rack(&player.rack)
            )?;
            if let Some(game_timers) = self.optional_game_timers {
                let clock_ms = game_timers.get_timer_as_at(now, i);
                write!(f, " {}", MsPrinter { ms: clock_ms })?;
                let adjustment = self.game_config.time_adjustment(clock_ms);
                if adjustment != 0 {
                    write!(f, " ({adjustment})")?;
                }
                if game_timers.turn as usize == i {
                    // may differ from game_state.turn if timer is paused
                    write!(f, " (timer running)")?;
                }
            }
            if self.game_state.turn as usize == i {
                write!(f, " (turn)")?;
            }
            writeln!(f)?;
        }
        Ok(())
    }
}

pub fn print_game_state(
    game_config: &game_config::GameConfig<'_>,
    game_state: &game_state::GameState,
    optional_game_timers: Option<&game_timers::GameTimers>,
) {
    print!(
        "{}",
        GameStatePrinter {
            game_config,
            game_state,
            optional_game_timers
        }
    );
}
