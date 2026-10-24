use fvid::media::CopyOptions;
use fvid_media::LosslessTransform;
use std::path::{Path, PathBuf};
use std::process::Command;

struct TempDir(PathBuf);
impl TempDir {
    fn new(tag: &str) -> Self {
        let dir = std::env::temp_dir().join(format!(
            "fvid-mp4-remux-custom-tags-{tag}-{}",
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
        .join("tests/fixtures/playback-errors/mp4-remux-custom-tags")
        .join(name)
}

fn bin_path() -> &'static str {
    env!("CARGO_BIN_EXE_fvid")
}

#[test]
fn mp4_remux_and_y4m_lossless_support_custom_container_and_stream_tags_and_deletions() {
    let dir = TempDir::new("custom-tags");
    let mp4 = fixture("two-track-avc.mp4");
    let y4m = fixture("tiny-two-frames.y4m");

    // 1. Public API `plan_remux` and `remux` on MP4 with custom container text tags and custom stream-scoped text tags.
    let mp4_opts = CopyOptions {
        streams: vec![1, 0],
        metadata_set: vec![
            ("title".into(), "Remuxed Custom MP4".into()),
            ("custom_container_tag".into(), "ContainerVal".into()),
            ("deleted_container_tag".into(), String::new()),
        ],
        metadata_delete: vec!["artist".into(), "deleted_container_tag".into()],
        stream_metadata_set: vec![
            (1, "title".into(), "Track 1 Custom Title".into()),
            (1, "custom_stream_tag".into(), "Stream1Val".into()),
            (0, "deleted_stream_tag".into(), String::new()),
            (0, "encoder".into(), "FVidCustomMux".into()),
        ],
        stream_metadata_delete: vec![
            (0, "language".into()),
            (0, "deleted_stream_tag".into()),
        ],
        ..Default::default()
    };

    let plan = fvid::media::plan_remux(&mp4, &mp4_opts)
        .expect("plan_remux MP4 with custom container and stream tags");
    assert_eq!(plan.command, "remux");
    assert_eq!(
        plan.streams.iter().map(|s| s.index).collect::<Vec<_>>(),
        vec![1, 0]
    );

    let remux_mkv = dir.0.join("remux-custom.mkv");
    let stats = fvid::media::remux(&mp4, &remux_mkv, &mp4_opts)
        .expect("remux MP4 to MKV with custom container and stream tags");
    assert!(stats.packets > 0);

    let probed = fvid::media::probe(&remux_mkv).unwrap();
    assert_eq!(
        probed.metadata.get("title").map(String::as_str),
        Some("Remuxed Custom MP4")
    );
    assert_eq!(
        probed.metadata.get("CUSTOM_CONTAINER_TAG").map(String::as_str),
        Some("ContainerVal")
    );
    assert!(!probed.metadata.contains_key("artist"));
    assert!(!probed.metadata.contains_key("DELETED_CONTAINER_TAG"));

    // Stream 0 in output corresponds to source track 1; stream 1 corresponds to source track 0.
    assert_eq!(
        probed.streams[0].metadata.get("title").map(String::as_str),
        Some("Track 1 Custom Title")
    );
    assert_eq!(
        probed.streams[0]
            .metadata
            .get("CUSTOM_STREAM_TAG")
            .map(String::as_str),
        Some("Stream1Val")
    );
    assert_eq!(
        probed.streams[1].metadata.get("ENCODER").map(String::as_str),
        Some("FVidCustomMux")
    );
    assert!(!probed.streams[1].metadata.contains_key("DELETED_STREAM_TAG"));

    // Reserved stream keys (`rotate`, `stereo_mode`, `alpha_mode`) must still be rejected.
    let bad_stream_key = CopyOptions {
        stream_metadata_set: vec![(0, "rotate".into(), "90".into())],
        ..Default::default()
    };
    assert!(fvid::media::plan_remux(&mp4, &bad_stream_key).is_err());
    assert!(fvid::media::remux(&mp4, &dir.0.join("bad-rotate.mkv"), &bad_stream_key).is_err());

    // 2. CLI `plan remux` and `remux` on MP4 with custom container and stream tags.
    let cli_mkv = dir.0.join("cli-remux-custom.mkv");
    let cli_remux = Command::new(bin_path())
        .args([
            "media",
            "remux",
            mp4.to_str().unwrap(),
            cli_mkv.to_str().unwrap(),
            "--streams",
            "1,0",
            "--metadata",
            "custom_cli_tag=CliContainerVal",
            "--stream-metadata",
            "1:custom_cli_stream=CliStreamVal",
        ])
        .output()
        .unwrap();
    assert!(
        cli_remux.status.success(),
        "cli remux failed: {}",
        String::from_utf8_lossy(&cli_remux.stderr)
    );
    let cli_probed = fvid::media::probe(&cli_mkv).unwrap();
    assert_eq!(
        cli_probed.metadata.get("CUSTOM_CLI_TAG").map(String::as_str),
        Some("CliContainerVal")
    );
    assert_eq!(
        cli_probed.streams[0]
            .metadata
            .get("CUSTOM_CLI_STREAM")
            .map(String::as_str),
        Some("CliStreamVal")
    );

    // 3. Y4M `transcode_lossless` stream metadata deletions (e.g. setting custom tag and deleting title/language).
    let y4m_opts = CopyOptions {
        stream_metadata_set: vec![
            (0, "kept_y4m_tag".into(), "KeptVal".into()),
            (0, "empty_y4m_tag".into(), String::new()),
        ],
        stream_metadata_delete: vec![
            (0, "title".into()),
            (0, "language".into()),
            (0, "custom_y4m_tag".into()),
        ],
        ..Default::default()
    };
    let y4m_mkv = dir.0.join("y4m-lossless.mkv");
    fvid::media::transcode_lossless(&y4m, &y4m_mkv, LosslessTransform::default(), &y4m_opts)
        .expect("transcode_lossless Y4M with stream_metadata_delete");
    let y4m_probed = fvid::media::probe(&y4m_mkv).unwrap();
    assert!(!y4m_probed.streams[0].metadata.contains_key("title"));
    assert!(!y4m_probed.streams[0].metadata.contains_key("CUSTOM_Y4M_TAG"));
    assert!(!y4m_probed.streams[0].metadata.contains_key("EMPTY_Y4M_TAG"));
    assert_eq!(
        y4m_probed.streams[0]
            .metadata
            .get("KEPT_Y4M_TAG")
            .map(String::as_str),
        Some("KeptVal")
    );

    // Also test Y4M `transcode_lossless` with ONLY `stream_metadata_delete` (no `stream_metadata_set`).
    let y4m_del_only_opts = CopyOptions {
        stream_metadata_delete: vec![(0, "language".into())],
        ..Default::default()
    };
    let y4m_del_mkv = dir.0.join("y4m-del-only.mkv");
    fvid::media::transcode_lossless(
        &y4m,
        &y4m_del_mkv,
        LosslessTransform::default(),
        &y4m_del_only_opts,
    )
    .expect("transcode_lossless Y4M with only stream_metadata_delete");
    let mut webm_reader = fvid_media::owned_webm::WebmReader::open(
        std::io::BufReader::new(std::fs::File::open(&y4m_del_mkv).unwrap()),
        Default::default(),
    )
    .unwrap();
    webm_reader.scan_all().unwrap();
    // When `description` is emitted (because `stream_metadata_delete` is non-empty), `track_languages` (0x22b59d) is written!
    assert_eq!(
        webm_reader.track_languages.get(&1).map(String::as_str),
        Some("und")
    );
}
