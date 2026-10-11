use super::tests::{synthetic_decoder, synthetic_picture};
use super::*;
use crate::codec::hevc_plane::Plane;
#[test]
fn layer_identity_preserves_canonical_motion_pixels_and_boundaries() {
    use crate::codec::hevc_motion::{Vector, predict, remap_references};
    use crate::codec::hevc_picture::{canonical_references, motion_boundary};
    let atlas =
        include_bytes!("../../tests/fixtures/playback-errors/hevc-motion-layer-identity.y4m");
    let magic = b"YUV4MPEG2 W64 H32 F25:1 Ip A1:1 C420\n";
    assert!(atlas.starts_with(magic));
    let source = synthetic_picture();
    let mut at = magic.len();
    let references: Vec<_> = [(0, 100), (2, 100), (5, 99)]
        .into_iter()
        .map(|(layer_id, poc)| {
            assert_eq!(&atlas[at..at + 6], b"FRAME\n");
            at += 6;
            let mut picture = source.translated_pocs(0, 16 << 20).unwrap();
            picture.dimensions = [64, 32];
            picture.depth = [8; 2];
            picture.planes = std::array::from_fn(|c| {
                let (w, h) = if c == 0 { (64, 32) } else { (32, 16) };
                let mut plane = Plane::new(w, h, 8, 16 << 20).unwrap();
                for sample in plane.samples_mut() {
                    *sample = u16::from(atlas[at]);
                    at += 1;
                }
                plane
            });
            Reference {
                layer_id,
                poc,
                long_term: true,
                picture: Some(Arc::new(picture)),
            }
        })
        .collect();
    assert_eq!(at, atlas.len());
    let slices = [
        [vec![references[0].clone(), references[1].clone()], vec![]],
        [
            vec![
                references[1].clone(),
                references[0].clone(),
                references[2].clone(),
            ],
            vec![],
        ],
    ];
    let canonical = canonical_references(&slices).unwrap();
    assert_eq!(
        canonical[0]
            .iter()
            .map(|r| (r.layer_id, r.poc))
            .collect::<Vec<_>>(),
        vec![(0, 100), (2, 100), (5, 99)]
    );
    let ids = |lists: &[Vec<Reference>; 2]| {
        lists
            .each_ref()
            .map(|l| l.iter().map(Reference::identity).collect::<Vec<_>>())
    };
    let motion = [
        Some(Vector {
            reference: 0,
            mv: [0, 0],
        }),
        None,
    ];
    let remapped = remap_references(motion, &ids(&slices[1]), &ids(&canonical)).unwrap();
    assert_eq!(remapped[0].unwrap().reference, 1);
    assert_eq!(
        predict(
            &canonical,
            remapped,
            [0, 0, 8, 8],
            0,
            8,
            None,
            &mut Vec::new()
        )
        .unwrap(),
        vec![97; 64]
    );
    assert!(motion_boundary(motion, remapped, &canonical));
    assert!(!motion_boundary(remapped, remapped, &canonical));
    // Header is a parsed owned stream; this harness exercises reference
    // identity operations, not entropy-coded scalable picture admission.
    use crate::codec::{
        config::NalUnits,
        hevc_inter_syntax::{Partition, Prediction},
        hevc_motion::Spatial,
        hevc_nal::NalHeader,
        hevc_slice::SliceHeader,
    };
    let (decoder, packet) = synthetic_decoder();
    let nal = NalUnits::new(&packet, 4)
        .unwrap()
        .map(Result::unwrap)
        .find(|n| NalHeader::parse(n).unwrap().is_vcl())
        .unwrap();
    let (sps, pps) = decoder.parameters();
    let mut header = SliceHeader::parse(nal, sps, pps, 16 << 20).unwrap();
    header.temporal_mvp = false;
    header.use_integer_mv = false;
    let spatial = Spatial {
        rect: [8, 8, 8, 8],
        cu: [8, 8, 8],
        partition: Partition::Full,
        part_index: 0,
        merge_log2: 2,
        ctu_log2: 6,
        poc: 101,
        lists: &canonical,
    };
    let selected = spatial
        .resolve(
            Prediction::Explicit {
                references: [Some(1), None],
                differences: [[0, 0]; 2],
                predictors: [0, 0],
            },
            &header,
            |x, y| match (x, y) {
                (7, 16) => Some([
                    Some(Vector {
                        reference: 0,
                        mv: [4, 0],
                    }),
                    None,
                ]),
                (7, 15) => Some([
                    Some(Vector {
                        reference: 1,
                        mv: [12, 0],
                    }),
                    None,
                ]),
                _ => None,
            },
        )
        .unwrap();
    // Annex F.8.5 retains the POC-based direct AMVP rule.
    assert_eq!(selected[0].unwrap().mv, [4, 0]);
    let mut conflicting = slices.clone();
    conflicting[1][0][0].long_term = false;
    assert!(
        canonical_references(&conflicting)
            .err()
            .unwrap()
            .to_string()
            .contains("conflicting classification")
    );
}

#[test]
fn layered_translation_shifts_each_nested_reference_by_its_saved_layer() {
    let source = synthetic_picture();
    let mut picture = source.translated_pocs(0, 16 << 20).unwrap();
    picture.reference_pocs = [vec![356, 356, 355], vec![]];
    picture.reference_layers = [vec![0, 2, 5], vec![]];
    picture.reference_long_term = [vec![false, true, true], vec![]];
    for child in picture.colour_planes.as_mut().unwrap() {
        let mut cloned = child.translated_pocs(0, 16 << 20).unwrap();
        cloned.reference_pocs = picture.reference_pocs.clone();
        cloned.reference_layers = picture.reference_layers.clone();
        cloned.reference_long_term = picture.reference_long_term.clone();
        *child = Arc::new(cloned);
    }
    let mut deltas = [0; 63];
    deltas[0] = 256;
    deltas[5] = 128;
    let shifted = picture.translated_layer_pocs(&deltas, 16 << 20).unwrap();
    assert_eq!(shifted.reference_pocs[0], vec![100, 356, 227]);
    assert_eq!(picture.reference_pocs[0], vec![356, 356, 355]);
    for child in shifted.colour_planes.as_ref().unwrap() {
        assert_eq!(child.reference_pocs[0], vec![100, 356, 227]);
    }
    assert_eq!(
        shifted.planes[0].samples().as_ptr(),
        picture.planes[0].samples().as_ptr()
    );
    picture.reference_layers[0][1] = 63;
    assert!(
        picture
            .translated_layer_pocs(&deltas, 16 << 20)
            .err()
            .unwrap()
            .to_string()
            .contains("layer out of range")
    );
}
