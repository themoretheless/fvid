//! Shared full-resolution CAVLC syntax with independent component nC histories.
use super::super::{
    avc::{Pps, Sps},
    avc_macroblock::{IntraCavlcReader, IntraMacroblock},
    avc_slice::SliceHeader,
};
use super::*;
pub(crate) enum Macroblock {
    Intra([IntraMacroblock; 3]),
    Skip {
        address: usize,
        qp: i32,
    },
    Coded {
        address: usize,
        header: super::super::avc_inter::InterHeader,
        coefficients: [Box<InterCoefficients>; 3],
    },
}
pub(crate) struct Reader<'a> {
    inter: Option<InterCavlcSlice<'a>>,
    intra: Option<IntraCavlcReader<'a>>,
    bits: BitReader<'a>,
    components: [IntraCavlcReader<'a>; 2],
    counts: [CoefficientField; 2],
    mbaff: bool,
}
impl<'a> Reader<'a> {
    pub(crate) fn new(
        header: &'a SliceHeader,
        mono: &'a [Sps; 3],
        pps: &'a Pps,
        count: usize,
    ) -> Result<Self> {
        let mbaff = mono[0].mb_adaptive_frame_field && !header.field_pic;
        let inter = if header.slice_type == SliceType::I {
            None
        } else {
            Some((if mbaff {
                InterCavlcSlice::new_mbaff
            } else if header.field_pic {
                InterCavlcSlice::new_fmo
            } else {
                InterCavlcSlice::new_mixed
            })(header, &mono[0], pps, count * 4096)?)
        };
        let intra = if inter.is_none() {
            Some((if mbaff {
                IntraCavlcReader::new_mbaff
            } else {
                IntraCavlcReader::new
            })(header, &mono[0], pps, count)?)
        } else {
            None
        };
        let mut bits = BitReader::new(&header.rbsp);
        bits.skip(header.entropy_bit_offset)?;
        let component = if mbaff {
            IntraCavlcReader::new_context_mbaff
        } else {
            IntraCavlcReader::new_context
        };
        Ok(Self {
            inter,
            intra,
            bits,
            components: [
                component(header, &mono[1], pps, count)?,
                component(header, &mono[2], pps, count)?,
            ],
            mbaff,
            counts: [
                CoefficientField::new(
                    mono[0].width_mbs as usize,
                    count / mono[0].width_mbs as usize,
                    count * 4096,
                )?,
                CoefficientField::new(
                    mono[0].width_mbs as usize,
                    count / mono[0].width_mbs as usize,
                    count * 4096,
                )?,
            ],
        })
    }
    pub(crate) fn pair_field(&self, pair: usize) -> Option<bool> {
        self.inter
            .as_ref()
            .and_then(|r| r.pair_field(pair))
            .or_else(|| self.intra.as_ref().and_then(|r| r.pair_field(pair)))
    }
    pub(crate) fn field_decoding(&self) -> bool {
        self.inter.as_ref().map_or_else(
            || self.intra.as_ref().is_some_and(|r| r.field_decoding()),
            |r| r.field_decoding(),
        )
    }
    pub(crate) fn read(&mut self) -> Result<Option<Macroblock>> {
        let mb = if let Some(inter) = &mut self.inter {
            inter.read_macroblock()?
        } else {
            let intra = self
                .intra
                .as_mut()
                .ok_or_else(|| invalid("missing shared 444 entropy reader"))?;
            intra.resume_444(&self.bits);
            let mb = intra.read_macroblock()?;
            if mb.is_some() {
                self.bits.skip(
                    intra
                        .bit_position()
                        .checked_sub(self.bits.position())
                        .ok_or_else(|| invalid("444 entropy cursor moved backwards"))?,
                )?;
            }
            mb.map(|mb| InterMacroblock::Intra(Box::new(mb)))
        };
        let Some(mb) = mb else { return Ok(None) };
        let field_mode = self.field_decoding();
        let address = match &mb {
            InterMacroblock::Intra(m) => m.address as usize,
            InterMacroblock::Skip { address, .. } | InterMacroblock::Coded { address, .. } => {
                *address
            }
        };
        if self.mbaff {
            for component in &mut self.components {
                component.record_pair_mode(address, field_mode)?;
            }
        }
        let intra_context = self.intra.as_ref();
        let (bits, pairs) = if let Some(inter) = &mut self.inter {
            (&mut inter.bits, Some(inter.pair_fields.as_slice()))
        } else {
            (&mut self.bits, None)
        };
        Ok(Some(match mb {
            InterMacroblock::Intra(y) => {
                let cb = self.components[0].read_444_component(bits, &y)?;
                let cr = self.components[1].read_444_component(bits, &y)?;
                for (p, mb) in [&cb, &cr].into_iter().enumerate() {
                    let (counts, _) = self.components[p].counts(mb.address as usize)?;
                    self.counts[p].store(mb.address as usize, 0, counts, [[0; 4]; 2])?;
                }
                Macroblock::Intra([*y, cb, cr])
            }
            InterMacroblock::Skip { address, qp } => {
                for p in 0..2 {
                    self.counts[p].store(address, 0, [0; 16], [[0; 4]; 2])?;
                    self.components[p].advance_444_inter(address, [0; 16])?;
                }
                Macroblock::Skip { address, qp }
            }
            InterMacroblock::Coded {
                address,
                header,
                coefficients,
            } => {
                let mut planes = Vec::with_capacity(2);
                for p in 0..2 {
                    let neighbours = if self.mbaff {
                        self.counts[p].neighbours_mbaff(address, 0, |pair| {
                            pairs
                                .and_then(|m| m.get(pair).copied().flatten())
                                .or_else(|| intra_context.and_then(|r| r.pair_field(pair)))
                        })?
                    } else {
                        self.counts[p].neighbours(address, 0)?
                    };
                    let c = super::super::avc_inter_coefficients::read_inter_coefficients_field(
                        bits,
                        header.residual.pattern,
                        header.residual.transform8,
                        neighbours,
                        field_mode,
                    )?;
                    self.counts[p].store(address, 0, c.luma_counts, [[0; 4]; 2])?;
                    self.components[p].advance_444_inter(address, c.luma_counts)?;
                    planes.push(Box::new(c));
                }
                let cr = planes.pop().unwrap();
                let cb = planes.pop().unwrap();
                Macroblock::Coded {
                    address,
                    header,
                    coefficients: [coefficients, cb, cr],
                }
            }
        }))
    }
}
