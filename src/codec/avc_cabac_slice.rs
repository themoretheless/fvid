//! Progressive P/B and explicit MBAFF P CABAC slice iteration.
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
    mbaff: bool,
    field_picture: bool,
    previous_skipped: bool,
}
impl<'a> InterCabacSlice<'a> {
    pub fn new(header: &'a SliceHeader, sps: &'a Sps, pps: &'a Pps, budget: usize) -> Result<Self> {
        Self::new_impl(header, sps, pps, budget, false)
    }
    pub fn new_mbaff(
        header: &'a SliceHeader,
        sps: &'a Sps,
        pps: &'a Pps,
        budget: usize,
    ) -> Result<Self> {
        if !matches!(header.slice_type, SliceType::P | SliceType::B)
            || sps.frame_mbs_only
            || !sps.mb_adaptive_frame_field
        {
            return Err(invalid("MBAFF CABAC dispatcher requires a P/B frame slice"));
        }
        Self::new_impl(header, sps, pps, budget, true)
    }
    fn new_impl(
        header: &'a SliceHeader,
        sps: &'a Sps,
        pps: &'a Pps,
        budget: usize,
        mbaff: bool,
    ) -> Result<Self> {
        if !matches!(header.slice_type, SliceType::P | SliceType::B)
            && !(header.slice_type == SliceType::I && header.field_pic && !mbaff)
        {
            return Err(invalid("CABAC dispatcher requires P/B or an I field slice"));
        }
        let count = (sps.width_mbs as usize)
            .checked_mul(sps.height_map_units as usize)
            .and_then(|n| {
                n.checked_mul(if !sps.frame_mbs_only && !header.field_pic {
                    2
                } else {
                    1
                })
            })
            .filter(|n| *n > 0 && *n <= 65536)
            .ok_or_else(|| invalid("CABAC mixed picture geometry exceeds limits"))?;
        let remaining = budget
            .checked_sub(count * 64)
            .ok_or_else(|| invalid("CABAC mixed context budget exceeded"))?;
        let height = sps.height_map_units as usize
            * if !sps.frame_mbs_only && !header.field_pic {
                2
            } else {
                1
            };
        let motion = if mbaff {
            CabacMotionContexts::new_mbaff(sps.width_mbs as usize, height, remaining)?
        } else {
            CabacMotionContexts::new(sps.width_mbs as usize, height, remaining)?
        };
        let reader = if mbaff {
            IntraCabacReader::new_context_mbaff(header, sps, pps, count)?
        } else {
            IntraCabacReader::new_context(header, sps, pps, count)?
        };
        Ok(Self {
            reader,
            motion,
            states: crate::buffer(count)?,
            width: sps.width_mbs as usize,
            slice: header.slice_type,
            active: [header.refs_l0, header.refs_l1],
            failed: false,
            mbaff,
            field_picture: header.field_pic,
            previous_skipped: false,
        })
    }
    pub fn pair_field(&self, pair: usize) -> Option<bool> {
        self.reader.pair_field(pair)
    }
    pub fn field_decoding(&self) -> bool {
        self.reader.field_decoding()
    }
    fn inferred_field(&self, at: usize) -> bool {
        let pair = at / 2;
        if pair % self.width != 0 {
            if let Some(field) = self.pair_field(pair - 1) {
                return field;
            }
        }
        pair.checked_sub(self.width)
            .and_then(|pair| self.pair_field(pair))
            .unwrap_or(false)
    }
    fn field_context(&self, at: usize) -> [bool; 2] {
        let pair = at / 2;
        [
            pair % self.width != 0 && self.pair_field(pair - 1) == Some(true),
            pair.checked_sub(self.width)
                .and_then(|p| self.pair_field(p))
                == Some(true),
        ]
    }
    fn mbaff_neighbours(
        &self,
        at: usize,
        field: bool,
        skipped_top: Option<usize>,
    ) -> Result<[u8; 2]> {
        Ok(super::avc_mbaff::macroblock_neighbours(
            at,
            self.width,
            self.states.len() / self.width,
            true,
            [1, 1],
            |pair| {
                if pair == at / 2 {
                    Some(field)
                } else {
                    self.pair_field(pair)
                }
            },
        )?
        .map(|owner| {
            owner.map_or(0, |address| {
                if Some(address) == skipped_top {
                    1
                } else {
                    self.states[address]
                }
            })
        }))
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
        if self.slice == SliceType::I {
            // I slices own their context bank and have no skip/type prefix
            // from the inter tables. The intra reader also owns termination.
            return self.reader.read_macroblock().map(|block| {
                block.map(|block| InterMacroblock::Intra(Box::new(block)))
            });
        }
        if self.reader.is_finished() {
            return Ok(None);
        }
        let at = self.reader.address();
        if at >= self.states.len() {
            return Err(invalid("CABAC macroblock exceeds picture"));
        }
        let provisional = if self.mbaff && (at % 2 == 0 || self.previous_skipped) {
            self.inferred_field(at)
        } else {
            self.pair_field(at / 2).unwrap_or(false)
        };
        let neighbours = if self.mbaff {
            self.mbaff_neighbours(at, provisional, None)?
        } else {
            [
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
            ]
        };
        let skipped = syntax::skip(
            self.reader.arithmetic()?,
            self.slice,
            neighbours.map(|v| v >= 2),
        )?;
        if self.mbaff {
            if at % 2 == 0 {
                let field = if skipped {
                    // A skipped top is reconstructed only after the bottom's
                    // skip/field syntax determines the pair mode. Probe a copy
                    // of the arithmetic bank; the real bottom consumes it later.
                    if at + 1 >= self.states.len() {
                        return Err(invalid("MBAFF skipped top lacks bottom block"));
                    }
                    let neighbours = self.mbaff_neighbours(at + 1, provisional, Some(at))?;
                    let contexts = self.field_context(at);
                    let mut probe = self.reader.arithmetic()?.clone();
                    if syntax::skip(&mut probe, self.slice, neighbours.map(|v| v >= 2))? {
                        provisional
                    } else {
                        syntax::field_decoding_flag(&mut probe, contexts)?
                    }
                } else {
                    let contexts = self.field_context(at);
                    syntax::field_decoding_flag(self.reader.arithmetic()?, contexts)?
                };
                self.reader.set_pair_field(field)?;
            } else if self.previous_skipped && !skipped {
                let contexts = self.field_context(at);
                let field = syntax::field_decoding_flag(self.reader.arithmetic()?, contexts)?;
                if self.pair_field(at / 2) != Some(field) {
                    return Err(invalid("MBAFF CABAC pair probe disagrees"));
                }
            }
        }
        self.previous_skipped = skipped;
        let field = self.field_picture || self.mbaff && self.pair_field(at / 2) == Some(true);
        if skipped {
            if self.mbaff {
                self.motion.store_non_inter_mbaff(at, 0, field)?;
            } else {
                self.motion.store_non_inter(at, 0)?;
            }
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
        let neighbours = if self.mbaff {
            self.mbaff_neighbours(at, field, None)?
        } else {
            neighbours
        };
        let code = syntax::macroblock_type(
            self.reader.arithmetic()?,
            self.slice,
            neighbours.map(|v| v == 3),
        )?;
        let offset = if self.slice == SliceType::P { 5 } else { 23 };
        if code >= offset {
            if self.mbaff {
                self.motion.store_non_inter_mbaff(at, 0, field)?;
            } else {
                self.motion.store_non_inter(at, 0)?;
            }
            let block = self
                .reader
                .read_embedded(code - offset)?
                .ok_or_else(|| invalid("missing embedded CABAC intra block"))?;
            self.states[at] = 3;
            return Ok(Some(InterMacroblock::Intra(Box::new(block))));
        }
        let partitions = if self.mbaff {
            let (arithmetic, modes) = self.reader.arithmetic_with_pair_fields()?;
            self.motion.read_prediction_mbaff_for_slice(
                arithmetic,
                at,
                0,
                self.slice,
                code,
                self.active.map(|n| n * if field { 2 } else { 1 }),
                field,
                |p| modes.get(p).filter(|v| **v != 255).map(|v| *v != 0),
            )?
        } else {
            self.motion.read_prediction_field(
                self.reader.arithmetic()?,
                at,
                0,
                self.slice,
                code,
                self.active,
                self.field_picture,
            )?
        };
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
