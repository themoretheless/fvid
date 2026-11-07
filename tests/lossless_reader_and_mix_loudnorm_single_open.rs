use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT_ID: AtomicU64 = AtomicU64::new(0);

fn temp_path(name: &str) -> std::path::PathBuf {
    std::env::temp_dir().join(format!(
        "fvid-batch622-{}-{}-{}",
        std::process::id(),
        NEXT_ID.fetch_add(1, Ordering::Relaxed),
        name
    ))
}

#[test]
fn is_owned_audio_source_short_circuits_non_container_and_lossless_reuses_reader() {
    let flac = Path::new("tests/fixtures/playback-errors/wave-ogg-flac-plan/flac-4ch-quad.flac");
    assert!(
        !fvid::native_media::is_owned_audio_source(flac).expect("is_owned_audio_source on flac")
    );

    let y4m_path = temp_path("sample.y4m");
    std::fs::write(
        &y4m_path,
        b"YUV4MPEG2 W2 H2 F24:1 Ip A1:1 C420\nFRAME\n\x10\x10\x10\x10\x80\x80",
    )
    .expect("write y4m");
    assert!(fvid::native_lossless::eligible(&y4m_path).expect("lossless eligible on y4m"));
    let _ = std::fs::remove_file(&y4m_path);
}

#[test]
fn mix_merge_concat_and_loudnorm_plan_reuse_handles_and_skip_duplicate_probes() {
    let wav = Path::new("tests/fixtures/playback-errors/wave-ogg-flac-plan/wave-24in32-valid.wav")
        .to_path_buf();
    let flac = Path::new("tests/fixtures/playback-errors/wave-ogg-flac-plan/flac-4ch-quad.flac")
        .to_path_buf();

    let merge_plan = fvid::native_audio_mix::plan_merge(&[flac.clone(), flac])
        .expect("plan merge on standalone flac pair");
    assert_eq!(merge_plan.command, "merge-audio");

    assert!(
        fvid::native_audio_mix::concat_eligible(&[wav.clone(), wav.clone()])
            .expect("concat eligible on wav pair")
    );

    let loudnorm_plan = fvid::native_loudnorm::try_plan(
        &wav,
        None,
        false,
        &fvid::media::CopyOptions::default(),
    )
    .expect("try_plan loudnorm on wav")
    .expect("should return plan");
    assert_eq!(loudnorm_plan.command, "loudnorm");
}
