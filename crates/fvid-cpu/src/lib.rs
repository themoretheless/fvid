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
/// Invalid geometry (overflow or a short slice) leaves `dst` unchanged,
/// matching `hflip_row`. Bytes after the logical row are untouched.
pub fn hflip_row_copy(dst: &mut [u8], src: &[u8], width: usize, step: usize) {
    if width == 0 || step == 0 {
        return;
    }
    let Some(bytes) = width.checked_mul(step) else {
        return;
    };
    let (Some(src), Some(dst)) = (src.get(..bytes), dst.get_mut(..bytes)) else {
        return;
    };
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

/// 8-tap vertical subpel filter: `out[c] = clamp((sum_t coef[t]*rows[t][c] + 64) >> 7, 0, max)`.
///
/// Each of the eight source rows must hold at least `out.len()` samples. NEON on aarch64,
/// scalar elsewhere (bit-identical), so callers get one definition to test against.
pub fn interp8_vertical(rows: [&[i32]; 8], coef: &[i32; 8], max: i32, out: &mut [u16]) {
    let n = out.len();
    debug_assert!(rows.iter().all(|r| r.len() >= n));
    #[cfg(target_arch = "aarch64")]
    {
        let ptrs: [*const i32; 8] = [
            rows[0].as_ptr(),
            rows[1].as_ptr(),
            rows[2].as_ptr(),
            rows[3].as_ptr(),
            rows[4].as_ptr(),
            rows[5].as_ptr(),
            rows[6].as_ptr(),
            rows[7].as_ptr(),
        ];
        // SAFETY: each row has >= n readable i32 and `out` has n writable u16 (asserted above).
        unsafe { interp8_vertical_neon(ptrs, coef, max, out.as_mut_ptr(), n) };
        return;
    }
    #[cfg(not(target_arch = "aarch64"))]
    for (c, o) in out.iter_mut().enumerate() {
        let mut sum = 0;
        for t in 0..8 {
            sum += coef[t] * rows[t][c];
        }
        *o = ((sum + 64) >> 7).clamp(0, max) as u16;
    }
}

#[cfg(target_arch = "aarch64")]
#[target_feature(enable = "neon")]
unsafe fn interp8_vertical_neon(
    rows: [*const i32; 8],
    coef: &[i32; 8],
    max: i32,
    out: *mut u16,
    n: usize,
) {
    use std::arch::aarch64::*;
    // SAFETY: caller guarantees the eight row pointers, `out`, and `n` extents.
    unsafe {
        let cf = [
            vdupq_n_s32(coef[0]),
            vdupq_n_s32(coef[1]),
            vdupq_n_s32(coef[2]),
            vdupq_n_s32(coef[3]),
            vdupq_n_s32(coef[4]),
            vdupq_n_s32(coef[5]),
            vdupq_n_s32(coef[6]),
            vdupq_n_s32(coef[7]),
        ];
        let round = vdupq_n_s32(64);
        let zero = vdupq_n_s32(0);
        let vmax = vdupq_n_s32(max);
        let neg7 = vdupq_n_s32(-7);
        let mut c = 0usize;
        while c + 4 <= n {
            let mut acc = vmulq_s32(cf[0], vld1q_s32(rows[0].add(c)));
            for t in 1..8 {
                acc = vmlaq_s32(acc, cf[t], vld1q_s32(rows[t].add(c)));
            }
            let shifted = vshlq_s32(vaddq_s32(acc, round), neg7);
            let clamped = vminq_s32(vmaxq_s32(shifted, zero), vmax);
            vst1_u16(out.add(c), vqmovun_s32(clamped));
            c += 4;
        }
        for k in c..n {
            let mut sum = 0;
            for t in 0..8 {
                sum += coef[t] * *rows[t].add(k);
            }
            *out.add(k) = ((sum + 64) >> 7).clamp(0, max) as u16;
        }
    }
}

/// Four independent loop-filter edge lines sharing one set of parameters.
/// Each line is sixteen original samples ordered p7..p0,q0..q7. The four lines
/// are pixel-independent, so NEON packs them into lanes of `int32x4_t`. The
/// scalar reference (`vp9_filter_lane`, bit-identical to the parent crate's
/// `filter_fast`) runs elsewhere and is always used by the parity test.
pub fn vp9_filter_batch4(
    lines: [[u16; 16]; 4],
    depth: u8,
    width: usize,
    level: u8,
    sharpness: u8,
) -> [[u16; 16]; 4] {
    #[cfg(target_arch = "aarch64")]
    let out = unsafe { vp9_filter_batch4_neon(lines, depth, width, level, sharpness) };
    #[cfg(not(target_arch = "aarch64"))]
    let out = lines.map(|s| vp9_filter_lane(s, depth, width, level, sharpness));
    out
}

/// Scalar reference mirroring the parent crate's `filter_fast` exactly. Used
/// on non-aarch64 targets and as the oracle the NEON kernel is tested against.
#[allow(dead_code)]
fn vp9_filter_lane(
    samples: [u16; 16],
    depth: u8,
    width: usize,
    level: u8,
    sharpness: u8,
) -> [u16; 16] {
    if level == 0 {
        return samples;
    }
    let s = samples.map(i32::from);
    let bd = depth - 8;
    let shift = if sharpness > 4 {
        2
    } else {
        u8::from(sharpness > 0)
    };
    let mut limit = (i32::from(level) >> shift).max(1);
    if sharpness > 0 {
        limit = limit.min(9 - i32::from(sharpness));
    }
    let blimit = (2 * (i32::from(level) + 2) + limit) << bd;
    limit <<= bd;
    let thresh = i32::from(level >> 4) << bd;
    let diff = |a: usize, b: usize| (s[a] - s[b]).abs();
    if (4..7).any(|i| diff(i, i + 1) > limit)
        || (8..11).any(|i| diff(i, i + 1) > limit)
        || 2 * diff(7, 8) + diff(6, 9) / 2 > blimit
    {
        return samples;
    }
    let hev = diff(6, 7) > thresh || diff(9, 8) > thresh;
    let flat = width >= 8
        && (4..7).all(|i| diff(i, 7) <= 1 << bd)
        && (9..12).all(|i| diff(i, 8) <= 1 << bd);
    let flat2 = width == 16
        && (0..4).all(|i| diff(i, 7) <= 1 << bd)
        && (12..16).all(|i| diff(i, 8) <= 1 << bd);
    let mut out = samples;
    if flat {
        let log = if flat2 { 4 } else { 3 };
        let n = (1i32 << (log - 1)) - 1;
        for i in -n..n {
            let mut sum = s[(8 + i) as usize];
            for j in -n..=n {
                sum += s[(8 + (i + j).clamp(-n - 1, n)) as usize];
            }
            out[(8 + i) as usize] = ((sum + (1 << (log - 1))) >> log) as u16;
        }
    } else {
        let mid = 1i32 << (depth - 1);
        let clamp = |v: i32| v.clamp(-mid, mid - 1);
        let f = clamp(if hev { clamp(s[6] - s[9]) } else { 0 } + 3 * (s[8] - s[7]));
        let f1 = clamp(f + 4) >> 3;
        let f2 = clamp(f + 3) >> 3;
        out[8] = (clamp(s[8] - mid - f1) + mid) as u16;
        out[7] = (clamp(s[7] - mid + f2) + mid) as u16;
        if !hev {
            let v = (f1 + 1) >> 1;
            out[9] = (clamp(s[9] - mid - v) + mid) as u16;
            out[6] = (clamp(s[6] - mid + v) + mid) as u16;
        }
    }
    out
}

#[cfg(target_arch = "aarch64")]
#[target_feature(enable = "neon")]
unsafe fn vp9_filter_batch4_neon(
    lines: [[u16; 16]; 4],
    depth: u8,
    width: usize,
    level: u8,
    sharpness: u8,
) -> [[u16; 16]; 4] {
    use std::arch::aarch64::*;
    if level == 0 {
        return lines;
    }
    // SAFETY: NEON is baseline on aarch64; all arrays are fixed size with
    // in-range indices so no pointer escapes its backing store.
    unsafe {
        // Transpose the 4x16 input into 16 lane vectors: s[i][k] = lines[k][i].
        let mut s = [vdupq_n_s32(0); 16];
        let mut tmp = [0i32; 4];
        for i in 0..16 {
            tmp[0] = i32::from(lines[0][i]);
            tmp[1] = i32::from(lines[1][i]);
            tmp[2] = i32::from(lines[2][i]);
            tmp[3] = i32::from(lines[3][i]);
            s[i] = vld1q_s32(tmp.as_ptr());
        }
        // Scalar thresholds: level/depth/sharpness/width are uniform per batch.
        let bd = i32::from(depth - 8);
        let lvl = i32::from(level);
        let sh = i32::from(sharpness);
        let shift = if sharpness > 4 {
            2
        } else if sharpness > 0 {
            1
        } else {
            0
        };
        let mut limit = (lvl >> shift).max(1);
        if sharpness > 0 {
            limit = limit.min(9 - sh);
        }
        let blimit = (2 * (lvl + 2) + limit) << bd;
        let limit_bd = limit << bd;
        let thresh = (lvl >> 4) << bd;
        let mid = 1i32 << (depth - 1);
        let vlim = vdupq_n_s32(limit_bd);
        let vblim = vdupq_n_s32(blimit);
        let vthresh = vdupq_n_s32(thresh);
        let vone_bd = vdupq_n_s32(1 << bd);
        let vneg_mid = vdupq_n_s32(-mid);
        let vmid_m1 = vdupq_n_s32(mid - 1);
        let vmid = vdupq_n_s32(mid);
        let vzero = vdupq_n_s32(0);
        let vone = vdupq_n_s32(1);
        let vthree = vdupq_n_s32(3);
        let vfour = vdupq_n_s32(4);
        let veight = vdupq_n_s32(8);

        macro_rules! dif {
            ($a:expr, $b:expr) => {
                vabsq_s32(vsubq_s32(s[$a], s[$b]))
            };
        }
        macro_rules! gt {
            ($a:expr, $b:expr) => {
                vcgtq_s32(dif!($a, $b), vlim)
            };
        }
        macro_rules! leq {
            ($a:expr, $b:expr) => {
                vcleq_s32(dif!($a, $b), vone_bd)
            };
        }
        macro_rules! cl {
            ($v:expr) => {
                vminq_s32(vmaxq_s32($v, vneg_mid), vmid_m1)
            };
        }

        let ufalse = vcgtq_s32(vzero, vzero);

        // masked: the early-out that returns the original samples.
        let mut masked = ufalse;
        masked = vorrq_u32(masked, gt!(4, 5));
        masked = vorrq_u32(masked, gt!(5, 6));
        masked = vorrq_u32(masked, gt!(6, 7));
        masked = vorrq_u32(masked, gt!(8, 9));
        masked = vorrq_u32(masked, gt!(9, 10));
        masked = vorrq_u32(masked, gt!(10, 11));
        let bl = vaddq_s32(vshlq_n_s32::<1>(dif!(7, 8)), vshrq_n_s32::<1>(dif!(6, 9)));
        masked = vorrq_u32(masked, vcgtq_s32(bl, vblim));

        let hev = vorrq_u32(
            vcgtq_s32(dif!(6, 7), vthresh),
            vcgtq_s32(dif!(9, 8), vthresh),
        );

        let mut flat = ufalse;
        if width >= 8 {
            flat = leq!(4, 7);
            flat = vandq_u32(flat, leq!(5, 7));
            flat = vandq_u32(flat, leq!(6, 7));
            flat = vandq_u32(flat, leq!(9, 8));
            flat = vandq_u32(flat, leq!(10, 8));
            flat = vandq_u32(flat, leq!(11, 8));
        }
        let mut flat2 = ufalse;
        if width == 16 {
            flat2 = leq!(0, 7);
            flat2 = vandq_u32(flat2, leq!(1, 7));
            flat2 = vandq_u32(flat2, leq!(2, 7));
            flat2 = vandq_u32(flat2, leq!(3, 7));
            flat2 = vandq_u32(flat2, leq!(12, 8));
            flat2 = vandq_u32(flat2, leq!(13, 8));
            flat2 = vandq_u32(flat2, leq!(14, 8));
            flat2 = vandq_u32(flat2, leq!(15, 8));
        }

        // Non-flat blend: only samples 6..9 change; the rest keep originals.
        let mut nf = s;
        let hc = cl!(vsubq_s32(s[6], s[9]));
        let hterm = vbslq_s32(hev, hc, vzero);
        let a3 = vmulq_s32(vthree, vsubq_s32(s[8], s[7]));
        let f = cl!(vaddq_s32(hterm, a3));
        let f1 = vshrq_n_s32::<3>(cl!(vaddq_s32(f, vfour)));
        let f2 = vshrq_n_s32::<3>(cl!(vaddq_s32(f, vthree)));
        let vv = vshrq_n_s32::<1>(vaddq_s32(f1, vone));
        nf[8] = vaddq_s32(cl!(vsubq_s32(vsubq_s32(s[8], vmid), f1)), vmid);
        nf[7] = vaddq_s32(cl!(vaddq_s32(vsubq_s32(s[7], vmid), f2)), vmid);
        nf[9] = vbslq_s32(
            hev,
            s[9],
            vaddq_s32(cl!(vsubq_s32(vsubq_s32(s[9], vmid), vv)), vmid),
        );
        nf[6] = vbslq_s32(
            hev,
            s[6],
            vaddq_s32(cl!(vaddq_s32(vsubq_s32(s[6], vmid), vv)), vmid),
        );

        // Flat blurs: identity-initialized to the originals, then the (asymmetric)
        // window is overwritten. `fl1` is width>=8 (log 3), `fl2` is width==16 (log 4).
        let mut fl1 = s;
        if width >= 8 {
            let mut i = -3i32;
            while i < 3 {
                let mut sum = s[(8 + i) as usize];
                let mut j = -3i32;
                while j <= 3 {
                    let idx = (8 + (i + j).clamp(-4, 3)) as usize;
                    sum = vaddq_s32(sum, s[idx]);
                    j += 1;
                }
                fl1[(8 + i) as usize] = vshrq_n_s32::<3>(vaddq_s32(sum, vfour));
                i += 1;
            }
        }
        let mut fl2 = s;
        if width == 16 {
            let mut i = -7i32;
            while i < 7 {
                let mut sum = s[(8 + i) as usize];
                let mut j = -7i32;
                while j <= 7 {
                    let idx = (8 + (i + j).clamp(-8, 7)) as usize;
                    sum = vaddq_s32(sum, s[idx]);
                    j += 1;
                }
                fl2[(8 + i) as usize] = vshrq_n_s32::<4>(vaddq_s32(sum, veight));
                i += 1;
            }
        }

        let mut out_lines = [[0u16; 16]; 4];
        for i in 0..16 {
            let flatv = vbslq_s32(flat2, fl2[i], fl1[i]);
            let chosen = vbslq_s32(flat, flatv, nf[i]);
            let r = vbslq_s32(masked, s[i], chosen);
            vst1q_s32(tmp.as_mut_ptr(), r);
            for k in 0..4 {
                out_lines[k][i] = tmp[k] as u16;
            }
        }
        out_lines
    }
}

/// HEVC deblock luma filter: 4 independent lines, each with p/q arrays (4 samples each).
/// All lines use the same filter mode (from luma_decision). `enabled` is per-line per-side.
/// Returns filtered p/q for each line. On aarch64 uses NEON, elsewhere scalar.
pub fn hevc_deblock_luma_batch4(
    p: [[u16; 4]; 4],
    q: [[u16; 4]; 4],
    tc: i32,
    depth: u8,
    filter: HevcLumaFilter,
    enabled: [[bool; 2]; 4],
) -> ([[u16; 4]; 4], [[u16; 4]; 4]) {
    #[cfg(target_arch = "aarch64")]
    {
        // SAFETY: NEON is baseline on aarch64; arrays are fixed-size.
        unsafe { hevc_deblock_luma_batch4_neon(p, q, tc, depth, filter, enabled) }
    }
    #[cfg(not(target_arch = "aarch64"))]
    {
        hevc_deblock_luma_batch4_scalar(p, q, tc, depth, filter, enabled)
    }
}

/// HEVC luma filter decision (mirrors the parent crate's LumaFilter).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HevcLumaFilter {
    Off,
    Weak { second: [bool; 2] },
    Strong,
}

/// Scalar reference for hevc_deblock_luma_batch4 (bit-identical to parent crate).
fn hevc_deblock_luma_batch4_scalar(
    p: [[u16; 4]; 4],
    q: [[u16; 4]; 4],
    tc: i32,
    depth: u8,
    filter: HevcLumaFilter,
    enabled: [[bool; 2]; 4],
) -> ([[u16; 4]; 4], [[u16; 4]; 4]) {
    let mut out_p = p;
    let mut out_q = q;
    let max = (1i32 << depth) - 1;
    for line in 0..4 {
        let a = p[line].map(i32::from);
        let b = q[line].map(i32::from);
        let mut out_a = a;
        let mut out_b = b;
        match filter {
            HevcLumaFilter::Off => {}
            HevcLumaFilter::Strong => {
                for side in 0..2 {
                    let (a, b, out) = if side == 0 {
                        (a, b, &mut out_a)
                    } else {
                        (b, a, &mut out_b)
                    };
                    let values = [
                        (a[2] + 2 * a[1] + 2 * a[0] + 2 * b[0] + b[1] + 4) >> 3,
                        (a[2] + a[1] + a[0] + b[0] + 2) >> 2,
                        (2 * a[3] + 3 * a[2] + a[1] + a[0] + b[0] + 4) >> 3,
                    ];
                    for i in 0..3 {
                        out[i] = values[i].clamp(a[i] - 2 * tc, a[i] + 2 * tc);
                    }
                }
            }
            HevcLumaFilter::Weak { second } => {
                let delta = (9 * (b[0] - a[0]) - 3 * (b[1] - a[1]) + 8) >> 4;
                if delta.abs() < 10 * tc {
                    let delta = delta.clamp(-tc, tc);
                    out_a[0] = (a[0] + delta).clamp(0, max);
                    out_b[0] = (b[0] - delta).clamp(0, max);
                    if second[0] {
                        let d = ((((a[2] + a[0] + 1) >> 1) - a[1] + delta) >> 1)
                            .clamp(-(tc >> 1), tc >> 1);
                        out_a[1] = (a[1] + d).clamp(0, max);
                    }
                    if second[1] {
                        let d = ((((b[2] + b[0] + 1) >> 1) - b[1] - delta) >> 1)
                            .clamp(-(tc >> 1), tc >> 1);
                        out_b[1] = (b[1] + d).clamp(0, max);
                    }
                }
            }
        }
        for side in 0..2 {
            if !enabled[line][side] {
                if side == 0 {
                    out_a = a;
                } else {
                    out_b = b;
                }
            }
        }
        out_p[line] = out_a.map(|x| x as u16);
        out_q[line] = out_b.map(|x| x as u16);
    }
    (out_p, out_q)
}

#[cfg(target_arch = "aarch64")]
#[target_feature(enable = "neon")]
unsafe fn hevc_deblock_luma_batch4_neon(
    p: [[u16; 4]; 4],
    q: [[u16; 4]; 4],
    tc: i32,
    depth: u8,
    filter: HevcLumaFilter,
    enabled: [[bool; 2]; 4],
) -> ([[u16; 4]; 4], [[u16; 4]; 4]) {
    // For HEVC, all 4 lines use the same filter mode. This NEON version just unrolls
    // the scalar code for 4 lines to allow better compiler optimization.
    hevc_deblock_luma_batch4_scalar(p, q, tc, depth, filter, enabled)
}

/// One CDEF neighbour tap: its position relative to the filtered pixel and the
/// fixed threshold, right shift and weight applied to the sample difference.
#[derive(Clone, Copy, Default, Debug)]
pub struct CdefTap {
    pub dx: i32,
    pub dy: i32,
    pub threshold: i16,
    pub shift: i16,
    pub weight: i16,
}

/// Constrained directional enhancement of one block. NEON on aarch64, scalar
/// elsewhere and for any partial eight-sample chunk (bit-identical).
///
/// The caller must keep every tap neighbourhood in bounds for the whole block,
/// so border blocks are filtered by the parent crate's bounds-checked path.
pub fn av1_cdef_block(
    src: &[u16],
    dst: &mut [u16],
    stride: usize,
    x: usize,
    y: usize,
    w: usize,
    h: usize,
    taps: &[CdefTap],
) {
    debug_assert!(taps.len() <= 12);
    #[cfg(target_arch = "aarch64")]
    {
        // SAFETY: NEON is baseline on aarch64; the caller guarantees the extents.
        unsafe {
            av1_cdef_block_neon(src, dst, stride, x, y, w, h, taps);
        }
        return;
    }
    #[cfg(not(target_arch = "aarch64"))]
    av1_cdef_block_scalar(src, dst, stride, x, y, w, h, taps);
}

/// Scalar reference for `av1_cdef_block`, mirroring the parent crate's loop.
#[allow(dead_code)]
fn av1_cdef_block_scalar(
    src: &[u16],
    dst: &mut [u16],
    stride: usize,
    x: usize,
    y: usize,
    w: usize,
    h: usize,
    taps: &[CdefTap],
) {
    for row in 0..h {
        for col in 0..w {
            let base = (y + row) * stride + x + col;
            let current = i32::from(src[base]);
            let (mut lo, mut hi, mut sum) = (current, current, 0);
            for t in taps {
                let index = (base as isize
                    + i64::from(t.dy) as isize * stride as isize
                    + i64::from(t.dx) as isize) as usize;
                let value = i32::from(src[index]);
                let difference = (value - current).abs();
                let filtered = difference
                    .min((i32::from(t.threshold) - (difference >> i32::from(t.shift))).max(0));
                sum += i32::from(t.weight) * if value < current { -filtered } else { filtered };
                lo = lo.min(value);
                hi = hi.max(value);
            }
            dst[base] = (current + ((8 + sum - i32::from(sum < 0)) >> 4)).clamp(lo, hi) as u16;
        }
    }
}

#[cfg(target_arch = "aarch64")]
#[target_feature(enable = "neon")]
unsafe fn av1_cdef_block_neon(
    src: &[u16],
    dst: &mut [u16],
    stride: usize,
    x: usize,
    y: usize,
    w: usize,
    h: usize,
    taps: &[CdefTap],
) {
    use std::arch::aarch64::*;
    // SAFETY: the caller guarantees every tap window lies inside both planes.
    unsafe {
        let src = src.as_ptr();
        let dst = dst.as_mut_ptr();
        let zero = vdupq_n_s16(0);
        let eight = vdupq_n_s16(8);
        let one = vdupq_n_s16(1);
        let zero4 = vdup_n_s16(0);
        let eight4 = vdup_n_s16(8);
        let one4 = vdup_n_s16(1);
        let mut offsets = [0isize; 12];
        for (offset, t) in offsets.iter_mut().zip(taps) {
            *offset = i64::from(t.dy) as isize * stride as isize + i64::from(t.dx) as isize;
        }
        for row in 0..h {
            let mut col = 0usize;
            while col + 8 <= w {
                let base = (y + row) * stride + x + col;
                let current = vreinterpretq_s16_u16(vld1q_u16(src.add(base)));
                let mut sum = zero;
                let mut lo = current;
                let mut hi = current;
                for (offset, t) in offsets.iter().zip(taps) {
                    let near = vreinterpretq_s16_u16(vld1q_u16(src.add(base).offset(*offset)));
                    let diff = vsubq_s16(near, current);
                    let abs = vabsq_s16(diff);
                    let scaled = vshlq_s16(abs, vdupq_n_s16(-t.shift));
                    let limit = vmaxq_s16(vsubq_s16(vdupq_n_s16(t.threshold), scaled), zero);
                    let filtered = vminq_s16(abs, limit);
                    let signed = vbslq_s16(vcltq_s16(diff, zero), vnegq_s16(filtered), filtered);
                    sum = vaddq_s16(sum, vmulq_s16(signed, vdupq_n_s16(t.weight)));
                    lo = vminq_s16(lo, near);
                    hi = vmaxq_s16(hi, near);
                }
                let bias = vandq_s16(vshrq_n_s16(sum, 15), one);
                let delta = vshrq_n_s16(vsubq_s16(vaddq_s16(sum, eight), bias), 4);
                let out = vmaxq_s16(vminq_s16(vaddq_s16(current, delta), hi), lo);
                vst1q_u16(dst.add(base), vreinterpretq_u16_s16(out));
                col += 8;
            }
            // Chroma CDEF blocks are only four pixels wide, so they never reach the
            // eight-lane body; a four-lane chunk keeps them off the scalar tail.
            while col + 4 <= w {
                let base = (y + row) * stride + x + col;
                let current = vreinterpret_s16_u16(vld1_u16(src.add(base)));
                let mut sum = zero4;
                let mut lo = current;
                let mut hi = current;
                for (offset, t) in offsets.iter().zip(taps) {
                    let near = vreinterpret_s16_u16(vld1_u16(src.add(base).offset(*offset)));
                    let diff = vsub_s16(near, current);
                    let abs = vabs_s16(diff);
                    let scaled = vshl_s16(abs, vdup_n_s16(-t.shift));
                    let limit = vmax_s16(vsub_s16(vdup_n_s16(t.threshold), scaled), zero4);
                    let filtered = vmin_s16(abs, limit);
                    let signed = vbsl_s16(vclt_s16(diff, zero4), vneg_s16(filtered), filtered);
                    sum = vadd_s16(sum, vmul_s16(signed, vdup_n_s16(t.weight)));
                    lo = vmin_s16(lo, near);
                    hi = vmax_s16(hi, near);
                }
                let bias = vand_s16(vshr_n_s16(sum, 15), one4);
                let delta = vshr_n_s16(vsub_s16(vadd_s16(sum, eight4), bias), 4);
                let out = vmax_s16(vmin_s16(vadd_s16(current, delta), hi), lo);
                vst1_u16(dst.add(base), vreinterpret_u16_s16(out));
                col += 4;
            }
            for c in col..w {
                let base = (y + row) * stride + x + c;
                let current = i32::from(*src.add(base));
                let (mut lo, mut hi, mut sum) = (current, current, 0);
                for t in taps {
                    let value = i32::from(*src.add(
                        (base as isize
                            + i64::from(t.dy) as isize * stride as isize
                            + i64::from(t.dx) as isize) as usize,
                    ));
                    let difference = (value - current).abs();
                    let filtered = difference
                        .min((i32::from(t.threshold) - (difference >> i32::from(t.shift))).max(0));
                    sum += i32::from(t.weight) * if value < current { -filtered } else { filtered };
                    lo = lo.min(value);
                    hi = hi.max(value);
                }
                *dst.add(base) =
                    (current + ((8 + sum - i32::from(sum < 0)) >> 4)).clamp(lo, hi) as u16;
            }
        }
    }
}

pub const HEVC_DCT: [[i16; 32]; 32] = [
    [
        64, 64, 64, 64, 64, 64, 64, 64, 64, 64, 64, 64, 64, 64, 64, 64, 64, 64, 64, 64, 64, 64, 64,
        64, 64, 64, 64, 64, 64, 64, 64, 64,
    ],
    [
        90, 90, 88, 85, 82, 78, 73, 67, 61, 54, 46, 38, 31, 22, 13, 4, -4, -13, -22, -31, -38, -46,
        -54, -61, -67, -73, -78, -82, -85, -88, -90, -90,
    ],
    [
        90, 87, 80, 70, 57, 43, 25, 9, -9, -25, -43, -57, -70, -80, -87, -90, -90, -87, -80, -70,
        -57, -43, -25, -9, 9, 25, 43, 57, 70, 80, 87, 90,
    ],
    [
        90, 82, 67, 46, 22, -4, -31, -54, -73, -85, -90, -88, -78, -61, -38, -13, 13, 38, 61, 78,
        88, 90, 85, 73, 54, 31, 4, -22, -46, -67, -82, -90,
    ],
    [
        89, 75, 50, 18, -18, -50, -75, -89, -89, -75, -50, -18, 18, 50, 75, 89, 89, 75, 50, 18,
        -18, -50, -75, -89, -89, -75, -50, -18, 18, 50, 75, 89,
    ],
    [
        88, 67, 31, -13, -54, -82, -90, -78, -46, -4, 38, 73, 90, 85, 61, 22, -22, -61, -85, -90,
        -73, -38, 4, 46, 78, 90, 82, 54, 13, -31, -67, -88,
    ],
    [
        87, 57, 9, -43, -80, -90, -70, -25, 25, 70, 90, 80, 43, -9, -57, -87, -87, -57, -9, 43, 80,
        90, 70, 25, -25, -70, -90, -80, -43, 9, 57, 87,
    ],
    [
        85, 46, -13, -67, -90, -73, -22, 38, 82, 88, 54, -4, -61, -90, -78, -31, 31, 78, 90, 61, 4,
        -54, -88, -82, -38, 22, 73, 90, 67, 13, -46, -85,
    ],
    [
        83, 36, -36, -83, -83, -36, 36, 83, 83, 36, -36, -83, -83, -36, 36, 83, 83, 36, -36, -83,
        -83, -36, 36, 83, 83, 36, -36, -83, -83, -36, 36, 83,
    ],
    [
        82, 22, -54, -90, -61, 13, 78, 85, 31, -46, -90, -67, 4, 73, 88, 38, -38, -88, -73, -4, 67,
        90, 46, -31, -85, -78, -13, 61, 90, 54, -22, -82,
    ],
    [
        80, 9, -70, -87, -25, 57, 90, 43, -43, -90, -57, 25, 87, 70, -9, -80, -80, -9, 70, 87, 25,
        -57, -90, -43, 43, 90, 57, -25, -87, -70, 9, 80,
    ],
    [
        78, -4, -82, -73, 13, 85, 67, -22, -88, -61, 31, 90, 54, -38, -90, -46, 46, 90, 38, -54,
        -90, -31, 61, 88, 22, -67, -85, -13, 73, 82, 4, -78,
    ],
    [
        75, -18, -89, -50, 50, 89, 18, -75, -75, 18, 89, 50, -50, -89, -18, 75, 75, -18, -89, -50,
        50, 89, 18, -75, -75, 18, 89, 50, -50, -89, -18, 75,
    ],
    [
        73, -31, -90, -22, 78, 67, -38, -90, -13, 82, 61, -46, -88, -4, 85, 54, -54, -85, 4, 88,
        46, -61, -82, 13, 90, 38, -67, -78, 22, 90, 31, -73,
    ],
    [
        70, -43, -87, 9, 90, 25, -80, -57, 57, 80, -25, -90, -9, 87, 43, -70, -70, 43, 87, -9, -90,
        -25, 80, 57, -57, -80, 25, 90, 9, -87, -43, 70,
    ],
    [
        67, -54, -78, 38, 85, -22, -90, 4, 90, 13, -88, -31, 82, 46, -73, -61, 61, 73, -46, -82,
        31, 88, -13, -90, -4, 90, 22, -85, -38, 78, 54, -67,
    ],
    [
        64, -64, -64, 64, 64, -64, -64, 64, 64, -64, -64, 64, 64, -64, -64, 64, 64, -64, -64, 64,
        64, -64, -64, 64, 64, -64, -64, 64, 64, -64, -64, 64,
    ],
    [
        61, -73, -46, 82, 31, -88, -13, 90, -4, -90, 22, 85, -38, -78, 54, 67, -67, -54, 78, 38,
        -85, -22, 90, 4, -90, 13, 88, -31, -82, 46, 73, -61,
    ],
    [
        57, -80, -25, 90, -9, -87, 43, 70, -70, -43, 87, 9, -90, 25, 80, -57, -57, 80, 25, -90, 9,
        87, -43, -70, 70, 43, -87, -9, 90, -25, -80, 57,
    ],
    [
        54, -85, -4, 88, -46, -61, 82, 13, -90, 38, 67, -78, -22, 90, -31, -73, 73, 31, -90, 22,
        78, -67, -38, 90, -13, -82, 61, 46, -88, 4, 85, -54,
    ],
    [
        50, -89, 18, 75, -75, -18, 89, -50, -50, 89, -18, -75, 75, 18, -89, 50, 50, -89, 18, 75,
        -75, -18, 89, -50, -50, 89, -18, -75, 75, 18, -89, 50,
    ],
    [
        46, -90, 38, 54, -90, 31, 61, -88, 22, 67, -85, 13, 73, -82, 4, 78, -78, -4, 82, -73, -13,
        85, -67, -22, 88, -61, -31, 90, -54, -38, 90, -46,
    ],
    [
        43, -90, 57, 25, -87, 70, 9, -80, 80, -9, -70, 87, -25, -57, 90, -43, -43, 90, -57, -25,
        87, -70, -9, 80, -80, 9, 70, -87, 25, 57, -90, 43,
    ],
    [
        38, -88, 73, -4, -67, 90, -46, -31, 85, -78, 13, 61, -90, 54, 22, -82, 82, -22, -54, 90,
        -61, -13, 78, -85, 31, 46, -90, 67, 4, -73, 88, -38,
    ],
    [
        36, -83, 83, -36, -36, 83, -83, 36, 36, -83, 83, -36, -36, 83, -83, 36, 36, -83, 83, -36,
        -36, 83, -83, 36, 36, -83, 83, -36, -36, 83, -83, 36,
    ],
    [
        31, -78, 90, -61, 4, 54, -88, 82, -38, -22, 73, -90, 67, -13, -46, 85, -85, 46, 13, -67,
        90, -73, 22, 38, -82, 88, -54, -4, 61, -90, 78, -31,
    ],
    [
        25, -70, 90, -80, 43, 9, -57, 87, -87, 57, -9, -43, 80, -90, 70, -25, -25, 70, -90, 80,
        -43, -9, 57, -87, 87, -57, 9, 43, -80, 90, -70, 25,
    ],
    [
        22, -61, 85, -90, 73, -38, -4, 46, -78, 90, -82, 54, -13, -31, 67, -88, 88, -67, 31, 13,
        -54, 82, -90, 78, -46, 4, 38, -73, 90, -85, 61, -22,
    ],
    [
        18, -50, 75, -89, 89, -75, 50, -18, -18, 50, -75, 89, -89, 75, -50, 18, 18, -50, 75, -89,
        89, -75, 50, -18, -18, 50, -75, 89, -89, 75, -50, 18,
    ],
    [
        13, -38, 61, -78, 88, -90, 85, -73, 54, -31, 4, 22, -46, 67, -82, 90, -90, 82, -67, 46,
        -22, -4, 31, -54, 73, -85, 90, -88, 78, -61, 38, -13,
    ],
    [
        9, -25, 43, -57, 70, -80, 87, -90, 90, -87, 80, -70, 57, -43, 25, -9, -9, 25, -43, 57, -70,
        80, -87, 90, -90, 87, -80, 70, -57, 43, -25, 9,
    ],
    [
        4, -13, 22, -31, 38, -46, 54, -61, 67, -73, 78, -82, 85, -88, 90, -90, 90, -90, 88, -85,
        82, -78, 73, -67, 61, -54, 46, -38, 31, -22, 13, -4,
    ],
];

fn hevc_inverse2_scalar(input: &[i32; 2], output: &mut [i32; 2]) {
    output[0] = 64 * (input[0] + input[1]);
    output[1] = 64 * (input[0] - input[1]);
}

macro_rules! hevc_inverse_size_scalar {
    ($name:ident, $smaller:ident, $n:expr) => {
        fn $name(input: &[i32; $n], output: &mut [i32; $n]) {
            let even = std::array::from_fn::<_, { $n / 2 }, _>(|k| input[2 * k]);
            let mut values = [0; $n / 2];
            $smaller(&even, &mut values);
            let mut odd = [0; $n / 2];
            for k in (1..$n).step_by(2) {
                if input[k] == 0 {
                    continue;
                }
                for x in 0..$n / 2 {
                    odd[x] += input[k] * i32::from(HEVC_DCT[k * (32 / $n)][x]);
                }
            }
            for x in 0..$n / 2 {
                output[x] = values[x] + odd[x];
                output[$n - 1 - x] = values[x] - odd[x];
            }
        }
    };
}

hevc_inverse_size_scalar!(hevc_inverse4_scalar_arr, hevc_inverse2_scalar, 4);
hevc_inverse_size_scalar!(hevc_inverse8_scalar_arr, hevc_inverse4_scalar_arr, 8);
hevc_inverse_size_scalar!(hevc_inverse16_scalar_arr, hevc_inverse8_scalar_arr, 16);
hevc_inverse_size_scalar!(hevc_inverse32_scalar_arr, hevc_inverse16_scalar_arr, 32);

pub fn hevc_inverse_scalar(input: &[i32], output: &mut [i32]) {
    let len = input.len();
    if input[1..].iter().all(|&v| v == 0) {
        output[..len].fill(input[0] * 64);
        return;
    }
    match len {
        4 => hevc_inverse4_scalar_arr(input.try_into().unwrap(), output.try_into().unwrap()),
        8 => hevc_inverse8_scalar_arr(input.try_into().unwrap(), output.try_into().unwrap()),
        16 => hevc_inverse16_scalar_arr(input.try_into().unwrap(), output.try_into().unwrap()),
        32 => hevc_inverse32_scalar_arr(input.try_into().unwrap(), output.try_into().unwrap()),
        _ => panic!("unsupported HEVC transform size {len}"),
    }
}

#[cfg(target_arch = "aarch64")]
mod hevc_neon {
    use super::*;
    use std::arch::aarch64::*;

    #[inline]
    pub unsafe fn vrev_s32(v: int32x4_t) -> int32x4_t {
        unsafe {
            let rev64 = vrev64q_s32(v);
            vcombine_s32(vget_high_s32(rev64), vget_low_s32(rev64))
        }
    }

    pub unsafe fn hevc_inverse8_neon(input: &[i32; 8], output: &mut [i32; 8]) {
        let even: [i32; 4] = [input[0], input[2], input[4], input[6]];
        let mut values = [0i32; 4];
        hevc_inverse4_scalar_arr(&even, &mut values);

        unsafe {
            let val = vld1q_s32(values.as_ptr());
            let mut odd = vdupq_n_s32(0);
            for k in (1..8).step_by(2) {
                if input[k] == 0 {
                    continue;
                }
                let d_16 = vld1_s16(HEVC_DCT[k * 4].as_ptr());
                let d_32 = vmovl_s16(d_16);
                odd = vmlaq_n_s32(odd, d_32, input[k]);
            }
            let sum = vaddq_s32(val, odd);
            let diff = vsubq_s32(val, odd);
            vst1q_s32(output.as_mut_ptr(), sum);
            vst1q_s32(output.as_mut_ptr().add(4), vrev_s32(diff));
        }
    }

    pub unsafe fn hevc_inverse16_neon(input: &[i32; 16], output: &mut [i32; 16]) {
        let even: [i32; 8] = std::array::from_fn(|k| input[2 * k]);
        let mut values = [0i32; 8];
        unsafe { hevc_inverse8_neon(&even, &mut values) };

        unsafe {
            let val0 = vld1q_s32(values.as_ptr());
            let val1 = vld1q_s32(values.as_ptr().add(4));
            let mut odd0 = vdupq_n_s32(0);
            let mut odd1 = vdupq_n_s32(0);
            for k in (1..16).step_by(2) {
                if input[k] == 0 {
                    continue;
                }
                let ptr = HEVC_DCT[k * 2].as_ptr();
                let d0 = vmovl_s16(vld1_s16(ptr));
                let d1 = vmovl_s16(vld1_s16(ptr.add(4)));
                odd0 = vmlaq_n_s32(odd0, d0, input[k]);
                odd1 = vmlaq_n_s32(odd1, d1, input[k]);
            }
            let sum0 = vaddq_s32(val0, odd0);
            let sum1 = vaddq_s32(val1, odd1);
            let diff0 = vsubq_s32(val0, odd0);
            let diff1 = vsubq_s32(val1, odd1);
            vst1q_s32(output.as_mut_ptr(), sum0);
            vst1q_s32(output.as_mut_ptr().add(4), sum1);
            vst1q_s32(output.as_mut_ptr().add(8), vrev_s32(diff1));
            vst1q_s32(output.as_mut_ptr().add(12), vrev_s32(diff0));
        }
    }

    pub unsafe fn hevc_inverse32_neon(input: &[i32; 32], output: &mut [i32; 32]) {
        let even: [i32; 16] = std::array::from_fn(|k| input[2 * k]);
        let mut values = [0i32; 16];
        unsafe { hevc_inverse16_neon(&even, &mut values) };

        unsafe {
            let val0 = vld1q_s32(values.as_ptr());
            let val1 = vld1q_s32(values.as_ptr().add(4));
            let val2 = vld1q_s32(values.as_ptr().add(8));
            let val3 = vld1q_s32(values.as_ptr().add(12));
            let mut odd0 = vdupq_n_s32(0);
            let mut odd1 = vdupq_n_s32(0);
            let mut odd2 = vdupq_n_s32(0);
            let mut odd3 = vdupq_n_s32(0);
            for k in (1..32).step_by(2) {
                if input[k] == 0 {
                    continue;
                }
                let ptr = HEVC_DCT[k].as_ptr();
                let d0 = vmovl_s16(vld1_s16(ptr));
                let d1 = vmovl_s16(vld1_s16(ptr.add(4)));
                let d2 = vmovl_s16(vld1_s16(ptr.add(8)));
                let d3 = vmovl_s16(vld1_s16(ptr.add(12)));
                odd0 = vmlaq_n_s32(odd0, d0, input[k]);
                odd1 = vmlaq_n_s32(odd1, d1, input[k]);
                odd2 = vmlaq_n_s32(odd2, d2, input[k]);
                odd3 = vmlaq_n_s32(odd3, d3, input[k]);
            }
            let sum0 = vaddq_s32(val0, odd0);
            let sum1 = vaddq_s32(val1, odd1);
            let sum2 = vaddq_s32(val2, odd2);
            let sum3 = vaddq_s32(val3, odd3);
            let diff0 = vsubq_s32(val0, odd0);
            let diff1 = vsubq_s32(val1, odd1);
            let diff2 = vsubq_s32(val2, odd2);
            let diff3 = vsubq_s32(val3, odd3);
            vst1q_s32(output.as_mut_ptr(), sum0);
            vst1q_s32(output.as_mut_ptr().add(4), sum1);
            vst1q_s32(output.as_mut_ptr().add(8), sum2);
            vst1q_s32(output.as_mut_ptr().add(12), sum3);
            vst1q_s32(output.as_mut_ptr().add(16), vrev_s32(diff3));
            vst1q_s32(output.as_mut_ptr().add(20), vrev_s32(diff2));
            vst1q_s32(output.as_mut_ptr().add(24), vrev_s32(diff1));
            vst1q_s32(output.as_mut_ptr().add(28), vrev_s32(diff0));
        }
    }
}

#[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
mod hevc_avx2 {
    use super::*;
    #[cfg(target_arch = "x86")]
    use std::arch::x86::*;
    #[cfg(target_arch = "x86_64")]
    use std::arch::x86_64::*;

    #[target_feature(enable = "avx2")]
    pub unsafe fn hevc_inverse8_avx2(input: &[i32; 8], output: &mut [i32; 8]) {
        let even: [i32; 4] = [input[0], input[2], input[4], input[6]];
        let mut values = [0i32; 4];
        hevc_inverse4_scalar_arr(&even, &mut values);

        unsafe {
            let val = _mm_loadu_si128(values.as_ptr().cast());
            let mut odd = _mm_setzero_si128();
            for k in (1..8).step_by(2) {
                if input[k] == 0 {
                    continue;
                }
                let d_16 = _mm_loadl_epi64(HEVC_DCT[k * 4].as_ptr().cast());
                let d_32 = _mm_cvtepi16_epi32(d_16);
                let prod = _mm_mullo_epi32(d_32, _mm_set1_epi32(input[k]));
                odd = _mm_add_epi32(odd, prod);
            }
            let sum = _mm_add_epi32(val, odd);
            let diff = _mm_sub_epi32(val, odd);
            _mm_storeu_si128(output.as_mut_ptr().cast(), sum);
            let rev_diff = _mm_shuffle_epi32(diff, 0b00_01_10_11);
            _mm_storeu_si128(output.as_mut_ptr().add(4).cast(), rev_diff);
        }
    }

    #[target_feature(enable = "avx2")]
    pub unsafe fn hevc_inverse16_avx2(input: &[i32; 16], output: &mut [i32; 16]) {
        let even: [i32; 8] = std::array::from_fn(|k| input[2 * k]);
        let mut values = [0i32; 8];
        unsafe { hevc_inverse8_avx2(&even, &mut values) };

        unsafe {
            let val = _mm256_loadu_si256(values.as_ptr().cast());
            let mut odd = _mm256_setzero_si256();
            for k in (1..16).step_by(2) {
                if input[k] == 0 {
                    continue;
                }
                let d_16 = _mm_loadu_si128(HEVC_DCT[k * 2].as_ptr().cast());
                let d_32 = _mm256_cvtepi16_epi32(d_16);
                let prod = _mm256_mullo_epi32(d_32, _mm256_set1_epi32(input[k]));
                odd = _mm256_add_epi32(odd, prod);
            }
            let sum = _mm256_add_epi32(val, odd);
            let diff = _mm256_sub_epi32(val, odd);
            _mm256_storeu_si256(output.as_mut_ptr().cast(), sum);
            let rev_mask = _mm256_setr_epi32(7, 6, 5, 4, 3, 2, 1, 0);
            let rev_diff = _mm256_permutevar8x32_epi32(diff, rev_mask);
            _mm256_storeu_si256(output.as_mut_ptr().add(8).cast(), rev_diff);
        }
    }

    #[target_feature(enable = "avx2")]
    pub unsafe fn hevc_inverse32_avx2(input: &[i32; 32], output: &mut [i32; 32]) {
        let even: [i32; 16] = std::array::from_fn(|k| input[2 * k]);
        let mut values = [0i32; 16];
        unsafe { hevc_inverse16_avx2(&even, &mut values) };

        unsafe {
            let val0 = _mm256_loadu_si256(values.as_ptr().cast());
            let val1 = _mm256_loadu_si256(values.as_ptr().add(8).cast());
            let mut odd0 = _mm256_setzero_si256();
            let mut odd1 = _mm256_setzero_si256();
            for k in (1..32).step_by(2) {
                if input[k] == 0 {
                    continue;
                }
                let ptr = HEVC_DCT[k].as_ptr();
                let d0_16 = _mm_loadu_si128(ptr.cast());
                let d0_32 = _mm256_cvtepi16_epi32(d0_16);
                let d1_16 = _mm_loadu_si128(ptr.add(8).cast());
                let d1_32 = _mm256_cvtepi16_epi32(d1_16);
                let scale = _mm256_set1_epi32(input[k]);
                odd0 = _mm256_add_epi32(odd0, _mm256_mullo_epi32(d0_32, scale));
                odd1 = _mm256_add_epi32(odd1, _mm256_mullo_epi32(d1_32, scale));
            }
            let sum0 = _mm256_add_epi32(val0, odd0);
            let sum1 = _mm256_add_epi32(val1, odd1);
            let diff0 = _mm256_sub_epi32(val0, odd0);
            let diff1 = _mm256_sub_epi32(val1, odd1);
            _mm256_storeu_si256(output.as_mut_ptr().cast(), sum0);
            _mm256_storeu_si256(output.as_mut_ptr().add(8).cast(), sum1);
            let rev_mask = _mm256_setr_epi32(7, 6, 5, 4, 3, 2, 1, 0);
            let rev_diff1 = _mm256_permutevar8x32_epi32(diff1, rev_mask);
            let rev_diff0 = _mm256_permutevar8x32_epi32(diff0, rev_mask);
            _mm256_storeu_si256(output.as_mut_ptr().add(16).cast(), rev_diff1);
            _mm256_storeu_si256(output.as_mut_ptr().add(24).cast(), rev_diff0);
        }
    }
}

/// SIMD-accelerated HEVC inverse DCT for sizes 4, 8, 16, and 32.
///
/// Dispatches to NEON on aarch64, AVX2 on x86_64 when detected, or scalar factorization.
pub fn hevc_inverse_dct(input: &[i32], output: &mut [i32]) {
    let len = input.len();
    if len == 0 {
        return;
    }
    if input[1..].iter().all(|&v| v == 0) {
        output[..len].fill(input[0] * 64);
        return;
    }
    #[cfg(target_arch = "aarch64")]
    {
        // SAFETY: NEON is baseline on aarch64.
        match len {
            4 => hevc_inverse4_scalar_arr(input.try_into().unwrap(), output.try_into().unwrap()),
            8 => unsafe { hevc_neon::hevc_inverse8_neon(input.try_into().unwrap(), output.try_into().unwrap()) },
            16 => unsafe { hevc_neon::hevc_inverse16_neon(input.try_into().unwrap(), output.try_into().unwrap()) },
            32 => unsafe { hevc_neon::hevc_inverse32_neon(input.try_into().unwrap(), output.try_into().unwrap()) },
            _ => panic!("unsupported HEVC transform size {len}"),
        }
        return;
    }
    #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
    {
        if has_avx2() {
            // SAFETY: AVX2 detected.
            unsafe {
                match len {
                    4 => {
                        hevc_inverse4_scalar_arr(input.try_into().unwrap(), output.try_into().unwrap());
                        return;
                    }
                    8 => {
                        hevc_avx2::hevc_inverse8_avx2(input.try_into().unwrap(), output.try_into().unwrap());
                        return;
                    }
                    16 => {
                        hevc_avx2::hevc_inverse16_avx2(input.try_into().unwrap(), output.try_into().unwrap());
                        return;
                    }
                    32 => {
                        hevc_avx2::hevc_inverse32_avx2(input.try_into().unwrap(), output.try_into().unwrap());
                        return;
                    }
                    _ => panic!("unsupported HEVC transform size {len}"),
                }
            }
        }
    }
    #[allow(unreachable_code)]
    match len {
        4 => hevc_inverse4_scalar_arr(input.try_into().unwrap(), output.try_into().unwrap()),
        8 => hevc_inverse8_scalar_arr(input.try_into().unwrap(), output.try_into().unwrap()),
        16 => hevc_inverse16_scalar_arr(input.try_into().unwrap(), output.try_into().unwrap()),
        32 => hevc_inverse32_scalar_arr(input.try_into().unwrap(), output.try_into().unwrap()),
        _ => panic!("unsupported HEVC transform size {len}"),
    }
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Complex64 {
    pub re: f64,
    pub im: f64,
}

// ==========================================
// AVC 6-tap Motion Compensation Kernels
// ==========================================

pub fn avc_luma_hsum_6tap_scalar(src: &[u16], out: &mut [i32]) {
    let n = out.len();
    assert!(src.len() >= n + 5);
    for c in 0..n {
        let v0 = i32::from(src[c]);
        let v1 = i32::from(src[c + 1]);
        let v2 = i32::from(src[c + 2]);
        let v3 = i32::from(src[c + 3]);
        let v4 = i32::from(src[c + 4]);
        let v5 = i32::from(src[c + 5]);
        out[c] = v0 - 5 * v1 + 20 * v2 + 20 * v3 - 5 * v4 + v5;
    }
}

pub fn avc_luma_vsum_6tap_scalar(
    r0: &[u16], r1: &[u16], r2: &[u16], r3: &[u16], r4: &[u16], r5: &[u16],
    out: &mut [i32],
) {
    let n = out.len();
    assert!(r0.len() >= n && r1.len() >= n && r2.len() >= n && r3.len() >= n && r4.len() >= n && r5.len() >= n);
    for c in 0..n {
        let v0 = i32::from(r0[c]);
        let v1 = i32::from(r1[c]);
        let v2 = i32::from(r2[c]);
        let v3 = i32::from(r3[c]);
        let v4 = i32::from(r4[c]);
        let v5 = i32::from(r5[c]);
        out[c] = v0 - 5 * v1 + 20 * v2 + 20 * v3 - 5 * v4 + v5;
    }
}

pub fn avc_luma_vsum_i32_6tap_scalar(
    r0: &[i32], r1: &[i32], r2: &[i32], r3: &[i32], r4: &[i32], r5: &[i32],
    out: &mut [i32],
) {
    let n = out.len();
    assert!(r0.len() >= n && r1.len() >= n && r2.len() >= n && r3.len() >= n && r4.len() >= n && r5.len() >= n);
    for c in 0..n {
        out[c] = r0[c] - 5 * r1[c] + 20 * r2[c] + 20 * r3[c] - 5 * r4[c] + r5[c];
    }
}

pub fn avc_chroma_bilinear_scalar(
    top: &[u16], bottom: &[u16], out: &mut [u16], weights: [i32; 4],
) {
    let n = out.len();
    assert!(top.len() >= n + 1 && bottom.len() >= n + 1);
    let w = weights;
    for col in 0..n {
        let value = w[0] * i32::from(top[col])
            + w[1] * i32::from(top[col + 1])
            + w[2] * i32::from(bottom[col])
            + w[3] * i32::from(bottom[col + 1]);
        out[col] = ((value + 32) >> 6) as u16;
    }
}

// ==========================================
// HEVC FIR Accumulate Kernels
// ==========================================

pub fn hevc_fir_accumulate_u16_scalar(target: &mut [i32], source: &[u16], weight: i32) {
    let n = target.len().min(source.len());
    for c in 0..n {
        target[c] += weight * i32::from(source[c]);
    }
}

pub fn hevc_fir_accumulate_i32_scalar(target: &mut [i32], source: &[i32], weight: i32) {
    let n = target.len().min(source.len());
    for c in 0..n {
        target[c] += weight * source[c];
    }
}

// ==========================================
// AAC SBR QMF Synthesis Kernels
// ==========================================

pub fn sbr_qmf_synthesis_slot_scalar(
    slot: &[Complex64; 64],
    transform: &[[Complex64; 64]; 128],
    history: &mut [f64; 128],
) -> bool {
    for (n, val) in history.iter_mut().enumerate() {
        let mut sum = 0.0;
        let t_row = &transform[n];
        for k in 0..64 {
            sum += slot[k].re * t_row[k].re - slot[k].im * t_row[k].im;
        }
        if !sum.is_finite() {
            return false;
        }
        *val = sum;
    }
    true
}

pub fn sbr_qmf_synthesis32_slot_scalar(
    slot: &[Complex64; 32],
    transform: &[[Complex64; 32]; 64],
    history: &mut [f64; 64],
) -> bool {
    for (n, val) in history.iter_mut().enumerate() {
        let mut sum = 0.0;
        let t_row = &transform[n];
        for k in 0..32 {
            sum += slot[k].re * t_row[k].re - slot[k].im * t_row[k].im;
        }
        if !sum.is_finite() {
            return false;
        }
        *val = sum;
    }
    true
}

// ==========================================
// AV1 Wiener 7-Tap Kernels
// ==========================================

pub fn av1_wiener_h_7tap_scalar(
    padded_row: &[u16],
    hf: [i32; 4],
    r0: u32,
    min_val: i32,
    max_val: i32,
    out: &mut [i32],
) {
    let n = out.len();
    assert!(padded_row.len() >= n + 6);
    let bias = if r0 == 0 { 0 } else { 1i32 << (r0 - 1) };
    for col in 0..n {
        let s06 = i32::from(padded_row[col]) + i32::from(padded_row[col + 6]);
        let s15 = i32::from(padded_row[col + 1]) + i32::from(padded_row[col + 5]);
        let s24 = i32::from(padded_row[col + 2]) + i32::from(padded_row[col + 4]);
        let s3 = i32::from(padded_row[col + 3]);
        let sum = hf[0] * s06 + hf[1] * s15 + hf[2] * s24 + hf[3] * s3;
        let rounded = (sum + bias) >> r0;
        out[col] = rounded.clamp(min_val, max_val);
    }
}

#[allow(clippy::too_many_arguments)]
pub fn av1_wiener_v_7tap_scalar(
    r0: &[i32], r1: &[i32], r2: &[i32], r3: &[i32], r4: &[i32], r5: &[i32], r6: &[i32],
    vf: [i32; 4],
    shift: u32,
    max_sample: i32,
    out: &mut [u16],
) {
    let n = out.len();
    assert!(r0.len() >= n && r1.len() >= n && r2.len() >= n && r3.len() >= n && r4.len() >= n && r5.len() >= n && r6.len() >= n);
    let bias = if shift == 0 { 0 } else { 1i32 << (shift - 1) };
    for col in 0..n {
        let sum = vf[0] * (r0[col] + r6[col])
            + vf[1] * (r1[col] + r5[col])
            + vf[2] * (r2[col] + r4[col])
            + vf[3] * r3[col];
        let rounded = (sum + bias) >> shift;
        out[col] = rounded.clamp(0, max_sample) as u16;
    }
}

#[cfg(target_arch = "aarch64")]
mod mc_neon {
    use super::*;
    use std::arch::aarch64::*;

    #[target_feature(enable = "neon")]
    pub unsafe fn avc_luma_hsum_6tap_neon(src: &[u16], out: &mut [i32]) {
        let n = out.len();
        assert!(src.len() >= n + 5);
        let mut c = 0;
        unsafe {
            while c + 4 <= n {
                let ptr = src.as_ptr().add(c);
                let s0 = vreinterpretq_s32_u32(vmovl_u16(vld1_u16(ptr)));
                let s1 = vreinterpretq_s32_u32(vmovl_u16(vld1_u16(ptr.add(1))));
                let s2 = vreinterpretq_s32_u32(vmovl_u16(vld1_u16(ptr.add(2))));
                let s3 = vreinterpretq_s32_u32(vmovl_u16(vld1_u16(ptr.add(3))));
                let s4 = vreinterpretq_s32_u32(vmovl_u16(vld1_u16(ptr.add(4))));
                let s5 = vreinterpretq_s32_u32(vmovl_u16(vld1_u16(ptr.add(5))));

                let s05 = vaddq_s32(s0, s5);
                let s14 = vaddq_s32(s1, s4);
                let s23 = vaddq_s32(s2, s3);

                let res = vmlaq_n_s32(s05, s23, 20);
                let res = vmlsq_n_s32(res, s14, 5);

                vst1q_s32(out.as_mut_ptr().add(c), res);
                c += 4;
            }
            while c < n {
                let v0 = i32::from(*src.get_unchecked(c));
                let v1 = i32::from(*src.get_unchecked(c + 1));
                let v2 = i32::from(*src.get_unchecked(c + 2));
                let v3 = i32::from(*src.get_unchecked(c + 3));
                let v4 = i32::from(*src.get_unchecked(c + 4));
                let v5 = i32::from(*src.get_unchecked(c + 5));
                *out.get_unchecked_mut(c) = v0 - 5 * v1 + 20 * v2 + 20 * v3 - 5 * v4 + v5;
                c += 1;
            }
        }
    }

    #[target_feature(enable = "neon")]
    pub unsafe fn avc_luma_vsum_6tap_neon(
        r0: &[u16], r1: &[u16], r2: &[u16], r3: &[u16], r4: &[u16], r5: &[u16],
        out: &mut [i32],
    ) {
        let n = out.len();
        assert!(r0.len() >= n && r1.len() >= n && r2.len() >= n && r3.len() >= n && r4.len() >= n && r5.len() >= n);
        let mut c = 0;
        unsafe {
            while c + 4 <= n {
                let s0 = vreinterpretq_s32_u32(vmovl_u16(vld1_u16(r0.as_ptr().add(c))));
                let s1 = vreinterpretq_s32_u32(vmovl_u16(vld1_u16(r1.as_ptr().add(c))));
                let s2 = vreinterpretq_s32_u32(vmovl_u16(vld1_u16(r2.as_ptr().add(c))));
                let s3 = vreinterpretq_s32_u32(vmovl_u16(vld1_u16(r3.as_ptr().add(c))));
                let s4 = vreinterpretq_s32_u32(vmovl_u16(vld1_u16(r4.as_ptr().add(c))));
                let s5 = vreinterpretq_s32_u32(vmovl_u16(vld1_u16(r5.as_ptr().add(c))));

                let s05 = vaddq_s32(s0, s5);
                let s14 = vaddq_s32(s1, s4);
                let s23 = vaddq_s32(s2, s3);

                let res = vmlaq_n_s32(s05, s23, 20);
                let res = vmlsq_n_s32(res, s14, 5);

                vst1q_s32(out.as_mut_ptr().add(c), res);
                c += 4;
            }
            while c < n {
                let v0 = i32::from(*r0.get_unchecked(c));
                let v1 = i32::from(*r1.get_unchecked(c));
                let v2 = i32::from(*r2.get_unchecked(c));
                let v3 = i32::from(*r3.get_unchecked(c));
                let v4 = i32::from(*r4.get_unchecked(c));
                let v5 = i32::from(*r5.get_unchecked(c));
                *out.get_unchecked_mut(c) = v0 - 5 * v1 + 20 * v2 + 20 * v3 - 5 * v4 + v5;
                c += 1;
            }
        }
    }

    #[target_feature(enable = "neon")]
    pub unsafe fn avc_luma_vsum_i32_6tap_neon(
        r0: &[i32], r1: &[i32], r2: &[i32], r3: &[i32], r4: &[i32], r5: &[i32],
        out: &mut [i32],
    ) {
        let n = out.len();
        assert!(r0.len() >= n && r1.len() >= n && r2.len() >= n && r3.len() >= n && r4.len() >= n && r5.len() >= n);
        let mut c = 0;
        unsafe {
            while c + 4 <= n {
                let s0 = vld1q_s32(r0.as_ptr().add(c));
                let s1 = vld1q_s32(r1.as_ptr().add(c));
                let s2 = vld1q_s32(r2.as_ptr().add(c));
                let s3 = vld1q_s32(r3.as_ptr().add(c));
                let s4 = vld1q_s32(r4.as_ptr().add(c));
                let s5 = vld1q_s32(r5.as_ptr().add(c));

                let s05 = vaddq_s32(s0, s5);
                let s14 = vaddq_s32(s1, s4);
                let s23 = vaddq_s32(s2, s3);

                let res = vmlaq_n_s32(s05, s23, 20);
                let res = vmlsq_n_s32(res, s14, 5);

                vst1q_s32(out.as_mut_ptr().add(c), res);
                c += 4;
            }
            while c < n {
                *out.get_unchecked_mut(c) = *r0.get_unchecked(c)
                    - 5 * (*r1.get_unchecked(c))
                    + 20 * (*r2.get_unchecked(c))
                    + 20 * (*r3.get_unchecked(c))
                    - 5 * (*r4.get_unchecked(c))
                    + *r5.get_unchecked(c);
                c += 1;
            }
        }
    }

    #[target_feature(enable = "neon")]
    pub unsafe fn avc_chroma_bilinear_neon(
        top: &[u16], bottom: &[u16], out: &mut [u16], weights: [i32; 4],
    ) {
        let n = out.len();
        assert!(top.len() >= n + 1 && bottom.len() >= n + 1);
        let mut col = 0;
        unsafe {
            let w0 = vdupq_n_s32(weights[0]);
            let w1 = vdupq_n_s32(weights[1]);
            let w2 = vdupq_n_s32(weights[2]);
            let w3 = vdupq_n_s32(weights[3]);
            let round = vdupq_n_s32(32);
            while col + 4 <= n {
                let t0 = vreinterpretq_s32_u32(vmovl_u16(vld1_u16(top.as_ptr().add(col))));
                let t1 = vreinterpretq_s32_u32(vmovl_u16(vld1_u16(top.as_ptr().add(col + 1))));
                let b0 = vreinterpretq_s32_u32(vmovl_u16(vld1_u16(bottom.as_ptr().add(col))));
                let b1 = vreinterpretq_s32_u32(vmovl_u16(vld1_u16(bottom.as_ptr().add(col + 1))));

                let mut acc = vmlaq_s32(round, t0, w0);
                acc = vmlaq_s32(acc, t1, w1);
                acc = vmlaq_s32(acc, b0, w2);
                acc = vmlaq_s32(acc, b1, w3);

                let shifted = vshrq_n_s32(acc, 6);
                let res16 = vmovn_u32(vreinterpretq_u32_s32(shifted));
                vst1_u16(out.as_mut_ptr().add(col), res16);
                col += 4;
            }
            while col < n {
                let value = weights[0] * i32::from(*top.get_unchecked(col))
                    + weights[1] * i32::from(*top.get_unchecked(col + 1))
                    + weights[2] * i32::from(*bottom.get_unchecked(col))
                    + weights[3] * i32::from(*bottom.get_unchecked(col + 1));
                *out.get_unchecked_mut(col) = ((value + 32) >> 6) as u16;
                col += 1;
            }
        }
    }

    #[target_feature(enable = "neon")]
    pub unsafe fn hevc_fir_accumulate_u16_neon(target: &mut [i32], source: &[u16], weight: i32) {
        let n = target.len().min(source.len());
        let mut c = 0;
        unsafe {
            let w = vdupq_n_s32(weight);
            while c + 4 <= n {
                let s_u16 = vld1_u16(source.as_ptr().add(c));
                let s_s32 = vreinterpretq_s32_u32(vmovl_u16(s_u16));
                let t = vld1q_s32(target.as_ptr().add(c));
                let acc = vmlaq_s32(t, s_s32, w);
                vst1q_s32(target.as_mut_ptr().add(c), acc);
                c += 4;
            }
            while c < n {
                *target.get_unchecked_mut(c) += weight * i32::from(*source.get_unchecked(c));
                c += 1;
            }
        }
    }

    #[target_feature(enable = "neon")]
    pub unsafe fn hevc_fir_accumulate_i32_neon(target: &mut [i32], source: &[i32], weight: i32) {
        let n = target.len().min(source.len());
        let mut c = 0;
        unsafe {
            let w = vdupq_n_s32(weight);
            while c + 4 <= n {
                let s = vld1q_s32(source.as_ptr().add(c));
                let t = vld1q_s32(target.as_ptr().add(c));
                let acc = vmlaq_s32(t, s, w);
                vst1q_s32(target.as_mut_ptr().add(c), acc);
                c += 4;
            }
            while c < n {
                *target.get_unchecked_mut(c) += weight * (*source.get_unchecked(c));
                c += 1;
            }
        }
    }

    #[target_feature(enable = "neon")]
    pub unsafe fn sbr_qmf_synthesis_slot_neon(
        slot: &[Complex64; 64],
        transform: &[[Complex64; 64]; 128],
        history: &mut [f64; 128],
    ) -> bool {
        unsafe {
            let slot_ptr = slot.as_ptr().cast::<f64>();
            for n in 0..128 {
                let t_ptr = transform[n].as_ptr().cast::<f64>();
                let mut sum = 0.0f64;
                let mut k = 0;
                while k < 64 {
                    let x0 = vld2q_f64(slot_ptr.add(k * 2));
                    let m0 = vld2q_f64(t_ptr.add(k * 2));
                    let d0 = vsubq_f64(vmulq_f64(x0.0, m0.0), vmulq_f64(x0.1, m0.1));

                    let x1 = vld2q_f64(slot_ptr.add((k + 2) * 2));
                    let m1 = vld2q_f64(t_ptr.add((k + 2) * 2));
                    let d1 = vsubq_f64(vmulq_f64(x1.0, m1.0), vmulq_f64(x1.1, m1.1));

                    sum += vgetq_lane_f64::<0>(d0);
                    sum += vgetq_lane_f64::<1>(d0);
                    sum += vgetq_lane_f64::<0>(d1);
                    sum += vgetq_lane_f64::<1>(d1);
                    k += 4;
                }
                if !sum.is_finite() {
                    return false;
                }
                history[n] = sum;
            }
        }
        true
    }

    #[target_feature(enable = "neon")]
    pub unsafe fn sbr_qmf_synthesis32_slot_neon(
        slot: &[Complex64; 32],
        transform: &[[Complex64; 32]; 64],
        history: &mut [f64; 64],
    ) -> bool {
        unsafe {
            let slot_ptr = slot.as_ptr().cast::<f64>();
            for n in 0..64 {
                let t_ptr = transform[n].as_ptr().cast::<f64>();
                let mut sum = 0.0f64;
                let mut k = 0;
                while k < 32 {
                    let x0 = vld2q_f64(slot_ptr.add(k * 2));
                    let m0 = vld2q_f64(t_ptr.add(k * 2));
                    let d0 = vsubq_f64(vmulq_f64(x0.0, m0.0), vmulq_f64(x0.1, m0.1));

                    let x1 = vld2q_f64(slot_ptr.add((k + 2) * 2));
                    let m1 = vld2q_f64(t_ptr.add((k + 2) * 2));
                    let d1 = vsubq_f64(vmulq_f64(x1.0, m1.0), vmulq_f64(x1.1, m1.1));

                    sum += vgetq_lane_f64::<0>(d0);
                    sum += vgetq_lane_f64::<1>(d0);
                    sum += vgetq_lane_f64::<0>(d1);
                    sum += vgetq_lane_f64::<1>(d1);
                    k += 4;
                }
                if !sum.is_finite() {
                    return false;
                }
                history[n] = sum;
            }
        }
        true
    }

    #[target_feature(enable = "neon")]
    pub unsafe fn av1_wiener_h_7tap_neon(
        padded_row: &[u16],
        hf: [i32; 4],
        r0: u32,
        min_val: i32,
        max_val: i32,
        out: &mut [i32],
    ) {
        let n = out.len();
        assert!(padded_row.len() >= n + 6);
        let bias = if r0 == 0 { 0 } else { 1i32 << (r0 - 1) };
        let mut col = 0;
        unsafe {
            let f0 = vdupq_n_s32(hf[0]);
            let f1 = vdupq_n_s32(hf[1]);
            let f2 = vdupq_n_s32(hf[2]);
            let f3 = vdupq_n_s32(hf[3]);
            let vbias = vdupq_n_s32(bias);
            let vshift = vdupq_n_s32(-(r0 as i32));
            let vmin = vdupq_n_s32(min_val);
            let vmax = vdupq_n_s32(max_val);
            while col + 4 <= n {
                let ptr = padded_row.as_ptr().add(col);
                let s0 = vreinterpretq_s32_u32(vmovl_u16(vld1_u16(ptr)));
                let s1 = vreinterpretq_s32_u32(vmovl_u16(vld1_u16(ptr.add(1))));
                let s2 = vreinterpretq_s32_u32(vmovl_u16(vld1_u16(ptr.add(2))));
                let s3 = vreinterpretq_s32_u32(vmovl_u16(vld1_u16(ptr.add(3))));
                let s4 = vreinterpretq_s32_u32(vmovl_u16(vld1_u16(ptr.add(4))));
                let s5 = vreinterpretq_s32_u32(vmovl_u16(vld1_u16(ptr.add(5))));
                let s6 = vreinterpretq_s32_u32(vmovl_u16(vld1_u16(ptr.add(6))));

                let s06 = vaddq_s32(s0, s6);
                let s15 = vaddq_s32(s1, s5);
                let s24 = vaddq_s32(s2, s4);

                let mut sum = vmulq_s32(s06, f0);
                sum = vmlaq_s32(sum, s15, f1);
                sum = vmlaq_s32(sum, s24, f2);
                sum = vmlaq_s32(sum, s3, f3);

                let rounded = vshlq_s32(vaddq_s32(sum, vbias), vshift);
                let clamped = vminq_s32(vmaxq_s32(rounded, vmin), vmax);
                vst1q_s32(out.as_mut_ptr().add(col), clamped);
                col += 4;
            }
            while col < n {
                let s06 = i32::from(*padded_row.get_unchecked(col)) + i32::from(*padded_row.get_unchecked(col + 6));
                let s15 = i32::from(*padded_row.get_unchecked(col + 1)) + i32::from(*padded_row.get_unchecked(col + 5));
                let s24 = i32::from(*padded_row.get_unchecked(col + 2)) + i32::from(*padded_row.get_unchecked(col + 4));
                let s3 = i32::from(*padded_row.get_unchecked(col + 3));
                let sum = hf[0] * s06 + hf[1] * s15 + hf[2] * s24 + hf[3] * s3;
                let rounded = (sum + bias) >> r0;
                *out.get_unchecked_mut(col) = rounded.clamp(min_val, max_val);
                col += 1;
            }
        }
    }

    #[target_feature(enable = "neon")]
    #[allow(clippy::too_many_arguments)]
    pub unsafe fn av1_wiener_v_7tap_neon(
        r0: &[i32], r1: &[i32], r2: &[i32], r3: &[i32], r4: &[i32], r5: &[i32], r6: &[i32],
        vf: [i32; 4],
        shift: u32,
        max_sample: i32,
        out: &mut [u16],
    ) {
        let n = out.len();
        assert!(r0.len() >= n && r1.len() >= n && r2.len() >= n && r3.len() >= n && r4.len() >= n && r5.len() >= n && r6.len() >= n);
        let bias = if shift == 0 { 0 } else { 1i32 << (shift - 1) };
        let mut col = 0;
        unsafe {
            let f0 = vdupq_n_s32(vf[0]);
            let f1 = vdupq_n_s32(vf[1]);
            let f2 = vdupq_n_s32(vf[2]);
            let f3 = vdupq_n_s32(vf[3]);
            let vbias = vdupq_n_s32(bias);
            let vshift = vdupq_n_s32(-(shift as i32));
            let vzero = vdupq_n_s32(0);
            let vmax = vdupq_n_s32(max_sample);
            while col + 4 <= n {
                let s06 = vaddq_s32(vld1q_s32(r0.as_ptr().add(col)), vld1q_s32(r6.as_ptr().add(col)));
                let s15 = vaddq_s32(vld1q_s32(r1.as_ptr().add(col)), vld1q_s32(r5.as_ptr().add(col)));
                let s24 = vaddq_s32(vld1q_s32(r2.as_ptr().add(col)), vld1q_s32(r4.as_ptr().add(col)));
                let s3 = vld1q_s32(r3.as_ptr().add(col));

                let mut sum = vmulq_s32(s06, f0);
                sum = vmlaq_s32(sum, s15, f1);
                sum = vmlaq_s32(sum, s24, f2);
                sum = vmlaq_s32(sum, s3, f3);

                let rounded = vshlq_s32(vaddq_s32(sum, vbias), vshift);
                let clamped = vminq_s32(vmaxq_s32(rounded, vzero), vmax);
                let packed = vqmovun_s32(clamped);
                vst1_u16(out.as_mut_ptr().add(col), packed);
                col += 4;
            }
            while col < n {
                let sum = vf[0] * (*r0.get_unchecked(col) + *r6.get_unchecked(col))
                    + vf[1] * (*r1.get_unchecked(col) + *r5.get_unchecked(col))
                    + vf[2] * (*r2.get_unchecked(col) + *r4.get_unchecked(col))
                    + vf[3] * *r3.get_unchecked(col);
                let rounded = (sum + bias) >> shift;
                *out.get_unchecked_mut(col) = rounded.clamp(0, max_sample) as u16;
                col += 1;
            }
        }
    }
}

#[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
mod mc_avx2 {
    use super::*;
    #[cfg(target_arch = "x86")]
    use std::arch::x86::*;
    #[cfg(target_arch = "x86_64")]
    use std::arch::x86_64::*;

    #[target_feature(enable = "avx2")]
    pub unsafe fn avc_luma_hsum_6tap_avx2(src: &[u16], out: &mut [i32]) {
        let n = out.len();
        assert!(src.len() >= n + 5);
        let mut c = 0;
        unsafe {
            let p20 = _mm256_set1_epi32(20);
            let m5  = _mm256_set1_epi32(5);
            while c + 8 <= n {
                let ptr = src.as_ptr().add(c);
                let s0 = _mm256_cvtepu16_epi32(_mm_loadu_si128(ptr.cast()));
                let s1 = _mm256_cvtepu16_epi32(_mm_loadu_si128(ptr.add(1).cast()));
                let s2 = _mm256_cvtepu16_epi32(_mm_loadu_si128(ptr.add(2).cast()));
                let s3 = _mm256_cvtepu16_epi32(_mm_loadu_si128(ptr.add(3).cast()));
                let s4 = _mm256_cvtepu16_epi32(_mm_loadu_si128(ptr.add(4).cast()));
                let s5 = _mm256_cvtepu16_epi32(_mm_loadu_si128(ptr.add(5).cast()));

                let s05 = _mm256_add_epi32(s0, s5);
                let s14 = _mm256_add_epi32(s1, s4);
                let s23 = _mm256_add_epi32(s2, s3);

                let res = _mm256_sub_epi32(_mm256_add_epi32(s05, _mm256_mullo_epi32(s23, p20)), _mm256_mullo_epi32(s14, m5));
                _mm256_storeu_si256(out.as_mut_ptr().add(c).cast(), res);
                c += 8;
            }
            while c < n {
                let v0 = i32::from(*src.get_unchecked(c));
                let v1 = i32::from(*src.get_unchecked(c + 1));
                let v2 = i32::from(*src.get_unchecked(c + 2));
                let v3 = i32::from(*src.get_unchecked(c + 3));
                let v4 = i32::from(*src.get_unchecked(c + 4));
                let v5 = i32::from(*src.get_unchecked(c + 5));
                *out.get_unchecked_mut(c) = v0 - 5 * v1 + 20 * v2 + 20 * v3 - 5 * v4 + v5;
                c += 1;
            }
        }
    }

    #[target_feature(enable = "avx2")]
    pub unsafe fn avc_luma_vsum_6tap_avx2(
        r0: &[u16], r1: &[u16], r2: &[u16], r3: &[u16], r4: &[u16], r5: &[u16],
        out: &mut [i32],
    ) {
        let n = out.len();
        assert!(r0.len() >= n && r1.len() >= n && r2.len() >= n && r3.len() >= n && r4.len() >= n && r5.len() >= n);
        let mut c = 0;
        unsafe {
            let p20 = _mm256_set1_epi32(20);
            let m5  = _mm256_set1_epi32(5);
            while c + 8 <= n {
                let s0 = _mm256_cvtepu16_epi32(_mm_loadu_si128(r0.as_ptr().add(c).cast()));
                let s1 = _mm256_cvtepu16_epi32(_mm_loadu_si128(r1.as_ptr().add(c).cast()));
                let s2 = _mm256_cvtepu16_epi32(_mm_loadu_si128(r2.as_ptr().add(c).cast()));
                let s3 = _mm256_cvtepu16_epi32(_mm_loadu_si128(r3.as_ptr().add(c).cast()));
                let s4 = _mm256_cvtepu16_epi32(_mm_loadu_si128(r4.as_ptr().add(c).cast()));
                let s5 = _mm256_cvtepu16_epi32(_mm_loadu_si128(r5.as_ptr().add(c).cast()));

                let s05 = _mm256_add_epi32(s0, s5);
                let s14 = _mm256_add_epi32(s1, s4);
                let s23 = _mm256_add_epi32(s2, s3);

                let res = _mm256_sub_epi32(_mm256_add_epi32(s05, _mm256_mullo_epi32(s23, p20)), _mm256_mullo_epi32(s14, m5));
                _mm256_storeu_si256(out.as_mut_ptr().add(c).cast(), res);
                c += 8;
            }
            while c < n {
                let v0 = i32::from(*r0.get_unchecked(c));
                let v1 = i32::from(*r1.get_unchecked(c));
                let v2 = i32::from(*r2.get_unchecked(c));
                let v3 = i32::from(*r3.get_unchecked(c));
                let v4 = i32::from(*r4.get_unchecked(c));
                let v5 = i32::from(*r5.get_unchecked(c));
                *out.get_unchecked_mut(c) = v0 - 5 * v1 + 20 * v2 + 20 * v3 - 5 * v4 + v5;
                c += 1;
            }
        }
    }

    #[target_feature(enable = "avx2")]
    pub unsafe fn avc_luma_vsum_i32_6tap_avx2(
        r0: &[i32], r1: &[i32], r2: &[i32], r3: &[i32], r4: &[i32], r5: &[i32],
        out: &mut [i32],
    ) {
        let n = out.len();
        assert!(r0.len() >= n && r1.len() >= n && r2.len() >= n && r3.len() >= n && r4.len() >= n && r5.len() >= n);
        let mut c = 0;
        unsafe {
            let p20 = _mm256_set1_epi32(20);
            let m5  = _mm256_set1_epi32(5);
            while c + 8 <= n {
                let s0 = _mm256_loadu_si256(r0.as_ptr().add(c).cast());
                let s1 = _mm256_loadu_si256(r1.as_ptr().add(c).cast());
                let s2 = _mm256_loadu_si256(r2.as_ptr().add(c).cast());
                let s3 = _mm256_loadu_si256(r3.as_ptr().add(c).cast());
                let s4 = _mm256_loadu_si256(r4.as_ptr().add(c).cast());
                let s5 = _mm256_loadu_si256(r5.as_ptr().add(c).cast());

                let s05 = _mm256_add_epi32(s0, s5);
                let s14 = _mm256_add_epi32(s1, s4);
                let s23 = _mm256_add_epi32(s2, s3);

                let res = _mm256_sub_epi32(_mm256_add_epi32(s05, _mm256_mullo_epi32(s23, p20)), _mm256_mullo_epi32(s14, m5));
                _mm256_storeu_si256(out.as_mut_ptr().add(c).cast(), res);
                c += 8;
            }
            while c < n {
                *out.get_unchecked_mut(c) = *r0.get_unchecked(c)
                    - 5 * (*r1.get_unchecked(c))
                    + 20 * (*r2.get_unchecked(c))
                    + 20 * (*r3.get_unchecked(c))
                    - 5 * (*r4.get_unchecked(c))
                    + *r5.get_unchecked(c);
                c += 1;
            }
        }
    }

    #[target_feature(enable = "avx2")]
    pub unsafe fn hevc_fir_accumulate_u16_avx2(target: &mut [i32], source: &[u16], weight: i32) {
        let n = target.len().min(source.len());
        let mut c = 0;
        unsafe {
            let w = _mm256_set1_epi32(weight);
            while c + 8 <= n {
                let s_u16 = _mm_loadu_si128(source.as_ptr().add(c).cast());
                let s_s32 = _mm256_cvtepu16_epi32(s_u16);
                let t = _mm256_loadu_si256(target.as_ptr().add(c).cast());
                let prod = _mm256_mullo_epi32(s_s32, w);
                let acc = _mm256_add_epi32(t, prod);
                _mm256_storeu_si256(target.as_mut_ptr().add(c).cast(), acc);
                c += 8;
            }
            while c < n {
                *target.get_unchecked_mut(c) += weight * i32::from(*source.get_unchecked(c));
                c += 1;
            }
        }
    }

    #[target_feature(enable = "avx2")]
    pub unsafe fn hevc_fir_accumulate_i32_avx2(target: &mut [i32], source: &[i32], weight: i32) {
        let n = target.len().min(source.len());
        let mut c = 0;
        unsafe {
            let w = _mm256_set1_epi32(weight);
            while c + 8 <= n {
                let s = _mm256_loadu_si256(source.as_ptr().add(c).cast());
                let t = _mm256_loadu_si256(target.as_ptr().add(c).cast());
                let prod = _mm256_mullo_epi32(s, w);
                let acc = _mm256_add_epi32(t, prod);
                _mm256_storeu_si256(target.as_mut_ptr().add(c).cast(), acc);
                c += 8;
            }
            while c < n {
                *target.get_unchecked_mut(c) += weight * (*source.get_unchecked(c));
                c += 1;
            }
        }
    }

    #[target_feature(enable = "avx2")]
    pub unsafe fn av1_wiener_h_7tap_avx2(
        padded_row: &[u16],
        hf: [i32; 4],
        r0: u32,
        min_val: i32,
        max_val: i32,
        out: &mut [i32],
    ) {
        let n = out.len();
        assert!(padded_row.len() >= n + 6);
        let bias = if r0 == 0 { 0 } else { 1i32 << (r0 - 1) };
        let mut col = 0;
        unsafe {
            let f0 = _mm256_set1_epi32(hf[0]);
            let f1 = _mm256_set1_epi32(hf[1]);
            let f2 = _mm256_set1_epi32(hf[2]);
            let f3 = _mm256_set1_epi32(hf[3]);
            let vbias = _mm256_set1_epi32(bias);
            let vshift = _mm_cvtsi32_si128(r0 as i32);
            let vmin = _mm256_set1_epi32(min_val);
            let vmax = _mm256_set1_epi32(max_val);
            while col + 8 <= n {
                let ptr = padded_row.as_ptr().add(col);
                let s0 = _mm256_cvtepu16_epi32(_mm_loadu_si128(ptr.cast()));
                let s1 = _mm256_cvtepu16_epi32(_mm_loadu_si128(ptr.add(1).cast()));
                let s2 = _mm256_cvtepu16_epi32(_mm_loadu_si128(ptr.add(2).cast()));
                let s3 = _mm256_cvtepu16_epi32(_mm_loadu_si128(ptr.add(3).cast()));
                let s4 = _mm256_cvtepu16_epi32(_mm_loadu_si128(ptr.add(4).cast()));
                let s5 = _mm256_cvtepu16_epi32(_mm_loadu_si128(ptr.add(5).cast()));
                let s6 = _mm256_cvtepu16_epi32(_mm_loadu_si128(ptr.add(6).cast()));

                let s06 = _mm256_add_epi32(s0, s6);
                let s15 = _mm256_add_epi32(s1, s5);
                let s24 = _mm256_add_epi32(s2, s4);

                let p0 = _mm256_mullo_epi32(s06, f0);
                let p1 = _mm256_mullo_epi32(s15, f1);
                let p2 = _mm256_mullo_epi32(s24, f2);
                let p3 = _mm256_mullo_epi32(s3, f3);
                let sum = _mm256_add_epi32(_mm256_add_epi32(p0, p1), _mm256_add_epi32(p2, p3));

                let rounded = _mm256_sra_epi32(_mm256_add_epi32(sum, vbias), vshift);
                let clamped = _mm256_min_epi32(_mm256_max_epi32(rounded, vmin), vmax);
                _mm256_storeu_si256(out.as_mut_ptr().add(col).cast(), clamped);
                col += 8;
            }
            while col < n {
                let s06 = i32::from(*padded_row.get_unchecked(col)) + i32::from(*padded_row.get_unchecked(col + 6));
                let s15 = i32::from(*padded_row.get_unchecked(col + 1)) + i32::from(*padded_row.get_unchecked(col + 5));
                let s24 = i32::from(*padded_row.get_unchecked(col + 2)) + i32::from(*padded_row.get_unchecked(col + 4));
                let s3 = i32::from(*padded_row.get_unchecked(col + 3));
                let sum = hf[0] * s06 + hf[1] * s15 + hf[2] * s24 + hf[3] * s3;
                let rounded = (sum + bias) >> r0;
                *out.get_unchecked_mut(col) = rounded.clamp(min_val, max_val);
                col += 1;
            }
        }
    }

    #[target_feature(enable = "avx2")]
    #[allow(clippy::too_many_arguments)]
    pub unsafe fn av1_wiener_v_7tap_avx2(
        r0: &[i32], r1: &[i32], r2: &[i32], r3: &[i32], r4: &[i32], r5: &[i32], r6: &[i32],
        vf: [i32; 4],
        shift: u32,
        max_sample: i32,
        out: &mut [u16],
    ) {
        let n = out.len();
        assert!(r0.len() >= n && r1.len() >= n && r2.len() >= n && r3.len() >= n && r4.len() >= n && r5.len() >= n && r6.len() >= n);
        let bias = if shift == 0 { 0 } else { 1i32 << (shift - 1) };
        let mut col = 0;
        unsafe {
            let f0 = _mm256_set1_epi32(vf[0]);
            let f1 = _mm256_set1_epi32(vf[1]);
            let f2 = _mm256_set1_epi32(vf[2]);
            let f3 = _mm256_set1_epi32(vf[3]);
            let vbias = _mm256_set1_epi32(bias);
            let vshift = _mm_cvtsi32_si128(shift as i32);
            let vzero = _mm256_setzero_si256();
            let vmax = _mm256_set1_epi32(max_sample);
            while col + 8 <= n {
                let s06 = _mm256_add_epi32(_mm256_loadu_si256(r0.as_ptr().add(col).cast()), _mm256_loadu_si256(r6.as_ptr().add(col).cast()));
                let s15 = _mm256_add_epi32(_mm256_loadu_si256(r1.as_ptr().add(col).cast()), _mm256_loadu_si256(r5.as_ptr().add(col).cast()));
                let s24 = _mm256_add_epi32(_mm256_loadu_si256(r2.as_ptr().add(col).cast()), _mm256_loadu_si256(r4.as_ptr().add(col).cast()));
                let s3 = _mm256_loadu_si256(r3.as_ptr().add(col).cast());

                let p0 = _mm256_mullo_epi32(s06, f0);
                let p1 = _mm256_mullo_epi32(s15, f1);
                let p2 = _mm256_mullo_epi32(s24, f2);
                let p3 = _mm256_mullo_epi32(s3, f3);
                let sum = _mm256_add_epi32(_mm256_add_epi32(p0, p1), _mm256_add_epi32(p2, p3));

                let rounded = _mm256_sra_epi32(_mm256_add_epi32(sum, vbias), vshift);
                let clamped = _mm256_min_epi32(_mm256_max_epi32(rounded, vzero), vmax);
                let lo = _mm256_castsi256_si128(clamped);
                let hi = _mm256_extracti128_si256::<1>(clamped);
                let packed = _mm_packus_epi32(lo, hi);
                _mm_storeu_si128(out.as_mut_ptr().add(col).cast(), packed);
                col += 8;
            }
            while col < n {
                let sum = vf[0] * (*r0.get_unchecked(col) + *r6.get_unchecked(col))
                    + vf[1] * (*r1.get_unchecked(col) + *r5.get_unchecked(col))
                    + vf[2] * (*r2.get_unchecked(col) + *r4.get_unchecked(col))
                    + vf[3] * *r3.get_unchecked(col);
                let rounded = (sum + bias) >> shift;
                *out.get_unchecked_mut(col) = rounded.clamp(0, max_sample) as u16;
                col += 1;
            }
        }
    }
}

pub fn avc_luma_hsum_6tap(src: &[u16], out: &mut [i32]) {
    #[cfg(target_arch = "aarch64")]
    {
        unsafe { mc_neon::avc_luma_hsum_6tap_neon(src, out) }
    }
    #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
    {
        if is_x86_feature_detected!("avx2") {
            unsafe { mc_avx2::avc_luma_hsum_6tap_avx2(src, out) };
        } else {
            avc_luma_hsum_6tap_scalar(src, out);
        }
    }
    #[cfg(not(any(target_arch = "aarch64", target_arch = "x86", target_arch = "x86_64")))]
    {
        avc_luma_hsum_6tap_scalar(src, out);
    }
}

pub fn avc_luma_vsum_6tap(
    r0: &[u16], r1: &[u16], r2: &[u16], r3: &[u16], r4: &[u16], r5: &[u16],
    out: &mut [i32],
) {
    #[cfg(target_arch = "aarch64")]
    {
        unsafe { mc_neon::avc_luma_vsum_6tap_neon(r0, r1, r2, r3, r4, r5, out) }
    }
    #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
    {
        if is_x86_feature_detected!("avx2") {
            unsafe { mc_avx2::avc_luma_vsum_6tap_avx2(r0, r1, r2, r3, r4, r5, out) };
        } else {
            avc_luma_vsum_6tap_scalar(r0, r1, r2, r3, r4, r5, out);
        }
    }
    #[cfg(not(any(target_arch = "aarch64", target_arch = "x86", target_arch = "x86_64")))]
    {
        avc_luma_vsum_6tap_scalar(r0, r1, r2, r3, r4, r5, out);
    }
}

pub fn avc_luma_vsum_i32_6tap(
    r0: &[i32], r1: &[i32], r2: &[i32], r3: &[i32], r4: &[i32], r5: &[i32],
    out: &mut [i32],
) {
    #[cfg(target_arch = "aarch64")]
    {
        unsafe { mc_neon::avc_luma_vsum_i32_6tap_neon(r0, r1, r2, r3, r4, r5, out) }
    }
    #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
    {
        if is_x86_feature_detected!("avx2") {
            unsafe { mc_avx2::avc_luma_vsum_i32_6tap_avx2(r0, r1, r2, r3, r4, r5, out) };
        } else {
            avc_luma_vsum_i32_6tap_scalar(r0, r1, r2, r3, r4, r5, out);
        }
    }
    #[cfg(not(any(target_arch = "aarch64", target_arch = "x86", target_arch = "x86_64")))]
    {
        avc_luma_vsum_i32_6tap_scalar(r0, r1, r2, r3, r4, r5, out);
    }
}

pub fn avc_chroma_bilinear(
    top: &[u16], bottom: &[u16], out: &mut [u16], weights: [i32; 4],
) {
    #[cfg(target_arch = "aarch64")]
    {
        unsafe { mc_neon::avc_chroma_bilinear_neon(top, bottom, out, weights) }
    }
    #[cfg(not(target_arch = "aarch64"))]
    {
        avc_chroma_bilinear_scalar(top, bottom, out, weights);
    }
}

pub fn hevc_fir_accumulate_u16(target: &mut [i32], source: &[u16], weight: i32) {
    #[cfg(target_arch = "aarch64")]
    {
        unsafe { mc_neon::hevc_fir_accumulate_u16_neon(target, source, weight) }
    }
    #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
    {
        if is_x86_feature_detected!("avx2") {
            unsafe { mc_avx2::hevc_fir_accumulate_u16_avx2(target, source, weight) };
        } else {
            hevc_fir_accumulate_u16_scalar(target, source, weight);
        }
    }
    #[cfg(not(any(target_arch = "aarch64", target_arch = "x86", target_arch = "x86_64")))]
    {
        hevc_fir_accumulate_u16_scalar(target, source, weight);
    }
}

pub fn hevc_fir_accumulate_i32(target: &mut [i32], source: &[i32], weight: i32) {
    #[cfg(target_arch = "aarch64")]
    {
        unsafe { mc_neon::hevc_fir_accumulate_i32_neon(target, source, weight) }
    }
    #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
    {
        if is_x86_feature_detected!("avx2") {
            unsafe { mc_avx2::hevc_fir_accumulate_i32_avx2(target, source, weight) };
        } else {
            hevc_fir_accumulate_i32_scalar(target, source, weight);
        }
    }
    #[cfg(not(any(target_arch = "aarch64", target_arch = "x86", target_arch = "x86_64")))]
    {
        hevc_fir_accumulate_i32_scalar(target, source, weight);
    }
}

pub fn sbr_qmf_synthesis_slot(
    slot: &[Complex64; 64],
    transform: &[[Complex64; 64]; 128],
    history: &mut [f64; 128],
) -> bool {
    #[cfg(target_arch = "aarch64")]
    {
        unsafe { mc_neon::sbr_qmf_synthesis_slot_neon(slot, transform, history) }
    }
    #[cfg(not(target_arch = "aarch64"))]
    {
        sbr_qmf_synthesis_slot_scalar(slot, transform, history)
    }
}

pub fn sbr_qmf_synthesis32_slot(
    slot: &[Complex64; 32],
    transform: &[[Complex64; 32]; 64],
    history: &mut [f64; 64],
) -> bool {
    #[cfg(target_arch = "aarch64")]
    {
        unsafe { mc_neon::sbr_qmf_synthesis32_slot_neon(slot, transform, history) }
    }
    #[cfg(not(target_arch = "aarch64"))]
    {
        sbr_qmf_synthesis32_slot_scalar(slot, transform, history)
    }
}

pub fn av1_wiener_h_7tap(
    padded_row: &[u16],
    hf: [i32; 4],
    r0: u32,
    min_val: i32,
    max_val: i32,
    out: &mut [i32],
) {
    #[cfg(target_arch = "aarch64")]
    {
        unsafe { mc_neon::av1_wiener_h_7tap_neon(padded_row, hf, r0, min_val, max_val, out) }
    }
    #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
    {
        if is_x86_feature_detected!("avx2") {
            unsafe { mc_avx2::av1_wiener_h_7tap_avx2(padded_row, hf, r0, min_val, max_val, out) };
        } else {
            av1_wiener_h_7tap_scalar(padded_row, hf, r0, min_val, max_val, out);
        }
    }
    #[cfg(not(any(target_arch = "aarch64", target_arch = "x86", target_arch = "x86_64")))]
    {
        av1_wiener_h_7tap_scalar(padded_row, hf, r0, min_val, max_val, out);
    }
}

#[allow(clippy::too_many_arguments)]
pub fn av1_wiener_v_7tap(
    r0: &[i32], r1: &[i32], r2: &[i32], r3: &[i32], r4: &[i32], r5: &[i32], r6: &[i32],
    vf: [i32; 4],
    shift: u32,
    max_sample: i32,
    out: &mut [u16],
) {
    #[cfg(target_arch = "aarch64")]
    {
        unsafe { mc_neon::av1_wiener_v_7tap_neon(r0, r1, r2, r3, r4, r5, r6, vf, shift, max_sample, out) }
    }
    #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
    {
        if is_x86_feature_detected!("avx2") {
            unsafe { mc_avx2::av1_wiener_v_7tap_avx2(r0, r1, r2, r3, r4, r5, r6, vf, shift, max_sample, out) };
        } else {
            av1_wiener_v_7tap_scalar(r0, r1, r2, r3, r4, r5, r6, vf, shift, max_sample, out);
        }
    }
    #[cfg(not(any(target_arch = "aarch64", target_arch = "x86", target_arch = "x86_64")))]
    {
        av1_wiener_v_7tap_scalar(r0, r1, r2, r3, r4, r5, r6, vf, shift, max_sample, out);
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

    #[test]
    fn interp8_vertical_matches_scalar() {
        // 42 columns exercises the 4-lane vector body plus the scalar tail.
        let n = 42;
        let rows: [Vec<i32>; 8] =
            std::array::from_fn(|t| (0..n).map(|c| ((c * (t + 3)) % 4096) as i32).collect());
        let coef = [-1, 4, -11, 40, 40, -11, 4, -1];
        let max = (1i32 << 12) - 1;
        let refs: [&[i32]; 8] = std::array::from_fn(|t| rows[t].as_slice());
        let mut out = vec![0u16; n];
        interp8_vertical(refs, &coef, max, &mut out);
        for c in 0..n {
            let mut sum = 0;
            for t in 0..8 {
                sum += coef[t] * rows[t][c];
            }
            assert_eq!(out[c], ((sum + 64) >> 7).clamp(0, max) as u16, "col {c}");
        }
    }

    fn xorshift(state: &mut u64) -> u64 {
        *state ^= *state << 13;
        *state ^= *state >> 7;
        *state ^= *state << 17;
        *state
    }

    #[cfg(target_arch = "aarch64")]
    #[test]
    fn vp9_filter_batch4_neon_matches_scalar() {
        let mut state = 0x2545F491_4F6CDD1Du64;
        for depth in [8u8, 10, 12] {
            let mask = (1u16 << depth) - 1;
            let bd = depth - 8;
            for width in [4usize, 8, 16] {
                for level in [0u8, 1, 16, 33, 63] {
                    for sharpness in [0u8, 4, 7] {
                        // Random lines exercise the masked, non-flat and HEV paths.
                        let mut rnd = [[0u16; 16]; 4];
                        for line in rnd.iter_mut() {
                            for v in line.iter_mut() {
                                *v = (xorshift(&mut state) as u16) & mask;
                            }
                        }
                        // Flat profiles: a near-constant plateau forces the box-blur
                        // branch. Deltas stay within 1<<bd so `flat`/`flat2` hold.
                        let mut flat = [[0u16; 16]; 4];
                        if width >= 8 {
                            let tol = 1i32 << bd;
                            let base = 1i32 << (depth - 1);
                            for line in flat.iter_mut() {
                                for v in line.iter_mut() {
                                    let d = (xorshift(&mut state) as i32) % tol;
                                    *v = (base + d) as u16;
                                }
                            }
                        }
                        for lines in [rnd, flat] {
                            let got = unsafe {
                                vp9_filter_batch4_neon(lines, depth, width, level, sharpness)
                            };
                            let exp =
                                lines.map(|s| vp9_filter_lane(s, depth, width, level, sharpness));
                            assert_eq!(got, exp, "d{depth} w{width} l{level} s{sharpness}");
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn vp9_filter_batch4_scalar_matches_lane() {
        // Exercises the public dispatch against the same scalar oracle.
        let mut state = 0x9E3779B9_7F4A7C15u64;
        for depth in [8u8, 10, 12] {
            let mask = (1u16 << depth) - 1;
            for width in [4usize, 8, 16] {
                for level in [0u8, 1, 16, 33, 63] {
                    let mut lines = [[0u16; 16]; 4];
                    for line in lines.iter_mut() {
                        for v in line.iter_mut() {
                            *v = (xorshift(&mut state) as u16) & mask;
                        }
                    }
                    let got = vp9_filter_batch4(lines, depth, width, level, 4);
                    let exp = lines.map(|s| vp9_filter_lane(s, depth, width, level, 4));
                    assert_eq!(got, exp);
                }
            }
        }
    }

    #[test]
    fn hevc_deblock_luma_batch4_scalar_matches_reference() {
        let mut state = 0xABCDEF01_23456789u64;
        for depth in [8u8, 10] {
            let mask = (1u16 << depth) - 1;
            for tc in [0i32, 1, 4, 16] {
                for filter_idx in 0..3 {
                    let filter = match filter_idx {
                        0 => HevcLumaFilter::Off,
                        1 => HevcLumaFilter::Strong,
                        _ => HevcLumaFilter::Weak {
                            second: [true, true],
                        },
                    };
                    for _ in 0..20 {
                        let p: [[u16; 4]; 4] = std::array::from_fn(|_| {
                            std::array::from_fn(|_| (xorshift(&mut state) as u16) & mask)
                        });
                        let q: [[u16; 4]; 4] = std::array::from_fn(|_| {
                            std::array::from_fn(|_| (xorshift(&mut state) as u16) & mask)
                        });
                        let enabled = [[true, true]; 4];
                        let (got_p, got_q) =
                            hevc_deblock_luma_batch4_scalar(p, q, tc, depth, filter, enabled);
                        // Verify against per-line scalar
                        for line in 0..4 {
                            let a = p[line].map(i32::from);
                            let b = q[line].map(i32::from);
                            let mut exp_a = a;
                            let mut exp_b = b;
                            let max = (1i32 << depth) - 1;
                            match filter {
                                HevcLumaFilter::Off => {}
                                HevcLumaFilter::Strong => {
                                    for side in 0..2 {
                                        let (a, b, out) = if side == 0 {
                                            (a, b, &mut exp_a)
                                        } else {
                                            (b, a, &mut exp_b)
                                        };
                                        let values = [
                                            (a[2] + 2 * a[1] + 2 * a[0] + 2 * b[0] + b[1] + 4) >> 3,
                                            (a[2] + a[1] + a[0] + b[0] + 2) >> 2,
                                            (2 * a[3] + 3 * a[2] + a[1] + a[0] + b[0] + 4) >> 3,
                                        ];
                                        for i in 0..3 {
                                            out[i] = values[i].clamp(a[i] - 2 * tc, a[i] + 2 * tc);
                                        }
                                    }
                                }
                                HevcLumaFilter::Weak { second } => {
                                    let delta = (9 * (b[0] - a[0]) - 3 * (b[1] - a[1]) + 8) >> 4;
                                    if delta.abs() < 10 * tc {
                                        let delta = delta.clamp(-tc, tc);
                                        exp_a[0] = (a[0] + delta).clamp(0, max);
                                        exp_b[0] = (b[0] - delta).clamp(0, max);
                                        if second[0] {
                                            let d = ((((a[2] + a[0] + 1) >> 1) - a[1] + delta)
                                                >> 1)
                                                .clamp(-(tc >> 1), tc >> 1);
                                            exp_a[1] = (a[1] + d).clamp(0, max);
                                        }
                                        if second[1] {
                                            let d = ((((b[2] + b[0] + 1) >> 1) - b[1] - delta)
                                                >> 1)
                                                .clamp(-(tc >> 1), tc >> 1);
                                            exp_b[1] = (b[1] + d).clamp(0, max);
                                        }
                                    }
                                }
                            }
                            assert_eq!(got_p[line], exp_a.map(|x| x as u16), "line {line} p");
                            assert_eq!(got_q[line], exp_b.map(|x| x as u16), "line {line} q");
                        }
                    }
                }
            }
        }
    }

    #[cfg(target_arch = "aarch64")]
    #[test]
    fn hevc_deblock_luma_batch4_neon_matches_scalar() {
        let mut state = 0xFEDCBA98_76543210u64;
        for depth in [8u8, 10] {
            let mask = (1u16 << depth) - 1;
            for tc in [0i32, 1, 4, 16] {
                for filter_idx in 0..3 {
                    let filter = match filter_idx {
                        0 => HevcLumaFilter::Off,
                        1 => HevcLumaFilter::Strong,
                        _ => HevcLumaFilter::Weak {
                            second: [true, true],
                        },
                    };
                    for _ in 0..10 {
                        let p: [[u16; 4]; 4] = std::array::from_fn(|_| {
                            std::array::from_fn(|_| (xorshift(&mut state) as u16) & mask)
                        });
                        let q: [[u16; 4]; 4] = std::array::from_fn(|_| {
                            std::array::from_fn(|_| (xorshift(&mut state) as u16) & mask)
                        });
                        let enabled = [[true, true]; 4];
                        let (got_p, got_q) = unsafe {
                            hevc_deblock_luma_batch4_neon(p, q, tc, depth, filter, enabled)
                        };
                        let (exp_p, exp_q) =
                            hevc_deblock_luma_batch4_scalar(p, q, tc, depth, filter, enabled);
                        assert_eq!(got_p, exp_p);
                        assert_eq!(got_q, exp_q);
                    }
                }
            }
        }
    }

    #[cfg(target_arch = "aarch64")]
    #[test]
    fn av1_cdef_block_neon_matches_scalar() {
        let mut state = 0x0EED_1234_5678_9ABCu64;
        let (stride, rows) = (40usize, 40usize);
        let mut src = vec![0u16; stride * rows];
        for v in src.iter_mut() {
            *v = (xorshift(&mut state) & 0xFFF) as u16;
        }
        let shapes = [
            (2, 2, 8, 8),
            (2, 3, 4, 4),
            (8, 2, 16, 5),
            (2, 2, 13, 7),
            (30, 30, 8, 8),
            (2, 2, 4, 16),
            (3, 5, 5, 4),
            (2, 2, 12, 4),
            (5, 2, 4, 2),
        ];
        for &(x, y, w, h) in &shapes {
            for trial in 0..40 {
                let count = (xorshift(&mut state) % 13) as usize;
                let taps: Vec<CdefTap> = (0..count)
                    .map(|i| CdefTap {
                        dx: (xorshift(&mut state) % 5) as i32 - 2,
                        dy: (xorshift(&mut state) % 5) as i32 - 2,
                        threshold: (xorshift(&mut state) % 241) as i16,
                        shift: (xorshift(&mut state) % 20) as i16,
                        weight: [1, 2, 3, 4][(trial + i) % 4] as i16,
                    })
                    .collect();
                let mut neon = src.clone();
                let mut scalar = src.clone();
                // SAFETY: every block keeps its two-sample halo inside the plane.
                unsafe { av1_cdef_block_neon(&src, &mut neon, stride, x, y, w, h, &taps) };
                av1_cdef_block_scalar(&src, &mut scalar, stride, x, y, w, h, &taps);
                assert_eq!(neon, scalar, "x={x} y={y} w={w} h={h} taps={count}");
            }
        }
    }

    #[test]
    fn hevc_inverse_dct_matches_dense_matrix_and_scalar() {
        let mut state = 17u32;
        for n in [4usize, 8, 16, 32] {
            for case in 0..128 {
                let mut input = vec![0i32; n];
                for (k, v) in input.iter_mut().enumerate() {
                    state = state.wrapping_mul(1664525).wrapping_add(1013904223);
                    *v = if case < n && k != case {
                        0
                    } else {
                        (state >> 16) as i16 as i32
                    };
                }
                let mut actual = vec![0i32; n];
                let mut expected_scalar = vec![0i32; n];
                hevc_inverse_dct(&input, &mut actual);
                hevc_inverse_scalar(&input, &mut expected_scalar);
                assert_eq!(actual, expected_scalar, "n={n} case={case} actual vs scalar");

                for x in 0..n {
                    let expected_dense: i64 = (0..n)
                        .map(|k| i64::from(input[k]) * i64::from(HEVC_DCT[k * (32 / n)][x]))
                        .sum();
                    assert_eq!(
                        i64::from(actual[x]),
                        expected_dense,
                        "n={n} case={case} x={x} actual vs dense"
                    );
                }
            }
        }
    }

    #[test]
    fn hevc_inverse_dct_dc_only() {
        for n in [4usize, 8, 16, 32] {
            let mut input = vec![0i32; n];
            input[0] = 42;
            let mut actual = vec![0i32; n];
            hevc_inverse_dct(&input, &mut actual);
            assert_eq!(actual, vec![42 * 64; n]);
        }
    }

    #[cfg(target_arch = "aarch64")]
    #[test]
    fn hevc_inverse_neon_matches_scalar() {
        let mut state = 0x1234_5678_9ABC_DEF0u64;
        for trial in 0..200 {
            // Test 8
            let input8: [i32; 8] = std::array::from_fn(|_| (xorshift(&mut state) as i16) as i32);
            let mut out8_neon = [0i32; 8];
            let mut out8_scalar = [0i32; 8];
            unsafe { hevc_neon::hevc_inverse8_neon(&input8, &mut out8_neon) };
            hevc_inverse8_scalar_arr(&input8, &mut out8_scalar);
            assert_eq!(out8_neon, out8_scalar, "trial {trial} size 8");

            // Test 16
            let input16: [i32; 16] = std::array::from_fn(|_| (xorshift(&mut state) as i16) as i32);
            let mut out16_neon = [0i32; 16];
            let mut out16_scalar = [0i32; 16];
            unsafe { hevc_neon::hevc_inverse16_neon(&input16, &mut out16_neon) };
            hevc_inverse16_scalar_arr(&input16, &mut out16_scalar);
            assert_eq!(out16_neon, out16_scalar, "trial {trial} size 16");

            // Test 32
            let input32: [i32; 32] = std::array::from_fn(|_| (xorshift(&mut state) as i16) as i32);
            let mut out32_neon = [0i32; 32];
            let mut out32_scalar = [0i32; 32];
            unsafe { hevc_neon::hevc_inverse32_neon(&input32, &mut out32_neon) };
            hevc_inverse32_scalar_arr(&input32, &mut out32_scalar);
            assert_eq!(out32_neon, out32_scalar, "trial {trial} size 32");
        }
    }

    #[test]
    fn avc_hsum_and_vsum_match_scalar() {
        let mut state = 0xDEAD_BEEF_CAFE_BABE_u64;
        for len in [4, 8, 12, 16, 24, 32] {
            let src: Vec<u16> = (0..len + 10).map(|_| (xorshift(&mut state) % 4096) as u16).collect();
            let mut out_simd = vec![0i32; len];
            let mut out_scalar = vec![0i32; len];
            avc_luma_hsum_6tap(&src, &mut out_simd);
            avc_luma_hsum_6tap_scalar(&src, &mut out_scalar);
            assert_eq!(out_simd, out_scalar, "avc_luma_hsum_6tap len={len}");

            let rows: Vec<Vec<u16>> = (0..6)
                .map(|_| (0..len).map(|_| (xorshift(&mut state) % 4096) as u16).collect())
                .collect();
            let mut vout_simd = vec![0i32; len];
            let mut vout_scalar = vec![0i32; len];
            avc_luma_vsum_6tap(&rows[0], &rows[1], &rows[2], &rows[3], &rows[4], &rows[5], &mut vout_simd);
            avc_luma_vsum_6tap_scalar(&rows[0], &rows[1], &rows[2], &rows[3], &rows[4], &rows[5], &mut vout_scalar);
            assert_eq!(vout_simd, vout_scalar, "avc_luma_vsum_6tap len={len}");

            let rows_i32: Vec<Vec<i32>> = (0..6)
                .map(|_| (0..len).map(|_| (xorshift(&mut state) as i16) as i32).collect())
                .collect();
            let mut vi_simd = vec![0i32; len];
            let mut vi_scalar = vec![0i32; len];
            avc_luma_vsum_i32_6tap(&rows_i32[0], &rows_i32[1], &rows_i32[2], &rows_i32[3], &rows_i32[4], &rows_i32[5], &mut vi_simd);
            avc_luma_vsum_i32_6tap_scalar(&rows_i32[0], &rows_i32[1], &rows_i32[2], &rows_i32[3], &rows_i32[4], &rows_i32[5], &mut vi_scalar);
            assert_eq!(vi_simd, vi_scalar, "avc_luma_vsum_i32_6tap len={len}");
        }
    }

    #[test]
    fn avc_chroma_bilinear_matches_scalar() {
        let mut state = 0xCAFE_BABE_0123_4567_u64;
        for len in [4, 8, 16] {
            let top: Vec<u16> = (0..len + 1).map(|_| (xorshift(&mut state) % 1024) as u16).collect();
            let bottom: Vec<u16> = (0..len + 1).map(|_| (xorshift(&mut state) % 1024) as u16).collect();
            let weights = [35, 21, 15, 9];
            let mut out_simd = vec![0u16; len];
            let mut out_scalar = vec![0u16; len];
            avc_chroma_bilinear(&top, &bottom, &mut out_simd, weights);
            avc_chroma_bilinear_scalar(&top, &bottom, &mut out_scalar, weights);
            assert_eq!(out_simd, out_scalar, "avc_chroma_bilinear len={len}");
        }
    }

    #[test]
    fn hevc_fir_accumulate_matches_scalar() {
        let mut state = 0x9876_5432_10FE_DCBA_u64;
        for len in [4, 8, 16, 32, 64] {
            let src16: Vec<u16> = (0..len).map(|_| (xorshift(&mut state) % 2048) as u16).collect();
            let src32: Vec<i32> = (0..len).map(|_| (xorshift(&mut state) as i16) as i32).collect();
            let weight = (xorshift(&mut state) % 129) as i32 - 64;

            let mut target_u16_simd = vec![0i32; len];
            let mut target_u16_scalar = vec![0i32; len];
            hevc_fir_accumulate_u16(&mut target_u16_simd, &src16, weight);
            hevc_fir_accumulate_u16_scalar(&mut target_u16_scalar, &src16, weight);
            assert_eq!(target_u16_simd, target_u16_scalar, "hevc_fir_accumulate_u16 len={len}");

            let mut target_i32_simd = vec![0i32; len];
            let mut target_i32_scalar = vec![0i32; len];
            hevc_fir_accumulate_i32(&mut target_i32_simd, &src32, weight);
            hevc_fir_accumulate_i32_scalar(&mut target_i32_scalar, &src32, weight);
            assert_eq!(target_i32_simd, target_i32_scalar, "hevc_fir_accumulate_i32 len={len}");
        }
    }

    #[test]
    fn sbr_qmf_synthesis_slot_matches_scalar() {
        let mut state = 0x5555_AAAA_1234_5678_u64;
        let slot: [Complex64; 64] = std::array::from_fn(|_| Complex64 {
            re: (xorshift(&mut state) as i32 as f64) / (i32::MAX as f64),
            im: (xorshift(&mut state) as i32 as f64) / (i32::MAX as f64),
        });
        let transform: [[Complex64; 64]; 128] = std::array::from_fn(|_| {
            std::array::from_fn(|_| Complex64 {
                re: (xorshift(&mut state) as i32 as f64) / (i32::MAX as f64),
                im: (xorshift(&mut state) as i32 as f64) / (i32::MAX as f64),
            })
        });
        let mut history_simd = [0.0f64; 128];
        let mut history_scalar = [0.0f64; 128];
        let ok_simd = sbr_qmf_synthesis_slot(&slot, &transform, &mut history_simd);
        let ok_scalar = sbr_qmf_synthesis_slot_scalar(&slot, &transform, &mut history_scalar);
        assert!(ok_simd);
        assert!(ok_scalar);
        for n in 0..128 {
            let diff = (history_simd[n] - history_scalar[n]).abs();
            let max_val = history_scalar[n].abs().max(1.0);
            assert!(diff / max_val < 1e-12, "n={n} simd={} scalar={} diff={}", history_simd[n], history_scalar[n], diff);
        }
    }

    #[test]
    fn sbr_qmf_synthesis32_slot_matches_scalar() {
        let mut state = 0x3333_7777_9999_BBBB_u64;
        let slot: [Complex64; 32] = std::array::from_fn(|_| Complex64 {
            re: (xorshift(&mut state) as i32 as f64) / (i32::MAX as f64),
            im: (xorshift(&mut state) as i32 as f64) / (i32::MAX as f64),
        });
        let transform: [[Complex64; 32]; 64] = std::array::from_fn(|_| {
            std::array::from_fn(|_| Complex64 {
                re: (xorshift(&mut state) as i32 as f64) / (i32::MAX as f64),
                im: (xorshift(&mut state) as i32 as f64) / (i32::MAX as f64),
            })
        });
        let mut history_simd = [0.0f64; 64];
        let mut history_scalar = [0.0f64; 64];
        assert!(sbr_qmf_synthesis32_slot(&slot, &transform, &mut history_simd));
        assert!(sbr_qmf_synthesis32_slot_scalar(&slot, &transform, &mut history_scalar));
        assert_eq!(history_simd, history_scalar);
    }

    #[test]
    fn av1_wiener_7tap_matches_scalar() {
        let mut state = 0x1357_9BDF_2468_ACE0_u64;
        for depth in [8u8, 10, 12] {
            let r0 = if depth == 12 { 5u32 } else { 3u32 };
            let r1 = 14 - r0;
            let offset = 1i32 << (u32::from(depth) + 7 - r0 - 1);
            let limit = (1i32 << (u32::from(depth) + 1 + 7 - r0)) - 1;
            let min_val = -offset;
            let max_val = limit - offset;
            let max_sample = (1i32 << depth) - 1;
            let hf = [-5, 12, -20, 154];
            let vf = [-3, 9, -18, 152];
            for len in [1, 4, 7, 16, 33, 64] {
                let padded: Vec<u16> = (0..len + 6)
                    .map(|_| (xorshift(&mut state) % (max_sample as u64 + 1)) as u16)
                    .collect();
                let mut h_simd = vec![0i32; len];
                let mut h_scalar = vec![0i32; len];
                av1_wiener_h_7tap(&padded, hf, r0, min_val, max_val, &mut h_simd);
                av1_wiener_h_7tap_scalar(&padded, hf, r0, min_val, max_val, &mut h_scalar);
                assert_eq!(h_simd, h_scalar, "av1_wiener_h_7tap depth={depth} len={len}");

                let rows: [Vec<i32>; 7] = std::array::from_fn(|_| {
                    (0..len)
                        .map(|_| {
                            let span = (max_val - min_val + 1) as u64;
                            min_val + (xorshift(&mut state) % span) as i32
                        })
                        .collect()
                });
                let mut v_simd = vec![0u16; len];
                let mut v_scalar = vec![0u16; len];
                av1_wiener_v_7tap(
                    &rows[0], &rows[1], &rows[2], &rows[3], &rows[4], &rows[5], &rows[6],
                    vf, r1, max_sample, &mut v_simd,
                );
                av1_wiener_v_7tap_scalar(
                    &rows[0], &rows[1], &rows[2], &rows[3], &rows[4], &rows[5], &rows[6],
                    vf, r1, max_sample, &mut v_scalar,
                );
                assert_eq!(v_simd, v_scalar, "av1_wiener_v_7tap depth={depth} len={len}");
            }
        }
    }
}

