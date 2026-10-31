//! Owned AAC-LD/AAC-ELD spatial stream context shared by native AAC frontends.
pub use super::aac_ld_mps_session::Stamp;
pub type Subbands = Vec<Vec<Vec<super::aac_sbr_qmf::Complex>>>;
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
        Self::new_domain(config, core, None)
    }
    pub fn new_with_sbr(
        config: &super::aac_ld_mps_config::Config,
        core: Core,
        double_rate: bool,
    ) -> Result<Self> {
        Self::new_domain(config, core, Some(double_rate))
    }
    fn new_domain(
        config: &super::aac_ld_mps_config::Config,
        core: Core,
        sbr: Option<bool>,
    ) -> Result<Self> {
        config.validate_low_delay_profile()?;
        if !matches!(core.object_type, 23 | 39)
            || !matches!(core.downscale, 1 | 2 | 3 | 4 | 8)
            || !core.sample_rate.is_multiple_of(u32::from(core.downscale))
            || !core.frame_samples.is_multiple_of(u16::from(core.downscale))
        {
            return Err(invalid(
                "invalid downscaled AAC LD/ELD spatial core geometry",
            ));
        }
        if core.has_sbr && core.downscale != 1 {
            return Err(unsupported(
                "downscaled coupled LD-SBR spatial filter banks are not yet implemented",
            ));
        }
        let bins = if config.sample_rate < 27713 {
            32
        } else if config.sample_rate >= 55426 {
            128
        } else {
            64
        };
        if core.has_sbr != sbr.is_some() {
            return Err(invalid(
                "LD-MPS SBR context requires explicit shared QMF mode",
            ));
        }
        let factor = if sbr == Some(true) { 2 } else { 1 };
        let rate = core
            .sample_rate
            .checked_div(u32::from(core.downscale))
            .and_then(|rate| rate.checked_mul(factor))
            .ok_or_else(|| invalid("LD-MPS output rate overflow"))?;
        let spatial_samples = usize::from(config.frame_slots) * bins;
        // Entropy/prediction retain nominal geometry; the spatial owner receives
        // short PCM at the explicitly selected output clock.
        let core_samples =
            usize::from(core.frame_samples / u16::from(core.downscale)) * factor as usize;
        // Without SBR,a spatial frame may cover several whole LD/ELD
        // packets (e.g.480-sample ELD with128 bins). Session retains each span.
        let frame_geometry = if !core.has_sbr {
            spatial_samples != 0 && core_samples != 0 && spatial_samples % core_samples == 0
        } else {
            spatial_samples == core_samples
        };
        if config.sample_rate != rate
            || config.tree.input_channels != core.channels
            || !frame_geometry
        {
            return Err(invalid(
                "LD-MPS rate, downmix channels or frame geometry disagree with AAC LD/ELD core",
            ));
        }
        if let Some(double) = sbr {
            if bins != if double { 64 } else { 32 }
                || usize::from(config.frame_slots) != usize::from(core.frame_samples) / 32
            {
                return Err(unsupported(
                    "LD-MPS coupled QMF bank geometry is not yet implemented",
                ));
            }
        }
        Ok(Self {
            core,
            config: config.clone(),
            session: if core.has_sbr {
                super::aac_ld_mps_session::Session::new_qmf(
                    config,
                    super::aac_ld_mps_decor::Mode::Complex,
                )?
            } else {
                super::aac_ld_mps_session::Session::new(
                    config,
                    super::aac_ld_mps_decor::Mode::Complex,
                )?
            },
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
    pub fn set_source_packet_ids(&mut self, enabled: bool) -> Result<()> {
        if !self.ready.is_empty() {
            return Err(invalid("LD-MPS source stamp mode change requires reset"));
        }
        self.session.set_source_packet_ids(enabled)
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
            let mut at = 0usize;
            self.ready
                .try_reserve(frame.sources.len())
                .map_err(|_| invalid("AAC LD-MPS ready queue allocation failed"))?;
            for source in frame.sources {
                let count = source
                    .samples
                    .checked_mul(channels)
                    .ok_or_else(|| invalid("AAC LD-MPS source stride overflow"))?;
                let end = at
                    .checked_add(count)
                    .ok_or_else(|| invalid("AAC LD-MPS source stride overflow"))?;
                if source.stamp.timescale != clock || end > frame.pcm.samples.len() {
                    return Err(invalid("AAC LD-MPS output source span mismatch"));
                }
                let mut samples = Vec::new();
                samples
                    .try_reserve(count)
                    .map_err(|_| invalid("AAC LD-MPS PCM allocation failed"))?;
                // Emit each original AAC source window separately, even when
                // one spatial frame covers several core packets. Container
                // padding/edits must remain attached to their own source PCM.
                for row in frame.pcm.samples[at..end].chunks_exact(channels) {
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
                    pts: source.stamp.pts,
                    duration: source.stamp.duration,
                });
                at = end;
            }
            if at != frame.pcm.samples.len() {
                return Err(invalid("AAC LD-MPS output source span mismatch"));
            }
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
        if self.core.has_sbr {
            return Err(invalid("LD-SBR spatial decoder requires shared QMF input"));
        }
        let channels = usize::from(self.config.tree.input_channels);
        if samples.len()
            != usize::from(self.core.frame_samples / u16::from(self.core.downscale)) * channels
        {
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
        self.decode_signal(packet, used_bits, Some(&planar), None, stamp)
    }
    pub fn decode_qmf(
        &mut self,
        packet: &[u8],
        used_bits: usize,
        qmf: &[Vec<Vec<super::aac_sbr_qmf::Complex>>],
        stamp: Stamp,
    ) -> Result<Option<AacFrame>> {
        if !self.core.has_sbr {
            return Err(invalid("AAC spatial decoder requires core PCM input"));
        }
        if qmf.len() != usize::from(self.config.frame_slots) {
            return Err(invalid("AAC LD-MPS shared QMF slot mismatch"));
        }
        self.decode_signal(packet, used_bits, None, Some(qmf), stamp)
    }
    fn decode_signal(
        &mut self,
        packet: &[u8],
        used_bits: usize,
        pcm: Option<&[Vec<f64>]>,
        qmf: Option<&[Vec<Vec<super::aac_sbr_qmf::Complex>>]>,
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
        for message in &messages {
            if message.kind == super::aac_ld_mps_transport::Kind::HeaderAndFrame {
                let (replacement, _) = super::aac_ld_mps_config::Config::read_in_band(
                    &mut super::bits::BitReader::new(&message.payload),
                )?;
                replacement.validate_low_delay_profile()?;
            }
        }
        if next.assembler.replacements() != self.assembler.replacements() {
            return Err(unsupported(
                "AAC LD-MPS replaced-fragment source association is not yet implemented",
            ));
        }
        let frames = if let Some(qmf) = qmf {
            next.session.submit_qmf(Some((qmf, stamp)), &messages)?
        } else {
            next.session
                .submit(Some((pcm.unwrap(), stamp)), &messages)?
        };
        next.queue_frames(frames, stamp.timescale)?;
        let output = next.ready.pop_front();
        *self = next;
        Ok(output)
    }
}
