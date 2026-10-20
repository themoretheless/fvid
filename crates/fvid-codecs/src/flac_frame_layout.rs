//! Allocation-free structural delimiting of one FLAC frame in a byte prefix.
use super::{Bits, Config, parse_header, validate_config};
use crate::{Result, invalid};

fn skip(b: &mut Bits<'_>, bits: usize) -> Result<()> {
    let end = b
        .pos
        .checked_add(bits)
        .filter(|end| *end <= b.bytes.len() * 8)
        .ok_or_else(|| invalid("truncated FLAC payload"))?;
    b.pos = end;
    Ok(())
}

fn skip_unary(b: &mut Bits<'_>) -> Result<()> {
    loop {
        let byte = *b
            .bytes
            .get(b.pos / 8)
            .ok_or_else(|| invalid("truncated FLAC unary code"))?;
        let offset = b.pos % 8;
        let remaining = 8 - offset;
        let zeros = (byte << offset).leading_zeros() as usize;
        if zeros < remaining {
            b.pos += zeros + 1;
            return Ok(());
        }
        b.pos += remaining;
    }
}

// Decode several Rice code lengths from one lookahead word, without decoding
// residual values or repeatedly checking/reloading every individual bit.
fn rice_values(b: &mut Bits<'_>, k: usize, mut count: usize) -> Result<()> {
    while count != 0 {
        let bytes = b
            .bytes
            .get(b.pos / 8..)
            .ok_or_else(|| invalid("truncated FLAC Rice code"))?;
        let len = bytes.len().min(8);
        if len == 0 {
            return Err(invalid("truncated FLAC Rice code"));
        }
        let mut raw = [0u8; 8];
        raw[..len].copy_from_slice(&bytes[..len]);
        let offset = b.pos % 8;
        let mut word = u64::from_be_bytes(raw) << offset;
        let mut available = len * 8 - offset;
        let mut used = 0;
        while count != 0 {
            let length = word.leading_zeros() as usize + 1 + k;
            if length > available {
                break;
            }
            count -= 1;
            used += length;
            available -= length;
            if count == 0 || available == 0 {
                break;
            }
            word <<= length;
        }
        b.pos += used;
        if used == 0 {
            skip_unary(b)?;
            skip(b, k)?;
            count -= 1;
        }
    }
    Ok(())
}

const CRC16_TABLE: [u16; 256] = {
    let mut table = [0u16; 256];
    let mut slot = 0;
    while slot < 256 {
        let mut value = (slot as u16) << 8;
        let mut bit = 0;
        while bit < 8 {
            value = if value & 0x8000 != 0 {
                (value << 1) ^ 0x8005
            } else {
                value << 1
            };
            bit += 1;
        }
        table[slot] = value;
        slot += 1;
    }
    table
};
fn crc16(bytes: &[u8]) -> u16 {
    let mut value = 0u16;
    for &byte in bytes {
        value = (value << 8) ^ CRC16_TABLE[usize::from((value >> 8) as u8 ^ byte)];
    }
    value
}

fn residual(b: &mut Bits<'_>, frames: usize, order: usize) -> Result<()> {
    let method = b.read(2)?;
    if method > 1 {
        return Err(invalid("reserved FLAC residual method"));
    }
    let parameter_bits = 4 + method as usize;
    let partitions = 1usize << b.read(4)?;
    if !frames.is_multiple_of(partitions) || frames / partitions <= order {
        return Err(invalid("invalid FLAC residual partition geometry"));
    }
    let size = frames / partitions;
    for partition in 0..partitions {
        let count = size - if partition == 0 { order } else { 0 };
        let k = b.read(parameter_bits)? as usize;
        if k == (1 << parameter_bits) - 1 {
            let width = b.read(5)? as usize;
            skip(
                b,
                count
                    .checked_mul(width)
                    .ok_or_else(|| invalid("FLAC residual size overflow"))?,
            )?;
        } else {
            rice_values(b, k, count)?;
        }
    }
    Ok(())
}

fn subframe(b: &mut Bits<'_>, frames: usize, bits: usize) -> Result<()> {
    if b.read(1)? != 0 {
        return Err(invalid("nonzero FLAC subframe padding"));
    }
    let kind = b.read(6)?;
    let wasted = if b.read(1)? != 0 {
        let mut count = 1;
        while b.read(1)? == 0 {
            count += 1;
            if count >= bits {
                return Err(invalid("FLAC wasted bits exhaust sample width"));
            }
        }
        count
    } else {
        0
    };
    if wasted >= bits {
        return Err(invalid("FLAC wasted bits exhaust sample width"));
    }
    let width = bits - wasted;
    match kind {
        0 => skip(b, width),
        1 => skip(
            b,
            frames
                .checked_mul(width)
                .ok_or_else(|| invalid("FLAC verbatim size overflow"))?,
        ),
        8..=12 | 32..=63 => {
            let lpc = kind >= 32;
            let order = if lpc {
                kind as usize - 31
            } else {
                kind as usize - 8
            };
            if order >= frames {
                return Err(invalid("FLAC predictor order exceeds block"));
            }
            skip(b, order * width)?;
            if lpc {
                let precision = b.read(4)? as usize + 1;
                if precision == 16 {
                    return Err(invalid("reserved FLAC LPC precision"));
                }
                if b.signed(5)? < 0 {
                    return Err(invalid("negative FLAC LPC shift"));
                }
                skip(b, order * precision)?;
            }
            residual(b, frames, order)
        }
        _ => Err(invalid("reserved FLAC subframe type")),
    }
}

pub(super) fn frame_size(packet: &[u8], config: Config) -> Result<usize> {
    validate_config(config)?;
    if packet.len() < 8 {
        return Err(invalid("truncated FLAC frame"));
    }
    packet
        .len()
        .checked_mul(8)
        .ok_or_else(|| invalid("FLAC bit extent overflow"))?;
    let mut b = Bits {
        bytes: packet,
        pos: 0,
    };
    let header = parse_header(&mut b, config)?;
    for channel in 0..header.channels {
        let side = (header.assignment == 8 && channel == 1)
            || (header.assignment == 9 && channel == 0)
            || (header.assignment == 10 && channel == 1);
        subframe(
            &mut b,
            header.frames,
            usize::from(header.bits) + usize::from(side),
        )?;
    }
    while b.pos % 8 != 0 {
        if b.read(1)? != 0 {
            return Err(invalid("nonzero FLAC footer padding"));
        }
    }
    let end = (b.pos / 8)
        .checked_add(2)
        .filter(|end| *end <= packet.len())
        .ok_or_else(|| invalid("truncated FLAC frame CRC"))?;
    if crc16(&packet[..end]) != 0 {
        return Err(invalid("FLAC frame CRC mismatch"));
    }
    Ok(end)
}
