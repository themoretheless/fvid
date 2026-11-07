use std::path::PathBuf;

fn temp_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "fvid-batch604-{name}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn decode_fvid_media_audio_to_matroska_checks_budget_once_and_succeeds() {
    let dir = temp_dir("media-budget");
    let source = PathBuf::from("tests/fixtures/playback-errors/wave-ogg-flac-plan/g711-alaw-48k-mono.wav");
    let dest = dir.join("out.mka");
    let mut options = fvid::media::CopyOptions::default();
    options.max_controlled_bytes = Some(16 * 1024 * 1024);
    let stats = fvid::media::decode_audio_transformed(
        &source,
        &dest,
        fvid::media::AudioDecodeTransform::default(),
        &options,
    )
    .expect("controlled Matroska audio decode should succeed");
    assert_eq!(stats.channels, 1);
    assert!(dest.is_file());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn native_loudnorm_to_matroska_with_budget_succeeds() {
    let dir = temp_dir("loudnorm-budget");
    let source = PathBuf::from("tests/fixtures/playback-errors/wave-ogg-flac-plan/wave-24in32-valid.wav");
    let dest = dir.join("norm.mka");
    let mut options = fvid::media::CopyOptions::default();
    options.max_controlled_bytes = Some(8 * 1024 * 1024);
    let stats = fvid::native_loudnorm::try_apply(&source, &dest, None, true, &options)
        .expect("loudnorm to mka with budget should succeed")
        .expect("loudnorm should be supported");
    assert_eq!(stats.channels, 1);
    assert!(dest.is_file());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn mix_and_concat_audio_reuse_inspected_geometry_and_masks() {
    let dir = temp_dir("mix-concat");
    let src = PathBuf::from("tests/fixtures/playback-errors/wave-ogg-flac-plan/wave-24in32-valid.wav");
    let sources = vec![src.clone(), src];
    let mix_out = dir.join("mixed.wav");
    let mix_stats = fvid::native_audio_mix::mix_audio(
        &sources,
        &mix_out,
        &fvid::native_audio_mix::MixAudioOptions::default(),
    )
    .expect("mix_audio should succeed");
    assert_eq!(mix_stats.channels, 1);
    assert!(mix_out.is_file());

    let concat_out = dir.join("concat.wav");
    let concat_stats = fvid::native_audio_mix::concat_audio(&sources, &concat_out, None, None)
        .expect("concat_audio should succeed");
    assert_eq!(concat_stats.channels, 1);
    assert!(concat_out.is_file());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn loudness_measurement_reuses_wave_reader_and_frame_aligned_sink() {
    let src = PathBuf::from("tests/fixtures/playback-errors/wave-ogg-flac-plan/wave-24in32-valid.wav");
    let weights = fvid::native_pcm::loudness_channel_weights(&src, None).expect("weights");
    let report = fvid::native_pcm::measure_loudness_file(&src, None, &weights, None)
        .expect("measure loudness");
    assert!(report.sample_frames > 0);
}
