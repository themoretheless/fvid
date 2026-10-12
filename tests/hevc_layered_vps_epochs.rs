//! Original coded VPS-sequence changes; extraction, queued snapshots and seek.
use fvid::{
    codec::{
        config::{HevcConfig, NalUnits},
        hevc_decoder::{LayeredDecoder, LayeredOutputQueue, SubstreamScan},
        hevc_vps::LayeredVps,
    },
    container::mp4::{Limits, Mp4Reader},
    playback::video::mp4::{LayeredMp4VideoReader, Mp4VideoReader},
};
use std::{io::Cursor, path::PathBuf, sync::Arc};
const BUDGET: usize = 64 << 20;
fn path(variant: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors")
        .join(format!("hevc-layered-vps-epoch-{variant}.mp4"))
}
fn reader(variant: &str, tid: u8) -> LayeredMp4VideoReader<Cursor<Vec<u8>>> {
    LayeredMp4VideoReader::open(
        Cursor::new(std::fs::read(path(variant)).unwrap()),
        Limits::default(),
        1,
        tid,
        BUDGET,
    )
    .unwrap()
}
fn packets(variant: &str) -> (Vec<u8>, Vec<Vec<u8>>) {
    let mut demux = Mp4Reader::open(
        Cursor::new(std::fs::read(path(variant)).unwrap()),
        Limits::default(),
    )
    .unwrap();
    let config = demux.tracks()[0].configuration.clone();
    let mut packets = Vec::new();
    for index in 0..demux.tracks()[0].samples.len() {
        let mut p = Vec::new();
        demux.read_packet(0, index, &mut p).unwrap();
        packets.push(p);
    }
    (config, packets)
}
fn first_new(variant: &str) -> usize {
    match variant {
        "before-first" => 1,
        "split" => 4,
        "bla" => 2,
        _ => 3,
    }
}
#[test]
fn coded_vps_changes_preserve_original_pixels_indices_and_old_output_snapshots() {
    let oracle = std::fs::read(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/playback-errors/hevc-pcm-reference-rext8.yuv"),
    )
    .unwrap();
    for variant in [
        "eos",
        "eob",
        "split",
        "before-first",
        "multiple",
        "short",
        "bla",
    ] {
        for tid in [0, 2, 6] {
            let mut reader = reader(variant, tid);
            for _ in 0..2 {
                let mut identities = Vec::new();
                let mut old_vps = None;
                while let Some(frame) = reader
                    .read_frame()
                    .unwrap_or_else(|e| panic!("{variant}/T{tid}: {e}"))
                {
                    let index = frame.sample_index;
                    let layer = frame.decoded.layer_id;
                    assert_eq!(frame.presentation_time.ticks, index as i64);
                    assert_eq!(frame.duration.ticks, 1);
                    assert_eq!(frame.decoded.access_unit, index as u64);
                    assert!(frame.decoded.signalled_output_layer().unwrap());
                    if index < first_new(variant) {
                        old_vps = Some(frame.decoded.binding.vps.clone());
                    } else {
                        if variant != "multiple" {
                            assert_eq!(layer, 3);
                        }
                        if let Some(old) = &old_vps {
                            if variant == "multiple" {
                                assert_eq!(old.as_ref(), frame.decoded.binding.vps.as_ref());
                            } else {
                                assert_ne!(old.as_ref(), frame.decoded.binding.vps.as_ref());
                            }
                        }
                    }
                    identities.push((index, layer));
                    let packed = frame.into_video_frame().unwrap().packed.unwrap();
                    assert_eq!(&packed.frame.data, &oracle[..packed.frame.data.len()]);
                }
                let mut expected = if variant == "before-first" {
                    vec![]
                } else {
                    vec![(0, 0), (0, 3), (1, 0), (1, 3)]
                };
                if variant == "multiple" {
                    expected.extend(
                        (first_new(variant)..first_new(variant) + 2).flat_map(|i| [(i, 0), (i, 3)]),
                    );
                } else {
                    expected.extend([(first_new(variant), 3), (first_new(variant) + 1, 3)]);
                }
                assert_eq!(identities, expected, "{variant}/T{tid}");
                reader.rewind();
            }
        }
    }
}
#[test]
fn extraction_inventory_starts_at_eos_and_includes_prefix_samples_before_new_vps() {
    for variant in ["eos", "split", "before-first"] {
        let source = reader(variant, 0);
        let (_, packets) = packets(variant);
        let metadata = if variant == "before-first" { 0 } else { 2 };
        let selected = source.selection_at(metadata).unwrap();
        assert!(
            selected
                .extract_packet(&packets[metadata], 4, BUDGET)
                .unwrap()
                .is_empty()
        );
        if variant != "before-first" {
            let old = source
                .selection_at(0)
                .unwrap()
                .extract_packet(&packets[0], 4, BUDGET)
                .unwrap();
            assert!(
                NalUnits::new(&old, 4)
                    .unwrap()
                    .any(|n| n.unwrap()[0] >> 1 == 39)
            );
        }
        assert!(source.selection_at(packets.len()).is_none());
        let full = reader(variant, 2);
        assert_eq!(
            full.selection_at(metadata)
                .unwrap()
                .extract_packet(&packets[metadata], 4, BUDGET)
                .unwrap(),
            packets[metadata]
        );
    }
}
#[test]
fn common_player_and_native_seek_cross_real_vps_epochs() {
    use fvid::playback_native::NativeReader;
    use std::time::Duration;
    for variant in [
        "eos",
        "eob",
        "split",
        "before-first",
        "multiple",
        "short",
        "bla",
    ] {
        let bytes = std::fs::read(path(variant)).unwrap();
        let mut source =
            Mp4VideoReader::open_software(Cursor::new(bytes.clone()), Limits::default(), BUDGET)
                .unwrap();
        let first = first_new(variant);
        let expected = if variant == "before-first" {
            vec![first, first + 1]
        } else {
            vec![0, 1, first, first + 1]
        };
        for _ in 0..2 {
            let mut samples = Vec::new();
            while let Some(frame) = source.read_frame().unwrap() {
                samples.push(frame.sample_index);
                assert_eq!(source.selected_layer_id(), Some(3));
            }
            assert_eq!(samples, expected);
            assert_eq!(source.seek_to_sync(first as i64), first as i64);
            for index in [first, first + 1] {
                let frame = source.read_frame().unwrap().unwrap();
                assert_eq!(frame.sample_index, index);
                assert_eq!(frame.presentation_time.ticks, index as i64);
            }
            assert!(source.read_frame().unwrap().is_none());
            source.rewind();
        }
        let mut native = NativeReader::software(Cursor::new(bytes), BUDGET).unwrap();
        assert!(
            native
                .seek_raw(Duration::from_millis(first as u64 * 40))
                .unwrap()
                .is_some()
        );
        native.rewind().unwrap();
        assert!(native.read_frame_raw().unwrap().is_some());
    }
}
#[test]
fn old_fixed_vps_selection_reproduces_the_failure_and_epoch_selection_accepts_it() {
    let source = reader("eos", 2);
    let (config, packets) = packets("eos");
    let config_view = HevcConfig::parse(&config).unwrap();
    let vps = Arc::new(
        LayeredVps::parse(
            config_view
                .arrays
                .iter()
                .find(|a| a.nal_type == 32)
                .unwrap()
                .units[0],
            BUDGET,
        )
        .unwrap(),
    );
    let mut scan = SubstreamScan::default();
    for packet in &packets {
        scan.observe(packet, 4).unwrap();
    }
    let fixed = scan.select(vps, 1, 2).unwrap();
    let mut decoder = LayeredDecoder::from_configuration(&config, 1, BUDGET).unwrap();
    let mut queue = LayeredOutputQueue::default();
    for packet in &packets[..3] {
        decoder
            .decode_selected_queued_packet(&mut queue, &fixed, packet, 2)
            .unwrap();
    }
    let poc = decoder.poc().clone();
    let error = decoder
        .decode_selected_queued_packet(&mut queue, &fixed, &packets[3], 2)
        .err()
        .unwrap()
        .to_string();
    assert_eq!(error, "HEVC extraction VPS disagrees with active picture");
    assert_eq!(decoder.poc(), &poc);
    assert_eq!(decoder.next_access_unit(), 3);
    let (released, _) = decoder
        .decode_selected_queued_packet(&mut queue, source.selection_at(3).unwrap(), &packets[3], 2)
        .unwrap();
    assert_eq!(
        released
            .iter()
            .map(|f| (f.access_unit, f.layer_id))
            .collect::<Vec<_>>(),
        [(0, 0), (0, 3), (1, 0), (1, 3), (3, 3)]
    );
}
#[test]
fn illegal_active_vps_replacement_is_still_refused() {
    let mut source = reader("illegal", 2);
    for _ in 0..2 {
        let error = source.read_frame().err().unwrap().to_string();
        assert_eq!(error, "HEVC active VPS changed before sequence end");
        assert!(
            source
                .read_frame()
                .err()
                .unwrap()
                .to_string()
                .contains("requires rewind")
        );
        source.rewind();
    }
}
#[test]
fn failed_new_vps_entropy_preserves_old_queued_frames_and_retry_uses_new_binding() {
    let source = reader("eos", 2);
    let (config, good) = packets("eos");
    let (_, bad) = packets("truncated");
    let mut decoder = LayeredDecoder::from_configuration(&config, 1, BUDGET).unwrap();
    let mut queue = LayeredOutputQueue::default();
    for (index, packet) in good[..3].iter().enumerate() {
        decoder
            .decode_selected_queued_packet(
                &mut queue,
                source.selection_at(index).unwrap(),
                packet,
                2,
            )
            .unwrap();
    }
    let old = queue.clone().drain();
    let poc = decoder.poc().clone();
    assert_eq!(queue.pending_access_units(), 2);
    let error = decoder
        .decode_selected_queued_packet(&mut queue, source.selection_at(3).unwrap(), &bad[3], 2)
        .err()
        .unwrap()
        .to_string();
    assert!(
        error.contains("truncated or oversized bit field"),
        "{error}"
    );
    assert_eq!(decoder.poc(), &poc);
    assert_eq!(decoder.next_access_unit(), 3);
    assert_eq!(queue.pending_access_units(), 2);
    for (before, after) in old.iter().zip(queue.clone().drain()) {
        assert!(Arc::ptr_eq(&before.binding, &after.binding));
        assert!(Arc::ptr_eq(&before.picture, &after.picture));
    }
    let (released, _) = decoder
        .decode_selected_queued_packet(&mut queue, source.selection_at(3).unwrap(), &good[3], 2)
        .unwrap();
    assert_eq!(released.len(), 5);
    for (before, after) in old.iter().zip(&released) {
        assert!(Arc::ptr_eq(&before.binding, &after.binding));
    }
    assert_ne!(released[0].binding.vps, released[4].binding.vps);
    let (last, _) = decoder
        .decode_selected_queued_packet(&mut queue, source.selection_at(4).unwrap(), &good[4], 2)
        .unwrap();
    assert_eq!((last[0].access_unit, last[0].layer_id), (4, 3));
    assert_eq!(queue.pending_access_units(), 0);
}

#[test]
fn selected_native_packet_entry_uses_last_unactivated_definition_of_same_vps_id() {
    let source = reader("multiple", 2);
    let (config, packets) = packets("multiple");
    let mut decoder = LayeredDecoder::from_configuration(&config, 1, BUDGET).unwrap();
    let mut identities = Vec::new();
    for (index, packet) in packets.iter().enumerate() {
        let (frames, _) = decoder
            .decode_selected_packet(source.selection_at(index).unwrap(), packet)
            .unwrap();
        for frame in frames {
            assert!(frame.output);
            identities.push((frame.access_unit, frame.layer_id));
        }
    }
    assert_eq!(
        identities,
        [
            (0, 0),
            (0, 3),
            (1, 0),
            (1, 3),
            (3, 0),
            (3, 3),
            (4, 0),
            (4, 3)
        ]
    );
}

#[test]
fn later_vps_reduces_highest_tid_to_selected_ols_temporal_limit() {
    let source = reader("short", 6);
    assert_eq!(source.selection_at(0).unwrap().highest_temporal_id(), 2);
    assert_eq!(source.selection_at(2).unwrap().highest_temporal_id(), 1);
    let (_, packets) = packets("short");
    assert!(
        source
            .selection_at(2)
            .unwrap()
            .extract_packet(&packets[2], 4, BUDGET)
            .unwrap()
            .is_empty()
    );
}

#[test]
fn failed_implicit_bla_cvsg_keeps_active_old_parameters_until_successful_retry() {
    for variant in ["bla", "graph", "idr"] {
        let source = reader(variant, 2);
        let (config, good) = packets(variant);
        let (_, bad) = packets(&format!("{variant}-truncated"));
        let mut decoder = LayeredDecoder::from_configuration(&config, 1, BUDGET).unwrap();
        let mut queue = LayeredOutputQueue::default();
        for (index, packet) in good[..2].iter().enumerate() {
            decoder
                .decode_selected_queued_packet(
                    &mut queue,
                    source.selection_at(index).unwrap(),
                    packet,
                    2,
                )
                .unwrap();
        }
        let active = decoder.parameters().active(0).unwrap().clone();
        let poc = decoder.poc().clone();
        let error = decoder
            .decode_selected_queued_packet(&mut queue, source.selection_at(2).unwrap(), &bad[2], 2)
            .err()
            .unwrap()
            .to_string();
        assert!(
            error.contains("truncated or oversized bit field"),
            "{error}"
        );
        assert!(Arc::ptr_eq(
            decoder.parameters().active(0).unwrap(),
            &active
        ));
        assert_eq!(decoder.poc(), &poc);
        assert_eq!(decoder.next_access_unit(), 2);
        assert_eq!(queue.pending_access_units(), 2);
        let (released, _) = decoder
            .decode_selected_queued_packet(&mut queue, source.selection_at(2).unwrap(), &good[2], 2)
            .unwrap();
        assert_eq!(
            released
                .iter()
                .map(|f| (f.access_unit, f.layer_id))
                .collect::<Vec<_>>(),
            [
                (0, 0),
                (0, 3),
                (1, 0),
                (1, 3),
                (2, if variant == "graph" { 4 } else { 3 })
            ]
        );
        assert_ne!(decoder.parameters().active(0).unwrap().vps, active.vps);
        assert_eq!(
            active.vps.extension.as_ref().unwrap().output.sets[1].output,
            9
        );
    }
}

#[test]
fn unchanged_vps_keeps_global_cvsg_extraction_inventory_across_eos() {
    let source = reader("multiple", 0);
    let (_, packets) = packets("multiple");
    let old = source
        .selection_at(0)
        .unwrap()
        .extract_packet(&packets[0], 4, BUDGET)
        .unwrap();
    assert!(
        !NalUnits::new(&old, 4)
            .unwrap()
            .any(|n| n.unwrap()[0] >> 1 == 39)
    );
    assert!(
        source
            .selection_at(2)
            .unwrap()
            .extract_packet(&packets[2], 4, BUDGET)
            .unwrap()
            .is_empty()
    );
}

#[test]
fn metadata_only_packet_preserves_old_queue_until_new_temporal_epoch_picture() {
    let source = reader("short", 2);
    let (config, packets) = packets("short");
    let mut decoder = LayeredDecoder::from_configuration(&config, 1, BUDGET).unwrap();
    let mut queue = LayeredOutputQueue::default();
    for index in 0..2 {
        decoder
            .decode_selected_queued_packet(
                &mut queue,
                source.selection_at(index).unwrap(),
                &packets[index],
                2,
            )
            .unwrap();
    }
    assert_eq!(queue.pending_access_units(), 2);
    let old = queue.clone().drain();
    let (frames, _) = decoder
        .decode_selected_queued_packet(&mut queue, source.selection_at(2).unwrap(), &packets[2], 1)
        .unwrap();
    assert!(frames.is_empty());
    assert_eq!(queue.pending_access_units(), 2);
    assert_eq!(decoder.next_access_unit(), 3);
    for (before, after) in old.iter().zip(queue.clone().drain()) {
        assert!(Arc::ptr_eq(&before.picture, &after.picture));
    }
    let (frames, _) = decoder
        .decode_selected_queued_packet(&mut queue, source.selection_at(3).unwrap(), &packets[3], 1)
        .unwrap();
    assert_eq!(frames.len(), 5);
}

#[test]
fn changed_dependency_graph_starts_new_poc_history_and_preserves_old_outputs() {
    let oracle = std::fs::read(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/playback-errors/hevc-pcm-reference-rext8.yuv"),
    )
    .unwrap();
    let mut source = reader("graph", 2);
    let mut identities = Vec::new();
    let mut old_binding = None;
    for _ in 0..2 {
        identities.clear();
        while let Some(frame) = source.read_frame().unwrap() {
            identities.push((
                frame.sample_index,
                frame.decoded.layer_id,
                frame.decoded.poc,
            ));
            if frame.sample_index == 0 {
                old_binding = Some(frame.decoded.binding.clone());
            }
            let packed = frame.into_video_frame().unwrap().packed.unwrap();
            assert_eq!(&packed.frame.data, &oracle[..packed.frame.data.len()]);
        }
        assert_eq!(
            identities,
            [
                (0, 0, 0),
                (0, 3, 0),
                (1, 0, 1),
                (1, 3, 1),
                (2, 4, 0),
                (3, 4, 1)
            ]
        );
        assert_eq!(
            old_binding
                .as_ref()
                .unwrap()
                .vps
                .extension
                .as_ref()
                .unwrap()
                .graph
                .layer_ids,
            [0, 3]
        );
        source.rewind();
    }
}

#[test]
fn common_player_seek_and_rewind_select_changed_graph_layer() {
    for variant in ["graph", "idr", "idr-graph"] {
        let next_layer = if variant == "idr" { 3 } else { 4 };
        let bytes = std::fs::read(path(variant)).unwrap();
        let mut source =
            Mp4VideoReader::open_software(Cursor::new(bytes), Limits::default(), BUDGET).unwrap();
        for _ in 0..2 {
            for (sample, layer) in [(0, 3), (1, 3), (2, next_layer), (3, next_layer)] {
                assert_eq!(source.read_frame().unwrap().unwrap().sample_index, sample);
                assert_eq!(source.selected_layer_id(), Some(layer));
            }
            assert!(source.read_frame().unwrap().is_none());
            assert_eq!(source.seek_to_sync(2), 2);
            assert_eq!(source.read_frame().unwrap().unwrap().sample_index, 2);
            assert_eq!(source.selected_layer_id(), Some(next_layer));
            source.rewind();
        }
    }
}

#[test]
fn multiple_vps_ids_select_actual_slice_parameter_chain() {
    let oracle = std::fs::read(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/playback-errors/hevc-pcm-reference-rext8.yuv"),
    )
    .unwrap();
    for variant in ["ids", "ids-split", "ids-unused"] {
        for tid in [0, 2, 6] {
            let mut source = reader(variant, tid);
            let mut identities = Vec::new();
            for _ in 0..2 {
                identities.clear();
                while let Some(frame) = source.read_frame().unwrap() {
                    assert_eq!(frame.presentation_time.ticks, frame.sample_index as i64);
                    assert_eq!(frame.duration.ticks, 1);
                    identities.push((
                        frame.sample_index,
                        frame.decoded.layer_id,
                        frame.decoded.binding.vps.vps.id,
                    ));
                    let packed = frame.into_video_frame().unwrap().packed.unwrap();
                    assert_eq!(&packed.frame.data, &oracle[..packed.frame.data.len()]);
                }
                let mut expected = vec![(0, 0, 0), (0, 3, 0), (1, 0, 0), (1, 3, 0)];
                if variant != "ids-unused" {
                    expected.extend([(3, 3, 1), (4, 3, 1)]);
                }
                assert_eq!(identities, expected, "{variant}");
                source.rewind();
            }
        }
    }
}

#[test]
fn common_player_seek_uses_referenced_vps_id() {
    for variant in ["ids", "ids-split", "ids-unused"] {
        let bytes = std::fs::read(path(variant)).unwrap();
        let mut source =
            Mp4VideoReader::open_software(Cursor::new(bytes), Limits::default(), BUDGET).unwrap();
        let expected = if variant == "ids-unused" {
            vec![0, 1]
        } else {
            vec![0, 1, 3, 4]
        };
        for _ in 0..2 {
            let mut actual = Vec::new();
            while let Some(frame) = source.read_frame().unwrap() {
                actual.push(frame.sample_index);
            }
            assert_eq!(actual, expected);
            let target = if variant == "ids-unused" { 0 } else { 3 };
            assert_eq!(source.seek_to_sync(target as i64), target as i64);
            assert_eq!(source.read_frame().unwrap().unwrap().sample_index, target);
            assert_eq!(source.selected_layer_id(), Some(3));
            source.rewind();
        }
    }
}

#[test]
fn header_vps_reference_lookup_does_not_activate_or_mutate_parameters() {
    use fvid::codec::{hevc_decoder::LayeredParameters, hevc_nal::NalHeader};
    let (config, packets) = packets("ids");
    let mut parameters = LayeredParameters::from_configuration(&config, BUDGET).unwrap();
    for (index, packet) in packets.iter().enumerate() {
        for nal in NalUnits::new(packet, 4).unwrap() {
            let nal = nal.unwrap();
            let header = NalHeader::parse(nal).unwrap();
            if matches!(header.unit_type, 32..=34) {
                parameters.receive(nal).unwrap();
            }
            if header.is_vcl() {
                let bytes = parameters.storage_bytes();
                assert_eq!(
                    parameters.referenced_vps(nal).unwrap()[2] >> 4,
                    if index < 3 { 0 } else { 1 }
                );
                assert_eq!(parameters.storage_bytes(), bytes);
                assert!(parameters.active(header.layer_id).is_none());
            }
        }
    }
}

#[test]
fn complete_target_idr_activates_vps_and_graph_with_exact_pixels() {
    let oracle = std::fs::read(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/playback-errors/hevc-pcm-reference-rext8.yuv"),
    )
    .unwrap();
    for variant in ["idr", "idr-graph"] {
        let layer = if variant == "idr-graph" { 4 } else { 3 };
        for tid in [0, 2, 6] {
            let mut source = reader(variant, tid);
            for _ in 0..2 {
                let mut identities = Vec::new();
                while let Some(frame) = source.read_frame().unwrap() {
                    assert_eq!(frame.presentation_time.ticks, frame.sample_index as i64);
                    identities.push((
                        frame.sample_index,
                        frame.decoded.layer_id,
                        frame.decoded.poc,
                    ));
                    let packed = frame.into_video_frame().unwrap().packed.unwrap();
                    assert_eq!(&packed.frame.data, &oracle[..packed.frame.data.len()]);
                }
                assert_eq!(
                    identities,
                    [
                        (0, 0, 0),
                        (0, 3, 0),
                        (1, 0, 1),
                        (1, 3, 1),
                        (2, layer, 0),
                        (3, layer, 1)
                    ]
                );
                source.rewind();
            }
        }
    }
}

#[test]
fn incomplete_target_idr_cannot_replace_active_vps() {
    for variant in ["idr-missing", "idr-mixed"] {
        let mut source = reader(variant, 2);
        let error = loop {
            match source.read_frame() {
                Ok(Some(_)) => {}
                Ok(None) => panic!("expected activation refusal"),
                Err(e) => break e.to_string(),
            }
        };
        assert!(
            error.contains("HEVC active VPS changed before sequence end"),
            "{variant}: {error}"
        );
    }
}

#[test]
fn coded_cross_layer_bla_activates_vps_before_later_enhancement_cra() {
    let oracle = std::fs::read(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/playback-errors/hevc-pcm-reference-rext8.yuv"),
    )
    .unwrap();
    for variant in ["cross-bla", "cross-bla-graph"] {
        let next_layer = if variant == "cross-bla-graph" { 4 } else { 3 };
        for tid in [0, 2, 6] {
            let mut source = reader(variant, tid);
            for _ in 0..2 {
                let mut actual = Vec::new();
                while let Some(frame) = source.read_frame().unwrap() {
                    assert_eq!(frame.presentation_time.ticks, frame.sample_index as i64);
                    actual.push((
                        frame.sample_index,
                        frame.decoded.layer_id,
                        frame.decoded.poc,
                    ));
                    let packed = frame.into_video_frame().unwrap().packed.unwrap();
                    assert_eq!(&packed.frame.data, &oracle[..packed.frame.data.len()]);
                }
                assert_eq!(
                    actual,
                    [
                        (0, 0, 0),
                        (0, 3, 0),
                        (1, 0, 1),
                        (1, 3, 1),
                        (3, next_layer, 1)
                    ]
                );
                source.rewind();
            }
        }
    }
}

#[test]
fn unset_cross_layer_bla_cannot_replace_active_vps() {
    let mut source = reader("cross-bla-unset", 2);
    let error = loop {
        match source.read_frame() {
            Ok(Some(_)) => {}
            Ok(None) => panic!("expected activation refusal"),
            Err(e) => break e.to_string(),
        }
    };
    assert!(
        error.contains("HEVC active VPS changed before sequence end"),
        "{error}"
    );
}

#[test]
fn failed_cross_layer_bla_preserves_old_parameters_poc_and_output() {
    let source = reader("cross-bla", 2);
    let (config, good) = packets("cross-bla");
    let (_, bad) = packets("cross-bla-truncated");
    let mut decoder = LayeredDecoder::from_configuration(&config, 1, BUDGET).unwrap();
    let mut queue = LayeredOutputQueue::default();
    for (index, packet) in good[..2].iter().enumerate() {
        decoder
            .decode_selected_queued_packet(
                &mut queue,
                source.selection_at(index).unwrap(),
                packet,
                2,
            )
            .unwrap();
    }
    let active = decoder.parameters().active(0).unwrap().clone();
    let poc = decoder.poc().clone();
    let old = queue.clone().drain();
    let error = decoder
        .decode_selected_queued_packet(&mut queue, source.selection_at(2).unwrap(), &bad[2], 2)
        .err()
        .unwrap()
        .to_string();
    assert!(
        error.contains("truncated or oversized bit field"),
        "{error}"
    );
    assert!(Arc::ptr_eq(
        decoder.parameters().active(0).unwrap(),
        &active
    ));
    assert_eq!(decoder.poc(), &poc);
    assert_eq!(decoder.next_access_unit(), 2);
    assert_eq!(queue.pending_access_units(), 2);
    let (released, _) = decoder
        .decode_selected_queued_packet(&mut queue, source.selection_at(2).unwrap(), &good[2], 2)
        .unwrap();
    assert_eq!(released.len(), 4);
    for (before, after) in old.iter().zip(&released) {
        assert!(Arc::ptr_eq(&before.binding, &after.binding));
        assert!(Arc::ptr_eq(&before.picture, &after.picture));
    }
    assert_ne!(decoder.parameters().active(0).unwrap().vps, active.vps);
    let (next, _) = decoder
        .decode_selected_queued_packet(&mut queue, source.selection_at(3).unwrap(), &good[3], 2)
        .unwrap();
    assert_eq!(
        next.iter()
            .map(|p| (p.access_unit, p.layer_id))
            .collect::<Vec<_>>(),
        [(3, 3)]
    );
}

#[test]
fn common_player_seek_crosses_coded_cross_layer_bla_with_delayed_output_layer() {
    for variant in ["cross-bla", "cross-bla-graph"] {
        let next_layer = if variant == "cross-bla-graph" { 4 } else { 3 };
        let bytes = std::fs::read(path(variant)).unwrap();
        let mut source =
            Mp4VideoReader::open_software(Cursor::new(bytes), Limits::default(), BUDGET).unwrap();
        for _ in 0..2 {
            let mut actual = Vec::new();
            while let Some(frame) = source.read_frame().unwrap() {
                actual.push((frame.sample_index, source.selected_layer_id()));
            }
            assert_eq!(actual, [(0, Some(3)), (1, Some(3)), (3, Some(next_layer))]);
            assert_eq!(source.seek_to_sync(2), 2);
            assert_eq!(source.read_frame().unwrap().unwrap().sample_index, 3);
            assert_eq!(source.selected_layer_id(), Some(next_layer));
            source.rewind();
        }
    }
}
