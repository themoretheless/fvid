use fvid::codec::av1_decoder::Decoder;
use std::io::Cursor;
fn root() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/playback-errors")
}
fn manifest() -> serde_json::Value {
    serde_json::from_slice(&std::fs::read(root().join("av1-alt-q-generated.json")).unwrap())
        .unwrap()
}
#[test]
fn active_alt_q_nonzero_residuals_match_independent_pixels() {
    let manifest = manifest();
    let fixtures = manifest["fixtures"].as_array().unwrap();
    assert_eq!(fixtures.len(), 25);
    for record in fixtures {
        let name = record["file"].as_str().unwrap();
        let bytes = std::fs::read(root().join(name)).unwrap();
        let expected = std::fs::read(root().join(record["reference"].as_str().unwrap())).unwrap();
        assert_eq!(expected.len(), 1536);
        let mut decoder = Decoder::new(8 << 20);
        for _ in 0..2 {
            let frames = decoder
                .decode_packet(&bytes)
                .unwrap_or_else(|e| panic!("{name}: {e}"));
            assert_eq!(frames.len(), 2);
            assert!(!frames[0].show);
            assert!(frames[1].show);
            let frame = &frames[1];
            assert_eq!(frame.picture.size, [32, 32]);
            assert_eq!(frame.picture.depth, 8);
            let pixels = frame
                .picture
                .planes
                .iter()
                .flat_map(|p| p.samples.iter().copied())
                .collect::<Vec<_>>();
            assert_eq!(
                pixels,
                expected.iter().map(|&v| u16::from(v)).collect::<Vec<_>>(),
                "{name}"
            );
            decoder.reset();
        }
    }
}
#[test]
fn previously_refused_active_alt_q_stream_now_decodes() {
    let manifest: serde_json::Value = serde_json::from_slice(
        &std::fs::read(root().join("av1-seg-inherit-generated.json")).unwrap(),
    )
    .unwrap();
    let records = manifest["active_acceptances"].as_array().unwrap();
    assert_eq!(records.len(), 1);
    for record in records {
        assert_eq!(record["acceptance"], true);
        let bytes = std::fs::read(root().join(record["file"].as_str().unwrap())).unwrap();
        let mut decoder = Decoder::new(8 << 20);
        for _ in 0..2 {
            let frames = decoder.decode_packet(&bytes).unwrap();
            assert_eq!(frames.len(), 2);
            assert_eq!(frames.iter().filter(|f| f.show).count(), 1);
            assert!(frames.iter().all(|f| {
                f.picture
                    .planes
                    .iter()
                    .all(|p| p.samples.iter().all(|&v| v == 128))
            }));
            decoder.reset();
        }
    }
}
#[test]
fn alt_q_webm_pixels_timing_rewind_and_seek_match_oracle() {
    let mut count = 0;
    for record in manifest()["fixtures"].as_array().unwrap() {
        let Some(name) = record["webm"].as_str() else {
            continue;
        };
        let bytes = std::fs::read(root().join(name)).unwrap();
        let expected = std::fs::read(root().join(record["reference"].as_str().unwrap())).unwrap();
        let mut reader =
            fvid::playback_webm::WebmVideoReader::open(Cursor::new(&bytes), 8 << 20).unwrap();
        for _ in 0..2 {
            let p = reader
                .read_frame_planes()
                .unwrap_or_else(|e| panic!("{name}: {e}"))
                .unwrap();
            let pixels =
                p.y.iter()
                    .chain(&p.cb)
                    .chain(&p.cr)
                    .copied()
                    .collect::<Vec<_>>();
            assert_eq!(pixels, expected, "{name}");
            assert_eq!(
                reader.frame_interval(),
                Some((0, 20_000_000, 1_000_000_000))
            );
            assert!(reader.read_frame_planes().unwrap().is_none());
            reader.rewind();
        }
        assert_eq!(reader.seek_to_sync(0).unwrap(), 0);
        assert!(reader.read_frame_planes().unwrap().is_some());
        count += 1;
    }
    assert_eq!(count, 6);
}
