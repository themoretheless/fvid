use crate::silk::define::*;
use crate::silk::macros::*;

#[inline]
pub fn silk_k2a_q16(a_q24: &mut [i32], rc_q16: &[i32], order: usize) {
    for k in 0..order {
        let rc = rc_q16[k];
        for n in 0..((k + 1) >> 1) {
            let tmp1 = a_q24[n];
            let tmp2 = a_q24[k - n - 1];
            a_q24[n] = silk_smlaww(tmp1, tmp2, rc);
            a_q24[k - n - 1] = silk_smlaww(tmp2, tmp1, rc);
        }
        a_q24[k] = -(rc << 8);
    }
}

pub fn silk_schur64(rc_q16: &mut [i32], c: &[i32], order: usize) -> i32 {
    let mut c_matrix = [[0i32; 2]; MAX_SHAPE_LPC_ORDER + 1];
    let mut c_tmp1_q30: i32;
    let mut c_tmp2_q30: i32;
    let mut rc_tmp_q31: i32;

    if c[0] <= 0 {
        for v in rc_q16.iter_mut().take(order) {
            *v = 0;
        }
        return 0;
    }

    for k in 0..=order {
        c_matrix[k][0] = c[k];
        c_matrix[k][1] = c[k];
    }

    let mut k = 0;
    while k < order {
        if c_matrix[k + 1][0].wrapping_abs() >= c_matrix[0][1] {
            if c_matrix[k + 1][0] > 0 {
                rc_q16[k] = -64880;
            } else {
                rc_q16[k] = 64880;
            }
            k += 1;
            break;
        }

        rc_tmp_q31 = silk_div32_varq(-c_matrix[k + 1][0], c_matrix[0][1], 31);

        rc_q16[k] = silk_rshift_round(rc_tmp_q31, 15);

        for n in 0..(order - k) {
            c_tmp1_q30 = c_matrix[n + k + 1][0];
            c_tmp2_q30 = c_matrix[n][1];

            c_matrix[n + k + 1][0] =
                c_tmp1_q30.wrapping_add(silk_smmul(c_tmp2_q30 << 1, rc_tmp_q31));
            c_matrix[n][1] = c_tmp2_q30.wrapping_add(silk_smmul(c_tmp1_q30 << 1, rc_tmp_q31));
        }
        k += 1;
    }

    while k < order {
        rc_q16[k] = 0;
        k += 1;
    }

    c_matrix[0][1].max(1)
}

#[inline(always)]
fn mac16_16(sum: i32, a: i16, b: i16) -> i32 {
    sum.wrapping_add((a as i32) * (b as i32))
}

#[inline]
fn xcorr_kernel_c(x: &[i16], y: &[i16], sum: &mut [i32; 4], len: usize) {
    fvid_cpu::opus_simd::silk_xcorr_kernel(x, y, sum, len);
}

pub fn silk_autocorr(
    results: &mut [i32],
    scale: &mut i32,
    input_data: &[i16],
    input_data_size: usize,
    correlation_count: usize,
) {
    #[inline]
    fn ec_ilog(x: u32) -> i32 {
        if x == 0 {
            0
        } else {
            32 - x.leading_zeros() as i32
        }
    }

    let n = input_data_size;
    let mut shift: i32;

    let xptr = input_data;

    let ac0_shift = ec_ilog((n + (n >> 4)) as u32) - 1;

    let mut ac0: i32 = 1 + ((n as i32) << 7);
    let mut i = n & 1;
    if n & 1 != 0 {
        ac0 += ((xptr[0] as i32) * (xptr[0] as i32)) >> ac0_shift;
    }
    while i < n {
        ac0 += ((xptr[i] as i32) * (xptr[i] as i32)) >> ac0_shift;
        ac0 += ((xptr[i + 1] as i32) * (xptr[i + 1] as i32)) >> ac0_shift;
        i += 2;
    }

    ac0 += ac0 >> 7;

    let ac0_log2 = ec_ilog(ac0 as u32) - 1;
    shift = ac0_log2 - 30 + ac0_shift + 1;
    shift /= 2;

    let mut xx_buf = [0i16; PE_MAX_FRAME_LENGTH];
    let xptr: &[i16];

    if shift > 0 {
        for j in 0..n {
            xx_buf[j] = silk_rshift_round(input_data[j] as i32, shift) as i16;
        }
        xptr = &xx_buf[..n];
    } else {
        shift = 0;
        xptr = input_data;
    }

    let lag = correlation_count - 1;
    let fast_n = n - lag;
    let max_pitch = lag + 1;

    let mut lag_idx = 0;
    while lag_idx + 3 < max_pitch {
        let mut sum = [0i32; 4];
        xcorr_kernel_c(xptr, &xptr[lag_idx..], &mut sum, fast_n);
        results[lag_idx] = sum[0];
        results[lag_idx + 1] = sum[1];
        results[lag_idx + 2] = sum[2];
        results[lag_idx + 3] = sum[3];
        lag_idx += 4;
    }

    while lag_idx < max_pitch {
        let mut sum = 0i32;
        for j in 0..fast_n {
            sum = sum.wrapping_add((xptr[j] as i32).wrapping_mul(xptr[j + lag_idx] as i32));
        }
        results[lag_idx] = sum;
        lag_idx += 1;
    }

    for k in 0..correlation_count {
        let mut d: i32 = 0;
        for i in (k + fast_n)..n {
            d = d.wrapping_add((xptr[i] as i32).wrapping_mul(xptr[i - k] as i32));
        }
        results[k] = results[k].wrapping_add(d);
    }

    shift *= 2;
    if shift <= 0 {
        let add_shift = (-shift).min(30);
        results[0] += 1i32 << add_shift;
    }
    if results[0] > 0 && results[0] < 268435456 {
        let shift2 = 29 - ec_ilog(results[0] as u32);
        for v in results[..correlation_count].iter_mut() {
            *v <<= shift2;
        }
        shift -= shift2;
    } else if results[0] >= 536870912 {
        let mut shift2 = 1;
        if results[0] >= 1073741824 {
            shift2 += 1;
        }
        for v in results[..correlation_count].iter_mut() {
            *v >>= shift2;
        }
        shift += shift2;
    }

    *scale = shift;
}

pub fn silk_sum_sqr_shift(energy: &mut i32, shift: &mut i32, x: &[i16], len: usize) {
    let shft = 31 - silk_clz32(len as i32);
    let nrg0 = (len as i32).wrapping_add(fvid_cpu::opus_simd::silk_sum_sqr_shift_loop(x, len, shft));
    let shft = (shft + 3 - silk_clz32(nrg0)).max(0);
    *shift = shft;
    *energy = fvid_cpu::opus_simd::silk_sum_sqr_shift_loop(x, len, shft);
}

/// Inner product with each term right-shifted before accumulation, so a long
/// or loud pair of vectors cannot overflow the accumulator (libopus
/// `silk_inner_prod_aligned_scale`). Shifting per term rather than once at the
/// end is what the reference does and what the callers' Q formats assume, so
/// this stays scalar: a vectorised version would have to reproduce the same
/// per-term truncation to be worth anything.
pub fn silk_inner_prod_aligned_scale(ptr1: &[i16], ptr2: &[i16], scale: i32, len: usize) -> i32 {
    let mut sum = 0i32;
    for i in 0..len {
        sum = silk_add_rshift32(sum, silk_smulbb(ptr1[i] as i32, ptr2[i] as i32), scale);
    }
    sum
}

#[inline(always)]
pub fn silk_inner_prod_aligned(ptr1: &[i16], ptr2: &[i16], len: usize) -> i32 {
    fvid_cpu::opus_simd::silk_inner_prod_aligned(ptr1, ptr2, len)
}

pub fn silk_corr_vector_fix(
    x: &[i16],
    t: &[i16],
    l: usize,
    order: usize,
    xt: &mut [i32],
    rshifts: i32,
) {
    let mut ptr1_idx = order - 1;
    if rshifts > 0 {
        for xt_val in xt[..order].iter_mut() {
            let mut inner_prod: i32 = 0;
            for i in 0..l {
                inner_prod = silk_add_rshift32(
                    inner_prod,
                    silk_smulbb(x[ptr1_idx + i] as i32, t[i] as i32),
                    rshifts,
                );
            }
            *xt_val = inner_prod;
            ptr1_idx = ptr1_idx.saturating_sub(1);
        }
    } else {
        for xt_val in xt[..order].iter_mut() {
            *xt_val = silk_inner_prod_aligned(&x[ptr1_idx..], t, l);
            ptr1_idx = ptr1_idx.saturating_sub(1);
        }
    }
}

pub fn silk_corr_matrix_fix(
    x: &[i16],
    l: usize,
    order: usize,
    xx: &mut [i32],
    nrg: &mut i32,
    rshifts: &mut i32,
) {
    silk_sum_sqr_shift(nrg, rshifts, x, l + order - 1);
    let mut energy = *nrg;

    for xi in x.iter().take(order - 1) {
        energy -= silk_rshift32(silk_smulbb(*xi as i32, *xi as i32), *rshifts);
    }

    xx[0] = energy;
    let ptr1_start_idx = order - 1;
    for j in 1..order {
        energy = silk_sub32(
            energy,
            silk_rshift32(
                silk_smulbb(
                    x[ptr1_start_idx + l - j] as i32,
                    x[ptr1_start_idx + l - j] as i32,
                ),
                *rshifts,
            ),
        );
        energy = silk_add32(
            energy,
            silk_rshift32(
                silk_smulbb(x[ptr1_start_idx - j] as i32, x[ptr1_start_idx - j] as i32),
                *rshifts,
            ),
        );
        xx[j * order + j] = energy;
    }

    for lag in 1..order {
        let ptr1_idx = ptr1_start_idx;
        let ptr2_idx = ptr1_start_idx - lag;
        let mut inner_prod: i32 = 0;
        if *rshifts > 0 {
            for i in 0..l {
                inner_prod = silk_add_rshift32(
                    inner_prod,
                    silk_smulbb(x[ptr1_idx + i] as i32, x[ptr2_idx + i] as i32),
                    *rshifts,
                );
            }
        } else {
            inner_prod = silk_inner_prod_aligned(&x[ptr1_idx..], &x[ptr2_idx..], l);
        }
        xx[lag] = inner_prod;
        xx[lag * order] = inner_prod;

        for j in 1..(order - lag) {
            inner_prod = silk_sub32(
                inner_prod,
                silk_rshift32(
                    silk_smulbb(x[ptr1_idx + l - j] as i32, x[ptr2_idx + l - j] as i32),
                    *rshifts,
                ),
            );
            inner_prod = silk_add32(
                inner_prod,
                silk_rshift32(
                    silk_smulbb(x[ptr1_idx - j] as i32, x[ptr2_idx - j] as i32),
                    *rshifts,
                ),
            );
            xx[j * order + (lag + j)] = inner_prod;
            xx[(lag + j) * order + j] = inner_prod;
        }
    }
}

const FREQ_TABLE_Q16: [i16; 27] = [
    12111, 9804, 8235, 7100, 6239, 5565, 5022, 4575, 4202, 3885, 3612, 3375, 3167, 2984, 2820,
    2674, 2542, 2422, 2313, 2214, 2123, 2038, 1961, 1889, 1822, 1760, 1702,
];

pub fn silk_apply_sine_window(px_win: &mut [i16], px: &[i16], win_type: i32, length: usize) {
    let mut s0_q16: i32;
    let mut s1_q16: i32;

    let idx = (length >> 2) - 4;
    let f_q16: i32 = FREQ_TABLE_Q16[idx] as i32;

    let c_q16: i32 = silk_smulwb(f_q16, -f_q16);

    if win_type == 1 {
        s0_q16 = 0;
        s1_q16 = f_q16 + (length as i32 >> 3);
    } else {
        s0_q16 = 1 << 16;
        s1_q16 = (1 << 16) + (c_q16 >> 1) + (length as i32 >> 4);
    }

    for k in (0..length).step_by(4) {
        px_win[k] = silk_smulwb((s0_q16 + s1_q16) >> 1, px[k] as i32) as i16;
        px_win[k + 1] = silk_smulwb(s1_q16, px[k + 1] as i32) as i16;
        s0_q16 = silk_smulwb(s1_q16, c_q16) + (s1_q16 << 1) - s0_q16 + 1;
        s0_q16 = s0_q16.min(1 << 16);

        px_win[k + 2] = silk_smulwb((s0_q16 + s1_q16) >> 1, px[k + 2] as i32) as i16;
        px_win[k + 3] = silk_smulwb(s0_q16, px[k + 3] as i32) as i16;
        s1_q16 = silk_smulwb(s0_q16, c_q16) + (s0_q16 << 1) - s1_q16;
        s1_q16 = s1_q16.min(1 << 16);
    }
}

#[inline(always)]
pub fn silk_pitch_xcorr(x: &[i16], y: &[i16], xcorr: &mut [i32], len: usize, max_pitch: usize) {
    debug_assert!(max_pitch > 0);
    debug_assert!(x.len() >= len);
    debug_assert!(xcorr.len() >= max_pitch);

    let y_len = y.len();

    let mut i = 0;
    while i + 3 < max_pitch {
        if y_len >= i + len + 3 {
            let mut sum = [0i32; 4];
            xcorr_kernel_c(x, &y[i..], &mut sum, len);
            xcorr[i] = sum[0];
            xcorr[i + 1] = sum[1];
            xcorr[i + 2] = sum[2];
            xcorr[i + 3] = sum[3];
            i += 4;
        } else {
            for k in i..i.saturating_add(4).min(max_pitch) {
                let avail = len.min(y_len.saturating_sub(k));
                let mut sum = 0i32;
                for j in 0..avail {
                    sum = mac16_16(sum, x[j], y[k + j]);
                }
                xcorr[k] = sum;
            }
            i = i.saturating_add(4).min(max_pitch);
        }
    }

    while i < max_pitch {
        let avail = len.min(y_len.saturating_sub(i));
        let mut sum = 0i32;
        for j in 0..avail {
            sum = mac16_16(sum, x[j], y[i + j]);
        }
        xcorr[i] = sum;
        i += 1;
    }
}

/// Correlation MAC over the warped states: `corr_qc[i] += (state[i]·state[0])>>16`
/// for `i in 0..=order`. Scalar oracle; the AVX2 twin (below) must match it
/// byte-for-byte (i64 accumulate, arithmetic `>>16`).
#[cfg(test)]
#[inline(always)]
fn warped_corr_update_scalar(corr_qc: &mut [i64], state_qs: &[i32], order: usize) {
    let state0 = state_qs[0];
    for i in 0..=order {
        corr_qc[i] += silk_rshift64(silk_smull(state_qs[i], state0), 2 * 13 - 10);
    }
}

#[inline(always)]
fn warped_corr_update(corr_qc: &mut [i64], state_qs: &[i32], order: usize) {
    fvid_cpu::opus_simd::warped_corr_update(corr_qc, state_qs, order);
}

pub fn silk_warped_autocorrelation_fix(
    corr: &mut [i32],
    scale: &mut i32,
    input: &[i16],
    warping_q16: i32,
    length: usize,
    order: usize,
) {
    const QC: i32 = 10;
    const QS: i32 = 13;

    let mut tmp1_qs: i32;
    let mut tmp2_qs: i32;
    let mut state_qs = [0i32; MAX_SHAPE_LPC_ORDER + 1];
    let mut corr_qc = [0i64; MAX_SHAPE_LPC_ORDER + 1];

    debug_assert!((order & 1) == 0);

    for &input_n in input.iter().take(length) {
        tmp1_qs = (input_n as i32) << QS;

        // Warped all-pass state update (serial recurrence — inherently scalar).
        let mut i = 0;
        while i < order {
            tmp2_qs = silk_smlaww(state_qs[i], state_qs[i + 1] - tmp1_qs, warping_q16);
            state_qs[i] = tmp1_qs;
            tmp1_qs = silk_smlaww(state_qs[i + 1], state_qs[i + 2] - tmp2_qs, warping_q16);
            state_qs[i + 1] = tmp2_qs;
            i += 2;
        }
        state_qs[order] = tmp1_qs;

        // Correlation accumulation: corr_qc[i] += (state_qs[i]·state_qs[0]) >> 16
        // for all i — a vector×scalar i64 MAC (state_qs[0] = input<<QS is the
        // per-sample scalar). Vectorizable now that it's split from the recurrence.
        warped_corr_update(&mut corr_qc, &state_qs, order);
    }

    let mut lsh = silk_clz64(corr_qc[0]) - 35;
    lsh = silk_limit_32(lsh, -12 - QC, 30 - QC);
    *scale = -(QC + lsh);
    if lsh >= 0 {
        for i in 0..=order {
            corr[i] = (corr_qc[i] << lsh) as i32;
        }
    } else {
        for i in 0..=order {
            corr[i] = (corr_qc[i] >> (-lsh)) as i32;
        }
    }
}

#[cfg(test)]
mod warped_corr_simd_tests {
    use super::*;

    #[test]
    fn simd_matches_scalar() {
        let mut s: u64 = 0xDEAD_BEEF_1357_9BDF;
        let mut rng = || {
            s ^= s << 13;
            s ^= s >> 7;
            s ^= s << 17;
            s
        };
        for order in [10usize, 12, 14, 16] {
            for _ in 0..2_000 {
                let mut state = [0i32; MAX_SHAPE_LPC_ORDER + 1];
                for v in state.iter_mut().take(order + 1) {
                    *v = (rng() as i32) >> (rng() as u32 % 4);
                }
                let mut a = [0i64; MAX_SHAPE_LPC_ORDER + 1];
                let mut b = [0i64; MAX_SHAPE_LPC_ORDER + 1];
                for k in 0..=order {
                    let init = ((rng() as i64) << 20) >> (rng() as u32 % 20);
                    a[k] = init;
                    b[k] = init;
                }
                warped_corr_update_scalar(&mut a, &state, order);
                warped_corr_update(&mut b, &state, order);
                assert_eq!(a, b, "order={order}");
            }
        }
    }
}

pub fn silk_schur(rc_q15: &mut [i16], c: &[i32], order: usize) -> i32 {
    let mut c_inner = [[0i32; 2]; MAX_LPC_ORDER + 1];
    let mut ctmp1: i32;
    let mut ctmp2: i32;
    let mut rc_tmp_q15: i32;

    assert!(order <= MAX_LPC_ORDER);

    let lz = c[0].leading_zeros() as i32;

    if lz < 2 {
        for i in 0..=order {
            c_inner[i][0] = c[i] >> 1;
            c_inner[i][1] = c[i] >> 1;
        }
    } else if lz > 2 {
        let lz_adj = lz - 2;
        for i in 0..=order {
            c_inner[i][0] = c[i] << lz_adj;
            c_inner[i][1] = c[i] << lz_adj;
        }
    } else {
        for i in 0..=order {
            c_inner[i][0] = c[i];
            c_inner[i][1] = c[i];
        }
    }

    for k in 0..order {
        if c_inner[k + 1][0].abs() >= c_inner[0][1] {
            if c_inner[k + 1][0] > 0 {
                rc_q15[k] = -32440;
            } else {
                rc_q15[k] = 32440;
            }
            return c_inner[0][1];
        }

        rc_tmp_q15 = -silk_div32_16(c_inner[k + 1][0], (c_inner[0][1] >> 15).max(1));

        rc_tmp_q15 = silk_sat16(rc_tmp_q15);

        rc_q15[k] = rc_tmp_q15 as i16;

        for n in 0..order - k {
            ctmp1 = c_inner[n + k + 1][0];
            ctmp2 = c_inner[n][1];
            c_inner[n + k + 1][0] = silk_smlawb(ctmp1, ctmp2 << 1, rc_tmp_q15);
            c_inner[n][1] = silk_smlawb(ctmp2, ctmp1 << 1, rc_tmp_q15);
        }
    }

    c_inner[0][1]
}

pub fn silk_k2a(a_q24: &mut [i32], rc_q15: &[i16], order: usize) {
    for k in 0..order {
        let rc = rc_q15[k] as i32;
        for n in 0..(k + 1) >> 1 {
            let tmp1 = a_q24[n];
            let tmp2 = a_q24[k - n - 1];
            a_q24[n] = silk_smlawb(tmp1, tmp2 << 1, rc);
            a_q24[k - n - 1] = silk_smlawb(tmp2, tmp1 << 1, rc);
        }
        a_q24[k] = -(rc << 9);
    }
}

pub fn silk_bwexpander(ar: &mut [i16], d: usize, mut chirp_q16: i32) {
    let chirp_minus_one_q16 = chirp_q16 - 65536;

    for ar_val in ar[..d - 1].iter_mut() {
        *ar_val = silk_rshift_round((*ar_val as i32).wrapping_mul(chirp_q16), 16) as i16;
        chirp_q16 += silk_rshift_round(chirp_q16.wrapping_mul(chirp_minus_one_q16), 16);
    }
    ar[d - 1] = silk_rshift_round((ar[d - 1] as i32).wrapping_mul(chirp_q16), 16) as i16;
}

pub fn silk_lpc_analysis_filter(
    out: &mut [i16],
    input: &[i16],
    b: &[i16],
    len: usize,
    d: usize,
    _arch: i32,
) {
    fvid_cpu::opus_simd::silk_lpc_analysis_filter(out, input, b, len, d);
}

pub fn silk_scale_copy_vector16(
    data_out: &mut [i16],
    data_in: &[i16],
    gain_q16: i32,
    data_size: usize,
) {
    for i in 0..data_size {
        let tmp32 = silk_smulwb(gain_q16, data_in[i] as i32);
        data_out[i] = silk_sat16(tmp32) as i16;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lpc_analysis_filter_ref(out: &mut [i16], input: &[i16], b: &[i16], len: usize, d: usize) {
        out[..d].fill(0);
        for ix in d..len {
            let mut s = 0i32;
            for k in 0..d {
                s = s.wrapping_add((input[ix - 1 - k] as i32).wrapping_mul(b[k] as i32));
            }
            let out32_q12 = ((input[ix] as i32) << 12).wrapping_sub(s);
            out[ix] = silk_sat16(silk_rshift_round(out32_q12, 12)) as i16;
        }
    }

    fn make_signal(len: usize, seed: u32) -> Vec<i16> {
        let mut s = seed;
        let mut out = vec![0i16; len];
        for v in &mut out {
            s = s.wrapping_mul(1664525).wrapping_add(1013904223);
            *v = (s >> 16) as i16;
        }
        out
    }

    #[test]
    fn test_silk_lpc_analysis_filter_matches_reference() {
        for d in (6usize..=16).step_by(2) {
            for &len in &[d + 8, d + 31, 160, 320] {
                let input = make_signal(len, (d as u32) * 17 + len as u32);
                let b = make_signal(d, 0x1234_5678 ^ d as u32);

                let mut out_opt = vec![0i16; len];
                let mut out_ref = vec![0i16; len];

                silk_lpc_analysis_filter(&mut out_opt, &input, &b, len, d, 0);
                lpc_analysis_filter_ref(&mut out_ref, &input, &b, len, d);

                assert_eq!(out_opt, out_ref, "mismatch for d={d}, len={len}");
            }
        }
    }
}

/// The SILK kernels are fixed-point, so unlike the CELT ones they must agree
/// with their scalar definitions *exactly* — a single-LSB difference changes
/// the bitstream. The AVX2 twins already have tests; these cover the NEON ones
/// and the dispatchers on every target.
#[cfg(test)]
mod simd_tests {
    use super::*;

    fn silk_inner_prod_aligned_scalar(a: &[i16], b: &[i16], len: usize) -> i32 {
        let mut sum = 0i32;
        for i in 0..len {
            sum = mac16_16(sum, a[i], b[i]);
        }
        sum
    }

    fn xcorr_kernel_scalar(x: &[i16], y: &[i16], sum: &mut [i32; 4], len: usize) {
        let mut s0 = 0i32;
        let mut s1 = 0i32;
        let mut s2 = 0i32;
        let mut s3 = 0i32;
        for i in 0..len {
            let xi = x[i];
            s0 = mac16_16(s0, xi, y[i]);
            s1 = mac16_16(s1, xi, y[i + 1]);
            s2 = mac16_16(s2, xi, y[i + 2]);
            s3 = mac16_16(s3, xi, y[i + 3]);
        }
        *sum = [s0, s1, s2, s3];
    }

    fn silk_lpc_analysis_filter_scalar(
        out: &mut [i16],
        input: &[i16],
        b: &[i16],
        len: usize,
        d: usize,
    ) {
        out[..d].fill(0);
        for ix in d..len {
            let mut s = 0i32;
            for k in 0..d {
                s = mac16_16(s, input[ix - 1 - k], b[k]);
            }
            let out32_q12 = ((input[ix] as i32) << 12).wrapping_sub(s);
            out[ix] = silk_sat16(silk_rshift_round(out32_q12, 12)) as i16;
        }
    }

    fn i16_noise(n: usize, seed: u32) -> Vec<i16> {
        let mut s = seed | 1;
        (0..n)
            .map(|_| {
                s = s.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                ((s >> 12) as i32 % 65536 - 32768) as i16
            })
            .collect()
    }

    #[test]
    fn simd_inner_prod_matches_the_scalar_definition() {
        for &len in &[0usize, 1, 3, 4, 7, 8, 15, 16, 31, 32, 100, 480] {
            let a = i16_noise(len, 0xA1 ^ len as u32);
            let b = i16_noise(len, 0xB1 ^ len as u32);
            let got = silk_inner_prod_aligned(&a, &b, len);
            let want = silk_inner_prod_aligned_scalar(&a, &b, len);
            assert_eq!(got, want, "silk_inner_prod_aligned len={len}");
        }
    }

    #[test]
    fn simd_sum_sqr_shift_matches_the_scalar_definition() {
        for &len in &[1usize, 3, 4, 8, 15, 16, 31, 64, 480] {
            for &amp in &[1i32, 100, 10_000, 32_767] {
                let x: Vec<i16> = i16_noise(len, 0xC1 ^ len as u32)
                    .iter()
                    .map(|v| ((*v as i32 * amp) / 32_768) as i16)
                    .collect();

                let (mut energy, mut shift) = (0i32, 0i32);
                silk_sum_sqr_shift(&mut energy, &mut shift, &x, len);

                // libopus `silk_sum_sqr_shift` shifts each *pair* sum rather
                // than the total, so the truncation accumulates per pair. The
                // scalar branch of the function above is the definition; a
                // vectorised path that accumulates in wider precision and
                // shifts once at the end would be more accurate but would not
                // be the same number, and the bitstream depends on it.
                let mut want = 0i32;
                let mut i = 0usize;
                while i + 1 < len {
                    let t = (x[i] as i32 * x[i] as i32) as u32;
                    let t = t.wrapping_add((x[i + 1] as i32 * x[i + 1] as i32) as u32);
                    want = want.wrapping_add((t >> shift) as i32);
                    i += 2;
                }
                if i < len {
                    let t = (x[i] as i32 * x[i] as i32) as u32;
                    want = want.wrapping_add((t >> shift) as i32);
                }
                assert_eq!(
                    energy, want,
                    "silk_sum_sqr_shift len={len} amp={amp} shift={shift}"
                );
            }
        }
    }

    #[test]
    fn simd_xcorr_kernel_matches_the_scalar_definition() {
        for &len in &[1usize, 2, 4, 5, 8, 12, 16, 33, 120] {
            let x = i16_noise(len, 0xD1 ^ len as u32);
            let y = i16_noise(len + 3, 0xE1 ^ len as u32);

            let mut want = [0i32; 4];
            xcorr_kernel_scalar(&x, &y, &mut want, len);

            let mut got = [0i32; 4];
            xcorr_kernel_c(&x, &y, &mut got, len);
            assert_eq!(got, want, "xcorr_kernel len={len}");
        }
    }

    #[test]
    fn simd_warped_corr_update_matches_the_scalar_definition() {
        for &order in &[2usize, 4, 6, 8, 16, 24] {
            let state: Vec<i32> = i16_noise(order + 1, 0xF1 ^ order as u32)
                .iter()
                .map(|v| *v as i32 * 4096)
                .collect();
            let seed: Vec<i64> = (0..order + 1).map(|i| (i as i64) * 7919 - 1000).collect();

            let mut got = seed.clone();
            warped_corr_update(&mut got, &state, order);
            let mut want = seed.clone();
            warped_corr_update_scalar(&mut want, &state, order);
            assert_eq!(got, want, "warped_corr_update order={order}");
        }
    }

    #[test]
    fn simd_lpc_analysis_filter_matches_the_scalar_definition() {
        for &order in &[8usize, 10, 12, 16] {
            for &len in &[order + 1, 32, 97, 480] {
                let x = i16_noise(len, 0x1A ^ len as u32);
                let b: Vec<i16> = i16_noise(order, 0x2A ^ order as u32)
                    .iter()
                    .map(|v| (*v as i32 / 8) as i16)
                    .collect();

                let mut got = vec![0i16; len];
                silk_lpc_analysis_filter(&mut got, &x, &b, len, order, 0);
                let mut want = vec![0i16; len];
                silk_lpc_analysis_filter_scalar(&mut want, &x, &b, len, order);
                assert_eq!(
                    got, want,
                    "silk_lpc_analysis_filter order={order} len={len}"
                );
            }
        }
    }
}
