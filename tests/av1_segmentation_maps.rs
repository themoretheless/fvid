use fvid::codec::av1_decoder::Decoder;
use std::io::Cursor;
fn root() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/playback-errors")
}
fn manifest() -> serde_json::Value {
    serde_json::from_slice(&std::fs::read(root().join("av1-seg-map-generated.json")).unwrap())
        .unwrap()
}
#[test]
fn explicit_temporal_and_inherited_segment_maps_match_independent_pixels() {
    let m = manifest();
    let records = m["fixtures"].as_array().unwrap();
    assert_eq!(records.len(), 48);
    for r in records {
        let name = r["file"].as_str().unwrap();
        let data = std::fs::read(root().join(name)).unwrap();
        let expected = std::fs::read(root().join(r["reference"].as_str().unwrap())).unwrap();
        let mut d = Decoder::new(16 << 20);
        for _ in 0..2 {
            let frames = d
                .decode_packet(&data)
                .unwrap_or_else(|e| panic!("{name}: {e}"));
            assert_eq!(frames.len(), 7);
            assert!(!frames[0].show);
            for (i, f) in frames.iter().enumerate() {
                let map = r["maps"][i]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|v| v.as_u64().unwrap() as u8)
                    .collect::<Vec<_>>();
                assert_eq!(f.picture.segment_ids, map, "{name} frame {i}");
            }
            let actual = frames
                .iter()
                .filter(|f| f.show)
                .flat_map(|f| {
                    f.picture
                        .planes
                        .iter()
                        .flat_map(|p| p.samples.iter().copied())
                })
                .collect::<Vec<_>>();
            assert_eq!(
                actual,
                expected.iter().map(|&v| u16::from(v)).collect::<Vec<_>>(),
                "{name}"
            );
            d.reset();
        }
    }
}
#[test]
fn segment_map_webm_pixels_rewind_and_seek() {
    for r in manifest()["fixtures"].as_array().unwrap() {
        let name = r["webm"].as_str().unwrap();
        let data = std::fs::read(root().join(name)).unwrap();
        let expected = std::fs::read(root().join(r["reference"].as_str().unwrap())).unwrap();
        let mut reader =
            fvid::playback_webm::WebmVideoReader::open(Cursor::new(&data), 16 << 20).unwrap();
        for _ in 0..2 {
            for i in 0..6 {
                let f = reader.read_frame_planes().unwrap().unwrap();
                let pixels =
                    f.y.iter()
                        .chain(&f.cb)
                        .chain(&f.cr)
                        .copied()
                        .collect::<Vec<_>>();
                assert_eq!(pixels, expected[i * 1536..(i + 1) * 1536], "{name} {i}");
            }
            assert!(reader.read_frame_planes().unwrap().is_none());
            reader.rewind();
        }
        reader.seek_to_sync(0).unwrap();
        assert!(reader.read_frame_planes().unwrap().is_some());
    }
}

#[test]
fn previously_refused_flat_map_updates_accept() {
    let m: serde_json::Value = serde_json::from_slice(
        &std::fs::read(root().join("av1-seg-inherit-generated.json")).unwrap(),
    )
    .unwrap();
    for r in m["map_acceptances"].as_array().unwrap() {
        let data = std::fs::read(root().join(r["file"].as_str().unwrap())).unwrap();
        let frames = Decoder::new(8 << 20).decode_packet(&data).unwrap();
        assert_eq!(frames.len(), 2);
        assert!(frames[1].show);
        assert!(frames[1]
            .picture
            .planes
            .iter()
            .all(|p| p.samples.iter().all(|&v| v == 128)));
    }
}
