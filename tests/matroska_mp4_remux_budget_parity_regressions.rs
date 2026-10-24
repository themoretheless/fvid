use fvid::media::CopyOptions;
use std::path::{Path, PathBuf};
use std::process::Command;

struct TempDir(PathBuf);
impl TempDir {
    fn new(tag: &str) -> Self {
        let dir = std::env::temp_dir().join(format!(
            "fvid-matroska-mp4-remux-budget-parity-{tag}-{}",
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
        .join("tests/fixtures/playback-errors/matroska-mp4-remux-budget-parity")
        .join(name)
}

fn bin_path() -> &'static str {
    env!("CARGO_BIN_EXE_fvid")
}

#[test]
fn matroska_identity_plan_remux_and_owned_plan_remux_respect_custom_packet_and_rss_budgets() {
    let dir = TempDir::new("matroska-copy");
    let mka = fixture("matroska-aac-48k.mka");

    // 1. Custom `max_packet_bytes` and `max_rss_bytes` on Matroska identity copy (`plan_remux` & `remux`)
    let valid_opts = CopyOptions {
        max_packet_bytes: 256,
        max_rss_bytes: Some(4096 * 1024 * 1024),
        ..Default::default()
    };

    let plan = fvid::media::plan_remux(&mka, &valid_opts)
        .expect("fvid::media::plan_remux Matroska with custom max_packet_bytes and max_rss_bytes");
    assert_eq!(plan.command, "remux");
    assert_eq!(plan.streams.len(), 1);
    assert_eq!(plan.streams[0].index, 0);
    assert_eq!(plan.streams[0].codec, "aac");
    assert!(
        plan.steps
            .iter()
            .any(|s| s.action == "budget" && s.detail.contains("256 bytes")),
        "expected packet budget step in Matroska remux plan: {:?}",
        plan.steps
    );
    assert!(
        plan.notes
            .iter()
            .any(|n| n.contains("encoded packet limit: 256 bytes") && n.contains("RSS limit:")),
        "expected budget note in Matroska remux plan: {:?}",
        plan.notes
    );

    let owned_plan = fvid_media::plan_remux(&mka, &valid_opts)
        .expect("fvid_media::plan_remux Matroska with custom max_packet_bytes and max_rss_bytes");
    assert_eq!(owned_plan.command, "remux");
    assert_eq!(owned_plan.streams.len(), 1);
    assert_eq!(owned_plan.streams[0].codec, "aac");

    let out_mka = dir.0.join("copied.mka");
    let stats = fvid::media::remux(&mka, &out_mka, &valid_opts)
        .expect("fvid::media::remux Matroska with custom max_packet_bytes and max_rss_bytes");
    assert_eq!(stats.packets, 3);
    assert_eq!(stats.backend, "fvid");

    // 2. Tight `max_packet_bytes` (e.g. 16 bytes < 64-byte packet) must be rejected by both
    // `fvid::media::plan_remux` and `fvid_media::plan_remux` (and `probe_webm_with_packet_limit`).
    let tight_opts = CopyOptions {
        max_packet_bytes: 16,
        ..Default::default()
    };
    let err_top = fvid::media::plan_remux(&mka, &tight_opts)
        .expect_err("fvid::media::plan_remux must reject packet larger than max_packet_bytes");
    assert!(
        err_top.contains("packet") || err_top.contains("limit") || err_top.contains("exceeds"),
        "unexpected error for tight max_packet_bytes: {err_top}"
    );
    let err_owned = fvid_media::plan_remux(&mka, &tight_opts)
        .expect_err("fvid_media::plan_remux must reject packet larger than max_packet_bytes");
    assert!(
        err_owned.contains("packet") || err_owned.contains("limit") || err_owned.contains("exceeds"),
        "unexpected error for tight max_packet_bytes in fvid_media: {err_owned}"
    );

    // 3. CLI `fvid media plan remux` and `fvid media remux` with `--max-packet-bytes` and `--max-rss-mib`
    let cli_plan = Command::new(bin_path())
        .args([
            "media",
            "plan",
            "remux",
            mka.to_str().unwrap(),
            "--max-packet-bytes",
            "256",
            "--max-rss-mib",
            "4096",
        ])
        .output()
        .unwrap();
    assert!(
        cli_plan.status.success(),
        "cli plan remux Matroska failed: {}",
        String::from_utf8_lossy(&cli_plan.stderr)
    );
}

#[test]
fn mp4_faststart_and_mp4_to_matroska_remux_unify_backend_and_rss_budgets() {
    let dir = TempDir::new("mp4-remux");
    let mp4_aac = fixture("mp4-aac-48k.m4a");
    let mp4_pcm = fixture("mp4-pcm-48k.m4a");

    // 1. MP4 -> Matroska (`.mka`) with `--streams 0`, `--max-packets 2`, `--max-packet-bytes 4096`,
    // `--max-rss-mib 4096`, and Matroska metadata mutations must report `backend: "fvid"` and
    // `fvid_payload_copies: 0` (matching default MP4 -> Matroska remux).
    let mp4_mka_opts = CopyOptions {
        streams: vec![0],
        max_packets: Some(2),
        max_packet_bytes: 4096,
        max_rss_bytes: Some(4096 * 1024 * 1024),
        metadata_set: vec![("title".into(), "MP4 AAC Custom MKA".into())],
        stream_metadata_set: vec![
            (0, "title".into(), "AAC Stream 0".into()),
            (0, "language".into(), "fra".into()),
        ],
        ..Default::default()
    };

    let plan_mka = fvid::media::plan_remux(&mp4_aac, &mp4_mka_opts)
        .expect("fvid::media::plan_remux MP4 -> Matroska with custom options");
    assert_eq!(plan_mka.command, "remux");
    assert_eq!(plan_mka.streams.len(), 1);
    assert_eq!(plan_mka.streams[0].codec, "aac");

    let out_mka = dir.0.join("mp4-custom.mka");
    let stats_mka = fvid::media::remux(&mp4_aac, &out_mka, &mp4_mka_opts)
        .expect("fvid::media::remux MP4 -> .mka with custom options");
    assert_eq!(stats_mka.packets, 2);
    assert_eq!(stats_mka.backend, "fvid");
    assert_eq!(stats_mka.fvid_payload_copies, 0);

    let probed = fvid::media::probe(&out_mka).unwrap();
    assert_eq!(
        probed.metadata.get("title").map(String::as_str),
        Some("MP4 AAC Custom MKA")
    );
    assert_eq!(
        probed.streams[0].metadata.get("title").map(String::as_str),
        Some("AAC Stream 0")
    );
    assert_eq!(
        probed.streams[0]
            .metadata
            .get("language")
            .map(String::as_str),
        Some("fra")
    );

    // 2. MP4 fast-start relocation (`.mp4` / `.m4a`) with `max_rss_bytes` must succeed on BOTH
    // `mp4-aac-48k.m4a` and `mp4-pcm-48k.m4a`, and `plan_remux` on `mp4-pcm-48k.m4a` with `max_rss_bytes`
    // must plan native MP4 fast-start relocation with budget notes.
    let rss_only_opts = CopyOptions {
        max_rss_bytes: Some(4096 * 1024 * 1024),
        ..Default::default()
    };

    let plan_pcm_faststart = fvid::media::plan_remux(&mp4_pcm, &rss_only_opts)
        .expect("fvid::media::plan_remux MP4 PCM with max_rss_bytes");
    assert_eq!(plan_pcm_faststart.command, "remux");
    assert_eq!(plan_pcm_faststart.streams[0].codec, "pcm_s16le");
    assert!(
        plan_pcm_faststart
            .notes
            .iter()
            .any(|n| n.contains("RSS limit:")),
        "expected RSS limit note in MP4 fast-start plan: {:?}",
        plan_pcm_faststart.notes
    );

    let out_aac_m4a = dir.0.join("relocated-aac.m4a");
    let stats_aac_m4a = fvid::media::remux(&mp4_aac, &out_aac_m4a, &rss_only_opts)
        .expect("fvid::media::remux MP4 AAC -> .m4a with max_rss_bytes");
    assert_eq!(stats_aac_m4a.backend, "fvid");
    assert!(stats_aac_m4a.payload_bytes > 0);

    let out_pcm_m4a = dir.0.join("relocated-pcm.m4a");
    let stats_pcm_m4a = fvid::media::remux(&mp4_pcm, &out_pcm_m4a, &rss_only_opts)
        .expect("fvid::media::remux MP4 PCM -> .m4a with max_rss_bytes");
    assert_eq!(stats_pcm_m4a.backend, "fvid");
    assert!(stats_pcm_m4a.payload_bytes > 0);

    // 3. CLI `fvid media remux` on MP4 -> `.m4a` with `--max-rss-mib 4096` and MP4 -> `.mka` with
    // `--streams 0 --max-packets 2`
    let cli_out_m4a = dir.0.join("cli-relocated.m4a");
    let cli_faststart = Command::new(bin_path())
        .args([
            "media",
            "remux",
            mp4_pcm.to_str().unwrap(),
            cli_out_m4a.to_str().unwrap(),
            "--max-rss-mib",
            "4096",
        ])
        .output()
        .unwrap();
    assert!(
        cli_faststart.status.success(),
        "cli remux MP4 -> .m4a --max-rss-mib 4096 failed: {}",
        String::from_utf8_lossy(&cli_faststart.stderr)
    );

    let cli_out_mka = dir.0.join("cli-mp4.mka");
    let cli_mka = Command::new(bin_path())
        .args([
            "media",
            "remux",
            mp4_aac.to_str().unwrap(),
            cli_out_mka.to_str().unwrap(),
            "--streams",
            "0",
            "--max-packets",
            "2",
        ])
        .output()
        .unwrap();
    assert!(
        cli_mka.status.success(),
        "cli remux MP4 -> .mka --streams 0 --max-packets 2 failed: {}",
        String::from_utf8_lossy(&cli_mka.stderr)
    );
    let cli_json: serde_json::Value = serde_json::from_slice(&cli_mka.stdout).unwrap();
    assert_eq!(cli_json["backend"], "fvid");
    assert_eq!(cli_json["packets"], 2);
}
