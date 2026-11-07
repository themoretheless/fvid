use std::fs;
use std::path::PathBuf;

fn temp_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("fvid-batch629-{}-{}", name, std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn concat_mix_and_merge_reuse_budget_metadata_and_plan_preflight() {
    let dir = temp_dir("audio-mix-concat");
    let wav = dir.join("src.wav");
    let mut wav_bytes = Vec::new();
    let sample_rate = 48000u32;
    let channels = 2u16;
    let frames = 64u32;
    let block_align = channels * 4;
    let byte_rate = sample_rate * u32::from(block_align);
    let data_bytes = frames * u32::from(block_align);
    let riff_size = 36 + data_bytes;
    wav_bytes.extend_from_slice(b"RIFF");
    wav_bytes.extend_from_slice(&riff_size.to_le_bytes());
    wav_bytes.extend_from_slice(b"WAVEfmt ");
    wav_bytes.extend_from_slice(&16u32.to_le_bytes());
    wav_bytes.extend_from_slice(&3u16.to_le_bytes());
    wav_bytes.extend_from_slice(&channels.to_le_bytes());
    wav_bytes.extend_from_slice(&sample_rate.to_le_bytes());
    wav_bytes.extend_from_slice(&byte_rate.to_le_bytes());
    wav_bytes.extend_from_slice(&block_align.to_le_bytes());
    wav_bytes.extend_from_slice(&32u16.to_le_bytes());
    wav_bytes.extend_from_slice(b"data");
    wav_bytes.extend_from_slice(&data_bytes.to_le_bytes());
    for i in 0..(frames * u32::from(channels)) {
        let sample = if i % 2 == 0 { 0.25f32 } else { -0.25f32 };
        wav_bytes.extend_from_slice(&sample.to_le_bytes());
    }
    fs::write(&wav, &wav_bytes).unwrap();

    let plan_default = fvid::native_plan::concat_mp4_matroska(&[wav.clone(), wav.clone()])
        .unwrap()
        .expect("sample-domain concat plan");
    assert_eq!(plan_default.command, "concat");

    let mut controlled = fvid::media::CopyOptions::default();
    controlled.max_controlled_bytes = Some(64 * 1024 * 1024);
    controlled.metadata_set.push(("title".into(), "batch629".into()));

    let dst_concat_wav = dir.join("concat.wav");
    let c_stats = fvid::native_audio_mix::concat_audio_with_options(
        &[wav.clone(), wav.clone()],
        &dst_concat_wav,
        &controlled,
    )
    .unwrap();
    assert_eq!(c_stats.sample_frames, 128);

    let dst_concat_mka = dir.join("concat.mka");
    let c_mka_stats = fvid::native_audio_mix::concat_audio_with_options(
        &[wav.clone(), wav.clone()],
        &dst_concat_mka,
        &controlled,
    )
    .unwrap();
    assert_eq!(c_mka_stats.sample_frames, c_stats.sample_frames);

    let mix_opts = fvid::native_audio_mix::MixAudioOptions {
        duration: fvid::native_audio_mix::MixDuration::Shortest,
        weights: vec![1.0, 1.0],
        normalize: true,
    };
    let dst_mix_wav = dir.join("mix.wav");
    let m_stats = fvid::native_audio_mix::mix_audio_with_controls(
        &[wav.clone(), wav.clone()],
        &dst_mix_wav,
        &mix_opts,
        &controlled,
    )
    .unwrap();
    assert_eq!(m_stats.sample_frames, 64);

    let dst_merge_mka = dir.join("merge.mka");
    let mg_stats = fvid::native_audio_mix::merge_audio_with_controls(
        &[wav.clone(), wav.clone()],
        &dst_merge_mka,
        &controlled,
    )
    .unwrap();
    assert_eq!(mg_stats.channels, 4);

    let _ = fs::remove_dir_all(&dir);
}
