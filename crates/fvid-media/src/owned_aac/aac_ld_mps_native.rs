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
#[derive(Clone, Debug, PartialEq)]
struct PartialSource {
    stamp: Stamp,
    samples: usize,
    pcm: Vec<f32>,
}
/// Native spatial history stays separate from the coded core channel layout.
#[derive(Clone, Debug, PartialEq)]
pub struct Decoder {
    core: Core,
    output_multiplier: u8,
    shared_qmf: bool,
    config: super::aac_ld_mps_config::Config,
    session: super::aac_ld_mps_session::Session,
    assembler: super::aac_ld_mps_transport::Assembler,
    ready: std::collections::VecDeque<AacFrame>,
    source_clock: Option<u32>,
    partial_source: Option<PartialSource>,
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
        if core.has_sbr && core.downscale == 3 {
            return Err(unsupported(
                "LD-SBR downscale factor three has no integral filter-bank geometry",
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
            return Err(invalid("LD-MPS SBR context requires explicit rate mode"));
        }
        // Select the shared LD-QMF interface by actual rate/bank geometry,
        // independently of spatial-frame partitioning. Other supported SBR
        // clocks use the PCM interface after complete CLDFB synthesis.
        let factor = if sbr == Some(true) { 2 } else { 1 };
        let shared_qmf =
            core.has_sbr && core.downscale == 1 && bins == if factor == 2 { 64 } else { 32 };
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
        // Spatial frames and AAC source windows are independent partitions.
        // Session slices continuous input; queue_frames reassembles each source
        // window before container padding/edits are applied.
        let frame_geometry = spatial_samples != 0 && core_samples != 0;
        if config.sample_rate != rate
            || config.tree.input_channels != core.channels
            || !frame_geometry
        {
            return Err(invalid(
                "LD-MPS rate, downmix channels or frame geometry disagree with AAC LD/ELD core",
            ));
        }
        let mut session = if shared_qmf {
            super::aac_ld_mps_session::Session::new_qmf(
                config,
                super::aac_ld_mps_decor::Mode::Complex,
            )?
        } else {
            super::aac_ld_mps_session::Session::new(config, super::aac_ld_mps_decor::Mode::Complex)?
        };
        session.enable_packet_slices();
        Ok(Self {
            core,
            output_multiplier: factor as u8,
            shared_qmf,
            config: config.clone(),
            session,
            assembler: Default::default(),
            ready: Default::default(),
            source_clock: None,
            partial_source: None,
        })
    }
    /// Required ingress domain, not the presence of LD-SBR in the bitstream.
    pub fn expects_shared_qmf(&self) -> bool {
        self.shared_qmf
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
        self.partial_source = None;
        self.source_clock = None;
    }
    pub fn set_source_packet_ids(&mut self, enabled: bool) -> Result<()> {
        if !self.ready.is_empty() || self.partial_source.is_some() {
            return Err(invalid("LD-MPS source stamp mode change requires reset"));
        }
        self.session.set_source_packet_ids(enabled)
    }
    pub fn delayed(&self) -> bool {
        !self.ready.is_empty()
            || self.partial_source.is_some()
            || self.session.queued_samples() != 0
            || self.session.queued_frames() != 0
            || self.assembler.has_pending()
    }
    pub fn finish(&mut self) -> Result<Option<AacFrame>> {
        self.assembler.finish()?;
        self.session.finish()?;
        if self.partial_source.is_some() {
            return Err(invalid("unfinished AAC LD-MPS source window"));
        }
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
        if let Some(partial) = &self.partial_source {
            f.vector(&partial.pcm)?;
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
                if source.stamp.timescale != clock
                    || source.original_stamp.timescale != clock
                    || source.samples == 0
                    || source.offset > source.source_samples
                    || end > frame.pcm.samples.len()
                {
                    return Err(invalid("AAC LD-MPS output source span mismatch"));
                }
                if source.offset == 0 {
                    if self.partial_source.is_some() {
                        return Err(invalid(
                            "AAC LD-MPS source window restarted before completion",
                        ));
                    }
                    let capacity = source
                        .source_samples
                        .checked_mul(channels)
                        .ok_or_else(|| invalid("AAC LD-MPS source window stride overflow"))?;
                    let mut pcm = Vec::new();
                    pcm.try_reserve_exact(capacity)
                        .map_err(|_| invalid("AAC LD-MPS PCM allocation failed"))?;
                    self.partial_source = Some(PartialSource {
                        stamp: source.original_stamp,
                        samples: source.source_samples,
                        pcm,
                    });
                }
                let partial = self
                    .partial_source
                    .as_mut()
                    .ok_or_else(|| invalid("AAC LD-MPS missing source-window prefix"))?;
                if partial.stamp != source.original_stamp
                    || partial.samples != source.source_samples
                    || partial.pcm.len() / channels != source.offset
                    || source.samples > source.source_samples.saturating_sub(source.offset)
                {
                    return Err(invalid("AAC LD-MPS source-window slice discontinuity"));
                }
                partial
                    .pcm
                    .try_reserve(count)
                    .map_err(|_| invalid("AAC LD-MPS PCM allocation failed"))?;
                for row in frame.pcm.samples[at..end].chunks_exact(channels) {
                    for c in 0..channels {
                        let value = row[if order.is_empty() { c } else { order[c] }] as f32;
                        if !value.is_finite() {
                            return Err(invalid("nonfinite AAC LD-MPS PCM output"));
                        }
                        partial.pcm.push(value);
                    }
                }
                if partial.pcm.len() / channels == partial.samples {
                    let complete = self.partial_source.take().unwrap();
                    self.ready.push_back(AacFrame {
                        samples: complete.pcm,
                        pts: complete.stamp.pts,
                        duration: complete.stamp.duration,
                    });
                }
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
    /// Canonical decoded PCM at the selected output clock. In the LD-SBR PCM
    /// domain this includes SBR synthesis, and used_bits includes its syntax.
    /// A malformed suffix, timing or render error retains all stream state.
    pub fn decode_core(
        &mut self,
        packet: &[u8],
        used_bits: usize,
        samples: &[f32],
        stamp: Stamp,
    ) -> Result<Option<AacFrame>> {
        if self.expects_shared_qmf() {
            return Err(invalid("LD-SBR spatial decoder requires shared QMF input"));
        }
        let channels = usize::from(self.config.tree.input_channels);
        if samples.len()
            != usize::from(self.core.frame_samples / u16::from(self.core.downscale))
                * usize::from(self.output_multiplier)
                * channels
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
        if !self.expects_shared_qmf() {
            return Err(invalid("AAC spatial decoder requires decoded PCM input"));
        }
        if qmf.len() != usize::from(self.core.frame_samples) / 32 {
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

#[cfg(test)]
mod source_window_tests {
    use super::super::{
        aac_ld_mps_pcm,
        aac_ld_mps_session::{Frame, SourceSpan},
        memory::Footprint,
    };
    use super::*;
    fn bytes(decoder: &Decoder) -> usize {
        let mut f = Footprint::new();
        decoder.visit_retained(&mut f).unwrap();
        f.total()
    }
    #[test]
    fn partial_window_capacity_is_counted_preserved_and_cleared_on_reset() {
        let config = super::super::aac_ld_mps_config::Config::parse(include_bytes!(
            "../../../../tests/fixtures/playback-errors/aac-eld-mps-core/tree7.ssc"
        ))
        .unwrap();
        let mut decoder = Decoder::new(
            &config,
            Core {
                object_type: 39,
                sample_rate: 48000,
                frame_samples: 512,
                channels: 1,
                has_sbr: false,
                downscale: 1,
            },
        )
        .unwrap();
        let original = Stamp {
            pts: 100,
            duration: 512,
            timescale: 48000,
        };
        let frame = |offset, value| Frame {
            pcm: aac_ld_mps_pcm::Frame {
                sample_rate: 48000,
                channels: 2,
                samples: vec![value; 512],
            },
            stamp: Stamp {
                pts: 100 + offset as i64,
                duration: 256,
                timescale: 48000,
            },
            sources: vec![SourceSpan {
                offset,
                source_samples: 512,
                original_stamp: original,
                samples: 256,
                stamp: Stamp {
                    pts: 100 + offset as i64,
                    duration: 256,
                    timescale: 48000,
                },
            }],
        };
        decoder.queue_frames(vec![frame(0, 0.01)], 48000).unwrap();
        assert!(decoder.ready.is_empty());
        assert!(decoder.delayed());
        let saved = decoder.clone();
        assert!(
            decoder
                .finish()
                .unwrap_err()
                .to_string()
                .contains("unfinished AAC LD-MPS source window")
        );
        assert_eq!(decoder, saved);
        assert!(decoder.set_source_packet_ids(true).is_err());
        let before = bytes(&decoder);
        let partial = decoder.partial_source.as_mut().unwrap();
        let old = partial.pcm.capacity();
        partial.pcm.reserve(old + 97);
        let added = (partial.pcm.capacity() - old) * 4;
        assert_eq!(bytes(&decoder), before + added);
        decoder.queue_frames(vec![frame(256, 0.02)], 48000).unwrap();
        assert!(decoder.partial_source.is_none());
        let complete = decoder.ready.front().unwrap();
        assert_eq!((complete.pts, complete.duration), (100, 512));
        assert_eq!(complete.samples.len(), 1024);
        assert!(complete.samples[..512].iter().all(|&v| v == 0.01f32));
        assert!(complete.samples[512..].iter().all(|&v| v == 0.02f32));
        decoder = saved;
        decoder.reset();
        assert!(decoder.partial_source.is_none());
        assert!(!decoder.delayed());
    }
}
