//! Bound native depth IV source and midpoint disparity, I-297/I-170.
use crate::codec::{
    hevc_cabac::SliceType,
    hevc_motion::{Motion, Reference, disparity::derivation, inter_view_candidate},
    hevc_picture::Picture,
    hevc_slice::{LayeredSliceContext, SliceHeader},
};
use crate::{Result, invalid};
/// Fixed-size active-list scratch. No new frame-sized storage or heap table.
pub struct DepthInterView<'a> {
    source: &'a Picture,
    disparity: [i64; 2],
    reference_view: u16,
    pocs: [[i32; 15]; 2],
    counts: [usize; 2],
    bidirectional: bool,
}
impl DepthInterView<'_> {
    pub fn disparity(&self)->derivation::TextureDisparity {
        derivation::TextureDisparity {
            vector:derivation::Vector{reference_view:self.reference_view,mv:self.disparity},
            refined_mv:self.disparity,
        }
    }
    pub fn candidate(&self, rect: [u32; 4]) -> Result<Option<Motion>> {
        let motion = inter_view_candidate::from_picture(
            self.source,
            [rect[0], rect[1]],
            [rect[2], rect[3]],
            self.disparity,
            [0, 0],
            [
                &self.pocs[0][..self.counts[0]],
                &self.pocs[1][..self.counts[1]],
            ],
            self.bidirectional,
        )?;
        Ok((motion != [None; 2]).then_some(motion))
    }
}
pub fn prepare<'a>(
    context: &LayeredSliceContext<'_>,
    header: &SliceHeader,
    poc: i32,
    lists: &'a [Vec<Reference>; 2],
) -> Result<Option<DepthInterView<'a>>> {
    if !context.sps.sps.depth_layer
        || header.slice_type == SliceType::I
        || context
            .sps
            .sps
            .three_d
            .as_ref()
            .is_none_or(|e| !e.inter_view_disparity[1])
    {
        return Ok(None);
    }
    context.validate_poc_header(header)?;
    let graph = &context
        .vps
        .extension
        .as_ref()
        .ok_or_else(|| invalid("HEVC depth IV requires VPS graph"))?
        .graph;
    if context.sps.sps.chroma_format != 0
        || !graph
            .depth_layer_ids()?
            .any(|id| id == context.sps.layer_id)
    {
        return Err(invalid(
            "HEVC depth IV requires a bound monochrome depth layer",
        ));
    }
    if graph.reference_list_layers(context.sps.layer_id)? == 0 {
        return Ok(None);
    }
    let current_view = graph.view_index(context.sps.layer_id)?;
    let mut pocs = [[0; 15]; 2];
    let mut counts = [0; 2];
    let mut default = None;
    for list in 0..2 {
        if lists[list].len() != usize::from(header.references[list]) || lists[list].len() > 15 {
            return Err(invalid("HEVC depth IV active lists disagree with slice"));
        }
        counts[list] = lists[list].len();
        for (i, reference) in lists[list].iter().enumerate() {
            pocs[list][i] = reference.poc;
            let view = graph.view_index(reference.layer_id)?;
            if !graph.depth_layer_ids()?.any(|id| id == reference.layer_id) {
                return Err(invalid("HEVC depth IV reference has wrong component role"));
            }
            if reference.poc == poc && view < current_view {
                default = Some(default.map_or(view, |old: u16| old.min(view)));
            }
        }
    }
    let Some(view) = default else { return Ok(None) };
    let camera = header
        .layered
        .as_ref()
        .and_then(|h| h.camera.as_ref())
        .ok_or_else(|| {
            invalid("HEVC depth IV requires camera parameters for default reference view")
        })?;
    if camera.view_index != current_view {
        return Err(invalid("HEVC depth IV camera view disagrees with binding"));
    }
    let disparity = derivation::depth(
        view,
        camera.disparity_transform(view, context.sps.sps.depth[0])?,
    )?
    .mv;
    let layer = graph
        .view_component_for_index(view, true)?
        .ok_or_else(|| invalid("HEVC depth IV reference component is absent"))?;
    // DefaultRefViewIdx comes from a same-POC active depth reference. The AU
    // owner has already validated its layer/POC/completion and prepared it;
    // temporal history with the same view is never a substitute here.
    let reference = lists
        .iter()
        .flatten()
        .find(|r| r.poc == poc && r.layer_id == layer)
        .ok_or_else(|| {
            invalid("HEVC depth IV canonical source is not active in current access unit")
        })?;
    if reference.layer_id == context.sps.layer_id || !reference.long_term {
        return Err(invalid(
            "HEVC depth IV source is not an inter-layer current-AU reference",
        ));
    }
    let source = reference
        .picture
        .as_deref()
        .ok_or_else(|| invalid("HEVC depth IV source is not completed"))?;
    if source.dimensions != context.sps.sps.dimensions
        || source.depth != context.sps.sps.depth
        || source.planes[1].dimensions() != [0, 0]
        || source.planes[2].dimensions() != [0, 0]
        || source.planes[0].dimensions() != source.dimensions.map(|n| n as usize)
        || !source.planes[0].complete()
    {
        return Err(invalid(
            "HEVC depth IV source representation or completion disagrees",
        ));
    }
    Ok(Some(DepthInterView {
        source,
        disparity,
        reference_view:view,
        pocs,
        counts,
        bidirectional: header.slice_type == SliceType::B,
    }))
}
