//! Allocation-free codebook admission before unpacking a Vorbis setup header.
use crate::vorbis::{PacketBits, SetupLimits, lookup1_values};
use crate::{Result, invalid};
#[derive(Clone, Copy)]
pub struct Admission {
    pub limits: SetupLimits,
    pub bytes: usize,
    pub max_dimension: usize,
}
fn add(total: &mut usize, n: usize) -> Result<()> {
    *total = total
        .checked_add(n)
        .ok_or_else(|| invalid("Vorbis setup admission overflow"))?;
    Ok(())
}
pub fn inspect(packet: &[u8]) -> Result<Admission> {
    let mut bits = PacketBits::new(packet);
    if bits.read(8)? != 5 {
        return Err(invalid("invalid Vorbis setup packet type"));
    }
    for b in b"vorbis" {
        if bits.read(8)? != u32::from(*b) {
            return Err(invalid("invalid Vorbis setup signature"));
        }
    }
    let count = bits.read(8)? as usize + 1;
    let (mut entries_total, mut nodes, mut values, mut max_dimension) =
        (0usize, 0usize, 0usize, 0usize);
    for _ in 0..count {
        if bits.read(24)? != 0x564342 {
            return Err(invalid("invalid Vorbis codebook sync"));
        }
        let dimension = bits.read(16)? as u16;
        let entries = bits.read(24)? as usize;
        if dimension == 0 || entries == 0 {
            return Err(invalid("empty Vorbis codebook geometry"));
        }
        max_dimension = max_dimension.max(usize::from(dimension));
        add(&mut entries_total, entries)?;
        let used = if bits.read(1)? == 0 {
            let sparse = bits.read(1)? != 0;
            let mut used = 0;
            for _ in 0..entries {
                if !sparse || bits.read(1)? != 0 {
                    bits.read(5)?;
                    used += 1;
                }
            }
            used
        } else {
            let mut length = bits.read(5)? + 1;
            let mut remaining = entries;
            while remaining != 0 {
                if length > 32 {
                    return Err(invalid("Vorbis ordered length exceeds 32"));
                }
                let width = (usize::BITS - remaining.leading_zeros()) as u8;
                let run = bits.read(width)? as usize;
                if run > remaining {
                    return Err(invalid("Vorbis ordered run exceeds entries"));
                }
                remaining -= run;
                length += 1;
            }
            entries
        };
        if used == 0 {
            return Err(invalid("empty Vorbis Huffman tree"));
        }
        // A valid complete binary tree has 2*used-1 nodes, with the single-entry
        // erratum represented by one node. Malformed trees remain parser errors.
        add(
            &mut nodes,
            used.checked_mul(2)
                .and_then(|n| n.checked_sub(1))
                .ok_or_else(|| invalid("Vorbis node count overflow"))?,
        )?;
        let kind = bits.read(4)?;
        if kind > 2 {
            return Err(invalid("reserved Vorbis lookup type"));
        }
        if kind != 0 {
            bits.skip(64)?;
            let width = bits.read(4)? as usize + 1;
            bits.read(1)?;
            let multiplicands = if kind == 1 {
                lookup1_values(entries as u32, dimension)? as usize
            } else {
                entries
                    .checked_mul(usize::from(dimension))
                    .ok_or_else(|| invalid("Vorbis multiplicand count overflow"))?
            };
            add(&mut values, multiplicands)?;
            bits.skip(
                multiplicands
                    .checked_mul(width)
                    .ok_or_else(|| invalid("Vorbis multiplicand bit count overflow"))?,
            )?;
        }
    }
    // Length lists, actual tree nodes and packed multiplicands. The fixed
    // reserve bounds all format-limited floor/residue/mapping/mode lists plus
    // their temporary setup representations (64 each, <=256 books).
    let mut bytes = 2 * 1024 * 1024;
    add(&mut bytes, entries_total)?;
    add(
        &mut bytes,
        nodes
            .checked_mul(crate::vorbis::huffman_node_bytes())
            .ok_or_else(|| invalid("Vorbis tree admission overflow"))?,
    )?;
    add(
        &mut bytes,
        values
            .checked_mul(2)
            .ok_or_else(|| invalid("Vorbis lookup admission overflow"))?,
    )?;
    Ok(Admission {
        limits: SetupLimits {
            entries: entries_total,
            nodes,
            multiplicands: values,
        },
        bytes,
        max_dimension,
    })
}
