use fvid::{container::mp4::Limits, playback::video::mp4::LayeredMp4VideoReader};
#[test]
fn coded_depth_iv_selects_camera_source_and_native_merge_motion() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/hevc-wedge");
    let manifest: serde_json::Value =
        serde_json::from_slice(&std::fs::read(root.join("depth-iv-manifest.json")).unwrap())
            .unwrap();
    let cases = manifest["cases"].as_array().unwrap();
    assert_eq!(cases.len(), 26);
    for case in cases {
        let name = case["file"].as_str().unwrap();
        let size = case["size"].as_u64().unwrap() as usize;
        let expected = case["shift"].as_i64().unwrap() as i32;
        let data = std::fs::read(root.join(name)).unwrap();
        let mut reader = LayeredMp4VideoReader::open(
            std::io::Cursor::new(data.clone()),
            Limits::default(),
            1,
            0,
            64 << 20,
        )
        .unwrap_or_else(|e| panic!("{name}: {e}"));
        for _ in 0..2 {
            let mut seen = Vec::new();
            while let Some(frame) = reader
                .read_frame()
                .unwrap_or_else(|e| panic!("{name}: {e}"))
            {
                let id = frame.decoded.layer_id;
                let poc = frame.decoded.poc;
                seen.push((id, poc));
                let shift = if poc == 1 && id == 8 {
                    expected
                } else if poc == 1 && id == 3 && !case["intra_source"].as_bool().unwrap() {
                    1
                } else {
                    0
                };
                if id == 8 && poc == 1 {
                    for y in (0..size).step_by(8) {
                        for x in (0..size).step_by(8) {
                            let motion = frame
                                .decoded
                                .picture
                                .prediction_motion_at([x as u32, y as u32])
                                .unwrap();
                            assert_eq!(
                                motion[0].unwrap().mv,
                                [0, case["motion"].as_i64().unwrap() * 4],
                                "{name}: MV{x},{y}"
                            );
                            if size == 32 {
                                assert_eq!(
                                    motion[0].unwrap().reference,
                                    case["reference"].as_u64().unwrap() as u8,
                                    "{name}: reference"
                                );
                            }
                            assert!(motion[1].is_none());
                        }
                    }
                }
                for y in 0..size {
                    for x in 0..size {
                        let shift = if id == 3
                            && poc == 1
                            && case["source_split"].as_bool() == Some(true)
                        {
                            if x < 8 { 1 } else { -1 }
                        } else {
                            shift
                        };
                        let row = (y as i32 + shift).clamp(0, size as i32 - 1) as usize;
                        let pixel = if id == 0 {
                            128
                        } else {
                            ((x * 7 + row * 11 + 37) % 256) as u16
                        };
                        assert_eq!(
                            frame.decoded.picture.planes[0].samples()[y * size + x],
                            pixel,
                            "{name}: layer{id} POC{poc} pixel{x},{y}"
                        );
                    }
                }
                assert_eq!(frame.presentation_time.ticks, i64::from(poc));
                assert_eq!(frame.duration.ticks, 1);
            }
            assert_eq!(
                seen,
                [
                    (0, 0),
                    (3, 0),
                    (5, 0),
                    (8, 0),
                    (0, 1),
                    (3, 1),
                    (5, 1),
                    (8, 1)
                ]
            );
            reader.rewind();
        }
        let expected_y = |index: usize| -> Vec<u8> {
            (0..size)
                .flat_map(|y| {
                    (0..size).map(move |x| {
                        let row = if index == 0 {
                            y
                        } else {
                            (y as i32 + expected).clamp(0, size as i32 - 1) as usize
                        };
                        ((x * 7 + row * 11 + 37) % 256) as u8
                    })
                })
                .collect()
        };
        let mut common = fvid::playback::video::mp4::Mp4VideoReader::open_software(
            std::io::Cursor::new(data.clone()),
            Limits::default(),
            64 << 20,
        )
        .unwrap();
        for _ in 0..2 {
            let mut count = 0;
            while let Some(frame) = common.read_frame().unwrap() {
                assert_eq!(common.selected_layer_id(), Some(8));
                let packed = frame.packed.unwrap();
                assert_eq!(
                    &packed.frame.data[..size * size],
                    expected_y(frame.sample_index),
                    "{name}: common player"
                );
                assert!(packed.frame.data[size * size..].iter().all(|&p| p == 128));
                count += 1;
            }
            assert_eq!(count, 2);
            common.rewind();
        }
        use fvid::playback::video::native::{NativeReader, RawFrame};
        let mut native = NativeReader::software(std::io::Cursor::new(data), 64 << 20).unwrap();
        let frame = native
            .seek_raw(std::time::Duration::from_millis(40))
            .unwrap()
            .unwrap();
        let RawFrame::Planar(packed) = frame else {
            panic!("expected native planar depth IV")
        };
        assert_eq!(
            &packed.frame.data[..size * size],
            expected_y(1),
            "{name}: seek"
        );
    }
}

#[test]
fn depth_iv_requires_camera_when_default_view_is_available() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/hevc-wedge/depth-iv-missing-camera.mp4");
    let mut reader = LayeredMp4VideoReader::open(
        std::io::Cursor::new(std::fs::read(path).unwrap()),
        Limits::default(),
        1,
        0,
        64 << 20,
    )
    .unwrap();
    let error = loop {
        match reader.read_frame() {
            Ok(Some(_)) => {}
            Err(error) => break error,
            Ok(None) => panic!("missing required camera accepted"),
        }
    };
    assert_eq!(
        error.to_string(),
        "HEVC depth IV requires camera parameters for default reference view"
    );
}
