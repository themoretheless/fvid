use fvid::codec::hevc_vps::{LayerDependencyPrefix, LayerSets};
fn data(name: &str) -> Vec<u8> {
    std::fs::read(format!(
        "{}/tests/fixtures/playback-errors/{name}",
        env!("CARGO_MANIFEST_DIR")
    ))
    .unwrap()
}
#[test]
fn annex_f_output_layers_and_reference_closure() {
    let cases: serde_json::Value = serde_json::from_slice(&data("hevc-output-sets.json")).unwrap();
    for c in cases.as_array().unwrap() {
        let name = c["file"].as_str().unwrap();
        let bytes = data(name);
        let layers = LayerSets {
            base_internal: true,
            base_available: true,
            max_layers_minus1: 3,
            max_layer_id: 15,
            sets: c["masks"]
                .as_array()
                .unwrap()
                .iter()
                .map(|v| v.as_u64().unwrap())
                .collect(),
        };
        let graph = LayerDependencyPrefix::parse(&bytes, 0, &layers, 0, 1 << 20).unwrap();
        let extension = graph.layer_sets(&bytes, &layers, 0, 1 << 20).unwrap();
        let result = extension.output_layers(&bytes, &graph, &layers, 0, 1 << 20);
        if let Some(error) = c["error"].as_str() {
            assert!(result.unwrap_err().to_string().contains(error), "{name}");
            continue;
        }
        let parsed = result.unwrap_or_else(|e| panic!("{name}: {e}"));
        assert_eq!(parsed.end_bit, c["end"].as_u64().unwrap() as usize);
        assert_eq!(parsed.default_output_idc, c["idc"].as_u64().unwrap() as u8);
        assert_eq!(
            u64::from(parsed.profile_count),
            c["profiles"].as_u64().unwrap()
        );
        if parsed.profile_count == 3 {
            assert_eq!(parsed.extra_profiles.len(), 1);
            assert_eq!(parsed.extra_profiles[0].0, 2);
            assert_eq!(parsed.extra_profiles[0].1.level, 120);
        } else {
            assert!(parsed.extra_profiles.is_empty());
        }
        assert_eq!(parsed.sets[0].output, 1);
        assert_eq!(parsed.sets[0].necessary, 1);
        assert_eq!(parsed.sets[0].profile_indices, vec![Some(1)]);
        assert_eq!(
            parsed.sets.len(),
            c["outputs"].as_array().unwrap().len() + 1
        );
        for (i, set) in parsed.sets.iter().enumerate().skip(1) {
            assert_eq!(
                set.layer_set_index as u64,
                c["indices"][i - 1].as_u64().unwrap()
            );
            assert_eq!(set.output, c["outputs"][i - 1].as_u64().unwrap());
            assert_eq!(set.necessary, c["required"][i - 1].as_u64().unwrap());
            let mask = layers.sets[set.layer_set_index];
            let expected_indices: Vec<Option<u8>> = [0u8, 3, 7, 15]
                .into_iter()
                .filter(|&id| mask & (1u64 << id) != 0)
                .map(|id| {
                    if set.necessary & (1u64 << id) != 0 {
                        Some(1)
                    } else {
                        None
                    }
                })
                .collect();
            assert_eq!(set.profile_indices, expected_indices);
            assert_eq!(
                set.alternative_output,
                c["alternatives"][i - 1].as_bool().unwrap()
            );
        }
        assert!(
            extension
                .output_layers(&bytes, &graph, &layers, 0, 0)
                .is_err()
        );
        assert!(
            extension
                .output_layers(&bytes[..bytes.len() - 1], &graph, &layers, 0, 1 << 20)
                .is_err()
        );
    }
}
