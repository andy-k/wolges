use super::{alphabet, board_layout};

pub struct StaticGameConfig<'a> {
    alphabet: &'a alphabet::Alphabet<'a>,
    board_layout: &'a board_layout::BoardLayout<'a>,
    rack_size: i8,
    num_players: u8,
}

pub enum GameConfig<'a> {
    Static(StaticGameConfig<'a>),
}

impl<'a> GameConfig<'a> {
    #[inline(always)]
    pub fn alphabet(&self) -> &'a alphabet::Alphabet<'a> {
        match self {
            GameConfig::Static(x) => x.alphabet,
        }
    }

    #[inline(always)]
    pub fn board_layout(&self) -> &'a board_layout::BoardLayout<'a> {
        match self {
            GameConfig::Static(x) => x.board_layout,
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
}

pub static COMMON_ENGLISH_GAME_CONFIG: GameConfig = GameConfig::Static(StaticGameConfig {
    alphabet: &alphabet::ENGLISH_ALPHABET,
    board_layout: &board_layout::COMMON_BOARD_LAYOUT,
    rack_size: 7,
    num_players: 2,
});
