//! MBAFF intra pictures and address-local inter reconstruction/publication.
use super::{
    avc::{Pps, Sps},
    avc_cabac_macroblock::IntraCabacReader,
    avc_deblock::{MbaffIntraBlock, mbaff_intra_plane},
    avc_macroblock::{IntraCavlcReader, IntraLuma},
    avc_mbaff::{Readiness420, layout, write_samples},
    avc_picture::{IntraPicture, reconstruct_macroblock},
    avc_scaling::ScalingMatrices,
    avc_slice::SliceHeader,
};
use crate::{Result, invalid};
fn plane(size: usize) -> Result<Vec<u16>> {
    let mut values = Vec::new();
    values
        .try_reserve_exact(size)
        .map_err(|_| invalid("cannot allocate MBAFF reconstruction plane"))?;
    values.resize(size, 0);
    Ok(values)
}
/// Reconstruct and publish an inter macroblock in frame or alternating field
/// rows. Prediction, scaling and component QPs are already resolved by caller.
/// Validation and residual reconstruction finish before any picture plane changes.
pub fn reconstruct_inter_macroblock(
    picture: &mut IntraPicture,
    address: usize,
    field: bool,
    prediction: super::avc_compensation::Prediction420,
    coefficients: Option<&super::avc_inter_coefficients::InterCoefficients>,
    transform8: bool,
    qps: [u8; 3],
    bypass: bool,
    scaling: &ScalingMatrices,
) -> Result<()> {
    let (w, h) = (picture.coded_width, picture.coded_height);
    let pixels = w
        .checked_mul(h)
        .ok_or_else(|| invalid("MBAFF picture size overflow"))?;
    if !(8..=14).contains(&picture.bit_depth)
        || w % 16 != 0
        || h % 32 != 0
        || prediction.dimensions() != (16, 16)
        || prediction.bit_depth() != picture.bit_depth
        || picture.y.len() != pixels
        || picture.cb.len() != pixels / 4
        || picture.cr.len() != pixels / 4
    {
        return Err(invalid("invalid MBAFF inter reconstruction picture"));
    }
    let maximum = (1u16 << picture.bit_depth) - 1;
    if prediction
        .y
        .iter()
        .chain(&prediction.cb)
        .chain(&prediction.cr)
        .any(|&v| v > maximum)
    {
        return Err(invalid("MBAFF inter prediction exceeds bit depth"));
    }
    let targets = [
        layout(address, w / 16, h / 16, true, field, [1, 1])?,
        layout(address, w / 16, h / 16, true, field, [2, 2])?,
    ];
    let reconstructed = if let Some(c) = coefficients {
        let luma = if transform8 {
            super::avc_compensation::InterLumaResidual::Blocks8(&c.luma8)
        } else {
            super::avc_compensation::InterLumaResidual::Blocks4(&c.luma4)
        };
        if bypass {
            prediction.reconstruct_inter_bypass(luma, &c.chroma_dc, &c.chroma_ac)?
        } else {
            prediction.reconstruct_inter(
                luma,
                &c.chroma_dc,
                &c.chroma_ac,
                qps,
                &[scaling.four[3], scaling.four[4], scaling.four[5]],
                &scaling.eight[1],
            )?
        }
    } else {
        prediction
    };
    write_samples(&mut picture.y, w, targets[0], &reconstructed.y)?;
    write_samples(&mut picture.cb, w / 2, targets[1], &reconstructed.cb)?;
    write_samples(&mut picture.cr, w / 2, targets[1], &reconstructed.cr)?;
    Ok(())
}
/// Decode one complete intra MBAFF CAVLC or CABAC slice.
/// Budget includes frame planes, a temporary prediction view and entropy/readiness
/// contexts. Input RBSP and caller-held output pictures are outside this budget.
pub fn decode_intra_picture(
    header: &SliceHeader,
    sps: &Sps,
    pps: &Pps,
    budget: usize,
) -> Result<IntraPicture> {
    decode_intra_slices(&[header], sps, pps, budget)
}
/// Reconstruct ordered intra slices with independent entropy and prediction state.
pub fn decode_intra_slices(
    headers: &[&SliceHeader],
    sps: &Sps,
    pps: &Pps,
    budget: usize,
) -> Result<IntraPicture> {
    let header = *headers
        .first()
        .ok_or_else(|| invalid("missing MBAFF intra slices"))?;
    if header.first_mb != 0 {
        return Err(invalid(
            "MBAFF intra picture must start at macroblock pair zero",
        ));
    }
    if sps.bit_depth_luma != sps.bit_depth_chroma {
        return Err(crate::unsupported(
            "MBAFF mixed component depths are not connected",
        ));
    }
    let (w, h) = sps.coded_dimensions();
    let (w, h) = (w as usize, h as usize);
    let pixels = w
        .checked_mul(h)
        .ok_or_else(|| invalid("MBAFF pixel count overflow"))?;
    let count = pixels / 256;
    let storage = pixels
        .checked_mul(6)
        .and_then(|n| {
            count
                .checked_mul(80 + std::mem::size_of::<MbaffIntraBlock>())
                .and_then(|c| n.checked_add(c))
        })
        .ok_or_else(|| invalid("MBAFF reconstruction budget overflow"))?;
    if storage > budget {
        return Err(invalid("MBAFF reconstruction exceeds memory budget"));
    }

    let scaling = ScalingMatrices::new(sps, pps)?;
    let mut readiness = Readiness420::new(w / 16, h / 16, true, count * 7)?;
    let mut picture = IntraPicture {
        coded_width: w,
        coded_height: h,
        crop: sps.crop.map(|n| n as usize),
        bit_depth: sps.bit_depth_luma,
        y: plane(pixels)?,
        cb: plane(pixels / 4)?,
        cr: plane(pixels / 4)?,
    };
    let mut seen = 0;
    let mut deblocking = Vec::new();
    deblocking
        .try_reserve_exact(count)
        .map_err(|_| invalid("cannot allocate MBAFF deblocking metadata"))?;
    for (index, header) in headers.iter().enumerate() {
        if header.first_mb as usize * 2 != seen {
            return Err(invalid("MBAFF slice coverage gap or overlap"));
        }
        let end = headers
            .get(index + 1)
            .map_or(count, |h| h.first_mb as usize * 2);
        readiness.reset_slice();
        let mut cavlc = if pps.cabac {
            None
        } else {
            Some(IntraCavlcReader::new_mbaff(header, sps, pps, count)?)
        };
        let mut cabac = if pps.cabac {
            Some(IntraCabacReader::new_mbaff(header, sps, pps, count)?)
        } else {
            None
        };
        loop {
            let mb = match (&mut cavlc, &mut cabac) {
                (Some(reader), _) => reader.read_macroblock()?,
                (_, Some(reader)) => reader.read_macroblock()?,
                _ => unreachable!(),
            };
            let Some(mut mb) = mb else { break };
            let address = mb.address as usize;
            if address != seen || seen >= end {
                return Err(invalid("MBAFF intra slice coverage gap"));
            }
            let field = match (&cavlc, &cabac) {
                (Some(reader), _) => reader.field_decoding(),
                (_, Some(reader)) => reader.field_decoding(),
                _ => unreachable!(),
            };
            let pcm = matches!(mb.luma, IntraLuma::Pcm { .. });
            let qp = if pcm { 0 } else { mb.qp };
            let chroma = |offset| {
                i32::from(super::avc_picture::chroma_qp(
                    qp,
                    offset,
                    sps.bit_depth_chroma,
                )) - 6 * (i32::from(sps.bit_depth_chroma) - 8)
            };
            deblocking.push(MbaffIntraBlock {
                qp: if pcm {
                    [0; 3]
                } else {
                    [
                        qp,
                        chroma(pps.chroma_qp_offset),
                        chroma(pps.second_chroma_qp_offset),
                    ]
                },
                field,
                transform8: matches!(mb.luma, IntraLuma::Blocks8 { .. }),
                slice: index,
                disable: u8::try_from(header.disable_deblocking_filter_idc)
                    .map_err(|_| invalid("invalid MBAFF deblocking disable value"))?,
                offsets: [header.alpha_offset, header.beta_offset],
            });
            let parity = if field { address % 2 } else { 0 };
            let step = if field { 2 } else { 1 };
            let view_h = h / step;
            let mut view = IntraPicture {
                coded_width: w,
                coded_height: view_h,
                crop: [0; 4],
                bit_depth: picture.bit_depth,
                y: plane(w * view_h)?,
                cb: plane(w * view_h / 4)?,
                cr: plane(w * view_h / 4)?,
            };
            for (source, dest, stride) in [
                (&picture.y, &mut view.y, w),
                (&picture.cb, &mut view.cb, w / 2),
                (&picture.cr, &mut view.cr, w / 2),
            ] {
                for (row, line) in dest.chunks_exact_mut(stride).enumerate() {
                    let start = (row * step + parity) * stride;
                    line.copy_from_slice(&source[start..start + stride]);
                }
            }
            let mut ready = crate::buffer(w * view_h / 16)?;
            for by in 0..view_h / 4 {
                for bx in 0..w / 4 {
                    let mut available = true;
                    for dy in 0..4 {
                        for dx in 0..4 {
                            available &= readiness
                                .available(0, [bx * 4 + dx, (by * 4 + dy) * step + parity])?;
                        }
                    }
                    ready[by * (w / 4) + bx] = u8::from(available);
                }
            }
            let geometry = layout(address, w / 16, h / 16, true, field, [1, 1])?;
            let logical_y = (geometry.origin[1] - parity) / step;
            mb.address = (logical_y / 16 * (w / 16) + geometry.origin[0] / 16) as u32;
            reconstruct_macroblock(&mut view, &mb, sps, pps, &scaling, &mut ready)?;
            for (component, source, dest, stride, side) in [
                (0, &view.y, &mut picture.y, w, 16),
                (1, &view.cb, &mut picture.cb, w / 2, 8),
                (2, &view.cr, &mut picture.cr, w / 2, 8),
            ] {
                let sub = if component == 0 { [1, 1] } else { [2, 2] };
                let target = layout(address, w / 16, h / 16, true, field, sub)?;
                let x = target.origin[0];
                let y = (target.origin[1] - parity) / step;
                let mut samples = [0u16; 256];
                for row in 0..side {
                    samples[row * side..row * side + side].copy_from_slice(
                        &source[(y + row) * stride + x..(y + row) * stride + x + side],
                    );
                }
                write_samples(dest, stride, target, &samples[..side * side])?;
                readiness.publish(address, component, [0, 0, side / 4, side / 4], field)?;
            }
            seen += 1;
        }
        if seen != end {
            return Err(invalid("incomplete MBAFF intra slice"));
        }
    }
    if seen != count {
        return Err(invalid("incomplete MBAFF intra picture"));
    }
    for (component, samples, width, height) in [
        (0, &mut picture.y, w, h),
        (1, &mut picture.cb, w / 2, h / 2),
        (2, &mut picture.cr, w / 2, h / 2),
    ] {
        mbaff_intra_plane(
            samples,
            width,
            height,
            picture.bit_depth,
            component,
            &deblocking,
        )?;
    }
    Ok(picture)
}

#[cfg(test)]
mod inter_tests {
    use super::super::{avc_compensation::Reference420, avc_inter_coefficients::InterCoefficients};
    use super::*;
    fn picture(depth: u8) -> IntraPicture {
        IntraPicture {
            coded_width: 32,
            coded_height: 32,
            crop: [0; 4],
            bit_depth: depth,
            y: vec![9; 1024],
            cb: vec![9; 256],
            cr: vec![9; 256],
        }
    }
    fn coefficients() -> InterCoefficients {
        InterCoefficients {
            luma4: [[0; 16]; 16],
            luma8: [[0; 64]; 4],
            chroma_dc: [[0; 4]; 2],
            chroma_ac: [[[0; 16]; 4]; 2],
            luma_counts: [0; 16],
            chroma_counts: [[0; 4]; 2],
        }
    }
    #[test]
    fn inter_residuals_publish_only_owned_frame_or_field_samples() {
        let scaling = ScalingMatrices {
            four: [[16; 16]; 6],
            eight: [[16; 64]; 2],
        };
        for depth in [8, 10] {
            let reference = Reference420::new(
                [&[50; 256], &[20; 64], &[20; 64]],
                16,
                16,
                [16, 8, 8],
                depth,
            )
            .unwrap();
            for field in [false, true] {
                for address in 0..4 {
                    for eight in [false, true] {
                        let mut p = picture(depth);
                        let mut c = coefficients();
                        c.luma4[0][0] = 5;
                        c.luma8[0][0] = 5;
                        c.chroma_dc[0][0] = -2;
                        c.chroma_dc[1][0] = 7;
                        reconstruct_inter_macroblock(
                            &mut p,
                            address,
                            field,
                            reference.predict([0; 2], [0; 2], [16; 2]).unwrap(),
                            Some(&c),
                            eight,
                            [0; 3],
                            true,
                            &scaling,
                        )
                        .unwrap();
                        for (component, samples, width, height, base, first) in [
                            (0, &p.y, 32usize, 32usize, 50, 55),
                            (1, &p.cb, 16usize, 16usize, 20, 18),
                            (2, &p.cr, 16usize, 16usize, 20, 27),
                        ] {
                            let sub = if component == 0 { [1, 1] } else { [2, 2] };
                            let target = layout(address, 2, 2, true, field, sub).unwrap();
                            for y in 0..height {
                                for x in 0..width {
                                    let dx = x.checked_sub(target.origin[0]);
                                    let dy = y.checked_sub(target.origin[1]);
                                    let inside = dx.is_some_and(|v| v < target.size[0])
                                        && dy.is_some_and(|v| {
                                            v % target.row_step == 0
                                                && v / target.row_step < target.size[1]
                                        });
                                    let expected = if !inside {
                                        9
                                    } else if [x, y] == target.origin {
                                        first
                                    } else {
                                        base
                                    };
                                    assert_eq!(
                                        samples[y * width + x],
                                        expected,
                                        "component {component} address {address} field {field} at {x},{y}"
                                    );
                                }
                            }
                        }
                    }
                }
            }
        }
    }
    #[test]
    fn invalid_plane_or_residual_never_partially_publishes() {
        let scaling = ScalingMatrices {
            four: [[16; 16]; 6],
            eight: [[16; 64]; 2],
        };
        let reference =
            Reference420::new([&[50; 256], &[20; 64], &[20; 64]], 16, 16, [16, 8, 8], 8).unwrap();
        for case in 0..5 {
            let mut p = picture(8);
            if case == 0 {
                p.cr.pop();
            }
            if case == 1 {
                p.bit_depth = 10;
            }
            let c = coefficients();
            let mut prediction = reference.predict([0; 2], [0; 2], [16; 2]).unwrap();
            if case == 4 {
                prediction.cr[0] = 256;
            }
            assert!(
                reconstruct_inter_macroblock(
                    &mut p,
                    if case == 2 { 4 } else { 0 },
                    true,
                    prediction,
                    Some(&c),
                    false,
                    if case == 3 { [255; 3] } else { [0; 3] },
                    false,
                    &scaling
                )
                .is_err()
            );
            assert!(p.y.iter().chain(&p.cb).chain(&p.cr).all(|&v| v == 9));
        }
    }
}
