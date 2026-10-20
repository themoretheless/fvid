use fvid::{
    codec::hevc_motion::disparity::{CameraTransform, samples},
    container::mp4::Limits,
    playback::video::mp4::LayeredMp4VideoReader,
};
#[test]
fn coded_depth_picture_drives_all_sample_derivation_modes() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/hevc-wedge");
    let manifest: serde_json::Value =
        serde_json::from_slice(&std::fs::read(root.join("disparity-sample-oracles.json")).unwrap())
            .unwrap();
    let bytes = std::fs::read(root.join("disparity-sample-oracles.i64")).unwrap();
    let expected: Vec<i64> = bytes
        .chunks_exact(8)
        .map(|v| i64::from_le_bytes(v.try_into().unwrap()))
        .collect();
    assert_eq!(bytes.len() % 8, 0);
    let movie = std::fs::read(root.join("disparity-depth-source.mp4")).unwrap();
    let mut reader = LayeredMp4VideoReader::open(
        std::io::Cursor::new(movie),
        Limits::default(),
        1,
        0,
        64 << 20,
    )
    .unwrap();
    let transform = CameraTransform::new([-3, -1, 2, 4], 2, 8).unwrap();
    let cases = manifest["cases"].as_array().unwrap();
    assert_eq!(cases.len(), 378);
    let mut checked = false;
    while let Some(frame) = reader.read_frame().unwrap() {
        if frame.decoded.poc != 0 {
            continue;
        }
        let picture = &frame.decoded.picture;
        if frame.decoded.layer_id == 0 {
            assert!(
                samples::from_picture(picture, [0, 0], [8, 8], [0, 0], 0, transform, 4096).is_err()
            );
            continue;
        }
        assert_eq!(picture.dimensions, [32, 32]);
        for y in 0..32 {
            for x in 0..32 {
                assert_eq!(
                    picture.planes[0].samples()[y * 32 + x],
                    ((x * 7 + y * 11 + 37) % 256) as u16
                );
            }
        }
        for case in cases {
            let block = [0, 1].map(|i| case["block"][i].as_u64().unwrap() as usize);
            let origin = [0, 1].map(|i| case["origin"][i].as_u64().unwrap() as u32);
            let vector = [0, 1].map(|i| case["vector"][i].as_i64().unwrap());
            let part = case["part"].as_u64().unwrap() as u8;
            let offset = case["offset"].as_u64().unwrap() as usize;
            let budget = samples::required_bytes(block).unwrap();
            let result =
                samples::from_picture(picture, origin, block, vector, part, transform, budget)
                    .unwrap();
            if part == 1 {
                // Raw depth derivation is available with no camera parameters.
                let raw =
                    samples::depth_from_picture(picture, origin, block, vector, budget).unwrap();
                assert_eq!(raw.values, result.values);
            }
            assert_eq!(result.dimensions, block);
            assert_eq!(
                result.horizontal_split,
                case["horizontal"].as_bool().unwrap()
            );
            assert_eq!(
                result.values,
                expected[offset..offset + block[0] * block[1]],
                "{case}"
            );
            assert!(
                samples::from_picture(picture, origin, block, vector, part, transform, budget - 1)
                    .is_err()
            );
        }
        let wrong = CameraTransform::new([1, 0, 0, 0], 0, 10).unwrap();
        assert!(samples::from_picture(picture, [0, 0], [8, 8], [0, 0], 0, wrong, 4096).is_err());
        checked = true;
    }
    assert!(checked);
}
