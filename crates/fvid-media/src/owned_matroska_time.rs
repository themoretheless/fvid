//! Exact conversion of Matroska track ticks (RFC 9559 section 11.2).
use std::cmp::Ordering;

// A positive IEEE float is an integer mantissa times a power of two.
// Keep the product in two limbs: u64 ticks * 53 bits * u64 scale.
fn scaled(ticks: u64, scale: u64, factor: f64) -> Option<(u128, Ordering)> {
    if scale == 0 || !factor.is_finite() || factor <= 0.0 {
        return None;
    }
    let bits = factor.to_bits();
    let exponent = ((bits >> 52) & 2047) as i32;
    let mut mantissa = bits & ((1u64 << 52) - 1);
    let shift = if exponent == 0 {
        -1074
    } else {
        mantissa |= 1u64 << 52;
        exponent - 1023 - 52
    };
    let product = u128::from(ticks) * u128::from(mantissa);
    let low_product = u128::from(product as u64) * u128::from(scale);
    let high_product = (product >> 64) * u128::from(scale);
    let low = low_product.wrapping_add(high_product << 64);
    let high = (high_product >> 64) + u128::from(low < low_product);
    if shift >= 0 {
        let shift = shift as u32;
        if high != 0 {
            return None;
        }
        if low == 0 {
            return Some((0, Ordering::Less));
        }
        if shift >= 128 || low > (u128::MAX >> shift) {
            return None;
        }
        return Some((low << shift, Ordering::Less));
    }
    let shift = (-shift) as u32;
    if shift < 128 {
        if high >= (1u128 << shift) {
            return None;
        }
        let quotient = (high << (128 - shift)) | (low >> shift);
        let remainder = low & ((1u128 << shift) - 1);
        Some((quotient, remainder.cmp(&(1u128 << (shift - 1)))))
    } else if shift == 128 {
        Some((high, low.cmp(&(1u128 << 127))))
    } else if shift < 256 {
        let shift = shift - 128;
        let remainder = high & ((1u128 << shift) - 1);
        let mut comparison = remainder.cmp(&(1u128 << (shift - 1)));
        if comparison == Ordering::Equal && low != 0 {
            comparison = Ordering::Greater;
        }
        Some((high >> shift, comparison))
    } else {
        Some((0, Ordering::Less))
    }
}

/// Round the complete timestamp to nanoseconds, with half ties away from zero.
/// Cluster ticks use the segment scale; only relative block ticks use the track scale.
pub fn timestamp_ns(cluster: u64, relative: i16, scale: u64, factor: f64) -> Option<i64> {
    let (delta, remainder) = scaled(u64::from(relative.unsigned_abs()), scale, factor)?;
    let base = u128::from(cluster) * u128::from(scale);
    let (negative, magnitude) = if relative >= 0 {
        (
            false,
            base.checked_add(delta)?
                .checked_add(u128::from(remainder != Ordering::Less))?,
        )
    } else if base > delta {
        (
            false,
            (base - delta).checked_sub(u128::from(remainder == Ordering::Greater))?,
        )
    } else {
        (
            true,
            (delta - base).checked_add(u128::from(remainder != Ordering::Less))?,
        )
    };
    if negative {
        if magnitude == 1u128 << 63 {
            Some(i64::MIN)
        } else {
            i64::try_from(magnitude).ok().map(|value| -value)
        }
    } else {
        i64::try_from(magnitude).ok()
    }
}

/// BlockDuration is in track ticks. DefaultDuration and CodecDelay are already ns.
pub fn duration_ns(ticks: u64, scale: u64, factor: f64) -> Option<u64> {
    let (whole, remainder) = scaled(ticks, scale, factor)?;
    u64::try_from(whole.checked_add(u128::from(remainder != Ordering::Less))?).ok()
}
