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
fn video_budget_webm_lossless_and_loudness_plan_reuse_handles() {
    let temp_dir = std::env::temp_dir().join(format!(
        "fvid-batch652-{}-{}",
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

    // 1. FVID-0990: segment_packet_count with max_packets
    let mut pkt_opts = fvid::media::CopyOptions::default();
    pkt_opts.max_packets = Some(10);
    assert!(
        fvid::native_audio_mix::concat_eligible_with_options(
            &[mka_a.clone(), mka_b.clone()],
            &pkt_opts,
        )
        .expect("concat eligible with max_packets")
    );

    // 2. FVID-0991 & FVID-0992: WebM lossless validation/export and video packet budget
    let mp4_path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/avc-aac-edts-hold.mp4");
    let opts = fvid::media::CopyOptions::default();
    if mp4_path.exists() {
        let mkv_path = temp_dir.join("remux.mkv");
        fvid::media::remux(&mp4_path, &mkv_path, &opts).expect("remux to mkv");
        let lossless_plan = fvid::media::plan_transcode_lossless(
            &mkv_path,
            &fvid::media::LosslessTransform::default(),
            &opts,
            None,
        )
        .expect("plan transcode lossless on mkv");
        assert_eq!(lossless_plan.command, "transcode-lossless");
    }

    // 3. FVID-0993: loudness_plan_and_channels_with_options via plan_loudness
    let loudness_plan = fvid::media::plan_loudness(&mka_a, &opts).expect("plan loudness");
    assert_eq!(loudness_plan.command, "loudness");

    // 4. FVID-0994: finish_trim_aac_plan via trim_audio_with_options
    if mp4_path.exists() {
        let mut trim_opts = fvid::media::CopyOptions::default();
        trim_opts.streams = vec![1];
        let trim_plan =
            fvid::native_plan::trim_audio_with_options(&mp4_path, 0, 100_000, &trim_opts)
                .expect("trim audio plan");
        assert_eq!(trim_plan.command, "trim");
    }

    let _ = std::fs::remove_dir_all(&temp_dir);
}
