use fvid::media::CopyOptions;
use std::path::PathBuf;

fn write_wav(path: &std::path::Path, sample_rate: u32, samples: &[i16]) {
    let data_bytes = (samples.len() * 2) as u32;
    let mut bytes = Vec::with_capacity(44 + data_bytes as usize);
    bytes.extend_from_slice(b"RIFF");
    bytes.extend_from_slice(&(36 + data_bytes).to_le_bytes());
    bytes.extend_from_slice(b"WAVEfmt ");
    bytes.extend_from_slice(&16u32.to_le_bytes());
    bytes.extend_from_slice(&1u16.to_le_bytes());
    bytes.extend_from_slice(&1u16.to_le_bytes());
    bytes.extend_from_slice(&sample_rate.to_le_bytes());
    bytes.extend_from_slice(&(sample_rate * 2).to_le_bytes());
    bytes.extend_from_slice(&2u16.to_le_bytes());
    bytes.extend_from_slice(&16u16.to_le_bytes());
    bytes.extend_from_slice(b"data");
    bytes.extend_from_slice(&data_bytes.to_le_bytes());
    for &sample in samples {
        bytes.extend_from_slice(&sample.to_le_bytes());
    }
    std::fs::write(path, bytes).expect("write wav");
}

#[test]
fn trim_plan_wave_and_concat_plan_reuse_audio_info() {
    let dir = std::env::temp_dir().join(format!(
        "fvid-batch638-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create temp dir");

    let wav_a = dir.join("tone-a.wav");
    let mka_a = dir.join("tone-a.mka");
    let mka_b = dir.join("tone-b.mka");
    let samples: Vec<i16> = (0..48_000)
        .map(|i| if (i / 50) % 2 == 0 { 6_000 } else { -6_000 })
        .collect();
    write_wav(&wav_a, 48_000, &samples);
    fvid::native_export::export_audio_pcm_selected(
        &wav_a, &mka_a, None, 1.0, None, None, None, None, None,
    )
    .expect("export mka_a");
    fvid::native_export::export_audio_pcm_selected(
        &wav_a, &mka_b, None, 1.0, None, None, None, None, None,
    )
    .expect("export mka_b");

    // 1. FVID-0920: plan_trim with single selected stream reuses AudioSourceInfo
    let mut selected_opts = CopyOptions::default();
    selected_opts.streams = vec![0];
    let trim_plan = fvid::media::plan_trim(&mka_a, 0, 500_000, &selected_opts)
        .expect("plan_trim with explicit stream 0");
    assert_eq!(trim_plan.command, "trim");
    assert_eq!(trim_plan.streams.len(), 1);
    assert_eq!(trim_plan.streams[0].index, 0);

    // 2. FVID-0921 & FVID-0922: plan_concat and concat with default options on Matroska audio
    let default_opts = CopyOptions::default();
    let inputs: Vec<PathBuf> = vec![mka_a.clone(), mka_b.clone()];
    let concat_plan = fvid::media::plan_concat(&inputs, &default_opts)
        .expect("plan_concat on mka inputs");
    assert_eq!(concat_plan.command, "concat");
    assert_eq!(concat_plan.streams.len(), 1);
    assert_eq!(concat_plan.streams[0].disposition, "decode_to_pcm");

    let concat_out = dir.join("concat.mka");
    let concat_stats = fvid::media::concat(&inputs, &concat_out, &default_opts)
        .expect("concat on mka inputs");
    assert_eq!(concat_stats.backend, "fvid");
    assert!(concat_out.exists());

    // 3. FVID-0923: trim_audio_wave single-pass constant audio source inspection
    let trim_wav = dir.join("trim-wave.wav");
    let wave_stats = fvid::native_export::trim_audio_wave(
        &mka_a,
        &trim_wav,
        0,
        250_000,
        Some(0),
        None,
        None,
    )
    .expect("trim_audio_wave on mka_a");
    assert_eq!(wave_stats.sample_rate, 48_000);
    assert!(trim_wav.exists());

    // 4. FVID-0924: uses_native_audio_route closes Mp4Reader and checks single-stream audio handler
    let mp4_path = PathBuf::from("tests/fixtures/playback-errors/valid-baseline-16x16.mp4");
    let mut invalid_stream_opts = CopyOptions::default();
    invalid_stream_opts.streams = vec![0]; // stream 0 is video in valid-baseline-16x16.mp4
    assert!(
        fvid::media::plan_decode_audio(&mp4_path, &Default::default(), &invalid_stream_opts)
            .is_err()
    );

    let _ = std::fs::remove_dir_all(&dir);
}
