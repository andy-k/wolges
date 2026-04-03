// Copyright (C) 2020-2026 Andy Kurnia.

/// Equity value for move evaluation, stored as i32 with scale factor 1000
/// (millipoints). Provides deterministic total ordering.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Equity(i32);

pub const SCALE: i32 = 1000;

// Opening penalty for vowels adjacent to premium squares (0.7 points per vowel).
pub const OPENING_HOTSPOT_PENALTY: i32 = 700; // 0.7 * SCALE

// Endgame penalty base for unplayed tiles (10 points).
pub const ENDGAME_PENALTY_BASE: i32 = 10_000; // 10 * SCALE

impl Equity {
    pub const NEG_INFINITY: Self = Self(i32::MIN);
    pub const INFINITY: Self = Self(i32::MAX);
    pub const ZERO: Self = Self(0);

    /// Construct from a raw scaled i32 value.
    #[inline(always)]
    pub fn new(v: i32) -> Self {
        Self(v)
    }

    /// Construct from an f32 value (scaled and rounded).
    #[inline(always)]
    pub fn from_f32(v: f32) -> Self {
        Self((v * SCALE as f32).round() as i32)
    }

    /// The raw scaled i32 value (1 unit = 0.001 equity points).
    #[inline(always)]
    pub fn raw(self) -> i32 {
        self.0
    }

    /// Convert to f64 for display or external use.
    #[inline(always)]
    pub fn as_f64(self) -> f64 {
        self.0 as f64 / SCALE as f64
    }

    #[inline(always)]
    pub fn is_finite(self) -> bool {
        self.0 != i32::MIN && self.0 != i32::MAX
    }
}

impl std::fmt::Display for Equity {
    fn fmt(&self, fmt: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let precision = fmt.precision().unwrap_or(3);
        let v = self.0 as f64 / SCALE as f64;
        write!(fmt, "{v:.precision$}")
    }
}

impl std::fmt::Debug for Equity {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::fmt::Display::fmt(self, f)
    }
}
