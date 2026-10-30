//! Owned AAC-ELD spatial stream context shared by native AAC frontends.
pub use super::aac_ld_mps_session::Stamp;
use super::{Result, aac_native::AacFrame, invalid, unsupported};
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Core {
    pub object_type: u32,
    pub sample_rate: u32,
    pub frame_samples: u16,
    pub channels: u8,
    pub has_sbr: bool,
    pub downscale: u8,
}
/// Native spatial history stays separate from the coded core channel layout.
#[derive(Clone, Debug, PartialEq)]
pub struct Decoder {
    core: Core,
    config: super::aac_ld_mps_config::Config,
    session: super::aac_ld_mps_session::Session,
    assembler: super::aac_ld_mps_transport::Assembler,
    ready: std::collections::VecDeque<AacFrame>,
    source_clock: Option<u32>,
}
impl Decoder {
    pub fn new(config: &super::aac_ld_mps_config::Config, core: Core) -> Result<Self> {
        if core.object_type != 39 || core.has_sbr || core.downscale != 1 {
            return Err(unsupported(
                "coupled LD-SBR/downscaled spatial filter banks are not yet implemented",
            ));
        }
        let bins = if config.sample_rate < 27713 {
            32
        } else if config.sample_rate >= 55426 {
            128
        } else {
            64
        };
        if config.sample_rate != core.sample_rate
            || config.tree.input_channels != core.channels
            || usize::from(config.frame_slots) * bins != usize::from(core.frame_samples)
        {
            return Err(invalid(
                "LD-MPS rate, downmix channels or frame geometry disagree with AAC ELD core",
            ));
        }
        Ok(Self {
            core,
            config: config.clone(),
            session: super::aac_ld_mps_session::Session::new(
                config,
                super::aac_ld_mps_decor::Mode::Complex,
            )?,
            assembler: Default::default(),
            ready: Default::default(),
            source_clock: None,
        })
    }
    pub fn mask(&self) -> u32 {
        match self.config.tree.code {
            0..=2 => 0x3f,
            3 | 5 => 0xff,
            4 | 6 => 0x63f,
            7 => 3,
            _ => unreachable!(),
        }
    }
    pub fn reset(&mut self) {
        self.session.reset();
        self.assembler.reset();
        self.ready.clear();
        self.source_clock = None;
    }
    pub fn delayed(&self) -> bool {
        !self.ready.is_empty()
            || self.session.queued_samples() != 0
            || self.session.queued_frames() != 0
            || self.assembler.has_pending()
    }
    pub fn finish(&mut self) -> Result<Option<AacFrame>> {
        self.assembler.finish()?;
        self.session.finish()?;
        Ok(self.ready.pop_front())
    }
    pub(crate) fn visit_retained(
        &self,
        f: &mut super::memory::Footprint,
    ) -> std::result::Result<(), String> {
        self.config.visit_retained(f)?;
        self.session.visit_retained(f)?;
        self.assembler.visit_retained(f)?;
        f.deque(&self.ready)?;
        for frame in &self.ready {
            f.vector(&frame.samples)?;
        }
        Ok(())
    }
    fn queue_frames(
        &mut self,
        frames: Vec<super::aac_ld_mps_session::Frame>,
        clock: u32,
    ) -> Result<()> {
        let channels = usize::from(self.config.tree.output_channels);
        let order: &[usize] = if matches!(self.config.tree.code, 4 | 6) {
            &[0, 1, 2, 3, 6, 7, 4, 5]
        } else {
            &[]
        };
        self.ready
            .try_reserve(frames.len())
            .map_err(|_| invalid("AAC LD-MPS ready queue allocation failed"))?;
        for frame in frames {
            if frame.stamp.timescale != clock
                || frame.pcm.channels != channels
                || frame.pcm.sample_rate != self.config.sample_rate
                || !frame.pcm.samples.len().is_multiple_of(channels)
            {
                return Err(invalid("AAC LD-MPS output clock or layout mismatch"));
            }
            let mut samples = Vec::new();
            samples
                .try_reserve(frame.pcm.samples.len())
                .map_err(|_| invalid("AAC LD-MPS PCM allocation failed"))?;
            // Preserve one source window per native response. WAVE back L/R
            // precede side L/R for the rear-surround trees4/6.
            for row in frame.pcm.samples.chunks_exact(channels) {
                for c in 0..channels {
                    let value = row[if order.is_empty() { c } else { order[c] }] as f32;
                    if !value.is_finite() {
                        return Err(invalid("nonfinite AAC LD-MPS PCM output"));
                    }
                    samples.push(value);
                }
            }
            self.ready.push_back(AacFrame {
                samples,
                pts: frame.stamp.pts,
                duration: frame.stamp.duration,
            });
        }
        Ok(())
    }
}

impl Decoder {
    pub fn initial_config(&self) -> &super::aac_ld_mps_config::Config {
        &self.config
    }
    pub fn channels(&self) -> u8 {
        self.config.tree.output_channels
    }
    /// Canonical core PCM and its exact consumed syntax boundary. All stream
    /// state remains unchanged on a malformed suffix, timing or render error.
    pub fn decode_core(
        &mut self,
        packet: &[u8],
        used_bits: usize,
        samples: &[f32],
        stamp: Stamp,
    ) -> Result<Option<AacFrame>> {
        if self
            .source_clock
            .is_some_and(|clock| clock != stamp.timescale)
        {
            return Err(invalid("AAC LD-MPS source clock changed without reset"));
        }
        let mut next = self.clone();
        next.source_clock = Some(stamp.timescale);
        let mut bits = super::bits::BitReader::new(packet);
        bits.skip(used_bits)?;
        let messages = super::aac_pce::read_eld_spatial_extensions(&mut bits, &mut next.assembler)?;
        if next.assembler.replacements() != self.assembler.replacements() {
            return Err(unsupported(
                "AAC LD-MPS replaced-fragment source association is not yet implemented",
            ));
        }
        let channels = usize::from(self.config.tree.input_channels);
        if samples.len() != usize::from(self.core.frame_samples) * channels {
            return Err(invalid("AAC LD-MPS core PCM channel mismatch"));
        }
        let planar: Vec<Vec<f64>> = (0..channels)
            .map(|c| {
                samples
                    .chunks_exact(channels)
                    .map(|r| f64::from(r[c]))
                    .collect()
            })
            .collect();
        let frames = next.session.submit(Some((&planar, stamp)), &messages)?;
        next.queue_frames(frames, stamp.timescale)?;
        let output = next.ready.pop_front();
        *self = next;
        Ok(output)
    }
}
