//! Owned AAC-LC raw-data-block decoder. The caller supplies ASC and strips any
//! ADTS/container framing. Remaining tools are explicit errors, never fallbacks.
use super::{
    aac_bands::BandTables, aac_channel::ChannelData, aac_noise::NoiseState, aac_pair::ChannelPair,
    aac_synthesis::LongSineSynthesis, bits::BitReader, config::AacConfig,
};
use crate::{Result, invalid, unsupported};

pub struct NativeAacDecoder {
    config: AacConfig,
    synthesis: Vec<LongSineSynthesis>,
    coupling_synthesis: Vec<Option<LongSineSynthesis>>,
    noise: NoiseState,
    program: Option<super::aac_pce::ProgramConfig>,
    mapping: Vec<usize>,
    channel_mask: u32,
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
}
impl NativeAacDecoder {
    pub fn new(asc: &[u8]) -> Result<Self> {
        let (config, program) = AacConfig::parse_with_program(asc)?;
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
                _ => unreachable!(),
            };
            (
                if config.channel_configuration == 7 {
                    0xff
                } else if config.channel_configuration == 11 {
                    0x13f
                } else if config.channel_configuration == 12 {
                    0x63f
                } else {
                    crate::native_export::default_pcm_mask(u16::from(config.channels))?
                },
                mapping.to_vec(),
            )
        };
        let synthesis = (0..config.channels)
            .map(|_| LongSineSynthesis::new(config.frame_samples as usize).map_err(crate::Error::from))
            .collect::<Result<Vec<_>>>()?;
        Ok(Self {
            config,
            synthesis,
            coupling_synthesis:vec![None;16],
            noise: NoiseState::default(),
            program,
            mapping,
            channel_mask,
        })
    }
    pub fn sample_rate(&self) -> u32 {
        self.config.sample_rate
    }
    pub fn channels(&self) -> u8 {
        self.config.channels
    }
    pub fn channel_mask(&self) -> u32 {
        self.channel_mask
    }
    /// Save overlap/window and perceptual-noise history after a complete packet.
    pub fn checkpoint(&self) -> AacCheckpoint {
        AacCheckpoint {config:self.config.clone(),program:self.program.clone(),
            synthesis:self.synthesis.clone(),coupling_synthesis:self.coupling_synthesis.clone(),noise:self.noise.clone(),
            mapping:self.mapping.clone(),channel_mask:self.channel_mask}
    }
    /// Restore without changing the decoder if configuration/layout differs.
    pub fn restore(&mut self, state:&AacCheckpoint) -> Result<()> {
        if self.config!=state.config || self.program!=state.program || self.mapping!=state.mapping || self.channel_mask!=state.channel_mask {
            return Err(invalid("AAC checkpoint configuration mismatch"));
        }
        self.synthesis=state.synthesis.clone();self.coupling_synthesis=state.coupling_synthesis.clone();self.noise=state.noise.clone();
        Ok(())
    }
    pub fn reset(&mut self) {
        for synth in &mut self.synthesis {
            synth.reset();
        }
        for synthesis in self.coupling_synthesis.iter_mut().flatten() {synthesis.reset();}
        self.noise.reset();
    }
    /// One raw_data_block, returning interleaved normalized floating PCM.
    /// A malformed/unsupported packet leaves all decoding state unchanged.
    pub fn decode(&mut self, packet: &[u8]) -> Result<Vec<f32>> {
        let mut bits = BitReader::new(packet);
        let mut noise = self.noise.clone();
        let mut channels = Vec::new();
        let mut decoded_elements=Vec::new();let mut couplings=Vec::new();
        let elements: &[u32] = match self.config.channel_configuration {
            1 => &[0],
            2 => &[1],
            3 => &[0, 1],
            4 => &[0, 1, 0],
            5 => &[0, 1, 1],
            6 => &[0, 1, 1, 3],
            7 | 12 => &[0, 1, 1, 1, 3],
            11 => &[0, 1, 1, 0, 3],
            _ => &[],
        };
        let mut element_index = 0;
        let mut tags = std::collections::HashSet::new();
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
                    target_offset =
                        found.ok_or_else(|| invalid("AAC element is absent from PCE"))?;
                }
                decoded_elements.push((element,tag,target_offset));
                if !tags.insert((element, tag)) {
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
                    let coupling=super::aac_coupling::Coupling::read(&mut bits,&self.config)?;
                    if self.program.as_ref().is_none_or(|p|!p.coupling.contains(&(coupling.point == 3,coupling.tag))) {return Err(invalid("AAC coupling is absent from configured PCE"));}
                    if !tags.insert((2,u32::from(coupling.tag))) {return Err(invalid("duplicate AAC coupling tag"));}
                    let spectrum=coupling.channel.spectrum_with_noise(&self.config,&mut noise)?;
                    let spectrum=coupling.channel.apply_tns(&self.config,spectrum)?;
                    couplings.push((coupling,spectrum));
                }
                4 => super::aac_pce::skip_data_stream(&mut bits)?,
                5 => {
                    let program = super::aac_pce::ProgramConfig::read(&mut bits, 0)?;
                    let expected = self.program.as_ref().ok_or_else(|| {
                        unsupported("in-band PCE needs an explicit configured program")
                    })?;
                    if program.coupling != expected.coupling || program.elements != expected.elements
                        || program.sample_rate != expected.sample_rate
                        || program.object_type != expected.object_type
                        || program.pcm_layout()?.0 != self.channel_mask
                    {
                        return Err(invalid("AAC in-band PCE changed the configured layout"));
                    }
                }
                6 => super::aac_pce::skip_fill(&mut bits)?,
                7 => break,
                _ => return Err(unsupported("AAC raw-data-block element is not implemented")),
            }
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
                if coupling.point != point {continue;}
                for target in &coupling.targets {
                    let (_, _, offset) = decoded_elements.iter().find(|(kind,tag,_)| *kind == u32::from(target.pair) && *tag == u32::from(target.tag)).ok_or_else(||invalid("AAC coupling target is absent"))?;
                    let mapped = self.mapping[*offset + target.channel as usize];
                    let (channel, destination, _) = channels.iter_mut().find(|(_,_,index)| *index == mapped).ok_or_else(||invalid("AAC coupling target is absent"))?;
                    if channel.info.sequence != coupling.channel.info.sequence {
                        return Err(invalid("AAC coupling target window sequence mismatch"));
                    }
                    coupling.mix_spectrum(target, &self.config, source, destination)?;
                }
            }
        }
        // byte_alignment bits have no audio payload.
        let n = self.config.frame_samples as usize;
        let mut synthesis = self.synthesis.clone();
        let mut output = vec![0.0; n * channels.len()];
        let mut pcm = vec![0.0; n];
        for (channel, spectrum, target) in &channels {
            synthesis[*target].synthesize_pcm(channel.info.sequence, channel.info.shape, spectrum, &mut pcm)?;
            for i in 0..n {
                output[i * channels.len() + target] = pcm[i] as f32;
            }
        }
        let mut coupling_synthesis=self.coupling_synthesis.clone();
        for (coupling,spectrum) in couplings {
            if coupling.point != 3 {continue;}
            let state=&mut coupling_synthesis[coupling.tag as usize];
            if state.is_none() {*state=Some(LongSineSynthesis::new(n)?);}
            state.as_mut().unwrap().synthesize_pcm(coupling.channel.info.sequence,coupling.channel.info.shape,&spectrum,&mut pcm)?;
            for target in coupling.targets {
                let kind=u32::from(target.pair);
                let (_,_,offset)=decoded_elements.iter().find(|(k,t,_)|*k==kind && *t==u32::from(target.tag)).ok_or_else(||invalid("AAC coupling target is absent"))?;
                let channel=self.mapping[*offset+target.channel as usize];
                for i in 0..n {output[i*channels.len()+channel]+=pcm[i] as f32*target.gain;}
            }
        }
        self.coupling_synthesis=coupling_synthesis;
        self.synthesis = synthesis;
        self.noise = noise;
        Ok(output)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn packets() -> Vec<&'static [u8]> {
        let data = include_bytes!("../../tests/fixtures/audio/aac-stereo.aac");
        let mut at = 0;
        let mut packets = Vec::new();
        while at < data.len() {
            let n = ((data[at + 3] as usize & 3) << 11)
                | ((data[at + 4] as usize) << 3)
                | (data[at + 5] as usize >> 5);
            packets.push(&data[at + 7..at + n]);
            at += n;
        }
        packets
    }
    #[test]
    fn checkpoint_restores_exact_continuous_pcm_and_rejects_other_config() {
        let packets=packets();assert!(packets.len()>4);
        let mut decoder=NativeAacDecoder::new(&[0x11,0x90]).unwrap();
        for packet in &packets[..3] {decoder.decode(packet).unwrap();}
        let state=decoder.checkpoint();
        let tail:Vec<_>=packets[3..].iter().map(|p|decoder.decode(p).unwrap()).collect();
        decoder.reset();decoder.restore(&state).unwrap();
        for (packet,expected) in packets[3..].iter().zip(&tail) {
            let actual=decoder.decode(packet).unwrap();
            assert_eq!(actual.iter().map(|v|v.to_bits()).collect::<Vec<_>>(),expected.iter().map(|v|v.to_bits()).collect::<Vec<_>>());
        }
        let mut other=NativeAacDecoder::new(&[0x12,0x10]).unwrap();
        assert!(other.restore(&state).is_err());
        let mut untouched=NativeAacDecoder::new(&[0x12,0x10]).unwrap();
        assert_eq!(other.decode(packets[0]).unwrap(),untouched.decode(packets[0]).unwrap());
    }
    #[test]
    fn public_decoder_matches_saved_pcm_and_reset() {
        let mut decoder = NativeAacDecoder::new(&[0x11, 0x90]).unwrap();
        let mut samples = Vec::new();
        for packet in packets() {
            samples.extend(decoder.decode(packet).unwrap());
        }
        let reference = include_bytes!("../../tests/fixtures/audio/aac-stereo-reference.f32le");
        assert_eq!(samples.len() * 4, reference.len());
        let mut squared = 0.0;
        let mut peak = 0.0f64;
        for (&sample, bytes) in samples.iter().zip(reference.chunks_exact(4)) {
            let error =
                f64::from(sample) - f64::from(f32::from_le_bytes(bytes.try_into().unwrap()));
            squared += error * error;
            peak = peak.max(error.abs());
        }
        assert!((squared / samples.len() as f64).sqrt() < 0.00015);
        assert!(peak < 0.003);
        decoder.reset();
        assert_eq!(decoder.decode(packets()[0]).unwrap(), samples[..2048]);
    }
    #[test]
    fn mono_44100_matches_pcm_reference() {
        let data = include_bytes!("../../tests/fixtures/audio/aac-mono-44k.aac");
        let reference = include_bytes!("../../tests/fixtures/audio/aac-mono-reference.f32le");
        let mut decoder = NativeAacDecoder::new(&[0x12, 0x08]).unwrap();
        assert_eq!((decoder.sample_rate(), decoder.channels()), (44100, 1));
        let mut at = 0;
        let mut samples = Vec::new();
        while at < data.len() {
            let n = ((data[at + 3] as usize & 3) << 11)
                | ((data[at + 4] as usize) << 3)
                | (data[at + 5] as usize >> 5);
            samples.extend(
                decoder
                    .decode(&data[at + 7..at + n])
                    .unwrap_or_else(|e| panic!("offset {at}: {e}")),
            );
            at += n;
        }
        assert_eq!(samples.len() * 4, reference.len());
        let mut squared = 0.0;
        let mut peak = 0.0f64;
        for (&sample, bytes) in samples.iter().zip(reference.chunks_exact(4)) {
            let error =
                f64::from(sample) - f64::from(f32::from_le_bytes(bytes.try_into().unwrap()));
            squared += error * error;
            peak = peak.max(error.abs());
        }
        let rms = (squared / samples.len() as f64).sqrt();
        assert!(rms < 0.00004, "RMS {rms}");
        assert!(peak < 0.0003, "peak {peak}");
    }
    #[test]
    fn packet_tns_filters_spectrum_before_synthesis() {
        use crate::codec::aac_huffman_tables::*;
        let fields = [
            (0u32, 3u8),
            (0, 4),
            (100, 8),
            (0, 1),
            (0, 2),
            (0, 1),
            (1, 6),
            (0, 1),
            (1, 4),
            (1, 5),
            (SCF_CODEBOOK_CODES[60], SCF_CODEBOOK_LENS[60]),
            (0, 1),
            (1, 1), // pulse absent, TNS present
            (1, 2),
            (0, 1),
            (49, 6),
            (1, 5),
            (0, 1),
            (0, 1),
            (1, 3),
            (0, 1),
            (SPECTRUM_CODEBOOK1_CODES[80], SPECTRUM_CODEBOOK1_LENS[80]),
            (7, 3),
        ];
        let mut packet = Vec::new();
        let mut n = 0;
        for (v, w) in fields {
            for b in (0..w).rev() {
                if n % 8 == 0 {
                    packet.push(0);
                }
                *packet.last_mut().unwrap() |= ((v >> b & 1) as u8) << (7 - n % 8);
                n += 1;
            }
        }
        let mut decoder = NativeAacDecoder::new(&[0x11, 0x88]).unwrap();
        let actual = decoder.decode(&packet).unwrap();
        let coefficient = (std::f64::consts::FRAC_PI_2 / 3.5).sin();
        let mut spectrum = vec![0.0; 1024];
        let mut previous = 0.0;
        for v in &mut spectrum[..4] {
            previous = 1.0 - coefficient * previous;
            *v = previous as f32;
        }
        let mut expected = vec![0.0; 1024];
        LongSineSynthesis::new(1024)
            .unwrap()
            .synthesize_pcm(
                crate::codec::aac_synthesis::WindowSequence::OnlyLong,
                crate::codec::aac_synthesis::WindowShape::Sine,
                &spectrum,
                &mut expected,
            )
            .unwrap();
        assert_eq!(
            actual,
            expected.iter().map(|&v| v as f32).collect::<Vec<_>>()
        );
    }
    #[test]
    fn real_tns_file_matches_pcm_reference() {
        let data = include_bytes!("../../tests/fixtures/audio/aac-tns.aac");
        let reference = include_bytes!("../../tests/fixtures/audio/aac-tns-reference.f32le");
        let config = AacConfig::parse(&[0x11, 0x88]).unwrap();
        let mut decoder = NativeAacDecoder::new(&[0x11, 0x88]).unwrap();
        let mut at = 0;
        let mut samples = Vec::new();
        let mut active = 0;
        while at < data.len() {
            let n = ((data[at + 3] as usize & 3) << 11)
                | ((data[at + 4] as usize) << 3)
                | (data[at + 5] as usize >> 5);
            let packet = &data[at + 7..at + n];
            let mut bits = BitReader::new(packet);
            loop {
                let element = bits.read(3).unwrap();
                if element == 6 {
                    let mut count = bits.read(4).unwrap() as usize;
                    if count == 15 {
                        count += bits.read(8).unwrap() as usize;
                        count -= 1;
                    }
                    bits.skip(count * 8).unwrap();
                } else {
                    assert_eq!(element, 0);
                    bits.read(4).unwrap();
                    let channel = ChannelData::read(&mut bits, &config).unwrap();
                    if let Some(tns) = channel.tns {
                        active += tns
                            .windows
                            .iter()
                            .flatten()
                            .filter(|f| !f.lpc.is_empty())
                            .count();
                    }
                    break;
                }
            }
            samples.extend(decoder.decode(packet).unwrap());
            at += n;
        }
        assert_eq!(active, 2, "fixture must exercise active TNS");
        assert_eq!(samples.len() * 4, reference.len());
        let mut squared = 0.0;
        let mut peak = 0.0f64;
        for (&sample, bytes) in samples.iter().zip(reference.chunks_exact(4)) {
            let error =
                f64::from(sample) - f64::from(f32::from_le_bytes(bytes.try_into().unwrap()));
            squared += error * error;
            peak = peak.max(error.abs());
        }
        let rms = (squared / samples.len() as f64).sqrt();
        assert!(rms < 0.0000001, "RMS {rms}");
        assert!(peak < 0.000001, "peak {peak}");
    }
    #[test]
    fn surround_pcm_matches_reference_channel_order() {
        let data = include_bytes!("../../tests/fixtures/audio/aac-51-active.aac");
        let reference = include_bytes!("../../tests/fixtures/audio/aac-51-reference.f32le");
        let mut decoder = NativeAacDecoder::new(&[0x11, 0xb0]).unwrap();
        let mut at = 0;
        let mut samples = Vec::new();
        while at < data.len() {
            let n = ((data[at + 3] as usize & 3) << 11)
                | ((data[at + 4] as usize) << 3)
                | (data[at + 5] as usize >> 5);
            samples.extend(
                decoder
                    .decode(&data[at + 7..at + n])
                    .unwrap_or_else(|e| panic!("offset {at}: {e}")),
            );
            at += n;
        }
        assert_eq!(samples.len() * 4, reference.len());
        let mut squared = [0.0; 6];
        let mut peak = [0.0f64; 6];
        for (i, (&sample, bytes)) in samples.iter().zip(reference.chunks_exact(4)).enumerate() {
            let error =
                f64::from(sample) - f64::from(f32::from_le_bytes(bytes.try_into().unwrap()));
            squared[i % 6] += error * error;
            peak[i % 6] = peak[i % 6].max(error.abs());
        }
        for ch in 0..6 {
            let rms = (squared[ch] / (samples.len() / 6) as f64).sqrt();
            assert!(rms < 1e-7, "channel {ch} RMS {rms}");
            assert!(peak[ch] < 1e-6, "channel {ch} peak {}", peak[ch]);
        }
    }
    #[test]
    fn bad_packet_does_not_advance_noise_or_overlap() {
        let packets = packets();
        let mut decoder = NativeAacDecoder::new(&[0x11, 0x90]).unwrap();
        let mut reference = NativeAacDecoder::new(&[0x11, 0x90]).unwrap();
        decoder.decode(packets[0]).unwrap();
        reference.decode(packets[0]).unwrap();
        let mut bad = packets[1].to_vec();
        bad.extend([0, 0]);
        assert!(decoder.decode(&bad).is_err());
        assert!(decoder.decode(&[]).is_err());
        assert_eq!(
            decoder.decode(packets[1]).unwrap(),
            reference.decode(packets[1]).unwrap()
        );
    }
}
