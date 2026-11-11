//! Independent Vorbis packet-to-PCM decode; container clocks remain caller-owned.
use crate::vorbis::{Floor, Identification, PacketBits, PacketMode, Setup};
use crate::vorbis_synthesis::{Overlap, window};
use crate::{Result, invalid};

fn resize_zeroed<T: Clone>(values: &mut Vec<T>, count: usize, value: T) -> Result<()> {
    values.clear();
    if values.capacity() < count {
        values
            .try_reserve_exact(count - values.len())
            .map_err(|_| invalid("cannot allocate Vorbis decode workspace"))?;
    }
    values.resize(count, value);
    Ok(())
}

fn zeroed<T: Clone>(count: usize, value: T) -> Result<Vec<T>> {
    let mut values = Vec::new();
    resize_zeroed(&mut values, count, value)?;
    Ok(values)
}

/// Stateful packet decoder. Output is interleaved f32 in Vorbis channel order.
/// The first packet primes overlap and returns zero samples. Granule trimming,
/// stream gains and output speaker permutation belong to the container adapter.
pub struct Decoder {
    identification: Identification,
    setup: Setup,
    overlap: Vec<Overlap>,
    workspace_bytes: usize,
    envelopes: Vec<Vec<f32>>,
    spectra: Vec<Vec<f32>>,
    nonzero: Vec<bool>,
    floor_scratch: Vec<u8>,
    submap_flags: Vec<bool>,
    imdct_scratch: Vec<(f32, f32)>,
    time: Vec<f32>,
    taper: Vec<f32>,
    channel_pcm: Vec<Vec<f32>>,
}
impl Decoder {
    pub fn new(
        identification: Identification,
        setup: Setup,
        workspace_bytes: usize,
    ) -> Result<Self> {
        if identification.channels == 0
            || identification.sample_rate == 0
            || identification
                .block_exponents
                .iter()
                .any(|e| !(6..=13).contains(e))
            || identification.block_exponents[0] > identification.block_exponents[1]
        {
            return Err(invalid("invalid Vorbis decoder identification"));
        }
        let channels = usize::from(identification.channels);
        if setup.mappings.iter().any(|m| m.mux.len() != channels) {
            return Err(invalid("Vorbis decoder mapping channel mismatch"));
        }
        let mut overlap = Vec::new();
        overlap
            .try_reserve_exact(channels)
            .map_err(|_| invalid("cannot allocate Vorbis channel state"))?;
        overlap.resize_with(channels, Overlap::default);
        let mut envelopes = Vec::new();
        envelopes
            .try_reserve_exact(channels)
            .map_err(|_| invalid("cannot allocate Vorbis floor channels"))?;
        envelopes.resize_with(channels, Vec::new);
        let mut spectra = Vec::new();
        spectra
            .try_reserve_exact(channels)
            .map_err(|_| invalid("cannot allocate Vorbis spectrum channels"))?;
        spectra.resize_with(channels, Vec::new);
        let mut channel_pcm = Vec::new();
        channel_pcm
            .try_reserve_exact(channels)
            .map_err(|_| invalid("cannot allocate Vorbis output channels"))?;
        channel_pcm.resize_with(channels, Vec::new);
        Ok(Self {
            identification,
            setup,
            overlap,
            workspace_bytes,
            envelopes,
            spectra,
            nonzero: zeroed(channels, false)?,
            floor_scratch: Vec::new(),
            submap_flags: Vec::new(),
            imdct_scratch: Vec::new(),
            time: Vec::new(),
            taper: Vec::new(),
            channel_pcm,
        })
    }
    pub fn channels(&self) -> u8 {
        self.identification.channels
    }
    pub fn sample_rate(&self) -> u32 {
        self.identification.sample_rate
    }
    pub fn reset(&mut self) {
        for state in &mut self.overlap {
            state.reset();
        }
    }
    pub fn decode(&mut self, packet: &[u8]) -> Result<Vec<f32>> {
        let mut bits = PacketBits::new(packet);
        let header = PacketMode::parse(&mut bits, &self.setup.modes)?;
        let mode = self.setup.modes[header.mode];
        let mapping = self
            .setup
            .mappings
            .get(usize::from(mode.mapping))
            .ok_or_else(|| invalid("invalid Vorbis mode mapping"))?;
        let channels = usize::from(self.identification.channels);
        let n = 1usize << self.identification.block_exponents[usize::from(mode.long)];
        // Includes spectral/floor vectors, window/time buffers, overlap history,
        // returned channel PCM, interleaved output, FFT and small vector overhead.
        let base = channels * n * 32 + n * 40 + 64 * 1024;
        if base > self.workspace_bytes {
            return Err(invalid("Vorbis frame exceeds workspace budget"));
        }
        let residue_values = (self.workspace_bytes - base) / 16;
        self.nonzero.fill(false);
        resize_zeroed(&mut self.floor_scratch, n / 2, 0u8)?;
        for channel in 0..channels {
            let submap = mapping
                .submaps
                .get(usize::from(mapping.mux[channel]))
                .ok_or_else(|| invalid("invalid Vorbis channel submap"))?;
            let floor = self
                .setup
                .floors
                .get(usize::from(submap.floor))
                .ok_or_else(|| invalid("invalid Vorbis submap floor"))?;
            resize_zeroed(&mut self.envelopes[channel], n / 2, 0f32)?;
            let used = match floor {
                Floor::One(floor) => match floor.decode_y(&mut bits, &self.setup.codebooks)? {
                    Some(y) => {
                        floor.curve(&y, &mut self.envelopes[channel], &mut self.floor_scratch)?;
                        true
                    }
                    None => false,
                },
                Floor::Zero(floor) => {
                    match floor.decode_lsp(&mut bits, &self.setup.codebooks, residue_values)? {
                        Some(lsp) => {
                            floor.curve(&lsp, &mut self.envelopes[channel])?;
                            true
                        }
                        None => false,
                    }
                }
            };
            self.nonzero[channel] = used;
        }
        mapping.propagate_nonzero(&mut self.nonzero)?;
        for channel in 0..channels {
            resize_zeroed(&mut self.spectra[channel], n / 2, 0f32)?;
        }
        for (index, submap) in mapping.submaps.iter().enumerate() {
            let residue = self
                .setup
                .residues
                .get(usize::from(submap.residue))
                .ok_or_else(|| invalid("invalid Vorbis submap residue"))?;
            self.submap_flags.clear();
            if self.submap_flags.capacity() < channels {
                self.submap_flags
                    .try_reserve_exact(channels - self.submap_flags.len())
                    .map_err(|_| invalid("cannot allocate Vorbis submap flags"))?;
            }
            for (channel, &mux) in mapping.mux.iter().enumerate() {
                if usize::from(mux) == index {
                    self.submap_flags.push(self.nonzero[channel]);
                }
            }
            let mut bundle = Vec::new();
            bundle
                .try_reserve_exact(channels)
                .map_err(|_| invalid("cannot allocate Vorbis submap bundle"))?;
            for (channel, spectrum) in self.spectra.iter_mut().enumerate() {
                if usize::from(mapping.mux[channel]) == index {
                    bundle.push(spectrum.as_mut_slice());
                }
            }
            residue.decode(
                &mut bits,
                &self.setup.codebooks,
                &self.submap_flags,
                &mut bundle,
                residue_values,
            )?;
        }
        mapping.inverse_coupling(&mut self.spectra)?;
        resize_zeroed(&mut self.imdct_scratch, n / 4, (0f32, 0f32))?;
        resize_zeroed(&mut self.time, n, 0f32)?;
        resize_zeroed(&mut self.taper, n, 0f32)?;
        window(
            1usize << self.identification.block_exponents[0],
            mode.long,
            header.previous_long,
            header.next_long,
            &mut self.taper,
        )?;
        for channel in 0..channels {
            for (value, &floor) in self.spectra[channel]
                .iter_mut()
                .zip(&self.envelopes[channel])
            {
                *value *= floor;
            }
            crate::vorbis_imdct::inverse_fast(
                &self.spectra[channel],
                &mut self.time,
                &mut self.imdct_scratch,
            )?;
            for (value, &weight) in self.time.iter_mut().zip(&self.taper) {
                *value *= weight;
            }
            self.overlap[channel].push_into(&self.time, &mut self.channel_pcm[channel])?;
        }
        let frames = self.channel_pcm.first().map_or(0, Vec::len);
        if self.channel_pcm.iter().any(|v| v.len() != frames) {
            return Err(invalid("Vorbis channel overlap clocks disagree"));
        }
        let mut interleaved = zeroed(frames * channels, 0f32)?;
        for (frame, to) in interleaved.chunks_exact_mut(channels).enumerate() {
            for (channel, value) in to.iter_mut().enumerate() {
                *value = self.channel_pcm[channel][frame];
            }
        }
        Ok(interleaved)
    }
}

/// Split the player/library's concatenated headers or Matroska Xiph-laced
/// identification/comment/setup configuration without searching payload markers.
pub fn configuration_headers(configuration: &[u8]) -> Result<(&[u8], &[u8])> {
    if configuration.starts_with(b"\x01vorbis") {
        if configuration.len() < 30 {
            return Err(invalid("truncated Vorbis identification configuration"));
        }
        return Ok((&configuration[..30], &configuration[30..]));
    }
    if configuration.first() != Some(&2) {
        return Err(invalid("invalid Vorbis Xiph header count"));
    }
    let mut at = 1usize;
    let mut sizes = [0usize; 2];
    for size in &mut sizes {
        loop {
            let byte = *configuration
                .get(at)
                .ok_or_else(|| invalid("truncated Vorbis Xiph lacing"))?;
            at += 1;
            *size = size
                .checked_add(usize::from(byte))
                .ok_or_else(|| invalid("Vorbis Xiph header size overflow"))?;
            if byte < 255 {
                break;
            }
        }
    }
    let ident_end = at
        .checked_add(sizes[0])
        .ok_or_else(|| invalid("Vorbis identification size overflow"))?;
    let setup_start = ident_end
        .checked_add(sizes[1])
        .ok_or_else(|| invalid("Vorbis comment size overflow"))?;
    let identification = configuration
        .get(at..ident_end)
        .ok_or_else(|| invalid("Vorbis identification exceeds configuration"))?;
    let comment = configuration
        .get(ident_end..setup_start)
        .ok_or_else(|| invalid("Vorbis comment exceeds configuration"))?;
    if !comment.starts_with(b"\x03vorbis") {
        return Err(invalid("missing Vorbis comment configuration"));
    }
    let setup = configuration
        .get(setup_start..)
        .ok_or_else(|| invalid("missing Vorbis setup configuration"))?;
    Ok((identification, setup))
}
