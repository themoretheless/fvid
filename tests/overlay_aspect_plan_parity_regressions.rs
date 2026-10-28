use fvid::{
    media::{self, CopyOptions, LosslessTransform, OverlaySpec, ScaleSize},
    media_control::CancelFlag,
    native_export,
    native_geometry::VideoGeometry,
    native_media,
    native_pixels::PixelFilters,
    native_plan,
    playback_native::NativeReader,
};
use std::{
    fs::File,
    io::BufReader,
    path::{Path, PathBuf},
    process::Command,
};

struct TempDir(PathBuf);
impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn temp_dir(tag: &str) -> TempDir {
    let dir = std::env::temp_dir().join(format!(
        "fvid-overlay-aspect-plan-parity-{tag}-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    TempDir(dir)
}

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/overlay-aspect-plan-parity")
        .join(name)
}

#[test]
fn rot90_main_or_foreground_preserves_upright_display_pixel_aspect_in_overlay_and_decode() {
    let d = temp_dir("rot90");
    let rot90 = fixture("rot90-par1x2.mkv");
    let par2x1 = fixture("upright-par2x1.y4m");
    let par1x2 = fixture("upright-par1x2.y4m");

    // Verify reader upright display pixel aspect is (2, 1) on both rot90 and par2x1.
    let mut rot_reader =
        NativeReader::software(BufReader::new(File::open(&rot90).unwrap()), usize::MAX).unwrap();
    assert!(rot_reader.read_frame_raw().unwrap().is_some());
    assert_eq!(rot_reader.rotation(), 90);
    assert_eq!(rot_reader.pixel_aspect(), (2, 1));

    let mut y4m_reader =
        NativeReader::software(BufReader::new(File::open(&par2x1).unwrap()), usize::MAX).unwrap();
    assert!(y4m_reader.read_frame_raw().unwrap().is_some());
    assert_eq!(y4m_reader.rotation(), 0);
    assert_eq!(y4m_reader.pixel_aspect(), (2, 1));
    assert_eq!(rot_reader.colour(), y4m_reader.colour());

    // 1. rot90-par1x2.mkv (upright PAR 2:1) paired with upright-par2x1.y4m (PAR 2:1) must succeed
    // in both directions across execution, decode, and planning.
    for (tag, main, fg) in [("rot-main", &rot90, &par2x1), ("rot-fg", &par2x1, &rot90)] {
        let plan = native_plan::overlay(main, fg, 0, 0)
            .unwrap_or_else(|e| panic!("{tag} native_plan::overlay failed: {e}"));
        assert_eq!(plan.command, "overlay");

        let pub_plan = media::plan_overlay(main, fg, 0, 0, &CopyOptions::default())
            .unwrap_or_else(|e| panic!("{tag} media::plan_overlay failed: {e}"));
        assert_eq!(pub_plan.command, "overlay");

        let dec_plan = native_plan::decode_overlay(main, fg, 0, 0)
            .unwrap_or_else(|e| panic!("{tag} native_plan::decode_overlay failed: {e}"));
        assert_eq!(dec_plan.command, "decode");

        let spec = OverlaySpec {
            path: fg.clone(),
            x: 0,
            y: 0,
        };
        let dec_stats = native_media::decode_video_pipeline_overlay(
            main,
            None,
            &VideoGeometry::default(),
            &PixelFilters::default(),
            Some(&spec),
        )
        .unwrap_or_else(|e| panic!("{tag} decode_video_pipeline_overlay failed: {e}"));
        assert_eq!(dec_stats.video_frames, 2);
        assert_eq!((dec_stats.width, dec_stats.height), (2, 4));

        let out = d.0.join(format!("{tag}.mkv"));
        let stats = native_export::overlay_video(main, fg, &out, 0, 0, None, None)
            .unwrap_or_else(|e| panic!("{tag} overlay_video failed: {e}"));
        assert_eq!(stats.video_frames, 2);

        let mut out_reader =
            NativeReader::software(BufReader::new(File::open(&out).unwrap()), usize::MAX).unwrap();
        assert!(out_reader.read_frame_raw().unwrap().is_some());
        assert_eq!(out_reader.dimensions(), [2, 4]);
        assert_eq!(out_reader.rotation(), 0);
        assert_eq!(out_reader.pixel_aspect(), (2, 1));
    }

    // 2. Pairing rot90-par1x2.mkv (upright PAR 2:1) with upright-par1x2.y4m (PAR 1:2) must fail
    // in both execution and planning!
    for (tag, main, fg) in [("mismatch-main", &rot90, &par1x2), ("mismatch-fg", &par1x2, &rot90)] {
        let out = d.0.join(format!("{tag}.mkv"));
        let err = native_export::overlay_video(main, fg, &out, 0, 0, None, None)
            .unwrap_err()
            .to_string();
        assert!(
            err.contains("overlay inputs require matching display pixel aspect and HDR metadata"),
            "{tag} overlay_video returned: {err}"
        );
        assert!(!out.exists());

        let plan_err = native_plan::overlay(main, fg, 0, 0).unwrap_err();
        assert!(
            plan_err.contains("overlay inputs require matching display pixel aspect and HDR metadata"),
            "{tag} native_plan::overlay returned: {plan_err}"
        );

        let dec_plan_err = native_plan::decode_overlay(main, fg, 0, 0).unwrap_err();
        assert!(
            dec_plan_err.contains("overlay inputs require matching display pixel aspect and HDR metadata"),
            "{tag} native_plan::decode_overlay returned: {dec_plan_err}"
        );

        let spec = OverlaySpec {
            path: fg.clone(),
            x: 0,
            y: 0,
        };
        let dec_err = native_media::decode_video_pipeline_overlay(
            main,
            None,
            &VideoGeometry::default(),
            &PixelFilters::default(),
            Some(&spec),
        )
        .unwrap_err()
        .to_string();
        assert!(
            dec_err.contains("overlay inputs require matching display pixel aspect and HDR metadata"),
            "{tag} decode_video_pipeline_overlay returned: {dec_err}"
        );
    }
}

#[test]
fn pre_overlay_geometry_transformed_aspect_and_subsampling_are_validated_in_execution_and_plan() {
    let d = temp_dir("geometry-aspect");
    let main_4x4 = fixture("upright-par1x1-4x4.y4m");
    let fg_par2x1 = fixture("upright-par2x1.y4m");
    let fg_par1x2 = fixture("upright-par1x2.y4m");

    // Scaling 4x4 (PAR 1:1, display 1:1) to 2x4 produces transformed PAR 2:1.
    // Therefore overlaying fg_par2x1 (PAR 2:1) onto scaled main_4x4 must succeed!
    let scale_geom = VideoGeometry {
        scale: Some([2, 4]),
        ..Default::default()
    };
    let out_scaled = d.0.join("scaled-par2x1.mkv");
    let stats = native_export::overlay_video_transformed(
        &main_4x4,
        &fg_par2x1,
        &out_scaled,
        0,
        0,
        None,
        None,
        &scale_geom,
        &PixelFilters::default(),
    )
    .expect("scaled main (post-scale PAR 2:1) with PAR 2:1 foreground should succeed");
    assert_eq!(stats.video_frames, 2);

    let spec_par2x1 = OverlaySpec {
        path: fg_par2x1.clone(),
        x: 0,
        y: 0,
    };
    let dec_stats = native_media::decode_video_pipeline_overlay(
        &main_4x4,
        None,
        &scale_geom,
        &PixelFilters::default(),
        Some(&spec_par2x1),
    )
    .expect("decode_video_pipeline_overlay with scaled main should succeed");
    assert_eq!((dec_stats.width, dec_stats.height), (2, 4));

    let transform_ok = LosslessTransform {
        scale: Some(ScaleSize {
            width: 2,
            height: 4,
        }),
        overlay: Some(spec_par2x1.clone()),
        ..Default::default()
    };
    let plan_ok = media::plan_transcode_lossless(&main_4x4, &transform_ok, &CopyOptions::default(), None)
        .expect("plan_transcode_lossless with matching post-scale PAR 2:1 should succeed");
    assert_eq!(plan_ok.command, "transcode-lossless");

    // Conversely, overlaying main_4x4 (PAR 1:1) scaled to 2x4 (post-scale PAR 2:1) with main_4x4 (PAR 1:1)
    // must be rejected in both execution and planning because post-scale main PAR is 2:1 != 1:1!
    let spec_par1x1 = OverlaySpec {
        path: main_4x4.clone(),
        x: 0,
        y: 0,
    };
    let out_mismatch = d.0.join("scaled-mismatch.mkv");
    let exec_err = native_export::overlay_video_transformed(
        &main_4x4,
        &main_4x4,
        &out_mismatch,
        0,
        0,
        None,
        None,
        &scale_geom,
        &PixelFilters::default(),
    )
    .unwrap_err()
    .to_string();
    assert!(
        exec_err.contains("overlay inputs require matching display pixel aspect and HDR metadata"),
        "overlay_video_transformed mismatch returned: {exec_err}"
    );
    assert!(!out_mismatch.exists());

    let transform_bad = LosslessTransform {
        scale: Some(ScaleSize {
            width: 2,
            height: 4,
        }),
        overlay: Some(spec_par1x1),
        ..Default::default()
    };
    let plan_err = media::plan_transcode_lossless(&main_4x4, &transform_bad, &CopyOptions::default(), None)
        .unwrap_err();
    assert!(
        plan_err.contains("overlay inputs require matching display pixel aspect and HDR metadata"),
        "plan_transcode_lossless mismatch returned: {plan_err}"
    );

    // Also verify CLI `plan decode` with `--scale 2:4 --overlay` validates post-geometry aspect:
    let bin = env!("CARGO_BIN_EXE_fvid");
    let cli_plan_ok = Command::new(bin)
        .args(["media", "plan", "decode"])
        .arg(&main_4x4)
        .args(["--scale", "2:4", "--overlay"])
        .arg(&fg_par2x1)
        .output()
        .unwrap();
    assert!(
        cli_plan_ok.status.success(),
        "cli plan decode failed: {}",
        String::from_utf8_lossy(&cli_plan_ok.stderr)
    );

    let cli_plan_bad = Command::new(bin)
        .args(["media", "plan", "decode"])
        .arg(&main_4x4)
        .args(["--scale", "2:4", "--overlay"])
        .arg(&fg_par1x2)
        .output()
        .unwrap();
    assert!(!cli_plan_bad.status.success());
}

#[test]
fn overlay_planners_reject_incompatible_colour_subsampling_unaligned_offset_and_cancelled_options() {
    let d = temp_dir("planner-parity");
    let main = fixture("upright-par2x1.y4m");
    let fullrange = fixture("upright-fullrange.y4m");
    let yuv444 = fixture("upright-444.y4m");

    // 1. Colour range mismatch
    for err in [
        native_plan::overlay(&main, &fullrange, 0, 0).unwrap_err(),
        native_plan::decode_overlay(&main, &fullrange, 0, 0).unwrap_err(),
        media::plan_overlay(&main, &fullrange, 0, 0, &CopyOptions::default()).unwrap_err(),
    ] {
        assert!(
            err.contains("overlay inputs require matching colour encoding and range"),
            "unexpected colour mismatch error: {err}"
        );
    }

    // 2. Chroma subsampling mismatch (C420 main vs C444 foreground)
    for err in [
        native_plan::overlay(&main, &yuv444, 0, 0).unwrap_err(),
        native_plan::decode_overlay(&main, &yuv444, 0, 0).unwrap_err(),
        media::plan_overlay(&main, &yuv444, 0, 0, &CopyOptions::default()).unwrap_err(),
    ] {
        assert!(
            err.contains("overlay requires matching chroma sampling"),
            "unexpected subsampling mismatch error: {err}"
        );
    }

    // 3. Unaligned chroma placement (x = 1 on 4:2:0)
    for err in [
        native_plan::overlay(&main, &main, 1, 0).unwrap_err(),
        native_plan::decode_overlay(&main, &main, 1, 0).unwrap_err(),
        media::plan_overlay(&main, &main, 1, 0, &CopyOptions::default()).unwrap_err(),
    ] {
        assert!(
            err.contains("overlay placement must align with chroma samples"),
            "unexpected unaligned placement error: {err}"
        );
    }

    // 4. Pre-cancelled CancelFlag in media::plan_overlay and media::plan_transcode_lossless
    let cancel = CancelFlag::new();
    cancel.cancel();
    let mut cancelled_opts = CopyOptions::default();
    cancelled_opts.cancel = Some(cancel);

    let ov_cancel_err = media::plan_overlay(&main, &main, 0, 0, &cancelled_opts).unwrap_err();
    assert_eq!(ov_cancel_err, "media operation cancelled");

    let lossless_cancel_err = media::plan_transcode_lossless(
        &main,
        &LosslessTransform::default(),
        &cancelled_opts,
        None,
    )
    .unwrap_err();
    assert_eq!(lossless_cancel_err, "media operation cancelled");

    let out_cancel = d.0.join("cancelled.mkv");
    let exec_cancel_err =
        media::overlay_video(&main, &main, &out_cancel, 0, 0, &cancelled_opts).unwrap_err();
    assert_eq!(exec_cancel_err, "media operation cancelled");
    assert!(!out_cancel.exists());
}

#[test]
fn cli_transcode_lossless_and_plan_with_mkv_overlay_use_owned_compositor() {
    let d = temp_dir("cli-mkv-overlay");
    let main_4x4 = fixture("upright-par1x1-4x4.y4m");
    let rot90_fg = fixture("rot90-par1x2.mkv");
    let bin = env!("CARGO_BIN_EXE_fvid");

    let plan_out = Command::new(bin)
        .args(["media", "plan", "transcode-lossless"])
        .arg(&main_4x4)
        .args(["--scale", "2:4", "--overlay"])
        .arg(&rot90_fg)
        .output()
        .unwrap();
    assert!(
        plan_out.status.success(),
        "cli plan transcode-lossless failed: {}",
        String::from_utf8_lossy(&plan_out.stderr)
    );
    let plan_json: serde_json::Value = serde_json::from_slice(&plan_out.stdout).unwrap();
    assert_eq!(plan_json["command"], "transcode-lossless");

    let cli_mkv = d.0.join("cli-out.mkv");
    let run_out = Command::new(bin)
        .args(["media", "transcode-lossless"])
        .arg(&main_4x4)
        .arg(&cli_mkv)
        .args(["--scale", "2:4", "--overlay"])
        .arg(&rot90_fg)
        .output()
        .unwrap();
    assert!(
        run_out.status.success(),
        "cli transcode-lossless failed: {}",
        String::from_utf8_lossy(&run_out.stderr)
    );
    let stats_json: serde_json::Value = serde_json::from_slice(&run_out.stdout).unwrap();
    assert_eq!(stats_json["backend"], "fvid");
    assert_eq!(stats_json["video_frames"], 2);

    let api_mkv = d.0.join("api-out.mkv");
    let transform = LosslessTransform {
        scale: Some(ScaleSize {
            width: 2,
            height: 4,
        }),
        overlay: Some(OverlaySpec {
            path: rot90_fg,
            x: 0,
            y: 0,
        }),
        ..Default::default()
    };
    media::transcode_lossless(&main_4x4, &api_mkv, transform, &CopyOptions::default()).unwrap();
    assert_eq!(std::fs::read(&cli_mkv).unwrap(), std::fs::read(&api_mkv).unwrap());
}
