use fvid::{container::mp4::Limits, playback::video::mp4::LayeredMp4VideoReader};
#[test]
fn forced_dc_only_prediction_splits_apply_offset_once_after_all_children() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/hevc-wedge");
    for maximum in [2, 3, 4, 5] {
        for dlt in [false, true] {
            let name = format!(
                "ordinary-dc-split-tb{maximum}{}-playback.mp4",
                if dlt { "-dlt" } else { "" }
            );
            let mut reader = LayeredMp4VideoReader::open(
                std::io::Cursor::new(std::fs::read(root.join(name)).unwrap()),
                Limits::default(),
                1,
                0,
                64 << 20,
            )
            .unwrap();
            for _ in 0..2 {
                let mut layers = Vec::new();
                while let Some(frame) = reader.read_frame().unwrap() {
                    layers.push(frame.decoded.layer_id);
                    if frame.decoded.layer_id == 3 {
                        assert_eq!(frame.decoded.picture.dimensions, [64, 64]);
                        assert!(frame.decoded.picture.planes[0].complete());
                        assert!(
                            frame.decoded.picture.planes[0]
                                .samples()
                                .iter()
                                .all(|&v| v == if dlt { 136 } else { 129 })
                        );
                    }
                }
                assert_eq!(layers, [0, 3]);
                reader.rewind();
            }
        }
    }
}
