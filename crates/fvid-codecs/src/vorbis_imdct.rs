//! FVid-owned Vorbis inverse MDCT using an N/4-point complex FFT with cached twiddles.
use crate::{Result, invalid};
use std::sync::OnceLock;

struct ImdctPlan {
    bitrev: Vec<u16>,
    pre: Vec<(f32, f32)>,
    post: Vec<(f32, f32)>,
    fft_twiddles: Vec<(f32, f32)>,
}

impl ImdctPlan {
    fn new(n: usize) -> Self {
        let l = n / 4;
        let mut bitrev = vec![0u16; l];
        let mut rev = 0usize;
        for slot in &mut bitrev {
            *slot = rev as u16;
            let mut bit = l >> 1;
            while rev & bit != 0 {
                rev ^= bit;
                bit >>= 1;
            }
            rev ^= bit;
        }
        let scale = 2.0 * std::f64::consts::PI / n as f64;
        let pre = (0..l)
            .map(|r| {
                let angle = scale * (r as f64 + 0.25);
                let (s, c) = angle.sin_cos();
                (c as f32, s as f32)
            })
            .collect();
        let post = (0..l)
            .map(|p| {
                let angle = scale * p as f64;
                let (s, c) = angle.sin_cos();
                (c as f32, s as f32)
            })
            .collect();
        // Concatenated stage twiddles for stage = 2, 4, ..., L (total L - 1 entries).
        let mut fft_twiddles = Vec::with_capacity(l.saturating_sub(1));
        let mut stage = 2usize;
        while stage <= l {
            let step = -2.0 * std::f64::consts::PI / stage as f64;
            for k in 0..stage / 2 {
                let (s, c) = (step * k as f64).sin_cos();
                fft_twiddles.push((c as f32, s as f32));
            }
            stage <<= 1;
        }
        Self {
            bitrev,
            pre,
            post,
            fft_twiddles,
        }
    }
}

fn plan(n: usize) -> &'static ImdctPlan {
    static PLANS: OnceLock<[ImdctPlan; 8]> = OnceLock::new();
    let idx = (n.trailing_zeros() as usize) - 6;
    &PLANS.get_or_init(|| std::array::from_fn(|i| ImdctPlan::new(64usize << i)))[idx]
}

/// Fast N/4-point complex f32 Vorbis IMDCT. `scratch` requires only `output.len() / 4` pairs.
pub fn inverse_fast(
    coefficients: &[f32],
    output: &mut [f32],
    scratch: &mut [(f32, f32)],
) -> Result<()> {
    let n = output.len();
    let l = n / 4;
    if !n.is_power_of_two()
        || !(64..=8192).contains(&n)
        || coefficients.len() != n / 2
        || scratch.len() < l
    {
        return Err(invalid("invalid Vorbis IMDCT geometry or scratch"));
    }
    let p = plan(n);
    let buf = &mut scratch[..l];
    let n2 = n / 2;
    for r in 0..l {
        let a = coefficients[2 * r];
        let b = coefficients[n2 - 1 - 2 * r];
        let (c, s) = p.pre[r];
        buf[usize::from(p.bitrev[r])] = (a * c + b * s, b * c - a * s);
    }
    let mut tw_offset = 0usize;
    let mut stage = 2usize;
    while stage <= l {
        let half = stage / 2;
        let twiddles = &p.fft_twiddles[tw_offset..tw_offset + half];
        for block in buf.chunks_exact_mut(stage) {
            let (left, right) = block.split_at_mut(half);
            for ((a, b), &(wr, wi)) in left.iter_mut().zip(right.iter_mut()).zip(twiddles) {
                let (ar, ai) = *a;
                let (br, bi) = *b;
                let tr = wr * br - wi * bi;
                let ti = wr * bi + wi * br;
                *a = (ar + tr, ai + ti);
                *b = (ar - tr, ai - ti);
            }
        }
        tw_offset += half;
        stage <<= 1;
    }
    let n8 = n / 8;
    for k in 0..n8 {
        let (fr, fi) = buf[k];
        let (c, s) = p.post[k];
        let re = fr * c + fi * s;
        let im = fr * s - fi * c;
        let i0 = l - 1 - 2 * k;
        output[n2 + i0] = -re;
        output[n - 1 - i0] = -re;
        output[i0] = im;
        output[n2 - 1 - i0] = -im;
    }
    for k in n8..l {
        let (fr, fi) = buf[k];
        let (c, s) = p.post[k];
        let re = fr * c + fi * s;
        let im = fr * s - fi * c;
        let i1 = 2 * k - l;
        output[i1] = re;
        output[n2 - 1 - i1] = -re;
        output[n2 + i1] = -im;
        output[n - 1 - i1] = -im;
    }
    Ok(())
}

/// Unnormalized Vorbis synthesis transform. For output length N and N/2
/// coefficients, computes sum X[k] cos(2*pi/N*(t+1/2+N/4)*(k+1/2)).
/// Caller supplies 2*N complex scratch values; no frame allocation is hidden.
pub fn inverse(coefficients: &[f32], output: &mut [f32], scratch: &mut [(f64, f64)]) -> Result<()> {
    let n = output.len();
    if !n.is_power_of_two()
        || !(64..=8192).contains(&n)
        || coefficients.len() != n / 2
        || scratch.len() < 2 * n
    {
        return Err(invalid("invalid Vorbis IMDCT geometry or scratch"));
    }
    let mut fast_scratch = [(0.0f32, 0.0f32); 2048];
    inverse_fast(coefficients, output, &mut fast_scratch[..n / 4])
}
#[cfg(test)]
mod tests {
    use super::*;
    fn direct(coefficients: &[f32], t: usize) -> f32 {
        let n = 2 * coefficients.len();
        coefficients
            .iter()
            .enumerate()
            .map(|(k, &x)| {
                f64::from(x)
                    * (2. * std::f64::consts::PI / n as f64
                        * (t as f64 + 0.5 + n as f64 / 4.)
                        * (k as f64 + 0.5))
                        .cos()
            })
            .sum::<f64>() as f32
    }
    #[test]
    fn fft_imdct_matches_direct_cosine_synthesis_at_every_legal_block_size() {
        for exponent in 6..=13 {
            let n = 1usize << exponent;
            let coefficients: Vec<_> = (0..n / 2)
                .map(|k| (((k * 37 + 11) % 101) as f32 - 50.) / 101.)
                .collect();
            let mut output = vec![0.; n];
            let mut scratch = vec![(0., 0.); 2 * n];
            inverse(&coefficients, &mut output, &mut scratch).unwrap();
            let indices: Vec<_> = if n <= 256 {
                (0..n).collect()
            } else {
                vec![
                    0,
                    1,
                    n / 4 - 1,
                    n / 4,
                    n / 2 - 1,
                    n / 2,
                    3 * n / 4,
                    n - 2,
                    n - 1,
                ]
            };
            for t in indices {
                let expected = direct(&coefficients, t);
                assert!(
                    (output[t] - expected).abs() < 0.00002,
                    "n={n} t={t}: {} != {expected}",
                    output[t]
                );
            }
        }
    }
    #[test]
    fn impulse_basis_zero_input_and_invalid_geometry_preserve_contract() {
        let mut coefficients = [0.; 32];
        coefficients[17] = 1.;
        let mut output = [0.; 64];
        let mut scratch = [(0., 0.); 128];
        inverse(&coefficients, &mut output, &mut scratch).unwrap();
        for (t, &value) in output.iter().enumerate() {
            assert!((value - direct(&coefficients, t)).abs() < 0.000001);
        }
        coefficients.fill(0.);
        inverse(&coefficients, &mut output, &mut scratch).unwrap();
        assert_eq!(output, [0.; 64]);
        output.fill(42.);
        assert!(inverse(&coefficients, &mut output, &mut scratch[..127]).is_err());
        assert_eq!(output, [42.; 64]);
    }
}
