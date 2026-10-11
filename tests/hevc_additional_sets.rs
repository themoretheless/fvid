use fvid::codec::hevc_vps::{LayerDependencyPrefix, LayerSets};
fn data(name: &str) -> Vec<u8> {
    std::fs::read(format!(
        "{}/tests/fixtures/playback-errors/{name}",
        env!("CARGO_MANIFEST_DIR")
    ))
    .unwrap()
}
#[test]
fn annex_f_additional_sets_and_temporal_limits() {
    let manifest: serde_json::Value =
        serde_json::from_slice(&data("hevc-additional-sets.json")).unwrap();
    for c in manifest.as_array().unwrap() {
        let bytes = data(c["file"].as_str().unwrap());
        let layers = LayerSets {
            base_internal: true,
            base_available: c["available"].as_bool().unwrap(),
            max_layers_minus1: 3,
            max_layer_id: 9,
            sets: vec![1, 1 | 4 | 32 | 512],
        };
        let sublayers = c["sublayers"].as_u64().unwrap() as u8;
        let prefix = LayerDependencyPrefix::parse(&bytes, 0, &layers, sublayers, 1 << 20).unwrap();
        assert_eq!(prefix.tree_partitions, vec![vec![0, 5], vec![2, 9]]);
        let result = prefix.layer_sets(&bytes, &layers, sublayers, 1 << 20);
        if let Some(error) = c["error"].as_str() {
            assert!(result.unwrap_err().to_string().contains(error));
            continue;
        }
        let parsed = result.unwrap();
        let mut expected = vec![vec![0], vec![0, 2, 5, 9]];
        for count in c["counts"].as_array().unwrap() {
            expected.push(match count.as_u64().unwrap() {
                1 => vec![2],
                2 => vec![2, 9],
                _ => panic!(),
            });
        }
        assert_eq!(parsed.layer_sets, expected);
        assert_eq!(parsed.end_bit, c["end"].as_u64().unwrap() as usize);
        let temporal: Vec<u8> = c["temporal"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_u64().unwrap() as u8)
            .collect();
        assert_eq!(parsed.sublayers_minus1, temporal);
        let ids = [0u8, 2, 5, 9];
        let maxima: Vec<u8> = expected
            .iter()
            .map(|set| {
                set.iter()
                    .map(|id| temporal[ids.iter().position(|v| v == id).unwrap()])
                    .max()
                    .unwrap()
            })
            .collect();
        assert_eq!(parsed.set_sublayers_minus1, maxima);
        let inferred = c["inferred"].as_bool().unwrap();
        assert_eq!(parsed.max_tid_ref_plus1[0][2], if inferred { 7 } else { 0 });
        assert_eq!(parsed.max_tid_ref_plus1[1][2], 7);
        assert_eq!(parsed.max_tid_ref_plus1[1][3], if inferred { 7 } else { 3 });
        assert_eq!(parsed.max_tid_ref_plus1[0][3], 7);
        assert!(parsed.default_ref_layers_active);
        assert!(prefix.layer_sets(&bytes, &layers, sublayers, 0).is_err());
        assert!(
            prefix
                .layer_sets(&bytes[..bytes.len() - 1], &layers, sublayers, 1 << 20)
                .is_err()
        );
    }
}
