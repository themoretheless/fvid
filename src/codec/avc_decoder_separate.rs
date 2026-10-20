//! Independent DPB/POC/motion state for H.264 separate colour planes.
use super::*;

pub(crate) fn project_parameters(sps: &Sps, pps: &Pps, plane: u8) -> Result<(Sps, Pps)> {
    use super::super::{avc::ScalingList, avc_scaling::ScalingMatrices};
    let scaling = ScalingMatrices::for_colour_plane(sps, pps, usize::from(plane))?;
    let mut sequence = sps.clone();
    sequence.chroma_format = 0;
    sequence.separate_colour_plane = false;
    sequence.bit_depth_chroma = sequence.bit_depth_luma;
    sequence.scaling_lists = None;
    let mut picture = pps.clone();
    // Resolve source fallback before projection. The monochrome engine must
    // see this plane's resolved matrices, including its inter matrices.
    let mut lists: Vec<_> = scaling
        .four
        .iter()
        .map(|weights| {
            Some(ScalingList::Explicit(
                super::super::avc_transform::FRAME_SCAN_4X4
                    .iter()
                    .map(|&position| weights[position])
                    .collect(),
            ))
        })
        .collect();
    if picture.transform_8x8 {
        lists.extend(scaling.eight.iter().map(|weights| {
            Some(ScalingList::Explicit(
                super::super::avc_transform8::FRAME_SCAN
                    .iter()
                    .map(|&position| weights[position])
                    .collect(),
            ))
        }));
    }
    picture.scaling_lists = Some(lists);
    Ok((sequence, picture))
}

impl AvcDecoder {
    pub(super) fn decode_separate(
        &mut self,
        packet: &[u8],
        allow_reordering: bool,
        sps: &Sps,
        header: &SliceHeader,
        pps: Pps,
    ) -> Result<Option<Arc<IntraPicture>>> {
        if !matches!(pps.slice_groups, super::super::avc::SliceGroups::Single) {
            return Err(invalid(
                "AVC High 4:4:4 Predictive forbids FMO slice groups",
            ));
        }
        if !header.idr
            && (self.active_sps != Some(sps.id) || self.decoded_sps.as_ref() != Some(sps))
        {
            return Err(invalid("AVC stream/configuration change requires IDR"));
        }
        if self.pending_field.is_some() {
            return Err(invalid(
                "unpaired AVC field before separate-plane configuration",
            ));
        }
        if header.idr {
            if self.has_pending_field() {
                return Err(invalid("unpaired AVC field before IDR"));
            }
            // Replace all histories together, releasing earlier reference
            // geometry before allocating any plane of the new IDR picture.
            self.separate_decoders = None;
        }
        let (width, height) = sps.coded_dimensions();
        let assembly_bytes =
            super::super::avc_picture::decoded_storage(sps, width as usize, height as usize)?;
        let child_budget = self
            .budget
            .checked_sub(assembly_bytes)
            .map(|n| n / 3)
            .filter(|&n| n > 0)
            .ok_or_else(|| invalid("AVC separate-plane decoder exceeds memory budget"))?;
        if self.separate_decoders.is_none() {
            if !header.idr {
                return Err(invalid("AVC separate-plane stream must begin with IDR"));
            }
            // Release an earlier nonseparate DPB before reserving plane state.
            self.dpb = None;
            self.field_dpb = None;
            self.frame_field_dpb = None;
            self.separate_decoders = Some(Box::new(std::array::from_fn(|plane| {
                Self::from_parameters(
                    self.length_size,
                    self.params.clone(),
                    child_budget,
                    Some(plane as u8),
                )
            })));
        }
        let children = self.separate_decoders.as_mut().unwrap();
        let mut output: [Option<Arc<IntraPicture>>; 3] = std::array::from_fn(|_| None);
        for (plane, child) in children.iter_mut().enumerate() {
            // Include parameter-only packets that the owner has already seen.
            child.params.clone_from(&self.params);
            child.budget = child_budget;
            output[plane] = if allow_reordering {
                child.decode_order(packet)?
            } else {
                child.decode(packet)?
            };
        }
        let first = &children[0];
        if children.iter().skip(1).any(|child| {
            child.last_poc != first.last_poc
                || child.active_sps != first.active_sps
                || child.has_pending_field() != first.has_pending_field()
                || child.field_pair_output != first.field_pair_output
        }) {
            return Err(invalid("AVC separate-plane picture states disagree"));
        }
        self.active_sps = first.active_sps;
        self.decoded_sps = Some(sps.clone());
        self.last_poc = first.last_poc;
        self.previous_reference = first.previous_reference;
        self.next_id = first.next_id;
        self.field_pair_output = first.field_pair_output;
        if output.iter().all(Option::is_none) {
            return Ok(None);
        }
        let [Some(y), Some(cb), Some(cr)] = output else {
            return Err(invalid("AVC separate planes completed different pictures"));
        };
        for plane in [&y, &cb, &cr] {
            if plane.coded_width != y.coded_width
                || plane.coded_height != y.coded_height
                || plane.crop != y.crop
                || plane.bit_depth != y.bit_depth
                || plane.bit_depth != sps.bit_depth_luma
                || plane.chroma_format()? != 0
            {
                return Err(invalid(
                    "AVC separate-plane output geometry or precision mismatch",
                ));
            }
        }
        let copy = |source: &[u16]| -> Result<Vec<u16>> {
            let mut values = Vec::new();
            values
                .try_reserve_exact(source.len())
                .map_err(|_| invalid("cannot allocate assembled AVC colour plane"))?;
            values.extend_from_slice(source);
            Ok(values)
        };
        let picture = IntraPicture {
            coded_width: y.coded_width,
            coded_height: y.coded_height,
            crop: y.crop,
            bit_depth: y.bit_depth,
            chroma_bit_depth: y.bit_depth,
            y: copy(&y.y)?,
            cb: copy(&cb.y)?,
            cr: copy(&cr.y)?,
        };
        let actual = picture
            .y
            .capacity()
            .checked_add(picture.cb.capacity())
            .and_then(|n| n.checked_add(picture.cr.capacity()))
            .and_then(|n| n.checked_mul(2));
        if actual.is_none_or(|n| n > assembly_bytes) {
            return Err(invalid("AVC assembled colour planes exceed memory budget"));
        }
        Ok(Some(Arc::new(picture)))
    }
}
