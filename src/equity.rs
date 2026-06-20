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

/// Whole-point score from a premultiplied (millipoint) integer score.
///
/// Scores are accumulated in millipoints: the alphabet tile values are
/// premultiplied by SCALE so movegen sums them straight into the equity scale,
/// saving a multiply at equity construction. A score is therefore always a
/// whole multiple of SCALE, so this division is exact. Use it at every display
/// or log boundary that emits a score (as opposed to an equity), so the
/// conversion lives in one place and is not missed.
#[inline(always)]
pub fn descale_score(millipoints: i32) -> i32 {
    millipoints / SCALE
}

/// Premultiplied (millipoint) score from a whole-point score. The inverse of
/// descale_score: use it at every input boundary that supplies a score in
/// points (a parsed position, a hardcoded fixture, a clock adjustment) so the
/// internal millipoint scale never leaks into caller-facing values.
#[inline(always)]
pub fn scale_score(points: i32) -> i32 {
    points * SCALE
}

impl Equity {
    pub const NEG_INFINITY: Self = Self(i32::MIN);
    pub const INFINITY: Self = Self(i32::MAX);
    pub const ZERO: Self = Self(0);

    /// Construct from a raw scaled i32 value.
    #[inline(always)]
    pub fn new(v: i32) -> Self {
        Self(v)
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
