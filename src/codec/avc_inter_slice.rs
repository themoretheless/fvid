//! Progressive CAVLC mixed intra/inter slice iteration.
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
        })
    }
    /// Configure complete intra/inter entropy dispatch from actual parameter sets.
    pub fn new_mixed(
        header: &'a super::avc_slice::SliceHeader,
        sps: &'a super::avc::Sps,
        pps: &'a super::avc::Pps,
        memory_limit: usize,
    ) -> Result<Self> {
        if header.field_pic {
            return Err(invalid("field slices are not supported"));
        }
        let count = (sps.width_mbs as usize)
            .checked_mul(sps.height_map_units as usize)
            .ok_or_else(|| invalid("AVC context size overflow"))?;
        let extra = count
            .checked_mul(40)
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
            sps.height_map_units as usize,
            header.first_mb as usize,
            syntax,
            remaining,
        )?;
        reader.intra = Some(super::avc_macroblock::IntraCavlcReader::new_context(
            header, sps, pps, 65536,
        )?);
        Ok(reader)
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
        if self.pending > 0 {
            self.counts.store(address, 0, [0; 16], [[0; 4]; 2])?;
            if let Some(intra) = &mut self.intra {
                intra.record_inter(address, [0; 16], [[0; 4]; 2])?;
            }
            self.pending -= 1;
            self.address += 1;
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
            let block = intra.read_embedded(
                &mut probe,
                address as u32,
                self.syntax.previous_qp,
                code - offset,
            )?;
            let (luma, chroma) = intra.counts(address)?;
            self.counts.store(address, 0, luma, chroma)?;
            // I_PCM preserves the preceding QP for following macroblocks.
            if !matches!(block.luma, super::avc_macroblock::IntraLuma::Pcm { .. }) {
                self.syntax.previous_qp = block.qp;
            }
            self.bits = probe;
            self.address += 1;
            self.need_run = true;
            return Ok(Some(InterMacroblock::Intra(Box::new(block))));
        }
        let (header, coefficients) =
            self.counts
                .read_inter(&mut self.bits, address, 0, &self.syntax)?;
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
