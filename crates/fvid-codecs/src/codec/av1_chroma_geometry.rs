//! Separate horizontal/vertical AV1 plane geometry; shared by reconstruction storage.
use super::super::av1_sequence::Color;
use crate::{Result, invalid};
pub(super) fn shifts(color: &Color, p: usize) -> [usize; 2] {
    if p == 0 {
        [0; 2]
    } else {
        color.subsampling.map(usize::from)
    }
}
/// AV1 CFL subsampling produces luma with three fractional bits.
pub(super) fn cfl_luma(
    color: &Color,
    samples: &[u16],
    stride: usize,
    chroma: [usize; 2],
    max_luma: [usize; 2],
) -> i32 {
    let [sx, sy] = shifts(color, 1);
    let x = (chroma[0] << sx).min(max_luma[0] - (1 << sx));
    let y = (chroma[1] << sy).min(max_luma[1] - (1 << sy));
    let mut total = 0;
    for dy in 0..=sy {
        for dx in 0..=sx {
            total += i32::from(samples[(y + dy) * stride + x + dx]);
        }
    }
    total << (3 - sx - sy)
}
pub(super) fn plane_size(color: &Color, p: usize, luma: [usize; 2]) -> [usize; 2] {
    let sub = if p == 0 {
        [false; 2]
    } else {
        color.subsampling
    };
    std::array::from_fn(|axis| luma[axis].div_ceil(1 << usize::from(sub[axis])))
}
/// Plane samples plus the three existing full-grid transform/decoded maps.
/// Preserve the old 4:2:0 admission margin; larger layouts need more samples.
pub(super) fn storage_bytes(color: &Color, mi: [usize; 2]) -> Result<usize> {
    let overflow = || invalid("AV1 image allocation overflow");
    let luma = [
        mi[0].checked_mul(4).ok_or_else(overflow)?,
        mi[1].checked_mul(4).ok_or_else(overflow)?,
    ];
    let samples = (0..3)
        .try_fold(0usize, |n, p| {
            let [w, h] = plane_size(color, p, luma);
            n.checked_add(w.checked_mul(h)?.checked_mul(std::mem::size_of::<u16>())?)
        })
        .ok_or_else(overflow)?;
    let area = mi[0].checked_mul(mi[1]).ok_or_else(overflow)?;
    let metadata = area
        .checked_mul(3 * (std::mem::size_of::<[usize; 2]>() + std::mem::size_of::<bool>()))
        .ok_or_else(overflow)?;
    let actual = samples.checked_add(metadata).ok_or_else(overflow)?;
    Ok(actual.max(area.checked_mul(100).ok_or_else(overflow)?))
}
#[cfg(test)]
mod tests {
    use super::*;
    fn color(subsampling: [bool; 2]) -> Color {
        Color {
            depth: 8,
            monochrome: false,
            primaries: 2,
            transfer: 2,
            matrix: 2,
            full_range: false,
            subsampling,
            chroma_position: 0,
            separate_uv_delta_q: false,
        }
    }
    #[test]
    fn cfl_preserves_three_fractional_bits_for_each_layout_and_edge() {
        let samples = [10, 20, 30, 40, 50, 60, 70, 80];
        for (sub, first, last) in [
            ([true, true], 280, 440),
            ([true, false], 120, 600),
            ([false, false], 80, 640),
        ] {
            let c = color(sub);
            assert_eq!(cfl_luma(&c, &samples, 4, [0, 0], [4, 2]), first);
            assert_eq!(cfl_luma(&c, &samples, 4, [99, 99], [4, 2]), last);
            assert_eq!(shifts(&c, 0), [0, 0]);
        }
    }
    #[test]
    fn independent_axes_include_odd_last_samples() {
        for (sub, expected) in [
            ([true, true], [33, 25]),
            ([true, false], [33, 49]),
            ([false, false], [65, 49]),
        ] {
            let c = color(sub);
            assert_eq!(plane_size(&c, 0, [65, 49]), [65, 49]);
            for p in 1..3 {
                assert_eq!(plane_size(&c, p, [65, 49]), expected);
                assert_eq!(plane_size(&c, p, [1, 1]), [1, 1]);
            }
        }
    }
    #[test]
    fn full_chroma_storage_is_admitted_instead_of_using_420_estimate() {
        let area = 16 * 32;
        let meta = 3 * (std::mem::size_of::<[usize; 2]>() + std::mem::size_of::<bool>());
        assert_eq!(
            storage_bytes(&color([true, true]), [16, 32]).unwrap(),
            area * 100
        );
        assert_eq!(
            storage_bytes(&color([true, false]), [16, 32]).unwrap(),
            area * (64 + meta).max(100)
        );
        assert_eq!(
            storage_bytes(&color([false, false]), [16, 32]).unwrap(),
            area * (96 + meta).max(100)
        );
        assert!(storage_bytes(&color([false, false]), [usize::MAX, 16]).is_err());
    }
}
