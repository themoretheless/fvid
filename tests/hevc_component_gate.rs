use fvid::{
    codec::{
        hevc_pps::{LayeredPps, LayeredPpsContext},
        hevc_slice::{LayeredSliceContext, SliceHeader},
        hevc_sps::{LayeredSps, LayeredSpsContext},
        hevc_vps::LayeredVps,
    },
    container::mp4::Limits,
    playback::video::mp4::LayeredMp4VideoReader,
};
fn root() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/hevc-wedge")
}
#[test]
fn component_flag_is_parsed_after_references_and_temporally_gated() {
    let read = |n: &str| std::fs::read(root().join(n)).unwrap();
    let vps = LayeredVps::parse(&read("skip-64.vps"), 1 << 20).unwrap();
    let base = LayeredSps::parse(
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
        &read("component-gate.sps"),
        &LayeredSpsContext {
            vps: &vps,
            layer_id: 3,
            output_layer_set: 1,
            active_sps: std::slice::from_ref(&base),
        },
        1 << 20,
    )
    .unwrap();
    let active = [base, depth.clone()];
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
    for (label, enabled) in [("off", false), ("on", true)] {
        let header = SliceHeader::parse_layered(
            &read(&format!("component-gate-{label}.nal")),
            &context,
            1 << 20,
        )
        .unwrap();
        let syntax = header.layered.as_ref().unwrap();
        assert_eq!(syntax.in_component_prediction, Some(enabled));
        assert_eq!(syntax.inter_layer_prediction, None);
        let mut nal = header.nal;
        nal.unit_type = 1;
        nal.temporal_id = 1;
        assert!(
            context
                .in_component_prediction_available(nal, syntax)
                .unwrap()
        );
        let mut limited = vps.clone();
        limited
            .extension
            .as_mut()
            .unwrap()
            .layer_sets
            .max_tid_ref_plus1[0][1] = 1;
        let restricted = LayeredSliceContext {
            vps: &limited,
            sps: &depth,
            pps: &pps,
        };
        assert!(
            !restricted
                .in_component_prediction_available(nal, syntax)
                .unwrap()
        );
        nal.temporal_id = 0;
        assert!(
            restricted
                .in_component_prediction_available(nal, syntax)
                .unwrap()
        );
        limited
            .extension
            .as_mut()
            .unwrap()
            .layer_sets
            .sublayers_minus1[0] = 0;
        let restricted = LayeredSliceContext {
            vps: &limited,
            sps: &depth,
            pps: &pps,
        };
        nal.temporal_id = 1;
        assert!(
            !restricted
                .in_component_prediction_available(nal, syntax)
                .unwrap()
        );
    }
}
#[test]
fn component_prediction_uses_completed_texture_and_skip_picture_plays() {
    for label in ["off", "on"] {
        let mut reader = LayeredMp4VideoReader::open(
            std::io::Cursor::new(
                std::fs::read(root().join(format!("in-component-{label}-playback.mp4"))).unwrap(),
            ),
            Limits::default(),
            1,
            0,
            64 << 20,
        )
        .unwrap();
        for _ in 0..2 {
            let mut layers = Vec::new();
            while let Some(frame) = reader.read_frame().unwrap() {
                layers.push(frame.decoded.layer_id);
                if frame.decoded.layer_id == 3 {
                    assert!(
                        frame.decoded.picture.planes[0]
                            .samples()
                            .iter()
                            .all(|&v| v == 128)
                    );
                }
            }
            assert_eq!(layers, [0, 3]);
            reader.rewind();
        }
    }
}
