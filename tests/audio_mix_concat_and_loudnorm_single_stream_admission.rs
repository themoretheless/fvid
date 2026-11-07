use std::path::PathBuf;

struct TempDir(PathBuf);
impl TempDir {
    fn new(name: &str) -> Self {
        let path = std::env::temp_dir().join(format!(
            "fvid-batch634-{}-{}-{}",
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
fn audio_mix_concat_and_loudnorm_single_stream_admission() {
    let dir = TempDir::new("admission");
    let wav = dir.0.join("tone.wav");
    write_wav(&wav, 48_000, &vec![3000i16; 24_000]);

    let mka = dir.0.join("tone.mka");
    fvid::native_export::export_audio_pcm_selected(
        &wav, &mka, None, 1.0, None, None, None, None, None,
    )
    .unwrap();

    // FVID-0900: eligible_with_options with custom max_packet_bytes
    let mut pkt_opts = fvid::media::CopyOptions::default();
    pkt_opts.max_packet_bytes = 64 * 1024;
    assert!(
        fvid::native_audio_mix::eligible_with_options(
            &[mka.clone(), mka.clone()],
            &pkt_opts
        )
        .unwrap()
    );

    // FVID-0901: concat_audio_with_options and validate_concat_segments_with_options
    assert!(
        fvid::native_audio_mix::concat_eligible_with_options(
            &[mka.clone(), mka.clone()],
            &pkt_opts
        )
        .unwrap()
    );
    let concat_out = dir.0.join("concat.wav");
    let concat_stats = fvid::native_audio_mix::concat_audio_with_options(
        &[mka.clone(), mka.clone()],
        &concat_out,
        &pkt_opts,
    )
    .unwrap();
    assert_eq!(concat_stats.sample_frames, 48_000);

    // FVID-0902: is_owned_audio_trim_source_with_interval_and_options
    assert!(
        fvid::native_media::is_owned_audio_trim_source_with_options(&mka, &pkt_opts)
            .unwrap()
    );

    // FVID-0903 & FVID-0904: loudnorm single-stream selection
    let mut stream_opts = fvid::media::CopyOptions::default();
    stream_opts.streams = vec![0];
    let plan = fvid::media::plan_loudnorm(&mka, None, false, &stream_opts).unwrap();
    assert_eq!(plan.command, "loudnorm");

    let loudnorm_out = dir.0.join("loudnorm.wav");
    let stats =
        fvid::media::apply_loudnorm(&mka, &loudnorm_out, None, &stream_opts).unwrap();
    assert_eq!(stats.sample_frames, 96_000);
}
