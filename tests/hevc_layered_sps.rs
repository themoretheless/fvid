use fvid::codec::{
    hevc_sps::{LayeredSps, LayeredSpsContext, Sps},
    hevc_vps::LayeredVps,
};
fn data(name: &str) -> Vec<u8> {
    std::fs::read(format!(
        "{}/tests/fixtures/playback-errors/{name}",
        env!("CARGO_MANIFEST_DIR")
    ))
    .unwrap()
}
fn base(vps: &LayeredVps, enabled: bool) -> LayeredSps {
    LayeredSps::parse(
        &data(if enabled {
            "hevc-layered-sps-active-base.sps"
        } else {
            "hevc-layered-sps-disabled-base.sps"
        }),
        &LayeredSpsContext {
            vps,
            layer_id: 0,
            output_layer_set: 0,
            active_sps: &[],
        },
        1 << 20,
    )
    .unwrap()
}
#[test]
fn layered_sps_resolves_representation_profile_dpb_scaling_and_shared_syntax() {
    use fvid::{
        codec::config::HevcConfig,
        container::mp4::{Limits, Mp4Reader},
    };
    use std::io::Cursor;
    let manifest: serde_json::Value =
        serde_json::from_slice(&data("hevc-layered-sps.json")).unwrap();
    for c in manifest["cases"].as_array().unwrap() {
        let prefix = c["prefix"].as_str().unwrap();
        let vps = LayeredVps::parse(&data(&format!("{prefix}.vps")), 1 << 20)
            .unwrap_or_else(|e| panic!("{prefix} VPS: {e}"));
        let references = [base(&vps, true)];
        let context = LayeredSpsContext {
            vps: &vps,
            layer_id: 3,
            output_layer_set: 1,
            active_sps: &references,
        };
        let bytes = data(&format!("{prefix}.sps"));
        let parsed = LayeredSps::parse(&bytes, &context, 1 << 20);
        // The short MP4 is a configuration reproducer, not a claim that its
        // declared scalable profile or inter-layer pictures can be decoded.
        let movie = Mp4Reader::open(
            Cursor::new(data(&format!("{prefix}-synthetic.mp4"))),
            Limits::default(),
        )
        .unwrap();
        let config = HevcConfig::parse(&movie.tracks()[0].configuration).unwrap();
        let configured = config
            .arrays
            .iter()
            .find(|a| a.nal_type == 33)
            .unwrap()
            .units
            .last()
            .unwrap();
        assert_eq!(*configured, bytes);
        let configured_vps = config
            .arrays
            .iter()
            .find(|a| a.nal_type == 32)
            .unwrap()
            .units[0];
        assert_eq!(configured_vps, data(&format!("{prefix}.vps")));
        let movie_result = LayeredSps::parse(configured, &context, 1 << 20);
        if let Some(error) = c["error"].as_str() {
            for e in [parsed.unwrap_err(), movie_result.unwrap_err()] {
                assert!(matches!(e, fvid::Error::Invalid(_)), "{prefix}: {e}");
                assert!(e.to_string().contains(error), "{prefix}: {e}");
            }
            continue;
        }
        let p = parsed.unwrap_or_else(|e| panic!("{prefix}: {e}"));
        assert_eq!(movie_result.unwrap(), p);
        assert_eq!(p.layer_id, 3);
        assert_eq!(
            p.nal_layer_id,
            if c["shared"].as_bool().unwrap() { 0 } else { 3 }
        );
        assert_eq!(p.multilayer_syntax, c["multilayer"].as_bool().unwrap());
        assert_eq!(
            p.representation_index.map(u64::from),
            c["representation"].as_u64()
        );
        let f = &c["format"];
        assert_eq!(
            u64::from(p.sps.chroma_format),
            f["chroma"].as_u64().unwrap()
        );
        assert_eq!(
            p.sps.separate_colour_plane,
            f["separate"].as_bool().unwrap()
        );
        assert_eq!(
            p.sps.dimensions.map(u64::from).as_slice(),
            f["size"]
                .as_array()
                .unwrap()
                .iter()
                .map(|v| v.as_u64().unwrap())
                .collect::<Vec<_>>()
        );
        assert_eq!(
            p.sps.depth.map(u64::from).as_slice(),
            f["depth"]
                .as_array()
                .unwrap()
                .iter()
                .map(|v| v.as_u64().unwrap())
                .collect::<Vec<_>>()
        );
        let separate = p.sps.separate_colour_plane;
        let sx = if !separate && matches!(p.sps.chroma_format, 1 | 2) {
            2
        } else {
            1
        };
        let sy = if !separate && p.sps.chroma_format == 1 {
            2
        } else {
            1
        };
        let expected_crop: Vec<u32> = f["crop"]
            .as_array()
            .unwrap()
            .iter()
            .enumerate()
            .map(|(i, v)| v.as_u64().unwrap() as u32 * if i < 2 { sx } else { sy })
            .collect();
        assert_eq!(p.sps.crop.as_slice(), expected_crop);
        assert_eq!(
            p.effective_profile.level,
            if c["independent"].as_bool().unwrap() {
                120
            } else {
                90
            }
        );
        if c["shared"].as_bool().unwrap() {
            assert_eq!(p.sps.profile.level, 120);
        }
        assert_eq!(p.sps.poc_bits, 8);
        assert_eq!(p.sps.long_term, vec![(1, true), (7, false)]);
        assert!(p.sps.temporal_mvp && p.sps.strong_intra_smoothing);
        assert!(p.sps.inter_view_mv_vert_constraint);
        for (row, expected) in p.output_ordering.iter().zip(c["rows"].as_array().unwrap()) {
            assert_eq!(
                u64::from(row.max_decoded_pictures),
                expected[0].as_u64().unwrap()
            );
            assert_eq!(
                u64::from(row.max_reorder_pictures),
                expected[1].as_u64().unwrap()
            );
            assert_eq!(
                u64::from(row.max_latency_increase_plus1),
                expected[2].as_u64().unwrap()
            );
        }
        assert_eq!(p.output_ordering.len(), c["rows"].as_array().unwrap().len());
        assert_eq!(p.max_sub_layers_minus1, 2);
        if p.multilayer_syntax {
            assert_eq!(p.sps.ordering, p.output_ordering);
        }
        assert_eq!(p.sps.pcm.is_some(), c["pcm"].as_bool().unwrap());
        assert_eq!(p.sps.scc_extension, c["scc"].as_bool().unwrap());
        if p.sps.scc_extension {
            assert_eq!(
                p.sps.palette.as_ref().unwrap().initial,
                vec![[3, 4, 5], [9, 10, 11]]
            );
        }
        let rps = c["rps"].as_u64().unwrap() as usize;
        assert_eq!(p.sps.short_term.iter().map(Vec::len).sum::<usize>(), rps);
        if rps != 0 {
            assert_eq!(
                p.sps.short_term[0]
                    .iter()
                    .map(|r| r.delta_poc)
                    .collect::<Vec<_>>(),
                (1..=rps).map(|n| -(n as i32)).collect::<Vec<_>>()
            );
        }
        match c["scaling"].as_str().unwrap() {
            "infer" => {
                assert_eq!(p.scaling_list_reference, Some(0));
                assert_eq!(p.sps.scaling_lists, references[0].sps.scaling_lists);
            }
            "explicit" => {
                assert_eq!(
                    &p.sps.scaling_lists.matrix(0, 0).unwrap().coefficients[..16],
                    &[17; 16]
                );
            }
            "default" => {
                assert_eq!(
                    p.sps.scaling_lists,
                    fvid::codec::hevc_scaling::ScalingLists::default()
                );
            }
            "disabled" => {
                assert!(!p.sps.scaling_lists_enabled);
            }
            _ => unreachable!(),
        }
        // No opaque future tail is present except SCC, where shorter opaque
        // data may be valid. Mandatory syntax truncations must all fail.
        if !p.sps.scc_extension {
            for end in 0..bytes.len() {
                assert!(
                    LayeredSps::parse(&bytes[..end], &context, 1 << 20).is_err(),
                    "{prefix}: truncated {end}"
                );
            }
        }
        if p.nal_layer_id != 0 {
            assert_eq!(
                Sps::parse(&bytes, 1 << 20).unwrap_err().to_string(),
                "HEVC multilayer decoding is not implemented"
            );
        }
    }
}
#[test]
fn scaling_inference_requires_an_active_enabled_unchained_reference() {
    let vps = LayeredVps::parse(&data("hevc-layered-sps-infer-scaling.vps"), 1 << 20).unwrap();
    let bytes = data("hevc-layered-sps-infer-scaling.sps");
    let context = LayeredSpsContext {
        vps: &vps,
        layer_id: 3,
        output_layer_set: 1,
        active_sps: &[],
    };
    assert!(
        LayeredSps::parse(&bytes, &context, 1 << 20)
            .unwrap_err()
            .to_string()
            .contains("not active")
    );
    for enabled in [false, true] {
        let mut refs = vec![base(&vps, enabled)];
        if enabled {
            refs[0].scaling_list_reference = Some(7);
        }
        let context = LayeredSpsContext {
            active_sps: &refs,
            ..context
        };
        assert!(
            LayeredSps::parse(&bytes, &context, 1 << 20)
                .unwrap_err()
                .to_string()
                .contains("reference parameters")
        );
    }
    let refs = vec![base(&vps, true); 2];
    let context = LayeredSpsContext {
        active_sps: &refs,
        ..context
    };
    assert!(LayeredSps::parse(&bytes, &context, 1 << 20).is_err());
}

#[test]
fn ordinary_main_and_main10_context_matches_the_existing_sps_reader() {
    use fvid::{
        codec::config::HevcConfig,
        container::mp4::{Limits, Mp4Reader},
    };
    use std::io::Cursor;
    for seed in ["main-ipb", "main10-ipb"] {
        let bytes = std::fs::read(format!(
            "{}/tests/fixtures/hevc/{seed}.mp4",
            env!("CARGO_MANIFEST_DIR")
        ))
        .unwrap();
        let movie = Mp4Reader::open(Cursor::new(bytes), Limits::default()).unwrap();
        let config = HevcConfig::parse(&movie.tracks()[0].configuration).unwrap();
        let vps = LayeredVps::parse(
            config
                .arrays
                .iter()
                .find(|a| a.nal_type == 32)
                .unwrap()
                .units[0],
            1 << 20,
        )
        .unwrap();
        let nal = config
            .arrays
            .iter()
            .find(|a| a.nal_type == 33)
            .unwrap()
            .units[0];
        let plain = Sps::parse(nal, 1 << 20).unwrap();
        let resolved = LayeredSps::parse(
            nal,
            &LayeredSpsContext {
                vps: &vps,
                layer_id: 0,
                output_layer_set: 0,
                active_sps: &[],
            },
            1 << 20,
        )
        .unwrap();
        assert_eq!(resolved.sps, plain);
        assert_eq!(resolved.effective_profile, plain.profile);
        assert_eq!(resolved.output_ordering, plain.ordering);
        assert_eq!(resolved.nal_layer_id, 0);
        assert!(!resolved.multilayer_syntax);
        assert!(
            resolved.representation_index.is_none() && resolved.scaling_list_reference.is_none()
        );
    }
}

#[test]
fn ols_output_limits_keep_temporal_layers_above_the_base_sps_bound() {
    use fvid::{
        codec::config::HevcConfig,
        container::mp4::{Limits, Mp4Reader},
    };
    use std::io::Cursor;
    let prefix = "hevc-layered-sps-wide-reorder";
    let movie = Mp4Reader::open(
        Cursor::new(data(&format!("{prefix}-synthetic.mp4"))),
        Limits::default(),
    )
    .unwrap();
    let config = HevcConfig::parse(&movie.tracks()[0].configuration).unwrap();
    let vps = LayeredVps::parse(
        config
            .arrays
            .iter()
            .find(|a| a.nal_type == 32)
            .unwrap()
            .units[0],
        1 << 20,
    )
    .unwrap();
    let nal = config
        .arrays
        .iter()
        .find(|a| a.nal_type == 33)
        .unwrap()
        .units[0];
    let p = LayeredSps::parse(
        nal,
        &LayeredSpsContext {
            vps: &vps,
            layer_id: 0,
            output_layer_set: 1,
            active_sps: &[],
        },
        1 << 20,
    )
    .unwrap();
    assert_eq!(p.max_sub_layers_minus1, 0);
    assert_eq!(p.sps.ordering.len(), 1);
    // OLS1's output scheduler still includes layer3's higher temporal pictures.
    assert_eq!(p.output_ordering.len(), 3);
    assert_eq!(
        p.output_ordering
            .iter()
            .map(|r| r.max_reorder_pictures)
            .collect::<Vec<_>>(),
        vec![300, 301, 302]
    );
    assert_eq!(
        p.output_ordering
            .iter()
            .map(|r| r.max_decoded_pictures)
            .collect::<Vec<_>>(),
        vec![4, 5, 6]
    );
}
