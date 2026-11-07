use std::path::PathBuf;

fn temp_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "fvid-batch605-{name}-{}-{}",
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
fn normalize_loudness_file_with_controlled_budget_succeeds() {
    let dir = temp_dir("normalize-budget");
    let source = dir.join("tone.wav");
    let sample_rate = 48_000u32;
    let frames = 24_000usize; // 500 ms >= 400 ms
    let data_bytes = (frames * 4) as u32;
    let mut wav = Vec::with_capacity(44 + frames * 4);
    wav.extend_from_slice(b"RIFF");
    wav.extend_from_slice(&(36 + data_bytes).to_le_bytes());
    wav.extend_from_slice(b"WAVEfmt ");
    wav.extend_from_slice(&16u32.to_le_bytes());
    wav.extend_from_slice(&3u16.to_le_bytes()); // IEEE float
    wav.extend_from_slice(&1u16.to_le_bytes()); // mono
    wav.extend_from_slice(&sample_rate.to_le_bytes());
    wav.extend_from_slice(&(sample_rate * 4).to_le_bytes());
    wav.extend_from_slice(&4u16.to_le_bytes());
    wav.extend_from_slice(&32u16.to_le_bytes());
    wav.extend_from_slice(b"data");
    wav.extend_from_slice(&data_bytes.to_le_bytes());
    for i in 0..frames {
        let sample = ((i as f32 * 440.0 * std::f32::consts::TAU) / sample_rate as f32).sin() * 0.25;
        wav.extend_from_slice(&sample.to_le_bytes());
    }
    std::fs::write(&source, &wav).unwrap();

    let dest = dir.join("normalized.mka");
    let mut options = fvid::media::CopyOptions::default();
    options.max_controlled_bytes = Some(8 * 1024 * 1024);
    let weights = fvid::native_pcm::loudness_channel_weights_with_options(&source, &options)
        .expect("default weights should succeed");
    assert_eq!(weights.len(), 1);
    let target = fvid::native_pcm::NormalizeTarget {
        integrated_lufs: -16.0,
        sample_peak_dbfs: -1.0,
    };
    let report = fvid::native_pcm::normalize_loudness_file_with_options(
        &source,
        &dest,
        &weights,
        target,
        &options,
        None,
    )
    .expect("normalize_loudness_file_with_options should succeed");
    assert_eq!(report.sample_frames, frames as u64);
    assert!(dest.is_file());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn native_plan_decode_audio_and_loudness_reuse_wave_reader() {
    let source = PathBuf::from("tests/fixtures/playback-errors/wave-ogg-flac-plan/wave-24in32-valid.wav");
    let options = fvid::media::CopyOptions::default();
    let decode_plan = fvid::native_plan::decode_audio_with_options(
        &source,
        &fvid::native_plan::AudioDecodeTransform::default(),
        &options,
    )
    .expect("decode_audio_with_options should succeed");
    assert_eq!(decode_plan.command, "decode-audio");
    assert_eq!(decode_plan.streams.len(), 1);

    let loud_plan = fvid::native_plan::loudness_with_options(&source, &options, None)
        .expect("loudness_with_options should succeed");
    assert_eq!(loud_plan.command, "loudness");
}
