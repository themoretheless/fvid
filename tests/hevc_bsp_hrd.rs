use fvid::codec::hevc_vps::{LayerSets, LayeredExtensionPrefix};
fn data(name: &str) -> Vec<u8> {
    std::fs::read(format!(
        "{}/tests/fixtures/playback-errors/{name}",
        env!("CARGO_MANIFEST_DIR")
    ))
    .unwrap()
}
#[test]
fn annex_f_bsp_hrd_partitions_schedules_and_inheritance() {
    let cases: serde_json::Value = serde_json::from_slice(&data("hevc-bsp.json")).unwrap();
    for c in cases.as_array().unwrap() {
        let name = c["file"].as_str().unwrap();
        let bytes = data(name);
        let layers = LayerSets {
            base_internal: true,
            base_available: true,
            max_layers_minus1: 3,
            max_layer_id: 15,
            sets: vec![1, 32905],
        };
        let prefix = LayeredExtensionPrefix::parse(&bytes, 0, &layers, 2, 1 << 20).unwrap();
        let vui = prefix
            .vui_prefix(&bytes, &layers, 1 << 20)
            .unwrap()
            .unwrap();
        let result = vui.bsp_hrd(&bytes, &prefix, &layers, &[], 2, true, 1 << 20);
        if let Some(error) = c["error"].as_str() {
            assert!(result.unwrap_err().to_string().contains(error), "{name}");
            continue;
        }
        let p = result.unwrap_or_else(|e| panic!("{name}: {e}")).unwrap();
        assert_eq!(p.end_bit, c["end"].as_u64().unwrap() as usize);
        assert_eq!(p.additional_hrd.len(), c["hrds"].as_u64().unwrap() as usize);
        for (i, h) in p.additional_hrd.iter().enumerate() {
            assert_eq!(h.layers.len(), 3);
            assert_eq!(h.common, p.additional_hrd[0].common);
            for sub in &h.layers {
                assert_eq!(u64::from(sub.cpb_count), c["cpbs"].as_u64().unwrap());
                if c["no_lists"].as_bool().unwrap() {
                    assert!(sub.nal.is_empty());
                    assert!(sub.vcl.is_empty());
                } else {
                    for (k, cpb) in sub.nal.iter().enumerate() {
                        assert_eq!(cpb.bit_rate_value, 10 + i as u32 * 40 + k as u32 * 10);
                        assert_eq!(cpb.size_value, 128 - i as u32 * 32 - k as u32);
                    }
                }
            }
        }
        assert_eq!(p.sets.len(), 2);
        assert!(p.sets[0].is_none());
        if p.additional_hrd.is_empty() {
            assert!(p.sets[1].is_none());
        } else {
            let schemes = p.sets[1].as_ref().unwrap();
            assert_eq!(schemes.len(), c["schemes"].as_u64().unwrap() as usize);
            for (i, s) in schemes.iter().enumerate() {
                assert_eq!(
                    s.partitions,
                    if i == 0 {
                        vec![1, 8, 128, 32768]
                    } else {
                        vec![9, 32896]
                    }
                );
                assert_eq!(s.schedules.len(), 3);
                for rows in &s.schedules {
                    assert_eq!(rows.len(), c["schedules"].as_u64().unwrap() as usize);
                    for (j, row) in rows.iter().enumerate() {
                        assert_eq!(row.len(), s.partitions.len());
                        for e in row {
                            assert_eq!(
                                e.hrd_index,
                                if c["inherited"].as_bool().unwrap() && j > 0 {
                                    1
                                } else {
                                    0
                                }
                            );
                            assert_eq!(
                                e.cpb_index,
                                if c["inherited"].as_bool().unwrap() {
                                    0
                                } else {
                                    j as u8
                                }
                            );
                        }
                    }
                }
            }
        }
        assert_eq!(p.base_parameter_compatibility, vec![None; 4]);
        assert!(
            vui.bsp_hrd(&bytes, &prefix, &layers, &[], 2, true, 0)
                .is_err()
        );
        assert!(
            vui.bsp_hrd(&bytes, &prefix, &layers, &[], 2, false, 1 << 20)
                .unwrap_err()
                .to_string()
                .contains("requires VPS timing")
        );
        assert!(
            vui.bsp_hrd(
                &bytes[..bytes.len() - 1],
                &prefix,
                &layers,
                &[],
                2,
                true,
                1 << 20
            )
            .is_err()
        );
    }
}
