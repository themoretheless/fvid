//! Owned ELD-specific LD-SBR configuration, element association and state.
use super::{
    Result, aac_eld_sbr_dsp::Dsp, aac_sbr_header::Header, aac_sbr_history::Stream, bits::BitReader,
    invalid, memory::Footprint,
};
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
        _ => {
            return Err(invalid(
                "invalid low-delay SBR channel configuration",
            ));
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
#[derive(Clone, Debug, PartialEq)]
pub struct Decoder {
    config: Config,
    core_rate: u32,
    slots: u8,
    configuration: u8,
    groups: Vec<Group>,
    lfe: Option<Dsp>,
}
impl Decoder {
    pub fn new(
        config: Config,
        core_rate: u32,
        frame_samples: u16,
        configuration: u8,
    ) -> Result<Self> {
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
                dsp: Dsp::new(config.double_rate),
            });
        }
        let lfe = kinds.contains(&3).then(|| Dsp::new(config.double_rate));
        Ok(Self {
            config,
            core_rate,
            slots: (frame_samples / 32) as u8,
            configuration,
            groups,
            lfe,
        })
    }
    pub fn initial_config(&self) -> &Config {
        &self.config
    }
    pub fn output_rate(&self) -> u32 {
        self.core_rate * if self.config.double_rate { 2 } else { 1 }
    }
    pub fn reset(&mut self) {
        for (group, header) in self.groups.iter_mut().zip(&self.config.headers) {
            group.stream = Stream::with_low_delay_header(header.clone());
            group.dsp.reset();
        }
        if let Some(lfe) = &mut self.lfe {
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
        if let Some(lfe) = &self.lfe {
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
        let mut bits = BitReader::new(bytes);
        bits.skip(offset)?;
        let output = self.process(&mut bits, end, core)?;
        Ok((output, bits.position() - offset))
    }
    pub fn process(
        &mut self,
        bits: &mut BitReader<'_>,
        end: usize,
        core: &[Vec<f32>],
    ) -> Result<Vec<Vec<f32>>> {
        let kinds = elements(self.configuration)?;
        let channels: usize = kinds.iter().map(|&k| if k == 1 { 2 } else { 1 }).sum();
        if core.len() != channels || core.iter().any(|p| p.len() != 32 * usize::from(self.slots)) {
            return Err(invalid("low-delay SBR core channel mismatch"));
        }
        let mut state = self.clone();
        let mut trial = bits.clone();
        let mut output = Vec::with_capacity(channels);
        let mut at = 0;
        let mut group = 0;
        for &kind in kinds {
            if kind == 3 {
                output.push(
                    state
                        .lfe
                        .as_mut()
                        .ok_or_else(|| invalid("missing low-delay SBR LFE state"))?
                        .upsample_lfe(&core[at], state.slots)?,
                );
                at += 1;
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
            )?;
            let refs: Vec<_> = core[at..at + width].iter().map(Vec::as_slice).collect();
            output.extend(g.dsp.process(&frame, &refs, state.core_rate, state.slots)?);
            at += width;
            group += 1;
        }
        *self = state;
        *bits = trial;
        Ok(output)
    }
}
