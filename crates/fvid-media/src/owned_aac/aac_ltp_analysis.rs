//! Owned ordinary LTP analysis filterbank; decoder admission remains separate.
use super::{
    Result,
    aac_imdct::Imdct,
    aac_synthesis::{WindowSequence, WindowShape, kbd_window},
    invalid, unsupported,
};
use std::{f64::consts::PI, sync::Arc};
#[derive(Clone)]
pub struct LtpAnalysis {
    n: usize,
    long: Arc<[Vec<f64>; 2]>,
    short: Arc<[Vec<f64>; 2]>,
    transform: Imdct,
    windowed: Vec<f64>,
    scratch: Vec<[f64; 2]>,
}
impl LtpAnalysis {
    pub(crate) fn visit_retained(
        &self,
        footprint: &mut super::memory::Footprint,
    ) -> std::result::Result<(), String> {
        for windows in [&self.long, &self.short] {
            if footprint.shared(windows)? {
                for window in windows.iter() {
                    footprint.vector(window)?;
                }
            }
        }
        self.transform.visit_retained(footprint)?;
        footprint.vector(&self.windowed)?;
        footprint.vector(&self.scratch)?;
        Ok(())
    }

    pub fn new(n: usize) -> Result<Self> {
        if !matches!(n, 960 | 1024) {
            return Err(invalid("invalid AAC LTP analysis geometry"));
        }
        let sine = |size: usize| {
            (0..2 * size)
                .map(|i| (PI * (i as f64 + 0.5) / (2 * size) as f64).sin())
                .collect()
        };
        let transform = Imdct::new(n)?;
        let scratch = vec![[0.0; 2]; transform.scratch_len()];
        Ok(Self {
            n,
            long: Arc::new([sine(n), kbd_window(n, 4.0)]),
            short: Arc::new([sine(n / 8), kbd_window(n / 8, 6.0)]),
            transform,
            windowed: vec![0.0; 2 * n],
            scratch,
        })
    }
    /// No state advancement or per-call allocation. Raw cosine-sum normalization.
    pub fn analyze(
        &mut self,
        input: &[f64],
        sequence: WindowSequence,
        previous: WindowShape,
        current: WindowShape,
        output: &mut [f64],
    ) -> Result<()> {
        let n = self.n;
        if input.len() != 2 * n || output.len() != n || input.iter().any(|x| !x.is_finite()) {
            return Err(invalid("invalid AAC LTP analysis samples"));
        }
        if sequence == WindowSequence::EightShort {
            return Err(unsupported(
                "AAC short-window LTP analysis is not integrated",
            ));
        }
        let shape = |s| usize::from(s == WindowShape::Kbd);
        let (before, after) = (shape(previous), shape(current));
        let small = n / 8;
        let flat = (n - small) / 2;
        for (i, value) in input.iter().enumerate() {
            let weight = match sequence {
                WindowSequence::LongStart if i >= n => {
                    let j = i - n;
                    if j < flat {
                        1.0
                    } else if j < flat + small {
                        self.short[after][small + j - flat]
                    } else {
                        0.0
                    }
                }
                WindowSequence::LongStop if i < n => {
                    if i < flat {
                        0.0
                    } else if i < flat + small {
                        self.short[before][i - flat]
                    } else {
                        1.0
                    }
                }
                _ => {
                    if i < n {
                        self.long[before][i]
                    } else {
                        self.long[after][i]
                    }
                }
            };
            self.windowed[i] = value * weight;
        }
        self.transform
            .forward_with_scratch(&self.windowed, output, &mut self.scratch)
    }
}

/// Add a TNS-analyzed long-window prediction only to signaled spectral bands.
/// Validate all selected sums before committing any caller-visible writes.
/// Prediction and residual must already share the same normalization.
pub fn apply_long_prediction(
    residual: &mut [f32],
    prediction: &[f64],
    offsets: &[usize],
    used: &[bool],
) -> Result<()> {
    if !matches!(residual.len(), 480 | 512 | 960 | 1024)
        || prediction.len() != residual.len()
        || offsets.first() != Some(&0)
        || offsets.last() != Some(&residual.len())
        || offsets.windows(2).any(|p| p[0] >= p[1])
        || used.len() > if residual.len() <= 512 { 37 } else { 40 }
        || used.len() >= offsets.len()
        || residual.iter().any(|x| !x.is_finite())
        || prediction.iter().any(|x| !x.is_finite())
    {
        return Err(invalid("invalid AAC LTP spectral bands"));
    }
    for (band, selected) in used.iter().enumerate() {
        if !selected {
            continue;
        }
        for bin in offsets[band]..offsets[band + 1] {
            let sum = residual[bin] as f64 + prediction[bin];
            if !sum.is_finite() || sum.abs() > f32::MAX as f64 {
                return Err(invalid("AAC LTP spectral addition overflow"));
            }
        }
    }
    for (band, selected) in used.iter().enumerate() {
        if !selected {
            continue;
        }
        for bin in offsets[band]..offsets[band + 1] {
            residual[bin] = (residual[bin] as f64 + prediction[bin]) as f32;
        }
    }
    Ok(())
}

impl LtpAnalysis {
    /// Compose the owned prediction stages without advancing decoder history.
    /// Input residual uses raw synthesis/MDCT units. Packet admission and PCM
    /// history normalization must be supplied and qualified by the decoder.
    pub fn predict_long(
        &mut self,
        history: &super::aac_ltp_history::LtpHistory,
        data: &super::aac_ltp_syntax::LtpData,
        sequence: WindowSequence,
        previous: WindowShape,
        current: WindowShape,
        offsets: &[usize],
        tns_max_band: usize,
        tns: Option<&super::aac_tns::TnsData>,
        residual: &mut [f32],
    ) -> Result<()> {
        let super::aac_ltp_syntax::Usage::Bands(used) = &data.usage else {
            return Err(unsupported(
                "AAC short-window LTP analysis is not integrated",
            ));
        };
        if residual.len() != self.n {
            return Err(invalid("invalid AAC LTP residual geometry"));
        }
        let mut estimate = vec![0.; 2 * self.n];
        let mut spectrum = vec![0.; self.n];
        history.estimate_long(data, &mut estimate)?;
        self.analyze(&estimate, sequence, previous, current, &mut spectrum)?;
        if let Some(tns) = tns {
            spectrum = tns.analyze_owned(spectrum, offsets, tns_max_band)?;
        }
        apply_long_prediction(residual, &spectrum, offsets, used)
    }
}

#[cfg(test)]
mod retained_tests {
    use super::*;
    #[test]
    fn spare_analysis_capacity_is_counted_including_shared_windows() {
        let mut analysis = LtpAnalysis::new(1024).unwrap();
        let inspect = |value: &LtpAnalysis| {
            let mut footprint = super::super::memory::Footprint::new();
            value.visit_retained(&mut footprint).unwrap();
            footprint.total()
        };
        let before = inspect(&analysis);
        let old_input = analysis.windowed.capacity();
        let old_scratch = analysis.scratch.capacity();
        let old_window = analysis.long[0].capacity();
        analysis.windowed.reserve(17);
        analysis.scratch.reserve(31);
        Arc::get_mut(&mut analysis.long).unwrap()[0].reserve(43);
        let added = (analysis.windowed.capacity() - old_input + analysis.long[0].capacity()
            - old_window)
            * std::mem::size_of::<f64>()
            + (analysis.scratch.capacity() - old_scratch) * std::mem::size_of::<[f64; 2]>();
        assert!(added > 0);
        assert_eq!(inspect(&analysis), before + added);
        assert_eq!(inspect(&analysis), before + added);
    }
}
