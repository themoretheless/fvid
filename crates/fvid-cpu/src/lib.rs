//! Fast planar row horizontal flips (8-bit groups).
//!
//! On x86_64 with AVX2, uses byte-shuffle reverses; otherwise falls back to
//! scalar pairwise swaps. Safe API; unsafe is confined to SIMD loads/stores.

/// Reverse `width` pixels of `step` bytes each in `row` (in-place).
pub fn hflip_row(row: &mut [u8], width: usize, step: usize) {
    if width == 0 || step == 0 {
        return;
    }
    let bytes = width.saturating_mul(step);
    let row = match row.get_mut(..bytes) {
        Some(r) => r,
        None => return,
    };
    if step == 1 {
        hflip_bytes(row);
        return;
    }
    let mut i = 0;
    let mut j = width - 1;
    while i < j {
        let a = i * step;
        let b = j * step;
        for k in 0..step {
            row.swap(a + k, b + k);
        }
        i += 1;
        j -= 1;
    }
}

/// Reverse `dst` from `src` (same logical width×step). Prefer out-of-place staging.
pub fn hflip_row_copy(dst: &mut [u8], src: &[u8], width: usize, step: usize) {
    if width == 0 || step == 0 {
        return;
    }
    let bytes = width * step;
    let src_len = src.len();
    let dst_len = dst.len();
    let src = &src[..bytes.min(src_len)];
    let dst = &mut dst[..bytes.min(dst_len)];
    if step == 1 {
        hflip_bytes_copy(dst, src);
        return;
    }
    for i in 0..width {
        let s = (width - 1 - i) * step;
        let d = i * step;
        dst[d..d + step].copy_from_slice(&src[s..s + step]);
    }
}

/// Reverse-copy a byte-planar image while preserving row order.
///
/// # Safety
/// Every row addressed by `height`, the signed strides, and `width` must be
/// readable from `src` and writable to `dst`; the regions must not overlap.
pub unsafe fn hflip_plane_copy(
    dst: *mut u8,
    dst_stride: isize,
    src: *const u8,
    src_stride: isize,
    width: usize,
    height: usize,
) {
    if width == 0 || height == 0 {
        return;
    }
    #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
    if has_avx2() {
        // SAFETY: AVX2 was detected and the caller guarantees all plane rows.
        unsafe {
            hflip_plane_copy_avx2(dst, dst_stride, src, src_stride, width, height);
        }
        return;
    }
    for row in 0..height {
        // SAFETY: Guaranteed by this function's caller.
        unsafe {
            let source = std::slice::from_raw_parts(src.offset(row as isize * src_stride), width);
            let target =
                std::slice::from_raw_parts_mut(dst.offset(row as isize * dst_stride), width);
            for (output, input) in target.iter_mut().zip(source.iter().rev()) {
                *output = *input;
            }
        }
    }
}

fn hflip_bytes(row: &mut [u8]) {
    #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
    {
        if has_avx2() {
            // SAFETY: AVX2 detected; `row` is a valid mutable byte slice.
            unsafe {
                hflip_bytes_avx2(row);
            }
            return;
        }
    }
    row.reverse();
}

fn hflip_bytes_copy(dst: &mut [u8], src: &[u8]) {
    let n = dst.len().min(src.len());
    let dst = &mut dst[..n];
    let src = &src[..n];
    #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
    {
        if has_avx2() {
            // SAFETY: AVX2 detected; equal-length slices.
            unsafe {
                hflip_bytes_copy_avx2(dst, src);
            }
            return;
        }
    }
    for (i, b) in src.iter().rev().enumerate() {
        dst[i] = *b;
    }
}

#[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
fn has_avx2() -> bool {
    use std::sync::OnceLock;
    static AVX2: OnceLock<bool> = OnceLock::new();
    *AVX2.get_or_init(|| is_x86_feature_detected!("avx2"))
}

#[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
#[target_feature(enable = "avx2")]
unsafe fn hflip_bytes_avx2(row: &mut [u8]) {
    #[cfg(target_arch = "x86")]
    use std::arch::x86::*;
    #[cfg(target_arch = "x86_64")]
    use std::arch::x86_64::*;

    // SAFETY: caller enabled AVX2; pointers stay in-bounds via left/right math.
    unsafe {
        let shuffle = _mm256_setr_epi8(
            15, 14, 13, 12, 11, 10, 9, 8, 7, 6, 5, 4, 3, 2, 1, 0, 15, 14, 13, 12, 11, 10, 9, 8, 7,
            6, 5, 4, 3, 2, 1, 0,
        );
        let ptr = row.as_mut_ptr();
        let len = row.len();
        let mut left = 0usize;
        let mut right = len;
        while right - left >= 64 {
            right -= 32;
            let a = _mm256_loadu_si256(ptr.add(left) as *const __m256i);
            let b = _mm256_loadu_si256(ptr.add(right) as *const __m256i);
            let a_rev = _mm256_shuffle_epi8(a, shuffle);
            let b_rev = _mm256_shuffle_epi8(b, shuffle);
            let a_sw = _mm256_permute2x128_si256(a_rev, a_rev, 0x01);
            let b_sw = _mm256_permute2x128_si256(b_rev, b_rev, 0x01);
            _mm256_storeu_si256(ptr.add(left) as *mut __m256i, b_sw);
            _mm256_storeu_si256(ptr.add(right) as *mut __m256i, a_sw);
            left += 32;
        }
        while right - left >= 2 {
            right -= 1;
            let tmp = *ptr.add(left);
            *ptr.add(left) = *ptr.add(right);
            *ptr.add(right) = tmp;
            left += 1;
        }
    }
}

#[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
#[target_feature(enable = "avx2")]
unsafe fn hflip_bytes_copy_avx2(dst: &mut [u8], src: &[u8]) {
    #[cfg(target_arch = "x86")]
    use std::arch::x86::*;
    #[cfg(target_arch = "x86_64")]
    use std::arch::x86_64::*;

    // SAFETY: caller enabled AVX2; `dst`/`src` share length `n`.
    unsafe {
        let shuffle = _mm256_setr_epi8(
            15, 14, 13, 12, 11, 10, 9, 8, 7, 6, 5, 4, 3, 2, 1, 0, 15, 14, 13, 12, 11, 10, 9, 8, 7,
            6, 5, 4, 3, 2, 1, 0,
        );
        let n = dst.len();
        let d = dst.as_mut_ptr();
        let s = src.as_ptr();
        let mut i = 0usize;
        while i + 128 <= n {
            for block in 0..4 {
                let output = i + block * 32;
                let src_off = n - 32 - output;
                let v = _mm256_loadu_si256(s.add(src_off) as *const __m256i);
                let rev = _mm256_shuffle_epi8(v, shuffle);
                let sw = _mm256_permute2x128_si256(rev, rev, 0x01);
                _mm256_storeu_si256(d.add(output) as *mut __m256i, sw);
            }
            i += 128;
        }
        while i + 32 <= n {
            let src_off = n - 32 - i;
            let v = _mm256_loadu_si256(s.add(src_off) as *const __m256i);
            let rev = _mm256_shuffle_epi8(v, shuffle);
            let sw = _mm256_permute2x128_si256(rev, rev, 0x01);
            _mm256_storeu_si256(d.add(i) as *mut __m256i, sw);
            i += 32;
        }
        while i < n {
            *d.add(i) = *s.add(n - 1 - i);
            i += 1;
        }
    }
}

#[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
#[target_feature(enable = "avx2")]
unsafe fn hflip_plane_copy_avx2(
    dst: *mut u8,
    dst_stride: isize,
    src: *const u8,
    src_stride: isize,
    width: usize,
    height: usize,
) {
    for row in 0..height {
        // SAFETY: Caller guarantees plane extents; this function requires AVX2.
        unsafe {
            let source = std::slice::from_raw_parts(src.offset(row as isize * src_stride), width);
            let target =
                std::slice::from_raw_parts_mut(dst.offset(row as isize * dst_stride), width);
            hflip_bytes_copy_avx2(target, source);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reverse_bytes_matches_std() {
        let mut a: Vec<u8> = (0..100).collect();
        let mut b = a.clone();
        hflip_bytes(&mut a);
        b.reverse();
        assert_eq!(a, b);
    }

    #[test]
    fn reverse_copy_matches() {
        let src: Vec<u8> = (0..97).collect();
        let mut dst = vec![0u8; 97];
        hflip_bytes_copy(&mut dst, &src);
        let mut expect = src.clone();
        expect.reverse();
        assert_eq!(dst, expect);
    }

    #[test]
    fn step2_swap() {
        let mut row = vec![1u8, 2, 3, 4, 5, 6];
        hflip_row(&mut row, 3, 2);
        assert_eq!(row, vec![5, 6, 3, 4, 1, 2]);
    }

    #[test]
    fn plane_copy_preserves_rows_and_padding() {
        let src = [1u8, 2, 3, 4, 99, 5, 6, 7, 8, 99];
        let mut dst = [77u8; 10];
        unsafe { hflip_plane_copy(dst.as_mut_ptr(), 5, src.as_ptr(), 5, 4, 2) };
        assert_eq!(dst, [4, 3, 2, 1, 77, 8, 7, 6, 5, 77]);
    }
}
