//! Whole-AU bindings/POC and actual pixels. Stateful DPB/player ownership is separate.
use fvid::codec::{
    config::{HevcConfig, NalUnits},
    hevc_decoder::{LayeredAccessUnit, LayeredParameters},
    hevc_inter_layer::PictureStatus,
    hevc_motion::Reference,
    hevc_nal::NalHeader,
    hevc_picture::{Picture, decode_layered},
    hevc_poc::LayeredPocState,
    hevc_reference_list::PictureIdentity,
};
use fvid::container::mp4::Mp4Reader;
use std::{
    io::Cursor,
    path::{Path, PathBuf},
    sync::Arc,
};
const BUDGET: usize = 32 << 20;
fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/playback-errors")
}
fn movie(name: &str) -> Mp4Reader<Cursor<Vec<u8>>> {
    Mp4Reader::open(
        Cursor::new(std::fs::read(root().join(name)).unwrap()),
        Default::default(),
    )
    .unwrap()
}
fn manifest(name: &str) -> serde_json::Value {
    serde_json::from_slice(&std::fs::read(root().join(name)).unwrap()).unwrap()
}
fn assert_pixels(picture: &Picture, oracle: &[u8], index: usize) {
    assert_eq!(picture.depth, [8; 2]);
    let size: usize = picture.planes.iter().map(|p| p.samples().len()).sum();
    let mut at = index * size;
    for plane in &picture.planes {
        let n = plane.samples().len();
        assert_eq!(
            plane.samples(),
            oracle[at..at + n]
                .iter()
                .map(|&v| u16::from(v))
                .collect::<Vec<_>>()
        );
        at += n;
    }
}

#[test]
fn all_original_layered_i_pictures_bind_and_reconstruct_as_complete_access_units() {
    let m = manifest("hevc-layered-reconstruction.json");
    assert_eq!(m["cases"].as_array().unwrap().len(), 10);
    for case in m["cases"].as_array().unwrap() {
        let file = case["file"].as_str().unwrap();
        let mut input = movie(file);
        let config_bytes = input.tracks()[0].configuration.clone();
        let config = HevcConfig::parse(&config_bytes).unwrap();
        let original = LayeredParameters::from_configuration(&config_bytes, BUDGET).unwrap();
        let state = LayeredPocState::default();
        let before = state.clone();
        let mut packet = vec![];
        input.read_packet(0, 0, &mut packet).unwrap();
        let plan =
            LayeredAccessUnit::parse(&original, &state, &packet, config.length_size, 1, BUDGET)
                .unwrap_or_else(|e| panic!("{file}: {e}"));
        assert_eq!(state, before);
        assert!(original.active(0).is_none() && original.active(3).is_none());
        assert_eq!(
            plan.pictures()
                .iter()
                .map(|p| p.binding.sps.layer_id)
                .collect::<Vec<_>>(),
            [0, 3]
        );
        let oracle = std::fs::read(
            root()
                .join(case["seed"].as_str().unwrap())
                .with_extension("yuv"),
        )
        .unwrap();
        for picture in plan.pictures() {
            assert_eq!(picture.poc, 0, "{file}");
            let lists = vec![[vec![], vec![]]; picture.slices.len()];
            let decoded = decode_layered(
                &picture.binding.context(),
                &picture.slices,
                picture.poc,
                &lists,
                BUDGET,
            )
            .unwrap_or_else(|e| panic!("{file} layer{}: {e}", picture.binding.sps.layer_id));
            assert_pixels(&decoded, &oracle, 0);
        }
        let (parameters, transition, pictures, end) = plan.into_parts();
        let published = transition.unwrap().into_state();
        assert_eq!(published.previous(0), Some(0));
        assert_eq!(published.previous(3), Some(0));
        assert_eq!(pictures.len(), 2);
        assert!(parameters.active(0).is_some() && parameters.active(3).is_some());
        assert_eq!(end.layers, 0);
        assert!(!end.bitstream);
    }
}

#[test]
fn malformed_whole_au_witnesses_fail_precisely_without_publishing_partial_state() {
    let m = manifest("hevc-layered-access-units.json");
    let mut failures = 0;
    for case in m["cases"].as_array().unwrap() {
        let Some(expected) = case["error"].as_str() else {
            continue;
        };
        failures += 1;
        let file = case["file"].as_str().unwrap();
        let mut input = movie(file);
        let config_bytes = input.tracks()[0].configuration.clone();
        let config = HevcConfig::parse(&config_bytes).unwrap();
        let original = LayeredParameters::from_configuration(&config_bytes, BUDGET).unwrap();
        let state = LayeredPocState::default();
        let before = state.clone();
        let held = original.storage_bytes();
        let mut packet = vec![];
        input.read_packet(0, 0, &mut packet).unwrap();
        let error =
            LayeredAccessUnit::parse(&original, &state, &packet, config.length_size, 1, BUDGET)
                .err()
                .unwrap();
        assert_eq!(error.to_string(), expected, "{file}");
        assert_eq!(state, before, "{file}");
        assert_eq!(original.storage_bytes(), held, "{file}");
        assert!(
            original.active(0).is_none() && original.active(3).is_none(),
            "{file}"
        );
        let mut baseline = movie(case["baseline"].as_str().unwrap());
        baseline.read_packet(0, 0, &mut packet).unwrap();
        let recovery =
            LayeredAccessUnit::parse(&original, &state, &packet, config.length_size, 1, BUDGET)
                .unwrap_or_else(|e| panic!("{file}: failed to recover: {e}"));
        assert_eq!(recovery.pictures().len(), 2);
        assert!(recovery.pictures().iter().all(|p| p.poc == 0));
    }
    assert!(failures >= 12);
}

#[test]
fn legal_non_vcl_placement_shared_pps_and_reserved_payloads_preserve_all_pixels() {
    let m = manifest("hevc-layered-access-units.json");
    let mut accepted = 0;
    for case in m["cases"].as_array().unwrap() {
        if !case["error"].is_null() {
            continue;
        }
        accepted += 1;
        let file = case["file"].as_str().unwrap();
        let mut input = movie(file);
        let config_bytes = input.tracks()[0].configuration.clone();
        let config = HevcConfig::parse(&config_bytes).unwrap();
        let original = LayeredParameters::from_configuration(&config_bytes, BUDGET).unwrap();
        let state = LayeredPocState::default();
        let mut packet = vec![];
        input.read_packet(0, 0, &mut packet).unwrap();
        if case["ignored"].as_bool() == Some(true) {
            let ignored = NalUnits::new(&packet, config.length_size)
                .unwrap()
                .map(Result::unwrap)
                .filter(|n| NalHeader::parse(n).unwrap().layer_id == 63)
                .count();
            assert_eq!(ignored, 2);
        }
        let plan =
            LayeredAccessUnit::parse(&original, &state, &packet, config.length_size, 1, BUDGET)
                .unwrap_or_else(|e| panic!("{file}: {e}"));
        assert_eq!(plan.pictures().len(), 2, "{file}");
        assert_eq!(
            plan.end_markers().layers,
            case["ended"].as_u64().unwrap_or(0)
        );
        assert_eq!(
            plan.end_markers().bitstream,
            case["eob"].as_bool().unwrap_or(false)
        );
        if let Some(layer) = case["pps_nal_layer"].as_u64() {
            assert_eq!(
                plan.parameters().active(3).unwrap().pps.nal_layer_id,
                layer as u8
            );
            assert_eq!(
                original.clone().activate(3, 1, 1).unwrap().pps.nal_layer_id,
                3
            );
        }
        let oracle = std::fs::read(
            root()
                .join(case["seed"].as_str().unwrap())
                .with_extension("yuv"),
        )
        .unwrap();
        for picture in plan.pictures() {
            let lists = vec![[vec![], vec![]]; picture.slices.len()];
            let decoded = decode_layered(
                &picture.binding.context(),
                &picture.slices,
                picture.poc,
                &lists,
                BUDGET,
            )
            .unwrap();
            assert_pixels(&decoded, &oracle, 0);
        }
        assert!(original.active(0).is_none() && original.active(3).is_none());
    }
    assert_eq!(accepted, 6);
}

#[test]
fn sequential_complete_units_drive_actual_temporal_and_same_au_inter_prediction() {
    let mut input = movie("hevc-layered-reconstruct-inter-prediction.mp4");
    let config_bytes = input.tracks()[0].configuration.clone();
    let config = HevcConfig::parse(&config_bytes).unwrap();
    let initial = LayeredParameters::from_configuration(&config_bytes, BUDGET).unwrap();
    let oracle = std::fs::read(root().join("hevc-pcm-reference-rext8.yuv")).unwrap();
    for _ in 0..2 {
        let mut parameters = initial.clone();
        let mut state = LayeredPocState::default();
        let mut dpb: Vec<(PictureIdentity, Arc<Picture>)> = vec![];
        let mut packet = vec![];
        for index in 0..2 {
            input.read_packet(0, index, &mut packet).unwrap();
            let before = state.clone();
            let plan = LayeredAccessUnit::parse(
                &parameters,
                &state,
                &packet,
                config.length_size,
                1,
                BUDGET,
            )
            .unwrap();
            assert_eq!(state, before);
            let mut completed: Vec<(PictureStatus, Arc<Picture>)> = vec![];
            for picture in plan.pictures() {
                assert_eq!(picture.poc, index as i32);
                let mut all_lists = vec![];
                let temporal: Vec<_> = dpb
                    .iter()
                    .map(|(id, picture)| Reference {
                        layer_id: id.layer_id,
                        poc: id.poc,
                        long_term: false,
                        picture: Some(picture.clone()),
                    })
                    .collect();
                for slice in 0..picture.slices.len() {
                    let (_, lists) = picture
                        .reference_lists(slice, index as u64, &temporal, &completed, BUDGET)
                        .unwrap();
                    all_lists.push(lists);
                    assert!(
                        picture
                            .reference_lists(slice, index as u64, &temporal, &completed, 1)
                            .is_err()
                    );
                }
                assert_eq!(
                    picture
                        .reference_lists(
                            picture.slices.len(),
                            index as u64,
                            &temporal,
                            &completed,
                            BUDGET
                        )
                        .err()
                        .unwrap()
                        .to_string(),
                    "HEVC layered slice index out of range"
                );
                if !temporal.is_empty() {
                    let mut duplicate = temporal.clone();
                    duplicate.push(temporal[0].clone());
                    assert_eq!(
                        picture
                            .reference_lists(0, index as u64, &duplicate, &completed, BUDGET)
                            .err()
                            .unwrap()
                            .to_string(),
                        "invalid or duplicate HEVC temporal picture identity"
                    );
                }
                if index == 1 && picture.binding.sps.layer_id == 0 {
                    assert!(
                        picture
                            .reference_lists(0, index as u64, &[], &completed, BUDGET)
                            .err()
                            .unwrap()
                            .to_string()
                            .contains("reference POC 0 is missing")
                    );
                }
                let decoded = Arc::new(
                    decode_layered(
                        &picture.binding.context(),
                        &picture.slices,
                        picture.poc,
                        &all_lists,
                        BUDGET,
                    )
                    .unwrap(),
                );
                assert_pixels(&decoded, &oracle, index as usize);
                let first = &picture.slices[0];
                let identity = PictureIdentity {
                    layer_id: first.nal.layer_id,
                    poc: picture.poc,
                };
                completed.push((
                    PictureStatus {
                        identity,
                        access_unit: index as u64,
                        nal: first.nal,
                        discardable: first.layered.as_ref().unwrap().discardable,
                    },
                    decoded,
                ));
            }
            // Publish only after both base and enhancement pictures succeeded.
            let (next_parameters, transition, _, end) = plan.into_parts();
            assert_eq!(end.layers, 0);
            assert!(!end.bitstream);
            parameters = next_parameters;
            state = transition.unwrap().into_state();
            dpb.extend(
                completed
                    .into_iter()
                    .map(|(status, picture)| (status.identity, picture)),
            );
            assert_eq!(state.previous(0), Some(index as i32));
            assert_eq!(state.previous(3), Some(index as i32));
        }
    }
}

#[test]
fn metadata_only_and_budget_refusals_keep_source_poc_and_bindings_unchanged() {
    let input = movie("hevc-layered-reconstruct-pcm-dependent.mp4");
    let config_bytes = input.tracks()[0].configuration.clone();
    let config = HevcConfig::parse(&config_bytes).unwrap();
    let parameters = LayeredParameters::from_configuration(&config_bytes, BUDGET).unwrap();
    let state = LayeredPocState::default();
    let pps = config
        .arrays
        .iter()
        .find(|a| a.nal_type == 34)
        .unwrap()
        .units[0];
    let mut packet = (pps.len() as u32).to_be_bytes().to_vec();
    packet.extend_from_slice(pps);
    let plan = LayeredAccessUnit::parse(&parameters, &state, &packet, 4, 1, BUDGET).unwrap();
    assert!(plan.pictures().is_empty() && plan.transition().is_none());
    assert!(LayeredAccessUnit::parse(&parameters, &state, &packet, 4, 1, 1).is_err());
    assert!(LayeredAccessUnit::parse(&parameters, &state, &packet, 4, 1, 4096).is_err());
    assert!(parameters.active(0).is_none() && parameters.active(3).is_none());
    assert_eq!(state, LayeredPocState::default());
    assert_eq!(
        parameters
            .clone()
            .activate(63, 1, 1)
            .unwrap_err()
            .to_string(),
        "invalid HEVC active layer ID"
    );
}

#[test]
fn layered_dpb_publishes_all_layers_atomically_and_retries_after_reconstruction_failure() {
    use fvid::codec::hevc_decoder::LayeredDpb;
    let mut input = movie("hevc-layered-reconstruct-inter-prediction.mp4");
    let bytes = input.tracks()[0].configuration.clone();
    let config = HevcConfig::parse(&bytes).unwrap();
    let initial = LayeredParameters::from_configuration(&bytes, BUDGET).unwrap();
    let oracle = std::fs::read(root().join("hevc-pcm-reference-rext8.yuv")).unwrap();
    let mut dpb = LayeredDpb::default();
    for _ in 0..2 {
        let mut parameters = initial.clone();
        let mut packet = vec![];
        for index in 0..2 {
            input.read_packet(0, index, &mut packet).unwrap();
            let original_poc = dpb.poc().clone();
            let original_ids: Vec<_> = dpb.references().map(Reference::identity).collect();
            let parse = |state: &LayeredPocState| {
                LayeredAccessUnit::parse(&parameters, state, &packet, config.length_size, 1, BUDGET)
                    .unwrap()
            };
            let make_picture = |picture: &fvid::codec::hevc_decoder::LayeredAccessPicture,
                                temporal: &[Reference],
                                completed: &[(PictureStatus, Arc<Picture>)],
                                budget: usize| {
                let lists = (0..picture.slices.len())
                    .map(|slice| {
                        picture
                            .reference_lists(slice, index as u64, temporal, completed, budget)
                            .map(|(_, lists)| lists)
                    })
                    .collect::<fvid::Result<Vec<_>>>()?;
                Ok(Arc::new(decode_layered(
                    &picture.binding.context(),
                    &picture.slices,
                    picture.poc,
                    &lists,
                    budget,
                )?))
            };
            let bad = parse(dpb.poc());
            let error = dpb
                .reconstruct(bad, BUDGET, |picture, temporal, completed, budget| {
                    if picture.binding.sps.layer_id == 3 {
                        return Err(fvid::Error::from(std::io::Error::other(
                            "injected reconstruction failure",
                        )));
                    }
                    make_picture(picture, temporal, completed, budget)
                })
                .err()
                .unwrap();
            assert!(
                error
                    .to_string()
                    .contains("injected reconstruction failure")
            );
            assert_eq!(dpb.poc(), &original_poc);
            assert_eq!(
                dpb.references()
                    .map(Reference::identity)
                    .collect::<Vec<_>>(),
                original_ids
            );
            assert_eq!(dpb.next_access_unit(), index as u64);
            let low = parse(dpb.poc());
            assert!(
                dpb.reconstruct(low, 1, |_, _, _, _| panic!(
                    "budget refusal must precede reconstruction"
                ))
                .is_err()
            );
            assert_eq!(dpb.poc(), &original_poc);
            let good = parse(dpb.poc());
            let (next, outputs, end) = dpb.reconstruct(good, BUDGET, make_picture).unwrap();
            assert_eq!(outputs.len(), 2);
            for decoded in outputs {
                assert_eq!(decoded.poc, index as i32);
                assert_pixels(&decoded.picture, &oracle, index);
            }
            assert_eq!(end, Default::default());
            assert_eq!(dpb.next_access_unit(), index as u64 + 1);
            assert_eq!(dpb.poc().previous(0), Some(index as i32));
            assert_eq!(dpb.poc().previous(3), Some(index as i32));
            parameters = next;
        }
        dpb.reset();
        assert!(dpb.references().next().is_none());
        assert_eq!(dpb.poc(), &LayeredPocState::default());
    }
}

#[test]
fn eos_and_eob_release_references_and_require_a_correct_irap_restart() {
    use fvid::codec::hevc_decoder::LayeredDpb;
    let oracle = std::fs::read(root().join("hevc-pcm-reference-rext8.yuv")).unwrap();
    for (name, kept, error) in [
        (
            "hevc-layered-au-eos-base.mp4",
            vec![],
            "HEVC picture after EOS must restart its source layer with IRAP",
        ),
        (
            "hevc-layered-au-eos-enhancement.mp4",
            vec![0],
            "HEVC picture after EOS must restart its source layer with IRAP",
        ),
        (
            "hevc-layered-au-aud-eos-eob.mp4",
            vec![],
            "HEVC access unit after EOB must begin with IRAP",
        ),
    ] {
        let mut input = movie(name);
        let bytes = input.tracks()[0].configuration.clone();
        let config = HevcConfig::parse(&bytes).unwrap();
        let initial = LayeredParameters::from_configuration(&bytes, BUDGET).unwrap();
        let mut packet = vec![];
        input.read_packet(0, 0, &mut packet).unwrap();
        let mut dpb = LayeredDpb::default();
        let plan =
            LayeredAccessUnit::parse(&initial, dpb.poc(), &packet, config.length_size, 1, BUDGET)
                .unwrap();
        let (parameters, outputs, _) = dpb.decode(plan, BUDGET).unwrap();
        assert_eq!(
            dpb.references().map(|r| r.layer_id).collect::<Vec<_>>(),
            kept,
            "{name}"
        );
        for layer in [0, 3] {
            assert_eq!(
                parameters.active(layer).is_some(),
                kept.contains(&layer),
                "{name}"
            );
        }
        assert_eq!(dpb.poc().previous(3), None);
        assert_eq!(
            dpb.poc().previous(0),
            if kept.is_empty() { None } else { Some(0) }
        );
        for decoded in outputs {
            assert_pixels(&decoded.picture, &oracle, 0);
        }
        // A predicted continuation must fail at the end-marker rule, before
        // reconstruction or a misleading missing-reference failure.
        let mut predicted = movie("hevc-layered-reconstruct-inter-prediction.mp4");
        predicted.read_packet(0, 1, &mut packet).unwrap();
        let before = dpb.poc().clone();
        assert_eq!(
            LayeredAccessUnit::parse(
                &parameters,
                dpb.poc(),
                &packet,
                config.length_size,
                1,
                BUDGET
            )
            .err()
            .unwrap()
            .to_string(),
            error,
            "{name}"
        );
        assert_eq!(dpb.poc(), &before);
        // An original IRAP packet is accepted without a caller-forced reset.
        let mut restart = movie("hevc-layered-reconstruct-pcm-dependent.mp4");
        restart.read_packet(0, 0, &mut packet).unwrap();
        let plan = LayeredAccessUnit::parse(
            &parameters,
            dpb.poc(),
            &packet,
            config.length_size,
            1,
            BUDGET,
        )
        .unwrap();
        let (_, outputs, _) = dpb.decode(plan, BUDGET).unwrap();
        for decoded in outputs {
            assert_pixels(&decoded.picture, &oracle, 0);
        }
        assert_eq!(
            dpb.references().map(|r| r.layer_id).collect::<Vec<_>>(),
            [0, 3]
        );
        assert_eq!(dpb.next_access_unit(), 2);
    }
}

#[test]
fn owned_layered_decode_prepares_references_and_reconstructs_every_saved_witness() {
    use fvid::codec::hevc_decoder::LayeredDpb;
    let manifest = manifest("hevc-layered-reconstruction.json");
    for case in manifest["cases"].as_array().unwrap() {
        let name = case["file"].as_str().unwrap();
        let mut input = movie(name);
        let bytes = input.tracks()[0].configuration.clone();
        let config = HevcConfig::parse(&bytes).unwrap();
        let initial = LayeredParameters::from_configuration(&bytes, BUDGET).unwrap();
        let oracle = std::fs::read(
            root()
                .join(case["seed"].as_str().unwrap())
                .with_extension("yuv"),
        )
        .unwrap();
        let mut dpb = LayeredDpb::default();
        for _ in 0..2 {
            let mut parameters = initial.clone();
            let mut packet = vec![];
            let count = if name.contains("inter-prediction") {
                2
            } else {
                1
            };
            for index in 0..count {
                input.read_packet(0, index, &mut packet).unwrap();
                let original = dpb.poc().clone();
                let make_plan = |state: &LayeredPocState| {
                    LayeredAccessUnit::parse(
                        &parameters,
                        state,
                        &packet,
                        config.length_size,
                        1,
                        BUDGET,
                    )
                    .unwrap()
                };
                assert!(dpb.decode(make_plan(dpb.poc()), 1).is_err());
                assert_eq!(dpb.poc(), &original);
                assert_eq!(dpb.next_access_unit(), index as u64);
                let (next, outputs, end) = dpb
                    .decode(make_plan(dpb.poc()), BUDGET)
                    .unwrap_or_else(|e| panic!("{name} packet {index}: {e}"));
                assert_eq!(
                    outputs.iter().map(|p| p.layer_id).collect::<Vec<_>>(),
                    [0, 3]
                );
                for output in outputs {
                    assert_pixels(&output.picture, &oracle, index);
                }
                for reference in dpb.references() {
                    assert_pixels(
                        reference.picture.as_ref().unwrap(),
                        &oracle,
                        reference.poc as usize,
                    );
                }
                assert_eq!(end, Default::default());
                parameters = next;
            }
            dpb.reset();
        }
    }
}

#[test]
fn stateful_layered_packet_decoder_owns_parameters_references_and_reset() {
    use fvid::codec::hevc_decoder::LayeredDecoder;
    let m = manifest("hevc-layered-reconstruction.json");
    for case in m["cases"].as_array().unwrap() {
        let name = case["file"].as_str().unwrap();
        let mut input = movie(name);
        let bytes = input.tracks()[0].configuration.clone();
        let oracle = std::fs::read(
            root()
                .join(case["seed"].as_str().unwrap())
                .with_extension("yuv"),
        )
        .unwrap();
        let mut decoder = LayeredDecoder::from_configuration(&bytes, 1, BUDGET).unwrap();
        for _ in 0..2 {
            let count = if name.contains("inter-prediction") {
                2
            } else {
                1
            };
            let mut packet = vec![];
            for index in 0..count {
                input.read_packet(0, index, &mut packet).unwrap();
                let (pictures, end) = decoder
                    .decode_packet(&packet)
                    .unwrap_or_else(|e| panic!("{name}: {e}"));
                assert_eq!(
                    pictures.iter().map(|p| p.layer_id).collect::<Vec<_>>(),
                    [0, 3]
                );
                for p in pictures {
                    let output_highest = case["output_highest"].as_bool().unwrap_or(false);
                    assert_eq!(
                        p.signalled_output_layer().unwrap(),
                        !output_highest || p.layer_id == 3
                    );
                    assert_pixels(&p.picture, &oracle, index);
                }
                assert_eq!(end, Default::default());
                assert_eq!(decoder.next_access_unit(), index as u64 + 1);
                assert!(decoder.parameters().active(0).is_some());
                assert!(decoder.parameters().active(3).is_some());
                assert_eq!(decoder.poc().previous(0), Some(index as i32));
                assert!(
                    decoder
                        .prime_parameters(&packet)
                        .err()
                        .unwrap()
                        .to_string()
                        .contains("requires a reset decoder")
                );
            }
            decoder.reset();
            assert_eq!(decoder.poc(), &LayeredPocState::default());
            assert_eq!(decoder.next_access_unit(), 0);
            assert!(decoder.parameters().active(0).is_none());
            assert!(decoder.parameters().active(3).is_none());
        }
    }
}

#[test]
fn layered_stream_priming_and_packet_errors_never_publish_partial_parameter_state() {
    use fvid::codec::hevc_decoder::LayeredDecoder;
    let baseline = movie("hevc-layered-reconstruct-pcm-independent.mp4");
    let bytes = baseline.tracks()[0].configuration.clone();
    let mut decoder = LayeredDecoder::from_configuration(&bytes, 1, BUDGET).unwrap();
    let original = decoder
        .parameters()
        .clone()
        .activate(0, 0, 1)
        .unwrap()
        .sps
        .sps
        .clone();
    let update = movie("hevc-layered-sps-content-update.mp4");
    let config = HevcConfig::parse(&update.tracks()[0].configuration).unwrap();
    let changed = config
        .arrays
        .iter()
        .find(|a| a.nal_type == 33)
        .unwrap()
        .units[1];
    let mut prime = (changed.len() as u32).to_be_bytes().to_vec();
    prime.extend_from_slice(changed);
    let mut invalid = prime.clone();
    invalid.extend_from_slice(&[0, 0, 0, 1, 0]);
    assert!(decoder.prime_parameters(&invalid).is_err());
    assert_eq!(
        decoder
            .parameters()
            .clone()
            .activate(0, 0, 1)
            .unwrap()
            .sps
            .sps,
        original
    );
    assert_eq!(decoder.poc(), &LayeredPocState::default());
    decoder.prime_parameters(&prime).unwrap();
    assert_ne!(
        decoder
            .parameters()
            .clone()
            .activate(0, 0, 1)
            .unwrap()
            .sps
            .sps,
        original
    );
    assert!(decoder.parameters().active(0).is_none());
    assert_eq!(decoder.next_access_unit(), 0);
    decoder.reset();
    assert_eq!(
        decoder
            .parameters()
            .clone()
            .activate(0, 0, 1)
            .unwrap()
            .sps
            .sps,
        original
    );
    let mut bad = movie("hevc-layered-au-descending-layer.mp4");
    let mut decoder =
        LayeredDecoder::from_configuration(&bad.tracks()[0].configuration, 1, BUDGET).unwrap();
    let mut baseline = movie("hevc-layered-reconstruct-pcm-dependent.mp4");
    let mut packet = vec![];
    bad.read_packet(0, 0, &mut packet).unwrap();
    assert_eq!(
        decoder.decode_packet(&packet).err().unwrap().to_string(),
        "HEVC layered pictures must increase in layer order"
    );
    assert_eq!(decoder.next_access_unit(), 0);
    assert!(decoder.parameters().active(0).is_none());
    baseline.read_packet(0, 0, &mut packet).unwrap();
    let (pictures, _) = decoder.decode_packet(&packet).unwrap();
    let oracle = std::fs::read(root().join("hevc-pcm-reference-rext8.yuv")).unwrap();
    for p in pictures {
        assert_pixels(&p.picture, &oracle, 0);
    }
    // Priming the skipped I AU must not publish its pixels/POC or make a P AU
    // accidentally decode using references from before reset.
    let mut inter = movie("hevc-layered-reconstruct-inter-prediction.mp4");
    let mut decoder =
        LayeredDecoder::from_configuration(&inter.tracks()[0].configuration, 1, BUDGET).unwrap();
    inter.read_packet(0, 0, &mut packet).unwrap();
    decoder.prime_parameters(&packet).unwrap();
    assert_eq!(decoder.poc(), &LayeredPocState::default());
    inter.read_packet(0, 1, &mut packet).unwrap();
    assert!(
        decoder
            .decode_packet(&packet)
            .err()
            .unwrap()
            .to_string()
            .contains("reference POC 0 is missing")
    );
    assert_eq!(decoder.next_access_unit(), 0);
    inter.read_packet(0, 0, &mut packet).unwrap();
    decoder.decode_packet(&packet).unwrap();
    inter.read_packet(0, 1, &mut packet).unwrap();
    decoder.decode_packet(&packet).unwrap();
}

#[test]
fn stateful_layered_decoder_applies_sequence_ends_without_external_parameter_updates() {
    use fvid::codec::hevc_decoder::LayeredDecoder;
    for (file, base_kept) in [
        ("hevc-layered-au-eos-base.mp4", false),
        ("hevc-layered-au-eos-enhancement.mp4", true),
        ("hevc-layered-au-aud-eos-eob.mp4", false),
    ] {
        let mut input = movie(file);
        let mut decoder =
            LayeredDecoder::from_configuration(&input.tracks()[0].configuration, 1, BUDGET)
                .unwrap();
        let mut packet = vec![];
        input.read_packet(0, 0, &mut packet).unwrap();
        let (pictures, end) = decoder.decode_packet(&packet).unwrap();
        assert_eq!(pictures.len(), 2);
        assert_ne!(end, Default::default());
        assert_eq!(decoder.parameters().active(0).is_some(), base_kept);
        assert!(decoder.parameters().active(3).is_none());
        let mut inter = movie("hevc-layered-reconstruct-inter-prediction.mp4");
        inter.read_packet(0, 1, &mut packet).unwrap();
        assert!(
            decoder
                .decode_packet(&packet)
                .err()
                .unwrap()
                .to_string()
                .contains(if end.bitstream {
                    "after EOB"
                } else {
                    "after EOS"
                })
        );
        assert_eq!(decoder.next_access_unit(), 1);
        inter.read_packet(0, 0, &mut packet).unwrap();
        let (pictures, _) = decoder.decode_packet(&packet).unwrap();
        let oracle = std::fs::read(root().join("hevc-pcm-reference-rext8.yuv")).unwrap();
        for p in pictures {
            assert_pixels(&p.picture, &oracle, 0);
        }
        assert!(decoder.parameters().active(0).is_some());
        assert!(decoder.parameters().active(3).is_some());
        assert_eq!(decoder.next_access_unit(), 2);
    }
}

#[test]
fn decoded_picture_bindings_survive_sps_replacement_eos_reset_and_owner_drop() {
    use fvid::codec::hevc_decoder::LayeredDecoder;
    let mut input = movie("hevc-layered-reconstruct-pcm-independent.mp4");
    let bytes = input.tracks()[0].configuration.clone();
    let mut decoder = LayeredDecoder::from_configuration(&bytes, 1, BUDGET).unwrap();
    let mut packet = vec![];
    input.read_packet(0, 0, &mut packet).unwrap();
    let (old, _) = decoder.decode_packet(&packet).unwrap();
    let old_sps = old[0].binding.sps.sps.clone();
    let old_pps = old[0].binding.pps.clone();
    for p in &old {
        assert_eq!(p.layer_id, p.binding.sps.layer_id);
        assert!(p.signalled_output_layer().unwrap());
    }
    decoder.reset();
    let update = movie("hevc-layered-sps-content-update.mp4");
    let config = HevcConfig::parse(&update.tracks()[0].configuration).unwrap();
    let sps = config
        .arrays
        .iter()
        .find(|a| a.nal_type == 33)
        .unwrap()
        .units[1];
    let mut prime = (sps.len() as u32).to_be_bytes().to_vec();
    prime.extend_from_slice(sps);
    decoder.prime_parameters(&prime).unwrap();
    let (new, _) = decoder.decode_packet(&packet).unwrap();
    assert_ne!(new[0].binding.sps.sps, old_sps);
    assert_eq!(old[0].binding.sps.sps, old_sps);
    assert_eq!(old[0].binding.pps, old_pps);
    decoder.reset();
    drop(decoder);
    let oracle = std::fs::read(root().join("hevc-pcm-reference-rext8.yuv")).unwrap();
    for p in old.iter().chain(&new) {
        assert_pixels(&p.picture, &oracle, 0);
        assert_eq!(p.picture.dimensions, p.binding.sps.sps.dimensions);
        assert_eq!(p.picture.depth, p.binding.sps.sps.depth);
        assert!(p.signalled_output_layer().unwrap());
    }
    let mut ended = movie("hevc-layered-au-aud-eos-eob.mp4");
    let mut decoder =
        LayeredDecoder::from_configuration(&ended.tracks()[0].configuration, 1, BUDGET).unwrap();
    ended.read_packet(0, 0, &mut packet).unwrap();
    let (pictures, end) = decoder.decode_packet(&packet).unwrap();
    assert!(end.bitstream);
    assert!(decoder.parameters().active(0).is_none());
    assert!(decoder.parameters().active(3).is_none());
    drop(decoder);
    for p in pictures {
        assert_eq!(p.layer_id, p.binding.pps.layer_id);
        assert_eq!(p.picture.dimensions, p.binding.sps.sps.dimensions);
        assert_pixels(&p.picture, &oracle, 0);
        assert!(p.signalled_output_layer().unwrap());
    }
}

#[test]
fn alternative_output_policy_reconstructs_pixels_and_refuses_inconsistent_flags_atomically() {
    use fvid::codec::hevc_decoder::LayeredDecoder;
    let m = manifest("hevc-layered-output-policy.json");
    assert_eq!(m["cases"].as_array().unwrap().len(), 4);
    for case in m["cases"].as_array().unwrap() {
        let name = case["file"].as_str().unwrap();
        let mut input = movie(name);
        let oracle = std::fs::read(
            root()
                .join(case["seed"].as_str().unwrap())
                .with_extension("yuv"),
        )
        .unwrap();
        let mut decoder =
            LayeredDecoder::from_configuration(&input.tracks()[0].configuration, 1, BUDGET)
                .unwrap();
        let mut packet = vec![];
        for index in 0..2 {
            input.read_packet(0, index, &mut packet).unwrap();
            let before = decoder.poc().clone();
            let result = decoder.decode_packet(&packet);
            if index == 1
                && let Some(error) = case["error"].as_str()
            {
                assert_eq!(result.err().unwrap().to_string(), error, "{name}");
                assert_eq!(decoder.poc(), &before);
                assert_eq!(decoder.next_access_unit(), 1);
                continue;
            }
            let (pictures, _) = result.unwrap_or_else(|e| panic!("{name} AU {index}: {e}"));
            let expected_layers = if index == 1 && case["missing"].as_bool().unwrap() {
                vec![0]
            } else {
                vec![0, 3]
            };
            assert_eq!(
                pictures.iter().map(|p| p.layer_id).collect::<Vec<_>>(),
                expected_layers
            );
            let mask = pictures
                .iter()
                .filter(|p| p.output)
                .fold(0u64, |m, p| m | (1 << p.layer_id));
            assert_eq!(
                mask,
                case["output_masks"][index].as_u64().unwrap(),
                "{name}"
            );
            for p in pictures {
                assert_eq!(p.nal.layer_id, p.layer_id);
                assert_eq!(p.access_unit, index as u64);
                assert_eq!(
                    p.coded_output,
                    !(index == 1 && case["hidden"].as_bool().unwrap())
                );
                assert_pixels(&p.picture, &oracle, index);
            }
            assert_eq!(decoder.next_access_unit(), index as u64 + 1);
        }
    }
}

#[test]
fn extracted_layer_and_temporal_substreams_drop_only_inapplicable_timing_and_decode() {
    use fvid::codec::hevc_decoder::{LayeredDecoder, extract_layered_substream};
    let mut input = movie("hevc-layered-extraction-timing.mp4");
    let config = input.tracks()[0].configuration.clone();
    let mut packet = vec![];
    input.read_packet(0, 0, &mut packet).unwrap();
    let units = |bytes: &[u8]| {
        NalUnits::new(bytes, 4)
            .unwrap()
            .map(|n| n.unwrap().to_vec())
            .collect::<Vec<_>>()
    };
    assert_eq!(
        extract_layered_substream(&packet, 4, 9, 6, BUDGET).unwrap(),
        packet
    );
    let oracle = std::fs::read(root().join("hevc-pcm-reference-rext8.yuv")).unwrap();
    for (layers, temporal, expected_layers) in [(1, 6, vec![0]), (9, 0, vec![0, 3])] {
        let extracted = extract_layered_substream(&packet, 4, layers, temporal, BUDGET).unwrap();
        let original = units(&packet);
        let retained = units(&extracted);
        // Metadata before any discarded picture is still removed globally.
        assert_eq!(
            retained
                .iter()
                .filter(|n| NalHeader::parse(n).unwrap().unit_type == 39)
                .count(),
            2
        );
        let sei_types = retained
            .iter()
            .filter(|n| NalHeader::parse(n).unwrap().unit_type == 39)
            .map(|n| {
                let rbsp = fvid::codec::hevc_nal::NalRbsp::parse(n, BUDGET).unwrap();
                fvid::codec::hevc_sei::messages(&rbsp.bytes).unwrap()[0].payload_type
            })
            .collect::<Vec<_>>();
        assert_eq!(sei_types, [144, 133]);
        for nal in &retained {
            assert!(original.contains(nal));
            let h = NalHeader::parse(nal).unwrap();
            assert!(layers & (1 << h.layer_id) != 0 && h.temporal_id <= temporal);
        }
        let mut decoder = LayeredDecoder::from_configuration(&config, 1, BUDGET).unwrap();
        let (pictures, _) = decoder.decode_packet(&extracted).unwrap();
        assert_eq!(
            pictures.iter().map(|p| p.layer_id).collect::<Vec<_>>(),
            expected_layers
        );
        for picture in pictures {
            assert_pixels(&picture.picture, &oracle, 0);
        }
        assert_eq!(
            extract_layered_substream(&extracted, 4, layers, temporal, BUDGET).unwrap(),
            extracted
        );
    }
    for (layers, temporal) in [(0, 0), (1u64 << 63, 0), (1, 7)] {
        assert_eq!(
            extract_layered_substream(&packet, 4, layers, temporal, BUDGET)
                .err()
                .unwrap()
                .to_string(),
            "invalid HEVC extraction target"
        );
    }
    assert!(extract_layered_substream(&packet, 4, 1, 0, 1).is_err());
    assert!(extract_layered_substream(&packet[..packet.len() - 1], 4, 1, 0, BUDGET).is_err());
}

#[test]
fn globally_scanned_selection_removes_earlier_timing_and_decodes_packets_atomically() {
    use fvid::codec::{
        hevc_decoder::{LayeredDecoder, SubstreamScan, extract_layered_substream},
        hevc_vps::LayeredVps,
    };
    let mut input = movie("hevc-layered-extraction-split-timing.mp4");
    let config = input.tracks()[0].configuration.clone();
    let vps = |config: &[u8]| {
        let c = HevcConfig::parse(config).unwrap();
        let n = &c.arrays.iter().find(|a| a.nal_type == 32).unwrap().units[0];
        Arc::new(LayeredVps::parse(n, BUDGET).unwrap())
    };
    let mut scan = SubstreamScan::default();
    let mut packets = vec![];
    for index in 0..2 {
        let mut packet = vec![];
        input.read_packet(0, index, &mut packet).unwrap();
        scan.observe(&packet, 4).unwrap();
        packets.push(packet);
    }
    let before = scan;
    assert!(
        scan.observe(&packets[1][..packets[1].len() - 1], 4)
            .is_err()
    );
    assert_eq!(scan, before);
    let selected = scan.select(vps(&config), 1, 0).unwrap();
    assert_eq!(selected.layers(), 9);
    assert_eq!(selected.output_layer_set(), 1);
    // Packet zero alone contains no discarded header. Whole-source policy differs.
    assert_eq!(
        extract_layered_substream(&packets[0], 4, 9, 0, BUDGET).unwrap(),
        packets[0]
    );
    let first = selected.extract_packet(&packets[0], 4, BUDGET).unwrap();
    let types = NalUnits::new(&first, 4)
        .unwrap()
        .map(|n| {
            let n = n.unwrap();
            let rbsp = fvid::codec::hevc_nal::NalRbsp::parse(n, BUDGET).unwrap();
            fvid::codec::hevc_sei::messages(&rbsp.bytes).unwrap()[0].payload_type
        })
        .collect::<Vec<_>>();
    assert_eq!(types, [144, 133]);
    let full = scan.select(vps(&config), 1, 6).unwrap();
    assert_eq!(
        full.extract_packet(&packets[0], 4, BUDGET).unwrap(),
        packets[0]
    );
    let mut decoder = LayeredDecoder::from_configuration(&config, 1, BUDGET).unwrap();
    let base = scan.select(vps(&config), 0, 0).unwrap();
    assert_eq!(base.layers(), 1);
    assert_eq!(
        decoder
            .decode_selected_packet(&base, &packets[0])
            .err()
            .unwrap()
            .to_string(),
        "HEVC extraction OLS disagrees with decoder"
    );
    assert_eq!(decoder.next_access_unit(), 0);
    let other = movie("hevc-layered-reconstruct-inter-prediction-output-highest.mp4");
    let stale = scan
        .select(vps(&other.tracks()[0].configuration), 1, 0)
        .unwrap();
    let other_config = HevcConfig::parse(&other.tracks()[0].configuration).unwrap();
    let other_vps = &other_config
        .arrays
        .iter()
        .find(|a| a.nal_type == 32)
        .unwrap()
        .units[0];
    let mut control_only = (other_vps.len() as u32).to_be_bytes().to_vec();
    control_only.extend_from_slice(other_vps);
    assert_eq!(
        decoder
            .decode_selected_packet(&selected, &control_only)
            .err()
            .unwrap()
            .to_string(),
        "HEVC extraction VPS disagrees with active picture"
    );
    assert_eq!(decoder.next_access_unit(), 0);
    assert!(decoder.parameters().active(0).is_none());

    assert_eq!(
        decoder
            .decode_selected_packet(&stale, &packets[1])
            .err()
            .unwrap()
            .to_string(),
        "HEVC extraction VPS disagrees with active picture"
    );
    assert_eq!(decoder.next_access_unit(), 0);
    assert_eq!(decoder.poc(), &LayeredPocState::default());
    assert!(
        decoder
            .decode_selected_packet(&selected, &packets[0])
            .unwrap()
            .0
            .is_empty()
    );
    let (pictures, _) = decoder
        .decode_selected_packet(&selected, &packets[1])
        .unwrap();
    assert_eq!(
        pictures.iter().map(|p| p.layer_id).collect::<Vec<_>>(),
        [0, 3]
    );
    let oracle = std::fs::read(root().join("hevc-pcm-reference-rext8.yuv")).unwrap();
    for p in pictures {
        assert_pixels(&p.picture, &oracle, 0);
    }
    // Highest-only OLS still decodes its base reference layer (F-57).
    assert_eq!(stale.layers(), 9);
    let mut highest =
        LayeredDecoder::from_configuration(&other.tracks()[0].configuration, 1, BUDGET).unwrap();
    let (pictures, _) = highest.decode_selected_packet(&stale, &packets[1]).unwrap();
    assert_eq!(
        pictures.iter().map(|p| p.layer_id).collect::<Vec<_>>(),
        [0, 3]
    );
    assert_eq!(
        pictures
            .iter()
            .filter(|p| p.output)
            .map(|p| p.layer_id)
            .collect::<Vec<_>>(),
        [3]
    );
    for p in pictures {
        assert_pixels(&p.picture, &oracle, 0);
    }
    let outside = [0, 0, 0, 3, 82, 9, 128]; // a layer absent from the completed scan
    assert_eq!(
        selected
            .extract_packet(&outside, 4, BUDGET)
            .err()
            .unwrap()
            .to_string(),
        "HEVC packet exceeds scanned extraction source"
    );
    assert!(scan.select(vps(&config), 99, 0).is_err());
}

#[test]
fn random_access_entry_eos_and_alternative_radl_follow_actual_coded_picture_flags() {
    use fvid::codec::hevc_decoder::LayeredDecoder;
    let m = manifest("hevc-layered-random-access.json");
    assert_eq!(m["cases"].as_array().unwrap().len(), 4);
    let oracle = std::fs::read(root().join("hevc-pcm-reference-rext8.yuv")).unwrap();
    for case in m["cases"].as_array().unwrap() {
        let name = case["file"].as_str().unwrap();
        let mut input = movie(name);
        let mut decoder =
            LayeredDecoder::from_configuration(&input.tracks()[0].configuration, 1, BUDGET)
                .unwrap();
        for _ in 0..2 {
            for index in 0..5 {
                let mut packet = vec![];
                input.read_packet(0, index, &mut packet).unwrap();
                let (pictures, _) = decoder
                    .decode_packet(&packet)
                    .unwrap_or_else(|e| panic!("{name} AU {index}: {e}"));
                let expected = case["decoded"][index]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|v| v.as_u64().unwrap() as u8)
                    .collect::<Vec<_>>();
                assert_eq!(
                    pictures.iter().map(|p| p.layer_id).collect::<Vec<_>>(),
                    expected,
                    "{name} AU {index}"
                );
                assert_eq!(
                    pictures
                        .iter()
                        .filter(|p| p.output)
                        .fold(0u64, |mask, p| mask | (1 << p.layer_id)),
                    case["output_masks"][index].as_u64().unwrap(),
                    "{name} AU {index}"
                );
                for p in pictures {
                    assert!(p.coded_output);
                    assert_eq!(p.poc, case["pocs"][index].as_i64().unwrap() as i32);
                    assert_pixels(&p.picture, &oracle, 0);
                }
            }
            decoder.reset();
        }
    }
}

#[test]
fn failed_later_irap_never_publishes_random_access_suppression_state() {
    use fvid::codec::hevc_decoder::LayeredDpb;
    let mut input = movie("hevc-layered-random-access-entry-and-later-cra.mp4");
    let config = input.tracks()[0].configuration.clone();
    let mut parameters = LayeredParameters::from_configuration(&config, BUDGET).unwrap();
    let mut dpb = LayeredDpb::default();
    let mut saved = None;
    for index in 0..3 {
        let mut packet = vec![];
        input.read_packet(0, index, &mut packet).unwrap();
        let plan = LayeredAccessUnit::parse(&parameters, dpb.poc(), &packet, 4, 1, BUDGET).unwrap();
        let (next, pictures, _) = dpb.decode(plan, BUDGET).unwrap();
        if index == 0 {
            saved = Some(pictures[0].picture.clone());
        }
        parameters = next;
    }
    let before = dpb.poc().clone();
    let ids = dpb
        .references()
        .map(Reference::identity)
        .collect::<Vec<_>>();
    let mut packet = vec![];
    input.read_packet(0, 3, &mut packet).unwrap();
    let plan = LayeredAccessUnit::parse(&parameters, dpb.poc(), &packet, 4, 1, BUDGET).unwrap();
    let error = dpb
        .reconstruct(plan, BUDGET, |picture, _, _, _| {
            if picture.binding.sps.layer_id == 3 {
                return Err(fvid::Error::from(std::io::Error::other(
                    "injected later CRA failure",
                )));
            }
            Ok(saved.clone().unwrap())
        })
        .err()
        .unwrap();
    assert!(error.to_string().contains("injected later CRA failure"));
    assert_eq!(dpb.poc(), &before);
    assert_eq!(dpb.next_access_unit(), 3);
    assert_eq!(
        dpb.references()
            .map(Reference::identity)
            .collect::<Vec<_>>(),
        ids
    );
    // Still associated with the first CRA: replay its leading RASL, not the
    // RASL of the failed later CRA. No partial flag transition may expose it.
    input.read_packet(0, 1, &mut packet).unwrap();
    let plan = LayeredAccessUnit::parse(&parameters, dpb.poc(), &packet, 4, 1, BUDGET).unwrap();
    let (next, pictures, _) = dpb.decode(plan, BUDGET).unwrap();
    assert!(pictures.is_empty());
    parameters = next;
    for index in [3, 4] {
        input.read_packet(0, index, &mut packet).unwrap();
        let plan = LayeredAccessUnit::parse(&parameters, dpb.poc(), &packet, 4, 1, BUDGET).unwrap();
        let (next, pictures, _) = dpb.decode(plan, BUDGET).unwrap();
        assert_eq!(pictures.len(), 2);
        assert!(pictures.iter().all(|p| p.output));
        parameters = next;
    }
}
