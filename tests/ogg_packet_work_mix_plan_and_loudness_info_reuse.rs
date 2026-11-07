use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT_ID: AtomicU64 = AtomicU64::new(0);

fn temp_path(name: &str) -> std::path::PathBuf {
    std::env::temp_dir().join(format!(
        "fvid-batch625-{}-{}-{}",
        std::process::id(),
        NEXT_ID.fetch_add(1, Ordering::Relaxed),
        name
    ))
}

#[test]
fn standalone_packet_work_and_audio_mix_plan_single_open() {
    let mp3 = std::path::PathBuf::from("tests/fixtures/playback-errors/wave-ogg-flac-plan/raw-stereo.mp3");
    let options = fvid::media::CopyOptions::default();

    let concat_plan = fvid::media::plan_concat(&[mp3.clone(), mp3.clone()], &options)
        .expect("concat plan on MP3 inputs exercising standalone_packet_work");
    assert_eq!(concat_plan.command, "concat");

    let float_wav1 = temp_path("mix_plan_float1.wav");
    let float_wav2 = temp_path("mix_plan_float2.wav");
    let _ = std::fs::remove_file(&float_wav1);
    let _ = std::fs::remove_file(&float_wav2);

    let frames = 64u32;
    let data_bytes = frames * 4;
    let riff_size = 36 + data_bytes;
    for path in [&float_wav1, &float_wav2] {
        let mut wav = Vec::with_capacity((44 + data_bytes) as usize);
        wav.extend_from_slice(b"RIFF");
        wav.extend_from_slice(&riff_size.to_le_bytes());
        wav.extend_from_slice(b"WAVEfmt ");
        wav.extend_from_slice(&16u32.to_le_bytes());
        wav.extend_from_slice(&3u16.to_le_bytes());
        wav.extend_from_slice(&1u16.to_le_bytes());
        wav.extend_from_slice(&8000u32.to_le_bytes());
        wav.extend_from_slice(&32000u32.to_le_bytes());
        wav.extend_from_slice(&4u16.to_le_bytes());
        wav.extend_from_slice(&32u16.to_le_bytes());
        wav.extend_from_slice(b"data");
        wav.extend_from_slice(&data_bytes.to_le_bytes());
        for i in 0..frames {
            let sample = if i % 2 == 0 { 0.25f32 } else { -0.25f32 };
            wav.extend_from_slice(&sample.to_le_bytes());
        }
        std::fs::write(path, &wav).expect("write float wav");
    }

    let mix_plan = fvid::media::plan_mix_audio_with_controls(
        &[float_wav1.clone(), float_wav2.clone()],
        &fvid::media::MixAudioOptions::default(),
        &options,
    )
    .expect("plan_mix_audio_with_controls on float WAVE");
    assert_eq!(mix_plan.command, "mix-audio");

    let merge_plan = fvid::media::plan_merge_audio_with_controls(
        &[float_wav1.clone(), float_wav2.clone()],
        &options,
    )
    .expect("plan_merge_audio_with_controls on float WAVE");
    assert_eq!(merge_plan.command, "merge-audio");

    let _ = std::fs::remove_file(&float_wav1);
    let _ = std::fs::remove_file(&float_wav2);
}

#[test]
fn container_loudness_plan_and_loudnorm_reuse_precomputed_metadata() {
    let m4a = Path::new("tests/fixtures/playback-errors/alac-resample-window.m4a");
    let mka = Path::new("tests/fixtures/playback-errors/alac-resample-window.mka");
    let options = fvid::media::CopyOptions::default();

    for source in [m4a, mka] {
        let plan = fvid::media::plan_loudness(source, &options)
            .expect("single-pass container plan_loudness");
        assert_eq!(plan.command, "loudness");

        let report = fvid::media::measure_loudness(source, &options)
            .expect("container measure_loudness with precomputed RIFF INFO");
        assert!(report.sample_frames > 0);
    }
}
