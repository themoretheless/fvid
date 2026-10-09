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
    // Conservative reconstruction scratch reservation for all three planes. Check
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
        let mut n = std::mem::size_of::<Self>() + 2 * std::mem::size_of::<usize>();
        // The assembled picture shares sample/availability allocations with
        // its children. Charge those allocations exactly once, in the children.
        if self.colour_planes.is_none() {
            for plane in &self.planes {
                n = n.checked_add(plane.storage_bytes()?)?;
            }
        }
        n = n.checked_add(
            self.motion
                .capacity()
                .checked_mul(std::mem::size_of::<super::hevc_motion::Motion>())?,
        )?;
        n = n.checked_add(
            self.sao
                .capacity()
                .checked_mul(std::mem::size_of::<super::hevc_sao::CtuSao>())?,
        )?;
        for l in 0..2 {
            n = n.checked_add(self.reference_pocs[l].capacity().checked_mul(4)?)?;
            n = n.checked_add(self.reference_long_term[l].capacity())?;
        }
        if let Some(planes) = &self.colour_planes {
            for plane in planes {
                n = n.checked_add(plane.storage_bytes()?)?;
            }
        }
        Some(n)
    }
}

#[cfg(test)]
mod tests {
    use super::super::hevc_decoder::HevcDecoder;

    #[test]
    fn assembled_colour_planes_share_completed_pixel_storage() {
        let bytes = include_bytes!(
            "../../tests/fixtures/playback-errors/hevc-separate-colour-planes-distinct-synthetic.mp4"
        );
        let config_at = bytes.windows(4).position(|v| v == b"hvcC").unwrap();
        let length =
            u32::from_be_bytes(bytes[config_at - 4..config_at].try_into().unwrap()) as usize;
        let config = &bytes[config_at + 4..config_at - 4 + length];
        let mut at = 0;
        let packet = loop {
            let length = u32::from_be_bytes(bytes[at..at + 4].try_into().unwrap()) as usize;
            assert!(length >= 8 && length <= bytes.len() - at);
            if &bytes[at + 4..at + 8] == b"mdat" {
                break &bytes[at + 8..at + length];
            }
            at += length;
        };
        let mut decoder = HevcDecoder::from_configuration(config, 16 << 20).unwrap();
        let decoded = decoder.decode_packet(packet).unwrap().unwrap();
        let children = decoded.picture.colour_planes.as_ref().unwrap();
        let storage = decoded.picture.storage_bytes().unwrap();
        assert!(
            storage >= 9 * 4096,
            "all three sample/availability buffers are charged"
        );
        assert!(
            storage < 18 * 4096,
            "assembled output must not double-charge shared buffers"
        );
        for (plane, child) in decoded.picture.planes.iter().zip(children) {
            assert_eq!(plane.samples().len(), 4096);
            assert_eq!(plane.samples().as_ptr(), child.planes[0].samples().as_ptr());
        }
        let previous = decoded.picture;
        decoder.reset();
        let next = decoder.decode_packet(packet).unwrap().unwrap();
        for (a, b) in previous.planes.iter().zip(&next.picture.planes) {
            assert_eq!(a.samples(), b.samples());
            assert_ne!(a.samples().as_ptr(), b.samples().as_ptr());
        }
    }
}
