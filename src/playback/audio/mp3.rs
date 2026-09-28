//! Audio playback of MPEG layer II and layer III out of the bare file the
//! format ships in.
//!
//! Unlike the PCM containers, which state a geometry and then hold samples, a
//! `.mp3` file states nothing up front: every frame carries its own header, and
//! a stream is a run of frames with tags bolted on either end. What the player
//! asks a container for - a codec tag, a rate, a channel count, packets with
//! timestamps - is therefore read out of the frames themselves, and the packets
//! handed to [`crate::codec::make_audio_decoder`] are whole frames, header
//! included, exactly as ffprobe reports them.
//!
//! Three measured facts shape this reader. A file usually begins with an ID3v2
//! tag whose length is a synchsafe number, and the audio starts after all of it.
//! The first frame of a stream written by LAME is an `Info` frame whose side
//! info carries a tag rather than audio, and the reference demuxer consumes it
//! without emitting a packet for it, so neither does this one - its placeholder
//! bitrate and channel mode are also the wrong geometry to trust. And a frame's
//! length is not stored anywhere: it is computed from the bitrate, rate and
//! padding bit of the header it starts with. Every constant below was checked
//! against the packet list ffprobe prints for the files under
//! `tests/fixtures/mp3`.
//!
//! Layer I is read and refused. Its frame length is the one number here that no
//! file on this machine states: the archive sample the coverage matrix pins for
//! K-Lite's "MPEG Layer I" row carries layer III headers and ffprobe names its
//! descriptor `mp3`, and this FFmpeg writes no layer I encoder, so the formula
//! would be a claim with nothing to check it against.

use crate::audio::{AudioStream, AudioTrack, EncodedPacket};
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

/// The tag [`crate::codec::make_audio_decoder`] answers for layer II, as
/// Matroska names it.
pub const LAYER_II: &str = "A_MPEG/L2";
/// The same for layer III: the tag the Matroska and MP4 readers already used for
/// this codec, before a bare `.mp3` file could be opened at all.
pub const LAYER_III: &str = "A_MPEG/L3";

/// A frame header's MPEG audio layer. The two-bit field counts downwards from
/// one, so a value of 1 names layer III.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Layer {
    One,
    Two,
    Three,
}

/// A frame header: what the frame is, how long it is, and what it costs to size
/// one.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Header {
    layer: Layer,
    /// MPEG-1, or one of the two back extensions.
    mpeg1: bool,
    sample_rate: u32,
    /// Mono, dual-channel, intensity or joint stereo.
    mode: u8,
    bit_rate: u32,
    samples: u32,
    size: usize,
}

impl Header {
    fn channels(&self) -> u16 {
        u16::from(self.mode != 3) + 1
    }

    /// The tag a stream of this layer plays under, and none for the layer whose
    /// frames this reader does not size.
    fn tag(&self) -> Option<&'static str> {
        match self.layer {
            Layer::Two => Some(LAYER_II),
            Layer::Three => Some(LAYER_III),
            Layer::One => None,
        }
    }
}

/// Which row of the bitrate tables a layer's header points at.
fn table_row(layer: Layer) -> usize {
    match layer {
        Layer::One => 0,
        Layer::Two => 1,
        Layer::Three => 2,
    }
}

/// Kilobits per second per bitrate index, one row per layer for the two version
/// families. A layer III stream at index 7 is 96 kbps, which is what the coverage
/// fixture is encoded at and what ffprobe reports for it. Index 15 names no
/// bitrate and index 0 only a free-format stream, so neither is ever taken.
const BITRATES: [[[u32; 16]; 3]; 2] = [
    // MPEG-1.
    [
        [
            0, 32, 64, 96, 128, 160, 192, 224, 256, 288, 320, 352, 384, 416, 448, 0,
        ],
        [
            0, 32, 48, 56, 64, 80, 96, 112, 128, 160, 192, 224, 256, 320, 384, 0,
        ],
        [
            0, 32, 40, 48, 56, 64, 80, 96, 112, 128, 160, 192, 224, 256, 320, 0,
        ],
    ],
    // MPEG-2 and MPEG-2.5 share these two rows.
    [
        [
            0, 32, 48, 56, 64, 80, 96, 112, 128, 144, 160, 176, 192, 224, 256, 0,
        ],
        [
            0, 8, 16, 24, 32, 40, 48, 56, 64, 80, 96, 112, 128, 144, 160, 0,
        ],
        [
            0, 8, 16, 24, 32, 40, 48, 56, 64, 80, 96, 112, 128, 144, 160, 0,
        ],
    ],
];

/// Sample rates by version family and by the header's two-bit index.
const SAMPLE_RATES: [[u32; 3]; 3] = [
    [44100, 48000, 32000], // MPEG-1
    [22050, 24000, 16000], // MPEG-2
    [11025, 12000, 8000],  // MPEG-2.5
];

/// Read one frame header, or none when these four bytes are not a header at all.
/// Layer I comes back as a header so that a stream built from it is refused by
/// name rather than quietly lost to the resync scan.
fn header(bytes: &[u8]) -> Option<Header> {
    let word = u32::from_be_bytes(bytes.get(..4)?.try_into().ok()?);
    if word >> 21 != 0x7ff {
        return None;
    }
    // The version field leaves one of its four values unused, as does the layer.
    let version = match (word >> 19) & 3 {
        3 => 0,
        2 => 1,
        0 => 2,
        _ => return None,
    };
    let layer = match (word >> 17) & 3 {
        3 => Layer::One,
        2 => Layer::Two,
        1 => Layer::Three,
        _ => return None,
    };
    let mpeg1 = version == 0;
    let rate_index = ((word >> 10) & 3) as usize;
    let bit_rate =
        BITRATES[usize::from(!mpeg1)][table_row(layer)][((word >> 12) & 0xf) as usize] * 1000;
    if bit_rate == 0 || rate_index == 3 {
        return None;
    }
    let sample_rate = SAMPLE_RATES[version][rate_index];
    let pad = ((word >> 9) & 1) as usize;
    // 144 samples per byte of bitrate in MPEG-1, half that in the extensions,
    // where a layer III frame carries 576 samples instead of 1152. Layer I counts
    // its length in slots of four bytes - and is refused before it is played.
    let (samples, size) = match layer {
        Layer::One => (
            384,
            12 * bit_rate as usize / sample_rate as usize * 4 + pad * 4,
        ),
        Layer::Two => (1152, 144 * bit_rate as usize / sample_rate as usize + pad),
        Layer::Three if mpeg1 => (1152, 144 * bit_rate as usize / sample_rate as usize + pad),
        Layer::Three => (576, 72 * bit_rate as usize / sample_rate as usize + pad),
    };
    Some(Header {
        layer,
        mpeg1,
        sample_rate,
        mode: ((word >> 6) & 3) as u8,
        bit_rate,
        samples,
        size,
    })
}

/// Where an `Info`/`Xing` frame keeps its tag: past the four header bytes and
/// past the side info, whose length the version family and the channel mode fix.
/// The three cases this machine's LAME writes were each checked at this offset -
/// 21 for MPEG-1 mono and for MPEG-2 stereo, 36 for MPEG-1 stereo.
fn info_tag_at(at: &Header) -> usize {
    4 + match (at.mpeg1, at.mode) {
        (true, 3) => 17,
        (true, _) => 32,
        (false, 3) => 9,
        (false, _) => 17,
    }
}

/// One frame of the stream: where it sits in the file, how long it is, and the
/// sample it starts on once the info frame ahead of it is out of the count.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Frame {
    pub start: usize,
    pub size: usize,
    pub pts: u64,
    pub samples: u32,
    pub bit_rate: u32,
}

/// A `.mp3` file: its geometry, the frames that play, and what was read around
/// them to get there.
#[derive(Clone, Debug)]
pub struct Mp3 {
    pub tag: &'static str,
    pub sample_rate: u32,
    pub channels: u16,
    pub frames: Vec<Frame>,
    /// Bytes of ID3v2 left in front of the audio, whether one tag or several.
    pub tag_bytes: usize,
    /// Whether the file ended in the 128-byte `TAG` record.
    pub id3v1: bool,
    /// Whether the first frame was an `Info`/`Xing` header rather than audio, and
    /// so is not one of `frames`.
    pub info_frame: bool,
    data: Vec<u8>,
}

impl Mp3 {
    /// Read a whole file: the tags, then the run of frames between them.
    pub fn parse(bytes: &[u8], limits: &Limits) -> Result<Self> {
        if bytes.len() > limits.file_bytes {
            return Err(invalid(&format!(
                "file is over the {} byte limit",
                limits.file_bytes
            )));
        }
        let tag_bytes = id3v2_bytes(bytes);
        let mut end = bytes.len();
        let id3v1 = tag_bytes + 128 <= end && bytes[end - 128..][..3] == *b"TAG";
        if id3v1 {
            end -= 128;
        }
        // The first header may sit anywhere ahead of it, since a file whose head was
        // chopped off still names its frames from there on. After that first header
        // the run must be contiguous: every frame starts where the last one ended,
        // with no gap to scan across. That is what a real MPEG audio stream looks
        // like end to end, and what a video file that happens to hold four bytes
        // resembling a header does not.
        let mut found: Vec<Header> = Vec::new();
        let mut starts: Vec<usize> = Vec::new();
        let mut pos = tag_bytes;
        while pos + 4 <= end && header(&bytes[pos..]).is_none() {
            pos += 1;
        }
        let first_start = pos;
        while pos + 4 <= end {
            let Some(at) = header(&bytes[pos..]) else {
                break;
            };
            if at.tag().is_none() {
                return Err(invalid(
                    "MPEG audio layer I is not a layer this reader sizes frames for",
                ));
            }
            if pos + at.size > end {
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
            pos += at.size;
        }
        if found.is_empty() {
            return Err(invalid("no MPEG audio frames in the file"));
        }
        // A run that stops well short of the end of the file was never the file: a
        // stream of frames fills it, and anything else is another container whose
        // bytes this reader happened to make a header out of.
        if (pos - first_start) * 10 < (end - first_start) * 9 {
            return Err(invalid(&format!(
                "the frames run to byte {pos} of a stream ending at {}",
                end
            )));
        }
        // The info frame states a placeholder bitrate where a variable-bitrate
        // stream has no bitrate to state, so it sets no geometry and plays nothing.
        let info_frame = found.len() > 1 && carries_info(bytes, &found[0], starts[0]);
        let skip = usize::from(info_frame);
        let first = found
            .get(skip)
            .ok_or_else(|| invalid("a stream that is nothing but an info frame"))?;
        let mut frames = Vec::with_capacity(found.len() - skip);
        let mut pts = 0u64;
        for (at, start) in found[skip..].iter().zip(&starts[skip..]) {
            if at.sample_rate != first.sample_rate
                || at.channels() != first.channels()
                || at.layer != first.layer
            {
                return Err(invalid(&format!(
                    "stream changes geometry at frame {}: {} Hz {} ch after {} Hz {} ch",
                    frames.len(),
                    at.sample_rate,
                    at.channels(),
                    first.sample_rate,
                    first.channels()
                )));
            }
            frames.push(Frame {
                start: *start,
                size: at.size,
                pts,
                samples: at.samples,
                bit_rate: at.bit_rate,
            });
            pts += u64::from(at.samples);
        }
        Ok(Self {
            // Safe to unwrap: a header is only kept once it names a played layer.
            tag: first.tag().unwrap_or(LAYER_III),
            sample_rate: first.sample_rate,
            channels: first.channels(),
            frames,
            tag_bytes,
            id3v1,
            info_frame,
            data: bytes.to_vec(),
        })
    }

    /// Frames the stream hands over as packets.
    pub fn packets(&self) -> usize {
        self.frames.len()
    }

    /// One frame's bytes, header included, as the decoder expects them.
    pub fn packet(&self, index: usize) -> &[u8] {
        let at = self.frames[index.min(self.frames.len() - 1)];
        &self.data[at.start..at.start + at.size]
    }

    /// Samples the stream runs to, which is its length for a file that states no
    /// duration of its own.
    pub fn samples(&self) -> u64 {
        self.frames
            .last()
            .map(|at| at.pts + u64::from(at.samples))
            .unwrap_or(0)
    }
}

/// Whether the frame at `start` is an `Info`/`Xing` header: its tag sits at a
/// fixed distance into the side info.
fn carries_info(bytes: &[u8], at: &Header, start: usize) -> bool {
    if at.layer != Layer::Three {
        return false;
    }
    let offset = start + info_tag_at(at);
    bytes
        .get(offset..offset + 4)
        .is_some_and(|word| word == *b"Xing" || word == *b"Info")
}

/// How many bytes of ID3v2 stand in front of the audio, counting a second tag
/// that follows the first. A header that does not hold its own length is treated
/// as no header at all, leaving its bytes to the frame scan.
fn id3v2_bytes(bytes: &[u8]) -> usize {
    let mut total = 0;
    while bytes
        .get(total..total + 10)
        .is_some_and(|head| head[..3] == *b"ID3" && head[3] != 0xff && head[5] & 0x0f == 0)
    {
        // The length is synchsafe: seven bits per byte, most significant first.
        let [a, b, c, d]: [u8; 4] = bytes[total + 6..total + 10]
            .try_into()
            .expect("four length bytes");
        if (a | b | c | d) & 0x80 != 0 {
            return total;
        }
        let next = 10
            + ((usize::from(a) << 21)
                | (usize::from(b) << 14)
                | (usize::from(c) << 7)
                | usize::from(d));
        if total + next > bytes.len() {
            return total;
        }
        total += next;
    }
    total
}

/// A `.mp3` file read as an audio track.
pub struct Mp3AudioReader {
    mp3: Mp3,
    packet: usize,
}

impl Mp3AudioReader {
    /// Read a whole file. The frames lie at lengths only their own headers state,
    /// so all of them are walked before the first one plays.
    pub fn open<R: Read>(mut reader: R, limits: Limits) -> Result<Self> {
        let mut bytes = Vec::new();
        let cap = u64::try_from(limits.file_bytes).unwrap_or(u64::MAX);
        reader.by_ref().take(cap + 1).read_to_end(&mut bytes)?;
        Ok(Self {
            mp3: Mp3::parse(&bytes, &limits)?,
            packet: 0,
        })
    }

    /// The file's own shape, for a caller that wants to say what it opened.
    pub fn mp3(&self) -> &Mp3 {
        &self.mp3
    }
}

impl AudioStream for Mp3AudioReader {
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
        Some(Duration::from_secs_f64(
            self.mp3.samples() as f64 / f64::from(self.mp3.sample_rate),
        ))
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
    }

    fn seek_to(&mut self, pts: i64) -> i64 {
        let target = pts.max(0) as u64;
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
    fn a_timestamp_lands_on_the_frame_at_or_before_it() {
        let mut reader = open(TONE);
        // The frames step by 1152 samples, so the middle of one is still reached
        // from its own start, and past the end lands on the last frame.
        assert_eq!(reader.seek_to(2_304), 2_304);
        assert_eq!(reader.seek_to(2_400), 2_304);
        assert_eq!(reader.seek_to(-1), 0);
        assert_eq!(reader.seek_to(1 << 30), 10_368);
        let packet = reader.next_packet().expect("packet").expect("last");
        assert_eq!(packet.pts, 10_368);
        assert!(reader.next_packet().expect("no more").is_none());
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
    fn a_layer_the_reader_does_not_size_is_refused_by_name() {
        // A layer I header: sync, MPEG-1, the layer field that counts down to one,
        // 224 kbps at 44.1 kHz. Nothing on this machine writes a file like it, which
        // is the whole reason it is refused rather than guessed at.
        let error = Mp3::parse(&0xffff_74c4u32.to_be_bytes(), &Limits::default())
            .err()
            .expect("layer I is refused");
        assert!(
            error
                .to_string()
                .contains("MPEG audio layer I is not a layer"),
            "{error}"
        );
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
