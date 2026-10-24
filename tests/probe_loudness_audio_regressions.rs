use fvid_media::{
    AudioDecodeTransform, CopyOptions, MixAudioOptions, apply_loudnorm, apply_loudnorm_dual,
    measure_loudness, plan_decode_audio, plan_loudness, plan_loudnorm, plan_merge_audio,
    plan_mix_audio, probe, probe_as,
};
use std::path::{Path, PathBuf};

fn fixture_path(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/probe-loudness-audio")
        .join(name)
}

struct TempDir(PathBuf);
impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
fn temp_dir(label: &str) -> TempDir {
    let dir = std::env::temp_dir().join(format!(
        "fvid-probe-loudness-audio-{label}-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create temp dir");
    TempDir(dir)
}

#[test]
fn g711_wave_and_ogg_and_bare_flac_and_mp3_probe_across_owned_and_native_layers() {
    let alaw = fixture_path("g711-alaw-48k.wav");
    let mulaw = fixture_path("g711-mulaw-48k.wav");
    let ogg_opus = fixture_path("ogg-opus-48k.ogg");
    let ogg_vorbis = fixture_path("ogg-vorbis-48k.ogg");
    let ogg_joint = fixture_path("ogg-joint-48k.ogg");
    let flac = fixture_path("raw-flac-48k.flac");
    let mp3 = fixture_path("raw-mp3-48k.mp3");

    for (path, hint, expected_format, expected_codec, expected_rate, expected_channels) in [
        (&alaw, Some("wav"), "wav", "pcm_alaw", 48_000, 1),
        (&mulaw, Some("wav"), "wav", "pcm_mulaw", 48_000, 1),
        (&ogg_opus, Some("ogg"), "ogg", "opus", 48_000, 2),
        (&ogg_vorbis, Some("ogg"), "ogg", "vorbis", 48_000, 2),
        (&ogg_joint, Some("ogg"), "ogg", "vorbis_opus", 48_000, 2),
        (&flac, Some("flac"), "flac", "flac", 8_000, 4),
        (&mp3, Some("mp3"), "mp3", "mp3", 48_000, 2),
    ] {
        let info_owned = probe(path).unwrap_or_else(|e| {
            panic!("fvid_media::probe failed for {}: {e}", path.display())
        });
        assert_eq!(info_owned.format, expected_format);
        assert_eq!(info_owned.streams.len(), 1);
        assert_eq!(info_owned.streams[0].codec, expected_codec);
        assert_eq!(info_owned.streams[0].sample_rate, expected_rate);
        assert_eq!(info_owned.streams[0].channels, expected_channels);

        let info_hinted = probe_as(path, hint).unwrap_or_else(|e| {
            panic!("fvid_media::probe_as({hint:?}) failed for {}: {e}", path.display())
        });
        assert_eq!(info_hinted.streams[0].codec, expected_codec);

        let info_native = fvid::native_probe::probe(path).unwrap_or_else(|e| {
            panic!("fvid::native_probe::probe failed for {}: {e}", path.display())
        });
        assert_eq!(info_native.format, expected_format);
        assert_eq!(info_native.streams[0].codec, expected_codec);
        assert_eq!(info_native.streams[0].sample_rate, expected_rate);
        assert_eq!(info_native.streams[0].channels, expected_channels);

        let info_native_hinted = fvid::native_probe::probe_as(path, hint).unwrap_or_else(|e| {
            panic!(
                "fvid::native_probe::probe_as({hint:?}) failed for {}: {e}",
                path.display()
            )
        });
        assert_eq!(info_native_hinted.streams[0].codec, expected_codec);
    }

    // Verify metadata extraction on G.711 WAVE and Ogg Opus
    let alaw_info = probe(&alaw).expect("alaw probe");
    assert_eq!(
        alaw_info.metadata.get("title").map(String::as_str),
        Some("Synthetic G711")
    );
    let opus_info = probe(&ogg_opus).expect("ogg opus probe");
    assert_eq!(
        opus_info.metadata.get("title").map(String::as_str),
        Some("Synthetic Opus")
    );
    assert_eq!(
        opus_info.streams[0].metadata.get("language").map(String::as_str),
        Some("eng")
    );
}

#[test]
fn g711_ogg_flac_and_mp3_supported_in_owned_loudness_and_loudnorm_pipelines() {
    let dir = temp_dir("loudness-loudnorm");
    let cases = [
        ("g711-alaw-48k.wav", 48_000, 1),
        ("g711-mulaw-48k.wav", 48_000, 1),
        ("ogg-opus-48k.ogg", 48_000, 2),
        ("ogg-vorbis-48k.ogg", 48_000, 2),
        ("ogg-joint-48k.ogg", 48_000, 2),
        ("raw-flac-48k.flac", 8_000, 4),
        ("raw-mp3-48k.mp3", 48_000, 2),
    ];

    for (name, expected_rate, expected_channels) in cases {
        let source = fixture_path(name);
        let options = CopyOptions::default();

        let l_plan = plan_loudness(&source, &options)
            .unwrap_or_else(|e| panic!("plan_loudness failed for {name}: {e}"));
        assert_eq!(l_plan.command, "loudness");

        let l_stats = measure_loudness(&source, &options)
            .unwrap_or_else(|e| panic!("measure_loudness failed for {name}: {e}"));
        assert_eq!(l_stats.channels, expected_channels);
        assert_eq!(l_stats.sample_rate, expected_rate);
        assert!(l_stats.sample_frames > 0);

        let ln_plan = plan_loudnorm(&source, None, false, &options)
            .unwrap_or_else(|e| panic!("plan_loudnorm failed for {name}: {e}"));
        assert_eq!(ln_plan.command, "loudnorm");

        let native_ln_plan = fvid::native_loudnorm::try_plan(&source, None, true, &options)
            .unwrap_or_else(|e| panic!("native_loudnorm::try_plan errored for {name}: {e}"))
            .unwrap_or_else(|| panic!("native_loudnorm::try_plan returned None for {name}"));
        assert_eq!(native_ln_plan.command, "loudnorm");

        let out_single = dir.0.join(format!("{name}-single.wav"));
        let stats_single = apply_loudnorm(&source, &out_single, None, &options)
            .unwrap_or_else(|e| panic!("apply_loudnorm failed for {name}: {e}"));
        assert_eq!(stats_single.channels, expected_channels);
        assert!(out_single.is_file());

        let out_dual = dir.0.join(format!("{name}-dual.wav"));
        let stats_dual = fvid::native_loudnorm::try_apply(&source, &out_dual, None, true, &options)
            .unwrap_or_else(|e| panic!("native_loudnorm::try_apply errored for {name}: {e}"))
            .unwrap_or_else(|| panic!("native_loudnorm::try_apply returned None for {name}"));
        assert_eq!(stats_dual.channels, expected_channels);
        assert!(out_dual.is_file());

        let out_media_dual = dir.0.join(format!("{name}-media-dual.wav"));
        let stats_media_dual = apply_loudnorm_dual(&source, &out_media_dual, None, &options)
            .unwrap_or_else(|e| panic!("apply_loudnorm_dual failed for {name}: {e}"));
        assert_eq!(stats_media_dual.sample_frames, stats_dual.sample_frames);
    }
}

#[test]
fn empty_trimmed_mp3_rejected_at_planning_stage_while_probe_reports_zero_duration() {
    let empty_mp3 = fixture_path("raw-mp3-empty-trimmed.mp3");
    let valid_mp3 = fixture_path("raw-mp3-48k.mp3");

    // Probe succeeds and reports 0 audible duration
    let info = probe(&empty_mp3).expect("probe on zero-audible-sample MP3 should succeed");
    assert_eq!(info.streams[0].duration, Some(0));
    assert_eq!(info.duration_us, Some(0));

    // Planning decode-audio, mix-audio, merge-audio, and loudness must reject 0-audible-sample MP3
    let decode_err = plan_decode_audio(
        &empty_mp3,
        &AudioDecodeTransform::default(),
        &CopyOptions::default(),
    )
    .expect_err("plan_decode_audio must reject MP3 whose LAME tag trims all samples");
    assert!(
        decode_err.contains("contains no audible samples"),
        "unexpected plan_decode_audio error: {decode_err}"
    );

    let mix_err = plan_mix_audio(
        &[valid_mp3.clone(), empty_mp3.clone()],
        &MixAudioOptions::default(),
    )
    .expect_err("plan_mix_audio must reject MP3 whose LAME tag trims all samples");
    assert!(
        mix_err.contains("contains no audible samples"),
        "unexpected plan_mix_audio error: {mix_err}"
    );

    let merge_err = plan_merge_audio(&[valid_mp3, empty_mp3.clone()])
        .expect_err("plan_merge_audio must reject MP3 whose LAME tag trims all samples");
    assert!(
        merge_err.contains("contains no audible samples"),
        "unexpected plan_merge_audio error: {merge_err}"
    );

    let loud_err = plan_loudness(&empty_mp3, &CopyOptions::default())
        .expect_err("plan_loudness must reject MP3 whose LAME tag trims all samples");
    assert!(
        loud_err.contains("contains no audible samples"),
        "unexpected plan_loudness error: {loud_err}"
    );
}
