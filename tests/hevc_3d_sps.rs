use fvid::{
    codec::{bits::BitReader, config::HevcConfig, hevc_3d_syntax::SpsExtension, hevc_sps::Sps},
    container::mp4::{Limits, Mp4Reader},
    playback_mp4::Mp4VideoReader,
};
use std::{
    io::Cursor,
    path::{Path, PathBuf},
};
fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/playback-errors/hevc-3d-sps")
}
fn manifest() -> serde_json::Value {
    serde_json::from_slice(&std::fs::read(root().join("manifest.json")).unwrap()).unwrap()
}
fn parse(bytes: &[u8]) -> fvid::Result<Sps> {
    let reader = Mp4Reader::open(Cursor::new(bytes), Limits::default())?;
    let config = HevcConfig::parse(&reader.tracks()[0].configuration)?;
    Sps::parse(
        &config
            .arrays
            .iter()
            .find(|a| a.nal_type == 33)
            .unwrap()
            .units[0],
        16 << 20,
    )
}
#[test]
fn complete_three_d_sps_syntax_preserves_all_fields_and_rolls_back_truncations() {
    for c in manifest()["syntax"].as_array().unwrap() {
        let data =
            std::fs::read(root().join(format!("{}.bin", c["name"].as_str().unwrap()))).unwrap();
        let end = c["bits"].as_u64().unwrap() as usize;
        let cb = [
            c["cb"][0].as_u64().unwrap() as u8,
            c["cb"][1].as_u64().unwrap() as u8,
        ];
        let mut reader = BitReader::new(&data);
        let s = SpsExtension::read(&mut reader, end, cb).unwrap();
        let flags = [
            s.inter_view_disparity[0],
            s.inter_view_mv_scaling[0],
            s.inter_view_residual_prediction,
            s.depth_reference,
            s.view_synthesis_prediction,
            s.depth_based_partition,
            s.inter_view_disparity[1],
            s.inter_view_mv_scaling[1],
            s.texture_motion,
            s.intra_contour,
            s.intra_dc_wedge,
            s.coding_tree_partition,
            s.inter_dc_only,
            s.skip_intra,
        ];
        assert_eq!(
            flags,
            c["flags"]
                .as_array()
                .unwrap()
                .iter()
                .map(|v| v.as_bool().unwrap())
                .collect::<Vec<_>>()
                .as_slice()
        );
        assert_eq!(
            [s.disparity_sub_block_log2, s.texture_sub_block_log2],
            [
                c["sub"][0].as_u64().unwrap() as u8,
                c["sub"][1].as_u64().unwrap() as u8
            ]
        );
        assert_eq!(s.active(), flags.iter().any(|v| *v));
        assert_eq!(s.active_for_depth(false), flags[..6].iter().any(|v| *v));
        assert_eq!(s.active_for_depth(true), flags[6..].iter().any(|v| *v));
        assert_eq!(reader.position(), end);
        for cut in 0..end {
            let mut reader = BitReader::new(&data);
            assert!(SpsExtension::read(&mut reader, cut, cb).is_err());
            assert_eq!(reader.position(), 0);
        }
    }
}
#[test]
fn inactive_sps_three_d_extension_preserves_i_p_b_pixels_and_clocks() {
    for c in manifest()["videos"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|c| c["error"].is_null())
    {
        let bytes = std::fs::read(root().join(c["file"].as_str().unwrap())).unwrap();
        let sps = parse(&bytes).unwrap_or_else(|e| panic!("{}: {e}", c["file"]));
        assert_eq!(
            sps.three_d.as_ref().unwrap().active_for_depth(false),
            c["file"].as_str().unwrap().contains("active-texture") || c["file"].as_str().unwrap().contains("inactive-base-vsp") || c["file"].as_str().unwrap().contains("inactive-base-residual")
        );
        if c["file"].as_str().unwrap().contains("depth-only") {
            assert!(sps.three_d.as_ref().unwrap().active_for_depth(true));
        }
        let control = std::fs::read(
            root()
                .parent()
                .unwrap()
                .parent()
                .unwrap()
                .join(format!("hevc/{}.mp4", c["seed"].as_str().unwrap())),
        )
        .unwrap();
        let mut baseline =
            Mp4VideoReader::open_software(Cursor::new(control), Limits::default(), 16 << 20)
                .unwrap();
        let mut expected = Vec::new();
        while let Some(frame) = baseline.read_frame().unwrap() {
            expected.push(frame);
        }
        assert_eq!(expected.len(), 17);
        let mut reader =
            Mp4VideoReader::open_software(Cursor::new(bytes), Limits::default(), 16 << 20).unwrap();
        for _ in 0..2 {
            for target in &expected {
                let frame = reader.read_frame().unwrap().unwrap();
                assert_eq!(frame.presentation_time, target.presentation_time);
                assert_eq!(frame.duration, target.duration);
                assert_eq!(frame.picture.y, target.picture.y);
                assert_eq!(frame.picture.cb, target.picture.cb);
                assert_eq!(frame.picture.cr, target.picture.cr);
            }
            assert!(reader.read_frame().unwrap().is_none());
            reader.rewind();
        }
    }
}
#[test]
fn active_tools_and_malformed_extension_remain_specific_refusals() {
    for c in manifest()["videos"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|c| !c["error"].is_null())
    {
        let bytes = std::fs::read(root().join(c["file"].as_str().unwrap())).unwrap();
        let message = c["error"].as_str().unwrap();
        assert_eq!(parse(&bytes).unwrap_err().to_string(), message);
        let error = Mp4VideoReader::open_software(Cursor::new(bytes), Limits::default(), 16 << 20)
            .err()
            .unwrap();
        assert_eq!(error.to_string(), message);
    }
}

#[test]
fn depth_contour_texture_view_scaling_and_depth_disparity_admit() {
    use fvid::codec::{
        hevc_sps::{LayeredSps, LayeredSpsContext},
        hevc_vps::LayeredVps,
    };
    let read = |name: &str| std::fs::read(root().join(name)).unwrap();
    let vps = LayeredVps::parse(&read("depth-context.vps"), 1 << 20).unwrap();
    let texture = LayeredSps::parse(
        &read("texture-context.sps"),
        &LayeredSpsContext {
            vps: &vps,
            layer_id: 0,
            output_layer_set: 0,
            active_sps: &[],
        },
        1 << 20,
    )
    .unwrap();
    let context = LayeredSpsContext {
        vps: &vps,
        layer_id: 3,
        output_layer_set: 1,
        active_sps: std::slice::from_ref(&texture),
    };
    let contour = LayeredSps::parse(&read("depth-active.sps"), &context, 1 << 20).unwrap();
    assert!(contour.sps.depth_layer);
    assert!(contour.sps.three_d.as_ref().unwrap().intra_contour);
    // This legacy fixture name is retained; its old texture-motion refusal is
    // now parameter acceptance, separately from the coded playback witness.
    let motion =
        LayeredSps::parse(&read("depth-unsupported-motion.sps"), &context, 1 << 20).unwrap();
    assert!(motion.sps.three_d.as_ref().unwrap().texture_motion);
    let view_motion = LayeredSps::parse(
        &read("depth-unsupported-interview-motion.sps"),
        &context,
        1 << 20,
    )
    .unwrap();
    assert!(
        view_motion
            .sps
            .three_d
            .as_ref()
            .unwrap()
            .inter_view_mv_scaling[1]
    );
    let disparity =
        LayeredSps::parse(&read("depth-unsupported-disparity.sps"), &context, 1 << 20).unwrap();
    assert!(disparity.sps.three_d.as_ref().unwrap().inter_view_disparity[1]);
    let inactive = LayeredSps::parse(&read("depth-inactive.sps"), &context, 1 << 20).unwrap();
    assert!(!inactive.sps.three_d.as_ref().unwrap().active());
    assert!(inactive.sps.depth_layer);
    assert!(!texture.sps.depth_layer);
    let mut changed_role = inactive.sps.clone();
    changed_role.depth_layer = false;
    assert_ne!(inactive.sps, changed_role);
    let no_extension = LayeredSps::parse(&read("depth-no-three-d.sps"), &context, 1 << 20).unwrap();
    assert!(no_extension.sps.depth_layer);
    assert!(no_extension.sps.three_d.is_none());
}

#[test]
fn known_extensions_read_three_d_before_scc_palette_and_future_data() {
    use fvid::codec::{
        hevc_sps::{LayeredSps, LayeredSpsContext},
        hevc_vps::LayeredVps,
    };
    let read = |name: &str| std::fs::read(root().join(name)).unwrap();
    let vps = LayeredVps::parse(&read("scc-context.vps"), 1 << 20).unwrap();
    let texture = LayeredSps::parse(
        &read("texture-context.sps"),
        &LayeredSpsContext {
            vps: &vps,
            layer_id: 0,
            output_layer_set: 0,
            active_sps: &[],
        },
        1 << 20,
    )
    .unwrap();
    let current = LayeredSps::parse(
        &read("scc-three-d.sps"),
        &LayeredSpsContext {
            vps: &vps,
            layer_id: 3,
            output_layer_set: 1,
            active_sps: std::slice::from_ref(&texture),
        },
        1 << 20,
    )
    .unwrap();
    assert!(current.sps.scc_extension);
    assert!(!current.sps.three_d.as_ref().unwrap().active());
    assert_eq!(
        current.sps.palette.as_ref().unwrap().initial,
        vec![[3, 4, 5], [9, 10, 11]]
    );
}

#[test]
fn unused_three_d_sps_updates_preserve_inter_picture_references_and_clocks() {
    for seed in ["main-ipb", "main10-ipb"] {
        let control = std::fs::read(
            root()
                .parent()
                .unwrap()
                .parent()
                .unwrap()
                .join(format!("hevc/{seed}.mp4")),
        )
        .unwrap();
        let mut baseline =
            Mp4VideoReader::open_software(Cursor::new(control), Limits::default(), 16 << 20)
                .unwrap();
        let mut expected = Vec::new();
        while let Some(frame) = baseline.read_frame().unwrap() {
            expected.push(frame);
        }
        assert_eq!(expected.len(), 17);
        for mode in ["inactive", "depth-only", "toggle", "update-allowed"] {
            let bytes = std::fs::read(root().join(format!("{seed}-update-{mode}.mp4"))).unwrap();
            let mut reader =
                Mp4VideoReader::open_software(Cursor::new(bytes), Limits::default(), 16 << 20)
                    .unwrap();
            for _ in 0..2 {
                for target in &expected {
                    let frame = reader.read_frame().unwrap().unwrap();
                    assert_eq!(frame.presentation_time, target.presentation_time);
                    assert_eq!(frame.duration, target.duration);
                    assert_eq!(frame.picture.y, target.picture.y);
                    assert_eq!(frame.picture.cb, target.picture.cb);
                    assert_eq!(frame.picture.cr, target.picture.cr);
                }
                assert!(reader.read_frame().unwrap().is_none());
                reader.rewind();
            }
        }
    }
}

#[test]
fn base_iv_sps_updates_remain_inert_without_reference_layers() {
    for seed in ["main-ipb", "main10-ipb"] {
        let control = std::fs::read(
            root()
                .parent()
                .unwrap()
                .parent()
                .unwrap()
                .join(format!("hevc/{seed}.mp4")),
        )
        .unwrap();
        let mut baseline =
            Mp4VideoReader::open_software(Cursor::new(control), Limits::default(), 16 << 20)
                .unwrap();
        let mut expected = Vec::new();
        while let Some(frame) = baseline.read_frame().unwrap() {
            expected.push(frame);
        }
        assert_eq!(expected.len(), 17);
        let bytes =
            std::fs::read(root().join(format!("{seed}-update-active-texture.mp4"))).unwrap();
        let mut reader =
            Mp4VideoReader::open_software(Cursor::new(bytes), Limits::default(), 16 << 20).unwrap();
        for _ in 0..2 {
            for target in &expected {
                let frame = reader.read_frame().unwrap().unwrap();
                assert_eq!(frame.presentation_time, target.presentation_time);
                assert_eq!(frame.duration, target.duration);
                assert_eq!(frame.picture.y, target.picture.y);
                assert_eq!(frame.picture.cb, target.picture.cb);
                assert_eq!(frame.picture.cr, target.picture.cr);
            }
            assert!(reader.read_frame().unwrap().is_none());
            reader.rewind();
        }
    }
}

#[test]
fn no_update_sei_still_forbids_unused_sps_metadata_changes() {
    for seed in ["main-ipb", "main10-ipb"] {
        let bytes = std::fs::read(root().join(format!("{seed}-update-no-update.mp4"))).unwrap();
        let mut reader =
            Mp4VideoReader::open_software(Cursor::new(bytes), Limits::default(), 16 << 20).unwrap();
        for _ in 0..2 {
            let mut refused = false;
            for _ in 0..18 {
                match reader.read_frame() {
                    Ok(Some(_)) => {}
                    Ok(None) => panic!("no-update SEI was bypassed"),
                    Err(e) => {
                        assert_eq!(
                            e.to_string(),
                            "HEVC active parameter SEI forbids parameter updates"
                        );
                        refused = true;
                        break;
                    }
                }
            }
            assert!(refused);
            reader.rewind();
        }
    }
}
