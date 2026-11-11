//! Annex I.8.5.7 depth/disparity samples (I-300..I-312).
use super::CameraTransform;
use crate::{Result, invalid};

#[derive(Debug)]
pub struct SampleArray {
    pub dimensions: [usize; 2],
    /// Defined by partIdc=2; false for the other two derivations.
    pub horizontal_split: bool,
    /// Raster order. partIdc=1 retains depth, otherwise backward disparity.
    pub values: Vec<i64>,
}

pub fn required_bytes(block: [usize; 2]) -> Result<usize> {
    if block.iter().any(|&n| n == 0 || n > 64) {
        return Err(invalid("HEVC disparity block dimensions exceed range"));
    }
    block[0]
        .checked_mul(block[1])
        .and_then(|n| n.checked_mul(std::mem::size_of::<i64>()))
        .and_then(|n| n.checked_add(std::mem::size_of::<SampleArray>()))
        .ok_or_else(|| invalid("HEVC disparity sample storage overflow"))
}

/// The caller resolves the current-AU reference depth view before this process.
/// Coordinates are coded luma coordinates; cropping is not applied here.
pub fn derive(
    depth: &[u16],
    dimensions: [usize; 2],
    origin: [u32; 2],
    block: [usize; 2],
    disparity: [i64; 2],
    part_idc: u8,
    transform: CameraTransform,
    budget: usize,
) -> Result<SampleArray> {
    let required = required_bytes(block)?;
    if part_idc > 2
        || dimensions.iter().any(|&n| n == 0 || n > u32::MAX as usize)
        || dimensions[0].checked_mul(dimensions[1]) != Some(depth.len())
    {
        return Err(invalid("invalid HEVC disparity depth sample geometry"));
    }
    if required > budget {
        return Err(invalid("HEVC disparity samples exceed memory budget"));
    }
    // Widen before adding the rounding term, including extreme public inputs.
    let reference = [0, 1].map(|i| i128::from(origin[i]) + ((i128::from(disparity[i]) + 2) >> 2));
    let at = |offset: [usize; 2]| -> Result<u16> {
        let p = [0, 1].map(|i| {
            (reference[i] + offset[i] as i128).clamp(0, dimensions[i] as i128 - 1) as usize
        });
        let sample = depth[p[1] * dimensions[0] + p[0]];
        if u32::from(sample) >= transform.sample_count() {
            return Err(invalid("HEVC disparity depth sample exceeds bit depth"));
        }
        Ok(sample)
    };
    let [width, height] = block;
    let horizontal_split = if part_idc != 2 {
        false
    } else if width % 8 != 0 || height % 8 != 0 {
        height % 8 != 0
    } else {
        (at([0, 0])? < at([width - 1, height - 1])?) == (at([width - 1, 0])? < at([0, height - 1])?)
    };
    let sub = match part_idc {
        0 => block,
        1 => [1, 1],
        _ if horizontal_split => [8, 4],
        _ => [4, 8],
    };
    if width % sub[0] != 0 || height % sub[1] != 0 {
        return Err(invalid("HEVC disparity partition does not tile block"));
    }
    let mut values = Vec::new();
    values
        .try_reserve_exact(width * height)
        .map_err(|_| invalid("HEVC disparity sample allocation failed"))?;
    let retained = values
        .capacity()
        .checked_mul(std::mem::size_of::<i64>())
        .and_then(|n| n.checked_add(std::mem::size_of::<SampleArray>()))
        .ok_or_else(|| invalid("HEVC disparity sample storage overflow"))?;
    if retained > budget {
        return Err(invalid("HEVC disparity samples exceed memory budget"));
    }
    values.resize(width * height, 0);
    for y in (0..height).step_by(sub[1]) {
        for x in (0..width).step_by(sub[0]) {
            let corners = [
                at([x, y])?,
                at([x + sub[0] - 1, y])?,
                at([x, y + sub[1] - 1])?,
                at([x + sub[0] - 1, y + sub[1] - 1])?,
            ];
            let maximum = u32::from(*corners.iter().max().unwrap());
            let value = if part_idc == 1 {
                i64::from(maximum)
            } else {
                transform.sample(maximum)?.backward
            };
            for row in y..y + sub[1] {
                values[row * width + x..row * width + x + sub[0]].fill(value);
            }
        }
    }
    Ok(SampleArray {
        dimensions: block,
        horizontal_split,
        values,
    })
}

/// Bind to an already reconstructed, uncropped monochrome depth picture.
/// The caller must select the depth-role layer of the requested reference view
/// in the current AU; monochrome alone does not establish layer identity.
pub fn from_picture(
    picture: &super::super::super::hevc_picture::Picture,
    origin: [u32; 2],
    block: [usize; 2],
    disparity: [i64; 2],
    part_idc: u8,
    transform: CameraTransform,
    budget: usize,
) -> Result<SampleArray> {
    if picture.planes[1].dimensions() != [0, 0]
        || picture.planes[2].dimensions() != [0, 0]
        || picture.planes[0].dimensions() != picture.dimensions.map(|n| n as usize)
        || !picture.planes[0].complete()
        || !(8..=16).contains(&picture.depth[0])
        || transform.sample_count() != 1u32 << picture.depth[0]
    {
        return Err(invalid(
            "HEVC disparity requires matching reconstructed depth picture",
        ));
    }
    derive(
        picture.planes[0].samples(),
        picture.dimensions.map(|n| n as usize),
        origin,
        block,
        disparity,
        part_idc,
        transform,
        budget,
    )
}

/// partIdc=1 requires no camera parameters. Supply only the reconstructed
/// depth picture's precision; a zero transform is used solely for range checks.
pub fn depth_from_picture(
    picture: &super::super::super::hevc_picture::Picture,
    origin: [u32; 2],
    block: [usize; 2],
    disparity: [i64; 2],
    budget: usize,
) -> Result<SampleArray> {
    let precision = CameraTransform::new([0; 4], 0, picture.depth[0])?;
    from_picture(picture, origin, block, disparity, 1, precision, budget)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn corner_maximum_not_interior_and_partition_geometry() {
        let t = CameraTransform::new([128, 0, 0, 0], 0, 8).unwrap();
        let mut depth = vec![1; 64];
        depth[27] = 255;
        let a = derive(&depth, [8, 8], [0, 0], [8, 8], [0, 0], 0, t, 4096).unwrap();
        assert!(a.values.iter().all(|&v| v == 1));
        let a = derive(&depth, [8, 8], [0, 0], [8, 8], [0, 0], 1, t, 4096).unwrap();
        assert_eq!(a.values[27], 255);
        for (block, horizontal) in [([8, 4], true), ([4, 8], false)] {
            assert_eq!(
                derive(&depth, [8, 8], [0, 0], block, [0, 0], 2, t, 4096)
                    .unwrap()
                    .horizontal_split,
                horizontal
            );
        }
        assert!(derive(&depth, [8, 8], [0, 0], [4, 4], [0, 0], 2, t, 4096).is_err());
    }
    #[test]
    fn bounds_budget_and_extreme_signed_rounding() {
        let t = CameraTransform::new([128, 0, 0, 0], 0, 8).unwrap();
        let depth = [1, 2, 3, 4];
        let budget = required_bytes([8, 8]).unwrap();
        assert!(derive(&depth, [2, 2], [0, 0], [8, 8], [0, 0], 0, t, budget - 1).is_err());
        let a = derive(
            &depth,
            [2, 2],
            [u32::MAX, u32::MAX],
            [8, 8],
            [i64::MIN, i64::MAX],
            0,
            t,
            budget,
        )
        .unwrap();
        assert!(a.values.iter().all(|&v| v == 3));
        for vector in [-3, -2, -1, 0, 1, 2, 3] {
            let a = derive(&depth, [2, 2], [0, 0], [1, 1], [vector, 0], 1, t, 4096).unwrap();
            assert_eq!(a.values[0], if vector >= 2 { 2 } else { 1 });
        }
        assert!(derive(&[256], [1, 1], [0, 0], [1, 1], [0, 0], 1, t, 4096).is_err());
        assert!(derive(&[], [0, 0], [0, 0], [1, 1], [0, 0], 0, t, 4096).is_err());
        assert!(derive(&depth, [2, 3], [0, 0], [1, 1], [0, 0], 0, t, 4096).is_err());
        assert!(derive(&depth, [2, 2], [0, 0], [1, 1], [0, 0], 3, t, 4096).is_err());
    }
}
