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
    /// FIR analysis for LTP prediction spectra, inverse to TNS synthesis.
    /// Consume a packet-local f64 buffer without a second allocation.
    pub fn analyze_owned(&self, mut spectrum: Vec<f64>, offsets: &[usize], max_band: usize) -> Result<Vec<f64>> {
        let size = offsets.last().copied().unwrap_or(0);
        if !matches!(spectrum.len(), 960 | 1024) || !matches!(self.windows.len(), 1 | 8)
            || size != spectrum.len()/self.windows.len() || offsets.first()!=Some(&0)
            || offsets.windows(2).any(|p|p[0]>=p[1]) || max_band>=offsets.len()
            || spectrum.iter().any(|x|!x.is_finite()) {
            return Err(invalid("invalid AAC TNS analysis geometry"));
        }
        if self.windows.iter().flatten().any(|f| f.lpc.len()>20 || f.lpc.iter().any(|x|!x.is_finite())) {
            return Err(invalid("invalid AAC TNS predictor"));
        }
        for (window, filters) in self.windows.iter().enumerate() {
            let mut top = offsets.len()-1;
            for filter in filters {
                let bottom = top.saturating_sub(filter.length);
                let (start,end)=(window*size+offsets[bottom.min(max_band)],window*size+offsets[top.min(max_band)]);
                top=bottom;
                let mut history=[0.0;20];
                for step in 0..end-start {
                    let index=if filter.reverse {end-1-step} else {start+step};
                    let original=spectrum[index];
                    let value=original+filter.lpc.iter().zip(history).map(|(a,x)|a*x).sum::<f64>();
                    if !value.is_finite() {return Err(invalid("AAC TNS analysis overflow"));}
                    spectrum[index]=value;
                    history.rotate_right(1);history[0]=original;
                }
            }
        }
        Ok(spectrum)
    }
    /// Apply each filter within its spectral band interval. `max_band` is
    /// min(max_sfb, sample-rate-specific TNS band limit). No caller mutation.
    pub fn filter(&self, spectrum: &[f32], offsets: &[usize], max_band: usize) -> Result<Vec<f32>> {
        self.validate(spectrum, offsets, max_band)?;
        self.filter_buffer(spectrum.to_vec(), offsets, max_band)
    }
    /// Consume a packet-local spectrum without allocating a second frame.
    /// Failure discards the consumed buffer; borrowed input APIs remain unchanged.
    pub fn filter_owned(
        &self,
        spectrum: Vec<f32>,
        offsets: &[usize],
        max_band: usize,
    ) -> Result<Vec<f32>> {
        self.validate(&spectrum, offsets, max_band)?;
        self.filter_buffer(spectrum, offsets, max_band)
    }
    fn validate(&self, spectrum: &[f32], offsets: &[usize], max_band: usize) -> Result<()> {
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
        Ok(())
    }
    fn filter_buffer(
        &self,
        mut output: Vec<f32>,
        offsets: &[usize],
        max_band: usize,
    ) -> Result<Vec<f32>> {
        let size = *offsets.last().unwrap();
        for (window, filters) in self.windows.iter().enumerate() {
            let mut top = offsets.len() - 1;
            for filter in filters {
                let bottom = top.saturating_sub(filter.length);
                let start = window * size + offsets[bottom.min(max_band)];
                let end = window * size + offsets[top.min(max_band)];
                top = bottom;
                if filter.lpc.len() > 20 || filter.lpc.iter().any(|x| !x.is_finite()) {
                    return Err(invalid("invalid AAC TNS predictor"));
                }
                let mut history = [0.0f64; 20];
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
            let address = input.as_ptr();
            let capacity = input.capacity();
            let owned = tns.filter_owned(input, &[0, 4, 8, 1024], 2).unwrap();
            assert_eq!(owned, output);
            assert_eq!(owned.as_ptr(), address);
            assert_eq!(owned.capacity(), capacity);
        }
    }
    #[test]
    fn consumed_short_spectra_preserve_each_window_and_direction() {
        for n in [960, 1024] {
            let width = n / 8;
            let tns = TnsData {
                windows: (0..8)
                    .map(|window| {
                        vec![TnsFilter {
                            length: 2,
                            reverse: window % 2 == 1,
                            lpc: vec![0.5],
                        }]
                    })
                    .collect(),
            };
            let mut input = vec![0.0; n];
            for window in 0..8 {
                input[window * width + if window % 2 == 1 { 7 } else { 4 }] = 1.0;
            }
            let output = tns.filter_owned(input, &[0, 4, 8, width], 2).unwrap();
            for window in 0..8 {
                assert_eq!(
                    &output[window * width + 4..window * width + 8],
                    if window % 2 == 1 {
                        &[-0.125, 0.25, -0.5, 1.0]
                    } else {
                        &[1.0, -0.5, 0.25, -0.125]
                    }
                );
                assert!(
                    output[window * width + 8..(window + 1) * width]
                        .iter()
                        .all(|&x| x == 0.0)
                );
            }
        }
    }
}
