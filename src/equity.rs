// Copyright (C) 2020-2026 Andy Kurnia.

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Equity(i32);

pub const SCALE: i32 = 1000;

pub const OPENING_HOTSPOT_PENALTY: i32 = 700; // 0.7 * SCALE

pub const ENDGAME_PENALTY_BASE: i32 = 10_000; // 10 * SCALE

// scores are accumulated in millipoints, so convert at every display boundary and
// nowhere else.
#[inline(always)]
pub fn descale_score(millipoints: i32) -> i32 {
    millipoints / SCALE
}

#[inline(always)]
pub fn scale_score(points: i32) -> i32 {
    points * SCALE
}

impl Equity {
    pub const NEG_INFINITY: Self = Self(i32::MIN);
    pub const INFINITY: Self = Self(i32::MAX);
    pub const ZERO: Self = Self(0);

    #[inline(always)]
    pub fn new(v: i32) -> Self {
        Self(v)
    }

    #[inline(always)]
    pub fn raw(self) -> i32 {
        self.0
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

#[cfg(test)]
mod tests {
    use super::Equity;

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
