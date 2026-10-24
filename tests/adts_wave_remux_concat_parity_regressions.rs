use fvid::media::CopyOptions;
use std::path::{Path, PathBuf};
use std::process::Command;

struct TempDir(PathBuf);
impl TempDir {
    fn new(tag: &str) -> Self {
        let dir = std::env::temp_dir().join(format!(
            "fvid-adts-wave-remux-concat-parity-{tag}-{}",
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
        .join("tests/fixtures/playback-errors/adts-wave-remux-concat-parity")
        .join(name)
}

fn bin_path() -> &'static str {
    env!("CARGO_BIN_EXE_fvid")
}

#[test]
fn adts_remux_and_plan_remux_accept_stream_zero_and_packet_budgets_across_mp4_and_matroska() {
    let dir = TempDir::new("adts-remux");
    let adts = fixture("adts-seg-a.aac");

    // 1. `fvid::media::plan_remux` and `fvid::media::remux` on ADTS with `streams: vec![0]`
    let stream_zero_opts = CopyOptions {
        streams: vec![0],
        max_packets: Some(2),
        max_packet_bytes: 4096,
        max_rss_bytes: Some(4096 * 1024 * 1024),
        ..Default::default()
    };

    let plan = fvid::media::plan_remux(&adts, &stream_zero_opts)
        .expect("fvid::media::plan_remux ADTS with streams=[0] and packet limits");
    assert_eq!(plan.command, "remux");
    assert_eq!(plan.streams.len(), 1);
    assert_eq!(plan.streams[0].index, 0);
    assert_eq!(plan.streams[0].codec, "aac");
    assert!(
        plan.steps
            .iter()
            .any(|s| s.action == "copy" && s.detail.contains("2 AAC packets")),
        "expected bounded packet count in plan: {:?}",
        plan.steps
    );
    assert!(
        plan.notes
            .iter()
            .any(|n| n.contains("encoded packet limit: 4096 bytes")),
        "expected budget note in plan: {:?}",
        plan.notes
    );

    let out_m4a = dir.0.join("stream0-limited.m4a");
    let stats_m4a = fvid::media::remux(&adts, &out_m4a, &stream_zero_opts)
        .expect("fvid::media::remux ADTS -> .m4a with streams=[0] and max_packets=2");
    assert_eq!(stats_m4a.packets, 2);
    assert_eq!(stats_m4a.backend, "fvid");

    // 2. ADTS -> Matroska `.mka` with `streams: vec![0]` + metadata mutations + packet limits
    let matroska_opts = CopyOptions {
        streams: vec![0],
        max_packets: Some(2),
        max_packet_bytes: 4096,
        max_rss_bytes: Some(4096 * 1024 * 1024),
        metadata_set: vec![("title".into(), "ADTS Stream 0 MKA".into())],
        stream_metadata_set: vec![
            (0, "title".into(), "AAC Track 0".into()),
            (0, "language".into(), "eng".into()),
        ],
        ..Default::default()
    };

    let plan_meta = fvid::media::plan_remux(&adts, &matroska_opts)
        .expect("fvid::media::plan_remux ADTS with streams=[0] and metadata mutations");
    assert!(plan_meta.steps.iter().any(|s| s.action == "metadata"));

    let out_mka = dir.0.join("stream0-meta.mka");
    let stats_mka = fvid::media::remux(&adts, &out_mka, &matroska_opts)
        .expect("fvid::media::remux ADTS -> .mka with streams=[0] and metadata mutations");
    assert_eq!(stats_mka.packets, 2);
    assert_eq!(stats_mka.backend, "fvid");

    let probed = fvid::media::probe(&out_mka).unwrap();
    assert_eq!(
        probed.metadata.get("title").map(String::as_str),
        Some("ADTS Stream 0 MKA")
    );
    assert_eq!(
        probed.streams[0].metadata.get("title").map(String::as_str),
        Some("AAC Track 0")
    );
    assert_eq!(
        probed.streams[0]
            .metadata
            .get("language")
            .map(String::as_str),
        Some("eng")
    );

    // 3. CLI `fvid media plan remux` and `fvid media remux` with `--streams 0`
    let cli_plan = Command::new(bin_path())
        .args([
            "media",
            "plan",
            "remux",
            adts.to_str().unwrap(),
            "--streams",
            "0",
            "--max-packets",
            "2",
        ])
        .output()
        .unwrap();
    assert!(
        cli_plan.status.success(),
        "cli plan remux ADTS --streams 0 failed: {}",
        String::from_utf8_lossy(&cli_plan.stderr)
    );

    let cli_out_m4a = dir.0.join("cli-stream0.m4a");
    let cli_remux = Command::new(bin_path())
        .args([
            "media",
            "remux",
            adts.to_str().unwrap(),
            cli_out_m4a.to_str().unwrap(),
            "--streams",
            "0",
            "--max-packets",
            "2",
        ])
        .output()
        .unwrap();
    assert!(
        cli_remux.status.success(),
        "cli remux ADTS -> .m4a --streams 0 failed: {}",
        String::from_utf8_lossy(&cli_remux.stderr)
    );

    // 4. Out-of-range stream index `streams: vec![1]` is cleanly refused on both plan and execution
    let bad_stream_opts = CopyOptions {
        streams: vec![1],
        ..Default::default()
    };
    let err_plan = fvid::media::plan_remux(&adts, &bad_stream_opts)
        .expect_err("ADTS plan_remux must reject stream 1");
    assert_eq!(err_plan, "ADTS has only stream 0");

    let bad_out = dir.0.join("bad-stream1.m4a");
    let err_remux = fvid::media::remux(&adts, &bad_out, &bad_stream_opts)
        .expect_err("ADTS remux must reject stream 1");
    assert_eq!(err_remux, "ADTS has only stream 0");
    assert!(!bad_out.exists());
}

#[test]
fn wave_and_adts_concat_to_matroska_and_wav_preserve_cli_and_api_option_parity() {
    let dir = TempDir::new("wave-adts-concat");
    let wave_a = fixture("wave-seg-a.wav");
    let wave_b = fixture("wave-seg-b.wav");
    let adts_a = fixture("adts-seg-a.aac");
    let adts_b = fixture("adts-seg-b.aac");

    // 1. CLI `fvid media concat out.mka wave_a.wav wave_b.wav` with default options must succeed
    let cli_wave_mka_default = dir.0.join("cli-wave-default.mka");
    let out_default = Command::new(bin_path())
        .args([
            "media",
            "concat",
            cli_wave_mka_default.to_str().unwrap(),
            wave_a.to_str().unwrap(),
            wave_b.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(
        out_default.status.success(),
        "cli concat WAVE -> .mka default failed: {}",
        String::from_utf8_lossy(&out_default.stderr)
    );
    let probed_default = fvid::media::probe(&cli_wave_mka_default).unwrap();
    assert_eq!(probed_default.streams.len(), 1);
    assert_eq!(probed_default.streams[0].codec, "pcm_f32le");

    // 1b. CLI `fvid media concat out.wav wave_a.wav wave_b.wav --max-packets 1` must route to
    // lossless WAVE block copy (`fvid::media::concat`), NOT decoded float32 PCM concat (`try_mp4_matroska_concat`)!
    // And `fvid media concat out.m4a adts_a.aac adts_b.aac --streams 0` via API & CLI must succeed!
    let api_adts_m4a = dir.0.join("api-adts-stream0.m4a");
    let adts_m4a_opts = CopyOptions {
        streams: vec![0],
        max_packets: Some(4),
        ..Default::default()
    };
    let stats_adts_m4a =
        fvid::media::concat(&[adts_a.clone(), adts_b.clone()], &api_adts_m4a, &adts_m4a_opts)
            .expect("fvid::media::concat ADTS -> .m4a with streams=[0] and max_packets=4");
    assert_eq!(stats_adts_m4a.packets, 4);
    assert_eq!(stats_adts_m4a.backend, "fvid");

    let api_adts_mka = dir.0.join("api-adts-stream0.mka");
    let adts_mka_opts = CopyOptions {
        streams: vec![0],
        max_packets: Some(4),
        metadata_set: vec![("title".into(), "API ADTS Concat MKA".into())],
        stream_metadata_set: vec![(0, "language".into(), "spa".into())],
        ..Default::default()
    };
    let stats_adts_mka =
        fvid::media::concat(&[adts_a.clone(), adts_b.clone()], &api_adts_mka, &adts_mka_opts)
            .expect("fvid::media::concat ADTS -> .mka with streams=[0], max_packets=4, and metadata");
    assert_eq!(stats_adts_mka.packets, 4);
    assert_eq!(stats_adts_mka.backend, "fvid");
    let probed_api_adts = fvid::media::probe(&api_adts_mka).unwrap();
    assert_eq!(
        probed_api_adts.metadata.get("title").map(String::as_str),
        Some("API ADTS Concat MKA")
    );
    assert_eq!(
        probed_api_adts.streams[0]
            .metadata
            .get("language")
            .map(String::as_str),
        Some("spa")
    );

    let bad_adts_concat_opts = CopyOptions {
        streams: vec![1],
        ..Default::default()
    };
    let bad_adts_concat_mka = dir.0.join("bad-adts-concat.mka");
    let err_adts_concat_mka = fvid::media::concat(
        &[adts_a.clone(), adts_b.clone()],
        &bad_adts_concat_mka,
        &bad_adts_concat_opts,
    )
    .expect_err("ADTS concat -> .mka must reject stream 1");
    assert_eq!(err_adts_concat_mka, "ADTS has only stream 0");
    assert!(!bad_adts_concat_mka.exists());

    // 2. CLI `fvid media concat out.mka wave_a.wav wave_b.wav --streams 0 --metadata ... --stream-metadata ...`
    let cli_wave_mka_meta = dir.0.join("cli-wave-meta.mka");
    let out_meta = Command::new(bin_path())
        .args([
            "media",
            "concat",
            cli_wave_mka_meta.to_str().unwrap(),
            wave_a.to_str().unwrap(),
            wave_b.to_str().unwrap(),
            "--streams",
            "0",
            "--metadata",
            "title=Joined WAVE Matroska",
            "--stream-metadata",
            "0:title=PCM Track 0",
            "--stream-metadata",
            "0:language=jpn",
        ])
        .output()
        .unwrap();
    assert!(
        out_meta.status.success(),
        "cli concat WAVE -> .mka with metadata failed: {}",
        String::from_utf8_lossy(&out_meta.stderr)
    );
    let probed_meta = fvid::media::probe(&cli_wave_mka_meta).unwrap();
    assert_eq!(
        probed_meta.metadata.get("title").map(String::as_str),
        Some("Joined WAVE Matroska")
    );
    assert_eq!(
        probed_meta.streams[0]
            .metadata
            .get("title")
            .map(String::as_str),
        Some("PCM Track 0")
    );
    assert_eq!(
        probed_meta.streams[0]
            .metadata
            .get("language")
            .map(String::as_str),
        Some("jpn")
    );

    // 3. CLI `fvid media concat out.mka adts_a.aac adts_b.aac --streams 0 --max-packets 4 --metadata ...`
    // (previously `try_mp4_matroska_concat` intercepted ADTS -> .mka, got `None` from `try_concat_mp4_matroska_with_options`,
    // and `try_native_audio_concat` called `fvid::media::concat` which routed ADTS `.mka` to `fvid_media::concat`!)
    let cli_adts_mka = dir.0.join("cli-adts-limited.mka");
    let out_adts = Command::new(bin_path())
        .args([
            "media",
            "concat",
            cli_adts_mka.to_str().unwrap(),
            adts_a.to_str().unwrap(),
            adts_b.to_str().unwrap(),
            "--streams",
            "0",
            "--max-packets",
            "4",
            "--metadata",
            "title=ADTS Concat Limited",
            "--stream-metadata",
            "0:language=ita",
        ])
        .output()
        .unwrap();
    assert!(
        out_adts.status.success(),
        "cli concat ADTS -> .mka with --streams 0 and --max-packets 4 failed: {}",
        String::from_utf8_lossy(&out_adts.stderr)
    );
    let stats_json: serde_json::Value =
        serde_json::from_slice(&out_adts.stdout).expect("valid JSON output");
    assert_eq!(stats_json["packets"], 4);
    assert_eq!(stats_json["backend"], "fvid");
    let probed_adts = fvid::media::probe(&cli_adts_mka).unwrap();
    assert_eq!(
        probed_adts.metadata.get("title").map(String::as_str),
        Some("ADTS Concat Limited")
    );
    assert_eq!(
        probed_adts.streams[0]
            .metadata
            .get("language")
            .map(String::as_str),
        Some("ita")
    );
}
