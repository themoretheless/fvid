use std::path::PathBuf;

struct TempDir(PathBuf);
impl TempDir {
    fn new(name: &str) -> Self {
        let path = std::env::temp_dir().join(format!(
            "fvid-batch631-{}-{}-{}",
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
    let num_samples = 64u32;
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
        let sample: i16 = if i % 2 == 0 { 1000 } else { -1000 };
        bytes.extend_from_slice(&sample.to_le_bytes());
    }
    std::fs::write(path, bytes).unwrap();
}

#[test]
fn matroska_pcm_export_and_lossless_overlay_plan_reuse_opened_readers() {
    let dir = TempDir::new("reuse");
    let wav_in = dir.0.join("in.wav");
    write_minimal_wav(&wav_in);
    let mka_mid = dir.0.join("mid.mka");
    let stats1 = fvid::native_export::export_audio_pcm_selected(
        &wav_in, &mka_mid, None, 1.0, None, None, None, None, None,
    )
    .unwrap();
    assert_eq!(stats1.sample_frames, 64);

    let mka_out = dir.0.join("out.mka");
    let mut opts = fvid::media::CopyOptions::default();
    opts.max_packet_bytes = 64 * 1024;
    let stats2 = fvid::native_export::export_audio_pcm_transformed_with_controls(
        &mka_mid, &mka_out, None, 1.0, None, None, &opts,
    )
    .unwrap();
    assert_eq!(stats2.sample_frames, 64);

    let decode_plan =
        fvid::native_plan::decode_audio_with_options(&mka_mid, &Default::default(), &opts).unwrap();
    assert_eq!(decode_plan.command, "decode-audio");

    let loudness_plan = fvid::native_plan::loudness_with_options(&mka_mid, &opts, None).unwrap();
    assert_eq!(loudness_plan.command, "loudness");

    let y4m_path = dir.0.join("main.y4m");
    let mut y4m = b"YUV4MPEG2 W2 H2 F1:1 Ip A1:1 C420\n".to_vec();
    for _ in 0..2 {
        y4m.extend_from_slice(b"FRAME\n");
        y4m.extend_from_slice(&[64, 64, 64, 64, 128, 128]);
    }
    std::fs::write(&y4m_path, y4m).unwrap();

    let mut transform = fvid::media::LosslessTransform::default();
    transform.horizontal_flip = true;
    transform.negate = Some("1".into());
    transform.overlay = Some(fvid::media::OverlaySpec {
        path: y4m_path.clone(),
        x: 0,
        y: 0,
    });
    let plan = fvid::media::plan_transcode_lossless(
        &y4m_path,
        &transform,
        &fvid::media::CopyOptions::default(),
        Some("ffv1"),
    )
    .unwrap();
    assert_eq!(plan.command, "transcode-lossless");
    assert!(plan.steps.iter().any(|s| s.action == "geometry"));
    assert!(plan.steps.iter().any(|s| s.action == "overlay"));
    assert!(plan.steps.iter().any(|s| s.action == "filter"));

    let out_mkv = dir.0.join("overlay-out.mkv");
    let ov_stats = fvid::media::transcode_lossless(
        &y4m_path,
        &out_mkv,
        transform,
        &fvid::media::CopyOptions::default(),
    )
    .unwrap();
    assert_eq!(ov_stats.video_frames, 2);
}
