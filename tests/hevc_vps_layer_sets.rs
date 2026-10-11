use fvid::codec::hevc_vps::Vps;
fn data(name: &str) -> Vec<u8> {
    std::fs::read(format!(
        "{}/tests/fixtures/playback-errors/{name}",
        env!("CARGO_MANIFEST_DIR")
    ))
    .unwrap()
}
#[test]
fn layered_vps_metadata_preserves_topology_and_refuses_malformed_sets() {
    let m: serde_json::Value = serde_json::from_slice(&data("hevc-vps-layer-sets.json")).unwrap();
    for c in m["cases"].as_array().unwrap() {
        let name = c["file"].as_str().unwrap();
        let bytes = data(name);
        let result = Vps::parse_layered_metadata(&bytes, 1 << 20);
        if let Some(error) = c["error"].as_str() {
            let error_value = result.unwrap_err();
            assert!(
                matches!(error_value, fvid::Error::Invalid(_)),
                "{name}: {error_value}"
            );
            assert!(
                error_value.to_string().contains(error),
                "{name}: {error_value}"
            );
            continue;
        }
        let v = result.unwrap_or_else(|e| panic!("{name}: {e}"));
        assert_eq!(
            u64::from(v.layers.max_layers_minus1),
            c["maximum"].as_u64().unwrap()
        );
        assert_eq!(
            v.layers.effective_max_layers(),
            (c["maximum"].as_u64().unwrap().min(62) + 1) as u8
        );
        assert_eq!(
            u64::from(v.layers.max_layer_id),
            c["max_id"].as_u64().unwrap()
        );
        assert_eq!(v.layers.base_internal, c["internal"].as_bool().unwrap());
        assert_eq!(v.layers.base_available, c["available"].as_bool().unwrap());
        assert_eq!(
            v.layers.sets,
            c["sets"]
                .as_array()
                .unwrap()
                .iter()
                .map(|v| v.as_u64().unwrap())
                .collect::<Vec<_>>()
        );
        assert!(
            matches!(
                Vps::parse(&bytes, 1 << 20),
                Err(fvid::Error::Unsupported(_))
            ),
            "metadata acceptance must not silently admit multilayer playback"
        );
    }
}
#[test]
fn layer_hrd_inherits_only_common_fields_and_keeps_distinct_cpbs() {
    let v = Vps::parse_layered_metadata(&data("hevc-vps-layer-sets-inherited-hrd.vps"), 1 << 20)
        .unwrap();
    assert_eq!(v.layer_hrd.len(), 3);
    let h = &v.layer_hrd;
    assert_eq!(h[0].parameters.common, h[1].parameters.common);
    assert_ne!(h[1].parameters.common, h[2].parameters.common);
    assert!(!h[1].common_parameters_present);
    assert!(h[2].common_parameters_present);
    for (i, h) in h.iter().enumerate() {
        assert_eq!(h.layer_set_index, i as u16);
        assert_eq!(
            h.parameters.layers[0].elemental_duration,
            Some(i as u16 + 1)
        );
        if i < 2 {
            assert_eq!(h.parameters.layers[0].nal[0].bit_rate_value, i as u32 + 2);
        }
        assert_eq!(h.parameters.layers[0].vcl[0].bit_rate_value, i as u32 + 4);
    }
    assert_eq!(v.hrd.as_ref().unwrap(), &h[0].parameters);
    let external =
        Vps::parse_layered_metadata(&data("hevc-vps-layer-sets-external-base.vps"), 1 << 20)
            .unwrap();
    assert!(external.hrd.is_none());
    assert_eq!(external.layer_hrd[0].layer_set_index, 1);
}
#[test]
fn large_layer_set_metadata_obeys_the_owned_budget() {
    let b = data("hevc-vps-layer-sets-maximum-sets.vps");
    let err = Vps::parse_layered_metadata(&b, b.len() + 256).unwrap_err();
    assert!(
        err.to_string().contains("metadata exceeds memory budget"),
        "{err}"
    );
    assert_eq!(
        Vps::parse_layered_metadata(&b, 1 << 20)
            .unwrap()
            .layers
            .sets
            .len(),
        1024
    );
}
