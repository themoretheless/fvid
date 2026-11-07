use fvid_media::{
    CopyOptions, decode_audio, decode_audio_transformed, measure_loudness, plan_decode_audio,
};
use fvid_media_info::AudioDecodeTransform;
use std::path::Path;

#[test]
fn ogg_raw_flac_raw_mp3_and_audio_export_reuse_open_file_handles() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
    let options = CopyOptions {
        max_controlled_bytes: Some(64 * 1024 * 1024),
        ..Default::default()
    };
    for rel in [
        "playback-errors/wave-ogg-flac-plan/ogg-vorbis-chained-5ch-0x37.ogg",
        "playback-errors/wave-ogg-flac-plan/ogg-joint-chained-5ch-0x37.ogg",
        "playback-errors/wave-ogg-flac-plan/flac-4ch-quad.flac",
        "playback-errors/wave-ogg-flac-plan/raw-stereo.mp3",
        "playback-errors/media-trim-loudness/g711-alaw-48k-450ms.wav",
        "playback-errors/loudness-normalize-budget-parity/stereo-48k.wav",
    ] {
        let source = root.join(rel);
        let plan_default =
            plan_decode_audio(&source, &AudioDecodeTransform::default(), &CopyOptions::default())
                .unwrap();
        let plan_admitted =
            plan_decode_audio(&source, &AudioDecodeTransform::default(), &options).unwrap();
        assert_eq!(plan_default.streams, plan_admitted.streams);

        let stem = source.file_stem().unwrap().to_string_lossy();
        let out_default = std::env::temp_dir().join(format!(
            "fvid-b643-default-{stem}-{}.wav",
            std::process::id()
        ));
        let out_admitted = std::env::temp_dir().join(format!(
            "fvid-b643-admitted-{stem}-{}.wav",
            std::process::id()
        ));
        let _ = std::fs::remove_file(&out_default);
        let _ = std::fs::remove_file(&out_admitted);
        let stats_default = decode_audio(&source, &out_default, &CopyOptions::default()).unwrap();
        let stats_admitted = decode_audio(&source, &out_admitted, &options).unwrap();
        assert_eq!(stats_default.sample_frames, stats_admitted.sample_frames);
        assert_eq!(
            std::fs::read(&out_default).unwrap(),
            std::fs::read(&out_admitted).unwrap()
        );
        std::fs::remove_file(&out_default).unwrap();
        std::fs::remove_file(&out_admitted).unwrap();
    }

    for rel in [
        "playback-errors/wave-ogg-flac-plan/raw-stereo.mp3",
        "playback-errors/media-trim-loudness/g711-alaw-48k-450ms.wav",
    ] {
        let source = root.join(rel);
        let loud_default = measure_loudness(&source, &CopyOptions::default()).unwrap();
        let loud_admitted = measure_loudness(&source, &options).unwrap();
        assert_eq!(loud_default.sample_frames, loud_admitted.sample_frames);
        assert_eq!(loud_default.integrated_lufs, loud_admitted.integrated_lufs);
    }

    let mp4_source = root.join("audio/aac-native-edit.m4a");
    let transform = AudioDecodeTransform {
        sample_rate: Some(48_000),
        channels: Some(1),
        ..Default::default()
    };
    let plan = plan_decode_audio(&mp4_source, &transform, &options).unwrap();
    assert_eq!(plan.command, "decode-audio");
    let out_mp4 = std::env::temp_dir().join(format!(
        "fvid-b643-mp4-target-{}.wav",
        std::process::id()
    ));
    let _ = std::fs::remove_file(&out_mp4);
    let stats = decode_audio_transformed(&mp4_source, &out_mp4, transform, &options).unwrap();
    assert_eq!(stats.channels, 1);
    assert_eq!(stats.sample_rate, 48_000);
    std::fs::remove_file(&out_mp4).unwrap();
}
