//! HEVC I/P/B slice headers, reference lists, weights and escaped entry points.
use super::{
    bits::BitReader,
    hevc_cabac::SliceType,
    hevc_nal::{NalHeader, NalRbsp},
    hevc_pps::{Deblocking, Pps},
    hevc_rps::{self, ShortTermReference},
    hevc_sps::Sps,
};
use crate::{Result, invalid};
#[derive(Clone, Copy, Debug)]
pub struct Weight {
    pub values: [i16; 3],
    pub offsets: [i16; 3],
}
#[derive(Clone, Debug)]
pub struct Weights {
    pub high_precision_offsets: bool,
    pub denominators: [u8; 2],
    pub lists: [Vec<Weight>; 2],
}
#[derive(Clone, Debug)]
pub struct SliceHeader {
    /// Dependent segment inherits syntax/context from the preceding slice.
    pub dependent: bool,
    pub nal: NalHeader,
    pub slice_type: SliceType,
    pub poc_lsb: u32,
    pub short_term: Vec<ShortTermReference>,
    pub long_term: Vec<super::hevc_long_term::LongTermReference>,
    /// Slice-local RPS syntax only; zero for IDR or an SPS-selected set.
    pub short_term_bit_length: usize,
    /// Predictor's delta POC count before deriving the slice-local set.
    pub short_term_predictor_delta_pocs: usize,
    pub temporal_mvp: bool,
    pub references: [u8; 2],
    pub list_modification: [Option<Vec<u8>>; 2],
    pub mvd_l1_zero: bool,
    pub cabac_init: bool,
    pub collocated_list: usize,
    pub collocated_ref: u8,
    pub max_merge_candidates: u8,
    pub weights: Option<Weights>,
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
    /// Entropy substreams in de-escaped RBSP coordinates.
    pub entropy_substreams: Vec<std::ops::Range<usize>>,
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
    pub fn parameter_set_id(nal: &[u8]) -> Result<u8> {
        let payload = NalRbsp::parse(nal, nal.len())?;
        if !payload.header.is_vcl() {
            return Err(invalid("expected HEVC VCL NAL"));
        }
        let mut b = BitReader::new(&payload.bytes);
        b.bit()?;
        if payload.header.is_irap() {
            b.bit()?;
        }
        Ok(ue(&mut b, 63)? as u8)
    }
    pub fn parse_idr(nal: &[u8], sps: &Sps, pps: &Pps, budget: usize) -> Result<Self> {
        let header = Self::parse(nal, sps, pps, budget)?;
        if !header.nal.is_idr() || header.nal.temporal_id != 0 {
            return Err(invalid("expected temporal-zero HEVC IDR slice"));
        }
        Ok(header)
    }
    /// Read a dependent segment using an already validated preceding header.
    pub fn parse_with_previous(
        nal: &[u8],
        sps: &Sps,
        pps: &Pps,
        budget: usize,
        previous: Option<&Self>,
    ) -> Result<Self> {
        if !pps.dependent_slices || nal.get(2).is_none_or(|byte| byte & 0x80 != 0) {
            return Self::parse(nal, sps, pps, budget);
        }
        let payload = NalRbsp::parse(nal, budget)?;
        let b = &mut BitReader::new(&payload.bytes);
        let first = b.bit()?;
        let no_output = payload.header.is_irap() && b.bit()?;
        let id = ue(b, 63)? as u8;
        if first || !pps.dependent_slices {
            return Self::parse(nal, sps, pps, budget);
        }
        if !b.bit()? {
            return Self::parse(nal, sps, pps, budget);
        }
        let previous =
            previous.ok_or_else(|| invalid("HEVC dependent segment has no preceding header"))?;
        payload.header.require_base_layer()?;
        if id != pps.id
            || pps.sps_id != sps.id
            || id != previous.pps_id
            || payload.header.unit_type != previous.nal.unit_type
            || payload.header.temporal_id != previous.nal.temporal_id
            || no_output != previous.no_output_of_prior_pictures
        {
            return Err(invalid(
                "HEVC dependent segment disagrees with preceding header",
            ));
        }
        let side = 1u64 << sps.coding_block_log2[1];
        let count = u64::from(sps.dimensions[0])
            .div_ceil(side)
            .checked_mul(u64::from(sps.dimensions[1]).div_ceil(side))
            .filter(|&n| n > 0 && n <= u64::from(u32::MAX))
            .ok_or_else(|| invalid("HEVC CTU address space exceeds supported range"))?
            as u32;
        let address = b.read((32 - (count - 1).leading_zeros()) as u8)?;
        if address <= previous.address || address >= count {
            return Err(invalid("HEVC dependent segment address out of range"));
        }
        let (entry_point_offsets, extension, entropy_byte_offset, entropy_substreams) =
            entropy_tail(nal, &payload.bytes, b, pps, count, budget)?;
        let mut result = previous.clone();
        result.dependent = true;
        result.first = false;
        result.address = address;
        result.nal = payload.header;
        result.no_output_of_prior_pictures = no_output;
        result.entry_point_offsets = entry_point_offsets;
        result.extension = extension;
        result.entropy_byte_offset = entropy_byte_offset;
        result.entropy_substreams = entropy_substreams;
        result.rbsp = payload.bytes;
        Ok(result)
    }
    pub fn parse(nal: &[u8], sps: &Sps, pps: &Pps, budget: usize) -> Result<Self> {
        let payload = NalRbsp::parse(nal, budget)?;
        payload.header.require_base_layer()?;
        if !matches!(payload.header.unit_type,0..=9|16..=21)
            || payload.header.is_irap() && payload.header.temporal_id != 0
        {
            return Err(invalid("unsupported HEVC VCL type or IRAP temporal layer"));
        }
        if !payload.header.is_vcl() {
            return Err(invalid("expected HEVC VCL NAL"));
        }
        if pps.sps_id != sps.id {
            return Err(invalid("HEVC slice parameter set mismatch"));
        }
        let b = &mut BitReader::new(&payload.bytes);
        let first = b.bit()?;
        let no_output_of_prior_pictures = payload.header.is_irap() && b.bit()?;
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
        let slice_type = match ue(b, 2)? {
            0 => SliceType::B,
            1 => SliceType::P,
            _ => SliceType::I,
        };
        if payload.header.is_irap() && slice_type != SliceType::I {
            return Err(invalid("HEVC IRAP slice must be intra"));
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
        let mut poc_lsb = 0;
        let mut short_term = Vec::new();
        let mut long_term = Vec::new();
        let mut short_term_bit_length = 0;
        let mut short_term_predictor_delta_pocs = 0;
        let mut temporal_mvp = false;
        if !payload.header.is_idr() {
            poc_lsb = b.read(sps.poc_bits)?;
            if !b.bit()? {
                let syntax = hevc_rps::read_short_term_syntax(b, &sps.short_term, true, 15)?;
                short_term_bit_length = syntax.bit_length;
                short_term_predictor_delta_pocs = syntax.predictor_delta_pocs;
                short_term = syntax.references;
            } else {
                let index = if sps.short_term.len() > 1 {
                    b.read((usize::BITS - (sps.short_term.len() - 1).leading_zeros()) as u8)?
                        as usize
                } else {
                    0
                };
                short_term = sps
                    .short_term
                    .get(index)
                    .ok_or_else(|| invalid("HEVC slice RPS index out of range"))?
                    .clone();
            }
            if sps.long_term_present {
                let capacity = sps.ordering.last()
                    .ok_or_else(|| invalid("HEVC SPS has no DPB ordering"))?
                    .max_decoded_pictures.saturating_sub(1);
                let remaining = usize::from(capacity).checked_sub(short_term.len())
                    .ok_or_else(|| invalid("HEVC short-term reference count exceeds DPB"))?;
                long_term = super::hevc_long_term::read_long_term(
                    b, &sps.long_term, sps.poc_bits,
                    u8::try_from(remaining).map_err(|_| invalid("HEVC DPB capacity overflow"))?,
                )?;
            }
            temporal_mvp = sps.temporal_mvp && b.bit()?;
        }
        let chroma = sps.chroma_format != 0 && !sps.separate_colour_plane;
        let sao = if sps.sao {
            [b.bit()?, chroma && b.bit()?]
        } else {
            [false; 2]
        };
        let mut references = [0; 2];
        let mut list_modification = [None, None];
        let mut mvd_l1_zero = false;
        let mut cabac_init = false;
        let mut collocated_list = 0;
        let mut collocated_ref = 0;
        let mut max_merge_candidates = 0;
        let mut weights = None;
        if slice_type != SliceType::I {
            references = [
                pps.default_references[0],
                if slice_type == SliceType::B {
                    pps.default_references[1]
                } else {
                    0
                },
            ];
            if b.bit()? {
                references[0] = ue(b, 14)? as u8 + 1;
                if slice_type == SliceType::B {
                    references[1] = ue(b, 14)? as u8 + 1;
                }
            }
            let total = short_term.iter().filter(|r| r.used).count()
                + long_term.iter().filter(|r| r.used).count();
            if total == 0 {
                return Err(invalid("HEVC inter slice has no current references"));
            }
            if pps.lists_modification && total > 1 {
                let width = (usize::BITS - (total - 1).leading_zeros()) as u8;
                for list in 0..if slice_type == SliceType::B { 2 } else { 1 } {
                    if b.bit()? {
                        let mut entries = Vec::new();
                        for _ in 0..references[list] {
                            let v = b.read(width)?;
                            if v as usize >= total {
                                return Err(invalid("HEVC reference-list entry out of range"));
                            }
                            entries.push(v as u8);
                        }
                        list_modification[list] = Some(entries);
                    }
                }
            }
            mvd_l1_zero = slice_type == SliceType::B && b.bit()?;
            cabac_init = pps.cabac_init_present && b.bit()?;
            if temporal_mvp {
                if slice_type == SliceType::B {
                    collocated_list = usize::from(!b.bit()?);
                }
                if references[collocated_list] > 1 {
                    collocated_ref = ue(b, u32::from(references[collocated_list] - 1))? as u8;
                }
            }
            if (slice_type == SliceType::P && pps.weighted_prediction)
                || (slice_type == SliceType::B && pps.weighted_biprediction)
            {
                let luma = ue(b, 7)? as u8;
                let chroma_denom = if chroma {
                    i32::from(luma) + se(b, -7, 7)?
                } else {
                    i32::from(luma)
                };
                if !(0..=7).contains(&chroma_denom) {
                    return Err(invalid("HEVC chroma weight denominator out of range"));
                }
                let mut table = Weights {
                    high_precision_offsets: sps.high_precision_offsets,
                    denominators: [luma, chroma_denom as u8],
                    lists: [Vec::new(), Vec::new()],
                };
                let mut flags = 0;
                for list in 0..if slice_type == SliceType::B { 2 } else { 1 } {
                    let mut luma_flags = Vec::new();
                    let mut chroma_flags = vec![false; references[list] as usize];
                    for _ in 0..references[list] {
                        let flag = b.bit()?;
                        flags += usize::from(flag);
                        luma_flags.push(flag);
                    }
                    if chroma {
                        for flag in &mut chroma_flags {
                            *flag = b.bit()?;
                            flags += 2 * usize::from(*flag);
                        }
                    }
                    if flags > 24 {
                        return Err(invalid("HEVC prediction weight flags exceed limit"));
                    }
                    for i in 0..references[list] as usize {
                        let mut weight = Weight {
                            values: [1 << luma, 1 << chroma_denom, 1 << chroma_denom],
                            offsets: [0; 3],
                        };
                        if luma_flags[i] {
                            weight.values[0] += se(b, -128, 127)? as i16;
                            let half = 1i32
                                << if sps.high_precision_offsets {
                                    sps.depth[0] - 1
                                } else {
                                    7
                                };
                            weight.offsets[0] = se(b, -half, half - 1)? as i16;
                        }
                        if chroma_flags[i] {
                            for c in 1..3 {
                                weight.values[c] += se(b, -128, 127)? as i16;
                                let half = 1i32
                                    << if sps.high_precision_offsets {
                                        sps.depth[1] - 1
                                    } else {
                                        7
                                    };
                                let delta = se(b, -4 * half, 4 * half - 1)?;
                                weight.offsets[c] = (delta + half
                                    - ((half * i32::from(weight.values[c])) >> chroma_denom))
                                    .clamp(-half, half - 1)
                                    as i16;
                            }
                        }
                        table.lists[list].push(weight);
                    }
                }
                weights = Some(table);
            }
            max_merge_candidates = 5 - ue(b, 4)? as u8;
        }
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
        let (entry_point_offsets, extension, entropy_byte_offset, entropy_substreams) =
            entropy_tail(nal, &payload.bytes, b, pps, count, budget)?;
        Ok(Self {
            dependent: false,
            nal: payload.header,
            slice_type,
            poc_lsb,
            short_term,
            long_term,
            short_term_bit_length,
            short_term_predictor_delta_pocs,
            temporal_mvp,
            references,
            list_modification,
            mvd_l1_zero,
            cabac_init,
            collocated_list,
            collocated_ref,
            max_merge_candidates,
            weights,
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
            entropy_substreams,
        })
    }
}
type EntropyTail = (Vec<u64>, Vec<u8>, usize, Vec<std::ops::Range<usize>>);
fn entropy_tail(
    nal: &[u8],
    rbsp: &[u8],
    b: &mut BitReader<'_>,
    pps: &Pps,
    count: u32,
    budget: usize,
) -> Result<EntropyTail> {
    let mut entry_point_offsets = Vec::new();
    if pps.tiles.is_some() || pps.entropy_sync {
        let entries = ue(b, count - 1)? as usize;
        if entries
            .checked_mul(24)
            .and_then(|n| n.checked_add(rbsp.len()))
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
    // Entry-point offsets count emulation-prevention bytes, whereas CABAC
    // consumes RBSP. Map boundaries before dropping the escaped input.
    let escaped = &nal[2..];
    let mut zeros = 0;
    let mut rbsp_position = 0;
    let mut next_boundary = None;
    let mut entry = 0;
    let mut start = entropy_byte_offset;
    let mut entropy_substreams = Vec::with_capacity(entry_point_offsets.len() + 1);
    for (i, &byte) in escaped.iter().enumerate() {
        let escape = zeros == 2 && byte == 3;
        if !escape && rbsp_position == entropy_byte_offset && next_boundary.is_none() && entry == 0
        {
            if let Some(&length) = entry_point_offsets.first() {
                next_boundary = Some(
                    i.checked_add(
                        usize::try_from(length)
                            .map_err(|_| invalid("HEVC entry point overflow"))?,
                    )
                    .ok_or_else(|| invalid("HEVC entry point overflow"))?,
                );
            }
        }
        if next_boundary == Some(i) {
            if escape || rbsp_position <= start {
                return Err(invalid("HEVC entry point is inside an escape or empty"));
            }
            entropy_substreams.push(start..rbsp_position);
            start = rbsp_position;
            entry += 1;
            next_boundary = entry_point_offsets
                .get(entry)
                .map(|&length| {
                    usize::try_from(length)
                        .ok()
                        .and_then(|length| i.checked_add(length))
                        .ok_or_else(|| invalid("HEVC entry point overflow"))
                })
                .transpose()?;
        }
        if escape {
            zeros = 0;
        } else {
            zeros = if byte == 0 { zeros + 1 } else { 0 };
            rbsp_position += 1;
        }
    }
    if entry != entry_point_offsets.len() {
        return Err(invalid("HEVC entry point exceeds entropy payload"));
    }
    entropy_substreams.push(start..rbsp.len());
    Ok((
        entry_point_offsets,
        extension,
        entropy_byte_offset,
        entropy_substreams,
    ))
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
