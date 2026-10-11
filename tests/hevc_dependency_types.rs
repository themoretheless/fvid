use fvid::codec::hevc_vps::{LayerSets, LayeredExtensionPrefix};
fn data(name: &str) -> Vec<u8> {
    std::fs::read(format!(
        "{}/tests/fixtures/playback-errors/{name}",
        env!("CARGO_MANIFEST_DIR")
    ))
    .unwrap()
}
#[test]
fn connected_annex_f_dependency_types_and_tail_bounds() {
    let cases: serde_json::Value =
        serde_json::from_slice(&data("hevc-dependency-types.json")).unwrap();
    for c in cases.as_array().unwrap() {
        let name = c["file"].as_str().unwrap();
        let bytes = data(name);
        let layers = LayerSets {
            base_internal: c["internal"].as_bool().unwrap(),
            base_available: true,
            max_layers_minus1: 3,
            max_layer_id: 15,
            sets: vec![1, 32905],
        };
        let result = LayeredExtensionPrefix::parse(&bytes, 0, &layers, 2, 1 << 20);
        if let Some(error) = c["error"].as_str() {
            assert!(result.unwrap_err().to_string().contains(error), "{name}");
            continue;
        }
        let parsed = result.unwrap_or_else(|e| panic!("{name}: {e}"));
        let p = &parsed.dependencies;
        assert_eq!(p.end_bit, c["end"].as_u64().unwrap() as usize);
        assert_eq!(u64::from(p.coded_width), c["width"].as_u64().unwrap());
        assert_eq!(p.common_type.map(u64::from), c["common"].as_u64());
        for i in 0..4 {
            for j in 0..4 {
                let expected = if i > 0 && j + 1 == i {
                    c["values"][i - 1].as_u64().map(|v| v as u32)
                } else {
                    None
                };
                assert_eq!(p.types[i][j], expected);
            }
        }
        assert_eq!(
            p.sample_prediction,
            c["sample"]
                .as_array()
                .unwrap()
                .iter()
                .map(|v| v.as_u64().unwrap())
                .collect::<Vec<_>>()
        );
        assert_eq!(
            p.motion_prediction,
            c["motion"]
                .as_array()
                .unwrap()
                .iter()
                .map(|v| v.as_u64().unwrap())
                .collect::<Vec<_>>()
        );
        assert_eq!(
            u64::from(p.ignored_extension_bytes),
            c["ignored"].as_u64().unwrap()
        );
        assert_eq!(p.vui_present, c["vui"].as_bool().unwrap());
        assert_eq!(parsed.graph.layer_ids, vec![0, 3, 7, 15]);
        assert_eq!(parsed.dpb.sets[1].as_ref().unwrap().len(), 3);
        assert!(LayeredExtensionPrefix::parse(&bytes, 0, &layers, 2, 0).is_err());
        // For VUI, truncating only its sentinel remains valid prefix parsing.
        if p.vui_present {
            assert!(
                LayeredExtensionPrefix::parse(&bytes[..bytes.len() - 1], 0, &layers, 2, 1 << 20)
                    .is_ok()
            );
        } else {
            assert!(
                LayeredExtensionPrefix::parse(&bytes[..bytes.len() - 1], 0, &layers, 2, 1 << 20)
                    .is_err()
            );
        }
    }
}
