//! Apple Lossless. A packet is a frame: channel elements of one or two channels
//! each, every one of them a Rice-coded residual stream put through an adaptive
//! linear predictor, and a stereo pair carrying the mid and the difference rather
//! than the two channels it stands for. Each frame states its own predictor
//! coefficients, its own Rice history and its own sample count, so nothing carries
//! from one frame to the next and a seek lands on a frame that decodes the same
//! however it was reached.

use crate::Result;
use crate::audio::{AudioDecode, AudioPacket};

pub use fvid_media::owned_alac::{AlacDecoder, COOKIE_FIELDS};

impl AudioDecode for AlacDecoder {
    /// Decode the frame the packet holds. Nothing carries between frames, so a
    /// packet always stands on its own.
    fn decode_encoded(
        &mut self,
        data: &[u8],
        pts: u64,
        _duration: u64,
    ) -> Result<Option<AudioPacket>> {
        let samples = self.decode_pcm(data)?;
        let bytes: Vec<u8> = samples
            .iter()
            .flat_map(|sample| sample.to_le_bytes())
            .collect();
        Ok(Some(AudioPacket {
            data: bytes,
            pts,
            timebase_num: 1,
            timebase_den: self.sample_rate(),
        }))
    }

    /// Frames are self-contained, so there is no state a seek has to undo.
    fn reset(&mut self) {}
}

#[cfg(all(test, feature = "player"))]
mod tests {
    use super::{AlacDecoder, COOKIE_FIELDS};
    use crate::audio::{AudioDecode, AudioStream};
    use crate::container::mp4::Limits as Mp4Limits;
    use crate::container::webm::Limits as WebmLimits;
    use crate::playback_mp4_audio::Mp4AudioReader;
    use crate::playback_webm_audio::WebmAudioReader;
    use std::io::Cursor;

    const MONO_16: &[u8] = include_bytes!("../../tests/fixtures/alac/mono-16.m4a");
    const MONO_24: &[u8] = include_bytes!("../../tests/fixtures/alac/mono-24.m4a");
    const STEREO_16: &[u8] = include_bytes!("../../tests/fixtures/alac/stereo-16.m4a");
    const STEREO_24: &[u8] = include_bytes!("../../tests/fixtures/alac/stereo-24.m4a");
    const STEREO_PAIR: &[u8] = include_bytes!("../../tests/fixtures/alac/stereo-pair-24.m4a");
    const SILENCE: &[u8] = include_bytes!("../../tests/fixtures/alac/silence-16.m4a");
    const NOISE: &[u8] = include_bytes!("../../tests/fixtures/alac/noise-24.m4a");
    const MATROSKA: &[u8] = include_bytes!("../../tests/fixtures/alac/stereo-24.mka");

    fn drain<S: AudioStream + ?Sized>(stream: &mut S) -> Vec<Vec<f32>> {
        let rate = stream.sample_rate();
        let mut decoder = crate::codec::make_audio_decoder(
            stream.codec(),
            stream.extra_data(),
            rate,
            stream.channels(),
            stream.bits_per_sample(),
        )
        .expect("ALAC decoder");
        let mut packets = Vec::new();
        while let Some(packet) = stream.next_packet().expect("packet") {
            let pcm = decoder
                .decode_encoded(&packet.data, packet.pts as u64, packet.duration as u64)
                .expect("frame")
                .expect("one frame per packet");
            assert_eq!((pcm.timebase_num, pcm.timebase_den), (1, rate));
            packets.push(
                pcm.data
                    .chunks_exact(4)
                    .map(|chunk| f32::from_le_bytes(chunk.try_into().unwrap()))
                    .collect(),
            );
        }
        packets
    }

    fn digest(samples: &[f32]) -> u64 {
        samples.iter().fold(0u64, |sum, sample| {
            sum.wrapping_add(u64::from(sample.to_bits()))
        })
    }

    fn m4a(bytes: &'static [u8]) -> Mp4AudioReader<Cursor<&'static [u8]>> {
        Mp4AudioReader::open(Cursor::new(bytes), Mp4Limits::default()).expect("opens")
    }

    #[test]
    fn every_frame_of_a_real_file_lands_on_the_references_bits() {
        for (name, bytes, counts, digests) in [
            (
                "mono 16-bit",
                MONO_16,
                &[4096, 4096, 628][..],
                &[
                    0x0000_07d3_2c93_0000,
                    0x0000_07d3_25ab_6000,
                    0x0000_013a_d171_e000,
                ][..],
            ),
            (
                "mono 24-bit",
                MONO_24,
                &[4096, 4096, 628][..],
                &[
                    0x0000_07d3_2c93_0000,
                    0x0000_07d3_25ab_6000,
                    0x0000_013a_d171_e000,
                ][..],
            ),
            (
                "stereo 16-bit",
                STEREO_16,
                &[4096, 4096, 628][..],
                &[
                    0x0000_0f9e_240c_c000,
                    0x0000_0f9e_1747_c000,
                    0x0000_0274_6099_0000,
                ][..],
            ),
            (
                "stereo 24-bit",
                STEREO_24,
                &[4096, 4096, 628][..],
                &[
                    0x0000_0f9e_23b5_4f60,
                    0x0000_0f9e_1718_b860,
                    0x0000_0274_6058_fbc0,
                ][..],
            ),
            (
                "stereo pair of two tones",
                STEREO_PAIR,
                &[4096, 4096, 628][..],
                &[
                    0x0000_0fa6_52d1_6000,
                    0x0000_0fa7_d4fc_d000,
                    0x0000_0274_1f92_5000,
                ][..],
            ),
            (
                "white noise 24-bit",
                NOISE,
                &[4096, 314][..],
                &[0x0000_0fd9_005f_0d3c, 0x0000_0135_2070_54c0][..],
            ),
            (
                "silence",
                SILENCE,
                &[2205][..],
                &[0x0000_0000_0000_0000][..],
            ),
        ] {
            let mut stream = m4a(bytes);
            let channels = usize::from(stream.channels());
            let packets = drain(&mut stream);
            assert_eq!(packets.len(), counts.len(), "{name}");
            for (index, samples) in packets.iter().enumerate() {
                assert_eq!(
                    samples.len(),
                    counts[index] * channels,
                    "{name} packet {index}"
                );
                assert_eq!(digest(samples), digests[index], "{name} packet {index}");
            }
        }
    }

    #[test]
    fn a_tones_first_samples_are_the_ones_the_references_bytes_hold() {
        for (bytes, expected) in [
            (
                STEREO_16,
                [
                    0x0000_0000,
                    0x0000_0000,
                    0x3bb5_0000,
                    0x3bb5_0000,
                    0x3c34_8000,
                    0x3c34_8000,
                ],
            ),
            (
                STEREO_24,
                [
                    0x0000_0000,
                    0x0000_0000,
                    0x3bb5_0400,
                    0x3bb5_0400,
                    0x3c34_aa00,
                    0x3c34_aa00,
                ],
            ),
            (
                NOISE,
                [
                    0xbe41_5e28,
                    0xbe41_5e28,
                    0x3f1f_31c0,
                    0x3f1f_31c0,
                    0x3e2f_bff8,
                    0x3e2f_bff8,
                ],
            ),
        ] {
            let packets = drain(&mut m4a(bytes));
            let patterns: Vec<u32> = packets[0][..6]
                .iter()
                .map(|sample| sample.to_bits())
                .collect();
            assert_eq!(patterns, expected.to_vec());
        }
    }

    #[test]
    fn a_tone_at_either_depth_holds_the_same_samples() {
        let shallow = drain(&mut m4a(MONO_16));
        let wide = drain(&mut m4a(MONO_24));
        assert_eq!(shallow, wide);
    }

    #[test]
    fn silence_decodes_to_silence() {
        let packets = drain(&mut m4a(SILENCE));
        assert_eq!(packets.len(), 1);
        assert!(packets[0].iter().all(|sample| *sample == 0.0));
        assert_eq!(packets[0].len(), 2205);
    }

    #[test]
    fn both_containers_reach_the_same_samples_from_the_same_frames() {
        let from_mp4 = drain(&mut m4a(STEREO_24));
        let mut stream = WebmAudioReader::open(Cursor::new(MATROSKA), WebmLimits::default())
            .expect("matroska opens");
        assert_eq!(stream.codec(), "A_ALAC");
        assert_eq!(stream.extra_data().len(), COOKIE_FIELDS);
        assert_eq!((stream.sample_rate(), stream.channels()), (44_100, 2));
        let from_mkv = drain(&mut stream);
        assert_eq!(from_mkv.len(), 3);
        assert_eq!(from_mkv[..2], from_mp4[..2]);
        assert_eq!(from_mkv[2].len(), 2833 * 2);
        assert_eq!(digest(&from_mkv[2]), 0x0000_0ae0_8722_bca0);
    }

    #[test]
    fn each_container_spelling_of_the_name_reaches_this_decoder() {
        let stream = m4a(STEREO_16);
        let (codec, extra_data, rate, channels) = (
            stream.codec().to_string(),
            stream.extra_data().to_vec(),
            stream.sample_rate(),
            stream.channels(),
        );
        assert_eq!(codec, "alac");
        let packet = m4a(STEREO_16)
            .next_packet()
            .expect("packet")
            .expect("a packet");
        let mut decoded = Vec::new();
        for name in ["alac", "A_ALAC"] {
            let mut decoder =
                crate::codec::make_audio_decoder(name, &extra_data, rate, channels, 0).expect(name);
            let pcm = decoder
                .decode_encoded(&packet.data, packet.pts as u64, packet.duration as u64)
                .expect(name)
                .expect("one frame per packet");
            decoded.push(pcm.data);
        }
        assert_eq!(decoded[0], decoded[1]);
        assert_eq!(decoded[0].len(), 4096 * 2 * 4);
        assert!(crate::codec::make_audio_decoder("alac", &[], rate, channels, 0).is_err());
    }

    #[test]
    fn a_frame_short_of_its_own_bytes_is_refused() {
        let mut stream = m4a(STEREO_24);
        let packet = stream.next_packet().expect("packet").expect("a packet");
        let mut decoder = AlacDecoder::new(stream.extra_data(), 44_100, 2).expect("decoder");
        for cut in [
            &packet.data[..packet.data.len() / 2],
            &packet.data[..1],
            &packet.data[..0],
        ] {
            let error = decoder
                .decode_encoded(cut, 0, 0)
                .err()
                .expect("a frame has to hold all of itself");
            assert!(error.to_string().contains("ALAC"), "{error}");
        }
        let pcm = decoder
            .decode_encoded(&packet.data, 0, 0)
            .expect("frame")
            .expect("one frame per packet");
        assert_eq!(pcm.data.len(), 4096 * 2 * 4);
    }
}
