// Copyright (C) 2020-2026 Andy Kurnia.

pub struct GameTimers {
    instant: std::time::Instant,
    pub clocks_ms: Box<[i64]>,
    pub turn: i8, // -1 for nobody's
}

impl GameTimers {
    #[inline(always)]
    pub fn new(num_players: u8) -> Self {
        Self {
            instant: std::time::Instant::now(),
            clocks_ms: vec![0; num_players as usize].into_boxed_slice(),
            turn: -1,
        }
    }

    #[inline(always)]
    pub fn reset_to(&mut self, initial_ms: i64) {
        self.clocks_ms.iter_mut().for_each(|m| *m = initial_ms);
        self.turn = -1;
        self.instant = std::time::Instant::now();
    }

    #[inline(always)]
    pub fn set_turn(&mut self, new_turn: i8) -> crate::error::Returns<()> {
        let new_instant = std::time::Instant::now();
        if let Ok(turn) = usize::try_from(self.turn)
            && turn < self.clocks_ms.len()
        {
            self.clocks_ms[turn] = self.get_timer_as_at(new_instant, turn)?;
        }
        self.instant = new_instant;
        self.turn = new_turn;
        Ok(())
    }

    #[inline(always)]
    pub fn get_timer_as_at(
        &self,
        instant: std::time::Instant,
        turn: usize,
    ) -> crate::error::Returns<i64> {
        let clock = self.clocks_ms.get(turn).copied().unwrap_or(0);
        if turn < self.clocks_ms.len() && usize::try_from(self.turn) == Ok(turn) {
            let elapsed =
                i64::try_from(instant.saturating_duration_since(self.instant).as_millis())?;
            clock
                .checked_sub(elapsed)
                .ok_or_else(|| format!("player {} clock is out of range", turn + 1).into())
        } else {
            Ok(clock)
        }
    }
}
