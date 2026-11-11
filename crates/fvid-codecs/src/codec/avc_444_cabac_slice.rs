//! Full-resolution residual dispatch shares the P/B arithmetic and motion state.
use super::super::{
    avc_inter_coefficients::InterCoefficients,
    avc_inter_slice::colour444::Macroblock,
    avc_macroblock::{IntraLuma, IntraMacroblock},
};
use super::*;
fn coefficients(mb: IntraMacroblock) -> Box<InterCoefficients> {
    Box::new(InterCoefficients {
        chroma422: None,
        luma_counts: mb
            .luma_levels
            .map(|block| block.iter().filter(|&&v| v != 0).count() as u8),
        luma4: mb.luma_levels,
        luma8: match mb.luma {
            IntraLuma::Blocks8 { levels, .. } => levels,
            _ => [[0; 64]; 4],
        },
        chroma_dc: [[0; 4]; 2],
        chroma_ac: [[[0; 16]; 4]; 2],
        chroma_counts: [[0; 4]; 2],
    })
}
impl<'a> InterCabacSlice<'a> {
    pub(crate) fn new_444(
        header: &'a SliceHeader,
        mono: &'a Sps,
        pps: &'a Pps,
        depth: u8,
        budget: usize,
    ) -> Result<Self> {
        Self::new_impl(
            header,
            mono,
            pps,
            budget,
            mono.mb_adaptive_frame_field && !header.field_pic,
            Some(depth),
        )
    }
    pub(crate) fn read_444_macroblock(&mut self) -> Result<Option<Macroblock>> {
        let Some(mb) = self.read_macroblock()? else {
            return Ok(None);
        };
        Ok(Some(match mb {
            InterMacroblock::Intra(y) => {
                let [cb, cr] = self.reader.take_444_components()?;
                Macroblock::Intra([*y, cb, cr])
            }
            InterMacroblock::Skip { address, qp } => {
                self.reader.record_444_skip(address)?;
                Macroblock::Skip { address, qp }
            }
            InterMacroblock::Coded {
                address,
                header,
                coefficients: y,
            } => {
                let [cb, cr] = self.reader.take_444_components()?;
                Macroblock::Coded {
                    address,
                    header,
                    coefficients: [y, coefficients(cb), coefficients(cr)],
                }
            }
        }))
    }
}
