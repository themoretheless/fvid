use fvid::{
    codec::{
        hevc_decoder::{LayeredAccessUnit, LayeredDpb, LayeredParameters},
        hevc_motion::{
            Reference, Vector,
            disparity::derivation::Vector as Disparity,
            inter_view_residual::{
                BoundSources, CurrentRps, PredictionInputs, predict_prepared, prepare_prediction,
                reference_targets,
            },
            texture::Identity,
        },
        hevc_reference_list::PictureIdentity,
    },
    container::mp4::Mp4Reader,
};
use std::io::Cursor;
#[test]
fn prepared_residual_predictions_use_bound_rps_and_native_picture_owners() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/hevc-wedge");
    for (name, source_inter) in [
        ("illumination-b-64-0-0-2-1--40-both.mp4", false),
        ("texture-refinement-b-1-0-bi-4.mp4", true),
    ] {
        let mut movie = Mp4Reader::open(
            Cursor::new(std::fs::read(root.join(name)).unwrap()),
            Default::default(),
        )
        .unwrap();
        let mut params =
            LayeredParameters::from_configuration(&movie.tracks()[0].configuration, 64 << 20)
                .unwrap();
        let mut dpb = LayeredDpb::default();
        let mut packet = Vec::new();
        for index in 0..3 {
            movie.read_packet(0, index, &mut packet).unwrap();
            let plan =
                LayeredAccessUnit::parse(&params, dpb.poc(), &packet, 4, 1, 64 << 20).unwrap();
            let snapshot = if index == 2 {
                let source = plan
                    .pictures()
                    .iter()
                    .find(|p| p.binding.sps.layer_id == 0)
                    .unwrap();
                Some(
                    CurrentRps::bind(
                        source,
                        0,
                        dpb.next_access_unit(),
                        &dpb.references().cloned().collect::<Vec<_>>(),
                        64 << 20,
                    )
                    .unwrap(),
                )
            } else {
                None
            };
            let header = (index == 2).then(|| {
                plan.pictures()
                    .iter()
                    .find(|p| p.binding.sps.layer_id == 5)
                    .unwrap()
                    .slices[0]
                    .clone()
            });
            let bindings: Vec<_> = plan.pictures().iter().map(|p| p.binding.clone()).collect();
            let (updated, frames, _) = dpb.decode(plan, 64 << 20).unwrap();
            params = updated;
            if index != 2 {
                continue;
            }
            let snapshot = snapshot.unwrap();
            let refs: Vec<Reference> = dpb.references().cloned().collect();
            let lookup = |layer, poc| {
                refs.iter()
                    .find(|r| r.layer_id == layer && r.poc == poc)
                    .unwrap()
                    .clone()
            };
            let lists = [
                vec![lookup(5, 0), lookup(0, 1), lookup(5, 2)],
                vec![lookup(5, 2), lookup(5, 0), lookup(0, 1)],
            ];
            let source = frames
                .iter()
                .find(|f| f.layer_id == 0)
                .unwrap()
                .picture
                .clone();
            let target = frames.iter().find(|f| f.layer_id == 5).unwrap();
            let graph = &target.binding.vps.extension.as_ref().unwrap().graph;
            let snapshots = [snapshot];
            let snapshot = &snapshots[0];
            let current = fvid::codec::hevc_inter_layer::PictureStatus {
                identity: PictureIdentity {
                    layer_id: 5,
                    poc: target.poc,
                },
                access_unit: target.access_unit,
                nal: target.nal,
                discardable: false,
            };
            let completed: Vec<_> = frames
                .iter()
                .filter(|p| p.layer_id < 5)
                .map(|p| {
                    (
                        fvid::codec::hevc_inter_layer::PictureStatus {
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
            let context = target.binding.context();
            let header = header.as_ref().unwrap();
            let components = fvid::codec::hevc_picture::ViewComponentPool::bind(
                &context, header, current, &bindings, false, &completed, 65536,
            )
            .unwrap();
            let sources =
                BoundSources::bind(&components, &context, header, current, &snapshots, &refs)
                    .unwrap();
            assert!(std::sync::Arc::ptr_eq(
                &sources.current_view(0).unwrap(),
                &source
            ));
            assert!(
                sources.temporal_view(0, 1).unwrap().is_none(),
                "a DPB picture outside the used source RPS must remain unavailable"
            );
            assert!(sources.current_view(65535).is_err());
            assert!(sources.temporal_view(65535, 0).is_err());
            assert!(
                BoundSources::bind(&components, &context, header, current, &[], &refs).is_err()
            );
            let repeated = [snapshot.clone(), snapshot.clone()];
            assert!(
                BoundSources::bind(&components, &context, header, current, &repeated, &refs)
                    .is_err()
            );
            let mut stale = current;
            stale.access_unit += 1;
            assert!(
                BoundSources::bind(&components, &context, header, stale, &snapshots, &refs)
                    .is_err()
            );
            let missing: Vec<_> = refs
                .iter()
                .filter(|r| r.layer_id != 0 || r.poc != 0)
                .cloned()
                .collect();
            assert!(
                BoundSources::bind(&components, &context, header, current, &snapshots, &missing)
                    .is_err()
            );

            let identities = lists.each_ref().map(|list| {
                list.iter()
                    .map(|r| Identity {
                        poc: r.poc,
                        view: graph.view_index(r.layer_id).unwrap(),
                    })
                    .collect::<Vec<_>>()
            });
            let targets = reference_targets(
                [&identities[0], &identities[1]],
                true,
                1,
                true,
                &[0],
                |view, poc| {
                    assert_eq!(view, 0);
                    Ok(sources.temporal_view(view, poc)?.is_some())
                },
            )
            .unwrap();
            let rect = [0, 0, 32, 32];
            let motion = [
                None,
                Some(Vector {
                    reference: 1,
                    mv: [4, 0],
                }),
            ];
            let inputs = PredictionInputs {
                lists: &lists,
                motion,
                targets: &targets,
                current_poc: 1,
                rect,
                disparity: Disparity {
                    reference_view: 0,
                    mv: [0, 0],
                },
            };
            let temporal = prepare_prediction(
                &inputs,
                1,
                |layer| graph.view_index(layer),
                |view| {
                    assert_eq!(view, 0);
                    sources.current_view(view)
                },
                |view, poc| {
                    assert_eq!(view, 0);
                    sources.temporal_view(view, poc)
                },
            )
            .unwrap();
            assert_eq!(
                temporal.base_identity(),
                PictureIdentity {
                    layer_id: 5,
                    poc: 2
                }
            );
            assert_eq!(temporal.motion(), [-4, 0]);
            assert!(temporal.has_residual());
            assert_eq!(temporal.residual_motion(), Some([[0, 0], [-4, 0]]));
            let own_future = lookup(5, 2);
            let source_future = lookup(0, 2);
            let sample = |r: &Reference, x: usize, y: usize| {
                i32::from(
                    r.picture.as_ref().unwrap().planes[0].samples()
                        [y * r.picture.as_ref().unwrap().dimensions[0] as usize + x],
                )
            };
            for weight in [1, 2] {
                let pixels = temporal.predict(rect, 0, 1, weight, 65536).unwrap();
                for y in 0usize..32 {
                    for x in 0usize..32 {
                        let sx = x.saturating_sub(1);
                        let current = i32::from(
                            source.planes[0].samples()[y * source.dimensions[0] as usize + x],
                        );
                        let wanted = sample(&own_future, sx, y) * 64
                            + ((current * 64 - sample(&source_future, sx, y) * 64) >> (weight - 1));
                        assert_eq!(
                            pixels[y * 32 + x],
                            wanted,
                            "{name}: temporal c0 {x},{y} weight{weight}"
                        );
                    }
                }
            }
            assert!(temporal.predict(rect, 0, 1, 1, 4096).is_err());
            let iv = PredictionInputs {
                motion: [
                    Some(Vector {
                        reference: 1,
                        mv: [0, 0],
                    }),
                    None,
                ],
                ..inputs
            };
            let prepared = prepare_prediction(
                &iv,
                0,
                |layer| graph.view_index(layer),
                |_| panic!("inter-view path uses the selected list's temporal picture"),
                |view, poc| {
                    assert_eq!(view, 0);
                    sources.temporal_view(view, poc)
                },
            )
            .unwrap();
            let dy = if source_inter { 4 } else { 0 };
            assert_eq!(prepared.residual_motion(), Some([[0, dy], [0, dy]]));
            let own_past = lookup(5, 0);
            let source_past = lookup(0, 0);
            let pixels = prepared.predict(rect, 0, 1, 1, 65536).unwrap();
            for y in 0usize..32 {
                for x in 0usize..32 {
                    let yy = (y + if source_inter { 1 } else { 0 })
                        .min(source.dimensions[1] as usize - 1);
                    let wanted = i32::from(
                        source.planes[0].samples()[y * source.dimensions[0] as usize + x],
                    ) * 64
                        + (sample(&own_past, x, yy) - sample(&source_past, x, yy)) * 64;
                    assert_eq!(pixels[y * 32 + x], wanted, "{name}: inter-view c0 {x},{y}");
                }
            }
            let mixed = PredictionInputs {
                motion: [
                    Some(Vector {
                        reference: 1,
                        mv: [0, 0],
                    }),
                    Some(Vector {
                        reference: 1,
                        mv: [4, 0],
                    }),
                ],
                ..iv
            };
            let mixed_plan = prepare_prediction(
                &mixed,
                0,
                |layer| graph.view_index(layer),
                |_| panic!("mixed IV uses opposite temporal list"),
                |view, poc| {
                    assert_eq!(view, 0);
                    sources.temporal_view(view, poc)
                },
            )
            .unwrap();
            assert_eq!(mixed_plan.residual_motion(), Some([[-4, 0], [-4, 0]]));
            let pixels = mixed_plan.predict(rect, 0, 1, 1, 65536).unwrap();
            for y in 0usize..32 {
                for x in 0usize..32 {
                    let sx = x.saturating_sub(1);
                    let wanted = i32::from(
                        source.planes[0].samples()[y * source.dimensions[0] as usize + x],
                    ) * 64
                        + (sample(&own_future, sx, y) - sample(&source_future, sx, y)) * 64;
                    assert_eq!(pixels[y * 32 + x], wanted, "{name}: mixed lists {x},{y}");
                }
            }
            let both_iv = PredictionInputs {
                motion: [
                    Some(Vector {
                        reference: 1,
                        mv: [0, 0],
                    }),
                    Some(Vector {
                        reference: 2,
                        mv: [0, 0],
                    }),
                ],
                ..iv
            };
            let second = prepare_prediction(
                &both_iv,
                1,
                |layer| graph.view_index(layer),
                |_| unreachable!(),
                |view, poc| {
                    assert_eq!(view, 0);
                    sources.temporal_view(view, poc)
                },
            )
            .unwrap();
            assert_eq!(second.residual_motion(), prepared.residual_motion());
            assert_eq!(
                second.predict(rect, 0, 1, 1, 65536).unwrap(),
                prepared.predict(rect, 0, 1, 1, 65536).unwrap()
            );
            // Finish actual decoded-owner residual predictions without clipping
            // either signed list before their weighted combination.
            for component in 0..3 {
                for residual_weight in [1, 2] {
                    let a = prepared
                        .predict(rect, component, 1, residual_weight, 65536)
                        .unwrap();
                    let b = temporal
                        .predict(rect, component, 1, residual_weight, 65536)
                        .unwrap();
                    let weights = fvid::codec::hevc_slice::Weights {
                        high_precision_offsets: false,
                        denominators: [2, 1],
                        lists: [
                            vec![
                                fvid::codec::hevc_slice::Weight {
                                    values: [0; 3],
                                    offsets: [0; 3],
                                },
                                fvid::codec::hevc_slice::Weight {
                                    values: [-3; 3],
                                    offsets: [7; 3],
                                },
                            ],
                            vec![
                                fvid::codec::hevc_slice::Weight {
                                    values: [0; 3],
                                    offsets: [0; 3],
                                },
                                fvid::codec::hevc_slice::Weight {
                                    values: [5; 3],
                                    offsets: [-4; 3],
                                },
                            ],
                        ],
                    };
                    for weighted in [None, Some(&weights)] {
                        for active in [0, 1, 2] {
                            let plans = match active {
                                0 => [Some((&prepared, 1)), None],
                                1 => [None, Some((&temporal, 1))],
                                _ => [Some((&prepared, 1)), Some((&temporal, 1))],
                            };
                            let output = predict_prepared(
                                plans,
                                component,
                                1,
                                residual_weight,
                                weighted,
                                65536,
                            )
                            .unwrap();
                            let expected: Vec<u16> = a
                                .iter()
                                .zip(&b)
                                .map(|(&a, &b)| {
                                    let (a, b) = (i64::from(a), i64::from(b));
                                    let shift = if component == 0 { 8 } else { 7 };
                                    let round = 1 << (shift - 1);
                                    let value = match (weighted.is_some(), active) {
                                        (false, 0) => (a + 32) >> 6,
                                        (false, 1) => (b + 32) >> 6,
                                        (false, _) => (a + b + 64) >> 7,
                                        (true, 0) => ((-3 * a + round) >> shift) + 7,
                                        (true, 1) => ((5 * b + round) >> shift) - 4,
                                        (true, _) => (-3 * a + 5 * b + (4 << shift)) >> (shift + 1),
                                    };
                                    value.clamp(0, 255) as u16
                                })
                                .collect();
                            assert_eq!(
                                output, expected,
                                "{name} weight{residual_weight} active{active}"
                            );
                            assert!(
                                predict_prepared(plans, component, 1, residual_weight, weighted, 0)
                                    .is_err()
                            );
                        }
                    }
                    assert!(
                        predict_prepared(
                            [Some((&prepared, 2)), None],
                            0,
                            1,
                            residual_weight,
                            Some(&weights),
                            65536
                        )
                        .is_err()
                    );
                }
            }
            let small = PredictionInputs {
                rect: [0, 0, 8, 8],
                ..iv
            };
            let small_plan = prepare_prediction(
                &small,
                0,
                |layer| graph.view_index(layer),
                |_| unreachable!(),
                |view, poc| {
                    assert_eq!(view, 0);
                    sources.temporal_view(view, poc)
                },
            )
            .unwrap();
            let pixels = small_plan.predict(small.rect, 1, 1, 1, 64).unwrap();
            let stride = source.planes[1].dimensions()[0];
            for y in 0..4 {
                for x in 0..4 {
                    assert_eq!(
                        pixels[y * 4 + x],
                        i32::from(source.planes[1].samples()[y * stride + x]) * 64,
                        "small chroma must retain bilinear base only"
                    );
                }
            }
            let only_l1 = PredictionInputs {
                motion: [
                    None,
                    Some(Vector {
                        reference: 2,
                        mv: [0, 0],
                    }),
                ],
                ..iv
            };
            let l1_plan = prepare_prediction(
                &only_l1,
                1,
                |layer| graph.view_index(layer),
                |_| unreachable!(),
                |view, poc| {
                    assert_eq!(view, 0);
                    sources.temporal_view(view, poc)
                },
            )
            .unwrap();
            if source_inter {
                assert_eq!(l1_plan.residual_motion(), prepared.residual_motion());
                assert_eq!(
                    l1_plan.predict(rect, 0, 1, 1, 65536).unwrap(),
                    prepared.predict(rect, 0, 1, 1, 65536).unwrap()
                );
            } else {
                assert_eq!(l1_plan.residual_motion(), Some([[0, 0], [0, 0]]));
                let pixels = l1_plan.predict(rect, 0, 1, 1, 65536).unwrap();
                for y in 0usize..32 {
                    for x in 0usize..32 {
                        let wanted = i32::from(
                            source.planes[0].samples()[y * source.dimensions[0] as usize + x],
                        ) * 64
                            + (sample(&own_future, x, y) - sample(&source_future, x, y)) * 64;
                        assert_eq!(
                            pixels[y * 32 + x],
                            wanted,
                            "L1-only zero fallback uses L1 temporal target"
                        );
                    }
                }
            }
            let mut invalid_target = targets.clone();
            invalid_target.temporal[1] = Some(
                fvid::codec::hevc_motion::inter_view_residual::TemporalTarget {
                    reference: 2,
                    poc: 1,
                },
            );
            let malformed = PredictionInputs {
                targets: &invalid_target,
                ..inputs
            };
            assert!(
                prepare_prediction(
                    &malformed,
                    1,
                    |_| unreachable!(),
                    |_| unreachable!(),
                    |_, _| unreachable!()
                )
                .is_err(),
                "a current-POC entry cannot masquerade as a temporal target"
            );
            let gated = reference_targets(
                [&identities[0], &identities[1]],
                true,
                1,
                true,
                &[0],
                |_, _| Ok(false),
            )
            .unwrap();
            let unavailable = PredictionInputs {
                targets: &gated,
                ..iv
            };
            let plan = prepare_prediction(
                &unavailable,
                0,
                |layer| graph.view_index(layer),
                |_| panic!("unavailable source must not be fetched"),
                |_, _| panic!("unavailable RPS must not be fetched"),
            )
            .unwrap();
            assert!(!plan.has_residual());
            assert!(
                prepared.predict([0, 0, 8, 8], 0, 1, 1, 65536).is_err(),
                "prepared CU binding must not be changed after motion derivation"
            );
        }
    }
}
