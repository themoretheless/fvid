use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT_ID: AtomicU64 = AtomicU64::new(0);

fn temp_path(name: &str) -> std::path::PathBuf {
    std::env::temp_dir().join(format!(
        "fvid-batch623-{}-{}-{}",
        std::process::id(),
        NEXT_ID.fetch_add(1, Ordering::Relaxed),
        name
    ))
}

#[test]
fn loudness_measure_and_default_weights_drop_probe_handles_cleanly() {
    let mp3 = Path::new("tests/fixtures/playback-errors/wave-ogg-flac-plan/raw-stereo.mp3");
    let options = fvid::media::CopyOptions::default();
    let weights = fvid::native_pcm::loudness_channel_weights_with_options(mp3, &options)
        .expect("default weights for stereo MP3");
    assert_eq!(weights, vec![1.0, 1.0]);
    let report = fvid::native_pcm::measure_loudness_file_with_options(mp3, &weights, &options)
        .expect("loudness measurement for stereo MP3");
    assert!(report.sample_frames > 0);
}

#[test]
fn owned_audio_plan_and_loudnorm_plan_reuse_single_descriptor_pass() {
    let mp3 = Path::new("tests/fixtures/playback-errors/wave-ogg-flac-plan/raw-stereo.mp3");
    let mka = Path::new("tests/fixtures/playback-errors/alac-resample-window.mka");
    let wav = Path::new("tests/fixtures/playback-errors/wave-ogg-flac-plan/wave-24in32-valid.wav");
    let options = fvid::media::CopyOptions::default();

    for source in [mp3, mka, wav] {
        let decode_plan = fvid::media::plan_decode_audio(
            source,
            &Default::default(),
            &options,
        )
        .expect("decode-audio plan");
        assert_eq!(decode_plan.command, "decode-audio");
    }

    for source in [mp3, mka] {
        let loudnorm_plan = fvid::native_loudnorm::try_plan(source, None, false, &options)
            .expect("container loudnorm plan")
            .expect("owned loudnorm plan");
        assert_eq!(loudnorm_plan.command, "loudnorm");
    }
}

#[test]
fn owned_audio_export_matroska_codec_dispatch_and_normalize_kind_reuse() {
    let mka = Path::new("tests/fixtures/playback-errors/alac-resample-window.mka");
    let mka_out = temp_path("alac_decoded.wav");
    let wav_in = temp_path("tone_500ms.wav");
    let norm_out = temp_path("normalized.wav");
    let _ = std::fs::remove_file(&mka_out);
    let _ = std::fs::remove_file(&wav_in);
    let _ = std::fs::remove_file(&norm_out);
    let options = fvid::media::CopyOptions::default();

    let stats = fvid::media::decode_audio(
        mka,
        &mka_out,
        &options,
    )
    .expect("decode Matroska ALAC via single-open codec dispatch");
    assert!(stats.sample_frames > 0);
    let _ = std::fs::remove_file(&mka_out);

    let sample_rate = 8000u32;
    let frames = 4000usize; // 500 ms
    let data_bytes = (frames * 2) as u32;
    let mut wav = Vec::with_capacity(44 + data_bytes as usize);
    wav.extend_from_slice(b"RIFF");
    wav.extend_from_slice(&(36 + data_bytes).to_le_bytes());
    wav.extend_from_slice(b"WAVEfmt ");
    wav.extend_from_slice(&16u32.to_le_bytes());
    wav.extend_from_slice(&1u16.to_le_bytes()); // PCM
    wav.extend_from_slice(&1u16.to_le_bytes()); // 1 channel
    wav.extend_from_slice(&sample_rate.to_le_bytes());
    wav.extend_from_slice(&(sample_rate * 2).to_le_bytes());
    wav.extend_from_slice(&2u16.to_le_bytes());
    wav.extend_from_slice(&16u16.to_le_bytes());
    wav.extend_from_slice(b"data");
    wav.extend_from_slice(&data_bytes.to_le_bytes());
    for i in 0..frames {
        let s: i16 = if (i / 10) % 2 == 0 { 8000 } else { -8000 };
        wav.extend_from_slice(&s.to_le_bytes());
    }
    std::fs::write(&wav_in, &wav).expect("write synthetic 500ms wav");

    let report = fvid::native_pcm::normalize_loudness_file_with_options(
        &wav_in,
        &norm_out,
        &[1.0],
        fvid::media_info::NormalizeTarget {
            integrated_lufs: -16.0,
            sample_peak_dbfs: -1.0,
        },
        &options,
        None,
    )
    .expect("normalize WAVE reusing native_measured flag");
    assert_eq!(report.sample_frames, frames as u64);
    let _ = std::fs::remove_file(&wav_in);
    let _ = std::fs::remove_file(&norm_out);
}
