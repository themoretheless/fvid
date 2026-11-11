//! Owned AAC-LC raw-data-block decoder. The caller supplies ASC and strips any
//! ADTS/container framing. Remaining tools are explicit errors, never fallbacks.
use super::aac_coupling_syntax::Coupling;
use super::aac_geometry::BandTables;
use super::{invalid, unsupported, Result};
fn default_pcm_mask(channels: u16) -> Result<u32> {
    crate::owned_wav::default_pcm_mask(channels).map_err(|e| invalid(&e))
}
use super::Error;
use super::{aac_sbr_history as sbr_history, aac_sbr_dsp as sbr_dsp, bits::BitReader as SbrBitReader};
fn ltp_data_for_channel(data: &super::aac_ltp_syntax::LtpData) -> std::borrow::Cow<'_, super::aac_ltp_syntax::LtpData> {
    std::borrow::Cow::Borrowed(data)
}
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
        let rendered=if let Some(frame)=frame {state.dsp.process(frame,&refs,rate,16,mode)} else {state.dsp.process_upsampling(&refs,rate,16,mode)}?;
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
        }?;
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
    ld_synthesis: Vec<super::aac_ld_channel::LdChannel>,
    eld_synthesis: Vec<super::aac_eld_synthesis::EldSynthesis>,
    eld_sbr: Option<super::aac_eld_sbr::Decoder>,
    protection: Option<super::aac_ep::Config>,
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
    ld_synthesis: Vec<super::aac_ld_channel::LdChannelCheckpoint>,
    eld_synthesis: Vec<super::aac_eld_synthesis::EldHistory>,
    eld_sbr: Option<super::aac_eld_sbr::Decoder>,
    protection: Option<super::aac_ep::Config>,
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
    pub fn from_parsed(parsed: AudioSpecificConfig) -> Result<Self> {
        if parsed.ps_present == Some(true) {
            return Err(unsupported("AAC parametric stereo synthesis is not yet implemented"));
        }
        if let Some(ep)=&parsed.protection {ep.validate_raw_access_unit()?;}
        let protection=parsed.protection.clone();
        let eld_sbr=parsed.eld_sbr.clone().map(|config|super::aac_eld_sbr::Decoder::new(config,parsed.core.sample_rate,parsed.core.frame_samples,parsed.core.channel_configuration)).transpose()?;
        let sbr_rate = if parsed.sbr_present == Some(true) && eld_sbr.is_none() {
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
        let synthesis = if matches!(config.object_type,3|4|19|23|39) { Vec::new() } else { vec![
            LongSineSynthesis::new(config.frame_samples as usize).map_err(Error::from)?;
            usize::from(config.channels)
        ] };
        let eld_synthesis=if config.object_type==39 {vec![super::aac_eld_synthesis::EldSynthesis::new(config.frame_samples as usize)?;usize::from(config.channels)]}else{Vec::new()};
        let ld_synthesis=if config.object_type==23 {vec![super::aac_ld_channel::LdChannel::new(config.frame_samples as usize)?;usize::from(config.channels)]}else{Vec::new()};
        let ltp_synthesis = if matches!(config.object_type,4|19) {
            vec![super::aac_ltp_channel::LtpChannel::new(config.frame_samples as usize)?; usize::from(config.channels)]
        } else { Vec::new() };
        let ltp_coupling_synthesis = if matches!(config.object_type,4|19) {vec![None;16]} else {Vec::new()};
        let ssr_synthesis = if config.object_type == 3 { vec![super::aac_ssr_synthesis::SsrSynthesis::new()?; usize::from(config.channels)] } else { Vec::new() };
        // Four-bit CCE tags occupy a separate fixed domain after audio slots.
        let element_slots = usize::from(config.channels) + 16;
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
            protection, eld_sbr, eld_synthesis, ld_synthesis, synthesis, ssr_synthesis, ltp_synthesis, ltp_coupling_synthesis,
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
            && matches!(parsed.core.object_type,1|2|3|4|17|19) && sbr_layout(&parsed)? {
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
        decoder.detect_sbr=matches!(parsed.core.object_type, 1 | 2 | 3 | 4 | 17 | 19) && parsed.sbr_present.is_none() && sbr_layout(&parsed)?;
        if decoder.detect_sbr { decoder.sbr_elements=vec![None; decoder.sbr_slots()]; }

        Ok(decoder)
    }
    fn sbr_slots(&self) -> usize {
        usize::from(self.config.channels) + 16
    }
    pub fn sample_rate(&self) -> u32 {
        self.eld_sbr.as_ref().map_or(self.sbr_rate.unwrap_or(self.config.sample_rate),|ld|ld.output_rate())
    }
    pub fn core_frame_samples(&self) -> u16 { self.config.frame_samples }
    /// Packet/container adapters retain the configured SSR access-unit clock;
    /// window-sequence row counts are internal synthesis geometry.
    pub fn use_fixed_packet_clock(&mut self) {
        if self.config.object_type==3 {self.ssr_fixed_clock=true;}
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
        AacCheckpoint {protection:self.protection.clone(),main_prediction:self.main_prediction.clone(),config:self.config.clone(),initial_program:self.initial_program.clone(),program:self.program.clone(),
            eld_synthesis:self.eld_synthesis.iter().map(super::aac_eld_synthesis::EldSynthesis::checkpoint).collect(),eld_sbr:self.eld_sbr.clone(),
            ld_synthesis:self.ld_synthesis.iter().map(super::aac_ld_channel::LdChannel::checkpoint).collect(),
            synthesis:self.synthesis.clone(),ltp_coupling_synthesis:self.ltp_coupling_synthesis.iter().map(|s|s.as_ref().map(super::aac_ltp_channel::LtpChannel::checkpoint)).collect(),ltp_synthesis:self.ltp_synthesis.iter().map(super::aac_ltp_channel::LtpChannel::checkpoint).collect(),ssr_synthesis:self.ssr_synthesis.clone(),ssr_coupling_synthesis:self.ssr_coupling_synthesis.clone(),ssr_alignment:self.ssr_alignment.clone(),ssr_alignment_tags:self.ssr_alignment_tags.clone(),ssr_pending_duration:self.ssr_pending_duration.clone(),ssr_fixed_clock:self.ssr_fixed_clock,ssr_source_alignment:self.ssr_source_alignment,coupling_synthesis:self.coupling_synthesis.clone(),noise:self.noise.clone(),
            mapping:self.mapping.clone(),channel_mask:self.channel_mask,
            sbr_rate:self.sbr_rate,detect_sbr:self.detect_sbr,sbr_detection_rate:self.sbr_detection_rate,sbr_elements:self.sbr_elements.clone()}
    }
    /// Restore without changing the decoder if configuration/layout differs.
    pub fn restore(&mut self, state:&AacCheckpoint) -> Result<()> {
        if self.protection!=state.protection {return Err(invalid("AAC EP checkpoint configuration mismatch"));}
        if self.config!=state.config || self.initial_program!=state.initial_program || self.mapping!=state.mapping || self.channel_mask!=state.channel_mask || self.detect_sbr!=state.detect_sbr || self.sbr_detection_rate!=state.sbr_detection_rate || (!self.detect_sbr && self.sbr_rate!=state.sbr_rate) {
            return Err(invalid("AAC checkpoint configuration mismatch"));
        }
        if self.eld_sbr.as_ref().map(|s|s.initial_config())!=state.eld_sbr.as_ref().map(|s|s.initial_config()) {return Err(invalid("AAC ELD low-delay SBR checkpoint configuration mismatch"));}
        if self.eld_synthesis.len()!=state.eld_synthesis.len(){return Err(invalid("AAC ELD checkpoint channel count mismatch"));}
        for (channel,saved)in self.eld_synthesis.iter_mut().zip(&state.eld_synthesis){channel.restore(saved)?;}
        if self.ld_synthesis.len()!=state.ld_synthesis.len() {return Err(invalid("AAC LD checkpoint channel count mismatch"));}
        for (channel,saved) in self.ld_synthesis.iter_mut().zip(&state.ld_synthesis) {channel.restore(saved)?;}
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
        self.eld_sbr=state.eld_sbr.clone();
        self.program=state.program.clone();
        self.main_prediction=state.main_prediction.clone();
        self.synthesis=state.synthesis.clone();self.ssr_synthesis=state.ssr_synthesis.clone();self.ssr_coupling_synthesis=state.ssr_coupling_synthesis.clone();self.coupling_synthesis=state.coupling_synthesis.clone();self.noise=state.noise.clone();self.sbr_rate=state.sbr_rate;self.sbr_elements=state.sbr_elements.clone();
        Ok(())
    }
    pub fn reset(&mut self) {
        self.program=self.initial_program.clone();
        self.ssr_alignment=None; self.ssr_alignment_tags.clear(); self.ssr_pending_duration=Default::default();self.ssr_fixed_clock=false;self.ssr_source_alignment=false;
        if let Some(ld)=&mut self.eld_sbr {ld.reset();}
        for synth in &mut self.synthesis {
            synth.reset();
        }
        for channel in &mut self.ld_synthesis {channel.reset();}
        for channel in &mut self.eld_synthesis {channel.reset();}
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
        self.decode_timed_in_clock(packet, pts, duration, self.config.sample_rate)
    }
    /// Preserve source timestamps/durations while interpreting the SSR packet
    /// window in its declared clock. Zero duration retains untimed synthesis.
    pub fn decode_timed_in_clock(&mut self, packet: &[u8], pts:i64, duration:u64, source_timescale:u32) -> Result<Option<AacFrame>> {
        if source_timescale == 0 {return Err(invalid("zero AAC source timescale"));}
        let recovered=self.protection.as_ref().map(|ep|ep.decode_raw_access_unit(packet)).transpose()?;
        let packet=recovered.as_deref().unwrap_or(packet);
        if self.config.object_type==39 {return self.decode_eld_timed(packet,pts,duration);}
        if self.config.object_type==23 {return self.decode_ld_timed(packet,pts,duration);}
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
        let mut ltp_data=if matches!(self.config.object_type,4|19) {vec![None;usize::from(self.config.channels)]} else {Vec::new()};
        let mut decoded_elements = Vec::new();
        let mut couplings = Vec::new();
        let pce_elements: Vec<u32> = current_program
            .as_ref()
            .filter(|_| self.config.channel_configuration == 0)
            .map(|p| {
                p.elements
                    .iter()
                    .map(|e| {
                        if e.position == super::aac_pce::Position::Lfe {
                            3
                        } else {
                            u32::from(e.pair)
                        }
                    })
                    .collect()
            })
            .unwrap_or_default();
        let elements: &[u32] = match self.config.channel_configuration {
            0 => &pce_elements,
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
            // ER AAC omits element IDs and END: the configured layout fixes
            // element kinds and count. Each element still carries four tag bits.
            let er = matches!(self.config.object_type,17|19);
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
                let tag = if er && current_program.is_none() {element_index as u32} else {signaled_tag};
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
                    let channel = if matches!(self.config.object_type,4|19) {
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
                    let pair = if matches!(self.config.object_type,4|19) {
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
                    let (coupling,prediction)=if matches!(self.config.object_type,4|19) {Coupling::read_ltp(&mut bits,&self.config)?} else {(Coupling::read(&mut bits,&self.config)?,None)};
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
                    let spectrum = if matches!(self.config.object_type,4|19) {
                        let slot=&mut ltp_coupling_synthesis[usize::from(coupling.tag)];
                        if slot.is_none() {let mut fresh=self.ltp_synthesis[0].clone();fresh.reset();*slot=Some(fresh);}
                        let tables=BandTables::for_config(&self.config)?;
                        let short=coupling.channel.info.sequence==super::aac_synthesis::WindowSequence::EightShort;
                        let offsets=if short {tables.short}else{tables.long};
                        let limit=BandTables::tns_limit(self.config.sample_rate,short).min(coupling.channel.info.max_sfb as usize);
                        let mut prediction=prediction;
                        if let Some(data)=&mut prediction {
                            if let super::aac_ltp_syntax::Usage::Bands(used)=&mut data.usage {coupling.channel.suppress_ltp_noise(used);}
                        }
                        let prediction=prediction.as_ref().map(ltp_data_for_channel);
                        slot.as_mut().unwrap().prepare_spectrum(spectrum,prediction.as_deref(),coupling.channel.info.sequence,coupling.channel.info.shape,offsets,limit,coupling.channel.tns.as_ref())?
                    } else {coupling.channel.apply_tns(&self.config, spectrum)?};
                    couplings.push((coupling, spectrum));
                }
                4 => super::aac_pce::skip_data_stream(&mut bits)?,
                5 => {
                    let program = super::aac_pce::ProgramConfig::read(&mut bits, 0)?;
                    if let Some(expected) = current_program.as_ref() {
                        if program.elements != expected.elements
                            || program.sample_rate != expected.sample_rate
                            || program.object_type != expected.object_type
                            || program.height_layers()? != expected.height_layers()?
                            || program.pcm_layout()?.0 != self.channel_mask
                        {
                            return Err(invalid("AAC in-band PCE changed the configured layout"));
                        }
                    } else {
                        // An indexed ASC already fixes PCM geometry. A matching
                        // PCE may name its elements without rebuilding histories.
                        let layout = program.pcm_layout()?;
                        let kinds = program.elements.iter().map(|element| {
                            if element.position == super::aac_pce::Position::Lfe {
                                3
                            } else { u32::from(element.pair) }
                        });
                        if u32::from(program.object_type) != self.config.object_type
                            || program.sample_rate != self.config.sample_rate
                            || program.channels() != usize::from(self.config.channels)
                            || layout.0 != self.channel_mask
                            || layout.1 != self.mapping
                            || !kinds.eq(elements.iter().copied())
                        {
                            return Err(invalid("AAC in-band PCE changed the configured layout"));
                        }
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
                    source.skip(input.position())?;
                    let rate = self.config.sample_rate.checked_mul(2).ok_or_else(|| invalid("SBR frequency overflow"))?;
                    let frame = state.stream.read(&mut source, end, crc, rate, (self.config.frame_samples/64) as u8, width)?;
                    // Dependent CCE remains spectral: retain/validate its FIL
                    // history, but do not synthesize another PCM contribution.
                    if previous_element == Some(2)
                        && couplings.last().is_some_and(|(c, _)| c.point != 3)
                        && frame.syntax.data.has_ps_extension()
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
        if matches!(self.config.object_type,17|19) {
            let groups:Vec<_>=decoded_elements.iter().filter(|(kind,_,_)|*kind!=3).copied().collect();
            let mut index=0;
            super::aac_pce::read_er_extensions(&mut bits, |input,end,_| {
                if sbr_rate.is_none() && self.detect_sbr {
                    sbr_rate=Some(self.sbr_detection_rate.unwrap_or(self.config.sample_rate.checked_mul(2).ok_or_else(||invalid("SBR frequency overflow"))?));
                }
                if sbr_rate.is_none() {return Err(unsupported("ER AAC SBR requires extension-aware stream signalling"));}
                let &(kind,_,offset)=groups.get(index).ok_or_else(||invalid("excess ER AAC SBR element"))?;
                let width=if kind==1 {2}else{1};
                let state=sbr_elements[offset].get_or_insert_with(||ElementSbr::new(width));
                if state.width!=width {return Err(invalid("SBR element width changed"));}
                let mut source=SbrBitReader::new(packet);
                source.skip(input.position())?;
                let rate=self.config.sample_rate.checked_mul(2).ok_or_else(||invalid("SBR frequency overflow"))?;
                let frame=state.stream.read(&mut source,end,false,rate,(self.config.frame_samples/64) as u8,width)?;
                input.skip(source.position()-input.position())?;
                sbr_frames[offset]=Some(frame);index+=1;
                Ok(())
            })?;
            if index!=0 && index!=groups.len() {return Err(invalid("missing ER AAC SBR element"));}
        }
        if bits.remaining() > 7 {
            return Err(invalid(if matches!(self.config.object_type,17|19) {"trailing bytes after ER AAC block"} else {"trailing bytes after AAC END"}));
        }
        for point in [0, 1] {
            if point == 1 {
                for (channel, spectrum, target) in &mut channels {
                    if matches!(self.config.object_type,4|19) {
                        let tables=BandTables::for_config(&self.config)?;
                        let short=channel.info.sequence==super::aac_synthesis::WindowSequence::EightShort;
                        let offsets=if short {tables.short}else{tables.long};
                        let limit=BandTables::tns_limit(self.config.sample_rate,short).min(channel.info.max_sfb as usize);
                        if let Some(data)=&mut ltp_data[*target] {
                            if let super::aac_ltp_syntax::Usage::Bands(used)=&mut data.usage {channel.suppress_ltp_noise(used);}
                        }
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
            let ticks = u128::from(self.config.frame_samples) * u128::from(source_timescale);
            let rate = u128::from(self.config.sample_rate);
            let configured_window = duration != 0 && (u128::from(duration) == ticks / rate || u128::from(duration) == ticks.div_ceil(rate));
            if configured_window && samples != self.config.frame_samples as usize {self.ssr_fixed_clock=true;}
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
                if matches!(self.config.object_type,4|19) {
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
                    }?;
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
                    // Spectral coupling already contributed to the target.
                    // Its own reconstructed PCM must still advance the LTP
                    // loop, including frames without prediction and PNS bands.
                    if matches!(self.config.object_type,4|19) {
                        ltp_coupling_synthesis[usize::from(coupling.tag)].as_mut().ok_or_else(||invalid("AAC LTP coupling state missing"))?.synthesize_spectrum(&spectrum,coupling.channel.info.sequence,coupling.channel.info.shape)?;
                    }
                    continue;
                }
                if matches!(self.config.object_type,4|19) {
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
                    }?;
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
impl NativeAacDecoder {
    fn decode_ld_timed(
        &mut self,
        packet: &[u8],
        pts: i64,
        duration: u64,
    ) -> Result<Option<AacFrame>> {
        use super::aac_ld_synthesis::LdWindowShape;
        let config = &self.config;
        let pce_elements: Vec<u8> = self
            .program
            .as_ref()
            .filter(|_| config.channel_configuration == 0)
            .map(|p| {
                p.elements
                    .iter()
                    .map(|e| {
                        if e.position == super::aac_pce::Position::Lfe {
                            3
                        } else {
                            u8::from(e.pair)
                        }
                    })
                    .collect()
            })
            .unwrap_or_default();
        let elements: &[u8] = match config.channel_configuration {
            0 if !pce_elements.is_empty() => &pce_elements,
            1 => &[0],
            2 => &[1],
            3 => &[0, 1],
            4 => &[0, 1, 0],
            5 => &[0, 1, 1],
            6 => &[0, 1, 1, 3],
            7 | 12 => &[0, 1, 1, 1, 3],
            11 => &[0, 1, 1, 0, 3],
            14 => &[0, 1, 1, 3, 1],
            _ => return Err(invalid("unsupported AAC LD configured layout")),
        };
        let bands =
            super::aac_ld_bands::LdBands::new(config.sample_rate, config.frame_samples as usize)?;
        let mut bits = BitReader::new(packet);
        let mut noise = self.noise.clone();
        let mut channels = Vec::with_capacity(usize::from(config.channels));
        for (index, element) in elements.iter().enumerate() {
            let tag = bits.read(4)? as u8;
            if let Some(program) = &self.program {
                if program.elements.get(index).is_none_or(|e| e.tag != tag) {
                    return Err(invalid("AAC element is absent from PCE"));
                }
            }
            if *element == 1 {
                let (pair, prediction) = ChannelPair::read_ld(&mut bits, config)?;
                let (left, right) = pair.spectra_with_noise(config, &mut noise)?;
                let [before, after] = prediction;
                channels.push((pair.left, left, before));
                channels.push((pair.right, right, after));
            } else {
                let (channel, prediction) = ChannelData::read_ld(&mut bits, config, false)?;
                let spectrum = channel.spectrum_with_noise(config, &mut noise)?;
                channels.push((channel, spectrum, prediction));
            }
        }
        super::aac_pce::skip_er_extensions(&mut bits)?;
        if channels.len() != usize::from(config.channels) || bits.remaining() > 7 {
            return Err(invalid(
                "trailing bytes or channel mismatch after AAC LD block",
            ));
        }
        // Whole-packet transaction: a later channel failure cannot advance an
        // earlier channel's PCM, lag, window, or noise history.
        let mut next = self.ld_synthesis.clone();
        let n = usize::from(config.frame_samples);
        let mut output = vec![0.; n * channels.len()];
        for (slot, (channel, residual, mut prediction)) in channels.into_iter().enumerate() {
            let target = self.mapping[slot];
            let shape = if channel.info.shape == super::aac_synthesis::WindowShape::Sine {
                LdWindowShape::Sine
            } else {
                LdWindowShape::LowOverlap
            };
            let limit = bands.tns_max_bands.min(usize::from(channel.info.max_sfb));
            if let Some(data) = &mut prediction { channel.suppress_ltp_noise(&mut data.used); }
            let pcm = next[target].process(
                residual,
                prediction.as_ref(),
                shape,
                bands.offsets,
                limit,
                channel.tns.as_ref(),
            )?;
            for (i, value) in pcm.iter().enumerate() {
                let value = *value as f32;
                if !value.is_finite() {
                    return Err(invalid("AAC LD PCM exceeds finite f32 output"));
                }
                output[i * usize::from(config.channels) + target] = value;
            }
        }
        self.ld_synthesis = next;
        self.noise = noise;
        Ok(Some(AacFrame {
            samples: output,
            pts,
            duration,
        }))
    }
}
impl NativeAacDecoder {
    fn decode_eld_timed(
        &mut self,
        packet: &[u8],
        pts: i64,
        duration: u64,
    ) -> Result<Option<AacFrame>> {
        let config = &self.config;
        let elements: &[u8] = match config.channel_configuration {
            1 => &[0],
            2 => &[1],
            3 => &[0, 1],
            4 => &[0, 1, 0],
            5 => &[0, 1, 1],
            6 => &[0, 1, 1, 3],
            7 | 12 => &[0, 1, 1, 1, 3],
            11 => &[0, 1, 1, 0, 3],
            14 => &[0, 1, 1, 3, 1],
            _ => return Err(invalid("unsupported AAC ELD configured layout")),
        };
        let mut bits = BitReader::new(packet);
        let mut noise = self.noise.clone();
        let mut channels = Vec::new();
        for &kind in elements {
            if kind == 1 {
                let pair = ChannelPair::read_eld(&mut bits, config)?;
                let (l, r) = pair.spectra_with_noise(config, &mut noise)?;
                channels.push((pair.left, l));
                channels.push((pair.right, r));
            } else {
                let c = ChannelData::read_eld(&mut bits, config, None, false)?;
                let s = c.spectrum_with_noise(config, &mut noise)?;
                channels.push((c, s));
            }
        }
        let mut next = self.eld_synthesis.clone();
        let mut planar = Vec::with_capacity(channels.len());
        for (slot, (channel, spectrum)) in channels.into_iter().enumerate() {
            planar.push(
                next[self.mapping[slot]].synthesize_pcm(&channel.apply_tns(config, spectrum)?)?,
            );
        }
        let mut ld = self.eld_sbr.clone();
        if let Some(ld) = &mut ld {
            let end = bits.position() + bits.remaining();
            let (output, used) = ld.process_prefix(packet, bits.position(), end, &planar)?;
            bits.skip(used)?;
            planar = output;
        }
        super::aac_pce::skip_er_extensions(&mut bits)?;
        if bits.remaining() > 7 || planar.len() != usize::from(config.channels) {
            return Err(invalid(
                "trailing bytes or channel mismatch after AAC ELD block",
            ));
        }
        while bits.remaining() > 0 {
            if bits.bit()? {
                return Err(invalid("nonzero AAC ELD alignment bits"));
            }
        }
        let n = planar.first().map_or(0, Vec::len);
        if n == 0 || planar.iter().any(|c| c.len() != n) {
            return Err(invalid("AAC ELD output channel length mismatch"));
        }
        let mut output = vec![0.0; n * planar.len()];
        for (slot, channel) in planar.iter().enumerate() {
            for (i, &value) in channel.iter().enumerate() {
                output[i * usize::from(config.channels) + self.mapping[slot]] = value;
            }
        }
        self.eld_synthesis = next;
        self.eld_sbr = ld;
        self.noise = noise;
        Ok(Some(AacFrame {
            samples: output,
            pts,
            duration,
        }))
    }
}

#[cfg(test)]
mod channel_window_tests {
    use super::*;
    #[test]
    fn all_standard_channels_share_immutable_synthesis_windows() {
        for configuration in 1..=7 {
            let decoder = NativeAacDecoder::new(&[0x12, configuration << 3]).unwrap();
            assert_eq!(
                decoder.synthesis.len(),
                usize::from(decoder.config.channels)
            );
            for channel in &decoder.synthesis {
                assert!(decoder.synthesis[0].shares_windows_with(channel));
            }
        }
    }
}

impl NativeAacDecoder {
    /// Retained heap allocation payload, including spare vector capacity and
    /// uniquely owned shared tables. Excludes allocator/Arc control headers,
    /// stack objects, packet temporaries and caller-owned output. This is not
    /// a peak decode/export budget or process RSS measurement.
    pub fn retained_payload_bytes(&self) -> Result<usize> {
        self.retained_payload_bytes_with_checkpoint(None)
    }
    /// Joint retained payload of this decoder and one optional checkpoint.
    /// Shared windows/IMDCT data are counted once across channels and snapshots.
    /// Successful inspection allocates no heap memory; its bounded workspace
    /// is on stack. Error diagnostics may allocate.
    pub fn retained_payload_bytes_with_checkpoint(
        &self,
        checkpoint: Option<&AacCheckpoint>,
    ) -> Result<usize> {
        fn visit(
            synthesis: &Vec<LongSineSynthesis>,
            coupling: &Vec<Option<LongSineSynthesis>>,
            mapping: &Vec<usize>,
            program: Option<&super::aac_pce::ProgramConfig>,
            footprint: &mut super::memory::Footprint,
        ) -> std::result::Result<(), String> {
            footprint.vector(synthesis)?;
            footprint.vector(coupling)?;
            footprint.vector(mapping)?;
            for state in synthesis
                .iter()
                .chain(coupling.iter().filter_map(|state| state.as_ref()))
            {
                state.visit_retained(footprint)?;
            }
            if let Some(program) = program {
                footprint.vector(&program.elements)?;
                footprint.vector(&program.associated_data)?;
                footprint.vector(&program.coupling)?;
                footprint.vector(&program.comment)?;
            }
            Ok(())
        }
        let mut footprint = super::memory::Footprint::new();
        if let Some(ep)=&self.protection {ep.visit_retained(&mut footprint).map_err(|e|invalid(&e))?;}
        if let Some(ep)=checkpoint.and_then(|c|c.protection.as_ref()) {ep.visit_retained(&mut footprint).map_err(|e|invalid(&e))?;}
        if let Some(ld)=&self.eld_sbr {ld.visit_retained(&mut footprint).map_err(|e|invalid(&e))?;}
        if let Some(ld)=checkpoint.and_then(|c|c.eld_sbr.as_ref()) {ld.visit_retained(&mut footprint).map_err(|e|invalid(&e))?;}

        footprint.vector(&self.eld_synthesis).map_err(|e|invalid(&e))?;
        for state in &self.eld_synthesis {state.visit_retained(&mut footprint).map_err(|e|invalid(&e))?;}
        if let Some(saved)=checkpoint {
            footprint.vector(&saved.eld_synthesis).map_err(|e|invalid(&e))?;
            for state in &saved.eld_synthesis {state.visit_retained(&mut footprint).map_err(|e|invalid(&e))?;}
        }
        footprint.vector(&self.ld_synthesis).map_err(|e|invalid(&e))?;
        for state in &self.ld_synthesis {state.visit_retained(&mut footprint).map_err(|e|invalid(&e))?;}
        if let Some(saved)=checkpoint {
            footprint.vector(&saved.ld_synthesis).map_err(|e|invalid(&e))?;
            for state in &saved.ld_synthesis {state.visit_retained(&mut footprint).map_err(|e|invalid(&e))?;}
        }
        footprint.vector(&self.ltp_synthesis).map_err(|e|invalid(&e))?;
        for state in &self.ltp_synthesis {state.visit_retained(&mut footprint).map_err(|e|invalid(&e))?;}
        footprint.vector(&self.ltp_coupling_synthesis).map_err(|e|invalid(&e))?;
        for state in self.ltp_coupling_synthesis.iter().flatten() {state.visit_retained(&mut footprint).map_err(|e|invalid(&e))?;}
        if let Some(saved)=checkpoint {
            footprint.vector(&saved.ltp_coupling_synthesis).map_err(|e|invalid(&e))?;
            for state in saved.ltp_coupling_synthesis.iter().flatten() {state.visit_retained(&mut footprint).map_err(|e|invalid(&e))?;}
            footprint.vector(&saved.ltp_synthesis).map_err(|e|invalid(&e))?;
            for state in &saved.ltp_synthesis {state.visit_retained(&mut footprint).map_err(|e|invalid(&e))?;}
        }
        for states in [Some(&self.ssr_synthesis), checkpoint.map(|state| &state.ssr_synthesis)].into_iter().flatten() {
            footprint.vector(states).map_err(|e| invalid(&e))?;
            for state in states { state.visit_retained(&mut footprint).map_err(|e| invalid(&e))?; }
        }
        for states in [Some(&self.ssr_coupling_synthesis), checkpoint.map(|state| &state.ssr_coupling_synthesis)].into_iter().flatten() {
            footprint.vector(states).map_err(|e| invalid(&e))?;
            for state in states.iter().flatten() {
                footprint.add(std::mem::size_of::<super::aac_ssr_synthesis::SsrSynthesis>()).map_err(|e| invalid(&e))?;
                state.visit_retained(&mut footprint).map_err(|e| invalid(&e))?;
            }
        }
        for banks in [Some(&self.main_prediction), checkpoint.map(|state| &state.main_prediction)].into_iter().flatten() {
            footprint.vector(banks).map_err(|e| invalid(&e))?;
            for bank in banks.iter().flatten() { footprint.add(bank.retained_payload_bytes()).map_err(|e| invalid(&e))?; }
        }
        for tags in [Some(&self.ssr_alignment_tags), checkpoint.map(|state| &state.ssr_alignment_tags)].into_iter().flatten() {
            footprint.vector(tags).map_err(|e| invalid(&e))?;
        }
        for alignment in [self.ssr_alignment.as_ref(), checkpoint.and_then(|state| state.ssr_alignment.as_ref())].into_iter().flatten() {
            footprint.add(alignment.retained_payload_bytes()?).map_err(|e| invalid(&e))?;
        }
        visit(
            &self.synthesis,
            &self.coupling_synthesis,
            &self.mapping,
            self.program.as_ref(),
            &mut footprint,
        )
        .map_err(|e| invalid(&e))?;
        if let Some(checkpoint) = checkpoint {
            visit(
                &checkpoint.synthesis,
                &checkpoint.coupling_synthesis,
                &checkpoint.mapping,
                checkpoint.program.as_ref(),
                &mut footprint,
            )
            .map_err(|e| invalid(&e))?;
        }
        for elements in [Some(&self.sbr_elements), checkpoint.map(|state| &state.sbr_elements)].into_iter().flatten() {
            footprint.vector(elements).map_err(|e| invalid(&e))?;
            for element in elements.iter().flatten() {
                element.stream.visit_retained(&mut footprint).map_err(|e| invalid(&e))?;
                element.dsp.visit_retained(&mut footprint).map_err(|e| invalid(&e))?;
            }
        }
        for queue in [Some(&self.ssr_pending_duration),checkpoint.map(|s|&s.ssr_pending_duration)].into_iter().flatten() {
            for packet in queue.iter() {
                if let Some(sbr)=&packet.sbr {
                    footprint.vector(&sbr.groups).map_err(|e|invalid(&e))?;
                    footprint.vector(&sbr.mapping).map_err(|e|invalid(&e))?;
                    footprint.vector(&sbr.couplings).map_err(|e|invalid(&e))?;
                    for (_,frame) in &sbr.couplings {
                        if let Some(frame)=frame {footprint.add(frame.retained_payload_bytes()?).map_err(|e|invalid(&e))?;}
                    }
                    for (_,_,frame) in &sbr.groups {
                        if let Some(frame)=frame {footprint.add(frame.retained_payload_bytes()?).map_err(|e|invalid(&e))?;}
                    }
                }
            }
        }
        Ok(footprint.total())
    }
}
#[cfg(test)]
mod retained_memory_tests {
    use super::*;
    fn scratch_payload(samples: usize) -> usize {
        (3 * samples + samples / 4) * std::mem::size_of::<f64>()
            + (2 * samples - 1).next_power_of_two() * std::mem::size_of::<[f64; 2]>()
    }
    #[test]
    fn occupied_coupling_slot_counts_its_independent_scratch() {
        for (samples, flag) in [(1024, 0), (960, 4)] {
            let mut decoder = NativeAacDecoder::new(&[0x12, 0x08 | flag]).unwrap();
            assert_eq!(usize::from(decoder.config.frame_samples), samples);
            let before = decoder.retained_payload_bytes().unwrap();
            // Ownership accounting only: this does not claim coupling profile acceptance.
            decoder.coupling_synthesis[0] = Some(decoder.synthesis[0].clone());
            assert_eq!(
                decoder.retained_payload_bytes().unwrap() - before,
                scratch_payload(samples)
            );
        }
    }
    #[test]
    fn channels_and_checkpoints_count_mutable_capacity_but_share_tables() {
        for (samples, flag) in [(1024, 0), (960, 4)] {
            let mono = NativeAacDecoder::new(&[0x12, 0x08 | flag]).unwrap();
            let stereo = NativeAacDecoder::new(&[0x12, 0x10 | flag]).unwrap();
            assert_eq!(usize::from(mono.config.frame_samples), samples);
            let state = std::mem::size_of::<LongSineSynthesis>() + scratch_payload(samples);
            assert_eq!(
                stereo.retained_payload_bytes().unwrap() - mono.retained_payload_bytes().unwrap(),
                state + std::mem::size_of::<usize>()
            );
            let checkpoint = mono.checkpoint();
            let added = state
                + std::mem::size_of::<usize>()
                + 16 * std::mem::size_of::<Option<LongSineSynthesis>>();
            assert_eq!(
                mono.retained_payload_bytes_with_checkpoint(Some(&checkpoint)).unwrap(),
                mono.retained_payload_bytes().unwrap() + added
            );
        }
    }
}

#[cfg(test)]
mod element_tag_tests {
    use super::ElementTags;
    #[test]
    fn bounded_tags_distinguish_element_kinds_and_reject_repeats() {
        let mut tags = ElementTags::default();
        for kind in 0..4 {
            for tag in 0..16 {
                assert!(tags.insert(kind, tag));
                assert!(!tags.insert(kind, tag));
            }
        }
        assert!(!tags.insert(4, 0));
        assert!(!tags.insert(0, 16));
        assert_eq!(std::mem::size_of::<ElementTags>(), 8);
    }
}

#[cfg(test)]
mod he_aac_native_tests { 
use super::*;
use serde_json::Value;
const PACKETS: &[u8] =
    include_bytes!("../../../../tests/fixtures/playback-errors/he-aac-sbr-packets.bin");
const PCM: &[u8] =
    include_bytes!("../../../../tests/fixtures/playback-errors/aac-sbr-dsp-pcm.f64le");
fn cases() -> Value {
    serde_json::from_slice(include_bytes!(
        "../../../../tests/fixtures/playback-errors/he-aac-sbr-packets.json"
    ))
    .unwrap()
}
fn asc(case: &Value) -> Vec<u8> {
    case["asc"]
        .as_str()
        .unwrap()
        .as_bytes()
        .chunks_exact(2)
        .map(|b| u8::from_str_radix(std::str::from_utf8(b).unwrap(), 16).unwrap())
        .collect()
}
fn packet(frame: &Value) -> &[u8] {
    let at = frame["offset"].as_u64().unwrap() as usize;
    &PACKETS[at..at + frame["bytes"].as_u64().unwrap() as usize]
}
#[test]
fn complete_authored_lc_plus_sbr_blocks_match_independent_pcm_and_clock() {
    let manifest = cases();
    for case in manifest["cases"].as_array().unwrap() {
        let config = asc(case);
        let mut decoder = NativeAacDecoder::new(&config).unwrap();
        let rate = if case["bands"] == 64 { 48000 } else { 24000 };
        assert_eq!((decoder.sample_rate(), decoder.channels()), (rate, 1));
        let mut output = Vec::new();
        for frame in case["frames"].as_array().unwrap() {
            let saved = decoder.checkpoint();
            let decoded = decoder.decode(packet(frame)).unwrap();
            decoder.reset();
            decoder.restore(&saved).unwrap();
            assert_eq!(decoded, decoder.decode(packet(frame)).unwrap());
            output.extend(decoded);
        }
        assert_eq!(output.len(), case["samples"].as_u64().unwrap() as usize);
        let offset = case["pcm_offset"].as_u64().unwrap() as usize;
        for (&value, bytes) in output
            .iter()
            .zip(PCM[offset..offset + 8 * output.len()].chunks_exact(8))
        {
            let expected = f64::from_le_bytes(bytes.try_into().unwrap()) as f32;
            assert_eq!(value.to_bits(), expected.to_bits());
        }
    }
}
#[test]
fn crc_failure_preserves_aac_and_sbr_histories_and_rejects_mismatched_checkpoints() {
    let manifest = cases();
    for case in manifest["cases"].as_array().unwrap() {
        let mut decoder = NativeAacDecoder::new(&asc(case)).unwrap();
        let frames = case["frames"].as_array().unwrap();
        decoder.decode(packet(&frames[0])).unwrap();
        let before = decoder.checkpoint();
        let raw = packet(&frames[1]);
        let mut broken = raw.to_vec();
        // The original silent SCE is 29 bits, FIL/type/count lead to bit40:
        // flip a stored CRC bit without changing core or SBR syntax.
        broken[5] ^= 0x80;
        assert!(
            decoder
                .decode(&broken)
                .unwrap_err()
                .to_string()
                .contains("CRC")
        );
        let expected = decoder.decode(raw).unwrap();
        decoder.restore(&before).unwrap();
        assert_eq!(decoder.decode(raw).unwrap(), expected);
        let mut lc = NativeAacDecoder::new(&[0x13, 0x08]).unwrap();
        assert!(lc.restore(&before).is_err());
    }
}
#[test]
fn extended_asc_keeps_core_rate_ps_and_unspecified_flags_distinct() {
    fn packed(text: &str) -> Vec<u8> {
        let mut bits = text.to_owned();
        while bits.len() % 8 != 0 {
            bits.push('0');
        }
        bits.as_bytes()
            .chunks_exact(8)
            .map(|b| u8::from_str_radix(std::str::from_utf8(b).unwrap(), 2).unwrap())
            .collect()
    }
    // Explicit AOT5, indexed24k core /48k extension, LC, 960 frame.
    let explicit = packed("00101 0110 0001 0011 00010 100".replace(' ', "").as_str());
    let parsed = AudioSpecificConfig::parse(&explicit).unwrap();
    assert_eq!(
        (
            parsed.core.sample_rate,
            parsed.output_sample_rate(),
            parsed.core.frame_samples
        ),
        (24000, 48000, 960)
    );
    assert_eq!((parsed.sbr_present, parsed.ps_present), (Some(true), None));
    assert!(AacConfig::parse(&explicit).is_err()); // legacy metadata remains guarded
    let core = AudioSpecificConfig::parse(&[0x13, 0x08]).unwrap();
    assert_eq!((core.sbr_present, core.ps_present), (None, None));
    // Core LC24k mono + sync SBR absent: preserve an explicit false flag.
    let absent = packed(
        "00010 0110 0001 000 01010110111 00101 0"
            .replace(' ', "")
            .as_str(),
    );
    let parsed = AudioSpecificConfig::parse(&absent).unwrap();
    assert_eq!(parsed.sbr_present, Some(false));
    assert_eq!(parsed.output_sample_rate(), 24000);
    assert!(AacConfig::parse(&absent).is_ok());
    let ps = packed("11101 0110 0001 0011 00010 000".replace(' ', "").as_str());
    let parsed = AudioSpecificConfig::parse(&ps).unwrap();
    assert_eq!(
        (parsed.ps_present, parsed.output_channels()),
        (Some(true), 2)
    );
    assert!(
        NativeAacDecoder::new(&ps)
            .err()
            .unwrap()
            .to_string()
            .contains("parametric stereo")
    );
    let sync_ps = packed(
        "00010 0110 0001 000 01010110111 00101 1 0011 10101001000 1"
            .replace(' ', "")
            .as_str(),
    );
    let parsed = AudioSpecificConfig::parse(&sync_ps).unwrap();
    assert_eq!(parsed.ps_present, Some(true));
    // Independent 24-bit core/extension frequency escapes.
    let text = format!(
        "00101 1111 {:024b} 0001 1111 {:024b} 00010 000",
        24000, 48000
    )
    .replace(' ', "");
    let escaped = packed(&text);
    let parsed = AudioSpecificConfig::parse(&escaped).unwrap();
    assert_eq!(
        (parsed.core.sample_rate, parsed.output_sample_rate()),
        (24000, 48000)
    );
    for end in 0..escaped.len() {
        assert!(AudioSpecificConfig::parse(&escaped[..end]).is_err());
    }
    let zero = packed(&format!("00101 1111 {:024b} 0001 0011 00010 000", 0).replace(' ', ""));
    assert!(AudioSpecificConfig::parse(&zero).is_err());
    let reserved = packed("00101 1101 0001 0011 00010 000".replace(' ', "").as_str());
    assert!(AudioSpecificConfig::parse(&reserved).is_err());
}

#[test]
fn complete_stereo_cpe_sbr_matches_both_independent_mono_oracles_and_crc_rollback() {
    let manifest: Value = serde_json::from_slice(include_bytes!(
        "../../../../tests/fixtures/playback-errors/he-aac-sbr-stereo.json"
    ))
    .unwrap();
    let data = include_bytes!("../../../../tests/fixtures/playback-errors/he-aac-sbr-stereo.bin");
    for case in manifest["cases"].as_array().unwrap() {
        let mut decoder = NativeAacDecoder::new(&asc(case)).unwrap();
        assert_eq!(decoder.channels(), 2);
        let mut output = Vec::new();
        for (n, frame) in case["frames"].as_array().unwrap().iter().enumerate() {
            let start = frame["offset"].as_u64().unwrap() as usize;
            let raw = &data[start..start + frame["bytes"].as_u64().unwrap() as usize];
            let saved = decoder.checkpoint();
            if n == 1 {
                let bit = frame["crc_bit"].as_u64().unwrap() as usize;
                let mut broken = raw.to_vec();
                broken[bit / 8] ^= 1 << (7 - bit % 8);
                assert!(
                    decoder
                        .decode(&broken)
                        .unwrap_err()
                        .to_string()
                        .contains("CRC")
                );
            }
            let decoded = decoder
                .decode(raw)
                .unwrap_or_else(|e| panic!("frame {n}, case {case}: {e}"));
            decoder.reset();
            decoder.restore(&saved).unwrap();
            assert_eq!(decoded, decoder.decode(raw).unwrap());
            output.extend(decoded);
        }
        let offset = case["pcm_offset"].as_u64().unwrap() as usize;
        assert_eq!(output.len(), 2 * case["samples"].as_u64().unwrap() as usize);
        for (pair, b) in output.chunks_exact(2).zip(PCM[offset..].chunks_exact(8)) {
            let expected = (f64::from_le_bytes(b.try_into().unwrap()) as f32).to_bits();
            assert_eq!(pair[0].to_bits(), expected, "left {case}");
            assert_eq!(pair[1].to_bits(), expected, "right {case}");
        }
    }
}

#[test]
fn missing_sbr_fill_keeps_pcm_clock_noise_history_and_checkpoint_replay() {
    let manifest: Value = serde_json::from_slice(include_bytes!(
        "../../../../tests/fixtures/playback-errors/he-aac-missing-sbr.json"
    ))
    .unwrap();
    let packets =
        include_bytes!("../../../../tests/fixtures/playback-errors/he-aac-missing-sbr.bin");
    let pcm = include_bytes!("../../../../tests/fixtures/playback-errors/he-aac-missing-sbr.f64le");
    for case in manifest["cases"].as_array().unwrap() {
        let mut decoder = NativeAacDecoder::new(&asc(case)).unwrap();
        let mut output = Vec::new();
        for frame in case["frames"].as_array().unwrap() {
            let at = frame["offset"].as_u64().unwrap() as usize;
            let raw = &packets[at..at + frame["bytes"].as_u64().unwrap() as usize];
            let checkpoint = decoder.checkpoint();
            assert!(decoder.decode(&[]).is_err());
            let rendered = decoder.decode(raw).unwrap();
            assert_eq!(
                rendered.len(),
                64 * case["slots"].as_u64().unwrap() as usize
                    * if case["bands"] == 64 { 2 } else { 1 }
            );
            decoder.reset();
            decoder.restore(&checkpoint).unwrap();
            assert_eq!(rendered, decoder.decode(raw).unwrap());
            output.extend(rendered);
        }
        let offset = case["pcm_offset"].as_u64().unwrap() as usize;
        assert_eq!(output.len(), case["samples"].as_u64().unwrap() as usize);
        for (&value, b) in output.iter().zip(pcm[offset..].chunks_exact(8)) {
            let expected = f64::from_le_bytes(b.try_into().unwrap()) as f32;
            assert!(
                (value - expected).abs() < 1e-7,
                "{case}: {value} vs {expected}"
            );
        }
    }
}

#[test]
fn implicit_sbr_with_container_output_clock_matches_independent_pcm() {
    let manifest: Value = serde_json::from_slice(include_bytes!(
        "../../../../tests/fixtures/playback-errors/he-aac-implicit-sbr.json"
    ))
    .unwrap();
    for case in manifest["cases"].as_array().unwrap() {
        let config = asc(case);
        assert_eq!(
            AudioSpecificConfig::parse(&config).unwrap().sbr_present,
            None
        );
        let mut decoder = NativeAacDecoder::new_with_output_rate(&config, 48000).unwrap();
        assert_eq!(decoder.sample_rate(), 48000);
        let mut output = Vec::new();
        for frame in case["frames"].as_array().unwrap() {
            let saved = decoder.checkpoint();
            let data = decoder.decode(packet(frame)).unwrap();
            decoder.reset();
            decoder.restore(&saved).unwrap();
            assert_eq!(data, decoder.decode(packet(frame)).unwrap());
            output.extend(data);
        }
        assert_eq!(output.len(), case["samples"].as_u64().unwrap() as usize);
        let at = case["pcm_offset"].as_u64().unwrap() as usize;
        for (&value, b) in output.iter().zip(PCM[at..].chunks_exact(8)) {
            assert_eq!(
                value.to_bits(),
                (f64::from_le_bytes(b.try_into().unwrap()) as f32).to_bits()
            );
        }
        // A fixed LC API has no output hint and must retain its old refusal.
        let mut strict = NativeAacDecoder::new(&config).unwrap();
        assert!(
            strict
                .decode(packet(&case["frames"][0]))
                .unwrap_err()
                .to_string()
                .contains("extension-aware stream signalling")
        );
        let mut off = config.clone();
        let mut bits = Vec::new();
        let push = |bits: &mut Vec<bool>, value: u32, n: u32| {
            for shift in (0..n).rev() {
                bits.push(value & (1 << shift) != 0);
            }
        };
        push(&mut bits, 0x2b7, 11);
        push(&mut bits, 5, 5);
        push(&mut bits, 0, 1);
        while bits.len() % 8 != 0 {
            bits.push(false);
        }
        for byte in bits.chunks_exact(8) {
            off.push(byte.iter().fold(0, |v, b| (v << 1) | u8::from(*b)));
        }
        assert_eq!(
            AudioSpecificConfig::parse(&off).unwrap().sbr_present,
            Some(false)
        );
        assert!(NativeAacDecoder::new_with_output_rate(&off, 48000).is_err());
        assert!(NativeAacDecoder::new_with_output_rate(&config, 44100).is_err());
    }
}

#[test]
fn sbr_discovery_changes_clock_atomically_and_restores_undetected_checkpoints() {
    let manifest: Value = serde_json::from_slice(include_bytes!(
        "../../../../tests/fixtures/playback-errors/he-aac-implicit-sbr.json"
    ))
    .unwrap();
    for case in manifest["cases"].as_array().unwrap() {
        let config = asc(case);
        let mut detected = NativeAacDecoder::new_with_sbr_detection(&config).unwrap();
        let mut fixed = NativeAacDecoder::new_with_output_rate(&config, 48000).unwrap();
        assert_eq!(detected.sample_rate(), 24000);
        let initial = detected.checkpoint();
        let first = packet(&case["frames"][0]);
        for cut in 0..first.len() {
            assert!(detected.decode(&first[..cut]).is_err());
            assert_eq!(detected.sample_rate(), 24000);
        }
        for (n, frame) in case["frames"].as_array().unwrap().iter().enumerate() {
            assert_eq!(
                detected.decode(packet(frame)).unwrap(),
                fixed.decode(packet(frame)).unwrap()
            );
            assert_eq!(detected.sample_rate(), 48000);
            if n == 0 {
                detected.restore(&initial).unwrap();
                assert_eq!(detected.sample_rate(), 24000);
                let actual = detected.decode(first).unwrap();
                let mut replay = NativeAacDecoder::new_with_output_rate(&config, 48000).unwrap();
                assert_eq!(actual, replay.decode(first).unwrap());
            }
        }
        let mut off = config.clone();
        off.extend_from_slice(&[0x56, 0xe5, 0]);
        assert_eq!(
            AudioSpecificConfig::parse(&off).unwrap().sbr_present,
            Some(false)
        );
        let mut forbidden = NativeAacDecoder::new_with_sbr_detection(&off).unwrap();
        assert!(forbidden.decode(first).is_err());
        assert_eq!(forbidden.sample_rate(), 24000);
        detected.reset();
        assert_eq!(detected.sample_rate(), 24000);
        let mut strict = NativeAacDecoder::new(&config).unwrap();
        assert!(strict.restore(&initial).is_err());
    }
    // After a valid LC prefix, discovery must retain the earlier delay-only QMF
    // state, and restoring a pre-discovery checkpoint must recover the core clock.
    let missing: Value = serde_json::from_slice(include_bytes!(
        "../../../../tests/fixtures/playback-errors/he-aac-missing-sbr.json"
    ))
    .unwrap();
    let raw = include_bytes!("../../../../tests/fixtures/playback-errors/he-aac-missing-sbr.bin");
    for case in missing["cases"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|c| c["bands"] == 64 && c["pattern"][0] == false && c["pattern"][1] == true)
    {
        let config = if case["slots"] == 16 {
            vec![0x13, 0x08]
        } else {
            vec![0x13, 0x0c]
        };
        let mut detected = NativeAacDecoder::new_with_sbr_detection(&config).unwrap();
        let mut fixed = NativeAacDecoder::new_with_output_rate(&config, 48000).unwrap();
        for (n, frame) in case["frames"].as_array().unwrap().iter().enumerate() {
            let at = frame["offset"].as_u64().unwrap() as usize;
            let packet = &raw[at..at + frame["bytes"].as_u64().unwrap() as usize];
            let actual = detected.decode(packet).unwrap();
            let reference = fixed.decode(packet).unwrap();
            if n == 0 {
                assert_eq!(actual.len() * 2, reference.len());
                assert_eq!(detected.sample_rate(), 24000);
            } else {
                assert_eq!(actual, reference);
                assert_eq!(detected.sample_rate(), 48000);
            }
        }
    }
}

 }

#[cfg(test)]
mod sbr_memory_tests {
    use super::*;
    #[test]
    fn retained_payload_counts_sbr_and_checkpoint_allocations_without_shared_window_duplication() {
        let cases: serde_json::Value=serde_json::from_slice(include_bytes!("../../../../tests/fixtures/playback-errors/he-aac-sbr-packets.json")).unwrap();
        let case=&cases["cases"][0];
        let asc:Vec<u8>=case["asc"].as_str().unwrap().as_bytes().chunks_exact(2).map(|b|u8::from_str_radix(std::str::from_utf8(b).unwrap(),16).unwrap()).collect();
        let mut decoder=NativeAacDecoder::new(&asc).unwrap();
        let before=decoder.retained_payload_bytes().unwrap();
        let binary=include_bytes!("../../../../tests/fixtures/playback-errors/he-aac-sbr-packets.bin");
        let frame=&case["frames"][0];let off=frame["offset"].as_u64().unwrap() as usize;
        decoder.decode(&binary[off..off+frame["bytes"].as_u64().unwrap() as usize]).unwrap();
        let after=decoder.retained_payload_bytes().unwrap();assert!(after>before+16_000);
        let checkpoint=decoder.checkpoint();
        let combined=decoder.retained_payload_bytes_with_checkpoint(Some(&checkpoint)).unwrap();
        assert!(combined>after+16_000);assert!(combined<after*2);
        decoder.reset();assert_eq!(decoder.retained_payload_bytes().unwrap(),before);
        decoder.restore(&checkpoint).unwrap();assert!(decoder.retained_payload_bytes().unwrap()>before+16_000);
    }
}

#[cfg(test)]
mod main_prediction_memory_tests {
    use super::*;
    #[test]
    fn main_predictor_allocations_and_cloned_checkpoints_are_accounted() {
        let main = NativeAacDecoder::new(&[0x0b, 0x08]).unwrap();
        let lc = NativeAacDecoder::new(&[0x13, 0x08]).unwrap();
        let bank_bytes = main.main_prediction.capacity()
            * std::mem::size_of::<Option<super::super::aac_main_predictor::MainPredictor>>()
            + main.main_prediction.iter().flatten().map(|bank| bank.retained_payload_bytes()).sum::<usize>();
        assert!(bank_bytes > 0);
        assert_eq!(main.retained_payload_bytes().unwrap(), lc.retained_payload_bytes().unwrap() + bank_bytes);
        let main_checkpoint = main.checkpoint();
        let lc_checkpoint = lc.checkpoint();
        assert_eq!(main.retained_payload_bytes_with_checkpoint(Some(&main_checkpoint)).unwrap(),
            lc.retained_payload_bytes_with_checkpoint(Some(&lc_checkpoint)).unwrap() + 2*bank_bytes);
    }
}

#[cfg(test)]
mod ltp_dispatch_tests {
    use super::*;
    use serde_json::Value;
    #[test]
    fn native_target_only_phase_controls_match_scalar_ltp_tns_pcm() {
        let manifest: Value = serde_json::from_slice(&bytes("aac-ltp-phase.json")).unwrap();
        let blob = bytes("aac-ltp-phase-control-packets.bin");
        let gold = bytes("aac-ltp-phase-control-reference.f32le");
        for case in manifest["cases"].as_array().unwrap() {
            let mut state = decoder(case["asc"].as_str().unwrap());
            for row in case["frames"].as_array().unwrap() {
                let at = row["control_offset"].as_u64().unwrap() as usize;
                let packet = &blob[at..at + row["control_bytes"].as_u64().unwrap() as usize];
                let saved = state.checkpoint();
                let pcm = state.decode(packet).unwrap();
                let at = row["control_reference_offset"].as_u64().unwrap() as usize;
                assert_eq!(pcm.len(), 1024);
                for (i, &sample) in pcm.iter().enumerate() {
                    let expected =
                        f32::from_le_bytes(gold[at + i * 4..at + i * 4 + 4].try_into().unwrap());
                    assert!(
                        (sample - expected).abs() < 1e-7,
                        "native staged TNS frame sample={i}: {sample} vs {expected}"
                    );
                }
                state.restore(&saved).unwrap();
                assert_eq!(state.decode(packet).unwrap(), pcm);
            }
        }
    }
    #[test]
    fn native_coupled_phase_videos_match_scalar_ltp_tns_pcm() {
        let manifest: Value = serde_json::from_slice(&bytes("aac-ltp-phase.json")).unwrap();
        let blob = bytes("aac-ltp-phase-packets.bin");
        let gold = bytes("aac-ltp-phase-reference.f32le");
        for case in manifest["cases"].as_array().unwrap() {
            let mut state = decoder(case["asc"].as_str().unwrap());
            let initial = state.checkpoint();
            for row in case["frames"].as_array().unwrap() {
                let at = row["offset"].as_u64().unwrap() as usize;
                let packet = &blob[at..at + row["bytes"].as_u64().unwrap() as usize];
                let saved = state.checkpoint();
                let pcm = state.decode(packet).unwrap();
                let at = row["reference_offset"].as_u64().unwrap() as usize;
                assert_eq!(pcm.len(), 1024);
                for (i, &sample) in pcm.iter().enumerate() {
                    let expected =
                        f32::from_le_bytes(gold[at + i * 4..at + i * 4 + 4].try_into().unwrap());
                    assert!(
                        (sample - expected).abs() < 1e-7,
                        "point={} sample={i}: {sample} vs {expected}",
                        case["point"]
                    );
                }
                state.restore(&saved).unwrap();
                assert_eq!(state.decode(packet).unwrap(), pcm);
            }
            state.reset();
            assert!(state.ltp_coupling_synthesis.iter().all(Option::is_none));
            state.restore(&initial).unwrap();
        }
    }
    #[test]
    fn authored_cce_gain_and_selection_videos_decode_and_replay() {
        let manifest: Value = serde_json::from_slice(&bytes("aac-ltp-coupling.json")).unwrap();
        let blob = bytes("aac-ltp-coupling-packets.bin");
        for case in manifest["cases"].as_array().unwrap() {
            let mut state = decoder(case["asc"].as_str().unwrap());
            for row in case["frames"].as_array().unwrap() {
                let at = row["offset"].as_u64().unwrap() as usize;
                let packet = &blob[at..at + row["bytes"].as_u64().unwrap() as usize];
                let saved = state.checkpoint();
                let pcm = state.decode(packet).unwrap();
                assert_eq!(pcm.len(), 1024 * usize::from(state.config.channels));
                assert!(pcm.iter().all(|x| x.is_finite()));
                state.restore(&saved).unwrap();
                assert_eq!(state.decode(packet).unwrap(), pcm);
            }
        }
    }
    #[test]
    fn independent_cce_late_routing_failure_preserves_slots_histories_and_memory() {
        let manifest: Value = serde_json::from_slice(&bytes("aac-ltp-phase.json")).unwrap();
        let case = &manifest["cases"][2];
        let blob = bytes("aac-ltp-phase-packets.bin");
        let bad = bytes("aac-ltp-phase-absent-target.bin");
        let mut state = decoder(case["asc"].as_str().unwrap());
        for row in case["frames"].as_array().unwrap() {
            let at = row["offset"].as_u64().unwrap() as usize;
            let packet = &blob[at..at + row["bytes"].as_u64().unwrap() as usize];
            let saved = state.checkpoint();
            let before = state.retained_payload_bytes().unwrap();
            let noise = state.noise.clone();
            assert!(
                state
                    .decode(&bad)
                    .unwrap_err()
                    .to_string()
                    .contains("AAC coupling target is absent")
            );
            assert_eq!(state.retained_payload_bytes().unwrap(), before);
            assert_eq!(state.noise, noise);
            assert_eq!(
                state
                    .ltp_coupling_synthesis
                    .iter()
                    .map(Option::is_some)
                    .collect::<Vec<_>>(),
                saved
                    .ltp_coupling_synthesis
                    .iter()
                    .map(Option::is_some)
                    .collect::<Vec<_>>()
            );
            let pcm = state.decode(packet).unwrap();
            state.restore(&saved).unwrap();
            assert_eq!(state.decode(packet).unwrap(), pcm);
        }
    }
    #[test]
    fn native_ltp_long_short_transitions_and_960_geometry_match_scalar_pcm() {
        let manifest: Value = serde_json::from_slice(&bytes("aac-ltp-transitions.json")).unwrap();
        let blob = bytes("aac-ltp-transitions-packets.bin");
        let gold = bytes("aac-ltp-transitions-reference.f32le");
        let wrong = bytes("aac-ltp-transitions-stale-short-reference.f32le");
        for case in manifest["cases"].as_array().unwrap() {
            let mut state = decoder(case["asc"].as_str().unwrap());
            let initial = state.checkpoint();
            let n = case["n"].as_u64().unwrap() as usize;
            let mut first = Vec::new();
            let mut stale_peak = 0f32;
            for row in case["frames"].as_array().unwrap() {
                let at = row["offset"].as_u64().unwrap() as usize;
                let packet = &blob[at..at + row["bytes"].as_u64().unwrap() as usize];
                let saved = state.checkpoint();
                let pcm = state.decode(packet).unwrap();
                let at = row["reference_offset"].as_u64().unwrap() as usize;
                assert_eq!(pcm.len(), n);
                for (i, &sample) in pcm.iter().enumerate() {
                    let expected =
                        f32::from_le_bytes(gold[at + i * 4..at + i * 4 + 4].try_into().unwrap());
                    let stale =
                        f32::from_le_bytes(wrong[at + i * 4..at + i * 4 + 4].try_into().unwrap());
                    stale_peak = stale_peak.max((sample - stale).abs());
                    assert!(
                        (sample - expected).abs() < 1e-7,
                        "n={n} seq={} sample={i}: {sample} vs {expected}",
                        row["sequence"]
                    );
                }
                state.restore(&saved).unwrap();
                assert_eq!(state.decode(packet).unwrap(), pcm);
                first.push(pcm);
            }
            assert!(
                stale_peak > 1e-6,
                "fixture must detect stale short-frame history: {stale_peak}"
            );
            state.reset();
            state.restore(&initial).unwrap();
            for (row, expected) in case["frames"].as_array().unwrap().iter().zip(first) {
                let at = row["offset"].as_u64().unwrap() as usize;
                assert_eq!(
                    state
                        .decode(&blob[at..at + row["bytes"].as_u64().unwrap() as usize])
                        .unwrap(),
                    expected
                );
            }
        }
    }
    fn bytes(name: &str) -> Vec<u8> {
        std::fs::read(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../tests/fixtures/playback-errors")
                .join(name),
        )
        .unwrap()
    }
    // Use authored AOT4 ASC through the public constructor, including PCE tags.
    fn decoder(asc: &str) -> NativeAacDecoder {
        let raw:Vec<u8>=asc.as_bytes().chunks_exact(2).map(|pair|u8::from_str_radix(std::str::from_utf8(pair).unwrap(),16).unwrap()).collect();
        assert_eq!(raw[0]>>3,4);
        NativeAacDecoder::new(&raw).unwrap()
    }
    fn qualify(case: &Value, blob: &[u8], reference: &[u8]) {
        let mut state = decoder(case["asc"].as_str().unwrap());
        let channels = usize::from(state.channels());
        assert!(state.synthesis.is_empty());
        assert_eq!(state.ltp_synthesis.len(), channels);
        let memory = state.retained_payload_bytes().unwrap();
        let mut first = None;
        for (frame, row) in case["frames"].as_array().unwrap().iter().enumerate() {
            let at = row["offset"].as_u64().unwrap() as usize;
            let packet = &blob[at..at + row["bytes"].as_u64().unwrap() as usize];
            let saved = state.checkpoint();
            let output = state
                .decode_timed(packet, (frame * 1024) as i64, 1024)
                .unwrap()
                .unwrap();
            assert_eq!(output.pts, (frame * 1024) as i64);
            assert_eq!(output.duration, 1024);
            assert_eq!(output.samples.len(), 1024 * channels);
            for (i, &sample) in output.samples.iter().enumerate() {
                let at = (frame * 1024 * channels + i) * 4;
                let gold = f32::from_le_bytes(reference[at..at + 4].try_into().unwrap());
                assert!(
                    (sample - gold).abs() < 1e-7,
                    "native LTP frame={frame} sample={i}: {sample} vs {gold}"
                );
            }
            if frame == 0 {
                first = Some((packet.to_vec(), output.samples.clone()));
            }
            state.restore(&saved).unwrap();
            assert_eq!(state.decode(packet).unwrap(), output.samples);
            assert_eq!(state.retained_payload_bytes().unwrap(), memory);
            // An invalid block leaves the complete LTP packet-boundary state unchanged.
            let mut bad = packet.to_vec();
            bad.extend([0, 0]);
            assert!(state.decode(&bad).is_err());
        }
        assert!(state.finish().unwrap().is_none());
        state.reset();
        let (packet, pcm) = first.unwrap();
        assert_eq!(state.decode(&packet).unwrap(), pcm);
    }
    #[test]
    fn authored_mono_packets_use_native_ltp_dispatch_and_checkpoints() {
        let manifest: Value = serde_json::from_slice(&bytes("aac-ltp-syntax.json")).unwrap();
        let blob = bytes("aac-ltp-packets.bin");
        for case in manifest["videos"].as_array().unwrap() {
            qualify(
                case,
                &blob,
                &bytes(&format!(
                    "aac-ltp-{}-external-reference.f32le",
                    case["name"].as_str().unwrap()
                )),
            );
        }
    }
    #[test]
    fn authored_stereo_packets_use_native_independent_ltp_states() {
        let manifest: Value = serde_json::from_slice(&bytes("aac-ltp-pair.json")).unwrap();
        let blob = bytes("aac-ltp-pair-packets.bin");
        for case in manifest["cases"].as_array().unwrap() {
            qualify(
                case,
                &blob,
                &bytes(&format!(
                    "aac-ltp-pair-{}-scalar-reference.f32le",
                    case["name"].as_str().unwrap()
                )),
            );
        }
    }
    #[test]
    fn native_ltp_checkpoints_count_only_histories_and_right_preparation_failure_preserves_histories()
     {
        let manifest: Value = serde_json::from_slice(&bytes("aac-ltp-pair.json")).unwrap();
        let case = &manifest["cases"][3];
        let blob = bytes("aac-ltp-pair-packets.bin");
        let mut state = decoder(case["asc"].as_str().unwrap());
        let saved = state.checkpoint();
        let expected = saved.ltp_synthesis.capacity()
            * std::mem::size_of::<super::super::aac_ltp_channel::LtpChannelCheckpoint>()
            + 2 * 5 * 1024 * std::mem::size_of::<f64>()
            + saved.ltp_coupling_synthesis.capacity()
                * std::mem::size_of::<Option<super::super::aac_ltp_channel::LtpChannelCheckpoint>>(
                )
            + saved.mapping.capacity() * std::mem::size_of::<usize>()
            + saved.coupling_synthesis.capacity()
                * std::mem::size_of::<Option<LongSineSynthesis>>();
        assert_eq!(
            state
                .retained_payload_bytes_with_checkpoint(Some(&saved))
                .unwrap()
                - state.retained_payload_bytes().unwrap(),
            expected
        );
        for row in case["frames"].as_array().unwrap().iter().take(5) {
            let at = row["offset"].as_u64().unwrap() as usize;
            state
                .decode(&blob[at..at + row["bytes"].as_u64().unwrap() as usize])
                .unwrap();
        }
        let row = &case["frames"][5];
        let at = row["offset"].as_u64().unwrap() as usize;
        let packet = &blob[at..at + row["bytes"].as_u64().unwrap() as usize];
        let saved = state.checkpoint();
        let gold = state.decode(packet).unwrap();
        state.restore(&saved).unwrap();
        // Controlled right-lane geometry failure after left preparation.
        // All preparation now precedes synthesis, so no history may advance.
        // This is a state-transaction test, not a reachable valid ASC.
        let right = state.ltp_synthesis[1].clone();
        state.ltp_synthesis[1] = super::super::aac_ltp_channel::LtpChannel::new(960).unwrap();
        assert!(
            state
                .decode(packet)
                .unwrap_err()
                .to_string()
                .contains("residual geometry")
        );
        state.ltp_synthesis[1] = right;
        assert_eq!(state.decode(packet).unwrap(), gold);
    }
}
#[cfg(test)]
mod ltp_sbr_tests {
    use super::*;
    fn bytes(name: &str) -> Vec<u8> {
        std::fs::read(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../tests/fixtures/playback-errors")
                .join(name),
        )
        .unwrap()
    }
    #[test]
    fn ltp_sbr_crc_failure_keeps_core_and_qmf_histories() {
        let m: serde_json::Value = serde_json::from_slice(&bytes("aac-ltp-sbr.json")).unwrap();
        let blob = bytes("aac-ltp-sbr-packets.bin");
        for case in m["cases"].as_array().unwrap() {
            let raw: Vec<u8> = case["asc"]
                .as_str()
                .unwrap()
                .as_bytes()
                .chunks_exact(2)
                .map(|v| u8::from_str_radix(std::str::from_utf8(v).unwrap(), 16).unwrap())
                .collect();
            let rate = case["rate"].as_u64().unwrap() as u32;
            let mut state = NativeAacDecoder::new_with_output_rate(&raw, rate).unwrap();
            let packet = |index: usize| {
                let row = &case["frames"][index];
                let at = row["offset"].as_u64().unwrap() as usize;
                &blob[at..at + row["bytes"].as_u64().unwrap() as usize]
            };
            state.decode(packet(0)).unwrap();
            let saved = state.checkpoint();
            let before = state.retained_payload_bytes().unwrap();
            let error = state.decode(&bytes("aac-ltp-sbr-bad-crc.bin")).unwrap_err();
            assert!(error.to_string().contains("SBR CRC mismatch"), "{error}");
            assert_eq!(state.retained_payload_bytes().unwrap(), before);
            assert_eq!(state.sample_rate(), rate);
            let actual = state.decode(packet(1)).unwrap();
            state.restore(&saved).unwrap();
            assert_eq!(state.decode(packet(1)).unwrap(), actual);
        }
    }
    #[test]
    fn ltp_sbr_signalling_clocks_match_scalar_pcm_and_restore() {
        let m: serde_json::Value = serde_json::from_slice(&bytes("aac-ltp-sbr.json")).unwrap();
        let blob = bytes("aac-ltp-sbr-packets.bin");
        for case in m["cases"].as_array().unwrap() {
            let raw: Vec<u8> = case["asc"]
                .as_str()
                .unwrap()
                .as_bytes()
                .chunks_exact(2)
                .map(|v| u8::from_str_radix(std::str::from_utf8(v).unwrap(), 16).unwrap())
                .collect();
            let rate = case["rate"].as_u64().unwrap() as u32;
            let gold = bytes(&format!("aac-ltp-sbr-{rate}-reference.f64le"));
            let mut state = NativeAacDecoder::new_with_output_rate(&raw, rate).unwrap();
            let mut first = Vec::new();
            let n = (rate / 24000 * 1024) as usize;
            for (frame, row) in case["frames"].as_array().unwrap().iter().enumerate() {
                let at = row["offset"].as_u64().unwrap() as usize;
                let packet = &blob[at..at + row["bytes"].as_u64().unwrap() as usize];
                let saved = state.checkpoint();
                let pcm = state.decode(packet).unwrap();
                assert_eq!(pcm.len(), n);
                assert_eq!(state.sample_rate(), rate);
                for (i, &sample) in pcm.iter().enumerate() {
                    let at = (frame * n + i) * 8;
                    let expected = f64::from_le_bytes(gold[at..at + 8].try_into().unwrap());
                    assert!(
                        (f64::from(sample) - expected).abs() < 1e-9,
                        "rate={rate} signal={} frame={frame} sample={i}: {sample} vs {expected}",
                        case["signal"]
                    );
                }
                state.restore(&saved).unwrap();
                assert_eq!(state.decode(packet).unwrap(), pcm);
                first.push(pcm);
            }
            state.reset();
            for (row, expected) in case["frames"].as_array().unwrap().iter().zip(first) {
                let at = row["offset"].as_u64().unwrap() as usize;
                assert_eq!(
                    state
                        .decode(&blob[at..at + row["bytes"].as_u64().unwrap() as usize])
                        .unwrap(),
                    expected
                );
            }
        }
    }
}
