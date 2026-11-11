use crate::celt::kiss_fft::{KissCpx, KissFftState, opus_fft_impl};
use std::f32::consts::PI;

const MAX_N2: usize = 960;
const MAX_N4: usize = 480;

pub struct MdctLookup {
    pub n: usize,
    kfft: Vec<Option<KissFftState>>,
    trig: Vec<f32>,
}

impl MdctLookup {
    pub fn new(n: usize, max_lm: usize) -> Self {
        let mut kfft = Vec::new();
        let mut trig = Vec::new();
        let mut curr_n = n;

        for shift in 0..=max_lm {
            let n4 = curr_n / 4;

            if shift == 0 {
                kfft.push(KissFftState::new(n4));
            } else if let Some(base) = kfft.first().unwrap().as_ref() {
                kfft.push(KissFftState::new_sub(base, n4));
            } else {
                kfft.push(None);
            }

            let n2 = curr_n / 2;
            for i in 0..n2 {
                let angle = 2.0 * PI * (i as f32 + 0.125) / curr_n as f32;
                trig.push(angle.cos());
            }

            curr_n >>= 1;
        }

        Self { n, kfft, trig }
    }

    fn get_trig(&self, shift: usize) -> (&[f32], usize) {
        let mut offset = 0;
        let mut curr_n = self.n;
        for _ in 0..shift {
            offset += curr_n / 2;
            curr_n >>= 1;
        }
        (&self.trig[offset..offset + curr_n / 2], curr_n / 4)
    }

    #[inline]
    pub fn forward(
        &self,
        input: &[f32],
        output: &mut [f32],
        window: &[f32],
        overlap: usize,
        shift: usize,
        stride: usize,
    ) {
        let st = self.kfft[shift]
            .as_ref()
            .expect("FFT state not initialized");
        let n = self.n >> shift;
        let n2 = n / 2;
        let n4 = n / 4;
        let scale = st.scale();

        let (trig, _) = self.get_trig(shift);
        let overlap2 = overlap / 2;

        assert!(
            n2 <= MAX_N2 && n4 <= MAX_N4,
            "MDCT forward: transform size {n} exceeds the scratch bound"
        );
        let mut f_buf = [0.0f32; MAX_N2];
        let mut f2_buf = [KissCpx { r: 0.0, i: 0.0 }; MAX_N4];

        let f = &mut f_buf[..n2];
        let f2 = &mut f2_buf[..n4];

        assert!(input.len() >= n2 + overlap2);
        assert!(window.len() >= overlap);
        assert!(
            output.len() >= n2,
            "MDCT forward: output buffer too small (need {}, have {})",
            n2,
            output.len()
        );

        {
            let mut yp = 0usize;
            let mut xp1 = overlap2;
            let mut xp2 = n2 - 1 + overlap2;
            let mut wp1 = overlap2;

            let mut wp2 = overlap2.saturating_sub(1);

            let limit = overlap.div_ceil(4);
            let mid = n4.saturating_sub(limit);

            let loop1_iters = limit.min(n4);
            for _ in 0..loop1_iters {
                let w1 = window[wp1];
                let w2 = window[wp2];

                f[yp] = input[xp1 + n2] * w2 + input[xp2] * w1;
                yp += 1;

                f[yp] = input[xp1] * w1 - input[xp2 - n2] * w2;
                yp += 1;

                xp1 += 2;
                xp2 -= 2;
                wp1 += 2;
                wp2 = wp2.saturating_sub(2);
            }

            for _ in limit..mid {
                f[yp] = input[xp2];
                yp += 1;

                f[yp] = input[xp1];
                yp += 1;
                xp1 += 2;
                xp2 -= 2;
            }

            // C: after the middle loop, i == max(limit, N4-limit) and the third
            // loop runs to N4. The old `if mid > limit {..} else { 0 }` yielded
            // ZERO iterations when mid <= limit — exactly the short-block case
            // (N == 2*overlap: n4 = 2*limit), leaving f[2*limit..n2) UNWRITTEN:
            // uninitialized-stack reads on every transient sub-MDCT (this is
            // what made the HYB-VBR bitstream hash move across builds).
            let loop3_iters = n4 - limit.max(mid);
            let mut wp1_l3 = 0usize;
            let mut wp2_l3 = overlap.saturating_sub(1);
            for _ in 0..loop3_iters {
                let w1 = window[wp1_l3];
                let w2 = window[wp2_l3];

                f[yp] = -input[xp1 - n2] * w1 + input[xp2] * w2;
                yp += 1;

                f[yp] = input[xp1] * w2 + input[xp2 + n2] * w1;
                yp += 1;

                xp1 += 2;
                xp2 -= 2;
                wp1_l3 += 2;
                wp2_l3 -= 2;
            }
        }

        fvid_cpu::opus_simd::mdct_pre_rotation(f, f2, trig, &st.bitrev[..n4], n4, scale);

        opus_fft_impl(st, f2);

        fvid_cpu::opus_simd::mdct_post_rotation(f2, trig, output, n4, n2, stride);
    }

    #[inline]
    pub fn backward(
        &self,
        input: &[f32],
        output: &mut [f32],
        window: &[f32],
        overlap: usize,
        shift: usize,
        stride: usize,
    ) {
        // A malformed frame can carry an out-of-range `shift` or a `stride`/size
        // inconsistent with the input buffer; bail gracefully rather than panic on
        // the FFT-state index or the (checked) pre/post-rotation reads. Inert for
        // valid streams (shift in range, buffers correctly sized).
        let Some(st) = self.kfft.get(shift).and_then(|s| s.as_ref()) else {
            return;
        };
        let n = self.n >> shift;
        let n2 = n / 2;
        let n4 = n / 4;
        let overlap2 = overlap / 2;
        if n4 == 0 || n4 > MAX_N4 || stride.saturating_mul(n2.saturating_sub(1)) >= input.len() {
            return;
        }

        let (trig, _) = self.get_trig(shift);

        let mut f2_buf = [KissCpx { r: 0.0, i: 0.0 }; MAX_N4];

        let f2 = &mut f2_buf[..n4];

        fvid_cpu::opus_simd::mdct_backward_pre_rotation(
            input,
            f2,
            trig,
            &st.bitrev[..n4],
            n4,
            n2,
            stride,
        );

        opus_fft_impl(st, f2);

        assert!(output.len() >= overlap2 + n2);

        mdct_backward_post_rotation_scalar(f2, trig, output, n4, n2, overlap2);

        fvid_cpu::opus_simd::mdct_tdac(output, window, overlap);
    }
}

fn mdct_backward_post_rotation_scalar(
    f2: &[KissCpx],
    trig: &[f32],
    output: &mut [f32],
    n4: usize,
    n2: usize,
    overlap2: usize,
) {
    for i in 0..((n4 + 1) >> 1) {
        let im0 = f2[i].r;
        let re0 = f2[i].i;
        let t0_0 = trig[i];
        let t1_0 = trig[n4 + i];

        let yr0 = re0 * t0_0 + im0 * t1_0;
        let yi0 = re0 * t1_0 - im0 * t0_0;

        let j = n4 - 1 - i;
        let im1 = f2[j].r;
        let re1 = f2[j].i;
        let t0_1 = trig[j];
        let t1_1 = trig[n4 + j];

        let yr1 = re1 * t0_1 + im1 * t1_1;
        let yi1 = re1 * t1_1 - im1 * t0_1;

        output[overlap2 + 2 * i] = yr0;
        output[overlap2 + n2 - 1 - 2 * i] = yi0;
        output[overlap2 + n2 - 2 - 2 * i] = yr1;
        output[overlap2 + 2 * i + 1] = yi1;
    }
}

#[cfg(test)]
mod mdct_tests {
    use super::*;

    #[test]
    fn test_mdct_backward_transient_no_blowup() {
        let mode = crate::celt::modes::default_mode();
        let shift = 3;
        let n = mode.mdct.n >> shift; // 120
        let overlap = mode.overlap; // 120
        let stride = 8;

        let frame_size = 960usize;
        let mut freq = vec![0.0f32; frame_size];
        for i in 0..frame_size {
            freq[i] = ((i as f32) * 0.01).sin() * 10.0;
        }

        let out_len = n + overlap; // 240
        let mut output0 = vec![0.0f32; out_len];
        let mut output1 = vec![0.0f32; out_len];

        mode.mdct.backward(
            &freq[0..],
            &mut output0,
            mode.window,
            overlap,
            shift,
            stride,
        );
        mode.mdct.backward(
            &freq[1..],
            &mut output1,
            mode.window,
            overlap,
            shift,
            stride,
        );

        let max0 = output0.iter().cloned().fold(f32::NEG_INFINITY, f32::max);
        let max1 = output1.iter().cloned().fold(f32::NEG_INFINITY, f32::max);

        assert!(max0.abs() < 500.0, "sub0 blowup: {}", max0);
        assert!(max1.abs() < 500.0, "sub1 blowup: {}", max1);
    }

    #[test]
    fn test_mdct_backward_stride1_neon_matches_scalar() {
        let mode = crate::celt::modes::default_mode();
        let shift = 0; // non-transient full-size MDCT
        let n = mode.mdct.n >> shift; // 1920
        let n2 = n / 2; // 960
        let n4 = n / 4; // 480
        let overlap = mode.overlap; // 120
        let overlap2 = overlap / 2; // 60
        let stride = 1;

        let freq_len = n2;
        let mut freq = vec![0.0f32; freq_len + 4];
        for i in 0..freq_len {
            freq[i] = ((i as f32) * 0.01).sin() * 4577.0;
        }

        let out_len = overlap2 + n2; // 60 + 960 = 1020
        let mut output_hw = vec![0.0f32; out_len + 100];
        mode.mdct.backward(
            &freq[..],
            &mut output_hw,
            mode.window,
            overlap,
            shift,
            stride,
        );

        let st = mode.mdct.kfft[shift].as_ref().unwrap();
        let (trig, _) = mode.mdct.get_trig(shift);

        use crate::celt::kiss_fft::KissCpx;
        let mut f2 = vec![KissCpx::new(0.0, 0.0); n4];
        for i in 0..n4 {
            let rev = st.bitrev[i] as usize;
            let x1 = freq[2 * i * stride];
            let x2 = freq[stride * (n2 - 1 - 2 * i)];
            let t0 = trig[i];
            let t1 = trig[n4 + i];
            let yr = x2 * t0 + x1 * t1;
            let yi = x1 * t0 - x2 * t1;
            f2[rev] = KissCpx::new(yi, yr);
        }
        crate::celt::kiss_fft::opus_fft_impl(st, &mut f2);

        let mut output_scalar = vec![0.0f32; out_len + 100];
        for i in 0..((n4 + 1) >> 1) {
            let im0 = f2[i].r;
            let re0 = f2[i].i;
            let t0_0 = trig[i];
            let t1_0 = trig[n4 + i];
            let yr0 = re0 * t0_0 + im0 * t1_0;
            let yi0 = re0 * t1_0 - im0 * t0_0;
            let j = n4 - 1 - i;
            let im1 = f2[j].r;
            let re1 = f2[j].i;
            let t0_1 = trig[j];
            let t1_1 = trig[n4 + j];
            let yr1 = re1 * t0_1 + im1 * t1_1;
            let yi1 = re1 * t1_1 - im1 * t0_1;
            output_scalar[overlap2 + 2 * i] = yr0;
            output_scalar[overlap2 + n2 - 1 - 2 * i] = yi0;
            output_scalar[overlap2 + n2 - 2 - 2 * i] = yr1;
            output_scalar[overlap2 + 2 * i + 1] = yi1;
        }
        // TDAC
        for i in 0..overlap2 {
            let x1 = output_scalar[overlap - 1 - i];
            let x2 = output_scalar[i];
            let wp1 = mode.window[i];
            let wp2 = mode.window[overlap - 1 - i];
            output_scalar[i] = x2 * wp2 - x1 * wp1;
            output_scalar[overlap - 1 - i] = x2 * wp1 + x1 * wp2;
        }

        let max_diff = output_hw[..out_len]
            .iter()
            .zip(output_scalar[..out_len].iter())
            .map(|(a, b)| (a - b).abs())
            .fold(0.0f32, f32::max);
        assert!(
            max_diff < 0.5,
            "stride=1 NEON vs scalar mismatch: max_diff={}",
            max_diff
        );
    }

    #[test]
    fn test_mdct_backward_neon_matches_scalar() {
        let mode = crate::celt::modes::default_mode();
        let shift = 3;
        let n = mode.mdct.n >> shift; // 240
        let n2 = n / 2; // 120
        let n4 = n / 4; // 60
        let overlap = mode.overlap; // 120
        let overlap2 = overlap / 2; // 60
        let stride = 8;

        // Build a realistic freq vector (sine wave @ 440Hz)
        let frame_size = 960usize;
        let mut freq = vec![0.0f32; frame_size];
        for i in 0..frame_size {
            freq[i] = ((i as f32) * 0.01).sin() * 200.0;
        }

        let out_len = n + overlap; // 360
        let mut output_hw = vec![0.0f32; out_len];
        mode.mdct.backward(
            &freq[0..],
            &mut output_hw,
            mode.window,
            overlap,
            shift,
            stride,
        );

        // Scalar reference
        let st = mode.mdct.kfft[shift].as_ref().unwrap();
        let (trig, _) = mode.mdct.get_trig(shift);

        use crate::celt::kiss_fft::KissCpx;
        let mut f2 = vec![KissCpx::new(0.0, 0.0); n4];
        for i in 0..n4 {
            let rev = st.bitrev[i] as usize;
            let x1 = freq[2 * i * stride];
            let x2 = freq[stride * (n2 - 1 - 2 * i)];
            let t0 = trig[i];
            let t1 = trig[n4 + i];
            let yr = x2 * t0 + x1 * t1;
            let yi = x1 * t0 - x2 * t1;
            f2[rev] = KissCpx::new(yi, yr);
        }
        crate::celt::kiss_fft::opus_fft_impl(st, &mut f2);

        let mut output_scalar = vec![0.0f32; out_len];
        for i in 0..((n4 + 1) >> 1) {
            let im0 = f2[i].r;
            let re0 = f2[i].i;
            let t0_0 = trig[i];
            let t1_0 = trig[n4 + i];
            let yr0 = re0 * t0_0 + im0 * t1_0;
            let yi0 = re0 * t1_0 - im0 * t0_0;
            let j = n4 - 1 - i;
            let im1 = f2[j].r;
            let re1 = f2[j].i;
            let t0_1 = trig[j];
            let t1_1 = trig[n4 + j];
            let yr1 = re1 * t0_1 + im1 * t1_1;
            let yi1 = re1 * t1_1 - im1 * t0_1;
            output_scalar[overlap2 + 2 * i] = yr0;
            output_scalar[overlap2 + n2 - 1 - 2 * i] = yi0;
            output_scalar[overlap2 + n2 - 2 - 2 * i] = yr1;
            output_scalar[overlap2 + 2 * i + 1] = yi1;
        }
        // TDAC
        for i in 0..overlap2 {
            let x1 = output_scalar[overlap - 1 - i];
            let x2 = output_scalar[i];
            let wp1 = mode.window[i];
            let wp2 = mode.window[overlap - 1 - i];
            output_scalar[i] = x2 * wp2 - x1 * wp1;
            output_scalar[overlap - 1 - i] = x2 * wp1 + x1 * wp2;
        }

        for i in 0..out_len {
            let diff = (output_hw[i] - output_scalar[i]).abs();
            if diff > 1e-3 {
                eprintln!(
                    "Mismatch at output[{}]: hw={} scalar={} diff={}",
                    i, output_hw[i], output_scalar[i], diff
                );
            }
        }
        let max_diff = output_hw
            .iter()
            .zip(output_scalar.iter())
            .map(|(a, b)| (a - b).abs())
            .fold(0.0f32, f32::max);
        assert!(
            max_diff < 0.1,
            "NEON/HW vs scalar mismatch: max_diff={}",
            max_diff
        );
    }

    // ---- SIMD rotation kernels vs their scalar definitions ----------------
    //
    // Each rotation has a scalar form written inline in `forward`/`backward`
    // for targets with no kernel. These tests transcribe that form and hold
    // the host's kernel to it, so a NEON build checks NEON and an x86 build
    // checks AVX.

    fn noise(n: usize, seed: u32) -> Vec<f32> {
        let mut s = seed | 1;
        (0..n)
            .map(|_| {
                s = s.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                (s >> 8) as f32 / (1 << 23) as f32 - 1.0
            })
            .collect()
    }

    /// A bit-reversal permutation of 0..n4, as `KissFftState::bitrev` supplies.
    fn bitrev_of(n4: usize) -> Vec<i16> {
        let mut v: Vec<i16> = (0..n4 as i16).collect();
        // Any permutation exercises the scatter; a rotate is enough and keeps
        // the expected output easy to state.
        v.rotate_left(n4 / 3 + 1);
        v
    }

    fn close(got: &[f32], want: &[f32], what: &str) {
        let scale = want.iter().fold(1.0f32, |m, v| m.max(v.abs()));
        for (i, (g, w)) in got.iter().zip(want).enumerate() {
            assert!(
                (g - w).abs() <= 1e-5 * scale,
                "{what}: element {i}: {g} vs {w}"
            );
        }
    }

    #[test]
    fn simd_mdct_rotations_match_their_scalar_definitions() {
        for &n4 in &[4usize, 8, 15, 16, 30, 60, 120, 240] {
            let n2 = 2 * n4;
            let f = noise(2 * n4, 0x6001 ^ n4 as u32);
            let trig = noise(2 * n4, 0x6002 ^ n4 as u32);
            let bitrev = bitrev_of(n4);
            let scale = 0.375f32;

            // --- forward pre-rotation ---
            {
                let mut want = vec![KissCpx::new(0.0, 0.0); n4];
                for i in 0..n4 {
                    let (re, im) = (f[2 * i], f[2 * i + 1]);
                    let (t0, t1) = (trig[i], trig[n4 + i]);
                    want[bitrev[i] as usize] =
                        KissCpx::new((re * t0 - im * t1) * scale, (im * t0 + re * t1) * scale);
                }
                let mut got = vec![KissCpx::new(0.0, 0.0); n4];
                fvid_cpu::opus_simd::mdct_pre_rotation(&f, &mut got, &trig, &bitrev, n4, scale);
                let flat_g: Vec<f32> = got.iter().flat_map(|c| [c.r, c.i]).collect();
                let flat_w: Vec<f32> = want.iter().flat_map(|c| [c.r, c.i]).collect();
                close(&flat_g, &flat_w, &format!("mdct pre-rotation n4={n4}"));
            }

            // --- forward post-rotation ---
            for &stride in &[1usize, 2] {
                let f2: Vec<KissCpx> = (0..n4)
                    .map(|i| KissCpx::new(f[2 * i], f[2 * i + 1]))
                    .collect();
                let out_len = stride * n2;
                let mut want = vec![0.0f32; out_len];
                for i in 0..n4 {
                    let fp = &f2[i];
                    let (t0, t1) = (trig[i], trig[n4 + i]);
                    want[i * 2 * stride] = fp.i * t1 - fp.r * t0;
                    want[stride * (n2 - 1 - 2 * i)] = fp.r * t1 + fp.i * t0;
                }
                let mut got = vec![0.0f32; out_len];
                fvid_cpu::opus_simd::mdct_post_rotation(&f2, &trig, &mut got, n4, n2, stride);
                close(
                    &got,
                    &want,
                    &format!("mdct post-rotation n4={n4} stride={stride}"),
                );
            }

            // --- backward pre-rotation ---
            for &stride in &[1usize, 2] {
                let input = noise(stride * n2, 0x6003 ^ n4 as u32);
                let mut want = vec![KissCpx::new(0.0, 0.0); n4];
                for i in 0..n4 {
                    let rev = bitrev[i] as usize;
                    let x1 = input[2 * i * stride];
                    let x2 = input[stride * (n2 - 1 - 2 * i)];
                    let (t0, t1) = (trig[i], trig[n4 + i]);
                    want[rev] = KissCpx::new(x1 * t0 - x2 * t1, x2 * t0 + x1 * t1);
                }
                let mut got = vec![KissCpx::new(0.0, 0.0); n4];
                fvid_cpu::opus_simd::mdct_backward_pre_rotation(
                    &input, &mut got, &trig, &bitrev, n4, n2, stride,
                );
                let flat_g: Vec<f32> = got.iter().flat_map(|c| [c.r, c.i]).collect();
                let flat_w: Vec<f32> = want.iter().flat_map(|c| [c.r, c.i]).collect();
                close(
                    &flat_g,
                    &flat_w,
                    &format!("mdct backward pre-rotation n4={n4} stride={stride}"),
                );
            }

            // --- backward post-rotation ---
            {
                let overlap2 = 4usize;
                let f2: Vec<KissCpx> = (0..n4)
                    .map(|i| KissCpx::new(f[2 * i], f[2 * i + 1]))
                    .collect();
                let mut want = vec![0.0f32; overlap2 + n2];
                for i in 0..((n4 + 1) >> 1) {
                    let (im0, re0) = (f2[i].r, f2[i].i);
                    let (t0_0, t1_0) = (trig[i], trig[n4 + i]);
                    let j = n4 - 1 - i;
                    let (im1, re1) = (f2[j].r, f2[j].i);
                    let (t0_1, t1_1) = (trig[j], trig[n4 + j]);
                    want[overlap2 + 2 * i] = re0 * t0_0 + im0 * t1_0;
                    want[overlap2 + n2 - 1 - 2 * i] = re0 * t1_0 - im0 * t0_0;
                    want[overlap2 + n2 - 2 - 2 * i] = re1 * t0_1 + im1 * t1_1;
                    want[overlap2 + 2 * i + 1] = re1 * t1_1 - im1 * t0_1;
                }
                let mut got = vec![0.0f32; overlap2 + n2];
                mdct_backward_post_rotation_scalar(&f2, &trig, &mut got, n4, n2, overlap2);
                close(&got, &want, &format!("mdct backward post-rotation n4={n4}"));
            }
        }
    }

    #[test]
    fn simd_mdct_tdac_matches_the_scalar_definition() {
        for &overlap in &[2usize, 4, 8, 15, 16, 30, 120] {
            let overlap2 = overlap / 2;
            let src = noise(overlap + 8, 0x7001 ^ overlap as u32);
            let window = noise(overlap, 0x7002 ^ overlap as u32);

            let mut want = src.clone();
            for i in 0..overlap2 {
                let x1 = want[overlap - 1 - i];
                let x2 = want[i];
                let wp1 = window[i];
                let wp2 = window[overlap - 1 - i];
                want[i] = x2 * wp2 - x1 * wp1;
                want[overlap - 1 - i] = x2 * wp1 + x1 * wp2;
            }

            let mut got = src.clone();
            fvid_cpu::opus_simd::mdct_tdac(&mut got, &window, overlap);
            close(&got, &want, &format!("mdct tdac overlap={overlap}"));
        }
    }
}
