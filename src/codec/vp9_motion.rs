//! VP9 motion-vector entropy and subpixel interpolation primitives.
use super::{
    vp9_bool::BoolDecoder,
    vp9_picture::Plane,
    vp9_probs::{Counts, Probabilities, counted},
    vp9_tables::SUBPEL_FILTERS,
};
use crate::{Result, invalid};
use fvid_cpu::interp8_vertical;
fn tree(b: &mut BoolDecoder<'_>, t: &[i8], p: &[u8], counts: &mut [[u32; 2]]) -> Result<u8> {
    let mut at = 0;
    loop {
        let v = t[at + usize::from(counted(b, p[at / 2], &mut counts[at / 2])?)];
        if v <= 0 {
            return Ok((-v) as u8);
        }
        at = v as usize;
    }
}
pub fn high_precision(mv: [i32; 2]) -> bool {
    mv.iter().all(|v| v.unsigned_abs() < 64)
}
pub fn read_vector(
    b: &mut BoolDecoder<'_>,
    p: &Probabilities,
    best: [i32; 2],
    allow_hp: bool,
) -> Result<[i32; 2]> {
    read_vector_counted(b, p, &mut Counts::filled([0; 2]), best, allow_hp)
}
pub(crate) fn read_vector_counted(
    b: &mut BoolDecoder<'_>,
    p: &Probabilities,
    counts: &mut Counts,
    best: [i32; 2],
    allow_hp: bool,
) -> Result<[i32; 2]> {
    let hp = allow_hp && high_precision(best);
    let joint = tree(b, &[0, 2, -1, 4, -2, -3], &p.mv_joint, &mut counts.mv_joint)?;
    let mut mv = best;
    for comp in 0..2 {
        if (comp == 0 && joint >= 2) || (comp == 1 && joint & 1 != 0) {
            let sign = counted(b, p.mv_sign[comp], &mut counts.mv_sign[comp])?;
            let class = tree(
                b,
                &[
                    0, 2, -1, 4, 6, 8, -2, -3, 10, 12, -4, -5, -6, 14, 16, 18, -7, -8, -9, -10,
                ],
                &p.mv_class[comp],
                &mut counts.mv_class[comp],
            )? as usize;
            let mag = if class == 0 {
                let bit = usize::from(counted(
                    b,
                    p.mv_class0_bit[comp],
                    &mut counts.mv_class0_bit[comp],
                )?);
                let frac = tree(
                    b,
                    &[0, 2, -1, 4, -2, -3],
                    &p.mv_class0_fr[comp][bit],
                    &mut counts.mv_class0_fr[comp][bit],
                )?;
                let high = if hp {
                    counted(b, p.mv_class0_hp[comp], &mut counts.mv_class0_hp[comp])?
                } else {
                    // Adaptation counts the implied low bit too.
                    counts.mv_class0_hp[comp][1] += 1;
                    true
                };
                (((bit as i32) << 3) | (i32::from(frac) << 1) | i32::from(high)) + 1
            } else {
                let mut bits = 0;
                for i in 0..class {
                    bits |= i32::from(counted(
                        b,
                        p.mv_bits[comp][i],
                        &mut counts.mv_bits[comp][i],
                    )?) << i;
                }
                let frac = tree(
                    b,
                    &[0, 2, -1, 4, -2, -3],
                    &p.mv_fr[comp],
                    &mut counts.mv_fr[comp],
                )?;
                let high = if hp {
                    counted(b, p.mv_hp[comp], &mut counts.mv_hp[comp])?
                } else {
                    counts.mv_hp[comp][1] += 1;
                    true
                };
                (2 << (class + 2)) + ((bits << 3) | (i32::from(frac) << 1) | i32::from(high)) + 1
            };
            mv[comp] = best[comp]
                .checked_add(if sign { -mag } else { mag })
                .ok_or_else(|| invalid("VP9 motion vector overflow"))?;
        }
    }
    Ok(mv)
}
/// Separable normative 8-tap interpolation, including reference-border extension.
/// Coordinates and steps use 1/16-sample units; filter 0/1/2/3 is regular/smooth/sharp/bilinear.
/// The `scratch` buffer holds the horizontal intermediate results and is reused across calls.
/// The `out` buffer receives the final output and is resized as needed.
pub fn interpolate(
    reference: &Plane,
    visible: [usize; 2],
    origin: [i64; 2],
    step: [usize; 2],
    size: [usize; 2],
    filter: usize,
    depth: u8,
    scratch: &mut Vec<i32>,
    out: &mut Vec<u16>,
) -> Result<()> {
    let [w, h] = size;
    let [vw, vh] = visible;
    if w == 0
        || h == 0
        || w > 64
        || h > 64
        || vw == 0
        || vh == 0
        || vw > reference.width
        || vh > reference.height
        || reference.samples.len() != reference.width.saturating_mul(reference.height)
        || filter >= 4
        || ![8, 10, 12].contains(&depth)
        || step.iter().any(|&s| s == 0 || s > 80)
        || origin.iter().any(|v| v.unsigned_abs() > 1 << 30)
    {
        return Err(invalid("invalid VP9 inter prediction parameters"));
    }
    let intermediate_height = ((h - 1) * step[1] + 15) / 16 + 8;
    out.resize(w * h, 0);
    if step == [16, 16] && origin.iter().all(|v| v & 15 == 0) {
        let mut idx = 0;
        for y in 0..h {
            let sy = ((origin[1] >> 4) + y as i64).clamp(0, vh as i64 - 1) as usize;
            for x in 0..w {
                let sx = ((origin[0] >> 4) + x as i64).clamp(0, vw as i64 - 1) as usize;
                out[idx] = reference.samples[sy * reference.width + sx];
                idx += 1;
            }
        }
        return Ok(());
    }
    let intermediate_len = w * intermediate_height;
    scratch.resize(intermediate_len, 0);
    let intermediate = &mut scratch[..intermediate_len];
    intermediate.fill(0);
    let max = (1i32 << depth) - 1;
    for r in 0..intermediate_height {
        for c in 0..w {
            let x = origin[0] + (step[0] * c) as i64;
            let y = ((origin[1] >> 4) + r as i64 - 3).clamp(0, vh as i64 - 1) as usize;
            let mut sum = 0;
            for tap in 0..8 {
                let xx = ((x >> 4) + tap as i64 - 3).clamp(0, vw as i64 - 1) as usize;
                sum += i32::from(SUBPEL_FILTERS[filter][(x & 15) as usize][tap])
                    * i32::from(reference.samples[y * reference.width + xx]);
            }
            intermediate[r * w + c] = ((sum + 64) >> 7).clamp(0, max);
        }
    }
    for r in 0..h {
        let y = (origin[1] & 15) as usize + step[1] * r;
        let base = y >> 4;
        let coef = SUBPEL_FILTERS[filter][y & 15].map(i32::from);
        let rows: [&[i32]; 8] =
            std::array::from_fn(|t| &intermediate[(base + t) * w..(base + t) * w + w]);
        interp8_vertical(rows, &coef, max, &mut out[r * w..r * w + w]);
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn interpolation_integer_identity_bilinear_and_border_extension() {
        let p = Plane {
            width: 4,
            height: 4,
            samples: (0..16).map(|v| v * 8).collect(),
        };
        let mut scratch = Vec::new();
        let mut out = Vec::new();
        for filter in 0..4 {
            interpolate(
                &p,
                [4, 4],
                [0, 0],
                [16, 16],
                [4, 4],
                filter,
                8,
                &mut scratch,
                &mut out,
            )
            .unwrap();
            assert_eq!(out, p.samples);
        }
        interpolate(
            &p,
            [4, 4],
            [8, 8],
            [16, 16],
            [1, 1],
            3,
            8,
            &mut scratch,
            &mut out,
        )
        .unwrap();
        assert_eq!(out, [20]);
        interpolate(
            &p,
            [4, 4],
            [-128, -128],
            [16, 16],
            [2, 2],
            0,
            8,
            &mut scratch,
            &mut out,
        )
        .unwrap();
        assert_eq!(out, [0; 4]);
    }
}
