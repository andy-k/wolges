// Copyright (C) 2020-2021 Andy Kurnia.

use super::{alphabet, board_layout};

pub struct StaticGameConfig<'a> {
    alphabet: alphabet::Alphabet<'a>,
    board_layout: board_layout::BoardLayout,
    rack_size: i8,
    num_players: u8,
}

pub enum GameConfig<'a> {
    Static(StaticGameConfig<'a>),
}

impl<'a> GameConfig<'a> {
    #[inline(always)]
    pub fn alphabet(&self) -> &alphabet::Alphabet<'a> {
        match self {
            GameConfig::Static(x) => &x.alphabet,
        }
    }

    #[inline(always)]
    pub fn board_layout(&self) -> &board_layout::BoardLayout {
        match self {
            GameConfig::Static(x) => &x.board_layout,
        }
    }

    #[inline(always)]
    pub fn rack_size(&self) -> i8 {
        match self {
            GameConfig::Static(x) => x.rack_size,
        }
    }

    #[inline(always)]
    pub fn num_players(&self) -> u8 {
        match self {
            GameConfig::Static(x) => x.num_players,
        }
    }

    #[inline(always)]
    pub fn num_played_bonus(&self, num_played: i8) -> i16 {
        match self {
            GameConfig::Static(x) => {
                // branchless
                50 & -((num_played >= x.rack_size) as i16)
            }
        }
    }

    // never positive
    #[inline(always)]
    pub fn time_adjustment(&self, clock_ms: i64) -> i16 {
        match self {
            GameConfig::Static(..) => {
                // branchless
                (-(((!clock_ms / 60000) + 1) * 10) as i16) & -((clock_ms < 0) as i16)
            }
        }
    }
}

pub fn make_common_english_game_config<'a>() -> GameConfig<'a> {
    GameConfig::Static(StaticGameConfig {
        alphabet: alphabet::make_english_alphabet(),
        board_layout: board_layout::make_common_board_layout(),
        rack_size: 7,
        num_players: 2,
    })
}

#[allow(dead_code)]
pub fn make_super_english_game_config<'a>() -> GameConfig<'a> {
    GameConfig::Static(StaticGameConfig {
        alphabet: alphabet::make_super_english_alphabet(),
        board_layout: board_layout::make_super_board_layout(),
        rack_size: 7,
        num_players: 2,
    })
}

#[allow(dead_code)]
pub fn make_polish_game_config<'a>() -> GameConfig<'a> {
    GameConfig::Static(StaticGameConfig {
        alphabet: alphabet::make_polish_alphabet(),
        board_layout: board_layout::make_common_board_layout(),
        rack_size: 7,
        num_players: 2,
    })
}
