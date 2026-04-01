// Copyright (C) 2020-2026 Andy Kurnia.

/// A newtype wrapper around f32 for equity values, providing total ordering.
#[derive(Clone, Copy)]
pub struct Equity(f32);

impl Equity {
    pub const NEG_INFINITY: Self = Self(f32::NEG_INFINITY);
    pub const INFINITY: Self = Self(f32::INFINITY);

    #[inline(always)]
    pub fn new(value: f32) -> Self {
        Self(value)
    }

    #[inline(always)]
    pub fn raw(self) -> f32 {
        self.0
    }

    #[inline(always)]
    pub fn is_finite(self) -> bool {
        self.0.is_finite()
    }
}

impl PartialEq for Equity {
    #[inline(always)]
    fn eq(&self, other: &Self) -> bool {
        self.0.to_bits() == other.0.to_bits()
    }
}

impl Eq for Equity {}

impl PartialOrd for Equity {
    #[inline(always)]
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Equity {
    #[inline(always)]
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.0.total_cmp(&other.0)
    }
}

impl std::fmt::Display for Equity {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

impl std::fmt::Debug for Equity {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}
