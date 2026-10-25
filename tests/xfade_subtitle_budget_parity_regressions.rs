use fvid::media::{self, CopyOptions, SubtitleCodec, SubtitleConvertOptions};
use std::path::{Path, PathBuf};

fn fixture_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/xfade-subtitle-budget-parity")
}

#[test]
fn xfade_plan_and_execution_preserve_zero_packet_byte_budget_and_invalid_header_parity() {
    let dir = fixture_dir();
    let main = dir.join("xfade-primary.y4m");
    let other = dir.join("xfade-secondary.y4m");

    let temp_dir = std::env::temp_dir().join(format!("fvid-batch253-xfade-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&temp_dir);
    std::fs::create_dir_all(&temp_dir).unwrap();

    // 1. max_packet_bytes == 0 must be rejected with "packet byte limit must be positive"
    // in both plan_xfade and xfade_video, not a generic unsupported cross-fade backend refusal.
    let mut zero_bytes = CopyOptions::default();
    zero_bytes.max_packet_bytes = 0;

    let plan_err = media::plan_xfade(&main, &other, "fade", 500_000, 500_000, &zero_bytes)
        .expect_err("plan_xfade must reject max_packet_bytes == 0");
    assert_eq!(plan_err, "packet byte limit must be positive");

    let out_zero = temp_dir.join("out-zero.mkv");
    let exec_err = media::xfade_video(
        &main,
        &other,
        &out_zero,
        "fade",
        500_000,
        500_000,
        &zero_bytes,
    )
    .expect_err("xfade_video must reject max_packet_bytes == 0");
    assert_eq!(exec_err, "packet byte limit must be positive");
    assert!(!out_zero.exists());

    // 2. Empty primary or secondary Y4M source or mismatched geometry must produce identical
    // specific diagnostics in plan_xfade and xfade_video.
    let empty_y4m = temp_dir.join("empty.y4m");
    std::fs::write(&empty_y4m, b"").unwrap();

    let empty_primary_plan = media::plan_xfade(
        &empty_y4m,
        &other,
        "fade",
        500_000,
        500_000,
        &CopyOptions::default(),
    )
    .unwrap_err();
    let out_empty_primary = temp_dir.join("out-empty-primary.mkv");
    let empty_primary_exec = media::xfade_video(
        &empty_y4m,
        &other,
        &out_empty_primary,
        "fade",
        500_000,
        500_000,
        &CopyOptions::default(),
    )
    .unwrap_err();
    assert_eq!(empty_primary_plan, "empty cross-fade primary source");
    assert_eq!(empty_primary_exec, empty_primary_plan);
    assert!(!out_empty_primary.exists());

    let empty_secondary_plan = media::plan_xfade(
        &main,
        &empty_y4m,
        "fade",
        500_000,
        500_000,
        &CopyOptions::default(),
    )
    .unwrap_err();
    let out_empty_secondary = temp_dir.join("out-empty-secondary.mkv");
    let empty_secondary_exec = media::xfade_video(
        &main,
        &empty_y4m,
        &out_empty_secondary,
        "fade",
        500_000,
        500_000,
        &CopyOptions::default(),
    )
    .unwrap_err();
    assert_eq!(empty_secondary_plan, "empty cross-fade secondary source");
    assert_eq!(empty_secondary_exec, empty_secondary_plan);
    assert!(!out_empty_secondary.exists());

    let _ = std::fs::remove_dir_all(&temp_dir);
}

#[test]
fn burn_subtitles_planning_enforces_encoded_ffv1_packet_byte_budget_on_y4m_and_matroska() {
    let dir = fixture_dir();
    let y4m = dir.join("xfade-primary.y4m");
    let burn_srt = dir.join("burn.srt");

    let temp_dir = std::env::temp_dir().join(format!("fvid-batch253-burn-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&temp_dir);
    std::fs::create_dir_all(&temp_dir).unwrap();

    // 1. On Y4M input (96x64 YUV420 = 9216 bytes raw frame; textured frame + burned text encodes to > 9500 bytes in FFV1):
    // Set max_packet_bytes = 9300 (> 9216 raw Y4M frame size, but < encoded FFV1 packet size).
    // Both plan_burn_subtitles and burn_subtitles must reject with "subtitle burn packet exceeds byte limit".
    let mut pkt9300 = CopyOptions::default();
    pkt9300.max_packet_bytes = 9300;

    let out_y4m = temp_dir.join("burn-y4m-9300.mkv");
    let y4m_exec_err = media::burn_subtitles(&y4m, &out_y4m, &burn_srt, &pkt9300)
        .expect_err("burn_subtitles must reject encoded FFV1 packet > 9300 bytes");
    assert!(
        y4m_exec_err.contains("subtitle burn packet exceeds byte limit"),
        "burn_subtitles y4m returned: {y4m_exec_err}"
    );
    assert!(!out_y4m.exists());

    let y4m_plan_err = media::plan_burn_subtitles(&y4m, &burn_srt, &pkt9300)
        .expect_err("plan_burn_subtitles must reject encoded FFV1 packet > 9300 bytes");
    assert_eq!(y4m_plan_err, y4m_exec_err);

    // 2. On Matroska FFV1 input: create a uniform 96x64 FFV1 Matroska source whose source packets are small (~60 bytes),
    // but burning the subtitle cue produces an encoded FFV1 packet > 200 bytes.
    // Set max_packet_bytes = 150 (> source FFV1 packets, < burned FFV1 packet).
    // Both plan_burn_subtitles and burn_subtitles must reject with the same packet byte limit error!
    let uniform_y4m = temp_dir.join("uniform-96x64.y4m");
    let mut uniform_bytes = b"YUV4MPEG2 W96 H64 F2:1 Ip A1:1 C420\n".to_vec();
    for _ in 0..2 {
        uniform_bytes.extend_from_slice(b"FRAME\n");
        uniform_bytes.extend(std::iter::repeat_n(32u8, 96 * 64));
        uniform_bytes.extend(std::iter::repeat_n(128u8, 48 * 32 * 2));
    }
    std::fs::write(&uniform_y4m, &uniform_bytes).unwrap();
    let mkv_source = temp_dir.join("uniform-source.mkv");
    media::transcode_lossless(
        &uniform_y4m,
        &mkv_source,
        Default::default(),
        &CopyOptions::default(),
    )
    .unwrap();

    let mut pkt150 = CopyOptions::default();
    pkt150.max_packet_bytes = 150;

    let out_mkv = temp_dir.join("burn-mkv-150.mkv");
    let mkv_exec_err = media::burn_subtitles(&mkv_source, &out_mkv, &burn_srt, &pkt150)
        .expect_err("burn_subtitles on Matroska source must reject burned FFV1 packet > 150 bytes");
    assert!(
        mkv_exec_err.contains("exceeds packet byte limit")
            || mkv_exec_err.contains("exceeds byte limit"),
        "burn_subtitles mkv returned: {mkv_exec_err}"
    );
    assert!(!out_mkv.exists());

    let mkv_plan_err = media::plan_burn_subtitles(&mkv_source, &burn_srt, &pkt150)
        .expect_err("plan_burn_subtitles on Matroska source must reject burned FFV1 packet > 150 bytes");
    assert_eq!(mkv_plan_err, mkv_exec_err);

    let _ = std::fs::remove_dir_all(&temp_dir);
}

#[test]
fn convert_subtitles_checks_formatted_ass_event_byte_limit_per_cue_before_subsequent_work_limit() {
    let dir = fixture_dir();
    let srt = dir.join("two-cues.srt");
    let mkv = dir.join("two-cues-subrip.mkv");

    let temp_dir = std::env::temp_dir().join(format!("fvid-batch253-sub-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&temp_dir);
    std::fs::create_dir_all(&temp_dir).unwrap();

    // In both two-cues.srt and two-cues-subrip.mkv:
    // Cue 0 raw text is 40 bytes ("SubRip cue zero payload with forty bytes"),
    // and its formatted ASS event payload ("0,0,Default,,0,0,0,,SubRip cue zero payload with forty bytes") is 60 bytes.
    // Cue 1 is the second cue.
    // With max_packet_bytes = 50 (> 40 raw bytes, < 60 formatted ASS bytes) and max_packets = Some(1):
    // Cue 0 already violates max_packet_bytes = 50! Both plan_convert_subtitles_with_controls and
    // convert_subtitles_with_controls must fail on cue 0 with "subtitle packet exceeds packet byte limit",
    // NOT advance to cue 1 and report "packet work limit exceeded".
    let sub_opts = SubtitleConvertOptions {
        streams: vec![],
        codec: SubtitleCodec::Ass,
    };
    let mut opts = CopyOptions::default();
    opts.max_packet_bytes = 50;
    opts.max_packets = Some(1);

    for (label, src) in [("srt", &srt), ("mkv", &mkv)] {
        let plan_err = media::plan_convert_subtitles_with_controls(src, &sub_opts, &opts)
            .expect_err("plan_convert_subtitles_with_controls must reject oversized cue 0");
        assert!(
            plan_err.contains("subtitle packet exceeds packet byte limit"),
            "plan_convert_subtitles_with_controls({label}) returned: {plan_err}"
        );

        let out = temp_dir.join(format!("out-{label}.mkv"));
        let exec_err = media::convert_subtitles_with_controls(src, &out, &sub_opts, &opts)
            .expect_err("convert_subtitles_with_controls must reject oversized cue 0");
        assert!(
            exec_err.contains("subtitle packet exceeds packet byte limit"),
            "convert_subtitles_with_controls({label}) returned: {exec_err}"
        );
        assert_eq!(exec_err, plan_err);
        assert!(!out.exists());
    }

    let _ = std::fs::remove_dir_all(&temp_dir);
}
