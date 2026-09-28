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
    noise: NoiseState,
}
impl NativeAacDecoder {
    pub fn new(asc: &[u8]) -> Result<Self> {
        let config = AacConfig::parse(asc)?;
        BandTables::for_config(&config)?;
        if !matches!(config.channels, 1 | 2) {
            return Err(unsupported(
                "owned AAC multichannel layout is not implemented",
            ));
        }
        let synthesis = (0..config.channels)
            .map(|_| LongSineSynthesis::new(config.frame_samples as usize))
            .collect::<Result<Vec<_>>>()?;
        Ok(Self {
            config,
            synthesis,
            noise: NoiseState::default(),
        })
    }
    pub fn sample_rate(&self) -> u32 {
        self.config.sample_rate
    }
    pub fn channels(&self) -> u8 {
        self.config.channels
    }
    pub fn reset(&mut self) {
        for synth in &mut self.synthesis {
            synth.reset();
        }
        self.noise.reset();
    }
    /// One raw_data_block, returning interleaved normalized floating PCM.
    /// A malformed/unsupported packet leaves all decoding state unchanged.
    pub fn decode(&mut self, packet: &[u8]) -> Result<Vec<f32>> {
        let mut bits = BitReader::new(packet);
        let mut noise = self.noise.clone();
        let mut channels = Vec::new();
        loop {
            match bits.read(3)? {
                0 => {
                    if self.config.channels != 1 || !channels.is_empty() {
                        return Err(unsupported("unexpected AAC single-channel layout"));
                    }
                    bits.read(4)?; // element tag
                    let channel = ChannelData::read(&mut bits, &self.config)?;
                    let spectrum = channel.spectrum_with_noise(&self.config, &mut noise)?;
                    let spectrum = channel.apply_tns(&self.config, spectrum)?;
                    channels.push((channel.info, spectrum));
                }
                1 => {
                    if self.config.channels != 2 || !channels.is_empty() {
                        return Err(unsupported("unexpected AAC channel-pair layout"));
                    }
                    bits.read(4)?;
                    let pair = ChannelPair::read(&mut bits, &self.config)?;
                    let (left, right) = pair.spectra_with_noise(&self.config, &mut noise)?;
                    let left = pair.left.apply_tns(&self.config, left)?;
                    let right = pair.right.apply_tns(&self.config, right)?;
                    channels.push((pair.left.info, left));
                    channels.push((pair.right.info, right));
                }
                4 => {
                    bits.read(4)?;
                    let align = bits.bit()?;
                    let mut count = bits.read(8)? as usize;
                    if count == 255 {
                        count += bits.read(8)? as usize;
                    }
                    if align {
                        bits.skip((8 - bits.position() % 8) % 8)?;
                    }
                    bits.skip(count * 8)?;
                }
                6 => {
                    let mut count = bits.read(4)? as usize;
                    if count == 15 {
                        count += bits.read(8)? as usize;
                        count -= 1;
                    }
                    if count > 0 {
                        let extension = bits.read(4)?;
                        if !matches!(extension, 0 | 1) {
                            return Err(unsupported("AAC fill extension tool is not implemented"));
                        }
                        bits.skip(count * 8 - 4)?;
                    }
                }
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
        // byte_alignment bits have no audio payload.
        let n = self.config.frame_samples as usize;
        let mut synthesis = self.synthesis.clone();
        let mut output = vec![0.0; n * channels.len()];
        let mut pcm = vec![0.0; n];
        for (index, (info, spectrum)) in channels.iter().enumerate() {
            synthesis[index].synthesize_pcm(info.sequence, info.shape, spectrum, &mut pcm)?;
            for i in 0..n {
                output[i * channels.len() + index] = pcm[i] as f32;
            }
        }
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
