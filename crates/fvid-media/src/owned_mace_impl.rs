// Shared pure MACE predictor and reconstruction engine. The includer supplies Result/invalid.
/// Samples one block codes, for either coding: 3-to-1 spends two bytes on them, 6-to-1
/// one byte, and both come out at 2.67 bits a sample.
pub const SAMPLES_PER_BLOCK: usize = 6;

/// Codes one byte holds, and so the number of table pairs a block's codes walk through.
const CODES_PER_BYTE: usize = 3;

/// Which of the two codings a byte of MACE is to be read as. The container states it -
/// a `MAC3` or `MAC6` fourcc in an ISO BMFF sample entry, the same fourcc in an AIFF-C
/// `COMM` chunk - and nothing in the byte itself distinguishes them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MaceCoding {
    /// Three samples a byte: each code yields one predictor output.
    ThreeToOne,
    /// Six samples a byte: each code yields an interpolated pair.
    SixToOne,
}

impl MaceCoding {
    /// Bytes one channel's block takes.
    pub fn bytes_per_channel(self) -> usize {
        match self {
            Self::ThreeToOne => 2,
            Self::SixToOne => 1,
        }
    }

    /// Bytes of one block: every channel's two or one bytes, laid out in the channel
    /// order the reference reads them in.
    pub fn block_bytes(self, channels: u16) -> usize {
        self.bytes_per_channel() * usize::from(channels)
    }

    /// The decoder dispatch's name for this coding, which is also this build's reference
    /// descriptor for it.
    pub fn codec(self) -> &'static str {
        match self {
            Self::ThreeToOne => "mace3",
            Self::SixToOne => "mace6",
        }
    }
}

/// How far a code moves its channel's position in the level table, for a row that is
/// four wide. The two ends are negative, so the codes at either end of the range pull the
/// position back towards the quiet rows while the middle ones push it up.
const STEP_WIDE: [i16; 8] = [-13, 8, 76, 222, 222, 76, 8, -13];

/// The same, for a row that is two wide.
const STEP_NARROW: [i16; 4] = [-18, 140, 140, -18];

/// Step levels for a four-wide row: 128 positions of four entries each, laid out flat as
/// the reference's `MACEtab2[][4]` is and read the same way - position `>> 4` times four,
/// plus the code. Codes 4 to 7 read that row backwards and return `-1 - level`, which is
/// how eight codes come out of four entries and why the table holds no negative number.
/// Its last rows are already clipped to full scale by the same hand that wrote them.
const LEVEL_WIDE: [i16; 512] = [
    37, 116, 206, 330, 39, 121, 216, 346, 41, 127, 225, 361, 42, 132, 235, 377, 44, 137, 245, 392,
    46, 144, 256, 410, 48, 150, 267, 428, 51, 157, 280, 449, 53, 165, 293, 470, 55, 172, 306, 490,
    58, 179, 319, 511, 60, 187, 333, 534, 63, 195, 348, 557, 66, 205, 364, 583, 69, 214, 380, 609,
    72, 223, 396, 635, 75, 233, 414, 663, 79, 244, 433, 694, 82, 254, 453, 725, 86, 265, 472, 756,
    90, 278, 495, 792, 94, 290, 516, 826, 98, 303, 538, 862, 102, 316, 562, 901, 107, 331, 588,
    942, 112, 345, 614, 983, 117, 361, 641, 1027, 122, 377, 670, 1074, 127, 394, 701, 1123, 133,
    411, 732, 1172, 139, 430, 764, 1224, 145, 449, 799, 1280, 152, 469, 835, 1337, 159, 490, 872,
    1397, 166, 512, 911, 1459, 173, 535, 951, 1523, 181, 558, 993, 1590, 189, 584, 1038, 1663, 197,
    610, 1085, 1738, 206, 637, 1133, 1815, 215, 665, 1183, 1895, 225, 695, 1237, 1980, 235, 726,
    1291, 2068, 246, 759, 1349, 2161, 257, 792, 1409, 2257, 268, 828, 1472, 2357, 280, 865, 1538,
    2463, 293, 903, 1606, 2572, 306, 944, 1678, 2688, 319, 986, 1753, 2807, 334, 1030, 1832, 2933,
    349, 1076, 1914, 3065, 364, 1124, 1999, 3202, 380, 1174, 2088, 3344, 398, 1227, 2182, 3494,
    415, 1281, 2278, 3649, 434, 1339, 2380, 3811, 453, 1398, 2486, 3982, 473, 1461, 2598, 4160,
    495, 1526, 2714, 4346, 517, 1594, 2835, 4540, 540, 1665, 2961, 4741, 564, 1740, 3093, 4953,
    589, 1818, 3232, 5175, 615, 1898, 3375, 5405, 643, 1984, 3527, 5647, 671, 2072, 3683, 5898,
    701, 2164, 3848, 6161, 733, 2261, 4020, 6438, 766, 2362, 4199, 6724, 800, 2467, 4386, 7024,
    836, 2578, 4583, 7339, 873, 2692, 4786, 7664, 912, 2813, 5001, 8008, 952, 2938, 5223, 8364,
    995, 3070, 5457, 8739, 1039, 3207, 5701, 9129, 1086, 3350, 5956, 9537, 1134, 3499, 6220, 9960,
    1185, 3655, 6497, 10404, 1238, 3818, 6788, 10869, 1293, 3989, 7091, 11355, 1351, 4166, 7407,
    11861, 1411, 4352, 7738, 12390, 1474, 4547, 8084, 12946, 1540, 4750, 8444, 13522, 1609, 4962,
    8821, 14126, 1680, 5183, 9215, 14756, 1756, 5415, 9626, 15415, 1834, 5657, 10057, 16104, 1916,
    5909, 10505, 16822, 2001, 6173, 10975, 17574, 2091, 6448, 11463, 18356, 2184, 6736, 11974,
    19175, 2282, 7037, 12510, 20032, 2383, 7351, 13068, 20926, 2490, 7679, 13652, 21861, 2601,
    8021, 14260, 22834, 2717, 8380, 14897, 23854, 2838, 8753, 15561, 24918, 2965, 9144, 16256,
    26031, 3097, 9553, 16982, 27193, 3236, 9979, 17740, 28407, 3380, 10424, 18532, 29675, 3531,
    10890, 19359, 31000, 3688, 11375, 20222, 32382, 3853, 11883, 21125, 32767, 4025, 12414, 22069,
    32767, 4205, 12967, 23053, 32767, 4392, 13546, 24082, 32767, 4589, 14151, 25157, 32767, 4793,
    14783, 26280, 32767, 5007, 15442, 27452, 32767, 5231, 16132, 28678, 32767, 5464, 16851, 29957,
    32767, 5708, 17603, 31294, 32767, 5963, 18389, 32691, 32767, 6229, 19210, 32767, 32767, 6507,
    20067, 32767, 32767, 6797, 20963, 32767, 32767, 7101, 21899, 32767, 32767, 7418, 22876, 32767,
    32767, 7749, 23897, 32767, 32767, 8095, 24964, 32767, 32767, 8456, 26078, 32767, 32767, 8833,
    27242, 32767, 32767, 9228, 28457, 32767, 32767, 9639, 29727, 32767, 32767,
];

/// The 128 positions again, two entries wide, mirrored at codes 2 and 3.
const LEVEL_NARROW: [i16; 256] = [
    64, 216, 67, 226, 70, 236, 74, 246, 77, 257, 80, 268, 84, 280, 88, 294, 92, 307, 96, 321, 100,
    334, 104, 350, 109, 365, 114, 382, 119, 399, 124, 416, 130, 434, 136, 454, 142, 475, 148, 495,
    155, 519, 162, 541, 169, 564, 176, 590, 185, 617, 193, 644, 201, 673, 210, 703, 220, 735, 230,
    767, 240, 801, 251, 838, 262, 876, 274, 914, 286, 955, 299, 997, 312, 1041, 326, 1089, 341,
    1138, 356, 1188, 372, 1241, 388, 1297, 406, 1354, 424, 1415, 443, 1478, 462, 1544, 483, 1613,
    505, 1684, 527, 1760, 551, 1838, 576, 1921, 601, 2007, 628, 2097, 656, 2190, 686, 2288, 716,
    2389, 748, 2496, 781, 2607, 816, 2724, 853, 2846, 891, 2973, 930, 3104, 972, 3243, 1016, 3389,
    1061, 3539, 1108, 3698, 1158, 3862, 1209, 4035, 1264, 4216, 1320, 4403, 1379, 4599, 1441, 4806,
    1505, 5019, 1572, 5244, 1642, 5477, 1715, 5722, 1792, 5978, 1872, 6245, 1955, 6522, 2043, 6813,
    2134, 7118, 2229, 7436, 2329, 7767, 2432, 8114, 2541, 8477, 2655, 8854, 2773, 9250, 2897, 9663,
    3026, 10094, 3162, 10546, 3303, 11016, 3450, 11508, 3604, 12020, 3765, 12556, 3933, 13118,
    4108, 13703, 4292, 14315, 4483, 14953, 4683, 15621, 4892, 16318, 5111, 17046, 5339, 17807,
    5577, 18602, 5826, 19433, 6086, 20300, 6358, 21205, 6642, 22152, 6938, 23141, 7248, 24173,
    7571, 25252, 7909, 26380, 8262, 27557, 8631, 28786, 9016, 30072, 9419, 31413, 9839, 32767,
    10278, 32767, 10737, 32767, 11216, 32767, 11717, 32767, 12240, 32767, 12786, 32767, 13356,
    32767, 13953, 32767, 14576, 32767, 15226, 32767, 15906, 32767, 16615, 32767,
];

/// The pair of tables one of a byte's three code slots reads: the wide row for the two
/// three-bit codes, the narrow one for the two-bit code between them. Both codings use
/// the same three slots, which is why a byte of one is a legal byte of the other.
#[derive(Clone, Copy, Debug)]
struct Tables {
    steps: &'static [i16],
    levels: &'static [i16],
    stride: usize,
}

const WIDE: Tables = Tables {
    steps: &STEP_WIDE,
    levels: &LEVEL_WIDE,
    stride: 4,
};

const NARROW: Tables = Tables {
    steps: &STEP_NARROW,
    levels: &LEVEL_NARROW,
    stride: 2,
};

const fn slot_tables(slot: usize) -> &'static Tables {
    match slot {
        1 => &NARROW,
        _ => &WIDE,
    }
}

/// The clip the reference applies on the way into the predictor, which it names
/// `mace_broken_clip_int16` and keeps to match the Mac binary's output bit for bit. Its
/// two branches are not twins: a value under a sample's bottom comes back as -32 767,
/// while the bottom value -32 768 itself is the one the comparison lets through. A
/// symmetric clamp differs at that first branch, and the predictor carries the
/// difference forward into every later sample.
fn clip_int16(value: i32) -> i16 {
    if value > 32_767 {
        32_767
    } else if value < -32_768 {
        -32_767
    } else {
        value as i16
    }
}

/// The reference's `QT_8S_2_16S`: the value's high byte, written into both halves of the
/// sample it returns. MACE codes at 8-bit resolution and hands the player a 16-bit stream
/// of it, which is what makes a MACE decode's low byte uninformative.
fn quantize(value: i32) -> i16 {
    ((value & 0xFF00) | ((value >> 8) & 0xFF)) as i16
}

/// One channel's predictor: where it stands in the level table, how wide a step its
/// interpolator is currently taking, and the two past samples 6-to-1 reads.
#[derive(Clone, Copy, Debug, Default)]
struct Channel {
    position: i16,
    adaptation: i16,
    past2: i16,
    past1: i16,
    level: i16,
}

impl Channel {
    /// The step a code stands for at this channel's position, and the move that code then
    /// makes in the position itself. The position decays towards zero on every read - the
    /// `index >> 5` the reference subtracts - which is what lets a run of small codes bring
    /// a loud passage back down without any code having to state a level of its own.
    fn read_table(&mut self, code: usize, tables: &Tables) -> i16 {
        let row = ((i32::from(self.position) & 0x7f0) >> 4) as usize * tables.stride;
        let level = if code < tables.stride {
            i32::from(tables.levels[row + code])
        } else {
            -1 - i32::from(tables.levels[row + 2 * tables.stride - code - 1])
        };
        self.position = (i32::from(self.position) + i32::from(tables.steps[code])
            - (i32::from(self.position) >> 5)) as i16;
        if self.position < 0 {
            self.position = 0;
        }
        level as i16
    }

    /// 3-to-1's use of a code: the step added to the running level is the sample.
    fn chomp3(&mut self, code: usize, tables: &Tables, out: &mut Vec<i16>) {
        let current = i32::from(clip_int16(
            i32::from(self.read_table(code, tables)) + i32::from(self.level),
        ));
        self.level = (current - (current >> 3)) as i16;
        out.push(quantize(current));
    }

    /// 6-to-1's use of a code: the same step, but its size is decided by an adaptation that
    /// follows the direction of the signal, and the output is the pair the interpolator
    /// places between the past sample and the new one.
    fn chomp6(&mut self, code: usize, tables: &Tables, out: &mut Vec<i16>) {
        let step = i32::from(self.read_table(code, tables));
        // A step in the same direction as the last sample grows the adaptation; one that
        // crosses it shrinks the step instead, which is how the coder stops ringing after
        // a transient.
        if (i32::from(self.past1) ^ step) >= 0 {
            self.adaptation = (i32::from(self.adaptation) + 506).min(32_767) as i16;
        } else if i32::from(self.adaptation) - 314 < -32_768 {
            self.adaptation = -32_767;
        } else {
            self.adaptation = (i32::from(self.adaptation) - 314) as i16;
        }
        let mut current = i32::from(clip_int16(step + i32::from(self.level)));
        self.level = ((current * i32::from(self.adaptation)) >> 15) as i16;
        current = (current >> 1) as i16 as i32;
        let between = (i32::from(self.past2) - current) >> 2;
        out.push(quantize(
            i32::from(self.past1) + i32::from(self.past2) - between,
        ));
        out.push(quantize(i32::from(self.past1) + current + between));
        self.past2 = self.past1;
        self.past1 = current as i16;
    }
}

/// The three codes of a byte, in the order each coding reads them: 3-to-1 takes the byte
/// from its low end up and 6-to-1 from its top down, so the same eight bits give the three
/// slots in opposite orders.
fn codes(byte: u8, coding: MaceCoding) -> [usize; CODES_PER_BYTE] {
    match coding {
        MaceCoding::ThreeToOne => [
            usize::from(byte & 7),
            usize::from((byte >> 3) & 3),
            usize::from(byte >> 5),
        ],
        MaceCoding::SixToOne => [
            usize::from(byte >> 5),
            usize::from((byte >> 3) & 3),
            usize::from(byte & 7),
        ],
    }
}

/// A MACE decoder, one predictor per channel.
pub struct MaceDecoder {
    coding: MaceCoding,
    sample_rate: u32,
    channels: u16,
    state: [Channel; 2],
}

impl MaceDecoder {
    /// Build a decoder for one of the two codings. The container's fourcc decides which,
    /// the channel count decides the block's width, and no setup block is read: neither
    /// coding has one.
    pub fn new(coding: MaceCoding, sample_rate: u32, channels: u16) -> Result<Self> {
        if channels < 1 || channels > 2 {
            return Err(invalid(&format!(
                "MACE codes one or two channels, one predictor each; this track names {channels}"
            )));
        }
        if sample_rate == 0 {
            return Err(invalid("MACE track has no sample rate"));
        }
        Ok(Self {
            coding,
            sample_rate,
            channels,
            state: [Channel::default(), Channel::default()],
        })
    }

    pub fn sample_rate(&self) -> u32 {
        self.sample_rate
    }
    pub fn channels(&self) -> u16 {
        self.channels
    }
    pub fn reset(&mut self) {
        self.state = [Channel::default(), Channel::default()];
    }
    fn decode_planar(&mut self, data: &[u8]) -> Result<[Vec<i16>; 2]> {
        let block = self.coding.block_bytes(self.channels);
        if data.len() % block != 0 {
            // The reference drops the standing-over bytes and reports the packet as
            // damaged; both containers this player reads MACE out of hand it whole blocks,
            // so a remainder is a file that lies about its geometry rather than a run to
            // be played as far as it goes.
            return Err(invalid(&format!(
                "{} bytes are not a whole number of {block}-byte {} blocks",
                data.len(),
                self.coding.codec()
            )));
        }
        let blocks = data.len() / block;
        let per_channel = self.coding.bytes_per_channel();
        let mut planar = [
            Vec::with_capacity(blocks * SAMPLES_PER_BLOCK),
            Vec::with_capacity(blocks * SAMPLES_PER_BLOCK),
        ];
        for channel in 0..usize::from(self.channels) {
            let state = &mut self.state[channel];
            // One channel's run at a time: the reference reads a channel's whole buffer
            // before the next one's, and a block's bytes for this channel start at
            // `channel * 2` (3-to-1) or `channel` (6-to-1) inside the block and step over
            // the other channel's share.
            let mut out = &mut planar[channel];
            for block_index in 0..blocks {
                for byte in 0..per_channel {
                    let at = channel * per_channel + block_index * block + byte;
                    for (slot, code) in codes(data[at], self.coding).into_iter().enumerate() {
                        let tables = slot_tables(slot);
                        match self.coding {
                            MaceCoding::ThreeToOne => state.chomp3(code, tables, &mut out),
                            MaceCoding::SixToOne => state.chomp6(code, tables, &mut out),
                        }
                    }
                }
            }
        }
        Ok(planar)
    }

    /// Owned float PCM for container exports, with the same persistent
    /// predictors as playback and without allocating an intermediate byte PCM.
    pub fn decode_pcm(&mut self, data: &[u8]) -> Result<Vec<f32>> {
        let planar = self.decode_planar(data)?;
        let frames = planar[0].len();
        let mut output = Vec::with_capacity(frames * usize::from(self.channels));
        for frame in 0..frames {
            for channel in 0..usize::from(self.channels) {
                output.push(f32::from(planar[channel][frame]) / 32768.0);
            }
        }
        Ok(output)
    }
}
