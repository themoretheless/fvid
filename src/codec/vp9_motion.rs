//! VP9 motion-vector entropy and subpixel interpolation primitives.
use super::{
    vp9_bool::BoolDecoder, vp9_picture::Plane, vp9_probs::Probabilities, vp9_tables::SUBPEL_FILTERS,
};
use crate::{Result, invalid};
fn tree(b: &mut BoolDecoder<'_>, t: &[i8], p: &[u8]) -> Result<u8> {
    let mut at = 0;
    loop {
        let v = t[at + usize::from(b.read(p[at / 2])?)];
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
    let hp = allow_hp && high_precision(best);
    let joint = tree(b, &[0, 2, -1, 4, -2, -3], &p.mv_joint)?;
    let mut mv = best;
    for comp in 0..2 {
        if (comp == 0 && joint >= 2) || (comp == 1 && joint & 1 != 0) {
            let sign = b.read(p.mv_sign[comp])?;
            let class = tree(
                b,
                &[
                    0, 2, -1, 4, 6, 8, -2, -3, 10, 12, -4, -5, -6, 14, 16, 18, -7, -8, -9, -10,
                ],
                &p.mv_class[comp],
            )? as usize;
            let mag = if class == 0 {
                let bit = usize::from(b.read(p.mv_class0_bit[comp])?);
                let frac = tree(b, &[0, 2, -1, 4, -2, -3], &p.mv_class0_fr[comp][bit])?;
                let high = if hp {
                    b.read(p.mv_class0_hp[comp])?
                } else {
                    true
                };
                (((bit as i32) << 3) | (i32::from(frac) << 1) | i32::from(high)) + 1
            } else {
                let mut bits = 0;
                for i in 0..class {
                    bits |= i32::from(b.read(p.mv_bits[comp][i])?) << i;
                }
                let frac = tree(b, &[0, 2, -1, 4, -2, -3], &p.mv_fr[comp])?;
                let high = if hp { b.read(p.mv_hp[comp])? } else { true };
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
pub fn interpolate(
    reference: &Plane,
    visible: [usize; 2],
    origin: [i64; 2],
    step: [usize; 2],
    size: [usize; 2],
    filter: usize,
    depth: u8,
) -> Result<Vec<u16>> {
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
    if step == [16, 16] && origin.iter().all(|v| v & 15 == 0) {
        let mut out = Vec::with_capacity(w * h);
        for y in 0..h {
            let sy = ((origin[1] >> 4) + y as i64).clamp(0, vh as i64 - 1) as usize;
            for x in 0..w {
                let sx = ((origin[0] >> 4) + x as i64).clamp(0, vw as i64 - 1) as usize;
                out.push(reference.samples[sy * reference.width + sx]);
            }
        }
        return Ok(out);
    }
    let mut intermediate = vec![0i32; w * intermediate_height];
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
    let mut out = vec![0; w * h];
    for r in 0..h {
        for c in 0..w {
            let y = (origin[1] & 15) as usize + step[1] * r;
            let mut sum = 0;
            for tap in 0..8 {
                sum += i32::from(SUBPEL_FILTERS[filter][y & 15][tap])
                    * intermediate[((y >> 4) + tap) * w + c];
            }
            out[r * w + c] = ((sum + 64) >> 7).clamp(0, max) as u16;
        }
    }
    Ok(out)
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
        for filter in 0..4 {
            assert_eq!(
                interpolate(&p, [4, 4], [0, 0], [16, 16], [4, 4], filter, 8).unwrap(),
                p.samples
            );
        }
        assert_eq!(
            interpolate(&p, [4, 4], [8, 8], [16, 16], [1, 1], 3, 8).unwrap(),
            [20]
        );
        assert_eq!(
            interpolate(&p, [4, 4], [-128, -128], [16, 16], [2, 2], 0, 8).unwrap(),
            [0; 4]
        );
    }
}
