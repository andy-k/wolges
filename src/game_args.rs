// Copyright (C) 2020-2026 Andy Kurnia.

use wolges::{error, game_config};

// the game a binary plays: a preset, and what replaces each of its parts.
// flatten it last: its help heading carries over to the arguments after it.
#[derive(clap::Args)]
#[command(next_help_heading = "Game")]
pub struct GameArgs {
    #[arg(
        long,
        value_name = "NAME|FILE",
        default_value = "english",
        help = "a bundled preset or a preset file"
    )]
    preset: String,
    #[arg(
        long,
        value_name = "NAME|FILE",
        help = "the tiles, instead of the preset's"
    )]
    tiles: Option<String>,
    #[arg(
        long,
        value_name = "NAME|FILE",
        help = "the board, instead of the preset's"
    )]
    board: Option<String>,
    #[arg(long, help = "the rack size, instead of the preset's")]
    rack_size: Option<u8>,
    #[arg(long, help = "jumbled rules; the word graph is then a .kad")]
    jumbled: bool,
    #[arg(long, help = "the number of players, instead of the preset's")]
    players: Option<u8>,
    #[arg(
        long,
        value_name = "TILES:POINTS,...",
        help = "the bonus by tiles played (none for no bonus), instead of the preset's"
    )]
    bingo_bonus: Option<String>,
    #[arg(
        long,
        help = "the scoreless turns in a row that end a game (0: never), instead of the preset's"
    )]
    zeros_to_end: Option<u8>,
    #[arg(
        long,
        help = "the passes in a row that end a game (0: never), instead of the preset's"
    )]
    passes_to_end: Option<u8>,
    #[arg(
        long,
        help = "the fewest tiles the bag holds for an exchange, instead of the preset's"
    )]
    exchange_limit: Option<i16>,
    #[arg(
        long,
        value_name = "N|unlimited",
        help = "the exchanges each player may make, instead of the preset's"
    )]
    exchanges: Option<String>,
}

impl GameArgs {
    // a preset file and the files the options give are read from disk.
    #[inline]
    pub fn make_game_config(&self) -> error::Returns<game_config::GameConfig> {
        game_config::Options {
            preset: &self.preset,
            tiles: self.tiles.as_deref(),
            board: self.board.as_deref(),
            rack_size: self.rack_size,
            jumbled: self.jumbled,
            players: self.players,
            bingo_bonus: self.bingo_bonus.as_deref(),
            zeros_to_end: self.zeros_to_end,
            passes_to_end: self.passes_to_end,
            exchange_limit: self.exchange_limit,
            exchanges: self.exchanges.as_deref(),
        }
        .make_game_config(&|path| Ok(std::fs::read_to_string(path)?))
    }
}
