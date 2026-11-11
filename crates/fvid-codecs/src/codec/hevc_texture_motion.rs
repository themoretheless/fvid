//! Annex I.8.5.3.2.9/.11 texture merge derivation primitives.
//! The caller supplies motion at the exact aligned texture location and the
//! reference lists of its owning slice. A temporal 16x16 map is insufficient.
use super::Motion;
use crate::{Result, invalid};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Identity {
    pub poc: i32,
    pub view: u16,
}
/// Depth motion in integer luma samples, deliberately distinct from the
/// quarter-sample `Vector` used by ordinary texture reconstruction.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Vector {
    pub reference: u8,
    pub samples: [i16; 2],
}
pub type Candidate = [Option<Vector>; 2];

/// Lossless publication in the decoder's quarter-sample motion domain. In
/// particular +8192 integer samples must remain +32768, not wrap to -32768.
pub fn quarter_motion(motion: Candidate) -> Motion {
    motion.map(|v| v.map(|v| super::Vector {
        reference: v.reference, mv: v.samples.map(|s| i64::from(s) * 4),
    }))
}
/// Recover published integer depth motion, rejecting fractional or oversized
/// values rather than rounding an unrelated ordinary texture prediction.
pub fn integer_motion(motion: Motion) -> Result<Candidate> {
    let mut result = [None; 2];
    for list in 0..2 {
        if let Some(vector) = motion[list] {
            if vector.mv.iter().any(|v| v % 4 != 0 || i16::try_from(v / 4).is_err()) {
                return Err(invalid("HEVC published depth motion is fractional or outside integer range"));
            }
            result[list] = Some(Vector { reference: vector.reference, samples: vector.mv.map(|v| (v / 4) as i16) });
        }
    }
    Ok(result)
}

/// Integer-luma depth prediction without narrowing to a quarter-sample i16.
/// Includes edge extension and uni/bi weighting in the sample domain. Sources
/// must be completed reference pictures; current-picture prediction uses a
/// separate availability-validated path and is deliberately not inferred here.
pub fn predict(
    lists: &[Vec<super::Reference>; 2], motion: Candidate, rect: [u32; 4],
    depth: u8, weights: Option<&crate::codec::hevc_slice::Weights>,
) -> Result<Vec<u16>> {
    let [x,y,w,h] = rect;
    if !(8..=16).contains(&depth) || w == 0 || h == 0 || w > 64 || h > 64
        || x.checked_add(w).is_none_or(|v| v > i32::MAX as u32)
        || y.checked_add(h).is_none_or(|v| v > i32::MAX as u32)
        || weights.is_some_and(|v| v.denominators[0] > 7)
    {
        return Err(invalid("invalid HEVC integer depth prediction geometry or format"));
    }
    let mut inputs = [None; 2];
    for list in 0..2 {
        let Some(vector) = motion[list] else { continue };
        let reference = lists[list].get(vector.reference as usize)
            .ok_or_else(|| invalid("HEVC integer depth reference outside active list"))?;
        let picture = reference.picture.as_deref()
            .ok_or_else(|| invalid("HEVC integer depth prediction requires completed reference"))?;
        let plane = &picture.planes[0];
        let [width,height] = plane.dimensions();
        if width == 0 || height == 0 || picture.depth[0] != depth
            || picture.dimensions != [width as u32, height as u32] || !plane.complete()
        {
            return Err(invalid("HEVC integer depth reference geometry, depth or availability mismatch"));
        }
        let weight = weights.map(|v| v.lists[list].get(vector.reference as usize)
            .ok_or_else(|| invalid("HEVC integer depth weight outside active list"))).transpose()?;
        let scale = weight.map_or(1, |v| i64::from(v.values[0]));
        let offset = weight.map_or(0, |v| i64::from(v.offsets[0])
            << if weights.is_some_and(|w| w.high_precision_offsets) { 0 } else { depth - 8 });
        inputs[list] = Some((plane, vector.samples, scale, offset));
    }
    if inputs.iter().all(Option::is_none) {
        return Err(invalid("HEVC integer depth prediction has no active reference"));
    }
    let denominator = weights.map_or(0, |v| v.denominators[0]);
    let maximum = (1i64 << depth) - 1;
    let mut result = Vec::with_capacity((w * h) as usize);
    for row in 0..h { for col in 0..w {
        let mut values = [None; 2];
        for list in 0..2 {
            if let Some((plane,mv,scale,offset)) = inputs[list] {
                let [width,height] = plane.dimensions();
                let xx = (i64::from(x) + i64::from(col) + i64::from(mv[0])).clamp(0, width as i64 - 1) as usize;
                let yy = (i64::from(y) + i64::from(row) + i64::from(mv[1])).clamp(0, height as i64 - 1) as usize;
                values[list] = Some((i64::from(plane.samples()[yy * width + xx]) * scale, offset));
            }
        }
        let value = match values {
            [Some((a,oa)),Some((b,ob))] => (a + b + ((oa + ob + 1) << denominator)) >> (denominator + 1),
            [Some((a,o)),None] | [None,Some((a,o))] =>
                ((a + if denominator == 0 {0} else {1 << (denominator - 1)}) >> denominator) + o,
            _ => unreachable!(),
        };
        result.push(value.clamp(0,maximum) as u16);
    }}
    Ok(result)
}

/// Surviving entries of the normative initial merge list, after its own spatial
/// duplicate rules. Nonspatial entries retain their identity and order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MergeKind { A1, B1, B0, A0, B2, Other }
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MergeSelection { Texture, Initial(usize) }

/// I-135/I-136 for depth texture prediction with the other Annex I inter-view
/// tools disabled. Compare motions in the same units/reference-index domain.
/// Only A1/B1 are pruned against T; equal B0/A0/B2 must remain in the list.
pub fn merge_order<T: PartialEq>(
    initial: &[(MergeKind, T)], texture: Option<&T>, maximum: usize,
) -> Result<Vec<MergeSelection>> {
    let specials=super::merge_order::Specials {texture,depth:true,..Default::default()};
    let mut result=Vec::with_capacity(maximum.min(6));
    super::merge_order::order_into(initial,&specials,maximum,|selection| {
        result.push(match selection {
            super::merge_order::Selection::Texture=>MergeSelection::Texture,
            super::merge_order::Selection::Initial(index)=>MergeSelection::Initial(index),
            _=>return Err(invalid("unexpected HEVC texture-only merge selection")),
        });
        Ok(())
    })?;
    Ok(result)
}

/// I-173 through I-176. Reject malformed/outside blocks before requesting a
/// texture PB; do not clamp to an unrelated neighbouring prediction block.
pub fn texture_location(rect: [u32; 4], dimensions: [u32; 2]) -> Result<[u32; 2]> {
    let [x, y, w, h] = rect;
    if w == 0 || h == 0
        || x.checked_add(w).is_none_or(|end| end > dimensions[0])
        || y.checked_add(h).is_none_or(|end| end > dimensions[1])
    {
        return Err(invalid("HEVC texture-motion block outside picture"));
    }
    Ok([(x + w / 2) & !7, (y + h / 2) & !7])
}

/// I-177 through I-180. Lists must already be resolved to POC/ViewIdx, not
/// compared by layer id: texture and depth have different layers in one view.
pub fn candidate(
    motion: Motion,
    texture_lists: [&[Identity]; 2],
    depth_lists: [&[Identity]; 2],
    bidirectional: bool,
) -> Result<Candidate> {
    let mut result = [None; 2];
    for list in 0..if bidirectional { 2 } else { 1 } {
        if depth_lists[list].len() > 15 || texture_lists[list].len() > 15 {
            return Err(invalid("HEVC texture-motion active reference list exceeds limit"));
        }
        let Some(vector) = motion[list] else { continue };
        if vector.mv.iter().any(|v| !(-32768..=32767).contains(v)) {
            return Err(invalid("HEVC texture source vector exceeds coded signed-16-bit range"));
        }
        let identity = texture_lists[list].get(vector.reference as usize)
            .ok_or_else(|| invalid("HEVC texture-motion reference outside owning slice list"))?;
        if let Some(index) = depth_lists[list].iter().position(|p| p == identity) {
            result[list] = Some(Vector {
                reference: index as u8,
                samples: vector.mv.map(|v| ((v + 2) >> 2) as i16),
            });
        }
    }
    Ok(result)
}

/// Texture candidate publication in integer depth motion units.
pub type Subblocks=super::subblock_motion::Subblocks<Candidate>;

/// I-150 through I-159, specialized to candidate T. `derive` resolves the
/// motion and slice references at each exact texture_location. Intra/unmatched
/// sub-blocks inherit the default only when both of their lists are inactive.
pub fn subblocks(
    rect: [u32; 4],
    dimensions: [u32; 2],
    log2_size: u8,
    mut derive: impl FnMut([u32; 2]) -> Result<Candidate>,
) -> Result<Subblocks> {
    let layout=super::subblock_motion::Layout::new(rect,dimensions,log2_size)?;
    // The reconstruction owner already reserves its conservative scratch peak;
    // geometry bounds this table to at most 64 entries, as before.
    super::subblock_motion::derive(layout,[None;2],usize::MAX,
        |block|derive(texture_location(block,dimensions)?))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn integer_publication_roundtrips_full_signed_range_without_quarter_wrap() {
        for v in i16::MIN..=i16::MAX {
            let source=[Some(Vector {reference:3,samples:[v,v]}),None];
            let published=quarter_motion(source);
            assert_eq!(published[0].unwrap().mv,[i64::from(v)*4;2]);
            assert_eq!(integer_motion(published).unwrap(),source);
        }
        let positive=quarter_motion([None,Some(Vector {reference:0,samples:[8192,-8192]})]);
        assert_eq!(positive[1].unwrap().mv,[32768,-32768]);
        assert!(integer_motion([Some(super::super::Vector {reference:0,mv:[1,0]}),None]).is_err());
        assert!(integer_motion([Some(super::super::Vector {reference:0,mv:[i64::from(i32::MAX),0]}),None]).is_err());
        let ids=[Identity {poc:0,view:0}];
        assert!(candidate(positive,[&[],&ids],[&[],&ids],true).is_err());
    }
    #[test]
    fn texture_merge_prunes_only_matching_a1_b1_and_preserves_other_identity() {
        use MergeKind::*;
        use MergeSelection::*;
        let initial = [(A1, 10), (B1, 20), (B0, 10), (A0, 10), (Other, 30)];
        assert_eq!(merge_order(&initial, Some(&10), 5).unwrap(),
            [Texture, Initial(1), Initial(2), Initial(3), Initial(4)]);
        assert_eq!(merge_order(&initial, Some(&20), 5).unwrap(),
            [Texture, Initial(0), Initial(2), Initial(3), Initial(4)]);
        assert_eq!(merge_order(&initial, Some(&30), 5).unwrap(),
            [Texture, Initial(0), Initial(1), Initial(2), Initial(3)]);
        let other = [(A1, 10), (B1, 20), (Other, 30), (Other, 30), (Other, 30)];
        assert_eq!(merge_order(&other, Some(&10), 5).unwrap(),
            [Texture, Initial(1), Initial(2), Initial(3), Initial(4)]);
        assert!(merge_order(&[(A1, 1), (A1, 2)], Some(&3), 2).is_err());
        assert!(merge_order::<i32>(&[], None, 0).is_err());
        assert!(merge_order(&[(Other, 1)], None, 2).is_err());
    }
    #[test]
    fn all_spatial_subsets_and_maxima_keep_initial_order_without_texture() {
        use MergeKind::*;
        use MergeSelection::*;
        for mask in 0..32 { for maximum in 1..=6 {
            let mut initial: Vec<_> = [A1, B1, B0, A0, B2].into_iter().enumerate()
                .filter(|(i,_)| mask & (1 << i) != 0)
                .map(|(i,k)| (k, i as i32)).take(maximum).collect();
            initial.resize(maximum, (Other, 100));
            assert_eq!(merge_order(&initial, None, maximum).unwrap(),
                (0..maximum).map(Initial).collect::<Vec<_>>());
            for t in -1..=5 {
                let order = merge_order(&initial, Some(&t), maximum).unwrap();
                assert_eq!(order.len(), maximum);
                assert_eq!(order[0], Texture);
                for entry in &order[1..] {
                    let Initial(index) = *entry else { panic!("duplicate T") };
                    let (kind, value) = initial[index];
                    assert!(!matches!(kind, A1 | B1) || value != t);
                }
            }
        }}
    }
    fn id(poc: i32, view: u16) -> Identity { Identity { poc, view } }
    fn vector(reference: u8, mv: [i16; 2]) -> super::super::Vector {
        super::super::Vector { reference, mv: mv.map(i64::from) }
    }
    #[test]
    fn matching_uses_view_poc_same_list_and_first_duplicate() {
        let tex = [id(7, 2), id(8, 3)];
        let depth = [id(7, 9), id(8, 2), id(7, 2), id(7, 2)];
        assert_eq!(candidate([Some(vector(0, [7, -7])), Some(vector(1, [6, -6]))],
            [&tex, &tex], [&depth, &[id(8, 3)]], true).unwrap(),
            [Some(Vector { reference: 2, samples: [2, -2] }), Some(Vector { reference: 0, samples: [2, -1] })]);
        assert_eq!(candidate([Some(vector(1, [0, 0])), None],
            [&tex, &[]], [&depth, &[id(8, 3)]], true).unwrap(), [None; 2]);
        // A P slice never consumes a texture L1 index, including an invalid one.
        assert_eq!(candidate([None, Some(vector(255, [0, 0]))],
            [&[], &[]], [&[], &[]], false).unwrap(), [None; 2]);
    }
    #[test]
    fn rounding_exhaustively_handles_signed_quarter_samples_without_overflow() {
        let refs = [id(0, 0)];
        for v in i16::MIN..=i16::MAX {
            let result = candidate([Some(vector(0, [v, v])), None],
                [&refs, &[]], [&refs, &[]], false).unwrap()[0].unwrap();
            let expected = (i32::from(v) + 2).div_euclid(4) as i16;
            assert_eq!(result.samples, [expected; 2]);
        }
        assert!(candidate([Some(vector(1, [0, 0])), None], [&refs, &[]], [&refs, &[]], false).is_err());
        assert!(candidate([None; 2], [&refs, &[]], [&[id(0, 0); 16], &[]], false).is_err());
    }
    #[test]
    fn aligned_centre_and_bad_geometry() {
        assert_eq!(texture_location([4, 12, 8, 16], [32, 32]).unwrap(), [8, 16]);
        assert_eq!(texture_location([0, 0, 4, 8], [8, 8]).unwrap(), [0, 0]);
        for rect in [[0, 0, 0, 8], [8, 0, 1, 1], [u32::MAX, 0, 2, 1]] {
            assert!(texture_location(rect, [8, 8]).is_err());
        }
    }
    #[test]
    fn raster_fallback_preserves_partial_list_and_requires_default() {
        let default = [Some(Vector { reference: 0, samples: [3, -2] }), None];
        let l1 = [None, Some(Vector { reference: 1, samples: [-1, 5] })];
        let mut visits = Vec::new();
        let result = subblocks([0, 0, 16, 16], [16, 16], 3, |p| {
            visits.push(p);
            Ok(match p { [8, 8] => default, [8, 0] => l1, _ => [None; 2] })
        }).unwrap();
        assert_eq!(visits, [[8, 8], [0, 0], [8, 0], [0, 8], [8, 8]]);
        assert_eq!(result.motion, [default, l1, default, default]);
        let mut visits = 0;
        let unavailable = subblocks([0, 0, 16, 16], [16, 16], 3, |_| {
            visits += 1; Ok([None; 2])
        }).unwrap();
        assert_eq!(visits, 1);
        assert!(unavailable.motion.is_empty());
    }
    #[test]
    fn nondivisible_block_stays_whole_and_errors_propagate() {
        let value = [Some(Vector { reference: 0, samples: [0; 2] }), None];
        let result = subblocks([0, 0, 12, 16], [16, 16], 3, |_| Ok(value)).unwrap();
        assert_eq!(result.size, [12, 16]);
        assert_eq!(result.motion, [value]);
        assert!(subblocks([0, 0, 8, 8], [8, 8], 2, |_| Ok(value)).is_err());
        assert!(subblocks([0, 0, 128, 8], [128, 8], 3, |_| Ok(value)).is_err());
        assert!(subblocks([0, 0, 8, 8], [8, 8], 3, |_| Err(invalid("test source failure"))).is_err());
    }
}

#[cfg(test)]
mod integer_prediction_tests {
    use super::*;
    use crate::codec::{hevc_decoder::{LayeredAccessUnit, LayeredDpb, LayeredParameters},
        hevc_motion::{Reference, Vector as QuarterVector}, hevc_plane::Plane,
        hevc_slice::{Weight, Weights}};
    use std::sync::Arc;
    fn source(depth: u8) -> Arc<crate::codec::hevc_picture::Picture> {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/fixtures/playback-errors/hevc-wedge");
        let mut input = crate::container::mp4::Mp4Reader::open(
            std::io::Cursor::new(std::fs::read(root.join("cqt-exact-texture-motion.mp4")).unwrap()),
            Default::default()).unwrap();
        let params = LayeredParameters::from_configuration(&input.tracks()[0].configuration,64<<20).unwrap();
        let mut dpb = LayeredDpb::default(); let mut packet = Vec::new();
        input.read_packet(0,0,&mut packet).unwrap();
        let plan = LayeredAccessUnit::parse(&params,dpb.poc(),&packet,4,1,64<<20).unwrap();
        let (_,frames,_) = dpb.decode(plan,64<<20).unwrap();
        let original = &frames.iter().find(|f|f.layer_id==3).unwrap().picture;
        let mut picture = original.translated_pocs(0,64<<20).unwrap();
        picture.depth = [depth;2];
        picture.planes[0] = Plane::new(32,32,depth,1<<20).unwrap();
        let samples:Vec<_> = (0..32).flat_map(|y| (0..32).map(move |x|
            ((x*1259+y*1741+177) % (1u32<<depth)) as u16)).collect();
        picture.planes[0].reconstruct_inter([0,0,32,32],&samples).unwrap();
        Arc::new(picture)
    }
    fn lists(picture: Arc<crate::codec::hevc_picture::Picture>) -> [Vec<Reference>;2] {
        std::array::from_fn(|_| vec![Reference {layer_id:3,poc:0,long_term:false,picture:Some(picture.clone())}])
    }
    #[test]
    fn integer_path_matches_existing_interpolation_for_uni_bi_weights_and_all_supported_depths() {
        for depth in 8..=16 {
            let lists = lists(source(depth));
            for mv in [[0,0],[1,-2],[-7,3],[8191,-8192]] {
                for mask in 1..=3 {
                    let motion = std::array::from_fn(|l| (mask & (1<<l) != 0).then_some(Vector {reference:0,samples:mv}));
                    let quarter = motion.map(|v|v.map(|v|QuarterVector {reference:v.reference,mv:v.samples.map(|x|i64::from(x)*4)}));
                    let mut variants = vec![None];
                    for denominator in [0,1,7] { for high_precision_offsets in [false,true] {
                        variants.push(Some(Weights {high_precision_offsets,denominators:[denominator;2],
                            lists:[vec![Weight {values:[-3;3],offsets:[-7;3]}],
                                   vec![Weight {values:[5;3],offsets:[11;3]}]]}));
                    }}
                    for weights in variants {
                        let actual = predict(&lists,motion,[3,5,8,8],depth,weights.as_ref()).unwrap();
                        let expected = super::super::predict_with_chroma_format(&lists,quarter,[3,5,8,8],0,depth,
                            weights.as_ref(),&mut Vec::new(),0).unwrap();
                        assert_eq!(actual,expected,"depth{depth} mv{mv:?} mask{mask} weights{weights:?}");
                    }
                }
            }
        }
    }
    #[test]
    fn full_integer_range_and_rounded_positive_texture_extreme_do_not_wrap() {
        let picture = source(16); let lists = lists(picture.clone());
        for (mv,point) in [([8192,-8192],[31usize,0]),([i16::MAX,i16::MIN],[31,0]),
            ([i16::MIN,i16::MAX],[0,31])] {
            let value = predict(&lists,[Some(Vector {reference:0,samples:mv}),None],[0,0,8,8],16,None).unwrap();
            assert_eq!(value,vec![picture.planes[0].samples()[point[1]*32+point[0]];64]);
            let motion=[Some(Vector {reference:0,samples:mv}),None];
            assert_eq!(value,super::super::predict_with_chroma_format(&lists,quarter_motion(motion),
                [0,0,8,8],0,16,None,&mut Vec::new(),0).unwrap());
        }
        let identity = [Identity {poc:0,view:0}];
        let motion = candidate([Some(QuarterVector {reference:0,mv:[i64::from(i16::MAX),i64::from(i16::MIN)]}),None],
            [&identity,&[]],[&identity,&[]],false).unwrap();
        assert_eq!(motion[0].unwrap().samples,[8192,-8192]);
        assert!(predict(&lists,motion,[0,0,8,8],16,None).is_ok());
    }
    #[test]
    fn invalid_or_incomplete_sources_refuse_before_prediction() {
        let picture = source(8); let mut lists = lists(picture);
        let motion = [Some(Vector {reference:0,samples:[0;2]}),None];
        for rect in [[0,0,0,8],[0,0,65,8],[u32::MAX,0,8,8]] {
            assert!(predict(&lists,motion,rect,8,None).is_err());
        }
        assert!(predict(&lists,motion,[0,0,8,8],10,None).is_err());
        assert!(predict(&lists,[None;2],[0,0,8,8],8,None).is_err());
        assert!(predict(&lists,[Some(Vector {reference:1,samples:[0;2]}),None],[0,0,8,8],8,None).is_err());
        let bad_weights = Weights {high_precision_offsets:false,denominators:[8;2],lists:[vec![],vec![]]};
        assert!(predict(&lists,motion,[0,0,8,8],8,Some(&bad_weights)).is_err());
        let missing_weights = Weights {denominators:[0;2],..bad_weights};
        assert!(predict(&lists,motion,[0,0,8,8],8,Some(&missing_weights)).is_err());
        let mut incomplete = lists[0][0].picture.as_ref().unwrap().translated_pocs(0,1<<20).unwrap();
        incomplete.planes[0] = Plane::new(32,32,8,1<<20).unwrap();
        lists[0][0].picture = Some(Arc::new(incomplete));
        assert!(predict(&lists,motion,[0,0,8,8],8,None).is_err());
        lists[0][0].picture = None;
        assert!(predict(&lists,motion,[0,0,8,8],8,None).is_err());
    }
}
