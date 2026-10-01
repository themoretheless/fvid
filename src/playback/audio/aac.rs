//! Audio playback adapter for the owned ADTS container reader.
use crate::audio::{AudioStream, AudioTrack, EncodedPacket};
use crate::Result;
use std::io::Read;
use std::time::Duration;
pub use crate::container::adts::{Aac, Frame, Header, Limits, TAG, esds_for, header};

/// A `.aac` file read as an audio track.
pub struct AacAudioReader {
    aac: Aac,
    extra_data: Vec<u8>,
    packet: usize,
    presentation_floor: u64,
}

impl AacAudioReader {
    /// Read a whole file. The frames lie at lengths only their own headers
    /// state, so all of them are walked before the first one plays.
    pub fn open<R: Read>(mut reader: R, limits: Limits) -> Result<Self> {
        let mut bytes = Vec::new();
        let cap = u64::try_from(limits.file_bytes).unwrap_or(u64::MAX);
        reader.by_ref().take(cap + 1).read_to_end(&mut bytes)?;
        let aac = Aac::parse(&bytes, &limits)?;
        let extra_data = aac.extra_data();
        Ok(Self {
            aac,
            extra_data,
            packet: 0,
            presentation_floor: 0,
        })
    }

    /// The file's own shape, for a caller that wants to say what it opened.
    pub fn aac(&self) -> &Aac {
        &self.aac
    }
}

impl AudioStream for AacAudioReader {
    fn codec(&self) -> &str {
        TAG
    }

    /// Packets are stamped in samples, the same timescale the decoder reports.
    fn timescale(&self) -> u32 {
        self.aac.sample_rate
    }

    fn sample_rate(&self) -> u32 {
        self.aac.sample_rate
    }

    fn channels(&self) -> u16 {
        self.aac.channels
    }

    fn duration(&self) -> Option<Duration> {
        Some(Duration::from_secs_f64(
            self.aac.samples() as f64 / f64::from(self.aac.sample_rate),
        ))
    }

    fn extra_data(&self) -> &[u8] {
        &self.extra_data
    }

    fn audio_tracks(&self) -> Vec<AudioTrack> {
        vec![AudioTrack {
            sample_rate: self.aac.sample_rate,
            channels: self.aac.channels,
            name: String::new(),
            language: String::new(),
        }]
    }

    fn next_packet(&mut self) -> Result<Option<EncodedPacket>> {
        let Some(at) = self.aac.frames.get(self.packet) else {
            return Ok(None);
        };
        let packet = EncodedPacket {
            data: self.aac.packet(self.packet).to_vec(),
            pts: at.pts as i64,
            duration: i64::from(self.aac.samples_per_frame),
        };
        self.packet += 1;
        Ok(Some(packet))
    }

    fn rewind(&mut self) {
        self.packet = 0;
        self.presentation_floor = 0;
    }

    fn present_decoded(&self, packet: crate::audio::AudioPacket, source_pts: i64) -> Result<Option<crate::audio::AudioPacket>> {
        if source_pts < 0 || (source_pts as u64) < self.presentation_floor { Ok(None) } else { Ok(Some(packet)) }
    }

    fn seek_to(&mut self, pts: i64) -> i64 {
        let target = pts.max(0) as u64;
        let index = self
            .aac
            .frames
            .partition_point(|at| at.pts <= target)
            .saturating_sub(1);
        self.presentation_floor = self.aac.frames[index].pts;
        self.packet = 0;
        self.presentation_floor as i64
    }
}

#[cfg(test)]
mod tests {
    use super::{Aac, AacAudioReader, Frame, Header, Limits, TAG, esds_for, header};
    use crate::audio::{AudioStream, EncodedPacket};
    use crate::codec::{config::aac_specific_config, make_audio_decoder};

    /// Three files under `tests/fixtures/audio`, each 48 kHz or 44.1 kHz AAC-LC
    /// written by this machine's FFmpeg and reproduced by it byte for byte, so
    /// the frame lengths the parser walks really are the ones the encoder wrote.
    /// Every command was run twice into two paths and the pair compared equal:
    ///
    /// ```text
    /// ffmpeg -f lavfi -i "sine=frequency=440:sample_rate=48000:duration=0.256" \
    ///        -c:a aac -b:a 128k -ac 2 tests/fixtures/audio/aac-stereo.aac
    /// ffmpeg -f lavfi -i "sine=frequency=440:sample_rate=44100:duration=0.128" \
    ///        -c:a aac -b:a 64k -ac 1 tests/fixtures/audio/aac-mono-44k.aac
    /// ffmpeg -f lavfi -i "sine=frequency=440:sample_rate=48000:duration=0.128" \
    ///        -c:a aac -b:a 192k -ac 6 -channel_layout 5.1 \
    ///        tests/fixtures/audio/aac-51.aac
    /// ```
    ///
    /// The stereo file's 4399 bytes are thirteen frames - 293, 370, 313, 314,
    /// 326, 319, 362, 358, 339, 356, 360, 324 and 365 - the mono one 1569 bytes
    /// in seven and the surround one 2377 in seven, and each file ends where its
    /// last frame does. The first frame of each is the encoder's tag: its audio is
    /// a fill element carrying `Lavc63.1.102` rather than samples, and FFmpeg's
    /// own demuxer lists it as a packet, so this one does too.
    const STEREO: &[u8] = include_bytes!("../../../tests/fixtures/audio/aac-stereo.aac");
    const MONO_44K: &[u8] = include_bytes!("../../../tests/fixtures/audio/aac-mono-44k.aac");
    const SURROUND: &[u8] = include_bytes!("../../../tests/fixtures/audio/aac-51.aac");

    fn parse(bytes: &[u8]) -> Aac {
        Aac::parse(bytes, &Limits::default()).expect("parses")
    }

    /// The header states its own length, so the walk is a stride and the
    /// whole-file assertion is about coverage: the frames must end where the
    /// file does, at exactly the lengths ffprobe counts.
    #[test]
    fn a_real_file_is_exactly_the_frames_its_headers_count() {
        let aac = parse(STEREO);
        assert_eq!((aac.sample_rate, aac.channels), (48_000, 2));
        assert_eq!(aac.samples_per_frame, 1024, "AAC-LC at a table rate");
        assert_eq!(aac.packets(), 13);
        assert_eq!(
            aac.frames.iter().map(|at| at.size).collect::<Vec<_>>(),
            [
                293, 370, 313, 314, 326, 319, 362, 358, 339, 356, 360, 324, 365
            ]
        );
        assert_eq!(
            aac.frames.last().map(|at| at.start + at.size),
            Some(STEREO.len())
        );
        assert!(
            aac.frames
                .iter()
                .all(|at| at.header_bytes == 7 && at.asc == aac.frames[0].asc),
            "one coding and one header width for the whole file"
        );
        // The other two files hold the same walk at the other rate index the
        // header reaches and at the widest layout a three-bit field names, so
        // neither the frequency nor the channel count can be a constant.
        for (bytes, rate, channels, count, asc) in [
            (STEREO, 48_000, 2, 13, [0x11, 0x90]),
            (MONO_44K, 44_100, 1, 7, [0x12, 0x08]),
            (SURROUND, 48_000, 6, 7, [0x11, 0xb0]),
        ] {
            let aac = parse(bytes);
            assert_eq!((aac.sample_rate, aac.channels), (rate, channels));
            assert_eq!(aac.packets(), count, "{rate} Hz {channels} ch");
            assert_eq!(aac.samples_per_frame, 1024);
            assert_eq!(aac.frames[0].asc, asc, "the config the header states");
            assert_eq!(
                aac.frames.last().map(|at| at.start + at.size),
                Some(bytes.len()),
                "the stream ends where the file does"
            );
        }
    }

    #[test]
    fn timestamps_count_the_samples_a_frame_holds() {
        let aac = parse(STEREO);
        for (index, at) in aac.frames.iter().enumerate() {
            assert_eq!(at.pts, index as u64 * 1024);
        }
        assert_eq!(aac.samples(), 13 * 1024);
        let reader = AacAudioReader::open(STEREO, Limits::default()).expect("opens");
        assert_eq!(reader.timescale(), 48_000);
        let duration = reader.duration().expect("a counted stream");
        assert_eq!(
            duration.as_micros(),
            277_333,
            "thirteen frames of 1024 samples at 48000 Hz, the same count ffprobe's duration reports"
        );
        assert_eq!(duration.as_millis(), 277);
    }

    /// The synthesized setup block has to be the one the decoder reads, which is
    /// a claim about the repository's own `esds` parser rather than about bytes
    /// nobody checked.
    #[test]
    fn the_setup_block_carries_the_config_the_header_states() {
        let aac = parse(STEREO);
        let extra = aac.extra_data();
        assert_eq!(extra.len(), 28);
        assert_eq!(
            aac_specific_config(&extra).expect("a record the parser accepts"),
            &aac.frames[0].asc,
            "the AudioSpecificConfig the frames spell"
        );
        assert_eq!(aac.frames[0].asc, [0x11, 0x90], "LC, 48000 Hz, 2 ch");
    }

    /// Matroska hands the same config over at its full written length, not the
    /// two bytes an ADTS header carries, so the wrapper has to grow with it:
    /// every descriptor states the length of what follows it.
    #[test]
    fn the_wrapper_grows_with_a_config_a_container_handed_over() {
        // The five bytes `tests/fixtures/audio/aac-stereo.mka` carries: the two
        // an ADTS header would state, plus the writer's own tail.
        let private = [0x11, 0x90, 0x56, 0xe5, 0x00];
        let esds = esds_for(&private).expect("a config of the written length");
        assert_eq!(esds.len(), 26 + private.len());
        assert_eq!(
            aac_specific_config(&esds).expect("a record the parser accepts"),
            private,
            "the whole config, not only its first two bytes"
        );
        // The two-byte ADTS case keeps its old shape, because the fixture tests
        // and the K-Lite gate read its length.
        assert_eq!(esds_for(&[0x11, 0x90]).expect("two bytes").len(), 28);
        // A block too short to name a coding is refused rather than wrapped.
        assert!(esds_for(&[0x11]).is_none());
        assert!(esds_for(&[]).is_none());
    }

    /// The reader is only useful if the decoder accepts what it hands over:
    /// stripped blocks, stamped in samples, under the tag the dispatch answers.
    #[test]
    fn the_packets_it_hands_over_decode_as_the_tag_says() {
        let mut reader = AacAudioReader::open(STEREO, Limits::default()).expect("opens");
        assert_eq!(reader.codec(), TAG);
        assert_eq!((reader.sample_rate(), reader.channels()), (48_000, 2));
        assert_eq!(
            reader.bits_per_sample(),
            0,
            "a compressed stream says no width"
        );
        assert_eq!(reader.audio_tracks()[0].label(), "2 ch 48000 Hz");
        let mut decoder = make_audio_decoder(
            reader.codec(),
            reader.extra_data(),
            reader.sample_rate(),
            reader.channels(),
            reader.bits_per_sample(),
        )
        .expect("the dispatch answers this tag");
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
                .expect("a frame of a real file decodes")
                .expect("and sounds");
            assert_eq!(
                audio.data.len() / 4 / 2,
                1024,
                "one raw block of interleaved f32 stereo"
            );
            assert_eq!((audio.pts, audio.timebase_den), (pts as u64, 48_000));
            heard += audio.data.len() / 4;
        }
        assert_eq!(stamps.len(), reader.aac().packets());
        assert_eq!(stamps[1], (1024, 1024), "pts is samples, not bytes");
        assert_eq!(heard, 13 * 1024 * 2, "every frame sounded its samples");
        reader.rewind();
        assert_eq!(reader.next_packet().expect("packet").expect("first").pts, 0);
    }

    /// Decode every packet the reader hands over and report the samples heard,
    /// with the width the decoder answered each frame at.
    fn decode_all(bytes: &[u8]) -> (u16, usize) {
        let mut reader = AacAudioReader::open(bytes, Limits::default()).expect("opens");
        let mut decoder = make_audio_decoder(
            reader.codec(),
            reader.extra_data(),
            reader.sample_rate(),
            reader.channels(),
            reader.bits_per_sample(),
        )
        .expect("the dispatch answers this tag");
        let rate = reader.sample_rate();
        let mut width = 0;
        let mut heard = 0usize;
        while let Some(EncodedPacket {
            data,
            pts,
            duration,
        }) = reader.next_packet().expect("packet")
        {
            let audio = decoder
                .decode_encoded(&data, pts as u64, duration as u64)
                .expect("a frame of a real file decodes")
                .expect("and sounds");
            width = audio.data.len() / 4 / 1024;
            assert_eq!((audio.pts, audio.timebase_den), (pts as u64, rate));
            heard += audio.data.len() / 4;
        }
        (width as u16, heard)
    }

    /// The block the reader writes is the one the decoder builds itself from, so
    /// the second geometry the stereo file cannot speak for is decoded too: mono
    /// at the other rate index the header reaches.
    #[test]
    fn a_mono_file_decodes_from_the_setup_block_its_frames_state() {
        let reader = AacAudioReader::open(MONO_44K, Limits::default()).expect("opens");
        assert_eq!(
            reader.audio_tracks()[0].label(),
            "1 ch 44100 Hz",
            "the layout is the header's own"
        );
        let (width, heard) = decode_all(MONO_44K);
        assert_eq!(width, 1);
        assert_eq!(heard, 7 * 1024);
    }

    /// The owned AAC decoder accepts the six-channel setup emitted by ADTS.
    #[test]
    fn a_five_one_file_states_the_setup_block_its_frames_describe() {
        let reader = AacAudioReader::open(SURROUND, Limits::default()).expect("opens");
        assert_eq!(reader.audio_tracks()[0].label(), "6 ch 48000 Hz");
        assert_eq!(
            aac_specific_config(reader.extra_data()).expect("a record the parser accepts"),
            &[0x11, 0xb0]
        );
        let frames = reader.aac().packets();
        let (width, heard) = decode_all(SURROUND);
        assert_eq!(width, 6);
        assert_eq!(heard, frames * 1024 * 6);
    }

    #[test]
    fn a_seek_lands_on_the_frame_at_or_before_the_sample_asked_for() {
        let mut reader = AacAudioReader::open(STEREO, Limits::default()).expect("opens");
        assert_eq!(reader.seek_to(3000), 2048, "the third frame starts at 2048");
        assert_eq!(reader.seek_to(2048), 2048, "a frame start stays there");
        assert_eq!(reader.seek_to(0), 0);
        assert_eq!(
            reader.seek_to(1 << 30),
            12 * 1024,
            "past the end is the last"
        );
        let packet = reader.next_packet().expect("packet").expect("frame");
        assert_eq!(packet.pts, 0, "seek consumes decoder preroll first");
        assert_eq!(packet.data, reader.aac.packet(0));
        assert_eq!(packet.duration, 1024);
        reader.rewind();
        assert_eq!(reader.seek_to(-5), 0, "a negative sample is the start");
    }

    /// A file of another shape must be refused rather than half-read: the player
    /// offers every reader in turn, so a false accept costs the file its real
    /// codec.
    #[test]
    fn a_file_that_is_not_a_run_of_adts_frames_is_refused() {
        let mp3 = include_bytes!("../../../tests/fixtures/mp3/tone.mp3");
        let error = Aac::parse(mp3, &Limits::default()).expect_err("an MP3 is no ADTS run");
        assert!(error.to_string().contains("no ADTS frames"), "{error}");
        // A truncated tail is the file's own damage, not another container's, so
        // it keeps the frames that fit - as long as they still fill the file the
        // way a real stream does, which is why the cut sits near the end.
        let mut chopped = STEREO[..3710 + 100].to_vec();
        assert_eq!(parse(&chopped).packets(), 11, "only the frames that fit");
        // A short run reaching into a file of another shape is the case the
        // coverage guard is for.
        chopped.extend(vec![0u8; 4000]);
        let error = Aac::parse(&chopped, &Limits::default()).expect_err("padding is no audio");
        assert!(error.to_string().contains("run to byte"), "{error}");
    }

    /// The header probe is where every structural refusal is made, so each one
    /// is named against the bytes that trigger it. The audio cases below are
    /// built from a real frame rather than invented, so only the one field the
    /// case is about changes.
    #[test]
    fn a_header_that_states_nothing_playable_is_refused() {
        let at = header(&STEREO[293..]).expect("a real frame");
        assert_eq!(
            at,
            Header {
                sample_rate: 48_000,
                channels: 2,
                header_bytes: 7,
                frame_bytes: 370,
                asc: [0x11, 0x90],
            }
        );
        assert_eq!(header(&STEREO[293..299]), None, "six bytes name no header");
        assert_eq!(
            header(&[0xff, 0xf1, 0, 0, 0, 0, 0]),
            None,
            "a stub is no frame"
        );
        // A frame whose stated length stops inside its own header holds nothing
        // the walk could pass on.
        assert_eq!(
            header(&[0xff, 0xf1, 0x4c, 0x80, 0, 0, 0]),
            None,
            "no payload"
        );
        // Channel config zero means the layout sits in a program config element
        // inside the audio, which no header here states.
        let mut pce = STEREO[293..293 + 14].to_vec();
        pce[2] &= !1;
        pce[3] &= 0x3F;
        assert_eq!(header(&pce).expect("PCE configuration is admitted").channels, 0, "layout is deferred to the PCE in the access unit");
        // Rate indexes 13, 14 and 15 are reserved in this header, unlike the
        // AudioSpecificConfig, which spells a 24-bit rate for 15.
        let mut reserved = STEREO[293..293 + 14].to_vec();
        reserved[2] = (reserved[2] & 0b1100_0011) | (15 << 2);
        assert_eq!(header(&reserved), None, "reserved frequency");
        // Two raw blocks in one frame are two sets of samples under one stamp.
        let mut packed = STEREO[293..293 + 14].to_vec();
        packed[6] |= 1;
        assert_eq!(header(&packed), None, "several blocks in one frame");
    }

    /// With the protection bit clear a frame carries two CRC bytes after its
    /// header, so the header is nine bytes long and the stated length still ends
    /// the frame. This machine's FFmpeg writes no CRC, so the width is read from
    /// the specification and checked here on the walk alone: the two bytes the
    /// frame counts as CRC are stripped with the header, and the audio that
    /// follows is left to a decoder no fixture exercises.
    #[test]
    fn a_frame_with_crc_states_a_nine_byte_header() {
        let unprotected = header(&STEREO[293..]).expect("a real frame");
        assert_eq!(unprotected.header_bytes, 7);
        let mut crc = STEREO[293..293 + 370].to_vec();
        crc[1] = 0xF0;
        let protected = header(&crc).expect("the same frame, protected");
        assert_eq!(protected.header_bytes, 9);
        assert_eq!(protected.frame_bytes, unprotected.frame_bytes);
        assert_eq!(
            protected.asc, unprotected.asc,
            "the CRC is no part of the coding"
        );
        let aac = Aac::parse(&crc, &Limits::default()).expect("a protected stream walks");
        assert_eq!(
            aac.frames[0],
            Frame {
                start: 0,
                size: 370,
                header_bytes: 9,
                asc: [0x11, 0x90],
                pts: 0,
            }
        );
        assert_eq!(aac.packet(0).len(), 370 - 9);
    }
}
