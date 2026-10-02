//! Owned four-phase true-peak FIR from ITU-R BS.1770-5 Annex 2.
//! https://www.itu.int/dms_pubrec/itu-r/rec/bs/R-REC-BS.1770-5-202311-I!!PDF-E.pdf
//! The filter estimates peaks on a 4x grid. At 48 kHz this is 192 kHz;
//! `new_for_rate` selects adequate oversampling, using a sinc FIR below 48 kHz.
//! Floating-point processing needs no intermediate 12.04 dB attenuation.
const TAPS: usize = 12;
// Exact coefficients, expressed as integer numerators over 8192. Rows are
// phases; columns run from the newest sample to the oldest history sample.
const COEFFICIENTS: [[i16; TAPS]; 4] = [
    [
        14, 90, -161, 272, -487, 1125, 7964, -838, 390, -218, 122, -68,
    ],
    [
        -239, 240, -424, 730, -1364, 3810, 6388, -1641, 832, -477, 271, -155,
    ],
    [
        -155, 271, -477, 832, -1641, 6388, 3810, -1364, 730, -424, 240, -239,
    ],
    [
        -68, 122, -218, 390, -838, 7964, 1125, -487, 272, -161, 90, 14,
    ],
];
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PeakReport {
    pub sample_frames: u64,
    pub sample_peak: f64,
    /// Maximum of original samples and the interpolated four-phase estimate.
    pub true_peak: f64,
    pub sample_peak_dbfs: Option<f64>,
    pub true_peak_dbfs: Option<f64>,
    /// The final FIR tail is included only after finish().
    pub finished: bool,
}
pub struct TruePeakMeter {
    interpolator: Option<Box<Interpolator>>,
    history: Vec<[f64; TAPS]>,
    position: usize,
    frames: u64,
    sample_peak: f64,
    interpolated_peak: f64,
    finished: bool,
}
impl TruePeakMeter {
    pub fn new(channels: usize) -> Result<Self, String> {
        if !(1..=64).contains(&channels) {
            return Err("true peak requires 1..=64 channels".into());
        }
        Ok(Self {
            interpolator: None,
            history: vec![[0.; TAPS]; channels],
            position: 0,
            frames: 0,
            sample_peak: 0.,
            interpolated_peak: 0.,
            finished: false,
        })
    }
    /// Low-rate PCM uses an owned windowed-sinc polyphase interpolator;
    /// the reconstructed grid is a power-of-two multiple of at least 192 kHz.
    /// Source samples and frame counts remain in the original clock domain.
    pub fn new_for_rate(sample_rate: u32, channels: usize) -> Result<Self, String> {
        if !(8000..=384000).contains(&sample_rate) {
            return Err("true peak sample rate must be in 8000..=384000 Hz".into());
        }
        let mut meter = Self::new(channels)?;
        if sample_rate < 48000 {
            let mut factor = 4;
            while sample_rate * factor < 192000 {
                factor *= 2;
            }
            meter.interpolator = Some(Box::new(Interpolator::new(channels, factor as usize)));
        }
        Ok(meter)
    }
    /// Process interleaved complete channel frames. Invalid chunks leave the
    /// previous readings and all filter history unchanged. No channel weighting
    /// is applied: peaks include LFE and do not cancel antiphase channels.
    pub fn push(&mut self, pcm: &[f64]) -> Result<(), String> {
        if self.finished {
            return Err("true peak meter already finished".into());
        }
        if pcm.len() % self.history.len() != 0
            || pcm.iter().any(|s| !s.is_finite() || s.abs() > 1e100)
        {
            return Err(
                "true peak requires complete finite PCM frames within numeric range".into(),
            );
        }
        let frames = self
            .frames
            .checked_add((pcm.len() / self.history.len()) as u64)
            .ok_or("true peak frame count overflow")?;
        for frame in pcm.chunks_exact(self.history.len()) {
            for sample in frame {
                self.sample_peak = self.sample_peak.max(sample.abs());
            }
            self.filter(frame);
        }
        self.frames = frames;
        Ok(())
    }
    fn filter(&mut self, frame: &[f64]) {
        if let Some(interpolator) = &mut self.interpolator {
            interpolator.filter(frame);
            return;
        }
        for (history, sample) in self.history.iter_mut().zip(frame) {
            history[self.position] = *sample;
        }
        for phase in COEFFICIENTS {
            for history in &self.history {
                let mut value = 0.;
                for (tap, coefficient) in phase.into_iter().enumerate() {
                    value += history[(self.position + TAPS - tap) % TAPS] * f64::from(coefficient)
                        / 8192.;
                }
                self.interpolated_peak = self.interpolated_peak.max(value.abs());
            }
        }
        self.position = (self.position + 1) % TAPS;
    }
    /// Flush the remaining zero-padded FIR tail, retaining original frame
    /// counts. Idempotent; accepting more PCM after finishing would split the
    /// signal with artificial silence, so push then refuses further input.
    pub fn finish(&mut self) {
        if self.finished {
            return;
        }
        let silence = [0.; 64];
        let tail = if self.interpolator.is_some() {
            31
        } else {
            TAPS - 1
        };
        for _ in 0..tail {
            self.filter(&silence[..self.history.len()]);
        }
        self.finished = true;
    }
    pub fn report(&self) -> PeakReport {
        let reconstructed_peak = self
            .interpolator
            .as_ref()
            .map(|m| m.peak)
            .unwrap_or(self.interpolated_peak);
        let true_peak = self.sample_peak.max(reconstructed_peak);
        PeakReport {
            sample_frames: self.frames,
            sample_peak: self.sample_peak,
            true_peak,
            sample_peak_dbfs: (self.sample_peak > 0.).then(|| 20. * self.sample_peak.log10()),
            true_peak_dbfs: (true_peak > 0.).then(|| 20. * true_peak.log10()),
            finished: self.finished,
        }
    }
}
// 32 taps per phase, symmetric Blackman window, unity DC per phase.
// This is interpolation, not a downsampling filter: no 0.94 bandwidth loss.
struct Interpolator {
    coefficients: Vec<[f64; 32]>,
    history: Vec<[f64; 32]>,
    position: usize,
    peak: f64,
}
impl Interpolator {
    fn new(channels: usize, factor: usize) -> Self {
        let coefficients = (0..factor)
            .map(|phase| {
                let fraction = phase as f64 / factor as f64;
                let mut row = [0.; 32];
                for (tap, weight) in row.iter_mut().enumerate() {
                    let distance = tap as f64 - 15. + fraction;
                    if distance.abs() >= 16. {
                        continue;
                    }
                    let angle = std::f64::consts::PI * distance / 16.;
                    let window = 0.42 + 0.5 * angle.cos() + 0.08 * (2. * angle).cos();
                    let phase = std::f64::consts::PI * distance;
                    let sinc = if phase.abs() < 1e-12 {
                        1.
                    } else {
                        phase.sin() / phase
                    };
                    *weight = window * sinc;
                }
                let dc: f64 = row.iter().sum();
                for value in &mut row {
                    *value /= dc;
                }
                row
            })
            .collect();
        Self {
            coefficients,
            history: vec![[0.; 32]; channels],
            position: 0,
            peak: 0.,
        }
    }
    fn filter(&mut self, frame: &[f64]) {
        for (history, sample) in self.history.iter_mut().zip(frame) {
            history[self.position] = *sample;
            for phase in &self.coefficients {
                let value: f64 = phase
                    .iter()
                    .enumerate()
                    .map(|(tap, coefficient)| {
                        history[(self.position + 32 - tap) % 32] * coefficient
                    })
                    .sum();
                self.peak = self.peak.max(value.abs());
            }
        }
        self.position = (self.position + 1) % 32;
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn quarter_rate_tone_exposes_intersample_peak_and_is_chunk_independent() {
        let pcm: Vec<_> = (0..1024)
            .map(|i| {
                let fade = (i.min(1023 - i) as f64 / 64.).min(1.);
                fade * (std::f64::consts::FRAC_PI_2 * i as f64 + std::f64::consts::FRAC_PI_4).sin()
            })
            .collect();
        let mut whole = TruePeakMeter::new(1).unwrap();
        whole.push(&pcm).unwrap();
        whole.finish();
        let expected = whole.report();
        assert!((expected.sample_peak - 0.5f64.sqrt()).abs() < 1e-12);
        assert!(
            (expected.true_peak_dbfs.unwrap()).abs() < 0.1,
            "{expected:?}"
        );
        assert!(expected.true_peak > expected.sample_peak * 1.4);
        for block in [1, 7, 31, 257] {
            let mut meter = TruePeakMeter::new(1).unwrap();
            for samples in pcm.chunks(block) {
                meter.push(samples).unwrap();
            }
            meter.finish();
            assert_eq!(meter.report(), expected);
        }
        let stereo: Vec<_> = pcm.iter().flat_map(|s| [*s, -*s]).collect();
        let mut meter = TruePeakMeter::new(2).unwrap();
        meter.push(&stereo).unwrap();
        meter.finish();
        assert_eq!(meter.report(), expected);
    }
    #[test]
    fn tail_and_exact_rational_convolution_match_without_counting_padding() {
        let mut edge = TruePeakMeter::new(1).unwrap();
        edge.push(&[-1., 1.]).unwrap();
        assert_eq!(edge.report().true_peak, 1.);
        edge.finish();
        assert_eq!(edge.report().true_peak, 8802. / 8192.);
        assert_eq!(edge.report().sample_frames, 2);
        let report = edge.report();
        edge.finish();
        assert_eq!(edge.report(), report);
        assert!(edge.push(&[0.]).is_err());
        // Offline integer convolution is an independent oracle for ring order,
        // both channels, startup padding and EOF tail. All divisions are exact
        // powers of two, so comparison need not hide errors behind tolerance.
        let values: Vec<i64> = (0..74).map(|i| (i * 7919 + 1237) % 65536 - 32768).collect();
        let mut expected = values
            .iter()
            .map(|v| v.abs() as f64 / 32768.)
            .fold(0., f64::max);
        for frame in 0..37 + 11 {
            for channel in 0..2 {
                for phase in COEFFICIENTS {
                    let mut sum = 0i64;
                    for (tap, coef) in phase.into_iter().enumerate() {
                        if frame >= tap && frame - tap < 37 {
                            sum += i64::from(coef) * values[(frame - tap) * 2 + channel];
                        }
                    }
                    expected = expected.max(sum.abs() as f64 / (8192. * 32768.));
                }
            }
        }
        let mut meter = TruePeakMeter::new(2).unwrap();
        let pcm: Vec<_> = values.iter().map(|v| *v as f64 / 32768.).collect();
        meter.push(&pcm).unwrap();
        meter.finish();
        assert_eq!(meter.report().true_peak, expected);
        assert_eq!(meter.report().sample_frames, 37);
    }
    #[test]
    fn silence_rejected_chunks_and_storage_are_bounded() {
        assert!(TruePeakMeter::new(0).is_err());
        assert!(TruePeakMeter::new(65).is_err());
        let mut meter = TruePeakMeter::new(2).unwrap();
        meter.push(&[0.25, -0.5]).unwrap();
        let before = meter.report();
        for invalid in [
            vec![1.],
            vec![1., f64::NAN],
            vec![1., f64::INFINITY],
            vec![1., 1e101],
        ] {
            assert!(meter.push(&invalid).is_err());
            assert_eq!(meter.report(), before);
        }
        let capacity = meter.history.capacity();
        for _ in 0..10000 {
            meter.push(&[0., 0.]).unwrap();
        }
        assert_eq!(meter.history.capacity(), capacity);
        assert_eq!(meter.report().sample_frames, 10001);
        let mut silent = TruePeakMeter::new(64).unwrap();
        silent.push(&[0.; 128]).unwrap();
        silent.finish();
        assert!(silent.report().true_peak_dbfs.is_none());
        assert!(silent.report().sample_peak_dbfs.is_none());
        assert_eq!(silent.report().sample_frames, 2);
    }
    #[test]
    fn low_rate_sinc_keeps_analytic_tone_and_matches_offline_tail() {
        let tone: Vec<f64> = (0usize..1024)
            .map(|i| {
                let fade = (i.min(1023 - i) as f64 / 64.).min(1.);
                fade * (std::f64::consts::FRAC_PI_2 * i as f64 + std::f64::consts::FRAC_PI_4).sin()
            })
            .collect();
        for rate in [8000, 12000, 16000, 22050, 24000, 32000, 44100] {
            let mut whole = TruePeakMeter::new_for_rate(rate, 1).unwrap();
            whole.push(&tone).unwrap();
            whole.finish();
            let report = whole.report();
            assert!(
                report.true_peak_dbfs.unwrap().abs() < 0.05,
                "{rate}: {report:?}"
            );
            let mut split = TruePeakMeter::new_for_rate(rate, 1).unwrap();
            for chunk in tone.chunks(7) {
                split.push(chunk).unwrap();
            }
            split.finish();
            assert_eq!(split.report(), report);
            let capacity = split.interpolator.as_ref().unwrap().history.capacity();
            assert_eq!(capacity, 1);
            let source = [-1., 1., -0.5, 0.25];
            let mut meter = TruePeakMeter::new_for_rate(rate, 1).unwrap();
            let coefficients = &meter.interpolator.as_ref().unwrap().coefficients;
            let mut expected = 1f64;
            for frame in 0..source.len() + 31 {
                for phase in coefficients {
                    let mut value = 0.;
                    for (tap, coefficient) in phase.iter().enumerate() {
                        if frame >= tap && frame - tap < source.len() {
                            value += source[frame - tap] * coefficient;
                        }
                    }
                    expected = expected.max(value.abs());
                }
            }
            meter.push(&source).unwrap();
            meter.finish();
            assert_eq!(meter.report().true_peak, expected);
            assert_eq!(meter.report().sample_frames, 4);
            let final_report = meter.report();
            meter.finish();
            assert_eq!(meter.report(), final_report);
            assert!(meter.push(&[0.]).is_err());
        }
    }
}
