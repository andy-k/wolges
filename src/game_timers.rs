// Copyright (C) 2020-2024 Andy Kurnia.

pub struct GameTimers {
    instant: std::time::Instant,
    pub clocks_ms: Box<[i64]>,
    pub turn: i8, // -1 for nobody's
}

impl GameTimers {
    pub fn new(num_players: u8) -> Self {
        Self {
            instant: std::time::Instant::now(),
            clocks_ms: vec![0; num_players as usize].into_boxed_slice(),
            turn: -1,
        }
    }

    pub fn reset_to(&mut self, initial_ms: i64) {
        self.clocks_ms.iter_mut().for_each(|m| *m = initial_ms);
        self.turn = -1;
        self.instant = std::time::Instant::now();
    }

    pub fn set_turn(&mut self, new_turn: i8) {
        let new_instant = std::time::Instant::now();
        if self.turn >= 0 && (self.turn as usize) < self.clocks_ms.len() {
            self.clocks_ms[self.turn as usize] -= new_instant
                .saturating_duration_since(self.instant)
                .as_millis() as i64;
        }
        self.instant = new_instant;
        self.turn = new_turn;
    }

    pub fn get_timer_as_at(&self, instant: std::time::Instant, turn: usize) -> i64 {
        if turn < self.clocks_ms.len() {
            self.clocks_ms[turn]
                - (-((turn == self.turn as usize) as i64)
                    & instant.saturating_duration_since(self.instant).as_millis() as i64)
        } else {
            0
        }
    }
}
