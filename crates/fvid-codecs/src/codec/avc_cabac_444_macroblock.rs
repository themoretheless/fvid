//! Shared CABAC intra headers with independent full-resolution component residual state.
use super::*;

pub(super) struct State {
    depth: u8,
    dc: [Vec<u8>; 2],
    coded: [Vec<u8>; 2],
    pending: Option<[IntraMacroblock; 2]>,
}
impl<'a> IntraCabacReader<'a> {
    /// 9.3.3.1.1.9: an 8x8 coded-block context cannot use a neighbouring
    /// 4x4 transform block. PCM and slice-unavailable neighbours infer one.
    pub(super) fn coded_context_4448(
        &self,
        grid: &[u8],
        x: usize,
        y: usize,
        intra: bool,
    ) -> Result<u8> {
        let w = self.sps.width_mbs as usize;
        if w == 0
            || grid.len() != self.types.len() * 16
            || x >= w * 4
            || y >= self.types.len() / w * 4
        {
            return Err(invalid("invalid 4:4:4 frame/field 8x8 context geometry"));
        }
        let neighbours = if self.mbaff {
            super::super::avc_mbaff::block_neighbours(
                self.address,
                [x % 4, y % 4],
                w,
                self.types.len() / w,
                true,
                [1, 1],
                |pair| self.pair_field(pair),
            )?
        } else {
            [
                x.checked_sub(1).map(|a| (a, y)),
                y.checked_sub(1).map(|b| (x, b)),
            ]
            .map(|n| n.map(|(nx, ny)| (ny / 4 * w + nx / 4, [nx % 4, ny % 4])))
        };
        let mut context = 0;
        for (direction, neighbour) in neighbours.into_iter().enumerate() {
            let flag = if let Some((owner, local)) = neighbour {
                if self.types[owner] == 255 {
                    u8::from(intra)
                } else if self.types[owner] == 25 {
                    1
                } else if self.eight[owner] != 1 {
                    0
                } else {
                    u8::from(grid[owner * 16 + local[1] * 4 + local[0]] == 1)
                }
            } else {
                u8::from(intra)
            };
            context |= flag << direction;
        }
        Ok(context)
    }

    pub(crate) fn new_444(
        header: &'a SliceHeader,
        mono: &'a Sps,
        pps: &'a Pps,
        max: usize,
        chroma_depth: u8,
    ) -> Result<Self> {
        if mono.chroma_format != 0
            || (mono.frame_mbs_only && header.field_pic)
            || !matches!(
                header.slice_type,
                SliceType::I | SliceType::P | SliceType::B
            )
            || !(8..=14).contains(&chroma_depth)
        {
            return Err(invalid(
                "invalid CABAC 4:4:4 frame/field component projection",
            ));
        }
        let mut reader = if mono.mb_adaptive_frame_field && !header.field_pic {
            Self::new_context_mbaff(header, mono, pps, max)?
        } else {
            Self::new_context(header, mono, pps, max)?
        };
        reader.cabac = AvcCabac::new_444(
            &header.rbsp,
            header.entropy_bit_offset,
            header.slice_type,
            header.cabac_init_idc as u8,
            header.slice_qp,
        )?;
        let count = reader.types.len();
        let grid = |n| -> Result<Vec<u8>> {
            let mut v = crate::buffer(n)?;
            v.fill(255);
            Ok(v)
        };
        reader.colour444 = Some(Box::new(State {
            depth: chroma_depth,
            dc: [grid(count)?, grid(count)?],
            coded: [grid(count * 16)?, grid(count * 16)?],
            pending: None,
        }));
        Ok(reader)
    }
    pub(crate) fn take_444_components(&mut self) -> Result<[IntraMacroblock; 2]> {
        self.colour444
            .as_mut()
            .and_then(|state| state.pending.take())
            .ok_or_else(|| invalid("missing CABAC 444 component residuals"))
    }
    pub(crate) fn record_444_skip(&mut self, address: usize) -> Result<()> {
        let state = self
            .colour444
            .as_mut()
            .ok_or_else(|| invalid("missing CABAC 444 skip contexts"))?;
        if address >= state.dc[0].len() || state.pending.is_some() {
            return Err(invalid("invalid CABAC 444 skip position"));
        }
        for p in 0..2 {
            state.dc[p][address] = 0;
            state.coded[p][address * 16..address * 16 + 16].fill(0);
        }
        Ok(())
    }
    pub(crate) fn read_444_macroblock(&mut self) -> Result<Option<[IntraMacroblock; 3]>> {
        if self.colour444.is_none() {
            return Err(invalid("missing CABAC 4:4:4 component state"));
        }
        let Some(y) = self.read_macroblock()? else {
            return Ok(None);
        };
        let [cb, cr] = self
            .colour444
            .as_mut()
            .and_then(|state| state.pending.take())
            .ok_or_else(|| invalid("missing CABAC 4:4:4 residual planes"))?;
        Ok(Some([y, cb, cr]))
    }
    pub(super) fn pcm_444(&mut self, mb: &mut IntraMacroblock) -> Result<()> {
        let state = self
            .colour444
            .as_mut()
            .ok_or_else(|| invalid("missing CABAC 4:4:4 PCM state"))?;
        let (y, [cb, cr]) =
            self.cabac
                .pcm_raw::<256>(self.sps.bit_depth_luma, state.depth, true)?;
        mb.qp = 0;
        mb.luma = IntraLuma::Pcm {
            y,
            cb: [0; 64],
            cr: [0; 64],
        };
        let mut c = mb.clone();
        c.luma = IntraLuma::Pcm {
            y: cb,
            cb: [0; 64],
            cr: [0; 64],
        };
        let mut r = mb.clone();
        r.luma = IntraLuma::Pcm {
            y: cr,
            cb: [0; 64],
            cr: [0; 64],
        };
        state.pending = Some([c, r]);
        let w = self.sps.width_mbs as usize;
        let at = self.address;
        for p in 0..2 {
            state.dc[p][at] = 1;
            for by in 0..4 {
                for bx in 0..4 {
                    state.coded[p][block_index(w, 4, at % w * 4 + bx, at / w * 4 + by)] = 1;
                }
            }
        }
        Ok(())
    }
    pub(super) fn read_444_coefficients(
        &mut self,
        mb: &IntraMacroblock,
        mb_type: u8,
        intra: bool,
    ) -> Result<()> {
        let mut state = self
            .colour444
            .take()
            .ok_or_else(|| invalid("missing CABAC 4:4:4 coefficient state"))?;
        let w = self.sps.width_mbs as usize;
        let at = self.address;
        let (mx, my) = (at % w, at / w);
        let field = self.current_field();
        let mut output = [mb.clone(), mb.clone()];
        for p in 0..2 {
            let component = &mut output[p];
            component.luma_dc.fill(0);
            component.luma_levels.fill([0; 16]);
            if let IntraLuma::Blocks8 { levels, .. } = &mut component.luma {
                levels.fill([0; 64]);
            }
            let categories = if p == 0 {
                [Cat::CbDc, Cat::CbAc, Cat::Cb4, Cat::Cb8]
            } else {
                [Cat::CrDc, Cat::CrAc, Cat::Cr4, Cat::Cr8]
            };
            state.dc[p][at] = 0;
            if mb_type != 0 {
                let inc = self.macro_coded_context(&state.dc[p], intra)?;
                let r = self.cabac.residual_444(categories[0], inc, field)?;
                state.dc[p][at] = u8::from(r.total_coefficients != 0);
                let mut levels = [0; 16];
                levels.copy_from_slice(&r.coefficients[..16]);
                component.luma_dc = inverse_scan_4x4(&levels, field);
            }
            if let IntraLuma::Blocks8 { levels, .. } = &mut component.luma {
                for block in 0..4 {
                    let (x, y) = (mx * 4 + block % 2 * 2, my * 4 + block / 2 * 2);
                    let mut coded = 0;
                    if component.coded_block_pattern & (1 << block) != 0 {
                        let inc = self.coded_context_4448(&state.coded[p], x, y, intra)?;
                        let r = self.cabac.residual_444(categories[3], inc, field)?;
                        coded = u8::from(r.total_coefficients != 0);
                        levels[block] =
                            super::super::avc_transform8::inverse_scan_8x8(&r.coefficients, field);
                    }
                    for by in 0..2 {
                        for bx in 0..2 {
                            state.coded[p][block_index(w, 4, x + bx, y + by)] = coded;
                        }
                    }
                }
            } else {
                for block in 0..16 {
                    let (bx, by) = luma_block_xy(block)?;
                    let (x, y) = (mx * 4 + bx, my * 4 + by);
                    let mut coded = 0;
                    if component.coded_block_pattern & (1 << (block / 4)) != 0 {
                        let inc = self.component_coded_context(&state.coded[p], 4, x, y, intra)?;
                        let r = self.cabac.residual_444(
                            categories[if mb_type == 0 { 2 } else { 1 }],
                            inc,
                            field,
                        )?;
                        coded = u8::from(r.total_coefficients != 0);
                        let mut levels = [0; 16];
                        levels.copy_from_slice(&r.coefficients[..16]);
                        if mb_type != 0 {
                            levels.copy_within(0..15, 1);
                            levels[0] = 0;
                        }
                        component.luma_levels[by * 4 + bx] = inverse_scan_4x4(&levels, field);
                    }
                    state.coded[p][block_index(w, 4, x, y)] = coded;
                }
            }
        }
        state.pending = Some(output);
        self.colour444 = Some(state);
        Ok(())
    }
}
