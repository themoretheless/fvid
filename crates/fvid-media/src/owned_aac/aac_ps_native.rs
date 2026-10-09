//! Complete owned mono AAC-LC/SBR/PS raw-data-block decoder.
//! Stereo PCM is delayed one packet for real hybrid lookahead; frame_index
//! refers to the original packet. Container timing/startup trimming is external.
use super::{
    Result,
    aac_channel::ChannelData,
    aac_coupling_syntax::{Coupling, Target},
    aac_geometry::BandTables,
    aac_noise::NoiseState,
    aac_pce, aac_sbr_dsp,
    aac_sbr_dsp::OutputRate,
    aac_sbr_history, aac_sbr_ps,
    aac_synthesis::LongSineSynthesis,
    bits::BitReader,
    config::{AacConfig, AudioSpecificConfig},
    invalid, unsupported,
};
#[derive(Clone, Default)]
struct CceState {
    stream: aac_sbr_history::Stream,
    dsp: aac_sbr_dsp::Dsp,
    synthesis: Option<LongSineSynthesis>,
}
#[derive(Clone)]
pub struct NativePsAacDecoder {
    config: AacConfig,
    program: Option<aac_pce::ProgramConfig>,
    output_rate: u32,
    mode: OutputRate,
    requires_in_band: bool,
    synthesis: LongSineSynthesis,
    noise: NoiseState,
    // Large fixed QMF/PS histories live on heap so packet transactions and
    // checkpoints do not multiply them on a normal playback thread stack.
    extension: Box<aac_sbr_ps::Decoder>,
    cce_states: Vec<Option<CceState>>,
    pending_coupling: Option<(u64, Vec<f32>)>,
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
        validate_mono_program(&parsed)?;
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
            program: parsed.program,
            output_rate,
            mode,
            requires_in_band,
            synthesis,
            noise: NoiseState::default(),
            extension: Default::default(),
            cce_states: vec![None; 16],
            pending_coupling: None,
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
            || self.program != checkpoint.state.program
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
        self.cce_states.fill(None);
        self.pending_coupling = None;
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
        let mut extension = None;
        let mut previous_channel = None;
        let mut couplings = Vec::new();
        let mut cce_tags = 0u16;
        let mut cce_frames = vec![None; 16];
        let rate = trial
            .config
            .sample_rate
            .checked_mul(2)
            .ok_or_else(|| invalid("PS frequency overflow"))?;
        let slots = (trial.config.frame_samples / 64) as u8;
        loop {
            match bits.read(3)? {
                0 => {
                    if core.is_some() {
                        return Err(invalid("duplicate PS AAC mono element"));
                    }
                    let tag = bits.read(4)? as u8;
                    validate_sce_tag(trial.program.as_ref(), tag)?;
                    let channel = ChannelData::read(&mut bits, &trial.config)?;
                    let spectrum = channel.spectrum_with_noise(&trial.config, &mut trial.noise)?;
                    core = Some((channel, spectrum, tag));
                    previous_channel = Some((0, tag));
                }
                2 => {
                    let coupling = Coupling::read(&mut bits, &trial.config)?;
                    validate_coupling(trial.program.as_ref(), &coupling, &mut cce_tags)?;
                    let spectrum = coupling
                        .channel
                        .spectrum_with_noise(&trial.config, &mut trial.noise)?;
                    let spectrum = coupling.channel.apply_tns(&trial.config, spectrum)?;
                    previous_channel = Some((2, coupling.tag));
                    couplings.push((coupling, spectrum));
                }
                4 => aac_pce::skip_data_stream(&mut bits)?,
                5 => read_program(&mut bits, trial.program.as_ref())?,
                6 => aac_pce::read_fill(&mut bits, |reader, end, crc| {
                    match previous_channel {
                        Some((0, _)) => {
                            if extension.is_some() {
                                return Err(invalid("duplicate PS SBR fill extension"));
                            }
                            // Defer synthesis: a CCE can follow the target's FIL.
                            extension = Some((reader.position(), end, crc));
                            reader.skip(end - reader.position())?;
                        }
                        Some((2, tag)) => {
                            if cce_frames[tag as usize].is_some() {
                                return Err(invalid("duplicate SBR fill for AAC element"));
                            }
                            let state = trial.cce_states[tag as usize]
                                .get_or_insert_with(CceState::default);
                            let frame = state.stream.read(reader, end, crc, rate, slots, 1)?;
                            if frame
                                .syntax
                                .data
                                .extended_data
                                .as_ref()
                                .is_some_and(|v| !v.is_empty())
                            {
                                return Err(unsupported(
                                    "SBR extended audio/PS synthesis is not yet implemented",
                                ));
                            }
                            cce_frames[tag as usize] = Some(frame);
                        }
                        _ => return Err(invalid("PS SBR fill precedes mono element")),
                    }
                    Ok(())
                })?,
                7 => break,
                _ => {
                    return Err(unsupported(
                        "PS AAC block requires one mono SCE and configured coupling",
                    ));
                }
            }
        }
        if bits.remaining() > 7 {
            return Err(invalid("trailing bytes after PS AAC END"));
        }
        let (channel, mut spectrum, tag) =
            core.ok_or_else(|| invalid("PS AAC block has no mono element"))?;
        for point in [0, 1] {
            if point == 1 {
                spectrum = channel.apply_tns(&trial.config, spectrum)?;
            }
            for (coupling, source) in &couplings {
                if coupling.point != point {
                    continue;
                }
                for target in &coupling.targets {
                    validate_coupling_target(target, tag)?;
                    if channel.info.sequence != coupling.channel.info.sequence {
                        return Err(invalid("AAC coupling target window sequence mismatch"));
                    }
                    coupling.mix_spectrum(target, &trial.config, source, &mut spectrum)?;
                }
            }
        }
        let n = usize::from(trial.config.frame_samples);
        let mut pcm = vec![0.; n];
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
        let output = if let Some((position, end, crc)) = extension {
            let mut reader = BitReader::new(packet);
            reader.skip(position)?;
            let output =
                trial
                    .extension
                    .read(&mut reader, end, crc, &pcm, rate, slots, trial.mode)?;
            if reader.position() != end {
                return Err(invalid("invalid SBR fill extension consumption"));
            }
            output
        } else {
            trial
                .extension
                .process_upsampling(&pcm, rate, slots, trial.mode)?
        };
        let mut left = Vec::new();
        for (coupling, spectrum) in couplings {
            if coupling.point != 3 {
                continue;
            }
            let state =
                trial.cce_states[coupling.tag as usize].get_or_insert_with(CceState::default);
            if state.synthesis.is_none() {
                state.synthesis = Some(LongSineSynthesis::new(n)?);
            }
            let synthesis = state.synthesis.as_mut().unwrap();
            let mut core = vec![0.; n];
            synthesis.synthesize_pcm(
                coupling.channel.info.sequence,
                coupling.channel.info.shape,
                &spectrum,
                &mut core,
            )?;
            let core: Vec<f32> = core.into_iter().map(|v| v as f32).collect();
            let rendered = if let Some(frame) = &cce_frames[coupling.tag as usize] {
                state
                    .dsp
                    .process(frame, &[&core], rate, slots, trial.mode)?
            } else {
                state
                    .dsp
                    .process_upsampling(&[&core], rate, slots, trial.mode)?
            };
            let samples = if trial.mode == OutputRate::Core {
                n
            } else {
                n * 2
            };
            if rendered[0].len() != samples {
                return Err(invalid("SBR coupling output length mismatch"));
            }
            if left.is_empty() {
                left = vec![0f32; samples];
            }
            for target in coupling.targets {
                validate_coupling_target(&target, tag)?;
                // CCE selects the SCE's channel 0, after SBR/PS. It does not
                // select a CPE pair or feed the PS mono analysis a second time.
                for (dest, &value) in left.iter_mut().zip(&rendered[0]) {
                    *dest += value as f32 * target.gain;
                }
            }
        }
        if left.iter().any(|v| !v.is_finite()) {
            return Err(invalid("AAC coupled PCM exceeds finite f32 output"));
        }
        let output = trial.coupled_output(output)?;
        if !left.is_empty() {
            let index = trial
                .extension
                .pending_frame_index()
                .ok_or_else(|| invalid("missing PS pending frame"))?;
            trial.pending_coupling = Some((index, left));
        }
        *self = trial;
        Ok(output)
    }
    fn coupled_output(&mut self, frame: Option<aac_sbr_ps::Frame>) -> Result<Option<Frame>> {
        let Some(mut frame) = self.output(frame)? else {
            return Ok(None);
        };
        if let Some((index, left)) = self.pending_coupling.take() {
            if index != frame.frame_index || left.len() * 2 != frame.pcm.len() {
                return Err(invalid("PS coupling pending frame mismatch"));
            }
            for (pair, value) in frame.pcm.chunks_exact_mut(2).zip(left) {
                pair[0] += value;
                if !pair[0].is_finite() {
                    return Err(invalid("AAC coupled PCM exceeds finite f32 output"));
                }
            }
        }
        Ok(Some(frame))
    }
    pub fn finish(&mut self) -> Result<Option<Frame>> {
        if self.requires_in_band && !self.ps_detected() {
            return Err(invalid(
                "in-band PS candidate reached EOF without a PS element",
            ));
        }
        let mut trial = self.clone();
        let output = trial.extension.finish()?;
        let output = trial.coupled_output(output)?;
        *self = trial;
        Ok(output)
    }
}

/// Transactional syntax-only PS presence negotiation. No core synthesis, QMF,
/// hybrid, decorrelation or stereo PCM is allocated or computed by this probe.
#[derive(Clone)]
pub struct InBandPsProbe {
    config: AacConfig,
    program: Option<aac_pce::ProgramConfig>,
    sbr: super::aac_sbr_history::Stream,
    ps: super::aac_ps_history::Stream,
    source_sbr: Vec<Option<aac_sbr_history::Stream>>,
    seen: bool,
}
impl InBandPsProbe {
    /// Whether the configured mono program is supported by the own PS parser.
    /// Other valid AAC layouts remain candidates for the ordinary decoder.
    pub fn accepts_mono_program(asc: &[u8]) -> Result<bool> {
        let parsed = AudioSpecificConfig::parse(asc)?;
        Ok(validate_mono_program(&parsed).is_ok())
    }
    pub fn new(asc: &[u8], output_rate: u32) -> Result<Self> {
        let parsed = AudioSpecificConfig::parse(asc)?;
        if parsed.ps_present == Some(false) || parsed.sbr_present == Some(false) {
            return Err(invalid(
                "in-band PS cannot override explicitly disabled PS or SBR",
            ));
        }
        parsed.resolve_output_rate(output_rate)?;
        validate_mono_program(&parsed)?;
        BandTables::for_config(&parsed.core)?;
        Ok(Self {
            config: parsed.core,
            program: parsed.program,
            sbr: Default::default(),
            ps: Default::default(),
            source_sbr: vec![None; 16],
            seen: false,
        })
    }
    pub fn ps_detected(&self) -> bool {
        self.seen
    }
    pub fn reset(&mut self) {
        self.sbr = Default::default();
        self.ps = Default::default();
        self.source_sbr.fill(None);
        self.seen = false;
    }
    /// A packet without SBR fill is valid for discovery, but does not claim PS.
    /// Syntax errors never commit partial headers, histories or presence flags.
    pub fn read(&mut self, packet: &[u8]) -> Result<bool> {
        let mut trial = self.clone();
        let mut bits = BitReader::new(packet);
        let mut core = false;
        let mut fill = false;
        let mut source_fills = 0u16;
        let mut tags = 0u16;
        let mut previous_channel = None;
        let rate = trial
            .config
            .sample_rate
            .checked_mul(2)
            .ok_or_else(|| invalid("PS frequency overflow"))?;
        let slots = (trial.config.frame_samples / 64) as u8;
        loop {
            match bits.read(3)? {
                0 => {
                    if core {
                        return Err(invalid("duplicate PS AAC mono element"));
                    }
                    let tag = bits.read(4)? as u8;
                    validate_sce_tag(trial.program.as_ref(), tag)?;
                    ChannelData::read(&mut bits, &trial.config)?;
                    core = true;
                    previous_channel = Some((0, tag));
                }
                2 => {
                    let coupling = Coupling::read(&mut bits, &trial.config)?;
                    validate_coupling(trial.program.as_ref(), &coupling, &mut tags)?;
                    let tag = trial.program.as_ref().map_or(0, |p| p.elements[0].tag);
                    for target in &coupling.targets {
                        validate_coupling_target(target, tag)?;
                    }
                    previous_channel = Some((2, coupling.tag));
                }
                4 => aac_pce::skip_data_stream(&mut bits)?,
                5 => read_program(&mut bits, trial.program.as_ref())?,
                6 => aac_pce::read_fill(&mut bits, |reader, end, crc| {
                    match previous_channel {
                        Some((0, _)) => {
                            if fill {
                                return Err(invalid("duplicate PS SBR fill extension"));
                            }
                            let frame = trial.sbr.read(reader, end, crc, rate, slots, 1)?;
                            let parsed = trial.ps.read_sbr_extensions(
                                frame.syntax.data.extended_data.as_deref().unwrap_or(&[]),
                                slots * 2,
                            )?;
                            trial.seen |= !parsed.is_empty();
                            fill = true;
                        }
                        Some((2, tag)) => {
                            let mask = 1u16 << tag;
                            if source_fills & mask != 0 {
                                return Err(invalid("duplicate SBR fill for AAC element"));
                            }
                            source_fills |= mask;
                            let source = trial.source_sbr[tag as usize]
                                .get_or_insert_with(aac_sbr_history::Stream::default);
                            let frame = source.read(reader, end, crc, rate, slots, 1)?;
                            if frame
                                .syntax
                                .data
                                .extended_data
                                .as_ref()
                                .is_some_and(|v| !v.is_empty())
                            {
                                return Err(unsupported(
                                    "SBR extended audio/PS synthesis is not yet implemented",
                                ));
                            }
                        }
                        _ => return Err(invalid("PS SBR fill precedes mono element")),
                    }
                    Ok(())
                })?,
                7 => break,
                _ => {
                    return Err(unsupported(
                        "PS AAC block requires one mono SCE and configured coupling",
                    ));
                }
            }
        }
        if !core {
            return Err(invalid("PS AAC block has no mono element"));
        }
        if bits.remaining() > 7 {
            return Err(invalid("trailing bytes after PS AAC END"));
        }
        let seen = trial.seen;
        *self = trial;
        Ok(seen)
    }
}

fn validate_mono_program(parsed: &AudioSpecificConfig) -> Result<()> {
    if parsed.core.object_type != 2 {
        return Err(unsupported("AAC SSR parametric stereo synthesis is not implemented"));
    }
    if parsed.core.channels != 1 {
        return Err(unsupported(
            "native PS decoder requires one mono AAC-LC element",
        ));
    }
    if let Some(program) = &parsed.program {
        if parsed.core.channel_configuration != 0
            || program.elements.len() != 1
            || program.elements[0].pair
            || program.elements[0].position != aac_pce::Position::Front
            || program.height_layers()? != [aac_pce::HeightLayer::Normal]
            || program.pcm_layout()? != (4, vec![0])
        {
            return Err(unsupported(
                "native PS PCE requires a sole normal front mono SCE",
            ));
        }
    } else if parsed.core.channel_configuration != 1 {
        return Err(unsupported(
            "native PS decoder requires one mono AAC-LC element",
        ));
    }
    Ok(())
}
fn validate_sce_tag(program: Option<&aac_pce::ProgramConfig>, tag: u8) -> Result<()> {
    if program.is_some_and(|p| p.elements[0].tag != tag) {
        return Err(invalid("PS AAC mono tag is not configured by PCE"));
    }
    Ok(())
}
fn read_program(bits: &mut BitReader<'_>, expected: Option<&aac_pce::ProgramConfig>) -> Result<()> {
    let program = aac_pce::ProgramConfig::read(bits, 0)?;
    let expected = expected
        .ok_or_else(|| unsupported("in-band PS PCE needs an explicit configured program"))?;
    if program.coupling != expected.coupling
        || program.elements != expected.elements
        || program.sample_rate != expected.sample_rate
        || program.object_type != expected.object_type
        || program.height_layers()? != expected.height_layers()?
        || program.pcm_layout()? != expected.pcm_layout()?
    {
        return Err(invalid("PS AAC in-band PCE changed the configured layout"));
    }
    Ok(())
}

fn validate_coupling(
    program: Option<&aac_pce::ProgramConfig>,
    coupling: &Coupling,
    tags: &mut u16,
) -> Result<()> {
    if program.is_none_or(|p| !p.coupling.contains(&(coupling.point == 3, coupling.tag))) {
        return Err(invalid("AAC coupling is absent from configured PCE"));
    }
    let mask = 1u16 << coupling.tag;
    if *tags & mask != 0 {
        return Err(invalid("duplicate AAC coupling tag"));
    }
    *tags |= mask;
    Ok(())
}
fn validate_coupling_target(target: &Target, tag: u8) -> Result<()> {
    if target.pair || target.tag != tag || target.channel != 0 {
        return Err(invalid("AAC coupling target is absent"));
    }
    Ok(())
}
