//! Owned Layer III frame decoder: bounded reservoir through interleaved PCM.
use crate::{
    Result, invalid,
    mp3_frame::{Frame, Granule, Reservoir, Version},
    mp3_huffman::Cursor,
    mp3_hybrid::{Hybrid, Window},
    mp3_scalefactors::{self, ScaleFactors},
    mp3_spectral::{self, Bands},
    mp3_stereo,
};

/// Exclusive frequency-line endpoints of Huffman regions (before reorder).
pub fn region_ends(version: Version, rate: u32, g: &Granule) -> Result<[usize; 2]> {
    let bands = Bands::for_rate(rate)?;
    if let Some((first, second)) = g.regions {
        if g.window != Window::Normal || usize::from(first) + usize::from(second) > 20 {
            return Err(invalid("invalid Layer III explicit region counts"));
        }
        Ok([
            bands.long[usize::from(first) + 1],
            bands.long[usize::from(first) + usize::from(second) + 2],
        ])
    } else {
        if g.window == Window::Normal {
            return Err(invalid("missing Layer III long region counts"));
        }
        let boundary = if g.window == Window::Short {
            if rate == 8000 { 72 } else { 36 }
        } else if version == Version::Mpeg1 {
            36
        } else if rate == 8000 {
            108
        } else {
            54
        };
        Ok([boundary, 576])
    }
}

pub struct Pcm {
    pub sample_rate: u32,
    pub channels: u16,
    pub samples: Vec<f32>,
}
/// A metadata marker cannot turn a frame with coded audio into a header.
pub fn is_info_frame(packet: &[u8]) -> Result<bool> {
    Ok(Frame::parse_info(packet)?.is_some())
}
#[derive(Clone, Default)]
pub struct Decoder {
    reservoir: Reservoir,
    hybrid: [Hybrid; 2],
    geometry: Option<(Version, u32, u16)>,
    free_base: Option<usize>,
}
impl Decoder {
    /// Conservative live decoder, transactional staging, PCM and Huffman-tree
    /// admission. Encoded input/container allocations must be added by callers.
    pub fn decode_admission_bytes() -> usize {
        3 * std::mem::size_of::<Self>() + 32 * 1024 + crate::mp3_huffman::admission_bytes()
    }

    /// Preserve history-only Xing/Info payload before the first audio frame.
    /// Placeholder header geometry does not configure the audible stream.
    pub fn prime_info(&mut self, packet: &[u8]) -> Result<()> {
        if self.geometry.is_some() || crate::mp3_gapless::tag_offset(packet)?.is_none() {
            return Err(invalid(
                "MP3 info priming requires an initial metadata frame",
            ));
        }
        let frame = Frame::parse_info(packet)?
            .ok_or_else(|| invalid("MP3 info priming cannot discard coded audio"))?;
        let staged = self.reservoir.prepare(&frame)?;
        self.reservoir.commit(staged)
    }
    pub fn reset(&mut self) {
        self.reservoir.reset();
        for hybrid in &mut self.hybrid {
            hybrid.reset();
        }
        self.geometry = None;
        self.free_base = None;
    }
    /// Decode one complete bounded MP3 frame. All history commits only after
    /// both channels/granules and the final PCM conversion succeed.
    pub fn decode(&mut self, packet: &[u8]) -> Result<Pcm> {
        let frame = Frame::parse(packet)?;
        let h = frame.header;
        let geometry = (h.version, h.sample_rate, h.channels);
        if self.geometry.is_some_and(|old| old != geometry) {
            return Err(invalid("Layer III geometry changed without decoder reset"));
        }
        let free_base = h
            .free
            .then(|| h.frame_bytes - usize::from((packet[2] >> 1) & 1));
        if self.geometry.is_some() && self.free_base != free_base {
            return Err(invalid(
                "Layer III free-format frame spacing changed without reset",
            ));
        }
        let staged = self.reservoir.prepare(&frame)?;
        let bands = Bands::for_rate(h.sample_rate)?;
        let mut hybrids = self.hybrid.clone();
        let mut previous: [Option<ScaleFactors>; 2] = [None, None];
        let mut output = Vec::new();
        output
            .try_reserve_exact(h.samples() * usize::from(h.channels))
            .map_err(|_| invalid("Layer III PCM allocation failed"))?;
        let mut index = 0;
        for granule in 0..h.granules() {
            let mut spectra = [[0.0; 576]; 2];
            let mut scales: [Option<ScaleFactors>; 2] = [None, None];
            for channel in 0..usize::from(h.channels) {
                let g = &frame.side.granules[granule][channel];
                let mut cursor = Cursor::new(&staged.bytes, staged.ranges[index].clone())?;
                index += 1;
                let sf = mp3_scalefactors::read(
                    &mut cursor,
                    h.version,
                    g,
                    granule as u8,
                    frame.side.scfsi[channel],
                    previous[channel].as_ref(),
                    h.mode == 1 && h.mode_extension & 1 != 0 && channel == 1,
                )?;
                let quantized = cursor.spectrum(
                    g.big_values,
                    region_ends(h.version, h.sample_rate, g)?,
                    g.tables,
                    u8::from(g.count1_table),
                )?;
                spectra[channel] =
                    mp3_spectral::reconstruct(&quantized.values, h.sample_rate, g, &sf)?;
                previous[channel] = Some(sf.clone());
                scales[channel] = Some(sf);
            }
            if h.channels == 2 {
                spectra = mp3_stereo::process(
                    &spectra[0],
                    &spectra[1],
                    h.version,
                    h.sample_rate,
                    &frame.side.granules[granule][0],
                    &frame.side.granules[granule][1],
                    scales[1]
                        .as_ref()
                        .ok_or_else(|| invalid("missing right scalefactors"))?,
                    h.mode,
                    h.mode_extension,
                )?;
            }
            let mut pcm = [[0.0; 576]; 2];
            for channel in 0..usize::from(h.channels) {
                let g = &frame.side.granules[granule][channel];
                pcm[channel] = hybrids[channel].process(
                    &spectra[channel],
                    g.window,
                    if g.mixed { bands.mixed_subbands() } else { 0 },
                )?;
            }
            for sample in 0..576 {
                for channel in 0..usize::from(h.channels) {
                    let value = pcm[channel][sample] as f32;
                    if !value.is_finite() {
                        return Err(invalid("Layer III PCM overflow"));
                    }
                    output.push(value);
                }
            }
        }
        self.reservoir.commit(staged)?;
        self.hybrid = hybrids;
        self.geometry = Some(geometry);
        self.free_base = free_base;
        Ok(Pcm {
            sample_rate: h.sample_rate,
            channels: h.channels,
            samples: output,
        })
    }
}
