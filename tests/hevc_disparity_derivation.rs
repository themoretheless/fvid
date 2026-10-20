//! Derive temporal disparity from coded native motion and parsed active lists.
//! The movie does not enable coded IV/DI/VSP tool admission.
use fvid::{
    codec::{
        hevc_decoder::{LayeredAccessUnit, LayeredDecoded, LayeredParameters},
        hevc_inter_layer::PictureStatus,
        hevc_motion::disparity::{self, derivation},
        hevc_poc::LayeredPocState,
        hevc_reference_list::PictureIdentity,
    },
    container::mp4::{Limits, Mp4Reader},
    playback::video::mp4::LayeredMp4VideoReader,
};
use std::io::Cursor;
#[test]
fn coded_temporal_disparity_uses_parsed_collocated_selection_and_reference_views() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/hevc-wedge/inter-view-temporal-scaling-on.mp4");
    let data = std::fs::read(path).unwrap();
    let mut input = Mp4Reader::open(Cursor::new(data.clone()), Limits::default()).unwrap();
    let parameters =
        LayeredParameters::from_configuration(&input.tracks()[0].configuration, 64 << 20).unwrap();
    let mut packet = Vec::new();
    input.read_packet(0, 0, &mut packet).unwrap();
    let first = LayeredAccessUnit::parse(
        &parameters,
        &LayeredPocState::default(),
        &packet,
        4,
        1,
        64 << 20,
    )
    .unwrap();
    let (parameters, transition, _, _) = first.into_parts();
    let state = transition.unwrap().into_state();
    input.read_packet(0, 1, &mut packet).unwrap();
    let planned = LayeredAccessUnit::parse(&parameters, &state, &packet, 4, 1, 64 << 20).unwrap();
    let current = planned
        .pictures()
        .iter()
        .find(|p| p.binding.sps.layer_id == 5)
        .unwrap();
    let header = &current.slices[0];
    let graph = &current.binding.vps.extension.as_ref().unwrap().graph;
    let mut reader =
        LayeredMp4VideoReader::open(Cursor::new(data), Limits::default(), 1, 0, 64 << 20).unwrap();
    for _ in 0..2 {
        let mut sources: Vec<LayeredDecoded> = Vec::new();
        while let Some(frame) = reader.read_frame().unwrap() {
            if frame.decoded.poc == 1 && frame.decoded.layer_id < 5 {
                sources.push(frame.decoded);
            }
        }
        assert_eq!(sources.len(), 2);
        let completed: Vec<_> = sources
            .iter()
            .map(|p| {
                (
                    PictureStatus {
                        identity: PictureIdentity {
                            layer_id: p.layer_id,
                            poc: p.poc,
                        },
                        access_unit: p.access_unit,
                        nal: p.nal,
                        discardable: false,
                    },
                    p.picture.clone(),
                )
            })
            .collect();
        let (_, lists) = current
            .reference_lists(0, sources[0].access_unit, &[], &completed, 64 << 20)
            .unwrap();
        let views: [Vec<u16>; 2] = [0, 1].map(|list| {
            lists[list]
                .iter()
                .map(|r| graph.view_index(r.layer_id).unwrap())
                .collect()
        });
        let metadata: [Vec<disparity::CandidatePicture>; 2] = [0, 1].map(|list| {
            lists[list]
                .iter()
                .map(|r| {
                    let source = sources
                        .iter()
                        .find(|p| p.layer_id == r.layer_id && p.poc == r.poc)
                        .unwrap();
                    assert_eq!(source.picture.nal_header(), source.nal);
                    r.disparity_metadata(graph).unwrap()
                })
                .collect()
        });
        let candidates = disparity::candidate_pictures(
            [&metadata[0], &metadata[1]],
            false,
            header.temporal_mvp,
            header.collocated_list == 0,
            header.collocated_ref as usize,
            2,
            current.poc,
        )
        .unwrap();
        assert_eq!(candidates, [(0, 1)]);
        let binding = current.binding.context();
        let prepared =
            derivation::PreparedTexture::prepare(&binding, header, current.poc, &lists).unwrap();
        assert_eq!(prepared.candidate_positions(), candidates);
        assert_eq!(prepared.default_view(), Some(0));
        let mut wrong_lists = lists.clone();
        wrong_lists[0].pop();
        assert!(
            derivation::PreparedTexture::prepare(&binding, header, current.poc, &wrong_lists)
                .is_err()
        );
        wrong_lists = lists.clone();
        wrong_lists[0][1].layer_id = 0;
        assert!(
            derivation::PreparedTexture::prepare(&binding, header, current.poc, &wrong_lists)
                .is_err()
        );
        wrong_lists = lists.clone();
        wrong_lists[0][1].picture = None;
        assert!(
            derivation::PreparedTexture::prepare(&binding, header, current.poc, &wrong_lists)
                .is_err()
        );
        let mut temporal = None;
        let prepared_temporal = prepared.temporal([0, 0], 32).unwrap();
        for (list, index) in candidates {
            let reference = &lists[list][index];
            temporal = derivation::temporal_from_picture(
                reference.picture.as_ref().unwrap(),
                [0, 0],
                32,
                graph.view_index(reference.layer_id).unwrap(),
                [&views[0], &views[1]],
                |layer| graph.view_index(layer),
            )
            .unwrap();
            if temporal.is_some() {
                break;
            }
        }
        assert_eq!(prepared_temporal, temporal);
        let derived = prepared
            .derive([0, 0], 32, false, None, 0, |_| {
                panic!("bound temporal source must precede spatial lookup")
            })
            .unwrap()
            .unwrap();
        assert_eq!(derived.vector, temporal.unwrap());
        assert_eq!(derived.refined_mv, derived.vector.mv);
        assert!(
            prepared
                .derive([1, 0], 32, false, None, 0, |_| Ok(None))
                .is_err()
        );
        assert_eq!(
            temporal,
            Some(derivation::Vector {
                reference_view: 0,
                mv: [0, 4]
            })
        );
        let default =
            disparity::default_reference_view([&metadata[0], &metadata[1]], false, 2, current.poc);
        assert_eq!(default, Some(0));
        let vector = derivation::texture(
            [0, 0],
            32,
            current.poc,
            false,
            2,
            5,
            temporal,
            default,
            |_| panic!("temporal disparity must precede spatial lookup"),
        )
        .unwrap()
        .unwrap();
        assert_eq!(vector, temporal.unwrap());
        assert_eq!(
            derivation::refine(vector, None, None, [0, 0], 32, 0)
                .unwrap()
                .refined_mv,
            [0, 4]
        );
        let source = &sources[1];
        assert_eq!(source.layer_id, 3);
        assert!(
            derivation::temporal_from_picture(
                &source.picture,
                [0, 0],
                32,
                1,
                [&[1], &[]],
                |layer| graph.view_index(layer)
            )
            .unwrap()
            .is_none()
        );
        reader.rewind();
    }
}

#[test]
fn spatial_priority_and_boundaries_match_independent_outcome_oracle() {
    use derivation::{Neighbour, NeighbourVector, Vector};
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/hevc-wedge/disparity-derivation-oracles.json");
    let oracle: serde_json::Value = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
    let cases = oracle["cases"].as_array().unwrap();
    assert_eq!(cases.len(), 1152);
    for case in cases {
        let g = case[0].as_u64().unwrap() as usize;
        let geometry = [0, 1, 2, 3, 4].map(|i| oracle["geometries"][g][i].as_u64().unwrap() as u32);
        let [x, y, size, merge_log, ctu_log] = geometry;
        let left = case[1].as_u64().unwrap();
        let top = case[2].as_u64().unwrap();
        let merge = case[3].as_bool().unwrap();
        let temporal = case[4].as_bool().unwrap().then_some(Vector {
            reference_view: 5,
            mv: [90, 91],
        });
        let default = case[5].as_bool().unwrap().then_some(0);
        let result = derivation::texture(
            [x, y],
            size,
            10,
            merge,
            merge_log as u8,
            ctu_log as u8,
            temporal,
            default,
            |point| {
                if point.iter().any(|&n| n < 0) {
                    return Ok(None);
                }
                let kind = if point[0] == i64::from(x) - 1 {
                    left
                } else {
                    top
                };
                let vector = |reference_view, poc, mv| {
                    Some(NeighbourVector {
                        reference_view,
                        poc,
                        mv,
                    })
                };
                let motion = match kind {
                    0 => return Ok(None),
                    1 => [vector(1, 10, [1, 2]), None],
                    2 => [vector(1, 9, [1, 2]), vector(2, 10, [3, 4])],
                    3 | 4 => [vector(1, 9, [1, 2]), None],
                    5 => [None; 2],
                    _ => unreachable!(),
                };
                Ok(Some(Neighbour {
                    motion,
                    skip: kind >= 4,
                    inter_view: kind >= 4,
                    refined: (kind >= 4).then_some(Vector {
                        reference_view: 3,
                        mv: [7, 8],
                    }),
                }))
            },
        )
        .unwrap();
        let expected = if case[6].is_null() {
            None
        } else {
            Some(Vector {
                reference_view: case[6][0].as_u64().unwrap() as u16,
                mv: [case[6][1].as_i64().unwrap(), case[6][2].as_i64().unwrap()],
            })
        };
        assert_eq!(result, expected, "{case}");
    }
}

#[test]
fn backward_depth_refinement_preserves_base_vector_and_budget() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/hevc-wedge/disparity-depth-source.mp4");
    let mut reader = LayeredMp4VideoReader::open(
        Cursor::new(std::fs::read(path).unwrap()),
        Limits::default(),
        1,
        0,
        64 << 20,
    )
    .unwrap();
    let mut depth = None;
    while let Some(frame) = reader.read_frame().unwrap() {
        if frame.decoded.layer_id == 3 && frame.decoded.poc == 1 {
            depth = Some(frame.decoded.picture.clone());
        }
    }
    let depth = depth.unwrap();
    let camera = disparity::CameraTransform::new([8, -1, 0, 0], 2, 8).unwrap();
    let budget = disparity::samples::required_bytes([8, 8]).unwrap();
    // Independent authored depth formula gives corner maxima 174,122,254;
    // this camera's backward transform is floor(depth / 64).
    for (mv, expected) in [([0, 4], 2), ([-129, 125], 1), ([i64::MAX, i64::MIN], 3)] {
        let vector = derivation::Vector {
            reference_view: 0,
            mv,
        };
        let result =
            derivation::refine(vector, Some(&depth), Some(camera), [0, 0], 8, budget).unwrap();
        assert_eq!(result.vector, vector);
        assert_eq!(result.refined_mv, [expected, 0]);
        assert!(
            derivation::refine(vector, Some(&depth), Some(camera), [0, 0], 8, budget - 1).is_err()
        );
        assert!(derivation::refine(vector, Some(&depth), None, [0, 0], 8, budget).is_err());
    }
    assert_eq!(derivation::depth(0, camera).unwrap().mv, [2, 0]);
}
