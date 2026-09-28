//! Owned ADTS framing and AAC configuration, independent of playback.
use crate::codec::config::AacConfig;
use crate::{Result, invalid};

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
