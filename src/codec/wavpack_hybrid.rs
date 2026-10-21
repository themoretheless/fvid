//! Hybrid main-stream error budgets. Correction offsets are a separate stream.
use super::*;
const BALANCE: u32 = 1 << 10;
pub(super) struct Hybrid {
    bitrate: bool,
    balance: bool,
    channels: usize,
    slow: [u32; 2],
    accum: [u32; 2],
    delta: [i32; 2],
    limit: [u32; 2],
}
impl Hybrid {
    pub fn new(block: &Block<'_>) -> Result<Self> {
        let channels = usize::from(block.coded_channels());
        let bitrate = block.flags & HYBRID_BITRATE != 0;
        let data = block.hybrid_profile;
        let required = channels * 2 * (1 + usize::from(bitrate));
        if data.len() != required && data.len() != required + channels * 2 {
            return Err(invalid("WavPack hybrid profile is the wrong length"));
        }
        let mut out = Self {
            bitrate,
            balance: block.flags & BALANCE != 0,
            channels,
            slow: [0; 2],
            accum: [0; 2],
            delta: [0; 2],
            limit: [0; 2],
        };
        let mut at = 0;
        if bitrate {
            for c in 0..channels {
                out.slow[c] = expand(i32::from(le16(data, at) as u16))?;
                at += 2;
            }
        }
        for c in 0..channels {
            out.accum[c] = u32::from(le16(data, at) as u16) << 16;
            at += 2;
        }
        if at < data.len() {
            for c in 0..channels {
                let log = i32::from(le16(data, at) as i16);
                out.delta[c] = if log < 0 {
                    (expand(-log)? as i32).wrapping_neg()
                } else {
                    expand(log)? as i32
                };
                at += 2;
            }
        }
        Ok(out)
    }
    pub fn update(&mut self) -> Result<()> {
        let mut rates = [0i32; 2];
        for c in 0..self.channels {
            self.accum[c] = self.accum[c].wrapping_add(self.delta[c] as u32);
            rates[c] = (self.accum[c] >> 16) as i32;
        }
        let levels = self.slow.map(|s| (s.wrapping_add(128) >> 8) as i32);
        if self.bitrate && self.balance && self.channels == 2 {
            let balance = (levels[1] - levels[0] + rates[1] + 1) >> 1;
            rates = if balance > rates[0] {
                [0, rates[0] * 2]
            } else if -balance > rates[0] {
                [rates[0] * 2, 0]
            } else {
                [rates[0] - balance, rates[0] + balance]
            };
        }
        for c in 0..self.channels {
            let log = if self.bitrate {
                levels[c] - rates[c] + 256
            } else {
                rates[c]
            };
            self.limit[c] = if log <= 0 { 0 } else { expand(log)? };
        }
        Ok(())
    }
    pub fn zero(&mut self, c: usize) {
        self.slow[c] = self.slow[c].wrapping_sub(self.slow[c].wrapping_add(128) >> 8);
    }
    pub fn value(
        &mut self,
        stream: &mut Stream<'_>,
        correction: Option<&mut Stream<'_>>,
        c: usize,
        low: i32,
        width: i32,
    ) -> Result<(i32, i32)> {
        let mut low = (low as u32) & 0x7fff_ffff;
        let mut high = low.wrapping_add(width as u32) & 0x7fff_ffff;
        high = high.max(low);
        let mut mid = (u64::from(low) + u64::from(high) + 1) as u32 >> 1;
        if self.limit[c] == 0 {
            mid = low + read_code(stream, (high - low) as i32)? as u32;
        } else {
            while high - low > self.limit[c] {
                if stream.get_bit()? != 0 {
                    low = mid;
                } else {
                    high = mid - 1;
                }
                mid = ((u64::from(low) + u64::from(high) + 1) >> 1) as u32;
            }
        }
        let negative = stream.get_bit()? != 0;
        let offset = if let Some(stream) = correction.filter(|_| self.limit[c] != 0) {
            let value = low + read_code(stream, (high - low) as i32)? as u32;
            if negative {
                (mid as i32).wrapping_sub(value as i32)
            } else {
                (value as i32).wrapping_sub(mid as i32)
            }
        } else {
            0
        };
        if self.bitrate {
            self.zero(c);
            self.slow[c] = self.slow[c].wrapping_add(logarithm(mid));
        }
        Ok((if negative { !(mid as i32) } else { mid as i32 }, offset))
    }
}
fn expand(log: i32) -> Result<u32> {
    if log > 8447 {
        return Err(invalid("WavPack hybrid profile exponent exceeds 32 bits"));
    }
    Ok(exp2_value(log) as u32)
}
fn logarithm(value: u32) -> u32 {
    let value = value.wrapping_add(value >> 9);
    let bits = 32 - value.leading_zeros();
    let index = if bits < 9 {
        value << (9 - bits)
    } else {
        value >> (bits - 9)
    };
    (bits << 8) + u32::from(LOG_TABLE[(index & 255) as usize])
}
const LOG_TABLE: [u8; 256] = [
    0, 1, 3, 4, 6, 7, 9, 10, 11, 13, 14, 16, 17, 18, 20, 21, 22, 24, 25, 26, 28, 29, 30, 32, 33,
    34, 36, 37, 38, 40, 41, 42, 44, 45, 46, 47, 49, 50, 51, 52, 54, 55, 56, 57, 59, 60, 61, 62, 63,
    65, 66, 67, 68, 69, 71, 72, 73, 74, 75, 77, 78, 79, 80, 81, 82, 84, 85, 86, 87, 88, 89, 90, 92,
    93, 94, 95, 96, 97, 98, 99, 100, 102, 103, 104, 105, 106, 107, 108, 109, 110, 111, 112, 113,
    114, 116, 117, 118, 119, 120, 121, 122, 123, 124, 125, 126, 127, 128, 129, 130, 131, 132, 133,
    134, 135, 136, 137, 138, 139, 140, 141, 142, 143, 144, 145, 146, 147, 148, 149, 150, 151, 152,
    153, 154, 155, 155, 156, 157, 158, 159, 160, 161, 162, 163, 164, 165, 166, 167, 168, 169, 169,
    170, 171, 172, 173, 174, 175, 176, 177, 178, 178, 179, 180, 181, 182, 183, 184, 185, 185, 186,
    187, 188, 189, 190, 191, 192, 192, 193, 194, 195, 196, 197, 198, 198, 199, 200, 201, 202, 203,
    203, 204, 205, 206, 207, 208, 208, 209, 210, 211, 212, 212, 213, 214, 215, 216, 216, 217, 218,
    219, 220, 220, 221, 222, 223, 224, 224, 225, 226, 227, 228, 228, 229, 230, 231, 231, 232, 233,
    234, 234, 235, 236, 237, 238, 238, 239, 240, 241, 241, 242, 243, 244, 244, 245, 246, 247, 247,
    248, 249, 249, 250, 251, 252, 252, 253, 254, 255, 255,
];
