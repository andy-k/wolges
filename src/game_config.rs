// Copyright (C) 2020-2026 Andy Kurnia.

use super::{alphabet, board_layout};

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
}
