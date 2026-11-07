use std::path::PathBuf;

struct TempDir(PathBuf);
impl TempDir {
    fn new(name: &str) -> Self {
        let path = std::env::temp_dir().join(format!(
            "fvid-batch635-{}-{}-{}",
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
fn decode_audio_matroska_export_and_mix_avoid_redundant_scans() {
    let dir = TempDir::new("scans");
    let wav = dir.0.join("tone.wav");
    write_wav(&wav, 48_000, &vec![4000i16; 12_000]);

    let mka = dir.0.join("tone.mka");
    fvid::native_export::export_audio_pcm_selected(
        &wav, &mka, None, 1.0, None, None, None, None, None,
    )
    .unwrap();

    // FVID-0905: uses_native_audio_route with single explicit stream
    let mut stream_opts = fvid::media::CopyOptions::default();
    stream_opts.streams = vec![0];
    let plan = fvid::media::plan_decode_audio(
        &mka,
        &fvid::media_info::AudioDecodeTransform::default(),
        &stream_opts,
    )
    .unwrap();
    assert_eq!(plan.command, "decode-audio");

    // FVID-0906: export_pcm_selected from Matroska to WAV
    let out_wav = dir.0.join("out.wav");
    let stats_wav = fvid::media::decode_audio(&mka, &out_wav, &stream_opts).unwrap();
    assert_eq!(stats_wav.sample_frames, 12_000);

    // FVID-0907: export_pcm_selected from Matroska to Matroska
    let out_mka = dir.0.join("out.mka");
    let stats_mka = fvid::media::decode_audio(&mka, &out_mka, &stream_opts).unwrap();
    assert_eq!(stats_mka.sample_frames, 12_000);

    // FVID-0908: is_owned_audio_source_with_interval_and_options on Matroska PCM
    assert!(
        fvid::native_media::is_owned_audio_source_with_options(
            &mka,
            &fvid::media::CopyOptions::default()
        )
        .unwrap()
    );

    // FVID-0909: validate_and_inspect_source_with_options in mix_audio
    let mix_out = dir.0.join("mix.wav");
    let mix_stats = fvid::media::mix_audio(
        &[mka.clone(), out_mka],
        &mix_out,
        &fvid::media_info::MixAudioOptions::default(),
    )
    .unwrap();
    assert_eq!(mix_stats.sample_frames, 12_000);
}
