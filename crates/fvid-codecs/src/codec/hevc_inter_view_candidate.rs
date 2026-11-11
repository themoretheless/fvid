//! Inter-view predicted merge candidate, Annex I.8.5.3.2.10 / I-164..I-172.
use super::{Motion, Vector};
use crate::{Result, invalid};

/// Round quarter-sample disparity, add block centre, align to the 8-sample
/// motion grid, then clip (in that order). Offset has the same quarter units.
pub fn reference_position(
    origin: [u32; 2],
    size: [u32; 2],
    disparity: [i64; 2],
    offset: [i32; 2],
    dimensions: [u32; 2],
) -> Result<[u32; 2]> {
    if size.iter().any(|&n| n == 0 || n > 64) || dimensions.contains(&0) {
        return Err(invalid("invalid HEVC inter-view merge block geometry"));
    }
    Ok([0, 1].map(|i| {
        let vector = i128::from(disparity[i]) + i128::from(offset[i]);
        let full = i128::from(origin[i]) + i128::from(size[i] >> 1) + ((vector + 2) >> 2);
        ((full >> 3) << 3).clamp(0, i128::from(dimensions[i]) - 1) as u32
    }))
}

/// Copy a same-POC reference's vector without temporal scaling or depth
/// conversion. Same-list source candidates precede opposite-list candidates;
/// source L1 is considered even for a current P slice. Saved source lists can
/// be canonical unions of multiple slices; target lists must be active lists.
pub fn motion(
    source: Motion,
    source_pocs: [&[i32]; 2],
    target_pocs: [&[i32]; 2],
    bidirectional: bool,
) -> Result<Motion> {
    let mut identities = [None; 2];
    for list in 0..2 {
        if let Some(vector) = source[list] {
            identities[list] = Some(
                *source_pocs[list]
                    .get(vector.reference as usize)
                    .ok_or_else(|| {
                        invalid("HEVC inter-view source reference outside saved list")
                    })?,
            );
        }
    }
    let mut result = [None; 2];
    for list in 0..if bidirectional { 2 } else { 1 } {
        if target_pocs[list].len() > 15 {
            return Err(invalid(
                "HEVC inter-view active reference list exceeds limit",
            ));
        }
        for source_list in [list, 1 - list] {
            let Some(poc) = identities[source_list] else {
                continue;
            };
            if let Some(index) = target_pocs[list]
                .iter()
                .position(|&candidate| candidate == poc)
            {
                result[list] = Some(Vector {
                    reference: index as u8,
                    mv: source[source_list].unwrap().mv,
                });
                break;
            }
        }
    }
    Ok(result)
}

/// Read exact saved PB motion from a reconstructed reference-view picture.
/// Caller resolves the same-role layer in the current AU and supplies the
/// current slice's active POC lists; ViewIdx equality is not a matching rule
/// for I-170 (unlike the separate texture-to-depth merge candidate).
pub fn from_picture(
    picture: &super::super::hevc_picture::Picture,
    origin: [u32; 2],
    size: [u32; 2],
    disparity: [i64; 2],
    offset: [i32; 2],
    target_pocs: [&[i32]; 2],
    bidirectional: bool,
) -> Result<Motion> {
    let position = reference_position(origin, size, disparity, offset, picture.dimensions)?;
    let source = picture.prediction_motion_at(position)?;
    motion(
        source,
        [&picture.reference_pocs[0], &picture.reference_pocs[1]],
        target_pocs,
        bidirectional,
    )
}

/// IV specialization of I-150..I-163. DispRefVec is resolved at each target
/// sub-block origin; its source lookup then applies I-164..I-169 exactly once.
pub fn subblocks(
    picture: &super::super::hevc_picture::Picture,
    rect: [u32; 4],
    dimensions: [u32; 2],
    log2_size: u8,
    mut disparity: impl FnMut([u32; 2]) -> Result<[i64; 2]>,
    target_pocs: [&[i32]; 2],
    bidirectional: bool,
    budget: usize,
) -> Result<super::subblock_motion::Subblocks<Motion>> {
    if picture.dimensions != dimensions {
        return Err(invalid("HEVC IV sub-block source dimensions disagree"));
    }
    let layout = super::subblock_motion::Layout::new(rect, dimensions, log2_size)?;
    super::subblock_motion::derive(layout, [None; 2], budget, |block| {
        let origin = [block[0], block[1]];
        from_picture(
            picture,
            origin,
            [block[2], block[3]],
            disparity(origin)?,
            [0, 0],
            target_pocs,
            bidirectional,
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    fn vector(reference: u8, x: i64) -> Option<Vector> {
        Some(Vector {
            reference,
            mv: [x, -x],
        })
    }
    #[test]
    fn same_list_priority_opposite_list_fallback_and_first_matching_slot() {
        let source = [vector(0, 4), vector(0, 12)];
        let pocs = [&[3][..], &[5][..]];
        let result = motion(source, pocs, [&[5, 3, 3], &[3, 5]], true).unwrap();
        assert_eq!(result, [vector(1, 4), vector(1, 12)]);
        let result = motion(source, pocs, [&[5, 5], &[]], false).unwrap();
        assert_eq!(result, [vector(0, 12), None]);
        assert_eq!(motion(source, pocs, [&[7], &[7]], true).unwrap(), [None; 2]);
        assert_eq!(
            motion([None; 2], [&[], &[]], [&[3], &[]], false).unwrap(),
            [None; 2]
        );
    }
    #[test]
    fn canonical_source_indices_depth_domain_and_invalid_lists() {
        let mut pocs = vec![-1; 256];
        pocs[255] = 8;
        assert_eq!(
            motion(
                [vector(255, 131068), None],
                [&pocs, &[]],
                [&[8], &[]],
                false
            )
            .unwrap(),
            [vector(0, 131068), None]
        );
        assert!(motion([vector(1, 0), None], [&[0], &[]], [&[0], &[]], false).is_err());
        assert!(motion([None; 2], [&[], &[]], [&[0; 16], &[]], false).is_err());
    }
    #[test]
    fn coordinate_order_rounding_and_widened_offsets() {
        assert_eq!(
            reference_position([0, 0], [8, 8], [14, -18], [0, 0], [32, 32]).unwrap(),
            [8, 0]
        );
        // Clip after alignment: the last sample of an unaligned width is 30.
        assert_eq!(
            reference_position([30, 0], [8, 8], [0, 0], [0, 0], [31, 31]).unwrap(),
            [30, 0]
        );
        assert_eq!(
            reference_position(
                [u32::MAX; 2],
                [64, 64],
                [i64::MIN, i64::MAX],
                [i32::MIN, i32::MAX],
                [32, 32]
            )
            .unwrap(),
            [0, 31]
        );
        assert!(reference_position([0; 2], [0, 8], [0; 2], [0; 2], [32; 2]).is_err());
        assert!(reference_position([0; 2], [8; 2], [0; 2], [0; 2], [0, 32]).is_err());
    }
}
