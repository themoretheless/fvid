//! DSP Group TrueSpeech: one 32-byte block to 30 ms of telephone speech, 240 samples
//! reconstructed per block and nothing in the block that says so.
//!
//! The coding is the 1990s' answer to a modem's worth of bandwidth - 8 kbit/s at 8 kHz,
//! one channel - and it is a code-excited linear predictor: a block spends most of its
//! 256 bits on an eight-tap filter rebuilt from eight codebook values, and the rest on
//! seven impulses per 60-sample subframe that the filter then rings against. Three things
//! make it the largest of this crate's Wave decoders to port, and each is a place where a
//! transcription from the reference goes silently wrong.
//!
//! First the bit order. The reference byte-swaps each of the block's four-byte words
//! before reading it, so a block is not one big-endian bit run but eight of them: the
//! first field takes its bits from byte 3 of the block downward, and the eighth word's
//! bits are the block's last field. A decoder that reads `data` MSB-first from byte 0
//! reconstructs a filter from the wrong codes and still returns 240 samples per block, so
//! nothing but a sample-for-sample comparison shows the error.
//!
//! Second the widths. The reference keeps its filters, its correlated vector and its
//! output in `int16_t` while computing in `int`, which means several of its stores
//! truncate rather than clamp - the blended filter pair in particular can reach 32 783
//! before the assignment wraps it. Every such store is a `as i16` here and not a `clamp`,
//! and the difference is a sample that is 65 536 away from its reference value.
//!
//! Third the two-point filter, which reads the history it is writing: with the smallest
//! offset the code can state, its tenth output feeds its own next output. That is not a
//! mistake to fix but a recurrence to reproduce, so the port keeps the reference's single
//! 206-entry scratch array and its indices rather than tidying it into a plain convolution.
//!
//! What the container adds is measured rather than assumed. Over this build's reference
//! and the shipped take, the same 46 144 payload bytes give the same 346 080 samples with
//! `wBlockAlign` 16 and 64, with `dwAvgBytesPerSec` 1 066 and 8 000, with
//! `wBitsPerSample` 0, 1 and 16, and with a `fact` chunk stating 4 frames as well as one
//! stating 346 080 - so the reader takes its geometry from the format number alone. The
//! one header field with an effect is the channel count, because the decoder is built for
//! one predictor and refuses any other layout. The block itself is not a claim the header
//! can make: a stated alignment of 33 does change what the reference hands its decoder,
//! and changes it into truncating the run, which is a demuxer's accident rather than a
//! geometry to read.

use crate::audio::{AudioDecode, AudioPacket, AudioSpec, SampleFormat};
use crate::{Result, invalid};

/// Bytes one block codes, which is the reference's only input geometry: it decodes
/// `buf_size / 32` blocks and leaves a shorter tail.
pub const BLOCK_BYTES: usize = 32;

/// Samples one subframe reconstructs.
const SUBFRAME: usize = 60;

/// Samples one block codes: four subframes, which is 30 ms at the 8 kHz the coding was
/// built for and what a Wave file's run divides by to count its frames.
pub const SAMPLES_PER_BLOCK: usize = 4 * SUBFRAME;

/// How far back the two-point filter reads the filtered run: the 146 samples the reference
/// keeps between blocks, which is two subframes and a bit more than one.
const FILTER_HISTORY: usize = 146;

/// The largest magnitude the synthesis writes out, one short of a sample's ends - the
/// reference clips to it on both sides so no product of its next stage can overflow.
const SYNTH_LIMIT: i32 = 0x7FFE;

/// The tables below are the reference's own `int16_t` arrays, spelled as it spells them
/// and so held as `u16`: the hex literals are its bit patterns, more than half of which do
/// not fit a signed 16-bit literal. A read casts the pattern back with `cast_signed`, which
/// is what the C store already does to the same bits.
///
/// The eight codebooks of the input vector, in the order a block states them. Each is a
/// monotone run through a Q15 range, which is why the low ones are mostly negative: the
/// vector holds reflection coefficients, and a codebook's top entries are the ones that
/// read as positive.
const CB_0: [u16; 32] = [
    0x8240, 0x8364, 0x84CE, 0x865D, 0x8805, 0x89DE, 0x8BD7, 0x8DF4, 0x9051, 0x92E2, 0x95DE, 0x990F,
    0x9C81, 0xA079, 0xA54C, 0xAAD2, 0xB18A, 0xB90A, 0xC124, 0xC9CC, 0xD339, 0xDDD3, 0xE9D6, 0xF893,
    0x096F, 0x1ACA, 0x29EC, 0x381F, 0x45F9, 0x546A, 0x63C3, 0x73B5,
];

const CB_1: [u16; 32] = [
    0x9F65, 0xB56B, 0xC583, 0xD371, 0xE018, 0xEBB4, 0xF61C, 0xFF59, 0x085B, 0x1106, 0x1952, 0x214A,
    0x28C9, 0x2FF8, 0x36E6, 0x3D92, 0x43DF, 0x49BB, 0x4F46, 0x5467, 0x5930, 0x5DA3, 0x61EC, 0x65F9,
    0x69D4, 0x6D5A, 0x709E, 0x73AD, 0x766B, 0x78F0, 0x7B5A, 0x7DA5,
];

const CB_2: [u16; 16] = [
    0x96F8, 0xA3B4, 0xAF45, 0xBA53, 0xC4B1, 0xCECC, 0xD86F, 0xE21E, 0xEBF3, 0xF640, 0x00F7, 0x0C20,
    0x1881, 0x269A, 0x376B, 0x4D60,
];

const CB_3: [u16; 16] = [
    0xC654, 0xDEF2, 0xEFAA, 0xFD94, 0x096A, 0x143F, 0x1E7B, 0x282C, 0x3176, 0x3A89, 0x439F, 0x4CA2,
    0x557F, 0x5E50, 0x6718, 0x6F8D,
];

const CB_4: [u16; 16] = [
    0xABE7, 0xBBA8, 0xC81C, 0xD326, 0xDD0E, 0xE5D4, 0xEE22, 0xF618, 0xFE28, 0x064F, 0x0EB7, 0x17B8,
    0x21AA, 0x2D8B, 0x3BA2, 0x4DF9,
];

const CB_5: [u16; 8] = [
    0xD51B, 0xF12E, 0x042E, 0x13C7, 0x2260, 0x311B, 0x40DE, 0x5385,
];

const CB_6: [u16; 8] = [
    0xB550, 0xC825, 0xD980, 0xE997, 0xF883, 0x0752, 0x1811, 0x2E18,
];

const CB_7: [u16; 8] = [
    0xCEF0, 0xE4F9, 0xF6BB, 0x0646, 0x14F5, 0x23FF, 0x356F, 0x4A8D,
];

/// The codebooks in the order a block reads them, which is the reverse of the order its
/// vector is used in: the reference spends its first three bits on the last coefficient.
const CODEBOOKS: [&[u16]; 8] = [&CB_7, &CB_6, &CB_5, &CB_4, &CB_3, &CB_2, &CB_1, &CB_0];

/// How wide each of those eight codes is, so the eight fill the block's first 32 bits.
const CODE_WIDTHS: [u32; 8] = [3, 3, 3, 4, 4, 4, 5, 5];

/// The pulse-position code, as four rows of 30 entries. A subframe's 27-bit field is
/// walked twice: its high 15 bits place three of the seven pulses in the subframe's first
/// half, starting at row 1, and its low 15 bits place the other four in its second half,
/// starting at row 0. Each row's entries fall toward zero, so a walk spends its coefficient
/// on the earliest position it can still afford and drops to the next row when it cannot.
const PULSE_VALUES: [u16; 120] = [
    0x0E46, 0x0CCC, 0x0B6D, 0x0A28, 0x08FC, 0x07E8, 0x06EB, 0x0604, 0x0532, 0x0474, 0x03C9, 0x0330,
    0x02A8, 0x0230, 0x01C7, 0x016C, 0x011E, 0x00DC, 0x00A5, 0x0078, 0x0054, 0x0038, 0x0023, 0x0014,
    0x000A, 0x0004, 0x0001, 0x0000, 0x0000, 0x0000, 0x0196, 0x017A, 0x015F, 0x0145, 0x012C, 0x0114,
    0x00FD, 0x00E7, 0x00D2, 0x00BE, 0x00AB, 0x0099, 0x0088, 0x0078, 0x0069, 0x005B, 0x004E, 0x0042,
    0x0037, 0x002D, 0x0024, 0x001C, 0x0015, 0x000F, 0x000A, 0x0006, 0x0003, 0x0001, 0x0000, 0x0000,
    0x001D, 0x001C, 0x001B, 0x001A, 0x0019, 0x0018, 0x0017, 0x0016, 0x0015, 0x0014, 0x0013, 0x0012,
    0x0011, 0x0010, 0x000F, 0x000E, 0x000D, 0x000C, 0x000B, 0x000A, 0x0009, 0x0008, 0x0007, 0x0006,
    0x0005, 0x0004, 0x0003, 0x0002, 0x0001, 0x0000, 0x0001, 0x0001, 0x0001, 0x0001, 0x0001, 0x0001,
    0x0001, 0x0001, 0x0001, 0x0001, 0x0001, 0x0001, 0x0001, 0x0001, 0x0001, 0x0001, 0x0001, 0x0001,
    0x0001, 0x0001, 0x0001, 0x0001, 0x0001, 0x0001, 0x0001, 0x0001, 0x0001, 0x0001, 0x0001, 0x0001,
];

/// The decay applied to the correlated vector, one factor per coefficient, all just under
/// a half: this is what keeps a block's filter stable against the next block's.
const DECAY_994_1000: [u16; 8] = [
    0x7F3B, 0x7E78, 0x7DB6, 0x7CF5, 0x7C35, 0x7B76, 0x7AB8, 0x79FC,
];

/// The 25 two-point filters, each a pair of taps. A subframe's offset code picks one of
/// these and one of the copy positions below, and the pair is what makes the stage a
/// two-point filter rather than a longer one.
const ORDER2_COEFFS: [[u16; 2]; 25] = [
    [0xED2F, 0x5239],
    [0x54F1, 0xE4A9],
    [0x2620, 0xEE3E],
    [0x09D6, 0x2C40],
    [0xEFB5, 0x2BE0],
    [0x3FE1, 0x3339],
    [0x442F, 0xE6FE],
    [0x4458, 0xF9DF],
    [0xF231, 0x43DB],
    [0x3DB0, 0xF705],
    [0x4F7B, 0xFEFB],
    [0x26AD, 0x0CDC],
    [0x33C2, 0x0739],
    [0x12BE, 0x43A2],
    [0x1BDF, 0x1F3E],
    [0x0211, 0x0796],
    [0x2AEB, 0x163F],
    [0x050D, 0x3A38],
    [0x0D1E, 0x0D78],
    [0x150F, 0x3346],
    [0x38A4, 0x0B7D],
    [0x2D5D, 0x1FDF],
    [0x19B7, 0x2822],
    [0x0D99, 0x1F12],
    [0x194C, 0x0CE6],
];

/// The 16 sets of four pulse magnitudes a subframe's 4-bit offset chooses between: one row
/// per set, and within a row the four 2-bit values a pulse code can take. The sets run from
/// a pair of units to half a sample's range, which is how 4 bits carry the excitation's
/// loudness.
const PULSE_SCALES: [[u16; 4]; 16] = [
    [0x0002, 0x0006, 0xFFFE, 0xFFFA],
    [0x0004, 0x000C, 0xFFFC, 0xFFF4],
    [0x0006, 0x0012, 0xFFFA, 0xFFEE],
    [0x000A, 0x001E, 0xFFF6, 0xFFE2],
    [0x0010, 0x0030, 0xFFF0, 0xFFD0],
    [0x0019, 0x004B, 0xFFE7, 0xFFB5],
    [0x0028, 0x0078, 0xFFD8, 0xFF88],
    [0x0040, 0x00C0, 0xFFC0, 0xFF40],
    [0x0065, 0x012F, 0xFF9B, 0xFED1],
    [0x00A1, 0x01E3, 0xFF5F, 0xFE1D],
    [0x0100, 0x0300, 0xFF00, 0xFD00],
    [0x0196, 0x04C2, 0xFE6A, 0xFB3E],
    [0x0285, 0x078F, 0xFD7B, 0xF871],
    [0x0400, 0x0C00, 0xFC00, 0xF400],
    [0x0659, 0x130B, 0xF9A7, 0xECF5],
    [0x0A14, 0x1E3C, 0xF5EC, 0xE1C4],
];

/// The first of the synthesis stage's two extra poles, which flattens the excitation into
/// a formant; the second is the long-term one the pitch gain pulls back in.
const DECAY_35_64: [u16; 8] = [
    0x4666, 0x26B8, 0x154C, 0x0BB6, 0x0671, 0x038B, 0x01F3, 0x0112,
];

const DECAY_3_4: [u16; 8] = [
    0x6000, 0x4800, 0x3600, 0x2880, 0x1E60, 0x16C8, 0x1116, 0x0CD1,
];

/// A block's fields, read out of its 256 bits in the order the reference reads them.
struct Frame {
    /// The eight codebook values of the input vector, held in the order the synthesis uses
    /// them - which is the opposite of the order the bits state them in.
    vector: [i16; 8],
    /// The two copy offsets, 8 bits each, split across the block in four bites.
    offset1: [i32; 2],
    /// The four 7-bit codes that choose both where a subframe copies from and which of the
    /// 25 two-point filters it copies through. 127 is the one value that means neither.
    offset2: [i32; 4],
    /// Which set of four pulse magnitudes each subframe's pulses come from.
    pulseoff: [i32; 4],
    /// Each subframe's 27-bit position field.
    pulsepos: [i32; 4],
    /// Each subframe's seven 2-bit pulse selectors, as one 14-bit field.
    pulseval: [i32; 4],
    /// Whether this block's first two subframes ring against their own filter, the
    /// previous block's, or a blend of the two.
    flag: i32,
}

/// A block read bit by bit, in the reference's own order. It byte-swaps each of the block's
/// four-byte words before reading any of them, so the bits come out of a word starting at
/// the byte that word's little-endian value puts at the top: the block's first field takes
/// its bits from byte 3 downward, not byte 0.
struct Bits {
    words: [u32; 8],
    at: usize,
}

impl Bits {
    fn new(block: &[u8]) -> Self {
        let mut words = [0u32; 8];
        for (index, word) in words.iter_mut().enumerate() {
            let four = &block[index * 4..index * 4 + 4];
            *word = u32::from_le_bytes(four.try_into().expect("four bytes"));
        }
        Self { words, at: 0 }
    }

    /// The next `width` bits, most significant first. No field is wider than 27 bits, so a
    /// read crosses at most one word boundary; taking the bits one at a time is how the
    /// reference's own `get_bits` behaves and costs nothing beside the synthesis.
    fn take(&mut self, width: u32) -> u32 {
        let mut value = 0u32;
        for _ in 0..width {
            let word = self.at / 32;
            let bit = 31 - self.at % 32;
            value = (value << 1) | (self.words[word] >> bit) & 1;
            self.at += 1;
        }
        value
    }

    /// All 256 bits of a block, in the order the reference spends them. Three of its fields
    /// are not laid out as they are read: the eight codebook values come out back to front,
    /// the four copy-offset codes are stated in the order the subframes do not run in, and
    /// the low four bits of the first offset are scattered one to each quartet of pulse
    /// fields. A decoder that reads the run in the order the numbers arrive has both its
    /// filter and its copy positions wrong, and still gets 240 samples per block.
    fn frame(&mut self) -> Frame {
        let mut vector = [0i16; 8];
        for (index, (codebook, width)) in CODEBOOKS.iter().zip(CODE_WIDTHS).enumerate() {
            vector[7 - index] = codebook[self.take(width) as usize].cast_signed();
        }
        let flag = self.take(1) as i32;
        let mut offset1 = [(self.take(4) as i32) << 4, 0];
        let mut offset2 = [0i32; 4];
        for quart in (0..4).rev() {
            offset2[quart] = self.take(7) as i32;
        }
        let mut pulseval = [0i32; 4];
        offset1[1] = self.take(4) as i32;
        pulseval[1] = self.take(14) as i32;
        pulseval[0] = self.take(14) as i32;
        offset1[1] |= (self.take(4) as i32) << 4;
        pulseval[3] = self.take(14) as i32;
        pulseval[2] = self.take(14) as i32;
        let mut pulsepos = [0i32; 4];
        let mut pulseoff = [0i32; 4];
        for quart in 0..4 {
            offset1[0] |= (self.take(1) as i32) << quart;
            pulsepos[quart] = self.take(27) as i32;
            pulseoff[quart] = self.take(4) as i32;
        }
        Frame {
            vector,
            offset1,
            offset2,
            pulseoff,
            pulsepos,
            pulseval,
            flag,
        }
    }
}

/// One filter set decayed by one of the two tables, in Q15.
fn decayed(taps: &[i16], decay: [u16; 8]) -> [i32; 8] {
    let mut out = [0i32; 8];
    for i in 0..8 {
        out[i] = (i32::from(decay[i].cast_signed()) * i32::from(taps[i])) >> 15;
    }
    out
}

/// TrueSpeech, one channel: 32 bytes in, 240 samples out.
pub struct TrueSpeechDecoder {
    sample_rate: u32,
    /// The previous block's correlated vector, which its first two subframes ring against.
    prevfilt: [i16; 8],
    /// The filtered run, 146 samples of it, that every subframe's two-point filter copies
    /// from and every subframe then lengthens.
    filtbuf: [i32; FILTER_HISTORY],
    /// This block's correlated vector, kept so the end of the block can save it as the next
    /// one's predecessor.
    cvector: [i16; 8],
    /// The first codebook value of the block, which the last synthesis stage feeds back
    /// through a quarter of itself.
    filtval: i32,
    /// The three synthesis delay lines. They are state between subframes and blocks in the
    /// reference too, and each is fully rewritten within one subframe, so only the first
    /// subframe of a run reads their cold zeros.
    tmp1: [i16; 8],
    tmp2: [i16; 8],
    tmp3: [i16; 8],
}

impl TrueSpeechDecoder {
    /// Build a decoder for a track. `sample_rate` is the track's own: the reconstruction
    /// does not read it, and only how fast the player plays the 240 samples back is its
    /// business - as with GSM, a file at any other rate still decodes the same numbers.
    pub fn new(sample_rate: u32, channels: u16) -> Result<Self> {
        if channels != 1 {
            return Err(invalid(&format!(
                "TrueSpeech codes one channel, this track names {channels}"
            )));
        }
        if sample_rate == 0 {
            return Err(invalid("TrueSpeech track has no sample rate"));
        }
        Ok(Self::cold(sample_rate))
    }

    fn cold(sample_rate: u32) -> Self {
        Self {
            sample_rate,
            prevfilt: [0; 8],
            filtbuf: [0; FILTER_HISTORY],
            cvector: [0; 8],
            filtval: 0,
            tmp1: [0; 8],
            tmp2: [0; 8],
            tmp3: [0; 8],
        }
    }

    pub fn spec(&self) -> AudioSpec {
        AudioSpec {
            sample_rate: self.sample_rate,
            channels: 1,
            format: SampleFormat::F32,
        }
    }

    /// The block's eight codes into the correlated input vector: each coefficient is built
    /// from the ones before it in its own block, then every one is decayed. This is the
    /// stage whose stores truncate, because the running sum of a coefficient can pass a
    /// sample's end before the reference writes it back into its `int16_t`.
    fn correlate_filter(&mut self, vector: &[i16; 8]) {
        let mut tmp = [0i16; 8];
        for i in 0..8 {
            if i > 0 {
                tmp[..i].copy_from_slice(&self.cvector[..i]);
                for j in 0..i {
                    let add = (i32::from(tmp[i - j - 1]) * i32::from(vector[i]) + 0x4000) >> 15;
                    self.cvector[j] = (i32::from(self.cvector[j]) + add) as i16;
                }
            }
            self.cvector[i] = ((8 - i32::from(vector[i])) >> 3) as i16;
        }
        for i in 0..8 {
            let decayed = i32::from(self.cvector[i]) * i32::from(DECAY_994_1000[i].cast_signed());
            self.cvector[i] = (decayed >> 15) as i16;
        }
        self.filtval = i32::from(vector[0]);
    }

    /// The block's four filter sets: two subframes' worth from the previous block, one from
    /// this one, and the last shared with it. The blend's two weights are two thirds and a
    /// third, and the store wraps rather than clamping.
    fn merge_filters(&self, flag: i32) -> [i16; 32] {
        let mut filters = [0i16; 32];
        for i in 0..8 {
            let previous = i32::from(self.prevfilt[i]);
            let current = i32::from(self.cvector[i]);
            if flag == 0 {
                filters[i] = previous as i16;
                filters[i + 8] = previous as i16;
            } else {
                filters[i] = ((current * 21846 + previous * 10923 + 16384) >> 15) as i16;
                filters[i + 8] = ((current * 10923 + previous * 21846 + 16384) >> 15) as i16;
            }
            filters[i + 16] = self.cvector[i];
            filters[i + 24] = self.cvector[i];
        }
        filters
    }

    /// A subframe's copy of the filtered run through its two-point filter. The offset code
    /// says both where in the 146-sample history to start and which pair of taps to use, so
    /// the same seven bits choose the delay and the shape.
    fn apply_two_point_filter(&self, frame: &Frame, quart: usize) -> [i16; SUBFRAME] {
        let mut newvec = [0i16; SUBFRAME];
        let code = frame.offset2[quart];
        if code == 127 {
            return newvec;
        }
        // One array for the history and the outputs, because with the least offset the
        // filter reads what it wrote two samples ago - the reference's own overlap, which
        // a split array would quietly drop.
        let mut tmp = [0i16; FILTER_HISTORY + SUBFRAME];
        for i in 0..FILTER_HISTORY {
            tmp[i] = self.filtbuf[i] as i16;
        }
        let off = ((code / 25) + frame.offset1[quart / 2] + 18).clamp(0, FILTER_HISTORY as i32 - 1);
        let taps = ORDER2_COEFFS[(code % 25) as usize].map(u16::cast_signed);
        let start = FILTER_HISTORY - 1 - off as usize;
        for i in 0..SUBFRAME {
            let a = i32::from(tmp[start + i]);
            let b = i32::from(tmp[start + i + 1]);
            let value = ((a * i32::from(taps[0]) + b * i32::from(taps[1]) + 0x2000) >> 14) as i16;
            newvec[i] = value;
            tmp[FILTER_HISTORY + i] = value;
        }
        newvec
    }

    /// A subframe's seven pulses: their magnitudes from its offset's set in the scale table,
    /// their positions walked out of the two halves of its 27-bit field. The field's high
    /// half pays for three pulses in the first 30 samples and its low half for four in the
    /// second, and a half whose coefficient the rows cannot spend leaves the rest of its
    /// samples silent - which is what a row's trailing zeros are for.
    fn place_pulses(&self, frame: &Frame, quart: usize, out: &mut [i16; SUBFRAME]) {
        let mut tmp = [0i16; 7];
        let mut value = frame.pulseval[quart];
        for i in 0..7 {
            let selector = (value & 3) as usize;
            value >>= 2;
            tmp[6 - i] = PULSE_SCALES[frame.pulseoff[quart] as usize][selector].cast_signed();
        }
        let position = frame.pulsepos[quart];
        // The high half pays for three pulses in the subframe's first 30 samples, starting
        // at the table's second row; the low half for four in its last 30, starting at its
        // first. The pulse magnitudes are drawn from the same seven in both halves, because
        // the second walk does not restart where the first left off.
        let mut pulses = 0usize;
        pulses = self.walk_pulses(position >> 15, 30, 3, 0..30, pulses, &tmp, out);
        self.walk_pulses(position & 0x7FFF, 0, 4, 30..SUBFRAME, pulses, &tmp, out);
    }

    /// One half of a subframe's position field spent on the table's rows: a row's entries
    /// fall toward zero, so the walk subtracts while it can afford to and drops to the next
    /// row at the first entry it cannot, placing a pulse where it lands. A row's trailing
    /// zeros cost a step and place nothing, which is how a coefficient the rows cannot spend
    /// - the four rows hold 31 930 of the 32 767 a half can state - leaves its remaining
    /// samples silent rather than wandering off the table.
    fn walk_pulses(
        &self,
        mut coefficient: i32,
        mut at: usize,
        allowed: usize,
        range: std::ops::Range<usize>,
        mut pulses: usize,
        tmp: &[i16; 7],
        out: &mut [i16; SUBFRAME],
    ) -> usize {
        let mut left = allowed;
        for index in range {
            if left == 0 {
                break;
            }
            let step = i32::from(PULSE_VALUES[at].cast_signed());
            at += 1;
            if coefficient >= step {
                coefficient -= step;
            } else {
                out[index] = tmp[pulses];
                pulses += 1;
                at += 30;
                left -= 1;
            }
        }
        pulses
    }

    /// The filtered run grows by this subframe's excitation, damped by an eighth, and the
    /// subframe's own samples take the undamped sum - the separation that lets the next
    /// subframe copy a smoothed history while this one is heard raw.
    fn update_filters(&mut self, newvec: &[i16; SUBFRAME], out: &mut [i16; SUBFRAME]) {
        self.filtbuf.copy_within(SUBFRAME..SUBFRAME + 86, 0);
        for i in 0..SUBFRAME {
            let filtered = i32::from(out[i]) + i32::from(newvec[i]) - (i32::from(newvec[i]) >> 3);
            self.filtbuf[i + 86] = filtered;
            out[i] = (i32::from(out[i]) + i32::from(newvec[i])) as i16;
        }
    }

    /// The subframe through its filter set: three stages of an eight-tap recursion, the
    /// last of which folds the pitch gain back in. The first stage multiplies through an
    /// unsigned product, as the reference does, so a wide pair of taps wraps before its
    /// shift rather than after it.
    fn synth(&mut self, filters: &[i16; 32], quart: usize, out: &mut [i16; SUBFRAME]) {
        let taps = &filters[quart * 8..quart * 8 + 8];
        for i in 0..SUBFRAME {
            let mut product = 0u32;
            for k in 0..8 {
                product = product
                    .wrapping_add((i32::from(self.tmp1[k]) as u32).wrapping_mul(taps[k] as u32));
            }
            let sum = (product.wrapping_add(0x800)) as i32 >> 12;
            let value = (i32::from(out[i]) + sum).clamp(-SYNTH_LIMIT, SYNTH_LIMIT) as i16;
            out[i] = value;
            self.tmp1.copy_within(0..7, 1);
            self.tmp1[0] = value;
        }

        let damped = decayed(taps, DECAY_35_64);
        for i in 0..SUBFRAME {
            let mut sum = 0i32;
            for k in 0..8 {
                sum = sum.wrapping_add(i32::from(self.tmp2[k]).wrapping_mul(damped[k]));
            }
            self.tmp2.copy_within(0..7, 1);
            self.tmp2[0] = out[i];
            let add = sum.wrapping_neg() >> 12;
            out[i] = (i32::from(out[i]) + add) as i16;
        }

        let damped = decayed(taps, DECAY_3_4);
        let gain = self.filtval - (self.filtval >> 2);
        for i in 0..SUBFRAME {
            let mut sum = i32::from(out[i]).wrapping_shl(12);
            for k in 0..8 {
                sum = sum.wrapping_add(i32::from(self.tmp3[k]).wrapping_mul(damped[k]));
            }
            let filtered =
                ((sum.wrapping_add(0x800)) >> 12).clamp(-SYNTH_LIMIT, SYNTH_LIMIT) as i16;
            self.tmp3.copy_within(0..7, 1);
            self.tmp3[0] = filtered;
            // The tap now at the delay line's second slot is the previous sample's filtered
            // value, and it is the pitch's own feedback term.
            let fed = i32::from(self.tmp3[1]).wrapping_mul(gain) >> 4;
            let mut sum = fed.wrapping_add(sum);
            sum = sum.wrapping_sub(sum >> 3);
            out[i] = ((sum.wrapping_add(0x800)) >> 12).clamp(-SYNTH_LIMIT, SYNTH_LIMIT) as i16;
        }
    }

    /// One block: its fields, its four filter sets, then four subframes of 60 samples.
    fn decode_block(&mut self, block: &[u8]) -> Vec<i16> {
        let frame = Bits::new(block).frame();
        self.correlate_filter(&frame.vector);
        let filters = self.merge_filters(frame.flag);
        let mut heard = Vec::with_capacity(SAMPLES_PER_BLOCK);
        for quart in 0..4 {
            let newvec = self.apply_two_point_filter(&frame, quart);
            let mut out = [0i16; SUBFRAME];
            self.place_pulses(&frame, quart, &mut out);
            self.update_filters(&newvec, &mut out);
            self.synth(&filters, quart, &mut out);
            heard.extend_from_slice(&out);
        }
        self.prevfilt.copy_from_slice(&self.cvector);
        heard
    }
}

impl AudioDecode for TrueSpeechDecoder {
    /// A packet is whole blocks, and each of them is 240 samples, so the run comes back as
    /// one packet with the caller's timestamp. The reference ends a block run at the same
    /// 32-byte step and ignores whatever tail falls past it.
    fn decode_encoded(
        &mut self,
        data: &[u8],
        pts: u64,
        _duration: u64,
    ) -> crate::Result<Option<AudioPacket>> {
        if data.len() < BLOCK_BYTES {
            return Err(invalid(&format!(
                "a TrueSpeech block is 32 bytes, this packet has {}",
                data.len()
            )));
        }
        let mut out = Vec::with_capacity(data.len() / BLOCK_BYTES * SAMPLES_PER_BLOCK * 4);
        for block in data.chunks_exact(BLOCK_BYTES) {
            for sample in self.decode_block(block) {
                out.extend_from_slice(&(f32::from(sample) / 32_768.0).to_le_bytes());
            }
        }
        Ok(Some(AudioPacket {
            data: out,
            pts,
            timebase_num: 1,
            timebase_den: self.sample_rate,
        }))
    }

    /// Drop the history: the next run starts as a cold decoder does, with no previous
    /// filter and an empty copy buffer, which is what the reference's fresh context is.
    fn reset(&mut self) {
        *self = Self::cold(self.sample_rate);
    }
}

#[cfg(test)]
mod tests {
    use super::{
        BLOCK_BYTES, Bits, CB_0, CB_1, CB_2, CB_3, CB_4, CB_5, CB_6, CB_7, SAMPLES_PER_BLOCK,
        SYNTH_LIMIT, TrueSpeechDecoder,
    };
    use crate::audio::{AudioDecode, AudioStream};
    use crate::playback_wav::{Limits, WavAudioReader};

    /// The take the decoder is proved against, and the samples the reference reads out of
    /// it: 1 442 whole blocks, 346 080 samples.
    const WAV: &[u8] = include_bytes!("../../tests/fixtures/truespeech/a6.wav");
    const S16: &[u8] = include_bytes!("../../tests/fixtures/truespeech/a6.s16");
    /// The take's first 32 blocks and their reference answer - small enough for a failing
    /// assertion to be readable, long enough for the filter's history to have filled.
    const HEAD_WAV: &[u8] = include_bytes!("../../tests/fixtures/truespeech/head.wav");
    const HEAD_S16: &[u8] = include_bytes!("../../tests/fixtures/truespeech/head.s16");
    /// Eight blocks of the same run, which is one of the player's own windows.
    const EIGHT_S16: &[u8] = include_bytes!("../../tests/fixtures/truespeech/eight.s16");
    /// Seven blocks written for this test, each with one part of its 256 bits set, and the
    /// run the reference makes of them.
    const CODEBOOK_WAV: &[u8] = include_bytes!("../../tests/fixtures/truespeech/codebook.wav");
    const CODEBOOK_S16: &[u8] = include_bytes!("../../tests/fixtures/truespeech/codebook.s16");
    /// A cold block followed by one whose every field is at its widest, which is the pair
    /// that drives the synthesis onto its clip, and the same block written on its own.
    const CLIP_WAV: &[u8] = include_bytes!("../../tests/fixtures/truespeech/clip.wav");
    const CLIP_S16: &[u8] = include_bytes!("../../tests/fixtures/truespeech/clip.s16");
    /// A block whose four copy offsets all ask for no history, written after the widest
    /// block and written on its own, with the reference's answers for both.
    const NOCOPY_WAV: &[u8] = include_bytes!("../../tests/fixtures/truespeech/nocopy.wav");
    const NOCOPY_S16: &[u8] = include_bytes!("../../tests/fixtures/truespeech/nocopy.s16");
    const COLD_WAV: &[u8] = include_bytes!("../../tests/fixtures/truespeech/cold.wav");
    const COLD_S16: &[u8] = include_bytes!("../../tests/fixtures/truespeech/cold.s16");

    /// One stream bit set in a block of zeros, in the reference's own numbering: bit 0 is
    /// the top bit of the block's fourth byte, because that is the byte its word swap puts
    /// first.
    fn bit(index: usize) -> [u8; 32] {
        let mut block = [0u8; 32];
        block[4 * (index / 32) + 3 - index % 32 / 8] = 1u8 << (7 - index % 8);
        block
    }

    /// Decode raw bytes straight through the decoder.
    fn decode(bytes: &[u8]) -> Vec<i16> {
        let mut decoder = TrueSpeechDecoder::new(8_000, 1).expect("mono at the coding's rate");
        let packet = decoder
            .decode_encoded(bytes, 0, 0)
            .expect("decodes")
            .expect("a block always yields samples");
        assert_eq!((packet.timebase_num, packet.timebase_den), (1, 8_000));
        packet
            .data
            .chunks_exact(4)
            .map(|chunk| {
                (f32::from_le_bytes(chunk.try_into().expect("four bytes")) * 32_768.0).round()
                    as i16
            })
            .collect()
    }

    /// The reference's own samples for a run.
    fn reference(bytes: &[u8]) -> Vec<i16> {
        bytes
            .chunks_exact(2)
            .map(|chunk| i16::from_le_bytes(chunk.try_into().expect("two bytes")))
            .collect()
    }

    /// The take's coded run: 46 144 bytes, which is 1 442 whole blocks. Its `fmt ` chunk
    /// carries a 32-byte extension, so the samples begin where the `data` header ends.
    fn run() -> &'static [u8] {
        &WAV[90..]
    }

    /// Open a file through the Wave reader alone, for the assertions about what it reads
    /// out of a header rather than what it makes of the bytes.
    fn open(wav: &[u8]) -> crate::Result<WavAudioReader> {
        WavAudioReader::open(wav, Limits::default())
    }

    /// The take with its `fmt ` chunk's fields rewritten: the offsets are the plain
    /// header's, which the file's own 34-byte extension follows.
    fn a6_with(byte_rate: u32, block_align: u16, bits: u16, channels: u16) -> Vec<u8> {
        let mut bytes = WAV.to_vec();
        bytes[22..24].copy_from_slice(&channels.to_le_bytes());
        bytes[28..32].copy_from_slice(&byte_rate.to_le_bytes());
        bytes[32..34].copy_from_slice(&block_align.to_le_bytes());
        bytes[34..36].copy_from_slice(&bits.to_le_bytes());
        bytes
    }

    /// The take with its `fact` chunk's one field rewritten.
    fn a6_stating(frames: u32) -> Vec<u8> {
        let mut bytes = WAV.to_vec();
        bytes[78..82].copy_from_slice(&frames.to_le_bytes());
        bytes
    }

    /// Decode through the container and the dispatch, the way the player does.
    fn through_the_player(wav: &[u8]) -> Vec<i16> {
        let mut reader = WavAudioReader::open(wav, Limits::default()).expect("opens");
        assert_eq!(reader.codec(), "truespeech");
        let mut decoder = crate::codec::make_audio_decoder(
            reader.codec(),
            reader.extra_data(),
            reader.sample_rate(),
            reader.channels(),
            reader.bits_per_sample(),
        )
        .expect("TrueSpeech arm");
        let rate = reader.sample_rate();
        let mut heard = Vec::new();
        while let Some(packet) = reader.next_packet().expect("packet") {
            let audio = decoder
                .decode_encoded(&packet.data, packet.pts as u64, packet.duration as u64)
                .expect("decode")
                .expect("a block always yields samples");
            assert_eq!((audio.timebase_num, audio.timebase_den), (1, rate));
            heard.extend(audio.data.chunks_exact(4).map(|chunk| {
                (f32::from_le_bytes(chunk.try_into().expect("four bytes")) * 32_768.0).round()
                    as i16
            }));
        }
        heard
    }

    #[test]
    fn a_blocks_bits_are_read_word_by_word_from_the_top_of_each() {
        // The reference byte-swaps every four-byte word before it reads a bit, so the
        // block's first field comes out of byte 3 and not byte 0. Each of these three
        // assertions is one stream bit, and the field it lands in is the reference's.
        assert_eq!(Bits::new(&bit(0)).frame().vector[7], CB_7[4].cast_signed());
        assert_eq!(Bits::new(&bit(24)).frame().vector[1], CB_1[2].cast_signed());
        assert_eq!(
            Bits::new(&bit(26)).frame().vector[0],
            CB_0[16].cast_signed()
        );
        // The flag is the 32nd bit of the run, which is byte 0's last - the same byte the
        // naive order starts from, and the field it would put there is a codebook.
        let zeros = Bits::new(&[0; 32]).frame();
        let blended = Bits::new(&bit(31)).frame();
        assert_eq!(blended.flag, 1);
        assert_eq!(blended.vector, zeros.vector);
        assert_eq!(
            (
                blended.offset1,
                blended.offset2,
                blended.pulsepos,
                blended.pulseoff,
                blended.pulseval
            ),
            (
                zeros.offset1,
                zeros.offset2,
                zeros.pulsepos,
                zeros.pulseoff,
                zeros.pulseval
            ),
            "one bit must move the flag and nothing else"
        );
    }

    #[test]
    fn a_zero_block_codes_the_codebooks_first_entries_and_no_offsets() {
        let frame = Bits::new(&[0; 32]).frame();
        assert_eq!(
            frame.vector,
            [
                CB_0[0], CB_1[0], CB_2[0], CB_3[0], CB_4[0], CB_5[0], CB_6[0], CB_7[0]
            ]
            .map(u16::cast_signed)
        );
        assert_eq!(
            (
                frame.flag,
                frame.offset1,
                frame.offset2,
                frame.pulsepos,
                frame.pulseoff,
                frame.pulseval
            ),
            (0, [0; 2], [0; 4], [0; 4], [0; 4], [0; 4])
        );
    }

    #[test]
    fn the_four_copy_offset_codes_are_stated_back_to_front() {
        // Set every one of them to its widest value and leave the rest of the block zero:
        // the field is the 28 bits after the first offset nibble, and the reference reads it
        // into its array from the last subframe to the first.
        let mut block = [0u8; 32];
        block[7] = 0x0F;
        block[6] = 0xFF;
        block[5] = 0xFF;
        block[4] = 0xFF;
        let frame = Bits::new(&block).frame();
        assert_eq!(frame.offset2, [127; 4], "the field is read descending");
        assert_eq!(
            frame.offset1, [0; 2],
            "the word's other nibble states the first offset, and this block leaves it at zero"
        );
    }

    #[test]
    fn a_block_of_nothing_is_silent_only_because_its_pulses_are_small() {
        // A zero block is not an empty one: every codebook's first entry is a wide negative
        // coefficient, and the position field of zero places its pulses at the very start of
        // each half with the smallest magnitudes the scale table holds.
        let heard = decode(&[0; BLOCK_BYTES]);
        assert_eq!(heard.len(), SAMPLES_PER_BLOCK);
        assert_eq!(&heard[..8], &[2, 1, 1, -1, 0, 0, 0, 0]);
        assert_eq!(heard.iter().map(|sample| sample.abs()).max(), Some(6));
    }

    #[test]
    fn a_blended_filter_runs_the_synthesis_into_its_clip() {
        // The flag's one bit says the subframes must ring against a blend of this block's
        // filter and its predecessor's. The all-ones block sets it along with every other
        // field, and written after a cold one the blend is a wide all-pass: measured, the
        // reference's answer for that pair runs to both ends of the synthesis limit within
        // the first subframe and sits on them for a half of the run.
        let heard = through_the_player(CLIP_WAV);
        let wanted = reference(CLIP_S16);
        assert_eq!(heard, wanted);
        assert_eq!(&wanted[..8], &[2, 1, 1, -1, 0, 0, 0, 0], "the cold block");
        assert_eq!(
            &wanted[SAMPLES_PER_BLOCK..SAMPLES_PER_BLOCK + 8],
            &[1, -4, 17, -92, 404, -1790, 7765, -32766],
            "the blended block rings from its first sample"
        );
        assert_eq!(
            SYNTH_LIMIT, 32_766,
            "the clip is one short of a sample's end"
        );
        assert_eq!(
            wanted.iter().filter(|s| s.abs() == 32_766).count(),
            233,
            "both ends of the clip, in the reference's own answer"
        );
        assert!(
            wanted.iter().all(|s| i32::from(s.abs()) <= SYNTH_LIMIT),
            "no sample may pass the clip the synthesis states"
        );
    }

    #[test]
    fn the_widest_copy_offset_code_still_rings_against_its_predecessor() {
        // 127 is the one value a subframe's copy offset can take that asks for no history,
        // so the two-point filter adds nothing to that subframe. Measured, that is not the
        // same as a fresh start: the same block written after the widest one the format
        // states is loud and saturating, and written on its own it is all but silent -
        // 240 samples against a maximum of six - because the short-term and long-term
        // poles the synthesis rings through are its own and no offset code cancels them.
        // A port that read 127 as "forget the state" would give the loud block the cold
        // block's answer, and this is the pair that would let it.
        let cold = through_the_player(COLD_WAV);
        assert_eq!(&cold[..8], &[2, 1, 1, -1, 0, 0, 0, 0]);
        assert_eq!(cold.iter().map(|s| s.abs()).max(), Some(6));
        assert_eq!(cold, reference(COLD_S16));
        let after = through_the_player(NOCOPY_WAV);
        assert_eq!(after, reference(NOCOPY_S16));
        assert_eq!(
            &after[SAMPLES_PER_BLOCK..SAMPLES_PER_BLOCK + 12],
            &[
                2, -9, 55, -315, 1783, -9912, 32766, -32766, 32766, -32766, 32766, -32766
            ],
            "the block that copies nothing still rings"
        );
    }

    #[test]
    fn every_written_block_lands_on_the_reference_codebook() {
        let heard = through_the_player(CODEBOOK_WAV);
        let wanted = reference(CODEBOOK_S16);
        assert_eq!(heard.len(), wanted.len());
        for (index, (from_decoder, from_ffmpeg)) in heard.iter().zip(&wanted).enumerate() {
            assert_eq!(
                from_decoder,
                from_ffmpeg,
                "sample {index} of {}: this decoder gives {from_decoder}, the reference gives {from_ffmpeg}",
                wanted.len()
            );
        }
    }

    #[test]
    fn every_sample_of_a_real_file_lands_on_the_reference() {
        let heard = through_the_player(WAV);
        let wanted = reference(S16);
        assert_eq!(heard.len(), wanted.len());
        for (index, (from_decoder, from_ffmpeg)) in heard.iter().zip(&wanted).enumerate() {
            assert_eq!(
                from_decoder,
                from_ffmpeg,
                "sample {index} of {}: this decoder gives {from_decoder}, the reference gives {from_ffmpeg}",
                wanted.len()
            );
        }
    }

    #[test]
    fn the_head_of_the_run_lands_on_the_same_reference_samples() {
        let heard = through_the_player(HEAD_WAV);
        let wanted = reference(HEAD_S16);
        assert_eq!(heard, wanted, "a prefix must not restart the state");
        let whole = through_the_player(WAV);
        assert_eq!(&whole[..heard.len()], &heard[..]);
    }

    #[test]
    fn the_state_carries_across_the_packets_the_container_hands_over() {
        // The player's window is eight blocks, and the decoder must give the same answer for
        // one packet of them as for eight packets of one: a block is not a reset point, it
        // is the unit the run is counted in.
        let data = run();
        let whole = decode(&data[..8 * BLOCK_BYTES]);
        assert_eq!(whole, reference(EIGHT_S16));
        let mut decoder = TrueSpeechDecoder::new(8_000, 1).expect("mono");
        let mut piecewise = Vec::new();
        for block in data[..8 * BLOCK_BYTES].chunks_exact(BLOCK_BYTES) {
            let packet = decoder
                .decode_encoded(block, 0, 0)
                .expect("decodes")
                .expect("samples");
            piecewise.extend(packet.data.chunks_exact(4).map(|chunk| {
                (f32::from_le_bytes(chunk.try_into().expect("four bytes")) * 32_768.0).round()
                    as i16
            }));
        }
        assert_eq!(piecewise, whole);
    }

    #[test]
    fn a_tail_shorter_than_a_block_codes_no_sample_and_a_packet_without_one_is_refused() {
        // Measured on the reference: a 33-byte run gives the same 240 samples a 32-byte one
        // does, because it decodes `size / 32` blocks and nothing else; a run that is not a
        // whole block gets an error out of it too, which is what this returns.
        let data = run();
        let mut decoder = TrueSpeechDecoder::new(8_000, 1).expect("mono");
        let tail = decoder
            .decode_encoded(&data[..33], 0, 0)
            .expect("a block plus a byte decodes")
            .expect("240 samples");
        assert_eq!(tail.data.len(), SAMPLES_PER_BLOCK * 4);
        assert_eq!(
            tail.data[..32],
            decode(&data[..32])[..8]
                .iter()
                .flat_map(|s| (f32::from(*s) / 32_768.0).to_le_bytes())
                .collect::<Vec<u8>>()[..32],
            "the extra byte must not reach the first samples"
        );
        assert!(
            decoder
                .decode_encoded(&data[..31], 0, 0)
                .err()
                .expect("a packet shorter than a block has no whole block in it")
                .to_string()
                .contains("32 bytes")
        );
    }

    #[test]
    fn a_track_that_is_not_mono_has_no_geometry_to_decode() {
        assert!(
            TrueSpeechDecoder::new(8_000, 2)
                .err()
                .expect("a stereo TrueSpeech track is refused")
                .to_string()
                .contains("one channel")
        );
        assert!(
            TrueSpeechDecoder::new(0, 1)
                .err()
                .expect("a track with no rate is refused")
                .to_string()
                .contains("no sample rate")
        );
    }

    #[test]
    fn the_format_number_alone_states_the_geometry() {
        // The take is the format's own writer: a 34-byte extension after the plain header,
        // a depth of one bit it does not mean, and a byte rate rounded off a value that is
        // not a whole number of bytes. Only the format number is read for the shape of the
        // run, and the reader says so by holding the header's claims beside its own.
        let reader = open(WAV).expect("the take opens");
        assert_eq!(reader.codec(), "truespeech");
        assert_eq!(reader.sample_rate(), 8_000);
        assert_eq!(reader.channels(), 1);
        assert_eq!(
            reader.bits_per_sample(),
            1,
            "the header's depth, unread by the coding"
        );
        let pcm = reader.wav().pcm;
        assert_eq!(pcm.block_align(), BLOCK_BYTES);
        assert_eq!(pcm.frames_per_block(), SAMPLES_PER_BLOCK);
        assert_eq!(
            pcm.derived_byte_rate(),
            None,
            "the geometry costs 1 066.6 bytes a second, which no field can state"
        );
        assert_eq!(reader.wav().frames, 1442 * SAMPLES_PER_BLOCK);
        assert_eq!(
            reader.wav().declared_frames,
            Some(345_972),
            "the take's own `fact` is carried, and contradicts its blocks by 108 frames"
        );
    }

    #[test]
    fn a_header_stating_any_other_byte_rate_or_depth_reads_the_same_run() {
        // Measured over the take: the reference decoded the same 692 160 bytes with
        // `dwAvgBytesPerSec` at 1 066 and at 8 000, and with `wBitsPerSample` at 0, 1 and
        // 16. The reader reads neither field for this coding, so the run's length cannot
        // come out depending on them.
        for (byte_rate, bits) in [(1_066, 0), (1_067, 1), (8_000, 16), (1, 65_535)] {
            let reader = open(&a6_with(byte_rate, 32, bits, 1))
                .expect("the byte rate and the depth are both unread by this coding");
            assert_eq!(
                reader.wav().frames,
                1442 * SAMPLES_PER_BLOCK,
                "{byte_rate}/{bits}"
            );
            assert_eq!(reader.bits_per_sample(), bits, "{byte_rate}/{bits}");
        }
        // The whole run of a file stating double its rate is the take's own answer.
        let heard = through_the_player(&a6_with(8_000, 32, 16, 1));
        assert_eq!(
            heard,
            reference(S16),
            "a lying byte rate must not cut the run"
        );
    }

    #[test]
    fn a_track_that_names_two_channels_is_refused() {
        // The reference's decoder is handed one channel and its block interleaves nothing,
        // so a stereo track has no geometry this port can read; measured, it fails there
        // too. The refusal is the reader's, before any decoder is built.
        let error = open(&a6_with(1_067, 32, 1, 2))
            .err()
            .expect("a stereo TrueSpeech track is refused");
        assert!(error.to_string().contains("one channel"), "{error}");
    }

    #[test]
    fn a_block_alignment_that_is_not_the_coding_s_own_is_refused() {
        // Unlike Creative's, this coding does have a block, and for the player the alignment is
        // what says where a run's units end: the window is cut in blocks of this width, so a
        // header stating 16 would hand the decoder half-blocks and one stating 64 would glue two
        // blocks into one. Measured, the reference reads neither field: the take's whole run came
        // back byte for byte the same at 16, 24, 32 and 64, because the coding counts its own 32
        // bytes and follows `data`. Only an alignment that does not divide the run reaches it -
        // at 33 the last packet holds ten bytes, the decoder answers "too small input buffer",
        // and 10 560 of the take's samples are gone - so a wrong alignment is not a harmless
        // lie about a field nothing reads, and the refusal is the player's own strictness.
        for alignment in [16u16, 64, 33] {
            let error = open(&a6_with(1_067, alignment, 1, 1))
                .err()
                .unwrap_or_else(|| panic!("a {alignment}-byte TrueSpeech block is refused"));
            assert!(
                error.to_string().contains("block alignment does not match"),
                "{alignment}: {error}"
            );
        }
        assert_eq!(
            open(&a6_with(1_067, 32, 1, 1))
                .expect("the coding's own alignment")
                .wav()
                .frames,
            1442 * SAMPLES_PER_BLOCK
        );
    }

    #[test]
    fn a_fact_that_contradicts_the_blocks_is_carried_rather_than_cross_checked() {
        // The take's own `fact` states 345 972 frames where its whole blocks code 346 080,
        // and the reference plays every one of them. G.722 and Creative's coding are held to
        // the field, because for those the run's length has nothing else to say it with;
        // here the blocks say it, and the disagreement stays visible as the pair of counts.
        for stated in [4u32, 345_972, 346_080, 999_999] {
            let reader = open(&a6_stating(stated)).expect("the fact is not checked");
            assert_eq!(reader.wav().frames, 1442 * SAMPLES_PER_BLOCK, "{stated}");
            assert_eq!(
                reader.wav().declared_frames,
                Some(stated as usize),
                "{stated}"
            );
        }
    }

    #[test]
    fn the_take_is_read_in_the_player_s_own_windows() {
        // Eight blocks to a window, because the player's grain is 2 048 frames and a block
        // codes 240 of them: the pts step is the window's frame count while its duration is
        // the 1 920 the bytes really code, the same grain a GSM run is read at. Every
        // window is whole, and the last holds the two blocks left over.
        let reader = open(WAV).expect("the take opens");
        assert_eq!(reader.wav().packets(), 181);
        assert_eq!(reader.wav().packet(0).len(), 8 * BLOCK_BYTES);
        assert_eq!(reader.wav().packet(180).len(), 2 * BLOCK_BYTES);
        assert_eq!(
            reader.duration(),
            Some(std::time::Duration::from_secs_f64(
                (1442 * SAMPLES_PER_BLOCK) as f64 / 8_000.0
            ))
        );
        let heard = through_the_player(WAV);
        assert_eq!(heard.len(), 1442 * SAMPLES_PER_BLOCK);
        assert_eq!(heard, reference(S16));
    }

    #[test]
    fn a_seek_restarts_the_coding_from_its_empty_history() {
        let data = run();
        let mut decoder = TrueSpeechDecoder::new(8_000, 1).expect("mono");
        let first = decoder
            .decode_encoded(&data[..4 * BLOCK_BYTES], 0, 0)
            .expect("decodes")
            .expect("samples");
        decoder.reset();
        let second = decoder
            .decode_encoded(&data[..4 * BLOCK_BYTES], 0, 0)
            .expect("decodes")
            .expect("samples");
        assert_eq!(first.data, second.data, "reset must be a cold decoder");
        let cold = decode(&data[..4 * BLOCK_BYTES]);
        assert_eq!(
            second
                .data
                .chunks_exact(4)
                .map(
                    |c| (f32::from_le_bytes(c.try_into().expect("four")) * 32_768.0).round() as i16
                )
                .collect::<Vec<i16>>(),
            cold
        );
    }
}
