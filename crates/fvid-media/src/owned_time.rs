//! Exact timestamp conversion without floating point or foreign rescale helpers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TimeBase {
    pub numerator: u32,
    pub denominator: u32,
}
pub fn rescale_exact(value: i64, source: TimeBase, target: TimeBase) -> Result<i64, String> {
    if source.numerator == 0
        || source.denominator == 0
        || target.numerator == 0
        || target.denominator == 0
    {
        return Err("invalid time base".into());
    }
    let numerator =
        i128::from(value) * i128::from(source.numerator) * i128::from(target.denominator);
    let denominator = i128::from(source.denominator) * i128::from(target.numerator);
    if numerator % denominator != 0 {
        return Err("output container cannot represent exact packet timing".into());
    }
    i64::try_from(numerator / denominator).map_err(|_| "rescaled timestamp overflow".into())
}
#[cfg(test)]
mod tests {
    use super::*;
    fn base(denominator: u32) -> TimeBase {
        TimeBase {
            numerator: 1,
            denominator,
        }
    }
    #[test]
    fn exact_signed_boundaries_fractional_refusal_and_overflow() {
        for value in [-48000, -48, 0, 48, 48000] {
            let millis = rescale_exact(value, base(48000), base(1000)).unwrap();
            assert_eq!(
                rescale_exact(millis, base(1000), base(48000)).unwrap(),
                value
            );
        }
        assert!(rescale_exact(1, base(44100), base(1000)).is_err());
        assert!(rescale_exact(i64::MAX, base(1), base(48000)).is_err());
        assert_eq!(
            rescale_exact(i64::MIN, base(u32::MAX), base(u32::MAX)).unwrap(),
            i64::MIN
        );
        assert!(rescale_exact(0, base(0), base(48000)).is_err());
    }
}
