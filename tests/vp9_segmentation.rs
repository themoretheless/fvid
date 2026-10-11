use fvid::{
    codec::vp9_decoder::Decoder,
    container::webm::{Limits, WebmReader},
    playback_webm::WebmVideoReader,
};
use std::io::Cursor;
fn data(name: &str) -> Vec<u8> {
    std::fs::read(format!(
        "{}/tests/fixtures/playback-errors/{name}",
        env!("CARGO_MANIFEST_DIR")
    ))
    .unwrap()
}
fn cases() -> Vec<serde_json::Value> {
    serde_json::from_slice::<serde_json::Value>(&data("vp9-segmentation.json")).unwrap()["cases"]
        .as_array()
        .unwrap()
        .clone()
}
#[test]
fn segmentation_maps_temporal_hidden_updates_and_references_match_pixels() {
    for c in cases() {
        let name = c["file"].as_str().unwrap();
        let mut r = WebmReader::open(Cursor::new(data(name)), Limits::default()).unwrap();
        r.scan_all().unwrap();
        let packets: Vec<_> = (0..r.packets.len())
            .map(|i| r.read_packet(i).unwrap())
            .collect();
        let mut d = Decoder::new(32 << 20);
        for _ in 0..2 {
            d.reset();
            d.decode(&packets[0]).unwrap();
            d.decode(&packets[1]).unwrap();
            for (i, frame) in c["frames"].as_array().unwrap().iter().enumerate() {
                let picture = d
                    .decode(&packets[i + 2])
                    .unwrap_or_else(|e| panic!("{name} packet {}: {e}", i + 2));
                assert_eq!(picture.header.show_frame, frame["shown"].as_bool().unwrap());
                let actual: Vec<_> = picture
                    .picture
                    .planes
                    .iter()
                    .flat_map(|p| p.samples.iter().flat_map(|v| v.to_le_bytes()))
                    .collect();
                assert_eq!(
                    actual,
                    data(frame["oracle"].as_str().unwrap()),
                    "{name} packet {}",
                    i + 2
                );
            }
        }
    }
}
#[test]
fn segmented_webm_playback_keeps_hidden_map_seek_and_timestamps() {
    for c in cases() {
        let name = c["file"].as_str().unwrap();
        let depth = c["depth"].as_u64().unwrap() as u8;
        let mut p = WebmVideoReader::open_software(Cursor::new(data(name)), 32 << 20).unwrap();
        for pass in 0..3 {
            if pass == 2 {
                assert_eq!(p.seek_to_sync(800_000_000).unwrap(), 0);
            } else {
                p.rewind();
            }
            assert!(p.read_frame_planes().unwrap().is_some());
            for frame in c["frames"]
                .as_array()
                .unwrap()
                .iter()
                .filter(|f| f["shown"] == true)
            {
                let actual = p.read_frame_planes().unwrap().unwrap();
                let actual: Vec<_> = [actual.y, actual.cb, actual.cr]
                    .into_iter()
                    .flatten()
                    .collect();
                let expected: Vec<_> = data(frame["oracle"].as_str().unwrap())
                    .chunks_exact(2)
                    .map(|v| (u16::from_le_bytes([v[0], v[1]]) >> (depth - 8)) as u8)
                    .collect();
                assert_eq!(actual, expected, "{name}");
                assert_eq!(
                    p.frame_interval().unwrap().0,
                    u128::from(frame["pts"].as_u64().unwrap()),
                    "{name}"
                );
            }
            assert!(p.read_frame_planes().unwrap().is_none());
        }
    }
}

#[test]
fn segment_quantizer_adjusts_coloured_residuals_and_clamps_negative_indices() {
    let manifest: serde_json::Value =
        serde_json::from_slice(&data("vp9-segmentation-quant.json")).unwrap();
    let cases = manifest["cases"].as_array().unwrap();
    assert_eq!(cases.len(), 72);
    for case in cases {
        let name = case["file"].as_str().unwrap();
        let mut d = Decoder::new(32 << 20);
        for _ in 0..2 {
            d.reset();
            let frame = d
                .decode(&data(name))
                .unwrap_or_else(|e| panic!("{name}: {e}"));
            assert_eq!(
                frame.header.segmentation.features[0][0],
                Some(case["q"].as_i64().unwrap() as i16)
            );
            assert_eq!(
                frame.header.segmentation.absolute,
                case["absolute"].as_bool().unwrap()
            );
            let actual: Vec<_> = frame
                .picture
                .planes
                .iter()
                .flat_map(|p| p.samples.iter().flat_map(|v| v.to_le_bytes()))
                .collect();
            let expected = data(case["oracle"].as_str().unwrap());
            assert_eq!(actual.len(), expected.len());
            for (i, (a, e)) in actual.iter().zip(&expected).enumerate() {
                assert_eq!(a, e, "{name} byte {i}");
            }
        }
    }
}

#[test]
fn segment_filter_override_matches_equivalent_frame_level_and_zero_is_exact() {
    let manifest: serde_json::Value =
        serde_json::from_slice(&data("vp9-segmentation-filter.json")).unwrap();
    let mut changed = 0;
    for c in manifest["cases"].as_array().unwrap() {
        let name = c["file"].as_str().unwrap();
        let mut d = Decoder::new(32 << 20);
        let frame = d.decode(&data(name)).unwrap();
        d.reset();
        let control = d.decode(&data(c["control"].as_str().unwrap())).unwrap();
        let actual: Vec<_> = frame
            .picture
            .planes
            .iter()
            .flat_map(|p| p.samples.iter().flat_map(|v| v.to_le_bytes()))
            .collect();
        let expected: Vec<_> = control
            .picture
            .planes
            .iter()
            .flat_map(|p| p.samples.iter().flat_map(|v| v.to_le_bytes()))
            .collect();
        assert_eq!(actual.len(), expected.len());
        for (i, (a, e)) in actual.iter().zip(&expected).enumerate() {
            assert_eq!(a, e, "{name} byte {i}");
        }
        let unfiltered = data(c["unfiltered"].as_str().unwrap());
        if c["level"] == 0 {
            assert_eq!(actual, unfiltered, "{name}");
        } else if actual != unfiltered {
            changed += 1;
        }
    }
    assert!(
        changed > 0,
        "positive filter controls must visibly exercise filtering"
    );
}

#[test]
fn segment_map_clears_on_resize_and_error_resilience_and_stays_cleared() {
    let m: serde_json::Value =
        serde_json::from_slice(&data("vp9-segmentation-reset.json")).unwrap();
    assert_eq!(m["cases"].as_array().unwrap().len(), 24);
    for c in m["cases"].as_array().unwrap() {
        let name = c["file"].as_str().unwrap();
        let mut r = WebmReader::open(Cursor::new(data(name)), Limits::default()).unwrap();
        r.scan_all().unwrap();
        let packets: Vec<_> = (0..r.packets.len())
            .map(|i| r.read_packet(i).unwrap())
            .collect();
        let mut d = Decoder::new(32 << 20);
        for _ in 0..2 {
            d.reset();
            for p in &packets[..2] {
                d.decode(p).unwrap();
            }
            let seed = d.decode(&packets[2]).unwrap();
            assert!(
                seed.picture.planes.iter().all(|p| p
                    .samples
                    .iter()
                    .all(|&v| v == 1 << (seed.picture.depth - 1))),
                "{name}: nonzero map must select neutral reference"
            );
            for (i, field) in [(3, "oracle"), (4, "returned")] {
                let f = d
                    .decode(&packets[i])
                    .unwrap_or_else(|e| panic!("{name} packet {i}: {e}"));
                let side = if i == 3 && c["kind"] == "resize" {
                    32
                } else {
                    64
                };
                assert_eq!(f.picture.size, [side, side], "{name}");
                assert_eq!(
                    f.header.error_resilient,
                    i == 3 && c["kind"] == "error-resilient",
                    "{name}"
                );
                let actual: Vec<_> = f
                    .picture
                    .planes
                    .iter()
                    .flat_map(|p| p.samples.iter().flat_map(|v| v.to_le_bytes()))
                    .collect();
                let expected = data(c[field].as_str().unwrap());
                assert_eq!(actual.len(), expected.len(), "{name}");
                for (i, (a, e)) in actual.iter().zip(&expected).enumerate() {
                    assert_eq!(a, e, "{name} byte {i}");
                }
            }
        }
    }
}

#[test]
fn nonzero_base_quantizer_uses_segment_absolute_delta_and_dct_residuals() {
    let m: serde_json::Value =
        serde_json::from_slice(&data("vp9-segmentation-lossy.json")).unwrap();
    assert_eq!(m["cases"].as_array().unwrap().len(), 72);
    for c in m["cases"].as_array().unwrap() {
        let name = c["file"].as_str().unwrap();
        let mut d = Decoder::new(32 << 20);
        for _ in 0..2 {
            d.reset();
            let f = d
                .decode(&data(name))
                .unwrap_or_else(|e| panic!("{name}: {e}"));
            assert!(!f.header.lossless());
            assert_eq!(u64::from(f.header.base_q), c["base"].as_u64().unwrap());
            let actual: Vec<_> = f
                .picture
                .planes
                .iter()
                .flat_map(|p| p.samples.iter().flat_map(|v| v.to_le_bytes()))
                .collect();
            let expected = data(c["oracle"].as_str().unwrap());
            assert_eq!(actual.len(), expected.len());
            for (i, (a, e)) in actual.iter().zip(&expected).enumerate() {
                assert_eq!(a, e, "{name} byte {i}");
            }
        }
    }
}

#[test]
fn segmentation_reset_webm_replays_size_changes_and_independent_maps_after_seek() {
    let m: serde_json::Value =
        serde_json::from_slice(&data("vp9-segmentation-reset.json")).unwrap();
    for c in m["cases"].as_array().unwrap() {
        let name = c["file"].as_str().unwrap();
        let depth = c["depth"].as_u64().unwrap() as u8;
        let mut p = WebmVideoReader::open_software(Cursor::new(data(name)), 32 << 20).unwrap();
        for pass in 0..3 {
            if pass == 2 {
                assert_eq!(p.seek_to_sync(1_000_000_000).unwrap(), 0);
            } else {
                p.rewind();
            }
            assert!(p.read_frame_planes().unwrap().is_some());
            let seed = p.read_frame_planes().unwrap().unwrap();
            assert!(
                [seed.y, seed.cb, seed.cr]
                    .into_iter()
                    .flatten()
                    .all(|v| v == 128),
                "{name}"
            );
            for (i, field) in [(3, "oracle"), (4, "returned")] {
                let f = p.read_frame_planes().unwrap().unwrap();
                let side = if i == 3 && c["kind"] == "resize" {
                    32
                } else {
                    64
                };
                assert_eq!([f.width, f.height], [side, side]);
                let actual: Vec<_> = [f.y, f.cb, f.cr].into_iter().flatten().collect();
                let expected: Vec<_> = data(c[field].as_str().unwrap())
                    .chunks_exact(2)
                    .map(|v| (u16::from_le_bytes([v[0], v[1]]) >> (depth - 8)) as u8)
                    .collect();
                assert_eq!(actual.len(), expected.len());
                for (j, (a, e)) in actual.iter().zip(&expected).enumerate() {
                    assert_eq!(a, e, "{name} frame {i} byte {j}");
                }
                assert_eq!(p.frame_interval().unwrap().0, i * 333_000_000);
            }
            assert!(p.read_frame_planes().unwrap().is_none());
        }
    }
}
