use std::path::{Path, PathBuf};
use std::process::Command;

struct TempDir(PathBuf);
impl TempDir {
    fn new(prefix: &str) -> Self {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let dir = std::env::temp_dir().join(format!(
            "fvid-{prefix}-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&dir).unwrap();
        Self(dir)
    }
}
impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn fixture_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/subtitle-convert-plan-metadata")
}

fn bin_path() -> &'static str {
    env!("CARGO_BIN_EXE_fvid")
}

#[test]
fn plan_convert_subtitles_and_writing_app_metadata_preservation_work_across_api_and_cli() {
    let fix = fixture_dir();
    let srt = fix.join("sample.srt");
    let ass = fix.join("sample.ass");
    let subrip_mkv = fix.join("subrip-source.mkv");
    let ass_mkv = fix.join("ass-source.mkv");
    let tmp = TempDir::new("sub-plan-meta");

    // 1. CLI `fvid media plan convert-subtitles` works across SRT, ASS, and Matroska sources
    //    both with and without an explicit OUTPUT.mkv argument, and accepts --progress as a no-op.
    for (src, expected_codec) in [
        (&srt, "subrip"),
        (&ass, "ass"),
        (&subrip_mkv, "subrip"),
        (&ass_mkv, "ass"),
    ] {
        let out_arg = tmp.0.join("never-created.mkv");
        let cli_plan_no_dest = Command::new(bin_path())
            .args([
                "media",
                "plan",
                "convert-subtitles",
                src.to_str().unwrap(),
                "--codec",
                "ass",
                "--metadata",
                "title=Planned Subtitles",
                "--stream-metadata",
                "0:language=fra",
                "--max-packets",
                "2",
                "--progress",
            ])
            .output()
            .unwrap();
        assert!(
            cli_plan_no_dest.status.success(),
            "CLI plan convert-subtitles without output failed for {}: {}",
            src.display(),
            String::from_utf8_lossy(&cli_plan_no_dest.stderr)
        );
        let plan_json: serde_json::Value = serde_json::from_slice(&cli_plan_no_dest.stdout).unwrap();
        assert_eq!(plan_json["command"], "convert-subtitles");
        assert_eq!(plan_json["streams"][0]["media_type"], "subtitle");
        assert_eq!(plan_json["streams"][0]["codec"], expected_codec);
        assert!(
            plan_json["steps"]
                .as_array()
                .unwrap()
                .iter()
                .any(|s| s["action"] == "metadata"),
            "expected metadata step in plan convert-subtitles"
        );
        assert!(
            plan_json["notes"]
                .as_array()
                .unwrap()
                .iter()
                .any(|n| n.as_str().unwrap_or_default().contains("packet work limit")),
            "expected budget note in plan convert-subtitles"
        );

        let cli_plan_with_dest = Command::new(bin_path())
            .args([
                "media",
                "plan",
                "convert-subtitles",
                src.to_str().unwrap(),
                out_arg.to_str().unwrap(),
                "--metadata-delete",
                "encoder",
                "--stream-metadata",
                "0:title=Custom Track",
            ])
            .output()
            .unwrap();
        assert!(
            cli_plan_with_dest.status.success(),
            "CLI plan convert-subtitles with output failed for {}: {}",
            src.display(),
            String::from_utf8_lossy(&cli_plan_with_dest.stderr)
        );
        assert!(
            !out_arg.exists(),
            "plan convert-subtitles must never create output file"
        );
    }

    // 2. Plan preflight enforces packet limits, byte limits, stream selection, and metadata validation
    let cli_plan_pkt_exceeded = Command::new(bin_path())
        .args([
            "media",
            "plan",
            "convert-subtitles",
            srt.to_str().unwrap(),
            "--max-packets",
            "1",
        ])
        .output()
        .unwrap();
    assert!(
        !cli_plan_pkt_exceeded.status.success(),
        "plan convert-subtitles must fail when source cues exceed --max-packets"
    );

    let cli_plan_bytes_exceeded = Command::new(bin_path())
        .args([
            "media",
            "plan",
            "convert-subtitles",
            ass.to_str().unwrap(),
            "--max-packet-bytes",
            "30",
        ])
        .output()
        .unwrap();
    assert!(
        !cli_plan_bytes_exceeded.status.success(),
        "plan convert-subtitles must fail when a cue exceeds --max-packet-bytes"
    );

    let cli_plan_bad_stream_meta = Command::new(bin_path())
        .args([
            "media",
            "plan",
            "convert-subtitles",
            subrip_mkv.to_str().unwrap(),
            "--stream-metadata",
            "1:title=InvalidStream",
        ])
        .output()
        .unwrap();
    assert!(
        !cli_plan_bad_stream_meta.status.success(),
        "plan convert-subtitles must reject stream metadata index != 0"
    );

    let cli_plan_dup_streams = Command::new(bin_path())
        .args([
            "media",
            "plan",
            "convert-subtitles",
            srt.to_str().unwrap(),
            "--streams",
            "0",
            "--streams",
            "0",
        ])
        .output()
        .unwrap();
    assert!(
        !cli_plan_dup_streams.status.success(),
        "plan convert-subtitles must reject duplicate --streams"
    );

    let cli_plan_bad_dest = Command::new(bin_path())
        .args([
            "media",
            "plan",
            "convert-subtitles",
            srt.to_str().unwrap(),
            tmp.0.join("bad.mp4").to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(
        !cli_plan_bad_dest.status.success(),
        "plan convert-subtitles must reject non-.mkv destination when provided"
    );

    // 3. Matroska convert-subtitles preserves source WritingApp (`encoder`) by default,
    //    and allows overriding (`--metadata encoder=...`) or deleting (`--metadata-delete encoder`).
    for (src, expected_encoder, expected_title, expected_custom_key, expected_custom_val) in [
        (
            &subrip_mkv,
            "FVidSubtitleMkvSynth",
            "SubRip Container Title",
            "CUSTOM_SUB_TAG",
            "SubRipCustomValue",
        ),
        (
            &ass_mkv,
            "FVidAssMkvSynth",
            "ASS Container Title",
            "CUSTOM_ASS_TAG",
            "AssCustomValue",
        ),
    ] {
        let preserved_out = tmp.0.join(format!(
            "preserved-{}",
            src.file_name().unwrap().to_str().unwrap()
        ));
        let stats = fvid::media::convert_subtitles(&src, &preserved_out, &Default::default())
            .expect("convert_subtitles failed on Matroska subtitle source");
        assert_eq!(stats.cues, 2);
        let probed = fvid::media::probe(&preserved_out).unwrap();
        assert_eq!(
            probed.metadata.get("encoder").map(String::as_str),
            Some(expected_encoder),
            "Matroska convert-subtitles must preserve source WritingApp (`encoder`)"
        );
        assert_eq!(
            probed.metadata.get("title").map(String::as_str),
            Some(expected_title)
        );
        assert_eq!(
            probed
                .metadata
                .get(expected_custom_key)
                .map(String::as_str),
            Some(expected_custom_val)
        );

        let edited_out = tmp.0.join(format!(
            "edited-{}",
            src.file_name().unwrap().to_str().unwrap()
        ));
        let cli_edit = Command::new(bin_path())
            .args([
                "media",
                "convert-subtitles",
                src.to_str().unwrap(),
                edited_out.to_str().unwrap(),
                "--metadata-delete",
                "encoder",
                "--metadata-delete",
                expected_custom_key,
                "--metadata",
                "title=Edited Subtitle Container",
                "-q",
            ])
            .output()
            .unwrap();
        assert!(
            cli_edit.status.success(),
            "CLI convert-subtitles with encoder deletion failed: {}",
            String::from_utf8_lossy(&cli_edit.stderr)
        );
        let edited_probed = fvid::media::probe(&edited_out).unwrap();
        assert_eq!(
            edited_probed.metadata.get("encoder").map(String::as_str),
            Some("FVid"),
            "deleted source encoder must not retain source WritingApp (falls back to FVid muxer WritingApp)"
        );
        assert!(!edited_probed.metadata.contains_key(expected_custom_key));
        assert_eq!(
            edited_probed.metadata.get("title").map(String::as_str),
            Some("Edited Subtitle Container")
        );

        let override_out = tmp.0.join(format!(
            "override-{}",
            src.file_name().unwrap().to_str().unwrap()
        ));
        let cli_override = Command::new(bin_path())
            .args([
                "media",
                "convert-subtitles",
                src.to_str().unwrap(),
                override_out.to_str().unwrap(),
                "--metadata",
                "encoder=CustomSubtitleEncoder",
                "-q",
            ])
            .output()
            .unwrap();
        assert!(
            cli_override.status.success(),
            "CLI convert-subtitles with encoder override failed: {}",
            String::from_utf8_lossy(&cli_override.stderr)
        );
        let override_probed = fvid::media::probe(&override_out).unwrap();
        assert_eq!(
            override_probed.metadata.get("encoder").map(String::as_str),
            Some("CustomSubtitleEncoder"),
            "explicit --metadata encoder=... must override source WritingApp"
        );
    }
}
