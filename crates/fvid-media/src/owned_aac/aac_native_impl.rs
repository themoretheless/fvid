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
#[derive(Clone)]
struct ElementSbr {
    width: usize,
    stream: sbr_history::Stream,
    dsp: sbr_dsp::Dsp,
}
impl ElementSbr {
    fn new(width: usize) -> Self {
        Self { width, stream: Default::default(), dsp: Default::default() }
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
    detect_sbr: bool,
    sbr_elements: Vec<Option<ElementSbr>>,
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
    detect_sbr: bool,
    sbr_elements: Vec<Option<ElementSbr>>,
}
fn sbr_layout(parsed: &AudioSpecificConfig) -> Result<bool> {
    let Some(_) = &parsed.program else {
        return Ok(matches!(parsed.core.channel_configuration, 1..=7 | 11 | 12 | 14));
    };
    Ok(parsed.core.channel_configuration == 0)
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
            if !sbr_layout(&parsed)? {
                return Err(unsupported("SBR AAC coupling synthesis is not yet implemented"));
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
        // Four-bit CCE tags occupy a separate fixed domain after audio slots.
        let element_slots = usize::from(config.channels) + if program.as_ref().is_some_and(|p| p.coupling.iter().any(|(independent, _)| *independent)) { 16 } else { 0 };
        Ok(Self {
            config,
            synthesis,
            coupling_synthesis:vec![None;16],
            noise: NoiseState::default(),
            program,
            mapping,
            channel_mask,
            sbr_rate, detect_sbr:false,
            sbr_elements: if sbr_rate.is_some() { vec![None; element_slots] } else { Vec::new() },
        })
    }
    /// Container-declared output clock can identify implicit dual-rate SBR.
    /// No silent rate guessing: unhinted ADTS and explicit SBR=false stay strict.
    pub fn new_with_output_rate(asc:&[u8], output_rate:u32) -> Result<Self> {
        let parsed=AudioSpecificConfig::parse(asc)?;
        parsed.resolve_output_rate(output_rate)?;
        let mut decoder=Self::new(asc)?;
        if decoder.sample_rate()!=output_rate {
            if !sbr_layout(&parsed)? {
                return Err(unsupported("implicit SBR AAC coupling synthesis is not yet implemented"));
            }
            decoder.sbr_rate=Some(output_rate);
            decoder.sbr_elements=vec![None; decoder.sbr_slots()];

        }
        Ok(decoder)
    }
    /// Discover dual-rate SBR from a valid FIL when ASC leaves SBR unspecified.
    /// The output clock may change at that packet; callers must negotiate it
    /// before publishing PCM. Fixed-clock container callers use output hints.
    pub fn new_with_sbr_detection(asc:&[u8]) -> Result<Self> {
        let parsed=AudioSpecificConfig::parse(asc)?;
        let mut decoder=Self::new(asc)?;
        decoder.detect_sbr=parsed.sbr_present.is_none() && sbr_layout(&parsed)?;
        if decoder.detect_sbr { decoder.sbr_elements=vec![None; decoder.sbr_slots()]; }

        Ok(decoder)
    }
    fn sbr_slots(&self) -> usize {
        usize::from(self.config.channels) + if self.program.as_ref().is_some_and(|p| p.coupling.iter().any(|(independent, _)| *independent)) { 16 } else { 0 }
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
            sbr_rate:self.sbr_rate,detect_sbr:self.detect_sbr,sbr_elements:self.sbr_elements.clone()}
    }
    /// Restore without changing the decoder if configuration/layout differs.
    pub fn restore(&mut self, state:&AacCheckpoint) -> Result<()> {
        if self.config!=state.config || self.program!=state.program || self.mapping!=state.mapping || self.channel_mask!=state.channel_mask || self.detect_sbr!=state.detect_sbr || (!self.detect_sbr && self.sbr_rate!=state.sbr_rate) {
            return Err(invalid("AAC checkpoint configuration mismatch"));
        }
        self.synthesis=state.synthesis.clone();self.coupling_synthesis=state.coupling_synthesis.clone();self.noise=state.noise.clone();self.sbr_rate=state.sbr_rate;self.sbr_elements=state.sbr_elements.clone();
        Ok(())
    }
    pub fn reset(&mut self) {
        for synth in &mut self.synthesis {
            synth.reset();
        }
        for synthesis in self.coupling_synthesis.iter_mut().flatten() {synthesis.reset();}
        self.noise.reset();
        if self.detect_sbr {self.sbr_rate=None;}
        self.sbr_elements.fill(None);
    }
    /// One raw_data_block, returning interleaved normalized floating PCM.
    /// A malformed/unsupported packet leaves all decoding state unchanged.
    pub fn decode(&mut self, packet: &[u8]) -> Result<Vec<f32>> {
        let mut bits = BitReader::new(packet);
        let mut noise = self.noise.clone();
        let mut sbr_rate = self.sbr_rate;
        let mut sbr_elements = self.sbr_elements.clone();
        let mut sbr_frames = vec![None; self.sbr_slots()];
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
                    if sbr_rate.is_none() && self.detect_sbr {
                        sbr_rate=Some(self.config.sample_rate.checked_mul(2).ok_or_else(||invalid("SBR frequency overflow"))?);
                    }
                    if sbr_rate.is_none() {
                        return Err(unsupported("AAC fill extension tool SBR requires extension-aware stream signalling"));
                    }
                    let (width, offset) = match previous_element {
                        Some(0 | 1) => {
                            let &(kind, _, offset) = decoded_elements.last().ok_or_else(|| invalid("SBR fill has no audio element"))?;
                            (if kind == 1 { 2 } else { 1 }, offset)
                        }
                        Some(2) => {
                            let (coupling, _) = couplings.last().ok_or_else(|| invalid("SBR fill has no coupling element"))?;
                            if coupling.point != 3 { return Err(unsupported("SBR fill on dependent AAC coupling is not implemented")); }
                            (1, usize::from(self.config.channels) + usize::from(coupling.tag))
                        }
                        _ => return Err(invalid("SBR fill must follow its audio element")),
                    };
                    if sbr_frames[offset].is_some() { return Err(invalid("duplicate SBR fill for AAC element")); }
                    let state = sbr_elements[offset].get_or_insert_with(|| ElementSbr::new(width));
                    if state.width != width { return Err(invalid("SBR element width changed")); }
                    let mut source = SbrBitReader::new(packet);
                    source.skip(input.position()).map_err(|e| invalid(&e.0))?;
                    let rate = self.config.sample_rate.checked_mul(2).ok_or_else(|| invalid("SBR frequency overflow"))?;
                    let frame = state.stream.read(&mut source, end, crc, rate, (self.config.frame_samples/64) as u8, width).map_err(|e| invalid(&e.0))?;
                    input.skip(source.position()-input.position())?;
                    sbr_frames[offset] = Some(frame);
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
            if sbr_rate.is_some() || self.detect_sbr {
                let rate = self.config.sample_rate.checked_mul(2).ok_or_else(|| invalid("SBR frequency overflow"))?;
                let mode = if sbr_rate == Some(self.config.sample_rate) { sbr_dsp::OutputRate::Core } else { sbr_dsp::OutputRate::Double };
                let samples = if mode == sbr_dsp::OutputRate::Core { n } else { n*2 };
                let mut expanded = vec![0.0; samples*channels.len()];
                for &(kind, _, offset) in &decoded_elements {
                    let width = if kind == 1 { 2 } else { 1 };
                    let state = sbr_elements[offset].get_or_insert_with(|| ElementSbr::new(width));
                    if state.width != width { return Err(invalid("SBR element width changed")); }
                    let planar: Vec<Vec<f32>> = (0..width).map(|c| output.chunks_exact(channels.len()).map(|row|row[self.mapping[offset+c]]).collect()).collect();
                    let refs: Vec<_> = planar.iter().map(Vec::as_slice).collect();
                    let rendered = if let Some(frame) = &sbr_frames[offset] {
                        state.dsp.process(frame, &refs, rate, (n/64) as u8, mode)
                    } else {
                        state.dsp.process_upsampling(&refs, rate, (n/64) as u8, mode)
                    }.map_err(|e| invalid(&e.0))?;
                    for (c,data) in rendered.iter().enumerate() {
                        if data.len()!=samples {return Err(invalid("SBR element output length mismatch"));}
                        for (i,&sample) in data.iter().enumerate() {
                            let value=sample as f32;
                            if !value.is_finite(){return Err(invalid("SBR PCM exceeds finite f32 output"));}
                            expanded[i*channels.len()+self.mapping[offset+c]]=value;
                        }
                    }
                }
                if sbr_rate.is_some() {output=expanded;}

            }
            // Independent coupling is applied to final target PCM, after SBR.
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
                let coupled: Vec<f32> = if sbr_rate.is_some() || self.detect_sbr {
                    let offset = usize::from(self.config.channels) + usize::from(coupling.tag);
                    let state = sbr_elements[offset].get_or_insert_with(|| ElementSbr::new(1));
                    let core: Vec<f32> = pcm.iter().map(|&value| value as f32).collect();
                    let rate = self.config.sample_rate.checked_mul(2).ok_or_else(|| invalid("SBR frequency overflow"))?;
                    let mode = if sbr_rate == Some(self.config.sample_rate) { sbr_dsp::OutputRate::Core } else { sbr_dsp::OutputRate::Double };
                    let rendered = if let Some(frame) = &sbr_frames[offset] {
                        state.dsp.process(frame, &[&core], rate, (n/64) as u8, mode)
                    } else {
                        state.dsp.process_upsampling(&[&core], rate, (n/64) as u8, mode)
                    }.map_err(|e| invalid(&e.0))?;
                    if sbr_rate.is_some() { rendered[0].iter().map(|&v| v as f32).collect() } else { core }
                } else { pcm.iter().map(|&v| v as f32).collect() };
                if coupled.len() != output.len()/channels.len() || coupled.iter().any(|v| !v.is_finite()) {
                    return Err(invalid("SBR coupling output length or finite PCM mismatch"));
                }
                for target in coupling.targets {
                    let kind = u32::from(target.pair);
                    let (_, _, offset) = decoded_elements
                        .iter()
                        .find(|(k, t, _)| *k == kind && *t == u32::from(target.tag))
                        .ok_or_else(|| invalid("AAC coupling target is absent"))?;
                    let channel = self.mapping[*offset + target.channel as usize];
                    for (i, &value) in coupled.iter().enumerate() {
                        output[i * channels.len() + channel] += value * target.gain;
                        if !output[i * channels.len() + channel].is_finite() { return Err(invalid("AAC coupled PCM exceeds finite f32 output")); }
                    }
                }
            }
            Ok(output)
        })();
        match result {
            Ok(output) => {
                self.noise = noise;
                self.sbr_rate = sbr_rate;
                self.sbr_elements = sbr_elements;
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
