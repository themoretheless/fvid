//! Player adapter for the canonical owned bare FLAC parser.
use crate::audio::{AudioStream, AudioTrack, EncodedPacket};
use crate::Result;
#[cfg(test)]
use fvid_media::owned_flac::{crc16, crc8, frame_header};
pub use fvid_media::owned_flac::{Flac, Frame, Limits, TAG};
use std::{io::Read, time::Duration};
/// A `.flac` file read as an audio track.
pub struct FlacAudioReader {
    flac: Flac,
    packet: usize,
}

impl FlacAudioReader {
    /// Read a whole file. Every frame's subframe layout and header/footer CRC
    /// are checked before playback; PCM reconstruction remains in the decoder.
    pub fn open<R: Read>(mut reader: R, limits: Limits) -> Result<Self> {
        let mut bytes = Vec::new();
        let cap = u64::try_from(limits.file_bytes).unwrap_or(u64::MAX);
        reader
            .by_ref()
            .take(cap.saturating_add(1))
            .read_to_end(&mut bytes)?;
        Ok(Self {
            flac: Flac::parse(&bytes, &limits)?,
            packet: 0,
        })
    }

    /// The file's own shape, for a caller that wants to say what it opened.
    pub fn flac(&self) -> &Flac {
        &self.flac
    }
}

impl AudioStream for FlacAudioReader {
    fn speaker_mask(&self)->Result<Option<u32>>{Ok(Some(self.flac.channel_mask))}
    fn codec(&self) -> &str {
        self.flac.tag
    }

    /// Packets are stamped in samples, the same timescale the decoder reports.
    fn timescale(&self) -> u32 {
        self.flac.sample_rate
    }

    fn sample_rate(&self) -> u32 {
        self.flac.sample_rate
    }

    fn channels(&self) -> u16 {
        self.flac.channels
    }

    fn bits_per_sample(&self) -> u16 {
        self.flac.bits_per_sample
    }

    fn duration(&self) -> Option<Duration> {
        Some(crate::audio::sample_duration(self.flac.samples(), self.flac.sample_rate))
    }

    /// The decoder is handed the file's own `STREAMINFO`, the same 34 bytes the
    /// Matroska reader takes out of a FLAC track's private data.
    fn extra_data(&self) -> &[u8] {
        &self.flac.stream_info
    }

    fn audio_tracks(&self) -> Vec<AudioTrack> {
        vec![AudioTrack {
            sample_rate: self.flac.sample_rate,
            channels: self.flac.channels,
            name: String::new(),
            language: String::new(),
        }]
    }

    fn next_packet(&mut self) -> Result<Option<EncodedPacket>> {
        let Some(at) = self.flac.frames.get(self.packet) else {
            return Ok(None);
        };
        let packet = EncodedPacket {
            data: self.flac.packet(self.packet).to_vec(),
            pts: at.pts as i64,
            duration: i64::from(at.samples),
        };
        self.packet += 1;
        Ok(Some(packet))
    }

    fn rewind(&mut self) {
        self.packet = 0;
    }

    fn seek_to(&mut self, pts: i64) -> i64 {
        let target = pts.max(0) as u64;
        if target >= self.flac.samples() {
            self.packet = self.flac.frames.len();
            return self.flac.samples() as i64;
        }
        let index = self
            .flac
            .frames
            .partition_point(|at| at.pts <= target)
            .saturating_sub(1);
        self.packet = index;
        self.flac.frames[index].pts as i64
    }
}

#[cfg(test)]
mod tests {
    use super::{crc16, Flac, FlacAudioReader, Frame, Limits, TAG};
    use crate::audio::{AudioStream, EncodedPacket};
    use crate::codec::make_audio_decoder;

    /// The four files under `tests/fixtures/flac`, all written by this machine's
    /// FFmpeg:
    ///   ffmpeg -f lavfi -i sine=frequency=440:sample_rate=48000:duration=0.2 \
    ///          -c:a flac -frames:a 40 tests/fixtures/flac/tone.flac
    ///   ffmpeg -f lavfi -i sine=frequency=440:sample_rate=44100:duration=0.2 \
    ///          -ac 2 -c:a flac -frames:a 60 tests/fixtures/flac/stereo.flac
    ///   ffmpeg -f lavfi -i sine=frequency=440:sample_rate=44100:duration=0.2 \
    ///          -ac 2 -sample_fmt s32 -c:a flac -frames:a 60 tests/fixtures/flac/twenty-four.flac
    ///   ffmpeg -f lavfi -i sine=frequency=440:sample_rate=8000:duration=0.2 \
    ///          -c:a flac -frames:a 200 tests/fixtures/flac/small-block.flac
    /// The frame lists asserted below are ffprobe's, packet for packet, and the
    /// sample counts are what `ffmpeg -f f32le` writes for the same file.
    const TONE: &[u8] = include_bytes!("../../../tests/fixtures/flac/tone.flac");
    const STEREO: &[u8] = include_bytes!("../../../tests/fixtures/flac/stereo.flac");
    const WIDE: &[u8] = include_bytes!("../../../tests/fixtures/flac/twenty-four.flac");
    const SMALL: &[u8] = include_bytes!("../../../tests/fixtures/flac/small-block.flac");

    fn parse(bytes: &[u8]) -> Flac {
        Flac::parse(bytes, &Limits::default()).expect("parses")
    }

    fn open(bytes: &[u8]) -> FlacAudioReader {
        FlacAudioReader::open(bytes, Limits::default()).expect("opens")
    }

    fn starts(flac: &Flac) -> Vec<usize> {
        flac.frames.iter().map(|at| at.start).collect()
    }

    fn sizes(flac: &Flac) -> Vec<usize> {
        flac.frames.iter().map(|at| at.size).collect()
    }

    #[test]
    fn a_real_files_frames_are_where_the_reference_demuxer_puts_them() {
        let flac = parse(TONE);
        assert_eq!(flac.tag, TAG);
        assert_eq!(
            (flac.sample_rate, flac.channels, flac.bits_per_sample),
            (48_000, 1, 16)
        );
        // The marker, a 34-byte STREAMINFO, a 44-byte comment and an 8192-byte
        // pad, so the audio starts at byte 8286 and runs to the end in ten frames.
        assert_eq!(flac.data_start, 8286);
        assert_eq!(flac.packets(), 10);
        assert_eq!(
            starts(&flac),
            vec![8286, 8672, 9024, 9355, 9725, 10_097, 10_452, 10_783, 11_153, 11_531]
        );
        assert_eq!(
            sizes(&flac),
            vec![386, 352, 331, 370, 372, 355, 331, 370, 378, 200]
        );
        assert_eq!(flac.samples(), 9_600);
        assert_eq!(
            (flac.stated_samples, flac.stated_frame_sizes),
            (9_600, (200, 386))
        );
        assert_eq!(
            flac.packet(0),
            &TONE[8286..8672],
            "a packet is the whole frame, header included"
        );
    }

    #[test]
    fn the_short_last_frame_states_its_own_block_size() {
        let flac = parse(TONE);
        assert_eq!(
            flac.frames[..9].iter().map(|at| at.samples).sum::<u32>(),
            9 * 1024
        );
        assert_eq!(flac.frames[9].samples, 384);
        assert_eq!(flac.frames[9].pts, 9 * 1024);
        // Six bytes of header for a table block size, eight for a stated one.
        assert_eq!(flac.frames[0].size, 386);
    }

    #[test]
    fn a_stereo_stream_is_two_channels_and_its_side_stereo_frame_plays() {
        let flac = parse(STEREO);
        assert_eq!(
            (flac.sample_rate, flac.channels, flac.bits_per_sample),
            (44_100, 2, 16)
        );
        assert_eq!(
            starts(&flac),
            vec![8286, 8645, 9009, 9347, 9723, 10_067, 10_441, 10_795, 11_154]
        );
        assert_eq!(
            sizes(&flac),
            vec![359, 364, 338, 376, 344, 374, 354, 359, 270]
        );
        assert_eq!(flac.samples(), 8_820);
    }

    #[test]
    fn a_twenty_four_bit_stream_keeps_its_width() {
        let flac = parse(WIDE);
        assert_eq!((flac.channels, flac.bits_per_sample), (2, 24));
        assert_eq!(
            sizes(&flac),
            vec![1269, 1289, 1232, 1293, 1238, 1298, 1266, 1267, 842]
        );
        assert_eq!(flac.samples(), 8_820);
    }

    #[test]
    fn a_stream_at_another_rate_and_frames_that_agree_with_ffprobe() {
        let flac = parse(SMALL);
        assert_eq!((flac.sample_rate, flac.channels), (8_000, 1));
        assert_eq!(starts(&flac), vec![8286, 8702]);
        assert_eq!(sizes(&flac), vec![416, 274]);
        assert_eq!(
            flac.frames.iter().map(|at| at.samples).collect::<Vec<_>>(),
            vec![1024, 576]
        );
        assert_eq!(flac.samples(), 1_600);
    }

    #[test]
    fn every_packet_of_a_real_file_decodes_to_the_samples_the_reference_counts() {
        for (name, bytes, want) in [
            ("tone", TONE, 9_600u64),
            ("stereo", STEREO, 8_820),
            ("twenty-four", WIDE, 8_820),
            ("small-block", SMALL, 1_600),
        ] {
            let mut stream = open(bytes);
            let mut decoder = make_audio_decoder(
                stream.codec(),
                stream.extra_data(),
                stream.sample_rate(),
                stream.channels(),
                stream.bits_per_sample(),
            )
            .unwrap_or_else(|error| panic!("{name}: {error}"));
            let mut heard = 0usize;
            let mut packets = 0;
            let step = usize::from(stream.channels()) * size_of::<f32>();
            while let Some(packet) = stream.next_packet().expect("packet") {
                packets += 1;
                let Some(pcm) = decoder
                    .decode_encoded(&packet.data, packet.pts.max(0) as u64, 0)
                    .unwrap_or_else(|error| panic!("{name} packet {packets}: {error}"))
                else {
                    continue;
                };
                assert_eq!(pcm.data.len() % step, 0, "{name} holds whole frames");
                heard += pcm.data.len() / step;
            }
            assert_eq!(
                heard, want as usize,
                "{name} plays every sample the reference counts"
            );
            assert_eq!(u64::try_from(heard).expect("fits"), stream.flac().samples());
        }
    }

    #[test]
    fn the_setup_record_the_decoder_wants_is_the_files_own_stream_info() {
        let stream = open(TONE);
        // The same 34 bytes a Matroska FLAC track carries as private data, which
        // the WebM reader also hands over: the file's own block, untouched.
        assert_eq!(stream.extra_data().len(), 34);
        assert_eq!(stream.extra_data(), &TONE[8..42]);
        assert_eq!(stream.bits_per_sample(), 16);
        assert_eq!(stream.audio_tracks().len(), 1);
        assert_eq!(stream.timescale(), 48_000);
        assert_eq!(
            stream.duration().map(|d| d.as_secs_f64()).unwrap_or(0.0),
            0.2_f64.abs()
        );
    }

    #[test]
    fn seek_and_rewind_land_on_a_frame_that_starts_the_samples_there() {
        let mut stream = open(TONE);
        assert_eq!(stream.seek_to(3_000), 2 * 1024);
        let packet = stream
            .next_packet()
            .expect("packet")
            .expect("one more packet");
        assert_eq!(packet.pts, 2 * 1024);
        assert_eq!(packet.duration, 1024);
        stream.rewind();
        let first = stream
            .next_packet()
            .expect("packet")
            .expect("the first packet");
        assert_eq!(first.pts, 0);
        assert_eq!(first.data, TONE[8286..8672]);
    }

    /// A SEEKTABLE of two points, 52 bytes, put between the first frame and the
    /// second one. The format allows it; this machine's FFmpeg does not, and its
    /// own reading of the same file merges the block into the packet before it.
    fn interleaved() -> Vec<u8> {
        let mut body = Vec::new();
        for point in 0..2u64 {
            // A seek point is three eight-byte numbers: the sample it lands on, the
            // byte its frame starts at, and the frame's index.
            body.extend_from_slice(&(1024 * point).to_be_bytes());
            body.extend_from_slice(&(8286 + 386 * point).to_be_bytes());
            body.extend_from_slice(&point.to_be_bytes());
        }
        let mut block = vec![0x03, 0, 0, u8::try_from(body.len()).expect("short block")];
        block.extend_from_slice(&body);
        let mut bytes = TONE[..8672].to_vec();
        bytes.extend_from_slice(&block);
        bytes.extend_from_slice(&TONE[8672..]);
        bytes
    }

    #[test]
    fn a_metadata_block_between_frames_ends_one_and_is_not_a_packet() {
        let bytes = interleaved();
        let flac = parse(&bytes);
        assert_eq!(
            flac.packets(),
            10,
            "the same ten frames, block walked around"
        );
        assert_eq!(
            sizes(&flac),
            vec![386, 352, 331, 370, 372, 355, 331, 370, 378, 200],
            "a block between frames joins neither"
        );
        assert_eq!(flac.samples(), 9_600);
        assert_eq!(flac.packet(1), &bytes[8724..9076]);
        assert_eq!(
            starts(&flac)[1],
            8672 + 52,
            "the second frame starts after the block"
        );
    }

    #[test]
    fn a_file_that_is_not_a_flac_file_is_not_a_stream() {
        for name in ["not audio at all", "", "RIFF....WAVEfmt ", "fLa"] {
            assert!(
                Flac::parse(name.as_bytes(), &Limits::default()).is_err(),
                "{name} opened as FLAC"
            );
        }
        let video = include_bytes!("../../../tests/fixtures/audio/pcm-screen.mov");
        let error = Flac::parse(video, &Limits::default()).expect_err("a movie is not a FLAC file");
        assert!(
            error.to_string().contains("not a FLAC file"),
            "said: {error}"
        );
    }

    #[test]
    fn a_frame_with_corrupted_residual_or_checksum_is_refused() {
        let mut bytes = TONE.to_vec();
        // Corrupt the first residual. Structural framing can reject its footer
        // padding before reaching the original checksum failure.
        bytes[8400] ^= 0xff;
        let error = Flac::parse(&bytes, &Limits::default()).expect_err("damage is not a stream");
        assert!(
            error
                .to_string()
                .contains("invalid FLAC frame at byte 8286"),
            "said: {error}"
        );
    }

    #[test]
    fn a_truncated_tail_is_refused() {
        for cut in [0usize, 1, 2, 40] {
            let bytes = &TONE[..TONE.len() - cut - 2];
            assert!(
                Flac::parse(bytes, &Limits::default()).is_err(),
                "a stream cut {cut} bytes short of a frame opened anyway"
            );
        }
    }

    #[test]
    fn a_block_that_reaches_past_the_file_is_refused() {
        let mut bytes = TONE.to_vec();
        // The padding block's length: three bytes big-endian behind its header at
        // byte 90, which is the last of the leading blocks. Rewritten to eight
        // megabytes, more than the file holds.
        bytes[91] = 0x7f;
        bytes[92] = 0xff;
        bytes[93] = 0xff;
        let error =
            Flac::parse(&bytes, &Limits::default()).expect_err("a lying block is not a boundary");
        assert!(error.to_string().contains("states"), "said: {error}");
    }

    #[test]
    fn a_stream_info_that_is_too_short_to_state_a_geometry_is_refused() {
        let mut bytes = TONE.to_vec();
        bytes[5..8].copy_from_slice(&[0, 0, 20]);
        assert!(Flac::parse(&bytes, &Limits::default()).is_err());
    }

    /// Rewrite one number in `STREAMINFO`, leaving the frames untouched, so the
    /// block and the audio it describes disagree.
    fn lie_total_samples(bytes: &[u8]) -> Vec<u8> {
        let mut out = bytes.to_vec();
        // The record's tenth through seventeenth bytes pack the rate, the channel
        // code, the width and, in their low 36 bits, the total sample count: keep
        // the three above it and change only the count.
        let rest = u64::from_be_bytes(out[18..26].try_into().expect("eight bytes"));
        out[18..26].copy_from_slice(&((rest & !((1u64 << 36) - 1)) | 12_345).to_be_bytes());
        out
    }

    #[test]
    fn a_stated_sample_count_that_disagrees_with_the_frames_is_refused() {
        let error = Flac::parse(&lie_total_samples(TONE), &Limits::default())
            .expect_err("a lying total was believed");
        assert!(
            error.to_string().contains("the frames hold 9600"),
            "said: {error}"
        );
    }

    #[test]
    fn stated_frame_sizes_that_disagree_with_the_frames_are_refused() {
        let mut bytes = TONE.to_vec();
        // The largest frame size is the record's seventh through ninth bytes, at
        // absolute 15: rewritten to 256 while the frames stay 200..386.
        bytes[15] = 0x00;
        bytes[16] = 0x01;
        bytes[17] = 0x00;
        let error = Flac::parse(&bytes, &Limits::default()).expect_err("a lying size was believed");
        assert!(
            error.to_string().contains("the frames are 200..386"),
            "said: {error}"
        );
    }

    /// Move one frame's rate code to another rate, then repair the header's CRC-8
    /// so the refusal is about the geometry and not about a broken header.
    fn lie_rate(bytes: &[u8]) -> Vec<u8> {
        let mut out = bytes.to_vec();
        let at = 8672;
        // The block size is the high nibble of the second description byte and the
        // rate code the low one: this file's frames say 1024 samples at 48000 Hz
        // (0xaa), and code 9 is 44100. The header's own CRC-8 is repaired
        // afterwards, so the refusal is about the geometry, not a broken header.
        out[at + 2] = (out[at + 2] & 0xf0) | 0x09;
        let crc = super::crc8(&out[at..at + 5]);
        out[at + 5] = crc;
        assert!(
            super::frame_header(&out[at..]).is_some(),
            "the frame still reads"
        );
        out
    }

    #[test]
    fn a_frame_that_names_another_rate_is_refused() {
        let error = Flac::parse(&lie_rate(TONE), &Limits::default())
            .expect_err("a midstream rate change opened");
        assert!(
            error
                .to_string()
                .contains("frame 1 states 44100 Hz in a 48000 Hz stream"),
            "said: {error}"
        );
    }

    #[test]
    fn a_frame_that_names_another_channel_count_is_refused() {
        let mut out = TONE.to_vec();
        let at = 9024;
        // Channel code 1 is two channels in a file whose blocks say one.
        out[at + 3] = (out[at + 3] & 0xf) | 0x10;
        let crc = super::crc8(&out[at..at + 5]);
        out[at + 5] = crc;
        let error = Flac::parse(&out, &Limits::default()).expect_err("a second channel appeared");
        assert!(
            error
                .to_string()
                .contains("frame 2 carries 2 channels in a stream of 1"),
            "said: {error}"
        );
    }

    #[test]
    fn limits_cut_a_file_that_would_run_on() {
        let tight = Limits {
            file_bytes: 1000,
            packets: 1 << 20,
        };
        assert!(
            Flac::parse(TONE, &tight).is_err(),
            "a file over the byte limit opened"
        );
        let few = Limits {
            file_bytes: 128 << 20,
            packets: 3,
        };
        let error = Flac::parse(TONE, &few).expect_err("four frames came through three");
        assert!(error.to_string().contains("packet limit"), "said: {error}");
    }

    #[test]
    fn a_frame_header_needs_its_own_checksum() {
        // The sync pattern and a plausible description, but no CRC-8 that fits: a
        // scan across compressed audio has to be able to say no this often.
        let mut bytes = [0xffu8, 0xf8, 0xaa, 0x08, 0x00, 0x00];
        assert!(super::frame_header(&bytes).is_none());
        bytes[5] = super::crc8(&bytes[..5]);
        assert!(super::frame_header(&bytes).is_some());
    }

    #[test]
    fn the_checksums_are_the_ones_the_format_names() {
        // Both polynomials with zero initial value and no reflection, checked
        // against the frame that starts this file's audio.
        assert_eq!(
            crc16(&TONE[8286..8670]),
            u16::from_be_bytes([TONE[8670], TONE[8671]])
        );
        assert_eq!(super::crc8(&TONE[8286..8291]), TONE[8291]);
    }

    #[test]
    fn packets_are_whole_frames_and_their_timestamps_count_samples() {
        let mut stream = open(STEREO);
        let mut pts = 0i64;
        let mut got: Vec<EncodedPacket> = Vec::new();
        while let Some(packet) = stream.next_packet().expect("packet") {
            assert_eq!(packet.pts, pts);
            pts += packet.duration;
            assert!(!packet.data.is_empty());
            got.push(packet);
        }
        assert_eq!(got.len(), 9);
        assert_eq!(pts, 8_820);
        assert_eq!(got[0].data, STEREO[8286..8645]);
        assert_eq!(
            got.iter()
                .map(|at| Frame {
                    start: 0,
                    size: at.data.len(),
                    pts: at.pts as u64,
                    samples: u32::try_from(at.duration).expect("samples"),
                })
                .collect::<Vec<_>>(),
            stream
                .flac()
                .frames
                .iter()
                .map(|at| Frame { start: 0, ..*at })
                .collect::<Vec<_>>(),
            "the packets the reader hands over are the frames it walked"
        );
    }
}
