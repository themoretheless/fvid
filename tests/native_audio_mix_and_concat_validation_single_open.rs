use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT_ID: AtomicU64 = AtomicU64::new(0);

fn temp_path(name: &str) -> PathBuf {
    std::env::temp_dir().join(format!(
        "fvid-batch626-{}-{}-{}",
        std::process::id(),
        NEXT_ID.fetch_add(1, Ordering::Relaxed),
        name
    ))
}

#[test]
fn native_audio_mix_eligible_validate_and_decode_single_open() {
    let mp3 = PathBuf::from("tests/fixtures/playback-errors/wave-ogg-flac-plan/raw-stereo.mp3");
    let wav = PathBuf::from("tests/fixtures/playback-errors/wave-ogg-flac-plan/wave-24in32-valid.wav");
    let options = fvid::media::CopyOptions::default();

    assert!(fvid::native_audio_mix::eligible_with_options(&[mp3.clone(), mp3.clone()], &options)
        .expect("MP3 eligible_with_options"));
    assert!(fvid::native_audio_mix::eligible_with_options(&[wav.clone(), wav.clone()], &options)
        .expect("WAVE eligible_with_options"));

    let mix_out = temp_path("mp3_mixed.wav");
    let _ = std::fs::remove_file(&mix_out);
    let stats = fvid::native_audio_mix::mix_audio_with_controls(
        &[mp3.clone(), mp3.clone()],
        &mix_out,
        &fvid::media::MixAudioOptions::default(),
        &options,
    )
    .expect("native_audio_mix::mix_audio_with_controls on MP3");
    assert!(stats.sample_frames > 0);
    let _ = std::fs::remove_file(&mix_out);
}

#[test]
fn native_audio_concat_reuses_active_geometry_and_packet_counts() {
    let mp3 = PathBuf::from("tests/fixtures/playback-errors/wave-ogg-flac-plan/raw-stereo.mp3");
    let options = fvid::media::CopyOptions {
        max_packets: Some(4),
        ..Default::default()
    };

    fvid::native_audio_mix::validate_concat_segments_with_options(
        &[mp3.clone(), mp3.clone()],
        &options,
    )
    .expect("validate_concat_segments_with_options under packet limit");

    let concat_out = temp_path("mp3_concat.wav");
    let _ = std::fs::remove_file(&concat_out);
    let stats = fvid::native_audio_mix::concat_audio_with_options(
        &[mp3.clone(), mp3],
        &concat_out,
        &options,
    )
    .expect("concat_audio_with_options under packet limit");
    assert!(stats.sample_frames > 0);
    let _ = std::fs::remove_file(&concat_out);
    let _ = Path::new(&concat_out);
}
