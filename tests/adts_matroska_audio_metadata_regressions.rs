use fvid::media::{AudioDecodeTransform, CopyOptions};
use std::path::{Path, PathBuf};
use std::process::Command;

struct TempDir(PathBuf);
impl TempDir {
    fn new(tag: &str) -> Self {
        let dir = std::env::temp_dir().join(format!(
            "fvid-adts-matroska-audio-metadata-{tag}-{}",
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
        .join("tests/fixtures/playback-errors/adts-matroska-audio-metadata")
        .join(name)
}

fn bin_path() -> &'static str {
    env!("CARGO_BIN_EXE_fvid")
}

#[test]
fn adts_remux_concat_and_matroska_pcm_decode_trim_metadata_mutations_work_in_api_and_cli() {
    let dir = TempDir::new("all");
    let adts_a = fixture("adts-seg-a-48k.aac");
    let adts_b = fixture("adts-seg-b-48k.aac");
    let wav = fixture("pcm-source-48k.wav");

    // 1. ADTS -> Matroska `remux` and `plan_remux` via `fvid::media` and `fvid_media` with container & stream metadata mutations.
    let adts_meta_opts = CopyOptions {
        metadata_delete: vec!["comment".into()],
        metadata_set: vec![
            ("title".into(), "ADTS Matroska Remux".into()),
            ("CUSTOM_CONTAINER_TAG".into(), "ContainerValue1".into()),
        ],
        stream_metadata_delete: vec![(0, "CUSTOM_DROP".into())],
        stream_metadata_set: vec![
            (0, "title".into(), "AAC Track Zero".into()),
            (0, "language".into(), "fra".into()),
            (0, "CUSTOM_STREAM_TAG".into(), "StreamValue1".into()),
        ],
        max_rss_bytes: Some(4096 * 1024 * 1024),
        ..Default::default()
    };

    let remux_plan = fvid::media::plan_remux(&adts_a, &adts_meta_opts)
        .expect("fvid::media::plan_remux ADTS with metadata edits");
    assert_eq!(remux_plan.command, "remux");
    assert!(remux_plan.steps.iter().any(|s| s.action == "metadata"));

    let owned_remux_plan = fvid_media::plan_remux(&adts_a, &adts_meta_opts)
        .expect("fvid_media::plan_remux ADTS with metadata edits");
    assert_eq!(owned_remux_plan.command, "remux");

    let remux_mka = dir.0.join("adts-remux.mka");
    let remux_stats = fvid::media::remux(&adts_a, &remux_mka, &adts_meta_opts)
        .expect("fvid::media::remux ADTS -> .mka with metadata edits");
    assert_eq!(remux_stats.packets, 3);
    let remux_probed = fvid::media::probe(&remux_mka).unwrap();
    assert_eq!(
        remux_probed.metadata.get("title").map(String::as_str),
        Some("ADTS Matroska Remux")
    );
    assert_eq!(
        remux_probed
            .metadata
            .get("CUSTOM_CONTAINER_TAG")
            .map(String::as_str),
        Some("ContainerValue1")
    );
    assert!(!remux_probed.metadata.contains_key("comment"));
    assert_eq!(
        remux_probed.streams[0]
            .metadata
            .get("title")
            .map(String::as_str),
        Some("AAC Track Zero")
    );
    assert_eq!(
        remux_probed.streams[0]
            .metadata
            .get("language")
            .map(String::as_str),
        Some("fra")
    );
    assert_eq!(
        remux_probed.streams[0]
            .metadata
            .get("CUSTOM_STREAM_TAG")
            .map(String::as_str),
        Some("StreamValue1")
    );
    assert!(!remux_probed.streams[0].metadata.contains_key("CUSTOM_DROP"));

    // Also verify `fvid_media::remux` directly.
    let owned_remux_mka = dir.0.join("owned-adts-remux.mka");
    let owned_remux_stats = fvid_media::remux(&adts_a, &owned_remux_mka, &adts_meta_opts)
        .expect("fvid_media::remux ADTS -> .mka with metadata edits");
    assert_eq!(owned_remux_stats.packets, 3);
    let owned_remux_probed = fvid::media::probe(&owned_remux_mka).unwrap();
    assert_eq!(
        owned_remux_probed.metadata.get("title").map(String::as_str),
        Some("ADTS Matroska Remux")
    );
    assert_eq!(
        owned_remux_probed.streams[0]
            .metadata
            .get("title")
            .map(String::as_str),
        Some("AAC Track Zero")
    );

    // 2. ADTS -> Matroska `concat` and `plan_concat` via `fvid::media`, `fvid_media`, and CLI.
    let adts_sources = vec![adts_a.clone(), adts_b.clone()];
    let concat_plan = fvid::media::plan_concat(&adts_sources, &adts_meta_opts)
        .expect("fvid::media::plan_concat ADTS with metadata edits");
    assert_eq!(concat_plan.command, "concat");
    assert!(concat_plan.steps.iter().any(|s| s.action == "metadata"));

    let owned_concat_plan = fvid_media::plan_concat(&adts_sources, &adts_meta_opts)
        .expect("fvid_media::plan_concat ADTS with metadata edits");
    assert_eq!(owned_concat_plan.command, "concat");

    let concat_mka = dir.0.join("adts-concat.mka");
    let concat_stats = fvid::media::concat(&adts_sources, &concat_mka, &adts_meta_opts)
        .expect("fvid::media::concat ADTS -> .mka with metadata edits");
    assert_eq!(concat_stats.packets, 6);
    assert_eq!(concat_stats.segments, 2);
    let concat_probed = fvid::media::probe(&concat_mka).unwrap();
    assert_eq!(
        concat_probed.metadata.get("title").map(String::as_str),
        Some("ADTS Matroska Remux")
    );
    assert_eq!(
        concat_probed.streams[0]
            .metadata
            .get("title")
            .map(String::as_str),
        Some("AAC Track Zero")
    );
    assert_eq!(
        concat_probed.streams[0]
            .metadata
            .get("language")
            .map(String::as_str),
        Some("fra")
    );
    assert_eq!(
        concat_probed.streams[0]
            .metadata
            .get("CUSTOM_STREAM_TAG")
            .map(String::as_str),
        Some("StreamValue1")
    );

    let cli_concat_plan = Command::new(bin_path())
        .args([
            "media",
            "plan",
            "concat",
            adts_a.to_str().unwrap(),
            adts_b.to_str().unwrap(),
            "--metadata",
            "title=CLI ADTS Concat",
            "--stream-metadata",
            "0:title=CLI AAC Track",
            "--stream-metadata",
            "0:language=deu",
        ])
        .output()
        .unwrap();
    assert!(
        cli_concat_plan.status.success(),
        "cli plan concat failed: {}",
        String::from_utf8_lossy(&cli_concat_plan.stderr)
    );

    let cli_concat_mka = dir.0.join("cli-adts-concat.mka");
    let cli_concat_out = Command::new(bin_path())
        .args([
            "media",
            "concat",
            cli_concat_mka.to_str().unwrap(),
            adts_a.to_str().unwrap(),
            adts_b.to_str().unwrap(),
            "--metadata",
            "title=CLI ADTS Concat",
            "--metadata",
            "CUSTOM_TAG=CLIConcatTag",
            "--stream-metadata",
            "0:title=CLI AAC Track",
            "--stream-metadata",
            "0:language=deu",
            "--stream-metadata",
            "0:CUSTOM_STREAM=CLIStreamVal",
        ])
        .output()
        .unwrap();
    assert!(
        cli_concat_out.status.success(),
        "cli concat failed: {}",
        String::from_utf8_lossy(&cli_concat_out.stderr)
    );
    let cli_concat_probed = fvid::media::probe(&cli_concat_mka).unwrap();
    assert_eq!(
        cli_concat_probed.metadata.get("title").map(String::as_str),
        Some("CLI ADTS Concat")
    );
    assert_eq!(
        cli_concat_probed
            .metadata
            .get("CUSTOM_TAG")
            .map(String::as_str),
        Some("CLIConcatTag")
    );
    assert_eq!(
        cli_concat_probed.streams[0]
            .metadata
            .get("title")
            .map(String::as_str),
        Some("CLI AAC Track")
    );
    assert_eq!(
        cli_concat_probed.streams[0]
            .metadata
            .get("language")
            .map(String::as_str),
        Some("deu")
    );
    assert_eq!(
        cli_concat_probed.streams[0]
            .metadata
            .get("CUSTOM_STREAM")
            .map(String::as_str),
        Some("CLIStreamVal")
    );

    // 3. `decode-audio` and `trim` to Matroska (`.mka` / `.mkv`) with container & stream metadata mutations.
    let pcm_mka_opts = CopyOptions {
        metadata_delete: vec!["artist".into()],
        metadata_set: vec![
            ("title".into(), "Decoded Matroska PCM".into()),
            ("CUSTOM_DECODE_TAG".into(), "DecodeTagVal".into()),
        ],
        stream_metadata_delete: vec![(0, "DROP_ME".into())],
        stream_metadata_set: vec![
            (0, "title".into(), "Decoded PCM Track".into()),
            (0, "language".into(), "jpn".into()),
            (0, "CUSTOM_PCM_STREAM".into(), "PcmStreamVal".into()),
        ],
        max_rss_bytes: Some(4096 * 1024 * 1024),
        ..Default::default()
    };

    let decode_plan = fvid::media::plan_decode_audio(
        &wav,
        &AudioDecodeTransform::default(),
        &pcm_mka_opts,
    )
    .expect("plan_decode_audio with Matroska container & stream metadata");
    assert_eq!(decode_plan.command, "decode-audio");
    assert!(decode_plan.steps.iter().any(|s| s.action == "metadata"));

    let decoded_mka = dir.0.join("decoded.mka");
    let decoded_stats = fvid::media::decode_audio(&wav, &decoded_mka, &pcm_mka_opts)
        .expect("decode_audio to .mka with Matroska container & stream metadata");
    assert_eq!(decoded_stats.sample_frames, 960);
    let decoded_probed = fvid::media::probe(&decoded_mka).unwrap();
    assert_eq!(
        decoded_probed.metadata.get("title").map(String::as_str),
        Some("Decoded Matroska PCM")
    );
    assert_eq!(
        decoded_probed
            .metadata
            .get("CUSTOM_DECODE_TAG")
            .map(String::as_str),
        Some("DecodeTagVal")
    );
    assert!(!decoded_probed.metadata.contains_key("artist"));
    assert_eq!(
        decoded_probed.streams[0]
            .metadata
            .get("title")
            .map(String::as_str),
        Some("Decoded PCM Track")
    );
    assert_eq!(
        decoded_probed.streams[0]
            .metadata
            .get("language")
            .map(String::as_str),
        Some("jpn")
    );
    assert_eq!(
        decoded_probed.streams[0]
            .metadata
            .get("CUSTOM_PCM_STREAM")
            .map(String::as_str),
        Some("PcmStreamVal")
    );
    assert!(!decoded_probed.streams[0].metadata.contains_key("DROP_ME"));

    // Also test `trim` to `.mka` on ADTS source with container & stream metadata mutations.
    let trim_plan = fvid::media::plan_trim(&adts_a, 0, 20_000, &pcm_mka_opts)
        .expect("plan_trim ADTS with Matroska metadata");
    assert_eq!(trim_plan.command, "trim");
    assert!(trim_plan.steps.iter().any(|s| s.action == "metadata"));

    let trimmed_mka = dir.0.join("trimmed.mka");
    let trimmed_stats = fvid::media::trim(&adts_a, &trimmed_mka, 0, 20_000, &pcm_mka_opts)
        .expect("trim ADTS -> .mka with Matroska metadata");
    assert_eq!(trimmed_stats.payload_bytes, 960 * 4);
    let trimmed_probed = fvid::media::probe(&trimmed_mka).unwrap();
    assert_eq!(
        trimmed_probed.metadata.get("title").map(String::as_str),
        Some("Decoded Matroska PCM")
    );
    assert_eq!(
        trimmed_probed.streams[0]
            .metadata
            .get("title")
            .map(String::as_str),
        Some("Decoded PCM Track")
    );
    assert_eq!(
        trimmed_probed.streams[0]
            .metadata
            .get("language")
            .map(String::as_str),
        Some("jpn")
    );

    // 4. CLI `decode-audio`, `plan decode-audio`, `trim`, and `plan trim` with `--stream-metadata` and `--metadata`.
    let cli_decode_plan = Command::new(bin_path())
        .args([
            "media",
            "plan",
            "decode-audio",
            wav.to_str().unwrap(),
            "--metadata",
            "title=CLI Decoded MKA",
            "--stream-metadata",
            "0:title=CLI Stream Title",
            "--stream-metadata",
            "0:language=spa",
        ])
        .output()
        .unwrap();
    assert!(
        cli_decode_plan.status.success(),
        "cli plan decode-audio failed: {}",
        String::from_utf8_lossy(&cli_decode_plan.stderr)
    );

    let cli_decoded_mka = dir.0.join("cli-decoded.mka");
    let cli_decode_out = Command::new(bin_path())
        .args([
            "media",
            "decode-audio",
            wav.to_str().unwrap(),
            cli_decoded_mka.to_str().unwrap(),
            "--metadata",
            "title=CLI Decoded MKA",
            "--metadata",
            "CUSTOM_MKA=CLIMkaVal",
            "--stream-metadata",
            "0:title=CLI Stream Title",
            "--stream-metadata",
            "0:language=spa",
            "--stream-metadata",
            "0:CUSTOM_TRACK=CLITrackVal",
        ])
        .output()
        .unwrap();
    assert!(
        cli_decode_out.status.success(),
        "cli decode-audio failed: {}",
        String::from_utf8_lossy(&cli_decode_out.stderr)
    );
    let cli_decoded_probed = fvid::media::probe(&cli_decoded_mka).unwrap();
    assert_eq!(
        cli_decoded_probed.metadata.get("title").map(String::as_str),
        Some("CLI Decoded MKA")
    );
    assert_eq!(
        cli_decoded_probed
            .metadata
            .get("CUSTOM_MKA")
            .map(String::as_str),
        Some("CLIMkaVal")
    );
    assert_eq!(
        cli_decoded_probed.streams[0]
            .metadata
            .get("title")
            .map(String::as_str),
        Some("CLI Stream Title")
    );
    assert_eq!(
        cli_decoded_probed.streams[0]
            .metadata
            .get("language")
            .map(String::as_str),
        Some("spa")
    );
    assert_eq!(
        cli_decoded_probed.streams[0]
            .metadata
            .get("CUSTOM_TRACK")
            .map(String::as_str),
        Some("CLITrackVal")
    );

    let cli_trim_plan = Command::new(bin_path())
        .args([
            "media",
            "plan",
            "trim",
            adts_a.to_str().unwrap(),
            "--from",
            "0",
            "--to",
            "0.02",
            "--metadata",
            "title=CLI Trimmed MKA",
            "--stream-metadata",
            "0:title=CLI Trimmed Stream",
        ])
        .output()
        .unwrap();
    assert!(
        cli_trim_plan.status.success(),
        "cli plan trim failed: {}",
        String::from_utf8_lossy(&cli_trim_plan.stderr)
    );

    let cli_trimmed_mka = dir.0.join("cli-trimmed.mka");
    let cli_trim_out = Command::new(bin_path())
        .args([
            "media",
            "trim",
            adts_a.to_str().unwrap(),
            cli_trimmed_mka.to_str().unwrap(),
            "--from",
            "0",
            "--to",
            "0.02",
            "--metadata",
            "title=CLI Trimmed MKA",
            "--stream-metadata",
            "0:title=CLI Trimmed Stream",
            "--stream-metadata",
            "0:language=ita",
        ])
        .output()
        .unwrap();
    assert!(
        cli_trim_out.status.success(),
        "cli trim failed: {}",
        String::from_utf8_lossy(&cli_trim_out.stderr)
    );
    let cli_trimmed_probed = fvid::media::probe(&cli_trimmed_mka).unwrap();
    assert_eq!(
        cli_trimmed_probed.metadata.get("title").map(String::as_str),
        Some("CLI Trimmed MKA")
    );
    assert_eq!(
        cli_trimmed_probed.streams[0]
            .metadata
            .get("title")
            .map(String::as_str),
        Some("CLI Trimmed Stream")
    );
    assert_eq!(
        cli_trimmed_probed.streams[0]
            .metadata
            .get("language")
            .map(String::as_str),
        Some("ita")
    );

    // 5. Refusal checks: out-of-range stream metadata index or metadata on raw `.f32le` or `.mp4` ADTS remux must fail cleanly.
    let bad_stream_opts = CopyOptions {
        stream_metadata_set: vec![(1, "title".into(), "Out of bounds".into())],
        ..Default::default()
    };
    assert!(
        fvid::media::remux(&adts_a, &dir.0.join("bad-stream.mka"), &bad_stream_opts).is_err(),
        "expected out-of-range stream index to fail on ADTS remux"
    );
    assert!(
        fvid::media::decode_audio(&wav, &dir.0.join("bad-stream-decode.mka"), &bad_stream_opts)
            .is_err(),
        "expected out-of-range stream index to fail on decode_audio"
    );
    assert!(
        fvid::media::decode_audio(&wav, &dir.0.join("bad-f32.f32le"), &pcm_mka_opts).is_err(),
        "expected metadata mutations on headerless .f32le to fail"
    );
}
