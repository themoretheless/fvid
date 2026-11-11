use crate::celt::lpc::{autocorr, lpc};

#[inline]
pub fn inner_prod(x: &[f32], y: &[f32], n: usize) -> f32 {
    fvid_cpu::opus_simd::inner_prod(x, y, n)
}

#[inline]
pub fn dual_inner_prod(x: &[f32], y1: &[f32], y2: &[f32], n: usize) -> (f32, f32) {
    fvid_cpu::opus_simd::dual_inner_prod(x, y1, y2, n)
}

#[inline]
pub fn pitch_xcorr(x: &[f32], y: &[f32], xcorr: &mut [f32], len: usize, max_pitch: usize) {
    fvid_cpu::opus_simd::pitch_xcorr(x, y, xcorr, len, max_pitch);
}

/// libopus `celt_fir5`: the fixed order-5 FIR the pitch downsampler whitens
/// with. Scalar on every target — five taps do not repay vectorising.
fn celt_fir5(x: &mut [f32], num: &[f32], n: usize) {
    let mut mem = [0.0f32; 5];

    let num0 = num[0];
    let num1 = num[1];
    let num2 = num[2];
    let num3 = num[3];
    let num4 = num[4];

    for i in 0..n {
        let mut sum = x[i];
        sum += num0 * mem[0];
        sum += num1 * mem[1];
        sum += num2 * mem[2];
        sum += num3 * mem[3];
        sum += num4 * mem[4];

        mem[4] = mem[3];
        mem[3] = mem[2];
        mem[2] = mem[1];
        mem[1] = mem[0];
        mem[0] = x[i];

        x[i] = sum;
    }
}

pub fn pitch_downsample(x: &[&[f32]], x_lp: &mut [f32], len: usize, c: usize, factor: usize) {
    let offset = factor / 2;

    if x_lp.len() < len {
        return;
    }

    // Stage 1: the three-tap decimating filter. Only this part is vectorised.
    let taps_done = factor == 2
        && c <= 2
        && fvid_cpu::opus_simd::pitch_downsample_taps_factor2(x, x_lp, len, c);

    if !taps_done {
        for i in 1..len {
            let mut val = 0.0f32;
            for k in 0..c {
                let x_k = x[k];

                let idx_m = factor * i - offset;
                let idx_p = factor * i + offset;
                let idx_c = factor * i;

                if idx_p < x_k.len() {
                    val += 0.25 * x_k[idx_m] + 0.25 * x_k[idx_p] + 0.5 * x_k[idx_c];
                }
            }
            x_lp[i] = val;
        }
    }

    pitch_downsample_boundary(x, x_lp, c, offset);

    // Stage 2: the whitening libopus applies to the decimated buffer. It is
    // not an optimisation and it is not optional — the pitch search that reads
    // this buffer is tuned for a whitened signal.
    pitch_downsample_whiten(x_lp, len);
}

/// Order-4 LPC whitening with an added zero, applied in place to the decimated
/// buffer (libopus `pitch_downsample`, everything after the three-tap filter).
fn pitch_downsample_whiten(x_lp: &mut [f32], len: usize) {
    let mut ac = [0.0f32; 5];
    autocorr(&x_lp[0..len], &mut ac, None, 0, 4, len);

    ac[0] *= 1.0001;

    for i in 1..=4 {
        let f = 0.008 * (i as f32);
        ac[i] -= ac[i] * f * f;
    }

    let mut lpc_coeffs = [0.0f32; 4];
    lpc(&mut lpc_coeffs, &ac, 4);

    let mut tmp = 1.0f32;
    for coeff in lpc_coeffs.iter_mut() {
        tmp *= 0.9;
        *coeff *= tmp;
    }

    let c1 = 0.8f32;
    let lpc2 = [
        lpc_coeffs[0] + c1,
        lpc_coeffs[1] + c1 * lpc_coeffs[0],
        lpc_coeffs[2] + c1 * lpc_coeffs[1],
        lpc_coeffs[3] + c1 * lpc_coeffs[2],
        c1 * lpc_coeffs[3],
    ];

    celt_fir5(x_lp, &lpc2, len);
}

#[inline]
fn pitch_downsample_boundary(x: &[&[f32]], x_lp: &mut [f32], c: usize, offset: usize) {
    let mut val = 0.0f32;
    for k in 0..c {
        let x_k = x[k];

        let idx_offset = offset;
        let idx_0 = 0;
        if idx_offset < x_k.len() {
            val += 0.25 * x_k[idx_offset] + 0.5 * x_k[idx_0];
        }
    }
    x_lp[0] = val;
}

#[inline(always)]
fn find_best_pitch(
    xcorr: &[f32],
    y: &[f32],
    len: usize,
    max_pitch: usize,
    best_pitch: &mut [usize; 2],
) {
    let mut best_num = [-1.0f32, -1.0f32];
    let mut best_den = [0.0f32, 0.0f32];

    best_pitch[0] = 0;
    best_pitch[1] = 1;

    let mut syy = fvid_cpu::opus_simd::sum_squares_plus_one(y, len);

    for i in 0..max_pitch {
        if xcorr[i] > 0.0 {
            let num = xcorr[i] * xcorr[i];
            if num * best_den[1] > best_num[1] * syy {
                if num * best_den[0] > best_num[0] * syy {
                    best_num[1] = best_num[0];
                    best_den[1] = best_den[0];
                    best_pitch[1] = best_pitch[0];
                    best_num[0] = num;
                    best_den[0] = syy;
                    best_pitch[0] = i;
                } else {
                    best_num[1] = num;
                    best_den[1] = syy;
                    best_pitch[1] = i;
                }
            }
        }
        syy += y[i + len] * y[i + len] - y[i] * y[i];
        if syy < 1.0 {
            syy = 1.0;
        }
    }
}

pub fn pitch_search(x_lp: &[f32], y: &[f32], mut len: usize, mut max_pitch: usize) -> usize {
    max_pitch >>= 1;
    len >>= 1;
    let lag = len + max_pitch;
    let len4 = len >> 1;
    let lag4 = lag >> 1;

    const MAX_LEN4: usize = 512;
    const MAX_LAG4: usize = 1024;
    const MAX_PITCH: usize = 512;

    // Same search either way; the only difference is where the three scratch
    // buffers live. Every size CELT actually asks for fits the stack bounds.
    if len4 <= MAX_LEN4 && lag4 <= MAX_LAG4 && max_pitch <= MAX_PITCH {
        let mut x_lp4 = [0.0f32; MAX_LEN4];
        let mut y_lp4 = [0.0f32; MAX_LAG4];
        let mut xcorr = [0.0f32; MAX_PITCH];
        pitch_search_impl(
            x_lp,
            y,
            len,
            max_pitch,
            &mut x_lp4[..len4],
            &mut y_lp4[..lag4],
            &mut xcorr[..max_pitch],
        )
    } else {
        let mut x_lp4 = vec![0.0f32; len4];
        let mut y_lp4 = vec![0.0f32; lag4];
        let mut xcorr = vec![0.0f32; max_pitch];
        pitch_search_impl(x_lp, y, len, max_pitch, &mut x_lp4, &mut y_lp4, &mut xcorr)
    }
}

/// The pitch search proper. `len` and `max_pitch` arrive already halved, and
/// the three scratch slices are sized `len >> 1`, `(len + max_pitch) >> 1` and
/// `max_pitch` respectively.
fn pitch_search_impl(
    x_lp: &[f32],
    y: &[f32],
    len: usize,
    max_pitch: usize,
    x_lp4: &mut [f32],
    y_lp4: &mut [f32],
    xcorr: &mut [f32],
) -> usize {
    let mut best_pitch = [0, 0];

    for j in 0..x_lp4.len() {
        x_lp4[j] = x_lp[2 * j];
    }
    for j in 0..y_lp4.len() {
        y_lp4[j] = y[2 * j];
    }

    pitch_xcorr(x_lp4, y_lp4, xcorr, len >> 1, max_pitch >> 1);

    find_best_pitch(xcorr, y_lp4, len >> 1, max_pitch >> 1, &mut best_pitch);

    for i in 0..max_pitch {
        // Lags the coarse pass ruled out stay at 0, not -1. `find_best_pitch`
        // ignores both (it only considers a positive correlation), but the
        // pseudo-interpolation below reads the winner's two neighbours
        // directly, and a skipped neighbour of -1 instead of 0 tips
        // `(c - a) > 0.7 * (b - a)` the other way — an off-by-one in the
        // reported lag (libopus pitch.c pitch_search).
        xcorr[i] = 0.0;
        if (i as i32 - 2 * best_pitch[0] as i32).abs() > 2
            && (i as i32 - 2 * best_pitch[1] as i32).abs() > 2
        {
            continue;
        }
        xcorr[i] = inner_prod(x_lp, &y[i..], len);
        if xcorr[i] < -1.0 {
            xcorr[i] = -1.0;
        }
    }

    find_best_pitch(xcorr, y, len, max_pitch, &mut best_pitch);

    let mut offset = 0;
    if best_pitch[0] > 0 && best_pitch[0] < max_pitch - 1 {
        let a = xcorr[best_pitch[0] - 1];
        let b = xcorr[best_pitch[0]];
        let c = xcorr[best_pitch[0] + 1];
        if (c - a) > 0.7 * (b - a) {
            offset = 1;
        } else if (a - c) > 0.7 * (b - c) {
            offset = -1;
        }
    }

    ((2 * best_pitch[0]) as isize).wrapping_sub(offset as isize) as usize
}

fn compute_pitch_gain(xy: f32, xx: f32, yy: f32) -> f32 {
    if xy <= 0.0 || xx <= 0.0 || yy <= 0.0 {
        return 0.0;
    }
    xy / (1.0 + xx * yy).sqrt()
}

static SECOND_CHECK: [usize; 16] = [0, 0, 3, 2, 3, 2, 5, 2, 3, 2, 3, 2, 5, 2, 3, 2];

#[inline(always)]
fn sum_squares(x: &[f32], n: usize) -> f32 {
    fvid_cpu::opus_simd::inner_prod(x, x, n)
}

pub fn remove_doubling(
    x: &[f32],
    mut max_period: usize,
    mut min_period: usize,
    mut n: usize,
    t0_ptr: &mut usize,
    mut prev_period: usize,
    prev_gain: f32,
) -> f32 {
    let min_period0 = min_period;
    max_period /= 2;
    min_period /= 2;
    *t0_ptr /= 2;
    prev_period /= 2;
    n /= 2;

    let x_target = &x[max_period..];

    if *t0_ptr >= max_period {
        *t0_ptr = max_period - 1;
    }

    let mut t = *t0_ptr;
    let t0 = *t0_ptr;

    const MAX_YY_SIZE: usize = 1024;
    let mut yy_lookup_buf = [0.0f32; MAX_YY_SIZE];
    let yy_lookup = &mut yy_lookup_buf[..=max_period];

    let xx = sum_squares(x_target, n);
    let xy = inner_prod(x_target, &x[max_period - t0..], n);

    yy_lookup[0] = xx;
    let mut yy_curr = xx;
    for i in 1..=max_period {
        yy_curr = yy_curr + x[max_period - i] * x[max_period - i]
            - x[max_period + n - i] * x[max_period + n - i];
        if yy_curr < 0.0 {
            yy_curr = 0.0;
        }
        yy_lookup[i] = yy_curr;
    }

    let mut best_xy = xy;
    let mut best_yy = yy_lookup[t0];
    let mut g = compute_pitch_gain(best_xy, xx, best_yy);
    let g0 = g;

    for k in 2..=15 {
        let t1 = (2 * t0 + k) / (2 * k);
        if t1 < min_period {
            break;
        }

        let t1b;
        if k == 2 {
            if t1 + t0 > max_period {
                t1b = t0;
            } else {
                t1b = t0 + t1;
            }
        } else {
            t1b = (2 * SECOND_CHECK[k] * t0 + k) / (2 * k);
        }

        let (xy_a, xy_b) =
            dual_inner_prod(x_target, &x[max_period - t1..], &x[max_period - t1b..], n);
        let xy_new = 0.5 * (xy_a + xy_b);
        let yy_new = 0.5 * (yy_lookup[t1] + yy_lookup[t1b]);
        let g1 = compute_pitch_gain(xy_new, xx, yy_new);

        let mut cont = 0.0f32;
        if (t1 as i32 - prev_period as i32).abs() <= 1 {
            cont = prev_gain;
        } else if (t1 as i32 - prev_period as i32).abs() <= 2 && 5 * k * k < t0 {
            cont = 0.5 * prev_gain;
        }

        let mut thresh = (0.7 * g0 - cont).max(0.3);
        if t1 < 3 * min_period {
            thresh = (0.85 * g0 - cont).max(0.4);
        } else if t1 < 2 * min_period {
            thresh = (0.9 * g0 - cont).max(0.5);
        }

        if g1 > thresh {
            best_xy = xy_new;
            best_yy = yy_new;
            t = t1;
            g = g1;
        }
    }

    best_xy = best_xy.max(0.0);
    let pg = if best_yy <= best_xy {
        1.0f32
    } else {
        best_xy / (best_yy + 1.0)
    };

    let mut xcorr_res = [0.0f32; 3];
    for k_idx in 0..3 {
        let lag = (t as i32 + k_idx as i32 - 1) as usize;
        xcorr_res[k_idx] = inner_prod(x_target, &x[max_period - lag..], n);
    }

    let mut offset = 0;
    if (xcorr_res[2] - xcorr_res[0]) > 0.7 * (xcorr_res[1] - xcorr_res[0]) {
        offset = 1;
    } else if (xcorr_res[0] - xcorr_res[2]) > 0.7 * (xcorr_res[1] - xcorr_res[2]) {
        offset = -1;
    }

    let pg = pg.min(g);
    *t0_ptr = (2 * t as i32 + offset) as usize;
    if *t0_ptr < min_period0 {
        *t0_ptr = min_period0;
    }

    pg
}

#[cfg(test)]
mod tests {
    use super::*;

    fn noise(n: usize, seed: u32) -> Vec<f32> {
        let mut s = seed | 1;
        (0..n)
            .map(|_| {
                s = s.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                (s >> 8) as f32 / (1 << 23) as f32 - 1.0
            })
            .collect()
    }

    /// The three-tap decimating filter at the head of libopus `pitch_downsample`,
    /// before the LPC whitening it shares with the real function.
    fn downsample_taps_reference(x: &[&[f32]], len: usize, c: usize, factor: usize) -> Vec<f32> {
        let offset = factor / 2;
        let mut out = vec![0.0f32; len];
        for xk in x.iter().take(c) {
            out[0] += 0.25 * xk[offset] + 0.5 * xk[0];
            for (i, o) in out.iter_mut().enumerate().skip(1).take(len - 1) {
                *o += 0.25 * xk[factor * i - offset]
                    + 0.25 * xk[factor * i + offset]
                    + 0.5 * xk[factor * i];
            }
        }
        out
    }

    /// `pitch_downsample` must decimate, not merely low-pass.
    ///
    /// The aarch64 path used one `vld1q` per tap and stored four outputs per
    /// iteration, which walks the input one sample per output instead of
    /// `factor`. Only the first lane of each group of four landed on the right
    /// input sample, so three quarters of the pitch buffer was wrong.
    /// Downstream that moved the pitch estimate itself: in the CELT
    /// concealment it shifted the extrapolated period by whole samples, and
    /// the encoder's prefilter analysis reads the same buffer.
    #[test]
    fn downsample_simd_matches_the_scalar_path() {
        for &len in &[8usize, 33, 100, 512, 1024] {
            let a = noise(2 * len, 0x5EED_0001 ^ len as u32);
            let b = noise(2 * len, 0x5EED_0002 ^ len as u32);

            for &c in &[1usize, 2] {
                let chans: Vec<&[f32]> = if c == 1 {
                    vec![&a[..]]
                } else {
                    vec![&a[..], &b[..]]
                };
                let mut got = vec![0.0f32; len];
                pitch_downsample(&chans, &mut got, len, c, 2);

                // Reference: the same taps, then the same whitening.
                let mut want = downsample_taps_reference(&chans, len, c, 2);
                let mut ac = [0.0f32; 5];
                autocorr(&want[0..len], &mut ac, None, 0, 4, len);
                ac[0] *= 1.0001;
                for i in 1..=4 {
                    let f = 0.008 * (i as f32);
                    ac[i] -= ac[i] * f * f;
                }
                let mut lc = [0.0f32; 4];
                lpc(&mut lc, &ac, 4);
                let mut tmp = 1.0f32;
                for v in lc.iter_mut() {
                    tmp *= 0.9;
                    *v *= tmp;
                }
                let c1 = 0.8f32;
                let lpc2 = [
                    lc[0] + c1,
                    lc[1] + c1 * lc[0],
                    lc[2] + c1 * lc[1],
                    lc[3] + c1 * lc[2],
                    c1 * lc[3],
                ];
                let mut mem = [0.0f32; 5];
                for i in 0..len {
                    let xi = want[i];
                    let mut sum = xi;
                    for (k, m) in mem.iter().enumerate() {
                        sum += lpc2[k] * m;
                    }
                    for k in (1..5).rev() {
                        mem[k] = mem[k - 1];
                    }
                    mem[0] = xi;
                    want[i] = sum;
                }

                let scale = want.iter().fold(1.0f32, |m, v| m.max(v.abs()));
                for (i, (g, w)) in got.iter().zip(&want).enumerate() {
                    assert!(
                        (g - w).abs() <= 1e-4 * scale,
                        "pitch_downsample len={len} c={c} sample {i}: {g} vs {w}"
                    );
                }
            }
        }
    }

    /// `inner_prod`, `dual_inner_prod` and `pitch_xcorr` all have SIMD paths;
    /// each must match the plain loop it stands in for.
    #[test]
    fn simd_correlations_match_the_scalar_definition() {
        for &n in &[1usize, 4, 7, 16, 63, 256, 664] {
            let x = noise(n, 0xABCD_0001 ^ n as u32);
            let y1 = noise(n, 0xABCD_0002 ^ n as u32);
            let y2 = noise(n, 0xABCD_0003 ^ n as u32);
            let want: f32 = (0..n).map(|i| x[i] * y1[i]).sum();
            let got = inner_prod(&x, &y1, n);
            assert!(
                (got - want).abs() <= 1e-4 * (1.0 + want.abs()),
                "inner_prod n={n}: {got} vs {want}"
            );

            let want2: f32 = (0..n).map(|i| x[i] * y2[i]).sum();
            let (g1, g2) = dual_inner_prod(&x, &y1, &y2, n);
            assert!(
                (g1 - want).abs() <= 1e-4 * (1.0 + want.abs())
                    && (g2 - want2).abs() <= 1e-4 * (1.0 + want2.abs()),
                "dual_inner_prod n={n}"
            );
        }

        for &(len, max_pitch) in &[(64usize, 8usize), (128, 33), (332, 155), (664, 310)] {
            let x = noise(len, 0x7777_0001 ^ len as u32);
            let y = noise(len + max_pitch + 4, 0x7777_0002 ^ len as u32);
            let mut got = vec![0.0f32; max_pitch];
            pitch_xcorr(&x, &y, &mut got, len, max_pitch);
            for i in 0..max_pitch {
                let want: f32 = (0..len).map(|j| x[j] * y[i + j]).sum();
                assert!(
                    (got[i] - want).abs() <= 1e-3 * (1.0 + want.abs()),
                    "pitch_xcorr len={len} lag {i}: {} vs {want}",
                    got[i]
                );
            }
        }
    }
}
