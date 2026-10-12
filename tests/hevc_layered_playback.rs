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
