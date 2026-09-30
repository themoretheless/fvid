/// Decimal seconds, parsed exactly into microseconds (no floating-point rounding).
pub fn parse_time(text: &str) -> std::result::Result<i64, String> {
    let (whole, fraction) = text.split_once('.').unwrap_or((text, ""));
    if whole.is_empty()
        || !whole.bytes().all(|b| b.is_ascii_digit())
        || fraction.len() > 6
        || !fraction.bytes().all(|b| b.is_ascii_digit())
    {
        return Err(
            "time must be nonnegative decimal seconds with at most 6 fractional digits".into(),
        );
    }
    let whole = whole.parse::<i64>().map_err(|_| "time overflow")?;
    let fraction = if fraction.is_empty() {
        0
    } else {
        fraction.parse::<i64>().map_err(|_| "invalid fraction")?
            * 10i64.pow(6 - fraction.len() as u32)
    };
    whole
        .checked_mul(1_000_000)
        .and_then(|v| v.checked_add(fraction))
        .ok_or_else(|| "time overflow".into())
}
#[cfg(test)]
mod tests {
    use super::parse_time;
    #[test]
    fn exact_microsecond_boundaries_and_overflow() {
        for (text, expected) in [
            ("0", 0),
            ("0.000001", 1),
            ("01.250000", 1_250_000),
            ("1.", 1_000_000),
            ("9223372036854.775807", i64::MAX),
        ] {
            assert_eq!(parse_time(text).unwrap(), expected);
        }
        for text in [
            "9223372036854.775808",
            "9223372036855",
            "18446744073709551615",
            ".5",
            "-1",
            "1e3",
            "NaN",
            "1.0000001",
            "1.2.3",
            "1:00",
        ] {
            assert!(parse_time(text).is_err(), "{text}");
        }
    }
}
