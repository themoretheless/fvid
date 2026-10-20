use fvid::codec::{
    hevc_picture::decode_layered,
    hevc_pps::{LayeredPps, LayeredPpsContext},
    hevc_slice::{LayeredSliceContext, SliceHeader},
    hevc_sps::{LayeredSps, LayeredSpsContext},
    hevc_vps::LayeredVps,
};
#[test]
fn parsed_skip_depth_idr_reconstructs_all_modes() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/hevc-wedge");
    let read = |n: &str| std::fs::read(root.join(n)).unwrap();
    for size in [64, 128] {
        let vps = LayeredVps::parse(&read(&format!("skip-{size}.vps")), 1 << 20).unwrap();
        let texture = LayeredSps::parse(
            &read("skip-texture.sps"),
            &LayeredSpsContext {
                vps: &vps,
                layer_id: 0,
                output_layer_set: 0,
                active_sps: &[],
            },
            1 << 20,
        )
        .unwrap();
        let depth = LayeredSps::parse(
            &read("skip-depth.sps"),
            &LayeredSpsContext {
                vps: &vps,
                layer_id: 3,
                output_layer_set: 1,
                active_sps: std::slice::from_ref(&texture),
            },
            1 << 20,
        )
        .unwrap();
        assert!(depth.sps.depth_layer);
        assert!(depth.sps.three_d.as_ref().unwrap().skip_intra);
        let active = [texture, depth.clone()];
        let pps = LayeredPps::parse(
            &read("skip-depth.pps"),
            &LayeredPpsContext {
                vps: &vps,
                sps: &depth,
                active_sps: &active,
                active_pps: &[],
            },
            1 << 20,
        )
        .unwrap();
        let context = LayeredSliceContext {
            vps: &vps,
            sps: &depth,
            pps: &pps,
        };
        for mode in 0..4 {
            let slice = SliceHeader::parse_layered(
                &read(&format!("skip-{size}-mode{mode}.nal")),
                &context,
                1 << 20,
            )
            .unwrap();
            assert!(
                slice
                    .layered
                    .as_ref()
                    .unwrap()
                    .inter_layer_prediction
                    .is_none()
            );
            let picture =
                decode_layered(&context, &[slice], 0, &[[Vec::new(), Vec::new()]], 32 << 20)
                    .unwrap();
            assert_eq!(picture.dimensions, [size, size]);
            assert!(picture.planes[0].complete());
            assert!(picture.planes[0].samples().iter().all(|&v| v == 128));
        }
    }
}

#[test]
fn parsed_pcm_gradient_seeds_all_skip_modes_without_flat_fallback() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/hevc-wedge");
    let read = |n: &str| std::fs::read(root.join(n)).unwrap();
    let vps = LayeredVps::parse(&read("mixed-skip.vps"), 1 << 20).unwrap();
    let texture = LayeredSps::parse(
        &read("skip-texture.sps"),
        &LayeredSpsContext {
            vps: &vps,
            layer_id: 0,
            output_layer_set: 0,
            active_sps: &[],
        },
        1 << 20,
    )
    .unwrap();
    let depth = LayeredSps::parse(
        &read("mixed-skip.sps"),
        &LayeredSpsContext {
            vps: &vps,
            layer_id: 3,
            output_layer_set: 1,
            active_sps: std::slice::from_ref(&texture),
        },
        1 << 20,
    )
    .unwrap();
    assert_eq!(depth.sps.depth, [10; 2]);
    let active = [texture, depth.clone()];
    let pps = LayeredPps::parse(
        &read("mixed-skip.pps"),
        &LayeredPpsContext {
            vps: &vps,
            sps: &depth,
            active_sps: &active,
            active_pps: &[],
        },
        1 << 20,
    )
    .unwrap();
    let context = LayeredSliceContext {
        vps: &vps,
        sps: &depth,
        pps: &pps,
    };
    for suffix in ["", "-high"] {
        for mode in 0..4 {
            let slice = SliceHeader::parse_layered(
                &read(&format!("mixed-skip-mode{mode}{suffix}.nal")),
                &context,
                1 << 20,
            )
            .unwrap();
            assert!(
                slice
                    .layered
                    .as_ref()
                    .unwrap()
                    .inter_layer_prediction
                    .is_none()
            );
            let picture =
                decode_layered(&context, &[slice], 0, &[[Vec::new(), Vec::new()]], 32 << 20)
                    .unwrap();
            let reference = read(&format!("mixed-skip-mode{mode}{suffix}.y16"));
            let expected: Vec<_> = reference
                .chunks_exact(2)
                .map(|b| u16::from_le_bytes([b[0], b[1]]))
                .collect();
            assert_eq!(picture.planes[0].samples(), expected, "mode={mode}");
            assert!(picture.planes[0].complete());
        }
    }
}

#[test]
fn parsed_wedge_dc_only_idr_reconstructs_distinct_segments() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/hevc-wedge");
    let read = |n: &str| std::fs::read(root.join(n)).unwrap();
    let vps = LayeredVps::parse(&read("wedge-picture.vps"), 1 << 20).unwrap();
    let texture = LayeredSps::parse(
        &read("skip-texture.sps"),
        &LayeredSpsContext {
            vps: &vps,
            layer_id: 0,
            output_layer_set: 0,
            active_sps: &[],
        },
        1 << 20,
    )
    .unwrap();
    let depth = LayeredSps::parse(
        &read("wedge-picture.sps"),
        &LayeredSpsContext {
            vps: &vps,
            layer_id: 3,
            output_layer_set: 1,
            active_sps: std::slice::from_ref(&texture),
        },
        1 << 20,
    )
    .unwrap();
    assert_eq!(depth.sps.depth, [8; 2]);
    let active = [texture, depth.clone()];
    for variant in ["wedge-picture", "wedge-dlt"] {
        let pps = LayeredPps::parse(
            &read(&format!("{variant}.pps")),
            &LayeredPpsContext {
                vps: &vps,
                sps: &depth,
                active_sps: &active,
                active_pps: &[],
            },
            1 << 20,
        )
        .unwrap();
        assert_eq!(pps.pps.depth_lookup_active, variant == "wedge-dlt");
        let context = LayeredSliceContext {
            vps: &vps,
            sps: &depth,
            pps: &pps,
        };
        for kind in ["wedge", "present", "absent"] {
            let nal = if kind == "wedge" {
                "wedge-picture.nal".to_string()
            } else {
                format!("ordinary-dc-{kind}.nal")
            };
            let slice = SliceHeader::parse_layered(&read(&nal), &context, 1 << 20).unwrap();
            assert!(
                slice
                    .layered
                    .as_ref()
                    .unwrap()
                    .inter_layer_prediction
                    .is_none()
            );
            let picture =
                decode_layered(&context, &[slice], 0, &[[Vec::new(), Vec::new()]], 32 << 20)
                    .unwrap();
            let reference = read(&if kind == "wedge" {
                format!("{variant}.y16")
            } else {
                format!(
                    "ordinary-dc-{kind}{}.y16",
                    if variant == "wedge-dlt" { "-dlt" } else { "" }
                )
            });
            let expected: Vec<_> = reference
                .chunks_exact(2)
                .map(|b| u16::from_le_bytes([b[0], b[1]]))
                .collect();
            assert_eq!(picture.planes[0].samples(), expected);
            assert!(picture.planes[0].complete());
        }
    }
}
