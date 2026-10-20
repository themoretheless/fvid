//! Owned MPEG Layer I allocation, requantization and polyphase reconstruction.
//! ISO/IEC 11172-3 sections 2.4.2.5 / 2.4.3.2; no external codec is linked.
use crate::{Result, invalid, mpeg_synthesis::Synthesis};
#[derive(Clone, Copy, Debug)]
pub struct Header {
    pub sample_rate: u32,
    pub channels: u16,
    pub frame_bytes: usize,
    pub joint_bound: usize,
    pub crc: bool,
    pub free: bool,
    pub padding_bytes: usize,
}
impl Header {
    pub fn parse(packet: &[u8]) -> Result<Self> {
        let bytes: [u8; 4] = packet
            .get(..4)
            .ok_or_else(|| invalid("truncated MP1 header"))?
            .try_into()
            .unwrap();
        let word = u32::from_be_bytes(bytes);
        let version = (word >> 19) & 3;
        let index = ((word >> 12) & 15) as usize;
        let rate_index = ((word >> 10) & 3) as usize;
        if word >> 21 != 0x7ff
            || version == 1
            || (word >> 17) & 3 != 3
            || index == 15
            || rate_index == 3
            || word & 3 == 2
        {
            return Err(invalid("invalid MP1 header"));
        }
        let sample_rate = [44100, 48000, 32000][rate_index]
            >> match version {
                3 => 0,
                2 => 1,
                _ => 2,
            };
        let rates = if version == 3 {
            [
                0, 32, 64, 96, 128, 160, 192, 224, 256, 288, 320, 352, 384, 416, 448, 0,
            ]
        } else {
            [
                0, 32, 48, 56, 64, 80, 96, 112, 128, 144, 160, 176, 192, 224, 256, 0,
            ]
        };
        let mode = (word >> 6) & 3;
        let padding_bytes = (((word >> 9) & 1) * 4) as usize;
        Ok(Self {
            sample_rate,
            channels: if mode == 3 { 1 } else { 2 },
            frame_bytes: if index == 0 {
                packet.len()
            } else {
                ((12000 * rates[index] / sample_rate) as usize) * 4 + padding_bytes
            },
            joint_bound: if mode == 1 {
                4 * (((word >> 4) & 3) as usize + 1)
            } else {
                32
            },
            crc: word & (1 << 16) == 0,
            free: index == 0,
            padding_bytes,
        })
    }
}
struct Bits<'a> {
    data: &'a [u8],
    position: usize,
    crc: u16,
}
impl Bits<'_> {
    fn bit(&mut self, bit: u16) {
        let top = (self.crc >> 15) ^ bit;
        self.crc <<= 1;
        if top != 0 {
            self.crc ^= 0x8005;
        }
    }
    fn read(&mut self, n: usize, protected: bool) -> Result<u32> {
        if n > 32 || n > self.data.len() * 8 - self.position {
            return Err(invalid("truncated MP1 payload"));
        }
        let mut value = 0;
        for _ in 0..n {
            let bit = (self.data[self.position / 8] >> (7 - self.position % 8)) & 1;
            self.position += 1;
            value = (value << 1) | u32::from(bit);
            if protected {
                self.bit(u16::from(bit));
            }
        }
        Ok(value)
    }
}
#[derive(Clone)]
pub struct Decoder {
    rate: u32,
    channels: u16,
    filters: [Synthesis; 2],
    free_base: Option<usize>,
}
impl Decoder {
    pub fn new(rate: u32, channels: u16) -> Result<Self> {
        if ![8000, 11025, 12000, 16000, 22050, 24000, 32000, 44100, 48000].contains(&rate)
            || !(1..=2).contains(&channels)
        {
            return Err(invalid("invalid MP1 output geometry"));
        }
        Ok(Self {
            rate,
            channels,
            filters: std::array::from_fn(|_| Synthesis::default()),
            free_base: None,
        })
    }
    pub fn reset(&mut self) {
        for filter in &mut self.filters {
            filter.reset();
        }
        self.free_base = None;
    }
    pub fn decode(&mut self, packet: &[u8]) -> Result<Vec<f32>> {
        let h = Header::parse(packet)?;
        if h.sample_rate != self.rate || h.channels != self.channels {
            return Err(invalid("MP1 frame geometry disagrees with output"));
        }
        if packet.len() != h.frame_bytes || packet.len() % 4 != 0 {
            return Err(invalid(
                "MP1 packet does not contain one complete four-byte-slot frame",
            ));
        }
        let base = if h.free {
            let base = packet
                .len()
                .checked_sub(h.padding_bytes)
                .ok_or_else(|| invalid("invalid MP1 free padding"))?;
            if self.free_base.is_some_and(|old| old != base) {
                return Err(invalid(
                    "free-format MP1 frame length changes beyond padding",
                ));
            }
            Some(base)
        } else {
            None
        };
        let prefix = if h.crc { 6 } else { 4 };
        let payload = packet
            .get(prefix..)
            .ok_or_else(|| invalid("truncated MP1 CRC"))?;
        let mut bits = Bits {
            data: payload,
            position: 0,
            crc: 0xffff,
        };
        for byte in &packet[2..4] {
            for shift in (0..8).rev() {
                bits.bit(u16::from((byte >> shift) & 1));
            }
        }
        let count = usize::from(self.channels);
        let mut widths = [[0usize; 32]; 2];
        for band in 0..32 {
            for channel in 0..if band < h.joint_bound { count } else { 1 } {
                let code = bits.read(4, true)? as usize;
                if code == 15 {
                    return Err(invalid("reserved MP1 bit allocation"));
                }
                widths[channel][band] = if code == 0 { 0 } else { code + 1 };
            }
            if band >= h.joint_bound {
                widths[1][band] = widths[0][band];
            }
        }
        if h.crc && bits.crc != u16::from_be_bytes(packet[4..6].try_into().unwrap()) {
            return Err(invalid("MP1 protected allocation CRC mismatch"));
        }
        let mut scale = [[0.; 32]; 2];
        for band in 0..32 {
            for channel in 0..count {
                if widths[channel][band] != 0 {
                    let index = bits.read(6, false)?;
                    scale[channel][band] = if index == 63 {
                        0.
                    } else {
                        2f64.powf(1. - f64::from(index) / 3.)
                    };
                }
            }
        }
        let mut filters = self.filters.clone();
        let mut output = Vec::new();
        output
            .try_reserve_exact(384 * count)
            .map_err(|_| invalid("MP1 PCM allocation failed"))?;
        for _ in 0..12 {
            let mut subbands = [[0.; 32]; 2];
            for band in 0..32 {
                for channel in 0..if band < h.joint_bound { count } else { 1 } {
                    let width = widths[channel][band];
                    if width == 0 {
                        continue;
                    }
                    let raw = bits.read(width, false)?;
                    // Invert the sign bit, interpret the fractional number and
                    // apply the Layer I linear requantization coefficients.
                    let value = (2. * f64::from(raw) + 2. - f64::from(1u32 << width))
                        / f64::from((1u32 << width) - 1);
                    if band < h.joint_bound {
                        subbands[channel][band] = value * scale[channel][band];
                    } else {
                        for ch in 0..count {
                            subbands[ch][band] = value * scale[ch][band];
                        }
                    }
                }
            }
            let mut pcm = [[0.; 32]; 2];
            for channel in 0..count {
                pcm[channel] = filters[channel].push(&subbands[channel])?;
            }
            for sample in 0..32 {
                for channel in 0..count {
                    let value = pcm[channel][sample] as f32;
                    if !value.is_finite() {
                        return Err(invalid("MP1 PCM overflow"));
                    }
                    output.push(value);
                }
            }
        }
        self.filters = filters;
        self.free_base = base;
        Ok(output)
    }
}
