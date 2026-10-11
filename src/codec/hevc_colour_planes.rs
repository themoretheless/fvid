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
    layer_id: u8,
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
                    result[l].push(Reference { layer_id: reference.layer_id, poc: reference.poc, long_term: reference.long_term, picture });
                }
            }
            Ok(result)
        }).collect::<Result<Vec<_>>>()?;
        children.push(Arc::new(hevc_picture::decode_slices_internal(
            &mono,
            pps,
            &headers,
            poc,
            &projected,
            budget / 3,
            layer_id,
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
        reference_layers: first.reference_layers.clone(),
        reference_long_term: first.reference_long_term.clone(),
        colour_planes: Some(children),
    };
    Ok(picture)
}

impl Picture {
    /// Additional live metadata needed to translate POC identities while the
    /// original picture may still be held by a playback caller. Pixels stay shared.
    pub(crate) fn poc_translation_bytes(&self) -> Option<usize> {
        let mut n = std::mem::size_of::<Self>() + 2 * std::mem::size_of::<usize>();
        n = n.checked_add(self.sao.len().checked_mul(std::mem::size_of::<super::hevc_sao::CtuSao>())?)?;
        n = n.checked_add(self.motion.len().checked_mul(std::mem::size_of::<super::hevc_motion::Motion>())?)?;
        for list in 0..2 {
            n = n.checked_add(self.reference_pocs[list].len().checked_mul(4)?)?;
            n = n.checked_add(self.reference_long_term[list].len())?;
            n = n.checked_add(self.reference_layers[list].len())?;
        }
        if let Some(children) = &self.colour_planes {
            for child in children {
                // Separate-plane children are monochrome leaves.
                if child.colour_planes.is_some() { return None; }
                n = n.checked_add(child.poc_translation_bytes()?)?;
            }
        }
        Some(n)
    }

    /// Copy only metadata, decrementing every collocated POC identity. Source
    /// Arc pictures remain untouched, including on allocation/range failure.
    /// `budget` covers the extra live copy, not the already retained pixels.
    pub(crate) fn translated_pocs(&self, delta: i64, budget: usize) -> Result<Self> {
        self.translated_layer_pocs(&[delta;63], budget)
    }
    /// Validate saved identities and report whether their reference layers move.
    pub(crate) fn poc_translation_required(&self, deltas: &[i64;63]) -> Result<bool> {
        let mut required=false;
        for list in 0..2 {
            if self.reference_layers[list].len()!=self.reference_pocs[list].len()
                || self.reference_long_term[list].len()!=self.reference_pocs[list].len() {
                return Err(crate::invalid("HEVC saved reference identity arrays disagree"));
            }
            for &layer in &self.reference_layers[list] {
                required |= *deltas.get(usize::from(layer)).ok_or_else(||crate::invalid("HEVC saved reference layer out of range"))?!=0;
            }
        }
        if let Some(children)=&self.colour_planes {
            for child in children { required |= child.poc_translation_required(deltas)?; }
        }
        Ok(required)
    }
    pub(crate) fn translated_layer_pocs(&self, deltas: &[i64;63], budget: usize) -> Result<Self> {
        self.poc_translation_required(deltas)?;
        if self.poc_translation_bytes().is_none_or(|n| n > budget) {
            return Err(invalid("HEVC POC translation exceeds metadata budget"));
        }
        fn copy<T: Copy>(source: &[T]) -> Result<Vec<T>> {
            let mut out = Vec::new();
            out.try_reserve_exact(source.len())
                .map_err(|_| invalid("cannot allocate HEVC POC translation metadata"))?;
            out.extend_from_slice(source);
            Ok(out)
        }
        let mut pocs = [copy(&self.reference_pocs[0])?, copy(&self.reference_pocs[1])?];
        for (list,pocs) in pocs.iter_mut().enumerate() {
            for (index,poc) in pocs.iter_mut().enumerate() {
                let delta=deltas[usize::from(self.reference_layers[list][index])];
                *poc = i64::from(*poc).checked_sub(delta)
                    .and_then(|v| i32::try_from(v).ok())
                    .ok_or_else(|| invalid("HEVC translated reference POC overflow"))?;
            }
        }
        let colour_planes = if let Some(children) = &self.colour_planes {
            Some([
                Arc::new(children[0].translated_layer_pocs(deltas, budget)?),
                Arc::new(children[1].translated_layer_pocs(deltas, budget)?),
                Arc::new(children[2].translated_layer_pocs(deltas, budget)?),
            ])
        } else { None };
        Ok(Self {
            #[cfg(test)]
            pcm_luma_samples: self.pcm_luma_samples,
            #[cfg(test)]
            cross_component_blocks: self.cross_component_blocks,
            #[cfg(test)]
            act_blocks: self.act_blocks,
            #[cfg(test)]
            current_picture_blocks: self.current_picture_blocks,
            #[cfg(test)]
            completed_picture_blocks: self.completed_picture_blocks,
            #[cfg(test)]
            bipredicted_blocks: self.bipredicted_blocks,
            #[cfg(test)]
            fractional_current_chroma_blocks: self.fractional_current_chroma_blocks,
            dimensions: self.dimensions,
            crop: self.crop,
            depth: self.depth,
            planes: self.planes.clone(),
            sao: copy(&self.sao)?,
            motion: copy(&self.motion)?,
            reference_pocs: pocs,
            reference_layers: [copy(&self.reference_layers[0])?,copy(&self.reference_layers[1])?],
            reference_long_term: [copy(&self.reference_long_term[0])?, copy(&self.reference_long_term[1])?],
            colour_planes,
        })
    }

    /// Additional retained reference allocations when source pixels remain
    /// borrowed. Shared sample/availability buffers are not charged twice.
    pub(crate) fn additional_reference_storage(&self, source: &Self) -> Option<usize> {
        let mut n = self.storage_bytes()?;
        match (&self.colour_planes, &source.colour_planes) {
            (None, None) => {
                for (prepared, original) in self.planes.iter().zip(&source.planes) {
                    if prepared.shares_storage(original) { n = n.checked_sub(prepared.storage_bytes()?)?; }
                }
            }
            (Some(prepared), Some(original)) => {
                for (a,b) in prepared.iter().zip(original) {
                    n = n.checked_sub(a.storage_bytes()?)?.checked_add(a.additional_reference_storage(b)?)?;
                }
            }
            _ => {}
        }
        Some(n)
    }

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
            n = n.checked_add(self.reference_layers[l].capacity())?;
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
    use super::*;

    pub(super) fn synthetic_decoder() -> (HevcDecoder, Vec<u8>) {
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
        let decoder = HevcDecoder::from_configuration(config, 16 << 20).unwrap();
        (decoder, packet.to_vec())
    }
    pub(super) fn synthetic_picture() -> Arc<Picture> {
        let (mut decoder, packet) = synthetic_decoder();
        decoder.decode_packet(&packet).unwrap().unwrap().picture
    }

    #[test]
    fn translated_pocs_preserve_pixels_motion_and_published_picture() {
        let mut original = synthetic_picture();
        let picture = Arc::get_mut(&mut original).unwrap();
        picture.reference_pocs = [vec![30, 29], vec![17]];
        picture.reference_layers = [vec![0,0],vec![0]];
        picture.reference_long_term = [vec![false, true], vec![false]];
        for (i, child) in picture.colour_planes.as_mut().unwrap().iter_mut().enumerate() {
            Arc::get_mut(child).unwrap().reference_pocs = [vec![30 + i as i32], vec![29]];
            Arc::get_mut(child).unwrap().reference_layers = [vec![0],vec![0]];
            Arc::get_mut(child).unwrap().reference_long_term = [vec![false],vec![false]];
        }
        let required = original.poc_translation_bytes().unwrap();
        assert!(original.translated_pocs(32, required - 1).is_err());
        assert!(original.translated_pocs(i64::MAX, required).is_err());
        let shifted = original.translated_pocs(32, required).unwrap();
        assert_eq!(original.reference_pocs, [vec![30, 29], vec![17]]);
        assert_eq!(shifted.reference_pocs, [vec![-2, -3], vec![-15]]);
        assert_eq!(shifted.reference_long_term, original.reference_long_term);
        assert_eq!(shifted.motion, original.motion);
        assert_eq!(shifted.sao, original.sao);
        for (i, (a, b)) in original.colour_planes.as_ref().unwrap().iter()
            .zip(shifted.colour_planes.as_ref().unwrap()).enumerate() {
            assert_eq!(a.reference_pocs, [vec![30 + i as i32], vec![29]]);
            assert_eq!(b.reference_pocs, [vec![-2 + i as i32], vec![-3]]);
            assert_eq!(a.motion, b.motion);
            for (old, new) in a.planes.iter().zip(&b.planes) {
                assert_eq!(old.samples(), new.samples());
                assert_eq!(old.samples().as_ptr(), new.samples().as_ptr());
            }
        }
        for (old, new) in original.planes.iter().zip(&shifted.planes) {
            assert_eq!(old.samples().as_ptr(), new.samples().as_ptr());
        }
    }

    #[test]
    fn assembled_colour_planes_share_completed_pixel_storage() {
        let (mut decoder, packet) = synthetic_decoder();
        let decoded = decoder.decode_packet(&packet).unwrap().unwrap();
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
        let next = decoder.decode_packet(&packet).unwrap().unwrap();
        for (a, b) in previous.planes.iter().zip(&next.picture.planes) {
            assert_eq!(a.samples(), b.samples());
            assert_ne!(a.samples().as_ptr(), b.samples().as_ptr());
        }
    }
}

#[cfg(test)]
#[path = "hevc_motion_identity_tests.rs"]
mod identity_tests;
