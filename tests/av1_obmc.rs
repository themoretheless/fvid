use fvid::codec::av1_decoder::Decoder;
use std::io::Cursor;
fn root() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/playback-errors")
}
fn check_global_webm(r: &serde_json::Value) {
    let name = r["webm"].as_str().unwrap();
    let data = std::fs::read(root().join(name)).unwrap();
    let expected = std::fs::read(root().join(r["reference"].as_str().unwrap())).unwrap();
    let mut reader =
        fvid::playback_webm::WebmVideoReader::open(Cursor::new(&data), 16 << 20).unwrap();
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
                expected[i * 1536..(i + 1) * 1536],
                "{name} {i}"
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
        expected[..1536],
        "{name}: seek pixels"
    );
}

#[test]
fn obmc_pixels_maps_reset_webm_and_seek_match_oracle() {
    let m: serde_json::Value =
        serde_json::from_slice(&std::fs::read(root().join("av1-obmc-generated.json")).unwrap())
            .unwrap();
    let records = m["fixtures"].as_array().unwrap();
    assert_eq!(records.len(), 2048);
    for mask in [2, 4, 8, 14] {
        assert_eq!(records.iter().filter(|r| r["obmc"] == mask).count(), 512);
    }
    for movement in [0, 1] {
        assert_eq!(
            records.iter().filter(|r| r["movement"] == movement).count(),
            1024
        );
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
            assert_eq!(frames.len(), 4, "{name}");
            assert!(!frames[0].show && !frames[1].show && frames[2].show && frames[3].show);
            for (index, frame) in frames.iter().enumerate() {
                assert_eq!(frame.picture.depth, 8);
                let map: Vec<u8> = r["maps"][index]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|v| v.as_u64().unwrap() as u8)
                    .collect();
                assert_eq!(frame.picture.segment_ids, map, "{name} frame {index}");
            }
            if r["level"].as_u64().unwrap() >= 256 {
                let pixels: Vec<u16> = frames[..2]
                    .iter()
                    .flat_map(|f| f.picture.planes[0].samples.iter().copied())
                    .collect();
                assert!(
                    pixels.iter().max().unwrap() - pixels.iter().min().unwrap() >= 64,
                    "{name}: weak contrast reproducer"
                );
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
        check_global_webm(r);
    }
}
