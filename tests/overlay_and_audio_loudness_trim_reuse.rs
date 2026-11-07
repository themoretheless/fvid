use std::path::PathBuf;

struct TempDir(PathBuf);
impl TempDir {
    fn new(name: &str) -> Self {
        let path = std::env::temp_dir().join(format!(
            "fvid-batch632-{}-{}-{}",
            name,
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&path).unwrap();
        Self(path)
    }
}
impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn write_minimal_wav(path: &std::path::Path) {
    let sample_rate = 8000u32;
    let channels = 1u16;
    let bits_per_sample = 16u16;
    let block_align = channels * (bits_per_sample / 8);
    let byte_rate = sample_rate * u32::from(block_align);
    let num_samples = 4000u32;
    let data_size = num_samples * u32::from(block_align);
    let riff_size = 36 + data_size;
    let mut bytes = Vec::with_capacity((44 + data_size) as usize);
    bytes.extend_from_slice(b"RIFF");
    bytes.extend_from_slice(&riff_size.to_le_bytes());
    bytes.extend_from_slice(b"WAVEfmt ");
    bytes.extend_from_slice(&16u32.to_le_bytes());
    bytes.extend_from_slice(&1u16.to_le_bytes());
    bytes.extend_from_slice(&channels.to_le_bytes());
    bytes.extend_from_slice(&sample_rate.to_le_bytes());
    bytes.extend_from_slice(&byte_rate.to_le_bytes());
    bytes.extend_from_slice(&block_align.to_le_bytes());
    bytes.extend_from_slice(&bits_per_sample.to_le_bytes());
    bytes.extend_from_slice(b"data");
    bytes.extend_from_slice(&data_size.to_le_bytes());
    for i in 0..num_samples {
        let sample: i16 = if i % 2 == 0 { 8000 } else { -8000 };
        bytes.extend_from_slice(&sample.to_le_bytes());
    }
    std::fs::write(path, bytes).unwrap();
}

#[test]
fn overlay_and_audio_loudness_trim_reuse_inspected_descriptors() {
    let dir = TempDir::new("reuse");
    let wav_in = dir.0.join("in.wav");
    write_minimal_wav(&wav_in);
    let mka_mid = dir.0.join("mid.mka");
    fvid::native_export::export_audio_pcm_selected(
        &wav_in, &mka_mid, None, 1.0, None, None, None, None, None,
    )
    .unwrap();

    // FVID-0891 + FVID-0892: loudness & normalize with max_controlled_bytes on Matroska
    let mut controlled_opts = fvid::media::CopyOptions::default();
    controlled_opts.max_controlled_bytes = Some(4 * 1024 * 1024);
    let measured =
        fvid::native_pcm::measure_loudness_file_with_options(&mka_mid, &[1.0], &controlled_opts)
            .unwrap();
    assert!(measured.sample_frames > 0);

    let norm_out = dir.0.join("norm.mka");
    let norm_report = fvid::native_pcm::normalize_loudness_file_with_options(
        &mka_mid,
        &norm_out,
        &[1.0],
        fvid::native_pcm::NormalizeTarget::default(),
        &controlled_opts,
        None,
    )
    .unwrap();
    assert_eq!(norm_report.sample_frames, 4000);

    // FVID-0893: default_weights_with_options with custom max_packet_bytes
    let mut pkt_opts = fvid::media::CopyOptions::default();
    pkt_opts.max_packet_bytes = 64 * 1024;
    let weights =
        fvid::native_pcm::loudness_channel_weights_with_options(&mka_mid, &pkt_opts).unwrap();
    assert_eq!(weights, vec![1.0]);

    // FVID-0894: trim_audio_with_options reuses AudioSourceInfo
    let trim_plan = fvid::native_plan::trim_audio_with_options(&mka_mid, 0, 250_000, &pkt_opts).unwrap();
    assert_eq!(trim_plan.command, "trim");

    // FVID-0890: plan_overlay and overlay_video reuse opened LosslessSource
    let y4m_path = dir.0.join("main.y4m");
    let mut y4m = b"YUV4MPEG2 W2 H2 F1:1 Ip A1:1 C420\n".to_vec();
    for _ in 0..2 {
        y4m.extend_from_slice(b"FRAME\n");
        y4m.extend_from_slice(&[64, 64, 64, 64, 128, 128]);
    }
    std::fs::write(&y4m_path, y4m).unwrap();
    let overlay_plan = fvid::media::plan_overlay(
        &y4m_path,
        &y4m_path,
        0,
        0,
        &fvid::media::CopyOptions::default(),
    )
    .unwrap();
    assert_eq!(overlay_plan.command, "overlay");
    let overlay_out = dir.0.join("overlay.mkv");
    let overlay_stats = fvid::media::overlay_video(
        &y4m_path,
        &y4m_path,
        &overlay_out,
        0,
        0,
        &fvid::media::CopyOptions::default(),
    )
    .unwrap();
    assert_eq!(overlay_stats.video_frames, 2);
}
