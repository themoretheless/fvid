//! Safe SIMD kernels for CELT (float) and SILK (fixed-point) Opus hot paths.
//!
//! All `unsafe` architecture intrinsics (`aarch64` NEON and `x86_64` SSE2/AVX/AVX2/FMA)
//! live inside `fvid-cpu` behind bounds-checked slice interfaces so `fvid-opus`
//! preserves `#![forbid(unsafe_code)]` with zero runtime safety compromise.

#[cfg(target_arch = "x86_64")]
use std::sync::atomic::{AtomicU8, Ordering};

#[cfg(target_arch = "x86_64")]
struct FeatureCache(AtomicU8);

#[cfg(target_arch = "x86_64")]
impl FeatureCache {
    const fn new() -> Self {
        Self(AtomicU8::new(0))
    }

    #[inline(always)]
    fn get(&self, probe: impl FnOnce() -> bool) -> bool {
        match self.0.load(Ordering::Relaxed) {
            1 => true,
            2 => false,
            _ => {
                let on = probe();
                self.0.store(if on { 1 } else { 2 }, Ordering::Relaxed);
                on
            }
        }
    }
}

#[cfg(target_arch = "x86_64")]
#[inline(always)]
fn have_avx() -> bool {
    static CACHE: FeatureCache = FeatureCache::new();
    CACHE.get(|| std::arch::is_x86_feature_detected!("avx"))
}

#[cfg(target_arch = "x86_64")]
#[inline(always)]
fn have_avx_fma() -> bool {
    static CACHE: FeatureCache = FeatureCache::new();
    CACHE.get(|| {
        std::arch::is_x86_feature_detected!("avx") && std::arch::is_x86_feature_detected!("fma")
    })
}

#[cfg(target_arch = "x86_64")]
#[inline(always)]
fn have_avx2() -> bool {
    static CACHE: FeatureCache = FeatureCache::new();
    CACHE.get(|| std::arch::is_x86_feature_detected!("avx2"))
}

#[cfg(target_arch = "x86_64")]
#[inline(always)]
fn have_avx2_fma() -> bool {
    static CACHE: FeatureCache = FeatureCache::new();
    CACHE.get(|| {
        std::arch::is_x86_feature_detected!("avx2") && std::arch::is_x86_feature_detected!("fma")
    })
}

// ---------------------------------------------------------------------------
// Complex sample representation for CELT KissFFT & MDCT
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, Default, PartialEq)]
#[repr(C)]
pub struct KissCpx {
    pub r: f32,
    pub i: f32,
}

impl KissCpx {
    #[inline(always)]
    pub const fn new(r: f32, i: f32) -> Self {
        Self { r, i }
    }
}

#[inline(always)]
fn c_mul(a: &KissCpx, b: &KissCpx) -> KissCpx {
    KissCpx {
        r: a.r * b.r - a.i * b.i,
        i: a.r * b.i + a.i * b.r,
    }
}

#[inline(always)]
fn c_sub(a: &KissCpx, b: &KissCpx) -> KissCpx {
    KissCpx {
        r: a.r - b.r,
        i: a.i - b.i,
    }
}

#[inline(always)]
fn c_add(a: &KissCpx, b: &KissCpx) -> KissCpx {
    KissCpx {
        r: a.r + b.r,
        i: a.i + b.i,
    }
}

// ---------------------------------------------------------------------------
// CELT Pitch & Inner Products
// ---------------------------------------------------------------------------

#[inline]
pub fn inner_prod(x: &[f32], y: &[f32], n: usize) -> f32 {
    #[cfg(target_arch = "aarch64")]
    {
        return unsafe { inner_prod_neon(x, y, n) };
    }
    #[cfg(target_arch = "x86_64")]
    {
        if have_avx_fma() {
            return unsafe { inner_prod_avx(x, y, n) };
        }
        return unsafe { inner_prod_sse2(x, y, n) };
    }
    #[allow(unreachable_code)]
    inner_prod_scalar(x, y, n)
}

#[inline]
fn inner_prod_scalar(x: &[f32], y: &[f32], n: usize) -> f32 {
    let x = &x[..n];
    let y = &y[..n];
    let mut sum = 0.0f32;
    for i in 0..n {
        sum += x[i] * y[i];
    }
    sum
}

#[cfg(target_arch = "aarch64")]
#[target_feature(enable = "neon")]
#[inline]
unsafe fn inner_prod_neon(x: &[f32], y: &[f32], n: usize) -> f32 {
    use std::arch::aarch64::*;
    let x = &x[..n];
    let y = &y[..n];
    let mut xy0 = vdupq_n_f32(0.0);
    let mut xy1 = vdupq_n_f32(0.0);
    let mut i = 0usize;
    while i + 8 <= n {
        unsafe {
            let x0 = vld1q_f32(x.as_ptr().add(i));
            let y0 = vld1q_f32(y.as_ptr().add(i));
            xy0 = vfmaq_f32(xy0, x0, y0);
            let x1 = vld1q_f32(x.as_ptr().add(i + 4));
            let y1 = vld1q_f32(y.as_ptr().add(i + 4));
            xy1 = vfmaq_f32(xy1, x1, y1);
        }
        i += 8;
    }
    if i + 4 <= n {
        unsafe {
            let x0 = vld1q_f32(x.as_ptr().add(i));
            let y0 = vld1q_f32(y.as_ptr().add(i));
            xy0 = vfmaq_f32(xy0, x0, y0);
        }
        i += 4;
    }
    let mut sum = vaddvq_f32(vaddq_f32(xy0, xy1));
    for j in i..n {
        sum += x[j] * y[j];
    }
    sum
}

#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "sse2")]
unsafe fn inner_prod_sse2(x: &[f32], y: &[f32], n: usize) -> f32 {
    use std::arch::x86_64::*;
    let x = &x[..n];
    let y = &y[..n];
    let mut sum0 = _mm_setzero_ps();
    let mut sum1 = _mm_setzero_ps();
    let mut i = 0usize;
    while i + 8 <= n {
        unsafe {
            let x0 = _mm_loadu_ps(x.as_ptr().add(i));
            let y0 = _mm_loadu_ps(y.as_ptr().add(i));
            sum0 = _mm_add_ps(sum0, _mm_mul_ps(x0, y0));
            let x1 = _mm_loadu_ps(x.as_ptr().add(i + 4));
            let y1 = _mm_loadu_ps(y.as_ptr().add(i + 4));
            sum1 = _mm_add_ps(sum1, _mm_mul_ps(x1, y1));
        }
        i += 8;
    }
    if i + 4 <= n {
        unsafe {
            let x0 = _mm_loadu_ps(x.as_ptr().add(i));
            let y0 = _mm_loadu_ps(y.as_ptr().add(i));
            sum0 = _mm_add_ps(sum0, _mm_mul_ps(x0, y0));
        }
        i += 4;
    }
    let sum = _mm_add_ps(sum0, sum1);
    let tmp = _mm_movehl_ps(sum, sum);
    let sum = _mm_add_ps(sum, tmp);
    let tmp2 = _mm_shuffle_ps(sum, sum, 0x55);
    let mut result = _mm_cvtss_f32(_mm_add_ss(sum, tmp2));
    while i < n {
        result += x[i] * y[i];
        i += 1;
    }
    result
}

#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx,fma")]
unsafe fn inner_prod_avx(x: &[f32], y: &[f32], n: usize) -> f32 {
    use std::arch::x86_64::*;
    let x = &x[..n];
    let y = &y[..n];
    let mut acc0 = _mm256_setzero_ps();
    let mut acc1 = _mm256_setzero_ps();
    let mut i = 0usize;
    while i + 16 <= n {
        unsafe {
            let x0 = _mm256_loadu_ps(x.as_ptr().add(i));
            let y0 = _mm256_loadu_ps(y.as_ptr().add(i));
            acc0 = _mm256_fmadd_ps(x0, y0, acc0);
            let x1 = _mm256_loadu_ps(x.as_ptr().add(i + 8));
            let y1 = _mm256_loadu_ps(y.as_ptr().add(i + 8));
            acc1 = _mm256_fmadd_ps(x1, y1, acc1);
        }
        i += 16;
    }
    while i + 8 <= n {
        unsafe {
            let x0 = _mm256_loadu_ps(x.as_ptr().add(i));
            let y0 = _mm256_loadu_ps(y.as_ptr().add(i));
            acc0 = _mm256_fmadd_ps(x0, y0, acc0);
        }
        i += 8;
    }
    let acc = _mm256_add_ps(acc0, acc1);
    let hi = _mm256_extractf128_ps(acc, 1);
    let lo = _mm256_castps256_ps128(acc);
    let sum4 = _mm_add_ps(lo, hi);
    let tmp = _mm_movehl_ps(sum4, sum4);
    let sum2 = _mm_add_ps(sum4, tmp);
    let tmp2 = _mm_shuffle_ps(sum2, sum2, 0x55);
    let mut result = _mm_cvtss_f32(_mm_add_ss(sum2, tmp2));
    while i < n {
        result += x[i] * y[i];
        i += 1;
    }
    result
}

#[inline]
pub fn dual_inner_prod(x: &[f32], y1: &[f32], y2: &[f32], n: usize) -> (f32, f32) {
    #[cfg(target_arch = "aarch64")]
    {
        return unsafe { dual_inner_prod_neon(x, y1, y2, n) };
    }
    #[cfg(target_arch = "x86_64")]
    {
        if have_avx_fma() {
            return unsafe { dual_inner_prod_avx(x, y1, y2, n) };
        }
        return unsafe { dual_inner_prod_sse2(x, y1, y2, n) };
    }
    #[allow(unreachable_code)]
    dual_inner_prod_scalar(x, y1, y2, n)
}

#[inline]
fn dual_inner_prod_scalar(x: &[f32], y1: &[f32], y2: &[f32], n: usize) -> (f32, f32) {
    let x = &x[..n];
    let y1 = &y1[..n];
    let y2 = &y2[..n];
    let mut xy1 = 0.0f32;
    let mut xy2 = 0.0f32;
    for i in 0..n {
        xy1 += x[i] * y1[i];
        xy2 += x[i] * y2[i];
    }
    (xy1, xy2)
}

#[cfg(target_arch = "aarch64")]
#[target_feature(enable = "neon")]
#[inline]
unsafe fn dual_inner_prod_neon(x: &[f32], y1: &[f32], y2: &[f32], n: usize) -> (f32, f32) {
    use std::arch::aarch64::*;
    let x = &x[..n];
    let y1 = &y1[..n];
    let y2 = &y2[..n];
    let mut xy1 = vdupq_n_f32(0.0);
    let mut xy2 = vdupq_n_f32(0.0);
    let mut i = 0usize;
    while i + 8 <= n {
        unsafe {
            let x0 = vld1q_f32(x.as_ptr().add(i));
            let x4 = vld1q_f32(x.as_ptr().add(i + 4));
            let y1_0 = vld1q_f32(y1.as_ptr().add(i));
            let y1_4 = vld1q_f32(y1.as_ptr().add(i + 4));
            let y2_0 = vld1q_f32(y2.as_ptr().add(i));
            let y2_4 = vld1q_f32(y2.as_ptr().add(i + 4));
            xy1 = vfmaq_f32(xy1, x0, y1_0);
            xy2 = vfmaq_f32(xy2, x0, y2_0);
            xy1 = vfmaq_f32(xy1, x4, y1_4);
            xy2 = vfmaq_f32(xy2, x4, y2_4);
        }
        i += 8;
    }
    if i + 4 <= n {
        unsafe {
            let x0 = vld1q_f32(x.as_ptr().add(i));
            let y1_0 = vld1q_f32(y1.as_ptr().add(i));
            let y2_0 = vld1q_f32(y2.as_ptr().add(i));
            xy1 = vfmaq_f32(xy1, x0, y1_0);
            xy2 = vfmaq_f32(xy2, x0, y2_0);
        }
        i += 4;
    }
    let mut s1 = vaddvq_f32(xy1);
    let mut s2 = vaddvq_f32(xy2);
    for j in i..n {
        s1 += x[j] * y1[j];
        s2 += x[j] * y2[j];
    }
    (s1, s2)
}

#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "sse2")]
unsafe fn dual_inner_prod_sse2(x: &[f32], y1: &[f32], y2: &[f32], n: usize) -> (f32, f32) {
    use std::arch::x86_64::*;
    let x = &x[..n];
    let y1 = &y1[..n];
    let y2 = &y2[..n];
    let mut xy1 = _mm_setzero_ps();
    let mut xy2 = _mm_setzero_ps();
    let mut i = 0usize;
    while i + 4 <= n {
        unsafe {
            let xi = _mm_loadu_ps(x.as_ptr().add(i));
            let y1i = _mm_loadu_ps(y1.as_ptr().add(i));
            let y2i = _mm_loadu_ps(y2.as_ptr().add(i));
            xy1 = _mm_add_ps(xy1, _mm_mul_ps(xi, y1i));
            xy2 = _mm_add_ps(xy2, _mm_mul_ps(xi, y2i));
        }
        i += 4;
    }
    let tmp1 = _mm_movehl_ps(xy1, xy1);
    let xy1 = _mm_add_ps(xy1, tmp1);
    let mut s1 = _mm_cvtss_f32(_mm_add_ss(xy1, _mm_shuffle_ps(xy1, xy1, 0x55)));

    let tmp2 = _mm_movehl_ps(xy2, xy2);
    let xy2 = _mm_add_ps(xy2, tmp2);
    let mut s2 = _mm_cvtss_f32(_mm_add_ss(xy2, _mm_shuffle_ps(xy2, xy2, 0x55)));

    while i < n {
        s1 += x[i] * y1[i];
        s2 += x[i] * y2[i];
        i += 1;
    }
    (s1, s2)
}

#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx,fma")]
unsafe fn dual_inner_prod_avx(x: &[f32], y1: &[f32], y2: &[f32], n: usize) -> (f32, f32) {
    use std::arch::x86_64::*;
    let x = &x[..n];
    let y1 = &y1[..n];
    let y2 = &y2[..n];
    let mut acc1 = _mm256_setzero_ps();
    let mut acc2 = _mm256_setzero_ps();
    let mut acc1b = _mm256_setzero_ps();
    let mut acc2b = _mm256_setzero_ps();
    let mut i = 0usize;
    while i + 16 <= n {
        unsafe {
            let xv0 = _mm256_loadu_ps(x.as_ptr().add(i));
            let xv1 = _mm256_loadu_ps(x.as_ptr().add(i + 8));
            let y1v0 = _mm256_loadu_ps(y1.as_ptr().add(i));
            let y2v0 = _mm256_loadu_ps(y2.as_ptr().add(i));
            let y1v1 = _mm256_loadu_ps(y1.as_ptr().add(i + 8));
            let y2v1 = _mm256_loadu_ps(y2.as_ptr().add(i + 8));
            acc1 = _mm256_fmadd_ps(xv0, y1v0, acc1);
            acc2 = _mm256_fmadd_ps(xv0, y2v0, acc2);
            acc1b = _mm256_fmadd_ps(xv1, y1v1, acc1b);
            acc2b = _mm256_fmadd_ps(xv1, y2v1, acc2b);
        }
        i += 16;
    }
    while i + 8 <= n {
        unsafe {
            let xv = _mm256_loadu_ps(x.as_ptr().add(i));
            let y1v = _mm256_loadu_ps(y1.as_ptr().add(i));
            let y2v = _mm256_loadu_ps(y2.as_ptr().add(i));
            acc1 = _mm256_fmadd_ps(xv, y1v, acc1);
            acc2 = _mm256_fmadd_ps(xv, y2v, acc2);
        }
        i += 8;
    }
    let acc1 = _mm256_add_ps(acc1, acc1b);
    let acc2 = _mm256_add_ps(acc2, acc2b);

    let sum41 = _mm_add_ps(_mm256_castps256_ps128(acc1), _mm256_extractf128_ps(acc1, 1));
    let s21 = _mm_add_ps(sum41, _mm_movehl_ps(sum41, sum41));
    let mut s1 = _mm_cvtss_f32(_mm_add_ss(s21, _mm_shuffle_ps(s21, s21, 0x55)));

    let sum42 = _mm_add_ps(_mm256_castps256_ps128(acc2), _mm256_extractf128_ps(acc2, 1));
    let s22 = _mm_add_ps(sum42, _mm_movehl_ps(sum42, sum42));
    let mut s2 = _mm_cvtss_f32(_mm_add_ss(s22, _mm_shuffle_ps(s22, s22, 0x55)));

    while i < n {
        s1 += x[i] * y1[i];
        s2 += x[i] * y2[i];
        i += 1;
    }
    (s1, s2)
}

#[inline]
pub fn pitch_xcorr(x: &[f32], y: &[f32], xcorr: &mut [f32], len: usize, max_pitch: usize) {
    if max_pitch == 0 {
        return;
    }
    assert!(x.len() >= len && xcorr.len() >= max_pitch);
    if len == 0 {
        xcorr[..max_pitch].fill(0.0);
        return;
    }
    assert!(y.len() >= max_pitch + len - 1);

    #[cfg(target_arch = "aarch64")]
    {
        if max_pitch >= 4 {
            unsafe { pitch_xcorr_neon(x, y, xcorr, len, max_pitch) };
            return;
        }
        for i in 0..max_pitch {
            xcorr[i] = unsafe { inner_prod_neon(x, &y[i..], len) };
        }
        return;
    }
    #[cfg(target_arch = "x86_64")]
    {
        if have_avx_fma() {
            unsafe { pitch_xcorr_avx(x, y, xcorr, len, max_pitch) };
            return;
        }
        unsafe { pitch_xcorr_sse2(x, y, xcorr, len, max_pitch) };
        return;
    }
    #[allow(unreachable_code)]
    for i in 0..max_pitch {
        xcorr[i] = inner_prod_scalar(x, &y[i..], len);
    }
}

#[cfg(target_arch = "aarch64")]
#[target_feature(enable = "neon")]
#[inline]
unsafe fn xcorr_kernel_neon(x: &[f32], y: &[f32], sum: &mut [f32; 4], mut len: usize) {
    use std::arch::aarch64::*;
    if len == 0 {
        *sum = [0.0; 4];
        return;
    }
    let x = &x[..len];
    let y = &y[..len + 3];
    let mut summ = vdupq_n_f32(0.0);
    let mut xi = x.as_ptr();
    let mut yi = y.as_ptr();
    let mut yy = unsafe { vld1q_f32(yi) };

    while len > 8 {
        unsafe {
            yi = yi.add(4);
            let yy1 = vld1q_f32(yi);
            yi = yi.add(4);
            let yy2 = vld1q_f32(yi);
            let xx0 = vld1q_f32(xi);
            xi = xi.add(4);
            let xx1 = vld1q_f32(xi);
            xi = xi.add(4);

            summ = vfmaq_lane_f32(summ, yy, vget_low_f32(xx0), 0);
            summ = vfmaq_lane_f32(summ, vextq_f32(yy, yy1, 1), vget_low_f32(xx0), 1);
            summ = vfmaq_lane_f32(summ, vextq_f32(yy, yy1, 2), vget_high_f32(xx0), 0);
            summ = vfmaq_lane_f32(summ, vextq_f32(yy, yy1, 3), vget_high_f32(xx0), 1);

            summ = vfmaq_lane_f32(summ, yy1, vget_low_f32(xx1), 0);
            summ = vfmaq_lane_f32(summ, vextq_f32(yy1, yy2, 1), vget_low_f32(xx1), 1);
            summ = vfmaq_lane_f32(summ, vextq_f32(yy1, yy2, 2), vget_high_f32(xx1), 0);
            summ = vfmaq_lane_f32(summ, vextq_f32(yy1, yy2, 3), vget_high_f32(xx1), 1);

            yy = yy2;
        }
        len -= 8;
    }

    if len > 4 {
        unsafe {
            yi = yi.add(4);
            let yy1 = vld1q_f32(yi);
            let xx0 = vld1q_f32(xi);
            xi = xi.add(4);

            summ = vfmaq_lane_f32(summ, yy, vget_low_f32(xx0), 0);
            summ = vfmaq_lane_f32(summ, vextq_f32(yy, yy1, 1), vget_low_f32(xx0), 1);
            summ = vfmaq_lane_f32(summ, vextq_f32(yy, yy1, 2), vget_high_f32(xx0), 0);
            summ = vfmaq_lane_f32(summ, vextq_f32(yy, yy1, 3), vget_high_f32(xx0), 1);

            yy = yy1;
        }
        len -= 4;
    }

    while len > 1 {
        unsafe {
            let xx = vld1_dup_f32(xi);
            xi = xi.add(1);
            summ = vfmaq_lane_f32(summ, yy, xx, 0);
            yi = yi.add(1);
            yy = vld1q_f32(yi);
        }
        len -= 1;
    }

    unsafe {
        let xx = vld1_dup_f32(xi);
        summ = vfmaq_lane_f32(summ, yy, xx, 0);
        vst1q_f32(sum.as_mut_ptr(), summ);
    }
}

#[cfg(target_arch = "aarch64")]
#[target_feature(enable = "neon")]
#[inline]
unsafe fn pitch_xcorr_neon(x: &[f32], y: &[f32], xcorr: &mut [f32], len: usize, max_pitch: usize) {
    let mut i = 0usize;
    while i + 4 <= max_pitch {
        let mut sum = [0.0f32; 4];
        unsafe { xcorr_kernel_neon(x, &y[i..], &mut sum, len) };
        xcorr[i] = sum[0];
        xcorr[i + 1] = sum[1];
        xcorr[i + 2] = sum[2];
        xcorr[i + 3] = sum[3];
        i += 4;
    }
    for j in i..max_pitch {
        xcorr[j] = unsafe { inner_prod_neon(x, &y[j..], len) };
    }
}

#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "sse2")]
unsafe fn xcorr_kernel_sse2(x: &[f32], y: &[f32], sum: &mut [f32; 4], len: usize) {
    use std::arch::x86_64::*;
    let x = &x[..len];
    let y = &y[..len + 3];
    let mut xsum1 = unsafe { _mm_loadu_ps(sum.as_ptr()) };
    let mut xsum2 = _mm_setzero_ps();
    let mut j = 0usize;
    while j + 4 <= len {
        unsafe {
            let x0 = _mm_loadu_ps(x.as_ptr().add(j));
            let yj = _mm_loadu_ps(y.as_ptr().add(j));
            let y3 = _mm_loadu_ps(y.as_ptr().add(j + 3));
            xsum1 = _mm_add_ps(xsum1, _mm_mul_ps(_mm_shuffle_ps(x0, x0, 0x00), yj));
            xsum2 = _mm_add_ps(
                xsum2,
                _mm_mul_ps(_mm_shuffle_ps(x0, x0, 0x55), _mm_shuffle_ps(yj, y3, 0x49)),
            );
            xsum1 = _mm_add_ps(
                xsum1,
                _mm_mul_ps(_mm_shuffle_ps(x0, x0, 0xaa), _mm_shuffle_ps(yj, y3, 0x9e)),
            );
            xsum2 = _mm_add_ps(xsum2, _mm_mul_ps(_mm_shuffle_ps(x0, x0, 0xff), y3));
        }
        j += 4;
    }
    if j < len {
        unsafe {
            xsum1 = _mm_add_ps(
                xsum1,
                _mm_mul_ps(_mm_set1_ps(*x.as_ptr().add(j)), _mm_loadu_ps(y.as_ptr().add(j))),
            );
        }
        j += 1;
        if j < len {
            unsafe {
                xsum2 = _mm_add_ps(
                    xsum2,
                    _mm_mul_ps(_mm_set1_ps(*x.as_ptr().add(j)), _mm_loadu_ps(y.as_ptr().add(j))),
                );
            }
            j += 1;
            if j < len {
                unsafe {
                    xsum1 = _mm_add_ps(
                        xsum1,
                        _mm_mul_ps(
                            _mm_set1_ps(*x.as_ptr().add(j)),
                            _mm_loadu_ps(y.as_ptr().add(j)),
                        ),
                    );
                }
            }
        }
    }
    unsafe { _mm_storeu_ps(sum.as_mut_ptr(), _mm_add_ps(xsum1, xsum2)) };
}

#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "sse2")]
unsafe fn pitch_xcorr_sse2(x: &[f32], y: &[f32], xcorr: &mut [f32], len: usize, max_pitch: usize) {
    let mut i = 0usize;
    while i + 4 <= max_pitch {
        let mut sum = [0.0f32; 4];
        unsafe { xcorr_kernel_sse2(x, &y[i..], &mut sum, len) };
        xcorr[i] = sum[0];
        xcorr[i + 1] = sum[1];
        xcorr[i + 2] = sum[2];
        xcorr[i + 3] = sum[3];
        i += 4;
    }
    for j in i..max_pitch {
        xcorr[j] = unsafe { inner_prod_sse2(x, &y[j..], len) };
    }
}

#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx,fma")]
unsafe fn xcorr_kernel_avx(x: &[f32], y: &[f32], sum: &mut [f32; 4], len: usize) {
    use std::arch::x86_64::*;
    let x = &x[..len];
    let y = &y[..len + 3];
    let mut xsum1 = unsafe { _mm_loadu_ps(sum.as_ptr()) };
    let mut xsum2 = _mm_setzero_ps();
    let mut j = 0usize;
    while j + 8 <= len {
        unsafe {
            let x0 = _mm_loadu_ps(x.as_ptr().add(j));
            let yj = _mm_loadu_ps(y.as_ptr().add(j));
            let y3 = _mm_loadu_ps(y.as_ptr().add(j + 3));
            xsum1 = _mm_fmadd_ps(_mm_shuffle_ps(x0, x0, 0x00), yj, xsum1);
            xsum2 = _mm_fmadd_ps(_mm_shuffle_ps(x0, x0, 0x55), _mm_shuffle_ps(yj, y3, 0x49), xsum2);
            xsum1 = _mm_fmadd_ps(_mm_shuffle_ps(x0, x0, 0xaa), _mm_shuffle_ps(yj, y3, 0x9e), xsum1);
            xsum2 = _mm_fmadd_ps(_mm_shuffle_ps(x0, x0, 0xff), y3, xsum2);

            let x1 = _mm_loadu_ps(x.as_ptr().add(j + 4));
            let yj4 = _mm_loadu_ps(y.as_ptr().add(j + 4));
            let y7 = _mm_loadu_ps(y.as_ptr().add(j + 7));
            xsum1 = _mm_fmadd_ps(_mm_shuffle_ps(x1, x1, 0x00), yj4, xsum1);
            xsum2 = _mm_fmadd_ps(
                _mm_shuffle_ps(x1, x1, 0x55),
                _mm_shuffle_ps(yj4, y7, 0x49),
                xsum2,
            );
            xsum1 = _mm_fmadd_ps(
                _mm_shuffle_ps(x1, x1, 0xaa),
                _mm_shuffle_ps(yj4, y7, 0x9e),
                xsum1,
            );
            xsum2 = _mm_fmadd_ps(_mm_shuffle_ps(x1, x1, 0xff), y7, xsum2);
        }
        j += 8;
    }
    if j + 4 <= len {
        unsafe {
            let x0 = _mm_loadu_ps(x.as_ptr().add(j));
            let yj = _mm_loadu_ps(y.as_ptr().add(j));
            let y3 = _mm_loadu_ps(y.as_ptr().add(j + 3));
            xsum1 = _mm_fmadd_ps(_mm_shuffle_ps(x0, x0, 0x00), yj, xsum1);
            xsum2 = _mm_fmadd_ps(_mm_shuffle_ps(x0, x0, 0x55), _mm_shuffle_ps(yj, y3, 0x49), xsum2);
            xsum1 = _mm_fmadd_ps(_mm_shuffle_ps(x0, x0, 0xaa), _mm_shuffle_ps(yj, y3, 0x9e), xsum1);
            xsum2 = _mm_fmadd_ps(_mm_shuffle_ps(x0, x0, 0xff), y3, xsum2);
        }
        j += 4;
    }
    if j < len {
        unsafe {
            xsum1 = _mm_fmadd_ps(
                _mm_set1_ps(*x.as_ptr().add(j)),
                _mm_loadu_ps(y.as_ptr().add(j)),
                xsum1,
            );
        }
        j += 1;
        if j < len {
            unsafe {
                xsum2 = _mm_fmadd_ps(
                    _mm_set1_ps(*x.as_ptr().add(j)),
                    _mm_loadu_ps(y.as_ptr().add(j)),
                    xsum2,
                );
            }
            j += 1;
            if j < len {
                unsafe {
                    xsum1 = _mm_fmadd_ps(
                        _mm_set1_ps(*x.as_ptr().add(j)),
                        _mm_loadu_ps(y.as_ptr().add(j)),
                        xsum1,
                    );
                }
            }
        }
    }
    unsafe { _mm_storeu_ps(sum.as_mut_ptr(), _mm_add_ps(xsum1, xsum2)) };
}

#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx,fma")]
unsafe fn pitch_xcorr_avx(x: &[f32], y: &[f32], xcorr: &mut [f32], len: usize, max_pitch: usize) {
    let mut i = 0usize;
    while i + 4 <= max_pitch {
        let mut sum = [0.0f32; 4];
        unsafe { xcorr_kernel_avx(x, &y[i..], &mut sum, len) };
        xcorr[i] = sum[0];
        xcorr[i + 1] = sum[1];
        xcorr[i + 2] = sum[2];
        xcorr[i + 3] = sum[3];
        i += 4;
    }
    for j in i..max_pitch {
        xcorr[j] = unsafe { inner_prod_avx(x, &y[j..], len) };
    }
}

/// Three-tap 2:1 pitch downsampler (`factor == 2`, `offset == 1`). Returns `true` if handled.
#[inline]
pub fn pitch_downsample_taps_factor2(x: &[&[f32]], x_lp: &mut [f32], len: usize, c: usize) -> bool {
    if x_lp.len() < len || len <= 1 || c == 0 || c > 2 || x.len() < c {
        return false;
    }
    #[cfg(target_arch = "aarch64")]
    {
        if c == 1 {
            let x0 = x[0];
            if x0.len() >= 2 * len {
                unsafe { pitch_downsample_taps_c1_neon(x0, x_lp, len) };
                return true;
            }
        } else if c == 2 {
            let x0 = x[0];
            let x1 = x[1];
            if x0.len() >= 2 * len && x1.len() >= 2 * len {
                unsafe { pitch_downsample_taps_c2_neon(x0, x1, x_lp, len) };
                return true;
            }
        }
    }
    false
}

#[cfg(target_arch = "aarch64")]
#[target_feature(enable = "neon")]
#[inline]
unsafe fn pitch_downsample_taps_c1_neon(x0: &[f32], x_lp: &mut [f32], len: usize) {
    use std::arch::aarch64::*;
    let x0 = &x0[..2 * len];
    let x_lp = &mut x_lp[..len];
    let v025 = vdupq_n_f32(0.25);
    let v05 = vdupq_n_f32(0.5);
    let mut i = 1usize;
    while i + 4 <= len {
        unsafe {
            let p = x0.as_ptr();
            let mid = vld2q_f32(p.add(2 * i));
            let lo = vld2q_f32(p.add(2 * i - 2));
            let mut val = vmulq_f32(lo.1, v025);
            val = vfmaq_f32(val, mid.1, v025);
            val = vfmaq_f32(val, mid.0, v05);
            vst1q_f32(x_lp.as_mut_ptr().add(i), val);
        }
        i += 4;
    }
    while i < len {
        let idx_c = 2 * i;
        if idx_c + 1 < x0.len() {
            x_lp[i] = 0.25 * x0[idx_c - 1] + 0.25 * x0[idx_c + 1] + 0.5 * x0[idx_c];
        }
        i += 1;
    }
}

#[cfg(target_arch = "aarch64")]
#[target_feature(enable = "neon")]
#[inline]
unsafe fn pitch_downsample_taps_c2_neon(x0: &[f32], x1: &[f32], x_lp: &mut [f32], len: usize) {
    use std::arch::aarch64::*;
    let x0 = &x0[..2 * len];
    let x1 = &x1[..2 * len];
    let x_lp = &mut x_lp[..len];
    let v025 = vdupq_n_f32(0.25);
    let v05 = vdupq_n_f32(0.5);
    let mut i = 1usize;
    while i + 4 <= len {
        unsafe {
            let p0 = x0.as_ptr();
            let mid0 = vld2q_f32(p0.add(2 * i));
            let lo0 = vld2q_f32(p0.add(2 * i - 2));
            let mut val0 = vmulq_f32(lo0.1, v025);
            val0 = vfmaq_f32(val0, mid0.1, v025);
            val0 = vfmaq_f32(val0, mid0.0, v05);

            let p1 = x1.as_ptr();
            let mid1 = vld2q_f32(p1.add(2 * i));
            let lo1 = vld2q_f32(p1.add(2 * i - 2));
            let mut val1 = vmulq_f32(lo1.1, v025);
            val1 = vfmaq_f32(val1, mid1.1, v025);
            val1 = vfmaq_f32(val1, mid1.0, v05);

            vst1q_f32(x_lp.as_mut_ptr().add(i), vaddq_f32(val0, val1));
        }
        i += 4;
    }
    while i < len {
        let idx_c = 2 * i;
        if idx_c + 1 < x0.len() && idx_c + 1 < x1.len() {
            let v0 = 0.25 * x0[idx_c - 1] + 0.25 * x0[idx_c + 1] + 0.5 * x0[idx_c];
            let v1 = 0.25 * x1[idx_c - 1] + 0.25 * x1[idx_c + 1] + 0.5 * x1[idx_c];
            x_lp[i] = v0 + v1;
        }
        i += 1;
    }
}

/// Computes `1.0 + sum_{j=0..len} y[j]^2` for `find_best_pitch`.
#[inline]
pub fn sum_squares_plus_one(y: &[f32], len: usize) -> f32 {
    let y = &y[..len];
    #[cfg(target_arch = "aarch64")]
    {
        return unsafe { sum_squares_plus_one_neon(y) };
    }
    #[cfg(target_arch = "x86_64")]
    {
        if have_avx() {
            return unsafe { sum_squares_plus_one_avx(y) };
        }
    }
    #[cfg(not(target_arch = "aarch64"))]
    {
        let mut sum = 1.0f32;
        for &v in y {
            sum += v * v;
        }
        sum
    }
}

#[cfg(target_arch = "aarch64")]
#[target_feature(enable = "neon")]
#[inline]
unsafe fn sum_squares_plus_one_neon(y: &[f32]) -> f32 {
    use std::arch::aarch64::*;
    let len = y.len();
    let mut sum_vec = vdupq_n_f32(0.0);
    let mut j = 0usize;
    while j + 16 <= len {
        unsafe {
            let y0 = vld1q_f32(y.as_ptr().add(j));
            let y1 = vld1q_f32(y.as_ptr().add(j + 4));
            let y2 = vld1q_f32(y.as_ptr().add(j + 8));
            let y3 = vld1q_f32(y.as_ptr().add(j + 12));
            sum_vec = vfmaq_f32(sum_vec, y0, y0);
            sum_vec = vfmaq_f32(sum_vec, y1, y1);
            sum_vec = vfmaq_f32(sum_vec, y2, y2);
            sum_vec = vfmaq_f32(sum_vec, y3, y3);
        }
        j += 16;
    }
    while j + 4 <= len {
        unsafe {
            let y0 = vld1q_f32(y.as_ptr().add(j));
            sum_vec = vfmaq_f32(sum_vec, y0, y0);
        }
        j += 4;
    }
    let mut sum = 1.0f32 + vaddvq_f32(sum_vec);
    for &v in &y[j..] {
        sum += v * v;
    }
    sum
}

#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx")]
unsafe fn sum_squares_plus_one_avx(y: &[f32]) -> f32 {
    use std::arch::x86_64::*;
    let len = y.len();
    let mut acc0 = _mm256_setzero_ps();
    let mut acc1 = _mm256_setzero_ps();
    let mut j = 0usize;
    while j + 16 <= len {
        unsafe {
            let y0 = _mm256_loadu_ps(y.as_ptr().add(j));
            let y1 = _mm256_loadu_ps(y.as_ptr().add(j + 8));
            acc0 = _mm256_add_ps(acc0, _mm256_mul_ps(y0, y0));
            acc1 = _mm256_add_ps(acc1, _mm256_mul_ps(y1, y1));
        }
        j += 16;
    }
    while j + 8 <= len {
        unsafe {
            let y0 = _mm256_loadu_ps(y.as_ptr().add(j));
            acc0 = _mm256_add_ps(acc0, _mm256_mul_ps(y0, y0));
        }
        j += 8;
    }
    let acc = _mm256_add_ps(acc0, acc1);
    let sum4 = _mm_add_ps(_mm256_castps256_ps128(acc), _mm256_extractf128_ps(acc, 1));
    let sum2 = _mm_add_ps(sum4, _mm_movehl_ps(sum4, sum4));
    let mut sum = 1.0f32 + _mm_cvtss_f32(_mm_add_ss(sum2, _mm_shuffle_ps(sum2, sum2, 0x55)));
    for &v in &y[j..] {
        sum += v * v;
    }
    sum
}

// ---------------------------------------------------------------------------
// CELT LPC FIR & IIR
// ---------------------------------------------------------------------------

#[inline]
pub fn celt_fir(x: &[f32], num: &[f32], y: &mut [f32], n: usize, ord: usize) {
    #[cfg(target_arch = "aarch64")]
    {
        unsafe { celt_fir_neon(x, num, y, n, ord) };
        return;
    }
    #[cfg(target_arch = "x86_64")]
    {
        if ord >= 8 && have_avx_fma() {
            unsafe { celt_fir_avx(x, num, y, n, ord) };
            return;
        }
    }
    #[cfg(not(target_arch = "aarch64"))]
    celt_fir_scalar(x, num, y, n, ord);
}

#[inline]
fn celt_fir_scalar(x: &[f32], num: &[f32], y: &mut [f32], n: usize, ord: usize) {
    let x = &x[..n];
    let num = &num[..ord];
    let y = &mut y[..n];
    for i in 0..n {
        let mut sum = x[i];
        for j in 0..ord {
            if i > j {
                sum += num[j] * x[i - j - 1];
            }
        }
        y[i] = sum;
    }
}

#[cfg(target_arch = "aarch64")]
#[target_feature(enable = "neon")]
#[inline]
unsafe fn celt_fir_neon(x: &[f32], num: &[f32], y: &mut [f32], n: usize, ord: usize) {
    use std::arch::aarch64::*;
    let x = &x[..n];
    let num = &num[..ord];
    let y = &mut y[..n];
    if ord < 4 {
        celt_fir_scalar(x, num, y, n, ord);
        return;
    }
    for i in 0..n {
        let mut sum = vdupq_n_f32(0.0);
        let mut j = 0usize;
        while j + 4 <= ord && i > j + 3 {
            unsafe {
                let coeff = vld1q_f32(num.as_ptr().add(j));
                let x_vals = vld1q_f32(x.as_ptr().add(i - j - 4));
                let x_rev = vrev64q_f32(x_vals);
                let x_rev = vextq_f32(x_rev, x_rev, 2);
                sum = vfmaq_f32(sum, coeff, x_rev);
            }
            j += 4;
        }
        let mut result = x[i] + vaddvq_f32(sum);
        while j < ord {
            if i > j {
                result += num[j] * x[i - j - 1];
            }
            j += 1;
        }
        y[i] = result;
    }
}

#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx,fma")]
unsafe fn celt_fir_avx(x: &[f32], num: &[f32], y: &mut [f32], n: usize, ord: usize) {
    use std::arch::x86_64::*;
    let x = &x[..n];
    let num = &num[..ord];
    let y = &mut y[..n];
    for i in 0..n {
        let mut sum = _mm256_setzero_ps();
        let mut j = 0usize;
        while j + 8 <= ord && i > j + 7 {
            unsafe {
                let coeff = _mm256_loadu_ps(num.as_ptr().add(j));
                let x_vals = _mm256_loadu_ps(x.as_ptr().add(i - j - 8));
                let swapped = _mm256_permute2f128_ps(x_vals, x_vals, 0x01);
                let x_rev = _mm256_shuffle_ps(swapped, swapped, 0b00_01_10_11);
                sum = _mm256_fmadd_ps(coeff, x_rev, sum);
            }
            j += 8;
        }
        let s4 = _mm_add_ps(_mm256_castps256_ps128(sum), _mm256_extractf128_ps(sum, 1));
        let s2 = _mm_add_ps(s4, _mm_movehl_ps(s4, s4));
        let mut result = x[i] + _mm_cvtss_f32(_mm_add_ss(s2, _mm_shuffle_ps(s2, s2, 0x55)));
        while j < ord {
            if i > j {
                result += num[j] * x[i - j - 1];
            }
            j += 1;
        }
        y[i] = result;
    }
}

#[inline]
pub fn celt_iir(x: &[f32], den: &[f32], y: &mut [f32], n: usize, ord: usize, mem: &mut [f32]) {
    #[cfg(target_arch = "aarch64")]
    {
        unsafe { celt_iir_neon(x, den, y, n, ord, mem) };
        return;
    }
    #[cfg(target_arch = "x86_64")]
    {
        if ord >= 8 && have_avx_fma() {
            unsafe { celt_iir_avx(x, den, y, n, ord, mem) };
            return;
        }
    }
    #[cfg(not(target_arch = "aarch64"))]
    celt_iir_scalar(x, den, y, n, ord, mem);
}

#[inline]
fn celt_iir_scalar(x: &[f32], den: &[f32], y: &mut [f32], n: usize, ord: usize, mem: &mut [f32]) {
    let x = &x[..n];
    let den = &den[..ord];
    let y = &mut y[..n];
    let mem = &mut mem[..ord];
    for i in 0..n {
        let mut sum = x[i];
        for j in 0..ord {
            sum -= den[j] * mem[j];
        }
        if ord > 1 {
            mem.copy_within(0..ord - 1, 1);
        }
        if ord > 0 {
            mem[0] = sum;
        }
        y[i] = sum;
    }
}

#[cfg(target_arch = "aarch64")]
#[target_feature(enable = "neon")]
#[inline]
unsafe fn celt_iir_neon(
    x: &[f32],
    den: &[f32],
    y: &mut [f32],
    n: usize,
    ord: usize,
    mem: &mut [f32],
) {
    use std::arch::aarch64::*;
    let x = &x[..n];
    let den = &den[..ord];
    let y = &mut y[..n];
    let mem = &mut mem[..ord];
    if ord < 4 {
        celt_iir_scalar(x, den, y, n, ord, mem);
        return;
    }
    for i in 0..n {
        let mut feedback = vdupq_n_f32(0.0);
        let mut j = 0usize;
        while j + 4 <= ord {
            unsafe {
                let coeff = vld1q_f32(den.as_ptr().add(j));
                let mem_vals = vld1q_f32(mem.as_ptr().add(j));
                feedback = vfmaq_f32(feedback, coeff, mem_vals);
            }
            j += 4;
        }
        let mut fb_sum = vaddvq_f32(feedback);
        while j < ord {
            fb_sum += den[j] * mem[j];
            j += 1;
        }
        let sum = x[i] - fb_sum;
        mem.copy_within(0..ord - 1, 1);
        mem[0] = sum;
        y[i] = sum;
    }
}

#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx,fma")]
unsafe fn celt_iir_avx(
    x: &[f32],
    den: &[f32],
    y: &mut [f32],
    n: usize,
    ord: usize,
    mem: &mut [f32],
) {
    use std::arch::x86_64::*;
    let x = &x[..n];
    let den = &den[..ord];
    let y = &mut y[..n];
    let mem = &mut mem[..ord];
    for i in 0..n {
        let mut feedback = _mm256_setzero_ps();
        let mut j = 0usize;
        while j + 8 <= ord {
            unsafe {
                let coeff = _mm256_loadu_ps(den.as_ptr().add(j));
                let mem_vals = _mm256_loadu_ps(mem.as_ptr().add(j));
                feedback = _mm256_fmadd_ps(coeff, mem_vals, feedback);
            }
            j += 8;
        }
        let s4 = _mm_add_ps(
            _mm256_castps256_ps128(feedback),
            _mm256_extractf128_ps(feedback, 1),
        );
        let s2 = _mm_add_ps(s4, _mm_movehl_ps(s4, s4));
        let mut fb_sum = _mm_cvtss_f32(_mm_add_ss(s2, _mm_shuffle_ps(s2, s2, 0x55)));
        while j < ord {
            fb_sum += den[j] * mem[j];
            j += 1;
        }
        let sum = x[i] - fb_sum;
        mem.copy_within(0..ord - 1, 1);
        mem[0] = sum;
        y[i] = sum;
    }
}

// ---------------------------------------------------------------------------
// CELT KissFFT Butterflies & MDCT Rotations / TDAC
// ---------------------------------------------------------------------------

#[inline]
pub fn kf_bfly2_m1(fout: &mut [KissCpx], n: usize) {
    #[cfg(target_arch = "aarch64")]
    {
        unsafe { kf_bfly2_m1_neon(fout, n) };
        return;
    }
    #[allow(unreachable_code)]
    kf_bfly2_m1_scalar(fout, n);
}

pub fn kf_bfly2_m1_scalar(fout: &mut [KissCpx], n: usize) {
    let fout = &mut fout[..2 * n];
    for i in 0..n {
        let idx = i * 2;
        let t = fout[idx + 1];
        fout[idx + 1] = c_sub(&fout[idx], &t);
        fout[idx] = c_add(&fout[idx], &t);
    }
}

#[cfg(target_arch = "aarch64")]
#[target_feature(enable = "neon")]
#[inline]
unsafe fn kf_bfly2_m1_neon(fout: &mut [KissCpx], n: usize) {
    use std::arch::aarch64::*;
    let fout = &mut fout[..2 * n];
    let ptr = fout.as_mut_ptr() as *mut f32;
    let mut i = 0usize;
    while i + 4 <= n {
        let base = i * 4;
        unsafe {
            let v0 = vld1q_f32(ptr.add(base));
            let v1 = vld1q_f32(ptr.add(base + 4));
            let v2 = vld1q_f32(ptr.add(base + 8));
            let v3 = vld1q_f32(ptr.add(base + 12));
            let r0 = vcombine_f32(
                vadd_f32(vget_low_f32(v0), vget_high_f32(v0)),
                vsub_f32(vget_low_f32(v0), vget_high_f32(v0)),
            );
            let r1 = vcombine_f32(
                vadd_f32(vget_low_f32(v1), vget_high_f32(v1)),
                vsub_f32(vget_low_f32(v1), vget_high_f32(v1)),
            );
            let r2 = vcombine_f32(
                vadd_f32(vget_low_f32(v2), vget_high_f32(v2)),
                vsub_f32(vget_low_f32(v2), vget_high_f32(v2)),
            );
            let r3 = vcombine_f32(
                vadd_f32(vget_low_f32(v3), vget_high_f32(v3)),
                vsub_f32(vget_low_f32(v3), vget_high_f32(v3)),
            );
            vst1q_f32(ptr.add(base), r0);
            vst1q_f32(ptr.add(base + 4), r1);
            vst1q_f32(ptr.add(base + 8), r2);
            vst1q_f32(ptr.add(base + 12), r3);
        }
        i += 4;
    }
    while i + 2 <= n {
        let base = i * 4;
        unsafe {
            let v0 = vld1q_f32(ptr.add(base));
            let v1 = vld1q_f32(ptr.add(base + 4));
            let r0 = vcombine_f32(
                vadd_f32(vget_low_f32(v0), vget_high_f32(v0)),
                vsub_f32(vget_low_f32(v0), vget_high_f32(v0)),
            );
            let r1 = vcombine_f32(
                vadd_f32(vget_low_f32(v1), vget_high_f32(v1)),
                vsub_f32(vget_low_f32(v1), vget_high_f32(v1)),
            );
            vst1q_f32(ptr.add(base), r0);
            vst1q_f32(ptr.add(base + 4), r1);
        }
        i += 2;
    }
    while i < n {
        let idx = i * 2;
        let t = fout[idx + 1];
        fout[idx + 1] = c_sub(&fout[idx], &t);
        fout[idx] = c_add(&fout[idx], &t);
        i += 1;
    }
}

#[inline]
pub fn kf_bfly4_m1(fout: &mut [KissCpx], n: usize) {
    #[cfg(target_arch = "aarch64")]
    {
        unsafe { kf_bfly4_m1_neon(fout, n) };
        return;
    }
    #[allow(unreachable_code)]
    kf_bfly4_m1_scalar(fout, n);
}

pub fn kf_bfly4_m1_scalar(fout: &mut [KissCpx], n: usize) {
    let fout = &mut fout[..4 * n];
    for i in 0..n {
        let base = i * 4;
        let scratch0 = c_sub(&fout[base], &fout[base + 2]);
        let sum02 = c_add(&fout[base], &fout[base + 2]);
        let scratch1 = c_add(&fout[base + 1], &fout[base + 3]);
        let diff13 = c_sub(&fout[base + 1], &fout[base + 3]);
        fout[base] = c_add(&sum02, &scratch1);
        fout[base + 2] = c_sub(&sum02, &scratch1);
        fout[base + 1] = KissCpx::new(scratch0.r + diff13.i, scratch0.i - diff13.r);
        fout[base + 3] = KissCpx::new(scratch0.r - diff13.i, scratch0.i + diff13.r);
    }
}

#[cfg(target_arch = "aarch64")]
#[target_feature(enable = "neon")]
#[inline]
unsafe fn kf_bfly4_m1_neon(fout: &mut [KissCpx], n: usize) {
    use std::arch::aarch64::*;
    let fout = &mut fout[..4 * n];
    let ptr = fout.as_mut_ptr() as *mut f32;
    let mut i = 0usize;
    while i + 2 <= n {
        let base = i * 8;
        unsafe {
            let v0 = vld1q_f32(ptr.add(base));
            let v1 = vld1q_f32(ptr.add(base + 4));
            let v2 = vld1q_f32(ptr.add(base + 8));
            let v3 = vld1q_f32(ptr.add(base + 12));

            let sum02_0 = vadd_f32(vget_low_f32(v0), vget_low_f32(v1));
            let diff02_0 = vsub_f32(vget_low_f32(v0), vget_low_f32(v1));
            let scr1_0 = vadd_f32(vget_high_f32(v0), vget_high_f32(v1));
            let dif13_0 = vsub_f32(vget_high_f32(v0), vget_high_f32(v1));
            let f0_0 = vadd_f32(sum02_0, scr1_0);
            let f2_0 = vsub_f32(sum02_0, scr1_0);
            let neg_d13_0 = vneg_f32(dif13_0);
            let j_d13_0 = vext_f32(neg_d13_0, dif13_0, 1);
            let mj_d13_0 = vext_f32(dif13_0, neg_d13_0, 1);
            let f1_0 = vadd_f32(diff02_0, mj_d13_0);
            let f3_0 = vadd_f32(diff02_0, j_d13_0);
            vst1q_f32(ptr.add(base), vcombine_f32(f0_0, f1_0));
            vst1q_f32(ptr.add(base + 4), vcombine_f32(f2_0, f3_0));

            let sum02_1 = vadd_f32(vget_low_f32(v2), vget_low_f32(v3));
            let diff02_1 = vsub_f32(vget_low_f32(v2), vget_low_f32(v3));
            let scr1_1 = vadd_f32(vget_high_f32(v2), vget_high_f32(v3));
            let dif13_1 = vsub_f32(vget_high_f32(v2), vget_high_f32(v3));
            let f0_1 = vadd_f32(sum02_1, scr1_1);
            let f2_1 = vsub_f32(sum02_1, scr1_1);
            let neg_d13_1 = vneg_f32(dif13_1);
            let j_d13_1 = vext_f32(neg_d13_1, dif13_1, 1);
            let mj_d13_1 = vext_f32(dif13_1, neg_d13_1, 1);
            let f1_1 = vadd_f32(diff02_1, mj_d13_1);
            let f3_1 = vadd_f32(diff02_1, j_d13_1);
            vst1q_f32(ptr.add(base + 8), vcombine_f32(f0_1, f1_1));
            vst1q_f32(ptr.add(base + 12), vcombine_f32(f2_1, f3_1));
        }
        i += 2;
    }
    if i < n {
        let base = i * 4;
        let scratch0 = c_sub(&fout[base], &fout[base + 2]);
        let sum02 = c_add(&fout[base], &fout[base + 2]);
        let scratch1 = c_add(&fout[base + 1], &fout[base + 3]);
        let diff13 = c_sub(&fout[base + 1], &fout[base + 3]);
        fout[base] = c_add(&sum02, &scratch1);
        fout[base + 2] = c_sub(&sum02, &scratch1);
        fout[base + 1] = KissCpx::new(scratch0.r + diff13.i, scratch0.i - diff13.r);
        fout[base + 3] = KissCpx::new(scratch0.r - diff13.i, scratch0.i + diff13.r);
    }
}

#[cfg(target_arch = "aarch64")]
#[target_feature(enable = "neon")]
#[inline]
unsafe fn neon_cmul_2(
    a: std::arch::aarch64::float32x4_t,
    b: std::arch::aarch64::float32x4_t,
) -> std::arch::aarch64::float32x4_t {
    use std::arch::aarch64::*;
    let dup_r = vcombine_f32(
        vdup_lane_f32(vget_low_f32(a), 0),
        vdup_lane_f32(vget_high_f32(a), 0),
    );
    let t1 = vmulq_f32(dup_r, b);
    let neg_a = vnegq_f32(a);
    let swap_neg = vtrnq_f32(neg_a, a).1;
    let rev_b = vrev64q_f32(b);
    let t2 = vmulq_f32(swap_neg, rev_b);
    vaddq_f32(t1, t2)
}

#[inline]
pub fn kf_bfly3(
    fout: &mut [KissCpx],
    fstride: usize,
    twiddles: &[KissCpx],
    m: usize,
    n: usize,
    mm: usize,
) {
    #[cfg(target_arch = "aarch64")]
    {
        unsafe { kf_bfly3_neon(fout, fstride, twiddles, m, n, mm) };
        return;
    }
    #[allow(unreachable_code)]
    kf_bfly3_scalar(fout, fstride, twiddles, m, n, mm);
}

pub fn kf_bfly3_scalar(
    fout: &mut [KissCpx],
    fstride: usize,
    twiddles: &[KissCpx],
    m: usize,
    n: usize,
    mm: usize,
) {
    let m2 = 2 * m;
    let epi3_i: f32 = -0.866_025_4;
    let stride2 = fstride * 2;
    for i in 0..n {
        let base = i * mm;
        let mut tw1 = 0usize;
        let mut tw2 = 0usize;
        for j in 0..m {
            let idx = base + j;
            let scratch1 = c_mul(&fout[idx + m], &twiddles[tw1]);
            let scratch2 = c_mul(&fout[idx + m2], &twiddles[tw2]);
            let scratch3 = c_add(&scratch1, &scratch2);
            let scratch0 = c_sub(&scratch1, &scratch2);
            let half_scratch3 = KissCpx::new(scratch3.r * 0.5, scratch3.i * 0.5);
            let fout_m = KissCpx::new(fout[idx].r - half_scratch3.r, fout[idx].i - half_scratch3.i);
            let scratch0_scaled = KissCpx::new(scratch0.r * epi3_i, scratch0.i * epi3_i);
            fout[idx] = c_add(&fout[idx], &scratch3);
            fout[idx + m] =
                KissCpx::new(fout_m.r - scratch0_scaled.i, fout_m.i + scratch0_scaled.r);
            fout[idx + m2] =
                KissCpx::new(fout_m.r + scratch0_scaled.i, fout_m.i - scratch0_scaled.r);
            tw1 += fstride;
            tw2 += stride2;
        }
    }
}

#[cfg(target_arch = "aarch64")]
#[target_feature(enable = "neon")]
#[inline]
unsafe fn kf_bfly3_neon(
    fout: &mut [KissCpx],
    fstride: usize,
    twiddles: &[KissCpx],
    m: usize,
    n: usize,
    mm: usize,
) {
    use std::arch::aarch64::*;
    assert!(
        n == 0
            || m == 0
            || (fout.len() >= (n - 1) * mm + 3 * m && twiddles.len() > (m - 1) * 2 * fstride),
        "kf_bfly3 runs outside its buffers"
    );
    let m2 = 2 * m;
    let epi3_i: f32 = -0.866_025_4;
    let stride2 = fstride * 2;
    let fout_ptr = fout.as_mut_ptr() as *mut f32;
    let tw_ptr = twiddles.as_ptr() as *const f32;

    for i in 0..n {
        let base = i * mm;
        let mut tw1 = 0usize;
        let mut tw2 = 0usize;
        let m_vec = m & !1;
        let mut j = 0usize;
        while j < m_vec {
            let idx0 = base + j;
            unsafe {
                let fm = vld1q_f32(fout_ptr.add(2 * (idx0 + m)));
                let tw1_v = vcombine_f32(
                    vld1_f32(tw_ptr.add(2 * tw1)),
                    vld1_f32(tw_ptr.add(2 * (tw1 + fstride))),
                );
                let s1 = neon_cmul_2(fm, tw1_v);

                let fm2 = vld1q_f32(fout_ptr.add(2 * (idx0 + m2)));
                let tw2_v = vcombine_f32(
                    vld1_f32(tw_ptr.add(2 * tw2)),
                    vld1_f32(tw_ptr.add(2 * (tw2 + stride2))),
                );
                let s2 = neon_cmul_2(fm2, tw2_v);

                let s3 = vaddq_f32(s1, s2);
                let s0 = vsubq_f32(s1, s2);
                let half_s3 = vmulq_n_f32(s3, 0.5);
                let f0 = vld1q_f32(fout_ptr.add(2 * idx0));
                let fout_m = vsubq_f32(f0, half_s3);
                let s0_scaled = vmulq_n_f32(s0, epi3_i);

                vst1q_f32(fout_ptr.add(2 * idx0), vaddq_f32(f0, s3));

                let neg_s0 = vnegq_f32(s0_scaled);
                let adj_m = vcombine_f32(
                    vext_f32(vget_low_f32(neg_s0), vget_low_f32(s0_scaled), 1),
                    vext_f32(vget_high_f32(neg_s0), vget_high_f32(s0_scaled), 1),
                );
                vst1q_f32(fout_ptr.add(2 * (idx0 + m)), vaddq_f32(fout_m, adj_m));

                let adj_m2 = vcombine_f32(
                    vext_f32(vget_low_f32(s0_scaled), vget_low_f32(neg_s0), 1),
                    vext_f32(vget_high_f32(s0_scaled), vget_high_f32(neg_s0), 1),
                );
                vst1q_f32(fout_ptr.add(2 * (idx0 + m2)), vaddq_f32(fout_m, adj_m2));
            }
            tw1 += 2 * fstride;
            tw2 += 2 * stride2;
            j += 2;
        }
        for j in m_vec..m {
            let idx = base + j;
            let scratch1 = c_mul(&fout[idx + m], &twiddles[tw1]);
            let scratch2 = c_mul(&fout[idx + m2], &twiddles[tw2]);
            let scratch3 = c_add(&scratch1, &scratch2);
            let scratch0 = c_sub(&scratch1, &scratch2);
            let half_scratch3 = KissCpx::new(scratch3.r * 0.5, scratch3.i * 0.5);
            let fout_m = KissCpx::new(fout[idx].r - half_scratch3.r, fout[idx].i - half_scratch3.i);
            let scratch0_scaled = KissCpx::new(scratch0.r * epi3_i, scratch0.i * epi3_i);
            fout[idx] = c_add(&fout[idx], &scratch3);
            fout[idx + m] =
                KissCpx::new(fout_m.r - scratch0_scaled.i, fout_m.i + scratch0_scaled.r);
            fout[idx + m2] =
                KissCpx::new(fout_m.r + scratch0_scaled.i, fout_m.i - scratch0_scaled.r);
            tw1 += fstride;
            tw2 += stride2;
        }
    }
}

#[inline]
pub fn kf_bfly5(
    fout: &mut [KissCpx],
    fstride: usize,
    twiddles: &[KissCpx],
    m: usize,
    n: usize,
    mm: usize,
) {
    #[cfg(target_arch = "aarch64")]
    {
        unsafe { kf_bfly5_neon(fout, fstride, twiddles, m, n, mm) };
        return;
    }
    #[allow(unreachable_code)]
    kf_bfly5_scalar(fout, fstride, twiddles, m, n, mm);
}

pub fn kf_bfly5_scalar(
    fout: &mut [KissCpx],
    fstride: usize,
    twiddles: &[KissCpx],
    m: usize,
    n: usize,
    mm: usize,
) {
    let ya = KissCpx::new(0.309_017, -0.95105652);
    let yb = KissCpx::new(-0.809_017, -0.58778525);
    let stride2 = fstride * 2;
    let stride3 = fstride * 3;
    let stride4 = fstride * 4;
    for i in 0..n {
        let base = i * mm;
        let mut tw1 = 0usize;
        let mut tw2 = 0usize;
        let mut tw3 = 0usize;
        let mut tw4 = 0usize;
        for u in 0..m {
            let idx0 = base + u;
            let idx1 = idx0 + m;
            let idx2 = idx0 + 2 * m;
            let idx3 = idx0 + 3 * m;
            let idx4 = idx0 + 4 * m;

            let scratch0 = fout[idx0];
            let scratch1 = c_mul(&fout[idx1], &twiddles[tw1]);
            let scratch2 = c_mul(&fout[idx2], &twiddles[tw2]);
            let scratch3 = c_mul(&fout[idx3], &twiddles[tw3]);
            let scratch4 = c_mul(&fout[idx4], &twiddles[tw4]);

            let scratch7 = c_add(&scratch1, &scratch4);
            let scratch10 = c_sub(&scratch1, &scratch4);
            let scratch8 = c_add(&scratch2, &scratch3);
            let scratch9 = c_sub(&scratch2, &scratch3);

            fout[idx0] = KissCpx::new(
                scratch0.r + scratch7.r + scratch8.r,
                scratch0.i + scratch7.i + scratch8.i,
            );
            let scratch5 = KissCpx::new(
                scratch0.r + scratch7.r * ya.r + scratch8.r * yb.r,
                scratch0.i + scratch7.i * ya.r + scratch8.i * yb.r,
            );
            let scratch6 = KissCpx::new(
                scratch10.i * ya.i + scratch9.i * yb.i,
                -(scratch10.r * ya.i + scratch9.r * yb.i),
            );
            fout[idx1] = c_sub(&scratch5, &scratch6);
            fout[idx4] = c_add(&scratch5, &scratch6);

            let scratch11 = KissCpx::new(
                scratch0.r + scratch7.r * yb.r + scratch8.r * ya.r,
                scratch0.i + scratch7.i * yb.r + scratch8.i * ya.r,
            );
            let scratch12 = KissCpx::new(
                scratch9.i * ya.i - scratch10.i * yb.i,
                scratch10.r * yb.i - scratch9.r * ya.i,
            );
            fout[idx2] = c_add(&scratch11, &scratch12);
            fout[idx3] = c_sub(&scratch11, &scratch12);

            tw1 += fstride;
            tw2 += stride2;
            tw3 += stride3;
            tw4 += stride4;
        }
    }
}

#[cfg(target_arch = "aarch64")]
#[target_feature(enable = "neon")]
#[inline]
unsafe fn kf_bfly5_neon(
    fout: &mut [KissCpx],
    fstride: usize,
    twiddles: &[KissCpx],
    m: usize,
    n: usize,
    mm: usize,
) {
    use std::arch::aarch64::*;
    assert!(
        n == 0
            || m == 0
            || (fout.len() >= (n - 1) * mm + 5 * m && twiddles.len() > (m - 1) * 4 * fstride),
        "kf_bfly5 runs outside its buffers"
    );
    let m2 = 2 * m;
    let m3 = 3 * m;
    let m4 = 4 * m;
    let stride2 = fstride * 2;
    let stride3 = fstride * 3;
    let stride4 = fstride * 4;
    let ya_r: f32 = 0.309_017;
    let ya_i: f32 = -0.95105652;
    let yb_r: f32 = -0.809_017;
    let yb_i: f32 = -0.58778525;

    let fout_ptr = fout.as_mut_ptr() as *mut f32;
    let tw_ptr = twiddles.as_ptr() as *const f32;

    for i in 0..n {
        let base = i * mm;
        let mut tw1 = 0usize;
        let mut tw2 = 0usize;
        let mut tw3 = 0usize;
        let mut tw4 = 0usize;
        let m_vec = m & !1;
        let mut j = 0usize;
        while j < m_vec {
            let idx0 = base + j;
            let (s1_arr, s2_arr, s3_arr, s4_arr, f0_arr): (
                [f32; 4],
                [f32; 4],
                [f32; 4],
                [f32; 4],
                [f32; 4],
            ) = unsafe {
                let f0 = vld1q_f32(fout_ptr.add(2 * idx0));
                let fm1_v = vld1q_f32(fout_ptr.add(2 * (idx0 + m)));
                let fm2_v = vld1q_f32(fout_ptr.add(2 * (idx0 + m2)));
                let fm3_v = vld1q_f32(fout_ptr.add(2 * (idx0 + m3)));
                let fm4_v = vld1q_f32(fout_ptr.add(2 * (idx0 + m4)));

                let tw1_v = vcombine_f32(
                    vld1_f32(tw_ptr.add(2 * tw1)),
                    vld1_f32(tw_ptr.add(2 * (tw1 + fstride))),
                );
                let tw2_v = vcombine_f32(
                    vld1_f32(tw_ptr.add(2 * tw2)),
                    vld1_f32(tw_ptr.add(2 * (tw2 + stride2))),
                );
                let tw3_v = vcombine_f32(
                    vld1_f32(tw_ptr.add(2 * tw3)),
                    vld1_f32(tw_ptr.add(2 * (tw3 + stride3))),
                );
                let tw4_v = vcombine_f32(
                    vld1_f32(tw_ptr.add(2 * tw4)),
                    vld1_f32(tw_ptr.add(2 * (tw4 + stride4))),
                );

                let s1 = neon_cmul_2(fm1_v, tw1_v);
                let s2 = neon_cmul_2(fm2_v, tw2_v);
                let s3 = neon_cmul_2(fm3_v, tw3_v);
                let s4 = neon_cmul_2(fm4_v, tw4_v);
                let mut a1 = [0.0f32; 4];
                let mut a2 = [0.0f32; 4];
                let mut a3 = [0.0f32; 4];
                let mut a4 = [0.0f32; 4];
                let mut a0 = [0.0f32; 4];
                vst1q_f32(a1.as_mut_ptr(), s1);
                vst1q_f32(a2.as_mut_ptr(), s2);
                vst1q_f32(a3.as_mut_ptr(), s3);
                vst1q_f32(a4.as_mut_ptr(), s4);
                vst1q_f32(a0.as_mut_ptr(), f0);
                (a1, a2, a3, a4, a0)
            };

            for k in 0..2 {
                let s1r = s1_arr[2 * k];
                let s1i = s1_arr[2 * k + 1];
                let s2r = s2_arr[2 * k];
                let s2i = s2_arr[2 * k + 1];
                let s3r = s3_arr[2 * k];
                let s3i = s3_arr[2 * k + 1];
                let s4r = s4_arr[2 * k];
                let s4i = s4_arr[2 * k + 1];
                let f0r = f0_arr[2 * k];
                let f0i = f0_arr[2 * k + 1];

                let s7r = s1r + s4r;
                let s7i = s1i + s4i;
                let s10r = s1r - s4r;
                let s10i = s1i - s4i;
                let s8r = s2r + s3r;
                let s8i = s2i + s3i;
                let s9r = s2r - s3r;
                let s9i = s2i - s3i;

                let idx = idx0 + k;
                fout[idx].r = f0r + s7r + s8r;
                fout[idx].i = f0i + s7i + s8i;

                let s5r = f0r + s7r * ya_r + s8r * yb_r;
                let s5i = f0i + s7i * ya_r + s8i * yb_r;
                let s6r = s10i * ya_i + s9i * yb_i;
                let s6i = -(s10r * ya_i + s9r * yb_i);

                fout[idx + m].r = s5r - s6r;
                fout[idx + m].i = s5i - s6i;
                fout[idx + m4].r = s5r + s6r;
                fout[idx + m4].i = s5i + s6i;

                let s11r = f0r + s7r * yb_r + s8r * ya_r;
                let s11i = f0i + s7i * yb_r + s8i * ya_r;
                let s12r = s9i * ya_i - s10i * yb_i;
                let s12i = s10r * yb_i - s9r * ya_i;

                fout[idx + m2].r = s11r + s12r;
                fout[idx + m2].i = s11i + s12i;
                fout[idx + m3].r = s11r - s12r;
                fout[idx + m3].i = s11i - s12i;
            }

            tw1 += 2 * fstride;
            tw2 += 2 * stride2;
            tw3 += 2 * stride3;
            tw4 += 2 * stride4;
            j += 2;
        }
        for j in m_vec..m {
            let idx = base + j;
            let scratch0 = fout[idx];
            let scratch1 = c_mul(&fout[idx + m], &twiddles[tw1]);
            let scratch2 = c_mul(&fout[idx + m2], &twiddles[tw2]);
            let scratch3 = c_mul(&fout[idx + m3], &twiddles[tw3]);
            let scratch4 = c_mul(&fout[idx + m4], &twiddles[tw4]);
            let scratch7 = c_add(&scratch1, &scratch4);
            let scratch10 = c_sub(&scratch1, &scratch4);
            let scratch8 = c_add(&scratch2, &scratch3);
            let scratch9 = c_sub(&scratch2, &scratch3);
            fout[idx].r = scratch0.r + scratch7.r + scratch8.r;
            fout[idx].i = scratch0.i + scratch7.i + scratch8.i;
            let scratch5 = KissCpx::new(
                scratch0.r + scratch7.r * ya_r + scratch8.r * yb_r,
                scratch0.i + scratch7.i * ya_r + scratch8.i * yb_r,
            );
            let scratch6 = KissCpx::new(
                scratch10.i * ya_i + scratch9.i * yb_i,
                -(scratch10.r * ya_i + scratch9.r * yb_i),
            );
            fout[idx + m] = c_sub(&scratch5, &scratch6);
            fout[idx + m4] = c_add(&scratch5, &scratch6);
            let scratch11 = KissCpx::new(
                scratch0.r + scratch7.r * yb_r + scratch8.r * ya_r,
                scratch0.i + scratch7.i * yb_r + scratch8.i * ya_r,
            );
            let scratch12 = KissCpx::new(
                scratch9.i * ya_i - scratch10.i * yb_i,
                scratch10.r * yb_i - scratch9.r * ya_i,
            );
            fout[idx + m2] = c_add(&scratch11, &scratch12);
            fout[idx + m3] = c_sub(&scratch11, &scratch12);
            tw1 += fstride;
            tw2 += stride2;
            tw3 += stride3;
            tw4 += stride4;
        }
    }
}

#[inline]
pub fn mdct_pre_rotation(
    f: &[f32],
    f2: &mut [KissCpx],
    trig: &[f32],
    bitrev: &[i16],
    n4: usize,
    scale: f32,
) {
    #[cfg(target_arch = "aarch64")]
    {
        unsafe { mdct_pre_rotation_neon(f, f2, trig, bitrev, n4, scale) };
        return;
    }
    #[allow(unreachable_code)]
    mdct_pre_rotation_scalar(f, f2, trig, bitrev, n4, scale);
}

fn mdct_pre_rotation_scalar(
    f: &[f32],
    f2: &mut [KissCpx],
    trig: &[f32],
    bitrev: &[i16],
    n4: usize,
    scale: f32,
) {
    let f = &f[..2 * n4];
    let trig = &trig[..2 * n4];
    let bitrev = &bitrev[..n4];
    for i in 0..n4 {
        let re = f[2 * i];
        let im = f[2 * i + 1];
        let t0 = trig[i];
        let t1 = trig[n4 + i];
        let yr = re * t0 - im * t1;
        let yi = im * t0 + re * t1;
        f2[bitrev[i] as usize] = KissCpx::new(yr * scale, yi * scale);
    }
}

#[cfg(target_arch = "aarch64")]
#[target_feature(enable = "neon")]
#[inline]
unsafe fn mdct_pre_rotation_neon(
    f: &[f32],
    f2: &mut [KissCpx],
    trig: &[f32],
    bitrev: &[i16],
    n4: usize,
    scale: f32,
) {
    use std::arch::aarch64::*;
    let f = &f[..2 * n4];
    let trig = &trig[..2 * n4];
    let bitrev = &bitrev[..n4];

    let vscale = vdupq_n_f32(scale);
    let f_ptr = f.as_ptr();
    let trig_ptr = trig.as_ptr();
    let n4_vec = n4 & !3;
    let mut i = 0usize;

    while i < n4_vec {
        let mut yr_arr = [0.0f32; 4];
        let mut yi_arr = [0.0f32; 4];
        unsafe {
            let t0 = vld1q_f32(trig_ptr.add(i));
            let t1 = vld1q_f32(trig_ptr.add(n4 + i));
            let f0 = vld1q_f32(f_ptr.add(2 * i));
            let f1 = vld1q_f32(f_ptr.add(2 * i + 4));
            let even_odd = vuzpq_f32(f0, f1);
            let yr = vmulq_f32(vsubq_f32(vmulq_f32(even_odd.0, t0), vmulq_f32(even_odd.1, t1)), vscale);
            let yi = vmulq_f32(vaddq_f32(vmulq_f32(even_odd.1, t0), vmulq_f32(even_odd.0, t1)), vscale);
            vst1q_f32(yr_arr.as_mut_ptr(), yr);
            vst1q_f32(yi_arr.as_mut_ptr(), yi);
        }
        for j in 0..4 {
            let rev = bitrev[i + j] as usize;
            f2[rev] = KissCpx::new(yr_arr[j], yi_arr[j]);
        }
        i += 4;
    }

    for i in n4_vec..n4 {
        let re = f[2 * i];
        let im = f[2 * i + 1];
        let t0 = trig[i];
        let t1 = trig[n4 + i];
        let yr = re * t0 - im * t1;
        let yi = im * t0 + re * t1;
        f2[bitrev[i] as usize] = KissCpx::new(yr * scale, yi * scale);
    }
}

#[inline]
pub fn mdct_post_rotation(
    f2: &[KissCpx],
    trig: &[f32],
    output: &mut [f32],
    n4: usize,
    n2: usize,
    stride: usize,
) {
    #[cfg(target_arch = "aarch64")]
    {
        if stride == 1 && n2 >= 2 * n4 && output.len() >= n2 {
            unsafe { mdct_post_rotation_neon(f2, trig, output, n4, n2) };
            return;
        }
    }
    mdct_post_rotation_scalar(f2, trig, output, n4, n2, stride);
}

fn mdct_post_rotation_scalar(
    f2: &[KissCpx],
    trig: &[f32],
    output: &mut [f32],
    n4: usize,
    n2: usize,
    stride: usize,
) {
    let f2 = &f2[..n4];
    let trig = &trig[..2 * n4];
    for i in 0..n4 {
        let fp = &f2[i];
        let t0 = trig[i];
        let t1 = trig[n4 + i];
        let yr = fp.i * t1 - fp.r * t0;
        let yi = fp.r * t1 + fp.i * t0;
        output[i * 2 * stride] = yr;
        output[stride * (n2 - 1 - 2 * i)] = yi;
    }
}

#[cfg(target_arch = "aarch64")]
#[target_feature(enable = "neon")]
#[inline]
unsafe fn mdct_post_rotation_neon(
    f2: &[KissCpx],
    trig: &[f32],
    output: &mut [f32],
    n4: usize,
    n2: usize,
) {
    use std::arch::aarch64::*;
    let f2 = &f2[..n4];
    let trig = &trig[..2 * n4];
    let output = &mut output[..n2];
    let f2_ptr = f2.as_ptr() as *const f32;
    let trig_ptr = trig.as_ptr();
    let n4_vec = n4 & !3;
    let mut i = 0usize;

    while i < n4_vec {
        let mut yr_arr = [0.0f32; 4];
        let mut yi_arr = [0.0f32; 4];
        unsafe {
            let c0 = vld1q_f32(f2_ptr.add(2 * i));
            let c1 = vld1q_f32(f2_ptr.add(2 * i + 4));
            let t0 = vld1q_f32(trig_ptr.add(i));
            let t1 = vld1q_f32(trig_ptr.add(n4 + i));
            let ri = vuzpq_f32(c0, c1);
            let yr = vsubq_f32(vmulq_f32(ri.1, t1), vmulq_f32(ri.0, t0));
            let yi = vaddq_f32(vmulq_f32(ri.0, t1), vmulq_f32(ri.1, t0));
            vst1q_f32(yr_arr.as_mut_ptr(), yr);
            vst1q_f32(yi_arr.as_mut_ptr(), yi);
        }
        for j in 0..4 {
            output[(i + j) * 2] = yr_arr[j];
            output[n2 - 1 - 2 * (i + j)] = yi_arr[j];
        }
        i += 4;
    }

    for i in n4_vec..n4 {
        let fp = &f2[i];
        let t0 = trig[i];
        let t1 = trig[n4 + i];
        output[i * 2] = fp.i * t1 - fp.r * t0;
        output[n2 - 1 - 2 * i] = fp.r * t1 + fp.i * t0;
    }
}

#[inline]
pub fn mdct_backward_pre_rotation(
    input: &[f32],
    f2: &mut [KissCpx],
    trig: &[f32],
    bitrev: &[i16],
    n4: usize,
    n2: usize,
    stride: usize,
) {
    #[cfg(target_arch = "aarch64")]
    {
        if stride == 1 && n2 >= 2 * n4 && input.len() >= n2 {
            unsafe { mdct_backward_pre_rotation_neon(input, f2, trig, bitrev, n4, n2) };
            return;
        }
    }
    mdct_backward_pre_rotation_scalar(input, f2, trig, bitrev, n4, n2, stride);
}

fn mdct_backward_pre_rotation_scalar(
    input: &[f32],
    f2: &mut [KissCpx],
    trig: &[f32],
    bitrev: &[i16],
    n4: usize,
    n2: usize,
    stride: usize,
) {
    let trig = &trig[..2 * n4];
    let bitrev = &bitrev[..n4];
    for i in 0..n4 {
        let rev = bitrev[i] as usize;
        let x1 = input[2 * i * stride];
        let x2 = input[stride * (n2 - 1 - 2 * i)];
        let t0 = trig[i];
        let t1 = trig[n4 + i];
        let yr = x2 * t0 + x1 * t1;
        let yi = x1 * t0 - x2 * t1;
        f2[rev] = KissCpx::new(yi, yr);
    }
}

#[cfg(target_arch = "aarch64")]
#[target_feature(enable = "neon")]
#[inline]
unsafe fn mdct_backward_pre_rotation_neon(
    input: &[f32],
    f2: &mut [KissCpx],
    trig: &[f32],
    bitrev: &[i16],
    n4: usize,
    n2: usize,
) {
    use std::arch::aarch64::*;
    let input = &input[..n2];
    let trig = &trig[..2 * n4];
    let bitrev = &bitrev[..n4];
    let in_ptr = input.as_ptr();
    let trig_ptr = trig.as_ptr();
    let n4_vec = n4 & !3;
    let mut i = 0usize;

    while i < n4_vec {
        let mut yr_arr = [0.0f32; 4];
        let mut yi_arr = [0.0f32; 4];
        unsafe {
            let f0 = vld1q_f32(in_ptr.add(2 * i));
            let f1 = vld1q_f32(in_ptr.add(2 * i + 4));
            let x1_v = vuzpq_f32(f0, f1).0;

            let g0 = vld1q_f32(in_ptr.add(n2 - 7 - 2 * i));
            let g1 = vld1q_f32(in_ptr.add(n2 - 3 - 2 * i));
            let x2_raw = vuzpq_f32(g0, g1).0;
            let x2_rev = vrev64q_f32(x2_raw);
            let x2_v = vextq_f32(x2_rev, x2_rev, 2);

            let t0 = vld1q_f32(trig_ptr.add(i));
            let t1 = vld1q_f32(trig_ptr.add(n4 + i));

            let yr = vaddq_f32(vmulq_f32(x2_v, t0), vmulq_f32(x1_v, t1));
            let yi = vsubq_f32(vmulq_f32(x1_v, t0), vmulq_f32(x2_v, t1));
            vst1q_f32(yr_arr.as_mut_ptr(), yr);
            vst1q_f32(yi_arr.as_mut_ptr(), yi);
        }
        for j in 0..4 {
            let rev = bitrev[i + j] as usize;
            f2[rev] = KissCpx::new(yi_arr[j], yr_arr[j]);
        }
        i += 4;
    }

    for i in n4_vec..n4 {
        let rev = bitrev[i] as usize;
        let x1 = input[2 * i];
        let x2 = input[n2 - 1 - 2 * i];
        let t0 = trig[i];
        let t1 = trig[n4 + i];
        let yr = x2 * t0 + x1 * t1;
        let yi = x1 * t0 - x2 * t1;
        f2[rev] = KissCpx::new(yi, yr);
    }
}

#[inline]
pub fn mdct_tdac(output: &mut [f32], window: &[f32], overlap: usize) {
    #[cfg(target_arch = "aarch64")]
    {
        unsafe { mdct_tdac_neon(output, window, overlap) };
        return;
    }
    #[cfg(target_arch = "x86_64")]
    {
        if have_avx() {
            unsafe { mdct_tdac_avx(output, window, overlap) };
            return;
        }
    }
    #[cfg(not(target_arch = "aarch64"))]
    mdct_tdac_scalar(output, window, overlap);
}

fn mdct_tdac_scalar(output: &mut [f32], window: &[f32], overlap: usize) {
    let output = &mut output[..overlap];
    let window = &window[..overlap];
    for i in 0..overlap / 2 {
        let x1 = output[overlap - 1 - i];
        let x2 = output[i];
        let wp1 = window[i];
        let wp2 = window[overlap - 1 - i];
        output[i] = x2 * wp2 - x1 * wp1;
        output[overlap - 1 - i] = x2 * wp1 + x1 * wp2;
    }
}

#[cfg(target_arch = "aarch64")]
#[target_feature(enable = "neon")]
#[inline]
unsafe fn mdct_tdac_neon(output: &mut [f32], window: &[f32], overlap: usize) {
    use std::arch::aarch64::*;
    let output = &mut output[..overlap];
    let window = &window[..overlap];
    let overlap2 = overlap / 2;
    if overlap2 < 4 {
        mdct_tdac_scalar(output, window, overlap);
        return;
    }
    let out_ptr = output.as_mut_ptr();
    let win_ptr = window.as_ptr();
    let n4 = overlap2 & !3;
    let mut i = 0usize;
    while i < n4 {
        unsafe {
            let x2_fwd = vld1q_f32(out_ptr.add(i));
            let x1_rev = vld1q_f32(out_ptr.add(overlap - 4 - i));
            let x1_r = vrev64q_f32(x1_rev);
            let x1 = vextq_f32(x1_r, x1_r, 2);

            let wp1 = vld1q_f32(win_ptr.add(i));
            let wp2_rev = vld1q_f32(win_ptr.add(overlap - 4 - i));
            let wp2_r = vrev64q_f32(wp2_rev);
            let wp2 = vextq_f32(wp2_r, wp2_r, 2);

            let out_fwd = vsubq_f32(vmulq_f32(x2_fwd, wp2), vmulq_f32(x1, wp1));
            let out_rev = vaddq_f32(vmulq_f32(x2_fwd, wp1), vmulq_f32(x1, wp2));
            let out_rev = vrev64q_f32(out_rev);
            let out_rev = vextq_f32(out_rev, out_rev, 2);

            vst1q_f32(out_ptr.add(i), out_fwd);
            vst1q_f32(out_ptr.add(overlap - 4 - i), out_rev);
        }
        i += 4;
    }
    for i in n4..overlap2 {
        let x1 = output[overlap - 1 - i];
        let x2 = output[i];
        output[i] = x2 * window[overlap - 1 - i] - x1 * window[i];
        output[overlap - 1 - i] = x2 * window[i] + x1 * window[overlap - 1 - i];
    }
}

#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx")]
#[inline]
unsafe fn avx_rev_ps(v: std::arch::x86_64::__m256) -> std::arch::x86_64::__m256 {
    use std::arch::x86_64::*;
    let swapped = _mm256_permute2f128_ps(v, v, 0x01);
    _mm256_shuffle_ps(swapped, swapped, 0b00_01_10_11)
}

#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx")]
unsafe fn mdct_tdac_avx(output: &mut [f32], window: &[f32], overlap: usize) {
    use std::arch::x86_64::*;
    let output = &mut output[..overlap];
    let window = &window[..overlap];
    let overlap2 = overlap / 2;
    let mut i = 0usize;
    while i + 8 <= overlap2 {
        let rev_idx = overlap - 8 - i;
        unsafe {
            let x2 = _mm256_loadu_ps(output.as_ptr().add(i));
            let x1 = avx_rev_ps(_mm256_loadu_ps(output.as_ptr().add(rev_idx)));
            let w1 = _mm256_loadu_ps(window.as_ptr().add(i));
            let w2 = avx_rev_ps(_mm256_loadu_ps(window.as_ptr().add(rev_idx)));

            let out_fwd = _mm256_sub_ps(_mm256_mul_ps(x2, w2), _mm256_mul_ps(x1, w1));
            let out_rev = _mm256_add_ps(_mm256_mul_ps(x2, w1), _mm256_mul_ps(x1, w2));

            _mm256_storeu_ps(output.as_mut_ptr().add(i), out_fwd);
            _mm256_storeu_ps(output.as_mut_ptr().add(rev_idx), avx_rev_ps(out_rev));
        }
        i += 8;
    }
    for i in i..overlap2 {
        let x1 = output[overlap - 1 - i];
        let x2 = output[i];
        let wp1 = window[i];
        let wp2 = window[overlap - 1 - i];
        output[i] = x2 * wp2 - x1 * wp1;
        output[overlap - 1 - i] = x2 * wp1 + x1 * wp2;
    }
}

// ---------------------------------------------------------------------------
// CELT Core, Bands & PVQ
// ---------------------------------------------------------------------------

#[inline]
pub fn l1_metric(tmp: &[f32], n: usize, lm: i32, bias: f32) -> f32 {
    let tmp = &tmp[..n];
    #[cfg(target_arch = "aarch64")]
    {
        if n >= 16 {
            return unsafe { l1_metric_neon(tmp, lm, bias) };
        }
    }
    #[cfg(target_arch = "x86_64")]
    {
        if n >= 16 && have_avx() {
            return unsafe { l1_metric_avx(tmp, lm, bias) };
        }
    }
    let mut l1 = 0.0f32;
    for &tv in tmp {
        l1 += tv.abs();
    }
    l1 + (lm as f32) * bias * l1
}

#[cfg(target_arch = "aarch64")]
#[target_feature(enable = "neon")]
#[inline]
unsafe fn l1_metric_neon(tmp: &[f32], lm: i32, bias: f32) -> f32 {
    use std::arch::aarch64::*;
    let n = tmp.len();
    let mut sum0 = vdupq_n_f32(0.0);
    let mut sum1 = vdupq_n_f32(0.0);
    let mut i = 0usize;
    while i + 16 <= n {
        unsafe {
            let v0 = vld1q_f32(tmp.as_ptr().add(i));
            let v1 = vld1q_f32(tmp.as_ptr().add(i + 4));
            let v2 = vld1q_f32(tmp.as_ptr().add(i + 8));
            let v3 = vld1q_f32(tmp.as_ptr().add(i + 12));
            sum0 = vaddq_f32(sum0, vaddq_f32(vabsq_f32(v0), vabsq_f32(v1)));
            sum1 = vaddq_f32(sum1, vaddq_f32(vabsq_f32(v2), vabsq_f32(v3)));
        }
        i += 16;
    }
    while i + 4 <= n {
        unsafe {
            let v = vld1q_f32(tmp.as_ptr().add(i));
            sum0 = vaddq_f32(sum0, vabsq_f32(v));
        }
        i += 4;
    }
    let mut l1 = vaddvq_f32(vaddq_f32(sum0, sum1));
    for &tv in &tmp[i..] {
        l1 += tv.abs();
    }
    l1 + (lm as f32) * bias * l1
}

#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx")]
unsafe fn l1_metric_avx(tmp: &[f32], lm: i32, bias: f32) -> f32 {
    use std::arch::x86_64::*;
    let n = tmp.len();
    let mut sum0 = _mm256_setzero_ps();
    let mut sum1 = _mm256_setzero_ps();
    let sign_mask = _mm256_set1_ps(-0.0);
    let mut i = 0usize;
    while i + 16 <= n {
        unsafe {
            let v0 = _mm256_loadu_ps(tmp.as_ptr().add(i));
            let v1 = _mm256_loadu_ps(tmp.as_ptr().add(i + 8));
            sum0 = _mm256_add_ps(sum0, _mm256_andnot_ps(sign_mask, v0));
            sum1 = _mm256_add_ps(sum1, _mm256_andnot_ps(sign_mask, v1));
        }
        i += 16;
    }
    while i + 8 <= n {
        unsafe {
            let v = _mm256_loadu_ps(tmp.as_ptr().add(i));
            sum0 = _mm256_add_ps(sum0, _mm256_andnot_ps(sign_mask, v));
        }
        i += 8;
    }
    let sum = _mm256_add_ps(sum0, sum1);
    let s4 = _mm_add_ps(_mm256_castps256_ps128(sum), _mm256_extractf128_ps(sum, 1));
    let s2 = _mm_add_ps(s4, _mm_movehl_ps(s4, s4));
    let mut l1 = _mm_cvtss_f32(_mm_add_ss(s2, _mm_shuffle_ps(s2, s2, 0x55)));
    for &tv in &tmp[i..] {
        l1 += tv.abs();
    }
    l1 + (lm as f32) * bias * l1
}

#[allow(clippy::too_many_arguments)]
#[inline]
pub fn comb_filter_const(
    y: &mut [f32],
    x: &[f32],
    y_idx: usize,
    x_idx: usize,
    t: usize,
    n: usize,
    g10: f32,
    g11: f32,
    g12: f32,
) {
    if n == 0 {
        return;
    }
    assert!(
        t >= 2 && x_idx >= t + 2 && x_idx + n <= x.len() && y_idx + n <= y.len(),
        "comb_filter_const: buffer out of bounds"
    );
    #[cfg(target_arch = "aarch64")]
    {
        unsafe { comb_filter_const_neon(y, x, y_idx, x_idx, t, n, g10, g11, g12) };
        return;
    }
    #[cfg(target_arch = "x86_64")]
    {
        if have_avx_fma() {
            unsafe { comb_filter_const_avx(y, x, y_idx, x_idx, t, n, g10, g11, g12) };
            return;
        }
        unsafe { comb_filter_const_sse2(y, x, y_idx, x_idx, t, n, g10, g11, g12) };
        return;
    }
    #[allow(unreachable_code)]
    comb_filter_const_scalar(y, x, y_idx, x_idx, t, n, g10, g11, g12);
}

#[allow(clippy::too_many_arguments)]
fn comb_filter_const_scalar(
    y: &mut [f32],
    x: &[f32],
    y_idx: usize,
    x_idx: usize,
    t: usize,
    n: usize,
    g10: f32,
    g11: f32,
    g12: f32,
) {
    let mut x4 = x[x_idx - t - 2];
    let mut x3 = x[x_idx - t - 1];
    let mut x2 = x[x_idx - t];
    let mut x1 = x[x_idx - t + 1];
    for i in 0..n {
        let x0 = x[x_idx + i - t + 2];
        y[y_idx + i] = x[x_idx + i] + g10 * x2 + g11 * (x1 + x3) + g12 * (x0 + x4);
        x4 = x3;
        x3 = x2;
        x2 = x1;
        x1 = x0;
    }
}

#[cfg(target_arch = "aarch64")]
#[allow(clippy::too_many_arguments)]
#[target_feature(enable = "neon")]
#[inline]
unsafe fn comb_filter_const_neon(
    y: &mut [f32],
    x: &[f32],
    y_idx: usize,
    x_idx: usize,
    t: usize,
    n: usize,
    g10: f32,
    g11: f32,
    g12: f32,
) {
    use std::arch::aarch64::*;
    let g10v = vdupq_n_f32(g10);
    let g11v = vdupq_n_f32(g11);
    let g12v = vdupq_n_f32(g12);
    let xspan = &x[x_idx - t - 2..x_idx + n];
    let xbase = unsafe { xspan.as_ptr().add(t + 2) };
    let ybase = y[y_idx..y_idx + n].as_mut_ptr();
    let mut x0v = unsafe { vld1q_f32(xbase.sub(t + 2)) };
    let mut i = 0usize;
    while i + 4 <= n {
        unsafe {
            let x4v = vld1q_f32(xbase.add(i).sub(t - 2));
            let x2v = vextq_f32(x0v, x4v, 2);
            let x1v = vextq_f32(x0v, x4v, 1);
            let x3v = vextq_f32(x0v, x4v, 3);
            let xi = vld1q_f32(xbase.add(i));
            let mut yi = vfmaq_f32(xi, g10v, x2v);
            yi = vfmaq_f32(yi, g11v, vaddq_f32(x1v, x3v));
            yi = vfmaq_f32(yi, g12v, vaddq_f32(x4v, x0v));
            vst1q_f32(ybase.add(i), yi);
            x0v = x4v;
        }
        i += 4;
    }
    let mut x0v_arr = [0.0f32; 4];
    unsafe { vst1q_f32(x0v_arr.as_mut_ptr(), x0v) };
    let mut sx4 = x0v_arr[0];
    let mut sx3 = x0v_arr[1];
    let mut sx2 = x0v_arr[2];
    let mut sx1 = x0v_arr[3];
    while i < n {
        let sx0 = x[x_idx + i - t + 2];
        y[y_idx + i] = x[x_idx + i] + g10 * sx2 + g11 * (sx1 + sx3) + g12 * (sx0 + sx4);
        sx4 = sx3;
        sx3 = sx2;
        sx2 = sx1;
        sx1 = sx0;
        i += 1;
    }
}

#[cfg(target_arch = "x86_64")]
#[allow(clippy::too_many_arguments)]
#[target_feature(enable = "sse2")]
unsafe fn comb_filter_const_sse2(
    y: &mut [f32],
    x: &[f32],
    y_idx: usize,
    x_idx: usize,
    t: usize,
    n: usize,
    g10: f32,
    g11: f32,
    g12: f32,
) {
    use std::arch::x86_64::*;
    let g10v = _mm_set1_ps(g10);
    let g11v = _mm_set1_ps(g11);
    let g12v = _mm_set1_ps(g12);
    let xspan = &x[x_idx - t - 2..x_idx + n];
    let xbase = unsafe { xspan.as_ptr().add(t + 2) };
    let ybase = y[y_idx..y_idx + n].as_mut_ptr();
    let mut x0v = unsafe { _mm_loadu_ps(xbase.sub(t + 2)) };
    let mut i = 0usize;
    while i + 4 <= n {
        unsafe {
            let x4v = _mm_loadu_ps(xbase.add(i).sub(t - 2));
            let x2v = _mm_shuffle_ps(x0v, x4v, 0x4e);
            let x1v = _mm_shuffle_ps(x0v, x2v, 0x99);
            let x3v = _mm_shuffle_ps(x2v, x4v, 0x99);
            let xi = _mm_loadu_ps(xbase.add(i));
            let yi = _mm_add_ps(
                _mm_add_ps(xi, _mm_mul_ps(g10v, x2v)),
                _mm_add_ps(
                    _mm_mul_ps(g11v, _mm_add_ps(x3v, x1v)),
                    _mm_mul_ps(g12v, _mm_add_ps(x4v, x0v)),
                ),
            );
            _mm_storeu_ps(ybase.add(i), yi);
            x0v = x4v;
        }
        i += 4;
    }
    let mut x0v_arr = [0.0f32; 4];
    unsafe { _mm_storeu_ps(x0v_arr.as_mut_ptr(), x0v) };
    let mut sx4 = x0v_arr[0];
    let mut sx3 = x0v_arr[1];
    let mut sx2 = x0v_arr[2];
    let mut sx1 = x0v_arr[3];
    while i < n {
        let sx0 = x[x_idx + i - t + 2];
        y[y_idx + i] = x[x_idx + i] + g10 * sx2 + g11 * (sx1 + sx3) + g12 * (sx0 + sx4);
        sx4 = sx3;
        sx3 = sx2;
        sx2 = sx1;
        sx1 = sx0;
        i += 1;
    }
}

#[cfg(target_arch = "x86_64")]
#[allow(clippy::too_many_arguments)]
#[target_feature(enable = "avx,fma")]
unsafe fn comb_filter_const_avx(
    y: &mut [f32],
    x: &[f32],
    y_idx: usize,
    x_idx: usize,
    t: usize,
    n: usize,
    g10: f32,
    g11: f32,
    g12: f32,
) {
    use std::arch::x86_64::*;
    let g10v = _mm256_set1_ps(g10);
    let g11v = _mm256_set1_ps(g11);
    let g12v = _mm256_set1_ps(g12);
    let xspan = &x[x_idx - t - 2..x_idx + n];
    let xbase = unsafe { xspan.as_ptr().add(t + 2) };
    let ybase = y[y_idx..y_idx + n].as_mut_ptr();
    let mut i = 0usize;
    while i + 16 <= n {
        unsafe {
            let xi_a = _mm256_loadu_ps(xbase.add(i));
            let x0_a = _mm256_loadu_ps(xbase.add(i).sub(t + 2));
            let x4_a = _mm256_loadu_ps(xbase.add(i).sub(t - 2));
            let x2_a = _mm256_loadu_ps(xbase.add(i).sub(t));
            let x1x3_a = _mm256_add_ps(
                _mm256_loadu_ps(xbase.add(i).sub(t + 1)),
                _mm256_loadu_ps(xbase.add(i).sub(t - 1)),
            );
            let mut yi_a = _mm256_fmadd_ps(g10v, x2_a, xi_a);
            yi_a = _mm256_fmadd_ps(g11v, x1x3_a, yi_a);
            yi_a = _mm256_fmadd_ps(g12v, _mm256_add_ps(x0_a, x4_a), yi_a);
            _mm256_storeu_ps(ybase.add(i), yi_a);

            let j = i + 8;
            let xi_b = _mm256_loadu_ps(xbase.add(j));
            let x0_b = _mm256_loadu_ps(xbase.add(j).sub(t + 2));
            let x4_b = _mm256_loadu_ps(xbase.add(j).sub(t - 2));
            let x2_b = _mm256_loadu_ps(xbase.add(j).sub(t));
            let x1x3_b = _mm256_add_ps(
                _mm256_loadu_ps(xbase.add(j).sub(t + 1)),
                _mm256_loadu_ps(xbase.add(j).sub(t - 1)),
            );
            let mut yi_b = _mm256_fmadd_ps(g10v, x2_b, xi_b);
            yi_b = _mm256_fmadd_ps(g11v, x1x3_b, yi_b);
            yi_b = _mm256_fmadd_ps(g12v, _mm256_add_ps(x0_b, x4_b), yi_b);
            _mm256_storeu_ps(ybase.add(j), yi_b);
        }
        i += 16;
    }
    while i + 8 <= n {
        unsafe {
            let xi = _mm256_loadu_ps(xbase.add(i));
            let x0 = _mm256_loadu_ps(xbase.add(i).sub(t + 2));
            let x4 = _mm256_loadu_ps(xbase.add(i).sub(t - 2));
            let x2 = _mm256_loadu_ps(xbase.add(i).sub(t));
            let x1x3 = _mm256_add_ps(
                _mm256_loadu_ps(xbase.add(i).sub(t + 1)),
                _mm256_loadu_ps(xbase.add(i).sub(t - 1)),
            );
            let mut yi = _mm256_fmadd_ps(g10v, x2, xi);
            yi = _mm256_fmadd_ps(g11v, x1x3, yi);
            yi = _mm256_fmadd_ps(g12v, _mm256_add_ps(x0, x4), yi);
            _mm256_storeu_ps(ybase.add(i), yi);
        }
        i += 8;
    }
    if i < n {
        comb_filter_const_scalar(y, x, y_idx + i, x_idx + i, t, n - i, g10, g11, g12);
    }
}

/// Bit-exact in-place comb filter constant region (`s + g10*r1 + g11*(r1p1+r1m1) + g12*(r1p2+r1m2)`
/// without FMA so every rounding matches the scalar loop). Returns the index `i` where scalar resumes.
#[allow(clippy::too_many_arguments)]
#[inline]
pub fn comb_filter_const_inplace(
    buf: &mut [f32],
    y_idx: usize,
    i: usize,
    n: usize,
    t1: usize,
    g10: f32,
    g11: f32,
    g12: f32,
) -> usize {
    if i >= n || t1 < 2 || y_idx + i < t1 + 2 || y_idx + n > buf.len() {
        return i;
    }
    #[cfg(target_arch = "aarch64")]
    {
        if i + 4 <= n && t1 >= 6 {
            return unsafe { comb_filter_const_inplace_neon(buf, y_idx, i, n, t1, g10, g11, g12) };
        }
    }
    #[cfg(target_arch = "x86_64")]
    {
        if i + 8 <= n && t1 >= 10 && have_avx2() {
            return unsafe { comb_filter_const_inplace_avx2(buf, y_idx, i, n, t1, g10, g11, g12) };
        }
    }
    i
}

#[cfg(target_arch = "aarch64")]
#[allow(clippy::too_many_arguments)]
#[target_feature(enable = "neon")]
#[inline]
unsafe fn comb_filter_const_inplace_neon(
    buf: &mut [f32],
    y_idx: usize,
    mut i: usize,
    n: usize,
    t1: usize,
    g10: f32,
    g11: f32,
    g12: f32,
) -> usize {
    use std::arch::aarch64::*;
    assert!(t1 >= 6 && y_idx + i >= t1 + 2 && y_idx + n <= buf.len());
    let vg10 = vdupq_n_f32(g10);
    let vg11 = vdupq_n_f32(g11);
    let vg12 = vdupq_n_f32(g12);
    let p = buf.as_mut_ptr();
    while i + 4 <= n {
        let idx = y_idx + i;
        let base = idx - t1;
        unsafe {
            let s = vld1q_f32(p.add(idx));
            let r1 = vld1q_f32(p.add(base));
            let r1p1 = vld1q_f32(p.add(base + 1));
            let r1m1 = vld1q_f32(p.add(base - 1));
            let r1p2 = vld1q_f32(p.add(base + 2));
            let r1m2 = vld1q_f32(p.add(base - 2));
            let a = vaddq_f32(r1p1, r1m1);
            let b = vaddq_f32(r1p2, r1m2);
            // Separate mul + add (no FMA) to match scalar rounding order bit-for-bit.
            let mut out = vaddq_f32(s, vmulq_f32(vg10, r1));
            out = vaddq_f32(out, vmulq_f32(vg11, a));
            out = vaddq_f32(out, vmulq_f32(vg12, b));
            vst1q_f32(p.add(idx), out);
        }
        i += 4;
    }
    i
}

#[cfg(target_arch = "x86_64")]
#[allow(clippy::too_many_arguments)]
#[target_feature(enable = "avx2")]
unsafe fn comb_filter_const_inplace_avx2(
    buf: &mut [f32],
    y_idx: usize,
    mut i: usize,
    n: usize,
    t1: usize,
    g10: f32,
    g11: f32,
    g12: f32,
) -> usize {
    use std::arch::x86_64::*;
    assert!(t1 >= 10 && y_idx + i >= t1 + 2 && y_idx + n <= buf.len());
    let vg10 = _mm256_set1_ps(g10);
    let vg11 = _mm256_set1_ps(g11);
    let vg12 = _mm256_set1_ps(g12);
    let p = buf.as_mut_ptr();
    while i + 8 <= n {
        let idx = y_idx + i;
        let base = idx - t1;
        unsafe {
            let s = _mm256_loadu_ps(p.add(idx));
            let r1 = _mm256_loadu_ps(p.add(base));
            let r1p1 = _mm256_loadu_ps(p.add(base + 1));
            let r1m1 = _mm256_loadu_ps(p.add(base - 1));
            let r1p2 = _mm256_loadu_ps(p.add(base + 2));
            let r1m2 = _mm256_loadu_ps(p.add(base - 2));
            let a = _mm256_add_ps(r1p1, r1m1);
            let b = _mm256_add_ps(r1p2, r1m2);
            let mut out = _mm256_add_ps(s, _mm256_mul_ps(vg10, r1));
            out = _mm256_add_ps(out, _mm256_mul_ps(vg11, a));
            out = _mm256_add_ps(out, _mm256_mul_ps(vg12, b));
            _mm256_storeu_ps(p.add(idx), out);
        }
        i += 8;
    }
    i
}

/// Computes `(emid, eside)` (seeded with `1e-15`) for CELT `stereo_itheta`.
#[inline]
pub fn stereo_energy_pair(x: &[f32], y: &[f32], stereo: bool, n: usize) -> (f32, f32) {
    let x = &x[..n];
    let y = &y[..n];
    #[cfg(target_arch = "aarch64")]
    {
        return unsafe { stereo_energy_pair_neon(x, y, stereo) };
    }
    #[cfg(target_arch = "x86_64")]
    {
        if n >= 8 && have_avx2_fma() {
            return unsafe { stereo_energy_pair_avx2(x, y, stereo) };
        }
    }
    #[cfg(not(target_arch = "aarch64"))]
    {
        let mut emid = 1e-15f32;
        let mut eside = 1e-15f32;
        if stereo {
            for i in 0..n {
                let m = x[i] + y[i];
                let s = x[i] - y[i];
                emid += m * m;
                eside += s * s;
            }
        } else {
            for i in 0..n {
                emid += x[i] * x[i];
                eside += y[i] * y[i];
            }
        }
        (emid, eside)
    }
}

#[cfg(target_arch = "aarch64")]
#[target_feature(enable = "neon")]
#[inline]
unsafe fn stereo_energy_pair_neon(x: &[f32], y: &[f32], stereo: bool) -> (f32, f32) {
    use std::arch::aarch64::*;
    let n = x.len();
    let mut emid = 1e-15f32;
    let mut eside = 1e-15f32;
    let mut sum_mid = vdupq_n_f32(0.0);
    let mut sum_side = vdupq_n_f32(0.0);
    let mut i = 0usize;
    if stereo {
        while i + 16 <= n {
            unsafe {
                let x0 = vld1q_f32(x.as_ptr().add(i));
                let x1 = vld1q_f32(x.as_ptr().add(i + 4));
                let x2 = vld1q_f32(x.as_ptr().add(i + 8));
                let x3 = vld1q_f32(x.as_ptr().add(i + 12));
                let y0 = vld1q_f32(y.as_ptr().add(i));
                let y1 = vld1q_f32(y.as_ptr().add(i + 4));
                let y2 = vld1q_f32(y.as_ptr().add(i + 8));
                let y3 = vld1q_f32(y.as_ptr().add(i + 12));
                let m0 = vaddq_f32(x0, y0);
                let m1 = vaddq_f32(x1, y1);
                let m2 = vaddq_f32(x2, y2);
                let m3 = vaddq_f32(x3, y3);
                let s0 = vsubq_f32(x0, y0);
                let s1 = vsubq_f32(x1, y1);
                let s2 = vsubq_f32(x2, y2);
                let s3 = vsubq_f32(x3, y3);
                sum_mid = vfmaq_f32(sum_mid, m0, m0);
                sum_mid = vfmaq_f32(sum_mid, m1, m1);
                sum_mid = vfmaq_f32(sum_mid, m2, m2);
                sum_mid = vfmaq_f32(sum_mid, m3, m3);
                sum_side = vfmaq_f32(sum_side, s0, s0);
                sum_side = vfmaq_f32(sum_side, s1, s1);
                sum_side = vfmaq_f32(sum_side, s2, s2);
                sum_side = vfmaq_f32(sum_side, s3, s3);
            }
            i += 16;
        }
        while i + 4 <= n {
            unsafe {
                let x0 = vld1q_f32(x.as_ptr().add(i));
                let y0 = vld1q_f32(y.as_ptr().add(i));
                let m0 = vaddq_f32(x0, y0);
                let s0 = vsubq_f32(x0, y0);
                sum_mid = vfmaq_f32(sum_mid, m0, m0);
                sum_side = vfmaq_f32(sum_side, s0, s0);
            }
            i += 4;
        }
        emid += vaddvq_f32(sum_mid);
        eside += vaddvq_f32(sum_side);
        for j in i..n {
            let m = x[j] + y[j];
            let s = x[j] - y[j];
            emid += m * m;
            eside += s * s;
        }
    } else {
        while i + 16 <= n {
            unsafe {
                let x0 = vld1q_f32(x.as_ptr().add(i));
                let x1 = vld1q_f32(x.as_ptr().add(i + 4));
                let x2 = vld1q_f32(x.as_ptr().add(i + 8));
                let x3 = vld1q_f32(x.as_ptr().add(i + 12));
                let y0 = vld1q_f32(y.as_ptr().add(i));
                let y1 = vld1q_f32(y.as_ptr().add(i + 4));
                let y2 = vld1q_f32(y.as_ptr().add(i + 8));
                let y3 = vld1q_f32(y.as_ptr().add(i + 12));
                sum_mid = vfmaq_f32(sum_mid, x0, x0);
                sum_mid = vfmaq_f32(sum_mid, x1, x1);
                sum_mid = vfmaq_f32(sum_mid, x2, x2);
                sum_mid = vfmaq_f32(sum_mid, x3, x3);
                sum_side = vfmaq_f32(sum_side, y0, y0);
                sum_side = vfmaq_f32(sum_side, y1, y1);
                sum_side = vfmaq_f32(sum_side, y2, y2);
                sum_side = vfmaq_f32(sum_side, y3, y3);
            }
            i += 16;
        }
        while i + 4 <= n {
            unsafe {
                let x0 = vld1q_f32(x.as_ptr().add(i));
                let y0 = vld1q_f32(y.as_ptr().add(i));
                sum_mid = vfmaq_f32(sum_mid, x0, x0);
                sum_side = vfmaq_f32(sum_side, y0, y0);
            }
            i += 4;
        }
        emid += vaddvq_f32(sum_mid);
        eside += vaddvq_f32(sum_side);
        for j in i..n {
            emid += x[j] * x[j];
            eside += y[j] * y[j];
        }
    }
    (emid, eside)
}

#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx")]
#[inline]
unsafe fn avx_hsum_ps(v: std::arch::x86_64::__m256) -> f32 {
    use std::arch::x86_64::*;
    let s4 = _mm_add_ps(_mm256_castps256_ps128(v), _mm256_extractf128_ps(v, 1));
    let s2 = _mm_add_ps(s4, _mm_movehl_ps(s4, s4));
    _mm_cvtss_f32(_mm_add_ss(s2, _mm_shuffle_ps(s2, s2, 0x55)))
}

#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx2,fma")]
unsafe fn stereo_energy_pair_avx2(x: &[f32], y: &[f32], stereo: bool) -> (f32, f32) {
    use std::arch::x86_64::*;
    let n = x.len();
    let mut sum_mid = _mm256_setzero_ps();
    let mut sum_side = _mm256_setzero_ps();
    let mut i = 0usize;
    if stereo {
        while i + 8 <= n {
            unsafe {
                let vx = _mm256_loadu_ps(x.as_ptr().add(i));
                let vy = _mm256_loadu_ps(y.as_ptr().add(i));
                let m = _mm256_add_ps(vx, vy);
                let s = _mm256_sub_ps(vx, vy);
                sum_mid = _mm256_fmadd_ps(m, m, sum_mid);
                sum_side = _mm256_fmadd_ps(s, s, sum_side);
            }
            i += 8;
        }
    } else {
        while i + 8 <= n {
            unsafe {
                let vx = _mm256_loadu_ps(x.as_ptr().add(i));
                let vy = _mm256_loadu_ps(y.as_ptr().add(i));
                sum_mid = _mm256_fmadd_ps(vx, vx, sum_mid);
                sum_side = _mm256_fmadd_ps(vy, vy, sum_side);
            }
            i += 8;
        }
    }
    let mut emid = 1e-15f32 + unsafe { avx_hsum_ps(sum_mid) };
    let mut eside = 1e-15f32 + unsafe { avx_hsum_ps(sum_side) };
    if stereo {
        for j in i..n {
            let m = x[j] + y[j];
            let s = x[j] - y[j];
            emid += m * m;
            eside += s * s;
        }
    } else {
        for j in i..n {
            emid += x[j] * x[j];
            eside += y[j] * y[j];
        }
    }
    (emid, eside)
}

#[inline]
pub fn compute_band_energy(band: &[f32]) -> f32 {
    #[cfg(target_arch = "aarch64")]
    {
        return unsafe { compute_band_energy_neon(band) };
    }
    #[cfg(target_arch = "x86_64")]
    {
        if band.len() >= 8 && have_avx2_fma() {
            return unsafe { compute_band_energy_avx2(band) };
        }
    }
    #[cfg(not(target_arch = "aarch64"))]
    {
        let sum = band.iter().fold(1e-27f32, |acc, &v| acc + v * v);
        sum.sqrt()
    }
}

#[cfg(target_arch = "aarch64")]
#[target_feature(enable = "neon")]
#[inline]
unsafe fn compute_band_energy_neon(band: &[f32]) -> f32 {
    use std::arch::aarch64::*;
    let n = band.len();
    let mut sum = 1e-27f32;
    let n16 = n & !15;
    if n16 > 0 {
        let mut acc0 = vdupq_n_f32(0.0);
        let mut acc1 = vdupq_n_f32(0.0);
        let mut acc2 = vdupq_n_f32(0.0);
        let mut acc3 = vdupq_n_f32(0.0);
        for i in (0..n16).step_by(16) {
            unsafe {
                let v0 = vld1q_f32(band.as_ptr().add(i));
                let v1 = vld1q_f32(band.as_ptr().add(i + 4));
                let v2 = vld1q_f32(band.as_ptr().add(i + 8));
                let v3 = vld1q_f32(band.as_ptr().add(i + 12));
                acc0 = vfmaq_f32(acc0, v0, v0);
                acc1 = vfmaq_f32(acc1, v1, v1);
                acc2 = vfmaq_f32(acc2, v2, v2);
                acc3 = vfmaq_f32(acc3, v3, v3);
            }
        }
        sum += vaddvq_f32(vaddq_f32(vaddq_f32(acc0, acc1), vaddq_f32(acc2, acc3)));
    }
    let n4 = (n & !3) - n16;
    if n4 > 0 {
        let mut acc = vdupq_n_f32(0.0);
        for i in (n16..n16 + n4).step_by(4) {
            unsafe {
                let v = vld1q_f32(band.as_ptr().add(i));
                acc = vfmaq_f32(acc, v, v);
            }
        }
        sum += vaddvq_f32(acc);
    }
    for &v in &band[(n16 + n4)..] {
        sum += v * v;
    }
    sum.sqrt()
}

#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx2,fma")]
unsafe fn compute_band_energy_avx2(band: &[f32]) -> f32 {
    use std::arch::x86_64::*;
    let n = band.len();
    let mut i = 0usize;
    let mut acc0 = _mm256_setzero_ps();
    let mut acc1 = _mm256_setzero_ps();
    while i + 16 <= n {
        unsafe {
            let v0 = _mm256_loadu_ps(band.as_ptr().add(i));
            let v1 = _mm256_loadu_ps(band.as_ptr().add(i + 8));
            acc0 = _mm256_fmadd_ps(v0, v0, acc0);
            acc1 = _mm256_fmadd_ps(v1, v1, acc1);
        }
        i += 16;
    }
    if i + 8 <= n {
        unsafe {
            let v0 = _mm256_loadu_ps(band.as_ptr().add(i));
            acc0 = _mm256_fmadd_ps(v0, v0, acc0);
        }
        i += 8;
    }
    let acc = _mm256_add_ps(acc0, acc1);
    let s4 = _mm_add_ps(_mm256_castps256_ps128(acc), _mm256_extractf128_ps(acc, 1));
    let s2 = _mm_add_ps(s4, _mm_movehl_ps(s4, s4));
    let mut sum = 1e-27f32 + _mm_cvtss_f32(_mm_add_ss(s2, _mm_shuffle_ps(s2, s2, 0x55)));
    for &v in &band[i..] {
        sum += v * v;
    }
    sum.sqrt()
}

#[inline]
pub fn scale_slice(src: &[f32], dst: &mut [f32], scale: f32, n: usize) {
    let src = &src[..n];
    let dst = &mut dst[..n];
    #[cfg(target_arch = "aarch64")]
    {
        if n >= 8 {
            unsafe { scale_slice_neon(src, dst, scale) };
            return;
        }
    }
    #[cfg(target_arch = "x86_64")]
    {
        if n >= 8 && have_avx2() {
            unsafe { scale_slice_avx2(src, dst, scale) };
            return;
        }
    }
    for (d, &s) in dst.iter_mut().zip(src) {
        *d = s * scale;
    }
}

#[cfg(target_arch = "aarch64")]
#[target_feature(enable = "neon")]
#[inline]
unsafe fn scale_slice_neon(src: &[f32], dst: &mut [f32], scale: f32) {
    use std::arch::aarch64::*;
    let n = src.len();
    let vscale = vdupq_n_f32(scale);
    let mut i = 0usize;
    while i + 16 <= n {
        unsafe {
            let s0 = vld1q_f32(src.as_ptr().add(i));
            let s1 = vld1q_f32(src.as_ptr().add(i + 4));
            let s2 = vld1q_f32(src.as_ptr().add(i + 8));
            let s3 = vld1q_f32(src.as_ptr().add(i + 12));
            vst1q_f32(dst.as_mut_ptr().add(i), vmulq_f32(s0, vscale));
            vst1q_f32(dst.as_mut_ptr().add(i + 4), vmulq_f32(s1, vscale));
            vst1q_f32(dst.as_mut_ptr().add(i + 8), vmulq_f32(s2, vscale));
            vst1q_f32(dst.as_mut_ptr().add(i + 12), vmulq_f32(s3, vscale));
        }
        i += 16;
    }
    while i + 4 <= n {
        unsafe {
            let s0 = vld1q_f32(src.as_ptr().add(i));
            vst1q_f32(dst.as_mut_ptr().add(i), vmulq_f32(s0, vscale));
        }
        i += 4;
    }
    for j in i..n {
        dst[j] = src[j] * scale;
    }
}

#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx2")]
unsafe fn scale_slice_avx2(src: &[f32], dst: &mut [f32], scale: f32) {
    use std::arch::x86_64::*;
    let n = src.len();
    let vscale = _mm256_set1_ps(scale);
    let mut i = 0usize;
    while i + 16 <= n {
        unsafe {
            let s0 = _mm256_loadu_ps(src.as_ptr().add(i));
            let s1 = _mm256_loadu_ps(src.as_ptr().add(i + 8));
            _mm256_storeu_ps(dst.as_mut_ptr().add(i), _mm256_mul_ps(s0, vscale));
            _mm256_storeu_ps(dst.as_mut_ptr().add(i + 8), _mm256_mul_ps(s1, vscale));
        }
        i += 16;
    }
    while i + 8 <= n {
        unsafe {
            let sv = _mm256_loadu_ps(src.as_ptr().add(i));
            _mm256_storeu_ps(dst.as_mut_ptr().add(i), _mm256_mul_ps(sv, vscale));
        }
        i += 8;
    }
    for j in i..n {
        dst[j] = src[j] * scale;
    }
}

#[inline]
pub fn renormalise_vector(x: &mut [f32], n: usize, gain: f32) {
    let x = &mut x[..n];
    #[cfg(target_arch = "aarch64")]
    {
        unsafe { renormalise_vector_neon(x, gain) };
        return;
    }
    #[cfg(target_arch = "x86_64")]
    {
        if n >= 16 && have_avx2_fma() {
            unsafe { renormalise_vector_avx2(x, gain) };
            return;
        }
    }
    #[cfg(not(target_arch = "aarch64"))]
    {
        let mut e = 1e-15f32;
        for &xv in x.iter() {
            e += xv * xv;
        }
        let norm = gain / e.sqrt();
        for xv in x.iter_mut() {
            *xv *= norm;
        }
    }
}

#[cfg(target_arch = "aarch64")]
#[target_feature(enable = "neon")]
#[inline]
unsafe fn renormalise_vector_neon(x: &mut [f32], gain: f32) {
    use std::arch::aarch64::*;
    let n = x.len();
    let mut sum_vec = vdupq_n_f32(0.0);
    let mut i = 0usize;
    while i + 16 <= n {
        unsafe {
            let x0 = vld1q_f32(x.as_ptr().add(i));
            let x1 = vld1q_f32(x.as_ptr().add(i + 4));
            let x2 = vld1q_f32(x.as_ptr().add(i + 8));
            let x3 = vld1q_f32(x.as_ptr().add(i + 12));
            sum_vec = vfmaq_f32(sum_vec, x0, x0);
            sum_vec = vfmaq_f32(sum_vec, x1, x1);
            sum_vec = vfmaq_f32(sum_vec, x2, x2);
            sum_vec = vfmaq_f32(sum_vec, x3, x3);
        }
        i += 16;
    }
    while i + 4 <= n {
        unsafe {
            let x0 = vld1q_f32(x.as_ptr().add(i));
            sum_vec = vfmaq_f32(sum_vec, x0, x0);
        }
        i += 4;
    }
    let mut e = 1e-15f32 + vaddvq_f32(sum_vec);
    for &xv in &x[i..] {
        e += xv * xv;
    }
    let norm = gain / e.sqrt();
    let vnorm = vdupq_n_f32(norm);
    i = 0;
    while i + 16 <= n {
        unsafe {
            let x0 = vld1q_f32(x.as_ptr().add(i));
            let x1 = vld1q_f32(x.as_ptr().add(i + 4));
            let x2 = vld1q_f32(x.as_ptr().add(i + 8));
            let x3 = vld1q_f32(x.as_ptr().add(i + 12));
            vst1q_f32(x.as_mut_ptr().add(i), vmulq_f32(x0, vnorm));
            vst1q_f32(x.as_mut_ptr().add(i + 4), vmulq_f32(x1, vnorm));
            vst1q_f32(x.as_mut_ptr().add(i + 8), vmulq_f32(x2, vnorm));
            vst1q_f32(x.as_mut_ptr().add(i + 12), vmulq_f32(x3, vnorm));
        }
        i += 16;
    }
    while i + 4 <= n {
        unsafe {
            let x0 = vld1q_f32(x.as_ptr().add(i));
            vst1q_f32(x.as_mut_ptr().add(i), vmulq_f32(x0, vnorm));
        }
        i += 4;
    }
    for xv in &mut x[i..] {
        *xv *= norm;
    }
}

#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx2,fma")]
unsafe fn renormalise_vector_avx2(x: &mut [f32], gain: f32) {
    use std::arch::x86_64::*;
    let n = x.len();
    let mut acc0 = _mm256_setzero_ps();
    let mut acc1 = _mm256_setzero_ps();
    let mut i = 0usize;
    while i + 16 <= n {
        unsafe {
            let v0 = _mm256_loadu_ps(x.as_ptr().add(i));
            let v1 = _mm256_loadu_ps(x.as_ptr().add(i + 8));
            acc0 = _mm256_fmadd_ps(v0, v0, acc0);
            acc1 = _mm256_fmadd_ps(v1, v1, acc1);
        }
        i += 16;
    }
    if i + 8 <= n {
        unsafe {
            let v0 = _mm256_loadu_ps(x.as_ptr().add(i));
            acc0 = _mm256_fmadd_ps(v0, v0, acc0);
        }
        i += 8;
    }
    let acc = _mm256_add_ps(acc0, acc1);
    let s4 = _mm_add_ps(_mm256_castps256_ps128(acc), _mm256_extractf128_ps(acc, 1));
    let s2 = _mm_add_ps(s4, _mm_movehl_ps(s4, s4));
    let mut e = 1e-15f32 + _mm_cvtss_f32(_mm_add_ss(s2, _mm_shuffle_ps(s2, s2, 0x55)));
    for &v in &x[i..] {
        e += v * v;
    }
    let norm = gain / e.sqrt();
    let vnorm = _mm256_set1_ps(norm);
    i = 0;
    while i + 16 <= n {
        unsafe {
            let v0 = _mm256_loadu_ps(x.as_ptr().add(i));
            let v1 = _mm256_loadu_ps(x.as_ptr().add(i + 8));
            _mm256_storeu_ps(x.as_mut_ptr().add(i), _mm256_mul_ps(v0, vnorm));
            _mm256_storeu_ps(x.as_mut_ptr().add(i + 8), _mm256_mul_ps(v1, vnorm));
        }
        i += 16;
    }
    while i + 8 <= n {
        unsafe {
            let v = _mm256_loadu_ps(x.as_ptr().add(i));
            _mm256_storeu_ps(x.as_mut_ptr().add(i), _mm256_mul_ps(v, vnorm));
        }
        i += 8;
    }
    for v in &mut x[i..] {
        *v *= norm;
    }
}

/// PVQ fast-select setup: writes `abs_x[..n] = |x[..n]|`, `signs_pm1[..n] = ±1`, returns `sum(|x|)`.
#[inline]
pub fn pvq_fast_select_init(
    x: &[f32],
    n: usize,
    abs_x: &mut [f32],
    signs_pm1: &mut [i32],
) -> f32 {
    let x = &x[..n];
    let abs_x = &mut abs_x[..n];
    let signs_pm1 = &mut signs_pm1[..n];
    #[cfg(target_arch = "aarch64")]
    {
        return unsafe { pvq_fast_select_init_neon(x, abs_x, signs_pm1) };
    }
    #[cfg(target_arch = "x86_64")]
    {
        if n >= 8 && have_avx2() {
            return unsafe { pvq_fast_select_init_avx2(x, abs_x, signs_pm1) };
        }
    }
    #[cfg(not(target_arch = "aarch64"))]
    {
        let mut sum = 0.0f32;
        for i in 0..n {
            let a = x[i].abs();
            abs_x[i] = a;
            signs_pm1[i] = if x[i] < 0.0 { -1 } else { 1 };
            sum += a;
        }
        sum
    }
}

#[cfg(target_arch = "aarch64")]
#[target_feature(enable = "neon")]
#[inline]
unsafe fn pvq_fast_select_init_neon(
    x: &[f32],
    abs_x: &mut [f32],
    signs: &mut [i32],
) -> f32 {
    use std::arch::aarch64::*;
    let n = x.len();
    let mut sum_vec = vdupq_n_f32(0.0);
    let zero_f = vdupq_n_f32(0.0);
    let one_i = vdupq_n_s32(1);
    let mut i = 0usize;
    while i + 8 <= n {
        unsafe {
            let vx0 = vld1q_f32(x.as_ptr().add(i));
            let vx1 = vld1q_f32(x.as_ptr().add(i + 4));
            let vabs0 = vabsq_f32(vx0);
            let vabs1 = vabsq_f32(vx1);
            vst1q_f32(abs_x.as_mut_ptr().add(i), vabs0);
            vst1q_f32(abs_x.as_mut_ptr().add(i + 4), vabs1);
            sum_vec = vaddq_f32(sum_vec, vaddq_f32(vabs0, vabs1));

            // clt mask: 0xFFFF_FFFF (-1) where x < 0.0, 0 otherwise -> (mask | 1) == -1 or +1.
            let neg0 = vreinterpretq_s32_u32(vcltq_f32(vx0, zero_f));
            let neg1 = vreinterpretq_s32_u32(vcltq_f32(vx1, zero_f));
            vst1q_s32(signs.as_mut_ptr().add(i), vorrq_s32(neg0, one_i));
            vst1q_s32(signs.as_mut_ptr().add(i + 4), vorrq_s32(neg1, one_i));
        }
        i += 8;
    }
    if i + 4 <= n {
        unsafe {
            let vx = vld1q_f32(x.as_ptr().add(i));
            let vabs = vabsq_f32(vx);
            vst1q_f32(abs_x.as_mut_ptr().add(i), vabs);
            sum_vec = vaddq_f32(sum_vec, vabs);
            let neg = vreinterpretq_s32_u32(vcltq_f32(vx, zero_f));
            vst1q_s32(signs.as_mut_ptr().add(i), vorrq_s32(neg, one_i));
        }
        i += 4;
    }
    let mut sum = vaddvq_f32(sum_vec);
    for j in i..n {
        let a = x[j].abs();
        abs_x[j] = a;
        sum += a;
        signs[j] = if x[j] < 0.0 { -1 } else { 1 };
    }
    sum
}

#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx2")]
unsafe fn pvq_fast_select_init_avx2(
    x: &[f32],
    abs_x: &mut [f32],
    signs: &mut [i32],
) -> f32 {
    use std::arch::x86_64::*;
    let n = x.len();
    let sign_mask = _mm256_set1_ps(-0.0f32);
    let zero_ps = _mm256_setzero_ps();
    let one_i = _mm256_set1_epi32(1);
    let mut acc = _mm256_setzero_ps();
    let mut i = 0usize;
    while i + 8 <= n {
        unsafe {
            let v = _mm256_loadu_ps(x.as_ptr().add(i));
            let a = _mm256_andnot_ps(sign_mask, v);
            _mm256_storeu_ps(abs_x.as_mut_ptr().add(i), a);
            acc = _mm256_add_ps(acc, a);
            let neg = _mm256_castps_si256(_mm256_cmp_ps(v, zero_ps, _CMP_LT_OS));
            _mm256_storeu_si256(
                signs.as_mut_ptr().add(i) as *mut __m256i,
                _mm256_or_si256(neg, one_i),
            );
        }
        i += 8;
    }
    let s4 = _mm_add_ps(_mm256_castps256_ps128(acc), _mm256_extractf128_ps(acc, 1));
    let s2 = _mm_add_ps(s4, _mm_movehl_ps(s4, s4));
    let mut sum = _mm_cvtss_f32(_mm_add_ss(s2, _mm_shuffle_ps(s2, s2, 0x55)));
    for j in i..n {
        let a = x[j].abs();
        abs_x[j] = a;
        sum += a;
        signs[j] = if x[j] < 0.0 { -1 } else { 1 };
    }
    sum
}

/// PVQ scalar-search setup (`n <= 31`): writes `abs_x[..n] = |x[..n]|`, `sign_01[..n] = (x < 0.0) as i32`.
#[inline]
pub fn pvq_search_scalar_init(
    x: &[f32],
    n: usize,
    abs_x: &mut [f32; 32],
    sign_01: &mut [i32; 32],
) -> f32 {
    let x = &x[..n];
    let abs_x = &mut abs_x[..n];
    let sign_01 = &mut sign_01[..n];
    #[cfg(target_arch = "aarch64")]
    {
        return unsafe { pvq_search_scalar_init_neon(x, abs_x, sign_01) };
    }
    #[cfg(target_arch = "x86_64")]
    {
        if n >= 8 && have_avx2() {
            return unsafe { pvq_search_scalar_init_avx2(x, abs_x, sign_01) };
        }
    }
    #[cfg(not(target_arch = "aarch64"))]
    {
        let mut sum = 0.0f32;
        for i in 0..n {
            let a = x[i].abs();
            abs_x[i] = a;
            sign_01[i] = i32::from(x[i] < 0.0);
            sum += a;
        }
        sum
    }
}

#[cfg(target_arch = "aarch64")]
#[target_feature(enable = "neon")]
#[inline]
unsafe fn pvq_search_scalar_init_neon(
    x: &[f32],
    abs_x: &mut [f32],
    sign_01: &mut [i32],
) -> f32 {
    use std::arch::aarch64::*;
    let n = x.len();
    let mut sum_vec = vdupq_n_f32(0.0);
    let zero_f = vdupq_n_f32(0.0);
    let mut i = 0usize;
    while i + 4 <= n {
        unsafe {
            let vx = vld1q_f32(x.as_ptr().add(i));
            let vabs = vabsq_f32(vx);
            vst1q_f32(abs_x.as_mut_ptr().add(i), vabs);
            sum_vec = vaddq_f32(sum_vec, vabs);
            let neg01 = vreinterpretq_s32_u32(vshrq_n_u32::<31>(vcltq_f32(vx, zero_f)));
            vst1q_s32(sign_01.as_mut_ptr().add(i), neg01);
        }
        i += 4;
    }
    let mut sum = vaddvq_f32(sum_vec);
    for j in i..n {
        let a = x[j].abs();
        abs_x[j] = a;
        sum += a;
        sign_01[j] = i32::from(x[j] < 0.0);
    }
    sum
}

#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx2")]
unsafe fn pvq_search_scalar_init_avx2(
    x: &[f32],
    abs_x: &mut [f32],
    sign_01: &mut [i32],
) -> f32 {
    use std::arch::x86_64::*;
    let n = x.len();
    let sign_mask = _mm256_set1_ps(-0.0f32);
    let zero_ps = _mm256_setzero_ps();
    let one_i = _mm256_set1_epi32(1);
    let mut acc = _mm256_setzero_ps();
    let mut i = 0usize;
    while i + 8 <= n {
        unsafe {
            let v = _mm256_loadu_ps(x.as_ptr().add(i));
            let a = _mm256_andnot_ps(sign_mask, v);
            _mm256_storeu_ps(abs_x.as_mut_ptr().add(i), a);
            acc = _mm256_add_ps(acc, a);
            let neg = _mm256_castps_si256(_mm256_cmp_ps(v, zero_ps, _CMP_LT_OS));
            _mm256_storeu_si256(
                sign_01.as_mut_ptr().add(i) as *mut __m256i,
                _mm256_and_si256(neg, one_i),
            );
        }
        i += 8;
    }
    let s4 = _mm_add_ps(_mm256_castps256_ps128(acc), _mm256_extractf128_ps(acc, 1));
    let s2 = _mm_add_ps(s4, _mm_movehl_ps(s4, s4));
    let mut sum = _mm_cvtss_f32(_mm_add_ss(s2, _mm_shuffle_ps(s2, s2, 0x55)));
    for j in i..n {
        let a = x[j].abs();
        abs_x[j] = a;
        sum += a;
        sign_01[j] = i32::from(x[j] < 0.0);
    }
    sum
}

#[inline]
pub fn exp_rotation1(x: &mut [f32], len: usize, stride: usize, c: f32, s: f32) {
    assert!(stride <= len, "exp_rotation1: stride {stride} exceeds len {len}");
    let x = &mut x[..len];
    #[cfg(target_arch = "aarch64")]
    {
        if stride >= 4 {
            unsafe { exp_rotation1_neon(x, len, stride, c, s) };
            return;
        }
    }
    #[cfg(target_arch = "x86_64")]
    {
        if stride >= 8 && have_avx_fma() {
            unsafe { exp_rotation1_avx(x, len, stride, c, s) };
            return;
        }
    }
    exp_rotation1_scalar(x, len, stride, c, s);
}

pub fn exp_rotation1_scalar(x: &mut [f32], len: usize, stride: usize, c: f32, s: f32) {
    let x = &mut x[..len];
    let ms = -s;
    for i in 0..(len - stride) {
        let x1 = x[i];
        let x2 = x[i + stride];
        x[i + stride] = c * x2 + s * x1;
        x[i] = c * x1 + ms * x2;
    }
    if len >= 2 * stride {
        for i in (0..(len - 2 * stride)).rev() {
            let x1 = x[i];
            let x2 = x[i + stride];
            x[i + stride] = c * x2 + s * x1;
            x[i] = c * x1 + ms * x2;
        }
    }
}

#[cfg(target_arch = "aarch64")]
#[target_feature(enable = "neon")]
#[inline]
unsafe fn exp_rotation1_neon(x: &mut [f32], len: usize, stride: usize, c: f32, s: f32) {
    use std::arch::aarch64::*;
    let vc = vdupq_n_f32(c);
    let vs = vdupq_n_f32(s);
    let mut i = 0usize;
    while i + 4 <= len - stride {
        unsafe {
            let vx1 = vld1q_f32(x.as_ptr().add(i));
            let vx2 = vld1q_f32(x.as_ptr().add(i + stride));
            let vy1 = vfmsq_f32(vmulq_f32(vx1, vc), vs, vx2);
            let vy2 = vfmaq_f32(vmulq_f32(vx2, vc), vs, vx1);
            vst1q_f32(x.as_mut_ptr().add(i), vy1);
            vst1q_f32(x.as_mut_ptr().add(i + stride), vy2);
        }
        i += 4;
    }
    for j in i..(len - stride) {
        let x1 = x[j];
        let x2 = x[j + stride];
        x[j + stride] = c * x2 + s * x1;
        x[j] = c * x1 - s * x2;
    }
    if len >= 2 * stride {
        let mut rem = len - 2 * stride;
        while rem >= 4 {
            let j = rem - 4;
            unsafe {
                let vx1 = vld1q_f32(x.as_ptr().add(j));
                let vx2 = vld1q_f32(x.as_ptr().add(j + stride));
                let vy1 = vfmsq_f32(vmulq_f32(vx1, vc), vs, vx2);
                let vy2 = vfmaq_f32(vmulq_f32(vx2, vc), vs, vx1);
                vst1q_f32(x.as_mut_ptr().add(j), vy1);
                vst1q_f32(x.as_mut_ptr().add(j + stride), vy2);
            }
            rem -= 4;
        }
        for j in (0..rem).rev() {
            let x1 = x[j];
            let x2 = x[j + stride];
            x[j + stride] = c * x2 + s * x1;
            x[j] = c * x1 - s * x2;
        }
    }
}

#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx,fma")]
unsafe fn exp_rotation1_avx(x: &mut [f32], len: usize, stride: usize, c: f32, s: f32) {
    use std::arch::x86_64::*;
    let vc = _mm256_set1_ps(c);
    let vs = _mm256_set1_ps(s);
    let mut i = 0usize;
    while i + 8 <= len - stride {
        unsafe {
            let vx1 = _mm256_loadu_ps(x.as_ptr().add(i));
            let vx2 = _mm256_loadu_ps(x.as_ptr().add(i + stride));
            let vy1 = _mm256_fnmadd_ps(vs, vx2, _mm256_mul_ps(vx1, vc));
            let vy2 = _mm256_fmadd_ps(vs, vx1, _mm256_mul_ps(vx2, vc));
            _mm256_storeu_ps(x.as_mut_ptr().add(i), vy1);
            _mm256_storeu_ps(x.as_mut_ptr().add(i + stride), vy2);
        }
        i += 8;
    }
    for j in i..(len - stride) {
        let x1 = x[j];
        let x2 = x[j + stride];
        x[j + stride] = c * x2 + s * x1;
        x[j] = c * x1 - s * x2;
    }
    if len >= 2 * stride {
        let mut rem = len - 2 * stride;
        while rem >= 8 {
            let j = rem - 8;
            unsafe {
                let vx1 = _mm256_loadu_ps(x.as_ptr().add(j));
                let vx2 = _mm256_loadu_ps(x.as_ptr().add(j + stride));
                let vy1 = _mm256_fnmadd_ps(vs, vx2, _mm256_mul_ps(vx1, vc));
                let vy2 = _mm256_fmadd_ps(vs, vx1, _mm256_mul_ps(vx2, vc));
                _mm256_storeu_ps(x.as_mut_ptr().add(j), vy1);
                _mm256_storeu_ps(x.as_mut_ptr().add(j + stride), vy2);
            }
            rem -= 8;
        }
        for j in (0..rem).rev() {
            let x1 = x[j];
            let x2 = x[j + stride];
            x[j + stride] = c * x2 + s * x1;
            x[j] = c * x1 - s * x2;
        }
    }
}

#[inline]
pub fn extract_collapse_mask(iy: &[i32], n: usize, b: usize) -> u32 {
    if b <= 1 {
        return 1;
    }
    let n0 = n / b;
    let iy = &iy[..b * n0];
    #[cfg(target_arch = "aarch64")]
    {
        return unsafe { extract_collapse_mask_neon(iy, n0, b) };
    }
    #[cfg(target_arch = "x86_64")]
    {
        if n0 >= 8 && have_avx2() {
            return unsafe { extract_collapse_mask_avx2(iy, n0, b) };
        }
    }
    #[cfg(not(target_arch = "aarch64"))]
    {
        let mut collapse_mask = 0u32;
        for i in 0..b {
            let mut tmp = 0i32;
            let base = i * n0;
            for j in 0..n0 {
                tmp |= iy[base + j];
            }
            if tmp != 0 {
                collapse_mask |= 1 << i;
            }
        }
        collapse_mask
    }
}

#[cfg(target_arch = "aarch64")]
#[target_feature(enable = "neon")]
#[inline]
unsafe fn extract_collapse_mask_neon(iy: &[i32], n0: usize, b: usize) -> u32 {
    use std::arch::aarch64::*;
    let mut collapse_mask = 0u32;
    let n4 = n0 & !3;
    for i in 0..b {
        let slice = &iy[i * n0..(i + 1) * n0];
        let mut any_nonzero = false;
        let mut j = 0usize;
        while j < n4 {
            let lane_nz = unsafe {
                let v = vld1q_s32(slice.as_ptr().add(j));
                vmaxvq_u32(vreinterpretq_u32_s32(v)) != 0
            };
            if lane_nz {
                any_nonzero = true;
                break;
            }
            j += 4;
        }
        if !any_nonzero {
            for &v in &slice[j..] {
                if v != 0 {
                    any_nonzero = true;
                    break;
                }
            }
        }
        if any_nonzero {
            collapse_mask |= 1 << i;
        }
    }
    collapse_mask
}

#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx2")]
unsafe fn extract_collapse_mask_avx2(iy: &[i32], n0: usize, b: usize) -> u32 {
    use std::arch::x86_64::*;
    let mut collapse_mask = 0u32;
    let n8 = n0 & !7;
    for i in 0..b {
        let slice = &iy[i * n0..(i + 1) * n0];
        let mut any_nonzero = false;
        let mut j = 0usize;
        while j < n8 {
            let nz = unsafe {
                let v = _mm256_loadu_si256(slice.as_ptr().add(j) as *const __m256i);
                _mm256_testz_si256(v, v) == 0
            };
            if nz {
                any_nonzero = true;
                break;
            }
            j += 8;
        }
        if !any_nonzero {
            for &v in &slice[j..] {
                if v != 0 {
                    any_nonzero = true;
                    break;
                }
            }
        }
        if any_nonzero {
            collapse_mask |= 1 << i;
        }
    }
    collapse_mask
}

#[inline]
pub fn alg_quant_resynth(y: &[i32], x: &mut [f32], n: usize, gain: f32) {
    let y = &y[..n];
    let x = &mut x[..n];
    #[cfg(target_arch = "aarch64")]
    {
        unsafe { alg_quant_resynth_neon(y, x, gain) };
        return;
    }
    #[cfg(target_arch = "x86_64")]
    {
        if n >= 8 && have_avx2_fma() {
            unsafe { alg_quant_resynth_avx2(y, x, gain) };
            return;
        }
    }
    #[cfg(not(target_arch = "aarch64"))]
    {
        let mut ryy = 0.0f32;
        for i in 0..n {
            let v = y[i] as f32;
            x[i] = v;
            ryy += v * v;
        }
        let g = gain / (1e-15 + ryy).sqrt();
        for v in x.iter_mut() {
            *v *= g;
        }
    }
}

#[cfg(target_arch = "aarch64")]
#[target_feature(enable = "neon")]
#[inline]
unsafe fn alg_quant_resynth_neon(y: &[i32], x: &mut [f32], gain: f32) {
    use std::arch::aarch64::*;
    let n = y.len();
    let n8 = n & !7;
    let mut sum_vec = vdupq_n_f32(0.0);
    let mut i = 0usize;
    while i < n8 {
        unsafe {
            let yf0 = vcvtq_f32_s32(vld1q_s32(y.as_ptr().add(i)));
            let yf1 = vcvtq_f32_s32(vld1q_s32(y.as_ptr().add(i + 4)));
            vst1q_f32(x.as_mut_ptr().add(i), yf0);
            vst1q_f32(x.as_mut_ptr().add(i + 4), yf1);
            sum_vec = vfmaq_f32(sum_vec, yf0, yf0);
            sum_vec = vfmaq_f32(sum_vec, yf1, yf1);
        }
        i += 8;
    }
    let mut ryy = vaddvq_f32(sum_vec);
    for j in i..n {
        let v = y[j] as f32;
        x[j] = v;
        ryy += v * v;
    }
    let g = gain / (1e-15 + ryy).sqrt();
    let vg = vdupq_n_f32(g);
    i = 0;
    while i < n8 {
        unsafe {
            let vx0 = vld1q_f32(x.as_ptr().add(i));
            let vx1 = vld1q_f32(x.as_ptr().add(i + 4));
            vst1q_f32(x.as_mut_ptr().add(i), vmulq_f32(vx0, vg));
            vst1q_f32(x.as_mut_ptr().add(i + 4), vmulq_f32(vx1, vg));
        }
        i += 8;
    }
    for v in &mut x[i..] {
        *v *= g;
    }
}

#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx2,fma")]
unsafe fn alg_quant_resynth_avx2(y: &[i32], x: &mut [f32], gain: f32) {
    use std::arch::x86_64::*;
    let n = y.len();
    let mut acc0 = _mm256_setzero_ps();
    let mut i = 0usize;
    while i + 8 <= n {
        unsafe {
            let yi = _mm256_loadu_si256(y.as_ptr().add(i) as *const __m256i);
            let yf = _mm256_cvtepi32_ps(yi);
            _mm256_storeu_ps(x.as_mut_ptr().add(i), yf);
            acc0 = _mm256_fmadd_ps(yf, yf, acc0);
        }
        i += 8;
    }
    let s4 = _mm_add_ps(_mm256_castps256_ps128(acc0), _mm256_extractf128_ps(acc0, 1));
    let s2 = _mm_add_ps(s4, _mm_movehl_ps(s4, s4));
    let mut ryy = _mm_cvtss_f32(_mm_add_ss(s2, _mm_shuffle_ps(s2, s2, 0x55)));
    for j in i..n {
        let v = y[j] as f32;
        x[j] = v;
        ryy += v * v;
    }
    let g = gain / (1e-15f32 + ryy).sqrt();
    let vg = _mm256_set1_ps(g);
    i = 0;
    while i + 8 <= n {
        unsafe {
            let v = _mm256_loadu_ps(x.as_ptr().add(i));
            _mm256_storeu_ps(x.as_mut_ptr().add(i), _mm256_mul_ps(v, vg));
        }
        i += 8;
    }
    for v in &mut x[i..] {
        *v *= g;
    }
}

// ---------------------------------------------------------------------------
// SILK Fixed-Point SIMD Kernels (Bit-Exact vs Scalar Oracles)
// ---------------------------------------------------------------------------

#[inline(always)]
fn silk_smulwb(a: i32, b: i32) -> i32 {
    (((a as i64) * (b as i16 as i64)) >> 16) as i32
}

#[inline(always)]
fn silk_smlawb(a: i32, b: i32, c: i32) -> i32 {
    a.wrapping_add(silk_smulwb(b, c))
}

#[inline(always)]
fn silk_rshift_round12(a: i32) -> i32 {
    (a >> 12) + ((a >> 11) & 1)
}

#[inline(always)]
fn silk_sat16(a: i32) -> i16 {
    a.clamp(i16::MIN as i32, i16::MAX as i32) as i16
}

#[inline]
pub fn silk_xcorr_kernel(x: &[i16], y: &[i16], sum: &mut [i32; 4], len: usize) {
    if len == 0 {
        return;
    }
    let x = &x[..len];
    assert!(
        y.len() >= len + 3,
        "silk_xcorr_kernel: y holds {} samples, needs len + 3 = {}",
        y.len(),
        len + 3
    );
    #[cfg(target_arch = "aarch64")]
    {
        unsafe { silk_xcorr_kernel_neon(x, y, sum, len) };
        return;
    }
    #[cfg(target_arch = "x86_64")]
    {
        if have_avx2() {
            unsafe { silk_xcorr_kernel_avx2(x, y, sum, len) };
            return;
        }
    }
    #[cfg(not(target_arch = "aarch64"))]
    silk_xcorr_kernel_scalar(x, y, sum, len);
}

#[cfg_attr(target_arch = "aarch64", allow(dead_code))]
fn silk_xcorr_kernel_scalar(x: &[i16], y: &[i16], sum: &mut [i32; 4], len: usize) {
    let mut y_0 = y[0];
    let mut y_1 = y[1];
    let mut y_2 = y[2];
    for j in 0..len {
        let y_3 = y[j + 3];
        let xv = x[j] as i32;
        sum[0] = sum[0].wrapping_add(xv.wrapping_mul(y_0 as i32));
        sum[1] = sum[1].wrapping_add(xv.wrapping_mul(y_1 as i32));
        sum[2] = sum[2].wrapping_add(xv.wrapping_mul(y_2 as i32));
        sum[3] = sum[3].wrapping_add(xv.wrapping_mul(y_3 as i32));
        y_0 = y_1;
        y_1 = y_2;
        y_2 = y_3;
    }
}

#[cfg(target_arch = "aarch64")]
#[target_feature(enable = "neon")]
#[inline]
unsafe fn silk_xcorr_kernel_neon(x: &[i16], y: &[i16], sum: &mut [i32; 4], mut len: usize) {
    use std::arch::aarch64::*;
    let mut acc = unsafe { vld1q_s32(sum.as_ptr()) };
    let mut xi = x.as_ptr();
    let mut yi = y.as_ptr();
    let mut off = 0usize;
    if y.len() >= 12 {
        let mut yy = unsafe { vld1q_s16(yi) };
        while len > 4 && off + 12 <= y.len() {
            unsafe {
                yi = yi.add(4);
                let yy1 = vld1q_s16(yi);
                acc = vmlal_s16(acc, vget_low_s16(yy), vld1_dup_s16(xi));
                acc = vmlal_s16(acc, vget_low_s16(vextq_s16(yy, yy1, 1)), vld1_dup_s16(xi.add(1)));
                acc = vmlal_s16(acc, vget_low_s16(vextq_s16(yy, yy1, 2)), vld1_dup_s16(xi.add(2)));
                acc = vmlal_s16(acc, vget_low_s16(vextq_s16(yy, yy1, 3)), vld1_dup_s16(xi.add(3)));
                xi = xi.add(4);
                yy = yy1;
            }
            off += 4;
            len -= 4;
        }
    }
    unsafe { vst1q_s32(sum.as_mut_ptr(), acc) };
    for k in 0..len {
        let xv = unsafe { *xi.add(k) as i32 };
        unsafe {
            sum[0] = sum[0].wrapping_add(xv * (*yi.add(k) as i32));
            sum[1] = sum[1].wrapping_add(xv * (*yi.add(k + 1) as i32));
            sum[2] = sum[2].wrapping_add(xv * (*yi.add(k + 2) as i32));
            sum[3] = sum[3].wrapping_add(xv * (*yi.add(k + 3) as i32));
        }
    }
}

#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx2")]
unsafe fn silk_xcorr_kernel_avx2(x: &[i16], y: &[i16], sum: &mut [i32; 4], len: usize) {
    use std::arch::x86_64::*;
    let mut acc = _mm_setzero_si128();
    let mut i = 0usize;
    while i + 4 <= len && i + 8 <= y.len() {
        unsafe {
            let x4 = _mm_loadl_epi64(x.as_ptr().add(i) as *const __m128i);
            let xdup = _mm_unpacklo_epi64(x4, x4);
            let y8 = _mm_loadu_si128(y.as_ptr().add(i) as *const __m128i);
            let y1 = _mm_bsrli_si128(y8, 2);
            let y2 = _mm_bsrli_si128(y8, 4);
            let y3 = _mm_bsrli_si128(y8, 6);
            let p01 = _mm_madd_epi16(xdup, _mm_unpacklo_epi64(y8, y1));
            let p23 = _mm_madd_epi16(xdup, _mm_unpacklo_epi64(y2, y3));
            acc = _mm_add_epi32(acc, _mm_hadd_epi32(p01, p23));
        }
        i += 4;
    }
    let mut tmp = [0i32; 4];
    unsafe { _mm_storeu_si128(tmp.as_mut_ptr() as *mut __m128i, acc) };
    sum[0] = sum[0].wrapping_add(tmp[0]);
    sum[1] = sum[1].wrapping_add(tmp[1]);
    sum[2] = sum[2].wrapping_add(tmp[2]);
    sum[3] = sum[3].wrapping_add(tmp[3]);
    silk_xcorr_kernel_scalar(&x[i..], &y[i..], sum, len - i);
}

#[inline]
pub fn silk_sum_sqr_shift_loop(x: &[i16], len: usize, shft: i32) -> i32 {
    let x = &x[..len];
    #[cfg(target_arch = "aarch64")]
    {
        return unsafe { silk_sum_sqr_shift_neon(x, shft) };
    }
    #[cfg(target_arch = "x86_64")]
    {
        if have_avx2() {
            return unsafe { silk_sum_sqr_shift_avx2(x, shft) };
        }
    }
    #[cfg(not(target_arch = "aarch64"))]
    silk_sum_sqr_shift_scalar(x, shft)
}

fn silk_sum_sqr_shift_scalar(x: &[i16], shft: i32) -> i32 {
    let len = x.len();
    let mut nrg = 0i32;
    let mut i = 0usize;
    while i + 1 < len {
        let a = x[i] as i32;
        let b = x[i + 1] as i32;
        let sq = ((a * a) as u32).wrapping_add((b * b) as u32);
        nrg = nrg.wrapping_add((sq >> shft) as i32);
        i += 2;
    }
    if i < len {
        let v = x[i] as i32;
        nrg = nrg.wrapping_add((((v * v) as u32) >> shft) as i32);
    }
    nrg
}

#[cfg(target_arch = "aarch64")]
#[target_feature(enable = "neon")]
#[inline]
unsafe fn silk_sum_sqr_shift_neon(x: &[i16], shft: i32) -> i32 {
    use std::arch::aarch64::*;
    let len = x.len();
    let mut acc = vdupq_n_s64(0i64);
    let shift_vec = vdupq_n_s64(-shft as i64);
    let mut i = 0usize;
    while i + 8 <= len {
        unsafe {
            let v = vld1q_s16(x.as_ptr().add(i));
            let lo = vget_low_s16(v);
            let hi = vget_high_s16(v);
            let sq_lo = vmull_s16(lo, lo);
            let sq_hi = vmull_s16(hi, hi);
            acc = vaddq_s64(acc, vshlq_s64(vpaddlq_s32(sq_lo), shift_vec));
            acc = vaddq_s64(acc, vshlq_s64(vpaddlq_s32(sq_hi), shift_vec));
        }
        i += 8;
    }
    let mut nrg = vaddvq_s64(acc) as i32;
    nrg = nrg.wrapping_add(silk_sum_sqr_shift_scalar(&x[i..], shft));
    nrg
}

#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx2")]
unsafe fn silk_sum_sqr_shift_avx2(x: &[i16], shft: i32) -> i32 {
    use std::arch::x86_64::*;
    let len = x.len();
    let mut acc = _mm256_setzero_si256();
    let vshift = _mm256_set1_epi32(shft);
    let mut i = 0usize;
    while i + 16 <= len {
        unsafe {
            let v = _mm256_loadu_si256(x.as_ptr().add(i) as *const __m256i);
            let sq = _mm256_madd_epi16(v, v);
            let shifted = if shft > 0 {
                _mm256_srlv_epi32(sq, vshift)
            } else {
                sq
            };
            acc = _mm256_add_epi32(acc, shifted);
        }
        i += 16;
    }
    let sum4 = _mm_add_epi32(_mm256_castsi256_si128(acc), _mm256_extracti128_si256(acc, 1));
    let sum2 = _mm_add_epi32(sum4, _mm_srli_si128(sum4, 8));
    let sum1 = _mm_add_epi32(sum2, _mm_srli_si128(sum2, 4));
    let nrg = _mm_cvtsi128_si32(sum1);
    nrg.wrapping_add(silk_sum_sqr_shift_scalar(&x[i..], shft))
}

#[inline]
pub fn silk_inner_prod_aligned(ptr1: &[i16], ptr2: &[i16], len: usize) -> i32 {
    let ptr1 = &ptr1[..len];
    let ptr2 = &ptr2[..len];
    #[cfg(target_arch = "aarch64")]
    {
        return unsafe { silk_inner_prod_aligned_neon(ptr1, ptr2) };
    }
    #[cfg(target_arch = "x86_64")]
    {
        if have_avx2() {
            return unsafe { silk_inner_prod_aligned_avx2(ptr1, ptr2) };
        }
    }
    #[cfg(not(target_arch = "aarch64"))]
    silk_inner_prod_aligned_scalar(ptr1, ptr2)
}

#[cfg_attr(target_arch = "aarch64", allow(dead_code))]
fn silk_inner_prod_aligned_scalar(ptr1: &[i16], ptr2: &[i16]) -> i32 {
    let len = ptr1.len();
    let len4 = len & !3;
    let mut sum0 = 0i32;
    let mut sum1 = 0i32;
    let mut sum2 = 0i32;
    let mut sum3 = 0i32;
    let mut i = 0usize;
    while i < len4 {
        sum0 = sum0.wrapping_add((ptr1[i] as i32).wrapping_mul(ptr2[i] as i32));
        sum1 = sum1.wrapping_add((ptr1[i + 1] as i32).wrapping_mul(ptr2[i + 1] as i32));
        sum2 = sum2.wrapping_add((ptr1[i + 2] as i32).wrapping_mul(ptr2[i + 2] as i32));
        sum3 = sum3.wrapping_add((ptr1[i + 3] as i32).wrapping_mul(ptr2[i + 3] as i32));
        i += 4;
    }
    while i < len {
        sum0 = sum0.wrapping_add((ptr1[i] as i32).wrapping_mul(ptr2[i] as i32));
        i += 1;
    }
    sum0.wrapping_add(sum1).wrapping_add(sum2).wrapping_add(sum3)
}

#[cfg(target_arch = "aarch64")]
#[target_feature(enable = "neon")]
#[inline]
unsafe fn silk_inner_prod_aligned_neon(ptr1: &[i16], ptr2: &[i16]) -> i32 {
    use std::arch::aarch64::*;
    let len = ptr1.len();
    let mut acc0 = vdupq_n_s32(0i32);
    let mut acc1 = vdupq_n_s32(0i32);
    let mut acc2 = vdupq_n_s32(0i32);
    let mut acc3 = vdupq_n_s32(0i32);
    let mut i = 0usize;
    while i + 32 <= len {
        unsafe {
            let a0 = vld1q_s16(ptr1.as_ptr().add(i));
            let b0 = vld1q_s16(ptr2.as_ptr().add(i));
            let a1 = vld1q_s16(ptr1.as_ptr().add(i + 8));
            let b1 = vld1q_s16(ptr2.as_ptr().add(i + 8));
            let a2 = vld1q_s16(ptr1.as_ptr().add(i + 16));
            let b2 = vld1q_s16(ptr2.as_ptr().add(i + 16));
            let a3 = vld1q_s16(ptr1.as_ptr().add(i + 24));
            let b3 = vld1q_s16(ptr2.as_ptr().add(i + 24));

            acc0 = vmlal_s16(acc0, vget_low_s16(a0), vget_low_s16(b0));
            acc0 = vmlal_high_s16(acc0, a0, b0);
            acc1 = vmlal_s16(acc1, vget_low_s16(a1), vget_low_s16(b1));
            acc1 = vmlal_high_s16(acc1, a1, b1);
            acc2 = vmlal_s16(acc2, vget_low_s16(a2), vget_low_s16(b2));
            acc2 = vmlal_high_s16(acc2, a2, b2);
            acc3 = vmlal_s16(acc3, vget_low_s16(a3), vget_low_s16(b3));
            acc3 = vmlal_high_s16(acc3, a3, b3);
        }
        i += 32;
    }
    while i + 8 <= len {
        unsafe {
            let a0 = vld1q_s16(ptr1.as_ptr().add(i));
            let b0 = vld1q_s16(ptr2.as_ptr().add(i));
            acc0 = vmlal_s16(acc0, vget_low_s16(a0), vget_low_s16(b0));
            acc0 = vmlal_high_s16(acc0, a0, b0);
        }
        i += 8;
    }
    let sum01 = vpaddq_s32(acc0, acc1);
    let sum23 = vpaddq_s32(acc2, acc3);
    let mut result = vaddvq_s64(vpaddlq_s32(vpaddq_s32(sum01, sum23)));
    while i < len {
        result += (ptr1[i] as i64) * (ptr2[i] as i64);
        i += 1;
    }
    result as i32
}

#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx2")]
unsafe fn silk_inner_prod_aligned_avx2(ptr1: &[i16], ptr2: &[i16]) -> i32 {
    use std::arch::x86_64::*;
    let len = ptr1.len();
    let mut acc0 = _mm256_setzero_si256();
    let mut acc1 = _mm256_setzero_si256();
    let mut i = 0usize;
    while i + 32 <= len {
        unsafe {
            let a0 = _mm256_loadu_si256(ptr1.as_ptr().add(i) as *const __m256i);
            let b0 = _mm256_loadu_si256(ptr2.as_ptr().add(i) as *const __m256i);
            let a1 = _mm256_loadu_si256(ptr1.as_ptr().add(i + 16) as *const __m256i);
            let b1 = _mm256_loadu_si256(ptr2.as_ptr().add(i + 16) as *const __m256i);
            acc0 = _mm256_add_epi32(acc0, _mm256_madd_epi16(a0, b0));
            acc1 = _mm256_add_epi32(acc1, _mm256_madd_epi16(a1, b1));
        }
        i += 32;
    }
    while i + 16 <= len {
        unsafe {
            let a = _mm256_loadu_si256(ptr1.as_ptr().add(i) as *const __m256i);
            let b = _mm256_loadu_si256(ptr2.as_ptr().add(i) as *const __m256i);
            acc0 = _mm256_add_epi32(acc0, _mm256_madd_epi16(a, b));
        }
        i += 16;
    }
    let acc = _mm256_add_epi32(acc0, acc1);
    let sum4 = _mm_add_epi32(_mm256_castsi256_si128(acc), _mm256_extracti128_si256(acc, 1));
    let sum2 = _mm_add_epi32(sum4, _mm_srli_si128(sum4, 8));
    let sum1 = _mm_add_epi32(sum2, _mm_srli_si128(sum2, 4));
    let mut result = _mm_cvtsi128_si32(sum1);
    if i + 8 <= len {
        unsafe {
            let a = _mm_loadu_si128(ptr1.as_ptr().add(i) as *const __m128i);
            let b = _mm_loadu_si128(ptr2.as_ptr().add(i) as *const __m128i);
            let p = _mm_madd_epi16(a, b);
            let p2 = _mm_add_epi32(p, _mm_srli_si128(p, 8));
            let p1 = _mm_add_epi32(p2, _mm_srli_si128(p2, 4));
            result = result.wrapping_add(_mm_cvtsi128_si32(p1));
        }
        i += 8;
    }
    while i < len {
        result = result.wrapping_add((ptr1[i] as i32).wrapping_mul(ptr2[i] as i32));
        i += 1;
    }
    result
}

#[inline]
pub fn silk_lpc_analysis_filter(
    out: &mut [i16],
    input: &[i16],
    b: &[i16],
    len: usize,
    d: usize,
) {
    assert!(d >= 6 && (d & 1) == 0 && d <= len);
    #[cfg(target_arch = "x86_64")]
    {
        if d <= 16 && have_avx2() {
            unsafe { silk_lpc_analysis_filter_avx2(out, input, b, len, d) };
            return;
        }
    }
    #[cfg(target_arch = "aarch64")]
    {
        if d <= 16 {
            unsafe { silk_lpc_analysis_filter_neon(out, input, b, len, d) };
            return;
        }
    }
    silk_lpc_analysis_filter_scalar(out, input, b, len, d);
}

fn silk_lpc_analysis_filter_scalar(
    out: &mut [i16],
    input: &[i16],
    b: &[i16],
    len: usize,
    d: usize,
) {
    let input = &input[..len];
    let out = &mut out[..len];
    let b = &b[..d];
    out[..d].fill(0);
    for ix in d..len {
        let hist = &input[ix - d..ix];
        let mut out32_q12 = 0i32;
        for (&bj, &xj) in b.iter().zip(hist.iter().rev()) {
            out32_q12 = out32_q12.wrapping_add((xj as i32) * (bj as i32));
        }
        out32_q12 = ((input[ix] as i32) << 12).wrapping_sub(out32_q12);
        out[ix] = silk_sat16(silk_rshift_round12(out32_q12));
    }
}

#[cfg(target_arch = "aarch64")]
#[target_feature(enable = "neon")]
#[inline]
unsafe fn silk_lpc_analysis_filter_neon(
    out: &mut [i16],
    input: &[i16],
    b: &[i16],
    len: usize,
    d: usize,
) {
    use std::arch::aarch64::*;
    let input = &input[..len];
    let out = &mut out[..len];
    let b = &b[..d];
    out[..d].fill(0);

    let mut b_rev_pad = [0i16; 16];
    for k in 0..d {
        b_rev_pad[15 - k] = b[k];
    }
    let simd_start = 16usize.max(d);
    for ix in d..simd_start.min(len) {
        let mut s = 0i32;
        for k in 0..d {
            s = s.wrapping_add((input[ix - 1 - k] as i32) * (b[k] as i32));
        }
        let out32_q12 = ((input[ix] as i32) << 12).wrapping_sub(s);
        out[ix] = silk_sat16(silk_rshift_round12(out32_q12));
    }
    if simd_start >= len {
        return;
    }
    let b_lo = unsafe { vld1q_s16(b_rev_pad.as_ptr()) };
    let b_hi = unsafe { vld1q_s16(b_rev_pad.as_ptr().add(8)) };
    for ix in simd_start..len {
        let s = unsafe {
            let base = input.as_ptr().add(ix - 16);
            let inp_lo = vld1q_s16(base);
            let inp_hi = vld1q_s16(base.add(8));
            let mut acc = vmull_s16(vget_low_s16(inp_lo), vget_low_s16(b_lo));
            acc = vmlal_high_s16(acc, inp_lo, b_lo);
            acc = vmlal_s16(acc, vget_low_s16(inp_hi), vget_low_s16(b_hi));
            acc = vmlal_high_s16(acc, inp_hi, b_hi);
            vaddvq_s64(vpaddlq_s32(acc)) as i32
        };
        let out32_q12 = ((input[ix] as i32) << 12).wrapping_sub(s);
        out[ix] = silk_sat16(silk_rshift_round12(out32_q12));
    }
}

#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx2")]
unsafe fn silk_lpc_analysis_filter_avx2(
    out: &mut [i16],
    input: &[i16],
    b: &[i16],
    len: usize,
    d: usize,
) {
    use std::arch::x86_64::*;
    let input = &input[..len];
    let out = &mut out[..len];
    let b = &b[..d];
    out[..d].fill(0);

    let mut b_rev_pad = [0i16; 16];
    for k in 0..d {
        b_rev_pad[15 - k] = b[k];
    }
    let avx_start = 16usize.max(d);
    for ix in d..avx_start.min(len) {
        let mut s = 0i32;
        for k in 0..d {
            s = s.wrapping_add((input[ix - 1 - k] as i32) * (b[k] as i32));
        }
        let out32_q12 = ((input[ix] as i32) << 12).wrapping_sub(s);
        out[ix] = silk_sat16(silk_rshift_round12(out32_q12));
    }
    if avx_start >= len {
        return;
    }
    let bv = unsafe { _mm256_loadu_si256(b_rev_pad.as_ptr() as *const __m256i) };
    for ix in avx_start..len {
        let s = unsafe {
            let iv = _mm256_loadu_si256(input.as_ptr().add(ix - 16) as *const __m256i);
            let prod = _mm256_madd_epi16(iv, bv);
            let sum128 = _mm_add_epi32(
                _mm256_castsi256_si128(prod),
                _mm256_extracti128_si256(prod, 1),
            );
            let sum64 = _mm_add_epi32(sum128, _mm_srli_si128(sum128, 8));
            let sum32 = _mm_add_epi32(sum64, _mm_srli_si128(sum64, 4));
            _mm_cvtsi128_si32(sum32)
        };
        let out32_q12 = ((input[ix] as i32) << 12).wrapping_sub(s);
        out[ix] = silk_sat16(silk_rshift_round12(out32_q12));
    }
}

/// Bit-exact warped correlation MAC: `corr_qc[i] += ((state_qs[i] as i64 * state_qs[0] as i64) >> 16)`.
#[inline]
pub fn warped_corr_update(corr_qc: &mut [i64], state_qs: &[i32], order: usize) {
    let corr_qc = &mut corr_qc[..order + 1];
    let state_qs = &state_qs[..order + 1];
    #[cfg(target_arch = "aarch64")]
    {
        unsafe { warped_corr_update_neon(corr_qc, state_qs) };
        return;
    }
    #[cfg(target_arch = "x86_64")]
    {
        if have_avx2() {
            unsafe { warped_corr_update_avx2(corr_qc, state_qs) };
            return;
        }
    }
    #[cfg(not(target_arch = "aarch64"))]
    warped_corr_update_scalar(corr_qc, state_qs);
}

#[cfg_attr(target_arch = "aarch64", allow(dead_code))]
fn warped_corr_update_scalar(corr_qc: &mut [i64], state_qs: &[i32]) {
    let state0 = state_qs[0] as i64;
    for (c, &s) in corr_qc.iter_mut().zip(state_qs) {
        *c += ((s as i64) * state0) >> 16;
    }
}

#[cfg(target_arch = "aarch64")]
#[target_feature(enable = "neon")]
#[inline]
unsafe fn warped_corr_update_neon(corr_qc: &mut [i64], state_qs: &[i32]) {
    use std::arch::aarch64::*;
    let n = corr_qc.len();
    let s0 = vdup_n_s32(state_qs[0]);
    let mut i = 0usize;
    while i + 4 <= n {
        unsafe {
            let st = vld1q_s32(state_qs.as_ptr().add(i));
            let p_lo = vshrq_n_s64::<16>(vmull_s32(vget_low_s32(st), s0));
            let p_hi = vshrq_n_s64::<16>(vmull_s32(vget_high_s32(st), s0));
            let c_lo = vld1q_s64(corr_qc.as_ptr().add(i));
            let c_hi = vld1q_s64(corr_qc.as_ptr().add(i + 2));
            vst1q_s64(corr_qc.as_mut_ptr().add(i), vaddq_s64(c_lo, p_lo));
            vst1q_s64(corr_qc.as_mut_ptr().add(i + 2), vaddq_s64(c_hi, p_hi));
        }
        i += 4;
    }
    let state0 = state_qs[0] as i64;
    while i < n {
        corr_qc[i] += ((state_qs[i] as i64) * state0) >> 16;
        i += 1;
    }
}

#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx2")]
unsafe fn warped_corr_update_avx2(corr_qc: &mut [i64], state_qs: &[i32]) {
    use std::arch::x86_64::*;
    let n = corr_qc.len();
    let s0v = _mm256_set1_epi64x(state_qs[0] as i64);
    let mut i = 0usize;
    while i + 4 <= n {
        unsafe {
            let s4 = _mm256_cvtepi32_epi64(_mm_loadu_si128(
                state_qs.as_ptr().add(i) as *const __m128i,
            ));
            let prod = _mm256_mul_epi32(s4, s0v);
            let sign = _mm256_cmpgt_epi64(_mm256_setzero_si256(), prod);
            let shifted =
                _mm256_or_si256(_mm256_srli_epi64(prod, 16), _mm256_slli_epi64(sign, 48));
            let cur = _mm256_loadu_si256(corr_qc.as_ptr().add(i) as *const __m256i);
            _mm256_storeu_si256(
                corr_qc.as_mut_ptr().add(i) as *mut __m256i,
                _mm256_add_epi64(cur, shifted),
            );
        }
        i += 4;
    }
    let state0 = state_qs[0] as i64;
    while i < n {
        corr_qc[i] += ((state_qs[i] as i64) * state0) >> 16;
        i += 1;
    }
}

/// Bit-exact SILK NSQ short LPC prediction dot product.
#[inline]
pub fn silk_lpc_prediction(
    ps_lpc_q14: &[i32],
    idx: usize,
    a_q12: &[i16],
    predict_lpc_order: i32,
) -> i32 {
    if predict_lpc_order <= 0 {
        return 0;
    }
    let order = predict_lpc_order as usize;
    assert!(
        idx + 1 >= order && idx < ps_lpc_q14.len() && a_q12.len() >= order,
        "silk_lpc_prediction: slice out of bounds"
    );
    #[cfg(target_arch = "aarch64")]
    {
        return unsafe { silk_lpc_prediction_neon(ps_lpc_q14, idx, a_q12, predict_lpc_order) };
    }
    #[cfg(target_arch = "x86_64")]
    {
        if have_avx2() {
            return unsafe { silk_lpc_prediction_avx2(ps_lpc_q14, idx, a_q12, predict_lpc_order) };
        }
    }
    #[cfg(not(target_arch = "aarch64"))]
    silk_lpc_prediction_scalar(ps_lpc_q14, idx, a_q12, predict_lpc_order)
}

#[cfg_attr(target_arch = "aarch64", allow(dead_code))]
fn silk_lpc_prediction_scalar(
    ps_lpc_q14: &[i32],
    idx: usize,
    a_q12: &[i16],
    predict_lpc_order: i32,
) -> i32 {
    let mut out = predict_lpc_order >> 1;
    for j in 0..predict_lpc_order as usize {
        out = silk_smlawb(out, ps_lpc_q14[idx - j], a_q12[j] as i32);
    }
    out
}

#[cfg(target_arch = "aarch64")]
#[target_feature(enable = "neon")]
#[inline]
unsafe fn silk_lpc_prediction_neon(
    ps_lpc_q14: &[i32],
    idx: usize,
    a_q12: &[i16],
    predict_lpc_order: i32,
) -> i32 {
    use std::arch::aarch64::*;
    let order = predict_lpc_order as usize;
    let lpc_base = unsafe { ps_lpc_q14.as_ptr().add(idx) };
    let a_ptr = a_q12.as_ptr();
    let mut acc = vdupq_n_s32(0i32);
    let neon_taps = order & !3;
    let mut j = 0usize;
    while j < neon_taps {
        unsafe {
            let lpc_asc = vld1q_s32(lpc_base.sub(j + 3));
            let coef_wide = vmovl_s16(vld1_s16(a_ptr.add(j)));
            let coef_rev = vrev64q_s32(vcombine_s32(
                vget_high_s32(coef_wide),
                vget_low_s32(coef_wide),
            ));
            let prod_lo = vmull_s32(vget_low_s32(lpc_asc), vget_low_s32(coef_rev));
            let prod_hi = vmull_s32(vget_high_s32(lpc_asc), vget_high_s32(coef_rev));
            let shr_lo = vshrn_n_s64::<16>(prod_lo);
            let shr_hi = vshrn_n_s64::<16>(prod_hi);
            acc = vaddq_s32(acc, vcombine_s32(shr_lo, shr_hi));
        }
        j += 4;
    }
    let mut out = (predict_lpc_order >> 1).wrapping_add(vaddvq_s32(acc));
    while j < order {
        out = silk_smlawb(out, ps_lpc_q14[idx - j], a_q12[j] as i32);
        j += 1;
    }
    out
}

#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx2")]
#[inline]
unsafe fn avx2_asr16_epi64(x: std::arch::x86_64::__m256i) -> std::arch::x86_64::__m256i {
    use std::arch::x86_64::*;
    let sign = _mm256_cmpgt_epi64(_mm256_setzero_si256(), x);
    _mm256_or_si256(_mm256_srli_epi64(x, 16), _mm256_slli_epi64(sign, 48))
}

#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx2")]
unsafe fn silk_lpc_prediction_avx2(
    ps_lpc_q14: &[i32],
    idx: usize,
    a_q12: &[i16],
    predict_lpc_order: i32,
) -> i32 {
    use std::arch::x86_64::*;
    let order = predict_lpc_order as usize;
    let lpc = ps_lpc_q14.as_ptr();
    let mut acc_e = _mm256_setzero_si256();
    let mut acc_o = _mm256_setzero_si256();
    let main = order & !7;
    let mut j = 0usize;
    while j < main {
        unsafe {
            let l = _mm256_loadu_si256(lpc.add(idx - j - 7) as *const __m256i);
            let c16 = _mm_loadu_si128(a_q12.as_ptr().add(j) as *const __m128i);
            let c = _mm256_cvtepi16_epi32(c16);
            let crev = _mm256_permutevar8x32_epi32(c, _mm256_setr_epi32(7, 6, 5, 4, 3, 2, 1, 0));
            let pe = _mm256_mul_epi32(l, crev);
            let po = _mm256_mul_epi32(
                _mm256_shuffle_epi32(l, 0b11_11_01_01),
                _mm256_shuffle_epi32(crev, 0b11_11_01_01),
            );
            acc_e = _mm256_add_epi64(acc_e, avx2_asr16_epi64(pe));
            acc_o = _mm256_add_epi64(acc_o, avx2_asr16_epi64(po));
        }
        j += 8;
    }
    let s = _mm256_add_epi64(acc_e, acc_o);
    let s2 = _mm_add_epi64(_mm256_castsi256_si128(s), _mm256_extracti128_si256(s, 1));
    let s3 = _mm_add_epi64(s2, _mm_unpackhi_epi64(s2, s2));
    let mut out = (predict_lpc_order >> 1).wrapping_add(_mm_cvtsi128_si32(s3));
    while j < order {
        out = silk_smlawb(out, ps_lpc_q14[idx - j], a_q12[j] as i32);
        j += 1;
    }
    out
}

/// Bit-exact cross-state warped shaping AR filter over `sar[tap][state]` (4 del-dec states).
#[inline]
pub fn nsq_shape_filter_soa(
    sar: &mut [[i32; 4]],
    diff: &[i32; 4],
    warp: i32,
    ar_shp_q13: &[i16],
    order: usize,
    base: i32,
    n_ar: &mut [i32; 4],
) {
    assert!(
        order >= 2 && (order & 1) == 0 && order <= 32 && sar.len() >= order && ar_shp_q13.len() >= order,
        "nsq_shape_filter_soa: invalid order or slice bounds"
    );
    #[cfg(target_arch = "aarch64")]
    {
        unsafe { nsq_shape_filter_soa_neon(sar, diff, warp, ar_shp_q13, order, base, n_ar) };
        return;
    }
    #[cfg(target_arch = "x86_64")]
    {
        if have_avx2() {
            unsafe { nsq_shape_filter_soa_avx2(sar, diff, warp, ar_shp_q13, order, base, n_ar) };
            return;
        }
    }
    #[cfg(not(target_arch = "aarch64"))]
    nsq_shape_filter_soa_scalar(sar, diff, warp, ar_shp_q13, order, base, n_ar);
}

#[cfg_attr(target_arch = "aarch64", allow(dead_code))]
fn nsq_shape_filter_soa_scalar(
    sar: &mut [[i32; 4]],
    diff: &[i32; 4],
    warp: i32,
    ar_shp_q13: &[i16],
    order: usize,
    base: i32,
    n_ar: &mut [i32; 4],
) {
    for k in 0..4 {
        let mut tmp2 = silk_smlawb(diff[k], sar[0][k], warp);
        let mut tmp1 = silk_smlawb(sar[0][k], sar[1][k].wrapping_sub(tmp2), warp);
        sar[0][k] = tmp2;
        let mut acc = silk_smlawb(base, tmp2, ar_shp_q13[0] as i32);
        let mut j = 2;
        while j < order {
            tmp2 = silk_smlawb(sar[j - 1][k], sar[j][k].wrapping_sub(tmp1), warp);
            sar[j - 1][k] = tmp1;
            acc = silk_smlawb(acc, tmp1, ar_shp_q13[j - 1] as i32);
            tmp1 = silk_smlawb(sar[j][k], sar[j + 1][k].wrapping_sub(tmp2), warp);
            sar[j][k] = tmp2;
            acc = silk_smlawb(acc, tmp2, ar_shp_q13[j] as i32);
            j += 2;
        }
        sar[order - 1][k] = tmp1;
        n_ar[k] = silk_smlawb(acc, tmp1, ar_shp_q13[order - 1] as i32);
    }
}

#[cfg(target_arch = "aarch64")]
#[target_feature(enable = "neon")]
#[inline]
unsafe fn neon_smlawb_4(
    a: std::arch::aarch64::int32x4_t,
    b: std::arch::aarch64::int32x4_t,
    c: std::arch::aarch64::int32x2_t,
) -> std::arch::aarch64::int32x4_t {
    use std::arch::aarch64::*;
    let p_lo = vshrn_n_s64::<16>(vmull_s32(vget_low_s32(b), c));
    let p_hi = vshrn_n_s64::<16>(vmull_s32(vget_high_s32(b), c));
    vaddq_s32(a, vcombine_s32(p_lo, p_hi))
}

#[cfg(target_arch = "aarch64")]
#[target_feature(enable = "neon")]
#[inline]
unsafe fn nsq_shape_filter_soa_neon(
    sar: &mut [[i32; 4]],
    diff: &[i32; 4],
    warp: i32,
    ar_shp_q13: &[i16],
    order: usize,
    base: i32,
    n_ar: &mut [i32; 4],
) {
    use std::arch::aarch64::*;
    let wv = vdup_n_s32(warp as i16 as i32);
    let mut vsar = [vdupq_n_s32(0); 32];
    for j in 0..order {
        vsar[j] = unsafe { vld1q_s32(sar[j].as_ptr()) };
    }
    let vdiff = unsafe { vld1q_s32(diff.as_ptr()) };
    let mut tmp2 = unsafe { neon_smlawb_4(vdiff, vsar[0], wv) };
    let mut tmp1 = unsafe { neon_smlawb_4(vsar[0], vsubq_s32(vsar[1], tmp2), wv) };
    vsar[0] = tmp2;
    let mut acc =
        unsafe { neon_smlawb_4(vdupq_n_s32(base), tmp2, vdup_n_s32(ar_shp_q13[0] as i32)) };
    let mut j = 2usize;
    while j < order {
        tmp2 = unsafe { neon_smlawb_4(vsar[j - 1], vsubq_s32(vsar[j], tmp1), wv) };
        vsar[j - 1] = tmp1;
        acc = unsafe { neon_smlawb_4(acc, tmp1, vdup_n_s32(ar_shp_q13[j - 1] as i32)) };
        tmp1 = unsafe { neon_smlawb_4(vsar[j], vsubq_s32(vsar[j + 1], tmp2), wv) };
        vsar[j] = tmp2;
        acc = unsafe { neon_smlawb_4(acc, tmp2, vdup_n_s32(ar_shp_q13[j] as i32)) };
        j += 2;
    }
    vsar[order - 1] = tmp1;
    acc = unsafe { neon_smlawb_4(acc, tmp1, vdup_n_s32(ar_shp_q13[order - 1] as i32)) };
    for j in 0..order {
        unsafe { vst1q_s32(sar[j].as_mut_ptr(), vsar[j]) };
    }
    unsafe { vst1q_s32(n_ar.as_mut_ptr(), acc) };
}

#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx2")]
#[inline]
unsafe fn avx2_smlawb_v(
    a: std::arch::x86_64::__m256i,
    b: std::arch::x86_64::__m256i,
    cb: std::arch::x86_64::__m256i,
) -> std::arch::x86_64::__m256i {
    use std::arch::x86_64::*;
    _mm256_add_epi64(a, unsafe { avx2_asr16_epi64(_mm256_mul_epi32(b, cb)) })
}

#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx2")]
#[inline]
unsafe fn avx2_sub32_wrap(
    a: std::arch::x86_64::__m256i,
    b: std::arch::x86_64::__m256i,
) -> std::arch::x86_64::__m256i {
    use std::arch::x86_64::*;
    let d32 = _mm256_sub_epi32(a, b);
    let packed = _mm256_permutevar8x32_epi32(d32, _mm256_setr_epi32(0, 2, 4, 6, 0, 2, 4, 6));
    _mm256_cvtepi32_epi64(_mm256_castsi256_si128(packed))
}

#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx2")]
#[inline]
unsafe fn avx2_load_soa4(p: &[i32; 4]) -> std::arch::x86_64::__m256i {
    use std::arch::x86_64::*;
    unsafe { _mm256_cvtepi32_epi64(_mm_loadu_si128(p.as_ptr() as *const __m128i)) }
}

#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx2")]
#[inline]
unsafe fn avx2_store_soa4(p: &mut [i32; 4], v: std::arch::x86_64::__m256i) {
    use std::arch::x86_64::*;
    let g = _mm256_permutevar8x32_epi32(v, _mm256_setr_epi32(0, 2, 4, 6, 0, 2, 4, 6));
    unsafe { _mm_storeu_si128(p.as_mut_ptr() as *mut __m128i, _mm256_castsi256_si128(g)) };
}

#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx2")]
unsafe fn nsq_shape_filter_soa_avx2(
    sar: &mut [[i32; 4]],
    diff: &[i32; 4],
    warp: i32,
    ar_shp_q13: &[i16],
    order: usize,
    base: i32,
    n_ar: &mut [i32; 4],
) {
    use std::arch::x86_64::*;
    let wb = _mm256_set1_epi64x(warp as i16 as i64);
    let mut vsar = [_mm256_setzero_si256(); 32];
    for j in 0..order {
        vsar[j] = unsafe { avx2_load_soa4(&sar[j]) };
    }
    let vdiff = unsafe { avx2_load_soa4(diff) };
    let mut tmp2 = unsafe { avx2_smlawb_v(vdiff, vsar[0], wb) };
    let mut tmp1 = unsafe { avx2_smlawb_v(vsar[0], avx2_sub32_wrap(vsar[1], tmp2), wb) };
    vsar[0] = tmp2;
    let mut acc = unsafe {
        avx2_smlawb_v(
            _mm256_set1_epi64x(base as i64),
            tmp2,
            _mm256_set1_epi64x(ar_shp_q13[0] as i64),
        )
    };
    let mut j = 2usize;
    while j < order {
        tmp2 = unsafe { avx2_smlawb_v(vsar[j - 1], avx2_sub32_wrap(vsar[j], tmp1), wb) };
        vsar[j - 1] = tmp1;
        acc = unsafe { avx2_smlawb_v(acc, tmp1, _mm256_set1_epi64x(ar_shp_q13[j - 1] as i64)) };
        tmp1 = unsafe { avx2_smlawb_v(vsar[j], avx2_sub32_wrap(vsar[j + 1], tmp2), wb) };
        vsar[j] = tmp2;
        acc = unsafe { avx2_smlawb_v(acc, tmp2, _mm256_set1_epi64x(ar_shp_q13[j] as i64)) };
        j += 2;
    }
    vsar[order - 1] = tmp1;
    acc = unsafe { avx2_smlawb_v(acc, tmp1, _mm256_set1_epi64x(ar_shp_q13[order - 1] as i64)) };
    for j in 0..order {
        unsafe { avx2_store_soa4(&mut sar[j], vsar[j]) };
    }
    unsafe { avx2_store_soa4(n_ar, acc) };
}

/// Bit-exact 8-tap fractional-FIR dot product in wrapping `i32`.
#[inline]
pub fn resampler_fir12_8(buf8: &[i16], c: &[i16; 8]) -> i32 {
    let buf8 = &buf8[..8];
    #[cfg(target_arch = "aarch64")]
    {
        return unsafe { resampler_fir12_8_neon(buf8, c) };
    }
    #[cfg(target_arch = "x86_64")]
    {
        return unsafe { resampler_fir12_8_sse2(buf8, c) };
    }
    #[allow(unreachable_code)]
    {
        let mut r = 0i32;
        for j in 0..8 {
            r = r.wrapping_add((buf8[j] as i32) * (c[j] as i32));
        }
        r
    }
}

#[cfg(target_arch = "aarch64")]
#[target_feature(enable = "neon")]
#[inline]
unsafe fn resampler_fir12_8_neon(buf8: &[i16], c: &[i16; 8]) -> i32 {
    use std::arch::aarch64::*;
    unsafe {
        let b = vld1q_s16(buf8.as_ptr());
        let cc = vld1q_s16(c.as_ptr());
        let p_lo = vmull_s16(vget_low_s16(b), vget_low_s16(cc));
        let acc = vmlal_high_s16(p_lo, b, cc);
        vaddvq_s64(vpaddlq_s32(acc)) as i32
    }
}

#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "sse2")]
unsafe fn resampler_fir12_8_sse2(buf8: &[i16], c: &[i16; 8]) -> i32 {
    use std::arch::x86_64::*;
    unsafe {
        let b = _mm_loadu_si128(buf8.as_ptr() as *const __m128i);
        let cc = _mm_loadu_si128(c.as_ptr() as *const __m128i);
        let m = _mm_madd_epi16(b, cc);
        let t = _mm_add_epi32(m, _mm_shuffle_epi32(m, 0b01_00_11_10));
        let t = _mm_add_epi32(t, _mm_shuffle_epi32(t, 0b00_00_00_01));
        _mm_cvtsi128_si32(t)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn silk_fixed_point_simd_bit_exact_with_scalar() {
        let mut s = 0x1234_5678_9ABC_DEF1u64;
        let mut rng = || {
            s ^= s << 13;
            s ^= s >> 7;
            s ^= s << 17;
            s
        };

        // 1. resampler_fir12_8
        for _ in 0..256 {
            let mut b = [0i16; 8];
            let mut c = [0i16; 8];
            for k in 0..8 {
                b[k] = rng() as i16;
                c[k] = rng() as i16;
            }
            let want = (0..8).fold(0i32, |acc, j| acc.wrapping_add((b[j] as i32) * (c[j] as i32)));
            assert_eq!(resampler_fir12_8(&b, &c), want);
        }

        // 2. silk_lpc_prediction & warped_corr_update & nsq_shape_filter_soa
        for order in [10usize, 12, 14, 16, 24] {
            for _ in 0..200 {
                if order <= 16 {
                    let mut lpc = [0i32; 64];
                    for v in &mut lpc {
                        *v = (rng() as i32) >> 6;
                    }
                    let mut a = [0i16; 16];
                    for v in a.iter_mut().take(order) {
                        *v = (rng() as i16) >> 2;
                    }
                    let idx = 32 + (rng() as usize % 16);
                    assert_eq!(
                        silk_lpc_prediction(&lpc, idx, &a, order as i32),
                        silk_lpc_prediction_scalar(&lpc, idx, &a, order as i32)
                    );

                    let mut c_got = [0i64; 25];
                    let mut c_want = [0i64; 25];
                    let mut st = [0i32; 25];
                    for k in 0..=order {
                        st[k] = (rng() as i32) >> 3;
                        let init = (rng() as i64) >> 12;
                        c_got[k] = init;
                        c_want[k] = init;
                    }
                    warped_corr_update(&mut c_got, &st, order);
                    warped_corr_update_scalar(&mut c_want[..=order], &st[..=order]);
                    assert_eq!(c_got, c_want);
                }

                let mut sar_got = vec![[0i32; 4]; order];
                for row in &mut sar_got {
                    for v in row {
                        *v = (rng() as i32) >> 8;
                    }
                }
                let mut sar_want = sar_got.clone();
                let diff = [
                    (rng() as i32) >> 8,
                    (rng() as i32) >> 8,
                    (rng() as i32) >> 8,
                    (rng() as i32) >> 8,
                ];
                let ar: Vec<i16> = (0..order).map(|_| (rng() >> 8) as i16).collect();
                let mut n_got = [0i32; 4];
                let mut n_want = [0i32; 4];
                nsq_shape_filter_soa(&mut sar_got, &diff, 13421, &ar, order, 8, &mut n_got);
                nsq_shape_filter_soa_scalar(&mut sar_want, &diff, 13421, &ar, order, 8, &mut n_want);
                assert_eq!(n_got, n_want);
                assert_eq!(sar_got, sar_want);
            }
        }
    }

    #[test]
    fn celt_float_simd_matches_scalar() {
        let n = 127usize;
        let x: Vec<f32> = (0..256).map(|i| ((i as f32) * 0.13).sin()).collect();
        let y: Vec<f32> = (0..256).map(|i| ((i as f32) * 0.07).cos()).collect();

        let ip_got = inner_prod(&x, &y, n);
        let ip_want = inner_prod_scalar(&x, &y, n);
        assert!((ip_got - ip_want).abs() < 1e-4);

        let (d1, d2) = dual_inner_prod(&x, &y, &x[3..], n);
        let (w1, w2) = dual_inner_prod_scalar(&x, &y, &x[3..], n);
        assert!((d1 - w1).abs() < 1e-4 && (d2 - w2).abs() < 1e-4);

        let mut xc_got = vec![0.0f32; 37];
        let mut xc_want = vec![0.0f32; 37];
        pitch_xcorr(&x, &y, &mut xc_got, 65, 37);
        for i in 0..37 {
            xc_want[i] = inner_prod_scalar(&x, &y[i..], 65);
            assert!((xc_got[i] - xc_want[i]).abs() < 1e-4);
        }

        // In-place comb filter bit-exactness
        let mut b_got = x.clone();
        let mut b_want = x.clone();
        let resumed = comb_filter_const_inplace(&mut b_got, 30, 0, 96, 15, 0.3, 0.15, 0.05);
        for i in resumed..96 {
            let idx = 30 + i;
            b_got[idx] = b_got[idx]
                + 0.3 * b_got[idx - 15]
                + 0.15 * (b_got[idx - 14] + b_got[idx - 16])
                + 0.05 * (b_got[idx - 13] + b_got[idx - 17]);
        }
        for i in 0..96 {
            let idx = 30 + i;
            b_want[idx] = b_want[idx]
                + 0.3 * b_want[idx - 15]
                + 0.15 * (b_want[idx - 14] + b_want[idx - 16])
                + 0.05 * (b_want[idx - 13] + b_want[idx - 17]);
        }
        assert_eq!(b_got, b_want);
    }
}
