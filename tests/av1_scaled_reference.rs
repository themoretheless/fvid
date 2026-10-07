use fvid::codec::av1_decoder::Decoder;
use std::io::Cursor;
fn root() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/playback-errors")
}
fn check_scaled_webm(r: &serde_json::Value) {
    let name = r["webm"].as_str().unwrap();
    let data = std::fs::read(root().join(name)).unwrap();
    let expected = std::fs::read(root().join(r["reference"].as_str().unwrap())).unwrap();
    let mut reader =
        fvid::playback_webm::WebmVideoReader::open(Cursor::new(&data), 16 << 20).unwrap();
    let frame_bytes = expected.len() / 2;
    for _ in 0..2 {
        for i in 0..2 {
            let f = reader
                .read_frame_planes()
                .unwrap_or_else(|e| panic!("{name}: {e}"))
                .unwrap();
            assert_eq!(
                f.y.iter()
                    .chain(&f.cb)
                    .chain(&f.cr)
                    .copied()
                    .collect::<Vec<_>>(),
                expected[i * frame_bytes..(i + 1) * frame_bytes],
                "{name} {i}"
            );
            assert_eq!(
                reader.dimensions(),
                [
                    r["current_size"][0].as_u64().unwrap() as usize,
                    r["current_size"][1].as_u64().unwrap() as usize
                ]
            );
            assert_eq!(
                reader.frame_interval(),
                Some((
                    i as u128 * 20_000_000,
                    (i as u128 + 1) * 20_000_000,
                    1_000_000_000
                ))
            );
        }
        assert!(reader.read_frame_planes().unwrap().is_none());
        reader.rewind();
    }
    assert_eq!(reader.seek_to_sync(0).unwrap(), 0);
    let first = reader.read_frame_planes().unwrap().unwrap();
    assert_eq!(
        first
            .y
            .iter()
            .chain(&first.cb)
            .chain(&first.cr)
            .copied()
            .collect::<Vec<_>>(),
        expected[..frame_bytes],
        "{name}: seek pixels"
    );
}

#[test]
fn scaled_reference_pixels_maps_reset_webm_and_seek_match_oracle() {
    let m: serde_json::Value = serde_json::from_slice(
        &std::fs::read(root().join("av1-scaled-reference-generated.json")).unwrap(),
    )
    .unwrap();
    let records = m["fixtures"].as_array().unwrap();
    assert_eq!(records.len(), 768);
    for filter in 0..4 {
        assert_eq!(
            records
                .iter()
                .filter(|r| r["interpolation"] == filter)
                .count(),
            192
        );
    }
    for case in 0..12 {
        assert_eq!(records.iter().filter(|r| r["case"] == case).count(), 64);
    }
    for r in records {
        let name = r["file"].as_str().unwrap();
        let bytes = std::fs::read(root().join(name)).unwrap();
        let expected = std::fs::read(root().join(r["reference"].as_str().unwrap())).unwrap();
        let mut decoder = Decoder::new(16 << 20);
        for _ in 0..2 {
            let frames = decoder
                .decode_packet(&bytes)
                .unwrap_or_else(|e| panic!("{name}: {e}"));
            assert_eq!(frames.len(), 3, "{name}");
            assert!(!frames[0].show && frames[1].show && frames[2].show);
            for (index, frame) in frames.iter().enumerate() {
                assert_eq!(frame.picture.depth, 8);
                let size = if index == 0 {
                    &r["reference_size"]
                } else {
                    &r["current_size"]
                };
                assert_eq!(
                    frame.picture.size,
                    [
                        size[0].as_u64().unwrap() as u32,
                        size[1].as_u64().unwrap() as u32
                    ],
                    "{name}: dimensions"
                );
                let map: Vec<u8> = r["maps"][index]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|v| v.as_u64().unwrap() as u8)
                    .collect();
                assert_eq!(frame.picture.segment_ids, map, "{name} frame {index}");
            }
            let actual: Vec<u8> = frames
                .iter()
                .filter(|f| f.show)
                .flat_map(|f| {
                    f.picture
                        .planes
                        .iter()
                        .flat_map(|p| p.samples.iter().map(|v| *v as u8))
                })
                .collect();
            assert_eq!(actual, expected, "{name}");
            decoder.reset();
        }
        check_scaled_webm(r);
    }
}

#[test]
fn out_of_range_reference_ratio_is_rejected() {
    let manifest: serde_json::Value = serde_json::from_slice(
        &std::fs::read(root().join("av1-scaled-reference-generated.json")).unwrap(),
    )
    .unwrap();
    for r in manifest["refusals"].as_array().unwrap() {
        let bytes = std::fs::read(root().join(r["file"].as_str().unwrap())).unwrap();
        let mut decoder = Decoder::new(16 << 20);
        let error = decoder
            .decode_packet(&bytes)
            .err()
            .expect("invalid reference ratio accepted")
            .to_string();
        assert!(error.contains(r["error"].as_str().unwrap()), "{error}");
    }
}
