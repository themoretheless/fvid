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
fn final_concat_metadata_video_selection_and_trim_handle_reuse_work() {
    let temp_dir = std::env::temp_dir().join(format!(
        "fvid-batch653-654-{}-{}",
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

    // 1. FVID-0995: validate_concat_segments_with_options skips redundant active-segment re-probe
    let mut concat_opts = fvid::media::CopyOptions::default();
    concat_opts.max_controlled_bytes = Some(16 * 1024 * 1024);
    fvid::native_audio_mix::validate_concat_segments_with_options(
        &[mka_a.clone(), mka_b.clone()],
        &concat_opts,
    )
    .expect("validate concat segments");

    // 2. FVID-0996: extract_source_wave_info_and_base & extract_source_pcm_metadata_from_reader
    let mix_plan = fvid::native_audio_mix::plan_with_controls(
        &[mka_a.clone(), mka_b.clone()],
        &fvid::native_audio_mix::MixAudioOptions::default(),
        &concat_opts,
    )
    .expect("mix plan with memory budget");
    assert_eq!(mix_plan.command, "mix-audio");

    // 3. FVID-0997, FVID-0998, FVID-0999, FVID-1000:
    //    validate_video_selection_with_options, audio_stream_index_from_metadata,
    //    is_owned_audio_trim_source, and trim_aac_with_options
    let mp4_path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/avc-aac-edts-hold.mp4");
    if mp4_path.exists() {
        let mut video_opts = fvid::media::CopyOptions::default();
        video_opts.streams = vec![0];
        let y4m_plan =
            fvid::native_plan::trim_y4m_with_options(&mp4_path, 0, 100_000, &video_opts)
                .expect("trim y4m plan");
        assert_eq!(y4m_plan.command, "trim");

        assert!(
            fvid::native_media::is_owned_audio_trim_source_with_options(
                &mp4_path,
                &fvid::media::CopyOptions::default(),
            )
            .expect("is_owned_audio_trim_source_with_options")
        );

        let mut aac_opts = fvid::media::CopyOptions::default();
        aac_opts.streams = vec![1];
        let aac_plan =
            fvid::native_plan::trim_aac_with_options(&mp4_path, 0, 100_000, &aac_opts)
                .expect("trim aac plan");
        assert_eq!(aac_plan.command, "trim");
    }

    let _ = std::fs::remove_dir_all(&temp_dir);
}
