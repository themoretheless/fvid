//! Constant-gain PCM normalization using owned loudness measurements.
pub use fvid_media_info::{NormalizeTarget, PcmLoudnessStats};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NormalizationPhase {
    Measure,
    Export,
}
impl NormalizationPhase {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Measure => "measure",
            Self::Export => "export",
        }
    }
}
#[derive(Debug, Clone, Copy)]
pub struct NormalizationProgress {
    pub phase: NormalizationPhase,
    pub packets: u64,
    pub payload_bytes: u64,
    pub phase_complete: bool,
    /// True only after output has been atomically published.
    pub done: bool,
}
#[derive(Clone)]
pub struct NormalizationProgressHook(std::sync::Arc<dyn Fn(NormalizationProgress) + Send + Sync>);
impl NormalizationProgressHook {
    pub fn new<F: Fn(NormalizationProgress) + Send + Sync + 'static>(callback: F) -> Self {
        Self(std::sync::Arc::new(callback))
    }
    pub fn for_phase(&self, phase: NormalizationPhase) -> fvid_control::ProgressHook {
        let callback = self.0.clone();
        fvid_control::ProgressHook::new(move |event| {
            callback(NormalizationProgress {
                phase,
                packets: event.packets,
                payload_bytes: event.payload_bytes,
                phase_complete: event.done,
                done: event.done && phase == NormalizationPhase::Export,
            })
        })
    }
}

#[derive(Debug, Clone, Copy)]
pub struct GainPlan {
    pub gain_db: f64,
    pub linear_gain: f64,
    pub peak_limited: bool,
}
impl GainPlan {
    pub fn new(measured: &PcmLoudnessStats, target: NormalizeTarget) -> Result<Self, String> {
        target.validate()?;
        let loudness = measured
            .integrated_lufs
            .filter(|v| v.is_finite())
            .ok_or("normalization requires non-silent audio of at least 400 ms")?;
        let peak = measured
            .sample_peak_dbfs
            .filter(|v| v.is_finite())
            .ok_or("normalization requires a nonzero finite sample peak")?;
        let desired = target.integrated_lufs - loudness;
        let ceiling = target.sample_peak_dbfs - peak;
        let gain_db = desired.min(ceiling);
        let linear_gain = 10f64.powf(gain_db / 20.0);
        if !gain_db.is_finite()
            || !linear_gain.is_finite()
            || !(0.0..=64.0).contains(&linear_gain)
            || linear_gain == 0.0
        {
            return Err(
                "required normalization gain is outside the owned export range (0, 64]".into(),
            );
        }
        Ok(Self {
            gain_db,
            linear_gain,
            peak_limited: ceiling < desired,
        })
    }
    /// Apply to a PCM chunk without changing channel order or filter history.
    /// Invalid input is rejected before any sample is modified.
    pub fn apply(self, pcm: &mut [f64]) -> Result<(), String> {
        if !self.linear_gain.is_finite()
            || !(0.0..=64.0).contains(&self.linear_gain)
            || self.linear_gain == 0.0
            || pcm
                .iter()
                .any(|v| !v.is_finite() || !(v * self.linear_gain).is_finite())
        {
            return Err("normalization requires finite PCM and a valid gain".into());
        }
        for sample in pcm {
            *sample *= self.linear_gain;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn measured(loudness: f64, peak: f64) -> PcmLoudnessStats {
        PcmLoudnessStats {
            sample_frames: 48_000,
            measured_blocks: 7,
            integrated_lufs: Some(loudness),
            sample_peak_dbfs: Some(peak),
            range_lu: None,
        }
    }
    #[test]
    fn gain_preserves_pcm_ratios_and_obeys_peak_ceiling() {
        let plan = GainPlan::new(
            &measured(-22.0, -12.0),
            NormalizeTarget {
                integrated_lufs: -16.0,
                sample_peak_dbfs: -1.5,
            },
        )
        .unwrap();
        assert_eq!(plan.gain_db, 6.0);
        assert!(!plan.peak_limited);
        let mut pcm = [0.125, -0.25, 0.0];
        plan.apply(&mut pcm).unwrap();
        assert!((pcm[0] - 0.24940778937110994).abs() < 1e-12);
        assert_eq!(pcm[1], -2.0 * pcm[0]);
        assert_eq!(pcm[2], 0.0);
        let limited = GainPlan::new(&measured(-22.0, -3.0), NormalizeTarget::default()).unwrap();
        assert_eq!(limited.gain_db, 1.5);
        assert!(limited.peak_limited);
        let mut peak = [10f64.powf(-3.0 / 20.0)];
        limited.apply(&mut peak).unwrap();
        assert!((20.0 * peak[0].log10() + 1.5).abs() < 1e-12);
    }
    #[test]
    fn invalid_measurements_and_chunks_do_not_publish_partial_pcm() {
        for stats in [
            measured(f64::NAN, -3.0),
            measured(-20.0, f64::INFINITY),
            measured(-200.0, -200.0),
        ] {
            assert!(GainPlan::new(&stats, NormalizeTarget::default()).is_err());
        }
        let plan = GainPlan::new(&measured(-22.0, -3.0), NormalizeTarget::default()).unwrap();
        let mut pcm = [0.25, f64::INFINITY];
        assert!(plan.apply(&mut pcm).is_err());
        assert_eq!(pcm[0], 0.25);
    }
}
