use fvid::codec::hevc_vps::{LayerDependencyPrefix, LayerSets};
fn data(name: &str) -> Vec<u8> {
    std::fs::read(format!(
        "{}/tests/fixtures/playback-errors/{name}",
        env!("CARGO_MANIFEST_DIR")
    ))
    .unwrap()
}
#[test]
fn annex_f_dpb_sublayer_inheritance_capacity_and_latency() {
    let cases: serde_json::Value = serde_json::from_slice(&data("hevc-dpb.json")).unwrap();
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
        let graph = LayerDependencyPrefix::parse(&bytes, 0, &layers, 2, 1 << 20).unwrap();
        let extension = graph.layer_sets(&bytes, &layers, 2, 1 << 20).unwrap();
        let output = extension
            .output_layers(&bytes, &graph, &layers, 2, 1 << 20)
            .unwrap();
        let representation = output
            .representation_formats(&bytes, &graph, &layers, 1 << 20)
            .unwrap();
        let result = representation.dpb_parameters(&bytes, &output, &extension, &layers, 1 << 20);
        if let Some(error) = c["error"].as_str() {
            assert!(result.unwrap_err().to_string().contains(error), "{name}");
            continue;
        }
        let parsed = result.unwrap_or_else(|e| panic!("{name}: {e}"));
        assert_eq!(parsed.end_bit, c["end"].as_u64().unwrap() as usize);
        assert!(parsed.sets[0].is_none());
        assert_eq!(parsed.sets.len(), c["sets"].as_array().unwrap().len() + 1);
        for (set, expected) in parsed
            .sets
            .iter()
            .skip(1)
            .zip(c["sets"].as_array().unwrap())
        {
            let rows = set.as_ref().unwrap();
            assert_eq!(rows.len(), 3);
            for (row, e) in rows.iter().zip(expected.as_array().unwrap()) {
                assert_eq!(row.explicitly_present, e["present"].as_bool().unwrap());
                assert_eq!(
                    row.decoded_pictures,
                    e["pictures"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .map(|v| v.as_u64().map(|n| n as u32))
                        .collect::<Vec<_>>()
                );
                assert_eq!(
                    u64::from(row.reorder_pictures),
                    e["reorder"].as_u64().unwrap()
                );
                assert_eq!(
                    u64::from(row.latency_increase_plus1),
                    e["latency"].as_u64().unwrap()
                );
                assert_eq!(row.max_latency_pictures(), e["max_latency"].as_u64());
            }
        }
        assert!(
            representation
                .dpb_parameters(&bytes, &output, &extension, &layers, 0)
                .is_err()
        );
        assert!(
            representation
                .dpb_parameters(
                    &bytes[..bytes.len() - 1],
                    &output,
                    &extension,
                    &layers,
                    1 << 20
                )
                .is_err()
        );
    }
}
