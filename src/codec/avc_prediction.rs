//! H.264 Intra16x16 prediction, before residual addition and deblocking.
//! Caller determines neighbour availability using slice boundaries and constrained-intra rules.
use crate::{Result, invalid};

#[derive(Clone, Copy, Debug)]
pub enum Intra16Mode {
    Vertical,
    Horizontal,
    Dc,
    Plane,
}

pub fn intra16(
    mode: Intra16Mode,
    top: Option<&[u16; 16]>,
    left: Option<&[u16; 16]>,
    top_left: Option<u16>,
    bit_depth: u8,
) -> Result<[u16; 256]> {
    if !(8..=14).contains(&bit_depth) {
        return Err(invalid("AVC prediction bit depth out of range"));
    }
    let max = (1u16 << bit_depth) - 1;
    if top.into_iter().chain(left).flatten().any(|&n| n > max) || top_left.is_some_and(|n| n > max)
    {
        return Err(invalid("AVC neighbour exceeds bit depth"));
    }
    let mut output = [0; 256];
    match mode {
        Intra16Mode::Vertical => {
            let t = top.ok_or_else(|| invalid("vertical prediction requires top neighbours"))?;
            for row in output.chunks_exact_mut(16) {
                row.copy_from_slice(t);
            }
        }
        Intra16Mode::Horizontal => {
            let l =
                left.ok_or_else(|| invalid("horizontal prediction requires left neighbours"))?;
            for (y, row) in output.chunks_exact_mut(16).enumerate() {
                row.fill(l[y]);
            }
        }
        Intra16Mode::Dc => {
            let sum = |v: &[u16; 16]| v.iter().map(|&n| u32::from(n)).sum::<u32>();
            let dc = match (top, left) {
                (Some(t), Some(l)) => (sum(t) + sum(l) + 16) >> 5,
                (Some(t), None) => (sum(t) + 8) >> 4,
                (None, Some(l)) => (sum(l) + 8) >> 4,
                _ => 1 << (bit_depth - 1),
            };
            output.fill(dc as u16);
        }
        Intra16Mode::Plane => {
            let t = top.ok_or_else(|| invalid("plane prediction requires top neighbours"))?;
            let l = left.ok_or_else(|| invalid("plane prediction requires left neighbours"))?;
            let corner = i32::from(
                top_left.ok_or_else(|| invalid("plane prediction requires top-left neighbour"))?,
            );
            let gradient = |v: &[u16; 16]| {
                (1..=8)
                    .map(|i| {
                        let low = if i == 8 { corner } else { i32::from(v[7 - i]) };
                        i as i32 * (i32::from(v[7 + i]) - low)
                    })
                    .sum::<i32>()
            };
            let a = 16 * (i32::from(t[15]) + i32::from(l[15]));
            let b = (5 * gradient(t) + 32) >> 6;
            let c = (5 * gradient(l) + 32) >> 6;
            for y in 0..16 {
                for x in 0..16 {
                    output[y * 16 + x] = ((a + b * (x as i32 - 7) + c * (y as i32 - 7) + 16) >> 5)
                        .clamp(0, i32::from(max)) as u16;
                }
            }
        }
    }
    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn directional_modes_preserve_neighbour_lines() {
        let line = std::array::from_fn(|i| (i * 7) as u16);
        let vertical = intra16(Intra16Mode::Vertical, Some(&line), None, None, 8).unwrap();
        for row in vertical.chunks_exact(16) {
            assert_eq!(row, line);
        }
        let horizontal = intra16(Intra16Mode::Horizontal, None, Some(&line), None, 8).unwrap();
        for (y, row) in horizontal.chunks_exact(16).enumerate() {
            assert_eq!(row, [line[y]; 16]);
        }
        assert!(intra16(Intra16Mode::Vertical, None, Some(&line), None, 8).is_err());
        assert!(intra16(Intra16Mode::Horizontal, Some(&line), None, None, 8).is_err());
    }
    #[test]
    fn dc_uses_only_available_neighbours() {
        for depth in [8, 10, 14] {
            assert_eq!(
                intra16(Intra16Mode::Dc, None, None, None, depth).unwrap(),
                [1 << (depth - 1); 256]
            );
        }
        assert_eq!(
            intra16(Intra16Mode::Dc, Some(&[20; 16]), Some(&[40; 16]), None, 8).unwrap(),
            [30; 256]
        );
        assert_eq!(
            intra16(Intra16Mode::Dc, None, Some(&[40; 16]), None, 8).unwrap(),
            [40; 256]
        );
    }
    #[test]
    fn plane_reproduces_constant_and_affine_surface() {
        assert_eq!(
            intra16(
                Intra16Mode::Plane,
                Some(&[80; 16]),
                Some(&[80; 16]),
                Some(80),
                8
            )
            .unwrap(),
            [80; 256]
        );
        // Surface p(x,y)=80+2*x+2*y. Top and left samples lie at y=-1 / x=-1.
        let edge = std::array::from_fn(|i| 78 + 2 * i as u16);
        let plane = intra16(Intra16Mode::Plane, Some(&edge), Some(&edge), Some(76), 8).unwrap();
        for y in 0..16 {
            for x in 0..16 {
                assert_eq!(plane[y * 16 + x], 80 + 2 * x as u16 + 2 * y as u16);
            }
        }
        assert!(intra16(Intra16Mode::Plane, Some(&edge), Some(&edge), None, 8).is_err());
        assert!(intra16(Intra16Mode::Dc, Some(&[256; 16]), None, None, 8).is_err());
    }
}

/// Reconstruct one Intra16x16 luma macroblock, prior to deblocking.
/// AC blocks and coefficients are in raster order; DC is supplied separately.
/// Scaling-list fallback must be resolved by the caller. Transform bypass is not this path.
pub fn reconstruct_intra16(
    prediction: &[u16; 256],
    dc_levels: &[i32; 16],
    ac_levels: &[[i32; 16]; 16],
    qp: u8,
    bit_depth: u8,
    scaling_weights: &[u8; 16],
) -> Result<[u16; 256]> {
    use super::avc_transform::{luma_dc_4x4, reconstruct_4x4, residual_4x4};
    if ac_levels.iter().any(|block| block[0] != 0) {
        return Err(invalid("Intra16 AC input contains a DC coefficient"));
    }
    let dc = luma_dc_4x4(dc_levels, qp, bit_depth, scaling_weights[0])?;
    let mut output = [0; 256];
    for block in 0..16 {
        let bx = (block % 4) * 4;
        let by = (block / 4) * 4;
        let mut predicted = [0; 16];
        for y in 0..4 {
            predicted[y * 4..y * 4 + 4]
                .copy_from_slice(&prediction[(by + y) * 16 + bx..(by + y) * 16 + bx + 4]);
        }
        let residual = residual_4x4(
            &ac_levels[block],
            qp,
            bit_depth,
            scaling_weights,
            Some(dc[block]),
        )?;
        let pixels = reconstruct_4x4(&predicted, &residual, bit_depth)?;
        for y in 0..4 {
            output[(by + y) * 16 + bx..(by + y) * 16 + bx + 4]
                .copy_from_slice(&pixels[y * 4..y * 4 + 4]);
        }
    }
    Ok(output)
}
#[cfg(test)]
mod reconstruction_tests {
    use super::*;
    #[test]
    fn macroblock_reconstruction_places_blocks_and_preserves_prediction() {
        let prediction = std::array::from_fn(|i| (i % 256) as u16);
        let ac = [[0; 16]; 16];
        assert_eq!(
            reconstruct_intra16(&prediction, &[0; 16], &ac, 0, 8, &[16; 16]).unwrap(),
            prediction
        );
        let mut dc = [0; 16];
        dc[0] = 64;
        assert_eq!(
            reconstruct_intra16(&[100; 256], &dc, &ac, 0, 8, &[16; 16]).unwrap(),
            [103; 256]
        );
        let mut ac = ac;
        ac[6][1] = 64;
        let out = reconstruct_intra16(&[100; 256], &[0; 16], &ac, 0, 8, &[16; 16]).unwrap();
        for y in 0..16 {
            for x in 0..16 {
                if (4..8).contains(&y) && (8..12).contains(&x) {
                    assert_ne!(out[y * 16 + x], 100);
                } else {
                    assert_eq!(out[y * 16 + x], 100);
                }
            }
        }
    }
}
