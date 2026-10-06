//! Frame/MBAFF CAVLC mixed intra/inter slice iteration.
use super::{
    avc_coefficient_field::CoefficientField,
    avc_inter::{InterHeader, InterSyntax},
    avc_inter_coefficients::InterCoefficients,
    avc_slice::SliceType,
    bits::BitReader,
};
use crate::{Result, invalid};
pub enum InterMacroblock {
    Intra(Box<super::avc_macroblock::IntraMacroblock>),
    Skip {
        address: usize,
        qp: i32,
    },
    Coded {
        address: usize,
        header: InterHeader,
        coefficients: Box<InterCoefficients>,
    },
}
pub struct InterCavlcSlice<'a> {
    bits: BitReader<'a>,
    syntax: InterSyntax,
    counts: CoefficientField,
    intra: Option<super::avc_macroblock::IntraCavlcReader<'a>>,
    address: usize,
    limit: usize,
    pending: usize,
    need_run: bool,
    finished: bool,
    failed: bool,
    width: usize,
    mbaff: bool,
    pair_fields: Vec<Option<bool>>,
    previous_skipped: bool,
}
impl<'a> InterCavlcSlice<'a> {
    /// RBSP offset follows the slice header. Geometry is in macroblocks; FMO and
    /// field pictures are not supported. Budget covers the coefficient field.
    pub fn new(
        rbsp: &'a [u8],
        offset: usize,
        width: usize,
        height: usize,
        first: usize,
        syntax: InterSyntax,
        memory_limit: usize,
    ) -> Result<Self> {
        if !matches!(syntax.slice, SliceType::P | SliceType::B) || syntax.chroma_array_type != 1 {
            return Err(invalid(
                "inter slice reader requires progressive 4:2:0 P/B CAVLC",
            ));
        }
        let limit = width
            .checked_mul(height)
            .ok_or_else(|| invalid("AVC picture size overflow"))?;
        if first >= limit {
            return Err(invalid("AVC first macroblock outside picture"));
        }
        super::avc_residual_syntax::update_qp(syntax.previous_qp, 0, syntax.bit_depth)?;
        let counts = CoefficientField::new(width, height, memory_limit)?;
        let mut bits = BitReader::new(rbsp);
        bits.skip(offset)?;
        Ok(Self {
            bits,
            syntax,
            counts,
            intra: None,
            address: first,
            limit,
            pending: 0,
            need_run: true,
            finished: false,
            failed: false,
            width,
            mbaff: false,
            pair_fields: Vec::new(),
            previous_skipped: false,
        })
    }
    /// Configure complete intra/inter entropy dispatch from actual parameter sets.
    pub fn new_mixed(
        header: &'a super::avc_slice::SliceHeader,
        sps: &'a super::avc::Sps,
        pps: &'a super::avc::Pps,
        memory_limit: usize,
    ) -> Result<Self> {
        Self::new_mixed_impl(header, sps, pps, memory_limit, false)
    }
    /// Syntax-only MBAFF dispatcher; picture reconstruction is separate.
    pub fn new_mbaff(
        header: &'a super::avc_slice::SliceHeader,
        sps: &'a super::avc::Sps,
        pps: &'a super::avc::Pps,
        memory_limit: usize,
    ) -> Result<Self> {
        if sps.frame_mbs_only || !sps.mb_adaptive_frame_field {
            return Err(invalid("MBAFF inter reader requires adaptive frame slices"));
        }
        Self::new_mixed_impl(header, sps, pps, memory_limit, true)
    }
    fn new_mixed_impl(
        header: &'a super::avc_slice::SliceHeader,
        sps: &'a super::avc::Sps,
        pps: &'a super::avc::Pps,
        memory_limit: usize,
        mbaff: bool,
    ) -> Result<Self> {
        if header.field_pic {
            return Err(crate::unsupported("field slices are not supported"));
        }
        let count = (sps.width_mbs as usize)
            .checked_mul(sps.height_map_units as usize)
            .and_then(|n| n.checked_mul(if mbaff { 2 } else { 1 }))
            .ok_or_else(|| invalid("AVC context size overflow"))?;
        let extra = count
            .checked_mul(40)
            .and_then(|n| n.checked_add(if mbaff { count } else { 0 }))
            .ok_or_else(|| invalid("AVC context budget overflow"))?;
        let remaining = memory_limit
            .checked_sub(extra)
            .ok_or_else(|| invalid("mixed slice exceeds context budget"))?;
        let syntax = InterSyntax {
            slice: header.slice_type,
            active_references: [header.refs_l0, header.refs_l1],
            previous_qp: header.slice_qp,
            bit_depth: sps.bit_depth_luma,
            chroma_array_type: 1,
            transform8_enabled: pps.transform_8x8,
            direct8_inference: sps.direct_8x8_inference,
        };
        let mut reader = Self::new(
            &header.rbsp,
            header.header_bits,
            sps.width_mbs as usize,
            sps.height_map_units as usize * if mbaff { 2 } else { 1 },
            (header.first_mb as usize)
                .checked_mul(if mbaff { 2 } else { 1 })
                .ok_or_else(|| invalid("MBAFF first macroblock overflow"))?,
            syntax,
            remaining,
        )?;
        reader.intra = Some(if mbaff {
            super::avc_macroblock::IntraCavlcReader::new_context_mbaff(header, sps, pps, 65536)?
        } else {
            super::avc_macroblock::IntraCavlcReader::new_context(header, sps, pps, 65536)?
        });
        reader.mbaff = mbaff;
        if mbaff {
            reader
                .pair_fields
                .try_reserve_exact(count / 2)
                .map_err(|_| invalid("cannot allocate MBAFF pair modes"))?;
            reader.pair_fields.resize(count / 2, None);
        }
        Ok(reader)
    }
    pub fn pair_field(&self, pair: usize) -> Option<bool> {
        self.pair_fields.get(pair).copied().flatten()
    }
    pub fn field_decoding(&self) -> bool {
        self.mbaff && self.address > 0 && self.pair_field((self.address - 1) / 2) == Some(true)
    }
    pub fn bit_position(&self) -> usize {
        self.bits.position()
    }
    pub fn read_macroblock(&mut self) -> Result<Option<InterMacroblock>> {
        if self.failed {
            return Err(invalid("AVC inter slice reader previously failed"));
        }
        let result = self.read_next();
        if result.is_err() {
            self.failed = true;
        }
        result
    }
    fn read_next(&mut self) -> Result<Option<InterMacroblock>> {
        if self.finished {
            return Ok(None);
        }
        if self.pending == 0 {
            if !self.bits.more_rbsp_data() {
                if self.mbaff && self.address % 2 != 0 {
                    return Err(invalid("MBAFF inter slice ends with incomplete pair"));
                }
                self.bits.finish_rbsp()?;
                self.finished = true;
                return Ok(None);
            }
            if self.address == self.limit {
                return Err(invalid("AVC slice exceeds picture macroblocks"));
            }
            if self.need_run {
                self.pending = self.bits.unsigned_golomb()? as usize;
                if self.pending > self.limit - self.address {
                    return Err(invalid("AVC skip run exceeds picture"));
                }
                self.need_run = false;
                // Even run=0 requires a following macroblock, not trailing bits.
            }
        }
        let address = self.address;
        if self.mbaff {
            let pair = address / 2;
            if self.pending > 0 {
                if address % 2 == 0 {
                    let mode = if self.pending == 1 {
                        // The bottom is coded: defer top decoding until its flag
                        // is available, without consuming bottom syntax here.
                        if !self.bits.more_rbsp_data() {
                            return Err(invalid("MBAFF skipped top lacks coded bottom"));
                        }
                        let mut probe = self.bits.clone();
                        probe.bit()?
                    } else {
                        let left = if pair % self.width > 0 {
                            self.pair_field(pair - 1)
                        } else {
                            None
                        };
                        left.or_else(|| {
                            pair.checked_sub(self.width)
                                .and_then(|p| self.pair_field(p))
                        })
                        .unwrap_or(false)
                    };
                    self.pair_fields[pair] = Some(mode);
                }
            } else if address % 2 == 0 || self.previous_skipped {
                let mode = self.bits.bit()?;
                if self.pair_field(pair).is_some_and(|old| old != mode) {
                    return Err(invalid("MBAFF inter pair mode changed"));
                }
                self.pair_fields[pair] = Some(mode);
            }
            let mode = self
                .pair_field(pair)
                .ok_or_else(|| invalid("MBAFF inter lacks pair mode"))?;
            if let Some(intra) = &mut self.intra {
                intra.record_pair_mode(address, mode)?;
            }
        }
        if self.pending > 0 {
            self.counts.store(address, 0, [0; 16], [[0; 4]; 2])?;
            if let Some(intra) = &mut self.intra {
                intra.record_inter(address, [0; 16], [[0; 4]; 2])?;
            }
            self.pending -= 1;
            self.address += 1;
            self.previous_skipped = true;
            return Ok(Some(InterMacroblock::Skip {
                address,
                qp: self.syntax.previous_qp,
            }));
        }
        let mut probe = self.bits.clone();
        let code = probe.unsigned_golomb()?;
        let offset = if self.syntax.slice == SliceType::P {
            5
        } else {
            23
        };
        if code >= offset {
            let intra = self
                .intra
                .as_mut()
                .ok_or_else(|| invalid("intra context was not configured for mixed slice"))?;
            let block = if self.mbaff {
                intra.read_embedded_mbaff(
                    &mut probe,
                    address as u32,
                    self.syntax.previous_qp,
                    code - offset,
                    self.pair_fields[address / 2].unwrap(),
                )?
            } else {
                intra.read_embedded(
                    &mut probe,
                    address as u32,
                    self.syntax.previous_qp,
                    code - offset,
                )?
            };
            let (luma, chroma) = intra.counts(address)?;
            self.counts.store(address, 0, luma, chroma)?;
            // I_PCM preserves the preceding QP for following macroblocks.
            if !matches!(block.luma, super::avc_macroblock::IntraLuma::Pcm { .. }) {
                self.syntax.previous_qp = block.qp;
            }
            self.bits = probe;
            self.address += 1;
            self.need_run = true;
            self.previous_skipped = false;
            return Ok(Some(InterMacroblock::Intra(Box::new(block))));
        }
        let (header, coefficients) = if self.mbaff {
            let mut syntax = self.syntax;
            if self.pair_field(address / 2) == Some(true) {
                for count in &mut syntax.active_references {
                    *count = count
                        .checked_mul(2)
                        .ok_or_else(|| invalid("MBAFF reference count overflow"))?;
                }
            }
            let modes = &self.pair_fields;
            self.counts
                .read_inter_mbaff(&mut self.bits, address, 0, &syntax, |p| {
                    modes.get(p).copied().flatten()
                })?
        } else {
            self.counts
                .read_inter(&mut self.bits, address, 0, &self.syntax)?
        };
        if let Some(intra) = &mut self.intra {
            intra.record_inter(
                address,
                coefficients.luma_counts,
                coefficients.chroma_counts,
            )?;
        }
        self.syntax.previous_qp = header.residual.qp;
        self.address += 1;
        self.need_run = true;
        self.previous_skipped = false;
        Ok(Some(InterMacroblock::Coded {
            address,
            header,
            coefficients: Box::new(coefficients),
        }))
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn pair_reader(data: &[u8], width: usize, height: usize) -> InterCavlcSlice<'_> {
        let mut reader = InterCavlcSlice::new(data, 0, width, height, 0, syntax(), 8192).unwrap();
        reader.mbaff = true;
        reader.pair_fields = vec![None; width * height / 2];
        reader
    }
    #[test]
    fn skipped_top_looks_ahead_to_bottom_flag_without_consuming_it() {
        // skip_run=1, field=1, P16x16/ref0/MVD0/CBP0, RBSP stop.
        let mut r = pair_reader(&[0x5f, 0xc0], 1, 2);
        assert!(matches!(
            r.read_macroblock().unwrap(),
            Some(InterMacroblock::Skip { address: 0, .. })
        ));
        assert_eq!(r.bit_position(), 3);
        assert!(r.field_decoding());
        let Some(InterMacroblock::Coded {
            address, header, ..
        }) = r.read_macroblock().unwrap()
        else {
            panic!("expected coded bottom")
        };
        assert_eq!(address, 1);
        assert_eq!(header.partitions[0].references, [Some(0), None]);
        assert_eq!(r.bit_position(), 9);
        assert!(r.field_decoding());
        assert!(r.read_macroblock().unwrap().is_none());
    }
    #[test]
    fn fully_skipped_pairs_inherit_left_then_top_with_frame_default() {
        let mut empty = pair_reader(&[0x70], 1, 2);
        for _ in 0..2 {
            assert!(matches!(
                empty.read_macroblock().unwrap(),
                Some(InterMacroblock::Skip { .. })
            ));
            assert!(!empty.field_decoding());
        }
        assert!(empty.read_macroblock().unwrap().is_none());
        // Coded field top, then three skips including the next complete pair.
        for (width, height) in [(2, 2), (1, 4)] {
            let mut r = pair_reader(&[0xfe, 0x48], width, height);
            assert!(matches!(
                r.read_macroblock().unwrap(),
                Some(InterMacroblock::Coded { address: 0, .. })
            ));
            for address in 1..4 {
                assert!(
                    matches!(r.read_macroblock().unwrap(), Some(InterMacroblock::Skip { address: a, .. }) if a == address)
                );
                assert!(r.field_decoding());
            }
            assert_eq!(r.pair_field(1), Some(true));
            assert!(r.read_macroblock().unwrap().is_none());
        }
        let mut bad = pair_reader(&[0x50], 1, 2); // skip top, missing coded bottom
        assert!(bad.read_macroblock().is_err());
        assert!(bad.read_macroblock().is_err());
    }
    fn syntax() -> InterSyntax {
        InterSyntax {
            slice: SliceType::P,
            active_references: [1, 0],
            previous_qp: 26,
            bit_depth: 8,
            chroma_array_type: 1,
            transform8_enabled: false,
            direct8_inference: true,
        }
    }
    #[test]
    fn skipped_coded_skipped_and_rbsp_end() {
        // skip_run=1 (010), coded mb: 1111, skip_run=1 (010), stop=1.
        let mut r = InterCavlcSlice::new(&[0x5e, 0xa0], 0, 3, 1, 0, syntax(), 8192).unwrap();
        assert!(matches!(
            r.read_macroblock().unwrap(),
            Some(InterMacroblock::Skip { address: 0, .. })
        ));
        assert!(matches!(
            r.read_macroblock().unwrap(),
            Some(InterMacroblock::Coded { address: 1, .. })
        ));
        assert!(matches!(
            r.read_macroblock().unwrap(),
            Some(InterMacroblock::Skip { address: 2, .. })
        ));
        assert!(r.read_macroblock().unwrap().is_none());
        assert!(r.read_macroblock().unwrap().is_none());
    }
    #[test]
    fn bounded_skip_and_poisoned_error() {
        // skip_run=2 cannot fit a one-macroblock picture.
        let mut r = InterCavlcSlice::new(&[0x70], 0, 1, 1, 0, syntax(), 8192).unwrap();
        assert!(r.read_macroblock().is_err());
        assert!(r.read_macroblock().is_err());
        // skip_run=0, missing macroblock payload.
        let mut r = InterCavlcSlice::new(&[0x80], 0, 1, 1, 0, syntax(), 8192).unwrap();
        // A lone stop bit is an empty slice payload, accepted by iteration;
        // the picture assembler must reject missing picture coverage.
        assert!(r.read_macroblock().unwrap().is_none());
    }
}
