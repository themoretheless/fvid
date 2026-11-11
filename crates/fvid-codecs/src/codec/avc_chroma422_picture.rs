//! Compact 4:2:2 intra chroma, H.264 8.3.4 and 8.5.11.
use super::super::avc_intra::ChromaMode;
use super::super::avc_macroblock::IntraMacroblock;
use super::{IntraLuma, IntraPicture, Pps, Sps, chroma_qp, residual_4x4};
use crate::{Result, invalid};
pub(super) fn transform_dc(levels: &[i32; 8], qp: u8, depth: u8, weight: u8) -> Result<[i32; 8]> {
    if !(8..=14).contains(&depth)
        || usize::from(qp) > 51 + 6 * usize::from(depth - 8)
        || weight == 0
    {
        return Err(invalid("invalid AVC 422 DC parameters"));
    }
    let matrix = [
        [1i64, 1, 1, 1],
        [1, 1, -1, -1],
        [1, -1, -1, 1],
        [1, -1, 1, -1],
    ];
    let q = usize::from(qp) + 3;
    let scale = [10i64, 11, 13, 14, 16, 18][q % 6] * i64::from(weight);
    let mut out = [0; 8];
    for y in 0..4 {
        for x in 0..2 {
            let f = (0..4)
                .map(|r| {
                    matrix[y][r]
                        * (i64::from(levels[r * 2])
                            + if x == 0 {
                                i64::from(levels[r * 2 + 1])
                            } else {
                                -i64::from(levels[r * 2 + 1])
                            })
                })
                .sum::<i64>();
            let v = f * scale;
            let v = if q / 6 >= 6 {
                v.checked_shl((q / 6 - 6) as u32)
                    .ok_or_else(|| invalid("AVC 422 DC overflow"))?
            } else {
                (v + (1i64 << (5 - q / 6))) >> (6 - q / 6)
            };
            out[y * 2 + x] =
                i32::try_from(v).map_err(|_| invalid("AVC 422 DC exceeds numeric range"))?;
        }
    }
    Ok(out)
}
pub(super) fn reconstruct(
    picture: &mut IntraPicture,
    mb: &IntraMacroblock,
    sps: &Sps,
    pps: &Pps,
    scaling: &super::super::avc_scaling::ScalingMatrices,
    ready: &[u8],
) -> Result<()> {
    let wide = mb
        .chroma422
        .as_ref()
        .ok_or_else(|| invalid("missing AVC 422 macroblock planes"))?;
    let stride = picture.coded_width / 2;
    let mx = mb.address as usize % (picture.coded_width / 16);
    let my = mb.address as usize / (picture.coded_width / 16);
    let (x, y) = (mx * 8, my * 16);
    let depth = sps.bit_depth_chroma;
    let max = (1u16 << depth) - 1;
    for (component, plane) in [&mut picture.cb, &mut picture.cr].into_iter().enumerate() {
        if matches!(mb.luma, IntraLuma::Pcm { .. }) {
            for row in 0..16 {
                plane[(y + row) * stride + x..(y + row) * stride + x + 8]
                    .copy_from_slice(&wide.pcm[component][row * 8..row * 8 + 8]);
            }
            continue;
        }
        let available =
            |px: usize, py: usize| ready[(py / 4) * (picture.coded_width / 4) + px * 2 / 4] != 0;
        let top: [Option<u16>; 8] = std::array::from_fn(|i| {
            (y > 0 && available(x + i, y - 1)).then(|| plane[(y - 1) * stride + x + i])
        });
        let left: [Option<u16>; 16] = std::array::from_fn(|i| {
            (x > 0 && available(x - 1, y + i)).then(|| plane[(y + i) * stride + x - 1])
        });
        let corner =
            (x > 0 && y > 0 && available(x - 1, y - 1)).then(|| plane[(y - 1) * stride + x - 1]);
        let mut prediction = [0u16; 128];
        let sum = |edge: &[Option<u16>]| -> Option<u32> {
            edge.iter()
                .try_fold(0u32, |n, v| Some(n + u32::from((*v)?)))
        };
        match mb.chroma_mode {
            ChromaMode::Dc => {
                for by in 0..4 {
                    for bx in 0..2 {
                        let (t, l) = (
                            sum(&top[bx * 4..bx * 4 + 4]),
                            sum(&left[by * 4..by * 4 + 4]),
                        );
                        let value = if bx > 0 && by == 0 {
                            t.or(l).map(|n| (n + 2) >> 2)
                        } else if bx == 0 && by > 0 {
                            l.or(t).map(|n| (n + 2) >> 2)
                        } else {
                            match (t, l) {
                                (Some(a), Some(b)) => Some((a + b + 4) >> 3),
                                (Some(a), None) | (None, Some(a)) => Some((a + 2) >> 2),
                                _ => None,
                            }
                        }
                        .unwrap_or(1 << (depth - 1)) as u16;
                        for row in 0..4 {
                            prediction
                                [(by * 4 + row) * 8 + bx * 4..(by * 4 + row) * 8 + bx * 4 + 4]
                                .fill(value);
                        }
                    }
                }
            }
            ChromaMode::Horizontal => {
                for row in 0..16 {
                    prediction[row * 8..row * 8 + 8].fill(
                        left[row].ok_or_else(|| invalid("AVC 422 horizontal requires left"))?,
                    );
                }
            }
            ChromaMode::Vertical => {
                for row in prediction.chunks_exact_mut(8) {
                    for i in 0..8 {
                        row[i] = top[i].ok_or_else(|| invalid("AVC 422 vertical requires top"))?;
                    }
                }
            }
            ChromaMode::Plane => {
                if top.iter().chain(&left).any(Option::is_none) || corner.is_none() {
                    return Err(invalid("AVC 422 plane requires all edges"));
                }
                let corner = i32::from(corner.unwrap());
                let t = top.map(|v| i32::from(v.unwrap()));
                let l = left.map(|v| i32::from(v.unwrap()));
                let h = (1..=4)
                    .map(|i| i as i32 * (t[3 + i] - if i == 4 { corner } else { t[3 - i] }))
                    .sum::<i32>();
                let v = (1..=8)
                    .map(|i| i as i32 * (l[7 + i] - if i == 8 { corner } else { l[7 - i] }))
                    .sum::<i32>();
                let a = 16 * (t[7] + l[15]);
                let b = (34 * h + 32) >> 6;
                let c = (5 * v + 32) >> 6;
                for row in 0..16 {
                    for col in 0..8 {
                        prediction[row * 8 + col] =
                            ((a + b * (col as i32 - 3) + c * (row as i32 - 7) + 16) >> 5)
                                .clamp(0, i32::from(max)) as u16;
                    }
                }
            }
        }
        let qp = chroma_qp(
            mb.qp,
            if component == 0 {
                pps.chroma_qp_offset
            } else {
                pps.second_chroma_qp_offset
            },
            depth,
        );
        let bypass = sps.transform_bypass && (mb.qp + 6 * (i32::from(sps.bit_depth_luma) - 8) == 0);
        let dc = if bypass {
            wide.dc[component]
        } else {
            transform_dc(
                &wide.dc[component],
                qp,
                depth,
                scaling.four[component + 1][0],
            )?
        };
        let mut residual = [0i32; 128];
        for block in 0..8 {
            let (bx, by) = (block % 2 * 4, block / 2 * 4);
            let values = if bypass {
                let mut v = wide.ac[component][block];
                v[0] = dc[block];
                v
            } else {
                residual_4x4(
                    &wide.ac[component][block],
                    qp,
                    depth,
                    &scaling.four[component + 1],
                    Some(dc[block]),
                )?
            };
            for row in 0..4 {
                residual[(by + row) * 8 + bx..(by + row) * 8 + bx + 4]
                    .copy_from_slice(&values[row * 4..row * 4 + 4]);
            }
        }
        if bypass {
            match mb.chroma_mode {
                ChromaMode::Horizontal => {
                    for row in residual.chunks_exact_mut(8) {
                        for col in 1..8 {
                            row[col] = row[col]
                                .checked_add(row[col - 1])
                                .ok_or_else(|| invalid("AVC 422 bypass overflow"))?;
                        }
                    }
                }
                ChromaMode::Vertical => {
                    for row in 1..16 {
                        for col in 0..8 {
                            let at = row * 8 + col;
                            residual[at] = residual[at]
                                .checked_add(residual[at - 8])
                                .ok_or_else(|| invalid("AVC 422 bypass overflow"))?;
                        }
                    }
                }
                _ => {}
            }
        }
        for row in 0..16 {
            for col in 0..8 {
                let at = row * 8 + col;
                plane[(y + row) * stride + x + col] =
                    (i64::from(prediction[at]) + i64::from(residual[at])).clamp(0, i64::from(max))
                        as u16;
            }
        }
    }
    Ok(())
}
