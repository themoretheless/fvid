use fvid::media::CopyOptions;
use std::path::{Path, PathBuf};
use std::process::Command;

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/audio-sample-concat-wave-metadata")
        .join(name)
}

fn bin_path() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_fvid"))
}

struct TempDir(PathBuf);
impl TempDir {
    fn new(tag: &str) -> Self {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let dir = std::env::temp_dir().join(format!(
            "fvid-audio-sample-concat-wave-metadata-{tag}-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
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

#[test]
fn sample_domain_audio_concat_supports_wave_and_matroska_outputs_and_metadata_in_api_and_cli() {
    let tmp = TempDir::new("concat");
    let pcm16 = fixture("pcm16-tagged-48k.wav");
    let pcm24 = fixture("pcm24-48k.wav");
    let g711 = fixture("g711-alaw-48k.wav");
    let flac = fixture("raw-flac-48k.flac");
    let ogg = fixture("ogg-opus-48k.ogg");
    let mp3 = fixture("tagged-48k.mp3");
    let mka_flac = fixture("matroska-flac-48k.mka");

    // 1. Sample-domain concat of bareMP3 + bareFLAC + Ogg Opus + Matroska FLAC + G.711 WAVE to `.wav`
    //    with WAVE INFO metadata mutations in API (`concat_audio_with_options` and `media::concat`) and CLI.
    let mixed = vec![
        mp3.clone(),
        flac.clone(),
        ogg.clone(),
        mka_flac.clone(),
        g711.clone(),
    ];
    let wave_meta_opts = CopyOptions {
        metadata_set: vec![
            ("title".into(), "Sample Concat Wave Title".into()),
            ("artist".into(), "Sample Concat Artist".into()),
        ],
        metadata_delete: vec!["comment".into()],
        ..Default::default()
    };
    assert!(
        fvid::native_audio_mix::concat_eligible(&mixed).unwrap(),
        "concat_eligible must accept mixed single-stream audio including bare MP3"
    );
    assert!(
        fvid::native_audio_mix::concat_eligible_with_options(&mixed, &wave_meta_opts).unwrap(),
        "concat_eligible_with_options must accept mixed single-stream audio with WAVE INFO metadata"
    );

    let plan_mixed = fvid::media::plan_concat(&mixed, &wave_meta_opts)
        .expect("plan_concat on mixed audio with metadata");
    assert_eq!(plan_mixed.command, "concat");
    assert_eq!(plan_mixed.streams[0].disposition, "decode_to_pcm");
    assert!(
        plan_mixed.steps.iter().any(|s| s.action == "metadata"),
        "expected metadata step in sample-domain concat plan: {:?}",
        plan_mixed.steps
    );

    let out_mixed_wav = tmp.0.join("mixed-concat.wav");
    let mixed_stats = fvid::media::concat(&mixed, &out_mixed_wav, &wave_meta_opts)
        .expect("media::concat mixed audio to .wav with INFO metadata");
    assert_eq!(mixed_stats.segments, 5);
    assert!(out_mixed_wav.is_file());
    let probed_mixed_wav = fvid::media::probe(&out_mixed_wav).unwrap();
    assert_eq!(probed_mixed_wav.streams.len(), 1);
    assert_eq!(probed_mixed_wav.streams[0].codec, "pcm_f32le");
    assert_eq!(probed_mixed_wav.streams[0].sample_rate, 48_000);
    assert_eq!(probed_mixed_wav.streams[0].channels, 1);
    assert_eq!(
        probed_mixed_wav.metadata.get("title").map(String::as_str),
        Some("Sample Concat Wave Title")
    );
    assert_eq!(
        probed_mixed_wav.metadata.get("artist").map(String::as_str),
        Some("Sample Concat Artist")
    );

    // Stream metadata on .wav concat output must be rejected cleanly without leaving partial files.
    let bad_wav_stream_meta = CopyOptions {
        stream_metadata_set: vec![(0, "title".into(), "ForbiddenOnWave".into())],
        ..Default::default()
    };
    let never_wav = tmp.0.join("never-stream-meta.wav");
    assert!(
        fvid::media::concat(&mixed, &never_wav, &bad_wav_stream_meta).is_err(),
        "sample-domain concat to .wav must reject stream metadata"
    );
    assert!(!never_wav.exists());

    // 2. Mixed PCM bit-depth WAVE (16-bit + 24-bit) or PCM WAVE + G.711 WAVE concat to `.wav` and `.mka`,
    //    plus homogeneous WAVE concat to `.mka`/`.mkv` with custom Matroska container and stream metadata!
    let mixed_depth_waves = vec![pcm16.clone(), pcm24.clone()];
    let plan_mixed_depth = fvid::media::plan_concat(&mixed_depth_waves, &wave_meta_opts)
        .expect("plan_concat on 16-bit + 24-bit WAVE must fall back to sample-domain PCM concat");
    assert_eq!(plan_mixed_depth.streams[0].disposition, "decode_to_pcm");

    let out_mixed_depth_wav = tmp.0.join("mixed-depth.wav");
    let depth_stats = fvid::media::concat(&mixed_depth_waves, &out_mixed_depth_wav, &wave_meta_opts)
        .expect("concat 16-bit + 24-bit WAVE to .wav");
    assert_eq!(depth_stats.payload_bytes, (1920 + 1920) * 4);
    let probed_depth_wav = fvid::media::probe(&out_mixed_depth_wav).unwrap();
    assert_eq!(probed_depth_wav.streams[0].codec, "pcm_f32le");
    assert_eq!(
        probed_depth_wav.metadata.get("title").map(String::as_str),
        Some("Sample Concat Wave Title")
    );

    let mka_meta_opts = CopyOptions {
        metadata_set: vec![
            ("title".into(), "Matroska Wave Concat".into()),
            ("CUSTOM_CONCAT_TAG".into(), "CustomConcatVal".into()),
        ],
        stream_metadata_set: vec![
            (0, "title".into(), "Wave Concat Stream 0".into()),
            (0, "language".into(), "fra".into()),
            (0, "CUSTOM_STREAM_TAG".into(), "CustomStreamVal".into()),
        ],
        ..Default::default()
    };
    let same_waves = vec![pcm16.clone(), pcm16.clone()];
    let plan_wave_mka = fvid::media::plan_concat(&same_waves, &mka_meta_opts)
        .expect("plan_concat on WAVE with Matroska stream/custom metadata");
    assert_eq!(plan_wave_mka.streams[0].disposition, "decode_to_pcm");
    assert!(plan_wave_mka.steps.iter().any(|s| s.action == "metadata"));

    let out_wave_mka = tmp.0.join("wave-concat-tagged.mka");
    fvid::media::concat(&same_waves, &out_wave_mka, &mka_meta_opts)
        .expect("media::concat WAVE -> .mka with Matroska metadata");
    let probed_wave_mka = fvid::media::probe(&out_wave_mka).unwrap();
    assert_eq!(
        probed_wave_mka.metadata.get("title").map(String::as_str),
        Some("Matroska Wave Concat")
    );
    assert_eq!(
        probed_wave_mka
            .metadata
            .get("CUSTOM_CONCAT_TAG")
            .map(String::as_str),
        Some("CustomConcatVal")
    );
    assert_eq!(
        probed_wave_mka.streams[0]
            .metadata
            .get("title")
            .map(String::as_str),
        Some("Wave Concat Stream 0")
    );
    assert_eq!(
        probed_wave_mka.streams[0]
            .metadata
            .get("language")
            .map(String::as_str),
        Some("fra")
    );
    assert_eq!(
        probed_wave_mka.streams[0]
            .metadata
            .get("CUSTOM_STREAM_TAG")
            .map(String::as_str),
        Some("CustomStreamVal")
    );

    // 3. CLI routing for sample-domain `concat` and `plan concat` to `.wav` and `.mka`/`.mkv`
    //    across mixed audio (MP3 + FLAC + Ogg), mixed-depth WAVE (16-bit + 24-bit), and WAVE + G.711 WAVE.
    let cli_plan_mixed = Command::new(bin_path())
        .args([
            "media",
            "plan",
            "concat",
            mp3.to_str().unwrap(),
            flac.to_str().unwrap(),
            ogg.to_str().unwrap(),
            "--metadata",
            "title=CLIMixedPlan",
        ])
        .output()
        .expect("cli plan concat mixed audio");
    assert!(
        cli_plan_mixed.status.success(),
        "CLI plan concat mixed audio failed: {}",
        String::from_utf8_lossy(&cli_plan_mixed.stderr)
    );
    let cli_plan_json: serde_json::Value = serde_json::from_slice(&cli_plan_mixed.stdout).unwrap();
    assert_eq!(cli_plan_json["command"], "concat");
    assert_eq!(cli_plan_json["streams"][0]["disposition"], "decode_to_pcm");

    let cli_mixed_wav = tmp.0.join("cli-mixed.wav");
    let cli_run_mixed = Command::new(bin_path())
        .args([
            "media",
            "concat",
            cli_mixed_wav.to_str().unwrap(),
            mp3.to_str().unwrap(),
            flac.to_str().unwrap(),
            ogg.to_str().unwrap(),
            "--metadata",
            "title=CLIMixedWave",
            "--metadata",
            "artist=CLIMixedArtist",
            "-q",
        ])
        .output()
        .expect("cli concat mixed audio to .wav");
    assert!(
        cli_run_mixed.status.success(),
        "CLI concat mixed audio to .wav failed: {}",
        String::from_utf8_lossy(&cli_run_mixed.stderr)
    );
    let probed_cli_mixed = fvid::media::probe(&cli_mixed_wav).unwrap();
    assert_eq!(
        probed_cli_mixed.metadata.get("title").map(String::as_str),
        Some("CLIMixedWave")
    );
    assert_eq!(
        probed_cli_mixed.metadata.get("artist").map(String::as_str),
        Some("CLIMixedArtist")
    );

    let cli_wave_g711_wav = tmp.0.join("cli-wave-g711.wav");
    let cli_run_wave_g711 = Command::new(bin_path())
        .args([
            "media",
            "concat",
            cli_wave_g711_wav.to_str().unwrap(),
            pcm16.to_str().unwrap(),
            g711.to_str().unwrap(),
            "--metadata",
            "title=CLIWaveG711",
            "--metadata-delete",
            "comment",
            "-q",
        ])
        .output()
        .expect("cli concat pcm16 + g711 to .wav");
    assert!(
        cli_run_wave_g711.status.success(),
        "CLI concat pcm16 + g711 to .wav failed: {}",
        String::from_utf8_lossy(&cli_run_wave_g711.stderr)
    );
    let probed_cli_wg = fvid::media::probe(&cli_wave_g711_wav).unwrap();
    assert_eq!(probed_cli_wg.streams[0].codec, "pcm_f32le");
    assert_eq!(
        probed_cli_wg.metadata.get("title").map(String::as_str),
        Some("CLIWaveG711")
    );

    let cli_plan_wave_mka = Command::new(bin_path())
        .args([
            "media",
            "plan",
            "concat",
            pcm16.to_str().unwrap(),
            pcm16.to_str().unwrap(),
            "--metadata",
            "CUSTOM_TAG=CLIWaveMka",
            "--stream-metadata",
            "0:title=CLIStreamTitle",
        ])
        .output()
        .expect("cli plan concat wave with stream metadata");
    assert!(
        cli_plan_wave_mka.status.success(),
        "CLI plan concat wave with stream metadata failed: {}",
        String::from_utf8_lossy(&cli_plan_wave_mka.stderr)
    );

    let cli_wave_mka = tmp.0.join("cli-wave.mka");
    let cli_run_wave_mka = Command::new(bin_path())
        .args([
            "media",
            "concat",
            cli_wave_mka.to_str().unwrap(),
            pcm16.to_str().unwrap(),
            pcm16.to_str().unwrap(),
            "--metadata",
            "title=CLIWaveMkaTitle",
            "--metadata",
            "CUSTOM_TAG=CLIWaveMkaVal",
            "--stream-metadata",
            "0:title=CLIWaveStream",
            "--stream-metadata",
            "0:language=deu",
            "-q",
        ])
        .output()
        .expect("cli concat wave to .mka with metadata");
    assert!(
        cli_run_wave_mka.status.success(),
        "CLI concat wave to .mka failed: {}",
        String::from_utf8_lossy(&cli_run_wave_mka.stderr)
    );
    let probed_cli_wave_mka = fvid::media::probe(&cli_wave_mka).unwrap();
    assert_eq!(
        probed_cli_wave_mka.metadata.get("title").map(String::as_str),
        Some("CLIWaveMkaTitle")
    );
    assert_eq!(
        probed_cli_wave_mka
            .metadata
            .get("CUSTOM_TAG")
            .map(String::as_str),
        Some("CLIWaveMkaVal")
    );
    assert_eq!(
        probed_cli_wave_mka.streams[0]
            .metadata
            .get("title")
            .map(String::as_str),
        Some("CLIWaveStream")
    );
    assert_eq!(
        probed_cli_wave_mka.streams[0]
            .metadata
            .get("language")
            .map(String::as_str),
        Some("deu")
    );
}
