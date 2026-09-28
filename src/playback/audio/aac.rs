//! Audio playback of AAC out of the bare ADTS file the coding ships in.
//!
//! A `.aac` file states nothing before its frames: each ADTS frame carries its
//! own header, and what the player asks a container for - a codec tag, a rate, a
//! channel count, setup data, packets with timestamps - is read out of the
//! frames themselves, as [`crate::playback_mp3`] and [`crate::playback_ac3`] do
//! for their codings. The packets handed to [`crate::codec::make_audio_decoder`]
//! are the raw blocks with their headers stripped, which is the shape the AAC
//! decoder is written for: it is the MP4 decoder, reached through the fourcc
//! `mp4a`, and it asks for an `esds` configuration record that this container
//! does not hold.
//!
//! That missing record is the one piece of work here that is not just a walk.
//! The decoder reads its `extra_data` through
//! [`crate::codec::config::aac_specific_config`], which wants a whole `esds`
//! payload - the four version/flags bytes, then an ES descriptor holding a
//! DecoderConfigDescriptor holding an AudioSpecificConfig - and rejects a bare
//! two-byte AudioSpecificConfig. So the two bytes are built from the frame
//! header, which states the coding, the rate index and the channel layout in the
//! same three bits and four that the AudioSpecificConfig does, and then wrapped
//! in the descriptors the decoder expects. The result is checked back through
//! [`crate::codec::config::AacConfig::parse`], so a bare file is refused for
//! exactly the reasons a container track is rather than by a second opinion.
//!
//! Two measured facts shape the walk. A frame's length is stored in it as a
//! thirteen-bit count of bytes including its own header, so unlike AC-3 no table
//! row has to be looked up - but the header is seven bytes or nine depending on
//! the protection bit, and a length below that names no audio at all. And every
//! frame of an AudioSpecificConfig-described stream holds one raw block of 1024
//! samples, which is what makes the timestamps a plain running count; a header
//! that packs several blocks into one frame is refused rather than mis-stamped.
//!
//! What the header cannot say is a high-rate extension: ADTS keeps the coding in
//! two profile bits that every encoder writes as one for both AAC-LC and HE-AAC,
//! so a file carrying SBR in its audio arrives here as the core geometry its
//! frames state, and nothing names it by that coding. This machine's FFmpeg
//! refuses to write the profile at all - `-profile:a aac_he` fails the encoder
//! before it opens a file - so there is no fixture to check the claim against,
//! and none is invented.

use crate::audio::{AudioStream, AudioTrack, EncodedPacket};
use crate::codec::config::AacConfig;
use crate::{Result, invalid};
use std::io::Read;
use std::time::Duration;

/// What a limit guards: how much the reader may take in, and how many frames it
/// may agree to list.
#[derive(Clone, Copy, Debug)]
pub struct Limits {
    /// Largest file accepted.
    pub file_bytes: usize,
    /// Most frames one stream may hold.
    pub packets: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            file_bytes: 128 << 20,
            packets: 1 << 20,
        }
    }
}

/// The tag [`crate::codec::make_audio_decoder`] answers for AAC. It is the MP4
/// fourcc rather than the Matroska `A_AAC` because the dispatch has no arm for
/// that one, and because the coding's setup block is an MP4 `esds`, which is
/// what this reader hands over with it.
pub const TAG: &str = "mp4a";

/// The rates an ADTS sampling frequency index names, in table order. Index 15 is
/// reserved here, where the AudioSpecificConfig instead spells a rate in 24
/// bits, so a header carrying 13, 14 or 15 states no rate this reader can trust.
const FREQUENCIES: [u32; 13] = [
    96000, 88200, 64000, 48000, 44100, 32000, 24000, 22050, 16000, 12000, 11025, 8000, 7350,
];

/// What an ADTS header states about itself: where its audio starts, how long the
/// frame is, and the coding and geometry to read out of it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Header {
    pub sample_rate: u32,
    pub channels: u16,
    /// Bytes of header, before the raw block: seven with no CRC, nine with one.
    pub header_bytes: usize,
    /// Total frame length, header and CRC included, as the frame states it.
    pub frame_bytes: usize,
    /// The two-byte AudioSpecificConfig this frame's coding, rate and layout
    /// spell, which is what the decoder's setup block carries.
    pub asc: [u8; 2],
}

/// Read just the fixed header of the ADTS frame that starts at `bytes`.
///
/// `None` for a run of bytes no ADTS frame can start with: a missing syncword or
/// layer, a reserved rate index, a channel config of zero (which means the
/// layout is carried in a program config element inside the audio, where this
/// reader does not look), a frame length that does not reach past the header it
/// is stored in, or a frame holding several raw blocks.
pub fn header(bytes: &[u8]) -> Option<Header> {
    let b = bytes.get(..7)?;
    // Twelve bits of syncword, then the one-bit version flag and the two-bit
    // layer, which is zero for AAC: masking the version out is what lets both
    // MPEG-2 and MPEG-4 ADTS through while still rejecting a stray 0xFFF of some
    // other format's bytes.
    if b[0] != 0xFF || b[1] & 0xF6 != 0xF0 {
        return None;
    }
    let object_type = ((b[2] >> 6) & 3) + 1;
    let frequency = (b[2] >> 2) & 0xF;
    let channels = (u16::from(b[2] & 1) << 2) | u16::from(b[3] >> 6);
    let frame_bytes =
        (usize::from(b[3] & 3) << 11) | (usize::from(b[4]) << 3) | (usize::from(b[5]) >> 5);
    let header_bytes = if b[1] & 1 == 1 { 7 } else { 9 };
    if frame_bytes < header_bytes {
        return None;
    }
    // Two bits at the end of the fixed header count the raw blocks after the
    // first; a frame that holds two holds two sets of samples, and the running
    // count of 1024 per frame would fall behind them.
    if b[6] & 3 != 0 {
        return None;
    }
    let &sample_rate = FREQUENCIES.get(usize::from(frequency))?;
    if channels == 0 {
        return None;
    }
    // The AudioSpecificConfig spells the coding in five bits rather than two, so
    // the low-bit form has to be written out for the common objects and the
    // frame's own rate index and layout follow it in the same widths.
    let asc = [
        (object_type << 3) | (frequency >> 1),
        ((frequency & 1) << 7) | ((channels as u8) << 3),
    ];
    Some(Header {
        sample_rate,
        channels,
        header_bytes,
        frame_bytes,
        asc,
    })
}

/// One frame of the stream: where it sits in the file, how long it is, and the
/// sample it starts on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Frame {
    pub start: usize,
    pub size: usize,
    /// Bytes of ADTS header on this frame, which the packet leaves off.
    pub header_bytes: usize,
    /// The coding and geometry this frame's header states, kept so a test can
    /// name what was read rather than a copy of the numbers the file says, and
    /// so the run can be checked frame against frame.
    pub asc: [u8; 2],
    pub pts: u64,
}

/// A `.aac` file: the geometry its frames state, the setup block synthesized
/// from them, and the frames that play.
#[derive(Clone, Debug)]
pub struct Aac {
    pub sample_rate: u32,
    pub channels: u16,
    /// Samples one frame holds, read out of the setup block rather than assumed.
    pub samples_per_frame: u32,
    pub frames: Vec<Frame>,
    data: Vec<u8>,
}

impl Aac {
    /// Read a whole file: the run of frames, from wherever the first header
    /// survives to wherever the last one ends.
    pub fn parse(bytes: &[u8], limits: &Limits) -> Result<Self> {
        if bytes.len() > limits.file_bytes {
            return Err(invalid(&format!(
                "file is over the {} byte limit",
                limits.file_bytes
            )));
        }
        // As in the other elementary readers, the first header may sit behind
        // whatever chopped file got in front of it, and after it the run must be
        // contiguous: each frame starts where the last one's stated length ended.
        let mut found: Vec<Header> = Vec::new();
        let mut starts: Vec<usize> = Vec::new();
        let mut pos = 0usize;
        while pos + 7 <= bytes.len() && header(&bytes[pos..]).is_none() {
            pos += 1;
        }
        let first_start = pos;
        while let Some(at) = header(bytes.get(pos..).unwrap_or_default()) {
            if pos + at.frame_bytes > bytes.len() {
                // A frame reaching past what the file holds is a truncated tail.
                break;
            }
            if found.len() >= limits.packets {
                return Err(invalid(&format!(
                    "stream is over the {} packet limit",
                    limits.packets
                )));
            }
            found.push(at);
            starts.push(pos);
            pos += at.frame_bytes;
        }
        if found.is_empty() {
            return Err(invalid("no ADTS frames in the file"));
        }
        // A run that stops well short of the end of the file was never the file:
        // a stream of frames fills it, and anything else is another container
        // whose bytes this reader happened to make a syncword out of.
        if (pos - first_start) * 10 < (bytes.len() - first_start) * 9 {
            return Err(invalid(&format!(
                "the frames run to byte {pos} of a file {} bytes long",
                bytes.len()
            )));
        }
        let first = found[0];
        // The setup block is one record for the stream, so the coding the first
        // frame states is checked against the repository's own AAC config
        // parser: it is what names AAC-LC as the only object this build decodes.
        let config = AacConfig::parse(&first.asc)?;
        if config.sample_rate != first.sample_rate || u16::from(config.channels) != first.channels {
            return Err(invalid(
                "the frame header and the AudioSpecificConfig disagree",
            ));
        }
        let mut frames = Vec::with_capacity(found.len());
        let mut pts = 0u64;
        for (at, start) in found.iter().zip(&starts) {
            if at.sample_rate != first.sample_rate
                || at.channels != first.channels
                || at.header_bytes != first.header_bytes
                || at.asc != first.asc
            {
                return Err(invalid(&format!(
                    "stream changes geometry at frame {}: {} Hz {} ch after {} Hz {} ch",
                    frames.len(),
                    at.sample_rate,
                    at.channels,
                    first.sample_rate,
                    first.channels
                )));
            }
            frames.push(Frame {
                start: *start,
                size: at.frame_bytes,
                header_bytes: at.header_bytes,
                asc: at.asc,
                pts,
            });
            pts += u64::from(config.frame_samples);
        }
        Ok(Self {
            sample_rate: first.sample_rate,
            channels: first.channels,
            samples_per_frame: u32::from(config.frame_samples),
            frames,
            data: bytes.to_vec(),
        })
    }

    /// Frames the stream hands over as packets.
    pub fn packets(&self) -> usize {
        self.frames.len()
    }

    /// One frame's audio: its bytes from past the header to its end, which is
    /// the raw block the decoder reads.
    pub fn packet(&self, index: usize) -> &[u8] {
        let at = self.frames[index.min(self.frames.len() - 1)];
        &self.data[at.start + at.header_bytes..at.start + at.size]
    }

    /// Samples the stream runs to, which is its length for a file that states no
    /// duration of its own.
    pub fn samples(&self) -> u64 {
        self.frames
            .last()
            .map(|at| at.pts + u64::from(self.samples_per_frame))
            .unwrap_or(0)
    }

    /// The `esds` payload the AAC decoder asks its setup block for: the frame's
    /// two-byte AudioSpecificConfig inside the descriptors the MP4 sample entry
    /// would have carried.
    pub fn extra_data(&self) -> Vec<u8> {
        esds_for(&self.frames[0].asc).expect("an ADTS frame header is always two bytes")
    }
}

/// Wrap an AudioSpecificConfig in the `esds` descriptors the MP4 sample entry
/// would have carried around it. Two containers hand that config over and
/// neither gives the wrapper: a bare `.aac` file states it in every frame
/// header, and Matroska keeps it in `CodecPrivate` under the tag `A_AAC`.
///
/// The lengths below are the byte counts of the records that follow them, and
/// the bitrates are left at zero, which is what a variable-rate stream states
/// and what the decoder ignores.
pub fn esds_for(asc: &[u8]) -> Option<Vec<u8>> {
    let width = asc.len();
    // Two bytes is the shortest config that names an object type, a rate and a
    // channel layout. Past 107 the outermost descriptor's own length no longer
    // fits the single byte every writer here uses for it, and no real
    // AudioSpecificConfig comes close: the fields 14496-3 defines sum to 17.
    if width < 2 || width > 107 {
        return None;
    }
    let mut out = vec![
        0, 0, 0, 0, // version and flags of the box itself
        3, (20 + width) as u8, // ES descriptor: the three bytes below and the record after them
        0, 1, // ES_ID, the same for every track of a file that names none
        0, // no stream name, no URL, no opaque data follows
        4, (15 + width) as u8, // DecoderConfigDescriptor: its header and the record after it
        0x40, // MPEG-4 audio
        0x15, // stream type 5 for audio, with no upstream and no backward config
        0, 0, 0, // buffer size, in 16-bit units
        0, 0, 0, 0, // maximum bitrate
        0, 0, 0, 0, // average bitrate
        5, width as u8, // DecSpecificInfo, holding the config itself
    ];
    out.extend_from_slice(asc);
    Some(out)
}

/// A `.aac` file read as an audio track.
pub struct AacAudioReader {
    aac: Aac,
    extra_data: Vec<u8>,
    packet: usize,
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
    }

    fn seek_to(&mut self, pts: i64) -> i64 {
        let target = pts.max(0) as u64;
        let index = self
            .aac
            .frames
            .partition_point(|at| at.pts <= target)
            .saturating_sub(1);
        self.packet = index;
        self.aac.frames[index].pts as i64
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
        assert_eq!(packet.pts, 12 * 1024);
        // The last frame of the file, from past its seven header bytes on.
        assert_eq!(packet.data, &STEREO[4034 + 7..]);
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
        assert_eq!(header(&pce), None, "a PCE layout is not read here");
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
