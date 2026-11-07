use fvid_control::CopyOptions;
use fvid_media::{decode_audio_transformed, plan_decode_audio};
use fvid_media_info::AudioDecodeTransform;
use std::path::Path;

fn unique_temp_dir(label: &str) -> std::path::PathBuf {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir = std::env::temp_dir().join(format!(
        "fvid-batch599-{}-{}-{}",
        label,
        std::process::id(),
        nanos
    ));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn ogg_audio_export_descriptor_and_matroska_mp4_adts_single_pass_info_succeed() {
    let dir = unique_temp_dir("single-pass");
    for (index, rel) in [
        "tests/fixtures/playback-errors/wave-ogg-flac-plan/ogg-vorbis-stereo.ogg",
        "tests/fixtures/playback-errors/wave-ogg-flac-plan/ogg-opus-stereo.ogg",
        "tests/fixtures/playback-errors/container-loudness-plan/flac-loudness.mka",
        "tests/fixtures/playback-errors/alac-resample-window.mka",
        "tests/fixtures/audio/aac-stereo.aac",
    ]
    .into_iter()
    .enumerate()
    {
        let src = Path::new(rel);
        let out = dir.join(format!("out-{index}.wav"));
        let opts = CopyOptions {
            max_controlled_bytes: Some(64 * 1024 * 1024),
            ..CopyOptions::default()
        };
        let plan =
            plan_decode_audio(src, &AudioDecodeTransform::default(), &opts).expect("plan decode");
        assert_eq!(plan.streams.len(), 1);
        let stats = decode_audio_transformed(
            src,
            &out,
            AudioDecodeTransform::default(),
            &opts,
        )
        .expect("decode audio");
        assert!(stats.sample_frames > 0);
    }
    let _ = std::fs::remove_dir_all(&dir);
}
