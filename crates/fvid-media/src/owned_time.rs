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

/// Nearest rounding with halfway values away from zero. Overflow is explicit.
pub fn rescale_nearest(value: i64, source: TimeBase, target: TimeBase) -> Result<i64, String> {
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
    let mut quotient = numerator / denominator;
    if (numerator % denominator).abs() * 2 >= denominator {
        quotient += numerator.signum();
    }
    i64::try_from(quotient).map_err(|_| "rescaled timestamp overflow".into())
}
/// Rescale ordinary packet timing, retaining timestamp sentinels and nonpositive duration.
pub fn packet_nearest(
    pts: i64,
    dts: i64,
    duration: i64,
    source: TimeBase,
    target: TimeBase,
) -> Result<[i64; 3], String> {
    let timestamp = |value| {
        if matches!(value, i64::MIN | i64::MAX) {
            Ok(value)
        } else {
            rescale_nearest(value, source, target)
        }
    };
    Ok([
        timestamp(pts)?,
        timestamp(dts)?,
        if duration > 0 {
            rescale_nearest(duration, source, target)?
        } else {
            duration
        },
    ])
}
#[cfg(test)]
mod rounded_tests {
    use super::*;
    #[test]
    fn signed_halfway_rounding_packet_sentinels_and_duration_rules() {
        let source = TimeBase {
            numerator: 1,
            denominator: 2,
        };
        let target = TimeBase {
            numerator: 1,
            denominator: 1,
        };
        for (value, expected) in [
            (-5, -3),
            (-4, -2),
            (-3, -2),
            (-2, -1),
            (-1, -1),
            (0, 0),
            (1, 1),
            (2, 1),
            (3, 2),
            (4, 2),
            (5, 3),
        ] {
            assert_eq!(rescale_nearest(value, source, target).unwrap(), expected);
        }
        assert_eq!(
            packet_nearest(i64::MIN, i64::MAX, 0, source, target).unwrap(),
            [i64::MIN, i64::MAX, 0]
        );
        assert_eq!(
            packet_nearest(3, -3, -1, source, target).unwrap(),
            [2, -2, -1]
        );
        assert_eq!(
            packet_nearest(3, -3, 3, source, target).unwrap(),
            [2, -2, 2]
        );
        assert!(rescale_nearest(i64::MAX, target, source).is_err());
    }
}
