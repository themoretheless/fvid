use fvid::media::{
    CopyOptions, MixAudioOptions, apply_loudnorm, mix_audio_with_controls, plan_concat_y4m,
    plan_loudnorm, plan_mix_audio_with_controls, plan_remux, plan_xfade, xfade_video,
};
use std::fs;
use std::path::PathBuf;

#[test]
fn xfade_audio_mix_y4m_layout_remux_plan_and_loudnorm_reuse_handles() {
    let temp = std::env::temp_dir().join(format!(
        "fvid-xfade-mix-loudnorm-handle-reuse-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&temp);
    fs::create_dir_all(&temp).unwrap();

    // 1. Y4M xfade plan & execution
    let y4m_in = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/rotate-grid.y4m");
    let xfade_plan = plan_xfade(
        &y4m_in,
        &y4m_in,
        "fade",
        20_000,
        0,
        &CopyOptions::default(),
    )
    .unwrap();
    assert_eq!(xfade_plan.command, "xfade");

    let xfade_out = temp.join("xfade.mkv");
    let xfade_stats = xfade_video(
        &y4m_in,
        &y4m_in,
        &xfade_out,
        "fade",
        20_000,
        0,
        &CopyOptions::default(),
    )
    .unwrap();
    assert!(xfade_stats.video_frames >= 1);

    // 2. Y4M concat plan (validate_video_packet_budget_and_layout)
    let concat_y4m_plan =
        plan_concat_y4m(&[y4m_in.clone(), y4m_in], &CopyOptions::default()).unwrap();
    assert_eq!(concat_y4m_plan.command, "concat");

    // 3. Remux plan on Matroska and MP4
    let remux_mkv_plan = plan_remux(&xfade_out, &CopyOptions::default()).unwrap();
    assert_eq!(remux_mkv_plan.command, "remux");
    assert_eq!(remux_mkv_plan.streams.len(), 1);

    let mp4_in = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/avc-aac.mp4");
    if mp4_in.exists() {
        let remux_mp4_plan = plan_remux(&mp4_in, &CopyOptions::default()).unwrap();
        assert_eq!(remux_mp4_plan.command, "remux");
    }

    // 4. Non-WAVE (FLAC) audio mix plan & execution
    let flac_in = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/wave-ogg-flac-plan/flac-4ch-quad.flac");
    let mix_opts = MixAudioOptions::default();
    let mix_plan = plan_mix_audio_with_controls(
        &[flac_in.clone(), flac_in.clone()],
        &mix_opts,
        &CopyOptions::default(),
    )
    .unwrap();
    assert_eq!(mix_plan.command, "mix-audio");

    let mix_out = temp.join("mixed.wav");
    let mix_stats = mix_audio_with_controls(
        &[flac_in.clone(), flac_in],
        &mix_out,
        &mix_opts,
        &CopyOptions::default(),
    )
    .unwrap();
    assert!(mix_stats.sample_frames > 0);

    // 5. WAVE loudnorm plan & execution (linear gain)
    let linear_args = Some(
        "I=-24:TP=-2:LRA=7:measured_I=-24:measured_TP=-6:measured_LRA=2:measured_thresh=-34:linear=true",
    );
    let ln_plan = plan_loudnorm(&mix_out, linear_args, false, &CopyOptions::default()).unwrap();
    assert_eq!(ln_plan.command, "loudnorm");

    let ln_out = temp.join("norm.wav");
    let ln_stats =
        apply_loudnorm(&mix_out, &ln_out, linear_args, &CopyOptions::default()).unwrap();
    assert_eq!(ln_stats.sample_frames, mix_stats.sample_frames);

    let _ = fs::remove_dir_all(&temp);
}
