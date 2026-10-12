//! Indexed source acceptance: owned layered decode, extraction and source clocks.
use fvid::container::mp4::Limits;
use fvid::playback::video::mp4::LayeredMp4VideoReader;
use std::{
    io::Cursor,
    path::{Path, PathBuf},
};
const BUDGET: usize = 64 << 20;
fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/playback-errors")
}
fn source(name: &str, tid: u8) -> LayeredMp4VideoReader<Cursor<Vec<u8>>> {
    LayeredMp4VideoReader::open(
        Cursor::new(std::fs::read(root().join(name)).unwrap()),
        Limits::default(),
        1,
        tid,
        BUDGET,
    )
    .unwrap()
}
#[test]
fn original_coded_layered_witnesses_reach_indexed_playback_and_packed_output() {
    let manifest: serde_json::Value = serde_json::from_slice(
        &std::fs::read(root().join("hevc-layered-reconstruction.json")).unwrap(),
    )
    .unwrap();
    for case in manifest["cases"].as_array().unwrap() {
        let name = case["file"].as_str().unwrap();
        let mut reader = source(name, 2);
        let oracle = std::fs::read(
            root()
                .join(case["seed"].as_str().unwrap())
                .with_extension("yuv"),
        )
        .unwrap();
        let layers = if case["output_highest"] == true {
            vec![3]
        } else {
            vec![0, 3]
        };
        let count = if case["inter"] == true { 2 } else { 1 };
        for _ in 0..2 {
            let mut identities = Vec::new();
            while let Some(frame) = reader
                .read_frame()
                .unwrap_or_else(|e| panic!("{name}: {e}"))
            {
                let p = &frame.decoded.picture;
                let bytes: usize = p.planes.iter().map(|p| p.samples().len()).sum();
                let pixels: Vec<_> = p
                    .planes
                    .iter()
                    .flat_map(|p| p.samples().iter().copied())
                    .collect();
                assert_eq!(
                    pixels,
                    oracle[frame.sample_index * bytes..(frame.sample_index + 1) * bytes]
                        .iter()
                        .map(|&v| u16::from(v))
                        .collect::<Vec<_>>(),
                    "{name}"
                );
                identities.push((frame.sample_index, frame.decoded.layer_id));
                assert_eq!(frame.presentation_time.ticks, frame.sample_index as i64);
                assert_eq!(frame.duration.ticks, 1);
                let index = frame.sample_index;
                let output = frame.into_video_frame().unwrap();
                let packed = output.packed.unwrap();
                assert_eq!(
                    packed.frame.data,
                    oracle[index * bytes..(index + 1) * bytes],
                    "{name}"
                );
            }
            assert_eq!(
                identities,
                (0..count)
                    .flat_map(|i| layers.iter().map(move |&l| (i, l)))
                    .collect::<Vec<_>>()
            );
            reader.rewind();
        }
    }
}
#[test]
fn reordered_outputs_keep_original_sample_pts_and_duration() {
    let mut reader = source("hevc-layered-output-reorder-ctts.mp4", 2);
    for _ in 0..2 {
        let mut out = Vec::new();
        while let Some(frame) = reader.read_frame().unwrap() {
            out.push((
                frame.presentation_time.ticks,
                frame.decoded.poc,
                frame.sample_index,
                frame.decoded.layer_id,
            ));
            assert_eq!(frame.duration.ticks, 1);
            assert_eq!(frame.presentation_time.timescale, 25);
        }
        assert_eq!(
            out,
            [
                (0, 0, 0, 0),
                (0, 0, 0, 3),
                (1, 1, 2, 0),
                (1, 1, 2, 3),
                (2, 2, 1, 0),
                (2, 2, 1, 3),
                (3, 3, 4, 0),
                (3, 3, 4, 3),
                (4, 4, 3, 0),
                (4, 4, 3, 3)
            ]
        );
        reader.rewind();
    }
}
#[test]
fn metadata_only_sample_extraction_and_seek_preserve_video_sample_identity() {
    let mut reader = source("hevc-layered-extraction-split-timing.mp4", 0);
    for seek in [false, true, false] {
        if seek {
            assert_eq!(reader.seek_to_sync(1), 1);
        } else {
            reader.rewind();
        }
        for layer in [0, 3] {
            let frame = reader.read_frame().unwrap().unwrap();
            assert_eq!(
                (
                    frame.sample_index,
                    frame.presentation_time.ticks,
                    frame.decoded.layer_id
                ),
                (1, 1, layer)
            );
        }
        assert!(reader.read_frame().unwrap().is_none());
    }
}
#[test]
fn seeking_to_later_irap_maps_decoder_relative_au_to_source_sample() {
    let mut reader = source("hevc-layered-output-bla-flush.mp4", 2);
    let first = reader.read_frame().unwrap().unwrap();
    assert_eq!(first.sample_index, 0);
    assert_eq!(reader.seek_to_sync(2), 2);
    for layer in [0, 3] {
        let frame = reader.read_frame().unwrap().unwrap();
        assert_eq!(
            (
                frame.sample_index,
                frame.presentation_time.ticks,
                frame.decoded.poc,
                frame.decoded.access_unit,
                frame.decoded.layer_id
            ),
            (2, 2, 2, 0, layer)
        );
    }
    assert!(reader.read_frame().unwrap().is_none());
    assert!(first.decoded.retained_bytes().unwrap() > 0);
}
#[test]
fn malformed_entropy_requires_source_reset_without_exposing_staged_output() {
    let mut reader = source("hevc-layered-output-dpb-fullness-truncated.mp4", 2);
    for _ in 0..2 {
        let error = reader.read_frame().err().unwrap();
        assert!(
            error
                .to_string()
                .contains("truncated or oversized bit field"),
            "{error}"
        );
        assert!(
            reader
                .read_frame()
                .err()
                .unwrap()
                .to_string()
                .contains("requires rewind")
        );
        reader.rewind();
    }
}

#[test]
fn common_mp4_entry_displays_highest_output_layer_with_composition_timing() {
    use fvid::playback::video::mp4::Mp4VideoReader;
    let bytes = std::fs::read(root().join("hevc-layered-output-reorder-ctts.mp4")).unwrap();
    let mut reader =
        Mp4VideoReader::open_software(Cursor::new(bytes), Limits::default(), BUDGET).unwrap();
    assert!(!reader.hardware_accelerated());
    for _ in 0..2 {
        let mut out = Vec::new();
        while let Some(frame) = reader.read_frame().unwrap() {
            assert_eq!(reader.selected_layer_id(), Some(3));
            assert!(frame.packed.is_some());
            out.push((
                frame.presentation_time.ticks,
                frame.duration.ticks,
                frame.sample_index,
            ));
        }
        assert_eq!(out, [(0, 1, 0), (1, 1, 2), (2, 1, 1), (3, 1, 4), (4, 1, 3)]);
        reader.rewind();
    }
}
#[test]
fn common_mp4_entry_displays_coded_alternative_layer_when_primary_is_suppressed() {
    use fvid::playback::video::mp4::Mp4VideoReader;
    let bytes =
        std::fs::read(root().join("hevc-layered-random-access-alternative-radl-ctts.mp4")).unwrap();
    let mut reader =
        Mp4VideoReader::open_software(Cursor::new(bytes), Limits::default(), BUDGET).unwrap();
    let mut out = Vec::new();
    while let Some(frame) = reader.read_frame().unwrap() {
        out.push((
            frame.presentation_time.ticks,
            frame.sample_index,
            reader.selected_layer_id().unwrap(),
        ));
    }
    assert_eq!(out, [(0, 1, 0), (1, 0, 3), (2, 2, 3), (3, 4, 3), (4, 3, 3)]);
}
#[test]
fn common_native_player_source_reads_rewinds_and_seeks_layered_hevc() {
    use fvid::playback::video::native::{NativeReader, RawFrame};
    use std::time::Duration;
    let bytes = std::fs::read(root().join("hevc-layered-output-reorder-ctts.mp4")).unwrap();
    let mut reader = NativeReader::software(Cursor::new(bytes), BUDGET).unwrap();
    assert_eq!(reader.video_codec(), "H.265");
    for _ in 0..2 {
        let mut count = 0;
        while let Some(frame) = reader.read_frame_raw().unwrap() {
            let RawFrame::Planar(p) = frame else {
                panic!("native packed output required");
            };
            assert!(!p.frame.data.is_empty());
            count += 1;
        }
        assert_eq!(count, 5);
        reader.rewind().unwrap();
    }
    assert!(matches!(
        reader.seek_raw(Duration::from_millis(80)).unwrap(),
        Some(RawFrame::Planar(_))
    ));
}

#[test]
fn layered_hdr_is_layer_local_persistent_and_attached_before_reordering() {
    let mut reader = source("hevc-layered-hdr-reorder-ctts.mp4", 2);
    for _ in 0..2 {
        let mut identities = Vec::new();
        while let Some(frame) = reader.read_frame().unwrap() {
            let layer = frame.decoded.layer_id;
            let light = frame.decoded.hdr.light;
            assert_eq!(
                (light.max_cll, light.max_fall),
                if layer == 0 {
                    (600., 120.)
                } else {
                    (1800., 350.)
                }
            );
            assert!(frame.decoded.hdr.mastering.is_none());
            identities.push((frame.presentation_time.ticks, frame.sample_index, layer));
        }
        assert_eq!(
            identities,
            [0, 2, 1, 4, 3]
                .into_iter()
                .enumerate()
                .flat_map(|(pts, index)| [0, 3].map(move |layer| (pts as i64, index, layer)))
                .collect::<Vec<_>>()
        );
        reader.rewind();
    }
}
#[test]
fn layered_hdr_new_clvs_clears_prior_layer_metadata_without_mutating_queued_frames() {
    let mut reader = source("hevc-layered-hdr-clvs-reset.mp4", 2);
    for _ in 0..2 {
        let mut identities = Vec::new();
        while let Some(frame) = reader.read_frame().unwrap() {
            let layer = frame.decoded.layer_id;
            let expected = match (frame.sample_index < 2, layer) {
                (true, 0) => (600., 120.),
                (true, 3) => (1800., 350.),
                (false, 0) => (900., 200.),
                (false, 3) => (0., 0.),
                _ => unreachable!(),
            };
            assert_eq!(
                (
                    frame.decoded.hdr.light.max_cll,
                    frame.decoded.hdr.light.max_fall
                ),
                expected
            );
            identities.push((frame.sample_index, layer));
        }
        assert_eq!(
            identities,
            (0..4)
                .flat_map(|index| [0, 3].map(move |layer| (index, layer)))
                .collect::<Vec<_>>()
        );
        reader.rewind();
    }
}
#[test]
fn layered_configuration_hdr_is_seeded_per_layer_and_restored_on_reset() {
    use fvid::{codec::hevc_decoder::LayeredDecoder, container::mp4::Mp4Reader};
    let bytes = std::fs::read(root().join("hevc-layered-reconstruct-pcm-dependent.mp4")).unwrap();
    let mut demux = Mp4Reader::open(Cursor::new(bytes), Limits::default()).unwrap();
    let mut config = demux.tracks()[0].configuration.clone();
    config[22] += 1;
    config.extend([39, 0, 3]);
    for (layer, peak, average) in [(0u8, 600u16, 120u16), (3, 1800, 350)] {
        let mut nal = vec![78, (layer << 3) | 1, 144, 4];
        nal.extend(peak.to_be_bytes());
        nal.extend(average.to_be_bytes());
        nal.push(128);
        config.extend((nal.len() as u16).to_be_bytes());
        config.extend(nal);
    }
    let display = fvid::color::hdr::MasteringDisplay::from_corners(
        (0.64, 0.33),
        (0.30, 0.60),
        (0.15, 0.06),
        (0.3127, 0.3290),
        1000.,
        0.005,
    )
    .unwrap();
    let mut mastering = fvid::codec::hevc_sei::output_hdr_nal(&fvid::color::hdr::HdrMetadata {
        mastering: Some(display),
        ..Default::default()
    })
    .unwrap();
    mastering[1] = (3 << 3) | 1;
    config.extend((mastering.len() as u16).to_be_bytes());
    config.extend(mastering);
    let mut packet = Vec::new();
    demux.read_packet(0, 0, &mut packet).unwrap();
    let mut decoder = LayeredDecoder::from_configuration(&config, 1, BUDGET).unwrap();
    for _ in 0..2 {
        let (frames, _) = decoder.decode_packet(&packet).unwrap();
        assert_eq!(frames.len(), 2);
        for frame in frames {
            if frame.layer_id == 0 {
                assert!(frame.hdr.mastering.is_none());
            } else {
                assert_eq!(frame.hdr.mastering.unwrap().max_luminance, 1000.);
            }
            let expected = if frame.layer_id == 0 {
                (600., 120.)
            } else {
                (1800., 350.)
            };
            assert_eq!(
                (frame.hdr.light.max_cll, frame.hdr.light.max_fall),
                expected
            );
        }
        decoder.reset();
    }
}
