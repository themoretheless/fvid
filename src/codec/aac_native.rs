//! Owned AAC-LC raw-data-block decoder. The caller supplies ASC and strips any
//! ADTS/container framing. Remaining tools are explicit errors, never fallbacks.
use crate::{Result, invalid, unsupported};
use super::aac_bands::BandTables;
use super::aac_coupling::Coupling;
use crate::native_export::default_pcm_mask;
use crate::Error;
include!("../../crates/fvid-media/src/owned_aac/aac_native_impl.rs");

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
        for (&sample, bytes) in samples.iter().zip(reference.as_chunks::<4>().0.iter()) {
            let error =
                f64::from(sample) - f64::from(f32::from_le_bytes(*bytes));
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
        for (&sample, bytes) in samples.iter().zip(reference.as_chunks::<4>().0.iter()) {
            let error =
                f64::from(sample) - f64::from(f32::from_le_bytes(*bytes));
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
        for (&sample, bytes) in samples.iter().zip(reference.as_chunks::<4>().0.iter()) {
            let error =
                f64::from(sample) - f64::from(f32::from_le_bytes(*bytes));
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
        for (i, (&sample, bytes)) in samples.iter().zip(reference.as_chunks::<4>().0.iter()).enumerate() {
            let error =
                f64::from(sample) - f64::from(f32::from_le_bytes(*bytes));
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
