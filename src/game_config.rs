// Copyright (C) 2020-2026 Andy Kurnia.

use super::{alphabet, board_layout, equity, error};
use std::str::FromStr;

pub enum GameRules {
    Classic,
    Jumbled,
}

pub struct StaticGameConfig {
    pub game_rules: GameRules,
    pub alphabet: alphabet::Alphabet,
    pub board_layout: board_layout::BoardLayout,
    pub rack_size: u8,
    pub num_played_bonus: [i16; 256],
    pub num_players: u8,
    pub num_passes_to_end: u8,
    pub challenges_are_passes: bool, // count challenge as pass turn or as zero turn
    pub num_zeros_to_end: u8,
    pub zeros_can_end_empty_board: bool,
    pub exchanges_are_zeros: bool,
    pub exchanges_allowed_per_player: i16,
    pub exchange_tile_limit: i16, // >= 1
}

pub enum GameConfig {
    Static(StaticGameConfig),
}

impl GameConfig {
    #[inline(always)]
    pub fn alphabet(&self) -> &alphabet::Alphabet {
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
    pub fn rack_size(&self) -> u8 {
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
    pub fn num_passes_to_end(&self) -> u8 {
        match self {
            GameConfig::Static(x) => x.num_passes_to_end,
        }
    }

    #[inline(always)]
    pub fn challenges_are_passes(&self) -> bool {
        match self {
            GameConfig::Static(x) => x.challenges_are_passes,
        }
    }

    #[inline(always)]
    pub fn num_zeros_to_end(&self) -> u8 {
        match self {
            GameConfig::Static(x) => x.num_zeros_to_end,
        }
    }

    #[inline(always)]
    pub fn zeros_can_end_empty_board(&self) -> bool {
        match self {
            GameConfig::Static(x) => x.zeros_can_end_empty_board,
        }
    }

    #[inline(always)]
    pub fn exchanges_are_zeros(&self) -> bool {
        match self {
            GameConfig::Static(x) => x.exchanges_are_zeros,
        }
    }

    #[inline(always)]
    pub fn exchanges_allowed_per_player(&self) -> i16 {
        match self {
            GameConfig::Static(x) => x.exchanges_allowed_per_player,
        }
    }

    #[inline(always)]
    pub fn exchange_tile_limit(&self) -> i16 {
        match self {
            GameConfig::Static(x) => x.exchange_tile_limit,
        }
    }

    #[inline(always)]
    pub fn num_played_bonus(&self, num_played: u8) -> i16 {
        match self {
            GameConfig::Static(x) => x.num_played_bonus[num_played as usize],
        }
    }

    // never positive
    #[inline(always)]
    pub fn time_adjustment(&self, clock_ms: i64) -> i64 {
        match self {
            GameConfig::Static(..) => {
                // branchless
                -(((!clock_ms / 60000) + 1) * 10) & -((clock_ms < 0) as i64)
            }
        }
    }

    #[inline(always)]
    pub fn game_rules(&self) -> &GameRules {
        match self {
            GameConfig::Static(x) => &x.game_rules,
        }
    }
}

#[inline]
pub fn make_num_played_bonus(bonuses: &[(u8, i16)]) -> [i16; 256] {
    let mut num_played_bonus = [0; 256];
    for &(num_played, bonus) in bonuses {
        num_played_bonus[num_played as usize] = bonus;
    }
    num_played_bonus
}

#[inline]
pub fn make_catalan_game_config() -> GameConfig {
    GameConfig::Static(StaticGameConfig {
        game_rules: GameRules::Classic,
        alphabet: alphabet::make_catalan_alphabet(),
        board_layout: board_layout::make_standard_board_layout(),
        rack_size: 7,
        num_played_bonus: make_num_played_bonus(&[(7, 50)]),
        num_players: 2,
        num_passes_to_end: 0,
        challenges_are_passes: false,
        num_zeros_to_end: 6,
        zeros_can_end_empty_board: true,
        exchanges_are_zeros: true,
        exchanges_allowed_per_player: i16::MAX,
        exchange_tile_limit: 7,
    })
}

#[inline]
pub fn make_jumbled_catalan_game_config() -> GameConfig {
    GameConfig::Static(StaticGameConfig {
        game_rules: GameRules::Jumbled,
        alphabet: alphabet::make_catalan_alphabet(),
        board_layout: board_layout::make_standard_board_layout(),
        rack_size: 7,
        num_played_bonus: make_num_played_bonus(&[(7, 50)]),
        num_players: 2,
        num_passes_to_end: 0,
        challenges_are_passes: false,
        num_zeros_to_end: 6,
        zeros_can_end_empty_board: true,
        exchanges_are_zeros: true,
        exchanges_allowed_per_player: i16::MAX,
        exchange_tile_limit: 7,
    })
}

#[inline]
pub fn make_super_catalan_game_config() -> GameConfig {
    GameConfig::Static(StaticGameConfig {
        game_rules: GameRules::Classic,
        alphabet: alphabet::make_super_catalan_alphabet(),
        board_layout: board_layout::make_super_board_layout(),
        rack_size: 7,
        num_played_bonus: make_num_played_bonus(&[(7, 50)]),
        num_players: 2,
        num_passes_to_end: 0,
        challenges_are_passes: false,
        num_zeros_to_end: 6,
        zeros_can_end_empty_board: true,
        exchanges_are_zeros: true,
        exchanges_allowed_per_player: i16::MAX,
        exchange_tile_limit: 7,
    })
}

#[inline]
pub fn make_jumbled_super_catalan_game_config() -> GameConfig {
    GameConfig::Static(StaticGameConfig {
        game_rules: GameRules::Jumbled,
        alphabet: alphabet::make_super_catalan_alphabet(),
        board_layout: board_layout::make_super_board_layout(),
        rack_size: 7,
        num_played_bonus: make_num_played_bonus(&[(7, 50)]),
        num_players: 2,
        num_passes_to_end: 0,
        challenges_are_passes: false,
        num_zeros_to_end: 6,
        zeros_can_end_empty_board: true,
        exchanges_are_zeros: true,
        exchanges_allowed_per_player: i16::MAX,
        exchange_tile_limit: 7,
    })
}

#[inline]
pub fn make_dutch_game_config() -> GameConfig {
    GameConfig::Static(StaticGameConfig {
        game_rules: GameRules::Classic,
        alphabet: alphabet::make_dutch_alphabet(),
        board_layout: board_layout::make_standard_board_layout(),
        rack_size: 7,
        num_played_bonus: make_num_played_bonus(&[(7, 50)]),
        num_players: 2,
        num_passes_to_end: 0,
        challenges_are_passes: false,
        num_zeros_to_end: 6,
        zeros_can_end_empty_board: true,
        exchanges_are_zeros: true,
        exchanges_allowed_per_player: i16::MAX,
        exchange_tile_limit: 7,
    })
}

#[inline]
pub fn make_jumbled_dutch_game_config() -> GameConfig {
    GameConfig::Static(StaticGameConfig {
        game_rules: GameRules::Jumbled,
        alphabet: alphabet::make_dutch_alphabet(),
        board_layout: board_layout::make_standard_board_layout(),
        rack_size: 7,
        num_played_bonus: make_num_played_bonus(&[(7, 50)]),
        num_players: 2,
        num_passes_to_end: 0,
        challenges_are_passes: false,
        num_zeros_to_end: 6,
        zeros_can_end_empty_board: true,
        exchanges_are_zeros: true,
        exchanges_allowed_per_player: i16::MAX,
        exchange_tile_limit: 7,
    })
}

#[inline]
pub fn make_english_game_config() -> GameConfig {
    GameConfig::Static(StaticGameConfig {
        game_rules: GameRules::Classic,
        alphabet: alphabet::make_english_alphabet(),
        board_layout: board_layout::make_standard_board_layout(),
        rack_size: 7,
        num_played_bonus: make_num_played_bonus(&[(7, 50)]),
        num_players: 2,
        num_passes_to_end: 0,
        challenges_are_passes: false,
        num_zeros_to_end: 6,
        zeros_can_end_empty_board: true,
        exchanges_are_zeros: true,
        exchanges_allowed_per_player: i16::MAX,
        exchange_tile_limit: 7,
    })
}

#[cfg(test)]
#[inline]
pub fn make_exchange_test_game_config() -> GameConfig {
    GameConfig::Static(StaticGameConfig {
        game_rules: GameRules::Classic,
        alphabet: alphabet::make_english_alphabet(),
        board_layout: board_layout::make_standard_board_layout(),
        rack_size: 7,
        num_played_bonus: make_num_played_bonus(&[(7, 50)]),
        num_players: 2,
        num_passes_to_end: 2,
        challenges_are_passes: true,
        num_zeros_to_end: 3,
        zeros_can_end_empty_board: false,
        exchanges_are_zeros: true,
        exchanges_allowed_per_player: i16::MAX,
        exchange_tile_limit: 1,
    })
}

#[cfg(test)]
#[inline]
pub fn make_exchange_unsolvable_test_game_config() -> GameConfig {
    GameConfig::Static(StaticGameConfig {
        game_rules: GameRules::Classic,
        alphabet: alphabet::make_english_alphabet(),
        board_layout: board_layout::make_standard_board_layout(),
        rack_size: 7,
        num_played_bonus: make_num_played_bonus(&[(7, 50)]),
        num_players: 2,
        num_passes_to_end: 2,
        challenges_are_passes: true,
        num_zeros_to_end: 3,
        zeros_can_end_empty_board: false,
        exchanges_are_zeros: false,
        exchanges_allowed_per_player: i16::MAX,
        exchange_tile_limit: 1,
    })
}

#[cfg(test)]
#[inline]
pub fn make_board_test_game_config(board_layout: board_layout::BoardLayout) -> GameConfig {
    GameConfig::Static(StaticGameConfig {
        game_rules: GameRules::Classic,
        alphabet: alphabet::make_english_alphabet(),
        board_layout,
        rack_size: 7,
        num_played_bonus: make_num_played_bonus(&[(7, 50)]),
        num_players: 2,
        num_passes_to_end: 0,
        challenges_are_passes: false,
        num_zeros_to_end: 6,
        zeros_can_end_empty_board: true,
        exchanges_are_zeros: true,
        exchanges_allowed_per_player: i16::MAX,
        exchange_tile_limit: 7,
    })
}

#[inline]
pub fn make_jumbled_english_game_config() -> GameConfig {
    GameConfig::Static(StaticGameConfig {
        game_rules: GameRules::Jumbled,
        alphabet: alphabet::make_english_alphabet(),
        board_layout: board_layout::make_standard_board_layout(),
        rack_size: 7,
        num_played_bonus: make_num_played_bonus(&[(7, 50)]),
        num_players: 2,
        num_passes_to_end: 0,
        challenges_are_passes: false,
        num_zeros_to_end: 6,
        zeros_can_end_empty_board: true,
        exchanges_are_zeros: true,
        exchanges_allowed_per_player: i16::MAX,
        exchange_tile_limit: 7,
    })
}

#[inline]
pub fn make_punctured_english_game_config() -> GameConfig {
    GameConfig::Static(StaticGameConfig {
        game_rules: GameRules::Classic,
        alphabet: alphabet::make_english_alphabet(),
        board_layout: board_layout::make_punctured_board_layout(),
        rack_size: 7,
        num_played_bonus: make_num_played_bonus(&[(7, 50)]),
        num_players: 2,
        num_passes_to_end: 0,
        challenges_are_passes: false,
        num_zeros_to_end: 6,
        zeros_can_end_empty_board: true,
        exchanges_are_zeros: true,
        exchanges_allowed_per_player: i16::MAX,
        exchange_tile_limit: 7,
    })
}

#[inline]
pub fn make_jumbled_punctured_english_game_config() -> GameConfig {
    GameConfig::Static(StaticGameConfig {
        game_rules: GameRules::Jumbled,
        alphabet: alphabet::make_english_alphabet(),
        board_layout: board_layout::make_punctured_board_layout(),
        rack_size: 7,
        num_played_bonus: make_num_played_bonus(&[(7, 50)]),
        num_players: 2,
        num_passes_to_end: 0,
        challenges_are_passes: false,
        num_zeros_to_end: 6,
        zeros_can_end_empty_board: true,
        exchanges_are_zeros: true,
        exchanges_allowed_per_player: i16::MAX,
        exchange_tile_limit: 7,
    })
}

#[inline]
pub fn make_hong_kong_english_game_config() -> GameConfig {
    GameConfig::Static(StaticGameConfig {
        game_rules: GameRules::Classic,
        alphabet: alphabet::make_hong_kong_english_alphabet(),
        board_layout: board_layout::make_standard_board_layout(),
        rack_size: 9,
        num_played_bonus: make_num_played_bonus(&[(9, 50)]),
        num_players: 2,
        num_passes_to_end: 0,
        challenges_are_passes: false,
        num_zeros_to_end: 6,
        zeros_can_end_empty_board: true,
        exchanges_are_zeros: true,
        exchanges_allowed_per_player: i16::MAX,
        exchange_tile_limit: 9,
    })
}

#[inline]
pub fn make_super_english_game_config() -> GameConfig {
    GameConfig::Static(StaticGameConfig {
        game_rules: GameRules::Classic,
        alphabet: alphabet::make_super_english_alphabet(),
        board_layout: board_layout::make_super_board_layout(),
        rack_size: 7,
        num_played_bonus: make_num_played_bonus(&[(7, 50)]),
        num_players: 2,
        num_passes_to_end: 0,
        challenges_are_passes: false,
        num_zeros_to_end: 6,
        zeros_can_end_empty_board: true,
        exchanges_are_zeros: true,
        exchanges_allowed_per_player: i16::MAX,
        exchange_tile_limit: 7,
    })
}

#[inline]
pub fn make_jumbled_super_english_game_config() -> GameConfig {
    GameConfig::Static(StaticGameConfig {
        game_rules: GameRules::Jumbled,
        alphabet: alphabet::make_super_english_alphabet(),
        board_layout: board_layout::make_super_board_layout(),
        rack_size: 7,
        num_played_bonus: make_num_played_bonus(&[(7, 50)]),
        num_players: 2,
        num_passes_to_end: 0,
        challenges_are_passes: false,
        num_zeros_to_end: 6,
        zeros_can_end_empty_board: true,
        exchanges_are_zeros: true,
        exchanges_allowed_per_player: i16::MAX,
        exchange_tile_limit: 7,
    })
}

#[inline]
pub fn make_french_game_config() -> GameConfig {
    GameConfig::Static(StaticGameConfig {
        game_rules: GameRules::Classic,
        alphabet: alphabet::make_french_alphabet(),
        board_layout: board_layout::make_standard_board_layout(),
        rack_size: 7,
        num_played_bonus: make_num_played_bonus(&[(7, 50)]),
        num_players: 2,
        num_passes_to_end: 0,
        challenges_are_passes: false,
        num_zeros_to_end: 6,
        zeros_can_end_empty_board: true,
        exchanges_are_zeros: true,
        exchanges_allowed_per_player: i16::MAX,
        exchange_tile_limit: 7,
    })
}

#[inline]
pub fn make_jumbled_french_game_config() -> GameConfig {
    GameConfig::Static(StaticGameConfig {
        game_rules: GameRules::Jumbled,
        alphabet: alphabet::make_french_alphabet(),
        board_layout: board_layout::make_standard_board_layout(),
        rack_size: 7,
        num_played_bonus: make_num_played_bonus(&[(7, 50)]),
        num_players: 2,
        num_passes_to_end: 0,
        challenges_are_passes: false,
        num_zeros_to_end: 6,
        zeros_can_end_empty_board: true,
        exchanges_are_zeros: true,
        exchanges_allowed_per_player: i16::MAX,
        exchange_tile_limit: 7,
    })
}

#[inline]
pub fn make_german_game_config() -> GameConfig {
    GameConfig::Static(StaticGameConfig {
        game_rules: GameRules::Classic,
        alphabet: alphabet::make_german_alphabet(),
        board_layout: board_layout::make_standard_board_layout(),
        rack_size: 7,
        num_played_bonus: make_num_played_bonus(&[(7, 50)]),
        num_players: 2,
        num_passes_to_end: 4,
        challenges_are_passes: false,
        num_zeros_to_end: 0,
        zeros_can_end_empty_board: true,
        exchanges_are_zeros: true,
        exchanges_allowed_per_player: i16::MAX,
        exchange_tile_limit: 7,
    })
}

#[inline]
pub fn make_jumbled_german_game_config() -> GameConfig {
    GameConfig::Static(StaticGameConfig {
        game_rules: GameRules::Jumbled,
        alphabet: alphabet::make_german_alphabet(),
        board_layout: board_layout::make_standard_board_layout(),
        rack_size: 7,
        num_played_bonus: make_num_played_bonus(&[(7, 50)]),
        num_players: 2,
        num_passes_to_end: 4,
        challenges_are_passes: false,
        num_zeros_to_end: 0,
        zeros_can_end_empty_board: true,
        exchanges_are_zeros: true,
        exchanges_allowed_per_player: i16::MAX,
        exchange_tile_limit: 7,
    })
}

#[inline]
pub fn make_norwegian_game_config() -> GameConfig {
    GameConfig::Static(StaticGameConfig {
        game_rules: GameRules::Classic,
        alphabet: alphabet::make_norwegian_alphabet(),
        board_layout: board_layout::make_standard_board_layout(),
        rack_size: 7,
        num_played_bonus: make_num_played_bonus(&[(7, 50)]),
        num_players: 2,
        num_passes_to_end: 0,
        challenges_are_passes: false,
        num_zeros_to_end: 6,
        zeros_can_end_empty_board: true,
        exchanges_are_zeros: true,
        exchanges_allowed_per_player: i16::MAX,
        exchange_tile_limit: 7,
    })
}

#[inline]
pub fn make_jumbled_norwegian_game_config() -> GameConfig {
    GameConfig::Static(StaticGameConfig {
        game_rules: GameRules::Jumbled,
        alphabet: alphabet::make_norwegian_alphabet(),
        board_layout: board_layout::make_standard_board_layout(),
        rack_size: 7,
        num_played_bonus: make_num_played_bonus(&[(7, 50)]),
        num_players: 2,
        num_passes_to_end: 0,
        challenges_are_passes: false,
        num_zeros_to_end: 6,
        zeros_can_end_empty_board: true,
        exchanges_are_zeros: true,
        exchanges_allowed_per_player: i16::MAX,
        exchange_tile_limit: 7,
    })
}

// http://www.pfs.org.pl/regulaminy.php
// select the second tab.
#[inline]
pub fn make_polish_game_config() -> GameConfig {
    GameConfig::Static(StaticGameConfig {
        game_rules: GameRules::Classic,
        alphabet: alphabet::make_polish_alphabet(),
        board_layout: board_layout::make_standard_board_layout(),
        rack_size: 7,
        num_played_bonus: make_num_played_bonus(&[(7, 50)]),
        num_players: 2,
        num_passes_to_end: 4,
        challenges_are_passes: false,
        num_zeros_to_end: 6,
        zeros_can_end_empty_board: true,
        exchanges_are_zeros: false,
        exchanges_allowed_per_player: 3,
        exchange_tile_limit: 7,
    })
}

#[inline]
pub fn make_jumbled_polish_game_config() -> GameConfig {
    GameConfig::Static(StaticGameConfig {
        game_rules: GameRules::Jumbled,
        alphabet: alphabet::make_polish_alphabet(),
        board_layout: board_layout::make_standard_board_layout(),
        rack_size: 7,
        num_played_bonus: make_num_played_bonus(&[(7, 50)]),
        num_players: 2,
        num_passes_to_end: 4,
        challenges_are_passes: false,
        num_zeros_to_end: 6,
        zeros_can_end_empty_board: true,
        exchanges_are_zeros: false,
        exchanges_allowed_per_player: 3,
        exchange_tile_limit: 7,
    })
}

#[inline]
pub fn make_slovene_game_config() -> GameConfig {
    GameConfig::Static(StaticGameConfig {
        game_rules: GameRules::Classic,
        alphabet: alphabet::make_slovene_alphabet(),
        board_layout: board_layout::make_standard_board_layout(),
        rack_size: 7,
        num_played_bonus: make_num_played_bonus(&[(7, 50)]),
        num_players: 2,
        num_passes_to_end: 0,
        challenges_are_passes: false,
        num_zeros_to_end: 6,
        zeros_can_end_empty_board: true,
        exchanges_are_zeros: true,
        exchanges_allowed_per_player: i16::MAX,
        exchange_tile_limit: 7,
    })
}

#[inline]
pub fn make_jumbled_slovene_game_config() -> GameConfig {
    GameConfig::Static(StaticGameConfig {
        game_rules: GameRules::Jumbled,
        alphabet: alphabet::make_slovene_alphabet(),
        board_layout: board_layout::make_standard_board_layout(),
        rack_size: 7,
        num_played_bonus: make_num_played_bonus(&[(7, 50)]),
        num_players: 2,
        num_passes_to_end: 0,
        challenges_are_passes: false,
        num_zeros_to_end: 6,
        zeros_can_end_empty_board: true,
        exchanges_are_zeros: true,
        exchanges_allowed_per_player: i16::MAX,
        exchange_tile_limit: 7,
    })
}

// https://fisescrabble.org/reglamentos/modalidad-clasica/
#[inline]
pub fn make_spanish_game_config() -> GameConfig {
    GameConfig::Static(StaticGameConfig {
        game_rules: GameRules::Classic,
        alphabet: alphabet::make_spanish_alphabet(),
        board_layout: board_layout::make_standard_board_layout(),
        rack_size: 7,
        num_played_bonus: make_num_played_bonus(&[(7, 50)]),
        num_players: 2,
        num_passes_to_end: 4,
        challenges_are_passes: true,
        num_zeros_to_end: 12,
        zeros_can_end_empty_board: false,
        exchanges_are_zeros: true,
        exchanges_allowed_per_player: i16::MAX,
        exchange_tile_limit: 1,
    })
}

#[inline]
pub fn make_jumbled_spanish_game_config() -> GameConfig {
    GameConfig::Static(StaticGameConfig {
        game_rules: GameRules::Jumbled,
        alphabet: alphabet::make_spanish_alphabet(),
        board_layout: board_layout::make_standard_board_layout(),
        rack_size: 7,
        num_played_bonus: make_num_played_bonus(&[(7, 50)]),
        num_players: 2,
        num_passes_to_end: 4,
        challenges_are_passes: true,
        num_zeros_to_end: 12,
        zeros_can_end_empty_board: false,
        exchanges_are_zeros: true,
        exchanges_allowed_per_player: i16::MAX,
        exchange_tile_limit: 1,
    })
}

#[inline]
pub fn make_swedish_game_config() -> GameConfig {
    GameConfig::Static(StaticGameConfig {
        game_rules: GameRules::Classic,
        alphabet: alphabet::make_swedish_alphabet(),
        board_layout: board_layout::make_standard_board_layout(),
        rack_size: 7,
        num_played_bonus: make_num_played_bonus(&[(7, 50)]),
        num_players: 2,
        num_passes_to_end: 0,
        challenges_are_passes: false,
        num_zeros_to_end: 6,
        zeros_can_end_empty_board: true,
        exchanges_are_zeros: true,
        exchanges_allowed_per_player: i16::MAX,
        exchange_tile_limit: 7,
    })
}

#[inline]
pub fn make_jumbled_swedish_game_config() -> GameConfig {
    GameConfig::Static(StaticGameConfig {
        game_rules: GameRules::Jumbled,
        alphabet: alphabet::make_swedish_alphabet(),
        board_layout: board_layout::make_standard_board_layout(),
        rack_size: 7,
        num_played_bonus: make_num_played_bonus(&[(7, 50)]),
        num_players: 2,
        num_passes_to_end: 0,
        challenges_are_passes: false,
        num_zeros_to_end: 6,
        zeros_can_end_empty_board: true,
        exchanges_are_zeros: true,
        exchanges_allowed_per_player: i16::MAX,
        exchange_tile_limit: 7,
    })
}

pub type MakeGameConfig = fn() -> GameConfig;

pub const GAME_CONFIGS: &[(&str, MakeGameConfig)] = &[
    ("catalan", make_catalan_game_config),
    ("dutch", make_dutch_game_config),
    ("english", make_english_game_config),
    ("french", make_french_game_config),
    ("german", make_german_game_config),
    ("hong-kong-english", make_hong_kong_english_game_config),
    ("norwegian", make_norwegian_game_config),
    ("polish", make_polish_game_config),
    ("slovene", make_slovene_game_config),
    ("spanish", make_spanish_game_config),
    ("super-catalan", make_super_catalan_game_config),
    ("super-english", make_super_english_game_config),
    ("swedish", make_swedish_game_config),
];

#[inline]
pub fn make_game_config_by_name(name: &str) -> Option<GameConfig> {
    GAME_CONFIGS
        .iter()
        .find(|(this_name, _)| *this_name == name)
        .map(|(_, make)| make())
}

#[inline]
pub fn parse_num_played_bonus(s: &str, rack_size: u8) -> error::Returns<[i16; 256]> {
    let mut bonuses = Vec::new();
    if s != "none" {
        for entry in s.split(',') {
            let (num_played, bonus) = entry
                .split_once(':')
                .ok_or("a bonus is tiles:points, as in 7:50")?;
            let num_played = u8::from_str(num_played)?;
            if !(1..=rack_size).contains(&num_played) {
                return Err(format!("a rack of {rack_size} cannot play {num_played} tiles").into());
            }
            if bonuses.iter().any(|&(n, _)| n == num_played) {
                return Err(format!("the bonus for {num_played} tiles is given twice").into());
            }
            bonuses.push((num_played, i16::from_str(bonus)?));
        }
    }
    Ok(make_num_played_bonus(&bonuses))
}

#[inline]
fn take<'a>(
    values: &mut std::collections::BTreeMap<&str, &'a str>,
    key: &str,
) -> error::Returns<&'a str> {
    values
        .remove(key)
        .ok_or_else(|| format!("{key} is missing").into())
}

#[inline]
fn yes_no(value: &str) -> error::Returns<bool> {
    match value {
        "yes" => Ok(true),
        "no" => Ok(false),
        _ => Err(format!("{value:?} is not yes or no").into()),
    }
}

#[inline]
fn parse_exchanges(value: &str) -> error::Returns<i16> {
    match value {
        "unlimited" => Ok(i16::MAX),
        exchanges => Ok(i16::from_str(exchanges)?),
    }
}

// a path in a preset file is relative to the preset file.
#[inline]
fn next_to(file: &str, path: &str) -> String {
    match std::path::Path::new(file).parent() {
        Some(dir) => dir.join(path).to_string_lossy().into_owned(),
        None => path.to_owned(),
    }
}

pub struct Options<'a> {
    pub preset: &'a str,
    pub tiles: Option<&'a str>,
    pub board: Option<&'a str>,
    pub rack_size: Option<u8>,
    pub jumbled: bool,
    pub players: Option<u8>,
    pub bingo_bonus: Option<&'a str>,
    pub zeros_to_end: Option<u8>,
    pub passes_to_end: Option<u8>,
    pub exchange_limit: Option<i16>,
    pub exchanges: Option<&'a str>,
}

impl Options<'_> {
    #[inline]
    pub fn make_game_config(
        &self,
        read_file: &impl Fn(&str) -> error::Returns<String>,
    ) -> error::Returns<GameConfig> {
        let GameConfig::Static(preset) = match make_game_config_by_name(self.preset) {
            Some(game_config) => game_config,
            None => GameConfig::new_static_from_text(&read_file(self.preset)?, &|path| {
                read_file(&next_to(self.preset, path))
            })?,
        };
        let rack_size = self.rack_size.unwrap_or(preset.rack_size);
        let game_config = GameConfig::Static(StaticGameConfig {
            game_rules: if self.jumbled {
                GameRules::Jumbled
            } else {
                preset.game_rules
            },
            alphabet: match self.tiles {
                Some(tiles) => alphabet::make_alphabet_from(tiles, read_file)?,
                None => preset.alphabet,
            },
            board_layout: match self.board {
                Some(board) => board_layout::make_board_layout_from(board, read_file)?,
                None => preset.board_layout,
            },
            rack_size,
            num_played_bonus: match self.bingo_bonus {
                Some(bingo_bonus) => parse_num_played_bonus(bingo_bonus, rack_size)?,
                None => preset.num_played_bonus,
            },
            num_players: self.players.unwrap_or(preset.num_players),
            num_passes_to_end: self.passes_to_end.unwrap_or(preset.num_passes_to_end),
            challenges_are_passes: preset.challenges_are_passes,
            num_zeros_to_end: self.zeros_to_end.unwrap_or(preset.num_zeros_to_end),
            zeros_can_end_empty_board: preset.zeros_can_end_empty_board,
            exchanges_are_zeros: preset.exchanges_are_zeros,
            exchanges_allowed_per_player: match self.exchanges {
                Some(exchanges) => parse_exchanges(exchanges)?,
                None => preset.exchanges_allowed_per_player,
            },
            exchange_tile_limit: self.exchange_limit.unwrap_or(preset.exchange_tile_limit),
        });
        game_config.check_parts()?;
        game_config.check_scores()?;
        Ok(game_config)
    }
}

impl GameConfig {
    #[inline]
    pub fn new_static_from_text(
        s: &str,
        read_file: &impl Fn(&str) -> error::Returns<String>,
    ) -> error::Returns<Self> {
        let mut values = std::collections::BTreeMap::new();
        for line in s.lines() {
            if line.trim().is_empty() || line.starts_with('#') {
                continue;
            }
            let (key, value) = line
                .trim_end()
                .split_once(' ')
                .ok_or_else(|| format!("{line:?} is not a key and a value"))?;
            if values.insert(key, value.trim_start()).is_some() {
                return Err(format!("{key} is given twice").into());
            }
        }
        let alphabet = alphabet::make_alphabet_from(take(&mut values, "tiles")?, read_file)?;
        let board_layout =
            board_layout::make_board_layout_from(take(&mut values, "board")?, read_file)?;
        let rack_size = u8::from_str(take(&mut values, "rack-size")?)?;
        let game_config = Self::Static(StaticGameConfig {
            game_rules: match take(&mut values, "rules")? {
                "classic" => GameRules::Classic,
                "jumbled" => GameRules::Jumbled,
                rules => return Err(format!("{rules:?} is not classic or jumbled").into()),
            },
            alphabet,
            board_layout,
            rack_size,
            num_played_bonus: parse_num_played_bonus(take(&mut values, "bingo-bonus")?, rack_size)?,
            num_players: u8::from_str(take(&mut values, "players")?)?,
            num_passes_to_end: u8::from_str(take(&mut values, "passes-to-end")?)?,
            challenges_are_passes: yes_no(take(&mut values, "challenges-are-passes")?)?,
            num_zeros_to_end: u8::from_str(take(&mut values, "zeros-to-end")?)?,
            zeros_can_end_empty_board: yes_no(take(&mut values, "zeros-can-end-empty-board")?)?,
            exchanges_are_zeros: yes_no(take(&mut values, "exchanges-are-zeros")?)?,
            exchanges_allowed_per_player: parse_exchanges(take(&mut values, "exchanges")?)?,
            exchange_tile_limit: i16::from_str(take(&mut values, "exchange-limit")?)?,
        });
        if let Some(key) = values.keys().next() {
            return Err(format!("{key} is not a preset key").into());
        }
        game_config.check_parts()?;
        game_config.check_scores()?;
        Ok(game_config)
    }

    #[inline]
    fn check_parts(&self) -> error::Returns<()> {
        if self.rack_size() == 0
            || self.num_players() == 0
            || self.exchanges_allowed_per_player() < 0
            || self.exchange_tile_limit() < 1
        {
            return Err("a game has a rack, a player and an exchange limit of 1 or more".into());
        }
        if let Some(num_played) = (self.rack_size()..=u8::MAX)
            .find(|&n| n > self.rack_size() && self.num_played_bonus(n) != 0)
        {
            return Err(format!(
                "a bonus for {num_played} tiles is more than a rack of {} holds",
                self.rack_size()
            )
            .into());
        }
        Ok(())
    }

    #[inline]
    pub fn most_one_play_can_score(&self) -> u128 {
        let board_layout = self.board_layout();
        let dim = board_layout.dim();
        let premiums = board_layout.premiums();
        let alphabet = self.alphabet();
        let rack_size = self.rack_size() as u128;
        let tile_score = (0..alphabet.len())
            .map(|tile| alphabet.score(tile).unsigned_abs() as u128)
            .max()
            .unwrap_or(0);
        let tile_multiplier = premiums
            .iter()
            .map(|premium| premium.tile_multiplier.max(1) as u128)
            .max()
            .unwrap_or(1);
        let word_multiplier = premiums
            .iter()
            .map(|premium| premium.word_multiplier.max(1) as u128)
            .max()
            .unwrap_or(1);
        let mut lane_word_multiplier = 1u128;
        for down in [false, true] {
            let (lanes, len) = if down {
                (dim.cols, dim.rows)
            } else {
                (dim.rows, dim.cols)
            };
            for lane in 0..lanes {
                let mut multipliers = (0..len)
                    .map(|i| {
                        let idx = if down {
                            dim.at_row_col(i, lane)
                        } else {
                            dim.at_row_col(lane, i)
                        };
                        premiums[idx].word_multiplier.max(1) as u128
                    })
                    .collect::<Vec<_>>();
                multipliers.sort_unstable_by(|a, b| b.cmp(a));
                lane_word_multiplier = lane_word_multiplier.max(
                    multipliers
                        .iter()
                        .take(self.rack_size() as usize)
                        .fold(1u128, |product, &m| product.saturating_mul(m)),
                );
            }
        }
        let face = dim.rows.max(dim.cols) as u128 * tile_score
            + rack_size * tile_score * (tile_multiplier - 1);
        let bonus = (0..=u8::MAX)
            .map(|num_played| self.num_played_bonus(num_played).unsigned_abs() as u128)
            .max()
            .unwrap_or(0);
        face.saturating_mul(lane_word_multiplier)
            .saturating_add(
                rack_size
                    .saturating_mul(face)
                    .saturating_mul(word_multiplier),
            )
            .saturating_add(bonus)
    }

    #[inline]
    pub fn check_scores(&self) -> error::Returns<()> {
        let most = self.most_one_play_can_score();
        if most.saturating_mul(equity::SCALE as u128) > i32::MAX as u128 {
            return Err(
                format!("one play here can score {most} points, past what a score holds").into(),
            );
        }
        Ok(())
    }

    #[inline]
    pub fn check_leaves(&self, (min_leave, max_leave): (i32, i32)) -> error::Returns<()> {
        let scale = equity::SCALE as i128;
        let most = self.most_one_play_can_score().min(i32::MAX as u128) as i128 * scale;
        let alphabet = self.alphabet();
        let score = |tile| alphabet.score(tile).unsigned_abs() as i128;
        let total_face = (0..alphabet.len())
            .map(|tile| alphabet.freq(tile) as i128 * score(tile))
            .sum::<i128>();
        let rack_face =
            self.rack_size() as i128 * (0..alphabet.len()).map(score).max().unwrap_or(0);
        let play_out = 2 * scale * total_face;
        let penalty = equity::ENDGAME_PENALTY_BASE as i128 + 2 * scale * rack_face;
        if most + (max_leave as i128).max(play_out) > i32::MAX as i128
            || -most + (min_leave as i128).min(-penalty) < i32::MIN as i128
        {
            return Err(format!(
                "leaves from {min_leave} to {max_leave} millipoints can take an equity past what it holds"
            )
            .into());
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[inline]
    fn a_long_overtime_is_still_a_penalty() {
        let gc = make_english_game_config();
        assert_eq!(gc.time_adjustment(i64::MAX), 0);
        assert_eq!(gc.time_adjustment(0), 0);
        assert_eq!(gc.time_adjustment(-1), -10);
        assert_eq!(gc.time_adjustment(-60_000), -10);
        assert_eq!(gc.time_adjustment(-60_001), -20);
        assert_eq!(gc.time_adjustment(-3_277 * 60_000), -32_770);
        assert!(gc.time_adjustment(i64::MIN) < -32_770);
    }

    #[test]
    #[inline]
    fn the_bonus_is_paid_for_a_full_rack_only() {
        for (gc, rack_size) in [
            (make_english_game_config(), 7),
            (make_hong_kong_english_game_config(), 9),
        ] {
            for num_played in 0..=u8::MAX {
                let bonus = if num_played == rack_size { 50 } else { 0 };
                assert_eq!(gc.num_played_bonus(num_played), bonus, "{num_played}");
            }
        }
    }

    #[inline]
    fn preset_text_of(tiles: &str, board: &str, gc: &GameConfig) -> String {
        let bonuses = (0..=u8::MAX)
            .filter(|&n| gc.num_played_bonus(n) != 0)
            .map(|n| format!("{n}:{}", gc.num_played_bonus(n)))
            .collect::<Vec<_>>();
        let yes_no = |b| if b { "yes" } else { "no" };
        format!(
            "# written by the test\n\ntiles {tiles}\nboard {board}\nrules {}\nrack-size {}\n\
             bingo-bonus {}\nplayers {}\npasses-to-end {}\nchallenges-are-passes {}\n\
             zeros-to-end {}\nzeros-can-end-empty-board {}\nexchanges-are-zeros {}\n\
             exchanges {}\nexchange-limit {}\n",
            match gc.game_rules() {
                GameRules::Classic => "classic",
                GameRules::Jumbled => "jumbled",
            },
            gc.rack_size(),
            if bonuses.is_empty() {
                "none".to_string()
            } else {
                bonuses.join(",")
            },
            gc.num_players(),
            gc.num_passes_to_end(),
            yes_no(gc.challenges_are_passes()),
            gc.num_zeros_to_end(),
            yes_no(gc.zeros_can_end_empty_board()),
            yes_no(gc.exchanges_are_zeros()),
            if gc.exchanges_allowed_per_player() == i16::MAX {
                "unlimited".to_string()
            } else {
                gc.exchanges_allowed_per_player().to_string()
            },
            gc.exchange_tile_limit(),
        )
    }

    #[inline]
    fn same_game_config(a: &GameConfig, b: &GameConfig) -> bool {
        let (alphabet, other_alphabet) = (a.alphabet(), b.alphabet());
        let (board_layout, other_board_layout) = (a.board_layout(), b.board_layout());
        matches!(
            (a.game_rules(), b.game_rules()),
            (GameRules::Classic, GameRules::Classic) | (GameRules::Jumbled, GameRules::Jumbled)
        ) && alphabet.len() == other_alphabet.len()
            && (0..alphabet.len()).all(|tile| {
                alphabet.of_board(tile) == other_alphabet.of_board(tile)
                    && alphabet.freq(tile) == other_alphabet.freq(tile)
                    && alphabet.score(tile) == other_alphabet.score(tile)
                    && alphabet.is_vowel(tile) == other_alphabet.is_vowel(tile)
            })
            && board_layout.dim().rows == other_board_layout.dim().rows
            && board_layout.dim().cols == other_board_layout.dim().cols
            && board_layout.star_row() == other_board_layout.star_row()
            && board_layout.star_col() == other_board_layout.star_col()
            && board_layout
                .premiums()
                .iter()
                .zip(other_board_layout.premiums())
                .all(|(p, q)| {
                    p.word_multiplier == q.word_multiplier && p.tile_multiplier == q.tile_multiplier
                })
            && a.rack_size() == b.rack_size()
            && (0..=u8::MAX).all(|n| a.num_played_bonus(n) == b.num_played_bonus(n))
            && a.num_players() == b.num_players()
            && a.num_passes_to_end() == b.num_passes_to_end()
            && a.challenges_are_passes() == b.challenges_are_passes()
            && a.num_zeros_to_end() == b.num_zeros_to_end()
            && a.zeros_can_end_empty_board() == b.zeros_can_end_empty_board()
            && a.exchanges_are_zeros() == b.exchanges_are_zeros()
            && a.exchanges_allowed_per_player() == b.exchanges_allowed_per_player()
            && a.exchange_tile_limit() == b.exchange_tile_limit()
    }

    #[inline]
    fn no_files(path: &str) -> error::Returns<String> {
        Err(format!("no file {path}").into())
    }

    #[test]
    #[inline]
    fn every_bundled_game_is_found_by_its_name() {
        for (i, (name, make)) in GAME_CONFIGS.iter().enumerate() {
            assert!(
                i == 0 || GAME_CONFIGS[i - 1].0 < *name,
                "{name} is out of order"
            );
            let gc = make_game_config_by_name(name).unwrap();
            assert_eq!(gc.rack_size(), make().rack_size(), "{name}");
            assert_eq!(
                gc.alphabet().num_tiles(),
                make().alphabet().num_tiles(),
                "{name}"
            );
        }
        assert!(make_game_config_by_name("klingon").is_none());
    }

    #[test]
    #[inline]
    fn every_bundled_game_reads_back_from_its_text() {
        for (name, make) in GAME_CONFIGS {
            let board = if name.starts_with("super-") {
                "super"
            } else {
                "standard"
            };
            let gc = make();
            let text = preset_text_of(name, board, &gc);
            let read = GameConfig::new_static_from_text(&text, &no_files).unwrap();
            assert!(same_game_config(&read, &gc), "{name}");
        }
        let gc = make_jumbled_english_game_config();
        let text = preset_text_of("english", "standard", &gc);
        let read = GameConfig::new_static_from_text(&text, &no_files).unwrap();
        assert!(same_game_config(&read, &gc), "jumbled");
        assert!(!same_game_config(&read, &make_english_game_config()));
    }

    #[test]
    #[inline]
    fn a_preset_reads_its_tiles_and_board_through_the_given_reader() {
        let gc = make_punctured_english_game_config();
        let tiles = "?\t?\t2\t0\t0\t0\t0\nA\ta\t98\t1\t1\t0\t0\n";
        let board = "star 1 1\n|# #|\n|   |\n|#-#|\n";
        let files = |path: &str| -> error::Returns<String> {
            match path {
                "tiles.txt" => Ok(tiles.to_string()),
                "board.txt" => Ok(board.to_string()),
                _ => no_files(path),
            }
        };
        let text = preset_text_of("tiles.txt", "board.txt", &gc);
        let read = GameConfig::new_static_from_text(&text, &files).unwrap();
        assert_eq!(read.alphabet().len(), 2);
        assert_eq!(read.alphabet().num_tiles(), 100);
        assert_eq!(read.board_layout().dim().rows, 3);
        assert_eq!(read.board_layout().star_col(), 1);
        assert!(GameConfig::new_static_from_text(&text, &no_files).is_err());
    }

    #[test]
    #[inline]
    fn a_preset_file_that_cannot_be_played_is_refused() {
        let good = preset_text_of("english", "standard", &make_english_game_config());
        assert!(GameConfig::new_static_from_text(&good, &no_files).is_ok());
        let edits: &[(&str, &str)] = &[
            ("rack-size 7\n", ""),
            ("rack-size 7\n", "rack-size 7\nrack-size 7\n"),
            ("rack-size 7\n", "rack-size 7\nrack-sizes 7\n"),
            ("rack-size 7\n", "rack-size seven\n"),
            ("rack-size 7\n", "rack-size 0\n"),
            ("rack-size 7\n", "rack-size\n"),
            ("rules classic\n", "rules chess\n"),
            ("bingo-bonus 7:50\n", "bingo-bonus 8:50\n"),
            ("bingo-bonus 7:50\n", "bingo-bonus 0:50\n"),
            ("bingo-bonus 7:50\n", "bingo-bonus 7:50,7:25\n"),
            ("bingo-bonus 7:50\n", "bingo-bonus 7\n"),
            ("players 2\n", "players 0\n"),
            (
                "challenges-are-passes no\n",
                "challenges-are-passes maybe\n",
            ),
            ("exchanges unlimited\n", "exchanges -1\n"),
            ("exchange-limit 7\n", "exchange-limit 0\n"),
            ("tiles english\n", "tiles klingon\n"),
            ("board standard\n", "board round\n"),
        ];
        for (old, new) in edits {
            assert_eq!(good.matches(old).count(), 1, "{old:?}");
            let text = good.replace(old, new);
            assert!(
                GameConfig::new_static_from_text(&text, &no_files).is_err(),
                "{new:?}"
            );
        }
        let no_bonus = good.replace("bingo-bonus 7:50\n", "bingo-bonus none\n");
        let read = GameConfig::new_static_from_text(&no_bonus, &no_files).unwrap();
        assert_eq!(read.num_played_bonus(7), 0);
    }

    #[test]
    #[inline]
    fn a_game_one_play_of_which_could_overflow_a_score_is_refused() {
        for (name, make) in GAME_CONFIGS {
            let gc = make();
            assert!(gc.check_scores().is_ok(), "{name}");
            assert!(gc.check_leaves((-100_000, 100_000)).is_ok(), "{name}");
            assert!(gc.check_leaves((i32::MIN, 0)).is_err(), "{name}");
            assert!(gc.check_leaves((0, i32::MAX)).is_err(), "{name}");
        }
        let quadruple_words = format!(
            "star 63 63\n{}",
            format!("|{}|\n", "~".repeat(127)).repeat(127)
        );
        let files = |path: &str| -> error::Returns<String> {
            match path {
                "board.txt" => Ok(quadruple_words.clone()),
                _ => no_files(path),
            }
        };
        let text = preset_text_of("english", "board.txt", &make_english_game_config());
        assert!(GameConfig::new_static_from_text(&text, &files).is_err());
        let text = preset_text_of("english", "super", &make_english_game_config());
        assert!(GameConfig::new_static_from_text(&text, &files).is_ok());
    }

    #[test]
    #[inline]
    fn an_option_overrides_its_part_of_the_preset() {
        let options = Options {
            preset: "english",
            tiles: None,
            board: None,
            rack_size: None,
            jumbled: false,
            players: None,
            bingo_bonus: None,
            zeros_to_end: None,
            passes_to_end: None,
            exchange_limit: None,
            exchanges: None,
        };
        let gc = options.make_game_config(&no_files).unwrap();
        assert!(same_game_config(&gc, &make_english_game_config()));
        let jumbled = Options {
            preset: "super-english",
            tiles: None,
            board: None,
            rack_size: None,
            jumbled: true,
            players: None,
            bingo_bonus: None,
            zeros_to_end: None,
            passes_to_end: None,
            exchange_limit: None,
            exchanges: None,
        };
        let gc = jumbled.make_game_config(&no_files).unwrap();
        assert!(same_game_config(
            &gc,
            &make_jumbled_super_english_game_config()
        ));
        let every_field = Options {
            preset: "hong-kong-english",
            tiles: Some("english"),
            board: Some("super"),
            rack_size: Some(8),
            jumbled: false,
            players: Some(3),
            bingo_bonus: Some("7:25,8:50"),
            zeros_to_end: Some(4),
            passes_to_end: Some(2),
            exchange_limit: Some(3),
            exchanges: Some("5"),
        };
        let gc = every_field.make_game_config(&no_files).unwrap();
        assert_eq!(gc.alphabet().num_tiles(), 100);
        assert_eq!(gc.board_layout().dim().rows, 21);
        assert_eq!(gc.rack_size(), 8);
        assert_eq!(gc.num_players(), 3);
        assert_eq!(gc.num_played_bonus(7), 25);
        assert_eq!(gc.num_played_bonus(8), 50);
        assert_eq!(gc.num_played_bonus(9), 0);
        assert_eq!(gc.num_zeros_to_end(), 4);
        assert_eq!(gc.num_passes_to_end(), 2);
        assert_eq!(gc.exchange_tile_limit(), 3);
        assert_eq!(gc.exchanges_allowed_per_player(), 5);
        let smaller_rack = Options {
            preset: "hong-kong-english",
            tiles: None,
            board: None,
            rack_size: Some(7),
            jumbled: false,
            players: None,
            bingo_bonus: None,
            zeros_to_end: None,
            passes_to_end: None,
            exchange_limit: None,
            exchanges: None,
        };
        assert!(smaller_rack.make_game_config(&no_files).is_err());
        let unknown = Options {
            preset: "chess",
            tiles: None,
            board: None,
            rack_size: None,
            jumbled: false,
            players: None,
            bingo_bonus: None,
            zeros_to_end: None,
            passes_to_end: None,
            exchange_limit: None,
            exchanges: None,
        };
        assert!(unknown.make_game_config(&no_files).is_err());
        let preset_text = preset_text_of("english", "standard", &make_spanish_game_config());
        let files = |path: &str| -> error::Returns<String> {
            match path {
                "spanish-ish.txt" => Ok(preset_text.clone()),
                _ => no_files(path),
            }
        };
        let from_file = Options {
            preset: "spanish-ish.txt",
            tiles: None,
            board: None,
            rack_size: None,
            jumbled: false,
            players: None,
            bingo_bonus: None,
            zeros_to_end: None,
            passes_to_end: None,
            exchange_limit: None,
            exchanges: Some("unlimited"),
        };
        let gc = from_file.make_game_config(&files).unwrap();
        assert_eq!(gc.exchange_tile_limit(), 1);
        assert_eq!(gc.exchanges_allowed_per_player(), i16::MAX);
        assert!(gc.challenges_are_passes());
        assert_eq!(gc.alphabet().num_tiles(), 100);
    }

    #[test]
    #[inline]
    fn a_preset_file_reads_its_parts_next_to_it() {
        let preset_text = preset_text_of("tiles.txt", "board.txt", &make_english_game_config());
        let tiles = "?\t?\t2\t0\t0\t0\t0\nA\ta\t98\t1\t1\t0\t0\n";
        let board = "star 1 1\n|# #|\n|   |\n|#-#|\n";
        let is = |path: &str, file: &str| std::path::Path::new(path) == std::path::Path::new(file);
        let files = |path: &str| -> error::Returns<String> {
            if is(path, "games/small.txt") {
                Ok(preset_text.clone())
            } else if is(path, "games/tiles.txt") {
                Ok(tiles.to_string())
            } else if is(path, "games/board.txt") {
                Ok(board.to_string())
            } else {
                no_files(path)
            }
        };
        let in_a_folder = Options {
            preset: "games/small.txt",
            tiles: None,
            board: None,
            rack_size: None,
            jumbled: false,
            players: None,
            bingo_bonus: None,
            zeros_to_end: None,
            passes_to_end: None,
            exchange_limit: None,
            exchanges: None,
        };
        let gc = in_a_folder.make_game_config(&files).unwrap();
        assert_eq!(gc.alphabet().num_tiles(), 100);
        assert_eq!(gc.board_layout().dim().rows, 3);
        let board_named_here = Options {
            preset: "games/small.txt",
            tiles: None,
            board: Some("board.txt"),
            rack_size: None,
            jumbled: false,
            players: None,
            bingo_bonus: None,
            zeros_to_end: None,
            passes_to_end: None,
            exchange_limit: None,
            exchanges: None,
        };
        assert!(board_named_here.make_game_config(&files).is_err());
    }
}
