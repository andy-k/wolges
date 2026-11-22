// Copyright (C) 2020-2026 Andy Kurnia.

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Equity(i32);

pub const SCALE: i32 = 1000;

pub const OPENING_HOTSPOT_PENALTY: i32 = 700; // 0.7 * SCALE

pub const ENDGAME_PENALTY_BASE: i32 = 10_000; // 10 * SCALE

impl Equity {
    pub const NEG_INFINITY: Self = Self(i32::MIN);
    pub const INFINITY: Self = Self(i32::MAX);
    pub const ZERO: Self = Self(0);

    #[inline(always)]
    pub const fn new(v: i32) -> Self {
        Self(v)
    }

    #[inline(always)]
    pub const fn raw(self) -> i32 {
        self.0
    }

    // scores are accumulated in millipoints, so convert at every display
    // boundary and nowhere else.
    #[inline(always)]
    pub const fn descaled(self) -> Points {
        Points(self.0 / SCALE)
    }

    // a sum or a difference of two scores, none past what an i32 holds
    #[inline(always)]
    pub fn checked_add(self, other: Self) -> Option<Self> {
        self.0.checked_add(other.0).map(Self)
    }

    #[inline(always)]
    pub fn checked_sub(self, other: Self) -> Option<Self> {
        self.0.checked_sub(other.0).map(Self)
    }

    // a sum or a difference of two finite equities, with no infinity to carry
    #[inline(always)]
    pub fn finite_add(self, other: Self) -> Self {
        debug_assert!(
            self.is_finite() && other.is_finite(),
            "{self:?} + {other:?}"
        );
        Self(self.0 + other.0)
    }

    #[inline(always)]
    pub fn finite_sub(self, other: Self) -> Self {
        debug_assert!(
            self.is_finite() && other.is_finite(),
            "{self:?} - {other:?}"
        );
        Self(self.0 - other.0)
    }

    #[inline(always)]
    pub fn as_f64(self) -> f64 {
        self.0 as f64 / SCALE as f64
    }

    #[inline(always)]
    pub fn is_finite(self) -> bool {
        self.0 != i32::MIN && self.0 != i32::MAX
    }
}

// the arithmetic of an f32 holding whole millipoints: an infinity stays one
// through a sum with a finite equity, and negating one gives the other
impl std::ops::Neg for Equity {
    type Output = Self;

    #[inline(always)]
    fn neg(self) -> Self {
        match self.0 {
            i32::MIN => Self::INFINITY,
            i32::MAX => Self::NEG_INFINITY,
            v => Self(-v),
        }
    }
}

impl std::ops::Add for Equity {
    type Output = Self;

    #[inline(always)]
    fn add(self, other: Self) -> Self {
        if !self.is_finite() {
            debug_assert!(other.is_finite() || other == self, "{self:?} + {other:?}");
            self
        } else if !other.is_finite() {
            other
        } else {
            Self(self.0 + other.0)
        }
    }
}

impl std::ops::Sub for Equity {
    type Output = Self;

    #[inline(always)]
    fn sub(self, other: Self) -> Self {
        self + -other
    }
}

impl std::ops::AddAssign for Equity {
    #[inline(always)]
    fn add_assign(&mut self, other: Self) {
        *self = *self + other;
    }
}

impl std::ops::SubAssign for Equity {
    #[inline(always)]
    fn sub_assign(&mut self, other: Self) {
        *self = *self - other;
    }
}

// as the float it stood for prints itself: {} prints a whole number with no
// decimals and a fraction with the few it needs, a precision that many; a width
// pads it as it pads a number, on the left unless told otherwise
impl std::fmt::Display for Equity {
    #[inline(always)]
    fn fmt(&self, fmt: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::fmt::Display::fmt(&self.as_f64(), fmt)
    }
}

impl std::fmt::Debug for Equity {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::fmt::Display::fmt(self, f)
    }
}

// a score in whole points, as a play, a rack, or a player has it
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct Points(i32);

impl Points {
    pub const ZERO: Self = Self(0);

    #[inline(always)]
    pub const fn new(v: i32) -> Self {
        Self(v)
    }

    #[inline(always)]
    pub const fn raw(self) -> i32 {
        self.0
    }

    #[inline(always)]
    pub const fn scaled(self) -> Equity {
        Equity(self.0 * SCALE)
    }
}

impl std::ops::Add for Points {
    type Output = Self;

    #[inline(always)]
    fn add(self, other: Self) -> Self {
        Self(self.0 + other.0)
    }
}

impl std::ops::Sub for Points {
    type Output = Self;

    #[inline(always)]
    fn sub(self, other: Self) -> Self {
        Self(self.0 - other.0)
    }
}

impl std::ops::Neg for Points {
    type Output = Self;

    #[inline(always)]
    fn neg(self) -> Self {
        Self(-self.0)
    }
}

impl std::ops::Mul<i32> for Points {
    type Output = Self;

    #[inline(always)]
    fn mul(self, times: i32) -> Self {
        Self(self.0 * times)
    }
}

impl std::ops::AddAssign for Points {
    #[inline(always)]
    fn add_assign(&mut self, other: Self) {
        self.0 += other.0;
    }
}

impl std::ops::SubAssign for Points {
    #[inline(always)]
    fn sub_assign(&mut self, other: Self) {
        self.0 -= other.0;
    }
}

impl std::iter::Sum for Points {
    #[inline(always)]
    fn sum<I: Iterator<Item = Self>>(iter: I) -> Self {
        Self(iter.map(|p| p.0).sum())
    }
}

impl std::fmt::Display for Points {
    #[inline(always)]
    fn fmt(&self, fmt: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::fmt::Display::fmt(&self.0, fmt)
    }
}

impl std::fmt::Debug for Points {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::fmt::Display::fmt(self, f)
    }
}

#[cfg(test)]
mod tests {
    use super::{Equity, Points};

    #[test]
    #[inline]
    fn an_equity_counts_as_an_f32_of_whole_millipoints() {
        let e = Equity::new;
        let (inf, neg_inf) = (Equity::INFINITY, Equity::NEG_INFINITY);
        assert_eq!(e(7000) + e(-2500), e(4500));
        assert_eq!(e(7000) - e(-2500), e(9500));
        assert_eq!(e(7000).finite_add(e(-2500)), e(4500));
        assert_eq!(e(7000).finite_sub(e(-2500)), e(9500));
        assert_eq!(-e(3), e(-3));
        assert_eq!(-inf, neg_inf);
        assert_eq!(-neg_inf, inf);
        for v in [e(0), e(1), e(-16_777_216), e(16_777_216)] {
            assert_eq!(inf + v, inf);
            assert_eq!(v + inf, inf);
            assert_eq!(neg_inf - v, neg_inf);
            assert_eq!(v - inf, neg_inf);
            assert_eq!(v - neg_inf, inf);
            assert_eq!(neg_inf + v, neg_inf);
        }
        assert_eq!(inf + inf, inf);
        assert_eq!(neg_inf - inf, neg_inf);
        let mut q = e(1);
        q += e(9);
        q -= e(3);
        assert_eq!(q, e(7));
        q -= inf;
        assert_eq!(q, neg_inf);
    }

    #[test]
    #[inline]
    fn points_count_and_print_as_their_integer() {
        let p = Points::new;
        assert_eq!(p(7) + p(5), p(12));
        assert_eq!(p(7) - p(12), p(-5));
        assert_eq!(-p(3), p(-3));
        assert_eq!(p(4) * 2, p(8));
        let mut q = p(1);
        q += p(9);
        q -= p(3);
        assert_eq!(q, p(7));
        assert_eq!([p(1), p(2), p(3)].into_iter().sum::<Points>(), p(6));
        assert_eq!(p(32).scaled(), Equity::new(32000));
        assert_eq!(p(-5).scaled(), Equity::new(-5000));
        assert_eq!(Equity::new(32999).descaled(), p(32));
        assert_eq!(Equity::new(-5999).descaled(), p(-5));
        assert_eq!(format!("{} {:4} {:?}", p(-7), p(32), p(0)), "-7   32 0");
    }

    #[test]
    #[inline]
    fn an_equity_prints_as_the_float_it_stood_for() {
        let e = Equity::new;
        for (millipoints, plain, fixed) in [
            (32000, "32", "32.000"),
            (-87250, "-87.25", "-87.250"),
            (1, "0.001", "0.001"),
            (0, "0", "0.000"),
            (-500, "-0.5", "-0.500"),
        ] {
            assert_eq!(format!("{}", e(millipoints)), plain);
            assert_eq!(format!("{:.3}", e(millipoints)), fixed);
            assert_eq!(
                format!("{}", e(millipoints)),
                format!("{}", e(millipoints).as_f64())
            );
            let v = e(millipoints).as_f64();
            assert_eq!(format!("{:8}", e(millipoints)), format!("{v:8}"));
            assert_eq!(format!("{:>8.3}", e(millipoints)), format!("{v:>8.3}"));
            assert_eq!(format!("{:<8}", e(millipoints)), format!("{v:<8}"));
            assert_eq!(format!("{:^9.1}", e(millipoints)), format!("{v:^9.1}"));
            assert_eq!(format!("{:+}", e(millipoints)), format!("{v:+}"));
            assert_eq!(format!("{:08.2}", e(millipoints)), format!("{v:08.2}"));
        }
        // a width pads an equity on the left, as it pads a number
        assert_eq!(format!("{:8}", e(32000)), "      32");
        assert_eq!(format!("{:8.3}", e(-87250)), " -87.250");
        assert_eq!(format!("{:<8}|", e(-500)), "-0.5    |");
    }
}
