// Copyright (C) 2020-2021 Andy Kurnia. All rights reserved.

use super::{bag, error, game_config, movegen};
use rand::prelude::*;

fn use_tiles<II: IntoIterator<Item = u8>>(
    rack: &mut Vec<u8>,
    tiles_iter: II,
) -> error::Returns<()> {
    for tile in tiles_iter {
        let pos = rack.iter().rposition(|&t| t == tile).ok_or("bad tile")?;
        rack.swap_remove(pos);
    }
    Ok(())
}

pub struct GamePlayer {
    pub score: i16,
    pub rack: Vec<u8>,
}

impl<'a> Clone for GamePlayer {
    #[inline(always)]
    fn clone(&self) -> Self {
        Self {
            score: self.score,
            rack: self.rack.clone(),
        }
    }

    #[inline(always)]
    fn clone_from(&mut self, source: &Self) {
        self.score.clone_from(&source.score);
        self.rack.clone_from(&source.rack);
    }
}

pub struct GameState<'a> {
    pub game_config: &'a game_config::GameConfig<'a>,
    pub players: Box<[GamePlayer]>,
    pub board_tiles: Box<[u8]>,
    pub bag: bag::Bag,
    pub turn: u8,
    pub zero_turns: u16,
}

impl<'a> Clone for GameState<'a> {
    #[inline(always)]
    fn clone(&self) -> Self {
        Self {
            game_config: self.game_config,
            players: self.players.clone(),
            board_tiles: self.board_tiles.clone(),
            bag: self.bag.clone(),
            turn: self.turn,
            zero_turns: self.zero_turns,
        }
    }

    #[inline(always)]
    fn clone_from(&mut self, source: &Self) {
        self.game_config.clone_from(&source.game_config);
        self.players.clone_from(&source.players);
        self.board_tiles.clone_from(&source.board_tiles);
        self.bag.clone_from(&source.bag);
        self.turn.clone_from(&source.turn);
        self.zero_turns.clone_from(&source.zero_turns);
    }
}

impl<'a> GameState<'a> {
    pub fn new(game_config: &'a game_config::GameConfig) -> Self {
        let board_layout = game_config.board_layout();
        let dim = board_layout.dim();
        let rack_size = game_config.rack_size() as usize;
        let alphabet = game_config.alphabet();
        Self {
            game_config,
            players: (0..game_config.num_players())
                .map(|_| GamePlayer {
                    score: 0,
                    rack: Vec::with_capacity(rack_size),
                })
                .collect(),
            board_tiles: vec![0u8; (dim.rows as usize) * (dim.cols as usize)].into_boxed_slice(),
            bag: bag::Bag::new(&alphabet),
            turn: 0,
            zero_turns: 0,
        }
    }

    pub fn clone_transient_stuffs_from(&mut self, source: &GameState) {
        // does not clone game_config
        self.players.clone_from(&source.players);
        self.board_tiles.clone_from(&source.board_tiles);
        self.bag.clone_from(&source.bag);
        self.turn.clone_from(&source.turn);
        self.zero_turns.clone_from(&source.zero_turns);
    }

    fn reset(&mut self) {
        for player in self.players.iter_mut() {
            player.score = 0;
            self.bag.0.extend_from_slice(&player.rack);
            player.rack.clear();
        }
        for &tile in self.board_tiles.iter().filter(|&&tile| tile != 0) {
            self.bag.0.push(tile);
        }
        self.board_tiles.iter_mut().for_each(|m| *m = 0);
        self.turn = 0;
        self.zero_turns = 0;
    }

    pub fn reset_and_draw_tiles(&mut self, mut rng: &mut dyn RngCore) {
        self.reset();
        self.bag.shuffle(&mut rng);
        for player in self.players.iter_mut() {
            self.bag
                .replenish(&mut player.rack, self.game_config.rack_size() as usize);
        }
    }

    pub fn current_player(&self) -> &GamePlayer {
        &self.players[self.turn as usize]
    }

    pub fn play(&mut self, mut rng: &mut dyn RngCore, play: &movegen::Play) -> error::Returns<()> {
        let current_player = &mut self.players[self.turn as usize];
        match play {
            movegen::Play::Exchange { tiles } => {
                use_tiles(&mut current_player.rack, tiles.iter().copied())?;
                self.bag.replenish(
                    &mut current_player.rack,
                    self.game_config.rack_size() as usize,
                );
                self.bag.put_back(&mut rng, &tiles);
                self.zero_turns += 1;
            }
            movegen::Play::Place {
                down,
                lane,
                idx,
                word,
                score,
            } => {
                let dim = self.game_config.board_layout().dim();
                let strider = if *down {
                    dim.down(*lane)
                } else {
                    dim.across(*lane)
                };

                // place the tiles
                for (i, &tile) in (*idx..).zip(word.iter()) {
                    if tile != 0 {
                        self.board_tiles[strider.at(i)] = tile;
                    }
                }

                current_player.score += score;
                use_tiles(
                    &mut current_player.rack,
                    word.iter().filter_map(|&tile| {
                        if tile != 0 {
                            Some(tile & !((tile as i8) >> 7) as u8)
                        } else {
                            None
                        }
                    }),
                )?;
                self.bag.replenish(
                    &mut current_player.rack,
                    self.game_config.rack_size() as usize,
                );
                self.zero_turns = 0;
            }
        }
        Ok(())
    }

    pub fn next_turn(&mut self) {
        let num_players = self.players.len() as u8;
        self.turn += 1;
        self.turn -= num_players & -((self.turn >= num_players) as i8) as u8;
    }

    pub fn check_game_ended(&self, final_scores: &mut [i16]) -> CheckGameEnded {
        if self.current_player().rack.is_empty() {
            for (i, player) in self.players.iter().enumerate() {
                final_scores[i] = player.score;
            }
            if self.players.len() == 2 {
                final_scores[self.turn as usize] += 2 * self
                    .game_config
                    .alphabet()
                    .rack_score(&self.players[(1 - self.turn) as usize].rack);
            } else {
                let mut earned = 0;
                for (i, player) in self.players.iter().enumerate() {
                    let this_rack = self.game_config.alphabet().rack_score(&player.rack);
                    final_scores[i] -= this_rack;
                    earned += this_rack;
                }
                final_scores[self.turn as usize] += earned;
            }
            CheckGameEnded::PlayedOut
        } else if self.zero_turns >= self.game_config.num_players() as u16 * 3 {
            for (i, player) in self.players.iter().enumerate() {
                final_scores[i] =
                    player.score - self.game_config.alphabet().rack_score(&player.rack);
            }
            CheckGameEnded::ZeroScores
        } else {
            CheckGameEnded::NotEnded
        }
    }
}

pub enum CheckGameEnded {
    NotEnded,
    PlayedOut,
    ZeroScores,
}
