//! H.265 F.10.1 extraction of a complete length-prefixed NAL sequence.
//! The caller supplies a dependency-closed layer list from the active VPS.
//! This does not rewrite additional layer sets (F.10.2/F.10.3).
use crate::codec::{
    config::NalUnits,
    hevc_nal::{NalHeader, NalRbsp},
};
use crate::{Result, invalid};

fn keep(
    nal: &[u8],
    h: NalHeader,
    layers: u64,
    temporal: u8,
    reduced: bool,
    budget: usize,
) -> Result<bool> {
    if h.layer_id == 63 || layers & (1u64 << h.layer_id) == 0 || h.temporal_id > temporal {
        return Ok(false);
    }
    if reduced && h.layer_id == 0 && matches!(h.unit_type, 39 | 40) {
        let rbsp = NalRbsp::parse(nal, budget)?;
        let mut at = 0usize;
        let mut timing = false;
        while at < rbsp.bytes.len() {
            if rbsp.bytes[at] == 128 && rbsp.bytes[at + 1..].iter().all(|&v| v == 0) {
                return Ok(!timing);
            }
            let mut code = || -> Result<usize> {
                let mut value = 0usize;
                loop {
                    let byte = *rbsp
                        .bytes
                        .get(at)
                        .ok_or_else(|| invalid("HEVC extraction truncated SEI field"))?;
                    at += 1;
                    value = value
                        .checked_add(byte as usize)
                        .ok_or_else(|| invalid("HEVC extraction SEI field overflow"))?;
                    if byte != 255 {
                        return Ok(value);
                    }
                }
            };
            let kind = code()?;
            let size = code()?;
            at = at
                .checked_add(size)
                .filter(|&end| end <= rbsp.bytes.len())
                .ok_or_else(|| invalid("HEVC extraction SEI payload exceeds RBSP"))?;
            // Scalable-nested messages are opaque: only top-level timing applies.
            timing |= matches!(kind, 0 | 1 | 130);
        }
        return Err(invalid("HEVC extraction SEI missing trailing bits"));
    }
    Ok(true)
}

/// Extract a complete supplied sequence, preserving retained NAL bytes and order.
/// Reduction is determined across the whole input, not independently per AU:
/// base-layer timing SEI may precede the first removed picture. To extract an
/// ongoing stream, first establish its global operation point; do not call this
/// separately per packet and assume packet-local timing metadata is sufficient.
/// Budget bounds additional output and temporary RBSP storage; input is borrowed.
pub fn extract_layered_substream(
    input: &[u8],
    length_size: u8,
    layers: u64,
    temporal: u8,
    budget: usize,
) -> Result<Vec<u8>> {
    if layers == 0 || layers >> 63 != 0 || temporal > 6 {
        return Err(invalid("invalid HEVC extraction target"));
    }
    let mut reduced = false;
    let mut scratch = 0usize;
    for nal in NalUnits::new(input, length_size)? {
        let nal = nal?;
        let h = NalHeader::parse(nal)?;
        reduced |= layers & (1u64 << h.layer_id) == 0 || h.temporal_id > temporal;
        if h.layer_id == 0
            && layers & 1 != 0
            && h.temporal_id <= temporal
            && matches!(h.unit_type, 39 | 40)
        {
            scratch = scratch.max(nal.len());
        }
    }
    let mut size = 0usize;
    for nal in NalUnits::new(input, length_size)? {
        let nal = nal?;
        if keep(
            nal,
            NalHeader::parse(nal)?,
            layers,
            temporal,
            reduced,
            budget,
        )? {
            size = size
                .checked_add(length_size as usize)
                .and_then(|s| s.checked_add(nal.len()))
                .ok_or_else(|| invalid("HEVC extraction output size overflow"))?;
        }
    }
    let overhead = std::mem::size_of::<Vec<u8>>() + std::mem::size_of::<NalRbsp>();
    let required = size
        .checked_add(if reduced { scratch } else { 0 })
        .and_then(|s| s.checked_add(overhead))
        .ok_or_else(|| invalid("HEVC extraction storage overflow"))?;
    if required > budget {
        return Err(invalid("HEVC extraction exceeds memory budget"));
    }
    let mut output = Vec::new();
    output
        .try_reserve_exact(size)
        .map_err(|_| invalid("cannot allocate HEVC extracted substream"))?;
    for nal in NalUnits::new(input, length_size)? {
        let nal = nal?;
        if keep(
            nal,
            NalHeader::parse(nal)?,
            layers,
            temporal,
            reduced,
            budget - size - overhead,
        )? {
            output.extend_from_slice(&(nal.len() as u32).to_be_bytes()[4 - length_size as usize..]);
            output.extend_from_slice(nal);
        }
    }
    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn pack(nals: &[&[u8]], length: u8) -> Vec<u8> {
        nals.iter()
            .flat_map(|n| {
                (n.len() as u32).to_be_bytes()[4 - length as usize..]
                    .iter()
                    .copied()
                    .chain(n.iter().copied())
                    .collect::<Vec<_>>()
            })
            .collect()
    }
    #[test]
    fn framing_opaque_discard_and_sei_message_walk() {
        let hdr = [78, 1, 144, 4, 0, 1, 0, 2, 128];
        // Full-NAL removal, including otherwise retained metadata in the same NAL.
        let mixed = [78, 1, 144, 4, 0, 1, 0, 2, 1, 0, 128];
        let future = [82, 9, 0, 0, 1]; // excluded layer 1: never de-escape invalid payload
        let valid = [2, 1, 128];
        for length in [1, 2, 4] {
            let input = pack(&[&hdr, &mixed, &future, &valid], length);
            assert_eq!(
                extract_layered_substream(&input, length, 1, 0, 4096).unwrap(),
                pack(&[&hdr, &valid], length)
            );
            let bad = pack(&[&[78, 1, 1, 255], &future], length);
            assert_eq!(
                extract_layered_substream(&bad, length, 1, 0, 4096)
                    .err()
                    .unwrap()
                    .to_string(),
                "HEVC extraction truncated SEI field"
            );
            let bad = pack(&[&[78, 1, 1, 5, 128], &future], length);
            assert_eq!(
                extract_layered_substream(&bad, length, 1, 0, 4096)
                    .err()
                    .unwrap()
                    .to_string(),
                "HEVC extraction SEI payload exceeds RBSP"
            );
        }
    }
}
