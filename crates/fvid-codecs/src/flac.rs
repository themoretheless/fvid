//! Owned FLAC frame reconstruction from RFC 9639. No codec backend.
use crate::{Result, invalid};
#[derive(Clone, Copy, Debug)]
pub struct Config {
    pub sample_rate: u32,
    pub channels: u16,
    pub bits: u8,
}
pub struct Frame {
    pub samples: Vec<i32>,
    pub frames: usize,
    pub config: Config,
}
struct Bits<'a> {
    bytes: &'a [u8],
    pos: usize,
}
impl Bits<'_> {
    fn read(&mut self, n: usize) -> Result<u64> {
        if n > 64
            || self
                .pos
                .checked_add(n)
                .is_none_or(|end| end > self.bytes.len() * 8)
        {
            return Err(invalid("truncated FLAC bits"));
        }
        let mut value = 0;
        for _ in 0..n {
            value = (value << 1) | u64::from((self.bytes[self.pos / 8] >> (7 - self.pos % 8)) & 1);
            self.pos += 1;
        }
        Ok(value)
    }
    fn signed(&mut self, n: usize) -> Result<i64> {
        if n == 0 {
            return Ok(0);
        }
        let value = self.read(n)?;
        Ok(((value << (64 - n)) as i64) >> (64 - n))
    }
    fn unary(&mut self) -> Result<u64> {
        let mut n = 0;
        while self.read(1)? == 0 {
            n += 1;
        }
        Ok(n)
    }
}
fn crc(bytes: &[u8], width: u32, poly: u32) -> u32 {
    let mut c = 0;
    let mask = (1 << width) - 1;
    for &v in bytes {
        c ^= u32::from(v) << (width - 8);
        for _ in 0..8 {
            c = ((c << 1) ^ if c & (1 << (width - 1)) != 0 { poly } else { 0 }) & mask;
        }
    }
    c
}
fn residual(b: &mut Bits<'_>, out: &mut [i64], order: usize) -> Result<()> {
    let method = b.read(2)?;
    if method > 1 {
        return Err(invalid("reserved FLAC residual method"));
    }
    let parameter_bits = 4 + method as usize;
    let partitions = 1usize << b.read(4)?;
    if !out.len().is_multiple_of(partitions) || out.len() / partitions <= order {
        return Err(invalid("invalid FLAC residual partition geometry"));
    }
    let size = out.len() / partitions;
    for partition in 0..partitions {
        let start = partition * size + if partition == 0 { order } else { 0 };
        let k = b.read(parameter_bits)? as usize;
        if k == (1 << parameter_bits) - 1 {
            let width = b.read(5)? as usize;
            for v in &mut out[start..(partition + 1) * size] {
                *v = b.signed(width)?;
            }
        } else {
            for v in &mut out[start..(partition + 1) * size] {
                let q = b.unary()?;
                let r = b.read(k)?;
                if q > (u64::from(u32::MAX) >> k) {
                    return Err(invalid("FLAC residual exceeds signed range"));
                }
                let folded = (q << k) | r;
                if folded >= u64::from(u32::MAX) {
                    return Err(invalid("FLAC residual exceeds signed range"));
                }
                *v = ((folded >> 1) as i64) ^ -((folded & 1) as i64);
            }
        }
    }
    Ok(())
}
fn subframe(b: &mut Bits<'_>, n: usize, bits: usize) -> Result<Vec<i64>> {
    if b.read(1)? != 0 {
        return Err(invalid("nonzero FLAC subframe padding"));
    }
    let kind = b.read(6)?;
    let wasted = if b.read(1)? != 0 {
        b.unary()?
            .checked_add(1)
            .ok_or_else(|| invalid("FLAC wasted bits overflow"))? as usize
    } else {
        0
    };
    if wasted >= bits {
        return Err(invalid("FLAC wasted bits exhaust sample width"));
    }
    let width = bits - wasted;
    let mut out = Vec::new();
    out.try_reserve_exact(n)
        .map_err(|_| invalid("FLAC sample allocation failed"))?;
    out.resize(n, 0);
    match kind {
        0 => {
            let v = b.signed(width)?;
            out.fill(v);
        }
        1 => {
            for v in &mut out {
                *v = b.signed(width)?;
            }
        }
        8..=12 | 32..=63 => {
            let lpc = kind >= 32;
            let order = if lpc {
                (kind - 31) as usize
            } else {
                (kind - 8) as usize
            };
            if order >= n {
                return Err(invalid("FLAC predictor order exceeds block"));
            }
            for v in &mut out[..order] {
                *v = b.signed(width)?;
            }
            let mut coefficients = Vec::new();
            let mut shift = 0;
            if lpc {
                let precision = b.read(4)? as usize + 1;
                if precision == 16 {
                    return Err(invalid("reserved FLAC LPC precision"));
                }
                shift = b.signed(5)?;
                if shift < 0 {
                    return Err(invalid("negative FLAC LPC shift"));
                }
                for _ in 0..order {
                    coefficients.push(b.signed(precision)?);
                }
            }
            residual(b, &mut out, order)?;
            for at in order..n {
                let prediction: i128 = if lpc {
                    coefficients
                        .iter()
                        .enumerate()
                        .map(|(j, &c)| i128::from(c) * i128::from(out[at - j - 1]))
                        .sum::<i128>()
                        >> shift
                } else {
                    match order {
                        0 => 0,
                        1 => i128::from(out[at - 1]),
                        2 => 2 * i128::from(out[at - 1]) - i128::from(out[at - 2]),
                        3 => {
                            3 * i128::from(out[at - 1]) - 3 * i128::from(out[at - 2])
                                + i128::from(out[at - 3])
                        }
                        4 => {
                            4 * i128::from(out[at - 1]) - 6 * i128::from(out[at - 2])
                                + 4 * i128::from(out[at - 3])
                                - i128::from(out[at - 4])
                        }
                        _ => unreachable!(),
                    }
                };
                out[at] = i64::try_from(prediction + i128::from(out[at]))
                    .map_err(|_| invalid("FLAC predictor overflow"))?;
                if out[at] < -(1i64 << (width - 1)) || out[at] >= (1i64 << (width - 1)) {
                    return Err(invalid("FLAC reconstructed sample exceeds width"));
                }
            }
        }
        _ => return Err(invalid("reserved FLAC subframe type")),
    }
    for v in &mut out {
        *v *= 1i64 << wasted;
    }
    Ok(out)
}
struct FrameHeader {
    frames: usize,
    assignment: u64,
    bits: u8,
    channels: u16,
}
fn validate_config(config: Config) -> Result<()> {
    if !(4..=32).contains(&config.bits)
        || !(1..=8).contains(&config.channels)
        || config.sample_rate == 0
        || config.sample_rate > 1048575
    {
        return Err(invalid("invalid FLAC stream geometry"));
    }
    Ok(())
}
fn parse_header(b: &mut Bits<'_>, config: Config) -> Result<FrameHeader> {
    if b.read(14)? != 0x3ffe || b.read(1)? != 0 {
        return Err(invalid("invalid FLAC sync or reserved bit"));
    }
    let variable = b.read(1)? != 0;
    let block = b.read(4)?;
    let rate = b.read(4)?;
    let assignment = b.read(4)?;
    let depth = b.read(3)?;
    if b.read(1)? != 0 {
        return Err(invalid("nonzero FLAC header reserved bit"));
    }
    let first = b.read(8)? as u8;
    let leading = first.leading_ones() as usize;
    let (mut number, extra) = if first < 128 {
        (u64::from(first), 0)
    } else if (2..=7).contains(&leading) {
        (u64::from(first & ((1 << (7 - leading)) - 1)), leading - 1)
    } else {
        return Err(invalid("invalid FLAC coded number"));
    };
    for _ in 0..extra {
        let byte = b.read(8)?;
        if byte & 0xc0 != 0x80 {
            return Err(invalid("invalid FLAC coded number continuation"));
        }
        number = (number << 6) | (byte & 63);
    }
    let minimum = match extra {
        0 => 0,
        1 => 128,
        2 => 2048,
        3 => 65536,
        4 => 2097152,
        5 => 67108864,
        6 => 2147483648,
        _ => unreachable!(),
    };
    if number < minimum
        || number
            > if variable {
                (1 << 36) - 1
            } else {
                (1 << 31) - 1
            }
    {
        return Err(invalid("noncanonical FLAC coded number"));
    }
    let frames = match block {
        1 => 192,
        2..=5 => 576usize << (block - 2),
        6 => b.read(8)? as usize + 1,
        7 => b.read(16)? as usize + 1,
        8..=15 => 256usize << (block - 8),
        _ => return Err(invalid("reserved FLAC block size")),
    };
    if frames > 65535 {
        return Err(invalid("FLAC block exceeds 65535 samples"));
    }
    let sample_rate = match rate {
        0 => config.sample_rate,
        1 => 88200,
        2 => 176400,
        3 => 192000,
        4 => 8000,
        5 => 16000,
        6 => 22050,
        7 => 24000,
        8 => 32000,
        9 => 44100,
        10 => 48000,
        11 => 96000,
        12 => b.read(8)? as u32 * 1000,
        13 => b.read(16)? as u32,
        14 => b.read(16)? as u32 * 10,
        _ => return Err(invalid("reserved FLAC sample rate")),
    };
    let bits = match depth {
        0 => config.bits,
        1 => 8,
        2 => 12,
        4 => 16,
        5 => 20,
        6 => 24,
        7 => 32,
        _ => return Err(invalid("reserved FLAC sample depth")),
    };
    let channels = match assignment {
        0..=7 => assignment as u16 + 1,
        8..=10 => 2,
        _ => return Err(invalid("reserved FLAC channel assignment")),
    };
    if channels != config.channels || sample_rate != config.sample_rate || bits != config.bits {
        return Err(invalid("FLAC frame geometry disagrees with stream"));
    }
    b.read(8)?;
    if crc(&b.bytes[..b.pos / 8], 8, 7) != 0 {
        return Err(invalid("FLAC header CRC mismatch"));
    }
    Ok(FrameHeader { frames, assignment, bits, channels })
}

#[path = "flac_frame_layout.rs"]
mod frame_layout;
/// Encoded size of the first frame in a byte prefix, without reconstructing PCM.
/// Checks entropy layout and header/footer CRC; later bytes belong to another
/// frame or container segment. Sample reconstruction checks remain in decode.
pub fn frame_size(packet: &[u8], config: Config) -> Result<usize> {
    frame_layout::frame_size(packet, config)
}

/// Coded sample count from a validated frame header, without reconstructing PCM.
/// Entropy and frame-footer integrity are checked separately by `decode`.
pub fn frame_samples(packet: &[u8], config: Config) -> Result<usize> {
    validate_config(config)?;
    let mut bits = Bits { bytes: packet, pos: 0 };
    Ok(parse_header(&mut bits, config)?.frames)
}

pub fn decode(packet: &[u8], config: Config) -> Result<Frame> {
    validate_config(config)?;
    if packet.len() < 8 {
        return Err(invalid("truncated FLAC frame"));
    }
    if crc(packet, 16, 0x8005) != 0 {
        return Err(invalid("FLAC frame CRC mismatch"));
    }
    let mut b = Bits {
        bytes: &packet[..packet.len() - 2],
        pos: 0,
    };
    let FrameHeader { frames, assignment, bits, channels } = parse_header(&mut b, config)?;
    let mut planes = Vec::new();
    for channel in 0..channels {
        let side = (assignment == 8 && channel == 1)
            || (assignment == 9 && channel == 0)
            || (assignment == 10 && channel == 1);
        planes.push(subframe(
            &mut b,
            frames,
            usize::from(bits) + usize::from(side),
        )?);
    }
    while b.pos % 8 != 0 {
        if b.read(1)? != 0 {
            return Err(invalid("nonzero FLAC footer padding"));
        }
    }
    if b.pos != b.bytes.len() * 8 {
        return Err(invalid("FLAC frame has trailing payload"));
    }
    let mut samples = Vec::new();
    samples
        .try_reserve_exact(frames * usize::from(channels))
        .map_err(|_| invalid("FLAC PCM allocation failed"))?;
    for at in 0..frames {
        if assignment >= 8 {
            let a = planes[0][at];
            let side = planes[1][at];
            let (left, right) = match assignment {
                8 => (a, a - side),
                9 => (a + side, side),
                10 => {
                    let mid = 2 * a + (side & 1);
                    ((mid + side) >> 1, (mid - side) >> 1)
                }
                _ => unreachable!(),
            };
            for v in [left, right] {
                if v < -(1i64 << (bits - 1)) || v >= (1i64 << (bits - 1)) {
                    return Err(invalid("FLAC stereo sample exceeds width"));
                }
                samples.push(v as i32);
            }
        } else {
            for plane in &planes {
                samples.push(plane[at] as i32);
            }
        }
    }
    Ok(Frame {
        samples,
        frames,
        config,
    })
}
