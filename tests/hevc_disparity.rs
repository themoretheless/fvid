use fvid::codec::hevc_motion::disparity::{CameraTransform, Disparity};
#[test]
fn camera_disparity_matches_independent_exact_fraction_oracles() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/hevc-wedge");
    let manifest: serde_json::Value =
        serde_json::from_slice(&std::fs::read(root.join("disparity-oracles.json")).unwrap())
            .unwrap();
    let cases = manifest["cases"].as_array().unwrap();
    assert_eq!(cases.len(), 270);
    for c in cases {
        let values: Vec<i32> = c["values"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_i64().unwrap().try_into().unwrap())
            .collect();
        let depth = c["depth"].as_u64().unwrap() as u8;
        let precision = c["precision"].as_u64().unwrap() as u8;
        let transform = CameraTransform::new(values.try_into().unwrap(), precision, depth).unwrap();
        assert_eq!(transform.sample_count(), 1u32 << depth);
        for p in c["points"].as_array().unwrap() {
            let sample = p[0].as_u64().unwrap() as u32;
            assert_eq!(
                transform.sample(sample).unwrap(),
                Disparity {
                    backward: p[1].as_i64().unwrap(),
                    forward: p[2].as_i64().unwrap()
                },
                "{c}: sample{sample}"
            );
        }
        assert!(transform.sample(1u32 << depth).is_err());
        if let Some(filename) = c["table"].as_str() {
            let data = std::fs::read(root.join(filename)).unwrap();
            assert_eq!(data.len(), 256 * 16);
            let needed = transform.required_table_bytes().unwrap();
            assert!(transform.table(needed - 1).is_err());
            let table = transform.table(needed).unwrap();
            for (sample, bytes) in data.chunks_exact(16).enumerate() {
                let expected = Disparity {
                    backward: i64::from_le_bytes(bytes[..8].try_into().unwrap()),
                    forward: i64::from_le_bytes(bytes[8..].try_into().unwrap()),
                };
                assert_eq!(
                    transform.sample(sample as u32).unwrap(),
                    expected,
                    "{filename}: direct {sample}"
                );
                assert_eq!(table[sample], expected, "{filename}: LUT {sample}");
            }
        }
        if depth == 16 && precision == 0 && c["name"] == "extreme" {
            let needed = transform.required_table_bytes().unwrap();
            assert!(transform.table(needed - 1).is_err());
            let table = transform.table(needed).unwrap();
            assert_eq!(table.len(), 65536);
            for p in c["points"].as_array().unwrap() {
                let d = p[0].as_u64().unwrap() as usize;
                assert_eq!(
                    table[d],
                    Disparity {
                        backward: p[1].as_i64().unwrap(),
                        forward: p[2].as_i64().unwrap()
                    }
                );
            }
        }
    }
}
