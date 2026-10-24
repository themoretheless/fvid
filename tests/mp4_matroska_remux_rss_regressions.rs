use fvid::media::CopyOptions;
use std::path::{Path, PathBuf};
use std::process::Command;

struct TempDir(PathBuf);
impl TempDir {
    fn new(tag: &str) -> Self {
        let dir = std::env::temp_dir().join(format!(
            "fvid-mp4-matroska-remux-rss-{tag}-{}",
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
        .join("tests/fixtures/playback-errors/mp4-matroska-remux-rss")
        .join(name)
}

fn bin_path() -> &'static str {
    env!("CARGO_BIN_EXE_fvid")
}

#[test]
fn mp4_matroska_and_adts_remux_support_metadata_streams_and_rss_budgets_in_api_and_cli() {
    let dir = TempDir::new("remux-rss");
    let mp4 = fixture("two-track-avc.mp4");
    let mkv = fixture("video.mkv");
    let adts = fixture("adts-48k.aac");

    assert_eq!(fvid::media::decode_video(&mkv).unwrap().video_frames, 1);

    // 1. Public API `plan_remux` & `remux` on MP4 with stream selection, metadata edits, packet cap, and RSS budget.
    let mp4_opts = CopyOptions {
        streams: vec![1, 0],
        max_packets: Some(3),
        max_rss_bytes: Some(4096 * 1024 * 1024),
        metadata_delete: vec!["artist".into()],
        metadata_set: vec![("title".into(), "Edited MP4 Title".into())],
        stream_metadata_set: vec![(1, "title".into(), "Renamed Track 1".into())],
        ..Default::default()
    };
    let mp4_plan = fvid::media::plan_remux(&mp4, &mp4_opts).expect("plan_remux MP4 with edits and RSS");
    assert_eq!(mp4_plan.command, "remux");
    assert_eq!(
        mp4_plan.streams.iter().map(|s| s.index).collect::<Vec<_>>(),
        vec![1, 0]
    );

    let mp4_out = dir.0.join("mp4-remux.mkv");
    let mp4_stats = fvid::media::remux(&mp4, &mp4_out, &mp4_opts).expect("remux MP4 to MKV with edits and RSS");
    assert_eq!(mp4_stats.packets, 3);
    let probed = fvid::media::probe(&mp4_out).unwrap();
    assert_eq!(
        probed.metadata.get("title").map(String::as_str),
        Some("Edited MP4 Title")
    );
    assert!(!probed.metadata.contains_key("artist"));

    // Tight RSS budget on MP4 plan_remux and remux fails cleanly.
    let tight_rss = CopyOptions {
        max_rss_bytes: Some(1024 * 1024),
        ..Default::default()
    };
    assert!(fvid::media::plan_remux(&mp4, &tight_rss).is_err());
    assert!(fvid::media::remux(&mp4, &dir.0.join("never-mp4.mkv"), &tight_rss).is_err());

    // 2. CLI `plan remux` and `remux` on MP4 with `--streams`, `--metadata`, `--stream-metadata`, `--max-packets`, and `--max-rss-mib`.
    let cli_plan_mp4 = Command::new(bin_path())
        .args([
            "media",
            "plan",
            "remux",
            mp4.to_str().unwrap(),
            "--streams",
            "1,0",
            "--max-packets",
            "3",
            "--max-rss-mib",
            "4096",
            "--metadata-delete",
            "artist",
            "--metadata",
            "title=CLI MP4 Title",
            "--stream-metadata",
            "1:title=CLI Track 1",
        ])
        .output()
        .unwrap();
    assert!(
        cli_plan_mp4.status.success(),
        "CLI plan remux MP4 failed: {}",
        String::from_utf8_lossy(&cli_plan_mp4.stderr)
    );

    let cli_mp4_out = dir.0.join("cli-mp4.mkv");
    let cli_remux_mp4 = Command::new(bin_path())
        .args([
            "media",
            "remux",
            mp4.to_str().unwrap(),
            cli_mp4_out.to_str().unwrap(),
            "--streams",
            "1,0",
            "--max-packets",
            "3",
            "--max-rss-mib",
            "4096",
            "--metadata-delete",
            "artist",
            "--metadata",
            "title=CLI MP4 Title",
            "--stream-metadata",
            "1:title=CLI Track 1",
        ])
        .output()
        .unwrap();
    assert!(
        cli_remux_mp4.status.success(),
        "CLI remux MP4 failed: {}",
        String::from_utf8_lossy(&cli_remux_mp4.stderr)
    );
    let cli_probed = fvid::media::probe(&cli_mp4_out).unwrap();
    assert_eq!(
        cli_probed.metadata.get("title").map(String::as_str),
        Some("CLI MP4 Title")
    );

    // 3. Matroska identity `plan_remux` and `remux` enforce `max_rss_bytes` in API and `--max-rss-mib` in CLI.
    let generous_rss = CopyOptions {
        max_rss_bytes: Some(4096 * 1024 * 1024),
        ..Default::default()
    };
    let mkv_plan = fvid::media::plan_remux(&mkv, &generous_rss).expect("Matroska plan_remux with RSS budget");
    assert_eq!(mkv_plan.command, "remux");
    let mkv_out = dir.0.join("mkv-remux.mkv");
    let mkv_stats = fvid::media::remux(&mkv, &mkv_out, &generous_rss).expect("Matroska remux with RSS budget");
    assert_eq!(mkv_stats.packets, 2);

    assert!(fvid::media::plan_remux(&mkv, &tight_rss).is_err());
    assert!(fvid::media::remux(&mkv, &dir.0.join("never-mkv.mkv"), &tight_rss).is_err());

    let cli_mkv_rss_err = Command::new(bin_path())
        .args([
            "media",
            "remux",
            mkv.to_str().unwrap(),
            dir.0.join("cli-never-mkv.mkv").to_str().unwrap(),
            "--max-rss-mib",
            "1",
        ])
        .output()
        .unwrap();
    assert!(!cli_mkv_rss_err.status.success());

    // 4. ADTS -> Matroska `remux` / `concat` in `fvid_media` enforce `max_rss_bytes`.
    let adts_sources = vec![adts.clone(), adts.clone()];
    assert!(fvid_media::plan_remux(&adts, &generous_rss).is_ok());
    assert!(fvid_media::plan_remux(&adts, &tight_rss).is_err());
    assert!(fvid_media::remux(&adts, &dir.0.join("adts-ok.mka"), &generous_rss).is_ok());
    assert!(fvid_media::remux(&adts, &dir.0.join("adts-err.mka"), &tight_rss).is_err());
    assert!(fvid_media::plan_concat(&adts_sources, &generous_rss).is_ok());
    assert!(fvid_media::plan_concat(&adts_sources, &tight_rss).is_err());
    assert!(fvid_media::concat(&adts_sources, &dir.0.join("adts-concat-ok.mka"), &generous_rss).is_ok());
    assert!(fvid_media::concat(&adts_sources, &dir.0.join("adts-concat-err.mka"), &tight_rss).is_err());
}
