//! Canonical owned bare FLAC metadata and structurally validated packet index.
use fvid_control::error::{invalid, Result};
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
#[doc(hidden)]
pub fn crc8(bytes: &[u8]) -> u8 {
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
/// verifies the encoded frame after its subframe structure establishes the end.
#[doc(hidden)]
pub fn crc16(bytes: &[u8]) -> u16 {
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
    let length = lead.leading_ones() as usize;
    if !(2..=7).contains(&length) { return None; }
    let encoded = bytes.get(..length)?;
    let mut value = u64::from(lead & (0x7f >> length));
    for byte in &encoded[1..] {
        if byte & 0xc0 != 0x80 { return None; }
        value = (value << 6) | u64::from(byte & 0x3f);
    }
    let minimum = [0, 0, 128, 2048, 65536, 2097152, 67108864, 2147483648][length];
    (value >= minimum).then_some((value, length))
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
#[doc(hidden)]
pub struct Header {
    number: u64,
    variable: bool,
    samples: u32,
    sample_rate: Option<u32>,
    bits: Option<u16>,
    channels: u16,
    len: usize,
}

/// Read a frame header, or none when these bytes are not one. The sync pattern is
/// insufficient by itself, so this is where the CRC-8 earns its keep: without it
/// the scan would strike a "header" every few tens of thousands of bytes of
/// compressed audio.
#[doc(hidden)]
pub fn frame_header(bytes: &[u8]) -> Option<Header> {
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
    if desc & 1 == 1 || block == 0 || rate == 15 || mode > 10 {
        return None;
    }
    let mut at = 4;
    // The frame or sample number comes first, then whatever the block-size and
    // sample-rate codes asked for. No file on this machine states both of those in
    // one frame, so their order here is the order the decoder this player uses
    // reads them in - and a file that keeps them the other way round fails the
    // CRC-8 below instead of playing nonsense.
    let (number, taken) = utf8_number(bytes.get(at..)?)?;
    let max = if bytes[1] & 1 != 0 { (1u64 << 36) - 1 } else { (1u64 << 31) - 1 };
    if number > max { return None; }
    at += taken;
    let samples = match block {
        1 => 192,
        2..=5 => 576 << (block - 2),
        6 => {
            let stated = *bytes.get(at)?;
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
        number,
        variable: bytes[1] & 1 != 0,
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
    /// Explicit comment mask, or the FLAC default speaker assignment.
    pub channel_mask: u32,
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
            return Err(invalid("FLAC file byte limit exceeded"));
        }
        let mut data = Vec::new();
        data.try_reserve_exact(bytes.len())
            .map_err(|_| invalid("FLAC input allocation failed"))?;
        data.extend_from_slice(bytes);
        Self::parse_owned(data, limits)
    }
    pub fn parse_owned(data: Vec<u8>, limits: &Limits) -> Result<Self> {
        Self::parse_owned_controlled(data, limits, None, usize::MAX)
    }
    pub(crate) fn parse_owned_controlled(
        data: Vec<u8>,
        limits: &Limits,
        budget: Option<usize>,
        packet_bytes: usize,
    ) -> Result<Self> {
        let bytes = data.as_slice();
        if bytes.len() > limits.file_bytes {
            return Err(invalid(&format!(
                "file is over the {} byte limit",
                limits.file_bytes
            )));
        }
        let head = Head::read(bytes)?;
        let end = bytes.len();
        // Budget reserved index storage, including unused geometric capacity.
        // The decoder allowance is unchanged from the existing controlled estimate.
        let base_estimate = data.capacity().checked_add(32 * 1024 * 1024)
            .ok_or_else(|| invalid("FLAC allocation estimate overflow"))?;
        let index_bytes = match budget {
            Some(max) => max.saturating_sub(base_estimate),
            None => usize::MAX - base_estimate,
        };
        let capacity_limit = (index_bytes / std::mem::size_of::<Frame>()).min(limits.packets);
        let mut frames: Vec<Frame> = Vec::new();
        let mut pts = 0u64;
        let mut blocking_strategy = None;
        let mut start = head.data_start;
        while start < end {
            let at = frame_header(&bytes[start..])
                .ok_or_else(|| invalid(&format!("no frame header at byte {start}")))?;
            // A valid CRC does not make a discontinuous coded clock valid.
            // Fixed streams count preceding frames; variable streams count
            // preceding samples. Do not silently replace either with our clock.
            if blocking_strategy.is_some_and(|variable| variable != at.variable) {
                return Err(invalid("FLAC blocking strategy changes within the stream"));
            }
            blocking_strategy = Some(at.variable);
            let expected_number = if at.variable {
                pts
            } else {
                u64::try_from(frames.len())
                    .map_err(|_| invalid("FLAC frame index exceeds coded-number range"))?
            };
            if at.number != expected_number {
                return Err(invalid(&format!(
                    "FLAC frame {} coded number {} disagrees with expected {expected_number}",
                    frames.len(), at.number
                )));
            }
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
            // PCM and residual bytes can contain sync, metadata and even matching
            // prefix CRCs. Follow the subframe syntax before checking its footer;
            // a checksum alone cannot establish an encoded frame boundary.
            let config = fvid_codecs::flac::Config {
                sample_rate: head.sample_rate, channels: head.channels, bits: head.bits_per_sample as u8,
            };
            let size = fvid_codecs::flac::frame_size(&bytes[start..], config)
                .map_err(|error| invalid(&format!("invalid FLAC frame at byte {start}: {error}")))?;
            debug_assert!(size >= at.len + 2);
            let stop = start.checked_add(size).filter(|stop| *stop <= end)
                .ok_or_else(|| invalid("FLAC frame boundary exceeds input"))?;
            let next = if stop == end { end } else {
                match marker(bytes, stop) {
                    Some(Mark::Frame) => stop,
                    Some(Mark::Block { after }) => skip_blocks(bytes, after),
                    None => return Err(invalid(&format!("unrecognized bytes after FLAC frame at byte {stop}"))),
                }
            };
            if frames.len() >= limits.packets {
                return Err(invalid(&format!(
                    "stream is over the {} packet limit",
                    limits.packets
                )));
            }
            if stop - start > packet_bytes {
                return Err(invalid("FLAC packet bytes exceed limit"));
            }
            let required = frames.len().checked_add(1)
                .ok_or_else(|| invalid("FLAC index overflow"))?;
            if required > capacity_limit {
                return Err(invalid(if budget.is_some() {
                    "controlled memory budget exceeded: FLAC input/index/decoder"
                } else { "FLAC index overflow" }));
            }
            if required > frames.capacity() {
                let capacity = frames.capacity().saturating_mul(2).max(4).min(capacity_limit);
                frames.try_reserve_exact(capacity - frames.len())
                    .map_err(|_| invalid("FLAC index allocation failed"))?;
                if frames.capacity() > capacity_limit {
                    return Err(invalid("FLAC index allocation exceeds configured capacity limit"));
                }
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
            channel_mask: crate::owned_flac_layout::channel_mask(&bytes[..head.data_start])?
                .unwrap_or_else(|| crate::owned_flac_decoder::channel_mask(head.channels)),
            frames,
            stream_info: head.stream_info.to_vec(),
            data_start: head.data_start,
            stated_samples: head.total_samples,
            stated_frame_sizes: (head.min_frame, head.max_frame),
            data,
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

#[cfg(test)]
mod index_growth_tests {
    use super::{Flac, Frame, Limits};
    const INPUT: &[u8] = include_bytes!("../../../tests/fixtures/playback-errors/flac-index-growth/frames-129.flac");
    #[test]
    fn index_capacity_respects_controlled_memory_even_between_growth_steps() {
        for entries in [128usize, 129, 130] {
            let data = INPUT.to_vec();
            let budget = data.capacity() + 32 * 1024 * 1024 + entries * std::mem::size_of::<Frame>();
            let parsed = Flac::parse_owned_controlled(data, &Limits::default(), Some(budget), usize::MAX);
            if entries < 129 {
                assert!(parsed.unwrap_err().to_string().contains("controlled memory budget exceeded"));
            } else {
                let flac = parsed.unwrap();
                assert_eq!(flac.frames.len(), 129);
                let reserved = flac.data.capacity() + 32 * 1024 * 1024 + flac.frames.capacity() * std::mem::size_of::<Frame>();
                assert!(reserved <= budget, "unused index capacity must be accounted for");
            }
        }
    }
    #[test]
    fn index_capacity_does_not_reserve_past_the_packet_limit() {
        let limits = Limits { packets: 129, ..Limits::default() };
        let flac = Flac::parse(INPUT, &limits).unwrap();
        assert_eq!(flac.frames.len(), 129);
        assert!(flac.frames.capacity() <= limits.packets);
    }
}
