//! Owned MPEG Layer II frame reconstruction and persistent polyphase synthesis.
use crate::{Result, invalid, mpeg_synthesis::Synthesis};
struct Bits<'a> {
    data: &'a [u8],
    pos: usize,
    crc: u16,
    protected: bool,
}
impl Bits<'_> {
    fn feed(&mut self, bit: u16) {
        let feedback = (self.crc >> 15) ^ bit;
        self.crc <<= 1;
        if feedback != 0 {
            self.crc ^= 0x8005;
        }
    }
    fn read(&mut self, n: usize) -> Result<u32> {
        if n > 32 || self.pos + n > self.data.len() * 8 {
            return Err(invalid("truncated MP2 bitstream"));
        }
        let mut value = 0;
        for _ in 0..n {
            let bit = (self.data[self.pos / 8] >> (7 - self.pos % 8)) & 1;
            self.pos += 1;
            value = (value << 1) | u32::from(bit);
            if self.protected {
                self.feed(u16::from(bit));
            }
        }
        Ok(value)
    }
}
#[derive(Clone, Copy, Debug)]
pub struct Header {
    pub sample_rate: u32,
    pub channels: u16,
    pub frame_bytes: usize,
    pub table: usize,
    pub joint_bound: usize,
    pub crc: bool,
}
impl Header {
    pub fn parse(packet: &[u8]) -> Result<Self> {
        if packet.len() < 4 {
            return Err(invalid("truncated MP2 header"));
        }
        let h = u32::from_be_bytes(packet[..4].try_into().unwrap());
        let version = (h >> 19) & 3;
        let layer = (h >> 17) & 3;
        let index = ((h >> 12) & 15) as usize;
        let rate_index = ((h >> 10) & 3) as usize;
        if h >> 21 != 0x7ff
            || version == 1
            || layer != 2
            || index == 15
            || rate_index == 3
            || h & 3 == 2
        {
            return Err(invalid("invalid MP2 header"));
        }
        let sample_rate = [44100, 48000, 32000][rate_index]
            >> match version {
                3 => 0,
                2 => 1,
                _ => 2,
            };
        let rates = if version == 3 {
            [
                0, 32, 48, 56, 64, 80, 96, 112, 128, 160, 192, 224, 256, 320, 384, 0,
            ]
        } else {
            [
                0, 8, 16, 24, 32, 40, 48, 56, 64, 80, 96, 112, 128, 144, 160, 0,
            ]
        };
        let bitrate = rates[index];
        let mode = (h >> 6) & 3;
        let channels = if mode == 3 { 1 } else { 2 };
        let per = bitrate / u32::from(channels);
        let table = if version != 3 {
            4
        } else if index == 0 {
            // Free format has explicit entries in Tables B.2a/B.2b, not an
            // estimated bitrate threshold (a frame length cannot recover that).
            if sample_rate == 48000 { 0 } else { 1 }
        } else if (sample_rate == 48000 && per >= 56) || (56..=80).contains(&per) {
            0
        } else if sample_rate != 48000 && per >= 96 {
            1
        } else if sample_rate != 32000 && per <= 48 {
            2
        } else {
            3
        };
        let limit = [27, 30, 8, 12, 30][table];
        let joint_bound = if mode == 1 {
            (4 * (((h >> 4) & 3) as usize + 1)).min(limit)
        } else {
            limit
        };
        Ok(Self {
            sample_rate,
            channels,
            frame_bytes: if index == 0 {
                packet.len()
            } else {
                (144000 * bitrate / sample_rate + ((h >> 9) & 1)) as usize
            },
            table,
            joint_bound,
            crc: h & (1 << 16) == 0,
        })
    }
}
// ISO/IEC 11172-3 Table B.2 and the low-sampling-frequency allocation layout.
fn allocation(table: usize, band: usize) -> (usize, &'static [u32]) {
    const A: &[u32] = &[
        3, 7, 15, 31, 63, 127, 255, 511, 1023, 2047, 4095, 8191, 16383, 32767, 65535,
    ];
    const B: &[u32] = &[
        3, 5, 7, 9, 15, 31, 63, 127, 255, 511, 1023, 2047, 4095, 8191, 65535,
    ];
    const C: &[u32] = &[3, 5, 7, 9, 15, 31, 65535];
    const D: &[u32] = &[3, 5, 65535];
    const E: &[u32] = &[
        3, 5, 9, 15, 31, 63, 127, 255, 511, 1023, 2047, 4095, 8191, 16383, 32767,
    ];
    const F: &[u32] = &[3, 5, 9, 15, 31, 63, 127];
    const G: &[u32] = &[
        3, 5, 7, 9, 15, 31, 63, 127, 255, 511, 1023, 2047, 4095, 8191, 16383,
    ];
    const H: &[u32] = &[3, 5, 9];
    match table {
        0 | 1 => match band {
            0..=2 => (4, A),
            3..=10 => (4, B),
            11..=22 => (3, C),
            _ => (2, D),
        },
        2 | 3 => {
            if band < 2 {
                (4, E)
            } else {
                (3, F)
            }
        }
        _ => match band {
            0..=3 => (4, G),
            4..=10 => (3, F),
            _ => (2, H),
        },
    }
}
fn triplet(b: &mut Bits<'_>, levels: u32) -> Result<[f64; 3]> {
    let grouped = matches!(levels, 3 | 5 | 9);
    let mut codes = [0; 3];
    if grouped {
        let width = match levels {
            3 => 5,
            5 => 7,
            _ => 10,
        };
        let mut word = b.read(width)?;
        if word >= levels.pow(3) {
            return Err(invalid("invalid MP2 grouped quantizer word"));
        }
        for v in &mut codes {
            *v = word % levels;
            word /= levels;
        }
    } else {
        let width = (levels + 1).ilog2() as usize;
        for v in &mut codes {
            *v = b.read(width)?;
            if *v >= levels {
                return Err(invalid("invalid MP2 ungrouped quantizer word"));
            }
        }
    }
    Ok(codes.map(|v| (2. * f64::from(v) + 1. - f64::from(levels)) / f64::from(levels)))
}
pub struct Decoder {
    sample_rate: u32,
    channels: u16,
    filters: [Synthesis; 2],
    free_base: Option<usize>,
}
impl Decoder {
    pub fn new(sample_rate: u32, channels: u16) -> Result<Self> {
        if sample_rate == 0 || !(1..=2).contains(&channels) {
            return Err(invalid("invalid MP2 output geometry"));
        }
        Ok(Self {
            sample_rate,
            channels,
            filters: std::array::from_fn(|_| Synthesis::default()),
            free_base: None,
        })
    }
    pub fn reset(&mut self) {
        self.free_base = None;
        for f in &mut self.filters {
            f.reset();
        }
    }
    pub fn decode(&mut self, packet: &[u8]) -> Result<Vec<f32>> {
        let h = Header::parse(packet)?;
        let free = packet[2] >> 4 == 0;
        let base = if free {
            let padding = usize::from((packet[2] >> 1) & 1);
            let base = packet
                .len()
                .checked_sub(padding)
                .ok_or_else(|| invalid("invalid free-format MP2 padding"))?;
            if self.free_base.is_some_and(|previous| previous != base) {
                return Err(invalid(
                    "free-format MP2 frame length changes beyond padding",
                ));
            }
            Some(base)
        } else {
            None
        };

        if h.sample_rate != self.sample_rate || h.channels != self.channels {
            return Err(invalid("MP2 frame geometry disagrees with output"));
        }
        if packet.len() != h.frame_bytes {
            return Err(invalid("MP2 packet does not contain one complete frame"));
        }
        let header = if h.crc { 6 } else { 4 };
        if packet.len() < header {
            return Err(invalid("truncated MP2 CRC"));
        }
        let expected = if h.crc {
            u16::from_be_bytes(packet[4..6].try_into().unwrap())
        } else {
            0
        };
        let mut b = Bits {
            data: &packet[header..],
            pos: 0,
            crc: 0xffff,
            protected: false,
        };
        if h.crc {
            for &v in &packet[2..4] {
                for bit in (0..8).rev() {
                    b.feed(u16::from((v >> bit) & 1));
                }
            }
            b.protected = true;
        }
        let channels = usize::from(h.channels);
        let limit = [27, 30, 8, 12, 30][h.table];
        let mut levels = [[0; 32]; 2];
        let mut scfsi = [[0; 32]; 2];
        let mut scales = [[[0.; 3]; 32]; 2];
        for sb in 0..limit {
            let (width, map) = allocation(h.table, sb);
            for ch in 0..if sb < h.joint_bound { channels } else { 1 } {
                let code = b.read(width)? as usize;
                levels[ch][sb] = if code == 0 { 0 } else { map[code - 1] };
            }
            if sb >= h.joint_bound {
                levels[1][sb] = levels[0][sb];
            }
        }
        for sb in 0..limit {
            for ch in 0..channels {
                if levels[ch][sb] != 0 {
                    scfsi[ch][sb] = b.read(2)?;
                }
            }
        }
        if h.crc && b.crc != expected {
            return Err(invalid("MP2 protected fields CRC mismatch"));
        }
        b.protected = false;
        let scale = |b: &mut Bits<'_>| -> Result<f64> {
            let index = b.read(6)?;
            Ok(if index == 63 {
                0.
            } else {
                2f64.powf(1. - f64::from(index) / 3.)
            })
        };
        for sb in 0..limit {
            for ch in 0..channels {
                if levels[ch][sb] == 0 {
                    continue;
                }
                let first = scale(&mut b)?;
                scales[ch][sb] = match scfsi[ch][sb] {
                    0 => [first, scale(&mut b)?, scale(&mut b)?],
                    1 => [first, first, scale(&mut b)?],
                    2 => [first; 3],
                    _ => {
                        let second = scale(&mut b)?;
                        [first, second, second]
                    }
                };
            }
        }
        let mut slots = [[[0.; 32]; 2]; 36];
        for group in 0..12 {
            for sb in 0..limit {
                if levels[0][sb] == 0 && levels[1][sb] == 0 {
                    continue;
                }
                if sb >= h.joint_bound {
                    let values = triplet(&mut b, levels[0][sb])?;
                    for ch in 0..channels {
                        for sample in 0..3 {
                            slots[group * 3 + sample][ch][sb] =
                                values[sample] * scales[ch][sb][group / 4];
                        }
                    }
                } else {
                    for ch in 0..channels {
                        if levels[ch][sb] != 0 {
                            let values = triplet(&mut b, levels[ch][sb])?;
                            for sample in 0..3 {
                                slots[group * 3 + sample][ch][sb] =
                                    values[sample] * scales[ch][sb][group / 4];
                            }
                        }
                    }
                }
            }
        }
        // Parsing and all validation complete before committing synthesis history.
        let mut filters = self.filters.clone();
        let mut output = Vec::new();
        output
            .try_reserve_exact(1152 * channels)
            .map_err(|_| invalid("MP2 PCM allocation failed"))?;
        for slot in slots {
            let mut pcm = [[0.; 32]; 2];
            for ch in 0..channels {
                pcm[ch] = filters[ch].push(&slot[ch])?;
            }
            for sample in 0..32 {
                for channel in pcm.iter().take(channels) {
                    output.push(channel[sample] as f32);
                }
            }
        }
        self.filters = filters;
        self.free_base = base;
        Ok(output)
    }
}
