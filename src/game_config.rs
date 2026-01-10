// Copyright (C) 2020-2026 Andy Kurnia.

use super::{alphabet, board_layout};

pub enum GameRules {
    Classic,
    Jumbled,
}

pub struct StaticGameConfig {
    game_rules: GameRules,
    alphabet: alphabet::Alphabet,
    board_layout: board_layout::BoardLayout,
    rack_size: u8,
    num_players: u8,
    num_passes_to_end: u8,
    challenges_are_passes: bool, // count challenge as pass turn or as zero turn
    num_zeros_to_end: u8,
    zeros_can_end_empty_board: bool,
    exchanges_are_zeros: bool,
    exchanges_allowed_per_player: i16,
    exchange_tile_limit: i16, // >= 1
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

    #[inline(always)]
    pub fn game_rules(&self) -> &GameRules {
        match self {
            GameConfig::Static(x) => &x.game_rules,
        }
    }
}

pub fn make_catalan_game_config() -> GameConfig {
    GameConfig::Static(StaticGameConfig {
        game_rules: GameRules::Classic,
        alphabet: alphabet::make_catalan_alphabet(),
        board_layout: board_layout::make_standard_board_layout(),
        rack_size: 7,
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

pub fn make_jumbled_catalan_game_config() -> GameConfig {
    GameConfig::Static(StaticGameConfig {
        game_rules: GameRules::Jumbled,
        alphabet: alphabet::make_catalan_alphabet(),
        board_layout: board_layout::make_standard_board_layout(),
        rack_size: 7,
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

pub fn make_super_catalan_game_config() -> GameConfig {
    GameConfig::Static(StaticGameConfig {
        game_rules: GameRules::Classic,
        alphabet: alphabet::make_super_catalan_alphabet(),
        board_layout: board_layout::make_super_board_layout(),
        rack_size: 7,
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

pub fn make_jumbled_super_catalan_game_config() -> GameConfig {
    GameConfig::Static(StaticGameConfig {
        game_rules: GameRules::Jumbled,
        alphabet: alphabet::make_super_catalan_alphabet(),
        board_layout: board_layout::make_super_board_layout(),
        rack_size: 7,
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

pub fn make_dutch_game_config() -> GameConfig {
    GameConfig::Static(StaticGameConfig {
        game_rules: GameRules::Classic,
        alphabet: alphabet::make_dutch_alphabet(),
        board_layout: board_layout::make_standard_board_layout(),
        rack_size: 7,
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

pub fn make_jumbled_dutch_game_config() -> GameConfig {
    GameConfig::Static(StaticGameConfig {
        game_rules: GameRules::Jumbled,
        alphabet: alphabet::make_dutch_alphabet(),
        board_layout: board_layout::make_standard_board_layout(),
        rack_size: 7,
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

pub fn make_english_game_config() -> GameConfig {
    GameConfig::Static(StaticGameConfig {
        game_rules: GameRules::Classic,
        alphabet: alphabet::make_english_alphabet(),
        board_layout: board_layout::make_standard_board_layout(),
        rack_size: 7,
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

pub fn make_jumbled_english_game_config() -> GameConfig {
    GameConfig::Static(StaticGameConfig {
        game_rules: GameRules::Jumbled,
        alphabet: alphabet::make_english_alphabet(),
        board_layout: board_layout::make_standard_board_layout(),
        rack_size: 7,
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

pub fn make_punctured_english_game_config() -> GameConfig {
    GameConfig::Static(StaticGameConfig {
        game_rules: GameRules::Classic,
        alphabet: alphabet::make_english_alphabet(),
        board_layout: board_layout::make_punctured_board_layout(),
        rack_size: 7,
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

pub fn make_jumbled_punctured_english_game_config() -> GameConfig {
    GameConfig::Static(StaticGameConfig {
        game_rules: GameRules::Jumbled,
        alphabet: alphabet::make_english_alphabet(),
        board_layout: board_layout::make_punctured_board_layout(),
        rack_size: 7,
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

pub fn make_hong_kong_english_game_config() -> GameConfig {
    GameConfig::Static(StaticGameConfig {
        game_rules: GameRules::Classic,
        alphabet: alphabet::make_hong_kong_english_alphabet(),
        board_layout: board_layout::make_standard_board_layout(),
        rack_size: 9,
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

pub fn make_super_english_game_config() -> GameConfig {
    GameConfig::Static(StaticGameConfig {
        game_rules: GameRules::Classic,
        alphabet: alphabet::make_super_english_alphabet(),
        board_layout: board_layout::make_super_board_layout(),
        rack_size: 7,
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

pub fn make_jumbled_super_english_game_config() -> GameConfig {
    GameConfig::Static(StaticGameConfig {
        game_rules: GameRules::Jumbled,
        alphabet: alphabet::make_super_english_alphabet(),
        board_layout: board_layout::make_super_board_layout(),
        rack_size: 7,
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

pub fn make_french_game_config() -> GameConfig {
    GameConfig::Static(StaticGameConfig {
        game_rules: GameRules::Classic,
        alphabet: alphabet::make_french_alphabet(),
        board_layout: board_layout::make_standard_board_layout(),
        rack_size: 7,
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

pub fn make_jumbled_french_game_config() -> GameConfig {
    GameConfig::Static(StaticGameConfig {
        game_rules: GameRules::Jumbled,
        alphabet: alphabet::make_french_alphabet(),
        board_layout: board_layout::make_standard_board_layout(),
        rack_size: 7,
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

pub fn make_german_game_config() -> GameConfig {
    GameConfig::Static(StaticGameConfig {
        game_rules: GameRules::Classic,
        alphabet: alphabet::make_german_alphabet(),
        board_layout: board_layout::make_standard_board_layout(),
        rack_size: 7,
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

pub fn make_jumbled_german_game_config() -> GameConfig {
    GameConfig::Static(StaticGameConfig {
        game_rules: GameRules::Jumbled,
        alphabet: alphabet::make_german_alphabet(),
        board_layout: board_layout::make_standard_board_layout(),
        rack_size: 7,
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

pub fn make_norwegian_game_config() -> GameConfig {
    GameConfig::Static(StaticGameConfig {
        game_rules: GameRules::Classic,
        alphabet: alphabet::make_norwegian_alphabet(),
        board_layout: board_layout::make_standard_board_layout(),
        rack_size: 7,
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

pub fn make_jumbled_norwegian_game_config() -> GameConfig {
    GameConfig::Static(StaticGameConfig {
        game_rules: GameRules::Jumbled,
        alphabet: alphabet::make_norwegian_alphabet(),
        board_layout: board_layout::make_standard_board_layout(),
        rack_size: 7,
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
pub fn make_polish_game_config() -> GameConfig {
    GameConfig::Static(StaticGameConfig {
        game_rules: GameRules::Classic,
        alphabet: alphabet::make_polish_alphabet(),
        board_layout: board_layout::make_standard_board_layout(),
        rack_size: 7,
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

pub fn make_jumbled_polish_game_config() -> GameConfig {
    GameConfig::Static(StaticGameConfig {
        game_rules: GameRules::Jumbled,
        alphabet: alphabet::make_polish_alphabet(),
        board_layout: board_layout::make_standard_board_layout(),
        rack_size: 7,
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

pub fn make_slovene_game_config() -> GameConfig {
    GameConfig::Static(StaticGameConfig {
        game_rules: GameRules::Classic,
        alphabet: alphabet::make_slovene_alphabet(),
        board_layout: board_layout::make_standard_board_layout(),
        rack_size: 7,
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

pub fn make_jumbled_slovene_game_config() -> GameConfig {
    GameConfig::Static(StaticGameConfig {
        game_rules: GameRules::Jumbled,
        alphabet: alphabet::make_slovene_alphabet(),
        board_layout: board_layout::make_standard_board_layout(),
        rack_size: 7,
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
pub fn make_spanish_game_config() -> GameConfig {
    GameConfig::Static(StaticGameConfig {
        game_rules: GameRules::Classic,
        alphabet: alphabet::make_spanish_alphabet(),
        board_layout: board_layout::make_standard_board_layout(),
        rack_size: 7,
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

pub fn make_jumbled_spanish_game_config() -> GameConfig {
    GameConfig::Static(StaticGameConfig {
        game_rules: GameRules::Jumbled,
        alphabet: alphabet::make_spanish_alphabet(),
        board_layout: board_layout::make_standard_board_layout(),
        rack_size: 7,
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
