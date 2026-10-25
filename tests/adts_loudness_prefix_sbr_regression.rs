use fvid::media::{self, CopyOptions};
use std::path::PathBuf;

fn fixture_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/adts-loudness-prefix-sbr")
}

#[test]
fn adts_loudness_and_loudnorm_negotiate_implicit_sbr_rate_and_enforce_prefix_budgets() {
    let sbr_11k = fixture_dir().join("sbr-11k-to-22k.aac");
    let lc_then_ps = fixture_dir().join("lc-then-ps-24k.aac");
    let temp = std::env::temp_dir().join(format!("fvid-batch270-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&temp);
    std::fs::create_dir_all(&temp).unwrap();

    // 1. 11025 Hz core ADTS with implicit SBR -> 22050 Hz output rate (which is a qualified
    //    true-peak PCM rate, whereas 11025 Hz is not).
    //    owned_adts_loudness / owned_adts_loudnorm / media::plan_loudnorm / media::apply_loudnorm
    //    must negotiate the 22050 Hz SBR output rate instead of rejecting the 11025 Hz header rate.
    let fvid_media_plan = fvid_media::plan_loudness(&sbr_11k, &CopyOptions::default()).unwrap();
    assert_eq!(fvid_media_plan.command, "loudness");
    let fvid_media_loud = fvid_media::measure_loudness(&sbr_11k, &CopyOptions::default()).unwrap();
    assert_eq!(fvid_media_loud.sample_rate, 22_050);
    assert_eq!(fvid_media_loud.channels, 1);

    for dual in [false, true] {
        let ln_plan = media::plan_loudnorm(&sbr_11k, None, dual, &CopyOptions::default()).unwrap();
        assert_eq!(ln_plan.command, "loudnorm");
        let fvid_ln_plan =
            fvid_media::plan_loudnorm(&sbr_11k, None, dual, &CopyOptions::default()).unwrap();
        assert_eq!(fvid_ln_plan.command, "loudnorm");
    }

    let wav_ln = temp.join("sbr-11k-loudnorm.wav");
    let ln_stats = media::apply_loudnorm(&sbr_11k, &wav_ln, None, &CopyOptions::default()).unwrap();
    assert_eq!(ln_stats.sample_rate, 192_000);
    assert!(wav_ln.is_file());

    let wav_ln_dual = temp.join("sbr-11k-loudnorm-dual.wav");
    let ln_dual_stats =
        media::apply_loudnorm_dual(&sbr_11k, &wav_ln_dual, None, &CopyOptions::default()).unwrap();
    assert!(ln_dual_stats.dual_pass);
    assert!(wav_ln_dual.is_file());

    // 2. Zero-packet andzero-byte budgets on fvid_media::plan_loudness, fvid_media::measure_loudness,
    //    fvid_media::plan_loudnorm, and fvid_media::apply_loudnorm.
    let zero_packets = CopyOptions {
        max_packets: Some(0),
        ..Default::default()
    };
    let err = fvid_media::plan_loudness(&lc_then_ps, &zero_packets).unwrap_err();
    assert!(
        err.contains("loudness requires at least one packet"),
        "expected zero-packet refusal from fvid_media::plan_loudness, got: {err}"
    );
    let err = fvid_media::measure_loudness(&lc_then_ps, &zero_packets).unwrap_err();
    assert!(
        err.contains("loudness requires at least one packet"),
        "expected zero-packet refusal from fvid_media::measure_loudness, got: {err}"
    );
    let err = fvid_media::plan_loudnorm(&lc_then_ps, None, false, &zero_packets).unwrap_err();
    assert!(
        err.contains("loudnorm requires at least one packet"),
        "expected zero-packet refusal from fvid_media::plan_loudnorm, got: {err}"
    );
    let out_zero = temp.join("zero.wav");
    let err = fvid_media::apply_loudnorm(&lc_then_ps, &out_zero, None, &zero_packets).unwrap_err();
    assert!(
        err.contains("loudnorm requires at least one packet"),
        "expected zero-packet refusal from fvid_media::apply_loudnorm, got: {err}"
    );
    assert!(!out_zero.exists());

    // 3. Unbounded prefix where packet 0 is 8 bytes and packet 1 is 35 bytes (> 16):
    //    With max_packet_bytes = 16 and max_packets = None, planning must reject packet 1
    //    across media::plan_loudnorm, fvid_media::plan_loudnorm, and fvid_media::plan_loudness!
    let full_tight = CopyOptions {
        max_packet_bytes: 16,
        max_packets: None,
        ..Default::default()
    };
    for (name, res) in [
        (
            "fvid_media::plan_loudness",
            fvid_media::plan_loudness(&lc_then_ps, &full_tight),
        ),
        (
            "fvid_media::plan_loudnorm",
            fvid_media::plan_loudnorm(&lc_then_ps, None, false, &full_tight),
        ),
        (
            "media::plan_loudnorm",
            media::plan_loudnorm(&lc_then_ps, None, false, &full_tight),
        ),
        (
            "media::plan_loudness",
            media::plan_loudness(&lc_then_ps, &full_tight),
        ),
    ] {
        let err = res.expect_err(&format!("{name} should reject oversized packet 1"));
        assert!(
            err.contains("ADTS packet exceeds budget"),
            "unexpected error from {name}: {err}"
        );
    }

    // 4. With max_packets = Some(1) and max_packet_bytes = 16:
    //    Only packet 0 (8 bytes, 24 kHz mono) is in the prefix, so all plans and executions succeed
    //    and include the budget note in the plan!
    let prefix_opts = CopyOptions {
        max_packet_bytes: 16,
        max_packets: Some(1),
        max_controlled_bytes: Some(64 * 1024 * 1024),
        ..Default::default()
    };
    for (name, plan) in [
        (
            "fvid_media::plan_loudness",
            fvid_media::plan_loudness(&lc_then_ps, &prefix_opts).unwrap(),
        ),
        (
            "fvid_media::plan_loudnorm",
            fvid_media::plan_loudnorm(&lc_then_ps, None, false, &prefix_opts).unwrap(),
        ),
        (
            "media::plan_loudnorm",
            media::plan_loudnorm(&lc_then_ps, None, false, &prefix_opts).unwrap(),
        ),
        (
            "media::plan_loudness",
            media::plan_loudness(&lc_then_ps, &prefix_opts).unwrap(),
        ),
    ] {
        let budget_notes: Vec<_> = plan
            .notes
            .iter()
            .filter(|n| n.starts_with("encoded packet limit:"))
            .collect();
        assert_eq!(
            budget_notes.len(),
            1,
            "{name} should include exactly 1 budget note, got: {:?}",
            plan.notes
        );
    }

    // 5. Explicit channel weights on media::plan_loudness and media::measure_loudness_with_weights
    //    when max_controlled_bytes is set on lc-then-ps-24k.aac (which expands to 2 channels
    //    without max_packets, or 1 channel with max_packets = Some(1)):
    let full_controlled = CopyOptions {
        max_controlled_bytes: Some(64 * 1024 * 1024),
        ..Default::default()
    };
    let loud_2ch = media::measure_loudness_with_weights(&lc_then_ps, &full_controlled, &[1.0, 1.0])
        .expect("full lc-then-ps with [1.0, 1.0] weights should succeed");
    assert_eq!(loud_2ch.sample_frames, 4096);

    let loud_1ch = media::measure_loudness_with_weights(&lc_then_ps, &prefix_opts, &[1.0])
        .expect("1-packet prefix lc-then-ps with [1.0] weight should succeed");
    assert_eq!(loud_1ch.sample_frames, 1024);

    let _ = std::fs::remove_dir_all(&temp);
}
