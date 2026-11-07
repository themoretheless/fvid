use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT_ID: AtomicU64 = AtomicU64::new(0);

fn temp_path(name: &str) -> std::path::PathBuf {
    std::env::temp_dir().join(format!(
        "fvid-batch624-{}-{}-{}",
        std::process::id(),
        NEXT_ID.fetch_add(1, Ordering::Relaxed),
        name
    ))
}

#[test]
fn container_loudness_single_pass_geometry_and_controlled_budget() {
    let m4a = Path::new("tests/fixtures/playback-errors/alac-resample-window.m4a");
    let options = fvid::media::CopyOptions {
        max_controlled_bytes: Some(32 * 1024 * 1024),
        ..Default::default()
    };

    let report = fvid::media::measure_loudness(m4a, &options)
        .expect("MP4 ALAC loudness measurement under controlled budget");
    assert!(report.sample_frames > 0);

    let plan = fvid::media::plan_loudness(m4a, &options)
        .expect("MP4 ALAC loudness plan under controlled budget");
    assert_eq!(plan.command, "loudness");
}

#[test]
fn mp4_and_g711_export_and_float_wave_mix_single_open() {
    let m4a = Path::new("tests/fixtures/playback-errors/alac-resample-window.m4a");
    let m4a_out = temp_path("m4a_decoded.wav");
    let float_wav1 = temp_path("float1.wav");
    let float_wav2 = temp_path("float2.wav");
    let mixed_out = temp_path("mixed.wav");
    let _ = std::fs::remove_file(&m4a_out);
    let _ = std::fs::remove_file(&float_wav1);
    let _ = std::fs::remove_file(&float_wav2);
    let _ = std::fs::remove_file(&mixed_out);
    let options = fvid::media::CopyOptions::default();

    let stats = fvid::media::decode_audio(m4a, &m4a_out, &options)
        .expect("MP4 ALAC single-open export");
    assert!(stats.sample_frames > 0);
    let _ = std::fs::remove_file(&m4a_out);

    // Create two 64-frame float32 mono 8 kHz WAVE files for mix_audio
    let frames = 64u32;
    let data_bytes = frames * 4;
    let riff_size = 36 + data_bytes;
    for (idx, path) in [&float_wav1, &float_wav2].into_iter().enumerate() {
        let mut wav = Vec::with_capacity((44 + data_bytes) as usize);
        wav.extend_from_slice(b"RIFF");
        wav.extend_from_slice(&riff_size.to_le_bytes());
        wav.extend_from_slice(b"WAVEfmt ");
        wav.extend_from_slice(&16u32.to_le_bytes());
        wav.extend_from_slice(&3u16.to_le_bytes()); // IEEE float
        wav.extend_from_slice(&1u16.to_le_bytes());
        wav.extend_from_slice(&8000u32.to_le_bytes());
        wav.extend_from_slice(&32000u32.to_le_bytes());
        wav.extend_from_slice(&4u16.to_le_bytes());
        wav.extend_from_slice(&32u16.to_le_bytes());
        wav.extend_from_slice(b"data");
        wav.extend_from_slice(&data_bytes.to_le_bytes());
        for i in 0..frames {
            let sample = if (i + idx as u32) % 2 == 0 { 0.25f32 } else { -0.25f32 };
            wav.extend_from_slice(&sample.to_le_bytes());
        }
        std::fs::write(path, &wav).expect("write float wav");
    }

    let mix_stats = fvid::media::mix_audio_with_controls(
        &[float_wav1.clone(), float_wav2.clone()],
        &mixed_out,
        &fvid::media::MixAudioOptions::default(),
        &options,
    )
    .expect("mix float32 WAVE files without redundant G.711 pre-probe");
    assert_eq!(mix_stats.sample_frames, u64::from(frames));

    let _ = std::fs::remove_file(&float_wav1);
    let _ = std::fs::remove_file(&float_wav2);
    let _ = std::fs::remove_file(&mixed_out);
}
