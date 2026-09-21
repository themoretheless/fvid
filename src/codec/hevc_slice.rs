//! HEVC slice-header parsing. Initial IDR path; entropy data remains untouched.
use super::{
    bits::BitReader,
    hevc_nal::{NalHeader, NalRbsp},
    hevc_pps::{Deblocking, Pps},
    hevc_sps::Sps,
};
use crate::{Result, invalid};
#[derive(Clone, Debug)]
pub struct SliceHeader {
    pub nal: NalHeader,
    pub first: bool,
    pub no_output_of_prior_pictures: bool,
    pub pps_id: u8,
    pub address: u32,
    pub picture_output: bool,
    pub colour_plane: u8,
    pub sao: [bool; 2],
    pub qp: i32,
    /// Effective PPS + slice offsets.
    pub chroma_qp_offsets: [i8; 2],
    pub deblocking: Deblocking,
    pub loop_filter_across_slices: bool,
    /// Byte lengths in escaped NAL data, including any emulation-prevention bytes.
    /// These must be translated to RBSP positions before splitting substreams.
    pub entry_point_offsets: Vec<u64>,
    pub extension: Vec<u8>,
    pub rbsp: Vec<u8>,
    pub entropy_byte_offset: usize,
}
fn ue(b: &mut BitReader<'_>, max: u32) -> Result<u32> {
    let n = b.unsigned_golomb()?;
    if n > max {
        return Err(invalid("HEVC slice value exceeds range"));
    }
    Ok(n)
}
fn se(b: &mut BitReader<'_>, min: i32, max: i32) -> Result<i32> {
    let n = b.signed_golomb()?;
    if !(min..=max).contains(&n) {
        return Err(invalid("HEVC slice signed value exceeds range"));
    }
    Ok(n)
}
impl SliceHeader {
    pub fn parse_idr(nal: &[u8], sps: &Sps, pps: &Pps, budget: usize) -> Result<Self> {
        let payload = NalRbsp::parse(nal, budget)?;
        payload.header.require_base_layer()?;
        if !payload.header.is_idr() || payload.header.temporal_id != 0 {
            return Err(invalid("expected temporal-zero HEVC IDR slice"));
        }
        if pps.sps_id != sps.id {
            return Err(invalid("HEVC slice parameter set mismatch"));
        }
        let b = &mut BitReader::new(&payload.bytes);
        let first = b.bit()?;
        let no_output_of_prior_pictures = b.bit()?;
        let pps_id = ue(b, 63)? as u8;
        if pps_id != pps.id {
            return Err(invalid("HEVC slice references another PPS"));
        }
        let side = 1u64 << sps.coding_block_log2[1];
        let count = u64::from(sps.dimensions[0])
            .div_ceil(side)
            .checked_mul(u64::from(sps.dimensions[1]).div_ceil(side))
            .filter(|&n| n > 0 && n <= u64::from(u32::MAX))
            .ok_or_else(|| invalid("HEVC CTU address space exceeds supported range"))?
            as u32;
        let address = if first {
            0
        } else {
            if pps.dependent_slices && b.bit()? {
                return Err(invalid(
                    "dependent HEVC slice segments need inherited header",
                ));
            }
            let width = (32 - (count - 1).leading_zeros()) as u8;
            let address = b.read(width)?;
            if address == 0 || address >= count {
                return Err(invalid("HEVC slice address out of range"));
            }
            address
        };
        b.skip(usize::from(pps.extra_slice_header_bits))?;
        if ue(b, 2)? != 2 {
            return Err(invalid("HEVC IDR slice must be intra"));
        }
        let picture_output = if pps.output_flag_present {
            b.bit()?
        } else {
            true
        };
        let colour_plane = if sps.separate_colour_plane {
            b.read(2)? as u8
        } else {
            0
        };
        if colour_plane > 2 {
            return Err(invalid("invalid HEVC colour plane"));
        }
        let chroma = sps.chroma_format != 0 && !sps.separate_colour_plane;
        let sao = if sps.sao {
            [b.bit()?, chroma && b.bit()?]
        } else {
            [false; 2]
        };
        let min_qp = -6 * (i32::from(sps.depth[0]) - 8);
        let qp = pps
            .initial_qp
            .checked_add(b.signed_golomb()?)
            .filter(|n| (min_qp..=51).contains(n))
            .ok_or_else(|| invalid("HEVC slice QP outside bit-depth range"))?;
        let mut chroma_qp_offsets = pps.chroma_qp_offsets;
        if pps.slice_chroma_qp_offsets {
            for value in &mut chroma_qp_offsets {
                let sum = i32::from(*value) + se(b, -12, 12)?;
                if !(-12..=12).contains(&sum) {
                    return Err(invalid("HEVC combined chroma QP offset out of range"));
                }
                *value = sum as i8;
            }
        }
        let mut deblocking = pps.deblocking;
        if deblocking.override_enabled && b.bit()? {
            deblocking.disabled = b.bit()?;
            deblocking.offsets_div2 = if deblocking.disabled {
                [0; 2]
            } else {
                [se(b, -6, 6)? as i8, se(b, -6, 6)? as i8]
            };
        }
        let loop_filter_across_slices =
            if pps.loop_filter_across_slices && (sao[0] || sao[1] || !deblocking.disabled) {
                b.bit()?
            } else {
                pps.loop_filter_across_slices
            };
        let mut entry_point_offsets = Vec::new();
        if pps.tiles.is_some() || pps.entropy_sync {
            let entries = ue(b, count - 1)? as usize;
            if entries
                .checked_mul(8)
                .and_then(|n| n.checked_add(payload.bytes.len()))
                .is_none_or(|n| n > budget)
            {
                return Err(invalid("HEVC entry-point metadata exceeds budget"));
            }
            if entries > 0 {
                let width = ue(b, 31)? as u8 + 1;
                entry_point_offsets
                    .try_reserve_exact(entries)
                    .map_err(|_| invalid("cannot allocate HEVC entry points"))?;
                for _ in 0..entries {
                    entry_point_offsets.push(u64::from(b.read(width)?) + 1);
                }
            }
        }
        let mut extension = Vec::new();
        if pps.slice_header_extension {
            let length = ue(b, 256)? as usize;
            for _ in 0..length {
                extension.push(b.read(8)? as u8);
            }
        }
        if !b.bit()? {
            return Err(invalid("missing HEVC header alignment bit"));
        }
        while b.position() % 8 != 0 {
            if b.bit()? {
                return Err(invalid("nonzero HEVC header alignment padding"));
            }
        }
        let entropy_byte_offset = b.position() / 8;
        if b.remaining() < 8 {
            return Err(invalid("missing HEVC entropy payload"));
        }
        Ok(Self {
            nal: payload.header,
            first,
            no_output_of_prior_pictures,
            pps_id,
            address,
            picture_output,
            colour_plane,
            sao,
            qp,
            chroma_qp_offsets,
            deblocking,
            loop_filter_across_slices,
            entry_point_offsets,
            extension,
            rbsp: payload.bytes,
            entropy_byte_offset,
        })
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn real_idr_header_matches_trace_and_keeps_cabac_bytes() {
        let hex =
            "42010101600000030090000003000003001ea020810596566924caf0168080000003008000000c84";
        let bytes: Vec<_> = (0..hex.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&hex[i..i + 2], 16).unwrap())
            .collect();
        let sps = Sps::parse(&bytes, 1024).unwrap();
        let pps = Pps::parse(&[0x44, 1, 0xc1, 0x72, 0xb4, 0x22, 0x40], &sps, 1024).unwrap();
        let nal = [
            0x28, 1, 0xaf, 0x1d, 0x80, 0xf7, 1, 0x5b, 0xd6, 0xbe, 0xcc, 0xf0,
        ];
        let h = SliceHeader::parse_idr(&nal, &sps, &pps, 1024).unwrap();
        assert!(h.first && h.picture_output && h.loop_filter_across_slices);
        assert_eq!(h.qp, 33);
        assert_eq!(h.sao, [true, true]);
        assert_eq!(h.entropy_byte_offset, 3);
        assert_eq!(&h.rbsp[h.entropy_byte_offset..], &nal[5..]);
        for end in 0..6 {
            assert!(SliceHeader::parse_idr(&nal[..end], &sps, &pps, 1024).is_err());
        }
        let mut bad = nal;
        bad[4] = 0;
        assert!(SliceHeader::parse_idr(&bad, &sps, &pps, 1024).is_err());
        bad = nal;
        bad[4] = 0x81;
        assert!(SliceHeader::parse_idr(&bad, &sps, &pps, 1024).is_err());
    }
}
