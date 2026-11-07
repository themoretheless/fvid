use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT_ID: AtomicU64 = AtomicU64::new(0);

fn temp_path(name: &str) -> std::path::PathBuf {
    std::env::temp_dir().join(format!(
        "fvid-batch619-{}-{}-{}",
        std::process::id(),
        NEXT_ID.fetch_add(1, Ordering::Relaxed),
        name
    ))
}

#[test]
fn plan_transcode_lossless_y4m_reuses_planar_eligibility() {
    let y4m_path = temp_path("sample.y4m");
    std::fs::write(
        &y4m_path,
        b"YUV4MPEG2 W2 H2 F24:1 Ip A1:1 C420\nFRAME\n\x10\x10\x10\x10\x80\x80FRAME\n\x20\x20\x20\x20\x80\x80",
    )
    .expect("write y4m");

    let transform = fvid::media::LosslessTransform {
        horizontal_flip: true,
        ..Default::default()
    };
    let plan = fvid::media::plan_transcode_lossless(
        &y4m_path,
        &transform,
        &fvid::media::CopyOptions::default(),
        None,
    )
    .expect("plan transcode lossless");
    let _ = std::fs::remove_file(&y4m_path);
    assert_eq!(plan.command, "transcode-lossless");
    assert_eq!(plan.streams.len(), 1);
    assert_eq!(plan.streams[0].codec, "rawvideo");
}

#[test]
fn plan_mix_and_merge_audio_with_default_controls_skip_duplicate_eligibility() {
    let wav = Path::new("tests/fixtures/playback-errors/wave-ogg-flac-plan/wave-24in32-valid.wav")
        .to_path_buf();
    let sources = vec![wav.clone(), wav];
    let mix_plan = fvid::media::plan_mix_audio(
        &sources,
        &fvid::media::MixAudioOptions::default(),
    )
    .expect("plan mix audio");
    assert_eq!(mix_plan.command, "mix-audio");

    let merge_plan = fvid::media::plan_merge_audio(&sources).expect("plan merge audio");
    assert_eq!(merge_plan.command, "merge-audio");
}

#[test]
fn decode_audio_route_and_remux_close_prefix_probe_before_delegating() {
    let wav = Path::new("tests/fixtures/playback-errors/wave-ogg-flac-plan/wave-24in32-valid.wav");
    let plan = fvid::media::plan_decode_audio(
        wav,
        &fvid::media::AudioDecodeTransform::default(),
        &fvid::media::CopyOptions::default(),
    )
    .expect("plan decode audio");
    assert_eq!(plan.command, "decode-audio");

    let adts = Path::new("tests/fixtures/playback-errors/adts-truncated-second-frame.aac");
    let out_mka = temp_path("out.mka");
    let err = fvid::media::remux(adts, &out_mka, &fvid::media::CopyOptions::default())
        .expect_err("truncated ADTS should fail during remux");
    let _ = std::fs::remove_file(&out_mka);
    assert!(!err.is_empty());
}
