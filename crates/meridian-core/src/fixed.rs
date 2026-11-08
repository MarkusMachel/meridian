//! Q32.32 fixed-point number (i64 raw, 32 fractional bits).
//! Multiplication and division use i128 intermediates so they cannot
//! overflow midway. Results that do not fit saturate (documented, tested).

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct Fixed(i64);

const FRAC_BITS: u32 = 32;

impl Fixed {
    pub const ZERO: Fixed = Fixed(0);
    pub const ONE: Fixed = Fixed(1 << FRAC_BITS);
    pub const MAX: Fixed = Fixed(i64::MAX);
    pub const MIN: Fixed = Fixed(i64::MIN);

    #[inline]
    pub const fn from_raw(raw: i64) -> Fixed {
        Fixed(raw)
    }
    #[inline]
    pub const fn raw(self) -> i64 {
        self.0
    }
    /// From a whole number. Saturates if out of range.
    pub fn from_int(v: i64) -> Fixed {
        Fixed(saturate((v as i128) << FRAC_BITS))
    }
    /// From a ratio, rounding toward zero. Panics if `den` is zero.
    pub fn from_ratio(num: i64, den: i64) -> Fixed {
        Fixed::from_int(num) / Fixed::from_int(den)
    }
    /// Whole part, rounding toward negative infinity.
    pub fn floor_int(self) -> i64 {
        self.0 >> FRAC_BITS
    }
    pub fn saturating_add(self, o: Fixed) -> Fixed {
        Fixed(self.0.saturating_add(o.0))
    }
    pub fn saturating_sub(self, o: Fixed) -> Fixed {
        Fixed(self.0.saturating_sub(o.0))
    }
    pub fn checked_add(self, o: Fixed) -> Option<Fixed> {
        self.0.checked_add(o.0).map(Fixed)
    }
    pub fn checked_sub(self, o: Fixed) -> Option<Fixed> {
        self.0.checked_sub(o.0).map(Fixed)
    }
    pub fn abs(self) -> Fixed {
        Fixed(self.0.saturating_abs())
    }
    /// Display only. Never feed the result back into the simulation.
    pub fn to_f64_for_display(self) -> f64 {
        self.0 as f64 / (1u64 << FRAC_BITS) as f64
    }
}

/// Multiply, rounding toward negative infinity, saturating on overflow.
impl std::ops::Mul for Fixed {
    type Output = Fixed;
    fn mul(self, o: Fixed) -> Fixed {
        Fixed(saturate((self.0 as i128 * o.0 as i128) >> FRAC_BITS))
    }
}

/// Divide, rounding toward zero, saturating on overflow.
/// Panics on division by zero (a bug in the caller).
impl std::ops::Div for Fixed {
    type Output = Fixed;
    fn div(self, o: Fixed) -> Fixed {
        assert!(o.0 != 0, "Fixed division by zero");
        Fixed(saturate(((self.0 as i128) << FRAC_BITS) / o.0 as i128))
    }
}

fn saturate(v: i128) -> i64 {
    if v > i64::MAX as i128 {
        i64::MAX
    } else if v < i64::MIN as i128 {
        i64::MIN
    } else {
        v as i64
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ops::{Div, Mul};

    #[test]
    fn basics() {
        assert_eq!(Fixed::from_int(3).mul(Fixed::from_int(4)), Fixed::from_int(12));
        assert_eq!(Fixed::from_int(12).div(Fixed::from_int(4)), Fixed::from_int(3));
        assert_eq!(Fixed::from_ratio(1, 2).mul(Fixed::from_int(2)), Fixed::ONE);
        assert_eq!(Fixed::from_ratio(1, 2).raw(), 1 << 31);
    }

    #[test]
    fn negatives_and_floor() {
        assert_eq!(Fixed::from_ratio(-1, 2).floor_int(), -1);
        assert_eq!(Fixed::from_ratio(3, 2).floor_int(), 1);
        assert_eq!(Fixed::from_int(-3).mul(Fixed::from_int(4)), Fixed::from_int(-12));
        // Division rounds toward zero.
        assert_eq!(Fixed::from_raw(-1).div(Fixed::from_int(2)), Fixed::ZERO);
    }

    #[test]
    fn wide_intermediate_does_not_overflow() {
        // 2^30 * 2^30 = 2^60 in real terms would overflow Q32.32, saturates.
        let big = Fixed::from_int(1 << 30);
        assert_eq!(big.mul(big), Fixed::MAX);
        // But a large value times a fraction stays exact via i128.
        let half = Fixed::from_ratio(1, 2);
        assert_eq!(big.mul(half), Fixed::from_int(1 << 29));
        // Dividing a large value by a large value works.
        assert_eq!(big.div(big), Fixed::ONE);
    }

    #[test]
    fn saturation_and_checked() {
        assert_eq!(Fixed::MAX.saturating_add(Fixed::ONE), Fixed::MAX);
        assert_eq!(Fixed::MIN.saturating_sub(Fixed::ONE), Fixed::MIN);
        assert_eq!(Fixed::MAX.checked_add(Fixed::ONE), None);
        assert_eq!(Fixed::from_int(i64::MAX), Fixed::MAX);
        assert_eq!(Fixed::MIN.abs(), Fixed::MAX);
    }

    #[test]
    #[should_panic]
    fn divide_by_zero_panics() {
        let _ = Fixed::ONE.div(Fixed::ZERO);
    }
}
