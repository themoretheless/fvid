//! Annex I.8.5.5 texture disparity derivation and I.8.5.6 depth default.
use super::{CameraTransform, samples};
use crate::{Result, invalid};
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Vector {
    pub reference_view: u16,
    pub mv: [i64; 2],
}
#[derive(Clone, Copy, Debug)]
pub struct NeighbourVector {
    pub reference_view: u16,
    pub poc: i32,
    pub mv: [i64; 2],
}
#[derive(Clone, Copy, Debug)]
pub struct Neighbour {
    /// Both owning-slice reference lists must already be resolved.
    pub motion: [Option<NeighbourVector>; 2],
    pub skip: bool,
    pub inter_view: bool,
    /// DispRefVec and RefViewIdx saved when this neighbour was reconstructed.
    pub refined: Option<Vector>,
}
#[derive(Clone, Copy, Debug)]
pub struct TemporalVector {
    pub reference_view: u16,
    pub mv: [i64; 2],
}

/// Bind spatial disparity input to a decoded picture's canonical motion lists
/// and coded skip flag. The caller supplies the independently published IV
/// selection/refinement state and enforces slice/tile/z-scan availability.
pub fn neighbour_from_picture(
    picture:&super::super::super::hevc_picture::Picture,
    point:[u32;2],inter_view:bool,refined:Option<Vector>,
    mut view:impl FnMut(u8)->Result<u16>,
)->Result<Option<Neighbour>> {
    let source=picture.prediction_motion_at(point)?;
    if source==[None;2] {return Ok(None);}
    let skip=picture.prediction_is_skipped_at(point)?;
    if skip && inter_view && refined.is_none() {
        return Err(invalid("HEVC IV skip neighbour has no saved disparity"));
    }
    let mut motion=[None;2];
    for list in 0..2 {
        if let Some(vector)=source[list] {
            let slot=vector.reference as usize;
            let layer=*picture.reference_layers[list].get(slot)
                .ok_or_else(||invalid("HEVC spatial disparity reference outside saved layer list"))?;
            let poc=*picture.reference_pocs[list].get(slot)
                .ok_or_else(||invalid("HEVC spatial disparity reference outside saved POC list"))?;
            motion[list]=Some(NeighbourVector{reference_view:view(layer)?,poc,mv:vector.mv});
        }
    }
    Ok(Some(Neighbour{motion,skip,inter_view,refined}))
}
/// Native saved-state bridge: no externally supplied IV/refinement labels.
pub fn published_neighbour_from_picture(
    picture:&super::super::super::hevc_picture::Picture,point:[u32;2],
    view:impl FnMut(u8)->Result<u16>,
)->Result<Option<Neighbour>> {
    let state=picture.inter_view_disparity_at(point)?;
    neighbour_from_picture(picture,point,state.is_some(),state.map(|state|Vector {
        reference_view:state.vector.reference_view,mv:state.refined_mv,
    }),view)
}

/// Slice-bound temporal disparity sources and active views. Picture metadata is
/// read from native decoded references, never supplied as guessed IRAP/TID.
pub struct PreparedTexture<'a> {
    graph:&'a crate::codec::hevc_vps_extension::LayerDependencyPrefix,
    pictures:[Option<(&'a crate::codec::hevc_picture::Picture,u16)>;2],
    positions:[(usize,usize);2],
    source_count:usize,
    views:[[u16;15];2],counts:[usize;2],default_view:Option<u16>,
    dimensions:[u32;2],poc:i32,merge_log2:u8,ctu_log2:u8,
}
impl<'a> PreparedTexture<'a> {
    pub fn prepare(
        context:&'a crate::codec::hevc_slice::LayeredSliceContext<'_>,
        header:&crate::codec::hevc_slice::SliceHeader,poc:i32,
        lists:&'a [Vec<crate::codec::hevc_motion::Reference>;2],
    )->Result<Self> {
        use crate::codec::hevc_cabac::SliceType;
        context.validate_poc_header(header)?;
        let graph=&context.vps.extension.as_ref()
            .ok_or_else(||invalid("HEVC texture disparity requires VPS graph"))?.graph;
        if context.sps.sps.depth_layer || graph.depth_layer_ids()?.any(|id|id==context.sps.layer_id) {
            return Err(invalid("HEVC texture disparity requires texture component"));
        }
        let current_view=graph.view_index(context.sps.layer_id)?;
        let empty=super::CandidatePicture{view_index:0,temporal_id:0,irap:false,poc:0};
        let mut metadata=[[empty;15];2];let mut views=[[0;15];2];let mut counts=[0;2];
        for list in 0..2 {
            if lists[list].len()!=usize::from(header.references[list]) || lists[list].len()>15 {
                return Err(invalid("HEVC texture disparity active lists disagree with slice"));
            }
            counts[list]=lists[list].len();
            for (slot,reference)in lists[list].iter().enumerate() {
                if graph.depth_layer_ids()?.any(|id|id==reference.layer_id) {
                    return Err(invalid("HEVC texture disparity reference has depth role"));
                }
                metadata[list][slot]=reference.disparity_metadata(graph)?;
                views[list][slot]=metadata[list][slot].view_index;
            }
        }
        let active=[&metadata[0][..counts[0]],&metadata[1][..counts[1]]];
        let chosen=super::candidate_pictures(active,header.slice_type==SliceType::B,
            header.temporal_mvp,header.collocated_list==0,header.collocated_ref as usize,current_view,poc)?;
        let mut pictures=[None;2];let mut positions=[(0,0);2];
        for (index,&(list,slot))in chosen.iter().enumerate() {
            let picture=lists[list][slot].picture.as_deref().unwrap();
            if picture.dimensions!=context.sps.sps.dimensions || picture.depth!=context.sps.sps.depth
                || picture.planes.iter().any(|p|!p.complete()) {
                return Err(invalid("HEVC texture disparity source representation or completion disagrees"));
            }
            picture.coding_unit_at([0,0])?;
            pictures[index]=Some((picture,metadata[list][slot].view_index));
            positions[index]=(list,slot);
        }
        Ok(Self{graph,pictures,positions,source_count:chosen.len(),views,counts,
            dimensions:context.sps.sps.dimensions,poc,merge_log2:context.pps.pps.parallel_merge_log2,
            ctu_log2:context.sps.sps.coding_block_log2[1],
            default_view:super::default_reference_view(active,header.slice_type==SliceType::B,current_view,poc)})
    }
    pub fn candidate_positions(&self)->&[(usize,usize)] {&self.positions[..self.source_count]}
    pub fn default_view(&self)->Option<u16> {self.default_view}
    pub fn temporal(&self,origin:[u32;2],size:u32)->Result<Option<Vector>> {
        temporal_position(origin,size)?;
        for &(picture,view)in self.pictures.iter().flatten() {
            let candidate=temporal_from_picture(picture,origin,size,view,
                [&self.views[0][..self.counts[0]],&self.views[1][..self.counts[1]]],
                |layer|self.graph.view_index(layer))?;
            if candidate.is_some() {return Ok(candidate);}
        }
        Ok(None)
    }
    /// Complete disparity component pipeline for one CU. The caller enforces
    /// spatial availability and binds any refinement depth/camera to the AU.
    /// This component API does not admit texture IV/DI/VSP coded tools.
    pub fn derive(
        &self,origin:[u32;2],size:u32,merge:bool,
        refinement:Option<(&crate::codec::hevc_picture::Picture,CameraTransform)>,budget:usize,
        neighbour:impl FnMut([i64;2])->Result<Option<Neighbour>>,
    )->Result<Option<TextureDisparity>> {
        if origin.iter().zip(self.dimensions).any(|(&v,limit)|v.checked_add(size).is_none_or(|end|end>limit)) {
            return Err(invalid("HEVC texture disparity CU outside picture"));
        }
        let base=texture(origin,size,self.poc,merge,self.merge_log2,self.ctu_log2,
            self.temporal(origin,size)?,self.default_view,neighbour)?;
        base.map(|base|refine(base,refinement.map(|v|v.0),refinement.map(|v|v.1),origin,size,budget)).transpose()
    }
}

/// I-294..I-296: prefer L0 then L1; any active current reference with the
/// candidate view suffices. Unlike spatial disparity, no POC equality is tested.
pub fn temporal(
    candidate_view: u16,
    motion: [Option<TemporalVector>; 2],
    active_views: [&[u16]; 2],
) -> Result<Option<Vector>> {
    if active_views.iter().any(|list| list.len() > 15) {
        return Err(invalid(
            "HEVC disparity active reference list exceeds limit",
        ));
    }
    for vector in motion.into_iter().flatten() {
        if vector.reference_view != candidate_view
            && active_views
                .iter()
                .any(|list| list.contains(&vector.reference_view))
        {
            return Ok(Some(Vector {
                reference_view: vector.reference_view,
                mv: vector.mv.map(i64::from),
            }));
        }
    }
    Ok(None)
}
/// I-292/I-293: use the 16-aligned CU centre, not the IV merge 8-grid lookup.
pub fn temporal_position(origin: [u32; 2], size: u32) -> Result<[u32; 2]> {
    if !matches!(size, 8 | 16 | 32 | 64) {
        return Err(invalid("invalid HEVC disparity coding block size"));
    }
    let mut point = [0; 2];
    for i in 0..2 {
        point[i] = origin[i]
            .checked_add(size / 2)
            .ok_or_else(|| invalid("HEVC disparity coding block coordinate overflow"))?
            & !15;
    }
    Ok(point)
}
/// Read temporal disparity from an owned decoded motion map and its canonical
/// reference-layer identities. Candidate ordering and DPB availability are the
/// caller's responsibility; the first available picture wins at I.8.5.5.2.
pub fn temporal_from_picture(
    picture: &super::super::super::hevc_picture::Picture,
    origin: [u32; 2],
    size: u32,
    candidate_view: u16,
    active_views: [&[u16]; 2],
    mut view: impl FnMut(u8) -> Result<u16>,
) -> Result<Option<Vector>> {
    let source = picture.prediction_motion_at(temporal_position(origin, size)?)?;
    let mut motion = [None; 2];
    for list in 0..2 {
        if let Some(vector) = source[list] {
            let layer = *picture.reference_layers[list]
                .get(vector.reference as usize)
                .ok_or_else(|| {
                    invalid("HEVC temporal disparity reference outside saved layer list")
                })?;
            motion[list] = Some(TemporalVector {
                reference_view: view(layer)?,
                mv: vector.mv,
            });
        }
    }
    temporal(candidate_view, motion, active_views)
}

/// I-280..I-288 plus default fallback. `neighbour` must enforce z-scan,
/// slice/tile and intra availability; this function applies parallel-merge and
/// B1 CTU-row restrictions itself. A later direct spatial disparity outranks
/// an earlier skip/inter-view fallback. Temporal disparity outranks both.
pub fn texture(
    origin: [u32; 2],
    size: u32,
    current_poc: i32,
    merge: bool,
    merge_log2: u8,
    ctu_log2: u8,
    temporal: Option<Vector>,
    default_view: Option<u16>,
    mut neighbour: impl FnMut([i64; 2]) -> Result<Option<Neighbour>>,
) -> Result<Option<Vector>> {
    temporal_position(origin, size)?;
    if !(2..=6).contains(&merge_log2) || !(4..=6).contains(&ctu_log2) || size > 1u32 << ctu_log2 {
        return Err(invalid("invalid HEVC spatial disparity merge/CTU geometry"));
    }
    if temporal.is_some() {
        return Ok(temporal);
    }
    let [x, y] = origin.map(i64::from);
    let side = i64::from(size);
    let mut fallback = [None; 2];
    for (index, point) in [[x - 1, y + side - 1], [x + side - 1, y - 1]]
        .into_iter()
        .enumerate()
    {
        if merge
            && x >> merge_log2 == point[0] >> merge_log2
            && y >> merge_log2 == point[1] >> merge_log2
        {
            continue;
        }
        let Some(neighbour) = neighbour(point)? else {
            continue;
        };
        let temporal_allowed = index == 0 || point[1] >> ctu_log2 == y >> ctu_log2;
        for vector in neighbour.motion.into_iter().flatten() {
            if vector.poc == current_poc {
                return Ok(Some(Vector {
                    reference_view: vector.reference_view,
                    mv: vector.mv.map(i64::from),
                }));
            }
            if temporal_allowed
                && fallback[index].is_none()
                && neighbour.skip
                && neighbour.inter_view
            {
                fallback[index] = Some(
                    neighbour
                        .refined
                        .ok_or_else(|| invalid("HEVC IV skip neighbour has no saved disparity"))?,
                );
            }
        }
    }
    Ok(fallback.into_iter().flatten().next().or_else(|| {
        default_view.map(|reference_view| Vector {
            reference_view,
            mv: [0; 2],
        })
    }))
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TextureDisparity {
    pub vector: Vector,
    pub refined_mv: [i64; 2],
}
/// I-289..I-291 with optional depth-reference refinement. Base DispVec is
/// retained even when the camera-derived horizontal DispRefVec replaces it.
pub fn refine(
    vector: Vector,
    depth: Option<&super::super::super::hevc_picture::Picture>,
    camera: Option<CameraTransform>,
    origin: [u32; 2],
    size: u32,
    budget: usize,
) -> Result<TextureDisparity> {
    temporal_position(origin, size)?;
    let refined_mv = if let Some(depth) = depth {
        let camera = camera
            .ok_or_else(|| invalid("HEVC disparity refinement requires camera parameters"))?;
        let values = samples::from_picture(
            depth,
            origin,
            [size as usize; 2],
            vector.mv,
            0,
            camera,
            budget,
        )?;
        [values.values[0], 0]
    } else {
        vector.mv
    };
    Ok(TextureDisparity { vector, refined_mv })
}
/// I-297..I-299: depth uses the midpoint depth's backward disparity, unclipped.
pub fn depth(reference_view: u16, camera: CameraTransform) -> Result<Vector> {
    Ok(Vector {
        reference_view,
        mv: [camera.sample(camera.sample_count() / 2)?.backward, 0],
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    fn motion(poc: i32, mv: i64) -> Neighbour {
        Neighbour {
            motion: [
                Some(NeighbourVector {
                    reference_view: 1,
                    poc,
                    mv: [mv, 0],
                }),
                None,
            ],
            skip: false,
            inter_view: false,
            refined: None,
        }
    }
    fn run(temporal: Option<Vector>, left: Neighbour, top: Neighbour) -> Option<Vector> {
        texture([16, 16], 8, 10, false, 2, 5, temporal, Some(0), |p| {
            Ok(Some(if p[0] == 15 { left } else { top }))
        })
        .unwrap()
    }
    #[test]
    fn temporal_then_direct_then_saved_skip_then_default_priority() {
        let saved = Vector {
            reference_view: 2,
            mv: [99, -7],
        };
        let mut left = motion(9, 1);
        left.skip = true;
        left.inter_view = true;
        left.refined = Some(saved);
        assert_eq!(run(None, left, motion(10, 3)).unwrap().mv, [3, 0]);
        assert_eq!(run(None, left, motion(9, 3)), Some(saved));
        assert_eq!(run(Some(saved), motion(10, 1), motion(10, 2)), Some(saved));
        assert_eq!(
            run(None, motion(9, 1), motion(9, 2)),
            Some(Vector {
                reference_view: 0,
                mv: [0, 0]
            })
        );
    }
    #[test]
    fn cross_ctu_top_can_supply_direct_but_not_saved_disparity() {
        let mut top = motion(9, 1);
        top.skip = true;
        top.inter_view = true;
        top.refined = Some(Vector {
            reference_view: 1,
            mv: [8, 0],
        });
        let run = |top| {
            texture([16, 32], 8, 10, false, 2, 5, None, Some(0), |p| {
                Ok((p[1] == 31).then_some(top))
            })
            .unwrap()
        };
        assert_eq!(run(top).unwrap().mv, [0, 0]);
        top.motion[0].as_mut().unwrap().poc = 10;
        assert_eq!(run(top).unwrap().mv, [1, 0]);
        assert!(
            texture([8, 8], 8, 10, true, 6, 6, None, Some(0), |_| panic!(
                "same merge region must not be queried"
            ))
            .is_ok()
        );
    }
    #[test]
    fn temporal_view_rules_midpoint_and_unavailable_default() {
        let motion = [
            Some(TemporalVector {
                reference_view: 1,
                mv: [3, -5],
            }),
            Some(TemporalVector {
                reference_view: 2,
                mv: [7, 9],
            }),
        ];
        assert_eq!(
            temporal(1, motion, [&[1, 2], &[]]).unwrap(),
            Some(Vector {
                reference_view: 2,
                mv: [7, 9]
            })
        );
        assert!(temporal(1, motion, [&[1], &[]]).unwrap().is_none());
        assert_eq!(temporal_position([8, 8], 8).unwrap(), [0, 0]);
        assert_eq!(temporal_position([8, 8], 16).unwrap(), [16, 16]);
        assert!(
            texture([0, 0], 8, 0, false, 2, 5, None, None, |_| Ok(None))
                .unwrap()
                .is_none()
        );
        let camera = CameraTransform::new([i32::MAX, 0, -i32::MAX, 1], 0, 8).unwrap();
        assert_eq!(depth(2, camera).unwrap().mv, [i64::from(i32::MAX), 0]);
    }
}
