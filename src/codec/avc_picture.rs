//! Assembly of complete progressive CAVLC/CABAC intra pictures from FVid's own primitives.
use super::{
    avc::{Pps, Sps},
    avc_intra::{chroma8, intra4},
    avc_macroblock::{IntraCavlcReader, IntraLuma, luma_block_xy},
    avc_prediction::{Intra16Mode, intra16, reconstruct_intra16},
    avc_slice::SliceHeader,
    avc_transform::{chroma_dc_2x2, reconstruct_4x4, residual_4x4},
};
use crate::{Result, invalid};
use std::io::Write;
#[derive(Debug)]
pub struct IntraPicture {
    pub coded_width: usize,
    pub coded_height: usize,
    pub crop: [usize; 4],
    pub bit_depth: u8,
    pub y: Vec<u16>,
    pub cb: Vec<u16>,
    pub cr: Vec<u16>,
}
impl IntraPicture {
    pub fn dimensions(&self) -> (usize, usize) {
        (
            self.coded_width - self.crop[0] - self.crop[1],
            self.coded_height - self.crop[2] - self.crop[3],
        )
    }
    /// Visible planar YUV420; samples above 8 bits are little endian u16.
    pub fn write_planar<W: Write>(&self, writer: &mut W) -> Result<()> {
        let (w, h) = self.dimensions();
        for (plane, shift) in [(&self.y, 0), (&self.cb, 1), (&self.cr, 1)] {
            let stride = self.coded_width >> shift;
            let x = self.crop[0] >> shift;
            let y = self.crop[2] >> shift;
            for row in 0..h >> shift {
                for &sample in &plane[(y + row) * stride + x..(y + row) * stride + x + (w >> shift)]
                {
                    if self.bit_depth == 8 {
                        writer.write_all(&[sample as u8])?;
                    } else {
                        writer.write_all(&sample.to_le_bytes())?;
                    }
                }
            }
        }
        Ok(())
    }
}
fn put<const N: usize>(
    plane: &mut [u16],
    stride: usize,
    x: usize,
    y: usize,
    size: usize,
    block: &[u16; N],
) {
    for row in 0..size {
        plane[(y + row) * stride + x..(y + row) * stride + x + size]
            .copy_from_slice(&block[row * size..row * size + size]);
    }
}
fn edges<const N: usize>(
    plane: &[u16],
    stride: usize,
    x: usize,
    y: usize,
) -> (Option<[u16; N]>, Option<[u16; N]>, Option<u16>) {
    let top = if y > 0 {
        Some(std::array::from_fn(|i| plane[(y - 1) * stride + x + i]))
    } else {
        None
    };
    let left = if x > 0 {
        Some(std::array::from_fn(|i| plane[(y + i) * stride + x - 1]))
    } else {
        None
    };
    let corner = if x > 0 && y > 0 {
        Some(plane[(y - 1) * stride + x - 1])
    } else {
        None
    };
    (top, left, corner)
}
pub(super) fn chroma_qp(qp: i32, offset: i32, depth: u8) -> u8 {
    const MAP: [i32; 22] = [
        29, 30, 31, 32, 32, 33, 34, 34, 35, 35, 36, 36, 37, 37, 37, 38, 38, 38, 39, 39, 39, 39,
    ];
    let bd = 6 * (i32::from(depth) - 8);
    let q = (qp + offset).clamp(-bd, 51);
    (if q < 30 { q } else { MAP[(q - 30) as usize] } + bd) as u8
}
/// Decode a single-slice intra picture. Unsupported reconstruction tools fail explicitly.
/// Budget covers planes and context grids, excluding the caller-owned RBSP and writer buffers.
pub fn decode_intra_picture(
    header: &SliceHeader,
    sps: &Sps,
    pps: &Pps,
    memory_limit: usize,
) -> Result<IntraPicture> {
    decode_intra_slices(&[header], sps, pps, memory_limit)
}
/// Reconstruct raster-ordered intra slices into shared planes, with independent
/// entropy and prediction availability for each slice.
pub fn decode_intra_slices(
    headers: &[&SliceHeader],
    sps: &Sps,
    pps: &Pps,
    memory_limit: usize,
) -> Result<IntraPicture> {
    let header = *headers
        .first()
        .ok_or_else(|| invalid("missing intra slices"))?;
    if header.first_mb != 0 {
        return Err(invalid("intra picture must begin at zero"));
    }
    if headers
        .iter()
        .any(|slice| slice.slice_type != super::avc_slice::SliceType::I)
    {
        return Err(invalid("intra reconstruction requires I slices"));
    }
    if sps.bit_depth_luma != sps.bit_depth_chroma {
        return Err(invalid("mixed component bit depths are not yet supported"));
    }
    let scaling = super::avc_scaling::ScalingMatrices::new(sps, pps)?;
    let (w, h) = sps.coded_dimensions();
    let (w, h) = (w as usize, h as usize);
    let pixels = w
        .checked_mul(h)
        .ok_or_else(|| invalid("picture dimensions overflow"))?;
    let count = pixels / 256;
    let bytes = pixels
        .checked_mul(3)
        .and_then(|n| {
            count
                .checked_mul(
                    (if pps.cabac { 76 } else { 69 })
                        + std::mem::size_of::<super::avc_deblock::MacroblockEdges>(),
                )
                .and_then(|c| n.checked_add(c))
        })
        .ok_or_else(|| invalid("picture budget overflow"))?;
    if bytes > memory_limit {
        return Err(invalid("decoded picture exceeds memory budget"));
    }
    let alloc = |count: usize| -> Result<Vec<u16>> {
        let mut v = Vec::new();
        v.try_reserve_exact(count)
            .map_err(|_| invalid("picture allocation failed"))?;
        v.resize(count, 0);
        Ok(v)
    };
    let mut picture = IntraPicture {
        coded_width: w,
        coded_height: h,
        crop: sps.crop.map(|n| n as usize),
        bit_depth: sps.bit_depth_luma,
        y: alloc(pixels)?,
        cb: alloc(pixels / 4)?,
        cr: alloc(pixels / 4)?,
    };
    let mut ready = crate::buffer(count * 16)?;
    let mut eight = crate::buffer(count)?;
    let mut qps = [vec![0; count], vec![0; count], vec![0; count]];
    let mut seen = 0;
    for (slice_index, header) in headers.iter().enumerate() {
        if header.first_mb as usize != seen {
            return Err(invalid("intra slice coverage gap or overlap"));
        }
        let end = headers
            .get(slice_index + 1)
            .map_or(count, |next| next.first_mb as usize);
        ready.fill(0);
        let mut cavlc = if pps.cabac {
            None
        } else {
            Some(IntraCavlcReader::new(header, sps, pps, 65536)?)
        };
        let mut cabac = if pps.cabac {
            Some(super::avc_cabac_macroblock::IntraCabacReader::new(
                header, sps, pps, 65536,
            )?)
        } else {
            None
        };
        let mut read_macroblock = || match (&mut cavlc, &mut cabac) {
            (Some(reader), _) => reader.read_macroblock(),
            (_, Some(reader)) => reader.read_macroblock(),
            _ => unreachable!(),
        };
        while let Some(mb) = read_macroblock()? {
            if mb.address as usize != seen || seen >= end {
                return Err(invalid("intra slice exceeds assigned macroblock range"));
            }
            qps[0][mb.address as usize] = mb.qp;
            for component in 0..2 {
                let offset = if component == 0 {
                    pps.chroma_qp_offset
                } else {
                    pps.second_chroma_qp_offset
                };
                qps[component + 1][mb.address as usize] =
                    i32::from(chroma_qp(mb.qp, offset, sps.bit_depth_chroma))
                        - 6 * (i32::from(sps.bit_depth_chroma) - 8);
            }
            eight[mb.address as usize] = u8::from(matches!(mb.luma, IntraLuma::Blocks8 { .. }));
            reconstruct_macroblock(&mut picture, &mb, sps, pps, &scaling, &mut ready)?;
            seen += 1;
        }
        if seen != end {
            return Err(invalid("incomplete intra slice"));
        }
    }
    if seen != count {
        return Err(invalid("incomplete intra picture"));
    }
    {
        super::avc_deblock::intra_plane(
            &mut picture.y,
            w,
            h,
            &qps[0],
            sps.bit_depth_luma,
            headers,
            false,
            &eight,
        )?;
        super::avc_deblock::intra_plane(
            &mut picture.cb,
            w / 2,
            h / 2,
            &qps[1],
            sps.bit_depth_chroma,
            headers,
            true,
            &eight,
        )?;
        super::avc_deblock::intra_plane(
            &mut picture.cr,
            w / 2,
            h / 2,
            &qps[2],
            sps.bit_depth_chroma,
            headers,
            true,
            &eight,
        )?;
    }
    Ok(picture)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> (Sps, Pps, SliceHeader) {
        let hex = "6742c01fda03c045fbc044000003000400000300f03c60ca80";
        let nal: Vec<_> = (0..hex.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&hex[i..i + 2], 16).unwrap())
            .collect();
        let mut sps = Sps::parse(&nal).unwrap();
        sps.width_mbs = 1;
        sps.height_map_units = 1;
        sps.crop = [0; 4];
        let pps = Pps::parse(&[0x68, 0xce, 0x09, 0xc8], &sps).unwrap();
        let mut header =
            SliceHeader::parse(&[0x65, 0x88, 0x84, 0x3a, 0x27, 0x80], &sps, &pps).unwrap();
        header.disable_deblocking_filter_idc = 1;
        (sps, pps, header)
    }

    #[test]
    fn full_neutral_picture_and_visible_crop() {
        let (mut sps, pps, header) = fixture();
        sps.crop = [2, 4, 2, 6];
        let picture = decode_intra_picture(
            &header,
            &sps,
            &pps,
            837 + std::mem::size_of::<super::super::avc_deblock::MacroblockEdges>(),
        )
        .unwrap();
        assert_eq!(picture.dimensions(), (10, 8));
        assert_eq!(picture.y, vec![128; 256]);
        assert_eq!(picture.cb, vec![128; 64]);
        assert_eq!(picture.cr, vec![128; 64]);
        let mut raw = Vec::new();
        picture.write_planar(&mut raw).unwrap();
        assert_eq!(raw, vec![128; 120]);
    }

    #[test]
    fn intra_pixels_exclude_unavailable_inter_neighbours() {
        let (mut sps, pps, header) = fixture();
        let mut reader = IntraCavlcReader::new(&header, &sps, &pps, 4096).unwrap();
        let mut mb = reader.read_macroblock().unwrap().unwrap();
        sps.width_mbs = 2;
        sps.height_map_units = 2;
        mb.address = 3;
        for luma in [
            IntraLuma::Block16(2),
            IntraLuma::Blocks4([super::super::avc_intra::Intra4Mode::Dc; 16]),
            IntraLuma::Blocks8 {
                modes: [super::super::avc_intra::Intra4Mode::Dc; 4],
                levels: [[0; 64]; 4],
            },
        ] {
            mb.luma = luma;
            for (available, expected) in [(0, 128), (1, 64)] {
                let mut picture = IntraPicture {
                    coded_width: 32,
                    coded_height: 32,
                    crop: [0; 4],
                    bit_depth: 8,
                    y: vec![64; 1024],
                    cb: vec![64; 256],
                    cr: vec![64; 256],
                };
                let mut ready = vec![available; 64];
                for y in 4..8 {
                    for x in 4..8 {
                        ready[y * 8 + x] = 0;
                    }
                }
                reconstruct_macroblock(
                    &mut picture,
                    &mb,
                    &sps,
                    &pps,
                    &crate::codec::avc_scaling::ScalingMatrices::new(&sps, &pps).unwrap(),
                    &mut ready,
                )
                .unwrap();
                for y in 16..32 {
                    assert!(
                        picture.y[y * 32 + 16..y * 32 + 32]
                            .iter()
                            .all(|&v| v == expected)
                    );
                }
                for plane in [&picture.cb, &picture.cr] {
                    for y in 8..16 {
                        assert!(
                            plane[y * 16 + 8..y * 16 + 16]
                                .iter()
                                .all(|&v| v == expected)
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn rejects_budget_partial_picture_and_truncated_payload() {
        let (mut sps, pps, mut header) = fixture();
        assert!(
            decode_intra_picture(
                &header,
                &sps,
                &pps,
                836 + std::mem::size_of::<super::super::avc_deblock::MacroblockEdges>()
            )
            .is_err()
        );
        sps.width_mbs = 2;
        assert!(decode_intra_picture(&header, &sps, &pps, 4096).is_err());
        sps.width_mbs = 1;
        header.rbsp.pop();
        assert!(decode_intra_picture(&header, &sps, &pps, 4096).is_err());
    }
}

fn available_edges<const N: usize>(
    plane: &[u16],
    stride: usize,
    x: usize,
    y: usize,
    ready: &[u8],
    scale: usize,
) -> (Option<[u16; N]>, Option<[u16; N]>, Option<u16>) {
    let available =
        |px: usize, py: usize| ready[(py * scale / 4) * (stride * scale / 4) + px * scale / 4] != 0;
    let (top, left, corner) = edges::<N>(plane, stride, x, y);
    (
        top.filter(|_| y > 0 && available(x, y - 1)),
        left.filter(|_| x > 0 && available(x - 1, y)),
        corner.filter(|_| x > 0 && y > 0 && available(x - 1, y - 1)),
    )
}

/// Shared scaling-aware intra reconstruction for a complete progressive picture.
/// Caller provides valid raster macroblock geometry and availability grid.
pub(super) fn reconstruct_macroblock(
    picture: &mut IntraPicture,
    mb: &super::avc_macroblock::IntraMacroblock,
    sps: &Sps,
    pps: &Pps,
    scaling: &super::avc_scaling::ScalingMatrices,
    ready: &mut [u8],
) -> Result<()> {
    let (w, h) = (picture.coded_width, picture.coded_height);
    let (mx, my) = (
        mb.address as usize % (w / 16),
        mb.address as usize / (w / 16),
    );
    let qp = (mb.qp + 6 * (i32::from(sps.bit_depth_luma) - 8)) as u8;
    let bypass = sps.transform_bypass && qp == 0;
    match &mb.luma {
        IntraLuma::Blocks8 { modes, levels } => {
            for block in 0..4 {
                let x = mx * 16 + block % 2 * 8;
                let y = my * 16 + block / 2 * 8;
                let (t, l, c) = available_edges::<8>(&picture.y, w, x, y, ready, 1);
                let right =
                    if y > 0 && x + 15 < w && ready[((y - 1) / 4) * (w / 4) + (x + 8) / 4] != 0 {
                        Some(std::array::from_fn(|i| picture.y[(y - 1) * w + x + 8 + i]))
                    } else {
                        None
                    };
                let pred = super::avc_intra::intra8(
                    modes[block],
                    t.as_ref(),
                    right.as_ref(),
                    l.as_ref(),
                    c,
                    sps.bit_depth_luma,
                )?;
                let residual = if bypass {
                    super::avc_bypass::residual(
                        &levels[block],
                        8,
                        super::avc_bypass::luma_direction(modes[block] as u8),
                    )?
                } else {
                    super::avc_transform8::residual_8x8(
                        &levels[block],
                        qp,
                        sps.bit_depth_luma,
                        &scaling.eight[0],
                    )?
                };
                let reconstructed =
                    super::avc_transform8::reconstruct_8x8(&pred, &residual, sps.bit_depth_luma)?;
                put(&mut picture.y, w, x, y, 8, &reconstructed);
                for by in 0..2 {
                    for bx in 0..2 {
                        ready[(y / 4 + by) * (w / 4) + x / 4 + bx] = 1;
                    }
                }
            }
        }
        IntraLuma::Pcm { y, cb, cr } => {
            put(&mut picture.y, w, mx * 16, my * 16, 16, y);
            put(&mut picture.cb, w / 2, mx * 8, my * 8, 8, cb);
            put(&mut picture.cr, w / 2, mx * 8, my * 8, 8, cr);
        }
        IntraLuma::Block16(mode) => {
            let direction = super::avc_bypass::luma_direction(*mode);
            let (t, l, c) = available_edges::<16>(&picture.y, w, mx * 16, my * 16, ready, 1);
            let mode = match mode {
                0 => Intra16Mode::Vertical,
                1 => Intra16Mode::Horizontal,
                2 => Intra16Mode::Dc,
                3 => Intra16Mode::Plane,
                _ => return Err(invalid("invalid Intra16 mode")),
            };
            let prediction = intra16(mode, t.as_ref(), l.as_ref(), c, sps.bit_depth_luma)?;
            let block = if bypass {
                let levels =
                    super::avc_bypass::blocks4::<256>(&mb.luma_levels, Some(&mb.luma_dc), 16)?;
                let residual = super::avc_bypass::residual(&levels, 16, direction)?;
                super::avc_transform::reconstruct(&prediction, &residual, sps.bit_depth_luma)?
            } else {
                reconstruct_intra16(
                    &prediction,
                    &mb.luma_dc,
                    &mb.luma_levels,
                    qp,
                    sps.bit_depth_luma,
                    &scaling.four[0],
                )?
            };
            put(&mut picture.y, w, mx * 16, my * 16, 16, &block);
        }
        IntraLuma::Blocks4(modes) => {
            for index in 0..16 {
                let (bx, by) = luma_block_xy(index)?;
                let x = mx * 16 + bx * 4;
                let y = my * 16 + by * 4;
                let available = |px: usize, py: usize| {
                    px < w && py < h && ready[(py / 4) * (w / 4) + px / 4] != 0
                };
                let (t, l, c) = edges::<4>(&picture.y, w, x, y);
                let t = t.filter(|_| y > 0 && available(x, y - 1));
                let l = l.filter(|_| x > 0 && available(x - 1, y));
                let c = c.filter(|_| x > 0 && y > 0 && available(x - 1, y - 1));
                let right = if y > 0 && x + 7 < w && available(x + 4, y - 1) {
                    Some(std::array::from_fn(|i| picture.y[(y - 1) * w + x + 4 + i]))
                } else {
                    None
                };
                let pred = intra4(
                    modes[by * 4 + bx],
                    t.as_ref(),
                    right.as_ref(),
                    l.as_ref(),
                    c,
                    sps.bit_depth_luma,
                )?;
                let residual = if bypass {
                    super::avc_bypass::residual(
                        &mb.luma_levels[by * 4 + bx],
                        4,
                        super::avc_bypass::luma_direction(modes[by * 4 + bx] as u8),
                    )?
                } else {
                    residual_4x4(
                        &mb.luma_levels[by * 4 + bx],
                        qp,
                        sps.bit_depth_luma,
                        &scaling.four[0],
                        None,
                    )?
                };
                let block = reconstruct_4x4(&pred, &residual, sps.bit_depth_luma)?;
                put(&mut picture.y, w, x, y, 4, &block);
                ready[(y / 4) * (w / 4) + x / 4] = 1;
            }
        }
    }
    if !matches!(mb.luma, IntraLuma::Pcm { .. }) {
        for component in 0..2 {
            let plane = if component == 0 {
                &mut picture.cb
            } else {
                &mut picture.cr
            };
            let stride = w / 2;
            let x = mx * 8;
            let y = my * 8;
            let (t, l, c) = available_edges::<8>(plane, stride, x, y, ready, 2);
            let prediction = chroma8(
                mb.chroma_mode,
                t.as_ref(),
                l.as_ref(),
                c,
                sps.bit_depth_chroma,
            )?;
            if bypass {
                use super::avc_bypass::{Direction, blocks4, residual};
                let direction = match mb.chroma_mode {
                    super::avc_intra::ChromaMode::Horizontal => Direction::Horizontal,
                    super::avc_intra::ChromaMode::Vertical => Direction::Vertical,
                    _ => Direction::None,
                };
                let levels =
                    blocks4::<64>(&mb.chroma_ac[component], Some(&mb.chroma_dc[component]), 8)?;
                let decoded = residual(&levels, 8, direction)?;
                let block =
                    super::avc_transform::reconstruct(&prediction, &decoded, sps.bit_depth_chroma)?;
                put(plane, stride, x, y, 8, &block);
                continue;
            }
            let offset = if component == 0 {
                pps.chroma_qp_offset
            } else {
                pps.second_chroma_qp_offset
            };
            let qp = chroma_qp(mb.qp, offset, sps.bit_depth_chroma);
            let dc = chroma_dc_2x2(
                &mb.chroma_dc[component],
                qp,
                sps.bit_depth_chroma,
                scaling.four[component + 1][0],
            )?;
            for block in 0..4 {
                let bx = (block % 2) * 4;
                let by = (block / 2) * 4;
                let pred = std::array::from_fn(|i| prediction[(by + i / 4) * 8 + bx + i % 4]);
                let residual = residual_4x4(
                    &mb.chroma_ac[component][block],
                    qp,
                    sps.bit_depth_chroma,
                    &scaling.four[component + 1],
                    Some(dc[block]),
                )?;
                let reconstructed = reconstruct_4x4(&pred, &residual, sps.bit_depth_chroma)?;
                put(plane, stride, x + bx, y + by, 4, &reconstructed);
            }
        }
    }
    for by in 0..4 {
        for bx in 0..4 {
            ready[(my * 4 + by) * (w / 4) + mx * 4 + bx] = 1;
        }
    }
    Ok(())
}
