//! Audio playback of FLAC out of the bare `.flac` file the format ships in.
//!
//! A FLAC file is a `fLaC` marker, a run of metadata blocks, and then a run of
//! audio frames. The blocks state the geometry once and for all, so unlike the
//! MPEG audio reader this one does not have to take a rate from a frame; unlike
//! the PCM containers it has no chunk holding the samples, because a FLAC frame
//! carries neither a length nor a count of them. Finding where a frame ends is
//! therefore the whole job, and the format answers it with a checksum: every
//! frame closes with a CRC-16 of itself, so a candidate end is an end only when
//! that checksum agrees.
//!
//! Four measured facts shape the reader, each checked against the packet list
//! ffprobe prints for the files under `tests/fixtures/flac`.
//!
//! * A frame header is not a fixed number of bytes. The block size sits either in
//!   a table or in the header itself, and the short last frame of a stream states
//!   its own block size in 8 or 16 bits, so a header is six bytes for a normal
//!   frame and eight for that last one.
//! * The header is checksummed on its own, with a CRC-8, so bytes that merely
//!   start with the sync pattern are not a header until that checksum says so.
//! * A frame ends where the next header begins, less the two CRC bytes - and the
//!   next thing may be a metadata block, which the format allows between frames.
//!   This machine's FFmpeg does not handle that: an interleaved block is merged
//!   into the packet before it and the decoder then reports `invalid sync code`
//!   while still counting ten frames, so the reference there is the format, not
//!   the tool.
//! * `STREAMINFO` states a total sample count and the smallest and largest frame
//!   size, and every file written here states all three exactly right, so a file
//!   whose numbers disagree with the frames it holds is refused rather than
//!   trusted.
//!
//! The decoder is symphonia's, the one the Matroska row already uses, and it asks
//! for the 34 bytes of `STREAMINFO` as setup data - the bare file keeps them in
//! its first block.

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

/// The tag [`crate::codec::make_audio_decoder`] answers for FLAC, as Matroska
/// names it, and the tag the WebM and MP4 readers already used for this codec
/// before a bare `.flac` could be opened at all.
pub const TAG: &str = "A_FLAC";

/// Bytes in the `STREAMINFO` record the decoder is handed as setup data.
const STREAM_INFO: usize = 34;

/// CRC-8 of a frame header: polynomial 0x07, no reflection, zero initial value.
fn crc8(bytes: &[u8]) -> u8 {
    let mut crc = 0u8;
    for byte in bytes {
        crc ^= byte;
        for _ in 0..8 {
            crc = if crc & 0x80 != 0 {
                (crc << 1) ^ 0x07
            } else {
                crc << 1
            };
        }
    }
    crc
}

/// CRC-16 of a frame: polynomial 0x8005, no reflection, zero initial value. This
/// is the number a frame ends with, and the only thing that says where it ended.
fn crc16(bytes: &[u8]) -> u16 {
    let mut crc = 0u16;
    for byte in bytes {
        crc ^= u16::from(*byte) << 8;
        for _ in 0..8 {
            crc = if crc & 0x8000 != 0 {
                (crc << 1) ^ 0x8005
            } else {
                crc << 1
            };
        }
    }
    crc
}

fn be16(bytes: &[u8]) -> u16 {
    let pair: [u8; 2] = bytes[..2].try_into().unwrap_or([0, 0]);
    u16::from_be_bytes(pair)
}

/// A 24-bit big-endian number, which is how `STREAMINFO` counts frame sizes.
fn be24(bytes: &[u8]) -> u32 {
    u32::from(bytes[0]) << 16 | u32::from(bytes[1]) << 8 | u32::from(bytes[2])
}

/// A block length is a byte count the walk adds to a byte position, so it widens
/// to the same type before the sum is checked against the file.
fn len24(bytes: &[u8]) -> usize {
    usize::from(bytes[0]) << 16 | usize::from(bytes[1]) << 8 | usize::from(bytes[2])
}

/// FLAC's own big-endian UTF-8 number: a value that fits in seven bits is one
/// byte, a longer one carries its continuation count in the leading byte. Returns
/// the value and how many bytes it took.
fn utf8_number(bytes: &[u8]) -> Option<(u64, usize)> {
    let lead = *bytes.first()?;
    if lead < 0x80 {
        return Some((u64::from(lead), 1));
    }
    let mut extra = 0;
    while extra < 8 && (lead >> (7 - extra)) & 1 == 1 {
        extra += 1;
    }
    if extra == 0 || extra > 7 || bytes.len() < extra + 1 {
        return None;
    }
    let mut value = u64::from(lead & (0x7f >> extra));
    for byte in &bytes[1..extra + 1] {
        if byte & 0xc0 != 0x80 {
            return None;
        }
        value = (value << 6) | u64::from(byte & 0x3f);
    }
    Some((value, extra + 1))
}

/// Sample rates a frame header's four-bit code names. Code 0 defers to
/// `STREAMINFO`, codes 12 to 14 spell the rate out in the header and are read
/// there, and 15 names nothing.
const HEADER_RATES: [Option<u32>; 15] = [
    None,
    Some(88_200),
    Some(176_400),
    Some(192_000),
    Some(8_000),
    Some(16_000),
    Some(22_050),
    Some(24_000),
    Some(32_000),
    Some(44_100),
    Some(48_000),
    Some(96_000),
    None,
    None,
    None,
];

/// Bits per sample for a frame header's three-bit code: 0 defers to `STREAMINFO`,
/// and one value in the middle is left unused by the format.
const HEADER_BITS: [Option<u16>; 8] = [
    None,
    Some(8),
    Some(12),
    None,
    Some(16),
    Some(20),
    Some(24),
    Some(32),
];

/// What one frame says about itself, and how many header bytes it says it in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Header {
    samples: u32,
    sample_rate: Option<u32>,
    bits: Option<u16>,
    channels: u16,
    len: usize,
}

/// Read a frame header, or none when these bytes are not one. The sync pattern is
/// only twelve bits of it, so this is where the CRC-8 earns its keep: without it
/// the scan would strike a "header" every few tens of thousands of bytes of
/// compressed audio.
fn frame_header(bytes: &[u8]) -> Option<Header> {
    if be16(bytes.get(..2)?) & 0xfffc != 0xfff8 {
        return None;
    }
    let desc = be16(bytes.get(2..4)?);
    let block = usize::from((desc >> 12) & 0xf);
    let rate = usize::from((desc >> 8) & 0xf);
    let mode = usize::from((desc >> 4) & 0xf);
    let bits = usize::from((desc >> 1) & 0x7);
    // One bit of the description is reserved and must read zero, and three of the
    // codes name nothing a stream is ever written with.
    if desc & 1 == 1 || block == 0 || block == 13 || block == 14 || rate == 15 || mode > 10 {
        return None;
    }
    let mut at = 4;
    // The frame or sample number comes first, then whatever the block-size and
    // sample-rate codes asked for. No file on this machine states both of those in
    // one frame, so their order here is the order the decoder this player uses
    // reads them in - and a file that keeps them the other way round fails the
    // CRC-8 below instead of playing nonsense.
    let (_, taken) = utf8_number(bytes.get(at..)?)?;
    at += taken;
    let samples = match block {
        1 => 192,
        2..=5 => 576 << (block - 2),
        6 => {
            let stated = *bytes.get(at)?;
            if stated == 0xff {
                return None;
            }
            at += 1;
            u32::from(stated) + 1
        }
        7 => {
            let stated = be16(bytes.get(at..at + 2)?);
            if stated == 0xffff {
                return None;
            }
            at += 2;
            u32::from(stated) + 1
        }
        _ => 256 << (block - 8),
    };
    let sample_rate = match rate {
        12 => {
            let stated = *bytes.get(at)?;
            if stated == 0xff {
                return None;
            }
            at += 1;
            Some(u32::from(stated) * 1000)
        }
        13 | 14 => {
            let stated = u32::from(be16(bytes.get(at..at + 2)?));
            at += 2;
            Some(if rate == 14 { stated * 10 } else { stated })
        }
        other => HEADER_RATES[other],
    };
    if *bytes.get(at)? != crc8(bytes.get(..at)?) {
        return None;
    }
    Some(Header {
        samples,
        sample_rate,
        bits: HEADER_BITS[bits],
        // The three side-stereo modes each carry two channels of their own.
        channels: if mode <= 7 { mode as u16 + 1 } else { 2 },
        len: at + 1,
    })
}

/// What can stand where a frame ends: another frame, or a metadata block, which
/// the format puts between frames and the sample run has to be walked around.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Mark {
    Frame,
    Block { after: usize },
}

fn marker(bytes: &[u8], at: usize) -> Option<Mark> {
    if frame_header(bytes.get(at..)?).is_some() {
        return Some(Mark::Frame);
    }
    let kind = *bytes.get(at)? & 0x7f;
    // A type the format has not assigned, or `STREAMINFO` - which belongs at the
    // head of the file and appears nowhere else - is not a boundary.
    if kind == 0 || kind > 6 {
        return None;
    }
    let after = at + 4 + len24(bytes.get(at + 1..at + 4)?);
    if after > bytes.len() {
        return None;
    }
    Some(Mark::Block { after })
}

/// Walk out of the metadata blocks that stand between two frames.
fn skip_blocks(bytes: &[u8], from: usize) -> usize {
    let mut pos = from;
    while let Some(Mark::Block { after }) = marker(bytes, pos) {
        pos = after;
    }
    pos
}

/// One frame of the stream: where it sits, how long it is, and the samples it
/// holds, which is what its own header states in its block-size field.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Frame {
    pub start: usize,
    pub size: usize,
    pub pts: u64,
    pub samples: u32,
}

/// What the file's own blocks say, and where the audio they describe begins.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Head {
    sample_rate: u32,
    channels: u16,
    bits_per_sample: u16,
    total_samples: u64,
    min_frame: u32,
    max_frame: u32,
    /// The record's own 34 bytes, which the decoder wants as setup data. They do
    /// not necessarily end where the audio begins: a file usually holds a comment
    /// and a padding block after them.
    stream_info: [u8; STREAM_INFO],
    data_start: usize,
}

impl Head {
    /// Walk the blocks behind the marker. A block that runs past the end of the
    /// file is not a block, and neither is a file whose blocks never name a
    /// `STREAMINFO`.
    fn read(bytes: &[u8]) -> Result<Self> {
        if bytes.len() < 4 + 4 + STREAM_INFO || bytes[..4] != *b"fLaC" {
            return Err(invalid("not a FLAC file"));
        }
        let mut pos = 4;
        let mut head: Option<Self> = None;
        loop {
            let flag = *bytes
                .get(pos)
                .ok_or_else(|| invalid("stream ended in a block header"))?;
            let size = bytes
                .get(pos + 1..pos + 4)
                .ok_or_else(|| invalid("stream ended in a block header"))?;
            let len = len24(size);
            let body = pos + 4;
            if body + len > bytes.len() {
                return Err(invalid(&format!(
                    "block at byte {pos} states {len} bytes in a file holding {} of them",
                    bytes.len() - body
                )));
            }
            if flag & 0x7f == 0 {
                if len < STREAM_INFO {
                    return Err(invalid(&format!(
                        "STREAMINFO holds {len} bytes, not the 34 it is defined by"
                    )));
                }
                let at = &bytes[body..body + STREAM_INFO];
                let rest = u64::from_be_bytes(at[10..18].try_into().unwrap_or([0; 8]));
                let mut stream_info = [0u8; STREAM_INFO];
                stream_info.copy_from_slice(at);
                head = Some(Self {
                    sample_rate: u32::try_from(rest >> 44).unwrap_or(0),
                    channels: u16::try_from((rest >> 41) & 7).unwrap_or(0) + 1,
                    bits_per_sample: u16::try_from((rest >> 36) & 0x1f).unwrap_or(0) + 1,
                    total_samples: rest & ((1u64 << 36) - 1),
                    min_frame: be24(&at[4..7]),
                    max_frame: be24(&at[7..10]),
                    stream_info,
                    data_start: 0,
                });
            }
            pos = body + len;
            if flag & 0x80 != 0 {
                break;
            }
        }
        let mut head =
            head.ok_or_else(|| invalid("no STREAMINFO block, so no geometry to play by"))?;
        if head.sample_rate == 0 {
            return Err(invalid("STREAMINFO states no sample rate"));
        }
        head.data_start = pos;
        Ok(head)
    }
}

/// A `.flac` file: the geometry its first block states, the frames that play, and
/// the record the decoder is handed along with them.
#[derive(Clone, Debug)]
pub struct Flac {
    pub tag: &'static str,
    pub sample_rate: u32,
    pub channels: u16,
    pub bits_per_sample: u16,
    pub frames: Vec<Frame>,
    /// The 34-byte `STREAMINFO` body, which the decoder needs as setup data.
    pub stream_info: Vec<u8>,
    /// Where the audio begins: past the marker and every leading block.
    pub data_start: usize,
    /// Total samples as `STREAMINFO` states them, before the frames are walked.
    pub stated_samples: u64,
    /// Smallest and largest frame size as `STREAMINFO` states them. Zero means it
    /// stated nothing, which is what a file with no idea of its own writes.
    pub stated_frame_sizes: (u32, u32),
    data: Vec<u8>,
}

impl Flac {
    /// Read a whole file: its blocks, then every frame between them, each one
    /// closed by the checksum it carries.
    pub fn parse(bytes: &[u8], limits: &Limits) -> Result<Self> {
        if bytes.len() > limits.file_bytes {
            return Err(invalid(&format!(
                "file is over the {} byte limit",
                limits.file_bytes
            )));
        }
        let head = Head::read(bytes)?;
        let end = bytes.len();
        let mut frames: Vec<Frame> = Vec::new();
        let mut pts = 0u64;
        let mut start = head.data_start;
        while start < end {
            let at = frame_header(&bytes[start..])
                .ok_or_else(|| invalid(&format!("no frame header at byte {start}")))?;
            if at.sample_rate.is_some_and(|rate| rate != head.sample_rate) {
                return Err(invalid(&format!(
                    "frame {} states {} Hz in a {} Hz stream",
                    frames.len(),
                    at.sample_rate.unwrap_or(0),
                    head.sample_rate
                )));
            }
            if at.bits.is_some_and(|bits| bits != head.bits_per_sample) {
                return Err(invalid(&format!(
                    "frame {} states {}-bit samples in a {}-bit stream",
                    frames.len(),
                    at.bits.unwrap_or(0),
                    head.bits_per_sample
                )));
            }
            if at.channels != head.channels {
                return Err(invalid(&format!(
                    "frame {} carries {} channels in a stream of {}",
                    frames.len(),
                    at.channels,
                    head.channels
                )));
            }
            // A frame ends at the next boundary whose last two bytes are the
            // CRC-16 of everything from here to there. A boundary that does not
            // check out is the middle of this frame, not its end. The block run a
            // boundary names belongs to neither frame: it stays out of this packet
            // and is walked past on the way to the next header.
            let mut seek = start + at.len;
            let (stop, next) = loop {
                if seek >= start + at.len + 2 {
                    if let Some(mark) = marker(bytes, seek) {
                        if crc16(&bytes[start..seek - 2]) == be16(&bytes[seek - 2..seek]) {
                            break match mark {
                                Mark::Frame => (seek, seek),
                                Mark::Block { after } => (seek, skip_blocks(bytes, after)),
                            };
                        }
                        if let Mark::Block { after } = mark {
                            seek = after;
                            continue;
                        }
                    }
                }
                seek += 1;
                if seek >= end {
                    // Nothing followed it, so this frame is the last thing the file
                    // holds and its checksum must sit against the end of the file.
                    if end >= start + at.len + 2
                        && crc16(&bytes[start..end - 2]) == be16(&bytes[end - 2..end])
                    {
                        break (end, end);
                    }
                    return Err(invalid(&format!(
                        "frame at byte {start} has no end its own checksum agrees with"
                    )));
                }
            };
            if frames.len() >= limits.packets {
                return Err(invalid(&format!(
                    "stream is over the {} packet limit",
                    limits.packets
                )));
            }
            frames.push(Frame {
                start,
                size: stop - start,
                pts,
                samples: at.samples,
            });
            pts += u64::from(at.samples);
            // Where the next frame begins: the header that ended the scan, or
            // whatever came after the block run that ended it.
            start = next;
        }
        if frames.is_empty() {
            return Err(invalid("no audio frames after the metadata blocks"));
        }
        // Both numbers come from the first block and both describe the frames, so
        // one that disagrees is a file whose blocks were not written against the
        // audio they hold.
        if head.total_samples != 0 && head.total_samples != pts {
            return Err(invalid(&format!(
                "STREAMINFO states {} samples, the frames hold {pts}",
                head.total_samples
            )));
        }
        let smallest = frames.iter().map(|at| at.size as u32).min().unwrap_or(0);
        let largest = frames.iter().map(|at| at.size as u32).max().unwrap_or(0);
        if head.min_frame != 0 && (head.min_frame != smallest || head.max_frame != largest) {
            return Err(invalid(&format!(
                "STREAMINFO states frame sizes {}..{}, the frames are {smallest}..{largest}",
                head.min_frame, head.max_frame
            )));
        }
        Ok(Self {
            tag: TAG,
            sample_rate: head.sample_rate,
            channels: head.channels,
            bits_per_sample: head.bits_per_sample,
            frames,
            stream_info: head.stream_info.to_vec(),
            data_start: head.data_start,
            stated_samples: head.total_samples,
            stated_frame_sizes: (head.min_frame, head.max_frame),
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

    /// Samples the stream runs to, counted off the frames themselves.
    pub fn samples(&self) -> u64 {
        self.frames
            .last()
            .map(|at| at.pts + u64::from(at.samples))
            .unwrap_or(0)
    }
}

/// A `.flac` file read as an audio track.
pub struct FlacAudioReader {
    flac: Flac,
    packet: usize,
}

impl FlacAudioReader {
    /// Read a whole file. Every frame's length is a checksum away, so all of them
    /// are walked before the first one plays.
    pub fn open<R: Read>(mut reader: R, limits: Limits) -> Result<Self> {
        let mut bytes = Vec::new();
        let cap = u64::try_from(limits.file_bytes).unwrap_or(u64::MAX);
        reader.by_ref().take(cap + 1).read_to_end(&mut bytes)?;
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
        Some(Duration::from_secs_f64(
            self.flac.samples() as f64 / f64::from(self.flac.sample_rate),
        ))
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
    use super::{Flac, FlacAudioReader, Frame, Limits, TAG, crc16};
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
    const TONE: &[u8] = include_bytes!("../tests/fixtures/flac/tone.flac");
    const STEREO: &[u8] = include_bytes!("../tests/fixtures/flac/stereo.flac");
    const WIDE: &[u8] = include_bytes!("../tests/fixtures/flac/twenty-four.flac");
    const SMALL: &[u8] = include_bytes!("../tests/fixtures/flac/small-block.flac");

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
            vec![
                8286, 8672, 9024, 9355, 9725, 10_097, 10_452, 10_783, 11_153, 11_531
            ]
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
        let video = include_bytes!("../tests/fixtures/audio/pcm-screen.mov");
        let error = Flac::parse(video, &Limits::default()).expect_err("a movie is not a FLAC file");
        assert!(
            error.to_string().contains("not a FLAC file"),
            "said: {error}"
        );
    }

    #[test]
    fn a_frame_whose_checksum_does_not_match_is_refused() {
        let mut bytes = TONE.to_vec();
        // One byte inside the first frame's residual: the frame can no longer end
        // anywhere, because no candidate boundary holds a checksum of the changed
        // bytes.
        bytes[8400] ^= 0xff;
        let error = Flac::parse(&bytes, &Limits::default()).expect_err("damage is not a stream");
        assert!(
            error
                .to_string()
                .contains("no end its own checksum agrees with"),
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
