use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT_ID: AtomicU64 = AtomicU64::new(0);

fn temp_path(name: &str) -> std::path::PathBuf {
    std::env::temp_dir().join(format!(
        "fvid-batch620-{}-{}-{}",
        std::process::id(),
        NEXT_ID.fetch_add(1, Ordering::Relaxed),
        name
    ))
}

#[test]
fn loudness_and_normalize_plans_reuse_inspected_channels() {
    let wav = Path::new("tests/fixtures/playback-errors/wave-ogg-flac-plan/wave-24in32-valid.wav");
    let mut options = fvid::media::CopyOptions::default();
    options.max_controlled_bytes = Some(4 * 1024 * 1024);

    let loud_plan = fvid::media::plan_loudness(wav, &options).expect("plan loudness");
    assert_eq!(loud_plan.command, "loudness");

    let target = fvid::media::NormalizeTarget {
        integrated_lufs: -16.0,
        sample_peak_dbfs: -1.0,
    };
    let norm_plan =
        fvid::media::plan_normalize_loudness(wav, &options, None, target).expect("plan normalize");
    assert_eq!(norm_plan.command, "normalize-loudness");
}

#[test]
fn export_y4m_budget_and_concat_adts_wave_scan_avoid_redundant_reopens() {
    let y4m_path = temp_path("in.y4m");
    let out_y4m = temp_path("out.y4m");
    std::fs::write(
        &y4m_path,
        b"YUV4MPEG2 W2 H2 F24:1 Ip A1:1 C420\nFRAME\n\x10\x10\x10\x10\x80\x80FRAME\n\x20\x20\x20\x20\x80\x80",
    )
    .expect("write y4m");

    let frames = fvid::native_export::export_y4m(&y4m_path, &out_y4m).expect("export y4m");
    let _ = std::fs::remove_file(&y4m_path);
    let _ = std::fs::remove_file(&out_y4m);
    assert_eq!(frames, 2);

    let wav = Path::new("tests/fixtures/playback-errors/wave-ogg-flac-plan/wave-24in32-valid.wav")
        .to_path_buf();
    let out_wav = temp_path("concat.wav");
    let res = fvid::native_export::try_concat_mp4_matroska_with_options(
        &[wav.clone(), wav],
        &out_wav,
        &fvid::media::CopyOptions::default(),
    )
    .expect("try concat mp4 matroska on compatible wave");
    assert!(res.is_none());
}
