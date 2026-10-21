//! Owned WavPack lossless/hybrid main-stream reconstruction and sample-frame assembly.
//!
//! Each component block owns its entropy, predictor history and sample CRC.
//! Integer fixup or bit-exact IEEE reconstruction completes before false-stereo
//! expansion. A packet contains all mono/stereo components for one sample window;
//! the assembler validates boundaries, time/format and declared speaker layout
//! before interleaving. A failed later component publishes neither PCM nor layout.
//! Raw packets keep their complete headers; Matroska packets restore the shared
//! version and per-component compact headers. Hybrid main streams reconstruct
//! their quantized PCM; paired correction streams reconstruct lossless PCM.
//! Raw/static/adaptive DSD bytes feed a persistent, transactional PCM decimator.

use crate::audio::{AudioDecode, AudioPacket};
use crate::{Result, invalid};

/// Bytes between a block's `wvpk` tag and its first metadata item.
pub(crate) const HEADER_BYTES: usize = 32;
/// Header offsets. Bytes 10 and 11 extend the block/sample counters to 40 bits.
const CK_SIZE: usize = 4;
const VERSION: usize = 8;
const TOTAL_SAMPLES: usize = 12;
const BLOCK_INDEX: usize = 16;
const BLOCK_SAMPLES: usize = 20;
const FLAGS: usize = 24;
const CRC: usize = 28;

/// Shared WavPack 4/5 stream-version interval (software version 5 still uses 0x410).
const MIN_VERSION: u16 = 0x402;
const MAX_VERSION: u16 = 0x410;
const MAX_CHANNELS: u16 = 4096;

/// Samples a block may hold. The reference decoder stops at this number, and nothing
/// measured here comes near it: the largest block in these fixtures holds 2400 samples.
pub(crate) const MAX_BLOCK_SAMPLES: u32 = 150_000;

/// Decorrelation passes a block may name, and the longest tap a positive term reaches -
/// which is also how many past samples a pass holds.
const MAX_PASSES: usize = 16;
const MAX_TERM: i32 = 8;

/// The header's flag bits. Cross-decorrelation is the one bit that says something and is
/// left alone for lossless/main streams; correction reconstruction uses the bit
/// to select the predictor history used by corrected stereo samples.
const BYTES_FIELD: u32 = 0x3;
const MONO: u32 = 1 << 2;
const HYBRID: u32 = 1 << 3;
const JOINT_STEREO: u32 = 1 << 4;
const SHAPING: u32 = 1 << 6;
const FLOAT: u32 = 1 << 7;
const INT32_DATA: u32 = 1 << 8;
const HYBRID_BITRATE: u32 = 1 << 9;
const INITIAL_BLOCK: u32 = 1 << 11;
const FINAL_BLOCK: u32 = 1 << 12;
const SHIFT_FIELD: u32 = 13;
const MAG_FIELD: u32 = 18;
const RATE_FIELD: u32 = 23;
const FALSE_STEREO: u32 = 1 << 30;
const DSD_DATA: u32 = 1 << 31;

/// Bits of an item's id byte: the type, and the three markers that say its length is
/// spelled in three bytes, that its body is one byte shorter than the words it counts,
/// and that decoding does not need it.
const ID_MASK: u8 = 0x3f;
const IDF_IGNORE: u8 = 0x20;
const IDF_ODD: u8 = 0x40;
const IDF_LONG: u8 = 0x80;

/// The item types this module has a case for.
const ID_DECORR_TERMS: u8 = 0x02;
const ID_DECORR_WEIGHTS: u8 = 0x03;
const ID_DECORR_SAMPLES: u8 = 0x04;
const ID_ENTROPY_VARS: u8 = 0x05;
const ID_HYBRID_PROFILE: u8 = 0x06;
const ID_SHAPING_WEIGHTS: u8 = 0x07;
const ID_FLOAT_INFO: u8 = 0x08;
const ID_INT32_INFO: u8 = 0x09;
const ID_BITS: u8 = 0x0a;
const ID_CORRECTION: u8 = 0x0b;
const ID_EXTRA_BITS: u8 = 0x0c;
const ID_CHANNEL_INFO: u8 = 0x0d;
const ID_DSD_BITS: u8 = 0x0e;
const ID_SAMPLE_RATE: u8 = 0x27;

/// The rates the header's four-bit index names. The index one past the end of the table
/// is not a rate: it says the block states its own rate in a three-byte item, which is
/// how a stream at anything else - the 37500 Hz of one fixture here - spells itself.
const RATES: [u32; 15] = [
    6_000, 8_000, 9_600, 11_025, 12_000, 16_000, 22_050, 24_000, 32_000, 44_100, 48_000, 64_000,
    88_200, 96_000, 192_000,
];

/// The ones-count at which a residual stops being spelled in unary and carries its own
/// length instead.
const LIMIT_ONES: i32 = 16;

/// A sample held to the top of a 32-bit word is a full scale of 2^31, whichever depth the
/// block stored it at.
const FULL_SCALE: f32 = 1.0 / (1u64 << 31) as f32;
/// The same for samples a block of up to 16 bits leaves in a 16-bit range.
const SHORT_SCALE: f32 = 1.0 / (1u32 << 15) as f32;

/// `2^(n/256) - 1` at eight bits: the table the stream's magnitudes are interpolated
/// through, copied entry for entry from the reference's.
const EXP2_TABLE: [u8; 256] = [
    0, 1, 1, 2, 3, 3, 4, 5, 6, 6, 7, 8, 8, 9, 10, 11, 11, 12, 13, 14, 14, 15, 16, 16, 17, 18, 19,
    19, 20, 21, 22, 22, 23, 24, 25, 25, 26, 27, 28, 29, 29, 30, 31, 32, 32, 33, 34, 35, 36, 36, 37,
    38, 39, 40, 40, 41, 42, 43, 44, 44, 45, 46, 47, 48, 48, 49, 50, 51, 52, 53, 53, 54, 55, 56, 57,
    58, 58, 59, 60, 61, 62, 63, 64, 65, 65, 66, 67, 68, 69, 70, 71, 72, 72, 73, 74, 75, 76, 77, 78,
    79, 80, 81, 81, 82, 83, 84, 85, 86, 87, 88, 89, 90, 91, 92, 93, 94, 94, 95, 96, 97, 98, 99,
    100, 101, 102, 103, 104, 105, 106, 107, 108, 109, 110, 111, 112, 113, 114, 115, 116, 117, 118,
    119, 120, 121, 122, 123, 124, 125, 126, 127, 128, 129, 130, 131, 132, 133, 135, 136, 137, 138,
    139, 140, 141, 142, 143, 144, 145, 146, 147, 149, 150, 151, 152, 153, 154, 155, 156, 157, 159,
    160, 161, 162, 163, 164, 165, 166, 168, 169, 170, 171, 172, 173, 175, 176, 177, 178, 179, 180,
    182, 183, 184, 185, 186, 188, 189, 190, 191, 192, 194, 195, 196, 197, 198, 200, 201, 202, 203,
    205, 206, 207, 208, 210, 211, 212, 214, 215, 216, 217, 219, 220, 221, 222, 224, 225, 226, 228,
    229, 230, 232, 233, 234, 236, 237, 238, 240, 241, 242, 244, 245, 246, 248, 249, 250, 252, 253,
    255,
];

fn le16(data: &[u8], at: usize) -> i16 {
    i16::from_le_bytes(data[at..at + 2].try_into().expect("two bytes"))
}

fn le32(data: &[u8], at: usize) -> u32 {
    u32::from_le_bytes(data[at..at + 4].try_into().expect("four bytes"))
}

/// The three-byte rate a block states when its header's index says custom.
fn le24(data: &[u8]) -> u32 {
    u32::from(data[0]) | u32::from(data[1]) << 8 | u32::from(data[2]) << 16
}

/// One decorrelation pass: what it predicts from, how fast it adapts, and the state the
/// block seeded it with.
#[derive(Clone, Copy)]
struct Pass {
    term: i32,
    delta: i32,
    weight_a: i32,
    weight_b: i32,
    samples_a: [i32; 8],
    samples_b: [i32; 8],
}

/// A block's header and the metadata items it carries, with everything this decoder
/// cannot use already refused. A reader of a whole file takes the geometry and the length
/// out of it; a decoder takes the items and calls [`Block::samples`].
pub(crate) struct Block<'a> {
    /// Bytes from this block's tag to the next block's, so a caller can walk a stream.
    pub length: usize,
    /// Where the block sits in the file, in samples.
    pub block_index: u64,
    pub block_samples: u32,
    /// Samples the whole stream runs to, which is what a duration is measured against.
    pub total_samples: Option<u64>,
    pub channels: u16,
    pub bits_per_sample: u16,
    pub sample_rate: u32,
    flags: u32,
    version: u16,
    channel_info: Option<frame::ChannelInfo>,
    crc: u32,
    terms: &'a [u8],
    weights: &'a [u8],
    decorr_samples: &'a [u8],
    entropy: &'a [u8],
    hybrid_profile: &'a [u8],
    shaping: &'a [u8],
    dsd: &'a [u8],
    correction_bits: Vec<&'a [u8]>,
    int32_info: &'a [u8],
    float_info: &'a [u8],
    extra_new: bool,
    extra_bits: Vec<&'a [u8]>,
    bits: Vec<&'a [u8]>,
}

impl<'a> Block<'a> {
    /// Read the block that starts at `at` within `data`, refusing anything a decoder
    /// could only guess at.
    pub(crate) fn parse(data: &'a [u8], at: usize) -> Result<Self> {
        if at
            .checked_add(HEADER_BYTES)
            .is_none_or(|end| end > data.len())
            || data[at..at + 4] != *b"wvpk"
        {
            return Err(invalid("WavPack block tag or header missing"));
        }
        let header = &data[at..];
        let ck_size = le32(header, CK_SIZE);
        if ck_size == u32::MAX {
            return Err(invalid("WavPack block does not state its own length"));
        }
        let length = 8usize
            .checked_add(usize::try_from(ck_size).expect("below the maximum"))
            .filter(|length| *length >= HEADER_BYTES)
            .ok_or_else(|| invalid("WavPack block is shorter than its header"))?;
        let end = at
            .checked_add(length)
            .ok_or_else(|| invalid("WavPack block runs past the packet"))?;
        let bytes = data
            .get(at..end)
            .ok_or_else(|| invalid("WavPack block runs past the packet"))?;
        let version =
            u16::from_le_bytes(bytes[VERSION..VERSION + 2].try_into().expect("two bytes"));
        if !(MIN_VERSION..=MAX_VERSION).contains(&version) {
            return Err(invalid("WavPack block is not of the 4.x coding"));
        }
        let block_samples = le32(bytes, BLOCK_SAMPLES);
        if block_samples == 0 || block_samples > MAX_BLOCK_SAMPLES {
            return Err(invalid("WavPack block states an unusable sample count"));
        }
        let flags = le32(bytes, FLAGS);
        if flags & HYBRID == 0 && flags & (HYBRID_BITRATE | SHAPING | (1 << 10)) != 0 {
            return Err(invalid("WavPack hybrid controls without hybrid coding"));
        }
        if flags & FLOAT != 0 && (flags & INT32_DATA != 0 || flags & BYTES_FIELD != 3) {
            return Err(invalid(
                "WavPack float block requires 32-bit storage without INT32 coding",
            ));
        }
        if flags & DSD_DATA != 0
            && flags
                & (HYBRID
                    | FLOAT
                    | INT32_DATA
                    | JOINT_STEREO
                    | SHAPING
                    | BYTES_FIELD
                    | (31 << SHIFT_FIELD))
                != 0
        {
            return Err(invalid(
                "WavPack DSD block has incompatible PCM coding flags",
            ));
        }
        if flags & (MONO | FALSE_STEREO) == (MONO | FALSE_STEREO) {
            return Err(invalid("WavPack block signals both mono and false stereo"));
        }
        let mut block = Self {
            length,
            block_index: u64::from(le32(bytes, BLOCK_INDEX)) | (u64::from(bytes[10]) << 32),
            block_samples,
            total_samples: (le32(bytes, TOTAL_SAMPLES) != u32::MAX).then(|| {
                u64::from(le32(bytes, TOTAL_SAMPLES)) + (u64::from(bytes[11]) << 32)
                    - u64::from(bytes[11])
            }),
            channels: if flags & MONO != 0 { 1 } else { 2 },
            bits_per_sample: u16::from((flags & BYTES_FIELD) as u8 + 1) * 8,
            sample_rate: 0,
            flags,
            version,
            channel_info: None,
            crc: le32(bytes, CRC),
            terms: &[],
            weights: &[],
            decorr_samples: &[],
            entropy: &[],
            hybrid_profile: &[],
            shaping: &[],
            dsd: &[],
            correction_bits: Vec::new(),
            int32_info: &[],
            float_info: &[],
            extra_new: false,
            extra_bits: Vec::new(),
            bits: Vec::new(),
        };
        let mut custom_rate = None;
        let mut at = HEADER_BYTES;
        while at + 2 <= length {
            let id = bytes[at];
            let (word_size, item_header) = if id & IDF_LONG != 0 {
                if at + 4 > length {
                    break;
                }
                (
                    u32::from(bytes[at + 1])
                        | u32::from(bytes[at + 2]) << 8
                        | u32::from(bytes[at + 3]) << 16,
                    4,
                )
            } else {
                (u32::from(bytes[at + 1]), 2)
            };
            let words = match word_size.checked_mul(2) {
                Some(words) => words as usize,
                None => return Err(invalid("WavPack metadata item length overflow")),
            };
            if words == 0 && id & IDF_ODD != 0 {
                return Err(invalid(
                    "WavPack metadata item trims a byte it does not have",
                ));
            }
            if at + item_header + words > length {
                return Err(invalid("WavPack metadata item runs past its block"));
            }
            let body =
                &bytes[at + item_header..at + item_header + words - usize::from(id & IDF_ODD != 0)];
            match id & ID_MASK {
                ID_DECORR_TERMS => block.terms = body,
                ID_DECORR_WEIGHTS => block.weights = body,
                ID_DECORR_SAMPLES => block.decorr_samples = body,
                ID_ENTROPY_VARS => block.entropy = body,
                ID_INT32_INFO => block.int32_info = body,
                ID_BITS => block.bits.push(body),
                // The reference's word buffer is 64 KiB, so a long block's residuals do
                // come in several items, which the decoder reads as one run.
                ID_EXTRA_BITS | 0x2c => {
                    let new = id & ID_MASK == 0x2c;
                    if !block.extra_bits.is_empty() && block.extra_new != new {
                        return Err(invalid("WavPack mixed extended bitstream formats"));
                    }
                    block.extra_new = new;
                    block.extra_bits.push(body);
                }
                ID_SAMPLE_RATE => {
                    if body.len() != 3 && body.len() != 4 {
                        return Err(invalid("WavPack custom sample rate is the wrong length"));
                    }
                    // The extended form carries 31 bits; bit 31 is reserved.
                    let rate =
                        le24(body) | (body.get(3).map_or(0, |&byte| u32::from(byte & 0x7f)) << 24);
                    if rate == 0 {
                        return Err(invalid("WavPack custom sample rate is zero"));
                    }
                    custom_rate = Some(rate);
                }
                ID_HYBRID_PROFILE => block.hybrid_profile = body,
                ID_SHAPING_WEIGHTS => {
                    if flags & HYBRID == 0 {
                        return Err(invalid("WavPack shaping metadata without hybrid coding"));
                    }
                    block.shaping = body;
                }
                ID_FLOAT_INFO => block.float_info = body,
                ID_CORRECTION => {
                    if flags & HYBRID == 0 {
                        return Err(invalid("WavPack correction without hybrid coding"));
                    }
                    block.correction_bits.push(body);
                }
                ID_CHANNEL_INFO => {
                    let info = frame::ChannelInfo::read(body)?;
                    if block.channel_info.is_some_and(|old| old != info) {
                        return Err(invalid("conflicting WavPack channel information"));
                    }
                    block.channel_info = Some(info);
                }
                ID_DSD_BITS => {
                    if body.len() < 2 {
                        return Err(invalid("WavPack DSD metadata is too short"));
                    }
                    if flags & DSD_DATA == 0 {
                        return Err(invalid("WavPack DSD metadata without DSD coding"));
                    }
                    if !block.dsd.is_empty() {
                        return Err(invalid("duplicate WavPack DSD metadata"));
                    }
                    block.dsd = body;
                }
                // Anything else - the encoder's own version note, a container's padding,
                // a trailer, an MD5 - is skipped on the length just read.
                _ => {}
            }
            at += item_header + body.len() + usize::from(id & IDF_ODD != 0);
        }
        if flags & DSD_DATA != 0 {
            if block.dsd.len() < 2 {
                return Err(invalid("WavPack DSD metadata is too short"));
            }
        } else {
            if block.terms.is_empty()
                || block.weights.is_empty()
                || block.decorr_samples.is_empty()
                || block.entropy.is_empty()
                || block.bits.is_empty()
            {
                return Err(invalid("WavPack block lacks the items its samples need"));
            }
            if flags & FLOAT != 0 && block.float_info.len() != 4 {
                return Err(invalid("WavPack float information must contain four bytes"));
            }
            if flags & FLOAT == 0 && !block.float_info.is_empty() {
                return Err(invalid("WavPack float metadata without float coding"));
            }
        }
        let index = ((flags >> RATE_FIELD) & 0xf) as usize;
        block.sample_rate = if index < RATES.len() {
            RATES[index]
        } else {
            custom_rate.ok_or_else(|| invalid("WavPack custom sample rate is not stated"))?
        };
        if flags & DSD_DATA != 0 {
            block.sample_rate = dsd::rate(block.dsd, block.sample_rate)?;
        }
        Ok(block)
    }

    /// Entropy/decorrelation channel count differs from output in false stereo.
    fn coded_channels(&self) -> u16 {
        if self.flags & (MONO | FALSE_STEREO) != 0 {
            1
        } else {
            2
        }
    }

    /// Interleaved output after fixup and stereo expansion; float blocks return
    /// IEEE-754 words unchanged, while integer blocks return scaled integers.
    pub(crate) fn samples(&self) -> Result<Vec<i32>> {
        if self.flags & DSD_DATA != 0 {
            return dsd::decode(self);
        }
        let (mut buffer, offsets) = self.residuals()?;
        if !self.correction_bits.is_empty() {
            buffer = correction::reconstruct(self, &buffer, &offsets)?;
        } else {
            for mut pass in self.passes()? {
                if self.coded_channels() == 1 {
                    mono_pass(&mut pass, &mut buffer);
                } else {
                    stereo_pass(&mut pass, &mut buffer);
                }
            }
            if self.coded_channels() == 2 && self.flags & JOINT_STEREO != 0 {
                // The block stored the mid and the difference; the ear wants both channels.
                for pair in buffer.chunks_exact_mut(2) {
                    pair[1] = pair[1].wrapping_sub(pair[0] >> 1);
                    pair[0] = pair[0].wrapping_add(pair[1]);
                }
            }
        }
        self.check(&buffer)?;
        let samples = if self.flags & FLOAT != 0 {
            self.float_values(&buffer)?
        } else {
            self.widen(&buffer)?
        };
        if self.flags & FALSE_STEREO == 0 {
            return Ok(samples);
        }
        // Entropy, decorrelation, both CRCs and INT32 fixup belong to the
        // single coded channel. Duplicate only the completed PCM samples.
        let mut stereo = Vec::new();
        stereo
            .try_reserve_exact(samples.len() * 2)
            .map_err(|_| invalid("cannot allocate WavPack false-stereo output"))?;
        for sample in samples {
            stereo.extend_from_slice(&[sample, sample]);
        }
        Ok(stereo)
    }

    /// The passes the block names, with the weights and the past samples it starts them
    /// from. Both items list the passes in the reverse of the order they run in, which is
    /// the order the encoder retired them in.
    fn passes(&self) -> Result<Vec<Pass>> {
        if self.terms.len() > MAX_PASSES {
            return Err(invalid("WavPack block names too many decorrelation terms"));
        }
        let mut passes = Vec::with_capacity(self.terms.len());
        for &byte in self.terms {
            let term = i32::from(byte & 0x1f) - 5;
            let delta = i32::from((byte >> 5) & 7);
            // The holes of the range the coding does not use, and the negative terms,
            // which predict one channel off the other and so need two of them.
            if term == 0
                || term < -3
                || (term > MAX_TERM && term < 17)
                || term > 18
                || (self.coded_channels() == 1 && term < 0)
            {
                return Err(invalid("WavPack block names a reserved decorrelation term"));
            }
            passes.push(Pass {
                term,
                delta,
                weight_a: 0,
                weight_b: 0,
                samples_a: [0; 8],
                samples_b: [0; 8],
            });
        }
        passes.reverse();

        let stereo = self.coded_channels() == 2;
        let stride = usize::from(stereo) + 1;
        if self.weights.len() / stride > passes.len() {
            return Err(invalid(
                "WavPack block states more weights than it has passes",
            ));
        }
        // One signed byte per channel per pass, from the last-named pass down; the passes
        // the item stops short of start cold, as the encoder left them.
        let mut bytes = self.weights;
        for pass in passes.iter_mut().rev() {
            if bytes.len() < stride {
                break;
            }
            pass.weight_a = restore_weight(bytes[0]);
            if stereo {
                pass.weight_b = restore_weight(bytes[1]);
            }
            bytes = &bytes[stride..];
        }

        // The samples each pass predicts from, in the same order and with the same
        // partiality: the width a pass needs follows from its term.
        let mut bytes = self.decorr_samples;
        for pass in passes.iter_mut().rev() {
            let (width, taps) = if pass.term > MAX_TERM {
                (stride * 4, 2)
            } else if pass.term < 0 {
                (4, 1)
            } else {
                (stride * 2 * pass.term as usize, pass.term as usize)
            };
            if bytes.len() < width {
                break;
            }
            let (item, rest) = bytes.split_at(width);
            bytes = rest;
            for tap in 0..taps {
                // A positive term's samples go one tap per pair of channels, and a
                // negative term's are the two channels' newest sample each; only the two
                // long terms keep a pair of samples per channel, stored together.
                let (offset_a, offset_b) = if pass.term > MAX_TERM {
                    (tap * 2, 4 + tap * 2)
                } else {
                    (tap * stride * 2, tap * stride * 2 + 2)
                };
                pass.samples_a[tap] = exp2_value(i32::from(le16(item, offset_a)));
                if stereo {
                    pass.samples_b[tap] = exp2_value(i32::from(le16(item, offset_b)));
                }
            }
        }
        if !bytes.is_empty() {
            return Err(invalid(
                "WavPack block leaves decorrelated samples that nobody reads",
            ));
        }
        Ok(passes)
    }

    /// The residuals, one per sample slot, read out of the block's entropy stream.
    fn residuals(&self) -> Result<(Vec<i32>, Vec<i32>)> {
        let count = self.block_samples as usize * usize::from(self.coded_channels());
        let mut stream = Stream::new(&self.bits);
        let mut correction =
            (!self.correction_bits.is_empty()).then(|| Stream::new(&self.correction_bits));
        let mut offsets = Vec::new();
        if correction.is_some() {
            offsets
                .try_reserve_exact(count)
                .map_err(|_| invalid("cannot allocate WavPack correction offsets"))?;
        }
        let [mut med0, mut med1] = self.medians()?;
        let stereo = self.coded_channels() == 2;
        let mut hybrid = (self.flags & HYBRID != 0)
            .then(|| hybrid::Hybrid::new(self))
            .transpose()?;
        let mut holding_zero = false;
        let mut holding_one = false;
        let mut zeros_run = 0i32;
        let mut buffer = Vec::with_capacity(count);

        while buffer.len() < count {
            let slot = buffer.len();
            let current_med_idx = if stereo { slot & 1 } else { 0 };

            // Both channels' medians decide, whichever channel the slot belongs to; a mono
            // block leaves the second set at zero.
            let can_skip_zeros = zeros_are_common(&med0, &med1, holding_one);

            if can_skip_zeros && !holding_zero {
                if zeros_run != 0 {
                    zeros_run -= 1;
                    if zeros_run != 0 {
                        if let Some(h) = &mut hybrid {
                            h.zero(current_med_idx);
                        }
                        if correction.is_some() {
                            offsets.push(0);
                        }
                        buffer.push(0);
                        continue;
                    }
                } else {
                    zeros_run = read_run(&mut stream)?;
                    if zeros_run != 0 {
                        med0 = [0; 3];
                        med1 = [0; 3];
                        if let Some(h) = &mut hybrid {
                            h.zero(current_med_idx);
                        }
                        if correction.is_some() {
                            offsets.push(0);
                        }
                        buffer.push(0);
                        continue;
                    }
                }
            }

            let ones = if holding_zero {
                holding_zero = false;
                0
            } else {
                let mut ones = 0i32;
                while ones < LIMIT_ONES + 1 && stream.get_bit()? != 0 {
                    ones += 1;
                }
                if ones >= LIMIT_ONES {
                    if ones == LIMIT_ONES + 1 {
                        return Err(invalid("WavPack block ends inside a residual"));
                    }
                    ones = LIMIT_ONES + read_run(&mut stream)?;
                }
                let low = i32::from(holding_one);
                holding_one = ones & 1 != 0;
                holding_zero = ones & 1 == 0;
                (ones >> 1) + low
            };
            if current_med_idx == 0
                && let Some(h) = &mut hybrid
            {
                h.update()?;
            }

            let current_med = match current_med_idx {
                0 => &mut med0,
                _ => &mut med1,
            };

            let base_band = band(current_med, 0);
            let (low, width) = if ones == 0 {
                dec_med(current_med, 0);
                (0, base_band - 1)
            } else if ones == 1 {
                let width = band(current_med, 1) - 1;
                inc_med(current_med, 0);
                dec_med(current_med, 1);
                (base_band, width)
            } else {
                let (base, width) = if ones == 2 {
                    (base_band + band(current_med, 1), band(current_med, 2) - 1)
                } else {
                    (
                        base_band + band(current_med, 1) + (ones - 2) * band(current_med, 2),
                        band(current_med, 2) - 1,
                    )
                };
                inc_med(current_med, 0);
                inc_med(current_med, 1);
                if ones == 2 {
                    dec_med(current_med, 2);
                } else {
                    inc_med(current_med, 2);
                }
                (base, width)
            };

            let sample = if let Some(h) = &mut hybrid {
                let (value, offset) = h.value(
                    &mut stream,
                    correction.as_mut(),
                    current_med_idx,
                    low,
                    width,
                )?;
                if correction.is_some() {
                    offsets.push(offset);
                }
                value
            } else {
                let value = low + read_code(&mut stream, width)?;
                if stream.get_bit()? != 0 {
                    !value
                } else {
                    value
                }
            };
            buffer.push(sample);
        }
        if let Some(stream) = correction {
            stream.finish_word_padding()?;
        }
        Ok((buffer, offsets))
    }
    /// The three bands each channel starts its entropy decode at, in the fixed point the
    /// stream states them.
    fn medians(&self) -> Result<[[i32; 3]; 2]> {
        let stereo = self.coded_channels() == 2;
        if self.entropy.len() != usize::from(stereo) * 6 + 6 {
            return Err(invalid("WavPack entropy variables are the wrong length"));
        }
        let read = |channel: usize| {
            let start = channel * 6;
            [
                exp2_value(i32::from(le16(self.entropy, start))),
                exp2_value(i32::from(le16(self.entropy, start + 2))),
                exp2_value(i32::from(le16(self.entropy, start + 4))),
            ]
        };
        Ok([read(0), if stereo { read(1) } else { [0; 3] }])
    }

    /// The block's own checksum over the samples it just rebuilt, and the magnitude the
    /// header says no sample of it may pass. The reference treats an overrun of that
    /// magnitude as a damaged block and mutes it; this one refuses it, which is the same
    /// decision with the sample count left intact for the caller to report.
    fn check(&self, buffer: &[i32]) -> Result<()> {
        let mut crc = 0xffff_ffffu32;
        for &sample in buffer {
            crc = crc.wrapping_mul(3).wrapping_add(sample as u32);
        }
        if crc != self.crc {
            return Err(invalid("WavPack block checksum does not match its samples"));
        }
        let limit = (1i64 << ((self.flags >> MAG_FIELD) & 0x1f)) + 2;
        let limit = if self.flags & HYBRID != 0 && self.correction_bits.is_empty() {
            limit * 3 + 128
        } else {
            limit
        };
        if let Some(&sample) = buffer
            .iter()
            .find(|&&sample| i64::from(sample).abs() > limit)
        {
            return Err(invalid(&format!(
                "WavPack sample {sample} is past the magnitude the block declares"
            )));
        }
        Ok(())
    }

    /// Put the samples where they are heard: the bits a block below a word left off the
    /// bottom, and the ones a 32-bit block either sent through its extra-bits stream or
    /// simply left out.
    fn widen(&self, buffer: &[i32]) -> Result<Vec<i32>> {
        let stored = usize::from(self.bits_per_sample / 8);
        let mut shift = ((self.flags >> SHIFT_FIELD) & 0x1f) as usize;
        let hybrid = self.flags & HYBRID != 0 && self.correction_bits.is_empty();
        let mut sent = 0u32;
        let (mut run, mut and, mut or) = (0u32, 0u32, 0u32);
        let mut parts: Vec<&[u8]> = Vec::new();
        let mut expected = None;
        if self.flags & INT32_DATA != 0 {
            if !self.int32_info.is_empty() {
                if self.int32_info.len() != 4 {
                    return Err(invalid("WavPack 32-bit information is the wrong length"));
                }
                let info = self.int32_info;
                if info[0] > 30 {
                    return Err(invalid("WavPack block sends more bits than a sample holds"));
                }
                sent = u32::from(info[0] & 0x1f);
                // The three ways a block leaves bits off the bottom of a sample - zeros it
                // puts back, ones it sets, and a low bit it duplicates - of which a block
                // states one. Each one that is stated takes the shift over from the last.
                if info[1] != 0 {
                    run = u32::from(info[1]);
                }
                if info[2] != 0 {
                    (and, or) = (1, 1);
                    run = u32::from(info[2]);
                }
                if info[3] != 0 {
                    and = 1;
                    run = u32::from(info[3]);
                }
                if run > 31 {
                    return Err(invalid("WavPack block shifts its samples out of a word"));
                }
            }
            if let Some(head) = self.extra_bits.first().copied() {
                // The stream's own checksum is the first four bytes of its first item,
                // and the bits the block sent follow them.
                if head.len() <= 4 {
                    return Err(invalid(
                        "WavPack extra bits have no room for their checksum",
                    ));
                }
                expected = Some(le32(head, 0));
                parts.push(&head[4..]);
                parts.extend(self.extra_bits[1..].iter().copied());
            } else if sent != 0 {
                if !hybrid {
                    return Err(invalid("WavPack block sends bits it has no stream for"));
                }
                // The main lossy stream may omit the correction/extra low bits.
                shift += sent as usize + run as usize;
                sent = 0;
                run = 0;
            } else if hybrid && stored == 4 {
                while run != 0 && shift < 8 {
                    run -= 1;
                    shift += 1;
                }
            }
        }
        if hybrid {
            shift &= 31;
        }
        let post_shift = (if stored <= 2 { 2 } else { 4 }) * 8 - stored * 8 + shift;
        if post_shift > 31 {
            return Err(invalid("WavPack block shifts its samples out of a word"));
        }
        let mut stream = expected.map(|_| Stream::new(&parts));
        let maximum_width = if self.extra_new {
            stream
                .as_mut()
                .ok_or_else(|| invalid("missing optimized WavPack integer stream"))?
                .get_bits(5)? as u32
        } else {
            0
        };
        let mut out = Vec::with_capacity(buffer.len());
        let mut crc = 0xffff_ffffu32;
        for &sample in buffer {
            let mut value = sample as u32;
            if sent != 0 {
                let precision = 32
                    - (if sample < 0 {
                        !(sample as u32)
                    } else {
                        sample as u32
                    })
                    .leading_zeros();
                let read = if maximum_width != 0 {
                    sent.saturating_sub((precision + sent).saturating_sub(maximum_width))
                } else {
                    sent
                };
                let bits = stream
                    .as_mut()
                    .ok_or_else(|| invalid("WavPack block sends bits it has no stream for"))?
                    .get_bits(read)?;
                value = (value.wrapping_shl(read) | bits as u32).wrapping_shl(sent - read);
            }
            if run != 0 {
                let bit = (value & and) | or;
                value = value.wrapping_add(bit).wrapping_shl(run).wrapping_sub(bit);
            }
            if stream.is_some() {
                crc = crc
                    .wrapping_mul(9)
                    .wrapping_add((value & 0xffff) * 3 + (value >> 16));
            }
            if hybrid {
                let precision = stored * 8;
                let minimum = (-(1i64 << (precision - 1)) >> shift) as i32;
                let maximum = (((1i64 << (precision - 1)) - 1) >> shift) as i32;
                value = (value as i32).clamp(minimum, maximum) as u32;
            }
            out.push(value.wrapping_shl(post_shift as u32) as i32);
        }
        if let (Some(stream), Some(expected)) = (stream, expected) {
            stream.finish_word_padding()?;
            if crc != expected {
                return Err(invalid("WavPack extra bits do not match their checksum"));
            }
        }
        Ok(out)
    }

    /// Main-stream float reconstruction has no mantissa correction bits. Lossy
    /// quantization can exceed the lossless sentinel, so normalize its exponent
    /// rather than applying the strict WVX mantissa bound.
    fn float_without_extra(&self, buffer: &[i32]) -> Result<Vec<i32>> {
        let mut output = Vec::new();
        output
            .try_reserve_exact(buffer.len())
            .map_err(|_| invalid("cannot allocate WavPack float output"))?;
        let shift = u32::from(self.float_info[1] & 31);
        for &sample in buffer {
            let shifted = sample.wrapping_shl(shift);
            let sign = u32::from(shifted < 0);
            let mut magnitude = shifted.unsigned_abs();
            let mut exponent = u32::from(self.float_info[2]);
            if sample == 0 {
                exponent = 0;
            } else if magnitude >= 0x1000000 {
                while magnitude & 0x0f000000 != 0 {
                    magnitude >>= 1;
                    exponent += 1;
                }
            } else if exponent != 0 {
                let mut count = 0u32;
                while magnitude & 0x800000 == 0 {
                    exponent -= 1;
                    if exponent == 0 {
                        break;
                    }
                    magnitude = magnitude.wrapping_shl(1);
                    count += 1;
                }
                let count = count & 31;
                if count != 0 && self.float_info[0] & 1 != 0 {
                    magnitude |= (1u32 << count) - 1;
                }
            }
            output.push(((sign << 31) | ((exponent & 255) << 23) | (magnitude & 0x7fffff)) as i32);
        }
        Ok(output)
    }

    fn float_values(&self, buffer: &[i32]) -> Result<Vec<i32>> {
        let flags = self.float_info[0];
        let shift = self.float_info[1] & 31;
        let maximum = u32::from(self.float_info[2]);
        // float_norm_exp is an optional output-normalization hint. The lossless
        // API preserves original IEEE bits, including -0 and NaN payloads.
        let mut parts = Vec::new();
        let expected = if let Some(first) = self.extra_bits.first() {
            if first.len() <= 4 || first.len() % 2 != 0 {
                return Err(invalid("invalid WavPack float extended stream"));
            }
            parts.push(&first[4..]);
            parts.extend(self.extra_bits[1..].iter().copied());
            Some(le32(first, 0))
        } else {
            None
        };
        if expected.is_none() {
            return self.float_without_extra(buffer);
        }
        let mut bits = expected.map(|_| Stream::new(&parts));
        let (min_zeros, max_ones) = if self.extra_new {
            let bits = bits
                .as_mut()
                .ok_or_else(|| invalid("missing WavPack optimized float stream"))?;
            (bits.get_bits(5)? as u32, bits.get_bits(5)? as u32)
        } else {
            (0, 0)
        };
        let mut crc = 0xffff_ffffu32;
        let mut output = Vec::new();
        output
            .try_reserve_exact(buffer.len())
            .map_err(|_| invalid("cannot allocate WavPack float output"))?;
        for &sample in buffer {
            let mut exponent = maximum;
            let mut sign = u32::from(sample < 0);
            let magnitude = u64::from(sample.unsigned_abs()) << shift;
            if magnitude > 0x1000000 {
                return Err(invalid("WavPack float core exceeds mantissa range"));
            }
            let mut magnitude = magnitude as u32;
            if let Some(bits) = bits.as_mut() {
                if sample == 0 {
                    exponent = 0;
                    if flags & 8 != 0 {
                        if bits.get_bit()? != 0 {
                            magnitude = bits.get_bits(23)? as u32;
                            if maximum >= 25 {
                                exponent = bits.get_bits(8)? as u32;
                            }
                            sign = bits.get_bit()? as u32;
                        } else if flags & 16 != 0 {
                            sign = bits.get_bit()? as u32;
                        }
                    }
                } else if magnitude == 0x1000000 {
                    magnitude = if bits.get_bit()? != 0 {
                        bits.get_bits(23)? as u32
                    } else {
                        0
                    };
                    exponent = 255;
                } else {
                    let mut shifted = 0u32;
                    while exponent != 0 && magnitude & 0x800000 == 0 {
                        exponent -= 1;
                        if exponent == 0 {
                            break;
                        }
                        magnitude <<= 1;
                        shifted += 1;
                    }
                    if shifted != 0 {
                        let mask = (1u32 << shifted) - 1;
                        if flags & 1 != 0 || flags & 2 != 0 && bits.get_bit()? != 0 {
                            magnitude |= mask;
                        } else if flags & 4 != 0 {
                            let mut zeros = if max_ones != 0 {
                                shifted.saturating_sub(max_ones)
                            } else {
                                0
                            };
                            zeros = zeros.max(min_zeros.min(shifted));
                            let count = shifted - zeros;
                            if count != 0 {
                                magnitude |= ((bits.get_bits(count)? as u32) << zeros) & mask;
                            }
                        }
                    }
                }
            }
            let mantissa = magnitude & 0x7fffff;
            exponent &= 255;
            let word = (sign << 31) | (exponent << 23) | mantissa;
            crc = crc
                .wrapping_mul(27)
                .wrapping_add(mantissa * 9 + exponent * 3 + sign);
            output.push(word as i32);
        }
        if let (Some(bits), Some(expected)) = (bits, expected) {
            bits.finish_word_padding()?;
            if crc != expected {
                return Err(invalid("WavPack float CRC does not match IEEE samples"));
            }
        }
        Ok(output)
    }

    /// The block's own geometry and the samples it holds: what a container that already
    /// states the shape ignores, and what a measurement of the decoder needs.
    #[allow(dead_code)]
    pub(crate) fn measured(&self) -> Result<(u16, u16, u32, Vec<i32>)> {
        Ok((
            self.channels,
            self.bits_per_sample,
            self.sample_rate,
            self.samples()?,
        ))
    }
}

/// The width of one band of a channel's entropy state.
fn band(med: &[i32; 3], index: usize) -> i32 {
    (med[index] >> 4) + 1
}

/// A band that a residual landed at the bottom of narrows: two units of its own decay.
fn dec_med(med: &mut [i32; 3], index: usize) {
    let divisor = 128 >> index;
    med[index] -= (med[index] + divisor - 2) / divisor * 2;
}

/// And a band a residual filled widens: five units of its own growth.
fn inc_med(med: &mut [i32; 3], index: usize) {
    let divisor = 128 >> index;
    med[index] += (med[index] + divisor) / divisor * 5;
}

/// Whether the stream is at the point where a run of zeros is the common case: neither
/// channel's first band has grown past one, and no residual is owed the parity of the
/// last one.
fn zeros_are_common(first: &[i32; 3], second: &[i32; 3], holding_one: bool) -> bool {
    (first[0] as u32) < 2 && !holding_one && (second[0] as u32) < 2
}

/// A weight the stream states in one byte, put back on the scale it works on.
fn restore_weight(byte: u8) -> i32 {
    let mut weight = i32::from(byte as i8) * 8;
    if weight > 0 {
        weight += (weight + 64) >> 7;
    }
    weight
}

/// What a pass predicts: its weight times the sample it leans on, rounded down. The
/// product goes through 64 bits because a weight of a thousand on a full-scale sample
/// does not fit a word.
fn apply_weight(weight: i32, sample: i32) -> i32 {
    ((i64::from(weight) * i64::from(sample) + 512) >> 10) as i32
}

/// A pass that overshot moves its weight toward the signal by its delta, and a pass that
/// undershot moves it away.
fn update_weight(weight: i32, delta: i32, source: i32, result: i32) -> i32 {
    if source != 0 && result != 0 {
        let s = (source ^ result) >> 31;
        return (delta ^ s) + (weight - s);
    }
    weight
}

/// The same, held inside the range the negative terms need: they predict one channel off
/// the other, so an unbounded weight would run away with the pair.
fn update_weight_clip(weight: i32, delta: i32, source: i32, result: i32) -> i32 {
    if source != 0 && result != 0 {
        let s = (source ^ result) >> 31;
        let mut weight = (weight ^ s) + (delta - s);
        if weight > 1024 {
            weight = 1024;
        }
        return (weight ^ s) - s;
    }
    weight
}

/// `2^(log/256)` above one, in the fixed point the stream states its magnitudes in.
fn exp2_value(log: i32) -> i32 {
    if log < 0 {
        return -exp2_value(-log);
    }
    let value = u32::from(EXP2_TABLE[(log & 0xff) as usize]) | 0x100;
    let log = log >> 8;
    if log <= 9 {
        (value >> (9 - log)) as i32
    } else {
        value.wrapping_shl(log as u32 - 9) as i32
    }
}

/// A length spelled in unary, with the escape past 32 that carries its own bits. The
/// reference spells a run this way for both the zeros and the ones that overflow.
fn read_run(stream: &mut Stream) -> Result<i32> {
    let mut count = 0;
    while count < 33 && stream.get_bit()? != 0 {
        count += 1;
    }
    if count == 33 {
        return Err(invalid("WavPack block ends inside a run length"));
    }
    if count < 2 {
        return Ok(count);
    }
    let mut run = 0;
    let mut mask = 1;
    while count > 1 {
        count -= 1;
        if stream.get_bit()? != 0 {
            run |= mask;
        }
        mask <<= 1;
    }
    Ok(run | mask)
}

/// The residual's offset inside its band, read as the shortest code that reaches past it.
fn read_code(stream: &mut Stream, maxcode: i32) -> Result<i32> {
    if maxcode < 2 {
        return Ok(if maxcode == 1 { stream.get_bit()? } else { 0 });
    }
    let bitcount = 32 - (maxcode as u32).leading_zeros();
    let extras = (1 << bitcount) - maxcode - 1;
    let mut code = stream.get_bits(bitcount - 1)?;
    if code >= extras {
        code = (code << 1) - extras + stream.get_bit()?;
    }
    Ok(code)
}

/// Run one decorrelation pass over a mono block's residuals.
fn mono_pass(pass: &mut Pass, buffer: &mut [i32]) {
    let mut samples = pass.samples_a;
    let mut weight = pass.weight_a;
    if pass.term > MAX_TERM {
        // The two long-tapped terms predict a curve through the two newest samples
        // rather than one of them.
        for residual in buffer.iter_mut() {
            let source = if pass.term == 17 {
                samples[0].wrapping_mul(2).wrapping_sub(samples[1])
            } else {
                samples[0].wrapping_mul(3).wrapping_sub(samples[1]) >> 1
            };
            let result = apply_weight(weight, source).wrapping_add(*residual);
            weight = update_weight(weight, pass.delta, source, *residual);
            *residual = result;
            samples[1] = samples[0];
            samples[0] = result;
        }
    } else {
        // A positive term reads its own past output, one `term` samples back, so it walks
        // a ring of eight - the longest tap - and the writes trail the reads by the term.
        let mut read = 0usize;
        let mut write = pass.term as usize & 7;
        for residual in buffer.iter_mut() {
            let source = samples[read];
            let result = apply_weight(weight, source).wrapping_add(*residual);
            weight = update_weight(weight, pass.delta, source, *residual);
            *residual = result;
            samples[write] = result;
            read = (read + 1) & 7;
            write = (write + 1) & 7;
        }
        if read != 0 {
            // The ring stopped part-way, so its contents are rotated away from the order
            // the samples of the next block would be read in. Only this block's state is
            // put back where the encoder had it.
            let rotated = samples;
            for (index, slot) in samples.iter_mut().enumerate() {
                *slot = rotated[(read + index) & 7];
            }
        }
    }
    pass.weight_a = weight;
    pass.samples_a = samples;
}

/// Run one decorrelation pass over a stereo block's residuals, which interleave.
fn stereo_pass(pass: &mut Pass, buffer: &mut [i32]) {
    let mut a = pass.samples_a;
    let mut b = pass.samples_b;
    let mut wa = pass.weight_a;
    let mut wb = pass.weight_b;
    let delta = pass.delta;
    match pass.term {
        17 | 18 => {
            for pair in buffer.chunks_exact_mut(2) {
                let (sama, samb) = if pass.term == 17 {
                    (
                        a[0].wrapping_mul(2).wrapping_sub(a[1]),
                        b[0].wrapping_mul(2).wrapping_sub(b[1]),
                    )
                } else {
                    (
                        a[0].wrapping_mul(3).wrapping_sub(a[1]) >> 1,
                        b[0].wrapping_mul(3).wrapping_sub(b[1]) >> 1,
                    )
                };
                a[1] = a[0];
                b[1] = b[0];
                let (ra, rb) = (pair[0], pair[1]);
                let new_a0 = apply_weight(wa, sama).wrapping_add(ra);
                pair[0] = new_a0;
                wa = update_weight(wa, delta, sama, ra);
                let new_b0 = apply_weight(wb, samb).wrapping_add(rb);
                pair[1] = new_b0;
                wb = update_weight(wb, delta, samb, rb);
                a[0] = new_a0;
                b[0] = new_b0;
            }
        }
        1..=MAX_TERM => {
            let mut read = 0usize;
            let mut write = pass.term as usize & 7;
            for pair in buffer.chunks_exact_mut(2) {
                let (sama, samb) = (a[read], b[read]);
                let (ra, rb) = (pair[0], pair[1]);
                let new_a0 = apply_weight(wa, sama).wrapping_add(ra);
                pair[0] = new_a0;
                a[write] = new_a0;
                wa = update_weight(wa, delta, sama, ra);
                let new_b0 = apply_weight(wb, samb).wrapping_add(rb);
                pair[1] = new_b0;
                b[write] = new_b0;
                wb = update_weight(wb, delta, samb, rb);
                read = (read + 1) & 7;
                write = (write + 1) & 7;
            }
            if read != 0 {
                let (old_a, old_b) = (a, b);
                for index in 0..8 {
                    a[index] = old_a[(read + index) & 7];
                    b[index] = old_b[(read + index) & 7];
                }
            }
        }
        // The three negative terms predict one channel off the other, which is why they
        // clip: an error in one channel would otherwise feed the next one amplified.
        -1 => {
            for pair in buffer.chunks_exact_mut(2) {
                let ra = pair[0];
                let left = ra.wrapping_add(apply_weight(wa, a[0]));
                wa = update_weight_clip(wa, delta, a[0], ra);
                pair[0] = left;
                let rb = pair[1];
                let right = rb.wrapping_add(apply_weight(wb, left));
                wb = update_weight_clip(wb, delta, left, rb);
                pair[1] = right;
                a[0] = right;
            }
        }
        -2 => {
            for pair in buffer.chunks_exact_mut(2) {
                let rb = pair[1];
                let right = rb.wrapping_add(apply_weight(wb, b[0]));
                wb = update_weight_clip(wb, delta, b[0], rb);
                pair[1] = right;
                let ra = pair[0];
                let left = ra.wrapping_add(apply_weight(wa, right));
                wa = update_weight_clip(wa, delta, right, ra);
                pair[0] = left;
                b[0] = left;
            }
        }
        _ => {
            for pair in buffer.chunks_exact_mut(2) {
                let ra = pair[0];
                let la = ra.wrapping_add(apply_weight(wa, a[0]));
                wa = update_weight_clip(wa, delta, a[0], ra);
                let rb = pair[1];
                let lb = rb.wrapping_add(apply_weight(wb, b[0]));
                wb = update_weight_clip(wb, delta, b[0], rb);
                pair[0] = la;
                pair[1] = lb;
                a[0] = lb;
                b[0] = la;
            }
        }
    }
    pass.weight_a = wa;
    pass.weight_b = wb;
    pass.samples_a = a;
    pass.samples_b = b;
}

/// A block's bitstream, read from the bottom of each byte up. The residuals of a long
/// block come in several metadata items, so the reader walks a list of them and takes up
/// where one stops.
struct Stream<'a> {
    parts: &'a [&'a [u8]],
    part: usize,
    index: usize,
    bit: u32,
}

impl<'a> Stream<'a> {
    fn new(parts: &'a [&'a [u8]]) -> Self {
        Self {
            parts,
            part: 0,
            index: 0,
            bit: 0,
        }
    }

    fn get_bit(&mut self) -> Result<i32> {
        loop {
            let part = self
                .parts
                .get(self.part)
                .ok_or_else(|| invalid("WavPack block ends inside its samples"))?;
            match part.get(self.index) {
                Some(&byte) => {
                    let value = i32::from((byte >> self.bit) & 1);
                    self.bit += 1;
                    if self.bit == 8 {
                        self.bit = 0;
                        self.index += 1;
                    }
                    return Ok(value);
                }
                None => {
                    self.part += 1;
                    self.index = 0;
                }
            }
        }
    }

    /// `count` bits, held the way the stream spells them: the first one low.
    fn get_bits(&mut self, count: u32) -> Result<i32> {
        let mut value = 0;
        for index in 0..count {
            value |= self.get_bit()? << index;
        }
        Ok(value)
    }

    /// Bitstreams are closed with one bits to a 16-bit word boundary.
    fn finish_word_padding(mut self) -> Result<()> {
        let remaining = self
            .parts
            .iter()
            .skip(self.part)
            .map(|p| p.len() * 8)
            .sum::<usize>()
            .saturating_sub(self.index * 8 + self.bit as usize);
        if remaining > 15 {
            return Err(invalid("WavPack block leaves its extra bits unread"));
        }
        for _ in 0..remaining {
            if self.get_bit()? != 1 {
                return Err(invalid("WavPack extra-bit padding is not all ones"));
            }
        }
        Ok(())
    }
}

#[path = "wavpack_frame.rs"]
mod frame;

/// One complete lossless sample frame per packet; blocks own their predictor state.
pub struct WavpackDecoder {
    channels: u16,
    matroska: Option<(u16, u32)>,
    configured_mask: Option<u32>,
    known_mask: Option<u32>,
    dsd_filter: Option<dsd::Filter>,
}

impl WavpackDecoder {
    /// The channel count the container states, which every block of the stream has to
    /// agree with.
    pub fn new(channels: u16) -> Result<Self> {
        if !(1..=MAX_CHANNELS).contains(&channels) {
            return Err(invalid("WavPack channel count outside format range"));
        }
        Ok(Self {
            channels,
            matroska: None,
            configured_mask: None,
            known_mask: None,
            dsd_filter: None,
        })
    }
    /// Matroska stores the version in CodecPrivate and a 12-byte block header.
    pub fn for_matroska(private: &[u8], sample_rate: u32, channels: u16) -> Result<Self> {
        if private.len() != 2 {
            return Err(invalid("Matroska WavPack requires a two-byte version"));
        }
        let version = u16::from_le_bytes(private.try_into().unwrap());
        if !(MIN_VERSION..=MAX_VERSION).contains(&version) || sample_rate == 0 {
            return Err(invalid("invalid Matroska WavPack version or sample rate"));
        }
        let mut decoder = Self::new(channels)?;
        decoder.matroska = Some((version, sample_rate));
        Ok(decoder)
    }
    /// Bind the container's immutable declared speaker layout across resets.
    pub fn with_speaker_mask(mut self, mask: Option<u32>) -> Result<Self> {
        if mask.is_some_and(|m| m.count_ones() > u32::from(self.channels)) {
            return Err(invalid("WavPack speaker mask exceeds channel count"));
        }
        self.configured_mask = mask;
        self.known_mask = mask;
        Ok(self)
    }

    /// Inspect the first coded frame's declared standard speaker positions.
    /// No PCM is synthesized, and all component headers/layout are validated.
    pub fn matroska_speaker_mask(
        private: &[u8],
        data: &[u8],
        rate: u32,
        channels: u16,
    ) -> Result<Option<u32>> {
        let decoder = Self::for_matroska(private, rate, channels)?;
        let raw = frame::restore_matroska(data, decoder.matroska.unwrap().0, channels)?;
        let parsed = frame::Frame::parse(&raw, channels)?;
        if parsed.blocks[0].sample_rate != rate {
            return Err(invalid("WavPack block sample rate disagrees with Matroska"));
        }
        Ok(parsed.mask)
    }
}

impl WavpackDecoder {
    /// Reconstruct a hybrid frame using its corresponding full-header correction frame.
    pub fn decode_with_correction(
        &mut self,
        main: &[u8],
        extra: &[u8],
        pts: u64,
    ) -> Result<Option<AudioPacket>> {
        let joined = if let Some((version, _)) = self.matroska {
            correction::matroska(main, extra, version, self.channels)?
        } else {
            correction::join(main, extra, self.channels)?
        };
        self.decode_encoded(&joined, pts, 0)
    }
    /// Adapt standard Matroska BlockAdditional correction metadata for the decoder.
    pub fn matroska_with_correction(
        private: &[u8],
        rate: u32,
        channels: u16,
        main: &[u8],
        extra: &[u8],
    ) -> Result<Vec<u8>> {
        let d = Self::for_matroska(private, rate, channels)?;
        correction::matroska(main, extra, d.matroska.unwrap().0, channels)
    }
}

impl WavpackDecoder {
    /// Recover canonical MSB-first DSD bytes without PCM filtering.
    /// The returned rate counts byte frames, eight DSD bits per channel/frame.
    pub fn decode_dsd(&self, data: &[u8]) -> Result<(Vec<u8>, u32)> {
        let rebuilt = self
            .matroska
            .map(|(version, _)| frame::restore_matroska(data, version, self.channels))
            .transpose()?;
        let frame = frame::Frame::parse(rebuilt.as_deref().unwrap_or(data), self.channels)?;
        let rate = frame.blocks[0].sample_rate;
        if self.matroska.is_some_and(|(_, expected)| expected != rate) {
            return Err(invalid("WavPack DSD rate disagrees with Matroska"));
        }
        Ok((dsd::bytes(&frame, self.channels)?, rate))
    }
    pub fn matroska_is_dsd(private: &[u8], data: &[u8], rate: u32, channels: u16) -> Result<bool> {
        let d = Self::for_matroska(private, rate, channels)?;
        let raw = frame::restore_matroska(data, d.matroska.unwrap().0, channels)?;
        let frame = frame::Frame::parse(&raw, channels)?;
        Ok(frame.blocks[0].flags & DSD_DATA != 0)
    }
}

impl AudioDecode for WavpackDecoder {
    /// Decode every component before publishing a complete interleaved frame.
    fn decode_encoded(
        &mut self,
        data: &[u8],
        pts: u64,
        _duration: u64,
    ) -> Result<Option<AudioPacket>> {
        let rebuilt = self
            .matroska
            .map(|(version, _)| frame::restore_matroska(data, version, self.channels))
            .transpose()?;
        let source = rebuilt.as_deref().unwrap_or(data);
        let assembled = frame::Frame::parse(source, self.channels)?;
        if self
            .known_mask
            .zip(assembled.mask)
            .is_some_and(|(a, b)| a != b)
        {
            return Err(invalid("WavPack speaker layout changed inside stream"));
        }
        let first = &assembled.blocks[0];
        let rate = first.sample_rate;
        if self.matroska.is_some_and(|(_, expected)| expected != rate) {
            return Err(invalid("WavPack block sample rate disagrees with Matroska"));
        }
        if first.flags & DSD_DATA != 0 {
            let bytes = dsd::bytes(&assembled, self.channels)?;
            let mut filter = match &self.dsd_filter {
                Some(filter) => filter.duplicate()?,
                None => dsd::Filter::new(self.channels)?,
            };
            let pcm = filter.convert(&bytes)?;
            let mut data = Vec::new();
            data.try_reserve_exact(
                pcm.len()
                    .checked_mul(4)
                    .ok_or_else(|| invalid("DSD PCM length overflow"))?,
            )
            .map_err(|_| invalid("cannot allocate DSD PCM packet"))?;
            for value in pcm {
                data.extend_from_slice(&value.to_le_bytes());
            }
            self.dsd_filter = Some(filter);
            if self.known_mask.is_none() {
                self.known_mask = assembled.mask;
            }
            return Ok(Some(AudioPacket {
                data,
                pts,
                timebase_num: 1,
                timebase_den: rate,
            }));
        }
        let count = (first.block_samples as usize)
            .checked_mul(usize::from(self.channels))
            .and_then(|n| n.checked_mul(4))
            .ok_or_else(|| invalid("WavPack output size overflow"))?;
        let mut out = Vec::new();
        out.try_reserve_exact(count)
            .map_err(|_| invalid("cannot allocate WavPack multichannel output"))?;
        out.resize(count, 0);
        let mut offset = 0;
        for block in &assembled.blocks {
            let samples = block.samples()?;
            let scale = if block.bits_per_sample <= 16 {
                SHORT_SCALE
            } else {
                FULL_SCALE
            };
            for (row, values) in samples
                .chunks_exact(usize::from(block.channels))
                .enumerate()
            {
                for (channel, &sample) in values.iter().enumerate() {
                    let at = (row * usize::from(self.channels) + offset + channel) * 4;
                    let bytes = if block.flags & FLOAT != 0 {
                        sample.to_le_bytes()
                    } else {
                        ((sample as f32) * scale).to_le_bytes()
                    };
                    out[at..at + 4].copy_from_slice(&bytes);
                }
            }
            offset += usize::from(block.channels);
        }
        if self.known_mask.is_none() {
            self.known_mask = assembled.mask;
        }
        Ok(Some(AudioPacket {
            data: out,
            pts,
            timebase_num: 1,
            timebase_den: rate,
        }))
    }

    /// Predictor blocks are independent; DSD PCM conversion also resets its FIR history.
    fn reset(&mut self) {
        self.known_mask = self.configured_mask;
        self.dsd_filter = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The `data` chunk of a RIFF/WAVE file as 16-bit samples.
    fn wav_samples(wav: &[u8]) -> Vec<i16> {
        assert_eq!(&wav[0..4], b"RIFF");
        let mut at = 12;
        while at + 8 <= wav.len() {
            let len = u32::from_le_bytes(wav[at + 4..at + 8].try_into().unwrap()) as usize;
            if &wav[at..at + 4] == b"data" {
                return wav[at + 8..at + 8 + len]
                    .chunks_exact(2)
                    .map(|b| i16::from_le_bytes([b[0], b[1]]))
                    .collect();
            }
            at += 8 + len + (len & 1);
        }
        panic!("no data chunk");
    }

    #[test]
    fn a_stereo_16_bit_file_decodes_to_ffmpegs_samples() {
        let wv = std::fs::read("tests/fixtures/wavpack/out.wv").expect("read out.wv");
        let wav = std::fs::read("tests/fixtures/wavpack/ff_out.wav").expect("read ff_out.wav");
        let reference = wav_samples(&wav);

        let mut decoder = WavpackDecoder::new(2).expect("stereo decoder");
        let mut decoded = Vec::new();
        let mut at = 0;
        // The file ends in an APE tag after its last block.
        while at + 8 <= wv.len() && &wv[at..at + 4] == b"wvpk" {
            let size = u32::from_le_bytes(wv[at + 4..at + 8].try_into().unwrap()) as usize + 8;
            let packet = decoder
                .decode_encoded(&wv[at..at + size], 0, 0)
                .expect("block decodes")
                .expect("block yields a packet");
            decoded.extend(
                packet
                    .data
                    .chunks_exact(4)
                    .map(|b| f32::from_le_bytes(b.try_into().unwrap())),
            );
            at += size;
        }

        assert_eq!(decoded.len(), reference.len());
        for (index, (&got, &want)) in decoded.iter().zip(&reference).enumerate() {
            assert_eq!(got, f32::from(want) / 32768.0, "sample {index}");
        }
    }
}

#[path = "wavpack_hybrid.rs"]
mod hybrid;

#[path = "wavpack_correction.rs"]
mod correction;

#[path = "wavpack_dsd.rs"]
mod dsd;

/// Coded geometry of one complete native sample frame, without synthesizing PCM.
#[derive(Clone, Copy, Debug)]
pub struct FrameInfo {
    pub channels: u16,
    pub sample_rate: u32,
    pub bits_per_sample: u16,
    pub samples: u32,
    pub block_index: u64,
    pub total_samples: Option<u64>,
    pub speaker_mask: Option<u32>,
    pub dsd: bool,
    pub float: bool,
}
impl WavpackDecoder {
    pub fn inspect_frame(data: &[u8], channels: u16) -> Result<FrameInfo> {
        Self::new(channels)?;
        let f = frame::Frame::parse(data, channels)?;
        let b = &f.blocks[0];
        Ok(FrameInfo {
            channels,
            sample_rate: b.sample_rate,
            bits_per_sample: if b.flags & DSD_DATA != 0 {
                1
            } else {
                b.bits_per_sample
            },
            samples: b.block_samples,
            block_index: b.block_index,
            total_samples: b.total_samples,
            speaker_mask: f.mask,
            dsd: b.flags & DSD_DATA != 0,
            float: b.flags & FLOAT != 0,
        })
    }
    /// Adapt paired native components for the ordinary packet decoder.
    pub fn native_with_correction(main: &[u8], extra: &[u8], channels: u16) -> Result<Vec<u8>> {
        correction::join(main, extra, channels)
    }
}
