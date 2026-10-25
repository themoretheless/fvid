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

#[test]
fn decode_audio_and_trim_support_controlled_memory_budget_with_matroska_metadata_and_destination_aware_cli_planning() {
    let dir = TempDir::new("budget-and-cli");
    let adts_a = fixture("adts-seg-a-48k.aac");
    let wav = fixture("pcm-source-48k.wav");

    // 1. API: `plan_decode_audio`, `decode_audio_transformed`, `plan_trim`, and `trim` accept `max_controlled_bytes`
    // alongside Matroska container and stream 0 metadata mutations.
    let mut controlled_mka_opts = CopyOptions {
        metadata_set: vec![
            ("title".into(), "Controlled Budget MKA".into()),
            ("CUSTOM_BUDGET_TAG".into(), "BudgetTagVal".into()),
        ],
        stream_metadata_set: vec![
            (0, "title".into(), "Controlled Stream".into()),
            (0, "language".into(), "deu".into()),
        ],
        max_controlled_bytes: Some(16 * 1024 * 1024),
        ..Default::default()
    };
    let decode_plan = fvid::media::plan_decode_audio(
        &adts_a,
        &AudioDecodeTransform::default(),
        &controlled_mka_opts,
    )
    .expect("plan_decode_audio should accept max_controlled_bytes with Matroska metadata edits");
    assert_eq!(decode_plan.command, "decode-audio");
    assert!(decode_plan.steps.iter().any(|s| s.action == "metadata"));

    let trim_plan = fvid::media::plan_trim(&adts_a, 0, 20_000, &controlled_mka_opts)
        .expect("plan_trim should accept max_controlled_bytes with Matroska metadata edits");
    assert_eq!(trim_plan.command, "trim");
    assert!(trim_plan.steps.iter().any(|s| s.action == "metadata"));

    let decoded_mka = dir.0.join("controlled-decoded.mka");
    let decode_stats = fvid::media::decode_audio(&adts_a, &decoded_mka, &controlled_mka_opts)
        .expect("decode_audio to .mka should accept max_controlled_bytes with metadata edits");
    assert!(decode_stats.sample_frames > 0);
    let probed_decode = fvid::media::probe(&decoded_mka).unwrap();
    assert_eq!(
        probed_decode.metadata.get("title").map(String::as_str),
        Some("Controlled Budget MKA")
    );
    assert_eq!(
        probed_decode
            .metadata
            .get("CUSTOM_BUDGET_TAG")
            .map(String::as_str),
        Some("BudgetTagVal")
    );
    assert_eq!(
        probed_decode.streams[0]
            .metadata
            .get("title")
            .map(String::as_str),
        Some("Controlled Stream")
    );

    let trimmed_mka = dir.0.join("controlled-trimmed.mka");
    let trim_stats = fvid::media::trim(&adts_a, &trimmed_mka, 0, 20_000, &controlled_mka_opts)
        .expect("trim to .mka should accept max_controlled_bytes with metadata edits");
    assert!(trim_stats.packets > 0);
    let probed_trim = fvid::media::probe(&trimmed_mka).unwrap();
    assert_eq!(
        probed_trim.metadata.get("title").map(String::as_str),
        Some("Controlled Budget MKA")
    );

    // Tight `max_controlled_bytes` (1 byte) still refuses before writing output.
    controlled_mka_opts.max_controlled_bytes = Some(1);
    let tight_err = fvid::media::plan_decode_audio(
        &wav,
        &AudioDecodeTransform::default(),
        &controlled_mka_opts,
    )
    .expect_err("tight max_controlled_bytes must refuse during plan_decode_audio");
    assert!(
        tight_err.contains("controlled memory budget exceeded"),
        "unexpected error: {tight_err}"
    );
    let tight_out = dir.0.join("tight-should-not-exist.mka");
    let tight_exec_err =
        fvid::media::decode_audio(&wav, &tight_out, &controlled_mka_opts)
            .expect_err("tight max_controlled_bytes must refuse during decode_audio");
    assert!(
        tight_exec_err.contains("controlled memory budget exceeded"),
        "unexpected error: {tight_exec_err}"
    );
    assert!(!tight_out.exists());

    // 2. CLI `plan decode-audio` and `plan trim` accept an optional destination path and flags before positional args.
    let cli_plan_mka = Command::new(bin_path())
        .args([
            "media",
            "plan",
            "decode-audio",
            "--max-memory-mib",
            "16",
            adts_a.to_str().unwrap(),
            dir.0.join("planned-out.mka").to_str().unwrap(),
            "--metadata",
            "CUSTOM_TAG=OkMka",
            "--stream-metadata",
            "0:title=TrackZero",
        ])
        .output()
        .unwrap();
    assert!(
        cli_plan_mka.status.success(),
        "cli plan decode-audio with OUTPUT.mka failed: {}",
        String::from_utf8_lossy(&cli_plan_mka.stderr)
    );

    let cli_plan_trim_mka = Command::new(bin_path())
        .args([
            "media",
            "plan",
            "trim",
            "--max-memory-mib",
            "16",
            adts_a.to_str().unwrap(),
            dir.0.join("planned-trim.mka").to_str().unwrap(),
            "--from",
            "0",
            "--to",
            "0.02",
            "--metadata",
            "CUSTOM_TAG=OkTrimMka",
            "--stream-metadata",
            "0:title=TrimTrackZero",
        ])
        .output()
        .unwrap();
    assert!(
        cli_plan_trim_mka.status.success(),
        "cli plan trim with OUTPUT.mka failed: {}",
        String::from_utf8_lossy(&cli_plan_trim_mka.stderr)
    );

    // 3. Destination-specific validation on `plan decode-audio` and `plan trim`:
    // `.wav` rejects `--stream-metadata` and non-RIFF-INFO container keys; `.f32le` rejects all metadata mutations; `.mp3` is rejected.
    for bad_args in [
        vec![
            "media",
            "plan",
            "decode-audio",
            wav.to_str().unwrap(),
            dir.0.join("planned.wav").to_str().unwrap(),
            "--stream-metadata",
            "0:title=NotAllowedOnWav",
        ],
        vec![
            "media",
            "plan",
            "decode-audio",
            wav.to_str().unwrap(),
            dir.0.join("planned.wav").to_str().unwrap(),
            "--metadata",
            "CUSTOM_NON_RIFF=NotAllowedOnWav",
        ],
        vec![
            "media",
            "plan",
            "decode-audio",
            wav.to_str().unwrap(),
            dir.0.join("planned.f32le").to_str().unwrap(),
            "--metadata",
            "title=NotAllowedOnF32le",
        ],
        vec![
            "media",
            "plan",
            "decode-audio",
            wav.to_str().unwrap(),
            dir.0.join("planned.mp3").to_str().unwrap(),
        ],
        vec![
            "media",
            "plan",
            "trim",
            wav.to_str().unwrap(),
            dir.0.join("planned-trim.wav").to_str().unwrap(),
            "--from",
            "0",
            "--to",
            "0.02",
            "--stream-metadata",
            "0:title=NotAllowedOnWav",
        ],
        vec![
            "media",
            "plan",
            "trim-pcm",
            wav.to_str().unwrap(),
            dir.0.join("planned-trim.mka").to_str().unwrap(),
            "--from",
            "0",
            "--to",
            "0.02",
        ],
        vec![
            "media",
            "decode-audio",
            wav.to_str().unwrap(),
            dir.0.join("bad.mp3").to_str().unwrap(),
        ],
        vec![
            "media",
            "decode-audio",
            wav.to_str().unwrap(),
            "--quiet",
        ],
        vec![
            "media",
            "decode-audio",
            wav.to_str().unwrap(),
            dir.0.join("ok.wav").to_str().unwrap(),
            "--unknown-decode-audio-flag",
        ],
    ] {
        let out = Command::new(bin_path()).args(&bad_args).output().unwrap();
        assert!(
            !out.status.success(),
            "expected failure for args {:?}",
            bad_args
        );
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert!(
            !stderr.contains("requires cargo build --release --features media"),
            "unexpected feature-gate fallback for {:?}: {stderr}",
            bad_args
        );
    }

    // 4. WAVE -> `.f32le` `trim` with custom budgets (`max_controlled_bytes`, `max_packet_bytes`, `max_packets`):
    // `fvid::media::trim` and `fvid media trim` on a WAVE input to `.f32le` with `--max-memory-mib` or `--max-packets`
    // must succeed via `decode_fvid_media_audio_to_destination` instead of failing in `trim_pcm` (`"WAVE PCM trim requires .wav output"`),
    // and must reject metadata mutations on `.f32le`.
    let f32le_budget_opts = CopyOptions {
        max_controlled_bytes: Some(16 * 1024 * 1024),
        ..Default::default()
    };
    let wav_trimmed_f32le = dir.0.join("wav-controlled-trimmed.f32le");
    let wav_trimmed_f32le_stats =
        fvid::media::trim(&wav, &wav_trimmed_f32le, 0, 10_000, &f32le_budget_opts)
            .expect("trim WAVE -> .f32le with max_controlled_bytes");
    assert!(wav_trimmed_f32le_stats.packets > 0);
    assert_eq!(
        std::fs::metadata(&wav_trimmed_f32le).unwrap().len(),
        480 * 4
    );

    let cli_wav_trimmed_f32le = dir.0.join("cli-wav-controlled-trimmed.f32le");
    let cli_wav_trimmed_f32le_out = Command::new(bin_path())
        .args([
            "media",
            "trim",
            wav.to_str().unwrap(),
            cli_wav_trimmed_f32le.to_str().unwrap(),
            "--from",
            "0",
            "--to",
            "0.01",
            "--max-memory-mib",
            "16",
            "--quiet",
        ])
        .output()
        .unwrap();
    assert!(
        cli_wav_trimmed_f32le_out.status.success(),
        "cli trim WAVE -> .f32le with --max-memory-mib failed: {}",
        String::from_utf8_lossy(&cli_wav_trimmed_f32le_out.stderr)
    );
    assert_eq!(
        std::fs::metadata(&cli_wav_trimmed_f32le).unwrap().len(),
        480 * 4
    );

    // Tight `max_controlled_bytes` (1 byte) on WAVE -> `.f32le` `trim` must refuse before writing output.
    let tight_wav_f32le = dir.0.join("tight-wav-trimmed.f32le");
    let tight_wav_f32le_err = fvid::media::trim(
        &wav,
        &tight_wav_f32le,
        0,
        10_000,
        &CopyOptions {
            max_controlled_bytes: Some(1),
            ..Default::default()
        },
    )
    .expect_err("tight max_controlled_bytes on WAVE -> .f32le trim must fail");
    assert!(
        tight_wav_f32le_err.contains("controlled memory budget exceeded"),
        "unexpected error: {tight_wav_f32le_err}"
    );
    assert!(!tight_wav_f32le.exists());
}

