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
fn loudnorm_trim_and_mix_spool_reuse_single_stream_audio_info() {
    let dir = std::env::temp_dir().join(format!(
        "fvid-batch637-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create temp dir");

    let wav_a = dir.join("tone-a.wav");
    let mka_a = dir.join("tone-a.mka");
    let mka_b = dir.join("tone-b.mka");
    // 1 second of 48 kHz sine-like alternating samples for loudnorm and trim
    let samples: Vec<i16> = (0..48_000)
        .map(|i| if (i / 50) % 2 == 0 { 8_000 } else { -8_000 })
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

    // 1. Default and single-stream loudnorm planning on Matroska audio
    let default_opts = CopyOptions::default();
    let plan_default = fvid::media::plan_loudnorm(&mka_a, None, false, &default_opts)
        .expect("default loudnorm plan");
    assert_eq!(plan_default.command, "loudnorm");
    assert_eq!(plan_default.streams.len(), 1);
    assert_eq!(plan_default.streams[0].index, 0);

    let mut selected_opts = CopyOptions::default();
    selected_opts.streams = vec![0];
    let plan_selected = fvid::media::plan_loudnorm(&mka_a, None, false, &selected_opts)
        .expect("selected loudnorm plan");
    assert_eq!(plan_selected.streams[0].index, 0);

    // 2. Default loudnorm apply on Matroska audio
    let loudnorm_out = dir.join("loudnorm-out.wav");
    let loudnorm_stats =
        fvid::media::apply_loudnorm(&mka_a, &loudnorm_out, None, &default_opts)
            .expect("apply loudnorm on mka_a");
    assert_eq!(loudnorm_stats.sample_frames, 192_000);
    assert!(loudnorm_out.exists());

    // 3. Single-stream trim with cached AudioSourceInfo
    let trim_out = dir.join("trimmed.wav");
    let trim_stats =
        fvid::media::trim(&mka_a, &trim_out, 0, 250_000, &selected_opts)
            .expect("trim mka_a with explicit stream 0");
    assert_eq!(trim_stats.backend, "fvid");
    assert!(trim_out.exists());

    // 4. Audio mix / concat spooling with pre-resolved stream index
    let concat_out = dir.join("concat.mka");
    let inputs: Vec<PathBuf> = vec![mka_a.clone(), mka_b.clone()];
    let concat_stats =
        fvid::media::concat(&inputs, &concat_out, &default_opts).expect("concat mka inputs");
    assert_eq!(concat_stats.backend, "fvid");
    assert!(concat_out.exists());

    let _ = std::fs::remove_dir_all(&dir);
}
