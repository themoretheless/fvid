use fvid::{container::mp4::Limits, playback::video::mp4::LayeredMp4VideoReader};
#[test]
fn mixed_depth_skip_mp4_reaches_queued_playback_and_rewind() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/hevc-wedge");
    let base_reference =
        std::fs::read(root.parent().unwrap().join("hevc-pcm-reference-rext8.yuv")).unwrap();
    for mode in 0..4 {
        let mut reader = LayeredMp4VideoReader::open(
            std::io::Cursor::new(
                std::fs::read(root.join(format!("skip-playback-mode{mode}.mp4"))).unwrap(),
            ),
            Limits::default(),
            1,
            0,
            64 << 20,
        )
        .unwrap();
        let raw = std::fs::read(root.join(format!("mixed-skip-mode{mode}-high.y16"))).unwrap();
        let expected: Vec<_> = raw
            .chunks_exact(2)
            .map(|b| u16::from_le_bytes([b[0], b[1]]))
            .collect();
        for _ in 0..2 {
            let mut layers = Vec::new();
            while let Some(frame) = reader.read_frame().unwrap() {
                layers.push(frame.decoded.layer_id);
                assert_eq!(frame.presentation_time.ticks, 0);
                if frame.decoded.layer_id == 3 {
                    assert_eq!(frame.decoded.picture.planes[0].samples(), expected);
                    assert_eq!(frame.decoded.picture.depth, [10; 2]);
                } else {
                    let samples: Vec<_> = frame
                        .decoded
                        .picture
                        .planes
                        .iter()
                        .flat_map(|p| p.samples().iter().copied())
                        .collect();
                    assert!(
                        samples
                            .iter()
                            .zip(&base_reference)
                            .all(|(&v, &r)| v == u16::from(r))
                    );
                    assert_eq!(samples.len(), 64 * 64 * 3 / 2);
                }
            }
            assert_eq!(layers, [0, 3]);
            reader.rewind();
        }
    }
}

#[test]
fn wedge_dc_only_mp4_reaches_depth_output_and_rewind() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/hevc-wedge");
    for variant in [
        "wedge",
        "wedge-dlt",
        "ordinary-dc-present",
        "ordinary-dc-absent",
        "ordinary-dc-present-dlt",
        "ordinary-dc-absent-dlt",
    ] {
        let mut reader = LayeredMp4VideoReader::open(
            std::io::Cursor::new(
                std::fs::read(root.join(format!("{variant}-playback.mp4"))).unwrap(),
            ),
            Limits::default(),
            1,
            0,
            64 << 20,
        )
        .unwrap();
        let reference = if variant == "wedge" {
            "wedge-picture.y16".to_string()
        } else {
            format!("{variant}.y16")
        };
        let raw = std::fs::read(root.join(reference)).unwrap();
        let expected: Vec<_> = raw
            .chunks_exact(2)
            .map(|b| u16::from_le_bytes([b[0], b[1]]))
            .collect();
        for _ in 0..2 {
            let mut layers = Vec::new();
            while let Some(frame) = reader.read_frame().unwrap() {
                layers.push(frame.decoded.layer_id);
                if frame.decoded.layer_id == 3 {
                    assert_eq!(frame.decoded.picture.dimensions, [32, 32]);
                    assert_eq!(frame.decoded.picture.planes[0].samples(), expected);
                    assert!(frame.decoded.picture.planes[0].complete());
                    assert_eq!(frame.presentation_time.ticks, 0);
                }
            }
            assert_eq!(layers, [0, 3]);
            reader.rewind();
        }
    }
}
