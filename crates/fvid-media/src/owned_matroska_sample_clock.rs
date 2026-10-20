//! Resolve sample-clock points against the two adjacent container ticks.
//!
//! A blanket one-tick tolerance also hides a real whole-tick gap when the
//! expected sample point is already on a tick. Test the floor/ceil bracket
//! instead; two ns cover independently rounded integer-ns coordinates.

/// Whether a declared packet point can quantize the cumulative sample point.
/// Source and epoch use the same timestamp coordinate system (including delay).
/// Zero rate or tick is invalid and never qualifies for sample-clock snapping.
pub fn quantized_sample_matches(
    source_ns: i64,
    epoch_ns: i64,
    samples: u64,
    rate: u32,
    tick_ns: u64,
) -> bool {
    if rate == 0 || tick_ns == 0 {
        return false;
    }
    // Every product is bounded below 2^97 by the argument types.
    let rate = i128::from(rate);
    let quantum = i128::from(tick_ns) * rate;
    let expected = i128::from(samples) * 1_000_000_000;
    let lower = expected.div_euclid(quantum) * quantum;
    let upper = lower
        + if expected.rem_euclid(quantum) == 0 {
            0
        } else {
            quantum
        };
    let declared = (i128::from(source_ns) - i128::from(epoch_ns)) * rate;
    (declared - lower).abs() <= 2 * rate || (declared - upper).abs() <= 2 * rate
}

#[cfg(test)]
mod tests {
    use super::quantized_sample_matches as matches;
    #[test]
    fn floor_and_ceil_are_allowed_but_a_tick_at_an_exact_boundary_is_real() {
        assert!(matches(21_000_000, 0, 1024, 48000, 1_000_000));
        assert!(matches(22_000_000, 0, 1024, 48000, 1_000_000));
        assert!(!matches(20_000_000, 0, 1024, 48000, 1_000_000));
        assert!(matches(64_000_000, 0, 3072, 48000, 1_000_000));
        assert!(!matches(65_000_000, 0, 3072, 48000, 1_000_000));
        assert!(!matches(63_000_000, 0, 3072, 48000, 1_000_000));
    }
    #[test]
    fn delay_offsets_and_integer_nanosecond_rounding_do_not_change_the_bracket() {
        assert!(matches(20_645_833, -354_167, 1024, 48000, 1_000_000));
        assert!(matches(21_645_833, -354_167, 1024, 48000, 1_000_000));
        assert!(!matches(64_645_833, -354_167, 3072, 48000, 1_000_000));
        assert!(matches(42_666_666, 0, 2048, 48000, 1));
        assert!(matches(42_666_667, 0, 2048, 48000, 1));
        assert!(!matches(42_666_670, 0, 2048, 48000, 1));
    }
    #[test]
    fn extreme_coordinates_are_bounded_without_wrapping_or_panicking() {
        // The full-range declared displacement is the upper tick itself.
        assert!(matches(i64::MAX, i64::MIN, u64::MAX, u32::MAX, u64::MAX));
        assert!(!matches(i64::MAX, i64::MIN, u64::MAX, u32::MAX, 1));
        assert!(matches(i64::MIN, i64::MIN, 0, u32::MAX, u64::MAX));
        assert!(!matches(0, 0, 0, 0, 1));
        assert!(!matches(0, 0, 0, 1, 0));
    }
}
