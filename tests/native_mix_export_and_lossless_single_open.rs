use std::path::{Path, PathBuf};
use fvid::{
    media::{self, CopyOptions, MixAudioOptions},
    native_audio_mix, native_lossless,
};

#[test]
fn native_audio_mix_concat_and_lossless_eligibility_avoid_redundant_reopens() {
    let wav = PathBuf::from("tests/fixtures/playback-errors/wave-ogg-flac-plan/wave-24in32-valid.wav");

    let mix_opts = MixAudioOptions::default();
    let copy_opts = CopyOptions::default();
    let mix_plan = native_audio_mix::plan_with_controls(
        &[wav.clone(), wav.clone()],
        &mix_opts,
        &copy_opts,
    )
    .expect("native mix-audio plan on WAVE pair");
    assert_eq!(mix_plan.command, "mix-audio");

    let mut bounded_opts = CopyOptions::default();
    bounded_opts.max_packets = Some(4);
    assert!(
        native_audio_mix::concat_eligible_with_options(
            &[wav.clone(), wav.clone()],
            &bounded_opts,
        )
        .expect("concat eligibility on WAVE pair with packet cap")
    );

    assert!(
        !native_lossless::eligible(&wav).expect("lossless eligibility on WAVE"),
        "WAVE should be rejected after a single 9-byte prefix read"
    );

    let out_dir = std::env::temp_dir().join(format!("fvid-batch618-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&out_dir);
    std::fs::create_dir_all(&out_dir).expect("create temp dir");
    let y4m_path = out_dir.join("sample.y4m");
    let mut y4m_bytes = b"YUV4MPEG2 W2 H2 F25:1 Ip A1:1 C420\nFRAME\n".to_vec();
    y4m_bytes.extend_from_slice(&[128u8; 6]);
    y4m_bytes.extend_from_slice(b"FRAME\n");
    y4m_bytes.extend_from_slice(&[128u8; 6]);
    std::fs::write(&y4m_path, &y4m_bytes).expect("write synthetic Y4M");

    assert!(
        native_lossless::eligible(&y4m_path).expect("lossless eligibility on Y4M"),
        "Y4M should be accepted by native_lossless::eligible"
    );

    let y4m_plan = media::plan_trim_y4m(&y4m_path, 0, 80_000, &copy_opts)
        .expect("plan_trim_y4m on synthetic Y4M");
    assert_eq!(y4m_plan.command, "trim");

    let _ = std::fs::remove_dir_all(&out_dir);
}
