//! Owned AAC temporal noise shaping spectral filters.
use super::{Result, invalid};

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
}

#[cfg(test)]
mod tests {
    use super::*;
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
}
