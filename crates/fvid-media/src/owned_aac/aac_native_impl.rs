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
#[derive(Clone)]
struct SsrPacketMetadata {
    duration: u64,
    sbr: Option<SsrSbrMetadata>,
}
#[derive(Clone)]
struct SsrSbrMetadata {
    // Discovery candidates advance DSP but publish the original core PCM.
    active: bool,
    output_rate: u32,
    groups: Vec<(usize, usize, Option<sbr_history::Frame>)>,
    mapping: Vec<usize>,
    couplings: Vec<(u8, Option<sbr_history::Frame>)>,
}
fn render_ssr_sbr(
    mut pcm: AacFrame,
    metadata: SsrPacketMetadata,
    elements: &mut [Option<ElementSbr>],
    core_rate: u32,
    channels: usize,
) -> Result<AacFrame> {
    pcm.duration = metadata.duration;
    let Some(metadata) = metadata.sbr else { return Ok(pcm); };
    if pcm.samples.len() != 1024 * channels {return Err(invalid("SSR SBR requires aligned 1024-sample core PCM"));}
    let mode = if metadata.output_rate == core_rate {sbr_dsp::OutputRate::Core} else {sbr_dsp::OutputRate::Double};
    let samples = if mode == sbr_dsp::OutputRate::Core {1024} else {2048};
    let mut expanded = vec![0.0; samples*channels];
    for (offset,width,frame) in &metadata.groups {
        let state = elements[*offset].get_or_insert_with(||ElementSbr::new(*width));
        if state.width != *width {return Err(invalid("SSR SBR element width changed"));}
        let planar:Vec<Vec<f32>>=(0..*width).map(|c|pcm.samples.chunks_exact(channels).map(|row|row[metadata.mapping[offset+c]]).collect()).collect();
        let refs:Vec<_>=planar.iter().map(Vec::as_slice).collect();
        let rate=core_rate.checked_mul(2).ok_or_else(||invalid("SBR frequency overflow"))?;
        let rendered=if let Some(frame)=frame {state.dsp.process(frame,&refs,rate,16,mode)} else {state.dsp.process_upsampling(&refs,rate,16,mode)}.map_err(|e|invalid(&e.0))?;
        for (c,data) in rendered.iter().enumerate() {
            if data.len()!=samples {return Err(invalid("SSR SBR output length mismatch"));}
            for (i,&sample) in data.iter().enumerate() {
                let value=sample as f32;
                if !value.is_finite() {return Err(invalid("SSR SBR PCM exceeds finite output"));}
                expanded[i*channels+metadata.mapping[offset+c]]=value;
            }
        }
    }
    if metadata.active {pcm.samples=expanded;}
    Ok(pcm)
}
fn render_ssr_sources(
    frame: super::aac_ssr_alignment::AlignedSources,
    metadata: SsrPacketMetadata,
    elements: &mut [Option<ElementSbr>],
    tags: &[u8],
    core_rate: u32,
    channels: usize,
) -> Result<AacFrame> {
    if frame.lanes.len() != channels + tags.len() {
        return Err(invalid("SSR SBR source geometry mismatch"));
    }
    // Upgrading the queue must not turn a still-pending core-only packet into
    // SBR output. Its original chunks/gains survive in the same source lanes.
    if metadata.sbr.is_none() {
        let mut samples = vec![0.0; frame.rows * channels];
        for lane in &frame.lanes {
            let mut row = 0;
            for chunk in lane {
                for &sample in &chunk.samples {
                    if row >= frame.rows {
                        return Err(invalid("SSR core source length mismatch"));
                    }
                    for gain in &chunk.outputs {
                        let slot = &mut samples[row * channels + gain.channel];
                        *slot += sample * gain.gain;
                        if !slot.is_finite() {
                            return Err(invalid("AAC coupled PCM exceeds finite f32 output"));
                        }
                    }
                    row += 1;
                }
            }
            if row != frame.rows {
                return Err(invalid("SSR core source length mismatch"));
            }
        }
        return Ok(AacFrame {
            samples,
            pts: frame.stamp as i64,
            duration: metadata.duration,
        });
    }
    if frame.rows != 1024 {
        return Err(invalid("SSR SBR source geometry mismatch"));
    }
    let extension = metadata
        .sbr
        .as_ref()
        .ok_or_else(|| invalid("SSR separated sources require SBR metadata"))?;
    let couplings = extension.couplings.clone();
    let output_rate = extension.output_rate;
    let active = extension.active;
    let mut core = vec![0.0; 1024 * channels];
    for channel in 0..channels {
        let samples: Vec<_> = frame.lanes[channel]
            .iter()
            .flat_map(|c| c.samples.iter().copied())
            .collect();
        if samples.len() != 1024 {
            return Err(invalid("SSR SBR source length mismatch"));
        }
        for (row, sample) in samples.into_iter().enumerate() {
            core[row * channels + channel] = sample;
        }
    }
    let mut output = render_ssr_sbr(
        AacFrame {
            samples: core,
            pts: frame.stamp as i64,
            duration: 0,
        },
        metadata,
        elements,
        core_rate,
        channels,
    )?;
    let mode = if output_rate == core_rate {
        sbr_dsp::OutputRate::Core
    } else {
        sbr_dsp::OutputRate::Double
    };
    let dsp_ratio = if mode == sbr_dsp::OutputRate::Core {
        1
    } else {
        2
    };
    let ratio = if active {dsp_ratio} else {1};
    let rate = core_rate
        .checked_mul(2)
        .ok_or_else(|| invalid("SBR frequency overflow"))?;
    for (tag, syntax) in couplings {
        let index = tags
            .iter()
            .position(|v| *v == tag)
            .ok_or_else(|| invalid("SSR SBR coupling source is absent"))?;
        let chunks = &frame.lanes[channels + index];
        let core: Vec<_> = chunks
            .iter()
            .flat_map(|c| c.samples.iter().copied())
            .collect();
        if core.len() != 1024 {
            return Err(invalid("SSR SBR coupling source length mismatch"));
        }
        let state = elements[channels + usize::from(tag)].get_or_insert_with(|| ElementSbr::new(1));
        if state.width != 1 {
            return Err(invalid("SSR SBR coupling width changed"));
        }
        let rendered = if let Some(syntax) = syntax {
            state.dsp.process(&syntax, &[&core], rate, 16, mode)
        } else {
            state.dsp.process_upsampling(&[&core], rate, 16, mode)
        }
        .map_err(|e| invalid(&e.0))?;
        if rendered.len() != 1 || rendered[0].len() != 1024 * dsp_ratio {
            return Err(invalid("SSR SBR coupling output length mismatch"));
        }
        let mut row = 0;
        for chunk in chunks {
            for _ in 0..chunk.samples.len() * ratio {
                let sample = if active {rendered[0][row] as f32} else {core[row]};
                if !sample.is_finite() {
                    return Err(invalid("SSR SBR coupling PCM exceeds finite output"));
                }
                for gain in &chunk.outputs {
                    let value = &mut output.samples[row * channels + gain.channel];
                    *value += sample * gain.gain;
                    if !value.is_finite() {
                        return Err(invalid("AAC coupled PCM exceeds finite f32 output"));
                    }
                }
                row += 1;
            }
        }
    }
    Ok(output)
}

/// PCM retains the timing of its source packet, including delayed SSR output.
#[derive(Debug, PartialEq)]
pub struct AacFrame {
    pub samples: Vec<f32>,
    pub pts: i64,
    pub duration: u64,
}
pub struct NativeAacDecoder {
    config: AacConfig,
    synthesis: Vec<LongSineSynthesis>,
    ltp_synthesis: Vec<super::aac_ltp_channel::LtpChannel>,
    ltp_coupling_synthesis: Vec<Option<super::aac_ltp_channel::LtpChannel>>,
    ssr_synthesis: Vec<super::aac_ssr_synthesis::SsrSynthesis>,
    ssr_coupling_synthesis: Vec<Option<Box<super::aac_ssr_synthesis::SsrSynthesis>>>,
    ssr_alignment: Option<super::aac_ssr_alignment::SsrPcmAlignment>,
    ssr_alignment_tags: Vec<u8>,
    ssr_pending_duration: super::aac_ssr_metadata::PacketQueue<SsrPacketMetadata>,
    ssr_fixed_clock: bool,
    ssr_source_alignment: bool,
    coupling_synthesis: Vec<Option<LongSineSynthesis>>,
    main_prediction: Vec<Option<super::aac_main_predictor::MainPredictor>>,
    noise: NoiseState,
    initial_program: Option<super::aac_pce::ProgramConfig>,
    program: Option<super::aac_pce::ProgramConfig>,
    mapping: Vec<usize>,
    channel_mask: u32,
    sbr_rate: Option<u32>,
    detect_sbr: bool,
    sbr_detection_rate: Option<u32>,
    sbr_elements: Vec<Option<ElementSbr>>,
}
/// Opaque complete packet-boundary state. Configuration and layout are retained
/// to reject restoring into a decoder with another configuration. The caller
/// must associate checkpoints with the corresponding stream and packet cursor.
#[derive(Clone)]
pub struct AacCheckpoint {
    config: AacConfig,
    initial_program: Option<super::aac_pce::ProgramConfig>,
    program: Option<super::aac_pce::ProgramConfig>,
    synthesis: Vec<LongSineSynthesis>,
    ltp_synthesis: Vec<super::aac_ltp_channel::LtpChannelCheckpoint>,
    ltp_coupling_synthesis: Vec<Option<super::aac_ltp_channel::LtpChannelCheckpoint>>,
    ssr_synthesis: Vec<super::aac_ssr_synthesis::SsrSynthesis>,
    ssr_coupling_synthesis: Vec<Option<Box<super::aac_ssr_synthesis::SsrSynthesis>>>,
    ssr_alignment: Option<super::aac_ssr_alignment::SsrPcmAlignment>,
    ssr_alignment_tags: Vec<u8>,
    ssr_pending_duration: super::aac_ssr_metadata::PacketQueue<SsrPacketMetadata>,
    ssr_fixed_clock: bool,
    ssr_source_alignment: bool,
    coupling_synthesis: Vec<Option<LongSineSynthesis>>,
    main_prediction: Vec<Option<super::aac_main_predictor::MainPredictor>>,
    noise: NoiseState,
    mapping: Vec<usize>,
    channel_mask: u32,
    sbr_rate: Option<u32>,
    detect_sbr: bool,
    sbr_detection_rate: Option<u32>,
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
        Self::from_parsed(AudioSpecificConfig::parse(asc)?)
    }
    fn from_parsed(parsed: AudioSpecificConfig) -> Result<Self> {
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
        let synthesis = if matches!(config.object_type,3|4) { Vec::new() } else { vec![
            LongSineSynthesis::new(config.frame_samples as usize).map_err(Error::from)?;
            usize::from(config.channels)
        ] };
        let ltp_synthesis = if config.object_type==4 {
            vec![super::aac_ltp_channel::LtpChannel::new(config.frame_samples as usize)?; usize::from(config.channels)]
        } else { Vec::new() };
        let ltp_coupling_synthesis = if config.object_type==4 {vec![None;16]} else {Vec::new()};
        let ssr_synthesis = if config.object_type == 3 { vec![super::aac_ssr_synthesis::SsrSynthesis::new()?; usize::from(config.channels)] } else { Vec::new() };
        // Four-bit CCE tags occupy a separate fixed domain after audio slots.
        let element_slots = usize::from(config.channels) + if program.is_some() { 16 } else { 0 };
        let ssr_coupling_synthesis = if config.object_type==3 {vec![None;16]} else {Vec::new()};
        let main_prediction = if config.object_type == 1 {
            let tables = BandTables::for_config(&config)?;
            let lines = tables.long[tables.prediction_limit.unwrap()];
            let bank = super::aac_main_predictor::MainPredictor::new(lines)?;
            let mut banks = vec![Some(bank); usize::from(config.channels)];
            banks.extend((0..16).map(|_| None)); banks
        } else { Vec::new() };
        Ok(Self {
            main_prediction,
            config,
            synthesis, ssr_synthesis, ltp_synthesis, ltp_coupling_synthesis,
            ssr_coupling_synthesis, ssr_alignment:None, ssr_alignment_tags:Vec::new(), ssr_pending_duration:Default::default(), ssr_fixed_clock:false,ssr_source_alignment:false,
            coupling_synthesis:vec![None;16],
            noise: NoiseState::default(),
            initial_program: program.clone(),
            program,
            mapping,
            channel_mask,
            sbr_rate, detect_sbr:false,sbr_detection_rate:None,
            sbr_elements: if sbr_rate.is_some() { vec![None; element_slots] } else { Vec::new() },
        })
    }
    /// Container-declared output clock can identify implicit dual-rate SBR.
    /// No silent rate guessing: unhinted ADTS and explicit SBR=false stay strict.
    /// Decode at a fixed container clock. Unspecified SBR presence may be
    /// discovered from FIL even when the output clock equals the core rate;
    /// discovery must not change this hint. Explicit disable flags are honored.
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
        } else if parsed.sbr_present.is_none()
            && matches!(parsed.core.object_type,1|2|3|4) && sbr_layout(&parsed)? {
            // A fixed core-rate hint does not mean SBR is absent. Keep the
            // negotiated clock while admitting a valid implicit SBR FIL.
            decoder.detect_sbr=true;
            decoder.sbr_detection_rate=Some(output_rate);
            decoder.sbr_elements=vec![None;decoder.sbr_slots()];
        }
        Ok(decoder)
    }
    /// Discover dual-rate SBR from a valid FIL when ASC leaves SBR unspecified.
    /// The output clock may change at that packet; callers must negotiate it
    /// before publishing PCM. Fixed-clock container callers use output hints.
    pub fn new_with_sbr_detection(asc:&[u8]) -> Result<Self> {
        let parsed=AudioSpecificConfig::parse(asc)?;
        let mut decoder=Self::new(asc)?;
        decoder.detect_sbr=matches!(parsed.core.object_type, 1 | 2 | 3 | 4) && parsed.sbr_present.is_none() && sbr_layout(&parsed)?;
        if decoder.detect_sbr { decoder.sbr_elements=vec![None; decoder.sbr_slots()]; }

        Ok(decoder)
    }
    fn sbr_slots(&self) -> usize {
        usize::from(self.config.channels) + if self.program.is_some() { 16 } else { 0 }
    }
    pub fn sample_rate(&self) -> u32 {
        self.sbr_rate.unwrap_or(self.config.sample_rate)
    }
    pub fn core_frame_samples(&self) -> u16 { self.config.frame_samples }
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
        AacCheckpoint {main_prediction:self.main_prediction.clone(),config:self.config.clone(),initial_program:self.initial_program.clone(),program:self.program.clone(),
            synthesis:self.synthesis.clone(),ltp_coupling_synthesis:self.ltp_coupling_synthesis.iter().map(|s|s.as_ref().map(super::aac_ltp_channel::LtpChannel::checkpoint)).collect(),ltp_synthesis:self.ltp_synthesis.iter().map(super::aac_ltp_channel::LtpChannel::checkpoint).collect(),ssr_synthesis:self.ssr_synthesis.clone(),ssr_coupling_synthesis:self.ssr_coupling_synthesis.clone(),ssr_alignment:self.ssr_alignment.clone(),ssr_alignment_tags:self.ssr_alignment_tags.clone(),ssr_pending_duration:self.ssr_pending_duration.clone(),ssr_fixed_clock:self.ssr_fixed_clock,ssr_source_alignment:self.ssr_source_alignment,coupling_synthesis:self.coupling_synthesis.clone(),noise:self.noise.clone(),
            mapping:self.mapping.clone(),channel_mask:self.channel_mask,
            sbr_rate:self.sbr_rate,detect_sbr:self.detect_sbr,sbr_detection_rate:self.sbr_detection_rate,sbr_elements:self.sbr_elements.clone()}
    }
    /// Restore without changing the decoder if configuration/layout differs.
    pub fn restore(&mut self, state:&AacCheckpoint) -> Result<()> {
        if self.config!=state.config || self.initial_program!=state.initial_program || self.mapping!=state.mapping || self.channel_mask!=state.channel_mask || self.detect_sbr!=state.detect_sbr || self.sbr_detection_rate!=state.sbr_detection_rate || (!self.detect_sbr && self.sbr_rate!=state.sbr_rate) {
            return Err(invalid("AAC checkpoint configuration mismatch"));
        }
        if self.ltp_synthesis.len()!=state.ltp_synthesis.len() { return Err(invalid("AAC LTP checkpoint channel count mismatch")); }
        for (channel,saved) in self.ltp_synthesis.iter_mut().zip(&state.ltp_synthesis) {channel.restore(saved)?;}
        if self.ltp_coupling_synthesis.len()!=state.ltp_coupling_synthesis.len() {return Err(invalid("AAC LTP checkpoint coupling count mismatch"));}
        for (channel,saved) in self.ltp_coupling_synthesis.iter_mut().zip(&state.ltp_coupling_synthesis) {
            if let Some(saved)=saved {
                if channel.is_none() {let mut fresh=self.ltp_synthesis[0].clone();fresh.reset();*channel=Some(fresh);}
                channel.as_mut().unwrap().restore(saved)?;
            } else {*channel=None;}
        }
        self.ssr_alignment=state.ssr_alignment.clone();self.ssr_alignment_tags=state.ssr_alignment_tags.clone();self.ssr_pending_duration=state.ssr_pending_duration.clone();self.ssr_fixed_clock=state.ssr_fixed_clock;self.ssr_source_alignment=state.ssr_source_alignment;
        self.program=state.program.clone();
        self.main_prediction=state.main_prediction.clone();
        self.synthesis=state.synthesis.clone();self.ssr_synthesis=state.ssr_synthesis.clone();self.ssr_coupling_synthesis=state.ssr_coupling_synthesis.clone();self.coupling_synthesis=state.coupling_synthesis.clone();self.noise=state.noise.clone();self.sbr_rate=state.sbr_rate;self.sbr_elements=state.sbr_elements.clone();
        Ok(())
    }
    pub fn reset(&mut self) {
        self.program=self.initial_program.clone();
        self.ssr_alignment=None; self.ssr_alignment_tags.clear(); self.ssr_pending_duration=Default::default();self.ssr_fixed_clock=false;self.ssr_source_alignment=false;
        for synth in &mut self.synthesis {
            synth.reset();
        }
        for channel in &mut self.ltp_synthesis {channel.reset();}
        self.ltp_coupling_synthesis.fill(None);
        for synthesis in self.coupling_synthesis.iter_mut().flatten() {synthesis.reset();}
        for state in &mut self.ssr_synthesis { state.reset(); }
        for state in self.ssr_coupling_synthesis.iter_mut().flatten() {state.reset();}
        for bank in self.main_prediction.iter_mut().flatten() { bank.reset(); }
        self.noise.reset();
        if self.detect_sbr {self.sbr_rate=None;}
        self.sbr_elements.fill(None);
    }
    /// One raw_data_block, returning interleaved normalized floating PCM.
    /// A malformed/unsupported packet leaves all decoding state unchanged.
    /// Untimed convenience API. An empty vector means one consumed delayed
    /// packet; callers must drain `finish` at EOF. Timed callers use decode_timed.
    pub fn decode(&mut self, packet: &[u8]) -> Result<Vec<f32>> {
        Ok(self
            .decode_timed(packet, 0, 0)?
            .map_or_else(Vec::new, |f| f.samples))
    }
    pub fn delayed(&self) -> bool {
        self.ssr_alignment.is_some()
    }
    pub fn finish(&mut self) -> Result<Option<AacFrame>> {
        let Some(mut alignment) = self.ssr_alignment.clone() else {return Ok(None);};
        let mut pending=self.ssr_pending_duration.clone();
        let mut elements=self.sbr_elements.clone();
        let output=if self.ssr_source_alignment {
            let Some(frame)=alignment.finish_sources()? else {self.ssr_alignment=Some(alignment);return Ok(None);};
            let metadata=pending.take(frame.stamp as i64)?;
            render_ssr_sources(frame,metadata,&mut elements,&self.ssr_alignment_tags,self.config.sample_rate,usize::from(self.config.channels))?
        } else {
            let Some(frame)=alignment.finish()? else {self.ssr_alignment=Some(alignment);return Ok(None);};
            let metadata=pending.take(frame.stamp as i64)?;
            render_ssr_sbr(AacFrame {samples:frame.samples,pts:frame.stamp as i64,duration:0},metadata,&mut elements,self.config.sample_rate,usize::from(self.config.channels))?
        };
        self.ssr_alignment=Some(alignment);self.ssr_pending_duration=pending;self.sbr_elements=elements;
        Ok(Some(output))
    }
    pub fn decode_timed(&mut self, packet: &[u8], pts:i64, duration:u64) -> Result<Option<AacFrame>> {
        let mut ltp_coupling_synthesis=self.ltp_coupling_synthesis.clone();
        let mut bits = BitReader::new(packet);
        let mut current_program = self.program.clone();
        let mut main_prediction = self.main_prediction.clone();
        let mut noise = self.noise.clone();
        let mut sbr_rate = self.sbr_rate;
        let mut sbr_elements = self.sbr_elements.clone();
        let mut sbr_frames = vec![None; self.sbr_slots()];
        let mut previous_element = None;
        let mut channels = Vec::new();
        let mut ltp_data=if self.config.object_type==4 {vec![None;usize::from(self.config.channels)]} else {Vec::new()};
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
            // ER-LC omits element IDs and END: the configured layout fixes
            // element kinds and count. Each element still carries four tag bits.
            let er = self.config.object_type == 17;
            let element = if er {elements.get(element_index).copied().unwrap_or(7)} else {bits.read(3)?};
            let mut target_offset = channels.len();
            if matches!(element, 0 | 1 | 3) {
                if current_program.is_none() && elements.get(element_index) != Some(&element) {
                    return Err(unsupported(
                        "AAC element order differs from standard layout",
                    ));
                }
                element_index += 1;
                let signaled_tag = bits.read(4)?;
                let tag = if er {element_index as u32} else {signaled_tag};
                if let Some(program) = &current_program {
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
                    let channel = if self.config.object_type==4 {
                        let (channel,data)=ChannelData::read_ltp(&mut bits,&self.config)?;
                        ltp_data[self.mapping[target_offset]]=data;channel
                    } else {ChannelData::read(&mut bits,&self.config)?};
                    let mut spectrum = channel.spectrum_with_noise(&self.config, &mut noise)?;
                    if self.config.object_type == 1 {
                        channel.predict_main(&self.config, main_prediction[self.mapping[target_offset]].as_mut().unwrap(), &mut spectrum)?;
                    }
                    channels.push((channel, spectrum, self.mapping[target_offset]));
                }
                1 => {
                    let pair = if self.config.object_type==4 {
                        let (pair,data)=ChannelPair::read_ltp(&mut bits,&self.config)?;
                        let [left,right]=data;
                        ltp_data[self.mapping[target_offset]]=left;
                        ltp_data[self.mapping[target_offset+1]]=right;pair
                    } else {ChannelPair::read(&mut bits,&self.config)?};
                    let (left, right) = if self.config.object_type == 1 {
                        let l = self.mapping[target_offset]; let r = self.mapping[target_offset+1];
                        let (left_bank, right_bank) = if l < r {
                            let (before, after) = main_prediction.split_at_mut(r); (before[l].as_mut().unwrap(), after[0].as_mut().unwrap())
                        } else {
                            let (before, after) = main_prediction.split_at_mut(l); (after[0].as_mut().unwrap(), before[r].as_mut().unwrap())
                        };
                        pair.spectra_with_main_prediction(&self.config, &mut noise, left_bank, right_bank)?
                    } else { pair.spectra_with_noise(&self.config, &mut noise)? };
                    channels.push((pair.left, left, self.mapping[target_offset]));
                    channels.push((pair.right, right, self.mapping[target_offset + 1]));
                }
                2 => {
                    let (coupling,prediction)=if self.config.object_type==4 {Coupling::read_ltp(&mut bits,&self.config)?} else {(Coupling::read(&mut bits,&self.config)?,None)};
                    if current_program
                        .as_ref()
                        .is_none_or(|p| !p.coupling.contains(&(coupling.point == 3, coupling.tag)))
                    {
                        return Err(invalid("AAC coupling is absent from configured PCE"));
                    }
                    if !tags.insert(2, u32::from(coupling.tag)) {
                        return Err(invalid("duplicate AAC coupling tag"));
                    }
                    let mut spectrum = coupling
                        .channel
                        .spectrum_with_noise(&self.config, &mut noise)?;
                    if self.config.object_type == 1 {
                        let slot = usize::from(self.config.channels) + usize::from(coupling.tag);
                        if main_prediction[slot].is_none() {
                            let tables = BandTables::for_config(&self.config)?;
                            main_prediction[slot] = Some(super::aac_main_predictor::MainPredictor::new(tables.long[tables.prediction_limit.unwrap()])?);
                        }
                        coupling.channel.predict_main(&self.config, main_prediction[slot].as_mut().unwrap(), &mut spectrum)?;
                    }
                    let spectrum = if self.config.object_type==4 {
                        let slot=&mut ltp_coupling_synthesis[usize::from(coupling.tag)];
                        if slot.is_none() {let mut fresh=self.ltp_synthesis[0].clone();fresh.reset();*slot=Some(fresh);}
                        let tables=BandTables::for_config(&self.config)?;
                        let short=coupling.channel.info.sequence==super::aac_synthesis::WindowSequence::EightShort;
                        let offsets=if short {tables.short}else{tables.long};
                        let limit=BandTables::tns_limit(self.config.sample_rate,short).min(coupling.channel.info.max_sfb as usize);
                        let prediction=prediction.as_ref().map(ltp_data_for_channel);
                        slot.as_mut().unwrap().prepare_spectrum(spectrum,prediction.as_deref(),coupling.channel.info.sequence,coupling.channel.info.shape,offsets,limit,coupling.channel.tns.as_ref())?
                    } else {coupling.channel.apply_tns(&self.config, spectrum)?};
                    couplings.push((coupling, spectrum));
                }
                4 => super::aac_pce::skip_data_stream(&mut bits)?,
                5 => {
                    let program = super::aac_pce::ProgramConfig::read(&mut bits, 0)?;
                    let expected = current_program.as_ref().ok_or_else(|| {
                        unsupported("in-band PCE needs an explicit configured program")
                    })?;
                    if program.elements != expected.elements
                        || program.sample_rate != expected.sample_rate
                        || program.object_type != expected.object_type
                        || program.height_layers()? != expected.height_layers()?
                        || program.pcm_layout()?.0 != self.channel_mask
                    {
                        return Err(invalid("AAC in-band PCE changed the configured layout"));
                    }
                    current_program = Some(program);
                }
                6 => super::aac_pce::read_fill(&mut bits, |input, end, crc| {
                    if sbr_rate.is_none() && self.detect_sbr {
                        sbr_rate=Some(match self.sbr_detection_rate {Some(rate)=>rate,None=>self.config.sample_rate.checked_mul(2).ok_or_else(||invalid("SBR frequency overflow"))?});
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
                    // Dependent CCE remains spectral: retain/validate its FIL
                    // history, but do not synthesize another PCM contribution.
                    if previous_element == Some(2)
                        && couplings.last().is_some_and(|(c, _)| c.point != 3)
                        && frame.syntax.data.extended_data.as_ref().is_some_and(|v| !v.is_empty())
                    {
                        return Err(unsupported("SBR extended audio/PS synthesis is not yet implemented"));
                    }
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
            return Err(invalid(if self.config.object_type == 17 {"trailing bytes after ER AAC block"} else {"trailing bytes after AAC END"}));
        }
        for point in [0, 1] {
            if point == 1 {
                for (channel, spectrum, target) in &mut channels {
                    if self.config.object_type==4 {
                        let tables=BandTables::for_config(&self.config)?;
                        let short=channel.info.sequence==super::aac_synthesis::WindowSequence::EightShort;
                        let offsets=if short {tables.short}else{tables.long};
                        let limit=BandTables::tns_limit(self.config.sample_rate,short).min(channel.info.max_sfb as usize);
                        let prediction=ltp_data[*target].as_ref().map(ltp_data_for_channel);
                        *spectrum=self.ltp_synthesis[*target].prepare_spectrum(std::mem::take(spectrum),prediction.as_deref(),channel.info.sequence,channel.info.shape,offsets,limit,channel.tns.as_ref())?;
                    } else {
                        *spectrum = channel.apply_tns(&self.config, std::mem::take(spectrum))?;
                    }
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
                    if self.config.object_type==3 && channel.info.shape != coupling.channel.info.shape {
                        return Err(invalid("AAC SSR dependent coupling window shape mismatch"));
                    }
                    coupling.mix_spectrum(target, &self.config, source, destination)?;
                }
            }
        }
        // byte_alignment bits have no audio payload.
        let fixed_clock_history=self.ssr_fixed_clock;
        if self.config.object_type==3 && sbr_rate.is_some() {self.ssr_fixed_clock=true;}
        let n = if self.config.object_type == 3 {
            let sequence = channels.iter().find(|(_,_,target)|*target==0).ok_or_else(||invalid("AAC primary output channel is absent"))?.0.info.sequence;
            let samples = super::aac_ssr_synthesis::SsrSynthesis::output_samples(sequence);
            // Standard AAC clocks keep 1024 rows per access unit. SSR window
            // transitions change internal block length, not that packet clock.
            // Duration zero preserves the explicitly untimed synthesis API.
            if duration == u64::from(self.config.frame_samples) && samples != self.config.frame_samples as usize {self.ssr_fixed_clock=true;}
            if self.ssr_fixed_clock {self.config.frame_samples as usize} else {samples}
        } else { self.config.frame_samples as usize };
        let alignment_history=self.ssr_alignment.clone();
        let alignment_tags_history=self.ssr_alignment_tags.clone();
        let pending_duration_history=self.ssr_pending_duration.clone();
        let source_alignment_history=self.ssr_source_alignment;
        let ltp_history:Vec<_>=self.ltp_synthesis.iter().map(super::aac_ltp_channel::LtpChannel::checkpoint).collect();
        let ssr_history = self.ssr_synthesis.clone();
        let ssr_coupling_history = self.ssr_coupling_synthesis.clone();
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
        let result = (|| -> Result<Option<AacFrame>> {
            if self.config.object_type == 3 {
                use super::aac_ssr_alignment::{LaneInput, OutputGain, SsrPcmAlignment};
                use super::aac_ssr_synthesis::SsrSynthesis;
                let mut lanes = vec![Vec::<f32>::new(); channels.len()];
                let mut gains: Vec<Vec<OutputGain>> = (0..channels.len())
                    .map(|channel| vec![OutputGain { channel, gain: 1.0 }])
                    .collect();
                for (channel, spectrum, target) in &channels {
                    let mut pcm = vec![0.0; SsrSynthesis::output_samples(channel.info.sequence)];
                    let gain = channel
                        .gain
                        .clone()
                        .unwrap_or(super::aac_gain_control::GainControl { bands: Vec::new() })
                        .into();
                    self.ssr_synthesis[*target].synthesize_pcm(
                        channel.info.sequence,
                        channel.info.shape,
                        &gain,
                        spectrum,
                        &mut pcm,
                    )?;
                    lanes[*target] = pcm.into_iter().map(|s| s as f32).collect();
                }
                let mut tags = Vec::new();
                // Stable tag order associates queues with source histories even
                // when packet element order changes.
                let mut independent: Vec<_> = couplings.iter().filter(|(c, _)| c.point == 3).collect();
                independent.sort_by_key(|(c, _)| c.tag);
                for (coupling, spectrum) in independent {
                    tags.push(coupling.tag);
                    let state = &mut self.ssr_coupling_synthesis[coupling.tag as usize];
                    if state.is_none() {
                        *state = Some(Box::new(SsrSynthesis::new()?));
                    }
                    let mut pcm = vec![0.0; SsrSynthesis::output_samples(coupling.channel.info.sequence)];
                    let gain = coupling
                        .channel
                        .gain
                        .clone()
                        .unwrap_or(super::aac_gain_control::GainControl { bands: Vec::new() })
                        .into();
                    state.as_mut().unwrap().synthesize_pcm(
                        coupling.channel.info.sequence,
                        coupling.channel.info.shape,
                        &gain,
                        spectrum,
                        &mut pcm,
                    )?;
                    lanes.push(pcm.into_iter().map(|s| s as f32).collect());
                    let mut outputs = Vec::new();
                    for target in &coupling.targets {
                        let (_, _, offset) = decoded_elements
                            .iter()
                            .find(|(k, t, _)| *k == u32::from(target.pair) && *t == u32::from(target.tag))
                            .ok_or_else(|| invalid("AAC coupling target is absent"))?;
                        outputs.push(OutputGain {
                            channel: self.mapping[*offset + target.channel as usize],
                            gain: target.gain,
                        });
                    }
                    gains.push(outputs);
                }
                // Keep established source lanes when a CCE is absent. Their
                // queued PCM/gains precede the explicitly silent missing input.
                // Coded synthesis history stays keyed by tag for later return.
                let coded_tags=tags.clone();
                for &tag in &self.ssr_alignment_tags {
                    if !tags.contains(&tag) {tags.push(tag);}
                }
                tags.sort_unstable();
                if tags!=coded_tags {
                    let mut canonical_lanes:Vec<_>=(0..channels.len()).map(|c|std::mem::take(&mut lanes[c])).collect();
                    let mut canonical_gains:Vec<_>=(0..channels.len()).map(|c|std::mem::take(&mut gains[c])).collect();
                    for &tag in &tags {
                        if let Some(index)=coded_tags.iter().position(|v|*v==tag) {
                            canonical_lanes.push(std::mem::take(&mut lanes[channels.len()+index]));
                            canonical_gains.push(std::mem::take(&mut gains[channels.len()+index]));
                        } else {
                            let index=self.ssr_alignment_tags.iter().position(|v|*v==tag).ok_or_else(||invalid("SSR absent source lane is missing"))?;
                            let alignment=self.ssr_alignment.as_ref().ok_or_else(||invalid("SSR absent source alignment is missing"))?;
                            let rows=alignment.absent_input_rows(channels.len()+index,n)?;
                            canonical_lanes.push(vec![0.0;rows]);
                            canonical_gains.push(Vec::new());
                        }
                    }
                    lanes=canonical_lanes;gains=canonical_gains;
                }
                // Timed discovery must warm each unmixed source before the first
                // FIL. Retain the caller's core PCM until SBR becomes active.
                let candidate_rate=sbr_rate.or_else(|| {
                    if self.detect_sbr && (self.ssr_fixed_clock || duration==u64::from(self.config.frame_samples)) {
                        self.sbr_detection_rate.or_else(||self.config.sample_rate.checked_mul(2))
                    } else {None}
                });
                let separate=self.ssr_source_alignment || (candidate_rate.is_some() && !tags.is_empty());
                if separate {
                    // Mixed alignment already retains each unmixed source and
                    // its gains. Upgrade without dropping or rebuilding it.
                    self.ssr_source_alignment=true;
                }
                let metadata=SsrPacketMetadata {duration,sbr:candidate_rate.map(|output_rate|SsrSbrMetadata {
                    active:sbr_rate.is_some(),
                    output_rate, mapping:self.mapping.clone(),
                    groups:decoded_elements.iter().map(|&(kind,_,offset)|(offset,if kind==1 {2} else {1},sbr_frames[offset].clone())).collect(),
                    couplings:tags.iter().map(|&tag|(tag,sbr_frames[channels.len()+usize::from(tag)].clone())).collect(),
                })};
                let needs_alignment = separate || candidate_rate.is_some() || lanes.iter().any(|lane| lane.len() != n);
                if self.ssr_alignment.is_none() && needs_alignment {
                    self.ssr_alignment = Some(SsrPcmAlignment::new(channels.len(), lanes.len())?);
                    self.ssr_alignment_tags = tags.clone();
                    self.ssr_source_alignment=separate;
                }
                if let Some(alignment) = &mut self.ssr_alignment {
                    if tags != self.ssr_alignment_tags {
                        let order:Vec<_>=(0..channels.len()).map(Some).chain(tags.iter().map(|tag|
                            self.ssr_alignment_tags.iter().position(|old|old==tag).map(|index|channels.len()+index)
                        )).collect();
                        alignment.extend_lanes(&order)?;
                        self.ssr_alignment_tags=tags.clone();
                    }
                    let inputs: Vec<_> = lanes
                        .iter()
                        .zip(&gains)
                        .map(|(samples, outputs)| LaneInput { samples, outputs })
                        .collect();
                    self.ssr_pending_duration.push(pts, metadata)?;
                    if separate {
                        let output=alignment.submit_sources(pts as u64,n,&inputs)?;
                        return output.map(|f| {
                            let metadata=self.ssr_pending_duration.take(f.stamp as i64)?;
                            render_ssr_sources(f,metadata,&mut sbr_elements,&self.ssr_alignment_tags,self.config.sample_rate,channels.len())
                        }).transpose();
                    }
                    let output = alignment.submit(pts as u64, n, &inputs)?;
                    return output.map(|f| {
                        let metadata=self.ssr_pending_duration.take(f.stamp as i64)?;
                        render_ssr_sbr(AacFrame {samples:f.samples,pts:f.stamp as i64,duration:0},metadata,&mut sbr_elements,self.config.sample_rate,channels.len())
                    }).transpose();
                }
                let mut samples = vec![0.0; n * channels.len()];
                for (lane, outputs) in lanes.iter().zip(&gains) {
                    for (i, &sample) in lane.iter().enumerate() {
                        for output in outputs {
                            let value = &mut samples[i * channels.len() + output.channel];
                            *value += sample * output.gain;
                            if !value.is_finite() {
                                return Err(invalid("AAC coupled PCM exceeds finite f32 output"));
                            }
                        }
                    }
                }
                return render_ssr_sbr(AacFrame {samples,pts,duration},metadata,&mut sbr_elements,self.config.sample_rate,channels.len()).map(Some);
            }
            let mut output = vec![0.0; n * channels.len()];
            let mut pcm = vec![0.0; n];
            for (channel, spectrum, target) in &channels {
                if self.config.object_type==4 {
                    pcm=self.ltp_synthesis[*target].synthesize_spectrum(spectrum,channel.info.sequence,channel.info.shape)?;
                } else {
                    self.synthesis[*target].synthesize_pcm(
                        channel.info.sequence,channel.info.shape,spectrum,&mut pcm,
                    )?;
                }
                for i in 0..n {
                    output[i * channels.len() + target] = pcm[i] as f32;
                }
            }
            // Warm-up must use the fixed output hint before FIL establishes
            // presence, so discovery preserves the same synthesis history.
            if sbr_rate.is_some() || self.detect_sbr {
                let rate = self.config.sample_rate.checked_mul(2).ok_or_else(|| invalid("SBR frequency overflow"))?;
                let mode = if sbr_rate.or(self.sbr_detection_rate) == Some(self.config.sample_rate) { sbr_dsp::OutputRate::Core } else { sbr_dsp::OutputRate::Double };
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
                if self.config.object_type==4 {
                    pcm=ltp_coupling_synthesis[usize::from(coupling.tag)].as_mut().ok_or_else(||invalid("AAC LTP coupling state missing"))?.synthesize_spectrum(&spectrum,coupling.channel.info.sequence,coupling.channel.info.shape)?;
                } else {
                    let state = &mut self.coupling_synthesis[coupling.tag as usize];
                    if state.is_none() {*state=Some(LongSineSynthesis::new(n)?);}
                    state.as_mut().unwrap().synthesize_pcm(coupling.channel.info.sequence,coupling.channel.info.shape,&spectrum,&mut pcm)?;
                }
                let coupled: Vec<f32> = if sbr_rate.is_some() || self.detect_sbr {
                    let offset = usize::from(self.config.channels) + usize::from(coupling.tag);
                    let state = sbr_elements[offset].get_or_insert_with(|| ElementSbr::new(1));
                    let core: Vec<f32> = pcm.iter().map(|&value| value as f32).collect();
                    let rate = self.config.sample_rate.checked_mul(2).ok_or_else(|| invalid("SBR frequency overflow"))?;
                    let mode = if sbr_rate.or(self.sbr_detection_rate) == Some(self.config.sample_rate) { sbr_dsp::OutputRate::Core } else { sbr_dsp::OutputRate::Double };
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
            Ok(Some(AacFrame {samples:output,pts,duration}))
        })();
        match result {
            Ok(output) => {
                self.ltp_coupling_synthesis=ltp_coupling_synthesis;
                self.program = current_program;
                self.main_prediction = main_prediction;
                self.noise = noise;
                self.sbr_rate = sbr_rate;
                self.sbr_elements = sbr_elements;
                Ok(output)
            }
            Err(error) => {
                for (state,saved) in self.ltp_synthesis.iter_mut().zip(&ltp_history) {state.restore(saved)?;}
                self.ssr_source_alignment=source_alignment_history;self.ssr_fixed_clock=fixed_clock_history;self.ssr_alignment=alignment_history;self.ssr_alignment_tags=alignment_tags_history;self.ssr_pending_duration=pending_duration_history;
                self.ssr_synthesis=ssr_history;self.ssr_coupling_synthesis=ssr_coupling_history;
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
