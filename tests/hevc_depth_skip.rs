use fvid::codec::{
    hevc_cabac::{HevcCabac, SliceType},
    hevc_depth_dc::{read_skip_intra_flag, read_skip_intra_mode},
    hevc_depth_prediction::single,
};
#[test]
fn arithmetic_skip_intra_flags_and_modes() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/hevc-wedge");
    let manifest: serde_json::Value =
        serde_json::from_slice(&std::fs::read(root.join("depth-skip.json")).unwrap()).unwrap();
    for case in manifest["cases"].as_array().unwrap() {
        let bytes = std::fs::read(root.join(case["file"].as_str().unwrap())).unwrap();
        let slice = match case["slice"].as_str().unwrap() {
            "I" => SliceType::I,
            "P" => SliceType::P,
            _ => SliceType::B,
        };
        let mut bins = HevcCabac::new(
            &bytes,
            0,
            slice,
            case["swap"].as_bool().unwrap(),
            case["qp"].as_i64().unwrap() as i32,
        )
        .unwrap();
        for block in case["blocks"].as_array().unwrap() {
            let flag = read_skip_intra_flag(
                &mut bins,
                block["skipped"].as_bool().unwrap(),
                block["enabled"].as_bool().unwrap(),
            )
            .unwrap();
            assert_eq!(flag, !block["mode"].is_null());
            if flag {
                assert_eq!(
                    u64::from(read_skip_intra_mode(&mut bins).unwrap()),
                    block["mode"].as_u64().unwrap()
                );
            }
        }
        assert!(bins.terminate().unwrap());
    }
}
#[test]
fn single_prediction_uses_midpoint_availability_without_substitution() {
    for size in [4, 8, 16, 32, 64] {
        for depth in [8, 10, 12, 16] {
            for mode in [2, 3] {
                for available in [false, true] {
                    let mut top = vec![Some(19); size];
                    let mut left = vec![Some(211); size];
                    if mode == 2 {
                        left[size / 2] = if available { Some(37) } else { None };
                    } else {
                        top[size / 2] = if available { Some(67) } else { None };
                    }
                    let mut output = vec![0; size * size];
                    single(size, depth, mode, &top, &left, &mut output).unwrap();
                    let expected = if available {
                        if mode == 2 { 37 } else { 67 }
                    } else {
                        1 << (depth - 1)
                    };
                    assert!(output.iter().all(|&v| v == expected));
                }
            }
        }
    }
    let mut output = [123; 16];
    let mut top = [Some(0); 4];
    top[2] = Some(256);
    assert!(single(4, 8, 3, &top, &[None; 4], &mut output).is_err());
    assert_eq!(output, [123; 16]);
    assert!(single(4, 8, 1, &[None; 4], &[None; 4], &mut output).is_err());
    assert_eq!(output, [123; 16]);
}
