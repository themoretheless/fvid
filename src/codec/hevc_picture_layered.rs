//! Reconstruction with an explicitly resolved Annex F active-layer binding.
use super::{Picture, Reference, decode_slices_internal};
use crate::codec::hevc_slice::{LayeredSliceContext, SliceHeader};
use crate::{Result, invalid};

/// Reconstruct one complete picture in the context's layer.
///
/// Callers derive the final POC and reference lists before reconstruction, and
/// commit their POC/DPB state only on success. Inter-layer references must have
/// been bound to this access unit and prepared with the PPS sample/motion
/// resampling APIs; they are already in the current representation. This entry
/// point does not replace the access-unit/DPB owner.
pub fn decode_layered(
    context: &LayeredSliceContext<'_>,
    slices: &[SliceHeader],
    poc: i32,
    lists: &[[Vec<Reference>; 2]],
    budget: usize,
) -> Result<Picture> {
    if slices.is_empty() || slices.len() != lists.len() {
        return Err(invalid("invalid HEVC layered picture slice set"));
    }
    let first = &slices[0];
    for (slice, lists) in slices.iter().zip(lists) {
        context.validate_poc_header(slice)?;
        if slice.nal != first.nal
            || slice.pps_id != first.pps_id
            || slice.poc_lsb != first.poc_lsb
            || slice.picture_output != first.picture_output
            || slice.no_output_of_prior_pictures != first.no_output_of_prior_pictures
            || slice.layered != first.layered
        {
            return Err(invalid("HEVC layered slices disagree on picture identity"));
        }
        let syntax = slice.layered.as_ref().unwrap();
        for reference in lists.iter().flatten() {
            if reference.layer_id > 62 {
                return Err(invalid("HEVC layered picture reference layer out of range"));
            }
            if reference.picture.is_none()
                && (!context.pps.pps.current_picture_reference
                    || reference.layer_id != context.sps.layer_id
                    || reference.poc != poc
                    || !reference.long_term)
            {
                return Err(invalid(
                    "invalid HEVC layered current-picture reference descriptor",
                ));
            }
            if reference.layer_id != context.sps.layer_id
                && (!syntax.reference_layers.contains(&reference.layer_id)
                    || reference.poc != poc
                    || !reference.long_term
                    || reference.picture.is_none())
            {
                return Err(invalid(
                    "HEVC layered picture has invalid inter-layer reference",
                ));
            }
        }
    }
    decode_slices_internal(
        &context.sps.sps,
        &context.pps.pps,
        slices,
        poc,
        lists,
        budget,
        context.sps.layer_id,
    )
}
