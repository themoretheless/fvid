//! Apple Lossless. A packet is a frame: channel elements of one or two channels
//! each, every one of them a Rice-coded residual stream put through an adaptive
//! linear predictor, and a stereo pair carrying the mid and the difference rather
//! than the two channels it stands for. Each frame states its own predictor
//! coefficients, its own Rice history and its own sample count, so nothing carries
//! from one frame to the next and a seek lands on a frame that decodes the same
//! however it was reached.
//!
//! The geometry lives in the magic cookie, whose fields both containers spell the
//! same way and place differently: an ISO BMFF entry nests an `alac` box, whose
//! four-byte version stands before the fields, while Matroska's `CodecPrivate`
//! starts at the fields themselves. Either way the reader hands over those fields,
//! which fix the frame length, the sample depth and the three Rice parameters every
//! compressed header is measured against. The rate and the channel count come from
//! the container, which owns the timeline the samples are stamped on.
//!
//! Bits come from the top of each byte down, and no sample is aligned to a byte
//! boundary: the frame's last residual can end mid-byte and the packet simply
//! stops. A frame that reads past its own bytes, that ends without its stop tag, or
//! that claims more samples than its cookie's frame length holds is refused whole -
//! which the audio path shows as a counted rejected packet rather than as silence.

use crate::audio::{AudioDecode, AudioPacket, AudioSpec, SampleFormat};
use crate::{Result, invalid};

include!("../../crates/fvid-media/src/owned_alac_impl.rs");

impl AlacDecoder {
    pub fn spec(&self) -> AudioSpec {
        AudioSpec {
            sample_rate: self.sample_rate,
            channels: self.channels as u16,
            format: SampleFormat::F32,
        }
    }
}

impl AudioDecode for AlacDecoder {
    /// Decode the frame the packet holds. Nothing carries between frames, so a
    /// packet always stands on its own.
    fn decode_encoded(
        &mut self,
        data: &[u8],
        pts: u64,
        _duration: u64,
    ) -> Result<Option<AudioPacket>> {
        let mut samples = Vec::new();
        self.frame(data, &mut samples)?;
        let bytes: Vec<u8> = samples
            .iter()
            .flat_map(|sample| sample.to_le_bytes())
            .collect();
        Ok(Some(AudioPacket {
            data: bytes,
            pts,
            timebase_num: 1,
            timebase_den: self.sample_rate,
        }))
    }

    /// Frames are self-contained, so there is no state a seek has to undo.
    fn reset(&mut self) {}
}

#[cfg(all(test, feature = "player"))]
mod tests {
    use super::{
        AlacDecoder, Bits, COOKIE_CHANNELS, COOKIE_FIELDS, FRAME_LENGTH, FULL_SCALE, Params,
        SAMPLE_SIZE, aligned, predict, rice, scalar, sign_extend, unmix,
    };
    use crate::audio::{AudioDecode, AudioStream};
    use crate::container::mp4::Limits as Mp4Limits;
    use crate::container::webm::Limits as WebmLimits;
    use crate::playback_mp4_audio::Mp4AudioReader;
    use crate::playback_webm_audio::WebmAudioReader;
    use std::io::Cursor;

    /// The cookie one of this build's own ALAC files carries: 4096-sample frames of
    /// 24-bit stereo, and the Rice parameters the reference encoder measures them by.
    const COOKIE: [u8; COOKIE_FIELDS] = [
        0x00, 0x00, 0x10, 0x00, 0x00, 0x18, 0x28, 0x0a, 0x0e, 0x02, 0x00, 0x00, 0x00, 0x00, 0x60,
        0x04, 0x00, 0x20, 0x4c, 0xc0, 0x00, 0x00, 0xac, 0x44,
    ];

    /// Files written by this machine's FFmpeg. Every one is a whole 0.2 s tone, so
    /// the frames it holds are the frame length the cookie states plus a short last
    /// one - except the noise, which is short enough to fit two frames, and the
    /// Matroska quarter second, whose last frame the pair test reads.
    ///
    /// ```sh
    /// sine=sine=frequency=440:sample_rate=44100:duration=0.2
    /// ffmpeg -f lavfi -i "$sine"                        -c:a alac mono-16.m4a
    /// ffmpeg -f lavfi -i "$sine" -ac 2                  -c:a alac stereo-16.m4a
    /// ffmpeg -f lavfi -i "$sine"                        -sample_fmt s32p -c:a alac mono-24.m4a
    /// ffmpeg -f lavfi -i "$sine" -ac 2                  -sample_fmt s32p -c:a alac stereo-24.m4a
    /// ffmpeg -f lavfi -i "$sine" -f lavfi -i sine=frequency=880:sample_rate=44100:duration=0.2 \
    ///        -filter_complex amerge=inputs=2 -sample_fmt s32p -c:a alac stereo-pair-24.m4a
    /// ffmpeg -f lavfi -i anullsrc=r=44100:cl=mono:d=0.05 -c:a alac silence-16.m4a
    /// ffmpeg -f lavfi -i anoisesrc=color=white:sample_rate=44100:amplitude=0.95:duration=0.1 \
    ///        -ac 2 -sample_fmt s32p -c:a alac noise-24.m4a
    /// ffmpeg -f lavfi -i sine=frequency=440:sample_rate=44100:duration=0.25 \
    ///        -ac 2 -sample_fmt s32p -c:a alac stereo-24.mka
    /// ```
    const MONO_16: &[u8] = include_bytes!("../../tests/fixtures/alac/mono-16.m4a");
    const MONO_24: &[u8] = include_bytes!("../../tests/fixtures/alac/mono-24.m4a");
    const STEREO_16: &[u8] = include_bytes!("../../tests/fixtures/alac/stereo-16.m4a");
    const STEREO_24: &[u8] = include_bytes!("../../tests/fixtures/alac/stereo-24.m4a");
    const STEREO_PAIR: &[u8] = include_bytes!("../../tests/fixtures/alac/stereo-pair-24.m4a");
    const SILENCE: &[u8] = include_bytes!("../../tests/fixtures/alac/silence-16.m4a");
    const NOISE: &[u8] = include_bytes!("../../tests/fixtures/alac/noise-24.m4a");
    const MATROSKA: &[u8] = include_bytes!("../../tests/fixtures/alac/stereo-24.mka");

    /// Decode every packet a reader hands over into one list of samples per packet.
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

    /// The sum of every sample's bit pattern: one number per packet, and one that no
    /// single sample can change without showing.
    fn digest(samples: &[f32]) -> u64 {
        samples.iter().fold(0u64, |sum, sample| {
            sum.wrapping_add(u64::from(sample.to_bits()))
        })
    }

    fn m4a(bytes: &'static [u8]) -> Mp4AudioReader<Cursor<&'static [u8]>> {
        Mp4AudioReader::open(Cursor::new(bytes), Mp4Limits::default()).expect("opens")
    }

    /// The cookie's fields, and the geometry a frame of them has to hold.
    #[test]
    fn the_cookie_fields_land_where_the_format_puts_them() {
        assert_eq!(
            Params::read(&COOKIE).expect("cookie"),
            Params {
                frame_length: 4096,
                sample_size: 24,
                history_mult: 40,
                initial_history: 10,
                rice_limit: 14,
                channels: 2,
            }
        );
    }

    /// Every geometry a frame cannot be read against is named at open, so a track
    /// that will not decode says why before the player tries a packet of it.
    #[test]
    fn a_cookie_that_states_nothing_a_frame_can_hold_is_refused() {
        for (cookie, wanted) in [
            (&COOKIE[..COOKIE_FIELDS - 1][..], "shorter"),
            (&[0u8; COOKIE_FIELDS][..], "frame length"),
        ] {
            let error = AlacDecoder::new(cookie, 44_100, 2)
                .err()
                .expect("refused setup data");
            assert!(error.to_string().contains(wanted), "{error}");
        }
        // Frames longer than the encoder's own default would be paid for per packet.
        let mut long = COOKIE;
        long[FRAME_LENGTH] = 1;
        assert!(
            AlacDecoder::new(&long, 44_100, 2)
                .err()
                .expect("a 65536-sample frame")
                .to_string()
                .contains("outside")
        );
        // A depth the coding does not have is refused rather than rounded to one.
        let mut shallow = COOKIE;
        shallow[SAMPLE_SIZE] = 12;
        assert!(
            AlacDecoder::new(&shallow, 44_100, 2)
                .err()
                .expect("no 12-bit ALAC")
                .to_string()
                .contains("12 bits")
        );
        // A track whose channels disagree with its cookie cannot say which layout
        // its samples are in, and a wider track places its elements by a map this
        // decoder has not read.
        assert!(AlacDecoder::new(&COOKIE, 44_100, 1).is_err());
        let mut six = COOKIE;
        six[COOKIE_CHANNELS] = 6;
        assert!(
            AlacDecoder::new(&six, 44_100, 6)
                .err()
                .expect("no element map")
                .to_string()
                .contains("no element map")
        );
        assert!(AlacDecoder::new(&COOKIE, 0, 2).is_err());
    }

    #[test]
    fn bits_come_from_the_top_of_the_byte_and_a_short_read_shows_itself() {
        let mut bits = Bits::new(&[0b1011_0000, 0b0000_0001]);
        assert_eq!(bits.take(1), 1);
        assert_eq!(bits.take(3), 0b011);
        assert_eq!(bits.peek(4), 0, "the first byte has four bits left");
        assert_eq!(bits.take(4), 0);
        assert_eq!(bits.take(7), 0, "the second byte keeps its one bit back");
        assert_eq!(bits.take(1), 1, "the set bit is the packet's last one");
        assert_eq!(bits.left(), 0);
        assert!(!bits.past_end());
        assert_eq!(bits.take(4), 0, "past the end reads zeros");
        assert!(bits.past_end());
        let mut wide = Bits::new(&[0xff, 0xff, 0xff, 0xff]);
        assert_eq!(wide.take(32), u32::MAX);
        let mut signed = Bits::new(&[0xff, 0xff, 0xff, 0xff]);
        assert_eq!(signed.signed(32), -1);
        let mut narrow = Bits::new(&[0x80]);
        assert_eq!(narrow.signed(8), -128);
    }

    #[test]
    fn a_run_of_nine_ones_asks_for_the_value_it_could_not_hold() {
        // Nine ones, then the 16 raw bits 0x1234 - which is what a value the
        // parameter cannot hold travels as.
        let mut escaped = Bits::new(&[0xff, 0x89, 0x1a, 0x00]);
        assert_eq!(scalar(&mut escaped, 3, 16), 0x1234);
        assert_eq!(
            escaped.left(),
            7,
            "the run's nine bits and the value took 25"
        );
        // A run of two at a Rice parameter of three: 2 * 7 = 14, plus the remainder
        // 0b110 less one.
        let mut coded = Bits::new(&[0b1101_1000]);
        assert_eq!(scalar(&mut coded, 3, 16), 19);
        assert_eq!(coded.left(), 2, "the run and its remainder took six bits");
        // At a parameter of one the run is the value: no remainder is read at all.
        let mut plain = Bits::new(&[0b1110_0000]);
        assert_eq!(scalar(&mut plain, 1, 16), 3);
        assert_eq!(plain.left(), 4);
    }

    /// The Rice parameters these examples use. A multiplier of zero leaves the
    /// history where it starts, so the parameter each sample is coded by - and
    /// whether the frame reaches its run-of-equal-samples branch at all - is settled
    /// by the history the test names.
    fn params(frame_length: usize, initial_history: u32) -> Params {
        Params {
            frame_length,
            sample_size: 16,
            history_mult: 40,
            initial_history,
            rice_limit: 14,
            channels: 1,
        }
    }

    #[test]
    fn a_residual_travels_as_a_magnitude_with_its_sign_in_the_last_bit() {
        // Rice codes of one and two, at the parameter a history of 1000 gives.
        let mut bits = Bits::new(&[0b0100_1100]);
        let mut out = [0i32; 2];
        rice(&mut bits, &mut out, 16, 0, &params(2, 1000)).expect("residuals");
        assert_eq!(out, [-1, 1], "{out:?}");
    }

    /// A history below 128 says the residuals have stopped varying, and the frame
    /// answers with one length instead of one value per sample. A history this small
    /// also codes the run's own length by a parameter of its own - seven minus the
    /// history's highest bit, which for a history of ten is four.
    #[test]
    fn a_run_of_equal_samples_is_coded_once_and_the_next_one_carries_the_sign() {
        // A residual of -1, a run of two zeros, then +1: the run leaves a modifier
        // behind that flips the sign of the sample after it, so the last value
        // travels as one and reads as two.
        let mut bits = Bits::new(&[0x87, 0x00]);
        let mut out = [0i32; 4];
        rice(&mut bits, &mut out, 16, 0, &params(4, 10)).expect("residuals");
        assert_eq!(out, [-1, 0, 0, 1], "{out:?}");
        assert_eq!(bits.left(), 7);
    }

    #[test]
    fn a_zero_run_that_outruns_its_frame_is_refused() {
        // One residual, then a run of three samples in a frame that holds three. The
        // reference clamps such a run to what fits; here the whole packet is refused
        // instead, so a frame cannot quietly end short of what it promised.
        let mut bits = Bits::new(&[0x88]);
        let mut out = [0i32; 3];
        let error = rice(&mut bits, &mut out, 16, 0, &params(3, 10))
            .err()
            .expect("a run has to stop inside the frame");
        assert!(error.to_string().contains("past the end"), "{error}");
    }

    #[test]
    fn a_channel_with_no_bits_left_in_its_frame_is_refused() {
        let mut bits = Bits::new(&[]);
        let mut out = [0i32; 2];
        let error = rice(&mut bits, &mut out, 16, 0, &params(2, 10))
            .err()
            .expect("nothing to read");
        assert!(error.to_string().contains("middle"), "{error}");
    }

    #[test]
    fn the_predictor_adds_each_residual_to_its_own_prediction() {
        // A filter with nothing in it: every sample is the one one place before it
        // plus its own residual. The adaptation still walks the coefficient it is
        // shown, so the one the frame left at zero ends where the window says.
        let mut coefficients = [0i16; 32];
        let mut out = [0i32; 5];
        predict(&[3, 1, 4, 1, 5], &mut out, 16, &mut coefficients, 1, 4);
        assert_eq!(out, [3, 4, 7, 5, 12], "{out:?}");
        assert_eq!(coefficients[0], 1, "the window is the sample before");
        assert_eq!(coefficients[1], 0, "only the taps the filter has are moved");

        // With no taps at all the residuals are the samples.
        let mut plain = [0i32; 3];
        predict(&[5, -5, 1], &mut plain, 16, &mut [0i16; 32], 0, 4);
        assert_eq!(plain, [5, -5, 1]);

        // The 31-tap spelling is the plain first-order pass, whatever the residuals.
        let mut once = [0i32; 4];
        predict(&[1, 2, 3, 4], &mut once, 16, &mut [0i16; 32], 31, 4);
        assert_eq!(once, [1, 3, 6, 10]);
    }

    #[test]
    fn a_predicted_sample_is_cut_to_the_width_the_frame_codes() {
        // A residual that runs the value past eight bits is held to eight bits, the
        // width a 16-bit frame with eight extra bits codes.
        let mut out = [0i32; 3];
        predict(&[100, 100, 100], &mut out, 8, &mut [0i16; 32], 31, 4);
        assert_eq!(out, [100, -56, 44], "{out:?}");
        assert_eq!(sign_extend(0xff, 8), -1);
        assert_eq!(sign_extend(0x7f, 8), 127);
        assert_eq!(sign_extend(-1, 32), -1);
    }

    #[test]
    fn a_pair_is_unmixed_by_the_weight_in_its_header() {
        // The pair holds the mid and the difference; the left sample is the
        // difference with the weighted mid taken off it, the right one is both.
        let mut left = super::Channel {
            samples: vec![10, 20],
            ..Default::default()
        };
        let mut right = super::Channel {
            samples: vec![4, -4],
            ..Default::default()
        };
        unmix(&mut left, &mut right, 2, 1, 1);
        assert_eq!(left.samples, [12, 18]);
        assert_eq!(right.samples, [8, 22]);
        // A weight of zero says the pair was stored as two plain channels.
        let mut same = super::Channel {
            samples: vec![7],
            ..Default::default()
        };
        let mut other = super::Channel {
            samples: vec![9],
            ..Default::default()
        };
        unmix(&mut same, &mut other, 1, 15, 0);
        assert_eq!((same.samples[0], other.samples[0]), (7, 9));
    }

    /// The reference holds a 16-bit sample in a word of its own and shifts a wider
    /// one up to the top, then divides by the width it stored. Every depth ends on
    /// the same scale, and a sample written past the depth wraps the same way there
    /// as it does here.
    #[test]
    fn every_depth_reaches_the_scale_the_reference_gives_it() {
        for (sample, depth, expected) in [
            (0xffff_fff0, 16u32, -16.0 / 32768.0),
            (0x0000_8000, 16, -1.0),
            (0xffff_ffff, 16, -1.0 / 32768.0),
            (1, 16, 1.0 / 32768.0),
            (1, 20, 1.0 / 524_288.0),
            (1, 24, 1.0 / 8_388_608.0),
            (0x00ff_ffff, 24, -1.0 / 8_388_608.0),
            (0x8000_0000, 32, -1.0),
            (0x7fff_ffff, 32, 1.0 - FULL_SCALE),
        ] {
            let value = aligned(sample, depth) as f32 * FULL_SCALE;
            assert_eq!(value, expected, "sample {sample:#x} at {depth} bits");
        }
    }

    /// Packet for packet: the counts are what the container lists, and each digest
    /// is over the 32-bit patterns of the samples `ffmpeg -f f32le` decodes for the
    /// same packet - so a predictor, a Rice stream or an unmix that went wrong shows
    /// up in the packet it went wrong in.
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

    /// The first samples of one tone, as the exact bit patterns `ffmpeg -f f32le`
    /// writes them. The 24-bit file's third sample carries 0x400 in its last two
    /// bytes, the resolution a decoder that dropped the frame's extra bits loses.
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
        // The 16- and 24-bit encodings of one sine: the same values, each scaled by
        // its own depth, landing on exactly the same floats.
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

    /// The two containers spell the same coding by different names and place the
    /// same fields differently, and the frames they carry are the same frames: one
    /// reader hands over a 36-byte `alac` box, the other the 24 field bytes out of
    /// its `CodecPrivate`, and both reach the samples.
    #[test]
    fn both_containers_reach_the_same_samples_from_the_same_frames() {
        let from_mp4 = drain(&mut m4a(STEREO_24));
        let mut stream = WebmAudioReader::open(Cursor::new(MATROSKA), WebmLimits::default())
            .expect("matroska opens");
        assert_eq!(stream.codec(), "A_ALAC");
        assert_eq!(stream.extra_data().len(), COOKIE_FIELDS);
        assert_eq!(
            (stream.sample_rate(), stream.channels()),
            (44_100, 2),
            "the geometry comes from the container"
        );
        let from_mkv = drain(&mut stream);
        assert_eq!(from_mkv.len(), 3);
        // The quarter-second file's first two frames are the fifth-of-a-second one's.
        assert_eq!(from_mkv[..2], from_mp4[..2]);
        assert_eq!(from_mkv[2].len(), 2833 * 2);
        assert_eq!(digest(&from_mkv[2]), 0x0000_0ae0_8722_bca0);
    }

    /// Each name a container gives this coding reaches the decoder, with the setup
    /// data that name travels with.
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
        // No setup data states no geometry, whatever the name says.
        assert!(crate::codec::make_audio_decoder("alac", &[], rate, channels, 0).is_err());
    }

    /// A frame that stops inside itself is refused whole rather than filled out with
    /// silence: the audio path counts the rejected packet, and a decoder that
    /// guessed the rest would hide the damage.
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
            // Every refusal names itself: a packet the decoder will not guess at.
            assert!(error.to_string().contains("ALAC"), "{error}");
        }
        // The whole packet still decodes after the refusals: nothing carried over.
        let pcm = decoder
            .decode_encoded(&packet.data, 0, 0)
            .expect("frame")
            .expect("one frame per packet");
        assert_eq!(pcm.data.len(), 4096 * 2 * 4);
    }
}
