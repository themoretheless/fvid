//! Native HEVC I/P/B reconstruction with independent slices, WPP and in-loop filters.
use super::{
    hevc_block,
    hevc_cabac::{HevcCabac, SliceType, Syntax},
    hevc_inter_syntax::{self, Partition},
    hevc_intra_syntax,
    hevc_motion::{self, Motion, Reference, Spatial},
    hevc_plane::Plane,
    hevc_pps::Pps,
    hevc_qp, hevc_sao,
    hevc_slice::SliceHeader,
    hevc_sps::Sps,
    hevc_transform_tree,
    hevc_tree::{self, Node, Visitor},
};
use crate::{Result, invalid};

#[path = "hevc_coding_map.rs"]
mod coding;

pub(crate) fn cqt_enabled(sps:&Sps,slice:&SliceHeader)->bool {
    sps.depth_layer && sps.three_d.as_ref().is_some_and(|e|e.coding_tree_partition)
        && slice.layered.as_ref().is_some_and(|s|s.in_component_prediction==Some(true))
        && slice.slice_type!=SliceType::I && !matches!(slice.nal.unit_type,16..=23)
}
pub(crate) fn texture_motion_enabled(sps:&Sps,slice:&SliceHeader)->bool {
    sps.depth_layer && sps.three_d.as_ref().is_some_and(|e|e.texture_motion)
        && slice.layered.as_ref().is_some_and(|h|h.in_component_prediction==Some(true))
        && slice.slice_type!=SliceType::I
}

fn depth_iv_enabled(sps:&Sps,slice:&SliceHeader)->bool {
    sps.depth_layer&&slice.slice_type!=SliceType::I&&sps.three_d.as_ref().is_some_and(|e|e.inter_view_disparity[1])
}

fn texture_iv_enabled(sps:&Sps,slice:&SliceHeader)->bool {
    !sps.depth_layer && slice.nal.layer_id!=0 && slice.slice_type!=SliceType::I && sps.three_d.as_ref().is_some_and(|e|e.inter_view_disparity[0]||e.view_synthesis_prediction||e.inter_view_residual_prediction||e.depth_based_partition)
}
pub(crate) fn texture_depth_refinement_enabled(context:&super::hevc_slice::LayeredSliceContext<'_>,slice:&SliceHeader)->Result<bool> {
    texture_component_enabled(context,slice,context.sps.sps.three_d.as_ref().is_some_and(|e|e.depth_reference))
}
pub(crate) fn texture_vsp_enabled(context:&super::hevc_slice::LayeredSliceContext<'_>,slice:&SliceHeader)->Result<bool> {
    let tools=context.sps.sps.three_d.as_ref().is_some_and(|e|e.view_synthesis_prediction);
    let potential=context.vps.extension.as_ref().map(|e|e.graph.reference_list_layers(context.sps.layer_id)).transpose()?.unwrap_or(0)>0;
    texture_component_enabled(context,slice,tools&&potential)
}
pub(crate) fn texture_dbbp_enabled(context:&super::hevc_slice::LayeredSliceContext<'_>,slice:&SliceHeader)->Result<bool> {
    context.validate_poc_header(slice)?;
    Ok(!context.sps.sps.depth_layer && slice.slice_type!=SliceType::I
        && context.sps.sps.three_d.as_ref().is_some_and(|e|e.depth_based_partition)
        && slice.layered.as_ref().is_some_and(|s|s.in_component_prediction==Some(true)))
}
fn texture_component_enabled(
    context:&super::hevc_slice::LayeredSliceContext<'_>,slice:&SliceHeader,enabled:bool,
)->Result<bool> {
    if context.sps.sps.depth_layer || slice.slice_type==SliceType::I
        || !enabled {
        return Ok(false);
    }
    let Some(syntax)=slice.layered.as_ref().filter(|s|s.in_component_prediction==Some(true)) else{return Ok(false)};
    if syntax.reference_layers.is_empty() {return Ok(true);}
    let Some(camera)=syntax.camera.as_ref() else{return Ok(false)};
    let graph=&context.vps.extension.as_ref().ok_or_else(||invalid("HEVC depth refinement requires VPS graph"))?.graph;
    // I-24 requires CpAvailableFlag for every active reference view, not just
    // one camera pair or the view eventually chosen by disparity derivation.
    for &layer in &syntax.reference_layers {
        let view=graph.view_index(layer)?;
        if !camera.pairs.iter().any(|pair|pair.reference_view==view) {return Ok(false);}
    }
    Ok(true)
}
fn iv_enabled(sps:&Sps,slice:&SliceHeader,context:Option<&super::hevc_slice::LayeredSliceContext<'_>>)->bool {
    (depth_iv_enabled(sps,slice)||texture_iv_enabled(sps,slice)) && context.is_some_and(|c|
        c.vps.extension.as_ref().is_some_and(|e|e.graph.reference_list_layers(c.sps.layer_id).is_ok_and(|n|n>0)))
}
fn prepare_texture_disparity<'a>(context:Option<&'a super::hevc_slice::LayeredSliceContext<'a>>,
    sps:&Sps,slice:&SliceHeader,poc:i32,lists:&'a [Vec<Reference>;2],
)->Result<Option<hevc_motion::disparity::derivation::PreparedTexture<'a>>> {
    if !texture_iv_enabled(sps,slice) {return Ok(None);}
    let context=context.ok_or_else(||crate::unsupported("HEVC texture IV requires layered reconstruction binding"))?;
    if context.vps.extension.as_ref().is_none_or(|e|e.graph.reference_list_layers(context.sps.layer_id).is_ok_and(|n|n==0)) {return Ok(None);}
    Ok(Some(hevc_motion::disparity::derivation::PreparedTexture::prepare(context,slice,poc,lists)?))
}

fn validate_coding_texture(sps:&Sps,texture:Option<&Picture>)->Result<()> {
    let texture=texture.ok_or_else(||crate::unsupported("HEVC coding-tree prediction requires current-AU TexturePic"))?;
    validate_contour_texture(sps,Some(texture))?;
    let map=texture.coding.as_ref().ok_or_else(||invalid("HEVC TexturePic has no coded-CU map"))?;
    if map.dimensions!=texture.dimensions {return Err(invalid("HEVC TexturePic coding-map dimensions disagree"));}
    Ok(())
}

pub struct Picture {
    pub(crate) nal:super::hevc_nal::NalHeader,
    pub(super) coding: Option<coding::Map>,
    /// Test-only queue evidence, returned only after successful worker join
    /// and complete plane reconstruction.
    #[cfg(test)]
    pub(crate) queued_dim_blocks: usize,
    #[cfg(test)]
    pub(crate) queued_inter_dc_blocks: usize,
    #[cfg(test)]
    pub(crate) queued_texture_blocks: usize,
    #[cfg(test)]
    pub(crate) queued_inter_blocks: usize,
    #[cfg(test)]
    pub(crate) pcm_luma_samples: usize,
    #[cfg(test)]
    pub(crate) cross_component_blocks: usize,
    #[cfg(test)]
    pub(crate) act_blocks: usize,
    #[cfg(test)]
    pub(crate) current_picture_blocks: usize,
    #[cfg(test)]
    pub(crate) completed_picture_blocks: usize,
    #[cfg(test)]
    pub(crate) bipredicted_blocks: usize,
    #[cfg(test)]
    pub(crate) fractional_current_chroma_blocks: usize,
    pub dimensions: [u32; 2],
    pub crop: [u32; 4],
    pub depth: [u8; 2],
    pub planes: [Plane; 3],
    /// Resolved SAO parameters in raster CTU order, including merged values.
    pub sao: Vec<hevc_sao::CtuSao>,
    /// Collocated motion at the normative 16x16 temporal-prediction grid.
    pub(crate) motion: Vec<Motion>,
    pub(crate) reference_pocs: [Vec<i32>; 2],
    pub(crate) reference_layers: [Vec<u8>; 2],
    pub(crate) reference_long_term: [Vec<bool>; 2],
    /// Independent reconstructed/motion state for separately coded planes.
    pub(crate) colour_planes: Option<[std::sync::Arc<Picture>; 3]>,
}

impl Picture {
    /// Header of the first coded slice, retained through reference preparation.
    pub fn nal_header(&self)->super::hevc_nal::NalHeader {self.nal}
    /// Exact motion at the 8x8-aligned texture location, in quarter samples.
    /// Unlike the temporal collocated map, this preserves adjacent 8x8 PBs.
    pub fn prediction_motion_at(&self, point: [u32; 2]) -> Result<Motion> {
        self.coding_unit_at(point)?;
        self.coding.as_ref().unwrap().motion_at(point)
    }
    /// Derive an Annex I texture merge candidate using this picture's saved,
    /// canonical reference identities. The caller resolves layer ids to ViewIdx.
    pub fn texture_merge_candidate_at(
        &self,
        point: [u32; 2],
        depth_lists: [&[hevc_motion::texture::Identity]; 2],
        bidirectional: bool,
        mut view: impl FnMut(u8) -> Result<u16>,
    ) -> Result<hevc_motion::texture::Candidate> {
        let motion = self.prediction_motion_at(point)?;
        let mut references = [Vec::new(), Vec::new()];
        // Canonical maps may union multiple slices and exceed the per-slice
        // active-list limit. Resolve only the referenced entry into a one-entry
        // owning identity list; target depth lists retain their active ordering.
        let mut selected = motion;
        for list in 0..if bidirectional { 2 } else { 1 } {
            if self.reference_pocs[list].len() != self.reference_layers[list].len() {
                return Err(invalid("HEVC saved texture reference identities disagree"));
            }
            if let Some(vector) = &mut selected[list] {
                let index = vector.reference as usize;
                let poc = *self.reference_pocs[list].get(index)
                    .ok_or_else(|| invalid("HEVC exact texture reference outside saved map"))?;
                let layer = self.reference_layers[list][index];
                references[list].push(hevc_motion::texture::Identity { poc, view: view(layer)? });
                vector.reference = 0;
            }
        }
        hevc_motion::texture::candidate(selected,
            [&references[0], &references[1]], depth_lists, bidirectional)
    }
    /// Geometry of the coded CU containing a luma position, before cropping.
    /// This is immutable decoding metadata, not inferred from image content.
    pub fn coding_unit_at(&self,point:[u32;2])->Result<(u8,Partition)> {
        let map=self.coding.as_ref().ok_or_else(||invalid("HEVC picture has no coded-CU map"))?;
        if map.dimensions!=self.dimensions {return Err(invalid("HEVC picture coding-map dimensions disagree"));}
        map.at(point)
    }
    /// Coded CU skip flag retained with its immutable geometry, before crop.
    /// Motion equality alone cannot distinguish skip from explicit prediction.
    pub fn prediction_is_skipped_at(&self,point:[u32;2])->Result<bool> {
        self.coding_unit_at(point)?;
        self.coding.as_ref().unwrap().skipped_at(point)
    }
    /// Saved base/refined disparity at the containing aligned 8x8 motion-map
    /// location, only where an IV candidate was selected at that location.
    pub fn inter_view_disparity_at(&self,point:[u32;2])->Result<Option<hevc_motion::disparity::derivation::TextureDisparity>> {
        self.coding_unit_at(point)?;
        self.coding.as_ref().unwrap().iv_at(point)
    }
}

/// Conservative reconstruction peak, including the temporary and retained CU map.
pub(crate) fn reconstruction_budget(count:usize)->Option<usize> {
    count.checked_mul(25)?.checked_add(65536)?
        // Wide motion increases both CU cells and queued Inter jobs. Reserve
        // a full motion for each owner at the minimum 4x4 prediction grid.
        .checked_add((count/16).checked_mul(2*std::mem::size_of::<Motion>())?)?
        .checked_add(count/32+2*std::mem::size_of::<usize>()+std::mem::align_of::<usize>()-1)
        // Exact 8x8 texture motion: temporary Vec plus retained Arc peak.
        .and_then(|n| n.checked_add((count/64).checked_mul(2*std::mem::size_of::<Motion>())?))
        .and_then(|n| n.checked_add(2*std::mem::size_of::<usize>()+std::mem::align_of::<usize>()-1))
}
fn residual_reconstruction_budget(count:usize,sps:&Sps,context:Option<&super::hevc_slice::LayeredSliceContext<'_>>)->Option<usize> {
    if sps.depth_layer || !sps.three_d.as_ref().is_some_and(|e|e.inter_view_residual_prediction)
        || context.is_none_or(|c|c.vps.extension.as_ref().is_none_or(|e|e.graph.reference_list_layers(c.sps.layer_id).is_ok_and(|n|n==0))) {return Some(0);}
    // Full residual CUs and inter-view subblocks are at least 8x8. Keep
    // ordinary interpolation scratch resident while residual scratch is live.
    (count/64).checked_mul(std::mem::size_of::<hevc_motion::inter_view_residual::Prediction>()+16)?
        .checked_add(65536)
}
fn dbbp_reconstruction_budget(count:usize,sps:&Sps)->Option<usize> {
    if sps.depth_layer || !sps.three_d.as_ref().is_some_and(|e|e.depth_based_partition) {return Some(0);}
    count.checked_add((count/256).checked_mul(std::mem::size_of::<hevc_motion::dbbp::Prediction>()+16)?)?.checked_add(65536)
}
fn iv_reconstruction_budget(count:usize,active:bool)->Option<usize> {
    reconstruction_budget(count)?.checked_add(if active {
        // Decoder Vec moves into the retained Arc; include their conversion
        // peak and Arc header. No storage is allocated for inactive pictures.
        (count/64).checked_mul(2*std::mem::size_of::<Option<hevc_motion::disparity::derivation::TextureDisparity>>())?
            .checked_add(2*std::mem::size_of::<usize>()+std::mem::align_of::<usize>()-1)?
    }else{0})
}
fn publish_iv_disparity(
    map:&mut [Option<hevc_motion::disparity::derivation::TextureDisparity>],
    dimensions:[u32;2],rect:[u32;4],state:hevc_motion::disparity::derivation::TextureDisparity,
)->Result<()> {
    let [w,h]=dimensions;
    if w%8!=0 || h%8!=0 || (w as usize/8).checked_mul(h as usize/8)!=Some(map.len())
        || rect[2]==0 || rect[3]==0
        || rect[0].checked_add(rect[2]).is_none_or(|end|end>w)
        || rect[1].checked_add(rect[3]).is_none_or(|end|end>h)
    {return Err(invalid("invalid HEVC IV publication geometry"));}
    // The retained map samples aligned locations, not any overlapping PB.
    // In a 4/12 AMP split, x=0 belongs to the first PB and x=8 to the second.
    let stride=w as usize/8;
    for y in rect[1].div_ceil(8)..(rect[1]+rect[3]).div_ceil(8) {
        for x in rect[0].div_ceil(8)..(rect[0]+rect[2]).div_ceil(8) {
            map[y as usize*stride+x as usize]=Some(state);
        }
    }
    Ok(())
}
#[cfg(test)]
mod iv_budget_tests {
    use super::*;
    #[test]
    fn narrow_amp_publication_preserves_aligned_neighbour_state() {
        use hevc_motion::disparity::derivation::{TextureDisparity,Vector};
        let a=TextureDisparity{vector:Vector{reference_view:0,mv:[1,0]},refined_mv:[1,0]};
        let b=TextureDisparity{vector:Vector{reference_view:1,mv:[2,0]},refined_mv:[2,0]};
        for split in [4,12] {
            let mut map=vec![None;4];
            publish_iv_disparity(&mut map,[16,16],[0,0,split,16],a).unwrap();
            publish_iv_disparity(&mut map,[16,16],[split,0,16-split,16],b).unwrap();
            for y in 0..2 {for x in 0..2 {
                assert_eq!(map[y*2+x],Some(if x*8<split as usize{a}else{b}));
            }}
            let mut map=vec![None;4];
            publish_iv_disparity(&mut map,[16,16],[0,0,16,split],a).unwrap();
            publish_iv_disparity(&mut map,[16,16],[0,split,16,16-split],b).unwrap();
            for y in 0..2 {for x in 0..2 {
                assert_eq!(map[y*2+x],Some(if y*8<split as usize{a}else{b}));
            }}
            let saved=map.clone();
            assert!(publish_iv_disparity(&mut map,[16,16],[u32::MAX,0,8,8],a).is_err());
            assert_eq!(map,saved);
        }
    }
    #[test]
    fn iv_state_conversion_peak_is_counted_only_for_active_pictures() {
        for count in [64,1024,4096,16384] {
            let base=reconstruction_budget(count).unwrap();
            let state:Vec<Option<hevc_motion::disparity::derivation::TextureDisparity>>=vec![None;count/64];
            let retained:std::sync::Arc<[_]>=state.clone().into();
            assert_eq!(iv_reconstruction_budget(count,false),Some(base));
            let extra=iv_reconstruction_budget(count,true).unwrap()-base;
            assert!(extra>=std::mem::size_of_val(state.as_slice())+std::mem::size_of_val(retained.as_ref()));
        }
        assert!(iv_reconstruction_budget(usize::MAX,true).is_none());
    }
}

fn reconstruct_ordinary_dc(plane:&mut Plane,origin:[usize;2],log:u8,maximum_log:u8,mode:u8,offset:i64,layer:u8,allowed:[u64;5],filters:[bool;3],depth:u8,tables:Option<&super::hevc_depth_tables::Tables>,scratch:&mut Vec<u16>)->Result<()> {
    let fallback=if tables.is_none() {Some(super::hevc_depth_tables::Tables {bit_depth:depth,layers:vec![super::hevc_depth_tables::Layer {id:layer,values:None}]})}else{None};
    plane.reconstruct_dc_only_split(origin,log,maximum_log,mode,filters,offset,tables.or(fallback.as_ref()).unwrap(),layer,scratch,
        |x,y|skip_angular_allowed(allowed,origin,1usize<<log,x,y))
}
fn reconstruct_wedge(plane:&mut Plane,origin:[usize;2],log:u8,pattern:&[bool],offsets:[i64;2],allowed:[u64;5],layer:u8,depth:u8,tables:Option<&super::hevc_depth_tables::Tables>,residual:&[i32])->Result<()> {
    let fallback=if tables.is_none() {Some(super::hevc_depth_tables::Tables {bit_depth:depth,layers:vec![super::hevc_depth_tables::Layer {id:layer,values:None}]})}else{None};
    let tables=tables.or(fallback.as_ref()).unwrap();
    plane.reconstruct_depth_partition(origin,log,pattern,offsets,tables,layer,residual,&mut Vec::new(),
        |x,y|skip_angular_allowed(allowed,origin,1usize<<log,x,y))
}
fn skip_angular_allowed(mask:[u64;5],origin:[usize;2],size:usize,x:usize,y:usize)->bool {
    let [ox,oy]=origin;
    let index=if y.checked_add(1)==Some(oy) && x>=ox && x-ox<2*size {x-ox}
        else if x.checked_add(1)==Some(ox) && y>=oy && y-oy<2*size {2*size+y-oy}
        else if x.checked_add(1)==Some(ox) && y.checked_add(1)==Some(oy) {4*size}
        else {return false;};
    mask[index/64]&(1<<(index%64))!=0
}
fn component_shifts(component: usize, format: u8) -> [usize; 2] {
    if component == 0 {
        [0, 0]
    } else {
        match format {
            1 => [1, 1],
            2 => [1, 0],
            _ => [0, 0],
        }
    }
}
/// Decode a complete Main/Main10 IDR slice, including WPP, QP deltas and filters.
/// Output planes have coded dimensions; crop is metadata. The supplied budget
/// covers owned pixels, CU/PB metadata and reconstruction scratch storage.
pub fn decode_idr(sps: &Sps, pps: &Pps, slice: &SliceHeader, budget: usize) -> Result<Picture> {
    if !slice.nal.is_idr() || slice.slice_type != SliceType::I {
        return Err(invalid("expected HEVC IDR picture"));
    }
    decode(sps, pps, slice, 0, &[Vec::new(), Vec::new()], budget)
}
pub fn decode(
    sps: &Sps,
    pps: &Pps,
    slice: &SliceHeader,
    poc: i32,
    lists: &[Vec<Reference>; 2],
    budget: usize,
) -> Result<Picture> {
    decode_internal(sps, pps, slice, poc, lists, budget, 0, None, None, None, None, None)
}

fn validate_contour_texture(sps: &Sps, texture: Option<&Picture>) -> Result<()> {
    let texture = texture.ok_or_else(|| crate::unsupported("HEVC contour prediction requires current-AU TexturePic"))?;
    let [width, height] = texture.planes[0].dimensions();
    if !texture.planes[0].complete() || width != texture.dimensions[0] as usize
        || height != texture.dimensions[1] as usize || width < sps.dimensions[0] as usize
        || height < sps.dimensions[1] as usize {
        return Err(invalid("HEVC contour TexturePic luma is incomplete or too small"));
    }
    Ok(())
}

fn decode_internal(
    sps: &Sps,
    pps: &Pps,
    slice: &SliceHeader,
    poc: i32,
    lists: &[Vec<Reference>; 2],
    budget: usize,
    layer_id: u8,
    texture: Option<&Picture>,
    context: Option<&super::hevc_slice::LayeredSliceContext<'_>>,
    view_components:Option<&ViewComponentPool>,
    refinement_components:Option<(&ViewComponentPool,super::hevc_inter_layer::PictureStatus)>,
    residual_sources:Option<&hevc_motion::inter_view_residual::BoundSources<'_>>,
) -> Result<Picture> {
    if sps.depth_layer && sps.three_d.as_ref().is_some_and(|e|e.intra_contour)
        && slice.layered.as_ref().is_some_and(|s|s.in_component_prediction==Some(true)) {
        validate_contour_texture(sps, texture)?;
    }
    if cqt_enabled(sps,slice) || texture_motion_enabled(sps,slice) {validate_coding_texture(sps,texture)?;}
    if texture_motion_enabled(sps,slice) && context.is_none() {
        return Err(crate::unsupported("HEVC texture motion requires layered reconstruction binding"));
    }
    if depth_iv_enabled(sps,slice) && context.is_none() {
        return Err(crate::unsupported("HEVC depth IV requires layered reconstruction binding"));
    }
    if pps.depth_lookup_active && (layer_id==0 || !sps.depth_layer || pps.depth_tables.is_none()) {
        return Err(crate::unsupported("HEVC depth lookup reconstruction requires a resolved depth layer"));
    }
    if sps.chroma_format > 3
        || sps.separate_colour_plane
        || !slice.first
        || slice.address != 0
        || slice.nal.layer_id != layer_id
    {
        return Err(crate::unsupported("unsupported HEVC picture tools"));
    }
    if slice.slice_type != SliceType::I && !(1..=super::hevc_slice::merge_candidate_limit(sps,slice.layered.as_ref(),context)?).contains(&slice.max_merge_candidates) {
        return Err(invalid("invalid HEVC merge candidate count"));
    }
    for list in 0..2 {
        if lists[list].len() != slice.references[list] as usize {
            return Err(invalid("HEVC reference-list length mismatch"));
        }
        for r in &lists[list] {
            if r.picture.is_none()
                && (!pps.current_picture_reference || r.poc != poc || r.layer_id != slice.nal.layer_id || !r.long_term)
            {
                return Err(invalid("invalid HEVC current-picture reference descriptor"));
            }
            if r.picture
                .as_ref()
                .is_some_and(|p| p.dimensions != sps.dimensions || p.depth != sps.depth)
            {
                return Err(invalid("HEVC reference geometry mismatch"));
            }
        }
    }
    let [min_cb, max_cb] = sps.coding_block_log2;
    let [min_tb, max_tb] = sps.transform_block_log2;
    if !(4..=6).contains(&max_cb)
        || !(3..=max_cb).contains(&min_cb)
        || !(2..=min_cb.min(5)).contains(&min_tb)
        || !(min_tb..=max_cb.min(5)).contains(&max_tb)
        || sps.transform_hierarchy_depth[1] > max_cb - min_tb
        || sps.depth.iter().any(|d| !(8..=16).contains(d))
        || sps.id != pps.sps_id
        || pps.id != slice.pps_id
    {
        return Err(invalid("invalid HEVC picture parameters"));
    }
    let [w, h] = sps.dimensions;
    if w == 0
        || h == 0
        || w % (1 << min_cb) != 0
        || h % (1 << min_cb) != 0
        || u64::from(sps.crop[0]) + u64::from(sps.crop[1]) >= u64::from(w)
        || u64::from(sps.crop[2]) + u64::from(sps.crop[3]) >= u64::from(h)
    {
        return Err(invalid("invalid HEVC picture dimensions or crop"));
    }
    let count = (w as usize)
        .checked_mul(h as usize)
        .ok_or_else(|| invalid("HEVC picture size overflow"))?;
    let [chroma_shift_x, chroma_shift_y] = component_shifts(1, sps.chroma_format);
    let required = iv_reconstruction_budget(count,iv_enabled(sps,slice,context))
        .and_then(|n|n.checked_add(residual_reconstruction_budget(count,sps,context)?)?.checked_add(dbbp_reconstruction_budget(count,sps)?))
        .ok_or_else(|| invalid("HEVC picture budget overflow"))?;
    if required > budget {
        return Err(invalid("HEVC picture exceeds decode budget"));
    }
    let tile_layout = pps
        .tiles
        .as_ref()
        .map(|tiles| {
            let side = 1u32 << max_cb;
            super::hevc_tiles::TileLayout::new(
                tiles,
                [w.div_ceil(side), h.div_ceil(side)],
                budget - required,
            )
        })
        .transpose()?;
    let chroma_offsets = slice.chroma_qp_offsets.map(i32::from);
    hevc_qp::components_with_format(
        slice.qp,
        sps.depth,
        chroma_offsets,
        [0; 2],
        sps.chroma_format,
    )?;

    let mut decoder = Decoder {
        #[cfg(test)]
        queued_dim_blocks: 0,
        #[cfg(test)]
        queued_inter_dc_blocks: 0,
        #[cfg(test)]
        queued_texture_blocks: 0,
        #[cfg(test)]
        queued_inter_blocks: 0,
        texture,
        context,
        view_components,
        refinement_components,
        residual_sources,
        depth_iv: context.map(|binding|hevc_motion::depth_inter_view::prepare(binding,slice,poc,lists)).transpose()?.flatten(),
        texture_disparity:prepare_texture_disparity(context,sps,slice,poc,lists)?,
        iv_state:iv_enabled(sps,slice,context).then(||vec![None;count/64]),
        palette: if sps.palette.is_some() {
            Some(super::hevc_palette::Predictor::from_parameters(sps, pps)?)
        } else {
            None
        },
        sps,
        pps,
        slice,
        poc,
        lists,
        slice_start: 0,
        tile_bounds: None,
        tile_layout,
        qp: slice.qp,
        chroma_offsets,
        cu_chroma_offsets: [0; 2],
        chroma_qp_coded: false,
        qp_coded: false,
        qp_prediction: slice.qp,
        qp_grid: vec![slice.qp; count / 64],
        edges: vec![[0; 2]; count / 16],
        cells: vec![Cell::default(); count / 16],
        jobs: None,
        reconstruction: Vec::new(),
        scratch: Vec::new(),
        pred_scratch: Vec::new(),
        transform_scratch: Vec::new(),
        residual_scratch: Vec::new(),
        #[cfg(test)]
        cross_component_blocks: 0,
        #[cfg(test)]
        act_blocks: 0,
        #[cfg(test)]
        current_picture_blocks: 0,
        #[cfg(test)]
        completed_picture_blocks: 0,
        #[cfg(test)]
        bipredicted_blocks: 0,
        #[cfg(test)]
        fractional_current_chroma_blocks: 0,
        planes: [
            Plane::new(w as usize, h as usize, sps.depth[0], count * 3)?,
            if sps.chroma_format == 0 {
                Plane::absent(sps.depth[1])
            } else {
                Plane::new(
                    w as usize >> chroma_shift_x,
                    h as usize >> chroma_shift_y,
                    sps.depth[1],
                    (count * 3) >> (chroma_shift_x + chroma_shift_y),
                )?
            },
            if sps.chroma_format == 0 {
                Plane::absent(sps.depth[1])
            } else {
                Plane::new(
                    w as usize >> chroma_shift_x,
                    h as usize >> chroma_shift_y,
                    sps.depth[1],
                    (count * 3) >> (chroma_shift_x + chroma_shift_y),
                )?
            },
        ],
    };
    let side = 1u32 << max_cb;
    let columns = w.div_ceil(side);
    let rows = h.div_ceil(side);
    let mut sao = Vec::with_capacity(columns as usize * rows as usize);
    let expected = if let Some(layout) = &decoder.tile_layout {
        if pps.entropy_sync {
            layout.rectangles.iter().map(|r| r[3] as usize).sum()
        } else {
            layout.rectangles.len()
        }
    } else if pps.entropy_sync {
        rows as usize
    } else {
        1
    };
    if slice.entropy_substreams.len() != expected {
        return Err(invalid(
            "HEVC entropy substream count does not match tile/row layout",
        ));
    }
    if slice
        .entropy_substreams
        .iter()
        .any(|r| r.start >= r.end || r.end > slice.rbsp.len())
    {
        return Err(invalid("invalid HEVC entropy substream bounds"));
    }
    let mut bins = HevcCabac::new(
        &slice.rbsp[slice.entropy_substreams[0].clone()],
        0,
        slice.slice_type,
        slice.cabac_init,
        slice.qp,
    )?;
    let initial_contexts = bins.contexts()?;
    let mut saved_contexts = None;
    let initial_palette = decoder.palette.clone();
    let mut saved_palette = None;
    if let Some(layout) = decoder.tile_layout.take() {
        sao = decode_tile_ctus(&mut decoder, slice, &layout)?;
        decoder.tile_layout = Some(layout);
    }
    std::thread::scope(|scope| -> Result<()> {
        if decoder.tile_layout.is_some() {
            return Ok(());
        }
        let workers = if count >= 128 * 96 && !pps.constrained_intra {
            let (sender, receiver) = std::sync::mpsc::sync_channel(rows as usize);
            let placeholders = [
                Plane::new(1, 1, 8, 3)?,
                Plane::new(1, 1, 8, 3)?,
                Plane::new(1, 1, 8, 3)?,
            ];
            let planes = std::mem::replace(&mut decoder.planes, placeholders);
            decoder.jobs = Some(sender);
            // Reconstruction owns shared sample planes and waits for every
            // preceding row. More consumers cannot run concurrently; one worker
            // overlaps reconstruction with parsing without per-row wake-up races.
            let num_workers = 1;
            let completed =
                std::sync::Arc::new((std::sync::Mutex::new(0usize), std::sync::Condvar::new()));
            let receiver = std::sync::Arc::new(std::sync::Mutex::new(receiver));
            let planes = std::sync::Arc::new(std::sync::Mutex::new(planes));
            let handles: Vec<_> = (0..num_workers)
                .map(|_| {
                    let receiver = receiver.clone();
                    let planes = planes.clone();
                    let completed = completed.clone();
                    scope.spawn(move || -> Result<()> {
                        #[cfg(feature = "player")]
                        fvid_platform::prioritize_playback_thread();
                        let mut scratch = Vec::new();
                        let mut pred_scratch = Vec::new();
                        let mut transform_scratch = Vec::new();
                        let mut residual_scratch = Vec::new();
                        loop {
                            let (row, commands) = {
                                let rx = receiver.lock().unwrap_or_else(|e| e.into_inner());
                                match rx.recv() {
                                    Ok(data) => data,
                                    Err(_) => return Ok(()),
                                }
                            };
                            let (lock, cvar) = &*completed;
                            {
                                let mut done = lock.lock().unwrap_or_else(|e| e.into_inner());
                                while *done < row {
                                    done = cvar.wait(done).unwrap_or_else(|e| e.into_inner());
                                }
                            }
                            {
                                let mut planes = planes.lock().unwrap_or_else(|e| e.into_inner());
                                reconstruct_row(
                                    &mut planes,
                                    commands,
                                    sps,
                                    pps,
                                    slice,
                                    lists,
                                    &mut scratch,
                                    &mut pred_scratch,
                                    &mut transform_scratch,
                                    &mut residual_scratch,
                                )?;
                            }
                            {
                                let mut done = lock.lock().unwrap_or_else(|e| e.into_inner());
                                *done = row + 1;
                                cvar.notify_all();
                            }
                        }
                    })
                })
                .collect();
            Some((handles, planes, completed))
        } else {
            None
        };
        let parsed = (|| -> Result<()> {
            for row in 0..rows {
                if pps.entropy_sync {
                    if row > 0 {
                        decoder.palette = saved_palette
                            .clone()
                            .unwrap_or_else(|| initial_palette.clone());
                        bins = HevcCabac::from_contexts(
                            &slice.rbsp[slice.entropy_substreams[row as usize].clone()],
                            0,
                            saved_contexts.as_ref().unwrap_or(&initial_contexts),
                        )?;
                    }
                    decoder.qp = slice.qp;
                }
                for col in 0..columns {
                    let index = sao.len();
                    let parameters = hevc_sao::read_ctu_with_scale(
                        &mut bins,
                        slice.sao,
                        sps.depth,
                        pps.sao_offset_scale,
                        if col > 0 { sao.get(index - 1) } else { None },
                        if row > 0 {
                            sao.get(index - columns as usize)
                        } else {
                            None
                        },
                    )?;
                    sao.push(parameters);
                    hevc_tree::read_ctu(
                        &mut bins,
                        &mut decoder,
                        [w, h],
                        [col * side, row * side],
                        max_cb,
                        min_cb,
                    )?;
                    if pps.entropy_sync && col == 1 {
                        saved_contexts = Some(bins.contexts()?);
                        saved_palette = Some(decoder.palette.clone());
                    }
                    let end = bins.terminate()?;
                    let last = row == rows - 1 && col == columns - 1;
                    if end != last {
                        return Err(invalid(
                            "HEVC slice termination does not match picture extent",
                        ));
                    }
                    if pps.entropy_sync && col == columns - 1 && !last && !bins.terminate()? {
                        return Err(invalid("missing Hevc end-of-substream bit"));
                    }
                }
                if let Some(sender) = &decoder.jobs {
                    sender
                        .send((row as usize, std::mem::take(&mut decoder.reconstruction)))
                        .map_err(|_| invalid("HEVC reconstruction worker stopped"))?;
                }
            }
            Ok(())
        })();
        // Close the channel on every parse exit, including truncated bitstreams.
        decoder.jobs.take();
        if let Some((handles, planes, _)) = workers {
            for handle in handles {
                handle
                    .join()
                    .map_err(|_| invalid("HEVC reconstruction worker panicked"))??;
            }
            parsed?;
            decoder.planes = std::sync::Arc::try_unwrap(planes)
                .map_err(|_| invalid("HEVC reconstruction planes still shared"))?
                .into_inner()
                .unwrap_or_else(|e| e.into_inner());
        } else {
            parsed?;
        }
        Ok(())
    })?;
    if !decoder.planes.iter().all(Plane::complete) {
        return Err(invalid("incomplete HEVC picture"));
    }
    if !slice.deblocking.disabled {
        deblock(&mut decoder, slice)?;
    }
    if slice.sao != [false, false] {
        for (component, plane) in decoder
            .planes
            .iter_mut()
            .take(if sps.chroma_format == 0 { 1 } else { 3 })
            .enumerate()
        {
            let parameters: Vec<_> = sao.iter().map(|p| p[component]).collect();
            let [shift_x, shift_y] = component_shifts(component, sps.chroma_format);
            let cells = &decoder.cells;
            let stride = sps.dimensions[0] as usize / 4;
            plane.apply_sao_rectangular(
                [max_cb - shift_x as u8, max_cb - shift_y as u8],
                &parameters,
                |a, b| {
                    if let Some(layout) = &decoder.tile_layout
                        && !pps.tiles.as_ref().unwrap().loop_filter_across
                    {
                        let side = 1usize << max_cb;
                        let address = |p: [usize; 2]| {
                            ((p[1] << shift_y) / side) * columns as usize + (p[0] << shift_x) / side
                        };
                        return layout.tile_ids[address(a)] == layout.tile_ids[address(b)];
                    }
                    true
                },
                |p| {
                    let c = cells[((p[1] << shift_y) / 4) * stride + (p[0] << shift_x) / 4];
                    c.bypass || (c.pcm && sps.pcm.as_ref().is_some_and(|p| p.loop_filter_disabled))
                },
            )?;
        }
    }
    let motion = if slice.slice_type == SliceType::I {
        Vec::new()
    } else {
        let mut grid = Vec::with_capacity(w.div_ceil(16) as usize * h.div_ceil(16) as usize);
        for y in (0..h).step_by(16) {
            for x in (0..w).step_by(16) {
                grid.push(decoder.cells[y as usize / 4 * (w as usize / 4) + x as usize / 4].motion);
            }
        }
        grid
    };
    let coding=Some(decoder.coding_map()?);
    Ok(Picture {
        nal:slice.nal,
        coding,
        colour_planes: None,
        #[cfg(test)]
        queued_dim_blocks: decoder.queued_dim_blocks,
        #[cfg(test)]
        queued_inter_dc_blocks: decoder.queued_inter_dc_blocks,
        #[cfg(test)]
        queued_texture_blocks: decoder.queued_texture_blocks,
        #[cfg(test)]
        queued_inter_blocks: decoder.queued_inter_blocks,
        #[cfg(test)]
        pcm_luma_samples: decoder.cells.iter().filter(|c| c.pcm).count() * 16,
        #[cfg(test)]
        cross_component_blocks: decoder.cross_component_blocks,
        #[cfg(test)]
        act_blocks: decoder.act_blocks,
        #[cfg(test)]
        current_picture_blocks: decoder.current_picture_blocks,
        #[cfg(test)]
        completed_picture_blocks: decoder.completed_picture_blocks,
        #[cfg(test)]
        bipredicted_blocks: decoder.bipredicted_blocks,
        #[cfg(test)]
        fractional_current_chroma_blocks: decoder.fractional_current_chroma_blocks,
        motion,
        reference_pocs: std::array::from_fn(|l| lists[l].iter().map(|r| r.poc).collect()),
        reference_layers: std::array::from_fn(|l| lists[l].iter().map(|r| r.layer_id).collect()),
        reference_long_term: std::array::from_fn(|l| {
            lists[l].iter().map(|r| r.long_term).collect()
        }),
        dimensions: [w, h],
        crop: sps.crop,
        depth: sps.depth,
        planes: decoder.planes,
        sao,
    })
}

/// Reconstruct independent slice segments into one shared picture.
pub fn decode_slices(
    sps: &Sps,
    pps: &Pps,
    slices: &[SliceHeader],
    poc: i32,
    slice_lists: &[[Vec<Reference>; 2]],
    budget: usize,
) -> Result<Picture> {
    decode_slices_internal(sps, pps, slices, poc, slice_lists, budget, 0, None, None, None, None, None)
}

pub(crate) fn decode_slices_internal(
    sps: &Sps,
    pps: &Pps,
    slices: &[SliceHeader],
    poc: i32,
    slice_lists: &[[Vec<Reference>; 2]],
    budget: usize,
    layer_id: u8,
    texture: Option<&Picture>,
    context: Option<&super::hevc_slice::LayeredSliceContext<'_>>,
    view_components:Option<&ViewComponentPool>,
    refinement_components:Option<(&ViewComponentPool,super::hevc_inter_layer::PictureStatus)>,
    residual_sources:Option<&hevc_motion::inter_view_residual::BoundSources<'_>>,
) -> Result<Picture> {
    if sps.separate_colour_plane {
        return super::hevc_colour_planes::decode(sps, pps, slices, poc, slice_lists, budget, layer_id);
    }
    if slices.len() != slice_lists.len() {
        return Err(invalid("HEVC slice reference count mismatch"));
    }
    if sps.depth_layer && sps.three_d.as_ref().is_some_and(|e|e.intra_contour)
        && slices.iter().any(|h|h.layered.as_ref().is_some_and(|s|s.in_component_prediction==Some(true))) {
        validate_contour_texture(sps, texture)?;
    }
    if slices.iter().any(|h|cqt_enabled(sps,h)||texture_motion_enabled(sps,h)) {validate_coding_texture(sps,texture)?;}
    if slices.iter().any(|h|texture_motion_enabled(sps,h)) && context.is_none() {
        return Err(crate::unsupported("HEVC texture motion requires layered reconstruction binding"));
    }
    if slices.iter().any(|h|depth_iv_enabled(sps,h)) && context.is_none() {
        return Err(crate::unsupported("HEVC depth IV requires layered reconstruction binding"));
    }
    if pps.depth_lookup_active && (layer_id==0 || !sps.depth_layer || pps.depth_tables.is_none()) {
        return Err(crate::unsupported("HEVC depth lookup reconstruction requires a resolved depth layer"));
    }
    if slices.len() == 1 {
        return decode_internal(sps, pps, &slices[0], poc, &slice_lists[0], budget, layer_id, texture, context, view_components, refinement_components, residual_sources);
    }
    let slice = slices
        .first()
        .ok_or_else(|| invalid("empty HEVC slice set"))?;
    if slice_lists.len() != slices.len() {
        return Err(invalid("HEVC slice reference count mismatch"));
    }
    let first_lists = &slice_lists[0];
    let canonical = canonical_references(slice_lists)?;
    if sps.chroma_format > 3 || sps.separate_colour_plane {
        return Err(crate::unsupported(
            "unsupported HEVC multi-slice picture tools",
        ));
    }
    for (index, slice) in slices.iter().enumerate() {
        if slice.first != (index == 0)
            || (index == 0 && slice.address != 0)
            || slice.nal.layer_id != layer_id
        {
            return Err(invalid("invalid HEVC slice partition start"));
        }
        if slice
            .entropy_substreams
            .iter()
            .any(|r| r.start >= r.end || r.end > slice.rbsp.len())
        {
            return Err(invalid("invalid HEVC entropy substream bounds"));
        }
        let reference_lists = &slice_lists[index];
        if slice.slice_type != SliceType::I && !(1..=super::hevc_slice::merge_candidate_limit(sps,slice.layered.as_ref(),context)?).contains(&slice.max_merge_candidates) {
            return Err(invalid("invalid HEVC merge candidate count"));
        }
        for list in 0..2 {
            if reference_lists[list].len() != slice.references[list] as usize {
                return Err(invalid("HEVC reference-list length mismatch"));
            }
            for r in &reference_lists[list] {
                if r.picture
                    .as_ref()
                    .is_some_and(|p| p.dimensions != sps.dimensions || p.depth != sps.depth)
                {
                    return Err(invalid("HEVC reference geometry mismatch"));
                }
            }
        }
        let [min_cb, max_cb] = sps.coding_block_log2;
        let [min_tb, max_tb] = sps.transform_block_log2;
        if !(4..=6).contains(&max_cb)
            || !(3..=max_cb).contains(&min_cb)
            || !(2..=min_cb.min(5)).contains(&min_tb)
            || !(min_tb..=max_cb.min(5)).contains(&max_tb)
            || sps.transform_hierarchy_depth[1] > max_cb - min_tb
            || sps.depth.iter().any(|d| !(8..=16).contains(d))
            || sps.id != pps.sps_id
            || pps.id != slice.pps_id
        {
            return Err(invalid("invalid HEVC picture parameters"));
        }
        let [w, h] = sps.dimensions;
        if w == 0
            || h == 0
            || w % (1 << min_cb) != 0
            || h % (1 << min_cb) != 0
            || u64::from(sps.crop[0]) + u64::from(sps.crop[1]) >= u64::from(w)
            || u64::from(sps.crop[2]) + u64::from(sps.crop[3]) >= u64::from(h)
        {
            return Err(invalid("invalid HEVC picture dimensions or crop"));
        }
        let count = (w as usize)
            .checked_mul(h as usize)
            .ok_or_else(|| invalid("HEVC picture size overflow"))?;
        let required = iv_reconstruction_budget(count,slices.iter().any(|h|iv_enabled(sps,h,context)))
            .and_then(|n|n.checked_add(residual_reconstruction_budget(count,sps,context)?)?.checked_add(dbbp_reconstruction_budget(count,sps)?))
            .ok_or_else(|| invalid("HEVC picture budget overflow"))?;
        if required > budget {
            return Err(invalid("HEVC picture exceeds decode budget"));
        }
    }
    let [min_cb, max_cb] = sps.coding_block_log2;
    let [w, h] = sps.dimensions;
    let count = w as usize * h as usize;
    let [chroma_shift_x, chroma_shift_y] = component_shifts(1, sps.chroma_format);
    let tile_layout = pps
        .tiles
        .as_ref()
        .map(|tiles| {
            let side = 1u32 << max_cb;
            super::hevc_tiles::TileLayout::new(
                tiles,
                [w.div_ceil(side), h.div_ceil(side)],
                budget.checked_sub(reconstruction_budget(count)
                    .ok_or_else(||invalid("HEVC tile reconstruction budget overflow"))?.saturating_sub(count))
                    .ok_or_else(||invalid("HEVC tile metadata exceeds reconstruction budget"))?,
            )
        })
        .transpose()?;
    let lists = first_lists;
    let chroma_offsets = slice.chroma_qp_offsets.map(i32::from);
    hevc_qp::components_with_format(
        slice.qp,
        sps.depth,
        chroma_offsets,
        [0; 2],
        sps.chroma_format,
    )?;

    let mut decoder = Decoder {
        #[cfg(test)]
        queued_dim_blocks: 0,
        #[cfg(test)]
        queued_inter_dc_blocks: 0,
        #[cfg(test)]
        queued_texture_blocks: 0,
        #[cfg(test)]
        queued_inter_blocks: 0,
        texture,
        context,
        view_components,
        refinement_components,
        residual_sources,
        depth_iv: context.map(|binding|hevc_motion::depth_inter_view::prepare(binding,slice,poc,lists)).transpose()?.flatten(),
        texture_disparity:prepare_texture_disparity(context,sps,slice,poc,lists)?,
        iv_state:slices.iter().any(|h|iv_enabled(sps,h,context)).then(||vec![None;count/64]),
        palette: if sps.palette.is_some() {
            Some(super::hevc_palette::Predictor::from_parameters(sps, pps)?)
        } else {
            None
        },
        sps,
        pps,
        slice,
        poc,
        lists,
        slice_start: 0,
        tile_bounds: None,
        tile_layout,
        qp: slice.qp,
        chroma_offsets,
        cu_chroma_offsets: [0; 2],
        chroma_qp_coded: false,
        qp_coded: false,
        qp_prediction: slice.qp,
        qp_grid: vec![slice.qp; count / 64],
        edges: vec![[0; 2]; count / 16],
        cells: vec![Cell::default(); count / 16],
        jobs: None,
        reconstruction: Vec::new(),
        scratch: Vec::new(),
        pred_scratch: Vec::new(),
        transform_scratch: Vec::new(),
        residual_scratch: Vec::new(),
        #[cfg(test)]
        cross_component_blocks: 0,
        #[cfg(test)]
        act_blocks: 0,
        #[cfg(test)]
        current_picture_blocks: 0,
        #[cfg(test)]
        completed_picture_blocks: 0,
        #[cfg(test)]
        bipredicted_blocks: 0,
        #[cfg(test)]
        fractional_current_chroma_blocks: 0,
        planes: [
            Plane::new(w as usize, h as usize, sps.depth[0], count * 3)?,
            if sps.chroma_format == 0 {
                Plane::absent(sps.depth[1])
            } else {
                Plane::new(
                    w as usize >> chroma_shift_x,
                    h as usize >> chroma_shift_y,
                    sps.depth[1],
                    (count * 3) >> (chroma_shift_x + chroma_shift_y),
                )?
            },
            if sps.chroma_format == 0 {
                Plane::absent(sps.depth[1])
            } else {
                Plane::new(
                    w as usize >> chroma_shift_x,
                    h as usize >> chroma_shift_y,
                    sps.depth[1],
                    (count * 3) >> (chroma_shift_x + chroma_shift_y),
                )?
            },
        ],
    };
    let side = 1u32 << max_cb;
    let columns = w.div_ceil(side);
    let rows = h.div_ceil(side);
    let total = columns
        .checked_mul(rows)
        .ok_or_else(|| invalid("HEVC CTU grid overflow"))?;
    if slices.iter().any(|s| s.address >= total) {
        return Err(invalid("HEVC slice address is outside the picture"));
    }
    let mut sao = vec![[hevc_sao::Sao::Off; 3]; total as usize];
    let scan = |raster: u32| tile_scan(&decoder.tile_layout, raster);
    let starts: Vec<_> = slices.iter().map(|s| scan(s.address)).collect();
    let initial_palette = decoder.palette.clone();
    let mut saved_palette = None;
    let mut previous_contexts = None;
    let mut saved = None;
    let mut visited = 0;
    for (index, slice) in slices.iter().enumerate() {
        let begin = starts[index];
        let end = starts.get(index + 1).copied().unwrap_or(total);
        if begin != visited || begin >= end || end > total || slice.pps_id != pps.id {
            return Err(invalid("HEVC slices do not partition the picture"));
        }
        decoder.slice = slice;
        decoder.lists = &slice_lists[index];
        decoder.depth_iv=context.map(|binding|hevc_motion::depth_inter_view::prepare(binding,slice,poc,&slice_lists[index])).transpose()?.flatten();
        decoder.texture_disparity=prepare_texture_disparity(context,sps,slice,poc,&slice_lists[index])?;
        if !slice.dependent {
            decoder.palette = initial_palette.clone();
            saved_palette = None;
            decoder.slice_start = begin;
            decoder.qp = slice.qp;
            decoder.qp_prediction = slice.qp;
            decoder.qp_coded = false;
            saved = None;
        }
        decoder.cu_chroma_offsets = [0; 2];
        decoder.chroma_qp_coded = false;
        decoder.chroma_offsets = slice.chroma_qp_offsets.map(i32::from);
        let expected = 1
            + (begin + 1..end)
                .filter(|&a| {
                    let (_, _, _, _, tile_start, row_start) =
                        ctu_position(&decoder.tile_layout, a, columns, rows);
                    tile_start || (pps.entropy_sync && row_start)
                })
                .count();
        if slice.entropy_substreams.len() != expected
            || slice
                .entropy_substreams
                .iter()
                .any(|r| r.start >= r.end || r.end > slice.rbsp.len())
        {
            return Err(invalid(
                "HEVC segment entropy substreams do not match tile/row layout",
            ));
        }
        let mut bins = HevcCabac::new(
            &slice.rbsp[slice.entropy_substreams[0].clone()],
            0,
            slice.slice_type,
            slice.cabac_init,
            slice.qp,
        )?;
        let initial = bins.contexts()?;
        let (_, _, _, _, tile_start, row_start) =
            ctu_position(&decoder.tile_layout, begin, columns, rows);
        if slice.dependent && !tile_start {
            if pps.entropy_sync && row_start {
                decoder.palette = saved_palette
                    .clone()
                    .unwrap_or_else(|| initial_palette.clone());
            }
            let contexts = if pps.entropy_sync && row_start {
                saved.as_ref().unwrap_or(&initial)
            } else {
                previous_contexts
                    .as_ref()
                    .ok_or_else(|| invalid("HEVC dependent segment has no preceding CABAC state"))?
            };
            bins = HevcCabac::from_contexts(
                &slice.rbsp[slice.entropy_substreams[0].clone()],
                0,
                contexts,
            )?;
        }
        let mut stream = 0;
        for address in begin..end {
            let (raster, row, col, [tx, ty, tw, th], tile_start, row_start) =
                ctu_position(&decoder.tile_layout, address, columns, rows);
            decoder.tile_bounds = decoder
                .tile_layout
                .as_ref()
                .map(|_| [tx * side, ty * side, tw * side, th * side]);
            if tile_start {
                decoder.palette = initial_palette.clone();
                saved_palette = None;
                saved = None;
                decoder.qp = slice.qp;
                decoder.qp_prediction = slice.qp;
                decoder.qp_coded = false;
            }
            if address != begin && (tile_start || (pps.entropy_sync && row_start)) {
                stream += 1;
                let bytes = &slice.rbsp[slice.entropy_substreams[stream].clone()];
                bins = if tile_start {
                    HevcCabac::new(bytes, 0, slice.slice_type, slice.cabac_init, slice.qp)?
                } else {
                    decoder.palette = saved_palette
                        .clone()
                        .unwrap_or_else(|| initial_palette.clone());
                    HevcCabac::from_contexts(bytes, 0, saved.as_ref().unwrap_or(&initial))?
                };
                decoder.qp = slice.qp;
            }
            let available =
                |other: u32| tile_scan(&decoder.tile_layout, other) >= decoder.slice_start;
            sao[raster as usize] = hevc_sao::read_ctu_with_scale(
                &mut bins,
                slice.sao,
                sps.depth,
                pps.sao_offset_scale,
                if col > tx && available(raster - 1) {
                    Some(&sao[raster as usize - 1])
                } else {
                    None
                },
                if row > ty && available(raster - columns) {
                    Some(&sao[(raster - columns) as usize])
                } else {
                    None
                },
            )?;
            hevc_tree::read_ctu(
                &mut bins,
                &mut decoder,
                [w, h],
                [col * side, row * side],
                max_cb,
                min_cb,
            )?;
            if pps.entropy_sync && col == tx + 1 {
                saved = Some(bins.contexts()?);
                saved_palette = Some(decoder.palette.clone());
            }
            let last = address + 1 == end;
            if bins.terminate()? != last {
                return Err(invalid(
                    "HEVC slice termination does not match segment extent",
                ));
            }
            let tile_end = col == tx + tw - 1 && row == ty + th - 1;
            let row_end = pps.entropy_sync && col == tx + tw - 1;
            if !last && (tile_end || row_end) && !bins.terminate()? {
                return Err(invalid("missing HEVC end-of-substream bit"));
            }
            visited += 1;
        }
        previous_contexts = Some(bins.contexts()?);
    }
    decoder.tile_bounds = None;
    if !decoder.planes.iter().all(Plane::complete) {
        return Err(invalid("incomplete HEVC multi-slice picture"));
    }
    let canonical_ids: [Vec<super::hevc_reference_list::PictureIdentity>; 2] =
        std::array::from_fn(|l| canonical[l].iter().map(Reference::identity).collect());
    let slice_pocs: Vec<[Vec<super::hevc_reference_list::PictureIdentity>; 2]> = slice_lists
        .iter()
        .map(|lists| std::array::from_fn(|l| lists[l].iter().map(Reference::identity).collect()))
        .collect();
    for y in (0..h).step_by(4) {
        for x in (0..w).step_by(4) {
            let address = y / side * columns + x / side;
            let scan = tile_scan(&decoder.tile_layout, address);
            let owner =
                slices.partition_point(|s| tile_scan(&decoder.tile_layout, s.address) <= scan) - 1;
            let source_pocs = &slice_pocs[owner];
            let cell = &mut decoder.cells[y as usize / 4 * (w as usize / 4) + x as usize / 4];
            cell.motion =
                super::hevc_motion::remap_references(cell.motion, source_pocs, &canonical_ids)?;
        }
    }
    decoder.lists = &canonical;
    deblock_slices(&mut decoder, slices)?;
    let owners = independent_owners(slices);
    for (component, plane) in decoder
        .planes
        .iter_mut()
        .take(if sps.chroma_format == 0 { 1 } else { 3 })
        .enumerate()
    {
        let parameters: Vec<_> = sao.iter().map(|p| p[component]).collect();
        let [shift_x, shift_y] = component_shifts(component, sps.chroma_format);
        let owner = |p: [usize; 2]| {
            let address = ((p[1] << shift_y) / side as usize) * columns as usize
                + (p[0] << shift_x) / side as usize;
            let scan = tile_scan(&decoder.tile_layout, address as u32);
            slices.partition_point(|s| tile_scan(&decoder.tile_layout, s.address) <= scan) - 1
        };
        plane.apply_sao_rectangular(
            [max_cb - shift_x as u8, max_cb - shift_y as u8],
            &parameters,
            |a, b| {
                let ia = owner(a);
                let ib = owner(b);
                let slice_allowed =
                    owners[ia] == owners[ib] || slices[ia.max(ib)].loop_filter_across_slices;
                let tile_allowed = decoder.tile_layout.as_ref().is_none_or(|layout| {
                    if pps.tiles.as_ref().unwrap().loop_filter_across {
                        return true;
                    }
                    let raster = |p: [usize; 2]| {
                        (p[1] << shift_y) / side as usize * columns as usize
                            + (p[0] << shift_x) / side as usize
                    };
                    layout.tile_ids[raster(a)] == layout.tile_ids[raster(b)]
                });
                slice_allowed && tile_allowed
            },
            |p| {
                let c = decoder.cells[((p[1] << shift_y) / 4) * (sps.dimensions[0] as usize / 4)
                    + (p[0] << shift_x) / 4];
                c.bypass || (c.pcm && sps.pcm.as_ref().is_some_and(|p| p.loop_filter_disabled))
            },
        )?;
    }
    let mut motion = Vec::new();
    if slices.iter().any(|s| s.slice_type != SliceType::I) {
        for y in (0..h).step_by(16) {
            for x in (0..w).step_by(16) {
                motion
                    .push(decoder.cells[y as usize / 4 * (w as usize / 4) + x as usize / 4].motion);
            }
        }
    }
    let coding=Some(decoder.coding_map()?);
    Ok(Picture {
        nal:slice.nal,
        coding,
        colour_planes: None,
        #[cfg(test)]
        queued_dim_blocks: decoder.queued_dim_blocks,
        #[cfg(test)]
        queued_inter_dc_blocks: decoder.queued_inter_dc_blocks,
        #[cfg(test)]
        queued_texture_blocks: decoder.queued_texture_blocks,
        #[cfg(test)]
        queued_inter_blocks: decoder.queued_inter_blocks,
        #[cfg(test)]
        pcm_luma_samples: decoder.cells.iter().filter(|c| c.pcm).count() * 16,
        #[cfg(test)]
        cross_component_blocks: decoder.cross_component_blocks,
        #[cfg(test)]
        act_blocks: decoder.act_blocks,
        #[cfg(test)]
        current_picture_blocks: decoder.current_picture_blocks,
        #[cfg(test)]
        completed_picture_blocks: decoder.completed_picture_blocks,
        #[cfg(test)]
        bipredicted_blocks: decoder.bipredicted_blocks,
        #[cfg(test)]
        fractional_current_chroma_blocks: decoder.fractional_current_chroma_blocks,
        motion,
        reference_pocs: std::array::from_fn(|l| canonical_ids[l].iter().map(|id| id.poc).collect()),
        reference_layers: std::array::from_fn(|l| canonical_ids[l].iter().map(|id| id.layer_id).collect()),
        reference_long_term: std::array::from_fn(|l| {
            canonical[l].iter().map(|r| r.long_term).collect()
        }),
        dimensions: [w, h],
        crop: sps.crop,
        depth: sps.depth,
        planes: decoder.planes,
        sao,
    })
}
#[derive(Clone, Copy, Default)]
struct Cell {
    residual_weight:u8,
    vsp:bool,
    partition:u8,
    ready: bool,
    depth: u8,
    skip: bool,
    intra: bool,
    mode: u8,
    motion: Motion,
    cbf: bool,
    bypass: bool,
    pcm: bool,
    palette: bool,
}
struct Decoder<'a> {
    #[cfg(test)]
    queued_dim_blocks: usize,
    #[cfg(test)]
    queued_inter_dc_blocks: usize,
    #[cfg(test)]
    queued_texture_blocks: usize,
    #[cfg(test)]
    queued_inter_blocks: usize,
    texture: Option<&'a Picture>,
    context: Option<&'a super::hevc_slice::LayeredSliceContext<'a>>,
    view_components:Option<&'a ViewComponentPool>,
    refinement_components:Option<(&'a ViewComponentPool,super::hevc_inter_layer::PictureStatus)>,
    residual_sources:Option<&'a hevc_motion::inter_view_residual::BoundSources<'a>>,
    // Source completion and camera/list binding are checked once per slice.
    depth_iv: Option<hevc_motion::depth_inter_view::DepthInterView<'a>>,
    texture_disparity:Option<hevc_motion::disparity::derivation::PreparedTexture<'a>>,
    iv_state:Option<Vec<Option<hevc_motion::disparity::derivation::TextureDisparity>>>,
    sps: &'a Sps,
    pps: &'a Pps,
    slice: &'a SliceHeader,
    poc: i32,
    lists: &'a [Vec<Reference>; 2],
    slice_start: u32,
    tile_bounds: Option<[u32; 4]>,
    tile_layout: Option<super::hevc_tiles::TileLayout>,
    palette: Option<super::hevc_palette::Predictor>,
    qp: i32,
    chroma_offsets: [i32; 2],
    cu_chroma_offsets: [i32; 2],
    chroma_qp_coded: bool,
    qp_coded: bool,
    qp_prediction: i32,
    qp_grid: Vec<i32>,
    edges: Vec<[u8; 2]>,
    cells: Vec<Cell>,
    planes: [Plane; 3],
    jobs: Option<std::sync::mpsc::SyncSender<(usize, Vec<Reconstruction>)>>,
    reconstruction: Vec<Reconstruction>,
    scratch: Vec<i32>,
    pred_scratch: Vec<u16>,
    transform_scratch: Vec<i32>,
    residual_scratch: Vec<i32>,
    #[cfg(test)]
    cross_component_blocks: usize,
    #[cfg(test)]
    act_blocks: usize,
    #[cfg(test)]
    current_picture_blocks: usize,
    #[cfg(test)]
    completed_picture_blocks: usize,
    #[cfg(test)]
    bipredicted_blocks: usize,
    #[cfg(test)]
    fractional_current_chroma_blocks: usize,
}
impl Decoder<'_> {
    fn coding_map(&mut self)->Result<coding::Map> {
        let [w,h]=self.sps.dimensions;let stride=w as usize/4;
        let mut entries=Vec::with_capacity(w as usize/8*(h as usize/8));
        let mut motion=Vec::with_capacity(w as usize/8*(h as usize/8));
        for y in (0..h as usize).step_by(8) {for x in (0..w as usize).step_by(8) {
            let cell=self.cells[y/4*stride+x/4];
            let log=self.sps.coding_block_log2[1].checked_sub(cell.depth)
                .filter(|&v|(3..=6).contains(&v)).ok_or_else(||invalid("invalid HEVC saved CU size"))?;
            if !cell.ready || cell.partition>7 {return Err(invalid("incomplete HEVC saved CU metadata"));}
            entries.push(((log-3)<<3)|cell.partition|(u8::from(cell.skip)<<5));
            motion.push(cell.motion);
        }}
        coding::Map::new([w,h],entries)?.with_motion(motion)?.with_iv(self.iv_state.take())
    }
    fn cqt_partition(&self,n:Node)->Result<(bool,u8)> {
        if !cqt_enabled(self.sps,self.slice) {return Ok((false,0));}
        let texture=self.texture.ok_or_else(||invalid("missing HEVC coding-tree TexturePic"))?;
        let (log,part)=texture.coding_unit_at([n.x,n.y])?;
        if log!=n.log2_size {return Ok((false,0));}
        Ok(match part {
            Partition::Full=>(true,0),Partition::Quarters=>(false,0),
            Partition::Horizontal|Partition::Top|Partition::Bottom=>(false,1),
            Partition::Vertical|Partition::Left|Partition::Right=>(false,2),
        })
    }
    fn read_dim_residual(&mut self,b:&mut HevcCabac<'_>,n:Node,bypass:bool)->Result<Option<hevc_block::Coefficients>> {
        if n.log2_size>self.sps.transform_block_log2[1] {
            return Err(invalid("HEVC residual DIM transform exceeds maximum size"));
        }
        let config=hevc_transform_tree::Config {
            log2_cu:n.log2_size,log2_min_transform:self.sps.transform_block_log2[0],
            log2_max_transform:self.sps.transform_block_log2[1],
            max_depth:self.sps.transform_hierarchy_depth[1],intra_split:false,
        };
        let mut residual=None;
        hevc_transform_tree::read_intra_with_chroma(b,[n.x,n.y],config,0,|b,u| {
            // I.7.4.9.8 still signals the split flag, but forbids splitting a DIM PB.
            if u.log2_size!=n.log2_size || u.depth!=0 || u.origin!=[n.x,n.y] {
                return Err(invalid("HEVC residual DIM prediction block cannot split its transform"));
            }
            if !u.coded[0] {return Ok(());}
            if self.pps.cu_qp_delta_depth.is_some() && !self.qp_coded {
                let delta=hevc_qp::read_delta(b,self.sps.depth[0])?;
                self.qp=hevc_qp::luma(self.qp_prediction,[None;2],delta,self.sps.depth[0])?;
                self.qp_coded=true;
            }
            let qp=self.component_qps(false)?[0];
            let config=hevc_block::Config {
                log2_size:u.log2_size,component:0,bit_depth:self.sps.depth[0],qp,
                // DIM uses intra scaling/4x4 DST and diagonal coefficient scan;
                // DC supplies these properties without enabling angular RDPCM.
                intra_mode:Some(1),transform_skip_enabled:self.pps.transform_skip,
                transquant_bypass:bypass,sign_hiding:self.pps.sign_data_hiding,
            };
            residual=Some(hevc_block::read_with_precision(b,config,self.sps.transform_skip_rotation,
                self.sps.transform_skip_context,self.sps.implicit_rdpcm,self.sps.explicit_rdpcm,
                self.pps.transform_skip_max_log2,self.sps.persistent_rice,self.sps.extended_precision,
                self.sps.cabac_bypass_alignment,false)?);
            Ok(())
        })?;
        Ok(residual)
    }
}
impl<'a> Visitor<HevcCabac<'a>> for Decoder<'_> {
    fn predicted_unsplit(&self,n:Node)->Result<bool> {
        if !cqt_enabled(self.sps,self.slice) {return Ok(false);}
        let texture=self.texture.ok_or_else(||invalid("missing HEVC coding-tree TexturePic"))?;
        Ok(n.log2_size<=texture.coding_unit_at([n.x,n.y])?.0)
    }
    fn neighbouring_depths(&self, n: Node) -> [Option<u8>; 2] {
        let at = |x: u32, y: u32| {
            self.cell(x as i32, y as i32)
                .filter(|v| v.ready)
                .map(|v| v.depth)
        };
        [
            n.x.checked_sub(1).and_then(|x| at(x, n.y)),
            n.y.checked_sub(1).and_then(|y| at(n.x, y)),
        ]
    }
    fn enter(&mut self, n: Node, split: bool) -> Result<()> {
        if self.slice.cu_chroma_qp_offset_enabled {
            let table = self
                .pps
                .chroma_qp_offset_list
                .as_ref()
                .ok_or_else(|| invalid("HEVC active chroma QP selection has no list"))?;
            if n.log2_size >= self.sps.coding_block_log2[1] - table.depth {
                self.chroma_qp_coded = false;
            }
        }
        if let Some(depth) = self.pps.cu_qp_delta_depth {
            let log = self.sps.coding_block_log2[1] - depth;
            if n.log2_size >= log {
                self.qp_coded = false;
                if !split || n.log2_size == log {
                    let width = self.sps.dimensions[0] as usize / 8;
                    let ctu = 1 << self.sps.coding_block_log2[1];
                    let a = (n.x % ctu != 0)
                        .then(|| self.qp_grid[n.y as usize / 8 * width + (n.x as usize - 1) / 8]);
                    let b = (n.y % ctu != 0)
                        .then(|| self.qp_grid[(n.y as usize - 1) / 8 * width + n.x as usize / 8]);
                    self.qp_prediction = hevc_qp::luma(self.qp, [a, b], 0, self.sps.depth[0])?;
                    self.qp = self.qp_prediction;
                }
            }
        }
        Ok(())
    }
    fn leaf(&mut self, b: &mut HevcCabac<'a>, n: Node) -> Result<()> {
        let bypass = self.pps.transquant_bypass && b.decision(Syntax::TransquantBypass, 0)?;
        let skip = if self.slice.slice_type != SliceType::I {
            let context = [
                self.cell(n.x as i32 - 1, n.y as i32),
                self.cell(n.x as i32, n.y as i32 - 1),
            ]
            .into_iter()
            .flatten()
            .filter(|c| c.ready && c.skip)
            .count();
            b.decision(Syntax::Skip, context)?
        } else {
            false
        };
        let depth_skip_enabled=self.sps.depth_layer && self.sps.three_d.as_ref().is_some_and(|s|s.skip_intra);
        if super::hevc_depth_dc::read_skip_intra_flag(b,skip,depth_skip_enabled)? {
            let mode=super::hevc_depth_dc::read_skip_intra_mode(b)?;
            if self.sps.chroma_format!=0 {
                return Err(crate::unsupported("HEVC chroma skip-intra reconstruction is not implemented"));
            }
            let size=1usize<<n.log2_size;
            let [x,y]=[n.x as usize,n.y as usize];
            let eligible=|(xx,yy):(usize,usize)| {
                if xx>=self.sps.dimensions[0] as usize || yy>=self.sps.dimensions[1] as usize {return false;}
                if self.tile_bounds.is_some_and(|[tx,ty,w,h]|xx<tx as usize || yy<ty as usize || xx>=(tx+w) as usize || yy>=(ty+h) as usize) {return false;}
                let side=1usize<<self.sps.coding_block_log2[1];
                let address=yy/side*(self.sps.dimensions[0] as usize).div_ceil(side)+xx/side;
                tile_scan(&self.tile_layout,address as u32)>=self.slice_start && (!self.pps.constrained_intra || self.cell(xx as i32,yy as i32).is_some_and(|c|c.intra))
            };
            if mode>=2 {
                let selected=if mode==2 {x.checked_sub(1).map(|xx|(xx,y+size/2))}
                    else {y.checked_sub(1).map(|yy|(x+size/2,yy))};
                let allowed=selected.is_some_and(eligible);
                if self.jobs.is_some() {self.reconstruction.push(Reconstruction::Single {origin:[x,y],log:n.log2_size,mode,allowed});}
                else {self.planes[0].reconstruct_single([x,y],n.log2_size,mode,|_,_|allowed)?;}
            } else {
                let mut allowed=[0u64;5];
                for index in 0..=4*size {
                    let point=if index<2*size {y.checked_sub(1).map(|yy|(x+index,yy))}
                        else if index<4*size {x.checked_sub(1).map(|xx|(xx,y+index-2*size))}
                        else {x.checked_sub(1).zip(y.checked_sub(1))};
                    if point.is_some_and(&eligible) {allowed[index/64]|=1<<(index%64);}
                }
                let filter_boundary=self.sps.filter_intra_boundary(if mode==0 {26}else{10},bypass);
                if self.jobs.is_some() {self.reconstruction.push(Reconstruction::SkipAngular {origin:[x,y],log:n.log2_size,mode,filter_boundary,allowed});}
                else {self.planes[0].reconstruct_skip_angular([x,y],n.log2_size,mode,filter_boundary,&mut self.pred_scratch,
                    |xx,yy|skip_angular_allowed(allowed,[x,y],size,xx,yy))?;}
            }
            let stride=self.sps.dimensions[0] as usize/4;
            for yy in y/4..(y+size)/4 {for xx in x/4..(x+size)/4 {
                self.cells[yy*stride+xx]=Cell {bypass,mode:if mode==0 {26}else if mode==1 {10}else{1},..Cell::default()};
            }}
            self.finish_cu(n,true,false);
            return Ok(());
        }
        let intra =
            !skip && (self.slice.slice_type == SliceType::I || b.decision(Syntax::PredMode, 0)?);
        if !intra {
            self.inter(b, n, skip, bypass)?;
            self.finish_cu(n, false, skip);
            return Ok(());
        }
        if self.palette.is_some()
            && n.log2_size <= self.sps.transform_block_log2[1]
            && b.decision(Syntax::PaletteMode, 0)?
        {
            let header = super::hevc_palette::Header::read(
                self.palette.as_ref().unwrap(),
                1usize << n.log2_size,
                b,
            )?;
            if header.escapes {
                if self.pps.cu_qp_delta_depth.is_some() && !self.qp_coded {
                    let delta = hevc_qp::read_delta(b, self.sps.depth[0])?;
                    self.qp =
                        hevc_qp::luma(self.qp_prediction, [None; 2], delta, self.sps.depth[0])?;
                    self.qp_coded = true;
                }
                self.read_chroma_qp(b, [true; 3], bypass)?;
            }
            let samples = header.samples(
                1usize << n.log2_size,
                self.sps.chroma_format,
                self.sps.depth,
                bypass,
                self.component_qps(false)?.map(i32::from),
                b,
            )?;
            self.palette
                .as_mut()
                .unwrap()
                .update(&header.entries, &header.reused)?;
            let stride = self.sps.dimensions[0] as usize / 4;
            let side = 1usize << n.log2_size;
            for y in n.y as usize / 4..(n.y as usize + side) / 4 {
                for x in n.x as usize / 4..(n.x as usize + side) / 4 {
                    self.cells[y * stride + x] = Cell {
                        bypass,
                        palette: true,
                        mode: 1,
                        ..Cell::default()
                    };
                }
            }
            for (component, samples) in samples
                .into_iter()
                .take(if self.sps.chroma_format == 0 { 1 } else { 3 })
                .enumerate()
            {
                let [sx, sy] = component_shifts(component, self.sps.chroma_format);
                let rect = [
                    n.x as usize >> sx,
                    n.y as usize >> sy,
                    side >> sx,
                    side >> sy,
                ];
                if self.jobs.is_some() {
                    self.reconstruction.push(Reconstruction::Pcm {
                        component,
                        rect,
                        samples,
                    });
                } else {
                    self.planes[component].reconstruct_inter(rect, &samples)?;
                }
            }
            self.finish_cu(n, true, false);
            return Ok(());
        }
        let (inferred_part,_) = self.cqt_partition(n)?;
        let nxn = !inferred_part && n.log2_size == self.sps.coding_block_log2[0]
            && !b.decision(Syntax::PartMode, 0)?;
        if !nxn
            && let Some(pcm) = &self.sps.pcm
            && (pcm.block_log2[0]..=pcm.block_log2[1]).contains(&n.log2_size)
            && b.terminate()?
        {
            let samples = b.read_pcm_with_chroma(
                n.log2_size,
                pcm.depth,
                self.sps.depth,
                self.sps.chroma_format,
            )?;
            let stride = self.sps.dimensions[0] as usize / 4;
            let side = 1usize << n.log2_size;
            for y in n.y as usize / 4..(n.y as usize + side) / 4 {
                for x in n.x as usize / 4..(n.x as usize + side) / 4 {
                    self.cells[y * stride + x] = Cell {
                        pcm: true,
                        bypass,
                        mode: 1,
                        ..Cell::default()
                    };
                }
            }
            for (component, samples) in samples
                .into_iter()
                .take(if self.sps.chroma_format == 0 { 1 } else { 3 })
                .enumerate()
            {
                let [shift_x, shift_y] = component_shifts(component, self.sps.chroma_format);
                let rect = [
                    n.x as usize >> shift_x,
                    n.y as usize >> shift_y,
                    side >> shift_x,
                    side >> shift_y,
                ];
                if self.jobs.is_some() {
                    self.reconstruction.push(Reconstruction::Pcm {
                        component,
                        rect,
                        samples,
                    });
                } else {
                    self.planes[component].reconstruct_inter(rect, &samples)?;
                }
            }
            self.finish_cu(n, true, false);
            return Ok(());
        }
        let depth_modes = self.sps.three_d.as_ref().map_or([false;2], |e| [
            e.intra_dc_wedge,
            e.intra_contour && self.slice.layered.as_ref().is_some_and(|s|s.in_component_prediction==Some(true)),
        ]);
        let mut nxn_depth = None;
        let codes = if self.sps.depth_layer && depth_modes.iter().any(|&v|v) {
            use super::hevc_intra_syntax::DepthLumaCode;
            if self.sps.chroma_format!=0 {return Err(crate::unsupported("HEVC chroma DIM reconstruction is not implemented"));}
            let log=n.log2_size-u8::from(nxn);
            let table=super::hevc_wedge::Table::new((1usize<<log).min(32),32768)?;
            let codes=hevc_intra_syntax::read_depth_luma(b,nxn,log,depth_modes,self.slice.nal.unit_type,Some(&table))?;
            let dc_only=super::hevc_depth_dc::read_dc_only(b,super::hevc_depth_dc::CodingUnit {
                intra:true,full_partition:!nxn,skipped:false,skip_intra:false,enabled:[depth_modes[0],false],
            })?;
            let dim=std::array::from_fn(|i|codes.get(i).is_some_and(|c|matches!(c,DepthLumaCode::Wedge(_)|DepthLumaCode::Contour)));
            let offsets=super::hevc_depth_dc::read_cu_offsets(b,true,nxn,dc_only,false,dim)?;
            if !nxn && (codes.iter().any(|c|matches!(c,DepthLumaCode::Wedge(_)|DepthLumaCode::Contour)) || dc_only) {
                if let [DepthLumaCode::Ordinary(code)]=codes.as_slice() {
                    let neighbour=|x:i32,y:i32|self.cell(x,y).filter(|c|c.ready).map(|c|if c.intra {c.mode}else{1});
                    let left=neighbour(n.x as i32-1,n.y as i32);
                    let top=if n.y%(1<<self.sps.coding_block_log2[1])==0 {None}else{neighbour(n.x as i32,n.y as i32-1)};
                    let mode=code.resolve(left,top)?;
                    let size=1usize<<log;let origin=[n.x as usize,n.y as usize];
                    let allowed=self.depth_neighbour_mask(origin,size);
                    let offset=i64::from(offsets[0][0]);let layer=self.slice.nal.layer_id;
                    let filters=[self.sps.strong_intra_smoothing,!self.sps.intra_smoothing_disabled,self.sps.filter_intra_boundary(mode,bypass)];
                    if self.jobs.is_some() {self.reconstruction.push(Reconstruction::OrdinaryDc {origin,log,mode,offset,layer,allowed,filters});}
                    else {reconstruct_ordinary_dc(&mut self.planes[0],origin,log,self.sps.transform_block_log2[1],mode,offset,layer,allowed,filters,self.sps.depth[0],self.pps.depth_tables.as_deref(),&mut self.pred_scratch)?;}
                    let stride=self.sps.dimensions[0] as usize/4;
                    for yy in origin[1]/4..(origin[1]+size)/4 {for xx in origin[0]/4..(origin[0]+size)/4 {
                        self.cells[yy*stride+xx]=Cell {bypass,mode,..Cell::default()};
                    }}
                    self.finish_cu(n,true,false);return Ok(());
                }
                let [mode]=codes.as_slice() else {
                    return Err(invalid("invalid HEVC full DIM prediction block count"));
                };
                let size=1usize<<log;let origin=[n.x as usize,n.y as usize];
                let mut pattern=vec![false;size*size];
                match mode {
                    DepthLumaCode::Wedge(index)=>table.get(*index,&mut pattern)?,
                    DepthLumaCode::Contour=>{
                        let texture=self.texture.ok_or_else(||invalid("missing HEVC contour TexturePic"))?;
                        super::hevc_depth_prediction::contour_pattern(texture.planes[0].samples(),
                            texture.planes[0].dimensions()[0],origin,size,texture.depth[0],&mut pattern)?;
                    }
                    _=>return Err(invalid("unexpected HEVC DIM mode")),
                }
                let allowed=self.depth_neighbour_mask(origin,size);
                let offsets=offsets[0].map(i64::from);
                let layer=self.slice.nal.layer_id;
                let residual=if dc_only {None}else{self.read_dim_residual(b,n,bypass)?};
                let coded=residual.is_some();
                if self.jobs.is_some() {
                    #[cfg(test)] {self.queued_dim_blocks+=1;}
                    self.reconstruction.push(Reconstruction::Wedge {origin,log,pattern,offsets,allowed,layer,residual});}
                else {
                    if let Some(residual)=residual {
                        residual.reconstruct(self.pps.scaling_lists.as_ref().unwrap_or(&self.sps.scaling_lists),
                            &mut self.transform_scratch,&mut self.residual_scratch)?;
                    } else {self.residual_scratch.clear();}
                    reconstruct_wedge(&mut self.planes[0],origin,log,&pattern,offsets,allowed,layer,
                        self.sps.depth[0],self.pps.depth_tables.as_deref(),&self.residual_scratch)?;
                }
                let stride=self.sps.dimensions[0] as usize/4;
                for yy in origin[1]/4..(origin[1]+size)/4 {for xx in origin[0]/4..(origin[0]+size)/4 {
                    self.cells[yy*stride+xx]=Cell {bypass,mode:1,cbf:coded,..Cell::default()};
                }}
                self.finish_cu(n,true,false);return Ok(());
            }
            let ordinary=codes.iter().map(|c|match c {
                DepthLumaCode::Ordinary(c)=>*c,
                // DIM neighbour modes normalize to DC for ordinary MPM derivation.
                _=>hevc_intra_syntax::LumaCode {mpm:true,value:1},
            }).collect::<Vec<_>>();
            if nxn && dim.iter().any(|&v|v) {nxn_depth=Some((codes,offsets,table));}
            ordinary
        } else {hevc_intra_syntax::read_luma(b, nxn)?};
        let mut luma_modes = Vec::with_capacity(codes.len());
        for (i, code) in codes.into_iter().enumerate() {
            let log = n.log2_size - u8::from(nxn);
            let p = Node {
                x: n.x + (i as u32 % 2) * (1 << log),
                y: n.y + (i as u32 / 2) * (1 << log),
                log2_size: log,
                depth: n.depth,
            };
            let neighbour = |x: u32, y: u32| {
                self.cell(x as i32, y as i32)
                    .filter(|v| v.ready)
                    .map(|v| if v.intra { v.mode } else { 1 })
            };
            let left = p.x.checked_sub(1).and_then(|x| neighbour(x, p.y));
            let top = if p.y % (1 << self.sps.coding_block_log2[1]) == 0 {
                None
            } else {
                p.y.checked_sub(1).and_then(|y| neighbour(p.x, y))
            };
            let is_dim=nxn_depth.as_ref().is_some_and(|(codes,_,_)|
                !matches!(codes[i],hevc_intra_syntax::DepthLumaCode::Ordinary(_)));
            let derived = if is_dim {1}else{code.resolve(left, top)?};
            luma_modes.push(derived);
            let stride = self.sps.dimensions[0] as usize / 4;
            for y in p.y as usize / 4..(p.y as usize + (1 << p.log2_size)) / 4 {
                for x in p.x as usize / 4..(p.x as usize + (1 << p.log2_size)) / 4 {
                    self.cells[y * stride + x] = Cell {
                        ready: true,
                        bypass,
                        depth: n.depth,
                        intra: true,
                        mode: derived,
                        partition:if nxn {3}else{0},
                        ..Cell::default()
                    };
                }
            }
        }
        let chroma_modes =
            hevc_intra_syntax::read_chroma_modes_and_codes(b, &luma_modes, self.sps.chroma_format)?;
        let c = hevc_transform_tree::Config {
            log2_cu: n.log2_size,
            log2_min_transform: self.sps.transform_block_log2[0],
            log2_max_transform: self.sps.transform_block_log2[1],
            max_depth: self.sps.transform_hierarchy_depth[1] + u8::from(nxn),
            intra_split: nxn,
        };
        hevc_transform_tree::read_intra_with_chroma(
            b,
            [n.x, n.y],
            c,
            self.sps.chroma_format,
            |b, u| {
                let index = if self.sps.chroma_format == 3 && nxn {
                    let half = 1u32 << (n.log2_size - 1);
                    ((u.origin[1] - n.y) / half * 2 + (u.origin[0] - n.x) / half) as usize
                } else {
                    0
                };
                let half=1u32<<(n.log2_size-1);
                let pb_index=if nxn {((u.origin[1]-n.y)/half*2+(u.origin[0]-n.x)/half)as usize}else{0};
                let dim_partition=nxn_depth.as_ref().and_then(|(codes,offsets,table)| {
                    match codes.get(pb_index)? {
                        hevc_intra_syntax::DepthLumaCode::Ordinary(_)=>None,
                        mode=>Some((*mode,offsets[pb_index],table)),
                    }
                });
                if dim_partition.is_some() && (u.log2_size!=n.log2_size-1 || u.depth!=1) {
                    return Err(invalid("HEVC NxN DIM prediction block cannot split its transform"));
                }
                let (chroma, chroma_code) = chroma_modes[index];
                let mode = self
                    .cell(u.origin[0] as i32, u.origin[1] as i32)
                    .ok_or_else(|| invalid("HEVC transform has no prediction block"))?
                    .mode;
                let act = self.pps.adaptive_colour_transform
                    && u.coded.iter().any(|&v| v)
                    && chroma_modes.iter().all(|&(_, code)| code == 4)
                    && b.decision(Syntax::ResidualAct, 0)?;
                if act && bypass && self.sps.depth[0] != self.sps.depth[1] {
                    return Err(invalid("HEVC bypass ACT requires equal component depths"));
                }
                #[cfg(test)]
                {
                    self.act_blocks += usize::from(act);
                }
                let mut act_blocks = std::array::from_fn(|_| None);
                let mut act_alphas = [0; 3];
                if self.pps.cu_qp_delta_depth.is_some()
                    && !self.qp_coded
                    && u.coded.iter().any(|&v| v)
                {
                    let delta = hevc_qp::read_delta(b, self.sps.depth[0])?;
                    self.qp =
                        hevc_qp::luma(self.qp_prediction, [None; 2], delta, self.sps.depth[0])?;
                    self.qp_coded = true;
                }
                let stride = self.sps.dimensions[0] as usize / 4;
                for k in 0..1usize << (u.log2_size - 2) {
                    let x = u.origin[0] as usize / 4;
                    let y = u.origin[1] as usize / 4;
                    self.edges[(y + k) * stride + x][0] |= 2;
                    self.edges[y * stride + x + k][1] |= 2;
                }
                self.read_chroma_qp(b, u.coded_components(), bypass)?;
                let qps = self.component_qps(act)?;
                let mut luma_residual = Vec::new();
                for block in u.component_blocks(self.sps.chroma_format) {
                    let component = block.component;
                    let alpha = if component != 0
                        && self.pps.cross_component_prediction
                        && u.coded[0]
                        && chroma_code == 4
                    {
                        super::hevc_cross_component::read_alpha(b, component)?
                    } else {
                        0
                    };
                    #[cfg(test)]
                    {
                        self.cross_component_blocks += usize::from(alpha != 0);
                    }
                    let c = hevc_block::Config {
                        log2_size: block.log2_size,
                        component: component as u8,
                        bit_depth: self.sps.depth[usize::from(component != 0)],
                        qp: qps[component],
                        intra_mode: Some(if component == 0 { mode } else { chroma }),
                        transform_skip_enabled: self.pps.transform_skip,
                        transquant_bypass: bypass,
                        sign_hiding: self.pps.sign_data_hiding,
                    };
                    let residual = if block.coded {
                        Some(hevc_block::read_with_precision(
                            b,
                            c,
                            self.sps.transform_skip_rotation,
                            self.sps.transform_skip_context,
                            self.sps.implicit_rdpcm,
                            self.sps.explicit_rdpcm,
                            self.pps.transform_skip_max_log2,
                            self.sps.persistent_rice,
                            self.sps.extended_precision,
                            self.sps.cabac_bypass_alignment,
                            self.sps.chroma_format == 3,
                        )?)
                    } else {
                        None
                    };
                    let residual = if act {
                        act_blocks[component] = residual;
                        act_alphas[component] = alpha;
                        None
                    } else {
                        residual
                    };
                    if component==0 && let Some((mode,offsets,table))=dim_partition {
                        let origin=block.origin.map(|v|v as usize);
                        let log=block.log2_size;let size=1usize<<log;
                        let mut pattern=vec![false;size*size];
                        match mode {
                            hevc_intra_syntax::DepthLumaCode::Wedge(index)=>table.get(index,&mut pattern)?,
                            hevc_intra_syntax::DepthLumaCode::Contour=>{
                                let texture=self.texture.ok_or_else(||invalid("missing HEVC contour TexturePic"))?;
                                super::hevc_depth_prediction::contour_pattern(texture.planes[0].samples(),
                                    texture.planes[0].dimensions()[0],origin,size,texture.depth[0],&mut pattern)?;
                            }
                            _=>return Err(invalid("unexpected HEVC NxN DIM mode")),
                        }
                        let offsets=offsets.map(i64::from);let layer=self.slice.nal.layer_id;
                        let allowed=self.depth_neighbour_mask(origin,size);
                        if self.jobs.is_some() {
                            #[cfg(test)] {self.queued_dim_blocks+=1;}
                            self.reconstruction.push(Reconstruction::Wedge {origin,log,pattern,offsets,allowed,layer,residual});
                        } else {
                            if let Some(residual)=residual {
                                residual.reconstruct(self.pps.scaling_lists.as_ref().unwrap_or(&self.sps.scaling_lists),
                                    &mut self.transform_scratch,&mut self.residual_scratch)?;
                            } else {self.residual_scratch.clear();}
                            reconstruct_wedge(&mut self.planes[0],origin,log,&pattern,offsets,allowed,layer,
                                self.sps.depth[0],self.pps.depth_tables.as_deref(),&self.residual_scratch)?;
                        }
                        let stride=self.sps.dimensions[0] as usize/4;
                        for yy in origin[1]/4..(origin[1]+size)/4 {for xx in origin[0]/4..(origin[0]+size)/4 {
                            self.cells[yy*stride+xx].cbf=block.coded;
                        }}
                        continue;
                    }
                    let origin = block.origin;
                    if self.jobs.is_some() {
                        self.reconstruction.push(Reconstruction::Intra {
                            component,
                            origin: origin.map(|v| v as usize),
                            log: c.log2_size,
                            mode: c.intra_mode.unwrap(),
                            filter_boundary: self
                                .sps
                                .filter_intra_boundary(c.intra_mode.unwrap(), bypass),
                            alpha: if act { 0 } else { alpha },
                            residual,
                        });
                    } else {
                        if let Some(block) = residual {
                            block.reconstruct(
                                self.pps
                                    .scaling_lists
                                    .as_ref()
                                    .unwrap_or(&self.sps.scaling_lists),
                                &mut self.transform_scratch,
                                &mut self.residual_scratch,
                            )?;
                        } else {
                            self.residual_scratch.clear();
                        }
                        if !act {
                            cross_residual(
                                component,
                                alpha,
                                c.log2_size,
                                self.sps.depth,
                                self.pps.cross_component_prediction,
                                &mut luma_residual,
                                &mut self.residual_scratch,
                            )?;
                        }
                        self.planes[component].reconstruct_intra_with_full_chroma_filters(
                            origin.map(|v| v as usize),
                            c.log2_size,
                            c.intra_mode.unwrap(),
                            component != 0,
                            self.sps.chroma_format == 3,
                            self.sps.strong_intra_smoothing,
                            !self.sps.intra_smoothing_disabled,
                            self.sps
                                .filter_intra_boundary(c.intra_mode.unwrap(), bypass),
                            &self.residual_scratch,
                            &mut self.pred_scratch,
                            |x, y| {
                                let [shift_x, shift_y] =
                                    component_shifts(component, self.sps.chroma_format);
                                let side = 1usize << self.sps.coding_block_log2[1];
                                let address = (y << shift_y) / side
                                    * (self.sps.dimensions[0] as usize).div_ceil(side)
                                    + (x << shift_x) / side;
                                if let Some([tx, ty, width, height]) = self.tile_bounds {
                                    let x = (x << shift_x) as u32;
                                    let y = (y << shift_y) as u32;
                                    if x < tx || y < ty || x >= tx + width || y >= ty + height {
                                        return false;
                                    }
                                }
                                if tile_scan(&self.tile_layout, address as u32) < self.slice_start {
                                    return false;
                                }
                                if !self.pps.constrained_intra {
                                    return true;
                                }
                                self.cells[((y << shift_y) / 4)
                                    * (self.sps.dimensions[0] as usize / 4)
                                    + (x << shift_x) / 4]
                                    .intra
                            },
                        )?;
                    }
                }
                if act {
                    self.finish_act(u.origin, u.log2_size, bypass, act_blocks, act_alphas)?;
                }
                Ok(())
            },
        )?;
        self.finish_cu(n, true, false);
        Ok(())
    }
}

/// Parse CTUs in tile-scan order; each tile has independent entropy contexts.
fn decode_tile_ctus<'a>(
    decoder: &mut Decoder<'a>,
    slice: &'a SliceHeader,
    layout: &super::hevc_tiles::TileLayout,
) -> Result<Vec<hevc_sao::CtuSao>> {
    let sps = decoder.sps;
    let pps = decoder.pps;
    let [w, h] = sps.dimensions;
    let [min_cb, max_cb] = sps.coding_block_log2;
    let side = 1u32 << max_cb;
    let columns = w.div_ceil(side);
    let total = layout.tile_scan_to_raster.len();
    let mut sao = vec![[hevc_sao::Sao::Off; 3]; total];
    let mut stream = 0;
    let mut visited = 0;
    for &[tx, ty, tw, th] in &layout.rectangles {
        decoder.tile_bounds = Some([tx * side, ty * side, tw * side, th * side]);
        decoder.qp = slice.qp;
        decoder.qp_prediction = slice.qp;
        decoder.qp_coded = false;
        decoder.cu_chroma_offsets = [0; 2];
        decoder.chroma_qp_coded = false;
        let mut bins = HevcCabac::new(
            &slice.rbsp[slice.entropy_substreams[stream].clone()],
            0,
            slice.slice_type,
            slice.cabac_init,
            slice.qp,
        )?;
        let initial = bins.contexts()?;
        let mut saved = None;
        decoder.palette = if sps.palette.is_some() {
            Some(super::hevc_palette::Predictor::from_parameters(sps, pps)?)
        } else {
            None
        };
        let initial_palette = decoder.palette.clone();
        let mut saved_palette = None;
        stream += 1;
        for row in ty..ty + th {
            if row != ty && pps.entropy_sync {
                decoder.palette = saved_palette
                    .clone()
                    .unwrap_or_else(|| initial_palette.clone());
                bins = HevcCabac::from_contexts(
                    &slice.rbsp[slice.entropy_substreams[stream].clone()],
                    0,
                    saved.as_ref().unwrap_or(&initial),
                )?;
                stream += 1;
                decoder.qp = slice.qp;
            }
            for col in tx..tx + tw {
                let raster = (row * columns + col) as usize;
                sao[raster] = hevc_sao::read_ctu_with_scale(
                    &mut bins,
                    slice.sao,
                    sps.depth,
                    pps.sao_offset_scale,
                    if col > tx {
                        Some(&sao[raster - 1])
                    } else {
                        None
                    },
                    if row > ty {
                        Some(&sao[raster - columns as usize])
                    } else {
                        None
                    },
                )?;
                hevc_tree::read_ctu(
                    &mut bins,
                    decoder,
                    [w, h],
                    [col * side, row * side],
                    max_cb,
                    min_cb,
                )?;
                if pps.entropy_sync && col == tx + 1 {
                    saved = Some(bins.contexts()?);
                    saved_palette = Some(decoder.palette.clone());
                }
                visited += 1;
                let last = visited == total;
                if bins.terminate()? != last {
                    return Err(invalid(
                        "HEVC tiled slice termination does not match picture extent",
                    ));
                }
                let tile_end = col == tx + tw - 1 && row == ty + th - 1;
                let row_end = pps.entropy_sync && col == tx + tw - 1;
                if !last && (tile_end || row_end) && !bins.terminate()? {
                    return Err(invalid("missing HEVC tile end-of-substream bit"));
                }
            }
        }
    }
    decoder.tile_bounds = None;
    Ok(sao)
}

// Rows preserve reconstruction order. CABAC and motion metadata can advance
// independently; intra prediction still sees only previously reconstructed pixels.
fn cross_residual(
    component: usize,
    alpha: i8,
    log: u8,
    depths: [u8; 2],
    enabled: bool,
    luma: &mut Vec<i32>,
    residual: &mut Vec<i32>,
) -> Result<()> {
    if !enabled {
        return Ok(());
    }
    if component == 0 {
        luma.clone_from(residual);
    } else if alpha != 0 {
        if residual.is_empty() {
            residual.resize(1usize << (2 * log), 0);
        }
        super::hevc_cross_component::modify(luma, residual, alpha, depths)?;
    }
    Ok(())
}
enum Reconstruction {
    Dbbp(Box<hevc_motion::dbbp::Prediction>),
    InterDc {origin:[usize;2],log:u8,offset:i64},
    OrdinaryDc {origin:[usize;2],log:u8,mode:u8,offset:i64,layer:u8,allowed:[u64;5],filters:[bool;3]},
    Wedge {origin:[usize;2],log:u8,pattern:Vec<bool>,offsets:[i64;2],allowed:[u64;5],layer:u8,residual:Option<hevc_block::Coefficients>},
    SkipAngular {origin:[usize;2],log:u8,mode:u8,filter_boundary:bool,allowed:[u64;5]},
    Single {origin:[usize;2],log:u8,mode:u8,allowed:bool},
    Act {
        origin: [usize; 2],
        log: u8,
        bypass: bool,
        blocks: [Option<hevc_block::Coefficients>; 3],
        alphas: [i8; 3],
    },
    Pcm {
        component: usize,
        rect: [usize; 4],
        samples: Vec<u16>,
    },
    Inter {
        motion: Motion,
        rect: [u32; 4],
        illumination: Option<hevc_motion::illumination::Prediction>,
        residual:Option<Box<hevc_motion::inter_view_residual::Prediction>>,
    },
    Intra {
        component: usize,
        origin: [usize; 2],
        log: u8,
        mode: u8,
        residual: Option<hevc_block::Coefficients>,
        filter_boundary: bool,
        alpha: i8,
    },
    Residual {
        component: usize,
        origin: [usize; 2],
        log: u8,
        residual: Option<hevc_block::Coefficients>,
        alpha: i8,
    },
}
fn reconstruct_row(
    planes: &mut [Plane; 3],
    commands: Vec<Reconstruction>,
    sps: &Sps,
    pps: &Pps,
    slice: &SliceHeader,
    lists: &[Vec<Reference>; 2],
    scratch: &mut Vec<i32>,
    pred_scratch: &mut Vec<u16>,
    transform_scratch: &mut Vec<i32>,
    residual_scratch: &mut Vec<i32>,
) -> Result<()> {
    let scaling = pps.scaling_lists.as_ref().unwrap_or(&sps.scaling_lists);
    let mut luma_residual = Vec::new();
    for command in commands {
        match command {
            Reconstruction::InterDc {origin,log,offset}=>planes[0].add_dc_residual(origin,log,offset)?,
            Reconstruction::OrdinaryDc {origin,log,mode,offset,layer,allowed,filters} => {
                reconstruct_ordinary_dc(&mut planes[0],origin,log,sps.transform_block_log2[1],mode,offset,layer,allowed,filters,sps.depth[0],pps.depth_tables.as_deref(),pred_scratch)?;
            }
            Reconstruction::Wedge {origin,log,pattern,offsets,allowed,layer,residual} => {
                if let Some(residual)=residual {residual.reconstruct(scaling,transform_scratch,residual_scratch)?;}
                else {residual_scratch.clear();}
                reconstruct_wedge(&mut planes[0],origin,log,&pattern,offsets,allowed,layer,sps.depth[0],pps.depth_tables.as_deref(),residual_scratch)?;
            }
            Reconstruction::SkipAngular {origin,log,mode,filter_boundary,allowed} => {
                planes[0].reconstruct_skip_angular(origin,log,mode,filter_boundary,pred_scratch,
                    |x,y|skip_angular_allowed(allowed,origin,1usize<<log,x,y))?;
            }
            Reconstruction::Single {origin,log,mode,allowed} => {
                planes[0].reconstruct_single(origin,log,mode,|_,_|allowed)?;
            }
            Reconstruction::Act {
                origin,
                log,
                bypass,
                blocks,
                alphas,
            } => {
                reconstruct_act(
                    planes,
                    origin,
                    log,
                    bypass,
                    blocks,
                    alphas,
                    sps,
                    pps,
                    transform_scratch,
                    residual_scratch,
                )?;
            }
            Reconstruction::Pcm {
                component,
                rect,
                samples,
            } => {
                planes[component].reconstruct_inter(rect, &samples)?;
            }
            Reconstruction::Dbbp(plan) => {
                for c in 0..if sps.chroma_format==0 {1}else{3} {
                    let shifts=component_shifts(c,sps.chroma_format);
                    let pixels=plan.predict(lists,c,sps.depth[usize::from(c!=0)],slice.weights.as_ref(),scratch,sps.chroma_format,(&*planes,sps.depth),65536)?;
                    planes[c].reconstruct_inter(std::array::from_fn(|axis|plan.rect[axis]as usize>>shifts[axis%2]),&pixels)?;
                }
            }
            Reconstruction::Inter { motion, rect, illumination, residual } => {
                for c in 0..if sps.chroma_format == 0 { 1 } else { 3 } {
                    let shifts = component_shifts(c, sps.chroma_format);
                    let pixels = if let Some(plan)=&residual {
                        plan.predict(c,sps.chroma_format,slice.weights.as_ref(),65536)?
                    } else if let Some(plan)=illumination {
                        hevc_motion::illumination::predict(lists,motion,rect,c,sps.depth[usize::from(c!=0)],scratch,sps.chroma_format,&*planes,sps.depth,plan)?
                    } else {hevc_motion::predict_with_current(
                        lists,
                        motion,
                        rect,
                        c,
                        sps.depth[usize::from(c != 0)],
                        slice.weights.as_ref(),
                        scratch,
                        sps.chroma_format,
                        Some((&*planes, sps.depth)),
                    )?};
                    planes[c].reconstruct_inter(
                        std::array::from_fn(|axis| rect[axis] as usize >> shifts[axis % 2]),
                        &pixels,
                    )?;
                }
            }
            Reconstruction::Intra {
                component,
                origin,
                log,
                mode,
                residual,
                filter_boundary,
                alpha,
            } => {
                if let Some(block) = residual {
                    block.reconstruct(scaling, transform_scratch, residual_scratch)?;
                } else {
                    residual_scratch.clear();
                }
                cross_residual(
                    component,
                    alpha,
                    log,
                    sps.depth,
                    pps.cross_component_prediction,
                    &mut luma_residual,
                    residual_scratch,
                )?;
                planes[component].reconstruct_intra_with_full_chroma_filters(
                    origin,
                    log,
                    mode,
                    component != 0,
                    sps.chroma_format == 3,
                    sps.strong_intra_smoothing,
                    !sps.intra_smoothing_disabled,
                    filter_boundary,
                    residual_scratch,
                    pred_scratch,
                    |_, _| true,
                )?;
            }
            Reconstruction::Residual {
                component,
                origin,
                log,
                residual,
                alpha,
            } => {
                if let Some(residual) = residual {
                    residual.reconstruct(scaling, transform_scratch, residual_scratch)?;
                } else {
                    residual_scratch.clear();
                }
                cross_residual(
                    component,
                    alpha,
                    log,
                    sps.depth,
                    pps.cross_component_prediction,
                    &mut luma_residual,
                    residual_scratch,
                )?;
                planes[component].add_residual(origin, log, residual_scratch)?;
            }
        }
    }
    Ok(())
}

fn ctu_position(
    layout: &Option<super::hevc_tiles::TileLayout>,
    address: u32,
    columns: u32,
    rows: u32,
) -> (u32, u32, u32, [u32; 4], bool, bool) {
    let raster = layout
        .as_ref()
        .map_or(address, |l| l.tile_scan_to_raster[address as usize]);
    let row = raster / columns;
    let col = raster % columns;
    let rectangle = layout.as_ref().map_or([0, 0, columns, rows], |l| {
        l.rectangles[l.tile_ids[raster as usize] as usize]
    });
    let [tx, ty, _, _] = rectangle;
    (
        raster,
        row,
        col,
        rectangle,
        col == tx && row == ty,
        col == tx,
    )
}
fn tile_scan(layout: &Option<super::hevc_tiles::TileLayout>, raster: u32) -> u32 {
    layout
        .as_ref()
        .map_or(raster, |l| l.raster_to_tile_scan[raster as usize])
}
fn deblock(decoder: &mut Decoder<'_>, slice: &SliceHeader) -> Result<()> {
    deblock_slices(decoder, std::slice::from_ref(slice))
}
fn independent_owners(slices: &[SliceHeader]) -> Vec<usize> {
    let mut owner = 0;
    slices
        .iter()
        .enumerate()
        .map(|(index, slice)| {
            if !slice.dependent {
                owner = index;
            }
            owner
        })
        .collect()
}
fn deblock_slices(decoder: &mut Decoder<'_>, slices: &[SliceHeader]) -> Result<()> {
    use super::hevc_deblock as d;
    let owners = independent_owners(slices);
    let [width, height] = decoder.sps.dimensions.map(|v| v as usize);
    // Complete vertical filtering precedes horizontal filtering.
    for direction in 0..2 {
        for y in (0..height).step_by(4) {
            for x in (0..width).step_by(4) {
                let edge = if direction == 0 { x } else { y };
                if edge == 0
                    || edge % 8 != 0
                    || decoder.edges[y / 4 * (width / 4) + x / 4][direction] == 0
                {
                    continue;
                }
                let side = 1usize << decoder.sps.coding_block_log2[1];
                let owner = |x: usize, y: usize| {
                    slices.partition_point(|s| {
                        tile_scan(&decoder.tile_layout, s.address)
                            <= tile_scan(
                                &decoder.tile_layout,
                                (y / side * width.div_ceil(side) + x / side) as u32,
                            )
                    }) - 1
                };
                let q_owner = owner(x, y);
                let p_owner = if direction == 0 {
                    owner(x - 1, y)
                } else {
                    owner(x, y - 1)
                };
                if let Some(layout) = &decoder.tile_layout
                    && !decoder.pps.tiles.as_ref().unwrap().loop_filter_across
                {
                    let columns = decoder.sps.dimensions[0].div_ceil(side as u32) as usize;
                    let q = y / side * columns + x / side;
                    let p = if direction == 0 {
                        y / side * columns + (x - 1) / side
                    } else {
                        (y - 1) / side * columns + x / side
                    };
                    if layout.tile_ids[p] != layout.tile_ids[q] {
                        continue;
                    }
                }
                let slice = &slices[q_owner];
                if slice.deblocking.disabled
                    || (owners[p_owner] != owners[q_owner]
                        && !slices[p_owner.max(q_owner)].loop_filter_across_slices)
                {
                    continue;
                }
                let q = decoder.qp_grid[y / 8 * (width / 8) + x / 8];
                let p = if direction == 0 {
                    decoder.qp_grid[y / 8 * (width / 8) + (x - 1) / 8]
                } else {
                    decoder.qp_grid[(y - 1) / 8 * (width / 8) + x / 8]
                };
                let a = decoder
                    .cell_raw(
                        if direction == 0 {
                            x as i32 - 1
                        } else {
                            x as i32
                        },
                        if direction == 0 {
                            y as i32
                        } else {
                            y as i32 - 1
                        },
                    )
                    .unwrap();
                let b = decoder.cell_raw(x as i32, y as i32).unwrap();
                let pcm_excluded = decoder
                    .sps
                    .pcm
                    .as_ref()
                    .is_some_and(|p| p.loop_filter_disabled);
                let filter_enabled = [
                    !(a.bypass || a.palette || (a.pcm && pcm_excluded)),
                    !(b.bypass || b.palette || (b.pcm && pcm_excluded)),
                ];
                let strength = if a.intra || b.intra {
                    2
                } else if decoder.edges[y / 4 * (width / 4) + x / 4][direction] & 2 != 0
                    && (a.cbf || b.cbf)
                {
                    1
                } else {
                    u8::from(motion_boundary(a.motion, b.motion, decoder.lists))
                };
                if strength == 0 {
                    continue;
                }
                let [beta, tc] = d::luma_thresholds(
                    [p, q],
                    strength,
                    slice.deblocking.offsets_div2,
                    decoder.sps.depth[0],
                )?;
                let plane = decoder.planes[0].samples_mut();
                let index = |line: usize, side: usize, distance: usize| {
                    let delta = if side == 0 {
                        -(distance as isize) - 1
                    } else {
                        distance as isize
                    };
                    if direction == 0 {
                        (y + line) * width + x.checked_add_signed(delta).unwrap()
                    } else {
                        y.checked_add_signed(delta).unwrap() * width + x + line
                    }
                };
                let input: [[[u16; 4]; 2]; 4] = std::array::from_fn(|l| {
                    std::array::from_fn(|s| std::array::from_fn(|v| plane[index(l, s, v)]))
                });
                let mode =
                    d::luma_decision([input[0], input[3]], [beta, tc], decoder.sps.depth[0])?;
                // Convert to fvid-cpu's HevcLumaFilter for batched processing
                let batch_filter = match mode {
                    d::LumaFilter::Off => fvid_cpu::HevcLumaFilter::Off,
                    d::LumaFilter::Strong => fvid_cpu::HevcLumaFilter::Strong,
                    d::LumaFilter::Weak { second } => fvid_cpu::HevcLumaFilter::Weak { second },
                };
                let p_samples: [[u16; 4]; 4] = std::array::from_fn(|l| input[l][0]);
                let q_samples: [[u16; 4]; 4] = std::array::from_fn(|l| input[l][1]);
                let (out_p, out_q) = fvid_cpu::hevc_deblock_luma_batch4(
                    p_samples,
                    q_samples,
                    tc,
                    decoder.sps.depth[0],
                    batch_filter,
                    [filter_enabled; 4],
                );
                for l in 0..4 {
                    for s in 0..2 {
                        let output = if s == 0 { out_p[l] } else { out_q[l] };
                        for v in 0..3 {
                            plane[index(l, s, v)] = output[v];
                        }
                    }
                }
                let shifts = component_shifts(1, decoder.sps.chroma_format);
                if edge % (8 << shifts[direction]) == 0
                    && (if direction == 0 { y } else { x }) % (4 << shifts[1 - direction]) == 0
                    && strength == 2
                    && decoder.sps.chroma_format != 0
                {
                    for c in 1..3 {
                        let tc = d::chroma_tc_with_format(
                            p,
                            q,
                            decoder.pps.chroma_qp_offsets[c - 1],
                            slice.deblocking.offsets_div2[1],
                            decoder.sps.depth,
                            decoder.sps.chroma_format,
                        )?;
                        let [shift_x, shift_y] = shifts;
                        let plane = decoder.planes[c].samples_mut();
                        for l in 0..4 {
                            let index = |side: usize, v: usize| {
                                let delta = if side == 0 {
                                    -(v as isize) - 1
                                } else {
                                    v as isize
                                };
                                if direction == 0 {
                                    ((y >> shift_y) + l) * (width >> shift_x)
                                        + (x >> shift_x).checked_add_signed(delta).unwrap()
                                } else {
                                    (y >> shift_y).checked_add_signed(delta).unwrap()
                                        * (width >> shift_x)
                                        + (x >> shift_x)
                                        + l
                                }
                            };
                            let a = [plane[index(0, 0)], plane[index(0, 1)]];
                            let b = [plane[index(1, 0)], plane[index(1, 1)]];
                            let output =
                                d::chroma_sample(a, b, tc, decoder.sps.depth[1], filter_enabled)?;
                            plane[index(0, 0)] = output[0];
                            plane[index(1, 0)] = output[1];
                        }
                    }
                }
            }
        }
    }
    Ok(())
}

impl Decoder<'_> {
    fn depth_neighbour_mask(&self,origin:[usize;2],size:usize)->[u64;5] {
        let [x,y]=origin;let mut mask=[0u64;5];
        for index in 0..=4*size {
            let point=if index<2*size {y.checked_sub(1).map(|yy|(x+index,yy))}
                else if index<4*size {x.checked_sub(1).map(|xx|(xx,y+index-2*size))}
                else {x.checked_sub(1).zip(y.checked_sub(1))};
            let allowed=point.is_some_and(|(xx,yy)| {
                if xx>=self.sps.dimensions[0] as usize || yy>=self.sps.dimensions[1] as usize {return false;}
                if self.tile_bounds.is_some_and(|[tx,ty,w,h]|xx<tx as usize || yy<ty as usize || xx>=(tx+w) as usize || yy>=(ty+h) as usize) {return false;}
                let side=1usize<<self.sps.coding_block_log2[1];
                let address=yy/side*(self.sps.dimensions[0] as usize).div_ceil(side)+xx/side;
                tile_scan(&self.tile_layout,address as u32)>=self.slice_start && (!self.pps.constrained_intra || self.cell(xx as i32,yy as i32).is_some_and(|c|c.intra))
            });
            if allowed {mask[index/64]|=1<<(index%64);}
        }
        mask
    }

    fn read_chroma_qp(
        &mut self,
        b: &mut HevcCabac<'_>,
        coded: [bool; 3],
        bypass: bool,
    ) -> Result<()> {
        if self.slice.cu_chroma_qp_offset_enabled
            && !self.chroma_qp_coded
            && !bypass
            && (coded[1] || coded[2])
        {
            let table = self
                .pps
                .chroma_qp_offset_list
                .as_ref()
                .ok_or_else(|| invalid("HEVC active chroma QP selection has no list"))?;
            self.cu_chroma_offsets = hevc_qp::read_chroma_offset(b, &table.entries)?;
            self.chroma_qp_coded = true;
        }
        Ok(())
    }

    fn cell(&self, x: i32, y: i32) -> Option<Cell> {
        if x < 0
            || y < 0
            || x as u32 >= self.sps.dimensions[0]
            || y as u32 >= self.sps.dimensions[1]
        {
            return None;
        }
        if let Some([tx, ty, width, height]) = self.tile_bounds {
            if x < tx as i32 || y < ty as i32 || x as u32 >= tx + width || y as u32 >= ty + height {
                return None;
            }
        }
        if x >= 0 && y >= 0 {
            let side = 1u32 << self.sps.coding_block_log2[1];
            let columns = self.sps.dimensions[0].div_ceil(side);
            if tile_scan(
                &self.tile_layout,
                y as u32 / side * columns + (x as u32 / side),
            ) < self.slice_start
            {
                return None;
            }
        }
        self.cell_raw(x, y)
    }
    fn cell_raw(&self, x: i32, y: i32) -> Option<Cell> {
        let [w, h] = self.sps.dimensions;
        if x < 0 || y < 0 || x as u32 >= w || y as u32 >= h {
            None
        } else {
            Some(self.cells[y as usize / 4 * (w as usize / 4) + x as usize / 4])
        }
    }
    fn finish_cu(&mut self, n: Node, intra: bool, skip: bool) {
        let stride = self.sps.dimensions[0] as usize / 8;
        for y in n.y as usize / 8..(n.y as usize + (1 << n.log2_size)) / 8 {
            for x in n.x as usize / 8..(n.x as usize + (1 << n.log2_size)) / 8 {
                self.qp_grid[y * stride + x] = self.qp;
            }
        }
        let stride = stride * 2;
        let x = n.x as usize / 4;
        let y = n.y as usize / 4;
        let side = 1usize << (n.log2_size - 2);
        for k in 0..side {
            self.edges[(y + k) * stride + x][0] |= 2;
            self.edges[y * stride + x + k][1] |= 2;
        }
        for y in n.y as usize / 4..(n.y as usize + (1 << n.log2_size)) / 4 {
            for x in n.x as usize / 4..(n.x as usize + (1 << n.log2_size)) / 4 {
                let cell = &mut self.cells[y * stride + x];
                cell.ready = true;
                cell.depth = n.depth;
                cell.intra = intra;
                cell.skip = skip;
            }
        }
    }
    fn reconstruct_inter_part(&mut self,n:Node,partition:Partition,skip:bool,bypass:bool,
        rect:[u32;4],motion:Motion,ic_enabled:bool,
        residual_input:Option<(u8,&hevc_motion::inter_view_residual::ReferenceTargets,hevc_motion::disparity::derivation::Vector)>)->Result<()> {
        hevc_motion::validate_current_vectors(
            self.lists,
            motion,
            rect,
            [n.x, n.y],
            self.sps.coding_block_log2[1],
            self.sps.chroma_format,
            |x, y| self.cell(x, y).is_some_and(|c| c.ready),
        )?;
        #[cfg(test)]
        if motion.iter().all(Option::is_some) {
            self.bipredicted_blocks += 1;
        }
        #[cfg(test)]
        for (list, vector) in motion.iter().enumerate() {
            if let Some(v) = vector {
                if self.lists[list][v.reference as usize].picture.is_none() {
                    self.current_picture_blocks += 1;
                    if self.sps.chroma_format == 1 && v.mv.iter().any(|x| x & 7 != 0) {
                        self.fractional_current_chroma_blocks += 1;
                    }
                } else {
                    self.completed_picture_blocks += 1;
                }
            }
        }
        let illumination=if ic_enabled {
            let context=self.context.ok_or_else(||invalid("HEVC illumination requires layered binding"))?;
            let graph=&context.vps.extension.as_ref().ok_or_else(||invalid("HEVC illumination requires view graph"))?.graph;
            let view=graph.view_index(self.slice.nal.layer_id)?;
            let mut cross_view=[false;2];
            for list in 0..2 {if let Some(v)=motion[list] {
                cross_view[list]=graph.view_index(self.lists[list][v.reference as usize].layer_id)?!=view;
            }}
            Some(hevc_motion::illumination::Prediction {origin:[n.x,n.y],log2_cu:n.log2_size,
                available:[self.cell(n.x as i32,n.y as i32-1).is_some_and(|c|c.ready),self.cell(n.x as i32-1,n.y as i32).is_some_and(|c|c.ready)],cross_view})
        } else {None};
        let residual=if let Some((weight,targets,disparity))=residual_input {
            let sources=self.residual_sources.ok_or_else(||invalid("missing HEVC residual source binding"))?;
            let graph=&self.context.ok_or_else(||invalid("missing HEVC residual context"))?.vps.extension.as_ref()
                .ok_or_else(||invalid("missing HEVC residual view graph"))?.graph;
            let inputs=hevc_motion::inter_view_residual::PredictionInputs {lists:self.lists,motion,targets,
                current_poc:self.poc,rect,disparity};
            let mut plans=[None,None];let mut references=[0;2];
            for list in 0..2 {if let Some(vector)=motion[list] {
                references[list]=vector.reference;
                plans[list]=Some(hevc_motion::inter_view_residual::prepare_prediction(&inputs,list,
                    |layer|graph.view_index(layer),|view|sources.current_view(view),|view,poc|sources.temporal_view(view,poc))?);
            }}
            Some(Box::new(hevc_motion::inter_view_residual::Prediction {plans,references,weight}))
        }else{None};
        if self.jobs.is_some() {
            #[cfg(test)] {self.queued_inter_blocks+=1;}
            self.reconstruction
                .push(Reconstruction::Inter { motion, rect, illumination, residual });
        } else {
            for c in 0..if self.sps.chroma_format == 0 { 1 } else { 3 } {
                let shifts = component_shifts(c, self.sps.chroma_format);
                let pixels = if let Some(plan)=&residual {
                    plan.predict(c,self.sps.chroma_format,self.slice.weights.as_ref(),65536)?
                } else if let Some(plan)=illumination {
                    hevc_motion::illumination::predict(self.lists,motion,rect,c,self.sps.depth[usize::from(c!=0)],&mut self.scratch,self.sps.chroma_format,&self.planes,self.sps.depth,plan)?
                } else {hevc_motion::predict_with_current(
                    self.lists,
                    motion,
                    rect,
                    c,
                    self.sps.depth[usize::from(c != 0)],
                    self.slice.weights.as_ref(),
                    &mut self.scratch,
                    self.sps.chroma_format,
                    Some((&self.planes, self.sps.depth)),
                )?};
                self.planes[c].reconstruct_inter(
                    std::array::from_fn(|axis| rect[axis] as usize >> shifts[axis % 2]),
                    &pixels,
                )?;
            }
        }
        self.publish_inter_cells(n,partition,skip,bypass,rect,motion,residual_input.map_or(0,|v|v.0));
        Ok(())
    }
    fn publish_inter_cells(&mut self,n:Node,partition:Partition,skip:bool,bypass:bool,rect:[u32;4],motion:Motion,residual_weight:u8) {
        let stride=self.sps.dimensions[0]as usize/4;
        let [x, y, w, h] = rect.map(|v| v as usize / 4);
        for yy in y..y + h {
            for xx in x..x + w {
                self.cells[yy * stride + xx] = Cell {
                    ready: true,
                    bypass,
                    depth: n.depth,
                    skip,
                    motion,
                    partition:partition as u8,
                    residual_weight,
                    ..Cell::default()
                };
            }
        }
    }
    fn prediction_edges(&mut self,rect:[u32;4]) {
        let stride=self.sps.dimensions[0] as usize/4;
        let [x,y,w,h]=rect.map(|v|v as usize/4);
        for yy in y..y+h {self.edges[yy*stride+x][0]|=1;}
        for xx in x..x+w {self.edges[y*stride+xx][1]|=1;}
    }
    fn disparity_neighbour(&self,point:[i64;2])->Result<Option<hevc_motion::disparity::derivation::Neighbour>> {
        use hevc_motion::disparity::derivation::{Neighbour,NeighbourVector,Vector};
        let (Ok(x),Ok(y))=(i32::try_from(point[0]),i32::try_from(point[1]))else{return Ok(None)};
        let Some(cell)=self.cell(x,y).filter(|c|c.ready&&!c.intra)else{return Ok(None)};
        let context=self.context.ok_or_else(||invalid("missing HEVC spatial disparity binding"))?;
        let graph=&context.vps.extension.as_ref().ok_or_else(||invalid("missing HEVC spatial disparity graph"))?.graph;
        let mut motion=[None;2];
        for list in 0..2 {if let Some(vector)=cell.motion[list] {
            let reference=self.lists[list].get(vector.reference as usize).ok_or_else(||invalid("HEVC disparity neighbour reference is missing"))?;
            motion[list]=Some(NeighbourVector{reference_view:graph.view_index(reference.layer_id)?,poc:reference.poc,mv:vector.mv});
        }}
        let state=self.iv_state.as_ref().and_then(|map|map[y as usize/8*(self.sps.dimensions[0]as usize/8)+x as usize/8]);
        Ok(Some(Neighbour{motion,skip:cell.skip,inter_view:state.is_some(),
            refined:state.map(|s|Vector{reference_view:s.vector.reference_view,mv:s.refined_mv})}))
    }
    fn inter(&mut self, b: &mut HevcCabac<'_>, n: Node, skip: bool, bypass: bool) -> Result<()> {
        if !self.sps.depth_layer && self.sps.three_d.as_ref().is_some_and(|e|e.inter_view_residual_prediction)
            && self.context.is_some_and(|c|c.vps.extension.as_ref().is_some_and(|e|
                e.graph.reference_list_layers(c.sps.layer_id).is_ok_and(|mask|mask!=0))) {
            let sources=self.residual_sources.ok_or_else(||crate::unsupported("HEVC residual prediction requires current-AU source RPS"))?;
            sources.validate_for(self.context.ok_or_else(||invalid("HEVC residual prediction requires layered context"))?,self.slice,self.poc)?;
        }
        if self.texture_disparity.is_some() {
            let sources=self.view_components.ok_or_else(||crate::unsupported("HEVC texture IV requires current-AU view components"))?;
            if sources.depth() {return Err(invalid("HEVC texture IV component pool has depth role"));}

        }
        let (inferred_part,part_idc)=self.cqt_partition(n)?;
        let partition = if skip {
            Partition::Full
        } else {
            hevc_inter_syntax::partition_predicted(
                b,
                n.log2_size,
                self.sps.coding_block_log2[0],
                self.sps.amp,
                inferred_part,part_idc,
            )?
        };
        let stride = self.sps.dimensions[0] as usize / 4;
        let mut full_merge = false;
        let mut cu_disparity=None;let mut disparity_prepared=false;
        let dbbp_eligible=!skip && n.log2_size>3 && matches!(partition,Partition::Horizontal|Partition::Vertical)
            && self.context.map(|context|texture_dbbp_enabled(context,self.slice)).transpose()?.unwrap_or(false);
        let staged=if dbbp_eligible {
            let rectangles=partition.rectangles(1<<n.log2_size);
            Some([hevc_inter_syntax::prediction(b,self.slice,skip,n.depth,[rectangles[0][2],rectangles[0][3]])?,
                hevc_inter_syntax::prediction(b,self.slice,skip,n.depth,[rectangles[1][2],rectangles[1][3]])?])
        }else{None};
        let mut dbbp=false;
        let mut dbbp_motions=[None,None];
        for (part_index, [dx, dy, w, h]) in partition
            .rectangles(1 << n.log2_size)
            .into_iter()
            .enumerate()
        {
            let rect = [n.x + dx, n.y + dy, w, h];
            let syntax = if let Some(staged)=staged {staged[part_index]}else{hevc_inter_syntax::prediction(b, self.slice, skip, n.depth, [w, h])?};
            let ic_flags=self.slice.layered.as_ref().and_then(|s|s.illumination_compensation).unwrap_or([false;2]);
            let merge_index=if let hevc_inter_syntax::Prediction::Merge(i)=syntax {Some(i)}else{None};
            let mut cross_view=[false;2];
            if ic_flags[0] && let hevc_inter_syntax::Prediction::Explicit {references,..}=syntax {
                let graph=&self.context.ok_or_else(||invalid("HEVC IC requires layered binding"))?.vps.extension.as_ref().ok_or_else(||invalid("HEVC IC requires view graph"))?.graph;
                let view=graph.view_index(self.slice.nal.layer_id)?;
                for list in 0..2 {if let Some(index)=references[list] {
                    let reference=self.lists[list].get(index as usize).ok_or_else(||invalid("HEVC IC reference index out of range"))?;
                    cross_view[list]=graph.view_index(reference.layer_id)?!=view;
                }}
            }
            if !disparity_prepared {
                if let Some(prepared)=&self.texture_disparity {
                    cu_disparity=prepared.derive([n.x,n.y],1<<n.log2_size,
                        matches!(syntax,hevc_inter_syntax::Prediction::Merge(_)),None,0,|point|self.disparity_neighbour(point))?;
                    if texture_depth_refinement_enabled(self.context.unwrap(),self.slice)? {
                        let (pool,current)=self.refinement_components.ok_or_else(||invalid("HEVC texture refinement requires current-AU depth pool"))?;
                        cu_disparity=cu_disparity.map(|state|pool.refine_disparity(
                            self.context.unwrap(),self.slice,current,state.vector,[n.x,n.y],1<<n.log2_size,65536)).transpose()?;
                    }
                }
                if dbbp_eligible {
                    dbbp=hevc_motion::dbbp::read_flag(b,hevc_motion::dbbp::CuSyntax {inter:true,skipped:skip,enabled:true,
                        disparity_available:cu_disparity.is_some(),log2_size:n.log2_size,partition})?;
                }
                disparity_prepared=true;
            }
            let residual_enabled=self.sps.three_d.as_ref().is_some_and(|e|e.inter_view_residual_prediction)
                && self.residual_sources.is_some();
            let residual_targets=if residual_enabled && partition==Partition::Full {
                let graph=&self.context.unwrap().vps.extension.as_ref().unwrap().graph;
                let mut identities=[[hevc_motion::texture::Identity {poc:0,view:0};15];2];
                for list in 0..2 {
                    if self.lists[list].len()>15 {return Err(invalid("HEVC residual active list exceeds limit"));}
                    for (index,r) in self.lists[list].iter().enumerate() {
                        identities[list][index]=hevc_motion::texture::Identity {poc:r.poc,view:graph.view_index(r.layer_id)?};
                    }
                }
                let layers=&self.slice.layered.as_ref().unwrap().reference_layers;
                if layers.len()>64 {return Err(invalid("HEVC residual reference views exceed limit"));}
                let mut views=[0u16;64];
                for (index,&layer) in layers.iter().enumerate() {views[index]=graph.view_index(layer)?;}
                Some(hevc_motion::inter_view_residual::reference_targets(
                    [&identities[0][..self.lists[0].len()],&identities[1][..self.lists[1].len()]],
                    self.slice.slice_type==SliceType::B,self.poc,cu_disparity.is_some(),&views[..layers.len()],
                    |view,poc|Ok(self.residual_sources.unwrap().temporal_view(view,poc)?.is_some()))?)
            }else{None};
            let residual_weight=hevc_motion::inter_view_residual::read_weight(b,
                hevc_motion::inter_view_residual::CuSyntax {inter:true,full_partition:partition==Partition::Full,
                    enabled:residual_enabled,reference_available:residual_targets.as_ref().is_some_and(|t|t.syntax_available()),
                    left_weight:self.cell(n.x as i32-1,n.y as i32).filter(|c|c.ready).map(|c|c.residual_weight)})?;
            let residual_input=if residual_weight!=0 {Some((residual_weight,residual_targets.as_ref().unwrap(),cu_disparity.unwrap().vector))}else{None};
            let illumination=hevc_motion::illumination::read_flag(b,hevc_motion::illumination::CuSyntax {
                inter:true,full_partition:partition==Partition::Full,slice_enabled:ic_flags[0],disabled_merge_zero:ic_flags[1],merge_index,cross_view,residual_prediction_weight:residual_weight,
            })?;
            let mut texture_iv=if let Some(state)=cu_disparity.filter(|_|matches!(syntax,hevc_inter_syntax::Prediction::Merge(_))&&w+h!=12) {
                Some(hevc_motion::texture_inter_view::derive(self.context.unwrap(),self.slice,self.poc,self.lists,
                    self.view_components.unwrap(),state,rect,65536)?)
            }else{None};
            if let (Some(candidates),Some(state),Some(context))=(&mut texture_iv,cu_disparity,self.context) {
                if !dbbp && !illumination && residual_weight==0 && texture_vsp_enabled(context,self.slice)? {
                    let (pool,current)=self.refinement_components.ok_or_else(||invalid("HEVC VSP requires current-AU depth pool"))?;
                    candidates.vsp=pool.view_synthesis(context,self.slice,current,state.vector,self.lists,rect,65536)?;
                    candidates.left_vsp=self.cell(rect[0]as i32-1,(rect[1]+rect[3]-1)as i32).is_some_and(|c|c.ready&&!c.intra&&c.vsp);
                }
            }
            full_merge = partition == Partition::Full
                && matches!(syntax, hevc_inter_syntax::Prediction::Merge(_));
            let spatial = Spatial {
                rect,
                cu: [n.x, n.y, 1 << n.log2_size],
                partition,
                part_index,
                merge_log2: self.pps.parallel_merge_log2,
                ctu_log2: self.sps.coding_block_log2[1],
                poc: self.poc,
                lists: self.lists,
            };
            let subblocks=if texture_motion_enabled(self.sps,self.slice)
                && matches!(syntax,hevc_inter_syntax::Prediction::Merge(_)) && w+h!=12 {
                let context=self.context.ok_or_else(||invalid("missing HEVC texture motion binding"))?;
                let texture=self.texture.ok_or_else(||invalid("missing HEVC texture motion source"))?;
                Some(derive_texture_subblocks(context,self.slice,texture,self.lists,rect)?)
            } else {None};
            let candidate=subblocks.as_ref().filter(|v|v.default.iter().any(Option::is_some))
                .map(|v|hevc_motion::texture::quarter_motion(v.default));
            let sample = |x,y| self.cell(x,y).filter(|c|c.ready && !c.intra).map(|c|c.motion);
            let resolved=if let Some(context)=self.context {
                let iv=if matches!(syntax,hevc_inter_syntax::Prediction::Merge(_))&&w+h!=12 {
                    self.depth_iv.as_ref().map(|source|source.candidate(rect)).transpose()?.flatten()
                }else{None};
                if let Some(candidates)=&texture_iv {
                    spatial.resolve_texture_layered(syntax,self.slice,sample,&candidates.specials(),context)?
                }else{spatial.resolve_layered_with_iv(syntax,self.slice,sample,candidate,iv,context)?}
            }else if self.sps.depth_layer {
                spatial.resolve_depth_with_texture(syntax,self.slice,sample,candidate)?
            } else {hevc_motion::Resolved::Ordinary(spatial.resolve(syntax,self.slice,sample)?)};
            let selected_iv=matches!(resolved,hevc_motion::Resolved::InterView(_)|hevc_motion::Resolved::InterViewShift(_));
            let mut motion=match resolved {
                hevc_motion::Resolved::InterView(_) if dbbp=>texture_iv.as_ref().ok_or_else(||invalid("missing DBBP IV default motion"))?.blocks.default,
                hevc_motion::Resolved::Ordinary(motion)=>motion,
                hevc_motion::Resolved::ViewSynthesis=>{
                    let vsp=texture_iv.as_mut().and_then(|c|c.vsp.take()).ok_or_else(||invalid("missing selected HEVC VSP sub-blocks"))?;
                    for index in 0..vsp.subblock_count() {
                        let (block,wide)=vsp.subblock(index)?;
                        let motion=wide.map(|v|v.map(|v|hevc_motion::Vector{reference:v.reference,mv:v.mv}));
                        self.reconstruct_inter_part(n,partition,skip,bypass,block,motion,illumination,residual_input)?;
                    }
                    let stride=self.sps.dimensions[0]as usize/4;
                    for y in rect[1]/4..(rect[1]+rect[3])/4 {for x in rect[0]/4..(rect[0]+rect[2])/4 {self.cells[y as usize*stride+x as usize].vsp=true;}}
                    self.prediction_edges(rect);continue;
                }
                hevc_motion::Resolved::InterView(_) if !self.sps.depth_layer=>{
                    let blocks=texture_iv.take().ok_or_else(||invalid("missing selected texture IV sub-blocks"))?.blocks;
                    let [sw,sh]=blocks.size;let cols=w/sw;
                    for (index,motion)in blocks.motion.into_iter().enumerate() {
                        let block=[rect[0]+index as u32%cols*sw,rect[1]+index as u32/cols*sh,sw,sh];
                        self.reconstruct_inter_part(n,partition,skip,bypass,block,motion,illumination,residual_input)?;
                    }
                    publish_iv_disparity(self.iv_state.as_mut().ok_or_else(||invalid("missing IV publication map"))?,
                        self.sps.dimensions,rect,cu_disparity.unwrap())?;
                    self.prediction_edges(rect);continue;
                }
                hevc_motion::Resolved::InterView(motion)|hevc_motion::Resolved::InterViewShift(motion)=>motion,
                hevc_motion::Resolved::Texture=>{
                    let blocks=subblocks.ok_or_else(||invalid("missing selected HEVC texture sub-blocks"))?;
                    #[cfg(test)] if self.jobs.is_some() {self.queued_texture_blocks+=blocks.motion.len();}
                    let [sw,sh]=blocks.size;let cols=w/sw;
                    if blocks.motion.len()!=(cols*(h/sh)) as usize {return Err(invalid("HEVC texture sub-block count disagrees"));}
                    for (index,motion) in blocks.motion.into_iter().enumerate() {
                        let block=[rect[0]+index as u32%cols*sw,rect[1]+index as u32/cols*sh,sw,sh];
                        self.reconstruct_inter_part(n,partition,skip,bypass,block,hevc_motion::texture::quarter_motion(motion),illumination,residual_input)?;
                    }
                    self.prediction_edges(rect);
                    continue;
                }
            };
            if dbbp {
                if motion.iter().all(Option::is_some) {
                    if matches!(syntax,hevc_inter_syntax::Prediction::Merge(_)) {motion[1]=None;}
                    else {return Err(invalid("HEVC DBBP explicit predictor is bidirectional"));}
                }
                dbbp_motions[part_index]=Some(motion);
                self.publish_inter_cells(n,partition,skip,bypass,rect,motion,0);
            }else {
            self.reconstruct_inter_part(n,partition,skip,bypass,rect,motion,illumination,residual_input)?;
            }
            if selected_iv {
                let state=if self.sps.depth_layer {self.depth_iv.as_ref().ok_or_else(||invalid("missing selected HEVC IV disparity"))?.disparity()}else{cu_disparity.ok_or_else(||invalid("missing texture IV disparity"))?};
                let map=self.iv_state.as_mut().ok_or_else(||invalid("missing HEVC IV publication storage"))?;
                publish_iv_disparity(map,self.sps.dimensions,rect,state)?;
            }
            self.prediction_edges(rect);
        }
        if dbbp {
            let (pool,current)=self.refinement_components.ok_or_else(||invalid("HEVC DBBP requires current-AU depth pool"))?;
            let contour=pool.dbbp_contour(self.context.unwrap(),self.slice,current,cu_disparity.unwrap(),[n.x,n.y],1<<n.log2_size,65536)?;
            let plan=Box::new(hevc_motion::dbbp::Prediction {contour,motions:[dbbp_motions[0].ok_or_else(||invalid("missing DBBP first predictor"))?,dbbp_motions[1].ok_or_else(||invalid("missing DBBP second predictor"))?],rect:[n.x,n.y,1<<n.log2_size,1<<n.log2_size],partition});
            if self.jobs.is_some() {
                #[cfg(test)] {self.queued_inter_blocks+=1;}
                self.reconstruction.push(Reconstruction::Dbbp(plan));
            }else {
                for c in 0..if self.sps.chroma_format==0 {1}else{3} {
                    let shifts=component_shifts(c,self.sps.chroma_format);
                    let pixels=plan.predict(self.lists,c,self.sps.depth[usize::from(c!=0)],self.slice.weights.as_ref(),&mut self.scratch,self.sps.chroma_format,(&self.planes,self.sps.depth),65536)?;
                    self.planes[c].reconstruct_inter(std::array::from_fn(|axis|plan.rect[axis]as usize>>shifts[axis%2]),&pixels)?;
                }
            }
        }
        let inter_dc_enabled=self.sps.depth_layer && self.sps.three_d.as_ref().is_some_and(|e|e.inter_dc_only);
        let dc_only=super::hevc_depth_dc::read_dc_only(b,super::hevc_depth_dc::CodingUnit {
            intra:false,full_partition:partition==Partition::Full,skipped:skip,
            skip_intra:false,enabled:[false,inter_dc_enabled],
        })?;
        if dc_only {
            if self.sps.chroma_format!=0 {return Err(invalid("HEVC inter DC-only requires monochrome depth"));}
            let offsets=super::hevc_depth_dc::read_cu_offsets(b,false,false,true,false,[false;4])?;
            let origin=[n.x as usize,n.y as usize];let offset=i64::from(offsets[0][0]);
            if self.jobs.is_some() {
                #[cfg(test)] {self.queued_inter_dc_blocks+=1;}
                self.reconstruction.push(Reconstruction::InterDc {origin,log:n.log2_size,offset});
            } else {self.planes[0].add_dc_residual(origin,n.log2_size,offset)?;}
            return Ok(());
        }
        if skip || (!full_merge && !b.decision(Syntax::RootCbf, 0)?) {
            return Ok(());
        }
        let config = hevc_transform_tree::Config {
            log2_cu: n.log2_size,
            log2_min_transform: self.sps.transform_block_log2[0],
            log2_max_transform: self.sps.transform_block_log2[1],
            max_depth: self.sps.transform_hierarchy_depth[0],
            intra_split: false,
        };
        hevc_transform_tree::read_inter_with_chroma(
            b,
            [n.x, n.y],
            config,
            partition != Partition::Full,
            self.sps.chroma_format,
            |b, u| {
                let act = self.pps.adaptive_colour_transform
                    && u.coded.iter().any(|&v| v)
                    && b.decision(Syntax::ResidualAct, 0)?;
                if act && bypass && self.sps.depth[0] != self.sps.depth[1] {
                    return Err(invalid("HEVC bypass ACT requires equal component depths"));
                }
                #[cfg(test)]
                {
                    self.act_blocks += usize::from(act);
                }
                let mut act_blocks = std::array::from_fn(|_| None);
                let mut act_alphas = [0; 3];
                if self.pps.cu_qp_delta_depth.is_some()
                    && !self.qp_coded
                    && u.coded.iter().any(|&v| v)
                {
                    let delta = hevc_qp::read_delta(b, self.sps.depth[0])?;
                    self.qp =
                        hevc_qp::luma(self.qp_prediction, [None; 2], delta, self.sps.depth[0])?;
                    self.qp_coded = true;
                }
                let x = u.origin[0] as usize / 4;
                let y = u.origin[1] as usize / 4;
                let side = 1usize << (u.log2_size - 2);
                for k in 0..side {
                    self.edges[(y + k) * stride + x][0] |= 2;
                    self.edges[y * stride + x + k][1] |= 2;
                }
                for yy in y..y + side {
                    for xx in x..x + side {
                        self.cells[yy * stride + xx].cbf = u.coded[0];
                    }
                }
                self.read_chroma_qp(b, u.coded_components(), bypass)?;
                let qps = self.component_qps(act)?;
                let mut luma_residual = Vec::new();
                for block in u.component_blocks(self.sps.chroma_format) {
                    let c = block.component;
                    let alpha = if c != 0 && self.pps.cross_component_prediction && u.coded[0] {
                        super::hevc_cross_component::read_alpha(b, c)?
                    } else {
                        0
                    };
                    #[cfg(test)]
                    {
                        self.cross_component_blocks += usize::from(alpha != 0);
                    }
                    if !block.coded && alpha == 0 {
                        continue;
                    }
                    let config = hevc_block::Config {
                        log2_size: block.log2_size,
                        component: c as u8,
                        bit_depth: self.sps.depth[usize::from(c != 0)],
                        qp: qps[c],
                        intra_mode: None,
                        transform_skip_enabled: self.pps.transform_skip,
                        transquant_bypass: bypass,
                        sign_hiding: self.pps.sign_data_hiding,
                    };
                    let residual = if block.coded {
                        Some(hevc_block::read_with_precision(
                            b,
                            config,
                            self.sps.transform_skip_rotation,
                            self.sps.transform_skip_context,
                            self.sps.implicit_rdpcm,
                            self.sps.explicit_rdpcm,
                            self.pps.transform_skip_max_log2,
                            self.sps.persistent_rice,
                            self.sps.extended_precision,
                            self.sps.cabac_bypass_alignment,
                            self.sps.chroma_format == 3,
                        )?)
                    } else {
                        None
                    };
                    if act {
                        act_blocks[c] = residual;
                        act_alphas[c] = alpha;
                        continue;
                    }
                    let origin = block.origin;
                    if self.jobs.is_some() {
                        self.reconstruction.push(Reconstruction::Residual {
                            component: c,
                            origin: origin.map(|v| v as usize),
                            log: config.log2_size,
                            alpha,
                            residual,
                        });
                    } else {
                        if let Some(residual) = residual {
                            residual.reconstruct(
                                self.pps
                                    .scaling_lists
                                    .as_ref()
                                    .unwrap_or(&self.sps.scaling_lists),
                                &mut self.transform_scratch,
                                &mut self.residual_scratch,
                            )?;
                        } else {
                            self.residual_scratch.clear();
                        }
                        cross_residual(
                            c,
                            alpha,
                            config.log2_size,
                            self.sps.depth,
                            self.pps.cross_component_prediction,
                            &mut luma_residual,
                            &mut self.residual_scratch,
                        )?;
                        self.planes[c].add_residual(
                            origin.map(|v| v as usize),
                            config.log2_size,
                            &self.residual_scratch,
                        )?;
                    }
                }
                if act {
                    self.finish_act(u.origin, u.log2_size, bypass, act_blocks, act_alphas)?;
                }
                Ok(())
            },
        )
    }
    fn component_qps(&self, act: bool) -> Result<[u8; 3]> {
        if !act {
            return hevc_qp::components_with_format(
                self.qp,
                self.sps.depth,
                self.chroma_offsets,
                self.cu_chroma_offsets,
                self.sps.chroma_format,
            );
        }
        let offsets = self.slice.act_qp_offsets.map(i32::from);
        let mut qps = hevc_qp::components_with_format(
            self.qp,
            self.sps.depth,
            [offsets[1], offsets[2]],
            self.cu_chroma_offsets,
            3,
        )?;
        qps[0] = (self.qp + 6 * i32::from(self.sps.depth[0] - 8) + offsets[0])
            .clamp(0, 51 + 6 * i32::from(self.sps.depth[0] - 8)) as u8;
        Ok(qps)
    }
    fn finish_act(
        &mut self,
        origin: [u32; 2],
        log: u8,
        bypass: bool,
        blocks: [Option<hevc_block::Coefficients>; 3],
        alphas: [i8; 3],
    ) -> Result<()> {
        let origin = origin.map(|v| v as usize);
        if self.jobs.is_some() {
            self.reconstruction.push(Reconstruction::Act {
                origin,
                log,
                bypass,
                blocks,
                alphas,
            });
            Ok(())
        } else {
            reconstruct_act(
                &mut self.planes,
                origin,
                log,
                bypass,
                blocks,
                alphas,
                self.sps,
                self.pps,
                &mut self.transform_scratch,
                &mut self.residual_scratch,
            )
        }
    }
}

/// Canonical slice references retain both layer and POC identity.
pub(crate) fn canonical_references(
    slice_lists: &[[Vec<Reference>; 2]],
) -> Result<[Vec<Reference>; 2]> {
    let mut canonical: [Vec<Reference>; 2] = [Vec::new(), Vec::new()];
    for lists in slice_lists {
        for list in 0..2 {
            for reference in &lists[list] {
                if reference.layer_id > 62 {
                    return Err(invalid("HEVC reference layer out of range"));
                }
                if let Some(existing) = canonical
                    .iter()
                    .flatten()
                    .find(|r| r.identity() == reference.identity())
                {
                    if existing.long_term != reference.long_term {
                        return Err(invalid(
                            "HEVC reference identity has conflicting classification",
                        ));
                    }
                }
                if !canonical[list]
                    .iter()
                    .any(|r| r.identity() == reference.identity())
                {
                    if canonical[list].len() == 256 {
                        return Err(invalid("HEVC picture reference map exceeds index range"));
                    }
                    canonical[list].push(reference.clone());
                }
            }
        }
    }
    Ok(canonical)
}

pub(crate) fn motion_boundary(a: Motion, b: Motion, lists: &[Vec<Reference>; 2]) -> bool {
    let convert = |m: Motion| {
        let mut output = [(super::hevc_reference_list::PictureIdentity {layer_id:0,poc:0}, [0; 2]); 2];
        let mut count = 0;
        for (l, v) in m.into_iter().enumerate() {
            if let Some(v) = v {
                output[count] = (lists[l][v.reference as usize].identity(), v.mv);
                count += 1;
            }
        }
        (output, count)
    };
    let (a, na) = convert(a);
    let (b, nb) = convert(b);
    if na != nb {
        return true;
    }
    let same = |a: (super::hevc_reference_list::PictureIdentity, [i64; 2]), b: (super::hevc_reference_list::PictureIdentity, [i64; 2])| {
        a.0 == b.0 && (0..2).all(|i| (i128::from(a.1[i]) - i128::from(b.1[i])).abs() < 4)
    };
    match na {
        0 => false,
        1 => !same(a[0], b[0]),
        _ => !(same(a[0], b[0]) && same(a[1], b[1]) || same(a[0], b[1]) && same(a[1], b[0])),
    }
}

fn reconstruct_act(
    planes: &mut [Plane; 3],
    origin: [usize; 2],
    log: u8,
    bypass: bool,
    blocks: [Option<hevc_block::Coefficients>; 3],
    alphas: [i8; 3],
    sps: &Sps,
    pps: &Pps,
    transform_scratch: &mut Vec<i32>,
    residual_scratch: &mut Vec<i32>,
) -> Result<()> {
    let mut samples = std::array::from_fn(|_| vec![0; 1usize << (2 * log)]);
    for (c, block) in blocks.into_iter().enumerate() {
        if let Some(block) = block {
            block.reconstruct(
                pps.scaling_lists.as_ref().unwrap_or(&sps.scaling_lists),
                transform_scratch,
                residual_scratch,
            )?;
            samples[c].copy_from_slice(residual_scratch);
        }
        if c != 0 && alphas[c] != 0 {
            let (y, chroma) = samples.split_at_mut(c);
            super::hevc_cross_component::modify(&y[0], &mut chroma[0], alphas[c], sps.depth)?;
        }
    }
    super::hevc_act::inverse(&mut samples, sps.depth, sps.extended_precision, bypass)?;
    for c in 0..3 {
        planes[c].add_residual(origin, log, &samples[c])?;
    }
    Ok(())
}

#[path = "hevc_view_component.rs"]
mod view_component;
pub use view_component::{select_current_view_component,ViewComponentPool};

#[path = "hevc_picture_layered.rs"]
mod layered;
pub use layered::{decode_layered,decode_layered_with_texture,decode_layered_with_components,decode_layered_with_refinement,decode_layered_with_sources,select_texture_picture,derive_texture_subblocks};

#[cfg(test)]
mod queued_dim_tests {
    #[test]
    fn actual_worker_queue_executes_all_dim_blocks_and_direct_control_executes_none() {
        use crate::codec::hevc_decoder::{LayeredAccessUnit,LayeredParameters,LayeredDpb};
        let root=std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/fixtures/playback-errors/hevc-wedge");
        for family in ["queued-dim","queued-dim-full","queued-dim-dc"] {
        for residual in if family.ends_with("-dc"){&["zero"][..]}else{&["zero","mixed"][..]} {
            for dlt in [false,true] {
                let stem=format!("{family}-{residual}{}",if dlt{"-dlt"}else{""});
                let expected:Vec<u16>=std::fs::read(root.join(format!("{stem}.y16"))).unwrap()
                    .chunks_exact(2).map(|b|u16::from_le_bytes([b[0],b[1]])).collect();
                for direct in [false,true] {
                    let movie=format!("{stem}{}.mp4",if direct{"-direct"}else{""});
                    let mut reader=crate::container::mp4::Mp4Reader::open(
                        std::io::Cursor::new(std::fs::read(root.join(&movie)).unwrap()),Default::default()).unwrap();
                    let params=LayeredParameters::from_configuration(&reader.tracks()[0].configuration,64<<20).unwrap();
                    let mut packet=Vec::new();reader.read_packet(0,0,&mut packet).unwrap();
                    let mut dpb=LayeredDpb::default();
                    let plan=LayeredAccessUnit::parse(&params,dpb.poc(),&packet,4,1,64<<20).unwrap();
                    let (_,frames,_)=dpb.decode(plan,64<<20).unwrap();
                    let depth=frames.iter().find(|f|f.layer_id==3).unwrap();
                    assert_eq!(depth.picture.dimensions,[128,96]);
                    assert_eq!(depth.picture.queued_dim_blocks,if direct{0}else if family=="queued-dim"{48}else{12},"{movie}");
                    assert_eq!(depth.picture.planes[0].samples(),expected,"{movie}");
                }
            }
        }
        }
    }
}

#[cfg(test)]
mod inter_dc_tests {
    #[test]
    fn inter_dc_worker_commands_follow_temporal_prediction_and_controls_enqueue_none() {
        use crate::codec::hevc_decoder::{LayeredAccessUnit,LayeredParameters,LayeredDpb};
        let root=std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/fixtures/playback-errors/hevc-wedge");
        for mode in ["merge","explicit","skip","disabled","partitioned","merge-residual"] {
            let active=matches!(mode,"merge"|"explicit");
            let label=if active{"clip"}else{"identity"};
            let stem=format!("inter-dc-128-{mode}-{label}");
            let expected:Vec<u16>=std::fs::read(root.join(format!("{stem}.y16"))).unwrap()
                .chunks_exact(2).map(|b|u16::from_le_bytes([b[0],b[1]])).collect();
            for dlt in [false,true] {
                for direct in [false,true] {
                    let movie=format!("{stem}{}{}.mp4",if dlt{"-dlt"}else{""},if direct{"-direct"}else{""});
                    let mut reader=crate::container::mp4::Mp4Reader::open(
                        std::io::Cursor::new(std::fs::read(root.join(&movie)).unwrap()),Default::default()).unwrap();
                    let mut params=LayeredParameters::from_configuration(&reader.tracks()[0].configuration,64<<20).unwrap();
                    let mut packet=Vec::new();let mut dpb=LayeredDpb::default();
                    for index in 0..3 {
                        reader.read_packet(0,index,&mut packet).unwrap();
                        let plan=LayeredAccessUnit::parse(&params,dpb.poc(),&packet,4,1,64<<20).unwrap();
                        let (updated,frames,_)=dpb.decode(plan,64<<20).unwrap();params=updated;
                        let frame=frames.iter().find(|f|f.layer_id==3).unwrap();
                        assert_eq!(frame.poc,index as i32,"{movie}");
                        assert_eq!(frame.picture.queued_inter_dc_blocks,if active && !direct && index>0{4}else{0},"{movie}");
                        assert_eq!(frame.picture.planes[0].samples(),&expected[index*128*128..(index+1)*128*128],"{movie}");
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod cqt_tests {
    #[test]
    fn cqt_requires_current_coded_texture_only_for_enabled_non_irap_inter_slices() {
        use super::*;
        use crate::codec::hevc_decoder::{LayeredParameters,LayeredAccessUnit,LayeredDpb};
        let root=std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/playback-errors/hevc-wedge");
        let mut movie=crate::container::mp4::Mp4Reader::open(
            std::io::Cursor::new(std::fs::read(root.join("cqt-64-ctu64-full-on-match.mp4")).unwrap()),Default::default()).unwrap();
        let mut params=LayeredParameters::from_configuration(&movie.tracks()[0].configuration,64<<20).unwrap();
        let mut packet=Vec::new();let mut dpb=LayeredDpb::default();
        movie.read_packet(0,0,&mut packet).unwrap();
        let first=LayeredAccessUnit::parse(&params,dpb.poc(),&packet,4,1,64<<20).unwrap();
        let depth=first.pictures().iter().find(|p|p.binding.sps.layer_id==3).unwrap();
        assert!(!cqt_enabled(&depth.binding.sps.sps,&depth.slices[0]));
        assert!(select_texture_picture(&depth.binding.context(),&depth.slices[0],0,0,&[]).unwrap().is_none());
        let intra=decode_layered(&depth.binding.context(),&depth.slices,0,&[[Vec::new(),Vec::new()]],64<<20).unwrap();
        assert_eq!(intra.coding_unit_at([0,0]).unwrap(),(5,Partition::Full));
        let (updated,frames,_)=dpb.decode(first,64<<20).unwrap();params=updated;
        let reference=frames.iter().find(|f|f.layer_id==3).unwrap().picture.clone();
        movie.read_packet(0,1,&mut packet).unwrap();
        let next=LayeredAccessUnit::parse(&params,dpb.poc(),&packet,4,1,64<<20).unwrap();
        let depth=next.pictures().iter().find(|p|p.binding.sps.layer_id==3).unwrap();
        let binding=depth.binding.clone();let slices=depth.slices.clone();let poc=depth.poc;
        assert!(cqt_enabled(&binding.sps.sps,&slices[0]));
        let lists=[[vec![Reference {layer_id:3,poc:0,long_term:false,picture:Some(reference)}],Vec::new()]];
        assert_eq!(decode_layered(&binding.context(),&slices,poc,&lists,64<<20).err().unwrap().to_string(),
            "HEVC coding-tree prediction requires current-AU TexturePic");
        let (_,frames,_)=dpb.decode(next,64<<20).unwrap();
        let texture=&frames.iter().find(|f|f.layer_id==0).unwrap().picture;
        let translated=texture.translated_pocs(5,64<<20).unwrap();
        assert!(texture.coding.as_ref().unwrap().shares_storage(translated.coding.as_ref().unwrap()));
        assert_eq!(texture.prediction_motion_at([8,8]).unwrap(),translated.prediction_motion_at([8,8]).unwrap());
        let refs=[hevc_motion::texture::Identity {poc:-5,view:2}];
        assert_eq!(translated.texture_merge_candidate_at([8,8],[&refs,&[]],false,|_|Ok(2)).unwrap(),
            [Some(hevc_motion::texture::Vector {reference:0,samples:[0;2]}),None]);
        let expected=&frames.iter().find(|f|f.layer_id==3).unwrap().picture;
        let decoded=decode_layered_with_texture(&binding.context(),&slices,poc,&lists,64<<20,Some(texture)).unwrap();
        assert_eq!(decoded.planes[0].samples(),expected.planes[0].samples());
        let mut incomplete=texture.translated_pocs(0,64<<20).unwrap();incomplete.coding=None;
        assert_eq!(decode_layered_with_texture(&binding.context(),&slices,poc,&lists,64<<20,Some(&incomplete)).err().unwrap().to_string(),
            "HEVC TexturePic has no coded-CU map");
        let mut off=slices[0].clone();off.layered.as_mut().unwrap().in_component_prediction=Some(false);
        assert!(!cqt_enabled(&binding.sps.sps,&off));
        for kind in 16..=23 {let mut irap=slices[0].clone();irap.nal.unit_type=kind;assert!(!cqt_enabled(&binding.sps.sps,&irap));}
    }
}

#[cfg(test)]
mod texture_owner_tests {
    #[test]
    fn active_texture_subblocks_execute_on_worker_and_direct_controls_do_not_enqueue() {
        use crate::codec::hevc_decoder::{LayeredAccessUnit,LayeredDpb,LayeredParameters};
        let root=std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/fixtures/playback-errors/hevc-wedge");
        for enabled in [false,true] {for direct in [false,true] {
            let name=format!("texture-motion-128-{}{}.mp4",if enabled{"on"}else{"off"},if direct{"-direct"}else{""});
            let mut movie=crate::container::mp4::Mp4Reader::open(
                std::io::Cursor::new(std::fs::read(root.join(&name)).unwrap()),Default::default()).unwrap();
            let mut params=LayeredParameters::from_configuration(&movie.tracks()[0].configuration,64<<20).unwrap();
            let mut dpb=LayeredDpb::default();let mut packet=Vec::new();
            let mut previous:Vec<u16>=(0..128).flat_map(|y|(0..128).map(move|x|((x*7+y*11+37)%256)as u16)).collect();
            for index in 0..3 {
                movie.read_packet(0,index,&mut packet).unwrap();
                let plan=LayeredAccessUnit::parse(&params,dpb.poc(),&packet,4,1,64<<20).unwrap();
                let (updated,frames,_)=dpb.decode(plan,64<<20).unwrap();params=updated;
                let picture=&frames.iter().find(|f|f.layer_id==3).unwrap().picture;
                assert_eq!(picture.queued_texture_blocks,if enabled&&!direct&&index>0{256}else{0},"{name}: POC{index}");
                let mut expected=Vec::with_capacity(128*128);
                for y in 0..128 {for x in 0..128 {
                    let shift=if enabled&&index>0{if y%32<8{1}else{-1}}else{0};
                    let yy=(y as i32+shift).clamp(0,127) as usize;
                    expected.push(previous[yy*128+x]);
                    assert_eq!(picture.planes[0].samples()[y*128+x],previous[yy*128+x],"{name}");
                }}
                previous=expected;
            }
        }}
    }
}

#[cfg(test)]
mod multicu_wide_owner_tests {
    #[test]
    fn wide_di_reconstruction_runs_on_worker_and_direct_controls_enqueue_nothing() {
        use crate::codec::hevc_decoder::{LayeredAccessUnit,LayeredDpb,LayeredParameters};
        let root=std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/playback-errors/hevc-wedge");
        for size in [64usize,128] {for direct in [false,true] {for offset in [0i64,2,-2,2147483647,-2147483647] {
            let name=format!("multicu-wide-{size}-{}-{offset}.mp4",usize::from(direct));
            let mut movie=crate::container::mp4::Mp4Reader::open(std::io::Cursor::new(std::fs::read(root.join(&name)).unwrap()),Default::default()).unwrap();
            let mut params=LayeredParameters::from_configuration(&movie.tracks()[0].configuration,64<<20).unwrap();
            let mut dpb=LayeredDpb::default();let mut packet=Vec::new();
            for index in 0..2 {
                movie.read_packet(0,index,&mut packet).unwrap();
                let plan=LayeredAccessUnit::parse(&params,dpb.poc(),&packet,4,1,64<<20).unwrap();
                let (updated,frames,_)=dpb.decode(plan,64<<20).unwrap();params=updated;
                let picture=&frames.iter().find(|p|p.layer_id==5).unwrap().picture;
                let queued=if index==1 && size*size>=128*96 && !direct {(size/32).pow(2)}else{0};
                assert_eq!(picture.queued_inter_blocks,queued,"{name} POC{index}");
                for y in 0..size {for x in 0..size {
                    let xx=if index==0 {x as i64}else{(x as i64+offset*2/4).clamp(0,size as i64-1)};
                    assert_eq!(picture.planes[0].samples()[y*size+x],((3*xx+5*y as i64+17)%256)as u16,"{name}: x{x} y{y}");
                }}
            }
        }}}
    }
}

#[cfg(test)]
mod multicu_variable_refinement_owner_tests {
    #[test]
    fn variable_refinement_matches_scalar_pixels_after_worker_join_and_direct_reconstruction() {
        use crate::codec::hevc_decoder::{LayeredAccessUnit,LayeredDpb,LayeredParameters};
        let root=std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/playback-errors/hevc-wedge");
        for colored in [false,true] {
        let prefix=if colored {"multicu-refinement-colored"}else{"multicu-refinement"};
        let golden:Vec<u16>=std::fs::read(root.join(format!("{prefix}.y16"))).unwrap().chunks_exact(2).map(|b|u16::from_le_bytes([b[0],b[1]])).collect();
        let mut at=0;
        for size in [64usize,128] {for direct in [false,true] {for (scale,offset) in [(128,0),(-128,0),(4,0),(-4,0),(16,-3),(-16,3),(1,0),(-1,0)] {
            let name=format!("{prefix}-{size}-{}-{scale}-{offset}.mp4",usize::from(direct));
            let mut movie=crate::container::mp4::Mp4Reader::open(std::io::Cursor::new(std::fs::read(root.join(&name)).unwrap()),Default::default()).unwrap();
            let mut params=LayeredParameters::from_configuration(&movie.tracks()[0].configuration,64<<20).unwrap();let mut dpb=LayeredDpb::default();let mut packet=Vec::new();
            for index in 0..2 {
                movie.read_packet(0,index,&mut packet).unwrap();
                let plan=LayeredAccessUnit::parse(&params,dpb.poc(),&packet,4,1,64<<20).unwrap();
                let (updated,frames,_)=dpb.decode(plan,64<<20).unwrap();params=updated;
                let picture=&frames.iter().find(|p|p.layer_id==5).unwrap().picture;
                assert_eq!(picture.queued_inter_blocks,if index==1 && size*size>=128*96 && !direct {(size/32).pow(2)}else{0},"{name}: worker ownership");
                if index==1 {
                    let mut position=at;
                    for component in 0..if colored {3}else{1} {
                        let n=if component==0 {size}else{size/2};
                        assert_eq!(picture.planes[component].samples(),&golden[position..position+n*n],"{name}: completed worker/direct c{component}");
                        position+=n*n;
                    }
                }
            }
            at+=if colored {size*size*3/2}else{size*size};
        }}}
        assert_eq!(at,golden.len());
        }
    }
}

#[cfg(test)]
mod multicu_iv_owner_tests {
    #[test]
    fn multi_cu_iv_subblocks_execute_on_worker_and_keep_exact_source_target_color() {
        use crate::codec::hevc_decoder::{LayeredAccessUnit,LayeredDpb,LayeredParameters};
        let root=std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/playback-errors/hevc-wedge");
        let golden:Vec<u16>=std::fs::read(root.join("multicu-iv.y16")).unwrap().chunks_exact(2).map(|b|u16::from_le_bytes([b[0],b[1]])).collect();let mut at=0;
        for size in [64usize,128] {for direct in [false,true] {for cut in [8,24] {for (scale,offset) in [(128,0),(-128,0),(4,0),(-4,0),(16,-3),(-16,3)] {
            let name=format!("multicu-iv-{size}-{}-{cut}-{scale}-{offset}.mp4",usize::from(direct));
            let mut movie=crate::container::mp4::Mp4Reader::open(std::io::Cursor::new(std::fs::read(root.join(&name)).unwrap()),Default::default()).unwrap();
            let mut params=LayeredParameters::from_configuration(&movie.tracks()[0].configuration,64<<20).unwrap();let mut dpb=LayeredDpb::default();let mut packet=Vec::new();
            for index in 0..2 {
                movie.read_packet(0,index,&mut packet).unwrap();let plan=LayeredAccessUnit::parse(&params,dpb.poc(),&packet,4,1,64<<20).unwrap();let (updated,frames,_)=dpb.decode(plan,64<<20).unwrap();params=updated;
                for (owner,id)in [0,5].into_iter().enumerate() {
                    let picture=&frames.iter().find(|p|p.layer_id==id).unwrap().picture;
                    let queued=if index==1 && size*size>=128*96 && !direct {(size/32).pow(2)*if id==0 {2}else{16}}else{0};
                    assert_eq!(picture.queued_inter_blocks,queued,"{name}: layer{id} worker ownership");
                    if index==1 {
                        let mut position=at+owner*size*size*3/2;
                        for component in 0..3 {let n=if component==0 {size}else{size/2};assert_eq!(picture.planes[component].samples(),&golden[position..position+n*n],"{name}: completed c{component} layer{id}");position+=n*n;}
                    }
                }
            }
            at+=size*size*3;
        }}}}
        assert_eq!(at,golden.len());
    }
}

#[cfg(test)]
mod native_vsp_owner_tests {
    #[test]
    fn vsp_subblocks_complete_on_worker_and_direct_paths_with_exact_pixels() {
        use crate::codec::hevc_decoder::{LayeredAccessUnit, LayeredDpb, LayeredParameters};
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/fixtures/playback-errors/hevc-wedge");
        let golden: Vec<u16> = std::fs::read(root.join("native-vsp.y16")).unwrap()
            .chunks_exact(2).map(|b| u16::from_le_bytes([b[0], b[1]])).collect();
        let mut at = 0;
        for size in [32usize, 64, 128] {
            for direct in [false, true] {
                for iv in [false, true] {
                    for refine in [false, true] {
                        for (scale, offset) in [(4,0),(-4,0),(128,0),(-128,0),(0,0),(0,2)] {
                            let name = format!("native-vsp-{size}-{}-{}-{}-{scale}-{offset}.mp4",
                                usize::from(direct), usize::from(iv), usize::from(refine));
                            let mut movie = crate::container::mp4::Mp4Reader::open(
                                std::io::Cursor::new(std::fs::read(root.join(&name)).unwrap()), Default::default()).unwrap();
                            let mut params = LayeredParameters::from_configuration(&movie.tracks()[0].configuration, 64<<20).unwrap();
                            let mut dpb = LayeredDpb::default();
                            let mut packet = Vec::new();
                            for index in 0..2 {
                                movie.read_packet(0,index,&mut packet).unwrap();
                                let plan = LayeredAccessUnit::parse(&params,dpb.poc(),&packet,4,1,64<<20).unwrap();
                                let (updated,frames,_) = dpb.decode(plan,64<<20).unwrap();
                                params = updated;
                                let picture = &frames.iter().find(|f|f.layer_id==5).unwrap().picture;
                                let queued = if index==1 && size*size>=128*96 && !direct {
                                    (size/32).pow(2)*32
                                } else {0};
                                assert_eq!(picture.queued_inter_blocks,queued,"{name}: ownership POC{index}");
                                if index==1 {
                                    let mut position=at;
                                    for c in 0..3 {
                                        let n=if c==0 {size}else{size/2};
                                        assert_eq!(picture.planes[c].samples(),&golden[position..position+n*n],"{name}: completed plane{c}");
                                        position+=n*n;
                                    }
                                }
                            }
                            at+=size*size*3/2;
                        }
                    }
                }
            }
        }
        assert_eq!(at,golden.len());
    }
}

#[cfg(test)]
mod illumination_gain_owner_tests {
    #[test]
    fn illumination_reads_preceding_reconstruction_on_worker_and_direct_paths() {
        use crate::codec::hevc_decoder::{LayeredAccessUnit,LayeredDpb,LayeredParameters};
        let root=std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/playback-errors/hevc-wedge");
        let golden:Vec<u16>=std::fs::read(root.join("illumination-gain.y16")).unwrap().chunks_exact(2).map(|b|u16::from_le_bytes([b[0],b[1]])).collect();
        let mut at=0;
        for size in [64usize,128] {for direct in [false,true] {for enabled in [false,true] {for (num,den,bias) in [(2,1,-40),(1,2,30),(0,1,200)] {
            let name=format!("illumination-gain-{size}-{}-{}-{num}-{den}-{bias}.mp4",usize::from(direct),usize::from(enabled));
            let mut movie=crate::container::mp4::Mp4Reader::open(std::io::Cursor::new(std::fs::read(root.join(&name)).unwrap()),Default::default()).unwrap();
            let mut params=LayeredParameters::from_configuration(&movie.tracks()[0].configuration,64<<20).unwrap();
            let mut dpb=LayeredDpb::default();let mut packet=Vec::new();
            for index in 0..2 {
                movie.read_packet(0,index,&mut packet).unwrap();
                let plan=LayeredAccessUnit::parse(&params,dpb.poc(),&packet,4,1,64<<20).unwrap();
                let (updated,frames,_)=dpb.decode(plan,64<<20).unwrap();params=updated;
                let picture=&frames.iter().find(|f|f.layer_id==5).unwrap().picture;
                assert_eq!(picture.queued_inter_blocks,if index==1&&size==128&&!direct {16}else{0},"{name}: actual worker ownership");
                if index==1 {
                    let mut position=at;
                    for c in 0..3 {
                        let n=if c==0 {size}else{size/2};
                        assert_eq!(picture.planes[c].samples(),&golden[position..position+n*n],"{name}: completed c{c}");
                        position+=n*n;
                    }
                }
            }
            at+=size*size*3/2;
        }}}}
        assert_eq!(at,golden.len());
    }
}

#[cfg(test)]
mod illumination_b_owner_tests {
    #[test]
    fn mixed_b_list_illumination_executes_on_worker_and_direct_paths() {
        use crate::codec::hevc_decoder::{LayeredAccessUnit,LayeredDpb,LayeredParameters};
        let root=std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/playback-errors/hevc-wedge");
        let golden:Vec<u16>=std::fs::read(root.join("illumination-b.y16")).unwrap().chunks_exact(2).map(|b|u16::from_le_bytes([b[0],b[1]])).collect();let mut at=0;
        for size in [64usize,128] {for direct in [false,true] {for enabled in [false,true] {for (num,den,bias) in [(2,1,-40),(1,2,30)] {for mode in ["both","past-source","source-future","l0","l1","temporal"] {
            let name=format!("illumination-b-{size}-{}-{}-{num}-{den}-{bias}-{mode}.mp4",usize::from(direct),usize::from(enabled));
            let mut movie=crate::container::mp4::Mp4Reader::open(std::io::Cursor::new(std::fs::read(root.join(&name)).unwrap()),Default::default()).unwrap();
            let mut params=LayeredParameters::from_configuration(&movie.tracks()[0].configuration,64<<20).unwrap();let mut dpb=LayeredDpb::default();let mut packet=Vec::new();
            for index in 0..3 {
                movie.read_packet(0,index,&mut packet).unwrap();
                let plan=LayeredAccessUnit::parse(&params,dpb.poc(),&packet,4,1,64<<20).unwrap();
                let (updated,frames,_)=dpb.decode(plan,64<<20).unwrap();params=updated;
                let picture=&frames.iter().find(|f|f.layer_id==5).unwrap().picture;
                assert_eq!(picture.queued_inter_blocks,if index==2&&size==128&&!direct {16}else{0},"{name}: worker ownership");
                if index==2 {
                    let mut position=at;
                    for c in 0..3 {
                        let n=if c==0 {size}else{size/2};
                        assert_eq!(picture.planes[c].samples(),&golden[position..position+n*n],"{name}: completed c{c}");position+=n*n;
                    }
                }
            }
            at+=size*size*3/2;
        }}}}}
        assert_eq!(at,golden.len());
    }
}

#[cfg(test)]
mod illumination_vsp_owner_tests {
    #[test]
    fn mixed_vsp_ic_commands_keep_boundary_dependencies_on_worker_and_direct_paths() {
        use crate::codec::hevc_decoder::{LayeredAccessUnit,LayeredDpb,LayeredParameters};
        let root=std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/playback-errors/hevc-wedge");
        let golden:Vec<u16>=std::fs::read(root.join("illumination-vsp.y16")).unwrap().chunks_exact(2).map(|b|u16::from_le_bytes([b[0],b[1]])).collect();let mut at=0;
        for size in [64usize,128] {for direct in [false,true] {for refine in [false,true] {for scale in [4,-4,128,-128] {
            let name=format!("illumination-vsp-{size}-{}-{}-{scale}.mp4",usize::from(direct),usize::from(refine));
            let mut movie=crate::container::mp4::Mp4Reader::open(std::io::Cursor::new(std::fs::read(root.join(&name)).unwrap()),Default::default()).unwrap();
            let mut params=LayeredParameters::from_configuration(&movie.tracks()[0].configuration,64<<20).unwrap();let mut dpb=LayeredDpb::default();let mut packet=Vec::new();
            for index in 0..2 {
                movie.read_packet(0,index,&mut packet).unwrap();
                let plan=LayeredAccessUnit::parse(&params,dpb.poc(),&packet,4,1,64<<20).unwrap();
                let (updated,frames,_)=dpb.decode(plan,64<<20).unwrap();params=updated;
                let picture=&frames.iter().find(|f|f.layer_id==5).unwrap().picture;
                assert_eq!(picture.queued_inter_blocks,if index==1&&size==128&&!direct {481}else{0},"{name}: worker ownership");
                if index==1 {
                    let mut position=at;
                    for c in 0..3 {
                        let n=if c==0 {size}else{size/2};
                        assert_eq!(picture.planes[c].samples(),&golden[position..position+n*n],"{name}: completed c{c}");position+=n*n;
                    }
                }
            }
            at+=size*size*3/2;
        }}}}
        assert_eq!(at,golden.len());
    }
}

#[cfg(test)]
mod residual_prediction_owner_tests {
    fn check(name:&str,size:usize,direct:bool,packets:usize) {
        use crate::codec::hevc_decoder::{LayeredAccessUnit,LayeredDpb,LayeredParameters};
        let root=std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/playback-errors/hevc-wedge");
        let mut movie=crate::container::mp4::Mp4Reader::open(std::io::Cursor::new(std::fs::read(root.join(name)).unwrap()),Default::default()).unwrap();
        let mut parameters=LayeredParameters::from_configuration(&movie.tracks()[0].configuration,64<<20).unwrap();
        let mut dpb=LayeredDpb::default();let mut packet=Vec::new();
        for index in 0..packets {
            movie.read_packet(0,index,&mut packet).unwrap();
            let plan=LayeredAccessUnit::parse(&parameters,dpb.poc(),&packet,4,1,64<<20).unwrap();
            let (updated,frames,_)=dpb.decode(plan,64<<20).unwrap();parameters=updated;
            if name.starts_with("residual-b-reference-motion") || name.starts_with("dbbp-b-iv-128-") || name.starts_with("dbbp-b-iv-l1-128-") {
                let source=frames.iter().find(|f|f.layer_id==0).unwrap();
                assert_eq!(source.picture.queued_inter_blocks,
                    if source.poc==1&&size==128&&!direct {2*(size/32).pow(2)}else{0},"{name}: source worker ownership");
            }
            let decoded=frames.iter().find(|f|f.layer_id==5).unwrap();let target=&decoded.picture;
            let commands=if name.starts_with("dbbp-base-refined-128-") {if name.split('-').nth(6)==Some("0"){32}else{16}}
                else if name.starts_with("dbbp-base-128-") {if name.split('-').nth(5)==Some("0"){32}else{16}}
                else if name.starts_with("dbbp-disparity-") {if name.split('-').nth(6)==Some("0"){32}else{16}}
                else if name.starts_with("dbbp-b-iv-l1-128-") {if name.split('-').nth(7)==Some("0"){144}else{16}}
                else if name.starts_with("dbbp-b-iv-128-") {if name.split('-').nth(6)==Some("0"){144}else{16}}
                else if name.starts_with("dbbp-b-128-") {if name.split('-').nth(5)==Some("0"){32}else{16}}
                else if name.starts_with("dbbp-cu16-") {if name.ends_with("-0.mp4"){128}else{64}}
                else if name.starts_with("dbbp-cu64-") {if name.ends_with("-0.mp4"){8}else{4}}
                else if name.starts_with("dbbp-") && name.ends_with("-0.mp4") {32}else{16};
            assert_eq!(target.queued_inter_blocks,if decoded.poc==1&&size==128&&!direct {commands}else{0},"{name}: actual worker ownership");
        }
    }
    #[test]
    fn dbbp_commands_execute_on_worker_and_direct_paths() {
        for direct in [false,true] {for vertical in [0,1] {for enabled in [0,1] {
            let name=format!("dbbp-p-128-{vertical}-{enabled}{}.mp4",if direct {"-direct"}else{""});
            check(&name,128,direct,2);
        }}}
    }
    #[test]
    fn dbbp_cu64_commands_execute_on_worker_and_direct_paths() {
        for direct in [false,true] {for vertical in [0,1] {for enabled in [0,1] {
            let name=format!("dbbp-cu64-128-{}-{vertical}-{enabled}.mp4",usize::from(direct));
            check(&name,128,direct,2);
        }}}
    }
    #[test]
    fn dbbp_cu16_commands_execute_on_worker_and_direct_paths() {
        for direct in [false,true] {for vertical in [0,1] {for enabled in [0,1] {
            let name=format!("dbbp-cu16-128-{}-{vertical}-{enabled}.mp4",usize::from(direct));
            check(&name,128,direct,2);
        }}}
    }
    #[test]
    fn dbbp_b_commands_execute_on_worker_and_direct_paths() {
        for direct in [false,true] {for vertical in [0,1] {for enabled in [0,1] {for weighted in [false,true] {
            let name=format!("dbbp-b-128-{}-{vertical}-{enabled}-2{}.mp4",usize::from(direct),if weighted {"-weighted"}else{""});
            check(&name,128,direct,3);
        }}}}
    }
    #[test]
    fn dbbp_b_iv_commands_execute_on_worker_and_direct_paths() {
        for direct in [false,true] {for vertical in [0,1] {for enabled in [0,1] {for weighted in [0,1] {
            let name=format!("dbbp-b-iv-128-{}-{vertical}-{enabled}-{weighted}-24-1--3.mp4",usize::from(direct));
            check(&name,128,direct,3);
        }}}}
    }
    #[test]
    fn dbbp_b_iv_l1_commands_execute_on_worker_and_direct_paths() {
        for direct in [false,true] {for vertical in [0,1] {for enabled in [0,1] {for weighted in [0,1] {
            let name=format!("dbbp-b-iv-l1-128-{}-{vertical}-{enabled}-{weighted}-24-1--3.mp4",usize::from(direct));
            check(&name,128,direct,3);
        }}}}
    }
    #[test]
    fn dbbp_refined_disparity_commands_execute_on_worker_and_direct_paths() {
        for direct in [false,true] {for vertical in [0,1] {for enabled in [0,1] {
            for (scale,offset,precision)in [(4,0,0),(-4,0,0),(128,0,0),(-128,0,0),(1,0,0),(-1,0,0),(16,-3,1),(-16,3,1)] {
                let name=format!("dbbp-disparity-32-128-{}-{vertical}-{enabled}-{scale}-{offset}-{precision}.mp4",usize::from(direct));
                check(&name,128,direct,2);
            }
        }}}
    }
    #[test]
    fn dbbp_spatial_base_commands_execute_on_worker_and_direct_paths() {
        for direct in [false,true] {for vertical in [0,1] {for enabled in [0,1] {
            for (mx,my)in [(1,3),(-7,-5),(9,-11),(-64,20),(128,-256),(-128,256)] {
                let name=format!("dbbp-base-128-{}-{vertical}-{enabled}-{mx}-{my}.mp4",usize::from(direct));
                check(&name,128,direct,2);
            }
        }}}
    }
    #[test]
    fn dbbp_joint_base_refinement_commands_execute_on_worker_and_direct_paths() {
        for direct in [false,true] {for vertical in [0,1] {for enabled in [0,1] {
            for (mx,my)in [(1,3),(-7,-5),(9,-11)] {
                for (scale,offset,precision)in [(4,0,0),(-4,0,0),(128,0,0),(-128,0,0),(16,-3,1),(-16,3,1)] {
                    let name=format!("dbbp-base-refined-128-{}-{vertical}-{enabled}-{mx}-{my}-{scale}-{offset}-{precision}.mp4",usize::from(direct));
                    check(&name,128,direct,2);
                }
            }
        }}}
    }
    #[test]
    fn residual_commands_really_execute_on_worker_and_direct_paths() {
        for size in [64usize,128] {for direct in [false,true] {for weight in ["0","1","2","mixed"] {
          for ic in [false,true] {for bias in [20,-80,200] {
            let suffix=if bias==20 {String::new()}else{format!("-bias{bias}")};
            let name=format!("residual-p-{size}-{}-{weight}{}{suffix}.mp4",usize::from(direct),if ic {"-ic"}else{""});
            check(&name,size,direct,2);
        }}}}}
    }
    #[test]
    fn fractional_residual_commands_really_execute_on_worker_and_direct_paths() {
        for direct in [false,true] {for weight in [1,2] {for (mx,my)in [(-7,-5),(1,3),(0,-7),(7,5)] {for reference in [0,1] {
            let name=format!("residual-fractional-128-{}-{weight}-20-{mx}-{my}{}.mp4",usize::from(direct),if reference==0 {"-temporal"}else{""});
            check(&name,128,direct,2);
        }}}}
    }
    #[test]
    fn b_residual_and_explicit_weights_execute_on_worker_and_direct_paths() {
        for size in [64usize,128] {for direct in [false,true] {
          for mode in ["both","past-source","source-future","l0","l1","temporal","future-past"] {
            for weight in ["0","1","2","mixed"] {for weighted in [false,true] {
                let name=format!("residual-b-{size}-{}-{mode}-{weight}{}.mp4",usize::from(direct),if weighted {"-weighted"}else{""});
                check(&name,size,direct,3);
            }}
          }
        }}
    }
    #[test]
    fn fractional_b_temporal_scaling_executes_on_worker_and_direct_paths() {
        for size in [64usize,128] {for direct in [false,true] {for mode in ["temporal","future-past"] {
            for weight in [1,2] {for weighted in [false,true] {
                for [[x0,y0],[x1,y1]] in [[[1,3],[-3,-1]],[[-7,5],[5,-7]],[[2,-3],[-1,6]]] {
                    let name=format!("residual-b-temporal-fractional-{size}-{}-{mode}-{weight}-{}-{x0}-{y0}-{x1}-{y1}.mp4",usize::from(direct),usize::from(weighted));
                    check(&name,size,direct,3);
                }
            }}
        }}}
    }
    #[test]
    fn mixed_fractional_b_disparity_executes_on_worker_and_direct_paths() {
        for size in [64usize,128] {for direct in [false,true] {
            for mode in ["both","past-source","source-future","l0","l1"] {
                for weight in [1,2] {for weighted in [false,true] {
                    for [[x0,y0],[x1,y1]] in [[[1,3],[-3,-1]],[[-7,5],[5,-7]],[[2,-3],[-1,6]]] {
                        let name=format!("residual-b-mixed-fractional-{size}-{}-{mode}-{weight}-{}-{x0}-{y0}-{x1}-{y1}.mp4",usize::from(direct),usize::from(weighted));
                        check(&name,size,direct,3);
                    }
                }}
            }
        }}
    }
    #[test]
    fn coded_source_reference_motion_executes_on_worker_and_direct_paths() {
        for size in [64usize,128] {for direct in [false,true] {for cut in [8,24] {for mode in ["both","l0","l1"] {
            for weight in [1,2] {for weighted in [false,true] {
                for [[x0,y0],[x1,y1]]in [[[29,3],[-35,-1]],[[30,3],[-34,-1]],[[135,-3],[-135,5]]] {
                    let name=format!("residual-b-reference-motion-{size}-{}-{cut}-{mode}-{weight}-{}-{x0}-{y0}-{x1}-{y1}.mp4",usize::from(direct),usize::from(weighted));
                    check(&name,size,direct,3);
                }
            }}
        }}}}
    }
    #[test]
    fn distinct_neighbour_motion_executes_on_worker_and_direct_paths() {
        for size in [64usize,128] {for direct in [false,true] {for mode in ["past-source","source-future"] {
            for weight in [1,2] {for weighted in [false,true] {for swap in [0,1] {
                let name=format!("residual-b-variable-neighbour-{size}-{}-{mode}-{weight}-{}-{swap}.mp4",usize::from(direct),usize::from(weighted));
                check(&name,size,direct,3);
            }}}
        }}}
    }
}
