//! Native texture IV/IVShift/DI/DIShift candidates from a validated AU pool.
use super::{Motion,Vector,Reference,inter_view_candidate,merge_order,subblock_motion,texture::Identity,view_synthesis,disparity::derivation::TextureDisparity};
use crate::codec::{hevc_picture::ViewComponentPool,hevc_slice::{LayeredSliceContext,SliceHeader},hevc_cabac::SliceType};
use crate::{Result,invalid};

pub struct Candidates {
    pub blocks:subblock_motion::Subblocks<Motion>,
    pub iv:Option<Motion>,pub shifted_iv:Option<Motion>,
    pub di:Option<Motion>,pub shifted_di:Option<Motion>,
    pub vsp:Option<view_synthesis::ViewSynthesis>,pub left_vsp:bool,
}
impl Candidates {
    pub fn specials(&self)->merge_order::Specials<'_,Motion> {
        merge_order::Specials{inter_view:self.iv.as_ref(),inter_view_shift:self.shifted_iv.as_ref(),
            disparity:self.di.as_ref(),disparity_shift:self.shifted_di.as_ref(),
            view_synthesis:self.vsp.is_some(),left_view_synthesis:self.left_vsp,..Default::default()}
    }
}
fn available(motion:Motion)->Option<Motion> {(motion!=[None;2]).then_some(motion)}
fn native(motion:view_synthesis::WideMotion)->Result<Motion> {
    let mut result=[None;2];
    for list in 0..2 {if let Some(vector)=motion[list] {
        result[list]=Some(Vector{reference:vector.reference,mv:vector.mv});
    }}
    Ok(result)
}
pub fn derive(
    context:&LayeredSliceContext<'_>,header:&SliceHeader,poc:i32,
    lists:&[Vec<Reference>;2],pool:&ViewComponentPool,state:TextureDisparity,
    rect:[u32;4],budget:usize,
)->Result<Candidates> {
    if context.sps.sps.depth_layer || pool.depth() {return Err(invalid("HEVC texture IV requires texture sources"));}
    let graph=&context.vps.extension.as_ref().ok_or_else(||invalid("HEVC texture IV requires VPS graph"))?.graph;
    let view=state.vector.reference_view;
    let layer=graph.view_component_for_index(view,false)?.ok_or_else(||invalid("HEVC texture IV reference view is absent"))?;
    if pool.layer(view)!=Some(layer) {return Err(invalid("HEVC texture IV current-AU source is missing"));}
    let source=pool.get(view).unwrap();
    let mut pocs=[[0;15];2];let mut identities=[[Identity{poc:0,view:0};15];2];let mut counts=[0;2];
    for list in 0..2 {
        if lists[list].len()!=usize::from(header.references[list]) || lists[list].len()>15 {
            return Err(invalid("HEVC texture IV active lists disagree with slice"));
        }
        counts[list]=lists[list].len();
        for (i,reference)in lists[list].iter().enumerate() {
            pocs[list][i]=reference.poc;
            identities[list][i]=Identity{poc:reference.poc,view:graph.view_index(reference.layer_id)?};
        }
    }
    if !context.sps.sps.three_d.as_ref().is_some_and(|e|e.inter_view_disparity[0]) {
        return Ok(Candidates{blocks:subblock_motion::Subblocks{size:[rect[2],rect[3]],default:[None;2],motion:Vec::new()},iv:None,shifted_iv:None,di:None,shifted_di:None,vsp:None,left_vsp:false});
    }
    let active=[&pocs[0][..counts[0]],&pocs[1][..counts[1]]];
    let bidirectional=header.slice_type==SliceType::B;
    let log=context.sps.sps.three_d.as_ref().ok_or_else(||invalid("HEVC texture IV requires 3D SPS"))?.disparity_sub_block_log2;
    let blocks=inter_view_candidate::subblocks(source,rect,context.sps.sps.dimensions,log,
        |_|Ok(state.refined_mv),active,bidirectional,budget)?;
    let shifted=inter_view_candidate::from_picture(source,[rect[0],rect[1]],[rect[2],rect[3]],
        state.refined_mv,[(rect[2]*2)as i32,(rect[3]*2)as i32],active,bidirectional)?;
    let (di,shifted_di)=view_synthesis::disparity_information(
        [&identities[0][..counts[0]],&identities[1][..counts[1]]],bidirectional,view,poc,state.refined_mv[0])?;
    Ok(Candidates{iv:available(blocks.default),blocks,shifted_iv:available(shifted),
        di:available(native(di)?),shifted_di:available(native(shifted_di)?),vsp:None,left_vsp:false})
}
