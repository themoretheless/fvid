use fvid::codec::hevc_vps::{LayeredVps, Vps};
fn data(name: &str) -> Vec<u8> {
    std::fs::read(format!(
        "{}/tests/fixtures/playback-errors/{name}",
        env!("CARGO_MANIFEST_DIR")
    ))
    .unwrap()
}
#[test]
fn complete_layered_vps_metadata_nal_and_rbsp_boundaries() {
    let cases: serde_json::Value = serde_json::from_slice(&data("hevc-layered-vps.json")).unwrap();
    for c in cases.as_array().unwrap() {
        let name = c["file"].as_str().unwrap();
        let bytes = data(name);
        let result = LayeredVps::parse(&bytes, 1 << 20);
        if let Some(error) = c["error"].as_str() {
            let e = result.unwrap_err();
            assert!(matches!(e, fvid::Error::Invalid(_)), "{name}: {e}");
            assert!(e.to_string().contains(error), "{name}: {e}");
            continue;
        }
        let p = result.unwrap_or_else(|e| panic!("{name}: {e}"));
        let extension = p.extension.as_ref().unwrap();
        assert_eq!(p.vps.layers.base_internal, c["internal"].as_bool().unwrap());
        assert_eq!(p.vps.layers.sets, vec![1, 32905]);
        assert_eq!(p.vps.ordering.len(), 3);
        assert_eq!(extension.graph.layer_ids, vec![0, 3, 7, 15]);
        if p.vps.layers.base_internal {
            let additional = extension.graph.base_additional_profile.as_ref().unwrap();
            assert!(additional.profile.is_none());
            assert_eq!(
                additional.level,
                if c["ptl_inheritance"].as_bool().unwrap() {
                    90
                } else {
                    120
                }
            );
        } else {
            assert!(extension.graph.base_additional_profile.is_none());
        }
        assert_eq!(extension.dpb.sets[1].as_ref().unwrap().len(), 3);
        assert!(p.vps.poc_lsb_aligned().unwrap());
        assert_eq!(p.future_extension, c["future"].as_bool().unwrap());
        assert_eq!(p.vui.is_some(), c["vui"].as_bool().unwrap());
        assert_eq!(p.bsp.is_some(), c["bsp"].as_bool().unwrap());
        assert_eq!(p.vps.timing.is_some(), p.bsp.is_some());
        if let Some(bsp) = p.bsp.as_ref() {
            assert_eq!(bsp.additional_hrd.len(), 1);
            assert_eq!(
                bsp.sets[1].as_ref().unwrap().len(),
                c["schemes"].as_u64().unwrap() as usize
            );
            if c["base_hrd"].as_bool().unwrap() {
                assert_eq!(p.vps.layer_hrd.len(), 1);
                assert_eq!(
                    bsp.additional_hrd[0].common,
                    p.vps.layer_hrd[0].parameters.common
                );
                for rows in &bsp.sets[1].as_ref().unwrap()[0].schedules {
                    for (i, row) in rows.iter().enumerate() {
                        for entry in row {
                            assert_eq!(entry.hrd_index, i as u16);
                            assert_eq!(entry.cpb_index, 0);
                        }
                    }
                }
            }
        }
        assert!(
            matches!(
                Vps::parse(&bytes, 1 << 20),
                Err(fvid::Error::Unsupported(_))
            ),
            "metadata must not admit multilayer playback"
        );
        assert!(LayeredVps::parse(&bytes, bytes.len() + 256).is_err());
        for end in 0..bytes.len() {
            let shortened = LayeredVps::parse(&bytes[..end], 1 << 20);
            if p.future_extension {
                // An opaque future tail has no coded length: a shorter tail
                // can itself end with valid trailing bits. Preserve semantics.
                if let Ok(short) = shortened {
                    assert!(short.future_extension);
                    assert_eq!(short.vps, p.vps);
                    assert_eq!(short.extension, p.extension);
                }
            } else {
                assert!(shortened.is_err(), "{name}: byte {end}");
            }
        }
    }
}

#[test]
fn selected_layer_profiles_inherit_general_and_sublayer_fields_from_predecessors() {
    use fvid::codec::hevc_profile::{ProfileTierLevel, SubLayer};
    let mut p = LayeredVps::parse(&data("hevc-layered-vps-ptl-inheritance.vps"), 1 << 20).unwrap();
    let effective = p.profile_for_layer(1, 15).unwrap();
    assert_eq!(effective.profile, p.vps.profile.profile);
    assert_eq!(effective.level, 90);
    assert_eq!(effective.sub_layers[0].profile.unwrap().idc, 2);
    assert_eq!(effective.sub_layers[0].level, Some(60));
    assert_eq!(effective.sub_layers[1].profile.unwrap().idc, 3);
    assert_eq!(effective.sub_layers[1].level, Some(30));
    // OLS0's inferred PTL index is 1, not the base PTL index zero.
    assert_eq!(p.profile_for_layer(0, 0).unwrap(), effective);
    for layer in [0, 3, 7] {
        assert_eq!(p.profile_for_layer(1, layer).unwrap(), effective);
    }
    assert!(p.profile_for_layer(0, 15).is_err());
    assert!(p.profile_for_layer(9, 0).is_err());
    assert!(p.profile_for_layer(1, 63).is_err());

    // A later absent profile inherits PTL1, including its replaced sublayer
    // fields, rather than restarting inheritance from the base VPS.
    let ext = p.extension.as_mut().unwrap();
    ext.output.profile_count = 3;
    ext.output.extra_profiles.push((
        2,
        ProfileTierLevel {
            profile: None,
            level: 93,
            sub_layers: vec![SubLayer::default(); 2],
        },
    ));
    ext.output.sets[1].profile_indices[3] = Some(2);
    let mut expected = effective.clone();
    expected.level = 93;
    assert_eq!(p.profile_for_layer(1, 15).unwrap(), expected);
    assert_eq!(p.profile_for_layer(1, 7).unwrap(), effective);
    let mut replacement = expected.profile.unwrap();
    replacement.idc = 4;
    replacement.compatibility = 0;
    let ext = p.extension.as_mut().unwrap();
    ext.output.extra_profiles[0].1.profile = Some(replacement);
    ext.output.extra_profiles[0].1.sub_layers[0].level = Some(63);
    expected.profile = Some(replacement);
    expected.sub_layers[0].level = Some(63);
    assert_eq!(p.profile_for_layer(1, 15).unwrap(), expected);
    let ext = p.extension.as_mut().unwrap();
    ext.output.sets[1].profile_indices[2] = None;
    assert!(p.profile_for_layer(1, 7).is_err());
    p.extension.as_mut().unwrap().graph.base_additional_profile = None;
    assert!(
        p.profile_for_layer(1, 15)
            .unwrap_err()
            .to_string()
            .contains("predecessor")
    );
}

#[test]
fn synthetic_mp4_reproduces_vps_profile_retention_and_illegal_profile_presence() {
    use fvid::{
        codec::config::HevcConfig,
        container::mp4::{Limits, Mp4Reader},
    };
    use std::io::Cursor;
    for seed in ["main-ipb", "main10-ipb"] {
        for kind in ["ptl-inheritance", "bad-ptl-profile"] {
            let movie = Mp4Reader::open(
                Cursor::new(data(&format!(
                    "hevc-layered-vps-{seed}-{kind}-synthetic.mp4"
                ))),
                Limits::default(),
            )
            .unwrap();
            let config = HevcConfig::parse(&movie.tracks()[0].configuration).unwrap();
            let nal = config
                .arrays
                .iter()
                .find(|a| a.nal_type == 32)
                .unwrap()
                .units[0];
            assert_eq!(nal, data(&format!("hevc-layered-vps-{kind}.vps")));
            let result = LayeredVps::parse(nal, 1 << 20);
            if kind == "bad-ptl-profile" {
                let e = result.unwrap_err();
                assert!(matches!(e, fvid::Error::Invalid(_)));
                assert_eq!(
                    e.to_string(),
                    "absent HEVC general profile has a sublayer profile"
                );
            } else {
                let p = result.unwrap();
                let effective = p.profile_for_layer(1, 15).unwrap();
                assert_eq!(effective.level, 90);
                assert_eq!(effective.sub_layers[0].profile.unwrap().idc, 2);
                assert_eq!(effective.sub_layers[0].level, Some(60));
                assert_eq!(effective.sub_layers[1].profile.unwrap().idc, 3);
                assert_eq!(effective.sub_layers[1].level, Some(30));
            }
        }
    }
}

#[test]
fn external_base_profile_is_ignored_during_profile_inheritance() {
    let p = LayeredVps::parse(&data("hevc-layered-vps-external.vps"), 1 << 20).unwrap();
    assert!(p.profile_for_layer(0, 0).is_err());
    let resolved = p.profile_for_layer(1, 15).unwrap();
    assert_eq!(
        resolved.profile.unwrap(),
        fvid::codec::hevc_profile::Profile {
            space: 0,
            tier: false,
            idc: 0,
            compatibility: 0,
            constraints: 0,
        }
    );
    assert_eq!(resolved.level, 120);
    assert_eq!(
        resolved.sub_layers,
        vec![fvid::codec::hevc_profile::SubLayer::default(); 2]
    );
}
