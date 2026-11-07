use std::path::Path;

fn write_wav(path: &Path, sample_rate: u32, samples: &[i16]) {
    let data_bytes = (samples.len() * 2) as u32;
    let riff_bytes = 36 + data_bytes;
    let byte_rate = sample_rate * 2;
    let mut bytes = Vec::with_capacity((44 + data_bytes) as usize);
    bytes.extend_from_slice(b"RIFF");
    bytes.extend_from_slice(&riff_bytes.to_le_bytes());
    bytes.extend_from_slice(b"WAVEfmt ");
    bytes.extend_from_slice(&16u32.to_le_bytes());
    bytes.extend_from_slice(&1u16.to_le_bytes());
    bytes.extend_from_slice(&1u16.to_le_bytes());
    bytes.extend_from_slice(&sample_rate.to_le_bytes());
    bytes.extend_from_slice(&byte_rate.to_le_bytes());
    bytes.extend_from_slice(&2u16.to_le_bytes());
    bytes.extend_from_slice(&16u16.to_le_bytes());
    bytes.extend_from_slice(b"data");
    bytes.extend_from_slice(&data_bytes.to_le_bytes());
    for &s in samples {
        bytes.extend_from_slice(&s.to_le_bytes());
    }
    std::fs::write(path, bytes).expect("write wav");
}

#[test]
fn audio_concat_loudness_decode_and_lossless_reuse_handles() {
    let temp_dir = std::env::temp_dir().join(format!(
        "fvid-batch651-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&temp_dir).expect("create temp_dir");
    let wav_a = temp_dir.join("a.wav");
    let wav_b = temp_dir.join("b.wav");
    write_wav(&wav_a, 8000, &[1000i16; 3200]);
    write_wav(&wav_b, 8000, &[1200i16; 3200]);

    let mka_a = temp_dir.join("a.mka");
    let mka_b = temp_dir.join("b.mka");
    fvid::native_export::export_audio_pcm_selected(
        &wav_a, &mka_a, None, 1.0, None, None, None, None, None,
    )
    .expect("export mka_a");
    fvid::native_export::export_audio_pcm_selected(
        &wav_b, &mka_b, None, 1.0, None, None, None, None, None,
    )
    .expect("export mka_b");

    // 1. FVID-0985: native_audio_mix::concat_eligible_with_options & concat_audio_with_options
    let opts = fvid::media::CopyOptions::default();
    assert!(
        fvid::native_audio_mix::concat_eligible_with_options(
            &[mka_a.clone(), mka_b.clone()],
            &opts,
        )
        .expect("concat eligible")
    );
    let concat_out = temp_dir.join("concat.mka");
    let concat_stats = fvid::native_audio_mix::concat_audio_with_options(
        &[mka_a.clone(), mka_b.clone()],
        &concat_out,
        &opts,
    )
    .expect("concat audio");
    assert_eq!(concat_stats.sample_rate, 8000);
    assert_eq!(concat_stats.channels, 1);
    assert_eq!(concat_stats.sample_frames, 6400);

    // 2. FVID-0986: native_pcm::loudness_channel_weights_with_options & measure_loudness_file_with_options
    let weights = fvid::native_pcm::loudness_channel_weights_with_options(&mka_a, &opts)
        .expect("default weights");
    assert_eq!(weights, vec![1.0]);
    let report = fvid::native_pcm::measure_loudness_file_with_options(&mka_a, &weights, &opts)
        .expect("measure loudness");
    assert_eq!(report.sample_frames, 3200);

    // 3. FVID-0987: native_plan::decode_audio_with_options
    let transform = fvid::media::AudioDecodeTransform {
        interval: None,
        sample_rate: Some(16000),
        channels: Some(2),
        volume: Some(1.0),
    };
    let plan = fvid::native_plan::decode_audio_with_options(&mka_a, &transform, &opts)
        .expect("plan decode audio");
    assert_eq!(plan.command, "decode-audio");

    // 4. FVID-0988: pcm_matroska WAVE metadata extraction via decode_audio_transformed
    let mut mka_opts = fvid::media::CopyOptions::default();
    mka_opts.max_controlled_bytes = Some(8 * 1024 * 1024);
    let wav_to_mka = temp_dir.join("wav_to_mka.mka");
    let export_stats = fvid::media::decode_audio_transformed(
        &wav_a,
        &wav_to_mka,
        fvid::media::AudioDecodeTransform::default(),
        &mka_opts,
    )
    .expect("export wav to mka with controlled memory");
    assert_eq!(export_stats.sample_frames, 3200);

    // 5. FVID-0989: native_lossless MP4 validation and transcode_lossless
    let mp4_path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/avc-aac-edts-hold.mp4");
    if mp4_path.exists() {
        let lossless_plan = fvid::media::plan_transcode_lossless(
            &mp4_path,
            &fvid::media::LosslessTransform::default(),
            &opts,
            None,
        )
        .expect("plan transcode lossless");
        assert_eq!(lossless_plan.command, "transcode-lossless");
    }

    let _ = std::fs::remove_dir_all(&temp_dir);
}
