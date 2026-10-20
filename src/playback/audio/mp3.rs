//! Player adapter for the canonical owned bare MPEG audio parser.
use crate::audio::{AudioStream, AudioTrack, EncodedPacket};
use crate::{Result, invalid};
use std::{io::Read, time::Duration};
pub use fvid_media::owned_mpeg_audio::{Frame, Layer, Limits, Mp3, LAYER_I, LAYER_II, LAYER_III};

/// A `.mp3` file read as an audio track.
pub struct Mp3AudioReader {
    mp3: Mp3,
    packet: usize,
    presentation_floor: Option<u64>,
}

impl Mp3AudioReader {
    /// Read a whole file. The frames lie at lengths only their own headers state,
    /// so all of them are walked before the first one plays.
    pub fn open<R: Read>(mut reader: R, limits: Limits) -> Result<Self> {
        let mut bytes = Vec::new();
        let cap = u64::try_from(limits.file_bytes).unwrap_or(u64::MAX);
        reader.by_ref().take(cap.saturating_add(1)).read_to_end(&mut bytes)?;
        Ok(Self {
            mp3: Mp3::parse_owned(bytes, &limits)?,
            packet: 0,
            presentation_floor: None,
        })
    }

    /// The file's own shape, for a caller that wants to say what it opened.
    pub fn mp3(&self) -> &Mp3 {
        &self.mp3
    }
}

impl AudioStream for Mp3AudioReader {
    fn make_decoder(&self) -> Result<Box<dyn crate::audio::AudioDecode>> {
        if self.mp3.tag == LAYER_III {
            let mut decoder = crate::codec::mp3_decoder::Mp3Decoder::new(self.sample_rate(), self.channels())?;
            if let Some(packet) = self.mp3.info_packet() { decoder.prime_info(packet)?; }
            Ok(Box::new(decoder))
        } else {
            crate::codec::make_audio_decoder(self.codec(), self.extra_data(), self.sample_rate(), self.channels(), self.bits_per_sample())
        }
    }
    fn preroll_target(&self) -> Option<i64> {
        matches!(self.mp3.tag, LAYER_I | LAYER_II | LAYER_III).then(|| (self.mp3.presentation_window().start
            + self.presentation_floor.unwrap_or(0)) as i64)
    }

    fn resume_preroll(&mut self, pts: i64) -> bool {
        if pts < 0 || self.preroll_target().is_none_or(|target| pts > target) {
            return false;
        }
        let Ok(index) = self.mp3.frames.binary_search_by_key(&(pts as u64), |frame| frame.pts) else {
            return false;
        };
        self.packet = index;
        true
    }

    fn present_decoded(&self, mut packet: crate::audio::AudioPacket, source_pts: i64)
        -> Result<Option<crate::audio::AudioPacket>> {
        if self.presentation_floor.is_none() && self.mp3.gapless.is_none() { return Ok(Some(packet)); }
        let window = self.mp3.presentation_window();
        let floor = window.start + self.presentation_floor.unwrap_or(0);
        let stride = usize::from(self.channels()) * 4;
        if stride == 0 || !packet.data.len().is_multiple_of(stride) {
            return Err(invalid("invalid MP3 preroll PCM geometry"));
        }
        let frames = packet.data.len() / stride;
        let skip = (i128::from(floor) - i128::from(source_pts)).max(0)
            .min(frames as i128) as usize;
        let last = (i128::from(window.end)-i128::from(source_pts)).max(0).min(frames as i128) as usize;
        if skip >= last { return Ok(None); }
        packet.data.copy_within(skip * stride..last * stride, 0);
        packet.data.truncate((last - skip) * stride);
        packet.pts = u64::try_from(i128::from(source_pts) + skip as i128 - i128::from(window.start))
            .map_err(|_| invalid("negative MP3 presentation timestamp"))?;
        packet.timebase_num = 1;
        packet.timebase_den = self.sample_rate();
        Ok(Some(packet))
    }
    fn codec(&self) -> &str {
        self.mp3.tag
    }

    /// Packets are stamped in samples, the same timescale the decoder reports.
    fn timescale(&self) -> u32 {
        self.mp3.sample_rate
    }

    fn sample_rate(&self) -> u32 {
        self.mp3.sample_rate
    }

    fn channels(&self) -> u16 {
        self.mp3.channels
    }

    fn duration(&self) -> Option<Duration> {
        let samples = self.mp3.presentation_samples();
        let rate = u64::from(self.mp3.sample_rate);
        Some(Duration::new(samples / rate, ((samples % rate) * 1_000_000_000 / rate) as u32))
    }

    /// A frame describes itself: this container stores no setup block.
    fn extra_data(&self) -> &[u8] {
        &[]
    }

    fn audio_tracks(&self) -> Vec<AudioTrack> {
        vec![AudioTrack {
            sample_rate: self.mp3.sample_rate,
            channels: self.mp3.channels,
            name: String::new(),
            language: String::new(),
        }]
    }

    fn next_packet(&mut self) -> Result<Option<EncodedPacket>> {
        let Some(at) = self.mp3.frames.get(self.packet) else {
            return Ok(None);
        };
        let packet = EncodedPacket {
            data: self.mp3.packet(self.packet).to_vec(),
            pts: at.pts as i64,
            duration: i64::from(at.samples),
        };
        self.packet += 1;
        Ok(Some(packet))
    }

    fn rewind(&mut self) {
        self.packet = 0;
        self.presentation_floor = None;
    }

    fn seek_to(&mut self, pts: i64) -> i64 {
        let target = pts.max(0) as u64;
        if matches!(self.mp3.tag, LAYER_I | LAYER_II | LAYER_III) {
            // Reservoir bytes and synthesis overlap both precede the target.
            // Decode from the beginning, then discard exact sample preroll.
            let target = target.min(self.mp3.presentation_samples());
            self.packet = 0;
            self.presentation_floor = Some(target);
            return target as i64;
        }
        if target >= self.mp3.samples() {
            self.packet = self.mp3.frames.len();
            return self.mp3.samples() as i64;
        }
        let index = self
            .mp3
            .frames
            .partition_point(|at| at.pts <= target)
            .saturating_sub(1);
        self.packet = index;
        self.mp3.frames[index].pts as i64
    }
}

#[cfg(test)]
mod tests {
    use super::{Frame, LAYER_II, LAYER_III, Limits, Mp3, Mp3AudioReader};
    use crate::audio::{AudioStream, EncodedPacket};
    use crate::codec::make_audio_decoder;

    /// The two files under `tests/fixtures/mp3`, both written by this machine's
    /// FFmpeg:
    ///   ffmpeg -f lavfi -i sine=frequency=440:sample_rate=48000:duration=0.2 \
    ///          -c:a libmp3lame -b:a 96k -frames:a 40 tests/fixtures/mp3/tone.mp3
    ///   ffmpeg -f lavfi -i sine=frequency=440:sample_rate=48000:duration=0.2 \
    ///          -c:a mp2 -b:a 128k -frames:a 40 tests/fixtures/mp3/layer-ii.mp2
    /// The frame lists asserted below are ffprobe's, packet for packet.
    const TONE: &[u8] = include_bytes!("../../../tests/fixtures/mp3/tone.mp3");
    const LAYER_II_FILE: &[u8] = include_bytes!("../../../tests/fixtures/mp3/layer-ii.mp2");

    fn parse(bytes: &[u8]) -> Mp3 {
        Mp3::parse(bytes, &Limits::default()).expect("parses")
    }

    fn open(bytes: &[u8]) -> Mp3AudioReader {
        Mp3AudioReader::open(bytes, Limits::default()).expect("opens")
    }

    #[test]
    fn a_real_files_frames_are_where_the_reference_demuxer_puts_them() {
        let mp3 = parse(TONE);
        assert_eq!(mp3.tag, LAYER_III);
        assert_eq!((mp3.sample_rate, mp3.channels), (48_000, 1));
        // An ID3v2.4 header holding 34 bytes of frames, then the `Info` frame the
        // muxer put first, then ten frames of 288 bytes from byte 332 to the end.
        assert_eq!(
            (mp3.tag_bytes, mp3.info_frame, mp3.id3v1),
            (44, true, false)
        );
        assert_eq!(mp3.packets(), 10);
        let starts: Vec<usize> = mp3.frames.iter().map(|at| at.start).collect();
        assert_eq!(starts, (0..10).map(|at| 332 + at * 288).collect::<Vec<_>>());
        assert!(mp3.frames.iter().all(|at| at.size == 288));
        assert!(mp3.frames.iter().all(|at| at.bit_rate == 96_000));
        assert_eq!(mp3.samples(), 11_520);
        assert_eq!(
            mp3.packet(0),
            &TONE[332..620],
            "a packet is the whole frame, header included"
        );
    }

    #[test]
    fn a_layer_ii_stream_is_read_by_the_same_walk() {
        let mp3 = parse(LAYER_II_FILE);
        assert_eq!(mp3.tag, LAYER_II);
        assert_eq!((mp3.sample_rate, mp3.channels), (48_000, 1));
        // No tag at either end and no info frame: the stream opens at byte 0 with
        // nine frames of 384 bytes, each holding 1152 samples.
        assert_eq!(
            (mp3.tag_bytes, mp3.info_frame, mp3.id3v1),
            (0, false, false)
        );
        assert_eq!(mp3.packets(), 9);
        assert_eq!(
            mp3.frames[1],
            Frame {
                start: 384,
                size: 384,
                pts: 1_152,
                samples: 1_152,
                bit_rate: 128_000,
            }
        );
        assert_eq!(mp3.samples(), 10_368);
    }

    #[test]
    fn the_packets_decode_to_the_track_the_stream_runs_to() {
        for (bytes, tag, samples) in [
            (TONE, LAYER_III, 11_520usize),
            (LAYER_II_FILE, LAYER_II, 10_368),
        ] {
            let mut reader = open(bytes);
            assert_eq!(reader.codec(), tag);
            assert_eq!((reader.sample_rate(), reader.channels()), (48_000, 1));
            assert_eq!(reader.timescale(), 48_000);
            assert_eq!(
                reader.bits_per_sample(),
                0,
                "a compressed stream says no width"
            );
            assert!(reader.extra_data().is_empty());
            assert_eq!(reader.audio_tracks()[0].label(), "1 ch 48000 Hz");
            let mut decoder = make_audio_decoder(tag, &[], 48_000, 1, 0).expect(tag);
            let mut heard = 0usize;
            let mut stamps = Vec::new();
            while let Some(EncodedPacket {
                data,
                pts,
                duration,
            }) = reader.next_packet().expect("packet")
            {
                stamps.push((pts, duration));
                let audio = decoder
                    .decode_encoded(&data, pts as u64, duration as u64)
                    .expect(tag)
                    .expect("every frame of a real file decodes");
                heard += audio.data.len() / 4;
            }
            assert_eq!(stamps.len(), reader.mp3().packets());
            assert_eq!(stamps[1], (1_152, 1_152), "pts is samples, not bytes");
            assert_eq!(heard, samples, "{tag} decoded {heard} of {samples}");
            reader.rewind();
            assert_eq!(reader.next_packet().expect("packet").expect("first").pts, 0);
        }
    }

    #[test]
    fn layer3_seek_rebuilds_history_and_preserves_exact_sample_target() {
        let mut reader = open(TONE);
        assert_eq!(reader.seek_to(2_304), 2_304);
        assert_eq!(reader.seek_to(2_400), 2_400);
        assert_eq!(reader.preroll_target(), Some(3_505));
        assert_eq!(reader.next_packet().unwrap().unwrap().pts, 0);
        assert_eq!(reader.seek_to(-1), 0);
        assert_eq!(reader.seek_to(1 << 30), 9_600);
        assert_eq!(reader.preroll_target(), Some(10_705));
        assert_eq!(reader.next_packet().unwrap().unwrap().pts, 0);
        reader.rewind();
        assert_eq!(reader.preroll_target(), Some(1_105));
    }

    #[test]
    fn a_tag_at_the_end_is_not_audio() {
        // The real file with an ID3v1 record bolted on: 128 bytes whose first three
        // spell `TAG`, which the walk stops before rather than sizing a frame from.
        let mut bytes = TONE.to_vec();
        let mut tail = vec![b'T', b'A', b'G'];
        tail.resize(128, 0x5a);
        bytes.extend_from_slice(&tail);
        let mp3 = parse(&bytes);
        assert!(mp3.id3v1);
        assert_eq!(mp3.packets(), 10);
        assert_eq!(mp3.frames.last().expect("frame").start + 288, TONE.len());
        // The same bytes with the tail present but no `TAG` spelling its name: the
        // audio run still holds, and the tail is neither a frame nor a tag.
        bytes[TONE.len()..TONE.len() + 3].copy_from_slice(b"zzz");
        let mp3 = parse(&bytes);
        assert!(!mp3.id3v1);
        assert_eq!(mp3.packets(), 10);
    }

    #[test]
    fn a_run_that_does_not_open_on_a_header_resyncs_to_the_next_one() {
        // Everything from the third frame on, behind 200 bytes of a chopped head:
        // the scan walks forward a byte at a time until a header checks out, and
        // what it finds plays from there.
        let mut bytes = vec![0xa5; 200];
        bytes.extend_from_slice(&TONE[908..]);
        let mp3 = parse(&bytes);
        assert_eq!(mp3.packets(), 8);
        assert_eq!(mp3.frames[0].start, 200);
        assert_eq!(mp3.frames[1].start, 488);
        assert_eq!(mp3.samples(), 8 * 1_152);
    }

    #[test]
    fn a_run_that_stops_short_of_the_end_of_the_file_is_not_a_stream() {
        // The dispatcher tries this reader last, over bytes no other container
        // claimed, so four bytes that happen to read as a header must not be enough
        // to open a file. Twice the tone with a second copy of itself after 2 kB of
        // nothing: the run breaks at the gap and covers less of the file than a
        // stream would, so the file is refused.
        let mut bytes = TONE.to_vec();
        bytes.extend_from_slice(&[0x11; 2048]);
        bytes.extend_from_slice(TONE);
        let error = Mp3::parse(&bytes, &Limits::default())
            .err()
            .expect("a run with a hole in it");
        assert!(error.to_string().contains("the frames run to"), "{error}");
        // And a file of another container entirely: a MOV whose PCM this player
        // reads elsewhere, which is not to be mistaken for MPEG audio because some
        // four bytes of it lined up.
        const MOV: &[u8] = include_bytes!("../../../tests/fixtures/audio/pcm-screen.mov");
        assert!(Mp3::parse(MOV, &Limits::default()).is_err());
    }

    #[test]
    fn a_layer_one_header_without_a_complete_frame_is_not_audio() {
        let error = Mp3::parse(&0xffff_74c4u32.to_be_bytes(), &Limits::default()).err().unwrap();
        assert!(error.to_string().contains("no MPEG audio frames"), "{error}");
    }

    #[test]
    fn a_file_that_holds_no_frame_header_is_refused() {
        for bytes in [
            &b"RIFF....WAVEfmt "[..],
            &[][..],
            // Sync, then a bitrate index of zero, which names no bitrate.
            &[0xff, 0xfb, 0x00, 0x00][..],
            // The one version value the format leaves unused.
            &[0xff, 0xef, 0x94, 0xc4][..],
        ] {
            assert!(Mp3::parse(bytes, &Limits::default()).is_err(), "{bytes:?}");
        }
    }

    #[test]
    fn a_stream_that_changes_its_geometry_midway_is_refused() {
        // Two frames of the layer II fixture, the second rewritten to name layer
        // III at the same rate - a legal thing for a file to do, and one the player
        // cannot honour, since it builds one decoder from one geometry.
        let mut bytes = LAYER_II_FILE[..768].to_vec();
        bytes[385] = 0xfb;
        let error = Mp3::parse(&bytes, &Limits::default())
            .err()
            .expect("a midstream layer change");
        assert!(error.to_string().contains("changes geometry"), "{error}");
    }

    #[test]
    fn an_id3v2_length_is_read_as_seven_bits_per_byte() {
        // The synchsafe rule is what makes the front of a file safe to skip, so one
        // header's own arithmetic is checked against the distance it stands for.
        let mut bytes = vec![b'I', b'D', b'3', 4, 0, 0, 0x00, 0x00, 0x00, 0x22];
        bytes.resize(44, 0);
        bytes.extend_from_slice(&TONE[44..]);
        assert_eq!(parse(&bytes).tag_bytes, 44);
        // A length byte with its high bit set is not a synchsafe number, whatever
        // the magic says, so the tag is left where it is and the frame scan walks
        // over it - which lands on the same audio, one byte earlier than the tag
        // claimed.
        bytes[6] = 0x80;
        let mp3 = parse(&bytes);
        assert_eq!(
            (mp3.tag_bytes, mp3.info_frame, mp3.packets()),
            (0, true, 10)
        );
        assert_eq!(mp3.frames[0].start, 332);
    }

    #[test]
    fn a_second_tag_follows_the_first() {
        // Two tags stacked in front of the audio: both are past, and the frames
        // still open where the second one ends.
        let mut bytes = TONE[..44].to_vec();
        bytes.extend_from_slice(&TONE[..44]);
        bytes.extend_from_slice(&TONE[44..]);
        let mp3 = parse(&bytes);
        assert_eq!((mp3.tag_bytes, mp3.packets()), (88, 10));
        assert_eq!(mp3.frames[0].start, 376);
    }
}
