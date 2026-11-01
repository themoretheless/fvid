//! Owned ELD-specific LD-SBR configuration, element association and state.
use super::{
    Result, aac_eld_sbr_dsp::Dsp, aac_sbr_header::Header, aac_sbr_history::Stream, bits::BitReader,
    invalid, memory::Footprint,
};
/// Noise-range admission is independent of ASC syntax. Strict follows the
/// normative 0..=30 range. AllowQ31 additionally reconstructs the original ISO
/// LD-SBR inverse-filter streams' five-bit 31 exactly, without clamping. Other
/// values and coupled balance limits remain strict; this is not a new profile.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum NoisePolicy {
    /// Enforce the normative reconstructed noise range.
    Strict,
    /// Also admit Q31 as an interoperability extension, preserving 2^-25.
    #[default]
    AllowQ31,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Config {
    pub double_rate: bool,
    pub crc: bool,
    pub headers: Vec<Header>,
}
fn elements(configuration: u8) -> Result<&'static [u8]> {
    Ok(match configuration {
        1 => &[0],
        2 => &[1],
        3 => &[0, 1],
        4 => &[0, 1, 0],
        5 => &[0, 1, 1],
        6 => &[0, 1, 1, 3],
        7 => &[0, 1, 1, 1, 3],
        11 => &[0, 1, 1, 0, 3],
        12 => &[0, 1, 1, 1, 3],
        14 => &[0, 1, 1, 3, 1],
        13 => &super::aac_pce::CONFIGURATION_13_ELEMENTS,
        _ => {
            return Err(invalid("invalid low-delay SBR channel configuration"));
        }
    })
}
impl Config {
    pub fn read_prefix(bytes: &[u8], offset: usize, configuration: u8) -> Result<(Self, usize)> {
        let mut bits = BitReader::new(bytes);
        bits.skip(offset)?;
        let result = Self::read(&mut bits, configuration)?;
        Ok((result, bits.position() - offset))
    }

    pub(crate) fn read(bits: &mut BitReader<'_>, configuration: u8) -> Result<Self> {
        let mut b = bits.clone();
        let double_rate = b.bit()?;
        let crc = b.bit()?;
        let count = elements(configuration)?
            .iter()
            .filter(|&&kind| kind != 3)
            .count();
        let end = b.position() + b.remaining();
        let mut headers = Vec::with_capacity(count);
        for _ in 0..count {
            headers.push(Header::read(&mut b, end)?);
        }
        *bits = b;
        Ok(Self {
            double_rate,
            crc,
            headers,
        })
    }
    pub fn output_rate(&self, core: u32) -> Result<u32> {
        core.checked_mul(if self.double_rate { 2 } else { 1 })
            .ok_or_else(|| invalid("low-delay SBR output rate overflow"))
    }
}
#[derive(Clone, Debug, PartialEq)]
struct Group {
    stream: Stream,
    dsp: Dsp,
}
enum Output {
    Pcm(Vec<Vec<f32>>),
    Qmf(Vec<Vec<Vec<super::aac_sbr_qmf::Complex>>>),
}
#[derive(Clone, Debug, PartialEq)]
pub struct Decoder {
    noise_policy: NoisePolicy,
    spatial: bool,
    config: Config,
    core_rate: u32,
    factor: usize,
    slots: u8,
    configuration: u8,
    groups: Vec<Group>,
    lfe: Vec<Dsp>,
}
impl Decoder {
    pub fn new(
        config: Config,
        core_rate: u32,
        frame_samples: u16,
        configuration: u8,
    ) -> Result<Self> {
        Self::new_downscaled(config, core_rate, frame_samples, configuration, 1)
    }
    /// Own centered low-rate filterbanks. Adapted processing requires both
    /// short core PCM and a nominal core companion for coded LF crossovers.
    pub fn new_downscaled(
        config: Config,
        core_rate: u32,
        frame_samples: u16,
        configuration: u8,
        factor: usize,
    ) -> Result<Self> {
        if !matches!(factor, 1 | 2 | 4 | 8) || !core_rate.is_multiple_of(factor as u32) {
            return Err(invalid("invalid LD-SBR downscale factor"));
        }
        let kinds = elements(configuration)?;
        if !matches!(frame_samples, 480 | 512) || core_rate == 0 {
            return Err(invalid("invalid low-delay SBR core geometry"));
        }
        let rate = core_rate
            .checked_mul(2)
            .ok_or_else(|| invalid("low-delay SBR frequency overflow"))?;
        let count = kinds.iter().filter(|&&kind| kind != 3).count();
        if count != config.headers.len() {
            return Err(invalid(
                "low-delay SBR header count disagrees with configured elements",
            ));
        }
        let mut groups = Vec::with_capacity(count);
        for header in &config.headers {
            super::aac_sbr_bands::FrequencyTables::from_header(header, rate)?;
            groups.push(Group {
                stream: Stream::with_low_delay_header(header.clone()),
                dsp: Dsp::new_downscaled(config.double_rate, factor),
            });
        }
        // Every LFE owns its analysis/synthesis history. In configuration13
        // two consecutive LFEs must not advance the same state twice per AU.
        let lfe = kinds.iter().filter(|&&kind| kind == 3)
            .map(|_| Dsp::new_downscaled(config.double_rate, factor)).collect();
        Ok(Self {
            noise_policy: NoisePolicy::default(),
            spatial: false,
            config,
            core_rate,
            factor,
            slots: (frame_samples / 32) as u8,
            configuration,
            groups,
            lfe,
        })
    }
    /// LD-MPS QMF-domain handoff. Uses the spatial LD analysis bank and
    /// returns normalized subbands without CLDFB/PCM synthesis. Output is in
    /// coded AAC element/channel order, indexed [channel][slot][band].
    pub fn new_spatial(
        config: Config,
        core_rate: u32,
        frame_samples: u16,
        configuration: u8,
    ) -> Result<Self> {
        let mut decoder = Self::new(config, core_rate, frame_samples, configuration)?;
        decoder.spatial = true;
        for group in &mut decoder.groups {
            group.dsp = Dsp::new_spatial(decoder.config.double_rate);
        }
        for lfe in &mut decoder.lfe {
            *lfe = Dsp::new_spatial(decoder.config.double_rate);
        }
        Ok(decoder)
    }
    pub fn is_spatial(&self) -> bool {
        self.spatial
    }
    pub fn process_prefix_qmf(
        &mut self,
        bytes: &[u8],
        offset: usize,
        end: usize,
        core: &[Vec<f32>],
    ) -> Result<(Vec<Vec<Vec<super::aac_sbr_qmf::Complex>>>, usize)> {
        let mut bits = BitReader::with_bit_len(bytes, end)?;
        bits.skip(offset)?;
        let output = self.process_qmf(&mut bits, end, core)?;
        Ok((output, bits.position() - offset))
    }
    pub fn process_qmf(
        &mut self,
        bits: &mut BitReader<'_>,
        end: usize,
        core: &[Vec<f32>],
    ) -> Result<Vec<Vec<Vec<super::aac_sbr_qmf::Complex>>>> {
        if !self.spatial {
            return Err(invalid("CLDFB LD-SBR cannot supply spatial QMF"));
        }
        match self.process_output(bits, end, core, None, true)? {
            Output::Qmf(output) => Ok(output),
            Output::Pcm(_) => unreachable!(),
        }
    }
    /// Current headers for syntax-only ESC reconstruction. Native synthesis
    /// commits these only after a complete valid decoded frame.
    pub fn esc_headers(
        &self,
        core_rate: u32,
        frame_samples: u16,
        configuration: u8,
    ) -> Result<Vec<Header>> {
        if self.core_rate != core_rate
            || u16::from(self.slots) * 32 != frame_samples
            || self.configuration != configuration
        {
            return Err(invalid("LD-SBR ESC context geometry mismatch"));
        }
        self.groups
            .iter()
            .map(|group| {
                group
                    .stream
                    .current_low_delay_header()
                    .cloned()
                    .ok_or_else(|| invalid("missing low-delay SBR configuration header"))
            })
            .collect()
    }
    pub fn noise_policy(&self) -> NoisePolicy { self.noise_policy }
    /// Select future noise admission while retaining the owned stream history.
    pub fn with_noise_policy(mut self, policy: NoisePolicy) -> Self { self.noise_policy = policy; self }
    pub fn initial_config(&self) -> &Config {
        &self.config
    }
    pub fn output_rate(&self) -> u32 {
        self.core_rate / self.factor as u32 * if self.config.double_rate { 2 } else { 1 }
    }
    pub fn reset(&mut self) {
        for (group, header) in self.groups.iter_mut().zip(&self.config.headers) {
            group.stream = Stream::with_low_delay_header(header.clone());
            group.dsp.reset();
        }
        for lfe in &mut self.lfe {
            lfe.reset();
        }
    }
    pub(crate) fn visit_retained(&self, f: &mut Footprint) -> std::result::Result<(), String> {
        f.vector(&self.config.headers)?;
        f.vector(&self.groups)?;
        for group in &self.groups {
            group.stream.visit_retained(f)?;
            group.dsp.visit_retained(f)?;
        }
        f.vector(&self.lfe)?;
        for lfe in &self.lfe {
            lfe.visit_retained(f)?;
        }
        Ok(())
    }
    pub fn process_prefix(
        &mut self,
        bytes: &[u8],
        offset: usize,
        end: usize,
        core: &[Vec<f32>],
    ) -> Result<(Vec<Vec<f32>>, usize)> {
        self.process_prefix_with_full(bytes, offset, end, core, None)
    }
    /// Supply the nominal companion when using an adapted decoder. Syntax
    /// and frequency tables continue to use the nominal core clock.
    pub fn process_prefix_with_full(
        &mut self,
        bytes: &[u8],
        offset: usize,
        end: usize,
        core: &[Vec<f32>],
        full: Option<&[Vec<f32>]>,
    ) -> Result<(Vec<Vec<f32>>, usize)> {
        let mut bits = BitReader::new(bytes);
        bits.skip(offset)?;
        let output = self.process_with_full(&mut bits, end, core, full)?;
        Ok((output, bits.position() - offset))
    }
    pub fn process(
        &mut self,
        bits: &mut BitReader<'_>,
        end: usize,
        core: &[Vec<f32>],
    ) -> Result<Vec<Vec<f32>>> {
        self.process_with_full(bits, end, core, None)
    }
    fn process_with_full(
        &mut self,
        bits: &mut BitReader<'_>,
        end: usize,
        core: &[Vec<f32>],
        full: Option<&[Vec<f32>]>,
    ) -> Result<Vec<Vec<f32>>> {
        if self.spatial {
            return Err(invalid("spatial LD-SBR requires QMF output"));
        }
        match self.process_output(bits, end, core, full, false)? {
            Output::Pcm(output) => Ok(output),
            Output::Qmf(_) => unreachable!(),
        }
    }
    fn process_output(
        &mut self,
        bits: &mut BitReader<'_>,
        end: usize,
        core: &[Vec<f32>],
        full: Option<&[Vec<f32>]>,
        qmf: bool,
    ) -> Result<Output> {
        let kinds = elements(self.configuration)?;
        let channels: usize = kinds.iter().map(|&k| if k == 1 { 2 } else { 1 }).sum();
        if core.len() != channels
            || core
                .iter()
                .any(|p| p.len() != 32 / self.factor * usize::from(self.slots))
        {
            return Err(invalid("low-delay SBR core channel mismatch"));
        }
        if self.factor != 1
            && full.is_none_or(|p| {
                p.len() != channels || p.iter().any(|v| v.len() != 32 * usize::from(self.slots))
            })
        {
            return Err(invalid("invalid LD-SBR companion channels"));
        }
        let mut state = self.clone();
        let mut trial = bits.clone();
        let mut output = Vec::with_capacity(if qmf { 0 } else { channels });
        let mut subbands = Vec::with_capacity(if qmf { channels } else { 0 });
        let mut at = 0;
        let mut group = 0;
        let mut lfe_index = 0;
        for &kind in kinds {
            if kind == 3 {
                let lfe = state
                    .lfe
                    .get_mut(lfe_index)
                    .ok_or_else(|| invalid("missing low-delay SBR LFE state"))?;
                if qmf {
                    subbands.push(lfe.upsample_lfe_qmf(&core[at], state.slots)?);
                } else {
                    output.push(lfe.upsample_lfe(
                        &core[at],
                        state.slots,
                        full.map(|p| p[at].as_slice()),
                    )?);
                }
                at += 1;
                lfe_index += 1;
                continue;
            }
            let width = if kind == 1 { 2 } else { 1 };
            let rate = state
                .core_rate
                .checked_mul(2)
                .ok_or_else(|| invalid("low-delay SBR frequency overflow"))?;
            let g = &mut state.groups[group];
            let frame = g.stream.read_low_delay(
                &mut trial,
                end,
                state.config.crc,
                rate,
                state.slots,
                width,
                state.noise_policy == NoisePolicy::AllowQ31,
            )?;
            let refs: Vec<_> = core[at..at + width].iter().map(Vec::as_slice).collect();
            let full_refs = full.map(|p| {
                p[at..at + width]
                    .iter()
                    .map(Vec::as_slice)
                    .collect::<Vec<_>>()
            });
            if qmf {
                subbands.extend(
                    g.dsp
                        .process_qmf(&frame, &refs, state.core_rate, state.slots)?,
                );
            } else {
                output.extend(g.dsp.process(
                    &frame,
                    &refs,
                    state.core_rate,
                    state.slots,
                    full_refs.as_deref(),
                )?);
            }
            at += width;
            group += 1;
        }
        *self = state;
        *bits = trial;
        Ok(if qmf {
            Output::Qmf(subbands)
        } else {
            Output::Pcm(output)
        })
    }
}
