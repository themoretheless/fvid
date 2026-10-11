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
