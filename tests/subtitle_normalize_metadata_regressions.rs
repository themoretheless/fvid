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
        .join("tests/fixtures/playback-errors/subtitle-normalize-metadata")
}

fn bin_path() -> &'static str {
    env!("CARGO_BIN_EXE_fvid")
}

#[test]
fn convert_subtitles_and_normalize_loudness_support_matroska_metadata_and_cue_limits() {
    let fix = fixture_dir();
    let srt = fix.join("cues-two.srt");
    let ass = fix.join("cues-two.ass");
    let wav = fix.join("tone-1s-48k.wav");
    let tmp = TempDir::new("sub-norm-meta");

    // 1. SRT and ASS multi-cue per-packet byte limit must reject oversized second cue without creating output
    let srt_pkt_out = tmp.0.join("srt-pkt-err.mkv");
    let srt_pkt_err = fvid::native_subtitle::try_convert_controlled(
        &srt,
        &srt_pkt_out,
        &[],
        50, // First cue is ~46 bytes, second cue is ~98 bytes
        None,
        None,
        None,
        None,
    );
    assert!(
        srt_pkt_err.is_err(),
        "try_convert_controlled on 2-cue SRT must reject cue exceeding max_packet_bytes=50"
    );
    assert!(!srt_pkt_out.exists());

    let ass_pkt_out = tmp.0.join("ass-pkt-err.mkv");
    let ass_pkt_err = fvid::native_subtitle::try_convert_controlled(
        &ass,
        &ass_pkt_out,
        &[],
        50,
        None,
        None,
        None,
        None,
    );
    assert!(
        ass_pkt_err.is_err(),
        "try_convert_controlled on 2-cue ASS must reject cue exceeding max_packet_bytes=50"
    );
    assert!(!ass_pkt_out.exists());

    // 2. convert-subtitles with container and stream metadata mutations on SRT -> MKV, ASS -> MKV, and MKV -> MKV
    let srt_meta_out = tmp.0.join("srt-meta.mkv");
    let mut sub_meta_controls = fvid::media::CopyOptions::default();
    sub_meta_controls.metadata_set = vec![
        ("title".into(), "SRT Container Title".into()),
        ("SUB_SOURCE".into(), "synthetic-srt".into()),
    ];
    sub_meta_controls.stream_metadata_set = vec![
        (0, "title".into(), "English Subtitles".into()),
        (0, "language".into(), "eng".into()),
        (0, "SUB_NOTE".into(), "track-zero".into()),
    ];
    let srt_stats = fvid::media::convert_subtitles_with_controls(
        &srt,
        &srt_meta_out,
        &Default::default(),
        &sub_meta_controls,
    )
    .expect("convert_subtitles_with_controls on SRT with metadata failed");
    assert_eq!(srt_stats.cues, 2);

    let srt_probed = fvid::media::probe(&srt_meta_out).unwrap();
    assert_eq!(
        srt_probed.metadata.get("title").map(String::as_str),
        Some("SRT Container Title")
    );
    assert_eq!(
        srt_probed.metadata.get("SUB_SOURCE").map(String::as_str),
        Some("synthetic-srt")
    );
    assert_eq!(
        srt_probed.streams[0].metadata.get("title").map(String::as_str),
        Some("English Subtitles")
    );
    assert_eq!(
        srt_probed.streams[0]
            .metadata
            .get("language")
            .map(String::as_str),
        Some("eng")
    );
    assert_eq!(
        srt_probed.streams[0]
            .metadata
            .get("SUB_NOTE")
            .map(String::as_str),
        Some("track-zero")
    );

    // Convert MKV -> MKV preserving and mutating/deleting metadata via CLI
    let mkv_re_out = tmp.0.join("mkv-re-meta.mkv");
    let cli_sub = Command::new(bin_path())
        .args([
            "media",
            "convert-subtitles",
            srt_meta_out.to_str().unwrap(),
            mkv_re_out.to_str().unwrap(),
            "--metadata-delete",
            "SUB_SOURCE",
            "--metadata",
            "title=Updated Subtitle Container",
            "--stream-metadata-delete",
            "0:SUB_NOTE",
            "--stream-metadata",
            "0:language=fra",
            "-q",
        ])
        .output()
        .unwrap();
    assert!(
        cli_sub.status.success(),
        "CLI convert-subtitles with metadata failed: {}",
        String::from_utf8_lossy(&cli_sub.stderr)
    );
    let mkv_re_probed = fvid::media::probe(&mkv_re_out).unwrap();
    assert_eq!(
        mkv_re_probed.metadata.get("title").map(String::as_str),
        Some("Updated Subtitle Container")
    );
    assert!(!mkv_re_probed.metadata.contains_key("SUB_SOURCE"));
    assert_eq!(
        mkv_re_probed.streams[0]
            .metadata
            .get("title")
            .map(String::as_str),
        Some("English Subtitles")
    );
    assert_eq!(
        mkv_re_probed.streams[0]
            .metadata
            .get("language")
            .map(String::as_str),
        Some("fra")
    );
    assert!(!mkv_re_probed.streams[0].metadata.contains_key("SUB_NOTE"));

    // Invalid stream metadata index on convert-subtitles must fail
    let bad_sub_out = tmp.0.join("bad-sub.mkv");
    let cli_bad_sub = Command::new(bin_path())
        .args([
            "media",
            "convert-subtitles",
            srt.to_str().unwrap(),
            bad_sub_out.to_str().unwrap(),
            "--stream-metadata",
            "1:title=Bad",
        ])
        .output()
        .unwrap();
    assert!(
        !cli_bad_sub.status.success(),
        "CLI convert-subtitles must reject stream metadata index != 0"
    );
    assert!(!bad_sub_out.exists());

    // 3. normalize-loudness and plan normalize-loudness to .mka and .wav with container & stream metadata
    let mut norm_mka_opts = fvid::media::CopyOptions::default();
    norm_mka_opts.metadata_set = vec![
        ("title".into(), "Normalized Audio Container".into()),
        ("NORM_TAG".into(), "mka-custom".into()),
    ];
    norm_mka_opts.stream_metadata_set = vec![
        (0, "title".into(), "Normalized Track 0".into()),
        (0, "language".into(), "deu".into()),
        (0, "TRACK_GAIN_NOTE".into(), "constant-gain".into()),
    ];
    let target = fvid::media::NormalizeTarget::default();
    let norm_plan = fvid::media::plan_normalize_loudness(&wav, &norm_mka_opts, None, target)
        .expect("plan_normalize_loudness with Matroska container and stream metadata failed");
    assert_eq!(norm_plan.command, "normalize-loudness");
    assert!(
        norm_plan.steps.iter().any(|s| s.action == "metadata"),
        "expected metadata step in normalize-loudness plan"
    );

    let norm_mka_out = tmp.0.join("normalized.mka");
    let cli_norm_plan = Command::new(bin_path())
        .args([
            "media",
            "plan",
            "normalize-loudness",
            wav.to_str().unwrap(),
            norm_mka_out.to_str().unwrap(),
            "--metadata",
            "title=Normalized Audio Container",
            "--metadata",
            "NORM_TAG=mka-custom",
            "--stream-metadata",
            "0:title=Normalized Track 0",
            "--stream-metadata",
            "0:language=deu",
            "--stream-metadata",
            "0:TRACK_GAIN_NOTE=constant-gain",
        ])
        .output()
        .unwrap();
    assert!(
        cli_norm_plan.status.success(),
        "CLI plan normalize-loudness to .mka with metadata failed: {}",
        String::from_utf8_lossy(&cli_norm_plan.stderr)
    );

    let cli_norm_run = Command::new(bin_path())
        .args([
            "media",
            "normalize-loudness",
            wav.to_str().unwrap(),
            norm_mka_out.to_str().unwrap(),
            "--metadata",
            "title=Normalized Audio Container",
            "--metadata",
            "NORM_TAG=mka-custom",
            "--stream-metadata",
            "0:title=Normalized Track 0",
            "--stream-metadata",
            "0:language=deu",
            "--stream-metadata",
            "0:TRACK_GAIN_NOTE=constant-gain",
            "-q",
        ])
        .output()
        .unwrap();
    assert!(
        cli_norm_run.status.success(),
        "CLI normalize-loudness to .mka with metadata failed: {}",
        String::from_utf8_lossy(&cli_norm_run.stderr)
    );
    let norm_probed = fvid::media::probe(&norm_mka_out).unwrap();
    assert_eq!(
        norm_probed.metadata.get("title").map(String::as_str),
        Some("Normalized Audio Container")
    );
    assert_eq!(
        norm_probed.metadata.get("NORM_TAG").map(String::as_str),
        Some("mka-custom")
    );
    assert_eq!(
        norm_probed.streams[0]
            .metadata
            .get("title")
            .map(String::as_str),
        Some("Normalized Track 0")
    );
    assert_eq!(
        norm_probed.streams[0]
            .metadata
            .get("language")
            .map(String::as_str),
        Some("deu")
    );
    assert_eq!(
        norm_probed.streams[0]
            .metadata
            .get("TRACK_GAIN_NOTE")
            .map(String::as_str),
        Some("constant-gain")
    );

    // WAVE output on normalize-loudness must still reject stream metadata and non-INFO container keys
    let bad_wav_out = tmp.0.join("bad-norm.wav");
    let cli_bad_wav = Command::new(bin_path())
        .args([
            "media",
            "normalize-loudness",
            wav.to_str().unwrap(),
            bad_wav_out.to_str().unwrap(),
            "--stream-metadata",
            "0:title=Unsupported On Wave",
        ])
        .output()
        .unwrap();
    assert!(
        !cli_bad_wav.status.success(),
        "CLI normalize-loudness to .wav must reject --stream-metadata"
    );
    assert!(!bad_wav_out.exists());

    // Valid WAVE INFO edits on normalize-loudness to .wav continue to work
    let ok_wav_out = tmp.0.join("ok-norm.wav");
    let cli_ok_wav = Command::new(bin_path())
        .args([
            "media",
            "normalize-loudness",
            wav.to_str().unwrap(),
            ok_wav_out.to_str().unwrap(),
            "--metadata-delete",
            "artist",
            "--metadata",
            "title=Wave Normalized Title",
            "-q",
        ])
        .output()
        .unwrap();
    assert!(
        cli_ok_wav.status.success(),
        "CLI normalize-loudness to .wav with INFO edits failed: {}",
        String::from_utf8_lossy(&cli_ok_wav.stderr)
    );
    let wav_probed = fvid::media::probe(&ok_wav_out).unwrap();
    assert_eq!(
        wav_probed.metadata.get("title").map(String::as_str),
        Some("Wave Normalized Title")
    );
    assert!(!wav_probed.metadata.contains_key("artist"));
}
