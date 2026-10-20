//! Reconstruction with an explicitly resolved Annex F active-layer binding.
use super::{Picture, Reference, decode_slices_internal};
use crate::codec::hevc_slice::{LayeredSliceContext, SliceHeader};
use crate::{Result, invalid};

/// Bind exact current-AU texture motion to active depth references through the
/// VPS ViewIdx graph. Source identity/POC/AU completion is owned by
/// select_texture_picture; this component does not decide tool availability.
pub fn derive_texture_subblocks(
    context: &LayeredSliceContext<'_>, header: &SliceHeader, texture: &Picture,
    lists: &[Vec<Reference>; 2], rect: [u32; 4],
) -> Result<crate::codec::hevc_motion::texture::Subblocks> {
    context.validate_poc_header(header)?;
    if !context.sps.sps.depth_layer || context.sps.sps.chroma_format != 0 {
        return Err(invalid("HEVC texture motion requires a monochrome depth binding"));
    }
    let extension = context.sps.sps.three_d.as_ref()
        .ok_or_else(|| invalid("HEVC texture motion has no 3D SPS binding"))?;
    super::validate_coding_texture(&context.sps.sps, Some(texture))?;
    let graph = &context.vps.extension.as_ref()
        .ok_or_else(|| invalid("HEVC texture motion requires a VPS layer graph"))?.graph;
    graph.view_index(context.sps.layer_id)?;
    let depth_layers = graph.depth_layer_ids()?.fold(0u64, |mask,layer| mask | (1u64 << layer));
    let mut targets = [Vec::new(), Vec::new()];
    for list in 0..2 {
        if lists[list].len() != usize::from(header.references[list]) || lists[list].len() > 15 {
            return Err(invalid("HEVC texture-motion active references disagree with slice"));
        }
        for reference in &lists[list] {
            if reference.layer_id > 62 || depth_layers & (1u64 << reference.layer_id) == 0 {
                return Err(invalid("HEVC texture-motion depth reference has wrong component role"));
            }
            targets[list].push(crate::codec::hevc_motion::texture::Identity {
                poc: reference.poc, view: graph.view_index(reference.layer_id)?,
            });
        }
    }
    crate::codec::hevc_motion::texture::subblocks(rect,context.sps.sps.dimensions,
        extension.texture_sub_block_log2,|point| texture.texture_merge_candidate_at(point,
            [&targets[0],&targets[1]],header.slice_type==crate::codec::hevc_cabac::SliceType::B,
            |layer|graph.view_index(layer)))
}

/// Select the completed current-AU texture component required by Annex I.
/// This shares immutable picture storage; it never consults temporal DPB history.
pub fn select_texture_picture(
    context:&LayeredSliceContext<'_>,header:&SliceHeader,poc:i32,access_unit:u64,
    completed:&[(crate::codec::hevc_inter_layer::PictureStatus,std::sync::Arc<Picture>)],
)->Result<Option<std::sync::Arc<Picture>>> {
    let needed=context.sps.sps.depth_layer && context.sps.sps.three_d.as_ref()
        .is_some_and(|e|e.intra_contour || e.texture_motion || e.coding_tree_partition);
    if !needed {return Ok(None);}
    context.validate_poc_header(header)?;
    if header.layered.as_ref().is_none_or(|s|s.in_component_prediction!=Some(true)) {return Ok(None);}
    let tools=context.sps.sps.three_d.as_ref().unwrap();
    if !tools.intra_contour && !tools.texture_motion && !super::cqt_enabled(&context.sps.sps,header) {return Ok(None);}
    let graph=&context.vps.extension.as_ref().ok_or_else(||invalid("HEVC TexturePic requires VPS layer graph"))?.graph;
    let layer=graph.view_component_layer(context.sps.layer_id,false)?
        .ok_or_else(||invalid("HEVC TexturePic view component is absent"))?;
    let mut found=completed.iter().filter(|(status,_)|status.identity.layer_id==layer);
    let (status,picture)=found.next().ok_or_else(||invalid("HEVC TexturePic is not completed in current access unit"))?;
    if found.next().is_some() {return Err(invalid("HEVC TexturePic repeats a layer identity"));}
    status.validate()?;
    if status.access_unit!=access_unit || status.identity.poc!=poc {
        return Err(invalid("HEVC TexturePic access unit or POC disagrees"));
    }
    let [width,height]=picture.planes[0].dimensions();
    if width!=picture.dimensions[0] as usize || height!=picture.dimensions[1] as usize
        || width<context.sps.sps.dimensions[0] as usize || height<context.sps.sps.dimensions[1] as usize
        || !picture.planes[0].complete() {
        return Err(invalid("HEVC TexturePic luma is incomplete or does not cover depth picture"));
    }
    if super::cqt_enabled(&context.sps.sps,header) || super::texture_motion_enabled(&context.sps.sps,header) {
        super::validate_coding_texture(&context.sps.sps,Some(picture))?;
    }
    Ok(Some(picture.clone()))
}

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
    decode_layered_with_texture(context, slices, poc, lists, budget, None)
}

/// Reconstruct with the immutable texture component selected from this access unit.
/// The access-unit owner must validate its layer identity, POC and completion.
pub fn decode_layered_with_texture(
    context: &LayeredSliceContext<'_>, slices: &[SliceHeader], poc: i32,
    lists: &[[Vec<Reference>; 2]], budget: usize, texture: Option<&Picture>,
) -> Result<Picture> {
    decode_layered_with_components(context,slices,poc,lists,budget,texture,None)
}

/// AU-owner entry retaining validated view components independently of RPLs.
pub fn decode_layered_with_components(
    context:&LayeredSliceContext<'_>,slices:&[SliceHeader],poc:i32,
    lists:&[[Vec<Reference>;2]],budget:usize,texture:Option<&Picture>,
    components:Option<(&super::ViewComponentPool,crate::codec::hevc_inter_layer::PictureStatus)>,
)->Result<Picture> {
    decode_layered_with_refinement(context,slices,poc,lists,budget,texture,components,None)
}

/// AU-owner entry with a separately bound current-AU depth component pool.
pub fn decode_layered_with_refinement(
    context:&LayeredSliceContext<'_>,slices:&[SliceHeader],poc:i32,
    lists:&[[Vec<Reference>;2]],budget:usize,texture:Option<&Picture>,
    components:Option<(&super::ViewComponentPool,crate::codec::hevc_inter_layer::PictureStatus)>,
    refinement:Option<(&super::ViewComponentPool,crate::codec::hevc_inter_layer::PictureStatus)>,
)->Result<Picture> {
    decode_layered_with_sources(context,slices,poc,lists,budget,texture,components,refinement,None)
}

/// AU-owner reconstruction with validated current-view picture RPS sources.
pub fn decode_layered_with_sources(
    context:&LayeredSliceContext<'_>,slices:&[SliceHeader],poc:i32,
    lists:&[[Vec<Reference>;2]],budget:usize,texture:Option<&Picture>,
    components:Option<(&super::ViewComponentPool,crate::codec::hevc_inter_layer::PictureStatus)>,
    refinement:Option<(&super::ViewComponentPool,crate::codec::hevc_inter_layer::PictureStatus)>,
    residual_sources:Option<&crate::codec::hevc_motion::inter_view_residual::BoundSources<'_>>,
)->Result<Picture> {
    if slices.is_empty() || slices.len() != lists.len() {
        return Err(invalid("invalid HEVC layered picture slice set"));
    }
    let first = &slices[0];
    if let Some(sources)=residual_sources {sources.validate_for(context,first,poc)?;}
    if let Some((pool,current))=components {
        pool.validate_for(context,first,current)?;
        if current.identity.poc!=poc {return Err(invalid("HEVC component pool reconstruction POC disagrees"));}
    }

    if let Some((pool,current))=refinement {
        pool.validate_for(context,first,current)?;
        if !pool.depth() || current.identity.poc!=poc {
            return Err(invalid("HEVC refinement component role or POC disagrees"));
        }
    }

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
        texture,
        Some(context),
        components.map(|v|v.0),
        refinement,
        residual_sources,
    )
}
