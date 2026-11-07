use std::path::PathBuf;

struct TempDir(PathBuf);
impl TempDir {
    fn new(name: &str) -> Self {
        let path = std::env::temp_dir().join(format!(
            "fvid-batch633-{}-{}-{}",
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
    for s in samples {
        bytes.extend_from_slice(&s.to_le_bytes());
    }
    std::fs::write(path, bytes).unwrap();
}

#[test]
fn loudness_plan_and_loudnorm_reuse_inspected_audio_metadata() {
    let dir = TempDir::new("reuse");
    let wav = dir.0.join("tone.wav");
    write_wav(&wav, 48_000, &vec![4096i16; 24_000]);

    let loudness_plan = fvid::native_plan::loudness_with_options(
        &wav,
        &fvid::media::CopyOptions::default(),
        None,
    )
    .unwrap();
    assert_eq!(loudness_plan.command, "loudness");
    assert_eq!(loudness_plan.streams[0].codec, "pcm_s16le");

    let mka = dir.0.join("tone.mka");
    fvid::native_export::export_audio_pcm_selected(
        &wav, &mka, None, 1.0, None, None, None, None, None,
    )
    .unwrap();

    let mka_loudness_plan = fvid::native_plan::loudness_with_options(
        &mka,
        &fvid::media::CopyOptions::default(),
        None,
    )
    .unwrap();
    assert_eq!(mka_loudness_plan.command, "loudness");

    let mut mix_opts = fvid::media::CopyOptions::default();
    mix_opts.max_packets = Some(4);
    let mix_plan = fvid::native_audio_mix::plan_with_controls(
        &[mka.clone(), mka.clone()],
        &fvid::media_info::MixAudioOptions::default(),
        &mix_opts,
    )
    .unwrap();
    assert_eq!(mix_plan.command, "mix-audio");

    let mut loudnorm_opts = fvid::media::CopyOptions::default();
    loudnorm_opts.max_packets = Some(4);
    let loudnorm_plan = fvid::media::plan_loudnorm(
        &mka,
        None,
        false,
        &loudnorm_opts,
    )
    .unwrap();
    assert_eq!(loudnorm_plan.command, "loudnorm");

    let loudnorm_out = dir.0.join("loudnorm.wav");
    let loudnorm_stats = fvid::media::apply_loudnorm(
        &mka,
        &loudnorm_out,
        None,
        &loudnorm_opts,
    )
    .unwrap();
    assert!(loudnorm_stats.sample_frames > 0);

    let mut trim_opts = fvid::media::CopyOptions::default();
    trim_opts.streams = vec![0];
    let trim_plan = fvid::media::plan_trim(&mka, 0, 100_000, &trim_opts).unwrap();
    assert_eq!(trim_plan.command, "trim");

    let trim_out = dir.0.join("trimmed.wav");
    let trim_stats = fvid::media::trim(&mka, &trim_out, 0, 100_000, &trim_opts).unwrap();
    assert!(trim_stats.packets > 0);
}
