use fvid::media::CopyOptions;
use std::path::{Path, PathBuf};
use std::process::Command;

struct TempDir(PathBuf);
impl TempDir {
    fn new(tag: &str) -> Self {
        let dir = std::env::temp_dir().join(format!(
            "fvid-subtitle-burn-matroska-xfade-meta-{tag}-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        Self(dir)
    }
}
impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/subtitle-burn-matroska-xfade-metadata")
        .join(name)
}

fn bin_path() -> &'static str {
    env!("CARGO_BIN_EXE_fvid")
}

#[test]
fn subtitle_burn_supports_compressed_and_rotated_matroska_preserves_writing_app_and_xfade_enforces_container_metadata_bounds() {
    let dir = TempDir::new("all");
    let vp9_source = fixture("vp9-burn-source.webm");
    let vp9_rotated = fixture("vp9-rotated-cropped-burn.mkv");
    let srt = fixture("burn-cues.srt");
    let xfade_main = fixture("xfade-main-4x4.y4m");
    let xfade_other = fixture("xfade-other-4x4.y4m");

    // 1. Single-track VP9 WebM/Matroska is admitted by `owned_subtitle_burn::supports`,
    //    `plan_burn_subtitles`, and `burn_subtitles` (both library API and CLI without PATH),
    //    and preserves source Matroska `WritingApp` (`"FVidVp9BurnSynth"`) as `ENCODER` metadata
    //    in both `burn_subtitles` and `transcode_lossless`.
    assert!(
        fvid_media::owned_subtitle_burn::supports(&vp9_source, &srt, &Default::default()),
        "owned_subtitle_burn::supports must admit single-track VP9 WebM"
    );
    let vp9_plan = fvid::media::plan_burn_subtitles(&vp9_source, &srt, &Default::default())
        .expect("plan_burn_subtitles on single-track VP9 WebM must succeed");
    assert_eq!(vp9_plan.command, "burn-subtitles");
    assert!(vp9_plan.graph.is_none());
    assert_eq!(vp9_plan.streams.len(), 1);
    assert_eq!(vp9_plan.streams[0].codec, "vp9");

    // Budget preflight in `plan_burn_subtitles` on VP9 Matroska must reject `max_packets=1`.
    let too_few_packets = CopyOptions {
        max_packets: Some(1),
        ..Default::default()
    };
    let pkt_err = fvid::media::plan_burn_subtitles(&vp9_source, &srt, &too_few_packets)
        .expect_err("plan_burn_subtitles on 3-frame VP9 WebM must reject max_packets=1");
    assert!(
        pkt_err.contains("packet count exceeds limit"),
        "unexpected max_packets error: {pkt_err}"
    );

    let vp9_burn_out = dir.0.join("vp9-burned.mkv");
    let burn_stats =
        fvid::media::burn_subtitles(&vp9_source, &vp9_burn_out, &srt, &Default::default())
            .expect("burn_subtitles on single-track VP9 WebM must succeed");
    assert_eq!(burn_stats.backend, "owned SRT burn-in FFV1 export");
    assert_eq!(burn_stats.video_frames, 3);
    assert_eq!(burn_stats.decoded_frames, 3);
    assert_eq!(burn_stats.encoder, "ffv1");

    let mut burned_reader = fvid_media::owned_webm::WebmReader::open(
        std::fs::File::open(&vp9_burn_out).unwrap(),
        Default::default(),
    )
    .unwrap();
    burned_reader.scan_all().unwrap();
    assert_eq!(burned_reader.packets.len(), 3);
    assert_eq!(burned_reader.tags.title, "VP9 Burn Source Title");
    assert_eq!(
        burned_reader.metadata.get("CUSTOM_SOURCE_NOTE").map(String::as_str),
        Some("PreservedCustomNote")
    );
    assert_eq!(
        burned_reader.metadata.get("ENCODER").map(String::as_str),
        Some("FVidVp9BurnSynth"),
        "burn_subtitles on Matroska must preserve source WritingApp as ENCODER tag"
    );

    // Verify frame 1 (PTS 500ms) has subtitle pixels burned in while frames 0 and 2 match each other.
    let mut decoder = fvid_media::owned_ffv1_decoder::Decoder::new(96, 64, 1 << 20).unwrap();
    let f0 = decoder.decode(&burned_reader.read_packet(0).unwrap()).unwrap();
    let f1 = decoder.decode(&burned_reader.read_packet(1).unwrap()).unwrap();
    let f2 = decoder.decode(&burned_reader.read_packet(2).unwrap()).unwrap();
    assert_eq!(f0.frame.data, f2.frame.data);
    assert_ne!(f1.frame.data, f0.frame.data, "frame 1 must contain burned-in SRT text");

    // Also verify `transcode_lossless` preserves source `WritingApp` (`"FVidVp9BurnSynth"`)
    // and allows deleting `encoder` via `metadata_delete`.
    let lossless_out = dir.0.join("vp9-lossless.mkv");
    fvid::media::transcode_lossless(&vp9_source, &lossless_out, Default::default(), &Default::default())
        .expect("transcode_lossless on VP9 source");
    let lossless_probe = fvid::media::probe(&lossless_out).unwrap();
    assert_eq!(
        lossless_probe.metadata.get("ENCODER").or_else(|| lossless_probe.metadata.get("encoder")).map(String::as_str),
        Some("FVidVp9BurnSynth")
    );
    let lossless_del_out = dir.0.join("vp9-lossless-no-encoder.mkv");
    let del_encoder_opts = CopyOptions {
        metadata_delete: vec!["encoder".into()],
        ..Default::default()
    };
    fvid::media::transcode_lossless(&vp9_source, &lossless_del_out, Default::default(), &del_encoder_opts)
        .expect("transcode_lossless with metadata_delete=[encoder]");
    let mut del_reader = fvid_media::owned_webm::WebmReader::open(
        std::fs::File::open(&lossless_del_out).unwrap(),
        Default::default(),
    )
    .unwrap();
    del_reader.scan_all().unwrap();
    assert!(!del_reader.metadata.contains_key("ENCODER"));

    // 2. Rotated (90 deg clockwise) and cropped Matroska source (`64x96` coded -> `56x96` cropped
    //    -> `96x56` post-rotation) is planned and burned at visible post-rotation dimensions `96x56`.
    assert!(
        fvid_media::owned_subtitle_burn::supports(&vp9_rotated, &srt, &Default::default()),
        "owned_subtitle_burn::supports must admit rotated+cropped VP9 Matroska"
    );
    let rot_plan = fvid::media::plan_burn_subtitles(&vp9_rotated, &srt, &Default::default())
        .expect("plan_burn_subtitles on rotated+cropped VP9 Matroska");
    assert_eq!(rot_plan.command, "burn-subtitles");
    let rot_out = dir.0.join("vp9-rotated-burned.mkv");
    let rot_stats = fvid::media::burn_subtitles(&vp9_rotated, &rot_out, &srt, &Default::default())
        .expect("burn_subtitles on rotated+cropped VP9 Matroska");
    assert_eq!(rot_stats.video_frames, 2);
    let rot_probe = fvid::media::probe(&rot_out).unwrap();
    assert_eq!(rot_probe.streams[0].width, 96);
    assert_eq!(rot_probe.streams[0].height, 56);

    // CLI `plan burn-subtitles` and `burn-subtitles` on VP9 Matroska without PATH.
    let cli_plan = Command::new(bin_path())
        .args([
            "media",
            "plan",
            "burn-subtitles",
            vp9_source.to_str().unwrap(),
            "--subs",
            srt.to_str().unwrap(),
        ])
        .env("PATH", "/nonexistent")
        .output()
        .unwrap();
    assert!(
        cli_plan.status.success(),
        "cli plan burn-subtitles on VP9 failed: {}",
        String::from_utf8_lossy(&cli_plan.stderr)
    );
    let cli_burn_out = dir.0.join("cli-vp9-burned.mkv");
    let cli_burn = Command::new(bin_path())
        .args([
            "media",
            "burn-subtitles",
            vp9_source.to_str().unwrap(),
            cli_burn_out.to_str().unwrap(),
            "--subs",
            srt.to_str().unwrap(),
            "-q",
        ])
        .env("PATH", "/nonexistent")
        .output()
        .unwrap();
    assert!(
        cli_burn.status.success(),
        "cli burn-subtitles on VP9 failed: {}",
        String::from_utf8_lossy(&cli_burn.stderr)
    );
    assert!(cli_burn_out.is_file());

    // 3. `owned_xfade` (`try_plan_xfade`, `plan_xfade`, `try_xfade_video`, `xfade_video`) and
    //    `owned_lossless::metadata` must enforce container metadata bounds:
    //    - at most 64 container metadata mutations (`metadata_set.len() + metadata_delete.len() <= 64`)
    //    - no NUL bytes in keys or values
    //    - non-empty key (`!key.is_empty()`), key length `<= 128`, and custom tag value length `<= 1024`.
    let empty_key_opts = CopyOptions {
        metadata_set: vec![("".into(), "val".into())],
        ..Default::default()
    };
    let long_key_opts = CopyOptions {
        metadata_set: vec![("K".repeat(129), "val".into())],
        ..Default::default()
    };
    let long_val_opts = CopyOptions {
        metadata_set: vec![("CUSTOM_KEY".into(), "V".repeat(1025))],
        ..Default::default()
    };
    let empty_del_opts = CopyOptions {
        metadata_delete: vec!["".into()],
        ..Default::default()
    };
    let long_del_opts = CopyOptions {
        metadata_delete: vec!["D".repeat(129)],
        ..Default::default()
    };
    let nul_opts = CopyOptions {
        metadata_set: vec![("CUSTOM_KEY".into(), "bad\0val".into())],
        ..Default::default()
    };
    let too_many_opts = CopyOptions {
        metadata_set: (0..65)
            .map(|i| (format!("KEY_{i}"), "val".into()))
            .collect(),
        ..Default::default()
    };

    for (label, bad_opts) in [
        ("empty metadata_set key", &empty_key_opts),
        ("oversized metadata_set key (>128)", &long_key_opts),
        ("oversized custom metadata_set value (>1024)", &long_val_opts),
        ("empty metadata_delete key", &empty_del_opts),
        ("oversized metadata_delete key (>128)", &long_del_opts),
        ("NUL in metadata_set value", &nul_opts),
        (">64 container metadata mutations", &too_many_opts),
    ] {
        assert!(
            fvid_media::owned_xfade::try_plan_xfade(
                &xfade_main,
                &xfade_other,
                "fade",
                500_000,
                500_000,
                bad_opts,
            )
            .unwrap()
            .is_none(),
            "try_plan_xfade must not admit {label}"
        );
        assert!(
            fvid::media::plan_xfade(
                &xfade_main,
                &xfade_other,
                "fade",
                500_000,
                500_000,
                bad_opts,
            )
            .is_err(),
            "plan_xfade must reject {label}"
        );
        let bad_out = dir.0.join("should-not-exist.mkv");
        assert!(
            fvid_media::owned_xfade::try_xfade_video(
                &xfade_main,
                &xfade_other,
                &bad_out,
                "fade",
                500_000,
                500_000,
                bad_opts,
            )
            .unwrap()
            .is_none(),
            "try_xfade_video must not admit {label}"
        );
        assert!(
            fvid::media::xfade_video(
                &xfade_main,
                &xfade_other,
                &bad_out,
                "fade",
                500_000,
                500_000,
                bad_opts,
            )
            .is_err(),
            "xfade_video must reject {label}"
        );
        assert!(!bad_out.exists());
    }
}
