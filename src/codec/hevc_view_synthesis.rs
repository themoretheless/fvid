//! Annex I disparity information and VSP merge candidates, I-181..I-194.
//! Wide vectors retain camera outputs without imposing coded-MVD clipping.
use super::{
    disparity::{CameraTransform, samples},
    texture::Identity,
};
use crate::{Result, invalid};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WideVector {
    pub reference: u8,
    pub mv: [i64; 2],
}
pub type WideMotion = [Option<WideVector>; 2];
fn validate_lists(lists: [&[Identity]; 2], bidirectional: bool) -> Result<()> {
    if lists
        .iter()
        .take(if bidirectional { 2 } else { 1 })
        .any(|list| list.len() > 15)
    {
        return Err(invalid(
            "HEVC disparity active reference list exceeds limit",
        ));
    }
    Ok(())
}

/// I-181..I-187: DI matches both current POC and reference ViewIdx. The
/// shifted variant adds four quarter samples to each active horizontal vector.
pub fn disparity_information(
    lists: [&[Identity]; 2],
    bidirectional: bool,
    reference_view: u16,
    current_poc: i32,
    horizontal: i64,
) -> Result<(WideMotion, WideMotion)> {
    validate_lists(lists, bidirectional)?;
    let mut direct = [None; 2];
    for list in 0..if bidirectional { 2 } else { 1 } {
        if let Some(index) = lists[list]
            .iter()
            .position(|p| p.poc == current_poc && p.view == reference_view)
        {
            direct[list] = Some(WideVector {
                reference: index as u8,
                mv: [horizontal, 0],
            });
        }
    }
    let mut shifted = direct;
    if direct != [None; 2] {
        let value = horizontal
            .checked_add(4)
            .ok_or_else(|| invalid("HEVC shifted disparity overflow"))?;
        for vector in shifted.iter_mut().flatten() {
            vector.mv[0] = value;
        }
    }
    Ok((direct, shifted))
}

/// I-188..I-191: VSP selects only the first matching view, across L0 then L1.
/// POC is deliberately not a condition here; this differs from DI and IV merge.
pub fn reference(
    lists: [&[Identity]; 2],
    bidirectional: bool,
    reference_view: u16,
) -> Result<Option<(usize, u8)>> {
    validate_lists(lists, bidirectional)?;
    for list in 0..if bidirectional { 2 } else { 1 } {
        if let Some(index) = lists[list].iter().position(|p| p.view == reference_view) {
            return Ok(Some((list, index as u8)));
        }
    }
    Ok(None)
}

#[derive(Debug)]
pub struct ViewSynthesis {
    list: usize,
    reference: u8,
    origin: [u32; 2],
    samples: samples::SampleArray,
}
impl ViewSynthesis {
    pub fn subblock_size(&self) -> [usize; 2] {
        if self.samples.horizontal_split {
            [8, 4]
        } else {
            [4, 8]
        }
    }
    pub fn dimensions(&self) -> [usize; 2] {
        self.samples.dimensions
    }
    pub fn samples(&self) -> &[i64] {
        &self.samples.values
    }
    /// I-192..I-194: one active list with horizontal backward disparity.
    pub fn motion_at(&self, point: [usize; 2]) -> Result<WideMotion> {
        let [w, h] = self.samples.dimensions;
        if point[0] >= w || point[1] >= h {
            return Err(invalid("HEVC VSP motion point outside block"));
        }
        let mut motion = [None; 2];
        motion[self.list] = Some(WideVector {
            reference: self.reference,
            mv: [self.samples.values[point[1] * w + point[0]], 0],
        });
        Ok(motion)
    }
    pub fn subblock_count(&self) -> usize {
        let [w, h] = self.dimensions();
        let [sw, sh] = self.subblock_size();
        (w / sw) * (h / sh)
    }
    /// Raster-order sub-block rectangle in coded picture coordinates and motion.
    pub fn subblock(&self, index: usize) -> Result<([u32; 4], WideMotion)> {
        if index >= self.subblock_count() {
            return Err(invalid("HEVC VSP sub-block index outside block"));
        }
        let [w, _] = self.dimensions();
        let [sw, sh] = self.subblock_size();
        let x = index % (w / sw) * sw;
        let y = index / (w / sw) * sh;
        Ok((
            [
                self.origin[0] + x as u32,
                self.origin[1] + y as u32,
                sw as u32,
                sh as u32,
            ],
            self.motion_at([x, y])?,
        ))
    }
}
pub fn required_bytes(block: [usize; 2]) -> Result<usize> {
    samples::required_bytes(block)?
        .checked_add(
            std::mem::size_of::<ViewSynthesis>() - std::mem::size_of::<samples::SampleArray>(),
        )
        .ok_or_else(|| invalid("HEVC VSP storage overflow"))
}

/// Caller resolves the current-AU depth-role layer of reference_view, not the
/// (potentially temporal) texture reference selected by I-188. An unavailable
/// VSP candidate requires neither depth storage nor camera parameters.
pub fn derive(
    depth: Option<&super::super::hevc_picture::Picture>,
    transform: Option<CameraTransform>,
    lists: [&[Identity]; 2],
    bidirectional: bool,
    reference_view: u16,
    rect: [u32; 4],
    disparity: [i64; 2],
    budget: usize,
) -> Result<Option<ViewSynthesis>> {
    let Some((list, reference)) = reference(lists, bidirectional, reference_view)? else {
        return Ok(None);
    };
    let [x, y, w, h] = rect;
    let block = [w as usize, h as usize];
    let required = required_bytes(block)?;
    if x.checked_add(w).is_none() || y.checked_add(h).is_none() {
        return Err(invalid("HEVC VSP block coordinate overflow"));
    }
    if required > budget {
        return Err(invalid("HEVC VSP samples exceed memory budget"));
    }
    let depth =
        depth.ok_or_else(|| invalid("HEVC VSP requires current-AU reference depth picture"))?;
    let transform =
        transform.ok_or_else(|| invalid("HEVC VSP requires reference camera parameters"))?;
    let extra = std::mem::size_of::<ViewSynthesis>() - std::mem::size_of::<samples::SampleArray>();
    let samples = samples::from_picture(
        depth,
        [x, y],
        block,
        disparity,
        2,
        transform,
        budget - extra,
    )?;
    Ok(Some(ViewSynthesis {
        list,
        reference,
        origin: [x, y],
        samples,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn di_requires_current_poc_and_view_vsp_uses_view_only_and_one_list() {
        let a = [
            Identity { poc: 9, view: 2 },
            Identity { poc: 10, view: 2 },
            Identity { poc: 10, view: 2 },
        ];
        let b = [Identity { poc: 10, view: 2 }];
        let (direct, shifted) = disparity_information([&a, &b], true, 2, 10, -7).unwrap();
        assert_eq!(
            direct,
            [
                Some(WideVector {
                    reference: 1,
                    mv: [-7, 0]
                }),
                Some(WideVector {
                    reference: 0,
                    mv: [-7, 0]
                })
            ]
        );
        assert!(shifted.into_iter().flatten().all(|v| v.mv == [-3, 0]));
        assert_eq!(reference([&a, &b], true, 2).unwrap(), Some((0, 0)));
        assert_eq!(reference([&[], &b], true, 2).unwrap(), Some((1, 0)));
        assert_eq!(reference([&[], &b], false, 2).unwrap(), None);
        assert_eq!(
            disparity_information([&a, &b], false, 2, 11, i64::MAX).unwrap(),
            ([None; 2], [None; 2])
        );
        assert!(disparity_information([&a, &b], false, 2, 10, i64::MAX).is_err());
    }
    #[test]
    fn unavailable_candidate_needs_no_depth_camera_or_allocation() {
        assert!(
            derive(None, None, [&[], &[]], true, 2, [0, 0, 8, 8], [0, 0], 0)
                .unwrap()
                .is_none()
        );
        let list = [Identity { poc: 0, view: 2 }];
        assert!(
            derive(
                None,
                None,
                [&list, &[]],
                false,
                2,
                [0, 0, 8, 8],
                [0, 0],
                4096
            )
            .is_err()
        );
        assert!(reference([&[list[0]; 16], &[]], false, 2).is_err());
    }
}
