use fvid::codec::{av1::Obus, av1_decoder::Decoder, av1_frame::Header, av1_sequence::Sequence};
use std::io::Cursor;
fn root() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/playback-errors")
}
fn manifest() -> serde_json::Value {
    serde_json::from_slice(
        &std::fs::read(root().join("av1-mixed-lossless-generated.json")).unwrap(),
    )
    .unwrap()
}
#[test]
fn mixed_lossless_larger_blocks_decode_without_selected_transform_symbols() {
    let m = manifest();
    let fixtures = m["fixtures"].as_array().unwrap();
    assert_eq!(fixtures.len(), 504);
    for residual in [-1, 0, 1] {
        assert_eq!(
            fixtures
                .iter()
                .filter(|r| r["residual"].as_i64() == Some(residual))
                .count(),
            168
        );
    }
    for r in fixtures {
        let name = r["file"].as_str().unwrap();
        let data = std::fs::read(root().join(name)).unwrap();
        let expected = std::fs::read(root().join(r["reference"].as_str().unwrap())).unwrap();
        if r["residual"] != 0 {
            for plane in [&expected[..1024], &expected[1024..1280], &expected[1280..]] {
                assert!(
                    plane.iter().any(|&v| v != 128),
                    "{name}: residual must change every plane"
                );
            }
        }
        let map = r["map"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_u64().unwrap() as u8)
            .collect::<Vec<_>>();
        let mut sequence = None;
        for obu in Obus::new(&data) {
            let obu = obu.unwrap();
            if obu.kind == 1 {
                sequence = Some(Sequence::parse(obu.payload).unwrap());
            }
            if obu.kind == 6 {
                let h = Header::parse(sequence.as_ref().unwrap(), obu.payload, 0, 0, &[None; 8])
                    .unwrap();
                assert!(h.lossless[0]);
                assert!(!h.lossless[1]);
                assert_eq!(h.tx_mode, if r["selected"] == true { 2 } else { 1 });
                assert_eq!(
                    h.quant.base,
                    u8::try_from(r["base"].as_u64().unwrap()).unwrap()
                );
                assert_eq!(h.disable_cdf_update, r["adaptive"] == false);
            }
        }
        let mut d = Decoder::new(8 << 20);
        for _ in 0..2 {
            let frames = d
                .decode_packet(&data)
                .unwrap_or_else(|e| panic!("{name}: {e}"));
            assert_eq!(frames.len(), 2);
            assert!(!frames[0].show);
            assert!(frames[1].show);
            for f in frames {
                assert_eq!(f.picture.segment_ids, map, "{name}");
                let pixels = f
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
            }
            d.reset();
        }
    }
}
#[test]
fn mixed_lossless_webm_replay_and_seek_match_saved_pixels() {
    for r in manifest()["fixtures"].as_array().unwrap() {
        let name = r["webm"].as_str().unwrap();
        let data = std::fs::read(root().join(name)).unwrap();
        let expected = std::fs::read(root().join(r["reference"].as_str().unwrap())).unwrap();
        let mut reader =
            fvid::playback_webm::WebmVideoReader::open(Cursor::new(&data), 8 << 20).unwrap();
        for _ in 0..2 {
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
                expected,
                "{name}"
            );
            assert!(reader.read_frame_planes().unwrap().is_none());
            reader.rewind();
        }
        assert_eq!(reader.seek_to_sync(0).unwrap(), 0);
        assert!(reader.read_frame_planes().unwrap().is_some());
    }
}
