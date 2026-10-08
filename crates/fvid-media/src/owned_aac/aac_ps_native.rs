//! Complete owned mono AAC-LC/SBR/PS raw-data-block decoder.
//! Stereo PCM is delayed one packet for real hybrid lookahead; frame_index
//! refers to the original packet. Container timing/startup trimming is external.
use super::{
    Result,
    aac_channel::ChannelData,
    aac_geometry::BandTables,
    aac_noise::NoiseState,
    aac_pce,
    aac_sbr_dsp::OutputRate,
    aac_sbr_ps,
    aac_synthesis::LongSineSynthesis,
    bits::BitReader,
    config::{AacConfig, AudioSpecificConfig},
    invalid, unsupported,
};
#[derive(Clone)]
pub struct NativePsAacDecoder {
    config: AacConfig,
    output_rate: u32,
    mode: OutputRate,
    requires_in_band: bool,
    synthesis: LongSineSynthesis,
    noise: NoiseState,
    // Large fixed QMF/PS histories live on heap so packet transactions and
    // checkpoints do not multiply them on a normal playback thread stack.
    extension: Box<aac_sbr_ps::Decoder>,
}
/// Complete opaque packet boundary, including queued frame and EOF status.
#[derive(Clone)]
pub struct Checkpoint {
    state: NativePsAacDecoder,
}
#[derive(Clone, Debug, PartialEq)]
pub struct Frame {
    pub frame_index: u64,
    pub sample_rate: u32,
    /// Normalized interleaved left/right PCM. Startup delay is not trimmed here.
    pub pcm: Vec<f32>,
}
impl NativePsAacDecoder {
    pub fn new(asc: &[u8]) -> Result<Self> {
        let parsed = AudioSpecificConfig::parse(asc)?;
        if parsed.ps_present != Some(true) || parsed.sbr_present != Some(true) {
            return Err(invalid(
                "native PS decoder requires explicit SBR and PS signalling",
            ));
        }
        let output_rate = parsed
            .extension_sample_rate
            .ok_or_else(|| invalid("missing PS output frequency"))?;
        Self::build(parsed, output_rate, false)
    }
    /// Candidate stereo PS decoder when ASC leaves PS unspecified. The caller
    /// supplies the negotiated output clock and must verify payload presence
    /// before publishing the stereo layout. Explicit disabled tools are honored.
    /// EOF without any in-band PS element rejects this candidate; ordinary mono
    /// AAC remains the job of NativeAacDecoder. First output is delayed as usual.
    pub fn new_with_in_band_ps(asc: &[u8], output_rate: u32) -> Result<Self> {
        let parsed = AudioSpecificConfig::parse(asc)?;
        if parsed.ps_present == Some(false) || parsed.sbr_present == Some(false) {
            return Err(invalid(
                "in-band PS cannot override explicitly disabled PS or SBR",
            ));
        }
        parsed.resolve_output_rate(output_rate)?;
        let requires_in_band = parsed.ps_present != Some(true);
        Self::build(parsed, output_rate, requires_in_band)
    }
    fn build(
        parsed: AudioSpecificConfig,
        output_rate: u32,
        requires_in_band: bool,
    ) -> Result<Self> {
        if parsed.program.is_some()
            || parsed.core.channels != 1
            || parsed.core.channel_configuration != 1
        {
            return Err(unsupported(
                "native PS decoder requires one mono AAC-LC element",
            ));
        }
        let double = parsed
            .core
            .sample_rate
            .checked_mul(2)
            .ok_or_else(|| invalid("PS frequency overflow"))?;
        let mode = if output_rate == parsed.core.sample_rate {
            OutputRate::Core
        } else if output_rate == double {
            OutputRate::Double
        } else {
            return Err(unsupported(
                "PS output frequency requires single or double core rate",
            ));
        };
        BandTables::for_config(&parsed.core)?;
        let synthesis = LongSineSynthesis::new(usize::from(parsed.core.frame_samples))?;
        Ok(Self {
            config: parsed.core,
            output_rate,
            mode,
            requires_in_band,
            synthesis,
            noise: NoiseState::default(),
            extension: Default::default(),
        })
    }
    pub fn sample_rate(&self) -> u32 {
        self.output_rate
    }
    pub fn channels(&self) -> u8 {
        2
    }
    pub fn channel_mask(&self) -> u32 {
        3
    }
    pub fn ps_detected(&self) -> bool {
        self.extension.ps_seen()
    }
    pub fn pending_frame_index(&self) -> Option<u64> {
        self.extension.pending_frame_index()
    }
    pub fn checkpoint(&self) -> Checkpoint {
        Checkpoint {
            state: self.clone(),
        }
    }
    pub fn restore(&mut self, checkpoint: &Checkpoint) -> Result<()> {
        if self.config != checkpoint.state.config
            || self.output_rate != checkpoint.state.output_rate
            || self.mode != checkpoint.state.mode
            || self.requires_in_band != checkpoint.state.requires_in_band
        {
            return Err(invalid("PS AAC checkpoint configuration mismatch"));
        }
        *self = checkpoint.state.clone();
        Ok(())
    }
    pub fn reset(&mut self) {
        self.synthesis.reset();
        self.noise.reset();
        self.extension.reset();
    }
    fn output(&self, frame: Option<aac_sbr_ps::Frame>) -> Result<Option<Frame>> {
        let Some(frame) = frame else {
            return Ok(None);
        };
        let mut pcm = Vec::with_capacity(frame.pcm[0].len() * 2);
        if frame.pcm[0].len() != frame.pcm[1].len() {
            return Err(invalid("PS channel sample counts differ"));
        }
        for (&l, &r) in frame.pcm[0].iter().zip(&frame.pcm[1]) {
            for sample in [l, r] {
                let sample = sample as f32;
                if !sample.is_finite() {
                    return Err(invalid("PS PCM exceeds finite f32 output"));
                }
                pcm.push(sample);
            }
        }
        Ok(Some(Frame {
            frame_index: frame.frame_index,
            sample_rate: self.output_rate,
            pcm,
        }))
    }
    /// Decode the full original packet, including native SCE spectral tools,
    /// IMDCT/window overlap, FIL length/CRC/SBR/PS and ID_END validation.
    /// First packet queues output; later packets emit the preceding frame.
    /// Malformed trailing syntax rolls back core, extension and pending PCM.
    pub fn decode(&mut self, packet: &[u8]) -> Result<Option<Frame>> {
        let mut trial = self.clone();
        let mut bits = BitReader::new(packet);
        let mut core = None;
        let mut extension_seen = false;
        let mut output = None;
        loop {
            match bits.read(3)? {
                0 => {
                    if core.is_some() {
                        return Err(invalid("duplicate PS AAC mono element"));
                    }
                    bits.read(4)?; // SCE tag: one element, no PCE mapping.
                    let channel = ChannelData::read(&mut bits, &trial.config)?;
                    let spectrum = channel.spectrum_with_noise(&trial.config, &mut trial.noise)?;
                    let spectrum = channel.apply_tns(&trial.config, spectrum)?;
                    let mut pcm = vec![0.; usize::from(trial.config.frame_samples)];
                    trial.synthesis.synthesize_pcm(
                        channel.info.sequence,
                        channel.info.shape,
                        &spectrum,
                        &mut pcm,
                    )?;
                    let pcm: Vec<f32> = pcm.into_iter().map(|v| v as f32).collect();
                    if pcm.iter().any(|v| !v.is_finite()) {
                        return Err(invalid("PS AAC core PCM exceeds finite f32 output"));
                    }
                    core = Some(pcm);
                }
                4 => aac_pce::skip_data_stream(&mut bits)?,
                6 => aac_pce::read_fill(&mut bits, |reader, end, crc| {
                    let pcm = core
                        .as_ref()
                        .ok_or_else(|| invalid("PS SBR fill precedes mono element"))?;
                    if extension_seen {
                        return Err(invalid("duplicate PS SBR fill extension"));
                    }
                    let rate = trial
                        .config
                        .sample_rate
                        .checked_mul(2)
                        .ok_or_else(|| invalid("PS frequency overflow"))?;
                    output = trial.extension.read(
                        reader,
                        end,
                        crc,
                        pcm,
                        rate,
                        (trial.config.frame_samples / 64) as u8,
                        trial.mode,
                    )?;
                    extension_seen = true;
                    Ok(())
                })?,
                7 => break,
                _ => {
                    return Err(unsupported(
                        "PS AAC block requires a sole mono SCE without coupling or PCE",
                    ));
                }
            }
        }
        if core.is_none() {
            return Err(invalid("PS AAC block has no mono element"));
        }
        if !extension_seen {
            return Err(unsupported("PS AAC block requires SBR/PS fill"));
        }
        if bits.remaining() > 7 {
            return Err(invalid("trailing bytes after PS AAC END"));
        }
        let output = trial.output(output)?;
        *self = trial;
        Ok(output)
    }
    pub fn finish(&mut self) -> Result<Option<Frame>> {
        if self.requires_in_band && !self.ps_detected() {
            return Err(invalid(
                "in-band PS candidate reached EOF without a PS element",
            ));
        }
        let mut trial = self.clone();
        let output = trial.extension.finish()?;
        let output = trial.output(output)?;
        *self = trial;
        Ok(output)
    }
}
