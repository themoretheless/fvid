//! Checked byte sizes for tightly packed, independently allocated image planes.

/// Calculate planar storage, rounding each chroma dimension upwards.
pub fn planar_bytes(
    width: usize,
    height: usize,
    chroma_shift: Option<(u32, u32)>,
    bytes_per_sample: usize,
) -> Result<usize, String> {
    if width == 0 || height == 0 || bytes_per_sample == 0 {
        return Err("invalid frame geometry for memory estimate".into());
    }
    let overflow = || "frame buffer size overflow".to_string();
    let mut samples = width.checked_mul(height).ok_or_else(overflow)?;
    if let Some((horizontal, vertical)) = chroma_shift {
        let divisor_x = 1usize.checked_shl(horizontal).ok_or_else(overflow)?;
        let divisor_y = 1usize.checked_shl(vertical).ok_or_else(overflow)?;
        let chroma = width
            .div_ceil(divisor_x)
            .checked_mul(height.div_ceil(divisor_y))
            .ok_or_else(overflow)?;
        samples = samples
            .checked_add(chroma.checked_mul(2).ok_or_else(overflow)?)
            .ok_or_else(overflow)?;
    }
    samples.checked_mul(bytes_per_sample).ok_or_else(overflow)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn independently_rounded_chroma_planes() {
        assert_eq!(planar_bytes(3, 3, Some((1, 1)), 1).unwrap(), 17);
        assert_eq!(planar_bytes(3, 3, Some((1, 0)), 1).unwrap(), 21);
        assert_eq!(planar_bytes(3, 3, Some((0, 0)), 1).unwrap(), 27);
        assert_eq!(planar_bytes(3, 3, Some((1, 1)), 2).unwrap(), 34);
        assert_eq!(planar_bytes(3, 3, None, 1).unwrap(), 9);
        assert_eq!(planar_bytes(3, 3, None, 4).unwrap(), 36);
    }
    #[test]
    fn rejects_empty_and_overflowing_storage() {
        for args in [
            (0, 1, None, 1),
            (1, 0, None, 1),
            (1, 1, None, 0),
            (usize::MAX, 2, None, 1),
            (usize::MAX, 1, Some((0, 0)), 1),
            (1, 1, Some((usize::BITS, 0)), 1),
        ] {
            assert!(planar_bytes(args.0, args.1, args.2, args.3).is_err());
        }
    }
}
