use std::path::PathBuf;

fn temp_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "fvid-batch606-{name}-{}-{}",
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
fn export_audio_pcm_wave_reuses_reader_and_batch_writes_samples() {
    let dir = temp_dir("wave-export");
    let source = PathBuf::from("tests/fixtures/playback-errors/wave-ogg-flac-plan/wave-24in32-valid.wav");
    let dest = dir.join("out.wav");
    let mut options = fvid::media::CopyOptions::default();
    options.max_controlled_bytes = Some(8 * 1024 * 1024);
    let stats = fvid::native_export::export_audio_pcm_transformed_with_controls(
        &source,
        &dest,
        None,
        1.0,
        None,
        None,
        &options,
    )
    .expect("WAVE export with controlled budget should succeed");
    assert_eq!(stats.channels, 1);
    assert!(stats.sample_frames > 0);
    assert!(dest.is_file());

    // Also test float32 WAVE fast path through decode_reader
    let dest2 = dir.join("out2.wav");
    let stats2 = fvid::native_export::export_audio_pcm_transformed_with_controls(
        &dest,
        &dest2,
        None,
        1.0,
        None,
        None,
        &options,
    )
    .expect("float32 WAVE export should succeed");
    assert_eq!(stats2.sample_frames, stats.sample_frames);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn export_y4m_pipeline_reuses_reader_and_validates_budget_once_for_single_source() {
    let dir = temp_dir("y4m-export");
    let y4m_src = dir.join("input.y4m");
    let mut raw = Vec::new();
    raw.extend_from_slice(b"YUV4MPEG2 W4 H4 F25:1 Ip A1:1 C420\nFRAME\n");
    raw.extend_from_slice(&[128u8; 24]);
    std::fs::write(&y4m_src, &raw).unwrap();

    let dest = dir.join("output.y4m");
    let mut options = fvid::media::CopyOptions::default();
    options.max_controlled_bytes = Some(4 * 1024 * 1024);
    let event = fvid::native_export::export_y4m_pipeline_with_options(
        &y4m_src,
        &dest,
        None,
        &Default::default(),
        &Default::default(),
        &options,
    )
    .expect("export_y4m_pipeline_with_options should succeed");
    assert_eq!(event.packets, 1);
    assert!(dest.is_file());
    let _ = std::fs::remove_dir_all(&dir);
}
