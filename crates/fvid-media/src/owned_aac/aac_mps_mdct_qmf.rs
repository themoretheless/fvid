//! Owned full-complex residual MDCT -> normalized 64-bin QMF with overlap.
//! This component does not select the native LD filterbank phase, apply ICC
//! corrections or enable residual AAC admission in the player.
use super::{
    Result, aac_ld_mps_residual::Block, aac_sbr_qmf::Complex, aac_synthesis::WindowSequence as W,
    invalid, memory::Footprint, unsupported,
};
use std::f64::consts::PI;
mod facts {
    include!("aac_mps_mdct_qmf_facts.rs");
}
fn frequency(m: usize) -> Result<Vec<f64>> {
    let half = facts::window(m)
        .ok_or_else(|| unsupported("unqualified residual MDCT-to-QMF frequency window"))?;
    let mut result = vec![0.; 2 * m];
    for i in 0..m - m % 2 {
        result[i] = half[i];
        result[2 * m - 1 - m % 2 - i] = half[i];
    }
    if m % 2 == 1 {
        result[m - 1] = half[m - 1];
    }
    Ok(result)
}
fn sine(period: usize, offset: usize, count: usize) -> Vec<f64> {
    (0..count)
        .map(|i| {
            (PI / (2 * period) as f64
                * (i as f64 + offset as f64 + 0.5 * ((period % 2 + 1) as f64)))
                .sin()
        })
        .collect()
}
fn windows(update: usize, sequence: W) -> Result<(Vec<f64>, Vec<f64>)> {
    let (m, time) = match sequence {
        W::OnlyLong => (update, sine(update, 0, 2 * update)),
        W::EightShort => {
            let m = if matches!(update, 16 | 32) {
                update / 8
            } else {
                2
            };
            (m, sine(m, 0, 2 * m))
        }
        W::LongStart | W::LongStop => {
            let (a, b, small) = match update {
                15 => (6, 7, 2),
                16 | 32 => (7 * update / 16, 7 * update / 16, update / 8),
                18 => (8, 8, 2),
                24 => (11, 11, 2),
                30 => (14, 14, 2),
                _ => return Err(unsupported("unqualified residual MDCT-to-QMF update")),
            };
            let time = if sequence == W::LongStart {
                [
                    sine(update, 0, update),
                    vec![1.; a],
                    sine(small, small, small),
                    vec![0.; b],
                ]
                .concat()
            } else {
                [
                    vec![0.; a],
                    sine(small, 0, small),
                    vec![1.; b],
                    sine(update, update, update),
                ]
                .concat()
            };
            (update, time)
        }
    };
    Ok((frequency(m)?, time))
}
fn project(spectrum: &[f64], frequency: &[f64], time: &[f64]) -> Vec<Vec<Complex>> {
    let m = time.len() / 2;
    let size = spectrum.len();
    let mut s = spectrum.to_vec();
    for start in (m..size).step_by(4 * m) {
        for v in &mut s[start..(start + 2 * m).min(size)] {
            *v = -*v;
        }
    }
    let mut main = s.clone();
    let mut slave = vec![0.; size];
    let low = m - m / 2;
    let high = m / 2;
    for row in 0..size / m - 1 {
        let j = row * m;
        let k = j + m;
        for i in 0..low {
            main[k + i] = frequency[high + i] * s[k + i];
            slave[j + i] = frequency[m + high + i] * s[k + i];
        }
        for i in low..m {
            main[j + i] = frequency[m - low + i] * s[j + i];
            slave[k + i] = frequency[i - low] * s[j + i];
        }
    }
    for i in 0..low {
        main[i] = frequency[high + i] * s[i];
        slave[m - 1 - i] = frequency[m + high + i] * s[i];
    }
    for i in low..m {
        let j = size - high + i - low;
        let k = size - i - 1;
        main[j] = frequency[m - low + i] * s[j];
        slave[k] = frequency[i - low] * s[j];
    }
    let width = 2 * m;
    let gain = (1. / width as f64).sqrt();
    let basis: Vec<_> = (0..width)
        .flat_map(|k| {
            (0..width).map(move |n| {
                let phase =
                    (k as f64 - (m / 2) as f64 + 0.5) * (n as f64 - m as f64 + 0.5) * PI / m as f64;
                let (im, re) = phase.sin_cos();
                Complex {
                    re: re * gain * time[k],
                    im: im * gain * time[k],
                }
            })
        })
        .collect();
    let mut result = vec![vec![Complex::default(); 64]; width];
    for band in 0..64 {
        let mut x = vec![0.; width];
        let at = band * m;
        if band % 2 == 0 {
            x[..m].copy_from_slice(&slave[at..at + m]);
            x[m..].copy_from_slice(&main[at..at + m]);
        } else {
            x[..m].copy_from_slice(&main[at..at + m]);
            x[m..].copy_from_slice(&slave[at..at + m]);
        }
        let phase = (258 * band + 385) as f64 * PI / 256.;
        let (im, re) = (-phase).sin_cos();
        for k in 0..width {
            let mut z = Complex::default();
            for n in 0..width {
                z.re += x[n] * basis[k * width + n].re;
                z.im += x[n] * basis[k * width + n].im;
            }
            result[k][band] = Complex {
                re: (re * z.re - im * z.im) / 32768.,
                im: (re * z.im + im * z.re) / 32768.,
            };
        }
    }
    result
}
fn add(out: &mut [Vec<Complex>], offset: usize, projection: Vec<Vec<Complex>>) -> Result<()> {
    if offset + projection.len() > out.len() {
        return Err(invalid("residual MDCT-to-QMF projection exceeds frame"));
    }
    for (a, b) in out[offset..].iter_mut().zip(projection) {
        for (x, y) in a.iter_mut().zip(b) {
            x.re += y.re;
            x.im += y.im;
        }
    }
    Ok(())
}
#[derive(Clone, Debug, PartialEq)]
pub struct Decoder {
    slots: usize,
    update: usize,
    tail: Vec<Vec<Complex>>,
}
impl Decoder {
    pub fn new(slots: usize, frames: usize) -> Result<Self> {
        if slots == 0 || slots > 32 || frames == 0 || frames > 4 || !slots.is_multiple_of(frames) {
            return Err(invalid("invalid residual MDCT-to-QMF geometry"));
        }
        let update = slots / frames;
        if !matches!(update, 15 | 16 | 18 | 24 | 30 | 32) {
            return Err(unsupported("unqualified residual MDCT-to-QMF update"));
        }
        Ok(Self {
            slots,
            update,
            tail: vec![vec![Complex::default(); 64]; slots],
        })
    }
    pub fn reset(&mut self) {
        for row in &mut self.tail {
            row.fill(Complex::default());
        }
    }
    /// Input uses raw AAC spectral units; output uses normalized QMF units.
    /// Failure retains every overlap row and does not mutate the supplied spectra.
    pub fn process(&mut self, blocks: &[Block]) -> Result<Vec<Vec<Complex>>> {
        if blocks.len() != self.slots / self.update {
            return Err(invalid("residual MDCT-to-QMF block count mismatch"));
        }
        let mut out = self.tail.clone();
        out.extend(vec![vec![Complex::default(); 64]; self.slots]);
        for (frame, block) in blocks.iter().enumerate() {
            let short = block.first.sequence == W::EightShort;
            let needs_extra = short && matches!(self.update, 18 | 24 | 30);
            if block.first.coefficients.len() != 1024
                || block.extra.is_some() != needs_extra
                || block
                    .extra
                    .as_ref()
                    .is_some_and(|e| e.sequence != W::EightShort || e.coefficients.len() != 1024)
                || block
                    .first
                    .coefficients
                    .iter()
                    .chain(block.extra.iter().flat_map(|e| &e.coefficients))
                    .any(|v| !v.is_finite())
            {
                return Err(invalid("invalid residual MDCT-to-QMF spectra"));
            }
            let (frequency, time) = windows(self.update, block.first.sequence)?;
            let offset = frame * self.update;
            if !short {
                let mut input = vec![0.; 64 * self.update];
                for (a, b) in input.iter_mut().zip(&block.first.coefficients) {
                    *a = f64::from(*b);
                }
                add(&mut out, offset, project(&input, &frequency, &time))?;
            } else {
                let all: Vec<_> = block
                    .first
                    .coefficients
                    .iter()
                    .chain(block.extra.iter().flat_map(|e| &e.coefficients))
                    .map(|&v| f64::from(v))
                    .collect();
                let (m, count, start, shift) = match self.update {
                    15 => (2, 7, 6, 2),
                    16 => (2, 8, 7, 2),
                    32 => (4, 8, 14, 4),
                    18 => (2, 9, 8, 2),
                    24 => (2, 12, 11, 2),
                    30 => (2, 15, 14, 2),
                    _ => unreachable!(),
                };
                for k in 0..count {
                    let (input, freq, time) = if self.update == 15 && k == 6 {
                        let input = (0..96)
                            .flat_map(|i| [all[k * 128 + i], all[(k + 1) * 128 + i]])
                            .collect::<Vec<_>>();
                        (
                            input,
                            self::frequency(3)?,
                            [sine(2, 0, 2), vec![1.], sine(2, 2, 2), vec![0.]].concat(),
                        )
                    } else {
                        let mut input = vec![0.; 64 * m];
                        input[..128].copy_from_slice(&all[k * 128..(k + 1) * 128]);
                        (input, frequency.clone(), time.clone())
                    };
                    add(
                        &mut out,
                        offset + start + k * shift,
                        project(&input, &freq, &time),
                    )?;
                }
            }
        }
        if out
            .iter()
            .flatten()
            .any(|v| !v.re.is_finite() || !v.im.is_finite())
        {
            return Err(invalid("overflowing residual MDCT-to-QMF output"));
        }
        self.tail = out.split_off(self.slots);
        Ok(out)
    }
    pub fn retained_payload_bytes(&self) -> Result<usize> {
        let mut f = Footprint::new();
        f.vector(&self.tail).map_err(|e| invalid(&e))?;
        for r in &self.tail {
            f.vector(r).map_err(|e| invalid(&e))?;
        }
        Ok(f.total())
    }
}
