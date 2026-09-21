//! Progressive mixed CABAC P/B slice iteration.
use super::{
    avc::{Pps, Sps},
    avc_cabac_inter as syntax,
    avc_cabac_macroblock::{InterNeighbourContext, IntraCabacReader},
    avc_cabac_motion::CabacMotionContexts,
    avc_inter::InterHeader,
    avc_inter_slice::InterMacroblock,
    avc_slice::{SliceHeader, SliceType},
};
use crate::{Result, invalid};
pub struct InterCabacSlice<'a> {
    reader: IntraCabacReader<'a>,
    motion: CabacMotionContexts,
    states: Vec<u8>,
    width: usize,
    slice: SliceType,
    active: [u32; 2],
    failed: bool,
}
impl<'a> InterCabacSlice<'a> {
    pub fn new(header: &'a SliceHeader, sps: &'a Sps, pps: &'a Pps, budget: usize) -> Result<Self> {
        if !matches!(header.slice_type, SliceType::P | SliceType::B) {
            return Err(invalid("CABAC mixed slice requires P/B"));
        }
        let count = (sps.width_mbs as usize)
            .checked_mul(sps.height_map_units as usize)
            .filter(|n| *n > 0 && *n <= 65536)
            .ok_or_else(|| invalid("CABAC mixed picture geometry exceeds limits"))?;
        let remaining = budget
            .checked_sub(count * 64)
            .ok_or_else(|| invalid("CABAC mixed context budget exceeded"))?;
        let motion = CabacMotionContexts::new(
            sps.width_mbs as usize,
            sps.height_map_units as usize,
            remaining,
        )?;
        let reader = IntraCabacReader::new_context(header, sps, pps, count)?;
        Ok(Self {
            reader,
            motion,
            states: crate::buffer(count)?,
            width: sps.width_mbs as usize,
            slice: header.slice_type,
            active: [header.refs_l0, header.refs_l1],
            failed: false,
        })
    }
    pub fn read_macroblock(&mut self) -> Result<Option<InterMacroblock>> {
        if self.failed {
            return Err(invalid("CABAC mixed slice previously failed"));
        }
        let result = self.read_next();
        if result.is_err() {
            self.failed = true;
        }
        result
    }
    fn read_next(&mut self) -> Result<Option<InterMacroblock>> {
        if self.reader.is_finished() {
            return Ok(None);
        }
        let at = self.reader.address();
        if at >= self.states.len() {
            return Err(invalid("CABAC macroblock exceeds picture"));
        }
        let neighbours = [
            if at % self.width > 0 {
                self.states[at - 1]
            } else {
                0
            },
            if at >= self.width {
                self.states[at - self.width]
            } else {
                0
            },
        ];
        if syntax::skip(
            self.reader.arithmetic()?,
            self.slice,
            neighbours.map(|v| v >= 2),
        )? {
            self.motion.store_non_inter(at, 0)?;
            let qp = self.reader.qp();
            self.reader.record_inter(&InterNeighbourContext {
                qp,
                qp_delta_nonzero: false,
                pattern: 0,
                transform8: false,
                luma_coded: [false; 16],
                chroma_dc: [false; 2],
                chroma_ac: [[false; 4]; 2],
            })?;
            self.states[at] = 1;
            return Ok(Some(InterMacroblock::Skip { address: at, qp }));
        }
        let code = syntax::macroblock_type(
            self.reader.arithmetic()?,
            self.slice,
            neighbours.map(|v| v == 3),
        )?;
        let offset = if self.slice == SliceType::P { 5 } else { 23 };
        if code >= offset {
            self.motion.store_non_inter(at, 0)?;
            let block = self
                .reader
                .read_embedded(code - offset)?
                .ok_or_else(|| invalid("missing embedded CABAC intra block"))?;
            self.states[at] = 3;
            return Ok(Some(InterMacroblock::Intra(Box::new(block))));
        }
        let partitions = self.motion.read_prediction(
            self.reader.arithmetic()?,
            at,
            0,
            self.slice,
            code,
            self.active,
        )?;
        let (residual, coefficients) = self.reader.read_inter_residual(&partitions)?;
        self.states[at] = if self.slice == SliceType::B && code == 0 {
            2
        } else {
            3
        };
        Ok(Some(InterMacroblock::Coded {
            address: at,
            header: InterHeader {
                mb_type: u32::from(code),
                partitions,
                residual,
            },
            coefficients: Box::new(coefficients),
        }))
    }
}
