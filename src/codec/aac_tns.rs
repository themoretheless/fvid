//! Owned AAC-LC temporal noise shaping side information.
use super::{aac_synthesis::WindowSequence, bits::BitReader};
use crate::{Result, invalid};
use std::f64::consts::FRAC_PI_2;

#[derive(Debug)]
pub struct TnsFilter {
    pub length: usize,
    pub reverse: bool,
    /// Predictor coefficients, excluding the leading unity term.
    pub lpc: Vec<f64>,
}
#[derive(Debug)]
pub struct TnsData {
    pub windows: Vec<Vec<TnsFilter>>,
}
impl TnsData {
    /// Apply each filter within its spectral band interval. `max_band` is
    /// min(max_sfb, sample-rate-specific TNS band limit). No caller mutation.
    pub fn filter(&self, spectrum: &[f32], offsets: &[usize], max_band: usize) -> Result<Vec<f32>> {
        let size = offsets.last().copied().unwrap_or(0);
        if !matches!(spectrum.len(), 960 | 1024)
            || !matches!(self.windows.len(), 1 | 8)
            || size != spectrum.len() / self.windows.len()
            || offsets.first() != Some(&0)
            || offsets.windows(2).any(|p| p[0] >= p[1])
            || max_band >= offsets.len()
            || spectrum.iter().any(|x| !x.is_finite())
        {
            return Err(invalid("invalid AAC TNS spectral geometry"));
        }
        let mut output = spectrum.to_vec();
        for (window, filters) in self.windows.iter().enumerate() {
            let mut top = offsets.len() - 1;
            for filter in filters {
                let bottom = top.saturating_sub(filter.length);
                let start = window * size + offsets[bottom.min(max_band)];
                let end = window * size + offsets[top.min(max_band)];
                top = bottom;
                if filter.lpc.len() > 12 || filter.lpc.iter().any(|x| !x.is_finite()) {
                    return Err(invalid("invalid AAC TNS predictor"));
                }
                let mut history = [0.0f64; 12];
                for step in 0..end - start {
                    let index = if filter.reverse {
                        end - 1 - step
                    } else {
                        start + step
                    };
                    let value = f64::from(output[index])
                        - filter
                            .lpc
                            .iter()
                            .zip(history)
                            .map(|(a, y)| a * y)
                            .sum::<f64>();
                    output[index] = value as f32;
                    if !output[index].is_finite() {
                        return Err(invalid("AAC TNS output overflow"));
                    }
                    history.rotate_right(1);
                    history[0] = value;
                }
            }
        }
        Ok(output)
    }
    /// Called after tns_data_present. Input cursor commits only on success.
    pub fn read(bits: &mut BitReader<'_>, sequence: WindowSequence) -> Result<Self> {
        let short = sequence == WindowSequence::EightShort;
        let mut cursor = bits.clone();
        let mut windows = Vec::with_capacity(if short { 8 } else { 1 });
        for _ in 0..if short { 8 } else { 1 } {
            let count = cursor.read(if short { 1 } else { 2 })?;
            let resolution = if count > 0 {
                3 + cursor.read(1)? as u8
            } else {
                3
            };
            let mut filters = Vec::new();
            for _ in 0..count {
                let length = cursor.read(if short { 4 } else { 6 })? as usize;
                let order = cursor.read(if short { 3 } else { 5 })? as usize;
                if order > if short { 7 } else { 12 } {
                    return Err(invalid("AAC-LC TNS order exceeds limit"));
                }
                let mut reverse = false;
                let mut lpc = Vec::with_capacity(order);
                if order > 0 {
                    reverse = cursor.bit()?;
                    let width = resolution - u8::from(cursor.bit()?);
                    for _ in 0..order {
                        let raw = cursor.read(width)? as i32;
                        let signed = if raw & (1 << (width - 1)) != 0 {
                            raw - (1 << width)
                        } else {
                            raw
                        };
                        let base = (1u32 << (resolution - 1)) as f64;
                        let denominator = base + if signed < 0 { 0.5 } else { -0.5 };
                        let reflection = (f64::from(signed) * FRAC_PI_2 / denominator).sin();
                        let previous = lpc.clone();
                        for i in 0..previous.len() {
                            lpc[i] = previous[i] + reflection * previous[previous.len() - 1 - i];
                        }
                        lpc.push(reflection);
                    }
                }
                filters.push(TnsFilter {
                    length,
                    reverse,
                    lpc,
                });
            }
            windows.push(filters);
        }
        *bits = cursor;
        Ok(Self { windows })
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn pack(fields: &[(u32, u8)]) -> (Vec<u8>, usize) {
        let mut data = Vec::new();
        let mut n = 0;
        for &(v, w) in fields {
            for b in (0..w).rev() {
                if n % 8 == 0 {
                    data.push(0);
                }
                *data.last_mut().unwrap() |= ((v >> b & 1) as u8) << (7 - n % 8);
                n += 1;
            }
        }
        (data, n)
    }
    #[test]
    fn signed_compressed_coefficients_use_original_resolution() {
        for resolution in [3u8, 4] {
            for compressed in [false, true] {
                let width = resolution - u8::from(compressed);
                let (data, n) = pack(&[
                    (1, 2),
                    ((resolution - 3) as u32, 1),
                    (20, 6),
                    (2, 5),
                    (1, 1),
                    (compressed as u32, 1),
                    (1, width),
                    ((1u32 << width) - 1, width),
                ]);
                let mut bits = BitReader::new(&data);
                let tns = TnsData::read(&mut bits, WindowSequence::OnlyLong).unwrap();
                let f = &tns.windows[0][0];
                let base = (1u32 << (resolution - 1)) as f64;
                let a = (FRAC_PI_2 / (base - 0.5)).sin();
                let b = (-FRAC_PI_2 / (base + 0.5)).sin();
                assert_eq!((f.length, f.reverse), (20, true));
                assert!((f.lpc[0] - a * (1.0 + b)).abs() < 1e-15);
                assert!((f.lpc[1] - b).abs() < 1e-15);
                assert_eq!(bits.position(), n);
            }
        }
    }
    #[test]
    fn short_empty_windows_and_zero_order_do_not_read_coefficients() {
        let (data, n) = pack(&[(1, 1), (0, 1), (4, 4), (0, 3), (0, 7)]);
        let mut bits = BitReader::new(&data);
        let tns = TnsData::read(&mut bits, WindowSequence::EightShort).unwrap();
        assert_eq!(tns.windows.len(), 8);
        assert!(tns.windows[0][0].lpc.is_empty());
        assert!(tns.windows[1..].iter().all(Vec::is_empty));
        assert_eq!(bits.position(), n);
    }
    #[test]
    fn filter_direction_band_clipping_and_first_order_impulse() {
        for reverse in [false, true] {
            let tns = TnsData {
                windows: vec![vec![TnsFilter {
                    length: 2,
                    reverse,
                    lpc: vec![0.5],
                }]],
            };
            let mut input = vec![0.0; 1024];
            input[if reverse { 7 } else { 4 }] = 1.0;
            input[0] = 9.0;
            input[8] = 7.0;
            let output = tns.filter(&input, &[0, 4, 8, 1024], 2).unwrap();
            let expected = if reverse {
                [-0.125, 0.25, -0.5, 1.0]
            } else {
                [1.0, -0.5, 0.25, -0.125]
            };
            assert_eq!(&output[4..8], &expected);
            assert_eq!(output[0], 9.0);
            assert_eq!(output[8], 7.0);
        }
    }
    #[test]
    fn excessive_order_and_truncation_preserve_cursor() {
        for data in [
            pack(&[(1, 2), (0, 1), (20, 6), (13, 5)]).0,
            vec![0x40],
            vec![],
        ] {
            let mut bits = BitReader::new(&data);
            assert!(TnsData::read(&mut bits, WindowSequence::OnlyLong).is_err());
            assert_eq!(bits.position(), 0);
        }
    }
}
