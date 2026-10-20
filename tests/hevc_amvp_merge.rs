use fvid::{container::mp4::Limits, playback::video::mp4::LayeredMp4VideoReader};
#[test]
fn amvp_copies_long_term_motion_while_merge_scales_by_view_distance() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/hevc-wedge");
    let manifest: serde_json::Value =
        serde_json::from_slice(&std::fs::read(root.join("amvp-merge-manifest.json")).unwrap())
            .unwrap();
    let cases = manifest["cases"].as_array().unwrap();
    assert_eq!(cases.len(), 8);
    for case in cases {
        let filename = case["file"].as_str().unwrap();
        let target = case["target_layer"].as_u64().unwrap() as u8;
        let source = case["source_layer"].as_u64().unwrap() as u8;
        let expected = case["expected_quarter_mv"].as_i64().unwrap() as i32;
        let depth = case["kind"].as_str().unwrap() == "depth";
        let mut reader = LayeredMp4VideoReader::open(
            std::io::Cursor::new(std::fs::read(root.join(filename)).unwrap()),
            Limits::default(),
            1,
            0,
            64 << 20,
        )
        .unwrap();
        for _ in 0..2 {
            let mut checked = 0;
            while let Some(frame) = reader.read_frame().unwrap() {
                if frame.decoded.poc != 1
                    || !matches!(frame.decoded.layer_id,id if id==source||id==target)
                {
                    continue;
                }
                let vector = if frame.decoded.layer_id == source {
                    4
                } else {
                    expected
                };
                for y in (0..32).step_by(8) {
                    for x in (0..32).step_by(8) {
                        let motion = frame.decoded.picture.prediction_motion_at([x, y]).unwrap();
                        assert_eq!(
                            motion[0].unwrap().mv,
                            [0, i64::from(vector)],
                            "{filename}: source checked, target motion at {x},{y}"
                        );
                        assert!(motion[1].is_none());
                    }
                }
                for y in 0..32 {
                    for x in 0..32 {
                        let row = (y + vector as usize / 4).min(31);
                        let value = if depth {
                            (x * 7 + row * 11 + 37) % 256
                        } else {
                            (x * 3 + row * 5 + 17) % 256
                        };
                        assert_eq!(
                            frame.decoded.picture.planes[0].samples()[y * 32 + x],
                            value as u16,
                            "{filename}: pixel {x},{y}"
                        );
                    }
                }
                assert_eq!(frame.presentation_time.ticks, 1);
                assert_eq!(frame.duration.ticks, 1);
                checked += 1;
            }
            assert_eq!(checked, 2);
            reader.rewind();
        }
    }
}
