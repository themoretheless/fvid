use std::f32::consts::PI;

pub use fvid_cpu::opus_simd::KissCpx;

pub const MAXFACTORS: usize = 8;

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

pub struct KissFftState {
    nfft: usize,
    scale: f32,
    shift: i32,
    factors: [i16; 2 * MAXFACTORS],
    pub bitrev: Vec<i16>,
    twiddles: Vec<KissCpx>,
}

fn kf_factor(n_orig: usize, factors: &mut [i16; 2 * MAXFACTORS]) -> bool {
    let mut n = n_orig;
    let mut p: i32 = 4;
    let mut stages = 0;

    loop {
        while !n.is_multiple_of(p as usize) {
            p = match p {
                4 => 2,
                2 => 3,
                _ => p + 2,
            };
            if p > 32000 || (p as i64) * (p as i64) > n as i64 {
                p = n as i32;
            }
        }
        n /= p as usize;

        if p > 5 {
            return false;
        }

        factors[2 * stages] = p as i16;

        if p == 2 && stages > 1 {
            factors[2 * stages] = 4;
            factors[2] = 2;
        }
        stages += 1;

        if n <= 1 {
            break;
        }
    }

    for i in 0..(stages / 2) {
        factors.swap(2 * i, 2 * (stages - i - 1));
    }

    n = n_orig;
    for i in 0..stages {
        n /= factors[2 * i] as usize;
        factors[2 * i + 1] = n as i16;
    }

    true
}

fn compute_bitrev_table(
    fout: i32,
    f: &mut [i16],
    fstride: usize,
    in_stride: usize,
    factors: &[i16],
) {
    let p = factors[0] as i32;
    let m = factors[1] as i32;

    if m == 1 {
        for j in 0..p {
            let idx = (j as usize) * fstride * in_stride;
            f[idx] = (fout + j) as i16;
        }
    } else {
        let mut fout = fout;
        let mut f_offset = 0usize;
        for _ in 0..p {
            compute_bitrev_table(
                fout,
                &mut f[f_offset..],
                fstride * (p as usize),
                in_stride,
                &factors[2..],
            );
            f_offset += fstride * in_stride;
            fout += m;
        }
    }
}

fn compute_twiddles(nfft: usize) -> Vec<KissCpx> {
    let two_pi_over_n = -2.0 * PI / nfft as f32;
    (0..nfft)
        .map(|i| {
            let phase = two_pi_over_n * i as f32;
            KissCpx::new(phase.cos(), phase.sin())
        })
        .collect()
}

impl KissFftState {
    pub fn new(nfft: usize) -> Option<Self> {
        let mut factors = [0i16; 2 * MAXFACTORS];
        if !kf_factor(nfft, &mut factors) {
            return None;
        }

        let scale = 1.0 / nfft as f32;
        let twiddles = compute_twiddles(nfft);

        let mut bitrev = vec![0i16; nfft];
        compute_bitrev_table(0, &mut bitrev, 1, 1, &factors);

        Some(Self {
            nfft,
            scale,
            shift: -1,
            factors,
            bitrev,
            twiddles,
        })
    }

    pub fn new_sub(base: &KissFftState, nfft: usize) -> Option<Self> {
        let mut factors = [0i16; 2 * MAXFACTORS];
        if !kf_factor(nfft, &mut factors) {
            return None;
        }

        let mut shift = 0i32;
        while shift < 32 && (nfft << shift) != base.nfft {
            shift += 1;
        }
        if shift >= 32 {
            return None;
        }

        let mut bitrev = vec![0i16; nfft];
        compute_bitrev_table(0, &mut bitrev, 1, 1, &factors);

        Some(Self {
            nfft,
            scale: 1.0 / nfft as f32,
            shift,
            factors,
            bitrev,
            twiddles: base.twiddles.clone(),
        })
    }

    #[inline]
    pub fn scale(&self) -> f32 {
        self.scale
    }
}

#[inline(always)]
fn kf_bfly2(fout: &mut [KissCpx], m: usize, n: usize) {
    if m == 1 {
        fvid_cpu::opus_simd::kf_bfly2_m1(fout, n);
    } else {
        let tw: f32 = std::f32::consts::FRAC_1_SQRT_2;
        for i in 0..n {
            let base = i * 8;

            let t = fout[base + 4];
            fout[base + 4] = c_sub(&fout[base], &t);
            fout[base] = c_add(&fout[base], &t);

            let t = KissCpx::new(
                (fout[base + 5].r + fout[base + 5].i) * tw,
                (fout[base + 5].i - fout[base + 5].r) * tw,
            );
            fout[base + 5] = c_sub(&fout[base + 1], &t);
            fout[base + 1] = c_add(&fout[base + 1], &t);

            let t = KissCpx::new(fout[base + 6].i, -fout[base + 6].r);
            fout[base + 6] = c_sub(&fout[base + 2], &t);
            fout[base + 2] = c_add(&fout[base + 2], &t);

            let t = KissCpx::new(
                (fout[base + 7].i - fout[base + 7].r) * tw,
                -(fout[base + 7].i + fout[base + 7].r) * tw,
            );
            fout[base + 7] = c_sub(&fout[base + 3], &t);
            fout[base + 3] = c_add(&fout[base + 3], &t);
        }
    }
}

#[inline(always)]
fn kf_bfly4(
    fout: &mut [KissCpx],
    fstride: usize,
    twiddles: &[KissCpx],
    m: usize,
    n: usize,
    mm: usize,
) {
    if m == 1 {
        fvid_cpu::opus_simd::kf_bfly4_m1(fout, n);
    } else {
        let stride2 = fstride * 2;
        let stride3 = fstride * 3;
        let m2 = 2 * m;
        let m3 = 3 * m;
        for i in 0..n {
            let base = i * mm;
            let mut tw1 = 0usize;
            let mut tw2 = 0usize;
            let mut tw3 = 0usize;

            for j in 0..m {
                let idx = base + j;

                let scratch0 = c_mul(&fout[idx + m], &twiddles[tw1]);
                let scratch1 = c_mul(&fout[idx + m2], &twiddles[tw2]);
                let scratch2 = c_mul(&fout[idx + m3], &twiddles[tw3]);

                let scratch5 = c_sub(&fout[idx], &scratch1);
                fout[idx] = c_add(&fout[idx], &scratch1);

                let scratch3 = c_add(&scratch0, &scratch2);
                let scratch4 = c_sub(&scratch0, &scratch2);

                fout[idx + m2] = c_sub(&fout[idx], &scratch3);
                fout[idx] = c_add(&fout[idx], &scratch3);

                fout[idx + m] = KissCpx::new(scratch5.r + scratch4.i, scratch5.i - scratch4.r);
                fout[idx + m3] = KissCpx::new(scratch5.r - scratch4.i, scratch5.i + scratch4.r);

                tw1 += fstride;
                tw2 += stride2;
                tw3 += stride3;
            }
        }
    }
}

#[inline(always)]
fn kf_bfly3(
    fout: &mut [KissCpx],
    fstride: usize,
    twiddles: &[KissCpx],
    m: usize,
    n: usize,
    mm: usize,
) {
    fvid_cpu::opus_simd::kf_bfly3(fout, fstride, twiddles, m, n, mm);
}

#[inline(always)]
fn kf_bfly5(
    fout: &mut [KissCpx],
    fstride: usize,
    twiddles: &[KissCpx],
    m: usize,
    n: usize,
    mm: usize,
) {
    fvid_cpu::opus_simd::kf_bfly5(fout, fstride, twiddles, m, n, mm);
}

pub fn opus_fft_impl(st: &KissFftState, fout: &mut [KissCpx]) {
    let factors = &st.factors;
    let twiddles = &st.twiddles;

    let mut fstride = [0usize; MAXFACTORS + 1];
    fstride[0] = 1;
    let mut l = 0;
    let mut m;

    loop {
        let p = factors[2 * l] as usize;
        m = factors[2 * l + 1] as usize;
        fstride[l + 1] = fstride[l] * p;
        l += 1;
        if m == 1 {
            break;
        }
    }

    let shift = if st.shift > 0 { st.shift as usize } else { 0 };

    m = factors[2 * l - 1] as usize;
    for i in (0..l).rev() {
        let p = factors[2 * i] as usize;
        let fstride_i = fstride[i];
        let fstride_adjusted = fstride_i << shift;

        let m2 = if i > 0 {
            factors[2 * i - 1] as usize
        } else {
            1
        };

        match p {
            2 => kf_bfly2(fout, m, fstride_i),
            4 => kf_bfly4(fout, fstride_adjusted, twiddles, m, fstride_i, m2),
            3 => kf_bfly3(fout, fstride_adjusted, twiddles, m, fstride_i, m2),
            5 => kf_bfly5(fout, fstride_adjusted, twiddles, m, fstride_i, m2),
            _ => {}
        }

        m = m2;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Whole-transform forward FFT. Production code drives `opus_fft_impl`
    /// directly with its own bit-reversal, so this scaffolding is test-only —
    /// it exists to give the round-trip tests a clean forward/inverse pair.
    fn opus_fft(st: &KissFftState, fin: &[KissCpx], fout: &mut [KissCpx]) {
        for i in 0..st.nfft {
            let x = &fin[i];
            fout[st.bitrev[i] as usize] = KissCpx::new(x.r * st.scale, x.i * st.scale);
        }
        opus_fft_impl(st, fout);
    }

    /// Inverse of [`opus_fft`], by conjugation.
    fn opus_ifft(st: &KissFftState, fin: &[KissCpx], fout: &mut [KissCpx]) {
        for i in 0..st.nfft {
            fout[st.bitrev[i] as usize] = KissCpx::new(fin[i].r, -fin[i].i);
        }
        opus_fft_impl(st, fout);
        for v in fout.iter_mut().take(st.nfft) {
            v.i = -v.i;
        }
    }

    fn almost_equal(a: f32, b: f32, tolerance: f32) -> bool {
        (a - b).abs() < tolerance
    }

    fn cpx_almost_equal(a: &KissCpx, b: &KissCpx, tolerance: f32) -> bool {
        almost_equal(a.r, b.r, tolerance) && almost_equal(a.i, b.i, tolerance)
    }

    #[test]
    fn test_kf_factor_60() {
        let st = KissFftState::new(60).unwrap();
        assert!(!st.factors.iter().all(|&f| f == 0));
        assert_eq!(st.bitrev.len(), 60);
    }

    #[test]
    fn test_kf_factor_240() {
        let st = KissFftState::new(240).unwrap();
        assert!(!st.factors.iter().all(|&f| f == 0));
        assert_eq!(st.bitrev.len(), 240);
    }

    #[test]
    fn test_bitrev_permutation() {
        for &nfft in &[60, 120, 240, 480] {
            let st = KissFftState::new(nfft).unwrap();
            let mut sorted: Vec<i16> = st.bitrev.clone();
            sorted.sort();
            let expected: Vec<i16> = (0..nfft).map(|x| x as i16).collect();
            assert_eq!(
                sorted,
                expected,
                "bitrev for nfft={} should be a permutation of 0..{}",
                nfft,
                nfft - 1
            );
        }
    }

    #[test]
    fn test_sub_fft() {
        let nfft = 480;
        let base = KissFftState::new(nfft).unwrap();

        for &n in &[60, 120, 240] {
            let sub = KissFftState::new_sub(&base, n).unwrap();
            assert_eq!(sub.nfft, n);
            assert_eq!(sub.twiddles.len(), nfft);
        }
    }

    #[test]
    fn test_fft_roundtrip_60() {
        let nfft = 60;
        let st = KissFftState::new(nfft).unwrap();

        let mut fin = vec![KissCpx::default(); nfft];
        let mut fout = vec![KissCpx::default(); nfft];
        let mut finv = vec![KissCpx::default(); nfft];

        fin[0] = KissCpx::new(1.0, 0.0);

        opus_fft(&st, &fin, &mut fout);
        opus_ifft(&st, &fout, &mut finv);

        for i in 0..nfft {
            let expected = if i == 0 { 1.0 } else { 0.0 };
            assert!(
                cpx_almost_equal(&finv[i], &KissCpx::new(expected, 0.0), 1e-5),
                "Roundtrip failed at index {}: got ({}, {}), expected ({}, 0)",
                i,
                finv[i].r,
                finv[i].i,
                expected
            );
        }
    }

    #[test]
    fn test_fft_roundtrip_120() {
        let nfft = 120;
        let st = KissFftState::new(nfft).unwrap();

        let mut fin = vec![KissCpx::default(); nfft];
        let mut fout = vec![KissCpx::default(); nfft];
        let mut finv = vec![KissCpx::default(); nfft];

        for i in 0..nfft {
            fin[i] = KissCpx::new((2.0 * PI * 3.0 * i as f32 / nfft as f32).sin(), 0.0);
        }

        opus_fft(&st, &fin, &mut fout);
        opus_ifft(&st, &fout, &mut finv);

        for i in 0..nfft {
            assert!(
                cpx_almost_equal(&finv[i], &fin[i], 1e-4),
                "Roundtrip failed at index {}: got ({}, {}), expected ({}, {})",
                i,
                finv[i].r,
                finv[i].i,
                fin[i].r,
                fin[i].i
            );
        }
    }

    #[test]
    fn test_fft_roundtrip_480() {
        let nfft = 480;
        let st = KissFftState::new(nfft).unwrap();

        let mut fin = vec![KissCpx::default(); nfft];
        let mut fout = vec![KissCpx::default(); nfft];
        let mut finv = vec![KissCpx::default(); nfft];

        for i in 0..nfft {
            fin[i] = KissCpx::new(
                (2.0 * PI * 7.0 * i as f32 / nfft as f32).sin(),
                (2.0 * PI * 11.0 * i as f32 / nfft as f32).cos(),
            );
        }

        opus_fft(&st, &fin, &mut fout);
        opus_ifft(&st, &fout, &mut finv);

        for i in 0..nfft {
            assert!(
                cpx_almost_equal(&finv[i], &fin[i], 1e-4),
                "Roundtrip failed at index {}: got ({}, {}), expected ({}, {})",
                i,
                finv[i].r,
                finv[i].i,
                fin[i].r,
                fin[i].i
            );
        }
    }

    #[test]
    fn test_fft_dc_component() {
        let nfft = 120;
        let st = KissFftState::new(nfft).unwrap();

        let mut fin = vec![KissCpx::default(); nfft];
        let mut fout = vec![KissCpx::default(); nfft];

        for i in 0..nfft {
            fin[i] = KissCpx::new(1.0, 0.0);
        }

        opus_fft(&st, &fin, &mut fout);

        assert!(almost_equal(fout[0].r, 1.0, 1e-5));
        assert!(almost_equal(fout[0].i, 0.0, 1e-5));

        for i in 1..nfft {
            assert!(
                cpx_almost_equal(&fout[i], &KissCpx::new(0.0, 0.0), 1e-5),
                "Non-DC component at index {} is ({}, {}), expected (0, 0)",
                i,
                fout[i].r,
                fout[i].i
            );
        }
    }

    // ---- SIMD butterflies vs their scalar definitions ---------------------

    fn cpx_noise(n: usize, seed: u32) -> Vec<KissCpx> {
        let mut s = seed | 1;
        let mut next = || {
            s = s.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            (s >> 8) as f32 / (1 << 23) as f32 - 1.0
        };
        (0..n).map(|_| KissCpx::new(next(), next())).collect()
    }

    fn close_cpx(got: &[KissCpx], want: &[KissCpx], what: &str) {
        let scale = want
            .iter()
            .fold(1.0f32, |m, c| m.max(c.r.abs()).max(c.i.abs()));
        for (i, (g, w)) in got.iter().zip(want).enumerate() {
            assert!(
                (g.r - w.r).abs() <= 1e-5 * scale && (g.i - w.i).abs() <= 1e-5 * scale,
                "{what}: element {i}: ({}, {}) vs ({}, {})",
                g.r,
                g.i,
                w.r,
                w.i
            );
        }
    }

    /// The `m == 1` radix-2 and radix-4 butterflies, each held against the
    /// arithmetic it is supposed to perform, written out here independently of
    /// the implementations.
    ///
    /// Both kernels are checked on every target, and the NEON forms are checked
    /// as well wherever they exist. The earlier version of this test called only
    /// the NEON kernel, behind a `cfg`, so off aarch64 it compared the
    /// untransformed input against the transformed reference and could not pass.
    /// Testing the scalar kernel too is what makes the name true on x86_64.
    ///
    /// The other radices have no `m == 1` special case; `fft_matches_a_direct_dft`
    /// covers them, on every target, against the transform they implement.
    #[test]
    fn butterfly_kernels_match_their_definitions() {
        for &n in &[1usize, 2, 3, 4, 7, 8, 15, 16, 33, 64] {
            // radix 2
            {
                let src = cpx_noise(2 * n, 0x8001 ^ n as u32);
                let mut want = src.clone();
                for i in 0..n {
                    let idx = i * 2;
                    let t = want[idx + 1];
                    want[idx + 1] = c_sub(&want[idx], &t);
                    want[idx] = c_add(&want[idx], &t);
                }
                let mut scalar = src.clone();
                fvid_cpu::opus_simd::kf_bfly2_m1_scalar(&mut scalar, n);
                close_cpx(&scalar, &want, &format!("kf_bfly2 scalar m=1 n={n}"));

                let mut simd = src.clone();
                fvid_cpu::opus_simd::kf_bfly2_m1(&mut simd, n);
                close_cpx(&simd, &want, &format!("kf_bfly2 simd m=1 n={n}"));
            }

            // radix 4
            {
                let src = cpx_noise(4 * n, 0x8002 ^ n as u32);
                let mut want = src.clone();
                for i in 0..n {
                    let base = i * 4;
                    let scratch0 = c_sub(&want[base], &want[base + 2]);
                    let sum02 = c_add(&want[base], &want[base + 2]);
                    let scratch1 = c_add(&want[base + 1], &want[base + 3]);
                    let diff13 = c_sub(&want[base + 1], &want[base + 3]);
                    want[base] = c_add(&sum02, &scratch1);
                    want[base + 2] = c_sub(&sum02, &scratch1);
                    want[base + 1] = KissCpx::new(scratch0.r + diff13.i, scratch0.i - diff13.r);
                    want[base + 3] = KissCpx::new(scratch0.r - diff13.i, scratch0.i + diff13.r);
                }
                let mut scalar = src.clone();
                fvid_cpu::opus_simd::kf_bfly4_m1_scalar(&mut scalar, n);
                close_cpx(&scalar, &want, &format!("kf_bfly4 scalar m=1 n={n}"));

                let mut simd = src.clone();
                fvid_cpu::opus_simd::kf_bfly4_m1(&mut simd, n);
                close_cpx(&simd, &want, &format!("kf_bfly4 simd m=1 n={n}"));
            }
        }
    }

    /// And the whole transform against the definition it implements. The
    /// butterflies above are only the `m == 1` cases; this covers every radix
    /// the factoriser picks, for each size CELT actually uses.
    #[test]
    fn fft_matches_a_direct_dft() {
        use std::f64::consts::PI;
        for &nfft in &[60usize, 120, 240, 480] {
            let st = KissFftState::new(nfft).expect("FFT size not factorable");
            let src = cpx_noise(nfft, 0x8003 ^ nfft as u32);

            // `opus_fft_impl` transforms in place and takes its input already
            // scattered through the bit-reversal table, which is how the MDCT
            // feeds it. Output is in natural order and unscaled.
            let mut got = vec![KissCpx::new(0.0, 0.0); nfft];
            for (i, x) in src.iter().enumerate() {
                got[st.bitrev[i] as usize] = *x;
            }
            opus_fft_impl(&st, &mut got);

            for k in 0..nfft {
                let (mut re, mut im) = (0.0f64, 0.0f64);
                for (j, s) in src.iter().enumerate() {
                    let ang = -2.0 * PI * (k as f64) * (j as f64) / nfft as f64;
                    let (sn, cs) = ang.sin_cos();
                    re += s.r as f64 * cs - s.i as f64 * sn;
                    im += s.r as f64 * sn + s.i as f64 * cs;
                }
                let (wr, wi) = (re, im);
                let tol = 1e-3 * (1.0 + wr.abs().max(wi.abs()));
                assert!(
                    (got[k].r as f64 - wr).abs() <= tol && (got[k].i as f64 - wi).abs() <= tol,
                    "fft nfft={nfft} bin {k}: ({}, {}) vs ({wr}, {wi})",
                    got[k].r,
                    got[k].i
                );
            }
        }
    }
}
