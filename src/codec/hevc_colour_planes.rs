//! Independent reconstruction and reference projection for separate planes.
use super::{
    hevc_motion::Reference,
    hevc_picture::{self, Picture},
    hevc_pps::Pps,
    hevc_slice::SliceHeader,
    hevc_sps::Sps,
};
use crate::{Result, invalid};
use std::sync::Arc;

pub(super) fn decode(
    sps: &Sps,
    pps: &Pps,
    slices: &[SliceHeader],
    poc: i32,
    lists: &[[Vec<Reference>; 2]],
    budget: usize,
) -> Result<Picture> {
    if slices.len() != lists.len() || sps.chroma_format != 3 {
        return Err(invalid("invalid HEVC separate colour plane slice set"));
    }
    // Three reconstruction scratch sets plus the assembled output. Check
    // before cloning any payloads or allocating plane storage.
    let pixels = (sps.dimensions[0] as usize)
        .checked_mul(sps.dimensions[1] as usize)
        .ok_or_else(|| invalid("HEVC colour plane storage overflow"))?;
    let payloads = slices
        .iter()
        .try_fold(0usize, |n, h| n.checked_add(h.rbsp.len()))
        .ok_or_else(|| invalid("HEVC colour plane payload size overflow"))?;
    let required = pixels
        .checked_mul(100)
        .and_then(|n| n.checked_add(3 * 65536))
        .and_then(|n| n.checked_add(payloads))
        .ok_or_else(|| invalid("HEVC colour plane storage overflow"))?;
    if required > budget {
        return Err(invalid("HEVC colour planes exceed decode budget"));
    }
    let mut mono = sps.clone();
    mono.chroma_format = 0;
    mono.separate_colour_plane = false;
    mono.depth = [sps.depth[0]; 2];
    let mut children = Vec::with_capacity(3);
    for plane in 0..3 {
        let indices: Vec<_> = slices
            .iter()
            .enumerate()
            .filter_map(|(i, h)| (h.colour_plane == plane).then_some(i))
            .collect();
        if indices.is_empty() {
            return Err(invalid("HEVC access unit is missing a colour plane"));
        }
        let headers: Vec<_> = indices.iter().map(|&i| slices[i].clone()).collect();
        let projected = indices.iter().map(|&i| {
            let mut result: [Vec<Reference>; 2] = [vec![], vec![]];
            for l in 0..2 {
                for reference in &lists[i][l] {
                    let picture = reference.picture.as_ref().map(|picture| {
                        picture.colour_planes.as_ref()
                            .map(|planes| Arc::clone(&planes[plane as usize]))
                            .ok_or_else(|| invalid("HEVC separate colour plane reference has no plane state"))
                    }).transpose()?;
                    result[l].push(Reference { poc: reference.poc, long_term: reference.long_term, picture });
                }
            }
            Ok(result)
        }).collect::<Result<Vec<_>>>()?;
        children.push(Arc::new(hevc_picture::decode_slices(
            &mono,
            pps,
            &headers,
            poc,
            &projected,
            budget / 3,
        )?));
    }
    let children: [Arc<Picture>; 3] = children
        .try_into()
        .map_err(|_| invalid("invalid HEVC plane count"))?;
    let first = &children[0];
    let picture = Picture {
        #[cfg(test)]
        pcm_luma_samples: children.iter().map(|p| p.pcm_luma_samples).sum(),
        #[cfg(test)]
        cross_component_blocks: 0,
        #[cfg(test)]
        act_blocks: 0,
        #[cfg(test)]
        current_picture_blocks: children.iter().map(|p| p.current_picture_blocks).sum(),
        #[cfg(test)]
        completed_picture_blocks: children.iter().map(|p| p.completed_picture_blocks).sum(),
        #[cfg(test)]
        bipredicted_blocks: children.iter().map(|p| p.bipredicted_blocks).sum(),
        #[cfg(test)]
        fractional_current_chroma_blocks: 0,
        dimensions: sps.dimensions,
        crop: sps.crop,
        depth: [sps.depth[0]; 2],
        planes: std::array::from_fn(|i| children[i].planes[0].clone()),
        sao: first.sao.clone(),
        motion: first.motion.clone(),
        reference_pocs: first.reference_pocs.clone(),
        reference_long_term: first.reference_long_term.clone(),
        colour_planes: Some(children),
    };
    Ok(picture)
}

impl Picture {
    /// Retained allocations owned by this picture, including separate plane
    /// motion/reference state. Children never contain other pictures.
    pub(crate) fn storage_bytes(&self) -> Option<usize> {
        let mut n = std::mem::size_of::<Self>();
        for plane in &self.planes {
            n = n.checked_add(plane.samples().len().checked_mul(3)?)?;
        }
        n = n.checked_add(
            self.motion
                .len()
                .checked_mul(std::mem::size_of::<super::hevc_motion::Motion>())?,
        )?;
        n = n.checked_add(
            self.sao
                .len()
                .checked_mul(std::mem::size_of::<super::hevc_sao::CtuSao>())?,
        )?;
        for l in 0..2 {
            n = n.checked_add(self.reference_pocs[l].len().checked_mul(4)?)?;
            n = n.checked_add(self.reference_long_term[l].len())?;
        }
        if let Some(planes) = &self.colour_planes {
            for plane in planes {
                n = n.checked_add(plane.storage_bytes()?)?;
            }
        }
        Some(n)
    }
}
