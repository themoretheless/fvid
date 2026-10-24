use fvid::media::{AudioDecodeTransform, CopyOptions, MixAudioOptions};
use std::path::{Path, PathBuf};
use std::process::Command;

struct TempDir(PathBuf);
impl TempDir {
    fn new(tag: &str) -> Self {
        let dir = std::env::temp_dir().join(format!(
            "fvid-audio-wave-metadata-mix-controls-{tag}-{}",
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
        .join("tests/fixtures/playback-errors/audio-wave-metadata-mix-controls")
        .join(name)
}

fn bin_path() -> &'static str {
    env!("CARGO_BIN_EXE_fvid")
}

#[test]
fn audio_decode_trim_metadata_edits_and_mix_merge_controls_work_in_api_and_cli() {
    let dir = TempDir::new("wave-meta-mix");
    let wav = fixture("tagged-48k.wav");
    let adts = fixture("adts-48k.aac");
    let flac = fixture("flac-48k.flac");

    // 1. Public API `plan_decode_audio` and `decode_audio` with WAVE INFO metadata mutations on WAVE and ADTS.
    let meta_opts = CopyOptions {
        metadata_delete: vec!["artist".into()],
        metadata_set: vec![("title".into(), "Edited WAV Title".into())],
        max_rss_bytes: Some(4096 * 1024 * 1024),
        ..Default::default()
    };
    let wav_plan = fvid::media::plan_decode_audio(
        &wav,
        &AudioDecodeTransform::default(),
        &meta_opts,
    )
    .expect("plan_decode_audio WAVE with metadata edits");
    assert_eq!(wav_plan.command, "decode-audio");
    assert!(wav_plan.steps.iter().any(|s| s.action == "metadata"));

    let decoded_wav = dir.0.join("decoded-tagged.wav");
    let wav_stats = fvid::media::decode_audio(&wav, &decoded_wav, &meta_opts)
        .expect("decode_audio WAVE with metadata edits");
    assert_eq!(wav_stats.sample_frames, 960);
    let wav_probed = fvid::media::probe(&decoded_wav).unwrap();
    assert_eq!(
        wav_probed.metadata.get("title").map(String::as_str),
        Some("Edited WAV Title")
    );
    assert!(!wav_probed.metadata.contains_key("artist"));
    assert_eq!(
        wav_probed.metadata.get("comment").map(String::as_str),
        Some("Preserve Me")
    );

    let adts_plan = fvid::media::plan_decode_audio(
        &adts,
        &AudioDecodeTransform::default(),
        &meta_opts,
    )
    .expect("plan_decode_audio ADTS with metadata edits");
    assert_eq!(adts_plan.command, "decode-audio");
    assert!(adts_plan.steps.iter().any(|s| s.action == "metadata"));

    let decoded_adts = dir.0.join("decoded-adts.wav");
    let adts_stats = fvid::media::decode_audio(&adts, &decoded_adts, &meta_opts)
        .expect("decode_audio ADTS with metadata edits");
    assert_eq!(adts_stats.sample_frames, 3072);
    let adts_probed = fvid::media::probe(&decoded_adts).unwrap();
    assert_eq!(
        adts_probed.metadata.get("title").map(String::as_str),
        Some("Edited WAV Title")
    );

    // 2. Public API `plan_trim` and `trim` with WAVE INFO metadata mutations on WAVE and ADTS.
    let trim_wav_plan = fvid::media::plan_trim(&wav, 0, 10_000, &meta_opts)
        .expect("plan_trim WAVE with metadata edits");
    assert_eq!(trim_wav_plan.command, "trim");

    let trimmed_wav = dir.0.join("trimmed-tagged.wav");
    let trim_wav_stats = fvid::media::trim(&wav, &trimmed_wav, 0, 10_000, &meta_opts)
        .expect("trim WAVE with metadata edits");
    assert_eq!(trim_wav_stats.payload_bytes, 480 * 2);
    let trimmed_wav_probed = fvid::media::probe(&trimmed_wav).unwrap();
    assert_eq!(
        trimmed_wav_probed.metadata.get("title").map(String::as_str),
        Some("Edited WAV Title")
    );
    assert!(!trimmed_wav_probed.metadata.contains_key("artist"));

    let trim_adts_plan = fvid::media::plan_trim(&adts, 0, 20_000, &meta_opts)
        .expect("plan_trim ADTS with metadata edits");
    assert_eq!(trim_adts_plan.command, "trim");

    let trimmed_adts = dir.0.join("trimmed-adts.wav");
    let trim_adts_stats = fvid::media::trim(&adts, &trimmed_adts, 0, 20_000, &meta_opts)
        .expect("trim ADTS with metadata edits");
    assert_eq!(trim_adts_stats.payload_bytes, 960 * 4);
    let trimmed_adts_probed = fvid::media::probe(&trimmed_adts).unwrap();
    assert_eq!(
        trimmed_adts_probed.metadata.get("title").map(String::as_str),
        Some("Edited WAV Title")
    );

    // 3. CLI `plan decode-audio`, `decode-audio`, `plan trim`, `trim`, `plan trim-pcm`, and `trim-pcm` with `--metadata` and `--metadata-delete`.
    let cli_plan_decode = Command::new(bin_path())
        .args([
            "media",
            "plan",
            "decode-audio",
            adts.to_str().unwrap(),
            "--metadata",
            "title=CLI Decoded Title",
            "--metadata-delete",
            "artist",
        ])
        .output()
        .unwrap();
    assert!(
        cli_plan_decode.status.success(),
        "cli plan decode-audio failed: {}",
        String::from_utf8_lossy(&cli_plan_decode.stderr)
    );
    let cli_plan_decode_json: serde_json::Value =
        serde_json::from_slice(&cli_plan_decode.stdout).unwrap();
    assert_eq!(cli_plan_decode_json["command"], "decode-audio");

    let cli_decoded_wav = dir.0.join("cli-decoded.wav");
    let cli_decode = Command::new(bin_path())
        .args([
            "media",
            "decode-audio",
            wav.to_str().unwrap(),
            cli_decoded_wav.to_str().unwrap(),
            "--metadata-delete",
            "artist",
            "--metadata",
            "title=CLI Decoded WAV",
        ])
        .output()
        .unwrap();
    assert!(
        cli_decode.status.success(),
        "cli decode-audio failed: {}",
        String::from_utf8_lossy(&cli_decode.stderr)
    );
    let cli_decoded_probed = fvid::media::probe(&cli_decoded_wav).unwrap();
    assert_eq!(
        cli_decoded_probed.metadata.get("title").map(String::as_str),
        Some("CLI Decoded WAV")
    );
    assert!(!cli_decoded_probed.metadata.contains_key("artist"));

    let cli_plan_trim = Command::new(bin_path())
        .args([
            "media",
            "plan",
            "trim",
            wav.to_str().unwrap(),
            "--from",
            "0",
            "--to",
            "0.01",
            "--metadata",
            "title=CLI Trimmed WAV",
        ])
        .output()
        .unwrap();
    assert!(
        cli_plan_trim.status.success(),
        "cli plan trim failed: {}",
        String::from_utf8_lossy(&cli_plan_trim.stderr)
    );

    let cli_trimmed_wav = dir.0.join("cli-trimmed.wav");
    let cli_trim = Command::new(bin_path())
        .args([
            "media",
            "trim",
            wav.to_str().unwrap(),
            cli_trimmed_wav.to_str().unwrap(),
            "--from",
            "0",
            "--to",
            "0.01",
            "--metadata-delete",
            "artist",
            "--metadata",
            "title=CLI Trimmed WAV",
        ])
        .output()
        .unwrap();
    assert!(
        cli_trim.status.success(),
        "cli trim failed: {}",
        String::from_utf8_lossy(&cli_trim.stderr)
    );
    let cli_trimmed_probed = fvid::media::probe(&cli_trimmed_wav).unwrap();
    assert_eq!(
        cli_trimmed_probed.metadata.get("title").map(String::as_str),
        Some("CLI Trimmed WAV")
    );
    assert!(!cli_trimmed_probed.metadata.contains_key("artist"));

    let cli_trimmed_pcm = dir.0.join("cli-trimmed-pcm.wav");
    let cli_trim_pcm = Command::new(bin_path())
        .args([
            "media",
            "trim-pcm",
            wav.to_str().unwrap(),
            cli_trimmed_pcm.to_str().unwrap(),
            "--from",
            "0",
            "--to",
            "0.01",
            "--metadata",
            "title=CLI Trimmed PCM",
        ])
        .output()
        .unwrap();
    assert!(
        cli_trim_pcm.status.success(),
        "cli trim-pcm failed: {}",
        String::from_utf8_lossy(&cli_trim_pcm.stderr)
    );
    assert_eq!(
        fvid::media::probe(&cli_trimmed_pcm)
            .unwrap()
            .metadata
            .get("title")
            .map(String::as_str),
        Some("CLI Trimmed PCM")
    );

    // 4. Public API `plan_mix_audio_with_controls`, `mix_audio_with_controls`, `plan_merge_audio_with_controls`, and `merge_audio_with_controls`.
    let mix_controls = CopyOptions {
        max_packets: Some(1),
        max_rss_bytes: Some(4096 * 1024 * 1024),
        ..Default::default()
    };
    let sources = vec![adts.clone(), flac.clone()];
    let mix_plan = fvid::media::plan_mix_audio_with_controls(
        &sources,
        &MixAudioOptions::default(),
        &mix_controls,
    )
    .expect("plan_mix_audio_with_controls");
    assert_eq!(mix_plan.command, "mix-audio");
    assert!(mix_plan
        .notes
        .iter()
        .any(|n| n.contains("packet work limit: Some(1)")));

    let mixed_mka = dir.0.join("mixed.mka");
    let mix_stats = fvid::media::mix_audio_with_controls(
        &sources,
        &mixed_mka,
        &MixAudioOptions::default(),
        &mix_controls,
    )
    .expect("mix_audio_with_controls");
    // 1 packet of ADTS = 1024 frames, 1 packet of FLAC = 1920 frames -> shortest = 1024 frames.
    assert_eq!(mix_stats.sample_frames, 1024);

    let merge_plan = fvid::media::plan_merge_audio_with_controls(&sources, &mix_controls)
        .expect("plan_merge_audio_with_controls");
    assert_eq!(merge_plan.command, "merge-audio");

    let merged_mka = dir.0.join("merged.mka");
    let merge_stats =
        fvid::media::merge_audio_with_controls(&sources, &merged_mka, &mix_controls)
            .expect("merge_audio_with_controls");
    assert_eq!(merge_stats.sample_frames, 1024);
    assert_eq!(merge_stats.channels, 2);

    // Tight RSS budget on mix/merge with controls fails cleanly.
    let tight_rss = CopyOptions {
        max_rss_bytes: Some(1024 * 1024),
        ..Default::default()
    };
    assert!(fvid::media::plan_mix_audio_with_controls(
        &sources,
        &MixAudioOptions::default(),
        &tight_rss
    )
    .is_err());
    assert!(fvid::media::plan_merge_audio_with_controls(&sources, &tight_rss).is_err());
}
