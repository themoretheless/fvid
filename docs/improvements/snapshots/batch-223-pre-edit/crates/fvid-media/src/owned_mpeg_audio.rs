//! Audio playback of MPEG layer II and layer III out of the bare file the
//! format ships in.
//!
//! Unlike the PCM containers, which state a geometry and then hold samples, a
//! `.mp3` file states nothing up front: every frame carries its own header, and
//! a stream is a run of frames with tags bolted on either end. What the player
//! asks a container for - a codec tag, a rate, a channel count, packets with
//! timestamps - is therefore read out of the frames themselves, and the packets
//! handed to the owned MPEG decoder are whole frames, header
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
//! Layer I uses four-byte slots, including its padding. All three layers use
//! owned kernels for bounded free-format frame qualification.

use fvid_control::error::{Result, invalid};

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

/// The tag the owned MPEG decoder answers for layer II, as
/// Matroska names it.
pub const LAYER_I: &str = "A_MPEG/L1";
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

    /// Matroska codec identifier for each owned MPEG audio layer.
    fn tag(&self) -> Option<&'static str> {
        match self.layer {
            Layer::Two => Some(LAYER_II),
            Layer::Three => Some(LAYER_III),
            Layer::One => Some(LAYER_I),
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
/// All supported layers retain their normative slot sizing.
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

// Free-format Layer II/III headers name geometry but not their frame length.
fn free_audio_header(bytes: &[u8]) -> Option<Header> {
    let mut four: [u8; 4] = bytes.get(..4)?.try_into().ok()?;
    if four[2] & 0xf0 != 0 || !matches!((four[1] >> 1) & 3, 1 | 2 | 3) {
        return None;
    }
    four[2] |= 0x10; // Use the normal parser only for version/rate/channel fields.
    let mut at = header(&four)?;
    at.bit_rate = 0;
    at.size = 0;
    Some(at)
}
fn audio_header(bytes: &[u8]) -> Option<Header> {
    header(bytes).or_else(|| free_audio_header(bytes))
}
fn same_free_geometry(a: Header, b: Header) -> bool {
    a.layer == b.layer && a.sample_rate == b.sample_rate && a.channels() == b.channels()
}
enum FreeDecoder {
    One(Box<fvid_codecs::mp1::Decoder>),
    Two(fvid_codecs::mp2::Decoder),
    Three(Box<fvid_codecs::mp3::Decoder>),
}
impl FreeDecoder {
    fn decode(&mut self, packet: &[u8]) -> Option<()> {
        match self {
            Self::One(d) => {
                d.decode(packet).ok()?;
            }
            Self::Two(d) => {
                d.decode(packet).ok()?;
            }
            Self::Three(d) => {
                d.decode(packet).ok()?;
            }
        }
        Some(())
    }
}
fn start_free_decoder(at: Header, packet: &[u8], info: Option<&[u8]>) -> Option<FreeDecoder> {
    if at.layer == Layer::One {
        let mut decoder =
            Box::new(fvid_codecs::mp1::Decoder::new(at.sample_rate, at.channels()).ok()?);
        decoder.decode(packet).ok()?;
        Some(FreeDecoder::One(decoder))
    } else if at.layer == Layer::Three {
        let mut decoder = Box::<fvid_codecs::mp3::Decoder>::default();
        if let Some(info) = info {
            decoder.prime_info(info).ok()?;
        }
        if fvid_codecs::mp3::is_info_frame(packet).ok()? {
            decoder.prime_info(packet).ok()?;
        } else {
            decoder.decode(packet).ok()?;
        }
        Some(FreeDecoder::Three(decoder))
    } else {
        let mut decoder = fvid_codecs::mp2::Decoder::new(at.sample_rate, at.channels()).ok()?;
        decoder.decode(packet).ok()?;
        Some(FreeDecoder::Two(decoder))
    }
}
fn padding_bytes(layer: Layer, third: u8) -> usize {
    usize::from((third >> 1) & 1) * if layer == Layer::One { 4 } else { 1 }
}
fn infer_free_audio_base(
    bytes: &[u8],
    start: usize,
    end: usize,
    at: Header,
    info: Option<&[u8]>,
    packet_bytes: usize,
) -> Option<usize> {
    let padding = padding_bytes(at.layer, bytes[start + 2]);
    // Bound scanning by the Layer II ceiling or the 640 kbit/s Layer III
    // free-format extension. A candidate needs a confirmed header chain.
    let coefficient = at.samples as usize / 8;
    let ceiling = match at.layer {
        Layer::One => 448000,
        Layer::Two => 384000,
        Layer::Three => 640000,
    };
    let maximum = (coefficient * ceiling / at.sample_rate as usize
        + if at.layer == Layer::One { 4 } else { 1 })
    .min(end - start)
    .min(packet_bytes);
    let mut rejected_bases = Vec::new();
    for size in 8..=maximum {
        let next = start + size;
        let Some(second) = free_audio_header(&bytes[next..end]) else {
            continue;
        };
        if !same_free_geometry(at, second) {
            continue;
        }
        let base = size.checked_sub(padding)?;
        // A damaged frame must not disappear inside a larger candidate whose
        // spacing is a multiple of an already confirmed header chain.
        if rejected_bases.iter().any(|&previous: &usize| {
            let multiple = base / previous;
            multiple >= 2
                && base <= multiple * (previous + if at.layer == Layer::One { 4 } else { 1 })
        }) {
            continue;
        }

        let second_end = next.checked_add(base + padding_bytes(at.layer, bytes[next + 2]))?;
        if second_end > end || second_end - next > packet_bytes {
            continue;
        }
        let confirmed = second_end == end
            || free_audio_header(&bytes[second_end..end]).is_some_and(|third| {
                if !same_free_geometry(at, third) {
                    return false;
                }
                if at.layer == Layer::Two {
                    return true;
                }
                // A fake ancillary header halfway through a valid frame must
                // not mark the real spacing as a multiple of a damaged chain.
                let third_end = second_end + base + padding_bytes(at.layer, bytes[second_end + 2]);
                third_end == end
                    || (third_end < end
                        && free_audio_header(&bytes[third_end..end])
                            .is_some_and(|fourth| same_free_geometry(at, fourth)))
            });
        if !confirmed {
            continue;
        }
        if let Some(mut decoder) = start_free_decoder(at, &bytes[start..next], info) {
            if decoder.decode(&bytes[next..second_end]).is_some() {
                return Some(base);
            }
            // A terminal two-header candidate can be a false header inside a
            // single EOF-bounded Layer III frame. Only a longer confirmed
            // chain rules out that interpretation. Keep Layer II's policy.
            if at.layer == Layer::Two || second_end < end {
                rejected_bases.push(base);
            }
        }
    }
    // With a single complete frame, the file boundary itself supplies the length.
    if rejected_bases.is_empty() && end - start <= maximum {
        if start_free_decoder(at, &bytes[start..end], info).is_some() {
            return (end - start).checked_sub(padding);
        }
    }
    None
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
    /// Trusted encoder delay/padding, after bounded tag/CRC validation.
    pub gapless: Option<fvid_codecs::mp3_gapless::Gapless>,
    presentation_window: std::ops::Range<u64>,
    info_range: Option<std::ops::Range<usize>>,
    data: Vec<u8>,
}

impl Mp3 {
    /// Read a whole file: the tags, then the run of frames between them.
    pub fn parse(bytes: &[u8], limits: &Limits) -> Result<Self> {
        let mut parsed = Self::parse_inner(bytes, limits, None, usize::MAX)?;
        parsed
            .data
            .try_reserve_exact(bytes.len())
            .map_err(|_| invalid("MPEG file allocation failed"))?;
        parsed.data.extend_from_slice(bytes);
        Ok(parsed)
    }
    /// Consume input storage without a second copy of the compressed file.
    pub fn parse_owned(bytes: Vec<u8>, limits: &Limits) -> Result<Self> {
        Self::parse_owned_admitted(bytes, limits, None)
    }
    /// Account input, growing index, free-format probing and decoder staging
    /// before index allocations. The caller admits input storage before reading.
    pub fn parse_owned_admitted(
        bytes: Vec<u8>,
        limits: &Limits,
        maximum: Option<usize>,
    ) -> Result<Self> {
        Self::parse_owned_controlled(bytes, limits, maximum, usize::MAX)
    }
    /// The encoded packet bound also applies to free-format inference probes.
    pub(crate) fn parse_owned_controlled(
        bytes: Vec<u8>,
        limits: &Limits,
        maximum: Option<usize>,
        packet_bytes: usize,
    ) -> Result<Self> {
        let mut parsed = Self::parse_inner(&bytes, limits, maximum, packet_bytes)?;
        parsed.data = bytes;
        Ok(parsed)
    }
    fn parse_inner(
        bytes: &[u8],
        limits: &Limits,
        maximum: Option<usize>,
        packet_bytes: usize,
    ) -> Result<Self> {
        let initial = bytes
            .len()
            .checked_add(fvid_codecs::mp3::Decoder::decode_admission_bytes())
            .ok_or_else(|| invalid("MPEG allocation estimate overflow"))?;
        if maximum.is_some_and(|max| initial > max) {
            return Err(invalid(
                "controlled memory budget exceeded: MPEG input/decoder",
            ));
        }
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
        while pos + 4 <= end && audio_header(&bytes[pos..]).is_none() {
            pos += 1;
        }
        let first_start = pos;
        let mut free_base = None;
        while pos + 4 <= end {
            let Some(mut at) = audio_header(&bytes[pos..]) else {
                break;
            };
            if at.bit_rate == 0 {
                let base = match free_base {
                    Some(base) => base,
                    None => infer_free_audio_base(
                        bytes,
                        pos,
                        end,
                        at,
                        found.first().and_then(|first| {
                            (found.len() == 1 && carries_info(bytes, first, starts[0]))
                                .then(|| &bytes[starts[0]..starts[0] + first.size])
                        }),
                        packet_bytes,
                    )
                    .ok_or_else(|| {
                        invalid("cannot infer free-format MPEG audio frame boundaries within encoded packet byte limit")
                    })?,
                };
                free_base = Some(base);
                at.size = base + padding_bytes(at.layer, bytes[pos + 2]);
                // Length-derived bitrate is an estimate; it does not select the
                // normative free-format allocation table in the owned decoder.
                at.bit_rate =
                    (base as u64 * u64::from(at.sample_rate) / u64::from(at.samples / 8)) as u32;
            }
            if at.size > packet_bytes {
                return Err(invalid("MPEG packet bytes exceed limit"));
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
            let entries = found
                .len()
                .checked_add(1)
                .ok_or_else(|| invalid("MPEG index overflow"))?;
            let index_bytes = entries
                .max(4)
                .checked_mul(
                    2 * (std::mem::size_of::<Header>()
                        + std::mem::size_of::<usize>()
                        + std::mem::size_of::<Frame>()),
                )
                .and_then(|n| n.checked_add(bytes.len()))
                .and_then(|n| n.checked_add(fvid_codecs::mp3::Decoder::decode_admission_bytes()))
                .ok_or_else(|| invalid("MPEG allocation estimate overflow"))?;
            if maximum.is_some_and(|max| index_bytes > max) {
                return Err(invalid(
                    "controlled memory budget exceeded: MPEG file/index/decoder",
                ));
            }
            found
                .try_reserve(1)
                .map_err(|_| invalid("MPEG header index allocation failed"))?;
            starts
                .try_reserve(1)
                .map_err(|_| invalid("MPEG offset index allocation failed"))?;
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
        let info_frame = carries_info(bytes, &found[0], starts[0]);
        let skip = usize::from(info_frame);
        let first = found
            .get(skip)
            .ok_or_else(|| invalid("a stream that is nothing but an info frame"))?;
        let mut frames = Vec::new();
        frames
            .try_reserve_exact(found.len() - skip)
            .map_err(|_| invalid("MPEG frame index allocation failed"))?;
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
        let gapless = if info_frame {
            fvid_codecs::mp3_gapless::parse(&bytes[starts[0]..starts[0] + found[0].size])
                .ok()
                .flatten()
                .filter(|info| {
                    info.frames
                        .is_none_or(|count| count as usize == frames.len())
                })
                .and_then(|info| info.gapless)
                .filter(|tag| tag.window(pts, 529).is_ok())
        } else {
            None
        };
        let presentation_window = gapless
            .map(|tag| tag.window(pts, 529).unwrap())
            .unwrap_or(0..pts);
        Ok(Self {
            // Safe to unwrap: a header is only kept once it names a played layer.
            tag: first.tag().unwrap_or(LAYER_III),
            sample_rate: first.sample_rate,
            channels: first.channels(),
            frames,
            tag_bytes,
            id3v1,
            info_frame,
            gapless,
            presentation_window,
            info_range: info_frame.then(|| starts[0]..starts[0] + found[0].size),
            data: Vec::new(),
        })
    }

    /// Trusted audible window in coded sample coordinates.
    pub fn presentation_window(&self) -> std::ops::Range<u64> {
        self.presentation_window.clone()
    }
    /// Optional metadata packet whose bytes seed the first audio reservoir.
    pub fn info_packet(&self) -> Option<&[u8]> {
        self.info_range.as_ref().map(|r| &self.data[r.clone()])
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
    /// Audible samples after trusted gapless edits; packet PTS remain coded.
    pub fn presentation_samples(&self) -> u64 {
        self.presentation_window.end - self.presentation_window.start
    }
}

/// Whether the frame at `start` is an `Info`/`Xing` header: its tag sits at a
/// fixed distance into the side info.
fn carries_info(bytes: &[u8], at: &Header, start: usize) -> bool {
    if at.layer != Layer::Three {
        return false;
    }
    bytes
        .get(start..start + at.size)
        .is_some_and(|packet| fvid_codecs::mp3::is_info_frame(packet).unwrap_or(false))
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

/// Signature probe only; complete structure and payload validation is separate.
pub fn has_signature(bytes: &[u8]) -> bool {
    bytes.starts_with(b"ID3") || audio_header(bytes).is_some()
}
