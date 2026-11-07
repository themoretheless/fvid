use fvid_control::CopyOptions;
use fvid_media::{
    apply_loudnorm, measure_loudness, owned_wave_plan, plan_loudnorm,
};
use std::path::Path;

#[test]
fn wave_plan_trim_and_g711_recognizes_single_open() {
    let g711 = Path::new("tests/fixtures/playback-errors/wave-ogg-flac-plan/g711-alaw-48k-mono.wav");
    let plan = owned_wave_plan::plan_trim(g711, 0, 50_000, &CopyOptions::default())
        .expect("plan trim g711");
    assert_eq!(plan.command, "trim");

    let dir = std::env::temp_dir().join(format!(
        "fvid-wave-plan-trim-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create temp dir");
    let wav = dir.join("pcm.wav");
    let mut bytes = Vec::new();
    let samples = [0i16, 1000, -1000, 500, -500, 0, 250, -250];
    let data_bytes = (samples.len() * 2) as u32;
    let riff_size = 36 + data_bytes;
    bytes.extend_from_slice(b"RIFF");
    bytes.extend_from_slice(&riff_size.to_le_bytes());
    bytes.extend_from_slice(b"WAVEfmt ");
    bytes.extend_from_slice(&16u32.to_le_bytes());
    bytes.extend_from_slice(&1u16.to_le_bytes());
    bytes.extend_from_slice(&1u16.to_le_bytes());
    bytes.extend_from_slice(&8000u32.to_le_bytes());
    bytes.extend_from_slice(&16000u32.to_le_bytes());
    bytes.extend_from_slice(&2u16.to_le_bytes());
    bytes.extend_from_slice(&16u16.to_le_bytes());
    bytes.extend_from_slice(b"data");
    bytes.extend_from_slice(&data_bytes.to_le_bytes());
    for s in samples {
        bytes.extend_from_slice(&s.to_le_bytes());
    }
    std::fs::write(&wav, &bytes).expect("write wav");

    let pcm_plan = owned_wave_plan::plan_trim(&wav, 0, 500_000, &CopyOptions::default())
        .expect("plan trim pcm wav");
    assert_eq!(pcm_plan.command, "trim");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn raw_flac_and_mp3_single_open_spool_and_container_loudnorm() {
    let mp3 = Path::new("tests/fixtures/playback-errors/wave-ogg-flac-plan/raw-stereo.mp3");
    let opus = Path::new("tests/fixtures/playback-errors/wave-ogg-flac-plan/ogg-opus-stereo.ogg");

    let mp3_loudness = measure_loudness(mp3, &CopyOptions::default()).expect("mp3 loudness");
    assert!(mp3_loudness.sample_frames > 0);

    let loudnorm_plan =
        plan_loudnorm(opus, None, false, &CopyOptions::default()).expect("opus loudnorm plan");
    assert_eq!(loudnorm_plan.command, "loudnorm");

    let out = std::env::temp_dir().join(format!(
        "fvid-batch603-opus-loudnorm-{}.wav",
        std::process::id()
    ));
    let _ = std::fs::remove_file(&out);
    let stats = apply_loudnorm(opus, &out, None, &CopyOptions::default()).expect("opus loudnorm");
    assert!(stats.sample_frames > 0);
    let _ = std::fs::remove_file(&out);
}
