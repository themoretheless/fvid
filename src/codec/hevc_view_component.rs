//! Current-AU ViewCompLayerId binding for Annex I inter-view/depth tools.
use super::Picture;
use crate::codec::{
    hevc_decoder::LayeredBinding,
    hevc_inter_layer::PictureStatus,
    hevc_slice::{LayeredSliceContext, SliceHeader},
};
use crate::{Result, invalid};
use std::sync::Arc;

/// Resolve one explicitly requested view component from completed current-AU
/// pictures. The pinned source binding is required: monochrome pixels alone cannot
/// identify a depth layer, and a caller must not relabel a picture by role.
///
/// This is same-resolution Annex I source binding, not Annex F resampling or
/// tool availability derivation. It shares immutable planes without allocation.
pub fn select_current_view_component(
    context: &LayeredSliceContext<'_>,
    header: &SliceHeader,
    current: PictureStatus,
    source_binding: &LayeredBinding,
    reference_view: u16,
    depth: bool,
    completed: &[(PictureStatus, Arc<Picture>)],
) -> Result<Arc<Picture>> {
    context.validate_poc_header(header)?;
    if !std::ptr::eq(source_binding.vps.as_ref(), context.vps)
        && source_binding.vps.as_ref() != context.vps
    {
        return Err(invalid(
            "HEVC reference view component uses a different VPS snapshot",
        ));
    }
    let source_sps = &source_binding.sps;
    current.validate()?;
    if current.identity.layer_id != context.sps.layer_id || current.nal != header.nal {
        return Err(invalid(
            "HEVC view-component current identity disagrees with slice",
        ));
    }
    let graph = &context
        .vps
        .extension
        .as_ref()
        .ok_or_else(|| invalid("HEVC view component requires VPS graph"))?
        .graph;
    let current_depth = graph
        .depth_layer_ids()?
        .any(|id| id == context.sps.layer_id);
    if current_depth != context.sps.sps.depth_layer {
        return Err(invalid(
            "HEVC current view-component role disagrees with SPS",
        ));
    }
    let layer = graph
        .view_component_for_index(reference_view, depth)?
        .ok_or_else(|| invalid("HEVC reference view component is absent from VPS"))?;
    if source_sps.layer_id != layer
        || source_sps.sps.depth_layer != depth
        || source_sps.sps.vps_id != context.sps.sps.vps_id
        || source_sps.output_layer_set != context.sps.output_layer_set
    {
        return Err(invalid(
            "HEVC reference view-component SPS identity disagrees",
        ));
    }
    let sps = &source_sps.sps;
    if sps.dimensions != context.sps.sps.dimensions
        || sps.dimensions.contains(&0)
        || sps.depth[0] != context.sps.sps.depth[0]
        || sps.separate_colour_plane
        || sps.chroma_format > 3
        || depth && sps.chroma_format != 0
    {
        return Err(invalid(
            "HEVC reference view-component representation disagrees",
        ));
    }
    let mut matches = completed
        .iter()
        .filter(|(status, _)| status.identity.layer_id == layer);
    let (status, picture) = matches.next().ok_or_else(|| {
        invalid("HEVC reference view component is not completed in current access unit")
    })?;
    if matches.next().is_some() {
        return Err(invalid(
            "HEVC reference view component repeats a layer identity",
        ));
    }
    status.validate()?;
    if picture.nal_header()!=status.nal {
        return Err(invalid("HEVC reference view-component NAL disagrees with decoded picture"));
    }
    if status.access_unit != current.access_unit || status.identity.poc != current.identity.poc {
        return Err(invalid(
            "HEVC reference view-component access unit or POC disagrees",
        ));
    }
    if status.nal.temporal_id > source_sps.max_sub_layers_minus1 {
        return Err(invalid(
            "HEVC reference view component exceeds SPS temporal sublayers",
        ));
    }
    if picture.dimensions != sps.dimensions || picture.depth != sps.depth {
        return Err(invalid(
            "HEVC reference view-component picture disagrees with SPS",
        ));
    }
    for component in 0..3 {
        let plane = &picture.planes[component];
        let expected = if component != 0 && sps.chroma_format == 0 {
            [0, 0]
        } else {
            let shifts = super::component_shifts(component, sps.chroma_format);
            [
                sps.dimensions[0] as usize >> shifts[0],
                sps.dimensions[1] as usize >> shifts[1],
            ]
        };
        if plane.dimensions() != expected
            || expected[0].checked_mul(expected[1]) != Some(plane.samples().len())
            || expected != [0, 0]
                && (!plane.complete()
                    || plane.bit_depth() != sps.depth[usize::from(component != 0)])
        {
            return Err(invalid(
                "HEVC reference view-component planes are incomplete or inconsistent",
            ));
        }
    }
    Ok(picture.clone())
}

/// Validated same-role components of one access unit, independently of RPL
/// membership. Immutable pictures stay shared; only bounded index metadata is
/// owned here. The AU owner supplies each source's pinned parameter snapshot.
pub struct ViewComponentPool {
    depth:bool,
    current:PictureStatus,
    binding:Arc<LayeredBinding>,
    sources:Vec<(u16,u8,Arc<Picture>)>,
}
impl ViewComponentPool {
    pub fn required_metadata_bytes(count:usize)->Option<usize> {
        if count>63 {return None;}
        count.checked_mul(std::mem::size_of::<(u16,u8,Arc<Picture>)>())?
            .checked_add(std::mem::size_of::<Self>())
    }
    pub fn bind(
        context:&LayeredSliceContext<'_>,header:&SliceHeader,current:PictureStatus,
        bindings:&[Arc<LayeredBinding>],depth:bool,
        completed:&[(PictureStatus,Arc<Picture>)],budget:usize,
    )->Result<Self> {
        context.validate_poc_header(header)?;current.validate()?;
        if current.identity.layer_id!=context.sps.layer_id || current.nal!=header.nal {
            return Err(invalid("HEVC component pool current identity disagrees with slice"));
        }
        if completed.len()>63 || bindings.len()>63 {
            return Err(invalid("HEVC component pool layer count exceeds limit"));
        }
        let mut current_bindings=bindings.iter().filter(|b|b.sps.layer_id==current.identity.layer_id);
        let current_binding=current_bindings.next().ok_or_else(||invalid("HEVC component pool current binding is missing"))?;
        if current_bindings.next().is_some() || current_binding.vps.as_ref()!=context.vps
            || current_binding.sps!=*context.sps || current_binding.pps!=*context.pps {
            return Err(invalid("HEVC component pool current binding disagrees with context"));
        }
        let graph=&context.vps.extension.as_ref()
            .ok_or_else(||invalid("HEVC component pool requires VPS graph"))?.graph;
        let depth_layers=graph.depth_layer_ids()?.fold(0u64,|mask,id|mask|(1u64<<id));
        if (depth_layers&(1u64<<context.sps.layer_id)!=0)!=context.sps.sps.depth_layer {
            return Err(invalid("HEVC component pool current role disagrees with SPS"));
        }
        let mut count=0;
        for (status,_)in completed {
            status.validate()?;
            graph.view_index(status.identity.layer_id)?;
            let view=graph.view_index(status.identity.layer_id)?;
            if graph.view_component_for_index(view,depth)?==Some(status.identity.layer_id) {count+=1;}
        }
        let required=Self::required_metadata_bytes(count)
            .ok_or_else(||invalid("HEVC component pool storage overflow"))?;
        if required>budget {return Err(invalid("HEVC component pool exceeds metadata budget"));}
        let mut sources=Vec::new();
        sources.try_reserve_exact(count).map_err(|_|invalid("cannot allocate HEVC component pool"))?;
        if Self::required_metadata_bytes(sources.capacity()).is_none_or(|n|n>budget) {
            return Err(invalid("HEVC component pool capacity exceeds metadata budget"));
        }
        for (status,_)in completed {
            let layer=status.identity.layer_id;
            let view=graph.view_index(layer)?;
            if graph.view_component_for_index(view,depth)?!=Some(layer) {continue;}
            if layer>=current.identity.layer_id {
                return Err(invalid("HEVC component pool source is not an earlier decoded layer"));
            }
            let mut matches=bindings.iter().filter(|b|b.sps.layer_id==layer);
            let binding=matches.next().ok_or_else(||invalid("HEVC component pool source binding is missing"))?;
            if matches.next().is_some() {return Err(invalid("HEVC component pool source binding is ambiguous"));}
            let picture=select_current_view_component(context,header,current,binding,view,depth,completed)?;
            if sources.iter().any(|(old,_,_)|*old==view) {
                return Err(invalid("HEVC component pool repeats a view"));
            }
            sources.push((view,layer,picture));
        }
        sources.sort_unstable_by_key(|(view,_,_)|*view);
        Ok(Self{depth,sources,current,binding:current_binding.clone()})
    }
    pub fn depth(&self)->bool {self.depth}
    pub fn len(&self)->usize {self.sources.len()}
    pub fn is_empty(&self)->bool {self.sources.is_empty()}
    pub fn metadata_bytes(&self)->usize {
        std::mem::size_of::<Self>()+self.sources.capacity()*std::mem::size_of::<(u16,u8,Arc<Picture>)>()
    }
    pub fn get(&self,view:u16)->Option<&Arc<Picture>> {
        self.sources.binary_search_by_key(&view,|(v,_,_)|*v).ok().map(|i|&self.sources[i].2)
    }
    pub fn views(&self)->impl Iterator<Item=u16> + '_ {
        self.sources.iter().map(|(view,_,_)|*view)
    }
    pub fn layer(&self,view:u16)->Option<u8> {
        self.sources.binary_search_by_key(&view,|(v,_,_)|*v).ok().map(|i|self.sources[i].1)
    }
    pub fn dbbp_contour(&self,context:&LayeredSliceContext<'_>,header:&SliceHeader,current:PictureStatus,
        state:crate::codec::hevc_motion::disparity::derivation::TextureDisparity,
        origin:[u32;2],size:u32,budget:usize)->Result<crate::codec::hevc_motion::dbbp::Contour> {
        self.validate_for(context,header,current)?;
        if !self.depth || context.sps.sps.depth_layer || origin.iter().zip(context.sps.sps.dimensions)
            .any(|(&v,limit)|v.checked_add(size).is_none_or(|end|end>limit)) {
            return Err(invalid("HEVC DBBP requires a bound depth component covering the CU"));
        }
        let picture=self.get(state.vector.reference_view).ok_or_else(||invalid("HEVC DBBP reference depth view is missing"))?;
        crate::codec::hevc_motion::dbbp::Contour::from_plane(&picture.planes[0],origin,size as usize,state.refined_mv,budget)
    }
    /// Refine against the depth component of DispVec's selected reference view.
    /// Both depth ownership and camera direction come from the current AU/slice.
    /// Tool availability remains the reconstruction caller's responsibility.
    pub fn refine_disparity(
        &self,context:&LayeredSliceContext<'_>,header:&SliceHeader,current:PictureStatus,
        vector:crate::codec::hevc_motion::disparity::derivation::Vector,
        origin:[u32;2],size:u32,budget:usize,
    )->Result<crate::codec::hevc_motion::disparity::derivation::TextureDisparity> {
        self.validate_for(context,header,current)?;
        if !self.depth || context.sps.sps.depth_layer {
            return Err(invalid("HEVC texture disparity refinement requires depth component pool"));
        }
        if origin.iter().zip(context.sps.sps.dimensions).any(|(&at,limit)|
            at.checked_add(size).is_none_or(|end|end>limit)) {
            return Err(invalid("HEVC disparity refinement CU outside picture"));
        }
        let depth=self.get(vector.reference_view)
            .ok_or_else(||invalid("HEVC disparity refinement reference depth view is missing"))?;
        let graph=&context.vps.extension.as_ref()
            .ok_or_else(||invalid("HEVC disparity refinement requires VPS graph"))?.graph;
        let camera=header.layered.as_ref().and_then(|s|s.camera.as_ref())
            .ok_or_else(||invalid("HEVC disparity refinement requires slice camera"))?;
        if camera.view_index!=graph.view_index(context.sps.layer_id)? {
            return Err(invalid("HEVC disparity refinement camera belongs to another view"));
        }
        let transform=camera.disparity_transform(vector.reference_view,depth.depth[0])?;
        crate::codec::hevc_motion::disparity::derivation::refine(
            vector,Some(depth),Some(transform),origin,size,budget)
    }
    /// I.8.5.3.2.13: bind depth/camera to the selected base disparity view.
    /// Depth sampling uses DispVec, not the independently refined DispRefVec.
    pub fn view_synthesis(
        &self,context:&LayeredSliceContext<'_>,header:&SliceHeader,current:PictureStatus,
        vector:crate::codec::hevc_motion::disparity::derivation::Vector,
        lists:&[Vec<crate::codec::hevc_motion::Reference>;2],rect:[u32;4],budget:usize,
    )->Result<Option<crate::codec::hevc_motion::view_synthesis::ViewSynthesis>> {
        use crate::codec::hevc_motion::{texture::Identity,view_synthesis};
        self.validate_for(context,header,current)?;
        if !self.depth || context.sps.sps.depth_layer {return Err(invalid("HEVC VSP requires depth component pool"));}
        let graph=&context.vps.extension.as_ref().ok_or_else(||invalid("HEVC VSP requires VPS graph"))?.graph;
        let mut identities=[[Identity{poc:0,view:0};15];2];let mut counts=[0;2];
        for list in 0..2 {
            if lists[list].len()>15 || lists[list].len()!=usize::from(header.references[list]) {return Err(invalid("HEVC VSP active references disagree with slice"));}
            counts[list]=lists[list].len();
            for (index,reference)in lists[list].iter().enumerate() {identities[list][index]=Identity{poc:reference.poc,view:graph.view_index(reference.layer_id)?};}
        }
        let active=[&identities[0][..counts[0]],&identities[1][..counts[1]]];
        let bidirectional=header.slice_type==crate::codec::hevc_cabac::SliceType::B;
        if view_synthesis::reference(active,bidirectional,vector.reference_view)?.is_none() {return Ok(None);}
        if (0..2).any(|a|rect[a].checked_add(rect[a+2]).is_none_or(|end|end>context.sps.sps.dimensions[a])) {return Err(invalid("HEVC VSP PB outside current picture"));}
        let depth=self.get(vector.reference_view).ok_or_else(||invalid("HEVC VSP reference depth view is missing"))?;
        let camera=header.layered.as_ref().and_then(|s|s.camera.as_ref()).ok_or_else(||invalid("HEVC VSP requires slice camera"))?;
        if camera.view_index!=graph.view_index(context.sps.layer_id)? {return Err(invalid("HEVC VSP camera belongs to another view"));}
        let transform=camera.disparity_transform(vector.reference_view,depth.depth[0])?;
        view_synthesis::derive(Some(depth),Some(transform),active,bidirectional,vector.reference_view,rect,vector.mv,budget)
    }
    pub fn validate_for(&self,context:&LayeredSliceContext<'_>,header:&SliceHeader,current:PictureStatus)->Result<()> {
        context.validate_poc_header(header)?;current.validate()?;
        if current!=self.current || current.nal!=header.nal || current.identity.layer_id!=context.sps.layer_id
            || self.binding.vps.as_ref()!=context.vps || self.binding.sps!=*context.sps
            || self.binding.pps!=*context.pps {
            return Err(invalid("HEVC component pool does not belong to current reconstruction"));
        }
        Ok(())
    }
}
