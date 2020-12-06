use super::{alphabet,board_layout};

pub trait TraitGameConfig<'a> {
fn alphabet(&self)->&'a alphabet::Alphabet<'a>;
fn board_layout(&self)->&'a board_layout::BoardLayout<'a>;
}

pub struct StaticGameConfig<'a> {
  alphabet : &'a alphabet::Alphabet<'a>,
  board_layout :&'a board_layout::BoardLayout<'a>,
}

impl<'a> TraitGameConfig<'a> for StaticGameConfig<'a> {

    #[inline(always)]
fn alphabet(&self)->&'a alphabet::Alphabet<'a>{self.alphabet}

    #[inline(always)]
fn board_layout(&self)->&'a board_layout::BoardLayout<'a>{self.board_layout}
}

pub enum GameConfig<'a> {
  Static(StaticGameConfig<'a>),
}

impl<'a> TraitGameConfig<'a> for GameConfig<'a> {

    #[inline(always)]
fn alphabet(&self)->&'a alphabet::Alphabet<'a>{match self{GameConfig::Static(x)=>x.alphabet()}}

    #[inline(always)]
fn board_layout(&self)->&'a board_layout::BoardLayout<'a>{match self{GameConfig::Static(x)=>x.board_layout()}}
}

pub static COMMON_ENGLISH_GAME_CONFIG : GameConfig = GameConfig::Static(StaticGameConfig{
  alphabet:  &alphabet::ENGLISH_ALPHABET,
  board_layout:  &board_layout::COMMON_BOARD_LAYOUT,
});
