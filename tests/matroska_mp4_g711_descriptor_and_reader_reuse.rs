use fvid_media::{
    CopyOptions, apply_loudnorm, decode_audio, decode_audio_transformed, measure_loudness,
    plan_decode_audio,
};
use fvid_media_info::AudioDecodeTransform;
use std::path::Path;

#[test]
fn matroska_and_mp4_audio_exports_reuse_scanned_readers_without_reindexing() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
    for rel in [
        "audio/aac-stereo.mka",
        "audio/pcm-int.mkv",
        "playback-errors/alac-resample-window.mka",
        "audio/aac-native-edit.m4a",
        "playback-errors/alac-resample-window.m4a",
        "audio/pcm-screen.mov",
    ] {
        let source = root.join(rel);
        let stem = source.file_stem().unwrap().to_string_lossy();
        let out_default = std::env::temp_dir().join(format!(
            "fvid-reader-reuse-default-{stem}-{}.wav",
            std::process::id()
        ));
        let out_admitted = std::env::temp_dir().join(format!(
            "fvid-reader-reuse-admitted-{stem}-{}.wav",
            std::process::id()
        ));
        let _ = std::fs::remove_file(&out_default);
        let _ = std::fs::remove_file(&out_admitted);
        let admitted = CopyOptions {
            max_controlled_bytes: Some(32 * 1024 * 1024),
            ..Default::default()
        };
        let stats_default = decode_audio(&source, &out_default, &CopyOptions::default()).unwrap();
        let stats_admitted = decode_audio(&source, &out_admitted, &admitted).unwrap();
        assert_eq!(stats_default.sample_frames, stats_admitted.sample_frames);
        assert_eq!(
            std::fs::read(&out_default).unwrap(),
            std::fs::read(&out_admitted).unwrap()
        );
        std::fs::remove_file(&out_default).unwrap();
        std::fs::remove_file(&out_admitted).unwrap();

        let loud_default = measure_loudness(&source, &CopyOptions::default()).unwrap();
        let loud_admitted = measure_loudness(&source, &admitted).unwrap();
        assert_eq!(loud_default.sample_frames, loud_admitted.sample_frames);
        assert_eq!(loud_default.integrated_lufs, loud_admitted.integrated_lufs);
    }
}

#[test]
fn mp4_and_g711_descriptors_reuse_open_file_handle_across_planning_and_loudness() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
    let mp4_source = root.join("audio/aac-native-edit.m4a");
    let transform = AudioDecodeTransform {
        sample_rate: Some(48_000),
        channels: Some(2),
        ..Default::default()
    };
    let options = CopyOptions {
        max_controlled_bytes: Some(32 * 1024 * 1024),
        ..Default::default()
    };
    let plan = plan_decode_audio(&mp4_source, &transform, &options).unwrap();
    assert_eq!(plan.command, "decode-audio");
    let out_mp4 = std::env::temp_dir().join(format!(
        "fvid-mp4-transform-reuse-{}.wav",
        std::process::id()
    ));
    let _ = std::fs::remove_file(&out_mp4);
    let stats = decode_audio_transformed(&mp4_source, &out_mp4, transform, &options).unwrap();
    assert!(stats.sample_frames > 0);
    std::fs::remove_file(&out_mp4).unwrap();

    for wave_rel in [
        "playback-errors/media-trim-loudness/g711-alaw-48k-450ms.wav",
        "playback-errors/loudness-normalize-budget-parity/stereo-48k.wav",
    ] {
        let wave_source = root.join(wave_rel);
        let wave_plan = plan_decode_audio(&wave_source, &Default::default(), &options).unwrap();
        assert_eq!(wave_plan.command, "decode-audio");
        let loud = measure_loudness(&wave_source, &options).unwrap();
        assert!(loud.sample_frames > 0);
    }
    let out_ln = std::env::temp_dir().join(format!(
        "fvid-wave-plan-reuse-ln-{}.wav",
        std::process::id()
    ));
    let _ = std::fs::remove_file(&out_ln);
    let ln = apply_loudnorm(&mp4_source, &out_ln, None, &options).unwrap();
    assert!(ln.sample_frames > 0);
    std::fs::remove_file(&out_ln).unwrap();
}
