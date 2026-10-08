use super::{
 aac_channel::ChannelData, aac_noise::NoiseState, aac_pair::ChannelPair,
    aac_synthesis::LongSineSynthesis, bits::BitReader, config::{AacConfig, AudioSpecificConfig},
};

// Element kinds 0..=3 and their four-bit instance tags form a fixed domain.
// Keep duplicate detection on stack instead of allocating a hash table per packet.
#[derive(Default)]
struct ElementTags([u16; 4]);
impl ElementTags {
    fn insert(&mut self, kind: u32, tag: u32) -> bool {
        let Some(slot) = self.0.get_mut(kind as usize) else {
            return false;
        };
        let Some(bit) = 1u16.checked_shl(tag) else {
            return false;
        };
        let fresh = *slot & bit == 0;
        *slot |= bit;
        fresh
    }
}
pub struct NativeAacDecoder {
    config: AacConfig,
    synthesis: Vec<LongSineSynthesis>,
    coupling_synthesis: Vec<Option<LongSineSynthesis>>,
    noise: NoiseState,
    program: Option<super::aac_pce::ProgramConfig>,
    mapping: Vec<usize>,
    channel_mask: u32,
    sbr_rate: Option<u32>,
    sbr_stream: Option<sbr_history::Stream>,
    sbr_dsp: Option<sbr_dsp::Dsp>,
}
/// Opaque complete packet-boundary state. Configuration and layout are retained
/// to reject restoring into a decoder with another configuration. The caller
/// must associate checkpoints with the corresponding stream and packet cursor.
#[derive(Clone)]
pub struct AacCheckpoint {
    config: AacConfig,
    program: Option<super::aac_pce::ProgramConfig>,
    synthesis: Vec<LongSineSynthesis>,
    coupling_synthesis: Vec<Option<LongSineSynthesis>>,
    noise: NoiseState,
    mapping: Vec<usize>,
    channel_mask: u32,
    sbr_rate: Option<u32>,
    sbr_stream: Option<sbr_history::Stream>,
    sbr_dsp: Option<sbr_dsp::Dsp>,
}
impl NativeAacDecoder {
    pub fn new(asc: &[u8]) -> Result<Self> {
        let parsed = AudioSpecificConfig::parse(asc)?;
        if parsed.ps_present == Some(true) {
            return Err(unsupported("AAC parametric stereo synthesis is not yet implemented"));
        }
        let sbr_rate = if parsed.sbr_present == Some(true) {
            let rate = parsed.extension_sample_rate.ok_or_else(|| invalid("missing SBR output frequency"))?;
            let core = parsed.core.sample_rate;
            if rate != core && core.checked_mul(2) != Some(rate) {
                return Err(unsupported("SBR output frequency requires single or double core rate"));
            }
            if parsed.program.is_some() || !matches!(parsed.core.channel_configuration, 1 | 2) {
                return Err(unsupported("SBR multielement/PCE synthesis is not yet implemented"));
            }
            Some(rate)
        } else { None };
        let config = parsed.core;
        let program = parsed.program;
        BandTables::for_config(&config)?;
        if program.is_none() && !matches!(config.channels, 1..=8) {
            return Err(unsupported("unsupported owned AAC channel layout"));
        }
        let (channel_mask, mapping) = if let Some(program) = &program {
            program.pcm_layout()?
        } else {
            let mapping: &[usize] = match config.channel_configuration {
                1 => &[0],
                2 => &[0, 1],
                3 => &[2, 0, 1],
                4 => &[2, 0, 1, 3],
                5 => &[2, 0, 1, 3, 4],
                6 => &[2, 0, 1, 4, 5, 3],
                // Configuration 7: center, inner-front pair, outer-front pair,
                // back pair, LFE; PCM uses ascending WAVE speaker bits.
                7 => &[2, 6, 7, 0, 1, 4, 5, 3],
                11 => &[2, 0, 1, 4, 5, 6, 3],
                12 => &[2, 0, 1, 6, 7, 4, 5, 3],
                // Configuration 14: normal FC, FL/FR, BL/BR, LFE,
                // followed by top-front left/right.
                14 => &[2, 0, 1, 4, 5, 3, 6, 7],
                _ => unreachable!(),
            };
            (
                if config.channel_configuration == 7 {
                    0xff
                } else if config.channel_configuration == 11 {
                    0x13f
                } else if config.channel_configuration == 12 {
                    0x63f
                } else if config.channel_configuration == 14 {
                    0x503f
                } else {
                    default_pcm_mask(u16::from(config.channels))?
                },
                mapping.to_vec(),
            )
        };
        // Clone initialized state so channels share immutable transforms/windows
        // while every channel retains independent overlap and scratch buffers.
        let synthesis = vec![
            LongSineSynthesis::new(config.frame_samples as usize).map_err(Error::from)?;
            usize::from(config.channels)
        ];
        Ok(Self {
            config,
            synthesis,
            coupling_synthesis:vec![None;16],
            noise: NoiseState::default(),
            program,
            mapping,
            channel_mask,
            sbr_rate,
            sbr_stream: sbr_rate.map(|_| sbr_history::Stream::default()),
            sbr_dsp: sbr_rate.map(|_| sbr_dsp::Dsp::default()),
        })
    }
    pub fn sample_rate(&self) -> u32 {
        self.sbr_rate.unwrap_or(self.config.sample_rate)
    }
    pub fn channels(&self) -> u8 {
        self.config.channels
    }
    pub fn channel_mask(&self) -> u32 {
        self.channel_mask
    }
    /// Save overlap/window and perceptual-noise history after a complete packet.
    /// Explicit PCE positions in emitted PCM order; None for indexed layouts.
    pub fn channel_positions(&self) -> Result<Option<Vec<super::aac_pce::ChannelPosition>>> {
        self.program.as_ref().map(|p| p.pcm_positions()).transpose()
    }
    pub fn checkpoint(&self) -> AacCheckpoint {
        AacCheckpoint {config:self.config.clone(),program:self.program.clone(),
            synthesis:self.synthesis.clone(),coupling_synthesis:self.coupling_synthesis.clone(),noise:self.noise.clone(),
            mapping:self.mapping.clone(),channel_mask:self.channel_mask,
            sbr_rate:self.sbr_rate,sbr_stream:self.sbr_stream.clone(),sbr_dsp:self.sbr_dsp.clone()}
    }
    /// Restore without changing the decoder if configuration/layout differs.
    pub fn restore(&mut self, state:&AacCheckpoint) -> Result<()> {
        if self.config!=state.config || self.program!=state.program || self.mapping!=state.mapping || self.channel_mask!=state.channel_mask || self.sbr_rate!=state.sbr_rate {
            return Err(invalid("AAC checkpoint configuration mismatch"));
        }
        self.synthesis=state.synthesis.clone();self.coupling_synthesis=state.coupling_synthesis.clone();self.noise=state.noise.clone();self.sbr_stream=state.sbr_stream.clone();self.sbr_dsp=state.sbr_dsp.clone();
        Ok(())
    }
    pub fn reset(&mut self) {
        for synth in &mut self.synthesis {
            synth.reset();
        }
        for synthesis in self.coupling_synthesis.iter_mut().flatten() {synthesis.reset();}
        self.noise.reset();
        if let Some(stream) = &mut self.sbr_stream { stream.reset(); }
        if let Some(dsp) = &mut self.sbr_dsp { dsp.reset(); }
    }
    /// One raw_data_block, returning interleaved normalized floating PCM.
    /// A malformed/unsupported packet leaves all decoding state unchanged.
    pub fn decode(&mut self, packet: &[u8]) -> Result<Vec<f32>> {
        let mut bits = BitReader::new(packet);
        let mut noise = self.noise.clone();
        let mut sbr_stream = self.sbr_stream.clone();
        let mut sbr_dsp = self.sbr_dsp.clone();
        let mut sbr_frame = None;
        let mut previous_element = None;
        let mut channels = Vec::new();
        let mut decoded_elements = Vec::new();
        let mut couplings = Vec::new();
        let elements: &[u32] = match self.config.channel_configuration {
            1 => &[0],
            2 => &[1],
            3 => &[0, 1],
            4 => &[0, 1, 0],
            5 => &[0, 1, 1],
            6 => &[0, 1, 1, 3],
            7 | 12 => &[0, 1, 1, 1, 3],
            11 => &[0, 1, 1, 0, 3],
            14 => &[0, 1, 1, 3, 1],
            _ => &[],
        };
        let mut element_index = 0;
        let mut tags = ElementTags::default();
        loop {
            let element = bits.read(3)?;
            let mut target_offset = channels.len();
            if matches!(element, 0 | 1 | 3) {
                if self.program.is_none() && elements.get(element_index) != Some(&element) {
                    return Err(unsupported(
                        "AAC element order differs from standard layout",
                    ));
                }
                element_index += 1;
                let tag = bits.read(4)?;
                if let Some(program) = &self.program {
                    let mut offset = 0;
                    let mut found = None;
                    for configured in &program.elements {
                        let kind = if configured.position == super::aac_pce::Position::Lfe {
                            3
                        } else {
                            u32::from(configured.pair)
                        };
                        if kind == element && u32::from(configured.tag) == tag {
                            found = Some(offset);
                            break;
                        }
                        offset += if configured.pair { 2 } else { 1 };
                    }
                    target_offset = found.ok_or_else(|| invalid("AAC element is absent from PCE"))?;
                }
                decoded_elements.push((element, tag, target_offset));
                if !tags.insert(element, tag) {
                    return Err(invalid("duplicate AAC element tag"));
                }
            }
            match element {
                0 | 3 => {
                    let channel = ChannelData::read(&mut bits, &self.config)?;
                    let spectrum = channel.spectrum_with_noise(&self.config, &mut noise)?;
                    channels.push((channel, spectrum, self.mapping[target_offset]));
                }
                1 => {
                    let pair = ChannelPair::read(&mut bits, &self.config)?;
                    let (left, right) = pair.spectra_with_noise(&self.config, &mut noise)?;
                    channels.push((pair.left, left, self.mapping[target_offset]));
                    channels.push((pair.right, right, self.mapping[target_offset + 1]));
                }
                2 => {
                    let coupling = Coupling::read(&mut bits, &self.config)?;
                    if self
                        .program
                        .as_ref()
                        .is_none_or(|p| !p.coupling.contains(&(coupling.point == 3, coupling.tag)))
                    {
                        return Err(invalid("AAC coupling is absent from configured PCE"));
                    }
                    if !tags.insert(2, u32::from(coupling.tag)) {
                        return Err(invalid("duplicate AAC coupling tag"));
                    }
                    let spectrum = coupling
                        .channel
                        .spectrum_with_noise(&self.config, &mut noise)?;
                    let spectrum = coupling.channel.apply_tns(&self.config, spectrum)?;
                    couplings.push((coupling, spectrum));
                }
                4 => super::aac_pce::skip_data_stream(&mut bits)?,
                5 => {
                    let program = super::aac_pce::ProgramConfig::read(&mut bits, 0)?;
                    let expected = self.program.as_ref().ok_or_else(|| {
                        unsupported("in-band PCE needs an explicit configured program")
                    })?;
                    if program.coupling != expected.coupling
                        || program.elements != expected.elements
                        || program.sample_rate != expected.sample_rate
                        || program.object_type != expected.object_type
                        || program.height_layers()? != expected.height_layers()?
                        || program.pcm_layout()?.0 != self.channel_mask
                    {
                        return Err(invalid("AAC in-band PCE changed the configured layout"));
                    }
                }
                6 => super::aac_pce::read_fill(&mut bits, |input, end, crc| {
                    let stream = sbr_stream.as_mut().ok_or_else(|| unsupported("AAC fill extension tool SBR requires extension-aware stream signalling"))?;
                    if !matches!(previous_element, Some(0 | 1)) || sbr_frame.is_some() {
                        return Err(invalid("SBR fill must follow its sole audio element"));
                    }
                    let mut source = SbrBitReader::new(packet);
                    source.skip(input.position()).map_err(|e| invalid(&e.0))?;
                    let rate = self.config.sample_rate.checked_mul(2).ok_or_else(|| invalid("SBR frequency overflow"))?;
                    let frame = stream.read(&mut source, end, crc, rate, (self.config.frame_samples/64) as u8, usize::from(self.config.channels)).map_err(|e| invalid(&e.0))?;
                    input.skip(source.position()-input.position())?;
                    sbr_frame = Some(frame);
                    Ok(())
                })?,
                7 => break,
                _ => return Err(unsupported("AAC raw-data-block element is not implemented")),
            }
            previous_element = Some(element);
        }
        if channels.len() != self.config.channels as usize {
            return Err(invalid("AAC block has no configured audio element"));
        }
        if bits.remaining() > 7 {
            return Err(invalid("trailing bytes after AAC END"));
        }
        for point in [0, 1] {
            if point == 1 {
                for (channel, spectrum, _) in &mut channels {
                    *spectrum = channel.apply_tns(&self.config, std::mem::take(spectrum))?;
                }
            }
            for (coupling, source) in &couplings {
                if coupling.point != point {
                    continue;
                }
                for target in &coupling.targets {
                    let (_, _, offset) = decoded_elements
                        .iter()
                        .find(|(kind, tag, _)| {
                            *kind == u32::from(target.pair) && *tag == u32::from(target.tag)
                        })
                        .ok_or_else(|| invalid("AAC coupling target is absent"))?;
                    let mapped = self.mapping[*offset + target.channel as usize];
                    let (channel, destination, _) = channels
                        .iter_mut()
                        .find(|(_, _, index)| *index == mapped)
                        .ok_or_else(|| invalid("AAC coupling target is absent"))?;
                    if channel.info.sequence != coupling.channel.info.sequence {
                        return Err(invalid("AAC coupling target window sequence mismatch"));
                    }
                    coupling.mix_spectrum(target, &self.config, source, destination)?;
                }
            }
        }
        // byte_alignment bits have no audio payload.
        let n = self.config.frame_samples as usize;
        let history: Vec<_> = self
            .synthesis
            .iter()
            .map(LongSineSynthesis::history)
            .collect();
        let coupling_history: Vec<_> = self
            .coupling_synthesis
            .iter()
            .map(|state| state.as_ref().map(LongSineSynthesis::history))
            .collect();
        let result = (|| -> Result<Vec<f32>> {
            let mut output = vec![0.0; n * channels.len()];
            let mut pcm = vec![0.0; n];
            for (channel, spectrum, target) in &channels {
                self.synthesis[*target].synthesize_pcm(
                    channel.info.sequence,
                    channel.info.shape,
                    spectrum,
                    &mut pcm,
                )?;
                for i in 0..n {
                    output[i * channels.len() + target] = pcm[i] as f32;
                }
            }
            for (coupling, spectrum) in couplings {
                if coupling.point != 3 {
                    continue;
                }
                let state = &mut self.coupling_synthesis[coupling.tag as usize];
                if state.is_none() {
                    *state = Some(LongSineSynthesis::new(n)?);
                }
                state.as_mut().unwrap().synthesize_pcm(
                    coupling.channel.info.sequence,
                    coupling.channel.info.shape,
                    &spectrum,
                    &mut pcm,
                )?;
                for target in coupling.targets {
                    let kind = u32::from(target.pair);
                    let (_, _, offset) = decoded_elements
                        .iter()
                        .find(|(k, t, _)| *k == kind && *t == u32::from(target.tag))
                        .ok_or_else(|| invalid("AAC coupling target is absent"))?;
                    let channel = self.mapping[*offset + target.channel as usize];
                    for i in 0..n {
                        output[i * channels.len() + channel] += pcm[i] as f32 * target.gain;
                    }
                }
            }
            if self.sbr_rate.is_some() {
                let planar: Vec<Vec<f32>> = (0..channels.len()).map(|c| output.chunks_exact(channels.len()).map(|row| row[c]).collect()).collect();
                let refs: Vec<_> = planar.iter().map(Vec::as_slice).collect();
                let rate = self.config.sample_rate.checked_mul(2).ok_or_else(|| invalid("SBR frequency overflow"))?;
                let mode = if self.sbr_rate == Some(self.config.sample_rate) { sbr_dsp::OutputRate::Core } else { sbr_dsp::OutputRate::Double };
                let dsp = sbr_dsp.as_mut().ok_or_else(|| invalid("missing SBR DSP state"))?;
                let rendered = if let Some(frame) = &sbr_frame {
                    dsp.process(frame, &refs, rate, (self.config.frame_samples/64) as u8, mode)
                } else {
                    dsp.process_upsampling(&refs, rate, (self.config.frame_samples/64) as u8, mode)
                }.map_err(|e| invalid(&e.0))?;
                let samples = rendered[0].len();
                output = vec![0.0; samples*channels.len()];
                for (channel, data) in rendered.iter().enumerate() {
                    for (i, &sample) in data.iter().enumerate() {
                        let sample = sample as f32;
                        if !sample.is_finite() { return Err(invalid("SBR PCM exceeds finite f32 output")); }
                        output[i*channels.len()+channel] = sample;
                    }
                }
            }
            Ok(output)
        })();
        match result {
            Ok(output) => {
                self.noise = noise;
                self.sbr_stream = sbr_stream;
                self.sbr_dsp = sbr_dsp;
                Ok(output)
            }
            Err(error) => {
                for (state, saved) in self.synthesis.iter_mut().zip(&history) {
                    state.restore_history(saved)?;
                }
                for (state, saved) in self.coupling_synthesis.iter_mut().zip(&coupling_history) {
                    if let Some(saved) = saved {
                        state
                            .as_mut()
                            .ok_or_else(|| invalid("AAC coupling rollback state missing"))?
                            .restore_history(saved)?;
                    } else {
                        *state = None;
                    }
                }
                Err(error)
            }
        }
    }
}
