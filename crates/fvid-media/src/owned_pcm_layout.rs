//! Bit-preserving planar-to-packed layouts; no floating-point arithmetic.
use std::ptr;

/// Pack equal-length planar samples without changing their representation.
/// All geometry is validated before output is touched.
pub fn pack(planes: &[&[u8]], output: &mut [u8], sample_bytes: usize) -> Result<(), String> {
    if planes.is_empty() || planes.len() > 64 || !matches!(sample_bytes, 1 | 2 | 3 | 4 | 8) {
        return Err("invalid PCM packing geometry".into());
    }
    let length = planes[0].len();
    if length % sample_bytes != 0 || planes.iter().any(|plane| plane.len() != length) {
        return Err("inconsistent PCM plane lengths".into());
    }
    let expected = length
        .checked_mul(planes.len())
        .ok_or("PCM packing size overflow")?;
    if output.len() != expected {
        return Err("incorrect packed PCM output size".into());
    }
    let pointers: Vec<_> = planes.iter().map(|plane| plane.as_ptr()).collect();
    // SAFETY: Slice extents and equal plane geometry were checked above.
    // Exclusive output borrowing prevents safe callers from overlapping input.
    unsafe {
        interleave(
            &pointers,
            output.as_mut_ptr(),
            length / sample_bytes,
            sample_bytes,
        );
    }
    Ok(())
}

/// # Safety
/// Every plane holds count * bytes readable bytes, destination holds
/// count * planes.len() * bytes writable bytes, and no source overlaps output.
pub unsafe fn interleave(planes: &[*const u8], output: *mut u8, count: usize, bytes: usize) {
    // SAFETY: All byte ranges follow the caller's checked block extents.
    unsafe {
        if planes.len() == 2 {
            match bytes {
                1 => stereo::<1>(planes, output, count),
                2 => stereo::<2>(planes, output, count),
                4 => stereo::<4>(planes, output, count),
                8 => stereo::<8>(planes, output, count),
                _ => generic(planes, output, count, bytes),
            }
        } else {
            generic(planes, output, count, bytes);
        }
    }
}
unsafe fn stereo<const B: usize>(planes: &[*const u8], output: *mut u8, count: usize) {
    let mut sample = 0;
    // SAFETY: NEON loads/stores support unaligned addresses. Four 32-bit groups
    // from each channel become eight packed groups, preserving NaNs and all bits.
    #[cfg(target_arch = "aarch64")]
    if B == 4 {
        use std::arch::aarch64::{uint32x4x2_t, vld1q_u32, vst2q_u32};
        unsafe {
            while sample + 4 <= count {
                let left = vld1q_u32(planes[0].add(sample * B).cast());
                let right = vld1q_u32(planes[1].add(sample * B).cast());
                vst2q_u32(output.add(sample * 2 * B).cast(), uint32x4x2_t(left, right));
                sample += 4;
            }
        }
    }
    // SAFETY: B is a compile-time width; remaining source/output ranges are disjoint.
    unsafe {
        while sample < count {
            ptr::copy_nonoverlapping(planes[0].add(sample * B), output.add(sample * 2 * B), B);
            ptr::copy_nonoverlapping(
                planes[1].add(sample * B),
                output.add((sample * 2 + 1) * B),
                B,
            );
            sample += 1;
        }
    }
}
unsafe fn generic(planes: &[*const u8], output: *mut u8, count: usize, bytes: usize) {
    // SAFETY: Caller supplied the checked readable and writable extents. Writing in
    // sample-major order makes output contiguous even for multichannel layouts.
    unsafe {
        for sample in 0..count {
            for (channel, &plane) in planes.iter().enumerate() {
                ptr::copy_nonoverlapping(
                    plane.add(sample * bytes),
                    output.add((sample * planes.len() + channel) * bytes),
                    bytes,
                );
            }
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn interleave_preserves_bits_unaligned_tails_and_guards() {
        for channels in [1, 2, 3, 6] {
            for bytes in [1, 2, 3, 4, 8] {
                for count in [0, 1, 2, 3, 4, 5, 15, 16, 17, 255, 256, 259] {
                    let storage: Vec<Vec<u8>> = (0..channels)
                        .map(|c| {
                            (0..count * bytes + 2)
                                .map(|i| ((i * 197 + c * 59) % 256) as u8)
                                .collect()
                        })
                        .collect();
                    let pointers: Vec<_> = storage.iter().map(|p| p[1..].as_ptr()).collect();
                    let size = channels * bytes * count;
                    let mut output = vec![0xa5; size + 2];
                    // SAFETY: Test intentionally offsets all valid arrays by one byte.
                    unsafe {
                        interleave(&pointers, output[1..].as_mut_ptr(), count, bytes);
                    }
                    let mut expected = Vec::new();
                    for sample in 0..count {
                        for plane in &storage {
                            expected.extend_from_slice(
                                &plane[1 + sample * bytes..1 + (sample + 1) * bytes],
                            );
                        }
                    }
                    assert_eq!(&output[1..size + 1], expected);
                    assert_eq!((output[0], output[size + 1]), (0xa5, 0xa5));
                }
            }
        }
    }
}

#[cfg(test)]
mod checked_tests {
    use super::pack;
    #[test]
    fn geometry_errors_preserve_output_and_24_bit_packing_is_exact() {
        let mut output = [0xa5; 12];
        for (planes, bytes) in [
            (vec![], 3),
            (vec![&[1u8, 2][..]], 3),
            (vec![&[0u8; 6][..], &[0u8; 3][..]], 3),
            (vec![&[0u8; 6][..]], 3),
        ] {
            assert!(pack(&planes, &mut output, bytes).is_err());
            assert_eq!(output, [0xa5; 12]);
        }
        pack(
            &[&[1, 2, 3, 4, 5, 6], &[7, 8, 9, 10, 11, 12]],
            &mut output,
            3,
        )
        .unwrap();
        assert_eq!(output, [1, 2, 3, 7, 8, 9, 4, 5, 6, 10, 11, 12]);
    }
}
