use fvid::codec::hevc_vps::{LayerSets, LayeredExtensionPrefix};
fn data(name: &str) -> Vec<u8> {
    std::fs::read(format!(
        "{}/tests/fixtures/playback-errors/{name}",
        env!("CARGO_MANIFEST_DIR")
    ))
    .unwrap()
}
#[test]
fn annex_f_general_vui_rates_signals_tiles_and_restrictions() {
    let cases: serde_json::Value = serde_json::from_slice(&data("hevc-layered-vui.json")).unwrap();
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
        let extension = LayeredExtensionPrefix::parse(&bytes, 0, &layers, 2, 1 << 20).unwrap();
        let result = extension.vui_prefix(&bytes, &layers, 1 << 20);
        if let Some(error) = c["error"].as_str() {
            assert!(result.unwrap_err().to_string().contains(error), "{name}");
            continue;
        }
        let p = result.unwrap().unwrap();
        let rich = c["rich"].as_bool().unwrap();
        assert_eq!(p.end_bit, c["end"].as_u64().unwrap() as usize);
        assert_eq!(p.picture_types_aligned, !rich);
        assert!(p.irap_aligned);
        assert_eq!(p.all_idr_aligned, rich);
        for (rows, e) in p.rates.iter().zip(c["rates"].as_array().unwrap()) {
            if e.is_null() {
                assert!(rows.is_none());
                continue;
            }
            for (row, e) in rows.as_ref().unwrap().iter().zip(e.as_array().unwrap()) {
                assert_eq!(
                    row.bit_rate.map(|(a, b)| [u64::from(a), u64::from(b)]),
                    e["bit_rate"]
                        .as_array()
                        .map(|v| [v[0].as_u64().unwrap(), v[1].as_u64().unwrap()])
                );
                assert_eq!(
                    row.picture_rate.map(|(a, b)| [u64::from(a), u64::from(b)]),
                    e["picture_rate"]
                        .as_array()
                        .map(|v| [v[0].as_u64().unwrap(), v[1].as_u64().unwrap()])
                );
            }
        }
        assert_eq!(
            p.video_signals.len(),
            c["signals"].as_array().unwrap().len()
        );
        for (s, e) in p.video_signals.iter().zip(c["signals"].as_array().unwrap()) {
            assert_eq!(u64::from(s.format), e["format"].as_u64().unwrap());
            assert_eq!(s.full_range, e["full_range"].as_bool().unwrap());
            assert_eq!(u64::from(s.primaries), e["primaries"].as_u64().unwrap());
            assert_eq!(u64::from(s.transfer), e["transfer"].as_u64().unwrap());
            assert_eq!(u64::from(s.matrix), e["matrix"].as_u64().unwrap());
        }
        assert_eq!(
            p.video_signal_indices,
            c["indices"]
                .as_array()
                .unwrap()
                .iter()
                .map(|v| v.as_u64().map(|v| v as u8))
                .collect::<Vec<_>>()
        );
        for (actual, key) in [
            (&p.tiles, "tiles"),
            (&p.loop_filter_not_across_tiles, "filters"),
            (&p.wpp, "wpp"),
        ] {
            assert_eq!(
                actual,
                &c[key]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|v| v.as_bool().unwrap())
                    .collect::<Vec<_>>()
            );
        }
        assert_eq!(
            p.aligned_tile_boundaries,
            c["alignment"]
                .as_array()
                .unwrap()
                .iter()
                .map(|v| v.as_u64().unwrap())
                .collect::<Vec<_>>()
        );
        assert_eq!(p.tiles_not_in_use, !rich);
        assert_eq!(p.wpp_not_in_use, !rich);
        assert_eq!(p.single_layer_non_irap, rich);
        assert_eq!(p.higher_layer_irap_skip, rich);
        let expected = c["restrictions"].as_array().unwrap();
        assert_eq!(
            p.restrictions
                .iter()
                .flatten()
                .filter(|v| v.is_some())
                .count(),
            expected.len()
        );
        for e in expected {
            let r = p.restrictions[e["dependent"].as_u64().unwrap() as usize]
                [e["reference"].as_u64().unwrap() as usize]
                .as_ref()
                .unwrap();
            assert_eq!(
                u64::from(r.spatial_offset_plus1),
                e["spatial"].as_u64().unwrap()
            );
            assert_eq!(r.ctu_based, e["ctu"].as_bool().unwrap());
            assert_eq!(
                u64::from(r.horizontal_offset_plus1),
                e["horizontal"].as_u64().unwrap()
            );
        }
        assert_eq!(p.bsp_hrd_present, c["bsp"].as_bool().unwrap());
        assert_eq!(p.base_parameter_compatibility, vec![None; 4]);
        assert!(extension.vui_prefix(&bytes, &layers, 0).is_err());
        if p.bsp_hrd_present {
            assert!(
                extension
                    .vui_prefix(&bytes[..bytes.len() - 1], &layers, 1 << 20)
                    .is_ok()
            );
        } else {
            assert!(
                extension
                    .vui_prefix(&bytes[..bytes.len() - 1], &layers, 1 << 20)
                    .is_err()
            );
        }
    }
}
