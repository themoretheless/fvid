//! Owned mono AAC Main/LC/SSR/LTP core with SBR/PS raw-data-block decoding.
//! Stereo PCM is delayed one LC or two SSR packets for alignment/lookahead; frame_index
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
#[derive(Clone)]
struct SsrFrame {
    prepared: aac_sbr_ps::PreparedFrame,
    sources: Vec<Option<aac_sbr_history::Frame>>,
}
#[derive(Clone)]
struct SsrState {
    synthesis: super::aac_ssr_synthesis::SsrSynthesis,
    alignment: super::aac_ssr_alignment::SsrPcmAlignment,
    prepared: std::collections::VecDeque<SsrFrame>,
    tags: Vec<u8>,
}
#[derive(Clone, Default)]
struct CceState {
    stream: aac_sbr_history::Stream,
    dsp: aac_sbr_dsp::Dsp,
    synthesis: Option<LongSineSynthesis>,
    ltp: Option<super::aac_ltp_channel::LtpChannel>,
    ssr_synthesis: Option<super::aac_ssr_synthesis::SsrSynthesis>,
    prediction: Option<super::aac_main_predictor::MainPredictor>,
}
#[derive(Clone)]
pub struct NativePsAacDecoder {
    config: AacConfig,
    initial_program: Option<aac_pce::ProgramConfig>,
    program: Option<aac_pce::ProgramConfig>,
    output_rate: u32,
    mode: OutputRate,
    requires_in_band: bool,
    synthesis: Option<LongSineSynthesis>,
    ltp: Option<super::aac_ltp_channel::LtpChannel>,
    ssr: Option<SsrState>,
    noise: NoiseState,
    prediction: Option<super::aac_main_predictor::MainPredictor>,
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
        let synthesis = if matches!(parsed.core.object_type, 3 | 4) {
            None
        } else {
            Some(LongSineSynthesis::new(usize::from(
                parsed.core.frame_samples,
            ))?)
        };
        let ssr = if parsed.core.object_type == 3 {
            Some(SsrState {
                synthesis: super::aac_ssr_synthesis::SsrSynthesis::new()?,
                alignment: super::aac_ssr_alignment::SsrPcmAlignment::new(1, 1)?,
                prepared: Default::default(),
                tags: Vec::new(),
            })
        } else {
            None
        };
        let ltp = if parsed.core.object_type == 4 {
            Some(super::aac_ltp_channel::LtpChannel::new(usize::from(parsed.core.frame_samples))?)
        } else {None};
        let prediction = main_prediction(&parsed.core)?;
        Ok(Self {
            config: parsed.core,
            prediction,
            initial_program: parsed.program.clone(),
            program: parsed.program,
            output_rate,
            mode,
            requires_in_band,
            synthesis,
            ltp,
            ssr,
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
        self.ssr
            .as_ref()
            .and_then(|s| s.prepared.back().map(|p| p.prepared.frame_index()))
            .or_else(|| self.extension.pending_frame_index())
    }
    pub fn checkpoint(&self) -> Checkpoint {
        Checkpoint {
            state: self.clone(),
        }
    }
    pub fn restore(&mut self, checkpoint: &Checkpoint) -> Result<()> {
        if self.config != checkpoint.state.config
            || self.initial_program != checkpoint.state.initial_program
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
        self.program = self.initial_program.clone();
        if let Some(synthesis) = &mut self.synthesis {
            synthesis.reset();
        }
        if let Some(ssr) = &mut self.ssr {
            ssr.synthesis.reset();
            ssr.alignment.reset();
            ssr.prepared.clear();
            ssr.tags.clear();
            ssr.alignment = super::aac_ssr_alignment::SsrPcmAlignment::new(1, 1)
                .expect("fixed mono SSR alignment geometry");
        }
        if let Some(ltp) = &mut self.ltp { ltp.reset(); }
        self.noise.reset();
        if let Some(bank) = &mut self.prediction { bank.reset(); }
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
    /// LC retains one packet; SSR additionally aligns variable window PCM.
    /// Frame identity always refers to the original packet, including EOF.
    /// Malformed trailing syntax rolls back core, extension and pending PCM.
    pub fn decode(&mut self, packet: &[u8]) -> Result<Option<Frame>> {
        let mut trial = self.clone();
        let mut bits = BitReader::new(packet);
        let mut core = None;
        let mut ltp_prediction = None;
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
                    let channel = if trial.config.object_type == 4 {
                        let (channel, prediction) = ChannelData::read_ltp(&mut bits, &trial.config)?;
                        ltp_prediction = prediction;
                        channel
                    } else { ChannelData::read(&mut bits, &trial.config)? };
                    let mut spectrum = channel.spectrum_with_noise(&trial.config, &mut trial.noise)?;
                    if let Some(bank) = &mut trial.prediction {
                        channel.predict_main(&trial.config, bank, &mut spectrum)?;
                    }
                    core = Some((channel, spectrum, tag));
                    previous_channel = Some((0, tag));
                }
                2 => {
                    let (coupling, ltp_data) = if trial.config.object_type == 4 {
                        Coupling::read_ltp(&mut bits, &trial.config)?
                    } else { (Coupling::read(&mut bits, &trial.config)?, None) };
                    validate_coupling(trial.program.as_ref(), &coupling, &mut cce_tags)?;
                    let mut spectrum = coupling
                        .channel
                        .spectrum_with_noise(&trial.config, &mut trial.noise)?;
                    if trial.config.object_type == 1 {
                        let state = trial.cce_states[coupling.tag as usize].get_or_insert_with(CceState::default);
                        if state.prediction.is_none() { state.prediction = main_prediction(&trial.config)?; }
                        coupling.channel.predict_main(&trial.config, state.prediction.as_mut().unwrap(), &mut spectrum)?;
                    }
                    let spectrum = if trial.config.object_type == 4 {
                        let state = trial.cce_states[coupling.tag as usize].get_or_insert_with(CceState::default);
                        if state.ltp.is_none() {
                            let mut source = trial.ltp.as_ref().ok_or_else(|| invalid("missing PS LTP state"))?.clone();
                            source.reset();
                            state.ltp = Some(source);
                        }
                        prepare_ltp(state.ltp.as_mut().unwrap(), &trial.config, &coupling.channel, ltp_data.as_ref(), spectrum)?
                    } else { coupling.channel.apply_tns(&trial.config, spectrum)? };
                    previous_channel = Some((2, coupling.tag));
                    couplings.push((coupling, spectrum));
                }
                4 => aac_pce::skip_data_stream(&mut bits)?,
                5 => {
                    trial.program = Some(read_program(&mut bits, trial.program.as_ref())?);
                }
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
                spectrum = if let Some(ltp) = &mut trial.ltp {
                    prepare_ltp(ltp, &trial.config, &channel, ltp_prediction.as_ref(), spectrum)?
                } else { channel.apply_tns(&trial.config, spectrum)? };
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
                    if trial.config.object_type == 3
                        && channel.info.shape != coupling.channel.info.shape
                    {
                        return Err(invalid("AAC SSR dependent coupling window shape mismatch"));
                    }
                    coupling.mix_spectrum(target, &trial.config, source, &mut spectrum)?;
                }
            }
        }
        let n = usize::from(trial.config.frame_samples);
        let mut pcm = if trial.ssr.is_some() {
            vec![0.; super::aac_ssr_synthesis::SsrSynthesis::output_samples(channel.info.sequence)]
        } else {
            vec![0.; n]
        };
        if let Some(ssr) = &mut trial.ssr {
            let gain = channel
                .gain
                .clone()
                .unwrap_or(super::aac_gain_control::GainControl { bands: Vec::new() })
                .into();
            ssr.synthesis.synthesize_pcm(
                channel.info.sequence,
                channel.info.shape,
                &gain,
                &spectrum,
                &mut pcm,
            )?;
        } else if let Some(ltp) = &mut trial.ltp {
            pcm = ltp.synthesize_spectrum(&spectrum, channel.info.sequence, channel.info.shape)?;
        } else {
            trial
                .synthesis
                .as_mut()
                .ok_or_else(|| invalid("missing PS LC synthesis"))?
                .synthesize_pcm(
                    channel.info.sequence,
                    channel.info.shape,
                    &spectrum,
                    &mut pcm,
                )?;
        }
        let pcm: Vec<f32> = pcm.into_iter().map(|v| v as f32).collect();
        if pcm.iter().any(|v| !v.is_finite()) {
            return Err(invalid("PS AAC core PCM exceeds finite f32 output"));
        }
        if trial.ssr.is_some() {
            let prepared = if let Some((position, end, crc)) = extension {
                let mut reader = BitReader::new(packet);
                reader.skip(position)?;
                let prepared =
                    trial
                        .extension
                        .prepare(&mut reader, end, crc, rate, slots, trial.mode)?;
                if reader.position() != end {
                    return Err(invalid("invalid SBR fill extension consumption"));
                }
                prepared
            } else {
                trial
                    .extension
                    .prepare_upsampling(rate, slots, trial.mode)?
            };
            use super::aac_ssr_alignment::{LaneInput, OutputGain};
            use super::aac_ssr_synthesis::SsrSynthesis;
            let stamp = prepared.frame_index();
            let mut coded = Vec::new();
            for (coupling, spectrum) in &couplings {
                if coupling.point != 3 {
                    continue;
                }
                let mut outputs = Vec::new();
                for target in &coupling.targets {
                    validate_coupling_target(target, tag)?;
                    outputs.push(OutputGain {
                        channel: 0,
                        gain: target.gain,
                    });
                }
                let state =
                    trial.cce_states[coupling.tag as usize].get_or_insert_with(CceState::default);
                if state.ssr_synthesis.is_none() {
                    state.ssr_synthesis = Some(SsrSynthesis::new()?);
                }
                let mut source =
                    vec![0.; SsrSynthesis::output_samples(coupling.channel.info.sequence)];
                let gain = coupling
                    .channel
                    .gain
                    .clone()
                    .unwrap_or(super::aac_gain_control::GainControl { bands: Vec::new() })
                    .into();
                state.ssr_synthesis.as_mut().unwrap().synthesize_pcm(
                    coupling.channel.info.sequence,
                    coupling.channel.info.shape,
                    &gain,
                    spectrum,
                    &mut source,
                )?;
                let source: Vec<f32> = source.into_iter().map(|v| v as f32).collect();
                coded.push((coupling.tag, source, outputs));
            }
            let ssr = trial.ssr.as_mut().unwrap();
            let mut tags = ssr.tags.clone();
            tags.extend(coded.iter().map(|c| c.0));
            tags.sort_unstable();
            tags.dedup();
            if tags != ssr.tags {
                let order: Vec<_> = std::iter::once(Some(0))
                    .chain(tags.iter().map(|tag| {
                        ssr.tags
                            .iter()
                            .position(|old| old == tag)
                            .map(|index| index + 1)
                    }))
                    .collect();
                ssr.alignment.extend_lanes(&order)?;
                ssr.tags = tags;
            }
            let mut lanes = vec![pcm];
            let mut outputs = vec![vec![OutputGain {
                channel: 0,
                gain: 1.0,
            }]];
            for (index, tag) in ssr.tags.iter().enumerate() {
                if let Some((_, source, gains)) = coded.iter().find(|c| c.0 == *tag) {
                    lanes.push(source.clone());
                    outputs.push(gains.clone());
                } else {
                    lanes.push(vec![0.; ssr.alignment.absent_input_rows(index + 1, n)?]);
                    outputs.push(Vec::new());
                }
            }
            let inputs: Vec<_> = lanes
                .iter()
                .zip(&outputs)
                .map(|(samples, outputs)| LaneInput { samples, outputs })
                .collect();
            ssr.prepared.push_back(SsrFrame {
                prepared,
                sources: cce_frames,
            });
            let aligned = ssr.alignment.submit_sources(stamp, n, &inputs)?;
            let output = trial.aligned_output(aligned)?;
            *self = trial;
            return Ok(output);
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
            let core = if let Some(ltp) = &mut state.ltp {
                ltp.synthesize_spectrum(&spectrum, coupling.channel.info.sequence, coupling.channel.info.shape)?
            } else {
                if state.synthesis.is_none() {state.synthesis = Some(LongSineSynthesis::new(n)?);}
                let mut core = vec![0.; n];
                state.synthesis.as_mut().unwrap().synthesize_pcm(
                    coupling.channel.info.sequence, coupling.channel.info.shape, &spectrum, &mut core,
                )?;
                core
            };
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
    fn aligned_output(
        &mut self,
        aligned: Option<super::aac_ssr_alignment::AlignedSources>,
    ) -> Result<Option<Frame>> {
        let Some(aligned) = aligned else {
            return Ok(None);
        };
        let ssr = self
            .ssr
            .as_mut()
            .ok_or_else(|| invalid("missing PS SSR alignment"))?;
        let metadata = ssr
            .prepared
            .front()
            .ok_or_else(|| invalid("missing PS SSR prepared frame"))?;
        if metadata.prepared.frame_index() != aligned.stamp || aligned.rows != 1024 {
            return Err(invalid("PS SSR aligned frame identity mismatch"));
        }
        let target: Vec<_> = aligned.lanes[0]
            .iter()
            .flat_map(|c| c.samples.iter().copied())
            .collect();
        if target.len() != 1024 {
            return Err(invalid("PS SSR aligned target length mismatch"));
        }
        let output = self
            .extension
            .process_prepared(&metadata.prepared, &target)?;
        let rate = self
            .config
            .sample_rate
            .checked_mul(2)
            .ok_or_else(|| invalid("PS frequency overflow"))?;
        let ratio = if self.mode == OutputRate::Core { 1 } else { 2 };
        let mut left = if ssr.tags.is_empty() {
            Vec::new()
        } else {
            vec![0f32; 1024 * ratio]
        };
        for (index, &tag) in ssr.tags.iter().enumerate() {
            let chunks = &aligned.lanes[index + 1];
            let samples: Vec<_> = chunks
                .iter()
                .flat_map(|c| c.samples.iter().copied())
                .collect();
            if samples.len() != 1024 {
                return Err(invalid("PS SSR aligned source length mismatch"));
            }
            let state = self.cce_states[tag as usize].get_or_insert_with(CceState::default);
            let rendered = if let Some(frame) = &metadata.sources[tag as usize] {
                state.dsp.process(frame, &[&samples], rate, 16, self.mode)?
            } else {
                state
                    .dsp
                    .process_upsampling(&[&samples], rate, 16, self.mode)?
            };
            if rendered[0].len() != left.len() {
                return Err(invalid("SBR coupling output length mismatch"));
            }
            let mut offset = 0;
            for chunk in chunks {
                let end = offset + chunk.samples.len() * ratio;
                for (dest, &sample) in left[offset..end].iter_mut().zip(&rendered[0][offset..end]) {
                    for gain in &chunk.outputs {
                        *dest += sample as f32 * gain.gain;
                        if !dest.is_finite() {
                            return Err(invalid("AAC coupled PCM exceeds finite f32 output"));
                        }
                    }
                }
                offset = end;
            }
        }
        ssr.prepared.pop_front();
        let output = self.coupled_output(output)?;
        if !left.is_empty() {
            self.pending_coupling = Some((aligned.stamp, left));
        }
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
        let output = if let Some(ssr) = &mut trial.ssr {
            let aligned = ssr.alignment.finish_sources()?;
            match trial.aligned_output(aligned)? {
                Some(output) => Some(output),
                None => {
                    let output = trial.extension.finish()?;
                    trial.coupled_output(output)?
                }
            }
        } else {
            let output = trial.extension.finish()?;
            trial.coupled_output(output)?
        };
        *self = trial;
        Ok(output)
    }
}

/// Transactional syntax-only PS presence negotiation. No core synthesis, QMF,
/// hybrid, decorrelation or stereo PCM is allocated or computed by this probe.
#[derive(Clone)]
pub struct InBandPsProbe {
    config: AacConfig,
    initial_program: Option<aac_pce::ProgramConfig>,
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
            initial_program: parsed.program.clone(),
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
        self.program = self.initial_program.clone();
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
                    if trial.config.object_type == 4 {
                        ChannelData::read_ltp(&mut bits, &trial.config)?;
                    } else {ChannelData::read(&mut bits, &trial.config)?;}
                    core = true;
                    previous_channel = Some((0, tag));
                }
                2 => {
                    let coupling = if trial.config.object_type == 4 {
                        Coupling::read_ltp(&mut bits, &trial.config)?.0
                    } else {Coupling::read(&mut bits, &trial.config)?};
                    validate_coupling(trial.program.as_ref(), &coupling, &mut tags)?;
                    let tag = trial.program.as_ref().map_or(0, |p| p.elements[0].tag);
                    for target in &coupling.targets {
                        validate_coupling_target(target, tag)?;
                    }
                    previous_channel = Some((2, coupling.tag));
                }
                4 => aac_pce::skip_data_stream(&mut bits)?,
                5 => {
                    trial.program = Some(read_program(&mut bits, trial.program.as_ref())?);
                }
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

fn prepare_ltp(
    state: &mut super::aac_ltp_channel::LtpChannel,
    config: &AacConfig,
    channel: &ChannelData,
    prediction: Option<&super::aac_ltp_syntax::LtpData>,
    spectrum: Vec<f32>,
) -> Result<Vec<f32>> {
    let tables = BandTables::for_config(config)?;
    let short = channel.info.sequence == super::aac_synthesis::WindowSequence::EightShort;
    let offsets = if short {tables.short} else {tables.long};
    let limit = BandTables::tns_limit(config.sample_rate, short).min(channel.info.max_sfb as usize);
    state.prepare_spectrum(spectrum, prediction, channel.info.sequence, channel.info.shape,
        offsets, limit, channel.tns.as_ref())
}

fn main_prediction(config: &AacConfig) -> Result<Option<super::aac_main_predictor::MainPredictor>> {
    if config.object_type != 1 { return Ok(None); }
    let tables = BandTables::for_config(config)?;
    let bands = tables.prediction_limit.ok_or_else(|| invalid("AAC Main prediction band limit missing"))?;
    Ok(Some(super::aac_main_predictor::MainPredictor::new(tables.long[bands])?))
}

fn validate_mono_program(parsed: &AudioSpecificConfig) -> Result<()> {
    if !matches!(parsed.core.object_type, 1 | 2 | 3 | 4) {
        return Err(unsupported(
            "AAC parametric stereo core profile is not implemented",
        ));
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
fn read_program(bits: &mut BitReader<'_>, expected: Option<&aac_pce::ProgramConfig>) -> Result<aac_pce::ProgramConfig> {
    let program = aac_pce::ProgramConfig::read(bits, 0)?;
    let expected = expected
        .ok_or_else(|| unsupported("in-band PS PCE needs an explicit configured program"))?;
    if program.elements != expected.elements
        || program.sample_rate != expected.sample_rate
        || program.object_type != expected.object_type
        || program.height_layers()? != expected.height_layers()?
        || program.pcm_layout()? != expected.pcm_layout()?
    {
        return Err(invalid("PS AAC in-band PCE changed the configured layout"));
    }
    // CCE roster is packet syntax state. Existing tag histories and queued PCM
    // survive removal; a returning tag resumes its own synthesis state.
    Ok(program)
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
